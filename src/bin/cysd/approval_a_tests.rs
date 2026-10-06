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
fn a4_unclosed_quote_and_mismatched_array_are_rejected_and_empty_tokens_follow_the_legacy_rule() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a4-reject", false, 980_011);
    let s = sign(&f, json!({"command_prefix": ["cys", "x"], "command_text": "cys 'x", "cwd": "/a"}));
    assert_eq!(s["error"]["code"], json!("invalid_params"), "{s}");
    let s = sign(&f, json!({"command_prefix": ["cys", "y"], "command_text": "cys x", "cwd": "/a"}));
    assert_eq!(s["error"]["code"], json!("invalid_params"), "배열과 원문이 어긋남: {s}");
    // 빈 토큰은 종전 배열 경로와 같이 버려진다(토큰 수 검사는 토큰화 결과에 적용).
    let s = sign(&f, json!({"command_prefix": ["cys", "x", "''"], "command_text": "cys x ''", "cwd": "/a"}));
    assert_eq!(s["ok"], json!(true), "{s}");
    let s = sign(&f, json!({"command_prefix": ["cys", "''"], "command_text": "cys ''", "cwd": "/a"}));
    assert_eq!(s["error"]["code"], json!("invalid_params"), "토큰화 결과가 1토큰: {s}");
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

// ── A3: 서명한 데몬에 묶고 폴더 비교를 건너뛴다 ───────────────────────────────

const HQ: &str = "/a/hq";
const OTHER: &str = "/a/other";

fn approved(r: &Value) -> bool {
    r["result"]["approved"] == json!(true)
}

fn lane_of_record(_f: &Fixture, id_contains: &str) -> Option<String> {
    crate::approval::load_records()
        .into_iter()
        .find(|r| r.command_prefix.join(" ").contains(id_contains))
        .and_then(|r| r.lane_value().map(|s| s.to_string()))
}

fn set_policy(f: &Fixture, body: &str) {
    let p = f.dir.join(".cys").join("policy.json");
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(&p, body).expect("write policy");
}

fn clear_policy(f: &Fixture) {
    let _ = std::fs::remove_file(f.dir.join(".cys").join("policy.json"));
}

#[test]
fn a3_exact_target_command_passes_from_any_folder_of_the_same_daemon() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a3-basic", false, 980_020);
    let cmd = "cys close-surface surface:5";
    let s = sign_text(&f, cmd, HQ, Some(3600));
    assert_eq!(s["ok"], json!(true), "{s}");
    assert!(lane_of_record(&f, "close-surface").is_some(), "대상 동사 + --ttl 서명에는 예약 값이 든다");
    // 다른 폴더에서도 통과.
    assert!(approved(&check(&f, cmd, OTHER, true)), "다른 폴더가 거부됐다");
    assert!(approved(&check(&f, cmd, OTHER, false)));
    assert!(approved(&check(&f, cmd, HQ, true)));
    // 꼬리(출력을 버리는 것)를 붙인 정확 명령.
    for tail in [" 2>&1", " >/dev/null", " 2>/dev/null", " &>/dev/null", " > /dev/null", " 2> /dev/null", " &> /dev/null", " > /dev/null 2>&1", " 2>&1 > /dev/null"] {
        let c = format!("{cmd}{tail}");
        let r = check(&f, &c, OTHER, true);
        assert!(approved(&r), "꼬리 {tail:?}: {r}");
    }
    // 뒤에 인자를 더 붙인 꼴 · 복합 명령은 폴더 건너뛰기 대상이 아니다.
    for bad in [
        "cys close-surface surface:5 --reap",
        "cys close-surface surface:5 surface:9",
        "cys close-surface surface:5 ; echo y",
        "cys close-surface surface:5 && echo y",
        "cys close-surface surface:5 | cat",
        "cys close-surface surface:5 `echo x`",
        "cys close-surface surface:5 $(echo x)",
        "cys close-surface surface:5 \"$(echo x)\"",
        "cys close-surface surface:5\necho y",
        "cys close-surface surface:5 > out.txt",
        "cys close-surface surface:5 2>&1 | tail",
    ] {
        let r = check(&f, bad, OTHER, false);
        assert!(!approved(&r), "건너뛰면 안 되는 꼴이 통과했다: {bad:?} {r}");
        assert_eq!(r["result"]["detail"]["code"], json!("cwd_mismatch"), "{bad:?}: {r}");
    }
    // 같은 따옴표 안의 `$(` 가 작은따옴표면 명령 치환이 아니다(리터럴) — 정확 명령이 아니므로 어차피 건너뛰지 않는다.
    let r = check(&f, "cys close-surface surface:5 '$(x)'", OTHER, false);
    assert!(!approved(&r));
    // 보조 표시: not_exact.
    let r = check(&f, "cys close-surface surface:5 --reap", OTHER, false);
    assert_eq!(r["result"]["detail"]["not_exact"], json!(true), "{r}");
    // 대상 없이 넓게 서명한 접두(`cys kill`)는 건너뛰기 대상이 아니다(S3).
    let s = sign_text(&f, "cys kill", HQ, Some(3600));
    assert_eq!(s["ok"], json!(true), "{s}");
    assert!(!approved(&check(&f, "cys kill 123", OTHER, true)));
    assert!(approved(&check(&f, "cys kill 123", HQ, true)), "같은 폴더는 종전 규칙대로 통과");
}

#[test]
fn a3_target_option_blocks_even_in_the_same_folder_and_names_the_reason() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a3-socket", false, 980_021);
    let cmd = "cys close-surface surface:5";
    assert_eq!(sign_text(&f, cmd, HQ, Some(3600))["ok"], json!(true));
    for opt in [
        "--socket /x/other.sock",
        "--socket=/x/other.sock",
        "-S /x/other.sock",
        "--soc\"\"ket /x/other.sock",
        "--sock''et=/x",
    ] {
        let c = format!("{cmd} {opt}");
        let r = check(&f, &c, HQ, false);
        assert!(!approved(&r), "{c}: 통과했다 {r}");
        assert_eq!(r["result"]["detail"]["code"], json!("lane_mismatch"), "{c}: {r}");
        assert_eq!(r["result"]["detail"]["target_option"], json!(true), "{c}: {r}");
        let r = check(&f, &c, OTHER, false);
        assert!(!approved(&r), "{c}: {r}");
    }
    // 대상 데몬을 바꾸는 옵션이 든 명령의 서명에는 예약 값을 넣지 않는다(종전 규칙의 레코드).
    let s = sign_text(&f, "cys kill 7 --socket /x/y.sock", HQ, Some(3600));
    assert_eq!(s["ok"], json!(true), "{s}");
    assert!(lane_of_record(&f, "kill 7").is_none(), "--socket 이 든 명령의 서명에 예약 값이 들어갔다");
}

#[test]
fn a3_lane_binds_to_the_signing_daemon_not_just_the_folder() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a3-two", false, 980_022);
    let other = crate::team_gate_tests::tmp_daemon("a3-two-dept", true);
    let cmd = "cys close-surface surface:5";
    assert_eq!(sign_text(&f, cmd, HQ, Some(3600))["ok"], json!(true));
    // 같은 사용자 폴더(같은 승인 저장소)를 읽는 **다른 데몬** — 폴더가 같아도 거부 · 사유 lane_mismatch.
    let r = rpc(&other, None, "approval.check", json!({"command": cmd, "cwd": HQ, "require_ttl": false}));
    assert!(!approved(&r), "다른 데몬에서 통과했다: {r}");
    assert_eq!(r["result"]["detail"]["code"], json!("lane_mismatch"), "{r}");
    let r = rpc(&other, None, "approval.check", json!({"command": cmd, "cwd": OTHER, "require_ttl": true}));
    assert!(!approved(&r), "{r}");
    // 요청이 실어 온 같은 이름의 환경은 모든 갈래에서 무시된다 — 본부의 값을 훔쳐 와도 소용없다.
    let hq_lane = lane_of_record(&f, "close-surface").expect("lane");
    let r = rpc(
        &other,
        None,
        "approval.check",
        json!({"command": cmd, "cwd": HQ, "env": {"CYS_APPROVAL_LANE": hq_lane}}),
    );
    assert!(!approved(&r), "요청이 실은 예약 값이 쓰였다: {r}");
    // 서명 쪽도 같다: 요청이 실은 예약 값은 버려지고 데몬의 값이 들어간다(대상 동사 서명) / 들어가지 않는다(대상 밖 서명).
    let s = sign(
        &f,
        json!({"command_prefix": ["cys", "kill", "9"], "cwd": HQ, "ttl_secs": 3600, "env": {"CYS_APPROVAL_LANE": "forged"}}),
    );
    assert_eq!(s["ok"], json!(true), "{s}");
    assert_eq!(lane_of_record(&f, "kill 9"), Some(hq_lane.clone()), "데몬의 값이 아니다");
    let s = sign(
        &f,
        json!({"command_prefix": ["git", "push"], "cwd": HQ, "ttl_secs": 3600, "env": {"CYS_APPROVAL_LANE": "forged"}}),
    );
    assert_eq!(s["ok"], json!(true), "{s}");
    assert!(lane_of_record(&f, "git push").is_none(), "대상 밖 서명에 예약 값이 남았다");
    let s = sign(
        &f,
        json!({"command_prefix": ["cys", "kill", "11"], "cwd": HQ, "env": {"CYS_APPROVAL_LANE": "forged"}}),
    );
    assert_eq!(s["ok"], json!(true), "{s}");
    assert!(lane_of_record(&f, "kill 11").is_none(), "무기한 서명에 예약 값이 남았다");
    // 무기한 승인과 대상 밖 명령은 종전 규칙 — 폴더가 같으면 다른 데몬에서도 맞는다(넓히지도 조이지도 않는다).
    let r = rpc(&other, None, "approval.check", json!({"command": "git push origin x", "cwd": HQ}));
    assert!(approved(&r), "종전 규칙이 바뀌었다: {r}");
    let r = rpc(&other, None, "approval.check", json!({"command": "git push origin x", "cwd": OTHER}));
    assert!(!approved(&r));
}

#[test]
fn a3_lane_survives_a_restart_with_the_same_state_dir_but_not_a_recreated_one() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a3-restart", false, 980_023);
    let cmd = "cys close-surface surface:5";
    assert_eq!(sign_text(&f, cmd, HQ, Some(3600))["ok"], json!(true));
    let lane1 = lane_of_record(&f, "close-surface").expect("lane");
    // 같은 상태 폴더로 데몬을 다시 띄운 것(캐시를 비운다 = 새 프로세스) → 같은 값 · 종전 승인 통과.
    crate::approval::forget_lane_cache();
    assert!(approved(&check(&f, cmd, OTHER, true)), "재기동 뒤 승인이 끊겼다");
    // 묶음 파일을 지우고 띄우면 새 값이 생겨 종전 승인은 폴더가 같아도 거부된다(상태 폴더가 새로 만들어진 것).
    let state_dir = crate::state::state_dir(&f.daemon.socket_path);
    let lane_file = state_dir.join(crate::approval::LANE_FILE);
    assert!(lane_file.exists(), "부팅 때 만든 묶음 파일이 없다");
    std::fs::remove_file(&lane_file).expect("rm");
    crate::approval::forget_lane_cache();
    let r = check(&f, cmd, HQ, true);
    assert!(!approved(&r), "{r}");
    assert_eq!(r["result"]["detail"]["code"], json!("lane_mismatch"), "{r}");
    let s = sign_text(&f, "cys close-surface surface:6", HQ, Some(3600));
    assert_eq!(s["ok"], json!(true));
    assert_ne!(lane_of_record(&f, "surface:6"), Some(lane1), "새 값이어야 한다");
    // 묶음 파일을 쓸 수 없는 폴더에서도 값이 나온다(메모리 값 · 로그 한 줄) — 같은 프로세스 안에서는 그 값이 유지된다.
    let blocked = std::env::temp_dir().join(format!("cys-a3-blocked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&blocked);
    std::fs::create_dir_all(blocked.join(crate::approval::LANE_FILE)).expect("디렉터리로 막는다");
    let sock = blocked.join("cysd.sock");
    let v1 = crate::approval::lane_value(&sock, &blocked);
    let v2 = crate::approval::lane_value(&sock, &blocked);
    assert_eq!(v1, v2, "프로세스가 사는 동안 값이 바뀌었다");
    assert!(v1.starts_with(&format!("{}|", sock.display())));
    let _ = std::fs::remove_dir_all(&blocked);
}

#[test]
fn a3_launch_agent_skips_the_folder_only_when_a_literal_absolute_cwd_is_written() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a3-launch", false, 980_024);
    let base = "cys launch-agent --role worker --agent claude";
    // 통과하는 꼴.
    for (i, cmd) in [
        format!("{base} --cwd /a/hq"),
        format!("{base} --cwd=/a/hq"),
        format!("{base} --cwd \"/a/hq w\""),
        format!("{base} --cwd C:/Users/me/ws"),
        format!("{base} --cwd 'C:\\Users\\me\\ws'"),
    ]
    .iter()
    .enumerate()
    {
        let s = sign_text(&f, cmd, HQ, Some(3600));
        assert_eq!(s["ok"], json!(true), "{cmd}: {s}");
        let r = check(&f, cmd, OTHER, true);
        assert!(approved(&r), "{i} {cmd}: {r}");
    }
    // 통과하지 않는 꼴 — 폴더가 다르면 거부되고 사유는 needs_launch_cwd.
    for cmd in [
        base.to_string(),
        format!("{base} --cwd ''"),
        format!("{base} --cwd $PWD"),
        format!("{base} --cwd ~+"),
        format!("{base} --cwd ~/x"),
        format!("{base} --cwd rel/dir"),
        format!("{base} --cwd"),
        format!("{base} --cwd /a/*"),
        format!("{base} --cwd=C:Usersmews"),
        format!("{base} --cwd C:\\Users\\me\\ws"), // 따옴표 밖의 역슬래시는 토크나이저가 지운다 → `C:Usersmews`
    ] {
        let s = sign_text(&f, &cmd, HQ, Some(3600));
        assert_eq!(s["ok"], json!(true), "{cmd}: {s}");
        let r = check(&f, &cmd, OTHER, true);
        assert!(!approved(&r), "{cmd}: 통과했다 {r}");
        assert_eq!(r["result"]["detail"]["code"], json!("cwd_mismatch"), "{cmd}: {r}");
        assert_eq!(r["result"]["detail"]["needs_launch_cwd"], json!(true), "{cmd}: {r}");
        // 같은 폴더에서는 종전 규칙대로 통과한다.
        assert!(approved(&check(&f, &cmd, HQ, true)), "{cmd}: 같은 폴더가 거부됐다");
    }
}

#[test]
fn a3_off_switch_and_broken_policy_return_to_the_0_14_43_rule() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a3-off", false, 980_025);
    let cmd = "cys close-surface surface:5";
    // 켠 채 서명한 레코드(예약 값 있음).
    assert_eq!(sign_text(&f, cmd, HQ, Some(3600))["ok"], json!(true));
    assert!(approved(&check(&f, cmd, OTHER, true)));
    let caps = rpc(&f.daemon, None, "approval.capabilities", json!({}));
    assert_eq!(caps["result"]["cwd_neutral_verbs"].as_array().map(|a| a.len()), Some(7), "{caps}");
    // 손잡이를 끈다 → 켠 동안 만든 레코드도 이 데몬에서 폴더가 같을 때만 맞는다 · 사유 neutral_off.
    set_policy(&f, r#"{"approval_cwd_neutral": false}"#);
    let r = check(&f, cmd, OTHER, true);
    assert!(!approved(&r), "{r}");
    assert_eq!(r["result"]["detail"]["neutral_off"], json!(true), "{r}");
    assert!(approved(&check(&f, cmd, HQ, true)), "같은 폴더 · 같은 데몬은 통과");
    let caps = rpc(&f.daemon, None, "approval.capabilities", json!({}));
    assert_eq!(caps["result"]["cwd_neutral_verbs"], json!([]), "꺼진 상태가 숨었다: {caps}");
    // 끈 채 서명하면 예약 값이 들어가지 않는다(0.14.43 의 레코드).
    assert_eq!(sign_text(&f, "cys close-surface surface:8", HQ, Some(3600))["ok"], json!(true));
    assert!(lane_of_record(&f, "surface:8").is_none());
    assert!(!approved(&check(&f, "cys close-surface surface:8", OTHER, true)));
    // 깨진 정책 파일 · 불리언이 아닌 값 = 끈 것으로 본다(설정을 확실히 읽었을 때만 새 규칙).
    for broken in ["{ not json", r#"{"approval_cwd_neutral": "yes"}"#, r#"{"approval_cwd_neutral": 1}"#] {
        set_policy(&f, broken);
        assert!(!approved(&check(&f, cmd, OTHER, true)), "{broken}: 새 규칙이 켜졌다");
    }
    // 키가 없으면 기본(켬) · 파일이 없으면 기본(켬).
    set_policy(&f, r#"{"deny_self_approve": true}"#);
    assert!(approved(&check(&f, cmd, OTHER, true)));
    clear_policy(&f);
    assert!(approved(&check(&f, cmd, OTHER, true)));
}

#[test]
fn a3_schedule_gate_never_matches_lane_records_and_old_rule_is_unchanged() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("a3-sched", false, 980_026);
    // 대상 동사 명령을 시간 한정으로 서명 → 예약 값이 든다. 스케줄 게이트(환경이 빈 채 `best_match`)는 맞지 않는다.
    assert_eq!(sign_text(&f, "cys close-surface surface:5", HQ, Some(3600))["ok"], json!(true));
    // 예약 값이 없는 레코드(무기한)는 0.14.43 과 같다.
    assert_eq!(sign_text(&f, "echo hello", HQ, None)["ok"], json!(true));
    let secret = crate::approval::signing_secret().expect("secret");
    let records = crate::approval::load_records();
    assert!(
        crate::approval::best_match(&records, &secret, "cys close-surface surface:5", Some(HQ), &[]).is_none(),
        "스케줄 게이트(환경 없음)가 예약 값이 든 레코드와 맞았다"
    );
    assert!(crate::approval::best_match(&records, &secret, "echo hello world", Some(HQ), &[]).is_some());
    assert!(crate::approval::best_match(&records, &secret, "echo hello world", Some(OTHER), &[]).is_none());
}

#[test]
fn a3_reserved_name_is_not_swallowed_by_the_sensitive_key_filter_and_record_stays_valid() {
    let _g = A_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let kept = crate::approval::sort_norm_env(&[(crate::approval::LANE_ENV_KEY.to_string(), "v".to_string())]);
    assert_eq!(kept.len(), 1, "예약 이름이 민감 키 거르기에 걸렸다 — 묶음이 조용히 사라진다");
    let f = fixture("a3-valid", false, 980_027);
    assert_eq!(sign_text(&f, "cys close-surface surface:5", HQ, Some(3600))["ok"], json!(true));
    let secret = crate::approval::signing_secret().expect("secret");
    let recs = crate::approval::load_records();
    let r = recs.iter().find(|r| r.lane_value().is_some()).expect("레코드");
    assert!(r.has_valid_signature(&secret), "예약 값을 넣은 레코드의 서명이 유효하지 않다");
    let mut sorted = r.environment.clone();
    sorted.sort();
    assert_eq!(sorted, r.environment, "환경이 정렬돼 있지 않다");
}

/// 동사 목록이 게이트 훅과 어긋나지 않게 — `CSO_CYS_TTL_VERBS`(6종) 와 `CSO_CYS_OPT_TTL` 의 열쇠(`cycle-agent`)를 **둘 다** 모은 합집합 = 데몬의 7종.
#[test]
fn a3_verbs_match_the_gate_hook_union() {
    let hook = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/cysjavis-pack/hooks/role-capability-gate.sh"))
        .expect("게이트 훅 파일");
    let quoted = |s: &str| -> Vec<String> {
        s.split('"').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, t)| t.to_string()).collect()
    };
    let line = hook.lines().find(|l| l.starts_with("CSO_CYS_TTL_VERBS = {")).expect("CSO_CYS_TTL_VERBS");
    let mut set: std::collections::BTreeSet<String> = quoted(line).into_iter().collect();
    assert_eq!(set.len(), 6, "{line}");
    let start = hook.find("CSO_CYS_OPT_TTL = {").expect("CSO_CYS_OPT_TTL");
    let block = &hook[start..start + hook[start..].find("\n}\n").expect("블록 끝")];
    for l in block.lines().skip(1) {
        if let Some((k, _)) = l.trim().split_once(':') {
            set.extend(quoted(k));
        }
    }
    let ours: std::collections::BTreeSet<String> =
        crate::approval::CWD_NEUTRAL_VERBS.iter().map(|s| s.to_string()).collect();
    assert_eq!(set, ours, "훅의 대상 동사 합집합과 데몬의 7종이 어긋났다");
    // 대상 데몬을 바꾸는 옵션도 훅과 같다.
    let l = hook.lines().find(|l| l.starts_with("CSO_CYS_TARGET_OPTS = (")).expect("CSO_CYS_TARGET_OPTS");
    assert_eq!(quoted(l), vec!["--socket".to_string(), "-S".to_string()]);
    assert!(crate::approval::has_socket_option(&["x".into(), "--socket".into()]));
    assert!(crate::approval::has_socket_option(&["x".into(), "--socket=/y".into()]));
    assert!(crate::approval::has_socket_option(&["x".into(), "-S".into()]));
    assert!(!crate::approval::has_socket_option(&["x".into(), "--sockets".into(), "-s".into()]));
}

/// 단순 명령 판정은 **원문 스캐너**로 한다 — 토큰화 결과로 판정하는 코드가 없음을 고정(독립 검증 X21): `"$(x)"` 와 `'$(x)'` 의 토큰은 같지만 판정이 갈린다.
#[test]
fn a3_simple_command_scanner_works_on_raw_text_not_on_tokens() {
    use crate::approval::simple_command_core as core;
    assert_eq!(crate::approval::tokenize("a \"$(x)\""), crate::approval::tokenize("a '$(x)'"));
    assert!(core("cys kill 1 \"$(x)\"").is_none(), "큰따옴표 안의 명령 치환은 복합이다");
    assert!(core("cys kill 1 '$(x)'").is_some(), "작은따옴표 안은 리터럴이다");
    assert!(core("cys kill 1 `x`").is_none());
    assert!(core("cys kill 1 ';'").is_some(), "따옴표 안의 연결 기호는 인자다");
    assert!(core(r"cys kill 1 \;").is_some(), "이스케이프된 연결 기호는 인자다");
    assert!(core("cys kill 1 ;").is_none());
    assert!(core("cys kill 1 '").is_none(), "닫히지 않은 따옴표");
    assert!(core("cys kill 1 > x").is_none());
    assert_eq!(core("cys kill 1 > /dev/null 2>&1"), Some(vec!["cys".into(), "kill".into(), "1".into()]));
    assert_eq!(core("cys kill 1 2>&1"), Some(vec!["cys".into(), "kill".into(), "1".into()]));
    assert!(core("cys kill 1 2>&1 x").is_none(), "꼬리 뒤에 다른 낱말이 있으면 꼬리가 아니다");
    assert!(core("cys kill 1 '>/dev/null'").map(|c| c.len()) == Some(4), "따옴표로 감싼 꼬리 글자는 인자다(떼지 않는다)");
    // 소스 고정: 판정 함수들이 토큰화 결과(`tokenize`)만으로 연결 기호를 보지 않는다 — 스캐너(`scan_words`)를 거친다.
    let src = include_str!("approval.rs");
    let body = &src[src.find("pub fn simple_command_core(").expect("앵커")..];
    let body = &body[..body.find("\n}\n").expect("끝")];
    assert!(body.contains("scan_words(raw)"), "단순 명령 판정이 원문 스캐너를 쓰지 않는다");
}
