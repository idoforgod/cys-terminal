//! T7 E6 경보 엔진 — 임계값·반복실패 경보의 순수 평가기 + 설정 로딩.
//! governance.rs watchdog가 에지 디바운스로 발화(능동 경보)하고, control.alerts RPC가 같은
//! 평가기로 현재 상태(UI 배지)를 노출한다 — 단일 진실원으로 둘이 갈라지지 않게.
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

/// 평가 입력 스냅샷 — 호출부(watchdog/RPC)가 락 잡고 수집(평가기는 락 무관·순수).
#[derive(Default)]
pub struct Snapshot {
    pub rates: Vec<(String, String, f64)>,        // (role, label, used_pct)
    /// CC v2 WS-A: 계정 단위 rate — (계정 라벨, 창, used_pct). 관측된 계정만.
    pub account_rates: Vec<(String, String, f64)>,
    pub weekly_cost_usd: f64,
    pub weekly_tokens: u64,
    pub tool_failures: Vec<(String, u64, u64, f64)>, // (tool, calls, fail, fail_rate)
    /// ★T-0147-2 A8: 델타게이트 배지(파일 oracle) + 데몬 게이트 신호(state 외부 oracle).
    /// 가산 필드 — `..Snapshot::default()` 호출부는 빈 벡터를 받아 무회귀다.
    pub gate_badges: Vec<GateBadge>,
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
            out.push(Alert {
                kind: "rate_limit".into(),
                key: format!("rate_limit:{role}:{label}"),
                severity: if *pct >= 95.0 { "crit" } else { "warn" }.into(),
                message: format!("{role} {label} rate {:.0}%", pct),
                detail: json!({"role": role, "label": label, "used_pct": pct}),
            });
        }
    }
    // 1b. CC v2: 계정 단위 rate (에지 디바운스 키 = 계정 라벨 — 같은 계정 다중 노드 중복발화 0)
    for (label, win, pct) in &snap.account_rates {
        if *pct >= cfg.account_warn_pct {
            out.push(Alert {
                kind: "account_rate".into(),
                key: format!("account_rate:{label}:{win}"),
                severity: if *pct >= cfg.account_crit_pct { "crit" } else { "warn" }.into(),
                message: format!("계정 {label} {win} rate {:.0}%", pct),
                detail: json!({"account": label, "win": win, "used_pct": pct}),
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
pub fn snapshot(daemon: &Arc<Daemon>, now: f64) -> Snapshot {
    let mut rates = Vec::new();
    {
        let surfaces = daemon.surfaces.lock().unwrap();
        for s in surfaces.values() {
            if s.exited.load(Ordering::Relaxed) {
                continue;
            }
            let role = s.role.lock().unwrap().clone().unwrap_or_else(|| "?".into());
            if let Some(u) = s.observed_usage.lock().unwrap().as_ref() {
                for w in &u.rate {
                    rates.push((role.clone(), w.label.clone(), w.used_pct));
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
    let account_rates = crate::accounts::alert_rates(daemon);
    // ★T-0147-2 A8 + N6b: 두 oracle 을 합친다 —
    //   파일 oracle(레인별 badges.json) = 게이트가 state 를 쓸 수 있을 때의 정상 경로,
    //   state 외부 oracle(gate_signal_badges) = 바로 그 state 를 못 쓸 때의 유일한 경로.
    // 합친 뒤 key 로 재정렬해야 evaluate() 이전에도 결정론이 선다.
    let mut gate_badges = collect_gate_badges(&crate::state::state_dir(&daemon.socket_path), now);
    gate_badges.extend(gate_signal_badges(now));
    gate_badges.sort_by(|a, b| a.key.cmp(&b.key));
    Snapshot { rates, account_rates, weekly_cost_usd, weekly_tokens, tool_failures, gate_badges }
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
