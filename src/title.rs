//! WP13-TABTITLE-0.14.53 (owner fix F1 · 2026-10-24) — the two pure decisions behind `cys set-title`.
//!
//! 【무엇을 고치는가】 오너 실측(OWNER-20261024-seven-fixes.txt §1): 좌석이 뜬 뒤 탭 제목을 바꾸는 명령이 없었다
//! (`cys --help` 에 title·rename 계열 부재 — 실측: `surface.rename` RPC 는 **이미 있고**(cysd/handlers.rs) GUI 의
//! 「이름 변경」 이 그것을 부른다. 없던 것은 CLI·권한 규칙·입력 정화다).
//!
//! 이 모듈은 **순수**다 — 데몬도, 소켓도, 프로세스도 모른다. 그래서 두 결정을 데몬 없이 검체로 돌릴 수 있고,
//! CLI(`cys set-title`)와 데몬(`surface.rename`)이 **같은 한 벌**을 쓴다(두 곳에 규칙을 베끼면 갈린다).

/// 제목으로 허용하는 최대 **문자** 수(바이트가 아니다 — 한글 제목이 ASCII 보다 먼저 잘리면 안 된다).
/// 60 은 pane 헤더가 `text-overflow: ellipsis` 로 줄이는 폭(style.css)의 몇 배이고 `cys list` 한 줄을 넘기지 않는 값이다.
pub const MAX_TITLE_CHARS: usize = 120;

/// 제목 정화 — (정화된 제목) 또는 (사람이 읽을 거절 사유).
///
/// ★제어문자는 **버린다**(거절하지 않는다): 붙여넣기에 끼어 오는 `\n`·`\t`·ESC 는 사용자의 의도가 아니고,
/// 그대로 저장하면 `cys list` 한 줄이 깨지고(줄바꿈) GUI 는 텍스트 노드에 그리며(무해) 터미널 출력에는 ESC 가
/// 그대로 흘러 **화면을 오염**시킨다. ⇒ 흘려보내지 않고 조용히 떨어뜨린 뒤, 남은 길이만 계약으로 본다.
/// ★길이는 **거절한다**(자르지 않는다): 조용히 자르면 사용자는 자기가 적은 제목이 아닌 것을 보게 되고,
/// 그 사실을 어디서도 알 수 없다(측정 가능한 거짓). 거절은 이유와 함께 즉시 보인다.
/// ★빈 문자열은 **정당하다** — 자동 제목(`{role}-{agent} · {folder}`)으로 되돌리는 기존 계약이다
/// (GUI 의 빈 이름 커밋과 같은 뜻 · `isAutoTitle`).
pub fn sanitize_title(raw: &str) -> Result<String, String> {
    let stripped: String = raw
        .chars()
        .filter(|c| !c.is_control() && *c != '\u{7f}')
        .collect();
    let trimmed = stripped.trim();
    let n = trimmed.chars().count();
    if n > MAX_TITLE_CHARS {
        return Err(format!(
            "title too long: {n} characters after dropping control characters (max {MAX_TITLE_CHARS})"
        ));
    }
    Ok(trimmed.to_string())
}

/// 이름 변경이 허용되는가 — `surface.close` 의 신원·소유 게이트를 그대로 비춘다(헤드 결정 Q1).
///
/// · `caller_surface == None` (익명): 통과. GUI·pane 밖 오너 터미널·데몬 내부 경로다. pane 안에서 도는 호출은
///   커널 peer pid 로 **항상** 자기 surface 로 해소되므로 익명을 위조할 수 없다(close 게이트의 논증 그대로).
/// · 자기 자신(`caller == target`): 통과.
/// · 이 데몬에서 역할이 `master` 인 호출자: 통과(부서장은 자기 데몬의 어느 좌석이든 고칠 수 있다).
/// · 그 밖: 거부. ⛔조용한 무동작이 아니라 이름 있는 오류다(호출자가 왜 안 됐는지 알아야 한다).
pub fn rename_allowed(caller_surface: Option<u64>, target: u64, caller_role: Option<&str>) -> bool {
    match caller_surface {
        None => true,
        Some(cs) if cs == target => true,
        Some(_) => caller_role == Some("master"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_dropped_not_refused() {
        assert_eq!(sanitize_title("a\nb\tc").unwrap(), "abc");
        assert_eq!(sanitize_title("x\u{1b}[31my").unwrap(), "x[31my"); // ESC 만 사라진다(나머지 글자는 사용자 것)
        assert_eq!(sanitize_title("\u{7f}del").unwrap(), "del");
        assert_eq!(sanitize_title("  spaced  ").unwrap(), "spaced");
    }

    #[test]
    fn an_empty_title_is_legal_it_means_back_to_the_automatic_one() {
        assert_eq!(sanitize_title("").unwrap(), "");
        assert_eq!(sanitize_title("   ").unwrap(), "");
        assert_eq!(sanitize_title("\n\t").unwrap(), "");
    }

    #[test]
    fn length_is_counted_in_characters_and_refused_never_truncated() {
        let ok: String = "가".repeat(MAX_TITLE_CHARS);
        assert_eq!(sanitize_title(&ok).unwrap().chars().count(), MAX_TITLE_CHARS);
        let long: String = "가".repeat(MAX_TITLE_CHARS + 1);
        let e = sanitize_title(&long).unwrap_err();
        assert!(e.contains("too long") && e.contains(&(MAX_TITLE_CHARS + 1).to_string()), "{e}");
        // ★자르지 않는다 — 거절이 성공으로 읽히면 사용자는 자기 제목이 아닌 것을 보게 된다.
        assert!(sanitize_title(&long).is_err());
        // 제어문자를 버린 **뒤** 길이를 센다: 제어문자로 부푼 입력은 통과한다.
        let padded: String = format!("{}{}", "\n".repeat(50), "가".repeat(MAX_TITLE_CHARS));
        assert!(sanitize_title(&padded).is_ok());
    }

    #[test]
    fn a_caller_may_rename_itself() {
        assert!(rename_allowed(Some(381), 381, Some("worker-9")));
        assert!(rename_allowed(Some(7), 7, None));
    }

    #[test]
    fn a_worker_may_not_rename_another_seat() {
        assert!(!rename_allowed(Some(375), 381, Some("worker-9")));
        assert!(!rename_allowed(Some(375), 381, None));
        assert!(!rename_allowed(Some(375), 381, Some("cso")));
        assert!(!rename_allowed(Some(375), 381, Some("reviewer-codex")));
    }

    #[test]
    fn master_may_rename_any_seat_of_this_daemon() {
        assert!(rename_allowed(Some(71), 381, Some("master")));
        assert!(rename_allowed(Some(71), 0, Some("master")));
    }

    #[test]
    fn an_unresolved_caller_passes_as_before() {
        // GUI(src-tauri) · pane 밖 오너 터미널 · 데몬 내부 경로 — close 게이트와 같은 판정.
        assert!(rename_allowed(None, 381, None));
        assert!(rename_allowed(None, 381, Some("worker-9")));
    }

    #[test]
    fn the_rule_is_not_a_role_name_prefix_match() {
        // "master" 로 시작하는 다른 역할이 생겨도 통과하지 않는다(정확 일치).
        assert!(!rename_allowed(Some(1), 2, Some("master-of-nothing")));
        assert!(!rename_allowed(Some(1), 2, Some("Master")));
    }
}
