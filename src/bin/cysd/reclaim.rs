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
            Decision::Defer(r) => r,
            Decision::NoCandidate => "no_candidate",
            Decision::Ambiguous(_) => "ambiguous",
            Decision::Bind { .. } => "bind",
        }
    }
}

/// 경로 비교 정규화 — **순수**(파일시스템 접근 0). 공백 제거 + 후행 슬래시 제거.
/// 심링크·상대경로 해소는 하지 않는다: 데몬이 기록한 `Surface.cwd` 와 훅이 넘긴 `$PWD` 는
/// 같은 기계의 같은 표기이므로 일치하며, 정규화를 넓히면 **다른 디렉터리를 같다고 말할** 위험만
/// 커진다(이 장치의 최악 오작동 방향).
pub fn norm_path(p: &str) -> String {
    let t = p.trim();
    let stripped = t.trim_end_matches('/');
    if stripped.is_empty() {
        t.to_string() // "/" 자체(또는 빈 문자열)는 그대로
    } else {
        stripped.to_string()
    }
}

/// 두 경로가 **둘 다 있고** 같은가. ★결측은 값이 아니다 — `None == None` 은 일치가 아니다.
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
pub fn restore_lease_path(state_dir: &Path) -> PathBuf {
    state_dir.join("phoenix").join("restore.lease")
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

#[cfg(test)]
mod tests {
    use super::*;

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

    fn decide_one(entries: &[LiveEntry]) -> Decision {
        decide(Some(9), None, Some(CFG), Some(CWD), 10_000.0, LeaseState::Free, entries)
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
            let d = decide(Some(9), None, Some(CFG), Some(CWD), 10_000.0, st, &e);
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
            decide(Some(9), None, None, None, 10_000.0, LeaseState::Free, &[e.clone()]),
            Decision::EnvMissing
        );
        assert_eq!(
            decide(Some(9), None, Some(""), Some("   "), 10_000.0, LeaseState::Free, &[e.clone()]),
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
            decide(Some(9), Some("cso"), Some(CFG), Some(CWD), 10_000.0, LeaseState::Free, &e),
            Decision::AlreadyRoled("cso".into())
        );
    }

    /// 발신 좌석 미해석 → 무결합(자기신고 surface_id 를 받지 않으므로 유일한 신원 경로다).
    #[test]
    fn caller_unresolved_binds_nothing() {
        let e = [ent("worker-2", 3, "empty", CFG, CWD)];
        assert_eq!(
            decide(None, None, Some(CFG), Some(CWD), 10_000.0, LeaseState::Free, &e),
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

    /// 경로 정규화는 후행 슬래시까지만 — 그 이상 넓히지 않는다(다른 디렉터리를 같다고 말할 위험).
    #[test]
    fn path_normalization_is_narrow() {
        assert_eq!(norm_path("/a/b/"), "/a/b");
        assert_eq!(norm_path(" /a/b "), "/a/b");
        assert_eq!(norm_path("/"), "/");
        assert!(same_path(Some("/a/b/"), Some("/a/b")));
        assert!(!same_path(Some("/a/b"), Some("/a/B")), "대소문자 관용 금지");
        assert!(!same_path(Some("~/dev/x"), Some("/Users/cys/dev/x")), "tilde 확장 금지(순수)");
        assert!(!same_path(None, None), "결측끼리 일치");
        assert!(!same_path(Some(""), Some("")), "빈 문자열끼리 일치");
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
}
