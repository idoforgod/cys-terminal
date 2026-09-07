//! ★(0.14.31 · WP-4 · 감사 에러 2) 역할 **자동 재결합**(reclaim) — 순수 판정 + phoenix lease 관측.
//!
//! **무엇을 고치는가**: 데몬을 재시작하면 좌석 env(`CYS_ROLE`)를 들고 있던 pane 은 사라지고,
//! 사람이 손으로 띄운 새 pane 은 역할이 없다. 그런데 데몬의 `roles` 맵에는 죽은 에이전트의
//! **빈 셸 좌석**이 그 역할 주소를 그대로 쥐고 있다 — 역할은 '있는데' 그 자리에 아무도 없다.
//! 그 결과 새 pane 은 지침 없이 앉고(치명위험 ③ 바보 좌석), `--to <role>` 라우팅은 빈 셸로
//! 배달되며, 사람은 자기 pane 이 왜 역할이 아닌지 알 길이 없다.
//!
//! **어떻게 고치는가**: 훅이 `cys reclaim-role --auto --config <dir> --cwd <dir>` 로 물으면,
//! 데몬이 **자기가 아는 사실만으로** "이 pane 이 그 빈 좌석의 정당한 후계자인가"를 판정한다.
//! 판정은 이 모듈의 순수 함수([`decide`])가 소유하고, 결합(승계)은 handlers 의 임계영역이
//! **같은 술어를 다시 통과시킨 뒤에만** 커밋한다.
//!
//! **왜 이렇게 좁은가** — 실패 방향이 한쪽이어야 하기 때문이다. 이 장치가 틀리는 방향은
//! 언제나 **"결합하지 않는다 + 안내한다"** 여야 한다. 잘못 결합하면 남의 역할 주소를 빼앗아
//! 라우팅·감시·큐를 끊는다(되돌릴 수 없는 조직 손상). 그래서:
//!   · 후보가 0 이거나 2 이상이면 **아무것도 하지 않는다**(모호함은 결합의 근거가 아니다).
//!   · 호출자의 `CLAUDE_CONFIG_DIR`·`PWD` 를 **인자로 받아** 후보와 대조한다 — 계정·프로젝트가
//!     다른 좌석의 역할을 가져오는 것이 이 장치의 최악 오작동이므로, 그 두 축이 **둘 다 있고
//!     둘 다 같을 때만** 후보가 된다(결측은 값이 아니다 — `None == None` 은 일치가 아니다).
//!   · ★그 두 축은 **자기신고이므로 먼저 인증한다**(R1): 데몬이 호출 좌석에 대해 스스로 아는
//!     `claude_config_dir` 과 그 좌석 셸의 실제 cwd(+생성 cwd)와 대조해, 어긋나면 후보를 아예
//!     고르지 않는다. 그렇지 않으면 무역할 pane 이 남의 계정·남의 프로젝트를 신고해 그 역할을
//!     가져갈 수 있다 — 같은 파일이 자기신고 `surface_id` 를 거절하는 것과 같은 이유다.
//!   · 경로 비교는 **플랫폼 표기까지 접는다**(R1): Windows 에서 데몬은 네이티브 `C:\…` 를
//!     기록하고 Git Bash 훅은 MSYS `/c/…` 를 넘긴다 — 접지 않으면 두 축이 영원히 안 만나
//!     이 수리가 그 플랫폼에 배포되지 않는다. 단 **공백·본문 대소문자는 절대 접지 않는다**
//!     (서로 다른 디렉터리를 같다고 말하는 것이 이 장치의 가장 나쁜 실패다).
//!   · phoenix 가 restore 중이면(lease 보유) 보류한다 — 부활과 재결합이 같은 역할을 두고
//!     경쟁하면 좌석이 두 개가 되거나 서로의 결합을 덮는다.
use std::path::{Path, PathBuf};

/// 새로 만든 좌석에 대한 유예(초). `env_injected=true`(데몬이 스폰한 좌석)는 이 시간 안에는
/// 후보가 되지 않는다 — 방금 `launch-agent` 로 뜬 좌석은 **에이전트가 아직 안 붙었을 뿐**이지
/// 빈 좌석이 아니다. 그 창에서 재결합하면 기동 중인 좌석의 역할을 빼앗는다(정본 §4 WP-4).
pub const NEW_SEAT_GRACE_SECS: f64 = 120.0;

/// `system.topology.live` 한 항목의 재결합 판정 입력(순수 · 락 밖으로 복사해 온 스냅샷).
#[derive(Debug, Clone, PartialEq)]
pub struct LiveEntry {
    pub role: String,
    pub surface_id: u64,
    /// `SeatState::as_str()` 문자열 — `"empty" | "occupied" | "unknown"`.
    /// ★`unknown`(프로브 미도달)은 `empty` 가 아니다. 판정 불가를 빈 좌석으로 접으면
    ///   데몬이 잠깐 못 본 좌석마다 역할이 이사한다.
    pub seat: String,
    pub env_injected: bool,
    pub created_at: f64,
    pub cwd: Option<String>,
    pub claude_config_dir: Option<String>,
    /// 이 좌석이 종료됐는가(exited). 종료 좌석은 후보가 아니다 — 회수(reap)·부활(phoenix)의
    /// 소관이고, 여기서 손대면 두 경로가 같은 역할을 두고 싸운다.
    pub exited: bool,
}

/// 재결합 판정 결과. **`Bind` 를 제외한 전부가 "결합 없음"** 이고, 어느 쪽도 오류가 아니다
/// (CLI 는 항상 exit 0 + `role=` 한 줄 — 계약 C).
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// 발신 pane 을 좌석으로 해석하지 못했다(pane 밖 실행·조상 체인 단절).
    CallerUnresolved,
    /// 호출자가 이미 **데몬 권위** 역할을 갖고 있다 — 결합 없이 그 역할을 돌려준다(멱등).
    AlreadyRoled(String),
    /// `--config`/`--cwd` 가 비었다. 대조 축이 없으면 후보를 고를 수 없다(fail-closed).
    EnvMissing,
    /// 신고된 `--config` 가 **데몬이 그 호출 좌석에 대해 아는 값**과 다르다 — 무결합.
    /// 자기신고 축으로 **남의 계정 dir 좌석**을 가져가는 경로를 닫는다(적대검증 R1 major).
    EnvMismatch,
    /// phoenix restore lease 보유 중(또는 락 기구 불능) — 보류. 다음 세션 시작에 다시 묻는다.
    Defer(&'static str),
    /// 조건을 만족하는 빈 좌석이 없다.
    NoCandidate,
    /// 후보가 2 이상 — **결합하지 않는다**(어느 쪽을 골라도 절반은 틀린다).
    Ambiguous(Vec<String>),
    /// 유일 후보 확정. `from_surface` 의 역할을 호출 좌석으로 승계한다.
    Bind { role: String, from_surface: u64 },
}

impl Decision {
    /// CLI·훅이 소비하는 사유 코드(안정 문자열 — 로그·안내문 분기용).
    pub fn reason(&self) -> &'static str {
        match self {
            Decision::CallerUnresolved => "caller_unresolved",
            Decision::AlreadyRoled(_) => "already_roled",
            Decision::EnvMissing => "caller_env_missing",
            Decision::EnvMismatch => "caller_env_mismatch",
            Decision::Defer(r) => r,
            Decision::NoCandidate => "no_candidate",
            Decision::Ambiguous(_) => "ambiguous",
            Decision::Bind { .. } => "bind",
        }
    }
}

/// 경로 비교 정규화 — **순수**(파일시스템 접근 0). 플랫폼 의미론을 인자로 받는다
/// (`windows=false` = POSIX · `true` = Windows). 실행 플랫폼용 래퍼는 [`norm_path`].
///
/// ## unix (`windows=false`) — **공백을 지우지 않는다**
/// `/tmp/proj ` 와 `/tmp/proj` 는 **서로 다른 디렉터리**다(POSIX 파일명에 공백은 합법). 종전 판은
/// `trim()` 으로 그 둘을 같다고 말했다 — 정규화가 아니라 **경로 정체성 변조**이고, 이 장치가
/// 절대 하면 안 되는 방향(다른 디렉터리를 같다고 말한다)의 오작동이다(codex 적대검증 R1 major).
/// 그래서 여기서는 **후행 `/` 제거 하나만** 한다. 빈·공백뿐인 입력의 거부는 [`same_path`] 소관이다
/// (거기서는 `trim` 을 '비어 있는가' 판정에만 쓰고 비교값은 원문 그대로 넘긴다).
///
/// ## windows (`windows=true`) — **한 표기로 접는다**
/// 같은 디렉터리가 최소 세 표기로 나타나기 때문이다: 데몬이 기록하는 네이티브
/// `C:\Users\x\proj`(`dirs::home_dir()`·`resolve_claude_config_dir` 파생) · Git Bash 훅의 MSYS
/// `/c/Users/x/proj`(`$PWD`) · cygpath 변환본 `C:/Users/x/proj`. 종전 판은 공백과 후행 `/` 만
/// 다듬어 **세 표기가 영원히 안 만났고**, 그래서 Windows 의 모든 호출이 `no_candidate` 였다 —
/// WP-4 수리가 그 플랫폼에 배포되지 않는다는 뜻이다(두 리뷰어 공통 major).
/// 접는 것은 **표기 차이뿐**이고, 접을 때마다 "다른 디렉터리를 같다고 말할" 위험을 먼저 본다:
///   ① 확장 길이 접두 `\\?\` · `\\?\UNC\` 제거 — 같은 대상의 다른 이름이다.
///   ② `\` → `/` — Windows 는 둘을 같은 구분자로 받는다.
///   ③ MSYS 드라이브 `/c/…` → `c:/…` · `/c` → `c:/`(드라이브 **루트**).
///   ④ **드라이브 문자만** 소문자화한다. ★경로 본문은 건드리지 않는다 — NTFS 는 디렉터리 단위로
///      대소문자 구분을 켤 수 있어(WSL·`fsutil file setCaseSensitiveInfo`) `C:\work\A` 와
///      `C:\work\a` 가 **동시에 존재**할 수 있고, 전체 소문자화는 그 둘을 같다고 말한다
///      (codex 적대검증 R1 blocking). 실제로 갈리는 축은 드라이브 문자 하나뿐이다 —
///      MSYS 는 소문자(`/c/…`), 네이티브·cygpath 는 대문자(`C:\…`)로 쓴다.
///   ⑤ 후행 `/` 제거. ★드라이브 **루트**(`c:/`)는 남긴다 — `c:`(드라이브 상대 경로: 그 드라이브의
///      *현재* 디렉터리를 뜻하며 `C:\` 와 다른 위치일 수 있다)와 같아지면 안 된다
///      (codex 적대검증 R1 blocking). UNC 루트(`//srv/share`)도 같은 이유로 보존한다.
/// ★UNC(`\\server\share`)는 `//server/share` 로 남긴다 — 드라이브 경로로 접으면 서로 다른
///   호스트를 같다고 말할 수 있다. 그리고 **공백은 여기서도 지우지 않는다**(같은 이유).
pub fn norm_path_on(p: &str, windows: bool) -> String {
    if !windows {
        let stripped = p.trim_end_matches('/');
        // "/" 자체(전부 슬래시)는 루트다 — 빈 문자열로 접지 않는다.
        return if stripped.is_empty() { p.to_string() } else { stripped.to_string() };
    }
    // ① 확장 길이 접두
    let mut t = p;
    if let Some(rest) = t.strip_prefix(r"\\?\UNC\").or_else(|| t.strip_prefix(r"\\?\unc\")) {
        // `\\?\UNC\server\share` 는 `\\server\share` 와 같은 대상이다.
        let joined = format!(r"\\{rest}");
        return norm_path_on(&joined, true);
    }
    if let Some(rest) = t.strip_prefix(r"\\?\") {
        t = rest;
    }
    // ② 구분자 통일
    let mut s: String = t.replace('\\', "/");
    // ③ MSYS 드라이브 표기 → 드라이브 표기. `/c/…`·`/c` 만 해당(두 글자 이상은 디렉터리다:
    //    `/cy/proj` 는 드라이브가 아니라 루트 밑 디렉터리다).
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'/' && (b[1] as char).is_ascii_alphabetic() {
        if b.len() == 2 {
            s = format!("{}:/", b[1] as char); // `/c` = C 드라이브 **루트**
        } else if b[2] == b'/' {
            s = format!("{}:{}", b[1] as char, &s[2..]);
        }
    }
    // ④ 드라이브 문자만 소문자화(본문 보존 — 위 doc ④ 참조)
    let bb = s.as_bytes();
    if bb.len() >= 2 && (bb[0] as char).is_ascii_alphabetic() && bb[1] == b':' {
        let mut out = String::with_capacity(s.len());
        out.push((bb[0] as char).to_ascii_lowercase());
        out.push_str(&s[1..]);
        s = out;
    }
    // ⑤ 후행 구분자 — **루트 아래로는 깎지 않는다**. 루트의 길이를 먼저 정하고 그 위에서만
    //    깎는다(종전처럼 "직전 글자가 `/` 면 멈춤"으로 쓰면 `C:////` 가 `c:////` 로 남아
    //    `C:\` 와 안 만난다 — codex 위임 검체가 잡았다).
    let bytes = s.as_bytes();
    let drive_prefixed =
        bytes.len() >= 2 && (bytes[0] as char).is_ascii_alphabetic() && bytes[1] == b':';
    let root_len = if drive_prefixed {
        if bytes.len() >= 3 && bytes[2] == b'/' { 3 } else { 2 } // `c:/`(루트) vs `c:`(드라이브 상대)
    } else if s.starts_with("//") {
        2 // UNC 접두 `//` 는 남긴다(드라이브 경로로 접히면 다른 호스트를 같다고 말한다)
    } else {
        1 // POSIX 루트 `/`
    };
    while s.len() > root_len && s.ends_with('/') {
        s.truncate(s.len() - 1);
    }
    s
}

/// 실행 플랫폼 의미론의 [`norm_path_on`]. 판정은 데몬이 도는 그 기계의 규칙을 따른다.
pub fn norm_path(p: &str) -> String {
    norm_path_on(p, cfg!(windows))
}

/// 두 경로가 **둘 다 있고** 같은가. ★결측은 값이 아니다 — `None == None` 은 일치가 아니다.
/// `trim()` 은 **'비어 있는가' 판정에만** 쓴다 — 비교값은 원문 그대로 [`norm_path`] 로 간다
/// (공백을 지우면 서로 다른 디렉터리가 같아진다).
fn same_path(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) if !x.trim().is_empty() && !y.trim().is_empty() => {
            norm_path(x) == norm_path(y)
        }
        _ => false,
    }
}

/// 이 항목이 재결합 후보인가 — **순수 술어**(판정 이원화 금지: 핸들러의 락 안 재검증도 이 함수를 쓴다).
pub fn is_candidate(
    e: &LiveEntry,
    caller_sid: u64,
    caller_cfg: &str,
    caller_cwd: &str,
    now: f64,
) -> bool {
    if e.surface_id == caller_sid || e.exited || e.role.trim().is_empty() {
        return false;
    }
    // ★예약 식별 등급은 좌석이 자칭할 수 없고(claim_role 게이트) 자동 승계 대상도 아니다.
    if e.role == "owner" || e.role == "creator" {
        return false;
    }
    if e.seat != "empty" {
        return false; // occupied·unknown 둘 다 거부(판정 불가는 빈 좌석이 아니다)
    }
    // 데몬이 env 를 실어 스폰한 좌석은 **유예 뒤에만** 후보다(기동 중 좌석 탈취 차단).
    if e.env_injected && now - e.created_at < NEW_SEAT_GRACE_SECS {
        return false;
    }
    same_path(e.cwd.as_deref(), Some(caller_cwd))
        && same_path(e.claude_config_dir.as_deref(), Some(caller_cfg))
}

/// 재결합 판정 — 순수. 부작용 0 · 시각과 lease 보유 여부까지 **주입**받는다(테스트 결정론).
#[allow(clippy::too_many_arguments)]
pub fn decide(
    caller_sid: Option<u64>,
    caller_role: Option<&str>,
    // ★데몬이 **호출 좌석에 대해 스스로 아는** 두 축. `caller_known_cfg` 는 surface.create 가
    //   해소해 기록한 `claude_config_dir`, `caller_known_cwds` 는 그 좌석에 대해 데몬이 아는
    //   **디렉터리들**(좌석 셸 pid 의 실제 cwd — sysinfo · cd 추적 — 과 좌석 생성 cwd)이다.
    //   신고값이 이것들과 어긋나면 결합하지 않는다 — 아래 게이트 참조.
    caller_known_cfg: Option<&str>,
    caller_known_cwds: &[String],
    caller_cfg: Option<&str>,
    caller_cwd: Option<&str>,
    now: f64,
    lease: LeaseState,
    entries: &[LiveEntry],
) -> Decision {
    let Some(caller_sid) = caller_sid else {
        return Decision::CallerUnresolved;
    };
    // 이미 역할이 있으면 결합하지 않는다(멱등). ★이 값은 호출자가 **데몬 권위로 대조한**
    //   역할이어야 한다(handlers: `surface.role` ∧ `roles[role] == caller_sid`).
    if let Some(r) = caller_role.filter(|r| !r.trim().is_empty()) {
        return Decision::AlreadyRoled(r.to_string());
    }
    let (Some(cfg), Some(cwd)) = (
        caller_cfg.filter(|v| !v.trim().is_empty()),
        caller_cwd.filter(|v| !v.trim().is_empty()),
    ) else {
        return Decision::EnvMissing;
    };
    // ★신고 축의 **인증**(codex·claude 공통 R1 major): 종전에는 `--config`·`--cwd` 가 전부
    //   호출자 자기신고였고, 데몬은 그 문자열을 **후보와만** 대조했다. 그래서 무역할 pane 이
    //   `--config <남의 계정 dir> --cwd <그 좌석 cwd>` 로 남의 역할을 가져갈 수 있었다 — 이 모듈이
    //   서두에서 "이 장치의 최악 오작동"이라 부른 바로 그 경로이고, 같은 파일이 자기신고
    //   `surface_id` 를 '위조 가능'하다며 거절하는 것과도 모순이었다.
    //   데몬은 호출 좌석의 `claude_config_dir` 을 **스스로 해소해 기록**해 둔다(surface.create).
    //   그 값과 신고값이 다르면 무결합이다(결측도 불일치 — `same_path` 계약).
    //   ★cwd 도 같이 인증한다(codex 적대검증 R1 blocking). 처음엔 "계정 dir 만 고정하면 남는
    //     노출은 `cys claim-role <role>` 로 이미 가능한 행위"라고 적었는데 **그 논거가 틀렸다**:
    //     `claim_role` 은 worker 계열 이름 충돌 시 **dedup**(worker-2 → worker-3)을 하므로
    //     "특정 좌석의 역할·caps·큐를 그 자리째 가져오는" 이 경로와 동등하지 않다. 그래서 같은
    //     계정 안 **다른 프로젝트**의 빈 좌석을 자기신고로 가져가는 문이 실제로 남아 있었다.
    //     대조 집합이 **둘**인 이유: 좌석 셸의 **지금 cwd**(OS 가 답한 사실 · cd 추적)와 좌석
    //     **생성 cwd** 는 둘 다 데몬이 아는 사실이라 공격자가 고를 수 없고, 사람이 pane 을 띄운
    //     뒤 프로젝트로 `cd` 했는지에 따라 훅의 `$PWD` 는 그중 하나다 — 인증 강도는 같으면서
    //     정상 시나리오를 죽이지 않는다(프로세스 cwd 를 못 읽는 기계에서는 생성 cwd 하나만 남고,
    //     그때 어긋나면 무결합이다 — 안전 방향).
    let cwd_ok = caller_known_cwds.iter().any(|k| same_path(Some(k), Some(cwd)));
    if !same_path(caller_known_cfg, Some(cfg)) || !cwd_ok {
        return Decision::EnvMismatch;
    }
    // ★lease 는 후보 계산 **앞**이다: 부활이 도는 중이면 무엇을 고르든 그 결정은 낡았다.
    match lease {
        LeaseState::Held => return Decision::Defer("restore_lease_held"),
        LeaseState::Unavailable => return Decision::Defer("restore_lease_unavailable"),
        LeaseState::Free => {}
    }
    let mut hits: Vec<&LiveEntry> = entries
        .iter()
        .filter(|e| is_candidate(e, caller_sid, cfg, cwd, now))
        .collect();
    match hits.len() {
        0 => Decision::NoCandidate,
        1 => Decision::Bind {
            role: hits[0].role.clone(),
            from_surface: hits[0].surface_id,
        },
        _ => {
            hits.sort_by(|a, b| a.role.cmp(&b.role));
            Decision::Ambiguous(hits.iter().map(|e| e.role.clone()).collect())
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// phoenix restore lease — **관측이 아니라 보유**한다
// ─────────────────────────────────────────────────────────────────────────────

/// lease 관측 결과. `Unavailable`(열기·락 기구 실패)은 **`Free` 로 강등하지 않는다** —
/// phoenix 는 가용성을 위해 fail-open 하지만, 여기서 fail-open 하면 "조정 상태를 모르는 채
/// 역할을 옮긴다"가 된다. 역할 변경 허가는 모를 때 **거절**한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState {
    Free,
    Held,
    Unavailable,
}

/// 획득한 lease 를 **커밋까지 붙잡는** RAII 가드. drop 에서 해제(unix 는 close 로 flock 자동
/// 해제 · Windows 는 명시 UnlockFile 후 close).
///
/// ★왜 "관측 후 즉시 해제"가 아닌가(codex 적대검증 blocking): 관측만 하고 놓으면 그 찰나에
///   phoenix 가 lease 를 얻어 restore 를 시작하고, 우리는 그 사실을 모른 채 결합을 커밋한다
///   (부활과 재결합이 같은 역할에 동시에 손을 댄다). 판정과 커밋 사이를 lease 로 잇는다.
pub struct LeaseGuard {
    file: std::fs::File,
    #[allow(dead_code)]
    path: PathBuf,
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            unsafe {
                libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                windows_sys::Win32::Storage::FileSystem::UnlockFile(
                    self.file.as_raw_handle() as _,
                    0,
                    0,
                    1,
                    0,
                );
            }
        }
        // file 은 여기서 drop 되며 닫힌다(unix flock 은 close 로도 해제된다 — 이중 안전).
    }
}

/// phoenix 의 restore lease 경로 — `javis_phoenix.py:_acquire_restore_lease` 와 **같은 파일**
/// (`<state_dir>/phoenix/restore.lease`). 경로가 갈리면 두 프로세스가 서로를 못 본다.
///
/// ★대칭 요건(적대검증 R1 minor): phoenix 쪽은 `state_dir_for(socket) =
/// os.path.realpath(os.path.dirname(socket))` 로 **심링크를 해소한 뒤** 이 경로를 만든다.
/// 이쪽 `state_dir` 은 `socket_path.parent()`(무정규화)라, 소켓 디렉터리 경로에 심링크가 끼면
/// 두 프로세스가 **서로 다른 파일**을 잠그고 배타가 조용히 사라진다(모를 때 거절한다는 이 모듈의
/// 원칙이 무음 fail-open 으로 뒤집힌다). 그래서 [`canonical_state_dir`] 로 같은 해소를 먼저 한다.
pub fn restore_lease_path(state_dir: &Path) -> PathBuf {
    canonical_state_dir(state_dir).join("phoenix").join("restore.lease")
}

/// `state_dir` 의 심링크 해소 — phoenix 의 `os.path.realpath` 와 같은 의미. 해소 실패(경로 부재
/// 등)는 **원본 그대로**다: 없는 디렉터리는 아래 `create_dir_all` 이 만들고, 그때는 심링크도
/// 없으므로 두 쪽의 결과가 같다. Windows 의 `canonicalize` 는 확장 길이 접두를 붙이지만
/// 그 경로도 같은 파일을 가리키므로 락 대상은 동일하다(문자열 비교에 쓰지 않는다).
fn canonical_state_dir(state_dir: &Path) -> PathBuf {
    std::fs::canonicalize(state_dir).unwrap_or_else(|_| state_dir.to_path_buf())
}

/// 비차단 배타 락 **획득 시도**. 성공하면 가드를 돌려주고(호출자가 커밋까지 보유),
/// 다른 보유자가 있으면 `Held`, 열기·락 기구 실패는 `Unavailable`(= 보류).
///
/// 락 영역은 phoenix 와 정확히 같아야 한다: unix `flock`(파일 전체) · Windows 는
/// `msvcrt.locking(LK_NBLCK, 1)` 이 seek(0) 뒤 **byte 0 한 바이트**를 잠그므로 같은 범위를 쓴다.
pub fn try_hold_restore_lease(state_dir: &Path) -> (LeaseState, Option<LeaseGuard>) {
    let path = restore_lease_path(state_dir);
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return (LeaseState::Unavailable, None);
        }
    }
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false) // ★phoenix 가 `open(path,"a+")` 로 여는 파일이다 — 절대 자르지 않는다
        .open(&path)
    {
        Ok(f) => f,
        Err(_) => return (LeaseState::Unavailable, None),
    };
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc == 0 {
            return (LeaseState::Free, Some(LeaseGuard { file, path }));
        }
        let err = std::io::Error::last_os_error();
        // EWOULDBLOCK(=EAGAIN) 만 '다른 보유자'다. 그 밖의 오류는 판정 불가 → 보류.
        return match err.raw_os_error() {
            Some(e) if e == libc::EWOULDBLOCK || e == libc::EAGAIN => (LeaseState::Held, None),
            _ => (LeaseState::Unavailable, None),
        };
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
        };
        let mut ov: windows_sys::Win32::System::IO::OVERLAPPED = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            LockFileEx(
                file.as_raw_handle() as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut ov,
            )
        };
        if ok != 0 {
            return (LeaseState::Free, Some(LeaseGuard { file, path }));
        }
        // 잠긴 것(ERROR_LOCK_VIOLATION=33)만 Held · 그 밖은 판정 불가 → 보류.
        return match std::io::Error::last_os_error().raw_os_error() {
            Some(33) => (LeaseState::Held, None),
            _ => (LeaseState::Unavailable, None),
        };
    }
    #[cfg(not(any(unix, windows)))]
    {
        drop(file);
        (LeaseState::Unavailable, None)
    }
}

/// ★재결합 커밋의 **테스트 이음매** — 락 밖 프로브와 임계영역 사이에서 한 번 불린다.
///
/// `raced_seat_refilled` 는 정의상 그 창에서만 성립한다(상태를 미리 세팅하면 후보 술어가 먼저
/// 걸려 다른 사유가 난다). 그래서 그 분기를 실측하려면 그 지점에 경합을 주입할 수 있어야 하고,
/// '실측되지 않은 방어'는 있다고 말할 수 없다.
///
/// ★왜 이 함수가 `handlers.rs` 가 아니라 여기 있는가: `handlers.rs` 에는 자기 소스를 읽어
/// **테스트 cfg 속성의 첫 출현**을 앵커로 프로덕션 구간을 자르는 소스핀이 여럿 있다
/// (`lease_gate_precedes_any_side_effect_in_both_arms` ·
/// `manual_reap_recheck_pins_state_changed_abort` · `queue_lock_order_contract_no_ab_ba` ·
/// `the_nonce_is_armed_at_exactly_one_place` · boot_supervisor 의 배선 핀). 그 속성이 arm 들보다
/// **앞에** 한 번이라도 나오면 슬라이스가 거기서 잘려 그 핀들이 통째로 무력화된다 —
/// 그 파일의 주석이 "주석에도 그 문자열을 적지 마라"라고 명시한 계약이고, 실제로 이 이음매를
/// 거기 두었을 때 핀 5건이 적색이었다(실측 2026-09-08). 그래서 조건부 정의는 이 파일에 두고,
/// `handlers.rs` 에는 **무조건 호출** 한 줄만 남긴다(릴리스 빌드에서는 빈 함수 · 인라인).
#[cfg(not(test))]
#[inline(always)]
pub fn race_seam() {}

#[cfg(test)]
pub fn race_seam() {
    tests::RECLAIM_RACE_HOOK.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f();
        }
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    thread_local! {
        /// `reclaim_commit` 의 락 밖 프로브와 임계영역 **사이**에 실행되는 테스트 훅
        /// (`handlers.rs` 의 검체가 `set_race_hook` 으로 심는다).
        pub(super) static RECLAIM_RACE_HOOK: std::cell::RefCell<Option<Box<dyn Fn()>>> =
            const { std::cell::RefCell::new(None) };
    }

    /// 훅을 심고/지운다 — `handlers.rs` 의 경합 검체 전용.
    pub(crate) fn set_race_hook(f: Option<Box<dyn Fn()>>) {
        RECLAIM_RACE_HOOK.with(|h| *h.borrow_mut() = f);
    }

    fn ent(role: &str, sid: u64, seat: &str, cfg: &str, cwd: &str) -> LiveEntry {
        LiveEntry {
            role: role.to_string(),
            surface_id: sid,
            seat: seat.to_string(),
            env_injected: false,
            created_at: 0.0,
            cwd: Some(cwd.to_string()),
            claude_config_dir: Some(cfg.to_string()),
            exited: false,
        }
    }

    const CFG: &str = "/Users/cys/.cys/claude";
    const CWD: &str = "/Users/cys/dev/proj";
    /// 데몬이 호출 좌석에 대해 아는 디렉터리 집합(실제 프로세스 cwd + 좌석 생성 cwd 대역).
    fn known_cwds() -> Vec<String> {
        vec![CWD.to_string()]
    }

    fn decide_one(entries: &[LiveEntry]) -> Decision {
        decide(Some(9), None, Some(CFG), &known_cwds(), Some(CFG), Some(CWD), 10_000.0, LeaseState::Free, entries)
    }

    /// 후보 1 → 결합.
    #[test]
    fn single_candidate_binds() {
        let e = [ent("worker-2", 3, "empty", CFG, CWD)];
        assert_eq!(
            decide_one(&e),
            Decision::Bind { role: "worker-2".into(), from_surface: 3 }
        );
    }

    /// 후보 0 → 무결합. occupied·unknown 좌석은 후보가 아니다(판정 불가는 빈 좌석이 아니다).
    #[test]
    fn zero_candidates_and_unknown_seat_is_not_empty() {
        assert_eq!(decide_one(&[]), Decision::NoCandidate);
        assert_eq!(decide_one(&[ent("cso", 3, "occupied", CFG, CWD)]), Decision::NoCandidate);
        assert_eq!(
            decide_one(&[ent("cso", 3, "unknown", CFG, CWD)]),
            Decision::NoCandidate,
            "seat=unknown(프로브 미도달)을 빈 좌석으로 접었다"
        );
    }

    /// 후보 2+ → **무결합**(모호함은 결합의 근거가 아니다).
    #[test]
    fn two_candidates_bind_nothing() {
        let e = [
            ent("worker-2", 3, "empty", CFG, CWD),
            ent("reviewer-codex", 4, "empty", CFG, CWD),
        ];
        match decide_one(&e) {
            Decision::Ambiguous(v) => assert_eq!(v, vec!["reviewer-codex", "worker-2"]),
            other => panic!("후보 2인데 {other:?}"),
        }
    }

    /// `env_injected=true` 인 갓 스폰된 좌석은 유예(120s) 안에는 후보가 아니다 —
    /// 기동 중(에이전트 미부착) 좌석의 역할을 빼앗는 경로를 닫는다. 유예 후에는 후보다.
    #[test]
    fn env_injected_new_seat_excluded_until_grace() {
        let mut e = ent("cso", 3, "empty", CFG, CWD);
        e.env_injected = true;
        e.created_at = 10_000.0 - (NEW_SEAT_GRACE_SECS - 1.0);
        assert_eq!(decide_one(&[e.clone()]), Decision::NoCandidate, "유예 안 좌석이 후보가 됐다");
        e.created_at = 10_000.0 - (NEW_SEAT_GRACE_SECS + 1.0);
        assert_eq!(
            decide_one(&[e]),
            Decision::Bind { role: "cso".into(), from_surface: 3 },
            "유예를 넘긴 좌석이 후보에서 빠졌다"
        );
    }

    /// ★lease 보유 중이면 후보가 있어도 보류. 락 기구 불능도 **보류**(fail-open 금지).
    #[test]
    fn lease_defers_even_with_a_perfect_candidate() {
        let e = [ent("worker-2", 3, "empty", CFG, CWD)];
        for (st, want) in [
            (LeaseState::Held, "restore_lease_held"),
            (LeaseState::Unavailable, "restore_lease_unavailable"),
        ] {
            let d = decide(Some(9), None, Some(CFG), &known_cwds(), Some(CFG), Some(CWD), 10_000.0, st, &e);
            assert_eq!(d, Decision::Defer(want), "lease {st:?} 인데 보류가 아니다");
        }
    }

    /// ★타 부서 좌석(같은 숫자 surface id 라도) — 계정 dir 이 다르면 후보가 아니다.
    /// 부서 데몬끼리 surface id 공간이 겹치므로 id 일치는 아무 것도 증명하지 않는다.
    #[test]
    fn other_department_config_dir_is_not_a_candidate() {
        let other = ent("cso", 3, "empty", "/Users/cys/.cys/claude-default-dept-2", CWD);
        assert_eq!(decide_one(&[other]), Decision::NoCandidate);
        // cwd 만 다른 경우도 마찬가지(프로젝트가 다르면 남의 역할이다).
        let elsewhere = ent("cso", 3, "empty", CFG, "/Users/cys/dev/other");
        assert_eq!(decide_one(&[elsewhere]), Decision::NoCandidate);
    }

    /// ★음성 대조 — **결측은 값이 아니다**: 후보의 cwd·config 가 없고 호출자도 없을 때
    /// `None == None` 으로 일치 판정되면 아무 좌석이나 재결합된다.
    #[test]
    fn missing_is_not_a_match() {
        let mut e = ent("cso", 3, "empty", CFG, CWD);
        e.cwd = None;
        e.claude_config_dir = None;
        // 호출자 축도 없는 경우 → EnvMissing(후보 계산 자체를 하지 않는다)
        assert_eq!(
            decide(Some(9), None, Some(CFG), &known_cwds(), None, None, 10_000.0, LeaseState::Free, &[e.clone()]),
            Decision::EnvMissing
        );
        assert_eq!(
            decide(Some(9), None, Some(CFG), &known_cwds(), Some(""), Some("   "), 10_000.0, LeaseState::Free, &[e.clone()]),
            Decision::EnvMissing,
            "빈 문자열 인자가 '지정됨'으로 취급됐다"
        );
        // 호출자 축이 있어도 후보의 결측은 일치가 아니다
        assert_eq!(decide_one(&[e]), Decision::NoCandidate);
    }

    /// 이미 역할이 있으면 결합 없이 그 역할을 돌려준다(멱등) — 후보가 있어도 손대지 않는다.
    #[test]
    fn already_roled_returns_role_without_binding() {
        let e = [ent("worker-2", 3, "empty", CFG, CWD)];
        assert_eq!(
            decide(Some(9), Some("cso"), Some(CFG), &known_cwds(), Some(CFG), Some(CWD), 10_000.0, LeaseState::Free, &e),
            Decision::AlreadyRoled("cso".into())
        );
    }

    /// 발신 좌석 미해석 → 무결합(자기신고 surface_id 를 받지 않으므로 유일한 신원 경로다).
    #[test]
    fn caller_unresolved_binds_nothing() {
        let e = [ent("worker-2", 3, "empty", CFG, CWD)];
        assert_eq!(
            decide(None, None, Some(CFG), &known_cwds(), Some(CFG), Some(CWD), 10_000.0, LeaseState::Free, &e),
            Decision::CallerUnresolved
        );
    }

    /// 자기 좌석·종료 좌석·빈 역할명은 후보가 아니다.
    #[test]
    fn self_exited_and_empty_role_excluded() {
        let mut mine = ent("cso", 9, "empty", CFG, CWD); // caller_sid = 9
        assert_eq!(decide_one(&[mine.clone()]), Decision::NoCandidate, "자기 좌석이 후보가 됐다");
        mine.surface_id = 3;
        let mut dead = mine.clone();
        dead.exited = true;
        assert_eq!(decide_one(&[dead]), Decision::NoCandidate, "종료 좌석이 후보가 됐다");
        let mut blank = mine.clone();
        blank.role = "  ".into();
        assert_eq!(decide_one(&[blank]), Decision::NoCandidate);
        // 예약 등급은 자동 승계 대상이 아니다
        for reserved in ["owner", "creator"] {
            let mut r = mine.clone();
            r.role = reserved.into();
            assert_eq!(decide_one(&[r]), Decision::NoCandidate, "{reserved} 가 후보가 됐다");
        }
    }

    /// unix 정규화는 **후행 슬래시까지만**이고 그 이상 넓히지 않는다(다른 디렉터리를 같다고
    /// 말할 위험). ★공백은 지우지 않는다 — `/tmp/proj ` 와 `/tmp/proj` 는 다른 디렉터리다.
    #[test]
    fn path_normalization_is_narrow_on_unix() {
        assert_eq!(norm_path_on("/a/b/", false), "/a/b");
        assert_eq!(norm_path_on("/a/b///", false), "/a/b");
        assert_eq!(norm_path_on("/", false), "/");
        assert_eq!(norm_path_on("///", false), "///", "루트류를 빈 문자열로 접었다");
        assert!(same_path(Some("/a/b/"), Some("/a/b")));
        assert!(!same_path(Some("/a/b"), Some("/a/B")), "대소문자 관용 금지");
        assert!(!same_path(Some("~/dev/x"), Some("/Users/cys/dev/x")), "tilde 확장 금지(순수)");
        assert!(!same_path(None, None), "결측끼리 일치");
        assert!(!same_path(Some(""), Some("")), "빈 문자열끼리 일치");
        assert!(!same_path(Some(" "), Some("  ")), "공백뿐인 값끼리 일치");
    }

    /// ★음성 대조(codex 적대검증 R1 major): **후행 공백만 다른 두 디렉터리는 같지 않다.**
    /// POSIX 파일명에 공백은 합법이라 `/tmp/proj ` 와 `/tmp/proj` 는 서로 다른 실재 디렉터리다.
    /// 종전 `trim()` 정규화는 그 둘을 같다고 말했다 — 정규화가 아니라 경로 정체성 변조이고,
    /// 그 한 줄로 **다른 프로젝트의 빈 좌석**이 후보가 된다.
    #[test]
    fn trailing_whitespace_paths_are_distinct_directories() {
        assert_eq!(norm_path_on("/tmp/proj ", false), "/tmp/proj ", "공백이 지워졌다");
        assert!(!same_path(Some("/tmp/proj "), Some("/tmp/proj")), "공백만 다른 경로가 일치");
        assert!(!same_path(Some("/tmp/proj"), Some(" /tmp/proj")), "선행 공백이 지워졌다");
        // 후보 술어까지 관통시킨다(문자열 함수만 고치고 술어를 놓치는 회귀 차단).
        let e = LiveEntry { cwd: Some("/tmp/proj ".into()), ..ent("worker-2", 3, "empty", CFG, CWD) };
        assert!(
            !is_candidate(&e, 9, CFG, "/tmp/proj", 10_000.0),
            "후행 공백만 다른 cwd 좌석이 후보가 됐다"
        );
    }

    /// ★Windows 표기 수렴(두 리뷰어 공통 major): 같은 디렉터리의 네이티브·MSYS·cygpath 표기가
    /// **한 값으로 접혀야** 한다. 접히지 않으면 Windows 의 모든 호출이 `no_candidate` 가 되어
    /// WP-4 수리가 그 플랫폼에 배포되지 않는다. macOS 에서도 재도록 플랫폼을 인자로 받는다.
    #[test]
    fn windows_path_spellings_converge() {
        let want = "c:/Users/x/proj";
        for spelling in [
            r"C:\Users\x\proj",     // 데몬이 기록하는 네이티브
            r"C:\Users\x\proj\",    // 후행 구분자
            "/c/Users/x/proj",      // Git Bash `$PWD` (MSYS · 드라이브만 소문자)
            "C:/Users/x/proj",      // cygpath -m / 혼합
            r"\\?\C:\Users\x\proj", // 확장 길이 접두
        ] {
            assert_eq!(norm_path_on(spelling, true), want, "표기 미수렴: {spelling}");
        }
        // MSYS `/c` 는 드라이브 **루트**다 — `C:\` 와 같은 곳이어야 한다.
        assert_eq!(norm_path_on(r"C:\", true), "c:/");
        assert_eq!(norm_path_on("/c", true), "c:/");
        assert_eq!(norm_path_on(r"C:\", true), norm_path_on("/c", true));
        // UNC 는 드라이브로 접지 않는다 — 서로 다른 호스트를 같다고 말하지 않기 위해서다.
        assert_eq!(norm_path_on(r"\\srv\share\p", true), "//srv/share/p");
        assert_eq!(norm_path_on(r"\\?\UNC\srv\share\p", true), "//srv/share/p");
        assert_ne!(
            norm_path_on(r"\\srv\share\p", true),
            norm_path_on(r"\\other\share\p", true),
            "다른 호스트의 UNC 가 같다고 접혔다"
        );
    }

    /// ★음성 대조 2건(codex 적대검증 R1 blocking 2종) — 접기가 **서로 다른 위치**를 같다고
    /// 말하지 않는다.
    ///  ⓐ 드라이브 **루트**(`C:\`)와 드라이브 **상대**(`C:`)는 다르다. 후자는 그 드라이브의
    ///    *현재* 디렉터리(예: `C:\work`)를 뜻하므로, 후행 슬래시를 무조건 깎으면 전혀 다른
    ///    디렉터리의 좌석이 유일 후보가 된다.
    ///  ⓑ 경로 본문의 대소문자는 접지 않는다. NTFS 는 디렉터리 단위로 대소문자 구분을 켤 수 있어
    ///    `C:\work\A` 와 `C:\work\a` 가 **동시에 존재**할 수 있다.
    #[test]
    fn windows_normalization_keeps_distinct_locations_distinct() {
        assert_eq!(norm_path_on("C:", true), "c:", "드라이브 상대 표기가 변형됐다");
        assert_ne!(
            norm_path_on(r"C:\", true),
            norm_path_on("C:", true),
            "드라이브 루트와 드라이브 상대경로가 같다고 접혔다"
        );
        assert_ne!(
            norm_path_on(r"C:\work\A", true),
            norm_path_on(r"C:\work\a", true),
            "대소문자만 다른 두 디렉터리가 같다고 접혔다(대소문자 구분 켜진 NTFS 디렉터리)"
        );
        // 드라이브 문자 자체의 대소문자는 접는다(MSYS 소문자 ↔ 네이티브 대문자).
        assert_eq!(norm_path_on(r"C:\work", true), norm_path_on(r"c:\work", true));
    }

    /// ★음성 대조: Windows 정규화가 **서로 다른 디렉터리를 같다고 말하지 않는다**.
    /// (드라이브가 다르다 · MSYS 두 글자 디렉터리는 드라이브가 아니다 · 공백은 남는다)
    #[test]
    fn windows_normalization_does_not_merge_distinct_dirs() {
        assert_ne!(norm_path_on(r"C:\p", true), norm_path_on(r"D:\p", true), "드라이브 무시");
        // `/cy/...` 는 MSYS 드라이브 표기가 아니다(드라이브는 한 글자) — 접으면 안 된다.
        assert_eq!(norm_path_on("/cy/proj", true), "/cy/proj");
        assert_ne!(norm_path_on("/c/proj", true), norm_path_on("/cy/proj", true));
        assert_ne!(norm_path_on(r"C:\p ", true), norm_path_on(r"C:\p", true), "공백이 지워졌다");
        assert_ne!(
            norm_path_on(r"\\srv\share", true),
            norm_path_on(r"C:\srv\share", true),
            "UNC 가 드라이브 경로로 접혔다"
        );
    }

    /// ★신고 축 인증(적대검증 R1 major·blocking): 데몬이 호출 좌석에 대해 **스스로 아는**
    /// `claude_config_dir`·실제 cwd 와 신고된 `--config`·`--cwd` 가 다르면 **결합하지 않는다**.
    /// 이것이 없으면 무역할 pane 이 남의 계정 dir·남의 프로젝트 좌석을 신고해 그 역할을
    /// 가져갈 수 있다(`claim-role` 은 worker 이름을 dedup 하므로 동등한 행위가 아니다).
    #[test]
    fn reported_axes_must_match_the_seat_the_daemon_knows() {
        const OTHER_CFG: &str = "/Users/cys/.cys/claude-dept-3";
        const OTHER_CWD: &str = "/Users/cys/dev/other-proj";
        // ⓐ 계정 dir 공격: 후보(남의 계정)와 일치하는 값을 신고 — 데몬이 아는 내 값은 CFG 다.
        assert_eq!(
            decide(Some(9), None, Some(CFG), &known_cwds(), Some(OTHER_CFG), Some(CWD), 10_000.0,
                   LeaseState::Free, &[ent("worker-2", 3, "empty", OTHER_CFG, CWD)]),
            Decision::EnvMismatch,
            "타 계정 dir 신고로 남의 역할을 가져갔다"
        );
        // ⓑ 프로젝트 공격: 같은 계정 안 **다른 cwd** 좌석을 신고로 가져간다.
        assert_eq!(
            decide(Some(9), None, Some(CFG), &known_cwds(), Some(CFG), Some(OTHER_CWD), 10_000.0,
                   LeaseState::Free, &[ent("worker-2", 3, "empty", CFG, OTHER_CWD)]),
            Decision::EnvMismatch,
            "타 프로젝트 좌석을 자기신고 cwd 로 가져갔다"
        );
        // ⓒ 결측도 불일치다(결측은 값이 아니다) — 데몬이 그 좌석의 축을 모르면 무결합.
        let no_cwds: Vec<String> = Vec::new();
        for (kc, kw) in [
            (None, &known_cwds()[..]),
            (Some(CFG), &no_cwds[..]),
            (None, &no_cwds[..]),
        ] {
            assert_eq!(
                decide(Some(9), None, kc, kw, Some(CFG), Some(CWD), 10_000.0, LeaseState::Free,
                       &[ent("worker-2", 3, "empty", CFG, CWD)]),
                Decision::EnvMismatch,
                "데몬이 모르는 좌석이 자기신고만으로 결합했다"
            );
        }
        // ⓓ 정직한 신고는 종전대로 결합한다(이 게이트가 장치를 죽이지 않는다).
        assert_eq!(
            decide(Some(9), None, Some(CFG), &known_cwds(), Some(CFG), Some(CWD), 10_000.0,
                   LeaseState::Free, &[ent("worker-2", 3, "empty", CFG, CWD)]),
            Decision::Bind { role: "worker-2".into(), from_surface: 3 }
        );
    }

    /// lease 파일 왕복: 우리가 잡고 있으면 두 번째 시도는 `Held`, 가드를 놓으면 다시 `Free`.
    #[cfg(unix)]
    #[test]
    fn lease_guard_is_exclusive_and_released_on_drop() {
        let dir = std::env::temp_dir().join(format!(
            "cys-reclaim-lease-{}-{}",
            std::process::id(),
            crate::state::now_epoch() as u64
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (st, guard) = try_hold_restore_lease(&dir);
        assert_eq!(st, LeaseState::Free);
        assert!(guard.is_some());
        assert!(restore_lease_path(&dir).exists(), "lease 파일이 생성되지 않았다");
        // ★같은 프로세스의 **다른 fd** 로 다시 잡으면 flock 은 배타적이다(같은 fd 는 재진입).
        let (st2, g2) = try_hold_restore_lease(&dir);
        assert_eq!(st2, LeaseState::Held, "보유 중인데 Free 로 보였다");
        assert!(g2.is_none());
        drop(guard);
        let (st3, _g3) = try_hold_restore_lease(&dir);
        assert_eq!(st3, LeaseState::Free, "drop 후에도 해제되지 않았다");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// ★심링크 대칭(적대검증 R1 minor): phoenix 는 `realpath(dirname(socket))` 로 lease 경로를
    /// 만든다. 우리 쪽 `state_dir` 은 `socket_path.parent()`(무정규화)라, 소켓 디렉터리에
    /// 심링크가 끼면 **두 프로세스가 서로 다른 파일을 잠그고** 배타가 무음으로 사라진다.
    /// 그래서 심링크 경유 state_dir 로 잡은 lease 는 실경로 state_dir 에서 `Held` 로 보여야 한다.
    #[cfg(unix)]
    #[test]
    fn lease_is_shared_through_a_symlinked_state_dir() {
        let base = std::env::temp_dir().join(format!(
            "cys-reclaim-lease-sym-{}-{}",
            std::process::id(),
            crate::state::now_epoch() as u64
        ));
        let real = base.join("real");
        let link = base.join("link");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let (st, guard) = try_hold_restore_lease(&link); // phoenix 대역: 심링크 경유
        assert_eq!(st, LeaseState::Free);
        assert!(guard.is_some());
        let (st2, g2) = try_hold_restore_lease(&real); // 데몬 대역: 실경로
        assert_eq!(st2, LeaseState::Held, "심링크 경유 lease 를 못 보고 배타가 사라졌다");
        assert!(g2.is_none());
        // 두 경로가 같은 파일을 가리킨다(경로 파생 자체의 대칭).
        assert_eq!(restore_lease_path(&link), restore_lease_path(&real));
        drop(guard);
        std::fs::remove_dir_all(&base).ok();
    }

    /// ★lease 판정 불능은 `Free` 로 강등되지 않는다 — 열 수 없는 경로(파일이 디렉터리 자리에
    /// 있는 상태)에서 `Unavailable` 이 나오고, `decide` 는 그것을 **보류**로 접는다.
    /// (phoenix 는 가용성을 위해 fail-open 하지만 '역할 변경 허가'는 모를 때 거절한다.)
    #[cfg(unix)]
    #[test]
    fn lease_open_failure_is_unavailable_not_free() {
        let base = std::env::temp_dir().join(format!(
            "cys-reclaim-lease-bad-{}-{}",
            std::process::id(),
            crate::state::now_epoch() as u64
        ));
        std::fs::create_dir_all(&base).unwrap();
        // `<state_dir>/phoenix` 자리에 **일반 파일**을 둬 create_dir_all 을 실패시킨다.
        std::fs::write(base.join("phoenix"), b"not-a-dir").unwrap();
        let (st, g) = try_hold_restore_lease(&base);
        assert_eq!(st, LeaseState::Unavailable, "락 기구 불능이 Free 로 접혔다");
        assert!(g.is_none());
        assert_eq!(
            decide(Some(9), None, Some(CFG), &known_cwds(), Some(CFG), Some(CWD), 10_000.0, st,
                   &[ent("worker-2", 3, "empty", CFG, CWD)]),
            Decision::Defer("restore_lease_unavailable"),
            "판정 불능인데 결합했다"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    // ── ★codex(gpt-6-astra) 위임 작성 · 전 줄 검토 후 채택 ────────────────────────────────
    // 사양(계약)만 주고 적대적 검체를 맡겼다. 채택 전 15종을 손으로 다시 계산해 대조했고,
    // **1건이 내 구현의 결함을 잡았다**: `C:////` 가 `c:////` 로 남아 `C:\` 와 만나지 못했다
    // (종전 ⑤가 "직전 글자가 `/` 면 멈춤"이라 UNC 보호가 드라이브 루트까지 잠갔다).
    // 그 지적대로 ⑤를 **루트 길이 기준**으로 다시 썼다 — 검체를 고친 것이 아니라 코드를 고쳤다.
    // 같은 Windows 디렉터리의 native/MSYS/cygpath 표기가 갈리면 좌석 승계가 끊긴다.
    #[test]
    fn norm_adversarial_native_msys_cygpath_converge() {
        for input in [r"C:\Work\Seat", "C:/Work/Seat", "/c/Work/Seat", "/C/Work/Seat/", r"c:\Work\Seat\"] {
            assert_eq!(norm_path_on(input, true), "c:/Work/Seat", "동일 디렉터리의 native/MSYS/cygpath 표기가 수렴하지 않았다: {input:?}");
        }
    }

    // 확장 드라이브 접두를 처리하지 못하면 같은 좌석 경로를 별개 위치로 오인한다.
    #[test]
    fn norm_adversarial_extended_drive_prefix_converges() {
        for input in [r"\\?\C:\Work\Seat", r"\\?\C:\Work\Seat\", r"\\?\c:\Work\Seat"] {
            assert_eq!(norm_path_on(input, true), "c:/Work/Seat", "확장 드라이브 접두 경로가 일반 경로로 수렴하지 않았다: {input:?}");
        }
    }

    // 확장 UNC 접두를 잘못 벗기면 네트워크 공유의 호스트 또는 루트를 잃을 수 있다.
    #[test]
    fn norm_adversarial_extended_unc_converges_and_keeps_root() {
        for input in [r"\\srv\Share", r"\\srv\Share\", "//srv/Share/", r"\\?\UNC\srv\Share\"] {
            assert_eq!(norm_path_on(input, true), "//srv/Share", "UNC 공유 루트의 접두 또는 후행 구분자 정규화가 틀렸다: {input:?}");
        }
        assert_eq!(norm_path_on(r"\\?\UNC\srv\Share\Seat", true), "//srv/Share/Seat", "확장 UNC 경로가 호스트·공유·본문을 보존하지 않았다");
    }

    // 드라이브 상대경로는 현재 디렉터리에 의존하므로 드라이브 루트와 합치면 다른 좌석을 승계한다.
    #[test]
    fn norm_adversarial_drive_root_is_not_drive_relative() {
        for input in ["C:/", r"C:\", "/c", "/C/", "C:////"] {
            assert_eq!(norm_path_on(input, true), "c:/", "드라이브 루트가 보존되지 않았다: {input:?}");
        }
        assert_eq!(norm_path_on("C:", true), "c:", "드라이브 상대경로에 루트 구분자가 추가되었다");
        assert_ne!(norm_path_on("C:/", true), norm_path_on("C:", true), "드라이브 루트와 드라이브 상대경로가 같다고 접혔다");
        assert_ne!(norm_path_on("C:/Seat", true), norm_path_on("C:Seat", true), "드라이브 절대경로와 드라이브 상대경로가 같다고 접혔다");
    }

    // 본문이 같아도 드라이브가 다르면 별개 위치이며, 이를 합치면 잘못된 좌석 소유자가 된다.
    #[test]
    fn norm_adversarial_distinct_drives_stay_distinct() {
        assert_ne!(norm_path_on(r"C:\Work\Seat", true), norm_path_on("/d/Work/Seat", true), "서로 다른 드라이브의 경로가 같다고 접혔다");
        assert_ne!(norm_path_on("/c", true), norm_path_on("/d", true), "서로 다른 드라이브 루트가 같다고 접혔다");
    }

    // 공유 이름이 같아도 UNC 호스트나 공유가 다르면 서로 다른 좌석 저장 위치다.
    #[test]
    fn norm_adversarial_unc_hosts_and_shares_stay_distinct() {
        let seat = norm_path_on(r"\\srv-a\Share\Seat", true);
        assert_ne!(seat, norm_path_on(r"\\?\UNC\srv-b\Share\Seat", true), "서로 다른 UNC 호스트의 경로가 같다고 접혔다");
        assert_ne!(seat, norm_path_on(r"\\srv-a\Other\Seat", true), "서로 다른 UNC 공유의 경로가 같다고 접혔다");
    }

    // 전체 소문자화는 계약상 구별해야 하는 본문 대소문자를 지워 다른 위치를 합친다.
    #[test]
    fn norm_adversarial_only_drive_letter_is_lowercased() {
        assert_eq!(norm_path_on(r"Z:\MiXeD\Seat", true), "z:/MiXeD/Seat", "드라이브 문자 외의 본문 대소문자가 바뀌었다");
        assert_ne!(norm_path_on("Z:/Seat", true), norm_path_on("z:/seat", true), "대소문자만 다른 경로 본문이 같다고 접혔다");
        assert_eq!(norm_path_on(r"\\HOST\Share\Seat", true), "//HOST/Share/Seat", "UNC 호스트 또는 공유의 대소문자가 바뀌었다");
    }

    // 공백을 trim하면 서로 다른 입력 경로가 합쳐져 잘못된 좌석 승계가 허용된다.
    #[test]
    fn norm_adversarial_windows_spaces_are_preserved() {
        for (input, expected) in [("C:/Seat ", "c:/Seat "), ("C:/Seat /", "c:/Seat "), (" C:/Seat", " C:/Seat")] {
            assert_eq!(norm_path_on(input, true), expected, "Windows 경로의 유효한 공백이 제거되었다: {input:?}");
        }
        assert_ne!(norm_path_on("C:/Seat ", true), norm_path_on("C:/Seat", true), "후행 공백만 다른 Windows 경로가 같다고 접혔다");
    }

    // MSYS 판별이 첫 글자만 읽으면 /cy 같은 일반 세그먼트를 C 드라이브로 오인한다.
    #[test]
    fn norm_adversarial_msys_requires_one_letter_segment() {
        for input in ["/cy", "/cy/Seat", "/cc/Seat", "/1/Seat"] {
            assert_eq!(norm_path_on(input, true), input, "한 글자 알파벳이 아닌 세그먼트를 MSYS 드라이브로 바꿨다: {input:?}");
        }
        assert_ne!(norm_path_on("/cy", true), norm_path_on("/c", true), "두 글자 세그먼트와 드라이브 루트가 같다고 접혔다");
        assert_ne!(norm_path_on("/cy/Seat", true), norm_path_on("/c/Seat", true), "두 글자 세그먼트와 MSYS 드라이브 경로가 같다고 접혔다");
    }

    // POSIX 공백은 파일명 일부이므로 선행·후행 공백 제거는 다른 디렉터리의 오인 일치다.
    #[test]
    fn norm_adversarial_posix_preserves_spaces() {
        for (input, expected) in [("/a ", "/a "), (" /a", " /a"), ("/a /", "/a "), ("/a/ ", "/a/ "), (" \t ", " \t ")] {
            assert_eq!(norm_path_on(input, false), expected, "POSIX 경로의 공백이 변경되었다: {input:?}");
        }
        assert_ne!(norm_path_on("/a ", false), norm_path_on("/a", false), "후행 공백만 다른 POSIX 디렉터리가 같다고 접혔다");
    }

    // POSIX에서 후행 슬래시 이외까지 손대면 Windows처럼 보이는 합법적인 파일명이 훼손된다.
    #[test]
    fn norm_adversarial_posix_removes_only_trailing_slashes() {
        for (input, expected) in [("/a///", "/a"), ("relative//", "relative"), ("/a//b/", "/a//b"), ("/C/Seat/", "/C/Seat"), (r"C:\Seat\", r"C:\Seat\"), ("/a/../b/", "/a/../b")] {
            assert_eq!(norm_path_on(input, false), expected, "POSIX 정규화가 후행 슬래시 외의 내용을 변경했거나 후행 슬래시를 남겼다: {input:?}");
        }
    }

    // 슬래시만 있는 POSIX 입력을 빈 문자열로 만들면 루트 경로가 결측값처럼 취급된다.
    #[test]
    fn norm_adversarial_posix_slash_only_roots_are_unchanged() {
        for input in ["", "/", "//", "///", "//////"] {
            assert_eq!(norm_path_on(input, false), input, "빈 입력 또는 슬래시만 있는 POSIX 루트의 원문이 바뀌었다: {input:?}");
        }
    }

    // 결측끼리 또는 공백뿐인 값끼리 일치하면 경로 증거 없이 좌석 승계를 허용하게 된다.
    #[test]
    fn same_adversarial_missing_and_blank_never_match() {
        let absent = [None, Some(""), Some(" "), Some("\t\r\n"), Some("\u{2003}")];
        for a in absent {
            for b in absent {
                assert!(!same_path(a, b), "결측 또는 trim 후 빈 경로끼리 일치로 판정했다: {a:?}, {b:?}");
            }
            assert!(!same_path(a, Some("/Seat")), "왼쪽 결측 또는 빈 경로가 유효 경로와 일치했다: {a:?}");
            assert!(!same_path(Some("/Seat"), a), "오른쪽 결측 또는 빈 경로가 유효 경로와 일치했다: {a:?}");
        }
    }

    // same_path가 검사 때의 trim 결과를 비교에도 쓰면 공백이 있는 다른 위치를 합친다.
    #[test]
    fn same_adversarial_compares_original_paths_after_normalization() {
        assert!(same_path(Some("/Seat"), Some("/Seat///")), "후행 슬래시만 다른 유효 경로가 일치하지 않았다");
        assert!(same_path(Some("/Seat "), Some("/Seat /")), "공백을 포함한 같은 경로의 후행 슬래시 표기가 일치하지 않았다");
        for (a, b) in [("/Seat ", "/Seat"), (" /Seat", "/Seat"), ("/Seat", "/seat")] {
            assert!(!same_path(Some(a), Some(b)), "공백 또는 본문 대소문자가 다른 원문 경로를 일치로 판정했다: {a:?}, {b:?}");
            assert!(!same_path(Some(b), Some(a)), "인수 순서를 바꾸자 서로 다른 원문 경로를 일치로 판정했다: {b:?}, {a:?}");
        }
    }
}
