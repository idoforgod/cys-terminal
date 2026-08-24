//! CC v2 WS-A: 계정 단위 rate limit 집계 — 노드(surface) 관측을 **계정** 차원으로 귀속한다.
//!
//! 핵심 사실(실측 2026-07-16):
//! - 계정 식별자 = 프로필 dir이 아니라 `<dir>/.claude.json`의 `oauthAccount.accountUuid`.
//!   프로필 dir은 계정에 N:1이다(~/.claude·~/.claude-work·~/.cys/claude* 가 같은 계정인 식).
//! - claude rate의 유일한 생산자는 statusline(usage.report)이다 — usage.rs claude transcript
//!   분기는 rate를 **이월**하며 updated_at을 현재로 갱신하므로, 여기(note_rate)에는
//!   **신선 생산된 rate만** 넘긴다(이월분 수용 시 stale이 최신으로 둔갑).
//! - 병합 = 창 벡터 통째 최신 승자(같은 계정 풀은 최신 관측이 진실).
//!
//! 잠금 순서 불변식: accounts → (해제) → analytics. 역순 금지(교착).

use crate::state::Daemon;
use crate::usage::RateWindow;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 스냅샷 영속 스로틀 — 같은 (계정,창)에서 pct 변화가 이 미만이면 INSERT 생략.
const SNAPSHOT_MIN_DELTA_PCT: f64 = 1.0;
/// 스냅샷 보존 창(초) — 초과분은 prune. 30일.
const SNAPSHOT_RETAIN_SECS: f64 = 30.0 * 86400.0;
/// prune 주기(초) — note 경로에서 저빈도 수행. 6시간.
const PRUNE_INTERVAL_SECS: f64 = 6.0 * 3600.0;
/// 부트 복원 창(초) — 이 안의 마지막 스냅샷으로 계정 뷰를 예열(stale 표시). 7일.
const BOOT_RESTORE_SECS: f64 = 7.0 * 86400.0;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountKey {
    pub provider: String,   // "claude" | "codex" | "antigravity" | (accounts.json 선언 provider)
    pub account_id: String, // claude: accountUuid · 그 외 단일 홈: "default"
}

#[derive(Clone, Debug)]
pub struct AccountView {
    pub key: AccountKey,
    pub label: String,        // claude: 이메일 · codex: "OpenAI Codex" · agy: "Antigravity (agy)"
    pub plan: Option<String>, // oauthAccount rate limit tier — 값이 있을 때만 UI 표시
    pub profiles: BTreeSet<String>, // 이 계정으로 관측된 프로필 dir들(홈 상대 표기)
    pub rate: Vec<RateWindow>,
    pub updated_at: f64, // 0.0 = 관측 전(발견만)
    pub source: String,  // "statusline" | "rollout" | "agy-rpc" | "adapter:<p>" | "snapshot"(부트 복원)
    pub adapter: bool,   // false = 관측 어댑터 없음(accounts.json adapter:"none" 선언 계정)
}

struct IdentEntry {
    mtime: f64,
    ident: Option<(String, String, Option<String>)>, // (accountUuid, email, plan)
}

#[derive(Default)]
pub struct AccountsState {
    views: HashMap<AccountKey, AccountView>,
    ident_cache: HashMap<PathBuf, IdentEntry>,
    last_persisted: HashMap<(AccountKey, String), f64>, // (key, 창 라벨) → 마지막 기록 pct
    last_prune: f64,
}

/// 세션 파일 경로 → 프로필 dir (`…/<profile>/projects/<munged>/<sess>.jsonl`의 profile 부분).
/// `/projects/` 마커 앞이 프로필 dir — 홈 `~/.claude*`와 `~/.cys/claude*` 모두 커버.
pub fn profile_dir_from_session(path: &str) -> Option<PathBuf> {
    let norm = path.replace('\\', "/");
    let idx = norm.find("/projects/")?;
    if idx == 0 {
        return None;
    }
    Some(PathBuf::from(&norm[..idx]))
}

/// 프로필 dir의 홈 상대 표기 (라벨·중복 제거용 — 계정 식별에는 쓰지 않는다)
fn profile_short(dir: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rel) = dir.strip_prefix(&home) {
            return rel.to_string_lossy().into_owned();
        }
    }
    dir.to_string_lossy().into_owned()
}

/// `<dir>/.claude.json` → oauthAccount 신원. 잡동사니 dir(.claude-worktrees·백업 등)은
/// 파일 부재/uuid 부재로 None → 관측 미귀속(유령 계정 0). 자격증명(.credentials.json)은 읽지 않는다.
fn claude_identity(
    state: &mut AccountsState,
    dir: &Path,
) -> Option<(String, String, Option<String>)> {
    let f = dir.join(".claude.json");
    let mtime = std::fs::metadata(&f)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())?;
    if let Some(e) = state.ident_cache.get(dir) {
        if e.mtime == mtime {
            return e.ident.clone();
        }
    }
    let ident = std::fs::read_to_string(&f)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| {
            let oa = v.get("oauthAccount")?;
            let uuid = oa.get("accountUuid")?.as_str()?.to_string();
            let email = oa
                .get("emailAddress")
                .and_then(|x| x.as_str())
                .unwrap_or(&uuid)
                .to_string();
            let plan = oa
                .get("userRateLimitTier")
                .or_else(|| oa.get("organizationRateLimitTier"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            Some((uuid, email, plan))
        });
    state
        .ident_cache
        .insert(dir.to_path_buf(), IdentEntry { mtime, ident: ident.clone() });
    ident
}

/// agent + 세션 파일 → (키, 라벨, plan, 프로필 표기). claude는 신원 해석 실패 시 None(스킵).
fn resolve(
    state: &mut AccountsState,
    agent: &str,
    session_file: &str,
) -> Option<(AccountKey, String, Option<String>, Option<String>)> {
    match agent {
        "claude" => {
            let dir = profile_dir_from_session(session_file)?;
            let (uuid, email, plan) = claude_identity(state, &dir)?;
            Some((
                AccountKey { provider: "claude".into(), account_id: uuid },
                email,
                plan,
                Some(profile_short(&dir)),
            ))
        }
        "codex" => Some((
            AccountKey { provider: "codex".into(), account_id: "default".into() },
            "OpenAI Codex".into(),
            None,
            Some(".codex".into()),
        )),
        "gemini" | "agy" | "antigravity" => Some((
            AccountKey { provider: "antigravity".into(), account_id: "default".into() },
            "Antigravity (agy)".into(),
            None,
            Some(".antigravity".into()),
        )),
        _ => None,
    }
}

/// 신선 생산된 rate 관측을 계정에 귀속·병합하고 스냅샷을 영속한다(스로틀·prune 포함).
/// **호출 계약: rate는 이번 관측이 실제 생산한 값만** — 이월(carryover) 금지(모듈 헤더 참조).
pub fn note_rate(
    daemon: &Arc<Daemon>,
    agent: &str,
    session_file: &str,
    rate: &[RateWindow],
    source: &str,
    now: f64,
) {
    if rate.is_empty() {
        return;
    }
    // 1) accounts 락 안에서 병합 + 영속 대상 수집 (analytics 락은 여기서 잡지 않는다 — 잠금 순서)
    let mut to_persist: Vec<(AccountKey, String, String, f64, Option<f64>)> = Vec::new();
    let mut do_prune = false;
    {
        let mut st = daemon.accounts.lock().unwrap();
        let Some((key, label, plan, profile)) = resolve(&mut st, agent, session_file) else {
            return; // 미귀속(신원 불명) — 유령 계정을 만들지 않는다
        };
        let view = st.views.entry(key.clone()).or_insert_with(|| AccountView {
            key: key.clone(),
            label: label.clone(),
            plan: plan.clone(),
            profiles: BTreeSet::new(),
            rate: Vec::new(),
            updated_at: 0.0,
            source: String::new(),
            adapter: true,
        });
        view.label = label;
        if plan.is_some() {
            view.plan = plan;
        }
        if let Some(p) = profile {
            view.profiles.insert(p);
        }
        // 최신 승자 — note는 신선 생산분만 받으므로 timestamp 비교로 충분
        if now >= view.updated_at {
            view.rate = rate.to_vec();
            view.updated_at = now;
            view.source = source.into();
        }
        for w in rate {
            let pk = (key.clone(), w.label.clone());
            let prev = st.last_persisted.get(&pk).copied();
            if prev.map_or(true, |p| (w.used_pct - p).abs() >= SNAPSHOT_MIN_DELTA_PCT) {
                st.last_persisted.insert(pk, w.used_pct);
                to_persist.push((
                    key.clone(),
                    st.views[&key].label.clone(),
                    w.label.clone(),
                    w.used_pct,
                    w.resets_at,
                ));
            }
        }
        if now - st.last_prune > PRUNE_INTERVAL_SECS {
            st.last_prune = now;
            do_prune = true;
        }
    }
    // 2) analytics 영속 (accounts 락 해제 후)
    if to_persist.is_empty() && !do_prune {
        return;
    }
    let guard = daemon.analytics.lock().unwrap();
    if let Some(conn) = guard.as_ref() {
        for (key, label, win, pct, resets) in &to_persist {
            crate::analytics::record_rate_snapshot(
                conn, now, &key.provider, &key.account_id, label, win, *pct, *resets,
            );
        }
        if do_prune {
            crate::analytics::prune_rate_snapshots(conn, now - SNAPSHOT_RETAIN_SECS);
        }
    }
}

/// 부트 시드 — ① 알려진 프로필 dir 스캔으로 계정 **발견**(관측 전에도 3계정이 다 보이게),
/// ② analytics 마지막 스냅샷(7d)으로 rate 예열(source:"snapshot"·stale 표시),
/// ③ ~/.cys/accounts.json 선언 계정 등록(미래 provider — adapter:"none"은 '관측 없음' 상주).
pub fn seed_known(daemon: &Arc<Daemon>) {
    if let Some(home) = dirs::home_dir() {
        // ★(U-17) 프로필 dir 열거 규칙은 **lib 정본 하나**다(`cys::profile_gate`). 종전엔 이
        //   함수 안에만 있었고, 인증 판정기가 같은 규칙을 재구현하면 두 벌이 갈린다(한쪽만
        //   새 부서 접두를 배우는 식) — 같은 목록을 두 소비처가 보게 한다.
        //   ★판정은 바뀌지 않는다: 정본 함수는 종전 두 루프와 **같은 이름 규칙·같은 순서**이며
        //   `is_dir()` 검사도 더하지 않는다(동작 동일성 유지 — 완화도 강화도 아니다).
        let dirs_to_check: Vec<PathBuf> = cys::profile_gate::enumerate_profile_dirs(&home);
        {
            let mut st = daemon.accounts.lock().unwrap();
            for dir in dirs_to_check {
                if let Some((uuid, email, plan)) = claude_identity(&mut st, &dir) {
                    let key = AccountKey { provider: "claude".into(), account_id: uuid };
                    let short = profile_short(&dir);
                    let v = st.views.entry(key.clone()).or_insert_with(|| AccountView {
                        key,
                        label: email.clone(),
                        plan: plan.clone(),
                        profiles: BTreeSet::new(),
                        rate: Vec::new(),
                        updated_at: 0.0,
                        source: String::new(),
                        adapter: true,
                    });
                    v.profiles.insert(short);
                }
            }
            if home.join(".codex").is_dir() {
                st.views
                    .entry(AccountKey { provider: "codex".into(), account_id: "default".into() })
                    .or_insert_with(|| AccountView {
                        key: AccountKey { provider: "codex".into(), account_id: "default".into() },
                        label: "OpenAI Codex".into(),
                        plan: None,
                        profiles: BTreeSet::from([".codex".to_string()]),
                        rate: Vec::new(),
                        updated_at: 0.0,
                        source: String::new(),
                        adapter: true,
                    });
            }
            if home.join(".antigravity").is_dir() {
                st.views
                    .entry(AccountKey {
                        provider: "antigravity".into(),
                        account_id: "default".into(),
                    })
                    .or_insert_with(|| AccountView {
                        key: AccountKey {
                            provider: "antigravity".into(),
                            account_id: "default".into(),
                        },
                        label: "Antigravity (agy)".into(),
                        plan: None,
                        profiles: BTreeSet::from([".antigravity".to_string()]),
                        rate: Vec::new(),
                        updated_at: 0.0,
                        source: String::new(),
                        adapter: true,
                    });
            }
        }
        // 선언 계정(~/.cys/accounts.json — pack 밖: pack 스윕/치유 사정권 회피)
        let decl = home.join(".cys/accounts.json");
        if let Ok(s) = std::fs::read_to_string(&decl) {
            if let Ok(v) = serde_json::from_str::<Value>(&s) {
                let mut st = daemon.accounts.lock().unwrap();
                for a in v.get("accounts").and_then(|x| x.as_array()).into_iter().flatten() {
                    let Some(provider) = a.get("provider").and_then(|x| x.as_str()) else {
                        continue;
                    };
                    let label = a
                        .get("label")
                        .and_then(|x| x.as_str())
                        .unwrap_or(provider)
                        .to_string();
                    let adapter =
                        a.get("adapter").and_then(|x| x.as_str()).unwrap_or("none") != "none";
                    let key =
                        AccountKey { provider: provider.into(), account_id: "default".into() };
                    st.views.entry(key.clone()).or_insert_with(|| AccountView {
                        key,
                        label,
                        plan: None,
                        profiles: BTreeSet::new(),
                        rate: Vec::new(),
                        updated_at: 0.0,
                        source: String::new(),
                        adapter,
                    });
                }
            }
        }
    }
    // 마지막 스냅샷으로 예열 — updated_at은 스냅샷 시각 그대로(신선한 척 금지)
    let rows = {
        let guard = daemon.analytics.lock().unwrap();
        guard.as_ref().map(|conn| {
            crate::analytics::last_rate_snapshots(
                conn,
                crate::state::now_epoch() - BOOT_RESTORE_SECS,
            )
        })
    };
    if let Some(rows) = rows {
        let mut st = daemon.accounts.lock().unwrap();
        for (ts, provider, account, label, win, pct, resets) in rows {
            let key = AccountKey { provider, account_id: account };
            let v = st.views.entry(key.clone()).or_insert_with(|| AccountView {
                key,
                label: label.clone(),
                plan: None,
                profiles: BTreeSet::new(),
                rate: Vec::new(),
                updated_at: 0.0,
                source: String::new(),
                adapter: true,
            });
            // 라이브 관측 전(발견만·또는 스냅샷 예열 중)에만 덮는다 — 신선 관측 우선.
            let seeded = v.source.is_empty() || v.source == "snapshot";
            if seeded {
                if let Some(w) = v.rate.iter_mut().find(|w| w.label == win) {
                    w.used_pct = pct;
                    w.resets_at = resets;
                } else {
                    v.rate.push(RateWindow { label: win, used_pct: pct, resets_at: resets });
                }
                v.source = "snapshot".into();
                if ts > v.updated_at {
                    v.updated_at = ts;
                }
            }
        }
    }
}

/// accounts.json의 adapter:"cmd" 계정 — 주기 실행해 rate JSON을 흡수하는 범용 풀 어댑터.
/// 출력 계약: `[{"label":"5h","used_pct":12.3,"resets_at":1234.0}, …]`. grok/GLM CLI 합류 지점.
pub fn spawn_custom_adapters(daemon: Arc<Daemon>) {
    let Some(home) = dirs::home_dir() else { return };
    let decl = home.join(".cys/accounts.json");
    let Ok(s) = std::fs::read_to_string(&decl) else { return };
    let Ok(v) = serde_json::from_str::<Value>(&s) else { return };
    for a in v.get("accounts").and_then(|x| x.as_array()).into_iter().flatten() {
        let (Some(provider), Some(cmd)) = (
            a.get("provider").and_then(|x| x.as_str()).map(|s| s.to_string()),
            a.get("cmd").and_then(|x| x.as_str()).map(|s| s.to_string()),
        ) else {
            continue;
        };
        if a.get("adapter").and_then(|x| x.as_str()) != Some("cmd") {
            continue;
        }
        let interval = a
            .get("interval_secs")
            .and_then(|x| x.as_u64())
            .unwrap_or(300)
            .max(60);
        let d = daemon.clone();
        tokio::spawn(async move {
            loop {
                // 플랫폼별 셸 위임 — Windows는 sh 부재(cmd /C). 실패는 무해(다음 주기 재시도).
                let fut = if cfg!(windows) {
                    tokio::process::Command::new("cmd").args(["/C", &cmd]).output()
                } else {
                    tokio::process::Command::new("sh").args(["-c", &cmd]).output()
                };
                if let Ok(Ok(out)) =
                    tokio::time::timeout(std::time::Duration::from_secs(10), fut).await
                {
                    if out.status.success() {
                        if let Ok(arr) = serde_json::from_slice::<Value>(&out.stdout) {
                            let rate: Vec<RateWindow> = arr
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|w| {
                                    Some(RateWindow {
                                        label: w.get("label")?.as_str()?.to_string(),
                                        used_pct: w.get("used_pct")?.as_f64()?,
                                        resets_at: w.get("resets_at").and_then(|x| x.as_f64()),
                                    })
                                })
                                .collect();
                            if !rate.is_empty() {
                                let now = crate::state::now_epoch();
                                let src = format!("adapter:{provider}");
                                note_custom(&d, &provider, &rate, &src, now);
                            }
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            }
        });
    }
}

/// 선언 provider(비 내장) 계정에 rate 반영 — note_rate의 resolve를 우회하는 직접 키 경로.
fn note_custom(daemon: &Arc<Daemon>, provider: &str, rate: &[RateWindow], source: &str, now: f64) {
    let mut st = daemon.accounts.lock().unwrap();
    let key = AccountKey { provider: provider.into(), account_id: "default".into() };
    let label = st.views.get(&key).map(|v| v.label.clone()).unwrap_or_else(|| provider.into());
    let v = st.views.entry(key.clone()).or_insert_with(|| AccountView {
        key,
        label,
        plan: None,
        profiles: BTreeSet::new(),
        rate: Vec::new(),
        updated_at: 0.0,
        source: String::new(),
        adapter: true,
    });
    if now >= v.updated_at {
        v.rate = rate.to_vec();
        v.updated_at = now;
        v.source = source.into();
        v.adapter = true;
    }
}

/// 소진 예측 최소 표본 수·스팬(초) — 미달 시 예측 미표시(표본 2개 기울기의 황당 예측 차단).
const PREDICT_MIN_POINTS: usize = 3;
const PREDICT_MIN_SPAN_SECS: f64 = 600.0;
/// 예측 대상 신선도(초) — stale 관측으로 예측하지 않는다.
const PREDICT_FRESH_SECS: f64 = 600.0;

/// 로컬 계정 뷰 → JSON 배열 (usage.accounts RPC·control.dashboard "accounts" 공용).
/// stale_secs는 읽기 시점 계산 — updated_at==0.0은 null(관측 전)로 정직 표기.
/// 5h 창에는 소진 예측(exhaust_at)을 붙인다 — 최근 60분 선형 기울기, 표본 미달·기울기≤0·
/// 리셋 후 소진이면 생략(정직한 공백). 잠금 순서: accounts → 해제 → analytics.
pub fn local_json(daemon: &Arc<Daemon>, now: f64) -> Value {
    let mut rows: Vec<Value> = {
        let st = daemon.accounts.lock().unwrap();
        let mut views: Vec<&AccountView> = st.views.values().collect();
        views.sort_by(|a, b| a.key.cmp(&b.key));
        views
            .into_iter()
            .map(|v| {
                json!({
                    "provider": v.key.provider,
                    "account_id": v.key.account_id,
                    "label": v.label,
                    "plan": v.plan,
                    "profiles": v.profiles.iter().collect::<Vec<_>>(),
                    "rate": v.rate,
                    "updated_at": if v.updated_at > 0.0 { json!(v.updated_at) } else { Value::Null },
                    "stale_secs": if v.updated_at > 0.0 { json!((now - v.updated_at).max(0.0)) } else { Value::Null },
                    "source": v.source,
                    "adapter": v.adapter,
                })
            })
            .collect()
    };
    // 소진 예측 — 신선(≤10분) 계정의 5h 창만. accounts 락 해제 후 analytics 조회(잠금 순서).
    let guard = daemon.analytics.lock().unwrap();
    if let Some(conn) = guard.as_ref() {
        for row in rows.iter_mut() {
            let fresh = row["stale_secs"].as_f64().map(|s| s <= PREDICT_FRESH_SECS).unwrap_or(false);
            if !fresh {
                continue;
            }
            let (provider, account) = (
                row["provider"].as_str().unwrap_or("").to_string(),
                row["account_id"].as_str().unwrap_or("").to_string(),
            );
            let resets_at = row["rate"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|w| w["label"] == "5h")
                .and_then(|w| w["resets_at"].as_f64());
            let series = crate::analytics::rate_series(conn, &provider, &account, "5h", now - 3600.0);
            if let Some(t) = predict_exhaust(&series, now, resets_at) {
                row["exhaust_at"] = json!(t);
            }
        }
    }
    Value::Array(rows)
}

/// 선형 소진 예측(순수 — 테스트 핀): 시계열 최소자승 기울기로 100% 도달 시각.
/// None = 표본 미달·스팬 미달·기울기≤0·이미 100%·예측이 리셋 이후(리셋이 먼저면 무의미).
pub fn predict_exhaust(series: &[(f64, f64)], now: f64, resets_at: Option<f64>) -> Option<f64> {
    if series.len() < PREDICT_MIN_POINTS {
        return None;
    }
    let span = series.last()?.0 - series.first()?.0;
    if span < PREDICT_MIN_SPAN_SECS {
        return None;
    }
    let n = series.len() as f64;
    let (sx, sy): (f64, f64) = series.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (mx, my) = (sx / n, sy / n);
    let (mut num, mut den) = (0.0, 0.0);
    for (x, y) in series {
        num += (x - mx) * (y - my);
        den += (x - mx) * (x - mx);
    }
    if den <= 0.0 {
        return None;
    }
    let slope = num / den; // %/초
    let last = series.last()?;
    if slope <= 0.0 || last.1 >= 100.0 {
        return None;
    }
    let t = last.0 + (100.0 - last.1) / slope;
    if t <= now {
        return None;
    }
    match resets_at {
        Some(r) if t >= r => None, // 리셋이 먼저 — 소진 경고 무의미
        _ => Some(t),
    }
}

/// alerts용 스냅샷: (라벨, 창, pct) — 관측된 계정만.
pub fn alert_rates(daemon: &Arc<Daemon>) -> Vec<(String, String, f64)> {
    let st = daemon.accounts.lock().unwrap();
    let mut out = Vec::new();
    for v in st.views.values() {
        if v.updated_at == 0.0 {
            continue;
        }
        for w in &v.rate {
            out.push((v.label.clone(), w.label.clone(), w.used_pct));
        }
    }
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cys-acct-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn profile_dir_extraction() {
        assert_eq!(
            profile_dir_from_session("/Users/x/.claude-work/projects/-a/s.jsonl"),
            Some(PathBuf::from("/Users/x/.claude-work"))
        );
        assert_eq!(
            profile_dir_from_session("/Users/x/.cys/claude-default-dept-2/projects/-a/s.jsonl"),
            Some(PathBuf::from("/Users/x/.cys/claude-default-dept-2"))
        );
        assert_eq!(profile_dir_from_session("no-projects-marker.jsonl"), None);
        // Windows 역슬래시 경로 내성
        assert_eq!(
            profile_dir_from_session("C:\\Users\\x\\.claude\\projects\\-a\\s.jsonl"),
            Some(PathBuf::from("C:/Users/x/.claude"))
        );
    }

    #[test]
    fn identity_parse_and_junk_dir_skip() {
        let dir = tmp("ident");
        // 정상 프로필
        std::fs::write(
            dir.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"u-1","emailAddress":"a@b.c","userRateLimitTier":"max_5x"}}"#,
        )
        .unwrap();
        let mut st = AccountsState::default();
        let got = claude_identity(&mut st, &dir).unwrap();
        assert_eq!(got, ("u-1".into(), "a@b.c".into(), Some("max_5x".into())));
        // 캐시 적중(mtime 동일 → 재파싱 없이 동일 결과)
        assert_eq!(claude_identity(&mut st, &dir).unwrap().0, "u-1");
        // 잡동사니 dir(.claude.json 없음) → None
        let junk = tmp("junk");
        assert!(claude_identity(&mut st, &junk).is_none());
        // uuid 없는 파손 파일 → None (유령 계정 0)
        let broken = tmp("broken");
        std::fs::write(broken.join(".claude.json"), r#"{"oauthAccount":{}}"#).unwrap();
        assert!(claude_identity(&mut st, &broken).is_none());
    }

    #[test]
    fn predict_exhaust_pins() {
        // 표본 미달(2개) → None
        assert!(predict_exhaust(&[(0.0, 10.0), (600.0, 20.0)], 700.0, None).is_none());
        // 스팬 미달(<600s) → None
        assert!(
            predict_exhaust(&[(0.0, 10.0), (100.0, 20.0), (200.0, 30.0)], 300.0, None).is_none()
        );
        // 정상: 0→60%가 3600초 — 100% 도달 ≈ 6000초
        let s = [(0.0, 0.0), (1800.0, 30.0), (3600.0, 60.0)];
        let t = predict_exhaust(&s, 3600.0, None).unwrap();
        assert!((t - 6000.0).abs() < 1.0, "t={t}");
        // 리셋이 소진보다 먼저 → None
        assert!(predict_exhaust(&s, 3600.0, Some(5000.0)).is_none());
        // 감소 추세(slope≤0) → None
        assert!(
            predict_exhaust(&[(0.0, 60.0), (1800.0, 40.0), (3600.0, 20.0)], 3600.0, None)
                .is_none()
        );
    }

    #[test]
    fn resolve_agents() {
        let mut st = AccountsState::default();
        // codex/agy는 세션 파일 불요·단일 계정
        let (k, l, _, _) = resolve(&mut st, "codex", "").unwrap();
        assert_eq!((k.provider.as_str(), k.account_id.as_str()), ("codex", "default"));
        assert_eq!(l, "OpenAI Codex");
        let (k, ..) = resolve(&mut st, "gemini", "").unwrap();
        assert_eq!(k.provider, "antigravity");
        // 미지 agent → None
        assert!(resolve(&mut st, "mystery", "").is_none());
        // claude인데 신원 해석 불가 → None(스킵 — 유령 계정 금지)
        assert!(resolve(&mut st, "claude", "/nonexist/projects/x/s.jsonl").is_none());
    }
}
