//! S1 수정2(오너 명령 2026-11-22) — 걷어낸 「피드백 보내기」가 사용자 홈에 남긴 폴더의 정리.
//!
//! 오너 명령: 「피드백 보내기」를 통째로 걷어내고, 업데이트되면 `~/.cys/feedback` 을 지운다 —
//! **그 폴더 하나만 · 정확한 경로로만 · 패턴으로 지우지 않는다 · 무엇을 지웠는지 남긴다.**
//!
//! 그 폴더는 그 기능만 썼다(묶음 `fb-YYYYMMDD-HHMMSS-xxxx/` — 설명·진단·첨부 사본). 기능이 사라졌으므로 남은 묶음은
//! 다시 쓰일 일이 없고, 사용자의 화면 캡처·로그 사본을 담고 있을 수 있어 남겨 둘 이유가 없다.
//!
//! 이 파일은 그 기능의 이름을 아는 **유일한** 앱 소스다(ui/src/nofeedback.test.ts 가 그 밖의 줄을 0 으로 잰다).
//! 앱 프로덕션 코드의 재귀 삭제도 이 파일의 한 곳뿐이다(아래 배선 핀).
//!
//! 【부르는 곳】 `main.rs` setup 부트 태스크 — 안전 모드 반환(비정규 경로 기동) **뒤**, 업데이트 반영 **앞**. 전 OS.
//! 【멱등】 폴더가 없으면 stat 한 번으로 끝난다(마커 불요 · 옛 판 앱이 나중에 다시 만든 폴더도 다음 기동이 치운다).
//! 【기록】 지웠으면 표준 오류 한 줄 + `~/.cys/state/feedback-removed-<버전>.log` 에 한 줄을 **덧붙인다**.
//! 【실패】 부트를 막지 않는다. 남은 것은 완전 초기화의 목록(`src/factory_reset.rs` 의 `feedback` 항목)이 여전히 덮는다.

/// 걷어낸 기능이 쓰던 폴더의 이름 — `~/.cys/` 바로 아래의 **이 이름 하나**(접두·패턴 매칭 0).
const RETIRED_FEEDBACK_DIR: &str = "feedback";

/// 잔여 폴더 정리의 결과(순수 값 — 기록과 로그의 재료).
#[derive(Debug, Clone, PartialEq, Eq)]
enum RetiredFeedbackCleanup {
    /// 폴더가 없다(대부분의 설치) — 아무것도 하지 않았다.
    Absent,
    /// 지웠다. `entries` = 직계 항목 이름(정렬) · `files`/`bytes` = 그 안의 일반 파일 수·크기 합.
    /// `link` 가 참이면 그 이름은 심볼릭 링크였고 **링크만** 지웠다(가리키던 곳은 건드리지 않았다).
    Removed {
        entries: Vec<String>,
        files: u64,
        bytes: u64,
        link: bool,
    },
    /// 손대지 않았다 — 이 기능이 만든 모양이 아니다(같은 이름의 일반 파일 · 기준 경로가 `~/.cys` 꼴이 아님).
    Skipped(String),
    /// 지우지 못했다(권한 등). 부트는 계속하고 다음 기동이 다시 시도한다.
    Failed(String),
}

/// 폴더 하나의 일반 파일 수·크기 합. 심볼릭 링크는 따라가지도 세지도 않는다. 읽기 실패는 0 으로 접는다(기록용 수치다).
fn dir_file_totals(dir: &std::path::Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            let Ok(ft) = ent.file_type() else {
                continue;
            };
            if ft.is_dir() {
                stack.push(ent.path());
            } else if ft.is_file() {
                files += 1;
                bytes += ent.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (files, bytes)
}

/// `cys_base`(= `~/.cys`) 바로 아래의 `feedback` **하나**를 지운다(테스트 가능한 심장 — 기준 경로를 주입받는다).
///
/// 지키는 것:
///   ① 대상은 `cys_base.join("feedback")` 한 경로다. 형제(`feedback2`·`feedback.bak`·`state/feedback` …)는 보지도 않는다.
///   ② `cys_base` 가 절대경로이고 마지막 성분이 `.cys` 일 때만 움직인다 — 홈을 못 구해 빈 경로가 오면 상대경로
///      `.cys/feedback`(현재 디렉터리 기준)을 지우게 되는 길을 막는다.
///   ③ 그 이름이 심볼릭 링크면 **링크만** 지운다(따라가지 않는다). 일반 파일이면 손대지 않는다(이 기능은 폴더만 만들었다).
///   ④ 폴더 안의 심볼릭 링크는 링크로 지워진다(표준 라이브러리의 재귀 삭제는 링크를 따라가지 않는다).
fn remove_retired_feedback_dir_in(cys_base: &std::path::Path) -> RetiredFeedbackCleanup {
    use RetiredFeedbackCleanup::{Absent, Failed, Removed, Skipped};
    if !cys_base.is_absolute() || cys_base.file_name() != Some(std::ffi::OsStr::new(".cys")) {
        return Skipped(format!("기준 경로가 ~/.cys 꼴이 아니다: {}", cys_base.display()));
    }
    let target = cys_base.join(RETIRED_FEEDBACK_DIR);
    let meta = match std::fs::symlink_metadata(&target) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Absent,
        Err(e) => return Failed(format!("상태 확인 실패: {e}")),
    };
    let ft = meta.file_type();
    if ft.is_symlink() {
        // 링크 자체만 지운다. 윈도우의 디렉터리 링크는 파일 삭제가 거부하므로 빈 디렉터리 삭제로 한 번 더 —
        // 둘 다 링크만 지우고 가리키는 곳으로 들어가지 않는다.
        return match std::fs::remove_file(&target).or_else(|_| std::fs::remove_dir(&target)) {
            Ok(()) => Removed {
                entries: Vec::new(),
                files: 0,
                bytes: 0,
                link: true,
            },
            Err(e) => Failed(format!("링크 삭제 실패: {e}")),
        };
    }
    if !ft.is_dir() {
        return Skipped("같은 이름의 일반 파일이다 — 이 기능이 만든 것이 아니다".to_string());
    }
    let mut entries: Vec<String> = std::fs::read_dir(&target)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    let (files, bytes) = dir_file_totals(&target);
    match std::fs::remove_dir_all(&target) {
        Ok(()) => Removed {
            entries,
            files,
            bytes,
            link: false,
        },
        Err(e) => Failed(format!("삭제 실패: {e}")),
    }
}

/// 기록 한 줄(순수) — 탭 구분: unix 초 · 앱 버전 · removed · 경로 · link · 직계 항목 수 · 파일 수 · 바이트 · 직계 항목 이름.
/// 지운 것이 없으면 None(없음·건너뜀·실패는 기록 파일을 만들지 않는다).
fn retired_feedback_record_line(
    now_unix: u64,
    target: &std::path::Path,
    out: &RetiredFeedbackCleanup,
) -> Option<String> {
    match out {
        RetiredFeedbackCleanup::Removed {
            entries,
            files,
            bytes,
            link,
        } => Some(format!(
            "{now_unix}\t{}\tremoved\t{}\tlink={link}\tentries={}\tfiles={files}\tbytes={bytes}\t{}\n",
            env!("CARGO_PKG_VERSION"),
            target.display(),
            entries.len(),
            entries.join(",")
        )),
        _ => None,
    }
}

/// setup 부트 태스크에서 부른다(전 OS). 판정은 stat 한 번이다 — 폴더가 없으면 아무 일도 없다(마커 불요 · 멱등 ·
/// 옛 판 앱이 나중에 다시 만든 폴더도 다음 기동이 치운다).
/// 지웠으면 ① 표준 오류에 한 줄 ② `~/.cys/state/feedback-removed-<버전>.log` 에 **덧붙여** 남긴다(무엇을·얼마나).
/// 실패는 부트를 막지 않는다 — 다음 기동이 다시 본다. 남은 것은 완전 초기화의 목록(factory_reset `feedback`)이 여전히 덮는다.
/// ★로그 문구는 기능의 이름을 부르지 않는다(경로가 무엇인지 말한다) — 제품의 말에서 그 이름은 팩 검체
/// (test_content_pins_parity.py 의 A-1-05)가 0 으로 잰다.
pub(crate) fn remove_retired_feedback_dir() {
    let base = cys::home_dir().join(".cys");
    let target = base.join(RETIRED_FEEDBACK_DIR);
    let out = remove_retired_feedback_dir_in(&base);
    match &out {
        RetiredFeedbackCleanup::Absent => {}
        RetiredFeedbackCleanup::Removed {
            entries,
            files,
            bytes,
            link,
        } => {
            eprintln!(
                "[cys-app] 걷어낸 기능이 남긴 폴더를 지웠습니다: {} (링크={link} · 직계 항목 {}개 · 파일 {files}개 · {bytes}바이트)",
                target.display(),
                entries.len()
            );
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(line) = retired_feedback_record_line(now, &target, &out) {
                use std::io::Write as _;
                let dir = base.join("state");
                let _ = std::fs::create_dir_all(&dir);
                let log = dir.join(format!("feedback-removed-{}.log", env!("CARGO_PKG_VERSION")));
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
                    let _ = f.write_all(line.as_bytes());
                }
            }
        }
        RetiredFeedbackCleanup::Skipped(why) | RetiredFeedbackCleanup::Failed(why) => {
            eprintln!(
                "[cys-app] 걷어낸 기능이 남긴 폴더를 정리하지 않았습니다: {} — {why}",
                target.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★배선 소스 핀 — 잔여 폴더 정리는 setup 부트 태스크에서, 안전 모드 반환(비정규 경로 기동) **뒤** · 업데이트 반영
    /// **앞**에 불린다. 앱의 재귀 삭제는 이 모듈의 한 곳뿐이고, 대상은 상수 이름 하나로 만든 경로다(패턴·접두 삭제가
    /// 끼어들면 붉다).
    #[test]
    fn retired_feedback_cleanup_is_wired_before_the_pending_update() {
        let main = include_str!("main.rs");
        let prod = &main[..main.find("#[cfg(test)]\nmod tests {").expect("main.rs 테스트 모듈 경계 소실")];
        let call = prod
            .find("retired_feedback::remove_retired_feedback_dir();")
            .expect("부트 태스크가 '피드백 보내기' 잔여 폴더 정리를 부르지 않는다 — 업데이트 뒤에도 ~/.cys/feedback 이 남는다");
        let upd = prod.find("maybe_apply_pending_update(&handle);").expect("업데이트 반영 호출 소실");
        let safe = prod.find("if verdict != BootPathVerdict::Canonical {").expect("안전 모드 반환 소실");
        assert!(safe < call, "정리가 안전 모드 반환보다 앞이다 — 임시 사본으로 뜬 앱이 사용자 폴더를 지운다");
        assert!(call < upd, "정리가 업데이트 반영 뒤다");
        assert_eq!(prod.matches("\nmod retired_feedback;\n").count(), 1, "모듈 선언이 하나가 아니다");
        assert_eq!(prod.matches("remove_dir_all(").count(), 0, "main.rs 프로덕션 코드에 재귀 삭제가 생겼다");
        let me = include_str!("retired_feedback.rs");
        let mine = &me[..me.find("#[cfg(test)]\nmod tests {").expect("이 모듈의 테스트 경계 소실")];
        assert_eq!(mine.matches("remove_dir_all(").count(), 1, "이 모듈의 재귀 삭제가 한 곳이 아니다");
        assert!(mine.contains("const RETIRED_FEEDBACK_DIR: &str = \"feedback\";"), "대상 이름 상수 소실");
        assert!(
            mine.contains("let target = cys_base.join(RETIRED_FEEDBACK_DIR);"),
            "삭제 대상이 `<~/.cys>/feedback` 한 경로로 만들어지지 않는다"
        );
    }

    // ───────── S1 수정2 — 잔여 폴더 정리의 동작(임시 홈) ─────────
    fn s1_fix2_home(tag: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("cys-s1-fix2-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }
    fn s1_fix2_put(p: &std::path::Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// ★`~/.cys/feedback` 은 통째로 사라지고, **그 경로 하나만** 사라진다 — 이름이 비슷한 형제·같은 이름의 다른 자리는
    /// 한 바이트도 바뀌지 않는다(패턴 삭제였다면 여기서 붉다). 결과는 무엇을 지웠는지 말하고, 두 번째 실행은 할 일이 없다.
    #[test]
    fn retired_feedback_dir_is_removed_and_only_that_path() {
        let home = s1_fix2_home("exact");
        let base = home.join(".cys");
        s1_fix2_put(&base.join("feedback/fb-20261122-101500-ab12/description.txt"), "설명 1234");
        s1_fix2_put(&base.join("feedback/fb-20261122-101500-ab12/attach/shot.png"), "PNGDATA");
        s1_fix2_put(&base.join("feedback/fb-20261121-090000-cd34/diag.json"), "{}");
        let keep = [
            base.join("feedback2/x.txt"),
            base.join("feedback.bak/x.txt"),
            base.join("feedbacks/x.txt"),
            base.join("my-feedback/x.txt"),
            base.join("state/feedback/x.txt"),
            base.join("pack/feedback/x.txt"),
            base.join("feedback.txt"),
            home.join("feedback/x.txt"),
            home.join(".cys-feedback/x.txt"),
        ];
        for k in &keep {
            s1_fix2_put(k, "KEEP");
        }
        let out = remove_retired_feedback_dir_in(&base);
        assert_eq!(
            out,
            RetiredFeedbackCleanup::Removed {
                entries: vec!["fb-20261121-090000-cd34".to_string(), "fb-20261122-101500-ab12".to_string()],
                files: 3,
                bytes: ("설명 1234".len() + "PNGDATA".len() + "{}".len()) as u64,
                link: false,
            }
        );
        assert!(std::fs::symlink_metadata(base.join("feedback")).is_err(), "~/.cys/feedback 이 남아 있다");
        for k in &keep {
            assert_eq!(
                std::fs::read_to_string(k).ok().as_deref(),
                Some("KEEP"),
                "지우면 안 되는 경로가 바뀌었다: {}",
                k.display()
            );
        }
        assert_eq!(remove_retired_feedback_dir_in(&base), RetiredFeedbackCleanup::Absent, "두 번째 실행이 무언가를 했다");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ★이 기능이 만든 모양이 아니면 손대지 않는다 — 폴더 없음 · 같은 이름의 일반 파일 · `.cys` 가 아닌 기준 경로 ·
    /// 상대 경로(홈을 못 구한 경우: 현재 디렉터리 기준으로 지우지 않는다).
    #[test]
    fn retired_feedback_cleanup_refuses_shapes_the_feature_never_made() {
        let home = s1_fix2_home("shapes");
        let base = home.join(".cys");
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(remove_retired_feedback_dir_in(&base), RetiredFeedbackCleanup::Absent);
        s1_fix2_put(&base.join("feedback"), "A FILE");
        assert!(matches!(remove_retired_feedback_dir_in(&base), RetiredFeedbackCleanup::Skipped(_)));
        assert_eq!(std::fs::read_to_string(base.join("feedback")).ok().as_deref(), Some("A FILE"));
        let other = home.join("not-cys");
        s1_fix2_put(&other.join("feedback/x.txt"), "KEEP");
        assert!(matches!(remove_retired_feedback_dir_in(&other), RetiredFeedbackCleanup::Skipped(_)));
        assert_eq!(std::fs::read_to_string(other.join("feedback/x.txt")).ok().as_deref(), Some("KEEP"));
        for rel in [".cys", "", "relative/.cys"] {
            assert!(
                matches!(remove_retired_feedback_dir_in(std::path::Path::new(rel)), RetiredFeedbackCleanup::Skipped(_)),
                "상대 기준 경로 {rel:?} 로 움직였다"
            );
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ★심볼릭 링크를 따라가지 않는다 — `~/.cys/feedback` 자체가 링크면 링크만, 폴더 안의 링크도 링크만 지운다.
    /// 링크가 가리키던 사용자 폴더는 그대로다.
    #[cfg(unix)]
    #[test]
    fn retired_feedback_cleanup_never_follows_a_symlink() {
        let home = s1_fix2_home("link");
        let base = home.join(".cys");
        std::fs::create_dir_all(&base).unwrap();
        let outside = home.join("documents");
        s1_fix2_put(&outside.join("thesis.txt"), "KEEP");
        std::os::unix::fs::symlink(&outside, base.join("feedback")).unwrap();
        assert_eq!(
            remove_retired_feedback_dir_in(&base),
            RetiredFeedbackCleanup::Removed {
                entries: Vec::new(),
                files: 0,
                bytes: 0,
                link: true
            }
        );
        assert!(std::fs::symlink_metadata(base.join("feedback")).is_err(), "링크가 남아 있다");
        assert_eq!(std::fs::read_to_string(outside.join("thesis.txt")).ok().as_deref(), Some("KEEP"));
        s1_fix2_put(&base.join("feedback/fb-1/description.txt"), "x");
        std::os::unix::fs::symlink(&outside, base.join("feedback/fb-1/link-to-documents")).unwrap();
        let out = remove_retired_feedback_dir_in(&base);
        assert!(
            matches!(out, RetiredFeedbackCleanup::Removed { files: 1, bytes: 1, link: false, .. }),
            "폴더 안의 링크를 파일로 세었거나 따라갔다: {out:?}"
        );
        assert!(std::fs::symlink_metadata(base.join("feedback")).is_err());
        assert_eq!(
            std::fs::read_to_string(outside.join("thesis.txt")).ok().as_deref(),
            Some("KEEP"),
            "폴더 안의 링크를 따라가 사용자 폴더를 지웠다"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 기록 한 줄 — 지운 것(경로·수치·직계 항목 이름)을 말하고, 지운 것이 없으면 줄이 없다.
    #[test]
    fn retired_feedback_record_line_names_what_was_removed() {
        let t = std::path::Path::new("/h/.cys/feedback");
        let removed = RetiredFeedbackCleanup::Removed {
            entries: vec!["fb-a".to_string(), "fb-b".to_string()],
            files: 5,
            bytes: 1234,
            link: false,
        };
        assert_eq!(
            retired_feedback_record_line(1795000000, t, &removed),
            Some(format!(
                "1795000000\t{}\tremoved\t/h/.cys/feedback\tlink=false\tentries=2\tfiles=5\tbytes=1234\tfb-a,fb-b\n",
                env!("CARGO_PKG_VERSION")
            ))
        );
        for quiet in [
            RetiredFeedbackCleanup::Absent,
            RetiredFeedbackCleanup::Skipped("x".to_string()),
            RetiredFeedbackCleanup::Failed("y".to_string()),
        ] {
            assert_eq!(retired_feedback_record_line(1, t, &quiet), None, "{quiet:?} 에 기록 줄이 생겼다");
        }
    }
}
