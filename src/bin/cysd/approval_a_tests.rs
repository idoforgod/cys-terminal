//! ★(0.14.44 · WP-A) 승인(approval) 수정 묶음의 RPC 수준 시험 — A2(미승인 사유) · A3(데몬 묶음·폴더 건너뛰기) · A4(command_text).
//!
//! 승인 저장소는 임시 폴더로 옮긴다(`approval::tests::with_store_root`) — 실제 `~/.cys` 는 건드리지 않는다.
#![cfg(test)]

use crate::approval::tests::with_store_root;
use crate::handlers::{dispatch, Reply};
use crate::state::Daemon;
use cys::Request;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) static A_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) struct Fixture {
    pub daemon: Arc<Daemon>,
    pub dir: std::path::PathBuf,
    pub master_pid: u32,
    _store: crate::approval::tests::StoreRootGuard,
}

pub(crate) fn fixture(tag: &str, dept: bool, pid: u32) -> Fixture {
    let daemon = crate::team_gate_tests::tmp_daemon(tag, dept);
    let dir = std::env::temp_dir().join(format!("cys-a-{tag}-{}-{pid}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let store = with_store_root(&dir);
    let sid = crate::team_gate_tests::seat(&daemon, "master", pid);
    daemon.roles.lock().unwrap().insert("master".into(), sid);
    *daemon.master_claimed_at.lock().unwrap() = Some(crate::state::now_epoch() - 120.0);
    Fixture { daemon, dir, master_pid: pid, _store: store }
}

pub(crate) fn rpc(d: &Arc<Daemon>, pid: Option<u32>, method: &str, params: Value) -> Value {
    let req = Request { id: json!(1), method: method.into(), params };
    match dispatch(d, req, pid) {
        Reply::Single(v) => v,
        _ => panic!("단일 응답이어야 한다"),
    }
}

pub(crate) fn sign(f: &Fixture, params: Value) -> Value {
    rpc(&f.daemon, Some(f.master_pid), "approval.sign", params)
}

pub(crate) fn check(f: &Fixture, command: &str, cwd: &str, require_ttl: bool) -> Value {
    rpc(
        &f.daemon,
        Some(f.master_pid),
        "approval.check",
        json!({"command": command, "cwd": cwd, "require_ttl": require_ttl}),
    )
}

fn detail_code(r: &Value) -> String {
    assert_eq!(r["ok"], json!(true), "RPC 실패: {r}");
    assert_eq!(r["result"]["approved"], json!(false), "승인돼 버렸다: {r}");
    r["result"]["detail"]["code"].as_str().unwrap_or("(없음)").to_string()
}

// ── A2: 여섯 코드 ────────────────────────────────────────────────────────

#[test]
fn a2_six_codes_each_reported() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a2-six", false, 980_001);
    // no_record
    let r = check(&f, "git push origin main", "/a/other", false);
    assert_eq!(detail_code(&r), "no_record");
    assert_eq!(r["result"]["reason"], json!(null), "reason 의 뜻은 종전 그대로(null)");
    // bad_quote
    let r = check(&f, "git push 'oops", "/a/other", false);
    assert_eq!(detail_code(&r), "bad_quote");
    // cwd_mismatch (무기한 · 대상 밖 명령 → 종전 규칙)
    let s = sign(&f, json!({"command_prefix": ["git", "push"], "cwd": "/a/hq"}));
    assert_eq!(s["ok"], json!(true), "{s}");
    let r = check(&f, "git push origin main", "/a/other", false);
    assert_eq!(detail_code(&r), "cwd_mismatch");
    let d = &r["result"]["detail"];
    assert_eq!(d["signed_cwd"], json!(["/a/hq"]));
    assert_eq!(d["requested_cwd"], json!("/a/other"));
    assert!(d["record_id"].as_str().unwrap_or("").starts_with("ap-"));
    // 서명값·비밀키는 싣지 않는다.
    assert!(!d.to_string().contains("signature"));
    // ttl_required: 같은 폴더에서 무기한 레코드를 --require-ttl 로 확인
    let r = check(&f, "git push origin main", "/a/hq", true);
    assert_eq!(detail_code(&r), "ttl_required");
    // expired: TTL 레코드를 만든 뒤 만료로 되돌려 다시 서명
    let s = sign(&f, json!({"command_prefix": ["echo", "hi"], "cwd": "/a/hq", "ttl_secs": 3600}));
    assert_eq!(s["ok"], json!(true), "{s}");
    {
        let secret = crate::approval::signing_secret().expect("secret");
        let mut recs = crate::approval::load_records();
        for r in recs.iter_mut() {
            if r.command_prefix.first().map(|t| t.as_str()) == Some("echo") {
                r.expires_at = Some(crate::state::now_epoch() - 5.0);
                r.sign(&secret);
            }
        }
        crate::approval::save_records(&recs).expect("save");
    }
    let r = check(&f, "echo hi", "/a/hq", false);
    assert_eq!(detail_code(&r), "expired");
    assert!(r["result"]["detail"]["expired_at"].as_f64().is_some());
}

/// 깨진 저장소(두 파일 각각) · 서명이 틀린 레코드 · 빈 접두 레코드 — 패닉 없이 미승인(종료코드 2 에 해당).
#[test]
fn a2_corrupt_inputs_never_panic_and_never_approve() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a2-corrupt", false, 980_002);
    let s = sign(&f, json!({"command_prefix": ["git", "push"], "cwd": "/a/hq"}));
    assert_eq!(s["ok"], json!(true), "{s}");
    // 서명이 틀린 레코드(폴더를 바꿔치기) + 빈 접두 레코드를 파일에 직접 끼운다.
    {
        let mut recs = crate::approval::load_records();
        let mut forged = recs[0].clone();
        forged.id = "ap-forged".into();
        forged.cwd = Some("/a/other".into()); // 서명은 그대로 → 불일치
        let mut empty = recs[0].clone();
        empty.id = "ap-empty".into();
        empty.command_prefix = vec![];
        recs.push(forged);
        recs.push(empty);
        crate::approval::save_records(&recs).expect("save");
    }
    let r = check(&f, "git push origin main", "/a/other", false);
    assert_eq!(r["result"]["approved"], json!(false), "{r}");
    assert_eq!(detail_code(&r), "cwd_mismatch", "유효한 레코드(/a/hq)만 설명한다");
    assert_eq!(r["result"]["detail"]["signed_cwd"], json!(["/a/hq"]));
    // 일반 저장소 파일 깨짐 → 판정 불가(reason) · 승인 아님.
    let main_path = f.dir.join(".cys").join("approvals.json");
    std::fs::write(&main_path, b"{ not json").expect("write");
    let r = check(&f, "git push origin main", "/a/hq", false);
    assert_eq!(r["result"]["approved"], json!(false), "{r}");
    assert!(r["result"]["reason"].as_str().is_some(), "깨진 저장소는 reason 으로 말한다: {r}");
    assert!(r["result"].get("detail").is_none(), "저장소를 못 읽었으면 detail 이 없다: {r}");
    // 시간 한정 파일 깨짐도 같다.
    std::fs::write(&main_path, b"{\"records\":[]}").expect("write");
    let ttl_path = f.dir.join(".cys").join("approvals-ttl.json");
    std::fs::write(&ttl_path, b"][").expect("write");
    let r = check(&f, "git push origin main", "/a/hq", false);
    assert_eq!(r["result"]["approved"], json!(false), "{r}");
    assert!(r["result"]["reason"].as_str().is_some(), "{r}");
}

// ── A4: command_text ─────────────────────────────────────────────────────

/// 파이썬 `shlex.quote` 와 같은 규칙(게이트 훅이 공백이 든 인자에 쓴다 · `role-capability-gate.sh:2364-2369`).
fn shlex_quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg.chars().all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c));
    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\"'\"'"))
    }
}

fn sign_text(f: &Fixture, text: &str, cwd: &str, ttl: Option<u64>) -> Value {
    let legacy: Vec<&str> = text.split_whitespace().collect();
    sign(
        f,
        json!({"command_prefix": legacy, "command_text": text, "cwd": cwd, "ttl_secs": ttl}),
    )
}

#[test]
fn a4_quoted_command_signed_and_checked_with_the_same_text() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a4-quote", false, 980_010);
    // R5-11 의 꼴: 따옴표가 든 정확 명령을 그대로 서명하고 그대로 확인 → 통과.
    for (signed, checked) in [
        (r#"cys close-surface "surface:5""#, r#"cys close-surface "surface:5""#),
        (r#"cys close-surface "surface:5""#, "cys close-surface surface:5"),
        ("cys close-surface surface:5", r#"cys close-surface "surface:5""#),
        ("cys send --text 'a b' x", "cys send --text 'a b' x"),
        ("cys send --text 'a b' x", r#"cys send --text "a b" x"#),
    ] {
        let s = sign_text(&f, signed, "/a/hq", None);
        assert_eq!(s["ok"], json!(true), "{signed}: {s}");
        let c = check(&f, checked, "/a/hq", false);
        assert_eq!(c["result"]["approved"], json!(true), "서명 {signed} / 확인 {checked}: {c}");
    }
}

#[test]
fn a4_unclosed_quote_and_mismatched_array_and_empty_token_are_rejected() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a4-reject", false, 980_011);
    let s = sign(&f, json!({"command_prefix": ["cys", "x"], "command_text": "cys 'x", "cwd": "/a"}));
    assert_eq!(s["error"]["code"], json!("invalid_params"), "{s}");
    let s = sign(&f, json!({"command_prefix": ["cys", "y"], "command_text": "cys x", "cwd": "/a"}));
    assert_eq!(s["error"]["code"], json!("invalid_params"), "배열과 원문이 어긋남: {s}");
    let s = sign(&f, json!({"command_prefix": ["cys", "x"], "command_text": "cys ''", "cwd": "/a"}));
    assert_eq!(s["error"]["code"], json!("invalid_params"), "빈 토큰: {s}");
    // 토큰 수 2 미만(토큰화 결과 기준).
    let s = sign(&f, json!({"command_prefix": ["cys"], "command_text": "cys", "cwd": "/a"}));
    assert_eq!(s["error"]["code"], json!("invalid_params"), "{s}");
    // 토큰화 결과 배열도 허용(원문과 일치).
    let s = sign(&f, json!({"command_prefix": ["cys", "a b"], "command_text": "cys 'a b'", "cwd": "/a"}));
    assert_eq!(s["ok"], json!(true), "{s}");
    // command_text 없는 종전 호출은 그대로.
    let s = sign(&f, json!({"command_prefix": ["git", "push"], "cwd": "/a"}));
    assert_eq!(s["ok"], json!(true), "{s}");
}

/// 교차 검체: 인자 30종에 대해 `shlex.quote` 로 만든 문자열을 데몬 `tokenize` 로 쪼개면 원래 인자가 나온다.
#[test]
fn a4_shlex_quote_roundtrips_through_the_daemon_tokenizer_for_30_args() {
    let args: Vec<String> = [
        "plain", "a b", "a  b", " lead", "trail ", "it's", "say \"hi\"", "back\\slash", "한글 인자", "한글",
        "", "  ", "tab\there", "new\nline", "$HOME", "`x`", "$(x)", "a;b", "a|b", "a&b", "a>b", "a<b",
        "*", "?", "[x]", "{a,b}", "~", "a'b'c", "\"'\"", "mix 'q' \"d\" \\ end",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(args.len(), 30);
    for a in &args {
        let quoted = format!("cmd {}", shlex_quote(a));
        let toks = crate::approval::tokenize(&quoted).expect("닫힌 따옴표");
        assert_eq!(toks, vec!["cmd".to_string(), a.clone()], "인자 {a:?} → {quoted:?}");
    }
}
