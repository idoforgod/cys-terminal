//! ★0.14.42 P5 — 대화 승인 **1회용 팀 생성 토큰**의 데몬 집행 회귀 핀.
//!
//! 설계 정본: `팀만들기-확인창-무반응-수정설계안-최종-20260923.md` §6-5(검증·소비 = 데몬) · §8-2(생성 →
//! 부트 티켓 → … → 생성 성공 뒤에만 allow) · §11 R6·R7·R8 · §13 P5.
//! 판정 정의처: `cysjavis-pack/bin/javis_teamtoken.py`(원장·전이·만료·결박 — 2단 권한). 데몬은 그
//! 모듈을 **부를 뿐**이고(사본 금지), 데몬만 아는 두 사실을 보탠다:
//!   ① 호출 좌석 = 커널 peer pid 의 조상 체인(`resolve_caller_surface`) — env·자기신고 불신
//!   ② 제안의 **현재** 본문·대기 여부 = 데몬 메모리(`feed_items`) — 파일(feed.jsonl) 불신
//!
//! 이 파일은 기존 공개 API(`dispatch` · U16 헬퍼)와 P3 CLI(픽스처용 `path`·`digest`)만 쓴다 — 구현
//! 전에도 컴파일되어 RED 를 **실행 시점**에 관측할 수 있게(미지 메서드·종전 거부 코드로 실패한다).
//!
//! 픽스처 원장은 훅(`issue`)이 쓰는 것과 같은 3줄(질문 열림 → 승인으로 닫힘 → 토큰 발급)을 손으로
//! 쓴다. 훅의 판별(배달 원장·기계 유래·전문 일치)은 P3/P4 의 몫이고 여기서 재는 것은 **데몬 집행**이다.
#![cfg(test)]

use crate::handlers::{dispatch, Reply};
use crate::state::Daemon;
use crate::team_gate_tests::{body, push, seat, tmp_daemon};
use base64::Engine as _;
use cys::Request;
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/cysjavis-pack/bin/javis_teamtoken.py");

fn b64(s: &str) -> String {
    base64::engine::general_purpose::URL_SAFE.encode(s.as_bytes())
}

fn tok(n: u64) -> String {
    format!("{n:032x}")
}

/// P3 CLI 를 데몬과 **같은 env**(소켓·상태 루트)로 직접 부른다 — 원장 경로 규약·정규형 본문해시를
/// 테스트가 Rust 로 다시 쓰지 않게(사본은 갈린다).
fn py(d: &Arc<Daemon>, args: &[&str]) -> Value {
    let out = cys::python_command("python3")
        .arg(SCRIPT)
        .args(args)
        .env("CYS_SOCKET", d.socket_path.to_string_lossy().as_ref())
        .env("CYS_STATE_DIR", crate::delivery::pack_state_dir())
        .env_remove("CYS_SURFACE_ID")
        .env_remove("AITERM_SURFACE_ID")
        .output()
        .expect("python3 실행");
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|e| {
        panic!("P3 CLI 출력 판독 실패({e}): {text:?} / {}", String::from_utf8_lossy(&out.stderr))
    })
}

fn ledger(d: &Arc<Daemon>) -> PathBuf {
    PathBuf::from(py(d, &["path"])["path"].as_str().expect("원장 경로"))
}

fn digest(d: &Arc<Daemon>, b: &str) -> String {
    py(d, &["digest", "--body-b64", &b64(b)])["body_digest"].as_str().expect("본문해시").to_string()
}

fn append(d: &Arc<Daemon>, recs: &[Value]) {
    let p = ledger(d);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&p).unwrap();
    for r in recs {
        writeln!(f, "{r}").unwrap();
    }
}

fn events(d: &Arc<Daemon>) -> Vec<Value> {
    std::fs::read_to_string(ledger(d))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("원장 줄"))
        .collect()
}

fn count(d: &Arc<Daemon>, event: &str, phase: Option<&str>) -> usize {
    events(d)
        .iter()
        .filter(|r| r["event"] == json!(event) && phase.map_or(true, |p| r["phase"] == json!(p)))
        .count()
}

/// 훅이 쓰는 것과 같은 발급 3줄. `ttl` = 토큰 만료까지 남은 초(음수 = 이미 만료).
fn issue(d: &Arc<Daemon>, proposal: &str, surface: u64, dig: &str, token: &str, ttl: f64) {
    let now = crate::state::now_epoch();
    // 질문 id 는 토큰의 **아래** 16자리 — 위 16자리는 작은 번호에서 전부 0 이라 질문 id 가 겹치고,
    // 겹치면 P3 판독기가 원장 손상(ledger_corrupt)으로 접는다(첫 구현에서 실제로 그렇게 접혔다).
    let ask = &token[16..];
    let s = surface.to_string();
    append(
        d,
        &[
            json!({"v": 1, "kind": "team-create-ask", "event": "ask_opened", "ask_id": ask,
                   "proposal_id": proposal, "surface": s, "body_digest": dig,
                   "opened_at": now - 5.0, "expires_at": now + 295.0, "pid": 1}),
            json!({"v": 1, "kind": "team-create-ask", "event": "ask_closed", "ask_id": ask,
                   "surface": s, "proposal_id": proposal, "why": "approved", "at": now - 1.0}),
            json!({"v": 1, "kind": "team-create-token", "event": "token_issued", "token": token,
                   "proposal_id": proposal, "surface": s, "body_digest": dig, "issued_at": now - 1.0,
                   "expires_at": now + ttl, "consumed": false, "ask_id": ask, "pid": 1, "ppid": 1,
                   "hook_session": null}),
        ],
    );
}

/// 발급 + 생성 단계 소비 + 생성 성공 기록(allow 권한 무장 상태 = `created`).
fn issue_armed(d: &Arc<Daemon>, proposal: &str, surface: u64, dig: &str, token: &str) {
    issue(d, proposal, surface, dig, token, 120.0);
    let now = crate::state::now_epoch();
    let s = surface.to_string();
    append(
        d,
        &[
            json!({"v": 1, "kind": "team-create-token", "event": "consumed", "token": token,
                   "phase": "create", "proposal_id": proposal, "surface": s, "body_digest": dig,
                   "consumed": true, "at": now, "pid": 1}),
            json!({"v": 1, "kind": "team-create-token", "event": "settled", "token": token,
                   "outcome": "created", "proposal_id": proposal, "surface": s, "at": now,
                   "dept": "dept-7", "grant_expires_at": now + 1800.0}),
        ],
    );
}

fn rpc(d: &Arc<Daemon>, pid: Option<u32>, method: &str, params: Value) -> Value {
    match dispatch(d, Request { id: json!(7), method: method.into(), params }, pid) {
        Reply::Single(v) => v,
        _ => panic!("{method} 는 단일 응답이어야 한다"),
    }
}

fn reply(d: &Arc<Daemon>, pid: Option<u32>, rid: &str, decision: &str, team_token: Option<&str>) -> Value {
    let mut p = json!({"request_id": rid, "decision": decision});
    if let Some(t) = team_token {
        p["team_token"] = json!(t);
    }
    rpc(d, pid, "feed.reply", p)
}

fn code(v: &Value) -> String {
    v["error"]["code"].as_str().unwrap_or("").to_string()
}

fn status(d: &Arc<Daemon>, rid: &str) -> (String, Option<String>) {
    let items = d.feed_items.lock().unwrap();
    let it = items.iter().find(|i| i.request_id == rid).expect("항목");
    (it.status.clone(), it.decision.clone())
}

/// 본부 master 좌석 + 대기 제안 1건.
fn proposal(tag: &str, pid: u32) -> (Arc<Daemon>, u64, String, String) {
    let d = tmp_daemon(tag, false);
    let sid = seat(&d, "master", pid);
    let rid = format!("tp-{tag}");
    let b = body(&rid, "일반저술출판부", "책을 쓰고 출판한다.\n원고를 편집한다.");
    let r = push(&d, Some(pid), &rid, &b, json!({}));
    assert_eq!(r["ok"], json!(true), "제안 발행 실패: {r}");
    (d, sid, rid, b)
}

// ── 전 과정(설계 §8-2 ①→⑦) ──────────────────────────────────────────────────

#[test]
fn p5_full_protocol_consume_settle_then_allow_from_publisher_seat() {
    let m = 720_001;
    let (d, sid, rid, b) = proposal("tt-full", m);
    let t = tok(1);
    issue(&d, &rid, sid, &digest(&d, &b), &t, 120.0);

    // ① 생성 단계 소비 — 데몬이 커널 신원으로 좌석을 대조하고, 자기 메모리의 **현재 본문**을 넘긴다.
    let c = rpc(&d, Some(m), "team.token.consume", json!({"team_token": t}));
    assert_eq!(c["ok"], json!(true), "정상 토큰의 생성 단계 소비가 거부됐다: {c}");
    assert_eq!(c["result"]["proposal_id"], json!(rid));
    let spec_b64 = c["result"]["spec_b64"].as_str().expect("spec_b64");
    assert_eq!(
        cys::team_spec::from_b64(spec_b64).expect("spec_b64 판독"),
        cys::team_spec::parse_body(&b).unwrap(),
        "소비 응답의 명세가 오너가 승인한 제안 본문과 다르다"
    );
    assert_eq!(count(&d, "consumed", Some("create")), 1, "원장에 생성 단계 소비 기록이 없다");
    // 1회성 — 같은 토큰의 두 번째 소비는 구체 코드로 거부된다(생성은 한 번뿐).
    let c2 = rpc(&d, Some(m), "team.token.consume", json!({"team_token": t}));
    assert_eq!(code(&c2), "token_consumed", "재사용 토큰이 거부되지 않았다: {c2}");
    // 생성 결과 기록 전에는 allow 권한이 없다 — 카드는 남는다.
    let early = reply(&d, Some(m), &rid, "allow", Some(&t));
    assert_eq!(code(&early), "grant_not_armed", "생성 확인 전 allow 가 통과했다: {early}");
    assert_eq!(status(&d, &rid).0, "pending");
    // 조회(인가 아님) — 같은 좌석·상태.
    let i = rpc(&d, Some(m), "team.token.inspect", json!({"team_token": t}));
    assert_eq!(i["ok"], json!(true), "{i}");
    assert_eq!(i["result"]["state"], json!("consumed"));
    assert_eq!(i["result"]["same_seat"], json!(true));
    assert_eq!(i["result"]["proposal_id"], json!(rid));

    // ② 생성 성공 기록 → allow 권한 무장.
    let s = rpc(&d, Some(m), "team.token.settle", json!({"team_token": t, "outcome": "created", "dept": "dept-7"}));
    assert_eq!(s["ok"], json!(true), "settle created 가 거부됐다: {s}");
    let s2 = rpc(&d, Some(m), "team.token.settle", json!({"team_token": t, "outcome": "created", "dept": "dept-7"}));
    assert_eq!(code(&s2), "already_settled", "{s2}");

    // ⑦ 생성 성공 뒤 allow — 발행 좌석 자신이지만 토큰이 오너 승인의 증거다.
    let a = reply(&d, Some(m), &rid, "allow", Some(&t));
    assert_eq!(a["ok"], json!(true), "무장된 토큰의 allow 가 거부됐다: {a}");
    assert_eq!(a["result"]["team_token"]["consumed"], json!(true), "allow 권한이 닫히지 않았다: {a}");
    assert_eq!(status(&d, &rid), ("resolved".to_string(), Some("allow".to_string())));
    assert_eq!(count(&d, "consumed", Some("allow")), 1);
    // 권한은 1회 — 해소된 항목은 데몬이 먼저 막는다.
    let a2 = reply(&d, Some(m), &rid, "allow", Some(&t));
    assert_eq!(a2["ok"], json!(false), "{a2}");
    let i2 = rpc(&d, Some(m), "team.token.inspect", json!({"team_token": t}));
    assert_eq!(i2["result"]["state"], json!("done"), "{i2}");
    // 카드가 정리된 뒤의 재실행도 사유는 '이미 쓴 토큰'이다(P3 판정 순서 = 토큰 상태가 먼저) — 제안 부재
    // (proposal_not_pending)로 답하면 cys-dept 의 멱등 보고(이미 만든 팀)가 이 시점에서 끊긴다(E2E 실측).
    let c3 = rpc(&d, Some(m), "team.token.consume", json!({"team_token": t}));
    assert_eq!(code(&c3), "token_consumed", "정리 뒤 재실행의 사유가 토큰 상태가 아니다: {c3}");
}

// ── 위조·누락·형식 ──────────────────────────────────────────────────────────

#[test]
fn p5_forged_or_missing_token_refused_with_specific_codes() {
    let m = 720_010;
    let (d, sid, rid, b) = proposal("tt-forge", m);
    issue_armed(&d, &rid, sid, &digest(&d, &b), &tok(10));
    let forged = tok(0xdead_beef);
    for (label, t, want) in [
        ("빈 토큰", String::new(), "token_missing"),
        ("미발급 토큰", forged.clone(), "token_unknown"),
        ("형식 밖", "not-a-token".to_string(), "token_unknown"),
        ("대문자 hex", tok(10).to_uppercase(), "token_unknown"),
        ("옵션 모양", format!("-{}", &tok(10)[1..]), "token_unknown"),
    ] {
        let x = reply(&d, Some(m), &rid, "allow", Some(&t));
        assert_eq!(code(&x), want, "feed.reply {label}: {x}");
        assert!(
            x["error"]["message"].as_str().is_some_and(|s| !s.is_empty()),
            "무안내 거부(메시지 없음) — {label}: {x}"
        );
        let c = rpc(&d, Some(m), "team.token.consume", json!({"team_token": t}));
        assert_eq!(code(&c), want, "team.token.consume {label}: {c}");
    }
    let c = rpc(&d, Some(m), "team.token.consume", json!({}));
    assert_eq!(code(&c), "token_missing", "토큰 인자 자체가 없을 때: {c}");
    // 위조 토큰 거부 문구 = P3 오너 문구(단일 출처) 그대로.
    let x = rpc(&d, Some(m), "team.token.consume", json!({"team_token": forged}));
    assert!(
        x["error"]["message"].as_str().unwrap_or("").contains("승인 말씀이 시스템에 닿지 않았습니다"),
        "위조 토큰의 안내가 P3 오너 문구가 아니다: {x}"
    );
    assert_eq!(status(&d, &rid).0, "pending", "거부됐는데 카드가 사라졌다");
    assert_eq!(count(&d, "consumed", None), 1, "거부 경로가 소비를 남겼다(픽스처의 create 1건만 있어야 한다)");
}

// ── 좌석 결박 = 커널 신원 ───────────────────────────────────────────────────

#[test]
fn p5_seat_binding_uses_kernel_identity_not_self_report() {
    let m = 720_020;
    let w = 720_021;
    let (d, sid, rid, b) = proposal("tt-seat", m);
    seat(&d, "worker", w);
    let dig = digest(&d, &b);
    let t = tok(20);
    issue(&d, &rid, sid, &dig, &t, 120.0);
    // 다른 좌석(워커)이 대표의 토큰을 쓰면 좌석 결박으로 거부.
    let c = rpc(&d, Some(w), "team.token.consume", json!({"team_token": t}));
    assert_eq!(code(&c), "token_surface_mismatch", "{c}");
    // 어느 좌석에도 귀속되지 않은 호출자(pane 밖)는 좌석 미상.
    let c = rpc(&d, None, "team.token.consume", json!({"team_token": t}));
    assert_eq!(code(&c), "surface_unknown", "{c}");
    // 좌석을 신고하는 인자는 받지 않는다(hook.decide 와 같은 규약) — 조용히 무시하지 않고 거절.
    for k in ["surface", "surface_id"] {
        let c = rpc(&d, Some(w), "team.token.consume", json!({"team_token": t, k: sid}));
        assert_eq!(code(&c), "invalid_params", "좌석 자기신고({k})가 받아들여졌다: {c}");
    }
    // allow 도 같은 결박: 무장된 토큰이라도 워커 좌석의 allow 는 거부.
    let t2 = tok(21);
    issue_armed(&d, &rid, sid, &dig, &t2);
    let x = reply(&d, Some(w), &rid, "allow", Some(&t2));
    assert_eq!(code(&x), "token_surface_mismatch", "{x}");
    let x = reply(&d, None, &rid, "allow", Some(&t2));
    assert_eq!(code(&x), "surface_unknown", "{x}");
    assert_eq!(status(&d, &rid).0, "pending");
    // 거부 뒤에도 정당한 좌석의 소비는 그대로 된다(거부가 토큰을 태우지 않는다).
    let ok = rpc(&d, Some(m), "team.token.consume", json!({"team_token": t}));
    assert_eq!(ok["ok"], json!(true), "{ok}");
}

// ── 결박: 본문·제안·만료 ────────────────────────────────────────────────────

#[test]
fn p5_body_changed_after_approval_is_token_body_mismatch() {
    let m = 720_030;
    let (d, sid, rid, b) = proposal("tt-body", m);
    let t = tok(30);
    issue(&d, &rid, sid, &digest(&d, &b), &t, 120.0);
    // 승인 뒤 본문이 바뀌었다(TOCTOU) — 데몬은 파일이 아니라 자기 메모리의 현재 본문으로 대조한다.
    let b2 = body(&rid, "일반저술출판부", "전혀 다른 일을 한다.");
    for it in d.feed_items.lock().unwrap().iter_mut() {
        if it.request_id == rid {
            it.body = b2.clone();
        }
    }
    let c = rpc(&d, Some(m), "team.token.consume", json!({"team_token": t}));
    assert_eq!(code(&c), "token_body_mismatch", "{c}");
    assert_eq!(count(&d, "consumed", None), 0);
}

#[test]
fn p5_withdrawn_proposal_or_other_proposal_is_refused() {
    let m = 720_040;
    let (d, sid, rid, b) = proposal("tt-gone", m);
    let dig = digest(&d, &b);
    let t = tok(40);
    issue(&d, &rid, sid, &dig, &t, 120.0);
    // 무장된 다른 제안의 토큰으로 이 카드를 allow — 제안 결박.
    let other = "tp-tt-other";
    let t_other = tok(41);
    issue_armed(&d, other, sid, &dig, &t_other);
    let x = reply(&d, Some(m), &rid, "allow", Some(&t_other));
    assert_eq!(code(&x), "token_proposal_mismatch", "{x}");
    // 제안을 거둔 뒤의 생성 소비 — 대기 중이 아니다.
    let s = reply(&d, Some(m), &rid, "superseded", None);
    assert_eq!(s["ok"], json!(true), "{s}");
    let c = rpc(&d, Some(m), "team.token.consume", json!({"team_token": t}));
    assert_eq!(code(&c), "proposal_not_pending", "{c}");
    assert_eq!(count(&d, "consumed", Some("create")), 1, "거둔 제안에 소비가 남았다(픽스처 1건만)");
}

#[test]
fn p5_expired_token_is_token_expired() {
    let m = 720_050;
    let (d, sid, rid, b) = proposal("tt-exp", m);
    let t = tok(50);
    issue(&d, &rid, sid, &digest(&d, &b), &t, -10.0);
    let c = rpc(&d, Some(m), "team.token.consume", json!({"team_token": t}));
    assert_eq!(code(&c), "token_expired", "{c}");
    assert!(c["error"]["message"].as_str().unwrap_or("").contains("만료"), "{c}");
}

// ── allow 전용 · 종전 거부 유지 ─────────────────────────────────────────────

#[test]
fn p5_token_never_authorizes_deny_and_absent_token_keeps_owner_gui_required() {
    let m = 720_060;
    let (d, sid, rid, b) = proposal("tt-deny", m);
    let t = tok(60);
    issue_armed(&d, &rid, sid, &digest(&d, &b), &t);
    // 토큰은 allow 에만 — deny(소각)·자유문구는 여전히 오너 GUI 전용.
    for dec in ["deny", "yes", "approve", "Allow"] {
        let x = reply(&d, Some(m), &rid, dec, Some(&t));
        assert_eq!(code(&x), "owner_gui_required", "토큰으로 '{dec}' 가 열렸다: {x}");
    }
    // 토큰 인자 없는 allow = 종전 그대로(토큰 경로를 시도하지 않은 호출).
    let x = reply(&d, Some(m), &rid, "allow", None);
    assert_eq!(code(&x), "owner_gui_required", "{x}");
    assert_eq!(status(&d, &rid).0, "pending");
    assert_eq!(count(&d, "consumed", Some("allow")), 0, "거부 경로가 allow 권한을 소비했다");
}

#[test]
fn p5_failed_creation_revokes_allow() {
    let m = 720_070;
    let (d, sid, rid, b) = proposal("tt-fail", m);
    let t = tok(70);
    issue(&d, &rid, sid, &digest(&d, &b), &t, 120.0);
    let c = rpc(&d, Some(m), "team.token.consume", json!({"team_token": t}));
    assert_eq!(c["ok"], json!(true), "{c}");
    let s = rpc(&d, Some(m), "team.token.settle", json!({"team_token": t, "outcome": "failed", "code": 8}));
    assert_eq!(s["ok"], json!(true), "{s}");
    let x = reply(&d, Some(m), &rid, "allow", Some(&t));
    assert_eq!(code(&x), "grant_revoked", "{x}");
    assert_eq!(status(&d, &rid).0, "pending", "생성 실패인데 카드가 정리됐다(제안은 그대로 남아야 한다)");
}

#[test]
fn p5_ledger_corrupt_is_refused_not_authorized() {
    let m = 720_080;
    let (d, sid, rid, b) = proposal("tt-corrupt", m);
    let t = tok(80);
    issue(&d, &rid, sid, &digest(&d, &b), &t, 120.0);
    let p = ledger(&d);
    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    writeln!(f, "{{not json").unwrap();
    let c = rpc(&d, Some(m), "team.token.consume", json!({"team_token": t}));
    assert_eq!(code(&c), "ledger_corrupt", "{c}");
    assert_eq!(status(&d, &rid).0, "pending");
}

/// 다른 kind 는 `team_token` 인자를 보지 않는다(무회귀 대조군 — 인자가 붙어도 종전 흐름).
#[test]
fn p5_other_kinds_ignore_team_token() {
    let d = tmp_daemon("tt-other", false);
    let w = 720_090;
    let m = 720_091;
    seat(&d, "worker", w);
    seat(&d, "master", m);
    let req = Request {
        id: json!(1),
        method: "feed.push".into(),
        params: json!({"kind": "permission", "title": "t", "body": "b", "request_id": "p-tt-1",
                       "wait": false, "tier": "c"}),
    };
    let Reply::Single(r) = dispatch(&d, req, Some(w)) else { panic!("single") };
    assert_eq!(r["ok"], json!(true), "{r}");
    let x = reply(&d, Some(m), "p-tt-1", "allow", Some("garbage"));
    assert_eq!(x["ok"], json!(true), "다른 kind 의 승인이 team_token 때문에 깨졌다: {x}");
}
