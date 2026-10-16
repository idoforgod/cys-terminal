//! Shared current-screen shell vetoes used by CLI readiness and receiver input checks.
//! Screen shape is a veto only: it does not prove process identity, stdin ownership or liveness.
//! The broad prompt-tail predicate and narrower bare-shell predicate intentionally differ.

pub const TUI_RENDER_MARKS: &[&str] = &["for shortcuts"];
pub const TUI_FRAME_RUN_MIN: usize = 8;
pub const BARE_SHELL_DEATH_TAIL_LINES: usize = 5;

pub fn screen_tail_lines(screen: &str, n: usize) -> String {
    let lines: Vec<&str> = screen.lines().filter(|l| !l.trim().is_empty()).collect();
    lines
        .iter()
        .rev()
        .take(n)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn windows_shell_prompt_shape(t: &str) -> bool {
    let Some(body) = t.strip_suffix('>') else {
        return false;
    };
    // ① PowerShell: `PS <위치>>` — 파일시스템/레지스트리/Env 공급자 공통, 축약 `PS>` 포함.
    //    연속행 프롬프트 `PS C:\x>>` 도 body 가 `PS …>` 라 여기서 참이 된다(프롬프트 맞다).
    if body == "PS" || body.starts_with("PS ") {
        return true;
    }
    // ② cmd.exe: `<드라이브문자>:\<경로>>` — `C:\>` · `C:\Users\x>` · `D:\work\cys>`.
    //    콜론+역슬래시까지 요구해 "3 > 2 이므로 a > b>" 류 산문을 배제한다.
    let mut it = body.chars();
    matches!(
        (it.next(), it.next(), it.next()),
        (Some(d), Some(':'), Some('\\')) if d.is_ascii_alphabetic()
    )
}

pub fn screen_tail_is_shell_prompt_on(text: &str, windows: bool) -> bool {
    let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) else {
        return false; // 화면 비어 있음 — 판단 보류(시간 폴백 유지)
    };
    let t = last.trim_end();
    // zsh "...%" / bash·sh "...$" / root "#" / powerlevel10k·starship "❯" —
    // 끝문자 기준(프롬프트 커스텀의 공통 꼬리). 오탐 효과는 '대기 후 명시 Err'(안전측).
    // Windows(PowerShell `PS C:\…>` · cmd `C:\>`)는 `>` 로 끝나므로 형태 요건과 함께 추가한다
    // (T-D4 / F4-cys-boot-launch-06 — 이게 빠져 Windows 에서 marker 없는 어댑터의 시간 폴백이
    // 죽은 PowerShell 을 ready 로 선언하고 54KB 디렉티브를 셸 명령으로 제출했다).
    // 판정 축은 그대로다 — '마지막 비공백 줄의 끝문자' 규칙을 유지한 채 종결자만 늘렸다.
    t.ends_with('%')
        || t.ends_with('$')
        || t.ends_with('#')
        || t.ends_with('❯')
        || (windows && windows_shell_prompt_shape(t))
}

pub fn screen_has_frame_rule(text: &str) -> bool {
    text.lines().any(|line| {
        let mut run = 0usize;
        for c in line.chars() {
            // Box Drawing `U+2500..=U+257F` · Block Elements `U+2580..=U+259F`.
            if matches!(c as u32, 0x2500..=0x259F) {
                run += 1;
                if run >= TUI_FRAME_RUN_MIN {
                    return true;
                }
            } else {
                run = 0;
            }
        }
        false
    })
}

pub fn screen_has_tui_render_evidence(text: &str) -> bool {
    if screen_has_frame_rule(text) {
        return true;
    }
    let flat = crate::first_run_gates::flatten(text);
    TUI_RENDER_MARKS
        .iter()
        .any(|m| flat.contains(&crate::first_run_gates::flatten(m)))
}

pub fn screen_is_bare_shell_on(text: &str, windows: bool) -> bool {
    if !screen_tail_is_shell_prompt_on(text, windows) {
        return false;
    }
    // ★사망 문면(다중 방어 ②) — 꼬리에 `command not found` 류가 보이면 렌더 잔상과 무관하게
    //   맨 셸이다. 판정 방향은 '주입 억제' 라 넓혀도 오살 위험이 없다.
    let tail = screen_tail_lines(text, BARE_SHELL_DEATH_TAIL_LINES);
    if screen_shows_launch_failure(&crate::first_run_gates::flatten(&tail)) {
        return true;
    }
    !screen_has_tui_render_evidence(text)
}

pub fn screen_shows_launch_failure(flat: &str) -> bool {
    // Unix: sh/zsh/bash "command not found" / 직접 실행 시 "No such file or directory" / "not found in PATH"
    flat.contains("commandnotfound")
        || flat.contains("notfoundinPATH")
        || flat.contains("Nosuchfileordirectory")
        // Windows PowerShell: "... is not recognized as the name of a cmdlet, function, ..."
        || flat.contains("isnotrecognizedasthenameofacmdlet")
        // Windows cmd.exe: "... is not recognized as an internal or external command, ..."
        || flat.contains("isnotrecognizedasaninternalorexternalcommand")
}
