//! ★A2(0.14.42 · WP-delivery) 직접 `send` + `send-key Return` 쌍의 의미 정합 — RPC 수준 회귀 핀.
//!
//! 설계 정본: WP-delivery 최종 설계 A2(`return_absorb_verdict` · `ticket_after_key_write` · D0~D5).
//! 원칙 한 줄: **흡수는 기계 본문을 제출하지 않을 Return 에만** 적용한다(queued Return · 빈 줄 ·
//! 사람 초안 위). 기계 본문(pending>0 ∧ human==0) 위 Return 은 누구 것이든 종전처럼 통과한다.
//!
//! 관측 축: 핸들러가 PTY 쓰기를 writer 에 넘기면 `apply_pending_input` 이 **반드시** 변이 세대
//! (`input_gen`)를 올린다 — 그래서 "세대 불변" 은 "이 요청이 아무것도 쓰지 않았다" 의 결정론 증거다
//! (writer 채널을 가로채지 않고도 쓰기 0 을 판정한다).
//!
//! 이 파일의 RPC 검체는 **기존 공개 API 만** 쓴다(dispatch · create_surface · caller_cache) —
//! 구현 전에도 컴파일되어 RED 를 실제로 관측할 수 있게 하기 위함이다(team_gate_tests 와 같은 규약).
//! 표(`return_tickets`) 직접 관측은 파일 끝 `ticket_observation` 절에만 둔다.
#![cfg(test)]

use crate::handlers::{dispatch, Reply};
use crate::state::{Daemon, Surface};
use cys::Request;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard};

/// 검체 1개의 격리 환경 — pack(acl.json 전부 허용)·원장 상태 디렉터리·env 락을 한 수명으로 묶는다.
struct Fx {
    daemon: Arc<Daemon>,
    dir: std::path::PathBuf,
    _g: MutexGuard<'static, ()>,
}

impl Drop for Fx {
    fn drop(&mut self) {
        for s in self.daemon.surfaces.lock().unwrap().values() {
            let mut child = s.child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
        }
        std::env::remove_var(cys::pack::ENV_PACK_DIR);
        std::env::remove_var("CYS_RETURN_ABSORB_SECS");
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fx(tag: &str) -> Fx {
    // CYS_PACK_DIR 는 프로세스 전역이다 — handlers ACL 검체·governance 큐 검체와 **같은 락**.
    let g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::delivery::tests::isolate_state_dir_for_thread(tag);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cys-a2-{tag}-{}-{}-{n}",
        std::process::id(),
        crate::state::now_epoch() as u64
    ));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
    std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);
    std::env::remove_var("CYS_RETURN_ABSORB_SECS");
    let daemon = Daemon::new(dir.join("cysd.sock"));
    Fx { daemon, dir, _g: g }
}

/// 좌석 1개 + 그 좌석으로 해석되는 synthetic 발신 pid(커널 peer pid 대역).
fn pane(fx: &Fx, role: &str, pid: u32) -> Arc<Surface> {
    let s = fx
        .daemon
        .create_surface(None, Some("sleep 30".into()), None, Some(role.into()), 24, 80)
        .expect("create surface");
    fx.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    bind(fx, pid, s.id);
    s
}

fn bind(fx: &Fx, pid: u32, sid: u64) {
    fx.daemon.caller_cache.lock().unwrap().insert(
        pid,
        crate::state::CallerCacheEntry::new(
            Some(sid),
            crate::state::now_epoch(),
            None,
            fx.daemon.caller_gen.load(Ordering::Relaxed),
        ),
    );
}

fn rpc(fx: &Fx, pid: Option<u32>, method: &str, params: Value) -> Value {
    let req = Request { id: json!(42), method: method.into(), params };
    let Reply::Single(resp) = dispatch(&fx.daemon, req, pid) else {
        panic!("expected single reply");
    };
    resp
}

/// (미러 계수, 사람 계수, 변이 세대) — 세대 불변 = 쓰기 0.
fn counts(s: &Arc<Surface>) -> (u64, u64, u64) {
    let human = s.pending_input.lock().unwrap().human;
    (
        s.pending_input_bytes.load(Ordering::Relaxed),
        human,
        s.input_gen.load(Ordering::Acquire),
    )
}

fn qlen(s: &Arc<Surface>) -> usize {
    s.pending_queue.lock().unwrap().len()
}

fn typing_on(s: &Arc<Surface>) {
    *s.last_human_input.lock().unwrap() = Some(std::time::Instant::now());
}

fn typing_off(s: &Arc<Surface>) {
    *s.last_human_input.lock().unwrap() = None;
}

fn direct(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>, text: &str) -> Value {
    rpc(fx, pid, "surface.send_text", json!({
        "surface_id": t.id, "text": text, "queued": false, "quiet": true,
    }))
}

/// `cys send` 의 B3 폴백을 그대로 흉내 낸다 — 직접 전송이 타이핑 가드(초안 게이트 포함)로 거부되면
/// `queued:true` 로 1회 재요청한다. 신 CLI 는 그 재요청에 `absorb_return:true` 를 싣는다.
/// 반환 = 재요청 응답(직접 전송이 통과했으면 패닉 — 검체 전제 위반).
fn send_fallback(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>, text: &str) -> Value {
    let first = direct(fx, pid, t, text);
    assert_eq!(first["ok"], json!(false), "전제: 직접 전송이 거부돼야 폴백이 일어난다: {first}");
    let msg = first["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains(cys::MSG_TYPING_GUARD), "전제: 폴백 대상 거부(타이핑 가드 문구): {first}");
    let r2 = rpc(fx, pid, "surface.send_text", json!({
        "surface_id": t.id, "text": text, "queued": true, "absorb_return": true, "quiet": true,
    }));
    assert_eq!(r2["ok"], json!(true), "큐 전환은 성공해야 한다: {r2}");
    r2
}

/// 신 CLI 의 단일 `send-key Return` — `pair_return:true`.
fn pair_return(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>) -> Value {
    rpc(fx, pid, "surface.send_key", json!({
        "surface_id": t.id, "key": "Return", "queued": false, "pair_return": true,
    }))
}

fn bus_count(fx: &Fx, name: &str) -> usize {
    fx.daemon.bus.tail(200).iter().filter(|e| e["name"] == name).count()
}

fn bus_last(fx: &Fx, name: &str) -> Option<Value> {
    fx.daemon.bus.tail(200).into_iter().filter(|e| e["name"] == name).last()
}

fn assert_absorbed(resp: &Value, kind: &str, body_state: &str) {
    assert_eq!(resp["ok"], json!(true), "흡수는 성공 응답이다(rc 0): {resp}");
    let r = &resp["result"];
    assert_eq!(r["absorbed"], json!(true), "흡수돼야 한다: {resp}");
    assert_eq!(r["sent"], json!(false), "흡수는 쓰지 않았다는 사실을 싣는다: {resp}");
    assert_eq!(r["absorb_kind"], json!(kind), "흡수 종류: {resp}");
    assert_eq!(r["body_state"], json!(body_state), "본문 상태: {resp}");
    assert!(r.get("queued").is_none(), "흡수 응답에 queued 키 금지(QUEUED 오독 방지): {resp}");
    assert!(r["ticket_age_ms"].is_u64(), "ticket_age_ms 관측값: {resp}");
}

fn assert_sent(resp: &Value) {
    assert_eq!(resp["ok"], json!(true), "종전 경로로 기록돼야 한다: {resp}");
    assert_eq!(resp["result"]["sent"], json!(true), "직접 기록(sent:true): {resp}");
    assert_ne!(resp["result"]["absorbed"], json!(true), "흡수되면 안 된다: {resp}");
}

const P: u32 = 996_100;

// ─────────────────────────── 적색→녹색 ───────────────────────────

/// 빈 줄 위 짝 Return 은 흡수 — 쓰기 0 · 계수 불변 · 큐 불변(빈 항목 0).
/// 종전: 타이핑 가드로 거부 → CLI 가 빈 큐 항목(text="")을 쌓았다(RC1 (b)) 또는 맨 CR(RC1 (c)).
#[test]
fn a2_pair_return_on_empty_line_absorbed() {
    let fx = fx("empty");
    let t = pane(&fx, "worker-1", P);
    let _x = pane(&fx, "worker-2", P + 1);
    typing_on(&t);
    let r2 = send_fallback(&fx, Some(P + 1), &t, "[보고] 완료");
    let before = counts(&t);
    let resp = pair_return(&fx, Some(P + 1), &t);
    let after = counts(&t);

    assert_absorbed(&resp, "pair", "queued");
    assert_eq!(resp["result"]["queue_entry_id"], r2["result"]["queue_entry_id"], "짝 본문 id: {resp}");
    assert_eq!(resp["result"]["depth"], json!(1));
    assert_eq!(after, before, "흡수는 쓰기·계수 변경 0(세대 불변)");
    assert_eq!(qlen(&t), 1, "흡수는 적재하지 않는다(빈 send-key 항목 0)");
    assert_eq!(r2["result"]["return_absorb"], json!(true), "표 발급 응답: {r2}");
    assert_eq!(r2["result"]["return_absorb_secs"], json!(30));
    assert_eq!(bus_count(&fx, "queue.return_absorbed"), 1, "흡수 이벤트 1건");
    let ev = bus_last(&fx, "queue.return_absorbed").unwrap();
    assert!(ev["payload"]["ticket_age_ms"].is_u64(), "운영 측정 축: {ev}");
}

/// 남의 기계 본문 위 짝 Return 은 **통과**(종전과 같다 — 본문 제출) · 본문 주인에게 보상 표.
/// 이어지는 본문 주인의 짝 Return 은 보상 표로 흡수 — 맨 CR 0(가로채기 연쇄 차단).
#[test]
fn a2_pair_return_over_other_body_passes_and_compensates() {
    let fx = fx("compensate");
    let t = pane(&fx, "worker-1", P + 10);
    let _x = pane(&fx, "worker-2", P + 11);
    let _y = pane(&fx, "worker-3", P + 12);
    assert_eq!(direct(&fx, Some(P + 12), &t, "yyy")["ok"], json!(true));
    assert_eq!(counts(&t).0, 3);
    send_fallback(&fx, Some(P + 11), &t, "xxx");
    let x_ret = pair_return(&fx, Some(P + 11), &t);
    assert_sent(&x_ret);
    assert_eq!(counts(&t).0, 0, "X 의 Return 이 Y 본문을 제출(종전과 같음)");
    let before = counts(&t);
    let y_ret = pair_return(&fx, Some(P + 12), &t);
    assert_absorbed(&y_ret, "compensation", "submitted");
    assert_eq!(counts(&t), before, "Y 의 짝 Return 은 쓰지 않는다(맨 CR 0)");
    assert_eq!(qlen(&t), 1, "X 본문은 큐에 그대로(CR 포함 배달 대기)");
}

/// 순차 대조(치명 음성): 자기 본문 A 가 줄에 있으면 자기 짝 Return 은 **절대** 흡수되지 않는다.
#[test]
fn a2_own_body_sequential_never_absorbed() {
    let fx = fx("own-seq");
    let t = pane(&fx, "worker-1", P + 20);
    let _x = pane(&fx, "worker-2", P + 21);
    assert_eq!(direct(&fx, Some(P + 21), &t, "AAA")["ok"], json!(true));
    send_fallback(&fx, Some(P + 21), &t, "BBB");
    let r = pair_return(&fx, Some(P + 21), &t);
    assert_sent(&r);
    assert_eq!(counts(&t).0, 0, "A 가 제출됐다");
    assert_eq!(qlen(&t), 1, "B 는 큐에(CR 포함 배달)");
    // 설계 D4: 소유자==발신자면 표 유지 — 같은 창 형제 프로세스의 짝 Return 용.
    let again = pair_return(&fx, Some(P + 21), &t);
    assert_absorbed(&again, "pair", "queued");
}

/// 병행 대조(치명 음성): 같은 조상 창 X 의 두 프로세스 P1·P2.
#[test]
fn a2_sibling_same_pane() {
    let fx = fx("sibling");
    let t = pane(&fx, "worker-1", P + 30);
    let x = pane(&fx, "worker-2", P + 31);
    bind(&fx, P + 32, x.id); // P2 — 같은 창의 형제 프로세스
    assert_eq!(direct(&fx, Some(P + 31), &t, "AAA")["ok"], json!(true));
    send_fallback(&fx, Some(P + 32), &t, "BBB");
    assert_sent(&pair_return(&fx, Some(P + 31), &t));
    assert_eq!(counts(&t).0, 0, "P1 Return 이 A 를 제출");
    let before = counts(&t);
    let p2 = pair_return(&fx, Some(P + 32), &t);
    assert_absorbed(&p2, "pair", "queued");
    assert_eq!(counts(&t), before);

    // 교차 잔여(수용): P2 폴백 → P1 직접(D2 소거) → P1 Return 통과 → P2 Return 은 비흡수(종전 동작).
    let t2 = pane(&fx, "worker-3", P + 33);
    typing_on(&t2);
    send_fallback(&fx, Some(P + 32), &t2, "B2");
    typing_off(&t2);
    assert_eq!(direct(&fx, Some(P + 31), &t2, "A2")["ok"], json!(true));
    assert_sent(&pair_return(&fx, Some(P + 31), &t2));
    assert_sent(&pair_return(&fx, Some(P + 32), &t2));
}

/// 구조 핀: 죽은 발신자 Z 의 본문이 줄에 묶여 있어도 X 의 짝 Return 이 **즉시** 제출하고,
/// X 본문은 대기 없이 배출 가능하다(원 A2 의 'TTL 뒤 재도착 필요' 결함 부재).
#[test]
fn a2_stuck_dead_sender_body_rescued_immediately() {
    let fx = fx("rescue");
    let t = pane(&fx, "worker-1", P + 40);
    let _x = pane(&fx, "worker-2", P + 41);
    let _z = pane(&fx, "worker-3", P + 42);
    assert_eq!(direct(&fx, Some(P + 42), &t, "ZZZ")["ok"], json!(true));
    send_fallback(&fx, Some(P + 41), &t, "xxx");
    assert_sent(&pair_return(&fx, Some(P + 41), &t));
    assert_eq!(counts(&t).0, 0);
    assert!(deliver_now(&fx, &t), "X 본문은 대기 없이 배출돼야 한다");
    assert_eq!(qlen(&t), 0);

    // owner 결측(미검증 발신자 본문) 변형 — 같은 결과.
    let t2 = pane(&fx, "worker-4", P + 43);
    assert_eq!(direct(&fx, None, &t2, "QQQ")["ok"], json!(true));
    send_fallback(&fx, Some(P + 41), &t2, "xxx");
    assert_sent(&pair_return(&fx, Some(P + 41), &t2));
    assert_eq!(counts(&t2).0, 0);
    assert!(deliver_now(&fx, &t2));
    assert_eq!(qlen(&t2), 0);
}

fn deliver_now(fx: &Fx, t: &Arc<Surface>) -> bool {
    for _ in 0..40 {
        if crate::governance::deliver_head_locked(
            &fx.daemon, t, false, false, None, Some(0), None, None,
        )
        .is_some()
        {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

/// queued 짝 Return(CLI 폴백 r2 · 명시 --queued)도 흡수 — 빈 항목 0(종전 +1).
#[test]
fn a2_queued_pair_return_absorbed_no_empty_item() {
    let fx = fx("queued-pair");
    let t = pane(&fx, "worker-1", P + 50);
    let _x = pane(&fx, "worker-2", P + 51);
    typing_on(&t);
    send_fallback(&fx, Some(P + 51), &t, "[보고] 본문");
    let resp = rpc(&fx, Some(P + 51), "surface.send_key", json!({
        "surface_id": t.id, "key": "Return", "queued": true, "pair_return": true,
    }));
    assert_absorbed(&resp, "pair", "queued");
    assert_eq!(qlen(&t), 1, "빈 send-key 항목을 쌓지 않는다");
}

/// 본문이 마지막 슬롯(100번째)을 차지한 뒤의 queued 짝 Return 은 queue_full 이 아니라 흡수(거짓 실패 0).
#[test]
fn a2_absorbed_return_never_hits_queue_full() {
    let fx = fx("full");
    let t = pane(&fx, "worker-1", P + 60);
    let _x = pane(&fx, "worker-2", P + 61);
    for i in 0..99 {
        let e = fx.daemon.next_queue_entry(format!("z{i}"), Some("surface:77".into()), "send");
        t.pending_queue.lock().unwrap().push_back(e);
    }
    typing_on(&t);
    let r2 = send_fallback(&fx, Some(P + 61), &t, "[보고] 100번째");
    assert_eq!(r2["result"]["depth"], json!(100));
    let resp = rpc(&fx, Some(P + 61), "surface.send_key", json!({
        "surface_id": t.id, "key": "Return", "queued": true, "pair_return": true,
    }));
    assert_absorbed(&resp, "pair", "queued");
    assert_eq!(qlen(&t), 100, "상한 불변 · 흡수는 적재하지 않는다");
}

/// 사람 초안 위 짝 Return 은 흡수 — 쓰기 0 · 빈 항목 0(종전: D-12/타이핑 가드 거부 → 빈 항목).
#[test]
fn a2_human_draft_pair_return_absorbed() {
    let fx = fx("human-draft");
    let t = pane(&fx, "worker-1", P + 70);
    let _x = pane(&fx, "worker-2", P + 71);
    let r = rpc(&fx, Some(P + 70), "surface.send_text", json!({
        "surface_id": t.id, "text": "abc", "human": true, "quiet": true,
    }));
    assert_eq!(r["ok"], json!(true));
    send_fallback(&fx, Some(P + 71), &t, "[보고] 본문");
    let before = counts(&t);
    assert_eq!((before.0, before.1), (3, 3));
    let resp = pair_return(&fx, Some(P + 71), &t);
    assert_absorbed(&resp, "pair", "queued");
    assert_eq!(counts(&t), before, "사람 초안 불변 · 쓰기 0");
    assert_eq!(qlen(&t), 1);
}

// ─────────────────────────── 음성 대조(결측형 포함) ───────────────────────────

/// 명시 `--queued`(absorb_return 없음) 뒤 Return 은 비흡수.
#[test]
fn a2_explicit_queued_send_issues_no_ticket() {
    let fx = fx("explicit-queued");
    let t = pane(&fx, "worker-1", P + 80);
    let _x = pane(&fx, "worker-2", P + 81);
    let r = rpc(&fx, Some(P + 81), "surface.send_text", json!({
        "surface_id": t.id, "text": "명시 큐", "queued": true, "quiet": true,
    }));
    assert_eq!(r["ok"], json!(true));
    assert!(r["result"].get("return_absorb").is_none(), "요청하지 않은 발급 필드: {r}");
    assert_sent(&pair_return(&fx, Some(P + 81), &t));
}

/// pair_return 없는 원시 RPC Return(inject_text·cycle 3분할·authoritative 경로의 모양)은 비흡수.
#[test]
fn a2_raw_return_without_pair_flag_not_absorbed() {
    let fx = fx("raw");
    let t = pane(&fx, "worker-1", P + 90);
    let _x = pane(&fx, "worker-2", P + 91);
    typing_on(&t);
    send_fallback(&fx, Some(P + 91), &t, "본문");
    typing_off(&t);
    let r = rpc(&fx, Some(P + 91), "surface.send_key", json!({
        "surface_id": t.id, "key": "Return", "queued": false,
    }));
    assert_sent(&r);
}

/// 비제출 키(Down)는 표를 소거한다 — 선택지 조작 뒤 Return 은 흡수되지 않는다.
/// 관측: Down 뒤 queued 짝 Return 은 표가 없어 종전처럼 적재된다(absorb_miss=no_ticket).
#[test]
fn a2_down_key_clears_ticket() {
    let fx = fx("down");
    let t = pane(&fx, "worker-1", P + 100);
    let _x = pane(&fx, "worker-2", P + 101);
    typing_on(&t);
    send_fallback(&fx, Some(P + 101), &t, "본문");
    typing_off(&t);
    let d = rpc(&fx, Some(P + 101), "surface.send_key", json!({"surface_id": t.id, "key": "Down"}));
    assert_eq!(d["ok"], json!(true), "{d}");
    let r = rpc(&fx, Some(P + 101), "surface.send_key", json!({
        "surface_id": t.id, "key": "Return", "queued": true, "pair_return": true,
    }));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(r["result"]["queued"], json!(true), "종전 적재: {r}");
    assert_eq!(r["result"]["absorbed"], json!(false), "{r}");
    assert_eq!(r["result"]["absorb_miss"], json!("no_ticket"), "{r}");
    assert_eq!(qlen(&t), 2);
}

/// authoritative:true 는 비흡수(권위 경로는 구조적으로 대상 밖).
#[test]
fn a2_authoritative_return_not_absorbed() {
    let fx = fx("auth");
    let t = pane(&fx, "worker-1", P + 110);
    let _x = pane(&fx, "worker-2", P + 111);
    typing_on(&t);
    send_fallback(&fx, Some(P + 111), &t, "본문");
    typing_off(&t);
    let r = rpc(&fx, Some(P + 111), "surface.send_key", json!({
        "surface_id": t.id, "key": "Return", "pair_return": true, "authoritative": true,
    }));
    assert_sent(&r);
}

/// C-m 별칭은 비흡수(이름 축 Return|Enter 만).
#[test]
fn a2_cm_alias_not_absorbed() {
    let fx = fx("cm");
    let t = pane(&fx, "worker-1", P + 120);
    let _x = pane(&fx, "worker-2", P + 121);
    typing_on(&t);
    send_fallback(&fx, Some(P + 121), &t, "본문");
    typing_off(&t);
    let r = rpc(&fx, Some(P + 121), "surface.send_key", json!({
        "surface_id": t.id, "key": "C-m", "pair_return": true,
    }));
    assert_sent(&r);
}

/// 결측형: caller_pid None 은 표를 만들지도(return_absorb:false) 흡수하지도 않는다.
#[test]
fn a2_unverified_caller_neither_issues_nor_absorbs() {
    let fx = fx("unverified");
    let t = pane(&fx, "worker-1", P + 130);
    let _x = pane(&fx, "worker-2", P + 131);
    typing_on(&t);
    let r2 = send_fallback(&fx, None, &t, "외부 본문");
    assert_eq!(r2["result"]["return_absorb"], json!(false), "미검증은 발급 0: {r2}");
    typing_off(&t);
    assert_sent(&pair_return(&fx, None, &t));
    // 검증 발신자 X 의 표가 있어도 미검증 Return 은 그 표를 쓰지 못한다.
    typing_on(&t);
    send_fallback(&fx, Some(P + 131), &t, "X 본문");
    typing_off(&t);
    assert_sent(&pair_return(&fx, None, &t));
    // X 의 표는 그대로 — X 자신의 짝 Return 은 흡수.
    assert_absorbed(&pair_return(&fx, Some(P + 131), &t), "pair", "queued");
}

/// 1장·1회용: 흡수 직후 두 번째 Return 은 통과한다(재전송 안내가 참).
#[test]
fn a2_ticket_is_single_use() {
    let fx = fx("single-use");
    let t = pane(&fx, "worker-1", P + 140);
    let _x = pane(&fx, "worker-2", P + 141);
    typing_on(&t);
    send_fallback(&fx, Some(P + 141), &t, "본문");
    typing_off(&t);
    assert_absorbed(&pair_return(&fx, Some(P + 141), &t), "pair", "queued");
    assert_sent(&pair_return(&fx, Some(P + 141), &t));
}

/// 롤백 노브: CYS_RETURN_ABSORB_SECS=0 이면 발급·흡수 전부 끔(종전 동작).
#[test]
fn a2_ttl_zero_restores_old_behavior() {
    let fx = fx("ttl0");
    std::env::set_var("CYS_RETURN_ABSORB_SECS", "0");
    let t = pane(&fx, "worker-1", P + 150);
    let _x = pane(&fx, "worker-2", P + 151);
    typing_on(&t);
    let r2 = send_fallback(&fx, Some(P + 151), &t, "본문");
    assert_eq!(r2["result"]["return_absorb"], json!(false), "{r2}");
    typing_off(&t);
    assert_sent(&pair_return(&fx, Some(P + 151), &t));
}

/// 표 나이 ≥ ttl 이면 비흡수 + 만료 이벤트(ticket_age_ms — TTL 재조정 근거).
#[test]
fn a2_expired_ticket_not_absorbed_and_reported() {
    let fx = fx("expired");
    std::env::set_var("CYS_RETURN_ABSORB_SECS", "1");
    let t = pane(&fx, "worker-1", P + 160);
    let _x = pane(&fx, "worker-2", P + 161);
    typing_on(&t);
    send_fallback(&fx, Some(P + 161), &t, "본문");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    typing_off(&t);
    assert_sent(&pair_return(&fx, Some(P + 161), &t));
    let ev = bus_last(&fx, "queue.return_absorb_expired").expect("만료 이벤트");
    assert!(ev["payload"]["ticket_age_ms"].as_u64().unwrap_or(0) >= 1000, "{ev}");
}

/// 끝이 개행인 본문(자동 제출 본문)의 폴백은 표 없음 · 여러 줄(중간 LF) 본문은 표 있음.
#[test]
fn a2_trailing_newline_body_no_ticket_but_interior_newline_has() {
    let fx = fx("newline");
    let t = pane(&fx, "worker-1", P + 170);
    let _x = pane(&fx, "worker-2", P + 171);
    typing_on(&t);
    let r = send_fallback(&fx, Some(P + 171), &t, "끝 개행\n");
    assert_eq!(r["result"]["return_absorb"], json!(false), "{r}");
    let r = send_fallback(&fx, Some(P + 171), &t, "첫 줄\n둘째 줄");
    assert_eq!(r["result"]["return_absorb"], json!(true), "{r}");
}

/// 사람 키로 세대가 바뀐 owner 는 보상 없음(결측 = 값 아님).
#[test]
fn a2_human_key_invalidates_owner_no_compensation() {
    let fx = fx("owner-gen");
    let t = pane(&fx, "worker-1", P + 180);
    let _x = pane(&fx, "worker-2", P + 181);
    let _y = pane(&fx, "worker-3", P + 182);
    assert_eq!(direct(&fx, Some(P + 182), &t, "yyy")["ok"], json!(true));
    // GUI 포커스 보고(사람 경로 · 자동응답) — 계수는 그대로지만 세대가 오른다.
    let g0 = counts(&t).2;
    let r = rpc(&fx, Some(P + 180), "surface.send_text", json!({
        "surface_id": t.id, "text": "\u{1b}[I", "human": true, "quiet": true,
    }));
    assert_eq!(r["ok"], json!(true));
    assert!(counts(&t).2 > g0, "전제: 사람 경로가 세대를 올린다");
    assert_eq!((counts(&t).0, counts(&t).1), (3, 0));
    send_fallback(&fx, Some(P + 181), &t, "xxx");
    assert_sent(&pair_return(&fx, Some(P + 181), &t));
    assert_sent(&pair_return(&fx, Some(P + 182), &t));
}

/// ② 무clear 핀: 활성 표가 있어도 cycle 3분할(C-u → send '/clear' → Return · 원시 요청)은 /clear 를 제출한다.
#[test]
fn a2_cycle_three_split_with_active_ticket() {
    let fx = fx("cycle");
    let t = pane(&fx, "worker-1", P + 190);
    let x = pane(&fx, "master", P + 191);
    bind(&fx, P + 192, x.id);
    typing_on(&t);
    send_fallback(&fx, Some(P + 191), &t, "본문");
    typing_off(&t);
    let cu = rpc(&fx, Some(P + 191), "surface.send_key", json!({"surface_id": t.id, "key": "C-u"}));
    assert_eq!(cu["ok"], json!(true), "{cu}");
    assert_eq!(direct(&fx, Some(P + 191), &t, "/clear")["ok"], json!(true));
    let r = rpc(&fx, Some(P + 191), "surface.send_key", json!({"surface_id": t.id, "key": "Return"}));
    assert_sent(&r);
    assert_eq!(counts(&t).0, 0, "/clear 제출");

    // 변형: C-u 뒤 표가 (형제 폴백으로) 재발급돼도 /clear 가 줄에 있으면 PassThrough 로 제출.
    let cu = rpc(&fx, Some(P + 191), "surface.send_key", json!({"surface_id": t.id, "key": "C-u"}));
    assert_eq!(cu["ok"], json!(true));
    assert_eq!(direct(&fx, Some(P + 191), &t, "/clear")["ok"], json!(true));
    let r2 = send_fallback(&fx, Some(P + 192), &t, "형제 본문");
    assert_eq!(r2["result"]["return_absorb"], json!(true), "{r2}");
    let r = pair_return(&fx, Some(P + 191), &t);
    assert_sent(&r);
    assert_eq!(counts(&t).0, 0, "/clear 제출(PassThrough)");
}

// ─────────────────────────── ticket_observation — 표 직접 관측 ───────────────────────────

fn ticket_of(t: &Arc<Surface>, sender: u64) -> Option<crate::state::ReturnTicketKind> {
    t.return_tickets.lock().unwrap().get(&sender).map(|x| x.kind.clone())
}

/// 표 수명 주기: 발급(Pair{entry_id}) → 자기 본문 제출은 유지 → 비제출 키 소거 → 직접 send 소거(D2)
/// → 남의 본문 제출은 발신자 소거 + 주인 보상.
#[test]
fn a2_ticket_lifecycle_observed() {
    let fx = fx("lifecycle");
    let t = pane(&fx, "worker-1", P + 200);
    let x = pane(&fx, "worker-2", P + 201);
    let y = pane(&fx, "worker-3", P + 202);
    // 발급 — 항목 id 가 표에 실린다.
    typing_on(&t);
    let r2 = send_fallback(&fx, Some(P + 201), &t, "본문");
    typing_off(&t);
    let want = r2["result"]["queue_entry_id"].as_str().unwrap().to_string();
    assert_eq!(
        ticket_of(&t, x.id),
        Some(crate::state::ReturnTicketKind::Pair { entry_id: want })
    );
    // 비제출 키(Down) 소거.
    let d = rpc(&fx, Some(P + 201), "surface.send_key", json!({"surface_id": t.id, "key": "Down"}));
    assert_eq!(d["ok"], json!(true));
    assert_eq!(ticket_of(&t, x.id), None, "Down 은 표를 소거한다");
    let cu = rpc(&fx, Some(P + 201), "surface.send_key", json!({"surface_id": t.id, "key": "C-u"}));
    assert_eq!(cu["ok"], json!(true));
    assert_eq!(counts(&t).0, 0);
    // 직접 send 성공(D2)은 자기 표를 소거하고 소유자를 각인한다.
    typing_on(&t);
    send_fallback(&fx, Some(P + 201), &t, "본문2");
    typing_off(&t);
    assert!(ticket_of(&t, x.id).is_some());
    assert_eq!(direct(&fx, Some(P + 201), &t, "own")["ok"], json!(true));
    assert_eq!(ticket_of(&t, x.id), None, "D2: 직접 send 성공은 짝 Return 표를 끝낸다");
    assert_eq!(t.pending_owner(), Some(x.id), "D0: 소유자 각인");
    // 자기 본문 제출은 표 유지(재발급 후 확인).
    typing_on(&t);
    send_fallback(&fx, Some(P + 201), &t, "본문3");
    typing_off(&t);
    assert_sent(&pair_return(&fx, Some(P + 201), &t));
    assert!(ticket_of(&t, x.id).is_some(), "소유자==발신자 제출은 표 유지");
    assert_eq!(t.pending_owner(), None, "제출(세대 변화)로 소유자 결측");
    // 남(Y)의 본문 제출 → X 표 소거 + Y 보상.
    assert_eq!(direct(&fx, Some(P + 202), &t, "yy")["ok"], json!(true));
    assert_eq!(t.pending_owner(), Some(y.id));
    assert_sent(&pair_return(&fx, Some(P + 201), &t));
    assert_eq!(ticket_of(&t, x.id), None);
    assert_eq!(ticket_of(&t, y.id), Some(crate::state::ReturnTicketKind::Compensation));
}

/// 보상 표는 ttl=0 이면 발급되지 않는다(노브 전면 차단) · 미검증 발신자의 제출은 아무 표도 만들지 않는다.
#[test]
fn a2_no_compensation_when_disabled_or_unverified() {
    let fx = fx("no-comp");
    let t = pane(&fx, "worker-1", P + 210);
    let _x = pane(&fx, "worker-2", P + 211);
    let y = pane(&fx, "worker-3", P + 212);
    assert_eq!(direct(&fx, Some(P + 212), &t, "yy")["ok"], json!(true));
    let r = rpc(&fx, None, "surface.send_key", json!({"surface_id": t.id, "key": "Return"}));
    assert_sent(&r);
    assert_eq!(ticket_of(&t, y.id), None, "미검증 발신자 제출 → 보상 없음");
    std::env::set_var("CYS_RETURN_ABSORB_SECS", "0");
    assert_eq!(direct(&fx, Some(P + 212), &t, "yy")["ok"], json!(true));
    assert_sent(&pair_return(&fx, Some(P + 211), &t));
    assert_eq!(ticket_of(&t, y.id), None, "ttl=0 → 보상 발급 0");
    assert!(t.return_tickets.lock().unwrap().is_empty());
}

/// 발급 때 만료분 정리 — 표 HashMap 은 대상마다 (살아 있는 발신자 수)로 유계.
#[test]
fn a2_issue_prunes_expired_tickets() {
    let fx = fx("prune");
    let t = pane(&fx, "worker-1", P + 220);
    let ttl = std::time::Duration::from_secs(30);
    for sender in 1..=5u64 {
        t.issue_return_ticket(sender, crate::state::ReturnTicketKind::Compensation, ttl);
    }
    // 1..=5 를 전부 만료 나이로 되돌린다.
    let old = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(31))
        .expect("단조 시계 31초 전");
    for tk in t.return_tickets.lock().unwrap().values_mut() {
        tk.issued = old;
    }
    t.issue_return_ticket(9, crate::state::ReturnTicketKind::Compensation, ttl);
    let keys: Vec<u64> = t.return_tickets.lock().unwrap().keys().copied().collect();
    assert_eq!(keys, vec![9], "발급 때 만료분 정리");
    // CAS: 발급 시각이 다르면 꺼내지 않는다.
    assert!(t.take_return_ticket(9, old).is_none());
    let at = t.peek_return_ticket(9).unwrap().issued;
    assert!(t.take_return_ticket(9, at).is_some());
    assert!(t.peek_return_ticket(9).is_none());
}
