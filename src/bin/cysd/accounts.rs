//! CC v2 WS-A: 계정 단위 rate limit 집계 — 노드(surface) 관측을 **계정** 차원으로 귀속한다.
//!
//! 핵심 사실(실측 2026-07-16):
//! - 계정 식별자 = 프로필 dir이 아니라 `<dir>/.claude.json`의 `oauthAccount.accountUuid`.
//!   프로필 dir은 계정에 N:1이다(~/.claude·~/.claude-work·~/.cys/claude* 가 같은 계정인 식).
//!   ★예외(0.14.42): `CLAUDE_CONFIG_DIR` 없이 띄운 기본 프로필 `~/.claude` 의 신원은 홈 직하
//!   `~/.claude.json` 이다 — 위치 규칙 정본은 `cys::profile_gate::identity_config_file`.
//! - 발견 대상(부트 시드): claude 프로필 dir 전부 · `~/.codex` · agy 데이터 폴더
//!   `~/.gemini/antigravity-cli`(구 `~/.antigravity` 호환) · `~/.cys/accounts.json` 선언 계정.
//!   관측이 없어도 행은 있다(updated_at null) — 화면은 관측 전 계정도 한 줄씩 보인다.
//! - claude rate의 유일한 생산자는 statusline(usage.report)이다 — usage.rs claude transcript
//!   분기는 rate를 **이월**하며 updated_at을 현재로 갱신하므로, 여기(note_rate)에는
//!   **신선 생산된 rate만** 넘긴다(이월분 수용 시 stale이 최신으로 둔갑).
//!   ★0.14.42 RC4-b: cys 창 **밖** Claude 세션의 statusline 도 계정 전용 입구(`usage.report_account` →
//!   [`report_outside`])로 들어온다(source "statusline-outside"). 좌석·배지·이벤트·임계는 건드리지 않고
//!   `note_rate` 하나만 부른다. 이 값은 **표시용**이다 — 같은 UID 의 아무 프로세스나 보낼 수 있으므로
//!   계정 경보(`alert_rates`)의 근거로 쓰지 않는다(오너 승인 2026-09-23 "표시용 값 · 위조 한계 문서화").
//!   ★fix-values-1: 경보 입력은 표시 승자와 **따로** 보관한다(`AccountsState::alert_inputs` — 창 밖이 아닌 출처의
//!   마지막 관측). 창 밖 값은 경보 입력을 만들지도 지우지도 않는다(억제·재발화 둘 다 차단). 스냅샷에도 출처를
//!   실어 재시작 뒤 복원도 같은 규칙을 따른다.
//! - ★0.14.42 RC2-b: agy 값의 주 경로는 agy 상태줄 훅(좌석 `usage.report` · source "agy-statusline")이다.
//! - 병합 = 창 벡터 통째 최신 승자(같은 계정 풀은 최신 관측이 진실).
//!
//! 잠금 순서 불변식: accounts → (해제) → analytics. 역순 금지(교착).

use crate::state::{Daemon, HideConsole};
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
    /// 관측 경로 고장 코드(예: "agy_http_403") — '관측 전'과 '경로가 고장나 못 읽음'을 구별한다.
    /// 신선 관측(note_rate)이 오면 지워진다. 지금은 agy-rpc 경로만 채운다(다른 경로는 데몬이 실패를 못 본다).
    pub source_error: Option<String>,
}

struct IdentEntry {
    file: PathBuf, // 실제로 읽은 신원 파일 — 기본 프로필은 폴더 밖(홈 직하)일 수 있다
    mtime: f64,
    ident: Option<(String, String, Option<String>)>, // (accountUuid, email, plan)
}

#[derive(Default)]
pub struct AccountsState {
    views: HashMap<AccountKey, AccountView>,
    ident_cache: HashMap<PathBuf, IdentEntry>,
    last_persisted: HashMap<(AccountKey, String), f64>, // (key, 창 라벨) → 마지막 기록 pct(출처 무관)
    /// 같은 키의 마지막 기록 pct 중 **경보 입력 출처**([`feeds_alerts`])의 것 — 창 밖 값과 좌석 값이 1%p 안으로
    /// 번갈아 와도 좌석 쪽 최신 값이 스냅샷에 남게 한다(재시작 뒤 경보 입력 복원의 정확도).
    last_persisted_alert: HashMap<(AccountKey, String), f64>,
    /// 계정 경보 입력 — 계정별 **창 밖이 아닌** 출처(좌석 statusline·rollout·agy·어댑터)의 마지막 신선 관측
    /// (관측 시각, rate). 표시 승자(`AccountView.rate` — 창 벡터 통째 최신 승자)와 **분리**한다(fix-values-1 SP-1·F1):
    /// 표시 승자에 경보 제외를 걸면 창 밖 보고 한 건이 좌석이 본 값을 경보에서 통째로 지우고(억제), 좌석 보고가
    /// 다시 최신이 되면 check_alerts 가 재무장된 키를 곧바로 다시 발화한다(30분 리마인드 우회 · 깜빡임).
    alert_inputs: HashMap<AccountKey, (f64, Vec<RateWindow>)>,
    last_prune: f64,
    /// cys 창 밖 보고의 빈도 상한 상태 — (정규화된 프로필 dir) → (마지막 수용 시각, 그때의 rate).
    /// 키 공간은 검증을 통과한 **알려진 프로필 dir** 뿐이라 크기가 유계다.
    outside_last: HashMap<PathBuf, (f64, Vec<RateWindow>)>,
}

/// agy 상태줄 훅이 좌석 `usage.report` 로 보낸 값의 계정 출처 라벨.
pub const AGY_STATUSLINE_SOURCE: &str = "agy-statusline";
/// cys 창 밖 Claude 세션(`usage.report_account`)이 보낸 값의 계정 출처 라벨 — 표시용(경보 제외).
pub const OUTSIDE_SOURCE: &str = "statusline-outside";
/// 창 밖 보고 `session_file` 길이 상한(바이트).
const OUTSIDE_SESSION_FILE_MAX: usize = 1024;
/// 창 밖 보고 rate 배열 원소 상한(5h·7d 둘 + 여유) — 넘으면 통째 거절.
pub const OUTSIDE_RATE_MAX_ENTRIES: usize = 4;
/// 창 밖 보고: 같은 프로필의 수용 간격 하한(초) — 값이 바뀌어도 이보다 잦으면 버린다.
const OUTSIDE_MIN_INTERVAL_SECS: f64 = 1.0;
/// 창 밖 보고: 같은 프로필·같은 값이면 이 창 안의 반복을 버린다(초).
const OUTSIDE_SAME_VALUE_SECS: f64 = 5.0;

/// 이 출처의 관측이 계정 경보 입력이 되는가 — 창 밖(표시용 · 같은 UID 위조 가능) 값만 아니다.
fn feeds_alerts(source: &str) -> bool {
    source != OUTSIDE_SOURCE
}

/// 경보 입력 갱신(창 밖이 아닌 출처만 · 경보 입력끼리 최신 승자). 호출자가 accounts 락을 잡고 있다.
fn note_alert_input(st: &mut AccountsState, key: &AccountKey, rate: &[RateWindow], source: &str, now: f64) {
    if !feeds_alerts(source) || rate.is_empty() {
        return;
    }
    let slot = st.alert_inputs.entry(key.clone()).or_insert((0.0, Vec::new()));
    if now >= slot.0 {
        *slot = (now, rate.to_vec());
    }
}

/// 창 밖 보고의 처리 결과(수용 또는 빈도 상한으로 버림). 거절은 `Err(사유 코드)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutsideOutcome {
    Accepted,
    Throttled,
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
fn profile_short(home: Option<&Path>, dir: &Path) -> String {
    if let Some(home) = home {
        if let Ok(rel) = dir.strip_prefix(home) {
            return rel.to_string_lossy().into_owned();
        }
    }
    dir.to_string_lossy().into_owned()
}

/// agy(Antigravity CLI) 데이터 폴더(홈 상대). agy 는 신원을 이 폴더의 토큰 파일 하나
/// (`antigravity-oauth-token`)로 든다 — 2026-09-23 실측: 라이브 agy 3개 모두 이 파일을 열고 있고,
/// agy 1.1.24 바이너리에 `antigravity-cli/settings.json`·`-oauth-token` 문자열이 있다. 종전 시드가 보던
/// `~/.antigravity` 는 이 맥에 없다(→ antigravity 계정이 영영 시드되지 않았다 · RCA RC1).
const AGY_DATA_DIR: &str = ".gemini/antigravity-cli";
/// 구 경로 — 종전 시드 기준. 호환으로 남긴다(있으면 같은 계정의 프로필로 함께 적는다).
const AGY_LEGACY_DIR: &str = ".antigravity";

/// 실제로 존재하는 agy 데이터 폴더(홈 상대 표기) — **존재만** 본다(토큰 내용은 읽지 않는다).
/// agy 는 데이터 폴더당 계정 1개다(토큰 파일 1개 · 계정 전환 없음 — RCA 1-3) → account_id 는 "default".
fn antigravity_profiles(home: &Path) -> Vec<String> {
    [AGY_DATA_DIR, AGY_LEGACY_DIR]
        .iter()
        .filter(|rel| home.join(rel).is_dir())
        .map(|rel| rel.to_string())
        .collect()
}

/// 프로필 dir → oauthAccount 신원. 신원 파일 위치는 `cys::profile_gate::identity_config_file` 정본을 따른다
/// (보통 `<dir>/.claude.json` · `CLAUDE_CONFIG_DIR` 없이 띄운 기본 프로필 `~/.claude` 만 홈 직하
/// `~/.claude.json`). 잡동사니 dir(.claude-worktrees·백업 등)은 파일 부재/uuid 부재로 None → 관측
/// 미귀속(유령 계정 0). 자격증명(.credentials.json)은 읽지 않는다.
/// (운영 경로는 홈을 명시하는 `claude_identity_at` 을 쓴다 — 이 얇은 판은 기존 검체용.)
#[cfg(test)]
fn claude_identity(
    state: &mut AccountsState,
    dir: &Path,
) -> Option<(String, String, Option<String>)> {
    claude_identity_at(state, dirs::home_dir().as_deref(), dir)
}

/// 홈을 인자로 받는 시험 이음매. `home == None`(홈 불명)이면 종전 규칙(폴더 안 파일만).
fn claude_identity_at(
    state: &mut AccountsState,
    home: Option<&Path>,
    dir: &Path,
) -> Option<(String, String, Option<String>)> {
    let f = match home {
        Some(h) => cys::profile_gate::identity_config_file(h, dir),
        None => dir.join(".claude.json"),
    };
    let mtime = std::fs::metadata(&f)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())?;
    // 캐시는 **읽은 파일** 기준 — 같은 dir 이라도 신원 파일이 바뀌면(명시 CLAUDE_CONFIG_DIR 로 폴더 안
    // 파일이 새로 생김) 다시 읽는다.
    if let Some(e) = state.ident_cache.get(dir) {
        if e.mtime == mtime && e.file == f {
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
            // ★RC5: 키별로 문자열 판독을 먼저 한다 — `userRateLimitTier` 가 **null 값으로 존재**하면
            //   `get` 이 Some(Null) 이라 종전 `or_else` 가 조직 등급으로 넘어가지 못했다(전 계정 plan 소실).
            let plan = ["userRateLimitTier", "organizationRateLimitTier"]
                .iter()
                .find_map(|k| oa.get(*k).and_then(|x| x.as_str()).filter(|t| !t.is_empty()))
                .map(|s| s.to_string());
            Some((uuid, email, plan))
        });
    state
        .ident_cache
        .insert(dir.to_path_buf(), IdentEntry { file: f, mtime, ident: ident.clone() });
    ident
}

/// agent + 세션 파일 → (키, 라벨, plan, 프로필 표기). claude는 신원 해석 실패 시 None(스킵).
/// (운영 경로는 홈을 명시하는 `resolve_at` 을 쓴다 — `note_rate_at` 경유. 이 얇은 판은 기존 검체용.)
#[cfg(test)]
fn resolve(
    state: &mut AccountsState,
    agent: &str,
    session_file: &str,
) -> Option<(AccountKey, String, Option<String>, Option<String>)> {
    resolve_at(state, dirs::home_dir().as_deref(), agent, session_file)
}

/// 홈을 인자로 받는 시험 이음매.
fn resolve_at(
    state: &mut AccountsState,
    home: Option<&Path>,
    agent: &str,
    session_file: &str,
) -> Option<(AccountKey, String, Option<String>, Option<String>)> {
    match agent {
        "claude" => {
            let dir = profile_dir_from_session(session_file)?;
            let (uuid, email, plan) = claude_identity_at(state, home, &dir)?;
            Some((
                AccountKey { provider: "claude".into(), account_id: uuid },
                email,
                plan,
                Some(profile_short(home, &dir)),
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
            // 실제로 있는 데이터 폴더를 적는다(없으면 표기 없음 — 지어내지 않는다)
            home.and_then(|h| antigravity_profiles(h).into_iter().next()),
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
    note_rate_at(daemon, dirs::home_dir().as_deref(), agent, session_file, rate, source, now);
}

/// 홈을 인자로 받는 시험 이음매(동작은 `note_rate` 와 같다). 반환: 계정에 **귀속됐는가**
/// (false = rate 가 비었거나 신원 불명 — 아무것도 쓰지 않았다).
fn note_rate_at(
    daemon: &Arc<Daemon>,
    home: Option<&Path>,
    agent: &str,
    session_file: &str,
    rate: &[RateWindow],
    source: &str,
    now: f64,
) -> bool {
    if rate.is_empty() {
        return false;
    }
    // 1) accounts 락 안에서 병합 + 영속 대상 수집 (analytics 락은 여기서 잡지 않는다 — 잠금 순서)
    let mut to_persist: Vec<(AccountKey, String, String, f64, Option<f64>)> = Vec::new();
    let mut do_prune = false;
    {
        let mut st = daemon.accounts.lock().unwrap();
        let Some((key, label, plan, profile)) = resolve_at(&mut st, home, agent, session_file) else {
            return false; // 미귀속(신원 불명) — 유령 계정을 만들지 않는다
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
            source_error: None,
        });
        view.label = label;
        if plan.is_some() {
            view.plan = plan;
        }
        if let Some(p) = profile {
            view.profiles.insert(p);
        }
        // 최신 승자 — note는 신선 생산분만 받으므로 timestamp 비교로 충분(표시용 · 출처 무관)
        if now >= view.updated_at {
            view.rate = rate.to_vec();
            view.updated_at = now;
            view.source = source.into();
        }
        // 신선 관측이 왔다 = 그 경로는 지금 동작한다 — 경로 고장 표기를 지운다.
        view.source_error = None;
        // 경보 입력은 따로 — 창 밖 값은 여기 들어오지 않고, 들어와 있던 좌석 값을 지우지도 않는다.
        note_alert_input(&mut st, &key, rate, source, now);
        // 스냅샷 스로틀은 두 기준 중 하나라도 1%p 이상 움직이면 기록한다: ① 출처 무관 마지막 기록(표시 복원이
        // 고르는 최신 행) ② 경보 입력 출처의 마지막 기록(경보 복원이 고르는 최신 행). ①만 보면 창 밖 값 바로 뒤에
        // 1%p 안으로 붙어 온 좌석 값이 버려져 재시작 뒤 경보 입력이 그보다 옛 좌석 값으로 복원된다.
        let alert_src = feeds_alerts(source);
        for w in rate {
            let pk = (key.clone(), w.label.clone());
            let moved = |prev: Option<f64>| prev.map_or(true, |p| (w.used_pct - p).abs() >= SNAPSHOT_MIN_DELTA_PCT);
            let due = moved(st.last_persisted.get(&pk).copied())
                || (alert_src && moved(st.last_persisted_alert.get(&pk).copied()));
            if due {
                st.last_persisted.insert(pk.clone(), w.used_pct);
                if alert_src {
                    st.last_persisted_alert.insert(pk, w.used_pct);
                }
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
        return true;
    }
    let guard = daemon.analytics.lock().unwrap();
    if let Some(conn) = guard.as_ref() {
        for (key, label, win, pct, resets) in &to_persist {
            crate::analytics::record_rate_snapshot(
                conn, now, &key.provider, &key.account_id, label, win, *pct, *resets, source,
            );
        }
        if do_prune {
            crate::analytics::prune_rate_snapshots(conn, now - SNAPSHOT_RETAIN_SECS);
        }
    }
    true
}

/// 부트 시드 — ① 알려진 프로필 dir 스캔으로 계정 **발견**(관측 전에도 3계정이 다 보이게),
/// ② analytics 마지막 스냅샷(7d)으로 rate 예열(source:"snapshot"·stale 표시),
/// ③ ~/.cys/accounts.json 선언 계정 등록(미래 provider — adapter:"none"은 '관측 없음' 상주).
pub fn seed_known(daemon: &Arc<Daemon>) {
    if let Some(home) = dirs::home_dir() {
        {
            let mut st = daemon.accounts.lock().unwrap();
            seed_discovered(&mut st, &home);
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
                        source_error: None,
                    });
                }
            }
        }
    }
    restore_from_snapshots(daemon, crate::state::now_epoch());
}

/// 부트 시드 ② — analytics 마지막 스냅샷으로 계정 뷰를 예열한다(홈 무접촉 · 시험 이음매).
/// 표시는 (계정,창)별 마지막 스냅샷(출처 무관), **경보 입력은 창 밖이 아닌 출처의 마지막 스냅샷만**으로 복원한다
/// (fix-values-1 SP-1 · 종전엔 재시작 한 번이 창 밖 값을 경보 입력으로 들였다). 출처 열이 생기기 전의 구 행은
/// 창 밖 값이 없던 시절의 것이라 경보 입력으로 친다(`analytics::last_rate_snapshots`).
fn restore_from_snapshots(daemon: &Arc<Daemon>, now: f64) {
    // 마지막 스냅샷으로 예열 — updated_at은 스냅샷 시각 그대로(신선한 척 금지)
    let (rows, alert_rows) = {
        let guard = daemon.analytics.lock().unwrap();
        match guard.as_ref() {
            Some(conn) => (
                Some(crate::analytics::last_rate_snapshots(conn, now - BOOT_RESTORE_SECS, None)),
                crate::analytics::last_rate_snapshots(conn, now - BOOT_RESTORE_SECS, Some(OUTSIDE_SOURCE)),
            ),
            None => (None, Vec::new()),
        }
    };
    // 경보 입력 복원 — (계정)별로 창을 모아 한 번에 싣는다. 이미 라이브 경보 입력이 있으면 덮지 않는다(신선 관측 우선).
    let mut restored: HashMap<AccountKey, (f64, Vec<RateWindow>)> = HashMap::new();
    for (ts, provider, account, _label, win, pct, resets) in alert_rows {
        let slot = restored
            .entry(AccountKey { provider, account_id: account })
            .or_insert((0.0, Vec::new()));
        slot.0 = slot.0.max(ts);
        slot.1.push(RateWindow { label: win, used_pct: pct, resets_at: resets });
    }
    if !restored.is_empty() {
        let mut st = daemon.accounts.lock().unwrap();
        for (key, (ts, mut rate)) in restored {
            rate.sort_by_key(|w| u8::from(w.label != "5h"));
            st.alert_inputs.entry(key).or_insert((ts, rate));
        }
    }
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
                source_error: None,
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

/// 부트 시드 ①(설치 흔적 스캔) — 홈을 인자로 받는 시험 이음매. 계정 **발견**만 한다(rate 없음).
fn seed_discovered(st: &mut AccountsState, home: &Path) {
    // ★(U-17) 프로필 dir 열거 규칙은 **lib 정본 하나**다(`cys::profile_gate`). 종전엔 이
    //   함수 안에만 있었고, 인증 판정기가 같은 규칙을 재구현하면 두 벌이 갈린다(한쪽만
    //   새 부서 접두를 배우는 식) — 같은 목록을 두 소비처가 보게 한다.
    //   ★판정은 바뀌지 않는다: 정본 함수는 종전 두 루프와 **같은 이름 규칙·같은 순서**이며
    //   `is_dir()` 검사도 더하지 않는다(동작 동일성 유지 — 완화도 강화도 아니다).
    let dirs_to_check: Vec<PathBuf> = cys::profile_gate::enumerate_profile_dirs(home);
    for dir in dirs_to_check {
        if let Some((uuid, email, plan)) = claude_identity_at(st, Some(home), &dir) {
            let key = AccountKey { provider: "claude".into(), account_id: uuid };
            let short = profile_short(Some(home), &dir);
            let v = st.views.entry(key.clone()).or_insert_with(|| AccountView {
                key,
                label: email.clone(),
                plan: plan.clone(),
                profiles: BTreeSet::new(),
                rate: Vec::new(),
                updated_at: 0.0,
                source: String::new(),
                adapter: true,
                source_error: None,
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
                source_error: None,
            });
    }
    // ★RC1: agy 데이터 폴더(`~/.gemini/antigravity-cli`) — 종전엔 `~/.antigravity` 만 봐서 이 맥의
    //   antigravity 계정이 영영 시드되지 않았다. 구 경로는 호환으로 함께 본다. **존재만** 본다.
    let agy = antigravity_profiles(home);
    if !agy.is_empty() {
        let key = AccountKey { provider: "antigravity".into(), account_id: "default".into() };
        let v = st.views.entry(key.clone()).or_insert_with(|| AccountView {
            key,
            label: "Antigravity (agy)".into(),
            plan: None,
            profiles: BTreeSet::new(),
            rate: Vec::new(),
            updated_at: 0.0,
            source: String::new(),
            adapter: true,
            source_error: None,
        });
        v.profiles.extend(agy);
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
                // ★U5(0.14.41): 콘솔 없는 cysd(GUI 서브시스템)가 콘솔 자식(cmd.exe)을 창 정책 없이
                //   띄우면 **주기마다 새 콘솔 창이 번쩍인다**(interval_secs 하한 60초 · 이 루프는
                //   홈 공용 accounts.json 을 읽는 모든 cysd = 본부 + 부서 데몬마다 돈다). hide_console =
                //   등급 Attached(CREATE_NO_WINDOW 단독 · unix 무동작) — 출력은 `.output()` 파이프로 받으므로
                //   흐름 무변경. 두 분기 모두 건다(census `consoleless_spawns_carry_window_policy`).
                let fut = if cfg!(windows) {
                    tokio::process::Command::new("cmd").args(["/C", &cmd]).hide_console().output()
                } else {
                    tokio::process::Command::new("sh").args(["-c", &cmd]).hide_console().output()
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
        key: key.clone(),
        label,
        plan: None,
        profiles: BTreeSet::new(),
        rate: Vec::new(),
        updated_at: 0.0,
        source: String::new(),
        adapter: true,
        source_error: None,
    });
    if now >= v.updated_at {
        v.rate = rate.to_vec();
        v.updated_at = now;
        v.source = source.into();
        v.adapter = true;
    }
    note_alert_input(&mut st, &key, rate, source, now);
}

/// agy 관측 경로(agy-rpc)의 고장 코드를 antigravity 계정 행에 싣는다(`None` = 지움).
/// 수집기가 한 틱을 통째로 본 뒤 부른다 — 그 틱에 한 좌석이라도 성공했으면 부르지 않는다(성공은
/// note_rate 가 지운다). agy 좌석이 하나도 없으면 `None` 으로 옛 오류를 지운다(좌석이 없는 것은 고장이 아니다).
///
/// ★행을 만드는 근거는 **시드와 같은 것**(agy 데이터 폴더 실재)뿐이다(fix-round-1 F1). 좌석의 `agent_meta ==
/// "gemini"` 는 계정의 흔적이 아니다 — cys 런처는 readiness 전에 meta 를 달므로 agy 미설치·agy 가 끝나 셸만 남은
/// 좌석·agents.json 에서 gemini 를 다른 CLI 로 바꾼 좌석도 gemini 좌석이고, 그 좌석의 `agy_no_process` 로 행을
/// 만들면 좌석이 닫힌 뒤 '관측 전' 유령 계정이 데몬 재시작까지 남는다. 그래서:
///   · 행이 있으면(부트 시드 · 신선 관측 · 선언) 오류만 싣는다 — 폴더 유무와 무관.
///   · 행이 없으면 데이터 폴더가 **지금** 있을 때만 만든다(부트 뒤 설치된 agy = 늦은 시드 · 재시작해도 같은 행).
///   · 둘 다 아니면 버린다 — 흔적 없는 기계의 좌석 오류는 보일 계정이 없다.
/// 이 규칙이면 오류 경로로 생긴 행은 전부 시드 근거를 가진 행이라, 좌석 0 에서 행을 지울 필요가 없다.
pub fn note_agy_error(daemon: &Arc<Daemon>, err: Option<&str>) {
    note_agy_error_at(daemon, dirs::home_dir().as_deref(), err)
}

/// 홈을 인자로 받는 시험 이음매. `home == None`(홈 불명) = 근거 확인 불가 → 새 행을 만들지 않는다.
fn note_agy_error_at(daemon: &Arc<Daemon>, home: Option<&Path>, err: Option<&str>) {
    let mut st = daemon.accounts.lock().unwrap();
    let key = AccountKey { provider: "antigravity".into(), account_id: "default".into() };
    match err {
        Some(code) => {
            if let Some(v) = st.views.get_mut(&key) {
                v.source_error = Some(code.to_string());
                return;
            }
            let profiles: BTreeSet<String> = home
                .map(|h| antigravity_profiles(h).into_iter().collect())
                .unwrap_or_default();
            if profiles.is_empty() {
                return; // 흔적 0 — 유령 계정을 만들지 않는다
            }
            st.views.insert(
                key.clone(),
                AccountView {
                    key,
                    label: "Antigravity (agy)".into(),
                    plan: None,
                    profiles,
                    rate: Vec::new(),
                    updated_at: 0.0,
                    source: String::new(),
                    adapter: true,
                    source_error: Some(code.to_string()),
                },
            );
        }
        None => {
            if let Some(v) = st.views.get_mut(&key) {
                v.source_error = None;
            }
        }
    }
}

/// agy 상태줄 훅이 이 데몬에 값을 보낸 적이 있고 그것이 antigravity 계정의 최신 출처인가 — 참이면 RPC
/// 수집기는 프로브를 멈춘다(CSRF 로 막힌 경로를 계속 두드려 값 있는 행에 '관측 실패'를 덧씌우지 않게).
pub fn agy_statusline_authoritative(daemon: &Arc<Daemon>) -> bool {
    let st = daemon.accounts.lock().unwrap();
    let key = AccountKey { provider: "antigravity".into(), account_id: "default".into() };
    st.views.get(&key).is_some_and(|v| v.source == AGY_STATUSLINE_SOURCE)
}

/// cys 창 밖 보고의 **모양** 검증(순수 — 파일시스템 무접촉). 통과하면 걸러진 rate 를 돌려준다.
/// `raw_len` = 요청의 rate 배열 원소 수(파싱 전) — 크기 상한은 파싱 전에 건다.
pub fn outside_shape(
    session_file: &str,
    rate: Vec<RateWindow>,
    raw_len: usize,
    now: f64,
) -> Result<Vec<RateWindow>, &'static str> {
    // 경로: usage.register 와 같은 규칙(절대 · `..` 없음 · .jsonl) + 길이 상한.
    let p = Path::new(session_file);
    if session_file.is_empty()
        || session_file.len() > OUTSIDE_SESSION_FILE_MAX
        || !p.is_absolute()
        || p.components().any(|c| matches!(c, std::path::Component::ParentDir))
        || p.extension().and_then(|e| e.to_str()) != Some("jsonl")
    {
        return Err("session_file_invalid");
    }
    if raw_len > OUTSIDE_RATE_MAX_ENTRIES {
        return Err("rate_invalid");
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for w in rate {
        if !seen.insert(w.label.clone()) {
            return Err("rate_invalid"); // 같은 창 두 번 = 기형 — 어느 쪽이 참인지 고르지 않는다
        }
        // 창별로 거른다(불량 창만 버림): 5h·7d 만 · 유한 0..=1000(100 초과는 UI 가 '100%+' 로 정직 표기) ·
        // 리셋 시각은 있으면 [now-1일, now+8일] 안.
        let label_ok = w.label == "5h" || w.label == "7d";
        let pct_ok = w.used_pct.is_finite() && (0.0..=1000.0).contains(&w.used_pct);
        let reset_ok = w
            .resets_at
            .is_none_or(|r| r.is_finite() && r >= now - 86400.0 && r <= now + 8.0 * 86400.0);
        if label_ok && pct_ok && reset_ok {
            out.push(w);
        }
    }
    if out.is_empty() {
        Err("rate_invalid")
    } else {
        Ok(out)
    }
}

/// cys 창 밖 Claude 세션의 계정 전용 보고 — 호출자 검사(pane 아님)는 **부른 쪽**(handlers)이 먼저 끝낸다.
pub fn report_outside(
    daemon: &Arc<Daemon>,
    session_file: &str,
    rate: &[RateWindow],
    now: f64,
) -> Result<OutsideOutcome, &'static str> {
    report_outside_at(daemon, dirs::home_dir().as_deref(), session_file, rate, now)
}

/// 홈을 인자로 받는 시험 이음매.
///
/// 검증(전부 거절 = 귀속 0 · fail-closed):
///   ① transcript 가 **실재하는 파일**이다(정규화 = 심볼릭 링크를 풀어 실제 위치로 판정).
///   ② 그 실제 위치가 **알려진 프로필 dir**(`profile_gate::enumerate_profile_dirs` — 계정 발견과 같은 목록)의
///      `<dir>/projects/` 아래다. 명명 규칙 밖 `CLAUDE_CONFIG_DIR` 세션은 귀속되지 않는다(한계 · 문서화).
///   ③ 귀속 표기 경로(열거된 dir 기준)가 `profile_dir_from_session` 으로 **같은 dir** 로 되돌아간다
///      (홈 경로 자체에 `/projects/` 가 든 기계에서 다른 dir 로 오귀속되는 것을 막는다).
///   ④ 빈도 상한(프로필 dir 단위): [`OUTSIDE_MIN_INTERVAL_SECS`] 하한 · 같은 값은 [`OUTSIDE_SAME_VALUE_SECS`].
///   ⑤ 그 프로필의 신원(`.claude.json` oauthAccount)이 읽힌다 — 아니면 `identity_unresolved`.
/// 쓰는 것은 `note_rate`(계정 뷰 + rate 스냅샷) 하나뿐이다 — 좌석·배지·이벤트·임계·비용 무접촉.
fn report_outside_at(
    daemon: &Arc<Daemon>,
    home: Option<&Path>,
    session_file: &str,
    rate: &[RateWindow],
    now: f64,
) -> Result<OutsideOutcome, &'static str> {
    let home = home.ok_or("home_unknown")?;
    // ① 실재 — 먼저 한다(없는 경로면 홈 폴더 열거조차 하지 않는다).
    let canon = std::fs::canonicalize(session_file).map_err(|_| "session_file_missing")?;
    if !canon.is_file() || canon.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return Err("session_file_missing");
    }
    // ② 알려진 프로필 dir 의 projects/ 아래(정규화끼리 비교 — 홈이 심볼릭 링크를 지나도 같게 판정)
    let (dir, canon_dir) = cys::profile_gate::enumerate_profile_dirs(home)
        .into_iter()
        .find_map(|d| {
            let cd = std::fs::canonicalize(&d).ok()?;
            let projects = cd.join("projects");
            (canon.starts_with(&projects) && canon != projects).then_some((d, cd))
        })
        .ok_or("session_file_outside_profiles")?;
    // ③ 귀속은 열거된(홈 기준) dir 로 표기한다 — 신원 규칙(기본 ~/.claude → 홈 직하)과 프로필 표기가 홈 기준이다.
    let rel = canon.strip_prefix(&canon_dir).map_err(|_| "session_file_outside_profiles")?;
    let attributed = dir.join(rel);
    let attributed = attributed.to_string_lossy().into_owned();
    if profile_dir_from_session(&attributed).as_deref() != Some(dir.as_path()) {
        return Err("session_file_ambiguous");
    }
    // ④ 빈도 상한 — 확인과 기록을 한 임계영역에서(동시 보고 둘이 함께 통과하지 않게).
    {
        let mut st = daemon.accounts.lock().unwrap();
        if let Some((t, last)) = st.outside_last.get(&canon_dir) {
            let dt = now - *t;
            if dt < OUTSIDE_MIN_INTERVAL_SECS || (dt < OUTSIDE_SAME_VALUE_SECS && last.as_slice() == rate) {
                return Ok(OutsideOutcome::Throttled);
            }
        }
        st.outside_last.insert(canon_dir, (now, rate.to_vec()));
    }
    // ⑤ 귀속 — note_rate 하나뿐
    if note_rate_at(daemon, Some(home), "claude", &attributed, rate, OUTSIDE_SOURCE, now) {
        Ok(OutsideOutcome::Accepted)
    } else {
        Err("identity_unresolved")
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
                    // null = 경로 고장 없음(관측 전이거나 정상). 값 = 그 경로가 지금 고장(예: agy_http_403).
                    "source_error": v.source_error,
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

/// alerts용 스냅샷: (라벨, 창, pct) — 경보 입력([`AccountsState::alert_inputs`])이 있는 계정만.
/// 창 밖(표시용) 값은 경보 근거가 아니다 — 같은 UID 의 아무 프로세스나 보낼 수 있다(위조 한계). 그래서 경보는
/// 같은 계정의 **창 밖이 아닌 관측 중 가장 최근 값**으로 판정한다 — 창 밖 값이 더 최신이어도 그 값이 남는다
/// (표시 숫자와 다를 수 있다). 창 밖 값만 있는 계정은 경보 입력이 없다.
pub fn alert_rates(daemon: &Arc<Daemon>) -> Vec<(String, String, f64)> {
    let st = daemon.accounts.lock().unwrap();
    let mut out = Vec::new();
    for (key, (t, rate)) in &st.alert_inputs {
        if *t == 0.0 {
            continue;
        }
        let Some(v) = st.views.get(key) else {
            continue; // 뷰 없는 경보 입력은 만들어지지 않는다(note 가 뷰를 먼저 만든다) — 방어
        };
        for w in rate {
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

    // ───────── 0.14.42 계정 누락 수리 — 재현 검체(수정 전 적색) ─────────

    fn write(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    const ID_NULL_TIER: &str = r#"{"oauthAccount":{"accountUuid":"u-home","emailAddress":"h@x.y","userRateLimitTier":null,"organizationRateLimitTier":"default_claude_max_20x"}}"#;

    /// RC5: `userRateLimitTier` 가 **null 값으로 존재**하면 조직 등급으로 넘어가야 한다(결측형 음성 대조).
    #[test]
    fn plan_falls_back_to_org_tier_when_user_tier_is_null() {
        let dir = tmp("nulltier");
        write(&dir.join(".claude.json"), ID_NULL_TIER);
        let mut st = AccountsState::default();
        let got = claude_identity(&mut st, &dir).unwrap();
        assert_eq!(got.2.as_deref(), Some("default_claude_max_20x"), "null 사용자 등급이 조직 등급을 가렸다");
        // 키 자체가 없을 때도 같다(부재형) · 사용자 등급이 값이면 그것이 이긴다(값형)
        let d2 = tmp("notier");
        write(&d2.join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"u2","organizationRateLimitTier":"org"}}"#);
        assert_eq!(claude_identity(&mut st, &d2).unwrap().2.as_deref(), Some("org"));
        let d3 = tmp("usertier");
        write(&d3.join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"u3","userRateLimitTier":"max_5x","organizationRateLimitTier":"org"}}"#);
        assert_eq!(claude_identity(&mut st, &d3).unwrap().2.as_deref(), Some("max_5x"));
        // 둘 다 null → None(없는 값을 지어내지 않는다)
        let d4 = tmp("bothnull");
        write(&d4.join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"u4","userRateLimitTier":null,"organizationRateLimitTier":null}}"#);
        assert_eq!(claude_identity(&mut st, &d4).unwrap().2, None);
    }

    /// RC3: `CLAUDE_CONFIG_DIR` 없이 띄운 기본 프로필 `~/.claude` 의 신원은 **홈 직하 `~/.claude.json`** 이다
    /// (Claude Code: `join(CLAUDE_CONFIG_DIR || homedir(), ".claude.json")`). 기본 프로필만 그렇다.
    #[test]
    fn default_profile_identity_reads_home_level_claude_json() {
        let home = tmp("home-default");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        write(&home.join(".claude.json"), ID_NULL_TIER);
        let mut st = AccountsState::default();
        let got = claude_identity_at(&mut st, Some(&home), &home.join(".claude"));
        assert_eq!(got.as_ref().map(|g| g.0.as_str()), Some("u-home"), "기본 프로필 신원을 못 읽었다");
        // 기본 프로필이 아닌 폴더는 홈 직하로 넘어가지 않는다(다른 계정을 주워 오지 않는다)
        std::fs::create_dir_all(home.join(".claude-9")).unwrap();
        assert!(claude_identity_at(&mut st, Some(&home), &home.join(".claude-9")).is_none());
        let other = tmp("elsewhere");
        assert!(claude_identity_at(&mut st, Some(&home), &other.join(".claude")).is_none());
        // 명시 CLAUDE_CONFIG_DIR=~/.claude 로 생긴 `<dir>/.claude.json` 이 있으면 그것이 이긴다
        // (캐시는 **읽은 파일** 기준 — 같은 dir 이라도 파일이 바뀌면 다시 읽는다).
        write(&home.join(".claude/.claude.json"), r#"{"oauthAccount":{"accountUuid":"u-explicit"}}"#);
        assert_eq!(claude_identity_at(&mut st, Some(&home), &home.join(".claude")).unwrap().0, "u-explicit");
        // 홈을 모르면 종전 규칙(폴더 안 파일만)
        let lone = tmp("lone");
        std::fs::create_dir_all(lone.join(".claude")).unwrap();
        write(&lone.join(".claude.json"), ID_NULL_TIER);
        assert!(claude_identity_at(&mut st, None, &lone.join(".claude")).is_none());
    }

    /// RC3(관측): 기본 프로필 세션(`~/.claude/projects/…`)의 statusline 보고가 계정에 귀속된다.
    #[test]
    fn default_profile_session_is_attributed() {
        let home = tmp("home-attr");
        std::fs::create_dir_all(home.join(".claude/projects/-w")).unwrap();
        write(&home.join(".claude.json"), ID_NULL_TIER);
        let mut st = AccountsState::default();
        let sess = home.join(".claude/projects/-w/s.jsonl");
        let got = resolve_at(&mut st, Some(&home), "claude", &sess.to_string_lossy());
        let (k, _, plan, prof) = got.expect("기본 프로필 세션이 어느 계정에도 귀속되지 않았다");
        assert_eq!((k.provider.as_str(), k.account_id.as_str()), ("claude", "u-home"));
        assert_eq!(plan.as_deref(), Some("default_claude_max_20x"));
        assert_eq!(prof.as_deref(), Some(".claude"));
    }

    /// RC1+RC3(발견): 가짜 홈의 설치 흔적만으로 **모든** 계정이 시드된다 —
    /// 기본 프로필(`~/.claude.json`)·Antigravity(`~/.gemini/antigravity-cli`)·구 경로(`~/.antigravity`) 호환.
    #[test]
    fn seed_discovers_default_profile_and_antigravity_data_dir() {
        let home = tmp("home-seed");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        write(&home.join(".claude.json"), ID_NULL_TIER);
        write(&home.join(".claude-3/.claude.json"), r#"{"oauthAccount":{"accountUuid":"u-3","emailAddress":"c@x.y"}}"#);
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        // agy 는 이 파일 하나로 신원을 든다 — **존재만** 본다(내용은 읽지 않는다: 더미 문자열)
        write(&home.join(".gemini/antigravity-cli/antigravity-oauth-token"), "dummy-not-a-token");
        let mut st = AccountsState::default();
        seed_discovered(&mut st, &home);
        let key = |p: &str, a: &str| AccountKey { provider: p.into(), account_id: a.into() };
        let def = st.views.get(&key("claude", "u-home")).expect("기본 프로필 계정이 시드되지 않았다(RC3)");
        assert!(def.profiles.contains(".claude"), "profiles={:?}", def.profiles);
        assert!(st.views.contains_key(&key("claude", "u-3")));
        assert!(st.views.contains_key(&key("codex", "default")));
        let agy = st.views.get(&key("antigravity", "default")).expect("antigravity 가 시드되지 않았다(RC1)");
        assert!(agy.profiles.contains(".gemini/antigravity-cli"), "profiles={:?}", agy.profiles);
        assert_eq!(agy.updated_at, 0.0, "발견만 — 관측 전");
        // 구 경로 호환: `~/.antigravity` 만 있어도 시드된다
        let old = tmp("home-legacy");
        std::fs::create_dir_all(old.join(".antigravity")).unwrap();
        let mut st2 = AccountsState::default();
        seed_discovered(&mut st2, &old);
        assert!(st2.views[&key("antigravity", "default")].profiles.contains(".antigravity"));
        // 음성 대조(부재형): 흔적이 하나도 없으면 antigravity 도 없다(유령 계정 0)
        let bare = tmp("home-bare");
        let mut st3 = AccountsState::default();
        seed_discovered(&mut st3, &bare);
        assert!(!st3.views.contains_key(&key("antigravity", "default")));
        assert!(st3.views.is_empty(), "빈 홈에서 계정이 생겼다: {:?}", st3.views.keys().collect::<Vec<_>>());
    }

    /// RC1(관측 표기): agy 관측의 프로필 표기는 실제 데이터 폴더다.
    #[test]
    fn antigravity_observation_labels_the_real_data_dir() {
        let home = tmp("home-agy-obs");
        std::fs::create_dir_all(home.join(".gemini/antigravity-cli")).unwrap();
        let mut st = AccountsState::default();
        let (k, _, _, prof) = resolve_at(&mut st, Some(&home), "gemini", "").unwrap();
        assert_eq!(k.provider, "antigravity");
        assert_eq!(prof.as_deref(), Some(".gemini/antigravity-cli"));
    }

    /// RC2(정직 표기): 관측 경로 고장은 '관측 전'과 구별돼 행에 실리고, 신선 관측이 오면 지워진다.
    /// (가짜 홈에 agy 데이터 폴더를 둔다 — 행 생성 근거. 라이브 홈에 기대면 폴더 없는 CI 에서 결과가 갈린다.)
    #[test]
    fn source_error_is_exposed_until_a_fresh_observation_clears_it() {
        let dir = tmp("daemon-srcerr");
        let home = tmp("home-srcerr");
        std::fs::create_dir_all(home.join(".gemini/antigravity-cli")).unwrap();
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        note_agy_error_at(&d, Some(&home), Some("agy_http_403"));
        let now = crate::state::now_epoch();
        let rows = local_json(&d, now);
        let row = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["provider"] == "antigravity")
            .expect("관측 경로 오류가 난 antigravity 계정이 행으로 나오지 않는다");
        assert_eq!(row["source_error"], "agy_http_403");
        assert!(row["updated_at"].is_null());
        note_rate(&d, "gemini", "", &[RateWindow { label: "5h".into(), used_pct: 10.0, resets_at: None }], "agy-rpc", now);
        let rows = local_json(&d, now);
        let row = rows.as_array().unwrap().iter().find(|r| r["provider"] == "antigravity").unwrap();
        assert!(row["source_error"].is_null(), "신선 관측 뒤에도 오류가 남았다: {row}");
        assert_eq!(row["source"], "agy-rpc");
        // 오류 해제(None) — 좌석이 사라지면 옛 오류를 남기지 않는다
        note_agy_error_at(&d, Some(&home), Some("agy_unreachable"));
        note_agy_error_at(&d, Some(&home), None);
        let rows = local_json(&d, now);
        let row = rows.as_array().unwrap().iter().find(|r| r["provider"] == "antigravity").unwrap();
        assert!(row["source_error"].is_null());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    fn agy_rows(d: &Arc<Daemon>) -> Vec<Value> {
        let now = crate::state::now_epoch();
        local_json(d, now)
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["provider"] == "antigravity")
            .cloned()
            .collect()
    }

    /// fix-round-1 F1(음성 대조): agy 흔적이 전혀 없는 홈 + agy 없는 gemini 좌석(agy 미설치·agy 가 끝나 셸만 남음·
    /// agents.json 의 gemini 를 다른 CLI 로 바꿈) → 수집기 오류가 **유령 Antigravity 계정 행을 만들지 않는다**.
    /// 수정 전: `note_agy_error(Some)` 가 행을 만들고, 좌석이 닫히면 오류만 지워 '관측 전' 유령이 재시작까지 남았다.
    #[test]
    fn agy_error_without_any_agy_trace_creates_no_account_row() {
        let dir = tmp("daemon-noghost");
        let home = tmp("home-noghost"); // 빈 홈 — ~/.gemini/antigravity-cli · ~/.antigravity 없음
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        for code in ["agy_no_process", "agy_no_port", "agy_unreachable", "agy_http_401", "agy_no_quota"] {
            note_agy_error_at(&d, Some(&home), Some(code));
            let rows = agy_rows(&d);
            assert!(rows.is_empty(), "흔적 없는 홈에서 오류 {code} 가 antigravity 행을 만들었다: {rows:?}");
        }
        // 좌석이 0이 된 뒤(None)에도 행이 없다 — '관측 전' 유령으로 남지 않는다
        note_agy_error_at(&d, Some(&home), None);
        assert!(agy_rows(&d).is_empty());
        // 홈을 모를 때도 만들지 않는다(근거 없음 = 행 없음)
        note_agy_error_at(&d, None, Some("agy_no_process"));
        assert!(agy_rows(&d).is_empty());
        assert!(local_json(&d, crate::state::now_epoch()).as_array().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// fix-round-1 F1(양성 대조): 행 생성의 근거는 **시드와 같은 것**(agy 데이터 폴더 실재)이다 — 부트 뒤에 설치된
    /// agy(늦은 시드)는 오류 틱에 행이 생기고 실제 폴더가 적힌다. 좌석이 0 이 되면 오류만 지우고 행은 남는다
    /// (폴더가 있으니 데몬을 재시작해도 시드되는 행 — 재시작 전후가 같다).
    #[test]
    fn agy_error_creates_row_only_on_the_seed_evidence() {
        let dir = tmp("daemon-lateseed");
        let home = tmp("home-lateseed");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        note_agy_error_at(&d, Some(&home), Some("agy_no_process"));
        assert!(agy_rows(&d).is_empty(), "폴더가 생기기 전");
        write(&home.join(".gemini/antigravity-cli/antigravity-oauth-token"), "dummy-not-a-token");
        note_agy_error_at(&d, Some(&home), Some("agy_no_process"));
        let rows = agy_rows(&d);
        assert_eq!(rows.len(), 1, "데이터 폴더가 생긴 뒤에는 행이 있어야 한다: {rows:?}");
        assert_eq!(rows[0]["source_error"], "agy_no_process");
        assert_eq!(rows[0]["profiles"], json!([".gemini/antigravity-cli"]));
        assert!(rows[0]["updated_at"].is_null(), "값은 지어내지 않는다");
        note_agy_error_at(&d, Some(&home), None);
        let rows = agy_rows(&d);
        assert_eq!(rows.len(), 1);
        assert!(rows[0]["source_error"].is_null());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// fix-round-1 F1(주석만): 이미 있는 행(신선 관측으로 생긴 행 포함)에는 폴더가 없어도 오류가 실린다 —
    /// 행을 만드는 것만 근거를 요구하고, 있는 행의 경로 고장 표기는 막지 않는다.
    #[test]
    fn agy_error_annotates_an_existing_row_without_the_data_dir() {
        let dir = tmp("daemon-annotate");
        let home = tmp("home-annotate");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let now = crate::state::now_epoch();
        note_rate(&d, "gemini", "", &[RateWindow { label: "5h".into(), used_pct: 7.0, resets_at: None }], "agy-rpc", now);
        note_agy_error_at(&d, Some(&home), Some("agy_http_403"));
        let rows = agy_rows(&d);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["source_error"], "agy_http_403");
        assert_eq!(rows[0]["source"], "agy-rpc", "관측값·출처는 그대로");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    // ───────── 0.14.42 RC4-b — cys 창 밖 Claude 세션의 계정 전용 보고(수정 전 적색) ─────────
    // 픽스처는 전부 합성값이다(u-out·o@example.test 등) — 실계정 식별자 금지.

    fn rw(label: &str, pct: f64, resets: Option<f64>) -> RateWindow {
        RateWindow { label: label.into(), used_pct: pct, resets_at: resets }
    }

    fn claude_rows(d: &Arc<Daemon>) -> Vec<Value> {
        local_json(d, crate::state::now_epoch())
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["provider"] == "claude")
            .cloned()
            .collect()
    }

    /// 모양 검증(파일시스템 무접촉): 경로는 절대·`..` 없음·`.jsonl`·길이 상한 · rate 는 5h/7d 만·중복 없음·
    /// 유한 0..=1000(100 초과는 자르지 않는다 — UI 가 100%+ 로 표기)·리셋 시각은 [now-1일, now+8일] · 원소 수 상한.
    #[test]
    fn outside_shape_rejects_malformed_reports() {
        let now = 1_800_000_000.0;
        let ok_rate = || vec![rw("5h", 33.0, Some(now + 3600.0)), rw("7d", 44.0, Some(now + 86400.0))];
        let good_path = std::env::temp_dir().join("x/.claude-3/projects/-w/s.jsonl"); // 플랫폼 절대경로
        let good = good_path.to_str().unwrap();
        assert_eq!(outside_shape(good, ok_rate(), 2, now).unwrap().len(), 2);
        for bad in [
            "",
            "relative/.claude-3/projects/-w/s.jsonl",
            "/Users/x/.claude-3/projects/-w/../../../etc/s.jsonl",
            "/Users/x/.claude-3/projects/-w/s.txt",
            "/Users/x/.claude-3/projects/-w/s.jsonl.bak",
        ] {
            assert_eq!(outside_shape(bad, ok_rate(), 2, now), Err("session_file_invalid"), "{bad:?}");
        }
        let long = good_path.with_file_name(format!("{}.jsonl", "a".repeat(1100)));
        assert_eq!(outside_shape(long.to_str().unwrap(), ok_rate(), 2, now), Err("session_file_invalid"), "길이 상한");
        // 크기 상한: 파싱 전 원소 수
        assert_eq!(outside_shape(good, ok_rate(), OUTSIDE_RATE_MAX_ENTRIES + 1, now), Err("rate_invalid"));
        // 중복 라벨 = 기형(통째 거절)
        assert_eq!(outside_shape(good, vec![rw("5h", 1.0, None), rw("5h", 2.0, None)], 2, now), Err("rate_invalid"));
        // 모르는 창·비유한·음수·과대·리셋 범위 밖 = 그 창만 버림 → 남는 게 없으면 거절(결측형 음성 대조)
        for w in [
            rw("1m", 10.0, None),
            rw("5h", f64::NAN, None),
            rw("5h", f64::INFINITY, None),
            rw("5h", -1.0, None),
            rw("5h", 1000.5, None),
            rw("5h", 10.0, Some(now - 2.0 * 86400.0)),
            rw("5h", 10.0, Some(now + 9.0 * 86400.0)),
            rw("5h", 10.0, Some(f64::NAN)),
        ] {
            assert_eq!(outside_shape(good, vec![w.clone()], 1, now), Err("rate_invalid"), "{w:?}");
        }
        assert_eq!(outside_shape(good, vec![], 0, now), Err("rate_invalid"), "빈 rate");
        let kept = outside_shape(good, vec![rw("5h", 120.0, None), rw("1m", 3.0, None)], 2, now).unwrap();
        assert_eq!(kept, vec![rw("5h", 120.0, None)], "100 초과는 자르지 않고 모르는 창만 버린다");
    }

    /// 실재하는 transcript + 알려진 프로필 dir → 그 프로필 신원의 계정에 귀속(source statusline-outside).
    /// 좌석·배지·이벤트는 건드리지 않는다(버스 seq 불변).
    #[test]
    fn outside_report_attributes_a_real_transcript_in_a_known_profile() {
        let dir = tmp("daemon-out-ok");
        let home = tmp("home-out-ok");
        write(&home.join(".claude-3/.claude.json"), r#"{"oauthAccount":{"accountUuid":"u-out","emailAddress":"o@example.test"}}"#);
        write(&home.join(".claude-3/projects/-w/s.jsonl"), "{}\n");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let seq0 = d.bus.latest_seq();
        let now = crate::state::now_epoch();
        let sess = home.join(".claude-3/projects/-w/s.jsonl");
        let got = report_outside_at(&d, Some(&home), &sess.to_string_lossy(), &[rw("5h", 33.0, None), rw("7d", 44.0, None)], now);
        assert_eq!(got, Ok(OutsideOutcome::Accepted), "창 밖 보고가 귀속되지 않았다");
        let rows = claude_rows(&d);
        let row = rows.iter().find(|r| r["account_id"] == "u-out").expect("계정 행 없음");
        assert_eq!(row["source"], OUTSIDE_SOURCE);
        assert_eq!(row["rate"][0]["used_pct"], json!(33.0));
        assert_eq!(row["profiles"], json!([".claude-3"]), "프로필 표기는 홈 상대(정규화 경로 아님)");
        assert_eq!(d.bus.latest_seq(), seq0, "창 밖 보고가 이벤트를 발행했다");
        assert!(d.surfaces.lock().unwrap().is_empty());
        // 기본 프로필(~/.claude — 신원은 홈 직하 ~/.claude.json · RC3 규칙)도 같은 입구로 귀속된다
        write(&home.join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"u-def","emailAddress":"d@example.test"}}"#);
        write(&home.join(".claude/projects/-w/t.jsonl"), "{}\n");
        let t = home.join(".claude/projects/-w/t.jsonl");
        assert_eq!(report_outside_at(&d, Some(&home), &t.to_string_lossy(), &[rw("5h", 5.0, None)], now), Ok(OutsideOutcome::Accepted));
        assert!(claude_rows(&d).iter().any(|r| r["account_id"] == "u-def" && r["profiles"] == json!([".claude"])));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 경로가 아무것도 증명하지 못하면 귀속하지 않는다(fail-closed · 유령 계정 0): 파일 부재 · 알려진 프로필 밖 ·
    /// 프로필 안이지만 projects/ 밖 · 밖을 가리키는 심볼릭 링크 · 신원 없는 프로필 · 홈 불명.
    #[test]
    fn outside_report_rejects_paths_that_prove_nothing() {
        let dir = tmp("daemon-out-bad");
        let home = tmp("home-out-bad");
        write(&home.join(".claude-3/.claude.json"), r#"{"oauthAccount":{"accountUuid":"u-3"}}"#);
        write(&home.join("elsewhere/.claude.json"), r#"{"oauthAccount":{"accountUuid":"u-else"}}"#);
        write(&home.join("elsewhere/projects/-w/s.jsonl"), "{}\n");
        write(&home.join(".claude-3/stray.jsonl"), "{}\n");
        std::fs::create_dir_all(home.join(".claude-5/projects/-w")).unwrap();
        write(&home.join(".claude-5/projects/-w/s.jsonl"), "{}\n"); // 신원(.claude.json) 없는 프로필
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let now = crate::state::now_epoch();
        let r = [rw("5h", 50.0, None)];
        let p = |rel: &str| home.join(rel).to_string_lossy().into_owned();
        assert_eq!(report_outside_at(&d, Some(&home), &p(".claude-3/projects/-w/nope.jsonl"), &r, now), Err("session_file_missing"));
        assert_eq!(report_outside_at(&d, Some(&home), &p("elsewhere/projects/-w/s.jsonl"), &r, now), Err("session_file_outside_profiles"));
        assert_eq!(report_outside_at(&d, Some(&home), &p(".claude-3/stray.jsonl"), &r, now), Err("session_file_outside_profiles"));
        #[cfg(unix)]
        {
            std::fs::create_dir_all(home.join(".claude-3/projects/-l")).unwrap();
            std::os::unix::fs::symlink(home.join("elsewhere/projects/-w/s.jsonl"), home.join(".claude-3/projects/-l/s.jsonl")).unwrap();
            assert_eq!(
                report_outside_at(&d, Some(&home), &p(".claude-3/projects/-l/s.jsonl"), &r, now),
                Err("session_file_outside_profiles"),
                "밖을 가리키는 링크는 정규화 경로로 판정한다"
            );
        }
        assert_eq!(report_outside_at(&d, Some(&home), &p(".claude-5/projects/-w/s.jsonl"), &r, now), Err("identity_unresolved"));
        assert_eq!(report_outside_at(&d, None, &p(".claude-3/projects/-w/nope.jsonl"), &r, now), Err("home_unknown"));
        assert!(claude_rows(&d).is_empty(), "거절된 보고가 계정 행을 만들었다: {:?}", claude_rows(&d));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 빈도 상한(프로필 dir 단위): 1초 안의 재보고는 값이 달라도 버리고, 같은 값은 5초 안에서 버린다.
    #[test]
    fn outside_report_is_rate_limited_per_profile() {
        let dir = tmp("daemon-out-rl");
        let home = tmp("home-out-rl");
        write(&home.join(".claude-3/.claude.json"), r#"{"oauthAccount":{"accountUuid":"u-rl"}}"#);
        write(&home.join(".claude-3/projects/-w/s.jsonl"), "{}\n");
        write(&home.join(".claude-3/projects/-w/t.jsonl"), "{}\n"); // 같은 프로필의 다른 세션
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let s = home.join(".claude-3/projects/-w/s.jsonl").to_string_lossy().into_owned();
        let t = home.join(".claude-3/projects/-w/t.jsonl").to_string_lossy().into_owned();
        let t0 = crate::state::now_epoch();
        let a = [rw("5h", 10.0, None)];
        let b = [rw("5h", 11.0, None)];
        assert_eq!(report_outside_at(&d, Some(&home), &s, &a, t0), Ok(OutsideOutcome::Accepted));
        assert_eq!(report_outside_at(&d, Some(&home), &t, &b, t0 + 0.5), Ok(OutsideOutcome::Throttled), "1초 하한(다른 세션·다른 값이어도)");
        assert_eq!(report_outside_at(&d, Some(&home), &s, &a, t0 + 2.0), Ok(OutsideOutcome::Throttled), "같은 값 5초");
        assert_eq!(report_outside_at(&d, Some(&home), &s, &b, t0 + 2.0), Ok(OutsideOutcome::Accepted), "값이 바뀌면 1초 뒤 수용");
        assert_eq!(report_outside_at(&d, Some(&home), &s, &b, t0 + 7.5), Ok(OutsideOutcome::Accepted), "같은 값도 5초 뒤 수용");
        let row = claude_rows(&d).into_iter().find(|r| r["account_id"] == "u-rl").unwrap();
        assert_eq!(row["rate"][0]["used_pct"], json!(11.0));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 창 밖 값은 **표시용** — 계정 경보의 근거가 아니다(위조 가능 · 오너 승인 범위). 같은 계정이라도 좌석 보고가
    /// 최신이면 경보 대상이다.
    #[test]
    fn outside_values_are_display_only_not_alert_inputs() {
        let dir = tmp("daemon-out-alert");
        let home = tmp("home-out-alert");
        write(&home.join(".claude-3/.claude.json"), r#"{"oauthAccount":{"accountUuid":"u-al","emailAddress":"al@example.test"}}"#);
        write(&home.join(".claude-3/projects/-w/s.jsonl"), "{}\n");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let now = crate::state::now_epoch();
        let sess = home.join(".claude-3/projects/-w/s.jsonl").to_string_lossy().into_owned();
        assert_eq!(report_outside_at(&d, Some(&home), &sess, &[rw("5h", 97.0, None)], now), Ok(OutsideOutcome::Accepted));
        assert!(alert_rates(&d).is_empty(), "창 밖(표시용) 값이 경보 입력이 됐다: {:?}", alert_rates(&d));
        assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("5h", 97.0, None)], "statusline", now + 1.0));
        assert_eq!(alert_rates(&d), vec![("al@example.test".to_string(), "5h".to_string(), 97.0)]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// RC2-b: agy 상태줄 값이 antigravity 계정의 최신 출처면 RPC 수집기는 물러선다(참) — RPC 값·관측 전·행 없음은 거짓.
    #[test]
    fn agy_statusline_becomes_the_authoritative_source() {
        let dir = tmp("daemon-agy-auth");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        assert!(!agy_statusline_authoritative(&d), "행 없음");
        let now = crate::state::now_epoch();
        note_rate(&d, "gemini", "", &[rw("5h", 1.0, None)], "agy-rpc", now);
        assert!(!agy_statusline_authoritative(&d), "RPC 값");
        note_rate(&d, "gemini", "", &[rw("5h", 2.0, None)], AGY_STATUSLINE_SOURCE, now + 1.0);
        assert!(agy_statusline_authoritative(&d), "상태줄 값이 들어왔는데 수집기가 물러서지 않는다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ───────── fix-values-1 SP-1·F1 — 경보 입력은 표시 승자와 따로 둔다(수정 전 적색) ─────────
    // 종전: 창 밖 값의 경보 제외가 계정 뷰의 **최신 출처**에 걸려 있었고 note_rate 는 뷰를 통째로 최신 보고로 덮었다.
    // 그래서 창 밖 보고 한 건이 그 계정을 경보 입력에서 통째로 뺐고(억제), 좌석 보고가 다시 최신이 되면
    // check_alerts 가 지워진 키를 새 경보로 곧바로 다시 냈다(30분 리마인드 우회). 픽스처는 전부 합성값이다.

    /// 합성 프로필 하나(신원 + 실재 transcript)를 만들고 그 세션 경로를 돌려준다.
    fn outside_profile(home: &Path, dir: &str, uuid: &str, email: &str) -> String {
        write(
            &home.join(format!("{dir}/.claude.json")),
            &format!(r#"{{"oauthAccount":{{"accountUuid":"{uuid}","emailAddress":"{email}"}}}}"#),
        );
        write(&home.join(format!("{dir}/projects/-w/s.jsonl")), "{}\n");
        home.join(format!("{dir}/projects/-w/s.jsonl")).to_string_lossy().into_owned()
    }

    /// 버스에 실린 계정 경보 중 이 키의 건수.
    fn account_alerts(d: &Arc<Daemon>, seq0: u64, key: &str) -> usize {
        d.bus
            .replay_after(seq0)
            .iter()
            .filter(|e| e["name"] == "alert.account_rate" && e["payload"]["key"] == key)
            .count()
    }

    /// ① 좌석 97% 뒤에 창 밖 5% 가 와도 경보 입력에는 좌석 97% 가 남는다(표시는 창 밖 5% — 최신 승자 그대로).
    #[test]
    fn alert_input_keeps_the_seat_value_when_an_outside_report_is_newer() {
        let dir = tmp("daemon-alert-keep");
        let home = tmp("home-alert-keep");
        let sess = outside_profile(&home, ".claude-3", "u-keep", "keep@example.test");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let t0 = crate::state::now_epoch();
        assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("5h", 97.0, None)], "statusline", t0));
        assert_eq!(report_outside_at(&d, Some(&home), &sess, &[rw("5h", 5.0, None)], t0 + 1.0), Ok(OutsideOutcome::Accepted));
        let row = claude_rows(&d).into_iter().find(|r| r["account_id"] == "u-keep").unwrap();
        assert_eq!((row["source"].clone(), row["rate"][0]["used_pct"].clone()), (json!(OUTSIDE_SOURCE), json!(5.0)), "표시는 최신 승자");
        assert_eq!(
            alert_rates(&d),
            vec![("keep@example.test".to_string(), "5h".to_string(), 97.0)],
            "창 밖 보고 한 건이 좌석이 본 97% 를 경보 입력에서 지웠다"
        );
        // 좌석이 새 값을 보내면 경보 입력도 그 값으로 바뀐다(좌석 관측끼리는 최신 승자)
        assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("5h", 40.0, None)], "statusline", t0 + 2.0));
        assert_eq!(alert_rates(&d), vec![("keep@example.test".to_string(), "5h".to_string(), 40.0)]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ② 좌석 → 창 밖 → 좌석 을 check_alerts 표본(30초) 사이에 번갈아 넣어도 REMIND 안에서 재발화하지 않는다.
    /// REMIND 가 지나면 한 번 더 낸다(리마인드 자체는 살아 있다 — 음성 대조).
    #[test]
    fn alternating_seat_and_outside_reports_do_not_refire_within_remind() {
        let dir = tmp("daemon-alert-alt");
        let home = tmp("home-alert-alt");
        let sess = outside_profile(&home, ".claude-3", "u-alt", "alt@example.test");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let cfg = crate::alerts::AlertConfig::default();
        let mut fired: HashMap<String, f64> = HashMap::new();
        let key = "account_rate:alt@example.test:5h";
        let seq0 = d.bus.latest_seq();
        let t0 = crate::state::now_epoch();
        assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("5h", 97.0, None)], "statusline", t0));
        crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1.0);
        assert_eq!(account_alerts(&d, seq0, key), 1, "좌석 97% 가 경보를 내지 않았다");
        for i in 0..4 {
            let base = t0 + 60.0 * f64::from(i) + 10.0;
            let r = report_outside_at(&d, Some(&home), &sess, &[rw("5h", 5.0 + f64::from(i), None)], base);
            assert_eq!(r, Ok(OutsideOutcome::Accepted));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, base + 20.0); // 창 밖이 최신인 표본
            assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("5h", 97.0, None)], "statusline", base + 30.0));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, base + 50.0); // 좌석이 최신인 표본
        }
        assert_eq!(account_alerts(&d, seq0, key), 1, "REMIND 안에서 같은 키가 다시 발화했다(깜빡임)");
        crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1.0 + crate::governance::ALERT_REMIND_SECS);
        assert_eq!(account_alerts(&d, seq0, key), 2, "REMIND 뒤 리마인드가 사라졌다");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ③ 표본마다 창 밖 보고가 최신이어도(좌석 직후 창 밖) 좌석이 본 97% 는 계정 경보를 낸다 — 억제 방향 음성 대조.
    /// 창 밖 값만 있는 계정은 여전히 경보 입력이 아니다(표시용 불변).
    #[test]
    fn outside_latest_at_every_tick_does_not_hide_the_seat_alert() {
        let dir = tmp("daemon-alert-supp");
        let home = tmp("home-alert-supp");
        let sess = outside_profile(&home, ".claude-3", "u-sup", "sup@example.test");
        let only_out = outside_profile(&home, ".claude-4", "u-oo", "oo@example.test");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let cfg = crate::alerts::AlertConfig::default();
        let mut fired: HashMap<String, f64> = HashMap::new();
        let seq0 = d.bus.latest_seq();
        let t0 = crate::state::now_epoch();
        for i in 0..4 {
            let base = t0 + 30.0 * f64::from(i);
            assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("5h", 97.0, None)], "statusline", base));
            let r = report_outside_at(&d, Some(&home), &sess, &[rw("5h", 97.0 + f64::from(i) * 0.1, None)], base + 1.0);
            assert_eq!(r, Ok(OutsideOutcome::Accepted));
            let r = report_outside_at(&d, Some(&home), &only_out, &[rw("5h", 99.0 - f64::from(i), None)], base + 1.0);
            assert_eq!(r, Ok(OutsideOutcome::Accepted));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, base + 2.0);
        }
        assert_eq!(account_alerts(&d, seq0, "account_rate:sup@example.test:5h"), 1, "좌석 97% 계정 경보가 억제됐다");
        assert_eq!(account_alerts(&d, seq0, "account_rate:oo@example.test:5h"), 0, "창 밖 값만으로 경보가 났다");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ④ 데몬 재시작 뒤 스냅샷 복원: 표시는 마지막 스냅샷(창 밖이어도) · 경보 입력은 창 밖이 아닌 출처의 마지막
    /// 스냅샷만. 창 밖 값만 있는 계정은 복원 뒤에도 경보 입력이 아니다. 좌석 값이 창 밖 값과 1%p 안으로 붙어
    /// 와도(스냅샷 스로틀) 좌석 쪽 최신 값이 기록된다.
    #[test]
    fn snapshot_restore_keeps_outside_values_out_of_alert_inputs() {
        let dir = tmp("daemon-alert-restore");
        let home = tmp("home-alert-restore");
        let sess = outside_profile(&home, ".claude-3", "u-rs", "rs@example.test");
        let only_out = outside_profile(&home, ".claude-4", "u-ro", "ro@example.test");
        let sock = dir.join("cysd.sock");
        let t0 = crate::state::now_epoch();
        {
            let d1 = crate::state::Daemon::new(sock.clone());
            let seat = |pct: f64, t: f64| {
                assert!(note_rate_at(&d1, Some(&home), "claude", &sess, &[rw("5h", pct, None)], "statusline", t));
            };
            seat(50.0, t0);
            assert_eq!(report_outside_at(&d1, Some(&home), &sess, &[rw("5h", 97.0, None)], t0 + 2.0), Ok(OutsideOutcome::Accepted));
            seat(97.3, t0 + 4.0); // 직전 기록(창 밖 97)과 0.3%p — 종전 스로틀은 이 좌석 값을 버렸다
            assert_eq!(report_outside_at(&d1, Some(&home), &sess, &[rw("5h", 5.0, None)], t0 + 6.0), Ok(OutsideOutcome::Accepted));
            assert_eq!(report_outside_at(&d1, Some(&home), &only_out, &[rw("5h", 96.0, None)], t0 + 8.0), Ok(OutsideOutcome::Accepted));
        }
        let d2 = crate::state::Daemon::new(sock);
        restore_from_snapshots(&d2, t0 + 10.0);
        let rows = claude_rows(&d2);
        let rs = rows.iter().find(|r| r["account_id"] == "u-rs").expect("복원 행");
        assert_eq!(rs["rate"][0]["used_pct"], json!(5.0), "표시 복원은 마지막 스냅샷(창 밖 5%)");
        assert_eq!(rs["source"], "snapshot");
        assert!(rows.iter().any(|r| r["account_id"] == "u-ro"), "창 밖 값만 있는 계정도 표시는 복원된다");
        assert_eq!(
            alert_rates(&d2),
            vec![("rs@example.test".to_string(), "5h".to_string(), 97.3)],
            "재시작 뒤 경보 입력에 창 밖 값이 들어왔거나 좌석 값이 사라졌다"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }
}
