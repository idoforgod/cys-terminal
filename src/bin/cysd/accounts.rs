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
//!   ★fix-values-2 RV-SP-2: 경보 입력은 그 관측의 **라벨**도 싣는다(경보 키 `account_rate:{label}:{win}` · 문구).
//!   창 밖 보고는 표시 라벨만 바꾸고 경보 키는 바꾸지 못한다.
//!   ★fix-values-2 RV-SP-1 — **이 분리는 인증 경계가 아니다**(알려진 한계 · 오너 결정 대기 · IMPL-values §7-4).
//!   분리는 검증되지 않은 창 밖 값이 경보를 흔들지 않게 하는 정확성 조치다. 좌석 경로 `usage.report` 의 소유
//!   게이트는 **다른 좌석 안의** 호출자만 막고 pane 밖 호출자(조상 체인에 pane 없음)는 통과시키며, session_file 도
//!   검증하지 않는다. 그래서 같은 UID 프로세스는 claude 좌석 번호 하나만 대고 **어느 계정이든** 경보 입력을
//!   넣거나(가짜 crit) 덮을(진짜 좌석 경보 억제) 수 있고, 그 값은 좌석 출처로 스냅샷에 남아 재시작 뒤 7일
//!   ([`BOOT_RESTORE_SECS`])까지 복원된다. 검체 `handlers::usage_report_from_outside_any_pane_still_feeds_account_alerts`
//!   가 이 동작을, `manual_states_that_alert_separation_is_not_an_auth_boundary` 가 매뉴얼 고지를 박제한다.
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
    /// 계정 경보 입력 — 계정별 **창 밖이 아닌** 출처(좌석 statusline·rollout·agy·어댑터)의 마지막 신선 관측.
    /// 표시 승자(`AccountView.rate` — 창 벡터 통째 최신 승자)와 **분리**한다(fix-values-1 SP-1·F1):
    /// 표시 승자에 경보 제외를 걸면 창 밖 보고 한 건이 좌석이 본 값을 경보에서 통째로 지우고(억제), 좌석 보고가
    /// 다시 최신이 되면 check_alerts 가 재무장된 키를 곧바로 다시 발화한다(30분 리마인드 우회 · 깜빡임).
    /// 라벨도 여기 싣는다([`AlertInput::label`] · fix-values-2 RV-SP-2).
    alert_inputs: HashMap<AccountKey, AlertInput>,
    last_prune: f64,
    /// cys 창 밖 보고의 빈도 상한 상태 — (정규화된 프로필 dir) → (마지막 수용 시각, 그때의 rate).
    /// 키 공간은 검증을 통과한 **알려진 프로필 dir** 뿐이라 크기가 유계다.
    outside_last: HashMap<PathBuf, (f64, Vec<RateWindow>)>,
    /// ★fatal-fix R1-F1: 창 밖 보고의 **선상한** 상태 — (원문 session_file 의 프로필 접두) → 마지막 통과 시각.
    /// 호출자 추적(프로세스 표 전체 스캔) **앞**에서 쓴다. 크기는 [`OUTSIDE_PRE_KEYS_MAX`] 로 유계다.
    outside_pre: HashMap<String, f64>,
    /// 선상한 전역 토큰 버킷 — (남은 토큰, 마지막 보충 시각). None = 가득.
    outside_pre_bucket: Option<(f64, f64)>,
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
/// ★fatal-fix R1-F1: 선상한 전역 버킷 — 용량(건)·초당 보충(건). 호출자 추적 1회 ≈ 40ms CPU(프로세스 1,100개 · 릴리스
/// sysinfo 실측)이므로 최악 약 0.12코어로 묶인다. 정상 부하(창 밖 프로필마다 초당 1건 이하 · CLI 가 같은 값을 60초
/// 안에 다시 보내지 않는다)는 전부 지나간다.
const OUTSIDE_PRE_BURST: f64 = 6.0;
const OUTSIDE_PRE_REFILL_PER_SEC: f64 = 3.0;
/// 선상한 키 수 상한 — 넘치면 만료분을 걷고, 그래도 넘치면 새 키를 버린다(메모리 유계).
const OUTSIDE_PRE_KEYS_MAX: usize = 256;
/// ★fatal-fix R1-F2: 같은 창 라벨의 두 관측이 **같은 리셋 창**인지 가르는 허용 오차(초). agy 는 리셋을
/// `now + reset_in_seconds` 로 지어 보내 보고마다 몇 초씩 흔들린다. 서로 다른 창은 리셋이 최소 창 길이만큼 떨어진다.
const SAME_WINDOW_TOLERANCE_SECS: f64 = 900.0;
/// 같은 리셋 창의 최댓값을 **다시 확인 없이** 쥐는 시간(초) — 경보 리마인드 간격과 같다. 제공자가 창 중간에 사용률을
/// 내려 주는 드문 경우(일괄 리셋 등)에 옛 최댓값이 7일 창 내내 crit 로 남지 않게 한다(최대 한 리마인드 간격).
const PEAK_HOLD_SECS: f64 = 1800.0;

/// 계정 경보 입력 한 건 — 창 밖이 아닌 출처의 마지막 신선 관측([`AccountsState::alert_inputs`]).
#[derive(Clone, Debug, Default)]
struct AlertInput {
    /// 관측 시각(0.0 = 없음).
    at: f64,
    /// 그 관측이 해석한 계정 라벨 — 경보 키(`account_rate:{label}:{win}`)와 경보 문구가 이것을 쓴다. 뷰 라벨(표시 승자)을
    /// 쓰면 같은 accountUuid 에 다른 emailAddress 를 가진 프로필의 **창 밖** 보고 한 건이 키를 갈아 끼워, 새 키로 발화하고
    /// 좌석이 다시 보고하면 원래 키가 REMIND 안에 재발화했다(fix-values-2 RV-SP-2 · F1 과 같은 증상).
    label: String,
    rate: Vec<RateWindow>,
    /// ★fatal-fix R1-F2: 창 라벨 → 지금 쥐고 있는 값이 **마지막으로 확인된** 시각(그 값 이상을 보고한 관측). 같은 리셋 창의
    /// 최댓값은 이 시각에서 [`PEAK_HOLD_SECS`] 까지만 쥔다 — 비면(복원분) `at` 으로 본다.
    peak_at: HashMap<String, f64>,
}

/// 이 출처의 관측이 계정 경보 입력이 되는가 — 창 밖(표시용) 값만 아니다. ★이것은 검증되지 않은 창 밖 값이 경보를
/// 흔들지 않게 하는 **정확성** 조치다 — 인증 경계가 아니다(좌석 경로도 같은 UID 위조가 가능하다 · 모듈 머리 주석).
fn feeds_alerts(source: &str) -> bool {
    source != OUTSIDE_SOURCE
}

/// 경보 입력 갱신(창 밖이 아닌 출처만 · 경보 입력끼리 최신 승자 — 값·라벨을 함께 바꾼다). 호출자가 accounts 락을 잡고 있다.
fn note_alert_input(
    st: &mut AccountsState,
    key: &AccountKey,
    label: &str,
    rate: &[RateWindow],
    source: &str,
    now: f64,
) {
    if !feeds_alerts(source) || rate.is_empty() {
        return;
    }
    let slot = st.alert_inputs.entry(key.clone()).or_default();
    if now >= slot.at {
        let (merged, peak_at) = merge_alert_windows(&slot.rate, slot.at, &slot.peak_at, rate, now);
        *slot = AlertInput { at: now, label: label.to_string(), rate: merged, peak_at };
    }
}

/// ★fatal-fix R1-F2: 경보 입력 병합(순수 — 핀). 창 목록은 새 관측의 것이되(종전처럼 새 관측에 없는 창은 버린다),
/// 창마다 **같은 리셋 창**의 이전 값이 있으면 사용률은 둘 중 큰 쪽이다 — 한 리셋 창 안의 사용률은 줄지 않으므로 낮은
/// 값은 낡은 관측이다(유휴 좌석의 옛 값 · 여러 좌석이 같은 계정을 번갈아 보고). 종전 최신 승자는 96↔79 를 오가며
/// 경보 키를 한 틱 비활성으로 떨어뜨렸고, 워치독은 그 키를 재무장해 다음 틱에 다시 냈다(30분 리마인드 우회).
/// 단 쥐고 있는 최댓값은 마지막 확인에서 [`PEAK_HOLD_SECS`] 까지만 쥔다(제공자가 창 중간에 값을 내린 경우의 상한).
/// 새 관측의 리셋이 이전보다 **창 하나 이상 뒤**면 새 창이라 그 값이 이기고, **앞**이면 지난 창의 늦은 보고라 이전 값을
/// 지킨다(단, 이전 값의 리셋이 관측 시각에서 창 길이 넘게 먼 미래면 믿지 않는다). 리셋 시각이 한쪽이라도 없으면 창을
/// 가를 근거가 없으므로 종전대로 새 값이 이긴다. 반환: (창 목록, 창별 마지막 확인 시각).
fn merge_alert_windows(
    prev: &[RateWindow],
    prev_at: f64,
    prev_peak_at: &HashMap<String, f64>,
    new: &[RateWindow],
    now: f64,
) -> (Vec<RateWindow>, HashMap<String, f64>) {
    let mut peak_at = HashMap::new();
    let merged = new
        .iter()
        .map(|n| {
            let fresh = |peak_at: &mut HashMap<String, f64>| {
                peak_at.insert(n.label.clone(), now);
                n.clone()
            };
            let Some(p) = prev.iter().find(|p| p.label == n.label) else {
                return fresh(&mut peak_at);
            };
            let (Some(pr), Some(nr)) = (p.resets_at, n.resets_at) else {
                return fresh(&mut peak_at);
            };
            if !(pr.is_finite() && nr.is_finite()) {
                return fresh(&mut peak_at);
            }
            let p_seen = prev_peak_at.get(&n.label).copied().unwrap_or(prev_at);
            if (nr - pr).abs() <= SAME_WINDOW_TOLERANCE_SECS {
                if n.used_pct >= p.used_pct || now - p_seen > PEAK_HOLD_SECS {
                    let mut w = fresh(&mut peak_at);
                    w.resets_at = Some(pr.max(nr));
                    return w;
                }
                peak_at.insert(n.label.clone(), p_seen);
                return RateWindow { label: n.label.clone(), used_pct: p.used_pct, resets_at: Some(pr.max(nr)) };
            }
            let prev_plausible = crate::usage::window_secs(&n.label)
                .map_or(true, |len| pr <= prev_at + len + SAME_WINDOW_TOLERANCE_SECS);
            if nr < pr && prev_plausible {
                peak_at.insert(n.label.clone(), p_seen);
                p.clone()
            } else {
                fresh(&mut peak_at)
            }
        })
        .collect();
    (merged, peak_at)
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
/// (운영 경로는 락 밖에서 판독하는 `claude_identity_unlocked` 를 쓴다 — 이 얇은 판은 기존 검체용.)
#[cfg(test)]
fn claude_identity(
    state: &mut AccountsState,
    dir: &Path,
) -> Option<(String, String, Option<String>)> {
    claude_identity_at(state, dirs::home_dir().as_deref(), dir)
}

/// 신원 한 건 — (accountUuid, email, plan).
type Ident = (String, String, Option<String>);

/// 신원 파일 크기 상한(바이트). 넘으면 신원 불명(귀속 0) — 병적 입력(거대 파일)이 판독 시간·메모리를 밀지 못하게.
/// 실제 `.claude.json` 은 수십 KB~수 MB 다(이 맥 실측 59,906바이트 · 대화 이력이 쌓인 사용자는 더 크다) — 넉넉히 둔다.
const IDENTITY_FILE_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// ★fatal-fix R4-F2·F3: 신원 파일의 위치와 mtime — **메타데이터만** 본다(내용 무접촉 · accounts 락 밖에서 부른다).
/// 일반 파일이 아니면(FIFO·장치·디렉터리) None — 그런 것은 **열지 않는다**: FIFO 는 여는 순간 쓰는 쪽이 올 때까지
/// 막히고, 종전에는 그 open 이 전역 accounts 락 안이라 워치독(`alert_rates`)·부트 시드(bind 전)·모든 사용량 RPC 가
/// 함께 섰다. 반환 None 은 '신원 불명'(관측 미귀속 · 유령 계정 0)과 같은 방향이다.
fn identity_file_meta(home: Option<&Path>, dir: &Path) -> Option<(PathBuf, f64)> {
    let f = match home {
        Some(h) => cys::profile_gate::identity_config_file(h, dir),
        None => dir.join(".claude.json"),
    };
    let md = std::fs::metadata(&f).ok()?;
    if !md.is_file() {
        return None;
    }
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())?;
    Some((f, mtime))
}

/// 신원 파일 판독·파싱 — **락 밖 전용**. 여는 것도 막히지 않게 연다(unix `O_NONBLOCK` — 일반 파일 읽기에는 영향이
/// 없다)고, 연 뒤 fstat 으로 **일반 파일**인지 다시 확인한다(stat 과 open 사이에 FIFO 로 바뀌는 경쟁 차단).
/// 크기 상한을 넘으면 읽지 않는다. 자격증명(.credentials.json)은 읽지 않는다.
fn read_identity_file(f: &Path) -> Option<Ident> {
    use std::io::Read;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NONBLOCK);
    }
    let file = opts.open(f).ok()?;
    let md = file.metadata().ok()?;
    if !md.is_file() || md.len() > IDENTITY_FILE_MAX_BYTES {
        return None;
    }
    let mut s = String::new();
    file.take(IDENTITY_FILE_MAX_BYTES + 1).read_to_string(&mut s).ok()?;
    if s.len() as u64 > IDENTITY_FILE_MAX_BYTES {
        return None;
    }
    parse_identity(&s)
}

/// `.claude.json` 본문 → 신원(순수).
fn parse_identity(s: &str) -> Option<Ident> {
    let v = serde_json::from_str::<Value>(s).ok()?;
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
}

/// 캐시 조회(파일시스템 무접촉 — 락 안에서 불러도 된다). `Some(ident)` = 적중.
/// 캐시는 **읽은 파일** 기준 — 같은 dir 이라도 신원 파일이 바뀌면(명시 CLAUDE_CONFIG_DIR 로 폴더 안 파일이 새로
/// 생김) 다시 읽는다.
fn ident_cached(state: &AccountsState, dir: &Path, f: &Path, mtime: f64) -> Option<Option<Ident>> {
    state
        .ident_cache
        .get(dir)
        .filter(|e| e.mtime == mtime && e.file == f)
        .map(|e| e.ident.clone())
}

fn ident_store(state: &mut AccountsState, dir: &Path, f: PathBuf, mtime: f64, ident: Option<Ident>) {
    state.ident_cache.insert(dir.to_path_buf(), IdentEntry { file: f, mtime, ident });
}

/// 홈을 인자로 받는 시험 이음매. `home == None`(홈 불명)이면 종전 규칙(폴더 안 파일만).
/// ★이 판은 `&mut AccountsState` 를 직접 받는다 — **데몬 락을 쥔 채 부르지 않는다**(파일 IO 가 있다). 운영 경로는
/// [`claude_identity_unlocked`]·[`discover_at`] 을 쓴다(검체 전용).
#[cfg(test)]
fn claude_identity_at(state: &mut AccountsState, home: Option<&Path>, dir: &Path) -> Option<Ident> {
    let (f, mtime) = identity_file_meta(home, dir)?;
    if let Some(hit) = ident_cached(state, dir, &f, mtime) {
        return hit;
    }
    let ident = read_identity_file(&f);
    ident_store(state, dir, f, mtime, ident.clone());
    ident
}

/// ★fatal-fix R4-F2: 운영 경로의 신원 해석 — 캐시 조회·기록만 짧게 락 안에서 하고 **파일 IO 는 전부 락 밖**이다.
fn claude_identity_unlocked(accounts: &std::sync::Mutex<AccountsState>, home: Option<&Path>, dir: &Path) -> Option<Ident> {
    let (f, mtime) = identity_file_meta(home, dir)?;
    let hit = {
        let st = accounts.lock().unwrap();
        ident_cached(&st, dir, &f, mtime)
    };
    if let Some(hit) = hit {
        return hit;
    }
    let ident = read_identity_file(&f);
    ident_store(&mut accounts.lock().unwrap(), dir, f, mtime, ident.clone());
    ident
}

/// 귀속 결과 — (키, 라벨, plan, 프로필 표기).
type Resolution = (AccountKey, String, Option<String>, Option<String>);

/// claude 프로필 dir + 신원 → 귀속 결과(순수).
fn claude_resolution(home: Option<&Path>, dir: &Path, ident: Ident) -> Resolution {
    let (uuid, email, plan) = ident;
    (
        AccountKey { provider: "claude".into(), account_id: uuid },
        email,
        plan,
        Some(profile_short(home, dir)),
    )
}

/// 단일 홈 provider(codex·agy) → 귀속 결과. 미지 agent → None. (agy 는 데이터 폴더 **존재**만 본다 — 메타데이터.)
fn fixed_resolution(home: Option<&Path>, agent: &str) -> Option<Resolution> {
    match agent {
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

/// agent + 세션 파일 → (키, 라벨, plan, 프로필 표기). claude는 신원 해석 실패 시 None(스킵).
/// (운영 경로는 락 밖에서 해석하는 `note_rate_at` 을 쓴다 — 이 얇은 판은 기존 검체용.)
#[cfg(test)]
fn resolve(
    state: &mut AccountsState,
    agent: &str,
    session_file: &str,
) -> Option<(AccountKey, String, Option<String>, Option<String>)> {
    resolve_at(state, dirs::home_dir().as_deref(), agent, session_file)
}

/// 홈을 인자로 받는 시험 이음매(검체 전용 — `&mut AccountsState` 를 직접 받는다).
#[cfg(test)]
fn resolve_at(
    state: &mut AccountsState,
    home: Option<&Path>,
    agent: &str,
    session_file: &str,
) -> Option<Resolution> {
    match agent {
        "claude" => {
            let dir = profile_dir_from_session(session_file)?;
            let ident = claude_identity_at(state, home, &dir)?;
            Some(claude_resolution(home, &dir, ident))
        }
        other => fixed_resolution(home, other),
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
/// ★fatal-fix R4-F2: 신원 해석(파일 IO)은 accounts 락 **밖**에서 끝낸 뒤 락을 잡는다.
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
    let resolved = match agent {
        "claude" => profile_dir_from_session(session_file).and_then(|dir| {
            claude_identity_unlocked(&daemon.accounts, home, &dir).map(|ident| claude_resolution(home, &dir, ident))
        }),
        other => fixed_resolution(home, other),
    };
    let Some(resolved) = resolved else {
        return false; // 미귀속(신원 불명) — 유령 계정을 만들지 않는다
    };
    note_resolved(daemon, resolved, rate, source, now)
}

/// ★fatal-fix (a): claude 좌석 보고를 **좌석의 설정 폴더**(데몬이 그 좌석에 넣어 준 `CLAUDE_CONFIG_DIR`)로 귀속한다 —
/// 호출자가 댄 transcript 경로로는 신원 파일을 고르지 않는다(호출자 경로로 파일시스템을 건드리지 않는다 · R4-F2 ③).
/// 대조([`session_in_profile`])는 부른 쪽(handlers)이 먼저 끝낸다.
pub fn note_rate_for_profile(daemon: &Arc<Daemon>, profile_dir: &Path, rate: &[RateWindow], source: &str, now: f64) -> bool {
    note_rate_for_profile_at(daemon, dirs::home_dir().as_deref(), profile_dir, rate, source, now)
}

/// 홈을 인자로 받는 시험 이음매.
fn note_rate_for_profile_at(
    daemon: &Arc<Daemon>,
    home: Option<&Path>,
    profile_dir: &Path,
    rate: &[RateWindow],
    source: &str,
    now: f64,
) -> bool {
    if rate.is_empty() {
        return false;
    }
    let Some(ident) = claude_identity_unlocked(&daemon.accounts, home, profile_dir) else {
        return false;
    };
    note_resolved(daemon, claude_resolution(home, profile_dir, ident), rate, source, now)
}

/// 귀속이 정해진 관측을 계정 뷰·경보 입력·스냅샷에 싣는다(락 안은 메모리 연산뿐 · 파일 IO 없음).
fn note_resolved(daemon: &Arc<Daemon>, resolved: Resolution, rate: &[RateWindow], source: &str, now: f64) -> bool {
    let (key, label, plan, profile) = resolved;
    // 1) accounts 락 안에서 병합 + 영속 대상 수집 (analytics 락은 여기서 잡지 않는다 — 잠금 순서)
    let mut to_persist: Vec<(AccountKey, String, String, f64, Option<f64>)> = Vec::new();
    let mut do_prune = false;
    {
        let mut st = daemon.accounts.lock().unwrap();
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
        // 표시 라벨은 최신 승자(출처 무관 · 표시 규칙 무변경). 경보 라벨은 아래 경보 입력에 따로 싣는다(RV-SP-2).
        view.label = label.clone();
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
        // 경보 입력은 따로 — 창 밖 값은 여기 들어오지 않고, 들어와 있던 좌석 값·라벨을 지우거나 바꾸지도 않는다.
        note_alert_input(&mut st, &key, &label, rate, source, now);
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
        // ★fatal-fix R4-F3: 발견(파일 IO)은 락 밖에서 끝내고, 락 안에서는 메모리에 싣기만 한다. 이 함수는 소켓 bind
        //   **전에** 동기로 돈다 — 종전에는 신원 파일 하나가 막히면(FIFO 등) 락을 쥔 채 부트 체인 전체가 섰다.
        let found = discover_at(&home);
        {
            let mut st = daemon.accounts.lock().unwrap();
            apply_discovered(&mut st, &home, found);
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
    // 경보 라벨은 그 행들(창 밖 제외) 중 가장 최근 행의 라벨 — 표시 복원의 라벨(창 밖 행일 수 있다)을 쓰지 않는다(RV-SP-2).
    let mut restored: HashMap<AccountKey, AlertInput> = HashMap::new();
    for (ts, provider, account, label, win, pct, resets) in alert_rows {
        let slot = restored.entry(AccountKey { provider, account_id: account }).or_default();
        if ts >= slot.at {
            slot.at = ts;
            slot.label = label;
        }
        slot.rate.push(RateWindow { label: win, used_pct: pct, resets_at: resets });
    }
    if !restored.is_empty() {
        let mut st = daemon.accounts.lock().unwrap();
        for (key, mut input) in restored {
            input.rate.sort_by_key(|w| u8::from(w.label != "5h"));
            st.alert_inputs.entry(key).or_insert(input);
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

/// 부트 시드 ①의 발견 결과(파일 IO 로 만든 것 — 락 밖에서 만든다).
struct Discovered {
    /// (프로필 dir, 신원 파일·mtime·신원) — 신원 파일이 일반 파일이 아니면 None(캐시에도 싣지 않는다).
    claude: Vec<(PathBuf, Option<(PathBuf, f64, Option<Ident>)>)>,
    codex: bool,
    agy: Vec<String>,
}

/// 부트 시드 ①의 **IO 절반** — 설치 흔적을 훑고 신원 파일을 읽는다(락 밖 전용 · FIFO 등 일반 파일이 아닌 신원은
/// 열지 않는다).
fn discover_at(home: &Path) -> Discovered {
    // ★(U-17) 프로필 dir 열거 규칙은 **lib 정본 하나**다(`cys::profile_gate`). 종전엔 이
    //   함수 안에만 있었고, 인증 판정기가 같은 규칙을 재구현하면 두 벌이 갈린다(한쪽만
    //   새 부서 접두를 배우는 식) — 같은 목록을 두 소비처가 보게 한다.
    //   ★판정은 바뀌지 않는다: 정본 함수는 종전 두 루프와 **같은 이름 규칙·같은 순서**이며
    //   `is_dir()` 검사도 더하지 않는다(동작 동일성 유지 — 완화도 강화도 아니다).
    let claude = cys::profile_gate::enumerate_profile_dirs(home)
        .into_iter()
        .map(|dir| {
            let read = identity_file_meta(Some(home), &dir).map(|(f, mtime)| {
                let ident = read_identity_file(&f);
                (f, mtime, ident)
            });
            (dir, read)
        })
        .collect();
    Discovered { claude, codex: home.join(".codex").is_dir(), agy: antigravity_profiles(home) }
}

/// 부트 시드 ①의 **메모리 절반**(락 안 · 파일 IO 없음).
fn apply_discovered(st: &mut AccountsState, home: &Path, found: Discovered) {
    for (dir, read) in found.claude {
        let Some((f, mtime, ident)) = read else {
            continue;
        };
        ident_store(st, &dir, f, mtime, ident.clone());
        if let Some((uuid, email, plan)) = ident {
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
    if found.codex {
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
    if !found.agy.is_empty() {
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
        v.profiles.extend(found.agy);
    }
}

/// 부트 시드 ①(설치 흔적 스캔) — 홈을 인자로 받는 시험 이음매. 계정 **발견**만 한다(rate 없음).
/// (운영 경로 `seed_known` 은 IO 절반을 락 밖에서 따로 부른다.)
#[cfg(test)]
fn seed_discovered(st: &mut AccountsState, home: &Path) {
    apply_discovered(st, home, discover_at(home));
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
    let label = v.label.clone();
    note_alert_input(&mut st, &key, &label, rate, source, now);
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
/// (운영 경로 = async 수집기는 비대기 판 [`try_note_agy_error`] 을 쓴다 — 이 판은 검체용.)
#[cfg(test)]
#[allow(dead_code)]
pub fn note_agy_error(daemon: &Arc<Daemon>, err: Option<&str>) {
    note_agy_error_at(daemon, dirs::home_dir().as_deref(), err)
}

/// 홈을 인자로 받는 시험 이음매. `home == None`(홈 불명) = 근거 확인 불가 → 새 행을 만들지 않는다.
#[cfg(test)]
fn note_agy_error_at(daemon: &Arc<Daemon>, home: Option<&Path>, err: Option<&str>) {
    // 데이터 폴더 존재 확인(메타데이터)은 락 밖에서 — 락 안은 메모리 연산뿐(R4-F2 와 같은 규율).
    let profiles = agy_error_profiles(home, err);
    let mut st = daemon.accounts.lock().unwrap();
    apply_agy_error(&mut st, err, profiles);
}

/// 오류로 행을 새로 만들 때의 근거(실재하는 agy 데이터 폴더) — 오류가 없으면 볼 필요가 없다.
fn agy_error_profiles(home: Option<&Path>, err: Option<&str>) -> BTreeSet<String> {
    match err {
        Some(_) => home.map(|h| antigravity_profiles(h).into_iter().collect()).unwrap_or_default(),
        None => BTreeSet::new(),
    }
}

/// 락 안 절반(메모리 연산뿐).
fn apply_agy_error(st: &mut AccountsState, err: Option<&str>, profiles: BTreeSet<String>) {
    let key = AccountKey { provider: "antigravity".into(), account_id: "default".into() };
    match err {
        Some(code) => {
            if let Some(v) = st.views.get_mut(&key) {
                v.source_error = Some(code.to_string());
                return;
            }
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
/// (운영 경로 = async 수집기는 비대기 판 [`try_agy_statusline_authoritative`] 을 쓴다 — 이 판은 검체용.)
#[cfg(test)]
pub fn agy_statusline_authoritative(daemon: &Arc<Daemon>) -> bool {
    let st = daemon.accounts.lock().unwrap();
    let key = AccountKey { provider: "antigravity".into(), account_id: "default".into() };
    st.views.get(&key).is_some_and(|v| v.source == AGY_STATUSLINE_SOURCE)
}

/// accounts 락을 **기다리지 않고** 잡는다 — None = 경합. async 문맥(수집기) 전용: 표준 뮤텍스를 기다리면 tokio 워커가
/// 붙잡히고, 그 워커가 IO 드라이버를 돌리던 것이면 데몬의 모든 소켓 요청이 멈춘다(fatal-fix R4-F1).
fn try_accounts(daemon: &Daemon) -> Option<std::sync::MutexGuard<'_, AccountsState>> {
    match daemon.accounts.try_lock() {
        Ok(g) => Some(g),
        Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

/// [`agy_statusline_authoritative`] 의 비대기 판 — None = 락 경합(부른 쪽은 그 틱을 건너뛴다).
pub fn try_agy_statusline_authoritative(daemon: &Arc<Daemon>) -> Option<bool> {
    let st = try_accounts(daemon)?;
    let key = AccountKey { provider: "antigravity".into(), account_id: "default".into() };
    Some(st.views.get(&key).is_some_and(|v| v.source == AGY_STATUSLINE_SOURCE))
}

/// [`note_agy_error`] 의 비대기 판 — 반환: 적었는가(false = 락 경합 · 다음 틱에 다시 적힌다).
pub fn try_note_agy_error(daemon: &Arc<Daemon>, err: Option<&str>) -> bool {
    let home = dirs::home_dir();
    let profiles = agy_error_profiles(home.as_deref(), err);
    let Some(mut st) = try_accounts(daemon) else {
        return false;
    };
    apply_agy_error(&mut st, err, profiles);
    true
}

/// ★fatal-fix R1-F1 · N3 · F5 · W4: 창 밖 보고의 **선상한** — 호출자 추적(새 pid 마다 프로세스 표 전체 스캔 · 호출당
/// 약 40ms CPU)과 파일시스템 검사 **앞**에서 부른다(락 안은 메모리 연산뿐). 원문 session_file 의 프로필 접두마다
/// [`OUTSIDE_MIN_INTERVAL_SECS`] 안의 재시도를 버리고, 전체로는 토큰 버킷([`OUTSIDE_PRE_BURST`]·
/// [`OUTSIDE_PRE_REFILL_PER_SEC`])을 넘는 시도를 버린다. 통과 = 뒤의 비싼 검사로 간다. 버려진 보고는 아무것도 쓰지
/// 않는다(표시용 값 한 건 — 다음 상태줄 호출이 다시 보낸다). 시계가 뒤로 가면 막지 않는다(근거 없음 = 통과).
pub fn outside_prethrottle(daemon: &Arc<Daemon>, session_file: &str, now: f64) -> bool {
    let key = profile_dir_from_session(session_file)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut st = daemon.accounts.lock().unwrap();
    outside_prethrottle_in(&mut st, key, now)
}

fn outside_prethrottle_in(st: &mut AccountsState, key: String, now: f64) -> bool {
    if let Some(t) = st.outside_pre.get(&key) {
        if now >= *t && now - *t < OUTSIDE_MIN_INTERVAL_SECS {
            return false;
        }
    }
    let (tokens, last) = st.outside_pre_bucket.unwrap_or((OUTSIDE_PRE_BURST, now));
    let tokens = (tokens + (now - last).max(0.0) * OUTSIDE_PRE_REFILL_PER_SEC).min(OUTSIDE_PRE_BURST);
    if tokens < 1.0 {
        st.outside_pre_bucket = Some((tokens, now));
        return false;
    }
    if st.outside_pre.len() >= OUTSIDE_PRE_KEYS_MAX && !st.outside_pre.contains_key(&key) {
        st.outside_pre.retain(|_, t| now >= *t && now - *t < OUTSIDE_MIN_INTERVAL_SECS);
        if st.outside_pre.len() >= OUTSIDE_PRE_KEYS_MAX {
            return false;
        }
    }
    st.outside_pre_bucket = Some((tokens - 1.0, now));
    st.outside_pre.insert(key, now);
    true
}

/// ★fatal-fix (a) · W2: 좌석 보고의 transcript 가 **그 좌석 설정 폴더**(`<profile_dir>/projects/` 아래)의 것인가.
/// 먼저 표기만 접어 비교하고(파일시스템 무접촉 · Windows 표기 4종 — `\`↔`/` · `\\?\` · MSYS `/c/` · 드라이브 대소 —
/// [`crate::reclaim::norm_path_on`]), 다르면 **둘 다 실재할 때만** 정규화(심볼릭 링크·`/tmp`↔`/private/tmp`·Windows
/// 실제 대소문자)로 한 번 더 본다. `profile_dir_from_session` 처럼 첫 `/projects/` 를 찾지 않는다 — 홈 경로 자체에
/// `/projects/` 가 들어 있어도 오판하지 않는다. 락 밖에서 부른다.
pub fn session_in_profile(session_file: &str, profile_dir: &str) -> bool {
    session_in_profile_on(session_file, profile_dir, cfg!(windows)) || session_in_profile_fs(session_file, profile_dir)
}

/// 위의 **순수** 절반(플랫폼 의미론을 인자로 받는다 — Windows 표기 검체가 unix 에서도 돈다).
pub fn session_in_profile_on(session_file: &str, profile_dir: &str, windows: bool) -> bool {
    if session_file.trim().is_empty() || profile_dir.trim().is_empty() {
        return false;
    }
    let s = crate::reclaim::norm_path_on(session_file, windows);
    let c = crate::reclaim::norm_path_on(profile_dir, windows);
    let prefix = if c.ends_with('/') { format!("{c}projects/") } else { format!("{c}/projects/") };
    let Some(rest) = s.strip_prefix(&prefix) else {
        return false;
    };
    !rest.is_empty() && !rest.split('/').any(|seg| seg == "..")
}

fn session_in_profile_fs(session_file: &str, profile_dir: &str) -> bool {
    let p = Path::new(session_file);
    if session_file.trim().is_empty() || profile_dir.trim().is_empty() || !p.is_absolute() {
        return false;
    }
    let Ok(cs) = std::fs::canonicalize(p) else {
        return false; // transcript 가 없으면 프로필 폴더는 건드리지도 않는다
    };
    let Ok(cc) = std::fs::canonicalize(profile_dir) else {
        return false;
    };
    let projects = cc.join("projects");
    cs.starts_with(&projects) && cs != projects
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
/// 창 밖(표시용) 값은 경보 근거가 아니다 — 검증되지 않은 값이 경보를 흔들지 않게 하려는 것이다. 그래서 경보는
/// 같은 계정의 **창 밖이 아닌 관측 중 가장 최근 값**으로 판정한다 — 창 밖 값이 더 최신이어도 그 값이 남는다
/// (표시 숫자와 다를 수 있다). 창 밖 값만 있는 계정은 경보 입력이 없다. 라벨(=경보 키의 일부)도 그 관측의 라벨이다
/// — 뷰 라벨(표시 승자)은 창 밖 보고가 바꿀 수 있다(fix-values-2 RV-SP-2).
/// ★인증 경계가 아니다(fix-values-2 RV-SP-1 · 알려진 한계 · 오너 결정 대기): 좌석 경로 `usage.report` 는 pane 밖
/// 호출자를 막지 않으므로, 같은 UID 프로세스는 좌석 번호 하나만 대고 **어느 계정이든** 여기 들어가는 값을 넣거나
/// 덮을 수 있다(가짜 경보 · 진짜 경보 억제 둘 다). 위조 값은 좌석 출처로 스냅샷에 남아 재시작 뒤에도 복원된다.
/// ★fatal-fix R3-2 · ROLE-3: 리셋 시각이 지난 창(리셋 시각이 없으면 창 길이보다 오래된 관측)은 싣지 않는다
/// ([`crate::usage::rate_window_live`] — UI 의 '리셋됨'과 같은 규칙).
pub fn alert_rates(daemon: &Arc<Daemon>) -> Vec<(String, String, f64)> {
    let now = crate::state::now_epoch();
    let st = daemon.accounts.lock().unwrap();
    let mut out = Vec::new();
    for input in st.alert_inputs.values() {
        if input.at == 0.0 {
            continue;
        }
        for w in &input.rate {
            if !crate::usage::rate_window_live(w, input.at, now) {
                continue;
            }
            out.push((input.label.clone(), w.label.clone(), w.used_pct));
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

    // ───────── fix-values-2 RV-SP-2 — 창 밖 보고는 **라벨**로도 경보에 닿지 않는다(수정 전 적색) ─────────
    // 종전: 경보 값은 alert_inputs 에서 읽었지만 라벨은 뷰(표시 승자)에서 읽었다. 경보 키는 `account_rate:{label}:{win}`
    // 이라, 같은 accountUuid 에 다른 emailAddress 를 가진 프로필의 창 밖 보고 한 건이 키를 갈아 끼웠다 — 새 키로 곧바로
    // 발화하고, 좌석이 다시 보고하면 원래 키가 REMIND 안에 재발화했다(F1 과 같은 증상). 픽스처는 전부 합성값이다.

    /// ⑤ 좌석 97 → 경보 1 → 같은 uuid·다른 이메일 프로필의 창 밖 보고 → 30초 표본 둘 → 좌석 97 → 표본.
    /// 기대: REMIND 안 계정 경보 합계 1건, 키는 좌석 라벨 하나. 표시 라벨은 최신 승자 그대로(표시 규칙 무변경 · 대조).
    #[test]
    fn outside_report_label_does_not_rekey_or_refire_account_alerts() {
        let dir = tmp("daemon-alert-label");
        let home = tmp("home-alert-label");
        let seat_sess = outside_profile(&home, ".claude-3", "u-lbl", "seat@example.test");
        let other_sess = outside_profile(&home, ".claude-7", "u-lbl", "zz@example.test");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let cfg = crate::alerts::AlertConfig::default();
        let mut fired: HashMap<String, f64> = HashMap::new();
        let seat_key = "account_rate:seat@example.test:5h";
        let seq0 = d.bus.latest_seq();
        let t0 = crate::state::now_epoch();
        assert!(note_rate_at(&d, Some(&home), "claude", &seat_sess, &[rw("5h", 97.0, None)], "statusline", t0));
        crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1.0);
        assert_eq!(account_alerts(&d, seq0, seat_key), 1, "좌석 97% 가 경보를 내지 않았다");
        assert_eq!(
            report_outside_at(&d, Some(&home), &other_sess, &[rw("5h", 3.0, None)], t0 + 5.0),
            Ok(OutsideOutcome::Accepted)
        );
        let row = claude_rows(&d).into_iter().find(|r| r["account_id"] == "u-lbl").unwrap();
        assert_eq!(row["label"], json!("zz@example.test"), "표시 라벨은 최신 승자(표시 규칙은 바꾸지 않는다)");
        crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 31.0);
        crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 61.0);
        assert!(note_rate_at(&d, Some(&home), "claude", &seat_sess, &[rw("5h", 97.0, None)], "statusline", t0 + 70.0));
        crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 91.0);
        let keys: Vec<String> = d
            .bus
            .replay_after(seq0)
            .iter()
            .filter(|e| e["name"] == "alert.account_rate")
            .map(|e| e["payload"]["key"].as_str().unwrap_or("").to_string())
            .collect();
        assert_eq!(
            keys,
            vec![seat_key.to_string()],
            "창 밖 보고가 경보 키를 갈아 끼워 REMIND 안에 새로 발화·재발화했다"
        );
        assert_eq!(
            alert_rates(&d),
            vec![("seat@example.test".to_string(), "5h".to_string(), 97.0)],
            "경보 라벨이 창 밖 보고의 라벨로 바뀌었다"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ⑥ 재시작 복원도 경보 라벨을 **경보 입력 출처의 스냅샷 행**에서 가져온다 — 창 밖 행의 라벨이 마지막이어도.
    #[test]
    fn snapshot_restore_takes_the_alert_label_from_seat_rows() {
        let dir = tmp("daemon-alert-label-rs");
        let home = tmp("home-alert-label-rs");
        let seat_sess = outside_profile(&home, ".claude-3", "u-lrs", "seat2@example.test");
        let other_sess = outside_profile(&home, ".claude-7", "u-lrs", "zz2@example.test");
        let sock = dir.join("cysd.sock");
        let t0 = crate::state::now_epoch();
        {
            let d1 = crate::state::Daemon::new(sock.clone());
            assert!(note_rate_at(&d1, Some(&home), "claude", &seat_sess, &[rw("5h", 97.0, None)], "statusline", t0));
            assert_eq!(
                report_outside_at(&d1, Some(&home), &other_sess, &[rw("5h", 3.0, None)], t0 + 2.0),
                Ok(OutsideOutcome::Accepted)
            );
        }
        let d2 = crate::state::Daemon::new(sock);
        restore_from_snapshots(&d2, t0 + 10.0);
        assert_eq!(alert_rates(&d2), vec![("seat2@example.test".to_string(), "5h".to_string(), 97.0)]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    // ───────── fix-values-2 RV-SP-1 — 문서 계약: 경보/표시 분리는 인증 경계가 아니다 ─────────
    // 종전 매뉴얼은 "같은 UID 프로그램이 보낼 수 있어서 창 밖 값을 경보에서 뺀다 · 경보는 좌석 값으로 판정"만 적어,
    // 좌석 값은 위조되지 않는 것처럼 읽혔다. 실제로는 좌석 경로 `usage.report` 가 pane 밖 호출자를 막지 않는다
    // (handlers 검체 `usage_report_from_outside_any_pane_still_feeds_account_alerts` 가 그 동작을 박제한다).
    /// 매뉴얼의 사이드바 사용량 절이 이 한계(좌석 경로로도 경보 값을 넣거나 덮을 수 있다)를 적고 있어야 한다.
    #[test]
    fn manual_states_that_alert_separation_is_not_an_auth_boundary() {
        let manual = include_str!("../../../USER-MANUAL.md");
        let start = manual.find("**창 밖 값은 표시용입니다.**").expect("사이드바 사용량 절의 창 밖 값 문단");
        let end = manual[start..].find("- 모이지 않는 경우:").map_or(manual.len(), |i| start + i);
        // 줄바꿈 위치와 무관하게 보도록 공백을 하나로 접는다.
        let para = manual[start..end].split_whitespace().collect::<Vec<_>>().join(" ");
        for needle in ["인증 경계가 아닙니다", "usage.report", "좌석 번호", "가짜", "나지 않게"] {
            assert!(para.contains(needle), "창 밖 값 문단에 같은 UID 한계({needle})가 없다:\n{para}");
        }
        assert!(para.contains("80%·95%"), "계정 경보 기본 임계는 80%·95% 다(alerts.rs AlertConfig::default):\n{para}");
    }

    /// ★fatal-fix W6: 매뉴얼의 agy 상태줄 연결 예시는 POSIX(`sh ~/…`) 하나뿐이었다 — 윈도우는 `~` 가 펼쳐지지 않고 `sh` 가
    /// 보통 PATH 에 없다. 팩의 윈도우 훅 규약(`bash "C:/…"` 정슬래시 + 따옴표 — javis_preflight `_cys_hook_cmd`)과 같은
    /// 모양의 예시와 '윈도우 미검증' 고지가 있어야 한다. (W5) 윈도우에서 곧바로 '상태줄 연결 필요'가 보이는 이유도 적는다.
    #[test]
    fn manual_gives_a_windows_agy_statusline_example() {
        let manual = include_str!("../../../USER-MANUAL.md");
        let start = manual.find("**Antigravity(agy) 값**").expect("agy 값 문단");
        let end = manual[start..].find("- 갱신: 약 30초마다").map_or(manual.len(), |i| start + i);
        let para = &manual[start..end];
        assert!(para.contains(r#"bash \"C:/Users/<you>/.cys/pack/hooks/cys-statusline.sh\""#), "윈도우 예시가 없다:\n{para}");
        assert!(para.contains("아직 실제로 확인하지 못했습니다"), "윈도우 미검증 고지가 없다");
        assert!(para.contains("Windows 에서는 cys 가 agy 내부 서버를 아예 찾을 수 없어"), "W5 고지가 없다");
    }

    // ───────── fatal-fix (2026-09-24) — 치명위험 재검증 지적 수정(수정 전 적색) ─────────
    // 픽스처는 전부 합성값(*@example.test · 임시 폴더)이다.

    /// FIFO 를 만든다(유닉스 전용 검체 이음매).
    #[cfg(unix)]
    fn mkfifo(p: &Path) {
        use std::os::unix::ffi::OsStrExt;
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let c = std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap();
        // SAFETY: 널 종단 경로 · 반환값만 본다.
        let rc = unsafe { libc::mkfifo(c.as_ptr(), 0o600) };
        assert_eq!(rc, 0, "mkfifo 실패: {}", std::io::Error::last_os_error());
    }

    /// 막힌 FIFO 판독자를 풀어 준다(적색 단계에서 검체 스레드가 영원히 남지 않게) — 판독자가 없으면 아무 일도 없다.
    #[cfg(unix)]
    fn release_fifo(p: &Path) {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(p);
    }

    /// R4-F2: 신원 파일(`.claude.json`)이 FIFO 면 종전에는 `accounts` 락을 쥔 채 open 에서 영원히 멈췄다 — 그동안
    /// 워치독의 `alert_rates` 가 같은 락에서 멈춰 큐 배달·데드맨이 전부 섰다. 이제 신원 판독은 락 밖이고 **일반 파일만**
    /// 연다: FIFO 신원은 '신원 불명'(귀속 0)으로 곧바로 끝나고 락은 잠깐도 묶이지 않는다.
    #[cfg(unix)]
    #[test]
    fn fatal_fix_fifo_identity_never_holds_the_accounts_lock() {
        let dir = tmp("ff-fifo-daemon");
        let home = tmp("ff-fifo-home");
        write(&home.join(".claude-z/projects/-w/s.jsonl"), "{}\n");
        let fifo = home.join(".claude-z/.claude.json");
        mkfifo(&fifo);
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let sess = home.join(".claude-z/projects/-w/s.jsonl").to_string_lossy().into_owned();
        let now = crate::state::now_epoch();
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let (d, home) = (d.clone(), home.clone());
            std::thread::spawn(move || {
                let _ = tx.send(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("5h", 50.0, None)], "statusline", now));
            });
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (tx2, rx2) = std::sync::mpsc::channel();
        {
            let d = d.clone();
            std::thread::spawn(move || {
                let _ = tx2.send(alert_rates(&d));
            });
        }
        let watchdog_side = rx2.recv_timeout(std::time::Duration::from_secs(3));
        let reporter_side = rx.recv_timeout(std::time::Duration::from_secs(3));
        release_fifo(&fifo);
        assert!(watchdog_side.is_ok(), "FIFO 신원 판독이 accounts 락을 쥔 채 멈췄다(워치독 alert_rates 정지)");
        assert_eq!(reporter_side, Ok(false), "FIFO 신원은 '신원 불명'으로 곧바로 끝나야 한다(귀속 0)");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ★fatal-fix (a) · W2: 좌석 설정 폴더 대조는 **표기 차이만** 접는다(파일시스템 무접촉 순수 절반). 윈도우에서 데몬이
    /// 기록하는 네이티브 `C:\Users\x\.cys\claude` 와 Claude 가 싣는 transcript 표기(역슬래시·정슬래시·MSYS `/c/`·
    /// 확장 길이 `\\?\` · 드라이브 대소)가 같은 폴더로 읽혀야 한다 — 문자열 비교면 윈도우 전 좌석이 늘 불일치다.
    /// 첫 `/projects/` 를 찾지 않는다(홈 경로에 `/projects/` 가 있어도 오판 없음) · `..` 는 거절 · 결측은 불일치.
    #[test]
    fn fatal_fix_session_in_profile_folds_notation_only() {
        let cfg = r"C:\Users\x\.cys\claude";
        for sf in [
            r"C:\Users\x\.cys\claude\projects\C--Users-x-p\s.jsonl",
            "C:/Users/x/.cys/claude/projects/C--Users-x-p/s.jsonl",
            "/c/Users/x/.cys/claude/projects/C--Users-x-p/s.jsonl",
            r"\\?\C:\Users\x\.cys\claude\projects\C--Users-x-p\s.jsonl",
            r"c:\Users\x\.cys\claude\projects\C--Users-x-p\s.jsonl",
        ] {
            assert!(session_in_profile_on(sf, cfg, true), "윈도우 표기가 같은 좌석 폴더로 읽히지 않는다: {sf}");
        }
        assert!(session_in_profile_on(r"C:\Users\x\.cys\claude\projects\a\s.jsonl", r"C:\Users\x\.cys\claude\", true), "후행 구분자");
        // 다른 폴더(접두만 같은 형제 폴더 포함)는 불일치
        assert!(!session_in_profile_on(r"C:\Users\x\.cys\claude-2\projects\a\s.jsonl", cfg, true));
        assert!(!session_in_profile_on(r"C:\Users\x\.claude-3\projects\a\s.jsonl", cfg, true));
        assert!(!session_in_profile_on(r"C:\Users\x\.cys\claude\projects", cfg, true), "projects 폴더 자체");
        assert!(!session_in_profile_on(r"C:\Users\x\.cys\claude\projects\..\..\.claude-3\projects\s.jsonl", cfg, true), "..");
        // unix: 홈에 /projects/ 가 있어도 좌석 폴더 기준으로 판정한다(profile_dir_from_session 의 첫 마커 오판 없음)
        assert!(session_in_profile_on("/home/projects/u/.cys/claude/projects/-w/s.jsonl", "/home/projects/u/.cys/claude", false));
        assert!(!session_in_profile_on("/home/projects/u/.claude-3/projects/-w/s.jsonl", "/home/projects/u/.cys/claude", false));
        // unix 는 대소문자·공백을 접지 않는다(다른 디렉터리를 같다고 말하지 않는다)
        assert!(!session_in_profile_on("/Users/x/.CYS/claude/projects/-w/s.jsonl", "/Users/x/.cys/claude", false));
        // 결측은 불일치(없는 값끼리 같다고 말하지 않는다)
        assert!(!session_in_profile_on("", "", false));
        assert!(!session_in_profile_on("/Users/x/.cys/claude/projects/-w/s.jsonl", " ", false));
    }

    /// ★fatal-fix (a): 표기가 달라도 **둘 다 실재하면** 정규화(심볼릭 링크)로 같은 폴더를 알아본다 — 운영 판(`session_in_profile`).
    #[cfg(unix)]
    #[test]
    fn fatal_fix_session_in_profile_follows_symlinks_when_both_exist() {
        let home = tmp("ff-sip");
        write(&home.join("real/.cys/claude/projects/-w/s.jsonl"), "{}\n");
        std::os::unix::fs::symlink(home.join("real"), home.join("link")).unwrap();
        let via_link = home.join("link/.cys/claude/projects/-w/s.jsonl").to_string_lossy().into_owned();
        let cfg = home.join("real/.cys/claude").to_string_lossy().into_owned();
        assert!(session_in_profile(&via_link, &cfg), "심볼릭 링크 표기가 같은 좌석 폴더로 읽히지 않는다");
        let missing = home.join("link/.cys/claude/projects/-w/none.jsonl").to_string_lossy().into_owned();
        assert!(!session_in_profile(&missing, &cfg), "실재하지 않는 transcript 는 정규화 근거가 아니다");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// R4-F3: 부트 시드가 bind 전에 같은 판독을 한다 — FIFO 신원 하나가 부트 체인 전체를 세웠다. 이제 일반 파일만
    /// 열므로 그 프로필만 건너뛰고(신원 불명) 나머지 계정은 그대로 시드된다.
    #[cfg(unix)]
    #[test]
    fn fatal_fix_boot_seed_skips_a_fifo_identity_without_blocking() {
        let home = tmp("ff-fifo-seed");
        write(&home.join(".claude-3/.claude.json"), r#"{"oauthAccount":{"accountUuid":"u-seed-ok","emailAddress":"ok@example.test"}}"#);
        std::fs::create_dir_all(home.join(".claude-z")).unwrap();
        let fifo = home.join(".claude-z/.claude.json");
        mkfifo(&fifo);
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let home = home.clone();
            std::thread::spawn(move || {
                let mut st = AccountsState::default();
                seed_discovered(&mut st, &home);
                let ids: Vec<String> = st.views.keys().map(|k| k.account_id.clone()).collect();
                let _ = tx.send(ids);
            });
        }
        let got = rx.recv_timeout(std::time::Duration::from_secs(3));
        release_fifo(&fifo);
        assert_eq!(got, Ok(vec!["u-seed-ok".to_string()]), "FIFO 신원이 부트 시드를 세웠다(bind 전 정지)");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// R1-F2: 같은 계정을 보는 좌석 둘이 **같은 리셋 창**의 서로 다른 시점 값을 번갈아 보내도(한쪽은 쉬고 있어 낡은
    /// 값) 경보 입력은 그 창의 **최댓값**으로 남는다 — 창 안의 사용률은 줄지 않으므로 낮은 값은 낡은 관측이다.
    /// 종전(최신 승자)은 96↔79 를 오가며 경보 키를 한 틱 비활성으로 떨어뜨려 재무장·재발화(REMIND 우회)했다.
    /// 새 리셋 창(리셋 시각이 창 길이만큼 뒤)의 값은 곧바로 이긴다 · 지난 창의 늦은 보고는 새 창 값을 덮지 않는다.
    #[test]
    fn fatal_fix_alternating_same_window_reports_keep_the_max_and_do_not_refire() {
        let dir = tmp("ff-alt-daemon");
        let home = tmp("ff-alt-home");
        let sess = outside_profile(&home, ".claude-3", "u-alt2", "alt2@example.test");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let cfg = crate::alerts::AlertConfig::default();
        let mut fired: HashMap<String, f64> = HashMap::new();
        let key = "account_rate:alt2@example.test:5h";
        let seq0 = d.bus.latest_seq();
        let t0 = crate::state::now_epoch();
        let r = t0 + 300.0; // 이 5h 창은 5분 뒤 리셋된다
        for i in 0..6 {
            let t = t0 + 23.0 * f64::from(i);
            // 두 좌석이 번갈아 — 96%(바쁜 좌석) · 79%(쉬는 좌석 · 리셋 시각은 몇 초 흔들린다)
            let (pct, jitter) = if i % 2 == 0 { (96.0, 0.0) } else { (79.0, 3.0) };
            assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("5h", pct, Some(r + jitter))], "statusline", t));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t + 1.0);
        }
        assert_eq!(account_alerts(&d, seq0, key), 1, "같은 창의 번갈이 보고가 REMIND 안에 경보를 다시 냈다");
        assert_eq!(alert_rates(&d), vec![("alt2@example.test".to_string(), "5h".to_string(), 96.0)]);
        // 최댓값은 마지막 확인에서 PEAK_HOLD_SECS 까지만 쥔다 — 그 뒤의 낮은 값(제공자가 창 중간에 내린 경우)은 이긴다.
        {
            let mut st = d.accounts.lock().unwrap();
            let rate_now = [rw("5h", 50.0, Some(r))];
            for input in st.alert_inputs.values_mut() {
                input.peak_at.insert("5h".into(), t0 - PEAK_HOLD_SECS - 1.0);
            }
            let key = st.alert_inputs.keys().next().cloned().unwrap();
            let label = st.alert_inputs[&key].label.clone();
            note_alert_input(&mut st, &key, &label, &rate_now, "statusline", t0 + 130.0);
        }
        assert_eq!(alert_rates(&d), vec![("alt2@example.test".to_string(), "5h".to_string(), 50.0)], "쥔 최댓값이 확인 없이 무기한 남았다");
        // 리셋 뒤 새 창(리셋 시각이 창 길이만큼 뒤)의 3% 는 곧바로 이긴다
        let r2 = t0 + 400.0 + 5.0 * 3600.0 - 100.0;
        assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("5h", 3.0, Some(r2))], "statusline", t0 + 400.0));
        assert_eq!(alert_rates(&d), vec![("alt2@example.test".to_string(), "5h".to_string(), 3.0)], "새 리셋 창 값이 이기지 못했다");
        // 지난 창의 늦은 보고(쉬던 좌석이 옛 창의 99% 를 뒤늦게)는 새 창 값을 덮지 않는다
        assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("5h", 99.0, Some(r))], "statusline", t0 + 430.0));
        assert_eq!(alert_rates(&d), vec![("alt2@example.test".to_string(), "5h".to_string(), 3.0)], "지난 창의 늦은 보고가 새 창을 덮었다");
        // 결측형 음성 대조: 리셋 시각이 없는 보고끼리는 종전 그대로 최신 승자(창을 가를 근거가 없다)
        assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("7d", 50.0, None)], "statusline", t0 + 460.0));
        assert!(note_rate_at(&d, Some(&home), "claude", &sess, &[rw("7d", 40.0, None)], "statusline", t0 + 490.0));
        assert_eq!(alert_rates(&d), vec![("alt2@example.test".to_string(), "7d".to_string(), 40.0)]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// R1-F2(agy 판): 여러 agy 좌석이 같은 계정의 다른 시점 쿼터를 번갈아 보내도 `account_rate:Antigravity (agy):5h`
    /// 는 REMIND 안에 한 번만 난다(P4 재현 모양 · agy 는 리셋을 `now+reset_in_seconds` 로 지어 보내 몇 초씩 흔들린다).
    #[test]
    fn fatal_fix_alternating_agy_seats_do_not_refire_the_account_alert() {
        let dir = tmp("ff-agy-alt");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let cfg = crate::alerts::AlertConfig::default();
        let mut fired: HashMap<String, f64> = HashMap::new();
        let key = "account_rate:Antigravity (agy):5h";
        let seq0 = d.bus.latest_seq();
        let t0 = crate::state::now_epoch();
        for i in 0..8 {
            let t = t0 + 23.0 * f64::from(i);
            let pct = if i % 2 == 0 { 96.0 } else { 79.0 };
            note_rate(&d, "gemini", "", &[rw("5h", pct, Some(t + 4000.0 - 23.0 * f64::from(i) + f64::from(i % 3)))], AGY_STATUSLINE_SOURCE, t);
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t + 1.0);
        }
        assert_eq!(account_alerts(&d, seq0, key), 1, "agy 좌석 번갈이가 계정 경보를 REMIND 안에 다시 냈다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R3-2 · ROLE-3: 리셋 시각이 지난 창은 경보 입력이 아니다(UI 의 '리셋됨' 규칙과 같다). 종전에는 유휴 agy 좌석의
    /// '100%' 가 리셋 뒤에도 남아 30분마다 crit 로 다시 울렸다. 리셋 시각이 없는 창은 관측 나이가 창 길이를 넘을 때만
    /// 뺀다(그 창은 리셋됐을 수밖에 없다). 리셋 시각이 epoch 초로 보이지 않으면(단위가 다른 원천) 빼는 근거로 쓰지 않는다.
    #[test]
    fn fatal_fix_windows_past_their_reset_are_not_alert_inputs() {
        let dir = tmp("ff-reset-daemon");
        let home = tmp("ff-reset-home");
        let sess = outside_profile(&home, ".claude-3", "u-rst", "rst@example.test");
        let d = crate::state::Daemon::new(dir.join("cysd.sock"));
        let now = crate::state::now_epoch();
        // 5h 창은 리셋이 지났다 · 7d 창은 아직
        assert!(note_rate_at(
            &d, Some(&home), "claude", &sess,
            &[rw("5h", 100.0, Some(now - 5.0)), rw("7d", 91.0, Some(now + 86400.0))],
            "statusline", now - 60.0,
        ));
        assert_eq!(alert_rates(&d), vec![("rst@example.test".to_string(), "7d".to_string(), 91.0)], "리셋이 지난 창이 경보 입력에 남았다");
        // agy 상태줄 값도 같다(유휴 좌석의 소진 값)
        note_rate(&d, "gemini", "", &[rw("5h", 100.0, Some(now - 1.0))], AGY_STATUSLINE_SOURCE, now - 30.0);
        assert!(
            !alert_rates(&d).iter().any(|(l, _, _)| l == "Antigravity (agy)"),
            "리셋이 지난 agy 창이 경보 입력에 남았다: {:?}", alert_rates(&d)
        );
        // 리셋 시각 없음: 5시간을 넘긴 5h 관측은 뺀다 · 그 안이면 남긴다
        let d2 = crate::state::Daemon::new(dir.join("cysd2.sock"));
        note_rate(&d2, "gemini", "", &[rw("5h", 99.0, None)], AGY_STATUSLINE_SOURCE, now - 6.0 * 3600.0);
        assert!(alert_rates(&d2).is_empty(), "창 길이를 넘긴 리셋 없는 관측이 남았다");
        note_rate(&d2, "gemini", "", &[rw("5h", 99.0, None)], AGY_STATUSLINE_SOURCE, now - 60.0);
        assert_eq!(alert_rates(&d2).len(), 1, "신선한 리셋 없는 관측이 빠졌다(과잉 제거)");
        // epoch 초로 보이지 않는 리셋(상대 초 등)은 빼는 근거가 아니다(지우는 쪽 오판 금지)
        let d3 = crate::state::Daemon::new(dir.join("cysd3.sock"));
        note_rate(&d3, "gemini", "", &[rw("5h", 99.0, Some(1200.0))], AGY_STATUSLINE_SOURCE, now - 60.0);
        assert_eq!(alert_rates(&d3).len(), 1, "단위가 다른 리셋 값으로 경보 입력을 지웠다");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }
}
