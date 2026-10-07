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
//!
//! ★0.14.43(B3 · 한도 경보 신선도 규칙 — 오너 결재): 경보 입력 = `rate_window_live` ∧ (사용 중 ∨ 관측 나이 ≤ 1800초). 로그인을 바꾼 뒤에도 옛
//! 계정의 리셋 전 값(99%)이 30분마다 CSO 로 가던 결함의 처방이다. 판정 불가 = 사용 중(실패 방향 = 경보 유지).
//! 좌석 신원 표([`seat_identity_view`])가 그 '사용 중'의 재료다 — **락 간선(새 간선 없음 · 겹쳐 쥐지 않는다)**:
//!   ① `surfaces` 락(좌석 표만 복사 · 파일 IO 없음) → **해제** → ② 폴더별 신원 판독(어떤 락도 쥐지 않은 채 · 60초 하한 캐시 `Daemon::seat_ident_cache` 는
//!   조회·기록 때만 순간 잡는다 · 캐시 락 안에서 다른 락을 잡지 않는다 · 실판독 `claude_identity_unlocked` 는 종전 규율대로 accounts 락을 순간만) →
//!   ③ `accounts` 락(메모리 연산뿐 — `alert_rates_with`·`local_json` 행 조립). surfaces 와 accounts 를 동시에 쥐는 곳은 없다.
//!
//! ★0.14.43(B1 · 현재 로그인 폴더·별명 — 오너 결정): `usage.accounts` 행에 가산 키 둘 — `current_profiles`(그 계정이 **지금** 로그인된 설정 폴더 · `profiles` 는 추가 전용이라 로그인을 바꾼 뒤에도 옛 계정에
//! 폴더가 남는다) · `alias`(`~/.cys/accounts.json` 의 `aliases` 객체 — **표시 전용**: 경보 키·라벨·이벤트·`alert_inputs` 에 쓰지 않는다). **락 간선은 새로 만들지 않는다** — `local_json` 은 accounts 락을 잡기
//! **전에** ① 알려진 프로필 폴더의 현재 신원 표([`known_profile_identities`] — 열거 정본 `cys::profile_gate::enumerate_profile_dirs` + B3 의 폴더별 60초 캐시 [`folder_identity`] 재사용 · 새 캐시 없음) ② 별명 표 갱신
//! ([`refresh_aliases`] — 60초 하한 · 파일 mtime 이 바뀔 때만 판독 · accounts 락은 하한 판정·저장 때만 순간) 을 끝내고, 락 안에서는 메모리 연산뿐이다(소스 핀 `b1_lock_order_wiring_pins`).
//!
//! ★0.14.43(R1F-US · 성찰 1회차 수정): **워치독 스레드는 신원 파일을 열지 않는다**(오너 절대 기준 ③ — 0.14.42 의 경보 점검은 이 경로에서 파일을 열지 않았다). 경보 틱은 **캐시 전용 조회**
//! ([`seat_identity_view_cached`] — 항목이 있으면 **만료됐어도 마지막 값**, 없으면 None = 판정 불가 = 사용 중 · IO 0 · stat 0)만 쓴다. 캐시(`Daemon::seat_ident_cache`)는 ① 상태줄 보고(`usage.report` 의 귀속 경로가 이미 읽은
//! 신원을 그대로 싣는다 — 추가 IO 0 · [`note_seat_identity`]) ② RPC 경로의 읽기-통과 조회([`folder_identity`] — 사용량·status·Control Center 호출)가 채운다. 읽기-통과는 캐시 미스일 때 IO **전에** `(now, 옛 값)` 으로
//! 자리를 찍어(선점) 디스크가 멈춰도 폴더당 60초에 요청 하나만 묶인다. 알려진 프로필 폴더 열거(홈·`~/.cys` 의 `read_dir`)도 같은 캐시에 60초 하한·같은 선점 꼴로 둔다([`enumerated_profile_dirs`]).
//! 신원 파일은 있는데(메타 성공) 이번 판독·파싱만 실패하면 직전 신원을 **한 주기만** 더 쓰고(연속 실패면 '읽지 못함'), `current_profiles` 는 이번에 신원을 **읽지 못한**(판독 실패가 유예 뒤까지 이어졌거나 처음 보는 폴더를 다른 요청이 아직
//! 읽는 중인 — [`FolderWho::Unread`]) 폴더가 낀 행에서는 키를 내지 않는다(화면은 `profiles` 폴백). 로그아웃(`oauthAccount` 없음)·신원 파일 없음처럼 신원 없음이 **확정**인 폴더([`FolderWho::NoLogin`])는 읽지 못한 것이 아니라 '이 계정의 폴더가 아니다'로
//! 세므로 그 계정의 다른 폴더가 없으면 `[]`(= 화면의 '이전 로그인')이 나온다(9650334f 와 같다).
//! 대가: 창이 닫혀 있고 좌석이 전부 유휴면(보고도 조회도 없으면) 로그인 전환 감지가 다음 보고·조회까지 늦는다 — 그동안은 마지막 값/판정 불가(= 사용 중)라 경보는 유지된다(0.14.42 와 같은 방향).

use crate::state::{Daemon, HideConsole};
use crate::usage::{ObservedUsage, RateWindow};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
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
    /// ★0.14.43(B1): 별명 표(표시 전용 · `~/.cys/accounts.json` 의 `aliases`) — [`refresh_aliases`] 가 락 밖에서 판독한 결과를 짧게 싣는다. 경보 입력과 무관하다.
    alias: AliasState,
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

/// ★R1F-US(m-2) 신원 파일 판독 결과 — 3값. '신원 없음(확정)'과 '이번엔 못 읽음(일시 실패일 수 있다)'을 가른다 — [`folder_identity`] 가 직전 신원을 한 주기 더 쓸지 이것으로 정한다.
enum IdentRead {
    /// `oauthAccount.accountUuid` 를 얻었다.
    Found(Ident),
    /// 신원 없음이 **확정**이다 — 신원 파일이 없거나 일반 파일이 아니고(메타 실패 · FIFO·장치·디렉터리 포함), 또는 파일은 읽혔고 JSON 인데 로그인 정보(`oauthAccount`·`accountUuid`)가 없다(로그아웃 등).
    NoIdentity,
    /// 신원 파일은 있는데(메타 성공) 이번 판독·파싱이 실패했다 — 열기·읽기 오류 · 열고 보니 일반 파일이 아님·크기 초과 · UTF-8 아님 · JSON 파싱 실패(쓰는 도중의 빈 파일·잘린 파일 포함).
    Unreadable,
}

/// 신원 파일 판독·파싱(3값) — **락 밖 전용**. 여는 것도 막히지 않게 연다(unix `O_NONBLOCK` — 일반 파일 읽기에는 영향이 없다)고, 연 뒤 fstat 으로 **일반 파일**인지 다시 확인한다(stat 과 open 사이에 FIFO 로
/// 바뀌는 경쟁 차단). 크기 상한을 넘으면 읽지 않는다. 자격증명(.credentials.json)은 읽지 않는다.
fn read_identity_outcome(f: &Path) -> IdentRead {
    use std::io::Read;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NONBLOCK);
    }
    let Ok(file) = opts.open(f) else {
        return IdentRead::Unreadable;
    };
    let Ok(md) = file.metadata() else {
        return IdentRead::Unreadable;
    };
    if !md.is_file() || md.len() > IDENTITY_FILE_MAX_BYTES {
        return IdentRead::Unreadable;
    }
    let mut s = String::new();
    if file.take(IDENTITY_FILE_MAX_BYTES + 1).read_to_string(&mut s).is_err() || s.len() as u64 > IDENTITY_FILE_MAX_BYTES {
        return IdentRead::Unreadable;
    }
    parse_identity_outcome(&s)
}

/// 신원 파일 판독·파싱 — [`read_identity_outcome`] 의 `Option` 판(종전 계약 그대로: 읽기 실패·신원 없음 모두 None).
fn read_identity_file(f: &Path) -> Option<Ident> {
    match read_identity_outcome(f) {
        IdentRead::Found(ident) => Some(ident),
        IdentRead::NoIdentity | IdentRead::Unreadable => None,
    }
}

/// `.claude.json` 본문 → 신원(순수 · 3값). JSON 으로 파싱되지 않으면(빈 문자열·잘린 본문 포함) `Unreadable`, JSON 인데 `oauthAccount.accountUuid`(문자열)가 없으면 `NoIdentity`.
fn parse_identity_outcome(s: &str) -> IdentRead {
    let Ok(v) = serde_json::from_str::<Value>(s) else {
        return IdentRead::Unreadable;
    };
    let Some(oa) = v.get("oauthAccount") else {
        return IdentRead::NoIdentity;
    };
    let Some(uuid) = oa.get("accountUuid").and_then(|x| x.as_str()).map(str::to_string) else {
        return IdentRead::NoIdentity;
    };
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
    IdentRead::Found((uuid, email, plan))
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
/// ★R1F-US(m-2): 3값판([`IdentRead`]). 신원 파일의 메타가 안 잡히면(없음·FIFO 등 일반 파일 아님) `NoIdentity`. mtime 캐시에는 `Found`·`NoIdentity` 만 싣는다 — `Unreadable`(일시 실패일 수 있다)은 싣지 않아
/// 다음 확인에 다시 읽는다.
fn claude_identity_probe(accounts: &std::sync::Mutex<AccountsState>, home: Option<&Path>, dir: &Path) -> IdentRead {
    let Some((f, mtime)) = identity_file_meta(home, dir) else {
        return IdentRead::NoIdentity;
    };
    let hit = {
        let st = accounts.lock().unwrap();
        ident_cached(&st, dir, &f, mtime)
    };
    match hit {
        Some(Some(ident)) => return IdentRead::Found(ident),
        Some(None) => return IdentRead::NoIdentity,
        None => {}
    }
    let read = read_identity_outcome(&f);
    match &read {
        IdentRead::Found(ident) => ident_store(&mut accounts.lock().unwrap(), dir, f, mtime, Some(ident.clone())),
        IdentRead::NoIdentity => ident_store(&mut accounts.lock().unwrap(), dir, f, mtime, None),
        IdentRead::Unreadable => {}
    }
    read
}

/// [`claude_identity_probe`] 의 `Option` 판(종전 계약 그대로 — 귀속 경로가 쓴다: 신원이 없거나 못 읽으면 None).
fn claude_identity_unlocked(accounts: &std::sync::Mutex<AccountsState>, home: Option<&Path>, dir: &Path) -> Option<Ident> {
    match claude_identity_probe(accounts, home, dir) {
        IdentRead::Found(ident) => Some(ident),
        IdentRead::NoIdentity | IdentRead::Unreadable => None,
    }
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

/// ★0.14.43(B3): [`note_rate`] 의 **귀속 계정 반환판** — 동작은 같고, 귀속된 account_id 를 돌려준다(None = rate 가 비었거나 신원 불명 —
/// 아무것도 쓰지 않았다). 좌석 `ObservedUsage::rate_account`(로그인 전환 판정의 입력)가 이 값을 쓴다.
pub fn note_rate_resolved(
    daemon: &Arc<Daemon>,
    agent: &str,
    session_file: &str,
    rate: &[RateWindow],
    source: &str,
    now: f64,
) -> Option<String> {
    note_rate_at_resolved(daemon, dirs::home_dir().as_deref(), agent, session_file, rate, source, now)
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
    note_rate_at_resolved(daemon, home, agent, session_file, rate, source, now).is_some()
}

/// [`note_rate_at`] 의 본체 — 귀속된 account_id 를 돌려준다(None = 미귀속).
fn note_rate_at_resolved(
    daemon: &Arc<Daemon>,
    home: Option<&Path>,
    agent: &str,
    session_file: &str,
    rate: &[RateWindow],
    source: &str,
    now: f64,
) -> Option<String> {
    if rate.is_empty() {
        return None;
    }
    let resolved = match agent {
        "claude" => profile_dir_from_session(session_file).and_then(|dir| {
            claude_identity_unlocked(&daemon.accounts, home, &dir).map(|ident| {
                // ★R1F-US(M-1): 이미 읽은 신원을 좌석 신원 캐시에 싣는다(추가 IO 0). 창 밖 보고는 싣지 않는다 — 좌석 폴더의 사실이 아니다.
                if feeds_alerts(source) {
                    note_seat_identity(daemon, &dir, &ident.0, now);
                }
                claude_resolution(home, &dir, ident)
            })
        }),
        other => fixed_resolution(home, other),
    };
    let Some(resolved) = resolved else {
        return None; // 미귀속(신원 불명) — 유령 계정을 만들지 않는다
    };
    let account_id = resolved.0.account_id.clone();
    note_resolved(daemon, resolved, rate, source, now);
    Some(account_id)
}

/// ★fatal-fix (a): claude 좌석 보고를 **좌석의 설정 폴더**(데몬이 그 좌석에 넣어 준 `CLAUDE_CONFIG_DIR`)로 귀속한다 —
/// 호출자가 댄 transcript 경로로는 신원 파일을 고르지 않는다(호출자 경로로 파일시스템을 건드리지 않는다 · R4-F2 ③).
/// 대조([`session_in_profile`])는 부른 쪽(handlers)이 먼저 끝낸다.
#[cfg_attr(not(test), allow(dead_code))] // 운영 호출처는 귀속 계정 반환판(`_resolved`)으로 옮겼다 — bool 판은 종전 계약을 보존한다
pub fn note_rate_for_profile(daemon: &Arc<Daemon>, profile_dir: &Path, rate: &[RateWindow], source: &str, now: f64) -> bool {
    note_rate_for_profile_at(daemon, dirs::home_dir().as_deref(), profile_dir, rate, source, now)
}

/// ★0.14.43(B3): [`note_rate_for_profile`] 의 **귀속 계정 반환판** — 좌석 설정 폴더의 **보고 시점 신원**으로 귀속한 account_id 를 돌려준다
/// (None = rate 가 비었거나 신원 판독 실패 — 아무것도 쓰지 않았다). 호출자(`usage.report`)가 좌석 `rate_account` 에 싣는다.
pub fn note_rate_for_profile_resolved(
    daemon: &Arc<Daemon>,
    profile_dir: &Path,
    rate: &[RateWindow],
    source: &str,
    now: f64,
) -> Option<String> {
    note_rate_for_profile_at_resolved(daemon, dirs::home_dir().as_deref(), profile_dir, rate, source, now)
}

/// 홈을 인자로 받는 시험 이음매.
#[cfg_attr(not(test), allow(dead_code))]
fn note_rate_for_profile_at(
    daemon: &Arc<Daemon>,
    home: Option<&Path>,
    profile_dir: &Path,
    rate: &[RateWindow],
    source: &str,
    now: f64,
) -> bool {
    note_rate_for_profile_at_resolved(daemon, home, profile_dir, rate, source, now).is_some()
}

/// [`note_rate_for_profile_at`] 의 본체 — 귀속된 account_id 를 돌려준다(None = 미귀속).
fn note_rate_for_profile_at_resolved(
    daemon: &Arc<Daemon>,
    home: Option<&Path>,
    profile_dir: &Path,
    rate: &[RateWindow],
    source: &str,
    now: f64,
) -> Option<String> {
    if rate.is_empty() {
        return None;
    }
    let ident = claude_identity_unlocked(&daemon.accounts, home, profile_dir)?;
    // ★R1F-US(M-1·②): 보고가 좌석 신원 캐시를 채운다 — 방금 읽은 신원을 그대로 싣는다(추가 IO 0). 워치독은 파일을 읽지 않고 이 캐시를 본다.
    if feeds_alerts(source) {
        note_seat_identity(daemon, profile_dir, &ident.0, now);
    }
    let resolved = claude_resolution(home, profile_dir, ident);
    let account_id = resolved.0.account_id.clone();
    note_resolved(daemon, resolved, rate, source, now);
    Some(account_id)
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
    if let Some(home) = account_home() {
        // ★fatal-fix R4-F3: 발견(파일 IO)은 락 밖에서 끝내고, 락 안에서는 메모리에 싣기만 한다. 이 함수는 소켓 bind
        //   **전에** 동기로 돈다 — 종전에는 신원 파일 하나가 막히면(FIFO 등) 락을 쥔 채 부트 체인 전체가 섰다.
        let found = discover_at(&home);
        {
            let mut st = daemon.accounts.lock().unwrap();
            apply_discovered(&mut st, &home, found);
        }
        // ★0.14.43(B1): 별명 표 부트 판독 1회(파일 IO 는 락 밖 · 실패·비정규 파일은 별명 없음) — 이후는 `local_json` 이 파일 mtime 이 바뀔 때만(60초 하한) 다시 읽는다. 선언 계정 파싱·적재는 아래 종전 그대로.
        refresh_aliases(daemon, Some(&home), crate::state::now_epoch());
        // 선언 계정(~/.cys/accounts.json — pack 밖: pack 스윕/치유 사정권 회피)
        // ★0.14.43(B1b): 이 판독은 소켓 bind **전**에 동기로 돈다 — 종전의 무제한 문자열 판독은 이 경로가 FIFO 면 열기에서 영원히 막혀 부트 체인 전체가 섰다. 안전 판독(일반 파일만 ·
        //   1MiB 상한)으로 바꿨고, 실패(부재·FIFO·디렉터리·초과·읽기 오류·UTF-8 아님)는 종전의 `Err` 와 같게 '선언 계정 없음'이다. 아래 파싱·적재는 무변경(같은 JSON → 같은 뷰).
        let decl = home.join(".cys/accounts.json");
        if let Some(s) = read_declared_accounts_text(&decl) {
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
    let Some(home) = account_home() else { return };
    let decl = home.join(".cys/accounts.json");
    // ★0.14.43(B1b): `seed_known` 바로 다음의 부트 단계(소켓 bind 전 · 동기) — 같은 파일이라 같은 안전 판독을 쓴다(FIFO 면 seed_known 을 고쳐도 여기서 부트가 선다). 실패는 종전처럼 어댑터 없음.
    let Some(s) = read_declared_accounts_text(&decl) else { return };
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
/// ★(R4-01) `raw` 가 `root` 의 **진하위** 경로인가(어휘적 · 파일시스템 무접촉). Windows 는 대소문자·구분자·`\\?\` 접두를
/// 가리지 않는다(훅이 넘기는 드라이브 문자 대소문자가 열거 경로와 달라도 같은 폴더다 — 판정이 좁아지면 정상 보고가 거절된다).
fn path_strictly_under(raw: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let norm = |p: &Path| {
            let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
            s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
        };
        let r = norm(root);
        let r = r.trim_end_matches('\\');
        let x = norm(raw);
        x.len() > r.len() + 1 && x.starts_with(r) && x.as_bytes()[r.len()] == b'\\'
    }
    #[cfg(not(windows))]
    {
        raw.starts_with(root) && raw != root
    }
}

fn report_outside_at(
    daemon: &Arc<Daemon>,
    home: Option<&Path>,
    session_file: &str,
    rate: &[RateWindow],
    now: f64,
) -> Result<OutsideOutcome, &'static str> {
    let home = home.ok_or("home_unknown")?;
    // ⓪ ★(0.14.42 · R4-01) **파일시스템을 건드리기 전에** 문자열로 먼저 거른다 — 호출자가 준 경로가 알려진 프로필 dir 의
    //   `projects/` 아래(어휘적 · `..` 없음)가 아니면 거절한다. 종전 첫 동작이 호출자 경로의 `canonicalize` 라, 무인증 RPC 가
    //   `/net/<host>/…`(autofs NFS 마운트)·응답 없는 SMB/NFS 경로를 주면 핸들러가 stat 에서 무기한 멈췄다 — 그런 핸들러
    //   128개가 입장 게이트 허가를 쥐면 ping 을 뺀 전 RPC(GUI 키 입력·훅·CLI)가 영구 대기한다(④ 관측 사각).
    //   프로필 dir 의 정규화(홈 아래 로컬 경로)는 호출자 경로와 무관하므로 후보에 함께 넣는다(심볼릭 링크 홈 호환).
    let raw = Path::new(session_file);
    if !raw.is_absolute() || raw.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("session_file_outside_profiles");
    }
    let lexically_inside = cys::profile_gate::enumerate_profile_dirs(home).into_iter().any(|d| {
        let mut roots = vec![d.join("projects")];
        if let Ok(cd) = std::fs::canonicalize(&d) {
            roots.push(cd.join("projects"));
        }
        roots.iter().any(|r| path_strictly_under(raw, r))
    });
    if !lexically_inside {
        return Err("session_file_outside_profiles");
    }
    // ① 실재 — 어휘 검사를 지난 경로만(프로필 dir 아래라 로컬 홈이다).
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
/// ★0.14.43(B3): 행마다 가산 키 3개 — `in_use`(true/false/null · [`account_in_use`]) · `rate_observed_at`(경보 입력의 관측 시각 — 없으면 null) ·
/// `rate[]` 원소의 `alert_eligible`(그 창이 경보 입력으로 적격인가 — 경보 입력이 없는 창(창 밖 표시용 값 등)은 false). 표시 승자·기존 키는 불변이다.
/// ★0.14.43(B1): 행마다 가산 키 2개 — `current_profiles`(배열 · [`current_profiles_for`]: claude 는 **지금** 이 계정으로 로그인된 알려진 프로필 폴더 · 그 밖은 `profiles` 그대로 · 구 데몬 응답엔 키가 없다) ·
/// `alias`(문자열|null · [`alias_for`] — 표시 전용). `profiles` 는 종전처럼 추가 전용이다.
/// ★R1F-US(m-1·m-2): `current_profiles` 의 폴더 표는 열거(60초 하한 캐시)에 **이미 신원을 읽은 좌석 폴더**를 합친 것이고(열거 밖 폴더를 쓰는 계정이 `in_use:true` 인데 `current_profiles:[]` 가 되지 않게 · 추가 IO 0),
/// 그 행의 `profiles` 에 든 폴더 가운데 이번에 신원을 **읽지 못한** 것(판독 실패가 유예 뒤까지 이어짐 · 처음 보는 폴더를 아직 읽는 중)이 있거나 열거가 실패하면 이 키를 **내지 않는다**(화면은 키 부재 → `profiles` 폴백 · 추가 전용 계약).
/// 로그아웃·신원 파일 없음(신원 없음 **확정**)은 읽지 못한 것이 아니다 — '이 계정의 폴더가 아니다'로 세어 그 계정의 다른 폴더가 없으면 `[]` 이다(보정).
/// 좌석 신원 표·알려진 프로필 폴더 신원 표·별명 표 갱신(전부 파일 IO)은 **accounts 락을 잡기 전에** 만든다.
pub fn local_json(daemon: &Arc<Daemon>, now: f64) -> Value {
    let view = seat_identity_view_at(daemon, now);
    let stale_secs = account_alert_stale_secs();
    let home = account_home();
    let seat_states = seat_folder_states(daemon, &view);
    let known = known_profile_identities(daemon, home.as_deref(), now).map(|k| merge_seat_folders(k, home.as_deref(), &seat_states));
    refresh_aliases(daemon, home.as_deref(), now);
    let mut rows: Vec<Value> = {
        let st = daemon.accounts.lock().unwrap();
        let mut views: Vec<&AccountView> = st.views.values().collect();
        views.sort_by(|a, b| a.key.cmp(&b.key));
        views
            .into_iter()
            .map(|v| {
                let in_use = account_in_use(&v.key.provider, &v.key.account_id, &view);
                let input = st.alert_inputs.get(&v.key).filter(|i| i.at > 0.0);
                let age = input.map_or(0.0, |i| (now - i.at).max(0.0));
                let rate: Vec<Value> = v
                    .rate
                    .iter()
                    .map(|w| {
                        // 같은 창 라벨의 **경보 입력**이 적격인가 — 경보 입력이 없으면(창 밖 표시용 값 · 관측 전) 경보 입력이 아니다.
                        let eligible = input.is_some_and(|i| {
                            i.rate.iter().find(|iw| iw.label == w.label).is_some_and(|iw| {
                                alert_eligible(crate::usage::rate_window_live(iw, i.at, now), in_use, age, stale_secs)
                            })
                        });
                        let mut o = serde_json::to_value(w).unwrap_or(Value::Null);
                        if let Some(m) = o.as_object_mut() {
                            m.insert("alert_eligible".into(), json!(eligible));
                        }
                        o
                    })
                    .collect();
                let mut row = json!({
                    "provider": v.key.provider,
                    "account_id": v.key.account_id,
                    "label": v.label,
                    "plan": v.plan,
                    "profiles": v.profiles.iter().collect::<Vec<_>>(),
                    "rate": rate,
                    "updated_at": if v.updated_at > 0.0 { json!(v.updated_at) } else { Value::Null },
                    "stale_secs": if v.updated_at > 0.0 { json!((now - v.updated_at).max(0.0)) } else { Value::Null },
                    "source": v.source,
                    "adapter": v.adapter,
                    // null = 경로 고장 없음(관측 전이거나 정상). 값 = 그 경로가 지금 고장(예: agy_http_403).
                    "source_error": v.source_error,
                    // ★0.14.43(B3) 가산 키 — true/false/null(판정 불가) · 경보 입력의 관측 시각(없으면 null).
                    "in_use": in_use,
                    "rate_observed_at": input.map_or(Value::Null, |i| json!(i.at)),
                    // ★0.14.43(B1) 가산 키 — 별명(표시 전용 · 없으면 null). 이 계정이 지금 로그인된 설정 폴더 `current_profiles`(표기는 `profiles` 와 같은 `profile_short`)는 아래에서 조건부로 싣는다.
                    "alias": alias_for(&st.alias.table, &v.key.account_id, &v.label),
                });
                // ★R1F-US(m-2·ⓑ): 신원을 읽지 못한 폴더가 낀 행 · 열거 실패는 키 부재 — '이전 로그인'을 단정하지 않는다(화면은 `profiles` 폴백).
                if let Some(cp) = current_profiles_for(&v.key.provider, &v.key.account_id, &v.profiles, known.as_deref()) {
                    row["current_profiles"] = json!(cp);
                }
                row
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

// ───────────────── ★0.14.43(B3) 경보 신선도 규칙 — 좌석 신원 표 · 순수 판정 · 계정 축 경보 입력 ─────────────────

/// 노브 `CYS_ACCOUNT_ALERT_STALE_SECS` 의 기본값(초) — 경보 입력 적격의 '관측 나이' 상한(오너 결재 1800 = 경보 리마인드 간격과 같다).
pub const ACCOUNT_ALERT_STALE_SECS_DEFAULT: f64 = 1800.0;
/// 좌석 설정 폴더 신원의 재판독 하한(초) — 같은 폴더는 이 안에 다시 stat 하지 않는다(status 폴링이 파일 접근을 늘리지 않게).
pub const SEAT_IDENT_CACHE_SECS: f64 = 60.0;
/// 신원 캐시에서 확인한 지 이만큼(초) 지난 폴더 항목은 신원 표를 만들 때 걷는다(크기 유계) · 항목 수가 [`SEAT_IDENT_CACHE_MAX`] 를 넘으면 비운다(병적 입력 상한 — 정상은 폴더 수십 개).
const SEAT_IDENT_PRUNE_SECS: f64 = 600.0;
const SEAT_IDENT_CACHE_MAX: usize = 512;

/// 노브 값 파서(순수 · 검체 대상). 미설정·빈 값·파싱 실패·음수·비유한 = 기본값(규칙은 켜진 채). `0` = 이 규칙 끔(0.14.42 동작에 **가깝지만 같지는 않다** — 좌석별 경보에서 리셋 시각이 없는 창의 생사를 갱신 시각이 아니라
/// 값을 실제로 관측한 시각으로 따지고, `usage.alert_resolved` 의 `cleared` 가 난다 · `USER-MANUAL.md` 노브 표·릴리스 노트 §4 · R2F-DM 성찰 2회차 A5 m11).
pub fn parse_stale_secs(raw: Option<&str>) -> f64 {
    match raw.map(str::trim) {
        None | Some("") => ACCOUNT_ALERT_STALE_SECS_DEFAULT,
        Some(v) => match v.parse::<f64>() {
            Ok(n) if n.is_finite() && n >= 0.0 => n,
            _ => ACCOUNT_ALERT_STALE_SECS_DEFAULT,
        },
    }
}

/// `CYS_ACCOUNT_ALERT_STALE_SECS` 를 읽는 **유일한 자리**(env 래퍼 — 판정은 [`parse_stale_secs`]).
pub fn account_alert_stale_secs() -> f64 {
    parse_stale_secs(std::env::var("CYS_ACCOUNT_ALERT_STALE_SECS").ok().as_deref())
}

/// 살아 있는(종료되지 않은) 좌석에 에이전트가 있는가.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentsAlive {
    pub claude: bool,
    pub codex: bool,
    /// agy(Antigravity) — 좌석의 agent 이름은 `gemini`.
    pub gemini: bool,
}

/// 좌석 한 개의 신원 항목.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatIdent {
    pub surface_id: u64,
    /// 좌석 에이전트 — `agent_meta` 이름, 없으면 관측 스냅샷의 agent. None = 모름(에이전트 증거 없는 셸 등).
    pub agent: Option<String>,
    /// 신원을 읽은 설정 폴더(claude 좌석만 · None = 폴더 미상 **또는** 검증되지 않은 기록뿐 — 아래 `unverified_dir`).
    pub folder: Option<String>,
    /// ★(0.14.45 · 성찰 M5) 검증되지 않은 기록 폴더(claude 좌석만 · [`SeatFolder::Unverified`]) — 호출자가 계정 폴더를 갈아끼웠거나 복원 좌석인데 transcript 가
    /// 아직 관측되지 않았다. 이 폴더의 신원을 읽지 않는다(계정 불일치·확인 필요 = `account.state:"mismatch"`). `folder` 가 Some 이면 늘 None.
    pub unverified_dir: Option<String>,
    /// 그 폴더의 **현재** 신원 account_id(None = 폴더 미상이거나 신원 판독 실패).
    pub current_account: Option<String>,
    /// 신원을 판독해 account_id 를 얻었는가(= `current_account.is_some()`).
    pub known: bool,
}

/// 좌석 신원 표 — 경보·status·계정 조회가 '사용 중'을 가르는 재료. 파일 IO 로 만든 값을 락 밖에서 한 번 만들어 여럿이 나눠 쓴다.
/// `Default`(= `collect_ok:false`)는 '수집 실패' — 모든 판정이 '모름'(실패 방향 = 경보 유지)이다.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeatIdentityView {
    /// 살아 있는 claude 좌석들이 쓰는 설정 폴더(중복 제거 · 폴더 오름차순) → 그 폴더의 현재 신원(None = 판독 실패).
    pub claude_folders: Vec<(String, Option<String>)>,
    /// 살아 있는 claude 좌석 중 설정 폴더를 알 수 없는 좌석이 있다(`claude_config_dir` 도 transcript 경로도 없다).
    pub claude_folder_unknown: bool,
    pub agents_alive: AgentsAlive,
    /// 좌석 목록 수집이 성공했는가(false = 락 오염 등 — 전부 '모름').
    pub collect_ok: bool,
    pub seats: Vec<SeatIdent>,
}

impl SeatIdentityView {
    /// 좌석의 현재 폴더 신원 — None = 좌석이 표에 없거나(종료·그사이 생성) 폴더 미상이거나 판독 실패(전부 '모름').
    pub fn current_for(&self, surface_id: u64) -> Option<&str> {
        self.seats
            .iter()
            .find(|s| s.surface_id == surface_id)
            .and_then(|s| s.current_account.as_deref())
    }
}

/// ★R1F-US(보정 · m-2 ⓑ) 폴더 신원의 **3값** — `current_profiles` 가 '이 계정의 폴더가 아니다'(신원 없음 **확정**)와 '이번에 읽지 못했다'를 가른다. 워치독(`peek_folder_ident`)은 `Known` 만 신원으로 본다(나머지는 None = 판정 불가).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FolderWho {
    /// 현재 신원(account_id)을 얻었다 — 판독이 한 번 실패해 직전 신원을 한 주기 더 쓰는 유예 중인 것도 여기다.
    Known(String),
    /// **신원 없음이 확정**이다 — 신원 파일이 없거나 일반 파일이 아니고(FIFO·장치·디렉터리 포함), 또는 파일은 읽혔는데 로그인 정보(`oauthAccount`·`accountUuid`)가 없다(로그아웃 등). 어느 계정의 폴더도 아니다.
    NoLogin,
    /// **이번에 읽지 못했다** — 판독 실패가 유예(한 주기) 뒤까지 이어졌거나, 처음 보는 폴더를 다른 요청이 아직 읽는 중이다(선점 자리 = '아직 모름' — 신원 없음 확정이 아니다). 누구의 폴더인지 모른다.
    Unread,
}

impl FolderWho {
    /// 현재 신원(account_id) — `Known` 일 때만.
    fn account(&self) -> Option<&str> {
        match self {
            FolderWho::Known(a) => Some(a.as_str()),
            FolderWho::NoLogin | FolderWho::Unread => None,
        }
    }
}

/// 좌석 신원 캐시 한 항목 — 마지막 확인 시각 · 그때의 신원 상태([`FolderWho`]) · 연속 일시 실패 수(0 = 직전 확인이 정상(또는 신원 없음 확정) · 1 = 판독 실패로 직전 신원을 한 주기 더 쓰는 중 · 2 이상 = 유예 뒤에도 못 읽음).
/// 선점 자리(IO 전에 찍는 항목)는 옛 항목의 상태·실패 수를 그대로 싣고, 옛 항목이 없던(처음 보는) 폴더의 자리는 `Unread`(아직 모름)다.
#[derive(Clone, Debug)]
struct IdentSlot {
    at: f64,
    state: FolderWho,
    misses: u8,
}

/// 좌석 신원의 60초 하한 캐시 — 폴더 → (확인 시각, 그때의 신원 상태 `FolderWho`). `Daemon::seat_ident_cache`. 신원 없음·읽지 못함도 담는다(없는 파일을 매번 stat 하지 않는다).
/// 크기: 신원 표를 만들 때 확인한 지 10분이 지난 항목을 걷고(상한 512 초과는 비운다) · 어느 소비자가 물은 폴더든 60초 하한 동안은 살아 있다.
/// ★R1F-US: 이 캐시는 세 길로 채워진다 — 읽기-통과 조회([`folder_identity`] · 미스 때 IO 전에 선점) · 상태줄 보고([`note_seat_identity`] · 귀속 경로가 읽은 신원을 그대로 · 추가 IO 0) ·
/// 알려진 프로필 폴더 열거([`enumerated_profile_dirs`] · 같은 60초 하한·선점). **워치독은 읽기만 한다**([`peek_folder_ident`] · 만료돼도 마지막 값 · IO 0 · stat 0).
#[derive(Default)]
pub struct SeatIdentCache {
    entries: HashMap<PathBuf, IdentSlot>,
    /// ★R1F-US(m-5) 알려진 프로필 폴더 열거 결과 — (홈, 확인 시각, 목록 | None = 열거 실패). 홈이 다르면 미스다.
    enumerated: Option<(PathBuf, f64, Option<Vec<PathBuf>>)>,
    /// 계측(검체·진단) — 캐시 적중 수 / 실제로 신원을 확인한 수(stat 1회 이상) / 프로필 폴더 열거를 실제로 한 수.
    pub hits: u64,
    pub reads: u64,
    pub enum_reads: u64,
}

#[cfg(test)]
impl SeatIdentCache {
    /// 시험 이음매 — 캐시를 비운다(60초 하한을 기다리지 않고 '신원이 바뀐 뒤'를 관측하게). 프로필 폴더 열거 결과도 함께 비운다.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.enumerated = None;
    }
}

/// 좌석의 에이전트 이름이 claude 인가.
fn agent_is_claude(agent: Option<&str>) -> bool {
    agent == Some("claude")
}

/// ★(0.14.45 · 성찰 M5) 좌석 설정 폴더의 **3값** — 기록된 폴더를 그대로 믿지 않는다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SeatFolder {
    /// 신원을 읽어도 되는 폴더 — 데몬이 스스로 해소한 기록(`config_dir_trusted`)이거나, 관측된 transcript 가 **실제로** 그 폴더 아래다.
    Verified(String),
    /// 기록은 있으나 **검증할 수 없다** — 호출자 env 가 계정 폴더를 갈아끼웠거나(`config_dir_trusted=false` · 복원 좌석 · 수동 `CLAUDE_CONFIG_DIR`)
    /// 아직 transcript 가 관측되지 않았다. 그 폴더의 신원을 이 좌석의 계정으로 **말하지 않는다**(값 = 기록된 폴더 · 표시 근거일 뿐).
    Unverified(String),
    /// 폴더 미상 — 기록도 관측도 없다.
    Unknown,
}

impl SeatFolder {
    /// 신원을 읽어도 되는 폴더만.
    fn verified(&self) -> Option<&str> {
        match self {
            SeatFolder::Verified(f) => Some(f.as_str()),
            SeatFolder::Unverified(_) | SeatFolder::Unknown => None,
        }
    }
    /// 검증되지 않은 기록 폴더만.
    fn unverified(&self) -> Option<&str> {
        match self {
            SeatFolder::Unverified(f) => Some(f.as_str()),
            SeatFolder::Verified(_) | SeatFolder::Unknown => None,
        }
    }
}

/// 두 폴더 경로가 같은 곳을 가리키는가(순수 · 문자열 비교) — 구분자(`\`→`/`)·끝 구분자를 접고, 윈도우에서는 ASCII 대소문자를 무시한다.
/// transcript 경로([`profile_dir_from_session`])는 정슬래시로 나오고 기록(`CLAUDE_CONFIG_DIR`)은 OS 표기라 그대로 비교하면 늘 다르다.
pub(crate) fn same_dir(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.trim().replace('\\', "/").trim_end_matches('/').to_string();
    let (a, b) = (norm(a), norm(b));
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

/// 이 좌석 설정 폴더(순수 · 3값 [`SeatFolder`]) — `config_dir` = 데몬이 좌석에 기록한 `CLAUDE_CONFIG_DIR` · `trusted` = 그 기록의 출처가 데몬인가
/// (`Surface::config_dir_trusted`) · `session_file` = 관측된 transcript 경로(빈 문자열 = 관측 없음).
///
/// ★(0.14.45 · 성찰 M5) 종전에는 기록된 폴더를 **그대로** 썼다 — 데몬이 `~/.cys/claude` 를 기록했는데(CYS_ACCOUNT_DIR 없음) claude 는 팩 agents.json 의
/// 부서 계정 폴더나 손으로 준 `CLAUDE_CONFIG_DIR` 로 떠 있으면, 카드가 **엉뚱한 계정을 확인된 것처럼** 보였다. 규칙(위에서부터 첫 일치):
///   1. 관측된 transcript 의 프로필 폴더가 있으면 **그것**이 사실이다(기록과 다르면 관측이 이긴다 · 같으면 그대로) → `Verified(관측)`.
///   2. 관측이 없고 기록이 신뢰되면 → `Verified(기록)`.
///   3. 관측이 없고 기록은 있으나 신뢰되지 않으면 → `Unverified(기록)`(계정 불일치·확인 필요 — 확인되지 않은 계정을 확인된 것으로 내지 않는다).
///   4. 둘 다 없으면 → `Unknown`.
pub(crate) fn resolve_seat_folder(config_dir: Option<&str>, trusted: bool, session_file: &str) -> SeatFolder {
    let recorded = config_dir.map(str::trim).filter(|c| !c.is_empty());
    let observed = if session_file.trim().is_empty() {
        None
    } else {
        profile_dir_from_session(session_file).map(|p| p.to_string_lossy().into_owned())
    };
    match (observed, recorded) {
        (Some(o), Some(r)) if same_dir(&o, r) => SeatFolder::Verified(r.to_string()),
        (Some(o), _) => SeatFolder::Verified(o),
        (None, Some(r)) if trusted => SeatFolder::Verified(r.to_string()),
        (None, Some(r)) => SeatFolder::Unverified(r.to_string()),
        (None, None) => SeatFolder::Unknown,
    }
}

/// 좌석 표의 한 행(surfaces 락 안에서 복사한 메모리 값뿐).
#[derive(Clone, Debug)]
pub(crate) struct SeatRow {
    pub(crate) id: u64,
    pub(crate) agent: Option<String>,
    /// 신원을 읽어도 되는 폴더(claude 좌석만 · [`SeatFolder::verified`]).
    pub(crate) folder: Option<String>,
    /// 검증되지 않은 기록 폴더(claude 좌석만 · [`SeatFolder::unverified`]) — 신원을 읽지 않는다.
    pub(crate) unverified_dir: Option<String>,
}

/// 좌석 표 복사(surfaces 락 안 — 메모리 복사뿐 · 파일 IO 없음). None = 수집 실패(락 오염).
fn collect_seat_rows(daemon: &Arc<Daemon>) -> Option<Vec<SeatRow>> {
    let surfaces = daemon.surfaces.lock().ok()?;
    let mut out = Vec::new();
    for s in surfaces.values() {
        if s.exited.load(Ordering::Relaxed) {
            continue;
        }
        let meta_agent = s.agent_meta.lock().ok()?.as_ref().map(|(a, _)| a.clone());
        let config_dir = s.claude_config_dir.lock().ok()?.clone();
        let (obs_agent, session_file) = match s.observed_usage.lock().ok()?.as_ref() {
            Some(u) => (Some(u.agent.clone()), u.session_file.clone()),
            None => (None, String::new()),
        };
        let agent = meta_agent.or(obs_agent);
        let folder = if agent_is_claude(agent.as_deref()) {
            resolve_seat_folder(config_dir.as_deref(), s.config_dir_trusted, &session_file)
        } else {
            SeatFolder::Unknown
        };
        out.push(SeatRow {
            id: s.id,
            agent,
            folder: folder.verified().map(str::to_string),
            unverified_dir: folder.unverified().map(str::to_string),
        });
    }
    Some(out)
}

/// 폴더의 현재 신원(account_id) — **읽기-통과**: [`folder_identity_state`] 의 `Option` 판(`Known` 이면 Some · 신원 없음 확정·읽지 못함은 None). 계약·락 규율은 본체 문서와 같다.
/// **워치독(경보 틱)은 이 함수를 부르지 않는다**(파일을 연다) — 캐시 전용 [`peek_folder_ident`] 를 쓴다.
/// (`pub(crate)` — 알려진 프로필 폴더 전체의 현재 신원이 필요한 후속 소비자가 같은 60초 캐시를 재사용한다 · 새로 만들지 않는다.)
pub(crate) fn folder_identity(daemon: &Arc<Daemon>, home: Option<&Path>, folder: &str, now: f64) -> Option<String> {
    folder_identity_state(daemon, home, folder, now).account().map(str::to_string)
}

/// 폴더의 현재 신원 **상태**([`FolderWho`]) — **읽기-통과**: 60초 하한 캐시 → 미스면 락 없이 신원 파일을 확인한다(`claude_identity_probe` — stat + mtime 캐시 · FIFO 등 일반 파일이 아니면 열지 않는다).
/// 캐시 락은 조회·기록 때만 순간 잡는다(그 안에서 다른 락을 잡지 않는다).
/// ★R1F-US(M-1·③) **선점**: 미스(항목 없음·만료·시계 역행)면 IO **전에** `(now, 옛 값)` 으로 자리를 찍는다(`refresh_aliases` 와 같은 꼴) — 디스크가 멈춰도 그 폴더는 60초에 요청 하나만 묶이고, 그동안 다른 호출은 옛 값을
/// 받는다(처음 보는 폴더는 `Unread` = '아직 모름' = 판정 불가 = 사용 중 — 신원 없음 확정(`NoLogin`)이 아니다).
/// ★R1F-US(m-2·ⓐ) **일시 실패 내성**: 신원 파일은 있는데(메타 성공) 이번 판독·파싱만 실패했으면 직전 신원을 **한 주기(60초)만** 더 쓴다(`Known` 유지) — 연속 실패가 이어지면 다음 주기에는 `Unread`(무한 연장 없음).
/// 파일이 없어졌거나 `oauthAccount` 가 없는 것(신원 없음 **확정**)은 즉시 `NoLogin` — 읽지 못한 것이 아니다(보정).
pub(crate) fn folder_identity_state(daemon: &Arc<Daemon>, home: Option<&Path>, folder: &str, now: f64) -> FolderWho {
    let key = PathBuf::from(folder);
    let (old_state, old_misses) = {
        let mut c = daemon.seat_ident_cache.lock().unwrap_or_else(|e| e.into_inner());
        let prev = c.entries.get(&key).cloned();
        if let Some(slot) = &prev {
            // 시계가 뒤로 가면(now < at) 만료로 본다 — 캐시가 영구히 붙지 않는다.
            if now >= slot.at && now - slot.at < SEAT_IDENT_CACHE_SECS {
                c.hits += 1;
                return slot.state.clone();
            }
        }
        let (state, misses) = prev.map_or((FolderWho::Unread, 0), |s| (s.state, s.misses));
        c.entries.insert(key.clone(), IdentSlot { at: now, state: state.clone(), misses });
        (state, misses)
    };
    let (state, misses) = match claude_identity_probe(&daemon.accounts, home, &key) {
        IdentRead::Found((uuid, _, _)) => (FolderWho::Known(uuid), 0),
        IdentRead::NoIdentity => (FolderWho::NoLogin, 0),
        IdentRead::Unreadable if matches!(old_state, FolderWho::Known(_)) && old_misses == 0 => (old_state, 1),
        IdentRead::Unreadable => (FolderWho::Unread, old_misses.saturating_add(1)),
    };
    let mut c = daemon.seat_ident_cache.lock().unwrap_or_else(|e| e.into_inner());
    c.reads += 1;
    c.entries.insert(key, IdentSlot { at: now, state: state.clone(), misses });
    state
}

/// ★R1F-US(M-1·①) **판독 금지** 조회 — IO 0 · stat 0(캐시 락 한 번 · 메모리뿐). 항목이 있으면 **만료됐어도 마지막 값**, 없으면 None(= 판정 불가 = 사용 중으로 취급 — 실패 방향은 경보 유지).
/// 워치독(경보 틱)이 부르는 신원 표는 이것만 쓴다. 캐시에 쓰지 않는다(계측·항목 무변).
fn peek_folder_ident(daemon: &Arc<Daemon>, folder: &str) -> Option<String> {
    peek_folder_state(daemon, folder).account().map(str::to_string)
}

/// [`peek_folder_ident`] 의 3값판 — 같은 판독 금지 조회(IO 0 · 캐시 락 한 번 · 캐시에 쓰지 않는다)로 항목의 상태([`FolderWho`])를 돌려준다. 항목이 없으면 `Unread`(아직 모름). `local_json` 이 좌석 신원 표의 `None` 을
/// '신원 없음 확정'과 '읽지 못함'으로 가르는 데 쓴다(추가 IO 0).
fn peek_folder_state(daemon: &Arc<Daemon>, folder: &str) -> FolderWho {
    let c = daemon.seat_ident_cache.lock().unwrap_or_else(|e| e.into_inner());
    c.entries.get(Path::new(folder)).map_or(FolderWho::Unread, |s| s.state.clone())
}

/// ★R1F-US(M-1·②) 보고가 캐시를 채운다 — `usage.report` 의 귀속 경로(`note_rate_for_profile_at_resolved`·`note_rate_at_resolved`)는 이미 그 좌석 폴더의 신원을 읽었다. 그 값을 같은 60초 캐시에 싣는다(**추가 IO 0**) —
/// 창을 닫아 둬도 '보고하는 좌석의 폴더'는 늘 신선하다(워치독은 읽지 않고 이 캐시만 본다). 더 새로 기록된 항목(다른 소비자가 방금 확인 — 60초 안의 미래 시각)은 옛 보고 시각으로 덮지 않는다 · 그보다 먼 미래의 항목은
/// 시계가 뒤로 간 것이라 만료로 보고 덮는다(`folder_identity` 와 같은 규율 — 캐시가 영구히 붙지 않는다) · 캐시가 가득(512)이면 새 폴더는 싣지 않는다(RPC 경로가 비운다). 일시 실패 카운트는 0 으로 돌아간다(방금 정상으로 읽었다).
fn note_seat_identity(daemon: &Arc<Daemon>, folder: &Path, account_id: &str, now: f64) {
    let mut c = daemon.seat_ident_cache.lock().unwrap_or_else(|e| e.into_inner());
    match c.entries.get(folder) {
        Some(slot) if slot.at > now && slot.at - now <= SEAT_IDENT_CACHE_SECS => {}
        None if c.entries.len() >= SEAT_IDENT_CACHE_MAX => {}
        _ => {
            c.entries.insert(folder.to_path_buf(), IdentSlot { at: now, state: FolderWho::Known(account_id.to_string()), misses: 0 });
        }
    }
}

/// 좌석 신원 표(**읽기-통과** — RPC 경로) — 지금 시각·실제 홈으로 만든다.
#[cfg_attr(not(test), allow(dead_code))] // 운영 호출처는 시각 주입판(`_at`)을 쓴다(한 틱·한 응답 안에서 같은 `now` 를 나눠 쓴다)
pub fn seat_identity_view(daemon: &Arc<Daemon>) -> SeatIdentityView {
    seat_identity_view_at(daemon, crate::state::now_epoch())
}

/// [`seat_identity_view`] 의 시각 주입판(`local_json`·status 세 곳·`control.alerts` 의 스냅샷이 자기 `now` 로 부른다 · 캐시 하한도 이 시각으로 센다 · 읽기-통과: 미스면 신원 파일을 확인한다).
/// **워치독(경보 틱)은 이 함수를 부르지 않는다** — 캐시 전용 [`seat_identity_view_cached`] 를 쓴다.
pub fn seat_identity_view_at(daemon: &Arc<Daemon>, now: f64) -> SeatIdentityView {
    seat_identity_view_in(daemon, dirs::home_dir().as_deref(), now)
}

/// 홈·시각을 인자로 받는 시험 이음매(**읽기-통과** — RPC 경로가 쓴다). 락 순서: surfaces(복사만) → 해제 → 폴더별 신원 판독(무락 · 60초 캐시) — accounts 락은 쥐지 않는다.
pub(crate) fn seat_identity_view_in(daemon: &Arc<Daemon>, home: Option<&Path>, now: f64) -> SeatIdentityView {
    let Some(rows) = collect_seat_rows(daemon) else {
        return SeatIdentityView::default(); // 수집 실패 = 전부 '모름'
    };
    // 폴더별 현재 신원 — 같은 폴더는 한 번만(좌석 여럿이 한 폴더를 쓰는 것이 보통이다).
    let mut by_folder: BTreeMap<String, Option<String>> = BTreeMap::new();
    for r in &rows {
        if let (true, Some(f)) = (agent_is_claude(r.agent.as_deref()), &r.folder) {
            if !by_folder.contains_key(f) {
                let who = folder_identity(daemon, home, f, now);
                by_folder.insert(f.clone(), who);
            }
        }
    }
    {
        // **낡은 항목만** 걷는다(확인한 지 [`SEAT_IDENT_PRUNE_SECS`] 가 지난 폴더) — 좌석이 오가며 폴더가 바뀌어도 크기가 유계이고, 같은 캐시를 쓰는 다른 소비자(알려진 프로필
        // 폴더 전체 등 — `folder_identity`)의 항목은 신원 표를 만들 때 지워지지 않는다(지우면 그 소비자의 60초 하한이 무력화돼 폴더를 매번 stat 한다).
        let mut c = daemon.seat_ident_cache.lock().unwrap_or_else(|e| e.into_inner());
        c.entries.retain(|_, s| now < s.at || now - s.at <= SEAT_IDENT_PRUNE_SECS);
        if c.entries.len() > SEAT_IDENT_CACHE_MAX {
            c.entries.clear();
        }
    }
    assemble_seat_view(rows, by_folder)
}

/// ★R1F-US(M-1) **캐시 전용** 신원 표 — 워치독(경보 틱) 전용. 파일을 열지도 stat 하지도 않는다(IO 0): 폴더마다 [`peek_folder_ident`] 로 캐시의 **마지막 값**(만료됐어도)을 쓰고 항목이 없으면 None(= 판정 불가 = 사용 중)이다.
/// 락 순서: surfaces(복사만) → 해제 → 캐시 락(순간 · 메모리뿐) — 겹쳐 쥐는 락 쌍이 없다. 캐시를 채우는 것은 상태줄 보고와 RPC 의 읽기-통과 조회다 · 이 함수는 캐시에 쓰지도 않는다(계측·항목·prune 무변).
pub fn seat_identity_view_cached(daemon: &Arc<Daemon>) -> SeatIdentityView {
    let Some(rows) = collect_seat_rows(daemon) else {
        return SeatIdentityView::default(); // 수집 실패 = 전부 '모름'
    };
    let mut by_folder: BTreeMap<String, Option<String>> = BTreeMap::new();
    for r in &rows {
        if let (true, Some(f)) = (agent_is_claude(r.agent.as_deref()), &r.folder) {
            if !by_folder.contains_key(f) {
                let who = peek_folder_ident(daemon, f);
                by_folder.insert(f.clone(), who);
            }
        }
    }
    assemble_seat_view(rows, by_folder)
}

/// 좌석 행 + 폴더별 신원 → 신원 표(순수 · 락 없음 · 파일 IO 없음) — 읽기-통과판과 캐시 전용판이 같은 조립을 쓴다.
fn assemble_seat_view(rows: Vec<SeatRow>, by_folder: BTreeMap<String, Option<String>>) -> SeatIdentityView {
    let mut agents = AgentsAlive::default();
    let mut claude_folder_unknown = false;
    let mut seats = Vec::with_capacity(rows.len());
    for SeatRow { id, agent, folder, unverified_dir } in rows {
        match agent.as_deref() {
            Some("claude") => {
                agents.claude = true;
                if folder.is_none() {
                    claude_folder_unknown = true;
                }
            }
            Some("codex") => agents.codex = true,
            Some("gemini" | "agy" | "antigravity") => agents.gemini = true,
            _ => {}
        }
        let current_account = folder.as_ref().and_then(|f| by_folder.get(f).cloned().flatten());
        seats.push(SeatIdent {
            surface_id: id,
            agent,
            folder,
            unverified_dir,
            known: current_account.is_some(),
            current_account,
        });
    }
    seats.sort_by_key(|s| s.surface_id);
    SeatIdentityView {
        claude_folders: by_folder.into_iter().collect(),
        claude_folder_unknown,
        agents_alive: agents,
        collect_ok: true,
        seats,
    }
}

/// ★순수: 이 계정이 **지금 쓰이는가**(계정 축 `in_use`) — Some(true)/Some(false)/None(판정 불가). 판정 불가는 경보 쪽에서 '사용 중'으로 읽는다.
///   · claude: 현재 신원이 이 계정인 살아 있는 claude 좌석이 있으면 Some(true)(다른 좌석이 모름이어도 — 참이 이긴다). 없고 살아 있는 claude 좌석 중
///     폴더 미상 또는 신원 판독 실패가 하나라도 있으면 **모든 claude 계정에 None**. 전부 판독됐고 아무도 이 계정이 아니면 Some(false) ·
///     살아 있는 claude 좌석이 0개여도 Some(false).
///   · codex / antigravity(agy): 그 에이전트의 살아 있는 좌석이 있으면 Some(true), 없으면 Some(false).
///   · 그 밖(선언 계정 등) · 좌석 목록 수집 실패: None.
/// `None == None` 을 '같다'로 치지 않는다 — 판독 실패 폴더는 어느 계정과도 일치하지 않는다(모름이지 같음이 아니다).
pub fn account_in_use(provider: &str, account_id: &str, view: &SeatIdentityView) -> Option<bool> {
    if !view.collect_ok {
        return None;
    }
    match provider {
        "claude" => {
            if view.claude_folders.iter().any(|(_, who)| who.as_deref() == Some(account_id)) {
                return Some(true);
            }
            if view.claude_folder_unknown || view.claude_folders.iter().any(|(_, who)| who.is_none()) {
                return None;
            }
            Some(false)
        }
        "codex" => Some(view.agents_alive.codex),
        "antigravity" | "gemini" | "agy" => Some(view.agents_alive.gemini),
        _ => None,
    }
}

/// ★순수: 좌석의 rate 가 **지금 쓰이는 로그인의 것인가** — 3값(Some(false) = 로그인이 바뀐 뒤 아직 새로 보고하지 않은 좌석 · None = 판정 불가).
/// 종료 좌석은 Some(false) · claude 가 아닌 좌석은 항상 Some(true). claude 는 보고 시점 귀속 계정(`rate_account`)과 현재 폴더 신원이 **둘 다 Some 이고
/// 다를 때만** Some(false), 둘 다 Some 이고 같으면 Some(true), 하나라도 None 이면 None.
pub fn seat_in_use_tri(rate_account: Option<&str>, current: Option<&str>, alive: bool, is_claude: bool) -> Option<bool> {
    if !alive {
        return Some(false);
    }
    if !is_claude {
        return Some(true);
    }
    match (rate_account, current) {
        (Some(a), Some(c)) => Some(a == c),
        _ => None,
    }
}

/// ★순수: 좌석 축 `seat_in_use` — 판정 불가는 '사용 중'이다(실패 방향 = 경보 유지). [`seat_in_use_tri`] 가 Some(false) 가 아닐 때 참.
#[cfg_attr(not(test), allow(dead_code))] // 검체가 표로 핀하는 2값판 — 운영은 3값판(`seat_in_use_tri`)을 쓴다(status 의 null 을 위해)
pub fn seat_in_use(rate_account: Option<&str>, current: Option<&str>, alive: bool, is_claude: bool) -> bool {
    seat_in_use_tri(rate_account, current, alive, is_claude) != Some(false)
}

/// ★순수: 경보 적격 = `live` ∧ (`in_use != Some(false)` ∨ 관측 나이 ≤ `stale_secs`). `stale_secs == 0` 은 이 규칙을 끈다(`live` 만 — 0.14.42 동작).
/// 판정 불가(`None`)는 사용 중으로 본다 · 나이가 비유한(NaN)이면 '넘지 않음'으로 본다(둘 다 경보를 지우는 쪽으로 오판하지 않는다).
pub fn alert_eligible(live: bool, in_use: Option<bool>, age: f64, stale_secs: f64) -> bool {
    if !live {
        return false;
    }
    if stale_secs <= 0.0 {
        return true;
    }
    in_use != Some(false) || !(age > stale_secs)
}

/// 계정 축 경보 입력 한 행(창 단위) — `rate_window_live` 인 창만 행이 된다. `eligible=false` 인 행이 '신선도 규칙으로 빠진' 창이다.
#[derive(Clone, Debug, PartialEq)]
pub struct AccountAlertRow {
    pub label: String,
    pub win: String,
    pub used_pct: f64,
    /// 경보 입력의 관측 시각(`AlertInput::at`).
    pub observed_at: f64,
    pub age_secs: f64,
    pub in_use: Option<bool>,
    pub resets_at: Option<f64>,
    /// PEAK_HOLD 잔여(초) — 같은 리셋 창의 최댓값을 더 쥐는 시간. `peak_at` 이 있을 때만(복원분은 None).
    pub held_secs: Option<f64>,
    pub eligible: bool,
}

/// 계정 축 경보 입력(적격 여부 표시) — 락 순서: 신원 표는 부른 쪽이 락 밖에서 만든다. 이 함수는 accounts 락 안에서 메모리 연산만 한다.
pub fn alert_rates_with(
    daemon: &Arc<Daemon>,
    view: &SeatIdentityView,
    now: f64,
    stale_secs: f64,
) -> Vec<AccountAlertRow> {
    let st = daemon.accounts.lock().unwrap();
    let mut out = Vec::new();
    for (key, input) in st.alert_inputs.iter() {
        if input.at == 0.0 {
            continue;
        }
        let in_use = account_in_use(&key.provider, &key.account_id, view);
        let age = (now - input.at).max(0.0);
        for w in &input.rate {
            // ★fatal-fix R3-2 · ROLE-3: 리셋이 지난 창은 행이 아니다(경보 근거 아님 · 신선도 규칙과 무관).
            if !crate::usage::rate_window_live(w, input.at, now) {
                continue;
            }
            let held_secs = input.peak_at.get(&w.label).map(|p| (PEAK_HOLD_SECS - (now - *p)).max(0.0));
            out.push(AccountAlertRow {
                label: input.label.clone(),
                win: w.label.clone(),
                used_pct: w.used_pct,
                observed_at: input.at,
                age_secs: age,
                in_use,
                resets_at: w.resets_at,
                held_secs,
                eligible: alert_eligible(true, in_use, age, stale_secs),
            });
        }
    }
    // 결정론: (라벨, 창) 오름차순 · 같은 키(같은 이메일의 두 계정)는 사용률 큰 쪽이 먼저 · 같은 값이면 관측 나이가 작은 쪽(R1F-US m-6: 스냅샷이 키당 값이 가장 큰 입력을 쥔다).
    out.sort_by(|a, b| {
        (&a.label, &a.win)
            .cmp(&(&b.label, &b.win))
            .then(b.used_pct.total_cmp(&a.used_pct))
            .then(a.age_secs.total_cmp(&b.age_secs))
    });
    out
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
/// ★0.14.43(B3): 신선도 규칙(리셋 전 ∧ (사용 중 ∨ 관측 나이 ≤ 1800초))에 **적격인 행만** 싣는다 — [`alert_rates_with`] 의 얇은 래퍼다(기존 호출처·검체 보존).
#[cfg_attr(not(test), allow(dead_code))]
pub fn alert_rates(daemon: &Arc<Daemon>) -> Vec<(String, String, f64)> {
    let now = crate::state::now_epoch();
    let view = seat_identity_view_at(daemon, now);
    alert_rates_with(daemon, &view, now, account_alert_stale_secs())
        .into_iter()
        .filter(|r| r.eligible)
        .map(|r| (r.label, r.win, r.used_pct))
        .collect()
}

/// 좌석 `usage` 객체의 wire(JSON) — `surface.list` · `org.status` · `control.dashboard` 세 곳이 **이 도우미 하나**를 쓴다. 기존 직렬화(`ObservedUsage` 의 필드)에
/// 계산 키를 **가산**한다: `rate_observed_at`(rate 가 새로 생산된 시각 · 모르면 null) · `rate_age_secs`(그 나이 · 모르면 null) · `rate_in_use`(true/false/null —
/// 로그인이 바뀐 뒤 아직 새로 보고하지 않은 좌석이면 false · 판정 불가면 null) · `rate[]` 원소마다 `alert_eligible`(이 창이 경보 입력이 될 수 있는가).
/// 원 uuid(`rate_account`)는 내보내지 않는다. `view` 는 부른 쪽이 **surfaces 락을 잡기 전에** 만든다(파일 IO).
pub fn seat_usage_wire(
    u: &ObservedUsage,
    surface_id: u64,
    alive: bool,
    view: &SeatIdentityView,
    now: f64,
    stale_secs: f64,
) -> Value {
    let mut v = serde_json::to_value(u).unwrap_or(Value::Null);
    let Some(obj) = v.as_object_mut() else {
        return v;
    };
    let observed = u.rate_observed_at;
    let known_at = observed.is_finite() && observed > 0.0;
    let age = if known_at { (now - observed).max(0.0) } else { 0.0 };
    obj.insert("rate_observed_at".into(), if known_at { json!(observed) } else { Value::Null });
    obj.insert("rate_age_secs".into(), if known_at { json!(age as u64) } else { Value::Null });
    let tri = seat_in_use_tri(
        u.rate_account.as_deref(),
        view.current_for(surface_id),
        alive,
        agent_is_claude(Some(u.agent.as_str())),
    );
    obj.insert("rate_in_use".into(), tri.map_or(Value::Null, Value::Bool));
    // 관측 시각을 모르면(0) 나이 판정 없이 적격(종전 동작) · 창의 생사는 종전처럼 updated_at 으로 본다.
    let live_at = if known_at { observed } else { u.updated_at };
    if let Some(Value::Array(arr)) = obj.get_mut("rate") {
        for (w, jv) in u.rate.iter().zip(arr.iter_mut()) {
            let live = alive && crate::usage::rate_window_live(w, live_at, now);
            if let Some(o) = jv.as_object_mut() {
                o.insert("alert_eligible".into(), json!(alert_eligible(live, tri, age, stale_secs)));
            }
        }
    }
    v
}

// ───────────────── ★0.14.45 좌석별 계정(`account`) — `surface.list`·`org.status` 좌석의 표시 전용 가산 키 ─────────────────
//
// 오너 요청(2026-10-07): "어느 부서·노드가 어느 계정을 쓰는지 몰라 pane 마다 `/config` 를 쳐야 한다." 데몬은 이미 좌석마다 설정 폴더와 그 폴더의
// **현재** 신원을 안다([`SeatIdentityView`] — 경보·`in_use` 의 재료). 그 사실을 좌석 행에 그대로 싣는다 — 화면은 이 키를 `usage.accounts` 행의
// (provider, account_id)와 맞춰 별명·이메일(툴팁)을 고른다. **새 IO 0**: 신원 표는 부른 쪽이 이미 만들었고(60초 캐시), 상태 구분은 캐시 엿보기([`peek_folder_state`])뿐이다.
// ★결측은 값이 아니다 — 모르는 것은 `account_id: null` + 사유(`state`)로 보낸다. 추측(폴더 이름으로 계정을 짐작)하지 않는다.

/// 좌석 에이전트 이름 → 계정 축 provider(`usage.accounts` 의 키와 같은 표기). 모르는 에이전트는 None — cys 가 그 신원을 읽는 경로가 없다.
fn seat_provider(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        "gemini" | "agy" | "antigravity" => Some("antigravity"),
        _ => None,
    }
}

/// ★순수: 좌석 하나의 `account` 객체. 에이전트 증거가 없는 좌석(셸)은 `null`.
///   · claude: `who` 가 `Known(id)` 면 `state:"known"` + `account_id`. 폴더 미상이면 `"folder_unknown"`, 로그인 없음 확정이면 `"no_login"`, 읽지 못했으면 `"unread"` — 셋 다 `account_id:null`.
///     `profile` 은 설정 폴더(`usage.accounts` 의 `profiles` 와 같은 홈 상대 표기) — 모를 때도 근거로 싣지만 **계정의 대용이 아니다**.
///     ★(0.14.45 · 성찰 M5) `unverified`(검증되지 않은 기록 폴더 — [`SeatFolder::Unverified`])가 있으면 `state:"mismatch"` · `account_id:null` · `profile` = 그 기록 폴더(근거 표기용).
///     기록 폴더의 신원을 읽어 내지 않는다 — 확인되지 않은 계정을 확인된 것처럼 보이지 않게(화면은 「계정 불일치·확인 필요」).
///   · codex · antigravity: 이 데몬의 계정 축에 그 제공자의 계정은 하나(`default` — 기기의 로그인 하나)뿐이다 → `state:"known"` · `account_id:"default"`.
///   · 그 밖 에이전트: `state:"unsupported"`(provider null).
pub(crate) fn seat_account_json(agent: Option<&str>, folder: Option<&str>, unverified: Option<&str>, who: Option<&FolderWho>, home: Option<&Path>) -> Value {
    let Some(agent) = agent else {
        return Value::Null;
    };
    let Some(provider) = seat_provider(agent) else {
        return json!({"provider": Value::Null, "agent": agent, "account_id": Value::Null, "profile": Value::Null, "state": "unsupported"});
    };
    if provider != "claude" {
        return json!({"provider": provider, "agent": agent, "account_id": "default", "profile": Value::Null, "state": "known"});
    }
    if let (None, Some(u)) = (folder, unverified) {
        let profile = profile_short(home, Path::new(u));
        return json!({"provider": provider, "agent": agent, "account_id": Value::Null, "profile": profile, "state": "mismatch"});
    }
    let profile = folder.map(|f| profile_short(home, Path::new(f)));
    let (account_id, state) = match (folder, who) {
        (None, _) => (None, "folder_unknown"),
        (Some(_), Some(FolderWho::Known(id))) => (Some(id.clone()), "known"),
        (Some(_), Some(FolderWho::NoLogin)) => (None, "no_login"),
        (Some(_), _) => (None, "unread"),
    };
    json!({"provider": provider, "agent": agent, "account_id": account_id, "profile": profile, "state": state})
}

/// 좌석 id → `account` 객체 표 — `surface.list`·`org.status` 가 **surfaces 락을 잡기 전에** 한 번 만든다(락 순서: 신원 표는 이미 만들어졌고, 여기서는 신원 캐시 락을
/// 폴더마다 순간 잡을 뿐이다 · 파일 IO 0). 표에 없는 좌석(종료 · 표를 만든 뒤 생성 · 수집 실패)은 부른 쪽이 `null` 로 싣는다.
/// 신원 표의 `current_account` 가 없는데 캐시가 `Known` 이면(표를 만든 뒤 다른 요청이 읽었다) 표와 어긋나지 않게 `unread` 로 둔다 — 한 응답 안의 `usage.rate_in_use` 와 다른 사실을 말하지 않는다.
pub fn seat_account_map(daemon: &Arc<Daemon>, view: &SeatIdentityView) -> HashMap<u64, Value> {
    let mut out = HashMap::new();
    if !view.collect_ok {
        return out;
    }
    let home = account_home();
    for s in &view.seats {
        let who = match (&s.current_account, &s.folder) {
            (Some(id), _) => Some(FolderWho::Known(id.clone())),
            (None, Some(f)) => match peek_folder_state(daemon, f) {
                FolderWho::Known(_) => Some(FolderWho::Unread),
                other => Some(other),
            },
            (None, None) => None,
        };
        out.insert(
            s.surface_id,
            seat_account_json(s.agent.as_deref(), s.folder.as_deref(), s.unverified_dir.as_deref(), who.as_ref(), home.as_deref()),
        );
    }
    out
}

// ───────────────── ★0.14.43(B1) 현재 로그인 폴더(`current_profiles`) · 별명(`alias`) — `usage.accounts` 행의 표시 전용 가산 키 ─────────────────

/// 별명 파일(`~/.cys/accounts.json`)의 확인 주기 하한(초) — 이 안에서는 stat 도 하지 않는다(사용량 RPC 폴링이 파일 접근을 늘리지 않게). 시계가 뒤로 가면(now < 마지막 확인) 만료로 본다.
const ALIAS_CHECK_SECS: f64 = 60.0;
/// 별명 파일 크기 상한(바이트) — 넘으면 별명 없음(병적 입력이 판독 시간·메모리를 밀지 못하게).
const ALIAS_FILE_MAX_BYTES: u64 = 64 * 1024;
/// 별명 길이 상한(char 기준) — 사이드바·툴팁을 밀지 못하게(화면의 `acctAlias` 도 같은 24자).
const ALIAS_MAX_CHARS: usize = 24;
/// ★0.14.43(B1b): 선언 계정 파일(`~/.cys/accounts.json` 의 `accounts` 배열)의 크기 상한(바이트) — **별명의 64KiB([`ALIAS_FILE_MAX_BYTES`])와 별개**다. 정상 파일의 종전 동작(상한 없는 판독)을 바꾸지 않을
/// 만큼 넉넉하게(1MiB) 두되, 병적 입력(거대·무한 파일)이 소켓 bind **전**의 부트 체인을 붙들지 못하게 한다.
const DECLARED_FILE_MAX_BYTES: u64 = 1024 * 1024;

/// 이 구역의 파일 판독(알려진 프로필 열거 · 별명 파일)이 보는 홈 — 운영은 `dirs::home_dir()` 하나다. 시험은 [`test_home`] 이음매로 **이 스레드에서만** 임시 홈으로 바꾼다.
fn account_home() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(h) = test_home::get() {
        return Some(h);
    }
    dirs::home_dir()
}

/// 검체 이음매(테스트 전용) — [`account_home`] 을 이 스레드에서만 임시 홈으로 대체한다(프로세스 env `HOME` 은 병렬 검체끼리 공유되므로 바꾸지 않는다 · 실제 홈의 파일을 읽지 않는다). 가드가 떨어지면 원복.
#[cfg(test)]
pub(crate) mod test_home {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    thread_local! {
        static HOME: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }
    /// 가드가 떨어지면 **이전 값**으로 되돌린다(중첩해서 걸어도 바깥 가드의 홈이 살아 있다).
    pub(crate) struct Guard(Option<PathBuf>);
    pub(crate) fn set(h: &Path) -> Guard {
        Guard(HOME.with(|c| c.borrow_mut().replace(h.to_path_buf())))
    }
    pub(crate) fn get() -> Option<PathBuf> {
        HOME.with(|c| c.borrow().clone())
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            let prev = self.0.take();
            HOME.with(|c| *c.borrow_mut() = prev);
        }
    }
}

/// 별명 표 상태([`AccountsState::alias`]) — 정제된 표 · 마지막으로 본 파일 서명 · 마지막 확인 시각 · 계측(검체·진단).
#[derive(Default)]
struct AliasState {
    /// 정제된 표 — 키(accountUuid 또는 이메일) 원문 → 정제된 별명([`sanitize_alias`]).
    table: HashMap<String, String>,
    /// 표를 만든 파일의 서명 (mtime 초, 바이트 수) — None = 읽을 수 있는 일반 파일이 없다(부재·FIFO 등 비정규·64KiB 초과·메타 실패·열기 실패).
    sig: Option<(f64, u64)>,
    /// 마지막 확인(stat 시도) 시각 — None = 아직 확인 전.
    checked_at: Option<f64>,
    /// 계측 — stat 시도 수 / 실제 판독 수(60초 하한 안에서는 둘 다 늘지 않는다).
    stats: u64,
    reads: u64,
}

/// ★R1F-US(n-2): 별명에서 걷는 글자 — 제어 문자(`char::is_control`)에 더해 양방향 제어(U+202A~202E · U+2066~2069) · 제로폭(U+200B~200F · U+FEFF) · 줄/문단 구분자(U+2028·2029). 화면 `ui/src/starvednotice.ts` 의
/// `INVISIBLE` 과 같은 집합이다(표시를 속이거나 깨뜨리는 것들).
fn is_hidden_alias_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200B}'..='\u{200F}' | '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// ★순수: 별명 값 정제 — 제어 문자·양방향 제어·제로폭([`is_hidden_alias_char`]) 제거 → 앞뒤 공백 제거 → 비면 None → 최대 24자(char 기준)로 자른다(절단이 공백에서 끝나면 그 공백도 걷는다). 표시 전용.
fn sanitize_alias(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|c| !is_hidden_alias_char(*c)).collect();
    let t = cleaned.trim();
    if t.is_empty() {
        return None;
    }
    let cut: String = t.chars().take(ALIAS_MAX_CHARS).collect();
    let cut = cut.trim_end();
    (!cut.is_empty()).then(|| cut.to_string())
}

/// ★순수: `accounts.json` 본문 → 별명 표. 형식 `{"aliases": {"<accountUuid 또는 이메일>": "업무용"}}`(키는 [`alias_for`] 가 account_id → label 순으로 찾는다).
/// JSON 오류(UTF-8 아님 포함)·루트가 객체가 아님·`aliases` 가 객체가 아님 = 빈 표. 값이 문자열이 아니거나 정제 뒤 비면 그 항목만 버린다(키가 빈 문자열인 항목도).
/// 선언 계정(`accounts` 배열)은 여기서 읽지 않는다 — `seed_known` 이 종전 그대로 읽는다.
fn parse_alias_table(body: &[u8]) -> HashMap<String, String> {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return HashMap::new();
    };
    let Some(obj) = v.get("aliases").and_then(Value::as_object) else {
        return HashMap::new();
    };
    obj.iter()
        .filter(|(k, _)| !k.is_empty())
        .filter_map(|(k, val)| val.as_str().and_then(sanitize_alias).map(|a| (k.clone(), a)))
        .collect()
}

/// 별명 파일의 서명 — **메타데이터만** 본다(내용 무접촉 · 락 밖). 읽을 수 있는 일반 파일(크기 ≤ 64KiB)이 아니면(부재·FIFO·장치·디렉터리·초과) None —
/// 그런 것은 **열지 않는다**(FIFO 는 여는 순간 쓰는 쪽이 올 때까지 막힌다).
fn alias_file_sig(path: &Path) -> Option<(f64, u64)> {
    let md = std::fs::metadata(path).ok()?;
    if !md.is_file() || md.len() > ALIAS_FILE_MAX_BYTES {
        return None;
    }
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64());
    Some((mtime, md.len()))
}

/// ★일반 파일 안전 판독(공용 · **락 밖 전용**) — 여는 것도 막히지 않게 연다(unix `O_NONBLOCK` — 일반 파일 읽기에는 영향이 없다)고, 연 뒤 fstat 으로 **일반 파일**·크기(`max_bytes`)를 다시 확인한다
/// (stat 과 open 사이에 FIFO 로 바뀌는 경쟁 차단 — [`read_identity_file`] 과 같은 규율). FIFO·장치·디렉터리는 읽지 않는다(FIFO 는 여는 순간 쓰는 쪽이 올 때까지 막힌다).
/// None = 열기·읽기 실패·비정규·`max_bytes` 초과 — 호출부가 각자의 '없음'으로 읽는다(별명: '별명 없음 · 다음 확인에 재시도' · 선언 계정: '선언 계정 없음').
fn read_regular_file_capped(f: &Path, max_bytes: u64) -> Option<Vec<u8>> {
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
    if !md.is_file() || md.len() > max_bytes {
        return None;
    }
    let mut buf = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > max_bytes {
        return None;
    }
    Some(buf)
}

/// 별명 파일 판독 — [`read_regular_file_capped`] 에 별명 상한(64KiB)을 준다.
fn read_alias_file(f: &Path) -> Option<Vec<u8>> {
    read_regular_file_capped(f, ALIAS_FILE_MAX_BYTES)
}

/// ★0.14.43(B1b): 선언 계정 파일 본문 — 부트 체인(소켓 bind **전**에 동기로 도는 `seed_known`·`spawn_custom_adapters`)이 쓴다. 종전의 무제한 문자열 판독은 FIFO 에서 영원히 막혀 부트 전체가 섰다 —
/// 이제 일반 파일만(상한 [`DECLARED_FILE_MAX_BYTES`] 1MiB) 읽고, 부재·FIFO·디렉터리·초과·읽기 오류·UTF-8 아님은 모두 None = 종전의 `Err` 와 같은 '선언 계정 없음'이다(조용히). 락 밖 전용.
fn read_declared_accounts_text(decl: &Path) -> Option<String> {
    String::from_utf8(read_regular_file_capped(decl, DECLARED_FILE_MAX_BYTES)?).ok()
}

/// 별명 표 갱신 — 부트(`seed_known`)에서 1회 · 이후 `local_json` 이 부른다. ① **60초 하한**: 마지막 확인에서 60초가 안 지났으면 stat 도 하지 않고 돌아온다(시계 역행은 만료) ·
/// ② 하한이 지났으면 파일 **서명(mtime·크기)** 만 본다 — 마지막 판독 때와 같으면 재판독 없음 · 바뀌었을 때만 읽는다 · ③ 읽을 수 없는 파일(부재·FIFO 등 비정규·64KiB 초과·JSON 오류·
/// `aliases` 가 객체 아님)은 **별명 없음**이고 기존 표는 비운다(조용히 — 어떤 경우에도 RPC 를 세우지 않는다).
/// 락: accounts 락은 ①의 하한 판정·③의 저장 때만 **순간** 잡는다(메모리뿐) — 파일 IO(stat·open·read)는 어떤 락도 쥐지 않은 채 한다. 다른 락을 쥔 채 부르지 않는다.
fn refresh_aliases(daemon: &Arc<Daemon>, home: Option<&Path>, now: f64) {
    // ① 하한 판정 + 검사권 선점(같은 틱의 동시 호출이 중복 판독하지 않게) — 락은 순간 · 파일 IO 없음
    let prev = {
        let mut st = daemon.accounts.lock().unwrap();
        let a = &mut st.alias;
        if a.checked_at.is_some_and(|at| now >= at && now - at < ALIAS_CHECK_SECS) {
            return;
        }
        a.checked_at = Some(now);
        a.stats += 1;
        a.sig
    };
    // ② 락 밖 — 서명(stat) → 바뀌었을 때만 내용을 읽는다
    let path = home.map(|h| h.join(".cys").join("accounts.json"));
    let seen = path.as_deref().and_then(alias_file_sig);
    if seen == prev {
        return; // 변화 없음(부재 → 부재 포함) — 표는 이미 그 파일의 것이다
    }
    let mut did_read = false;
    let (table, sig) = match (path.as_deref(), seen) {
        (Some(p), Some(_)) => {
            did_read = true;
            match read_alias_file(p) {
                Some(body) => (parse_alias_table(&body), seen),
                None => (HashMap::new(), None), // 열기·읽기 실패 — 별명 없음 · 다음 확인(60초 뒤)에 다시 읽는다
            }
        }
        _ => (HashMap::new(), None), // 부재·비정규·초과 — 별명 없음
    };
    // ③ 저장 — 락은 순간(메모리뿐)
    let mut st = daemon.accounts.lock().unwrap();
    st.alias.table = table;
    st.alias.sig = sig;
    st.alias.reads += u64::from(did_read);
}

/// ★순수: 행의 별명 — `account_id` 키가 `label`(이메일) 키보다 우선한다. 없으면 None. (표시 전용 — 경보 키·라벨·이벤트에 쓰지 않는다.)
/// ★R1F-US(n-1): `account_id == "default"` 인 행(Codex·Antigravity·선언 계정 — 계정 id 가 전부 `default`)은 `label` 키를 **먼저** 본다 — `{"default":"X","OpenAI Codex":"코덱스"}` 에서 `default` 키가 라벨 키를 가리지 않는다
/// (라벨 키가 없을 때만 `default` 키로 내려간다 · 그 밖 행은 종전 순서).
fn alias_for<'a>(table: &'a HashMap<String, String>, account_id: &str, label: &str) -> Option<&'a str> {
    let hit = if account_id == "default" {
        table.get(label).or_else(|| table.get(account_id))
    } else {
        table.get(account_id).or_else(|| table.get(label))
    };
    hit.map(String::as_str)
}

/// ★R1F-US(m-5) 알려진 프로필 폴더 열거 — 열거 정본(`cys::profile_gate::enumerate_profile_dirs`: 홈과 `~/.cys` 의 `read_dir`)을 **60초 하한**으로 캐시한다(`SeatIdentCache` · 신원 항목과 같은 선점 꼴: 미스(항목 없음·만료·
/// 시계 역행·다른 홈)면 IO 전에 `(now, 옛 값)` 으로 자리를 찍는다 — 디스크가 멈춰도 요청 하나만 묶이고, 5초 폴링이 열거를 늘리지 않는다). 반환 None = **열거 실패**: 정본이 읽기 오류를 조용히 빈 목록으로 접으므로, 목록이 비었으면
/// 홈 `read_dir` 가 되는지로 가른다(안 되면 실패 · 되면 `Some([])` = 정말 프로필 폴더가 없다). 선점 중 처음 보는 홈이면 None. 캐시 락은 조회·기록 때만 순간 잡는다(IO 는 락 밖).
fn enumerated_profile_dirs(daemon: &Arc<Daemon>, home: &Path, now: f64) -> Option<Vec<PathBuf>> {
    {
        let mut c = daemon.seat_ident_cache.lock().unwrap_or_else(|e| e.into_inner());
        let prev = c.enumerated.as_ref().filter(|(h, _, _)| h.as_path() == home).cloned();
        if let Some((_, at, list)) = &prev {
            // 시계가 뒤로 가면(now < at) 만료로 본다.
            if now >= *at && now - *at < SEAT_IDENT_CACHE_SECS {
                return list.clone();
            }
        }
        let old = prev.and_then(|(_, _, list)| list);
        c.enumerated = Some((home.to_path_buf(), now, old));
    }
    let list = cys::profile_gate::enumerate_profile_dirs(home);
    let result = (!list.is_empty() || std::fs::read_dir(home).is_ok()).then_some(list);
    let mut c = daemon.seat_ident_cache.lock().unwrap_or_else(|e| e.into_inner());
    c.enum_reads += 1;
    c.enumerated = Some((home.to_path_buf(), now, result.clone()));
    result
}

/// 알려진 claude 프로필 폴더와 각 폴더의 **현재** 신원 상태([`FolderWho`]) — (표시 표기, 신원 상태). 폴더 목록은 부트 시드(`discover_at`)와 **같은 열거 정본**
/// (`cys::profile_gate::enumerate_profile_dirs` — 기본 프로필 `~/.claude` 포함 · 60초 하한 캐시 [`enumerated_profile_dirs`])이고, 표기는 `profiles` 원소를 만드는 `profile_short` 다. 신원은 B3 의 폴더별 60초 하한 캐시
/// ([`folder_identity_state`])로 읽는다(새 캐시 없음 — 같은 폴더를 좌석 신원 표가 이미 읽었으면 적중). 신원 없음 확정(파일 없음·FIFO·`oauthAccount` 없음 = `NoLogin`)과 읽지 못함(파싱·읽기 실패의 유예 뒤 = `Unread`)은 둘 다
/// 어느 계정과도 일치하지 않지만 `current_profiles` 에서는 다르게 다룬다(보정).
/// ★R1F-US: 반환 None = 폴더 목록을 못 얻었다(홈 불명 · 열거 실패) — 부른 쪽은 `current_profiles` 키를 내지 않는다.
/// 파일 IO(열거 + 신원)는 어떤 락도 쥐지 않은 채 한다 — 부른 쪽이 accounts 락을 잡기 **전에** 부른다.
fn known_profile_identities(daemon: &Arc<Daemon>, home: Option<&Path>, now: f64) -> Option<Vec<(String, FolderWho)>> {
    let h = home?;
    let dirs = enumerated_profile_dirs(daemon, h, now)?;
    Some(
        dirs.into_iter()
            .map(|dir| {
                let who = folder_identity_state(daemon, home, &dir.to_string_lossy(), now);
                (profile_short(home, &dir), who)
            })
            .collect(),
    )
}

/// 좌석 신원 표(`view.claude_folders`)의 폴더를 3값으로 — 신원을 읽은 폴더는 `Known`, 신원 표에서 None 인 폴더는 캐시 항목의 상태(`NoLogin`: 신원 없음 확정 · `Unread`: 읽지 못함)로 가른다(캐시 조회뿐 — 추가 IO 0 ·
/// 항목이 없으면 `Unread` = 아직 모름). `local_json` 이 accounts 락을 잡기 **전에** 부른다.
fn seat_folder_states(daemon: &Arc<Daemon>, view: &SeatIdentityView) -> Vec<(String, FolderWho)> {
    view.claude_folders
        .iter()
        .map(|(folder, who)| {
            let state = match who {
                Some(account) => FolderWho::Known(account.clone()),
                None => peek_folder_state(daemon, folder),
            };
            (folder.clone(), state)
        })
        .collect()
}

/// ★순수(R1F-US · m-1): 알려진 폴더 표에 **좌석 폴더**를 합친다 — 좌석 폴더의 3값 상태(`seat` — [`seat_folder_states`] · 이미 읽은 신원)를 같은 표기(`profile_short`)로 더한다(추가 IO 0). 열거 규칙 밖 좌석 폴더
/// (`CYS_ACCOUNT_DIR=<임의>` · 부서 카탈로그의 임의 계정 폴더)를 쓰는 계정이 `in_use:true` 인데 `current_profiles:[]`(= 화면의 `● 사용 중` + `이전 로그인`)이 되는 모순을 없앤다. 같은 표기가 이미 있으면(열거된 폴더)
/// 그대로 둔다 · 구분자(`\` 와 `/`)는 같은 것으로 본다(윈도우에서는 열거 경로와 보고 경로가 섞인다).
fn merge_seat_folders(mut known: Vec<(String, FolderWho)>, home: Option<&Path>, seat: &[(String, FolderWho)]) -> Vec<(String, FolderWho)> {
    for (folder, who) in seat {
        let short = profile_short(home, Path::new(folder));
        let norm = short.replace('\\', "/");
        if !known.iter().any(|(p, _)| p.replace('\\', "/") == norm) {
            known.push((short, who.clone()));
        }
    }
    known
}

/// ★순수: 행의 `current_profiles` — claude 는 **현재 신원이 이 계정**인 알려진 폴더만(정렬·중복 제거 · 신원 없음·읽지 못함 폴더는 누구의 것도 아니다 — `None == None` 을 '같다'로 치지 않는다).
/// 그 밖 provider(codex·agy·선언 계정)는 폴더당 계정 1개이고 신원 전환이 없으므로 그 행의 `profiles` 그대로. `profiles` 자체는 건드리지 않는다(추가 전용 · 이관·삭제 없음).
/// ★R1F-US(m-2·ⓑ · 보정): **None = 키를 내지 않는다** — claude 행에서 ① 폴더 표를 못 얻었거나(`known` None — 홈 불명·열거 실패) ② 그 행의 `profiles` 에 든 폴더 가운데 이번에 신원을 **읽지 못한**
/// 것(표에서 `Unread` — 판독 실패가 유예 뒤까지 이어졌거나 처음 보는 폴더를 아직 읽는 중)이 하나라도 있으면, `[]`(= 어디에도 로그인돼 있지 않음 = 화면의 '이전 로그인')을 단정하지 않는다(화면은 키 부재 → `profiles` 폴백).
/// **신원 없음이 확정인 폴더(`NoLogin` — 로그아웃·신원 파일 없음)는 읽지 못한 것이 아니다** — '이 계정의 폴더가 아니다'로 세므로 그 계정의 다른 폴더가 없으면 `[]` 이다(9650334f 와 같다).
/// 신원 판독이 일시 실패한 폴더는 [`folder_identity_state`] 가 직전 신원을 한 주기 더 쓰므로(`Known`) 여기까지 오지 않는다. `profiles` 와 표의 폴더는 구분자(`\` 와 `/`)를 같은 것으로 맞춰 비교한다(윈도우).
fn current_profiles_for(
    provider: &str,
    account_id: &str,
    profiles: &BTreeSet<String>,
    known: Option<&[(String, FolderWho)]>,
) -> Option<Vec<String>> {
    if provider != "claude" {
        return Some(profiles.iter().cloned().collect());
    }
    let known = known?;
    let norm = |p: &str| p.replace('\\', "/");
    let unread: BTreeSet<String> = known.iter().filter(|(_, who)| *who == FolderWho::Unread).map(|(p, _)| norm(p)).collect();
    if profiles.iter().any(|p| unread.contains(&norm(p))) {
        return None;
    }
    Some(
        known
            .iter()
            .filter(|(_, who)| who.account() == Some(account_id))
            .map(|(p, _)| p.clone())
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect(),
    )
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
        // ★(R4-01) 프로필 밖 경로는 **파일시스템을 건드리기 전에** 거절한다 — 없는 경로라도 `missing` 이 아니라
        //   `outside_profiles`(= canonicalize 에 닿지 않았다). `..` 로 프로필 밖을 가리키는 어휘 우회도 같은 판정.
        //   RED(HEAD 1b614e47): 첫 경로가 canonicalize 실패로 session_file_missing.
        assert_eq!(
            report_outside_at(&d, Some(&home), "/net/cys-unreachable-host/projects/-w/s.jsonl", &r, now),
            Err("session_file_outside_profiles"),
            "프로필 밖 경로를 파일시스템으로 먼저 확인했다(응답 없는 마운트면 무기한 대기)"
        );
        assert_eq!(
            report_outside_at(&d, Some(&home), &p(".claude-3/projects/../../elsewhere/projects/-w/s.jsonl"), &r, now),
            Err("session_file_outside_profiles"),
            "`..` 어휘 우회"
        );
        assert_eq!(report_outside_at(&d, Some(&home), "relative/projects/s.jsonl", &r, now), Err("session_file_outside_profiles"));
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
        // ★0.14.42 agy 자동 연결: 윈도우 예시는 **따옴표 없는** 정슬래시 경로다 — command 안의 따옴표가 글자 그대로 넘어가
        //   경로가 깨졌다는 공개 보고 둘(agy_statusline 모듈 머리)이 있어, 종전 `bash \"C:/…\"` 예시를 거둔다. 예시 문자열은
        //   코드가 만드는 명령과 같아야 한다(doctor 가 같은 함수로 이 컴퓨터용 명령을 보여 준다).
        // ★0.14.45: 윈도우 명령은 `.cmd` 래퍼 직접 실행(역슬래시 · 따옴표 없음) — 매뉴얼의 실제 명령 줄과 JSON 예시(역슬래시
        //   이스케이프) 둘 다 코드가 만드는 명령과 같아야 한다.
        let win = cys::agy_statusline::link_command_for("C:/Users/x/.cys/pack", true, true)
            .expect("윈도우 자동 연결 명령")
            .replace(r"\x\", r"\<you>\");
        assert!(para.contains(&win), "윈도우 명령이 코드의 명령({win})과 다르다:\n{para}");
        let win_json = serde_json::to_string(&win).unwrap();
        assert!(para.contains(&win_json), "윈도우 JSON 예시가 코드의 명령({win_json})과 다르다");
        assert!(!para.contains(r#"bash \"C:/"#), "따옴표 두른 윈도우 예시가 남았다");
        assert!(!para.contains("\"command\": \"bash C:/"), "옛 bash 윈도우 예시가 권장 모양으로 남았다");
        assert!(para.contains("아직 실제로 확인하지 못했습니다"), "윈도우 미검증 고지가 없다");
        assert!(para.contains("Windows 에서는 cys 가 agy 내부 서버를 아예 찾을 수 없어"), "W5 고지가 없다");
    }

    /// ★0.14.42 agy 상태줄 자동 연결(오너 승인 2026-09-24) — 매뉴얼이 코드의 계약을 그대로 적는다: 넣는 명령(표지 포함)·
    /// 비었거나 없을 때만 · 사용자 설정 불가침 · 되돌리기 노브 둘 · 윈도우는 시험 실행 통과 시 연결(0.14.45) · 다시 넣지 않음 ·
    /// 환경변수 표 등재.
    #[test]
    fn manual_documents_the_agy_statusline_autolink_contract() {
        use cys::agy_statusline as agy;
        let manual = include_str!("../../../USER-MANUAL.md");
        let start = manual.find("**Antigravity(agy) 값**").expect("agy 값 문단");
        let end = manual[start..].find("- 갱신: 약 30초마다").map_or(manual.len(), |i| start + i);
        let para = &manual[start..end];
        let unix = agy::link_command_for("/Users/x/.cys/pack", false, true).unwrap().replace("/x/", "/<you>/");
        assert!(para.contains(&unix), "자동 연결 명령({unix})이 매뉴얼에 없다:\n{para}");
        assert!(para.contains(agy::MARKER) && para.contains("stack_with_default"));
        assert!(para.contains("비어 있거나 없으면"), "조건(비었거나 없을 때만)이 없다");
        assert!(para.contains("덮지 않습니다"), "사용자 설정 불가침 고지가 없다");
        assert!(para.contains(&format!("{}=0", agy::ENV_KNOB)) && para.contains(&format!("~/.cys/{}", agy::OFF_FILE)), "되돌리기 노브");
        assert!(para.contains("이제 Windows 도 자동으로 연결합니다") && para.contains("쓰기 전 시험 실행"), "윈도우 켬 + 시험 실행 고지");
        assert!(para.contains("파일을 한 바이트도 바꾸지 않고"), "시험 실패 시 무변경 고지");
        assert!(para.contains("다시 넣지 않습니다") && para.contains("cys doctor --fix"), "다시 넣지 않음·다시 연결 방법");
        assert!(para.contains(agy::BACKUP_SUFFIX), "백업 고지");
        // 재개(2026-09-24 15시): macOS 판 agy 역어셈블 사실(`sh -c` · 5초) · 래퍼 부재 시 미연결 · 윈도우 Git Bash 부재 시 동작
        assert!(para.contains("`sh -c`") && para.contains("5초"), "agy 가 상태줄을 부르는 방식(macOS 판 확인)이 없다");
        assert!(para.contains(&format!("hooks/{}`", agy::SCRIPT)) && para.contains(&format!("hooks/{}`)이 없을 때", agy::SCRIPT_CMD)), "래퍼 부재 시 넣지 않는다는 고지가 없다");
        assert!(para.contains("`cmd /c`") && para.contains("WSL"), "윈도우 cmd /c · 옛 bash 연결 안내가 없다");
        let env = &manual[manual.find("## 16. 환경변수 레퍼런스").expect("§16")..];
        assert!(env.contains(&format!("| `{}` |", agy::ENV_KNOB)), "§16 표에 노브가 없다");
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

    // ═════════ ★0.14.43(B3) 시나리오 — 경보 틱(`check_alerts_with`)에 시각을 주입해 본다(실시간 대기 없음) ═════════
    // 이 모듈은 신규 API 에 기대지 않는다(기존 공개 면 — Daemon · check_alerts_with · note_* · local_json · 버스 이벤트뿐) — 수정 전 코드에 그대로 붙여
    // 적색을 보이는 용도이기도 하다(WORKLOG 의 적색 로그). 픽스처는 전부 합성값이다(u-b3-a · a-b3@example.test 등) — 실계정 식별자 금지.
    mod b3_scenarios {
        use super::*;

        /// 설정 폴더 `dir`(home 상대)의 로그인을 (uuid, email)로 쓴다. 신원 파일 mtime 을 `bump` 로 매번 다르게 맞춘다(mtime 캐시가 옛 신원을 붙들지 않게).
        pub(super) fn b3_login(home: &Path, dir: &str, uuid: &str, email: &str, bump: u64) -> PathBuf {
            let f = home.join(dir).join(".claude.json");
            write(&f, &format!(r#"{{"oauthAccount":{{"accountUuid":"{uuid}","emailAddress":"{email}"}}}}"#));
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + bump);
            std::fs::File::options().write(true).open(&f).unwrap().set_modified(t).unwrap();
            home.join(dir)
        }

        /// 실제 PTY 좌석 하나(`sleep 30`) — 에이전트·설정 폴더를 지정한다. 신원 표가 보는 것은 이 두 필드(+관측 스냅샷)뿐이다.
        pub(super) fn b3_seat(d: &Arc<Daemon>, role: &str, agent: &str, cfg: Option<&Path>) -> Arc<crate::state::Surface> {
            let s = d
                .create_surface(None, Some("sleep 30".into()), None, Some(role.to_string()), 24, 80)
                .expect("create surface");
            d.surfaces.lock().unwrap().insert(s.id, s.clone());
            *s.agent_meta.lock().unwrap() = Some((agent.into(), agent.into()));
            *s.claude_config_dir.lock().unwrap() = cfg.map(|c| c.to_string_lossy().into_owned());
            s
        }

        /// 버스에서 이 이름의 이벤트 payload 들(seq0 이후).
        pub(super) fn b3_events(d: &Arc<Daemon>, seq0: u64, name: &str) -> Vec<Value> {
            d.bus.replay_after(seq0).into_iter().filter(|e| e["name"] == name).map(|e| e["payload"].clone()).collect()
        }

        /// `usage.alert_resolved` 이벤트의 (key, reason) 들 — 이 티켓이 다루는 한도 경보 키(`account_rate:`·`rate_limit:`)만. 프로세스 전역 게이트 신호(`alerts::GATE_SIGNALS`)와
        /// 같은 초에 만든 데몬이 나눠 쓰는 analytics DB 가 병렬 검체에서 새어 들어와 만드는 다른 종류의 키(node_liveness · repeated_failure)는 이 검체의 관심사가 아니다.
        pub(super) fn b3_resolved(d: &Arc<Daemon>, seq0: u64) -> Vec<(String, String)> {
            b3_events(d, seq0, "usage.alert_resolved")
                .iter()
                .map(|p| (p["key"].as_str().unwrap_or("").to_string(), p["reason"].as_str().unwrap_or("").to_string()))
                .filter(|(k, _)| k.starts_with("account_rate:") || k.starts_with("rate_limit:"))
                .collect()
        }

        pub(super) fn b3_fixture(tag: &str) -> (Arc<Daemon>, PathBuf, PathBuf, PathBuf) {
            let dir = tmp(&format!("b3-{tag}-daemon"));
            let home = tmp(&format!("b3-{tag}-home"));
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let f = b3_login(&home, ".cys/claude-b3", "u-b3-a", "a-b3@example.test", 1);
            (d, dir, home, f)
        }

        pub(super) const KEY_A: &str = "account_rate:a-b3@example.test:5h";
        pub(super) const KEY_B: &str = "account_rate:b-b3@example.test:5h";

        /// ★핵심 시나리오(오너 제보): 좌석 폴더의 로그인을 A → B 로 바꾼 뒤에도 A 의 옛 값(99% · 리셋 전)이 30분마다 울렸다. 이제:
        /// 좌석이 A 로 99% 보고(리셋 +2h) → 발화 · 같은 폴더를 B 로 바꾸고 B 가 10% 보고 → +70·+1799초 A 유지(REMIND 재발행 0) · **+1801초 A 키 소멸** ·
        /// `usage.alert_resolved{reason:"stale"}` 정확히 1건 · B 는 임계 미만이라 0건 · 이후 틱에 해소 알림이 되풀이되지 않는다.
        #[test]
        fn b3_login_switch_retires_the_old_account_alert_after_thirty_minutes() {
            let (d, dir, home, f) = b3_fixture("s1");
            let cfg = crate::alerts::AlertConfig::default();
            let mut fired: HashMap<String, f64> = HashMap::new();
            let _seat = b3_seat(&d, "worker-b3", "claude", Some(&f));
            let t0 = crate::state::now_epoch();
            let seq0 = d.bus.latest_seq();
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1.0);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 1, "A 99% 가 경보를 내지 않았다(전제)");
            // 같은 폴더의 로그인을 B 로 바꾸고 B 가 10% 보고
            b3_login(&home, ".cys/claude-b3", "u-b3-b", "b-b3@example.test", 2);
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 10.0, Some(t0 + 9000.0))], "statusline", t0 + 5.0));
            for dt in [70.0, 600.0, 1799.0] {
                crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + dt);
            }
            assert_eq!(account_alerts(&d, seq0, KEY_A), 1, "관측 1800초 이내(+1799)인데 A 경보가 바뀌었다(재발행 또는 소멸)");
            assert!(b3_resolved(&d, seq0).is_empty(), "+1799초에 해소 알림이 나갔다: {:?}", b3_resolved(&d, seq0));
            // +1801초: 관측 나이 1801 > 1800 ∧ A 는 지금 쓰이지 않는다 → 입력에서 빠진다(REMIND 시각이 왔어도 재발행하지 않는다)
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1801.0);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 1, "+1801초에도 A 경보가 REMIND 로 다시 나갔다(옛 로그인의 99% 가 계속 울린다)");
            assert_eq!(b3_resolved(&d, seq0), vec![(KEY_A.to_string(), "stale".to_string())], "붙들림 해소 알림은 정확히 1건(reason=stale)");
            assert_eq!(account_alerts(&d, seq0, KEY_B), 0, "B 10% 는 임계 미만인데 경보가 났다");
            // 이후 틱: 해소 알림이 되풀이되지 않는다
            for dt in [1831.0, 1861.0, 3000.0] {
                crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + dt);
            }
            assert_eq!(b3_resolved(&d, seq0).len(), 1, "붙들린 키의 해소 알림이 되풀이됐다");
            assert_eq!(account_alerts(&d, seq0, KEY_A), 1);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 음성 대조: 로그인을 바꾸지 않으면(+2h 유휴) A 경보는 리셋까지 유지된다(REMIND 리마인드도 종전대로). 리셋이 지나면 '해소'(cleared)다.
        #[test]
        fn b3_unchanged_login_keeps_the_account_alert_until_its_window_resets() {
            let (d, dir, home, f) = b3_fixture("s2");
            let cfg = crate::alerts::AlertConfig::default();
            let mut fired: HashMap<String, f64> = HashMap::new();
            let _seat = b3_seat(&d, "worker-b3", "claude", Some(&f));
            let t0 = crate::state::now_epoch();
            let seq0 = d.bus.latest_seq();
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
            for dt in [1.0, 70.0, 1799.0] {
                crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + dt);
            }
            assert_eq!(account_alerts(&d, seq0, KEY_A), 1);
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1801.0);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 2, "사용 중인 계정의 REMIND 리마인드가 사라졌다");
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 3601.0);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 3, "사용 중인 계정의 2회차 리마인드가 사라졌다(관측이 아무리 오래돼도 사용 중이면 유지)");
            assert!(b3_resolved(&d, seq0).is_empty(), "사용 중 계정에 해소 알림이 나갔다");
            // 리셋(+2h)이 지나면 창이 죽는다 — 신선도가 아니라 해소
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 7201.0);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 3);
            assert_eq!(b3_resolved(&d, seq0), vec![(KEY_A.to_string(), "cleared".to_string())]);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 판정 불가 = 사용 중: 좌석 폴더의 신원 파일이 사라져 읽을 수 없으면(결측 ≠ 미사용) +1801초에도 경보를 유지한다(진짜 한도 경보를 지우는 쪽으로 오판하지 않는다).
        #[test]
        fn b3_unreadable_identity_keeps_the_account_alert() {
            let (d, dir, home, f) = b3_fixture("s3");
            let cfg = crate::alerts::AlertConfig::default();
            let mut fired: HashMap<String, f64> = HashMap::new();
            let _seat = b3_seat(&d, "worker-b3", "claude", Some(&f));
            let t0 = crate::state::now_epoch();
            let seq0 = d.bus.latest_seq();
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1.0);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 1);
            std::fs::remove_file(f.join(".claude.json")).unwrap();
            // ★R1F-US: 워치독은 신원 파일을 읽지 않는다 — '판독 불가'가 캐시에 실리려면 RPC 의 읽기-통과 조회(60초 하한이 지난 뒤)가 한 번 지나가야 한다(이 검체의 전제를 채운다 · 단언은 그대로).
            assert_eq!(seat_identity_view_in(&d, Some(&home), t0 + 61.0).current_for(_seat.id), None, "전제: 읽기-통과 조회가 지워진 신원을 판독 불가로 실었다");
            for dt in [70.0, 1799.0, 1801.0] {
                crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + dt);
            }
            assert_eq!(account_alerts(&d, seq0, KEY_A), 2, "신원 판독 불가인데 +1801초에 A 경보가 사라졌다(REMIND 도 나가야 한다)");
            assert!(b3_resolved(&d, seq0).is_empty(), "판정 불가 계정에 해소 알림이 나갔다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 재시작 복원: 스냅샷으로 복원된 A(관측 2시간 전 · 리셋 전 · 사용 중 아님)는 발화 0 — 사용 중이면(그 폴더를 쓰는 살아 있는 좌석이 생기면) 발화한다.
        #[test]
        fn b3_restored_snapshot_without_a_user_stays_silent_and_with_a_user_fires() {
            let dir = tmp("b3-s4-daemon");
            let home = tmp("b3-s4-home");
            let f = b3_login(&home, ".cys/claude-b3", "u-b3-a", "a-b3@example.test", 1);
            let sock = dir.join("cysd.sock");
            let cfg = crate::alerts::AlertConfig::default();
            let t0 = crate::state::now_epoch();
            {
                let d1 = crate::state::Daemon::new(sock.clone());
                assert!(note_rate_for_profile_at(&d1, Some(&home), &f, &[rw("5h", 97.0, Some(t0 + 7200.0))], "statusline", t0 - 7200.0));
            }
            let d2 = crate::state::Daemon::new(sock);
            restore_from_snapshots(&d2, t0);
            let mut fired: HashMap<String, f64> = HashMap::new();
            let seq0 = d2.bus.latest_seq();
            crate::governance::check_alerts_with(&d2, &mut fired, &cfg, t0 + 1.0);
            assert_eq!(account_alerts(&d2, seq0, KEY_A), 0, "사용 중이 아닌 2시간 전 복원값이 발화했다");
            assert!(b3_resolved(&d2, seq0).is_empty(), "발화한 적 없는 키에 해소 알림이 나갔다");
            let _seat = b3_seat(&d2, "worker-b3-r", "claude", Some(&f));
            // ★R1F-US: 워치독은 캐시만 본다 — 좌석 폴더의 현재 신원(A)을 읽기-통과 조회로 먼저 실어, '일치하는 좌석이 있어서 발화'를 검체가 계속 본다(캐시가 비었어도 판정 불가 = 사용 중이라 발화는 하지만 이유가 다르다).
            assert_eq!(seat_identity_view_in(&d2, Some(&home), t0 + 30.0).current_for(_seat.id), Some("u-b3-a"), "전제: 좌석 폴더의 현재 신원이 캐시에 실렸다");
            crate::governance::check_alerts_with(&d2, &mut fired, &cfg, t0 + 31.0);
            assert_eq!(account_alerts(&d2, seq0, KEY_A), 1, "그 계정을 쓰는 좌석이 있는데 복원값이 발화하지 않았다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 깜빡임 금지: 신선도 규칙으로 빠진 키는 `fired` 시각을 유지한다 — 다시 적격이 돼도 마지막 발행에서 REMIND 가 지나기 전에는 재발행하지 않고,
        /// REMIND 가 지나면 정상 재발행한다. (최악의 모양 — 방금 리마인드한 직후에 빠지는 경우 — 을 `fired` 시각 주입으로 재현한다.)
        #[test]
        fn b3_stale_hold_keeps_the_fired_time_so_re_eligibility_inside_remind_does_not_refire() {
            let (d, dir, home, fa) = b3_fixture("s5");
            let cfg = crate::alerts::AlertConfig::default();
            // 좌석은 B 를 쓴다 — A 는 지금 쓰이지 않는다
            let fb = b3_login(&home, ".cys/claude-b3-b", "u-b3-b", "b-b3@example.test", 2);
            let _seat = b3_seat(&d, "worker-b3", "claude", Some(&fb));
            let t0 = crate::state::now_epoch();
            let seq0 = d.bus.latest_seq();
            // ★R1F-US: 워치독은 신원 파일을 읽지 않고 캐시만 본다 — 좌석 폴더(fb)의 현재 신원(B)을 RPC 의 읽기-통과 조회로 먼저 캐시에 싣는다(이 검체의 전제를 채운다 · 단언은 그대로).
            assert_eq!(seat_identity_view_in(&d, Some(&home), t0).current_for(_seat.id), Some("u-b3-b"), "전제: 좌석 폴더의 현재 신원이 캐시에 실렸다");
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 99.0, Some(t0 + 20_000.0))], "statusline", t0));
            let mut fired: HashMap<String, f64> = HashMap::new();
            fired.insert(KEY_A.to_string(), t0 + 1790.0); // A 가 마지막으로 발행된 시각 — REMIND 1800초 창이 아직 열려 있다
            // 관측 1801초 → 부적격(붙들림): fired 시각 유지 · 해소 알림(stale) 1건 · 재발행 0
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1801.0);
            assert_eq!(fired.get(KEY_A), Some(&(t0 + 1790.0)), "붙든 키의 fired 시각이 지워졌다/바뀌었다(재무장 → 깜빡임)");
            assert_eq!(b3_resolved(&d, seq0), vec![(KEY_A.to_string(), "stale".to_string())]);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 0);
            // A 가 다시 적격(새 관측 · 나이 0)이 됐다 — 마지막 발행(+1790)에서 30초뿐이라 재발행하지 않는다
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 99.0, Some(t0 + 20_000.0))], "statusline", t0 + 1810.0));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1820.0);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 0, "REMIND 안에 다시 적격이 되자 재발행했다(깜빡임)");
            // REMIND(마지막 발행 + 1800 = +3590)가 지나면 종전처럼 한 번 낸다 — 음성 대조(리마인드가 영영 죽지 않았다)
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 3590.0);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 1, "REMIND 뒤에도 재발행하지 않는다(붙들림이 리마인드를 죽였다)");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 해소 알림의 사유: 신선도가 아니라 값이 내려가 풀리면 `cleared` — 정확히 1건. (같은 리셋 창의 최댓값은 마지막 확인 뒤 PEAK_HOLD 1800초까지 쥔다.)
        #[test]
        fn b3_resolved_reason_is_cleared_when_the_value_drops() {
            let (d, dir, home, f) = b3_fixture("s6");
            let cfg = crate::alerts::AlertConfig::default();
            let mut fired: HashMap<String, f64> = HashMap::new();
            let _seat = b3_seat(&d, "worker-b3", "claude", Some(&f));
            let t0 = crate::state::now_epoch();
            let seq0 = d.bus.latest_seq();
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1.0);
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 10.0, Some(t0 + 7200.0))], "statusline", t0 + 1900.0));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1910.0);
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1940.0);
            assert_eq!(b3_resolved(&d, seq0), vec![(KEY_A.to_string(), "cleared".to_string())], "값이 내려가 풀린 키의 해소 알림이 1건이 아니거나 사유가 다르다");
            assert!(!fired.contains_key(KEY_A), "해소된 키가 fired 에 남았다(재무장 실패)");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }
    }

    // ═════════ ★0.14.43(B3) 단위 — 순수 판정 표 · 좌석 신원 표 · 60초 캐시 · 계정 조회 가산 키 ═════════
    // 픽스처는 전부 합성값이다(u-b3-a · a-b3@example.test 등) — 실계정 식별자 금지. 시각은 주입한다(실시간 대기 없음).
    mod b3_units {
        use super::b3_scenarios::{b3_events, b3_fixture, b3_login, b3_resolved, b3_seat, KEY_A};
        use super::*;

        fn view(folders: &[(&str, Option<&str>)], unknown: bool, codex: bool, gemini: bool, ok: bool) -> SeatIdentityView {
            SeatIdentityView {
                claude_folders: folders.iter().map(|(f, w)| (f.to_string(), w.map(str::to_string))).collect(),
                claude_folder_unknown: unknown,
                agents_alive: AgentsAlive { claude: !folders.is_empty() || unknown, codex, gemini },
                collect_ok: ok,
                seats: Vec::new(),
            }
        }

        /// 계정 축 `in_use` 표 — claude(일치 · 불일치 · 폴더 미상 · 판독 실패 · 좌석 0) · codex·agy(생존 유무) · 그 밖 provider · 수집 실패.
        #[test]
        fn b3_account_in_use_table() {
            let (a, b) = ("u-b3-a", "u-b3-b");
            let cases: Vec<(&str, SeatIdentityView, &str, &str, Option<bool>)> = vec![
                ("claude 일치", view(&[("/f1", Some(a))], false, false, false, true), "claude", a, Some(true)),
                ("claude 전원 판독 · 아무도 이 계정이 아님", view(&[("/f1", Some(a))], false, false, false, true), "claude", b, Some(false)),
                ("claude 두 좌석 두 계정 — a", view(&[("/f1", Some(a)), ("/f2", Some(b))], false, false, false, true), "claude", a, Some(true)),
                ("claude 두 좌석 두 계정 — b", view(&[("/f1", Some(a)), ("/f2", Some(b))], false, false, false, true), "claude", b, Some(true)),
                ("claude 폴더 미상 좌석 + 다른 좌석이 이 계정 → 참이 이긴다", view(&[("/f1", Some(a))], true, false, false, true), "claude", a, Some(true)),
                ("claude 폴더 미상 좌석 · 이 계정 아님 → 판정 불가", view(&[("/f1", Some(a))], true, false, false, true), "claude", b, None),
                ("claude 폴더 미상 좌석만 → 판정 불가", view(&[], true, false, false, true), "claude", a, None),
                ("claude 신원 판독 실패 좌석 · 이 계정 아님 → 모든 claude 계정 판정 불가", view(&[("/f1", Some(a)), ("/f2", None)], false, false, false, true), "claude", b, None),
                ("claude 신원 판독 실패 좌석이 있어도 다른 좌석이 이 계정이면 참", view(&[("/f1", Some(a)), ("/f2", None)], false, false, false, true), "claude", a, Some(true)),
                ("claude 판독 실패(None)가 빈 account_id 와 '같다'로 읽히지 않는다(None==None 함정)", view(&[("/f1", None)], false, false, false, true), "claude", "", None),
                ("claude 살아 있는 좌석 0 → 미사용", view(&[], false, false, false, true), "claude", a, Some(false)),
                ("codex 좌석 있음", view(&[], false, true, false, true), "codex", "default", Some(true)),
                ("codex 좌석 없음", view(&[], false, false, true, true), "codex", "default", Some(false)),
                ("agy 좌석 있음", view(&[], false, false, true, true), "antigravity", "default", Some(true)),
                ("agy 좌석 없음", view(&[], false, true, false, true), "antigravity", "default", Some(false)),
                ("그 밖 provider(선언 계정)", view(&[("/f1", Some(a))], false, true, true, true), "grok", "default", None),
                ("수집 실패 — claude", view(&[("/f1", Some(a))], false, true, true, false), "claude", a, None),
                ("수집 실패 — codex", view(&[("/f1", Some(a))], false, true, true, false), "codex", "default", None),
                ("수집 실패 — agy", view(&[("/f1", Some(a))], false, true, true, false), "antigravity", "default", None),
            ];
            for (what, v, provider, account, want) in cases {
                assert_eq!(account_in_use(provider, account, &v), want, "{what}");
            }
            // 기본값(Default)은 '수집 실패' — 전부 모름
            assert_eq!(account_in_use("claude", a, &SeatIdentityView::default()), None);
        }

        /// 좌석 축 `seat_in_use` 표 — (rate_account, 현재 신원) 조합 · 비 claude · 종료 좌석. 3값판이 status 의 null 을 만든다.
        #[test]
        fn b3_seat_in_use_table() {
            let t = |ra: Option<&str>, cur: Option<&str>, alive: bool, claude: bool| {
                (seat_in_use(ra, cur, alive, claude), seat_in_use_tri(ra, cur, alive, claude))
            };
            assert_eq!(t(None, None, true, true), (true, None), "None·None → 사용 중(판정 불가)");
            assert_eq!(t(Some("A"), None, true, true), (true, None), "신원 판독 불가 → 사용 중");
            assert_eq!(t(None, Some("B"), true, true), (true, None), "귀속 없음 → 사용 중");
            assert_eq!(t(Some("A"), Some("A"), true, true), (true, Some(true)));
            assert_eq!(t(Some("A"), Some("B"), true, true), (false, Some(false)), "둘 다 Some 이고 다를 때만 미사용");
            assert_eq!(t(Some("A"), Some("B"), true, false), (true, Some(true)), "claude 가 아닌 좌석은 항상 사용 중");
            assert_eq!(t(None, None, true, false), (true, Some(true)));
            assert_eq!(t(Some("A"), Some("A"), false, true), (false, Some(false)), "종료 좌석은 미사용");
            assert_eq!(t(None, None, false, false), (false, Some(false)));
        }

        /// 경보 적격 표 — stale 0(끔) · 경계(1799·1800·1801) · 판정 불가 · 리셋 지난 창 · 비유한 나이.
        #[test]
        fn b3_alert_eligible_table() {
            let e = alert_eligible;
            assert!(!e(false, Some(true), 0.0, 1800.0), "리셋 지난 창은 어떤 경우에도 부적격");
            assert!(!e(false, None, 0.0, 0.0), "규칙을 꺼도 리셋 지난 창은 부적격");
            assert!(e(true, Some(false), 99_999.0, 0.0), "stale 0 = 이 규칙 끔 → live 만");
            assert!(e(true, Some(false), 1799.0, 1800.0));
            assert!(e(true, Some(false), 1800.0, 1800.0), "관측 나이 ≤ 1800 — 경계는 적격");
            assert!(!e(true, Some(false), 1801.0, 1800.0), "미사용 ∧ 1801초 → 부적격");
            assert!(e(true, None, 99_999.0, 1800.0), "판정 불가 = 사용 중 — 나이와 무관하게 적격");
            assert!(e(true, Some(true), 99_999.0, 1800.0), "사용 중이면 나이와 무관하게 적격");
            assert!(e(true, Some(false), f64::NAN, 1800.0), "비유한 나이는 '넘지 않음' — 경보를 지우는 쪽으로 오판하지 않는다");
            assert!(e(true, Some(false), -5.0, 1800.0), "시계 역행(음수 나이)도 적격");
        }

        /// 노브 `CYS_ACCOUNT_ALERT_STALE_SECS` 의 순수 파서 — 미설정·빈 값·쓰레기·음수·비유한 = 기본 1800(규칙은 켜진 채) · `0` = 끔.
        #[test]
        fn b3_stale_knob_parser() {
            assert_eq!(parse_stale_secs(None), 1800.0);
            for junk in ["", "  ", "abc", "-1", "NaN", "inf", "-inf", "1800abc", "1,800"] {
                assert_eq!(parse_stale_secs(Some(junk)), 1800.0, "쓰레기 값 {junk:?} 은 기본값");
            }
            assert_eq!(parse_stale_secs(Some("0")), 0.0, "0 = 이 규칙 끔");
            assert_eq!(parse_stale_secs(Some(" 900 ")), 900.0);
            assert_eq!(parse_stale_secs(Some("1e3")), 1000.0);
            assert_eq!(parse_stale_secs(Some("0.5")), 0.5);
        }

        /// 실제 좌석으로 만든 신원 표 — 살아 있는 claude 좌석의 폴더 신원 · 에이전트 생존 · 폴더 미상 · transcript 경로 폴백 · 종료 좌석 제외.
        #[test]
        fn b3_seat_identity_view_reads_the_current_folder_login_of_live_claude_seats() {
            let dir = tmp("b3-u5-daemon");
            let home = tmp("b3-u5-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let f1 = b3_login(&home, ".cys/claude-b3-1", "u-b3-a", "a-b3@example.test", 1);
            let f2 = b3_login(&home, ".cys/claude-b3-2", "u-b3-b", "b-b3@example.test", 2);
            let s1 = b3_seat(&d, "worker-b3-1", "claude", Some(&f1));
            let s2 = b3_seat(&d, "worker-b3-2", "claude", Some(&f2));
            let s_codex = b3_seat(&d, "reviewer-b3-codex", "codex", None);
            // 에이전트 증거가 없는 셸 좌석 — claude 판정에 끼지 않는다
            let shell = d
                .create_surface(None, Some("sleep 30".into()), None, Some("shell-b3".into()), 24, 80)
                .expect("create surface");
            d.surfaces.lock().unwrap().insert(shell.id, shell.clone());
            let t0 = crate::state::now_epoch();
            let fs = |p: &Path| p.to_string_lossy().into_owned();
            let v = seat_identity_view_in(&d, Some(&home), t0);
            assert!(v.collect_ok && !v.claude_folder_unknown);
            assert_eq!(
                v.claude_folders,
                vec![(fs(&f1), Some("u-b3-a".to_string())), (fs(&f2), Some("u-b3-b".to_string()))]
            );
            assert_eq!(v.agents_alive, AgentsAlive { claude: true, codex: true, gemini: false });
            assert_eq!(v.current_for(s1.id), Some("u-b3-a"));
            assert_eq!(v.current_for(s2.id), Some("u-b3-b"));
            assert_eq!((v.current_for(s_codex.id), v.current_for(shell.id)), (None, None));
            assert!(v.seats.iter().find(|s| s.surface_id == s1.id).is_some_and(|s| s.known && s.folder.as_deref() == Some(fs(&f1).as_str())));
            assert!(v.seats.iter().any(|s| s.surface_id == shell.id && s.agent.is_none() && !s.known), "셸 좌석이 표에서 빠졌거나 claude 로 읽혔다");
            assert_eq!(account_in_use("claude", "u-b3-a", &v), Some(true));
            assert_eq!(account_in_use("claude", "u-b3-b", &v), Some(true));
            assert_eq!(account_in_use("claude", "u-b3-zzz", &v), Some(false), "전원 판독됐고 아무도 아닌 계정은 미사용");
            assert_eq!(account_in_use("codex", "default", &v), Some(true));
            assert_eq!(account_in_use("antigravity", "default", &v), Some(false));
            // 공개 래퍼(지금 시각·실제 홈)도 같은 표를 낸다 — 같은 60초 캐시를 쓰므로 방금 만든 표와 같다
            let live = seat_identity_view(&d);
            assert!(live.collect_ok);
            assert_eq!(live.claude_folders, v.claude_folders);
            assert_eq!(live.agents_alive, v.agents_alive);
            // 폴더 미상 claude 좌석(설정 폴더 기록도 transcript 경로도 없다) → 모든 claude 계정 판정 불가(참이 이기는 계정은 제외)
            let s3 = b3_seat(&d, "worker-b3-3", "claude", None);
            let v = seat_identity_view_in(&d, Some(&home), t0 + SEAT_IDENT_CACHE_SECS);
            assert!(v.claude_folder_unknown);
            assert_eq!(account_in_use("claude", "u-b3-zzz", &v), None);
            assert_eq!(account_in_use("claude", "u-b3-a", &v), Some(true));
            // 그 좌석이 transcript 경로를 관측하면(session_file → 프로필 폴더) 폴더를 안다
            *s3.observed_usage.lock().unwrap() = Some(ObservedUsage {
                agent: "claude".into(),
                ctx_tokens: None,
                ctx_window: None,
                ctx_pct: None,
                rate: vec![],
                source: "statusline".into(),
                session_file: f1.join("projects/-w/s.jsonl").to_string_lossy().into_owned(),
                updated_at: t0,
                rate_observed_at: 0.0,
                rate_account: None,
            });
            let v = seat_identity_view_in(&d, Some(&home), t0 + SEAT_IDENT_CACHE_SECS);
            assert!(!v.claude_folder_unknown, "transcript 경로 폴백이 폴더를 알려 주지 않았다");
            assert_eq!(v.current_for(s3.id), Some("u-b3-a"));
            assert_eq!(account_in_use("claude", "u-b3-zzz", &v), Some(false));
            // 종료된 좌석은 표에서 빠진다
            for s in [&s1, &s3] {
                s.exited.store(true, Ordering::Relaxed);
            }
            let v = seat_identity_view_in(&d, Some(&home), t0 + SEAT_IDENT_CACHE_SECS);
            assert_eq!(v.claude_folders, vec![(fs(&f2), Some("u-b3-b".to_string()))]);
            assert_eq!(account_in_use("claude", "u-b3-a", &v), Some(false), "종료된 좌석의 로그인이 '사용 중'으로 남았다");
            assert_eq!(v.current_for(s1.id), None);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 60초 하한 캐시 — 같은 폴더는 60초 안에 다시 stat 하지 않는다(연속 호출 N회에 실판독 1회). 계측(`reads`·`hits`)과 **관측**(파일을 지워도 60초 안에는 옛 신원)
        /// 둘로 핀한다. 시계가 뒤로 가면 캐시가 붙지 않는다.
        #[test]
        fn b3_view_checks_each_folder_at_most_once_per_sixty_seconds() {
            let dir = tmp("b3-u6-daemon");
            let home = tmp("b3-u6-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let f1 = b3_login(&home, ".cys/claude-b3-1", "u-b3-a", "a-b3@example.test", 1);
            let s1 = b3_seat(&d, "worker-b3-1", "claude", Some(&f1));
            let t0 = crate::state::now_epoch();
            for i in 0..12 {
                let v = seat_identity_view_in(&d, Some(&home), t0 + f64::from(i) * 4.0);
                assert_eq!(v.current_for(s1.id), Some("u-b3-a"));
            }
            {
                let c = d.seat_ident_cache.lock().unwrap();
                assert_eq!((c.reads, c.hits), (1, 11), "연속 12회 호출에 실판독이 1회가 아니다(status 폴링이 stat 을 늘린다)");
            }
            // 파일을 지워도 60초 안에는 옛 신원(실제로 다시 보지 않았다는 관측) · 60초가 되면 다시 본다
            std::fs::remove_file(f1.join(".claude.json")).unwrap();
            assert_eq!(seat_identity_view_in(&d, Some(&home), t0 + 59.0).current_for(s1.id), Some("u-b3-a"));
            assert_eq!(seat_identity_view_in(&d, Some(&home), t0 + 60.0).current_for(s1.id), None);
            assert_eq!(d.seat_ident_cache.lock().unwrap().reads, 2);
            // 시계가 뒤로 가면(now < 확인 시각) 만료로 본다 — 캐시가 영구히 붙지 않는다
            let _ = seat_identity_view_in(&d, Some(&home), t0 - 5.0);
            assert_eq!(d.seat_ident_cache.lock().unwrap().reads, 3);
            // 좌석이 쓰지 않게 돼도 60초 하한 안의 항목은 걷지 않는다 — 같은 캐시를 쓰는 다른 소비자(알려진 프로필 폴더 전체 · `folder_identity`)의 항목이 신원 표 빌드에 지워지면 그 소비자가 폴더를 매번 stat 한다
            s1.exited.store(true, Ordering::Relaxed);
            let _ = seat_identity_view_in(&d, Some(&home), t0 + 200.0);
            assert_eq!(d.seat_ident_cache.lock().unwrap().entries.len(), 1, "좌석이 쓰지 않는다고 하한 안의 항목을 걷었다");
            let fo = b3_login(&home, ".cys/claude-b3-other", "u-b3-o", "o-b3@example.test", 3);
            let key = fo.to_string_lossy().into_owned();
            assert_eq!(folder_identity(&d, Some(&home), &key, t0 + 200.0).as_deref(), Some("u-b3-o"));
            let reads = d.seat_ident_cache.lock().unwrap().reads;
            let _ = seat_identity_view_in(&d, Some(&home), t0 + 230.0);
            assert_eq!(folder_identity(&d, Some(&home), &key, t0 + 231.0).as_deref(), Some("u-b3-o"));
            assert_eq!(d.seat_ident_cache.lock().unwrap().reads, reads, "신원 표 빌드가 다른 소비자의 캐시 항목을 걷어 다시 stat 했다");
            // 확인한 지 10분이 지난 항목은 걷는다(크기 유계)
            let _ = seat_identity_view_in(&d, Some(&home), t0 + 900.0);
            assert!(d.seat_ident_cache.lock().unwrap().entries.is_empty(), "낡은 폴더 항목이 캐시에 남았다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// FIFO 신원 파일(읽으면 막히는 파일)을 가진 **좌석 폴더**가 신원 표·계정 조회(`local_json`)·경보 틱을 세우지 않는다(R4-F2 패턴 — 새 경로도 덮는다).
        /// 신원 판독 불가 = 판정 불가 = 사용 중이므로 +1801초에도 A 경보는 발화한다(지우는 쪽 오판 금지).
        #[cfg(unix)]
        #[test]
        fn b3_fifo_identity_in_a_seat_folder_blocks_nothing() {
            let dir = tmp("b3-u7-daemon");
            let home = tmp("b3-u7-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let fa = b3_login(&home, ".cys/claude-b3", "u-b3-a", "a-b3@example.test", 1);
            let ff = home.join(".cys/claude-b3-fifo");
            let fifo = ff.join(".claude.json");
            mkfifo(&fifo);
            let _seat = b3_seat(&d, "worker-b3", "claude", Some(&ff));
            let t0 = crate::state::now_epoch();
            let seq0 = d.bus.latest_seq();
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
            let (tx, rx) = std::sync::mpsc::channel();
            {
                let (d, home) = (d.clone(), home.clone());
                std::thread::spawn(move || {
                    let v = seat_identity_view_in(&d, Some(&home), t0 + 1.0);
                    let rows = local_json(&d, t0 + 1.0);
                    let mut fired: HashMap<String, f64> = HashMap::new();
                    crate::governance::check_alerts_with(&d, &mut fired, &crate::alerts::AlertConfig::default(), t0 + 1801.0);
                    let _ = tx.send((v, rows.as_array().map(Vec::len), fired.contains_key(KEY_A)));
                });
            }
            let out = rx.recv_timeout(std::time::Duration::from_secs(5));
            release_fifo(&fifo);
            let (v, rows_len, fired_a) = out.expect("FIFO 신원이 신원 표·계정 조회·경보 틱을 멈췄다");
            assert!(v.collect_ok);
            assert_eq!(v.claude_folders, vec![(ff.to_string_lossy().into_owned(), None)], "FIFO 신원은 판독 실패(None)여야 한다");
            assert_eq!(account_in_use("claude", "u-b3-a", &v), None);
            assert_eq!(rows_len, Some(1), "계정 조회가 행을 내지 못했다");
            assert_eq!((fired_a, account_alerts(&d, seq0, KEY_A)), (true, 1), "판정 불가인데 +1801초 A 경보가 발화하지 않았다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// `usage.accounts` 행의 가산 키 — `in_use`(true/false/null) · `rate_observed_at`(경보 입력의 관측 시각 · 없으면 null) · `rate[].alert_eligible`.
        /// 기존 키는 불변. 좌석 목록 수집 실패(락 오염)면 `in_use` 는 전부 null(실패 방향 = 경보 유지).
        #[test]
        fn b3_local_json_rows_carry_in_use_rate_observed_at_and_alert_eligible() {
            let (d, dir, home, fa) = b3_fixture("u8");
            let fb = b3_login(&home, ".cys/claude-b3-b", "u-b3-b", "b-b3@example.test", 2);
            let seat = b3_seat(&d, "worker-b3", "claude", Some(&fa));
            let outside_sess = outside_profile(&home, ".claude-9", "u-b3-c", "c-b3@example.test");
            let t0 = crate::state::now_epoch();
            // A: 지금 보고(좌석이 쓰는 로그인) · B: 2시간 전 관측(좌석이 쓰지 않는다 · 리셋 전)
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 97.0, Some(t0 + 7200.0))], "statusline", t0));
            assert!(note_rate_for_profile_at(&d, Some(&home), &fb, &[rw("5h", 90.0, Some(t0 + 3600.0))], "statusline", t0 - 7200.0));
            // codex: 좌석 없음 · 관측은 1분 전(30분 안)
            note_rate(&d, "codex", "", &[rw("7d", 40.0, Some(t0 + 86400.0))], "rollout", t0 - 60.0);
            // 창 밖 표시용 값만 있는 계정 C(경보 입력 없음)
            assert_eq!(report_outside_at(&d, Some(&home), &outside_sess, &[rw("5h", 50.0, None)], t0), Ok(OutsideOutcome::Accepted));
            let rows = local_json(&d, t0 + 1.0);
            let row = |id: &str| rows.as_array().unwrap().iter().find(|r| r["account_id"] == id).cloned().unwrap_or_else(|| panic!("행 {id} 없음: {rows}"));
            let (a, b, c, cx) = (row("u-b3-a"), row("u-b3-b"), row("u-b3-c"), row("default"));
            assert_eq!((a["in_use"].clone(), a["rate_observed_at"].clone(), a["rate"][0]["alert_eligible"].clone()), (json!(true), json!(t0), json!(true)));
            assert_eq!((b["in_use"].clone(), b["rate_observed_at"].clone(), b["rate"][0]["alert_eligible"].clone()), (json!(false), json!(t0 - 7200.0), json!(false)), "미사용 · 2시간 전 관측은 부적격");
            assert_eq!((c["in_use"].clone(), c["rate_observed_at"].clone(), c["rate"][0]["alert_eligible"].clone()), (json!(false), Value::Null, json!(false)), "창 밖 값만 있는 계정은 경보 입력이 아니다");
            assert_eq!((cx["provider"].clone(), cx["in_use"].clone(), cx["rate"][0]["alert_eligible"].clone()), (json!("codex"), json!(false), json!(true)), "좌석 없는 codex · 관측 61초 → 나이 안이라 적격");
            // 기존 키 불변(가산뿐)
            for r in [&a, &b, &c, &cx] {
                for k in ["provider", "account_id", "label", "plan", "profiles", "rate", "updated_at", "stale_secs", "source", "adapter", "source_error"] {
                    assert!(r.get(k).is_some(), "기존 키 {k} 소실: {r}");
                }
                for k in ["label", "used_pct", "resets_at"] {
                    assert!(r["rate"][0].get(k).is_some(), "rate 원소의 기존 키 {k} 소실: {r}");
                }
            }
            assert_eq!(a["rate"][0]["used_pct"], json!(97.0));
            // codex 좌석이 생기면 사용 중
            let _codex_seat = b3_seat(&d, "reviewer-b3", "codex", None);
            let cx = local_json(&d, t0 + 2.0).as_array().unwrap().iter().find(|r| r["provider"] == "codex").cloned().unwrap();
            assert_eq!(cx["in_use"], json!(true));
            // 좌석 목록 수집 실패(surfaces 락 오염) → in_use 는 전부 null
            let dd = d.clone();
            assert!(std::thread::spawn(move || {
                let _g = dd.surfaces.lock().unwrap();
                panic!("surfaces 락 오염(검체)");
            })
            .join()
            .is_err());
            let rows = local_json(&d, t0 + 3.0);
            assert!(rows.as_array().unwrap().iter().all(|r| r["in_use"].is_null()), "수집 실패인데 in_use 가 판정됐다: {rows}");
            drop(seat);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 계정 축 행(`alert_rates_with`) — 관측 시각·나이·in_use·리셋·PEAK_HOLD 잔여가 실리고, 적격 판정이 행마다 붙는다. 복원분은 held 가 없다.
        #[test]
        fn b3_alert_rows_carry_age_in_use_reset_and_held() {
            let (d, dir, home, fa) = b3_fixture("u9");
            let _seat = b3_seat(&d, "worker-b3", "claude", Some(&fa));
            let t0 = crate::state::now_epoch();
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0 + 100.0));
            let view = seat_identity_view_in(&d, Some(&home), t0 + 400.0);
            let rows = alert_rates_with(&d, &view, t0 + 400.0, 1800.0);
            assert_eq!(rows.len(), 1);
            let r = &rows[0];
            assert_eq!((r.label.as_str(), r.win.as_str(), r.used_pct, r.in_use, r.eligible), ("a-b3@example.test", "5h", 99.0, Some(true), true));
            assert_eq!((r.observed_at, r.age_secs, r.resets_at), (t0 + 100.0, 300.0, Some(t0 + 7200.0)));
            assert_eq!(r.held_secs, Some(1500.0), "PEAK_HOLD 1800 − (마지막 확인 이후 300초)");
            // 같은 시각을 stale 0(끔)으로 — 적격은 그대로, 나이 큰 미사용 입력도 적격(0.14.42 동작)
            let v0 = SeatIdentityView { collect_ok: true, ..SeatIdentityView::default() };
            let off = alert_rates_with(&d, &v0, t0 + 3000.0, 0.0);
            assert!(!off.is_empty() && off.iter().all(|r| r.eligible), "노브 0 인데 부적격 행이 있다: {off:?}");
            let on = alert_rates_with(&d, &v0, t0 + 3000.0, 1800.0);
            assert!(!on.is_empty() && on.iter().all(|r| !r.eligible), "수집 성공 · 좌석 0 · 관측 2900초 → 부적격이어야 한다: {on:?}");
            // 리셋이 지난 창은 행이 아니다(신선도와 무관)
            assert!(alert_rates_with(&d, &view, t0 + 7201.0, 1800.0).is_empty());
            // 복원분(peak_at 없음)은 held 가 없다
            let rdir = tmp("b3-u9-restore");
            let sock = rdir.join("cysd.sock");
            {
                let d1 = crate::state::Daemon::new(sock.clone());
                assert!(note_rate_for_profile_at(&d1, Some(&home), &fa, &[rw("5h", 97.0, Some(t0 + 7200.0))], "statusline", t0 - 10.0));
            }
            let d2 = crate::state::Daemon::new(sock);
            restore_from_snapshots(&d2, t0);
            let rr = alert_rates_with(&d2, &v0, t0 + 1.0, 1800.0);
            assert_eq!((rr.len(), rr[0].held_secs), (1, None), "복원분에 held 가 있다");
            let _ = std::fs::remove_dir_all(&rdir);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 귀속 계정 반환판 — 좌석 설정 폴더의 보고 시점 신원을 돌려준다(None = 비었거나 판독 실패). bool 판은 종전 의미 그대로다.
        #[test]
        fn b3_note_rate_resolved_returns_the_attributed_account_and_bool_wrappers_keep_their_meaning() {
            let (d, dir, home, fa) = b3_fixture("u10");
            let t0 = crate::state::now_epoch();
            let sess = fa.join("projects/-w/s.jsonl").to_string_lossy().into_owned();
            write(&fa.join("projects/-w/s.jsonl"), "{}\n");
            assert_eq!(
                note_rate_for_profile_at_resolved(&d, Some(&home), &fa, &[rw("5h", 50.0, None)], "statusline", t0),
                Some("u-b3-a".to_string())
            );
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 50.0, None)], "statusline", t0 + 1.0), "bool 판은 귀속되면 true");
            // 공개 래퍼(실제 홈 · 지금 시각) — `_at` 판과 같은 의미: bool 판 · 귀속 계정 반환판
            assert!(note_rate_for_profile(&d, &fa, &[rw("5h", 51.0, None)], "statusline", crate::state::now_epoch()), "공개 bool 판은 귀속되면 true");
            assert_eq!(
                note_rate_for_profile_resolved(&d, &fa, &[rw("5h", 52.0, None)], "statusline", crate::state::now_epoch()),
                Some("u-b3-a".to_string()),
                "공개 귀속 계정 반환판"
            );
            assert_eq!(note_rate_at_resolved(&d, Some(&home), "claude", &sess, &[rw("5h", 50.0, None)], "statusline", t0 + 2.0), Some("u-b3-a".to_string()));
            assert_eq!(note_rate_at_resolved(&d, Some(&home), "codex", "", &[rw("7d", 50.0, None)], "rollout", t0 + 3.0), Some("default".to_string()));
            // 귀속 불가: 빈 rate · 신원 판독 불가 폴더 · 모르는 agent
            assert_eq!(note_rate_for_profile_at_resolved(&d, Some(&home), &fa, &[], "statusline", t0), None);
            assert!(!note_rate_for_profile_at(&d, Some(&home), &fa, &[], "statusline", t0), "bool 판은 빈 rate 면 false");
            let nofolder = home.join(".cys/claude-b3-none");
            assert_eq!(note_rate_for_profile_at_resolved(&d, Some(&home), &nofolder, &[rw("5h", 50.0, None)], "statusline", t0), None);
            assert!(!note_rate_for_profile_at(&d, Some(&home), &nofolder, &[rw("5h", 50.0, None)], "statusline", t0));
            assert_eq!(note_rate_at_resolved(&d, Some(&home), "nope", "", &[rw("5h", 50.0, None)], "statusline", t0), None);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 락 순서 배선 핀(소스): 신원 판독(파일 IO)은 어떤 락도 쥐지 않은 채 한다 — `local_json` 은 accounts 락을 잡기 **전에** 신원 표를 만들고, 좌석 표 복사
        /// (`collect_seat_rows` · surfaces 락 안)는 파일 IO 를 부르지 않으며, 신원 표 본체는 그 복사가 끝난 **뒤에** 폴더를 판독한다. (표를 락 안에서 만들면
        /// FIFO·느린 파일 하나가 사용량 RPC 전부와 워치독을 세운다 — R4-F2 전례.)
        #[test]
        fn b3_lock_order_wiring_pins() {
            let src = include_str!("accounts.rs");
            let func = |head: &str| -> String {
                let body = src.split(head).nth(1).unwrap_or_else(|| panic!("{head} 소실"));
                body[..body.find("\n}\n").unwrap_or_else(|| panic!("{head} 끝"))].to_string()
            };
            let lj = func("pub fn local_json(");
            let view = lj.find("seat_identity_view_at(").expect("local_json 이 신원 표를 만들지 않는다");
            let lock = lj.find("daemon.accounts.lock()").expect("local_json 의 accounts 락");
            assert!(view < lock, "local_json 이 accounts 락을 잡은 **뒤에** 신원 표를 만든다(파일 IO 가 락 안)");
            let c = func("fn collect_seat_rows(");
            assert!(c.contains("daemon.surfaces.lock()"), "좌석 표 복사가 surfaces 락을 잡지 않는다(핀 앵커 소실)");
            for io in ["folder_identity(", "claude_identity_unlocked(", "claude_identity_probe(", "peek_folder_ident(", "std::fs::", "read_identity_file(", "read_identity_outcome(", "identity_file_meta(", "accounts.lock("] {
                assert!(!c.contains(io), "surfaces 락 안(좌석 표 복사)에서 {io} 를 부른다");
            }
            let v = func("pub(crate) fn seat_identity_view_in(");
            let (copy, read) = (v.find("collect_seat_rows(").expect("복사 호출"), v.find("folder_identity(").expect("판독 호출"));
            assert!(copy < read, "신원 판독이 좌석 표 복사(surfaces 락 해제) 앞에 있다");
            assert!(!v.contains("accounts.lock("), "신원 표 본체가 accounts 락을 직접 잡는다(겹쳐 쥐는 간선)");
            let f = func("fn folder_identity_state(");
            // ★R1F-US: 실판독 앵커가 `claude_identity_unlocked`(Option) → `claude_identity_probe`(3값)로 바뀌었다 — 계약(캐시 락은 조회·선점 한 번 · 기록 한 번, 판독은 그 사이 무락)은 그대로다.
            let cache_lock_end = f.find("claude_identity_probe(").expect("실판독 호출");
            assert!(f[..cache_lock_end].matches("seat_ident_cache.lock()").count() == 1 && f[cache_lock_end..].contains("seat_ident_cache.lock()"), "캐시 락은 조회·기록 때만 순간 잡는다(판독을 사이에 두고 두 번)");
            let rw = func("pub fn alert_rates_with(");
            assert!(!rw.contains("seat_identity_view") && !rw.contains("std::fs::"), "alert_rates_with 가 accounts 락 안에서 신원을 판독한다");
        }

        /// 노브 0 = 0.14.42 동작: 같은 로그인 전환 시나리오에서 `stale_secs=0` 이면 A 경보가 +1801초에도 REMIND 로 유지되고 해소 알림이 없다. 기본(1800)이면 소멸한다(음성 대조).
        #[test]
        fn b3_zero_knob_keeps_the_old_login_alert_and_the_default_retires_it() {
            let run = |tag: &str, stale: f64| {
                let (d, dir, home, f) = b3_fixture(tag);
                let cfg = crate::alerts::AlertConfig::default();
                let mut fired: HashMap<String, f64> = HashMap::new();
                let _seat = b3_seat(&d, "worker-b3", "claude", Some(&f));
                let t0 = crate::state::now_epoch();
                let seq0 = d.bus.latest_seq();
                assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
                crate::governance::check_alerts_with_stale(&d, &mut fired, &cfg, t0 + 1.0, stale);
                b3_login(&home, ".cys/claude-b3", "u-b3-b", "b-b3@example.test", 2);
                assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 10.0, Some(t0 + 9000.0))], "statusline", t0 + 5.0));
                for dt in [70.0, 1799.0, 1801.0] {
                    crate::governance::check_alerts_with_stale(&d, &mut fired, &cfg, t0 + dt, stale);
                }
                let out = (account_alerts(&d, seq0, KEY_A), b3_resolved(&d, seq0));
                let _ = std::fs::remove_dir_all(&dir);
                let _ = std::fs::remove_dir_all(&home);
                out
            };
            assert_eq!(run("u11a", 0.0), (2, vec![]), "노브 0 인데 0.14.42 와 다르다(A 유지 + REMIND 1회 · 해소 알림 없음)");
            assert_eq!(run("u11b", 1800.0), (1, vec![(KEY_A.to_string(), "stale".to_string())]));
        }

        /// 경보 `detail` 가산 키가 실제 틱 산출물(버스 이벤트 payload)에 실린다 — observed_at · age_secs · in_use · reset_in_secs · held_secs · 기존 키 불변.
        #[test]
        fn b3_fired_alert_detail_carries_observation_meta() {
            let (d, dir, home, f) = b3_fixture("u12");
            let cfg = crate::alerts::AlertConfig::default();
            let mut fired: HashMap<String, f64> = HashMap::new();
            let _seat = b3_seat(&d, "worker-b3", "claude", Some(&f));
            let t0 = crate::state::now_epoch();
            let seq0 = d.bus.latest_seq();
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 30.0);
            let evs = b3_events(&d, seq0, "alert.account_rate");
            assert_eq!(evs.len(), 1);
            let det = &evs[0]["detail"];
            assert_eq!(det["account"], json!("a-b3@example.test"));
            assert_eq!((det["win"].clone(), det["used_pct"].clone()), (json!("5h"), json!(99.0)), "기존 detail 키가 바뀌었다");
            assert_eq!(det["observed_at"], json!(t0));
            assert_eq!(det["age_secs"], json!(30));
            assert_eq!(det["in_use"], json!(true));
            assert_eq!(det["reset_in_secs"], json!(7170));
            assert_eq!(det["held_secs"], json!(1770));
            assert_eq!(evs[0]["key"], json!(KEY_A));
            assert_eq!(evs[0]["isolate"], json!(true));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }
    }

    // ═════════ ★0.14.43(B1) — 현재 로그인 폴더(`current_profiles`) · 별명(`alias`) ═════════
    // 픽스처는 전부 합성값이다(u-b1-a · a-b1@example.test 등) — 실계정 식별자 금지. 홈은 임시 폴더이고 [`test_home`] 이음매가 **이 스레드의** `account_home()` 만 바꾼다
    // (실제 홈의 파일·`~/.cys/accounts.json` 을 읽지 않는다). 시각은 주입한다(실시간 대기 없음 — 60초 하한은 `now` 로 센다).
    mod b1_units {
        use super::b3_scenarios::{b3_login, b3_seat};
        use super::*;

        const KEY_A: &str = "account_rate:a-b1@example.test:5h";

        fn row(rows: &Value, id: &str) -> Value {
            rows.as_array().unwrap().iter().find(|r| r["account_id"] == id).cloned().unwrap_or_else(|| panic!("행 {id} 없음: {rows}"))
        }

        /// 문자열 배열 → 정렬·중복 제거한 목록. 윈도우에서는 열거 경로(`\`)와 보고 경로(`/`)가 섞여 오므로 구분자를 `/` 로 접는다(화면의 `normalizeProfile` 과 같다 — 맥·리눅스는 그대로).
        fn strs(v: &Value) -> Vec<String> {
            v.as_array()
                .unwrap_or_else(|| panic!("배열이 아니다: {v}"))
                .iter()
                .map(|x| x.as_str().unwrap().replace('\\', "/"))
                .collect::<BTreeSet<String>>()
                .into_iter()
                .collect()
        }

        fn s(items: &[&str]) -> Vec<String> {
            items.iter().map(|x| x.to_string()).collect()
        }

        /// 별명 파일을 쓴다 — mtime 을 `bump` 로 맞춘다(같은 초 안에 다시 써도 mtime 이 달라지게).
        fn put_alias_bytes(home: &Path, body: &[u8], bump: u64) {
            let f = home.join(".cys/accounts.json");
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(&f, body).unwrap();
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_100_000 + bump);
            std::fs::File::options().write(true).open(&f).unwrap().set_modified(t).unwrap();
        }

        fn put_aliases(home: &Path, body: &str, bump: u64) {
            put_alias_bytes(home, body.as_bytes(), bump);
        }

        /// 별명 상태 한 장 — (stat 시도 수, 실제 판독 수, 이 키의 별명).
        fn alias_snap(d: &Arc<Daemon>, key: &str) -> (u64, u64, Option<String>) {
            let st = d.accounts.lock().unwrap();
            (st.alias.stats, st.alias.reads, st.alias.table.get(key).cloned())
        }

        /// 폴더 표 한 장 — `Some(a)` = `Known(a)` · `None` = 신원 없음 확정(`NoLogin` — 어느 계정의 폴더도 아니다). 읽지 못함(`Unread`)은 새 검체(`r1f_*`)가 직접 만든다.
        fn known(pairs: &[(&str, Option<&str>)]) -> Vec<(String, FolderWho)> {
            pairs.iter().map(|(p, w)| (p.to_string(), w.map_or(FolderWho::NoLogin, |a| FolderWho::Known(a.to_string())))).collect()
        }

        // ───────────────────────── current_profiles ─────────────────────────

        /// ★핵심 시나리오(오너 제보): 좌석 폴더의 로그인을 A → B 로 바꾸면 `profiles`(추가 전용)에는 옛 계정에도 폴더가 남지만 `current_profiles` 는 지금 로그인된 쪽만 갖는다.
        /// 판독 실패 폴더는 누구의 current 에도 없고 · codex 행은 profiles 그대로다. 신원 판독은 B3 의 60초 캐시를 타므로 시각을 주입한다.
        #[test]
        fn b1_current_profiles_follow_the_folder_login_while_profiles_stay_additive() {
            let dir = tmp("b1-s1-daemon");
            let home = tmp("b1-s1-home");
            let _h = test_home::set(&home);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let fa = b3_login(&home, ".cys/claude-b1", "u-b1-a", "a-b1@example.test", 1);
            b3_login(&home, ".claude-b1x", "u-b1-b", "b-b1@example.test", 1); // B 는 다른 폴더에 로그인 — 부트 발견으로 행이 생긴다(좌석 폴더에서 보고하기 전)
            write(&home.join(".claude-b1bad/.claude.json"), "{not json"); // 신원 판독 실패 폴더
            std::fs::create_dir_all(home.join(".codex")).unwrap();
            {
                let mut st = d.accounts.lock().unwrap();
                seed_discovered(&mut st, &home);
            }
            let t0 = crate::state::now_epoch();
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 40.0, Some(t0 + 7200.0))], "statusline", t0));
            // ① 로그인 A: 행 A 는 좌석 폴더가 현재 · 행 B 는 자기 폴더 · codex 는 profiles 그대로 · 판독 실패 폴더는 누구에게도 없다
            let rows = local_json(&d, t0 + 1.0);
            let (a, b, cx) = (row(&rows, "u-b1-a"), row(&rows, "u-b1-b"), row(&rows, "default"));
            assert_eq!(strs(&a["current_profiles"]), s(&[".cys/claude-b1"]));
            assert_eq!(strs(&b["current_profiles"]), s(&[".claude-b1x"]));
            assert_eq!((cx["provider"].clone(), strs(&cx["current_profiles"])), (json!("codex"), s(&[".codex"])), "codex 는 폴더당 계정 1개 — profiles 그대로");
            assert_eq!(strs(&cx["current_profiles"]), strs(&cx["profiles"]));
            for r in rows.as_array().unwrap() {
                assert!(!strs(&r["current_profiles"]).iter().any(|p| p.contains("b1bad")), "신원 판독 실패 폴더가 어느 계정의 current 에 들어갔다: {r}");
            }
            // ② 같은 폴더의 로그인을 B 로 바꾼다 — 60초 캐시 하한이 지난 뒤 행 A 는 current 에서 빠지고(profiles 는 그대로) 행 B 가 폴더를 갖는다
            b3_login(&home, ".cys/claude-b1", "u-b1-b", "b-b1@example.test", 2);
            let rows = local_json(&d, t0 + 1.0 + SEAT_IDENT_CACHE_SECS);
            let (a, b) = (row(&rows, "u-b1-a"), row(&rows, "u-b1-b"));
            assert_eq!(strs(&a["current_profiles"]), Vec::<String>::new(), "로그인을 B 로 바꿨는데 옛 계정 A 에 현재 폴더가 남았다");
            assert_eq!(strs(&a["profiles"]), s(&[".cys/claude-b1"]), "profiles 는 추가 전용 — 이관·삭제 없음(동작 무변)");
            assert_eq!(strs(&b["current_profiles"]), s(&[".claude-b1x", ".cys/claude-b1"]), "새 계정 B(보고 전 발견 시드 행)가 좌석 폴더를 현재로 갖지 못했다");
            assert_eq!(strs(&b["profiles"]), s(&[".claude-b1x"]), "current_profiles 가 profiles 를 건드렸다(보고 전이라 좌석 폴더가 아직 없어야 한다)");
            // ③ B 가 좌석 폴더에서 보고하면 — 옛 계정과 새 계정 둘 다 profiles 에 좌석 폴더를 갖는다(제보된 결함 모양) · current 가 둘을 가른다
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 10.0, Some(t0 + 9000.0))], "statusline", t0 + 62.0));
            let rows = local_json(&d, t0 + 63.0);
            let (a, b) = (row(&rows, "u-b1-a"), row(&rows, "u-b1-b"));
            assert_eq!(strs(&a["profiles"]), s(&[".cys/claude-b1"]));
            assert_eq!(strs(&b["profiles"]), s(&[".claude-b1x", ".cys/claude-b1"]));
            assert_eq!(strs(&a["current_profiles"]), Vec::<String>::new());
            assert_eq!(strs(&b["current_profiles"]), s(&[".claude-b1x", ".cys/claude-b1"]));
            // 기존 키는 불변(가산뿐)
            for k in ["provider", "account_id", "label", "plan", "profiles", "rate", "updated_at", "stale_secs", "source", "adapter", "source_error", "in_use", "rate_observed_at"] {
                assert!(a.get(k).is_some(), "기존 키 {k} 소실: {a}");
            }
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// `current_profiles_for` 순수 표 — claude: 현재 신원이 이 계정인 폴더만(정렬·중복 제거) · 판독 실패(None)는 누구의 것도 아니다(`None == None` 함정 포함) ·
        /// 그 밖 provider: profiles 그대로 · 폴더 목록이 비면(홈 불명) 빈 배열.
        #[test]
        fn b1_current_profiles_for_table() {
            // ★R1F-US: 반환형이 `Option<Vec<_>>`(None = 키를 내지 않는다) · 폴더 표가 `Option<&[_]>`(None = 홈 불명·열거 실패)로 바뀌었다. 아래 옛 단언은 '키를 내는' 경우(Some)를 **그대로** 본다 —
            // 키 부재 경우는 새 검체 `r1f_current_profiles_key_is_absent_when_a_profiles_folder_is_unread_or_the_listing_failed`.
            let cp = |p: &str, a: &str, pr: &BTreeSet<String>, k: &[(String, FolderWho)]| {
                current_profiles_for(p, a, pr, Some(k)).expect("키를 내야 하는 경우인데 None")
            };
            let profiles: BTreeSet<String> = s(&[".cys/claude-b1", ".claude-b1z"]).into_iter().collect();
            let k = known(&[(".cys/claude-b1", Some("A")), (".claude-b1x", Some("B")), (".claude-b1bad", None), (".claude-b1y", Some("A")), (".cys/claude-b1", Some("A"))]);
            assert_eq!(cp("claude", "A", &profiles, &k), s(&[".claude-b1y", ".cys/claude-b1"]), "일치하는 폴더만 · 정렬 · 중복 제거");
            assert_eq!(cp("claude", "B", &profiles, &k), s(&[".claude-b1x"]));
            assert!(cp("claude", "C", &profiles, &k).is_empty(), "아무 폴더도 아니면 빈 배열(profiles 에 남은 폴더는 무관)");
            assert!(cp("claude", "", &profiles, &k).is_empty(), "판독 실패(None) 폴더가 빈 account_id 와 같다고 읽혔다(None==None 함정)");
            assert!(!cp("claude", "A", &profiles, &k).iter().any(|p| p.contains("bad")), "판독 실패 폴더가 current 에 들어갔다");
            assert!(cp("claude", "A", &profiles, &[]).is_empty(), "열거는 성공했지만 알려진 폴더가 0개면 빈 배열");
            let codex: BTreeSet<String> = s(&[".codex"]).into_iter().collect();
            assert_eq!(cp("codex", "default", &codex, &k), s(&[".codex"]), "codex 는 profiles 그대로");
            let agy: BTreeSet<String> = s(&[".gemini/antigravity-cli", ".antigravity"]).into_iter().collect();
            assert_eq!(cp("antigravity", "default", &agy, &k), s(&[".antigravity", ".gemini/antigravity-cli"]), "agy 는 profiles 그대로(정렬된 집합)");
            assert!(cp("codex", "default", &BTreeSet::new(), &k).is_empty(), "profiles 가 비면 빈 배열");
            assert!(cp("grok", "default", &BTreeSet::new(), &k).is_empty(), "선언 계정(그 밖 provider)도 profiles 그대로");
        }

        /// 알려진 프로필 폴더 표 — 열거 정본(기본 프로필 `~/.claude` 포함)·`profile_short` 표기·B3 의 60초 캐시 재사용(좌석 신원 표가 이미 읽은 폴더는 다시 stat 하지 않는다 · 새 캐시 없음)·
        /// 기본 프로필의 신원은 홈 직하 `~/.claude.json` · 홈 불명이면 빈 표.
        #[test]
        fn b1_known_profile_identities_reuse_the_b3_cache_and_read_the_default_profile_login() {
            let dir = tmp("b1-u3-daemon");
            let home = tmp("b1-u3-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let fa = b3_login(&home, ".cys/claude-b1", "u-b1-a", "a-b1@example.test", 1);
            b3_login(&home, ".claude-b1x", "u-b1-b", "b-b1@example.test", 2);
            std::fs::create_dir_all(home.join(".claude")).unwrap(); // 기본 프로필 — 폴더 안에는 신원 파일이 없고 홈 직하 `.claude.json` 이 신원이다
            write(&home.join(".claude.json"), r#"{"oauthAccount":{"accountUuid":"u-b1-d","emailAddress":"d-b1@example.test"}}"#);
            write(&home.join(".claude-b1bad/.claude.json"), "{not json");
            let _seat = b3_seat(&d, "worker-b1", "claude", Some(&fa));
            let t0 = crate::state::now_epoch();
            let sorted = |v: Vec<(String, FolderWho)>| {
                let mut v: Vec<(String, FolderWho)> = v.into_iter().map(|(p, w)| (p.replace('\\', "/"), w)).collect(); // 윈도우 열거 경로의 `\` 를 접는다
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v
            };
            // 좌석 신원 표가 좌석 폴더를 먼저 읽는다 → 같은 캐시이므로 알려진 폴더 표는 그 폴더를 다시 읽지 않는다
            let _ = seat_identity_view_in(&d, Some(&home), t0);
            assert_eq!(d.seat_ident_cache.lock().unwrap().reads, 1);
            let got = sorted(known_profile_identities(&d, Some(&home), t0 + 1.0).expect("열거 성공(R1F-US: 반환형 Option)"));
            // ★R1F-US(보정): 표의 값이 3값이다 — 파싱이 안 되는 신원 파일(`{not json`)은 '읽지 못함'(`Unread` — 직전 신원이 없어 유예 없이)이고, 신원 없음 확정(`NoLogin`)과 다르다.
            let mut want = known(&[(".claude", Some("u-b1-d")), (".claude-b1x", Some("u-b1-b")), (".cys/claude-b1", Some("u-b1-a"))]);
            want.push((".claude-b1bad".to_string(), FolderWho::Unread));
            want.sort_by(|a, b| a.0.cmp(&b.0));
            assert_eq!(got, want, "열거 정본·표기·기본 프로필(홈 직하 신원)·읽지 못함(Unread)");
            {
                let c = d.seat_ident_cache.lock().unwrap();
                assert_eq!((c.reads, c.hits), (4, 1), "좌석 폴더 1 + 나머지 3 = 실판독 4 · 좌석 폴더는 적중 1 — 캐시가 갈렸거나 새 캐시를 만들었다");
            }
            // 60초 안의 연속 호출은 stat 0회
            for dt in [5.0, 30.0, 59.0] {
                let _ = known_profile_identities(&d, Some(&home), t0 + dt);
            }
            assert_eq!(d.seat_ident_cache.lock().unwrap().reads, 4, "60초 하한 안에서 폴더를 다시 판독했다");
            // 파일을 지워도(= 관측) 하한 안에서는 옛 신원 · 하한이 지나면 다시 본다
            std::fs::remove_file(home.join(".claude.json")).unwrap();
            let got = sorted(known_profile_identities(&d, Some(&home), t0 + 59.5).expect("열거 성공"));
            assert_eq!(got[0], (".claude".to_string(), FolderWho::Known("u-b1-d".to_string())), "하한 안인데 기본 프로필 신원이 바뀌었다");
            let got = sorted(known_profile_identities(&d, Some(&home), t0 + 200.0).expect("열거 성공"));
            assert_eq!(got[0], (".claude".to_string(), FolderWho::NoLogin), "하한이 지났는데 지워진 신원이 그대로다(파일이 없어진 것은 신원 없음 확정 — 읽지 못함이 아니다)");
            // 홈 불명 → 빈 표(파일을 보지 않는다)
            assert!(known_profile_identities(&d, None, t0).is_none(), "홈 불명은 목록을 못 얻은 것(None) — R1F-US: 부른 쪽이 current_profiles 키를 내지 않는다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// FIFO 신원 파일을 가진 **알려진 프로필 폴더**(좌석이 쓰지 않는 폴더)가 `usage.accounts`(`local_json`)를 세우지 않고, 그 폴더는 누구의 current 에도 없다(R4-F2 패턴).
        #[cfg(unix)]
        #[test]
        fn b1_fifo_identity_in_a_known_profile_folder_blocks_nothing_and_is_nobodys_current() {
            let dir = tmp("b1-u4-daemon");
            let home = tmp("b1-u4-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let fa = b3_login(&home, ".cys/claude-b1", "u-b1-a", "a-b1@example.test", 1);
            let fifo = home.join(".claude-b1fifo/.claude.json");
            mkfifo(&fifo);
            let t0 = crate::state::now_epoch();
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 40.0, Some(t0 + 7200.0))], "statusline", t0));
            let (tx, rx) = std::sync::mpsc::channel();
            {
                let (d, home) = (d.clone(), home.clone());
                std::thread::spawn(move || {
                    let _h = test_home::set(&home);
                    let _ = tx.send(local_json(&d, t0 + 1.0));
                });
            }
            let out = rx.recv_timeout(std::time::Duration::from_secs(5));
            release_fifo(&fifo);
            let rows = out.expect("FIFO 신원을 가진 알려진 프로필 폴더가 usage.accounts 를 멈췄다");
            assert_eq!(strs(&row(&rows, "u-b1-a")["current_profiles"]), s(&[".cys/claude-b1"]));
            for r in rows.as_array().unwrap() {
                assert!(!strs(&r["current_profiles"]).iter().any(|p| p.contains("b1fifo")), "신원 판독 실패(FIFO) 폴더가 current 에 들어갔다: {r}");
            }
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        // ───────────────────────── alias ─────────────────────────

        /// 별명 값 정제 표 — 제어 문자 제거 · 앞뒤 공백 제거 · 빈 값 무시 · 최대 24자(char 기준) 절단.
        #[test]
        fn b1_sanitize_alias_table() {
            let long_mix = format!("{}\n{}", "가".repeat(20), "나".repeat(20));
            let cases: Vec<(&str, String, Option<String>)> = vec![
                ("평문", "업무용".into(), Some("업무용".into())),
                ("앞뒤 공백", "  개인  ".into(), Some("개인".into())),
                ("탭·개행·CR 은 제어 문자 — 제거", "\t업무\n용\r\n".into(), Some("업무용".into())),
                ("NUL·BEL·DEL·C1(0x85) 제거", "a\u{0}b\u{7}c\u{7f}d\u{85}e".into(), Some("abcde".into())),
                ("ESC 시퀀스의 제어 바이트 제거(터미널 오염 차단)", "x\u{1b}[31my".into(), Some("x[31my".into())),
                ("내부 공백은 유지", "업무 용".into(), Some("업무 용".into())),
                ("빈 문자열", "".into(), None),
                ("공백뿐(전각 공백 포함)", "   \u{3000} ".into(), None),
                ("제어 문자뿐", "\u{0}\u{1}\n".into(), None),
                ("정확히 24자", "가".repeat(24), Some("가".repeat(24))),
                ("25자 → 24자", "가".repeat(25), Some("가".repeat(24))),
                ("30자 ASCII → 24자", "a".repeat(30), Some("a".repeat(24))),
                ("이모지 30개 → 24개(char 기준)", "😀".repeat(30), Some("😀".repeat(24))),
                ("제어 문자를 걷은 뒤 24자", long_mix, Some(format!("{}{}", "가".repeat(20), "나".repeat(4)))),
                ("절단이 공백에서 끝나면 그 공백도 걷는다", format!("{} bbb", "a".repeat(23)), Some("a".repeat(23))),
            ];
            for (what, raw, want) in cases {
                assert_eq!(sanitize_alias(&raw), want, "{what}");
                if let Some(w) = want {
                    assert!(w.chars().count() <= 24 && !w.chars().any(char::is_control), "{what}: 결과가 정제 규칙을 어겼다");
                }
            }
        }

        /// 별명 표 파서 — 값이 문자열이 아니거나 정제 뒤 비면 그 항목만 버린다 · 키가 빈 문자열이면 버린다 · `aliases` 가 객체가 아니거나 루트가 객체가 아니거나 JSON 오류·UTF-8 아님이면 빈 표 ·
        /// 같은 파일의 `accounts` 배열(선언 계정)은 읽지 않는다.
        #[test]
        fn b1_parse_alias_table_shapes() {
            let p = |t: &str| parse_alias_table(t.as_bytes());
            let want: HashMap<String, String> = [("k1", "업무"), ("k10", "가")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert_eq!(
                p(r#"{"aliases":{"k1":"  업무  ","k2":5,"k3":null,"k4":["x"],"k5":{"a":1},"k6":true,"k7":"","k8":"   ","k9":"\u0007\u0000","":"빈키","k10":"가"}}"#),
                want,
                "문자열이 아니거나 정제 뒤 비는 값 · 빈 키는 그 항목만 버린다"
            );
            assert_eq!(p(r#"{"accounts":[{"provider":"grok","label":"Grok","adapter":"none"}],"aliases":{"k1":"업무"}}"#).len(), 1, "accounts 배열은 별명 표와 무관하다");
            for bad in [
                r#"{"accounts":[{"provider":"grok"}]}"#, // aliases 없음
                r#"{"aliases":["a","b"]}"#,
                r#"{"aliases":"x"}"#,
                r#"{"aliases":null}"#,
                r#"{"aliases":7}"#,
                r#"["aliases"]"#,
                r#""aliases""#,
                "null",
                "{not json",
                "",
            ] {
                assert!(p(bad).is_empty(), "{bad:?} 는 빈 표여야 한다");
            }
            assert!(parse_alias_table(&[0x7b, 0xff, 0xfe, 0x7d]).is_empty(), "UTF-8 이 아닌 본문");
            // 별명 표는 정제 규칙을 통과한 값만 담는다(24자 절단 포함)
            let long = format!(r#"{{"aliases":{{"u":"{}"}}}}"#, "가".repeat(30));
            assert_eq!(p(&long).get("u").map(|v| v.chars().count()), Some(24));
        }

        /// `alias_for` 우선순위 — account_id 키가 label(이메일) 키보다 우선 · 어느 쪽만 있어도 찾는다 · 비 claude 행은 label(예: `OpenAI Codex`)로 찾는다.
        #[test]
        fn b1_alias_for_prefers_account_id_over_label() {
            let t: HashMap<String, String> =
                [("u-b1-a", "ID별명"), ("a-b1@example.test", "메일별명"), ("OpenAI Codex", "코덱스")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert_eq!(alias_for(&t, "u-b1-a", "a-b1@example.test"), Some("ID별명"), "둘 다 있으면 account_id 가 이긴다");
            assert_eq!(alias_for(&t, "u-b1-zzz", "a-b1@example.test"), Some("메일별명"), "이메일 키만");
            assert_eq!(alias_for(&t, "u-b1-a", "other@example.test"), Some("ID별명"), "account_id 키만");
            assert_eq!(alias_for(&t, "default", "OpenAI Codex"), Some("코덱스"), "비 claude 행은 라벨 키로");
            assert_eq!(alias_for(&t, "u-none", "none@example.test"), None);
            assert_eq!(alias_for(&HashMap::new(), "u-b1-a", "a-b1@example.test"), None);
        }

        /// 끝에서 끝까지(부트 시드 → `usage.accounts` 행): account_id 키 · 이메일 키 · 둘 다(account_id 우선) · 라벨 키(codex·선언 계정) · 별명 없는 행은 null.
        /// 부트에서 1회 판독하고(stat 1 · read 1) 하한 안의 RPC 는 stat 도 하지 않는다. 선언 계정(`accounts` 배열) 판독은 종전 그대로다.
        #[test]
        fn b1_boot_seed_loads_aliases_once_and_rows_carry_the_alias() {
            let dir = tmp("b1-u7-daemon");
            let home = tmp("b1-u7-home");
            let _h = test_home::set(&home);
            b3_login(&home, ".cys/claude-b1", "u-b1-a", "a-b1@example.test", 1);
            b3_login(&home, ".claude-b1x", "u-b1-b", "b-b1@example.test", 1);
            b3_login(&home, ".claude-b1y", "u-b1-c", "c-b1@example.test", 1);
            b3_login(&home, ".claude-b1w", "u-b1-w", "w-b1@example.test", 1);
            std::fs::create_dir_all(home.join(".codex")).unwrap();
            put_aliases(
                &home,
                r#"{"accounts":[{"provider":"grok","label":"Grok Test","adapter":"none"}],
                    "aliases":{"u-b1-a":"ID별명A","a-b1@example.test":"메일별명A","b-b1@example.test":"메일별명B","u-b1-c":"ID별명C",
                               "OpenAI Codex":"코덱스","Grok Test":"그록","no-such":"없는계정"}}"#,
                1,
            );
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            seed_known(&d);
            assert_eq!(alias_snap(&d, "u-b1-a"), (1, 1, Some("ID별명A".to_string())), "부트 시드가 별명을 1회 판독하지 않았다");
            let boot_at = d.accounts.lock().unwrap().alias.checked_at.expect("부트 시드가 별명 확인 시각을 남기지 않았다");
            for dt in [1.0, 2.0, 30.0] {
                let _ = local_json(&d, boot_at + dt);
            }
            assert_eq!((alias_snap(&d, "u-b1-a").0, alias_snap(&d, "u-b1-a").1), (1, 1), "부트 직후 60초 안의 RPC 가 stat 을 했다");
            let rows = local_json(&d, boot_at + 31.0);
            let al = |id: &str| row(&rows, id)["alias"].clone();
            assert_eq!(al("u-b1-a"), json!("ID별명A"), "둘 다 있으면 account_id 우선");
            assert_eq!(al("u-b1-b"), json!("메일별명B"), "이메일 키만");
            assert_eq!(al("u-b1-c"), json!("ID별명C"), "account_id 키만");
            assert_eq!(al("u-b1-w"), Value::Null, "별명이 없는 행은 null");
            let others: Vec<(String, Value)> = rows
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["provider"] != "claude")
                .map(|r| (r["provider"].as_str().unwrap().to_string(), r["alias"].clone()))
                .collect();
            assert!(others.contains(&("codex".to_string(), json!("코덱스"))), "라벨 키로 찾는 codex 행: {others:?}");
            assert!(others.contains(&("grok".to_string(), json!("그록"))), "선언 계정은 종전처럼 로드되고 라벨 키로 별명을 찾는다: {others:?}");
            // 별명 파일을 바꿔도 60초 안에는 반영되지 않고(stat 0회) · 60초 뒤 mtime 이 바뀐 것을 보고 반영한다
            put_aliases(&home, r#"{"aliases":{"u-b1-a":"새별명"}}"#, 2);
            assert_eq!(row(&local_json(&d, boot_at + 59.0), "u-b1-a")["alias"], json!("ID별명A"));
            assert_eq!(row(&local_json(&d, boot_at + 61.0), "u-b1-a")["alias"], json!("새별명"));
            assert_eq!(row(&local_json(&d, boot_at + 61.0), "u-b1-b")["alias"], Value::Null, "파일에서 빠진 별명이 남았다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 재판독 규율 — 부트 1회 + 파일 mtime(서명)이 바뀔 때만 · 60초 하한 안에서는 stat 0회 · 변화가 없으면 stat 은 하되 재판독은 없다 · 시계가 뒤로 가면 만료로 본다.
        #[test]
        fn b1_alias_file_is_reread_only_when_the_file_changes_and_never_checked_inside_sixty_seconds() {
            let dir = tmp("b1-u8-daemon");
            let home = tmp("b1-u8-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            put_aliases(&home, r#"{"aliases":{"u-b1-a":"첫째"}}"#, 1);
            let t0 = crate::state::now_epoch();
            refresh_aliases(&d, Some(&home), t0);
            assert_eq!(alias_snap(&d, "u-b1-a"), (1, 1, Some("첫째".to_string())));
            // 새 내용·새 mtime 으로 바꿔도 60초 안에는 stat 도 하지 않는다(별명이 옛 값 그대로 = 파일을 다시 보지 않았다는 관측)
            put_aliases(&home, r#"{"aliases":{"u-b1-a":"둘째"}}"#, 2);
            for dt in [0.0, 1.0, 30.0, 59.9] {
                refresh_aliases(&d, Some(&home), t0 + dt);
            }
            assert_eq!(alias_snap(&d, "u-b1-a"), (1, 1, Some("첫째".to_string())), "60초 하한 안에서 stat 을 했다");
            // 60초가 되면 확인 → mtime 이 바뀌었으니 재판독
            refresh_aliases(&d, Some(&home), t0 + 60.0);
            assert_eq!(alias_snap(&d, "u-b1-a"), (2, 2, Some("둘째".to_string())));
            // 변화 없음 — 확인(stat)은 하되 재판독은 없다
            refresh_aliases(&d, Some(&home), t0 + 120.0);
            refresh_aliases(&d, Some(&home), t0 + 180.0);
            assert_eq!(alias_snap(&d, "u-b1-a"), (4, 2, Some("둘째".to_string())), "mtime 이 같은데 재판독했다");
            // 시계 역행(now < 마지막 확인)은 만료 — 하한이 영구히 붙지 않는다
            refresh_aliases(&d, Some(&home), t0 + 10.0);
            assert_eq!(alias_snap(&d, "u-b1-a").0, 5);
            // 파일이 사라지면(부재) 별명 표는 비워진다 · 부재 → 부재는 재판독 없음
            std::fs::remove_file(home.join(".cys/accounts.json")).unwrap();
            refresh_aliases(&d, Some(&home), t0 + 100.0);
            assert_eq!(alias_snap(&d, "u-b1-a"), (6, 2, None), "부재는 stat 만 하고 파일을 읽지 않는다");
            refresh_aliases(&d, Some(&home), t0 + 200.0);
            assert_eq!(alias_snap(&d, "u-b1-a"), (7, 2, None), "부재 → 부재인데 재판독했다");
            // 홈 불명이면 파일을 보지 않는다(별명 없음)
            refresh_aliases(&d, None, t0 + 300.0);
            assert_eq!(alias_snap(&d, "u-b1-a"), (8, 2, None));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 쓸 수 없는 별명 파일(JSON 오류 · 빈 파일 · `aliases` 가 객체 아님 · 루트가 배열 · UTF-8 아님 · 64KiB 초과 · 디렉터리) = 별명 없음 + **기존 별명 표는 비운다** ·
        /// 정확히 64KiB 는 유효(양성 대조).
        #[test]
        fn b1_unusable_alias_files_mean_no_aliases_and_clear_the_old_table() {
            let dir = tmp("b1-u9-daemon");
            let home = tmp("b1-u9-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let stem = r#"{"aliases":{"u-b1-a":"한계"},"pad":""}"#;
            let exact = format!(r#"{{"aliases":{{"u-b1-a":"한계"}},"pad":"{}"}}"#, "p".repeat(65536 - stem.len()));
            assert_eq!(exact.len() as u64, ALIAS_FILE_MAX_BYTES, "픽스처 계산 오류");
            let over = format!("{exact}\n");
            let cases: Vec<(&str, Vec<u8>)> = vec![
                ("JSON 오류", b"{not json".to_vec()),
                ("빈 파일", Vec::new()),
                ("루트가 배열", b"[1,2]".to_vec()),
                ("aliases 가 배열", br#"{"aliases":["a"]}"#.to_vec()),
                ("aliases 가 문자열", br#"{"aliases":"a"}"#.to_vec()),
                ("aliases 가 null", br#"{"aliases":null}"#.to_vec()),
                ("UTF-8 아님", vec![0x7b, 0xff, 0xfe, 0x7d]),
                ("64KiB 초과(1바이트)", over.into_bytes()),
            ];
            let mut t = crate::state::now_epoch();
            let mut bump = 10u64;
            for (what, body) in cases {
                // 유효한 별명을 먼저 싣고(양성 대조) → 쓸 수 없는 내용으로 바꾸면 표가 비워진다
                bump += 1;
                put_aliases(&home, r#"{"aliases":{"u-b1-a":"유효"}}"#, bump);
                t += 100.0;
                refresh_aliases(&d, Some(&home), t);
                assert_eq!(alias_snap(&d, "u-b1-a").2, Some("유효".to_string()), "{what}: 양성 대조 실패");
                bump += 1;
                put_alias_bytes(&home, &body, bump);
                t += 100.0;
                refresh_aliases(&d, Some(&home), t);
                assert_eq!(alias_snap(&d, "u-b1-a").2, None, "{what}: 쓸 수 없는 파일인데 옛 별명이 남았다");
                assert!(d.accounts.lock().unwrap().alias.table.is_empty(), "{what}: 표가 비워지지 않았다");
            }
            // 디렉터리(비정규) — 열지 않는다
            bump += 1;
            put_aliases(&home, r#"{"aliases":{"u-b1-a":"유효"}}"#, bump);
            t += 100.0;
            refresh_aliases(&d, Some(&home), t);
            assert_eq!(alias_snap(&d, "u-b1-a").2, Some("유효".to_string()));
            std::fs::remove_file(home.join(".cys/accounts.json")).unwrap();
            std::fs::create_dir_all(home.join(".cys/accounts.json")).unwrap();
            t += 100.0;
            refresh_aliases(&d, Some(&home), t);
            assert_eq!(alias_snap(&d, "u-b1-a").2, None, "디렉터리인 accounts.json 에서 옛 별명이 남았다");
            std::fs::remove_dir_all(home.join(".cys/accounts.json")).unwrap();
            // 정확히 64KiB 는 유효
            bump += 1;
            put_aliases(&home, &exact, bump);
            t += 100.0;
            refresh_aliases(&d, Some(&home), t);
            assert_eq!(alias_snap(&d, "u-b1-a").2, Some("한계".to_string()), "정확히 64KiB 파일을 거절했다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 별명 파일이 FIFO 여도(읽으면 막히는 파일) 부트 판독 경로·`usage.accounts`·재판독이 멈추지 않는다 — 별명 없음 + RPC 무정지.
        #[cfg(unix)]
        #[test]
        fn b1_fifo_alias_file_blocks_nothing() {
            let dir = tmp("b1-u10-daemon");
            let home = tmp("b1-u10-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let fa = b3_login(&home, ".cys/claude-b1", "u-b1-a", "a-b1@example.test", 1);
            let fifo = home.join(".cys/accounts.json");
            mkfifo(&fifo);
            let t0 = crate::state::now_epoch();
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 40.0, Some(t0 + 7200.0))], "statusline", t0));
            let (tx, rx) = std::sync::mpsc::channel();
            {
                let (d, home) = (d.clone(), home.clone());
                std::thread::spawn(move || {
                    let _h = test_home::set(&home);
                    refresh_aliases(&d, Some(&home), t0);
                    let r1 = local_json(&d, t0 + 1.0);
                    let r2 = local_json(&d, t0 + 100.0); // 하한이 지난 두 번째 확인도 막히지 않는다
                    let _ = tx.send((r1, r2));
                });
            }
            let out = rx.recv_timeout(std::time::Duration::from_secs(5));
            release_fifo(&fifo);
            let (r1, r2) = out.expect("FIFO 별명 파일이 별명 판독·usage.accounts 를 멈췄다");
            for r in [&r1, &r2] {
                assert_eq!(row(r, "u-b1-a")["alias"], Value::Null, "FIFO 별명 파일인데 별명이 실렸다");
            }
            assert_eq!(d.accounts.lock().unwrap().alias.reads, 0, "FIFO 를 열어 읽으려 했다(비정규 파일은 열지 않는다)");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 별명은 표시 전용이다 — 경보 키·라벨·이벤트 payload(key·message·detail)·`alert_inputs` 에 나타나지 않는다. 별명이 정말 실려 있는 상태에서(음성 대조: 행에는 보인다) 경보 틱을 돌려
        /// 발행된 모든 이벤트에 별명 문자열이 0이고, 경보 키·라벨은 이메일 그대로다.
        #[test]
        fn b1_alias_is_display_only_and_never_reaches_alert_keys_labels_or_events() {
            let dir = tmp("b1-u11-daemon");
            let home = tmp("b1-u11-home");
            let _h = test_home::set(&home);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let fa = b3_login(&home, ".cys/claude-b1", "u-b1-a", "a-b1@example.test", 1);
            put_aliases(&home, r#"{"aliases":{"u-b1-a":"ZZ별칭-ID","a-b1@example.test":"ZZ별칭-MAIL"}}"#, 1);
            let _seat = b3_seat(&d, "worker-b1", "claude", Some(&fa));
            let cfg = crate::alerts::AlertConfig::default();
            let mut fired: HashMap<String, f64> = HashMap::new();
            let t0 = crate::state::now_epoch();
            let seq0 = d.bus.latest_seq();
            assert!(note_rate_for_profile_at(&d, Some(&home), &fa, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
            refresh_aliases(&d, Some(&home), t0);
            assert_eq!(row(&local_json(&d, t0 + 1.0), "u-b1-a")["alias"], json!("ZZ별칭-ID"), "전제: 별명이 행에 실려 있어야 이 검체가 의미 있다");
            for dt in [1.0, 1801.0] {
                crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + dt);
            }
            assert_eq!(account_alerts(&d, seq0, KEY_A), 2, "경보 키는 별명이 아니라 이메일 라벨이다(REMIND 까지 2건)");
            let all = serde_json::to_string(&d.bus.replay_after(seq0)).unwrap();
            assert!(all.contains("account_rate"), "전제: 경보 이벤트가 발행돼 있어야 한다");
            assert!(!all.contains("ZZ별칭"), "별명이 이벤트(key·message·detail)에 새었다: {all}");
            assert!(fired.keys().all(|k| !k.contains("ZZ별칭")), "별명이 fired 키에 새었다: {:?}", fired.keys().collect::<Vec<_>>());
            let ev = d.bus.replay_after(seq0).into_iter().find(|e| e["name"] == "alert.account_rate").expect("경보 이벤트");
            assert_eq!(ev["payload"]["detail"]["account"], json!("a-b1@example.test"), "경보 라벨이 이메일이 아니다");
            // 경보 입력(alert_inputs)의 라벨·경보 행(alert_rates_with)의 라벨도 이메일 그대로
            {
                let st = d.accounts.lock().unwrap();
                let key = AccountKey { provider: "claude".into(), account_id: "u-b1-a".into() };
                assert_eq!(st.alert_inputs.get(&key).map(|i| i.label.as_str()), Some("a-b1@example.test"));
            }
            let view = seat_identity_view_in(&d, Some(&home), t0 + 1.0);
            let rows = alert_rates_with(&d, &view, t0 + 1.0, 1800.0);
            assert!(!rows.is_empty() && rows.iter().all(|r| r.label == "a-b1@example.test"), "경보 행의 라벨이 이메일이 아니다: {rows:?}");
            // 경보 틱이 쓰는 순수 평가가 만든 Alert 자체(key · message · detail)도 같다 — 별명 문자열 0 · 키·문구의 계정 이름은 이메일
            let alerts = crate::alerts::evaluate(&crate::alerts::snapshot_with_stale(&d, t0 + 1.0, 1800.0), &cfg);
            let acct: Vec<&crate::alerts::Alert> = alerts.iter().filter(|a| a.kind == "account_rate").collect();
            assert_eq!(acct.len(), 1, "전제: 계정 경보 1건이 평가돼야 한다: {alerts:?}");
            assert_eq!((acct[0].key.as_str(), acct[0].message.as_str()), (KEY_A, "계정 a-b1@example.test 5h rate 99%"));
            for a in &alerts {
                let one = a.to_value().to_string();
                assert!(!one.contains("ZZ별칭"), "별명이 Alert(key·message·detail)에 새었다: {one}");
            }
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 락 순서·배선 핀(소스) — `local_json` 은 accounts 락을 잡기 **전에** 알려진 프로필 신원 표와 별명 표 갱신(파일 IO)을 끝내고, 락 안에서는 순수 함수만 부른다 ·
        /// `refresh_aliases` 는 파일 IO 를 두 번의 순간 락 **사이**에서 한다 · `known_profile_identities` 는 새 캐시·직접 락 없이 B3 의 `folder_identity` 만 쓴다 · 순수 함수에는 파일 IO·락이 없다.
        #[test]
        fn b1_lock_order_wiring_pins() {
            let src = include_str!("accounts.rs");
            let func = |head: &str| -> String {
                let body = src.split(head).nth(1).unwrap_or_else(|| panic!("{head} 소실"));
                body[..body.find("\n}\n").unwrap_or_else(|| panic!("{head} 끝"))].to_string()
            };
            let lj = func("pub fn local_json(");
            let lock = lj.find("daemon.accounts.lock()").expect("local_json 의 accounts 락");
            for pre in ["seat_identity_view_at(", "known_profile_identities(", "refresh_aliases("] {
                let at = lj.find(pre).unwrap_or_else(|| panic!("local_json 이 {pre} 를 부르지 않는다"));
                assert!(at < lock, "local_json 이 accounts 락을 잡은 **뒤에** {pre} 를 부른다(파일 IO 가 락 안)");
            }
            let under_lock = &lj[lock..];
            for io in [
                "std::fs::",
                "enumerate_profile_dirs(",
                "folder_identity(",
                "folder_identity_state(",
                "peek_folder_state(",
                "seat_folder_states(",
                "known_profile_identities(",
                "refresh_aliases(",
                "alias_file_sig(",
                "read_alias_file(",
                "account_home(",
            ] {
                assert!(!under_lock.contains(io), "local_json 의 accounts 락 안에서 {io} 를 부른다");
            }
            assert!(under_lock.contains("current_profiles_for(") && under_lock.contains("alias_for("), "행 가산 키가 순수 함수에서 오지 않는다");
            // refresh_aliases: 락 1(하한 판정 — 블록으로 닫힘) → 파일 IO → 락 2(저장)
            let r = func("fn refresh_aliases(");
            let (l1, l2) = (r.find("daemon.accounts.lock()").expect("락 1"), r.rfind("daemon.accounts.lock()").expect("락 2"));
            assert!(l1 < l2, "refresh_aliases 의 락이 한 번뿐이다(하한 판정·저장 분리 소실)");
            for io in ["alias_file_sig", "read_alias_file("] {
                let at = r.find(io).unwrap_or_else(|| panic!("refresh_aliases 가 {io} 를 부르지 않는다"));
                assert!(l1 < at && at < l2, "{io} 가 두 순간 락 사이가 아니다(락을 쥔 채 파일 IO)");
                assert!(r[l1..at].contains("\n    };"), "{io} 앞에서 하한 판정 락 블록이 닫히지 않았다(락을 쥔 채 파일 IO)");
            }
            // known_profile_identities: 열거 정본 + B3 캐시 도우미만 — 직접 락·직접 캐시 접근·신원 재구현 없음
            // ★R1F-US(m-5): 열거도 같은 60초 하한 캐시에 둔다 — `known_profile_identities` 는 열거 캐시 도우미(`enumerated_profile_dirs`)와 B3 의 `folder_identity` 만 부르고(직접 락·직접 캐시 접근·열거 정본 직접 호출·신원 재구현 없음),
            // 열거 정본 호출은 도우미 안에서 **두 번의 순간 캐시 락 사이**(선점 → IO → 기록)에서만 한다.
            let k = func("fn known_profile_identities(");
            assert!(k.contains("enumerated_profile_dirs(") && k.contains("folder_identity_state("), "열거 캐시 도우미 또는 B3 캐시 도우미(3값 읽기-통과)를 쓰지 않는다");
            for bad in ["lock(", "seat_ident_cache", "claude_identity", "read_identity_file(", "identity_file_meta(", "HashMap", "enumerate_profile_dirs(", "std::fs::"] {
                assert!(!k.contains(bad), "known_profile_identities 가 {bad} 를 쓴다(새 캐시·새 락 간선 · 열거를 캐시 밖에서 한다)");
            }
            let e = func("fn enumerated_profile_dirs(");
            let (l1, io, l2) = (e.find("seat_ident_cache.lock()").expect("락 1"), e.find("cys::profile_gate::enumerate_profile_dirs(").expect("열거 정본"), e.rfind("seat_ident_cache.lock()").expect("락 2"));
            assert!(l1 < io && io < l2, "열거 정본 호출이 두 순간 캐시 락 사이가 아니다(락을 쥔 채 read_dir)");
            assert!(e[l1..io].contains("\n    }\n"), "열거 앞에서 조회·선점 락 블록이 닫히지 않았다(락을 쥔 채 read_dir)");
            assert!(e[l1..io].contains("c.enumerated = Some("), "열거 전에 선점(`(now, 옛 값)`)이 없다 — 멈춘 디스크에서 호출마다 열거에 묶인다");
            // 순수 함수: 파일 IO·락 없음
            for head in ["fn sanitize_alias(", "fn parse_alias_table(", "fn alias_for<", "fn current_profiles_for("] {
                let f = func(head);
                for bad in ["std::fs::", "lock(", "dirs::", "OpenOptions"] {
                    assert!(!f.contains(bad), "순수 함수 {head} 가 {bad} 를 쓴다");
                }
            }
            // 홈은 한 곳(account_home)에서만 — 이 구역의 파일 판독이 dirs::home_dir 을 직접 부르지 않는다
            assert!(func("fn account_home(").contains("dirs::home_dir()"));
            assert!(!func("fn refresh_aliases(").contains("home_dir"), "refresh_aliases 가 홈을 직접 찾는다(시험 이음매 우회)");
        }

        /// 별명은 표시 전용 — 경보 경로(`alert_rates*`·`note_alert_input`·`merge_alert_windows`·`note_resolved`·스냅샷 복원)와 경보 모듈(alerts·governance·alert_route)이 별명 표를 참조하지 않는다(소스 핀).
        #[test]
        fn b1_alias_is_not_referenced_by_alert_code_source_pin() {
            let src = include_str!("accounts.rs");
            let func = |head: &str| -> String {
                let body = src.split(head).nth(1).unwrap_or_else(|| panic!("{head} 소실"));
                body[..body.find("\n}\n").unwrap_or_else(|| panic!("{head} 끝"))].to_string()
            };
            for head in [
                "pub fn alert_rates_with(",
                "pub fn alert_rates(",
                "fn note_alert_input(",
                "fn merge_alert_windows(",
                "fn note_resolved(",
                "fn restore_from_snapshots(",
                "fn feeds_alerts(",
            ] {
                let f = func(head);
                for bad in ["alias", "Alias"] {
                    assert!(!f.contains(bad), "경보 경로 {head} 가 별명({bad})을 참조한다 — 별명은 표시 전용이다");
                }
            }
            for (name, text) in [("alerts.rs", include_str!("alerts.rs")), ("governance.rs", include_str!("governance.rs")), ("alert_route.rs", include_str!("alert_route.rs"))] {
                for bad in ["AliasState", "alias_for(", ".alias.", "refresh_aliases"] {
                    assert!(!text.contains(bad), "{name} 가 별명 표({bad})를 참조한다 — 별명은 표시 전용이다");
                }
            }
        }

        // ───────────────────────── ★0.14.43(B1b) — 선언 계정 판독의 안전 판독(부트 체인) ─────────────────────────
        // `seed_known`·`spawn_custom_adapters` 는 소켓 bind **전**에 동기로 돈다(main.rs). 종전의 무제한 문자열 판독은 `~/.cys/accounts.json` 이 FIFO 면 열기에서 영원히 막혀 부트 체인 전체가 섰다.
        // 안전 판독: 일반 파일만(unix O_NONBLOCK + fstat) · 상한 1MiB(별명의 64KiB 와 별개) · 실패는 전부 '선언 계정 없음'(종전 `Err` 와 같다 · 파싱·적재는 무변경).
        // 픽스처 크기는 **리터럴**로 센다(상수를 바꾼 돌연변이가 거대한 픽스처를 만들지 않게). 합성 provider 이름(grokb1b 등)만 쓴다.

        const MIB: usize = 1024 * 1024;

        /// 선언 계정 한 건 + `pad` 로 총 `total` 바이트를 맞춘 JSON 본문. `extra` 는 `accounts` 와 같은 객체에 들어가는 추가 필드(예: `"aliases":{…},`).
        fn declared_body(entry: &str, extra: &str, total: usize) -> String {
            let stem = format!(r#"{{"accounts":[{entry}],{extra}"pad":""}}"#);
            assert!(stem.len() <= total, "픽스처가 너무 작다: {} > {total}", stem.len());
            let body = format!(r#"{{"accounts":[{entry}],{extra}"pad":"{}"}}"#, "p".repeat(total - stem.len()));
            assert_eq!(body.len(), total, "픽스처 크기 계산 오류");
            body
        }

        const GROK_NONE: &str = r#"{"provider":"grokb1b","label":"Grok B1b","adapter":"none"}"#;
        const GROK_CMD: &str = r#"{"provider":"grokb1b","label":"Grok B1b","adapter":"cmd","cmd":"true"}"#;

        /// 임시 홈(선언 계정 파일 `body` 가 있으면 그 내용으로)에서 부트 시드를 돌리고 (데몬 · 데몬 폴더 · 홈 · 홈 가드) 를 돌려준다 — 이 스레드의 `account_home()` 만 바뀐다(가드를 쥐고 있는 동안).
        fn boot_seed(tag: &str, body: Option<&[u8]>) -> (Arc<Daemon>, PathBuf, PathBuf, test_home::Guard) {
            let dir = tmp(&format!("b1b-{tag}-daemon"));
            let home = tmp(&format!("b1b-{tag}-home"));
            if let Some(b) = body {
                put_alias_bytes(&home, b, 1);
            }
            let guard = test_home::set(&home);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            seed_known(&d);
            (d, dir, home, guard)
        }

        /// 선언 계정 뷰 표 — (provider, account_id) → (라벨, 어댑터 여부 · 프로필 수 · 관측 고장 코드). 선언 계정만 있는 임시 홈이면 전체 뷰와 같다.
        fn view_table(d: &Arc<Daemon>) -> BTreeMap<(String, String), (String, bool, usize, Option<String>)> {
            let st = d.accounts.lock().unwrap();
            st.views
                .iter()
                .map(|(k, v)| ((k.provider.clone(), k.account_id.clone()), (v.label.clone(), v.adapter, v.profiles.len(), v.source_error.clone())))
                .collect()
        }

        /// 종전(B1b 이전) 선언 계정 적재 — 무제한 문자열 판독 + 같은 파싱·적재. 이 검체의 **차분 기준**이다(본문은 종전 코드 그대로 · 새 판독이 같은 JSON 에서 같은 뷰를 만드는지 잰다).
        fn old_declared_views(home: &Path) -> BTreeMap<(String, String), (String, bool, usize, Option<String>)> {
            let mut st = AccountsState::default();
            let decl = home.join(".cys/accounts.json");
            if let Ok(s) = std::fs::read_to_string(&decl) {
                if let Ok(v) = serde_json::from_str::<Value>(&s) {
                    for a in v.get("accounts").and_then(|x| x.as_array()).into_iter().flatten() {
                        let Some(provider) = a.get("provider").and_then(|x| x.as_str()) else {
                            continue;
                        };
                        let label = a.get("label").and_then(|x| x.as_str()).unwrap_or(provider).to_string();
                        let adapter = a.get("adapter").and_then(|x| x.as_str()).unwrap_or("none") != "none";
                        let key = AccountKey { provider: provider.into(), account_id: "default".into() };
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
            st.views
                .iter()
                .map(|(k, v)| ((k.provider.clone(), k.account_id.clone()), (v.label.clone(), v.adapter, v.profiles.len(), v.source_error.clone())))
                .collect()
        }

        /// 임시 홈의 `spawn_custom_adapters` 를 **돌지 않는 런타임** 안에서 부르고 살아 있는 어댑터 태스크 수를 돌려준다(런타임을 구동하지 않으므로 `sh` 는 뜨지 않는다 — 스폰된 태스크는 한 번도 폴링되지 않는다).
        fn adapter_tasks(d: &Arc<Daemon>, home: &Path) -> usize {
            let _h = test_home::set(home);
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            let _g = rt.enter();
            spawn_custom_adapters(d.clone());
            rt.metrics().num_alive_tasks()
        }

        /// ★(a) 일반 파일 — 종전과 같은 선언 계정 뷰(차분): 라벨 기본값(provider) · 어댑터 기본값(none) · `cmd` 어댑터 · provider 없음/문자열 아님/객체 아님은 건너뜀 · 중복 provider 는 첫 항목 ·
        /// 문자열이 아닌 라벨은 무시. 같은 JSON 을 종전 코드(무제한 문자열 판독)로 적재한 뷰와 **같다**.
        #[test]
        fn b1b_declared_accounts_views_match_the_old_loader_on_a_regular_file() {
            let body = r#"{"accounts":[
                {"provider":"grokb1b","label":"Grok B1b","adapter":"none"},
                {"provider":"glmb1b"},
                {"provider":"cmdb1b","label":"Cmd B1b","adapter":"cmd","cmd":"true","interval_secs":120},
                {"label":"제공자 없음"},
                {"provider":7,"label":"숫자 제공자"},
                {"provider":"dupb1b","label":"첫째"},
                {"provider":"dupb1b","label":"둘째","adapter":"cmd"},
                "문자열 항목",
                null,
                {"provider":"labelnumb1b","label":5}
              ],"aliases":{"Grok B1b":"그록"}}"#;
            let (d, dir, home, _h) = boot_seed("a", Some(body.as_bytes()));
            let got = view_table(&d);
            assert_eq!(got, old_declared_views(&home), "새 안전 판독이 종전과 다른 선언 계정 뷰를 만들었다");
            // 차분 기준이 헛돌지 않게 값도 직접 핀한다
            let k = |p: &str| (p.to_string(), "default".to_string());
            assert_eq!(got.len(), 5, "건너뛰어야 할 항목이 적재됐거나 적재해야 할 항목이 빠졌다: {got:?}");
            assert_eq!(got[&k("grokb1b")], ("Grok B1b".to_string(), false, 0, None));
            assert_eq!(got[&k("glmb1b")], ("glmb1b".to_string(), false, 0, None), "라벨 기본값 = provider · 어댑터 기본값 none");
            assert_eq!(got[&k("cmdb1b")], ("Cmd B1b".to_string(), true, 0, None));
            assert_eq!(got[&k("dupb1b")].0, "첫째", "중복 provider 는 첫 항목이 이긴다(종전 or_insert)");
            assert_eq!(got[&k("labelnumb1b")].0, "labelnumb1b", "문자열이 아닌 라벨은 무시");
            // 같은 파일의 별명도 읽혔다(두 판독이 한 파일을 나눠 쓴다)
            assert_eq!(alias_snap(&d, "Grok B1b").2, Some("그록".to_string()));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// ★(b) 선언 계정 파일이 **FIFO** 여도 부트 시드(`seed_known`)가 멈추지 않는다 — 5초 안에 끝나고 선언 계정 0 · 별명 0 · 발견(프로필 폴더)은 그대로 진행된다. (종전 판독이면 열기에서 영원히 막힌다.)
        #[cfg(unix)]
        #[test]
        fn b1b_fifo_accounts_json_does_not_stop_the_boot_seed() {
            let dir = tmp("b1b-b-daemon");
            let home = tmp("b1b-b-home");
            b3_login(&home, ".cys/claude-b1b", "u-b1b-a", "a-b1b@example.test", 1); // 발견 대상 하나 — 부트가 계속 진행됐는지의 증거
            let fifo = home.join(".cys/accounts.json");
            mkfifo(&fifo);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let (tx, rx) = std::sync::mpsc::channel();
            {
                let (d, home) = (d.clone(), home.clone());
                std::thread::spawn(move || {
                    let _h = test_home::set(&home);
                    seed_known(&d);
                    let _ = tx.send(local_json(&d, crate::state::now_epoch()));
                });
            }
            let out = rx.recv_timeout(std::time::Duration::from_secs(5));
            release_fifo(&fifo);
            let rows = out.expect("선언 계정 파일이 FIFO 인데 부트 시드(seed_known)가 멈췄다(소켓 bind 전 부트 체인 정지)");
            assert_eq!(rows.as_array().map(Vec::len), Some(1), "FIFO 선언 계정 파일에서 선언 계정이 생겼다(또는 발견이 빠졌다): {rows}");
            assert_eq!(row(&rows, "u-b1b-a")["alias"], Value::Null, "FIFO 에서 별명이 실렸다");
            {
                let st = d.accounts.lock().unwrap();
                assert!(st.alias.table.is_empty() && st.alias.reads == 0, "FIFO 를 열어 읽으려 했다(별명)");
            }
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// ★(b') 같은 FIFO 에서 `spawn_custom_adapters`(부트에서 `seed_known` 바로 다음 단계 · 같은 파일을 읽는다)도 멈추지 않는다 — 5초 안에 끝나고 어댑터 태스크 0.
        #[cfg(unix)]
        #[test]
        fn b1b_fifo_accounts_json_does_not_stop_spawn_custom_adapters() {
            let dir = tmp("b1b-b2-daemon");
            let home = tmp("b1b-b2-home");
            let fifo = home.join(".cys/accounts.json");
            mkfifo(&fifo);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let (tx, rx) = std::sync::mpsc::channel();
            {
                let (d, home) = (d.clone(), home.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(adapter_tasks(&d, &home));
                });
            }
            let out = rx.recv_timeout(std::time::Duration::from_secs(5));
            release_fifo(&fifo);
            assert_eq!(out.expect("선언 계정 파일이 FIFO 인데 spawn_custom_adapters 가 멈췄다(소켓 bind 전 부트 체인 정지)"), 0, "FIFO 에서 어댑터가 떴다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// ★(c) 상한 1MiB — 정확히 1MiB 는 적재 · 1바이트 넘으면 선언 계정 0(별명 0 · 어댑터 0). 두 판독(`seed_known`·`spawn_custom_adapters`)이 같은 상한을 쓴다.
        #[test]
        fn b1b_declared_accounts_file_over_one_mebibyte_is_no_declared_accounts() {
            let at = declared_body(GROK_CMD, "", MIB);
            let over = declared_body(GROK_CMD, "", MIB + 1);
            // 정확히 1MiB — 양성 대조
            {
                let (d, dir, home, _h) = boot_seed("c1", Some(at.as_bytes()));
                assert_eq!(view_table(&d).keys().cloned().collect::<Vec<_>>(), vec![("grokb1b".to_string(), "default".to_string())], "정확히 1MiB 파일을 거절했다");
                assert_eq!(adapter_tasks(&d, &home), 1, "정확히 1MiB 파일의 cmd 어댑터가 뜨지 않았다");
                let _ = std::fs::remove_dir_all(&dir);
                let _ = std::fs::remove_dir_all(&home);
            }
            // 1MiB + 1바이트
            {
                let (d, dir, home, _h) = boot_seed("c2", Some(over.as_bytes()));
                assert!(view_table(&d).is_empty(), "1MiB 를 넘는 파일에서 선언 계정이 적재됐다: {:?}", view_table(&d));
                assert_eq!(adapter_tasks(&d, &home), 0, "1MiB 를 넘는 파일에서 cmd 어댑터가 떴다");
                assert!(d.accounts.lock().unwrap().alias.table.is_empty());
                let _ = std::fs::remove_dir_all(&dir);
                let _ = std::fs::remove_dir_all(&home);
            }
        }

        /// ★(d) 두 상한은 **따로**다 — 64KiB 초과·1MiB 이하 파일은 선언 계정이 적재되고(1MiB 상한) 별명은 없다(64KiB 상한). 같은 파일을 64KiB 이하로 줄이면 별명이 나타난다(차이는 크기뿐).
        #[test]
        fn b1b_declared_accounts_between_the_two_caps_load_while_aliases_do_not() {
            let big = declared_body(GROK_NONE, r#""aliases":{"Grok B1b":"그록"},"#, 100 * 1024);
            assert!(big.len() > 64 * 1024 && big.len() <= MIB);
            let (d, dir, home, _h) = boot_seed("d", Some(big.as_bytes()));
            assert_eq!(view_table(&d).keys().cloned().collect::<Vec<_>>(), vec![("grokb1b".to_string(), "default".to_string())], "64KiB 를 넘는 파일의 선언 계정이 적재되지 않았다(상한이 별명과 같아졌다)");
            assert_eq!(alias_snap(&d, "Grok B1b"), (1, 0, None), "64KiB 를 넘는 파일에서 별명이 읽혔다(상한이 선언 계정과 같아졌다)");
            // 같은 내용을 64KiB 이하로 줄이면 별명이 나타난다
            let boot_at = d.accounts.lock().unwrap().alias.checked_at.expect("부트 확인 시각");
            put_aliases(&home, &declared_body(GROK_NONE, r#""aliases":{"Grok B1b":"그록"},"#, 2 * 1024), 2);
            refresh_aliases(&d, Some(&home), boot_at + 61.0);
            assert_eq!(alias_snap(&d, "Grok B1b"), (2, 1, Some("그록".to_string())), "파일을 줄였는데 별명이 나타나지 않았다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 읽을 수 없는 선언 계정 파일(디렉터리 · JSON 오류 · 빈 파일 · 루트가 배열 · `accounts` 가 배열 아님 · **UTF-8 아님**) = 선언 계정 없음(종전 `Err`/파싱 실패와 같다) — 두 판독 모두.
        /// UTF-8 아님 픽스처는 손실 변환(`from_utf8_lossy`)하면 유효한 선언이 되도록 만들었다(엄격 판독이 종전 의미임을 핀).
        #[test]
        fn b1b_unreadable_declared_files_mean_no_declared_accounts() {
            let mut bad_utf8 = br#"{"accounts":[{"provider":"grokb1b","label":""#.to_vec();
            bad_utf8.push(0xff);
            bad_utf8.extend_from_slice(br#"","adapter":"cmd","cmd":"true"}]}"#);
            let cases: Vec<(&str, Option<Vec<u8>>)> = vec![
                ("디렉터리", None),
                ("JSON 오류", Some(b"{not json".to_vec())),
                ("빈 파일", Some(Vec::new())),
                ("루트가 배열", Some(b"[]".to_vec())),
                ("accounts 가 배열 아님", Some(br#"{"accounts":{"provider":"grokb1b","adapter":"cmd","cmd":"true"}}"#.to_vec())),
                ("UTF-8 아님", Some(bad_utf8)),
            ];
            for (i, (what, body)) in cases.into_iter().enumerate() {
                let tag = format!("e{i}");
                let dir = tmp(&format!("b1b-{tag}-daemon"));
                let home = tmp(&format!("b1b-{tag}-home"));
                match &body {
                    Some(b) => put_alias_bytes(&home, b, 1),
                    None => std::fs::create_dir_all(home.join(".cys/accounts.json")).unwrap(), // 디렉터리
                }
                let _h = test_home::set(&home);
                let d = crate::state::Daemon::new(dir.join("cysd.sock"));
                seed_known(&d);
                assert!(view_table(&d).is_empty(), "{what}: 선언 계정이 적재됐다: {:?}", view_table(&d));
                assert_eq!(adapter_tasks(&d, &home), 0, "{what}: cmd 어댑터가 떴다");
                let _ = std::fs::remove_dir_all(&dir);
                let _ = std::fs::remove_dir_all(&home);
            }
        }

        /// `spawn_custom_adapters` 가 안전 판독으로도 종전처럼 일반 파일의 `cmd` 어댑터를 띄운다(양성 대조 — 위 음성 검체들이 헛돌지 않는다): provider·cmd 문자열 ∧ `adapter == "cmd"` 인 항목만 · 그 밖은 건너뜀.
        #[test]
        fn b1b_spawn_custom_adapters_still_spawns_the_cmd_adapters_of_a_regular_file() {
            let home = tmp("b1b-g-home");
            let _h = test_home::set(&home);
            put_aliases(
                &home,
                r#"{"accounts":[
                    {"provider":"grokb1b","cmd":"true","adapter":"cmd"},
                    {"provider":"glmb1b","cmd":"true","adapter":"cmd","interval_secs":10},
                    {"provider":"nocmdb1b","adapter":"cmd"},
                    {"provider":"noadapterb1b","cmd":"true"},
                    {"provider":"nonecmdb1b","cmd":"true","adapter":"none"},
                    {"cmd":"true","adapter":"cmd"},
                    {"provider":"badcmdb1b","cmd":7,"adapter":"cmd"}
                  ]}"#,
                1,
            );
            let dir = tmp("b1b-g-daemon");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            assert_eq!(adapter_tasks(&d, &home), 2, "cmd 어댑터 2건(grokb1b·glmb1b)만 떠야 한다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 소스 핀: 부트 체인의 두 판독이 선언 계정 파일을 **안전 판독으로만** 연다 — 직접 판독·직접 열기 0 · 공용 안전 판독 하나(별명·선언 계정이 같은 구현을 부른다 — 복사본 금지) ·
        /// 안전 판독의 규율(O_NONBLOCK · fstat 일반 파일 · 상한 인자) · 두 상한은 별개 값(선언 1MiB > 별명 64KiB).
        #[test]
        fn b1b_boot_chain_reads_declared_accounts_only_through_the_safe_reader_source_pin() {
            let src = include_str!("accounts.rs");
            let func = |head: &str| -> String {
                let body = src.split(head).nth(1).unwrap_or_else(|| panic!("{head} 소실"));
                body[..body.find("\n}\n").unwrap_or_else(|| panic!("{head} 끝"))].to_string()
            };
            for head in ["pub fn seed_known(", "pub fn spawn_custom_adapters("] {
                let f = func(head);
                assert!(f.contains("read_declared_accounts_text("), "{head} 가 안전 판독을 쓰지 않는다");
                for bad in ["read_to_string", "std::fs::read", "File::open", "OpenOptions", "dirs::home_dir"] {
                    assert!(!f.contains(bad), "{head} 가 {bad} 를 직접 쓴다 — 부트 체인의 FIFO 정지(안전 판독 우회) 또는 시험 이음매 우회");
                }
            }
            let r = func("fn read_regular_file_capped(");
            for must in ["O_NONBLOCK", "is_file()", "max_bytes", "take(max_bytes + 1)"] {
                assert!(r.contains(must), "공용 안전 판독에서 {must} 가 사라졌다");
            }
            assert!(func("fn read_alias_file(").contains("read_regular_file_capped(f, ALIAS_FILE_MAX_BYTES)"), "별명 판독이 공용 안전 판독(별명 상한)을 부르지 않는다");
            assert!(func("fn read_declared_accounts_text(").contains("read_regular_file_capped(decl, DECLARED_FILE_MAX_BYTES)"), "선언 계정 판독이 공용 안전 판독(선언 상한)을 부르지 않는다");
            assert_eq!(DECLARED_FILE_MAX_BYTES, 1024 * 1024, "선언 계정 상한은 1MiB 다");
            assert_eq!(ALIAS_FILE_MAX_BYTES, 64 * 1024, "별명 상한은 64KiB 다(선언 계정 상한과 별개)");
            // 프로덕션 구간 전체에서 선언 계정 파일을 무제한 판독으로 여는 곳이 없다
            let prod = &src[..src.find("\n#[cfg(test)]\nmod tests {").expect("테스트 모듈 경계")];
            assert!(!prod.contains("read_to_string(&decl)"), "프로덕션에 선언 계정 파일의 무제한 판독이 남아 있다");
        }
    }

    // ═════════ ★0.14.43(R1F-US · 성찰 1회차 수정) — 워치독의 신원 판독 제거 · 판독 실패 내성 · 열거 캐시 · 열거 밖 좌석 폴더 · 경보 키 메타 · 별명 ═════════
    // 픽스처는 전부 합성값이다(u-b3-a · a-b3@example.test · u-r1-* 등) — 실계정 식별자 금지. 시각은 주입한다(실시간 대기 없음 — 60초 하한은 `now` 로 센다). 홈은 임시 폴더이고 [`test_home`] 이음매가 **이 스레드의**
    // `account_home()` 만 바꾼다. 표시: 검체 이름 앞의 `r1f_` = 이 라운드의 새 검체(수정 전 코드에서 적색이거나 새 API 를 쓰는 것은 WORKLOG 의 돌연변이 표에 적색 로그를 붙였다).
    mod r1f_us {
        use super::b3_scenarios::{b3_events, b3_fixture, b3_login, b3_resolved, b3_seat, KEY_A};
        use super::*;

        fn row(rows: &Value, id: &str) -> Value {
            rows.as_array().unwrap().iter().find(|r| r["account_id"] == id).cloned().unwrap_or_else(|| panic!("행 {id} 없음: {rows}"))
        }

        /// 문자열 배열 → 정렬·중복 제거한 목록(윈도우의 `\` 는 `/` 로 접는다 — 화면의 `normalizeProfile` 과 같다).
        fn strs(v: &Value) -> Vec<String> {
            v.as_array()
                .unwrap_or_else(|| panic!("배열이 아니다: {v}"))
                .iter()
                .map(|x| x.as_str().unwrap().replace('\\', "/"))
                .collect::<BTreeSet<String>>()
                .into_iter()
                .collect()
        }

        fn s(items: &[&str]) -> Vec<String> {
            items.iter().map(|x| x.to_string()).collect()
        }

        /// 폴더 표 한 장 — 값 표기: `"A"` = `Known("A")` · `"-"` = `NoLogin`(신원 없음 확정) · `"?"` = `Unread`(이번에 읽지 못함).
        fn known_pairs(pairs: &[(&str, &str)]) -> Vec<(String, FolderWho)> {
            pairs
                .iter()
                .map(|(p, w)| {
                    let who = match *w {
                        "-" => FolderWho::NoLogin,
                        "?" => FolderWho::Unread,
                        a => FolderWho::Known(a.to_string()),
                    };
                    (p.to_string(), who)
                })
                .collect()
        }

        /// ★(R2F-DM · ⓑ) 윈도우의 열거 경로(`\\`)와 보고 경로(`/`)가 섞여 오는 `current_profiles` 를 이 모듈의 검체는 `strs` 로 접어 비교한다 — 그 도우미가 실제로 `\\` 를 `/` 로 접고 정렬·중복을 지우는지(맥에서도 잰다),
        /// 그리고 윈도우에서 붉던 두 검체(`r1f_a_logged_out_folder…` · `r1f_a_folder_another_request…`)가 날 `Value` 를 `==` 로 대조하지 않고 그 도우미를 거치는지(소스) 박는다.
        #[test]
        fn r2f_dm_strs_folds_windows_separators_and_the_two_windows_red_tests_use_it() {
            assert_eq!(strs(&json!([".cys\\claude-r1", ".cys/claude-r1", ".claude-r1x"])), s(&[".claude-r1x", ".cys/claude-r1"]), "구분자 접기 · 정렬 · 중복 제거");
            assert_eq!(strs(&json!([])), Vec::<String>::new());
            let src = include_str!("accounts.rs");
            for name in [
                concat!("fn r1f_a_logged_out_folder_or_one_without_an_identity_file", "_gives_an_empty_current_profiles_not_an_absent_key()"),
                concat!("fn r1f_a_folder_another_request_is_still_reading_for_the_first_time", "_is_unread_not_no_login()"),
            ] {
                let i = src.find(name).unwrap_or_else(|| panic!("{name} 소실"));
                let body = &src[i..i + src[i..].find("\n        }\n").expect("검체 끝")];
                assert!(body.contains("strs("), "{name}: 윈도우 구분자를 접는 `strs` 를 거치지 않는다 — 날 `Value` 를 `/` 리터럴과 비교하면 윈도우에서 붉다");
                assert!(!body.contains(".cloned(), Some(json!([\".cys/"), "{name}: `current_profiles` 날 값을 `/` 리터럴과 직접 비교한다(윈도우에서 `\\`)");
            }
        }

        const ID_W: &str = r#"{"oauthAccount":{"accountUuid":"u-r1-w","emailAddress":"w-r1@example.test"}}"#;

        /// 신원 파일을 임의 본문으로 쓴다(mtime 은 `bump` 로 맞춘다 — 같은 초 안에 다시 써도 mtime 이 달라지게).
        fn put_body(folder: &Path, body: &str, bump: u64) {
            let p = folder.join(".claude.json");
            write(&p, body);
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + bump);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }

        const ID_A: &str = r#"{"oauthAccount":{"accountUuid":"u-b3-a","emailAddress":"a-b3@example.test"}}"#;

        /// 좌석의 관측 스냅샷을 직접 싣는다 — 좌석 축 경보 입력(`rate_account` = 보고 시점 신원 · `rate_observed_at` = rate 가 새로 생산된 시각).
        fn put_usage(s: &Arc<crate::state::Surface>, pct: f64, resets: f64, observed_at: f64, rate_account: Option<&str>) {
            *s.observed_usage.lock().unwrap() = Some(ObservedUsage {
                agent: "claude".into(),
                ctx_tokens: None,
                ctx_window: None,
                ctx_pct: None,
                rate: vec![rw("5h", pct, Some(resets))],
                source: "statusline".into(),
                session_file: String::new(),
                updated_at: observed_at,
                rate_observed_at: observed_at,
                rate_account: rate_account.map(str::to_string),
            });
        }

        fn counters(d: &Arc<Daemon>) -> (u64, u64, usize) {
            let c = d.seat_ident_cache.lock().unwrap();
            (c.reads, c.hits, c.entries.len())
        }

        // ───────────────────────── [M-1] 워치독은 신원 파일을 읽지 않는다 ─────────────────────────

        /// ★M-1·①: 워치독이 쓰는 신원 표(`seat_identity_view_cached`)는 **판독 금지**다 — 캐시가 비면 None(판정 불가 · 모든 claude 계정 `None` = 사용 중으로 취급), 항목이 있으면 만료됐어도 **마지막 값**
        /// (신원 파일을 바꾸거나 지워도 본 값은 그대로 · 계측 `reads`·`hits`·항목 수 불변). 읽기-통과 조회가 지나가야 새 값이 실린다.
        #[test]
        fn r1f_watchdog_view_is_cache_only_an_empty_cache_is_unknown_and_an_expired_entry_keeps_its_last_value() {
            let (d, dir, home, f) = b3_fixture("r1a");
            let seat = b3_seat(&d, "worker-r1", "claude", Some(&f));
            let key = f.to_string_lossy().into_owned();
            // ① 캐시가 비었다 → 판정 불가(None) · 어떤 파일도 읽지 않았다
            let v = seat_identity_view_cached(&d);
            assert!(v.collect_ok);
            assert_eq!(v.claude_folders, vec![(key.clone(), None)], "빈 캐시인데 폴더 신원이 판독됐다(워치독이 파일을 읽었다)");
            assert_eq!(v.current_for(seat.id), None);
            assert_eq!(account_in_use("claude", "u-b3-a", &v), None, "빈 캐시 = 판정 불가 = 사용 중으로 취급(경보 유지)");
            assert_eq!(counters(&d), (0, 0, 0), "캐시 전용 조회가 계측·항목을 건드렸다");
            // ② 읽기-통과 조회가 캐시를 채운다 — 아주 옛 시각(1970)으로 채워 '이미 만료된 항목'을 만든다
            assert_eq!(seat_identity_view_in(&d, Some(&home), 1000.0).current_for(seat.id), Some("u-b3-a"));
            assert_eq!(counters(&d), (1, 0, 1));
            // ③ 만료된 항목도 마지막 값을 낸다 · 신원 파일을 B 로 바꾸거나 지워도 본 값은 그대로(파일을 열지도 stat 하지도 않는다) · 계측 불변
            b3_login(&home, ".cys/claude-b3", "u-b3-b", "b-b3@example.test", 9);
            assert_eq!(seat_identity_view_cached(&d).current_for(seat.id), Some("u-b3-a"), "워치독 조회가 바뀐 신원 파일을 읽었다");
            std::fs::remove_file(f.join(".claude.json")).unwrap();
            let v = seat_identity_view_cached(&d);
            assert_eq!(v.current_for(seat.id), Some("u-b3-a"), "워치독 조회가 지워진 신원 파일을 봤다");
            assert_eq!(account_in_use("claude", "u-b3-a", &v), Some(true));
            assert_eq!(counters(&d), (1, 0, 1), "캐시 전용 조회가 계측·항목을 건드렸다");
            // ④ 읽기-통과 조회(60초 하한이 지난 시각)가 지나가야 새 값이 실린다 — 지워진 신원은 판독 불가(None)
            assert_eq!(seat_identity_view_in(&d, Some(&home), 2000.0).current_for(seat.id), None);
            assert_eq!(seat_identity_view_cached(&d).current_for(seat.id), None);
            assert_eq!(counters(&d).0, 2);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// ★M-1 핵심(동작): 경보 틱(`check_alerts_with`)은 신원 파일을 읽지 않는다 — 좌석이 A 로 보고한 뒤 폴더 로그인이 B 로 바뀌어도(보고·조회 없이) +1801초 틱은 캐시의 A 를 쓴다(A 는 사용 중 → 경보 유지 · REMIND 재발행).
        /// 읽기-통과 조회(RPC)가 한 번 지나가면 그때 비로소 B 가 실려 A 가 소멸한다(해소 알림 stale 1건). 종전 코드는 틱이 만료된 캐시를 스스로 다시 읽어 +1801초에 A 를 소멸시켰다(적색).
        #[test]
        fn r1f_alert_tick_never_reads_the_identity_file_so_a_login_switch_is_seen_only_after_a_report_or_a_query() {
            let (d, dir, home, f) = b3_fixture("r1b");
            let cfg = crate::alerts::AlertConfig::default();
            let mut fired: HashMap<String, f64> = HashMap::new();
            let _seat = b3_seat(&d, "worker-r1", "claude", Some(&f));
            let t0 = crate::state::now_epoch();
            let seq0 = d.bus.latest_seq();
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1.0);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 1, "전제: A 99% 가 발화했다");
            // 폴더 로그인을 B 로 바꾼다 — 그러나 보고도 조회도 없다
            b3_login(&home, ".cys/claude-b3", "u-b3-b", "b-b3@example.test", 2);
            for dt in [70.0, 1799.0, 1801.0] {
                crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + dt);
            }
            assert_eq!(account_alerts(&d, seq0, KEY_A), 2, "틱이 신원 파일을 다시 읽어 +1801초에 A 를 소멸시켰다(워치독이 파일을 읽는다) — 캐시의 A(사용 중)면 REMIND 재발행이 맞다");
            assert!(b3_resolved(&d, seq0).is_empty(), "캐시의 신원이 A 인데 해소 알림이 나갔다: {:?}", b3_resolved(&d, seq0));
            assert_eq!(d.seat_ident_cache.lock().unwrap().reads, 0, "경보 틱이 신원 실판독 계수를 올렸다");
            // 읽기-통과 조회(RPC)가 한 번 지나가면(60초 하한이 지난 시각) 새 신원이 캐시에 실리고 다음 틱이 그것을 본다 → A 는 더는 쓰이지 않는다(관측 1800초 초과) → 소멸
            let _ = seat_identity_view_in(&d, Some(&home), t0 + 1802.0);
            assert_eq!(d.seat_ident_cache.lock().unwrap().reads, 1);
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1810.0);
            assert_eq!(account_alerts(&d, seq0, KEY_A), 2, "소멸해야 할 A 가 다시 발행됐다");
            assert_eq!(b3_resolved(&d, seq0), vec![(KEY_A.to_string(), "stale".to_string())], "RPC 조회가 새 신원을 실은 뒤의 틱에서 A 가 붙들림 소멸(stale)해야 한다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// ★M-1·②: 상태줄 보고가 좌석 신원 캐시를 채운다(추가 IO 0) — 보고 직후 읽기-통과 조회는 파일을 다시 읽지 않고(실판독 0) 캐시 전용 조회도 그 신원을 본다. 로그인이 바뀐 뒤의 새 보고는 캐시를 새 신원으로 갈고,
        /// 더 새로 기록된 항목은 옛 시각의 보고로 덮지 않으며, 창 밖 보고(표시용)는 캐시를 채우지 않는다.
        #[test]
        fn r1f_a_report_fills_the_identity_cache_without_reading_again() {
            let (d, dir, home, f) = b3_fixture("r1c");
            let seat = b3_seat(&d, "worker-r1", "claude", Some(&f));
            let t0 = crate::state::now_epoch();
            assert_eq!(counters(&d), (0, 0, 0));
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 40.0, Some(t0 + 7200.0))], "statusline", t0));
            assert_eq!(counters(&d), (0, 0, 1), "보고가 캐시를 채우지 않았다(또는 추가 판독을 했다)");
            assert_eq!(seat_identity_view_cached(&d).current_for(seat.id), Some("u-b3-a"));
            // 읽기-통과 조회(하한 안)는 적중 — 파일을 다시 읽지 않는다
            assert_eq!(seat_identity_view_in(&d, Some(&home), t0 + 10.0).current_for(seat.id), Some("u-b3-a"));
            assert_eq!(counters(&d), (0, 1, 1), "보고 직후 조회가 파일을 다시 읽었다");
            // 로그인이 B 로 바뀐 뒤 새 보고(B)가 오면 캐시가 B 로 갈린다 — 창이 닫혀 있어도(조회 없이) 워치독 표가 B 를 본다
            b3_login(&home, ".cys/claude-b3", "u-b3-b", "b-b3@example.test", 2);
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 10.0, Some(t0 + 9000.0))], "statusline", t0 + 20.0));
            assert_eq!(seat_identity_view_cached(&d).current_for(seat.id), Some("u-b3-b"), "새 로그인으로 보고했는데 캐시가 옛 신원이다");
            // 더 새로 기록된 항목은 옛 시각의 보고가 덮지 않는다(t0+20 에 B 를 기록한 뒤, 파일을 A 로 되돌려 t0+5 시각으로 보고해도 캐시는 B 그대로)
            b3_login(&home, ".cys/claude-b3", "u-b3-a", "a-b3@example.test", 3);
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 11.0, Some(t0 + 9000.0))], "statusline", t0 + 5.0));
            assert_eq!(seat_identity_view_cached(&d).current_for(seat.id), Some("u-b3-b"), "더 새로 기록된 항목을 옛 시각의 보고가 덮었다");
            // 시계가 크게 뒤로 간 경우(60초보다 먼 미래의 항목)는 만료로 보고 덮는다 — 캐시가 영구히 붙지 않는다(`folder_identity` 와 같은 규율)
            let key = f.to_string_lossy().into_owned();
            assert_eq!(folder_identity(&d, Some(&home), &key, t0 + 100_000.0).as_deref(), Some("u-b3-a"), "전제: 먼 미래 시각의 항목(A)");
            b3_login(&home, ".cys/claude-b3", "u-b3-b", "b-b3@example.test", 4);
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 12.0, Some(t0 + 9000.0))], "statusline", t0 + 40.0));
            assert_eq!(seat_identity_view_cached(&d).current_for(seat.id), Some("u-b3-b"), "시계가 크게 뒤로 갔는데 먼 미래의 항목을 보고가 덮지 못했다(캐시가 영구히 붙는다)");
            // 창 밖 보고(표시용)는 캐시를 채우지 않는다
            let o = outside_profile(&home, ".claude-r1o", "u-r1-o", "o-r1@example.test");
            let before = counters(&d).2;
            assert_eq!(report_outside_at(&d, Some(&home), &o, &[rw("5h", 5.0, None)], t0 + 30.0), Ok(OutsideOutcome::Accepted));
            assert_eq!(counters(&d).2, before, "창 밖 보고가 좌석 신원 캐시를 채웠다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// ★M-1·③ 선점: 읽기-통과는 캐시 미스일 때 IO **전에** `(now, 옛 값)` 으로 자리를 찍는다 — 판독이 멈춰도(여기서는 accounts 락을 쥔 채 판독 스레드를 세워 흉내 낸다 — `claude_identity_probe` 가 mtime 캐시 조회에서 그 락에
        /// 막힌다) 같은 폴더를 다시 묻는 호출은 막히지 않고 옛 값을 받는다(폴더당 60초에 요청 하나만 묶인다). 선점이 없으면 두 번째 호출도 같은 락에서 막힌다(적색 — 검체 스레드가 남지 않게 마지막에 락을 놓는다).
        #[test]
        fn r1f_a_stalled_read_blocks_one_request_per_folder_per_minute_because_the_slot_is_taken_before_the_io() {
            let (d, dir, home, f) = b3_fixture("r1d");
            let key = f.to_string_lossy().into_owned();
            let t0 = 1_800_000_000.0;
            assert_eq!(folder_identity(&d, Some(&home), &key, t0).as_deref(), Some("u-b3-a"));
            let guard = d.accounts.lock().unwrap(); // '멈춘 IO' — 판독이 이 락에서 멈춘다
            let (tx1, rx1) = std::sync::mpsc::channel();
            {
                let (d1, home1, key1) = (d.clone(), home.clone(), key.clone());
                std::thread::spawn(move || {
                    let _ = tx1.send(folder_identity(&d1, Some(&home1), &key1, t0 + 61.0));
                });
            }
            // 첫 호출이 IO 에 들어가기 전에 자리를 찍었다 — 캐시 항목의 확인 시각이 t0+61 이 된다
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let marked = loop {
                let at = d.seat_ident_cache.lock().unwrap().entries.get(Path::new(&key)).map(|slot| slot.at);
                if at == Some(t0 + 61.0) {
                    break true;
                }
                if std::time::Instant::now() > deadline {
                    break false;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            // 두 번째 호출(하한 안) — 막히지 않고 옛 값을 받는다
            let (tx2, rx2) = std::sync::mpsc::channel();
            if marked {
                let (d2, home2, key2) = (d.clone(), home.clone(), key.clone());
                std::thread::spawn(move || {
                    let _ = tx2.send(folder_identity(&d2, Some(&home2), &key2, t0 + 62.0));
                });
            }
            let second = marked.then(|| rx2.recv_timeout(std::time::Duration::from_secs(3)));
            drop(guard); // '멈춘 IO' 해제 — 첫 호출이 끝난다
            let first = rx1.recv_timeout(std::time::Duration::from_secs(5));
            assert!(marked, "읽기-통과가 IO 전에 자리를 찍지 않았다(멈춘 디스크에서 같은 폴더를 묻는 모든 호출이 같은 IO 에 묶인다)");
            assert_eq!(
                second.expect("marked").expect("두 번째 호출이 첫 호출의 멈춘 IO 에 묶였다").as_deref(),
                Some("u-b3-a"),
                "선점 중 다른 호출은 옛 값을 받는다"
            );
            assert_eq!(first.expect("첫 호출이 끝나지 않았다").as_deref(), Some("u-b3-a"));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 소스 핀(M-1): 워치독 진입점(`check_alerts_with_stale`)에서 닿는 **신원 경로**의 함수 사슬에 신원 파일 접근이 없다 — 진입점은 캐시 전용 스냅샷만 부르고, 사슬의 함수(캐시 전용 스냅샷 · 본체 · 캐시 전용 신원 표 ·
        /// 캐시 조회 · 좌석 표 복사 · 조립 · 계정 축 행)에 `folder_identity(`(읽기-통과)·`claude_identity_*`·`std::fs::`·`identity_file_meta(`·`read_identity_*`·`enumerate_profile_dirs(` 가 없다.
        /// (상태 폴더 `read_dir`(게이트 배지)·팩 설정 판독은 0.14.42 에도 있던 별개 경로 — 신원이 아니다.)
        #[test]
        fn r1f_watchdog_identity_chain_has_no_file_access_source_pin() {
            let (acc, alr, gov, usg) = (include_str!("accounts.rs"), include_str!("alerts.rs"), include_str!("governance.rs"), include_str!("usage.rs"));
            let func = |src: &str, head: &str| -> String {
                let body = src.split(head).nth(1).unwrap_or_else(|| panic!("{head} 소실"));
                body[..body.find("\n}\n").unwrap_or_else(|| panic!("{head} 끝"))].to_string()
            };
            let forbidden = [
                "folder_identity(",
                "folder_identity_state(",
                "claude_identity_unlocked(",
                "claude_identity_probe(",
                "claude_identity_at(",
                "std::fs::",
                "identity_file_meta(",
                "read_identity_file(",
                "read_identity_outcome(",
                "enumerate_profile_dirs(",
                "enumerated_profile_dirs(",
                "known_profile_identities(",
                // ★(R2F-DM · 성찰 2회차 A1 m-1 ⓑ) `use` 로 접두를 줄인 판독 — `std::fs::` 접두 꼴만 막으면 `File::open(`·`OpenOptions`·`metadata(`·`read_dir(` 단독 꼴은 지나갔다.
                "File::open",
                "OpenOptions",
                "metadata(",
                "read_dir(",
            ];
            let g = func(gov, "pub(crate) fn check_alerts_with_stale(");
            assert!(g.contains("snapshot_cached_with_stale("), "워치독 진입점이 캐시 전용 스냅샷을 부르지 않는다");
            for bad in ["snapshot_with_stale(", "alerts::snapshot(", "seat_identity_view_at(", "seat_identity_view_in(", "seat_identity_view("] {
                assert!(!g.contains(bad), "워치독 진입점이 읽기-통과 경로({bad})를 부른다");
            }
            for (src, head) in [
                (alr, "pub fn snapshot_cached_with_stale("),
                (alr, "fn snapshot_from_view("),
                (acc, "pub fn seat_identity_view_cached("),
                (acc, "fn peek_folder_ident("),
                (acc, "fn peek_folder_state("),
                (acc, "fn collect_seat_rows("),
                (acc, "fn assemble_seat_view("),
                (acc, "pub fn alert_rates_with("),
                // ★(R2F-DM · 성찰 2회차 A1 m-1 ⓑ) 사슬에 실제로 있는데 목록에 없던 여섯 + 진입점 둘 — 지금은 전부 순수·위임이지만 여기에 판독이 들어가도 초록이었다.
                (acc, "pub fn account_in_use("),
                (acc, "pub fn seat_in_use_tri("),
                (acc, "pub fn alert_eligible("),
                (alr, "fn keep_best("),
                (alr, "fn reset_in_secs("),
                (usg, "pub fn rate_window_live("),
                (gov, "fn check_alerts("),
                (gov, "pub(crate) fn check_alerts_with("),
            ] {
                let f = func(src, head);
                for bad in forbidden {
                    assert!(!f.contains(bad), "워치독 사슬 {head} 가 {bad} 를 쓴다(신원 파일 접근)");
                }
            }
            assert!(func(alr, "fn snapshot_from_view(").contains("alert_rates_with(") && !func(alr, "fn snapshot_from_view(").contains("seat_identity_view"), "본체가 신원 표를 직접 만든다");
            // 캐시 전용 조회는 캐시에 쓰지 않는다(항목·계측 무변)
            let v = func(acc, "pub fn seat_identity_view_cached(");
            assert!(v.contains("peek_folder_ident(") && !v.contains("seat_ident_cache") && !v.contains("entries"), "캐시 전용 신원 표가 캐시를 직접 만진다");
            for head in ["fn peek_folder_ident(", "fn peek_folder_state("] {
                let p = func(acc, head);
                for bad in ["insert(", ".reads", ".hits", "retain(", "clear("] {
                    assert!(!p.contains(bad), "캐시 전용 조회 {head} 가 캐시를 바꾼다({bad})");
                }
            }
            // 읽기-통과(folder_identity)에는 선점이 IO(판독) 앞에 있다 — 선점 → 판독 → 기록
            let fi = func(acc, "pub(crate) fn folder_identity_state(");
            let (preempt, io, record) = (fi.find("c.entries.insert(").expect("선점"), fi.find("claude_identity_probe(").expect("판독"), fi.rfind("c.entries.insert(").expect("기록"));
            assert!(preempt < io && io < record, "folder_identity_state 가 IO 전에 자리를 찍지 않는다(선점 → 판독 → 기록 순서)");
        }

        /// 캐시가 데워진 뒤에는 읽기-통과 스냅샷과 캐시 전용 스냅샷이 **같은 입력**을 낸다(좌석 축·계정 축 값·메타 · 붙들 키) — 두 진입점이 갈라지지 않는다.
        /// 계정 B(좌석이 쓰지 않는 로그인 · 관측 3000초 전 97%)는 신선도 규칙으로 빠져 붙들 키가 된다 — 캐시가 비었다면(= 신원을 몰랐다면) B 는 '판정 불가 = 사용 중'으로 적격이 되어 두 스냅샷이 갈라진다.
        #[test]
        fn r1f_cached_and_read_through_snapshots_agree_once_the_cache_is_warm() {
            let (d, dir, home, f) = b3_fixture("r1p");
            let fb = b3_login(&home, ".cys/claude-b3-b", "u-b3-b", "b-b3@example.test", 2); // 좌석이 쓰지 않는 로그인 B
            let seat = b3_seat(&d, "worker-r1", "claude", Some(&f));
            let t0 = crate::state::now_epoch();
            assert!(note_rate_for_profile_at(&d, Some(&home), &f, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0));
            assert!(note_rate_for_profile_at(&d, Some(&home), &fb, &[rw("5h", 97.0, Some(t0 + 7200.0))], "statusline", t0 - 3000.0));
            put_usage(&seat, 97.0, t0 + 7200.0, t0, Some("u-b3-a"));
            let _ = seat_identity_view_in(&d, Some(&home), t0);
            let cfg = crate::alerts::AlertConfig::default();
            let a = crate::alerts::snapshot_with_stale(&d, t0 + 30.0, 1800.0);
            let b = crate::alerts::snapshot_cached_with_stale(&d, t0 + 30.0, 1800.0, &cfg);
            assert_eq!((&a.rates, &a.account_rates), (&b.rates, &b.account_rates));
            assert_eq!((&a.rate_meta, &a.account_meta), (&b.rate_meta, &b.account_meta));
            assert_eq!(a.stale_suppressed, b.stale_suppressed);
            assert_eq!(a.stale_suppressed, vec!["account_rate:b-b3@example.test:5h".to_string()], "B(사용 중 아님 · 관측 3000초)는 붙들 키여야 한다 — 두 진입점 모두");
            assert_eq!(a.account_rates, vec![("a-b3@example.test".to_string(), "5h".to_string(), 99.0)], "A 만 계정 축 입력이다");
            assert!(!a.rates.is_empty() && !a.account_rates.is_empty(), "전제: 두 축 모두 입력이 있어야 한다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        // ───────────────────────── [m-5] 프로필 폴더 열거도 60초 하한 캐시 ─────────────────────────

        /// ★m-5: 알려진 프로필 폴더 열거(홈·`~/.cys` 의 `read_dir`)는 60초 하한이다 — 하한 안에 만든 새 폴더는 보이지 않고(다시 열거하지 않는다) 하한이 지나면 보인다 · 시계 역행은 만료 · 5초 폴링(`local_json`) 12회에 열거 1회.
        #[test]
        fn r1f_profile_folder_listing_is_cached_for_sixty_seconds() {
            let dir = tmp("r1g-daemon");
            let home = tmp("r1g-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            b3_login(&home, ".claude-r1a", "u-r1-a", "a-r1@example.test", 1);
            let t0 = 1_800_000_000.0;
            let names = |d: &Arc<Daemon>, t: f64| -> Vec<String> {
                let mut v: Vec<String> = known_profile_identities(d, Some(&home), t).expect("열거 성공").into_iter().map(|(p, _)| p.replace('\\', "/")).collect();
                v.sort();
                v
            };
            let enum_reads = |d: &Arc<Daemon>| d.seat_ident_cache.lock().unwrap().enum_reads;
            assert_eq!(names(&d, t0), s(&[".claude-r1a"]));
            assert_eq!(enum_reads(&d), 1);
            b3_login(&home, ".claude-r1b", "u-r1-b", "b-r1@example.test", 2);
            assert_eq!(names(&d, t0 + 30.0), s(&[".claude-r1a"]), "60초 안인데 홈을 다시 열거했다(5초 폴링이 read_dir 을 늘린다)");
            assert_eq!(enum_reads(&d), 1);
            assert_eq!(names(&d, t0 + 60.0), s(&[".claude-r1a", ".claude-r1b"]), "하한이 지났는데 새 폴더가 보이지 않는다");
            assert_eq!(enum_reads(&d), 2);
            // 시계가 뒤로 가면(now < 확인 시각) 만료로 본다 — 캐시가 영구히 붙지 않는다
            let _ = names(&d, t0 - 5.0);
            assert_eq!(enum_reads(&d), 3);
            // 5초 폴링 12회(Control Center Live 의 usage.accounts·control.dashboard)에 열거 1회
            let _h = test_home::set(&home);
            let base = enum_reads(&d);
            for i in 0..12 {
                let _ = local_json(&d, t0 + 200.0 + 5.0 * f64::from(i));
            }
            assert_eq!(enum_reads(&d), base + 1, "5초 폴링 12회에 홈 열거가 1회가 아니다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 열거 실패와 '프로필 폴더가 정말 없음'을 가른다 — 정본(`enumerate_profile_dirs`)은 읽기 오류를 조용히 빈 목록으로 접으므로 홈 `read_dir` 가 되는지로 가른다: 홈이 있으면 `Some([])`, 홈이 없으면 `None`.
        #[test]
        fn r1f_a_failed_listing_is_not_the_same_as_no_profile_folders() {
            let dir = tmp("r1j-daemon");
            let empty_home = tmp("r1j-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            assert_eq!(enumerated_profile_dirs(&d, &empty_home, 1_800_000_000.0), Some(Vec::new()), "프로필 폴더가 하나도 없는 홈은 열거 성공(빈 목록)이다");
            let missing = empty_home.join("not-there");
            assert_eq!(enumerated_profile_dirs(&d, &missing, 1_800_000_000.0), None, "읽을 수 없는 홈은 열거 실패(None)다");
            assert_eq!(known_profile_identities(&d, Some(&missing), 1_800_000_100.0), None);
            assert_eq!(known_profile_identities(&d, None, 1_800_000_100.0), None, "홈 불명도 목록을 못 얻은 것이다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&empty_home);
        }

        // ───────────────────────── [m-2] 신원 판독이 한 번 실패해도 '이전 로그인'을 단정하지 않는다 ─────────────────────────

        /// ★m-2 ⓐ: 신원 파일은 있는데(메타 성공) 이번 판독·파싱만 실패하면 직전 신원을 **한 주기(60초)만** 더 쓴다 — 연속 실패면 다음 주기에는 None. 정상으로 읽히면 카운트가 돌아온다.
        /// 파일이 없어진 것 · `oauthAccount` 가 없는 것(로그아웃)은 종전처럼 즉시 None.
        #[test]
        fn r1f_a_failed_read_keeps_the_previous_identity_for_one_cycle_only() {
            let dir = tmp("r1e-daemon");
            let home = tmp("r1e-home");
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let f = b3_login(&home, ".cys/claude-b3", "u-b3-a", "a-b3@example.test", 1);
            let key = f.to_string_lossy().into_owned();
            let id = |t: f64| folder_identity(&d, Some(&home), &key, t);
            let t0 = 1_800_000_000.0;
            assert_eq!(id(t0).as_deref(), Some("u-b3-a"));
            // 쓰는 도중의 잘린 파일(새 mtime — 판독은 실패한다)
            put_body(&f, "{\"oauthAccount\":", 2);
            assert_eq!(id(t0 + 60.0).as_deref(), Some("u-b3-a"), "일시 실패 첫 주기에 직전 신원을 쓰지 않았다('이전 로그인' 단정)");
            // 하한(60초) 안에서는 다시 판독하지 않는다(실패 중에도 stat 폭주 없음)
            let reads = d.seat_ident_cache.lock().unwrap().reads;
            assert_eq!(id(t0 + 100.0).as_deref(), Some("u-b3-a"));
            assert_eq!(d.seat_ident_cache.lock().unwrap().reads, reads, "실패한 항목이 60초 하한 안에서 다시 판독됐다");
            assert_eq!(id(t0 + 120.0), None, "연속 실패인데 직전 신원이 한 주기 더 연장됐다(무한 연장)");
            // 정상 복구 → 카운트 리셋 → 새 실패(빈 파일 — 쓰기 직전)에서 한 주기 연장이 되살아난다
            put_body(&f, ID_A, 3);
            assert_eq!(id(t0 + 180.0).as_deref(), Some("u-b3-a"));
            put_body(&f, "", 4);
            assert_eq!(id(t0 + 240.0).as_deref(), Some("u-b3-a"), "복구 뒤 새 실패에서 한 주기 연장이 없다");
            assert_eq!(id(t0 + 300.0), None);
            // 확정 '신원 없음'은 연장하지 않는다 — 파일이 사라짐 · 로그인 정보(oauthAccount)가 없는 정상 JSON(로그아웃)
            let f2 = b3_login(&home, ".cys/claude-b3-2", "u-b3-b", "b-b3@example.test", 5);
            let k2 = f2.to_string_lossy().into_owned();
            assert_eq!(folder_identity(&d, Some(&home), &k2, t0).as_deref(), Some("u-b3-b"));
            std::fs::remove_file(f2.join(".claude.json")).unwrap();
            assert_eq!(folder_identity(&d, Some(&home), &k2, t0 + 60.0), None, "파일이 사라졌는데 직전 신원을 연장했다");
            let f3 = b3_login(&home, ".cys/claude-b3-3", "u-b3-c", "c-b3@example.test", 6);
            let k3 = f3.to_string_lossy().into_owned();
            assert_eq!(folder_identity(&d, Some(&home), &k3, t0).as_deref(), Some("u-b3-c"));
            put_body(&f3, "{\"projects\":{}}", 7);
            assert_eq!(folder_identity(&d, Some(&home), &k3, t0 + 60.0), None, "로그아웃(oauthAccount 없음)인데 직전 신원을 연장했다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 판독 3값 표(순수) — JSON 이 아니면(빈 문자열·잘린 본문) `Unreadable`, JSON 인데 `oauthAccount`·`accountUuid` 가 없거나 문자열이 아니면 `NoIdentity`, 있으면 `Found`.
        #[test]
        fn r1f_parse_identity_outcome_table() {
            let kind = |body: &str| match parse_identity_outcome(body) {
                IdentRead::Found((u, e, _)) => format!("found:{u}:{e}"),
                IdentRead::NoIdentity => "none".to_string(),
                IdentRead::Unreadable => "unreadable".to_string(),
            };
            assert_eq!(kind(ID_A), "found:u-b3-a:a-b3@example.test");
            assert_eq!(kind(r#"{"oauthAccount":{"accountUuid":"u-x"}}"#), "found:u-x:u-x", "이메일이 없으면 uuid 가 라벨");
            for none in ["{}", "[]", "null", "5", r#""x""#, r#"{"oauthAccount":null}"#, r#"{"oauthAccount":{}}"#, r#"{"oauthAccount":{"accountUuid":7}}"#, r#"{"oauthAccount":{"accountUuid":null}}"#] {
                assert_eq!(kind(none), "none", "{none:?}");
            }
            for bad in ["", "   ", "{", "{\"oauthAccount\":", "{not json", "\u{feff}{}"] {
                assert_eq!(kind(bad), "unreadable", "{bad:?}");
            }
        }

        /// ★m-2 ⓑ(순수 · 보정): `current_profiles_for` 가 **None = 키를 내지 않는다** — claude 행에서 ① 폴더 표를 못 얻었거나(홈 불명·열거 실패) ② 그 행의 `profiles` 에 든 폴더 가운데 이번에 신원을 **읽지 못한** 것(표에서 `Unread`)이 있으면.
        /// **신원 없음 확정(`NoLogin` — 로그아웃·신원 파일 없음)은 읽지 못한 것이 아니다** — '이 계정의 폴더가 아니다'로 세어 그 계정의 다른 폴더가 없으면 `[]`(9650334f 와 같다). 표에 없는 폴더(열거 밖 · 삭제됨)도 '읽지 못함'이 아니다 ·
        /// 구분자(`\` 와 `/`)는 같은 것 · codex·agy·선언 계정은 언제나 profiles 그대로.
        #[test]
        fn r1f_current_profiles_key_is_absent_only_for_an_unread_folder_or_a_failed_listing() {
            let ps = |items: &[&str]| -> BTreeSet<String> { items.iter().map(|x| x.to_string()).collect() };
            let k = known_pairs(&[(".cys/claude-r1", "A"), (".claude-r1x", "B"), (".claude-r1bad", "?"), (".claude-r1out", "-")]);
            // 읽은 폴더만 가진 행 → 키를 낸다
            assert_eq!(current_profiles_for("claude", "A", &ps(&[".cys/claude-r1"]), Some(&k)), Some(s(&[".cys/claude-r1"])));
            assert_eq!(current_profiles_for("claude", "C", &ps(&[".claude-r1gone"]), Some(&k)), Some(Vec::new()), "표에 없는 폴더(삭제·열거 밖)는 '읽지 못함'이 아니다 — 이전 로그인이 맞다");
            // ★보정 (가): 신원 없음 확정(NoLogin — 로그아웃·신원 파일 없음) 폴더만 가진 계정 → 키 부재가 아니라 `[]`('이전 로그인')
            assert_eq!(current_profiles_for("claude", "Z", &ps(&[".claude-r1out"]), Some(&k)), Some(Vec::new()), "신원 없음 확정 폴더만 가진 계정이 키를 내지 않았다 — 읽지 못함으로 섞었다");
            // NoLogin 폴더와 자기 폴더가 섞인 행 → 자기 폴더만(NoLogin 은 이 계정의 폴더가 아니다)
            assert_eq!(current_profiles_for("claude", "A", &ps(&[".cys/claude-r1", ".claude-r1out"]), Some(&k)), Some(s(&[".cys/claude-r1"])));
            // ★보정 (나): 읽지 못한(Unread) 폴더가 profiles 에 든 행 → 키 부재(빈 배열이 아니다) — NoLogin 이 섞여 있어도 마찬가지
            assert_eq!(current_profiles_for("claude", "A", &ps(&[".cys/claude-r1", ".claude-r1bad"]), Some(&k)), None, "읽지 못한 폴더가 낀 행이 키를 냈다");
            assert_eq!(current_profiles_for("claude", "Z", &ps(&[".claude-r1bad"]), Some(&k)), None);
            assert_eq!(current_profiles_for("claude", "A", &ps(&[".claude-r1out", ".claude-r1bad"]), Some(&k)), None, "NoLogin 이 섞였다고 읽지 못한 폴더를 가렸다");
            // 윈도우: 표는 `\`, profiles 는 `/` 로 섞여 와도 같은 폴더다 — 읽지 못함이면 키 부재 · 신원 없음 확정이면 `[]`
            let kw = known_pairs(&[(".cys\\claude-r1", "?")]);
            assert_eq!(current_profiles_for("claude", "A", &ps(&[".cys/claude-r1"]), Some(&kw)), None, "구분자가 다른 같은 폴더(읽지 못함)를 놓쳤다");
            let kn = known_pairs(&[(".cys\\claude-r1", "-")]);
            assert_eq!(current_profiles_for("claude", "A", &ps(&[".cys/claude-r1"]), Some(&kn)), Some(Vec::new()), "구분자가 다른 같은 폴더(신원 없음 확정)가 읽지 못함으로 읽혔다");
            // 폴더 표를 못 얻었다(홈 불명 · 열거 실패) → 모든 claude 행 키 부재 · 열거 성공·폴더 0개는 빈 배열
            assert_eq!(current_profiles_for("claude", "A", &ps(&[".cys/claude-r1"]), None), None);
            assert_eq!(current_profiles_for("claude", "A", &ps(&[".cys/claude-r1"]), Some(&[])), Some(Vec::new()));
            // 그 밖 provider 는 폴더 표와 무관하게 profiles 그대로
            assert_eq!(current_profiles_for("codex", "default", &ps(&[".codex"]), None), Some(s(&[".codex"])));
            assert_eq!(current_profiles_for("antigravity", "default", &ps(&[".antigravity"]), Some(&k)), Some(s(&[".antigravity"])));
            assert_eq!(current_profiles_for("grok", "default", &ps(&[]), None), Some(Vec::new()));
        }

        /// ★m-2 ⓑ(끝에서 끝까지 · `usage.accounts` 행): 신원 판독이 **연속** 실패한 폴더가 낀 행은 `current_profiles` 키를 내지 않는다(첫 실패 주기는 직전 신원을 한 주기 더 써 키가 그대로다) — 읽은 폴더만 가진 행은 키를
        /// 낸다. 홈 열거 자체가 실패하면 모든 claude 행이 키를 내지 않고 codex 행은 profiles 그대로다. 종전은 둘 다 `[]`(= 화면의 '이전 로그인')을 단정했다.
        #[test]
        fn r1f_local_json_omits_current_profiles_for_an_unreadable_folder_and_for_every_claude_row_when_the_listing_fails() {
            let dir = tmp("r1k-daemon");
            let home = tmp("r1k-home");
            let _h = test_home::set(&home);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let fa = b3_login(&home, ".cys/claude-r1", "u-r1-a", "a-r1@example.test", 1);
            b3_login(&home, ".claude-r1x", "u-r1-b", "b-r1@example.test", 1);
            std::fs::create_dir_all(home.join(".codex")).unwrap();
            {
                let mut st = d.accounts.lock().unwrap();
                seed_discovered(&mut st, &home); // 부트 발견 — 행 A·B·codex 가 생기고 profiles 에 폴더가 실린다
            }
            let t0 = crate::state::now_epoch();
            let rows = local_json(&d, t0);
            assert_eq!(strs(&row(&rows, "u-r1-a")["current_profiles"]), s(&[".cys/claude-r1"]), "전제: 정상일 때 A 는 자기 폴더를 현재로 갖는다");
            // 폴더 A 의 신원 파일이 읽히지 않는다(잘린 JSON) — 첫 실패 주기: 직전 신원을 한 주기 더 쓴다 → '이전 로그인'을 단정하지 않는다
            put_body(&fa, "{\"oauthAccount\"", 2);
            let rows = local_json(&d, t0 + 61.0);
            assert_eq!(strs(&row(&rows, "u-r1-a")["current_profiles"]), s(&[".cys/claude-r1"]), "일시 실패 한 번에 '이전 로그인'(빈 배열)을 단정했다");
            // 연속 실패 → 키 부재(빈 배열이 아니다) · 읽은 폴더만 가진 행 B 는 키를 낸다 · codex 는 profiles 그대로
            let rows = local_json(&d, t0 + 122.0);
            let a = row(&rows, "u-r1-a");
            assert!(a.get("current_profiles").is_none(), "신원을 읽지 못한 폴더가 낀 행이 current_profiles 를 냈다: {a}");
            assert_eq!(strs(&a["profiles"]), s(&[".cys/claude-r1"]), "profiles(추가 전용 폴백)는 그대로여야 한다");
            assert_eq!(strs(&row(&rows, "u-r1-b")["current_profiles"]), s(&[".claude-r1x"]), "읽은 폴더만 가진 행은 키를 낸다");
            assert_eq!(strs(&row(&rows, "default")["current_profiles"]), s(&[".codex"]));
            // 홈 열거 자체가 실패(읽을 수 없는 홈) → 모든 claude 행 키 부재 · codex 는 그대로
            let gone = home.join("not-there");
            let _h2 = test_home::set(&gone);
            let rows = local_json(&d, t0 + 500.0);
            for r in rows.as_array().unwrap().iter().filter(|r| r["provider"] == "claude") {
                assert!(r.get("current_profiles").is_none(), "열거 실패인데 claude 행이 current_profiles 를 냈다: {r}");
            }
            assert_eq!(strs(&row(&rows, "default")["current_profiles"]), s(&[".codex"]), "codex 행은 열거와 무관하다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        // ───────────────────────── [보정 · m-2 ⓑ] 신원 없음 확정(가)과 이번에 읽지 못함(나)을 가른다 ─────────────────────────

        /// ★보정 (가): **신원 없음이 확정**인 폴더 — 로그아웃(파일은 읽히고 JSON 인데 `oauthAccount` 없음)·신원 파일이 아예 없음 — 는 '읽지 못함'이 아니라 '이 계정의 폴더가 아니다'다 → 그 계정에 다른 폴더가 없으면
        /// `current_profiles` 는 키 부재가 아니라 `[]`(= 화면의 '이전 로그인' · 9650334f 와 같다). 다른 계정은 영향이 없다 · 로그아웃이 계속돼도 `[]` 그대로(연장·키 부재로 바뀌지 않는다).
        #[test]
        fn r1f_a_logged_out_folder_or_one_without_an_identity_file_gives_an_empty_current_profiles_not_an_absent_key() {
            let dir = tmp("r1m-daemon");
            let home = tmp("r1m-home");
            let _h = test_home::set(&home);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let fa = b3_login(&home, ".cys/claude-r1", "u-r1-a", "a-r1@example.test", 1); // 곧 로그아웃
            let fb = b3_login(&home, ".claude-r1x", "u-r1-b", "b-r1@example.test", 1); // 곧 신원 파일이 사라진다(폴더는 남는다 — 이름 규칙에 맞아 열거된다)
            b3_login(&home, ".claude-r1y", "u-r1-c", "c-r1@example.test", 1); // 그대로
            {
                let mut st = d.accounts.lock().unwrap();
                seed_discovered(&mut st, &home); // 부트 발견 — 행 A·B·C 가 생기고 profiles 에 폴더가 실린다
            }
            let t0 = crate::state::now_epoch();
            // ★(R2F-DM · ⓑ) 윈도우의 열거 경로는 `\` 라 `current_profiles` 도 `.cys\claude-r1` 로 나온다 — `strs` 가 구분자를 `/` 로 접는다(화면의 `normalizeProfile` 과 같은 규칙 · 이 파일의 다른 검체가 이미 쓰는 도우미).
            //   제품 출력은 바꾸지 않는다 · 맥·리눅스는 `/` 뿐이라 값이 그대로다.
            let cp = |rows: &Value, id: &str| row(rows, id).get("current_profiles").map(|v| json!(strs(v)));
            let rows = local_json(&d, t0);
            assert_eq!(cp(&rows, "u-r1-a"), Some(json!([".cys/claude-r1"])), "전제: 정상일 때 A 는 자기 폴더를 현재로 갖는다");
            put_body(&fa, r#"{"projects":{}}"#, 2); // 로그아웃: 파일은 있고 JSON 인데 oauthAccount 가 없다
            std::fs::remove_file(fb.join(".claude.json")).unwrap(); // 신원 파일이 아예 없다
            for dt in [61.0, 122.0] {
                let rows = local_json(&d, t0 + dt);
                assert_eq!(cp(&rows, "u-r1-a"), Some(json!([])), "+{dt}초: 로그아웃(신원 없음 확정)한 폴더만 가진 계정이 `[]` 가 아니다 — 읽지 못함으로 섞어 키를 뺐다(또는 직전 신원을 연장했다): {}", row(&rows, "u-r1-a"));
                assert_eq!(cp(&rows, "u-r1-b"), Some(json!([])), "+{dt}초: 신원 파일이 없는 폴더만 가진 계정이 `[]` 가 아니다: {}", row(&rows, "u-r1-b"));
                assert_eq!(cp(&rows, "u-r1-c"), Some(json!([".claude-r1y"])), "+{dt}초: 다른 계정은 영향이 없다");
            }
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// ★보정 (나)-선점: **처음 보는 폴더를 다른 요청이 아직 읽는 중**이면(선점 자리) 그 폴더는 '신원 없음 확정'이 아니라 '아직 모름'(`Unread`)이다 — 그 폴더가 `profiles` 에 든 행은 키가 나오지 않는다.
        /// 읽기-통과를 accounts 락 위에서 세워 놓고(첫 요청 = 선점 후 멈춤) 다른 요청이 같은 폴더를 물으면 `Unread` 를 받는다(`NoLogin` 이 아니다 · 막히지 않는다). 알려진 폴더 표·`current_profiles_for` 도 같다 — 판독이 끝나면 실제
        /// 상태(`Known`)가 된다. 끝에서 끝(`local_json`)은 같은 모양의 선점 자리를 직접 실어 본다(accounts 락을 쥔 채는 `local_json` 이 행을 만들 수 없다). 대조: 자리가 사라진 뒤(읽은 뒤)에는 같은 행이 키를 낸다.
        #[test]
        fn r1f_a_folder_another_request_is_still_reading_for_the_first_time_is_unread_not_no_login() {
            let dir = tmp("r1o-daemon");
            let dir2 = tmp("r1o-daemon2");
            let home = tmp("r1o-home");
            let _h = test_home::set(&home);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let f = b3_login(&home, ".cys/claude-r1", "u-r1-a", "a-r1@example.test", 1);
            let key = f.to_string_lossy().into_owned();
            let t0 = 1_800_000_000.0;
            let guard = d.accounts.lock().unwrap(); // '멈춘 IO' — 처음 보는 폴더의 첫 읽기가 이 락에서 멈춘다
            let (tx1, rx1) = std::sync::mpsc::channel();
            {
                let (d1, home1, key1) = (d.clone(), home.clone(), key.clone());
                std::thread::spawn(move || {
                    let _ = tx1.send(folder_identity_state(&d1, Some(&home1), &key1, t0));
                });
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let marked = loop {
                if d.seat_ident_cache.lock().unwrap().entries.contains_key(Path::new(&key)) {
                    break true;
                }
                if std::time::Instant::now() > deadline {
                    break false;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            // 선점 자리 — 다른 요청의 눈에는 '아직 모름'. 막히지 않는다(accounts 락이 필요 없다).
            let peeked = marked.then(|| peek_folder_state(&d, &key));
            let second = marked.then(|| {
                let (tx2, rx2) = std::sync::mpsc::channel();
                let (d2, home2, key2) = (d.clone(), home.clone(), key.clone());
                std::thread::spawn(move || {
                    let _ = tx2.send(folder_identity_state(&d2, Some(&home2), &key2, t0 + 1.0));
                });
                rx2.recv_timeout(std::time::Duration::from_secs(3))
            });
            let table = marked.then(|| known_profile_identities(&d, Some(&home), t0 + 2.0));
            drop(guard); // '멈춘 IO' 해제 — 첫 요청이 끝난다
            let first = rx1.recv_timeout(std::time::Duration::from_secs(5));
            assert!(marked, "읽기-통과가 IO 전에 선점 자리를 찍지 않았다");
            assert_eq!(peeked, Some(FolderWho::Unread), "처음 보는 폴더의 선점 자리가 '아직 모름'(Unread)이 아니다(신원 없음 확정으로 읽혔다)");
            assert_eq!(second.expect("marked").expect("두 번째 요청이 첫 요청의 멈춘 IO 에 묶였다"), FolderWho::Unread, "선점 중 다른 요청이 신원 없음 확정(NoLogin)을 받았다");
            let table = table.expect("marked").expect("열거 성공");
            let table_norm: Vec<(String, FolderWho)> = table.iter().map(|(p, w)| (p.replace('\\', "/"), w.clone())).collect(); // 윈도우 열거 경로의 `\` 를 접는다
            assert_eq!(table_norm, known_pairs(&[(".cys/claude-r1", "?")]), "알려진 폴더 표가 선점 자리를 '아직 모름'으로 싣지 않았다");
            let profiles: BTreeSet<String> = [".cys/claude-r1".to_string()].into_iter().collect();
            assert_eq!(current_profiles_for("claude", "u-r1-a", &profiles, Some(&table)), None, "읽는 중인 폴더가 낀 행이 키를 냈다");
            assert_eq!(first.expect("첫 요청이 끝나지 않았다"), FolderWho::Known("u-r1-a".to_string()), "판독이 끝나면 실제 상태가 된다");
            assert_eq!(peek_folder_state(&d, &key), FolderWho::Known("u-r1-a".to_string()));
            // 끝에서 끝: 같은 모양의 선점 자리(처음 보는 폴더 · Unread · 읽는 중)를 직접 실은 데몬 — 행 A 가 키를 내지 않는다
            let d2 = crate::state::Daemon::new(dir2.join("cysd.sock"));
            {
                let mut st = d2.accounts.lock().unwrap();
                seed_discovered(&mut st, &home);
            }
            let now = t0 + 100.0;
            d2.seat_ident_cache.lock().unwrap().entries.insert(PathBuf::from(&key), IdentSlot { at: now, state: FolderWho::Unread, misses: 0 });
            let rows = local_json(&d2, now + 1.0);
            assert!(row(&rows, "u-r1-a").get("current_profiles").is_none(), "선점 자리(읽는 중) 위에서 행이 current_profiles 를 냈다: {}", row(&rows, "u-r1-a"));
            let rows = local_json(&d2, now + 62.0); // 60초 하한이 지나 읽기-통과가 실제 신원을 읽는다
            // ★(R2F-DM · ⓑ) 윈도우의 열거 경로(`\`)를 `/` 로 접어 비교한다(`strs`) — 위 `table_norm` 과 같은 이유다.
            assert_eq!(row(&rows, "u-r1-a").get("current_profiles").map(|v| json!(strs(v))), Some(json!([".cys/claude-r1"])), "자리가 사라진(읽은) 뒤에도 키가 안 나온다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&dir2);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// ★보정: 좌석 폴더(열거 밖 포함)의 신원 표 None 도 같은 두 뜻이다 — `seat_folder_states` 가 캐시 항목으로 가른다(추가 IO 0): 신원 없음 확정 = `NoLogin` · 읽지 못함·항목 없음(아직 모름) = `Unread` · 신원을 읽은 폴더 = `Known`.
        #[test]
        fn r1f_seat_folder_states_split_a_none_identity_into_no_login_and_unread_from_the_cache() {
            let (d, dir, home, f1) = b3_fixture("r1n");
            let f2 = b3_login(&home, ".cys/claude-b3-2", "u-b3-b", "b-b3@example.test", 2); // 곧 로그아웃
            let f3 = b3_login(&home, ".cys/claude-b3-3", "u-b3-c", "c-b3@example.test", 3); // 곧 깨진다
            let _s1 = b3_seat(&d, "worker-r1a", "claude", Some(&f1));
            let _s2 = b3_seat(&d, "worker-r1b", "claude", Some(&f2));
            let _s3 = b3_seat(&d, "worker-r1c", "claude", Some(&f3));
            put_body(&f2, r#"{"projects":{}}"#, 4); // 로그아웃 — 신원 없음 확정
            put_body(&f3, "{\"oauthAccount\"", 5); // 잘린 JSON — 처음 읽기라 직전 신원이 없다 → 읽지 못함
            let t0 = crate::state::now_epoch();
            let view = seat_identity_view_in(&d, Some(&home), t0);
            let st = seat_folder_states(&d, &view);
            let fs = |p: &Path| p.to_string_lossy().into_owned();
            assert_eq!(
                st,
                vec![(fs(&f1), FolderWho::Known("u-b3-a".to_string())), (fs(&f2), FolderWho::NoLogin), (fs(&f3), FolderWho::Unread)],
                "신원 표의 None 이 신원 없음 확정(NoLogin)과 읽지 못함(Unread)으로 갈리지 않았다"
            );
            // 캐시 항목이 사라지면(아직 모름) None 인 폴더는 Unread — 신원 없음 확정으로 읽지 않는다
            d.seat_ident_cache.lock().unwrap().clear();
            let st = seat_folder_states(&d, &view);
            assert_eq!(st[1].1, FolderWho::Unread, "캐시 항목이 없는 폴더를 신원 없음 확정으로 읽었다");
            assert_eq!(st[0].1, FolderWho::Known("u-b3-a".to_string()), "신원을 읽은 폴더는 캐시와 무관하다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// ★보정(좌석 폴더): 열거 규칙 밖 좌석 폴더(`CYS_ACCOUNT_DIR=<임의>`)의 계정이 ① 로그아웃(신원 없음 확정) → `current_profiles: []` ② 다시 로그인 → 그 폴더 ③ 판독 첫 실패 → 직전 신원 유예 → 그 폴더 ④ 연속 실패(읽지 못함)
        /// → 키 부재. 좌석 폴더의 합류(m-1)도 같은 구분을 쓴다.
        #[test]
        fn r1f_an_out_of_rules_seat_folder_follows_the_same_split_logged_out_is_empty_and_unreadable_is_absent() {
            let dir = tmp("r1q-daemon");
            let home = tmp("r1q-home");
            let _h = test_home::set(&home);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            b3_login(&home, ".claude-r1x", "u-r1-x", "x-r1@example.test", 1); // 이름 규칙 안의 다른 계정(열거는 비어 있지 않다)
            let work = b3_login(&home, "work/acct", "u-r1-w", "w-r1@example.test", 2); // 이름 규칙 밖
            let _seat = b3_seat(&d, "worker-r1", "claude", Some(&work));
            let t0 = crate::state::now_epoch();
            assert!(note_rate_for_profile_at(&d, Some(&home), &work, &[rw("5h", 40.0, Some(t0 + 7200.0))], "statusline", t0));
            let cp = |t: f64| row(&local_json(&d, t), "u-r1-w").get("current_profiles").cloned();
            assert_eq!(cp(t0 + 1.0), Some(json!(["work/acct"])), "전제: 좌석이 쓰는 계정은 자기 폴더를 현재로 갖는다");
            put_body(&work, r#"{"projects":{}}"#, 3); // ① 로그아웃
            assert_eq!(cp(t0 + 61.0), Some(json!([])), "좌석 폴더가 로그아웃(신원 없음 확정)인데 키가 빠졌거나 폴더가 남았다");
            put_body(&work, ID_W, 4); // ② 다시 로그인
            assert_eq!(cp(t0 + 122.0), Some(json!(["work/acct"])));
            put_body(&work, "{\"oauthAccount\"", 5); // ③ 읽기 실패 첫 주기 — 직전 신원을 한 주기 더
            assert_eq!(cp(t0 + 183.0), Some(json!(["work/acct"])), "일시 실패 한 번에 이전 로그인(빈 배열)·키 부재를 단정했다");
            assert_eq!(cp(t0 + 244.0), None, "④ 유예 뒤에도 못 읽는 좌석 폴더가 낀 행이 키를 냈다(읽지 못함)");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        // ───────────────────────── [m-1] 이름 규칙 밖 좌석 폴더 ─────────────────────────

        /// ★m-1 ⓐ: 열거 규칙 밖 좌석 폴더(`CYS_ACCOUNT_DIR=<임의>` · 부서 카탈로그의 임의 계정 폴더)를 쓰는 계정은 `in_use:true` + `current_profiles` 에 그 폴더가 든다(종전: `[]` → 화면 `● 사용 중` + `이전 로그인`).
        /// 좌석이 떠나면 그 폴더는 알려진 폴더가 아니므로 `[]`(in_use:false 와 일관 — 이전 로그인이 맞다).
        #[test]
        fn r1f_a_seat_folder_outside_the_naming_rules_still_shows_its_account_as_current() {
            let dir = tmp("r1l-daemon");
            let home = tmp("r1l-home");
            let _h = test_home::set(&home);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            b3_login(&home, ".claude-r1x", "u-r1-x", "x-r1@example.test", 1); // 이름 규칙 안의 다른 계정(열거는 비어 있지 않다)
            let work = b3_login(&home, "work/acct", "u-r1-w", "w-r1@example.test", 2); // 이름 규칙 밖(CYS_ACCOUNT_DIR=<임의>)
            let seat = b3_seat(&d, "worker-r1", "claude", Some(&work));
            let t0 = crate::state::now_epoch();
            assert!(note_rate_for_profile_at(&d, Some(&home), &work, &[rw("5h", 40.0, Some(t0 + 7200.0))], "statusline", t0));
            let rows = local_json(&d, t0 + 1.0);
            let w = row(&rows, "u-r1-w");
            assert_eq!(w["in_use"], json!(true), "전제: 좌석이 쓰는 계정은 in_use:true");
            assert_eq!(strs(&w["profiles"]), s(&["work/acct"]));
            assert_eq!(strs(&w["current_profiles"]), s(&["work/acct"]), "열거 밖 좌석 폴더의 계정이 in_use:true 인데 current_profiles 가 비었다('● 사용 중' + '이전 로그인')");
            // 좌석이 떠나면 — 열거 밖 폴더는 알려진 폴더가 아니다(in_use:false 와 일관 · 이전 로그인)
            seat.exited.store(true, Ordering::Relaxed);
            let rows = local_json(&d, t0 + 2.0);
            let w = row(&rows, "u-r1-w");
            assert_eq!(w["in_use"], json!(false));
            assert_eq!(strs(&w["current_profiles"]), Vec::<String>::new(), "좌석이 떠났는데 열거 밖 폴더가 현재로 남았다");
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 순수: 좌석 폴더(3값 상태)를 알려진 폴더 표에 한 번만 합친다 — `profiles` 와 같은 표기(`profile_short`)로 · 같은 표기(구분자 `\` 와 `/` 는 같은 것)가 이미 있으면 그대로(열거된 폴더 우선) · 상태(`Known`·`NoLogin`·`Unread`)는 그대로 싣는다.
        #[test]
        fn r1f_merge_seat_folders_adds_a_seat_folder_once_in_the_profiles_notation() {
            let home = Path::new("/h");
            let seat: Vec<(String, FolderWho)> = vec![
                ("/h/work/acct".to_string(), FolderWho::Known("W".to_string())),
                ("/h/.cys/claude-a".to_string(), FolderWho::Known("A".to_string())),
                ("/elsewhere/acct".to_string(), FolderWho::Unread),
                ("/h/work/out".to_string(), FolderWho::NoLogin),
            ];
            let k = known_pairs(&[(".cys/claude-a", "A"), (".claude-b", "B")]);
            let merged = merge_seat_folders(k, Some(home), &seat);
            assert_eq!(
                merged,
                known_pairs(&[(".cys/claude-a", "A"), (".claude-b", "B"), ("work/acct", "W"), ("/elsewhere/acct", "?"), ("work/out", "-")]),
                "좌석 폴더는 profile_short 표기로 한 번만 합쳐지고 상태가 그대로다(열거된 폴더는 그대로)"
            );
            // 구분자만 다른 같은 표기는 합치지 않는다
            let kw = known_pairs(&[("work\\acct", "W")]);
            assert_eq!(merge_seat_folders(kw.clone(), Some(home), &seat).iter().filter(|(p, _)| p.replace('\\', "/") == "work/acct").count(), 1);
            // 좌석 폴더가 없으면(수집 실패 · 좌석 0) 아무것도 더하지 않는다
            assert_eq!(merge_seat_folders(kw.clone(), Some(home), &[]), kw);
        }

        // ───────────────────────── [m-6] 같은 경보 키를 여러 좌석·계정이 나눠 쓸 때 ─────────────────────────

        /// ★m-6(a): 같은 경보 키를 쓰는 입력이 둘 이상이면 스냅샷이 **값이 가장 큰 입력 하나**를 쥐고 그 입력의 메타(`age=`·`in_use=`·`reset=`)를 싣는다 — 3시간 전 99% 가 5초 전 91% 의 나이(`age=5`)로
        /// 나가지 않는다. 좌석 축(같은 역할명 좌석 둘 — 생성 순서를 바꿔 두 번)·계정 축(같은 이메일의 두 계정) 모두. 발행되는 경보(틱)의 detail 도 같은 입력의 것이다.
        #[test]
        fn r1f_a_shared_alert_key_publishes_the_value_and_the_meta_of_the_same_input() {
            for swap in [false, true] {
                let (d, dir, home, f1) = b3_fixture(&format!("r1h{}", u8::from(swap)));
                let f2 = b3_login(&home, ".cys/claude-b3-2", "u-b3-b", "b-b3@example.test", 2);
                let seat_a = b3_seat(&d, "worker-r1", "claude", Some(&f1));
                let seat_b = b3_seat(&d, "worker-r1", "claude", Some(&f2));
                let t0 = crate::state::now_epoch();
                let _ = seat_identity_view_in(&d, Some(&home), t0); // 두 폴더의 현재 신원(A·B)을 캐시에 싣는다
                let ((hi, hi_acct), (lo, lo_acct)) = if swap { ((&seat_b, "u-b3-b"), (&seat_a, "u-b3-a")) } else { ((&seat_a, "u-b3-a"), (&seat_b, "u-b3-b")) };
                put_usage(hi, 99.0, t0 + 7200.0, t0 - 10_800.0, Some(hi_acct)); // 3시간 전 99% — 사용 중(귀속 == 현재 신원)이라 적격
                put_usage(lo, 91.0, t0 + 7200.0, t0 - 5.0, Some(lo_acct)); // 5초 전 91%
                let key = ("worker-r1".to_string(), "5h".to_string());
                let snap = crate::alerts::snapshot_with_stale(&d, t0, 1800.0);
                assert_eq!(snap.rates, vec![("worker-r1".to_string(), "5h".to_string(), 99.0)], "swap={swap}: 같은 키의 입력이 하나로 접히지 않았거나 값이 가장 큰 입력이 아니다: {:?}", snap.rates);
                let m = &snap.rate_meta[&key];
                assert_eq!((m.age_secs as u64, m.in_use, m.observed_at), (10_800, Some(true), t0 - 10_800.0), "swap={swap}: 메타가 값을 낸 입력(99%)의 것이 아니다: {m:?}");
                // 틱이 실제로 내는 경보 — 값과 detail 이 같은 입력의 것
                let seq0 = d.bus.latest_seq();
                let mut fired: HashMap<String, f64> = HashMap::new();
                crate::governance::check_alerts_with(&d, &mut fired, &crate::alerts::AlertConfig::default(), t0);
                let ev: Vec<Value> = b3_events(&d, seq0, "alert.rate_limit").into_iter().filter(|p| p["key"] == "rate_limit:worker-r1:5h").collect();
                assert_eq!(ev.len(), 1, "swap={swap}");
                assert_eq!((ev[0]["detail"]["used_pct"].clone(), ev[0]["detail"]["age_secs"].clone()), (json!(99.0), json!(10_800)), "swap={swap}: 발행된 경보의 값과 age 가 다른 입력의 것이다: {}", ev[0]);
                let _ = std::fs::remove_dir_all(&dir);
                let _ = std::fs::remove_dir_all(&home);
            }
            // ── 계정 축: 같은 이메일(=같은 경보 키)의 두 계정 ──
            let (d, dir, home, _f) = b3_fixture("r1h2");
            let d1 = b3_login(&home, ".cys/claude-dup1", "u-r1-d1", "dup-r1@example.test", 5);
            let d2 = b3_login(&home, ".cys/claude-dup2", "u-r1-d2", "dup-r1@example.test", 6);
            let _s1 = b3_seat(&d, "worker-r1d1", "claude", Some(&d1));
            let _s2 = b3_seat(&d, "worker-r1d2", "claude", Some(&d2));
            let t0 = crate::state::now_epoch();
            let _ = seat_identity_view_in(&d, Some(&home), t0);
            assert!(note_rate_for_profile_at(&d, Some(&home), &d1, &[rw("5h", 99.0, Some(t0 + 7200.0))], "statusline", t0 - 10_800.0));
            assert!(note_rate_for_profile_at(&d, Some(&home), &d2, &[rw("5h", 91.0, Some(t0 + 7200.0))], "statusline", t0 - 5.0));
            let snap = crate::alerts::snapshot_with_stale(&d, t0, 1800.0);
            assert_eq!(snap.account_rates, vec![("dup-r1@example.test".to_string(), "5h".to_string(), 99.0)], "같은 이메일 두 계정의 입력이 값이 가장 큰 하나로 접히지 않았다: {:?}", snap.account_rates);
            let m = &snap.account_meta[&("dup-r1@example.test".to_string(), "5h".to_string())];
            assert_eq!((m.age_secs as u64, m.in_use, m.observed_at), (10_800, Some(true), t0 - 10_800.0), "계정 축 메타가 값을 낸 입력(99%)의 것이 아니다: {m:?}");
            let seq0 = d.bus.latest_seq();
            let mut fired: HashMap<String, f64> = HashMap::new();
            crate::governance::check_alerts_with(&d, &mut fired, &crate::alerts::AlertConfig::default(), t0);
            let ev: Vec<Value> = b3_events(&d, seq0, "alert.account_rate").into_iter().filter(|p| p["key"] == "account_rate:dup-r1@example.test:5h").collect();
            assert_eq!(ev.len(), 1);
            assert_eq!((ev[0]["detail"]["used_pct"].clone(), ev[0]["detail"]["age_secs"].clone()), (json!(99.0), json!(10_800)), "발행된 계정 경보의 값과 age 가 다른 입력의 것이다: {}", ev[0]);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// 틱 한 번의 시나리오 도우미(m-6 b): 같은 키(`rate_limit:worker-r1:5h`)를 쓰는 좌석 셋 — s1 은 사용 중·신선한 99%(발화), s2 는 로그인이 바뀐 좌석의 **낡은** 창(`stale_pct`), s3 은 나중에 같은 키로 새로 교차하는 좌석.
        /// s1 이 떠난 뒤(+31초 틱) 키가 붙들렸는지(`fired`)·해소 알림 사유·s3 의 새 교차(+61초 틱)가 발행됐는지를 돌려준다.
        /// (`tag` 는 검체마다 달라야 한다 — 임시 폴더 이름이 같으면 병렬 검체끼리 서로의 폴더를 지운다.)
        fn hold_scenario(tag: &str, stale_pct: f64) -> (bool, Vec<(String, String)>, usize) {
            let (d, dir, home, f1) = b3_fixture(tag);
            let f2 = b3_login(&home, ".cys/claude-b3-2", "u-b3-b", "b-b3@example.test", 2);
            let f3 = b3_login(&home, ".cys/claude-b3-3", "u-b3-c", "c-b3@example.test", 3);
            let s1 = b3_seat(&d, "worker-r1", "claude", Some(&f1));
            let s2 = b3_seat(&d, "worker-r1", "claude", Some(&f2));
            let s3 = b3_seat(&d, "worker-r1", "claude", Some(&f3)); // 관측이 생기기 전에는 입력이 없다
            let t0 = crate::state::now_epoch();
            let _ = seat_identity_view_in(&d, Some(&home), t0); // 세 폴더의 현재 신원을 캐시에 싣는다(워치독은 읽지 않는다)
            let key = "rate_limit:worker-r1:5h";
            let cfg = crate::alerts::AlertConfig::default();
            let mut fired: HashMap<String, f64> = HashMap::new();
            let seq0 = d.bus.latest_seq();
            put_usage(&s1, 99.0, t0 + 20_000.0, t0, Some("u-b3-a")); // 사용 중 · 신선 → 적격 → 발화
            put_usage(&s2, stale_pct, t0 + 20_000.0, t0 - 5_000.0, Some("u-r1-old")); // 귀속 계정 ≠ 현재 신원(로그인이 바뀐 좌석) · 관측 5000초 → 부적격(붙들림 후보)
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 1.0);
            assert_eq!(b3_events(&d, seq0, "alert.rate_limit").iter().filter(|p| p["key"] == key).count(), 1, "전제: s1 99% 가 발화했다");
            s1.exited.store(true, Ordering::Relaxed); // 진짜 경보의 주인이 사라진다
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 31.0);
            let held = fired.contains_key(key);
            let resolved = b3_resolved(&d, seq0);
            put_usage(&s3, 95.0, t0 + 20_000.0, t0 + 50.0, Some("u-b3-c")); // 다른 좌석의 새 교차(같은 키)
            crate::governance::check_alerts_with(&d, &mut fired, &cfg, t0 + 61.0);
            let published = b3_events(&d, seq0, "alert.rate_limit").iter().filter(|p| p["key"] == key).count();
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
            (held, resolved, published)
        }

        /// ★m-6(b): 임계 **미만**의 낡은 창은 같은 키를 쓰는 다른 좌석의 발화 키를 붙들지 않는다 — 주인(s1)이 떠나면 키가 풀리고(해소 사유 `cleared`) 다른 좌석(s3)의 새 교차가 **즉시** 발행된다(0.14.42 와 같다).
        /// 종전: 낡은 10% 창이 키를 붙들어(`stale`) s3 의 새 교차가 마지막 발행에서 30분이 될 때까지 늦게 나갔다.
        #[test]
        fn r1f_a_below_threshold_stale_window_does_not_hold_another_seats_fired_key() {
            let (held, resolved, published) = hold_scenario("r1i-lo", 10.0);
            assert!(!held, "임계 미만의 낡은 창이 남의 발화 키를 붙들었다(키가 fired 에 남았다)");
            assert_eq!(resolved, vec![("rate_limit:worker-r1:5h".to_string(), "cleared".to_string())], "붙들림이 아니라 해소(cleared)여야 한다");
            assert_eq!(published, 2, "다른 좌석의 새 교차가 즉시 발행되지 않았다(30분 늦게 나간다)");
        }

        /// 음성 대조(m-6 b): 임계 **이상**의 낡은 창(경보였을 값이 신선도 규칙으로 빠진 것)은 여전히 키를 붙든다 — 주인이 떠나도 키가 `fired` 에 남고(해소 사유 `stale`) REMIND 안에는 다른 좌석의 교차가 다시 발행되지 않는다
        /// (B3 의 깜빡임 금지는 그대로 — 붙들림이 죽지 않았다).
        #[test]
        fn r1f_an_above_threshold_stale_window_still_holds_the_key() {
            let (held, resolved, published) = hold_scenario("r1i-hi", 95.0);
            assert!(held, "임계 이상의 낡은 창이 키를 붙들지 못했다(깜빡임 금지가 죽었다)");
            assert_eq!(resolved, vec![("rate_limit:worker-r1:5h".to_string(), "stale".to_string())]);
            assert_eq!(published, 1, "붙들린 키가 REMIND 안에 다시 발행됐다");
        }

        // ───────────────────────── [n-1 · n-2] 별명 ─────────────────────────

        /// ★n-1: `account_id == "default"` 인 행(Codex·Antigravity·선언 계정)은 `label` 키를 **먼저** 본다 — `{"default":"X","OpenAI Codex":"코덱스"}` 에서 코덱스가 `X` 가 되지 않는다(라벨 키가 없는 행만 `default` 키로 내려간다).
        /// 그 밖 행(claude)은 종전 순서(account_id → label)다.
        #[test]
        fn r1f_alias_for_default_rows_looks_at_the_label_key_first() {
            let t: HashMap<String, String> = [("default", "X"), ("OpenAI Codex", "코덱스"), ("u-r1-a", "ID별명"), ("a-r1@example.test", "메일별명")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert_eq!(alias_for(&t, "default", "OpenAI Codex"), Some("코덱스"), "default 행인데 default 키가 라벨 키를 가렸다");
            assert_eq!(alias_for(&t, "default", "Antigravity (agy)"), Some("X"), "라벨 키가 없는 default 행은 default 키로 내려간다");
            assert_eq!(alias_for(&t, "u-r1-a", "a-r1@example.test"), Some("ID별명"), "claude 행은 종전 순서(account_id 먼저)");
            assert_eq!(alias_for(&t, "u-r1-zzz", "a-r1@example.test"), Some("메일별명"));
            let only_default: HashMap<String, String> = [("default".to_string(), "X".to_string())].into_iter().collect();
            assert_eq!(alias_for(&only_default, "u-r1-a", "a-r1@example.test"), None, "default 키는 claude 행에 붙지 않는다");
            assert_eq!(alias_for(&HashMap::new(), "default", "OpenAI Codex"), None);
        }

        /// ★n-2: 별명 정제가 제어 문자에 더해 양방향 제어(U+202A~202E · U+2066~2069)·제로폭(U+200B~200F · U+FEFF)·줄/문단 구분자(U+2028·2029)도 걷는다 — 화면 `starvednotice.ts` 의 `INVISIBLE` 과 같은 집합.
        /// 경계 바로 밖의 글자(U+200A · U+2010 · U+202F · U+2065 · U+206A)와 일반 한글·이모지는 남는다.
        #[test]
        fn r1f_sanitize_alias_also_strips_bidi_controls_and_zero_width_characters() {
            let stripped = [
                '\u{200B}', '\u{200C}', '\u{200D}', '\u{200E}', '\u{200F}', '\u{2028}', '\u{2029}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{FEFF}',
            ];
            for c in stripped {
                assert_eq!(sanitize_alias(&format!("업무{c}용")).as_deref(), Some("업무용"), "U+{:04X} 가 걷히지 않았다", c as u32);
                assert_eq!(sanitize_alias(&format!("{c}{c}")), None, "U+{:04X} 만으로 된 별명은 비어야 한다", c as u32);
            }
            assert_eq!(sanitize_alias("\u{202E}evil\u{202C}").as_deref(), Some("evil"), "RLO 로 표시를 뒤집는 별명");
            for c in ['\u{200A}', '\u{2010}', '\u{202F}', '\u{2065}', '\u{206A}', '\u{00A0}'] {
                let t = format!("a{c}b");
                assert_eq!(sanitize_alias(&t).as_deref(), Some(t.as_str()), "경계 밖 글자 U+{:04X} 까지 걷었다", c as u32);
            }
            assert_eq!(sanitize_alias("개인 😀").as_deref(), Some("개인 😀"));
            // 표 파서를 거쳐도 같다(별명 파일 → 표)
            let t = parse_alias_table("{\"aliases\":{\"k\":\"\\u202e업무\\u200b용\"}}".as_bytes());
            assert_eq!(t.get("k").map(String::as_str), Some("업무용"));
        }
    }

    /// ★0.14.45 좌석별 계정(`account`) — 사이드바 부서 카드·사용량 패널 역매핑의 재료.
    mod seat_account_0145 {
        use super::b3_scenarios::{b3_login, b3_seat};
        use super::*;

        /// 순수 표 — claude 의 네 상태(known · folder_unknown · no_login · unread) · codex/agy 는 기기 로그인 하나(`default`) · 모르는 에이전트 · 셸(null).
        /// 모르는 것은 `account_id: null` 이다 — 폴더 이름으로 계정을 짐작하지 않는다(결측은 값이 아니다).
        #[test]
        fn seat_account_json_table() {
            let home = Path::new("/h");
            let known = FolderWho::Known("u-1".into());
            let cases: Vec<(&str, Option<&str>, Option<&str>, Option<&FolderWho>, Value)> = vec![
                ("셸", None, None, None, Value::Null),
                (
                    "claude known",
                    Some("claude"),
                    Some("/h/.claude-4"),
                    Some(&known),
                    json!({"provider":"claude","agent":"claude","account_id":"u-1","profile":".claude-4","state":"known"}),
                ),
                (
                    "claude 폴더 미상",
                    Some("claude"),
                    None,
                    None,
                    json!({"provider":"claude","agent":"claude","account_id":null,"profile":null,"state":"folder_unknown"}),
                ),
                (
                    "claude 로그아웃",
                    Some("claude"),
                    Some("/h/.cys/claude"),
                    Some(&FolderWho::NoLogin),
                    json!({"provider":"claude","agent":"claude","account_id":null,"profile":".cys/claude","state":"no_login"}),
                ),
                (
                    "claude 판독 실패",
                    Some("claude"),
                    Some("/h/.claude-2"),
                    Some(&FolderWho::Unread),
                    json!({"provider":"claude","agent":"claude","account_id":null,"profile":".claude-2","state":"unread"}),
                ),
                (
                    "claude 캐시 항목 없음",
                    Some("claude"),
                    Some("/elsewhere/acct"),
                    None,
                    json!({"provider":"claude","agent":"claude","account_id":null,"profile":"/elsewhere/acct","state":"unread"}),
                ),
                (
                    "codex",
                    Some("codex"),
                    None,
                    None,
                    json!({"provider":"codex","agent":"codex","account_id":"default","profile":null,"state":"known"}),
                ),
                (
                    "agy(gemini 키)",
                    Some("gemini"),
                    None,
                    None,
                    json!({"provider":"antigravity","agent":"gemini","account_id":"default","profile":null,"state":"known"}),
                ),
                (
                    "모르는 에이전트",
                    Some("grok"),
                    None,
                    None,
                    json!({"provider":null,"agent":"grok","account_id":null,"profile":null,"state":"unsupported"}),
                ),
            ];
            for (what, agent, folder, who, want) in cases {
                assert_eq!(seat_account_json(agent, folder, None, who, Some(home)), want, "{what}");
            }
            // ★(성찰 M5) 검증되지 않은 기록 폴더 = mismatch(계정 null · profile 은 기록 폴더 — 근거 표기) · 검증된 폴더가 있으면 기록은 무시된다.
            assert_eq!(
                seat_account_json(Some("claude"), None, Some("/h/.cys/claude"), None, Some(home)),
                json!({"provider":"claude","agent":"claude","account_id":null,"profile":".cys/claude","state":"mismatch"}),
                "검증되지 않은 기록 폴더의 신원을 계정으로 내면 안 된다"
            );
            assert_eq!(
                seat_account_json(Some("claude"), Some("/h/.claude-4"), Some("/h/.cys/claude"), Some(&known), Some(home))["state"],
                json!("known"),
                "검증된 폴더가 있으면 그 신원이다"
            );
            assert_eq!(seat_account_json(Some("codex"), None, Some("/h/x"), None, Some(home))["state"], json!("known"), "codex 는 폴더 축이 없다");
        }

        /// 실제 좌석으로 만든 표 — 같은 계정을 나눠 쓰는 두 좌석 · 다른 계정 좌석 · 로그아웃 폴더 · 폴더 미상 · codex · 셸 · 종료 좌석.
        /// 로그인을 바꾸면(캐시 하한이 지난 뒤) 같은 좌석의 account_id 가 바뀐다 — 좌석을 다른 계정으로 다시 띄운 경우와 같다. 수집 실패면 표가 비어 전부 null.
        #[test]
        fn seat_account_map_follows_each_seats_current_folder_login() {
            let dir = tmp("acct0145-daemon");
            let home = tmp("acct0145-home");
            let _g = test_home::set(&home);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let fa = b3_login(&home, ".claude-4", "u-a", "a@example.test", 1);
            let fb = b3_login(&home, ".claude-1", "u-b", "b@example.test", 2);
            let fout = home.join(".claude-2");
            std::fs::create_dir_all(&fout).unwrap(); // 신원 파일 없음 = 로그인 없음 확정
            let master = b3_seat(&d, "master", "claude", Some(&fa));
            let worker = b3_seat(&d, "worker", "claude", Some(&fa));
            let cso = b3_seat(&d, "cso", "claude", Some(&fb));
            let out = b3_seat(&d, "worker-2", "claude", Some(&fout));
            let nofolder = b3_seat(&d, "worker-3", "claude", None);
            let codex = b3_seat(&d, "reviewer-codex", "codex", None);
            let gone = b3_seat(&d, "worker-4", "claude", Some(&fb));
            gone.exited.store(true, Ordering::Relaxed);
            let shell = d.create_surface(None, Some("sleep 30".into()), None, None, 24, 80).expect("create surface");
            d.surfaces.lock().unwrap().insert(shell.id, shell.clone());
            let t0 = crate::state::now_epoch();
            let view = seat_identity_view_in(&d, Some(&home), t0);
            let m = seat_account_map(&d, &view);
            let acct = |id: u64| m.get(&id).cloned();
            assert_eq!(acct(master.id).unwrap()["account_id"], json!("u-a"));
            assert_eq!(acct(worker.id).unwrap()["account_id"], json!("u-a"), "같은 폴더를 쓰는 두 좌석은 같은 계정이다");
            assert_eq!(acct(master.id).unwrap()["profile"], json!(".claude-4"), "profile 은 usage.accounts 의 profiles 와 같은 홈 상대 표기");
            assert_eq!(acct(cso.id).unwrap()["account_id"], json!("u-b"));
            let o = acct(out.id).unwrap();
            assert_eq!((o["account_id"].clone(), o["state"].clone()), (Value::Null, json!("no_login")), "로그아웃 폴더에 계정을 지어냈다: {o}");
            let n = acct(nofolder.id).unwrap();
            assert_eq!((n["account_id"].clone(), n["state"].clone()), (Value::Null, json!("folder_unknown")));
            assert_eq!(acct(codex.id).unwrap()["account_id"], json!("default"));
            assert_eq!(acct(shell.id), Some(Value::Null), "셸 좌석은 null");
            assert_eq!(acct(gone.id), None, "종료 좌석은 표에 없다(부른 쪽이 null)");
            // 같은 폴더의 로그인을 B 로 바꾼다 — 60초 하한이 지난 뒤의 조회가 새 계정을 본다
            b3_login(&home, ".claude-4", "u-c", "c@example.test", 3);
            let view = seat_identity_view_in(&d, Some(&home), t0 + SEAT_IDENT_CACHE_SECS + 1.0);
            let m = seat_account_map(&d, &view);
            assert_eq!(m[&master.id]["account_id"], json!("u-c"), "계정 전환이 좌석 계정에 반영되지 않았다");
            // 수집 실패(기본값) — 표가 비어 전부 null
            assert!(seat_account_map(&d, &SeatIdentityView::default()).is_empty());
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }

        /// ★(0.14.45 · 성찰 M5 · 순수 표) 좌석 폴더 3값 — 관측 transcript 가 이기고 · 관측이 없으면 신뢰된 기록만 검증된 폴더 · 신뢰되지 않은 기록은 `Unverified` · 둘 다 없으면 `Unknown`.
        #[test]
        fn resolve_seat_folder_table() {
            use SeatFolder::*;
            let rec = "/h/.cys/claude";
            let tr = "/h/.claude-4/projects/-x/abc.jsonl";
            let cases: Vec<(&str, Option<&str>, bool, &str, SeatFolder)> = vec![
                ("기록 신뢰 · 관측 없음 → 기록", Some(rec), true, "", Verified(rec.into())),
                ("기록 불신 · 관측 없음 → 불일치(검증 불가)", Some(rec), false, "", Unverified(rec.into())),
                ("기록 신뢰 · 관측이 다른 폴더 → 관측이 이긴다", Some(rec), true, tr, Verified("/h/.claude-4".into())),
                ("기록 불신 · 관측이 다른 폴더 → 관측", Some(rec), false, tr, Verified("/h/.claude-4".into())),
                ("기록 불신 · 관측이 같은 폴더 → 검증됨(기록 표기)", Some("/h/.claude-4/"), false, tr, Verified("/h/.claude-4/".into())),
                ("기록 없음 · 관측 → 관측", None, true, tr, Verified("/h/.claude-4".into())),
                ("빈 기록 · 관측 → 관측", Some("  "), false, tr, Verified("/h/.claude-4".into())),
                ("기록 없음 · 관측 없음 → 미상", None, true, "", Unknown),
                ("기록 없음 · 프로필을 못 자르는 관측 → 미상", None, true, "/projects/x.jsonl", Unknown),
                ("기록 불신 · 프로필을 못 자르는 관측 → 불일치", Some(rec), false, "/tmp/noproj.jsonl", Unverified(rec.into())),
            ];
            for (what, cfg, trusted, sf, want) in cases {
                assert_eq!(resolve_seat_folder(cfg, trusted, sf), want, "{what}");
            }
            assert!(same_dir("/h/.claude-4", "/h/.claude-4/") && same_dir(r"C:\u\.claude-4", "C:/u/.claude-4"));
            assert!(!same_dir("/h/.claude-4", "/h/.claude-40") && !same_dir("/h/.claude-4", "/h/.cys/claude"));
            assert_eq!(same_dir("/h/A", "/h/a"), cfg!(windows), "대소문자 무시는 윈도우만");
        }

        /// ★(0.14.45 · 성찰 M5) 실제 좌석 — 호출자가 계정 폴더를 준 좌석(`config_dir_trusted=false`)은 기록 폴더에 로그인이 있어도 `mismatch`(그 계정을 확인된 것으로
        /// 내지 않는다). transcript 가 관측되면 **그 폴더**의 신원이 계정이다(기록과 달라도). 신뢰된 기록 좌석은 종전대로. 신원 표의 `in_use` 도 불일치 좌석을 '모름' 으로 센다.
        #[test]
        fn seat_account_map_never_confirms_an_unverified_recorded_folder() {
            let dir = tmp("acct0145m5-daemon");
            let home = tmp("acct0145m5-home");
            let _g = test_home::set(&home);
            let d = crate::state::Daemon::new(dir.join("cysd.sock"));
            let recorded = b3_login(&home, ".cys/claude", "u-rec", "rec@example.test", 1);
            let actual = b3_login(&home, ".claude-4", "u-act", "act@example.test", 2);
            // 호출자 오버라이드 = 신뢰되지 않은 기록(복원 좌석 · 수동 CLAUDE_CONFIG_DIR 과 같은 꼴)
            let untrusted = d
                .create_surface_with_env(None, Some("sleep 30".into()), None, Some("worker".into()), 24, 80, &[], Some(recorded.to_string_lossy().into_owned()))
                .expect("create surface");
            d.surfaces.lock().unwrap().insert(untrusted.id, untrusted.clone());
            *untrusted.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
            assert!(!untrusted.config_dir_trusted && untrusted.claude_config_dir.lock().unwrap().as_deref() == Some(recorded.to_str().unwrap()));
            let trusted = b3_seat(&d, "master", "claude", Some(&recorded));
            assert!(trusted.config_dir_trusted);
            let t0 = crate::state::now_epoch();
            let view = seat_identity_view_in(&d, Some(&home), t0);
            let m = seat_account_map(&d, &view);
            let u = &m[&untrusted.id];
            assert_eq!((u["state"].clone(), u["account_id"].clone(), u["profile"].clone()), (json!("mismatch"), Value::Null, json!(".cys/claude")), "{u}");
            assert_eq!(m[&trusted.id]["account_id"], json!("u-rec"), "신뢰된 기록 좌석은 종전대로");
            let ident = view.seats.iter().find(|s| s.surface_id == untrusted.id).unwrap();
            assert_eq!((ident.folder.clone(), ident.unverified_dir.as_deref(), ident.known), (None, Some(recorded.to_str().unwrap()), false));
            assert!(view.claude_folder_unknown, "불일치 좌석은 신원 표에서 폴더 미상과 같은 급(어느 계정의 in_use 도 확정하지 않는다)");
            assert_eq!(account_in_use("claude", "u-rec", &view), Some(true), "신뢰된 좌석이 u-rec 을 쓴다");
            assert_eq!(account_in_use("claude", "u-act", &view), None, "불일치 좌석이 있으면 다른 계정의 '안 씀' 을 확정하지 않는다");
            // transcript 관측 → 관측 폴더의 신원(기록과 다르다)
            let sf = actual.join("projects").join("-x").join("abc.jsonl").to_string_lossy().into_owned();
            *untrusted.observed_usage.lock().unwrap() = Some(crate::usage::ObservedUsage {
                agent: "claude".into(),
                ctx_tokens: None,
                ctx_window: None,
                ctx_pct: None,
                rate: vec![],
                source: "transcript".into(),
                session_file: sf,
                updated_at: t0,
                rate_observed_at: 0.0,
                rate_account: None,
            });
            let view = seat_identity_view_in(&d, Some(&home), t0 + SEAT_IDENT_CACHE_SECS + 1.0);
            let m = seat_account_map(&d, &view);
            let u = &m[&untrusted.id];
            assert_eq!((u["state"].clone(), u["account_id"].clone(), u["profile"].clone()), (json!("known"), json!("u-act"), json!(".claude-4")), "관측 폴더가 계정이다: {u}");
            assert!(!view.claude_folder_unknown);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&home);
        }
    }
}
