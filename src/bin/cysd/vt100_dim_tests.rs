//! ★(0.14.47 · 계측·관측 보강) 벤더 vt100 사본의 **흐림(SGR 2) 패치** 지킴이 시험.
//!
//! 패치가 하는 일: SGR 2 를 칸에 기억하고(22 · 0 으로 내림) `Cell::dim()` 으로 읽게 한다. **칸 비교와 다시 그리기 출력
//! (`contents_formatted` · `contents_diff`)에는 넣지 않는다** — attach 첫 덩어리의 바이트가 한 바이트도 바뀌면 안 된다.
//! 재적용 경로 = `vendor/vt100/CYS-PATCHES.md`. 상류 갱신으로 사본을 덮어 패치가 빠지면 이 파일이 컴파일 단계(`dim()` 없음) 또는
//! 단언에서 죽는다.
//!
//! 고정값(`*_PRE`)은 **패치를 넣기 전**의 벤더 사본(태그 v0.14.46)에 같은 바이트를 먹여 떠 둔 것이다(저장소 밖 재생기 ·
//! 증거 `_evidence/dept5-M1-item1-20261008/INSTR-impl/golden/`). 패치 뒤에 다시 떠서 바꾸지 않는다 — 바꾸면 이 시험은 아무것도 지키지 않는다.

fn parser(rows: u16, cols: u16, bytes: &[u8]) -> vt100::Parser {
    let mut p = vt100::Parser::new(rows, cols, 0);
    p.process(bytes);
    p
}

/// 속성 행렬 — 굵게·기울임·밑줄·반전·전경(색인·256·RGB)·배경(색인·256) 각각 + 흐림 단독 · 굵게와 섞임 · 22 가 둘을 함께 내림 · 0 으로 내림.
const MATRIX: &[u8] = b"\x1b[1mB\x1b[22m\x1b[3mI\x1b[23m\x1b[4mU\x1b[24m\x1b[7mV\x1b[27m\x1b[31mF\x1b[39m\x1b[42mG\x1b[49m\x1b[38;5;246mX\x1b[39m\x1b[38;2;1;2;3mR\x1b[39m\x1b[48;5;16mK\x1b[49m plain\r\n\x1b[2mdim only\x1b[22m after\r\n\x1b[1m\x1b[2mBD\x1b[22mx\x1b[2m\x1b[1mDB\x1b[22my\r\n\x1b[2mD0\x1b[0mz\x1b[1;2mJ\x1b[22;3mK\x1b[0m\r\n\x1b[2m\x1b[31mdimred\x1b[39m\x1b[22m \x1b[2;4mdu\x1b[24mdd\x1b[22m.";
/// 패치 전 `contents_formatted()`(8×60).
const MATRIX_FORMATTED_PRE: &[u8] = b"\x1b[?25h\x1b[m\x1b[H\x1b[J\x1b[1mB\x1b[22;3mI\x1b[23;4mU\x1b[24;7mV\x1b[31;27mF\x1b[39;42mG\x1b[38;5;246;49mX\x1b[38;2;1;2;3mR\x1b[39;48;5;16mK\x1b[m plain\r\ndim only after\r\n\x1b[1mBD\x1b[mx\x1b[1mDB\x1b[my\r\nD0z\x1b[1mJ\x1b[22;3mK\r\n\x1b[31;23mdimred\x1b[m \x1b[4mdu\x1b[mdd.";
const MATRIX_CONTENTS_PRE: &str = "BIUVFGXRK plain\ndim only after\nBDxDBy\nD0zJK\ndimred dudd.";

/// claude 2.1.294 가 실제로 낸 순서 **꼴**을 본뜬 손 검체(시험 자리 TESTSEAT6·7 의 원시 바이트 — 원문은 저장소에 넣지 않는다 · 판정 T16 ④).
/// 본뜬 걸음: ⓐ부트 머리(굵게 → SGR 22 → 색 246) ⓑ`d1-slash-mo` 의 명령 목록 행(색 246 과 굵게가 번갈아 · 굵게마다 22)
/// ⓒ`r1`·`c-a-4-bs` 의 안내문 복귀(반전 한 칸 `T` + SGR 2 … 22) ⓓ`b1-up`(윗 행의 흐림 꼬리표 ` History 2/2 ` + 기본 표현 글 + 반전 커서 칸)
/// ⓔ`b1-home`(첫 글자를 반전으로 다시 그림). 원문에는 굵게와 흐림이 **한 22 로 함께** 내려가는 자리가 없었다(0건) — 그 꼴은 `MATRIX` 가 따로 짠다.
const CLAUDE_SHAPED: &[u8] = b"\x1b[38;5;174m \x1b[48;5;16m\xe2\x96\x9b\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x9b\xe2\x96\x88\x1b[12G\x1b[39m\x1b[49m\x1b[1mClaude\x1b[19GCode\x1b[24G\x1b[22m\x1b[38;5;246mv2.1.294\x1b[39m\r\r\n\x1b[38;5;246m \x1b[5G/\x1b[39m\x1b[1mmo\x1b[22m\x1b[38;5;246mbile\x1b[33GShow\x1b[39m\r\r\n\x1b[4;1H\xe2\x9d\xaf\xc2\xa0\x1b[7mT\x1b[27m\x1b[2mry \"fix lint errors\"\r\x1b[2C\x1b[2B\x1b[22m\x1b[38;5;246m? for shortcuts\x1b[39m\x1b[3;4H \x1b[2mHistory 2/2\x1b[22m \x1b[4;3Hsecond\x1b[10Gbeta\x1b[15Gentry\x1b[7m \x1b[27m\x1b[K\x1b[4;3H\x1b[7ms\x1b[4G\x1b[27m\x1b[4;3H";
const CLAUDE_FORMATTED_PRE: &[u8] = b"\x1b[?25h\x1b[m\x1b[H\x1b[J\x1b[38;5;174m \x1b[48;5;16m\xe2\x96\x9b\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x9b\xe2\x96\x88\x1b[4C\x1b[39;49;1mClaude\x1b[CCode\x1b[C\x1b[38;5;246;22mv2.1.294\r\n \x1b[3C/\x1b[39;1mmo\x1b[38;5;246;22mbile\x1b[21CShow\x1b[3;4H\x1b[m History 2/2 \r\n\xe2\x9d\xaf\xc2\xa0\x1b[7ms\x1b[mecondibetanentry\x1b[7m \x1b[6;3H\x1b[38;5;246;27m? for shortcuts\x1b[4;3H\x1b[m";
const CLAUDE_CONTENTS_PRE: &str = " ▛███▛█    Claude Code v2.1.294\n    /mobile                     Show\n    History 2/2 \n❯\u{a0}secondibetanentry \n\n  ? for shortcuts";
const CLAUDE_CURSOR_PRE: (u16, u16) = (3, 2);

/// `bytes` 에서 SGR 의 흐림 매개변수(2)만 뺀 쌍둥이 — `ESC[2m` → 없음 · `ESC[1;2m` → `ESC[1m` · `ESC[2;4m` → `ESC[4m`. 색 지정(38;5;n · 38;2;r;g;b · 48;…)의 2 는 건드리지 않는다.
fn strip_dim(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b';') {
                j += 1;
            }
            if bytes.get(j) == Some(&b'm') {
                let params: Vec<&str> = std::str::from_utf8(&bytes[i + 2..j]).unwrap().split(';').collect();
                let mut kept: Vec<&str> = Vec::new();
                let mut k = 0;
                while k < params.len() {
                    match params[k] {
                        "38" | "48" => {
                            let n = if params.get(k + 1) == Some(&"5") { 3 } else { 5 };
                            kept.extend(&params[k..(k + n).min(params.len())]);
                            k += n;
                        }
                        "2" => k += 1,
                        other => {
                            kept.push(other);
                            k += 1;
                        }
                    }
                }
                if !kept.is_empty() {
                    out.extend_from_slice(format!("\x1b[{}m", kept.join(";")).as_bytes());
                }
                i = j + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// 시험 1 — SGR 2 를 먹인 칸이 흐림으로 읽힌다(패치 소실 지킴이).
#[test]
fn vt100_dim_patch_sgr2_cell_reads_dim() {
    let p = parser(4, 20, b"\x1b[2mab\x1b[22mc");
    let s = p.screen();
    let lost = "vendor/vt100 흐림 패치 소실? — 재적용 경로는 vendor/vt100/CYS-PATCHES.md";
    assert!(s.cell(0, 0).expect("칸").dim(), "SGR 2 뒤 첫 칸이 흐림이 아니다 — {lost}");
    assert!(s.cell(0, 1).expect("칸").dim(), "SGR 2 뒤 둘째 칸이 흐림이 아니다 — {lost}");
    assert!(!s.cell(0, 2).expect("칸").dim(), "SGR 22 뒤 칸이 여전히 흐림이다 — {lost}");
    assert!(!s.cell(0, 3).expect("칸").dim(), "쓰지 않은 칸이 흐림이다");
    assert_eq!(s.contents(), "abc");
}

/// 시험 2 — 22 는 굵게와 흐림을 **함께** 내리고, 0 도 내린다. 굵게만 켠 칸은 흐림이 아니고 흐림만 켠 칸은 굵게가 아니다.
#[test]
fn vt100_dim_sgr22_clears_dim_and_bold() {
    let p = parser(2, 20, b"\x1b[1;2mA\x1b[22mB\x1b[2mC\x1b[0mD\x1b[1mE\x1b[22mF\x1b[2m\x1b[1mG\x1b[22mH");
    let s = p.screen();
    let at = |c: u16| {
        let x = s.cell(0, c).expect("칸");
        (x.bold(), x.dim())
    };
    assert_eq!(at(0), (true, true), "A: 굵게+흐림");
    assert_eq!(at(1), (false, false), "B: 22 가 둘 다 내린다");
    assert_eq!(at(2), (false, true), "C: 흐림만");
    assert_eq!(at(3), (false, false), "D: 0 이 흐림을 내린다");
    assert_eq!(at(4), (true, false), "E: 굵게만(흐림 아님)");
    assert_eq!(at(5), (false, false), "F: 22 가 굵게를 종전대로 내린다");
    assert_eq!(at(6), (true, true), "G: 2 뒤 1");
    assert_eq!(at(7), (false, false), "H");
}

/// 시험 3 — 다시 그리기 출력에 흐림이 실리지 않는다: ⓐ패치 전 고정값과 바이트가 같다 ⓑ흐림만 뺀 쌍둥이 화면과 출력·칸 비교가 같다.
#[test]
fn vt100_dim_not_in_formatted_output() {
    let p = parser(8, 60, MATRIX);
    assert_eq!(p.screen().contents(), MATRIX_CONTENTS_PRE);
    assert_eq!(
        p.screen().contents_formatted(),
        MATRIX_FORMATTED_PRE,
        "contents_formatted 바이트가 패치 전 고정값과 다르다 — attach 첫 덩어리가 바뀐다"
    );
    let twin = parser(8, 60, &strip_dim(MATRIX));
    assert_eq!(twin.screen().contents_formatted(), MATRIX_FORMATTED_PRE, "흐림을 뺀 쌍둥이도 같은 바이트여야 한다(검체 자체 점검)");
    let mut dim_cells = 0;
    for r in 0..8u16 {
        for c in 0..60u16 {
            let (a, b) = (p.screen().cell(r, c).expect("칸"), twin.screen().cell(r, c).expect("칸"));
            assert!(a == b, "({r},{c}) 흐림만 다른 칸이 비교에서 달랐다 — 칸 비교는 흐림을 보지 않는다");
            assert!(!b.dim(), "쌍둥이에 흐림이 남았다(strip_dim 검체 오류)");
            dim_cells += usize::from(a.dim());
        }
    }
    assert!(dim_cells >= 20, "검체에 흐림 칸이 충분히 있어야 이 시험이 뜻이 있다: {dim_cells}");
    // 흐림만 다른 두 화면의 차분 = 같은 화면끼리의 차분.
    let same = parser(8, 60, MATRIX);
    assert_eq!(p.screen().contents_diff(twin.screen()), p.screen().contents_diff(same.screen()));
    assert_eq!(twin.screen().contents_diff(p.screen()), twin.screen().contents_diff(parser(8, 60, &strip_dim(MATRIX)).screen()));
}

/// 시험 3 보강(codex I2) — 칸 비교와 차분은 흐림 **말고는** 종전대로 전부 본다: 굵게·기울임·밑줄·반전·전경(색인·256·RGB)·배경(색인·256) 각각.
/// 변이 「흐림 말고 다른 속성 하나까지 가림」은 이 시험(과 시험 3 의 고정값)을 죽인다.
#[test]
fn vt100_cell_compare_still_sees_every_other_attr() {
    let plain = parser(2, 10, b"x");
    let base_diff = plain.screen().contents_diff(parser(2, 10, b"x").screen());
    for (name, sgr) in [
        ("굵게", "1"),
        ("기울임", "3"),
        ("밑줄", "4"),
        ("반전", "7"),
        ("전경 색인", "31"),
        ("전경 256", "38;5;246"),
        ("전경 RGB", "38;2;1;2;3"),
        ("배경 색인", "42"),
        ("배경 256", "48;5;16"),
    ] {
        let other = parser(2, 10, format!("\x1b[{sgr}mx").as_bytes());
        let (a, b) = (plain.screen().cell(0, 0).expect("칸"), other.screen().cell(0, 0).expect("칸"));
        assert!(a != b, "{name}: 칸 비교가 이 속성의 차이를 보지 못한다");
        assert_ne!(other.screen().contents_diff(plain.screen()), base_diff, "{name}: 차분이 이 속성의 차이를 내지 않는다");
        assert_ne!(other.screen().contents_formatted(), plain.screen().contents_formatted(), "{name}: 다시 그리기 출력에 이 속성이 없다");
        // 같은 속성 위에 흐림만 얹은 칸은 그 칸과 같다.
        let with_dim = parser(2, 10, format!("\x1b[{sgr};2mx").as_bytes());
        let c = with_dim.screen().cell(0, 0).expect("칸");
        assert!(c.dim(), "{name}+흐림: 흐림이 읽히지 않는다");
        assert!(b == c, "{name}: 흐림만 얹은 칸이 비교에서 달랐다");
        assert_eq!(with_dim.screen().contents_formatted(), other.screen().contents_formatted(), "{name}: 흐림을 얹자 다시 그리기 바이트가 달라졌다");
    }
    // 흐림만 다른 칸 = 같다.
    let dim_only = parser(2, 10, b"\x1b[2mx");
    assert!(plain.screen().cell(0, 0) == dim_only.screen().cell(0, 0));
    assert_eq!(dim_only.screen().contents_diff(plain.screen()), base_diff);
}

/// 시험 4 — claude 가 낸 순서 꼴의 바이트를 먹인 화면(글자·커서·다시 그리기 바이트)이 패치 전 고정값과 같다. 흐림은 칸에서만 읽힌다.
#[test]
fn vt100_replay_claude_shaped_bytes_screen_unchanged() {
    let p = parser(8, 60, CLAUDE_SHAPED);
    let s = p.screen();
    assert_eq!(s.contents(), CLAUDE_CONTENTS_PRE, "화면 글자가 패치 전과 다르다");
    assert_eq!(s.cursor_position(), CLAUDE_CURSOR_PRE, "커서가 패치 전과 다르다");
    assert_eq!(s.contents_formatted(), CLAUDE_FORMATTED_PRE, "다시 그리기 바이트가 패치 전과 다르다");
    // 양성 대조 — 검체가 정말 흐림 칸을 만든다(윗 행 꼬리표 ` History 2/2 ` 의 H = 3행(0 기반 2) 4열).
    let h = s.cell(2, 4).expect("칸");
    assert_eq!(h.contents(), "H");
    assert!(h.dim(), "꼬리표가 흐림으로 읽히지 않는다");
    // 굵게로 그린 머리 글자는 흐림이 아니고, 22 뒤의 색 246 글자는 굵게도 흐림도 아니다.
    let (cl, ver) = (s.cell(0, 11).expect("칸"), s.cell(0, 23).expect("칸"));
    assert_eq!((cl.contents(), cl.bold(), cl.dim()), ("C".to_string(), true, false));
    assert_eq!((ver.contents(), ver.bold(), ver.dim()), ("v".to_string(), false, false));
}
