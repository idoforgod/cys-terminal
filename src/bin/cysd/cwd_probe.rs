//! ★(0.14.41 · U18) 좌석 작업 폴더 **읽기 관측** — 관측·안내 전용(스폰 동작 불변).
//!
//! 【왜 필요한가】 macOS 가 폴더 접근 권한(TCC)으로 막은 폴더는 **stat·chdir 은 통과하고
//! 목록 읽기·파일 열기만 EPERM** 이다(착수 실험 `_evidence/impl-9items-20260923/D/exp-sandbox-tcc`
//! · phase1 U18 반박 R1). 그래서 portable-pty 의 `is_dir()` 필터는 참이 되고, 좌석 셸은
//! **그 폴더 안에서** 조용히 뜬다 — 좌석도 사람도 왜 파일이 안 읽히는지 모른다(오너 U18).
//! 같은 실험에서 claude 2.1.280 은 그 폴더에서도 대화형 화면까지 떴다. 그래서 cwd 를 홈으로
//! 바꾸는 대체(이주)는 하지 않고(설계 §2 · phoenix 미러·in-seat 경로를 흔든다), **사실만 드러낸다**:
//!   ① `Surface.cwd_blocked` — `surface.list`·`org.status` 의 `cwd_blocked`(GUI 가 3초 루프로 당겨
//!      폴더별 고정 토스트를 띄운다)
//!   ② pane env `CYS_CWD_BLOCKED=<그 폴더>` — SessionStart 훅이 좌석에 1줄 고지한다.
//!
//! 【판정 규약】
//! - **EPERM(errno 1) 하나만** 막힘이다. TCC·샌드박스 거부의 신호가 그것이다. EACCES(13 · 유닉스
//!   권한)·ENOENT(부재)·그 밖의 오류는 막힘이 아니다(알림 없음 — 원인이 다르고 처방도 다르다).
//! - **시한 초과·스레드 생성 실패·패닉은 전부 `Unknown`** = 알림 없음 · 동작 불변. 반박 D-4:
//!   데몬 자식은 권한 창을 띄우지 못하므로 시한 초과의 실제 원인은 iCloud File Provider·네트워크
//!   볼륨·느린 디스크다(오너 기계의 데스크탑이 iCloud 관리 폴더). 그것을 '막힘'으로 부르면 허용된
//!   좌석에 거짓 경보가 간다.
//! - **역할 좌석만** 관측한다. 사람이 연 일반 pane(role 없음)은 대상이 아니다(반박 D-10).
//! - **macOS 에서만** IO 한다. 다른 OS 는 호출부가 `cfg!(target_os = "macos")` 로 가둬 IO 0 ·
//!   env 조작 0 이다(Windows 스폰 경로 무변경).
//!
//! 【폭주·전멸 상한(코드로 박힌 값)】
//! - 관측은 좌석 생성 1회당 read_dir 1회 · 재시도 0.
//! - 전용 스레드는 `std::thread::Builder::spawn`(실패 = Result → Unknown · panic 아님).
//! - 같은 경로의 관측이 아직 안 끝났으면 새 스레드를 만들지 않는다(→ Unknown). 동시에 떠 있을 수
//!   있는 관측 스레드는 전체 [`MAX_INFLIGHT`] 개가 상한이다 — 멈춘 볼륨이 스레드를 쌓지 못한다.
//! - 호출부는 [`CWD_PROBE_TIMEOUT`] 만 기다린다. 이 지점은 surface.create 의 **락 미보유 구간**
//!   (handlers.rs 게이트 ⑤ 주석)이고 핸들러는 spawn_blocking 위라 데몬 전체가 멈추지 않는다.
//! - 롤백: `CYS_CWD_PROBE=0` 또는 마스터 스위치 `CYS_BOOT_GATES=0` → IO 0 · 필드·env 없음(종전).

use std::io;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

/// pane env 키 — 이 좌석이 생성될 때 작업 폴더 목록 읽기가 EPERM 이었다(값 = 그 폴더).
/// 소비자: `cysjavis-pack/hooks/session-start.sh`(좌석 1줄 고지). 파리티 핀은 아래 tests.
pub const ENV_CWD_BLOCKED: &str = "CYS_CWD_BLOCKED";

/// 개별 롤백 노브 — `"0"` 이면 관측하지 않는다(IO 0 · 종전 동작).
pub const ENV_CWD_PROBE: &str = "CYS_CWD_PROBE";

/// 관측 시한. 로컬 디스크의 read_dir 1회는 마이크로초 단위다 — 이 시한은 '느린 볼륨에서
/// 좌석 생성이 얼마나 늦어져도 되는가'의 상한이지 판정 근거가 아니다(넘기면 Unknown).
pub const CWD_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// 동시에 떠 있을 수 있는 관측 스레드 상한(경로 무관 전체). 넘치면 관측하지 않는다(Unknown).
pub const MAX_INFLIGHT: usize = 8;

/// 관측 결과.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CwdProbe {
    /// 목록을 읽었다.
    Readable,
    /// EPERM — macOS 폴더 접근 권한(TCC)·샌드박스 거부.
    Blocked,
    /// 다른 오류(EACCES·ENOENT 등) — 막힘이 아니다(알림 없음).
    OtherError,
    /// 판정 불가(시한 초과·스레드 생성 실패·패닉·동시 관측 상한) — 알림 없음 · 동작 불변.
    Unknown,
}

/// 좌석에 기록되는 막힘 사실(`surface.list`·`org.status` 의 `cwd_blocked` object).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CwdBlocked {
    /// 요청된 작업 폴더(= 실제 스폰 cwd — 스폰 동작은 바꾸지 않는다).
    pub path: String,
    /// 홈 바로 아래 보호 폴더면 그 이름(`Desktop`·`Documents`·`Downloads`) — 안내 문구 전용.
    /// 판정에는 쓰지 않는다(iCloud·외장 볼륨 같은 다른 보호 위치도 errno 로 잡힌다).
    pub folder: Option<&'static str>,
}

impl CwdBlocked {
    /// wire 표현 — 소비자 술어는 "object 인가" 하나다(필드는 표시 전용).
    pub fn to_wire(&self) -> serde_json::Value {
        serde_json::json!({"path": self.path, "folder": self.folder})
    }
}

/// 읽기 결과 분류(순수). errno 1(EPERM)만 막힘이다.
pub fn classify(r: &io::Result<()>) -> CwdProbe {
    match r {
        Ok(()) => CwdProbe::Readable,
        Err(e) if e.raw_os_error() == Some(1) => CwdProbe::Blocked,
        Err(_) => CwdProbe::OtherError,
    }
}

/// 관측을 할 것인가(순수). macOS ∧ 역할 좌석 ∧ 두 롤백 스위치가 켜져 있지 않을 때만.
pub fn enabled_from(
    is_macos: bool,
    role: Option<&str>,
    knob: Option<&str>,
    boot_gates: Option<&str>,
) -> bool {
    is_macos
        && role.map(|r| !r.trim().is_empty()).unwrap_or(false)
        && knob.map(str::trim) != Some("0")
        && !cys::boot_gates_master_off_from(boot_gates)
}

/// 홈 바로 아래 보호 폴더 이름(순수 · 안내 문구 전용). 경로 구분자 경계를 지킨다
/// (`~/DesktopX` 는 Desktop 이 아니다).
pub fn protected_folder_of(path: &str, home: &str) -> Option<&'static str> {
    let home = home.trim_end_matches('/');
    if home.is_empty() {
        return None;
    }
    let rest = path.strip_prefix(home)?.strip_prefix('/')?;
    let first = rest.split('/').next().unwrap_or("");
    ["Desktop", "Documents", "Downloads"].into_iter().find(|f| *f == first)
}

/// 실제 읽기 — 목록을 열고 첫 항목까지 읽는다(열기는 되고 읽기만 막히는 경우까지 잡는다).
pub fn read_first_entry(path: &str) -> io::Result<()> {
    let mut it = std::fs::read_dir(path)?;
    match it.next() {
        Some(Err(e)) => Err(e),
        _ => Ok(()),
    }
}

static INFLIGHT: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct InflightGuard(String);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        let mut g = INFLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = g.iter().position(|p| *p == self.0) {
            g.swap_remove(i);
        }
    }
}

/// 시한부 관측 — 전용 스레드에서 `reader(path)` 를 돌리고 `timeout` 만 기다린다.
/// 어떤 실패도 호출부로 전파하지 않는다(전부 [`CwdProbe::Unknown`]).
pub fn probe_bounded(path: &str, timeout: Duration, reader: fn(&str) -> io::Result<()>) -> CwdProbe {
    {
        let mut g = INFLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        if g.iter().any(|p| p == path) || g.len() >= MAX_INFLIGHT {
            return CwdProbe::Unknown;
        }
        g.push(path.to_string());
    }
    // sync_channel(1): 수신자가 시한으로 떠난 뒤에도 send 가 막히지 않는다(버퍼 1칸).
    let (tx, rx) = mpsc::sync_channel::<CwdProbe>(1);
    let p = path.to_string();
    let spawned = std::thread::Builder::new()
        .name("cysd-cwd-probe".into())
        .spawn(move || {
            let _guard = InflightGuard(p.clone());
            let verdict = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader(&p))) {
                Ok(r) => classify(&r),
                Err(_) => CwdProbe::Unknown,
            };
            let _ = tx.send(verdict);
        });
    if spawned.is_err() {
        // 클로저가 돌지 않았으니 가드도 없다 — 등록을 직접 걷는다.
        drop(InflightGuard(path.to_string()));
        return CwdProbe::Unknown;
    }
    rx.recv_timeout(timeout).unwrap_or(CwdProbe::Unknown)
}

/// surface 생성 합류점에서 부르는 관측 1회. `Some` 은 EPERM 확정일 때만이다.
/// env 판독은 생성 시 1회(U-20·좌석 토큰 선례). 어떤 실패도 `None`(= 종전 동작)으로 접는다.
pub fn observe(role: Option<&str>, cwd: &str) -> Option<CwdBlocked> {
    let knob = std::env::var(ENV_CWD_PROBE).ok();
    let gates = std::env::var(cys::ENV_BOOT_GATES).ok();
    if !enabled_from(cfg!(target_os = "macos"), role, knob.as_deref(), gates.as_deref()) {
        return None;
    }
    let verdict = std::panic::catch_unwind(|| probe_bounded(cwd, CWD_PROBE_TIMEOUT, read_first_entry))
        .unwrap_or(CwdProbe::Unknown);
    if verdict != CwdProbe::Blocked {
        return None;
    }
    let home = dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    Some(CwdBlocked { path: cwd.to_string(), folder: protected_folder_of(cwd, &home) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn classify_only_eperm_is_blocked() {
        assert_eq!(classify(&Ok(())), CwdProbe::Readable);
        assert_eq!(classify(&Err(io::Error::from_raw_os_error(1))), CwdProbe::Blocked);
        // EACCES(유닉스 권한)·ENOENT·ENOTDIR 은 막힘이 아니다 — 원인·처방이 다르다.
        for errno in [2, 13, 20] {
            assert_eq!(
                classify(&Err(io::Error::from_raw_os_error(errno))),
                CwdProbe::OtherError,
                "errno {errno}"
            );
        }
        // errno 없는 합성 오류도 막힘이 아니다.
        assert_eq!(
            classify(&Err(io::Error::new(io::ErrorKind::PermissionDenied, "x"))),
            CwdProbe::OtherError
        );
    }

    #[test]
    fn enabled_only_for_mac_role_seats_and_both_switches_roll_back() {
        assert!(enabled_from(true, Some("worker"), None, None));
        assert!(!enabled_from(false, Some("worker"), None, None), "비-mac 은 IO 0");
        assert!(!enabled_from(true, None, None, None), "역할 없는 일반 pane 은 관측하지 않는다");
        assert!(!enabled_from(true, Some("  "), None, None));
        assert!(!enabled_from(true, Some("worker"), Some("0"), None), "개별 노브");
        assert!(!enabled_from(true, Some("worker"), None, Some("0")), "마스터 스위치 CYS_BOOT_GATES=0");
        assert!(enabled_from(true, Some("worker"), Some("1"), Some("1")));
    }

    #[test]
    fn protected_folder_respects_path_boundaries() {
        // (경로 표본은 secret-scan 의 더미 사용자 이름만 쓴다)
        let h = "/Users/user";
        assert_eq!(protected_folder_of("/Users/user/Desktop", h), Some("Desktop"));
        assert_eq!(protected_folder_of("/Users/user/Desktop/CYSjavis/x", h), Some("Desktop"));
        assert_eq!(protected_folder_of("/Users/user/Documents/a", "/Users/user/"), Some("Documents"));
        assert_eq!(protected_folder_of("/Users/user/Downloads", h), Some("Downloads"));
        assert_eq!(protected_folder_of("/Users/user/DesktopX", h), None);
        assert_eq!(protected_folder_of("/Users/user", h), None);
        assert_eq!(protected_folder_of("/Volumes/Ext/Desktop", h), None);
        // 홈 접두가 우연히 겹치는 이웃 경로는 홈 아래가 아니다(문자열 접두 ≠ 경로 접두).
        assert_eq!(protected_folder_of("/tmp/hh/Desktop", "/tmp/h"), None);
        assert_eq!(protected_folder_of("/Users/user/Desktop", ""), None);
    }

    fn eperm(_: &str) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(1))
    }
    fn slow(_: &str) -> io::Result<()> {
        std::thread::sleep(Duration::from_millis(600));
        Ok(())
    }
    fn boom(_: &str) -> io::Result<()> {
        panic!("reader panic")
    }

    #[test]
    fn bounded_probe_folds_every_failure_into_unknown() {
        assert_eq!(probe_bounded("/u18/eperm", Duration::from_secs(2), eperm), CwdProbe::Blocked);
        // 시한 초과 = Unknown(막힘 아님 · 반박 D-4) — 그리고 시한만큼만 기다린다.
        let t0 = Instant::now();
        assert_eq!(probe_bounded("/u18/slow", Duration::from_millis(50), slow), CwdProbe::Unknown);
        assert!(t0.elapsed() < Duration::from_millis(500), "시한을 넘겨 기다렸다: {:?}", t0.elapsed());
        // 같은 경로의 관측이 아직 돌고 있으면 새 스레드를 만들지 않는다(Unknown).
        assert_eq!(probe_bounded("/u18/slow", Duration::from_secs(2), eperm), CwdProbe::Unknown);
        // 패닉은 Unknown 으로 흡수된다(호출부 전파 0).
        assert_eq!(probe_bounded("/u18/boom", Duration::from_secs(2), boom), CwdProbe::Unknown);
        // 느린 관측이 끝나면 등록이 걷혀 같은 경로를 다시 잴 수 있다.
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(probe_bounded("/u18/slow", Duration::from_secs(2), eperm), CwdProbe::Blocked);
    }

    #[test]
    fn inflight_cap_bounds_hung_threads() {
        fn hang(_: &str) -> io::Result<()> {
            std::thread::sleep(Duration::from_millis(700));
            Ok(())
        }
        let paths: Vec<String> = (0..MAX_INFLIGHT + 3).map(|i| format!("/u18/cap/{i}")).collect();
        let mut unknown_fast = 0;
        for p in &paths {
            let t0 = Instant::now();
            let v = probe_bounded(p, Duration::from_millis(10), hang);
            assert_eq!(v, CwdProbe::Unknown);
            if t0.elapsed() < Duration::from_millis(5) {
                unknown_fast += 1;
            }
        }
        // 상한을 넘긴 3건 이상은 스레드를 만들지 않고 즉시 Unknown 이다.
        assert!(unknown_fast >= 3, "동시 관측 상한이 스레드를 막지 못했다: {unknown_fast}");
        std::thread::sleep(Duration::from_millis(900));
        assert!(INFLIGHT.lock().unwrap().iter().all(|p| !p.starts_with("/u18/cap/")), "등록이 걷히지 않았다");
    }

    #[test]
    fn real_reader_on_readable_and_missing_dirs() {
        let d = std::env::temp_dir().join(format!("cys-u18-probe-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let ds = d.to_string_lossy().into_owned();
        assert_eq!(probe_bounded(&ds, CWD_PROBE_TIMEOUT, read_first_entry), CwdProbe::Readable);
        std::fs::write(d.join("f"), "x").unwrap();
        assert_eq!(probe_bounded(&ds, CWD_PROBE_TIMEOUT, read_first_entry), CwdProbe::Readable);
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(probe_bounded(&ds, CWD_PROBE_TIMEOUT, read_first_entry), CwdProbe::OtherError);
    }

    #[test]
    fn observe_is_off_for_plain_panes() {
        // 역할 없는 pane 은 어떤 OS 에서도 관측하지 않는다(IO 0).
        assert_eq!(observe(None, "/definitely/missing/u18"), None);
    }

    /// 파리티 핀 — 훅이 읽는 env 키가 이 상수와 같다(측정 불능은 실패).
    #[test]
    fn session_start_hook_reads_the_same_env_key() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("cysjavis-pack/hooks/session-start.sh");
        let src = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        assert!(
            src.contains(&format!("${{{ENV_CWD_BLOCKED}:-}}")),
            "session-start.sh 가 {ENV_CWD_BLOCKED} 를 읽지 않는다 — 데몬이 실은 사실이 좌석에 닿지 않는다"
        );
    }

    /// 이 모듈의 사건은 **이벤트로 내지 않는다**(소비자는 pull 이고, master 는 `cys events` 를
    /// 직접 구독하므로 이벤트는 곧 에이전트 가시 신호다). 누군가 이름을 붙여 내더라도 에이전트
    /// 큐 라우팅 허용목록 밖이어야 한다.
    #[test]
    fn cwd_blocked_is_never_agent_routable() {
        assert!(!crate::alert_route::routable("surface.cwd_blocked"));
    }
}
