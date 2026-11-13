//! T7 E6 경보 엔진 — 임계값·반복실패 경보의 순수 평가기 + 설정 로딩.
//! governance.rs watchdog가 에지 디바운스로 발화(능동 경보)하고, control.alerts RPC가 같은
//! 평가기(`evaluate`)로 현재 상태(UI 배지)를 노출한다 — **평가기가 하나**라 판정 규칙이 갈라지지 않는다.
//! ★(R2F-DM · 성찰 2회차 A1 n-12 ⓑ) 다만 **입력 스냅샷은 같지 않다**: 워치독은 캐시 전용 스냅샷(`snapshot_cached_with_stale` · IO 0 · 신원 캐시가 데워지기 전에는 '판정 불가 = 사용 중'),
//! `control.alerts` 는 읽기-통과 스냅샷(`snapshot` · 신원 파일을 읽는다)을 쓴다 — 캐시가 데워질 때까지 두 입력이 다를 수 있다(R1F-US M-1). "단일 진실원" 은 평가기에 대한 말이지 입력에 대한 말이 아니다.
//! ★자동응답 금지(governance 교리): 감지·격상(이벤트)만, cycle/clear/budget 판단은 master의 몫.
//! 데이터 소스: 노드 rate(observed_usage) + 7d usage_records(비용·토큰) + 7d events(반복실패).

use crate::state::Daemon;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};

/// governance.rs 의 동명 헬퍼와 **같은 규약**(미설정·파싱 실패=기본값). 그쪽은 private 이고
/// pub 승격은 governance 내부 노브 표면을 넓히므로, 재발명 대신 3줄 로컬 사본을 둔다.
fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// 경보 임계 설정 — alerts-config.json(pack)에서 로드, 누락/파손은 기본값(graceful degrade).
#[derive(Clone, Debug, PartialEq)]
pub struct AlertConfig {
    pub rate_limit_pct: f64,  // 노드 rate ≥ 이 %(5h/7d 쿼터) → 경보. 기본 90.
    pub weekly_cost_usd: f64, // 7d 비용 ≥ → 경보. 0=비활성(한도 미설정 시 오경보 방지). 기본 0.
    pub weekly_tokens: u64,   // 7d 토큰 ≥ → 경보. 0=비활성. 기본 0.
    pub fail_count: u64,      // 툴 실패수 ≥ → 경보. 기본 5.
    pub fail_rate: f64,       // 동시에 실패율 ≥. 기본 0.3.
    pub fail_min_calls: u64,  // 최소 호출수(소표본 노이즈 차단). 기본 5.
    // CC v2 WS-A: 계정 단위 rate 경보(노드 경보와 별개 축 — 계정이 진실 풀).
    pub account_warn_pct: f64, // 기본 80.
    pub account_crit_pct: f64, // 기본 95.
}

impl Default for AlertConfig {
    fn default() -> Self {
        AlertConfig {
            rate_limit_pct: 90.0,
            weekly_cost_usd: 0.0,
            weekly_tokens: 0,
            fail_count: 5,
            fail_rate: 0.3,
            fail_min_calls: 5,
            account_warn_pct: 80.0,
            account_crit_pct: 95.0,
        }
    }
}

impl AlertConfig {
    /// pack의 alerts-config.json 로드. 없거나 파싱 실패면 기본값.
    pub fn load() -> Self {
        let path = cys::pack::pack_dir().join("alerts-config.json");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .map(|v| Self::from_value(&v))
            .unwrap_or_default()
    }

    /// 부분 설정도 허용 — 빠진 키는 기본값(순수·테스트 핀).
    pub fn from_value(v: &Value) -> Self {
        let d = Self::default();
        let f = |k: &str, def: f64| v.get(k).and_then(|x| x.as_f64()).unwrap_or(def);
        let u = |k: &str, def: u64| v.get(k).and_then(|x| x.as_u64()).unwrap_or(def);
        AlertConfig {
            rate_limit_pct: f("rate_limit_pct", d.rate_limit_pct),
            weekly_cost_usd: f("weekly_cost_usd", d.weekly_cost_usd),
            weekly_tokens: u("weekly_tokens", d.weekly_tokens),
            fail_count: u("fail_count", d.fail_count),
            fail_rate: f("fail_rate", d.fail_rate),
            fail_min_calls: u("fail_min_calls", d.fail_min_calls),
            account_warn_pct: f("account_warn_pct", d.account_warn_pct),
            account_crit_pct: f("account_crit_pct", d.account_crit_pct),
        }
    }
}

/// ★T-0147-2 §3 Phase1 A8: 델타게이트(python)가 파일로 내놓는 **판정 배지** 1건.
/// 데몬은 판정하지 않는다 — 읽어서 기존 alerts 소비 경로(control.alerts RPC = Control Center
/// 배지)에 그대로 태울 뿐이다. 그래서 GUI 신규 작업이 0 이다(설계 '배지 전진 배치'의 요체).
#[derive(Clone, Debug, PartialEq)]
pub struct GateBadge {
    pub key: String,      // 레인 접두 포함 최종 키 (예: "report_gate/quiet")
    pub severity: String, // "info" | "warn" | "critical" (게이트 어휘 — Alert 로 갈 때 매핑)
    pub message: String,
    pub detail: Value,
}

/// ★0.14.43(B3): 경보 입력 한 건의 **관측 메타** — 경보 `detail`·요약에 실려 CSO 가 "이 값이 얼마나 오래된 것인가"를 읽는다.
/// 값 자체는 `Snapshot::rates`/`account_rates` 가 나르고, 이것은 같은 `(역할|라벨, 창)` 키의 곁가지다(없으면 detail 에 가산 키가 안 붙는다).
#[derive(Clone, Debug, PartialEq)]
pub struct RateMeta {
    /// 이 값이 새로 생산된 시각(epoch 초 · 0.0 = 모름).
    pub observed_at: f64,
    /// 관측 나이(초 · 음수 없음). `observed_at` 이 0.0 이면 의미 없다(detail 에는 null).
    pub age_secs: f64,
    /// 이 값이 지금 쓰이는 계정의 것인가 — true/false/None(판정 불가).
    pub in_use: Option<bool>,
    pub resets_at: Option<f64>,
    /// 리셋까지 남은 초(`resets_at − now` · 음수 가능). `resets_at` 이 epoch 초로 보이지 않거나 없으면 None.
    pub reset_in_secs: Option<f64>,
    /// 같은 리셋 창의 최댓값을 더 쥐는 시간(초 — `accounts::PEAK_HOLD_SECS` 잔여). 계정 축에서 `peak_at` 이 있을 때만.
    pub held_secs: Option<f64>,
}

impl RateMeta {
    /// 경보 `detail` 에 가산할 키 — 기존 detail 키는 건드리지 않는다. 시각·나이는 정수 초(나이는 내림), 모르면 null.
    fn add_to(&self, detail: &mut Value) {
        let known = self.observed_at.is_finite() && self.observed_at > 0.0;
        let secs = |x: f64| if x.is_finite() && x >= 0.0 { json!(x as u64) } else { Value::Null };
        detail["observed_at"] = if known { json!(self.observed_at) } else { Value::Null };
        detail["age_secs"] = if known { secs(self.age_secs) } else { Value::Null };
        detail["in_use"] = self.in_use.map_or(Value::Null, Value::Bool);
        detail["reset_in_secs"] = match self.reset_in_secs {
            Some(r) if r.is_finite() => json!(r as i64),
            _ => Value::Null,
        };
        detail["held_secs"] = self.held_secs.map_or(Value::Null, secs);
    }
}

/// 평가 입력 스냅샷 — 호출부(watchdog/RPC)가 락 잡고 수집(평가기는 락 무관·순수).
#[derive(Default)]
pub struct Snapshot {
    /// (role, label, used_pct) — ★R1F-US(m-6): 스냅샷(`snapshot_*`)이 만든 것은 같은 키 (역할, 창)당 **하나**다 — 값이 가장 큰 입력(같은 값이면 관측 나이가 작은 쪽). 값과 `rate_meta` 가 같은 입력의 것이어야 한다.
    pub rates: Vec<(String, String, f64)>,
    /// CC v2 WS-A: 계정 단위 rate — (계정 라벨, 창, used_pct). 관측된 계정만. ★R1F-US(m-6): 스냅샷이 만든 것은 같은 키 (라벨, 창)당 하나(값이 가장 큰 입력).
    pub account_rates: Vec<(String, String, f64)>,
    pub weekly_cost_usd: f64,
    pub weekly_tokens: u64,
    pub tool_failures: Vec<(String, u64, u64, f64)>, // (tool, calls, fail, fail_rate)
    /// ★T-0147-2 A8: 델타게이트 배지(파일 oracle) + 데몬 게이트 신호(state 외부 oracle).
    /// 가산 필드 — `..Snapshot::default()` 호출부는 빈 벡터를 받아 무회귀다.
    pub gate_badges: Vec<GateBadge>,
    /// ★0.14.43(B3) 가산 필드(Default 로 비어 무회귀): 계정 축 경보 입력의 관측 메타 — 키 (계정 라벨, 창).
    pub account_meta: HashMap<(String, String), RateMeta>,
    /// ★0.14.43(B3) 좌석 축 경보 입력의 관측 메타 — 키 (역할, 창 라벨).
    pub rate_meta: HashMap<(String, String), RateMeta>,
    /// ★0.14.43(B3) **신선도 규칙이 입력에서 뺀** 창의 경보 키(`account_rate:{라벨}:{창}` · `rate_limit:{역할}:{창}`) — 리셋 전인데도 '지금 쓰이지 않고 관측이
    /// 30분(노브)을 넘은' 값이다. 임계 판정은 `evaluate` 의 몫이라 임계 미만 창도 섞일 수 있다(발화한 적 없는 키는 틱의 `fired.retain` 이 어차피 버린다).
    /// 틱(`governance::check_alerts_with`)이 이 키를 `fired` 에 붙들어 둔다 — 다시 적격이 돼도 REMIND 안에는 재발행하지 않게(깜빡임 금지).
    /// ★R1F-US(m-6·b): 워치독 틱 판([`snapshot_cached_with_stale`])은 **임계 이상**인 창(경보 입력이었다면 `evaluate` 가 경보로 냈을 값)의 키만 싣는다 — 임계 미만의 낡은 창이 같은 키를 쓰는 다른 좌석·계정의
    /// 발화 키를 붙들어 새 교차가 30분 늦게 나가는 일이 없다. RPC 판([`snapshot`]·[`snapshot_with_stale`])은 이 필드를 쓰지 않으므로 임계로 좁히지 않는다(전부 싣는다).
    pub stale_suppressed: Vec<String>,
}

/// 단일 경보.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub kind: String,     // "rate_limit" | "weekly_budget" | "repeated_failure"
    pub key: String,      // 에지 디바운스 키 (예: "rate_limit:worker:5h")
    pub severity: String, // "warn" | "crit"
    pub message: String,
    pub detail: Value,
}

impl Alert {
    pub fn to_value(&self) -> Value {
        json!({"kind": self.kind, "key": self.key, "severity": self.severity,
               "message": self.message, "detail": self.detail})
    }
    /// "warn"|"crit" String 표면을 단일 술어 Severity로 파생(기존 String 필드 불변·외과적).
    pub fn severity_enum(&self) -> crate::severity::Severity {
        crate::severity::Severity::from(self.severity.as_str())
    }
}

/// 순수 평가 — 스냅샷+설정 → 현재 발화 중인 경보(key 오름차순 결정론 정렬).
pub fn evaluate(snap: &Snapshot, cfg: &AlertConfig) -> Vec<Alert> {
    let mut out: Vec<Alert> = Vec::new();
    // 1. rate limit (노드 5h/7d 쿼터)
    for (role, label, pct) in &snap.rates {
        if *pct >= cfg.rate_limit_pct {
            let mut detail = json!({"role": role, "label": label, "used_pct": pct});
            // ★0.14.43(B3): 관측 메타가 있으면 가산 키(observed_at·age_secs·in_use·reset_in_secs·held_secs) — 기존 detail 키는 불변.
            if let Some(m) = snap.rate_meta.get(&(role.clone(), label.clone())) {
                m.add_to(&mut detail);
            }
            out.push(Alert {
                kind: "rate_limit".into(),
                key: format!("rate_limit:{role}:{label}"),
                severity: if *pct >= 95.0 { "crit" } else { "warn" }.into(),
                message: format!("{role} {label} rate {:.0}%", pct),
                detail,
            });
        }
    }
    // 1b. CC v2: 계정 단위 rate (에지 디바운스 키 = 계정 라벨 — 같은 계정 다중 노드 중복발화 0)
    for (label, win, pct) in &snap.account_rates {
        if *pct >= cfg.account_warn_pct {
            let mut detail = json!({"account": label, "win": win, "used_pct": pct});
            if let Some(m) = snap.account_meta.get(&(label.clone(), win.clone())) {
                m.add_to(&mut detail);
            }
            out.push(Alert {
                kind: "account_rate".into(),
                key: format!("account_rate:{label}:{win}"),
                severity: if *pct >= cfg.account_crit_pct { "crit" } else { "warn" }.into(),
                message: format!("계정 {label} {win} rate {:.0}%", pct),
                detail,
            });
        }
    }
    // 2. 주간 예산 한도 (0=비활성)
    if cfg.weekly_cost_usd > 0.0 && snap.weekly_cost_usd >= cfg.weekly_cost_usd {
        out.push(Alert {
            kind: "weekly_budget".into(),
            key: "weekly_budget:cost".into(),
            severity: "warn".into(),
            message: format!("주간 비용 ${:.2} ≥ 한도 ${:.2}", snap.weekly_cost_usd, cfg.weekly_cost_usd),
            detail: json!({"cost_usd": snap.weekly_cost_usd, "limit": cfg.weekly_cost_usd}),
        });
    }
    if cfg.weekly_tokens > 0 && snap.weekly_tokens >= cfg.weekly_tokens {
        out.push(Alert {
            kind: "weekly_budget".into(),
            key: "weekly_budget:tokens".into(),
            severity: "warn".into(),
            message: format!("주간 토큰 {} ≥ 한도 {}", snap.weekly_tokens, cfg.weekly_tokens),
            detail: json!({"tokens": snap.weekly_tokens, "limit": cfg.weekly_tokens}),
        });
    }
    // 3. 반복 실패 (fail수·실패율·최소표본 동시 충족)
    for (tool, calls, fail, rate) in &snap.tool_failures {
        if *fail >= cfg.fail_count && *calls >= cfg.fail_min_calls && *rate >= cfg.fail_rate {
            out.push(Alert {
                kind: "repeated_failure".into(),
                key: format!("repeated_failure:{tool}"),
                severity: if *rate >= 0.5 { "crit" } else { "warn" }.into(),
                message: format!("{tool} 반복 실패 {}/{} ({:.0}%)", fail, calls, rate * 100.0),
                detail: json!({"tool": tool, "calls": calls, "fail": fail, "fail_rate": rate}),
            });
        }
    }
    // 4. ★T-0147-2 §3 Phase1 A8: 노드 생존 판정 배지 → 기존 Alert 표면으로 승격.
    //    데몬은 판정자가 아니라 **운반자**다(판정은 python 델타게이트). severity 어휘 매핑:
    //    게이트 "critical"→"crit", 그 밖("warn"·미지)→"warn" — Alert 의 wire 어휘는 warn|crit
    //    둘뿐이므로 미지 값이 새 severity 로 새어나가지 않게 warn 으로 좁힌다.
    //    ★`info` 는 **경보로 승격하지 않는다**(T-0147-2 §1-B N1: 정상 대기의 배지는 `quiet`다).
    //      게이트는 매 주기 배지 파일을 쓰므로 info 를 warn 으로 접으면 **정상 운영 중 상시
    //      경고**가 켜진다 — 이 설계가 없애려는 바로 그 오발화다. info 배지는 배지 파일에
    //      그대로 남아 관측 가능하고(암전 아님), 경보 표면만 warn·crit 로 유지한다.
    //    ★정렬 **앞**에 push 해야 out 전체가 key 오름차순 결정론을 유지한다.
    for b in snap.gate_badges.iter().filter(|b| b.severity != "info") {
        out.push(Alert {
            kind: "node_liveness".into(),
            key: format!("node_liveness:{}", b.key),
            severity: if b.severity == "critical" { "crit" } else { "warn" }.into(),
            message: b.message.clone(),
            detail: b.detail.clone(),
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// ★T-0147-2 §3 Phase1 A8 — 델타게이트 배지 수집(파일 oracle)
// ─────────────────────────────────────────────────────────────────────────────

/// state_dir 직속에서 게이트 레인을 찾아 각 레인의 `badges.json` 을 배지로 읽는다.
///
/// 레인 = 파일명이 `report_gate` 로 시작하는 **디렉터리**(`report_gate`, `report_gate-dept-2` …).
/// 부서 데몬마다 레인이 갈리므로(설계 층3 B7 레인 분리) 키에 레인 접두를 붙여 충돌을 막는다.
///
/// 침묵 금지 규약 3종 —
///   ① `badges.json` 이 있는데 `updated_at` 이 `CYS_GATE_BADGE_STALE_SECS`(기본 1800s)보다
///      오래되면 그 레인의 배지 대신 **데드맨 배지 1개**를 낸다. 게이트가 죽어 배지가 낡은 채
///      "quiet" 를 계속 보여주는 것이 정확히 암전이다.
///   ② 파싱 실패는 조용히 넘기지 않고 `badges-corrupt` 합성 배지로 노출한다(부패 은닉 금지).
///   ③ 파일 부재·권한 실패만 조용히 skip — 게이트 미설치 레인은 **정상 경우**이고, 그것까지
///      경보로 만들면 설치 안 한 부서마다 상시 경보가 켜진다.
pub fn collect_gate_badges(state_dir: &std::path::Path, now: f64) -> Vec<GateBadge> {
    /// 레인 상한 — 부서 데몬이 늘어도 스냅샷 비용·배지 수를 유계로 묶는다.
    const MAX_LANES: usize = 16;
    /// 레인당 배지 상한 — 게이트 버그로 배지가 폭주해도 CC 배지판을 덮지 못하게.
    const MAX_BADGES_PER_LANE: usize = 32;
    let stale_secs = env_u64("CYS_GATE_BADGE_STALE_SECS", 1800) as f64;
    let Ok(rd) = std::fs::read_dir(state_dir) else {
        return Vec::new(); // state_dir 자체 부재·권한 실패 = 배지 없음(조용히)
    };
    let mut lanes: Vec<(String, std::path::PathBuf)> = Vec::new();
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        if !name.starts_with("report_gate") {
            continue;
        }
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        lanes.push((name, path));
    }
    lanes.sort_by(|a, b| a.0.cmp(&b.0)); // 정렬 후 앞 16 — 어떤 16개가 잡히는지도 결정론이어야 한다
    lanes.truncate(MAX_LANES);

    let mut out: Vec<GateBadge> = Vec::new();
    for (dirname, path) in lanes {
        let Ok(raw) = std::fs::read_to_string(path.join("badges.json")) else {
            continue; // 미설치 레인(정상) — ③
        };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            // ② 부패 은닉 금지. lane 값을 못 읽으므로 디렉터리명으로 키를 짓는다.
            out.push(GateBadge {
                key: format!("{dirname}/badges-corrupt"),
                severity: "warn".into(),
                message: format!("게이트 배지 파일 파싱 실패 — 판정 결과를 읽을 수 없다(레인 {dirname})"),
                detail: json!({"lane": dirname}),
            });
            continue;
        };
        let lane = v
            .get("lane")
            .and_then(|x| x.as_str())
            .unwrap_or(&dirname)
            .to_string();
        // updated_at 부재는 0.0 → age 가 epoch 전체가 되어 데드맨으로 잡힌다(스키마 위반 노출).
        let age = now - v.get("updated_at").and_then(|x| x.as_f64()).unwrap_or(0.0);
        if stale_secs > 0.0 && age > stale_secs {
            // ① 데드맨: 낡은 배지를 그대로 보여주면 죽은 게이트가 "정상"을 계속 방송한다.
            out.push(GateBadge {
                key: format!("{lane}/gate-stale"),
                severity: "warn".into(),
                message: format!(
                    "게이트 배지 갱신 정지 {}초 — 델타게이트 데드맨(레인 {})",
                    age as u64, lane
                ),
                detail: json!({"lane": lane, "age_secs": age as u64}),
            });
            continue;
        }
        let badges = v.get("badges").and_then(|x| x.as_array());
        for b in badges.map(|a| a.as_slice()).unwrap_or(&[]).iter().take(MAX_BADGES_PER_LANE) {
            let Some(key) = b.get("key").and_then(|x| x.as_str()) else {
                continue; // key 없는 배지는 조인 불가 — 승격 대상 아님
            };
            out.push(GateBadge {
                key: format!("{lane}/{key}"),
                severity: b
                    .get("severity")
                    .and_then(|x| x.as_str())
                    .unwrap_or("warn")
                    .to_string(),
                message: b
                    .get("message")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                detail: b.get("detail").cloned().unwrap_or_else(|| json!({})),
            });
        }
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// ★T-0147-2 §1-B N6b — 게이트 신호(state 외부 oracle)
// ─────────────────────────────────────────────────────────────────────────────

/// 게이트가 **state 를 못 쓰는 상황**에서 올린 신호: name → (관측 시각, detail).
/// 같은 state 에 있는 원장·badges.json 을 oracle 로 기대하면 자기모순이므로(설계 R3-GATE),
/// 데몬 프로세스 메모리가 유일한 수용처다. 재시작에 사라지는 것은 의도된 성질이다 —
/// 재시작 후에도 문제가 남아 있으면 게이트가 다음 주기에 다시 신호한다.
static GATE_SIGNALS: OnceLock<Mutex<HashMap<String, (f64, Value)>>> = OnceLock::new();

fn gate_signals() -> &'static Mutex<HashMap<String, (f64, Value)>> {
    GATE_SIGNALS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 게이트 신호 흡수(schedule.rs sniffer 가 호출). 같은 name 재도착은 시각 갱신 = 최신 관측 승.
pub fn note_gate_signal(name: &str, detail: Value) {
    let now = crate::state::now_epoch();
    let ttl = env_u64("CYS_GATE_SIGNAL_TTL_SECS", 1800) as f64;
    let mut g = gate_signals().lock().unwrap_or_else(|e| e.into_inner());
    g.insert(name.to_string(), (now, detail));
    // 무한 성장 차단: name 공간은 schedule.rs allowlist 로 유계지만, TTL 2배 지난 항목은 GC.
    if ttl > 0.0 {
        g.retain(|_, (t, _)| now - *t <= 2.0 * ttl);
    }
}

/// 순수부(테스트 핀): 프로세스 전역 static 을 만지지 않고 TTL 경계·매핑만 검증할 수 있게 분리.
fn gate_signal_badges_from(
    entries: &HashMap<String, (f64, Value)>,
    now: f64,
    ttl: f64,
) -> Vec<GateBadge> {
    let mut out: Vec<GateBadge> = entries
        .iter()
        .filter(|(_, (t, _))| now - t < ttl)
        .map(|(name, (t, detail))| {
            let age = (now - t) as u64;
            // detail 이 객체가 아니면 감싸서 age_secs 를 붙일 자리를 만든다(정보 유실 0).
            let mut d = if detail.is_object() {
                detail.clone()
            } else {
                json!({ "detail": detail.clone() })
            };
            if let Some(o) = d.as_object_mut() {
                o.insert("age_secs".into(), json!(age));
            }
            GateBadge {
                key: format!("signal/{name}"),
                // state 기록 불능은 '보고 자체가 불가능한' 상태라 critical — 다른 신호는 warn.
                severity: if name.ends_with("state_unwritable") {
                    "critical"
                } else {
                    "warn"
                }
                .into(),
                // 문구는 일반형 1종. name 별 의미·맥락은 detail 이 나른다(문구 분기 = 계약 증식).
                message: format!("게이트 신호 {name} — 대장 기록 불능(state 외부 oracle)"),
                detail: d,
            }
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

/// TTL(`CYS_GATE_SIGNAL_TTL_SECS`, 기본 1800s) 내 게이트 신호를 배지로. 0=비활성(전부 만료).
pub fn gate_signal_badges(now: f64) -> Vec<GateBadge> {
    let ttl = env_u64("CYS_GATE_SIGNAL_TTL_SECS", 1800) as f64;
    let g = gate_signals().lock().unwrap_or_else(|e| e.into_inner());
    gate_signal_badges_from(&g, now, ttl)
}

/// 데몬에서 평가 스냅샷 수집 — 노드 rate(in-memory) + 7d usage_records/events(analytics).
/// 락 순서: surfaces → (해제) → analytics. consumption 미사용(교착 회피).
/// ★0.14.43(B3) 신선도 규칙: 좌석 신원 표를 **한 번** 만들어 좌석 축·계정 축이 함께 쓴다 — 락 순서 surfaces(복사만) → 해제 → 신원 표(캐시) →
/// surfaces(좌석 rate 수집 — 파일 IO 없음) → 해제 → accounts(메모리 연산뿐) → 해제 → analytics. 겹쳐 쥐는 락 쌍이 새로 생기지 않는다.
/// ★R1F-US(M-1): **이 판은 RPC 경로(`control.alerts`)용 읽기-통과**다 — 신원 표가 캐시 미스일 때 신원 파일을 확인한다(폴더별 60초 하한 · 미스 때 IO 전에 선점). **워치독(경보 틱)은 이 함수를 부르지 않는다** —
/// 스냅샷이 신원 파일을 열면 그 디스크가 응답하지 않을 때 같은 틱의 큐 배달·사망 감지·데드맨 점검이 함께 설 수 있다(오너 절대 기준 ③ — 코드 경로로 본 것이고 멈춘 디스크로 실측하지는 않았다).
/// 워치독은 [`snapshot_cached_with_stale`](캐시 전용 · IO 0)을 쓴다.
pub fn snapshot(daemon: &Arc<Daemon>, now: f64) -> Snapshot {
    snapshot_with_stale(daemon, now, crate::accounts::account_alert_stale_secs())
}

/// [`snapshot`] 의 시험 이음매 — 신선도 규칙의 나이 상한(초)을 인자로 받는다(`0` = 규칙 끔 = 0.14.42 동작에 **가깝지만 같지는 않다** — 좌석별 경보에서 리셋 시각이 없는 창의 생사를 갱신 시각이 아니라 값을 실제로 관측한 시각으로 따지고,
/// `usage.alert_resolved` 의 `cleared` 가 난다 · `USER-MANUAL.md` 노브 표·릴리스 노트 §4 · R2F-DM 성찰 2회차 A5 m11). 환경변수를 건드리지 않고(병렬 검체가 서로 오염되지 않게)
/// 노브 값별 시나리오를 돌린다. 운영은 [`snapshot`] 이 노브 값(`CYS_ACCOUNT_ALERT_STALE_SECS`)을 넘긴다. 락 순서 등 본체 계약은 [`snapshot`] 의 문서와 같다(읽기-통과 신원 표).
pub fn snapshot_with_stale(daemon: &Arc<Daemon>, now: f64, stale_secs: f64) -> Snapshot {
    let view = crate::accounts::seat_identity_view_at(daemon, now);
    snapshot_from_view(daemon, now, stale_secs, &view, None)
}

/// ★R1F-US(M-1) **워치독 틱 전용** 스냅샷 — 신원 표를 **캐시 전용**으로 만든다([`crate::accounts::seat_identity_view_cached`] · IO 0 · stat 0: 항목이 있으면 만료됐어도 마지막 값 · 없으면 None = 판정 불가 = 사용 중 —
/// 실패 방향은 경보 유지 = 0.14.42 동작). 캐시는 상태줄 보고와 RPC 의 읽기-통과 조회가 채운다. 락 순서·본체 계약은 [`snapshot`] 과 같다(본체 [`snapshot_from_view`]).
/// ★R1F-US(m-6·b): `cfg` 는 붙들 키를 **임계로 좁히는** 데만 쓴다 — 신선도 규칙으로 빠진 창 가운데 **임계 이상**(`evaluate` 가 경보로 냈을 값)인 것의 키만 `stale_suppressed` 에 싣는다. 임계 미만의 낡은 창이 같은 키를
/// 쓰는 다른 좌석·계정의 발화 키를 붙들어 새 교차가 30분 늦게 나가는 일을 없앤다(0.14.42 는 즉시).
pub fn snapshot_cached_with_stale(daemon: &Arc<Daemon>, now: f64, stale_secs: f64, cfg: &AlertConfig) -> Snapshot {
    let view = crate::accounts::seat_identity_view_cached(daemon);
    snapshot_from_view(daemon, now, stale_secs, &view, Some(cfg))
}

/// 스냅샷 본체 — 신원 표(`view`)는 **부른 쪽이** 만들어 넘긴다(읽기-통과판은 파일 IO 가 있으므로 surfaces 락을 잡기 전에). 이 본체는 신원 표를 만들지 않는다(락 순서·IO 위치의 계약).
/// `hold` = 붙들 키를 임계로 좁힐 설정(None = 좁히지 않는다). ★R1F-US(m-6·a): 같은 키의 입력이 둘 이상이면 **값이 가장 큰 입력 하나**를 쥐고 그 입력의 메타를 싣는다(발행되는 경보의 값과 `age=`·`in_use=`·`reset=` 이 같은 입력의 것).
fn snapshot_from_view(daemon: &Arc<Daemon>, now: f64, stale_secs: f64, view: &crate::accounts::SeatIdentityView, hold: Option<&AlertConfig>) -> Snapshot {
    let mut seat_best: HashMap<(String, String), (f64, RateMeta)> = HashMap::new();
    let mut stale_suppressed: Vec<String> = Vec::new();
    {
        let surfaces = daemon.surfaces.lock().unwrap();
        // 좌석 id 순으로 훑는다 — 같은 값·같은 나이의 입력이 겹칠 때 어느 입력이 남는지가 `HashMap` 순서에 달리지 않는다.
        let mut seats: Vec<&Arc<crate::state::Surface>> = surfaces.values().collect();
        seats.sort_by_key(|s| s.id);
        for s in seats {
            if s.exited.load(Ordering::Relaxed) {
                continue;
            }
            let role = s.role.lock().unwrap().clone().unwrap_or_else(|| "?".into());
            if let Some(u) = s.observed_usage.lock().unwrap().as_ref() {
                // ★fatal-fix N1: agy 좌석(agent gemini)의 쿼터는 **계정 하나**의 사실이다 — 좌석마다 싣으면 좌석 N개가 같은
                //   쿼터로 키 2N개를 만들어 CSO 시간당 경보 예산을 잠식했다(좌석 3개 → 키 8개). 같은 사실은 계정 축
                //   (`account_rate:Antigravity (agy)`)이 덮는다. 좌석 배지 표시는 그대로다(경보 입력에서만 뺀다).
                if u.agent == "gemini" {
                    continue;
                }
                // ★0.14.43(B3): rate 의 관측 시각은 `rate_observed_at`(새로 생산된 시각 · 이월되는 값) — `updated_at` 은 transcript 이월에도 now 로 찍혀 rate 의 나이가
                //   아니다. 모르면(0) 종전처럼 `updated_at` 으로 창의 생사만 보고 나이 판정은 하지 않는다(적격).
                let known_at = u.rate_observed_at.is_finite() && u.rate_observed_at > 0.0;
                let live_at = if known_at { u.rate_observed_at } else { u.updated_at };
                let age = if known_at { (now - u.rate_observed_at).max(0.0) } else { 0.0 };
                let in_use = crate::accounts::seat_in_use_tri(
                    u.rate_account.as_deref(),
                    view.current_for(s.id),
                    true,
                    u.agent == "claude",
                );
                for w in &u.rate {
                    // ★fatal-fix R3-2: 리셋이 지난 창은 경보 근거가 아니다(UI '리셋됨'과 같은 규칙).
                    if !crate::usage::rate_window_live(w, live_at, now) {
                        continue;
                    }
                    if !crate::accounts::alert_eligible(true, in_use, age, stale_secs) {
                        // 신선도 규칙으로만 빠진 창 — 틱이 이 키를 `fired` 에 붙들어 깜빡임(재발행)을 막는다. ★R1F-US(m-6·b): 틱 판은 임계 이상인 창만 붙든다.
                        if hold.is_none_or(|c| w.used_pct >= c.rate_limit_pct) {
                            stale_suppressed.push(format!("rate_limit:{role}:{}", w.label));
                        }
                        continue;
                    }
                    keep_best(
                        &mut seat_best,
                        (role.clone(), w.label.clone()),
                        w.used_pct,
                        RateMeta {
                            observed_at: if known_at { u.rate_observed_at } else { 0.0 },
                            age_secs: age,
                            in_use,
                            resets_at: w.resets_at,
                            reset_in_secs: reset_in_secs(w.resets_at, now),
                            held_secs: None,
                        },
                    );
                }
            }
        }
    }
    let since = crate::analytics::window_since(now, "7d");
    let (weekly_cost_usd, weekly_tokens, tool_failures) = {
        let guard = daemon.analytics.lock().unwrap();
        match guard.as_ref() {
            Some(conn) => {
                let a = crate::analytics::analytics_summary(conn, since);
                let cost = a["totals"]["cost_usd"].as_f64().unwrap_or(0.0);
                let toks = a["totals"]["tokens"].as_u64().unwrap_or(0);
                let sk = crate::analytics::skills_summary(conn, since);
                let fails = sk["failures"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .map(|f| {
                                (
                                    f["name"].as_str().unwrap_or("").to_string(),
                                    f["calls"].as_u64().unwrap_or(0),
                                    f["fail"].as_u64().unwrap_or(0),
                                    f["fail_rate"].as_f64().unwrap_or(0.0),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (cost, toks, fails)
            }
            None => (0.0, 0, Vec::new()),
        }
    };
    // ★0.14.43(B3): 계정 축 — 적격 행만 입력이 되고, 신선도 규칙으로 빠진 창은 `stale_suppressed` 로 따로 안다.
    let mut account_best: HashMap<(String, String), (f64, RateMeta)> = HashMap::new();
    for r in crate::accounts::alert_rates_with(daemon, view, now, stale_secs) {
        if !r.eligible {
            // ★R1F-US(m-6·b): 틱 판은 임계 이상인 창만 붙든다.
            if hold.is_none_or(|c| r.used_pct >= c.account_warn_pct) {
                stale_suppressed.push(format!("account_rate:{}:{}", r.label, r.win));
            }
            continue;
        }
        keep_best(
            &mut account_best,
            (r.label.clone(), r.win.clone()),
            r.used_pct,
            RateMeta {
                observed_at: r.observed_at,
                age_secs: r.age_secs,
                in_use: r.in_use,
                resets_at: r.resets_at,
                reset_in_secs: reset_in_secs(r.resets_at, now),
                held_secs: r.held_secs,
            },
        );
    }
    stale_suppressed.sort();
    stale_suppressed.dedup();
    // 같은 키의 입력은 하나로 접혔다(값이 가장 큰 입력) — 키 오름차순으로 낸다(결정론).
    let mut rates: Vec<(String, String, f64)> = seat_best.iter().map(|((r, l), (p, _))| (r.clone(), l.clone(), *p)).collect();
    rates.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let rate_meta: HashMap<(String, String), RateMeta> = seat_best.into_iter().map(|(k, (_, m))| (k, m)).collect();
    let mut account_rates: Vec<(String, String, f64)> = account_best.iter().map(|((l, w), (p, _))| (l.clone(), w.clone(), *p)).collect();
    account_rates.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let account_meta: HashMap<(String, String), RateMeta> = account_best.into_iter().map(|(k, (_, m))| (k, m)).collect();
    // ★T-0147-2 A8 + N6b: 두 oracle 을 합친다 —
    //   파일 oracle(레인별 badges.json) = 게이트가 state 를 쓸 수 있을 때의 정상 경로,
    //   state 외부 oracle(gate_signal_badges) = 바로 그 state 를 못 쓸 때의 유일한 경로.
    // 합친 뒤 key 로 재정렬해야 evaluate() 이전에도 결정론이 선다.
    let mut gate_badges = collect_gate_badges(&crate::state::state_dir(&daemon.socket_path), now);
    gate_badges.extend(gate_signal_badges(now));
    gate_badges.sort_by(|a, b| a.key.cmp(&b.key));
    Snapshot {
        rates,
        account_rates,
        weekly_cost_usd,
        weekly_tokens,
        tool_failures,
        gate_badges,
        account_meta,
        rate_meta,
        stale_suppressed,
    }
}

/// 리셋까지 남은 초 — `resets_at` 이 epoch 초로 보일 때만(상대 초 등 다른 단위의 값에서 의미 없는 음수를 만들지 않는다).
fn reset_in_secs(resets_at: Option<f64>, now: f64) -> Option<f64> {
    resets_at.filter(|r| r.is_finite() && *r >= 1.0e9).map(|r| r - now)
}

/// ★R1F-US(m-6·a) 같은 키의 입력이 둘 이상이면(같은 역할명·역할 없음(`?`) 좌석 여럿 · 같은 이메일의 두 계정) **값(사용률)이 가장 큰 입력**을 쥐고 그 입력의 메타를 함께 쥔다 — 발행되는 경보의 값과 메타(`age=`·`in_use=`·`reset=`)가
/// 같은 입력의 것이어야 한다(종전엔 나이가 가장 작은 입력의 메타를 쥐어 "3시간 전 99%" 가 `age=5` 로 나갈 수 있었다). 같은 값이면 관측 나이가 작은 쪽. (`evaluate` 는 키당 경보 하나로 나가므로 입력도 키당 하나면 된다.)
fn keep_best(map: &mut HashMap<(String, String), (f64, RateMeta)>, key: (String, String), pct: f64, meta: RateMeta) {
    use std::collections::hash_map::Entry;
    match map.entry(key) {
        Entry::Vacant(v) => {
            v.insert((pct, meta));
        }
        Entry::Occupied(mut o) => {
            let better = {
                let (cur_pct, cur_meta) = o.get();
                pct > *cur_pct || (pct == *cur_pct && meta.age_secs < cur_meta.age_secs)
            };
            if better {
                o.insert((pct, meta));
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ★(0.15.0 · 「스스로 해결」 M4 · 설계 §4-4 · §6-4 (1)(4)(7) · RULINGS AD8, BF1–BF6 · track T3b) 결함 감지기
// ─────────────────────────────────────────────────────────────────────────────
//
// The repeated-failure detection above (`evaluate` §3) judges per TOOL ("Bash 반복 실패") — useless for a
// repair, which needs to know WHICH hook / script failed and how. This section adds that identity and turns a
// repeated failure into ONE defect-intake row per signature (`repairs/intake/<lane_key>/<sig>.json`, T3's row
// format; the signature is T3's `intake::parse_report` over the ONE normaliser `intake::stable_error_line`,
// RULINGS BF3), with the same thresholds (`AlertConfig` fail_count / fail_rate / fail_min_calls) applied per
// identity over the same 7-day window.
// The module's doctrine holds: ★자동응답 금지 — 감지·격상만. Escalation here is a ledger row (DATA, never
// authority); nothing in this section queues, pushes, publishes, alerts or starts a repair. The master hears
// at most one line per report-gate run, built by the gate from the ledger (T3, RULINGS AD5).
// IDENTITY (RULINGS BF1) = the EXECUTED program: a failure belongs to a pack hook / script only when that file
// is what the shell call RAN — the program of the call's FINAL simple command, or the script operand of an
// interpreter there — and only in THIS lane's pack. A mention (an argument of grep / cat / diff / a user's
// script, a pipeline member piped onward, a heredoc body, a quoted string, a comment) is no identity.
// INPUT (RULINGS BE1 — the CLI forwards it from Claude Code's `PostToolUseFailure`): `tool_name` "Bash",
// `tool_input.command`, the top-level `error` (head + tail), `is_interrupt`; a call is a `PRE_TOOL` event.
// Three places, three costs:
//   ① `observe_tool_event` — on the hook's request path: the command is classified BEFORE any lock (one
//      bounded linear pass, RULINGS BF5), then a short update of in-memory tables under the detector's own
//      leaf mutex. No disk, no spawn on the path (the flusher is started once, lazily, by an injectable hook).
//   ② `flush_defect_intake` — on its own thread (`cysd-defect-intake`, 10 s): the row writes, under T3's
//      per-file mutex (`intake:<sig>`, one writer per file) and the intake's OWN quota
//      (`alert_route::IntakeShare` — never the general share, the clear reserve or the CSO queue).
//      Never on the watchdog tick: a disk that stops answering must not stall seat-death detection (F3/F4).
//   ③ the exclusion list (`cys::repair::intake::EXCLUSIONS` + this detector's `DETECTOR_EXCLUSIONS`) —
//      failures that happen BY DESIGN, on positive evidence only (RULINGS BF2), never count.

/// Rollback switch of the detector (default on; `0`/`false`/`off`/`no` → off), read once per process.
pub const DEFECT_INTAKE_ENV: &str = "CYS_DEFECT_INTAKE";
/// Pack scripts tracked per daemon (beyond: the idlest one that has nothing due is evicted — RULINGS BF4).
pub const DETECTOR_MAX_RELS: usize = 1024;
/// Signatures tracked per daemon (beyond: the oldest one that is not due is evicted, else the oldest).
pub const DETECTOR_MAX_SIGNATURES: usize = 512;
/// How much of a command line is classified (longer → cut at a char boundary).
pub const COMMAND_SCAN_MAX_BYTES: usize = 64 * 1024;
/// How much failure text is kept (longer → its head and its tail, where a traceback ends).
pub const ERROR_KEEP_BYTES: usize = 32 * 1024;
/// The rule's window in days — like `evaluate` §3's 7-day usage window (RULINGS BF4).
pub const DETECTOR_WINDOW_DAYS: i64 = 7;
/// The reporter label of rows the detector writes.
pub const DETECTOR_REPORTER: &str = "detector";
/// The flusher thread's period (seconds).
pub const FLUSH_TICK_SECS: u64 = 10;
// "발견 즉시" (D:37): a row appears within one flusher period of the rule being met — keep it short.
const _: () = assert!(FLUSH_TICK_SECS > 0 && FLUSH_TICK_SECS <= 30);

/// The detector's own entries of the exclusion list (scope `Detector`): a verdict tool whose non-zero
/// exit IS its designed answer — excluded only on POSITIVE evidence (RULINGS BF2): the exit code as the
/// first line AND the tool's own fixed verdict marker for that code (read from the tool; file:line
/// beside each), never when the text shows an interpreter or launch failure
/// (`intake::shows_launch_failure`). A tool without a stable marker per answer has NO entry — its
/// by-design "no" becomes one bundled row (D:658): `bin/javis_task.py` (its exits 3/4/5/8/9 are printed
/// with different texts at different call sites).
pub const DETECTOR_EXCLUSIONS: &[cys::repair::intake::Exclusion] = {
    use cys::repair::intake::{DesignedExit, Exclusion, ExclusionRule, ExclusionScope};
    &[
        // javis_resource_gate.py:90 EXIT_SOFT 1 / EXIT_HARD 2; cmd_check prints `verdict: <v>` (text,
        // :2528) or `"verdict": "<v>"` (--json, :2526). Usage 64 and internal 70 stay defects.
        Exclusion {
            reason: "designed-answer:javis_resource_gate.check",
            windows_only: false,
            scope: ExclusionScope::Detector,
            rule: ExclusionRule::DesignedAnswer {
                rel: "bin/javis_resource_gate.py",
                sub: Some("check"),
                answers: &[
                    DesignedExit { exit: 2, markers: &[&["verdict: hard_block"], &["\"verdict\": \"hard_block\""]] },
                    DesignedExit { exit: 1, markers: &[&["verdict: soft_warn"], &["\"verdict\": \"soft_warn\""]] },
                ],
            },
        },
        // javis_orchestra.py:794–798 CHECK_EXIT_MISSING 1 / ACK_PENDING 12; cmd_check's summary lines
        // (:1146 "종합: 필수 …/… 생존 — 부재:", :1162 "… 각성 ACK 미확인") or the --json payload id
        // `"schema": "orchestra-check/v1"` with `"exit": N` (:1011). NOT exit 2 (UNJUDGEABLE, RULINGS BP2):
        // its marker "[orchestra check] cys status 수집 실패" is printed for a missing `cys`, a failing
        // `cys status` and any exception in the helper (:386–397) — it cannot tell an answer from a defect.
        Exclusion {
            reason: "designed-answer:javis_orchestra.check",
            windows_only: false,
            scope: ExclusionScope::Detector,
            rule: ExclusionRule::DesignedAnswer {
                rel: "bin/javis_orchestra.py",
                sub: Some("check"),
                answers: &[
                    DesignedExit {
                        exit: 1,
                        markers: &[&["종합: 필수 ", " 생존 — 부재: "], &["\"schema\": \"orchestra-check/v1\"", "\"exit\": 1"]],
                    },
                    DesignedExit {
                        exit: 12,
                        markers: &[&["종합: ", "각성 ACK 미확인"], &["\"schema\": \"orchestra-check/v1\"", "\"exit\": 12"]],
                    },
                ],
            },
        },
        // javis_wakeup.py:71,75 EXIT_EMPTY 5 / EXIT_PAUSED 4; cmd_drain prints "refused: AUTOPILOT_PAUSED"
        // (:656) or `"refused": "paused"` (:659) and "nothing pending" (:670); cmd_cancel "nothing to
        // cancel" (:617) or `{"cancelled": 0` (:632). Usage 2 and park failure 6 stay defects.
        Exclusion {
            reason: "designed-answer:javis_wakeup",
            windows_only: false,
            scope: ExclusionScope::Detector,
            rule: ExclusionRule::DesignedAnswer {
                rel: "bin/javis_wakeup.py",
                sub: None,
                answers: &[
                    DesignedExit { exit: 4, markers: &[&["refused: AUTOPILOT_PAUSED"], &["\"refused\": \"paused\""]] },
                    DesignedExit {
                        exit: 5,
                        markers: &[&["nothing pending"], &["nothing to cancel"], &["\"cancelled\": 0"]],
                    },
                ],
            },
        },
    ]
};

/// The detector's exclusion lookup: the list's shared table first (`intake::EXCLUSIONS` — T3's Windows
/// `cys-dept` entry), then the detector's own entries. Returns the reason.
pub fn detector_excluded(rel: &str, sub: Option<&str>, error: &str, windows: bool) -> Option<&'static str> {
    use cys::repair::intake::{excluded_in, EXCLUSIONS};
    excluded_in(EXCLUSIONS, rel, sub, error, windows).or_else(|| excluded_in(DETECTOR_EXCLUSIONS, rel, sub, error, windows))
}

/// One `usage.event` as the detector sees it (built by the `usage.event` arm from its params).
#[derive(Clone, Copy, Debug)]
pub struct ToolObservation<'a> {
    /// `PRE_TOOL` | `POST_TOOL` | … (the CLI's mapping).
    pub event_type: &'a str,
    /// The raw hook event name (`PreToolUse`, `PostToolUse`, `PostToolUseFailure` …).
    pub raw_event: &'a str,
    /// Claude Code's tool name; only `Bash` runs a command line.
    pub tool_name: &'a str,
    pub tool_input: &'a Value,
    pub exit_code: Option<i64>,
    /// The failure text (`PostToolUseFailure` `error`, head + tail — RULINGS BE1).
    pub error: Option<&'a str>,
    /// The failure reached Claude Code as an abort (`is_interrupt`) — not a failure of the script.
    pub interrupted: bool,
}

/// The pack hook / script a shell call RAN (RULINGS BF1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackCall {
    /// The call ran no pack hook / script of this lane.
    None,
    /// It ran exactly this one: its lane-independent pack-relative path and its first argument.
    One { rel: String, sub: Option<String> },
}

/// This lane's pack-root spellings, built once per daemon: the daemon's own pack dir (and its Git Bash
/// `/c/…` form, and its `~` / `$HOME` / `${HOME}` forms when it lies under the home), plus the seat's
/// lane-scoped `$CYS_PACK_DIR` / `${CYS_PACK_DIR}` / `${CYS_PACK_DIR:-…}`. Another lane's pack, a copy
/// in a test rig or any other directory is not this lane's pack.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackRoots {
    prefixes: Vec<String>,
}

fn slash_path(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_string()
}

impl PackRoots {
    pub fn new(pack_dir: Option<&str>, home: Option<&str>) -> Self {
        let mut prefixes = vec!["$CYS_PACK_DIR/".to_string(), "${CYS_PACK_DIR}/".to_string()];
        if let Some(pd) = pack_dir.map(slash_path).filter(|p| p.len() > 1) {
            prefixes.push(format!("{pd}/"));
            let b = pd.as_bytes();
            if b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/' {
                prefixes.push(format!("/{}{}/", (b[0] as char).to_ascii_lowercase(), &pd[2..]));
            }
            if let Some(h) = home.map(slash_path).filter(|h| h.len() > 1) {
                if let Some(rest) = pd.strip_prefix(&h).filter(|r| r.starts_with('/')) {
                    for head in ["~", "$HOME", "${HOME}"] {
                        prefixes.push(format!("{head}{rest}/"));
                    }
                }
            }
        }
        PackRoots { prefixes }
    }

    /// The rel of `word` when it names a hook / script inside this lane's pack. Linear in the word.
    fn rel_of(&self, word: &str, steps: &mut usize) -> Option<String> {
        let w = word.replace('\\', "/");
        *steps += w.len();
        let rest = if let Some(after) = w.strip_prefix("${CYS_PACK_DIR:") {
            // `${CYS_PACK_DIR:-<default>}/…` — the matching `}` of THIS word, then `/` (one pass)
            let mut depth = 1usize;
            let mut close = None;
            for (k, c) in after.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(k);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            after[close? + 1..].strip_prefix('/')?.to_string()
        } else {
            let p = self.prefixes.iter().find(|p| w.starts_with(p.as_str()))?;
            w[p.len()..].to_string()
        };
        if rest.contains(['$', '`', '(', ')', '{', '}', '*', '?', '[', ']']) {
            return None;
        }
        let rel = cys::repair::intake::normalize_rel(&rest).ok()?;
        is_script_rel(&rel).then_some(rel)
    }
}

/// The command string of a shell call (`tool_input.command`, a string or a list of strings).
fn shell_command(tool_input: &Value) -> Option<String> {
    match tool_input.get("command")? {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => {
            let parts: Vec<&str> = a.iter().filter_map(|x| x.as_str()).collect();
            (!parts.is_empty()).then(|| parts.join(" "))
        }
        _ => None,
    }
}

fn cut_at_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

/// A pack-relative path that names a hook / script (not a document, a glob or a directory).
fn is_script_rel(rel: &str) -> bool {
    let last = rel.rsplit('/').next().unwrap_or(rel);
    if last.is_empty() {
        return false;
    }
    match last.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty() && matches!(ext, "py" | "sh" | "bash" | "js" | "mjs" | "cjs" | "ps1")
        }
        None => rel.starts_with("bin/") && rel.len() > "bin/".len(),
    }
}

/// `NAME=value` before a command (`FOO=1 python3 …`).
fn is_env_assignment(w: &str) -> bool {
    match w.split_once('=') {
        Some((name, _)) => {
            name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// The words of the call when its WHOLE text is exactly ONE simple command (RULINGS BS1), read by ONE
/// left-to-right pass (every char is visited at most once; `steps` counts the visits — RULINGS BF5).
/// Quotes are removed from words (single quotes are literal); `${…}` stays inside its word. `None` — a
/// stated MISS — for any control operator (`&&` `||` `;` `|` `|&` `&`, a newline between commands), a
/// subshell or group parenthesis, any substitution (`$(`, `$((`, a backtick — inside double quotes
/// too —, `<(`, `>(`), a heredoc, here-string or input redirection, an output redirection to anything
/// but `/dev/null` or a descriptor copy (`2>&1`, `>&2`), and any reading that is not certain (an
/// unterminated quote or `${`, a trailing backslash, a redirection without its target).
fn single_simple_command(src: &str, steps: &mut usize) -> Option<Vec<String>> {
    let cs: Vec<char> = src.chars().collect();
    let n = cs.len();
    let mut i = 0usize;
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut to_devnull = false; // the next word is an output redirection's target: only `/dev/null`
    macro_rules! end_word {
        () => {
            if in_word {
                let w = std::mem::take(&mut word);
                if to_devnull {
                    if w != "/dev/null" {
                        return None;
                    }
                    to_devnull = false;
                } else {
                    words.push(w);
                }
                in_word = false;
            }
        };
    }
    while i < n {
        let c = cs[i];
        *steps += 1;
        match c {
            ' ' | '\t' | '\r' => {
                end_word!();
                i += 1;
            }
            '\n' => {
                // only whitespace may follow: a newline BETWEEN commands is a second command
                end_word!();
                if cs[i..].iter().any(|d| !d.is_whitespace()) {
                    return None;
                }
                *steps += n - i;
                i = n;
            }
            '#' if !in_word => {
                while i < n && cs[i] != '\n' {
                    i += 1;
                    *steps += 1;
                }
            }
            '\\' => {
                if i + 1 >= n {
                    return None;
                }
                if cs[i + 1] != '\n' {
                    word.push(cs[i + 1]);
                    in_word = true;
                }
                i += 2;
                *steps += 1;
            }
            '\'' => {
                in_word = true;
                i += 1;
                while i < n && cs[i] != '\'' {
                    word.push(cs[i]);
                    i += 1;
                    *steps += 1;
                }
                if i >= n {
                    return None;
                }
                i += 1;
            }
            '"' => {
                in_word = true;
                i += 1;
                while i < n && cs[i] != '"' {
                    if cs[i] == '`' || (cs[i] == '$' && i + 1 < n && cs[i + 1] == '(') {
                        return None;
                    }
                    if cs[i] == '\\' && i + 1 < n && matches!(cs[i + 1], '"' | '\\' | '$' | '`') {
                        i += 1;
                        *steps += 1;
                    }
                    word.push(cs[i]);
                    i += 1;
                    *steps += 1;
                }
                if i >= n {
                    return None;
                }
                i += 1;
            }
            '$' if i + 1 < n && cs[i + 1] == '(' => return None,
            '$' if i + 1 < n && cs[i + 1] == '{' => {
                in_word = true;
                word.push_str("${");
                i += 2;
                let mut depth = 1usize;
                while i < n && depth > 0 {
                    let d = cs[i];
                    if d == '`' || (d == '$' && i + 1 < n && cs[i + 1] == '(') {
                        return None;
                    }
                    if d == '{' {
                        depth += 1;
                    } else if d == '}' {
                        depth -= 1;
                    }
                    word.push(d);
                    i += 1;
                    *steps += 1;
                }
                if depth > 0 {
                    return None;
                }
            }
            '&' if i + 1 < n && cs[i + 1] == '>' => {
                // `&>` / `&>>`: both streams — to `/dev/null` only
                end_word!();
                if to_devnull {
                    return None; // the previous redirection never got its target
                }
                i += 2;
                if i < n && cs[i] == '>' {
                    i += 1;
                }
                to_devnull = true;
            }
            '`' | ';' | '&' | '|' | '(' | ')' | '<' => return None,
            '>' => {
                // an fd number right before it (`2>`) belongs to the redirection
                if in_word && !word.is_empty() && word.chars().all(|d| d.is_ascii_digit()) {
                    word.clear();
                    in_word = false;
                }
                end_word!();
                if to_devnull {
                    return None; // the previous redirection never got its target (`> 2>…`)
                }
                i += 1;
                if i < n && cs[i] == '(' {
                    return None; // `>(…)`
                }
                if i < n && (cs[i] == '>' || cs[i] == '|') {
                    i += 1;
                }
                if i < n && cs[i] == '&' {
                    // a descriptor copy: `>&2`, `2>&1` (digits only — `>&-` and `>&file` are not)
                    i += 1;
                    let start = i;
                    while i < n && cs[i].is_ascii_digit() {
                        i += 1;
                        *steps += 1;
                    }
                    if i == start {
                        return None;
                    }
                } else {
                    to_devnull = true;
                }
            }
            _ => {
                in_word = true;
                word.push(c);
                i += 1;
            }
        }
    }
    if in_word {
        if to_devnull {
            return (word == "/dev/null" && !words.is_empty()).then_some(words);
        }
        words.push(word);
    }
    (!to_devnull && !words.is_empty()).then_some(words)
}

/// The next word as a subcommand (`None` for a flag or nothing).
fn sub_at(words: &[String], k: usize) -> Option<String> {
    words
        .get(k)
        .filter(|w| !w.is_empty() && !w.starts_with('-'))
        .map(|w| w.chars().take(64).collect())
}

/// The script operand of an interpreter at `words[k]` (`None` = runs inline code / a module / nothing).
fn interpreter_operand(prog: &str, words: &[String], mut k: usize) -> Option<usize> {
    let base = prog.rsplit('/').next().unwrap_or(prog);
    let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    let is_python = base == "py"
        || base
            .strip_prefix("python")
            .is_some_and(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.'));
    let is_shell = matches!(base, "sh" | "bash" | "zsh");
    let is_node = base == "node";
    let is_pwsh = base.eq_ignore_ascii_case("pwsh");
    if !(is_python || is_shell || is_node || is_pwsh) {
        return None;
    }
    while let Some(w) = words.get(k) {
        if w == "--" {
            return (k + 1 < words.len()).then_some(k + 1);
        }
        let flag = w.starts_with('-') || (is_shell && w.starts_with('+'));
        if !flag {
            return Some(k);
        }
        let low = w.to_ascii_lowercase();
        if is_python {
            let letters = &w[1..];
            if !w.starts_with("--") && (letters.contains('c') || letters.contains('m')) {
                return None; // `-c code` / `-m module`
            }
            if w == "-W" || w == "-X" {
                k += 1;
            }
        } else if is_shell {
            if !w.starts_with("--") && w[1..].contains('c') {
                return None; // `-c string`
            }
            if matches!(w.as_str(), "-o" | "+o" | "-O" | "+O") {
                k += 1;
            }
        } else if is_node {
            if matches!(
                w.as_str(),
                "-e" | "--eval" | "-p" | "--print" | "-r" | "--require" | "--import" | "--loader" | "--experimental-loader"
            ) {
                return None; // inline code or a preloaded module (RULINGS BS1: flags that take no code)
            }
            if matches!(w.as_str(), "-C" | "--conditions") {
                k += 1;
            }
        } else {
            if matches!(low.as_str(), "-file" | "-f") {
                return (k + 1 < words.len()).then_some(k + 1);
            }
            if matches!(low.as_str(), "-command" | "-c" | "-encodedcommand" | "-e" | "-ec") {
                return None;
            }
            if matches!(
                low.as_str(),
                "-executionpolicy" | "-ex" | "-ep" | "-workingdirectory" | "-wd" | "-configurationname" | "-outputformat" | "-of" | "-inputformat" | "-if" | "-windowstyle" | "-w" | "-version" | "-v" | "-settingsfile" | "-custompipename"
            ) {
                k += 1;
            }
        }
        k += 1;
    }
    None
}

/// The identity of a shell call (RULINGS BF1, BS1): given ONLY when the call's whole text is ONE simple
/// command — optional `NAME=value` assignments (the value holding no `${`; `$(` and backticks are
/// refused anywhere), optionally ONE interpreter (`python*` / `py` / `sh` / `bash` / `zsh` / `node` /
/// `pwsh`, with flags that take no code), then a hook / script of THIS lane's pack and its arguments
/// (`cys-dept` run through PATH is the pack's `bin/cys-dept`). Every compound call is a stated miss
/// (`cd dir && <script>` included — DEFERRED #76). Linear in the command's length (RULINGS BF5).
pub fn pack_call(tool_input: &Value, roots: &PackRoots) -> PackCall {
    pack_call_counted(tool_input, roots).0
}

/// [`pack_call`] and the number of steps it took (the linear-cost pin of RULINGS BF5).
pub fn pack_call_counted(tool_input: &Value, roots: &PackRoots) -> (PackCall, usize) {
    let mut steps = 0usize;
    let Some(cmd) = shell_command(tool_input) else {
        return (PackCall::None, steps);
    };
    let cmd = cut_at_char_boundary(&cmd, COMMAND_SCAN_MAX_BYTES);
    let Some(words) = single_simple_command(cmd, &mut steps) else {
        return (PackCall::None, steps);
    };
    let mut k = 0;
    while let Some(w) = words.get(k).filter(|w| is_env_assignment(w)) {
        if w.contains("${") {
            return (PackCall::None, steps); // an assignment that expands (RULINGS BS1)
        }
        k += 1;
    }
    let Some(prog) = words.get(k) else {
        return (PackCall::None, steps);
    };
    if let Some(rel) = roots.rel_of(prog, &mut steps) {
        return (PackCall::One { rel, sub: sub_at(&words, k + 1) }, steps);
    }
    if prog == "cys-dept" || prog.ends_with("/.local/bin/cys-dept") {
        return (PackCall::One { rel: "bin/cys-dept".into(), sub: sub_at(&words, k + 1) }, steps);
    }
    if let Some(op) = interpreter_operand(prog, &words, k + 1) {
        if let Some(rel) = roots.rel_of(&words[op], &mut steps) {
            return (PackCall::One { rel, sub: sub_at(&words, op + 1) }, steps);
        }
    }
    (PackCall::None, steps)
}

/// What one observation is, decided WITHOUT the detector's state (so it runs before the lock).
#[derive(Clone, Debug, PartialEq)]
pub enum Classified {
    /// Not a call / failure of this lane's pack script (counted when it was a failure).
    Ignored { failure: bool },
    Call { rel: String },
    Excluded { reason: &'static str },
    /// A failure of `rel` without a text → it counts toward the rate but forms no signature (O-C1-9).
    FailureNoText { rel: String },
    Failure { report: cys::repair::intake::Report },
}

fn is_failure(o: &ToolObservation<'_>) -> bool {
    !o.interrupted
        && (o.raw_event == "PostToolUseFailure"
            || o.event_type == "PostToolUseFailure"
            || (o.event_type == "POST_TOOL" && o.exit_code.is_some_and(|c| c != 0)))
}

/// Keeps a failure text bounded: the whole text up to [`ERROR_KEEP_BYTES`], else its head and its tail.
fn bounded_error(e: &str) -> std::borrow::Cow<'_, str> {
    if e.len() <= ERROR_KEEP_BYTES {
        return std::borrow::Cow::Borrowed(e);
    }
    let head = cut_at_char_boundary(e, ERROR_KEEP_BYTES / 2);
    let mut t = e.len() - ERROR_KEEP_BYTES / 2;
    while !e.is_char_boundary(t) {
        t += 1;
    }
    std::borrow::Cow::Owned(format!("{head}\n…\n{}", &e[t..]))
}

/// ① part 1 — PURE and lock-free: identity, failure or call, exclusion, signature.
pub fn classify(o: &ToolObservation<'_>, roots: &PackRoots, windows: bool) -> Classified {
    let failure = is_failure(o);
    let call = !failure && !o.interrupted && o.event_type == "PRE_TOOL";
    if (!failure && !call) || o.tool_name != "Bash" {
        return Classified::Ignored { failure };
    }
    let (rel, sub) = match pack_call(o.tool_input, roots) {
        PackCall::None => return Classified::Ignored { failure },
        PackCall::One { rel, sub } => (rel, sub),
    };
    if call {
        return Classified::Call { rel };
    }
    let raw = o.error.map(bounded_error).filter(|e| !e.trim().is_empty());
    if let Some(reason) = detector_excluded(&rel, sub.as_deref(), raw.as_deref().unwrap_or(""), windows) {
        return Classified::Excluded { reason };
    }
    match raw.as_deref().map(|e| cys::repair::intake::parse_report(&rel, e)) {
        Some(Ok(report)) => Classified::Failure { report },
        _ => Classified::FailureNoText { rel },
    }
}

/// One script's calls and failures per day (the rule's window — RULINGS BF4).
#[derive(Clone, Debug, Default)]
struct RelStats {
    days: std::collections::VecDeque<(i64, u64, u64)>,
    last_seen: f64,
}

fn day_of(t: f64) -> i64 {
    (t / 86_400.0).floor() as i64
}

impl RelStats {
    fn add(&mut self, now: f64, calls: u64, fails: u64) {
        let d = day_of(now);
        while self.days.front().is_some_and(|(x, _, _)| *x <= d - DETECTOR_WINDOW_DAYS) {
            self.days.pop_front();
        }
        match self.days.back_mut() {
            Some((x, c, f)) if *x == d => {
                *c = c.saturating_add(calls);
                *f = f.saturating_add(fails);
            }
            _ => self.days.push_back((d, calls, fails)),
        }
        self.last_seen = self.last_seen.max(now);
    }

    /// (calls, failures) inside the window.
    fn totals(&self, now: f64) -> (u64, u64) {
        let d = day_of(now);
        self.days
            .iter()
            .filter(|(x, _, _)| *x > d - DETECTOR_WINDOW_DAYS)
            .fold((0, 0), |(c, f), (_, x, y)| (c + x, f + y))
    }
}

#[derive(Clone, Debug)]
struct SigStats {
    report: cys::repair::intake::Report,
    /// Failures seen and not yet in the row.
    unflushed: u64,
    first_seen: f64,
    last_seen: f64,
    /// A row of this signature was written by this daemon incarnation.
    written: bool,
}

/// The detector's counters (memory only).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DetectorCounters {
    pub calls: u64,
    pub failures: u64,
    pub failures_without_identity: u64,
    pub no_error_text: u64,
    pub excluded: u64,
    pub excluded_by_reason: std::collections::BTreeMap<&'static str, u64>,
    pub evicted_signatures: u64,
    pub evicted_pending_signatures: u64,
    pub evicted_rels: u64,
    pub rows_new: u64,
    pub rows_updated: u64,
    pub corrupt_rows: u64,
    pub write_failed: u64,
}

/// The detector's state for one lane daemon (memory only — a restart starts it again, §9).
#[derive(Debug, Default)]
pub struct Detector {
    rels: HashMap<String, RelStats>,
    sigs: HashMap<String, SigStats>,
    /// The rule's thresholds, refreshed by the flusher from alerts-config.json (no I/O on the hook path).
    cfg: AlertConfig,
    pub counters: DetectorCounters,
}

/// What one observation did (for the tests and the counters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observed {
    Ignored,
    Call,
    /// A failure with a signature, counted for its row.
    Failure,
    /// Counted toward the script's failure rate, but no text → no signature (O-C1-9) → no row.
    NoErrorText,
    /// On the exclusion list (the reason is counted in `DetectorCounters::excluded_by_reason`).
    Excluded,
}

/// The repeated-failure rule of `evaluate` §3, per IDENTITY, inside the window: failures ≥ `fail_count`,
/// calls ≥ `fail_min_calls`, failure rate ≥ `fail_rate` (calls = max(PRE_TOOL count, failures)).
fn rel_meets(r: &RelStats, cfg: &AlertConfig, now: f64) -> bool {
    let (calls, fails) = r.totals(now);
    let calls = calls.max(fails);
    fails >= cfg.fail_count && calls >= cfg.fail_min_calls && calls > 0 && (fails as f64 / calls as f64) >= cfg.fail_rate
}

impl Detector {
    fn is_due(&self, s: &SigStats, now: f64) -> bool {
        s.unflushed > 0 && (s.written || self.rels.get(&s.report.rel).is_some_and(|r| rel_meets(r, &self.cfg, now)))
    }

    /// Room for one more script: evict the idlest script that has no due signature, else the idlest.
    fn room_for_rel(&mut self, now: f64) {
        if self.rels.len() < DETECTOR_MAX_RELS {
            return;
        }
        let busy: std::collections::HashSet<&str> = self
            .sigs
            .values()
            .filter(|s| self.is_due(s, now))
            .map(|s| s.report.rel.as_str())
            .collect();
        let pick = |only_idle: bool| {
            self.rels
                .iter()
                .filter(|(k, _)| !only_idle || !busy.contains(k.as_str()))
                .min_by(|a, b| a.1.last_seen.total_cmp(&b.1.last_seen).then_with(|| a.0.cmp(b.0)))
                .map(|(k, _)| k.clone())
        };
        if let Some(k) = pick(true).or_else(|| pick(false)) {
            self.rels.remove(&k);
            self.counters.evicted_rels += 1;
        }
    }

    /// Room for one more signature: evict the oldest that is not due, else the oldest (RULINGS BF4 —
    /// a full table never blinds the detector: a NEW defect is always recorded).
    fn room_for_sig(&mut self, now: f64) {
        if self.sigs.len() < DETECTOR_MAX_SIGNATURES {
            return;
        }
        let pick = |only_not_due: bool| {
            self.sigs
                .iter()
                .filter(|(_, s)| !only_not_due || !self.is_due(s, now))
                .min_by(|a, b| a.1.last_seen.total_cmp(&b.1.last_seen).then_with(|| a.0.cmp(b.0)))
                .map(|(k, s)| (k.clone(), s.unflushed > 0))
        };
        if let Some((k, pending)) = pick(true).or_else(|| pick(false)) {
            self.sigs.remove(&k);
            self.counters.evicted_signatures += 1;
            if pending {
                self.counters.evicted_pending_signatures += 1;
            }
        }
    }

    fn bump_rel(&mut self, rel: &str, now: f64, calls: u64, fails: u64) {
        if !self.rels.contains_key(rel) {
            self.room_for_rel(now);
        }
        self.rels.entry(rel.to_string()).or_default().add(now, calls, fails);
    }

    /// Tracked sizes (rels, signatures, signatures with a pending count).
    pub fn tracked(&self) -> (usize, usize, usize) {
        let pending = self.sigs.values().filter(|s| s.unflushed > 0).count();
        (self.rels.len(), self.sigs.len(), pending)
    }
}

/// ① part 2 — the short update under the detector's lock (bounded: map updates; a table scan of at most
/// [`DETECTOR_MAX_SIGNATURES`] + [`DETECTOR_MAX_RELS`] entries only when a table is full).
pub fn record(det: &mut Detector, c: Classified, now: f64) -> Observed {
    match c {
        Classified::Ignored { failure } => {
            if failure {
                det.counters.failures_without_identity += 1;
            }
            Observed::Ignored
        }
        Classified::Call { rel } => {
            det.bump_rel(&rel, now, 1, 0);
            det.counters.calls += 1;
            Observed::Call
        }
        Classified::Excluded { reason } => {
            det.counters.failures += 1;
            det.counters.excluded += 1;
            *det.counters.excluded_by_reason.entry(reason).or_insert(0) += 1;
            Observed::Excluded
        }
        Classified::FailureNoText { rel } => {
            det.counters.failures += 1;
            det.counters.no_error_text += 1;
            det.bump_rel(&rel, now, 0, 1);
            Observed::NoErrorText
        }
        Classified::Failure { report } => {
            det.counters.failures += 1;
            det.bump_rel(&report.rel, now, 0, 1);
            if let Some(s) = det.sigs.get_mut(&report.signature) {
                s.unflushed = s.unflushed.saturating_add(1);
                s.last_seen = now;
                return Observed::Failure;
            }
            det.room_for_sig(now);
            det.sigs.insert(
                report.signature.clone(),
                SigStats { report, unflushed: 1, first_seen: now, last_seen: now, written: false },
            );
            Observed::Failure
        }
    }
}

/// `classify` + `record` in one call (tests).
#[cfg(test)]
pub fn observe(det: &mut Detector, o: &ToolObservation<'_>, roots: &PackRoots, now: f64, windows: bool) -> Observed {
    let c = classify(o, roots, windows);
    record(det, c, now)
}

/// One row write the flusher owes.
#[derive(Clone, Debug, PartialEq)]
pub struct DueRow {
    pub signature: String,
    pub report: cys::repair::intake::Report,
    pub delta: u64,
    pub first_seen: f64,
    pub last_seen: f64,
}

/// The signatures whose script meets the rule inside the window (or that this incarnation already wrote)
/// and that have a pending count — sorted by signature (determinism).
pub fn due_signatures(det: &Detector, now: f64) -> Vec<DueRow> {
    let mut out: Vec<DueRow> = det
        .sigs
        .iter()
        .filter(|(_, s)| det.is_due(s, now))
        .map(|(k, s)| DueRow {
            signature: k.clone(),
            report: s.report.clone(),
            delta: s.unflushed,
            first_seen: s.first_seen,
            last_seen: s.last_seen,
        })
        .collect();
    out.sort_by(|a, b| a.signature.cmp(&b.signature));
    out
}

/// One daemon's detector + its own quota share.
#[derive(Debug, Default)]
pub struct DefectIntake {
    pub det: Mutex<Detector>,
    pub share: Mutex<crate::alert_route::IntakeShare>,
    /// This lane's pack-root spellings (the identity's "this lane"), built once.
    roots: PackRoots,
    /// When the flusher last logged this lane's counters (`Now::mono`; throttled to one line per
    /// [`crate::alert_route::INTAKE_ROW_COOLDOWN_SECS`]).
    last_log_mono: Mutex<Option<f64>>,
}

type IntakeRegistry = Mutex<HashMap<std::path::PathBuf, (std::sync::Weak<Daemon>, Arc<DefectIntake>)>>;

fn intake_registry() -> &'static IntakeRegistry {
    static R: OnceLock<IntakeRegistry> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// This daemon's detector (one per daemon, keyed by its socket; a dead daemon's entry is dropped).
pub fn intake_for(daemon: &Arc<Daemon>) -> Arc<DefectIntake> {
    let mut g = intake_registry().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((w, d)) = g.get(&daemon.socket_path) {
        if w.strong_count() > 0 && std::sync::Weak::ptr_eq(w, &Arc::downgrade(daemon)) {
            return Arc::clone(d);
        }
    }
    g.retain(|_, (w, _)| w.strong_count() > 0);
    let d = Arc::new(DefectIntake {
        roots: production_roots(),
        ..DefectIntake::default()
    });
    g.insert(daemon.socket_path.clone(), (Arc::downgrade(daemon), Arc::clone(&d)));
    d
}

/// The daemons the flusher serves (the live entries of the registry).
fn registered_lanes() -> Vec<std::sync::Weak<Daemon>> {
    let mut g = intake_registry().lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|_, (w, _)| w.strong_count() > 0);
    g.values().map(|(w, _)| w.clone()).collect()
}

#[cfg(not(test))]
fn production_roots() -> PackRoots {
    let pd = cys::pack::pack_dir();
    let home = cys::home_dir();
    PackRoots::new(Some(&pd.to_string_lossy()), Some(&home.to_string_lossy()))
}

#[cfg(test)]
fn production_roots() -> PackRoots {
    // tests never resolve the real pack dir or home: a fixed, fictional base-lane spelling
    PackRoots::new(Some("/Users/x/.cys/pack"), Some("/Users/x"))
}

fn detector_enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| crate::alert_route::enabled_from(std::env::var(DEFECT_INTAKE_ENV).ok().as_deref()))
}

/// How the flusher is started: once per process (`once`), by `start` (RULINGS BF6 — injectable).
pub struct FlusherStart<'a> {
    pub once: &'a OnceLock<()>,
    pub start: &'a dyn Fn(),
}

/// ① The `usage.event` arm's call — never fails the request, never panics out.
pub fn observe_tool_event(daemon: &Arc<Daemon>, o: &ToolObservation<'_>) {
    static FLUSHER: OnceLock<()> = OnceLock::new();
    observe_tool_event_with(daemon, o, &FlusherStart { once: &FLUSHER, start: &start_flusher_thread });
}

/// [`observe_tool_event`] with the flusher's start injected. The command is classified BEFORE the
/// detector's lock is taken (RULINGS BF5); under the lock only the in-memory tables change.
pub fn observe_tool_event_with(daemon: &Arc<Daemon>, o: &ToolObservation<'_>, flusher: &FlusherStart<'_>) {
    if !detector_enabled() || o.tool_input.get("command").is_none() {
        return;
    }
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let intake = intake_for(daemon);
        let c = classify(o, &intake.roots, cfg!(windows));
        if c == (Classified::Ignored { failure: false }) {
            return;
        }
        let now = crate::state::now_epoch();
        let r = {
            let mut d = intake.det.lock().unwrap_or_else(|e| e.into_inner());
            record(&mut d, c, now)
        };
        if r == Observed::Failure {
            flusher.once.get_or_init(|| (flusher.start)());
        }
    }));
}

/// What one flush did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlushReport {
    pub due: usize,
    pub written_new: usize,
    pub updated: usize,
    pub held_cooldown: usize,
    pub held_hourly: usize,
    pub corrupt: usize,
    pub failed: usize,
}

enum RowOutcome {
    Written { new_row: bool },
    Held(crate::alert_route::IntakeVerdict),
    Corrupt,
    Failed,
}

/// T3's per-file mutex for intake rows (`ledger_verbs::report` takes the same key in the same map):
/// one writer per row file inside this daemon.
fn with_row_mutex<T>(daemon: &Daemon, sig: &str, f: impl FnOnce() -> T) -> T {
    let m = {
        let mut map = daemon.repair_mutexes.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(
            map.entry(format!("intake:{sig}"))
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    };
    let _g = m.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// ② Writes the rows the detector owes, under the intake's OWN quota. Runs on the flusher thread (and in
/// tests). Writes only THIS lane's rows (G4); a corrupt row is never overwritten (O-C1-6) — its count
/// stays pending. Nothing is queued, pushed, published or alerted. `cfg` (alerts-config.json, read by the
/// flusher) also becomes the thresholds the hook path's table eviction uses.
pub fn flush_defect_intake(daemon: &Arc<Daemon>, root: &std::path::Path, cfg: &AlertConfig, now: crate::alert_route::Now) -> FlushReport {
    use crate::alert_route::IntakeVerdict;
    use cys::repair::intake::{load_row, row_path_in, IntakeRow, MAX_REPORTERS};
    let lane = cys::repair::types::LaneKey::of_socket(&daemon.socket_path);
    let intake = intake_for(daemon);
    let due = {
        let mut d = intake.det.lock().unwrap_or_else(|e| e.into_inner());
        d.cfg = cfg.clone();
        due_signatures(&d, now.epoch)
    };
    let mut rep = FlushReport {
        due: due.len(),
        ..FlushReport::default()
    };
    for row in due {
        let path = row_path_in(root, &lane, &row.signature);
        let outcome = with_row_mutex(daemon, &row.signature, || {
            let existing = match load_row(&path) {
                Ok(x) => x,
                Err(_) => return RowOutcome::Corrupt,
            };
            let new_row = existing.is_none();
            let verdict = intake
                .share
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .decide(&row.signature, new_row, now.mono);
            if verdict != IntakeVerdict::Write {
                intake.share.lock().unwrap_or_else(|e| e.into_inner()).note_held(verdict);
                return RowOutcome::Held(verdict);
            }
            let out = match existing {
                Some(mut r) => {
                    r.count = r.count.saturating_add(row.delta);
                    r.last_seen = r.last_seen.max(row.last_seen);
                    if !r.reporters.iter().any(|x| x == DETECTOR_REPORTER) && r.reporters.len() < MAX_REPORTERS {
                        r.reporters.push(DETECTOR_REPORTER.to_string());
                    }
                    r
                }
                None => IntakeRow {
                    schema: cys::repair::REPAIR_SCHEMA,
                    signature: row.signature.clone(),
                    lane: lane.clone(),
                    rel: row.report.rel.clone(),
                    error_class: row.report.error_class.clone(),
                    first_line: row.report.first_line.clone(),
                    first_seen: row.first_seen,
                    last_seen: row.last_seen,
                    count: row.delta,
                    reporters: vec![DETECTOR_REPORTER.to_string()],
                },
            };
            match cys::atomic_write_json(&path, &out) {
                Ok(()) => {
                    intake
                        .share
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .commit(&row.signature, new_row, now.mono);
                    RowOutcome::Written { new_row }
                }
                Err(_) => RowOutcome::Failed,
            }
        });
        let mut d = intake.det.lock().unwrap_or_else(|e| e.into_inner());
        match outcome {
            RowOutcome::Written { new_row } => {
                if let Some(s) = d.sigs.get_mut(&row.signature) {
                    s.unflushed = s.unflushed.saturating_sub(row.delta);
                    s.written = true;
                }
                if new_row {
                    d.counters.rows_new += 1;
                    rep.written_new += 1;
                } else {
                    d.counters.rows_updated += 1;
                    rep.updated += 1;
                }
            }
            RowOutcome::Held(IntakeVerdict::HoldHourly) => rep.held_hourly += 1,
            RowOutcome::Held(_) => rep.held_cooldown += 1,
            RowOutcome::Corrupt => {
                d.counters.corrupt_rows += 1;
                rep.corrupt += 1;
            }
            RowOutcome::Failed => {
                d.counters.write_failed += 1;
                rep.failed += 1;
            }
        }
    }
    rep
}

/// The detector's and the share's counters (the daemon log line; tests).
pub fn defect_intake_snapshot(daemon: &Arc<Daemon>, mono: f64) -> Value {
    let intake = intake_for(daemon);
    let (c, tracked) = {
        let d = intake.det.lock().unwrap_or_else(|e| e.into_inner());
        (d.counters.clone(), d.tracked())
    };
    let share = intake.share.lock().unwrap_or_else(|e| e.into_inner()).snapshot(mono);
    json!({
        "calls": c.calls, "failures": c.failures,
        "failures_without_identity": c.failures_without_identity,
        "no_error_text": c.no_error_text, "excluded": c.excluded,
        "excluded_by_reason": c.excluded_by_reason,
        "evicted_signatures": c.evicted_signatures, "evicted_pending_signatures": c.evicted_pending_signatures,
        "evicted_rels": c.evicted_rels,
        "rows_new": c.rows_new, "rows_updated": c.rows_updated,
        "corrupt_rows": c.corrupt_rows, "write_failed": c.write_failed,
        "tracked_rels": tracked.0, "tracked_signatures": tracked.1, "pending_signatures": tracked.2,
        "share": share,
    })
}

/// The ledger root the flusher writes under: production `~/.cys/local/repairs` (the same root as the
/// `repair.*` verbs); in this binary's tests a per-process temporary root (never a real ledger).
fn intake_root(daemon: &Daemon) -> std::path::PathBuf {
    #[cfg(test)]
    {
        std::env::temp_dir()
            .join(format!("cys-defect-intake-test-{}", std::process::id()))
            .join(cys::repair::types::LaneKey::of_socket(&daemon.socket_path).0)
    }
    #[cfg(not(test))]
    {
        let _ = daemon;
        cys::repair::paths::repairs_root()
    }
}

/// ONE pass of the flusher thread — its whole loop body (RULINGS BF6): the lanes, the thresholds and the
/// ledger root come from the injected sources (production: the registry, alerts-config.json,
/// [`intake_root`]); each lane is flushed under its own catch_unwind. Returns how many lanes were flushed.
pub fn flusher_pass(
    lanes: &dyn Fn() -> Vec<std::sync::Weak<Daemon>>,
    cfg: &dyn Fn() -> AlertConfig,
    root_of: &dyn Fn(&Daemon) -> std::path::PathBuf,
) -> usize {
    let lanes = lanes();
    if lanes.is_empty() {
        return 0;
    }
    let cfg = cfg();
    let mut n = 0;
    for w in &lanes {
        let Some(d) = w.upgrade() else { continue };
        let root = root_of(&d);
        let now = crate::alert_route::Now::live(&d);
        let flushed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let rep = flush_defect_intake(&d, &root, &cfg, now);
            log_flush(&d, &rep, now.mono);
        }));
        if flushed.is_ok() {
            n += 1;
        }
    }
    n
}

/// One stderr line (the daemon log) with this lane's counters when a flush wrote, held or failed
/// something — at most once per [`crate::alert_route::INTAKE_ROW_COOLDOWN_SECS`] per lane. A log line,
/// never a message: nothing reaches a queue or the event bus.
fn log_flush(daemon: &Arc<Daemon>, rep: &FlushReport, mono: f64) -> bool {
    let acted = rep.written_new + rep.updated + rep.held_cooldown + rep.held_hourly + rep.corrupt + rep.failed;
    if acted == 0 {
        return false;
    }
    let intake = intake_for(daemon);
    {
        let mut last = intake.last_log_mono.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| mono - t < crate::alert_route::INTAKE_ROW_COOLDOWN_SECS) {
            return false;
        }
        *last = Some(mono);
    }
    eprintln!("[cysd] defect-intake: due={} {}", rep.due, defect_intake_snapshot(daemon, mono));
    true
}

/// The production start of the flusher thread (`cysd-defect-intake`): a loop of [`flusher_pass`] every
/// [`FLUSH_TICK_SECS`]. Started lazily by the first identified failure — the daemon's start order is
/// untouched. In this binary's tests it starts nothing (they drive [`flusher_pass`] directly).
fn start_flusher_thread() {
    #[cfg(not(test))]
    {
        let spawned = std::thread::Builder::new()
            .name("cysd-defect-intake".into())
            .spawn(|| loop {
                std::thread::sleep(std::time::Duration::from_secs(FLUSH_TICK_SECS));
                let _ = flusher_pass(&registered_lanes, &AlertConfig::load, &intake_root);
            });
        if let Err(e) = spawned {
            eprintln!("[cysd] defect-intake: flusher thread not started ({e}) — rows are not written");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_partial_and_defaults() {
        assert_eq!(AlertConfig::from_value(&json!({})), AlertConfig::default());
        let c = AlertConfig::from_value(&json!({"rate_limit_pct": 80, "weekly_cost_usd": 50}));
        assert_eq!(c.rate_limit_pct, 80.0);
        assert_eq!(c.weekly_cost_usd, 50.0);
        assert_eq!(c.fail_count, 5, "빠진 키는 기본값");
    }

    #[test]
    fn evaluate_fires_expected_alerts() {
        let cfg = AlertConfig { weekly_cost_usd: 10.0, ..AlertConfig::default() };
        let snap = Snapshot {
            rates: vec![
                ("worker".into(), "5h".into(), 92.0),  // ≥90 warn
                ("master".into(), "7d".into(), 97.0),  // ≥95 crit
                ("cso".into(), "5h".into(), 50.0),     // 미발화
            ],
            account_rates: vec![
                ("a@b.c".into(), "5h".into(), 85.0),   // ≥80 warn
                ("a@b.c".into(), "7d".into(), 96.0),   // ≥95 crit
                ("x@y.z".into(), "5h".into(), 40.0),   // 미발화
            ],
            weekly_cost_usd: 12.0, // ≥10 → 발화
            weekly_tokens: 0,      // 한도 0 = 비활성
            tool_failures: vec![
                ("Bash".into(), 10, 6, 0.6),  // fail6≥5·calls10≥5·rate0.6≥0.3 → crit
                ("Edit".into(), 3, 3, 1.0),   // calls3<5(min) → 미발화
                ("Read".into(), 20, 2, 0.1),  // rate0.1<0.3 → 미발화
            ],
            // ★T-0147-2 A8 가산 필드 — 이 핀은 기존 3축(rate·budget·failure) 전용이라 배지 없음.
            gate_badges: Vec::new(),
            // ★0.14.43(B3) 가산 필드(관측 메타·신선도 억제 키)는 비어 있다 — 이 핀의 입력 모양은 종전 그대로다(구조체 리터럴에 기본값 꼬리만 붙였다).
            ..Snapshot::default()
        };
        let alerts = evaluate(&snap, &cfg);
        let keys: Vec<&str> = alerts.iter().map(|a| a.key.as_str()).collect();
        assert!(keys.contains(&"rate_limit:worker:5h"));
        assert!(keys.contains(&"rate_limit:master:7d"));
        assert!(!keys.contains(&"rate_limit:cso:5h"));
        // CC v2: 계정 축 — warn/crit 경계·미발화 확인
        assert!(keys.contains(&"account_rate:a@b.c:5h"));
        assert!(keys.contains(&"account_rate:a@b.c:7d"));
        assert!(!keys.contains(&"account_rate:x@y.z:5h"));
        assert!(alerts.iter().any(|a| a.key == "account_rate:a@b.c:7d" && a.severity == "crit"));
        assert!(alerts.iter().any(|a| a.key == "account_rate:a@b.c:5h" && a.severity == "warn"));
        assert!(keys.contains(&"weekly_budget:cost"));
        assert!(keys.contains(&"repeated_failure:Bash"));
        assert!(!keys.iter().any(|k| k.contains("Edit") || k.contains("Read")));
        // 심각도
        let crit: Vec<&str> = alerts.iter().filter(|a| a.severity == "crit").map(|a| a.key.as_str()).collect();
        assert!(crit.contains(&"rate_limit:master:7d") && crit.contains(&"repeated_failure:Bash"));
        // 결정론 정렬(key asc)
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    /// 락 순서 배선 핀(소스): 스냅샷은 신원 표(읽기-통과는 파일 IO 가 있다 · 락 없이)를 surfaces 락을 잡기 **전에** 만들고, 계정 축 입력(accounts 락)은 surfaces 락을 놓은 뒤에 읽는다.
    /// ★R1F-US: 스냅샷이 두 진입점(`snapshot_with_stale` 읽기-통과 · `snapshot_cached_with_stale` 캐시 전용)과 본체(`snapshot_from_view`)로 갈렸다 — 두 진입점이 각자 신원 표를 만든 **뒤에** 본체를 부르고,
    /// 본체는 신원 표를 인자로만 받는다(안에서 만들지 않는다). 계약(신원 표 → surfaces → accounts 순서)은 같다.
    #[test]
    fn b3_snapshot_builds_the_identity_view_before_taking_the_surfaces_lock() {
        let src = include_str!("alerts.rs");
        let func = |head: &str| -> String {
            let body = src.split(head).nth(1).unwrap_or_else(|| panic!("{head} 소실"));
            body[..body.find("\n}\n").unwrap_or_else(|| panic!("{head} 끝"))].to_string()
        };
        for head in ["pub fn snapshot_with_stale(", "pub fn snapshot_cached_with_stale("] {
            let entry = func(head);
            let view = entry.find("seat_identity_view").unwrap_or_else(|| panic!("{head}: 신원 표 호출"));
            let call = entry.find("snapshot_from_view(").unwrap_or_else(|| panic!("{head}: 본체 호출"));
            assert!(view < call, "{head}: 신원 표가 본체(surfaces 락을 잡는다) 호출 뒤에 만들어진다(파일 IO 가 락 안)");
            assert!(!entry.contains("daemon.surfaces.lock()"), "{head}: 진입점이 surfaces 락을 직접 쥔다");
        }
        let body = func("fn snapshot_from_view(");
        assert!(!body.contains("seat_identity_view"), "본체가 신원 표를 직접 만든다(surfaces 락 안의 파일 IO 위험)");
        let surfaces = body.find("daemon.surfaces.lock()").expect("surfaces 락");
        let accounts = body.find("alert_rates_with(").expect("계정 축 입력");
        let block_end = body[surfaces..].find("\n    }\n").map(|i| surfaces + i).expect("surfaces 블록 끝");
        assert!(block_end < accounts, "계정 축 입력(accounts 락)이 surfaces 락을 쥔 채 읽힌다");
    }

    // ═════════ ★0.14.43(B3) evaluate — 관측 메타가 detail 에 가산 키로 실린다(기존 키·문구·심각도 불변) ═════════
    #[test]
    fn b3_evaluate_adds_observation_meta_to_detail_and_keeps_the_old_keys() {
        let cfg = AlertConfig::default();
        let mut snap = Snapshot {
            rates: vec![("worker".into(), "5h".into(), 97.0)],
            account_rates: vec![("a@b.c".into(), "5h".into(), 99.0)],
            ..Snapshot::default()
        };
        let by = |v: &[Alert], k: &str| v.iter().find(|a| a.key == k).cloned().unwrap_or_else(|| panic!("키 {k} 없음: {v:?}"));
        // 메타 없음 → detail 은 종전 그대로(가산 키 0)
        let plain = evaluate(&snap, &cfg);
        assert_eq!(by(&plain, "account_rate:a@b.c:5h").detail, json!({"account": "a@b.c", "win": "5h", "used_pct": 99.0}));
        assert_eq!(by(&plain, "rate_limit:worker:5h").detail, json!({"role": "worker", "label": "5h", "used_pct": 97.0}));
        // 메타 있음 — 정수 초(나이는 내림) · 음수 리셋 가능 · 모르는 값은 null
        snap.account_meta.insert(
            ("a@b.c".into(), "5h".into()),
            RateMeta { observed_at: 1_000_000.5, age_secs: 1801.9, in_use: Some(false), resets_at: Some(1_005_400.0), reset_in_secs: Some(-5.0), held_secs: Some(12.7) },
        );
        snap.rate_meta.insert(
            ("worker".into(), "5h".into()),
            RateMeta { observed_at: 0.0, age_secs: 0.0, in_use: None, resets_at: None, reset_in_secs: None, held_secs: None },
        );
        let rich = evaluate(&snap, &cfg);
        let acct = by(&rich, "account_rate:a@b.c:5h");
        let d = &acct.detail;
        assert_eq!((d["account"].clone(), d["win"].clone(), d["used_pct"].clone()), (json!("a@b.c"), json!("5h"), json!(99.0)), "기존 detail 키가 바뀌었다");
        assert_eq!(d["observed_at"], json!(1_000_000.5));
        assert_eq!(d["age_secs"], json!(1801));
        assert_eq!(d["in_use"], json!(false));
        assert_eq!(d["reset_in_secs"], json!(-5));
        assert_eq!(d["held_secs"], json!(12));
        assert_eq!((acct.severity.as_str(), acct.message.as_str()), ("crit", "계정 a@b.c 5h rate 99%"), "심각도·문구 불변");
        let seat = by(&rich, "rate_limit:worker:5h").detail;
        for k in ["observed_at", "age_secs", "in_use", "reset_in_secs", "held_secs"] {
            assert!(seat[k].is_null(), "모르는 값 {k} 은 null 이어야 한다: {seat}");
        }
        assert_eq!((seat["role"].clone(), seat["label"].clone(), seat["used_pct"].clone()), (json!("worker"), json!("5h"), json!(97.0)));
        // 다른 키의 메타는 붙지 않는다(키 정확 일치)
        snap.rate_meta.clear();
        snap.account_meta.clear();
        snap.account_meta.insert(("a@b.c".into(), "7d".into()), RateMeta { observed_at: 5.0, age_secs: 1.0, in_use: Some(true), resets_at: None, reset_in_secs: None, held_secs: None });
        assert_eq!(by(&evaluate(&snap, &cfg), "account_rate:a@b.c:5h").detail, json!({"account": "a@b.c", "win": "5h", "used_pct": 99.0}));
    }

    #[test]
    fn severity_enum_maps_warn_crit() {
        use crate::severity::Severity;
        let warn = Alert {
            kind: "rate_limit".into(),
            key: "k".into(),
            severity: "warn".into(),
            message: String::new(),
            detail: json!({}),
        };
        let crit = Alert { severity: "crit".into(), ..warn.clone() };
        assert_eq!(warn.severity_enum(), Severity::Recoverable);
        assert_eq!(crit.severity_enum(), Severity::Critical);
        // 기존 String wire 필드 불변(외과적 — 파생자만 추가)
        assert_eq!(warn.severity, "warn");
        assert_eq!(crit.severity, "crit");
    }

    #[test]
    fn weekly_budget_disabled_when_zero() {
        let cfg = AlertConfig::default(); // weekly_cost_usd=0
        let snap = Snapshot { weekly_cost_usd: 9999.0, ..Snapshot::default() };
        assert!(evaluate(&snap, &cfg).is_empty(), "한도 0이면 비용 경보 비활성");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ★T-0147-2 §3 Phase1 A8 — 델타게이트 배지
    // 테스트는 전부 std::env::temp_dir() 격리 디렉터리다(라이브 ~/.cys/state 무접촉).
    // env 노브도 건드리지 않는다 — 병렬 테스트 프로세스 전역 오염 차단(기본값으로만 검증).
    // ─────────────────────────────────────────────────────────────────────────

    fn badge_tmpdir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::AtomicU64;
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "cys-gatebadge-{}-{}-{}",
            tag,
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_lane(root: &std::path::Path, dirname: &str, body: &str) {
        let lane = root.join(dirname);
        std::fs::create_dir_all(&lane).unwrap();
        std::fs::write(lane.join("badges.json"), body).unwrap();
    }

    #[test]
    fn collect_gate_badges_reads_two_lanes_with_lane_prefixed_keys() {
        let root = badge_tmpdir("two-lanes");
        let now = 1_000_000.0;
        write_lane(
            &root,
            "report_gate",
            &json!({"schema_version":1,"updated_at":now-5.0,"lane":"base",
                    "badges":[{"key":"quiet","severity":"info","message":"정상 대기",
                               "detail":{"nodes":4}}]})
            .to_string(),
        );
        write_lane(
            &root,
            "report_gate-dept-2",
            &json!({"schema_version":1,"updated_at":now-5.0,"lane":"dept-2",
                    "badges":[{"key":"quiet","severity":"warn","message":"라벨 미조인"}]})
            .to_string(),
        );
        // 레인 아닌 디렉터리·파일은 무시돼야 한다(state_dir 에는 온갖 것이 산다).
        std::fs::create_dir_all(root.join("transcripts")).unwrap();
        std::fs::write(root.join("report_gate.json"), "{}").unwrap(); // 디렉터리 아님
        let got = collect_gate_badges(&root, now);
        let keys: Vec<&str> = got.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["base/quiet", "dept-2/quiet"],
            "레인 접두로 같은 badge key 가 충돌하지 않아야 한다 + key 오름차순 결정론"
        );
        assert_eq!(got[0].severity, "info");
        assert_eq!(got[0].detail["nodes"].as_u64(), Some(4));
        assert_eq!(got[1].message, "라벨 미조인");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_gate_badges_deadman_replaces_stale_lane() {
        let root = badge_tmpdir("stale");
        let now = 1_000_000.0;
        // 기본 stale 임계 1800s 를 크게 넘긴 갱신 시각 — 배지 내용은 "quiet" 지만 믿으면 암전이다.
        write_lane(
            &root,
            "report_gate",
            &json!({"schema_version":1,"updated_at":now-7200.0,"lane":"base",
                    "badges":[{"key":"quiet","severity":"info","message":"정상 대기"}]})
            .to_string(),
        );
        let got = collect_gate_badges(&root, now);
        assert_eq!(got.len(), 1, "stale 레인은 배지 대신 데드맨 1개만");
        assert_eq!(got[0].key, "base/gate-stale");
        assert_eq!(got[0].severity, "warn");
        assert_eq!(got[0].detail["age_secs"].as_u64(), Some(7200));
        assert!(got[0].message.contains("데드맨"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_gate_badges_surfaces_corrupt_json_and_skips_missing() {
        let root = badge_tmpdir("corrupt");
        let now = 1_000_000.0;
        write_lane(&root, "report_gate", "{이건 json 이 아니다");
        // 게이트 미설치 레인(badges.json 부재) — 조용히 skip 이 정상(설치 안 한 부서 상시경보 방지).
        std::fs::create_dir_all(root.join("report_gate-dept-9")).unwrap();
        let got = collect_gate_badges(&root, now);
        assert_eq!(got.len(), 1, "부패 1건만 노출, 미설치 레인은 무발화");
        assert_eq!(got[0].key, "report_gate/badges-corrupt");
        assert_eq!(got[0].severity, "warn");
        // state_dir 자체가 없어도 패닉 없이 빈 벡터
        assert!(collect_gate_badges(&root.join("nope"), now).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn evaluate_maps_gate_badges_to_node_liveness_alerts_in_sorted_order() {
        let snap = Snapshot {
            gate_badges: vec![
                GateBadge {
                    key: "base/node-dead".into(),
                    severity: "critical".into(),
                    message: "노드 사망".into(),
                    detail: json!({"role": "worker"}),
                },
                GateBadge {
                    key: "base/aaa-warn".into(),
                    severity: "warn".into(),
                    message: "라벨 미조인".into(),
                    detail: json!({}),
                },
                GateBadge {
                    key: "base/bbb-info".into(),
                    severity: "info".into(),
                    message: "정상 대기".into(),
                    detail: json!({}),
                },
                GateBadge {
                    key: "base/ccc-unknown".into(),
                    severity: "보라색".into(), // 미지 어휘
                    message: "?".into(),
                    detail: json!({}),
                },
            ],
            ..Snapshot::default()
        };
        let alerts = evaluate(&snap, &AlertConfig::default());
        let keys: Vec<&str> = alerts.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "node_liveness:base/aaa-warn",
                "node_liveness:base/ccc-unknown",
                "node_liveness:base/node-dead",
            ],
            "정렬 전에 push 돼야 key 오름차순 결정론이 유지된다"
        );
        let sev = |k: &str| {
            alerts.iter().find(|a| a.key == k).map(|a| a.severity.clone()).unwrap()
        };
        assert_eq!(sev("node_liveness:base/node-dead"), "crit", "critical→crit");
        assert_eq!(sev("node_liveness:base/aaa-warn"), "warn");
        // ★info 는 경보로 승격하지 않는다 — 게이트가 매 주기 쓰는 `quiet` 배지가 경보가 되면
        //   정상 운영 중 상시 경고가 켜진다(T-0147-2 가 없애려는 오발화 그 자체).
        assert!(
            !keys.contains(&"node_liveness:base/bbb-info"),
            "info 배지가 경보로 승격됐다(정상 대기의 상시 경고 = 오발화 재유입)"
        );
        assert_eq!(sev("node_liveness:base/ccc-unknown"), "warn", "미지 어휘도 warn 으로 좁힘");
        assert!(alerts.iter().all(|a| a.kind == "node_liveness"));
        // message/detail 은 배지 그대로(데몬은 운반자)
        let dead = alerts.iter().find(|a| a.key == "node_liveness:base/node-dead").unwrap();
        assert_eq!(dead.message, "노드 사망");
        assert_eq!(dead.detail["role"].as_str(), Some("worker"));
    }

    #[test]
    fn gate_signal_badges_ttl_boundary_and_severity() {
        let mut m: HashMap<String, (f64, Value)> = HashMap::new();
        m.insert("gate.state_unwritable".into(), (900.0, json!({"job_id": "j1"})));
        m.insert("gate.other_thing".into(), (900.0, json!({"job_id": "j2"})));
        // TTL 내(age 100 < 1800)
        let got = gate_signal_badges_from(&m, 1000.0, 1800.0);
        let keys: Vec<&str> = got.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["signal/gate.other_thing", "signal/gate.state_unwritable"],
            "key 오름차순 결정론"
        );
        let sev = |k: &str| got.iter().find(|b| b.key == k).map(|b| b.severity.clone()).unwrap();
        assert_eq!(sev("signal/gate.state_unwritable"), "critical");
        assert_eq!(sev("signal/gate.other_thing"), "warn");
        let sig = got.iter().find(|b| b.key == "signal/gate.state_unwritable").unwrap();
        assert_eq!(sig.detail["job_id"].as_str(), Some("j1"));
        assert_eq!(sig.detail["age_secs"].as_u64(), Some(100), "age_secs 가산");
        assert!(sig.message.contains("state 외부 oracle"));
        // TTL 경계: age == ttl 은 만료(엄격 미만)
        assert!(gate_signal_badges_from(&m, 900.0 + 1800.0, 1800.0).is_empty());
        assert_eq!(gate_signal_badges_from(&m, 900.0 + 1799.0, 1800.0).len(), 2);
        // ttl 0 = 비활성(전부 만료)
        assert!(gate_signal_badges_from(&m, 900.0, 0.0).is_empty());
    }

    #[test]
    fn note_gate_signal_is_visible_through_public_reader() {
        // 프로세스 전역 static 을 쓰므로 **존재만** 단언한다(다른 테스트가 넣은 항목의
        // 부재를 단언하면 병렬 실행에서 깨진다 — 이 파일 테스트는 서로 격리돼야 한다).
        let name = format!("gate.probe_{}", std::process::id());
        note_gate_signal(&name, json!({"job_id": "probe"}));
        let now = crate::state::now_epoch();
        let got = gate_signal_badges(now);
        let mine = got
            .iter()
            .find(|b| b.key == format!("signal/{name}"))
            .expect("흡수한 신호는 공개 reader 로 보여야 한다");
        assert_eq!(mine.severity, "warn");
        assert_eq!(mine.detail["job_id"].as_str(), Some("probe"));
        // 비객체 detail 도 정보 유실 없이 감싸진다
        let name2 = format!("gate.probe2_{}", std::process::id());
        note_gate_signal(&name2, json!("문자열 detail"));
        let got2 = gate_signal_badges(crate::state::now_epoch());
        let m2 = got2.iter().find(|b| b.key == format!("signal/{name2}")).unwrap();
        assert_eq!(m2.detail["detail"].as_str(), Some("문자열 detail"));
        assert!(m2.detail.get("age_secs").is_some());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ★(0.15.0 · 「스스로 해결」 M4 · RULINGS AD8, BF1–BF6 · track T3b) 결함 감지기 검체
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod defect_intake_tests {
    use super::*;
    use cys::repair::intake;
    use cys::repair::types::LaneKey;

    fn bash(cmd: &str) -> Value {
        json!({ "command": cmd })
    }

    fn one(rel: &str, sub: Option<&str>) -> PackCall {
        PackCall::One { rel: rel.into(), sub: sub.map(str::to_string) }
    }

    /// The base lane of the tests (the same fictional spelling `production_roots` gives under test).
    fn base() -> PackRoots {
        PackRoots::new(Some("/Users/x/.cys/pack"), Some("/Users/x"))
    }

    fn pre<'a>(input: &'a Value) -> ToolObservation<'a> {
        ToolObservation {
            event_type: "PRE_TOOL",
            raw_event: "PreToolUse",
            tool_name: "Bash",
            tool_input: input,
            exit_code: None,
            error: None,
            interrupted: false,
        }
    }

    /// The failure shape RULINGS BE1 has the CLI forward (`PostToolUseFailure` → POST_TOOL, exit 1, `error`).
    fn fail<'a>(input: &'a Value, error: Option<&'a str>) -> ToolObservation<'a> {
        ToolObservation {
            event_type: "POST_TOOL",
            raw_event: "PostToolUseFailure",
            tool_name: "Bash",
            tool_input: input,
            exit_code: Some(1),
            error,
            interrupted: false,
        }
    }

    pub(super) fn tmp_root(tag: &str) -> std::path::PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("cys-t3b-{tag}-{}-{n}", std::process::id()))
    }

    fn rows_in(root: &std::path::Path, lane: &LaneKey) -> Vec<intake::IntakeRow> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(root.join("intake").join(&lane.0)) {
            for e in rd.flatten() {
                if let Ok(Some(r)) = intake::load_row(&e.path()) {
                    out.push(r);
                }
            }
        }
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        out
    }

    /// RULINGS BF1 / BS1 — positives: the identity is the pack script of THIS lane that a call made of
    /// ONE simple command ran — bare, or through one interpreter, with assignments in front, with an
    /// output redirection to `/dev/null` or a descriptor copy — spelled with any of the lane's pack roots.
    #[test]
    fn identity_is_the_executed_program() {
        let pc = |c: &str| pack_call(&bash(c), &base());
        // bare script
        assert_eq!(pc("~/.cys/pack/bin/cys-dept status"), one("bin/cys-dept", Some("status")));
        assert_eq!(pc("$CYS_PACK_DIR/bin/javis_x.py status"), one("bin/javis_x.py", Some("status")));
        assert_eq!(pc("cys-dept create team-x"), one("bin/cys-dept", Some("create")));
        // interpreter + script
        assert_eq!(pc("python3 ~/.cys/pack/bin/javis_x.py check --json"), one("bin/javis_x.py", Some("check")));
        assert_eq!(pc(r#"python3 "$HOME/.cys/pack/bin/javis_x.py""#), one("bin/javis_x.py", None));
        assert_eq!(pc("python3 ${HOME}/.cys/pack/bin/javis_x.py run"), one("bin/javis_x.py", Some("run")));
        assert_eq!(pc("python3 /Users/x/.cys/pack/bin/javis_x.py run"), one("bin/javis_x.py", Some("run")));
        assert_eq!(pc(r#"sh "${CYS_PACK_DIR}/hooks/guard.sh""#), one("hooks/guard.sh", None));
        assert_eq!(pc(r#"python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_x.py" check"#), one("bin/javis_x.py", Some("check")));
        assert_eq!(pc("python3 ${CYS_PACK_DIR:-${HOME}/.cys/pack}/bin/javis_x.py"), one("bin/javis_x.py", None));
        assert_eq!(pc("python3 -u -B -X importtime ~/.cys/pack/bin/javis_x.py run"), one("bin/javis_x.py", Some("run")));
        assert_eq!(pc("bash -e ~/.cys/pack/hooks/sub/x.sh go"), one("hooks/sub/x.sh", Some("go")));
        assert_eq!(pc("node ~/.cys/pack/bin/x.mjs"), one("bin/x.mjs", None));
        assert_eq!(pc("python3 ~/.cys/pack/bin/javis_x.py 'a $(b) `c`; d'"), one("bin/javis_x.py", Some("a $(b) `c`; d")), "single quotes are literal");
        // assignments + script
        assert_eq!(pc("FOO=1 BAR=x python3 ~/.cys/pack/bin/javis_x.py run"), one("bin/javis_x.py", Some("run")));
        assert_eq!(pc("A=1 ~/.cys/pack/bin/cys-dept list"), one("bin/cys-dept", Some("list")));
        // redirections to /dev/null or a descriptor copy
        assert_eq!(pc("python3 ~/.cys/pack/bin/javis_x.py run 2>&1"), one("bin/javis_x.py", Some("run")));
        assert_eq!(pc("python3 ~/.cys/pack/bin/javis_x.py run >/dev/null"), one("bin/javis_x.py", Some("run")));
        assert_eq!(pc("python3 ~/.cys/pack/bin/javis_x.py run > /dev/null 2>&1"), one("bin/javis_x.py", Some("run")));
        assert_eq!(pc("python3 ~/.cys/pack/bin/javis_x.py run 2>/dev/null >&2"), one("bin/javis_x.py", Some("run")));
        assert_eq!(pc("python3 ~/.cys/pack/bin/javis_x.py run &>/dev/null"), one("bin/javis_x.py", Some("run")));
        // a trailing newline or comment is not a second command
        assert_eq!(pc("python3 ~/.cys/pack/bin/javis_x.py run\n"), one("bin/javis_x.py", Some("run")));
        assert_eq!(pc("cys-dept list   # the teams"), one("bin/cys-dept", Some("list")));
        // Windows: the pack dir with a drive letter, its Git Bash `/c/…` form, `pwsh -File`
        let win = PackRoots::new(Some(r"C:\Users\x\.cys\pack"), Some(r"C:\Users\x"));
        assert_eq!(pack_call(&bash(r#"python "C:\Users\x\.cys\pack\bin\javis_x.py" check"#), &win), one("bin/javis_x.py", Some("check")));
        assert_eq!(pack_call(&bash("python /c/Users/x/.cys/pack/bin/javis_x.py"), &win), one("bin/javis_x.py", None));
        assert_eq!(pack_call(&bash("pwsh -NoProfile -File C:/Users/x/.cys/pack/bin/x.ps1"), &win), one("bin/x.ps1", None));
        // a department daemon: ITS pack
        let dept = PackRoots::new(Some("/Users/x/.cys/pack-dept-a"), Some("/Users/x"));
        assert_eq!(pack_call(&bash("python3 ~/.cys/pack-dept-a/bin/javis_x.py"), &dept), one("bin/javis_x.py", None));
        assert_eq!(pack_call(&bash("python3 $CYS_PACK_DIR/bin/javis_x.py"), &dept), one("bin/javis_x.py", None));
        // a list command
        assert_eq!(pack_call(&json!({"command": ["python3", "/Users/x/.cys/pack/bin/a.py", "x"]}), &base()), one("bin/a.py", Some("x")));
    }

    /// RULINGS BF1 — negatives: a MENTION is never an identity — an operand of another program, a user
    /// script's argument, a pipeline member piped onward, a heredoc body, a quoted string, a comment,
    /// another lane's pack, a test rig, a command followed by another, a background or negated command,
    /// inline code, a lookup, a non-script.
    #[test]
    fn mentions_are_never_an_identity() {
        for c in [
            "grep pat ~/.cys/pack/hooks/guard.sh",
            "cat ~/.cys/pack/bin/javis_x.py",
            "diff ~/.cys/pack/bin/a.py b.py",
            "test -f ~/.cys/pack/bin/a.py",
            "[ -x ~/.cys/pack/bin/a.py ]",
            "python3 ./mytool.py ~/.cys/pack/bin/javis_x.py",
            "./run.sh ~/.cys/pack/hooks/guard.sh",
            "python3 ~/.cys/pack/bin/javis_x.py | grep x",
            "python3 ~/.cys/pack/bin/javis_x.py |& tee log",
            "cat > /tmp/a.sh <<EOF\npython3 ~/.cys/pack/bin/javis_x.py\nEOF\n",
            "bash <<'EOF'\npython3 ~/.cys/pack/bin/javis_x.py\nEOF\n",
            "cat <<-EOF\n\tpython3 ~/.cys/pack/bin/javis_x.py\n\tEOF\necho done",
            "echo \"python3 ~/.cys/pack/bin/javis_x.py\"",
            "bash -c \"python3 ~/.cys/pack/bin/javis_x.py\"",
            "bash -lc 'python3 ~/.cys/pack/bin/javis_x.py'",
            "ls # python3 ~/.cys/pack/bin/javis_x.py",
            "python3 ~/.cys/pack-dept-a/bin/javis_x.py",
            "python3 /tmp/rig/.cys/pack/bin/javis_x.py",
            "python3 /Users/user/.cys/pack/bin/javis_x.py",
            "python3 ~/.cys/pack/bin/javis_x.py && echo ok",
            "python3 ~/.cys/pack/bin/javis_x.py || true",
            "python3 ~/.cys/pack/bin/javis_x.py &",
            "python3 ~/.cys/pack/bin/javis_x.py & ",
            "! python3 ~/.cys/pack/bin/javis_x.py",
            "python3 -c 'import x' ~/.cys/pack/bin/javis_x.py",
            "python3 -m pytest ~/.cys/pack/bin/tests/test_x.py",
            "node -e 'x' ~/.cys/pack/bin/x.js",
            "command -v ~/.cys/pack/bin/javis_x.py",
            "python3 ~/.cys/pack/directives/MASTER_DIRECTIVE.md",
            "python3 ~/.cys/pack/bin/*.py",
            "python3 ~/.cys/pack/../evil.py",
            "python3 ~/.cys/pack/bin/$(name).py",
            "python3 ./javis_x.py",
            "npm test",
        ] {
            assert_eq!(pack_call(&bash(c), &base()), PackCall::None, "a mention became an identity: {c:?}");
        }
        // the base lane's pack is not a department daemon's
        let dept = PackRoots::new(Some("/Users/x/.cys/pack-dept-a"), Some("/Users/x"));
        assert_eq!(pack_call(&bash("python3 ~/.cys/pack/bin/javis_x.py"), &dept), PackCall::None);
        // other tools and other events are never classified as calls / failures of a script
        let g = bash("grep pat ~/.cys/pack/hooks/guard.sh");
        assert_eq!(classify(&fail(&g, Some("Exit code 1")), &base(), false), Classified::Ignored { failure: true });
        assert_eq!(classify(&pre(&g), &base(), false), Classified::Ignored { failure: false });
        let x = bash("python3 ~/.cys/pack/bin/javis_x.py");
        let edit = ToolObservation { tool_name: "Edit", ..fail(&x, Some("E: x")) };
        assert_eq!(classify(&edit, &base(), false), Classified::Ignored { failure: true });
        let abort = ToolObservation { interrupted: true, ..fail(&x, Some("E: x")) };
        assert_eq!(classify(&abort, &base(), false), Classified::Ignored { failure: false }, "an abort is not a failure of the script");
    }

    /// RULINGS BS1 — ONLY a call that is one simple command gets an identity: one negative per control
    /// operator and construct that can end or replace the call before or after the script, the three
    /// routes of the second re-check verbatim, and the forms earlier rounds let through (`cd … &&`, `;`,
    /// `||`) — every one a stated miss. Calls are counted by the same rule as failures.
    #[test]
    fn only_one_simple_command_gets_an_identity() {
        let pc = |c: &str| pack_call(&bash(c), &base());
        let x = "python3 ~/.cys/pack/bin/javis_x.py";
        let negatives = [
            format!("true && {x}"),
            format!("false || {x}"),
            format!("echo a; {x}"),
            format!("echo a\n{x}"),
            format!("echo a | {x}"),
            format!("echo a |& {x}"),
            format!("{x} &"),
            format!("({x})"),
            format!("{{ {x}; }}"),
            format!("{x} $(cat .args)"),
            format!("{x} \"$(date)\""),
            format!("{x} `date`"),
            format!("{x} $((1 + 2))"),
            format!("{x} <(echo a)"),
            format!("{x} >(cat)"),
            format!("exit 0; {x}"),
            format!("return 1 2>/dev/null; {x}"),
            format!("set -e; {x}"),
            format!("cd /tmp && {x}"),
            format!("! {x}"),
            format!("time {x}"),
            format!("timeout 5 {x}"),
            format!("{x} <<EOF\nin\nEOF\n"),
            format!("{x} <<< data"),
            format!("{x} < in.txt"),
            format!("{x} > out.txt"),
            format!("{x} >> log.txt"),
            format!("{x} 2> err.txt"),
            format!("{x} &> out.txt"),
            format!("{x} >&-"),
            format!("{x} > /unwritable/file"),
            format!("{x} > 2>/dev/null"),
            format!("A=${{HOME}} {x}"),
            format!("{x} 'unterminated"),
            format!("{x} ${{X"),
            format!("{x} \\"),
            format!("env A=1 {x}"),
            format!("nohup {x}"),
            format!("node -r ./pre.js ~/.cys/pack/bin/x.js"),
            // the second re-check's routes, verbatim
            format!("cd /nonexistent && {x}"),
            format!("ID=$(cat .id) && {x}"),
            format!("cd x || exit 1; {x}"),
            format!("set -euo pipefail; {x}"),
            format!("set -o pipefail; a | {x}"),
        ];
        for c in &negatives {
            assert_eq!(pc(c), PackCall::None, "a compound or uncertain call got an identity: {c:?}");
            assert_eq!(classify(&pre(&bash(c)), &base(), false), Classified::Ignored { failure: false }, "counted as a call: {c:?}");
        }
        assert_eq!(negatives.len(), 44);
        // zsh's wording of the failed `cd` (Claude Code on macOS runs commands through zsh `eval`)
        let zsh = bash(&format!("cd /nonexistent && {x}"));
        assert_eq!(
            classify(&fail(&zsh, Some("Exit code 1\n(eval):cd:1: no such file or directory: /nonexistent")), &base(), false),
            Classified::Ignored { failure: true }
        );
    }

    /// RULINGS BS1 at the daemon: the second re-check's routes failing five times each file NO row and
    /// count no call (a `cd … && <script>` failing INSIDE the script is a stated miss too); the same
    /// script run as one simple command and failing five times files its row.
    #[test]
    fn compound_calls_file_nothing_a_single_call_does() {
        let daemon = crate::team_gate_tests::tmp_daemon("t3b-bs1", false);
        let root = tmp_root("bs1");
        let routes = [
            ("cd /nonexistent && python3 ~/.cys/pack/bin/javis_x.py run", "Exit code 1\n(eval):cd:1: no such file or directory: /nonexistent"),
            ("ID=$(cat .id) && python3 ~/.cys/pack/bin/javis_x.py run", "Exit code 1\ncat: .id: No such file or directory"),
            ("cd x || exit 1; python3 ~/.cys/pack/bin/javis_x.py run", "Exit code 1\n(eval):cd:1: no such file or directory: x"),
            ("set -euo pipefail; python3 ~/.cys/pack/bin/javis_x.py run", "Exit code 1\nKeyError: 'k'"),
            ("set -o pipefail; a | python3 ~/.cys/pack/bin/javis_x.py run", "Exit code 127\n(eval):1: command not found: a"),
            ("cd /tmp && python3 ~/.cys/pack/bin/javis_x.py run", "Exit code 1\nKeyError: 'k'"),
        ];
        for (cmd, err) in routes {
            let input = bash(cmd);
            for _ in 0..5 {
                observe_tool_event(&daemon, &pre(&input));
                observe_tool_event(&daemon, &fail(&input, Some(err)));
            }
        }
        let rep = flush_defect_intake(&daemon, &root, &AlertConfig::default(), crate::alert_route::Now::at(10.0));
        assert_eq!(rep.due, 0, "{rep:?}");
        assert!(!root.join("intake").exists(), "a compound call was filed against the script");
        let snap = defect_intake_snapshot(&daemon, 10.0);
        assert_eq!((snap["calls"].as_u64(), snap["failures"].as_u64(), snap["failures_without_identity"].as_u64()), (Some(0), Some(0), Some(30)), "{snap}");
        // the same script as ONE simple command → its row
        let single = bash("python3 ~/.cys/pack/bin/javis_x.py run 2>&1");
        for _ in 0..5 {
            observe_tool_event(&daemon, &pre(&single));
            observe_tool_event(&daemon, &fail(&single, Some("Exit code 1\nKeyError: 'k'")));
        }
        let rep = flush_defect_intake(&daemon, &root, &AlertConfig::default(), crate::alert_route::Now::at(20.0));
        assert_eq!((rep.due, rep.written_new), (1, 1), "{rep:?}");
        let rows = rows_in(&root, &LaneKey::of_socket(&daemon.socket_path));
        assert_eq!((rows.len(), rows[0].rel.as_str(), rows[0].count), (1, "bin/javis_x.py", 5));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RULINGS BF1 at the daemon: five failing `grep pat ~/.cys/pack/hooks/guard.sh` calls (no match →
    /// exit 1) file NO row and count NO call — ordinary user activity produces nothing.
    #[test]
    fn five_failing_greps_of_a_pack_file_file_nothing() {
        let daemon = crate::team_gate_tests::tmp_daemon("t3b-grep", false);
        let root = tmp_root("grep");
        let g = bash("grep pat ~/.cys/pack/hooks/guard.sh");
        for _ in 0..5 {
            observe_tool_event(&daemon, &pre(&g));
            observe_tool_event(&daemon, &fail(&g, Some("Exit code 1")));
        }
        let rep = flush_defect_intake(&daemon, &root, &AlertConfig::default(), crate::alert_route::Now::at(10.0));
        assert_eq!(rep.due, 0);
        assert!(!root.join("intake").exists(), "a grep of a pack file filed a defect");
        let snap = defect_intake_snapshot(&daemon, 10.0);
        assert_eq!((snap["calls"].as_u64(), snap["failures"].as_u64(), snap["tracked_rels"].as_u64()), (Some(0), Some(0), Some(0)), "{snap}");
        assert_eq!(snap["failures_without_identity"], 5);
    }

    /// RULINGS BF5: classification is ONE pass — the steps stay linear in the command's length for the
    /// worst inputs (unclosed `${CYS_PACK_DIR:`, unclosed `$(`, heredocs that never end, quotes, escapes);
    /// a structural pin, no wall clock.
    #[test]
    fn classification_cost_is_linear_in_the_command() {
        let roots = base();
        let unit = [
            "${CYS_PACK_DIR:",
            "$(",
            "x <<A\n",
            "\"",
            "'",
            "\\",
            "a | ",
            "${CYS_PACK_DIR:-${HOME}/.cys/pack}/bin/x.py ",
            "python3 ~/.cys/pack/bin/javis_x.py; ",
            "<(",
            "#\n",
        ];
        for u in unit {
            let reps = COMMAND_SCAN_MAX_BYTES / u.len() + 1;
            let cmd = u.repeat(reps);
            let chars = cut_at_char_boundary(&cmd, COMMAND_SCAN_MAX_BYTES).chars().count();
            let (_, steps) = pack_call_counted(&bash(&cmd), &roots);
            assert!(steps <= 3 * chars + 64, "{u:?}: {steps} steps for {chars} chars — not linear");
        }
        // the hook path: classify before the lock, nothing else under it but the table update
        let src = include_str!("alerts.rs");
        let body = src.split("pub fn observe_tool_event_with(").nth(1).expect("observe_tool_event_with");
        let body = &body[..body.find("\n}\n").expect("end")];
        let (cl, lk) = (body.find("classify(").expect("classify"), body.find(".det.lock(").expect("lock"));
        assert!(cl < lk, "the command is classified under the detector's lock");
        assert_eq!(body.matches("classify(").count(), 1);
        let locked = &body[lk..];
        assert!(!locked.contains("classify(") && !locked.contains("pack_call"), "classification inside the lock");
    }

    /// Structural pin (A 1-26 / 1-28, F1): the hook-path functions do no I/O, spawn nothing themselves
    /// (the flusher's start is the injected `FlusherStart`, once) and reach no queue, bus or alert
    /// route; the writer reaches no queue, bus or alert route either. Positive control included.
    #[test]
    fn hot_path_is_memory_only_and_nothing_sends() {
        let src = include_str!("alerts.rs");
        let body = |head: &str| -> String {
            let b = src.split(head).nth(1).unwrap_or_else(|| panic!("{head} missing"));
            b[..b.find("\n}\n").unwrap_or_else(|| panic!("{head} end"))].to_string()
        };
        let io = ["std::fs", "atomic_write", "File::", "Command::", "spawn(", "read_to_string", "load()", "thread::"];
        let send = ["bus.publish", "enqueue", "pending_queue", "alert_route::route", "handle_event", "javis_wakeup", "send_input", "push_queue"];
        let planted = "let _ = std::fs::write(p, b);";
        assert!(io.iter().any(|n| planted.contains(n)), "checker is blind");
        for head in [
            "pub fn observe_tool_event(",
            "pub fn observe_tool_event_with(",
            "pub fn classify(",
            "pub fn record(",
            "pub fn pack_call_counted(",
            "fn single_simple_command(",
            "fn interpreter_operand(",
            "pub fn detector_excluded(",
            "fn is_failure(",
            "fn bounded_error(",
        ] {
            let b = body(head);
            for n in io.iter().chain(send.iter()) {
                assert!(!b.contains(n), "{head} reaches `{n}` — the hook path must stay memory-only");
            }
        }
        assert!(body("pub fn observe_tool_event_with(").contains("flusher.once.get_or_init("), "the flusher must start once, through the injected hook");
        for head in ["pub fn flush_defect_intake(", "pub fn flusher_pass(", "fn log_flush("] {
            let b = body(head);
            for n in send {
                assert!(!b.contains(n), "{head} sends through `{n}` — intake is data, never a message");
            }
        }
        // the per-file mutex key is T3's (`ledger_verbs::report`) — one writer per row file
        assert!(src.contains("map.entry(format!(\"intake:{sig}\"))"));
        assert!(include_str!("repair.rs").contains("ctx.locked(&format!(\"intake:{}\", rep.signature)"), "T3's report verb changed its row mutex key");
        // the `usage.event` arm feeds the detector once
        let h = include_str!("handlers.rs");
        let arm = h.split("\"usage.event\" => {").nth(1).expect("usage.event arm");
        let arm = &arm[..arm.find("\"org.status\" =>").expect("next arm")];
        assert!(arm.contains("crate::alerts::observe_tool_event("), "the observation event no longer reaches the detector");
        assert_eq!(h.matches("crate::alerts::observe_tool_event(").count(), 1);
    }

    /// The thresholds of `evaluate` §3 apply per IDENTITY inside the 7-day window (RULINGS BF4); another
    /// script's failures do not count; the config is honoured.
    #[test]
    fn thresholds_apply_per_identity_inside_the_window() {
        let mut d = Detector::default();
        assert_eq!(d.cfg, AlertConfig::default(), "the rule starts from the default thresholds");
        let a = bash("python3 ~/.cys/pack/bin/a.py run");
        let b = bash("python3 ~/.cys/pack/bin/b.py run");
        let t = 1_800_000_000.0;
        for i in 0..4 {
            assert_eq!(observe(&mut d, &pre(&a), &base(), t + i as f64, false), Observed::Call);
            assert_eq!(observe(&mut d, &fail(&a, Some("Exit code 1\nKeyError: 'k'")), &base(), t + 0.5, false), Observed::Failure);
        }
        observe(&mut d, &pre(&b), &base(), t, false);
        observe(&mut d, &fail(&b, Some("E: b")), &base(), t, false);
        assert!(due_signatures(&d, t).is_empty(), "4 failures are below fail_count 5");
        observe(&mut d, &pre(&a), &base(), t + 5.0, false);
        observe(&mut d, &fail(&a, Some("Exit code 1\nKeyError: 'k'")), &base(), t + 5.5, false);
        let due = due_signatures(&d, t + 6.0);
        assert_eq!(due.len(), 1, "only a.py met the rule: {due:?}");
        assert_eq!((due[0].report.rel.as_str(), due[0].delta, due[0].first_seen, due[0].last_seen), ("bin/a.py", 5, t + 0.5, t + 5.5));
        // a week later the same counts are outside the window
        assert!(due_signatures(&d, t + 8.0 * 86_400.0).is_empty(), "counts older than 7 days still met the rule");
        // a rate below 30 %: 20 calls, 5 failures → not due; the 6th failure (30 %) → due
        let mut d = Detector::default();
        let c = bash("python3 ~/.cys/pack/bin/c.py");
        for _ in 0..20 {
            observe(&mut d, &pre(&c), &base(), t, false);
        }
        for _ in 0..5 {
            observe(&mut d, &fail(&c, Some("E: c")), &base(), t, false);
        }
        assert!(due_signatures(&d, t).is_empty(), "25 % < 30 %");
        observe(&mut d, &fail(&c, Some("E: c")), &base(), t, false);
        assert_eq!(due_signatures(&d, t).len(), 1, "30 % = fail_rate");
        d.cfg = AlertConfig { fail_count: 7, ..AlertConfig::default() };
        assert!(due_signatures(&d, t).is_empty(), "alerts-config.json's fail_count is honoured");
    }

    /// The exclusion list on POSITIVE evidence (RULINGS BF2) before anything is counted; a failure
    /// without text forms no row.
    #[test]
    fn exclusions_need_positive_evidence_and_unnamed_failures_form_no_row() {
        let cfg_t = 1_800_000_000.0;
        let dept = bash("cys-dept create team-x");
        let err127 = "Exit code 127\n/usr/bin/bash: line 1: cys-dept: command not found";
        // Windows: by design, never counted
        let mut d = Detector::default();
        for _ in 0..10 {
            observe(&mut d, &pre(&dept), &base(), cfg_t, true);
            assert_eq!(observe(&mut d, &fail(&dept, Some(err127)), &base(), cfg_t, true), Observed::Excluded);
        }
        assert!(due_signatures(&d, cfg_t).is_empty());
        assert_eq!(d.counters.excluded_by_reason.get("windows-cys-dept-not-on-path"), Some(&10));
        // macOS: a missing cys-dept is a defect
        let mut d = Detector::default();
        for _ in 0..5 {
            observe(&mut d, &pre(&dept), &base(), cfg_t, false);
            observe(&mut d, &fail(&dept, Some(err127)), &base(), cfg_t, false);
        }
        assert_eq!(due_signatures(&d, cfg_t).len(), 1);
        // a polled `orchestra check` with its designed answer files nothing; its crash or a syntax error does
        let mut d = Detector::default();
        let chk = bash("python3 ~/.cys/pack/bin/javis_orchestra.py check");
        for _ in 0..10 {
            observe(&mut d, &pre(&chk), &base(), cfg_t, false);
            observe(&mut d, &fail(&chk, Some("Exit code 1\nLLM orchestrating 노드 점검 (4종 의무 + grok 선택):\n  ✗ worker — 미기동 (x)\n종합: 필수 3/4 생존 — 부재: worker → `cys boot`로 기동하라")), &base(), cfg_t, false);
        }
        assert!(due_signatures(&d, cfg_t).is_empty(), "a boot that polls `orchestra check` files nothing");
        assert_eq!(d.counters.excluded_by_reason.get("designed-answer:javis_orchestra.check"), Some(&10));
        for _ in 0..5 {
            observe(&mut d, &fail(&chk, Some("Exit code 1\n  File \"/Users/x/.cys/pack/bin/javis_orchestra.py\", line 9\n    def (\n        ^\nSyntaxError: invalid syntax")), &base(), cfg_t, false);
        }
        assert_eq!(due_signatures(&d, cfg_t).len(), 1, "a syntax error of the same tool is a defect");
        // the same exit code WITHOUT the tool's marker is a defect (no positive evidence)
        let mut d = Detector::default();
        let gate = bash("python3 ~/.cys/pack/bin/javis_resource_gate.py check");
        for _ in 0..5 {
            observe(&mut d, &pre(&gate), &base(), cfg_t, false);
            observe(&mut d, &fail(&gate, Some("Exit code 2\nusage: javis_resource_gate.py [-h]")), &base(), cfg_t, false);
        }
        assert_eq!(due_signatures(&d, cfg_t).len(), 1, "an exit code alone excluded a failure");
        // no text: counted toward the rate, no signature (O-C1-9) → no row
        let mut d = Detector::default();
        let a = bash("python3 ~/.cys/pack/bin/a.py");
        for _ in 0..10 {
            observe(&mut d, &pre(&a), &base(), cfg_t, false);
            let today = ToolObservation { raw_event: "PostToolUse", ..fail(&a, None) };
            assert_eq!(observe(&mut d, &today, &base(), cfg_t, false), Observed::NoErrorText);
        }
        assert!(due_signatures(&d, cfg_t).is_empty());
        assert_eq!(d.counters.no_error_text, 10);
    }

    /// The detector's own exclusion entries match each tool's verdict marker with its exit code, never
    /// a launch failure, never `javis_task.py` (no stable marker → no entry); every reason distinct.
    #[test]
    fn detector_exclusions_name_only_marked_designed_answers() {
        use cys::repair::intake::ExclusionScope;
        assert!(DETECTOR_EXCLUSIONS.iter().all(|e| e.scope == ExclusionScope::Detector));
        let mut reasons: Vec<&str> = DETECTOR_EXCLUSIONS.iter().chain(intake::EXCLUSIONS).map(|e| e.reason).collect();
        let n = reasons.len();
        reasons.sort_unstable();
        reasons.dedup();
        assert_eq!(reasons.len(), n, "a reason names two entries");
        let ex = |rel: &str, sub: Option<&str>, err: &str| detector_excluded(rel, sub, err, false);
        let rg = "bin/javis_resource_gate.py";
        assert_eq!(ex(rg, Some("check"), "Exit code 2\nverdict: hard_block\n  ✗ load"), Some("designed-answer:javis_resource_gate.check"));
        assert_eq!(ex(rg, Some("check"), "Exit code 1\n{\n \"verdict\": \"soft_warn\"\n}"), Some("designed-answer:javis_resource_gate.check"));
        assert_eq!(ex(rg, Some("check"), "Exit code 1\nverdict: hard_block"), None, "the marker of another answer");
        assert_eq!(ex(rg, Some("check"), "Exit code 70\ninternal"), None);
        assert_eq!(ex(rg, Some("check"), "Exit code 2\npython3: can't open file '/x/javis_resource_gate.py': [Errno 2] No such file or directory"), None);
        let oc = "bin/javis_orchestra.py";
        assert_eq!(ex(oc, Some("check"), "Exit code 12\n종합: 4종 의무 노드 전부 생존(ready) — 그러나 각성 ACK 미확인: worker"), Some("designed-answer:javis_orchestra.check"));
        assert_eq!(ex(oc, Some("check"), "Exit code 1\n{\"exit\": 1, \"schema\": \"orchestra-check/v1\"}"), Some("designed-answer:javis_orchestra.check"));
        // RULINGS BP2: exit 2 is NOT an answer — its marker is also printed for a missing / failing `cys`
        // and any exception in the helper; such a failure is one bundled row like any other
        assert_eq!(ex(oc, Some("check"), "Exit code 2\n[orchestra check] cys status 수집 실패(데몬 미가동?) — `cys ping` 확인 후 재실행"), None);
        assert_eq!(ex(oc, Some("check"), "Exit code 2\n{\"exit\": 2, \"schema\": \"orchestra-check/v1\"}"), None);
        assert_eq!(ex(oc, Some("check"), "Exit code 1\nTraceback (most recent call last):\n  File \"x\"\nKeyError: 'k'\n종합: 필수 3/4 생존 — 부재: x"), None);
        assert_eq!(ex(oc, Some("task-prompt"), "Exit code 1\n종합: 필수 3/4 생존 — 부재: x"), None);
        let wk = "bin/javis_wakeup.py";
        assert_eq!(ex(wk, Some("drain"), "Exit code 4\nrefused: AUTOPILOT_PAUSED — 정지 중에는 wake 를 배달하지 않는다"), Some("designed-answer:javis_wakeup"));
        assert_eq!(ex(wk, Some("drain"), "Exit code 5\nnothing pending"), Some("designed-answer:javis_wakeup"));
        assert_eq!(ex(wk, Some("cancel"), "Exit code 5\n{\"cancelled\": 0, \"ids\": []}"), Some("designed-answer:javis_wakeup"));
        assert_eq!(ex(wk, Some("drain"), "Exit code 6\npark failed"), None);
        assert_eq!(ex(wk, Some("drain"), "Exit code 5"), None, "no marker");
        // javis_task.py has no entry: its "no" is one bundled row
        for c in [3, 4, 5, 6, 8, 9] {
            assert_eq!(ex("bin/javis_task.py", Some("checkout"), &format!("Exit code {c}\nconflict: x")), None);
        }
        // the shared table (T3's Windows cys-dept entry) is consulted too
        let dept = "Exit code 127\n/usr/bin/bash: line 1: cys-dept: command not found";
        assert_eq!(detector_excluded("bin/cys-dept", Some("create"), dept, true), Some("windows-cys-dept-not-on-path"));
        assert_eq!(detector_excluded("bin/cys-dept", Some("create"), dept, false), None);
    }

    /// RULINGS BF4: a full table never blinds the detector — with every signature slot taken (each one
    /// pending, none due) and every script slot taken, a NEW defect is still recorded and becomes a row.
    #[test]
    fn full_tables_still_record_a_new_defect() {
        let t = 1_800_000_000.0;
        let mut d = Detector::default();
        for i in 0..DETECTOR_MAX_SIGNATURES {
            let s = bash(&format!("python3 ~/.cys/pack/bin/s{i}.py"));
            assert_eq!(observe(&mut d, &fail(&s, Some(&format!("E{i}: x"))), &base(), t + i as f64, false), Observed::Failure);
        }
        for i in DETECTOR_MAX_SIGNATURES..DETECTOR_MAX_RELS {
            observe(&mut d, &pre(&bash(&format!("python3 ~/.cys/pack/bin/s{i}.py"))), &base(), t + i as f64, false);
        }
        assert_eq!((d.tracked().0, d.tracked().1), (DETECTOR_MAX_RELS, DETECTOR_MAX_SIGNATURES));
        assert!(due_signatures(&d, t + 2_000.0).is_empty(), "one failure each is below the rule");
        // a new script with a new defect, five times
        let fresh = bash("python3 ~/.cys/pack/bin/fresh.py run");
        for _ in 0..5 {
            observe(&mut d, &pre(&fresh), &base(), t + 2_000.0, false);
            assert_eq!(observe(&mut d, &fail(&fresh, Some("Exit code 1\nValueError: new")), &base(), t + 2_000.0, false), Observed::Failure);
        }
        let due = due_signatures(&d, t + 2_000.0);
        assert_eq!(due.len(), 1, "{due:?}");
        assert_eq!((due[0].report.rel.as_str(), due[0].delta), ("bin/fresh.py", 5), "the new defect was not recorded");
        assert_eq!((d.counters.evicted_signatures, d.counters.evicted_pending_signatures), (1, 1));
        assert!(d.counters.evicted_rels >= 1);
        assert_eq!((d.tracked().0, d.tracked().1), (DETECTOR_MAX_RELS, DETECTOR_MAX_SIGNATURES), "still bounded");
        // and it becomes a row
        let daemon = crate::team_gate_tests::tmp_daemon("t3b-full", false);
        let root = tmp_root("full");
        *intake_for(&daemon).det.lock().unwrap() = d;
        let rep = flush_defect_intake(&daemon, &root, &AlertConfig::default(), crate::alert_route::Now { mono: 10.0, epoch: t + 2_000.0 });
        assert_eq!(rep.written_new, 1, "{rep:?}");
        let rows = rows_in(&root, &LaneKey::of_socket(&daemon.socket_path));
        assert_eq!((rows.len(), rows[0].rel.as_str(), rows[0].count), (1, "bin/fresh.py", 5));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn feed(daemon: &Arc<Daemon>, cmd: &str, error: &str, n: usize) {
        let input = bash(cmd);
        for _ in 0..n {
            observe_tool_event(daemon, &pre(&input));
            observe_tool_event(daemon, &fail(&input, Some(error)));
        }
    }

    /// ★THE FLOOD TEST (design §6-4 (7) · D:1095 · I-F1-02): the same failure 1,000× → ONE row of this
    /// lane (count 1,000), the report gate's input is ONE defect, and nothing reaches a queue, the event
    /// bus or the alert route (the router half is `alert_route::drills::drill_defect_flood_does_not_delay_
    /// seat_death_or_clear_signals`). A second, different script gets its own row.
    #[test]
    fn flood_of_one_failure_is_one_row_and_touches_no_queue_bus_or_alert_route() {
        let daemon = crate::team_gate_tests::tmp_daemon("t3b-flood", false);
        let root = tmp_root("flood");
        let lane = LaneKey::of_socket(&daemon.socket_path);
        let cfg = AlertConfig::default();
        let alert_before = format!("{:?}", *daemon.alert_route.lock().unwrap_or_else(|e| e.into_inner()));
        let bus_before = daemon.bus.latest_seq();
        feed(&daemon, "python3 ~/.cys/pack/bin/javis_x.py run", "Exit code 1\nTraceback (most recent call last):\n  File \"/tmp/q/javis_x.py\", line 4\nKeyError: 'k' (pid 41234)", 1000);
        let rep = flush_defect_intake(&daemon, &root, &cfg, crate::alert_route::Now::at(1_000.0));
        assert_eq!((rep.due, rep.written_new), (1, 1), "{rep:?}");
        let rows = rows_in(&root, &lane);
        assert_eq!(rows.len(), 1, "1,000 identical failures must be ONE row");
        assert_eq!((rows[0].count, rows[0].rel.as_str(), rows[0].first_line.as_str()), (1000, "bin/javis_x.py", "KeyError: 'k' (pid <n>)"));
        assert_eq!(rows[0].reporters, vec![DETECTOR_REPORTER.to_string()]);
        let lanes: Vec<_> = std::fs::read_dir(root.join("intake")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(lanes, vec![std::ffi::OsString::from(&lane.0)]);
        let (defects, broken) = intake::load_all_in(&root);
        assert_eq!((defects.len(), broken.len(), defects[0].count), (1, 0, 1000));
        assert_eq!(daemon.bus.latest_seq(), bus_before, "the detector published on the event bus");
        assert_eq!(format!("{:?}", *daemon.alert_route.lock().unwrap_or_else(|e| e.into_inner())), alert_before, "the detector touched the alert route");
        assert!(daemon.surfaces.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
        assert_eq!(flush_defect_intake(&daemon, &root, &cfg, crate::alert_route::Now::at(1_001.0)).due, 0);
        // inside the row cooldown: held (counted, not lost); after it: written
        feed(&daemon, "python3 ~/.cys/pack/bin/javis_x.py run", "Exit code 1\nKeyError: 'k' (pid 50000)", 10);
        let held = flush_defect_intake(&daemon, &root, &cfg, crate::alert_route::Now::at(1_100.0));
        assert_eq!((held.due, held.held_cooldown, held.updated), (1, 1, 0));
        assert_eq!(rows_in(&root, &lane)[0].count, 1000);
        let after = flush_defect_intake(&daemon, &root, &cfg, crate::alert_route::Now::at(1_000.0 + crate::alert_route::INTAKE_ROW_COOLDOWN_SECS + 1.0));
        assert_eq!(after.updated, 1);
        assert_eq!(rows_in(&root, &lane)[0].count, 1010);
        // control: a different broken script → its own single row
        feed(&daemon, "python3 ~/.cys/pack/bin/javis_y.py", "Exit code 1\nModuleNotFoundError: No module named 'javis_z'", 5);
        flush_defect_intake(&daemon, &root, &cfg, crate::alert_route::Now::at(2_000.0));
        let rows = rows_in(&root, &lane);
        assert_eq!((rows.len(), rows[1].rel.as_str(), rows[1].count), (2, "bin/javis_y.py", 5));
        assert_eq!(daemon.bus.latest_seq(), bus_before);
        // the guard rails the flood must not weaken (appendix A 1-10; queue cap 100 untouched)
        assert_eq!((crate::alert_route::HOURLY_CAP, crate::alert_route::CLEAR_RESERVE, crate::alert_route::CSO_QUEUE_HEADROOM), (20, 26, 50));
        assert!(include_str!("handlers.rs").contains("if q.len() >= 100 {"), "the queue cap 100 changed");
        let snap = defect_intake_snapshot(&daemon, 2_000.0);
        assert_eq!((snap["rows_new"].as_u64(), snap["share"]["new_rows_total"].as_u64()), (Some(2), Some(2)));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RULINGS BF3: the SAME raw failure text filed by `cys repair report` and observed by the detector
    /// is ONE row (one normaliser), updated, never replaced; an existing row is not a NEW row for the share.
    #[test]
    fn manual_report_and_detector_share_one_row() {
        let daemon = crate::team_gate_tests::tmp_daemon("t3b-share", false);
        let root = tmp_root("share");
        let lane = LaneKey::of_socket(&daemon.socket_path);
        let raw = "Exit code 1\nloading 3 rows\nTraceback (most recent call last):\n  File \"/tmp/r9/a.py\", line 9, in <module>\nKeyError: 'k' (pid 40001)";
        let r = intake::parse_report("bin/a.py", raw).unwrap();
        assert!(matches!(intake::record_in(&root, &lane, &r, "worker", 10.0, 0, false, raw).unwrap(), intake::Recorded::New(_)));
        feed(&daemon, "python3 ~/.cys/pack/bin/a.py", raw, 5);
        let rep = flush_defect_intake(&daemon, &root, &AlertConfig::default(), crate::alert_route::Now::at(500.0));
        assert_eq!((rep.written_new, rep.updated), (0, 1), "{rep:?}");
        let rows = rows_in(&root, &lane);
        assert_eq!(rows.len(), 1, "two sources, one failure → one row");
        assert_eq!((rows[0].count, rows[0].first_seen, rows[0].first_line.as_str()), (6, 10.0, "KeyError: 'k' (pid <n>)"));
        assert_eq!(rows[0].reporters, vec!["worker".to_string(), DETECTOR_REPORTER.to_string()]);
        assert_eq!(intake_for(&daemon).share.lock().unwrap().new_rows_1h(500.0), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A corrupt row is never overwritten (O-C1-6), its count stays pending, and the other rows of the
    /// same flush are still written (F4).
    #[test]
    fn corrupt_row_is_never_overwritten_and_flush_continues() {
        let daemon = crate::team_gate_tests::tmp_daemon("t3b-corrupt", false);
        let root = tmp_root("corrupt");
        let lane = LaneKey::of_socket(&daemon.socket_path);
        let bad = intake::parse_report("bin/a.py", "Exit code 1\nE: a").unwrap();
        let p = intake::row_path_in(&root, &lane, &bad.signature);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"{not json").unwrap();
        feed(&daemon, "python3 ~/.cys/pack/bin/a.py", "Exit code 1\nE: a", 5);
        feed(&daemon, "python3 ~/.cys/pack/bin/b.py", "Exit code 1\nE: b", 5);
        let rep = flush_defect_intake(&daemon, &root, &AlertConfig::default(), crate::alert_route::Now::at(10.0));
        assert_eq!((rep.due, rep.corrupt, rep.written_new), (2, 1, 1), "{rep:?}");
        assert_eq!(std::fs::read(&p).unwrap(), b"{not json");
        assert_eq!(rows_in(&root, &lane).iter().map(|r| r.rel.as_str()).collect::<Vec<_>>(), vec!["bin/b.py"]);
        assert_eq!(intake_for(&daemon).det.lock().unwrap().tracked().2, 1, "the corrupt row's count is kept, not lost");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The intake's own hourly share for NEW rows holds the 31st new signature — in memory, with its
    /// count — and writes it when the window frees (hold, never drop).
    #[test]
    fn hourly_new_row_share_holds_and_never_drops() {
        let daemon = crate::team_gate_tests::tmp_daemon("t3b-hourly", false);
        let root = tmp_root("hourly");
        let lane = LaneKey::of_socket(&daemon.socket_path);
        let n = crate::alert_route::INTAKE_NEW_ROWS_PER_HOUR + 1;
        for i in 0..n {
            feed(&daemon, &format!("python3 ~/.cys/pack/bin/s{i:02}.py"), "Exit code 1\nE: x", 5);
        }
        let rep = flush_defect_intake(&daemon, &root, &AlertConfig::default(), crate::alert_route::Now::at(100.0));
        assert_eq!((rep.due, rep.written_new, rep.held_hourly), (n, n - 1, 1), "{rep:?}");
        let rep = flush_defect_intake(&daemon, &root, &AlertConfig::default(), crate::alert_route::Now::at(100.0 + crate::alert_route::WINDOW_SECS + 1.0));
        assert_eq!((rep.due, rep.written_new), (1, 1));
        let rows = rows_in(&root, &lane);
        assert_eq!(rows.len(), n);
        assert!(rows.iter().all(|r| r.count == 5), "the held signature kept its full count");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// G4: two lanes that see the same failure keep one row EACH; the aggregation on read counts ONE
    /// defect with both lanes. (Each lane runs its OWN pack: `$CYS_PACK_DIR` is lane-scoped.)
    #[test]
    fn two_lanes_keep_their_own_rows() {
        let base_d = crate::team_gate_tests::tmp_daemon("t3b-lane-a", false);
        let dept_d = crate::team_gate_tests::tmp_daemon("t3b-lane-b", true);
        let root = tmp_root("lanes");
        for d in [&base_d, &dept_d] {
            feed(d, "python3 $CYS_PACK_DIR/bin/a.py", "Exit code 1\nE: same", 500);
            flush_defect_intake(d, &root, &AlertConfig::default(), crate::alert_route::Now::at(10.0));
        }
        let (la, lb) = (LaneKey::of_socket(&base_d.socket_path), LaneKey::of_socket(&dept_d.socket_path));
        assert_ne!(la, lb);
        assert_eq!((rows_in(&root, &la)[0].count, rows_in(&root, &lb)[0].count), (500, 500));
        let (defects, _) = intake::load_all_in(&root);
        assert_eq!((defects.len(), defects[0].count, defects[0].lanes.len()), (1, 1000, 2));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RULINGS BF6: the first IDENTIFIED failure starts the flusher exactly once (an injected start; a
    /// call, a mention or an unidentified failure starts nothing); one pass of the flusher's loop body
    /// writes the row into an isolated root; a dead daemon is skipped; the log line is throttled.
    #[test]
    fn first_failure_starts_the_flusher_once_and_a_pass_writes_the_row() {
        let daemon = crate::team_gate_tests::tmp_daemon("t3b-pass", false);
        let starts = std::sync::atomic::AtomicUsize::new(0);
        let once = OnceLock::new();
        let start = || {
            starts.fetch_add(1, Ordering::SeqCst);
        };
        let fl = FlusherStart { once: &once, start: &start };
        let x = bash("python3 ~/.cys/pack/bin/a.py");
        let g = bash("grep a ~/.cys/pack/bin/a.py");
        observe_tool_event_with(&daemon, &pre(&x), &fl);
        observe_tool_event_with(&daemon, &fail(&g, Some("Exit code 1")), &fl);
        observe_tool_event_with(&daemon, &fail(&x, None), &fl);
        assert_eq!(starts.load(Ordering::SeqCst), 0, "a call, a mention or a failure without text started the flusher");
        for _ in 0..4 {
            observe_tool_event_with(&daemon, &pre(&x), &fl);
            observe_tool_event_with(&daemon, &fail(&x, Some("Exit code 1\nE: a")), &fl);
        }
        observe_tool_event_with(&daemon, &fail(&x, Some("Exit code 1\nE: a")), &fl);
        assert_eq!(starts.load(Ordering::SeqCst), 1, "the flusher must start once");
        assert!(registered_lanes().iter().any(|w| w.upgrade().is_some_and(|d| Arc::ptr_eq(&d, &daemon))));
        let w = Arc::downgrade(&daemon);
        let flushed = flusher_pass(&|| vec![w.clone(), std::sync::Weak::new()], &AlertConfig::default, &intake_root);
        assert_eq!(flushed, 1, "the live lane is flushed, the dead one skipped");
        let root = intake_root(&daemon);
        let rows = rows_in(&root, &LaneKey::of_socket(&daemon.socket_path));
        assert_eq!((rows.len(), rows[0].count), (1, 5));
        assert_eq!(flusher_pass(&Vec::<std::sync::Weak<Daemon>>::new, &AlertConfig::default, &intake_root), 0);
        let rep = FlushReport { due: 1, written_new: 1, ..FlushReport::default() };
        assert!(!log_flush(&daemon, &rep, 1.0), "the pass above logged — throttled");
        assert!(log_flush(&daemon, &rep, 1.0e6), "after the cooldown");
        assert!(!log_flush(&daemon, &FlushReport { due: 3, ..FlushReport::default() }, 2.0e6), "nothing happened → no line");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RULINGS BF6: the share is CONNECTED — with the router's general share (20/h) and clear reserve
    /// (26/h) full on this daemon, the detector still writes its rows, and the router's state is unchanged.
    #[test]
    fn full_general_share_and_clear_reserve_never_hold_an_intake_row() {
        let daemon = crate::team_gate_tests::tmp_daemon("t3b-full-router", false);
        let root = tmp_root("router");
        let mono = 5_000.0;
        {
            let mut st = daemon.alert_route.lock().unwrap();
            for i in 0..crate::alert_route::HOURLY_CAP {
                st.routed_window.push_back(mono - 1.0 - i as f64);
            }
            for i in 0..crate::alert_route::CLEAR_RESERVE {
                st.clear_window.push_back(mono - 1.0 - i as f64);
                st.routed_window.push_back(mono - 1.0 - i as f64);
            }
            assert_eq!((st.general_1h(mono), st.clear_1h(mono)), (crate::alert_route::HOURLY_CAP, crate::alert_route::CLEAR_RESERVE));
        }
        let before = format!("{:?}", *daemon.alert_route.lock().unwrap());
        for i in 0..3 {
            feed(&daemon, &format!("python3 ~/.cys/pack/bin/r{i}.py"), "Exit code 1\nE: r", 5);
        }
        let rep = flush_defect_intake(&daemon, &root, &AlertConfig::default(), crate::alert_route::Now::at(mono));
        assert_eq!((rep.written_new, rep.held_hourly, rep.held_cooldown), (3, 0, 0), "a full router share held an intake row: {rep:?}");
        assert_eq!(format!("{:?}", *daemon.alert_route.lock().unwrap()), before, "the detector moved the router's counters");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A 1-26 / D:658: no hook that runs on every tool call or every prompt files a report, and the
    /// common frame never talks to the daemon or the ledger (the observation event of `cys-hook.sh`
    /// is the only channel). The list = the per-call / per-prompt hooks (impact W2 CH-10 ③).
    #[test]
    fn no_hook_files_a_report_per_call() {
        let hooks = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("cysjavis-pack/hooks");
        let per_call = [
            "_lib.sh", "cys-hook.sh", "pre-dispatch.sh", "role-capability-gate.sh", "appbuild-gate.sh",
            "grill-gate.sh", "grill-arm.sh", "guard.sh", "pack-guard.sh", "commit-memory-nudge.sh",
            "grill-count.sh", "role-bootstrap.sh",
        ];
        let planted = "cys repair report --file x --error y";
        assert!(planted.contains("repair report"), "checker is blind");
        for name in per_call {
            let text = std::fs::read_to_string(hooks.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            for needle in ["repair report", "repair.report", "repair_report"] {
                assert!(!text.contains(needle), "{name} files a defect per call ({needle}) — D:658 forbids it");
            }
        }
        let lib = std::fs::read_to_string(hooks.join("_lib.sh")).unwrap();
        for needle in ["usage-event", "repair", "cys usage"] {
            assert!(!lib.contains(needle), "_lib.sh reaches `{needle}` — the frame is shared by 25 hooks (A 1-26)");
        }
    }
}

/// ★The observation hook's contract with a stub `cys` on a private PATH (no daemon, private HOME, no
/// overlays): exit 0, nothing on stdout, exactly ONE `cys usage-event-stdin` spawn per call and the
/// hook JSON — identity and failure text included — reaches it byte for byte; a `cys` that exits
/// without reading still leaves stdin drained; no `cys` at all is silent. (Appendix A 1-26 / 1-28;
/// the Windows rule "훅 안에서 두 번째 프로세스를 띄우지 않음" is the spawn count.)
#[cfg(all(test, unix))]
mod observation_hook_contract {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

    struct Rig {
        dir: std::path::PathBuf,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn rig(tag: &str, stub: Option<&str>) -> Rig {
        let dir = super::defect_intake_tests::tmp_root(&format!("hook-{tag}"));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("home/.cys/local/hooks")).unwrap();
        if let Some(body) = stub {
            let p = dir.join("bin/cys");
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Rig { dir }
    }

    /// Runs the real `cys-hook.sh` with `input` on stdin; returns (exit code, stdout, stdin fully written).
    fn run(r: &Rig, input: &[u8]) -> (i32, Vec<u8>, bool) {
        let hook = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("cysjavis-pack/hooks/cys-hook.sh");
        let mut child = Command::new("/bin/sh")
            .arg(&hook)
            .env_clear()
            .env("HOME", r.dir.join("home"))
            .env("PATH", format!("{}:/usr/bin:/bin", r.dir.join("bin").display()))
            .env("STUB_LOG", r.dir.join("calls.log"))
            .env("STUB_IN", r.dir.join("stdin.bin"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("sh");
        let mut stdin = child.stdin.take().unwrap();
        let wrote = stdin.write_all(input).is_ok();
        drop(stdin);
        let out = child.wait_with_output().expect("wait");
        (out.status.code().unwrap_or(-1), out.stdout, wrote)
    }

    const RECORDING_STUB: &str = "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$STUB_LOG\"\ncat >> \"$STUB_IN\"\nexit 0\n";

    #[test]
    fn cys_hook_carries_the_identity_in_one_silent_spawn() {
        let failure = serde_json::json!({
            "hook_event_name": "PostToolUseFailure", "tool_name": "Bash",
            "tool_input": {"command": "python3 ~/.cys/pack/bin/javis_x.py run"},
            "error": "Exit code 1\nKeyError: 'k'", "is_interrupt": false
        })
        .to_string();
        for (i, input) in [failure.clone(), "{not json".to_string(), String::new()].iter().enumerate() {
            let r = rig(&format!("rec{i}"), Some(RECORDING_STUB));
            for _ in 0..2 {
                let (code, stdout, wrote) = run(&r, input.as_bytes());
                assert_eq!(code, 0, "the observation hook must always exit 0");
                assert!(stdout.is_empty(), "the observation hook wrote to stdout: {stdout:?}");
                assert!(wrote);
            }
            let calls = std::fs::read_to_string(r.dir.join("calls.log")).unwrap_or_default();
            assert_eq!(calls, "usage-event-stdin\nusage-event-stdin\n", "one `cys` spawn per call, no other verb (input {i})");
            let got = std::fs::read(r.dir.join("stdin.bin")).unwrap_or_default();
            assert_eq!(got, [input.as_bytes(), input.as_bytes()].concat(), "the hook JSON must reach the CLI unchanged");
        }
    }

    #[test]
    fn cys_hook_drains_stdin_when_cys_fails_and_is_silent_without_cys() {
        let big = format!("{{\"hook_event_name\":\"PostToolUse\",\"tool_response\":\"{}\"}}", "x".repeat(1 << 20));
        let r = rig("fail", Some("#!/bin/sh\nexit 1\n"));
        let (code, stdout, wrote) = run(&r, big.as_bytes());
        assert_eq!((code, stdout.is_empty(), wrote), (0, true, true), "a failing cys must not leave the writer with EPIPE");
        let r = rig("none", None);
        let (code, stdout, wrote) = run(&r, big.as_bytes());
        assert_eq!((code, stdout.is_empty(), wrote), (0, true, true));
    }
}

/// ★The wiring, through the real request path: `usage.event` from a seat → the detector → a row after
/// the flush. Today's event shape (a failure only as `exit_code`, no text) files nothing.
#[cfg(all(test, unix))]
mod defect_intake_drills {
    use super::*;
    use cys::repair::types::LaneKey;
    use cys::Request;

    fn usage_event(daemon: &Arc<Daemon>, params: Value) -> Value {
        let req = Request { id: json!(1), method: "usage.event".into(), params };
        match crate::handlers::dispatch(daemon, req, None) {
            crate::handlers::Reply::Single(v) => v,
            _ => panic!("single reply"),
        }
    }

    #[test]
    fn usage_event_failure_reaches_the_detector_through_the_arm() {
        let daemon = crate::team_gate_tests::tmp_daemon("t3b-arm", false);
        let sid = crate::team_gate_tests::seat(&daemon, "worker", 4_242_001);
        let root = super::defect_intake_tests::tmp_root("arm");
        let input = json!({"command": "python3 ~/.cys/pack/bin/javis_x.py run"});
        let today = json!({"command": "python3 ~/.cys/pack/bin/javis_today.py"});
        for _ in 0..5 {
            for inp in [&input, &today] {
                let v = usage_event(&daemon, json!({"surface_id": sid, "event_type": "PRE_TOOL", "raw_hook_event": "PreToolUse", "tool_name": "Bash", "tool_input": inp}));
                assert_eq!(v["ok"], true, "{v}");
            }
            // CCR-1's shape
            let v = usage_event(&daemon, json!({"surface_id": sid, "event_type": "POST_TOOL", "raw_hook_event": "PostToolUseFailure", "tool_name": "Bash", "tool_input": input, "exit_code": 1, "error": "Exit code 1\nKeyError: 'k'"}));
            assert_eq!(v["ok"], true, "{v}");
            // today's shape (`tool_response.is_error` → exit_code 1, no text)
            let v = usage_event(&daemon, json!({"surface_id": sid, "event_type": "POST_TOOL", "raw_hook_event": "PostToolUse", "tool_name": "Bash", "tool_input": today, "exit_code": 1}));
            assert_eq!(v["ok"], true, "{v}");
            // an abort of the same script (`is_interrupt: true`) is not a failure of the script
            let v = usage_event(&daemon, json!({"surface_id": sid, "event_type": "POST_TOOL", "raw_hook_event": "PostToolUseFailure", "tool_name": "Bash", "tool_input": input, "exit_code": 1, "error": "Exit code 130\nKeyboardInterrupt", "is_interrupt": true}));
            assert_eq!(v["ok"], true, "{v}");
            // a mention: a failing grep of a pack file
            let v = usage_event(&daemon, json!({"surface_id": sid, "event_type": "POST_TOOL", "raw_hook_event": "PostToolUseFailure", "tool_name": "Bash", "tool_input": {"command": "grep x ~/.cys/pack/bin/javis_x.py"}, "exit_code": 1, "error": "Exit code 1"}));
            assert_eq!(v["ok"], true, "{v}");
        }
        let rep = flush_defect_intake(&daemon, &root, &AlertConfig::default(), crate::alert_route::Now::at(10.0));
        assert_eq!((rep.due, rep.written_new), (1, 1), "{rep:?}");
        let (defects, _) = cys::repair::intake::load_all_in(&root);
        assert_eq!(defects.len(), 1);
        assert_eq!((defects[0].rel.as_str(), defects[0].count), ("bin/javis_x.py", 5));
        assert_eq!(defects[0].lanes, vec![LaneKey::of_socket(&daemon.socket_path)]);
        let snap = defect_intake_snapshot(&daemon, 10.0);
        assert_eq!(snap["no_error_text"], 5, "{snap}");
        assert_eq!(snap["calls"], 10, "{snap}");
        assert_eq!(snap["failures"], 10, "the abort and the grep were counted as script failures: {snap}");
        assert_eq!(snap["failures_without_identity"], 5, "{snap}");
        let q = daemon.get_surface(sid).map(|s| s.pending_queue.lock().unwrap().len());
        assert_eq!(q, Some(0), "the seat's queue grew");
        let _ = std::fs::remove_dir_all(&root);
    }
}
