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
//! 【한 번만】 지우는 일은 **한 번**이다(RULING-04 M5 — 오너의 명령은 「업데이트되면 지운다」다). 표식은 기록 **파일**
//!         `~/.cys/state/feedback-removed.log`(판 번호 없는 이름) 자체다(FIX 2 · RULING-S1-PRODUCT-07 B): 그 파일이 있으면 —
//!         완결된 줄이든 끊긴 줄이든 비었든 — 다시 지우지 않는다. 그 뒤에 그 자리에 생긴 폴더(옛 판 앱이 다시 만든 것 포함)는 남는다.
//!         폴더가 없으면 stat 한 번으로 끝난다(기록 파일도 만들지 않는다).
//! 【기록】 **기록이 먼저다**: 지울 것이 보이면 기록 파일을 로그 자신으로 **새로 만들고**, 시도 줄(`attempt`)을 써서 디스크까지
//!         내린 **뒤에** 지운다. 만들 수 없으면 지우지 않는다(다음 기동이 다시 해 본다 — 다시 해 보는 것은 이 경우뿐이다).
//!         만들었으나 시도 줄을 쓰지 못하면 지우지 않고, 그 폴더는 남는다(앱이 나중에 지우지 않는다 — 한 줄로 알린다).
//!         지운 뒤 그 파일에 끝 줄(`removed` · 도중에 멈췄으면 `partial`)을 **덧붙이고** 표준 오류에 한 줄을 낸다.
//! 【실패】 부트를 막지 않는다. 남은 것은 완전 초기화의 목록(`src/factory_reset.rs` 의 `feedback` 항목)이 여전히 덮는다.

/// 걷어낸 기능이 쓰던 폴더의 이름 — `~/.cys/` 바로 아래의 **이 이름 하나**(접두·패턴 매칭 0).
const RETIRED_FEEDBACK_DIR: &str = "feedback";

/// 지운 것의 기록 파일 이름 — `~/.cys/state/` 바로 아래. 판 번호를 넣지 않는다: 이 기록이 「이미 한 번 지웠다」의 표식이고,
/// 판이 바뀌어도 같은 표식을 보아야 한다.
const RETIRED_FEEDBACK_RECORD: &str = "feedback-removed.log";

/// 기록에서 읽는 양의 상한 — 기록은 몇 줄이다. 이보다 크면 「확인할 수 없다」로 본다(지우지 않는다).
const RETIRED_FEEDBACK_RECORD_READ_MAX: u64 = 1024 * 1024;

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
    /// 일부만 지웠다 — 재귀 삭제가 도중에 멈췄다(권한·사용 중인 파일). `gone` = 사라진 직계 항목 이름(정렬) ·
    /// `files`/`bytes` = 사라진 일반 파일 수·크기(지우기 전 − 남은 것) · `left` = 남은 직계 항목 이름 · `why` = 멈춘 사유.
    /// 사라진 것이 기록되므로 다음 기동은 다시 지우지 않는다(한 번만) — 남은 것은 완전 초기화의 목록이 덮는다.
    Partial {
        gone: Vec<String>,
        files: u64,
        bytes: u64,
        left: Vec<String>,
        why: String,
    },
    /// 손대지 않았다 — 이 기능이 만든 모양이 아니다(같은 이름의 일반 파일 · 기준 경로가 `~/.cys` 꼴이 아님).
    Skipped(String),
    /// 지우지 못했다(권한 등) — 사라진 것이 하나도 없다. 부트는 계속하고 다음 기동이 다시 시도한다.
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

/// 폴더 하나의 직계 항목 이름(정렬). 읽기 실패는 빈 목록으로 접는다(기록용이다).
fn dir_entry_names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// `cys_base`(= `~/.cys`) 바로 아래의 `feedback` **하나**를 지운다(테스트 가능한 심장 — 기준 경로를 주입받는다).
///
/// 지키는 것:
///   ① 대상은 `cys_base.join("feedback")` 한 경로다. 형제(`feedback2`·`feedback.bak`·`state/feedback` …)는 보지도 않는다.
///   ② `cys_base` 가 절대경로이고 마지막 성분이 `.cys` 일 때만 움직인다 — 홈을 못 구해 빈 경로가 오면 상대경로
///      `.cys/feedback`(현재 디렉터리 기준)을 지우게 되는 길을 막는다.
///   ③ 그 이름이 심볼릭 링크면 **링크만** 지운다(따라가지 않는다). 일반 파일이면 손대지 않는다(이 기능은 폴더만 만들었다).
///   ④ 폴더 안의 심볼릭 링크는 링크로 지워진다(표준 라이브러리의 재귀 삭제는 링크를 따라가지 않는다).
///   ⑤ 재귀 삭제가 도중에 멈추면 「지우지 않았다」고 말하지 않는다 — 다시 세어 사라진 것을 `Partial` 로 돌려준다.
fn remove_retired_feedback_dir_in(cys_base: &std::path::Path) -> RetiredFeedbackCleanup {
    use RetiredFeedbackCleanup::{Absent, Failed, Partial, Removed, Skipped};
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
    let entries = dir_entry_names(&target);
    let (files, bytes) = dir_file_totals(&target);
    match std::fs::remove_dir_all(&target) {
        Ok(()) => Removed {
            entries,
            files,
            bytes,
            link: false,
        },
        Err(e) => {
            // 재귀 삭제는 첫 오류에서 멈춘다 — 그 앞의 항목은 이미 사라졌다. 다시 세어 사라진 것을 말한다.
            let left = dir_entry_names(&target);
            let (files_left, bytes_left) = dir_file_totals(&target);
            let gone: Vec<String> = entries.iter().filter(|n| !left.contains(n)).cloned().collect();
            let files_gone = files.saturating_sub(files_left);
            if gone.is_empty() && files_gone == 0 {
                Failed(format!("삭제 실패: {e}"))
            } else {
                Partial {
                    gone,
                    files: files_gone,
                    bytes: bytes.saturating_sub(bytes_left),
                    left,
                    why: format!("삭제 실패: {e}"),
                }
            }
        }
    }
}

/// 기록 줄에 넣는 이름 — 제어 문자(줄바꿈·탭)를 `?` 로 바꾼다. 폴더 이름이 줄바꿈을 품고 있어도 기록은 **한 줄**이다.
fn retired_feedback_record_name(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { '?' } else { c }).collect()
}

/// 기록 줄의 이름 칸 — 쉼표로 잇는다.
fn retired_feedback_record_names(v: &[String]) -> String {
    v.iter()
        .map(|n| retired_feedback_record_name(n))
        .collect::<Vec<_>>()
        .join(",")
}

/// 기록 한 줄(순수) — 탭 구분: unix 초 · 앱 버전 · removed|partial · 경로 · link · 사라진 직계 항목 수 · 파일 수 · 바이트 ·
/// 사라진 직계 항목 이름. `partial` 은 그 뒤에 `left=<남은 직계 항목 이름>` 과 멈춘 사유가 붙는다.
/// 사라진 것이 없으면 None(없음·건너뜀·실패는 기록 파일을 만들지 않는다).
fn retired_feedback_record_line(
    now_unix: u64,
    target: &std::path::Path,
    out: &RetiredFeedbackCleanup,
) -> Option<String> {
    let names = retired_feedback_record_names;
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
            names(entries)
        )),
        RetiredFeedbackCleanup::Partial {
            gone,
            files,
            bytes,
            left,
            why,
        } => Some(format!(
            "{now_unix}\t{}\tpartial\t{}\tlink=false\tentries={}\tfiles={files}\tbytes={bytes}\t{}\tleft={}\t{}\n",
            env!("CARGO_PKG_VERSION"),
            target.display(),
            gone.len(),
            names(gone),
            names(left),
            retired_feedback_record_name(why)
        )),
        _ => None,
    }
}

/// 표준 오류 한 줄. `eprintln!` 은 표준 오류가 닫힌 파이프이면 패닉한다 — 부트 태스크를 풀어 버리지 않게 쓰기 결과를 버린다.
fn retired_feedback_note(msg: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{msg}");
}

/// setup 부트 태스크에서 부른다(전 OS). 폴더가 없으면 stat 한 번으로 끝난다. 지우는 일은 한 번뿐이다 — 기록에 「지웠다」 줄이
/// 있으면 그 자리에 다시 생긴 폴더는 건드리지 않는다. 실패는 부트를 막지 않는다 — 하나도 지우지 못했으면(기록 줄 없음) 다음
/// 기동이 다시 해 본다. 남은 것은 완전 초기화의 목록(factory_reset `feedback`)이 여전히 덮는다.
pub(crate) fn remove_retired_feedback_dir() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = remove_retired_feedback_dir_at(&cys::home_dir(), now);
}

/// 홈을 주입받는 몸통(테스트가 임시 홈으로 부른다) — `<home>/.cys/feedback` 을 정리하고 결과를 돌려준다.
/// 사라진 것이 있으면 ① `<home>/.cys/state/feedback-removed.log`(판 번호 없는 이름 — 한 번 지웠다는 표식) 에 **덧붙여** 남기고(무엇을·얼마나)
/// ② 표준 오류에 한 줄을 낸다. 기록이 먼저다 — 표준 오류가 닫혀 있어도 기록은 남는다.
/// ★로그 문구는 기능의 이름을 부르지 않는다(경로가 무엇인지 말한다) — 제품의 말에서 그 이름은 팩 검체
/// (test_content_pins_parity.py 의 A-1-05)가 0 으로 잰다.
fn remove_retired_feedback_dir_at(home: &std::path::Path, now_unix: u64) -> RetiredFeedbackCleanup {
    remove_retired_feedback_dir_noting(home, now_unix, &mut |msg: &str| retired_feedback_note(msg))
}

/// 윈도우 `CreateFileW` 의 `FILE_FLAG_OPEN_REPARSE_POINT` — 마지막 성분이 링크(재분석 지점)면 가리키는 곳이 아니라 링크 자신을 연다.
#[cfg(windows)]
const RETIRED_FEEDBACK_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// 윈도우 파일 속성 `FILE_ATTRIBUTE_REPARSE_POINT`.
#[cfg(windows)]
const RETIRED_FEEDBACK_ATTR_REPARSE_POINT: u32 = 0x0000_0400;

/// 기록 파일을 **로그 자신**으로만 연다(읽기 + 덧붙이기 · 없으면 만든다). 열 수 없거나 로그 자신이 아니면 사유를 돌려준다.
///
/// 지키는 것(리뷰 M1 — 기록 자리에 놓인 링크 너머의 남의 파일에 기록 줄이 쓰였다):
///   ① 마지막 성분의 링크를 **따라가지 않는다**(유닉스 `O_NOFOLLOW` · 윈도우 재분석 지점 그대로 열기) — 파일로 가는 링크 ·
///      끊긴 링크 · 폴더로 가는 링크 어느 것도 그 너머를 열거나 만들지 않는다.
///   ② 연 것이 **일반 파일**인지를 연 핸들로 확인한다(경로를 다시 보지 않는다 — 확인과 쓰기 사이에 바뀔 틈이 없다).
///      파이프·장치·폴더·(윈도우) 재분석 지점은 거부한다.
///   ③ 유닉스: 이름이 둘 이상인 파일(하드 링크)은 거부한다 — 다른 이름으로 불리는 남의 파일일 수 있다.
///   ④ 읽기 + 덧붙이기로 연다. 읽기는 「이미 지웠는가」 표식을 **같은 핸들**로 읽기 위해서다(경로를 두 번 열지 않는다).
///      그리고 읽기+쓰기로 여는 파이프는 읽는 쪽을 기다리지 않는다(쓰기 전용 열기는 끝없이 기다린다 — 부트가 거기서 선다).
///      열린 파이프는 ②가 거부한다.
/// 부모 폴더(`~/.cys` · `~/.cys/state`)가 링크인 것은 따라간다 — cys 의 다른 상태 파일이 놓이는 바로 그 자리이고,
/// 삭제 대상의 부모를 다루는 방식과 같다.
/// `create_new` (FIX 2 · RULING-S1-PRODUCT-07 B): 참이면 **새로 만들 때만** 연다(이미 있으면 실패) — 이 파일을 만든 기동만이
/// 지울 수 있다. 거짓이면 이미 있는 기록을 읽으려고 연다(만들지 않는다).
fn retired_feedback_open_record(log: &std::path::Path, create_new: bool) -> Result<std::fs::File, String> {
    if let Some(dir) = log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true).append(true).create_new(create_new);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        opts.custom_flags(RETIRED_FEEDBACK_OPEN_REPARSE_POINT);
    }
    let f = opts
        .open(log)
        .map_err(|e| format!("열지 못했다(링크는 따라가지 않는다): {e}"))?;
    let meta = f.metadata().map_err(|e| format!("연 것을 확인하지 못했다: {e}"))?;
    if !meta.file_type().is_file() {
        return Err("일반 파일이 아니다".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if meta.nlink() != 1 {
            return Err(format!("이름이 {}개인 파일이다(하드 링크)", meta.nlink()));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        if meta.file_attributes() & RETIRED_FEEDBACK_ATTR_REPARSE_POINT != 0 {
            return Err("링크(재분석 지점)다".to_string());
        }
    }
    Ok(f)
}

/// 기록에 「지웠다」 줄(셋째 칸이 `removed` 또는 `partial`)이 이미 있는가 — **연 핸들**에서 읽는다. 어느 판이 적었는지는 보지
/// 않는다(한 번 지웠으면 끝이다). 읽지 못하거나 상한보다 크면 사유를 돌려준다 — 확인할 수 없으면 지우지 않는 쪽이다.
fn retired_feedback_record_has_removal(record: &mut std::fs::File) -> Result<bool, String> {
    use std::io::Read as _;
    let mut buf = Vec::new();
    record
        .by_ref()
        .take(RETIRED_FEEDBACK_RECORD_READ_MAX + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("읽지 못했다: {e}"))?;
    if buf.len() as u64 > RETIRED_FEEDBACK_RECORD_READ_MAX {
        return Err(format!("{RETIRED_FEEDBACK_RECORD_READ_MAX}바이트보다 크다"));
    }
    Ok(String::from_utf8_lossy(&buf)
        .lines()
        .any(|l| matches!(l.split('\t').nth(2), Some("removed") | Some("partial"))))
}

/// 지우기 전의 관문이 내는 답.
enum RetiredFeedbackGate {
    /// 지우지 않는다 — 이 결과로 끝난다(지울 것이 없다 · 기준 경로가 아니다 · 볼 수 없다 · 기록 파일을 만들 수 없다 · 기록 파일이
    /// 이미 있다).
    Stop(RetiredFeedbackCleanup),
    /// 지워도 된다 — 이 기동이 기록 파일을 **방금 새로 만들었고** 그 핸들이 열려 있다(로그 자신).
    Go(std::fs::File),
}

/// ★지우기 **전에** 서는 관문(RULING-04 M5 · FIX 2 RULING-S1-PRODUCT-07 B) — 삭제 함수는 이 관문이 `Go` 를 낸 뒤에만 불린다.
///   ① 기준 경로가 `~/.cys` 꼴이 아니거나 폴더가 없거나 볼 수 없거나 폴더·링크가 아니면 거기서 끝난다(기록 파일을 만들지 않는다).
///   ② 기록 **파일**이 이미 있으면 — 완결된 줄이든 끊긴 줄이든 비었든, 어떤 모양이든 — **지우지 않는다**: 그 파일이 있다는
///      것은 앞선 기동이 지우기 직전까지 왔다는 뜻이다(한 번만). 알림은 그 내용을 보고 「이미 지웠다」 또는 「시도(attempt)의
///      끝이 없다 — 다시 지우지 않는다」를 한 줄로 말한다.
///   ③ 없으면 로그 자신으로 **새로 만든다** — 만들 수 없으면 지우지 않는다(아무것도 지우지 않았고 시도의 표식도 없으니 다음
///      기동이 다시 해 본다 — 다시 해 보는 것은 이 경우뿐이다).
fn retired_feedback_record_gate(
    cys_base: &std::path::Path,
    target: &std::path::Path,
    log: &std::path::Path,
) -> RetiredFeedbackGate {
    use RetiredFeedbackCleanup::{Absent, Failed, Skipped};
    use RetiredFeedbackGate::{Go, Stop};
    if !cys_base.is_absolute() || cys_base.file_name() != Some(std::ffi::OsStr::new(".cys")) {
        return Stop(Skipped(format!("기준 경로가 ~/.cys 꼴이 아니다: {}", cys_base.display())));
    }
    match std::fs::symlink_metadata(target) {
        Ok(m) if m.is_dir() || m.file_type().is_symlink() => {}
        // 이 기능은 폴더만 만들었다 — 같은 이름의 일반 파일은 지우지 않고, 시도의 표식(기록 파일)도 만들지 않는다
        Ok(_) => return Stop(Skipped("같은 이름의 일반 파일이다 — 이 기능이 만든 것이 아니다".to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Stop(Absent),
        Err(e) => return Stop(Failed(format!("상태 확인 실패: {e}"))),
    }
    match std::fs::symlink_metadata(log) {
        Ok(_) => {
            let why = match retired_feedback_open_record(log, false)
                .and_then(|mut f| retired_feedback_record_has_removal(&mut f))
            {
                Ok(true) => format!("이미 한 번 지웠다(기록: {}) — 다시 지우지 않는다", log.display()),
                Ok(false) => format!(
                    "앞선 기동이 지우기 전에 기록 파일을 만들었고 그 끝(removed/partial)이 없다 — 시도(attempt)의 결과를 알 수 없어 \
                     다시 지우지 않는다(기록: {}). 이 폴더는 그대로 남고 앱이 나중에 지우지 않는다",
                    log.display()
                ),
                Err(why) => format!("기록 파일이 이미 있어(확인할 수 없다: {why}) 다시 지우지 않는다: {}", log.display()),
            };
            return Stop(Skipped(why));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Stop(Skipped(format!(
                "기록 파일 자리를 확인할 수 없어 지우지 않았다: {} — {e}",
                log.display()
            )))
        }
    }
    match retired_feedback_open_record(log, true) {
        Ok(f) => Go(f),
        Err(why) => Stop(Skipped(format!(
            "지운 것을 적을 기록 파일을 만들 수 없어 지우지 않았다(다음 기동이 다시 해 본다): {} — {why}",
            log.display()
        ))),
    }
}

/// `remove_retired_feedback_dir_at` 의 몸통 — 알림 줄을 받는 곳(`note`)을 주입받는다(프로덕션 = 표준 오류 ·
/// 테스트 = 모으는 그릇).
fn remove_retired_feedback_dir_noting(
    home: &std::path::Path,
    now_unix: u64,
    note: &mut dyn FnMut(&str),
) -> RetiredFeedbackCleanup {
    let mut retired_feedback_note = |msg: &str| note(msg);
    let base = home.join(".cys");
    let target = base.join(RETIRED_FEEDBACK_DIR);
    let log = base.join("state").join(RETIRED_FEEDBACK_RECORD);
    let out = match retired_feedback_record_gate(&base, &target, &log) {
        RetiredFeedbackGate::Stop(out) => out,
        RetiredFeedbackGate::Go(mut record) => {
            use std::io::Write as _;
            // ★FIX 2 (RULING-S1-PRODUCT-07 B): 기록 파일이 방금 생겼다 — 그것이 시도의 표식이다. 지우기 **전에** 시도 줄을 쓰고
            // 디스크까지 내린다. 쓰지 못하면 지우지 않는다 — 파일은 이미 있으므로 다음 기동도 지우지 않는다(폴더가 남는 쪽이다).
            let attempt = format!(
                "{now_unix}\t{}\tattempt\t{}\n",
                env!("CARGO_PKG_VERSION"),
                target.display()
            );
            match record.write_all(attempt.as_bytes()).and_then(|()| record.sync_all()) {
                Err(e) => RetiredFeedbackCleanup::Skipped(format!(
                    "기록 파일은 만들었으나 지우기 전의 줄(attempt)을 쓰지 못해 지우지 않았다: {} — {e}. 기록 파일이 있으므로 \
                     다음 기동도 지우지 않는다 — 이 폴더는 그대로 남고 앱이 나중에 지우지 않는다",
                    log.display()
                )),
                Ok(()) => {
                    let out = remove_retired_feedback_dir_in(&base);
                    if let Some(line) = retired_feedback_record_line(now_unix, &target, &out) {
                        // 끝 줄을 쓰지 못한 것(디스크가 찼다 등)은 조용히 넘기지 않는다 — 지운 내용은 바로 아래 알림 줄에 있다.
                        // 시도 줄이 이미 있으므로 다음 기동은 다시 지우지 않는다.
                        if let Err(e) = record.write_all(line.as_bytes()) {
                            retired_feedback_note(&format!(
                                "[cys-app] 지운 것의 기록을 끝맺지 못했습니다: {} — 쓰지 못했다: {e} (시도 줄이 있으므로 다음 기동은 다시 지우지 않는다)",
                                log.display()
                            ));
                        }
                    }
                    out
                }
            }
        }
    };
    match &out {
        RetiredFeedbackCleanup::Absent => {}
        RetiredFeedbackCleanup::Removed {
            entries,
            files,
            bytes,
            link,
        } => retired_feedback_note(&format!(
            "[cys-app] 걷어낸 기능이 남긴 폴더를 지웠습니다: {} (링크={link} · 직계 항목 {}개 · 파일 {files}개 · {bytes}바이트)",
            target.display(),
            entries.len()
        )),
        RetiredFeedbackCleanup::Partial {
            gone,
            files,
            bytes,
            left,
            why,
        } => retired_feedback_note(&format!(
            "[cys-app] 걷어낸 기능이 남긴 폴더를 일부만 지웠습니다: {} (사라진 직계 항목 {}개 · 파일 {files}개 · {bytes}바이트 · 남은 직계 항목 {}개) — {why}",
            target.display(),
            gone.len(),
            left.len()
        )),
        RetiredFeedbackCleanup::Skipped(why) | RetiredFeedbackCleanup::Failed(why) => retired_feedback_note(&format!(
            "[cys-app] 걷어낸 기능이 남긴 폴더를 정리하지 않았습니다: {} — {why}",
            target.display()
        )),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★배선 소스 핀 — 잔여 폴더 정리는 setup 부트 태스크에서, 안전 모드 반환(비정규 경로 기동) **뒤** · 업데이트 반영
    /// **바로 앞**에, 업데이트 반영과 **같은 블록 깊이**로, 조건·속성 없이 불린다(OS 전용 블록 안으로 옮기거나 조건을
    /// 씌우면 붉다). 앱의 재귀 삭제는 이 모듈의 한 곳뿐이고, 대상은 상수 이름 하나로 만든 경로다(패턴·접두 삭제가
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
        // 줄 단위: 호출은 그 한 문장뿐인 줄이고(같은 줄의 `if …`·`{ … }` 금지), 업데이트 반영과 들여쓰기가 같고,
        // 둘 사이에는 빈 줄·주석뿐이며, 호출 바로 앞의 문장 줄이 속성(`#[cfg…]`)이 아니다.
        let lines: Vec<&str> = prod.lines().collect();
        let ci = lines
            .iter()
            .position(|l| l.contains("retired_feedback::remove_retired_feedback_dir();"))
            .expect("호출 줄 소실");
        let ui = lines
            .iter()
            .position(|l| l.contains("maybe_apply_pending_update(&handle);"))
            .expect("업데이트 반영 줄 소실");
        assert_eq!(
            prod.matches("retired_feedback::remove_retired_feedback_dir();").count(),
            1,
            "호출이 한 곳이 아니다"
        );
        assert_eq!(
            lines[ci].trim(),
            "retired_feedback::remove_retired_feedback_dir();",
            "호출 줄에 다른 것이 붙었다(조건 등)"
        );
        let indent = |l: &str| l.len() - l.trim_start().len();
        assert_eq!(
            indent(lines[ci]),
            indent(lines[ui]),
            "호출이 업데이트 반영과 다른 블록 깊이에 있다(조건문·OS 전용 블록 안)"
        );
        assert!(
            ci < ui
                && lines[ci + 1..ui].iter().all(|l| {
                    let t = l.trim();
                    t.is_empty() || t.starts_with("//")
                }),
            "호출과 업데이트 반영 사이에 다른 문장이 끼었다"
        );
        let prev = lines[..ci]
            .iter()
            .rev()
            .map(|l| l.trim())
            .find(|t| !t.is_empty() && !t.starts_with("//"))
            .expect("호출 앞 문장 소실");
        assert!(!prev.starts_with("#["), "호출에 속성이 붙었다: {prev}");
        assert_eq!(prod.matches("\nmod retired_feedback;\n").count(), 1, "모듈 선언이 하나가 아니다");
        assert_eq!(prod.matches("remove_dir_all(").count(), 0, "main.rs 프로덕션 코드에 재귀 삭제가 생겼다");
        let me = include_str!("retired_feedback.rs");
        let mine = &me[..me.find("#[cfg(test)]\nmod tests {").expect("이 모듈의 테스트 경계 소실")];
        assert_eq!(mine.matches("remove_dir_all(").count(), 1, "이 모듈의 재귀 삭제가 한 곳이 아니다");
        assert_eq!(mine.matches("remove_file(").count(), 1, "이 모듈의 파일 삭제가 한 곳(링크)이 아니다");
        assert_eq!(mine.matches("remove_dir(").count(), 1, "이 모듈의 빈 디렉터리 삭제가 한 곳(링크)이 아니다");
        assert!(mine.contains("const RETIRED_FEEDBACK_DIR: &str = \"feedback\";"), "대상 이름 상수 소실");
        assert!(
            mine.contains("let _ = remove_retired_feedback_dir_at(&cys::home_dir(), now);"),
            "부트가 부르는 함수가 사용자 홈(cys::home_dir)을 몸통에 넘기지 않는다"
        );
        assert!(mine.contains("let base = home.join(\".cys\");"), "몸통의 기준 경로가 `<home>/.cys` 가 아니다");
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
    /// `root` 아래 전부(상대 경로 → 종류와 내용)의 목록. `skip` 으로 시작하는 경로는 뺀다. 「그 폴더 밖은 한 바이트도
    /// 바뀌지 않았다」를 형제 몇 개가 아니라 **트리 전체**로 재는 데 쓴다.
    fn s1_fix2_tree(root: &std::path::Path, skip: &[std::path::PathBuf]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for ent in std::fs::read_dir(&d).unwrap().flatten() {
                let p = ent.path();
                if skip.iter().any(|s| p.starts_with(s)) {
                    continue;
                }
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                let ft = ent.file_type().unwrap();
                if ft.is_dir() {
                    out.push((rel, "<dir>".to_string()));
                    stack.push(p);
                } else if ft.is_symlink() {
                    out.push((rel, "<link>".to_string()));
                } else {
                    out.push((rel, std::fs::read_to_string(&p).unwrap_or_else(|_| "<unreadable>".to_string())));
                }
            }
        }
        out.sort();
        out
    }
    fn s1_fix2_log(home: &std::path::Path) -> std::path::PathBuf {
        home.join(".cys")
            .join("state")
            .join("feedback-removed.log")
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
            base.join("accounts.json"),
            base.join("depts.json"),
            home.join("feedback/x.txt"),
            home.join(".cys-feedback/x.txt"),
        ];
        for k in &keep {
            s1_fix2_put(k, "KEEP");
        }
        let skip = [base.join("feedback")];
        let before = s1_fix2_tree(&home, &skip);
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
        assert_eq!(s1_fix2_tree(&home, &skip), before, "그 폴더 밖의 무엇이 생기거나 사라지거나 바뀌었다");
        assert_eq!(remove_retired_feedback_dir_in(&base), RetiredFeedbackCleanup::Absent, "두 번째 실행이 무언가를 했다");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ★이 기능이 만든 모양이 아니면 손대지 않는다 — 폴더 없음 · 같은 이름의 일반 파일 · `.cys` 가 아닌 기준 경로
    /// (이름이 `.cys` 로 **끝나기만** 하는 폴더 포함) · 상대 경로(홈을 못 구한 경우: 현재 디렉터리 기준으로 지우지 않는다).
    #[test]
    fn retired_feedback_cleanup_refuses_shapes_the_feature_never_made() {
        let home = s1_fix2_home("shapes");
        let base = home.join(".cys");
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(remove_retired_feedback_dir_in(&base), RetiredFeedbackCleanup::Absent);
        s1_fix2_put(&base.join("feedback"), "A FILE");
        assert!(matches!(remove_retired_feedback_dir_in(&base), RetiredFeedbackCleanup::Skipped(_)));
        assert_eq!(std::fs::read_to_string(base.join("feedback")).ok().as_deref(), Some("A FILE"));
        for odd in ["not-cys", "my.cys", ".cys2", "x.cys"] {
            let other = home.join(odd);
            s1_fix2_put(&other.join("feedback/x.txt"), "KEEP");
            assert!(
                matches!(remove_retired_feedback_dir_in(&other), RetiredFeedbackCleanup::Skipped(_)),
                "기준 경로 {odd:?} 로 움직였다"
            );
            assert_eq!(std::fs::read_to_string(other.join("feedback/x.txt")).ok().as_deref(), Some("KEEP"));
        }
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

    /// 기록 한 줄 — 지운 것(경로·수치·직계 항목 이름)을 말하고, 지운 것이 없으면 줄이 없다. 항목 이름에 줄바꿈·탭이
    /// 들어 있어도 기록은 **한 줄 · 아홉 칸**이다(이름으로 가짜 줄을 만들 수 없다).
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
        let odd = RetiredFeedbackCleanup::Removed {
            entries: vec!["fb-1\n1795000001\t0.0.0\tremoved\t/elsewhere".to_string()],
            files: 1,
            bytes: 1,
            link: false,
        };
        let line = retired_feedback_record_line(1795000000, t, &odd).unwrap();
        assert_eq!(line.matches('\n').count(), 1, "이름 속의 줄바꿈이 기록에 두 번째 줄을 만들었다: {line:?}");
        assert!(line.ends_with('\n'));
        assert_eq!(line.split('\t').count(), 9, "이름 속의 탭이 기록의 칸을 늘렸다: {line:?}");
    }

    /// ★몸통(홈 주입) — 받은 홈의 `.cys/feedback` 을 지우고, 무엇을 지웠는지 `<home>/.cys/state/` 의 기록 파일에
    /// **덧붙인다**. 그 폴더와 기록 파일 밖은 트리 전체가 그대로다. 없으면 아무 일도 없고, 홈을 못 구한 경우(상대 경로)에는
    /// 움직이지 않는다.
    #[test]
    fn retired_feedback_wrapper_cleans_the_given_home_and_appends_the_record() {
        let home = s1_fix2_home("wrapper");
        let base = home.join(".cys");
        s1_fix2_put(&base.join("feedback/fb-1/description.txt"), "abc");
        for k in ["accounts.json", "depts.json", "state/other.log", "pack/bin/tool", "feedback2/x.txt"] {
            s1_fix2_put(&base.join(k), "KEEP");
        }
        s1_fix2_put(&home.join("notes.txt"), "KEEP");
        let log = s1_fix2_log(&home);
        let skip = [base.join("feedback"), log.clone()];
        let before = s1_fix2_tree(&home, &skip);
        let out = remove_retired_feedback_dir_at(&home, 1795000001);
        assert_eq!(
            out,
            RetiredFeedbackCleanup::Removed {
                entries: vec!["fb-1".to_string()],
                files: 1,
                bytes: 3,
                link: false
            }
        );
        assert!(std::fs::symlink_metadata(base.join("feedback")).is_err(), "받은 홈의 폴더가 남아 있다");
        // FIX 2 (RULING-07 B): 기록은 시도 줄(attempt)이 먼저, 지운 뒤 끝 줄(removed)
        let line1 = format!(
            "1795000001\t{v}\tattempt\t{t}\n1795000001\t{v}\tremoved\t{t}\tlink=false\tentries=1\tfiles=1\tbytes=3\tfb-1\n",
            v = env!("CARGO_PKG_VERSION"),
            t = base.join("feedback").display()
        );
        assert_eq!(std::fs::read_to_string(&log).ok().as_deref(), Some(line1.as_str()), "기록 파일이 없거나 다르다");
        assert_eq!(s1_fix2_tree(&home, &skip), before, "그 폴더와 기록 파일 밖의 무엇이 바뀌었다");
        // ★한 번 지운 기록이 있으면 다시 지우지 않는다(RULING-04 M5) — 그 자리에 나중에 생긴 폴더는 그대로 남고 기록도 그대로다
        s1_fix2_put(&base.join("feedback/fb-2/diag.json"), "{}");
        let out2 = remove_retired_feedback_dir_at(&home, 1795000002);
        assert!(
            matches!(out2, RetiredFeedbackCleanup::Skipped(_)),
            "이미 지운 기록이 있는데 또 지웠다: {out2:?}"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("feedback/fb-2/diag.json")).ok().as_deref(),
            Some("{}"),
            "나중에 그 자리에 생긴 폴더를 지웠다"
        );
        let text = std::fs::read_to_string(&log).unwrap();
        assert_eq!(text, line1, "두 번째 기동이 기록을 바꿨다");
        // 그 폴더가 (사람 손으로) 사라진 뒤에는 아무 일도 없다 — 기록도 늘지 않는다
        std::fs::remove_dir_all(base.join("feedback")).unwrap();
        assert_eq!(remove_retired_feedback_dir_at(&home, 1795000003), RetiredFeedbackCleanup::Absent);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), text);
        assert_eq!(s1_fix2_tree(&home, &skip), before);
        // 홈을 못 구한 경우(상대 경로)에는 움직이지 않는다
        for rel in ["", ".", "relative-home"] {
            assert!(
                matches!(
                    remove_retired_feedback_dir_at(std::path::Path::new(rel), 1),
                    RetiredFeedbackCleanup::Skipped(_)
                ),
                "상대 홈 {rel:?} 로 움직였다"
            );
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ★재귀 삭제가 도중에 멈추면 「지웠다」고도 「지우지 않았다」고도 하지 않는다 — 사라진 묶음을 기록에 그대로 적고
    /// (`partial`). 사라진 것이 **기록된** 뒤에는 다음 기동이 다시 지우지 않는다(한 번만 · RULING-04 M5) — 남은 묶음은 그대로 남는다.
    /// 하나도 못 지웠으면 실패다. FIX 2 (RULING-S1-PRODUCT-07 B): 기록 파일은 지우기 전에 생겼고 시도 줄(attempt)만 남는다 —
    /// 끝 줄이 없는 기록이므로 다음 기동도 **지우지 않는다**(폴더가 남는 쪽이다. 다시 해 보는 것은 기록 파일을 만들 수조차 없던 경우뿐).
    #[cfg(unix)]
    #[test]
    fn retired_feedback_halfway_failure_records_what_is_gone() {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |p: &std::path::Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
        let names = |d: &std::path::Path| {
            let mut v: Vec<String> = std::fs::read_dir(d)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        // ① 하나도 못 지우는 경우: 묶음 하나, 그 안은 쓰기 금지 폴더 하나뿐
        let home = s1_fix2_home("stuck");
        let fb = home.join(".cys").join("feedback");
        s1_fix2_put(&fb.join("fb-only/attach/shot.png"), "PNG");
        let lock = fb.join("fb-only/attach");
        mode(&lock, 0o555);
        if std::fs::write(lock.join("probe"), "x").is_ok() {
            // 권한을 무시하는 사용자(root)로는 삭제 실패를 만들 수 없다 — 이 검체는 잴 것이 없다.
            mode(&lock, 0o755);
            let _ = std::fs::remove_dir_all(&home);
            return;
        }
        let out = remove_retired_feedback_dir_at(&home, 1795000008);
        mode(&lock, 0o755);
        assert!(matches!(out, RetiredFeedbackCleanup::Failed(_)), "하나도 못 지웠는데 실패가 아니다: {out:?}");
        assert!(lock.join("shot.png").exists());
        let attempt_only = format!(
            "1795000008\t{}\tattempt\t{}\n",
            env!("CARGO_PKG_VERSION"),
            fb.display()
        );
        assert_eq!(
            std::fs::read_to_string(s1_fix2_log(&home)).unwrap_or_default(),
            attempt_only,
            "사라진 것이 없으면 기록은 시도 줄 하나뿐이어야 한다(끝 줄 없음)"
        );
        // 끝 줄이 없는 기록이 있으므로 다음 기동은 장애물이 사라졌어도 **지우지 않는다** — 폴더와 기록이 그대로다
        let out_retry = remove_retired_feedback_dir_at(&home, 1795000018);
        assert!(matches!(out_retry, RetiredFeedbackCleanup::Skipped(_)), "{out_retry:?}");
        assert!(lock.join("shot.png").exists(), "끝 줄 없는 기록이 있는데 다음 기동이 지웠다");
        assert_eq!(std::fs::read_to_string(s1_fix2_log(&home)).unwrap_or_default(), attempt_only);
        let _ = std::fs::remove_dir_all(&home);

        // ② 일부만 지워지는 경우: 묶음 넷. 재귀 삭제는 디렉터리 읽기 순서로 지우다 첫 오류에서 멈추므로,
        //    읽기 순서의 **마지막** 묶음을 못 지우게 만들면 그 앞의 셋이 사라진다.
        let home = s1_fix2_home("partial");
        let fb = home.join(".cys").join("feedback");
        for b in ["fb-a", "fb-b", "fb-c", "fb-d"] {
            s1_fix2_put(&fb.join(b).join("description.txt"), "12345");
            s1_fix2_put(&fb.join(b).join("attach/shot.png"), "PNG");
        }
        let order: Vec<String> = std::fs::read_dir(&fb)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let stuck = order.last().unwrap().clone();
        let lock = fb.join(&stuck).join("attach");
        mode(&lock, 0o555);
        let out = remove_retired_feedback_dir_at(&home, 1795000009);
        mode(&lock, 0o755);
        assert!(
            !matches!(out, RetiredFeedbackCleanup::Removed { .. }),
            "도중에 멈춘 삭제를 '지웠다'로 돌려주었다: {out:?}"
        );
        let left = names(&fb);
        assert!(left.contains(&stuck), "못 지우게 만든 묶음이 사라졌다");
        let gone: Vec<String> = order.iter().filter(|n| !left.contains(n)).cloned().collect();
        let log = s1_fix2_log(&home);
        // (읽기 순서가 달라 앞의 묶음이 하나도 사라지지 않은 파일 시스템이면 이름으로 잴 것이 없다 — 아래 '다음 기동'만 잰다.)
        if !gone.is_empty() {
            let text = std::fs::read_to_string(&log).expect("묶음이 사라졌는데 기록 파일이 없다");
            // FIX 2 (RULING-07 B): 첫 줄은 시도 줄, 둘째 줄이 partial
            assert_eq!(text.lines().count(), 2);
            assert_eq!(text.lines().next().unwrap().split('\t').nth(2), Some("attempt"));
            let f: Vec<&str> = text.lines().nth(1).unwrap().split('\t').collect();
            assert_eq!(f.len(), 11, "partial 기록의 칸 수: {f:?}");
            assert_eq!(f[0], "1795000009");
            assert_eq!(f[2], "partial");
            assert_eq!(f[3], fb.display().to_string());
            assert_eq!(f[5], format!("entries={}", gone.len()));
            let mut named: Vec<String> = f[8].split(',').map(|s| s.to_string()).collect();
            named.sort();
            let mut gone_sorted = gone.clone();
            gone_sorted.sort();
            assert_eq!(named, gone_sorted, "기록의 이름이 사라진 묶음과 다르다");
            let files: u64 = f[6].strip_prefix("files=").unwrap().parse().unwrap();
            let g = gone.len() as u64;
            assert!(files == 2 * g || files == 2 * g + 1, "사라진 파일 수가 맞지 않는다: {files} (묶음 {g}개)");
            assert_eq!(f[9], format!("left={}", left.join(",")));
            assert!(f[10].contains("삭제 실패"));
        }
        // 장애물이 사라진 다음 기동: 기록 파일이 있으므로(partial 이든, 하나도 못 지워 시도 줄뿐이든) **다시 지우지 않는다** —
        // 남은 묶음과 기록은 그대로다(FIX 2 · RULING-07 B).
        let record_before = std::fs::read_to_string(&log).expect("지우기를 시도했는데 기록 파일이 없다");
        let out2 = remove_retired_feedback_dir_at(&home, 1795000010);
        assert!(
            matches!(out2, RetiredFeedbackCleanup::Skipped(_)),
            "기록 파일이 있는데 또 지웠다: {out2:?}"
        );
        assert_eq!(names(&fb), left, "남은 묶음이 바뀌었다");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), record_before);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// ★상태 확인이 「없음」이 아닌 오류로 끝나면 「없음」으로 접지 않는다(권한이 없어 못 본 것을 '할 일 없음'으로
    /// 읽으면 폴더가 남아도 아무도 모른다).
    #[cfg(unix)]
    #[test]
    fn retired_feedback_stat_error_is_not_read_as_absent() {
        use std::os::unix::fs::PermissionsExt as _;
        let home = s1_fix2_home("stat");
        let base = home.join(".cys");
        s1_fix2_put(&base.join("feedback/fb-1/description.txt"), "x");
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o000)).unwrap();
        let privileged = std::fs::read_dir(&base).is_ok();
        let out = remove_retired_feedback_dir_in(&base);
        // 지우기 전의 관문도 같은 답을 낸다(몸통을 거친 길) — 못 본 것을 「없음」으로 접으면 알림도 없이 지나간다
        let out_gate = remove_retired_feedback_dir_at(&home, 1795000020);
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
        if !privileged {
            assert!(matches!(out, RetiredFeedbackCleanup::Failed(_)), "볼 수 없는 폴더를 다르게 읽었다: {out:?}");
            assert!(
                matches!(out_gate, RetiredFeedbackCleanup::Failed(_)),
                "관문이 볼 수 없는 폴더를 다르게 읽었다: {out_gate:?}"
            );
            assert!(base.join("feedback/fb-1/description.txt").exists());
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    // ───────── S1 수정2 · 보완(리뷰 M1 · RULING-04 M5) — 기록은 로그 자신에만 · 삭제는 한 번만 ─────────
    /// 리뷰어 검체 11 의 모양: `<root>/home` 에 묶음 하나(`.cys/feedback/bundle/x` 1바이트)와 빈 `.cys/state/`,
    /// 그 옆 `<root>/outside/` 는 홈 **밖**이다. (root, home, outside) 를 돌려준다.
    #[cfg(unix)]
    fn s1_fix2_m1_fixture(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let root = s1_fix2_home(tag);
        let home = root.join("home");
        let outside = root.join("outside");
        s1_fix2_put(&home.join(".cys/feedback/bundle/x"), "x");
        std::fs::create_dir_all(home.join(".cys/state")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        (root, home, outside)
    }
    /// 「지우지 않았다」의 검사 묶음: 결과가 건너뜀이고, 묶음이 그대로이고, 알림은 「정리하지 않았습니다」 한 줄(기록 경로 포함)뿐이다.
    #[cfg(unix)]
    fn s1_fix2_not_deleted(
        tag: &str,
        home: &std::path::Path,
        log: &std::path::Path,
        out: &RetiredFeedbackCleanup,
        notes: &[String],
        bad: &mut Vec<String>,
    ) {
        if !matches!(out, RetiredFeedbackCleanup::Skipped(_)) {
            bad.push(format!("{tag}: 건너뜀이 아니다: {out:?}"));
        }
        if std::fs::read_to_string(home.join(".cys/feedback/bundle/x")).ok().as_deref() != Some("x") {
            bad.push(format!("{tag}: 기록할 자리가 없는데(또는 이미 지웠는데) 폴더를 지웠다"));
        }
        let told = notes
            .iter()
            .filter(|n| n.contains("정리하지 않았습니다") && n.contains(&log.display().to_string()))
            .count();
        if told != 1 || notes.len() != 1 {
            bad.push(format!("{tag}: 알림이 「정리하지 않았습니다」(기록 경로 포함) 한 줄이 아니다: {notes:?}"));
        }
    }

    /// ★기록 줄은 **로그 자신인 일반 파일**에만 덧붙고, 그 자리를 **지우기 전에** 연다. 기록 파일 자리에 다른 곳을 가리키는 것
    /// (파일로 가는 링크 · 끊긴 링크 · 폴더로 가는 링크 · 다른 파일과 몸을 나눈 하드 링크)이 놓여 있으면: 그 너머에 한 바이트도
    /// 쓰지 않고, 그 자리의 것을 바꾸지 않고, **폴더를 지우지 않고**, 그 사실을 한 줄로 알린다(기록할 수 없는 삭제는 하지 않는다).
    /// 대조(`regular-fresh`): 아직 없는 기록 파일이면 새로 만들고, 시도 줄을 쓰고, 지우고, 끝 줄을 덧붙인다.
    /// FIX 2 (RULING-S1-PRODUCT-07 B): `regular`(이미 있는 평범한 기록 파일 · 끝 줄 없음)는 앞선 시도의 표식이므로 이제 지우지 않는다.
    /// (리뷰어 재현 — 파일로 가는 링크: 홈 밖의 `unrelated-notes.txt` 가 15 → 214 바이트가 됐다.)
    #[cfg(unix)]
    #[test]
    fn retired_feedback_record_is_written_only_into_the_log_itself() {
        use std::os::unix::fs::MetadataExt as _;
        let mut bad: Vec<String> = Vec::new();
        for shape in ["regular", "regular-fresh", "link-to-file", "link-dangling", "link-to-dir", "hard-link", "state-is-a-file"] {
            let (root, home, outside) = s1_fix2_m1_fixture(&format!("m1-{shape}"));
            let log = s1_fix2_log(&home);
            let notes_file = outside.join("unrelated-notes.txt");
            match shape {
                "regular" => std::fs::write(&log, "EARLIER LINE\n").unwrap(),
                // 첫 실행: 기록 파일도 `state/` 폴더도 아직 없다 — 둘 다 만들어지고 줄이 남는다
                "regular-fresh" => std::fs::remove_dir(home.join(".cys/state")).unwrap(),
                "link-to-file" => {
                    std::fs::write(&notes_file, "ORIGINAL NOTES\n").unwrap();
                    std::os::unix::fs::symlink(&notes_file, &log).unwrap();
                }
                "link-dangling" => std::os::unix::fs::symlink(&notes_file, &log).unwrap(),
                "link-to-dir" => std::os::unix::fs::symlink(&outside, &log).unwrap(),
                "hard-link" => {
                    std::fs::write(&notes_file, "ORIGINAL NOTES\n").unwrap();
                    std::fs::hard_link(&notes_file, &log).unwrap();
                }
                // 기록 파일이 놓일 폴더 자리에 일반 파일이 있다 — 기록 파일을 만들 수 없다
                "state-is-a-file" => {
                    std::fs::remove_dir(home.join(".cys/state")).unwrap();
                    std::fs::write(home.join(".cys/state"), "NOT A FOLDER\n").unwrap();
                }
                _ => unreachable!(),
            }
            let outside_before = s1_fix2_tree(&outside, &[]);
            let mut notes: Vec<String> = Vec::new();
            let out = remove_retired_feedback_dir_noting(&home, 1795344000, &mut |m: &str| notes.push(m.to_string()));
            // FIX 2 (RULING-07 B): the record is the attempt line first, then the removed line
            let line = format!(
                "1795344000\t{v}\tattempt\t{t}\n1795344000\t{v}\tremoved\t{t}\tlink=false\tentries=1\tfiles=1\tbytes=1\tbundle\n",
                v = env!("CARGO_PKG_VERSION"),
                t = home.join(".cys/feedback").display()
            );
            // FIX 2 (RULING-07 B): only a start that CREATES the record file may delete - an existing regular record ("regular",
            // with an earlier line and no end line) is an earlier attempt, so it now stops like the other shapes
            if shape == "regular-fresh" {
                let want = RetiredFeedbackCleanup::Removed {
                    entries: vec!["bundle".to_string()],
                    files: 1,
                    bytes: 1,
                    link: false,
                };
                if out != want {
                    bad.push(format!("{shape}: 삭제 결과가 다르다: {out:?}"));
                }
                if std::fs::symlink_metadata(home.join(".cys/feedback")).is_ok() {
                    bad.push(format!("{shape}: 지울 폴더가 남았다"));
                }
                let text = std::fs::read_to_string(&log).unwrap_or_default();
                if text != line {
                    bad.push(format!("{shape}: 새 기록 파일이 시도 줄 + 지운 줄이 아니다: {text:?}"));
                }
                if notes.len() != 1 || !notes[0].contains("폴더를 지웠습니다") {
                    bad.push(format!("{shape}: 알림이 「지웠습니다」 한 줄이 아니다: {notes:?}"));
                }
            } else {
                s1_fix2_not_deleted(shape, &home, &log, &out, &notes, &mut bad);
                let outside_after = s1_fix2_tree(&outside, &[]);
                if outside_after != outside_before {
                    bad.push(format!("{shape}: 홈 밖이 바뀌었다: {outside_before:?} -> {outside_after:?}"));
                }
                // 그 자리의 것은 그대로다(링크는 같은 곳을 가리키는 링크로 · 하드 링크는 이름 둘인 채로 · 파일은 그 내용으로)
                let same = match shape {
                    "regular" => std::fs::read_to_string(&log).ok().as_deref() == Some("EARLIER LINE\n"),
                    "state-is-a-file" => {
                        std::fs::read_to_string(home.join(".cys/state")).ok().as_deref() == Some("NOT A FOLDER\n")
                    }
                    "hard-link" => std::fs::symlink_metadata(&log)
                        .map(|m| m.file_type().is_file() && m.nlink() == 2)
                        .unwrap_or(false),
                    _ => {
                        let to = if shape == "link-to-dir" { &outside } else { &notes_file };
                        std::fs::symlink_metadata(&log).map(|m| m.file_type().is_symlink()).unwrap_or(false)
                            && std::fs::read_link(&log).ok().as_deref() == Some(to.as_path())
                    }
                };
                if !same {
                    bad.push(format!("{shape}: 기록 파일 자리의 것이 바뀌었다"));
                }
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(
            bad.is_empty(),
            "기록이 로그 자신 밖에 쓰였거나, 기록할 자리 없이 지웠거나, 알리지 않았다:\n{}",
            bad.join("\n")
        );
    }

    /// ★삭제는 **한 번만**이다(RULING-04 M5 — 오너의 명령은 「업데이트되면 지운다」이고, 기동마다 되풀이되는 삭제는 나중에 그
    /// 자리에 놓인 것을 지운다). 표식은 이미 쓰는 기록이다: 기록에 「지웠다」 줄(`removed` · `partial`)이 있으면 — 어느 판이
    /// 적었든 — 다시 지우지 않는다. 기록 파일 이름에는 판 번호가 없다(판이 바뀌어도 같은 표식을 본다).
    /// 표식이 아닌 것(셋째 칸이 아닌 자리의 낱말)은 「지웠다」로 읽지 않고, 읽을 수 없을 만큼 큰 기록은 「확인할 수 없다」 = 지우지 않는다.
    /// FIX 2 (RULING-S1-PRODUCT-07 B): 어느 모양이든 기록 **파일**이 이미 있으면 지우지 않는다 — 그 내용은 알림의 문장만 가른다.
    #[cfg(unix)]
    #[test]
    fn retired_feedback_delete_runs_once_and_the_record_is_the_marker() {
        let mut bad: Vec<String> = Vec::new();
        // ① 두 번의 기동: 첫 기동은 지우고 적는다. 그 자리에 다시 생긴 폴더는 둘째 기동이 건드리지 않는다.
        {
            let (root, home, _outside) = s1_fix2_m1_fixture("m5-twice");
            let state = home.join(".cys/state");
            let record = state.join("feedback-removed.log");
            let mut notes: Vec<String> = Vec::new();
            let out1 = remove_retired_feedback_dir_noting(&home, 1795344000, &mut |m: &str| notes.push(m.to_string()));
            if !matches!(out1, RetiredFeedbackCleanup::Removed { files: 1, bytes: 1, link: false, .. }) {
                bad.push(format!("twice: 첫 기동이 지우지 않았다: {out1:?}"));
            }
            let listed: Vec<String> = std::fs::read_dir(&state)
                .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                .unwrap_or_default();
            if listed != vec!["feedback-removed.log".to_string()] {
                bad.push(format!("twice: 기록 파일이 판 번호 없는 이름 하나가 아니다: {listed:?}"));
            }
            let text1 = std::fs::read_to_string(&record).unwrap_or_default();
            // FIX 2 (RULING-07 B): 시도 줄(attempt) 다음에 「지웠다」 줄
            let kinds: Vec<&str> = text1.lines().filter_map(|l| l.split('\t').nth(2)).collect();
            if kinds != vec!["attempt", "removed"] {
                bad.push(format!("twice: 첫 기동의 기록이 시도 줄 + 「지웠다」 줄이 아니다: {text1:?}"));
            }
            s1_fix2_put(&home.join(".cys/feedback/bundle/x"), "x");
            s1_fix2_put(&home.join(".cys/feedback/later/keep.txt"), "PLACED LATER");
            let mut notes2: Vec<String> = Vec::new();
            let out2 = remove_retired_feedback_dir_noting(&home, 1795344099, &mut |m: &str| notes2.push(m.to_string()));
            s1_fix2_not_deleted("twice(둘째 기동)", &home, &record, &out2, &notes2, &mut bad);
            if std::fs::read_to_string(home.join(".cys/feedback/later/keep.txt")).ok().as_deref() != Some("PLACED LATER") {
                bad.push("twice: 나중에 그 자리에 놓인 파일을 지웠다".to_string());
            }
            if std::fs::read_to_string(&record).unwrap_or_default() != text1 {
                bad.push("twice: 둘째 기동이 기록을 바꿨다".to_string());
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        // ② 표식의 모양: (기록 내용, 지워야 하는가). FIX 2 (RULING-S1-PRODUCT-07 B): 기록 **파일**이 있으면 내용과 상관없이
        //    지우지 않는다 — 내용은 알림이 「이미 지웠다」인지 「시도의 끝이 없다」인지만 가른다. 그래서 지우는 모양은 이제 없다.
        let other_version_removed = "1790000000\t0.0.1\tremoved\t/somewhere/.cys/feedback\tlink=false\tentries=1\tfiles=1\tbytes=1\tb\n";
        let partial = "1790000000\t0.0.1\tpartial\t/somewhere/.cys/feedback\tlink=false\tentries=1\tfiles=1\tbytes=1\tb\tleft=c\twhy\n";
        let word_elsewhere = "note\tremoved\tsomething\tremoved\tpartial\n";
        let too_big = "x".repeat(1024 * 1024 + 1);
        let after_other_lines = format!("EARLIER LINE\n{other_version_removed}");
        for (tag, content, deletes) in [
            ("marker-other-version", other_version_removed, false),
            ("marker-partial", partial, false),
            ("marker-word-elsewhere", word_elsewhere, false),
            ("marker-after-other-lines", after_other_lines.as_str(), false),
            ("marker-too-big-to-read", too_big.as_str(), false),
        ] {
            let (root, home, _outside) = s1_fix2_m1_fixture(&format!("m5-{tag}"));
            let record = home.join(".cys/state/feedback-removed.log");
            std::fs::write(&record, content).unwrap();
            let mut notes: Vec<String> = Vec::new();
            let out = remove_retired_feedback_dir_noting(&home, 1795344000, &mut |m: &str| notes.push(m.to_string()));
            let after = std::fs::read_to_string(&record).unwrap_or_default();
            if deletes {
                if !matches!(out, RetiredFeedbackCleanup::Removed { files: 1, .. }) {
                    bad.push(format!("{tag}: 표식이 아닌데 지우지 않았다: {out:?}"));
                }
                if !after.starts_with(content) || after.lines().count() != content.lines().count() + 1 {
                    bad.push(format!("{tag}: 기록에 한 줄이 덧붙지 않았다"));
                }
            } else {
                s1_fix2_not_deleted(tag, &home, &record, &out, &notes, &mut bad);
                if after != content {
                    bad.push(format!("{tag}: 지우지 않았는데 기록이 바뀌었다"));
                }
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(bad.is_empty(), "한 번만 지운다는 규칙이 깨졌다:\n{}", bad.join("\n"));
    }

    /// ★기록 파일 자리에 파이프(FIFO)가 놓여 있어도 그 앞에서 **멈춰 서지 않고**, 파이프에 **쓰지 않고**, 기록할 자리가
    /// 없으므로 **지우지 않는다**. 읽는 쪽이 없는 파이프를 쓰기 전용으로 열면 열기가 끝없이 기다린다 — 부트 태스크가 거기서
    /// 서면 앱이 뜨지 않는다. 읽는 쪽이 있는 파이프는 열기가 곧바로 되므로, 「연 것이 일반 파일인가」 확인만이 기록 줄과 파이프
    /// 사이에 서 있다. 멈춤은 다른 스레드에서 돌리고 시한으로 잰다.
    #[cfg(unix)]
    #[test]
    fn retired_feedback_record_never_waits_on_a_pipe_at_the_log_path() {
        use std::io::Read as _;
        use std::os::unix::ffi::OsStrExt as _;
        use std::os::unix::fs::{FileTypeExt as _, OpenOptionsExt as _};
        let mut bad: Vec<String> = Vec::new();
        for with_reader in [false, true] {
            let tag = if with_reader { "m1-fifo-reader" } else { "m1-fifo" };
            let (root, home, _outside) = s1_fix2_m1_fixture(tag);
            let log = s1_fix2_log(&home);
            let c_log = std::ffi::CString::new(log.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c_log.as_ptr(), 0o600) }, 0, "파이프를 만들지 못했다");
            let open_reader = || std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&log);
            let mut reader = if with_reader { Some(open_reader().expect("읽는 쪽을 열지 못했다")) } else { None };
            let (tx, rx) = std::sync::mpsc::channel();
            let home_for_worker = home.clone();
            let worker = std::thread::spawn(move || {
                let mut notes: Vec<String> = Vec::new();
                let out = remove_retired_feedback_dir_noting(&home_for_worker, 1795344000, &mut |m: &str| {
                    notes.push(m.to_string())
                });
                let _ = tx.send((out, notes));
            });
            let got = rx.recv_timeout(std::time::Duration::from_secs(20)); // wallclock: guard 멈춤 감시 — 정상 경로는 열기가 기다리지 않아 곧바로 돌아온다. 이 시한은 결함(끝없는 대기)일 때만 닿는다
            if got.is_err() {
                bad.push(format!("{tag}: 기록 파일 자리의 파이프 앞에서 멈춰 섰다(20초 안에 돌아오지 않았다)"));
                // 멈춰 선 쓰는 쪽을 풀어 준다(읽는 쪽이 열리면 열기가 끝난다) — 테스트 프로세스에 선 스레드를 남기지 않는다.
                if reader.is_none() {
                    reader = open_reader().ok();
                }
            }
            // 돌아온 경우에만 스레드를 거둔다 — 끝내 돌아오지 않는 스레드를 기다리면 이 테스트 자신이 선다(남은 스레드는 테스트
            // 프로세스가 끝날 때 함께 사라진다).
            if let Ok((out, notes)) = got {
                let _ = worker.join();
                s1_fix2_not_deleted(tag, &home, &log, &out, &notes, &mut bad);
            }
            // 파이프로 들어간 바이트가 없다(읽는 쪽이 있을 때만 잴 수 있다)
            if let Some(r) = reader.as_mut() {
                let mut buf = [0u8; 512];
                let n = r.read(&mut buf).unwrap_or(0);
                if with_reader && n != 0 {
                    bad.push(format!("{tag}: 파이프에 {n}바이트가 쓰였다"));
                }
            }
            if !std::fs::symlink_metadata(&log).map(|m| m.file_type().is_fifo()).unwrap_or(false) {
                bad.push(format!("{tag}: 기록 파일 자리의 파이프가 바뀌었다"));
            }
            drop(reader);
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(bad.is_empty(), "기록 파일 자리의 파이프를 잘못 다뤘다:\n{}", bad.join("\n"));
    }

    // ───────── S1 FIX 2 (ADDENDUM 6 · reviewer-codex r2 M5) — the record comes FIRST; a failed or cut-short record never lets a
    // later start delete ─────────
    /// ★FIX 2 (closes mutant F2-05): a same-name REGULAR FILE at `~/.cys/feedback` is not this feature's folder - the gate stops
    /// before the record is touched. The file stays, NO record file is created (a record file would be an attempt marker for a
    /// delete that never happens), and one note says why.
    #[test]
    fn retired_feedback_a_same_name_file_gets_no_record_and_no_attempt() {
        let home = s1_fix2_home("fix3-same-name-file");
        s1_fix2_put(&home.join(".cys/feedback"), "A FILE");
        let log = s1_fix2_log(&home);
        let mut notes: Vec<String> = Vec::new();
        let out = remove_retired_feedback_dir_noting(&home, 1795344000, &mut |m: &str| notes.push(m.to_string()));
        assert!(matches!(out, RetiredFeedbackCleanup::Skipped(_)), "not a skip: {out:?}");
        assert_eq!(std::fs::read_to_string(home.join(".cys/feedback")).ok().as_deref(), Some("A FILE"));
        assert!(
            std::fs::symlink_metadata(&log).is_err(),
            "a record file was created for a delete that never happens: {:?}",
            std::fs::read_to_string(&log)
        );
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("정리하지 않았습니다"), "{notes:?}");
        let _ = std::fs::remove_dir_all(&home);
    }
    /// The line the body must write and flush BEFORE it removes anything: unix seconds · app version · `attempt` · target path.
    #[cfg(unix)]
    fn s1_fix3_attempt_line(now: u64, home: &std::path::Path) -> String {
        format!(
            "{now}\t{}\tattempt\t{}\n",
            env!("CARGO_PKG_VERSION"),
            home.join(".cys/feedback").display()
        )
    }

    /// CHILD SIDE ONLY - in an ordinary test run the two variables are absent and this returns at once. The parent
    /// (`s1_fix3_run_limited`) starts this test binary again with them: the body then runs under a REAL file-size limit
    /// (`RLIMIT_FSIZE`, SIGXFSZ ignored - the reviewer's recorded case), so writes into the record fail the way a full disk does.
    #[cfg(unix)]
    #[test]
    fn s1_fix3_child_runs_the_body_under_a_file_size_limit() {
        let (Ok(home), Ok(limit)) = (std::env::var("S1_FIX3_CHILD_HOME"), std::env::var("S1_FIX3_FSIZE")) else {
            return;
        };
        let limit: libc::rlim_t = limit.parse().expect("S1_FIX3_FSIZE is not a number");
        unsafe {
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            let rl = libc::rlimit {
                rlim_cur: limit,
                rlim_max: limit,
            };
            assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &rl), 0, "setrlimit(RLIMIT_FSIZE) failed");
        }
        let mut notes: Vec<String> = Vec::new();
        let out = remove_retired_feedback_dir_noting(std::path::Path::new(&home), 1795344000, &mut |m: &str| {
            notes.push(m.to_string())
        });
        for n in &notes {
            println!("S1FIX3-NOTE {}", n.replace('\n', " "));
        }
        println!("S1FIX3-RESULT {out:?}");
    }

    /// Runs the body for `home` in a child process under a file-size limit of `limit` bytes. Returns (result line, notes).
    #[cfg(unix)]
    fn s1_fix3_run_limited(home: &std::path::Path, limit: u64) -> (String, Vec<String>) {
        let module = module_path!().split_once("::").map(|(_, rest)| rest).unwrap_or(module_path!());
        let name = format!("{module}::s1_fix3_child_runs_the_body_under_a_file_size_limit");
        let out = std::process::Command::new(std::env::current_exe().expect("no current_exe"))
            .args([name.as_str(), "--exact", "--nocapture", "--test-threads=1"])
            .env("S1_FIX3_CHILD_HOME", home)
            .env("S1_FIX3_FSIZE", limit.to_string())
            .output()
            .expect("could not start the child test process");
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        // libtest prints `test <name> ... ` on the same line before the child's first output line - find the marker anywhere
        let after = |l: &str, marker: &str| l.find(marker).map(|i| l[i + marker.len()..].to_string());
        let result = text
            .lines()
            .find_map(|l| after(l, "S1FIX3-RESULT "))
            .unwrap_or_else(|| {
                format!(
                    "<no result line - child status {:?}, stderr {:?}>",
                    out.status,
                    String::from_utf8_lossy(&out.stderr).chars().take(400).collect::<String>()
                )
            });
        let notes = text.lines().filter_map(|l| after(l, "S1FIX3-NOTE ")).collect();
        (result, notes)
    }

    /// ★FIX 2 (ADDENDUM 6 · reviewer-codex r2 M5): the one-time state is on disk BEFORE anything is removed, and a record that is
    /// missing its end - because a write failed or the process stopped between two steps - never lets a later start delete.
    /// At e56f86f5 the body deleted FIRST and wrote its only marker AFTER: under a real file-size limit of 0 (the reviewer's case)
    /// the folder went, the record stayed empty, and the next start deleted a file the user had put there later.
    /// RULING-S1-PRODUCT-07 (option B): the record FILE is the attempt - once it exists (complete, cut short or empty) no later
    /// start deletes; only a start that could not even create it leaves the next start free to try. Pass line: the later file
    /// survives in every case below.
    ///   ① limit 0 (nothing can be written into the record): nothing is removed, one note says so; the second start, with a file
    ///      placed later, deletes nothing either (the empty record file exists).
    ///   ② limit = the attempt line's length: the attempt line gets in, the folder is removed, the end line cannot be written;
    ///      a later start leaves the later file alone and says once that an attempt has no end. (At e56f86f5 this case passes by
    ///      accident: macOS writes the part of a line that fits, so a cut-off `removed` line protects the next start - measured
    ///      2026-11-23, a 20-byte write under a 10-byte limit returned 10, the next write EFBIG. Its record check is red there.)
    ///   ③ stop points, as the on-disk state each leaves: after the record file is created (empty) · after the attempt line ·
    ///      after the removal · after the end line.
    ///   ④ the ordinary case: removed once, the record is the attempt line then the `removed` line; a second start deletes nothing.
    #[cfg(unix)]
    #[test]
    fn retired_feedback_a_failed_or_cut_short_record_never_lets_a_later_start_delete() {
        let mut bad: Vec<String> = Vec::new();
        let now = 1795344000u64;
        let later = |home: &std::path::Path| home.join(".cys/feedback/later/keep.txt");
        let later_kept = |home: &std::path::Path| {
            std::fs::read_to_string(home.join(".cys/feedback/later/keep.txt")).ok().as_deref() == Some("PLACED LATER")
        };
        // ① the reviewer's recorded case, first start: file-size limit 0
        {
            let (root, home, _outside) = s1_fix2_m1_fixture("fix3-limit0");
            let log = s1_fix2_log(&home);
            let (result, notes) = s1_fix3_run_limited(&home, 0);
            if !result.starts_with("Skipped(") {
                bad.push(format!("limit-0: not a skip: {result}"));
            }
            if std::fs::read_to_string(home.join(".cys/feedback/bundle/x")).ok().as_deref() != Some("x") {
                bad.push(format!("limit-0: the folder was removed although nothing could be recorded first: {result}"));
            }
            let told = notes
                .iter()
                .filter(|n| n.contains("정리하지 않았습니다") && n.contains(&log.display().to_string()))
                .count();
            if told != 1 || notes.len() != 1 {
                bad.push(format!("limit-0: the note is not one 'not cleaned' line naming the record: {notes:?}"));
            }
            // second start without the limit (RULING-07 B): the record FILE exists - empty - so no later start deletes
            let record_after_first = std::fs::read(&log).ok();
            s1_fix2_put(&home.join(".cys/feedback/bundle/x"), "x");
            s1_fix2_put(&later(&home), "PLACED LATER");
            let mut notes2: Vec<String> = Vec::new();
            let out2 = remove_retired_feedback_dir_noting(&home, now + 99, &mut |m: &str| notes2.push(m.to_string()));
            s1_fix2_not_deleted("limit-0(second start)", &home, &log, &out2, &notes2, &mut bad);
            if !later_kept(&home) {
                bad.push(format!("limit-0: the second start deleted the later file (record after the first start: {record_after_first:?})"));
            }
            if std::fs::read(&log).ok() != record_after_first {
                bad.push("limit-0: the second start changed the record".to_string());
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        // ② the attempt line fits, the end line does not
        {
            let (root, home, _outside) = s1_fix2_m1_fixture("fix3-limit-attempt");
            let log = s1_fix2_log(&home);
            let attempt = s1_fix3_attempt_line(now, &home);
            let (result, notes) = s1_fix3_run_limited(&home, attempt.len() as u64);
            if !result.starts_with("Removed {") {
                bad.push(format!("limit-attempt: the first start did not remove: {result}"));
            }
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if text != attempt {
                bad.push(format!("limit-attempt: the record is not exactly the attempt line: {text:?}"));
            }
            if !notes.iter().any(|n| n.contains(&log.display().to_string())) {
                bad.push(format!("limit-attempt: no note names the record whose end could not be written: {notes:?}"));
            }
            s1_fix2_put(&home.join(".cys/feedback/bundle/x"), "x");
            s1_fix2_put(&later(&home), "PLACED LATER");
            let mut notes2: Vec<String> = Vec::new();
            let out2 = remove_retired_feedback_dir_noting(&home, now + 99, &mut |m: &str| notes2.push(m.to_string()));
            s1_fix2_not_deleted("limit-attempt(second start)", &home, &log, &out2, &notes2, &mut bad);
            if !later_kept(&home) {
                bad.push("limit-attempt: the second start deleted the later file".to_string());
            }
            if !notes2.iter().any(|n| n.contains("attempt")) {
                bad.push(format!("limit-attempt: the second start does not say an attempt has no end: {notes2:?}"));
            }
            if std::fs::read_to_string(&log).unwrap_or_default() != text {
                bad.push("limit-attempt: the second start changed the record".to_string());
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        // ③ stop points - the record each one leaves, then a later start with a file placed later
        for (tag, folder_gone, record_tail) in [
            ("stop-after-file-created", false, "empty"),
            ("stop-after-attempt", false, ""),
            ("stop-after-removal", true, ""),
            ("stop-after-end-line", true, "removed"),
        ] {
            let (root, home, _outside) = s1_fix2_m1_fixture(&format!("fix3-{tag}"));
            let log = s1_fix2_log(&home);
            let mut record = if record_tail == "empty" { String::new() } else { s1_fix3_attempt_line(now, &home) };
            if record_tail == "removed" {
                record.push_str(&format!(
                    "{now}\t{}\tremoved\t{}\tlink=false\tentries=1\tfiles=1\tbytes=1\tbundle\n",
                    env!("CARGO_PKG_VERSION"),
                    home.join(".cys/feedback").display()
                ));
            }
            std::fs::write(&log, &record).unwrap();
            if folder_gone {
                std::fs::remove_dir_all(home.join(".cys/feedback")).unwrap();
                s1_fix2_put(&home.join(".cys/feedback/bundle/x"), "x");
            }
            s1_fix2_put(&later(&home), "PLACED LATER");
            let mut notes: Vec<String> = Vec::new();
            let out = remove_retired_feedback_dir_noting(&home, now + 99, &mut |m: &str| notes.push(m.to_string()));
            s1_fix2_not_deleted(tag, &home, &log, &out, &notes, &mut bad);
            if !later_kept(&home) {
                bad.push(format!("{tag}: a later start deleted the later file"));
            }
            let says_attempt = notes.iter().any(|n| n.contains("attempt"));
            if says_attempt != (record_tail != "removed") {
                bad.push(format!("{tag}: the note does not say whether an attempt has no end: {notes:?}"));
            }
            if std::fs::read_to_string(&log).unwrap_or_default() != record {
                bad.push(format!("{tag}: a later start changed the record"));
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        // ④ the ordinary case: removed once, recorded as attempt line then removed line
        {
            let (root, home, _outside) = s1_fix2_m1_fixture("fix3-ordinary");
            let log = s1_fix2_log(&home);
            let mut notes: Vec<String> = Vec::new();
            let out = remove_retired_feedback_dir_noting(&home, now, &mut |m: &str| notes.push(m.to_string()));
            if !matches!(out, RetiredFeedbackCleanup::Removed { files: 1, bytes: 1, link: false, .. }) {
                bad.push(format!("ordinary: not removed: {out:?}"));
            }
            let want = format!(
                "{}{now}\t{}\tremoved\t{}\tlink=false\tentries=1\tfiles=1\tbytes=1\tbundle\n",
                s1_fix3_attempt_line(now, &home),
                env!("CARGO_PKG_VERSION"),
                home.join(".cys/feedback").display()
            );
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if text != want {
                bad.push(format!("ordinary: the record is not the attempt line then the removed line: {text:?}"));
            }
            s1_fix2_put(&home.join(".cys/feedback/bundle/x"), "x");
            s1_fix2_put(&later(&home), "PLACED LATER");
            let mut notes2: Vec<String> = Vec::new();
            let out2 = remove_retired_feedback_dir_noting(&home, now + 99, &mut |m: &str| notes2.push(m.to_string()));
            s1_fix2_not_deleted("ordinary(second start)", &home, &log, &out2, &notes2, &mut bad);
            if !later_kept(&home) {
                bad.push("ordinary: the second start deleted the later file".to_string());
            }
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(
            bad.is_empty(),
            "a failed or cut-short record let a later start delete, or a delete ran with no record first:\n{}",
            bad.join("\n")
        );
    }
}
