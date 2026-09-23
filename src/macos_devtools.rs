//! U15(0.14.41 · 오너 항목 「개발자 도구 없는 맥의 설치 창 · 자동 점검이 조용히 멈춤」) —
//! CLT(명령어 라인 개발자 도구) 판정과 그 판정을 소비하는 세 자리의 **단일 정의처**.
//!
//! 【사실(조사 U15 · 반박 U15.refute 확정)】 macOS 의 `/usr/bin/python3`·`/usr/bin/git`(+ cc·make 등
//! 78개 하드링크)은 진짜 프로그램이 아니라 "개발자 도구를 설치하라"는 **시스템 창을 띄우는 껍데기(셔임)**
//! 다. CLT 도 Xcode.app 도 없는 맥에서 이것을 실행하면 창이 뜨고 명령은 비0으로 끝난다(저장소 선례
//! bbdfcde5 finding 7). 에이전트 좌석은 대화형 로그인 셸(`zsh -l`)이라 path_helper 가 `/usr/bin` 을
//! 동봉 runtime 앞으로 되돌린다(cysd state.rs 대화형 `-l` 갈래 — 5e34723c 가 '문서화 엣지'로 남긴 그
//! 자리가 실은 launch-agent·boot 좌석 전부의 본 경로다). 그래서 훅·부트 자식·회수가 PATH 의 `python3`
//! 로 셔임을 부르고, 훅은 fail-open 이라 **아무 일도 없는 것처럼** 멈춘다.
//!
//! 【이 모듈의 계약】
//!   ① **셔임을 실행하지 않는다.** 판정은 디스크 stat 만 한다(프로세스 스폰 0).
//!   ② 판정 순서는 libxcselect 기본 탐색과 같다: `DEVELOPER_DIR`(설정됐고 디렉터리면) →
//!      `/var/db/xcode_select_link` → `/Applications/Xcode.app/Contents/Developer` →
//!      `/Library/Developer/CommandLineTools`. **처음 실재하는 개발자 디렉터리 하나**를 고르고
//!      (선택은 하나다 — 뒤 후보로 넘어가지 않는다), 그 안의 `usr/bin/<tool>` 이 실행 가능 파일이면
//!      present 다.
//!   ③ 소비자 셋 — pane·스케줄·GUI 스폰 env ⑦(`CYS_PY` + 마커) · 부트 감독 자식 PATH · `cys boot`
//!      회수 자식 PATH — 은 전부 [`clt_absent_bundled_python_for`] **한 판정**을 거친다:
//!      macOS ∧ 마스터 롤백(`CYS_BOOT_GATES=0`) 아님 ∧ 동봉 python3 실재 ∧ CLT python3 부재일 때만
//!      Some. 그 밖(윈도우·리눅스·CLT 있는 맥·롤백)은 None 이고, 소비자 출력이 **종전과 바이트 단위로
//!      같다**(조건부 쌍 규율 — lib.rs ⑤ CLAUDE_CODE_GIT_BASH_PATH · ⑥ npm_config_prefix 와 같다).
//!   ④ 셸 짝(`cysjavis-pack/hooks/_lib.sh` `cys_clt_tool_present`)은 같은 후보 표를 쓴다 — 표 문면
//!      파리티는 아래 테스트 `shell_twin_uses_the_same_candidate_table` 가 대조한다.
//!
//! 【한계(정직)】 Xcode.app 이 선택됐지만 라이선스에 동의하지 않은 맥은 present 로 판정되는데 셔임은
//! 라이선스 오류로 실패한다(창은 뜨지 않는다 · 설계 D4 · 이번 범위 밖). 에이전트가 Bash 도구에서
//! 직접 치는 `python3`·`git`, Claude Code 자신의 git 호출은 이 모듈이 닿지 않는다(설계 §2 제외:
//! A4 CLAUDE_ENV_FILE · Phase B git 동봉 전환) — 그래서 알림 문구가 그 사실을 말한다.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// libxcselect 가 가장 먼저 보는 env — 설정됐고 디렉터리면 그것이 **선택된** 개발자 디렉터리다.
pub const ENV_DEVELOPER_DIR: &str = "DEVELOPER_DIR";

/// `DEVELOPER_DIR` 다음의 기본 탐색 후보(순서 = 우선순위). 셸 짝 `_lib.sh` 가 같은 표를 쓴다.
/// `/var/db/xcode_select_link` 는 `xcode-select -s` 가 만드는 심링크라 `is_dir()` 이 링크를 따라간다.
pub const DEV_DIR_CANDIDATES: [&str; 3] = [
    "/var/db/xcode_select_link",
    "/Applications/Xcode.app/Contents/Developer",
    "/Library/Developer/CommandLineTools",
];

/// 훅 프리루드가 존중하는 인터프리터 env(`_lib.sh` `cys_resolve_py` 첫 갈래 — 미리 설정된 값 우선).
pub const ENV_CYS_PY: &str = "CYS_PY";
/// ⑦ 조건부 쌍이 **왜** 실렸는지 남기는 마커 키. 소비자: `cys-dept` 헤더(마커가 있을 때만 PATH 선두).
pub const ENV_CYS_PY_ORIGIN: &str = "CYS_PY_ORIGIN";
/// 마커 값 — "CLT 가 없는 맥이라 동봉 python 을 명시했다". 셸 쪽은 이 문자열과 **정확 일치**만 인정한다.
pub const CYS_PY_ORIGIN_CLT_ABSENT: &str = "bundled-clt-absent";

/// 실행 가능한 **일반 파일**인가(디렉터리·실행 비트 없는 파일은 아니다). 비-unix 는 파일 실재로 접는다.
fn is_executable_file(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// libxcselect 탐색 모사 — **선택된** 개발자 디렉터리(순수: env·후보를 인자로 받고 디스크 stat 만 한다).
///
/// `DEVELOPER_DIR` 가 비어 있지 않으면: ★MINOR-4(리뷰1 · `man xcode-select` ENVIRONMENT 절 실측) —
/// 이 값은 실제 Developer contents 디렉터리여도 되고, **Xcode 앱 번들 루트**(예:
/// `/Applications/Xcode-beta.app`)여도 된다. 후자는 xcode-select 가 설치하는 셔임들이 내부에서
/// `Contents/Developer` 로 **자동 변환**한다("the xcode-select provided shims will automatically
/// convert to the full Developer contents subdirectory"). 그래서 먼저 `<DEVELOPER_DIR>/Contents/
/// Developer` 가 디렉터리면 그것을 고르고(앱 루트 케이스 — 이 축 없으면 앱 루트를 쓰는 개발자
/// 기계를 "CLT 없음"으로 오판해 불필요하게 동봉 python 을 주입한다), 아니면 값을 있는 그대로 쓴다
/// (이미 `…/Contents/Developer` 이거나 CLT 루트인 정상 케이스). 둘 다 아니면 후보 중 **처음 실재
/// 하는 디렉터리 하나**. 하나도 없으면 None(= CLT·Xcode 둘 다 없는 맥 — 셔임이 설치 창을 띄우는
/// 바로 그 상태). 셸 짝은 `_lib.sh` `cys_clt_tool_present`(같은 순서).
pub fn selected_developer_dir_in(
    developer_dir_env: Option<&OsStr>,
    candidates: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(d) = developer_dir_env.filter(|d| !d.is_empty()) {
        let p = PathBuf::from(d);
        let converted = p.join("Contents").join("Developer");
        if converted.is_dir() {
            return Some(converted);
        }
        if p.is_dir() {
            return Some(p);
        }
    }
    candidates.iter().find(|c| c.is_dir()).cloned()
}

/// `tool` 이 선택된 개발자 디렉터리의 `usr/bin` 에 실행 가능 파일로 있는가(순수 코어 — 테스트 대상).
///
/// ★셔임을 실행해 보지 않는다 — 실행이 곧 설치 창이다. 경로 구분자가 든 이름은 도구 이름이 아니므로
/// 거짓이다(`../` 로 개발자 디렉터리 밖을 보는 판정이 생기지 않게).
pub fn clt_tool_present_in(
    developer_dir_env: Option<&OsStr>,
    candidates: &[PathBuf],
    tool: &str,
) -> bool {
    if tool.is_empty() || tool.contains('/') || tool.contains('\\') {
        return false;
    }
    selected_developer_dir_in(developer_dir_env, candidates)
        .map(|d| is_executable_file(&d.join("usr").join("bin").join(tool)))
        .unwrap_or(false)
}

/// [`clt_tool_present_in`] 을 **이 프로세스의 `DEVELOPER_DIR`** 와 고정 후보 표로 부른다(관측 층).
/// OS 를 묻지 않는다 — OS 게이트는 소비 판정([`clt_absent_bundled_python_for`])의 몫이다.
pub fn clt_tool_present(tool: &str) -> bool {
    let cands: Vec<PathBuf> = DEV_DIR_CANDIDATES.iter().map(PathBuf::from).collect();
    clt_tool_present_in(std::env::var_os(ENV_DEVELOPER_DIR).as_deref(), &cands, tool)
}

/// 동봉 python3 절대경로 — `runtime_bin_dirs`(mac: `Contents/Resources/runtime/python/bin` 등)에서
/// 실행 가능한 `python3` 를 처음 찾은 것. 없으면 None(개발 빌드·손상 번들 — 종전 동작 유지).
pub fn bundled_python3_path(exe_dir: &Path) -> Option<PathBuf> {
    crate::runtime_bin_dirs(exe_dir)
        .into_iter()
        .map(|d| d.join("python3"))
        .find(|p| is_executable_file(p))
}

/// ★U15 **단일 판정**(순수 — OS·롤백·동봉 경로·CLT 관측을 인자로 받는다).
///
/// 다음이 **모두** 참일 때만 동봉 python 경로를 돌려준다:
///   ⓐ `os == "macos"` — 윈도우·리눅스는 셔임이 없다(윈도우 무접촉 계약).
///   ⓑ `!master_off` — 마스터 롤백(`CYS_BOOT_GATES=0`)이면 종전 동작 완전 복귀(새 축은 마스터에 접는다).
///   ⓒ 동봉 python3 실재.
///   ⓓ CLT python3 **부재** — CLT 가 있는 맥(오너 기계 포함)은 한 바이트도 바꾸지 않는다.
/// `clt_python_present` 는 ⓐⓑⓒ 가 참일 때만 호출된다(윈도우·리눅스에서 개발자 디렉터리 stat 0).
pub fn clt_absent_bundled_python_for(
    os: &str,
    master_off: bool,
    bundled: Option<&Path>,
    clt_python_present: impl FnOnce() -> bool,
) -> Option<PathBuf> {
    if os != "macos" || master_off {
        return None;
    }
    let b = bundled?;
    if clt_python_present() {
        return None;
    }
    Some(b.to_path_buf())
}

/// [`clt_absent_bundled_python_for`] 를 **현재 프로세스**(OS·`CYS_BOOT_GATES`·`DEVELOPER_DIR`)로 부른다.
/// 비-macOS 는 디스크를 한 번도 보지 않고 None 이다.
pub fn clt_absent_bundled_python(exe_dir: &Path) -> Option<PathBuf> {
    let os = std::env::consts::OS;
    if os != "macos" {
        return None;
    }
    let master_off =
        crate::boot_gates_master_off_from(std::env::var(crate::ENV_BOOT_GATES).ok().as_deref());
    let bundled = bundled_python3_path(exe_dir);
    clt_absent_bundled_python_for(os, master_off, bundled.as_deref(), || {
        clt_tool_present("python3")
    })
}

/// ⑦ `CYS_PY` + 마커 조건부 쌍 주입의 **순수 코어**(env 도 디스크도 보지 않는다).
///
/// 불가침 계약([`crate::inject_claude_code_git_bash_path_for`] 과 동형):
///   ① `user_has_cys_py` — 사용자 프로세스 env 에 `CYS_PY` 가 있으면 **절대 덮지 않는다**(venv 등 의도적 설정).
///   ② 이미 쌓인 쌍에 같은 키가 있으면 손대지 않는다(later-wins 뒤집기·중복 금지).
///   ③ `bundled_when_clt_absent == None`(= 판정 불충족)이면 아무것도 얹지 않는다 — 종전과 바이트 동일.
/// 두 키는 **함께만** 나간다 — 마커 없는 `CYS_PY` 나 `CYS_PY` 없는 마커는 소비자가 오판한다.
pub fn inject_cys_py_for(
    env_pairs: &mut Vec<(String, String)>,
    bundled_when_clt_absent: Option<&Path>,
    user_has_cys_py: bool,
) {
    if user_has_cys_py {
        return;
    }
    let Some(p) = bundled_when_clt_absent else {
        return;
    };
    if env_pairs
        .iter()
        .any(|(k, _)| k == ENV_CYS_PY || k == ENV_CYS_PY_ORIGIN)
    {
        return;
    }
    env_pairs.push((ENV_CYS_PY.to_string(), p.to_string_lossy().into_owned()));
    env_pairs.push((
        ENV_CYS_PY_ORIGIN.to_string(),
        CYS_PY_ORIGIN_CLT_ABSENT.to_string(),
    ));
}

/// `dir` 을 PATH **맨 앞**에 얹은 값(순수). 기존 성분은 순서 그대로 뒤에 보존한다.
/// join 실패(성분에 구분자 포함 — 동봉 경로에선 비실재)는 `current` 를 그대로 돌려준다(무변경 = 안전측).
pub fn path_with_dir_first(dir: &Path, current: Option<&OsStr>) -> OsString {
    let mut parts: Vec<PathBuf> = vec![dir.to_path_buf()];
    if let Some(cur) = current {
        parts.extend(std::env::split_paths(cur));
    }
    std::env::join_paths(parts)
        .unwrap_or_else(|_| current.map(OsStr::to_os_string).unwrap_or_default())
}

/// `exe` 의 **정규화된** 부모 디렉터리(리뷰1 MAJOR-2 실측 대응).
///
/// ★사실(2026-09-23 리뷰1 rustc 프로브 실측): macOS 의 `std::env::current_exe()` 는 심링크를
/// **풀지 않는다** — 절대경로로 불러도 PATH 탐색으로 불러도 같다(`cys.rs` 옛 주석 "이미 realpath"는
/// 거짓이었다). 이 기계엔 앱이 까는 루트 심링크 `/usr/local/bin/cys → …/cys.app/Contents/MacOS/cys`
/// 가 있고, 좌석 Bash·`session-start.sh` BOOT_CMD·`role-bootstrap-legacy.sh` spawn 폴백이 모두
/// PATH 로 그 심링크를 고른다. 심링크 경로를 그대로 `.parent()` 하면 `/usr/local/bin` 이 나와
/// [`bundled_python3_path`] 가 `None`(번들 레이아웃이 아니므로) → [`clt_absent_child_path`] 가
/// 무력화된다(회수 자식이 여전히 `/usr/bin/python3` 셔임을 부른다). `canonicalize` 로 심링크를
/// 실체까지 푼 뒤 부모를 구하면 `…/Contents/MacOS` 가 나와 번들 탐지가 다시 선다.
///
/// canonicalize 실패(대상이 이미 사라졌거나 권한 문제 — 드묾)는 **원래 경로로 폴백**한다(에러로
/// 죽지 않는다 · 실패측은 종전 동작인 "무접촉"으로 자연히 수렴한다 — `clt_absent_child_path` 가
/// 그 경로에서 번들을 못 찾으면 None 을 돌려줄 뿐이다).
pub fn canonicalized_exe_parent(exe: &Path) -> Option<PathBuf> {
    let resolved = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    resolved.parent().map(Path::to_path_buf)
}

/// `cys boot` 회수 자식(`escalate_reclaim`)의 PATH — CLT 없는 맥에서만 Some(동봉 python 디렉터리 선두).
///
/// ★인터프리터 **후보를 넓히지 않는다**: 회수는 kill 을 포함한 파괴 경로라 Windows 보수 판정(스폰 사이트
/// 1개 · `python3` 단일 · 건강성 H-SAFE-W ⓔ)이 걸려 있다. 그래서 프로그램 이름은 그대로 `python3` 이고,
/// 자식 PATH 선두만 바꿔 **그 이름이 동봉본으로 풀리게** 한다(unix `Command` 는 자식 PATH 로 프로그램을
/// 찾는다 — 아래 테스트 `child_path_governs_program_lookup` 이 실측으로 못박는다). None 이면 호출부가
/// PATH 를 건드리지 않는다(윈도우·리눅스·CLT 있는 맥 = 종전과 동일).
pub fn clt_absent_child_path(exe_dir: &Path, current: Option<&OsStr>) -> Option<OsString> {
    let py = clt_absent_bundled_python(exe_dir)?;
    let dir = py.parent()?;
    Some(path_with_dir_first(dir, current))
}

/// Tauri 기동 안내 문구(순수). 두 도구가 모두 있으면 None(정상 맥은 말이 없어야 한다).
///
/// ★정직 계약: 이번 수리가 닿는 곳(훅·부트 점검·회수·체크리스트·부서 도구)과 닿지 않는 곳(에이전트가
/// 직접 치는 `python3`·`git`, Claude Code 자신의 git 확인)을 **둘 다** 말한다. "전부 정상"이라고 말하면
/// 그 뒤에 뜨는 설치 창이 거짓 안내가 된다. 재설치 채널(bundle-damaged)과 섞지 않는다 — 고장이 아니다.
pub fn devtools_missing_notice(python_present: bool, git_present: bool) -> Option<String> {
    if python_present && git_present {
        return None;
    }
    let mut missing: Vec<&str> = Vec::new();
    if !python_present {
        missing.push("python3");
    }
    if !git_present {
        missing.push("git");
    }
    Some(format!(
        "이 맥에는 '명령어 라인 개발자 도구'(Command Line Tools)가 없습니다 — 없는 도구: {}. \
         이 맥의 /usr/bin/python3·/usr/bin/git 은 '개발자 도구를 설치하라'는 창을 띄우는 껍데기라서, \
         cys 는 훅·부트 점검·자동 복구를 앱에 들어 있는 python 으로 돌립니다. \
         다만 에이전트가 직접 python3·git 명령을 칠 때와 Claude Code 가 git 을 확인할 때는 그 설치 창이 \
         뜰 수 있습니다 — '나중에'를 눌러도 cys 는 계속 동작합니다. \
         git 원격 기능(자기개선 push 등)까지 쓰려면 그 창의 '설치'를 누르거나 터미널에서 \
         xcode-select --install 을 실행하세요.",
        missing.join("·")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// 테스트마다 고유한 임시 루트(pid + 태그) — 병렬 테스트 간섭 방지.
    fn scratch(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("cys-u15-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(&base).expect("임시 루트 생성 실패");
        base
    }

    /// `<dev>/usr/bin/<tool>` 을 실행 가능 파일로 깐다(내용은 무관 — 실행하지 않는다).
    fn plant_tool(dev: &Path, tool: &str) {
        let d = dev.join("usr").join("bin");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(tool);
        std::fs::write(&p, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// ★CLT 판정 행렬 — libxcselect 탐색 순서(DEVELOPER_DIR → link → Xcode → CLT) + "선택은 하나".
    #[test]
    fn clt_judgment_matrix_follows_xcselect_order() {
        let base = scratch("matrix");
        let link = base.join("xcode_select_link");
        let xcode = base.join("Xcode.app").join("Contents").join("Developer");
        let clt = base.join("CommandLineTools");
        let cands = vec![link.clone(), xcode.clone(), clt.clone()];

        // ⓪ 아무것도 없다 = CLT·Xcode 없는 맥(설치 창 상태) → 부재.
        assert!(!clt_tool_present_in(None, &cands, "python3"), "빈 맥을 present 로 판정했다");
        assert_eq!(selected_developer_dir_in(None, &cands), None);

        // ① CLT 만 있고 도구가 있다 → present.
        plant_tool(&clt, "python3");
        assert!(clt_tool_present_in(None, &cands, "python3"), "CLT python3 를 못 봤다");
        assert!(!clt_tool_present_in(None, &cands, "git"), "없는 git 을 present 로 판정했다");

        // ② 앞 후보(Xcode)가 실재하면 **그것이 선택**된다 — 도구가 없으면 뒤 CLT 로 넘어가지 않는다.
        std::fs::create_dir_all(&xcode).unwrap();
        assert_eq!(selected_developer_dir_in(None, &cands), Some(xcode.clone()));
        assert!(
            !clt_tool_present_in(None, &cands, "python3"),
            "선택된 Xcode 에 도구가 없는데 뒤 후보(CLT)로 넘어갔다 — 셔임은 선택된 디렉터리만 본다"
        );
        plant_tool(&xcode, "python3");
        assert!(clt_tool_present_in(None, &cands, "python3"));

        // ③ select link 가 최우선 — 링크 대상에 도구가 없으면 부재(링크 있는데 도구 삭제).
        #[cfg(unix)]
        {
            let target = base.join("link-target");
            std::fs::create_dir_all(&target).unwrap();
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert_eq!(selected_developer_dir_in(None, &cands), Some(link.clone()));
            assert!(
                !clt_tool_present_in(None, &cands, "python3"),
                "선택 링크 대상에 도구가 없는데 present 로 판정했다(링크 있고 도구 삭제)"
            );
            plant_tool(&target, "python3");
            assert!(clt_tool_present_in(None, &cands, "python3"), "링크를 따라가지 못했다");
            std::fs::remove_file(&link).unwrap();
        }

        // ④ DEVELOPER_DIR 이 디렉터리면 최우선 — 후보 표를 보지 않는다.
        let devdir = base.join("custom-dev");
        std::fs::create_dir_all(&devdir).unwrap();
        assert!(
            !clt_tool_present_in(Some(devdir.as_os_str()), &cands, "python3"),
            "DEVELOPER_DIR 선택을 무시하고 후보 표를 봤다"
        );
        plant_tool(&devdir, "python3");
        assert!(clt_tool_present_in(Some(devdir.as_os_str()), &[], "python3"));
        // DEVELOPER_DIR 이 디렉터리가 아니거나 비었으면 후보 표로 내려간다.
        let bogus = base.join("no-such-dir");
        assert_eq!(
            selected_developer_dir_in(Some(bogus.as_os_str()), &cands),
            Some(xcode.clone())
        );
        assert_eq!(
            selected_developer_dir_in(Some(OsStr::new("")), &cands),
            Some(xcode.clone())
        );

        // ④' ★MINOR-4(리뷰1): DEVELOPER_DIR 이 Xcode **앱 번들 루트**(Contents/Developer 자체가
        //    아님)여도, 그 안의 Contents/Developer 가 실재하면 그것으로 변환해 선택한다(man
        //    xcode-select 셔임 변환 규약 — 이 축 없으면 앱 루트를 쓰는 CLT 기계를 부재로 오판한다).
        let app_root = base.join("Xcode-beta.app");
        let app_dev = app_root.join("Contents").join("Developer");
        plant_tool(&app_dev, "python3");
        assert_eq!(
            selected_developer_dir_in(Some(app_root.as_os_str()), &cands),
            Some(app_dev.clone()),
            "Xcode 앱 번들 루트를 Contents/Developer 로 변환하지 않았다"
        );
        assert!(
            clt_tool_present_in(Some(app_root.as_os_str()), &cands, "python3"),
            "앱 번들 루트로 DEVELOPER_DIR 를 준 CLT 기계를 부재로 오판했다"
        );
        // 앱 루트 안에 Contents/Developer 가 없으면(라이선스 미동의 등) 변환하지 않고 값을 그대로
        // 쓴다(기존 계약 유지 — usr/bin 이 없으니 도구는 결국 부재로 판정된다).
        let bare_app_root = base.join("Xcode-bare.app");
        std::fs::create_dir_all(&bare_app_root).unwrap();
        assert_eq!(
            selected_developer_dir_in(Some(bare_app_root.as_os_str()), &cands),
            Some(bare_app_root.clone()),
            "Contents/Developer 가 없는 앱 루트에서 값을 그대로 쓰지 않았다"
        );
        assert!(!clt_tool_present_in(Some(bare_app_root.as_os_str()), &cands, "python3"));

        // ⑤ 실행 비트 없는 파일·이름에 구분자 → 부재.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = xcode.join("usr").join("bin").join("python3");
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(!clt_tool_present_in(None, &cands, "python3"), "실행 불가 파일을 도구로 셌다");
        }
        assert!(!clt_tool_present_in(None, &cands, "../bin/python3"));
        assert!(!clt_tool_present_in(None, &cands, ""));
        std::fs::remove_dir_all(&base).ok();
    }

    /// ★단일 판정 진리표 — 4조건 전부 참일 때만 Some · OS 게이트가 CLT stat 보다 먼저.
    #[test]
    fn clt_absent_decision_truth_table_and_laziness() {
        let py = Path::new("/App/cys.app/Contents/Resources/runtime/python/bin/python3");
        let calls = Cell::new(0u32);
        let probe = |present: bool| {
            let c = &calls;
            move || {
                c.set(c.get() + 1);
                present
            }
        };
        // 유일한 참 조합.
        assert_eq!(
            clt_absent_bundled_python_for("macos", false, Some(py), probe(false)),
            Some(py.to_path_buf())
        );
        // CLT 있는 맥 → None(오너 기계 무변경).
        assert_eq!(clt_absent_bundled_python_for("macos", false, Some(py), probe(true)), None);
        // 동봉 부재 → None(개발 빌드·손상 번들 — 종전 동작).
        assert_eq!(clt_absent_bundled_python_for("macos", false, None, probe(false)), None);
        // 마스터 롤백 → None.
        assert_eq!(clt_absent_bundled_python_for("macos", true, Some(py), probe(false)), None);
        let before = calls.get();
        // 윈도우·리눅스·기타 → None 이고 CLT 관측을 **호출조차 하지 않는다**.
        for os in ["windows", "linux", "freebsd", "ios", ""] {
            assert_eq!(
                clt_absent_bundled_python_for(os, false, Some(py), probe(false)),
                None,
                "OS {os:?} 에서 동봉 python 치환이 켜졌다 — 윈도우 무접촉 계약 위반"
            );
        }
        assert_eq!(calls.get(), before, "비-macOS 에서 CLT 관측(stat)을 실행했다");
        // 롤백·동봉 부재도 관측 전에 접힌다.
        let before = calls.get();
        let _ = clt_absent_bundled_python_for("macos", true, Some(py), probe(false));
        let _ = clt_absent_bundled_python_for("macos", false, None, probe(false));
        assert_eq!(calls.get(), before, "관측이 필요 없는 갈래에서 CLT stat 을 했다");
        // 이 호스트 배선: 비-macOS 호스트면 실제 번들 레이아웃이 있어도 None.
        if std::env::consts::OS != "macos" {
            assert_eq!(clt_absent_bundled_python(Path::new("/nonexistent")), None);
        }
    }

    /// ★⑦ 조건부 쌍 불가침 계약 — 두 키는 함께만 · 사용자 값 불가침 · 기존 쌍 불가침 · 미충족 무주입.
    #[test]
    fn inject_cys_py_pairs_contract() {
        let py = Path::new("/App/cys.app/Contents/Resources/runtime/python/bin/python3");
        let mut on = Vec::new();
        inject_cys_py_for(&mut on, Some(py), false);
        assert_eq!(
            on,
            vec![
                (ENV_CYS_PY.to_string(), py.to_string_lossy().into_owned()),
                (ENV_CYS_PY_ORIGIN.to_string(), CYS_PY_ORIGIN_CLT_ABSENT.to_string()),
            ],
            "CLT 부재 맥에 CYS_PY+마커 두 쌍이 정확히 실리지 않았다"
        );
        assert_eq!(ENV_CYS_PY, "CYS_PY", "훅 프리루드가 읽는 키 이름이 바뀌었다");
        assert_eq!(CYS_PY_ORIGIN_CLT_ABSENT, "bundled-clt-absent", "셸 짝과 정확 일치하는 마커 값");

        let mut user = Vec::new();
        inject_cys_py_for(&mut user, Some(py), true);
        assert!(user.is_empty(), "사용자 CYS_PY 를 덮었다: {user:?}");

        let mut none = Vec::new();
        inject_cys_py_for(&mut none, None, false);
        assert!(none.is_empty(), "판정 불충족인데 쌍을 얹었다(바이트 동일 계약 위반): {none:?}");

        let mut dup = vec![(ENV_CYS_PY.to_string(), "/mine/python3".to_string())];
        inject_cys_py_for(&mut dup, Some(py), false);
        assert_eq!(dup.len(), 1, "같은 키를 중복으로 얹었다: {dup:?}");
        assert_eq!(dup[0].1, "/mine/python3", "먼저 실린 값을 덮었다");
    }

    /// ★생산 배선 핀 — `spawn_env_pairs` 가 ⑦ 을 실제로 부르고, 판정 불충족이면 키 집합이 **종전과 같다**.
    ///
    /// 동봉 python 이 없는 exe_dir 은 어느 호스트에서나 무주입이어야 한다(= 윈도우·리눅스·개발 빌드의
    /// 바이트 동일 핀). 가짜 번들(mac 레이아웃)을 깐 exe_dir 은 호스트 판정을 그대로 따른다 —
    /// 비-mac 호스트·CLT 있는 mac·사용자 CYS_PY·롤백이면 무주입, 그 밖(CLT 없는 mac)이면 두 쌍.
    #[test]
    fn spawn_env_pairs_wires_seventh_pair_only_when_clt_absent() {
        let keys = |pairs: &[(String, String)]| -> Vec<String> {
            pairs
                .iter()
                .map(|(k, _)| k.clone())
                .filter(|k| k == ENV_CYS_PY || k == ENV_CYS_PY_ORIGIN)
                .collect()
        };
        let nobundle =
            crate::spawn_env_pairs(Path::new("/nonexistent-exe-dir-for-pin"), "/usr/bin:/bin", Some("/Users/user"), None);
        assert!(
            keys(&nobundle).is_empty(),
            "동봉 python 이 없는데 CYS_PY 가 실렸다 — 판정 불충족 = 종전과 바이트 동일 계약 위반"
        );

        // 가짜 mac 번들: <base>/Contents/MacOS(exe_dir) + <base>/Contents/Resources/runtime/python/bin/python3
        let base = scratch("wire");
        let exe_dir = base.join("Contents").join("MacOS");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let pybin = base.join("Contents").join("Resources").join("runtime").join("python").join("bin");
        std::fs::create_dir_all(&pybin).unwrap();
        let fake_py = pybin.join("python3");
        std::fs::write(&fake_py, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_py, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let pairs = crate::spawn_env_pairs(&exe_dir, "/usr/bin:/bin", Some("/Users/user"), None);
        let expect_on = std::env::consts::OS == "macos"
            && !clt_tool_present("python3")
            && std::env::var_os(ENV_CYS_PY).is_none()
            && !crate::boot_gates_master_off_from(std::env::var(crate::ENV_BOOT_GATES).ok().as_deref());
        if expect_on {
            let got: Vec<(String, String)> = pairs
                .iter()
                .filter(|(k, _)| k == ENV_CYS_PY || k == ENV_CYS_PY_ORIGIN)
                .cloned()
                .collect();
            assert_eq!(
                got,
                vec![
                    (ENV_CYS_PY.to_string(), fake_py.to_string_lossy().into_owned()),
                    (ENV_CYS_PY_ORIGIN.to_string(), CYS_PY_ORIGIN_CLT_ABSENT.to_string()),
                ],
                "CLT 없는 mac 호스트인데 ⑦ 배선이 동봉 python 을 싣지 않았다"
            );
        } else {
            assert!(
                keys(&pairs).is_empty(),
                "이 호스트(OS={}·CLT python3 있음 또는 사용자 CYS_PY/롤백)에서 ⑦ 이 켜졌다: {:?}",
                std::env::consts::OS,
                keys(&pairs)
            );
        }
        std::fs::remove_dir_all(&base).ok();
    }

    /// ★생산 배선 핀(호스트 무관 · 리뷰1 MAJOR-1 대응) — `spawn_env_pairs_with` 에 가짜 ⑦ 판정을
    /// 주입해, 리눅스·윈도우·CLT 있는 맥·이 개발기를 포함한 **어느 CI 레인에서든** 양성 갈래(두 쌍
    /// 주입)와 음성 갈래(무주입) 를 둘 다 실행·단언한다. 위 `spawn_env_pairs_wires_seventh_pair_…`
    /// 는 실제 호스트 판정을 재는 통합 핀으로 남기고, 이 테스트가 ⑦ 배선 자체의 1차 방어선이다.
    /// ⑦ 호출이 통째로 지워지는 회귀(리뷰1 실측 뮤테이션 R1)는 `expect_on` 분기 없이 이 테스트
    /// 하나로 즉사한다.
    #[test]
    fn spawn_env_pairs_with_wires_seventh_pair_regardless_of_host() {
        let py = Path::new("/App/cys.app/Contents/Resources/runtime/python/bin/python3");
        let exe_dir = Path::new("/irrelevant-for-this-test/Contents/MacOS");

        // 양성: 판정이 Some 이고 사용자 CYS_PY 가 없다 → 두 쌍이 정확히 실린다(호스트 무관).
        let on = crate::spawn_env_pairs_with(exe_dir, "/usr/bin:/bin", Some("/Users/user"), None, Some(py), false);
        let got: Vec<(String, String)> = on
            .iter()
            .filter(|(k, _)| k == ENV_CYS_PY || k == ENV_CYS_PY_ORIGIN)
            .cloned()
            .collect();
        assert_eq!(
            got,
            vec![
                (ENV_CYS_PY.to_string(), py.to_string_lossy().into_owned()),
                (ENV_CYS_PY_ORIGIN.to_string(), CYS_PY_ORIGIN_CLT_ABSENT.to_string()),
            ],
            "판정이 Some 인데 spawn_env_pairs_with 가 ⑦ 두 쌍을 정확히 싣지 않았다 \
             (⑦ 호출이 지워졌거나 값이 갈렸다면 여기서 잡힌다 — 호스트 무관)"
        );

        // 음성 ⓐ: 판정이 None(동봉 python 없음·CLT 있음 등) → ⑦ 키가 0개(바이트 동일 계약).
        let off = crate::spawn_env_pairs_with(exe_dir, "/usr/bin:/bin", Some("/Users/user"), None, None, false);
        assert!(
            !off.iter().any(|(k, _)| k == ENV_CYS_PY || k == ENV_CYS_PY_ORIGIN),
            "판정이 None 인데 ⑦ 키가 실렸다: {off:?}"
        );

        // 음성 ⓑ: 판정은 Some 이지만 사용자가 이미 CYS_PY 를 설정했다 → 덮지 않는다(불가침 계약).
        let user = crate::spawn_env_pairs_with(exe_dir, "/usr/bin:/bin", Some("/Users/user"), None, Some(py), true);
        assert!(
            !user.iter().any(|(k, _)| k == ENV_CYS_PY || k == ENV_CYS_PY_ORIGIN),
            "사용자 CYS_PY 가 있는데 ⑦ 이 덮었다: {user:?}"
        );
    }

    /// ★생산 배선 소스 핀(리뷰1 MAJOR-1 최소 방어선) — `spawn_env_pairs`(공용 스폰 규약의 유일
    /// 생산 진입점)가 실제 호스트 판정을 `spawn_env_pairs_with` 에 넘기고, 그 판정이 이 모듈의
    /// 단일 판정 함수(`clt_absent_bundled_python`)에서 온다. 위 배선 핀들이 죽어도(예: 둘 다 통째로
    /// 지워지는 변이) 이 소스 대조가 독립적으로 잡는다 — `boot_child_path`(부트 감독)·
    /// `clt_absent_child_path`(회수) 소비자와 같은 급의 소스 핀이다.
    #[test]
    fn spawn_env_pairs_wires_seventh_verdict_through_single_source_source_pin() {
        let src = include_str!("lib.rs");
        let prod = src.split("#[cfg(test)]").next().expect("프로덕션 구간 분리 실패");
        let i = prod.find("pub fn spawn_env_pairs(").expect("spawn_env_pairs 소실");
        let body = &prod[i..prod[i..].find("\n}\n").map(|e| i + e).unwrap_or(prod.len())];
        assert!(
            body.contains("spawn_env_pairs_with("),
            "spawn_env_pairs 가 순수 조립부(spawn_env_pairs_with)를 거치지 않는다"
        );
        assert!(
            body.contains("macos_devtools::clt_absent_bundled_python(exe_dir)"),
            "⑦ 판정을 이 모듈의 단일 판정(clt_absent_bundled_python) 밖에서 구한다(판정 사본 = 조건 드리프트)"
        );

        let j = prod.find("pub fn spawn_env_pairs_with(").expect("spawn_env_pairs_with 소실");
        let with_body = &prod[j..prod[j..].find("\n}\n").map(|e| j + e).unwrap_or(prod.len())];
        assert!(
            with_body.contains("macos_devtools::inject_cys_py_for(&mut env, clt_absent_bundled, user_has_cys_py)"),
            "spawn_env_pairs_with 가 ⑦ 주입 코어(inject_cys_py_for)를 인자 그대로 부르지 않는다"
        );
    }

    /// 자식 PATH 합성 — 동봉 디렉터리 선두 + 나머지 순서 보존 · PATH 부재에서도 선두 한 조각.
    #[test]
    fn path_with_dir_first_keeps_rest_in_order() {
        let dir = Path::new("/App/cys.app/Contents/Resources/runtime/python/bin");
        let cur = std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
        let got = path_with_dir_first(dir, Some(cur.as_os_str()));
        let parts: Vec<PathBuf> = std::env::split_paths(&got).collect();
        assert_eq!(
            parts,
            vec![dir.to_path_buf(), PathBuf::from("/usr/bin"), PathBuf::from("/bin")],
            "동봉 디렉터리가 선두가 아니거나 기존 순서가 깨졌다"
        );
        let alone = path_with_dir_first(dir, None);
        assert_eq!(std::env::split_paths(&alone).collect::<Vec<_>>(), vec![dir.to_path_buf()]);
        // 비-macOS 호스트: 회수 자식 PATH 는 절대 바뀌지 않는다(윈도우 파괴 경로 무접촉).
        if std::env::consts::OS != "macos" {
            assert_eq!(clt_absent_child_path(Path::new("/nonexistent"), Some(cur.as_os_str())), None);
        }
        // 동봉 python 이 없는 exe_dir → 어느 호스트에서나 None(= 호출부 PATH 무접촉).
        assert_eq!(
            clt_absent_child_path(Path::new("/nonexistent-exe-dir-for-pin"), Some(cur.as_os_str())),
            None
        );
    }

    /// ★리뷰1 MAJOR-2 회귀 — 가짜 번들 + 심링크. `/usr/local/bin/cys → 번들 안 실체` 와 동형인
    /// 심링크를 만들어 두 가지를 못박는다: ① `canonicalized_exe_parent` 는 어느 unix 호스트에서도
    /// 심링크를 실체 부모까지 푼다(canonicalize 는 OS 중립). ② (macOS 호스트에 한해) 그 정규화된
    /// 부모에서만 [`bundled_python3_path`] 가 동봉 python 을 찾고, **심링크 디렉터리 자체**에서는
    /// 못 찾는다(= 수리 전 버그의 실측 재현 — [`runtime_bin_dirs`](crate::runtime_bin_dirs) 가
    /// macOS 에서만 `Contents/Resources/runtime` 레이아웃을 스캔하므로 다른 호스트에선 애초에
    /// 둘 다 None 이다).
    #[test]
    fn canonicalized_exe_parent_resolves_symlinked_launcher_to_bundled_python() {
        if !cfg!(unix) {
            return; // 심링크·실행비트 의미론은 unix 전용(윈도우 무접촉 계약 — 이 파일의 다른 unix 게이트 테스트와 동형).
        }
        // canonicalize the scratch root itself — on macOS `$TMPDIR` lives under a symlink
        // (`/var` → `/private/var`), so comparing against a non-canonicalized `base` would fail
        // for the wrong reason (a *different* symlink than the one this test means to exercise).
        let base = std::fs::canonicalize(scratch("symlink-major2")).unwrap();

        // 가짜 앱 번들: <base>/Contents/MacOS/cys(실행파일) + …/Resources/runtime/python/bin/python3.
        let macos_dir = base.join("Contents").join("MacOS");
        std::fs::create_dir_all(&macos_dir).unwrap();
        let real_exe = macos_dir.join("cys");
        std::fs::write(&real_exe, b"#!/bin/sh\nexit 0\n").unwrap();
        let pybin = base
            .join("Contents")
            .join("Resources")
            .join("runtime")
            .join("python")
            .join("bin");
        std::fs::create_dir_all(&pybin).unwrap();
        let fake_py = pybin.join("python3");
        std::fs::write(&fake_py, b"#!/bin/sh\nexit 0\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&real_exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::set_permissions(&fake_py, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        // 루트 심링크: <base>/usr-local-bin/cys → 위 실행파일(오너 기계의 /usr/local/bin/cys 와 동형).
        let link_dir = base.join("usr-local-bin");
        std::fs::create_dir_all(&link_dir).unwrap();
        let link = link_dir.join("cys");
        std::os::unix::fs::symlink(&real_exe, &link).unwrap();

        // ① canonicalize 는 어느 unix 호스트에서도 심링크를 실체 부모까지 푼다.
        let resolved = canonicalized_exe_parent(&link).expect("심링크 부모 해소 실패");
        assert_eq!(resolved, macos_dir, "canonicalize 결과가 실제 MacOS 디렉터리가 아니다");

        // canonicalize 실패(대상 소실) → 원래 경로로 폴백(에러로 죽지 않는다).
        let ghost_parent = base.join("nonexistent-parent-for-fallback");
        let ghost = ghost_parent.join("cys");
        assert_eq!(
            canonicalized_exe_parent(&ghost),
            Some(ghost_parent),
            "canonicalize 실패에서 원래 부모로 폴백하지 않았다"
        );

        // ② macOS 호스트에서만: 정규화된 부모는 동봉 python 을 찾고, 심링크 디렉터리 자체는 못 찾는다
        //    (= MAJOR-2 버그의 실측 재현 — 수리 전에는 escalate_reclaim 이 후자를 썼다).
        if cfg!(target_os = "macos") {
            assert_eq!(
                bundled_python3_path(&link_dir),
                None,
                "심링크 디렉터리에서 번들 python 을 찾았다 — 이 검체의 버그 재현 전제가 깨졌다"
            );
            assert_eq!(
                bundled_python3_path(&resolved),
                Some(fake_py.clone()),
                "canonicalize 된 exe_dir 에서 동봉 python 을 찾지 못했다 — MAJOR-2 수리가 무효다"
            );
        } else {
            // 다른 호스트는 runtime_bin_dirs 가 이 레이아웃을 아예 스캔하지 않는다(cfg 게이트) —
            // 둘 다 None 이 정상이고, 이 축의 회귀 방어는 macOS 호스트의 몫이다.
            assert_eq!(bundled_python3_path(&resolved), None);
        }

        std::fs::remove_dir_all(&base).ok();
    }

    /// ★전제 실측 — unix `Command` 는 **자식 env 의 PATH** 로 프로그램을 찾는다(회수 수리의 기전).
    /// 이 전제가 깨지면 `clt_absent_child_path` 를 얹어도 `python3` 가 여전히 셔임으로 풀린다.
    #[test]
    fn child_path_governs_program_lookup() {
        if !cfg!(unix) {
            return; // 윈도우는 이 기전을 쓰지 않는다(회수 PATH 무접촉 — 위 핀).
        }
        let base = scratch("lookup");
        let name = format!("cysu15probe{}", std::process::id());
        let p = base.join(&name);
        std::fs::write(&p, b"#!/bin/sh\necho CHILD-PATH-LOOKUP-OK\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let out = std::process::Command::new(&name)
            .env("PATH", path_with_dir_first(&base, Some(OsStr::new("/usr/bin:/bin"))))
            .output()
            .expect("자식 PATH 로 프로그램을 찾지 못했다 — 회수 수리의 전제가 깨졌다");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "CHILD-PATH-LOOKUP-OK");
        std::fs::remove_dir_all(&base).ok();
    }

    /// 안내 문구 — 두 도구가 있으면 침묵 · 없으면 무엇이 없는지·무엇이 여전히 창을 띄우는지·'나중에'·설치 방법.
    #[test]
    fn devtools_notice_is_silent_when_present_and_honest_when_absent() {
        assert_eq!(devtools_missing_notice(true, true), None, "정상 맥에서 안내가 떴다");
        let both = devtools_missing_notice(false, false).expect("CLT 없는 맥에 안내가 없다");
        for needle in ["명령어 라인 개발자 도구", "python3·git", "나중에", "xcode-select --install",
                       "에이전트가 직접", "Claude Code"] {
            assert!(both.contains(needle), "안내에 {needle:?} 가 없다: {both}");
        }
        assert!(!both.contains("Defender"), "맥 안내에 윈도우 문구가 섞였다");
        assert!(!both.contains("재설치"), "고장이 아닌데 재설치를 권한다(bundle-damaged 채널과 혼동)");
        let git_only = devtools_missing_notice(true, false).unwrap();
        assert!(git_only.contains("없는 도구: git."), "빠진 도구 목록이 틀렸다: {git_only}");
    }

    /// ★셸 짝 파리티 — `_lib.sh` `cys_clt_tool_present` 기본 후보 표가 이 파일의 표와 **같은 순서**다.
    /// (배포 팩에는 Rust 소스가 없으므로 레포 체크아웃에서만 잰다 — 파일 부재는 실패다: 레포 안이다.)
    #[test]
    fn shell_twin_uses_the_same_candidate_table() {
        let lib = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("cysjavis-pack")
            .join("hooks")
            .join("_lib.sh");
        let body = std::fs::read_to_string(&lib).expect("_lib.sh 를 읽지 못했다 — 측정 불능은 통과가 아니다");
        let table = DEV_DIR_CANDIDATES.join(":");
        assert!(
            body.contains(&table),
            "_lib.sh 의 CLT 후보 표가 Rust 정본({table})과 다르다 — 셸·Rust 판정이 갈린다"
        );
        assert!(body.contains("cys_clt_tool_present()"), "셸 판정 함수가 사라졌다");
        // 마커 소비자(cys-dept 헤더)는 이 파일의 값과 **정확 일치**로만 켠다 — 값이 갈리면 조용히 꺼진다.
        let dept_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("cysjavis-pack")
            .join("bin")
            .join("cys-dept");
        let dept = std::fs::read_to_string(&dept_path).expect("cys-dept 를 읽지 못했다");
        let want = format!("\"${{{ENV_CYS_PY_ORIGIN}:-}}\" = \"{CYS_PY_ORIGIN_CLT_ABSENT}\"");
        assert!(
            dept.contains(&want),
            "cys-dept 가 마커를 정확 일치({want})로 비교하지 않는다 — Rust 마커와 셸 소비자가 갈린다"
        );
    }
}
