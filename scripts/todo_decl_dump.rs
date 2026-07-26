//! 2언어 파리티 하네스의 **Rust 측 덤퍼** — 입력 파일들을 `cys::todo_decl`로 판정해
//! 정규화된 TSV 한 줄씩 표준출력에 낸다. 짝은 `cysjavis-pack/bin/tests/parity_todo_decl.py`
//! (Python 측 판정 + 두 출력의 기계 비교)이고, CI 잡이 그 스크립트를 부른다.
//!
//! **왜 cargo 타깃이 아니라 rustc 단독 컴파일인가**
//!   ① `src/bin/*.rs`에 두면 cargo가 자동으로 **배포 바이너리 타깃**으로 잡는다(릴리스 자산에
//!      진단 도구가 섞인다). `examples/`에 두면 이번엔 루트 lib 전체(rusqlite bundled·tokio
//!      …)를 링크해야 해서 파리티 한 번에 풀 빌드가 붙는다 — 브랜치 CI에 얹을 비용이 아니다.
//!   ② 이 파서는 **크레이트 의존이 0**이도록 설계됐다(데몬 워치독 틱 경로). rustc 단독 컴파일이
//!      성공한다는 사실 자체가 그 불변식의 기계 증명이다. 파서가 다른 모듈에 의존하기 시작하면
//!      이 컴파일이 먼저 깨져서 알려준다.
//!
//! 사용법(스크립트가 호출한다 — 손으로 부를 일은 거의 없다):
//! ```text
//!   rustc --edition 2021 -C debuginfo=0 scripts/todo_decl_dump.rs -o <bin>
//!   <bin> --codes                       # 진단 코드 목록(줄당 1개) 출력 후 종료
//!   <bin> --my-scope pack --scopes pack,pack-dept-dept-1 < cases.tsv
//!       stdin  : `<name>\t<path>` 줄 목록
//!       stdout : `<name>\t<verdict>\t<diag_code>\t<owner>\t<scope>\t<status>\t<legacy>`
//!                (미선언이면 owner/scope/status = `-`, 유효 선언이면 diag_code = `-`)
//! ```
//! 이름·경로에 탭이 들어오면 필드가 밀리므로 거부한다(조용한 오정렬 금지).

#[path = "../src/todo_decl.rs"]
mod todo_decl;

use std::io::{Read, Write};

/// 값 부재 표기. 빈 문자열을 쓰면 TSV에서 "필드가 없음"과 구분되지 않는다.
const NONE: &str = "-";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--codes") {
        // 양 언어가 **같은 코드 집합**을 갖는지 스크립트가 대조한다(ADR-4 C-2).
        let mut out = String::new();
        for c in todo_decl::diag::ALL {
            out.push_str(c);
            out.push('\n');
        }
        print!("{out}");
        return;
    }

    let mut my_scope = String::new();
    let mut scopes: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--my-scope" => {
                my_scope = next_arg(&args, &mut i);
            }
            "--scopes" => {
                let raw = next_arg(&args, &mut i);
                scopes = raw
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            other => die(&format!("알 수 없는 인자: {other}")),
        }
        i += 1;
    }
    if my_scope.is_empty() {
        die("--my-scope 는 필수다");
    }

    let mut stdin = String::new();
    if std::io::stdin().read_to_string(&mut stdin).is_err() {
        die("stdin 을 읽을 수 없다");
    }

    let scope_exists = |s: &str| scopes.iter().any(|e| e == s);
    let mut out = String::new();
    for line in stdin.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let (name, path) = match (parts.next(), parts.next(), parts.next()) {
            (Some(n), Some(p), None) => (n, p),
            // `die`가 `!`를 반환하므로 match 팔의 타입이 맞는다(unreachable 보조 불필요).
            _ => die(&format!("입력 줄 형식 위반(탭 2필드여야 한다): {line:?}")),
        };

        // 파일 읽기 계약도 파리티 대상이다 — 선두 HEAD_BYTES **바이트** → lossy 디코드.
        // 읽기 실패는 빈 머리말로 흘린다(Python `read_head` 의 fail-open 과 동일 의미).
        let raw = std::fs::read(path).unwrap_or_default();
        let head = todo_decl::head_from_bytes(&raw);
        let parsed = todo_decl::parse(&head);
        let verdict = todo_decl::classify(parsed.as_ref().ok(), &my_scope, &scope_exists);

        let (code, owner, scope, status, legacy) = match &parsed {
            Ok(d) => (
                NONE,
                d.owner.as_str(),
                d.scope.as_str(),
                d.status.as_str(),
                if d.legacy { "true" } else { "false" },
            ),
            Err(e) => (e.code, NONE, NONE, NONE, "false"),
        };
        out.push_str(&format!(
            "{name}\t{}\t{code}\t{owner}\t{scope}\t{status}\t{legacy}\n",
            verdict.as_str()
        ));
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
    eprintln!("todo_decl_dump: {msg}");
    std::process::exit(2)
}
