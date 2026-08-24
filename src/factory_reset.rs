//! factory_reset — "완전 초기화"(팩토리 리셋): 사용 흔적 전부를 격리하고 설치 초기 상태로.
//!
//! 계약·인벤토리·실패 모델의 정본은 docs/DESIGN-factory-reset.md. 핵심 불변식:
//!  · 삭제가 아니라 **격리**(mv → `~/.local/state/cys-trash/factory-reset-<UTC>/` + manifest).
//!  · `~/.cys` 는 통째 이동 금지 — **알려진 항목 열거**로만 격리하고 미등록 파일(오너 배치:
//!    *.env·인증서 등)은 제자리 보존한다. 라이선스 파일은 기본 보존(opt-in 격리).
//!  · 격리는 **데몬 전멸 실측 확인 뒤에만**(살아있는 데몬 밑에서 DB 를 옮기지 않는다).
//!  · 팩 스크립트 무의존 — 리셋은 팩이 깨진 상태에서도 돌아야 하는 최후 수단이라 코어가
//!    Rust lib 에 있다(부서 purge 의 javis_org.py 위임과 의도적으로 다른 결).

use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────
// 대상 인벤토리(설계 §3) — 2026-08-16 소스·실기기 전수 조사에서 파생한 단일 진실.
// ~/.cys 직하 분류: 정확 이름 / 접두 / 라이선스(기본 보존) / 그 외 전부 = 미등록 보존.
// ─────────────────────────────────────────────────────────────────────────────

/// ~/.cys 직하에서 격리하는 정확 이름. (pack·조직 레지스트리·상태·마커·토글 전부)
const CYS_BASE_EXACT: [&str; 25] = [
    "pack",
    "pack.prev",
    ".pack-download",
    ".pack-journal",
    ".pack-accepted.json",
    ".pack-apply.lock",
    ".pack-reinject-pending.json",
    "claude",
    "state",
    "state-generations",
    "state-harness",
    "_round",
    "depts.json",
    "depts.json.lock",
    "dept-catalog.json",
    "dept-catalog.json.lock",
    "dept-missions",
    "dept-snapshots",
    "accounts.json",
    "policy.json",
    "profile.json",
    "approvals.json",
    ".approval-secret",
    ".gui-onboarded",
    ".last-app-version",
];

/// ~/.cys 직하에서 격리하는 정확 이름(2차) — 배열 상수 길이 고정을 피하려 분리하지 않고
/// 접두로 못 잡는 단건들을 이어 담는다.
const CYS_BASE_EXACT2: [&str; 6] = [
    ".pending-restore",
    "ime-debug",
    "allow-app-mouse",
    "url-allow-hosts",
    "harness-creator",
    // ★A1(성찰 확정): GUI 전출(핸드오프)이 프로젝트 상대 경로를 못 잡을 때 쓰는 폴백 —
    // transfer-<sid>-<ts>.md 에 세션 결정·리스크가 남는 대화 파생 흔적이다
    // (src-tauri/src/main.rs home_dir_path 폴백 · ui/src/main.ts 전출 경로).
    "transfers",
];

/// ~/.cys 직하에서 격리하는 접두. `pack-dept-<name>`(유령 `pack-dept---help` 포함),
/// `claude-<acct>`, `.pack-staging`(+`-init-<pid>`), `.master-bootstrapped`(+`-<lane>`).
const CYS_BASE_PREFIX: [&str; 4] = [
    "pack-dept-",
    "claude-",
    ".pack-staging",
    ".master-bootstrapped",
];

/// 기본 보존(오너 구매물) — `purge_license` 로만 격리 대상이 된다. 파일명 지식은 정적 핀
/// (라이선스 파일명 리터럴은 license.rs에만)에 따라 license 표면에서 가져온다.
const LICENSE_FILES: [&str; 2] = crate::license::LICENSE_BASENAMES;

/// ★A8(성찰 확정): `~/.cys/local` 은 **오너 저작 오버레이**다 — 업데이터가 절대 건드리지 않는
/// 사용자 전용 영역(직접 만든 지침 append·스킬·훅·노트). 종전엔 무조건 격리했는데 두 계약을
/// 동시에 깼다: ①고지문 "직접 넣은 파일은 삭제되지 않습니다" ②격리본은 14일 뒤 reap 가 영구
/// 소거하므로 **사용자 저작물이 사라진다**. 게다가 settings.json 에 직접 등록된 local 훅은
/// strip 대상(pack) 밖이라 격리 후 "No such file" 깨진 훅으로 영구 잔존했다.
/// → 기본 보존(등록 훅도 계속 유효 = dangling 0), 연습 잔재까지 지우려면 `--purge-local` opt-in.
const LOCAL_OVERLAY: &str = "local";

/// `$TMPDIR` 소거 대상 접두(캐시 등급 한정 — 격리 독트린의 예외는 OS 관리 임시 영역뿐).
const TEMP_SWEEP_PREFIX: [&str; 7] = [
    "cys-paste",
    "cys-ime.log",
    "cys-pack-guard",
    "cys_chan_test_",
    "cycverftest-",
    "cso_watch_prev.",
    // ★W7: `cys daemon install` 이 schtasks 등록에 쓰는 임시 XML(데몬 절대경로·계정명 포함).
    // 등록 중 중단되면 %TEMP% 에 남는다(cys.rs 의 Windows daemon install 경로).
    "cysd-task.xml",
];

/// ★★Windows 전용 치명 방어(감사 확정 2026-08-16): `%LOCALAPPDATA%\cys` 는 **NSIS 설치
/// 디렉토리이자 동시에 메인 데몬의 상태 디렉토리**다(tauri.windows.conf.json installMode
/// "currentUser" + productName "cys" · cysd/state.rs state_dir). 그래서 이 디렉토리를 통째로
/// 격리하면 **앱 자신(cys.exe·cysd.exe·runtime/·resources/)을 언인스톨**해 버린다.
/// → `~/.cys` 와 같은 교리를 적용한다: **알려진 상태 항목만** 격리하고 나머지(=설치본)는 보존.
/// 놓친 상태 파일이 남는 것은 불편이지만, 앱을 옮기는 것은 복구 불능급 사고다(fail-safe 방향).
const WIN_STATE_EXACT: [&str; 21] = [
    "transcripts.db",
    "analytics.db",
    "channels.db",
    "feed.jsonl",
    "feed.jsonl.tmp",
    "approval_audit.jsonl",
    "queue-state.json",
    "queue-state-v2.json",
    "autopilot.json",
    "topology.json",
    "dept_tombstones.json",
    "event.seq",
    "schedule_state.json",
    "learn_stuck_debounce.json",
    "operator.token",
    "heartbeat",
    "lockloss.state",
    "cysd.log",
    "phoenix-restore.log",
    "dead-letters.jsonl",
    "oob-cooldowns.json",
];

/// 접두로 잡는 Windows 상태 항목(부서 슬러그 디렉토리·저널/스풀 디렉토리·손상 격리본).
const WIN_STATE_PREFIX: [&str; 7] = [
    "cys-dept-",          // 부서 데몬 슬러그 디렉토리(state.rs pipe_slug 규약)
    "phoenix",            // phoenix/ · phoenix-embed/
    "office-bridge",      // office-bridge/ · office-bridge.log
    "report_gate",        // report_gate*/badges.json 레인
    "cycle_autopilot",    // 사이클 오토파일럿 상태
    "schedule_state.json.corrupt-", // 손상 격리본
    "analytics.db-",      // WAL/SHM 사이드카
];

/// D1a 계승 보호 루트 — realpath 가 이 중 하나면 어떤 격리도 금지.
const PROTECTED_ROOTS: [&str; 6] = ["/", "/Users", "/tmp", "/var", "/private/tmp", "/private/var"];

// ─────────────────────────────────────────────────────────────────────────────
// 경로 루트 — env override 를 **의도적으로 무시**하고 홈에서 파생한다. 리셋 대상은
// "표준 설치"이고, override 환경(테스트 샌드박스·커스텀 소켓)은 리셋의 대상이 아니다.
// 테스트는 이 구조체에 temp 루트를 주입한다(라이브 홈 무접촉).
// ─────────────────────────────────────────────────────────────────────────────

pub struct ResetRoots {
    pub home: PathBuf,
    /// ~/.cys
    pub cys_base: PathBuf,
    /// unix ~/.local/state (Windows %LOCALAPPDATA% 소비자는 state_dirs()가 흡수)
    pub state_root: PathBuf,
    /// 격리 목적지 루트(~/.local/state/cys-trash — cys-dept 규약과 동일 루트)
    pub trash_root: PathBuf,
    /// macOS ~/Library (WebKit·Caches·LaunchAgents). 타 OS = None.
    pub library: Option<PathBuf>,
    /// macOS Darwin 캐시 컨테이너(getconf DARWIN_USER_CACHE_DIR). 실패·타 OS = None.
    pub darwin_cache: Option<PathBuf>,
    /// std::env::temp_dir()
    pub temp: PathBuf,
    /// 작업기억 워크스페이스 루트(CYS_ROOT 규약 기본 ~/Desktop/CYSjavis)
    pub workspace_root: PathBuf,
    /// Windows 데몬 상태 루트(%LOCALAPPDATA%\cys — 메인+부서 슬러그 디렉토리가 이 안에 산다,
    /// cysd state.rs 슬러그 격리 규약·GUIDE-clean-reset §윈도우 2단계 5번). unix = None.
    pub win_local_state: Option<PathBuf>,
    /// Windows GUI WebView 데이터(%LOCALAPPDATA%\com.cysjavis.terminal — macOS WebKit 대응).
    /// unix = None.
    pub win_webview_data: Option<PathBuf>,
}

impl ResetRoots {
    /// 라이브 머신 루트. 홈 미해석 시 None(리셋 불가 — 조용한 "/" 폴백 금지).
    pub fn live() -> Option<Self> {
        let home = dirs::home_dir()?;
        let workspace_root = std::env::var("CYS_ROOT")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("Desktop/CYSjavis"));
        let darwin_cache = if cfg!(target_os = "macos") {
            std::process::Command::new("getconf")
                .arg("DARWIN_USER_CACHE_DIR")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        } else {
            None
        };
        // Windows: 데몬 상태는 ~/.local/state 가 아니라 %LOCALAPPDATA%\cys (state.rs 슬러그
        // 격리 규약). trash 는 가이드 규약대로 %USERPROFILE%\.local\state\cys-trash 유지.
        let (win_local_state, win_webview_data) = if cfg!(windows) {
            let dld = dirs::data_local_dir();
            (
                dld.as_ref().map(|d| d.join("cys")),
                dld.as_ref().map(|d| d.join("com.cysjavis.terminal")),
            )
        } else {
            (None, None)
        };
        Some(ResetRoots {
            cys_base: home.join(".cys"),
            state_root: home.join(".local/state"),
            trash_root: home.join(".local/state/cys-trash"),
            library: cfg!(target_os = "macos").then(|| home.join("Library")),
            darwin_cache,
            temp: std::env::temp_dir(),
            workspace_root,
            win_local_state,
            win_webview_data,
            home,
        })
    }
}

pub struct ResetOptions {
    pub purge_license: bool,
    /// 오너 저작 오버레이(~/.cys/local)까지 격리한다. 기본 false(보존) — LOCAL_OVERLAY 주석 참조.
    pub purge_local: bool,
    /// 사용자 프로젝트 폴더 안의 `_round`(작업기억)까지 격리한다. 기본 false(보고만) —
    /// round_candidates 주석의 P0-3 판정 참조.
    pub purge_round: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Quarantine,
    Keep,
}

pub struct PlanItem {
    pub path: PathBuf,
    pub label: String,
    pub size_bytes: u64,
    pub action: Action,
    /// ★A5/W5(성찰 확정): 실행 중인 **자기 앱**이 점유한 GUI 웹층(WebKit/WebView2·캐시·plist).
    /// Windows 는 열린 파일을 품은 디렉토리 rename 이 공유 위반으로 **항상** 실패하고, macOS 는
    /// 성공하더라도 살아있는 WebView·cfprefsd 가 원 경로를 다시 만들 수 있다. 이 등급의 실패는
    /// 결함이 아니라 **예정된 이연**이므로 `failed`(=부분 실패)로 세지 않고 `deferred` 로 보고해
    /// "앱 종료 후 재실행하면 정리된다"고 안내한다(맥 정상/윈도 매번 부분실패 비대칭 제거).
    pub best_effort: bool,
    /// ★P0-2: `~/.cys`·데몬 상태 루트 **밖**의 경로인가(사용자 프로젝트 폴더 안 작업기억 등).
    /// 확인 모달이 이 항목만 따로 강조해 "내 폴더에서 뭐가 사라지는지"를 승인 전에 보여준다.
    pub outside_state: bool,
}

pub struct ResetPlan {
    pub stamp: String,
    pub trash_dir: PathBuf,
    pub quarantine: Vec<PlanItem>,
    pub keep: Vec<PlanItem>,
    /// 자동 수정 금지 — 사람이 판단할 잔재 안내(zshrc alias·codex trust·프로젝트 파일).
    pub report_only: Vec<String>,
    /// cys 훅·statusLine 을 외과 제거할 개인 프로필 settings.json 목록.
    pub strip_settings: Vec<PathBuf>,
    /// pack 스킬 심링크를 제거할 개인 프로필 skills/ 디렉토리 목록.
    pub strip_skill_dirs: Vec<PathBuf>,
    pub temp_sweep: Vec<PathBuf>,
    /// launchd plist(존재 시에만 등록 해제 수행 — 테스트 temp 홈에선 자연히 스킵).
    pub launchd_plist: Option<PathBuf>,
    pub purge_license: bool,
    /// strip 범위 결정에도 쓰인다 — local 을 격리했으면 local 을 가리키는 훅도 함께 해제해야
    /// dangling 이 안 남고, 보존했으면 그 훅은 **유효하므로 건드리지 않는다**(대칭 계약).
    pub purge_local: bool,
    pub purge_round: bool,
    /// ★P0-6: 격리 목적지가 지금 쓸 수 있는가(쓰기 0 실측). Err 면 **데몬을 건드리기 전에**
    /// 거부해야 한다 — 종전엔 전멸·launchd 해제 뒤에야 실패해 시스템만 더 망가졌다.
    pub trash_root_ready: Result<(), String>,
    /// ★P0-5: 이전 초기화가 중단된 흔적(manifest 없는 factory-reset-* 폴더). 프리뷰·완료 양쪽에
    /// 노출해 "격리 폴더가 둘인데 어느 쪽에서 복구하나" 혼선을 없앤다.
    pub interrupted_prior: Vec<PathBuf>,
}

impl ResetPlan {
    pub fn quarantine_total_bytes(&self) -> u64 {
        self.quarantine.iter().map(|i| i.size_bytes).sum()
    }
}

fn dir_size(p: &Path) -> u64 {
    let meta = match std::fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(_) => return 0,
    };
    if !meta.is_dir() {
        return meta.len();
    }
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            match e.file_type() {
                // 심링크는 따라가지 않는다(격리 대상 밖 크기 오산·루프 방지).
                Ok(ft) if ft.is_dir() && !ft.is_symlink() => total += dir_size(&e.path()),
                Ok(_) => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

/// D1a 계승 격리 적격 게이트: 실존·심링크 루트 거부·realpath 가 $HOME 아래·$HOME 자신 아님·
/// 보호 루트 아님. 하나라도 어긋나면 부적격(사유 반환) — 격리하지 않고 보고만 한다.
fn quarantine_eligible(p: &Path, home: &Path) -> Result<(), &'static str> {
    let meta = std::fs::symlink_metadata(p).map_err(|_| "absent")?;
    if meta.file_type().is_symlink() {
        return Err("symlink-root"); // 링크만 옮기면 실체가 남는다 — 자동 격리 거부.
    }
    let real = std::fs::canonicalize(p).map_err(|_| "canonicalize-failed")?;
    let home_real = std::fs::canonicalize(home).map_err(|_| "home-canonicalize-failed")?;
    if real == home_real {
        return Err("is-home");
    }
    if !real.starts_with(&home_real) {
        return Err("outside-home");
    }
    if PROTECTED_ROOTS.iter().any(|r| Path::new(r) == real) {
        return Err("protected-root");
    }
    Ok(())
}

fn push_quarantine(items: &mut Vec<PlanItem>, path: PathBuf, label: &str) {
    push_item(items, path, label, false, false);
}

/// GUI 웹층 전용 — 실패해도 부분 실패로 세지 않는 이연 등급(best_effort). PlanItem 주석 참조.
fn push_best_effort(items: &mut Vec<PlanItem>, path: PathBuf, label: &str) {
    push_item(items, path, label, true, false);
}

/// 사용자 폴더(프로젝트 작업기억 등) — 모달이 경로를 **반드시** 노출해야 하는 등급(P0-2).
fn push_outside(items: &mut Vec<PlanItem>, path: PathBuf, label: &str) {
    push_item(items, path, label, false, true);
}

fn push_item(
    items: &mut Vec<PlanItem>,
    path: PathBuf,
    label: &str,
    best_effort: bool,
    outside_state: bool,
) {
    let size_bytes = dir_size(&path);
    items.push(PlanItem {
        path,
        label: label.to_string(),
        size_bytes,
        action: Action::Quarantine,
        best_effort,
        outside_state,
    });
}

/// `_round` 후보를 **격리 대상**과 **보고 대상**으로 나눈다.
///
/// ★P0-3 판정(시뮬레이션 2026-08-16): 종전엔 ACTIVE_PROJECT 포인터가 가리키는 **사용자
/// 프로젝트 폴더 안의 _round** 까지 말없이 격리했다. 두 가지가 동시에 깨졌다:
///  ① 격리본은 `cys-dept reap` 이 14일 뒤 `rm -rf` 하므로 **동의 없이 옮긴 사용자 저작물이
///     영구 소거**된다(생초보 가이드는 "일반 폴더 파일은 안 지워진다"고 단언한다).
///  ② 같은 워크스페이스 안의 다른 프로젝트 _round 는 남는데 왜 이것만 지워지는지 설명 불가.
/// → **기본 격리는 $HOME/_round 와 <workspace>/_round 둘뿐**. 프로젝트 _round 는 순회해서
/// 찾되 `report_only` 로 "남습니다 — 직접 정리하세요"라고 **고지**만 한다. 전면 격리는
/// `--purge-round` opt-in. `~/.cys/local` 을 기본 보존으로 되돌린 A8 판정과 같은 논리다.
///
/// 반환: (격리 후보, 보고 후보). 어느 쪽이든 **_round 디렉토리 자체**만 다루며 작업 폴더
/// 본체는 어떤 경우에도 대상이 아니다(D1a 실사고 교훈).
fn round_candidates(roots: &ResetRoots, purge_round: bool) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let core = vec![roots.home.join("_round"), roots.workspace_root.join("_round")];
    let mut project: Vec<PathBuf> = Vec::new();

    // ACTIVE_PROJECT 포인터가 지목한 프로젝트.
    for base in &core {
        if let Ok(s) = std::fs::read_to_string(base.join("ACTIVE_PROJECT")) {
            if let Some(line) = s.lines().next().map(str::trim).filter(|l| !l.is_empty()) {
                project.push(PathBuf::from(line).join("_round"));
            }
        }
    }
    // 워크스페이스 직하 1단계 순회 — "남는 것"을 빠짐없이 고지하기 위한 발견(격리 아님).
    if let Ok(rd) = std::fs::read_dir(&roots.workspace_root) {
        let mut kids: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path().join("_round"))
            .filter(|p| p.is_dir())
            .collect();
        kids.sort();
        project.extend(kids);
    }

    let dedup = |v: Vec<PathBuf>, seen: &mut Vec<PathBuf>| -> Vec<PathBuf> {
        let mut out = Vec::new();
        for b in v {
            let key = std::fs::canonicalize(&b).unwrap_or_else(|_| b.clone());
            if !seen.contains(&key) {
                seen.push(key);
                out.push(b);
            }
        }
        out
    };
    let mut seen: Vec<PathBuf> = Vec::new();
    let core = dedup(core, &mut seen);
    let project = dedup(project, &mut seen);
    if purge_round {
        let mut all = core;
        all.extend(project);
        (all, Vec::new())
    } else {
        (core, project)
    }
}

/// 격리 목적지 루트가 **지금 쓸 수 있는가**(디렉토리인가·생성 가능한가·쓰기 가능한가).
///
/// ★P0-6: 종전엔 `create_dir_all` 이 데몬 전멸·launchd 해제가 **끝난 뒤** 첫 동작이라,
/// `~/.local/state/cys-trash` 가 파일인 기계에서는 시스템만 더 망가뜨리고 실패했다.
/// 계획 단계(쓰기 0 프리뷰)에서 미리 판정해 **모달을 띄우기 전에** 거부한다.
/// 쓰기 가능 여부는 임시 파일 생성·삭제로 실측한다(권한만 보면 ACL·읽기전용 볼륨을 놓친다).
fn probe_trash_root(trash_root: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(trash_root) {
        Ok(m) if m.file_type().is_dir() => {}
        Ok(_) => {
            return Err(format!(
                "{} 가 디렉토리가 아니다(파일·심링크) — 그 항목을 옮기거나 지운 뒤 다시 시도하라",
                trash_root.display()
            ))
        }
        Err(_) => {
            std::fs::create_dir_all(trash_root)
                .map_err(|e| format!("{} 생성 실패: {e}", trash_root.display()))?;
        }
    }
    let probe = trash_root.join(format!(".write-probe-{}", std::process::id()));
    std::fs::write(&probe, b"probe").map_err(|e| {
        format!("{} 에 쓸 수 없다: {e} — 권한·디스크 상태를 확인하라", trash_root.display())
    })?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// 쓰기 0 프리뷰 — 파일시스템을 읽기만 하고 계획을 만든다.
pub fn build_plan(roots: &ResetRoots, opts: &ResetOptions) -> ResetPlan {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let trash_dir = roots.trash_root.join(format!("factory-reset-{stamp}"));
    let mut quarantine: Vec<PlanItem> = Vec::new();
    let mut keep: Vec<PlanItem> = Vec::new();
    let mut report_only: Vec<String> = Vec::new();

    // ── ~/.cys 직하: 알려진 항목만 격리, 미등록은 보존(오너 배치 추정) ──
    if let Ok(rd) = std::fs::read_dir(&roots.cys_base) {
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let is_license = LICENSE_FILES.contains(&name.as_str());
            let is_local = name == LOCAL_OVERLAY;
            // ★P2-2 ④: `claude-` 접두는 **계정 격리 디렉토리**(claude-<슬러그>)를 겨냥한 규칙이다.
            // 파일에까지 적용하면 사용자가 둔 `claude-notes.md`·`claude-key.env` 가 "미등록 파일
            // 보존" 규칙을 건너뛰고 격리된다 — 보존 계약이 접두 규칙에 지는 역전이었다.
            let is_dir = path.is_dir();
            let known = CYS_BASE_EXACT.contains(&name.as_str())
                || CYS_BASE_EXACT2.contains(&name.as_str())
                || CYS_BASE_PREFIX
                    .iter()
                    .any(|p| name.starts_with(p) && (is_dir || !p.ends_with('-')));
            if is_license {
                if opts.purge_license {
                    push_quarantine(&mut quarantine, path, "라이선스(--purge-license)");
                } else {
                    keep.push(PlanItem {
                        size_bytes: dir_size(&path),
                        path,
                        label: "라이선스 — 기본 보존".into(),
                        action: Action::Keep,
                        best_effort: false,
                        outside_state: false,
                    });
                }
            } else if is_local {
                if opts.purge_local {
                    push_quarantine(&mut quarantine, path, "사용자 오버레이(--purge-local)");
                } else {
                    keep.push(PlanItem {
                        size_bytes: dir_size(&path),
                        path,
                        label: "직접 만든 지침·스킬·훅 오버레이 — 기본 보존(--purge-local 로 격리)".into(),
                        action: Action::Keep,
                        best_effort: false,
                        outside_state: false,
                    });
                }
            } else if known {
                push_quarantine(&mut quarantine, path, "cys 상태·팩·조직");
            } else {
                keep.push(PlanItem {
                    size_bytes: dir_size(&path),
                    path,
                    label: "미등록 파일(사용자 배치 추정) — 보존".into(),
                    action: Action::Keep,
                    best_effort: false,
                    outside_state: false,
                });
            }
        }
    }

    // ── ~/.local/state: 메인 + 부서 데몬 상태(등록·고아 불문). cys-trash 는 목적지라 제외 ──
    if let Ok(rd) = std::fs::read_dir(&roots.state_root) {
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == "cys" || name.starts_with("cys-dept-") {
                push_quarantine(&mut quarantine, e.path(), "데몬 상태(대화기억·DB 포함)");
            }
        }
    }

    // ── 작업기억 _round — 격리(핵심 2곳)와 보고(사용자 프로젝트)로 분리 · P0-3 판정 ──
    let (round_quarantine, round_report) = round_candidates(roots, opts.purge_round);
    for cand in round_quarantine {
        let outside = !cand.starts_with(&roots.home.join("_round"))
            && !cand.starts_with(&roots.cys_base)
            && !cand.starts_with(&roots.state_root);
        match quarantine_eligible(&cand, &roots.home) {
            Ok(()) if outside => push_outside(&mut quarantine, cand, "작업기억(_round)"),
            Ok(()) => push_quarantine(&mut quarantine, cand, "작업기억(_round)"),
            Err("absent") => {}
            Err(reason) => report_only.push(format!(
                "{} — 자동 격리 부적격({reason}) · 필요 시 수동 정리",
                cand.display()
            )),
        }
    }
    for cand in round_report {
        if cand.is_dir() {
            report_only.push(format!(
                "{} — 프로젝트 작업기억은 **남습니다**(직접 정리). 여기 남은 SESSION_STATE.md·\
                 tasks 가 새 조직과 충돌할 수 있으니 새로 시작하려면 이 폴더를 지우세요. \
                 전부 함께 지우려면 --purge-round",
                cand.display()
            ));
        }
    }

    // ── macOS GUI 층 ──
    if let Some(lib) = &roots.library {
        for rel in [
            "WebKit/com.cysjavis.terminal",
            "Caches/com.cysjavis.terminal",
            "Preferences/com.cysjavis.terminal.plist",
        ] {
            let p = lib.join(rel);
            if p.exists() {
                push_best_effort(&mut quarantine, p, "GUI 저장값(화면·캐시)");
            }
        }
    }
    if let Some(dc) = &roots.darwin_cache {
        let p = dc.join("com.cysjavis.terminal");
        if p.exists() {
            push_best_effort(&mut quarantine, p, "GUI 캐시(Darwin)");
        }
    }

    // ── Windows 층 ──
    // ★%LOCALAPPDATA%\cys 는 **앱 설치 디렉토리와 같은 곳**이다(WIN_STATE_* 주석 참조).
    // 통째로 옮기면 앱을 언인스톨한다 — 알려진 상태 항목만 골라 격리하고 설치본은 보존한다.
    if let Some(root) = &roots.win_local_state {
        if let Ok(rd) = std::fs::read_dir(root) {
            let mut entries: Vec<_> = rd.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let name = e.file_name().to_string_lossy().into_owned();
                let known = WIN_STATE_EXACT.contains(&name.as_str())
                    || WIN_STATE_PREFIX.iter().any(|p| name.starts_with(p));
                if known {
                    push_quarantine(&mut quarantine, e.path(), "데몬 상태(Windows)");
                } else {
                    keep.push(PlanItem {
                        size_bytes: 0, // 설치본 크기는 세지 않는다(대상이 아니므로 계산도 낭비).
                        path: e.path(),
                        label: "앱 설치 파일 — 보존(초기화 대상 아님)".into(),
                        action: Action::Keep,
                        best_effort: false,
                        outside_state: false,
                    });
                }
            }
        }
    }
    // WebView2 데이터는 별도 디렉토리라 통째 격리 가능하나, 실행 중 앱이 점유하므로 이연 등급.
    if let Some(p) = &roots.win_webview_data {
        if p.exists() {
            push_best_effort(&mut quarantine, p.clone(), "GUI 저장값(WebView)");
        }
    }

    // ── 외부 등록 해제 대상(격리가 아니라 외과 제거) ──
    let mut strip_settings = Vec::new();
    let mut strip_skill_dirs = Vec::new();
    // ★P1-1 ④: 등록기(javis_preflight)는 `CLAUDE_CONFIG_DIR` 을 **최우선** 대상으로 훅을 심는데
    // 해제 대상 열거는 `~/.claude*` 뿐이라, 그 프로필의 훅만 살아남아 깨진 채 남았다.
    // ~/.cys 안(=격리로 함께 사라지는 격리형 프로필)은 제외한다 — 파일째 없어지므로 무의미.
    let mut profile_dirs = crate::pack::personal_profile_dirs_under(&roots.home);
    if let Ok(v) = std::env::var("CLAUDE_CONFIG_DIR") {
        let p = PathBuf::from(&v);
        if !v.is_empty() && p.is_dir() && !p.starts_with(&roots.cys_base) && !profile_dirs.contains(&p)
        {
            profile_dirs.push(p);
        }
    }
    for dir in profile_dirs {
        let s = dir.join("settings.json");
        if s.exists() {
            strip_settings.push(s);
        }
        let sk = dir.join("skills");
        if sk.is_dir() {
            strip_skill_dirs.push(sk);
        }
    }

    // ── $TMPDIR 캐시 소거 대상 ──
    let mut temp_sweep = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&roots.temp) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if TEMP_SWEEP_PREFIX.iter().any(|p| name.starts_with(p))
                || name.starts_with(".cys-guard-timeout-absent-")
            {
                temp_sweep.push(e.path());
            }
        }
    }

    // ★P1-1 ③: 프로필에 남아 있는 옛 백업들 — cys 훅 전문이 들어 있어 되돌리면 죽은 훅이
    // 부활한다. 우리가 만든 게 아니므로 지우지 않고 존재를 알린다.
    {
        let mut stale_backups = 0usize;
        for dir in crate::pack::personal_profile_dirs_under(&roots.home) {
            if let Ok(rd) = std::fs::read_dir(&dir) {
                stale_backups += rd
                    .flatten()
                    .filter(|e| {
                        let n = e.file_name().to_string_lossy().into_owned();
                        n.starts_with("settings.json.bak") || n == "settings.json.cys-lock"
                    })
                    .count();
            }
        }
        if stale_backups > 0 {
            report_only.push(format!(
                "~/.claude*/ 에 cys 훅이 든 옛 settings 백업 {stale_backups}개가 남아 있습니다 — \
                 되돌리면 죽은 훅이 부활하니 필요 없으면 지우세요(settings.json.bak*)"
            ));
        }
    }

    // ── 보고만(자동 수정 금지) ──
    let zshrc = roots.home.join(".zshrc");
    if std::fs::read_to_string(&zshrc)
        .map(|s| s.contains("cys"))
        .unwrap_or(false)
    {
        report_only.push(format!(
            "{} 에 cys 관련 줄이 있다(오너 저작 alias 등) — 자동 수정하지 않음",
            zshrc.display()
        ));
    }
    // ★P2-2 ③: `--purge-local` 이면 `~/.cys/local/skills` 를 가리키는 **외부 심링크**가 통째로
    // 끊긴다(실측: ~/.codex/skills 아래 다수). 우리가 만든 링크가 아니라 지우지 않지만, 끊길
    // 개수를 **미리 세어 알린다** — 모르고 실행하면 다른 도구의 스킬 목록이 조용히 깨진다.
    if opts.purge_local {
        let needle = format!(
            "{}/{LOCAL_OVERLAY}",
            roots.cys_base.to_string_lossy().replace('\\', "/")
        );
        let mut dangling = 0usize;
        let mut roots_hit: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&roots.home) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !(name.starts_with(".codex") || name.starts_with(".gemini") || name.starts_with(".agents")) {
                    continue;
                }
                let skills = e.path().join("skills");
                let mut n = 0usize;
                if let Ok(links) = std::fs::read_dir(&skills) {
                    for l in links.flatten() {
                        if std::fs::read_link(l.path())
                            .map(|t| t.to_string_lossy().replace('\\', "/").contains(&needle))
                            .unwrap_or(false)
                        {
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    dangling += n;
                    roots_hit.push(format!("{} ({n}개)", skills.display()));
                }
            }
        }
        if dangling > 0 {
            report_only.push(format!(
                "--purge-local 로 ~/.cys/local 을 격리하면 다른 도구의 스킬 링크 {dangling}개가 \
                 끊깁니다: {} — 링크는 지우지 않으니 필요하면 직접 정리하세요",
                roots_hit.join(", ")
            ));
        }
    }

    // ★P2-2 ①: gemini 도 cys 경로를 신뢰 목록에 담을 수 있다 — codex 만 알리고 gemini 는
    // 침묵하던 비대칭을 없앤다(둘 다 서드파티 설정이라 자동 수정은 하지 않는다).
    for rel in [".gemini/settings.json", ".gemini/config/skills.json"] {
        let g = roots.home.join(rel);
        if std::fs::read_to_string(&g)
            .map(|s| s.contains(".cys") || s.contains("CYSjavis"))
            .unwrap_or(false)
        {
            report_only.push(format!(
                "{} 에 cys 경로 항목이 있다(서드파티 설정) — 자동 수정하지 않음",
                g.display()
            ));
        }
    }
    let codex = roots.home.join(".codex/config.toml");
    if std::fs::read_to_string(&codex)
        .map(|s| s.contains(".cys") || s.contains("CYSjavis"))
        .unwrap_or(false)
    {
        report_only.push(format!(
            "{} 에 cys 경로 신뢰 항목이 있다(서드파티 설정) — 자동 수정하지 않음",
            codex.display()
        ));
    }
    report_only.push(
        "프로젝트 폴더 안의 CLAUDE.md·.mcp.json·.vibecoding 은 작업 폴더 불가침 원칙으로 \
         건드리지 않는다 — 필요 시 해당 프로젝트에서 직접 삭제"
            .into(),
    );

    // ★P2-2 ⑤: 기존 격리 보관본(과거 부서 purge·이전 리셋)은 **보존**이 설계인데 화면 어디에도
    // 나오지 않아, 사용자는 유령 부서 이름이 디스크에 남은 걸 나중에 발견하고 놀랐다.
    if let Ok(rd) = std::fs::read_dir(&roots.trash_root) {
        let mut prior: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        prior.sort();
        for p in prior {
            let size = dir_size(&p);
            keep.push(PlanItem {
                label: "기존 격리 보관본 — 보존(직접 지우려면 이 폴더를 삭제)".into(),
                path: p,
                size_bytes: size,
                action: Action::Keep,
                best_effort: false,
                outside_state: false,
            });
        }
    }

    let launchd_plist = roots
        .library
        .as_ref()
        .map(|l| l.join("LaunchAgents/com.cysjavis.cysd.plist"))
        .filter(|p| p.exists());

    ResetPlan {
        stamp,
        trash_dir,
        quarantine,
        keep,
        report_only,
        strip_settings,
        strip_skill_dirs,
        temp_sweep,
        launchd_plist,
        purge_license: opts.purge_license,
        purge_local: opts.purge_local,
        purge_round: opts.purge_round,
        trash_root_ready: probe_trash_root(&roots.trash_root),
        interrupted_prior: interrupted_prior_resets(&roots.trash_root),
    }
}

/// 이전 초기화가 중단된 흔적 — `manifest.json` 이 없는 `factory-reset-*` 격리 폴더.
/// ★P0-5: 중단 후 재실행은 **새 폴더**를 파므로 사용자는 두 폴더 중 어디서 복구할지 모른다.
/// 프리뷰·완료 화면 양쪽에 이 목록을 노출하는 것이 그 혼선의 단일 해법이다.
fn interrupted_prior_resets(trash_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(trash_root) {
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .map(|n| n.to_string_lossy().starts_with("factory-reset-"))
                        .unwrap_or(false)
                    && !p.join("manifest.json").exists()
            })
            .collect();
        dirs.sort();
        out = dirs;
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// 리셋 센티널 — "지금 초기화 중이니 데몬을 새로 띄우지 마라"는 프로세스 간 신호.
//
// ★ABSOLUTE ANCHOR 방어 설계(부트 체인 불가침): 이 파일이 남으면 데몬이 영영 못 뜨는
// **전 pane 사망 등급** 사고가 된다. 그래서 판정은 **fail-open** 이다 —
//   ① TTL(15분) 경과 → 무시(+정리)  ② 기록한 pid 가 죽었으면 → 무시(+정리)
// 즉 리셋이 중간에 죽어도 다음 기동은 정상이며, "차단"은 살아있는 리셋 프로세스가
// 실제로 진행 중인 짧은 창에서만 성립한다. cysd 기동 자체에는 이 검사를 넣지 않는다
// (부트 체인 최소 침습) — 실제 스폰 경로인 CLI autostart·GUI ensure_daemon 만 막는다.
// ─────────────────────────────────────────────────────────────────────────────

/// 센티널 TTL — 리셋 1회(정지 폴링 12s + 격리)의 상한을 넉넉히 덮되, 사고 시 자동 해제되는 값.
const SENTINEL_TTL_SECS: u64 = 900;

/// 센티널 경로 오버라이드 env — **테스트·진단 전용**.
///
/// ★왜 존재하는가(2026-08-24 · CI 전량 배선의 선행 조건): 센티널 회귀 테스트는 판정을
/// 시험하려고 센티널을 **쓰고 지운다**. 경로가 `$HOME/.local/state/…` 로 고정돼 있으면 그
/// 테스트가 두 가지를 동시에 깨뜨린다 —
///   ① 같은 기계에서 러너가 둘 이상 돌면(로컬 cargo test ⇄ CI 잡 ⇄ 다른 워커) **같은 파일**을
///      서로 쓰고 지워 무작위 적색이 난다. 결정론 게이트가 비결정론의 원천이 되는 형태다.
///   ② 진짜 완전 초기화가 진행 중이면 테스트가 **살아 있는 가드를 지운다**. 그 가드의 존재
///      이유가 "리셋 중 데몬 부활 = 전 pane 사망 등급" 차단이므로, 이건 테스트가 제품의
///      최고 위험 등급 안전장치를 무력화하는 것이다.
/// ∴ 경로를 주입 가능하게 만들고 테스트는 임시 디렉터리만 쓴다(사용자 홈 무접촉).
///
/// ⚠**프로덕션은 설정하지 않는다.** 쓰는 쪽(`ResetSentinel::arm` = 리셋 실행 프로세스)과 읽는
/// 쪽(`reset_in_progress` = CLI autostart·GUI ensure_daemon)이 **서로 다른 프로세스**라,
/// 한쪽에만 이 값이 있으면 두 프로세스가 다른 파일을 보고 가드가 조용히 무력해진다. 그래서
/// ⓐ 상대경로·빈 값은 **무시**하고 기본 경로로 되돌아가며(오타로 가드를 잃지 않는다)
/// ⓑ 이 스위치는 어떤 제품 경로에서도 설정되지 않는다(설정 지점은 테스트뿐).
pub const ENV_RESET_SENTINEL: &str = "CYS_FACTORY_RESET_SENTINEL";

/// 센티널 경로(홈 파생 — 데몬 상태 루트와 같은 부모라 리셋 대상과 함께 사라지지 않는다).
/// `CYS_FACTORY_RESET_SENTINEL`(절대경로)이 있으면 그쪽을 쓴다 — 위 상수 문서 참조.
pub fn sentinel_path() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os(ENV_RESET_SENTINEL) {
        let p = PathBuf::from(v);
        // 절대경로만 인정한다 — 상대경로는 프로세스 cwd 에 따라 갈려서 쓰는 쪽과 읽는 쪽이
        // 다른 파일을 보게 된다(가드 무력화). 빈 값·상대경로는 조용히 기본 경로로 되돌린다.
        if p.is_absolute() {
            return Some(p);
        }
    }
    Some(
        dirs::home_dir()?
            .join(".local/state")
            .join(".cys-factory-reset-in-progress"),
    )
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    pid != 0 && unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
#[cfg(not(unix))]
fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]), true);
    sys.process(sysinfo::Pid::from_u32(pid)).is_some()
}

/// 센티널 기록(형식: `<unix_ts> <pid>`). 실패는 무시 — 센티널은 **보조** 방어층이고,
/// 이걸 못 써서 리셋 자체가 막히면 안 된다(주 방어는 정지 실측 + quiescent 게이트).
/// 이 pid 가 cys 계열 프로세스인가(리셋을 실행할 수 있는 주체) — 센티널 소유자 검증용.
fn pid_is_cys_family(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]), true);
    sys.process(sysinfo::Pid::from_u32(pid))
        .and_then(|p| p.name().to_str().map(|s| s.to_ascii_lowercase()))
        .map(|b| {
            let b = b.rsplit(['/', '\\']).next().unwrap_or(&b).to_string();
            b == "cys" || b == "cys.exe" || b == "cys-app" || b == "cys-app.exe"
        })
        .unwrap_or(false)
}

/// 부팅 식별자 — 재부팅하면 값이 바뀐다. 센티널이 재부팅을 건너 살아남는 것을 막는다.
fn boot_id() -> u64 {
    sysinfo::System::boot_time()
}

fn write_sentinel() {
    if let Some(p) = sentinel_path() {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(
            &p,
            format!("{} {} {}", now_unix(), std::process::id(), boot_id()),
        );
    }
}

/// 센티널 해제(성공·실패 무관 반드시 호출 — ResetSentinel 가드가 Drop 에서 보장).
fn clear_sentinel() {
    if let Some(p) = sentinel_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// 센티널 RAII 가드 — 패닉·조기 return 에도 해제를 보장한다(잔존 = 부트 불능 위험).
pub struct ResetSentinel;
impl ResetSentinel {
    pub fn arm() -> Self {
        write_sentinel();
        ResetSentinel
    }
}
impl Drop for ResetSentinel {
    fn drop(&mut self) {
        clear_sentinel();
    }
}

/// 지금 다른 프로세스가 완전 초기화를 진행 중인가 — 데몬 스폰 직전에 묻는 질문.
/// **fail-open**: 파일 없음·형식 불량·TTL 초과·기록 pid 사망은 전부 false(+ 잔재 정리).
pub fn reset_in_progress() -> bool {
    reset_in_progress_with(&|pid| pid_alive(pid) && pid_is_cys_family(pid))
}

/// 위 함수의 판정자 주입판 — 테스트가 "리셋 주인이 살아있다"를 흉내낼 수 있게 한다
/// (테스트 바이너리의 프로세스 이름은 cys 계열이 아니므로 라이브 판정자로는 검증 불가).
pub fn reset_in_progress_with(owner_alive: &dyn Fn(u32) -> bool) -> bool {
    let Some(p) = sentinel_path() else {
        return false;
    };
    let Ok(body) = std::fs::read_to_string(&p) else {
        return false;
    };
    let mut it = body.split_whitespace();
    let ts = it.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let pid = it.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    // ★부팅 식별자(선택 필드 — 구 형식 호환): 재부팅 후에는 **같은 pid 가 다른 프로세스**일 수
    // 있고, 그때 센티널을 살아있다고 오판하면 데몬이 최대 TTL 동안 못 뜬다(전 pane 사망 등급).
    let recorded_boot = it.next().and_then(|v| v.parse::<u64>().ok());
    // ★허용오차 필수(감사 확정): Windows 의 boot_time 은 tick 파생이라 호출마다 초 단위로
    // 흔들린다. 정확 일치를 요구하면 **같은 부팅인데 불일치**로 판정해 살아있는 센티널을
    // 삭제하고 가드를 통째로 무력화한다. 재부팅은 분 단위로 값이 달라지므로 120초 창이면
    // "다른 부팅"만 정확히 걸러낸다.
    const BOOT_SKEW_TOLERANCE: u64 = 120;
    let same_boot = recorded_boot
        .map(|b| b.abs_diff(boot_id()) <= BOOT_SKEW_TOLERANCE)
        .unwrap_or(true);
    let fresh = ts > 0 && now_unix().saturating_sub(ts) < SENTINEL_TTL_SECS;
    if fresh && owner_alive(pid) && same_boot {
        return true;
    }
    let _ = std::fs::remove_file(&p); // 만료·고아 센티널은 즉시 청소(자기잠금 방지).
    false
}

// ─────────────────────────────────────────────────────────────────────────────
// 실행 1단계 — 정지·등록 해제 (부수효과 크므로 테스트에서 호출하지 않는다)
// ─────────────────────────────────────────────────────────────────────────────

/// Windows GUI 프로세스에서 자식 콘솔 창이 뜨지 않게 한다(CREATE_NO_WINDOW 상당).
///
/// ★N4 편입(2026-08-24) — 종전엔 이 자리에서 그 flag 값(0x0800_0000)을 **직접** 얹었고,
/// 그래서 U-7 의 규약("프로덕션의 자식 분리·콘솔 정책은 [`crate::SpawnPolicy`] 하나만
/// 경유한다") **밖**에 있었다. 규약을 지키는 소스 핀의 스캔 목록이 화이트리스트였던 탓에
/// 이 파일은 목록 밖이었고 — "화이트리스트 관리 자체가 결함원" 이라는 그 핀의 진단이 옳았음을
/// **트리의 실물 위반이 증명한 자리**가 여기다.
///
/// 지금은 등급 [`crate::ChildLifetime::Attached`] 로 위임한다. 값·행동은 종전과 같다
/// (`Attached` = 분리 없음 + Windows 콘솔 창 은폐). 등급 선택의 근거도 그대로다:
/// 이 헬퍼의 세 호출부(schtasks · taskkill 2곳)는 전부 바로 뒤에서 `output()` 으로 끝까지
/// 기다리는 **유계 자식**이라 떼면 안 된다.
#[cfg(windows)]
fn no_console_win(cmd: &mut std::process::Command) {
    use crate::SpawnPolicy as _;
    cmd.spawn_policy(crate::ChildLifetime::Attached);
}

/// 이름이 정확히 `cysd` 인 전 프로세스 pid 수집(doctor `pid_is_cysd` 판정 기준 공유 —
/// macOS comm 15자 절단으로 `pkill -x` 가 못 잡는 함정을 sysinfo 로 회피).
fn scan_cysd_pids() -> Vec<u32> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    // ★W4: Windows 는 세대교체 잔재가 `cysd.prev.exe`·`cysd.prev2.exe` 로 남아 직접 기동될 수
    // 있다(GUIDE-clean-reset-KR.md 의 taskkill 목록이 그 실재를 증언한다). 이름 판정을 그 계열까지
    // 넓힌다 — 놓치면 "전멸 확인"이 거짓이 되고 살아있는 데몬 밑에서 DB 를 옮기게 된다.
    let is_cysd = |base: &str| -> bool {
        if cfg!(windows) {
            let b = base.to_ascii_lowercase();
            b == "cysd.exe" || (b.starts_with("cysd.prev") && b.ends_with(".exe"))
        } else {
            base == "cysd"
        }
    };
    sys.processes()
        .iter()
        .filter(|(_, p)| {
            p.name()
                .to_string_lossy()
                .rsplit(['/', '\\'])
                .next()
                .map(is_cysd)
                .unwrap_or(false)
        })
        .map(|(pid, _)| pid.as_u32())
        .collect()
}

/// ★P0-6: 정지 단계가 이미 남긴 **비가역 부수효과**를 사용자에게 알리는 단일 문구.
/// 실패·중단 경로 전부(CLI stderr·GUI 토스트)가 이 문장을 쓴다 — "실패했으니 원래대로겠지"라는
/// 치명적 오해를 막는다(실제로는 세션이 전멸했고 자동시작 등록이 해제됐다).
pub fn stop_side_effects_note() -> &'static str {
    "⚠ 데몬은 이미 정지되었고(열려 있던 세션 전부 종료) 자동시작 등록이 해제/비활성되었습니다. \
     격리는 진행되지 않아 데이터는 그대로입니다. 되살리려면: 앱을 다시 실행하거나 \
     `cys daemon install` (Windows 에서 작업이 비활성으로 남았다면 `schtasks /Change /TN cysd /ENABLE`)"
}

/// 격리가 성공한 뒤에만 plist 파일을 지운다(P0-6 — 실패 시 자동시작을 잃지 않게).
/// bootout 은 이미 stop 단계에서 끝났으므로 여기서는 파일 정리뿐이다.
fn finalize_launchd_removal(plan: &ResetPlan) {
    #[cfg(target_os = "macos")]
    if let Some(plist) = &plan.launchd_plist {
        let _ = std::fs::remove_file(plist);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = plan;
}

/// launchd/schtasks 등록 해제 + cysd 전멸(TERM→대기→KILL→실측 확인).
/// ★순서가 생명: KeepAlive 등록을 먼저 끊지 않으면 kill 직후 launchd 가 되살린다
/// (daemon install --takeover 와 GUIDE-clean-reset-KR.md §맥 2단계 ①과 동일 근거).
/// 반환: 종료시킨 프로세스 수. 전멸 실패는 Err — 호출부는 격리로 진행하면 안 된다.
pub fn stop_daemons_and_unregister(
    plan: &ResetPlan,
    progress: &mut dyn FnMut(&str, &str),
) -> Result<usize, String> {
    // ★P0-1: 센티널 무장은 **호출부의 `ResetSentinel::arm()`** 책임이다(여기서 직접 쓰지 않는다).
    // 종전엔 이 자리에서 write_sentinel() 을 불러 해제자가 아무 데서도 안 돌았고, 리셋이 실패하면
    // 데몬을 최대 900초 못 살렸으며 성공해도 센티널 파일이 남아 "설치 초기 상태" 계약을 깼다.
    // RAII 가드는 조기 return·패닉에도 Drop 으로 반드시 해제된다.
    debug_assert!(reset_in_progress(), "호출부가 ResetSentinel::arm() 을 걸어야 한다");
    // ★A2(성찰 확정 2026-08-16): bootout 은 **라벨 대상**이라 plist 파일 존재와 무관하게
    // 항상 실행해야 한다. 종전엔 전 블록이 `if let Some(plist)`(=파일 존재) 안에 있어,
    // 사용자가 과거 수동 정리로 plist 만 지운 기계(가이드 절차 부분 수행)에서 **적재된 잡이
    // 그대로 남고** KeepAlive 가 kill 직후 cysd 를 되살렸다(ThrottleInterval 10s > 폴링창).
    // 해제 뒤 `launchctl list` 로 **미적재를 실측**하는 것이 kill 진입의 하드 게이트다.
    #[cfg(target_os = "macos")]
    {
        progress("stop", "launchd 등록 해제(라벨 대상)");
        let uid = unsafe { libc::getuid() };
        let _ = std::process::Command::new("launchctl")
            .arg("bootout")
            .arg(format!("gui/{uid}/{}", crate::launchd::LAUNCHD_LABEL))
            .output();
        if let Some(plist) = &plan.launchd_plist {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", "-w"])
                .arg(plist)
                .output();
            // ★P0-6: plist **파일 삭제는 여기서 하지 않는다**. bootout 으로 미적재가 확정되므로
            // KeepAlive 부활은 이미 막혔고, 파일까지 지운 뒤 격리가 실패하면 사용자는 자동시작
            // 등록을 잃은 채 "실패했으니 원래대로겠지"라고 오해한다. 삭제는 격리 성공 후
            // (finalize_launchd_removal) — 실패 시엔 다음 로그인에 자동 복구된다.
        }
        // 해제 실측 — 적재가 남아 있으면 KeepAlive 부활이 확정적이므로 진행하지 않는다.
        // (bootout 은 비동기 반영 여지가 있어 짧게 폴링한다.)
        let mut unloaded = false;
        for _ in 0..20 {
            if !crate::launchd::is_loaded() {
                unloaded = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if !unloaded {
            return Err(format!(
                "launchd 잡({})이 여전히 적재 중 — KeepAlive 가 데몬을 되살리므로 격리를 진행하지 \
                 않는다. 수동 해제 후 재시도: launchctl bootout gui/{uid}/{}",
                crate::launchd::LAUNCHD_LABEL,
                crate::launchd::LAUNCHD_LABEL
            ));
        }
    }
    #[cfg(windows)]
    {
        progress("stop", "작업 스케줄러 등록 해제");
        // ★W6: GUI(windowed cys-app)에서 콘솔 창이 깜박이지 않도록 CREATE_NO_WINDOW 를 건다
        // (리포 관례 — src-tauri no_console 와 같은 이유). 없으면 리셋 중 검은 창이 여러 번 뜬다.
        for args in [["/End", "/TN", "cysd"].as_slice(), ["/Delete", "/TN", "cysd", "/F"].as_slice()] {
            let mut c = std::process::Command::new("schtasks");
            c.args(args);
            no_console_win(&mut c);
            let _ = c.output();
        }
        let _ = plan; // macOS 전용 필드 사용에 대한 타깃별 unused 경고 억제(계약 아님·형태 유지).
    }

    let initial = scan_cysd_pids();
    let mut killed = 0usize;
    if !initial.is_empty() {
        // ★P0-6 문구 정직화: Windows 는 TERM 이 아니라 프로세스 트리 강제 종료다(taskkill /T /F) —
        // pane 자식(claude·python·git-bash)까지 즉사하므로 "정상 종료"라고 부르면 거짓말이다.
        let how = if cfg!(windows) {
            "프로세스 트리 강제 종료(미저장분 손실)"
        } else {
            "정상 종료 신호(TERM)"
        };
        progress("stop", &format!("cysd {}개 {how}", initial.len()));
        for pid in &initial {
            #[cfg(unix)]
            unsafe {
                // TERM = cysd shutdown_cleanup(스코프 프로세스 정리·소켓 제거) 경로.
                libc::kill(*pid as libc::pid_t, libc::SIGTERM);
            }
            #[cfg(windows)]
            {
                let mut c = std::process::Command::new("taskkill");
                c.args(["/PID", &pid.to_string(), "/T", "/F"]);
                no_console_win(&mut c);
                let _ = c.output();
            }
            killed += 1;
        }
        // 최대 8초 전멸 폴링 → 잔존자 KILL → 최대 4초 재확인.
        // ★P0-6: 이 구간이 무출력이면 사용자는 "멈췄다"고 오인한다 — 1초마다 남은 시간을 알린다.
        for i in 0..80 {
            if scan_cysd_pids().is_empty() {
                break;
            }
            if i % 10 == 0 {
                progress("stop", &format!("cysd 종료 대기 {}/8초", i / 10));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let stragglers = scan_cysd_pids();
        if !stragglers.is_empty() {
            progress("stop", &format!("잔존 {}개 강제 종료(KILL)", stragglers.len()));
            for pid in &stragglers {
                #[cfg(unix)]
                unsafe {
                    libc::kill(*pid as libc::pid_t, libc::SIGKILL);
                }
                #[cfg(windows)]
                {
                    let mut c = std::process::Command::new("taskkill");
                    c.args(["/PID", &pid.to_string(), "/T", "/F"]);
                    no_console_win(&mut c);
                    let _ = c.output();
                }
            }
            for _ in 0..40 {
                if scan_cysd_pids().is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    let remaining = scan_cysd_pids();
    if !remaining.is_empty() {
        return Err(format!(
            "cysd {}개가 종료되지 않았다(pid {:?}) — 격리를 진행하지 않는다. 수동 종료 후 재시도",
            remaining.len(),
            remaining
        ));
    }
    Ok(killed)
}

// ─────────────────────────────────────────────────────────────────────────────
// 실행 2단계 — 격리·해제·소거 (루트 주입으로 테스트 가능)
// ─────────────────────────────────────────────────────────────────────────────

/// 격리 진입 하드 게이트: 이 루트의 데몬 락(cys/cys.lock·cys-dept-*/cys.lock) 홀더 pid 가
/// 살아있는 cysd 면 거부. 전역 프로세스 스캔이 아니라 **루트 스코프**라, 다른 루트(테스트
/// 샌드박스)의 게이트가 라이브 데몬에 오탐하지 않는다. 판정자는 주입(deadman 관례).
pub fn verify_roots_quiescent(
    roots: &ResetRoots,
    pid_alive_and_cysd: &dyn Fn(u32) -> bool,
    any_cysd_running: &dyn Fn() -> bool,
) -> Result<(), String> {
    // ★W3(성찰 확정): 락파일 검사만으로는 **Windows 에서 게이트가 통째로 무력**이다 —
    // Windows cysd 의 싱글턴은 named pipe 선점이라 cys.lock 을 애초에 만들지 않고(cysd/main.rs
    // 의 lock 경로는 cfg(unix)), 상태 루트도 %LOCALAPPDATA%\cys 라 검사 경로와 다르다.
    // 그래서 OS 무관한 **프로세스 실측**을 게이트의 1차 조건으로 올린다(unix 에도 강화로 작용:
    // 락을 아직 못 잡은 기동 직후 cysd 도 잡힌다).
    if any_cysd_running() {
        return Err(
            "살아있는 cysd 프로세스가 있다 — 격리를 진행하지 않는다(정지 후 재시도). \
             GUI 앱이 떠 있으면 먼저 종료하라: 앱이 데몬을 자동으로 되살린다"
                .into(),
        );
    }
    let mut locks: Vec<PathBuf> = vec![roots.state_root.join("cys/cys.lock")];
    if let Ok(rd) = std::fs::read_dir(&roots.state_root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("cys-dept-") {
                locks.push(e.path().join("cys.lock"));
            }
        }
    }
    // Windows 상태 루트(%LOCALAPPDATA%\cys)의 슬러그 디렉토리도 같은 규약으로 훑는다.
    if let Some(win_root) = &roots.win_local_state {
        locks.push(win_root.join("cys.lock"));
        if let Ok(rd) = std::fs::read_dir(win_root) {
            for e in rd.flatten() {
                locks.push(e.path().join("cys.lock"));
            }
        }
    }
    for lock in locks {
        let Ok(s) = std::fs::read_to_string(&lock) else {
            continue;
        };
        if let Ok(pid) = s.trim().parse::<u32>() {
            if pid != 0 && pid_alive_and_cysd(pid) {
                return Err(format!(
                    "살아있는 cysd(pid {pid})가 {} 을 보유 중 — 정지 후 재시도",
                    lock.display()
                ));
            }
        }
    }
    Ok(())
}

/// GUI 앱(cys-app)이 떠 있는가 — CLI 리셋의 사전 거부 조건.
/// ★A3b(성찰 확정): 앱이 살아 있으면 그 앱의 부트 재시도 루프·재시작 버튼·drain 사이드카가
/// 리셋 도중 cysd 를 되살린다(앱 안의 JS 가드는 CLI 리셋을 알지 못한다). 센티널이 스폰을
/// 막아 주지만, 애초에 앱을 끄게 하는 편이 훨씬 단순하고 확실하다.
pub fn gui_app_running() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| {
        p.name()
            .to_string_lossy()
            .rsplit(['/', '\\'])
            .next()
            .map(|b| {
                let b = b.to_ascii_lowercase();
                // 번들 실행 파일명 실측: /Applications/cys.app/Contents/MacOS/cys-app (Windows: cys-app.exe).
                b == "cys-app" || b == "cys-app.exe"
            })
            .unwrap_or(false)
    })
}

/// 라이브 "cysd 가 하나라도 살아있는가" 판정자(프로덕션 기본) — quiescent 게이트 1차 조건.
pub fn live_any_cysd_running() -> bool {
    !scan_cysd_pids().is_empty()
}

/// 라이브 판정자(프로덕션 기본): pid 생존 + 프로세스명 cysd.
pub fn live_pid_is_cysd(pid: u32) -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]), true);
    sys.process(sysinfo::Pid::from_u32(pid))
        .map(|p| {
            p.name()
                .to_string_lossy()
                .rsplit(['/', '\\'])
                .next()
                .map(|b| b == "cysd" || b == "cysd.exe")
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

pub struct ResetReport {
    pub trash_dir: PathBuf,
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub failed: Vec<(PathBuf, String)>,
    pub kept: Vec<PathBuf>,
    /// 사람용 해제 내역("~/.claude/settings.json: 훅 5·statusLine·스킬링크 186" 식).
    pub stripped: Vec<String>,
    pub report_only: Vec<String>,
    pub temp_swept: usize,
    /// 격리 **도중** cysd 부활이 실측되면 경고 문구(정상 = None). 침묵 금지 계약.
    pub revived_warning: Option<String>,
    /// 실행 중 앱이 점유해 **이연**된 GUI 웹층 항목(부분 실패 아님 — 앱 종료 후 재실행 시 정리).
    pub deferred: Vec<(PathBuf, String)>,
    /// 계획에는 있었으나 실행 시점에 이미 없던 항목 수(재실행·수동 정리) — 예고 건수와
    /// 완료 건수의 차이를 설명하는 유일한 근거(P0-4).
    pub skipped_absent: usize,
    /// 복구 지도(manifest.json)를 실제로 썼는가. false 면 journal.ndjson 이 유일한 지도다.
    pub manifest_written: bool,
    /// 이전에 중단된 초기화의 격리 폴더(있으면 사용자가 두 폴더를 혼동한다 — 반드시 고지).
    pub interrupted_prior: Vec<PathBuf>,
    /// 격리 전에 `<trash_dir>/owner-settings/` 로 따로 복사해 둔 오너 판단 설정 파일명(P2-2 ②).
    pub owner_settings_saved: Vec<String>,
}

impl ResetReport {
    pub fn ok(&self) -> bool {
        self.failed.is_empty()
    }

    /// 완료 요약을 사람이 읽을 한 장으로 — `<trash_dir>/REPORT.txt` 로 영속되고 CLI·GUI 가
    /// 같은 문장을 인용한다(P0-4: 화면에서 사라져도 남는 단일 진실).
    pub fn human_report(&self) -> String {
        let mut o = String::new();
        o.push_str("cys 완전 초기화 결과
");
        o.push_str(&format!("격리 위치: {}
", self.trash_dir.display()));
        o.push_str(&format!(
            "이동 {}건 · 이미 없음 {}건 · 이연 {}건 · 실패 {}건
",
            self.moved.len(),
            self.skipped_absent,
            self.deferred.len(),
            self.failed.len()
        ));
        o.push_str(&format!(
            "복구 지도: {}
",
            if self.manifest_written {
                "manifest.json (없으면 journal.ndjson)"
            } else {
                "⚠ manifest.json 기록 실패 — journal.ndjson 이 유일한 지도"
            }
        ));
        if let Some(w) = &self.revived_warning {
            o.push_str(&format!("
⚠ {w}
"));
        }
        if !self.failed.is_empty() {
            o.push_str("
[정리되지 않은 항목 — 직접 확인 필요]
");
            for (p, e) in &self.failed {
                o.push_str(&format!("  {} : {e}
", p.display()));
            }
        }
        if !self.deferred.is_empty() {
            o.push_str("
[이연 — 앱이 사용 중이라 옮기지 못함]
");
            for (p, e) in &self.deferred {
                o.push_str(&format!("  {} : {e}
", p.display()));
            }
            o.push_str("  → 앱을 종료한 뒤 외부 터미널에서 `cys factory-reset` 을 한 번 더 실행하면 정리됩니다.
");
        }
        if !self.stripped.is_empty() {
            o.push_str("
[해제된 외부 등록]
");
            for s in &self.stripped {
                o.push_str(&format!("  {s}
"));
            }
        }
        if !self.kept.is_empty() {
            o.push_str("
[보존된 항목]
");
            for p in &self.kept {
                o.push_str(&format!("  {}
", p.display()));
            }
        }
        if !self.owner_settings_saved.is_empty() {
            o.push_str("\n[오너 설정 사본 — 재온보딩이 재생성하지 않는 파일]\n");
            o.push_str(&format!(
                "  {}/owner-settings/ : {}\n",
                self.trash_dir.display(),
                self.owner_settings_saved.join(", ")
            ));
            o.push_str("  → 예전 설정을 그대로 쓰려면 이 파일들을 ~/.cys/ 로 되돌리세요.\n");
        }
        if !self.report_only.is_empty() {
            o.push_str("
[자동 정리하지 않음 — 직접 확인하세요]
");
            for r in &self.report_only {
                o.push_str(&format!("  {r}
"));
            }
        }
        if !self.interrupted_prior.is_empty() {
            o.push_str("
[이전에 중단된 초기화 흔적 — 복구 지도는 각 폴더의 journal.ndjson]
");
            for p in &self.interrupted_prior {
                o.push_str(&format!("  {}
", p.display()));
            }
        }
        o.push_str("
복구: 이 폴더의 manifest.json(없으면 journal.ndjson)의 to→from 을 되돌리거나
");
        o.push_str("      `cys factory-reset --undo <이 폴더>` 를 실행하세요.
");
        o
    }
}

/// 격리·해제·소거 본체. **호출 전제**: stop_daemons_and_unregister 성공(또는 데몬이 원래
/// 없음). 내부에서 verify_roots_quiescent 를 하드 게이트로 한 번 더 실측한다.
pub fn execute_quarantine(
    plan: &ResetPlan,
    roots: &ResetRoots,
    pid_alive_and_cysd: &dyn Fn(u32) -> bool,
    any_cysd_running: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(&str, &str),
) -> Result<ResetReport, String> {
    verify_roots_quiescent(roots, pid_alive_and_cysd, any_cysd_running)?;

    std::fs::create_dir_all(&plan.trash_dir)
        .map_err(|e| format!("격리 디렉토리 생성 실패 {}: {e}", plan.trash_dir.display()))?;

    // ★J1(성찰 확정): 복구 지도를 **이동 전에** 한 줄씩 선기록한다(append-only journal).
    // 종전엔 전 항목 이동이 끝난 뒤에야 manifest 를 썼다 — 중도 사망(강제종료·전원·업데이트
    // 재시작)이면 이미 옮겨진 항목의 원위치를 잃는다. 목적지가 `NNN-<basename>` 이라 동명 항목
    // (_round 최대 3곳)은 지도 없이는 역산이 불가능하다. 리포의 저널-선기록 관례(.pack-journal)와
    // 같은 결. journal.ndjson 은 manifest.json 이 기록되면 그 부분집합이 되지만, 중도 사망 시
    // **유일한 복구 지도**로 남는다(그래서 지우지 않는다).
    let journal_path = plan.trash_dir.join("journal.ndjson");
    let mut journal = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&journal_path)
        .map_err(|e| format!("격리 저널 생성 실패 {}: {e}", journal_path.display()))?;

    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = Vec::new();
    let mut deferred: Vec<(PathBuf, String)> = Vec::new();
    let mut skipped_absent = 0usize; // ★P0-4: 예고 건수 vs 완료 건수 차이를 설명하기 위한 집계.
    for (i, item) in plan.quarantine.iter().enumerate() {
        if !item.path.exists() && std::fs::symlink_metadata(&item.path).is_err() {
            skipped_absent += 1;
            continue; // 재실행 안전: 이미 없는 항목은 건너뛴다(가이드 FAQ 계약).
        }
        let base = item
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "item".into());
        // 목적지 = <seq>-<basename> — 동명 항목(_round 다수 등) 충돌 없이 manifest 가 복구 지도.
        let dest = plan.trash_dir.join(format!("{i:03}-{base}"));
        // 선기록 + fsync — 이 줄이 디스크에 닿은 뒤에만 mv 한다(순서가 **계약**이다).
        // ★P0-5: 종전엔 write/flush/fsync 결과를 셋 다 버려서(`let _ =`) 저널 줄이 빠진 채 mv 가
        // 진행됐다 — 그러면 `042-_round`·`043-_round` 중 어느 것이 사용자 프로젝트였는지 영영
        // 알 수 없다. 지도를 못 남기면 **옮기지 않는다**(항목 단위 fail-closed).
        let journal_ok = {
            use std::io::Write;
            let line = serde_json::json!({
                "from": item.path.to_string_lossy(),
                "to": dest.to_string_lossy(),
            });
            writeln!(journal, "{line}")
                .and_then(|_| journal.flush())
                .and_then(|_| journal.sync_data())
        };
        if let Err(e) = journal_ok {
            failed.push((
                item.path.clone(),
                format!("복구 지도(journal.ndjson) 기록 실패 — 이동하지 않았다: {e}"),
            ));
            continue;
        }
        progress("quarantine", &format!("{} → trash", item.path.display()));
        match std::fs::rename(&item.path, &dest) {
            Ok(()) => moved.push((item.path.clone(), dest)),
            // best_effort(=실행 중 앱이 점유한 GUI 웹층) 실패는 부분 실패가 아니라 이연이다.
            Err(e) if item.best_effort => deferred.push((item.path.clone(), e.to_string())),
            Err(e) => failed.push((item.path.clone(), e.to_string())),
        }
    }

    // ★A2b(성찰 확정): 게이트는 진입 시 1회뿐이라 **격리 도중 부활**을 못 본다. 사후 재실측해
    // 부활을 발견하면 침묵하지 않고 보고에 싣는다(사용자는 앱을 끄고 재실행해야 한다).
    let revived = if any_cysd_running() {
        progress("verify", "⚠ 격리 도중 cysd 부활 감지");
        Some(
            "격리 도중 cysd 가 다시 기동했다 — 앱(cys-app)이 떠 있었을 가능성이 높다. \
             앱을 종료한 뒤 `cys factory-reset` 을 한 번 더 실행해 잔여 상태를 정리하라."
                .to_string(),
        )
    } else {
        None
    };

    // manifest — 복구의 단일 진실(역방향 mv 지도). 격리 디렉토리 안에 동봉.
    let manifest = serde_json::json!({
        "kind": "cys-factory-reset",
        "version": env!("CARGO_PKG_VERSION"),
        "stamp": plan.stamp,
        "restore": "각 entries[].to 를 entries[].from 으로 mv 하면 복구된다",
        "journal": "journal.ndjson — 이동 직전 선기록(중도 사망 시 이 파일이 유일한 복구 지도)",
        "entries": moved.iter().map(|(f, t)| serde_json::json!({
            "from": f.to_string_lossy(), "to": t.to_string_lossy(),
        })).collect::<Vec<_>>(),
        "failed": failed.iter().map(|(p, e)| serde_json::json!({
            "path": p.to_string_lossy(), "error": e,
        })).collect::<Vec<_>>(),
        "kept": plan.keep.iter().map(|k| k.path.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "revived_warning": revived.clone(),
        "deferred": deferred.iter().map(|(p, e)| serde_json::json!({
            "path": p.to_string_lossy(), "error": e,
        })).collect::<Vec<_>>(),
    });
    // ★P0-5: manifest 실패로 **Err 를 전파하면 안 된다**. 이 시점엔 이미 전 항목이 옮겨졌는데
    // 호출부는 그 Err 를 "격리 미진입(부분 이동 0)"으로 문장화해 정반대 정보를 보여줬고,
    // 훅 해제·심링크 제거·임시 소거가 통째로 건너뛰어져 사라진 팩을 가리키는 훅이 남았다.
    // 실패는 보고에 싣고(manifest_written=false) 나머지 단계는 **반드시 이어서 실행**한다.
    // journal.ndjson 은 이미 디스크에 있으므로 복구 지도는 살아 있다.
    let manifest_written = match serde_json::to_string_pretty(&manifest) {
        Ok(body) => match crate::pack::write_atomic(&plan.trash_dir.join("manifest.json"), body.as_bytes())
        {
            Ok(()) => true,
            Err(e) => {
                failed.push((
                    plan.trash_dir.join("manifest.json"),
                    format!("복구 지도 기록 실패(journal.ndjson 이 유일한 지도로 남는다): {e}"),
                ));
                false
            }
        },
        Err(e) => {
            failed.push((
                plan.trash_dir.join("manifest.json"),
                format!("복구 지도 직렬화 실패(journal.ndjson 이 유일한 지도로 남는다): {e}"),
            ));
            false
        }
    };

    // ── 외부 등록 외과 제거 ──
    let mut stripped: Vec<String> = Vec::new();
    // ★P2-2 ②: policy.json·accounts.json·profile.json 은 **오너가 판단해 넣은 설정**인데
    // 재온보딩이 재생성하지 않는다(빈 상태로 시작). 격리 자체는 유지하되(초기화 목적),
    // 되돌리기 쉽도록 별도 사본을 남기고 REPORT 에 위치를 적는다.
    let owner_settings_dir = plan.trash_dir.join("owner-settings");
    let mut owner_settings_saved: Vec<String> = Vec::new();
    for name in ["policy.json", "accounts.json", "profile.json"] {
        let src = roots.cys_base.join(name);
        if src.is_file() {
            let _ = std::fs::create_dir_all(&owner_settings_dir);
            if std::fs::copy(&src, owner_settings_dir.join(name)).is_ok() {
                owner_settings_saved.push(name.to_string());
            }
        }
    }
    if !owner_settings_saved.is_empty() {
        progress(
            "backup",
            &format!("오너 설정 사본 {}건 보관", owner_settings_saved.len()),
        );
    }

    let backup_dir = plan.trash_dir.join("settings-backups");
    for s in &plan.strip_settings {
        progress("strip", &format!("{} cys 훅 제거", s.display()));
        match strip_cys_from_settings_to(s, &roots.cys_base, plan.purge_local, &backup_dir) {
            Ok(removed) if removed.is_empty() => {}
            Ok(removed) => stripped.push(format!("{}: {}", s.display(), removed.join("·"))),
            // ★P1-1 ⑥: 실패는 **행동 가능한 조치**로 승격한다. 종전 문구("파일 무변경")는 사실만
            // 말하고 사용자가 뭘 해야 하는지 침묵해, 리셋 후 매 세션 훅 오류를 방치하게 했다.
            Err(e) => failed.push((
                s.clone(),
                format!(
                    "훅 제거 실패(파일 무변경): {e} — 이 파일을 열어 `.cys/pack` 를 가리키는 \
                     hooks 항목과 statusLine 을 직접 지우세요(안 지우면 매 세션 훅 오류)"
                ),
            )),
        }
    }
    progress("strip", "pack 스킬 심링크 정리");
    for dir in &plan.strip_skill_dirs {
        let n = remove_pack_skill_links(dir, &roots.cys_base);
        if n > 0 {
            stripped.push(format!("{}: pack 스킬 심링크 {n}개 제거", dir.display()));
        }
    }
    #[cfg(target_os = "macos")]
    {
        // defaults 도메인은 cfprefsd 캐시가 있어 파일 격리만으론 안 지워진다 — 병행 삭제.
        let _ = std::process::Command::new("defaults")
            .args(["delete", "com.cysjavis.terminal"])
            .output();
    }

    // ── 임시 캐시 소거(rm — OS 관리 임시 영역 한정 예외) ──
    progress("sweep", "임시 캐시 소거");
    let mut temp_swept = 0usize;
    for p in &plan.temp_sweep {
        let ok = if p.is_dir() {
            std::fs::remove_dir_all(p).is_ok()
        } else {
            std::fs::remove_file(p).is_ok()
        };
        if ok {
            temp_swept += 1;
        }
    }

    // ★P0-6: plist 파일 삭제는 **격리가 끝난 지금** 한다(정지 단계가 아니라).
    finalize_launchd_removal(plan);

    let report = ResetReport {
        trash_dir: plan.trash_dir.clone(),
        moved,
        failed,
        kept: plan.keep.iter().map(|k| k.path.clone()).collect(),
        stripped,
        report_only: plan.report_only.clone(),
        temp_swept,
        revived_warning: revived,
        deferred,
        skipped_absent,
        manifest_written,
        interrupted_prior: plan.interrupted_prior.clone(),
        owner_settings_saved,
    };

    // ★P0-4: 결과를 **디스크에 영속**한다. 화면 토스트는 60초 뒤 사라지고 완료 모달을 닫으면
    // 실패 내역이 영영 없어진다 — 사용자가 나중에 "뭐가 안 지워졌지?"를 확인할 단일 파일.
    let _ = crate::pack::write_atomic(
        &plan.trash_dir.join("REPORT.txt"),
        report.human_report().as_bytes(),
    );
    Ok(report)
}

// ─────────────────────────────────────────────────────────────────────────────
// 확인 문구 정규화 — "육안상 같은데 거부당한다"를 없앤다(P2-1).
//
// ★배경(시뮬레이션 지적): 확인 프롬프트는 `실행하려면 "완전 초기화" 를 정확히 입력:` 이라
// 화면에서 드래그 복사하면 **큰따옴표가 딸려 오고**, `trim()` 은 따옴표를 깎지 않아 즉시 거부됐다.
// 또 macOS 는 자모 분리(NFD) 한글을 흔히 만들고(파일시스템·일부 입력기·웹 복사), NBSP·전각
// 공백·ZWSP 가 섞여도 육안으로는 구분이 안 된다. 전부 같은 뜻이므로 같게 취급한다.
// ★의존성 0: 전면 NFC 가 아니라 **한글 음절 합성**만 직접 구현한다(대상 문구가 한글이므로 충분).
// ─────────────────────────────────────────────────────────────────────────────

/// 한글 자모(L+V[+T])를 완성형 음절로 합성한다(유니코드 표준 알고리즘 · NFD→NFC 부분집합).
fn compose_hangul(input: &str) -> String {
    const L_BASE: u32 = 0x1100;
    const V_BASE: u32 = 0x1161;
    const T_BASE: u32 = 0x11A7;
    const S_BASE: u32 = 0xAC00;
    const V_COUNT: u32 = 21;
    const T_COUNT: u32 = 28;

    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i] as u32;
        let l_idx = c.wrapping_sub(L_BASE);
        // 초성 + 중성이 이어질 때만 합성 시도.
        if l_idx < 19 && i + 1 < chars.len() {
            let v_idx = (chars[i + 1] as u32).wrapping_sub(V_BASE);
            if v_idx < V_COUNT {
                let mut t_idx = 0u32;
                let mut consumed = 2usize;
                if i + 2 < chars.len() {
                    let t = (chars[i + 2] as u32).wrapping_sub(T_BASE);
                    if t >= 1 && t < T_COUNT {
                        t_idx = t;
                        consumed = 3;
                    }
                }
                let syllable = S_BASE + (l_idx * V_COUNT + v_idx) * T_COUNT + t_idx;
                if let Some(ch) = char::from_u32(syllable) {
                    out.push(ch);
                    i += consumed;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 확인 문구 비교용 정규화 — 따옴표 제거·공백류 통일·한글 합성·양끝 정리.
/// GUI(resetconfirm.ts `normalizePhrase`)와 **같은 규칙**이어야 한다(한쪽만 관용하면 계약이 갈린다).
pub fn normalize_confirm_phrase(input: &str) -> String {
    let composed = compose_hangul(input);
    let unquoted = {
        let t = composed.trim();
        let pairs = [('"', '"'), ('\'', '\''), ('\u{201C}', '\u{201D}'), ('\u{2018}', '\u{2019}')];
        let mut cur = t;
        loop {
            let mut stripped = false;
            for (a, b) in pairs {
                let mut ch = cur.chars();
                if cur.chars().count() >= 2 && ch.next() == Some(a) && cur.ends_with(b) {
                    cur = &cur[a.len_utf8()..cur.len() - b.len_utf8()];
                    stripped = true;
                    break;
                }
            }
            if !stripped {
                break;
            }
        }
        cur.to_string()
    };
    unquoted
        .chars()
        // ZWSP·ZWNJ·ZWJ·BOM 은 제거, 그 외 공백류(NBSP·전각 등)는 보통 공백으로.
        .filter(|c| !matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'))
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ─────────────────────────────────────────────────────────────────────────────
// 복구(undo) — "되돌릴 수 있습니다"라는 고지를 제품이 실제로 이행하는 경로.
//
// ★P0(시뮬레이션 확정): 모달·가이드·CLI 가 모두 "격리 보관되어 되돌릴 수 있다"고 안심시키는데
// 정작 복구 수단이 제품에 없었다(터미널로 manifest 를 열어 수십 건을 손으로 mv 하는 것이 전부).
// 격리본은 14일 뒤 reap 대상이므로, 복구가 어려우면 사실상 비가역 삭제와 같다.
// ─────────────────────────────────────────────────────────────────────────────

pub struct UndoPlan {
    pub trash_dir: PathBuf,
    /// (격리위치 → 원위치). manifest.json 우선, 없으면 journal.ndjson(중도 중단분).
    pub entries: Vec<(PathBuf, PathBuf)>,
    pub source: &'static str,
    /// 원위치에 이미 무언가 있어 되돌릴 수 없는 항목(덮어쓰기 금지 — 사용자 판단 영역).
    pub blocked: Vec<(PathBuf, PathBuf)>,
}

/// 격리 폴더에서 복구 계획을 읽는다(쓰기 0). manifest 가 없으면 저널로 폴백한다.
pub fn read_undo_plan(trash_dir: &Path) -> Result<UndoPlan, String> {
    let mut entries: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut source = "manifest.json";
    let manifest = trash_dir.join("manifest.json");
    if let Ok(body) = std::fs::read_to_string(&manifest) {
        let v: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("manifest 파싱 실패: {e}"))?;
        for e in v["entries"].as_array().cloned().unwrap_or_default() {
            if let (Some(f), Some(t)) = (e["from"].as_str(), e["to"].as_str()) {
                entries.push((PathBuf::from(t), PathBuf::from(f)));
            }
        }
    } else {
        source = "journal.ndjson";
        let jl = std::fs::read_to_string(trash_dir.join("journal.ndjson")).map_err(|e| {
            format!(
                "{} 에서 복구 지도를 찾지 못했다(manifest.json·journal.ndjson 둘 다 없음): {e}",
                trash_dir.display()
            )
        })?;
        for line in jl.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // 중도 사망으로 잘린 마지막 줄은 건너뛴다.
            };
            if let (Some(f), Some(t)) = (v["from"].as_str(), v["to"].as_str()) {
                entries.push((PathBuf::from(t), PathBuf::from(f)));
            }
        }
    }
    // 저널은 "이동 시도"라 실제로 옮겨지지 않은 줄이 있을 수 있다 — 격리본 실재분만 남긴다.
    let (present, _absent): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|(from, _)| std::fs::symlink_metadata(from).is_ok());
    let (blocked, restorable): (Vec<_>, Vec<_>) = present
        .into_iter()
        .partition(|(_, to)| std::fs::symlink_metadata(to).is_ok());
    Ok(UndoPlan {
        trash_dir: trash_dir.to_path_buf(),
        entries: restorable,
        source,
        blocked,
    })
}

/// 복구 실행 — 격리본을 원위치로 되돌린다(역방향 mv). 원위치가 이미 차 있으면 **건너뛴다**
/// (덮어쓰기 금지 — 리셋 후 재온보딩으로 새로 생긴 상태를 파괴하지 않는다).
/// 반환: (복구 수, 실패 목록).
pub fn execute_undo(
    plan: &UndoPlan,
    progress: &mut dyn FnMut(&str, &str),
) -> (usize, Vec<(PathBuf, String)>) {
    let mut restored = 0usize;
    let mut failed: Vec<(PathBuf, String)> = Vec::new();
    for (from, to) in &plan.entries {
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        progress("undo", &format!("{} → {}", from.display(), to.display()));
        match std::fs::rename(from, to) {
            Ok(()) => restored += 1,
            Err(e) => failed.push((to.clone(), e.to_string())),
        }
    }
    (restored, failed)
}

// ─────────────────────────────────────────────────────────────────────────────
// 개인 프로필 외과 제거 — merge_desired_hooks(추가만)의 역연산(우리 것만 제거).
// ─────────────────────────────────────────────────────────────────────────────

/// 명령 문자열이 이 설치의 팩(~/.cys/pack·pack-dept-*) 아래를 가리키는가.
/// Windows 훅 명령은 슬래시 정규화 경로라 양쪽을 '/' 로 맞춰 비교한다.
fn command_points_into_pack(command: &str, cys_base: &Path) -> bool {
    // ★P1-1 ②: 경계 일치. 종전엔 `<base>/pack` **부분 문자열**이라 사용자가 만든
    // `~/.cys/pack-notes/hooks/x.sh` 같은 경로까지 "우리 것"으로 오판해, 파일은 보존되는데
    // 그 훅 등록만 조용히 해제되는 A8 대칭 계약의 정확한 역전이 일어났다.
    // 우리 것은 `<base>/pack`(정확히) 또는 `<base>/pack-dept-<name>` 뿐이다.
    let base = cys_base.to_string_lossy().replace('\\', "/");
    let cmd = command.replace('\\', "/");
    let pack = format!("{base}/pack");
    let mut from = 0usize;
    while let Some(i) = cmd[from..].find(&pack) {
        let at = from + i;
        let rest = &cmd[at + pack.len()..];
        // 뒤가 경로 구분자·끝·따옴표면 팩 본체, `-dept-` 로 이어지면 부서 팩.
        if rest.is_empty()
            || rest.starts_with('/')
            || rest.starts_with('"')
            || rest.starts_with('\'')
            || rest.starts_with(char::is_whitespace)
            || rest.starts_with("-dept-")
        {
            return true;
        }
        from = at + pack.len();
    }
    false
}

/// 이 명령이 **격리로 사라질 경로**를 가리키는가 — strip 대상 판정의 정본.
/// pack(항상 격리) ∪ local(--purge-local 일 때만 격리). 보존되는 local 을 가리키는 훅은
/// 리셋 후에도 **유효**하므로 건드리지 않는다(A8 대칭 계약 — 과잉 제거는 사용자 설정 파괴다).
fn command_points_into_quarantined(command: &str, cys_base: &Path, include_local: bool) -> bool {
    if command_points_into_pack(command, cys_base) {
        return true;
    }
    if include_local {
        let needle = format!("{}/{LOCAL_OVERLAY}", cys_base.to_string_lossy().replace('\\', "/"));
        return command.replace('\\', "/").contains(&needle);
    }
    false
}

/// `shlex.quote` 단일따옴표 출력의 역변환(CYS_PREV_STATUSLINE 원복용).
/// 형식 밖 입력은 None — 호출부는 안전 기본값(제거)으로 처리한다.
fn shell_unquote_single(s: &str) -> Option<String> {
    let t = s.trim();
    if !t.starts_with('\'') || !t.ends_with('\'') || t.len() < 2 {
        // 따옴표 불필요 토큰(공백·특수문자 없음)은 shlex.quote 가 그대로 내보낸다.
        if !t.is_empty() && !t.contains('\'') && !t.contains(' ') {
            return Some(t.to_string());
        }
        return None;
    }
    Some(t[1..t.len() - 1].replace("'\"'\"'", "'"))
}

/// settings.json 에서 **cys 팩을 가리키는 훅·statusLine 만** 제거한다.
///
/// 계약(merge_desired_hooks 대칭):
///  · 제거만 한다 — 사용자 훅·타 도구 훅·배열 순서 불가침. 빈 껍데기(entry/event/hooks)만 청소.
///  · statusLine: 우리 것이면 제거하되 `CYS_PREV_STATUSLINE=<prev>` 보존분은 원복.
///  · 변경이 없으면 파일을 쓰지 않는다(백업도 안 만든다).
///  · symlink·파싱 실패 = 거부(Err — 설정 파일 클로버 금지).
///  · 변경 시 `.bak-factory-reset` 백업 후 write_atomic.
///
/// 반환: 사람용 제거 라벨(빈 벡터 = 무변경).
pub fn strip_cys_from_settings(settings_path: &Path, cys_base: &Path) -> Result<Vec<String>, String> {
    strip_cys_from_settings_scoped(settings_path, cys_base, false)
}

/// 위 함수의 범위 지정판 — `strip_local=true` 면 `~/.cys/local` 을 가리키는 훅·statusLine 도
/// 함께 해제한다(local 을 격리했을 때만. 보존 시엔 그 훅이 유효하므로 남긴다 · A8).
pub fn strip_cys_from_settings_scoped(
    settings_path: &Path,
    cys_base: &Path,
    strip_local: bool,
) -> Result<Vec<String>, String> {
    strip_settings_inner(settings_path, cys_base, strip_local, None)
}

/// `backup_dir` 가 주어지면 백업을 **격리 폴더 안**에 남긴다(P1-1 ③ — 프로필에 cys 훅 전문이
/// 든 파일을 새로 늘리지 않는다. "cys 흔적은 격리 폴더 한 곳"이 계약이다).
pub fn strip_cys_from_settings_to(
    settings_path: &Path,
    cys_base: &Path,
    strip_local: bool,
    backup_dir: &Path,
) -> Result<Vec<String>, String> {
    strip_settings_inner(settings_path, cys_base, strip_local, Some(backup_dir))
}

fn strip_settings_inner(
    settings_path: &Path,
    cys_base: &Path,
    strip_local: bool,
    backup_dir: Option<&Path>,
) -> Result<Vec<String>, String> {
    // 매칭 술어만 주입형으로 일반화(G3 축1 — hooks-prune 이 세 번째 소비자) — factory reset 의
    // 기존 거동(전체 설치 소유 판정 + statusLine 원복 + 백업 규약)은 byte-identical 로 보존한다.
    let matcher = |c: &str| command_points_into_quarantined(c, cys_base, strip_local);
    match backup_dir {
        Some(dir) => {
            strip_settings_matching(settings_path, cys_base, &matcher, true, StripBackup::Dir(dir))
        }
        None => strip_settings_matching(
            settings_path,
            cys_base,
            &matcher,
            true,
            StripBackup::Suffix(".bak-factory-reset"),
        ),
    }
}

/// strip 계열의 백업 지정 — 기존 두 형태(옆자리 접미 / 격리 폴더)를 한 타입으로 표현한다.
/// hooks-prune(`.bak-cys-dept`)이 세 번째 소비자가 되면서 승격 — 각 소비자의 거동은 종전 그대로.
enum StripBackup<'a> {
    /// `<settings>.<suffix>` 옆자리 백업(단독 호출·테스트 폴백·hooks-prune).
    Suffix(&'a str),
    /// 지정 디렉터리 안 `<프로필태그>.settings.json`(factory reset 격리 폴더 규약 · P1-1 ③).
    Dir(&'a Path),
}

/// 명령 문자열이 **특정 팩 루트** 아래를 가리키는가 — `cys hooks-prune` 의 소유 판정.
/// 훅 명령 문자열(`hook_command_for`)에는 설치 시점 팩 절대경로가 그대로 박혀 있으므로
/// **경로가 곧 소유 ID** 다(태깅 제2 SOT 기각 — G3 축1 확정).
///
/// 정규화: 양변 '/' 통일 + pack_root 는 원문과 `pack::resolve_for_compare`(존재 접두 canonicalize —
/// macOS /tmp↔/private/tmp·심링크 흡수) **두 형태 모두** 후보로 삼는다 — 죽은 경로(팩 삭제 후
/// 잔존 훅)는 canonicalize 가 원문으로 접히고, 산 경로는 심링크 차이를 흡수한다.
/// 경계 일치는 `command_points_into_pack` 과 동일 규약(뒤가 '/'·따옴표·공백·끝) — 부분 문자열
/// 오판(`pack-dept-d1` 이 `pack-dept-d10` 을 잡는 것)을 차단한다.
/// Windows 한계(감수 범위 · 리뷰 MINOR): 비교는 대소문자 민감 — NTFS 무구분 경로로 케이스만 다른
/// 잔존 훅은 미탐지/미제거(fail-safe: 오삭제 없음). unix 케이스 민감 파일계에서 무구분 비교는
/// 역으로 오삭제 표면이라 전 OS 폴딩은 금지 — Windows 부서 churn 표면이 생기는 릴리스에서
/// cfg(windows) 케이스 폴딩으로 승격한다(cys.rs acquire_settings_lock 의 감수 범위와 같은 트랙).
pub(crate) fn command_points_into_pack_root(command: &str, pack_root: &Path) -> bool {
    let cmd = command.replace('\\', "/");
    let raw = pack_root.to_string_lossy().replace('\\', "/");
    let resolved = crate::pack::resolve_for_compare(pack_root)
        .to_string_lossy()
        .replace('\\', "/");
    let mut needles: Vec<String> = vec![raw.trim_end_matches('/').to_string()];
    let r = resolved.trim_end_matches('/').to_string();
    if !needles.contains(&r) {
        needles.push(r);
    }
    for needle in needles {
        if needle.is_empty() {
            continue;
        }
        let mut from = 0usize;
        while let Some(i) = cmd[from..].find(&needle) {
            let at = from + i;
            let rest = &cmd[at + needle.len()..];
            if rest.is_empty()
                || rest.starts_with('/')
                || rest.starts_with('"')
                || rest.starts_with('\'')
                || rest.starts_with(char::is_whitespace)
            {
                return true;
            }
            from = at + needle.len();
        }
    }
    false
}

/// settings.json 에서 **지정 팩 루트를 가리키는 훅 항목만** 제거한다(G3 축1 치유층 코어).
///
/// 계약(strip_cys_from_settings 대칭 — 검증된 기계 재사용·신규 파서 0):
///  · 제거만 한다 — 사용자 훅·타 도구 훅·타 팩(base 포함) 훅·배열 순서 불가침.
///  · **훅 항목만** 본다(statusLine 은 factory reset 소유 판정 소관 — 여기서 건드리지 않는다).
///  · 변경이 없으면 파일을 쓰지 않는다(백업도 안 만든다 — 멱등).
///  · symlink 는 홈 아래 일반 파일로 해소될 때만 그 실파일 대상, 그 외 거부 / 파싱 실패 = 거부.
///  · 변경 시 백업(`backup_dir` 지정 시 그 안 / 아니면 옆자리 `.bak-cys-dept`) 후 write_atomic
///    + 원 권한 복원 — 실패 시 무변조(Err).
///
/// 반환: 사람용 제거 라벨(`"SessionStart×1"`) — 빈 벡터 = 대상 없음.
pub fn strip_hooks_pointing_into_pack(
    settings_path: &Path,
    pack_root: &Path,
    backup_dir: Option<&Path>,
) -> Result<Vec<String>, String> {
    // 홈 경계 파생용 base 는 팩 루트의 부모(~/.cys 규약) — strip_settings_inner 와 동일 파생.
    let cys_base = pack_root
        .parent()
        .ok_or_else(|| format!("pack root has no parent: {}", pack_root.display()))?
        .to_path_buf();
    let matcher = |c: &str| command_points_into_pack_root(c, pack_root);
    match backup_dir {
        Some(dir) => {
            strip_settings_matching(settings_path, &cys_base, &matcher, false, StripBackup::Dir(dir))
        }
        None => strip_settings_matching(
            settings_path,
            &cys_base,
            &matcher,
            false,
            StripBackup::Suffix(".bak-cys-dept"),
        ),
    }
}

/// 읽기 전용 판정판(--dry-run·doctor 탐지용) — 제거 대상 라벨만 산출하고 아무것도 쓰지 않는다.
/// 파일 부재 = 빈 벡터(대상 없음) / 파싱 실패 = Err(측정 불능 ≠ 통과 — fail-closed 보고).
/// symlink 규약은 제거판과 **동일 판정**(`resolve_symlinked_settings`)이다[리뷰 MINOR 봉인] —
/// 종전엔 여기가 링크를 그냥 따라 읽어 dry-run 이 '제거 예정'을 약속하고 실제 실행이 거부(exit 1)
/// 하는 관측 불일치가 있었다(둘 다 fail-closed 방향이라 오삭제는 없었으나 약속≠실행).
pub fn hooks_pointing_into_pack(
    settings_path: &Path,
    pack_root: &Path,
) -> Result<Vec<String>, String> {
    let cys_base = pack_root
        .parent()
        .ok_or_else(|| format!("pack root has no parent: {}", pack_root.display()))?;
    let target = resolve_symlinked_settings(settings_path, cys_base)?;
    let raw = match std::fs::read_to_string(&target) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("read error: {e}")),
    };
    let root: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("parse error: {e}"))?;
    let mut out: Vec<String> = Vec::new();
    if let Some(hooks) = root.get("hooks").and_then(|h| h.as_object()) {
        for (ev, arr) in hooks {
            let Some(arr) = arr.as_array() else { continue };
            let n = arr
                .iter()
                .filter_map(|entry| entry.get("hooks").and_then(|v| v.as_array()))
                .flatten()
                .filter(|h| {
                    h.get("command")
                        .and_then(|c| c.as_str())
                        .map(|c| command_points_into_pack_root(c, pack_root))
                        .unwrap_or(false)
                })
                .count();
            if n > 0 {
                out.push(format!("{ev}×{n}"));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// settings 경로의 심링크 해소 — 탐지판(`hooks_pointing_into_pack`)·제거판
/// (`strip_settings_matching`) **공용 단일 판정**[리뷰 MINOR: dry-run↔실행 관측 일치].
///
/// ★P1-1 ①: 도트파일 저장소로 settings.json 을 **심링크**해 쓰는 구성은 흔하다. 종전엔
/// 그냥 거부해서, 리셋 후 그 사용자는 매 Claude Code 세션마다 사라진 팩을 가리키는 훅이
/// 전부 "No such file" 로 실패하는 걸 봐야 했다(조치 안내도 없이).
/// → 링크 자체는 절대 건드리지 않되(계약 보존), 링크가 가리키는 **홈 아래 일반 파일**이면
///   그 실파일을 대상으로 이어간다. 홈 밖·파일 아님은 기존대로 거부(클로버 금지).
///
/// 홈 경계는 **cys_base 의 부모**에서 파생한다(cys_base = <home>/.cys 규약) — 전역
/// dirs::home_dir() 을 쓰면 루트가 주입된 환경(테스트 샌드박스)에서 판정이 어긋난다.
/// 양쪽 다 canonicalize 해서 /var ↔ /private/var 같은 심링크 접두 차이를 흡수한다.
fn resolve_symlinked_settings(settings_path: &Path, cys_base: &Path) -> Result<PathBuf, String> {
    if !std::fs::symlink_metadata(settings_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Ok(settings_path.to_path_buf());
    }
    let real = std::fs::canonicalize(settings_path)
        .map_err(|e| format!("{} is a symlink and unresolvable: {e}", settings_path.display()))?;
    let home_ok = cys_base
        .parent()
        .and_then(|h| std::fs::canonicalize(h).ok())
        .map(|h| real.starts_with(&h))
        .unwrap_or(false);
    if !home_ok || !real.is_file() {
        return Err(format!(
            "{} is a symlink to {} (홈 밖이거나 일반 파일이 아님) — 건드리지 않는다. \
             그 파일에서 `.cys/pack` 를 가리키는 훅을 직접 지우세요",
            settings_path.display(),
            real.display()
        ));
    }
    Ok(real)
}

fn strip_settings_matching(
    settings_path: &Path,
    cys_base: &Path,
    matcher: &dyn Fn(&str) -> bool,
    strip_statusline: bool,
    backup: StripBackup,
) -> Result<Vec<String>, String> {
    // 심링크 규약은 탐지판과 공용 단일 판정(resolve_symlinked_settings doc 참조).
    let target = resolve_symlinked_settings(settings_path, cys_base)?;
    let settings_path = target.as_path();
    let raw = match std::fs::read_to_string(settings_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("read error: {e}")),
    };
    let mut root: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("parse error: {e}"))?;
    let obj = root.as_object_mut().ok_or("settings root is not an object")?;
    let mut removed: Vec<String> = Vec::new();

    if let Some(hooks) = obj.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        let events: Vec<String> = hooks.keys().cloned().collect();
        for ev in events {
            let Some(arr) = hooks.get_mut(&ev).and_then(|v| v.as_array_mut()) else {
                continue;
            };
            for entry in arr.iter_mut() {
                let Some(inner) = entry.get_mut("hooks").and_then(|v| v.as_array_mut()) else {
                    continue;
                };
                let before = inner.len();
                inner.retain(|h| {
                    !h.get("command")
                        .and_then(|c| c.as_str())
                        .map(matcher)
                        .unwrap_or(false)
                });
                let n = before - inner.len();
                if n > 0 {
                    removed.push(format!("{ev}×{n}"));
                }
            }
            arr.retain(|entry| {
                entry
                    .get("hooks")
                    .and_then(|v| v.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(true) // hooks 배열이 없는 이형 entry 는 사용자 소유 — 불가침.
            });
            if arr.is_empty() {
                hooks.remove(&ev);
            }
        }
        if hooks.is_empty() {
            obj.remove("hooks");
        }
    }

    if let Some(cmd) = obj
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .filter(|_| strip_statusline)
    {
        if matcher(&cmd) {
            // ★W1(성찰 확정): 구분자는 OS별 훅 명령 형식과 1:1 이다 —
            // unix `sh <abs>` / Windows `bash "<정슬래시 abs>"`(javis_preflight._cys_hook_cmd·
            // pack::hook_command_for). `" sh "` 만 찾으면 Windows 에서 항상 원복 실패 →
            // 사용자의 기존 statusLine 이 무음 삭제된다. 두 형식을 모두 인식한다.
            let restored = cmd
                .strip_prefix("CYS_PREV_STATUSLINE=")
                .and_then(|rest| {
                    rest.rfind(" sh ")
                        .or_else(|| rest.rfind(" bash "))
                        .map(|i| rest[..i].to_string())
                })
                .and_then(|q| shell_unquote_single(&q));
            match restored {
                Some(prev) => {
                    obj.insert(
                        "statusLine".into(),
                        serde_json::json!({"type": "command", "command": prev}),
                    );
                    removed.push("statusLine(이전 값 원복)".into());
                }
                None => {
                    obj.remove("statusLine");
                    removed.push("statusLine".into());
                }
            }
        }
    }

    if removed.is_empty() {
        return Ok(removed);
    }
    // ★P1-1 ③: 백업은 **격리 폴더 안**으로. 종전엔 프로필 옆에 `.bak-factory-reset` 을 만들어
    // "cys 흔적을 지운다"면서 cys 훅 전문이 든 파일을 프로필마다 새로 늘렸다(그리고 그 백업을
    // 되돌리면 죽은 훅이 부활한다). backup_dir 이 없으면(단독 호출·테스트) 종전 위치로 폴백한다.
    match backup {
        StripBackup::Dir(dir) => {
            let _ = std::fs::create_dir_all(dir);
            // 프로필 식별자를 파일명에 담는다(.claude / .claude-2 …가 전부 settings.json 이므로).
            let tag = settings_path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "profile".into());
            std::fs::copy(settings_path, dir.join(format!("{tag}.settings.json")))
                .map_err(|e| format!("backup failed: {e}"))?;
        }
        StripBackup::Suffix(sfx) => {
            let backup = format!("{}{sfx}", settings_path.display());
            std::fs::copy(settings_path, &backup).map_err(|e| format!("backup failed: {e}"))?;
        }
    }
    // ★P1-1 ⑤: write_atomic 은 tmp+rename 이라 **원본 권한이 사라진다** — 읽기 전용(0444)으로
    // 잠가 둔 설정이 조용히 쓰기 가능해졌다. 교체 후 원래 권한을 복원한다.
    let orig_perm = std::fs::metadata(settings_path).ok().map(|m| m.permissions());
    let body = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    crate::pack::write_atomic(settings_path, body.as_bytes()).map_err(|e| e.to_string())?;
    if let Some(perm) = orig_perm {
        let _ = std::fs::set_permissions(settings_path, perm);
    }
    Ok(removed)
}

/// `~/.claude*/skills/` 에서 **대상이 팩(~/.cys/pack…)인 심링크만** 제거한다.
/// 실디렉토리·타 대상 링크 불가침(preflight 심링크 파밍의 역연산). 반환: 제거 수.
pub fn remove_pack_skill_links(skills_dir: &Path, cys_base: &Path) -> usize {
    let needle = format!("{}/pack", cys_base.to_string_lossy().replace('\\', "/"));
    let mut n = 0usize;
    if let Ok(rd) = std::fs::read_dir(skills_dir) {
        for e in rd.flatten() {
            let p = e.path();
            let is_link = std::fs::symlink_metadata(&p)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if !is_link {
                continue;
            }
            let target_hits = std::fs::read_link(&p)
                .map(|t| t.to_string_lossy().replace('\\', "/").contains(&needle))
                .unwrap_or(false);
            // ★W2(성찰 확정): Windows 는 **디렉토리 심링크**를 remove_file 로 지울 수 없다
            // (RemoveDirectory 필요). unix 는 unlink 라 첫 시도에서 성공하므로 무영향.
            // 링크가 어느 타입으로 만들어졌든 지워지도록 두 경로를 모두 시도한다.
            if target_hits
                && (std::fs::remove_file(&p).is_ok() || std::fs::remove_dir(&p).is_ok())
            {
                n += 1;
            }
        }
    }
    n
}

// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// ★P1-4: 경로를 문자열로 단언할 때 구분자를 정규화한다. Windows 는 `\\` 라
    /// `"…/_round"` 같은 단언이 통째로 거짓이 되어 **Windows 에서만 빨간** 테스트가 된다
    /// (그 자체가 시뮬레이션이 지적한 "돌지 않는/못 도는 게이트"의 한 형태다).
    fn norm(p: &Path) -> String {
        p.to_string_lossy().replace('\\', "/")
    }
    fn norm_all(v: &[PathBuf]) -> Vec<String> {
        v.iter().map(|p| norm(p)).collect()
    }

    /// 리포 관례(pack.rs 테스트와 동일): temp_dir + 태그·pid — 시작 시 청소, 종료 후 잔존 허용.
    fn test_home(tag: &str) -> PathBuf {
        let td = std::env::temp_dir().join(format!("cys-freset-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        td
    }

    fn mk(p: &Path) {
        std::fs::create_dir_all(p).unwrap();
    }
    fn touch(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn fake_roots(home: &Path) -> ResetRoots {
        ResetRoots {
            home: home.to_path_buf(),
            cys_base: home.join(".cys"),
            state_root: home.join(".local/state"),
            trash_root: home.join(".local/state/cys-trash"),
            library: None,
            darwin_cache: None,
            temp: home.join("tmpzone"),
            workspace_root: home.join("Desktop/CYSjavis"),
            win_local_state: None,
            win_webview_data: None,
        }
    }

    /// ★★Windows 치명 회귀(감사 확정 2026-08-16): `%LOCALAPPDATA%\cys` 는 **NSIS 설치
    /// 디렉토리이자 메인 데몬 상태 디렉토리**다(installMode currentUser · productName cys).
    /// 통째 격리는 앱 언인스톨과 같다 — 상태 항목만 골라 옮기고 설치본은 반드시 보존한다.
    /// 이 핀이 없으면 "맥은 멀쩡한데 윈도에서 앱이 사라지는" 사고가 그대로 난다.
    #[test]
    fn windows_install_dir_is_never_quarantined() {
        let td = test_home("winroots");
        let mut r = fake_roots(&td);
        let lad = td.join("AppData/Local");
        let inst = lad.join("cys");
        // 설치본(NSIS currentUser) — 절대 대상이 아니다.
        for f in ["cys.exe", "cysd.exe", "cys-app.exe", "uninstall.exe"] {
            touch(&inst.join(f), "MZ");
        }
        mk(&inst.join("runtime"));
        mk(&inst.join("resources"));
        // 데몬 상태 — 격리 대상.
        touch(&inst.join("transcripts.db"), "db");
        touch(&inst.join("topology.json"), "{}");
        touch(&inst.join("cysd.log"), "log");
        mk(&inst.join("cys-dept-d1"));
        mk(&inst.join("phoenix"));
        mk(&lad.join("com.cysjavis.terminal"));
        r.win_local_state = Some(inst.clone());
        r.win_webview_data = Some(lad.join("com.cysjavis.terminal"));

        let plan = build_plan(&r, &ResetOptions { purge_license: false, purge_local: false, purge_round: false });
        let q: Vec<String> = plan.quarantine.iter().map(|i| norm(&i.path)).collect();
        let k: Vec<String> = plan.keep.iter().map(|i| norm(&i.path)).collect();

        for app in ["cys.exe", "cysd.exe", "cys-app.exe", "uninstall.exe", "runtime", "resources"] {
            let suffix = format!("/cys/{app}");
            assert!(
                !q.iter().any(|p| p.ends_with(&suffix)),
                "설치본을 격리하면 앱이 사라진다: {app} / {q:?}"
            );
            assert!(k.iter().any(|p| p.ends_with(&suffix)), "보존 목록 누락: {app} / {k:?}");
        }
        for st in ["transcripts.db", "topology.json", "cysd.log", "cys-dept-d1", "phoenix"] {
            let suffix = format!("/cys/{st}");
            assert!(q.iter().any(|p| p.ends_with(&suffix)), "상태 누락: {st} / {q:?}");
        }
        assert!(q.iter().any(|p| p.ends_with("com.cysjavis.terminal")), "{q:?}");
        assert!(
            plan.quarantine.iter().any(|i| i.best_effort && i.path.ends_with("com.cysjavis.terminal")),
            "WebView 는 이연 등급이어야 한다"
        );
    }

    fn seed_practice_tree(home: &Path) -> ResetRoots {
        let r = fake_roots(home);
        mk(&r.temp);
        // ~/.cys — 알려진 상태 + 오너 배치 + 라이선스.
        mk(&r.cys_base.join("pack/round"));
        mk(&r.cys_base.join("pack-dept-dept-1"));
        mk(&r.cys_base.join("pack-dept---help")); // 유령 부서 잔재도 접두로 잡힌다.
        mk(&r.cys_base.join("claude-default-dept-1"));
        mk(&r.cys_base.join("state-generations/20260811T002345Z"));
        touch(&r.cys_base.join("depts.json"), "{}");
        touch(&r.cys_base.join(".master-bootstrapped"), "x");
        touch(&r.cys_base.join(".master-bootstrapped-dept-1"), "x");
        mk(&r.cys_base.join("transfers"));
        touch(&r.cys_base.join("apple-notary.env"), "SECRET");
        touch(&r.cys_base.join(crate::license::LICENSE_BASENAMES[0]), "{}");
        touch(&r.cys_base.join(crate::license::LICENSE_BASENAMES[1]), "sig");
        // 데몬 상태.
        mk(&r.state_root.join("cys"));
        mk(&r.state_root.join("cys-dept-dept-1"));
        mk(&r.state_root.join("cys-trash/old-entry"));
        mk(&r.state_root.join("claude")); // Claude Code 소유 — cys 아님.
        // 작업기억.
        touch(&home.join("_round/SESSION_STATE.md"), "s");
        touch(
            &r.workspace_root.join("_round/ACTIVE_PROJECT"),
            &home.join("proj-a").to_string_lossy(),
        );
        touch(&home.join("proj-a/_round/tasks/t.json"), "{}");
        r
    }

    #[test]
    fn plan_quarantines_known_state_and_keeps_owner_files() {
        let td = test_home("plan-known");
        let r = seed_practice_tree(&td);
        let plan = build_plan(&r, &ResetOptions { purge_license: false, purge_local: false, purge_round: false });
        let q: Vec<String> = plan.quarantine.iter().map(|i| norm(&i.path)).collect();
        // 알려진 상태는 전부 격리 목록에.
        for must in [
            ".cys/pack",
            ".cys/pack-dept-dept-1",
            ".cys/pack-dept---help",
            ".cys/claude-default-dept-1",
            ".cys/depts.json",
            ".cys/.master-bootstrapped",
            ".cys/.master-bootstrapped-dept-1",
            ".cys/state-generations",
            ".cys/transfers",
            ".local/state/cys",
            ".local/state/cys-dept-dept-1",
            "_round",
        ] {
            assert!(q.iter().any(|p| p.ends_with(must)), "missing {must}: {q:?}");
        }
        // ★P0-3 판정: 사용자 **프로젝트 폴더 안**의 _round 는 기본 격리 대상이 아니다(고지만).
        // 격리본은 14일 뒤 reap 대상이라, 동의 없이 옮기면 사용자 저작물을 영구 소거하게 된다.
        assert!(
            !q.iter().any(|p| p.ends_with("proj-a/_round")),
            "프로젝트 _round 는 기본 격리 금지: {q:?}"
        );
        assert!(
            plan.report_only
                .iter()
                .any(|r| r.replace('\\', "/").contains("proj-a/_round") && r.contains("남습니다")),
            "프로젝트 _round 는 '남는다'고 고지해야 한다: {:?}",
            plan.report_only
        );
        // 오너 파일·라이선스·트래시·Claude Code 상태는 절대 격리 안 됨.
        let lic0 = crate::license::LICENSE_BASENAMES[0];
        for never in ["apple-notary.env", lic0, "cys-trash", ".local/state/claude"] {
            assert!(!q.iter().any(|p| p.contains(never)), "must keep {never}: {q:?}");
        }
        let kept: Vec<String> = plan.keep.iter().map(|i| norm(&i.path)).collect();
        assert!(kept.iter().any(|p| p.ends_with("apple-notary.env")));
        assert!(kept.iter().any(|p| p.ends_with(lic0)));
    }

    #[test]
    fn plan_purge_license_optin_moves_license() {
        let td = test_home("license");
        let r = seed_practice_tree(&td);
        let plan = build_plan(&r, &ResetOptions { purge_license: true, purge_local: false, purge_round: false });
        assert!(plan
            .quarantine
            .iter()
            .any(|i| i.path.ends_with(crate::license::LICENSE_BASENAMES[0])));
    }

    /// 트립와이어(설계 §7): plan 은 $HOME 자신·홈 밖·보호 루트를 절대 포함하지 않는다.
    #[test]
    fn plan_never_targets_home_or_protected_roots() {
        let td = test_home("tripwire");
        let r = seed_practice_tree(&td);
        // 악성 ACTIVE_PROJECT: 홈 자신·홈 밖을 가리켜도 게이트가 걸러야 한다.
        touch(
            &r.workspace_root.join("_round/ACTIVE_PROJECT"),
            &&td.to_string_lossy(),
        );
        mk(&&td.join("_round")); // home/_round 는 정당 대상.
        let plan = build_plan(&r, &ResetOptions { purge_license: false, purge_local: false, purge_round: false });
        let home_real = std::fs::canonicalize(&td).unwrap();
        for item in &plan.quarantine {
            let real = std::fs::canonicalize(&item.path).unwrap_or(item.path.clone());
            assert_ne!(real, home_real, "plan must never target $HOME itself");
            assert!(real.starts_with(&home_real), "plan escaped home: {real:?}");
            assert!(
                !PROTECTED_ROOTS.iter().any(|p| Path::new(p) == real),
                "protected root in plan: {real:?}"
            );
        }
    }

    #[test]
    #[cfg(unix)] // symlink 픽스처 — Windows 는 권한 필요라 제외.
    fn round_symlink_root_is_refused() {
        let td = test_home("symroot");
        let r = fake_roots(&td);
        mk(&&td.join("elsewhere/real_round"));
        std::os::unix::fs::symlink(
            &td.join("elsewhere/real_round"),
            &td.join("_round"),
        )
        .unwrap();
        let plan = build_plan(&r, &ResetOptions { purge_license: false, purge_local: false, purge_round: false });
        assert!(
            !plan.quarantine.iter().any(|i| i.path.ends_with("_round")),
            "symlink _round must not be auto-quarantined"
        );
        assert!(plan
            .report_only
            .iter()
            .any(|s| s.contains("symlink-root")));
    }

    #[test]
    fn execute_moves_writes_manifest_and_is_rerun_safe() {
        let td = test_home("exec");
        let r = seed_practice_tree(&td);
        let plan = build_plan(&r, &ResetOptions { purge_license: false, purge_local: false, purge_round: false });
        let not_cysd = |_pid: u32| false;
        let mut noop = |_p: &str, _d: &str| {};
        let no_daemon = || false;
        let rep = execute_quarantine(&plan, &r, &not_cysd, &no_daemon, &mut noop).unwrap();
        assert!(rep.ok(), "failed items: {:?}", rep.failed);
        // 원위치 소멸 + 격리 위치 실존.
        assert!(!r.cys_base.join("pack").exists());
        assert!(!r.state_root.join("cys").exists());
        assert!(r.cys_base.join("apple-notary.env").exists(), "owner file must survive");
        assert!(r.cys_base.join(crate::license::LICENSE_BASENAMES[0]).exists());
        assert!(r.state_root.join("cys-trash/old-entry").exists(), "old trash must survive");
        for (_, to) in &rep.moved {
            assert!(to.starts_with(&plan.trash_dir), "dest outside trash: {to:?}");
            assert!(to.exists());
        }
        // ★J1: 저널이 이동 **전에** 선기록됐다 — 중도 사망 시 유일한 복구 지도.
        let jl = std::fs::read_to_string(plan.trash_dir.join("journal.ndjson")).unwrap();
        let jlines: Vec<&str> = jl.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(jlines.len(), rep.moved.len(), "저널 줄 수 = 이동 시도 수여야 한다");
        for l in &jlines {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            assert!(v["from"].is_string() && v["to"].is_string(), "저널 줄 형식: {l}");
        }
        // 동명 항목(_round 다수)도 저널로 원위치가 구분된다.
        let round_lines: Vec<&&str> = jlines.iter().filter(|l| l.contains("_round")).collect();
        if round_lines.len() > 1 {
            let froms: std::collections::HashSet<String> = round_lines
                .iter()
                .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["from"].as_str().unwrap().to_string())
                .collect();
            assert_eq!(froms.len(), round_lines.len(), "동명 _round 의 원위치가 저널에서 유일해야 한다");
        }
        // manifest 는 복구 지도.
        let m: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(plan.trash_dir.join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(m["entries"].as_array().unwrap().len(), rep.moved.len());
        // 재실행 안전: 같은 plan 재실행은 전부 스킵되고 성공한다.
        let rep2 = execute_quarantine(&plan, &r, &not_cysd, &no_daemon, &mut noop).unwrap();
        assert!(rep2.ok());
        assert!(rep2.moved.is_empty());
    }

    #[test]
    fn quiescent_gate_blocks_live_cysd_holder() {
        let td = test_home("gate");
        let r = seed_practice_tree(&td);
        touch(&r.state_root.join("cys/cys.lock"), "4242");
        let live = |pid: u32| pid == 4242;
        let no_daemon = || false;
        assert!(verify_roots_quiescent(&r, &live, &no_daemon).is_err());
        let plan = build_plan(&r, &ResetOptions { purge_license: false, purge_local: false, purge_round: false });
        let mut noop = |_p: &str, _d: &str| {};
        assert!(execute_quarantine(&plan, &r, &live, &no_daemon, &mut noop).is_err());
        // 게이트 거부 시 부분 이동 0.
        assert!(r.cys_base.join("pack").exists());
        // 홀더 사망(또는 pid 재사용으로 cysd 아님) → 통과.
        let dead = |_pid: u32| false;
        assert!(verify_roots_quiescent(&r, &dead, &no_daemon).is_ok());
    }

    #[test]
    fn strip_removes_only_cys_hooks_and_restores_prev_statusline() {
        let td = test_home("strip");
        let home = &td;
        let cys_base = home.join(".cys");
        let s = home.join(".claude/settings.json");
        let pack_hook = format!("sh {}/pack/hooks/cys-hook.sh", cys_base.display());
        let dept_hook = format!("sh {}/pack-dept-d1/hooks/role-bootstrap.sh", cys_base.display());
        let status = format!(
            "CYS_PREV_STATUSLINE='my status' sh {}/pack/hooks/cys-statusline.sh",
            cys_base.display()
        );
        touch(
            &s,
            &serde_json::to_string_pretty(&serde_json::json!({
                "model": "opus",
                "hooks": {
                    "SessionStart": [
                        {"hooks": [
                            {"type": "command", "command": pack_hook},
                            {"type": "command", "command": "echo user-hook"}
                        ]},
                        {"hooks": [{"type": "command", "command": dept_hook}]}
                    ],
                    "Stop": [
                        {"hooks": [{"type": "command", "command": pack_hook}]}
                    ]
                },
                "statusLine": {"type": "command", "command": status}
            }))
            .unwrap(),
        );
        let removed = strip_cys_from_settings(&s, &cys_base).unwrap();
        assert!(!removed.is_empty());
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&s).unwrap()).unwrap();
        // 사용자 훅·타 설정 불가침.
        assert_eq!(root["model"], "opus");
        let ss = root["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 1, "empty entry must be pruned: {ss:?}");
        assert_eq!(ss[0]["hooks"][0]["command"], "echo user-hook");
        // cys 만 있던 이벤트는 통째 청소.
        assert!(root["hooks"].get("Stop").is_none());
        // statusLine 은 보존분 원복.
        assert_eq!(root["statusLine"]["command"], "my status");
        // 백업 생성.
        assert!(home.join(".claude/settings.json.bak-factory-reset").exists());
        // 멱등: 재실행은 무변경(백업 클로버 없음).
        let again = strip_cys_from_settings(&s, &cys_base).unwrap();
        assert!(again.is_empty());
    }

    /// ★A8 회귀: 오너 저작 오버레이(~/.cys/local)는 **기본 보존**이고, 그 훅 등록도 그대로 둔다
    /// (보존된 파일을 가리키므로 유효 — 제거하면 사용자 설정 파괴). --purge-local 이면 둘 다 함께.
    #[test]
    fn local_overlay_preserved_by_default_and_hooks_stay_consistent() {
        let td = test_home("localovl");
        let r = seed_practice_tree(&td);
        mk(&r.cys_base.join("local/hooks"));
        touch(&r.cys_base.join("local/hooks/owner-active.sh"), "#!/bin/sh\n");
        let settings = td.join(".claude/settings.json");
        let local_hook = format!("sh {}/local/hooks/owner-active.sh", r.cys_base.display());
        let pack_hook = format!("sh {}/pack/hooks/cys-hook.sh", r.cys_base.display());
        let write_settings = || {
            touch(
                &settings,
                &serde_json::to_string_pretty(&serde_json::json!({
                    "hooks": {"SessionStart": [{"hooks": [
                        {"type": "command", "command": local_hook},
                        {"type": "command", "command": pack_hook}
                    ]}]}
                }))
                .unwrap(),
            );
        };

        // 기본: local 은 keep, local 훅은 잔존(유효), pack 훅만 제거 → dangling 0.
        write_settings();
        let plan = build_plan(&r, &ResetOptions { purge_license: false, purge_local: false, purge_round: false });
        assert!(
            plan.keep.iter().any(|k| k.path.ends_with("local")),
            "local 은 기본 보존이어야 한다: {:?}",
            plan.keep.iter().map(|k| k.path.display().to_string()).collect::<Vec<_>>()
        );
        assert!(!plan.quarantine.iter().any(|i| i.path.ends_with("local")));
        strip_cys_from_settings_scoped(&settings, &r.cys_base, plan.purge_local).unwrap();
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let cmds = root["hooks"]["SessionStart"][0]["hooks"].as_array().unwrap();
        assert_eq!(cmds.len(), 1, "pack 훅만 제거되어야 한다: {cmds:?}");
        assert!(cmds[0]["command"].as_str().unwrap().contains("/local/hooks/"));

        // --purge-local: local 격리 + 그 훅도 함께 해제(격리했으면 dangling 이 되므로).
        write_settings();
        let plan2 = build_plan(&r, &ResetOptions { purge_license: false, purge_local: true, purge_round: false });
        assert!(plan2.quarantine.iter().any(|i| i.path.ends_with("local")));
        strip_cys_from_settings_scoped(&settings, &r.cys_base, plan2.purge_local).unwrap();
        let root2: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(
            root2["hooks"].get("SessionStart").is_none(),
            "local 격리 시엔 local 훅도 제거되어야 한다: {root2}"
        );
    }

    /// 파싱 실패는 여전히 거부한다(설정 클로버 금지). 심링크 처리는 P1-1 로 바뀌었으므로
    /// `strip_follows_symlink_to_real_file_in_home` 가 그 계약을 따로 잠근다.
    #[test]
    #[cfg(unix)] // symlink 픽스처 — Windows 는 권한 필요라 제외.
    fn strip_refuses_broken_json_and_dangling_symlink() {
        let td = test_home("striprefuse");
        let home = &td;
        let cys_base = home.join(".cys");
        // 끊어진 심링크(대상 부재) → canonicalize 실패 → 거부.
        let dangling = home.join(".claude/settings.json");
        std::fs::create_dir_all(dangling.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(home.join("nope.json"), &dangling).unwrap();
        assert!(strip_cys_from_settings(&dangling, &cys_base).is_err());
        // 파싱 실패 → 거부 + 원본 불변(빈 객체로 덮어쓰기 금지).
        let broken = home.join(".claude-2/settings.json");
        touch(&broken, "{not json");
        assert!(strip_cys_from_settings(&broken, &cys_base).is_err());
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), "{not json");
    }

    #[test]
    #[cfg(unix)] // symlink 픽스처 — Windows 는 권한 필요라 제외.
    fn skill_links_only_pack_targets_removed() {
        let td = test_home("skills");
        let home = &td;
        let cys_base = home.join(".cys");
        let skills = home.join(".claude/skills");
        mk(&skills);
        mk(&cys_base.join("pack/skills/appbuild"));
        mk(&home.join("my-own-skill"));
        std::os::unix::fs::symlink(cys_base.join("pack/skills/appbuild"), skills.join("appbuild"))
            .unwrap();
        std::os::unix::fs::symlink(home.join("my-own-skill"), skills.join("mine")).unwrap();
        mk(&skills.join("real-dir"));
        assert_eq!(remove_pack_skill_links(&skills, &cys_base), 1);
        assert!(!skills.join("appbuild").exists());
        assert!(skills.join("mine").exists(), "user symlink must survive");
        assert!(skills.join("real-dir").exists(), "real dir must survive");
    }

    /// ★W1 회귀: Windows 훅 형식(`bash "C:/…"`)에서도 이전 statusLine 이 원복된다.
    /// 이 경로가 깨지면 Windows 사용자의 statusLine 이 무음 삭제된다(맥만 정상 = 비대칭 사고).
    #[test]
    fn strip_restores_prev_statusline_windows_format() {
        let td = test_home("winstatus");
        let home = &td;
        let cys_base = home.join(".cys");
        let s = home.join(".claude/settings.json");
        // preflight._cys_hook_cmd 의 Windows 산출물과 동일 형식(정슬래시 + 따옴표).
        let win_cmd = format!(
            "CYS_PREV_STATUSLINE='my status' bash \"{}/pack/hooks/cys-statusline.sh\"",
            cys_base.to_string_lossy().replace('\\', "/")
        );
        touch(
            &s,
            &serde_json::to_string_pretty(&serde_json::json!({
                "statusLine": {"type": "command", "command": win_cmd}
            }))
            .unwrap(),
        );
        let removed = strip_cys_from_settings(&s, &cys_base).unwrap();
        assert!(removed.iter().any(|r| r.contains("원복")), "{removed:?}");
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&s).unwrap()).unwrap();
        assert_eq!(root["statusLine"]["command"], "my status");
    }

    /// ★A3 회귀(ABSOLUTE ANCHOR): 센티널은 **fail-open** 이어야 한다 — 만료·고아 센티널이
    /// 데몬 기동을 영구히 막으면 '전 pane 사망' 등급 사고가 된다. 신선+생존일 때만 차단한다.
    #[test]
    fn sentinel_is_fail_open_and_self_cleaning() {
        // ★격리(2026-08-24): 종전 이 테스트는 **라이브 `$HOME/.local/state/…` 센티널**을 쓰고
        //   지웠다. 그래서 ⓐ러너가 둘 이상 돌면 무작위 적색이고 ⓑ진짜 리셋이 진행 중이면
        //   테스트가 그 가드를 삭제했다(= 전 pane 사망 등급 안전장치 무력화). 지금은 프로세스
        //   고유 임시 경로만 쓴다 — 사용자 홈 무접촉·동시 실행 안전.
        let td = test_home("sentinel");
        let sp = td.join("sentinel-marker");
        let _env = crate::pack::EnvGuard::set(ENV_RESET_SENTINEL, &sp);
        let path = sentinel_path().expect("오버라이드가 있으면 경로는 항상 산출된다");
        assert_eq!(path, sp, "env 오버라이드가 센티널 경로에 반영되지 않았다(격리 실패)");
        assert!(
            !path.starts_with(dirs::home_dir().unwrap_or_default().join(".local/state")),
            "테스트가 여전히 라이브 상태 루트를 쓴다: {}",
            path.display()
        );
        let write = |body: &str| {
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p).unwrap();
            }
            std::fs::write(&path, body).unwrap();
        };

        // ① 신선 + 자기 pid 생존 → 차단.
        // 라이브 판정자는 "cys 계열 프로세스"를 요구하므로(테스트 바이너리는 아니다) 주입한다.
        let alive = |_pid: u32| true;
        let dead = |_pid: u32| false;
        write(&format!("{} {}", now_unix(), std::process::id()));
        assert!(reset_in_progress_with(&alive), "신선한 센티널은 차단해야 한다");
        // ★pid 재사용 오탐 차단: 주인이 cys 계열이 아니면(=남의 프로세스) 무효 + 자동 청소.
        assert!(!reset_in_progress_with(&dead), "남의 pid 는 리셋 주인이 아니다");
        assert!(!path.exists());

        // ② TTL 초과 → 무시 + 자동 청소.
        write(&format!("{} {}", now_unix() - SENTINEL_TTL_SECS - 1, std::process::id()));
        assert!(!reset_in_progress_with(&alive), "만료 센티널은 무시해야 한다");
        assert!(!path.exists(), "만료 센티널은 자동 청소되어야 한다");

        // ③ 죽은 pid(예약 상한 근처의 미사용 pid) → 무시 + 청소.
        write(&format!("{} 4294967294", now_unix()));
        assert!(!reset_in_progress_with(&dead), "고아 센티널은 무시해야 한다");
        assert!(!path.exists());

        // ④ 형식 불량 → 무시.
        write("garbage");
        assert!(!reset_in_progress_with(&alive));

        // ⑤ 재부팅 후 pid 재사용 → 무시(부팅 식별자 불일치). 이 검사가 없으면 살아있는 남의
        // 프로세스를 리셋 주인으로 오판해 데몬이 TTL 동안 못 뜬다(전 pane 사망 등급).
        write(&format!("{} {} {}", now_unix(), std::process::id(), boot_id() + 100_000));
        assert!(!reset_in_progress_with(&alive), "다른 부팅의 센티널은 무효여야 한다");
        assert!(!path.exists());

        // ⑤-b Windows boot_time 흔들림(초 단위)은 같은 부팅으로 본다 — 정확 일치를 요구하면
        // 살아있는 센티널을 스스로 지워 가드가 무력화된다(맥에선 재현 안 되는 Windows 사고).
        write(&format!("{} {} {}", now_unix(), std::process::id(), boot_id() + 3));
        assert!(reset_in_progress_with(&alive), "초 단위 흔들림은 같은 부팅으로 인정해야 한다");

        // ⑥ 구 형식(부팅 식별자 없음)은 호환 — 신선+생존이면 여전히 차단(fail-open 회귀 금지).
        write(&format!("{} {}", now_unix(), std::process::id()));
        assert!(reset_in_progress_with(&alive), "구 형식 센티널도 인식해야 한다");

        // ⑤ RAII 가드는 Drop 에서 반드시 해제한다.
        {
            let _g = ResetSentinel::arm();
            assert!(reset_in_progress_with(&alive));
        }
        assert!(!reset_in_progress_with(&alive), "가드 Drop 후 센티널이 남으면 안 된다");

        // ⑦ 오버라이드 계약 자신 — 상대경로·빈 값은 **무시**하고 기본(홈 파생)으로 되돌아간다.
        //    (오타 하나로 쓰는 쪽과 읽는 쪽이 다른 파일을 보는 것 = 가드 조용한 무력화.)
        {
            let _rel = crate::pack::EnvGuard::set(ENV_RESET_SENTINEL, "relative/path");
            let d = sentinel_path().expect("홈이 있으면 기본 경로 산출");
            assert!(d.is_absolute() && d.ends_with(".cys-factory-reset-in-progress"),
                    "상대경로 오버라이드가 채택됐다: {}", d.display());
        }
        {
            let _empty = crate::pack::EnvGuard::set(ENV_RESET_SENTINEL, "");
            let d = sentinel_path().expect("홈이 있으면 기본 경로 산출");
            assert!(d.ends_with(".cys-factory-reset-in-progress"),
                    "빈 값 오버라이드가 채택됐다: {}", d.display());
        }
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★P0-3 회귀: `--purge-round` opt-in 이면 프로젝트 _round 도 격리 대상이 된다(대칭 계약).
    #[test]
    fn purge_round_optin_includes_project_rounds() {
        let td = test_home("purgeround");
        let r = seed_practice_tree(&td);
        std::fs::create_dir_all(r.workspace_root.join("projB/_round")).unwrap();
        let plan = build_plan(
            &r,
            &ResetOptions { purge_license: false, purge_local: false, purge_round: true },
        );
        let q: Vec<String> = plan.quarantine.iter().map(|i| norm(&i.path)).collect();
        assert!(q.iter().any(|p| p.ends_with("proj-a/_round")), "{q:?}");
        assert!(q.iter().any(|p| p.ends_with("projB/_round")), "{q:?}");
    }

    /// ★P0-2 회귀: 사용자 폴더 밖 항목은 `outside_state` 로 표식돼 모달이 경로를 노출할 수 있다.
    #[test]
    fn outside_state_flag_marks_user_folder_items() {
        let td = test_home("outside");
        let r = seed_practice_tree(&td);
        let plan = build_plan(
            &r,
            &ResetOptions { purge_license: false, purge_local: false, purge_round: true },
        );
        let outside: Vec<String> = plan
            .quarantine
            .iter()
            .filter(|i| i.outside_state)
            .map(|i| norm(&i.path))
            .collect();
        assert!(outside.iter().any(|p| p.ends_with("proj-a/_round")), "{outside:?}");
        // ~/.cys·상태 루트 항목은 강조 대상이 아니다(모두 강조하면 강조가 의미를 잃는다).
        assert!(!outside.iter().any(|p| p.contains("/.cys/pack")), "{outside:?}");
    }

    /// ★P0-6 회귀: 격리 목적지가 못 쓰는 상태면 **계획 단계에서** 걸러진다(데몬 무접촉).
    #[test]
    fn trash_root_probe_rejects_before_touching_daemons() {
        let td = test_home("trashprobe");
        let r = seed_practice_tree(&td);
        // cys-trash 자리에 파일을 둔다(실사고 재현).
        let _ = std::fs::remove_dir_all(&r.trash_root);
        std::fs::create_dir_all(r.trash_root.parent().unwrap()).unwrap();
        std::fs::write(&r.trash_root, b"not a dir").unwrap();
        let plan = build_plan(
            &r,
            &ResetOptions { purge_license: false, purge_local: false, purge_round: false },
        );
        assert!(plan.trash_root_ready.is_err(), "파일이면 사전 거부해야 한다");
        // 정상 상태에서는 통과한다.
        std::fs::remove_file(&r.trash_root).unwrap();
        let plan2 = build_plan(
            &r,
            &ResetOptions { purge_license: false, purge_local: false, purge_round: false },
        );
        assert!(plan2.trash_root_ready.is_ok(), "{:?}", plan2.trash_root_ready);
    }

    /// ★P0-5 회귀: 저널을 못 쓰면 **옮기지 않는다**(복구 지도 없는 이동 금지).
    #[test]
    #[cfg(unix)]
    fn journal_write_failure_blocks_the_move() {
        use std::os::unix::fs::PermissionsExt;
        let td = test_home("journalfail");
        let r = seed_practice_tree(&td);
        let plan = build_plan(
            &r,
            &ResetOptions { purge_license: false, purge_local: false, purge_round: false },
        );
        std::fs::create_dir_all(&plan.trash_dir).unwrap();
        // 저널을 읽기 전용으로 미리 만들어 append 를 실패시킨다.
        let jp = plan.trash_dir.join("journal.ndjson");
        std::fs::write(&jp, b"").unwrap();
        std::fs::set_permissions(&jp, std::fs::Permissions::from_mode(0o444)).unwrap();
        let not_cysd = |_p: u32| false;
        let no_daemon = || false;
        let mut noop = |_p: &str, _d: &str| {};
        let rep = execute_quarantine(&plan, &r, &not_cysd, &no_daemon, &mut noop);
        // 저널 자체를 열지 못하면 Err, 열려도 쓰기 실패면 전 항목이 failed 로 남고 이동 0.
        match rep {
            Err(_) => {}
            Ok(rep) => {
                assert!(rep.moved.is_empty(), "지도를 못 남기면 옮기면 안 된다: {:?}", rep.moved);
                assert!(!rep.failed.is_empty());
                assert!(r.cys_base.join("pack").exists(), "원위치 보존");
            }
        }
        let _ = std::fs::set_permissions(&jp, std::fs::Permissions::from_mode(0o644));
    }

    /// ★P0-4/P0-5 회귀: manifest 를 못 써도 **격리는 성공으로 계속**되고(훅 해제까지 수행)
    /// 보고에 manifest_written=false 로 정직하게 드러난다 + REPORT.txt 가 남는다.
    #[test]
    fn report_persists_and_counts_are_decomposed() {
        let td = test_home("report");
        let r = seed_practice_tree(&td);
        // 계획에 있으나 실행 전 사라지는 항목을 만든다(skipped_absent 검증).
        let plan = build_plan(
            &r,
            &ResetOptions { purge_license: false, purge_local: false, purge_round: false },
        );
        std::fs::remove_dir_all(r.cys_base.join("state-generations")).unwrap();
        let not_cysd = |_p: u32| false;
        let no_daemon = || false;
        let mut noop = |_p: &str, _d: &str| {};
        let rep = execute_quarantine(&plan, &r, &not_cysd, &no_daemon, &mut noop).unwrap();
        assert!(rep.manifest_written);
        assert!(rep.skipped_absent >= 1, "사라진 항목이 집계돼야 한다");
        let txt = std::fs::read_to_string(plan.trash_dir.join("REPORT.txt")).unwrap();
        assert!(txt.contains("cys 완전 초기화 결과"));
        assert!(txt.contains("이미 없음"));
        assert!(txt.contains("--undo"));
    }

    /// ★P0(복구) 회귀: undo 가 격리본을 원위치로 되돌리고, 원위치가 차 있으면 덮어쓰지 않는다.
    #[test]
    fn undo_restores_and_never_overwrites() {
        let td = test_home("undo");
        let r = seed_practice_tree(&td);
        let plan = build_plan(
            &r,
            &ResetOptions { purge_license: false, purge_local: false, purge_round: false },
        );
        let not_cysd = |_p: u32| false;
        let no_daemon = || false;
        let mut noop = |_p: &str, _d: &str| {};
        let rep = execute_quarantine(&plan, &r, &not_cysd, &no_daemon, &mut noop).unwrap();
        assert!(!r.cys_base.join("pack").exists());

        // 재온보딩으로 새로 생긴 상태를 흉내 — 이 자리는 덮어쓰지 않아야 한다.
        std::fs::create_dir_all(r.cys_base.join("pack")).unwrap();

        let up = read_undo_plan(&rep.trash_dir).unwrap();
        assert!(
            up.blocked.iter().any(|(_, to)| norm(to).ends_with(".cys/pack")),
            "{:?}",
            norm_all(&up.blocked.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>())
        );
        let (restored, failed) = execute_undo(&up, &mut noop);
        assert!(failed.is_empty(), "{failed:?}");
        assert!(restored > 0);
        assert!(r.state_root.join("cys").exists(), "데몬 상태가 원위치로 돌아와야 한다");
    }

    /// ★P0(복구) 회귀: manifest 가 없으면 journal.ndjson 으로 복구 지도를 읽는다(중도 중단분).
    #[test]
    fn undo_falls_back_to_journal_when_manifest_missing() {
        let td = test_home("undojournal");
        let r = seed_practice_tree(&td);
        let plan = build_plan(
            &r,
            &ResetOptions { purge_license: false, purge_local: false, purge_round: false },
        );
        let not_cysd = |_p: u32| false;
        let no_daemon = || false;
        let mut noop = |_p: &str, _d: &str| {};
        let rep = execute_quarantine(&plan, &r, &not_cysd, &no_daemon, &mut noop).unwrap();
        std::fs::remove_file(rep.trash_dir.join("manifest.json")).unwrap();
        let up = read_undo_plan(&rep.trash_dir).unwrap();
        assert_eq!(up.source, "journal.ndjson");
        assert!(!up.entries.is_empty());
        // 중단 흔적으로도 감지된다(다음 리셋 프리뷰가 고지).
        assert!(interrupted_prior_resets(&r.trash_root)
            .iter()
            .any(|p| p == &rep.trash_dir));
    }

    /// ★P0-1 트립와이어: 센티널 **배선 누락**을 CI 가 잡는다.
    /// 이 결함(arm() 호출부가 테스트뿐)이 시뮬레이션에서 발견된 방식 그대로를 기계 단언으로 굳힌다:
    /// ①실행 경로 두 곳(CLI·GUI)이 `ResetSentinel::arm()` 을 부른다 ②stop 단계는 센티널을
    /// 직접 쓰지 않는다(해제자 없는 무장 금지) ③stop 단계가 plist 를 삭제하지 않는다(P0-6).
    #[test]
    fn sentinel_and_stop_phase_wiring_pins() {
        let cli = include_str!("bin/cys.rs");
        assert!(
            cli.contains("ResetSentinel::arm()"),
            "CLI 실행 경로에 센티널 무장이 없다 — 리셋 중 데몬 부활을 막지 못한다"
        );
        let me = include_str!("factory_reset.rs");
        let start = me
            .find("pub fn stop_daemons_and_unregister")
            .expect("stop_daemons_and_unregister 소실 — 트립와이어 재배선 필요");
        let seg = &me[start..start + me[start..].find("\npub fn ").unwrap_or(me.len() - start)];
        // 주석은 계약 설명(왜 하지 않는가)을 담으므로 **코드 줄만** 판정한다.
        let code: String = seg
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let seg = code.as_str();
        assert!(
            !seg.contains("write_sentinel()"),
            "stop 단계가 센티널을 직접 무장했다 — 해제자(Drop)가 없는 경로가 되살아났다(P0-1)"
        );
        assert!(
            !seg.contains("remove_file(plist)"),
            "stop 단계가 plist 를 삭제한다 — 격리 실패 시 자동시작만 잃는 경로가 되살아났다(P0-6)"
        );
    }

    /// ★P1-1 ①: 심링크 settings 는 거부가 아니라 **실파일 추종**(홈 아래 일반 파일일 때만).
    /// 도트파일 저장소 구성에서 리셋 후 매 세션 훅 오류가 나던 결함의 회귀 핀.
    #[test]
    #[cfg(unix)]
    fn strip_follows_symlink_to_real_file_in_home() {
        let td = test_home("symfollow");
        let home = &td;
        let cys_base = home.join(".cys");
        let real = home.join("dotfiles/settings.json");
        let pack_hook = format!("sh {}/pack/hooks/cys-hook.sh", cys_base.display());
        touch(
            &real,
            &serde_json::to_string_pretty(&serde_json::json!({
                "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": pack_hook}]}]}
            }))
            .unwrap(),
        );
        let link = home.join(".claude/settings.json");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        // 홈 아래 실파일 → 추종해서 제거한다(링크 자체는 그대로 링크로 남는다).
        let removed = strip_cys_from_settings(&link, &cys_base).unwrap();
        assert!(!removed.is_empty(), "심링크 대상 실파일에서 제거되어야 한다");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&real).unwrap()).unwrap();
        assert!(root["hooks"].get("SessionStart").is_none(), "{root}");

        // 홈 **밖** 대상은 여전히 거부(클로버 금지 계약 보존).
        let outside_dir = std::env::temp_dir().join(format!("cys-freset-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside_dir);
        std::fs::create_dir_all(&outside_dir).unwrap();
        let outside = outside_dir.join("settings.json");
        touch(&outside, "{}");
        let link2 = home.join(".claude-2/settings.json");
        std::fs::create_dir_all(link2.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&outside, &link2).unwrap();
        assert!(strip_cys_from_settings(&link2, &cys_base).is_err());
    }

    /// ★P1-1 ②: needle 경계 일치 — 사용자가 만든 `~/.cys/pack-notes` 훅은 우리 것이 아니다.
    #[test]
    fn needle_matches_pack_boundary_only() {
        // 개인 경로 리터럴 금지(secret-scan 하드 게이트) — 더미 홈 루트로 조립한다.
        let home = "/dummy-home";
        let base_s = format!("{home}/.cys");
        let base = Path::new(&base_s);
        assert!(command_points_into_pack(&format!("sh {base_s}/pack/hooks/a.sh"), base));
        assert!(command_points_into_pack(&format!("sh {base_s}/pack-dept-d1/hooks/a.sh"), base));
        assert!(command_points_into_pack(&format!("bash \"{base_s}/pack/hooks/a.sh\""), base));
        // 사용자 저작 디렉토리 — 파일은 보존되므로 훅도 보존해야 대칭이다.
        assert!(!command_points_into_pack(&format!("sh {base_s}/pack-notes/hooks/a.sh"), base));
        assert!(!command_points_into_pack(&format!("sh {base_s}/packages/x.sh"), base));
    }

    /// G3 축1: 특정 팩 루트 소유 판정(hooks-prune) — 경계 일치·역슬래시 정규화·형제 팩 무오판.
    #[test]
    fn pack_root_boundary_match() {
        let root = Path::new("/home/u/.cys/pack-dept-d1");
        assert!(command_points_into_pack_root("sh /home/u/.cys/pack-dept-d1/hooks/a.sh", root));
        assert!(command_points_into_pack_root(
            "bash \"/home/u/.cys/pack-dept-d1/hooks/a.sh\"",
            root
        ));
        // Windows 훅 명령(역슬래시) 정규화
        assert!(command_points_into_pack_root(
            "bash \"C:\\Users\\x\\.cys\\pack-dept-d1\\hooks\\a.sh\"",
            Path::new("C:\\Users\\x\\.cys\\pack-dept-d1")
        ));
        // 형제 팩 오판 금지: d1 판정이 d10 을 잡으면 사용자/타 부서 훅 오삭제다
        assert!(!command_points_into_pack_root("sh /home/u/.cys/pack-dept-d10/hooks/a.sh", root));
        // base 팩 루트 판정은 부서 팩을 잡지 않는다(--allow-base prune 이 부서 훅을 못 지움)
        assert!(!command_points_into_pack_root(
            "sh /home/u/.cys/pack-dept-d1/hooks/a.sh",
            Path::new("/home/u/.cys/pack")
        ));
        assert!(!command_points_into_pack_root("sh /home/u/myhooks/a.sh", root));
    }

    /// G3 축1(hooks_prune_scope): base 훅·사용자 외부 훅·형제 부서 훅 혼재에서 **대상 부서 훅만**
    /// 제거 — 순서/타 항목 보존 · 백업(.bak-cys-dept) 생성 · 2회차 무변경 무쓰기(멱등) ·
    /// statusLine 무접촉(훅 항목만) · 죽은 경로(팩 dir 부재)도 제거(치유 대상).
    #[test]
    fn hooks_prune_scope_removes_only_target_pack() {
        let home = std::env::temp_dir().join(format!(
            "cys-hooks-prune-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        let base = home.join(".cys");
        std::fs::create_dir_all(&base).unwrap();
        let dept = base.join("pack-dept-d1"); // 의도적으로 디스크 미생성 = 죽은 경로 치유 검증
        let base_s = base.to_string_lossy().into_owned();
        let settings = home.join("settings.json");
        let body = serde_json::json!({
            "theme": "dark",
            "statusLine": {"type": "command",
                           "command": format!("sh {base_s}/pack-dept-d1/hooks/status.sh")},
            "hooks": {
                "SessionStart": [
                    {"hooks": [{"type": "command",
                                "command": format!("sh {base_s}/pack/hooks/session-start.sh")}]},
                    {"hooks": [{"type": "command",
                                "command": format!("sh {base_s}/pack-dept-d1/hooks/session-start.sh")}]},
                    {"hooks": [{"type": "command",
                                "command": format!("sh {base_s}/pack-dept-d10/hooks/session-start.sh")}]},
                    {"hooks": [{"type": "command", "command": "sh /home/u/myhooks/mine.sh"}]}
                ],
                "UserPromptSubmit": [
                    {"hooks": [{"type": "command",
                                "command": format!("sh {base_s}/pack-dept-d1/hooks/role-bootstrap.sh")}]}
                ]
            }
        });
        std::fs::write(&settings, serde_json::to_string_pretty(&body).unwrap()).unwrap();

        // 읽기 전용 판정판(dry-run·doctor 탐지)이 같은 술어로 같은 라벨을 낸다 + 무변경.
        let before = std::fs::read_to_string(&settings).unwrap();
        let mut probe = hooks_pointing_into_pack(&settings, &dept).unwrap();
        probe.sort();
        assert_eq!(probe, vec!["SessionStart×1".to_string(), "UserPromptSubmit×1".to_string()]);
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), before, "판정판이 파일을 썼다");

        let removed = strip_hooks_pointing_into_pack(&settings, &dept, None).unwrap();
        assert_eq!(removed.len(), 2, "부서 훅 2건(이벤트 2종) 제거 기대: {removed:?}");
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["theme"], "dark", "비-훅 키 소실");
        assert_eq!(
            after["statusLine"]["command"],
            format!("sh {base_s}/pack-dept-d1/hooks/status.sh"),
            "hooks-prune 이 statusLine 을 건드렸다(훅 항목만 계약 위반)"
        );
        let ss = after["hooks"]["SessionStart"].as_array().unwrap();
        let cmds: Vec<&str> =
            ss.iter().map(|e| e["hooks"][0]["command"].as_str().unwrap()).collect();
        assert_eq!(
            cmds,
            vec![
                format!("sh {base_s}/pack/hooks/session-start.sh"),
                format!("sh {base_s}/pack-dept-d10/hooks/session-start.sh"),
                "sh /home/u/myhooks/mine.sh".to_string()
            ],
            "base·형제 부서·사용자 훅 보존 + 순서 보존 위반"
        );
        assert!(
            after["hooks"].get("UserPromptSubmit").is_none(),
            "빈 이벤트 껍데기가 청소되지 않았다"
        );
        let backup = home.join("settings.json.bak-cys-dept");
        assert!(backup.exists(), "per-run 백업(.bak-cys-dept) 부재");
        let backup_body: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&backup).unwrap()).unwrap();
        assert_eq!(
            backup_body["hooks"]["SessionStart"].as_array().unwrap().len(),
            4,
            "백업이 제거 전 원본이 아니다(복원 불가)"
        );

        // 멱등: 2회차 무변경·무쓰기(백업 클로버 없음 — 원본 mtime/내용 불변으로 판정)
        let after_body = std::fs::read_to_string(&settings).unwrap();
        std::fs::write(&backup, "sentinel").unwrap();
        let removed2 = strip_hooks_pointing_into_pack(&settings, &dept, None).unwrap();
        assert!(removed2.is_empty(), "멱등 위반: {removed2:?}");
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), after_body, "무변경인데 재작성");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "sentinel",
            "무변경 재실행이 백업을 클로버했다"
        );

        // 파싱 실패 = 거부(측정 불능 ≠ 통과) · 파일 부재 = 대상 없음(Ok 빈 벡터)
        let broken = home.join("broken.json");
        std::fs::write(&broken, "{not json").unwrap();
        assert!(strip_hooks_pointing_into_pack(&broken, &dept, None).is_err());
        assert!(hooks_pointing_into_pack(&broken, &dept).is_err());
        assert!(strip_hooks_pointing_into_pack(&home.join("absent.json"), &dept, None)
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ★리뷰 MINOR(관측 일치) 핀: 탐지판(dry-run·프로브)과 제거판의 symlink 정책이 **한 판정**
    /// (`resolve_symlinked_settings`)이다 — dry-run 이 '제거 예정'을 약속해 놓고 실행이 거부
    /// (exit 1)하던 불일치 봉인. 홈 밖 링크=둘 다 거부 · 홈 안 일반 파일 링크=둘 다 실파일 대상
    /// (라벨 일치·링크 불가침).
    #[cfg(unix)]
    #[test]
    fn probe_and_strip_agree_on_symlink_policy() {
        let root = std::env::temp_dir().join(format!("cys-fr-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let dept = home.join(".cys").join("pack-dept-s1");
        std::fs::create_dir_all(&dept).unwrap();
        let hook_body = serde_json::json!({
            "hooks": {"SessionStart": [
                {"hooks": [{"type": "command",
                            "command": format!("sh {}/hooks/session-start.sh", dept.display())}]}
            ]}
        });

        // ① 홈 밖 실파일을 가리키는 링크: 탐지판·제거판 동일 거부 + 실파일 무변조.
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let out_file = outside.join("settings.json");
        std::fs::write(&out_file, serde_json::to_string(&hook_body).unwrap()).unwrap();
        let link = home.join("settings-out.json");
        std::os::unix::fs::symlink(&out_file, &link).unwrap();
        let before = std::fs::read_to_string(&out_file).unwrap();
        assert!(
            hooks_pointing_into_pack(&link, &dept).is_err(),
            "탐지판이 홈 밖 링크의 제거를 약속했다(제거판은 거부 — 관측 불일치)"
        );
        assert!(strip_hooks_pointing_into_pack(&link, &dept, None).is_err());
        assert_eq!(
            std::fs::read_to_string(&out_file).unwrap(),
            before,
            "거부 경로가 홈 밖 실파일을 썼다"
        );

        // ② 홈 안 일반 파일을 가리키는 링크: 둘 다 실파일 대상 — 탐지 라벨 == 제거 라벨.
        let inner = home.join("real-settings.json");
        std::fs::write(&inner, serde_json::to_string(&hook_body).unwrap()).unwrap();
        let link2 = home.join("settings-in.json");
        std::os::unix::fs::symlink(&inner, &link2).unwrap();
        let probe = hooks_pointing_into_pack(&link2, &dept).unwrap();
        assert_eq!(probe, vec!["SessionStart×1".to_string()]);
        let removed = strip_hooks_pointing_into_pack(&link2, &dept, None).unwrap();
        assert_eq!(removed, probe, "dry-run 약속과 실행 결과 불일치");
        assert!(
            std::fs::symlink_metadata(&link2).unwrap().file_type().is_symlink(),
            "링크 자체를 건드렸다(계약 위반)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ★P1-1 ③⑤: 백업은 격리 폴더 안으로, 원본 권한(0444 잠금)은 보존된다.
    #[test]
    #[cfg(unix)]
    fn strip_backups_go_to_trash_and_permissions_survive() {
        use std::os::unix::fs::PermissionsExt;
        let td = test_home("stripbak");
        let home = &td;
        let cys_base = home.join(".cys");
        let s = home.join(".claude/settings.json");
        let pack_hook = format!("sh {}/pack/hooks/cys-hook.sh", cys_base.display());
        touch(
            &s,
            &serde_json::to_string_pretty(&serde_json::json!({
                "hooks": {"Stop": [{"hooks": [{"type": "command", "command": pack_hook}]}]}
            }))
            .unwrap(),
        );
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o444)).unwrap();
        let backup_dir = home.join("trash/settings-backups");
        let removed = strip_cys_from_settings_to(&s, &cys_base, false, &backup_dir).unwrap();
        assert!(!removed.is_empty());
        assert!(backup_dir.join(".claude.settings.json").exists(), "백업이 격리 폴더에 있어야 한다");
        assert!(
            !home.join(".claude/settings.json.bak-factory-reset").exists(),
            "프로필에 백업을 새로 만들면 안 된다(cys 흔적 증식)"
        );
        let mode = std::fs::metadata(&s).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o444, "원본 권한이 보존되어야 한다");
    }

    /// ★P2-1 회귀: 육안상 같은 입력은 같게 받는다 — 따옴표 동반 복사·NFD 자모·NBSP/전각/ZWSP.
    /// 이걸 안 하면 사용자는 "분명히 똑같이 쳤는데" 거부당하고 재입력 기회도 없었다.
    #[test]
    fn confirm_phrase_normalization_accepts_lookalikes() {
        let want = "완전 초기화";
        let same = [
            "완전 초기화",
            "  완전 초기화  ",
            "\"완전 초기화\"",              // 프롬프트에서 드래그 복사
            "'완전 초기화'",
            "\u{201C}완전 초기화\u{201D}",   // 곡선 따옴표(자동 교정)
            "완전\u{00A0}초기화",            // NBSP
            "완전\u{3000}초기화",            // 전각 공백
            "완전 \u{200B}초기화",           // ZWSP
            "완전  초기화",                  // 공백 중복
            // NFD(자모 분리) — macOS 파일시스템·일부 입력기 산출물
            "\u{110B}\u{116A}\u{11AB}\u{110C}\u{1165}\u{11AB} \u{110E}\u{1169}\u{1100}\u{1175}\u{1112}\u{116A}",
        ];
        for s in same {
            assert_eq!(
                normalize_confirm_phrase(s),
                want,
                "같은 문구로 받아야 한다: {s:?}"
            );
        }
        // 다른 문구는 여전히 다르다(관용이 정확성을 삼키면 안 된다).
        for s in ["완전초기화", "완전 삭제", "초기화", ""] {
            assert_ne!(normalize_confirm_phrase(s), want, "{s:?}");
        }
    }

    #[test]
    fn shell_unquote_single_variants() {
        assert_eq!(shell_unquote_single("'my status'").as_deref(), Some("my status"));
        assert_eq!(shell_unquote_single("plain").as_deref(), Some("plain"));
        assert_eq!(
            shell_unquote_single("'it'\"'\"'s ok'").as_deref(),
            Some("it's ok")
        );
        assert_eq!(shell_unquote_single(""), None);
    }
}
