//! ★U16(0.14.41) 팀 만들기 제안(`kind=team-create-request`) — 데몬 잠금 회귀 핀.
//!
//! 설계 정본: `_reports/수정설계-0.14.41-오너18항목-20260923.md` §3 U16 "데몬 잠금".
//!   ① 제안자 = **본부(base) 레인의 master 좌석**뿐(데몬이 커널 peer pid 로 각인한 발행 좌석의 역할).
//!   ② 같은 kind 의 대기(pending) 1건 · 24시간 3건.
//!   ③ 해소 = operator token(GUI) 뿐 — 예외는 **발행 좌석 자신의 `superseded`**(자기 제안 거두기) 하나.
//!   ④ 원격 미러 금지(tier 를 데몬이 d 로 고정) · `--wait` 금지(도구 시간초과가 카드를 timeout 으로
//!      소각하던 F5 경로) · CEO 자동결재 제외.
//!
//! ★정직한 한계(설계서 · 반박 R5): 이 잠금은 **사고 방지 층**이다. 같은 UID 의 프로세스가
//!   operator.token 파일을 읽어 raw RPC 를 보내면 ③을 지날 수 있다(M11 수준 — src-tauri
//!   feed_reply 주석과 같은 성격). "에이전트는 팀을 만들 수 없다"는 보안 경계 주장이 아니다.
//!
//! 이 파일은 **기존 공개 API 만** 쓴다(dispatch · create_surface · caller_cache) — 구현 전에도
//! 컴파일되어 RED 를 실제로 관측할 수 있게 하기 위함이다.
#![cfg(test)]

use crate::handlers::{dispatch, Reply};
use crate::state::Daemon;
use cys::Request;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;

const KIND: &str = "team-create-request";

fn tmp_daemon(tag: &str, dept_lane: bool) -> Arc<Daemon> {
    crate::delivery::tests::isolate_state_dir_for_thread(tag);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cys-u16-{tag}-{}-{}-{n}",
        std::process::id(),
        crate::state::now_epoch() as u64
    ));
    // 부서 레인 = 소켓 경로 컴포넌트에 `cys-dept-` 접두(cys::is_dept_socket 규약).
    let sock = if dept_lane {
        dir.join("cys-dept-dept-9").join("cys.sock")
    } else {
        dir.join("cysd.sock")
    };
    let _ = std::fs::create_dir_all(sock.parent().unwrap());
    Daemon::new(sock)
}

/// ★REVIEW1 m1(a): `tmp_daemon` 과 같지만 `CYS_APPROVE_AUTO_ROUTE=1` 로 Config 를 캡처한다
/// (Config::from_env 는 Daemon::new 안에서 한 번만 읽으므로 생성 **전**에 세워야 한다).
/// env 변수는 전역이라 `governance::PACK_ENV_LOCK`(handlers.rs 의 w3 계열 테스트와 같은 락)으로
/// 감싸 병행 테스트와의 경합을 막는다.
fn tmp_daemon_auto_route_on(tag: &str, dept_lane: bool) -> Arc<Daemon> {
    let _g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("CYS_APPROVE_AUTO_ROUTE", "1");
    let d = tmp_daemon(tag, dept_lane);
    std::env::remove_var("CYS_APPROVE_AUTO_ROUTE"); // config 캡처 후 즉시 정리
    d
}

/// 역할을 가진 좌석 1개 + 그 좌석에 귀속되는 synthetic 발신 pid.
fn seat(daemon: &Arc<Daemon>, role: &str, pid: u32) -> u64 {
    let s = daemon
        .create_surface(None, Some("sleep 30".into()), None, None, 24, 80)
        .expect("create seat");
    *s.role.lock().unwrap() = Some(role.into());
    daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    daemon.caller_cache.lock().unwrap().insert(
        pid,
        crate::state::CallerCacheEntry::new(
            Some(s.id),
            crate::state::now_epoch(),
            None,
            daemon.caller_gen.load(Ordering::Relaxed),
        ),
    );
    s.id
}

fn body(id: &str, display: &str, purpose: &str) -> String {
    json!({"v": 1, "id": id, "display": display, "purpose": purpose}).to_string()
}

fn push(daemon: &Arc<Daemon>, pid: Option<u32>, rid: &str, b: &str, extra: Value) -> Value {
    let mut params = json!({"kind": KIND, "title": "t", "body": b, "request_id": rid,
                            "wait": false, "tier": "d"});
    if let (Some(o), Some(e)) = (params.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    let req = Request { id: json!(1), method: "feed.push".into(), params };
    match dispatch(daemon, req, pid) {
        Reply::Single(v) => v,
        _ => panic!("feed.push 는 단일 응답이어야 한다(wait 금지)"),
    }
}

fn reply(daemon: &Arc<Daemon>, pid: Option<u32>, rid: &str, decision: &str, token: Option<&str>) -> Value {
    let mut params = json!({"request_id": rid, "decision": decision});
    if let Some(t) = token {
        params["operator_token"] = json!(t);
    }
    let req = Request { id: json!(2), method: "feed.reply".into(), params };
    match dispatch(daemon, req, pid) {
        Reply::Single(v) => v,
        _ => panic!("single"),
    }
}

fn status_of(daemon: &Arc<Daemon>, rid: &str) -> Option<String> {
    daemon
        .feed_items
        .lock()
        .unwrap()
        .iter()
        .find(|i| i.request_id == rid)
        .map(|i| i.status.clone())
}

fn count_kind(daemon: &Arc<Daemon>) -> usize {
    daemon.feed_items.lock().unwrap().iter().filter(|i| i.kind == KIND).count()
}

// ── ① 제안자 ─────────────────────────────────────────────────────────────

#[test]
fn u16_base_master_can_propose_and_tier_is_pinned_d() {
    let d = tmp_daemon("ok", false);
    seat(&d, "master", 710_001);
    // 클라이언트가 tier=a 를 주장해도 데몬이 d 로 고정해야 한다(원격 미러 금지).
    let r = push(&d, Some(710_001), "tp-ok-1", &body("tp-ok-1", "영상편집팀", "유튜브 영상 편집"),
                 json!({"tier": "a"}));
    assert_eq!(r["ok"], json!(true), "본부 master 의 정상 제안이 거부됐다: {r}");
    let items = d.feed_items.lock().unwrap();
    let it = items.iter().find(|i| i.request_id == "tp-ok-1").expect("항목");
    assert_eq!(it.tier.as_deref(), Some("d"), "팀 제안의 tier 는 데몬이 d 로 고정해야 한다(원격 미러 금지)");
    assert!(!it.auto_route, "팀 제안은 CEO 자동결재 대상이 아니다");
    assert_eq!(it.status, "pending");
}

/// ★REVIEW1 m1(a): 위 테스트(`…tier_is_pinned_d`)의 `assert!(!it.auto_route)` 는 테스트 데몬이
/// `approve_auto_route` 기본 OFF 라 공허하다(그 게이트 자체가 이미 auto_route 를 false 로 만든다 —
/// handlers.rs `auto_route = !team_proposal && daemon.config.approve_auto_route && …`에서
/// `!team_proposal` 항 두 곳(M-E 뮤테이션)을 지워도 이 조건이 여전히 OFF 로 막혀 안 걸린다).
/// 여기서는 **flag ON + AutoEligible 서술 + 발행자 귀속**까지 셋을 전부 참으로 만들어, 남은
/// 변수가 `team_proposal` 배제 그 자체뿐인 상태에서 대조한다 — `!team_proposal` 이 없으면
/// 이 테스트가 반드시 깨진다.
#[test]
fn u16_team_proposal_excluded_from_ceo_auto_route_even_when_flag_on() {
    let d = tmp_daemon_auto_route_on("autoon", false);
    seat(&d, "master", 710_005);
    // approval_risk::AUTO_MARKERS 의 "학습추천" — title_for(spec) 가 아니라 body(JSON 원문)에
    // 실려 derive_risk 가 AutoEligible 로 분류한다(정규화는 공백·중점만 벗기고 한글은 보존).
    let r = push(
        &d,
        Some(710_005),
        "tp-auto-1",
        &body("tp-auto-1", "학습팀", "학습추천 콘텐츠를 만든다"),
        json!({}),
    );
    assert_eq!(r["ok"], json!(true), "본부 master 의 정상 제안이 거부됐다: {r}");
    let items = d.feed_items.lock().unwrap();
    let it = items.iter().find(|i| i.request_id == "tp-auto-1").expect("항목");
    assert_eq!(it.risk_class.as_deref(), Some("auto"), "AutoEligible 서술이 auto 로 안 분류됐다(계측 무효)");
    assert!(
        !it.auto_route,
        "flag ON + AutoEligible 인데도 team_proposal 은 CEO 자동결재 대상이면 안 된다(M-E 뮤테이션 생존 지점)"
    );
    assert_eq!(it.status, "pending");
}

#[test]
fn u16_non_master_or_unattributed_or_dept_lane_cannot_propose() {
    // (a) 발행 좌석 미상(pane 밖 프로세스) — 각인 없음
    let d = tmp_daemon("anon", false);
    let r = push(&d, None, "tp-anon", &body("tp-anon", "팀", "일"), json!({}));
    assert_eq!(r["ok"], json!(false), "귀속 없는 발행이 통과했다: {r}");
    assert_eq!(count_kind(&d), 0, "거부된 제안이 항목을 남겼다(부작용 0 위반)");

    // (b) 워커 좌석
    let d = tmp_daemon("worker", false);
    seat(&d, "worker", 710_002);
    let r = push(&d, Some(710_002), "tp-w", &body("tp-w", "팀", "일"), json!({}));
    assert_eq!(r["ok"], json!(false), "워커 좌석의 제안이 통과했다: {r}");
    assert_eq!(count_kind(&d), 0);

    // (c) CSO 좌석(feed push 권한은 있으나 제안자는 아니다)
    let d = tmp_daemon("cso", false);
    seat(&d, "cso", 710_003);
    let r = push(&d, Some(710_003), "tp-c", &body("tp-c", "팀", "일"), json!({}));
    assert_eq!(r["ok"], json!(false), "CSO 좌석의 제안이 통과했다: {r}");

    // (d) 부서 레인의 master(=팀장) — 팀 안에서 팀을 낳지 않는다
    let d = tmp_daemon("deptm", true);
    seat(&d, "master", 710_004);
    let r = push(&d, Some(710_004), "tp-dm", &body("tp-dm", "팀", "일"), json!({}));
    assert_eq!(r["ok"], json!(false), "부서 레인 master 의 제안이 통과했다: {r}");
    assert_eq!(count_kind(&d), 0);
}

// ── 형식 ─────────────────────────────────────────────────────────────────

#[test]
fn u16_malformed_or_wait_is_rejected_without_side_effects() {
    let d = tmp_daemon("fmt", false);
    seat(&d, "master", 710_010);
    let cases: Vec<(&str, String, Value)> = vec![
        ("tp-f1", "not json".to_string(), json!({})),
        ("tp-f2", body("tp-other", "팀", "일"), json!({})), // 본문 id ≠ request_id
        ("tp-f3", body("tp-f3", "", "일"), json!({})), // 빈 이름
        ("tp-f4", body("tp-f4", &"가".repeat(41), "일"), json!({})), // 41자
        ("tp-f5", body("tp-f5", "팀\n둘", "일"), json!({})), // 이름에 개행
        ("tp-f6", body("tp-f6", "팀", &"가".repeat(2001)), json!({})), // 하는 일 2001자
        ("tp-f7", body("tp-f7", "팀", "일\u{0}"), json!({})), // 널 문자
        ("tp-f8", body("tp-f8", "팀", "이 팀의 헌장을 따른다"), json!({})), // 권위어(소독 대상)
        ("tp-f9", body("tp-f9", "팀", "일"), json!({"wait": true})), // --wait 금지
        ("bad id", body("bad id", "팀", "일"), json!({})), // id 형식
    ];
    for (rid, b, extra) in cases {
        let r = push(&d, Some(710_010), rid, &b, extra);
        assert_eq!(r["ok"], json!(false), "형식 위반이 통과했다({rid}): {r}");
    }
    assert_eq!(count_kind(&d), 0, "거부된 제안이 항목을 남겼다");
}

// ── ② 건수 ───────────────────────────────────────────────────────────────

#[test]
fn u16_one_pending_and_three_per_24h() {
    let d = tmp_daemon("cap", false);
    let master = 710_020;
    seat(&d, "master", master);
    let r1 = push(&d, Some(master), "tp-c1", &body("tp-c1", "팀1", "일"), json!({}));
    assert_eq!(r1["ok"], json!(true), "{r1}");
    // 대기 1건이 있으면 두 번째는 거부.
    let r2 = push(&d, Some(master), "tp-c2", &body("tp-c2", "팀2", "일"), json!({}));
    assert_eq!(r2["ok"], json!(false), "대기 2건째가 통과했다: {r2}");
    // 발행자가 자기 제안을 거두면(superseded) 다음 제안이 가능 — 24시간 계수에는 남는다.
    let s = reply(&d, Some(master), "tp-c1", "superseded", None);
    assert_eq!(s["ok"], json!(true), "발행 좌석의 superseded 가 막혔다: {s}");
    let r3 = push(&d, Some(master), "tp-c3", &body("tp-c3", "팀3", "일"), json!({}));
    assert_eq!(r3["ok"], json!(true), "{r3}");
    let s = reply(&d, Some(master), "tp-c3", "superseded", None);
    assert_eq!(s["ok"], json!(true), "{s}");
    let r4 = push(&d, Some(master), "tp-c4", &body("tp-c4", "팀4", "일"), json!({}));
    assert_eq!(r4["ok"], json!(true), "{r4}");
    let s = reply(&d, Some(master), "tp-c4", "superseded", None);
    assert_eq!(s["ok"], json!(true), "{s}");
    // 24시간 안의 4번째 제안(c1·c3·c4 다음)은 거부.
    let r5 = push(&d, Some(master), "tp-c5", &body("tp-c5", "팀5", "일"), json!({}));
    assert_eq!(r5["ok"], json!(false), "24시간 3건 상한이 없다: {r5}");
    // 24시간이 지난 항목은 계수에서 빠진다.
    for it in d.feed_items.lock().unwrap().iter_mut() {
        if it.kind == KIND {
            it.created_at -= 86_401.0;
        }
    }
    let r6 = push(&d, Some(master), "tp-c6", &body("tp-c6", "팀6", "일"), json!({}));
    assert_eq!(r6["ok"], json!(true), "24시간이 지나도 계수가 남는다: {r6}");
}

// ── ③ 해소 ───────────────────────────────────────────────────────────────

#[test]
fn u16_only_operator_token_resolves_except_publisher_superseded() {
    let d = tmp_daemon("rep", false);
    let master = 710_030;
    let worker = 710_031;
    seat(&d, "master", master);
    seat(&d, "worker", worker);
    let r = push(&d, Some(master), "tp-r1", &body("tp-r1", "팀", "일"), json!({}));
    assert_eq!(r["ok"], json!(true), "{r}");

    // 발행자 자신: deny·yes·approve·자유문구 전부 거부(반박 M3 — 구독 흐름이 카드를 소각하던 경로).
    for dec in ["deny", "yes", "approve", "ok 만들어", "allow", "dismissed"] {
        let x = reply(&d, Some(master), "tp-r1", dec, None);
        assert_eq!(x["ok"], json!(false), "발행자의 '{dec}' 가 해소됐다: {x}");
    }
    // 다른 좌석(워커): allow 포함 어떤 결정도 거부.
    for dec in ["allow", "deny", "superseded"] {
        let x = reply(&d, Some(worker), "tp-r1", dec, None);
        assert_eq!(x["ok"], json!(false), "다른 좌석의 '{dec}' 가 해소됐다: {x}");
    }
    // 귀속 없는 외부 프로세스의 deny 도 거부.
    let x = reply(&d, None, "tp-r1", "deny", None);
    assert_eq!(x["ok"], json!(false), "외부 프로세스의 deny 가 해소됐다: {x}");
    // 틀린 토큰도 거부.
    let x = reply(&d, None, "tp-r1", "allow", Some("wrong-token"));
    assert_eq!(x["ok"], json!(false), "틀린 operator token 이 통과했다: {x}");
    assert_eq!(status_of(&d, "tp-r1").as_deref(), Some("pending"), "카드가 오너 확인 전에 사라졌다");

    // 오너 GUI(operator token): 해소 가능.
    let tok = d.operator_token.clone().expect("테스트 데몬은 operator token 을 발급한다");
    let x = reply(&d, None, "tp-r1", "allow", Some(&tok));
    assert_eq!(x["ok"], json!(true), "operator token allow 가 막혔다: {x}");
    assert_eq!(status_of(&d, "tp-r1").as_deref(), Some("resolved"));

    // 오너 GUI 의 deny(만들지 않기)도 해소 가능.
    let r = push(&d, Some(master), "tp-r2", &body("tp-r2", "팀2", "일"), json!({}));
    assert_eq!(r["ok"], json!(true), "{r}");
    let x = reply(&d, None, "tp-r2", "deny", Some(&tok));
    assert_eq!(x["ok"], json!(true), "operator token deny 가 막혔다: {x}");
}

/// 다른 kind 는 이 잠금의 영향을 받지 않는다(무회귀 대조군).
#[test]
fn u16_other_kinds_unaffected() {
    let d = tmp_daemon("other", false);
    let worker = 710_040;
    let master = 710_041;
    seat(&d, "worker", worker);
    seat(&d, "master", master);
    let req = Request {
        id: json!(1),
        method: "feed.push".into(),
        params: json!({"kind": "permission", "title": "t", "body": "not json", "request_id": "p-1",
                       "wait": false, "tier": "c"}),
    };
    let Reply::Single(r) = dispatch(&d, req, Some(worker)) else { panic!("single") };
    assert_eq!(r["ok"], json!(true), "{r}");
    let x = reply(&d, Some(master), "p-1", "allow", None);
    assert_eq!(x["ok"], json!(true), "다른 좌석의 일반 승인 흐름이 깨졌다: {x}");
    let items = d.feed_items.lock().unwrap();
    assert_eq!(items.iter().find(|i| i.request_id == "p-1").unwrap().tier.as_deref(), Some("c"));
}

/// ★REVIEW1 m1(b): CLI(`cys team-propose`)가 부르는 `cys::team_spec::team_gate_ok` 를 **실제
/// 데몬 응답 모양**(이 파일의 `dispatch` 경로 — handlers.rs feed.push 가 만드는 그 JSON)과
/// 맞대 본다. `src/bin/cys.rs` 의 `rpc_roundtrip` 은 `resp["result"]` 를 벗겨 돌려주므로,
/// CLI 가 실제로 보는 것은 `["result"]` 안쪽이다 — 그 껍질을 그대로 재현한다. handlers.rs 가
/// `team_gate: 1` 표지를 빠뜨리는 회귀(뮤테이션 M-G)가 나면 이 테스트가 깨진다(종전에는
/// `cys.rs` 쪽 파싱 테스트만 있어 이 표지 자체를 잰 적이 없었다 — REVIEW1 뮤테이션 생존).
#[test]
fn u16_daemon_response_shape_satisfies_cli_team_gate_ok() {
    let d = tmp_daemon("gate-shape", false);
    seat(&d, "master", 710_050);
    let r = push(&d, Some(710_050), "tp-shape-1", &body("tp-shape-1", "팀", "일"), json!({}));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert!(
        cys::team_spec::team_gate_ok(&r["result"]),
        "실제 데몬 응답이 CLI 의 team_gate_ok 를 통과 못 한다(표지 소실): {r}"
    );
    // 대조군: kind 가 team-create-request 가 아니면 이 표지가 없다(다른 kind 무변경 확인).
    let worker = 710_051;
    seat(&d, "worker", worker);
    let req = Request {
        id: json!(9),
        method: "feed.push".into(),
        params: json!({"kind": "permission", "title": "t", "body": "b", "request_id": "p-shape-1",
                       "wait": false, "tier": "c"}),
    };
    let Reply::Single(other) = dispatch(&d, req, Some(worker)) else { panic!("single") };
    assert_eq!(other["ok"], json!(true), "{other}");
    assert!(
        !cys::team_spec::team_gate_ok(&other["result"]),
        "team-create-request 가 아닌 kind 에도 team_gate 표지가 붙었다: {other}"
    );
}
