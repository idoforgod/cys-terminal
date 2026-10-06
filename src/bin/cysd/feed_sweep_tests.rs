//! ★(0.14.44 · C1·C3) 승인 Feed — 닫힌 좌석의 화면 감지 승인 쓸기(C1) · `feed.list` 파생 칸(C3)의 데몬 쪽 시험.
#![cfg(test)]

use crate::handlers::{dispatch, Reply};
use crate::state::Daemon;
use cys::Request;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn status_of(d: &Arc<Daemon>, rid: &str) -> Option<(String, Option<String>)> {
    d.feed_items
        .lock()
        .unwrap()
        .iter()
        .find(|i| i.request_id == rid)
        .map(|i| (i.status.clone(), i.decision.clone()))
}

/// 데몬이 올린 항목 하나를 만들고 request_id 를 돌려준다(`push_feed_notification` 은 id 를 돌려주지 않으므로 마지막 항목을 읽는다).
fn daemon_item(d: &Arc<Daemon>, kind: &str, sid: Option<u64>) -> String {
    d.push_feed_notification(kind, "t", "b", sid);
    let items = d.feed_items.lock().unwrap();
    items.last().map(|i| i.request_id.clone()).expect("항목")
}

fn client_push(d: &Arc<Daemon>, pid: Option<u32>, rid: &str, kind: &str, sid: Option<u64>) -> Value {
    let mut params = json!({"kind": kind, "title": "t", "body": "b", "request_id": rid, "wait": false});
    if let Some(s) = sid {
        params["surface_id"] = json!(s);
    }
    let req = Request { id: json!(1), method: "feed.push".into(), params };
    match dispatch(d, req, pid) {
        Reply::Single(v) => v,
        _ => panic!("single"),
    }
}

#[test]
fn c1_sweeps_only_daemon_issued_approval_and_gate_items_of_closed_seats() {
    let d = crate::team_gate_tests::tmp_daemon("c1-sweep", false);
    let alive = crate::team_gate_tests::seat(&d, "worker", 990_001);
    let exited = crate::team_gate_tests::seat(&d, "worker", 990_002);
    d.surfaces.lock().unwrap().get(&exited).expect("seat").exited.store(true, Ordering::Relaxed);
    let gone: u64 = 99_999;
    // 데몬이 올린 항목 — 좌석 없음 · 종료 · 살아 있음 · 좌석 번호 없음 (approval) + 관문(first_run_gate).
    let a_gone = daemon_item(&d, "approval", Some(gone));
    let a_exited = daemon_item(&d, "approval", Some(exited));
    let a_alive = daemon_item(&d, "approval", Some(alive));
    let a_nosid = daemon_item(&d, "approval", None);
    let g_gone = daemon_item(&d, crate::governance::GATE_FEED_KIND, Some(gone));
    // 데몬이 올렸지만 종류가 다른 것(정보성 알림 · 부트 실패 보고)은 쓸지 않는다.
    let n_gone = daemon_item(&d, "notification", Some(gone));
    let w_gone = daemon_item(&d, "warning", Some(gone));
    // 클라이언트가 올린 항목(어느 종류든) — 좌석이 없어도 쓸지 않는다.
    let kinds = ["approval", "permission", "question", "first_run_gate", "cycle-verify", "learn_proposal", "team-create-request", "notification"];
    for (i, k) in kinds.iter().enumerate() {
        // `daemon-` 접두는 클라이언트가 쓸 수 없다 — 일반 id 로 올린다.
        let r = client_push(&d, None, &format!("cli-{i}"), k, Some(gone));
        assert!(r["ok"] == json!(true) || r["error"].is_object(), "{k}: {r}");
    }
    let n = crate::governance::sweep_orphan_daemon_approvals_inner(&d);
    assert_eq!(n, 3, "좌석 없음 · 종료 · 관문(좌석 없음)만 닫힌다");
    for rid in [&a_gone, &a_exited, &g_gone] {
        assert_eq!(status_of(&d, rid), Some(("resolved".into(), Some("stale-cleared".into()))), "{rid}");
    }
    for rid in [&a_alive, &a_nosid, &n_gone, &w_gone] {
        assert_eq!(status_of(&d, rid).map(|s| s.0), Some("pending".into()), "{rid} 가 닫혔다");
    }
    // 클라이언트 항목은 전부 그대로(올라간 것만).
    for (i, k) in kinds.iter().enumerate() {
        if let Some((st, _)) = status_of(&d, &format!("cli-{i}")) {
            assert_eq!(st, "pending", "클라이언트 항목 {k} 가 닫혔다");
        }
    }
    // 멱등 — 두 번째 쓸기는 0건.
    assert_eq!(crate::governance::sweep_orphan_daemon_approvals_inner(&d), 0);
}

#[test]
fn c1_knob_pure_function_env_and_policy_zero_turn_it_off_and_only_zero() {
    use crate::knobs::feed_orphan_sweep_enabled_from as on;
    assert!(on(None, None));
    assert!(!on(Some("0"), None));
    assert!(!on(Some(" 0 "), None));
    assert!(on(Some("1"), None));
    assert!(on(Some("off"), None), "0 이 아닌 값은 켜는 쪽 — 되돌리는 값은 0 하나");
    assert!(!on(None, Some(&json!({"CYS_FEED_ORPHAN_SWEEP": 0}))));
    assert!(!on(None, Some(&json!({"CYS_FEED_ORPHAN_SWEEP": false}))));
    assert!(!on(None, Some(&json!({"CYS_FEED_ORPHAN_SWEEP": "0"}))));
    assert!(on(None, Some(&json!({"CYS_FEED_ORPHAN_SWEEP": 1}))), "켜는 값은 파일에서 받지 않는다(기본이 켬이므로 변화 없음)");
    assert!(on(None, Some(&json!({"other": 0}))));
}

/// 소스 고정 — 쓸기는 화면 감지 함수(`check_approvals`) **밖**의 별도 함수이고(에이전트 정의 파일을 읽지 못해도 돈다) · `.await` 가 없고 · 색인 접근·`unwrap`·`expect` 가 없다.
#[test]
fn c1_sweep_is_a_separate_tick_function_without_await_or_panics() {
    let src = include_str!("governance.rs");
    let prod = &src[..src.find("\n#[cfg(test)]").expect("테스트 앵커")];
    let ca = prod.find("fn check_approvals(").expect("check_approvals");
    let ca_body = &prod[ca..ca + prod[ca..].find("\n}\n").expect("끝")];
    assert!(!ca_body.contains("sweep_orphan_daemon_approvals"), "쓸기가 화면 감지 함수 안으로 들어갔다");
    let a = prod.find("fn sweep_orphan_daemon_approvals(").expect("쓸기 함수");
    let b = prod.find("pub(crate) fn sweep_orphan_daemon_approvals_inner(").expect("본체");
    let body = &prod[a..b + prod[b..].find("\n}\n").expect("끝")];
    assert!(!body.contains(".await"), "틱 안의 기다림");
    assert!(!body.contains(".unwrap()") && !body.contains(".expect("), "패닉 경로");
    assert!(body.contains("#[deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]"));
    // 틱 배선: 15초 틱 블록 안, check_approvals 바로 뒤.
    let tick = prod.find("check_approvals(&daemon, &mut approval_debounce, &mut scan_caches);").expect("틱 호출");
    assert!(prod[tick..tick + 400].contains("sweep_orphan_daemon_approvals(&daemon);"));
    // 쓸기 본체에서 잠금 둘을 겹쳐 쥐지 않는다 — `feed_items` 와 `surfaces` 잠금이 각각 한 문장에서 끝난다(`let` 로 가드를 묶어 두는 꼴이 없다).
    assert!(!body.contains("let _g") && !body.contains("let g ="), "잠금 가드를 변수로 쥔다");
}
