//! todo **스캔 집합** 파리티 하네스의 Rust 측 덤퍼 — `cys::todo_scan`이 만드는 스캔 루트와
//! 발견 파일 목록을 그대로 표준출력에 낸다. 짝은
//! `cysjavis-pack/bin/tests/parity_todo_scan.py`(Python `javis_report.discover_todo_files`와
//! **같은 임시 트리**를 먹여 집합 비교)이고, CI 잡이 그 스크립트를 부른다.
//!
//! **왜 존재하는가**: 소비자 C1(Python 보고기)·C2(Rust 데몬)는 배제 *정책*은 같았는데
//! **스캔 집합**이 달랐다 — 데몬이 정본 위치 `pack/round`를 보지 않아, 데몬에 배선한 유령
//! 배제·선언 판정이 정본 todo에 한 번도 적용되지 않았다(설계 §14 S18). 정책 일치를 아무리
//! 리뷰해도 "무엇을 보는가"가 다르면 두 소비자는 다른 세계를 산다. 그 갈림을 잡는 기계다.
//!
//! **왜 cargo 타깃이 아니라 rustc 단독 컴파일인가**: `scripts/todo_decl_dump.rs`와 같은 이유다
//! (배포 바이너리 오염 회피 + 크레이트 의존 0 불변식의 기계 증명). `cys::todo_scan`이 std 밖의
//! 무언가에 의존하기 시작하면 이 컴파일이 먼저 깨져서 알려준다.
//!
//! 사용법(스크립트가 호출한다):
//! ```text
//!   rustc --edition 2021 -C debuginfo=0 scripts/todo_scan_dump.rs -o <bin>
//!   <bin> --pack <팩경로|-> --cwd <경로> [--cwd <경로> …]
//!       env    : CYS_TODO_DIRS 그대로 존중
//!       stdout : `root\t<경로>` 줄들 + `file\t<정규경로>` 줄들 (각각 정렬)
//! ```

#[path = "../src/todo_scan.rs"]
mod todo_scan;

use std::io::Write;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut pack: Option<PathBuf> = None;
    let mut cwds: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--pack" => {
                let v = next_arg(&args, &mut i);
                // `-` = 팩 경로 없음(종전 데몬 동작 재현용 — S18 전/후 대비에 쓴다).
                if v != "-" {
                    pack = Some(PathBuf::from(v));
                }
            }
            "--cwd" => {
                let v = next_arg(&args, &mut i);
                cwds.push(v);
            }
            other => die(&format!("알 수 없는 인자: {other}")),
        }
        i += 1;
    }

    let roots = todo_scan::scan_roots(
        pack.as_deref(),
        &cwds,
        std::env::var("CYS_TODO_DIRS").ok().as_deref(),
    );
    let files = todo_scan::discover(&roots);

    let mut out = String::new();
    for r in &roots {
        out.push_str(&format!("root\t{}\n", r.display()));
    }
    for f in &files {
        out.push_str(&format!("file\t{}\n", f.display()));
    }
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    if lock.write_all(out.as_bytes()).is_err() || lock.flush().is_err() {
        die("stdout 기록 실패");
    }
}

fn next_arg(args: &[String], i: &mut usize) -> String {
    *i += 1;
    match args.get(*i) {
        Some(v) => v.clone(),
        None => die(&format!("{} 뒤에 값이 없다", args[*i - 1])),
    }
}

fn die(msg: &str) -> ! {
    eprintln!("todo_scan_dump: {msg}");
    std::process::exit(2)
}
