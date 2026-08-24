//! 프로필 **인증 전제** 순수 판정기(U-17) — "이 프로필로 좌석을 만들면 로그인 관문 앞에 서는가".
//!
//! ## 왜 판정이 시드보다 먼저인가
//!
//! 첫기동 관문 시드(U-19)는 **로그인 화면을 지운다**. 미인증 프로필에 시드를 먼저 내면 노드는
//! 관문 없이 태어난 것처럼 보이지만 실제로는 인증이 없어 곧 401·`Not logged in` 으로 죽는다.
//! 그래서 "인증되어 있는가" 를 **시드 이전에** 판정할 수 있어야 한다. 이 모듈이 그 판정기다.
//!
//! ## ★실측 (V-g · 2026-08-24 · Claude Code 2.1.241 · macOS · 격리 `CLAUDE_CONFIG_DIR`)
//!
//! `claude auth status --json` 을 격리 config dir 로 6조합 측정한 결과 —
//!
//! | 조합 | rc | `loggedIn` | `authMethod` | `apiProvider` |
//! |---|---|---|---|---|
//! | env 없음 | 1 | false | `none` | firstParty |
//! | `ANTHROPIC_API_KEY` | 0 | true | `api_key` | firstParty (`apiKeySource:ANTHROPIC_API_KEY`) |
//! | `CLAUDE_CODE_OAUTH_TOKEN` | 0 | true | `oauth_token` | firstParty |
//! | `ANTHROPIC_AUTH_TOKEN` | 0 | true | `oauth_token` | firstParty |
//! | `CLAUDE_CODE_USE_BEDROCK=1` | 0 | true | `third_party` | bedrock |
//! | `CLAUDE_CODE_USE_VERTEX=1` | 0 | true | `third_party` | vertex |
//! | (구독 로그인 · MEMORY 정본 2026-08-23) | 0 | true | `claude.ai` | firstParty |
//!
//! ### ★이 측정이 뒤집은 전제 — `.claude.json` 만으로는 인증을 알 수 없다
//!
//! **API 키로 인증된 프로필과 아무 인증도 없는 프로필의 `.claude.json` 은 동일했다**(둘 다
//! `firstStartTime`·`machineID`·`migrationVersion` 등 8키 · `oauthAccount` 없음 ·
//! `hasCompletedOnboarding` 없음). 자격증명은 config 파일이 아니라 **환경변수·Keychain·
//! settings 헬퍼**에 있고, macOS Keychain 서비스명은
//! `Claude Code-credentials-<sha256(configDir 절대경로)[:8]>` 로 **경로 해시 봉인**이다
//! (MEMORY `boot-determinism-audit-canon` 2026-08-23 실측: 복사·이동·`oauthAccount` 시드로는
//! 인증되지 않는다 = **E-2 반증**).
//!
//! 귀결 둘 — 둘 다 이 모듈의 계약으로 박제한다:
//!   ① **`oauthAccount` 존재를 인증 근거로 쓰지 않는다.** 파싱은 하되(진단·위조 탐지용)
//!      **등급 판정 입력이 아니다**. 존재만으로 통과시키면 위조된 신원이 좌석을 얻는다.
//!   ② **`oauthAccount` 를 만들지 않는다.** 이 모듈에는 쓰기 API 가 하나도 없다(검체가 소스에서
//!      `fs::write`·`File::create`·`OpenOptions`·`write_all` 부재를 단언한다 — 위조 신원 금지).
//!
//! ## 두 축을 섞지 않는다 — 등급(무엇인가) ↔ 증거(어떻게 알았는가)
//!
//! * **등급 축**([`AuthClass`], 8값): `Unknown` 은 **통과가 아니다**. 8값 중 통과는 5값뿐이고
//!   `NotLoggedIn`·`OnboardingPending`·`Unknown` 은 전부 비통과다. "측정 불능을 통과로 접는 것"
//!   이 이 저장소의 반복 사고 원인이므로 여기서 fail-closed 를 지킨다.
//! * **증거 축**([`EvidenceGrade`]): 오라클(`claude auth status --json`)을 거쳤는가, 아니면
//!   `.claude.json` 뿐인가. **차단 정책은 이 모듈이 소유하지 않는다**(U-18 이 소비한다) —
//!   산출자는 자기 산출물의 통과를 판정하지 않는다.
//!
//! ### ★U-18 에게 (오살 경보 · 이 저장소 제1 계약)
//!
//! 오라클 없이(`EvidenceGrade::ConfigOnly`) 이 판정기로 **차단하면 안 된다.** 위 측정대로
//! config 만 보는 경로는 정상 API키·oauth_token·bedrock 사용자를 전부 `Unknown` 으로 낸다 —
//! 그 위에 차단을 걸면 **살아있는 좌석을 전멸**시킨다(오살 ≫ 오탐). 반대로 오라클을 돌린 뒤의
//! `Unknown` 은 진짜 측정 불능(스키마 드리프트·rc 모순)이므로 차단해도 좋다.
//! 오라클을 못 돌리는 상황(claude 바이너리 부재·PATH 파손)은 **어차피 스폰도 불가능한 상태**라
//! 별도 예외가 아니다 — 다만 그 판정은 스폰 시도 실패로 내는 것이지 이 모듈의 등급으로 내는 게
//! 아니다.
//!
//! ## 롤백 스위치 — **마스터 하나 + 축 노브 하나**
//!
//! | 스위치 | 값 | 되돌아가는 범위 |
//! |---|---|---|
//! | **`CYS_BOOT_GATES`** | `0` | ★이 캠페인이 추가한 판정 축 **전부**(readiness·주입 가드·신뢰 정책·보류 귀결·**이 축**) |
//! | `CYS_PROFILE_GATE_OBSERVE_ONLY` | `1` | 이 판정기만 **관측·보고 전용**으로 강등(차단 안 함) |
//!
//! env 를 읽는 곳은 [`observe_only`] 하나뿐이고 축 판정은 순수 [`observe_only_from`] 에 있다.
//! 느슨한 truthy 를 받지 않는 것(`== Some("1")`)은 형제 게이트와 같은 규율이다.
//!
//! ## 부작용 경고 — 오라클은 순수하지 않다(이 모듈은 오라클을 부르지 않는다)
//!
//! `claude auth status --json` 은 대상 config dir 에 `.claude.json`·`.claude.json.lock`·
//! `backups/` 를 **생성한다**(V-g 실측). 그래서 이 모듈은 프로세스를 띄우지 않고 **이미 얻은
//! stdout 을 파싱만** 한다([`parse_oracle`]). 오라클을 실제로 실행하는 자리는 CLI 껍데기이며,
//! 데몬은 동기 서브프로세스 호출 금지다(U-18: tokio 워커 점유 · `PIPE_LISTENER_POOL=8` 포화).

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────
// 롤백 축 — 마스터 스위치에 접힌다
// ─────────────────────────────────────────────────────────────────────────────

/// ★롤백 스위치의 env 이름(1지점).
pub const ENV_OBSERVE_ONLY: &str = "CYS_PROFILE_GATE_OBSERVE_ONLY";

/// 이 판정기를 **관측·보고 전용**으로 강등할 것인가(= 차단하지 않는다).
///
/// 자기 축의 노브와 **상위 접기값**을 OR 한다 — 마스터 스위치(`CYS_BOOT_GATES=0`) 하나로
/// 전 축이 종전 복귀한다(BLOCK-3). 사고 순간에 사람이 노브를 조합할 수는 없다.
/// **env 를 읽는 유일한 지점.**
pub fn observe_only() -> bool {
    observe_only_from(std::env::var(ENV_OBSERVE_ONLY).ok().as_deref())
        || crate::gate_axes_forced_legacy()
}

/// 위 판정의 순수 절반(테스트가 env 를 건드리지 않게 분리).
pub fn observe_only_from(raw: Option<&str>) -> bool {
    raw == Some("1")
}

// ─────────────────────────────────────────────────────────────────────────────
// 프로필 dir 열거 규칙 — **정본 하나**(재구현 금지)
// ─────────────────────────────────────────────────────────────────────────────

/// 프로필 dir 이 사는 **뿌리 두 곳**. 뿌리마다 이름 규칙이 다르다(홈은 점 접두, `~/.cys` 는 아님).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileRoot {
    /// `$HOME` 직하 — `~/.claude`, `~/.claude-2`, `~/.claude-cysinsight` …
    Home,
    /// `$HOME/.cys` 직하 — `~/.cys/claude`, `~/.cys/claude-default-dept-1` …
    CysHome,
}

/// ★열거 규칙의 **순수 정본**. 종전엔 `cysd/accounts.rs seed_known` 안에만 있었고, 인증 판정기가
/// 같은 규칙을 재구현하면 두 벌이 갈린다(한쪽만 새 부서 접두를 배우는 식). 규칙은 여기 하나다.
///
/// ★**디렉터리 여부를 보지 않는다** — `seed_known` 종전 동작과 바이트 동일한 판정을 유지하기
/// 위해서다(이름만 맞으면 후보에 넣고, 실제 판별은 `<dir>/.claude.json` 읽기가 한다).
/// 여기에 `is_dir()` 을 더하는 것은 완화가 아니라 **동작 변경**이므로 별도 단위에서 다룬다.
pub fn is_profile_dir_name(root: ProfileRoot, name: &str) -> bool {
    match root {
        // `~/.claude.json`(홈 직하 파일)은 `.claude` 도 아니고 `.claude-` 접두도 아니라 제외된다.
        ProfileRoot::Home => name == ".claude" || name.starts_with(".claude-"),
        ProfileRoot::CysHome => name == "claude" || name.starts_with("claude-"),
    }
}

/// 위 규칙의 **유일한 IO 껍데기**. `seed_known`(계정 발견)과 `cys profile-auth`(인증 판정)가
/// 같은 목록을 본다.
///
/// 읽기 실패(홈 부재·권한)는 종전과 같이 **조용히 빈 목록**이다 — 이 함수는 열거이지 판정이
/// 아니고, 판정(측정 불능 = 비통과)은 [`classify`] 가 프로필 **하나씩** 소유한다.
pub fn enumerate_profile_dirs(home: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for (root, base) in [
        (ProfileRoot::Home, home.to_path_buf()),
        (ProfileRoot::CysHome, home.join(".cys")),
    ] {
        for e in std::fs::read_dir(&base).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if is_profile_dir_name(root, &name) {
                out.push(e.path());
            }
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// 등급 — 8값. `Unknown` 은 통과가 아니다.
// ─────────────────────────────────────────────────────────────────────────────

/// 프로필의 인증 등급. **8값**이며 통과는 앞의 다섯뿐이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthClass {
    /// `authMethod:"claude.ai"` — 사람이 OAuth 로그인한 구독 계정.
    Subscription,
    /// `authMethod:"oauth_token"` — `CLAUDE_CODE_OAUTH_TOKEN` / `ANTHROPIC_AUTH_TOKEN`.
    OauthToken,
    /// `authMethod:"api_key"` — `ANTHROPIC_API_KEY`.
    ApiKey,
    /// `authMethod:"api_key_helper"` — `settings.json` 의 `apiKeyHelper`.
    ApiKeyHelper,
    /// `authMethod:"third_party"` — Bedrock / Vertex.
    ThirdParty,
    /// `loggedIn:false` + `authMethod:"none"` — **확정 미인증**. 좌석을 만들면 로그인 관문에 선다.
    NotLoggedIn,
    /// `.claude.json` 부재 또는 온보딩 미완 — **확정 관문 앞**(테마 선택·온보딩 마법사).
    OnboardingPending,
    /// ★**측정 불능**. 통과가 아니다 — 스키마 드리프트·rc 모순·손상 config·오라클 부재.
    Unknown,
}

impl AuthClass {
    /// 안정 문자열(JSON·로그 계약). 값을 바꾸면 소비자 계약이 깨진다.
    pub fn as_str(self) -> &'static str {
        match self {
            AuthClass::Subscription => "subscription",
            AuthClass::OauthToken => "oauth_token",
            AuthClass::ApiKey => "api_key",
            AuthClass::ApiKeyHelper => "api_key_helper",
            AuthClass::ThirdParty => "third_party",
            AuthClass::NotLoggedIn => "not_logged_in",
            AuthClass::OnboardingPending => "onboarding_pending",
            AuthClass::Unknown => "unknown",
        }
    }

    /// ★**이 판정기의 계약**: 인증이 확인된 다섯 등급만 통과다.
    ///
    /// `Unknown` 이 `false` 인 것이 이 함수의 존재 이유다 — 측정 불능을 통과로 접으면
    /// 미인증 프로필이 좌석을 얻고, 관문 화면의 `❯` 가 readiness 오탐을 내고, 신뢰·면책
    /// Return 연쇄가 그 좌석을 죽인다(킬체인).
    pub fn allows_spawn(self) -> bool {
        matches!(
            self,
            AuthClass::Subscription
                | AuthClass::OauthToken
                | AuthClass::ApiKey
                | AuthClass::ApiKeyHelper
                | AuthClass::ThirdParty
        )
    }

    /// 8값 전수(진리표·검체 전용). 값이 늘면 여기부터 적색이 난다.
    pub const ALL: [AuthClass; 8] = [
        AuthClass::Subscription,
        AuthClass::OauthToken,
        AuthClass::ApiKey,
        AuthClass::ApiKeyHelper,
        AuthClass::ThirdParty,
        AuthClass::NotLoggedIn,
        AuthClass::OnboardingPending,
        AuthClass::Unknown,
    ];

    /// 오라클 `authMethod` 문자열 → 등급. 미지 값은 `None`(= 측정 불능, 통과 아님).
    pub fn from_auth_method(method: &str) -> Option<AuthClass> {
        Some(match method {
            "claude.ai" => AuthClass::Subscription,
            "oauth_token" => AuthClass::OauthToken,
            "api_key" => AuthClass::ApiKey,
            "api_key_helper" => AuthClass::ApiKeyHelper,
            "third_party" => AuthClass::ThirdParty,
            "none" => AuthClass::NotLoggedIn,
            _ => return None,
        })
    }
}

/// **증거 축** — 등급을 무엇으로 알았는가. 차단 정책(U-18)이 소비한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceGrade {
    /// `claude auth status --json` 을 거쳤다. 이 등급 위에서만 차단이 정당하다.
    OracleVerified,
    /// `.claude.json` 뿐이다. ★이 위에서 차단하면 정상 API키·토큰·bedrock 좌석이 전멸한다.
    ConfigOnly,
}

impl EvidenceGrade {
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceGrade::OracleVerified => "oracle_verified",
            EvidenceGrade::ConfigOnly => "config_only",
        }
    }
}

/// 그 등급이 나온 **이유**. 사고 후 원인 추적이 가능해야 하므로 판정과 함께 보존한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// 오라클 `authMethod` 매핑 성공.
    OracleAuthMethod,
    /// 오라클 `loggedIn:false` + `authMethod:"none"`.
    OracleLoggedOut,
    /// 오라클 `authMethod` 가 미지 값 — 스키마 드리프트(측정 불능).
    OracleUnknownMethod,
    /// 오라클의 `loggedIn` 과 `authMethod` 가 서로 모순(측정 불능).
    OracleSelfContradiction,
    /// 오라클의 `loggedIn` 과 프로세스 rc 가 모순(측정 불능 · rc0⟺true 가 실측 계약).
    OracleContradictsExit,
    /// 오라클 stdout 을 파싱하지 못했다(측정 불능).
    OracleUnparsable,
    /// `.claude.json` 부재 — 관문(테마 선택) 앞.
    ConfigAbsent,
    /// `.claude.json` 을 읽지 못했다(권한·IO) — 측정 불능.
    ConfigUnreadable,
    /// `.claude.json` 이 JSON 이 아니거나 최상위가 객체가 아니다 — 측정 불능.
    ConfigMalformed,
    /// `hasCompletedOnboarding` 이 참이 아니다 — 온보딩 관문 앞.
    ConfigOnboardingIncomplete,
    /// `hasCompletedOnboarding` 이 bool 이 아니다 — 측정 불능.
    ConfigOnboardingUnreadable,
    /// ★오라클이 없다. config 의 `oauthAccount` 주장은 **인증이 아니다**(경로해시 봉인 · E-2).
    ConfigClaimIsNotAuthentication,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::OracleAuthMethod => "oracle_auth_method",
            Reason::OracleLoggedOut => "oracle_logged_out",
            Reason::OracleUnknownMethod => "oracle_unknown_method",
            Reason::OracleSelfContradiction => "oracle_self_contradiction",
            Reason::OracleContradictsExit => "oracle_contradicts_exit",
            Reason::OracleUnparsable => "oracle_unparsable",
            Reason::ConfigAbsent => "config_absent",
            Reason::ConfigUnreadable => "config_unreadable",
            Reason::ConfigMalformed => "config_malformed",
            Reason::ConfigOnboardingIncomplete => "config_onboarding_incomplete",
            Reason::ConfigOnboardingUnreadable => "config_onboarding_unreadable",
            Reason::ConfigClaimIsNotAuthentication => "config_claim_is_not_authentication",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 관측 재료 — 전부 순수 파서가 만든다
// ─────────────────────────────────────────────────────────────────────────────

/// 3값 관측 — "있다 / 없다 / **읽을 수 없다**". `Option` 으로 접으면 부재와 측정 불능이 섞인다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tri {
    Yes,
    No,
    Unreadable,
}

impl Tri {
    pub fn as_str(self) -> &'static str {
        match self {
            Tri::Yes => "yes",
            Tri::No => "no",
            Tri::Unreadable => "unreadable",
        }
    }
}

/// `<dir>/.claude.json` 한 파일의 관측 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigEvidence {
    /// 파일(또는 프로필 dir)이 없다 — 관문 앞.
    Absent,
    /// 열지 못했다(권한·IO) — 측정 불능.
    Unreadable,
    /// JSON 이 아니거나 최상위가 객체가 아니다 — 측정 불능.
    Malformed,
    Parsed {
        /// `hasCompletedOnboarding == true` 인가.
        onboarding: Tri,
        /// `oauthAccount.accountUuid` 가 비지 않은 문자열인가.
        ///
        /// ★**등급 판정 입력이 아니다**(E-2 반증 — 존재해도 인증이 아니다). 진단·위조 탐지
        /// 전용으로만 보존한다. 이 필드를 [`classify`] 의 분기 조건으로 되돌리면 위조된 신원이
        /// 좌석을 얻는다.
        oauth_claim: Tri,
    },
}

impl ConfigEvidence {
    pub fn state_str(&self) -> &'static str {
        match self {
            ConfigEvidence::Absent => "absent",
            ConfigEvidence::Unreadable => "unreadable",
            ConfigEvidence::Malformed => "malformed",
            ConfigEvidence::Parsed { .. } => "parsed",
        }
    }
}

/// `claude auth status --json` 한 번의 산출(파싱 결과 — 이 모듈은 프로세스를 띄우지 않는다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleEvidence {
    pub logged_in: bool,
    pub auth_method: String,
    pub api_provider: Option<String>,
    pub api_key_source: Option<String>,
    /// 프로세스 rc. 실측 계약: `rc == 0 ⟺ loggedIn == true`. 갈리면 측정 불능이다.
    pub exit_code: Option<i32>,
}

/// 한 프로필에 대해 모은 관측 전량.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileEvidence {
    pub config: ConfigEvidence,
    /// `None` = 오라클을 돌리지 않았거나 파싱하지 못했다.
    pub oracle: Option<OracleEvidence>,
    /// 오라클을 **돌리기는 했는데** 파싱에 실패했는가(부재와 구별해 이유를 보존한다).
    pub oracle_attempted: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// 순수 파서
// ─────────────────────────────────────────────────────────────────────────────

/// `.claude.json` 본문 → 관측. **순수**(IO 없음).
pub fn parse_config(text: &str) -> ConfigEvidence {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return ConfigEvidence::Malformed,
    };
    let obj: &Map<String, Value> = match v.as_object() {
        Some(o) => o,
        None => return ConfigEvidence::Malformed,
    };
    let onboarding = match obj.get("hasCompletedOnboarding") {
        None | Some(Value::Null) => Tri::No,
        Some(Value::Bool(true)) => Tri::Yes,
        Some(Value::Bool(false)) => Tri::No,
        // bool 이 아닌 값 = 우리가 아는 스키마가 아니다. 부재로 접지 않는다(측정 불능).
        Some(_) => Tri::Unreadable,
    };
    let oauth_claim = match obj.get("oauthAccount") {
        None | Some(Value::Null) => Tri::No,
        Some(Value::Object(oa)) => match oa.get("accountUuid").and_then(|x| x.as_str()) {
            Some(s) if !s.trim().is_empty() => Tri::Yes,
            // `oauthAccount` 는 있는데 uuid 를 못 읽는다 = 손상 또는 시드된 껍데기.
            _ => Tri::Unreadable,
        },
        Some(_) => Tri::Unreadable,
    };
    ConfigEvidence::Parsed { onboarding, oauth_claim }
}

/// `<dir>/.claude.json` 을 **읽기만** 한다(이 모듈의 유일한 IO — 쓰기는 어디에도 없다).
pub fn read_config(profile_dir: &Path) -> ConfigEvidence {
    match std::fs::read_to_string(profile_dir.join(".claude.json")) {
        Ok(text) => parse_config(&text),
        // dir 부재도 파일 부재도 같은 뜻이다: 이 프로필은 아직 관문 앞이다.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ConfigEvidence::Absent,
        Err(_) => ConfigEvidence::Unreadable,
    }
}

/// stdout 에서 **첫 JSON 객체**만 잘라낸다.
///
/// ★이 저장소의 선재 결함(러너 `--json` stdout 에 검체 출력이 섞임)과 같은 함정이다 —
/// 도구 stdout 은 언제든 앞뒤에 잡음이 붙는다. 전량 파싱은 잡음 한 줄에 측정 불능이 되고,
/// 측정 불능은 이 판정기에서 **비통과**이므로 잡음이 좌석을 죽인다. 그래서 파이썬
/// `raw_decode` 와 같은 규율로 첫 객체를 괄호 짝으로 찾는다(문자열·이스케이프 인지).
fn first_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = s.find('{')?;
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for i in start..bytes.len() {
        let c = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return s.get(start..=i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `claude auth status --json` stdout → 관측. **순수**(프로세스 미기동).
///
/// `loggedIn`(bool)과 `authMethod`(string) 둘 다 있어야 관측으로 인정한다 — 하나라도 없으면
/// 우리가 아는 스키마가 아니므로 `None`(측정 불능)이다.
pub fn parse_oracle(stdout: &str, exit_code: Option<i32>) -> Option<OracleEvidence> {
    let slice = first_json_object(stdout)?;
    let v: Value = serde_json::from_str(slice).ok()?;
    let obj = v.as_object()?;
    let logged_in = obj.get("loggedIn")?.as_bool()?;
    let auth_method = obj.get("authMethod")?.as_str()?.to_string();
    Some(OracleEvidence {
        logged_in,
        auth_method,
        api_provider: obj.get("apiProvider").and_then(|x| x.as_str()).map(str::to_string),
        api_key_source: obj.get("apiKeySource").and_then(|x| x.as_str()).map(str::to_string),
        exit_code,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// 판정 — 순수함수 하나가 소유한다
// ─────────────────────────────────────────────────────────────────────────────

/// 판정 결과. 등급·증거·이유 셋을 함께 들고 다닌다(하나만 남기면 사고 후 추적이 끊긴다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub class: AuthClass,
    pub grade: EvidenceGrade,
    pub reason: Reason,
}

impl Verdict {
    /// 등급 축의 통과 여부. **정책이 아니다** — 차단할지는 U-18 이 [`Verdict::grade`] 와 함께 정한다.
    pub fn allows_spawn(self) -> bool {
        self.class.allows_spawn()
    }
}

/// ★**판정의 유일한 소유자**. env 도 IO 도 없다(전량 진리표 대상).
///
/// 규약:
///   ① 오라클이 있으면 오라클이 정본이다. 자기모순(`loggedIn` ↔ `authMethod` ↔ rc)은
///      **다수결로 접지 않고** 측정 불능(`Unknown`)으로 낸다.
///   ② 오라클이 없으면 config 로 **확정 관문**(`Absent` / 온보딩 미완)만 말할 수 있고,
///      그 밖은 전부 `Unknown` 이다.
///   ③ ★`oauth_claim` 은 어떤 분기 조건도 아니다 — 존재가 인증이 아니기 때문이다(E-2 반증).
pub fn classify(ev: &ProfileEvidence) -> Verdict {
    if let Some(o) = &ev.oracle {
        let grade = EvidenceGrade::OracleVerified;
        // rc 교차검증 — 실측 계약 `rc == 0 ⟺ loggedIn`. 갈리면 우리가 보는 도구가 아니다.
        if let Some(rc) = o.exit_code {
            if (rc == 0) != o.logged_in {
                return Verdict {
                    class: AuthClass::Unknown,
                    grade,
                    reason: Reason::OracleContradictsExit,
                };
            }
        }
        let mapped = AuthClass::from_auth_method(&o.auth_method);
        return match (o.logged_in, mapped) {
            // 미인증은 `authMethod:"none"` 과만 정합한다.
            (false, Some(AuthClass::NotLoggedIn)) => Verdict {
                class: AuthClass::NotLoggedIn,
                grade,
                reason: Reason::OracleLoggedOut,
            },
            (false, _) => Verdict {
                class: AuthClass::Unknown,
                grade,
                reason: Reason::OracleSelfContradiction,
            },
            // 인증됐다는데 방법이 `none` 이면 모순이다.
            (true, Some(AuthClass::NotLoggedIn)) => Verdict {
                class: AuthClass::Unknown,
                grade,
                reason: Reason::OracleSelfContradiction,
            },
            (true, Some(c)) => Verdict { class: c, grade, reason: Reason::OracleAuthMethod },
            // 새 authMethod = 스키마 드리프트. 통과로 접지 않는다.
            (true, None) => Verdict {
                class: AuthClass::Unknown,
                grade,
                reason: Reason::OracleUnknownMethod,
            },
        };
    }

    let grade = EvidenceGrade::ConfigOnly;
    if ev.oracle_attempted {
        // 돌렸는데 못 읽었다 — config 로 관문을 확정할 수 있으면 그쪽이 더 구체적인 사실이다.
        if let ConfigEvidence::Absent = ev.config {
            return Verdict {
                class: AuthClass::OnboardingPending,
                grade,
                reason: Reason::ConfigAbsent,
            };
        }
        if let ConfigEvidence::Parsed { onboarding: Tri::No, .. } = ev.config {
            return Verdict {
                class: AuthClass::OnboardingPending,
                grade,
                reason: Reason::ConfigOnboardingIncomplete,
            };
        }
        return Verdict { class: AuthClass::Unknown, grade, reason: Reason::OracleUnparsable };
    }

    match ev.config {
        ConfigEvidence::Absent => Verdict {
            class: AuthClass::OnboardingPending,
            grade,
            reason: Reason::ConfigAbsent,
        },
        ConfigEvidence::Unreadable => {
            Verdict { class: AuthClass::Unknown, grade, reason: Reason::ConfigUnreadable }
        }
        ConfigEvidence::Malformed => {
            Verdict { class: AuthClass::Unknown, grade, reason: Reason::ConfigMalformed }
        }
        ConfigEvidence::Parsed { onboarding: Tri::No, .. } => Verdict {
            class: AuthClass::OnboardingPending,
            grade,
            reason: Reason::ConfigOnboardingIncomplete,
        },
        ConfigEvidence::Parsed { onboarding: Tri::Unreadable, .. } => Verdict {
            class: AuthClass::Unknown,
            grade,
            reason: Reason::ConfigOnboardingUnreadable,
        },
        // ★온보딩은 끝났다. 그러나 **config 만으로는 인증을 알 수 없다**(V-g: API키 프로필과
        //   미인증 프로필의 `.claude.json` 이 동일했다). `oauth_claim` 이 있어도 없어도 같다 —
        //   주장은 인증이 아니다(경로해시 봉인 · E-2 반증).
        ConfigEvidence::Parsed { onboarding: Tri::Yes, .. } => Verdict {
            class: AuthClass::Unknown,
            grade,
            reason: Reason::ConfigClaimIsNotAuthentication,
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 보고 — `cys profile-auth --account <dir> --json` 의 본문
// ─────────────────────────────────────────────────────────────────────────────

/// 판정 전량을 JSON 으로. **키 이름은 계약**이다(GUI·python 소비자가 읽는다).
pub fn report_json(profile_dir: &Path, ev: &ProfileEvidence, v: &Verdict) -> Value {
    let (onboarding, oauth_claim) = match &ev.config {
        ConfigEvidence::Parsed { onboarding, oauth_claim } => {
            (Some(onboarding.as_str()), Some(oauth_claim.as_str()))
        }
        _ => (None, None),
    };
    json!({
        "profile_dir": profile_dir.to_string_lossy(),
        "auth_class": v.class.as_str(),
        "allows_spawn": v.allows_spawn(),
        "evidence_grade": v.grade.as_str(),
        "reason": v.reason.as_str(),
        "observe_only": observe_only(),
        "config": {
            "state": ev.config.state_str(),
            "has_completed_onboarding": onboarding,
            // ★진단 전용 — 판정 입력이 아니다(E-2). 소비자가 이 값으로 통과를 정하면 위조가 통한다.
            "oauth_account_claim": oauth_claim,
        },
        "oracle": ev.oracle.as_ref().map(|o| json!({
            "logged_in": o.logged_in,
            "auth_method": o.auth_method,
            "api_provider": o.api_provider,
            "api_key_source": o.api_key_source,
            "exit_code": o.exit_code,
        })),
        "oracle_attempted": ev.oracle_attempted,
    })
}

/// `cys profile-auth` 서브커맨드의 **본문 전량**(CLI 는 인자 파싱과 출력만 한다).
///
/// `oracle_stdout` 은 CLI 껍데기가 이미 실행해 얻은 `claude auth status --json` 의 stdout 이다
/// — 이 함수도, 이 모듈도 프로세스를 띄우지 않는다(부작용은 껍데기가 소유한다).
pub fn profile_auth_report(
    profile_dir: &Path,
    oracle_stdout: Option<(&str, Option<i32>)>,
) -> (Value, Verdict) {
    let ev = ProfileEvidence {
        config: read_config(profile_dir),
        oracle: oracle_stdout.and_then(|(out, rc)| parse_oracle(out, rc)),
        oracle_attempted: oracle_stdout.is_some(),
    };
    let v = classify(&ev);
    (report_json(profile_dir, &ev, &v), v)
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    // ── V-g 픽스처 (2026-08-24 실측 · 격리 CLAUDE_CONFIG_DIR · Claude Code 2.1.241) ──
    //
    // `docs/evidence/` 에 커밋될 산출물(U-3)의 **문면 그대로**를 여기 박제한다. 파서 검체가
    // 실물 문면에서 떨어지면 스키마 드리프트를 못 잡는다.
    const VG_NOT_LOGGED_IN: &str =
        "{\n  \"loggedIn\": false,\n  \"authMethod\": \"none\",\n  \"apiProvider\": \"firstParty\"\n}";
    const VG_API_KEY: &str = "{\n  \"loggedIn\": true,\n  \"authMethod\": \"api_key\",\n  \
         \"apiProvider\": \"firstParty\",\n  \"apiKeySource\": \"ANTHROPIC_API_KEY\"\n}";
    const VG_OAUTH_TOKEN: &str = "{\n  \"loggedIn\": true,\n  \"authMethod\": \"oauth_token\",\n  \
         \"apiProvider\": \"firstParty\"\n}";
    const VG_THIRD_PARTY: &str = "{\n  \"loggedIn\": true,\n  \"authMethod\": \"third_party\",\n  \
         \"apiProvider\": \"bedrock\"\n}";
    const VG_API_KEY_HELPER: &str =
        "{\n  \"loggedIn\": true,\n  \"authMethod\": \"api_key_helper\",\n  \
         \"apiProvider\": \"firstParty\",\n  \"apiKeySource\": \"apiKeyHelper\"\n}";
    // 구독 로그인 문면(MEMORY 정본 2026-08-23 — 이 기계는 격리 dir 로 재현 불가:
    // 자격증명이 configDir 경로 해시로 봉인되어 있다).
    const VG_SUBSCRIPTION: &str = "{\"loggedIn\":true,\"authMethod\":\"claude.ai\",\
         \"apiProvider\":\"firstParty\",\"subscriptionType\":\"max\"}";

    // ── `.claude.json` 픽스처 (같은 실측) ──
    //
    // ★핵심: 아래 둘은 **실제로 같은 파일**이었다 — 하나는 ANTHROPIC_API_KEY 로 인증된 프로필,
    //   하나는 아무 인증도 없는 프로필. config 만으로 인증을 판정할 수 없다는 증거다.
    const CFG_FRESH: &str = "{\"firstStartTime\":\"2026-08-24T08:20:00.000Z\",\
         \"firstStartVersion\":\"2.1.241\",\"hasResetAutoModeOptInForDefaultOffer\":true,\
         \"machineID\":\"deadbeef\",\"migrationVersion\":1,\"opusProMigrationComplete\":true,\
         \"seenNotifications\":[],\"sonnet1m45MigrationComplete\":true}";
    // 구독 프로필의 실제 형태(홈 프로필 5종 전수 관측 · 값은 redact).
    const CFG_SUBSCRIPTION_CLAIM: &str = "{\"hasCompletedOnboarding\":true,\"userID\":\"u\",\
         \"oauthAccount\":{\"accountUuid\":\"c45eaec5-0000-0000-0000-000000000000\",\
         \"emailAddress\":\"redacted@example.com\",\"userRateLimitTier\":\"max\"}}";

    fn ev(config: ConfigEvidence, oracle: Option<OracleEvidence>) -> ProfileEvidence {
        let attempted = oracle.is_some();
        ProfileEvidence { config, oracle, oracle_attempted: attempted }
    }

    fn oracle(text: &str, rc: Option<i32>) -> OracleEvidence {
        parse_oracle(text, rc).expect("V-g 픽스처가 파싱되지 않는다 — 스키마 드리프트")
    }

    // ─────────────────────────────────────────────────────────────────────
    // ① V-g 픽스처 전수 파싱 100%
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn vg_fixtures_parse_into_every_authenticated_class() {
        let cases: [(&str, Option<i32>, AuthClass); 6] = [
            (VG_SUBSCRIPTION, Some(0), AuthClass::Subscription),
            (VG_OAUTH_TOKEN, Some(0), AuthClass::OauthToken),
            (VG_API_KEY, Some(0), AuthClass::ApiKey),
            (VG_API_KEY_HELPER, Some(0), AuthClass::ApiKeyHelper),
            (VG_THIRD_PARTY, Some(0), AuthClass::ThirdParty),
            (VG_NOT_LOGGED_IN, Some(1), AuthClass::NotLoggedIn),
        ];
        for (text, rc, want) in cases {
            let o = oracle(text, rc);
            let v = classify(&ev(parse_config(CFG_FRESH), Some(o)));
            assert_eq!(v.class, want, "V-g 픽스처 등급 이탈: {text}");
            assert_eq!(v.grade, EvidenceGrade::OracleVerified);
            assert_eq!(v.allows_spawn(), want != AuthClass::NotLoggedIn, "통과 판정 이탈: {text}");
        }
    }

    /// ★설계 게이트의 3프로필(구독·미인증·API키) — 100% 파싱 + 등급 일치.
    #[test]
    fn three_profile_gate_subscription_unauthenticated_apikey() {
        assert_eq!(
            classify(&ev(parse_config(CFG_SUBSCRIPTION_CLAIM), Some(oracle(VG_SUBSCRIPTION, Some(0)))))
                .class,
            AuthClass::Subscription
        );
        assert_eq!(
            classify(&ev(parse_config(CFG_FRESH), Some(oracle(VG_NOT_LOGGED_IN, Some(1))))).class,
            AuthClass::NotLoggedIn
        );
        assert_eq!(
            classify(&ev(parse_config(CFG_FRESH), Some(oracle(VG_API_KEY, Some(0))))).class,
            AuthClass::ApiKey
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // ② `unknown` fail-closed — 통과가 아니다
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn unknown_is_never_a_pass_and_exactly_five_classes_pass() {
        assert!(!AuthClass::Unknown.allows_spawn(), "★측정 불능이 통과로 접혔다");
        assert!(!AuthClass::NotLoggedIn.allows_spawn());
        assert!(!AuthClass::OnboardingPending.allows_spawn());
        assert_eq!(AuthClass::ALL.len(), 8, "auth_class 는 8값 계약이다");
        assert_eq!(AuthClass::ALL.iter().filter(|c| c.allows_spawn()).count(), 5);
        // 문자열 계약이 중복되지 않는다(소비자가 등급을 구별할 수 있어야 한다).
        let mut seen: Vec<&str> = AuthClass::ALL.iter().map(|c| c.as_str()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 8, "auth_class 문자열이 충돌한다");
    }

    #[test]
    fn measurement_failures_all_land_on_unknown() {
        // 손상 JSON · 권한 오류 · 최상위 non-object · 스키마 드리프트 · rc 모순 · 자기모순.
        let cases: [(ProfileEvidence, Reason); 7] = [
            (ev(ConfigEvidence::Unreadable, None), Reason::ConfigUnreadable),
            (ev(parse_config("{not json"), None), Reason::ConfigMalformed),
            (ev(parse_config("[1,2,3]"), None), Reason::ConfigMalformed),
            (
                ev(parse_config("{\"hasCompletedOnboarding\":\"true\"}"), None),
                Reason::ConfigOnboardingUnreadable,
            ),
            (
                ev(
                    ConfigEvidence::Absent,
                    parse_oracle("{\"loggedIn\":true,\"authMethod\":\"quantum\"}", Some(0)),
                ),
                Reason::OracleUnknownMethod,
            ),
            (
                ev(
                    ConfigEvidence::Absent,
                    parse_oracle("{\"loggedIn\":true,\"authMethod\":\"api_key\"}", Some(1)),
                ),
                Reason::OracleContradictsExit,
            ),
            (
                ev(
                    ConfigEvidence::Absent,
                    parse_oracle("{\"loggedIn\":true,\"authMethod\":\"none\"}", Some(0)),
                ),
                Reason::OracleSelfContradiction,
            ),
        ];
        for (e, want) in cases {
            let v = classify(&e);
            assert_eq!(v.class, AuthClass::Unknown, "측정 불능이 unknown 이 아니다: {e:?}");
            assert_eq!(v.reason, want, "이유가 보존되지 않는다: {e:?}");
            assert!(!v.allows_spawn());
        }
    }

    /// 파일 부재 축 — dir 이 없든 파일만 없든 **관문 앞**이다(통과 아님).
    #[test]
    fn absent_config_is_a_gate_not_a_pass() {
        let v = classify(&ev(ConfigEvidence::Absent, None));
        assert_eq!(v.class, AuthClass::OnboardingPending);
        assert_eq!(v.reason, Reason::ConfigAbsent);
        assert!(!v.allows_spawn());
        // 실제 파일 시스템에서도 같은 값이 나온다(dir 자체가 없는 경우 포함).
        let missing = std::path::Path::new("/nonexistent-cys-profile-gate-probe/x");
        assert_eq!(read_config(missing), ConfigEvidence::Absent);
    }

    // ─────────────────────────────────────────────────────────────────────
    // ③ ★E-2 박제 — `oauthAccount` 존재는 인증이 아니다 / 위조 신원 금지
    // ─────────────────────────────────────────────────────────────────────

    /// ★계측 타당성(in-band) — **수리 전 술어**(`oauthAccount` 존재 = 인증)는 위조 config 를
    /// 통과시켰다. 지금 판정기는 같은 입력에서 통과시키지 않는다.
    ///
    /// 이것이 완화가 아니라 강화인 이유: 종전 술어는 파일에 객체 하나만 써 넣으면 참이 됐고,
    /// 자격증명은 `sha256(configDir)` 로 봉인돼 있어 그 파일은 **인증과 무관**하다(실측:
    /// 복사·이동·시드 어느 것도 로그인시키지 못한다).
    #[test]
    fn pre_fix_predicate_accepted_a_forged_identity_and_now_does_not() {
        let forged = "{\"hasCompletedOnboarding\":true,\
             \"oauthAccount\":{\"accountUuid\":\"00000000-forged-0000-0000-000000000000\",\
             \"emailAddress\":\"attacker@example.com\"}}";
        // 구 술어 재현 — "oauthAccount.accountUuid 가 있으면 인증됐다".
        let old_predicate = matches!(
            parse_config(forged),
            ConfigEvidence::Parsed { oauth_claim: Tri::Yes, .. }
        );
        assert!(old_predicate, "계측 무효: 구 술어가 위조 config 에서 참이 아니라면 E-2 서사가 틀린 것");
        // 지금 판정기: 오라클 없이는 통과 없음.
        let v = classify(&ev(parse_config(forged), None));
        assert_eq!(v.class, AuthClass::Unknown);
        assert_eq!(v.reason, Reason::ConfigClaimIsNotAuthentication);
        assert!(!v.allows_spawn(), "★위조된 신원이 좌석을 얻었다");
    }

    /// `oauth_claim` 은 **어떤 분기 조건도 아니다** — 세 값 전부에서 같은 판정이 나온다.
    #[test]
    fn oauth_claim_is_diagnostic_only_and_never_changes_the_verdict() {
        let mut seen = Vec::new();
        for claim in [Tri::Yes, Tri::No, Tri::Unreadable] {
            let e = ev(ConfigEvidence::Parsed { onboarding: Tri::Yes, oauth_claim: claim }, None);
            seen.push(classify(&e));
        }
        assert!(
            seen.windows(2).all(|w| w[0] == w[1]),
            "oauthAccount 주장이 판정을 갈랐다 — E-2 반증 이전으로 되돌아갔다: {seen:?}"
        );
        assert_eq!(seen[0].class, AuthClass::Unknown);
    }

    /// ★**위조 신원 금지의 소스 핀** — 이 모듈에 쓰기 API 가 하나도 없다.
    /// 합성 표본으로 탐지기 자체를 먼저 시험한다(트리에 위반이 0이면 탐지기가 고장나도 초록이다).
    #[test]
    fn this_module_can_never_write_a_claude_json() {
        const WRITE_APIS: [&str; 6] = [
            "fs::write(",
            "File::create(",
            // ★`OpenOptions::new(` 로 여는 이유: 모듈 헤더 산문이 이 계약을 설명하며 타입명을
            //   맨몸으로 언급한다. 산문 언급까지 위반으로 잡으면 탐지기가 자기 문서에 걸려
            //   **판정이 문서 편집에 좌우된다**(계측 무효). 호출 형태만 금지한다.
            "OpenOptions::new(",
            "write_all(",
            "to_writer(",
            "create_dir_all(",
        ];
        let detector = |src: &str| WRITE_APIS.iter().filter(|a| src.contains(**a)).count();
        // ⓐ 합성 양성 — 탐지기가 실제로 잡는가.
        assert_eq!(
            detector("let f = std::fs::File::create(p)?; f.write_all(b\"{}\")?;"),
            2,
            "계측 무효: 쓰기 API 탐지기가 합성 표본을 잡지 못한다"
        );
        // ⓑ 실물 — 이 파일에는 0건이어야 한다. 테스트 블록 자신은 잘라낸다(리터럴 자기참조 제외).
        let src = include_str!("profile_gate.rs");
        let body = &src[..src.find("#[cfg(test)]").expect("테스트 앵커 부재")];
        assert_eq!(detector(body), 0, "★이 모듈이 쓰기 API 를 얻었다 — 위조 신원 금지 계약 붕괴");
        // ⓒ 그리고 `oauthAccount` 는 **읽기 키**로만 등장한다(생성 리터럴 금지).
        assert!(
            !body.contains("\"oauthAccount\":"),
            "★`oauthAccount` 를 만드는 리터럴이 생겼다 — 위조 신원 금지"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // ④ 오살 방지 — config 만으로는 절대 통과가 나오지 않는다
    // ─────────────────────────────────────────────────────────────────────

    /// ★`ConfigOnly` 증거로는 **어떤 통과 등급도** 나오지 않는다(전수).
    ///
    /// 그 역이 U-18 의 오살 경보다: config 만 보고 차단하면 정상 API키·토큰·bedrock 좌석이
    /// 전멸한다. 그래서 등급(비통과)과 증거(ConfigOnly)를 **함께** 내보내고, 차단 정책은
    /// 증거 축까지 본 뒤 U-18 이 정한다.
    #[test]
    fn config_only_evidence_never_produces_a_passing_class() {
        let configs = [
            ConfigEvidence::Absent,
            ConfigEvidence::Unreadable,
            ConfigEvidence::Malformed,
        ];
        let mut all: Vec<ConfigEvidence> = configs.to_vec();
        for onboarding in [Tri::Yes, Tri::No, Tri::Unreadable] {
            for oauth_claim in [Tri::Yes, Tri::No, Tri::Unreadable] {
                all.push(ConfigEvidence::Parsed { onboarding, oauth_claim });
            }
        }
        assert_eq!(all.len(), 12, "config 관측 전수가 12조합이 아니다");
        for c in all {
            for attempted in [false, true] {
                let e = ProfileEvidence {
                    config: c.clone(),
                    oracle: None,
                    oracle_attempted: attempted,
                };
                let v = classify(&e);
                assert_eq!(v.grade, EvidenceGrade::ConfigOnly, "증거 등급 오분류: {e:?}");
                assert!(!v.allows_spawn(), "★config 만으로 통과가 났다: {e:?} → {v:?}");
            }
        }
    }

    /// 오라클이 있으면 **오라클이 정본**이다 — config 가 무엇이든 등급이 흔들리지 않는다.
    #[test]
    fn oracle_wins_over_config_in_every_config_shape() {
        for text in [VG_SUBSCRIPTION, VG_API_KEY, VG_NOT_LOGGED_IN] {
            let rc = if text == VG_NOT_LOGGED_IN { Some(1) } else { Some(0) };
            let base = classify(&ev(ConfigEvidence::Absent, Some(oracle(text, rc)))).class;
            for c in [
                ConfigEvidence::Unreadable,
                ConfigEvidence::Malformed,
                parse_config(CFG_FRESH),
                parse_config(CFG_SUBSCRIPTION_CLAIM),
            ] {
                assert_eq!(
                    classify(&ev(c.clone(), Some(oracle(text, rc)))).class,
                    base,
                    "config 가 오라클 판정을 흔들었다"
                );
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // ⑤ stdout 잡음 내성 — 잡음이 좌석을 죽이면 안 된다
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn oracle_parser_survives_noise_around_the_json() {
        let noisy = format!("warning: node deprecation\n{VG_API_KEY}\ntrailing junk {{oops");
        let o = parse_oracle(&noisy, Some(0)).expect("잡음 앞뒤로 첫 객체를 못 찾았다");
        assert_eq!(o.auth_method, "api_key");
        assert_eq!(o.api_key_source.as_deref(), Some("ANTHROPIC_API_KEY"));
        // 중괄호가 문자열 안에 있어도 짝을 잘못 세지 않는다.
        let braces = "{\"loggedIn\":true,\"authMethod\":\"api_key\",\"note\":\"a}b{c\"} tail";
        assert_eq!(parse_oracle(braces, Some(0)).unwrap().auth_method, "api_key");
        // 이스케이프된 따옴표.
        let esc = "{\"loggedIn\":true,\"authMethod\":\"api_key\",\"note\":\"say \\\"}\\\"\"} tail";
        assert_eq!(parse_oracle(esc, Some(0)).unwrap().auth_method, "api_key");
        // 스키마가 아니면 관측이 아니다(부재 → 측정 불능).
        assert!(parse_oracle("no json here", None).is_none());
        assert!(parse_oracle("{\"loggedIn\":true}", None).is_none(), "authMethod 없이 관측 인정");
        assert!(parse_oracle("{\"authMethod\":\"api_key\"}", None).is_none());
        assert!(parse_oracle("{\"loggedIn\":\"true\",\"authMethod\":\"api_key\"}", None).is_none());
    }

    /// 오라클을 **돌렸는데 못 읽은** 경우도 통과가 아니다(부재와 이유가 구별된다).
    #[test]
    fn attempted_but_unparsable_oracle_is_not_a_pass() {
        let e = ProfileEvidence {
            config: parse_config(CFG_SUBSCRIPTION_CLAIM),
            oracle: parse_oracle("segfault", Some(139)),
            oracle_attempted: true,
        };
        let v = classify(&e);
        assert_eq!(v.class, AuthClass::Unknown);
        assert_eq!(v.reason, Reason::OracleUnparsable);
        assert!(!v.allows_spawn());
    }

    // ─────────────────────────────────────────────────────────────────────
    // ⑥ 열거 규칙 — 정본 하나(재구현 금지)
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn profile_dir_name_rule_matches_the_seed_known_behavior() {
        for n in [".claude", ".claude-2", ".claude-cysinsight", ".claude-2-dept-pub", ".claude-"] {
            assert!(is_profile_dir_name(ProfileRoot::Home, n), "홈 프로필 누락: {n}");
        }
        for n in [".claude.json", ".claudia", ".cys", "claude", "", ".clau"] {
            assert!(!is_profile_dir_name(ProfileRoot::Home, n), "홈에서 오탐: {n}");
        }
        for n in ["claude", "claude-default-dept-1", "claude-"] {
            assert!(is_profile_dir_name(ProfileRoot::CysHome, n), "~/.cys 프로필 누락: {n}");
        }
        for n in [".claude", "claudia", "pack", ""] {
            assert!(!is_profile_dir_name(ProfileRoot::CysHome, n), "~/.cys 에서 오탐: {n}");
        }
    }

    #[test]
    fn enumerate_covers_both_roots_and_skips_the_home_level_config_file() {
        let base = std::env::temp_dir().join(format!("cys-pg-enum-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in [".claude", ".claude-2", ".claudia", ".cys/claude", ".cys/claude-x", ".cys/pack"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        // 홈 직하의 `.claude.json` 은 프로필 dir 이 아니다(이름 규칙이 배제한다).
        std::fs::write(base.join(".claude.json"), "{}").unwrap();
        let mut got: Vec<String> = enumerate_profile_dirs(&base)
            .iter()
            .map(|p| p.strip_prefix(&base).unwrap().to_string_lossy().into_owned())
            .collect();
        got.sort();
        let want = [".claude", ".claude-2", ".cys/claude", ".cys/claude-x"]
            .iter()
            .map(|s| s.replace('/', std::path::MAIN_SEPARATOR_STR))
            .collect::<Vec<_>>();
        assert_eq!(got, want, "열거 규칙이 seed_known 종전 집합과 갈렸다");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★재구현 금지의 소스 핀 — `cysd/accounts.rs` 가 자기 이름 규칙을 다시 들고 있지 않다.
    #[test]
    fn accounts_seed_known_does_not_reimplement_the_enumeration_rule() {
        let acc = include_str!("bin/cysd/accounts.rs");
        // ⓐ 합성 양성 — 탐지기가 실제로 잡는가(구 트리 문면 그대로).
        let old = "if name == \".claude\" || name.starts_with(\".claude-\") {";
        let detect = |s: &str| s.contains(".starts_with(\".claude-\")") || s.contains("\"claude-\"");
        assert!(detect(old), "계측 무효: 열거 규칙 탐지기가 구 문면을 잡지 못한다");
        // ⓑ 실물 — 0건 + 정본 경유.
        assert!(!detect(acc), "★accounts.rs 가 열거 규칙을 재구현했다 — 규칙이 두 벌로 갈린다");
        assert!(
            acc.contains("cys::profile_gate::enumerate_profile_dirs("),
            "accounts.rs 가 열거 정본을 경유하지 않는다"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // ⑦ 롤백 축 — 마스터 스위치에 접힌다
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn observe_only_switch_is_strict_and_folds_into_the_master() {
        assert!(!observe_only_from(None), "미설정이 관측 전용으로 접혔다(기능 소실)");
        assert!(observe_only_from(Some("1")));
        for loose in ["true", "yes", "on", "", "0", "01", " 1"] {
            assert!(!observe_only_from(Some(loose)), "느슨한 값 {loose:?} 이 축을 껐다");
        }
        assert_eq!(ENV_OBSERVE_ONLY, "CYS_PROFILE_GATE_OBSERVE_ONLY");
        // 마스터 스위치 하나로 이 축도 종전(관측 전용)으로 돌아간다.
        let master = crate::gate_axes_from(Some("0"), None, None, None, None, None, None);
        assert!(master.profile_gate_observe_only, "★마스터 스위치가 이 축에 닿지 않는다");
        let none = crate::gate_axes_from(None, None, None, None, None, None, None);
        assert!(!none.profile_gate_observe_only, "기본에서 축이 꺼졌다");
        // 축 노브 단독도 자기 축만 끈다(교차 오염 금지).
        let only = crate::gate_axes_from(None, None, None, None, None, None, Some("1"));
        assert!(only.profile_gate_observe_only);
        assert!(!only.readiness_legacy && !only.gate_pending_close, "축 노브가 다른 축을 건드렸다");
        // env 판독은 1지점이다(형제 축과 같은 규율).
        let src = include_str!("profile_gate.rs");
        let body = &src[..src.find("#[cfg(test)]").expect("테스트 앵커 부재")];
        assert_eq!(body.matches("std::env::var(ENV_OBSERVE_ONLY)").count(), 1);
        assert_eq!(body.matches("|| crate::gate_axes_forced_legacy()").count(), 1);
    }

    // ─────────────────────────────────────────────────────────────────────
    // ⑧ 보고 계약
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn report_json_carries_both_axes_and_never_hides_measurement_failure() {
        let dir = std::path::Path::new("/tmp/profile-x");
        let e = ev(parse_config(CFG_SUBSCRIPTION_CLAIM), None);
        let v = classify(&e);
        let j = report_json(dir, &e, &v);
        assert_eq!(j["auth_class"], "unknown");
        assert_eq!(j["allows_spawn"], false);
        assert_eq!(j["evidence_grade"], "config_only");
        assert_eq!(j["reason"], "config_claim_is_not_authentication");
        assert_eq!(j["config"]["state"], "parsed");
        assert_eq!(j["config"]["oauth_account_claim"], "yes");
        assert!(j["oracle"].is_null());
        assert_eq!(j["oracle_attempted"], false);

        let e2 = ev(parse_config(CFG_FRESH), Some(oracle(VG_API_KEY, Some(0))));
        let j2 = report_json(dir, &e2, &classify(&e2));
        assert_eq!(j2["auth_class"], "api_key");
        assert_eq!(j2["allows_spawn"], true);
        assert_eq!(j2["evidence_grade"], "oracle_verified");
        assert_eq!(j2["oracle"]["auth_method"], "api_key");
        assert_eq!(j2["oracle"]["exit_code"], 0);
    }

    #[test]
    fn profile_auth_report_reads_config_without_touching_it() {
        let dir = std::env::temp_dir().join(format!("cys-pg-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".claude.json"), CFG_SUBSCRIPTION_CLAIM).unwrap();
        let before = std::fs::read_to_string(dir.join(".claude.json")).unwrap();
        let (j, v) = profile_auth_report(&dir, Some((VG_SUBSCRIPTION, Some(0))));
        assert_eq!(v.class, AuthClass::Subscription);
        assert!(v.allows_spawn());
        assert_eq!(j["evidence_grade"], "oracle_verified");
        // ★파일 무변경 — 판정기는 사용자 소유 파일을 건드리지 않는다.
        assert_eq!(std::fs::read_to_string(dir.join(".claude.json")).unwrap(), before);
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "판정기가 프로필 dir 에 파일을 만들었다"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
