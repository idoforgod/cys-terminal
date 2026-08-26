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
        // ★P3-6 좌석별 '엄격 관측 증명' 래치 (sid → (증명된 bin_base, 연속 엄격 관측 틱)).
        // watchdog 태스크 로컬 = 단일 writer. 아래 누수 차단 블록이 살아있는 surface 로 솎는다.
        // 데몬 재시작 후에는 비어 있고, 에이전트가 살아 있으면 몇 틱 안에 다시 증명된다
        // (미증명 구간의 거동 = 종전과 동일이므로 재시작이 회귀를 만들지 않는다).
        let mut agent_strict_proof: HashMap<u64, Option<(String, u32)>> = HashMap::new();
        let mut feed_reminded: HashMap<String, f64> = HashMap::new();
        let mut approval_debounce: HashMap<(u64, String), f64> = HashMap::new();
        // ★(U-16) 스캔 캐시 — 정규식 선컴파일 · 관문 코퍼스 · 관문 디바운스 · 격상 천장.
        // watchdog 태스크 로컬 = 단일 writer(형제 맵들과 같은 규약). 무효화 규칙은 ScanCaches doc.
        let mut scan_caches = ScanCaches::default();
        let mut queue_depth_alerted: HashMap<u64, f64> = HashMap::new();
        // ★G1(W2-D): 기아 경보(queue.starved) 전용 쿨다운 — depth_high 맵과 별도 축.
        let mut queue_starve_alerted: HashMap<u64, f64> = HashMap::new();
        // ★G2(W3-A): role 데드맨 상태 — watchdog 태스크 로컬(단일 writer). 구 단일 f64
        // 디바운스에서 role별 {misses·last_ok·death/idle 디바운스·좌석 점유 관측} 맵으로 승격.
        let mut deadman = DeadmanTracker::default();
        let mut alert_fired: HashMap<String, f64> = HashMap::new();
        // (learn gaps C12②) 재시작에도 디바운스 창 유지 — state 파일에서 복원.
        let mut learn_stuck_debounce: HashMap<u64, f64> =
            load_learn_stuck_debounce(&daemon.socket_path);
        let mut zombie_miss: HashMap<u64, u32> = HashMap::new();
        let mut launch_flag_warned: std::collections::HashSet<u64> =
            std::collections::HashSet::new();
        // ★P3-4 좌석별 (연속 관측실패 틱 수, 마지막 진단 발행 시각) — watchdog 태스크 로컬
        // (단일 writer). 아래 누수 차단 블록이 살아있는 surface 집합으로 솎는다.
        let mut launch_flag_blind: HashMap<u64, (u32, f64)> = HashMap::new();
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
                // ★watchdog 틱 순서 불변식 — 유일 명문화 지점(앵커는 라인번호가 아니라 함수명):
                //   refresh_seat_cache → deliver_queued → check_agent_death → check_role_deadman
                // ★SEAT: 프로세스 표를 갓 refresh 한 이 지점이 좌석 판정의 유일한 write 시점이다.
                // deliver_queued 보다 **먼저** 갱신해야 같은 틱의 배달이 최신 좌석 사실을 보고,
                // 사망·데드맨 계열 검사는 그 좌석·프로세스 사실의 소비자라 write 시점 뒤에 온다.
                // check_role_deadman 은 같은 틱의 신선한 seat_cache·seat_agent_cache(둘 다
                // refresh_seat_cache 가 단일 writer)와 check_agent_death 가 갓
                // 갱신한 agent_seen/agent_exit_notified 상태머신을 읽는다 — 이 순서가 깨지면
                // 데드맨이 stale 재료로 판정한다(미래 리팩터 시 이 4단 순서를 유지하라).
                refresh_seat_cache(&daemon, &sys);
                check_load(&daemon, &mut last_load_alert);
                check_surfaces(&daemon, &sys, &mut last_dup_alert, &mut last_proc_alert);
                check_idle(&daemon);
                deliver_queued(&daemon, &mut queue_depth_alerted, &mut queue_starve_alerted);
                reap_orphan_ledger(&daemon, &sys);
                reap_exited_surfaces(&daemon);
                reap_zombie_surfaces(&daemon, &sys, &mut zombie_miss);
                check_agent_death(&daemon, &sys, &mut restart_counts, &mut agent_strict_proof);
                check_surface_crash(&daemon);
                check_feed_aging(&daemon, &mut feed_reminded);
                check_feed_backlog(&daemon, &mut feed_backlog_alerted);
                check_approval_stall(&daemon, &mut approval_stall_fired);
                check_role_deadman(&daemon, &mut deadman);
                // 저빈도 검사(15초): 파일 stat·화면 렌더 — 5초마다 돌릴 필요 없음
                if tick_no.is_multiple_of(3) {
                    check_todo(&daemon);
                    check_approvals(&daemon, &mut approval_debounce, &mut scan_caches);
                    check_launch_flags(
                        &daemon,
                        &sys,
                        &mut launch_flag_warned,
                        &mut launch_flag_blind,
                    );
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
                queue_starve_alerted.retain(|sid, _| live_surface_ids.contains(sid));
                // ★(U-16) 관문 스캔의 좌석 키 맵도 같은 규약으로 솎는다.
                scan_caches.prune_surfaces(&live_surface_ids);
                learn_stuck_debounce.retain(|sid, _| live_surface_ids.contains(sid));
                zombie_miss.retain(|sid, _| live_surface_ids.contains(sid));
                agent_strict_proof.retain(|sid, _| live_surface_ids.contains(sid));
                launch_flag_warned.retain(|sid| live_surface_ids.contains(sid));
                launch_flag_blind.retain(|sid, _| live_surface_ids.contains(sid));
                // role 데드맨 트래커도 현재 감시 role 집합으로 솎는다(24/365 누수 차단 관례).
                deadman.retain_roles(&deadman_watched_roles());
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

/// 【P4-5】 env → **u32 클램프**. `as u32` 는 좁힘이 아니라 **wrap** 이라, 사람이 적어 넣은
/// 값의 의미와 정반대의 귀결을 낸다:
///   · `4294967296 → 0`   축이 조용히 **전면 비활성**(킬스위치를 누르지 않았는데 눌린다)
///   · `4294967297 → 1`   연속 **1틱**만으로 임계 성립 — 이 저장소에서 그 방향은 **오사망**이다.
///     (`CYS_AGENT_STRICT_PROOF_TICKS` 는 "스치는 도우미 프로세스로 증명이 서지 않게" 하려고
///      존재하는데, 절단이 정확히 그 도우미에게 증명을 세워 준다.)
/// 그래서 `cys.rs rpc_idle_timeout_with` 의 `v.min(MAX)` 와 **같은 규율**로 상한에 붙인다 —
/// 거대값은 언제나 '더 보수적인' 쪽으로만 접힌다. 규율이 커밋 안에서 갈리지 않게 한다.
fn env_u32(key: &str, default: u32) -> u32 {
    clamp_env_u32(env_u64(key, u64::from(default)))
}

/// [`env_u32`] 의 **순수 코어**(진리표 대상) — env 를 만지지 않는다. 프로세스 전역 env 를
/// 흔드는 검체는 병렬 테스트에서 서로를 오염시켜 **계측기 자체가 불안정**해진다.
fn clamp_env_u32(raw: u64) -> u32 {
    u32::try_from(raw).unwrap_or(u32::MAX)
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

/// 【P3-6】 좌석 자손 관측 → **생존 증거의 등급**(순수). 판정이 아니라 *증거 등급*이다 —
/// 등급을 생존/사망으로 접는 것은 `agent_alive_from_liveness` 의 몫이다(관측과 정책의 분리).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentLiveness {
    /// ① **엄격 증거** — 실행 증거(토큰 basename 일치·`.js` 번들·패키지 세그먼트 위의 `.js`).
    ///    `cmdline_matches_agent_exec` 가 인정하는 등급이며, 이것이 곧 '이 좌석의 에이전트를
    ///    엄격 매처로 볼 수 있다'는 증명 재료다.
    AliveStrict,
    /// ② **광의 증거만** — 생존 매처(`cmdline_matches_agent`)에만 걸린다. 실제로는 경로
    ///    세그먼트(`…/claude/…`) 하나로 잡힌 **비에이전트 자손**일 수 있다
    ///    (`tail -f ~/.cys/claude/x.log` · `less ~/dev/claude/NOTES.md` · `grep -r … claude-code/`).
    AliveBroadOnly,
    /// ③ 어떤 매치도 없다 — 종전대로 사망 상태머신으로 넘어간다.
    NoEvidence,
}

impl AgentLiveness {
    /// 이벤트 payload 에 싣는 증거 등급 표기(사후 감사가 '왜 죽었다고 했나'를 읽는다).
    fn as_str(self) -> &'static str {
        match self {
            AgentLiveness::AliveStrict => "strict",
            AgentLiveness::AliveBroadOnly => "broad_only",
            AgentLiveness::NoEvidence => "none",
        }
    }
}

/// 좌석 자손 cmdline 목록에서 생존 증거 등급을 뽑는다(순수 · ∃ 의미라 **순서 무관**).
/// strict ⊆ broad 이므로 등급은 전순서다: 엄격 매치가 하나라도 있으면 `AliveStrict`.
pub fn decide_agent_liveness(cmdlines: &[String], bin_base: &str) -> AgentLiveness {
    if cmdlines
        .iter()
        .any(|c| cmdline_matches_agent_exec(c, bin_base))
    {
        return AgentLiveness::AliveStrict;
    }
    if cmdlines.iter().any(|c| cmdline_matches_agent(c, bin_base)) {
        return AgentLiveness::AliveBroadOnly;
    }
    AgentLiveness::NoEvidence
}

/// 【P3-6】 이 좌석이 '엄격 매처로 자기 에이전트를 볼 수 있다'는 것이 **이미 증명**되었는가를
/// 갱신하고 반환한다(순수 · 상태는 호출자 = watchdog 태스크 로컬이 소유).
///
/// 【왜 증명을 요구하는가 — 오살 0칸 논증】 P3-6 수리의 핵심은 "광의 증거만 있는 좌석을
/// 사망으로 판정할 수 있는가"이고, 무조건 그렇게 하면 *광의로만 보이는 형태로 뜨는 에이전트*
/// 를 오살한다. 그래서 **그 좌석에서 엄격 매처가 실제로 그 에이전트를 본 적이 있을 때만**
/// 좁힌다. 증명된 좌석이라면 "에이전트가 살아 있다 ⇒ 엄격 매치가 있다"가 관측으로 성립하므로,
/// 엄격 매치가 사라졌는데 광의 매치만 남은 상태는 **그 광의 매치가 에이전트가 아니라는 뜻**
/// 이다. 미증명 좌석에서는 좁히지 않고 종전 거동(광의=생존)을 그대로 둔다 — 이 설계는
/// 자기보호적이다: 좁힘은 자기가 안전함을 스스로 입증한 좌석에서만 켜진다.
///
/// 【히스테리시스】 증명은 **연속 `arm_ticks` 틱**의 엄격 관측을 요구한다. 한 틱만 스치는
/// 엄격 매치(사용자가 손으로 돌린 `node …/claude-code/cli.js --version` 같은 도우미)로
/// 증명이 서는 것을 막는다. 한 번 선 증명은 내리지 않는다 — 에이전트가 죽어 엄격 증거가
/// 사라지는 것이 정확히 우리가 잡으려는 상태이기 때문이다.
/// `arm_ticks == 0` 은 **좁힘 전면 비활성**(킬스위치 — 종전 거동 복귀).
/// bin_base 가 바뀌면(좌석의 에이전트 교체) 증명은 처음부터 다시 쌓는다.
pub fn update_strict_proof(
    proof: &mut Option<(String, u32)>,
    bin_base: &str,
    strict_now: bool,
    arm_ticks: u32,
) -> bool {
    if arm_ticks == 0 {
        return false;
    }
    match proof {
        Some((seen_bin, streak)) if seen_bin == bin_base => {
            if strict_now {
                *streak = streak.saturating_add(1);
            } else if *streak < arm_ticks {
                *streak = 0; // 아직 증명 전 — 연속이 끊기면 처음부터
            }
        }
        _ => *proof = Some((bin_base.to_string(), u32::from(strict_now))),
    }
    proof
        .as_ref()
        .is_some_and(|(seen_bin, streak)| seen_bin == bin_base && *streak >= arm_ticks)
}

/// 【P3-6】 증거 등급 + 증명 여부 → 생존 판정(순수).
///
/// 【고친 결함】 U-5 의 argv 승격은 `cmdline_matches_agent` 의 관측 대상을 `name()` 한 토큰에서
/// **명령줄 전체**로 넓혔다. 라운드 1 은 같은 넓힘의 *다른 소비자*(readiness 안전 밸브 —
/// `refresh_seat_cache` 의 `seat_agent_cache`)만 보고 거기에만 엄격 매처 AND 를 걸었고,
/// **사망감지의 `alive` 에는 보정이 없었다**. 그래서 에이전트가 죽은 뒤에도 자손 중 아무거나
/// argv 에 `…/claude/…` 경로 세그먼트를 가진 채 살아 있으면(`tail -f ~/.cys/claude/x.log`,
/// 백그라운드 툴 자식) `alive=true` → `agent.exited` 미발행 → node-recover 미발동 →
/// **고아 좌석**. 개정 전에는 관측이 `name()` 한 토큰(경로 구분자가 없다)이라 세그먼트 매칭이
/// 원리상 발화할 수 없었다 — 승격이 만든 **새 오탐 축**이다.
///
/// 【판정표】
///   AliveStrict                → 생존 (종전과 동일 · strict ⊆ broad 라 새 오살 경로 없음)
///   AliveBroadOnly ∧ 미증명    → 생존 (종전 거동 보존 — 좁히지 않는다)
///   AliveBroadOnly ∧ 증명됨    → **사망 후보** (새 오탐 축을 여기서만 닫는다)
///   NoEvidence                 → 사망 후보 (종전과 동일)
pub fn agent_alive_from_liveness(liveness: AgentLiveness, strict_proven: bool) -> bool {
    match liveness {
        AgentLiveness::AliveStrict => true,
        AgentLiveness::AliveBroadOnly => !strict_proven,
        AgentLiveness::NoEvidence => false,
    }
}

/// T2-5 에이전트 사망 감지: 셸은 살았는데 그 위의 에이전트 프로세스만 죽은 상태를
/// 즉시 잡는다 (기존엔 pane.idle 300초가 최초 신호 — '생각 중'과 구분 불가).
/// 판정: 자식 트리에서 agents.json 등록 바이너리가 '한 번 보였다가 사라짐' 전이.
fn check_agent_death(
    daemon: &Arc<Daemon>,
    sys: &System,
    restart_counts: &mut HashMap<u64, u32>,
    strict_proof: &mut HashMap<u64, Option<(String, u32)>>,
) {
    // ★P3-6 좁힘 무장 임계(연속 틱). 0 = 좁힘 비활성(종전 거동). 기본 2 — watchdog 5초 주기라
    // 10초 연속 엄격 관측이면 증명이 선다(스치는 도우미 프로세스는 통과하지 못한다).
    let strict_arm_ticks = env_u32("CYS_AGENT_STRICT_PROOF_TICKS", 2);
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
        // ★G5-③(W5-A) claim_role 관측 등록의 2-표본째 확정 훅 — Windows arm 이 스테이징한
        // pending_agent_obs 를, 프로세스 표가 이미 refresh 된 이 자리(추가 스냅샷 비용 0)에서
        // 재관측해 확정/포기/유보를 판정한다(판정은 순수 함수 confirm_pending_obs 단독).
        // unix 에서는 pending 이 구조적으로 항상 None(claim_role 이 즉시 등록)이라 no-op —
        // 훅 자체는 무cfg 로 두어 순수 판정자·소거 규약을 전 OS 테스트가 봉인한다.
        // strict ⊆ broad 부분집합 보증(select_observed_agent 주석)이 그대로 성립하므로
        // "등록됐는데 사망감지(broad)가 못 보는" 오살 비대칭은 신설되지 않는다.
        let pending = s.pending_agent_obs.lock().unwrap().clone();
        if let Some(pending) = pending {
            // 가드 재확인: meta 미확정·역할 보유일 때만 확정 자격. 그 외(set_meta 선점·역할
            // 해제)는 pending 이 무의미해진 것 — 확정 실패가 아니라 승계/해제이므로 조용히 소거.
            let guard_ok =
                s.agent_meta.lock().unwrap().is_none() && s.role.lock().unwrap().is_some();
            if !guard_ok {
                *s.pending_agent_obs.lock().unwrap() = None;
            } else {
                let candidates = known_agent_candidates();
                // ★argv 승격(U-5): 에이전트 식별은 명령줄 토큰 매칭이다 — 이름 한 토큰
                // (`node.exe` 래퍼)으로는 참/거짓을 낼 수 없다. 범위는 이 좌석의 자손만.
                let cmds: Vec<String> = collect_descendants_with_cmd(sys, s.pid)
                    .into_iter()
                    .map(|(_, cmd)| cmd)
                    .collect();
                let current = select_observed_agent(&cmds, &candidates);
                match confirm_pending_obs(&pending, current.as_ref(), now, PENDING_OBS_TTL_SECS) {
                    PendingVerdict::Commit => {
                        // Commit ⇒ current=Some(동일 에이전트) — 신선한 2표본째를 기록한다
                        // (bin 은 같은 후보표 파생이라 통상 동일 · 표 갱신 시 신선한 쪽이 진실).
                        if let Some((agent, bin)) = current {
                            *s.agent_meta.lock().unwrap() = Some((agent.clone(), bin.clone()));
                            // 실관측 파생 arming — unix 즉시 등록 경로(claim_role 핸들러)와 동일
                            // 의미론: 허위 DEAD 과도기 없이 사망감지 상태머신을 정직하게 무장.
                            s.agent_seen.store(true, Ordering::Relaxed);
                            s.agent_exit_notified.store(false, Ordering::Relaxed);
                            *s.pending_agent_obs.lock().unwrap() = None;
                            let role = s.role.lock().unwrap().clone();
                            daemon.bus.publish(
                                "agent.observed",
                                "system",
                                Some(s.id),
                                json!({"role": role, "agent": agent, "agent_bin": bin,
                                       "via": "claim_role_probe_win_confirmed"}),
                            );
                            // 확정 meta 는 콜드부트 부활 재료 — topology 에 즉시 영속
                            // (claim_role 핸들러의 persist 와 동일 의무).
                            persist_topology(daemon);
                        }
                    }
                    PendingVerdict::Drop { reason } => {
                        *s.pending_agent_obs.lock().unwrap() = None;
                        // 확정 실패 침묵 방지(W5-A [MAJOR]) — 관측·감사가 '등록이 왜 안 됐나'를
                        // 이벤트로 본다(fail-closed 는 유지하되 fail-silent 는 금지).
                        let role = s.role.lock().unwrap().clone();
                        daemon.bus.publish(
                            "agent.observe_dropped",
                            "system",
                            Some(s.id),
                            json!({"role": role, "agent": pending.0, "agent_bin": pending.1,
                                   "reason": reason,
                                   "surface_ref": cys::surface_ref(s.id)}),
                        );
                    }
                    PendingVerdict::Keep => {}
                }
            }
        }
        let Some((agent, bin)) = s.agent_meta.lock().unwrap().clone() else {
            continue;
        };
        let bin_base = bin.rsplit(['/', '\\']).next().unwrap_or(&bin).to_string();
        // ★argv 승격(U-5): 생존 매칭은 cmdline 토큰 단위이므로 이름 폴백으로는 래퍼 기동
        // (`node.exe <…>/claude`)을 오사망으로 읽는다 → 허위 agent.exited → node-recover 재기동
        // 스폰(자원 관점에서 비용이 가장 비싼 오판). 승격 범위는 이 좌석의 자손 pid 만.
        // ★P3-6: 생존 판정은 **증거 등급 + 좌석별 증명**의 두 단계다(근거 전문은
        // `agent_alive_from_liveness` 주석). 광의 매치 하나로 생존을 선언하던 종전 한 줄은
        // U-5 argv 승격 뒤 `tail -f ~/.cys/claude/x.log` 같은 비에이전트 자손을 생존 증거로
        // 승격시켜 고아 좌석을 만들었다. 좁힘은 '엄격 매처가 이 좌석의 에이전트를 본 적 있다'가
        // 증명된 좌석에서만 켜지므로 오살 경로를 새로 열지 않는다.
        let cmdlines: Vec<String> = collect_descendants_with_cmd(sys, s.pid)
            .into_iter()
            .map(|(_, cmdline)| cmdline)
            .collect();
        let liveness = decide_agent_liveness(&cmdlines, &bin_base);
        let strict_proven = update_strict_proof(
            strict_proof.entry(s.id).or_default(),
            &bin_base,
            liveness == AgentLiveness::AliveStrict,
            strict_arm_ticks,
        );
        let alive = agent_alive_from_liveness(liveness, strict_proven);
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
                   // ★P3-6 사후 감사용 증거 등급 — "왜 죽었다고 판정했나"를 이벤트가 답한다.
                   // broad_only = 광의 매치는 있었으나 증명된 좌석이라 에이전트로 인정하지 않음.
                   "evidence": liveness.as_str(),
                   "strict_proven": strict_proven,
                   "restart_count": restart_counts.get(&s.id).copied().unwrap_or(0)}),
        );
        if !auto_restart {
            continue;
        }
        // 401·로그인 만료로 죽은 에이전트의 무한 재기동 루프 차단.
        // ★판정은 state::auth_blocked_by_recent_health 단일 술어 — 여기서 룰 목록·창을 복제해
        //   갖고 있으면 한쪽만 바뀔 때 차단이 조용히 샌다(T3-G2). 근거 원장은 recent_health 뿐이며,
        //   run_health_rules 는 담화로 억제한 매칭도 이 원장에는 남긴다(억제≠인터록 해제).
        let auth_blocked = crate::state::auth_blocked_by_recent_health(
            &daemon.recent_health.lock().unwrap(),
            s.id,
            now,
        );
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
            use cys::SpawnPolicy as _;
            let cli = crate::state::sibling_cli_path();
            let _ = tokio::time::timeout(
                Duration::from_secs(180),
                tokio::process::Command::new(cli)
                    .arg("node-recover")
                    .arg("--surface")
                    .arg(cys::surface_ref(sid))
                    // ★P3-2(봉인 구멍) 데몬이 낳는 CLI 는 **전부** autostart 를 봉인한다.
                    // 근거는 U-7 이 `schedule.rs::launch_via_cli` 에 같은 호출을 넣은 것과
                    // 동일하다: 이 자식 cys 가 소켓 연결에 실패하면 `spawn_detached_daemon`
                    // 으로 **라이벌 데몬을 낳는다**(데몬 종료 중·소켓 교체 중에 정확히 그 창이
                    // 열린다). 자기 데몬이 자기 경쟁자를 스폰하는 재귀 기동은 폭주 경로다.
                    // main.rs 의 office-bridge·auto-restore·phoenix self-test 는 셋 다 이미
                    // 걸어 두었고, 트리에서 빠진 마지막 한 곳이 여기였다.
                    // (등급은 `Attached` 의미 — 아래 `.output()` 으로 끝까지 기다리는 유계
                    //  자식이라 분리하지 않는다. hide_console 은 종전 그대로.)
                    .no_autostart()
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
    let threshold = env_u32("CYS_RSI_STUCK_RESTARTS", 3);
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

// ─────────────────────────────────────────────────────────────────────────────
// ★G2(W3-A) role dead-man v2 — idle 과 death 의 판정 축 분리 (결함 8 봉합)
//
// v1은 침묵(last_output 경과) 단독으로 master.deadman(alert)을 발화해 살아있는 zsh를
// 사망으로 오라벨했다. v2에서 침묵은 정보성 master.idle 로 강등되고, master.deadman 은
// [surface 소멸 / EOF / 셸 pid 부재 / 에이전트 사망(좌석 빈사)] 프로세스 생존 실패가
// grace(기본 60s) + 연속 miss 확증(기본 3틱=15s)을 소진했을 때만 발화한다.
//
// env 노브(★CYS_ROLE_DEADMAN_* — 이 역할(role) 데드맨 전용. startup-lock 데드맨
// (deadman.rs)과는 **무관한 별개 서브시스템**이다 — 이름공간 혼동 금지):
//   CYS_MASTER_DEADMAN_SECS       존치·기본 900 — 의미가 '침묵(idle) 임계'로 정련.
//                                 0 = idle 신호만 비활성(사망 판정은 fail-closed 로 상시 유지 —
//                                 v1의 '전체 비활성'에서 의도적으로 축소된 유일한 행동 변경).
//   CYS_ROLE_DEADMAN_CONFIRM_TICKS 신설·기본 3(최소 1) — DeadCandidate 연속 관측 확증 틱 수.
//   CYS_ROLE_DEADMAN_GRACE_SECS    신설·기본 60 — 부트/승계 직후 무카운트 창(오살 방지).
//   CYS_ROLE_DEADMAN_DEBOUNCE_SECS 신설·기본 300 — 현행 하드코딩 5분 디바운스의 승격(기본 불변).
//   CYS_ROLE_DEADMAN_IDLE_DEBOUNCE_SECS 신설 — master.idle 전용 디바운스.
//                                 기본 = CYS_ROLE_DEADMAN_DEBOUNCE_SECS(=300) 체이닝 —
//                                 미설정 시 현행 동작 그대로, 설정 시 death 재상기 주기와 분리
//                                 (death 를 조여도 idle 소음이 같이 늘지 않는다 — 리뷰 MINOR).
//   CYS_ROLE_DEADMAN_ROLES         신설·기본 "master" — 감시 role CSV(일반화 opt-in).
// ─────────────────────────────────────────────────────────────────────────────

/// 감시 대상 role 집합 — CSV env 의 단일 해석처(check_role_deadman·prune 공유. 이원화 금지).
fn deadman_watched_roles() -> Vec<String> {
    let csv = std::env::var("CYS_ROLE_DEADMAN_ROLES").unwrap_or_default();
    let v: Vec<String> = csv
        .split(',')
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    if v.is_empty() {
        vec!["master".to_string()]
    } else {
        v
    }
}

/// ★G2 role 데드맨 트래커 — watchdog 태스크 로컬(단일 writer) role별 상태.
/// 연속 miss 카운터는 reap_zombie_surfaces 의 zombie_miss 관용구와 동형: 슬립/웨이크의
/// timestamp 점프에도 N회 **실관측**을 요구해 위양성이 없다.
#[derive(Default)]
pub struct DeadmanTracker {
    /// role → 연속 DeadCandidate 관측 수(Alive/Idle 에서 리셋 · Unknown 은 무증감).
    misses: HashMap<String, u32>,
    /// role → 마지막 생존(Alive/Idle) 확정 관측 epoch — 발화 payload 의 last_ok_epoch.
    last_ok: HashMap<String, f64>,
    /// role → 마지막 master.deadman 발화 epoch(디바운스).
    last_death_alert: HashMap<String, f64>,
    /// role → 마지막 master.idle 발화 epoch(디바운스 — death 와 별도 축).
    last_idle_alert: HashMap<String, f64>,
    /// role → 그 role 좌석(surface id)에서 **기지 에이전트 엄격 관측**(seat_agent_cache —
    /// refresh_seat_cache 가 cmdline_matches_agent_exec 로 갱신)이 잡힌 적 있는 sid.
    /// meta 부재 보조축(SeatVacantNoMeta)의 armed 조건 — agent_seen 상태머신의 무meta 판.
    /// ★원시 Occupied(아무 자손) 관측으로 armed 금지(BLOCK 교정): vim/빌드 좌석 오살 = 결함 8 동형.
    /// sid 를 저장해 role 이 다른 surface 로 재바인딩되면 자동 해제된다(새 좌석 재관측 요구).
    seat_agent_seen: HashMap<String, u64>,
    /// role → (sid, meta 있음 ∧ agent_seen=false 상태의 최초 관측 epoch) — 기동 즉사 쌍둥이
    /// 셀(AgentNeverStarted)의 grace 앵커. set_meta(재기동 제스처)는 created_at/claimed 를
    /// 갱신하지 않으므로 일반 grace 가 못 덮는 재등록 직후 스폰 지연을 이 상태 나이 기준
    /// grace 로 보호한다(늦출 뿐 오라벨하지 않는다 — fail-safe 방향).
    never_seen_since: HashMap<String, (u64, f64)>,
}

impl DeadmanTracker {
    /// 24/365 누수 차단: 감시 role 집합 밖의 키를 전 맵에서 솎는다(watchdog prune 블록 호출).
    fn retain_roles(&mut self, roles: &[String]) {
        self.misses.retain(|r, _| roles.contains(r));
        self.last_ok.retain(|r, _| roles.contains(r));
        self.last_death_alert.retain(|r, _| roles.contains(r));
        self.last_idle_alert.retain(|r, _| roles.contains(r));
        self.seat_agent_seen.retain(|r, _| roles.contains(r));
        self.never_seen_since.retain(|r, _| roles.contains(r));
    }
}

/// ★G2 role dead-man v2: idle(침묵)과 death(프로세스 생존 실패)를 분리 판정한다.
/// 판정은 순수함수 liveness_verdict(SeatState 인접 정의) — 이 함수는 그 판정의 집행자다.
/// 절차: role 해소 → grace → 판정 → (Idle=master.idle / DeadCandidate=miss 누적 →
/// confirm 소진 시 master.deadman). 이벤트명·category("alert")·reason 구값
/// ("… surface gone"/"… surface exited")은 보존, payload 는 additive 확장.
/// ★침묵(idle) 단독으로 death 를 발화하는 경로는 존재하지 않는다 — 절대 불변(결함 8).
fn check_role_deadman(daemon: &Arc<Daemon>, tracker: &mut DeadmanTracker) {
    let idle_threshold = env_u64("CYS_MASTER_DEADMAN_SECS", 900);
    let confirm_ticks = env_u32("CYS_ROLE_DEADMAN_CONFIRM_TICKS", 3).max(1);
    let grace_secs = env_u64("CYS_ROLE_DEADMAN_GRACE_SECS", 60);
    let debounce_secs = env_u64("CYS_ROLE_DEADMAN_DEBOUNCE_SECS", 300);
    // idle 전용 디바운스(리뷰 MINOR): 기본은 death 노브 체이닝(현행 동작 불변) — 설정 시 분리.
    let idle_debounce_secs = env_u64("CYS_ROLE_DEADMAN_IDLE_DEBOUNCE_SECS", debounce_secs);
    let now = now_epoch();
    for role in deadman_watched_roles() {
        let Some(sid) = daemon.roles.lock().unwrap().get(&role).copied() else {
            // 역할 미등록 — 데몬 단독 가동 등 정상 상황(v1 의미 유지). 스테일 카운터 청소.
            tracker.misses.remove(&role);
            tracker.seat_agent_seen.remove(&role);
            tracker.never_seen_since.remove(&role);
            continue;
        };
        let surface = daemon.get_surface(sid);
        // ② grace: 부트·승계 직후는 판정 자체를 쉰다(무증감 — 오살 방지). 앵커는
        // max(master_claimed_at(master 한정), created_at). surface 소멸 시 created_at 부재 →
        // 데몬 내부 사실(gone)이라 즉시 카운트 대상.
        let claimed = if role == "master" {
            daemon.master_claimed_at.lock().unwrap().unwrap_or(0.0)
        } else {
            0.0
        };
        let created = surface.as_ref().map(|s| s.created_at).unwrap_or(0.0);
        if now - claimed.max(created) < grace_secs as f64 {
            continue;
        }
        // meta 부재 보조축 arming: 이 role 좌석(sid)에서 **기지 에이전트 엄격 관측**
        // (seat_agent_cache — refresh_seat_cache 가 같은 틱에 cmdline_matches_agent_exec 로
        // 갱신)이 잡힌 적 있는가. ★원시 Occupied(아무 자손)로 armed 금지(BLOCK 교정):
        // vim/less/빌드 자손 1틱 관측 → 프롬프트 복귀(Empty) → 살아있는 맨 셸 오살 = 결함 8 동형.
        let seat = surface
            .as_ref()
            .map(|s| SeatState::from_u8(s.seat_cache.load(Ordering::Relaxed)))
            .unwrap_or(SeatState::Unknown);
        if surface
            .as_ref()
            .is_some_and(|s| s.seat_agent_cache.load(Ordering::Relaxed))
        {
            tracker.seat_agent_seen.insert(role.clone(), sid);
        }
        let seat_agent_seen = tracker.seat_agent_seen.get(&role) == Some(&sid);
        // ★기동 즉사 쌍둥이 셀(리뷰 MAJOR) 상태 추적: meta 있음 ∧ agent_seen=false(첫 sysinfo
        // 관측 이전) 상태의 최초 관측 시각. 이 상태로 grace 를 소진해야만 AgentNeverStarted
        // 후보가 된다 — set_meta 는 created_at/claimed 를 갱신하지 않으므로 일반 grace 가 못
        // 덮는 재등록(node-recover ③) 직후 스폰 지연을 상태 나이 grace 가 보호한다(fail-safe).
        let never_seen = surface.as_ref().is_some_and(|s| {
            s.agent_meta.lock().unwrap().is_some() && !s.agent_seen.load(Ordering::Relaxed)
        });
        let never_seen_grace_expired = if never_seen {
            let entry = tracker.never_seen_since.entry(role.clone()).or_insert((sid, now));
            if entry.0 != sid {
                *entry = (sid, now); // 좌석 재바인딩 = 새 기동 창(grace 재시작)
            }
            now - entry.1 >= grace_secs as f64
        } else {
            tracker.never_seen_since.remove(&role);
            false
        };
        // 셸 생존 입력은 state::pid_alive 단일 정의처(liveness_verdict doc 의 권위 규정).
        let shell_alive = surface
            .as_ref()
            .map(|s| crate::state::pid_alive(s.pid))
            .unwrap_or(false);
        let verdict = liveness_verdict(
            surface.as_deref(),
            seat,
            shell_alive,
            seat_agent_seen,
            never_seen_grace_expired,
            idle_threshold,
        );
        // 발화 payload 공통 재료(inputs) — 판정 축·입력값을 전부 기록(관측 가능성 요구 b).
        let agent_name: Option<String> = surface
            .as_ref()
            .and_then(|s| s.agent_meta.lock().unwrap().as_ref().map(|(a, _)| a.clone()));
        let agent_alive: Option<bool> = surface.as_ref().and_then(|s| {
            if agent_name.is_some() && s.agent_seen.load(Ordering::Relaxed) {
                Some(!s.agent_exit_notified.load(Ordering::Relaxed))
            } else {
                None // meta 부재 또는 미기동(관측 이전) = 미측정(null) — 측정 불능 ≠ 사망
            }
        });
        let status_age_secs: Option<f64> = surface.as_ref().and_then(|s| {
            s.agent_status
                .lock()
                .unwrap()
                .as_ref()
                .map(|st| (now - st.updated_at).max(0.0))
        });
        let idle_secs: u64 = surface
            .as_ref()
            .map(|s| s.last_output.lock().unwrap().elapsed().as_secs())
            .unwrap_or(0);
        match verdict {
            LivenessVerdict::Alive => {
                tracker.last_ok.insert(role.clone(), now);
                tracker.misses.remove(&role);
            }
            LivenessVerdict::Unknown => {
                // ③ 측정 불능 ≠ 사망 ≠ 생존: misses 증가도 last_ok 갱신도 하지 않는다.
                // (Unknown 이 카운터를 리셋하면 간헐 프로브 실패가 진짜 사망 확증을 세탁한다.)
            }
            LivenessVerdict::Idle { idle_secs } => {
                // 생존 확정 + 침묵 — 정보성 master.idle 만. death 카운터는 생존으로 리셋.
                tracker.last_ok.insert(role.clone(), now);
                tracker.misses.remove(&role);
                let last = tracker.last_idle_alert.get(&role).copied().unwrap_or(0.0);
                if now - last >= idle_debounce_secs as f64 {
                    tracker.last_idle_alert.insert(role.clone(), now);
                    // category 는 GUI onDaemonEvent 폴백 토스트 레인("watchdog"/"health"/"feed")
                    // 부재 값 "info" — idle 은 alert 가 아니다(무한 토스트 차단 · G2 BLOCKER 결정).
                    daemon.bus.publish(
                        "master.idle",
                        "info",
                        Some(sid),
                        json!({
                            "role": role, "surface_ref": cys::surface_ref(sid),
                            "axis": "silence", "idle_secs": idle_secs,
                            "threshold_secs": idle_threshold, "debounce_secs": idle_debounce_secs,
                            "process_alive": true, "agent_alive": agent_alive,
                            "last_output_epoch": now - idle_secs as f64,
                            "severity": "info",
                        }),
                    );
                }
            }
            LivenessVerdict::DeadCandidate { axis } => {
                // ④ 에이전트 계열 축 한정 소켓측 반증: confirm 창 내 status.set 자기보고가
                // 있으면 생존 증거로 인정(Alive 취급). 부재는 증거가 아니다 — fail-safe 방향.
                let confirm_window = (confirm_ticks as u64 * WATCHDOG_INTERVAL_SECS) as f64;
                let socket_rebuts = matches!(
                    axis,
                    DeadmanAxis::AgentDead
                        | DeadmanAxis::SeatVacantNoMeta
                        | DeadmanAxis::AgentNeverStarted
                ) && status_age_secs.is_some_and(|age| age <= confirm_window);
                if socket_rebuts {
                    tracker.last_ok.insert(role.clone(), now);
                    tracker.misses.remove(&role);
                    continue;
                }
                let misses = tracker.misses.entry(role.clone()).or_insert(0);
                *misses += 1;
                let misses = *misses;
                if misses < confirm_ticks {
                    continue; // ⑤ 확증 미소진 — 발화 금지(연속 실관측 N회 요구)
                }
                let last = tracker.last_death_alert.get(&role).copied().unwrap_or(0.0);
                if now - last < debounce_secs as f64 {
                    continue; // role별 디바운스(기본 300s — v1 하드코딩 승격)
                }
                tracker.last_death_alert.insert(role.clone(), now);
                // reason: 진짜 사망 2계열 구값 보존(role=master 에서 v1 문자열과 동일).
                // "master silent" reason 은 소멸 — 그 소멸이 결함 8 수정의 본질.
                let reason = match axis {
                    DeadmanAxis::SurfaceGone => format!("{role} surface gone"),
                    DeadmanAxis::SurfaceExited => format!("{role} surface exited"),
                    DeadmanAxis::ShellProcDead => "shell process dead".to_string(),
                    DeadmanAxis::AgentDead => "agent process dead".to_string(),
                    DeadmanAxis::SeatVacantNoMeta => "agent seat empty (no meta)".to_string(),
                    DeadmanAxis::AgentNeverStarted => {
                        "agent never started (seat empty)".to_string()
                    }
                };
                daemon.bus.publish(
                    "master.deadman",
                    "alert",
                    Some(sid),
                    json!({
                        "reason": reason,
                        "axis": axis.as_str(),
                        "role": role,
                        "surface_ref": cys::surface_ref(sid),
                        // 전 축 공통 top-level(HUD p.get("idle_secs") 소비 연속성)
                        "idle_secs": idle_secs,
                        "inputs": {
                            "pid": surface.as_ref().map(|s| s.pid),
                            "seat_state": surface.as_ref().map(|_| seat.as_str()),
                            "agent_meta": agent_name,
                            "agent_alive": agent_alive,
                            "status_age_secs": status_age_secs,
                        },
                        "thresholds": {
                            "confirm_ticks": confirm_ticks,
                            "tick_secs": WATCHDOG_INTERVAL_SECS,
                            "grace_secs": grace_secs,
                            "debounce_secs": debounce_secs,
                        },
                        "misses": misses,
                        "last_ok_epoch": tracker.last_ok.get(&role).copied().unwrap_or(0.0),
                    }),
                );
            }
        }
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
/// 충돌 시 **디스크가 이긴다**(사용자 주권 불변).
///
/// ★(U-16 doc 정정 · 2026-08-24) 종전 이 자리에는 "approval_patterns 는 *감지 전용*(자동 응답
/// 절대 없음)" 이라고 적혀 있었다. **데몬 안에서는 참이지만 키 전체로는 거짓**이다 — 같은 키를
/// CLI 도 읽고, 거기에는 폴더신뢰 자동확인(`trust-prompt` 1건)이 붙어 있다(U-15). 그 문장을
/// 그대로 읽은 다음 감사자가 "감지 전용이니 관문 문면도 여기 넣으면 되겠다" 로 오독하면
/// **면책 창에 Return 이 가는 킬체인**이 그대로 돌아온다(2026-07-29 실사고). 정정 후 계약:
///   · 이 키의 소비자는 **둘**이다 — 데몬 격상 스캔(응답 0) · CLI 폴더신뢰 자동확인(응답 1건).
///   · 그래서 이 키에는 **자동응답이 붙어도 안전한 문면만** 넣는다.
///   · **첫기동 관문 문면은 이 키에 합치지 않는다.** 별도 키(`first_run_gates`)·별도 스캐너·
///     별도 feed kind 로 간다(U-16 절 참조). 그 분리를 `approval_patterns_union_*` 검체가
///     기계로 집행한다.
/// 합집합이 안전측인 이유는 그대로다: 추가 패턴의 대가는 과잉 감지(사람이 한 번 더 본다)이고
/// 미감지의 대가는 워커 hang 이다. 순수 함수로 분리해 테스트 가능하게 둔다.
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

// ═══════════════════════════════════════════════════════════════════════════
// ★(U-16) 데몬 첫기동 관문 스캔 — approval 스캔과 **분리된** 두 번째 스캐너
// ═══════════════════════════════════════════════════════════════════════════
//
// ## 왜 `approval_patterns` 에 합치지 않는가
//
// 한 키에 넣으면 성질이 반대인 두 사실이 한 네임스페이스에서 섞인다:
//   · **approval** = "사람이 예/아니오를 눌러야 할 프롬프트". 이 축에는 **자동응답 계약이
//     붙어 있다**(CLI 의 폴더신뢰 자동확인 1건이 그 계약의 유일 소비자다).
//   · **관문**     = "온보딩이 아직 끝나지 않았다". 이 축은 **자동응답이 금지**된 사실이다 —
//     면책 창의 기본 포커스가 `No, exit` 이라 Return 한 발이 좌석을 rc 1 로 죽인 2026-07-29
//     실사고가 그 금지의 근거다.
// 둘이 한 키가 되는 순간 approval 소비자가 관문 문면을 **자기 계약(자동응답)** 으로 읽을 수
// 있게 되고, 그 오독의 귀결이 정확히 이 저장소가 가장 비싸게 치른 사고다. 그래서 **키도,
// 스캐너도, feed kind 도, 디바운스 축도** 나눈다(합치는 것은 diff 한 줄이지만 되돌리는 데는
// 좌석 하나가 든다).
//
// ## 문면의 진실원천은 `cys::first_run_gates` 하나다
//
// 코퍼스·매칭 규칙(needle OR ∧ 위젯 서명 AND)·자기규칙(보편 토큰 단독 위젯 금지 등)은 전부
// 그 모듈이 소유한다. 여기서는 **소비만** 한다 — 사본을 만들면 문면이 갈리고(S-1 샷건
// 서저리), 자기규칙을 우회하면 BLOCK-1/BLOCK-2 오탐이 데몬 층에서 그대로 되살아난다.
//
// ## 생애 창 상한 — 치명위험 ①(이벤트·큐 폭주) 차단
//
// 스캔은 **첫 각성 ack(`awakened_at` 래치) 이전 · 좌석 나이 상한 이내**에만 열린다. 그렇게
// 하지 않으면 작업 중인 노드가 화면에 관문 문면을 출력하는 순간(이 캠페인의 감사 문서나
// `src/first_run_gates.rs` 를 `cat` 하는 순간이 그렇다) master 각성 push 가 반복된다.
// U-14 의 주입 가드가 **같은 축을 같은 래치로** 닫는다(`inject_guard::decide` 의 `awakened`)
// — 두 층이 다른 기준으로 창을 재면 한쪽만 열려 오탐이 새어 나온다.
// 창이 닫힌 귀결은 언제나 '아무 것도 발행하지 않음' = **정확히 종전 동작**이다.
//
// ## 자동 응답 절대 금지 — 주석이 아니라 **타입**으로
//
// 이 스캐너는 `first_run_gates::action_policy` 를 **호출하지 않는다**. 산출 타입
// [`GateSighting`] 에는 `select_index`·`literal`·`down` 같은 **키 재료가 하나도 없다** —
// 없는 값은 보낼 수 없다. 통과 액션의 집행은 자기 게이트를 가진 부트 경로(U-14/U-19) 소관이다.
//
// ## 자기발화 차단 — 격상 문안에 **화면 인용을 싣지 않는다**
//
// approval 격상은 매칭 줄 `excerpt` 를 문안에 싣는다. 관문에서 같은 짓을 하면 그 문안이
// master pane 에 렌더되는 순간 **needle 이 화면에 다시 나타나** 스캔·주입 가드가 자기 자신을
// 잡는다(U-18 이 같은 형태를 인증 처방 문안에서 겪었다). 그래서 관문 격상 문안은 좌석 참조와
// **관문 id·제목(한국어)** 만 싣는다. 이 불변식은 `gate_escalation_text_is_not_itself_a_gate`
// 가 코퍼스 전량에 대해 기계로 집행한다.

/// ★U-16 롤백 스위치의 env 이름. `0` → 데몬 관문 스캐너를 통째로 끈다(= 이 단위 착지 이전).
const ENV_GATE_SCAN: &str = "CYS_GATE_SCAN";

/// 관문 feed 항목의 `kind`. approval 네임스페이스와 **다른 문자열**인 것이 오염 차단의 핵심이다
/// — `has_pending_daemon_approval`·approval stale-clear 는 `kind == "approval"` 로 거르므로
/// 두 스캐너의 생명주기가 서로를 종결시키지 않는다.
const GATE_FEED_KIND: &str = "first_run_gate";

/// 관문 격상 디바운스 창(초). approval 디바운스(60초)와 **같은 값·다른 축**이다.
const GATE_SCAN_DEBOUNCE_SECS: f64 = 60.0;

/// 생애 창 상한의 기본값(초). 첫기동 관문은 좌석이 태어난 직후에만 뜬다 — 이 창을 넘긴
/// 좌석은 **관측하지 않는다**. 비대칭이 값을 정한다: 놓치는 대가는 '종전대로 감지 0' 이고,
/// 오탐의 대가는 master 각성 폭주다.
const GATE_SCAN_WINDOW_SECS: f64 = 600.0;

/// 창 안에서 한 좌석이 낼 수 있는 격상 수의 **절대 상한**. 실측 관문은 6종이고 좌석 하나가
/// 그보다 많이 격상할 이유가 없다 — 코퍼스가 커지거나 화면이 요동쳐도 폭주하지 않게 하는
/// 마지막 천장이다(치명위험 ①). 상한에 닿은 좌석은 창이 **영구히 닫힌다**(카운터를 되돌리는
/// 경로가 없다 — 되돌리면 천장이 천장이 아니다).
const GATE_SCAN_MAX_ESCALATIONS: u32 = 8;

/// 데몬 관문 스캐너가 켜져 있는가 — **env 를 읽는 유일한 지점**(롤백 스위치 1지점 규약).
///
/// ★마스터 스위치에 접는다(`cys::gate_axes_forced_legacy()`): 사고 순간에 사람이 노브를
/// 조합할 수 없다는 것이 BLOCK-3 의 교훈이고, 이 캠페인이 추가한 축은 `CYS_BOOT_GATES=0`
/// 하나로 **전부** 종전 복귀해야 한다. 이 축의 '종전' 은 **스캔 없음**이다.
///
/// ★왜 lib 의 [`cys::GateAxes`] 필드로 올리지 않았는가: 그 타입은 **좌석 판정·전송에 참여하는
/// 축**의 집합이고, BLOCK-4 불변식("보류 장치가 꺼지면 판정도 전부 느슨해진다")이 그 넷
/// 사이에서만 의미를 갖는다. 관측 전용 노브를 그 타입에 섞으면 불변식의 대상이 흐려진다 —
/// 대신 **접기값을 소비**해서 마스터 스위치의 도달 범위는 그대로 지킨다.
fn gate_scan_enabled() -> bool {
    gate_scan_enabled_with(
        std::env::var(ENV_GATE_SCAN).ok().as_deref(),
        cys::gate_axes_forced_legacy(),
    )
}

/// 위 판정의 **순수 코어** — 자기 노브와 상위 접기값의 합류(진리표 대상).
///
/// `forced_legacy` 는 `cys::gate_axes_forced_legacy()`(마스터 `CYS_BOOT_GATES=0` ∨ 보류 장치
/// 꺼짐)의 값이다. **어느 한쪽만으로도 꺼진다** — 롤백이 '반쯤 되돌아가는' 것이 가장 위험하다는
/// 것이 이 캠페인의 BLOCK-3/BLOCK-4 교훈이고, 관측 축도 예외가 아니다.
fn gate_scan_enabled_with(raw: Option<&str>, forced_legacy: bool) -> bool {
    gate_scan_enabled_from(raw) && !forced_legacy
}

/// 자기 노브 **하나만** 보는 순수 술어. 형제 게이트
/// (`cys::gate_pending_axis_enabled_from`)와 같은 **엄격 비교** — 오타 하나로 안전장치가
/// 조용히 뒤집히지 않는다.
fn gate_scan_enabled_from(raw: Option<&str>) -> bool {
    raw != Some("0")
}

/// 관문 스캔 개폐 판정 입력의 **전량**. 이 술어는 판정 중에 파일·시계·전역·env 를 읽지 않는다
/// (숨은 입력 금지 — `inject_guard::Observed` 와 같은 규율).
#[derive(Debug, Clone, Copy, PartialEq)]
struct GateScanWindow {
    /// 롤백 스위치를 접은 값.
    enabled: bool,
    /// 이 **어댑터**가 관문 코퍼스의 적용 대상으로 선언돼 있는가(`first_run_gates` 봉투 실재).
    ///
    /// ★코드 정본 코퍼스는 **claude 를 실측한 것**이다(`MEASURED_ON` 도 claude 버전이다).
    ///   그것을 어댑터 구분 없이 모든 좌석에 들이대면, 예컨대 codex 좌석 화면에
    ///   `Enter to confirm`·`Esc to cancel` 같은 흔한 위젯 줄과 유사 문면이 함께 뜨는 순간
    ///   **남의 관문**으로 격상된다. 그래서 적용 대상은 봉투(vendor 임베드 또는 사용자 디스크)가
    ///   **선언**한 어댑터로 한정한다 — 다른 어댑터를 넣고 싶으면 봉투를 선언하면 된다
    ///   (코드 수정 없이 넓힐 수 있고, 선언 없이는 넓어지지 않는다).
    declared: bool,
    /// 이 좌석이 첫 각성 ack 를 받았는가(`awakened_at` 래치 존재 = 창 닫힘).
    awakened: bool,
    /// 좌석 나이(초).
    age_secs: f64,
    /// 창 상한(초).
    window_secs: f64,
    /// 이 좌석이 창 안에서 이미 낸 격상 수.
    escalations: u32,
    /// 격상 수 천장.
    max_escalations: u32,
}

/// 지금 이 좌석의 화면을 관문으로 스캔해도 되는가 — **순수**(진리표 대상).
///
/// 닫힘의 귀결은 언제나 '아무 것도 발행하지 않음' = 정확히 종전 동작이다. 그래서 이 술어는
/// **오살 방향으로 열리지 않고**, 판정 불능(나이가 NaN·무한 = 시계 사고)은 **닫는다**.
fn gate_scan_open(w: &GateScanWindow) -> bool {
    gate_scan_observe_open(w) && w.escalations < w.max_escalations
}

/// 이 좌석의 화면을 **관측**해도 되는가 — 순수(진리표 대상). [`gate_scan_open`] 에서
/// **격상 천장 축만 뺀** 술어다.
///
/// ★왜 나눴는가(결함 4): 천장의 목적은 **폭주 방지**이지 **신호 삭제**가 아니다. 종전에는
/// 천장에 닿는 순간 창이 영구히 닫히면서, 관문이 아직 화면에 떠 있어도 pending feed 항목을
/// `gate-window-closed` 로 종결했다 — 운영자의 **유일한 배지 신호**가 사라진다. 시나리오는
/// 평범하다: 좌석이 관문 앞에 멈춘 채 사람이 feed 항목을 8회 확인/해소 → `seat.count` 천장 →
/// 남은 pending 전량 종결. 그래서 두 축을 분리한다:
///   · **관측**(이 술어) — 화면에서 관문이 사라졌는지 계속 본다. 발행이 없으므로 폭주가 아니다.
///     생애 창(`window_secs`)·각성 ack·롤백이 여전히 상한이라 무한 관측이 아니다.
///   · **발행**([`gate_scan_open`]) — 천장에 닿으면 멈춘다. 천장은 그대로 천장이다.
fn gate_scan_observe_open(w: &GateScanWindow) -> bool {
    w.enabled
        && w.declared
        && !w.awakened
        && w.age_secs.is_finite()
        && w.age_secs <= w.window_secs
}

/// 이 에이전트의 관문 override 봉투를 고른다 — **디스크 우선 · 부재 시에만 vendor 임베드**.
///
/// `cys.rs fill_missing_fields` 와 **같은 규칙**이다. 사본이 아니라 동형 재현인 이유: 데몬은
/// CLI 의 `load_agent_spec` 을 지나지 않으므로 계층을 여기서 한 번 더 성립시켜야 한다.
///   · 디스크에 키가 **있으면**(명시 `null` 포함) 디스크가 이긴다 — 사용자 주권 불변.
///   · 키가 **아예 없을 때만** 임베드 봉투를 쓴다 — 기존 설치 기계 도달 경로(K-1).
/// 어느 쪽에도 없으면 `None` = 코드 정본만 쓴다(그것이 코퍼스의 SOT 다).
fn gate_envelope<'a>(
    disk: &'a serde_json::Value,
    embed: &'a serde_json::Value,
    agent: &str,
) -> Option<&'a serde_json::Value> {
    let key = cys::first_run_gates::ADAPTER_KEY;
    if let Some(v) = disk.get(agent).and_then(|a| a.get(key)) {
        return Some(v);
    }
    embed.get(agent).and_then(|a| a.get(key))
}

/// 화면에서 본 관문의 요약. **키 재료가 하나도 없다** — `select_index`·`literal`·`down` 이
/// 이 타입에 없는 것이 "데몬은 자동 응답하지 않는다" 계약의 구조적 형태다(주석이 아니라 타입).
#[derive(Debug, Clone, PartialEq, Eq)]
struct GateSighting {
    id: String,
    title: String,
    /// 사람이 1회 해야 통과한다(로그인·OAuth) — 처방 문안이 갈린다.
    human_only: bool,
}

/// 화면 → 관문 요약. 매칭은 코퍼스가 소유한 `identify`(needle OR ∧ **위젯 서명 AND**) 하나만
/// 쓴다. 여기서 needle 만 보는 느슨한 술어(`inject_guard::needle_hit`)를 쓰면 안 된다 —
/// 그 술어의 안전 방향은 '놓치지 않기' 이고, 데몬 격상의 안전 방향은 '깨우지 않기' 다.
fn gate_sighting(gates: &[cys::first_run_gates::Gate], screen: &str) -> Option<GateSighting> {
    let g = cys::first_run_gates::identify(gates, screen)?;
    Some(GateSighting {
        id: g.id.clone(),
        title: g.title.clone(),
        human_only: g.passability == cys::first_run_gates::Passability::HumanOnly,
    })
}

/// 관문 격상 문안(feed 본문 · master 각성 큐 · 이벤트가 공유하는 단일 생성처).
///
/// ★화면 인용 금지(자기발화 차단 — 위 절 참조). 입력은 좌석 참조와 관문 **식별자**뿐이다.
fn gate_escalation_text(surface_ref: &str, hit: &GateSighting) -> String {
    let what = if hit.human_only {
        "사람이 1회 처리해야 통과한다(로그인·OAuth 는 기계가 통과시킬 수 없다)"
    } else {
        "통과 액션은 부트 경로가 자기 게이트 아래에서 집행한다 — 데몬은 응답하지 않는다"
    };
    format!(
        "[관문감지] {surface_ref} 첫기동 관문 보류 (id={} · {}) — {what}. \
         화면 확인은 cys read-screen 으로 하라(문면은 자기발화 차단을 위해 싣지 않는다).",
        hit.id, hit.title
    )
}

/// 데몬이 발행한 이 좌석의 **관문** feed 항목 id 스냅샷. `kind` 가 approval 과 다르므로
/// `Daemon::pending_daemon_approvals`(kind=="approval")와 서로 간섭하지 않는다 — 두 스캐너의
/// 생명주기 분리는 이 한 글자 차이가 집행한다.
fn pending_gate_items(daemon: &Arc<Daemon>, surface_id: u64) -> Vec<String> {
    daemon
        .feed_items
        .lock()
        .unwrap()
        .iter()
        .filter(|i| {
            i.status == "pending"
                && i.kind == GATE_FEED_KIND
                && i.surface_id == Some(surface_id)
                && crate::state::is_daemon_issued(&i.request_id)
        })
        .map(|i| i.request_id.clone())
        .collect()
}

/// 좌석별 관문 격상 상태(watchdog 태스크 로컬).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct GateSeatState {
    /// 창 안에서 낸 격상 수. **되돌리지 않는다**(천장이 천장이려면 단조여야 한다).
    count: u32,
    /// 창이 닫힌 뒤 pending 항목을 이미 정리했는가 — 매 틱 feed 락을 다시 잡지 않기 위한 래치.
    cleaned: bool,
}

/// ★틱마다 재컴파일·재해소하지 않기 위한 스캔 캐시. watchdog 태스크 로컬 = **단일 writer**
/// (데몬 전역 상태로 올리면 재시작 영속·RPC 노출 계약이 딸려오는데, 여기 필요한 것은
/// 프로세스 수명의 캐시뿐이다 — `APPROVAL_WAKEUP_RECENT` 와 같은 판단).
///
/// ## 무효화 규칙 (명시)
///
/// | 맵 | 키 | 왜 stale 이 될 수 없는가 | 성장 차단 |
/// |---|---|---|---|
/// | `approval_re` | 패턴 **문자열 그 자체** | `Regex::new` 은 순수·결정론 — 같은 문자열의 산출은 영원히 같다 | 매 pass 관측된 패턴 집합으로 `retain` |
/// | `gate_corpus` | (override 스위치 값, 봉투 `Value` **전문**) | `first_run_gates::resolve_with` 의 **입력 전량**이 키다 — 하나라도 다르면 재해소 | 매 pass 관측된 agent 집합으로 `retain` |
/// | `gate_debounce` | (surface, 관문 id) | 시각 비교라 stale 개념 없음 | 살아있는 surface 집합으로 `retain`(watchdog 누수 차단 블록) |
/// | `gate_escalations` | surface | 좌석 수명과 동일 | 위와 같음 |
///
/// ★mtime·해시 같은 **추정 키를 쓰지 않은 이유**: 추정 키는 "같은 나노초에 같은 길이로
/// 고치면 안 바뀐 것으로 본다" 는 조용한 오답을 남긴다. 봉투는 작은 JSON 서브트리라 전문
/// 비교가 싸고, 싸면 추정할 이유가 없다. 사람이 `agents.json` 을 고치면 **다음 pass 에 반영**된다.
#[derive(Default)]
struct ScanCaches {
    approval_re: HashMap<String, Option<regex::Regex>>,
    gate_corpus: HashMap<String, GateCorpusEntry>,
    gate_debounce: HashMap<(u64, String), f64>,
    gate_escalations: HashMap<u64, GateSeatState>,
}

/// 캐시된 관문 코퍼스 한 항목 — 키(입력 전량)와 산출을 함께 들고 있다.
struct GateCorpusEntry {
    override_on: bool,
    envelope: Option<serde_json::Value>,
    gates: Arc<Vec<cys::first_run_gates::Gate>>,
    /// ★(N9) 해소가 남긴 사유 — 자기규칙 **수리**·Fatal 관문 **강제 복원**·완화 **거부**.
    /// 종전엔 `.gates` 만 취하고 이것을 버렸다. 그 셋은 전부 "사용자가 선언한 값을 시스템이
    /// 바꾸는 행위"인데, 데몬에서 그 사실이 어디에도 남지 않으면 사용자는 자기 override 가
    /// 왜 안 먹는지 알 방법이 없다(CLI 는 같은 정보를 `cys.rs::resolve_gate_corpus` 가 낸다).
    notes: Vec<String>,
}

impl ScanCaches {
    /// approval 패턴 정규식을 **선컴파일 캐시**에서 얻는다. 컴파일 실패도 `None` 으로 캐시해
    /// 매 pass 같은 손상 패턴을 다시 컴파일하지 않는다(판정은 종전과 동일 — 실패 = 건너뜀).
    fn approval_regex(&mut self, pattern: &str) -> Option<&regex::Regex> {
        self.approval_re
            .entry(pattern.to_string())
            .or_insert_with(|| regex::Regex::new(pattern).ok())
            .as_ref()
    }

    /// 관문 코퍼스를 캐시에서 얻는다(미스면 `resolve_with` 로 해소해 적재).
    ///
    /// ★(N9) 해소가 남긴 사유는 **여기서 stderr 로 발행**한다. 발행은 아래 판정 절반이
    /// 돌려주는 값에만 의존하므로, 유계성(캐시 미스 1회)은 그 절반의 검체가 지킨다.
    fn corpus(
        &mut self,
        agent: &str,
        envelope: Option<&serde_json::Value>,
        override_on: bool,
    ) -> Arc<Vec<cys::first_run_gates::Gate>> {
        let (gates, fresh_notes) = self.corpus_with_notes(agent, envelope, override_on);
        for n in &fresh_notes {
            eprintln!("[gate-corpus] {agent}: {n}");
        }
        gates
    }

    /// 위의 **판정 절반** — 반환 `notes` 는 **캐시 미스일 때만** 비어 있지 않다.
    ///
    /// ★유계성이 여기에 있다: 캐시 히트는 빈 벡터를 돌려주므로 위 발행 루프가 아무것도 찍지
    /// 않는다. 히트에서도 채워 돌려주면 watchdog 틱마다 같은 줄이 반복돼 24/365 데몬 로그가
    /// 덮인다(반복 발행은 재난 ① 방향). 보관은 계속한다 — 버리는 것과 조용한 것은 다르다.
    fn corpus_with_notes(
        &mut self,
        agent: &str,
        envelope: Option<&serde_json::Value>,
        override_on: bool,
    ) -> (Arc<Vec<cys::first_run_gates::Gate>>, Vec<String>) {
        if let Some(e) = self.gate_corpus.get(agent) {
            if e.override_on == override_on && e.envelope.as_ref() == envelope {
                return (e.gates.clone(), Vec::new());
            }
        }
        let resolved = cys::first_run_gates::resolve_with(envelope, override_on);
        let gates = Arc::new(resolved.gates);
        self.gate_corpus.insert(
            agent.to_string(),
            GateCorpusEntry {
                override_on,
                envelope: envelope.cloned(),
                gates: gates.clone(),
                notes: resolved.notes,
            },
        );
        // ★발행 재료는 **보관본에서 읽는다** — 보관본과 발행본이 갈라질 자리를 없앤다.
        //   (그리고 보관이 장식이 아님을 프로덕션이 스스로 증명한다: 이 읽기가 없으면
        //    `notes` 는 검체만 보는 죽은 필드가 되고, 그것은 '버렸다' 와 구별되지 않는다.)
        let fresh = self
            .gate_corpus
            .get(agent)
            .map(|e| e.notes.clone())
            .unwrap_or_default();
        (gates, fresh)
    }

    /// pass 끝 정리 — 이번 pass 에서 관측되지 않은 키를 버린다(24/365 데몬 누수 차단).
    /// surface 키 맵은 여기서 손대지 않는다(watchdog 틱의 살아있는 surface 집합이 소유).
    fn prune_pass(
        &mut self,
        seen_patterns: &std::collections::HashSet<String>,
        seen_agents: &std::collections::HashSet<String>,
    ) {
        self.approval_re.retain(|k, _| seen_patterns.contains(k));
        self.gate_corpus.retain(|k, _| seen_agents.contains(k));
    }

    /// 살아있는 surface 집합으로 좌석 키 맵을 솎는다(watchdog 누수 차단 블록에서 호출).
    fn prune_surfaces(&mut self, live: &std::collections::HashSet<u64>) {
        self.gate_debounce.retain(|(sid, _), _| live.contains(sid));
        self.gate_escalations.retain(|sid, _| live.contains(sid));
    }
}

/// T4-16 승인 격상 스캔: agents.json의 approval_patterns를 visible screen에 매칭.
/// ★자동 응답 절대 금지 — 감지·격상(이벤트+feed 항목)만. 판단은 master의 몫.
///
/// ★(U-16) 같은 pass 에서 **두 번째 스캐너**가 돈다 — 첫기동 관문 스캔. 두 스캐너는 화면
/// 스냅샷 하나를 공유하지만(파서 락 1회) 그 밖의 모든 것(키·코퍼스·디바운스·feed kind·
/// 롤백 스위치)이 분리돼 있다. 위 U-16 절이 그 분리의 근거를 전부 담고 있다.
fn check_approvals(
    daemon: &Arc<Daemon>,
    debounce: &mut HashMap<(u64, String), f64>,
    caches: &mut ScanCaches,
) {
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
    // ★env 는 pass 당 한 번만 읽는다(좌석마다 읽으면 같은 pass 안에서 판정이 갈릴 수 있다).
    let scan_on = gate_scan_enabled();
    let override_on = cys::first_run_gates::override_enabled();
    let mut seen_patterns: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_agents: std::collections::HashSet<String> = std::collections::HashSet::new();
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    for s in surfaces {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        let Some((agent, _)) = s.agent_meta.lock().unwrap().clone() else {
            continue;
        };
        seen_agents.insert(agent.clone());
        let patterns = merged_approval_patterns(&agents, &embed_agents, &agent);
        for p in &patterns {
            if let Some(pat) = p["pattern"].as_str() {
                seen_patterns.insert(pat.to_string());
            }
        }
        // ★(U-16) 관문 창 개폐 — 화면 스냅샷을 뜨기 **전에** 판정한다(닫힌 좌석에 비용 0).
        let envelope = gate_envelope(&agents, &embed_agents, &agent);
        let seat = caches.gate_escalations.get(&s.id).copied().unwrap_or_default();
        let window = GateScanWindow {
            enabled: scan_on,
            declared: envelope.is_some(),
            awakened: s.awakened_at.lock().unwrap().is_some(),
            age_secs: now - s.created_at,
            window_secs: GATE_SCAN_WINDOW_SECS,
            escalations: seat.count,
            max_escalations: GATE_SCAN_MAX_ESCALATIONS,
        };
        // ★관측 창과 발행 천장은 **다른 축**이다(결함 4). 천장에 닿아도 관측은 계속하고
        //   발행만 멈춘다 — 그래야 관문이 화면에 실재하는 동안 배지가 살아 있고,
        //   관문이 사라지는 순간 같은 생명주기 경로(`scan_first_run_gate` 의 stale-clear)로
        //   종결된다. 천장은 `may_escalate` 로 그대로 남는다.
        let observe_open = gate_scan_observe_open(&window);
        let may_escalate = gate_scan_open(&window);
        if !observe_open && seat.count > 0 && !seat.cleaned {
            // **관측 자체가 닫힌** 좌석(각성 ack·생애 창 초과·롤백·미선언)만 종결한다. 그
            // 좌석은 화면을 더 보지 않으므로 배지를 살려 두면 영구 pending 으로 남는다.
            // 천장으로 닫힌 좌석은 여기 오지 않는다 — 그것이 이 수리의 전부다.
            for rid in pending_gate_items(daemon, s.id) {
                daemon.resolve_feed_item(&rid, "gate-window-closed");
            }
            caches.gate_escalations.insert(
                s.id,
                GateSeatState {
                    cleaned: true,
                    ..seat
                },
            );
        }
        if patterns.is_empty() && !observe_open {
            continue;
        }
        let patterns = &patterns;
        // 화면 스냅샷은 pass 당 좌석 1회 — 두 스캐너가 같은 스냅샷을 본다(파서 락 중복 0).
        let screen = s
            .parser
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .screen()
            .contents();
        let mut any_match = false;
        for p in patterns {
            let (Some(name), Some(pattern)) = (p["name"].as_str(), p["pattern"].as_str()) else {
                continue;
            };
            // ★선컴파일 캐시(무효화 규칙 = ScanCaches doc). 판정 자체는 종전과 동일하다 —
            //   같은 패턴 문자열의 Regex 는 결정론이므로 캐시 히트는 재컴파일과 등가다.
            let Some(m) = caches.approval_regex(pattern).and_then(|re| re.find(&screen)) else {
                continue;
            };
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
        if !any_match && !patterns.is_empty() {
            for rid in daemon.pending_daemon_approvals(s.id) {
                daemon.resolve_feed_item(&rid, "stale-cleared");
            }
        }
        // ─────────────────────────────────────────────────────────────────────
        // ★(U-16) 두 번째 스캐너 — 첫기동 관문. 위 approval 블록과 상태를 공유하지 않는다.
        //   본체를 **별도 함수**로 뽑아 둔 이유: "이 스캐너에는 키를 보내는 코드가 없다"는
        //   계약이 한 화면 안에서 감사돼야 하기 때문이다(자동응답 금지 · 리뷰 렌즈 분리).
        // ─────────────────────────────────────────────────────────────────────
        if observe_open {
            scan_first_run_gate(
                &GateScanCtx {
                    daemon,
                    surface: &s,
                    agent: &agent,
                    screen: &screen,
                    envelope,
                    override_on,
                    may_escalate,
                    now,
                },
                seat,
                caches,
            );
        }
    }
    caches.prune_pass(&seen_patterns, &seen_agents);
}

/// 관문 스캐너 입력의 전량(창 개폐는 호출부가 이미 판정했다 — 여기서 다시 열지 않는다).
struct GateScanCtx<'a> {
    daemon: &'a Arc<Daemon>,
    surface: &'a Arc<crate::state::Surface>,
    agent: &'a str,
    screen: &'a str,
    /// `agents.json` override 봉투(디스크 우선 · 부재 시 임베드) — `gate_envelope` 산출.
    envelope: Option<&'a serde_json::Value>,
    /// `first_run_gates` override 롤백 스위치의 pass 값.
    override_on: bool,
    /// **새 격상을 발행해도 되는가**(격상 천장 미도달). 관측 창과 다른 축이다 — 결함 4 참조.
    may_escalate: bool,
    now: f64,
}

/// ★(U-16) 첫기동 관문 스캐너 본체 — **감지·격상만 한다**.
///
/// ## 이 함수에 없는 것 (계약)
///
/// `write_tx`·`send_key`·`inject_text`·`first_run_gates::action_policy` 어느 것도 호출하지
/// 않는다. 통과 액션의 산출조차 하지 않는다 — [`GateSighting`] 에 키 재료가 없으므로
/// **없는 값은 보낼 수 없다**. 관문을 통과시키는 것은 자기 게이트를 가진 부트 경로(U-14/U-19)
/// 소관이고, 데몬 watchdog 틱이 그 일을 하면 면책 창의 `No, exit` 를 누르는 사고(2026-07-29)가
/// 좌석 전량으로 확대된다.
///
/// ## 틱 계약
///
/// 이 함수는 `.await` 를 하지 않는다(watchdog 틱의 유일한 await 는 상단 sleep 하나라는 계약).
/// 잡는 락은 전부 한 문장 안에서 끝난다 — feed·role·큐 락을 겹쳐 잡지 않는다.
fn scan_first_run_gate(cx: &GateScanCtx, seat: GateSeatState, caches: &mut ScanCaches) {
    let daemon = cx.daemon;
    let sid = cx.surface.id;
    let gates = caches.corpus(cx.agent, cx.envelope, cx.override_on);
    let Some(hit) = gate_sighting(&gates, cx.screen) else {
        // 화면에서 관문이 사라졌다 = 사람이 통과시켰다 → pending 항목 종결(생명주기).
        if seat.count > 0 {
            for rid in pending_gate_items(daemon, sid) {
                daemon.resolve_feed_item(&rid, "stale-cleared");
            }
        }
        return;
    };
    // ★격상 천장(결함 4) — 여기서 멈추는 것은 **발행**이지 관측이 아니다. 위 종결 분기를
    //   이미 지나왔으므로, 관문이 화면에 있는 동안 배지는 살아 있고 사라지면 종결된다.
    //   ★이 return 이 없으면 천장 뒤에도 새 항목이 계속 발행돼 천장이 사문이 된다(폭주 재개).
    //   그래서 순서가 계약이다: '관문 소실 종결' 이 먼저, '천장' 이 그 다음.
    if !cx.may_escalate {
        return;
    }
    // L3 코얼레싱 — 이 좌석의 관문 항목이 아직 pending 이면 같은 에피소드다(재발행 금지).
    if !pending_gate_items(daemon, sid).is_empty() {
        return;
    }
    let key = (sid, hit.id.clone());
    if caches
        .gate_debounce
        .get(&key)
        .map(|t| cx.now - t < GATE_SCAN_DEBOUNCE_SECS)
        .unwrap_or(false)
    {
        return;
    }
    caches.gate_debounce.insert(key, cx.now);
    caches.gate_escalations.insert(
        sid,
        GateSeatState {
            count: seat.count + 1,
            cleaned: false,
        },
    );
    let role = cx.surface.role.lock().unwrap().clone();
    let text = gate_escalation_text(&cys::surface_ref(sid), &hit);
    let agent = cx.agent;
    daemon.bus.publish(
        "boot_gate.detected",
        "feed",
        Some(sid),
        json!({"surface_ref": cys::surface_ref(sid), "role": role, "agent": agent,
               "gate": hit.id, "title": hit.title, "human_only": hit.human_only}),
    );
    daemon.push_feed_notification(
        GATE_FEED_KIND,
        &format!("{agent} 첫기동 관문 감지 ({})", cys::surface_ref(sid)),
        &text,
        Some(sid),
    );
    // master 각성은 approval 과 **같은 큐 경로**를 탄다(문구 dedupe 5분 · 큐 cap 100 ·
    // 조용시점 배달). ★approval 이 이미 이 좌석으로 master 를 깨웠다면 건너뛴다 —
    // 같은 pane 을 보라는 요구를 두 번 보내지 않는다(치명위험 ① 억제이며, 진단 자체는
    // feed 항목에 이미 남아 있으므로 잃는 정보가 없다).
    if !daemon.has_pending_daemon_approval(sid) {
        enqueue_master_wakeup(daemon, sid, &text);
    }
}

/// ★T-0147-2 층1 I4: 승인 wakeup 중복 억제 상태 — 문구 해시 → 마지막 적재 시각.
///
/// Daemon(state.rs)에 필드를 늘리지 않고 모듈 static 으로 두는 이유: 이 억제는 governance 의
/// approval 네임스페이스 안에서만 의미가 있고(설계 층3 '발행자 불변식' ② — pack 큐와 trigger 가
/// 겹치지 않으므로 전역 dedupe 가 아니라 자체 문구 dedupe 가 계약이다), 데몬 전역 상태로 승격하면
/// 재시작 영속·RPC 노출 같은 계약이 딸려온다. 여기 필요한 건 프로세스 수명의 5분 창뿐이다.
static APPROVAL_WAKEUP_RECENT: std::sync::OnceLock<std::sync::Mutex<HashMap<u64, f64>>> =
    std::sync::OnceLock::new();

fn approval_wakeup_recent() -> &'static std::sync::Mutex<HashMap<u64, f64>> {
    APPROVAL_WAKEUP_RECENT.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// 문구 지문. 전문 비교 대신 해시를 쓰는 이유는 창 맵이 감지 문구(수백 바이트 excerpt 포함)를
/// 그대로 붙들지 않게 하기 위함이다 — 충돌 시 결과는 '한 번 덜 깨움'이라 안전 방향이다.
fn approval_wakeup_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// ★T-0147-2 층1 I4 술어(순수 — 부작용0·테스트 핀). 승인 감지 에피소드마다 master stdin 을
/// 두드리던 경로에 창(기본 5분) dedupe 를 건다. 판정 근거 2종:
///   ① 직전 큐에 **동일 문구**가 아직 배달 대기 중 = 같은 사실을 두 번 읽힐 이유가 없다.
///   ② 창 안에 같은 문구를 이미 적재했다 = 배달돼 사라졌더라도 master 는 방금 그것을 봤다.
/// `window_secs <= 0.0` 이면 항상 false — 노브 비활성 시 **종전 무억제 동작**으로 정확히 되돌린다.
fn approval_wakeup_suppressed(
    queue: &std::collections::VecDeque<crate::state::QueueEntry>,
    recent: &HashMap<u64, f64>,
    text: &str,
    now: f64,
    window_secs: f64,
) -> bool {
    if window_secs <= 0.0 {
        return false;
    }
    // ★G1(W2-A): 원소가 QueueEntry로 승격돼도 dedupe는 **문구 단위** 의미 불변 —
    // id가 달라도 같은 문구는 억제된다(층1 I4 계약 유지).
    if queue.iter().any(|e| e.text == text) {
        return true;
    }
    recent
        .get(&approval_wakeup_hash(text))
        .is_some_and(|t| now - t < window_secs)
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
    // ★T-0147-2 층1 I4: pending_queue 삽입 **전** 문구 dedupe(기본 창 5분, 0=비활성).
    let window = env_u64("CYS_APPROVAL_WAKEUP_DEDUPE_SECS", 300) as f64;
    let now = now_epoch();
    let mut suppressed = false;
    let enqueued = {
        let mut q = s.pending_queue.lock().unwrap();
        // 락 순서: pending_queue → APPROVAL_WAKEUP_RECENT. 이 static 은 여기서만 잡히므로
        // 역순 획득자가 존재할 수 없다(데드락 무관).
        let mut recent = approval_wakeup_recent()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // 무한 성장 차단: 창의 2배를 넘긴 항목은 어떤 판정에도 쓰이지 않는다(24/365 데몬 누수).
        if window > 0.0 {
            recent.retain(|_, t| now - *t <= 2.0 * window);
        }
        if approval_wakeup_suppressed(&q, &recent, text, now, window) {
            suppressed = true;
            None
        } else if q.len() >= 100 {
            None // 큐 포화 — 종전대로 조용히 무시
        } else {
            // ★G1(W2-A): enqueue 3경로 중 governance 경로 — origin 태그로 관통 추적.
            // ★G1(W2-B): 이벤트에 항목 조준점(queue_entry_id/seq/enqueued_at)을 동봉하려
            // entry를 밖으로 넘긴다(발급-적재-발행의 항목 동일성 보장).
            let entry = daemon.next_queue_entry(text.to_string(), None, "governance-approval");
            q.push_back(entry.clone());
            recent.insert(approval_wakeup_hash(text), now);
            Some((entry, q.len()))
        }
    };
    if suppressed {
        // 침묵 금지: 억제도 관측 가능한 사실로 남긴다 — '깨우지 않았다'가 '사건이 없었다'로
        // 읽히면 wakeup 홍수를 고치다 무음 고장을 새로 만든 셈이 된다.
        daemon.bus.publish(
            "queue.suppressed",
            "queue",
            Some(master_sid),
            json!({"from": "governance-approval", "reason": "dup_within_window",
                   "window_secs": window}),
        );
        return;
    }
    // 적재 성공 시에만 queue.enqueued — 기존 발행 경로 무회귀(수락 증거의 의미 불변).
    // ★G1(W2-B): payload는 enqueue 3경로 공용 빌더(기존 키 bytes/depth/from 불변 + additive).
    let Some((entry, depth)) = enqueued else {
        return;
    };
    daemon.bus.publish(
        "queue.enqueued",
        "queue",
        Some(master_sid),
        crate::state::queue_enqueued_payload(&entry, depth, json!("governance-approval"), None),
    );
    daemon.persist_queue_state();
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
                    && crate::state::is_daemon_issued(&i.request_id)
            })
            .map(|i| i.request_id.clone())
            .collect();
        let st = items
            .iter()
            .filter(|i| {
                i.status == "pending"
                    && i.kind == "approval"
                    && crate::state::is_daemon_issued(&i.request_id)
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
    // ★폭 변환은 **넓히는 쪽**으로 한다. `threshold as usize` 는 32비트 타깃에서 좁힘이라
    //   `2^32` 가 0 이 되고, 위 `threshold == 0`(비활성) 검사를 이미 지난 뒤이므로
    //   `total >= 0` = **항상 참** → 백로그 경보가 상시 발화한다(P4-5 형제 결함).
    if total as u64 >= threshold {
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

/// `check_launch_flags` 의 **3상 판정** — 순수 함수(관측 결과 → 행동).
/// (`Hash` 는 검체가 순열별 판정을 집합으로 접어 **순서 불변성**을 단언하기 위한 것 — P3-1.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LaunchFlagAction {
    /// ① 관측 성공 ∧ 플래그 있음 — 정규 복귀. 1회 래치를 **재무장**한다.
    Rearm,
    /// ② 관측 성공 ∧ 플래그 없음 — 비정규 기동. 래치가 비어 있으면 **1회 발행**한다.
    Warn,
    /// ③ 관측 실패(`name()` 폴백) 또는 에이전트 자손 무관측 — **재무장도 발행도 하지 않는다**.
    Skip,
}

/// 좌석 자손 관측 → 기동 플래그 판정(순수).
///
/// 【고친 결함 P1-2(치명·폭주 채널)】 종전 판정은 2상이었다: 플래그가 보이면 `warned.remove`
/// (재무장), 안 보이면 발행. U-5 이전에는 관측 문자열이 **항상** `name()` 한 토큰이라
/// `cmdline.contains("--dangerously-skip-permissions")` 가 원리상 참이 될 수 없었고, 그래서
/// 재무장 가지가 죽어 있어 좌석당 1회로 끝났다. U-5 argv 승격 이후 그 값은 **argv 조회 성공
/// 여부에 따라 진동**한다(Windows `argv_snapshot` = `OpenProcess` + PEB `ReadProcessMemory`,
/// EDR·권한·종료 경주로 간헐 실패 → `name()` 폴백). 진동의 귀결:
///   성공 틱 → 재무장 → 실패 틱 → 플래그 미관측 → **이벤트 발행** → 성공 틱 → 재무장 → …
/// = watchdog 15초 주기(tick_no % 3) × 좌석 수만큼 **영구 발행**. 이 검사 주석 자신이
/// "2026-07-07 feed 폭주 재발방지"라고 적힌 자리에서 같은 폭주가 되살아났다.
///
/// 【수리의 축】 "플래그 부재"와 "관측 실패"를 **분리**한다. 부정 판정(=발행)은 argv 를 실제로
/// 읽은 관측에서만 내린다. 폴백 관측은 아무 상태도 바꾸지 않으므로(③) 진동이 래치에 닿지 못한다.
///
/// 【매칭 선택 규약】 자손 중 매처에 걸리는 것이 여럿일 수 있다. **argv 관측된 매치를 우선**
/// 채택하고, 그런 매치가 하나도 없을 때만 폴백 매치 유무를 본다 — 한 자손의 argv 조회 실패가
/// 다른 자손의 성공 관측을 덮지 않게 하기 위함이다.
///
/// 【고친 결함 P3-1(치명·폭주 채널 2호)】 그 규약을 구현이 지키지 않았다. 종전 코드는
/// `.find(|(_, c, src)| src == Argv && matches(c))` — 조건에 걸리는 **첫 하나**로 Rearm/Warn 이
/// 갈렸다. 그 "첫"의 순서는 `descendant_pids` 의 children 인덱스에서 나오고, 그 인덱스는
/// `sys.processes()` **HashMap 순회**로 채워진다 — 매 refresh 의 삽입·삭제로 순서가 바뀔 수
/// 있는, **우리가 통제하지 못하는 입력**이다. 한편 매처는 오살 방지를 위해 의도적으로 넓어
/// (토큰 basename + 경로 세그먼트) 좌석 자손에 매치가 **여럿** 생긴다:
///   Windows `powershell → cmd.exe(…\claude-2.cmd) → claude-2.exe` — 래퍼와 실물이 둘 다 매치
///   Unix    에이전트가 부른 `less ~/dev/claude/NOTES.md`·`which claude` 가 실물과 함께 매치
/// 두 프로세스의 argv 조회는 각각 독립적으로 성공/실패하므로 목록의 순서·가독성이 틱마다
/// 갈아탄다: 래퍼 먼저·무플래그 → Warn(발행) → 실물 먼저·플래그 → Rearm(래치 해제) → 다시
/// Warn(**재발행**) = 좌석당 15~30초마다 영구 발행. P1-2 가 막은 축(한 프로세스의 가독성
/// 진동)과 **다른 축**(여러 프로세스 사이의 순서)이라 P1-2 수리로는 닫히지 않았다.
///
/// 【수리의 축】 판정을 **∃ 의미(순서 무관)** 로 바꾼다 — 아래 세 줄이 곧 계약이다.
/// 우선순위는 `Rearm > Warn > Skip`: 플래그를 **가진** argv 매치가 하나라도 있으면 그 좌석은
/// 정규로 떠 있는 것이고(래퍼가 플래그를 재조립해 잃었는지는 좌석의 정규성과 무관하다),
/// 부정 판정(Warn=발행)은 **argv 로 읽은 매치가 전부 무플래그일 때만** 낸다.
/// 발행 방향으로 기우는 판정은 이 자리에서 곧 폭주이므로, 모호성은 침묵 쪽으로 접는다
/// (경고는 강제력 없는 조언이고 폭주는 재난이다 — 비용이 대칭이 아니다).
pub fn decide_launch_flag_action(
    descendants: &[(u32, String, CmdSource)],
    bin_base: &str,
) -> LaunchFlagAction {
    // argv 로 **실제 관측된** 매치만이 판정 근거다(폴백 문자열의 "플래그 없음"은 부정이
    // 아니라 미관측 — P1-2). 아래 두 술어는 모두 ∃ 이라 목록 순서에 의존하지 않는다.
    let mut saw_argv_match = false;
    for (_, cmdline, src) in descendants {
        if *src != CmdSource::Argv || !cmdline_matches_agent(cmdline, bin_base) {
            continue;
        }
        saw_argv_match = true;
        // ① ∃ argv매치 ∧ 플래그보유 — 정규 기동 확인. (조기 반환해도 ∃ 이므로 순서 무관)
        if cmdline.contains("--dangerously-skip-permissions") {
            return LaunchFlagAction::Rearm;
        }
    }
    if saw_argv_match {
        // ② ∃ argv매치 ∧ 전부 무플래그 — 관측된 부정. 여기서만 1회 발행한다.
        return LaunchFlagAction::Warn;
    }
    // ③ 폴백 매치만 있거나(관측 실패) 매치 자체가 없다(에이전트 자손 무관측) — 무행동.
    //    후자는 종전에도 `continue` 였으므로 거동 무변이다.
    //    ★전자(관측 실패)의 **연속 지속**은 P3-4 가 `launch_flag_unobservable` 로 따로 본다.
    LaunchFlagAction::Skip
}

/// 【P3-4】 이 관측이 **판정 불능**인가 — 매처에 걸리는 자손은 있는데 argv 를 하나도 읽지
/// 못했다(= `Skip` 중 '관측 실패' 쪽).
///
/// 【왜 필요한가】 `Skip` 은 P1-2 가 폭주를 막으려고 만든 **영구 침묵**이다. argv 조회가
/// *간헐* 실패하는 환경에서는 그 침묵이 정확히 옳다. 그러나 argv 조회가 **항상** 실패하는
/// 환경(Windows EDR 이 PEB `ReadProcessMemory` 를 전면 차단)에서는 `check_launch_flags` 가
/// 영원히 `Skip` 만 낸다 — 개정 전에는 (틀린 이유로) 경고가 떴고 지금은 아무 말도 안 한다.
/// 즉 **"감시자가 못 보고 있다"는 사실 자체가 관측되지 않는다**. 형태가 자가치유 전멸
/// (재난③)과 같다: 장치는 살아 있는데 신호가 0이라 죽은 것과 구별되지 않는다.
///
/// 【계약】 이 술어는 **판정을 바꾸지 않는다**(Skip 은 그대로 Skip). 오직 진단 이벤트의
/// 게이트일 뿐이며, 발행 유계성은 `decide_unobservable_report` 가 별도로 책임진다.
/// "매치가 아예 없는" 경우(에이전트 자손 무관측)는 **불능이 아니다** — 볼 것이 없는 것과
/// 못 보는 것은 다르고, 전자를 진단하면 빈 좌석마다 잡음이 난다.
pub fn launch_flag_unobservable(descendants: &[(u32, String, CmdSource)], bin_base: &str) -> bool {
    let mut saw_fallback_match = false;
    for (_, cmdline, src) in descendants {
        if !cmdline_matches_agent(cmdline, bin_base) {
            continue;
        }
        match src {
            // argv 를 하나라도 읽었으면 판정이 가능했다는 뜻 — 불능이 아니다.
            CmdSource::Argv => return false,
            CmdSource::NameFallback => saw_fallback_match = true,
        }
    }
    saw_fallback_match
}

/// 【P3-4】 연속 관측 실패 진단의 **유계 발행 판정**(순수). 반환 true = 지금 1회 발행.
/// 상태(`streak`·`last_emit`)는 호출자(watchdog 태스크 로컬)가 소유한다.
///
/// ★새 폭주를 만들지 않는 것이 이 함수의 존재 이유다. 유계성은 두 겹이다:
///   ① **연속** 임계 — 한 번이라도 관측에 성공하면(Rearm|Warn) streak 이 0으로 접힌다.
///      ∴ argv 조회가 *간헐* 실패하는 환경(P1-2 의 진동)은 임계에 도달하지 못한다.
///   ② **쿨다운** — 임계를 넘긴 뒤에도 좌석당 `cooldown` 초에 1건이 상한이다.
///      ∴ 영구 불능 환경에서도 발행률은 `1/cooldown` 으로 유계다(무한 루프 없음).
/// `threshold == 0` 은 비활성(킬스위치).
pub fn decide_unobservable_report(
    observed_now: bool,
    unobservable_now: bool,
    streak: &mut u32,
    last_emit: &mut f64,
    now: f64,
    threshold: u32,
    cooldown: f64,
) -> bool {
    if observed_now {
        // 관측 성공 = 감시자가 보고 있다. streak 과 쿨다운을 **함께** 재무장한다
        // (다음 실명 구간이 오면 임계를 새로 채워야 하고, 그때는 즉시 보고할 수 있어야 한다).
        *streak = 0;
        *last_emit = f64::NEG_INFINITY;
        return false;
    }
    if !unobservable_now {
        // 볼 대상이 없다(에이전트 자손 무관측) — 실명이 아니므로 streak 을 늘리지 않는다.
        *streak = 0;
        return false;
    }
    *streak = streak.saturating_add(1);
    if threshold == 0 || *streak < threshold {
        return false;
    }
    if now - *last_emit < cooldown {
        return false;
    }
    *last_emit = now;
    true
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
    blind: &mut HashMap<u64, (u32, f64)>,
) {
    // ★P3-4 진단 계측 파라미터. 기본 40틱 = 이 검사의 15초 주기로 **10분 연속 실명**,
    // 쿨다운 3600초 = 좌석당 시간당 1건 상한. 임계 0 = 킬스위치(비활성).
    let blind_ticks = env_u32("CYS_LAUNCH_FLAG_BLIND_TICKS", 40);
    let blind_cooldown = env_u64("CYS_LAUNCH_FLAG_BLIND_COOLDOWN_SECS", 3600) as f64;
    let now = now_epoch();
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
        // ★argv 승격(U-5): 이 검사는 정의상 **플래그 문자열**을 읽는다 — argv 갱신 없이는
        // `cmdline.contains("--dangerously-skip-permissions")` 가 원리상 참이 될 수 없어
        // 살아있는 모든 claude 노드에 node.nonstandard_launch 를 1회씩 발행했다.
        // 15초 주기(tick_no % 3) × claude meta 좌석 한정 — 승격 비용의 상한이 여기서 닫힌다.
        // ★P1-2: 판정은 3상이다(근거 전문은 `decide_launch_flag_action` 주석). 관측 실패를
        //   '플래그 없음'으로 읽으면 argv 조회 진동이 15초마다 이벤트를 재발행한다.
        let observed = collect_descendants_with_cmd_src(sys, s.pid);
        let action = decide_launch_flag_action(&observed, &bin_base);
        // ★P3-4: `Skip` 의 영구 침묵을 진단한다 — **판정은 건드리지 않는다**(아래 match 그대로).
        // 관측 실패가 N틱 연속이면 저빈도로 1회 알린다: "감시자가 못 보고 있다"는 사실 자체가
        // 관측되지 않는 상태(재난③ 형태)를 깨는 것이 목적이고, 유계성은
        // `decide_unobservable_report` 의 연속 임계 + 쿨다운 두 겹이 책임진다.
        {
            let entry = blind.entry(s.id).or_insert((0, f64::NEG_INFINITY));
            let observed_now = matches!(
                action,
                LaunchFlagAction::Rearm | LaunchFlagAction::Warn
            );
            let unobservable_now =
                !observed_now && launch_flag_unobservable(&observed, &bin_base);
            if decide_unobservable_report(
                observed_now,
                unobservable_now,
                &mut entry.0,
                &mut entry.1,
                now,
                blind_ticks,
                blind_cooldown,
            ) {
                let role = s.role.lock().unwrap().clone();
                daemon.bus.publish(
                    "node.launch_flag_unobservable",
                    "watchdog",
                    Some(s.id),
                    json!({"agent": agent, "role": role, "surface_ref": cys::surface_ref(s.id),
                           "blind_ticks": entry.0, "check_interval_secs":
                               WATCHDOG_INTERVAL_SECS * 3,
                           "note": "에이전트 자손의 argv 를 연속으로 읽지 못해 기동 플래그를 \
                                    판정할 수 없다(권한·EDR 차단 가능) — 이 좌석의 비정규 기동 \
                                    감시는 현재 무력 상태다"}),
                );
            }
        }
        match action {
            LaunchFlagAction::Rearm => {
                warned.remove(&s.id); // 정규 복귀 — 재무장
                continue;
            }
            // 관측 실패·에이전트 자손 무관측 — 래치를 건드리지 않는다(진동 차단).
            LaunchFlagAction::Skip => continue,
            LaunchFlagAction::Warn => {}
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
///
/// ★(P1) seat 토큰 **비편입 계약**: `Surface.seat_token` 은 topology.json 에 **절대 싣지
/// 않는다**(인메모리+PTY env 한정 — 필드 doc 참조). 영속하면 same-UID 절취 표면이 커지고,
/// restore 는 어차피 재생성(새 토큰)이라 회복 가치가 0이다. 이 함수는 필드를 손으로 골라
/// json! 조립하므로 '조립에 추가하지 않는 한' 배제가 기본값이다 — 아래 조립에 seat_token 을
/// 추가하는 변경은 계약 위반(회귀 핀 `seat_token_never_persisted_or_listed` 가 적색으로 잡는다).
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
                       // (W1) 원 계정 config_dir 영속 — restore가 이 값을 launch 문자열에 인라인해
                       // 데몬 env 변동에도 원 대화(.jsonl)로 정확히 재개한다. 구 topology(필드 없음)는
                       // 로드 시 None → 기존 동작(템플릿 전개)으로 하위호환.
                       "claude_config_dir": s.claude_config_dir.lock().unwrap().clone(),
                       "pack_reinject": s.pack_reinject.lock().unwrap().clone(),
                       // ★(W2 · B6) 각성 래치 영속 — 데몬 재시작 생존이 **필수**다(비평2 B-1).
                       // 인메모리 단독이면 재시작 직후 건강한 전 팀이 래치를 잃고, 부트 체인은
                       // '각성 확정'을 영영 못 본다(legacy-presumed 로 영구 강등 = 신호 무력화).
                       // restore 가 이 값을 surface.create 의 awakened_at 파라미터로 되돌린다.
                       // 구 topology(키 부재)는 로드 시 null → None(하위호환·단방향 계약 유지).
                       "awakened_at": *s.awakened_at.lock().unwrap(),
                       // ★(U-10) 관문 보류 **관측 슬롯** — 좌석 제4 등급을 재기동을 넘겨 사람이
                       // 읽을 수 있게 남긴다(치명위험 ③: '미완성 좌석을 정상으로 센다' 의 가시화).
                       // ★일부러 **하이드레이션하지 않는다**: restore 가 이 값을 되돌리면 stale
                       // 보류가 영속해 좌석이 영원히 미충족 → 부트 재시도 라이브락(A1 클래스)이다.
                       // 만료 규약과 함께 U-11 이 정할 사안이고, 이 단위는 슬롯만 만든다.
                       // 구 topology(키 부재)·킬스위치 off 는 null — 소비자는 항을 생략한다.
                       (cys::GATE_PENDING_KEY): s.gate_pending_wire()})
            })
        })
        .collect();
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
        "schema_version": 1,          // ★A-S1 스키마 마커 — 이 키 부재=legacy topology(phoenix 는 경고+진행)
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

// ─────────────────────────────────────────────────────────────────────────────
// ★G2(W3-A) 생존 판정 순수함수 — idle(침묵)과 death(프로세스 생존 실패)의 축 분리
// ─────────────────────────────────────────────────────────────────────────────

/// master.deadman v2 의 사망 판정 축. **침묵(silence)은 이 enum 에 없다** — 침묵이
/// 사망 사유가 될 수 없음을 타입으로 봉인한다(결함 8: 살아있는 zsh 침묵 → 사망 오라벨).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadmanAxis {
    /// roles 매핑은 있는데 surface 가 테이블에서 소멸(데몬 내부 사실 — Unknown 없음).
    SurfaceGone,
    /// surface 의 셸이 EOF 로 자력종료(exited 플래그 — 데몬 내부 사실).
    SurfaceExited,
    /// 셸 pid 커널 프로브 사망(exited 미설정 half-open — reap_zombie 와 같은 층의 관측).
    ShellProcDead,
    /// 좌석 빈사: 셸은 생존인데 에이전트만 죽음 — check_agent_death 의
    /// agent_seen/agent_exit_notified 상태머신(같은 틱 sysinfo 관측) 재사용.
    AgentDead,
    /// meta 부재 보조축(수동 기동 master 사각 봉인): role 좌석에서 **기지 에이전트 엄격
    /// 관측**(cmdline_matches_agent_exec)이 잡힌 적 있는데 지금 Empty ∧ agent_meta 부재 —
    /// 상태머신이 못 보는 에이전트 사망 후보. ★armed 경계는 원시 Occupied(아무 자손)가
    /// 아니다(BLOCK 교정) — vim/빌드 좌석의 프롬프트 복귀 오살 = 결함 8 동형 차단.
    SeatVacantNoMeta,
    /// 기동 즉사 쌍둥이 셀(리뷰 MAJOR): meta 있음 ∧ agent_seen=false(set_meta 리셋 후 첫
    /// sysinfo 관측 전 크래시 — 오타 플래그·바이너리 부재·auth 실패) ∧ seat=Empty 지속.
    /// check_agent_death 는 agent_seen 게이트로 영구 skip 이라 상태머신이 이 죽음을 영영 못
    /// 본다 — 좌석 Empty 지속이 유일한 관측 증거다(v1 은 900s 후 "master silent"로나마 울렸다).
    AgentNeverStarted,
}

impl DeadmanAxis {
    /// payload.axis 노출용 문자열(소비자는 name 키잉 — axis 는 additive 관측 필드).
    pub fn as_str(self) -> &'static str {
        match self {
            DeadmanAxis::SurfaceGone => "surface_gone",
            DeadmanAxis::SurfaceExited => "surface_exited",
            DeadmanAxis::ShellProcDead => "shell_proc_dead",
            DeadmanAxis::AgentDead => "agent_dead",
            DeadmanAxis::SeatVacantNoMeta => "seat_vacant_no_meta",
            DeadmanAxis::AgentNeverStarted => "agent_never_started",
        }
    }
}

/// role 좌석 생존 판정 결과 — 순수(부작용 0). Idle 은 Alive 의 하위 상태(생존 확정 + 침묵)라
/// death 카운터를 리셋하며, Unknown(프로브 미도달)은 '측정 불능 ≠ 사망 ≠ 생존'으로
/// 소비처가 무증감(카운트도 리셋도 없음)을 집행한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessVerdict {
    Alive,
    Unknown,
    Idle { idle_secs: u64 },
    DeadCandidate { axis: DeadmanAxis },
}

/// ★순수 판정자(이 파일의 기존 관례: T5-2 무음 크래시 술어·alerts::evaluate 와 동형) —
/// 부작용 0·테스트 핀 가능. 생존 권위는 데몬 내부 사실(exited)·커널 프로브·check_agent_death
/// 상태머신·seat_cache 재사용 — pgrep 금지 제약을 원천 충족.
///
/// 입력 계약:
/// - `shell_pid_alive`: **state::pid_alive 단일 정의처**(kill-0/OpenProcess 커널 프로브)로
///   호출자가 계산한다. ★도구 출력 충돌 시 권위: sysinfo 표(seat)와 이 값이 어긋나면
///   셸 생존은 pid_alive 가 이긴다(seat 는 좌석 보조축 판정에만 쓰인다).
/// - `seat_agent_seen`: 이 role 좌석(동일 sid)에서 **기지 에이전트 엄격 관측**
///   (seat_agent_cache — cmdline_matches_agent_exec 매칭)이 잡힌 적 있는가 — DeadmanTracker
///   가 기억(무meta 좌석의 agent_seen 판). false 면 '한 번도 에이전트가 없던 좌석'이라
///   Empty 여도 사망 후보가 아니다. ★원시 Occupied(vim/빌드 등 아무 자손) 관측은 armed
///   근거가 아니다(BLOCK 교정) — 맨 셸 오살 = 결함 8 동형 재발 차단의 핵심 경계.
/// - `never_seen_grace_expired`: meta 있음 ∧ agent_seen=false 상태가 grace 를 소진했는가 —
///   DeadmanTracker 가 상태 최초 관측 시각으로 계산(기동 즉사 쌍둥이 셀의 부트 창 보호).
///
/// 진리표(위에서 아래로 첫 매치):
///   surface 부재                            → DeadCandidate(SurfaceGone)
///   exited                                  → DeadCandidate(SurfaceExited)
///   !shell_pid_alive                        → DeadCandidate(ShellProcDead)
///   meta ∧ agent_seen ∧ agent_exit_notified → DeadCandidate(AgentDead)
///   meta ∧ !agent_seen ∧ grace소진 ∧ Empty   → DeadCandidate(AgentNeverStarted)
///   meta ∧ !agent_seen ∧ grace소진 ∧ Unknown → Unknown (프로브 미도달 — 무증감)
///   무meta ∧ armed ∧ seat=Empty             → DeadCandidate(SeatVacantNoMeta)
///   무meta ∧ armed ∧ seat=Unknown           → Unknown (프로브 미도달 — 무증감)
///   idle_secs ≥ threshold(>0)               → Idle    (침묵은 여기서 끝 — death 불가)
///   그 외                                   → Alive
pub fn liveness_verdict(
    surface: Option<&crate::state::Surface>,
    seat: SeatState,
    shell_pid_alive: bool,
    seat_agent_seen: bool,
    never_seen_grace_expired: bool,
    idle_threshold_secs: u64,
) -> LivenessVerdict {
    let Some(s) = surface else {
        return LivenessVerdict::DeadCandidate {
            axis: DeadmanAxis::SurfaceGone,
        };
    };
    if s.exited.load(Ordering::Relaxed) {
        return LivenessVerdict::DeadCandidate {
            axis: DeadmanAxis::SurfaceExited,
        };
    }
    if !shell_pid_alive {
        return LivenessVerdict::DeadCandidate {
            axis: DeadmanAxis::ShellProcDead,
        };
    }
    let has_meta = s.agent_meta.lock().unwrap().is_some();
    if has_meta {
        // check_agent_death 가 같은 틱에 sysinfo 로 갱신한 상태머신 재사용(판정 이원화 금지).
        let seen = s.agent_seen.load(Ordering::Relaxed);
        if seen && s.agent_exit_notified.load(Ordering::Relaxed) {
            return LivenessVerdict::DeadCandidate {
                axis: DeadmanAxis::AgentDead,
            };
        }
        // ★기동 즉사 쌍둥이 셀(리뷰 MAJOR): set_meta 는 agent_seen=false 로 리셋한다 —
        // 에이전트가 첫 sysinfo 관측 전에 죽으면(오타 플래그·바이너리 부재·auth 실패)
        // check_agent_death 는 agent_seen 게이트로 영구 skip 이라 AgentDead 축이 영영 못
        // 밟힌다. 좌석 Empty 지속이 유일한 증거다. grace(never-seen 상태 나이) 소진 후에만
        // 후보 — 정상 기동(스폰 지연) 창은 grace 가 보호한다. Occupied 는 무판정 통과
        // (기동 중 래퍼 프로세스 가시 — 다음 틱 check_agent_death 가 seen 을 확정한다).
        if !seen && never_seen_grace_expired {
            match seat {
                SeatState::Empty => {
                    return LivenessVerdict::DeadCandidate {
                        axis: DeadmanAxis::AgentNeverStarted,
                    }
                }
                SeatState::Unknown => return LivenessVerdict::Unknown,
                SeatState::Occupied => {}
            }
        }
    } else if seat_agent_seen {
        match seat {
            SeatState::Empty => {
                return LivenessVerdict::DeadCandidate {
                    axis: DeadmanAxis::SeatVacantNoMeta,
                }
            }
            SeatState::Unknown => return LivenessVerdict::Unknown,
            SeatState::Occupied => {}
        }
    }
    // ★침묵은 Idle 로만 사상된다 — DeadCandidate 사유가 될 수 없음이 이 함수의 존재 이유.
    let idle_secs = s.last_output.lock().unwrap().elapsed().as_secs();
    if idle_threshold_secs > 0 && idle_secs >= idle_threshold_secs {
        return LivenessVerdict::Idle { idle_secs };
    }
    LivenessVerdict::Alive
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

/// ★G2(W3-A BLOCK 교정)의 **생산자측 판정 하나**를 순수 함수로 노출한다 — 좌석 자손 cmdline
/// 목록이 '기지 에이전트 관측'으로 셈해지는가.
///
/// 왜 별도 함수인가(테스트 가능성 계약): refresh_seat_cache 는 `&System`(실 프로세스 표)을
/// 받으므로 단위 테스트가 구동할 수 없고, 데드맨 테스트는 전부 `seat_agent_cache` 를 직접
/// store 해 **소비측**만 고정한다. 그 결과 '생산자가 그 값을 무엇으로 만드는가'가 무핀이었고,
/// 실제로 이 자리를 `true` 로 치환하는 mutation(= BLOCK 교정 이전의 원시 Occupied armed)이
/// 전체 테스트를 통과했다. 판정을 여기로 끌어내 행위 테스트로 못박는다.
///
/// 계약: **엄격 매칭**(cmdline_matches_agent_exec 계열 select_observed_agent)만 관측으로
/// 인정한다. vim/less/tail/빌드 등 비에이전트 자손은 관측이 아니다 — 그것을 관측으로 치면
/// 자손 1틱 관측 후 프롬프트 복귀(Empty)가 살아있는 맨 셸 좌석을 사망 후보로 오라벨한다
/// (결함 8 동형). 후보 목록이 비면 false(보조축 조용히 off · fail-closed).
pub(crate) fn seat_agent_observed(cmds: &[String], candidates: &[(String, String)]) -> bool {
    if candidates.is_empty() {
        return false;
    }
    select_observed_agent(cmds, candidates).is_some()
}

/// ★SEAT 캐시 갱신 — **단일 writer**(watchdog 틱). 판정 재료(전 프로세스 표)를 이미 refresh 한
/// 지점에서 한 번만 계산해 캐시에 싣는다. RPC 읽기 경로(surface.list·status·deliver_queued)는
/// 재조회 없이 이 값을 소비한다(비용 중복 0).
pub fn refresh_seat_cache(daemon: &Arc<Daemon>, sys: &System) {
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    // ★G2(W3-A BLOCK 교정) 기지 에이전트 후보는 틱당 1회 지연 로드(agents.json 파일 IO 절약 —
    // 무meta Occupied 좌석이 하나도 없는 틱은 IO 0).
    let mut candidates: Option<Vec<(String, String)>> = None;
    for s in surfaces {
        let seat = seat_state(sys, &s);
        s.seat_cache.store(seat.as_u8(), Ordering::Relaxed);
        // ★G2(W3-A BLOCK 교정) 좌석 에이전트 엄격 관측: meta 부재 보조축(SeatVacantNoMeta)의
        // armed 경계는 '아무 자손'(원시 Occupied)이 아니라 **기지 에이전트 엄격 매칭**
        // (cmdline_matches_agent_exec — R2 확정 strict 매처·select_observed_agent 재사용)이다.
        // 원시 Occupied 로 armed 하면 vim/less/tail/빌드 등 비에이전트 자손 1틱 관측만으로
        // 살아있는 맨 셸 좌석의 프롬프트 복귀(Empty)가 사망 후보로 오라벨된다 — 결함 8 동형.
        // strict ⊆ broad(생존 매처)라 '관측 armed 됐는데 생존 매처가 못 보는' 비대칭 신설 없음.
        // 무meta 좌석 한정(meta 좌석은 check_agent_death 의 agent_seen 상태머신이 담당).
        // 후보 로드 실패(agents.json 파싱 불가)=빈 목록=미관측 — 보조축이 조용히 꺼진다
        // (fail-closed · known_agent_candidates 의 기존 규약과 동일).
        let observed = seat == SeatState::Occupied
            && s.agent_meta.lock().unwrap().is_none()
            && {
                let cands = candidates.get_or_insert_with(known_agent_candidates);
                !cands.is_empty() && {
                    // ★argv 승격(U-5): 이 자리의 계약이 명시적으로
                    // "cmdline_matches_agent_exec 엄격 매칭"이다(위 주석) — 이름 한 토큰으로는
                    // 그 계약이 성립하지 않는다. 범위는 meta 부재·Occupied 좌석의 자손만이라
                    // 틱당 승격 대상이 구조적으로 소수다.
                    let cmds: Vec<String> = collect_descendants_with_cmd(sys, s.pid)
                        .into_iter()
                        .map(|(_, cmd)| cmd)
                        .collect();
                    seat_agent_observed(&cmds, cands)
                }
            };
        s.seat_agent_cache.store(observed, Ordering::Relaxed);
    }
}

/// ★SEAT 승계 봉쇄 판정 — 이 좌석의 agent_meta 가 승계를 막아야 하는가.
///
/// **죽은 에이전트의 meta 는 봉쇄하지 않는다**(2026-08-12 R3 확정 · 계약 개정): 관측 기반
/// 등록(claim_role_probe)이 선언 master 전원에 meta 를 심은 뒤로, "meta 존재=무조건 봉쇄"는
/// master 의 CLI 가 죽은 좌석을 영구 봉쇄해 — 오너의 재선언(직전 릴리스까지 좌석 승계로
/// 성공하던 제스처)이 claim_denied → 부서 자동 창설로 격상되는 회귀를 낳았다. 사망감지
/// 상태머신이 이미 진실을 안다: agent_exit_notified=true(죽음 관측·복귀 시 자동 리셋)면
/// 그 meta 는 죽은 좌석의 기록이지 살아있는 점유가 아니다. node-recover 부활과의 경합은
/// 오너 명시 제스처(재선언·launch-agent)가 이긴다 — 부활은 역할 선점 시 정중히 물러난다.
/// meta 있음 ∧ 죽음 미관측(살아있거나 미상)은 종전대로 봉쇄(fail-closed).
fn meta_blocks_seat(s: &crate::state::Surface) -> bool {
    let has_meta = s.agent_meta.lock().unwrap().is_some();
    has_meta && !s.agent_exit_notified.load(Ordering::Relaxed)
}

/// ★SEAT 2차(승계 정책): 이 좌석의 특권 role 을 다른 surface 가 가져가도 되는가.
/// 커널 사실이 Empty 이고 + 살아있는 agent_meta 부재(죽은 에이전트의 meta 는 봉쇄하지 않는다 —
/// meta_blocks_seat) + 최근 사람 입력 없음(사용자가 지금 claude 를 띄우려 타이핑 중일 수 있다)
/// 셋을 **모두** 만족할 때만 true. Unknown 은 false(현행=거부 유지).
pub fn seat_claimable(sys: &System, s: &crate::state::Surface) -> bool {
    if seat_state(sys, s) != SeatState::Empty {
        return false;
    }
    if meta_blocks_seat(s) {
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

/// ★(T-0147-7 W2 · CS-5① / 비평2 C-4) **live-slot 계약** — latest-wins 의 agent_alive 한정 보호.
///
/// 종전 계약: 비특권 역할(reviewer-* 등)은 `roles.insert` 의 **latest-wins**(최신 surface 승리 —
/// state.rs:513·1881 명문). 그 결과 살아 일하는 리뷰어의 역할 주소를 새 pane 이 조용히 빼앗아
/// `--to reviewer-codex` 라우팅·알림·deadman 감시가 끊길 수 있다.
///
/// **전면 제거는 금지다**(금지 방향 ⑤): latest-wins 는 사실상의 self-heal 경로다 — 행 걸린 리뷰어를
/// "새로 띄우면 승리"로 회수하는 유일한 무마찰 수단이고, 그걸 없애면 B1/B2 가 고치려는 '리뷰어 영구
/// 결손'을 다른 문으로 재도입한다. 그래서 보호 범위를 **agent_alive 좌석 하나로 한정**한다:
///   agent_alive = agent_meta 등록됨 ∧ agent_seen ∧ ¬agent_exit_notified
///   → 이 셋을 모두 만족하는 좌석만 보호(=탈취 거부). 죽은·행 걸린·메타 없는 좌석은 **현행 유지**.
///
/// 반환: true = 이 좌석은 살아있는 에이전트가 점유 중(보호 대상).
pub fn slot_agent_alive(s: &crate::state::Surface) -> bool {
    if s.exited.load(Ordering::Relaxed) {
        return false;
    }
    let has_meta = s.agent_meta.lock().unwrap().is_some();
    has_meta
        && s.agent_seen.load(Ordering::Relaxed)
        && !s.agent_exit_notified.load(Ordering::Relaxed)
}

/// ★(T-0147-7 W2 · G13/G14) 좌석 승계 **임계영역 내 저비용 재검증**.
///
/// **왜 필요한가**: `seat_claimable_now` 는 전 프로세스 표를 refresh 하므로 반드시 락 **밖**에서
/// 돈다(락 보유 중 수십 ms = 데몬 전체 정지). 그 결과 '프로브 → 임계영역 진입' 사이에 창이 열려
/// 있고, 종전에는 그 창에서 재검증이 **0**이었다(재감사 G13/G14): 프로브 후에 사용자가 그 pane 에
/// claude 를 띄우거나(자손 생성) 타이핑을 시작해도 승계가 그대로 진행돼, **살아있는 좌석의 역할을
/// 빼앗아** 알림·라우팅·deadman 감시를 끊었다.
///
/// **왜 이 함수는 저비용인가**: 프로세스 표를 다시 훑지 않는다. 승계 직전 창에서 **바뀔 수 있는
/// 값싼 사실**만 다시 본다 —
///   ① `exited`        : 그 사이 종료됐다(승계 대상이 아니라 정리 대상 — 다른 경로가 처리한다).
///   ② `agent_meta`    : 그 사이 에이전트가 등록됐다(= 사람이 CLI 를 띄웠다. 죽은 에이전트의 좌석은
///                       node-recover 영역이지 탈취 대상이 아니다 — seat_claimable 의 동일 규약).
///   ③ `last_human_input`: 그 사이 사람이 타이핑했다(사용자가 지금 그 pane 을 쓰고 있다).
///   ④ `seat_cache`    : watchdog 이 그 사이 Occupied 로 갱신했다(커널 사실이 값싸게 도착한 경로).
/// 하나라도 걸리면 **승계를 취소**한다(TOCTOU 를 원리상 소거할 수는 없지만, 창을 '락 진입 1회'로
/// 좁히고 값싼 반증을 모두 소진한다 — reclaim 의 kill-직전 재조회와 동형 설계).
///
/// 반환: `None`=승계 계속 / `Some(사유)`=승계 취소.
pub fn seat_takeover_recheck(s: &crate::state::Surface) -> Option<&'static str> {
    if s.exited.load(Ordering::Relaxed) {
        return Some("프로브 후 좌석이 종료됨(승계 대상 아님 — reap 경로가 처리)");
    }
    // ★죽은 좌석 승계 개정(2026-08-12)과 짝: 프로브 시점에 이미 있던 '죽은 에이전트의 meta'
    //   (exit_notified=true)는 취소 사유가 아니다 — 그것까지 취소하면 seat_claimable 개정이
    //   이 재검증에서 전부 무효화된다. 살아있는(또는 미상) meta 만 '그 사이 CLI 기동'의 증거다.
    if meta_blocks_seat(s) {
        return Some("프로브 후 살아있는 agent_meta 관측(사람이 CLI 를 띄웠다 — node-recover 영역)");
    }
    let human_recent = s
        .last_human_input
        .lock()
        .unwrap()
        .map(|t| t.elapsed().as_secs() < queue_human_quiet_secs())
        .unwrap_or(false);
    if human_recent {
        return Some("프로브 후 사람 입력 관측(사용자가 그 pane 을 사용 중)");
    }
    if SeatState::from_u8(s.seat_cache.load(Ordering::Relaxed)) == SeatState::Occupied {
        return Some("프로브 후 좌석 캐시가 Occupied 로 갱신됨(자손 프로세스 생성)");
    }
    None
}

/// Walk the process table and collect all descendants of `root`.
/// 에이전트 생존 매칭 — cmdline의 어느 토큰이든 ①basename 정확 일치 ②`.js` 번들 일치
/// (`…/gemini.js`) ③경로 세그먼트 일치(`…/gemini/…` 또는 `…/gemini-cli/…` 패키지 경로)면
/// 생존으로 본다. 구(舊) 규칙(앞 3토큰 제한 + basename 단일 일치)은 npm 래퍼 에이전트
/// (`node --옵션 …/@google/gemini-cli/bundle/gemini.js`)를 놓쳐 agent_alive=false 오판 →
/// orchestra check 상시 FAIL → 멀쩡한 노드 수선·오살(quit·close) 연쇄를 낳았다
/// (2026-06-12 실측). false-negative(오살)가 false-positive보다 훨씬 위험하므로 매칭을
/// 넓힌다 — 검사 범위는 어차피 해당 surface의 자손 프로세스로 한정된다.
/// Windows 실행 확장자 화이트리스트 — PATHEXT의 실행 가능 형태 중 에이전트 기동에 실제로
/// 쓰이는 것만. ★임의 확장자 strip 금지: 목록을 열면 `claude.backup` 같은 무관 파일이
/// 생존으로 오판돼 죽음 은폐(node-recover 거부)로 번진다.
const WIN_EXEC_EXTS: [&str; 5] = ["cmd", "bat", "exe", "ps1", "com"];

/// basename에서 Windows 실행 확장자 **1개만** 제거한다. 반환 `(본체, strip 발생 여부)`.
/// 확장자 판정 자체는 대소문자 무시(Windows 파일시스템 규약: `.CMD` == `.cmd`).
/// ★두 번째 반환값이 곧 "이 토큰은 Windows 실행 표기다"라는 증거다 — 호출부는 이때만
/// 본체 비교를 대소문자 무시로 완화한다. 확장자 없는 bare 토큰은 대소문자를 보존해
/// 유닉스 의미(`Claude` ≠ `claude`, 서로 다른 파일)를 지킨다.
/// `.js`는 여기 없다 — 번들 특례는 호출부의 기존 `.js` 규칙이 그대로 담당한다(의미 불변).
fn strip_win_exec_ext(base: &str) -> (&str, bool) {
    match base.rfind('.') {
        // idx>0: `.cmd` 같은 도트파일(본체가 빈 문자열)은 strip 대상이 아니다.
        Some(idx) if idx > 0 => {
            let ext = &base[idx + 1..];
            if WIN_EXEC_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)) {
                (&base[..idx], true)
            } else {
                (base, false)
            }
        }
        _ => (base, false),
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
///
/// ★확장자 정규화(2026-07-29 현장 결함 2호 실측): Windows에서 개명 래퍼로 기동하면 트리가
/// `powershell → cmd.exe(…\claude-2.cmd) → claude.exe`가 되는데, 등록 bin_base는 확장자 없는
/// `claude-2`라 어느 토큰과도 일치하지 않았다 → agent_alive 영구 false → boot_node reclaim()이
/// `taskkill /T`로 멀쩡한 pane을 오살. 비교 직전 **토큰 basename과 bin_base**에서 등록 실행
/// 확장자 1개를 벗겨 같은 지평에 세운다. 확장자가 이미 양측에 있거나 둘 다 없는 기존 경로는
/// 정규화 결과가 동일하므로 판정이 바뀌지 않는다(무회귀).
///
/// ★정규화 반경은 **basename 한정**이다 — 경로 세그먼트에는 적용하지 않는다(codex R1 major①).
/// 세그먼트까지 벗기면 `node C:\work\claude.cmd\helper.js` 같은 *무관 디렉터리*가 생존 증거로
/// 승격돼 죽음 은폐(node-recover 거부)를 낳는다. 디렉터리명은 실행 파일명이 아니다.
///
/// ★지원 계약(codex R1 missing④): 매칭 대상은 **관측된 프로세스 cmdline**이지 기동 표기
/// (alias·`.lnk` 바로가기·셸 함수)가 아니다. alias나 `.lnk`로 기동해도 실행 후에는 해석된
/// 실물 실행 파일이 cmdline에 나타나므로 토큰 매칭으로 충분하다. cmdline에 실물이 끝내
/// 나타나지 않는 기동 형태는 이 함수의 지원 밖이며, 그 경우 생존 판정은 다른 근거를 써야 한다.
pub fn cmdline_matches_agent(cmdline: &str, bin_base: &str) -> bool {
    if bin_base.is_empty() {
        return false;
    }
    // 기대 이름도 같은 정규화를 거쳐야 대칭이 된다(`claude.exe` 등록 vs `claude` 토큰).
    let (want, want_win) = strip_win_exec_ext(bin_base);
    if want.is_empty() {
        return false;
    }
    // 패키지 세그먼트는 `<bin>-cli`·`<bin>-code` 정확 일치만(실존 npm 패키지명:
    // @google/gemini-cli·@anthropic-ai/claude-code) — `<bin>-` 접두 전체를 열면
    // claude-code-router·grok-1-weights 같은 무관 경로가 생존으로 오판된다(적대 검증 R1:
    // 죽음 은폐 → node-recover 거부의 역결함).
    // ★패키지명은 **원형 bin_base**에서 파생한다(정규화 前) — 세그먼트 규칙 전체를 수정 전
    // 원형으로 유지하기 위함이다. 차분 실측(2026-07-29): 여기서 정규화값 want 를 쓰면
    // bin_base=`claude.exe`·세그먼트 `claude.exe` 조합이 True→False 로 뒤집혀(216건)
    // 생존 인지를 잃는다 = 오살 방향 회귀. 세그먼트는 손대지 않는 것이 정답이다.
    let pkg_cli = format!("{bin_base}-cli");
    let pkg_code = format!("{bin_base}-code");
    cmdline.split_whitespace().any(|tok| {
        let base = tok.rsplit(['/', '\\']).next().unwrap_or(tok);
        let (tok_base, tok_win) = strip_win_exec_ext(base);
        // ★본체 대소문자 무시는 **Windows형 토큰에 한정**(codex R1 major②): 어느 한쪽이라도
        // 등록 실행 확장자를 실제로 벗은 경우에만 완화한다. 대소문자 무구분은 Windows
        // 파일시스템의 성질이지 유닉스의 성질이 아니므로, bare 토큰끼리는 정확 비교를 유지해
        // 유닉스에서 `Claude`와 `claude`가 뭉개지지 않게 한다.
        let name_hit = if tok_win || want_win {
            tok_base.eq_ignore_ascii_case(want)
        } else {
            tok_base == want
        };
        if name_hit || base.strip_suffix(".js").is_some_and(|b| b == want) {
            return true;
        }
        // 경로 세그먼트 매칭은 실제 경로 토큰에서만 (단어 인자 오탐 방지).
        // ★세그먼트는 **무정규화 원형** — strip도, 대소문자 완화도, bin_base 정규화도 없다.
        // 비교 대상이 want 가 아니라 bin_base 인 것에 유의(위 pkg_cli 주석의 216건 근거).
        tok.contains(['/', '\\'])
            && tok
                .split(['/', '\\'])
                .any(|seg| seg == bin_base || seg == pkg_cli || seg == pkg_code)
    })
}

/// ★등록 전용 엄격 매처(2026-08-12 R2 확정 — governance FP 교정).
///
/// 생존판정 매처(`cmdline_matches_agent`)는 오살(false-negative) 방지를 위해 **의도적으로
/// 넓다** — 경로 세그먼트(`…/claude/…`)까지 생존 증거로 승격한다. 그 비용 부호가 **등록**에서는
/// 뒤집힌다: Linux(argv 가시 플랫폼)에서 `tail -f /home/u/claude/dev.log` 나
/// `vim /home/u/proj/claude-code/README.md` 가 도는 agent 없는 pane 을 claim 하는 순간, 세그먼트
/// 매칭이 meta=(claude,claude)·agent_seen=true 를 **오등록**하고 topology 에 영속시켜 콜드부트가
/// 엉뚱한 CLI 를 부활시킨다(무기록이 오기록보다 낫다는 관측 등록 원칙 위반).
///
/// 등록은 좁게 본다: ①토큰 basename 정확 일치(win 확장자 정규화 동일) ②`.js` 번들 basename
/// 일치(`…/gemini.js`) ③npm 패키지 세그먼트(`<bin>-cli`/`<bin>-code`)는 **실행 스크립트 토큰
/// (basename 이 `.js` 로 끝나는 경로)에서만** — `node …/@anthropic-ai/claude-code/cli.js` 는
/// 잡고, `vim …/claude-code/README.md` 는 버린다.
///
/// 비대칭 안전성: 이 매처는 생존 매처의 **부분집합**이다(strict ⊆ broad). 위험한 방향의
/// 비대칭은 "등록은 됐는데 사망감지가 못 보는" 쪽(등록 broad·생존 strict)이고, 그 반대인
/// 이 구성에서는 등록된 좌석을 사망감지가 반드시 본다 — 오살 경로 신설 없음.
pub fn cmdline_matches_agent_exec(cmdline: &str, bin_base: &str) -> bool {
    if bin_base.is_empty() {
        return false;
    }
    let (want, want_win) = strip_win_exec_ext(bin_base);
    if want.is_empty() {
        return false;
    }
    let pkg_cli = format!("{bin_base}-cli");
    let pkg_code = format!("{bin_base}-code");
    cmdline.split_whitespace().any(|tok| {
        let base = tok.rsplit(['/', '\\']).next().unwrap_or(tok);
        let (tok_base, tok_win) = strip_win_exec_ext(base);
        let name_hit = if tok_win || want_win {
            tok_base.eq_ignore_ascii_case(want)
        } else {
            tok_base == want
        };
        if name_hit || base.strip_suffix(".js").is_some_and(|b| b == want) {
            return true;
        }
        // 패키지 세그먼트는 실행 스크립트 경로(basename 이 .js)에서만 — 문서·로그 등
        // 데이터 파일 인자의 디렉터리명은 실행 증거가 아니다.
        base.ends_with(".js")
            && tok.contains(['/', '\\'])
            && tok
                .split(['/', '\\'])
                .any(|seg| seg == pkg_cli || seg == pkg_code)
    })
}

/// 자손 pid 트리만 수집한다(문자열 미조회) — collect_descendants 계열의 공통 골격.
/// pid 재사용으로 부모 링크에 사이클이 생겨도 무한루프하지 않게 방문 집합을 유지한다.
/// 반환 순서는 종전 collect_descendants 의 DFS 순서와 동일하다(소비자 순서 의존 무변경).
fn descendant_pids(sys: &System, root: u32) -> Vec<u32> {
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
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    seen.insert(root);
    while let Some(p) = stack.pop() {
        if let Some(kids) = children.get(&p) {
            for &kid in kids {
                if !seen.insert(kid) {
                    continue;
                }
                out.push(kid);
                stack.push(kid);
            }
        }
    }
    out
}

/// 프로세스 표에 **이미 실린 사실만으로** 만드는 관측 문자열.
/// argv 가 비면 종전대로 `name()` 한 토큰으로 접힌다.
fn observed_cmdline(sys: &System, pid: u32) -> String {
    sys.process(Pid::from_u32(pid))
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
        .unwrap_or_default()
}

/// 자손 프로세스 (pid, 관측문자열) 목록 — **argv 미승격판**.
///
/// ★계측 사실(sysinfo 0.33.1 `src/common/system.rs:291-305`): `System::refresh_processes`
/// 는 `ProcessRefreshKind::nothing().with_memory().with_cpu().with_disk_usage()
/// .with_exe(OnlyIfNotSet)` 로 위임하며 **`cmd`(argv) 를 갱신하지 않는다**. 그래서 그렇게
/// 갱신된 표에서 `Process::cmd()` 는 항상 비고, 이 함수의 반환값은 사실상 "명령줄"이
/// 아니라 **실행 파일 이름 한 토큰**이다(예: Windows 래퍼 기동 시 `node.exe`).
/// ∴ argv 를 실제로 **판정에 쓰는** 지점은 `collect_descendants_with_cmd` 를 써야 한다.
/// 개수·존재 여부만 보는 소비자(seat_state·reap 계열)는 이 함수가 맞다(추가 비용 0).
pub fn collect_descendants(sys: &System, root: u32) -> Vec<(u32, String)> {
    descendant_pids(sys, root)
        .into_iter()
        .map(|kid| {
            let cmdline = observed_cmdline(sys, kid);
            (kid, cmdline)
        })
        .collect()
}

/// 자손 프로세스 (pid, **명령줄**) 목록 — argv 승격판.
///
/// **왜 전역 refresh 를 승격하지 않고 별도 함수인가**
///
/// ① **의미(결정적 이유)** — 전역 표의 `cmd` 를 채우면 같은 표를 읽는 **모든** 소비자의 관측
///    문자열이 동시에 바뀐다. 그중 `check_surfaces` 의 cmdline 은 무엇인가를 판별하는
///    식별자가 아니라 동일물을 세는 **그룹핑 키**이고, 그 그룹이 자동 kill 대상 집합을 정한다
///    (plan_duplicate_alerts → `auto_kill`). 즉 전역 승격은 관측 정확도 수리가 아니라
///    **자동 kill 폭발 반경의 재정의**를 겸하게 된다 — 이 단위는 거기에 손대지 않는다.
///    트리 구성은 호출자가 이미 가진 `sys`(전역 refresh 결과)에서 하고 argv 만 **격리
///    스냅샷**으로 조회하므로, 같은 틱의 다른 소비자의 관측 의미는 그대로다.
///
/// ② **비용(실측 2026-08-23 · macOS · 프로세스 1,074개 · bench_argv_promotion_* 재현 가능)** —
///    전역 refresh 현행 kind 3.3~3.9 ms/회, 거기에 `with_cmd(OnlyIfNotSet)` 를 얹어도
///    3.4~3.5 ms/회로 **macOS 에서는 유의 증가가 관측되지 않았다**(macOS 는 `with_exe` 때문에
///    이미 `KERN_PROCARGS2` 를 읽고 있어 argv 파싱이 덤이다). ∴ "전역 승격은 비싸다"는
///    macOS 실측으로 지지되지 않으므로 근거로 쓰지 않는다 — 근거는 ①이다.
///    Linux(`/proc/<pid>/cmdline` 별도 open)·Windows(프로세스별 PEB 원격 읽기)는 미측정.
///
/// ③ **이 함수 자신의 비용** — 좌석 5개 기준 틱당 +1.2 ms(전역 refresh 대비 ~35%,
///    틱 주기 5,000 ms 의 0.024%). macOS 의 `refresh_processes_specifics(Some(..))` 는 내부에서
///    `proc_listallpids` 전수 열거를 먼저 하므로(sysinfo 0.33.1 `unix/apple/system.rs:250-278`)
///    **호출 1회당 열거 1회**다 — 좌석 수에 선형이다. 좌석이 수십 개로 커지면 좌석별 호출을
///    틱당 배치 스냅샷 1회로 접는 것이 다음 개선점이다(현재 편성 규모에서는 불요).
///
/// **폴백 규약(fail-same)**: argv 조회 실패(권한 부족·경주로 종료)이면 종전과 동일하게
/// `name()` 을 돌려준다. 승격 실패가 판정을 완화하지 않고 종전 동작으로 떨어질 뿐이다.
pub fn collect_descendants_with_cmd(sys: &System, root: u32) -> Vec<(u32, String)> {
    collect_descendants_with_cmd_src(sys, root)
        .into_iter()
        .map(|(pid, cmd, _)| (pid, cmd))
        .collect()
}

/// 관측 문자열의 **출처** — "argv 를 실제로 읽었는가, `name()` 폴백인가".
///
/// 【왜 필요한가 · P1-2】 `collect_descendants_with_cmd` 의 fail-same 폴백은 *관측 정확도* 관점에선
/// 옳지만, 소비자가 그 문자열을 **부정 판정의 근거**로 쓰면 곧바로 틀린다: 폴백 문자열에
/// 플래그가 없는 것은 "플래그가 없다"가 아니라 "**관측하지 못했다**" 이기 때문이다. 그 구분이
/// 없으면 argv 조회가 간헐 실패하는 환경(Windows `OpenProcess`+PEB `ReadProcessMemory` 가
/// EDR·권한·종료 경주로 실패)에서 판정이 **진동**하고, 진동은 곧 이벤트 폭주가 된다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmdSource {
    /// `argv_snapshot` 이 실제 argv 를 돌려줬다 — 문자열이 명령줄 전체다.
    Argv,
    /// argv 미관측 → `name()` 한 토큰으로 접혔다 — **부정 판정의 근거로 쓸 수 없다**.
    NameFallback,
}

/// `collect_descendants_with_cmd` 와 동일하되 **출처를 함께** 돌려준다.
/// 기존 소비자는 얇은 래퍼(`collect_descendants_with_cmd`)를 그대로 쓰므로 거동 무변이고,
/// 관측 실패를 구분해야 하는 소비자만 이 함수를 쓴다.
pub fn collect_descendants_with_cmd_src(sys: &System, root: u32) -> Vec<(u32, String, CmdSource)> {
    let kids = descendant_pids(sys, root);
    if kids.is_empty() {
        return Vec::new();
    }
    let argv = argv_snapshot(&kids);
    kids.into_iter()
        .map(|kid| match argv.get(&kid) {
            Some(cmd) => (kid, cmd.clone(), CmdSource::Argv),
            None => (kid, observed_cmdline(sys, kid), CmdSource::NameFallback),
        })
        .collect()
}

/// 지정 pid 집합의 argv 만 담은 **격리 스냅샷**. 전역 프로세스 표를 건드리지 않는다.
/// `remove_dead_processes=false` — 이 스냅샷은 생존 판정의 근거가 아니라 문자열 재료다
/// (생존 판정은 종전대로 호출자의 `sys` 가 단독 소유).
/// `UpdateKind::Always` 인 이유: 매 호출 새 `System` 이라 `cmd` 가 항상 비어 있어
/// `OnlyIfNotSet` 과 결과가 동일하고, 의도(=지금 읽어온다)가 명시적이다.
fn argv_snapshot(pids: &[u32]) -> HashMap<u32, String> {
    let targets: Vec<Pid> = pids.iter().map(|p| Pid::from_u32(*p)).collect();
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&targets),
        false,
        sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
    );
    let mut out: HashMap<u32, String> = HashMap::new();
    for &pid in pids {
        let Some(pr) = sys.process(Pid::from_u32(pid)) else {
            continue;
        };
        let parts: Vec<String> = pr
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        if parts.is_empty() {
            continue; // argv 미관측 — 호출부가 name() 폴백
        }
        out.insert(pid, parts.join(" "));
    }
    out
}

// ── 관측 기반 에이전트 등록 (2026-08 · 현장 결함 2호: claim-role 전용 pane 부활 불가) ──
//
// 문제: `cys claim-role` 로만 역할을 쥔 pane(사람이 직접 CLI 를 띄운 좌석)은 agent_meta 가
// 영영 None 이라 topology.json 에 agent 없이 영속되고, 콜드부트 부활(`cys restore`·phoenix)이
// "agent 미상 — 건너뜀"으로 그 역할을 영구 제외한다 — 재부팅마다 역할 소실이 100% 재현됐다.
// 원설계가 기본값 추정을 거부한 이유("임의 기본값(claude) 추정은 다른 에이전트를 쓰는 좌석에
// 엉뚱한 CLI 를 띄운다" — cys.rs restore 주석)는 옳다. 그래서 추정이 아니라 **관측**을 기록한다:
// claim 순간 좌석의 자손 프로세스에서 기지(旣知) 에이전트가 '정확히 하나' 보일 때만 그 관측값을
// agent_meta 로 등록한다(모호·무관측=무기록 — 현행과 동일하게 fail-closed).
//
// 관할 확장 주의(성찰 확정): agent_meta 는 사망감지·좌석승계·node-recover 의 관할 스위치다.
// 그래서 등록은 ①역할 claim 시점 1회 ②agent_meta==None 일 때만 ③unix 한정(Windows 는 래퍼
// cmd/node 계층이 관측을 흐려 오식별→오살 위험, 2026-07-29 교훈 — fail-closed 유지)으로 좁힌다.

/// agents.json(JSON 값)에서 (에이전트 이름, 실행 바이너리 basename) 후보를 파생한다 — 순수 함수.
/// `_` 접두 키(_schema·_doc)는 메타라 제외. cmd 의 선두 env 대입(`KEY=val`)을 건너뛴 첫 토큰의
/// basename 이 바이너리다(cys.rs extract_bin 과 동일 규약 — `CLAUDE_CONFIG_DIR=… claude …`).
pub fn agent_candidates_from_json(agents: &serde_json::Value) -> Vec<(String, String)> {
    let Some(obj) = agents.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, spec) in obj {
        if name.starts_with('_') {
            continue;
        }
        let Some(cmd) = spec.get("cmd").and_then(|v| v.as_str()) else {
            continue;
        };
        // env 대입 토큰 판별: `IDENT=…` 형태(cys.rs is_env_assignment 과 동일 취지의 보수 판정).
        let is_env_assign = |tok: &str| {
            tok.split('=').next().is_some_and(|k| {
                !k.is_empty()
                    && tok.contains('=')
                    && k.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !k.chars().next().unwrap_or('0').is_ascii_digit()
            })
        };
        let Some(bin_tok) = cmd.split_whitespace().find(|t| !is_env_assign(t)) else {
            continue;
        };
        let base = bin_tok.rsplit(['/', '\\']).next().unwrap_or(bin_tok);
        if !base.is_empty() {
            out.push((name.clone(), base.to_string()));
        }
    }
    out
}

/// 자손 cmdline 목록 × 후보 목록 → 관측된 에이전트. **정확히 한 에이전트**가 매칭될 때만
/// Some — 둘 이상 매칭(모호)·무매칭이면 None(무기록이 오기록보다 낫다) — 순수 함수.
/// ★매처는 등록 전용 엄격판(`cmdline_matches_agent_exec` — strict ⊆ broad)이다(2026-08-12
/// R2 확정): 종전의 생존 매처 재사용은 경로 세그먼트 FP(`tail -f ~/claude/dev.log`)를 등록으로
/// 승격시켰다. strict 로 등록된 좌석은 broad 생존 매처가 반드시 보므로(부분집합) "등록은 됐는데
/// 사망감지가 못 보는" 비대칭(오살)은 신설되지 않는다 — 위험 방향은 그 반대뿐이다.
pub fn select_observed_agent(
    descendant_cmds: &[String],
    candidates: &[(String, String)],
) -> Option<(String, String)> {
    let mut hit: Option<(String, String)> = None;
    for (agent, bin_base) in candidates {
        if descendant_cmds
            .iter()
            .any(|cmd| cmdline_matches_agent_exec(cmd, bin_base))
        {
            match &hit {
                None => hit = Some((agent.clone(), bin_base.clone())),
                // 서로 다른 에이전트가 동시 매칭 = 모호 → 무기록.
                Some((prev, _)) if prev != agent => return None,
                Some(_) => {}
            }
        }
    }
    hit
}

/// 후보 원천: 디스크 pack_dir/agents.json(user 소유 — 있으면 그 키가 이긴다) ∪ 임베드
/// agents.json(디스크에 없는 키만 보충 — cys.rs load_agent_spec 의 폴백 계층과 동일 취지).
/// 어느 쪽도 파싱 불가면 빈 목록(fail-closed — 관측 등록 자체가 조용히 꺼진다).
pub fn known_agent_candidates() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Ok(raw) = std::fs::read_to_string(cys::pack::pack_dir().join("agents.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            out = agent_candidates_from_json(&v);
        }
    }
    if let Some((_, content)) = cys::pack::PACK_ALL.iter().find(|(r, _)| *r == "agents.json") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(content) {
            for cand in agent_candidates_from_json(&v) {
                if !out.iter().any(|(n, _)| *n == cand.0) {
                    out.push(cand);
                }
            }
        }
    }
    out
}

/// 좌석의 자손 프로세스를 그 시점 프로브로 관측해 에이전트를 식별한다.
/// 전 프로세스 표 refresh 비용을 지불하므로 **드문 경로(claim-role)에서, 락 밖에서만** 부른다
/// (seat_claimable_now 와 동일 근거 — 락 보유 중 수십 ms = 데몬 전체 정지).
pub fn observe_agent_on_surface(s: &crate::state::Surface) -> Option<(String, String)> {
    let candidates = known_agent_candidates();
    if candidates.is_empty() {
        return None;
    }
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    // ★argv 승격(U-5): claim_role 관측 등록의 재료도 명령줄 토큰 매칭이다.
    let cmds: Vec<String> = collect_descendants_with_cmd(&sys, s.pid)
        .into_iter()
        .map(|(_, cmd)| cmd)
        .collect();
    select_observed_agent(&cmds, &candidates)
}

/// ★G5-③(W5-A) 2-표본 확정 대기(pending_agent_obs)의 TTL — 이 시간 안에 2표본째가 일치하지
/// 않으면 확정을 포기한다(Drop). watchdog 틱(5초) 대비 넉넉한 값: 정상 경로는 다음 틱(≤5초)에
/// 확정되므로, TTL 도달 = governance 정체·관측 플래핑이며 오래된 1표본은 부활 재료로 못 쓴다.
pub(crate) const PENDING_OBS_TTL_SECS: f64 = 120.0;

/// ★G5-③(W5-A) 2-표본 확정 판정 — 순수 함수(진리표 테스트 핀 대상 · judge_holder/
/// plan_duplicate_kills 관례와 동형). 상태 변경·이벤트 발행은 호출부(check_agent_death 훅)에
/// 잔류한다. `Drop` 은 사유를 담는다(호출부가 TTL 비교를 재유도하면 판정 두 벌=드리프트).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PendingVerdict {
    /// 동일 단일 에이전트 재관측 — meta 확정 자격.
    Commit,
    /// 확정 포기(pending 소거) — reason: "ttl_expired" | "agent_mismatch".
    Drop { reason: &'static str },
    /// 판단 유보(무관측·모호) — 다음 틱 재시도(TTL 이 상한).
    Keep,
}

/// 진리표(설계 원문 G5-③ · 회귀 핀 pending_verdict_truth_table):
///   동일 단일 에이전트 재관측=Commit / 상이 에이전트=Drop / 무관측=Keep / TTL 초과=Drop /
///   모호(select_observed_agent None)=Keep→TTL Drop.
/// TTL 검사가 최우선이다 — TTL 을 넘긴 1표본은 같은 에이전트가 재관측돼도 신뢰하지 않는다
/// (그 사이 좌석 재용도화·플래핑 가능성 — fail-closed·claim 재시도가 정도).
pub(crate) fn confirm_pending_obs(
    pending: &(String, String, f64),
    current: Option<&(String, String)>,
    now: f64,
    ttl_secs: f64,
) -> PendingVerdict {
    if now - pending.2 > ttl_secs {
        return PendingVerdict::Drop { reason: "ttl_expired" };
    }
    match current {
        // 무관측·모호(둘 다 select_observed_agent None) — 순간 혼선일 수 있어 유보.
        None => PendingVerdict::Keep,
        Some((agent, _)) if *agent == pending.0 => PendingVerdict::Commit,
        // 상이 에이전트 관측 = 1표본이 순간 혼선이었다는 반증 — 즉시 포기.
        Some(_) => PendingVerdict::Drop { reason: "agent_mismatch" },
    }
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

// ─────────────────────────────────────────────────────────────────────────────
// ★T3-G2 중복 서버 오탐 수리 (2026-08-01 윈도우 실사고 · 스크린샷 실측)
//
// 사고: 5노드 **정상** 편성에서 `중복 서버 4개: powershell.exe` · `중복 서버 5개: claude.exe`
// 경보가 연발했다. 원인은 판정이 **이름(cmdline 문자열) 전역 계수**였다는 것 하나다 —
// 구 `check_surfaces` 는 전 surface 의 자손을 **하나의 cmdline_groups 맵**에 부어 넣고
// `pids.len() >= duplicate_threshold(3)` 이면 발화했다. 노드 CLI 는 **노드당 1개**가 정상인데,
// 노드가 3개만 되면 그 정상 편성 자체가 임계를 넘는다(=편성 규모가 곧 경보). 발생원이 없는데
// 경보만 나니 노드들이 그 경보를 수리 일감으로 삼아 무한 자가수리 루프의 연료가 됐다.
//
// 수리 원칙: "중복 서버"는 **이름이 같은 것**이 아니라 **같은 것을 두 번 점유한 것**이다.
//   ① 소유 기반 제외 — 노드 소속 인프라(등록된 에이전트 CLI·pane 셸/콘솔 호스트·cys 자신)는
//      계수 분모에서 뺀다. 노드당 1개가 정상인 프로세스는 애초에 중복의 후보가 아니다.
//   ② 스코프 분리 — 불투명 명령의 중복은 **한 surface 안**에서만 센다(`bun server.ts × 36`
//      = 한 워커가 서버를 쌓은 진짜 사고). surface 를 가로지르는 같은 이름은 편성이지 중복이 아니다.
//   ③ 종단점 기반 진짜 중복 — cmdline 에 **명시 포트·유닉스 소켓**이 있으면 그것이 서비스의
//      정체다. 같은 종단점을 둘 이상이 점유하면 이름·소유자가 달라도 진짜 충돌 → 임계 2
//      (오너 계약 "동일 서버 2개+ 즉시 정리").
//   ④ 조상 사슬 접기 — `sh -c "bun --port 3000"` 처럼 부모·자식이 같은 종단점을 물려 받으면
//      2개로 세지 않는다(래퍼는 서버가 아니다).
// ─────────────────────────────────────────────────────────────────────────────

/// 중복 판정 임계·쿨다운 (단일 등재소).
/// · 불투명 명령(surface 스코프): `Config::duplicate_threshold` 기본 **3** (`CYS_DUP_THRESHOLD`) — 종전값 유지.
/// · 종단점(포트·소켓) 스코프: `Config::duplicate_endpoint_threshold` 기본 **2** (`CYS_DUP_ENDPOINT_THRESHOLD`).
/// · 쿨다운: 그룹 키당 **60초**. `LOAD_DEBOUNCE_SECS` 와 **같은 값이어야 한다** —
///   `prune_watchdog_debounce_maps` 가 `last_dup_alert` 를 그 창으로 만료시키므로, 여기만 늘리면
///   프룬이 창 안 엔트리를 먼저 비워 재발화가 앞당겨진다(디바운스 의미 파손).
const DUP_ALERT_COOLDOWN_SECS: f64 = LOAD_DEBOUNCE_SECS;

/// 자동 정리(auto_kill) 최소 나이 — 빌드 중 잠깐 뜬 프로세스 오살 방지.
const DUP_MIN_AGE_SECS: f64 = 45.0;

/// 한 프로세스의 관측 사실(순수 판정 입력). sys·daemon 의존 수집은 `check_surfaces` 에 잔류한다.
#[derive(Clone, Debug)]
pub(crate) struct ProcObs {
    pub pid: u32,
    /// 부모 pid(조상 사슬 접기용). 미상이면 0.
    pub ppid: u32,
    /// 이 프로세스를 자손으로 갖는 surface.
    pub surface_id: u64,
    pub cmdline: String,
    /// ★소유 판정: 이 프로세스가 **노드 자체**(등록 에이전트 CLI·pane 셸/콘솔 호스트·cys 계열)인가.
    /// 노드당 1개가 정상이므로 중복 계수에서 제외하고, 자동 정리 대상에서도 영구 배제한다.
    pub node_owned: bool,
    /// 관측 시점의 나이(초). 종단점 스코프의 **오탐 완충**에만 쓴다 — `psql --port 5432` 같은
    /// 클라이언트 도구가 잠깐 겹쳐 뜬 것을 "서버 2개 점유"로 오판하지 않게 한다.
    pub age_secs: f64,
}

/// 중복으로 판정된 한 그룹.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DupGroup {
    /// "surface" = 한 surface 안의 동일 명령 다중 인스턴스 / "endpoint" = 동일 포트·소켓 다중 점유.
    pub scope: &'static str,
    /// 디바운스·payload 키(스코프 포함 — 스코프가 다르면 별개 그룹).
    pub key: String,
    /// 대표 cmdline(구 payload 하위호환 — UI `중복 서버 N개: <cmdline>`).
    pub cmdline: String,
    /// surface 스코프일 때만 Some.
    pub surface_id: Option<u64>,
    pub pids: Vec<u32>,
    /// 자동 정리 허용 여부. surface 스코프만 true — 종단점 판정은 휴리스틱(클라이언트 인자
    /// `--port` 오인 가능)이라 **경보만** 하고 집행은 사람(CSO)에게 남긴다.
    pub killable: bool,
}

/// "3000" · "127.0.0.1:3000" · ":3000" → 3000. 0·비수치는 None.
fn parse_port(v: &str) -> Option<u16> {
    let tail = v.rsplit(':').next()?;
    let p: u16 = tail.parse().ok()?;
    (p != 0).then_some(p)
}

/// cmdline 에서 **명시 종단점**(포트·유닉스 소켓)을 뽑는다. 없으면 None(=불투명 명령).
///
/// ★`-p` 단문자 플래그는 **의도적으로 제외**한다 — `mkdir -p`·`docker -p`·`ssh -p 22` 처럼
/// 의미가 과적된 플래그라 클라이언트를 서버로 오인한다. 명시적 장문 플래그만 신뢰한다.
pub(crate) fn endpoint_key(cmdline: &str) -> Option<String> {
    const PORT_EQ: &[&str] = &["--port=", "-port=", "port=", "--listen=", "--addr=", "--bind="];
    const PORT_SP: &[&str] = &["--port", "-port", "--listen", "--addr", "--bind"];
    const SOCK_EQ: &[&str] = &["--socket=", "--unix-socket=", "--unix=", "--sock="];
    const SOCK_SP: &[&str] = &["--socket", "--unix-socket", "--unix", "--sock"];
    let toks: Vec<&str> = cmdline.split_whitespace().collect();
    for (i, t) in toks.iter().enumerate() {
        let lower = t.to_ascii_lowercase();
        for pfx in PORT_EQ {
            if let Some(v) = lower.strip_prefix(pfx) {
                if let Some(p) = parse_port(v) {
                    return Some(format!("port:{p}"));
                }
            }
        }
        if PORT_SP.contains(&lower.as_str()) {
            if let Some(p) = toks.get(i + 1).and_then(|v| parse_port(v)) {
                return Some(format!("port:{p}"));
            }
        }
        for pfx in SOCK_EQ {
            if let Some(v) = lower.strip_prefix(pfx) {
                if !v.is_empty() {
                    return Some(format!("socket:{v}"));
                }
            }
        }
        if SOCK_SP.contains(&lower.as_str()) {
            if let Some(v) = toks.get(i + 1) {
                if !v.is_empty() && !v.starts_with('-') {
                    return Some(format!("socket:{v}"));
                }
            }
        }
    }
    None
}

/// ★순수 판정 — 관측치에서 **진짜 중복**만 뽑는다. 부작용 0 · sys 무의존 → 양방향 시뮬로 실증한다.
///
/// 불변식:
///  ① `node_owned` 프로세스는 어떤 그룹에도 들어가지 않는다(정상 편성 = 경보 0의 근거).
///  ② 불투명 명령은 **같은 surface 안**에서만 계수한다 — 노드 수가 늘어도 경보가 늘지 않는다.
///  ③ 종단점(포트·소켓)이 명시된 프로세스는 종단점 스코프로만 계수한다(이중 발화 금지).
///  ④ 같은 그룹 안에서 부모가 함께 잡히면 부모를 뺀다(래퍼 중복 계수 금지).
///  ⑤ 종단점 스코프는 `min_age_secs` 이상 산 프로세스만 센다 — 클라이언트 도구(`psql --port …`)가
///     잠깐 겹친 것을 "서버 2개"로 오판하면 경보 소음이 되살아난다. surface 스코프(같은 pane 안
///     동일 명령 N개)는 그 자체로 명확해 나이 게이트를 두지 않는다(종전 동작 보존).
///  ⑥ 출력은 결정론 정렬(scope, key) · pids 는 asc.
pub(crate) fn plan_duplicate_alerts(
    obs: &[ProcObs],
    dup_threshold: usize,
    endpoint_threshold: usize,
    min_age_secs: f64,
) -> Vec<DupGroup> {
    // pid 중복 관측(트리 겹침) 제거 — 첫 관측을 채택.
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let cands: Vec<&ProcObs> = obs
        .iter()
        .filter(|o| !o.node_owned && !o.cmdline.is_empty()) // 불변식 ①
        .filter(|o| seen.insert(o.pid))
        .collect();

    // 그룹 키 산출 — 종단점 우선(불변식 ③).
    let mut groups: HashMap<(&'static str, String), Vec<&ProcObs>> = HashMap::new();
    for o in &cands {
        let (scope, key) = match endpoint_key(&o.cmdline) {
            Some(ep) => ("endpoint", ep),
            // surface 스코프 키: `<sid>#<cmdline>` — sid 는 숫자이므로 `#` 하나로 모호성이 없다.
            None => ("surface", format!("{}#{}", o.surface_id, o.cmdline)), // 불변식 ②
        };
        groups.entry((scope, key)).or_default().push(o);
    }

    let mut out: Vec<DupGroup> = Vec::new();
    for ((scope, key), members) in groups {
        // 불변식 ④: 같은 그룹 안에 부모가 있으면 그 부모는 뺀다(래퍼).
        let member_pids: std::collections::HashSet<u32> = members.iter().map(|m| m.pid).collect();
        let mut kept: Vec<&&ProcObs> = members
            .iter()
            .filter(|m| !members.iter().any(|c| c.ppid == m.pid && member_pids.contains(&c.pid)))
            // 불변식 ⑤: 종단점만 나이 게이트.
            .filter(|m| scope != "endpoint" || m.age_secs >= min_age_secs)
            .collect();
        kept.sort_by_key(|m| m.pid); // 불변식 ⑥
        let threshold = if scope == "endpoint" { endpoint_threshold } else { dup_threshold };
        if threshold == 0 || kept.len() < threshold {
            continue;
        }
        let first = kept[0];
        out.push(DupGroup {
            scope,
            key: format!("{scope}:{key}"),
            cmdline: first.cmdline.clone(),
            surface_id: (scope == "surface").then_some(first.surface_id),
            pids: kept.iter().map(|m| m.pid).collect(),
            killable: scope == "surface",
        });
    }
    out.sort_by(|a, b| (a.scope, &a.key).cmp(&(b.scope, &b.key))); // 불변식 ⑥
    out
}

/// pane 기동 사슬의 **배관**(셸·콘솔 호스트). 노드마다 1개 이상 항상 존재하는 것이 정상이며
/// 그 자체로 서버가 될 수 없다. 윈도우 실사고의 `powershell.exe × 4` 가 바로 이 부류다.
const SHELL_BASENAMES: &[&str] = &[
    "sh", "bash", "zsh", "dash", "fish", "ksh", "csh", "tcsh", "login", "env", "powershell",
    "pwsh", "cmd", "conhost", "openconsole", "winpty", "conpty",
];

/// 한 자손 프로세스가 **노드 소속 인프라**인가 — 중복 계수·자동 정리에서 빼는 유일 판정.
///
/// `agent_bins` = 살아있는 전 surface 가 `launch-agent` 로 등록한 에이전트 실행 파일 basename
/// 목록(= 데몬이 아는 "노드 소속"의 유일 근거). **`Surface.cmd` 는 쓰지 않는다** — 그 필드는
/// 셸이 아니라 *사용자 명령*을 담기도 해서(`cmd.unwrap_or(shell)`, state.rs `create_surface_with_env`),
/// 그걸 소유 근거로 쓰면 `cmd="bun server.ts"` 로 연 pane 에서 **정작 감시해야 할 bun 이 통째로
/// 제외**된다(자기무력화).
///
/// ★의도된 비대칭: 판정이 흔들릴 때는 "제외" 쪽으로 기운다. 여기서 헛치면(위양성) 경보 소음이
/// 노드의 자가수리 루프를 먹이고, 놓치면(위음성) 중복 서버 하나를 사람이 `cys ps` 로 잡으면 된다.
pub(crate) fn is_node_owned(cmdline: &str, agent_bins: &[String]) -> bool {
    // ① 등록된 에이전트 CLI — 노드당 1개가 정상(claude·codex·agy…).
    if agent_bins.iter().any(|b| cmdline_matches_agent(cmdline, b)) {
        return true;
    }
    let base = cmdline
        .split_whitespace()
        .next()
        .unwrap_or("")
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let base = base.strip_suffix(".exe").unwrap_or(&base);
    if base.is_empty() {
        return false;
    }
    // ② cys 자신(CLI·데몬·채널 브리지) — 관측자가 관측 대상이 되면 안 된다.
    // ③ 셸·콘솔 호스트 배관.
    matches!(base, "cys" | "cysd") || SHELL_BASENAMES.contains(&base)
}

/// 완화책 ③: surface별 자식 수 감시 + ★소유·종단점 기반 진짜 중복 서버 감지.
fn check_surfaces(
    daemon: &Daemon,
    sys: &System,
    last_dup_alert: &mut HashMap<String, f64>,
    last_proc_alert: &mut HashMap<u64, f64>,
) {
    // ★락 순서: surfaces 맵 락은 **여기서 끝낸다**(Arc 복제 후 즉시 해제). agent_meta 는 그 뒤에
    //   잠근다 — surfaces 를 쥔 채 agent_meta 를 잠그면 check_agent_death·handlers 의
    //   (surfaces 해제 → agent_meta) 순서와 역전돼 교착 여지가 생긴다.
    let live: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    let surfaces: Vec<(u64, u32)> = live.iter().map(|s| (s.id, s.pid)).collect();
    // ★소유 원장: 살아있는 전 surface 가 launch-agent 로 등록한 에이전트 실행 파일 basename.
    // "노드 소속 프로세스"를 데몬이 아는 유일한 근거이며, 이것이 이름 계수를 소유 계수로 바꾼다.
    let agent_bins: Vec<String> = {
        let mut v: Vec<String> = live
            .iter()
            .filter_map(|s| s.agent_meta.lock().unwrap().clone())
            .map(|(_, bin)| bin.rsplit(['/', '\\']).next().unwrap_or(&bin).to_string())
            .collect();
        v.sort();
        v.dedup();
        v
    };

    let mut obs: Vec<ProcObs> = Vec::new();
    for (sid, root_pid) in &surfaces {
        // ★U-5 범위 외(의도적 비승격): 여기의 cmdline 은 무엇인가를 판별하는 **식별자**가
        // 아니라 동일물 둘을 세는 **그룹핑 키**다(plan_duplicate_alerts → auto_kill 대상 집합).
        // 이름 → 전체 argv 로 바꾸면 그룹 경계가 재정의되어 **자동 kill 의 폭발 반경이 이동**하고
        // 그것은 이 단위(관측 정확도)가 아니라 별도 단위의 판정이다. 안전측 기본값으로
        // 종전 관측(이름)을 유지한다 — 완화가 아니라 **무변경**이다.
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
        let obs_now = now_epoch();
        for (pid, cmdline) in descendants {
            if cmdline.is_empty() {
                continue;
            }
            let (ppid, age_secs) = sys
                .process(Pid::from_u32(pid))
                .map(|p| {
                    (
                        p.parent().map(|x| x.as_u32()).unwrap_or(0),
                        obs_now - p.start_time() as f64,
                    )
                })
                .unwrap_or((0, 0.0));
            obs.push(ProcObs {
                pid,
                ppid,
                surface_id: *sid,
                node_owned: is_node_owned(&cmdline, &agent_bins),
                cmdline,
                age_secs,
            });
        }
    }

    // ★판정은 순수 함수 단독(부작용 0) — 집행(publish·kill)만 여기 잔류한다.
    let groups = plan_duplicate_alerts(
        &obs,
        daemon.config.duplicate_threshold,
        daemon.config.duplicate_endpoint_threshold,
        DUP_MIN_AGE_SECS,
    );
    for g in groups {
        let now = now_epoch();
        // 쿨다운: 그룹 키당 DUP_ALERT_COOLDOWN_SECS(60초).
        let fire = last_dup_alert
            .get(&g.key)
            .map(|t| now - t > DUP_ALERT_COOLDOWN_SECS)
            .unwrap_or(true);
        if !fire {
            continue;
        }
        last_dup_alert.insert(g.key.clone(), now);
        let auto_kill = daemon.config.auto_kill_duplicates && g.killable;
        daemon.bus.publish(
            "watchdog.duplicate_procs",
            "watchdog",
            g.surface_id,
            // 구 필드(cmdline·count·pids·auto_kill)는 그대로 — UI·HUD 하위호환. 나머지는 순수 추가.
            json!({"cmdline": g.cmdline, "count": g.pids.len(), "pids": g.pids,
                   "auto_kill": auto_kill, "scope": g.scope, "key": g.key,
                   "surface_id": g.surface_id,
                   "threshold": if g.scope == "endpoint" {
                       daemon.config.duplicate_endpoint_threshold
                   } else {
                       daemon.config.duplicate_threshold
                   }}),
        );
        if !auto_kill {
            continue;
        }
        // 디렉티브 스펙 "45초+/3개+": 정책 판정은 순수 함수(plan_duplicate_kills)에
        // 위임하고, sys 의존 입력 수집·집행(kill_pid·publish)만 controller에 잔류한다.
        // sys 의존 입력을 순수 경계 밖에서 미리 수집(start_time은 System에서만 조회 가능).
        let ages: Vec<(u32, f64)> = g
            .pids
            .iter()
            .filter_map(|&pid| {
                sys.process(Pid::from_u32(pid))
                    .map(|p| (pid, p.start_time() as f64))
            })
            .collect();
        if ages.is_empty() {
            continue;
        }
        let (kept, killed) = plan_duplicate_kills(ages, now, DUP_MIN_AGE_SECS);
        if killed.is_empty() {
            continue;
        }
        for &pid in &killed {
            kill_pid(pid); // 집행 (controller 잔류)
        }
        daemon.bus.publish(
            // 집행 (controller 잔류)
            "watchdog.duplicates_killed",
            "watchdog",
            g.surface_id,
            json!({"cmdline": g.cmdline, "kept": kept, "killed": killed,
                   "min_age_secs": DUP_MIN_AGE_SECS, "scope": g.scope, "key": g.key}),
        );
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
/// ★G4(W4-C) pub(crate) 격상: handlers `manual_reap_denial`(수동 reap RPC의 grace 판정)이
/// 재사용한다 — 수치 단일 정의처. 수동/자동 reap이 다른 잣대를 쓰면 '수동은 통과, 자동은
/// 미달' 드리프트가 난다(grace 값 변경은 반드시 이 함수 한 곳에서만).
pub(crate) fn exited_surface_due(has_role: bool, elapsed_secs: u64) -> bool {
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
    // 미배달 큐 폐기 통지 — queued:true 응답을 받은 발신자의 무음 메시지 유실 차단
    // (★G1(W2-B): payload는 폐기 3발행처 공용 빌더 — 스키마 단일 소유).
    let dropped: Vec<crate::state::QueueEntry> =
        surface.pending_queue.lock().unwrap().drain(..).collect();
    if !dropped.is_empty() {
        daemon.bus.publish(
            "queue.dropped",
            "queue",
            Some(id),
            crate::state::queue_dropped_payload("surface_closed", &dropped, None),
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
/// 머리는 항상 방금 보낸 항목이다. 그래도 머리 일치를 확인하고 제거하는 belt-and-suspenders
/// 가드 — 무조건 pop_front이 미배달 새 머리를 삼키는 일을 구조적으로 차단한다.
/// ★G1(W2-A): 판정을 텍스트에서 **id**로 승격 — 텍스트 비교는 동일 문구 중복 항목
/// (빈 문자열 Return 큐가 대표례)에서 원리상 모호했다. id는 유일하므로 가드가 완전해진다.
fn pop_delivered_head(q: &mut std::collections::VecDeque<crate::state::QueueEntry>, delivered_id: &str) {
    if q.front().map(|e| e.id.as_str()) == Some(delivered_id) {
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

// ─── ★G1(W2-D): 단계형 quiet(기아·결함 1 봉인) 노브 3종 — 기본값 잠금 배포 ───────────
// 활성화 절차(2단 롤아웃 · 브리프 확정): 1단(관측 배치 = **현재 기본값**) MAX_WAIT=0·
// STARVE=0 — 배달 동작·주입 바이트 완전 현행 동일, queue.delivered 의 wait_secs 분포만
// 관측된다. 2단(활성) 실측 분포 확인 후 데몬 env 에 CYS_QUEUE_MAX_WAIT_SECS=120 ·
// CYS_QUEUE_STARVE_ALERT_SECS=600 권장값을 설정한다. 각 노브는 즉시 현행 복원 스위치를
// 겸한다(0 재설정 = 구동작 — 무회귀 절대 불변).

/// 단계형 배달의 머리 최대 대기(초) — 머리 항목의 (uptime 클램프) 대기가 이 값 이상이면
/// quiet 임계를 `queue_overdue_quiet_secs()`(기본 1s)로 낮춘 '제한 배달(overdue)' 자격을
/// 얻는다. **기본 0 = 단계형 비활성 = 현행 quiet 3s 규칙 그대로**(활성 권장값 120 — 위
/// 롤아웃 주석). human_typing·pause·queue_paused·empty_seat 게이트는 이 노브와 무관하게
/// 어떤 단계에서도 절대 면제되지 않는다(절대 불변).
fn queue_max_wait_secs() -> u64 {
    std::env::var("CYS_QUEUE_MAX_WAIT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// overdue(제한 배달) 단계의 quiet 임계(초) — 기본 1. '출력 중 주입 금지' 의미론의
/// 하한이라 판정(queue_quiet_verdict)이 1 미만 설정을 1로 승격한다(0초 강제주입 봉인 —
/// 회귀 핀 테스트 대상).
fn queue_overdue_quiet_secs() -> u64 {
    std::env::var("CYS_QUEUE_OVERDUE_QUIET_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// 기아 경보 임계(초) — 머리 대기(uptime 클램프)가 이 값 이상인 채 배달이 막혀 있으면
/// `queue.starved` 발행(전용 쿨다운 5분 · depth_high 와 별도 축). **기본 0 = 비활성**
/// (활성 권장값 600). 경보는 발행뿐 — 자동 조치 없음(hint 문구 계약 = state.rs).
fn queue_starve_alert_secs() -> u64 {
    std::env::var("CYS_QUEUE_STARVE_ALERT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// ★G1(W2-D): 단계형 quiet 판정 결과 — 순수 판정자(queue_quiet_verdict)의 어휘.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuietVerdict {
    /// 배달 허용. `overdue=true` = max-wait 초과 머리의 '제한 배달'(완화 quiet 1s 로 통과).
    /// 정상 quiet(기본 3s)를 채운 배달은 단계와 무관하게 `overdue=false`.
    Deliver { overdue: bool },
    /// 아직 바쁨(출력 중) — 이번 틱 배달 보류.
    WaitBusy,
}

/// ★G1(W2-D): 단계형 quiet 순수 판정자 — 기아(결함 1) 봉인의 본체. 부작용·시계·env 없이
/// 입력만으로 판정한다(approval_wakeup_suppressed 순수 술어 관례와 동형 — 테스트 핀 대상).
///
/// 규칙(우선순위 순):
/// 1. `quiet_for >= quiet`(현행 3s 규칙 충족) → 정상 배달(overdue=false) — 현행 의미론 불변.
/// 2. `max_wait == 0`(단계형 비활성) 또는 `head_wait_secs < max_wait` → WaitBusy(현행 그대로).
/// 3. overdue 단계: `quiet_for >= max(overdue_quiet, 1)` → 제한 배달(overdue=true).
///    `quiet_for == 0`(지금 출력 중)은 **어떤 단계에서도 배달 금지** — '출력 중 주입 금지'
///    의미론은 overdue 에도 불변이다(0초 강제주입 아님 · 하한 1s 구조 봉인).
///
/// human_typing·pause·queue_paused·empty_seat 는 이 판정의 **입력이 아니다** — 그 게이트들은
/// 호출부(deliver_queued)에서 이 판정과 무관하게 항상 별도 적용된다(면제 불가 · 절대 불변).
pub(crate) fn queue_quiet_verdict(
    head_wait_secs: u64,
    quiet_for: u64,
    quiet: u64,
    max_wait: u64,
    overdue_quiet: u64,
) -> QuietVerdict {
    if quiet_for >= quiet {
        return QuietVerdict::Deliver { overdue: false };
    }
    if max_wait == 0 || head_wait_secs < max_wait {
        return QuietVerdict::WaitBusy;
    }
    if quiet_for >= overdue_quiet.max(1) {
        QuietVerdict::Deliver { overdue: true }
    } else {
        QuietVerdict::WaitBusy
    }
}

/// ★G1(W2-D BLOCKER): overdue·기아 자격의 머리 대기 측정 — daemon/surface **uptime 클램프**.
///
/// 왜 enqueued_at 원값으로 재지 않는가: 재기동 직후엔 last_human_input(휘발·메모리)이 비어
/// typing 가드가 무방비다. WAL 생존 항목의 원 enqueued_at 로 재면 부트 즉시 overdue 자격이
/// 되어 stale 백로그가 부트체인 최취약 창(phoenix 부활·디렉티브 주입이 겹치는 구간)에 몰려
/// 배달된다. 대기는 '이 부트의 이 surface 가 실제로 기다리게 한 시간'만 센다:
/// 기준점 = max(enqueued_at, daemon.started_at, surface.created_at).
/// 역행(시계 스큐·NTP 점프·미상 시각)은 0 클램프 — 측정 불능은 overdue 부적격(fail-closed).
pub(crate) fn queue_head_wait_secs(
    now: f64,
    enqueued_at: f64,
    daemon_started_at: f64,
    surface_created_at: f64,
) -> u64 {
    let anchor = enqueued_at.max(daemon_started_at).max(surface_created_at);
    (now - anchor).max(0.0) as u64
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
    } else if blocked_by.starts_with("empty_seat") {
        // ★SEAT: 이 사유는 '좌석에 에이전트가 없다'는 뜻이다 — 임계·quiet 노브로는 풀리지 않는다.
        // 조치는 좌석에 에이전트를 앉히는 것(부활·수동 연결)이므로 그 손잡이를 가리킨다.
        "좌석에 에이전트가 없다 — 그 pane 에서 직접 agent 를 실행하거나 `cys restore`(부활)로 \
         좌석을 채우면 보류분이 순서대로 배달된다(메시지는 보존 중·유실 아님)"
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

/// ★G1(W2-D): 기아 경보 — 머리 대기(uptime 클램프)가 임계 이상인 채 배달이 막혀 있으면
/// `queue.starved` 를 전용 쿨다운(5분 · depth_high 의 depth_alerted 맵과 **별도**)으로
/// 발행한다. depth_high 는 적체 **양**의 경보, starved 는 머리 **나이**의 경보 — depth 1
/// 이라도 오래 막히면 기아다(10분+ 무간극 출력 노드는 병리 상태 — 침묵 대신 경보).
/// 모든 '막힘' 분기에서 depth 경보와 나란히 호출한다(한 분기라도 빠지면 그 사유의 기아가
/// 침묵한다). 발행뿐 — 자동 조치 없음: 강제 배달(queue.deliver·W2-E)은 운영자(사람) 판단의
/// 몫이다(hint 문구 계약 = state.rs::QUEUE_STARVED_HINT · LLM 자동 반응 유도 금지).
fn alert_queue_starved_if_stalled(
    daemon: &Arc<Daemon>,
    s: &Arc<crate::state::Surface>,
    starve_alerted: &mut HashMap<u64, f64>,
    blocked_by: &str,
    head: &crate::state::QueueEntry,
    head_wait_secs: u64,
    depth: usize,
) {
    let threshold = queue_starve_alert_secs();
    if threshold == 0 || head_wait_secs < threshold {
        return;
    }
    let now = now_epoch();
    let last = starve_alerted.get(&s.id).copied().unwrap_or(0.0);
    if now - last < QUEUE_ALERT_COOLDOWN_SECS {
        return;
    }
    starve_alerted.insert(s.id, now);
    daemon.bus.publish(
        "queue.starved",
        "queue",
        Some(s.id),
        crate::state::queue_starved_payload(
            &cys::surface_ref(s.id),
            s.role.lock().unwrap().clone(),
            head,
            head_wait_secs,
            depth,
            blocked_by,
        ),
    );
}

/// ★T-0147-2 §2 층3 A3′(= §8 R2-C3 수용): 배달 텍스트에 봉입된 wakeup entry id(`W-<hex>`) 추출.
///
/// `queue.enqueued`는 **수락 증거**이지 배달 영수증이 아니다. javis_wakeup 의 상태머신
/// (`armed → seen-claim → enqueue(W-id 봉입) → Inject-ack → disarm`)에서 critical-tier 는
/// 실제 `WriteReq::Inject` 가 일어났다는 영수증(`queue.delivered`)을 보고서만 disarm 한다 —
/// 그래서 데몬이 배달한 텍스트에서 W-id 를 되읽어 에코해야 python 게이트가 조인할 수 있다.
///
/// 정규식 대신 바이트 스캔인 이유: 문법이 고정(`W-` + `[0-9a-f]{1,32}`)이고 이 함수는 배달
/// 틱마다 도는 경로다 — Regex 컴파일/의존을 새로 끌 이유가 없다(크레이트에 regex 가 이미 있어도
/// 이 문법에는 과잉이다).
///
/// 경계 규약: `W-` 뒤 hex 0자는 무시(`W-`·`W-ZZZZ`), hex 뒤 첫 비-hex 문자가 종료 경계
/// (`[wakeup W-a1b2c3d4e5]` → `W-a1b2c3d4e5`), 등장 순서 보존 + 중복 제거, 최대 32개.
fn wakeup_entry_ids(text: &str) -> Vec<String> {
    /// 한 배달 텍스트에서 뽑는 id 상한 — digest 병합이라도 이 이상은 페이로드 비대만 낳는다.
    const MAX_IDS: usize = 32;
    /// id 본문 hex 상한(계약: 1~32자).
    const MAX_HEX: usize = 32;
    let is_hex = |c: u8| c.is_ascii_digit() || (b'a'..=b'f').contains(&c);
    let b = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i + 1 < b.len() {
        if b[i] != b'W' || b[i + 1] != b'-' {
            i += 1;
            continue;
        }
        let start = i + 2;
        let mut end = start;
        while end < b.len() && end - start < MAX_HEX && is_hex(b[end]) {
            end += 1;
        }
        // hex 0자 = 오형 → `W-` 다음 문자부터 재스캔(겹친 `W-W-a1` 도 놓치지 않는다).
        i = if end > start { end } else { start };
        if end == start {
            continue;
        }
        let id = format!("W-{}", &text[start..end]);
        if !out.iter().any(|x| x == &id) {
            out.push(id);
            if out.len() >= MAX_IDS {
                break;
            }
        }
    }
    out
}

/// ★G1(W2-D): 배달 성과 — deliver_head_locked 의 반환값. queue.deliver RPC(W2-E)가
/// 응답({queue_entry_id, seq, remaining})을 조립하는 재료를 겸한다.
#[derive(Debug)]
pub(crate) struct Delivered {
    pub entry: crate::state::QueueEntry,
    pub remaining: usize,
}

/// ★G1(W2-D): 배달 임계영역 **단일 헬퍼** — watchdog 틱(deliver_queued)과 queue.deliver
/// RPC(운영자 강제 배달·W2-E)가 공유한다. 두 경로가 각자 구현하면 한쪽만 고쳐진다
/// (migrate_seat_queue 주석의 관례) — 큐 배달 구현은 이 함수 하나뿐이어야 한다.
/// **호출 전제**: 안전 게이트(kill-switch pause·queue_paused·human_typing·empty_seat +
/// quiet 판정)는 호출부 책임 — 이 헬퍼는 게이트를 통과한 뒤의 원자 배달만 담당한다.
///
/// 임계영역(현행 순서·원자성 그대로 — 절대 불변): pending_queue 락 획득 → front →
/// record_audited → try_send → pop_delivered_head(id) → 락 해제.
///
/// - ★R1 배달 원장 — 주입보다 앞(delivery.rs 불변식 ①). `cys send --queued` 는 enqueue
///   시점에 조기 반환하므로 **여기가 유일한 주입 지점**이다. 임계영역(pending_queue 락)
///   안인 이유: 락 밖에서 미리 기록하면 "A 를 기록하고 B 를 배달"하는 창이 열려 배달분이
///   원장에 없을 수 있다(= 게이트 개방 = 치명). 레코드는 수백 바이트 append 라 락 보유는
///   순간이고, 블로킹 PTY write 는 여전히 writer 스레드가 한다(watchdog 무정지).
/// - TOCTOU 차단: front 읽기·writer 인계·pop 을 한 임계영역으로 묶는다. queue.clear·
///   close_surface 는 같은 락으로 drain 하므로 '읽고서 인계하는' 사이에 끼어들 수 없다.
/// - try_send 실패 = None(메시지 보존 — 다음 틱 재시도). 락이 배달자 2원화(틱+RPC)를
///   직렬화하고 pop-by-id 가 상대가 이미 배달한 머리의 오삼킴을 구조 차단한다.
/// - forced/overdue 는 이벤트 층 구분일 뿐 임계영역 동작·원장 스키마는 동일하다(불변).
/// - ★G1(W2-E) `expect_head_id`: Some(id)이고 락 획득 시점 머리가 그 id 가 아니면 **아무것도
///   하지 않고** None — RPC 강제 배달이 게이트 통과·조준 해석과 실배달 사이 창에서 틱·clear 와
///   경합해도 '조준한 항목이 아닌 다음 항목'을 forced 로 오배달하지 않는다(pop-by-id 와 같은
///   belt-and-suspenders 층). watchdog 틱은 None 을 넘긴다('그 시점 머리'가 곧 조준 — 현행 동일).
pub(crate) fn deliver_head_locked(
    daemon: &Arc<Daemon>,
    s: &Arc<crate::state::Surface>,
    forced: bool,
    overdue: bool,
    expect_head_id: Option<&str>,
) -> Option<Delivered> {
    let delivered = {
        let mut q = s.pending_queue.lock().unwrap();
        let entry = q.front().cloned()?;
        if expect_head_id.is_some_and(|want| want != entry.id) {
            return None; // 조준 항목이 더는 머리가 아니다(경합) — 무부작용 반환
        }
        crate::delivery::record_audited(
            daemon,
            s.id,
            &entry.text,
            crate::delivery::Origin::Queue,
            None,
        );
        let req = crate::state::WriteReq::Inject {
            text: entry.text.clone(),
            cr_delay_ms: 400,
            clear_first: false, // queued 배달은 quiet 대기 후라 선정리 불필요(현행 동작 보존)
        };
        if s.write_tx.try_send(req).is_err() {
            return None; // 인계 실패 — 메시지 보존, 다음 틱 재시도
        }
        // ★G1(W2-A): pop 판정은 방금 인계한 항목의 **id** — 동일 텍스트 중복 항목 오삼킴 차단.
        pop_delivered_head(&mut q, &entry.id);
        Delivered { entry, remaining: q.len() }
    };
    // T4-17 에코 제외 창 — 큐 배달도 원격 주입이다
    *s.last_injected.lock().unwrap() = Some(std::time::Instant::now());
    // ★T-0147-2 §2 층3 A3′(R2-C3): 배달 영수증에 봉입 W-id 를 **배열**로 에코한다.
    // 배열인 이유 — javis_wakeup 의 digest 모드(층1 I6)가 같은 target 의 N건을 1회
    // Inject 로 병합하므로, 병합된 **전** W-id 가 ack 돼야 critical-tier 가 disarm 된다.
    // 하나라도 빠지면 그 사건은 seen-store 에 inflight 로 남아 TTL 마다 영구 재enqueue 된다
    // (= wakeup 홍수 재발). 봉입 id 가 없는 일반 큐 배달은 빈 배열이다.
    // surface_ref 는 python 게이트가 target 을 surface id 정수 재조립 없이 조인하도록 가산.
    // (entry_ids = W-id 에코 계약 — 큐 항목 id(queue_entry_id)와 별개 체계·키명 불변.)
    // ★G1(W2-B): payload는 공용 빌더 — 기존 4키 불변 + queue_entry_id/seq/enqueued_at/
    // delivered_at/wait_secs additive. W-id 에코는 **원문 text 스캔** 그대로다.
    // ★G1(W2-D): overdue/forced 는 이벤트 층 additive — 원장(delivery.rs)은 무변경.
    let entry_ids = wakeup_entry_ids(&delivered.entry.text);
    daemon.bus.publish(
        "queue.delivered",
        "queue",
        Some(s.id),
        crate::state::queue_delivered_payload(
            &delivered.entry,
            delivered.remaining,
            &entry_ids,
            &cys::surface_ref(s.id),
            now_epoch(),
            overdue,
            forced,
        ),
    );
    // P7 큐 WAL: 배달로 줄어든 큐를 디스크에 반영(스냅샷 최신화).
    daemon.persist_queue_state();
    Some(delivered)
}

/// ★G1(W2-E): queue.deliver(운영자 강제 배달) 거부 사유 — RPC err 코드와 1:1(결정론 소비
/// 계약 · CLI 는 code 접두로 '게이트 거부 exit'를 가른다). 강제(forced)는 'quiet **대기**
/// 생략'만이며 안전 게이트는 전부 유지한다 — 2026-07-17 빈 좌석 zsh 오타이핑 사고·R1 MED-2
/// 를 운영자 경로에서도 재개방하지 않는다(절대 불변).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForceDeliverDenied {
    /// 헬스 조치(pause-queue)가 이 surface 배달을 보류 중.
    QueuePaused,
    /// 사람 입력 흔적 신선(기본 30s) — 미완성 입력 이어붙기/제출 차단(R1 MED-2 · 면제 불가).
    TypingGuard,
    /// role 좌석인데 에이전트 미연결 — 빈 셸 zsh 에 문자 타이핑되는 사고 경로 차단.
    EmptySeat,
    /// ★성찰 BLOCKER: forced 에도 overdue_quiet(기본 1s) 하한 — 출력 한복판 주입 금지.
    OutputBusy { quiet_for: u64, need: u64 },
    /// 배달할 항목 없음.
    QueueEmpty,
    /// 조준 entry_id 가 머리가 아님 — 순서 변경은 allow_reorder 명시로만(무음 재정렬 금지).
    NotHead { index: usize },
    /// 조준 entry_id 가 큐에 없음(이미 배달·폐기됐거나 오타).
    NotFound,
    /// 게이트·조준 통과 후 실배달 직전 경합(틱이 먼저 배달·clear drain·writer busy) — 재시도 대상.
    Raced,
}

impl ForceDeliverDenied {
    /// RPC err code — cys CLI `queue_deliver_exit_code` 의 게이트 판정 접두와 1:1.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            ForceDeliverDenied::QueuePaused => "queue_paused",
            ForceDeliverDenied::TypingGuard => "typing_guard",
            ForceDeliverDenied::EmptySeat => "empty_seat",
            ForceDeliverDenied::OutputBusy { .. } => "output_busy",
            ForceDeliverDenied::QueueEmpty => "queue_empty",
            ForceDeliverDenied::NotHead { .. } => "not_head_requires_allow_reorder",
            ForceDeliverDenied::NotFound => "not_found",
            ForceDeliverDenied::Raced => "delivery_failed",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            ForceDeliverDenied::QueuePaused => {
                "queue paused by health action (pause-queue) — 해제 후 재시도".into()
            }
            ForceDeliverDenied::TypingGuard => {
                "human typed recently — 사람 입력 보호(R1 MED-2)는 강제 배달로도 면제 불가".into()
            }
            ForceDeliverDenied::EmptySeat => {
                "role seat has no agent — 좌석을 채우면 보류분이 순서대로 배달된다(유실 아님)".into()
            }
            ForceDeliverDenied::OutputBusy { quiet_for, need } => format!(
                "output streaming (quiet {quiet_for}s < {need}s) — 강제 배달도 출력 중 주입은 \
                 금지(overdue_quiet 하한)"
            ),
            ForceDeliverDenied::QueueEmpty => "pending queue is empty".into(),
            ForceDeliverDenied::NotHead { index } => format!(
                "entry is at index {index}, not head — 순서를 바꾸려면 allow_reorder 를 명시하라"
            ),
            ForceDeliverDenied::NotFound => {
                "entry_id not in pending queue (already delivered/dropped? — queue.delivered/\
                 queue.dropped 이벤트 확인)"
                    .into()
            }
            ForceDeliverDenied::Raced => {
                "delivery raced (watchdog delivered first, queue cleared, or writer busy) — \
                 queue list 재확인 후 재시도"
                    .into()
            }
        }
    }
}

/// ★G1(W2-E): 운영자 강제 배달 — queue.deliver RPC 의 본체(단건 전용 · --all 드레인 금지
/// [성찰 BLOCKER] — 드레인이 필요하면 매 호출마다 데몬 게이트가 재평가되는 단건 반복이
/// 유일 경로이며, 그마저 v1 CLI 는 제공하지 않는다: 틱당 1건 페이싱을 뚫는 유일 경로 차단).
///
/// 강제의 의미 = 'quiet **대기**(기본 3s·틱 스케줄) 생략'만이다. 안전 게이트는 전부 유지:
/// - kill-switch pause(daemon.paused)·발신 ACL 은 호출부(handlers "queue.deliver")가 이
///   함수 **앞**에서 집행한다(설계 게이트 순서 ①②).
/// - 이 함수는 ③empty_seat → ④human typing → ⑤queue_paused → ⑥output quiet 하한
///   [성찰 BLOCKER: forced 게이트 목록에 출력 quiet 가 없으면 출력 한복판 주입 허용] 순서로
///   집행 — watchdog 틱(deliver_queued)과 동일 판정 재료·동일 면제 불가(절대 불변).
/// - 배달 자체는 단일 헬퍼 deliver_head_locked 공유(두 경로 갈라짐 금지 관례) +
///   expect_head_id 로 경합 시 오배달을 구조 차단.
///
/// 비머리 조준(entry_id ≠ 머리)은 allow_reorder 명시 시에만 머리로 끌어올린 뒤 배달하며,
/// 재정렬은 배달 성패와 무관하게 queue.reordered 로 발행·WAL 반영한다(무음 재정렬 금지 —
/// 재정렬이 이미 일어난 사실 자체가 순서 사건이다).
pub(crate) fn force_deliver_entry(
    daemon: &Arc<Daemon>,
    s: &Arc<crate::state::Surface>,
    entry_id: Option<&str>,
    allow_reorder: bool,
) -> Result<Delivered, ForceDeliverDenied> {
    // 게이트 ③ empty_seat — watchdog 틱과 동일 판정(Unknown 은 통과 = 현행 동작 강등).
    if s.role.lock().unwrap().is_some()
        && SeatState::from_u8(s.seat_cache.load(Ordering::Relaxed)) == SeatState::Empty
    {
        return Err(ForceDeliverDenied::EmptySeat);
    }
    // 게이트 ④ human typing — 어떤 경로(overdue·forced)에서도 면제 금지(절대 불변).
    let human_recent = s
        .last_human_input
        .lock()
        .unwrap()
        .map(|t| t.elapsed().as_secs() < queue_human_quiet_secs())
        .unwrap_or(false);
    if human_recent {
        return Err(ForceDeliverDenied::TypingGuard);
    }
    // 게이트 ⑤ queue_paused(헬스 조치) — 강제 배달로 우회 불가.
    if s.queue_paused_until
        .lock()
        .unwrap()
        .map(|t| t > std::time::Instant::now())
        .unwrap_or(false)
    {
        return Err(ForceDeliverDenied::QueuePaused);
    }
    // 게이트 ⑥ [성찰 BLOCKER] forced 에도 overdue_quiet(기본 1s·하한 1s) — '출력 중 주입
    // 금지' 의미론은 운영자 강제로도 불변이다(queue_quiet_verdict 의 overdue 하한과 동일 값).
    let need = queue_overdue_quiet_secs().max(1);
    let quiet_for = s.last_output.lock().unwrap().elapsed().as_secs();
    if quiet_for < need {
        return Err(ForceDeliverDenied::OutputBusy { quiet_for, need });
    }
    // 조준 해석(+ 필요 시 머리 끌어올림) — pending_queue 락 한 임계영역에서 원자 수행.
    let (target, reordered_from) = {
        let mut q = s.pending_queue.lock().unwrap();
        if q.is_empty() {
            return Err(ForceDeliverDenied::QueueEmpty);
        }
        match entry_id {
            None => (q.front().expect("non-empty").clone(), None),
            Some(tid) => {
                let Some(pos) = q.iter().position(|e| e.id == tid) else {
                    return Err(ForceDeliverDenied::NotFound);
                };
                if pos == 0 {
                    (q.front().expect("non-empty").clone(), None)
                } else if !allow_reorder {
                    return Err(ForceDeliverDenied::NotHead { index: pos });
                } else {
                    let entry = q.remove(pos).expect("position checked");
                    q.push_front(entry.clone());
                    (entry, Some(pos))
                }
            }
        }
    };
    if let Some(from_index) = reordered_from {
        // 재정렬은 그 자체가 순서 사건 — 이후 배달이 실패(경합)해도 발행·WAL 반영은 유지된다
        // (큐 순서가 실제로 바뀌었으므로 침묵이 오히려 계약 위반).
        daemon.bus.publish(
            "queue.reordered",
            "queue",
            Some(s.id),
            crate::state::queue_reordered_payload(
                &cys::surface_ref(s.id),
                &target,
                from_index,
                "force_deliver",
            ),
        );
        daemon.persist_queue_state();
    }
    // 배달 = 단일 헬퍼 공유(forced=true·overdue=false — 이벤트 층 구분만, 임계영역 동일).
    // expect_head_id: 게이트·조준과 실배달 사이 창에서 틱이 먼저 배달했거나 clear 가 drain
    // 했으면 무부작용 None → Raced(조준 아닌 다음 항목을 forced 로 오배달하지 않는다).
    deliver_head_locked(daemon, s, true, false, Some(&target.id))
        .ok_or(ForceDeliverDenied::Raced)
}

/// 인플라이트 큐 배달자: 대상 surface가 quiet 임계(기본 3초) 이상 조용하면 큐에서 한 건 주입.
/// 연속 배달은 다음 틱 — 메시지 사이 자연 간격이 생겨 에이전트가 한 건씩 소화한다.
/// 배달이 막힌 채 적체되면(depth ≥ 임계) `queue.depth_high`를 쿨다운(5분)으로 발행한다.
/// ★G1(W2-D): busy 판정만 단계형(queue_quiet_verdict — 기본 노브 0 잠금 = 현행 동일)으로
/// 치환하고, 막힘 분기마다 기아 경보(queue.starved — 기본 비활성)를 나란히 점검한다.
/// human_typing·pause·queue_paused·empty_seat 게이트는 코드·순서 완전 불변 — overdue 라도
/// 절대 면제 없음(절대 불변).
fn deliver_queued(
    daemon: &Arc<Daemon>,
    depth_alerted: &mut HashMap<u64, f64>,
    starve_alerted: &mut HashMap<u64, f64>,
) {
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
    // ★G1(W2-D): 노브는 틱당 1회 로드 — surface 루프 안 env 재조회 방지(판정 재료 고정).
    let quiet = queue_quiet_secs();
    let max_wait = queue_max_wait_secs();
    let overdue_quiet = queue_overdue_quiet_secs();
    let surfaces: Vec<Arc<crate::state::Surface>> =
        daemon.surfaces.lock().unwrap().values().cloned().collect();
    for s in surfaces {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        // ★G1(W2-D): 머리 스냅샷 — 빈 큐는 어느 분기에서도 할 일이 없다(depth 경보는
        // depth≥임계≥1, 기아 경보는 머리 존재가 전제 — 관측 동등·무회귀). 락은 순간 보유.
        // 스냅샷과 실제 배달(deliver_head_locked) 사이 머리가 바뀌는 창은 clear(drain·
        // 배달 0건으로 안전)뿐이고, 배달 자체는 헬퍼 임계영역이 그 시점 머리로 원자 수행한다.
        let (head, depth) = {
            let q = s.pending_queue.lock().unwrap();
            match q.front().cloned() {
                Some(h) => (h, q.len()),
                None => continue,
            }
        };
        // ★G1(W2-D BLOCKER): overdue·기아 자격의 대기 = uptime 클램프 측정(부트 직후
        // typing 가드 공백 창 봉인 — queue_head_wait_secs doc 참조).
        let head_wait =
            queue_head_wait_secs(now_epoch(), head.enqueued_at, daemon.started_at, s.created_at);
        // T4-17 헬스 조치: pause-queue 발동 중인 surface는 배달 보류 — 적체는 침묵 금지
        if s.queue_paused_until
            .lock()
            .unwrap()
            .map(|t| t > std::time::Instant::now())
            .unwrap_or(false)
        {
            alert_queue_depth_if_high(daemon, &s, depth_alerted, "queue_paused(헬스 조치)");
            alert_queue_starved_if_stalled(
                daemon, &s, starve_alerted, "queue_paused(헬스 조치)", &head, head_wait, depth,
            );
            continue;
        }
        // 아직 바쁨(출력 중) — steer는 즉시 전송이 담당, 큐는 기다린다.
        // ★G1(W2-D): busy 판정만 단계형 순수 판정자로 치환 — 기본 노브(max_wait=0)에서는
        // 현행 quiet 3s 규칙과 바이트 동일하게 동작한다(무회귀 절대 불변).
        let quiet_for = s.last_output.lock().unwrap().elapsed().as_secs();
        let overdue = match queue_quiet_verdict(head_wait, quiet_for, quiet, max_wait, overdue_quiet)
        {
            QuietVerdict::WaitBusy => {
                alert_queue_depth_if_high(daemon, &s, depth_alerted, "busy(출력 중)");
                alert_queue_starved_if_stalled(
                    daemon, &s, starve_alerted, "busy(출력 중)", &head, head_wait, depth,
                );
                continue;
            }
            QuietVerdict::Deliver { overdue } => overdue,
        };
        // 사람 입력 흔적이 식기 전 배달 금지 — 미완성 입력에 이어붙기/제출 차단(R1 MED-2).
        // ★G1(W2-D): 이 게이트는 단계형 완화(overdue)의 면제 대상이 **절대 아니다** —
        // verdict 가 Deliver{overdue:true}여도 사람 흔적이 신선하면 배달 0건(회귀 핀 테스트).
        let human_recent = s
            .last_human_input
            .lock()
            .unwrap()
            .map(|t| t.elapsed().as_secs() < queue_human_quiet_secs())
            .unwrap_or(false);
        if human_recent {
            alert_queue_depth_if_high(daemon, &s, depth_alerted, "human_typing(사람 입력 직후)");
            alert_queue_starved_if_stalled(
                daemon, &s, starve_alerted, "human_typing(사람 입력 직후)", &head, head_wait, depth,
            );
            continue;
        }
        // ★SEAT 게이트(2026-07-17 실사고 수리): **role 좌석**인데 좌석이 비었으면(에이전트 없음)
        // 배달을 보류한다. 종전엔 quiet 이기만 하면 배달해, 빈 셸이 role 을 쥔 동안 리뷰어 verdict·
        // 워커 보고가 zsh 프롬프트에 문자로 타이핑돼 **보고가 증발**했다(surface:112 실측).
        //
        // 판정 기준을 'role 유무'로 둔 이유: pending_queue 는 텍스트만 담아(anchor 미보존) 항목별
        // role-앵커 여부를 구분할 수 없다. 그런데 role 좌석은 정의상 에이전트 자리이므로 'role 있는
        // surface'가 role-앵커 메시지의 실질 대상이다. role 없는 맨 셸의 `--queued` 자동화는
        // 종전 그대로 통과한다(무회귀).
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
            );
            alert_queue_starved_if_stalled(
                daemon,
                &s,
                starve_alerted,
                "empty_seat(좌석에 에이전트 미연결)",
                &head,
                head_wait,
                depth,
            );
            continue;
        }
        // ★G1(W2-D): 배달 임계영역은 단일 헬퍼(deliver_head_locked — RPC 강제 배달과 공유).
        // pop은 writer 채널 인계 성공 후에만 — 실패 시 메시지를 보존해 다음 틱에 재시도.
        // 블로킹 write·sleep은 surface 전용 writer 스레드가 수행하므로 watchdog은 멈추지 않는다.
        if deliver_head_locked(daemon, &s, false, overdue, None).is_some() {
            // 배달 성공 = 기아 해소 — 쿨다운 리셋(다음 기아는 새 사건으로 다시 경보).
            starve_alerted.remove(&s.id);
        }
    }
}

/// reap 계열 테스트는 CYS_REAP_EXITED*·CYS_ROLE_DEADMAN* env를 만지므로 직렬화한다.
/// ★G4(W4-C): governance::tests 사설 static 에서 **크레이트 테스트 공용**(pub(crate))으로
/// 격상 — handlers::tests 의 수동 reap 테스트도 같은 env 를 읽으므로(grace 판정
/// exited_surface_due 재사용), 모듈별 락 두 개로는 서로를 직렬화하지 못한다.
#[cfg(test)]
pub(crate) static REAP_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// env를 테스트 종료 시(패닉 포함) 이전 값으로 원복하는 가드 —
/// 없던 값은 remove, 있던 값은 원복. 프로세스 전역 env 누수 차단.
#[cfg(test)]
pub(crate) struct ReapEnvGuard {
    prev: Vec<(&'static str, Option<String>)>,
}
#[cfg(test)]
impl ReapEnvGuard {
    pub(crate) fn set(vars: &[(&'static str, &str)]) -> Self {
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
#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::{
        approval_wakeup_suppressed, check_surfaces, collect_descendants, endpoint_key,
        is_node_owned, kill_pid, learn_stuck_candidates, merged_approval_patterns,
        plan_duplicate_alerts, plan_duplicate_kills, wakeup_entry_ids, ProcObs,
    };

    // ─────────────────────────────────────────────────────────────────────────
    // ★U-5 · sysinfo 프로세스 정보 갱신 승격(argv) — 계측 타당성 + 비용
    // ─────────────────────────────────────────────────────────────────────────

    /// 드릴 자식: **이름에는 에이전트 식별자가 없고 argv 에만 있는** 프로세스를 띄운다.
    /// `sh -c '<script>' <argv0> <arg1>` 형식이라 name 은 `sh`, argv 는 5토큰이 된다.
    /// 스크립트 끝의 `; :` 는 셸의 exec 최적화(자기 자신을 sleep 으로 치환)를 막는 관례다
    /// (같은 파일 live_normal_formation_* 드릴의 선례와 동일한 이유).
    #[cfg(unix)]
    fn spawn_argv_masked_child() -> std::process::Child {
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 30 ; :")
            .arg("/opt/cys-u5-drill/claude")
            .arg("--dangerously-skip-permissions")
            .spawn()
            .expect("드릴 자식 spawn 실패")
    }

    /// 드릴 자식의 손자(`sleep 30`)를 **지금 다시 읽은** 프로세스 표에서 수집한다.
    ///
    /// ★P2-4: 손자는 `sh` 가 fork 한 뒤에야 표에 실리므로, 자식이 처음 보인 순간의 스냅샷으로는
    /// 거의 항상 빈 목록이 나온다(→ 자식만 죽고 `sleep 30` 이 ppid=1 고아로 잔존). 그래서 회수
    /// 직전에 새 표를 뜬다. 손자 생성이 늦어질 수 있으므로 최대 ~2초 짧게 폴링하되, 끝내 못
    /// 보면 빈 목록을 돌려준다(정리 실패로 테스트를 적색으로 만들지는 않는다 — 판정 축 아님).
    #[cfg(unix)]
    fn collect_grandkids_fresh(kid: u32) -> Vec<u32> {
        let mut fresh = sysinfo::System::new();
        for _ in 0..20 {
            fresh.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            let g = super::descendant_pids(&fresh, kid);
            if !g.is_empty() {
                return g;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Vec::new()
    }

    /// 지정 pid 가 프로세스 표에 자손으로 보일 때까지 최대 ~5초 기다린다.
    #[cfg(unix)]
    fn wait_until_visible(sys: &mut sysinfo::System, root: u32, want: u32) -> bool {
        for _ in 0..50 {
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            if super::descendant_pids(sys, root).contains(&want) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    }

    /// ★계측 타당성 검체(U-5) — argv 승격이 **실제로 참/거짓을 뒤집는가**.
    ///
    /// 재현하는 실측 결함: 노드가 래퍼로 뜨면(Windows 실측의 `node.exe … claude …`) 프로세스
    /// **이름**에는 에이전트 식별자가 없다. sysinfo 0.33.1 의 `refresh_processes` 는 argv 를
    /// 갱신하지 않으므로(common/system.rs:291-305) 미승격 수집기는 이름 한 토큰만 돌려주고,
    /// 그 위에서 도는 두 판정이 **원리상 거짓**이 된다:
    ///   ① `cmdline_matches_agent` (check_agent_death 의 생존 매칭 → agent_alive)
    ///   ② `--dangerously-skip-permissions` 플래그 검사 (check_launch_flags)
    ///
    /// ★적색 증명(in-band): 아래 ①' ②' 단언이 **미승격 수집기의 결과가 거짓임**을 같은
    /// 검체 안에서 박제한다. 즉 승격을 되돌리면(=with_cmd 를 미승격판으로 치환하면)
    /// ① ② 단언이 그대로 적색이 된다. 판정 축을 옮긴 것이 아니라 **같은 술어에 옳은 입력을
    /// 먹인 것**임이 이 대칭 단언으로 드러난다.
    #[cfg(unix)]
    #[test]
    fn live_argv_promotion_flips_agent_predicates_name_fallback_cannot_decide() {
        let mut child = spawn_argv_masked_child();
        let kid = child.id();
        let me = std::process::id();
        let mut sys = sysinfo::System::new();
        let visible = wait_until_visible(&mut sys, me, kid);

        let plain = super::collect_descendants(&sys, me);
        let promoted = super::collect_descendants_with_cmd(&sys, me);

        let plain_cmd = plain
            .iter()
            .find(|(p, _)| *p == kid)
            .map(|(_, c)| c.clone());
        let promoted_cmd = promoted
            .iter()
            .find(|(p, _)| *p == kid)
            .map(|(_, c)| c.clone());

        // 정리는 단언보다 먼저 — 실패해도 드릴 자식(과 그 sleep 손자)을 남기지 않는다.
        // ★P2-4: 손자 수집을 **재refresh 후**에 한다. 종전에는 `wait_until_visible` 이 남긴
        //   스냅샷(=자식이 처음 보인 순간)에서 손자를 찾았는데, 그 시점엔 `sh` 가 아직
        //   `sleep 30` 을 fork 하기 전인 경우가 대부분이라 목록이 비었고 → 자식만 kill 되어
        //   `sleep 30` 이 **ppid=1 고아**로 남았다(실측: 3회 실행 = 고아 3개).
        //   이 저장소는 자원 거버넌스가 엄격하다 — 테스트가 프로세스를 남기면 안 된다.
        let grandkids = collect_grandkids_fresh(kid);
        let _ = child.kill();
        let _ = child.wait();
        for g in grandkids {
            super::kill_pid(g);
        }

        assert!(visible, "전제 실패: 드릴 자식이 프로세스 표에 나타나지 않음");
        let plain_cmd = plain_cmd.expect("전제 실패: 미승격 수집기가 드릴 자식을 못 봄");
        let promoted_cmd = promoted_cmd.expect("전제 실패: 승격 수집기가 드릴 자식을 못 봄");

        // ①' 미승격 = 이름 한 토큰 — 에이전트 식별 불가(적색 증명의 좌변)
        assert!(
            !super::cmdline_matches_agent(&plain_cmd, "claude"),
            "미승격 관측이 에이전트를 식별해 버렸다(드릴 전제 붕괴): {plain_cmd:?}"
        );
        // ②' 미승격 = 기동 플래그 관측 불가
        assert!(
            !plain_cmd.contains("--dangerously-skip-permissions"),
            "미승격 관측에 기동 플래그가 보였다(드릴 전제 붕괴): {plain_cmd:?}"
        );

        // ① 승격 = 생존 매칭 참
        assert!(
            super::cmdline_matches_agent(&promoted_cmd, "claude"),
            "argv 승격 후에도 생존 매칭이 거짓: {promoted_cmd:?}"
        );
        // ② 승격 = 기동 플래그 관측 참
        assert!(
            promoted_cmd.contains("--dangerously-skip-permissions"),
            "argv 승격 후에도 기동 플래그가 안 보임: {promoted_cmd:?}"
        );
    }

    /// ★P1-2(치명·폭주 채널) 회귀 박제 — **관측 실패가 섞여도 이벤트는 1회를 넘지 않는다.**
    ///
    /// 【재현하는 결함】 U-5 argv 승격 이후 `check_launch_flags` 의 관측 문자열은 argv 조회
    /// 성공 여부에 따라 진동한다(Windows `argv_snapshot` = `OpenProcess` + PEB
    /// `ReadProcessMemory` — EDR·권한·종료 경주로 간헐 실패 → `name()` 폴백). 종전 2상 판정은
    /// 폴백 문자열의 "플래그 없음"을 **관측된 부정**으로 읽어, 정규 플래그로 뜬 좌석에서
    ///   성공 틱 → `warned.remove`(재무장) → 실패 틱 → 발행 → 성공 틱 → 재무장 → …
    /// 를 **watchdog 15초 주기마다 영구 반복**했다.
    ///
    /// 【적색 증명(in-band)】 아래 `two_state_reference` 는 **수정 전 판정 그대로**다(폴백을
    /// 관측으로 취급하는 2상). 같은 틱열을 두 판정에 먹여 발행 수를 비교하므로, 수리를 되돌려
    /// 3상을 2상으로 되돌리면 이 검체가 그대로 적색이 된다 — 계측기 자신을 먼저 시험한다.
    #[test]
    fn launch_flag_observation_failure_does_not_republish_the_once_only_warning() {
        use super::{decide_launch_flag_action, CmdSource, LaunchFlagAction};
        const SID: u64 = 7;
        // 현장 실제 형태 — Windows 개명 래퍼 기동의 등록 bin_base 는 확장자 없는 `claude-2` 다
        // (`cmdline_matches_agent_normalizes_windows_exec_extensions` 픽스처와 동일 지평).
        const BIN: &str = "claude-2";
        // argv 관측 성공 — 정규 플래그로 뜬 좌석.
        let ok_flag = vec![(
            101u32,
            "cmd.exe /c C:\\Users\\x\\.local\\bin\\claude-2.cmd --dangerously-skip-permissions"
                .to_string(),
            CmdSource::Argv,
        )];
        // argv 관측 성공 — 플래그 없이 수동 기동한 좌석(진짜 경고 대상).
        let ok_noflag = vec![(
            101u32,
            "cmd.exe /c C:\\Users\\x\\.local\\bin\\claude-2.cmd".to_string(),
            CmdSource::Argv,
        )];
        // argv 조회 실패(`OpenProcess`/PEB 읽기 실패) → `name()` 한 토큰으로 접힘. 매처에는
        // 걸리므로(확장자 정규화 후 basename `claude-2`) 종전 판정은 이것을 '플래그 없는 정상
        // 관측'으로 오해했다 — 그 오해가 곧 진동이고 폭주다.
        let fallback = vec![(
            101u32,
            "claude-2.exe".to_string(),
            CmdSource::NameFallback,
        )];

        // 수정 전(2상) 판정의 참조 구현 — 폴백을 관측으로 취급한다.
        let two_state_reference = |obs: &Vec<(u32, String, CmdSource)>| {
            match obs
                .iter()
                .find(|(_, c, _)| super::cmdline_matches_agent(c, BIN))
            {
                Some((_, cmdline, _)) => {
                    if cmdline.contains("--dangerously-skip-permissions") {
                        LaunchFlagAction::Rearm
                    } else {
                        LaunchFlagAction::Warn
                    }
                }
                None => LaunchFlagAction::Skip,
            }
        };

        // watchdog 틱을 그대로 흉내 낸다 — `warned` 래치 + 발행 카운터.
        let run = |ticks: &[&Vec<(u32, String, CmdSource)>],
                   decide: &dyn Fn(&Vec<(u32, String, CmdSource)>) -> LaunchFlagAction|
         -> usize {
            let mut warned: std::collections::HashSet<u64> = std::collections::HashSet::new();
            let mut published = 0usize;
            for obs in ticks {
                match decide(obs) {
                    LaunchFlagAction::Rearm => {
                        warned.remove(&SID);
                    }
                    LaunchFlagAction::Warn => {
                        if warned.insert(SID) {
                            published += 1;
                        }
                    }
                    LaunchFlagAction::Skip => {}
                }
            }
            published
        };

        // ── ① 정규 플래그 좌석 + 관측 진동(성공/실패 교대) 40틱 = 실측 10분 ──────────
        let flapping: Vec<&Vec<_>> = (0..40)
            .map(|i| if i % 2 == 0 { &ok_flag } else { &fallback })
            .collect();
        // 적색 증명의 좌변: 수정 전 판정은 좌석당 **틱 절반만큼** 발행한다.
        let before = run(&flapping, &two_state_reference);
        assert!(
            before > 1,
            "드릴 전제 붕괴: 수정 전 판정이 진동으로 재발행하지 않았다({before}건) — \
             이 검체가 P1-2 를 시험하지 못한다"
        );
        assert_eq!(before, 20, "수정 전 진동 발행 수가 예상과 다르다: {before}");
        // 수리 후: 정규 플래그 좌석이므로 경고 자체가 **0건**이어야 한다.
        let after = run(&flapping, &|o| decide_launch_flag_action(o, BIN));
        assert_eq!(
            after, 0,
            "정규 플래그 좌석인데 관측 실패가 섞였다고 경고를 발행했다({after}건) — feed 폭주 채널"
        );

        // ── ② 진짜 비정규 좌석 + 관측 진동 40틱 → 정확히 **1회**(1회 경고 계약 유지) ──
        let flapping_noflag: Vec<&Vec<_>> = (0..40)
            .map(|i| if i % 2 == 0 { &ok_noflag } else { &fallback })
            .collect();
        let after_noflag = run(&flapping_noflag, &|o| decide_launch_flag_action(o, BIN));
        assert_eq!(
            after_noflag, 1,
            "비정규 좌석 경고가 1회를 넘었다({after_noflag}건)"
        );

        // ── ③ 정규 복귀는 여전히 재무장한다(경고→복귀→재이탈 = 다시 1회) ────────────
        let cycle: Vec<&Vec<_>> = vec![&ok_noflag, &ok_noflag, &ok_flag, &ok_noflag, &ok_noflag];
        assert_eq!(
            run(&cycle, &|o| decide_launch_flag_action(o, BIN)),
            2,
            "정규 복귀 후 재이탈에서 재무장이 동작하지 않았다(1회 경고 계약 소실)"
        );

        // ── ④ 3상 진리표 직접 확인 ──────────────────────────────────────────────
        assert_eq!(
            decide_launch_flag_action(&ok_flag, BIN),
            LaunchFlagAction::Rearm
        );
        assert_eq!(
            decide_launch_flag_action(&ok_noflag, BIN),
            LaunchFlagAction::Warn
        );
        assert_eq!(
            decide_launch_flag_action(&fallback, BIN),
            LaunchFlagAction::Skip,
            "관측 실패(name 폴백)를 '플래그 없음'으로 판정했다 — 진동 → 폭주 재발"
        );
        // 에이전트 자손이 아예 없으면 종전대로 무행동(거동 무변).
        assert_eq!(
            decide_launch_flag_action(&[], BIN),
            LaunchFlagAction::Skip
        );
        assert_eq!(
            decide_launch_flag_action(
                &[(9u32, "zsh -il".to_string(), CmdSource::Argv)],
                BIN
            ),
            LaunchFlagAction::Skip
        );
        // ⑤ 한 자손의 argv 실패가 다른 자손의 성공 관측을 덮지 않는다(매칭 선택 규약).
        let mixed = vec![
            (101u32, "claude-2.exe".to_string(), CmdSource::NameFallback),
            (
                102u32,
                "C:\\Users\\x\\.local\\bin\\claude-2.exe --dangerously-skip-permissions"
                    .to_string(),
                CmdSource::Argv,
            ),
        ];
        assert_eq!(
            decide_launch_flag_action(&mixed, BIN),
            LaunchFlagAction::Rearm,
            "폴백 매치가 앞선다는 이유로 argv 관측 매치를 덮었다"
        );
    }

    /// 【P3-1 · 폭주 채널 2호】 `decide_launch_flag_action` 이 **자손 순서에 지배**되던 결함.
    ///
    /// 【결함의 기계】 종전 판정은 `.find(|(_, c, src)| src == Argv && matches(c))` — 조건에
    /// 걸리는 **첫 하나**로 Rearm/Warn 이 갈렸다. 그 "첫"의 순서는 `descendant_pids` 의
    /// children 인덱스에서 나오고, 그 인덱스는 `sys.processes()` **HashMap 순회**로 채워진다
    /// — 매 refresh 의 삽입·삭제로 순서가 바뀔 수 있다. 한편 매처(`cmdline_matches_agent`)는
    /// 오살 방지를 위해 **의도적으로 넓어**(토큰 basename + 경로 세그먼트) 좌석 자손에 매치가
    /// **여럿** 생긴다:
    ///   Windows  `powershell → cmd.exe(…\claude.cmd) → claude.exe` — 래퍼와 실물이 둘 다 매치
    ///   Unix     에이전트가 부른 `less ~/dev/claude/NOTES.md` 가 실물과 함께 매치
    /// 두 프로세스의 argv 조회(`OpenProcess`+PEB)는 **각각 독립적으로** 성공/실패하므로,
    /// 관측 목록의 순서와 가독성이 틱마다 갈아탄다:
    ///   틱 A 래퍼 먼저·무플래그 → Warn → `node.nonstandard_launch` 발행
    ///   틱 B 실물 먼저·플래그   → Rearm → 래치 해제
    ///   틱 C 다시 A            → **재발행**
    /// = 좌석당 15~30초마다 영구 발행. `check_launch_flags` 주석 자신이 "2026-07-07 feed
    /// 폭주 재발방지"라 적은 바로 그 채널이다(P1-2 가 막은 축과 **다른 축** — P1-2 는 한
    /// 프로세스의 가독성 진동, 여기는 **여러 프로세스 사이의 순서**).
    ///
    /// 【수리의 축】 판정을 **∃ 의미(순서 무관)** 로 바꾼다:
    ///   ∃ argv매치 ∧ 플래그보유 → Rearm / ∃ argv매치(전부 무플래그) → Warn / 그 외 → Skip.
    /// 주석은 이미 "argv 관측된 매치를 **우선 채택**"이라 적혀 있었고 구현만 "**첫** 매치"였다.
    ///
    /// 【적색 증명(in-band)】 `first_match_reference` 는 **수정 전 구현 그대로**다. 같은 입력을
    /// 두 판정에 먹여 (a) 순열 불변성 (b) 틱열 발행 수를 비교한다 — 수리를 되돌리면 이 검체가
    /// 그대로 적색이 된다(계측기 자신을 먼저 시험하는 이 파일의 관례).
    #[test]
    fn launch_flag_verdict_is_order_independent_across_contending_descendants() {
        use super::{decide_launch_flag_action, CmdSource, LaunchFlagAction};
        const SID: u64 = 11;
        const BIN: &str = "claude";
        const FLAG: &str = "--dangerously-skip-permissions";

        // ── 현장 형태 픽스처 ────────────────────────────────────────────────────
        // Windows 래퍼: `.cmd` 는 WIN_EXEC_EXTS 라 basename 정규화 후 `claude` 로 매치된다.
        // 래퍼 프로세스의 argv 에는 플래그가 **전달 표기로 남지 않는 경우가 있다**(cmd.exe 가
        // 인자를 재조립) — 이 검체의 전제는 "래퍼와 실물이 플래그에서 다툰다" 이다.
        let wrapper_noflag = (
            101u32,
            "cmd.exe /c C:\\Users\\x\\.local\\bin\\claude.cmd".to_string(),
            CmdSource::Argv,
        );
        let real_flag = (
            102u32,
            format!("C:\\Users\\x\\.local\\bin\\claude.exe {FLAG}"),
            CmdSource::Argv,
        );
        // Unix 등가: 에이전트가 띄운 무관 프로세스가 경로 세그먼트로 매처에 걸린다.
        let decoy_unix = (
            103u32,
            "less /home/u/dev/claude/NOTES.md".to_string(),
            CmdSource::Argv,
        );
        let real_unix_flag = (
            104u32,
            format!("/home/u/.local/bin/claude {FLAG}"),
            CmdSource::Argv,
        );
        // 실물의 argv 만 읽히지 않은 틱(래퍼는 읽힘) — 가독성 교대의 다른 반쪽.
        let real_fallback = (105u32, "claude.exe".to_string(), CmdSource::NameFallback);

        // 수정 전(첫 매치 채택) 판정의 참조 구현 — 순서에 지배된다.
        let first_match_reference = |obs: &[(u32, String, CmdSource)]| match obs
            .iter()
            .find(|(_, c, src)| *src == CmdSource::Argv && super::cmdline_matches_agent(c, BIN))
        {
            Some((_, cmdline, _)) => {
                if cmdline.contains(FLAG) {
                    LaunchFlagAction::Rearm
                } else {
                    LaunchFlagAction::Warn
                }
            }
            None => LaunchFlagAction::Skip,
        };

        // ── ① 순열 불변성 — 같은 집합이면 순서와 무관하게 같은 판정 ─────────────
        // (프로세스 표 순회 순서는 우리가 통제할 수 없는 입력이다. 판정이 그것에 의존하면
        //  그 자체로 결함이다 — 여기서 성질로 못박는다.)
        let permutations = |set: &[(u32, String, CmdSource)]| -> Vec<Vec<(u32, String, CmdSource)>> {
            // 3! 까지만 필요 — 재귀 없이 전개.
            let mut out = Vec::new();
            let n = set.len();
            let mut idx: Vec<usize> = (0..n).collect();
            // Heap's algorithm (반복형)
            let mut c = vec![0usize; n];
            out.push(idx.iter().map(|&i| set[i].clone()).collect::<Vec<_>>());
            let mut i = 0;
            while i < n {
                if c[i] < i {
                    if i % 2 == 0 {
                        idx.swap(0, i);
                    } else {
                        idx.swap(c[i], i);
                    }
                    out.push(idx.iter().map(|&k| set[k].clone()).collect::<Vec<_>>());
                    c[i] += 1;
                    i = 0;
                } else {
                    c[i] = 0;
                    i += 1;
                }
            }
            out
        };

        for set in [
            vec![wrapper_noflag.clone(), real_flag.clone()],
            vec![decoy_unix.clone(), real_unix_flag.clone()],
            vec![
                wrapper_noflag.clone(),
                decoy_unix.clone(),
                real_flag.clone(),
            ],
            vec![
                real_fallback.clone(),
                wrapper_noflag.clone(),
                real_flag.clone(),
            ],
        ] {
            let perms = permutations(&set);
            assert!(perms.len() >= 2, "순열 전개 실패(드릴 전제 붕괴)");
            let verdicts: std::collections::HashSet<LaunchFlagAction> = perms
                .iter()
                .map(|p| decide_launch_flag_action(p, BIN))
                .collect();
            assert_eq!(
                verdicts.len(),
                1,
                "판정이 자손 **순서**에 따라 갈렸다({verdicts:?}) — sys.processes() HashMap \
                 순회 순서가 이벤트 발행을 지배한다 = 폭주 채널"
            );
            assert_eq!(
                verdicts.into_iter().next().unwrap(),
                LaunchFlagAction::Rearm,
                "플래그를 가진 argv 매치가 존재하는데 Rearm 이 아니다(∃ 의미 위반)"
            );
            // 드릴 전제: 수정 전 판정은 같은 집합에서 순서에 따라 갈렸다.
            let before: std::collections::HashSet<LaunchFlagAction> = perms
                .iter()
                .map(|p| first_match_reference(p))
                .collect();
            assert!(
                before.len() > 1,
                "드릴 전제 붕괴: 수정 전 판정이 순서에 지배되지 않았다 — 이 픽스처가 P3-1 을 \
                 시험하지 못한다(집합: {set:?})"
            );
        }

        // ── ② 틱 시뮬레이션 — 순서 교대 + 가독성 교대가 동시에 일어나는 좌석 ────
        // watchdog 래치를 그대로 흉내 낸다(발행 카운터).
        let run = |ticks: &[Vec<(u32, String, CmdSource)>],
                   decide: &dyn Fn(&[(u32, String, CmdSource)]) -> LaunchFlagAction|
         -> usize {
            let mut warned: std::collections::HashSet<u64> = std::collections::HashSet::new();
            let mut published = 0usize;
            for obs in ticks {
                match decide(obs) {
                    LaunchFlagAction::Rearm => {
                        warned.remove(&SID);
                    }
                    LaunchFlagAction::Warn => {
                        if warned.insert(SID) {
                            published += 1;
                        }
                    }
                    LaunchFlagAction::Skip => {}
                }
            }
            published
        };
        // 4주기 틱열: (래퍼 먼저) → (실물 먼저) → (실물 argv 실패·래퍼 먼저) → (실물 먼저)
        // — 순서와 가독성이 동시에 갈아탄다. 실물은 정규 플래그로 떠 있으므로 **경고 0** 이 정답.
        let cycle = [
            vec![wrapper_noflag.clone(), real_flag.clone()],
            vec![real_flag.clone(), wrapper_noflag.clone()],
            vec![wrapper_noflag.clone(), real_flag.clone()],
            vec![real_flag.clone(), wrapper_noflag.clone()],
        ];
        let ticks: Vec<Vec<(u32, String, CmdSource)>> =
            (0..40).map(|i| cycle[i % cycle.len()].clone()).collect();
        let before = run(&ticks, &first_match_reference);
        assert!(
            before > 1,
            "드릴 전제 붕괴: 수정 전 판정이 순서 교대로 재발행하지 않았다({before}건)"
        );
        assert_eq!(before, 20, "수정 전 순서 진동 발행 수가 예상과 다르다: {before}");
        let after = run(&ticks, &|o| decide_launch_flag_action(o, BIN));
        assert_eq!(
            after, 0,
            "정규 플래그로 떠 있는 좌석인데 자손 순서가 바뀐다고 경고를 발행했다({after}건) \
             — feed 폭주 채널"
        );

        // ── ③ 조건 약화 없음: 진짜 비정규 좌석은 여전히 정확히 1회 경고한다 ─────
        // (argv 매치가 **전부** 무플래그 = 관측된 부정 → Warn)
        let noflag_only = [
            vec![wrapper_noflag.clone(), decoy_unix.clone()],
            vec![decoy_unix.clone(), wrapper_noflag.clone()],
        ];
        let noflag_ticks: Vec<Vec<(u32, String, CmdSource)>> = (0..40)
            .map(|i| noflag_only[i % noflag_only.len()].clone())
            .collect();
        assert_eq!(
            run(&noflag_ticks, &|o| decide_launch_flag_action(o, BIN)),
            1,
            "비정규 좌석 경고가 1회 계약을 벗어났다(약화 또는 폭주)"
        );
        assert_eq!(
            decide_launch_flag_action(&[wrapper_noflag.clone()], BIN),
            LaunchFlagAction::Warn
        );

        // ── ④ 폴백은 여전히 판정 근거가 아니다(P1-2 계약 유지) ──────────────────
        assert_eq!(
            decide_launch_flag_action(&[real_fallback.clone()], BIN),
            LaunchFlagAction::Skip,
            "관측 실패(name 폴백)를 판정 근거로 썼다 — P1-2 회귀"
        );
        // 폴백 + argv 무플래그 매치 → argv 쪽만 근거가 된다(Warn).
        assert_eq!(
            decide_launch_flag_action(&[real_fallback, wrapper_noflag], BIN),
            LaunchFlagAction::Warn
        );
    }

    /// 【P3-4 · 영구 침묵】 `Skip` 이 "관측 실패"와 "볼 것 없음"을 함께 삼키던 결함.
    ///
    /// 【결함의 기계】 P1-2 는 폭주를 막으려고 관측 실패를 `Skip`(무행동)으로 접었다. argv 조회가
    /// *간헐* 실패하는 환경에서는 그것이 정확히 옳다. 그러나 **항상** 실패하는 환경(Windows EDR
    /// 이 PEB `ReadProcessMemory` 를 전면 차단)에서는 `check_launch_flags` 가 영원히 `Skip` 만
    /// 낸다 — 개정 전에는 (틀린 이유로) 경고가 떴고 지금은 아무 말도 안 한다. 즉 **"감시자가
    /// 못 보고 있다"는 사실 자체가 관측되지 않는다**. 형태가 자가치유 전멸(재난③)과 같다:
    /// 장치는 살아 있는데 신호가 0이라 죽은 것과 구별되지 않는다.
    ///
    /// 【수리와 그 대가】 진단 이벤트를 새로 내되 **새 폭주를 만들지 않는 것**이 조건이다.
    /// 유계성은 두 겹으로 강제하고 이 검체가 그 유계성을 단언한다:
    ///   ① 연속 임계 — 한 번이라도 관측에 성공하면 streak 이 0으로 접힌다(진동은 임계 미달).
    ///   ② 쿨다운   — 임계를 넘긴 뒤에도 좌석당 `cooldown` 초에 1건이 상한.
    #[test]
    fn launch_flag_permanent_blindness_reports_once_and_stays_bounded() {
        use super::{
            decide_launch_flag_action, decide_unobservable_report, launch_flag_unobservable,
            CmdSource, LaunchFlagAction,
        };
        const BIN: &str = "claude";
        // 이 검사는 watchdog 3틱마다(=15초) 돈다 — 실측 주기를 그대로 쓴다.
        const INTERVAL: f64 = (super::WATCHDOG_INTERVAL_SECS * 3) as f64;
        const THRESHOLD: u32 = 40; // 40 × 15초 = 10분 연속 실명
        const COOLDOWN: f64 = 3600.0;

        // 틱열을 (관측성공?, 불능?) 로 주고 발행 수를 센다 — watchdog 상태를 그대로 흉내.
        let run = |ticks: &[(bool, bool)], threshold: u32| -> usize {
            let mut streak = 0u32;
            let mut last = f64::NEG_INFINITY;
            let mut emitted = 0usize;
            for (i, (obs, unobs)) in ticks.iter().enumerate() {
                if decide_unobservable_report(
                    *obs,
                    *unobs,
                    &mut streak,
                    &mut last,
                    i as f64 * INTERVAL,
                    threshold,
                    COOLDOWN,
                ) {
                    emitted += 1;
                }
            }
            emitted
        };

        // ── ① 영구 실명(12시간) — 침묵하지 않는다 ∧ 유계다 ─────────────────────
        let hours12: Vec<(bool, bool)> = vec![(false, true); (12.0 * 3600.0 / INTERVAL) as usize];
        let total_secs = hours12.len() as f64 * INTERVAL;
        let emitted = run(&hours12, THRESHOLD);
        assert!(
            emitted >= 1,
            "12시간 연속 관측 불능인데 한 번도 알리지 않았다 — 영구 침묵(재난③ 형태) 미수리"
        );
        let bound = (total_secs / COOLDOWN).ceil() as usize + 1;
        assert!(
            emitted <= bound,
            "진단 발행이 쿨다운 상한을 넘었다({emitted}건 > {bound}건) — 새 폭주를 만들었다"
        );
        // 첫 발행은 임계 **직후**여야 한다(더 일찍 = 진동에 반응, 더 늦음 = 무의미한 지연).
        let head: Vec<(bool, bool)> = vec![(false, true); THRESHOLD as usize - 1];
        assert_eq!(run(&head, THRESHOLD), 0, "임계 미달인데 발행했다");
        let head_exact: Vec<(bool, bool)> = vec![(false, true); THRESHOLD as usize];
        assert_eq!(run(&head_exact, THRESHOLD), 1, "임계 도달에서 정확히 1회여야 한다");

        // ── ② 간헐 실패(P1-2 진동)는 **절대** 발행하지 않는다 ──────────────────
        // 관측 성공이 streak 을 접으므로, 성공/실패가 갈아타는 한 임계에 닿지 못한다.
        let flapping: Vec<(bool, bool)> = (0..10_000)
            .map(|i| if i % 2 == 0 { (true, false) } else { (false, true) })
            .collect();
        assert_eq!(
            run(&flapping, THRESHOLD),
            0,
            "간헐 argv 실패(P1-2 진동)에 진단이 반응했다 — 폭주 채널을 새로 열었다"
        );
        // 임계 직전까지 갔다가 한 번 성공해도 재무장된다(연속의 의미).
        let mut near: Vec<(bool, bool)> = vec![(false, true); THRESHOLD as usize - 1];
        near.push((true, false));
        near.extend(vec![(false, true); THRESHOLD as usize - 1]);
        assert_eq!(run(&near, THRESHOLD), 0, "성공 1틱이 연속 카운터를 접지 못했다");

        // ── ③ '볼 것이 없는' 좌석은 진단 대상이 아니다(빈 좌석 잡음 금지) ───────
        let nothing: Vec<(bool, bool)> = vec![(false, false); 10_000];
        assert_eq!(
            run(&nothing, THRESHOLD),
            0,
            "에이전트 자손이 아예 없는 좌석에 실명 진단을 냈다"
        );

        // ── ④ 킬스위치(threshold=0)는 완전 비활성 ───────────────────────────────
        assert_eq!(run(&hours12, 0), 0, "threshold=0 킬스위치가 동작하지 않았다");

        // ── ⑤ `launch_flag_unobservable` 진리표 — 진단 게이트가 정확히 `Skip` 의 \
        //      '관측 실패' 쪽만 덮는가 ────────────────────────────────────────────
        let argv_flag = (
            201u32,
            "/home/u/.local/bin/claude --dangerously-skip-permissions".to_string(),
            CmdSource::Argv,
        );
        let argv_noflag = (202u32, "/home/u/.local/bin/claude".to_string(), CmdSource::Argv);
        let fb_match = (203u32, "claude".to_string(), CmdSource::NameFallback);
        let fb_other = (204u32, "zsh".to_string(), CmdSource::NameFallback);
        let argv_other = (205u32, "zsh -il".to_string(), CmdSource::Argv);

        assert!(
            launch_flag_unobservable(&[fb_match.clone()], BIN),
            "매처에 걸리는 자손의 argv 를 못 읽었는데 불능이 아니라고 했다"
        );
        assert_eq!(
            decide_launch_flag_action(&[fb_match.clone()], BIN),
            LaunchFlagAction::Skip,
            "진단 게이트와 판정 분기가 어긋났다(불능인데 Skip 이 아니다)"
        );
        // argv 를 하나라도 읽었으면 불능이 아니다 — 판정이 가능했다.
        assert!(!launch_flag_unobservable(&[argv_flag.clone()], BIN));
        assert!(!launch_flag_unobservable(&[argv_noflag.clone()], BIN));
        assert!(!launch_flag_unobservable(
            &[fb_match.clone(), argv_flag.clone()],
            BIN
        ));
        // 매치가 아예 없으면 불능이 아니다(볼 것이 없는 것 ≠ 못 보는 것).
        assert!(!launch_flag_unobservable(&[], BIN));
        assert!(!launch_flag_unobservable(&[fb_other, argv_other], BIN));
        // 발행 분기(Rearm|Warn)와 진단은 상호 배타 — 이중 신호 금지.
        for obs in [vec![argv_flag], vec![argv_noflag]] {
            let act = decide_launch_flag_action(&obs, BIN);
            assert!(matches!(
                act,
                LaunchFlagAction::Rearm | LaunchFlagAction::Warn
            ));
            assert!(
                !launch_flag_unobservable(&obs, BIN),
                "관측 성공 틱에 실명 진단이 함께 켜졌다(이중 신호)"
            );
        }
    }

    /// 【P3-6 · 고아 좌석】 `check_agent_death` 의 `alive` 가 U-5 argv 승격으로 넓어졌는데
    /// **보정이 없던** 결함. 라운드 1 은 같은 넓힘의 *다른 소비자*(readiness 안전 밸브 —
    /// `refresh_seat_cache` 의 `seat_agent_cache`)만 보고 거기에만 엄격 매처 AND 를 걸었다.
    ///
    /// 【결함의 기계】 U-5 이전의 관측 문자열은 `name()` **한 토큰**이라 경로 구분자가 없었고,
    /// 그래서 `cmdline_matches_agent` 의 경로 세그먼트 규칙은 원리상 발화할 수 없었다. 승격 후
    /// 관측이 명령줄 전체가 되면서, 좌석 자손 중 **아무거나** argv 에 `…/claude/…` 세그먼트를
    /// 가진 채 살아 있으면 `alive=true` 가 된다:
    ///   `tail -f ~/.cys/claude/session.log` · `less ~/dev/claude/NOTES.md`
    ///   `grep -rn foo ~/dev/claude-code/src`
    /// 에이전트가 죽어도 이런 자손 하나가 남아 있으면 `agent.exited` 미발행 → node-recover
    /// 미발동 → **고아 좌석**(좌석은 점유된 것처럼 보이는데 그 위에 에이전트가 없다).
    ///
    /// 【수리와 그 정당화 — 오살 0칸】 좁힘은 **그 좌석에서 엄격 매처가 자기 에이전트를 실제로
    /// 본 적이 있을 때만** 켠다(`update_strict_proof`). 증명된 좌석에서는 "에이전트가 살아
    /// 있다 ⇒ 엄격 매치가 있다"가 관측으로 성립하므로, 엄격 매치가 사라지고 광의 매치만
    /// 남은 상태는 **그 광의 매치가 에이전트가 아니라는 뜻**이다. 미증명 좌석(엄격 매처로는
    /// 보이지 않는 형태로 뜨는 가상의 에이전트)에서는 좁히지 않고 종전 거동을 그대로 둔다 —
    /// 좁힘이 자기 안전성을 스스로 입증한 좌석에서만 켜지는 자기보호적 설계다.
    ///
    /// 【적색 증명(in-band)】 `broad_any_reference` 는 **수정 전 한 줄 그대로**다. 고아 시나리오
    /// 틱열을 두 판정에 먹여 (a) 수정 전은 영구 alive(=고아) (b) 수정 후는 사망 판정으로
    /// 넘어감을 대조한다.
    #[test]
    fn agent_liveness_ignores_broad_only_evidence_once_strict_is_proven() {
        use super::{
            agent_alive_from_liveness, cmdline_matches_agent, cmdline_matches_agent_exec,
            decide_agent_liveness, update_strict_proof, AgentLiveness,
        };
        const BIN: &str = "claude";
        const ARM: u32 = 2;

        // ── 실물 에이전트 형태(전부 **엄격** 증거) — 기존 검체들이 쓰는 실측 코퍼스 ──
        let real_forms: [&str; 6] = [
            "/Users/x/.local/bin/claude --dangerously-skip-permissions",
            "node /Users/x/.npm-global/bin/claude",
            "node /n/m/@anthropic-ai/claude-code/cli.js",
            "node --enable-source-maps /usr/local/lib/node_modules/@anthropic-ai/claude-code/cli.js",
            "cmd.exe /c C:\\Users\\x\\.local\\bin\\claude.cmd",
            "C:\\Users\\x\\.local\\bin\\claude.exe --dangerously-skip-permissions",
        ];
        // ── U-5 가 새로 만든 오탐 축(광의로만 걸린다 = 실행 증거 아님) ──────────────
        let broad_only_decoys: [&str; 4] = [
            "tail -f /Users/x/.cys/claude/session.log",
            "less /home/u/dev/claude/NOTES.md",
            "grep -rn foo /home/u/dev/claude-code/src",
            "python3 /opt/claude/tools/report.py",
        ];
        // ── 아무 관계 없는 자손 ────────────────────────────────────────────────────
        let unrelated: [&str; 3] = ["zsh -il", "vim notes.md", "node /x/y.js"];

        // ── ① 증거 등급 분류 + strict ⊆ broad 포함관계 ──────────────────────────
        for f in real_forms {
            assert!(
                cmdline_matches_agent_exec(f, BIN) && cmdline_matches_agent(f, BIN),
                "실물 형태가 엄격 증거로 잡히지 않는다(좁힘의 안전 전제 붕괴): {f}"
            );
            assert_eq!(
                decide_agent_liveness(&[f.to_string()], BIN),
                AgentLiveness::AliveStrict,
                "실물 형태 등급 오분류: {f}"
            );
        }
        for d in broad_only_decoys {
            assert!(
                cmdline_matches_agent(d, BIN) && !cmdline_matches_agent_exec(d, BIN),
                "이 픽스처가 '광의만' 축을 시험하지 못한다: {d}"
            );
            assert_eq!(
                decide_agent_liveness(&[d.to_string()], BIN),
                AgentLiveness::AliveBroadOnly,
                "광의 전용 오탐 등급 오분류: {d}"
            );
        }
        for u in unrelated {
            assert_eq!(
                decide_agent_liveness(&[u.to_string()], BIN),
                AgentLiveness::NoEvidence,
                "무관 자손을 증거로 셌다: {u}"
            );
        }
        // 등급은 ∃ 이라 **순서 무관**이다(P3-1 과 동일한 성질 — 프로세스 표 순회 순서 비의존).
        let mixed = vec![
            broad_only_decoys[0].to_string(),
            real_forms[2].to_string(),
            unrelated[0].to_string(),
        ];
        let mut rev = mixed.clone();
        rev.reverse();
        assert_eq!(decide_agent_liveness(&mixed, BIN), AgentLiveness::AliveStrict);
        assert_eq!(decide_agent_liveness(&rev, BIN), AgentLiveness::AliveStrict);

        // ── ② 오살 0칸(제1 계약) — 실물이 살아 있으면 증명 여부와 무관하게 항상 생존 ──
        for f in real_forms {
            for d in broad_only_decoys {
                let cmds = vec![d.to_string(), f.to_string()];
                for proven in [false, true] {
                    assert!(
                        agent_alive_from_liveness(decide_agent_liveness(&cmds, BIN), proven),
                        "살아있는 에이전트를 죽었다고 판정했다(오살) — proven={proven} \
                         real={f} decoy={d}"
                    );
                }
            }
        }
        // 미증명 좌석은 광의 증거만으로도 종전대로 생존이다(좁힘을 켜지 않는다).
        for d in broad_only_decoys {
            assert!(
                agent_alive_from_liveness(
                    decide_agent_liveness(&[d.to_string()], BIN),
                    false
                ),
                "미증명 좌석에서 좁힘이 켜졌다 — 종전 거동 보존 위반: {d}"
            );
        }

        // ── ③ 고아 수리: 증명된 좌석에서는 광의 전용 증거가 생존이 아니다 ─────────
        for d in broad_only_decoys {
            assert!(
                !agent_alive_from_liveness(decide_agent_liveness(&[d.to_string()], BIN), true),
                "증명된 좌석인데 비에이전트 자손을 생존 증거로 승격했다(고아 좌석): {d}"
            );
        }
        // 증거가 아예 없으면 종전과 동일하게 사망 후보(거동 무변).
        assert!(!agent_alive_from_liveness(AgentLiveness::NoEvidence, false));
        assert!(!agent_alive_from_liveness(AgentLiveness::NoEvidence, true));

        // ── ④ 증명 래치 히스테리시스 ────────────────────────────────────────────
        let mut proof: Option<(String, u32)> = None;
        assert!(
            !update_strict_proof(&mut proof, BIN, true, ARM),
            "1틱 엄격 관측으로 증명이 섰다 — 스치는 도우미 프로세스에 좁힘이 켜진다"
        );
        assert!(update_strict_proof(&mut proof, BIN, true, ARM), "연속 2틱이면 증명");
        // 한 번 선 증명은 내리지 않는다(에이전트 사망으로 엄격 증거가 사라지는 것이 표적).
        assert!(update_strict_proof(&mut proof, BIN, false, ARM));
        assert!(update_strict_proof(&mut proof, BIN, false, ARM));
        // 증명 전 연속이 끊기면 처음부터.
        let mut p2: Option<(String, u32)> = None;
        assert!(!update_strict_proof(&mut p2, BIN, true, ARM));
        assert!(!update_strict_proof(&mut p2, BIN, false, ARM));
        assert!(!update_strict_proof(&mut p2, BIN, true, ARM), "끊긴 연속이 이어졌다");
        assert!(update_strict_proof(&mut p2, BIN, true, ARM));
        // 좌석의 에이전트가 바뀌면(bin_base 변경) 증명은 처음부터 다시 쌓는다.
        assert!(!update_strict_proof(&mut p2, "codex", true, ARM));
        assert!(update_strict_proof(&mut p2, "codex", true, ARM));
        // 킬스위치: arm_ticks=0 이면 절대 증명되지 않는다(= 좁힘 전면 비활성·종전 거동).
        let mut p3: Option<(String, u32)> = None;
        for _ in 0..100 {
            assert!(
                !update_strict_proof(&mut p3, BIN, true, 0),
                "arm_ticks=0 킬스위치가 동작하지 않았다"
            );
        }

        // ── ⑤ 고아 시나리오 틱 시뮬레이션(수리 전/후 대조) ──────────────────────
        // 틱 1..=4  실물 + decoy 공존(정상 가동)
        // 틱 5..=40 실물 사망, decoy 만 잔존 → 여기서 사망 판정이 나와야 한다.
        let broad_any_reference =
            |cmds: &[String]| cmds.iter().any(|c| cmdline_matches_agent(c, BIN));
        let ticks: Vec<Vec<String>> = (0..40)
            .map(|i| {
                if i < 4 {
                    vec![broad_only_decoys[0].to_string(), real_forms[0].to_string()]
                } else {
                    vec![broad_only_decoys[0].to_string()]
                }
            })
            .collect();
        let mut proof: Option<(String, u32)> = None;
        let mut alive_after: Vec<bool> = Vec::new();
        for cmds in &ticks {
            let liveness = decide_agent_liveness(cmds, BIN);
            let proven = update_strict_proof(
                &mut proof,
                BIN,
                liveness == AgentLiveness::AliveStrict,
                ARM,
            );
            alive_after.push(agent_alive_from_liveness(liveness, proven));
        }
        let alive_before: Vec<bool> = ticks.iter().map(|c| broad_any_reference(c)).collect();
        assert!(
            alive_before.iter().all(|&a| a),
            "드릴 전제 붕괴: 수정 전 판정이 이 틱열에서 사망을 냈다 — 고아를 시험하지 못한다"
        );
        assert!(
            alive_after[..4].iter().all(|&a| a),
            "에이전트가 살아 있는 구간에서 사망 판정이 났다(오살)"
        );
        assert!(
            alive_after[4..].iter().all(|&a| !a),
            "에이전트 사망 후에도 잔존 자손 때문에 생존으로 읽혔다 — 고아 좌석 미수리"
        );

        // ⑥ 증명이 서기 전에 에이전트가 죽으면(엄격 관측 1틱뿐) 좁히지 않는다 —
        //    안전 쪽(종전 거동)으로 접힌다. 증명은 안전의 전제이지 편의가 아니다.
        let short: Vec<Vec<String>> = vec![
            vec![broad_only_decoys[1].to_string(), real_forms[0].to_string()],
            vec![broad_only_decoys[1].to_string()],
            vec![broad_only_decoys[1].to_string()],
        ];
        let mut p4: Option<(String, u32)> = None;
        let verdicts: Vec<bool> = short
            .iter()
            .map(|cmds| {
                let l = decide_agent_liveness(cmds, BIN);
                let pr = update_strict_proof(&mut p4, BIN, l == AgentLiveness::AliveStrict, ARM);
                agent_alive_from_liveness(l, pr)
            })
            .collect();
        assert_eq!(
            verdicts,
            vec![true, true, true],
            "증명 미성립 좌석에서 좁힘이 켜졌다 — 히스테리시스 무력"
        );
    }

    /// `collect_descendants_with_cmd_src` 가 출처를 정직하게 싣는가 — 얇은 래퍼가 종전 반환값과
    /// 완전히 동형인지도 함께 확인한다(기존 소비자 거동 무변 증명).
    #[cfg(unix)]
    #[test]
    fn cmd_source_is_reported_and_wrapper_stays_identical() {
        use super::CmdSource;
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let me = std::process::id();
        let with_src = super::collect_descendants_with_cmd_src(&sys, me);
        let plain = super::collect_descendants_with_cmd(&sys, me);
        let folded: Vec<(u32, String)> = with_src
            .iter()
            .map(|(p, c, _)| (*p, c.clone()))
            .collect();
        assert_eq!(folded, plain, "얇은 래퍼가 종전 반환값과 달라졌다(소비자 거동 변경)");
        for (pid, cmd, src) in &with_src {
            match src {
                // argv 로 읽었다면 문자열이 비지 않는다(argv_snapshot 은 빈 argv 를 넣지 않는다).
                CmdSource::Argv => assert!(!cmd.is_empty(), "argv 출처인데 문자열이 비었다: {pid}"),
                // 폴백은 name() 한 토큰 — 공백이 없거나(단일 토큰) 표에서 사라져 빈 문자열이다.
                CmdSource::NameFallback => {}
            }
        }
    }

    /// 승격판이 argv 를 못 읽는 프로세스에서 **종전 동작으로 떨어지는가**(fail-same).
    /// pid 1(init/launchd)은 통상 argv 조회가 막히거나 비므로 폴백 경로를 밟는다 —
    /// 어느 쪽이든 "빈 문자열이 아니다"가 계약이다(관측 소실 금지).
    #[cfg(unix)]
    #[test]
    fn argv_promotion_falls_back_to_name_when_argv_unreadable() {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let me = std::process::id();
        let plain = super::collect_descendants(&sys, me);
        let promoted = super::collect_descendants_with_cmd(&sys, me);
        // 같은 표·같은 순간이므로 pid 집합과 순서가 동일해야 한다(트리 골격 공유 증명).
        let a: Vec<u32> = plain.iter().map(|(p, _)| *p).collect();
        let b: Vec<u32> = promoted.iter().map(|(p, _)| *p).collect();
        assert_eq!(a, b, "승격판이 자손 트리 골격을 바꿨다");
        for (i, (_, cmd)) in promoted.iter().enumerate() {
            if plain[i].1.is_empty() {
                continue; // 표에서 사라진 pid — 양쪽 모두 빈 문자열
            }
            assert!(
                !cmd.is_empty(),
                "승격판이 관측을 잃었다(폴백 미작동): pid={}",
                plain[i].0
            );
        }
    }

    /// ★비용 계측(U-5 게이트 ③) — 승격 전/후 '틱 1회분' 소요 시간.
    /// CI 게이트가 아니라 **측정 전용**이다(머신 부하에 좌우되는 수치를 초록/적색으로 바꾸면
    /// 그것이 곧 완화의 통로가 된다). `cargo test --bins -- --ignored --nocapture` 로 재현한다.
    #[cfg(unix)]
    #[test]
    #[ignore = "측정 전용 — 수치는 보고서에 박제, CI 판정 축 아님"]
    fn bench_argv_promotion_cost_versus_watchdog_tick() {
        const SEATS: usize = 5;
        let mut kids: Vec<std::process::Child> = (0..SEATS).map(|_| spawn_argv_masked_child()).collect();
        let roots: Vec<u32> = kids.iter().map(|c| c.id()).collect();
        let me = std::process::id();
        let mut sys = sysinfo::System::new();
        for r in &roots {
            wait_until_visible(&mut sys, me, *r);
        }
        let bench = |label: &str, f: &mut dyn FnMut()| {
            let n = 20;
            let t0 = std::time::Instant::now();
            for _ in 0..n {
                f();
            }
            let per = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
            println!("[U-5 bench] {label}: {per:.3} ms/회");
            per
        };
        let mut s2 = sysinfo::System::new();
        let t_refresh = bench("전역 refresh_processes(All) — 틱의 기존 고정비", &mut || {
            s2.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        });
        let t_plain = bench("승격 전: collect_descendants × 5좌석", &mut || {
            for r in &roots {
                std::hint::black_box(super::collect_descendants(&sys, *r));
            }
        });
        let t_prom = bench("승격 후: collect_descendants_with_cmd × 5좌석", &mut || {
            for r in &roots {
                std::hint::black_box(super::collect_descendants_with_cmd(&sys, *r));
            }
        });
        println!(
            "[U-5 bench] 틱 증분 = {:.3} ms (전역 refresh 대비 {:.1}%)",
            t_prom - t_plain,
            (t_prom - t_plain) / t_refresh * 100.0
        );
        // ★P2-4 와 같은 이유로 손자는 **재refresh 후** 수집한다(고아 `sleep 30` 잔존 차단).
        for c in kids.iter_mut() {
            let g = collect_grandkids_fresh(c.id());
            let _ = c.kill();
            let _ = c.wait();
            for p in g {
                super::kill_pid(p);
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ★T-0147-2 §2 층3 A3′(R2-C3) — queue.delivered 의 W-id 에코
    // ─────────────────────────────────────────────────────────────────────────

    /// 단건: 대괄호·공백에 둘러싸여도 hex 종료 경계에서 정확히 끊긴다.
    #[test]
    fn wakeup_entry_ids_extracts_single_id_with_boundary() {
        assert_eq!(
            wakeup_entry_ids("[wakeup W-a1b2c3d4e5] master 확인 요망"),
            vec!["W-a1b2c3d4e5".to_string()]
        );
        // 줄바꿈·구두점도 종료 경계
        assert_eq!(wakeup_entry_ids("id=W-0f9\n다음 줄"), vec!["W-0f9".to_string()]);
    }

    /// digest 3건: 병합 배달의 **전** id 가 순서대로 나와야 critical-tier 가 disarm 된다
    /// (하나라도 누락되면 그 사건은 inflight 로 남아 TTL 마다 영구 재enqueue = 홍수 재발).
    #[test]
    fn wakeup_entry_ids_extracts_all_digest_ids_in_order_dedup() {
        let text = "[digest 3건]\n- W-aaa1 노드 사망\n- W-bbb2 stall\n- W-ccc3 데드락\n(재게시 W-aaa1)";
        assert_eq!(
            wakeup_entry_ids(text),
            vec!["W-aaa1".to_string(), "W-bbb2".to_string(), "W-ccc3".to_string()],
            "등장 순서 보존 + 중복 제거"
        );
    }

    /// 0건: 봉입 id 없는 일반 큐 배달은 빈 배열(에코 계약상 키는 항상 존재).
    #[test]
    fn wakeup_entry_ids_empty_when_no_token() {
        assert!(wakeup_entry_ids("평범한 큐 메시지 — id 없음").is_empty());
        assert!(wakeup_entry_ids("").is_empty());
    }

    /// 오형: `W-` 뒤 hex 0자(`W-`, `W-ZZZZ`, 대문자 hex)는 전부 무시. 겹친 `W-W-a1` 도 회수.
    #[test]
    fn wakeup_entry_ids_rejects_malformed_tokens() {
        assert!(wakeup_entry_ids("W-").is_empty());
        assert!(wakeup_entry_ids("W- a1b2").is_empty(), "공백은 hex 아님");
        assert!(wakeup_entry_ids("W-ZZZZ").is_empty());
        assert!(wakeup_entry_ids("W-A1B2").is_empty(), "hex 는 소문자 [0-9a-f] 만");
        assert_eq!(wakeup_entry_ids("W-W-a1"), vec!["W-a1".to_string()]);
    }

    /// 상한: id 32개에서 절단(페이로드 비대 차단) · hex 본문도 32자에서 절단.
    #[test]
    fn wakeup_entry_ids_caps_count_and_hex_length() {
        let many: String = (0..40).map(|i| format!("W-{i:04x} ")).collect();
        assert_eq!(wakeup_entry_ids(&many).len(), 32);
        let long = format!("W-{}", "a".repeat(40));
        assert_eq!(wakeup_entry_ids(&long), vec![format!("W-{}", "a".repeat(32))]);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ★T-0147-2 층1 I4 — 승인 wakeup 중복 억제(창 5분)
    // ─────────────────────────────────────────────────────────────────────────

    /// 직전 큐에 동일 문구가 배달 대기 중이면 억제 — 같은 사실로 master 를 두 번 깨우지 않는다.
    #[test]
    fn approval_wakeup_suppressed_when_same_text_still_queued() {
        // ★G1(W2-A): 원소가 QueueEntry로 승격돼도 dedupe는 문구 단위(id 무관) — 의미 불변 핀.
        let mut q = std::collections::VecDeque::new();
        q.push_back(qe("aw-1", "[승인감지] claude surface:7 …"));
        let recent = std::collections::HashMap::new();
        assert!(approval_wakeup_suppressed(
            &q,
            &recent,
            "[승인감지] claude surface:7 …",
            1000.0,
            300.0
        ));
        // 다른 문구는 통과(다른 사실은 깨울 가치가 있다)
        assert!(!approval_wakeup_suppressed(
            &q,
            &recent,
            "[승인감지] codex surface:9 …",
            1000.0,
            300.0
        ));
    }

    /// 창 안/밖 경계 — 배달돼 큐에서 사라진 뒤에도 창 동안은 억제, 창을 넘기면 재발화.
    #[test]
    fn approval_wakeup_window_boundary_in_and_out() {
        let q = std::collections::VecDeque::new();
        let text = "[승인감지] claude surface:7 …";
        let mut recent = std::collections::HashMap::new();
        recent.insert(super::approval_wakeup_hash(text), 1000.0);
        assert!(
            approval_wakeup_suppressed(&q, &recent, text, 1299.0, 300.0),
            "창 안(299s<300s) 억제"
        );
        assert!(
            !approval_wakeup_suppressed(&q, &recent, text, 1300.0, 300.0),
            "창 경계(=300s) 부터 재발화 — 방치 차단 원목적 보존"
        );
        assert!(!approval_wakeup_suppressed(&q, &recent, text, 9999.0, 300.0));
    }

    /// window 0 = 노브 비활성 → 종전 무억제 동작으로 정확히 복귀(회귀 탈출구).
    #[test]
    fn approval_wakeup_dedupe_disabled_when_window_zero() {
        let mut q = std::collections::VecDeque::new();
        let text = "[승인감지] claude surface:7 …";
        q.push_back(qe("aw-0", text));
        let mut recent = std::collections::HashMap::new();
        recent.insert(super::approval_wakeup_hash(text), 1000.0);
        assert!(!approval_wakeup_suppressed(&q, &recent, text, 1000.0, 0.0));
        assert!(!approval_wakeup_suppressed(&q, &recent, text, 1000.0, -1.0));
    }

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

    // ─────────────────────────────────────────────────────────────────────────
    // ★T3-G2 중복 서버 오탐 수리 — 양방향 대조(수용 기준).
    //   ① 정상 편성 시뮬(노드 5 = claude 5 · powershell 5) → 경보 0
    //   ② 진짜 중복 시뮬(동일 종단점 서버 2개 / 한 surface 안 동일 명령 3개) → 경보 발화
    // 델타 증명: 같은 ①입력을 **수리 전 판정 미러**에 넣으면 실사고 그대로 경보가 나온다.
    // ─────────────────────────────────────────────────────────────────────────

    /// ★수리 전(pre-fix) 판정 미러 — 구 `check_surfaces` 의 이름 전역 계수 그대로다
    /// (`cmdline_groups: HashMap<String, Vec<u32>>` 에 전 surface 자손을 부어 넣고
    /// `pids.len() >= threshold` 면 발화). 비교 기준을 코드로 박제한다
    /// (state.rs `health_matches_pre_fix` 와 같은 관행).
    fn dup_alerts_pre_fix(obs: &[ProcObs], threshold: usize) -> Vec<(String, usize)> {
        let mut groups: HashMap<String, Vec<u32>> = HashMap::new();
        for o in obs {
            if !o.cmdline.is_empty() {
                groups.entry(o.cmdline.clone()).or_default().push(o.pid);
            }
        }
        let mut out: Vec<(String, usize)> = groups
            .into_iter()
            .filter(|(_, pids)| pids.len() >= threshold)
            .map(|(c, pids)| (c, pids.len()))
            .collect();
        out.sort();
        out
    }

    /// 5노드 정상 편성 관측치: surface 마다 셸(powershell.exe) 1 + 에이전트 CLI(claude.exe) 1.
    /// 실사고 스크린샷("중복 서버 4개: powershell.exe" · "중복 서버 5개: claude.exe")의 구성이다.
    fn normal_formation_obs() -> Vec<ProcObs> {
        let agent_bins = vec!["claude.exe".to_string()];
        let mut obs = Vec::new();
        for i in 0..5u64 {
            let root = 1000 + (i as u32) * 10;
            for (off, cmdline) in [(1u32, "powershell.exe -NoLogo"), (2, "claude.exe --resume")] {
                let cmdline = cmdline.to_string();
                obs.push(ProcObs {
                    pid: root + off,
                    ppid: if off == 1 { root } else { root + 1 },
                    surface_id: i,
                    node_owned: is_node_owned(&cmdline, &agent_bins),
                    cmdline,
                    age_secs: 3600.0,
                });
            }
        }
        obs
    }

    /// ★①정상 편성 → 경보 0 (+ 수리 전 미러에서는 경보가 났다는 델타 증명).
    #[test]
    fn normal_five_node_formation_raises_zero_duplicate_alerts() {
        let obs = normal_formation_obs();

        // 델타(전제): 수리 전 이름 전역 계수는 실사고 그대로 2건을 발화했다.
        let pre = dup_alerts_pre_fix(&obs, 3);
        assert_eq!(
            pre,
            vec![
                ("claude.exe --resume".to_string(), 5),
                ("powershell.exe -NoLogo".to_string(), 5),
            ],
            "전제 실패: 수리 전 미러가 실사고를 재현하지 못함"
        );

        // 수리 후: 소유 제외 + surface 스코프 계수 → 0건.
        let after = plan_duplicate_alerts(&obs, 3, 2, 45.0);
        assert!(after.is_empty(), "정상 편성에서 중복 경보 발화: {after:#?}");

        // 소유 판정 자체도 핀 — 노드 CLI·자기 셸은 전부 노드 소속으로 인식돼야 한다.
        assert!(obs.iter().all(|o| o.node_owned), "노드 인프라 소유 판정 누락: {obs:#?}");
    }

    /// 노드가 20개로 늘어도 경보는 0 — "편성 규모가 곧 경보"였던 구조적 결함의 회귀 가드.
    #[test]
    fn formation_scale_does_not_create_alerts() {
        let agent_bins = vec!["claude".to_string()];
        let obs: Vec<ProcObs> = (0..20u64)
            .map(|i| ProcObs {
                pid: 5000 + i as u32,
                ppid: 1,
                surface_id: i,
                cmdline: "/usr/local/bin/claude --dangerously-skip-permissions".into(),
                node_owned: is_node_owned(
                    "/usr/local/bin/claude --dangerously-skip-permissions",
                    &agent_bins,
                ),
                age_secs: 3600.0,
            })
            .collect();
        assert!(plan_duplicate_alerts(&obs, 3, 2, 45.0).is_empty());
    }

    /// ★②-a 진짜 중복: 서로 다른 노드(=역할)가 **같은 포트**를 점유 → 임계 2에서 발화.
    #[test]
    fn same_endpoint_held_by_two_roles_fires_alert() {
        let mk = |pid: u32, sid: u64, cmd: &str| ProcObs {
            pid,
            ppid: 1,
            surface_id: sid,
            cmdline: cmd.to_string(),
            node_owned: false,
            age_secs: 3600.0,
        };
        let obs = vec![
            mk(2001, 1, "bun /work/api/server.ts --port 3000"),
            // 이름도 소유 노드도 다르지만 같은 포트 = 진짜 충돌
            mk(2002, 2, "node /work/api/dist/main.js --port=3000"),
            // 무관한 다른 포트는 그룹이 다르다(오탐 금지)
            mk(2003, 3, "vite --port 5173"),
        ];
        let alerts = plan_duplicate_alerts(&obs, 3, 2, 45.0);
        assert_eq!(alerts.len(), 1, "동일 포트 2개 점유가 발화하지 않음: {alerts:#?}");
        assert_eq!(alerts[0].scope, "endpoint");
        assert_eq!(alerts[0].key, "endpoint:port:3000");
        assert_eq!(alerts[0].pids, vec![2001, 2002]);
        assert!(!alerts[0].killable, "종단점 판정은 경보만 — 자동 kill 금지");
    }

    /// ★②-b 진짜 중복: **한 surface 안**에 같은 명령 3개(실사고 원형 `bun server.ts × 36`).
    #[test]
    fn same_command_piled_in_one_surface_fires_alert_and_is_killable() {
        let mk = |pid: u32| ProcObs {
            pid,
            ppid: 900,
            surface_id: 7,
            cmdline: "bun /work/server.ts".into(),
            node_owned: false,
            age_secs: 3600.0,
        };
        let obs = vec![mk(3003), mk(3001), mk(3002)];
        let alerts = plan_duplicate_alerts(&obs, 3, 2, 45.0);
        assert_eq!(alerts.len(), 1, "한 surface 내 3중복이 발화하지 않음: {alerts:#?}");
        assert_eq!(alerts[0].scope, "surface");
        assert_eq!(alerts[0].surface_id, Some(7));
        assert_eq!(alerts[0].pids, vec![3001, 3002, 3003], "pid asc 결정론 정렬");
        assert!(alerts[0].killable, "한 surface 내 동일 명령 누적은 자동 정리 대상");
    }

    /// 같은 명령이 여러 surface 에 **1개씩** 흩어진 것은 중복이 아니다(스코프 분리의 핵심).
    #[test]
    fn same_command_spread_one_per_surface_is_not_duplicate() {
        let mk = |pid: u32, sid: u64| ProcObs {
            pid,
            ppid: 900,
            surface_id: sid,
            cmdline: "python3 -m http.server".into(),
            node_owned: false,
            age_secs: 3600.0,
        };
        let obs = vec![mk(4001, 1), mk(4002, 2), mk(4003, 3), mk(4004, 4)];
        assert!(plan_duplicate_alerts(&obs, 3, 2, 45.0).is_empty());
    }

    /// 래퍼(부모)와 서버(자식)가 같은 종단점을 물려받아도 2개로 세지 않는다(불변식 ④).
    #[test]
    fn wrapper_parent_and_child_sharing_endpoint_collapse_to_one() {
        let obs = vec![
            ProcObs { pid: 100, ppid: 1, surface_id: 1, age_secs: 3600.0,
                      cmdline: "sh -c bun server.ts --port 8080".into(), node_owned: false },
            ProcObs { pid: 101, ppid: 100, surface_id: 1, age_secs: 3600.0,
                      cmdline: "bun server.ts --port 8080".into(), node_owned: false },
        ];
        assert!(
            plan_duplicate_alerts(&obs, 3, 2, 45.0).is_empty(),
            "래퍼-자식 사슬을 중복으로 오판"
        );
    }

    /// 소유 제외는 자동 정리 후보에서도 노드 CLI 를 영구 배제한다 —
    /// 구 로직 + `CYS_AUTOKILL_DUP=1` 이면 정상 편성 5노드 중 **4개를 죽였을** 잠복 결함의 가드.
    #[test]
    fn node_cli_is_never_a_kill_candidate_even_when_piled() {
        let agent_bins = vec!["claude".to_string()];
        let obs: Vec<ProcObs> = (0..4u32)
            .map(|i| ProcObs {
                pid: 7000 + i,
                ppid: 900,
                surface_id: 3, // 같은 surface 안에 4개 — 스코프 계수로도 임계 초과 조건
                cmdline: "claude --resume".into(),
                node_owned: is_node_owned("claude --resume", &agent_bins),
                age_secs: 3600.0,
            })
            .collect();
        assert!(
            plan_duplicate_alerts(&obs, 3, 2, 45.0).is_empty(),
            "노드 CLI 가 중복/kill 후보로 승격됨 — 오살 경로"
        );
    }

    /// 종단점 추출 계약 핀 — 명시 장문 플래그만 신뢰하고 `-p` 류 과적 플래그는 보지 않는다.
    #[test]
    fn endpoint_key_extracts_explicit_forms_only() {
        assert_eq!(endpoint_key("bun server.ts --port 3000"), Some("port:3000".into()));
        assert_eq!(endpoint_key("node main.js --port=3000"), Some("port:3000".into()));
        assert_eq!(endpoint_key("uvicorn app:app --bind 0.0.0.0:8000"), Some("port:8000".into()));
        assert_eq!(endpoint_key("app --socket /tmp/app.sock"), Some("socket:/tmp/app.sock".into()));
        // 과적 플래그·비수치·포트0은 종단점으로 보지 않는다(클라이언트 오인 차단)
        assert_eq!(endpoint_key("ssh -p 22 host"), None);
        assert_eq!(endpoint_key("mkdir -p /tmp/x"), None);
        assert_eq!(endpoint_key("srv --port 0"), None);
        assert_eq!(endpoint_key("srv --port auto"), None);
        assert_eq!(endpoint_key("plain command with args"), None);
    }

    /// 소유 판정 계약 핀 — 무관한 서드파티 프로세스는 노드 소속이 아니다(과잉 제외 금지).
    #[test]
    fn is_node_owned_does_not_swallow_third_party_processes() {
        let bins = vec!["claude".to_string(), "codex".to_string()];
        assert!(is_node_owned("claude --resume", &bins));
        assert!(is_node_owned("/opt/x/codex exec", &bins));
        assert!(is_node_owned("/bin/zsh -l", &bins), "pane 배관(셸)");
        assert!(is_node_owned("powershell.exe -NoLogo", &bins), "windows 콘솔 배관");
        assert!(is_node_owned("cys send --to master hi", &bins), "cys 자신");
        // 서드파티 서버·도구는 반드시 계수 대상으로 남아야 한다
        assert!(!is_node_owned("bun /work/server.ts", &bins));
        assert!(!is_node_owned("node /work/main.js", &bins));
        assert!(!is_node_owned("python3 -m http.server", &bins));
    }

    /// 종단점 나이 게이트(불변식 ⑤) — 갓 뜬 클라이언트 도구가 잠깐 겹친 것은 "서버 2개"가 아니다.
    /// surface 스코프(같은 pane 안 동일 명령 N개)는 종전대로 나이 게이트 없이 즉시 발화한다.
    #[test]
    fn endpoint_scope_requires_minimum_age_but_surface_scope_does_not() {
        let young = |pid: u32, sid: u64, cmd: &str| ProcObs {
            pid,
            ppid: 1,
            surface_id: sid,
            cmdline: cmd.to_string(),
            node_owned: false,
            age_secs: 3.0, // 방금 뜸
        };
        // 종단점: 둘 다 어리면 발화하지 않는다
        let obs = vec![
            young(11, 1, "psql --port 5432 mydb"),
            young(12, 2, "psql --port 5432 other"),
        ];
        assert!(
            plan_duplicate_alerts(&obs, 3, 2, 45.0).is_empty(),
            "갓 뜬 클라이언트 겹침을 중복 서버로 오판"
        );
        // 나이 게이트를 0으로 낮추면 같은 입력이 발화한다(게이트가 원인임을 확정)
        assert_eq!(plan_duplicate_alerts(&obs, 3, 2, 0.0).len(), 1);
        // surface 스코프는 어려도 발화(종전 동작 보존)
        let piled = vec![
            young(21, 5, "bun server.ts"),
            young(22, 5, "bun server.ts"),
            young(23, 5, "bun server.ts"),
        ];
        assert_eq!(plan_duplicate_alerts(&piled, 3, 2, 45.0).len(), 1);
    }

    /// 임계 0 = 비활성(escape hatch) — 종단점 판정을 끌 수 있어야 한다.
    #[test]
    fn zero_threshold_disables_scope() {
        let obs = vec![
            ProcObs { pid: 1, ppid: 0, surface_id: 1, cmdline: "srv --port 9000".into(), node_owned: false, age_secs: 3600.0 },
            ProcObs { pid: 2, ppid: 0, surface_id: 2, cmdline: "srv --port 9000".into(), node_owned: false, age_secs: 3600.0 },
        ];
        assert_eq!(plan_duplicate_alerts(&obs, 3, 2, 45.0).len(), 1);
        assert!(plan_duplicate_alerts(&obs, 3, 0, 45.0).is_empty(), "endpoint 임계 0 = 비활성");
    }

    // ── ★격리 실행(live) 실증 — 실제 프로세스 트리로 `check_surfaces` 전 경로를 돌린다.
    //    순수함수 단위테스트가 판정을 보증한다면, 아래 둘은 **배선**(관측 수집→소유 판정→발화)이
    //    실제 커널 사실 위에서 같은 결론을 내는지 본다. 라이브 데몬·라이브 원장 무접촉(격리 소켓).

    /// 자손 프로세스가 뜰 때까지 최대 ~5초 기다렸다가 check_surfaces 를 돌리고 발화 경보를 회수.
    #[cfg(unix)]
    fn drill_duplicate_alerts(daemon: &Arc<Daemon>, expect_descendants: usize) -> Vec<serde_json::Value> {
        use sysinfo::{ProcessesToUpdate, System};
        let mut sys = System::new();
        let roots: Vec<u32> = daemon.surfaces.lock().unwrap().values().map(|s| s.pid).collect();
        let mut seen = 0usize;
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            sys.refresh_processes(ProcessesToUpdate::All, true);
            seen = roots.iter().map(|r| collect_descendants(&sys, *r).len()).sum();
            if seen >= expect_descendants {
                break;
            }
        }
        assert!(
            seen >= expect_descendants,
            "전제 실패: 자손 프로세스가 뜨지 않음(관측 {seen} < 기대 {expect_descendants})"
        );
        let seq = daemon.bus.tail(1).first().and_then(|e| e["seq"].as_u64()).unwrap_or(0);
        let mut dup: HashMap<String, f64> = HashMap::new();
        let mut proc_: HashMap<u64, f64> = HashMap::new();
        check_surfaces(daemon, &sys, &mut dup, &mut proc_);
        daemon
            .bus
            .replay_after(seq)
            .into_iter()
            .filter(|e| e["name"].as_str() == Some("watchdog.duplicate_procs"))
            .collect()
    }

    /// ★①(live) 5노드 정상 편성 — 각 pane 이 같은 명령 1개씩 → 신규 중복 경보 **0건**.
    /// 수리 전 이름 전역 계수라면 `sleep …` 5개가 임계 3을 넘겨 발화했을 구성이다.
    #[cfg(unix)]
    #[test]
    fn live_normal_formation_fires_no_duplicate_alert() {
        let daemon = drill_daemon("dup-live-normal");
        for _ in 0..5 {
            // `; :` 로 셸의 exec 최적화를 막아 자식 프로세스가 실제로 갈라지게 한다.
            let s = daemon
                .create_surface(None, Some("sleep 12 ; :".into()), None, None, 24, 80)
                .expect("create surface");
            daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        }
        // 델타 증명(live): **같은 순간의 실제 프로세스 표**를 수리 전 이름 전역 계수에 넣으면
        // 정상 편성인데도 경보가 난다 — 실사고가 코드가 아니라 현실에서 성립했음을 보인다.
        {
            use sysinfo::{ProcessesToUpdate, System};
            let mut sys = System::new();
            let mut pre: Vec<(String, usize)> = Vec::new();
            for _ in 0..50 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                sys.refresh_processes(ProcessesToUpdate::All, true);
                let obs: Vec<ProcObs> = daemon
                    .surfaces
                    .lock()
                    .unwrap()
                    .values()
                    .flat_map(|s| {
                        collect_descendants(&sys, s.pid)
                            .into_iter()
                            .map(move |(pid, cmdline)| ProcObs {
                                pid,
                                ppid: 0,
                                surface_id: s.id,
                                cmdline,
                                node_owned: false,
                                age_secs: 0.0,
                            })
                    })
                    .collect();
                pre = dup_alerts_pre_fix(&obs, 3);
                if !pre.is_empty() {
                    break;
                }
            }
            assert!(
                !pre.is_empty(),
                "전제 실패: live 정상 편성이 수리 전 미러에서 경보를 내지 않음"
            );
        }

        let alerts = drill_duplicate_alerts(&daemon, 5);
        assert!(alerts.is_empty(), "정상 편성 live 드릴에서 중복 경보 발화: {alerts:#?}");
        for s in daemon.surfaces.lock().unwrap().values() {
            kill_pid(s.pid);
        }
    }

    /// ★②(live) 진짜 중복 — 한 pane 이 같은 명령 3개를 쌓았다 → 경보 **1건**(surface 스코프).
    #[cfg(unix)]
    #[test]
    fn live_real_pile_in_one_surface_fires_duplicate_alert() {
        let daemon = drill_daemon("dup-live-pile");
        let s = daemon
            .create_surface(
                None,
                Some("sleep 13 & sleep 13 & sleep 13 & wait".into()),
                None,
                None,
                24,
                80,
            )
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        let alerts = drill_duplicate_alerts(&daemon, 3);
        assert_eq!(alerts.len(), 1, "진짜 중복이 발화하지 않음: {alerts:#?}");
        let p = &alerts[0]["payload"];
        assert_eq!(p["scope"].as_str(), Some("surface"));
        assert_eq!(p["surface_id"].as_u64(), Some(s.id));
        assert!(p["count"].as_u64().unwrap_or(0) >= 3, "count 필드 하위호환: {p}");
        // ★macOS 관측 한계 기록: sysinfo 가 인자를 못 읽으면 cmdline 이 이름("sleep")으로 축약된다
        //   → 이름 계수의 해상도는 플랫폼에 따라 더 나빠진다(소유·스코프 기반 판정이 필요한 또 하나의 근거).
        assert!(p["cmdline"].as_str().unwrap_or("").contains("sleep"), "cmdline 필드 하위호환: {p}");
        kill_pid(s.pid);
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

    /// ★특성화 기준선(2026-07-29 · 확장자 정규화 수술 前 박제): 현행 부트스트랩이 실제로
    /// 의존하는 그린 경로 — 이 판정들은 수술 후에도 **한 건도 바뀌면 안 된다**.
    /// (수술 반경 검증용 — 확장자 정규화가 등록 확장자 계열 밖으로 새면 여기서 깨진다.)
    #[test]
    fn cmdline_matches_agent_bootstrap_characterization() {
        use super::cmdline_matches_agent as m;
        // ── macOS 실사용 형태 (cys launch-agent 가 실제로 띄우는 꼴) ──
        // 개명 래퍼 = 쉘 스크립트(확장자 없음) → basename 직접 일치
        assert!(m(
            "/bin/bash /Users/x/bin/claude-cysinsight --dangerously-skip-permissions",
            "claude-cysinsight"
        ));
        assert!(m("/usr/local/bin/claude --dangerously-skip-permissions", "claude"));
        assert!(m("/opt/homebrew/bin/codex --dangerously-bypass-approvals-and-sandbox", "codex"));
        assert!(m("codex --dangerously-bypass-approvals-and-sandbox", "codex"));
        // npm 래퍼(2026-06-12 결함 픽스처)는 위 covers_npm_wrapper_forms 가 정본 —
        // 여기서는 기준선 대표 1건만 재확인한다.
        assert!(m(
            "node --no-warnings /Users/user/.npm-global/lib/node_modules/@google/gemini-cli/bundle/gemini.js",
            "gemini"
        ));
        // ── Windows 기본형(확장자가 양측에 모두 있는 꼴 — 수술 前에도 green) ──
        assert!(m(
            "C:\\Users\\x\\AppData\\Local\\Programs\\claude\\claude.exe --dangerously-skip-permissions",
            "claude.exe"
        ));
        // ── 오탐 없던 비매칭 경로(죽음 은폐 방지 규칙) ──
        assert!(!m("node /opt/claude-code-router/index.js", "claude"));
        assert!(!m("tail -f logs/claude-archive/x.log", "claude"));
        assert!(!m("my-claude-helper --x", "claude"));
        assert!(!m("/a/grok-1-weights/loader.js", "grok"));
        assert!(!m("python3 train.py gemini-style-arg", "gemini"));
        // 등록 외 확장자는 이름 본체로 취급(strip 금지) — unix 파일명 무영향
        assert!(!m("/usr/local/bin/claude.something", "claude"));
        assert!(!m("", "claude"));
        assert!(!m("node /x/y.js", ""));
    }

    /// ★불변식 박제(2026-07-29 현장 결함 2호 · Windows 실기 확정): 개명 래퍼(`claude-2.cmd`)
    /// 기동 시 트리는 `powershell → cmd.exe(…\claude-2.cmd) → claude.exe`인데 등록 bin_base는
    /// 확장자 없는 `claude-2`다. 확장자를 벗기지 않으면 일치 토큰이 영원히 없어 agent_alive
    /// 영구 false → boot_node reclaim()의 `taskkill /T`가 멀쩡한 pane을 오살한다.
    #[test]
    fn cmdline_matches_agent_normalizes_windows_exec_extensions() {
        use super::cmdline_matches_agent as m;
        // ① 결함 재현 픽스처 — sysinfo(Windows)는 CommandLineToArgvW로 argv를 얻으므로
        //    따옴표는 이미 제거된 상태로 join된다(따옴표 트리밍은 불필요·미도입).
        assert!(m(
            "cmd.exe /c C:\\Users\\x\\.local\\bin\\claude-2.cmd --dangerously-skip-permissions",
            "claude-2"
        ));
        // ② 확장자 대소문자 무시(Windows 파일시스템 규약)
        assert!(m("cmd.exe /c C:\\bin\\claude-2.CMD --dangerously-skip-permissions", "claude-2"));
        assert!(m("cmd.exe /c C:\\bin\\claude-2.Cmd --dangerously-skip-permissions", "claude-2"));
        // ★본체 대소문자 규칙 계약(codex R1 major② 확정): 등록 확장자가 **실제로 벗겨진**
        // Windows형 토큰에 한해 본체도 대소문자 무시로 비교한다 — 대소문자 무구분은 Windows
        // 파일시스템의 성질이기 때문이다. Windows 실기에서 래퍼 basename이 통째 대문자로
        // 관측되는 형태(`CLAUDE-2.CMD`)가 실재하므로 이를 생존으로 인정해야 오살을 막는다.
        assert!(m("cmd.exe /c C:\\bin\\CLAUDE-2.CMD --x", "claude-2"));
        assert!(m("cmd.exe /c C:\\bin\\Claude-2.cmd --x", "claude-2"));
        assert!(m("cmd.exe /c C:\\bin\\claude-2.cmd --x", "CLAUDE-2.CMD")); // bin_base 측 대칭
        // ★그러나 확장자 없는 bare 토큰은 **정확 비교 유지** — 유닉스에서 `CLAUDE-2`와
        // `claude-2`는 서로 다른 파일이고, 뭉개면 '생존 오판'의 재료가 된다(반경 봉인).
        assert!(!m("/usr/local/bin/CLAUDE-2 --x", "claude-2"));
        assert!(!m("/usr/local/bin/Claude --x", "claude"));
        // ③ 기본형 무회귀: 확장자가 양측에 다 있어도, 한쪽에만 있어도 성립
        assert!(m("C:\\Program Files\\claude\\claude.exe --x", "claude.exe"));
        assert!(m("C:\\Program Files\\claude\\claude.exe --x", "claude"));
        assert!(m("C:\\bin\\claude-2.exe --x", "claude-2.cmd"));
        // ④ 오탐 차단 — 유사 이름은 확장자를 벗겨도 본체가 다르다(오살 위험 방향 사수)
        assert!(!m("C:\\bin\\claude-2.cmdx --x", "claude-2"));
        assert!(!m("C:\\bin\\xclaude-2.cmd --x", "claude-2"));
        assert!(!m("C:\\bin\\claude-27.cmd --x", "claude-2"));
        // ⑤ 나머지 등록 확장자 각 1건
        assert!(m("powershell -File C:\\bin\\claude-2.ps1", "claude-2"));
        assert!(m("cmd.exe /c C:\\bin\\agy.bat --yolo", "agy"));
        assert!(m("C:\\bin\\codex.com", "codex"));
        // ⑥ unix 무영향 — 등록 외 확장자는 이름 본체이므로 strip 금지
        assert!(m("/usr/local/bin/claude --dangerously-skip-permissions", "claude"));
        assert!(!m("/usr/local/bin/claude.something", "claude"));
        assert!(!m("/usr/local/bin/claude.backup", "claude"));
        assert!(m("node /n/m/@google/gemini-cli/bundle/gemini.js", "gemini")); // .js 특례 불변
        // ⑦ 도트파일·정규화 후 빈 이름은 매칭 대상이 아니다(공백 매처 방어)
        assert!(!m("/usr/local/bin/.cmd", "claude"));
        assert!(!m("C:\\bin\\claude.exe --x", ".exe"));
        // ⑧ 경로 세그먼트는 **무정규화 원형** — 패키지 규칙(`<bin>-cli`·`<bin>-code`) 의미 불변
        assert!(m("node C:\\n\\m\\@anthropic-ai\\claude-code\\cli.js", "claude"));
        assert!(!m("node C:\\opt\\claude-code-router\\index.js", "claude"));
        // ★세그먼트 strip 철회 박제(codex R1 major①): 디렉터리명은 실행 파일명이 아니다.
        // 세그먼트까지 확장자를 벗기면 무관 디렉터리가 생존 증거로 승격돼 죽음을 은폐한다.
        assert!(!m("node C:\\work\\claude.cmd\\helper.js", "claude"));
        assert!(!m("node C:\\work\\claude.exe\\helper.js", "claude"));
        assert!(!m("node /work/gemini.cmd/helper.js", "gemini"));
    }

    /// ★관측 기반 agent 등록(2026-08 현장 결함 2호) — 후보 파생의 계약 박제.
    /// agents.json 의 cmd 에서 선두 env 대입을 건너뛴 첫 토큰 basename 이 바이너리다.
    /// 임베드 팩 실물(agents.json)로도 교차 검증한다 — 스키마 드리프트 시 여기서 먼저 죽는다.
    #[test]
    fn agent_candidates_from_json_derives_bin_basenames() {
        use super::agent_candidates_from_json as cands;
        let v: serde_json::Value = serde_json::json!({
            "_schema": 2,
            "_doc": "메타 — 후보가 아니다",
            "claude": {"cmd": "claude --dangerously-skip-permissions"},
            "gemini": {"cmd": "~/.local/bin/agy --dangerously-skip-permissions"},
            "withenv": {"cmd": "CLAUDE_CONFIG_DIR=\"$HOME/.cys/claude\" claude --x"},
            "nocmd": {"notes": "cmd 없음 — 후보 제외"},
        });
        let got = cands(&v);
        assert!(got.contains(&("claude".into(), "claude".into())));
        assert!(got.contains(&("gemini".into(), "agy".into()))); // 절대경로 → basename
        assert!(got.contains(&("withenv".into(), "claude".into()))); // env 대입 건너뜀
        assert!(!got.iter().any(|(n, _)| n == "_schema" || n == "_doc" || n == "nocmd"));
        // 임베드 팩 실물 교차 검증: claude·gemini(agy)·codex 가 파생돼야 한다.
        let embedded: serde_json::Value = cys::pack::PACK_ALL
            .iter()
            .find(|(r, _)| *r == "agents.json")
            .map(|(_, c)| serde_json::from_str(c).expect("임베드 agents.json 파싱"))
            .expect("임베드에 agents.json 존재");
        let real = cands(&embedded);
        assert!(real.contains(&("claude".into(), "claude".into())));
        assert!(real.contains(&("gemini".into(), "agy".into())));
        assert!(real.contains(&("codex".into(), "codex".into())));
    }

    /// 선택 규칙 박제: 정확히 한 에이전트 매칭일 때만 Some — 모호(2종 동시)·무매칭은 None.
    /// 오기록(엉뚱한 CLI 부활·사망감지 오배선)보다 무기록(현행 유지)이 낫다는 fail-closed 계약.
    #[test]
    fn select_observed_agent_requires_exactly_one_match() {
        use super::select_observed_agent as sel;
        let cands: Vec<(String, String)> = vec![
            ("claude".into(), "claude".into()),
            ("gemini".into(), "agy".into()),
            ("codex".into(), "codex".into()),
        ];
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // 단일 관측 → Some (셸·무관 프로세스 혼재 무해)
        assert_eq!(
            sel(&s(&["-zsh", "claude --dangerously-skip-permissions"]), &cands),
            Some(("claude".into(), "claude".into()))
        );
        // npm 래퍼 형태도 SOT 매처를 그대로 탄다
        assert_eq!(
            sel(&s(&["node /n/m/@anthropic-ai/claude-code/cli.js"]), &cands),
            Some(("claude".into(), "claude".into()))
        );
        // 같은 에이전트 다중 프로세스(부모+헬퍼)는 모호가 아니다
        assert_eq!(
            sel(&s(&["claude --x", "node /n/m/@anthropic-ai/claude-code/cli.js"]), &cands),
            Some(("claude".into(), "claude".into()))
        );
        // 서로 다른 에이전트 동시 관측 = 모호 → None
        assert_eq!(sel(&s(&["claude --x", "codex --y"]), &cands), None);
        // 무관측 → None
        assert_eq!(sel(&s(&["-zsh", "vim notes.md"]), &cands), None);
        // 빈 후보(agents.json 파싱 불가) → None (관측 등록이 조용히 꺼진다)
        assert_eq!(sel(&s(&["claude --x"]), &[]), None);
    }

    /// ★[생산자측 핀] seat_agent_cache 를 **만드는 쪽**의 경계 — W3-A BLOCK 교정의 핵심.
    ///
    /// 배경(mutation 실증): 데드맨 테스트는 전부 `seat_agent_cache` 를 직접 store 해
    /// **소비측**(트래커가 armed 값을 어떻게 쓰는가)만 고정한다. 그래서 생산자
    /// (refresh_seat_cache)의 엄격 매칭을 `true` 로 치환해 **원시 Occupied 로 armed** 하는
    /// mutation(= BLOCK 교정 이전 거동)이 전체 테스트를 통과했다. 그 거동은 vim/less/빌드 등
    /// 비에이전트 자손 1틱 관측 → 프롬프트 복귀(Empty) → **살아있는 맨 셸 master 를 사망으로
    /// 오라벨**한다(결함 8 동형 오살 경보). 이 테스트가 그 mutation 을 적색으로 만든다.
    #[test]
    fn seat_agent_observed_pins_strict_matcher_not_raw_occupied() {
        use super::seat_agent_observed as obs;
        let cands: Vec<(String, String)> = vec![
            ("claude".into(), "claude".into()),
            ("gemini".into(), "agy".into()),
        ];
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // ① 비에이전트 자손만 있는 좌석 = 관측 아님. ★`true` 치환 mutation 이 여기서 죽는다.
        for non_agent in [
            vec!["vim notes.md"],
            vec!["less /var/log/x"],
            vec!["tail -f build.log"],
            vec!["cargo build --release"],
            vec!["-zsh", "git status"],
        ] {
            assert!(
                !obs(&s(&non_agent), &cands),
                "비에이전트 자손({non_agent:?})을 관측으로 셈하면 맨 셸 좌석이 사망 오라벨된다"
            );
        }
        // ② 자손 0개(프롬프트 복귀)도 관측 아님 — 원시 Occupied 판정과의 분리 확인.
        assert!(!obs(&[], &cands));
        // ③ 기지 에이전트 자손 = 관측. (엄격 매처를 항상-false 로 만드는 역방향 mutation 차단)
        assert!(obs(&s(&["claude --dangerously-skip-permissions"]), &cands));
        assert!(obs(&s(&["-zsh", "agy --x"]), &cands));
        // ④ 후보 목록 부재(agents.json 파싱 불가) = 보조축 off(fail-closed).
        assert!(!obs(&s(&["claude --x"]), &[]));
        // ⑤ 배선 핀: refresh_seat_cache 가 이 판정을 실제로 소비한다(생산자 경계 우회 금지).
        //    순수 함수만 있고 호출부가 상수면 ①~④가 전부 무의미해지므로 호출 자체를 못박는다.
        let src = include_str!("governance.rs");
        let prod = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let at = prod
            .find("pub fn refresh_seat_cache")
            .expect("refresh_seat_cache 소실");
        let body = &prod[at..];
        let end = body.find("\n}\n").unwrap_or(body.len());
        assert!(
            body[..end].contains("seat_agent_observed(&cmds, cands)"),
            "refresh_seat_cache 가 엄격 매처 판정을 우회했다 — 원시 Occupied armed 회귀 위험"
        );
    }

    /// ★등록 매처 FP 회귀 핀(2026-08-12 R2 확정 · governance.rs 경로 세그먼트 FP):
    /// 생존 매처의 경로 세그먼트 규칙이 등록에서는 오등록(콜드부트가 엉뚱한 CLI 부활)을 낳는다 —
    /// 등록은 strict 매처(cmdline_matches_agent_exec)를 쓰고, 데이터 파일 인자의 디렉터리명은
    /// 실행 증거로 승격되지 않는다. strict ⊆ broad(생존) 포함 관계도 함께 핀.
    #[test]
    fn registration_matcher_rejects_path_segment_data_args() {
        use super::select_observed_agent as sel;
        use super::{cmdline_matches_agent, cmdline_matches_agent_exec};
        let cands: Vec<(String, String)> = vec![("claude".into(), "claude".into())];
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // 데이터 파일 인자의 디렉터리 세그먼트 — 생존 매처는 잡지만(넓음·의도) 등록은 거부한다.
        for fp in [
            "tail -f /home/u/claude/dev.log",
            "vim /home/u/proj/claude-code/README.md",
            "less /var/tmp/claude-code/notes.txt",
        ] {
            assert!(cmdline_matches_agent(fp, "claude"), "생존 매처는 넓다(전제): {fp}");
            assert!(!cmdline_matches_agent_exec(fp, "claude"), "등록 매처는 FP 거부: {fp}");
            assert_eq!(sel(&s(&[fp]), &cands), None, "등록 오등록 금지: {fp}");
        }
        // 실행 증거는 계속 잡는다: 네이티브 바이너리·npm 래퍼(.js 실행 스크립트).
        for tp in [
            "claude --dangerously-skip-permissions",
            "node /n/m/@anthropic-ai/claude-code/cli.js",
            "node --max-old-space-size=4096 /n/m/@google/gemini-cli/bundle/gemini.js",
        ] {
            let want = if tp.contains("gemini") { "gemini" } else { "claude" };
            assert!(cmdline_matches_agent_exec(tp, want), "등록 매처 실행 증거 유지: {tp}");
            assert!(cmdline_matches_agent(tp, want), "strict ⊆ broad 포함 관계: {tp}");
        }
        // .js 로 끝나지 않는 경로의 세그먼트 매칭은 등록에서 무효(broad 전용).
        assert!(!cmdline_matches_agent_exec("node /home/u/claude-code/helper.py", "claude"));
    }

    /// ★G5-③(W5-A) strict 매처 Windows 픽스처 — 기존 픽스처군의 백슬래시 계열 증설.
    /// Windows 개방(2-표본 확정)의 1·2표본 모두 이 매처를 타므로, 실전 cmdline 형태
    /// (npm 래퍼 백슬래시 경로 · cmd 셔틀 · 데이터 파일 인자)의 판정을 전 OS 에서 봉인한다.
    #[test]
    fn registration_matcher_windows_fixtures() {
        use super::select_observed_agent as sel;
        use super::cmdline_matches_agent_exec;
        let cands: Vec<(String, String)> = vec![
            ("claude".into(), "claude".into()),
            ("codex".into(), "codex".into()),
        ];
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // npm 래퍼(백슬래시 경로·패키지 세그먼트 claude-code) — 매치.
        let npm = r"node C:\Users\x\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\cli.js";
        assert!(cmdline_matches_agent_exec(npm, "claude"));
        assert_eq!(sel(&s(&[npm]), &cands), Some(("claude".into(), "claude".into())));
        // cmd 셔틀(.cmd 확장자 정규화·대소문자 무구분) — 매치.
        let shim = r"cmd /C claude.cmd";
        assert!(cmdline_matches_agent_exec(shim, "claude"));
        assert_eq!(sel(&s(&[shim]), &cands), Some(("claude".into(), "claude".into())));
        // 데이터 파일 인자(claude.log — 비실행 확장자) — 무매치(등록 오염 금지).
        let tailer = r"powershell Get-Content claude.log -Wait";
        assert!(!cmdline_matches_agent_exec(tailer, "claude"));
        assert_eq!(sel(&s(&[tailer]), &cands), None);
        // 이종 2 에이전트 동시(백슬래시 계열) = 모호 — None(2-표본이어도 등록 불가 재료).
        assert_eq!(sel(&s(&[npm, r"C:\Tools\codex.exe --y"]), &cands), None);
    }

    /// ★G5-③(W5-A) 2-표본 확정 진리표 — 순수 판정자 confirm_pending_obs 회귀 핀
    /// (설계 원문 G5-③ 명세 그대로: Commit/Drop/Keep + TTL 우선).
    #[test]
    fn pending_verdict_truth_table() {
        use super::{confirm_pending_obs as judge, PendingVerdict as V};
        let pending = ("claude".to_string(), "claude".to_string(), 1000.0);
        let cur = |a: &str| (a.to_string(), a.to_string());
        // ① 동일 단일 에이전트 재관측 = Commit.
        assert_eq!(judge(&pending, Some(&cur("claude")), 1005.0, 120.0), V::Commit);
        // ② 상이 에이전트 = Drop(agent_mismatch) — 1표본 순간 혼선의 반증.
        assert_eq!(
            judge(&pending, Some(&cur("codex")), 1005.0, 120.0),
            V::Drop { reason: "agent_mismatch" }
        );
        // ③ 무관측(None) = Keep — 다음 틱 유보.
        assert_eq!(judge(&pending, None, 1005.0, 120.0), V::Keep);
        // ④ TTL 초과 = Drop(ttl_expired) — 동일 에이전트 재관측이어도 오래된 1표본은 불신.
        assert_eq!(
            judge(&pending, Some(&cur("claude")), 1000.0 + 120.1, 120.0),
            V::Drop { reason: "ttl_expired" }
        );
        // ⑤ 모호(select_observed_agent None 귀결) = Keep → TTL 도달 시 Drop.
        assert_eq!(judge(&pending, None, 1000.0 + 119.9, 120.0), V::Keep);
        assert_eq!(
            judge(&pending, None, 1000.0 + 120.1, 120.0),
            V::Drop { reason: "ttl_expired" }
        );
        // 경계: TTL 정확히 도달(now-obs == ttl)은 아직 유효(초과 조건은 strict >).
        assert_eq!(judge(&pending, Some(&cur("claude")), 1120.0, 120.0), V::Commit);
    }

    /// ★G5-③(W5-A) Drop 이벤트 배선 핀 — TTL 초과 pending 은 check_agent_death 훅이 소거하고
    /// agent.observe_dropped(reason=ttl_expired) 를 발행한다(확정 실패 침묵 방지 [MAJOR]).
    /// meta 는 확정 없이 기록되지 않는다(fail-closed — 무기록이 오기록보다 낫다).
    #[test]
    fn pending_obs_ttl_drop_emits_observe_dropped() {
        let daemon = drill_daemon("pending-ttl-drop");
        let id = spawn_role_surface(&daemon, "worker");
        let s = daemon.surfaces.lock().unwrap().get(&id).cloned().unwrap();
        *s.pending_agent_obs.lock().unwrap() = Some((
            "claude".into(),
            "claude".into(),
            now_epoch() - super::PENDING_OBS_TTL_SECS - 1.0,
        ));
        let sys = sysinfo::System::new(); // 빈 프로세스 표 = 재관측 None(모호/무관측 대역)
        let mut rc: HashMap<u64, u32> = HashMap::new();
        // (P3-6) 엄격 관측 증명 래치 — 이 드릴들은 pending 확정·소거 경로만 보므로 빈 채로 둔다.
        let mut sp: HashMap<u64, Option<(String, u32)>> = HashMap::new();
        super::check_agent_death(&daemon, &sys, &mut rc, &mut sp);
        assert!(
            s.pending_agent_obs.lock().unwrap().is_none(),
            "TTL 초과 pending 은 소거돼야 한다"
        );
        assert!(
            s.agent_meta.lock().unwrap().is_none(),
            "확정 없는 meta 기록 금지(fail-closed)"
        );
        let dropped: Vec<serde_json::Value> = daemon
            .bus
            .replay_after(0)
            .into_iter()
            .filter(|e| e["name"].as_str() == Some("agent.observe_dropped"))
            .collect();
        assert_eq!(dropped.len(), 1, "Drop 은 침묵하지 않는다 — 이벤트 정확히 1건");
        assert_eq!(dropped[0]["payload"]["reason"].as_str(), Some("ttl_expired"));
        assert_eq!(dropped[0]["payload"]["agent"].as_str(), Some("claude"));
        assert_eq!(dropped[0]["payload"]["role"].as_str(), Some("worker"));
    }

    /// ★G5-③(W5-A) Keep·가드 소거 의미 핀 — ① 신선 pending + 무관측 = Keep(소거·이벤트 0)
    /// ② set_meta 선점(가드 실패) = 조용한 소거(확정 실패가 아니라 승계 — Drop 이벤트 아님).
    #[test]
    fn pending_obs_keep_then_superseded_clears_silently() {
        let daemon = drill_daemon("pending-keep-guard");
        let id = spawn_role_surface(&daemon, "worker");
        let s = daemon.surfaces.lock().unwrap().get(&id).cloned().unwrap();
        let sys = sysinfo::System::new();
        let mut rc: HashMap<u64, u32> = HashMap::new();
        // (P3-6) 엄격 관측 증명 래치 — 이 드릴들은 pending 확정·소거 경로만 보므로 빈 채로 둔다.
        let mut sp: HashMap<u64, Option<(String, u32)>> = HashMap::new();
        // ① 무관측 = Keep — pending 유지·이벤트 없음(다음 틱 유보).
        *s.pending_agent_obs.lock().unwrap() =
            Some(("claude".into(), "claude".into(), now_epoch()));
        super::check_agent_death(&daemon, &sys, &mut rc, &mut sp);
        assert!(
            s.pending_agent_obs.lock().unwrap().is_some(),
            "무관측은 Keep — TTL 내 재시도 유보"
        );
        // ② 다른 경로(set_meta)가 meta 를 선점 — pending 은 무의미해져 조용히 소거된다.
        *s.agent_meta.lock().unwrap() = Some(("codex".into(), "codex".into()));
        super::check_agent_death(&daemon, &sys, &mut rc, &mut sp);
        assert!(
            s.pending_agent_obs.lock().unwrap().is_none(),
            "meta 선점 시 pending 소거(스테이징 잔존 금지)"
        );
        assert_eq!(
            s.agent_meta.lock().unwrap().clone(),
            Some(("codex".into(), "codex".into())),
            "선점 meta 는 pending 이 덮지 않는다"
        );
        assert!(
            daemon
                .bus
                .replay_after(0)
                .iter()
                .all(|e| e["name"].as_str() != Some("agent.observe_dropped")),
            "Keep·승계 소거는 Drop 이벤트를 발행하지 않는다"
        );
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

    /// ★G1(W2-A): 테스트 큐 원소 — (id, text) 쌍으로 QueueEntry를 합성한다.
    /// seq·enqueued_at은 pop-by-id 판정과 무관하므로 고정값(판정 재료는 id 하나).
    fn qe(id: &str, text: &str) -> crate::state::QueueEntry {
        crate::state::QueueEntry {
            id: id.to_string(),
            seq: 0,
            text: text.to_string(),
            enqueued_at: 0.0,
            from: None,
            origin: "test".to_string(),
        }
    }

    /// id를 텍스트에서 파생("id-<text>")한 QueueEntry 큐 — 기존 텍스트 기반 테스트의 최소 이식.
    fn q(items: &[&str]) -> VecDeque<crate::state::QueueEntry> {
        items.iter().map(|s| qe(&format!("id-{s}"), s)).collect()
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
        // 정상 경로: 보낸 항목이 여전히 머리 → 제거(판정 = id). 뒤 항목은 보존.
        let mut deque = q(&["msg1", "msg2"]);
        pop_delivered_head(&mut deque, "id-msg1");
        assert_eq!(deque, q(&["msg2"]));
    }

    #[test]
    fn pop_delivered_head_noop_on_empty_after_clear() {
        // lost-clear 시나리오: front 읽은 뒤 락이 풀린 창에서 queue.clear가 drain →
        // 빈 큐. 핵심은 '빈 큐를 건드리지 않고' 손상 없이 빠져나오는 것.
        // (이미 PTY로 간 메시지는 회수 불가 — 아키텍처 한계)
        let mut deque = q(&[]);
        pop_delivered_head(&mut deque, "id-msg1");
        assert!(deque.is_empty());
    }

    #[test]
    fn pop_delivered_head_preserves_new_message_after_clear_and_enqueue() {
        // 유해 변종(이 수정의 핵심 회귀 가드): front("msgA") 읽고 락 해제 →
        // 그 창에서 clear가 drain([]) 후 새 메시지 "msgB" enqueue → 큐=["msgB"].
        // 무조건 pop_front이면 미배달 "msgB"를 삼켜 조용히 유실시킨다.
        // 머리가 보낸 "msgA"(id)가 아니므로 제거하지 않아야 한다 — "msgB"는 다음 틱에 배달.
        let mut deque = q(&["msgB"]);
        pop_delivered_head(&mut deque, "id-msgA");
        assert_eq!(deque, q(&["msgB"]), "미배달 새 메시지가 유실되면 안 된다");
    }

    #[test]
    fn pop_delivered_head_preserves_replacement_head() {
        // clear→enqueue가 여러 건이어도 머리 불일치면 한 건도 삼키지 않는다.
        let mut deque = q(&["msgB", "msgC"]);
        pop_delivered_head(&mut deque, "id-msgA");
        assert_eq!(deque, q(&["msgB", "msgC"]));
    }

    /// ★G1(W2-A) 신규 핀 — 텍스트 비교 시절 모호성의 봉인: **동일 텍스트**(빈 문자열 Return
    /// 2건 — send-key --queued의 실경로)라도 id가 다르면 절대 pop하지 않는다.
    /// 텍스트 비교였다면 배달된 1번 항목의 ack가 미배달 2번 항목을 오삼킴할 수 있었다.
    #[test]
    fn pop_delivered_head_same_text_different_id_never_pops() {
        // 시나리오: front(id=ret-1) 읽고 락 해제 → 그 창에서 clear+재enqueue로 머리가
        // 같은 텍스트("")의 다른 항목(id=ret-2)으로 교체 → ret-1 ack가 ret-2를 삼키면 안 된다.
        let mut deque: VecDeque<crate::state::QueueEntry> =
            [qe("ret-2", ""), qe("ret-3", "")].into_iter().collect();
        pop_delivered_head(&mut deque, "ret-1");
        assert_eq!(deque.len(), 2, "동일 텍스트라도 id 불일치면 pop 금지(오삼킴 차단)");
        // 대조군: 머리 id 일치 시에만 정확히 그 항목 하나를 제거.
        pop_delivered_head(&mut deque, "ret-2");
        assert_eq!(deque.len(), 1);
        assert_eq!(deque.front().map(|e| e.id.as_str()), Some("ret-3"));
    }

    // ── TOCTOU 회귀 가드: read-handoff-pop 단일 임계영역 ──
    // deliver_queued의 핵심 불변식을 production과 동일한 락 규율로 재현한다:
    // front 읽기·writer 인계·pop을 pending_queue 락 한 임계영역으로 묶으면,
    // 같은 락으로 drain하는 queue.clear/close_surface는 '읽고서 인계하는' 사이에
    // 끼어들 수 없다. 따라서 '주입된 메시지는 반드시 큐에서도 제거된 것'이고,
    // clear가 비운 메시지는 결코 writer로 가지 않는다.
    use std::sync::mpsc::sync_channel;
    use std::sync::{Arc, Mutex};

    // production deliver_queued의 임계영역과 동일한 순서(★G1 W2-A: QueueEntry·pop-by-id 이식):
    // 락 획득 → front().cloned() → try_send(writer) → pop_delivered_head(id) → 락 해제.
    fn deliver_one_atomic(
        queue: &Mutex<VecDeque<crate::state::QueueEntry>>,
        writer: &std::sync::mpsc::SyncSender<String>,
    ) -> Option<String> {
        let mut q = queue.lock().unwrap();
        let entry = q.front().cloned()?;
        // 논블로킹 인계. 실패 시 메시지 보존(pop 안 함).
        if writer.try_send(entry.text.clone()).is_err() {
            return None;
        }
        pop_delivered_head(&mut q, &entry.id);
        Some(entry.text)
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
                let _: Vec<crate::state::QueueEntry> = qc.lock().unwrap().drain(..).collect();
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

    // ─────────── ★G1(W2-D): 단계형 quiet(기아 봉인) — 순수 판정자·uptime 클램프 핀 ───────────

    use super::{
        deliver_head_locked, deliver_queued, queue_head_wait_secs, queue_quiet_verdict,
        QuietVerdict,
    };

    /// [회귀 핀·기아 ①] wait < max_wait → 현행 quiet 3s 규칙 그대로(현행 의미론 핀).
    #[test]
    fn queue_quiet_verdict_pins_current_rule_below_max_wait() {
        assert_eq!(queue_quiet_verdict(30, 2, 3, 120, 1), QuietVerdict::WaitBusy);
        assert_eq!(
            queue_quiet_verdict(30, 3, 3, 120, 1),
            QuietVerdict::Deliver { overdue: false }
        );
    }

    /// [회귀 핀·기아 ②] wait ≥ max_wait && quiet_for ≥ 1 → 제한 배달(overdue:true).
    /// 정상 quiet(3s)를 채운 배달은 단계와 무관하게 정상 배달(overdue:false)로 분류된다.
    #[test]
    fn queue_quiet_verdict_overdue_lowers_quiet_to_one_second() {
        assert_eq!(
            queue_quiet_verdict(120, 1, 3, 120, 1),
            QuietVerdict::Deliver { overdue: true }
        );
        assert_eq!(
            queue_quiet_verdict(500, 2, 3, 120, 1),
            QuietVerdict::Deliver { overdue: true }
        );
        assert_eq!(
            queue_quiet_verdict(500, 3, 3, 120, 1),
            QuietVerdict::Deliver { overdue: false },
            "정상 quiet 충족은 overdue 표기 없이 나간다"
        );
    }

    /// [회귀 핀·기아 ③ — 핵심 안전 핀] '출력 중 주입 금지'는 overdue 에도 불변:
    /// quiet_for=0 은 어떤 단계에서도 배달 금지(0초 강제주입 아님). overdue_quiet=0
    /// 오설정도 하한 1s 로 승격된다(구조 봉인).
    #[test]
    fn queue_quiet_verdict_never_injects_while_output_streaming() {
        assert_eq!(queue_quiet_verdict(10_000, 0, 3, 120, 1), QuietVerdict::WaitBusy);
        assert_eq!(
            queue_quiet_verdict(10_000, 0, 3, 120, 0),
            QuietVerdict::WaitBusy,
            "overdue_quiet=0 오설정도 0초 주입 경로를 열지 못한다(하한 1s 승격)"
        );
        assert_eq!(
            queue_quiet_verdict(10_000, 1, 3, 120, 0),
            QuietVerdict::Deliver { overdue: true }
        );
    }

    /// [회귀 핀·기아 ④] max_wait=0(기본값·단계형 비활성) → 항상 현행 3s 규칙
    /// (비활성 = 구동작 복원 핀 — 기본값 상태 전 기존 동작 바이트 동일의 근거).
    #[test]
    fn queue_quiet_verdict_disabled_restores_current_behavior() {
        assert_eq!(queue_quiet_verdict(u64::MAX, 2, 3, 0, 1), QuietVerdict::WaitBusy);
        assert_eq!(
            queue_quiet_verdict(u64::MAX, 3, 3, 0, 1),
            QuietVerdict::Deliver { overdue: false }
        );
    }

    /// [★BLOCKER 핀] overdue·기아 자격의 대기 측정 = daemon/surface uptime 클램프 —
    /// WAL 생존 항목(enqueued_at 과거)도 부트 시각 이전 대기는 세지 않는다. 재기동 직후
    /// last_human_input(휘발)이 비어 typing 가드가 무방비인 창에 stale 백로그가 즉시
    /// overdue 최전선 배달되는 병리(성찰 R4 — GUI Update→rotate 플로우 실발화)의 봉인.
    #[test]
    fn queue_head_wait_clamps_to_boot_and_surface_uptime() {
        // 정상: 부트·생성 후 enqueue → 대기 = now - enqueued_at.
        assert_eq!(queue_head_wait_secs(1000.0, 990.0, 900.0, 910.0), 10);
        // 부트 클램프: enqueued_at 이 부트보다 과거면 부트 시각부터 센다.
        assert_eq!(queue_head_wait_secs(1000.0, 100.0, 995.0, 910.0), 5);
        // surface 클램프: surface 가 데몬보다 늦게 생겼으면 생성 시각부터(이관·rehome 수신분).
        assert_eq!(queue_head_wait_secs(1000.0, 100.0, 900.0, 998.0), 2);
        // 역행(시계 스큐·미래 enqueued_at)은 0 클램프 — 측정 불능은 overdue 부적격(fail-closed).
        assert_eq!(queue_head_wait_secs(1000.0, 2000.0, 900.0, 910.0), 0);
    }

    /// 큐 게이트 통합 테스트는 CYS_QUEUE_* env 를 만지므로 직렬화(REAP_ENV_LOCK 관례 동형).
    static QUEUE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// CYS_QUEUE_* env 를 테스트 종료 시(패닉 포함) 이전 값으로 원복하는 가드 —
    /// 없던 값은 remove, 있던 값은 원복(ReapEnvGuard 관례 동형 · 프로세스 전역 env 누수 차단).
    struct QueueEnvGuard {
        prev: Vec<(&'static str, Option<String>)>,
    }
    impl QueueEnvGuard {
        fn set(vars: &[(&'static str, &str)]) -> Self {
            let prev = vars
                .iter()
                .map(|(k, v)| {
                    let old = std::env::var(k).ok();
                    std::env::set_var(k, v);
                    (*k, old)
                })
                .collect();
            QueueEnvGuard { prev }
        }
    }
    impl Drop for QueueEnvGuard {
        fn drop(&mut self) {
            for (k, old) in &self.prev {
                match old {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    /// [무회귀 핀·절대 불변] 기본 노브(max_wait=0 잠금)에서 배달 동작은 현행과 완전 동일:
    /// enqueued_at 이 아무리 과거라도 quiet 3s 미달이면 보류, 충족이면 배달.
    #[test]
    fn deliver_queued_default_knobs_keep_current_quiet_rule() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _env = QueueEnvGuard::set(&[
            ("CYS_QUEUE_MAX_WAIT_SECS", "0"),
            ("CYS_QUEUE_STARVE_ALERT_SECS", "0"),
        ]);
        let daemon = drill_daemon("w2d-default");
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, None, 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        let mut e = daemon.next_queue_entry("w2d 기본값 무회귀 핀".into(), None, "test");
        e.enqueued_at = 1.0; // 아무리 과거라도 비활성(0)에서는 단계 승격 없음
        s.pending_queue.lock().unwrap().push_back(e);
        // 초기 셸 출력(로그인 프로파일)이 last_output 을 덮지 않게 안정화 후 스탬프.
        std::thread::sleep(std::time::Duration::from_millis(600));
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(2);
        *s.last_human_input.lock().unwrap() = None;
        let mut depth = HashMap::new();
        let mut starve = HashMap::new();
        deliver_queued(&daemon, &mut depth, &mut starve);
        assert_eq!(
            s.pending_queue.lock().unwrap().len(),
            1,
            "비활성(0) = 현행 3s 규칙 그대로 — quiet 2s 는 보류(구동작 복원 핀)"
        );
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(4);
        deliver_queued(&daemon, &mut depth, &mut starve);
        assert!(
            s.pending_queue.lock().unwrap().is_empty(),
            "quiet 3s+ 는 현행대로 배달"
        );
        let delivered: Vec<_> = daemon
            .bus
            .tail(30)
            .into_iter()
            .filter(|ev| ev["name"] == "queue.delivered")
            .collect();
        assert_eq!(delivered.len(), 1);
        assert_eq!(
            delivered[0]["payload"]["overdue"],
            serde_json::json!(false),
            "기본값 배달은 overdue 표기 없음(무회귀)"
        );
    }

    /// [회귀 핀·R1 MED-2] human_typing 가드는 단계형 완화(overdue)의 면제 대상이 절대
    /// 아니다 — verdict 가 제한 배달을 허용해도 사람 흔적이 신선하면 배달 0건. 흔적 소거
    /// 후에야 overdue 제한 배달(quiet 2s < 3s 인데도)이 나가고 영수증에 overdue:true 가
    /// 남는다(2단 롤아웃 실측 근거). deliver_head_locked 경유 실행 경로 통합 검증.
    #[test]
    fn deliver_queued_human_guard_blocks_even_overdue_then_limited_delivery_flows() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _env = QueueEnvGuard::set(&[
            ("CYS_QUEUE_MAX_WAIT_SECS", "1"),
            ("CYS_QUEUE_OVERDUE_QUIET_SECS", "1"),
            ("CYS_QUEUE_STARVE_ALERT_SECS", "0"),
        ]);
        let daemon = drill_daemon("w2d-overdue");
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, None, 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        let e = daemon.next_queue_entry("w2d overdue 핀".into(), None, "test");
        s.pending_queue.lock().unwrap().push_back(e);
        // uptime 클램프 기준(부트·생성 시각)으로 max_wait(1s)를 **실제로** 넘긴다
        // (+ 초기 셸 출력 안정화 겸용 — enqueued_at 만 과거로 꾸며서는 클램프 때문에 안 넘는다).
        std::thread::sleep(std::time::Duration::from_millis(1300));
        // quiet 2s: 현행 3s 규칙으로는 보류지만 overdue(1s)로는 배달 자격.
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(2);
        // ★사람 흔적 신선 → overdue 대기라도 배달 0건(면제 절대 없음).
        *s.last_human_input.lock().unwrap() = Some(std::time::Instant::now());
        let mut depth = HashMap::new();
        let mut starve = HashMap::new();
        deliver_queued(&daemon, &mut depth, &mut starve);
        assert_eq!(
            s.pending_queue.lock().unwrap().len(),
            1,
            "human_typing 가드는 overdue 완화의 면제 대상이 아니다(R1 MED-2)"
        );
        // 사람 흔적 소거 → 같은 quiet(2s < 3s) 조건에서 overdue 제한 배달이 나간다.
        *s.last_human_input.lock().unwrap() = None;
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(2);
        deliver_queued(&daemon, &mut depth, &mut starve);
        assert!(
            s.pending_queue.lock().unwrap().is_empty(),
            "overdue 단계: 완화 quiet(1s)로 제한 배달"
        );
        let delivered: Vec<_> = daemon
            .bus
            .tail(30)
            .into_iter()
            .filter(|ev| ev["name"] == "queue.delivered")
            .collect();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0]["payload"]["overdue"], serde_json::json!(true));
        assert_eq!(
            delivered[0]["payload"]["forced"],
            serde_json::json!(false),
            "watchdog 배달은 forced 아님 — forced 는 queue.deliver RPC(W2-E) 전용"
        );
    }

    /// [starved 경보] 머리 대기 ≥ 임계 && 배달 막힘 지속 → queue.starved 정확히 1회 +
    /// 쿨다운(5분) 내 재발행 억제 + 배달 성공 시 쿨다운 리셋. 기아 경보는 단계형
    /// (max_wait)과 독립 축 — 비활성(0) 상태에서도 경보만 작동한다.
    #[test]
    fn queue_starved_alert_fires_once_with_cooldown_and_resets_on_delivery() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _env = QueueEnvGuard::set(&[
            ("CYS_QUEUE_MAX_WAIT_SECS", "0"),
            ("CYS_QUEUE_STARVE_ALERT_SECS", "1"),
        ]);
        let daemon = drill_daemon("w2d-starve");
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, None, 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        let e = daemon.next_queue_entry("기아 경보 핀".into(), None, "test");
        let head_id = e.id.clone();
        s.pending_queue.lock().unwrap().push_back(e);
        std::thread::sleep(std::time::Duration::from_millis(1300)); // uptime 클램프 대기 ≥ 1s
        let starved_count = |daemon: &Arc<Daemon>| {
            daemon
                .bus
                .tail(50)
                .iter()
                .filter(|ev| ev["name"] == "queue.starved")
                .count()
        };
        // busy(출력 직후·quiet_for=0)로 막는다 → 기아 경보 1회.
        *s.last_output.lock().unwrap() = std::time::Instant::now();
        let mut depth = HashMap::new();
        let mut starve = HashMap::new();
        deliver_queued(&daemon, &mut depth, &mut starve);
        assert_eq!(starved_count(&daemon), 1, "임계 도달 막힘 → queue.starved 1회");
        let ev = daemon
            .bus
            .tail(50)
            .into_iter()
            .find(|ev| ev["name"] == "queue.starved")
            .unwrap();
        assert_eq!(ev["payload"]["head_entry_id"], serde_json::json!(head_id));
        assert_eq!(ev["payload"]["blocked_by"], serde_json::json!("busy(출력 중)"));
        assert_eq!(
            ev["payload"]["hint"],
            serde_json::json!(crate::state::QUEUE_STARVED_HINT),
            "hint 문구 = 운영자(사람) 판단 계약 그대로(자동 반응 유도 금지)"
        );
        // 막힘 지속 → 쿨다운 내 재발행 억제.
        *s.last_output.lock().unwrap() = std::time::Instant::now();
        deliver_queued(&daemon, &mut depth, &mut starve);
        assert_eq!(starved_count(&daemon), 1, "쿨다운(5분) 내 재발행 억제");
        assert!(starve.contains_key(&s.id));
        // 막힘 해제 → 배달 → 쿨다운 리셋(다음 기아는 새 사건으로 다시 경보 가능).
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(4);
        *s.last_human_input.lock().unwrap() = None;
        deliver_queued(&daemon, &mut depth, &mut starve);
        assert!(s.pending_queue.lock().unwrap().is_empty(), "막힘 해제 후 정상 배달");
        assert!(
            !starve.contains_key(&s.id),
            "배달 성공 = 기아 해소 — 쿨다운 리셋"
        );
    }

    /// deliver_head_locked 단독 계약: 머리를 id 로 pop 하고 remaining 을 보고하며,
    /// 빈 큐는 None(부작용 0 — queue.delivered 미발행). watchdog 틱·queue.deliver RPC
    /// (W2-E)가 이 단일 헬퍼를 공유한다는 전제의 기초 핀.
    #[test]
    fn deliver_head_locked_pops_by_id_and_reports_remaining() {
        let daemon = drill_daemon("w2d-headlock");
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, None, 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        // 빈 큐 = None + 이벤트 0(부작용 없음).
        assert!(deliver_head_locked(&daemon, &s, false, false, None).is_none());
        assert_eq!(
            daemon
                .bus
                .tail(30)
                .iter()
                .filter(|ev| ev["name"] == "queue.delivered")
                .count(),
            0
        );
        let e1 = daemon.next_queue_entry("첫째".into(), None, "test");
        let e2 = daemon.next_queue_entry("둘째".into(), None, "test");
        let id1 = e1.id.clone();
        {
            let mut q = s.pending_queue.lock().unwrap();
            q.push_back(e1);
            q.push_back(e2);
        }
        let d = deliver_head_locked(&daemon, &s, true, false, None).expect("머리 배달");
        assert_eq!(d.entry.id, id1, "배달 = 머리 항목(id 판정)");
        assert_eq!(d.remaining, 1);
        assert_eq!(s.pending_queue.lock().unwrap().len(), 1, "pop 은 배달분 하나만");
        let ev = daemon
            .bus
            .tail(30)
            .into_iter()
            .find(|ev| ev["name"] == "queue.delivered")
            .unwrap();
        assert_eq!(ev["payload"]["queue_entry_id"], serde_json::json!(id1));
        assert_eq!(
            ev["payload"]["forced"],
            serde_json::json!(true),
            "forced 플래그는 이벤트 층에 그대로 실린다(RPC 감사 근거)"
        );
    }

    // ─────────── ★G1(W2-E): queue.deliver(운영자 강제 배달) — 게이트·경합·공유 핀 ───────────

    use super::{force_deliver_entry, ForceDeliverDenied, SeatState};

    /// W2-E 테스트 공용 준비물: 격리 데몬 + surface 1개(+원장 스레드 격리) — 초기 셸 출력이
    /// 판정 재료(last_output)를 덮지 않게 안정화한 뒤 돌려준다.
    fn force_deliver_rig(tag: &str, role: Option<&str>) -> (Arc<Daemon>, Arc<crate::state::Surface>) {
        crate::delivery::tests::isolate_state_dir_for_thread(tag);
        let daemon = drill_daemon(tag);
        let s = daemon
            .create_surface(None, Some("sleep 30".into()), None, role.map(|r| r.into()), 24, 80)
            .expect("create surface");
        daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
        std::thread::sleep(std::time::Duration::from_millis(700)); // 초기 셸 출력 안정화
        (daemon, s)
    }

    /// [★W2-E 신규 핀] deliver_head_locked expect_head_id — 조준 항목이 더는 머리가 아니면
    /// (틱 선배달·clear 경합) **무부작용** None: pop 0·원장 0·이벤트 0. RPC 강제 배달이
    /// '조준 아닌 다음 항목'을 forced 로 오배달하는 경로의 구조 봉인(pop-by-id 와 같은 층).
    #[test]
    fn deliver_head_locked_expect_id_mismatch_is_noop() {
        let (daemon, s) = force_deliver_rig("w2e-expect", None);
        let e1 = daemon.next_queue_entry("현재 머리".into(), None, "test");
        s.pending_queue.lock().unwrap().push_back(e1.clone());
        // 조준(다른 id)과 머리 불일치 → 배달·pop·이벤트 전무.
        assert!(deliver_head_locked(&daemon, &s, true, false, Some("q0.999")).is_none());
        assert_eq!(s.pending_queue.lock().unwrap().len(), 1, "불일치 시 pop 금지");
        assert_eq!(
            daemon.bus.tail(30).iter().filter(|ev| ev["name"] == "queue.delivered").count(),
            0,
            "불일치 시 배달 영수증도 없다(무부작용)"
        );
        // 대조군: 일치하면 정상 배달.
        assert!(deliver_head_locked(&daemon, &s, true, false, Some(&e1.id)).is_some());
        assert!(s.pending_queue.lock().unwrap().is_empty());
    }

    /// [W2-E 본계약] 강제 = 'quiet 대기 생략'만: 정상 quiet(3s) 미달(2s)이라 watchdog 은
    /// 보류할 조건에서도 배달되고, 영수증은 forced:true·overdue:false. 조준 생략 = 머리.
    #[test]
    fn force_deliver_skips_quiet_wait_and_marks_forced() {
        let (daemon, s) = force_deliver_rig("w2e-happy", None);
        let e1 = daemon.next_queue_entry("첫째".into(), None, "test");
        let e2 = daemon.next_queue_entry("둘째".into(), None, "test");
        {
            let mut q = s.pending_queue.lock().unwrap();
            q.push_back(e1.clone());
            q.push_back(e2.clone());
        }
        // quiet 2s: 현행 3s 규칙으로는 보류 조건 — 강제는 대기를 생략한다(하한 1s 는 충족).
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(2);
        *s.last_human_input.lock().unwrap() = None;
        let d = force_deliver_entry(&daemon, &s, None, false).expect("강제 배달");
        assert_eq!(d.entry.id, e1.id, "조준 생략 = 머리 항목");
        assert_eq!(d.remaining, 1);
        assert_eq!(
            s.pending_queue.lock().unwrap().front().map(|e| e.id.clone()),
            Some(e2.id.clone()),
            "단건 전용 — 다음 항목은 남는다(드레인 아님)"
        );
        let ev = daemon
            .bus
            .tail(30)
            .into_iter()
            .find(|ev| ev["name"] == "queue.delivered")
            .unwrap();
        assert_eq!(ev["payload"]["forced"], serde_json::json!(true));
        assert_eq!(
            ev["payload"]["overdue"],
            serde_json::json!(false),
            "forced 는 overdue 와 별개 축(운영자 강제 ≠ 단계형 완화)"
        );
    }

    /// [게이트 독립 핀 ④] human typing 신선 → 강제로도 배달 0건(R1 MED-2 면제 절대 불가).
    #[test]
    fn force_deliver_typing_guard_never_exempt() {
        let (daemon, s) = force_deliver_rig("w2e-typing", None);
        let e = daemon.next_queue_entry("보류 대상".into(), None, "test");
        s.pending_queue.lock().unwrap().push_back(e);
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(5);
        *s.last_human_input.lock().unwrap() = Some(std::time::Instant::now());
        let denied = force_deliver_entry(&daemon, &s, None, false).unwrap_err();
        assert_eq!(denied, ForceDeliverDenied::TypingGuard);
        assert_eq!(denied.code(), "typing_guard");
        assert_eq!(s.pending_queue.lock().unwrap().len(), 1, "배달 0건 — 큐 보존");
    }

    /// [게이트 독립 핀 ⑤] queue_paused(헬스 조치) → 강제 배달도 거부(우회 불가).
    #[test]
    fn force_deliver_queue_paused_refused() {
        let (daemon, s) = force_deliver_rig("w2e-qpause", None);
        let e = daemon.next_queue_entry("보류 대상".into(), None, "test");
        s.pending_queue.lock().unwrap().push_back(e);
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(5);
        *s.queue_paused_until.lock().unwrap() =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
        let denied = force_deliver_entry(&daemon, &s, None, false).unwrap_err();
        assert_eq!(denied, ForceDeliverDenied::QueuePaused);
        assert_eq!(denied.code(), "queue_paused");
        assert_eq!(s.pending_queue.lock().unwrap().len(), 1);
    }

    /// [게이트 독립 핀 ③] role 좌석 + 좌석 Empty → 거부(2026-07-17 빈 좌석 zsh 오타이핑
    /// 사고를 운영자 경로에서 재개방하지 않는다). Unknown 은 통과(현행 동작 강등 — 대조군).
    #[test]
    fn force_deliver_empty_seat_refused_unknown_passes() {
        let (daemon, s) = force_deliver_rig("w2e-seat", Some("worker-w2e"));
        let e = daemon.next_queue_entry("보류 대상".into(), None, "test");
        s.pending_queue.lock().unwrap().push_back(e);
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(5);
        *s.last_human_input.lock().unwrap() = None;
        s.seat_cache.store(SeatState::Empty.as_u8(), AtomicOrdering::Relaxed);
        let denied = force_deliver_entry(&daemon, &s, None, false).unwrap_err();
        assert_eq!(denied, ForceDeliverDenied::EmptySeat);
        assert_eq!(denied.code(), "empty_seat");
        assert_eq!(s.pending_queue.lock().unwrap().len(), 1);
        // 대조군: Unknown(프로브 미도달) = 통과 — 판정 실패가 강제 경로를 막는 새 장애 금지.
        s.seat_cache.store(SeatState::Unknown.as_u8(), AtomicOrdering::Relaxed);
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(5);
        assert!(force_deliver_entry(&daemon, &s, None, false).is_ok());
    }

    /// [★성찰 BLOCKER 핀 ⑥] forced 에도 overdue_quiet(기본 1s) 하한 — 출력 한복판 주입
    /// 금지는 운영자 강제로도 불변. 오설정(0)도 하한 1s 로 승격된다(queue_quiet_verdict 동형).
    #[test]
    fn force_deliver_output_busy_floor_holds_even_forced() {
        let _g = QUEUE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _env = QueueEnvGuard::set(&[("CYS_QUEUE_OVERDUE_QUIET_SECS", "0")]);
        let (daemon, s) = force_deliver_rig("w2e-busy", None);
        let e = daemon.next_queue_entry("보류 대상".into(), None, "test");
        s.pending_queue.lock().unwrap().push_back(e);
        *s.last_human_input.lock().unwrap() = None;
        // 지금 출력 중(quiet_for=0) — env 0 오설정도 하한 1s 를 뚫지 못한다.
        *s.last_output.lock().unwrap() = std::time::Instant::now();
        let denied = force_deliver_entry(&daemon, &s, None, false).unwrap_err();
        assert_eq!(denied, ForceDeliverDenied::OutputBusy { quiet_for: 0, need: 1 });
        assert_eq!(denied.code(), "output_busy");
        assert_eq!(s.pending_queue.lock().unwrap().len(), 1, "출력 중 강제 주입 0건");
        // 하한 충족(1s+) 시 배달 — 하한이 '영구 차단'이 아님도 함께 핀.
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(2);
        assert!(force_deliver_entry(&daemon, &s, None, false).is_ok());
    }

    /// [조준 핀] 빈 큐 = queue_empty · 미지 entry_id = not_found — 게이트 통과 후의 조준
    /// 실패는 안전 게이트 거부와 구분되는 별도 코드(CLI exit 1 계열)다.
    #[test]
    fn force_deliver_empty_queue_and_unknown_id() {
        let (daemon, s) = force_deliver_rig("w2e-aim", None);
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(5);
        *s.last_human_input.lock().unwrap() = None;
        assert_eq!(
            force_deliver_entry(&daemon, &s, None, false).unwrap_err(),
            ForceDeliverDenied::QueueEmpty
        );
        let e = daemon.next_queue_entry("실존 항목".into(), None, "test");
        s.pending_queue.lock().unwrap().push_back(e);
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(5);
        let denied = force_deliver_entry(&daemon, &s, Some("q0.404"), false).unwrap_err();
        assert_eq!(denied, ForceDeliverDenied::NotFound);
        assert_eq!(denied.code(), "not_found");
        assert_eq!(s.pending_queue.lock().unwrap().len(), 1, "미지 조준은 무부작용");
    }

    /// [재정렬 핀] 비머리 조준: allow_reorder 미지정 → not_head 거부(순서 불변·무이벤트),
    /// 지정 → queue.reordered {queue_entry_id, from_index, to_index:0, cause:force_deliver}
    /// 발행 후 그 항목이 forced 배달되고, 종전 머리는 순서대로 남는다.
    #[test]
    fn force_deliver_not_head_requires_allow_reorder_then_reorders_with_event() {
        let (daemon, s) = force_deliver_rig("w2e-reorder", None);
        let e1 = daemon.next_queue_entry("머리".into(), None, "test");
        let e2 = daemon.next_queue_entry("가운데".into(), None, "test");
        let e3 = daemon.next_queue_entry("꼬리".into(), None, "test");
        {
            let mut q = s.pending_queue.lock().unwrap();
            q.push_back(e1.clone());
            q.push_back(e2.clone());
            q.push_back(e3.clone());
        }
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(5);
        *s.last_human_input.lock().unwrap() = None;
        // ① allow_reorder 미지정 → 거부 + 순서 완전 불변 + 재정렬 이벤트 없음(무음 재정렬 금지의 역핀).
        let denied = force_deliver_entry(&daemon, &s, Some(&e3.id), false).unwrap_err();
        assert_eq!(denied, ForceDeliverDenied::NotHead { index: 2 });
        assert_eq!(denied.code(), "not_head_requires_allow_reorder");
        {
            let q = s.pending_queue.lock().unwrap();
            let order: Vec<&str> = q.iter().map(|e| e.id.as_str()).collect();
            assert_eq!(order, vec![e1.id.as_str(), e2.id.as_str(), e3.id.as_str()]);
        }
        assert_eq!(
            daemon.bus.tail(30).iter().filter(|ev| ev["name"] == "queue.reordered").count(),
            0
        );
        // ② allow_reorder 지정 → 재정렬 이벤트 + 조준 항목 forced 배달 + 나머지 순서 보존.
        *s.last_output.lock().unwrap() =
            std::time::Instant::now() - std::time::Duration::from_secs(5);
        let d = force_deliver_entry(&daemon, &s, Some(&e3.id), true).expect("재정렬 강제 배달");
        assert_eq!(d.entry.id, e3.id, "배달 = 조준 항목(끌어올린 머리)");
        assert_eq!(d.remaining, 2);
        {
            let q = s.pending_queue.lock().unwrap();
            let order: Vec<&str> = q.iter().map(|e| e.id.as_str()).collect();
            assert_eq!(order, vec![e1.id.as_str(), e2.id.as_str()], "잔여는 원 순서 그대로");
        }
        let rev = daemon
            .bus
            .tail(30)
            .into_iter()
            .find(|ev| ev["name"] == "queue.reordered")
            .expect("재정렬은 명시 이벤트로 발행(무음 금지)");
        assert_eq!(rev["payload"]["queue_entry_id"], serde_json::json!(e3.id));
        assert_eq!(rev["payload"]["from_index"], serde_json::json!(2));
        assert_eq!(rev["payload"]["to_index"], serde_json::json!(0));
        assert_eq!(rev["payload"]["cause"], serde_json::json!("force_deliver"));
        let dev = daemon
            .bus
            .tail(30)
            .into_iter()
            .find(|ev| ev["name"] == "queue.delivered")
            .unwrap();
        assert_eq!(dev["payload"]["queue_entry_id"], serde_json::json!(e3.id));
        assert_eq!(dev["payload"]["forced"], serde_json::json!(true));
    }

    /// [★W2-E 공유 확인 핀 — 소스핀] watchdog 틱(deliver_queued)과 queue.deliver RPC
    /// (force_deliver_entry)는 배달 임계영역 **단일 헬퍼** deliver_head_locked 를 공유한다:
    /// ① 큐 배달의 원장 기록 지점(crate::delivery::record_audited 호출)은 프로덕션 코드에
    ///    정확히 1곳 — 배달 구현이 두 벌로 갈라지는 순간(한쪽만 고쳐지는 관례 위반) 깨진다.
    /// ② 두 경로 함수 몸통이 각각 그 헬퍼를 호출한다.
    /// (main.rs·state.rs 소스핀 관례 동형 — 로직 무변경 검증 전용.)
    #[test]
    fn queue_delivery_single_helper_shared_by_tick_and_rpc() {
        let src = include_str!("governance.rs");
        let prod = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        assert_eq!(
            prod.matches("crate::delivery::record_audited").count(),
            1,
            "큐 배달 원장 기록 지점은 deliver_head_locked 안 정확히 1곳이어야 한다 — \
             늘었다면 배달 구현이 갈라졌다(단일 헬퍼 관례 위반)"
        );
        let tick_at = prod.find("fn deliver_queued").expect("watchdog 틱 배달자 소실");
        let rpc_at = prod.find("fn force_deliver_entry").expect("RPC 강제 배달 본체 소실");
        let tick_body = &prod[tick_at..];
        let rpc_body = &prod[rpc_at..tick_at];
        assert!(
            tick_body.contains("deliver_head_locked("),
            "watchdog 틱이 단일 헬퍼 호출을 잃었다"
        );
        assert!(
            rpc_body.contains("deliver_head_locked("),
            "queue.deliver RPC 경로가 단일 헬퍼 호출을 잃었다"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ★P4-5 절단 캐스트 — `as u32` 는 클램프가 아니다
    //
    // 같은 캠페인의 같은 커밋이 `cys.rs rpc_idle_timeout_with` 에서 `v.min(MAX)` 로 고친
    // 결함의 **새 인스턴스**가 이 파일에 남아 있었다(규율이 커밋 안에서 갈렸다). 아래 셋은
    // ① 순수 진리표(거대값 축) ② 소비자 방향 확인(오사망 축) ③ 전수 소스핀 이다.
    // ─────────────────────────────────────────────────────────────────────────

    /// ★거대값 진리표 — 절단은 **단조**가 아니고, 그 비단조 구간이 곧 결함이다.
    ///
    /// 두 wrap 귀결의 **방향이 서로 다르다**는 것이 요점이다:
    ///   · `2^32   → 0` 축이 조용히 전면 비활성(사람은 '아주 큰 임계'를 적었다)
    ///   · `2^32+1 → 1` 임계가 **가장 느슨해진다** — 이 저장소에서 그 방향은 오사망이다.
    #[test]
    fn env_u32_clamps_instead_of_truncating() {
        use super::clamp_env_u32 as c;
        assert_eq!(c(0), 0, "0 은 킬스위치 — 의미를 바꾸지 않는다");
        assert_eq!(c(2), 2);
        assert_eq!(c(u64::from(u32::MAX)), u32::MAX, "경계값은 그대로 통과");
        assert_eq!(c(4_294_967_296), u32::MAX, "2^32 가 0 이 되면 축이 조용히 꺼진다");
        assert_eq!(
            c(4_294_967_297),
            u32::MAX,
            "2^32+1 이 1 이 되면 연속 1틱만으로 증명이 선다 — 주석이 막겠다던 \
             '스치는 도우미 프로세스'가 그대로 증명을 세운다(오사망)"
        );
        assert_eq!(c(u64::MAX), u32::MAX);
        // ★일반화 — 절단이면 반드시 깨지는 성질. 개별 값 나열은 다음 절단을 못 잡는다.
        let probes = [
            0u64,
            1,
            2,
            40,
            4_294_967_295,
            4_294_967_296,
            4_294_967_297,
            8_589_934_592,
            u64::MAX,
        ];
        for w in probes.windows(2) {
            assert!(
                c(w[0]) <= c(w[1]),
                "단조성 위반: {} → {} 인데 {} → {} (큰 값이 더 느슨해진다)",
                w[0],
                c(w[0]),
                w[1],
                c(w[1])
            );
        }
    }

    /// ★클램프의 **방향** — 거대값은 "증명이 서지 않는다"(보수) 쪽으로만 접혀야 한다.
    ///
    /// ⓐ 절단본에서는 `2^32+1 → 1` 이라 **첫 틱에** 증명이 선다(오사망).
    /// ⓑ 양성 대조군 — 정상 임계 2 에서는 2틱 연속으로 증명이 서야 한다. ⓑ가 없으면
    ///    `update_strict_proof` 가 통째로 죽어도 ⓐ가 초록이라 아무것도 증명하지 못한다.
    #[test]
    fn giant_strict_proof_ticks_never_arms_the_proof() {
        use super::{clamp_env_u32, update_strict_proof};
        let arm = clamp_env_u32(4_294_967_297);
        let mut proof = None;
        for tick in 0..1000 {
            assert!(
                !update_strict_proof(&mut proof, "claude", true, arm),
                "거대 임계가 1틱으로 절단돼 {tick}틱에 증명이 섰다 — \
                 스치는 도우미 프로세스가 살아있는 좌석을 사망 판정시킨다"
            );
        }
        let mut ctrl = None;
        assert!(!update_strict_proof(&mut ctrl, "claude", true, 2), "1틱째는 미증명");
        assert!(
            update_strict_proof(&mut ctrl, "claude", true, 2),
            "정상 임계에서 증명이 서지 않으면 좁힘 자체가 죽은 것이다(양성 대조군)"
        );
    }

    /// ★전수 소스핀 — 프로덕션 슬라이스에 **좁힘 캐스트가 하나도 없다**.
    ///
    /// 개별 결함 3건을 고쳐도 다음 사람이 같은 자리에 다시 넣으면 그만이다. 그래서 값이
    /// 아니라 **규율**을 못박는다: env·길이·카운터의 폭 변환은 `try_from(..).unwrap_or(MAX)`
    /// 또는 `min(MAX)` 로 쓴다. (`SeatState::as_u8` 의 `self as u8` 는 enum 판별식 캐스트라
    ///  폭이 줄지 않는다 — 금칙 목록에 넣지 않는다.)
    ///
    /// ★계측 타당성: 트리에 위반이 0 이면 탐지기가 고장나도 초록이다. 그래서 합성 변조본으로
    /// 탐지 능력 자체를 함께 시험한다(코드에 심으면 적발 · 주석에 있으면 무시).
    #[test]
    fn production_slice_has_no_narrowing_casts() {
        /// `//` 줄주석 제거 — 이 핀은 "무엇을 **쓰는가**" 를 보는 장치이고 주석은 설명이다
        /// (바로 위 검체들의 설명문에 `as u32` 라는 글자가 그대로 들어 있다).
        fn strip_line_comments(src: &str) -> String {
            src.lines()
                .map(|l| match l.find("//") {
                    Some(i) => &l[..i],
                    None => l,
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        const NARROWING: [&str; 3] = ["as u32", "as u16", "as usize"];
        let src = include_str!("governance.rs");
        let raw = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let code = strip_line_comments(raw);
        let hits: Vec<&str> = NARROWING
            .iter()
            .copied()
            .filter(|w| code.contains(w))
            .collect();
        assert!(
            hits.is_empty(),
            "프로덕션에 좁힘 캐스트가 남았다: {hits:?} — `as` 는 wrap 이다. \
             `u32::try_from(v).unwrap_or(u32::MAX)` 또는 `v.min(MAX)` 로 클램프하라"
        );
        // 합성 변조본 ① — 코드에 심으면 반드시 적발된다(탐지기 생존 증명).
        for w in NARROWING {
            let mutant = format!("{code}\nfn __mutant(v: u64) -> u32 {{ v {w} }}\n");
            assert!(
                NARROWING.iter().any(|x| strip_line_comments(&mutant).contains(x)),
                "합성 변조본에서 {w:?} 를 적발하지 못했다 = 탐지기 고장"
            );
        }
        // 합성 변조본 ② — 주석 안의 같은 문장은 적발하지 않는다(핀이 설명을 죽이지 않는다).
        let commented = format!("{raw}\n// 여기서 as u32 로 자르면 안 된다\n");
        assert!(
            NARROWING
                .iter()
                .all(|w| !strip_line_comments(&commented).contains(w)),
            "주석 문장을 위반으로 읽었다 — 다음 사람이 설명을 지우거나 핀을 완화하게 된다"
        );
    }

    /// reap 경계: exited 후 grace 미만이면 보존(포렌식·복구 윈도우), 이상이면 회수.
    /// 역할 노드는 60초, 비역할은 10초로 더 빨리 정리 — 자력종료 surface 누수 차단의 핵심 불변식.
    /// ★G4(W4-C): 공유 락 + 기본값 명시 고정 — grace env 를 만지는 테스트(governance·handlers
    /// 수동 reap)와 직렬화해, env 설정 창과 겹칠 때의 경계값 flake 를 소거한다(핀 강화·무약화).
    #[test]
    fn exited_surface_due_respects_role_grace() {
        use super::exited_surface_due;
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[
            ("CYS_REAP_EXITED_GRACE_SECS", "60"),
            ("CYS_REAP_EXITED_NONROLE_GRACE_SECS", "10"),
        ]);
        // 역할 노드: 기본 60초 grace — 경계 직전 보존, 경계에서 회수
        assert!(!exited_surface_due(true, 59), "역할 노드는 grace 내(59s)에 보존돼야");
        assert!(exited_surface_due(true, 60), "역할 노드는 grace 경계(60s)에서 회수돼야");
        // 비역할(스크래치·one-shot): 기본 10초 grace — 더 빨리 정리
        assert!(!exited_surface_due(false, 9), "비역할은 grace 내(9s)에 보존돼야");
        assert!(exited_surface_due(false, 10), "비역할은 grace 경계(10s)에서 회수돼야");
    }

    // ─────────── ★묘비 게이트: reap≠묘비, owner-close=묘비 (부활 불변식) ───────────

    use super::{
        close_surface, load_tombstones_from_disk, load_tombstones_rev_from_disk, now_epoch,
        persist_topology, reap_exited_surfaces, CloseCause, Daemon,
    };
    use std::sync::atomic::Ordering as AtomicOrdering;

    // reap 계열 env 직렬화 락·원복 가드 — ★G4(W4-C) 모듈 스코프(pub(crate)·cfg(test))로
    // 격상해 handlers::tests(수동 reap RPC의 grace 판정 테스트)와 **한 락을 공유**한다.
    // 모듈별 사설 락 두 개는 서로를 직렬화하지 못해 CYS_REAP_EXITED_GRACE_SECS 경합 flake 가 난다.
    use super::{ReapEnvGuard, REAP_ENV_LOCK};

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

    // ─────────── ★G2(W3-A): role dead-man v2 — idle/death 축 분리 회귀 핀 ───────────
    //
    // 결함 8의 박제: 살아있는 zsh 의 침묵(임계 20s·침묵 21s)이 master.deadman(alert)으로
    // 오라벨되던 v1 경로를 타입(LivenessVerdict — 침묵은 Idle 로만 사상)과 절차(grace +
    // 연속 confirm 소진에서만 death)로 봉인했음을 실측 시나리오 그대로 핀한다.
    // env 를 만지는 통합 핀은 전부 REAP_ENV_LOCK 직렬화(기본값 의존 테스트 포함 — 경합 차단).

    use super::{check_role_deadman, liveness_verdict, DeadmanAxis, DeadmanTracker, LivenessVerdict};

    /// master role 좌석 하나를 가진 격리 데몬 — (daemon, sid).
    fn deadman_daemon(tag: &str) -> (Arc<Daemon>, u64) {
        let daemon = drill_daemon(tag);
        let sid = spawn_role_surface(&daemon, "master");
        (daemon, sid)
    }

    fn events_named(daemon: &Arc<Daemon>, name: &str) -> Vec<serde_json::Value> {
        daemon
            .bus
            .replay_after(0)
            .into_iter()
            .filter(|e| e["name"] == name)
            .collect()
    }

    fn backdate_output(s: &crate::state::Surface, secs: u64) {
        *s.last_output.lock().unwrap() = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(secs))
            .expect("clock too young for backdate");
    }

    /// [핀1·결함 8 박제 — 순수 판정자 진리표] 침묵은 어떤 좌석 상태에서도 Idle 로만 사상되고
    /// DeadCandidate 가 될 수 없다. 실측 시나리오(임계 20s·침묵 21s의 살아있는 zsh) 그대로.
    #[test]
    fn liveness_verdict_silence_maps_to_idle_never_death() {
        let (daemon, sid) = deadman_daemon("lv-idle");
        let s = daemon.get_surface(sid).unwrap();
        backdate_output(&s, 21);
        // 맨 셸(무meta·미arm) — seat=Empty 여도 침묵은 Idle 이다(한 번도 에이전트가 없던 좌석).
        for seat in [SeatState::Empty, SeatState::Occupied, SeatState::Unknown] {
            match liveness_verdict(Some(&s), seat, true, false, false, 20) {
                LivenessVerdict::Idle { idle_secs } => {
                    assert!(idle_secs >= 21, "실측 침묵 21s 이상이어야: {idle_secs}")
                }
                v => panic!("침묵 단독은 Idle 이어야 한다(결함 8) — got {v:?} (seat={seat:?})"),
            }
        }
        // armed + Occupied 도 침묵이면 Idle(자손 존재 = 생존).
        assert!(matches!(
            liveness_verdict(Some(&s), SeatState::Occupied, true, true, false, 20),
            LivenessVerdict::Idle { .. }
        ));
        // 에이전트 생존(meta·seen·미notified) + 침묵 = Idle — 에이전트 '생각 중' 오살 금지.
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/bin/claude".into()));
        s.agent_seen.store(true, AtomicOrdering::Relaxed);
        s.agent_exit_notified.store(false, AtomicOrdering::Relaxed);
        assert!(matches!(
            liveness_verdict(Some(&s), SeatState::Empty, true, true, false, 20),
            LivenessVerdict::Idle { .. }
        ));
        // 임계 0 = idle 신호만 비활성(Alive) — 사망 축 판정은 별개(아래 진리표).
        *s.agent_meta.lock().unwrap() = None;
        assert_eq!(
            liveness_verdict(Some(&s), SeatState::Occupied, true, false, false, 0),
            LivenessVerdict::Alive
        );
    }

    /// [핀1보강 — 순수 판정자 사망 축 진리표] 4축 + meta 부재 보조축 + Unknown 무증감 값.
    #[test]
    fn liveness_verdict_death_axes_truth_table() {
        let (daemon, sid) = deadman_daemon("lv-axes");
        let s = daemon.get_surface(sid).unwrap();
        // surface 소멸(데몬 내부 사실)
        assert_eq!(
            liveness_verdict(None, SeatState::Unknown, false, false, false, 900),
            LivenessVerdict::DeadCandidate { axis: DeadmanAxis::SurfaceGone }
        );
        // 셸 pid 커널 프로브 사망(half-open) — state::pid_alive 입력이 권위
        assert_eq!(
            liveness_verdict(Some(&s), SeatState::Occupied, false, false, false, 900),
            LivenessVerdict::DeadCandidate { axis: DeadmanAxis::ShellProcDead }
        );
        // 좌석 빈사: meta ∧ seen ∧ exit_notified(check_agent_death 상태머신 재사용)
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/bin/claude".into()));
        s.agent_seen.store(true, AtomicOrdering::Relaxed);
        s.agent_exit_notified.store(true, AtomicOrdering::Relaxed);
        assert_eq!(
            liveness_verdict(Some(&s), SeatState::Empty, true, true, false, 900),
            LivenessVerdict::DeadCandidate { axis: DeadmanAxis::AgentDead }
        );
        // ★기동 즉사 쌍둥이 셀(리뷰 MAJOR): meta ∧ !agent_seen(set_meta 리셋 후 첫 관측 전
        // 크래시) — grace 소진 ∧ Empty 만 후보. 미소진·Occupied·Unknown 은 각각 fail-safe.
        s.agent_seen.store(false, AtomicOrdering::Relaxed);
        s.agent_exit_notified.store(false, AtomicOrdering::Relaxed);
        assert_eq!(
            liveness_verdict(Some(&s), SeatState::Empty, true, false, true, 900),
            LivenessVerdict::DeadCandidate { axis: DeadmanAxis::AgentNeverStarted },
            "기동 즉사(meta ∧ 미관측 ∧ Empty 지속)는 사망 후보여야 — 미탐 셀 회귀 금지"
        );
        assert_eq!(
            liveness_verdict(Some(&s), SeatState::Empty, true, false, false, 900),
            LivenessVerdict::Alive,
            "grace 미소진 = 판정 휴지(정상 기동 스폰 지연 보호 — fail-safe)"
        );
        assert_eq!(
            liveness_verdict(Some(&s), SeatState::Unknown, true, false, true, 900),
            LivenessVerdict::Unknown,
            "미기동 좌석의 프로브 미도달은 Unknown(무증감)"
        );
        assert_eq!(
            liveness_verdict(Some(&s), SeatState::Occupied, true, false, true, 900),
            LivenessVerdict::Alive,
            "기동 중 래퍼 프로세스 가시(Occupied)는 무판정 통과"
        );
        // meta 부재 보조축: armed(기지 에이전트 엄격 관측 이력) ∧ 지금 Empty → 후보 / Unknown → 무증감
        *s.agent_meta.lock().unwrap() = None;
        assert_eq!(
            liveness_verdict(Some(&s), SeatState::Empty, true, true, false, 900),
            LivenessVerdict::DeadCandidate { axis: DeadmanAxis::SeatVacantNoMeta }
        );
        assert_eq!(
            liveness_verdict(Some(&s), SeatState::Unknown, true, true, false, 900),
            LivenessVerdict::Unknown,
            "armed 좌석의 프로브 미도달은 Unknown(측정 불능 ≠ 사망 ≠ 생존)"
        );
        // EOF 자력종료(exited) — 최우선 축(surface 가 있으면)
        s.exited.store(true, AtomicOrdering::Relaxed);
        assert_eq!(
            liveness_verdict(Some(&s), SeatState::Occupied, true, false, false, 900),
            LivenessVerdict::DeadCandidate { axis: DeadmanAxis::SurfaceExited }
        );
    }

    /// [핀1 통합·결함 8 종결] 살아있는 셸이 임계 20s·침묵 21s 로 몇 틱을 돌아도
    /// master.deadman 0건 — master.idle(category=info·GUI 폴백 토스트 레인 부재)만 1건(디바운스).
    #[test]
    fn deadman_silence_alone_is_idle_not_death() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[
            ("CYS_MASTER_DEADMAN_SECS", "20"),
            ("CYS_ROLE_DEADMAN_GRACE_SECS", "0"),
        ]);
        let (daemon, sid) = deadman_daemon("silence");
        let s = daemon.get_surface(sid).unwrap();
        let mut tracker = DeadmanTracker::default();
        for _ in 0..5 {
            backdate_output(&s, 21);
            check_role_deadman(&daemon, &mut tracker);
        }
        assert_eq!(
            events_named(&daemon, "master.deadman").len(),
            0,
            "침묵 단독으로 death 발화 — 결함 8 재발"
        );
        let idles = events_named(&daemon, "master.idle");
        assert_eq!(idles.len(), 1, "idle 은 정보성 1건(300s 디바운스)이어야");
        let e = &idles[0];
        assert_eq!(e["category"], "info", "GUI category 폴백 토스트 레인(watchdog) 낙하 금지");
        let p = &e["payload"];
        assert_eq!(p["axis"], "silence");
        assert_eq!(p["role"], "master");
        assert_eq!(p["threshold_secs"], 20);
        assert_eq!(p["process_alive"], true);
        assert_eq!(p["severity"], "info");
        assert!(p["idle_secs"].as_u64().unwrap() >= 21);
        // 디바운스 해제 후 재발화(정보 신호의 재상기 — death 와 별도 디바운스 축)
        tracker
            .last_idle_alert
            .insert("master".into(), now_epoch() - 301.0);
        backdate_output(&s, 21);
        check_role_deadman(&daemon, &mut tracker);
        assert_eq!(events_named(&daemon, "master.idle").len(), 2);
        assert_eq!(events_named(&daemon, "master.deadman").len(), 0);
    }

    /// [핀2] DeadCandidate 연속 1·2회는 무발화, 3회째(기본 CONFIRM_TICKS)에 1회 발화 + misses==3.
    #[test]
    fn deadman_fires_only_after_confirm_ticks() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let (daemon, sid) = deadman_daemon("confirm");
        let s = daemon.get_surface(sid).unwrap();
        s.exited.store(true, AtomicOrdering::Relaxed);
        let mut tracker = DeadmanTracker::default();
        check_role_deadman(&daemon, &mut tracker);
        check_role_deadman(&daemon, &mut tracker);
        assert_eq!(
            events_named(&daemon, "master.deadman").len(),
            0,
            "확증 미소진(2/3) 발화 금지"
        );
        check_role_deadman(&daemon, &mut tracker);
        let deaths = events_named(&daemon, "master.deadman");
        assert_eq!(deaths.len(), 1, "3틱째 확증 소진 발화");
        assert_eq!(deaths[0]["payload"]["misses"], 3);
    }

    /// [핀3] Unknown(프로브 미도달) 틱은 misses 증가도 last_ok 갱신도 없다 —
    /// 간헐 프로브 실패가 사망 확증 카운터를 세탁하지 못한다.
    #[test]
    fn deadman_unknown_probe_neither_counts_nor_resets() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let (daemon, sid) = deadman_daemon("unknown");
        let s = daemon.get_surface(sid).unwrap();
        let mut tracker = DeadmanTracker::default();
        // arm: 기지 에이전트 엄격 관측(refresh_seat_cache 의 seat_agent_cache 대역 —
        // ★원시 Occupied 만으로는 armed 되지 않는다 · BLOCK 교정) — 생존 틱
        s.seat_cache.store(SeatState::Occupied.as_u8(), AtomicOrdering::Relaxed);
        s.seat_agent_cache.store(true, AtomicOrdering::Relaxed);
        check_role_deadman(&daemon, &mut tracker);
        // 이후 틱은 에이전트 미관측(프롬프트 복귀 — refresh 대역이 false 로 되돌린다)
        s.seat_agent_cache.store(false, AtomicOrdering::Relaxed);
        let ok_after_arm = tracker.last_ok.get("master").copied().expect("생존 관측 기록");
        // Empty 지속 2틱 → miss 2
        s.seat_cache.store(SeatState::Empty.as_u8(), AtomicOrdering::Relaxed);
        check_role_deadman(&daemon, &mut tracker);
        check_role_deadman(&daemon, &mut tracker);
        assert_eq!(tracker.misses.get("master"), Some(&2));
        // Unknown 틱 — 증가도 리셋도 없음
        s.seat_cache.store(SeatState::Unknown.as_u8(), AtomicOrdering::Relaxed);
        check_role_deadman(&daemon, &mut tracker);
        assert_eq!(tracker.misses.get("master"), Some(&2), "Unknown 이 카운트하면 안 됨");
        assert_eq!(
            tracker.last_ok.get("master").copied(),
            Some(ok_after_arm),
            "Unknown 이 last_ok 를 전진시키면 안 됨(측정 불능 ≠ 생존)"
        );
        assert_eq!(events_named(&daemon, "master.deadman").len(), 0);
        // Empty 재관측 → miss 3 = 확증 소진 발화(연속 실관측 3회 요구가 세탁되지 않았음의 증명)
        s.seat_cache.store(SeatState::Empty.as_u8(), AtomicOrdering::Relaxed);
        check_role_deadman(&daemon, &mut tracker);
        let deaths = events_named(&daemon, "master.deadman");
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0]["payload"]["axis"], "seat_vacant_no_meta");
        assert_eq!(deaths[0]["payload"]["misses"], 3);
    }

    /// [핀4] miss 누적 중 에이전트 복귀(agent.recovered 경로 = exit_notified 리셋)가
    /// misses 를 0으로 되돌린다 — 이후 재실패도 3회를 새로 요구.
    #[test]
    fn deadman_agent_recovery_resets_misses() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let (daemon, sid) = deadman_daemon("recovery");
        let s = daemon.get_surface(sid).unwrap();
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/bin/claude".into()));
        s.agent_seen.store(true, AtomicOrdering::Relaxed);
        s.agent_exit_notified.store(true, AtomicOrdering::Relaxed);
        let mut tracker = DeadmanTracker::default();
        check_role_deadman(&daemon, &mut tracker);
        check_role_deadman(&daemon, &mut tracker);
        assert_eq!(tracker.misses.get("master"), Some(&2));
        // 복귀(check_agent_death 의 agent.recovered 와 동일 상태 전이)
        s.agent_exit_notified.store(false, AtomicOrdering::Relaxed);
        check_role_deadman(&daemon, &mut tracker);
        assert_eq!(tracker.misses.get("master"), None, "복귀 = misses 리셋");
        // 재사망 2틱 — 새로 세므로 여전히 무발화
        s.agent_exit_notified.store(true, AtomicOrdering::Relaxed);
        check_role_deadman(&daemon, &mut tracker);
        check_role_deadman(&daemon, &mut tracker);
        assert_eq!(events_named(&daemon, "master.deadman").len(), 0);
    }

    /// [핀5·요구 e] 좌석 빈사(셸 생존·에이전트 사망)는 axis=agent_dead 로 셸 사망과 구분 노출.
    #[test]
    fn deadman_agent_dead_axis_when_shell_alive() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let (daemon, sid) = deadman_daemon("agent-dead");
        let s = daemon.get_surface(sid).unwrap();
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/bin/claude".into()));
        s.agent_seen.store(true, AtomicOrdering::Relaxed);
        s.agent_exit_notified.store(true, AtomicOrdering::Relaxed);
        s.seat_cache.store(SeatState::Empty.as_u8(), AtomicOrdering::Relaxed);
        let mut tracker = DeadmanTracker::default();
        for _ in 0..3 {
            check_role_deadman(&daemon, &mut tracker);
        }
        let deaths = events_named(&daemon, "master.deadman");
        assert_eq!(deaths.len(), 1);
        let p = &deaths[0]["payload"];
        assert_eq!(p["axis"], "agent_dead");
        assert_eq!(p["reason"], "agent process dead");
        assert_eq!(p["inputs"]["seat_state"], "empty");
        assert_eq!(p["inputs"]["agent_alive"], false);
        assert_eq!(p["inputs"]["agent_meta"], "claude");
        assert_eq!(p["role"], "master");
    }

    /// [핀6] v2 payload 계약 전 필드 + surface_exited 의 reason 구값 그대로(하위호환 핀).
    #[test]
    fn deadman_payload_contract_v2() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let (daemon, sid) = deadman_daemon("payload");
        let s = daemon.get_surface(sid).unwrap();
        let mut tracker = DeadmanTracker::default();
        // 생존 1틱 → last_ok 기록(발화 payload 의 last_ok_epoch 실측)
        check_role_deadman(&daemon, &mut tracker);
        s.exited.store(true, AtomicOrdering::Relaxed);
        for _ in 0..3 {
            check_role_deadman(&daemon, &mut tracker);
        }
        let deaths = events_named(&daemon, "master.deadman");
        assert_eq!(deaths.len(), 1);
        let e = &deaths[0];
        assert_eq!(e["category"], "alert", "이벤트 category 불변(소비자 계약)");
        let p = &e["payload"];
        // 하위호환: reason 구값 verbatim (v1 "master surface exited")
        assert_eq!(p["reason"], "master surface exited");
        assert_eq!(p["axis"], "surface_exited");
        assert_eq!(p["role"], "master");
        assert!(p["surface_ref"].as_str().is_some());
        assert!(p["idle_secs"].as_u64().is_some(), "top-level idle_secs — HUD 연속성");
        for key in ["pid", "seat_state", "agent_meta", "agent_alive", "status_age_secs"] {
            assert!(
                p["inputs"].as_object().unwrap().contains_key(key),
                "inputs.{key} 부재"
            );
        }
        assert_eq!(p["thresholds"]["confirm_ticks"], 3);
        assert_eq!(p["thresholds"]["tick_secs"], 5);
        assert_eq!(p["thresholds"]["grace_secs"], 0);
        assert_eq!(p["thresholds"]["debounce_secs"], 300);
        assert_eq!(p["misses"], 3);
        assert!(
            p["last_ok_epoch"].as_f64().unwrap() > 0.0,
            "생존 관측이 있었으므로 last_ok_epoch 실측값"
        );
    }

    /// [핀7·의도적 변경 박제] CYS_MASTER_DEADMAN_SECS=0 은 idle 신호만 끈다 —
    /// 사망 판정(fail-closed)은 상시 유지(v1 전체 비활성에서 축소된 유일한 행동 변경).
    #[test]
    fn deadman_secs_zero_disables_idle_only() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[
            ("CYS_MASTER_DEADMAN_SECS", "0"),
            ("CYS_ROLE_DEADMAN_GRACE_SECS", "0"),
        ]);
        let (daemon, sid) = deadman_daemon("secs-zero");
        let s = daemon.get_surface(sid).unwrap();
        let mut tracker = DeadmanTracker::default();
        for _ in 0..4 {
            backdate_output(&s, 1000);
            check_role_deadman(&daemon, &mut tracker);
        }
        assert_eq!(events_named(&daemon, "master.idle").len(), 0, "0 = idle 비활성");
        assert_eq!(events_named(&daemon, "master.deadman").len(), 0);
        s.exited.store(true, AtomicOrdering::Relaxed);
        for _ in 0..3 {
            check_role_deadman(&daemon, &mut tracker);
        }
        assert_eq!(
            events_named(&daemon, "master.deadman").len(),
            1,
            "0 이어도 진짜 사망(surface exited)은 발화해야(fail-closed)"
        );
    }

    /// [핀8] 발화 후 디바운스(기본 300s) 내 동일 role 재발화 억제, 경과 후 재발화.
    #[test]
    fn deadman_debounce() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let (daemon, sid) = deadman_daemon("debounce");
        let s = daemon.get_surface(sid).unwrap();
        s.exited.store(true, AtomicOrdering::Relaxed);
        let mut tracker = DeadmanTracker::default();
        for _ in 0..5 {
            check_role_deadman(&daemon, &mut tracker);
        }
        assert_eq!(events_named(&daemon, "master.deadman").len(), 1, "디바운스 내 재발화 억제");
        // 디바운스 경과 시뮬레이션 → 재발화(계속 죽어있는 role 의 재상기)
        tracker
            .last_death_alert
            .insert("master".into(), now_epoch() - 301.0);
        check_role_deadman(&daemon, &mut tracker);
        let deaths = events_named(&daemon, "master.deadman");
        assert_eq!(deaths.len(), 2, "디바운스 경과 후 재발화");
        assert_eq!(deaths[1]["payload"]["misses"], 6, "연속 miss 누적 관측치 보존");
    }

    /// [핀10] grace 창(부트·승계 직후) 내에는 DeadCandidate 도 무카운트 — 오살 방지.
    #[test]
    fn deadman_grace_window() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "3600")]);
        let (daemon, sid) = deadman_daemon("grace");
        let s = daemon.get_surface(sid).unwrap();
        s.exited.store(true, AtomicOrdering::Relaxed);
        let mut tracker = DeadmanTracker::default();
        for _ in 0..3 {
            check_role_deadman(&daemon, &mut tracker);
        }
        assert_eq!(events_named(&daemon, "master.deadman").len(), 0, "grace 내 발화 금지");
        assert_eq!(tracker.misses.get("master"), None, "grace 내 카운트 자체 금지");
    }

    /// [핀14·BLOCK 교정] 비에이전트 자손(vim/less/tail/빌드)의 원시 Occupied 관측은 meta 부재
    /// 보조축을 arm 하지 못한다 — 무meta 수동 claim master 좌석에서 vim 1틱 → 프롬프트 복귀가
    /// 살아있는 master 를 사망으로 오라벨하는 결함 8 동형 신규 레인의 봉쇄 핀.
    #[test]
    fn deadman_bare_shell_descendant_does_not_arm_vacant_axis() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let (daemon, sid) = deadman_daemon("bare-occupied");
        let s = daemon.get_surface(sid).unwrap();
        let mut tracker = DeadmanTracker::default();
        // vim 등 비에이전트 자손 틱: seat=Occupied 인데 기지 에이전트 엄격 관측은 없음
        // (refresh_seat_cache 대역 — seat_agent_cache=false 유지).
        s.seat_cache.store(SeatState::Occupied.as_u8(), AtomicOrdering::Relaxed);
        check_role_deadman(&daemon, &mut tracker);
        assert!(
            tracker.seat_agent_seen.get("master").is_none(),
            "원시 Occupied 가 arm 하면 안 됨(armed 경계 = 기지 에이전트 엄격 관측)"
        );
        // 프롬프트 복귀(Empty) 지속 — 어떤 틱 수에도 death 후보·발화 0 이어야 한다.
        s.seat_cache.store(SeatState::Empty.as_u8(), AtomicOrdering::Relaxed);
        for _ in 0..5 {
            check_role_deadman(&daemon, &mut tracker);
        }
        assert_eq!(
            events_named(&daemon, "master.deadman").len(),
            0,
            "vim 좌석의 프롬프트 복귀가 사망 오라벨 — 결함 8 동형 재발"
        );
        assert_eq!(tracker.misses.get("master"), None, "미armed 좌석은 카운트 자체 금지");
        // 대조군: 같은 좌석에서 기지 에이전트 엄격 관측이 잡히면 arm → Empty 지속은 정당 발화.
        s.seat_cache.store(SeatState::Occupied.as_u8(), AtomicOrdering::Relaxed);
        s.seat_agent_cache.store(true, AtomicOrdering::Relaxed);
        check_role_deadman(&daemon, &mut tracker);
        s.seat_agent_cache.store(false, AtomicOrdering::Relaxed);
        s.seat_cache.store(SeatState::Empty.as_u8(), AtomicOrdering::Relaxed);
        for _ in 0..3 {
            check_role_deadman(&daemon, &mut tracker);
        }
        let deaths = events_named(&daemon, "master.deadman");
        assert_eq!(deaths.len(), 1, "엄격 관측 armed 좌석의 빈사는 종전대로 발화(보조축 목적 보존)");
        assert_eq!(deaths[0]["payload"]["axis"], "seat_vacant_no_meta");
    }

    /// [핀15·MAJOR 쌍둥이 셀] 기동 즉사: meta 있음(set_meta 가 agent_seen=false 리셋) ∧
    /// 에이전트가 첫 sysinfo 관측 전 사망(agent_seen 영구 false → check_agent_death 영구 skip)
    /// ∧ seat=Empty 지속 — death 채널이 영구 부재하던 미탐 셀의 봉쇄 핀(v1 은 900s 후
    /// "master silent"로나마 울렸다).
    #[test]
    fn deadman_agent_never_started_axis() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let (daemon, sid) = deadman_daemon("never-started");
        let s = daemon.get_surface(sid).unwrap();
        // set_meta 직후 상태 재현: meta 등록 + agent_seen/exit_notified=false 리셋
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/bin/claude".into()));
        s.agent_seen.store(false, AtomicOrdering::Relaxed);
        s.agent_exit_notified.store(false, AtomicOrdering::Relaxed);
        s.seat_cache.store(SeatState::Empty.as_u8(), AtomicOrdering::Relaxed);
        let mut tracker = DeadmanTracker::default();
        check_role_deadman(&daemon, &mut tracker);
        // never-seen 상태 추적이 무장됐는가(쌍둥이 셀 전용 grace 앵커)
        assert_eq!(
            tracker.never_seen_since.get("master").map(|(id, _)| *id),
            Some(sid),
            "meta ∧ 미관측 상태의 최초 관측이 기록돼야(grace 앵커)"
        );
        check_role_deadman(&daemon, &mut tracker);
        assert_eq!(events_named(&daemon, "master.deadman").len(), 0, "확증 미소진(2/3) 발화 금지");
        check_role_deadman(&daemon, &mut tracker);
        let deaths = events_named(&daemon, "master.deadman");
        assert_eq!(deaths.len(), 1, "기동 즉사도 confirm 소진 시 발화해야(미탐 셀 봉쇄)");
        let p = &deaths[0]["payload"];
        assert_eq!(p["axis"], "agent_never_started");
        assert_eq!(p["reason"], "agent never started (seat empty)");
        assert_eq!(p["inputs"]["agent_meta"], "claude");
        assert!(p["inputs"]["agent_alive"].is_null(), "관측 이전 = 미측정(null) — 측정 불능 ≠ 사망");
        // 기동 성공(check_agent_death 가 seen 확정) → 상태 추적 해제·카운터 리셋
        s.agent_seen.store(true, AtomicOrdering::Relaxed);
        check_role_deadman(&daemon, &mut tracker);
        assert!(tracker.never_seen_since.get("master").is_none(), "기동 확정 = 추적 해제");
        assert_eq!(tracker.misses.get("master"), None, "기동 확정 = misses 리셋");
    }

    /// [핀16·쌍둥이 셀 소켓 반증] 기동 즉사 후보도 confirm 창 내 status.set 자기보고가 있으면
    /// 생존으로 반증된다 — sysinfo 매칭이 못 보는 에이전트의 오살 방지(fail-safe).
    #[test]
    fn deadman_agent_never_started_socket_rebuts() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let (daemon, sid) = deadman_daemon("never-started-rebut");
        let s = daemon.get_surface(sid).unwrap();
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/bin/claude".into()));
        s.agent_seen.store(false, AtomicOrdering::Relaxed);
        s.seat_cache.store(SeatState::Empty.as_u8(), AtomicOrdering::Relaxed);
        *s.agent_status.lock().unwrap() = Some(crate::state::AgentStatus {
            state: "working".into(),
            context_pct: None,
            task: None,
            updated_at: now_epoch(),
        });
        let mut tracker = DeadmanTracker::default();
        for _ in 0..5 {
            check_role_deadman(&daemon, &mut tracker);
        }
        assert_eq!(events_named(&daemon, "master.deadman").len(), 0, "신선한 소켓 활동 = 생존");
        assert_eq!(tracker.misses.get("master"), None, "반증 시 misses 리셋");
    }

    /// [핀17·MINOR] idle 디바운스는 전용 노브(CYS_ROLE_DEADMAN_IDLE_DEBOUNCE_SECS)로 death
    /// 디바운스와 분리된다 — 기본은 death 노브 체이닝(현행 불변), 설정 시 독립.
    #[test]
    fn deadman_idle_debounce_knob_separate() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[
            ("CYS_MASTER_DEADMAN_SECS", "20"),
            ("CYS_ROLE_DEADMAN_GRACE_SECS", "0"),
            ("CYS_ROLE_DEADMAN_IDLE_DEBOUNCE_SECS", "0"),
        ]);
        let (daemon, sid) = deadman_daemon("idle-knob");
        let s = daemon.get_surface(sid).unwrap();
        let mut tracker = DeadmanTracker::default();
        for _ in 0..3 {
            backdate_output(&s, 21);
            check_role_deadman(&daemon, &mut tracker);
        }
        let idles = events_named(&daemon, "master.idle");
        assert_eq!(idles.len(), 3, "idle 디바운스 0 = 매 틱 발화(death 노브와 독립)");
        assert_eq!(idles[0]["payload"]["debounce_secs"], 0, "payload 는 idle 전용 노브 값");
        assert_eq!(events_named(&daemon, "master.deadman").len(), 0);
    }

    /// [핀11·소켓 반증] 에이전트 계열 축은 confirm 창 내 status.set 자기보고(소켓 활동)로
    /// 반증된다 — 활동=생존 증거, 부재는 증거 아님(fail-safe 방향).
    #[test]
    fn deadman_socket_activity_rebuts_agent_death() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let (daemon, sid) = deadman_daemon("socket-rebut");
        let s = daemon.get_surface(sid).unwrap();
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/bin/claude".into()));
        s.agent_seen.store(true, AtomicOrdering::Relaxed);
        s.agent_exit_notified.store(true, AtomicOrdering::Relaxed);
        let fresh = crate::state::AgentStatus {
            state: "working".into(),
            context_pct: None,
            task: None,
            updated_at: now_epoch(),
        };
        *s.agent_status.lock().unwrap() = Some(fresh.clone());
        let mut tracker = DeadmanTracker::default();
        for _ in 0..5 {
            check_role_deadman(&daemon, &mut tracker);
        }
        assert_eq!(events_named(&daemon, "master.deadman").len(), 0, "신선한 소켓 활동 = 생존");
        assert_eq!(tracker.misses.get("master"), None, "반증 시 misses 리셋");
        // 소켓 활동이 낡으면(confirm 창 밖) 반증 소멸 → 정상 확증 경로
        *s.agent_status.lock().unwrap() = Some(crate::state::AgentStatus {
            updated_at: now_epoch() - 1000.0,
            ..fresh
        });
        for _ in 0..3 {
            check_role_deadman(&daemon, &mut tracker);
        }
        let deaths = events_named(&daemon, "master.deadman");
        assert_eq!(deaths.len(), 1);
        assert!(
            deaths[0]["payload"]["inputs"]["status_age_secs"].as_f64().unwrap() >= 999.0,
            "발화 payload 에 소켓 나이 실측 기록"
        );
    }

    /// [핀12] surface 소멸(roles 매핑만 잔존) — axis=surface_gone·reason 구값 보존.
    #[test]
    fn deadman_surface_gone_axis() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[("CYS_ROLE_DEADMAN_GRACE_SECS", "0")]);
        let daemon = drill_daemon("gone");
        daemon.roles.lock().unwrap().insert("master".into(), 424_242);
        let mut tracker = DeadmanTracker::default();
        for _ in 0..3 {
            check_role_deadman(&daemon, &mut tracker);
        }
        let deaths = events_named(&daemon, "master.deadman");
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0]["payload"]["axis"], "surface_gone");
        assert_eq!(deaths[0]["payload"]["reason"], "master surface gone", "reason 구값 보존");
        assert!(deaths[0]["payload"]["inputs"]["pid"].is_null(), "소멸 surface 입력은 null");
    }

    /// [핀13·요구 f] CYS_ROLE_DEADMAN_ROLES CSV 로 role 일반화 — worker 좌석도 동일 절차.
    #[test]
    fn deadman_role_generalization_csv() {
        let _g = REAP_ENV_LOCK.lock().unwrap();
        let _env = ReapEnvGuard::set(&[
            ("CYS_ROLE_DEADMAN_ROLES", "master, worker"),
            ("CYS_ROLE_DEADMAN_GRACE_SECS", "0"),
        ]);
        let daemon = drill_daemon("roles-csv");
        let wid = spawn_role_surface(&daemon, "worker");
        let w = daemon.get_surface(wid).unwrap();
        w.exited.store(true, AtomicOrdering::Relaxed);
        let mut tracker = DeadmanTracker::default();
        for _ in 0..3 {
            check_role_deadman(&daemon, &mut tracker);
        }
        let deaths = events_named(&daemon, "master.deadman");
        assert_eq!(deaths.len(), 1, "worker 좌석 사망도 동일 이벤트명으로 발화(role 키는 payload)");
        assert_eq!(deaths[0]["payload"]["role"], "worker");
        assert_eq!(deaths[0]["payload"]["reason"], "worker surface exited");
    }

    // ═════════════════════════════════════════════════════════════════════════
    // ★(U-16) 데몬 첫기동 관문 스캔 — 분리·생애 창·자기발화·캐시 무효화
    // ═════════════════════════════════════════════════════════════════════════
    //
    // ## 계측 타당성 규율 (이 절의 모든 검체)
    //
    // 이 단위의 핀 대부분은 **새 심볼**을 겨눈다 — 수리 전 트리에는 그 심볼이 없으므로
    // "되돌려서 적색을 본다" 를 그대로 할 수 없다. 그래서 검체마다 **양성 대조군**을 함께
    // 넣는다: 같은 검체 안에서 (ⓐ 탐지되어야 하는 합성 표본이 실제로 탐지되고) (ⓑ 탐지되면
    // 안 되는 표본이 탐지되지 않는지)를 **둘 다** 단언한다. ⓐ가 없으면 탐지기가 통째로 고장나도
    // 초록이 되고, 그 초록은 아무것도 증명하지 않는다.

    use super::{
        gate_envelope, gate_escalation_text, gate_scan_enabled_from, gate_scan_enabled_with,
        gate_scan_observe_open, gate_scan_open, gate_sighting, pending_gate_items,
        scan_first_run_gate, GateScanCtx, GateScanWindow, GateSeatState, GateSighting, ScanCaches,
        GATE_FEED_KIND, GATE_SCAN_MAX_ESCALATIONS, GATE_SCAN_WINDOW_SECS,
    };
    use cys::first_run_gates as frg;

    /// 창이 **열린** 기준 상태(각 검체가 축 하나씩만 흔든다).
    fn open_window() -> GateScanWindow {
        GateScanWindow {
            enabled: true,
            declared: true,
            awakened: false,
            age_secs: 1.0,
            window_secs: GATE_SCAN_WINDOW_SECS,
            escalations: 0,
            max_escalations: GATE_SCAN_MAX_ESCALATIONS,
        }
    }

    /// ★생애 창 진리표. 닫힘의 귀결은 언제나 '아무 것도 발행하지 않음' = 종전 동작이므로
    /// 이 술어는 **오살 방향으로 열리지 않는다** — 판정 불능(NaN·무한)도 닫는다.
    #[test]
    fn gate_scan_window_truth_table() {
        assert!(gate_scan_open(&open_window()), "기준 상태는 열려 있어야 한다");

        let closed_by: [(&str, GateScanWindow); 7] = [
            (
                "롤백 스위치",
                GateScanWindow {
                    enabled: false,
                    ..open_window()
                },
            ),
            (
                "봉투 미선언 어댑터(코퍼스는 claude 실측이다)",
                GateScanWindow {
                    declared: false,
                    ..open_window()
                },
            ),
            (
                "첫 각성 ack 이후",
                GateScanWindow {
                    awakened: true,
                    ..open_window()
                },
            ),
            (
                "창 상한 초과",
                GateScanWindow {
                    age_secs: GATE_SCAN_WINDOW_SECS + 0.001,
                    ..open_window()
                },
            ),
            (
                "격상 천장",
                GateScanWindow {
                    escalations: GATE_SCAN_MAX_ESCALATIONS,
                    ..open_window()
                },
            ),
            (
                "나이 NaN(시계 사고)",
                GateScanWindow {
                    age_secs: f64::NAN,
                    ..open_window()
                },
            ),
            (
                "나이 무한",
                GateScanWindow {
                    age_secs: f64::INFINITY,
                    ..open_window()
                },
            ),
        ];
        for (why, w) in closed_by {
            assert!(!gate_scan_open(&w), "{why}: 창이 닫혀야 한다");
        }
        // 경계는 포함(<=): 상한 그 순간까지는 관측한다.
        assert!(gate_scan_open(&GateScanWindow {
            age_secs: GATE_SCAN_WINDOW_SECS,
            ..open_window()
        }));
        // 시계 되돌림으로 나이가 음수여도 창 안이다(창은 생애 **앞쪽**에 있다).
        assert!(gate_scan_open(&GateScanWindow {
            age_secs: -5.0,
            ..open_window()
        }));
        // 롤백 스위치의 순수 절반 — 형제 게이트와 같은 엄격 비교.
        assert!(gate_scan_enabled_from(None), "미설정 = 켜짐");
        assert!(!gate_scan_enabled_from(Some("0")), "0 = 꺼짐");
        for loose in ["off", "false", "no", "", "1", "00"] {
            assert!(
                gate_scan_enabled_from(Some(loose)),
                "{loose:?}: 느슨한 falsy 로 안전장치가 조용히 꺼지면 안 된다"
            );
        }
        // ★마스터 스위치 접기 — 사고 순간에 사람이 쥐는 손잡이는 하나여야 한다(BLOCK-3).
        //   자기 노브가 켜져 있어도 상위 접기값(`CYS_BOOT_GATES=0` ∨ 보류 장치 꺼짐)이 참이면
        //   스캐너는 꺼진다. 이 축의 '종전' 은 **스캔 없음**이므로 꺼짐은 종전 동작이다.
        assert!(gate_scan_enabled_with(None, false), "기본 = 켜짐");
        assert!(
            !gate_scan_enabled_with(None, true),
            "마스터 스위치를 눌렀는데 데몬 스캐너가 계속 돈다 — 반쪽 롤백(BLOCK-3 의 형태)"
        );
        assert!(!gate_scan_enabled_with(Some("0"), false), "자기 노브만으로도 꺼진다");
        assert!(!gate_scan_enabled_with(Some("0"), true));
    }

    /// ★N-2 오탐 회귀 — **작업 중 노드가 이 캠페인의 감사 문서·소스를 `cat` 해도 격상 0**.
    ///
    /// 합성 표본은 **실제 관문 문면(needle + 위젯 서명 전량)** 을 본문에 그대로 담는다.
    /// 그래서 이 검체는 두 가지를 동시에 증명한다:
    ///   ⓐ **탐지기는 살아 있다** — 창이 열려 있으면 이 화면은 관문으로 잡힌다(양성 대조군).
    ///       ⓐ가 없으면 `identify` 가 통째로 고장나도 ⓑ가 초록이라 아무것도 증명 못 한다.
    ///   ⓑ **그럼에도 격상은 0** — 각성 ack 를 받은 좌석은 창이 닫혀 스캔 자체를 하지 않는다.
    /// 즉 오탐을 막는 것은 '문면이 안 걸려서' 가 아니라 **생애 창**이다. 이 구분이 요점이다.
    #[test]
    fn audit_doc_cat_is_detectable_but_never_escalates_after_awakening() {
        let gates = frg::builtin();
        let audit_screen = format!(
            "worker@mac cys-terminal-rel % sed -n '40,60p' src/first_run_gates.rs\n\
             //! 관문 6종의 실제 순서: 테마 → 로그인방식 → OAuth → 폴더신뢰 → 면책 → 새기능안내.\n\
             {}\n\
             worker@mac cys-terminal-rel % \n",
            frg::fixtures::FOLDER_TRUST
        );

        // ⓐ 양성 대조군 — 탐지기 자체는 이 화면을 관문으로 본다.
        let seen = gate_sighting(&gates, &audit_screen);
        assert_eq!(
            seen.as_ref().map(|h| h.id.as_str()),
            Some("folder-trust"),
            "탐지기가 고장났다 — 이 합성 표본은 관문 문면 전량을 담고 있다"
        );

        // ⓑ 그럼에도 각성한 좌석에서는 스캔이 열리지 않는다 = 격상 0.
        assert!(
            !gate_scan_open(&GateScanWindow {
                awakened: true,
                ..open_window()
            }),
            "각성 ack 이후에 창이 열리면 감사 문서 cat 이 master 를 반복 각성시킨다"
        );
        // 나이만으로도 닫힌다(각성 래치가 없는 좌석의 두 번째 방어선).
        assert!(!gate_scan_open(&GateScanWindow {
            age_secs: GATE_SCAN_WINDOW_SECS + 1.0,
            ..open_window()
        }));
    }

    /// 코퍼스 자기규칙 소비 확인 — 관문이 **아닌** 화면 전량에서 목격 0.
    /// (규칙 자체는 `first_run_gates` 가 소유한다. 여기서 재확인하는 것은 데몬 스캐너가
    ///  느슨한 술어로 갈아타지 않았다는 사실이다 — needle 단독 매칭이면 아래가 적색이 된다.)
    #[test]
    fn gate_sighting_is_silent_on_non_gate_screens() {
        let gates = frg::builtin();
        for (id, screen) in frg::fixtures::NON_GATE_SCREENS {
            assert_eq!(
                gate_sighting(&gates, screen),
                None,
                "{id}: 관문이 아닌 화면을 관문으로 격상하면 작업 중 노드가 폭주한다"
            );
        }
        // ★위젯 서명 AND 가 **실제로 일하고 있는지**를 재는 합성 표본.
        //
        //   위 `NON_GATE_SCREENS` 만으로는 이 검체가 아무것도 증명하지 못한다 — 코퍼스의
        //   needle 은 이미 그 표본들에 단독으로도 걸리지 않도록 자기규칙이 강제하므로,
        //   데몬 스캐너가 느슨한 needle 단독 술어로 갈아타도 초록이 유지된다(실측: 그 변이를
        //   넣고 돌렸더니 통과했다). 그래서 **needle 은 있고 위젯 서명은 없는** 화면을 따로
        //   짓는다. 이 표본에서 목격이 나오면 AND 가 우회된 것이다.
        let needle_without_widget = "[boot] 좌석 진단 로그\n\
             관문 대기 여부 확인: Quick safety check: Is this a project you created or one you trust\n\
             user@mac cys-terminal-rel % \n";
        assert!(
            frg::builtin()
                .iter()
                .any(|g| cys::inject_guard::needle_hit(g, needle_without_widget)),
            "표본이 needle 을 잃었다 — 이 표본은 needle 을 담고 위젯만 없어야 의미가 있다"
        );
        assert_eq!(
            gate_sighting(&gates, needle_without_widget),
            None,
            "위젯 서명 AND 가 우회됐다 — needle 하나로 관문이 성립하면 로그 한 줄이 격상을 부른다"
        );

        // 양성 대조군 — 진짜 관문 화면은 전부 잡힌다(탐지기 생존 증명).
        for (id, screen) in [
            ("theme", frg::fixtures::THEME),
            ("login-method", frg::fixtures::LOGIN_METHOD),
            ("oauth-code", frg::fixtures::OAUTH_CODE),
            ("folder-trust", frg::fixtures::FOLDER_TRUST),
            ("bypass-disclaimer", frg::fixtures::TRUST_ECHO_THEN_DISCLAIMER),
            ("feature-announce-fullscreen", frg::fixtures::FEATURE_FULLSCREEN),
        ] {
            assert_eq!(
                gate_sighting(&gates, screen).map(|h| h.id),
                Some(id.to_string()),
                "{id}: 진짜 관문을 못 잡으면 위 침묵은 아무것도 증명하지 않는다"
            );
        }
    }

    /// ★자기발화 차단 — 격상 문안이 **그 자신을 관문으로 만들지 않는다**.
    ///
    /// 문안은 master pane 에 렌더된다. 거기에 needle 이 실리면 스캔·주입 가드가 자기 자신을
    /// 잡아 좌석이 조용히 막힌다(U-18 이 인증 처방 문안에서 겪은 형태와 동형). 그래서
    /// **위젯 AND 를 요구하지 않는 느슨한 술어**(`needle_hit`)로 검사한다 — 실제 pane 에서는
    /// 문안 주변에 어떤 위젯 문자가 함께 그려질지 알 수 없기 때문이다.
    #[test]
    fn gate_escalation_text_is_not_itself_a_gate() {
        let gates = frg::builtin();
        for g in &gates {
            let hit = GateSighting {
                id: g.id.clone(),
                title: g.title.clone(),
                human_only: g.passability == frg::Passability::HumanOnly,
            };
            let text = gate_escalation_text("surface:7", &hit);
            assert!(
                gate_sighting(&gates, &text).is_none(),
                "{}: 격상 문안이 관문으로 재식별된다(자기발화)",
                g.id
            );
            for other in &gates {
                assert!(
                    !cys::inject_guard::needle_hit(other, &text),
                    "{}: 격상 문안에 {} 의 needle 이 실렸다 — pane 렌더 시 자기발화한다",
                    g.id,
                    other.id
                );
                for echo in &other.confirm_echo {
                    assert!(
                        !frg::flatten(&text).contains(&frg::flatten(echo)),
                        "{}: 격상 문안에 확인 에코가 실렸다(2026-07-29 킬체인의 형태)",
                        g.id
                    );
                }
            }
            // 문안은 **키 재료를 담지 않는다** — 데몬은 자동 응답하지 않는다.
            for forbidden in ["Return", "Enter to confirm", "아래키", "down"] {
                assert!(
                    !text.contains(forbidden),
                    "{}: 격상 문안이 키 조작을 지시한다({forbidden}) — 자동응답 금지 계약 위반",
                    g.id
                );
            }
        }
        // 양성 대조군 — 같은 술어가 진짜 관문 화면에서는 참이다(술어 생존 증명).
        let trust = gates.iter().find(|g| g.id == "folder-trust").unwrap();
        assert!(cys::inject_guard::needle_hit(
            trust,
            frg::fixtures::FOLDER_TRUST
        ));
    }

    /// 봉투 계층 — **디스크 우선 · 부재 시에만 vendor 임베드**(`fill_missing_fields` 동형).
    #[test]
    fn gate_envelope_prefers_disk_then_embed() {
        let key = frg::ADAPTER_KEY;
        let embed = serde_json::json!({ "claude": { key: {"source": "builtin", "gates": []} } });

        // ① 디스크에 키가 있으면 디스크가 이긴다.
        let disk = serde_json::json!({ "claude": { key: {"source": "replace", "gates": []} } });
        assert_eq!(
            gate_envelope(&disk, &embed, "claude")
                .and_then(|v| v.get("source"))
                .and_then(|v| v.as_str()),
            Some("replace"),
            "사용자 주권 — 디스크 선언이 vendor 를 이긴다"
        );
        // ② 명시 null 도 '디스크 선언' 이다(의도적으로 비움 → 코드 정본만).
        let disk_null = serde_json::json!({ "claude": { key: serde_json::Value::Null } });
        assert_eq!(
            gate_envelope(&disk_null, &embed, "claude"),
            Some(&serde_json::Value::Null),
            "명시 null 을 임베드로 덮으면 사용자 주권이 깨진다"
        );
        // ③ 키가 **아예 없을 때만** 임베드가 채운다(기존 설치 기계 도달 경로 K-1).
        let disk_missing = serde_json::json!({ "claude": { "cmd": "claude" } });
        assert_eq!(
            gate_envelope(&disk_missing, &embed, "claude")
                .and_then(|v| v.get("source"))
                .and_then(|v| v.as_str()),
            Some("builtin"),
            "키 부재 시 vendor 봉투가 도달해야 한다"
        );
        // ④ 어느 쪽에도 없으면 None = 코드 정본만.
        assert_eq!(gate_envelope(&disk_missing, &embed, "codex"), None);

        // ⑤ ★적용 대상 한정 — 봉투를 선언하지 않은 어댑터는 스캔 대상이 아니다.
        //    코드 정본 코퍼스는 claude 실측이므로(`MEASURED_ON` 도 claude 버전), 어댑터 구분
        //    없이 들이대면 남의 화면을 남의 관문으로 격상한다. 실제 출하 팩으로 재확인한다.
        let pack: serde_json::Value = serde_json::from_str(
            cys::pack::PACK_ALL
                .iter()
                .find(|(r, _)| *r == "agents.json")
                .map(|(_, c)| *c)
                .expect("임베드 agents.json"),
        )
        .expect("임베드 agents.json 파싱");
        let empty = serde_json::json!({});
        assert!(
            gate_envelope(&empty, &pack, "claude").is_some(),
            "claude 는 선언돼 있어야 한다 — 아니면 아래 침묵이 아무것도 증명하지 않는다"
        );
        for other in ["codex", "gemini", "grok"] {
            assert_eq!(
                gate_envelope(&empty, &pack, other),
                None,
                "{other}: 미선언 어댑터가 claude 실측 코퍼스의 적용 대상이 됐다"
            );
        }
    }

    /// ★캐시 무효화 규칙 — 키가 `resolve_with` 의 **입력 전량**이라 히트는 재해소와 등가다.
    /// 봉투가 바뀌면 즉시 재해소하고(= mtime 추정으로 놓치는 창이 없다), 관측되지 않은 키는
    /// pass 끝에서 솎인다.
    #[test]
    fn scan_caches_are_keyed_by_full_input_and_pruned() {
        let mut c = ScanCaches::default();

        // ── 정규식 선컴파일 캐시 ────────────────────────────────────────────
        assert!(c.approval_regex("Do you want to (proceed|allow)").is_some());
        assert!(
            c.approval_regex("(((").is_none(),
            "손상 패턴은 None — 종전과 같이 건너뛴다"
        );
        assert_eq!(c.approval_re.len(), 2, "실패도 캐시해 매 pass 재컴파일 안 한다");

        // ── 관문 코퍼스 캐시 ────────────────────────────────────────────────
        let base = c.corpus("claude", None, true);
        let again = c.corpus("claude", None, true);
        assert!(
            std::sync::Arc::ptr_eq(&base, &again),
            "같은 입력인데 재해소했다 — 틱마다 코퍼스를 다시 만들면 캐시가 무의미하다"
        );

        // 봉투가 바뀌면 재해소(다른 Arc). replace 봉투로 코퍼스를 통째 교체해 산출도 확인.
        let env = serde_json::json!({
            "source": "replace",
            "measured_on": frg::MEASURED_ON,
            "gates": [{
                "id": "synthetic-gate",
                "title": "합성 관문",
                "needles": ["Synthetic gate question line"],
                "widget": ["Synthetic option A", "Synthetic option B"],
                "passability": "human_only",
                "human_reason": "합성"
            }]
        });
        let replaced = c.corpus("claude", Some(&env), true);
        assert!(
            !std::sync::Arc::ptr_eq(&base, &replaced),
            "봉투가 바뀌었는데 캐시가 stale 을 돌려줬다"
        );
        // ★N1 정정 — replace 봉투 소비는 일어나되(양성 대조군) 킬체인 관문은 **강제 복원**된다.
        //   종전 이 자리의 `vec!["synthetic-gate"]` 는 "replace 한 줄이 Fatal 관문 5종을
        //   없애는 것이 정상"을 데몬 쪽에서도 박제하고 있었다.
        let fatal: Vec<String> = frg::builtin()
            .into_iter()
            .filter(|g| g.absence_is_fatal())
            .map(|g| g.id)
            .collect();
        assert_eq!(
            replaced.first().map(|g| g.id.as_str()),
            Some("synthetic-gate"),
            "봉투 소비가 실제로 일어나야 한다(양성 대조군): {:?}",
            replaced.iter().map(|g| g.id.as_str()).collect::<Vec<_>>()
        );
        assert!(
            !replaced.iter().any(|g| g.id == "theme"),
            "가역 관문까지 되살아났다 = 코드 정본 폴백(사용자 주권 침해)의 형태다"
        );
        for id in &fatal {
            assert!(
                replaced.iter().any(|g| &g.id == id),
                "Fatal 관문 {id} 이 데몬 캐시 경로에서 사라졌다"
            );
        }
        assert_eq!(replaced.len(), 1 + fatal.len());
        // 롤백 스위치(override off)가 바뀌면 역시 재해소 — 코드 정본으로 되돌아온다.
        let off = c.corpus("claude", Some(&env), false);
        assert!(
            off.iter().any(|g| g.id == "folder-trust"),
            "override 를 끄면 코드 정본이어야 한다"
        );
        assert!(!std::sync::Arc::ptr_eq(&replaced, &off));

        // ── pass 끝 정리 ────────────────────────────────────────────────────
        c.gate_debounce.insert((7, "folder-trust".into()), 1.0);
        c.gate_escalations.insert(7, GateSeatState::default());
        c.prune_pass(
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert!(c.approval_re.is_empty() && c.gate_corpus.is_empty(), "미관측 키 누수");
        assert_eq!(c.gate_debounce.len(), 1, "좌석 키는 pass 정리 소관이 아니다");

        // ── 좌석 정리(watchdog 누수 차단 블록) ──────────────────────────────
        c.prune_surfaces(&std::collections::HashSet::new());
        assert!(c.gate_debounce.is_empty() && c.gate_escalations.is_empty());
    }

    /// ★N9 — 데몬 경로에서 코퍼스 자기규칙 **수리가 침묵하지 않는다**(그러나 **유계**다).
    ///
    /// 【무엇이 틀렸었는가】 데몬의 캐시 진입점은 `resolve_with(...).gates` 만 취하고
    /// `.notes` 를 통째로 버렸다. `repair_gate` 는 **사용자가 선언한 값을 시스템이 바꾸는
    /// 행위**이고(그리고 N1 이후에는 Fatal 관문의 **강제 복원**까지 한다), 데몬에서 그 사실이
    /// 어디에도 남지 않으면 사용자는 자기 override 가 왜 안 먹는지 알 방법이 없다.
    /// CLI 는 같은 정보를 `cys.rs::resolve_gate_corpus` 가 stderr 로 내고 있었다 — 두 경로가
    /// 갈려 있던 것이다.
    ///
    /// 【그러나 반복 발행은 재난 ① 방향】 이 캐시는 watchdog 틱마다 조회된다. 히트에서도
    /// 찍으면 24/365 데몬 로그가 같은 줄로 덮여 진짜 신호가 묻힌다. 그래서 **캐시 미스 1회**
    /// 라는 유계성을 이 검체가 단언한다.
    #[test]
    fn daemon_gate_corpus_notes_are_published_once_per_resolution() {
        // ⓐ 계측 타당성 — 이 봉투가 **실제로** 사유를 만든다(사유가 0이면 아무것도 못 잰다).
        let env = serde_json::json!({
            "source": "replace",
            "gates": [{"id": "synthetic-gate", "needles": ["Synthetic gate question line?"]}]
        });
        let resolved = frg::resolve_with(Some(&env), true);
        assert!(
            !resolved.notes.is_empty(),
            "이 봉투가 사유를 만들지 않는다면 이 검체는 아무것도 재지 못한다(계측 무효)"
        );

        // ⓑ ★배선 핀 — 데몬이 그 사유를 버리지 않는다.
        let src = include_str!("governance.rs");
        let prod = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let at = prod.find("    fn corpus(").expect("코퍼스 캐시 진입점 소실");
        let body = &prod[at..];
        let end = body.find("\n    }\n").expect("코퍼스 캐시 본문 끝을 못 찾았다");
        assert!(
            !body[..end].contains("resolve_with(envelope, override_on).gates"),
            "데몬이 해소 사유(.notes)를 그 자리에서 통째로 버린다 — 자기규칙 수리·Fatal 복원이 \
             사용자에게 보이지 않는다"
        );
        assert!(
            body[..end].contains("for n in &fresh_notes"),
            "사유를 보관만 하고 아무 데도 내지 않는다 — 사용자에겐 그대로 침묵이다"
        );

        // ⓒ ★유계성 — 발행 재료는 **캐시 미스에서만** 나온다. 히트에서도 나오면 watchdog 틱
        //    마다 같은 줄이 반복돼 로그가 판정을 덮는다(재난 ① 방향).
        let mut c = ScanCaches::default();
        let (_, first) = c.corpus_with_notes("claude", Some(&env), true);
        assert_eq!(
            first, resolved.notes,
            "데몬 경로의 사유가 해소본과 다르다(중간에 갈렸다)"
        );
        let (_, again) = c.corpus_with_notes("claude", Some(&env), true);
        assert!(
            again.is_empty(),
            "캐시 히트에서도 사유가 다시 나온다 — 24/365 데몬에서 같은 줄이 무한 반복된다: \
             {again:?}"
        );
        // 그러나 **버린 것은 아니다** — 캐시가 보관하고 있어 다음 소비자가 읽을 수 있다.
        assert_eq!(
            c.gate_corpus["claude"].notes, resolved.notes,
            "사유가 캐시에 보관되지 않았다(발행 1회 뒤 증발)"
        );
    }

    /// ★오염 차단 핀 — 관문 코퍼스가 `approval_patterns` 로 **흘러들지 않는다**.
    ///
    /// approval 키에는 자동응답 계약(CLI 폴더신뢰 자동확인)이 붙어 있다. 거기에 관문 문면이
    /// 섞이면 면책 창에 Return 이 가는 킬체인이 되살아난다(2026-07-29). 그래서 겹침은
    /// **문서화된 1건**(`trust-prompt` = folder-trust 의 구 문면 하위호환)만 허용한다.
    ///
    /// ★핀을 지우지 않고 이사시킨다: 면제는 상수로 선언하고, 그 면제가 **실재하지 않으면**
    /// 그것도 적색이다(면제표가 쓰레기통이 되는 것을 막는다 — `NEEDLE_EXEMPTIONS` 와 같은 규율).
    #[test]
    fn approval_patterns_union_excludes_first_run_gate_corpus() {
        /// (approval 패턴 이름, 겹치는 관문 id, 근거) — 이 표 밖의 겹침은 전부 적색.
        const ALLOWED_OVERLAP: &[(&str, &str, &str)] = &[(
            "trust-prompt",
            "folder-trust",
            "U-15 폴더신뢰 자동확인의 감지 문면. 이 1건만 자동응답 계약을 갖는다.",
        )];

        let embed: serde_json::Value = serde_json::from_str(
            cys::pack::PACK_ALL
                .iter()
                .find(|(r, _)| *r == "agents.json")
                .map(|(_, c)| *c)
                .expect("임베드 agents.json"),
        )
        .expect("임베드 agents.json 파싱");
        let disk = serde_json::json!({});
        let gates = frg::builtin();

        let violations = |patterns: &[serde_json::Value]| -> Vec<String> {
            let mut out = Vec::new();
            for p in patterns {
                let (Some(name), Some(pat)) = (p["name"].as_str(), p["pattern"].as_str()) else {
                    continue;
                };
                let flat = frg::flatten(pat);
                for g in &gates {
                    for n in &g.needles {
                        if !flat.contains(&frg::flatten(n)) {
                            continue;
                        }
                        if ALLOWED_OVERLAP
                            .iter()
                            .any(|(an, ag, _)| *an == name && *ag == g.id)
                        {
                            continue;
                        }
                        out.push(format!("approval_patterns[{name}] ⊃ gate[{}] needle", g.id));
                    }
                }
            }
            out
        };

        for agent in ["claude", "gemini", "codex", "grok"] {
            let merged = merged_approval_patterns(&disk, &embed, agent);
            assert!(
                violations(&merged).is_empty(),
                "{agent}: 관문 문면이 approval 키로 흘러들었다 — {:?}",
                violations(&merged)
            );
        }
        // 문서화된 겹침은 **실재해야** 한다(면제가 유령이면 표가 썩는다).
        let claude = merged_approval_patterns(&disk, &embed, "claude");
        for (name, gid, _) in ALLOWED_OVERLAP {
            let p = claude
                .iter()
                .find(|p| p["name"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("면제 {name} 이 approval_patterns 에 없다"));
            let flat = frg::flatten(p["pattern"].as_str().unwrap_or(""));
            let g = gates.iter().find(|g| g.id == *gid).expect("면제 관문 id");
            assert!(
                g.needles.iter().any(|n| flat.contains(&frg::flatten(n))),
                "면제 {name}↔{gid} 겹침이 사라졌다 — 면제를 지우거나 근거를 갱신하라"
            );
        }
        // ★양성 대조군 — 오염된 합성 팩은 반드시 적색이어야 한다(탐지기 생존 증명).
        let contaminated = serde_json::json!({ "claude": { "approval_patterns": [
            {"name": "gate-theme", "pattern": "Choose the text style that looks best with your terminal"}
        ]}});
        let merged = merged_approval_patterns(&contaminated, &embed, "claude");
        assert!(
            !violations(&merged).is_empty(),
            "탐지기가 고장났다 — 관문 needle 을 approval 키에 넣었는데도 초록이다"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ★결함 4 — 격상 천장은 **폭주 방지**이지 **신호 삭제**가 아니다
    // ─────────────────────────────────────────────────────────────────────────

    /// ★두 축의 분리 진리표: 천장은 **발행만** 닫고 **관측은 닫지 않는다**.
    ///
    /// 나머지 축(롤백·미선언·각성 ack·생애 창·시계 사고)은 관측까지 닫는다 — 그 좌석은
    /// 화면을 더 보지 않으므로 배지를 살려 두면 영구 pending 이 된다. 이 비대칭이 요점이다.
    #[test]
    fn escalation_ceiling_closes_publishing_but_not_observation() {
        let at_ceiling = GateScanWindow {
            escalations: GATE_SCAN_MAX_ESCALATIONS,
            ..open_window()
        };
        assert!(
            !gate_scan_open(&at_ceiling),
            "천장에 닿았는데 발행이 열려 있다 — 천장이 사문이면 master 각성 폭주가 돌아온다"
        );
        assert!(
            gate_scan_observe_open(&at_ceiling),
            "천장이 관측까지 닫았다 — 관문이 화면에 떠 있어도 배지가 종결돼 운영자의 \
             유일한 신호가 사라진다(결함 4)"
        );
        // 천장을 훌쩍 넘긴 값에서도 같다(카운터는 단조 증가라 넘어설 수 있다).
        assert!(gate_scan_observe_open(&GateScanWindow {
            escalations: GATE_SCAN_MAX_ESCALATIONS + 7,
            ..open_window()
        }));
        // 나머지 축은 **관측까지** 닫는다 — 그래야 배지 생명주기가 끝난다.
        let closes_observation: [(&str, GateScanWindow); 5] = [
            ("롤백 스위치", GateScanWindow { enabled: false, ..open_window() }),
            ("봉투 미선언 어댑터", GateScanWindow { declared: false, ..open_window() }),
            ("첫 각성 ack 이후", GateScanWindow { awakened: true, ..open_window() }),
            (
                "생애 창 초과",
                GateScanWindow { age_secs: GATE_SCAN_WINDOW_SECS + 0.001, ..open_window() },
            ),
            ("나이 NaN(시계 사고)", GateScanWindow { age_secs: f64::NAN, ..open_window() }),
        ];
        for (why, w) in closes_observation {
            assert!(!gate_scan_observe_open(&w), "{why}: 관측이 닫혀야 한다");
            assert!(!gate_scan_open(&w), "{why}: 발행도 함께 닫혀야 한다");
        }
        // 기준 상태는 둘 다 열려 있다(양성 대조군 — 술어가 통째로 false 면 위가 다 무의미).
        assert!(gate_scan_open(&open_window()) && gate_scan_observe_open(&open_window()));
    }

    /// ★천장 뒤의 스캐너 행동 — **배지는 유지 · 발행은 0 · 관문이 사라지면 종결**.
    ///
    /// 세 축을 한 검체에서 함께 본다. 하나라도 빠지면 수리가 다른 결함으로 바뀐다:
    ///   ⓐ 배지 유지만 하고 발행을 막지 않으면 → **천장 사문**(폭주 재개).
    ///   ⓑ 발행만 막고 배지를 종결하면 → 원래 결함 그대로.
    ///   ⓒ 관문이 사라져도 종결하지 않으면 → 영구 pending(배지 오염).
    #[test]
    fn ceiling_keeps_the_badge_and_publishes_nothing_until_the_gate_is_gone() {
        let (daemon, s) = force_deliver_rig("gate-ceiling", None);
        let sid = s.id;
        let gate_screen = frg::fixtures::FOLDER_TRUST;
        let at_ceiling = GateSeatState {
            count: GATE_SCAN_MAX_ESCALATIONS,
            cleaned: false,
        };
        let mut caches = ScanCaches::default();

        // ⓐ 천장 · 관문이 화면에 실재 · pending 배지 **없음** → 새 발행 0(천장은 천장이다).
        let before = daemon.bus.latest_seq();
        scan_first_run_gate(
            &GateScanCtx {
                daemon: &daemon,
                surface: &s,
                agent: "claude",
                screen: gate_screen,
                envelope: None,
                override_on: false,
                may_escalate: false,
                now: 1000.0,
            },
            at_ceiling,
            &mut caches,
        );
        assert!(
            pending_gate_items(&daemon, sid).is_empty(),
            "천장 뒤에 새 배지를 발행했다 — 천장이 사문이 되면 master 각성 폭주가 돌아온다"
        );
        assert_eq!(
            daemon.bus.latest_seq(),
            before,
            "천장 뒤에 이벤트를 발행했다(폭주 재개)"
        );

        // ⓑ 이미 떠 있는 배지는 **유지**된다 — 운영자의 유일한 신호다.
        daemon.push_feed_notification(GATE_FEED_KIND, "관문 감지", "body", Some(sid));
        scan_first_run_gate(
            &GateScanCtx {
                daemon: &daemon,
                surface: &s,
                agent: "claude",
                screen: gate_screen,
                envelope: None,
                override_on: false,
                may_escalate: false,
                now: 2000.0,
            },
            at_ceiling,
            &mut caches,
        );
        assert_eq!(
            pending_gate_items(&daemon, sid).len(),
            1,
            "관문이 화면에 떠 있는데 배지를 종결했다 — 운영자의 유일한 신호가 사라진다(결함 4)"
        );

        // ⓒ 관문이 화면에서 사라지면 같은 경로로 **종결**된다(생명주기는 계속 돈다).
        scan_first_run_gate(
            &GateScanCtx {
                daemon: &daemon,
                surface: &s,
                agent: "claude",
                screen: "worker@mac cys-terminal-rel % \n",
                envelope: None,
                override_on: false,
                may_escalate: false,
                now: 3000.0,
            },
            at_ceiling,
            &mut caches,
        );
        assert!(
            pending_gate_items(&daemon, sid).is_empty(),
            "관문이 사라졌는데 배지가 남았다 — 배지 영구 오염(수리가 다른 결함이 됐다)"
        );
    }

    /// ★배선 핀 — 배지 종결은 **관측** 술어가, 발행은 **천장** 술어가 지배한다.
    ///
    /// 순수 술어만 갈라 두고 호출부가 여전히 한 축을 쓰면 위 검체는 전부 무의미해진다.
    #[test]
    fn badge_lifecycle_is_wired_to_observation_not_to_the_ceiling() {
        let src = include_str!("governance.rs");
        let prod = &src[..src.find("#[cfg(test)]").expect("테스트 모듈 앵커 소실")];
        let at = prod.find("fn check_approvals").expect("check_approvals 소실");
        let body = &prod[at..];
        let close_at = body
            .find("gate-window-closed")
            .expect("창 닫힘 종결 사유 문자열 소실");
        let guard = body[..close_at]
            .rfind("if !observe_open && seat.count > 0")
            .expect("배지 종결이 관측 술어로 가드되지 않는다 — 천장이 다시 신호를 지운다");
        assert!(guard < close_at);
        assert!(
            body[..close_at].contains("let observe_open = gate_scan_observe_open(")
                && body[..close_at].contains("let may_escalate = gate_scan_open("),
            "두 축이 각자의 술어에서 오지 않는다(한 축으로 되돌아갔다)"
        );
        assert!(
            body.contains("may_escalate,"),
            "천장 값이 스캐너로 전달되지 않는다 — 천장 뒤에도 발행이 계속된다(폭주 재개)"
        );
        let scan_at = prod.find("fn scan_first_run_gate").expect("스캐너 본체 소실");
        let scan_body = &prod[scan_at..];
        let gone = scan_body.find("stale-cleared").expect("관문 소실 종결 경로 소실");
        let ceiling = scan_body
            .find("if !cx.may_escalate")
            .expect("스캐너가 천장을 집행하지 않는다");
        assert!(
            gone < ceiling,
            "천장 검사가 '관문이 사라졌다' 종결보다 앞이면, 천장 뒤 좌석의 배지가 영원히 남는다"
        );
    }

    /// feed 네임스페이스 분리 — 두 스캐너의 생명주기가 서로를 종결시키지 않는다.
    #[test]
    fn gate_feed_kind_is_isolated_from_approval_namespace() {
        let dir = std::env::temp_dir().join(format!("cys_gate_feed_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let daemon = std::sync::Arc::new(crate::state::Daemon::new(dir.join("cysd.sock")));

        daemon.push_feed_notification(GATE_FEED_KIND, "관문 감지 (surface:7)", "body", Some(7));
        assert_eq!(pending_gate_items(&daemon, 7).len(), 1, "관문 항목 pending");
        assert!(
            !daemon.has_pending_daemon_approval(7),
            "관문 항목이 approval 코얼레싱을 삼키면 승인 감지가 조용히 죽는다(워커 hang)"
        );
        assert!(
            daemon.pending_daemon_approvals(7).is_empty(),
            "approval stale-clear 가 관문 항목을 종결시키면 진단이 사라진다"
        );

        daemon.push_feed_notification("approval", "승인 대기 (surface:7)", "body", Some(7));
        assert!(daemon.has_pending_daemon_approval(7));
        assert_eq!(
            pending_gate_items(&daemon, 7).len(),
            1,
            "approval 항목이 관문 스냅샷에 섞이면 안 된다"
        );
        assert!(pending_gate_items(&daemon, 8).is_empty(), "타 surface 독립");

        let ids = pending_gate_items(&daemon, 7);
        assert!(daemon.resolve_feed_item(&ids[0], "stale-cleared").is_some());
        assert!(pending_gate_items(&daemon, 7).is_empty());
        assert!(
            daemon.has_pending_daemon_approval(7),
            "관문 항목 종결이 approval 항목까지 끌고 가면 안 된다"
        );

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
