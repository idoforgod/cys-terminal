//! todo 파일 **스캔 집합**의 단일 정의 — 소비자 C1(Python 보고기)과 C2(Rust 데몬)가
//! *같은 파일 집합*을 보게 만드는 이음매 계약(설계 §14 S18 · W14).
//!
//! **왜 이 모듈이 존재하는가**
//!   C1과 C2는 배제 **정책**은 이미 동일했다(`todo_is_countable` ↔ `classify_files`). 그런데
//!   **스캔 루트**가 달랐다 — C1은 `pack/round` + surface cwd/_round + `CYS_TODO_DIRS`, C2는
//!   surface cwd/_round + `CYS_TODO_DIRS`뿐이었다. 그리고 이 조직의 **정본 todo 위치는
//!   `${CYS_PACK_DIR}/round/`** 다(위임 티켓·`cys todo-path`·보고기가 모두 그곳을 쓴다).
//!   즉 데몬에 배선한 선언 판정·verdict/owner payload·유령 배제가 **정본 todo에는 한 번도
//!   적용되지 않았다**. 정책이 같아도 보는 것이 다르면 두 소비자는 다른 세계를 산다.
//!
//! **왜 lib 계층인가 (`todo_decl`과 같은 이유)**
//!   ① 소비자가 둘 이상이다(cysd 워치독 · 파리티 하네스). 각자 구현하면 즉시 drift다.
//!   ② **크레이트 의존 0**이라 `rustc` 단독 컴파일이 된다 —
//!      `scripts/todo_scan_dump.rs`가 이 파일을 `#[path]`로 물어 단독 빌드되고,
//!      `cysjavis-pack/bin/tests/parity_todo_scan.py`가 Python `javis_report.discover_todo_files`와
//!      **같은 임시 트리**를 양쪽에 먹여 파일 집합을 대조한다. 문서가 아니라 그 기계가 계약이다.
//!
//! **정직한 비대칭 1건(문서화)**: Python 보고기는 `cys status --json`의 `live_cwd`(현재 cd 위치)와
//! spawn-time `cwd`를 **둘 다** 루트로 넣는다. 데몬은 `live_cwd`를 상태로 보관하지 않으므로
//! (요청 시점에 sysinfo로 조회한다) surface 루트는 spawn-time cwd뿐이다 = Python이 상위집합이다.
//! 이 모듈은 **주어진 루트 목록에 대한 파일 집합**과 **공통 루트 3종**(pack/round · cwd/_round ·
//! `CYS_TODO_DIRS`)의 구성 규칙을 계약으로 고정하고, 위 비대칭은 여기 명시로만 남긴다 —
//! 숨기면 그것이 다음 이음매가 된다.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// todo 파일 접미. Python `javis_report`의 glob `*_TODO.md`와 같은 규칙이다.
pub const TODO_SUFFIX: &str = "_TODO.md";

/// `CYS_TODO_DIRS`(PATH류 목록)를 플랫폼 규약대로 분해한다.
///
/// `std::env::split_paths`는 Unix에서 ':' · Windows에서 ';'로 가르며, Windows 드라이브 문자
/// 콜론(`C:\…`)을 구분자로 오인하지 않는다 — ':' 하드코딩이 Windows 절대경로를 `C`와 `\…`로
/// 쪼개 워치를 무력화하던 버그를 차단한다. 빈 항목은 기존 동작과 동일하게 버린다.
pub fn parse_todo_dirs(raw: &str) -> Vec<PathBuf> {
    std::env::split_paths(raw)
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

/// 스캔 루트 목록을 **양 소비자 공통 규칙**으로 조립한다(정렬·중복 제거).
///
///   ① `pack_dir/round` — **정본 위치**. 이것이 빠져 있던 것이 S18이다.
///   ② 각 surface의 `cwd/_round`
///   ③ `CYS_TODO_DIRS`의 각 항목(그 자체가 디렉터리다 — `_round`를 덧붙이지 않는다)
///
/// 존재하지 않는 루트도 그대로 담는다(존재 판정은 `discover`가 한다) — 순수 함수라 테스트가
/// 파일시스템 없이 규칙만 검증할 수 있다.
pub fn scan_roots(
    pack_dir: Option<&Path>,
    surface_cwds: &[String],
    todo_dirs_env: Option<&str>,
) -> Vec<PathBuf> {
    let mut out: BTreeSet<PathBuf> = BTreeSet::new();
    if let Some(p) = pack_dir {
        out.insert(p.join("round"));
    }
    for cwd in surface_cwds {
        if cwd.is_empty() {
            continue;
        }
        out.insert(PathBuf::from(cwd).join("_round"));
    }
    if let Some(raw) = todo_dirs_env {
        out.extend(parse_todo_dirs(raw));
    }
    out.into_iter().collect()
}

/// 루트들에서 `*_TODO.md`를 찾는다 — **비재귀**·정규경로·정렬·중복 제거.
///
/// Python `javis_report.discover_todo_files`와 같은 규칙이다:
///   · 비재귀 glob(하위 디렉터리를 파고들지 않는다 — 백업 트리 흡입 방지)
///   · 일반 파일만(디렉터리·깨진 심링크 제외)
///   · **정규경로(realpath) 기준 중복 제거** — 심링크로 같은 파일이 두 루트에 보여도 1건이다.
///     정규화에 실패하면(권한·경합) 원 경로를 그대로 쓴다(fail-open — 조용히 버리지 않는다).
pub fn discover(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: BTreeSet<PathBuf> = BTreeSet::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(TODO_SUFFIX) {
                continue;
            }
            let path = entry.path();
            // 심링크를 따라간 뒤 일반 파일인지 본다(Python `os.path.isfile`과 동일 의미).
            match std::fs::metadata(&path) {
                Ok(m) if m.is_file() => {}
                _ => continue,
            }
            found.insert(std::fs::canonicalize(&path).unwrap_or(path));
        }
    }
    found.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_todo_dirs_drops_empty_entries() {
        #[cfg(unix)]
        {
            let dirs = parse_todo_dirs("/a/b::/c/d");
            assert_eq!(dirs, vec![PathBuf::from("/a/b"), PathBuf::from("/c/d")]);
        }
        assert!(parse_todo_dirs("").is_empty());
    }

    /// ★S18 회귀 핀 — **정본 위치 `pack/round`가 루트에 반드시 들어간다.**
    /// 빠지면 데몬에 배선한 선언 판정·유령 배제가 정본 todo에 한 번도 적용되지 않는다.
    #[test]
    fn scan_roots_always_includes_pack_round() {
        let roots = scan_roots(
            Some(Path::new("/home/u/.cys/pack")),
            &["/w/proj".to_string()],
            Some("/extra/dir"),
        );
        assert!(
            roots.contains(&PathBuf::from("/home/u/.cys/pack/round")),
            "정본 위치가 스캔 루트에 없다: {roots:?}"
        );
        assert!(roots.contains(&PathBuf::from("/w/proj/_round")));
        assert!(roots.contains(&PathBuf::from("/extra/dir")));
    }

    /// 팩 경로가 없으면(=주입 안 함) 그 루트만 빠지고 나머지는 그대로다 — 순수 규칙.
    #[test]
    fn scan_roots_are_sorted_deduped_and_optional() {
        let roots = scan_roots(
            None,
            &["/w".to_string(), "/w".to_string(), String::new()],
            None,
        );
        assert_eq!(roots, vec![PathBuf::from("/w/_round")]);
    }

    #[test]
    fn discover_finds_only_todo_files_non_recursively() {
        let dir = std::env::temp_dir().join(format!(
            "cys-todo-scan-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let deep = dir.join("nested");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(dir.join("WORKER_TODO.md"), "x").unwrap();
        std::fs::write(dir.join("NOTES.md"), "x").unwrap();
        std::fs::write(deep.join("DEEP_TODO.md"), "x").unwrap();
        std::fs::create_dir_all(dir.join("DIR_TODO.md")).unwrap(); // 디렉터리는 파일이 아니다

        let got = discover(&[dir.clone()]);
        let names: Vec<String> = got
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(names, vec!["WORKER_TODO.md".to_string()]);
        // 같은 파일이 두 루트로 보여도 정규경로 기준 1건이다.
        assert_eq!(discover(&[dir.clone(), dir.clone()]).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
