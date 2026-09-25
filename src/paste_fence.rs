//! 괄호 붙여넣기 울타리(bracketed paste) 살균 — cys·cysd 공용 **단일 정의처**(0.14.42 · 설계 C D1).
//!
//! 【왜】 주입 경로는 본문을 `ESC[200~ … ESC[201~` 로 감싸 TUI 에 "이것은 붙여넣기다" 라고 알린다. 수신 TUI
//! (xterm 식 파서 · Node `emitKeys` 식 파서)와 데몬 자신의 계수기(`governance::pending_input_step`)는 모두
//! **첫 CLOSE 에서** 붙여넣기를 닫는다. 그래서 본문 안에 CLOSE(또는 그 8비트 C1 형 `U+009B 201~`)가 있으면
//! 봉투가 조기에 끝나고, 그 뒤 글자는 **키 입력**으로 해석된다 — 줄바꿈은 제출, ESC 는 취소가 된다.
//! 본문 끝이 미완성 이스케이프(`…ESC[1` 같은)이면 뒤에 붙는 CLOSE 의 ESC 가 그 시퀀스에 먹혀 봉투가 아예
//! 닫히지 않는다(Node `emitKeys` 는 CSI 숫자 뒤 다음 글자를 종결로 삼킨다 — 파서 지식 기반 추정).
//!
//! 【무엇을 지우나】 네 표지(OPEN·CLOSE·C1 OPEN·C1 CLOSE)와 **끝의 미완성 이스케이프**뿐이다. 본문 중간의
//! 다른 ESC·C0 는 범위 밖이다(울타리 무결성과 무관 — CLOSE 는 끝에만 온다).
//!
//! 【결과 성질】(전수 검체 `exhaustive_small_alphabet_properties`) 표지 0 · 끝 미완성 0 · 멱등 · 원문의
//! 부분수열. 빠른 경로: ESC 도 U+009B 도 없으면 `Cow::Borrowed`(할당 0 · 바이트 동일). 바뀐 것이 없어도
//! `Borrowed`(완결된 색 코드 등은 그대로).
//!
//! 【비용】 O(n) 두 단계. ① 단일 패스 스택 — 글자를 하나씩 넣고 넣은 글자가 `~` 일 때만 끝이 표지인지 본다
//! (넣기 전의 결과에는 표지가 없으므로 새 표지는 끝에서만 생긴다 — 한 번 검사로 충분). 표지는 ESC 나 선두
//! 바이트 0xC2 로 시작하므로 `truncate` 는 늘 글자 경계다(패닉 없음). ② 끝 미완성 이스케이프 절단 — 잘라 낸
//! 길이에 비례(접미를 잘라 표지가 새로 생기지 않는다). 종전 설계 초안의 고정점 `replace` 반복은 O(n²)
//! (12 KB 병리 입력 2002회 반복)이라 배달 락 안에서 쓸 수 없었다.
use std::borrow::Cow;

/// 붙여넣기 시작 표지(7비트).
pub const OPEN: &str = "\x1b[200~";
/// 붙여넣기 끝 표지(7비트).
pub const CLOSE: &str = "\x1b[201~";
/// 붙여넣기 시작 표지 — 8비트 C1 CSI(U+009B) 형.
pub const C1_OPEN: &str = "\u{9b}200~";
/// 붙여넣기 끝 표지 — 8비트 C1 CSI(U+009B) 형.
pub const C1_CLOSE: &str = "\u{9b}201~";

/// 울타리 본문 살균(위 모듈 문서 참조). 결과: 표지 0 · 끝 미완성 이스케이프 0 · 멱등 · 원문의 부분수열.
pub fn sanitize(body: &str) -> Cow<'_, str> {
    // 빠른 경로: 표지·미완성 이스케이프는 ESC 나 U+009B 로만 시작한다 — 둘 다 없으면 할당 0.
    if !body.contains(['\x1b', '\u{9b}']) {
        return Cow::Borrowed(body);
    }
    let mut out = String::with_capacity(body.len());
    let mut changed = false;
    // ① 단일 패스 스택 — out 에는 언제나 표지가 없다(불변식). 새 표지는 방금 넣은 글자로 끝나야
    //    하고 네 표지는 모두 `~` 로 끝나므로, `~` 를 넣었을 때만 끝을 본다.
    for ch in body.chars() {
        out.push(ch);
        if ch == '~' {
            for m in [OPEN, CLOSE, C1_OPEN, C1_CLOSE] {
                if out.ends_with(m) {
                    out.truncate(out.len() - m.len()); // 표지는 ESC·0xC2 로 시작 → 글자 경계
                    changed = true;
                    break;
                }
            }
        }
    }
    // ② 끝 미완성 이스케이프 절단 — 마지막 ESC/U+009B 부터 끝까지가 미완성이면 잘라 내고 반복한다.
    //    실파서는 ESC 를 만나면 진행 중 시퀀스를 버리고 새로 시작하므로 끝 상태는 마지막 ESC 의 꼬리만이
    //    정한다. 접미를 잘라 표지가 새로 생기는 일은 없다(①의 불변식은 부분 문자열 제거에 닫혀 있다).
    while let Some(p) = out.rfind(['\x1b', '\u{9b}']) {
        if !escape_incomplete(&out[p..]) {
            break;
        }
        out.truncate(p);
        changed = true;
    }
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(body)
    }
}

/// `t` 는 ESC 또는 U+009B 로 시작한다 — 이 꼬리가 **종결 전에 끝나는** 이스케이프인가.
///
/// 미완성으로 보는 것(뒤에 붙는 CLOSE 의 ESC·`[` 가 이 시퀀스에 먹힐 수 있는 모양):
///   · 홀로 선 ESC
///   · CSI(7비트 `ESC [` · 8비트 U+009B) 파라미터·중간 바이트(0x20–0x3F)만 있고 종결(0x40–0x7E) 없음
///   · Node `emitKeys` 의 `ESC [ [` 변형 — 다음 한 글자가 종결인데 그 글자가 없음
///   · SS3/SS2(`ESC O`·`ESC N`) 뒤 글자 없음
///   · ESC + 중간 바이트(0x20–0x2F)만 있고 종결(0x30–0x7E) 없음(`ESC (` 등)
/// 그 밖(두 글자 이스케이프 `ESC 7`·`ESC \` · 종결된 CSI · 비정형 글자로 끊긴 시퀀스)은 완결로 본다 —
/// 지우는 쪽이 아니라 보존하는 쪽이 기본이다(본문 손실 최소).
fn escape_incomplete(t: &str) -> bool {
    let mut it = t.chars();
    match it.next() {
        Some('\u{9b}') => csi_incomplete(it.as_str(), false),
        Some('\x1b') => match it.next() {
            None => true,
            Some('[') => csi_incomplete(it.as_str(), true),
            Some('O') | Some('N') => it.next().is_none(),
            Some(c) if ('\x20'..='\x2f').contains(&c) => it.all(|c| ('\x20'..='\x2f').contains(&c)),
            Some(_) => false,
        },
        _ => false,
    }
}

/// CSI 도입부 뒤 `rest` 가 종결 전에 끝나는가. `node_double_bracket` = 7비트 `ESC [` 형(Node 의 `ESC [ [`
/// 변형을 인정한다).
fn csi_incomplete(rest: &str, node_double_bracket: bool) -> bool {
    let mut it = rest.chars();
    if node_double_bracket && rest.starts_with('[') {
        it.next();
        return it.next().is_none();
    }
    for c in it {
        if ('\x20'..='\x3f').contains(&c) {
            continue; // 파라미터·중간 바이트
        }
        return false; // 종결(0x40–0x7E) 또는 비정형 글자 — 시퀀스가 끝났다
    }
    true
}

/// 소유 문자열판 [`sanitize`] — 바뀐 것이 없으면 **받은 `String` 을 그대로** 돌려준다(추가 할당·복사 0).
/// 데몬이 원장 선기록 전에 본문을 한 번 살균해 원장·주입 두 곳에 같은 값을 쓸 때 쓴다(설계 C D3).
pub fn sanitize_owned(body: String) -> String {
    if let Cow::Owned(s) = sanitize(&body) {
        return s;
    }
    body
}

/// 울타리 봉투 — `OPEN + sanitize(body) + CLOSE`. 표지가 없고 끝이 완결인 본문은 종전
/// `format!("\x1b[200~{body}\x1b[201~")` 과 바이트가 같다.
pub fn wrap(body: &str) -> String {
    let inner = sanitize(body);
    let mut out = String::with_capacity(OPEN.len() + inner.len() + CLOSE.len());
    out.push_str(OPEN);
    out.push_str(&inner);
    out.push_str(CLOSE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKERS: [&str; 4] = [OPEN, CLOSE, C1_OPEN, C1_CLOSE];

    fn has_marker(s: &str) -> bool {
        MARKERS.iter().any(|m| s.contains(m))
    }

    /// 끝 미완성 판정의 **검체 쪽 독립 구현**(생산 판정과 따로 적어 서로 검산한다).
    fn tail_incomplete_ref(s: &str) -> bool {
        let Some(p) = s.rfind(['\x1b', '\u{9b}']) else {
            return false;
        };
        let t: Vec<char> = s[p..].chars().collect();
        let csi = |rest: &[char], node: bool| -> bool {
            if node && rest.first() == Some(&'[') {
                return rest.len() < 2;
            }
            for &c in rest {
                if ('\x20'..='\x3f').contains(&c) {
                    continue;
                }
                return false;
            }
            true
        };
        match t[0] {
            '\u{9b}' => csi(&t[1..], false),
            _ => match t.get(1) {
                None => true,
                Some('[') => csi(&t[2..], true),
                Some('O') | Some('N') => t.len() < 3,
                Some(c) if ('\x20'..='\x2f').contains(c) => t[2..].iter().all(|c| ('\x20'..='\x2f').contains(c)),
                Some(_) => false,
            },
        }
    }

    fn is_subsequence(small: &str, big: &str) -> bool {
        let mut it = big.chars();
        small.chars().all(|c| it.any(|b| b == c))
    }

    /// ① 평문(한글·이모지·\n·\t·\r)과 완결 ESC 는 Borrowed 이고 바이트 동일.
    #[test]
    fn plain_and_complete_escapes_are_borrowed_identical() {
        for s in [
            "",
            "hello",
            "한글 본문 😀\n둘째 줄\t탭\r\n셋째",
            "\x1b[0m색\x1b[31m빨강\x1b[0m",
            "\x1b[1;2H",
            "끝이 ST\x1b\\",
            "끝 (B\x1b(B",
            "Node F1 \x1b[[A",
            "X\u{9b}Y",
        ] {
            let r = sanitize(s);
            assert!(matches!(r, Cow::Borrowed(_)), "Borrowed 여야 한다: {s:?}");
            assert_eq!(r, s);
            assert_eq!(wrap(s), format!("\x1b[200~{s}\x1b[201~"), "종전 봉투와 바이트 동일: {s:?}");
        }
    }

    /// ② 표지 4종 제거 · ③ 중첩 구성도 표지 0.
    #[test]
    fn markers_and_nested_constructions_removed() {
        assert_eq!(sanitize("a\x1b[201~b"), "ab");
        assert_eq!(sanitize("a\x1b[200~b"), "ab");
        assert_eq!(sanitize("a\u{9b}201~b"), "ab");
        assert_eq!(sanitize("a\u{9b}200~b"), "ab");
        for s in [
            "\x1b[20\x1b[201~1~",
            "\x1b\x1b[201~",
            "\u{9b}20\x1b[201~1~",
            "M1\x1b[201~M2\nM3",
            "\x1b[2\x1b[201~01~",
            "\x1b[\x1b[201~201~",
        ] {
            let r = sanitize(s);
            assert!(!has_marker(&r), "표지가 남았다: {s:?} → {r:?}");
            assert!(!has_marker(&wrap(s)[OPEN.len()..wrap(s).len() - CLOSE.len()]), "{s:?}");
        }
        assert_eq!(sanitize("M1\x1b[201~M2\nM3"), "M1M2\nM3");
    }

    /// ④ 끝 미완성 이스케이프 표 — 절단 대 보존.
    #[test]
    fn trailing_incomplete_escape_table() {
        for (s, want) in [
            ("x\x1b", "x"),
            ("x\x1b[20", "x"),
            ("x\x1b[1;", "x"),
            ("x\x1bO", "x"),
            ("x\x1bN", "x"),
            ("x\u{9b}2", "x"),
            ("x\x1b(", "x"),
            ("x\x1b[[", "x"),
            ("a\x1b[1\x1b[2", "a"),
            ("x\x1b[201", "x"),
        ] {
            assert_eq!(sanitize(s), want, "절단: {s:?}");
        }
        for s in ["x\x1b[0m", "x\x1b(B", "x\x1b[[A", "x\x1bOP", "x\x1b7", "x\x1b[1ä"] {
            assert_eq!(sanitize(s), s, "보존: {s:?}");
        }
    }

    /// ⑤ 알파벳 {ESC,'[','2','0','1','~','x',U+009B} 길이 6 이하 전수(299,593건):
    /// 표지 0 · 멱등 · 부분수열 · 끝 미완성 0 · 봉투 안 표지 0 · OPEN 은 [0] 에 1개 · CLOSE 는 끝에 1개.
    #[test]
    fn exhaustive_small_alphabet_properties() {
        let alpha = ['\x1b', '[', '2', '0', '1', '~', 'x', '\u{9b}'];
        let mut n = 0usize;
        let mut buf: Vec<usize> = Vec::new();
        for len in 0..=6usize {
            buf.clear();
            buf.resize(len, 0);
            loop {
                let s: String = buf.iter().map(|&i| alpha[i]).collect();
                n += 1;
                let r = sanitize(&s);
                assert!(!has_marker(&r), "표지: {s:?} → {r:?}");
                assert!(!tail_incomplete_ref(&r), "끝 미완성: {s:?} → {r:?}");
                assert_eq!(sanitize(&r), r, "멱등: {s:?}");
                assert!(is_subsequence(&r, &s), "부분수열: {s:?} → {r:?}");
                let w = wrap(&s);
                assert!(w.starts_with(OPEN) && w.ends_with(CLOSE), "{s:?}");
                assert_eq!(w.matches(OPEN).count(), 1, "OPEN 1개: {s:?}");
                assert_eq!(w.matches(CLOSE).count(), 1, "CLOSE 1개: {s:?}");
                assert_eq!(w.find(CLOSE), Some(w.len() - CLOSE.len()), "CLOSE 는 끝에만: {s:?}");
                assert!(!w.contains(C1_OPEN) && !w.contains(C1_CLOSE), "{s:?}");
                // 다음 조합
                let mut i = 0;
                loop {
                    if i == len {
                        break;
                    }
                    buf[i] += 1;
                    if buf[i] < alpha.len() {
                        break;
                    }
                    buf[i] = 0;
                    i += 1;
                }
                if i == len {
                    break;
                }
            }
        }
        assert_eq!(n, 299_593, "전수 건수");
    }

    /// `sanitize_owned` 는 `sanitize` 와 같은 값이고, 바뀐 것이 없으면 받은 버퍼를 그대로 돌려준다.
    #[test]
    fn sanitize_owned_matches_and_reuses_buffer() {
        let s = String::from("평문 본문");
        let ptr = s.as_ptr();
        let r = sanitize_owned(s);
        assert_eq!(r, "평문 본문");
        assert_eq!(r.as_ptr(), ptr, "무변경이면 같은 버퍼(복사 0)");
        assert_eq!(sanitize_owned("a\x1b[201~b".into()), "ab");
        assert_eq!(sanitize_owned("t\x1b[1".into()), "t");
    }

    /// ⑥ 성능: 병리 입력 약 1 MB 가 5 초 안(디버그)이고 결과 표지 0 · 표지 없는 1 MB 는 Borrowed.
    #[test]
    fn linear_time_on_pathological_input() {
        let k = 1_000_000 / 6;
        let mut s = String::with_capacity(k * 7);
        for _ in 0..k {
            s.push_str("\x1b[20");
        }
        s.push_str(CLOSE);
        for _ in 0..k {
            s.push_str("1~");
        }
        let t0 = std::time::Instant::now();
        let r = sanitize(&s);
        let took = t0.elapsed();
        assert!(!has_marker(&r), "표지 0");
        assert!(!tail_incomplete_ref(&r));
        assert!(took < std::time::Duration::from_secs(5), "O(n) 이어야 한다: {took:?} ({} B)", s.len());
        let plain = "가".repeat(350_000);
        assert!(matches!(sanitize(&plain), Cow::Borrowed(_)), "표지 없는 1 MB 는 할당 0");
    }
}
