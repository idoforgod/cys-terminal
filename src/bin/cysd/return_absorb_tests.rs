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
        std::env::remove_var("CYS_RETURN_ABSORB_REFLEX_MS");
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
    std::env::remove_var("CYS_RETURN_ABSORB_REFLEX_MS");
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
    t.return_tickets
        .lock()
        .unwrap()
        .get(&crate::state::PairKey::Verified(sender))
        .map(|x| x.kind.clone())
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
    let v = crate::state::PairKey::Verified;
    for sender in 1..=5u64 {
        t.issue_return_ticket(v(sender), crate::state::ReturnTicketKind::Compensation, ttl);
    }
    // 1..=5 를 전부 만료 나이로 되돌린다.
    let old = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(31))
        .expect("단조 시계 31초 전");
    for tk in t.return_tickets.lock().unwrap().values_mut() {
        tk.issued = old;
    }
    t.issue_return_ticket(v(9), crate::state::ReturnTicketKind::Compensation, ttl);
    let keys: Vec<crate::state::PairKey> = t.return_tickets.lock().unwrap().keys().copied().collect();
    assert_eq!(keys, vec![v(9)], "발급 때 만료분 정리");
    // CAS: 발급 시각이 다르면 꺼내지 않는다.
    assert!(t.take_return_ticket(v(9), old).is_none());
    let at = t.peek_return_ticket(v(9)).unwrap().issued;
    assert!(t.take_return_ticket(v(9), at).is_some());
    assert!(t.peek_return_ticket(v(9)).is_none());
}

/// ★(B) 좌석당 상한: 서로 다른 Claimed 100장을 발급해도 64장 이하 · 가장 새 표는 남는다(퇴출은 가장 오래된 것부터).
/// Claimed 번호 공간은 식별 불가 호출자의 자기신고라 열려 있다 — 상한이 없으면 표가 무한히 자란다.
#[test]
fn b_ticket_map_capped_newest_kept() {
    let fx = fx("b-cap");
    let t = pane(&fx, "worker-1", P + 760);
    let ttl = std::time::Duration::from_secs(30);
    let c = crate::state::PairKey::Claimed;
    for n in 0..100u64 {
        t.issue_return_ticket(c(10_000 + n), crate::state::ReturnTicketKind::Compensation, ttl);
    }
    let m = t.return_tickets.lock().unwrap();
    assert!(m.len() <= crate::state::RETURN_TICKETS_CAP, "상한: {}", m.len());
    assert_eq!(m.len(), crate::state::RETURN_TICKETS_CAP);
    assert!(m.contains_key(&c(10_099)), "가장 새 표는 남는다");
    assert!(!m.contains_key(&c(10_000)), "가장 오래된 표부터 퇴출");
}

/// ★(B) 표 1장에 두 스레드가 동시에 CAS 소비 — 정확히 1개만 꺼낸다(1회성).
#[test]
fn b_concurrent_take_exactly_one() {
    let fx = fx("b-race");
    let t = pane(&fx, "worker-1", P + 770);
    let key = crate::state::PairKey::Claimed(CEO_FROM);
    for _ in 0..50 {
        t.issue_return_ticket(key, crate::state::ReturnTicketKind::Compensation, std::time::Duration::from_secs(30));
        let at = t.peek_return_ticket(key).unwrap().issued;
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let hs: Vec<_> = (0..2)
            .map(|_| {
                let (t, b) = (t.clone(), barrier.clone());
                std::thread::spawn(move || {
                    b.wait();
                    t.take_return_ticket(key, at).is_some()
                })
            })
            .collect();
        let got: usize = hs.into_iter().map(|h| h.join().unwrap() as usize).sum();
        assert_eq!(got, 1, "정확히 1회 소비");
    }
}

// ─────────────── A2-F1 — 승인이 살아 있으면 흡수는 반사 창 안으로만(워커 hang 금지) ───────────────
//
// 리뷰 A2-F1(major): 표는 큐 전환 사유와 무관하게 발급되고 TTL(30초) 동안 같은 발신자의 첫 단일 Return 을
// 삼켰다. 모달 전환 때 CLI 는 "짝 Return 을 보내지 마라" 라고 안내한다 — 안내를 따르면 표가 남아 **그다음
// 의도적 승인 Return** 이 쓰기 0 · rc 0 으로 사라진다(샌드박스 r1b: 5.6초 뒤 Return → ABSORBED · 도달 b'').
// 불변식(governance `draft_gate_modal_verdict` doc · U8-P1): SubmitKey 는 무변경 — master 의 Return 이
// 화면 승인의 유일한 수단이다(막으면 워커 hang). 아래 검체가 그 불변식의 회귀 핀이다.

/// 좌석을 **권한 창(모달) 전경**으로 만든다 — U8-P1 검체와 같은 실 claude 권한 프롬프트 픽스처.
fn paint_modal(t: &Arc<Surface>) {
    *t.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
    let modal = cys::first_run_gates::fixtures::LIVE_PERMISSION_PROMPT.replace('\n', "\r\n");
    let mut parser = t.parser.lock().unwrap();
    parser.process(b"\x1b[2J\x1b[H");
    parser.process(modal.as_bytes());
}

/// 모달 전경 좌석으로의 `cys send` 폴백 — 직접 전송이 `[draft_gate:modal]` 로 거부된 뒤 큐 전환(표 발급).
fn send_fallback_modal(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>, text: &str) -> Value {
    let first = direct(fx, pid, t, text);
    let msg = first["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("[draft_gate:modal]"), "전제: 모달 전경 거부로 전환돼야 한다: {first}");
    send_fallback_after_denial(fx, pid, t, text)
}

fn send_fallback_after_denial(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>, text: &str) -> Value {
    let r2 = rpc(fx, pid, "surface.send_text", json!({
        "surface_id": t.id, "text": text, "queued": true, "absorb_return": true, "quiet": true,
    }));
    assert_eq!(r2["ok"], json!(true), "큐 전환은 성공해야 한다: {r2}");
    assert_eq!(r2["result"]["return_absorb"], json!(true), "표 발급: {r2}");
    r2
}

/// 발신자 표를 `ms` 만큼 늙힌다 — 벽시계 대기 없이 결정론(r1b 재현 나이 5.6초 등).
fn age_ticket(t: &Arc<Surface>, sender: u64, ms: u64) {
    let mut m = t.return_tickets.lock().unwrap();
    let tk = m.get_mut(&crate::state::PairKey::Verified(sender)).expect("늙힐 표가 있어야 한다");
    tk.issued = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_millis(ms))
        .expect("단조 시계");
}

/// 데몬 발행 승인 feed(화면 어휘가 아니라 feed 사실) — `approval_or_gate_pending` 의 승인 축.
fn push_daemon_approval(fx: &Fx, sid: u64, n: u32) {
    fx.daemon.feed_items.lock().unwrap().push(crate::state::FeedItem {
        request_id: format!("{}a2f1-approval-{n}", crate::state::DAEMON_REQ_PREFIX),
        kind: "approval".into(),
        title: "Allow execution".into(),
        body: "Allow execution of `cargo test`?".into(),
        surface_id: Some(sid),
        status: "pending".into(),
        decision: None,
        created_at: crate::state::now_epoch(),
        resolved_at: None,
        tier: None,
        publisher_pid: None,
        publisher_pgid: None,
        publisher_surface: None,
        risk_class: None,
        auto_route: false,
        resolver_surface: None,
        resolver_pid: None,
    });
}

/// ★A2-F1 핵심 재현(r1b 의 모달 판): master X → 리뷰어 R(권한 창 전경) `send` 가 모달로 큐 전환 → X 는
/// 안내대로 짝 Return 을 보내지 않음 → 5.6초 뒤 X 가 read-screen 후 **승인 Return** → **쓰기 1회**.
/// 종전(629ebd51): ABSORBED · 쓰기 0 · rc 0 — 창은 그대로, 본문은 모달 뒤에 묶여 워커 hang.
#[test]
fn a2f1_modal_approval_return_beyond_reflex_is_written_once() {
    let fx = fx("f1-modal");
    let t = pane(&fx, "reviewer-1", P + 300);
    let x = pane(&fx, "master", P + 301);
    paint_modal(&t);
    send_fallback_modal(&fx, Some(P + 301), &t, "[지시] 리뷰 착수");
    age_ticket(&t, x.id, 5_600);
    let before = counts(&t);
    let resp = pair_return(&fx, Some(P + 301), &t);
    let after = counts(&t);

    assert_sent(&resp);
    assert_eq!(resp["result"]["absorb_miss"], json!("approval_live"), "사유: {resp}");
    assert!(after.2 > before.2, "승인 Return 은 PTY 에 써야 한다(세대 증가 = 쓰기): {before:?} → {after:?}");
    assert_eq!(qlen(&t), 1, "본문은 큐에 그대로(모달이 닫힌 뒤 CR 포함 배달)");
    assert_eq!(bus_count(&fx, "queue.return_absorbed"), 0, "흡수 이벤트 0");
    let ev = bus_last(&fx, "queue.return_absorb_bypassed").expect("우회 이벤트(반사 창 재조정 근거)");
    assert_eq!(ev["payload"]["reason"], json!("approval_live"), "{ev}");
    assert!(ev["payload"]["ticket_age_ms"].as_u64().unwrap_or(0) >= 5_600, "{ev}");
    assert_eq!(ev["payload"]["reflex_ms"], json!(2000), "{ev}");
    assert_eq!(ticket_of(&t, x.id), None, "쓴 뒤 정산이 표를 소거(빈 줄 위 제출)");
}

/// 대조(S22 방지 유지): 같은 모달 좌석이라도 반사 창 **안**의 짝 Return(같은 셸 체인)은 흡수 — 쓰기 0.
#[test]
fn a2f1_modal_pair_return_inside_reflex_still_absorbed() {
    let fx = fx("f1-reflex");
    // 반사 창을 넉넉히(25초) — 부하가 큰 러너에서 픽스처 준비가 기본 2초를 넘겨 대조가 흔들리지 않게.
    std::env::set_var("CYS_RETURN_ABSORB_REFLEX_MS", "25000");
    let t = pane(&fx, "reviewer-1", P + 310);
    let _x = pane(&fx, "master", P + 311);
    paint_modal(&t);
    send_fallback_modal(&fx, Some(P + 311), &t, "[지시] 리뷰 착수");
    let before = counts(&t);
    let resp = pair_return(&fx, Some(P + 311), &t);
    assert_absorbed(&resp, "pair", "queued");
    assert_eq!(counts(&t), before, "반사 창 안 짝 Return 은 창을 누르지 않는다(쓰기 0)");
    assert_eq!(bus_count(&fx, "queue.return_absorb_bypassed"), 0);
}

/// 노브: `CYS_RETURN_ABSORB_REFLEX_MS=0` 이면 승인이 살아 있는 좌석에서는 갓 발급된 표도 흡수하지 않는다.
#[test]
fn a2f1_reflex_zero_never_absorbs_while_approval_live() {
    let fx = fx("f1-reflex0");
    std::env::set_var("CYS_RETURN_ABSORB_REFLEX_MS", "0");
    let t = pane(&fx, "reviewer-1", P + 320);
    let _x = pane(&fx, "master", P + 321);
    paint_modal(&t);
    send_fallback_modal(&fx, Some(P + 321), &t, "[지시] 리뷰 착수");
    let before = counts(&t);
    let resp = pair_return(&fx, Some(P + 321), &t);
    assert_sent(&resp);
    assert!(counts(&t).2 > before.2, "쓰기 1회");
}

/// 승인 feed 축(화면 모달 서명 없음): 타이핑 가드로 전환된 뒤 승인 feed 가 걸린 좌석에 반사 창 밖 Return
/// → 쓰기 1회. queued 짝 Return(CLI 폴백 r2)도 흡수하지 않고 종전처럼 적재한다(absorb_miss=approval_live).
#[test]
fn a2f1_feed_approval_return_beyond_reflex_is_not_absorbed() {
    let fx = fx("f1-feed");
    let t = pane(&fx, "worker-1", P + 330);
    let x = pane(&fx, "master", P + 331);
    typing_on(&t);
    send_fallback(&fx, Some(P + 331), &t, "[지시] 본문");
    typing_off(&t);
    push_daemon_approval(&fx, t.id, 1);
    age_ticket(&t, x.id, 5_600);
    let before = counts(&t);
    let resp = pair_return(&fx, Some(P + 331), &t);
    assert_sent(&resp);
    assert_eq!(resp["result"]["absorb_miss"], json!("approval_live"), "{resp}");
    assert!(counts(&t).2 > before.2, "쓰기 1회");

    // queued 변형 — 새 전환(표 재발급) 뒤 반사 창 밖 queued 짝 Return.
    typing_on(&t);
    send_fallback(&fx, Some(P + 331), &t, "[지시] 본문2");
    age_ticket(&t, x.id, 5_600);
    let q0 = qlen(&t);
    let r = rpc(&fx, Some(P + 331), "surface.send_key", json!({
        "surface_id": t.id, "key": "Return", "queued": true, "pair_return": true,
    }));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(r["result"]["queued"], json!(true), "종전 적재: {r}");
    assert_eq!(r["result"]["absorbed"], json!(false), "{r}");
    assert_eq!(r["result"]["absorb_miss"], json!("approval_live"), "{r}");
    assert_eq!(qlen(&t), q0 + 1);
}

/// 대조(A2 범위 보존): 승인이 **없는** 좌석은 반사 창 밖(5.6초)이어도 TTL 안이면 종전처럼 흡수한다 —
/// 좁힘은 승인이 살아 있을 때만이다(빈 줄 맨 CR · 빈 큐 항목 방지 불변).
#[test]
fn a2f1_no_approval_keeps_ttl_absorb() {
    let fx = fx("f1-noappr");
    let t = pane(&fx, "worker-1", P + 340);
    let x = pane(&fx, "master", P + 341);
    typing_on(&t);
    send_fallback(&fx, Some(P + 341), &t, "[보고] 완료");
    typing_off(&t);
    age_ticket(&t, x.id, 5_600);
    let before = counts(&t);
    let resp = pair_return(&fx, Some(P + 341), &t);
    assert_absorbed(&resp, "pair", "queued");
    assert_eq!(counts(&t), before);
    assert_eq!(bus_count(&fx, "queue.return_absorb_bypassed"), 0);
    // 해소된(pending 아님) 승인 feed 는 승인이 아니다 — 결측·과거는 값이 아니다.
    typing_on(&t);
    send_fallback(&fx, Some(P + 341), &t, "[보고] 완료2");
    typing_off(&t);
    push_daemon_approval(&fx, t.id, 2);
    for it in fx.daemon.feed_items.lock().unwrap().iter_mut() {
        it.status = "resolved".into();
    }
    age_ticket(&t, x.id, 5_600);
    assert_absorbed(&pair_return(&fx, Some(P + 341), &t), "pair", "queued");
}

// ─────── RF1-GATE-NARROW — 첫기동 관문은 좁힘에서 뺀다(A2 TTL 흡수 유지 · 온보딩 치명 방향 차단) ───────
//
// 리뷰 RF1-GATE-NARROW(major · 샌드박스 gsa 재현): A2-F1 의 좁힘 술어(`seat_approval_live` =
// 승인·관문 feed ∨ 모달 전경)는 첫기동 관문도 "승인이 살아 있다" 로 읽었다. 관문 좌석으로의 `send` 가
// `[draft_gate:modal]` 로 큐 전환된 뒤 반사 창(2초) 밖에 오는 짝 Return(LLM 이 도구 호출을 따로 해서 늦게
// 오는 관례적 Return)이 흡수되지 않고 PTY 에 써져 **관문의 기본 선택지**를 눌렀다 — 면책·2.1.261+
// 폴더신뢰는 `No, exit`(rc 1 좌석 사망), fullscreen 안내는 `Yes, try it`(관측 전제 붕괴). 둘 다
// `AbsenceCost::Fatal`. 치명 관문에서 맨 Return 은 정답 조작이 아니다(정답 = 방향키로 통과 선택지 라벨에
// 옮긴 뒤 Return — 비제출 키는 D4 정산이 표를 먼저 지운다) → 좁힘이 살려 주는 정당한 Return 이 없고 치명
// Return 만 통과시켰다.
// 아래 검체가 "관문 좌석은 A2 TTL 흡수 그대로" 의 회귀 핀이다(A2 629ebd51 의 보호 복원).

/// 좌석 화면을 `screen` 으로 칠한다(claude 어댑터 — 마커·모달 판정이 붙는 좌석).
fn paint_screen(t: &Arc<Surface>, screen: &str) {
    *t.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
    let s = screen.replace('\n', "\r\n");
    let mut parser = t.parser.lock().unwrap();
    parser.process(b"\x1b[2J\x1b[H");
    parser.process(s.as_bytes());
}

/// 2.1.261+ 폴더신뢰 창(선택지 순서 `No, exit` 먼저 · 기본 포커스 `No, exit` — cys.rs
/// `GATE_DEFAULT_FOCUS_WARNING` · first_run_gates `default_index` doc 의 H-2 실측 사실). 문면·위젯은 코퍼스
/// `folder-trust` 의 needle·서명 그대로이고 선택지 **순서만** 뒤집은 합성 화면이다.
const FOLDER_TRUST_NO_EXIT_FIRST: &str = "Accessing workspace: <cwd>\n\
    Quick safety check: Is this a project you created or one you trust?\n\
    ❯ 1. No, exit\n\
    \x20 2. Yes, I trust this folder\n\
    Enter to confirm · Esc to cancel\n";

/// 데몬 발행 **관문** feed(`GATE_FEED_KIND`) — 스캐너가 관문을 격상한 사실.
fn push_gate_feed(fx: &Fx, sid: u64, n: u32) {
    fx.daemon.feed_items.lock().unwrap().push(crate::state::FeedItem {
        request_id: format!("{}rf1-gate-{n}", crate::state::DAEMON_REQ_PREFIX),
        kind: crate::governance::GATE_FEED_KIND.into(),
        title: "claude 첫기동 관문 감지".into(),
        body: "[관문감지] id=bypass-disclaimer".into(),
        surface_id: Some(sid),
        status: "pending".into(),
        decision: None,
        created_at: crate::state::now_epoch(),
        resolved_at: None,
        tier: None,
        publisher_pid: None,
        publisher_pgid: None,
        publisher_surface: None,
        risk_class: None,
        auto_route: false,
        resolver_surface: None,
        resolver_pid: None,
    });
}

/// 관문 좌석 공통 재현: X `send`(모달 전환 · 큐 · 표) → 5.6초 뒤 X 의 단일 짝 Return → **흡수 · 쓰기 0**.
/// A2-F1(03af1684) 에서는 `absorb_miss=approval_live` · 쓰기 1회(= 관문 기본 선택지 확정)였다.
fn assert_gate_late_pair_return_absorbed(tag: &str, pid: u32, screen: &str) {
    let fx = fx(tag);
    let t = pane(&fx, "worker-1", pid);
    let x = pane(&fx, "master", pid + 1);
    paint_screen(&t, screen);
    assert!(
        cys::first_run_gates::identify(&cys::first_run_gates::builtin(), &t.parser.lock().unwrap().screen().contents())
            .is_some(),
        "전제: 좌석 화면이 코퍼스 관문이어야 한다({tag})"
    );
    send_fallback_modal(&fx, Some(pid + 1), &t, "[보고] 각성 완료");
    age_ticket(&t, x.id, 5_600);
    let before = counts(&t);
    let resp = pair_return(&fx, Some(pid + 1), &t);
    assert_absorbed(&resp, "pair", "queued");
    assert_eq!(counts(&t), before, "관문 좌석의 늦은 짝 Return 은 기본 선택지를 누르지 않는다(쓰기 0) — {tag}");
    assert_eq!(qlen(&t), 1, "본문은 큐에 그대로(관문이 닫힌 뒤 배달)");
    assert_eq!(bus_count(&fx, "queue.return_absorb_bypassed"), 0, "좁힘 우회 0 — {tag}");
    assert_eq!(bus_count(&fx, "queue.return_absorbed"), 1, "흡수 이벤트 1건 — {tag}");
}

/// ★RF1 핵심(gsa 재현 · 면책 창 · 기본 포커스 `No, exit` = rc 1 좌석 사망).
#[test]
fn rf1_gate_disclaimer_late_pair_return_absorbed_zero_write() {
    assert_gate_late_pair_return_absorbed(
        "rf1-disc",
        P + 400,
        cys::first_run_gates::fixtures::TRUST_ECHO_THEN_DISCLAIMER,
    );
}

/// ★RF1 fullscreen 안내(기본 포커스 `Yes, try it` = 대체 화면·마우스 보고 → 관측 전제 붕괴).
#[test]
fn rf1_gate_fullscreen_late_pair_return_absorbed_zero_write() {
    assert_gate_late_pair_return_absorbed(
        "rf1-full",
        P + 410,
        cys::first_run_gates::fixtures::FEATURE_FULLSCREEN,
    );
}

/// ★RF1 2.1.261+ 폴더신뢰(선택지 `No, exit` 먼저 · 기본 포커스 `No, exit`).
#[test]
fn rf1_gate_folder_trust_no_exit_first_late_pair_return_absorbed_zero_write() {
    assert_gate_late_pair_return_absorbed("rf1-trust", P + 420, FOLDER_TRUST_NO_EXIT_FIRST);
}

/// ★RF1 관문 증거는 승인 feed 보다 앞선다 — claude `approval_patterns.trust-prompt` 가 폴더신뢰 구 문면과
/// 겹치므로(문서화된 1건) 관문 화면이 **승인 feed(kind=approval)** 를 낳을 수 있다. 관문 화면 + 승인 feed +
/// 관문 feed 가 함께 걸려도 늦은 짝 Return 은 흡수된다. 반사 창 0(= "승인이 살아 있으면 흡수 안 함")에서도
/// 관문은 승인이 아니므로 흡수된다.
#[test]
fn rf1_gate_veto_beats_approval_feed_and_reflex_zero() {
    let fx = fx("rf1-veto");
    std::env::set_var("CYS_RETURN_ABSORB_REFLEX_MS", "0");
    let t = pane(&fx, "worker-1", P + 430);
    let x = pane(&fx, "master", P + 431);
    paint_screen(&t, cys::first_run_gates::fixtures::TRUST_ECHO_THEN_DISCLAIMER);
    send_fallback_modal(&fx, Some(P + 431), &t, "[보고] 각성 완료");
    push_daemon_approval(&fx, t.id, 1);
    push_gate_feed(&fx, t.id, 1);
    age_ticket(&t, x.id, 5_600);
    let before = counts(&t);
    let resp = pair_return(&fx, Some(P + 431), &t);
    assert_absorbed(&resp, "pair", "queued");
    assert_eq!(counts(&t), before, "관문 화면 위 Return 은 승인 feed 가 있어도 쓰지 않는다");
    assert_eq!(bus_count(&fx, "queue.return_absorb_bypassed"), 0);

    // 관문 화면 + 승인 feed 만(관문 feed 는 아직 스캐너 주기 전) — 화면 식별이 단독으로 막는다.
    fx.daemon.feed_items.lock().unwrap().retain(|i| i.kind != crate::governance::GATE_FEED_KIND);
    send_fallback_modal(&fx, Some(P + 431), &t, "[보고] 각성 완료2");
    age_ticket(&t, x.id, 5_600);
    let before = counts(&t);
    assert_absorbed(&pair_return(&fx, Some(P + 431), &t), "pair", "queued");
    assert_eq!(counts(&t), before, "화면 관문 식별 단독으로도 쓰기 0");

    // queued 짝 Return(CLI 폴백 r2)도 관문 좌석에서는 흡수(종전 A2) — 적재 0.
    send_fallback_modal(&fx, Some(P + 431), &t, "[보고] 각성 완료3");
    age_ticket(&t, x.id, 5_600);
    let q0 = qlen(&t);
    let r = rpc(&fx, Some(P + 431), "surface.send_key", json!({
        "surface_id": t.id, "key": "Return", "queued": true, "pair_return": true,
    }));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(r["result"]["absorbed"], json!(true), "관문 좌석 queued 짝 Return 흡수: {r}");
    assert_eq!(qlen(&t), q0, "빈 Return 큐 항목 0(관문이 닫힌 뒤 맨 CR 이 배달되지 않는다)");
}

/// ★RF1 관문 feed 는 **거부 증거**다(승인 증거가 아니다) — 화면이 비관문 권한 창이어도 관문 feed 가
/// pending 이면 좁히지 않는다. 실패 방향: 스캐너 feed 가 한 주기(≤15초) 늦게 종결되는 사이의 진짜 승인
/// Return 이 1회 흡수된다 → ABSORBED 통지 + 재전송 1회로 회복(가역). 반대로 관문을 승인으로 세면 치명(비가역).
#[test]
fn rf1_gate_feed_is_veto_not_approval() {
    let fx = fx("rf1-gfeed");
    let t = pane(&fx, "worker-1", P + 440);
    let x = pane(&fx, "master", P + 441);
    paint_modal(&t);
    send_fallback_modal(&fx, Some(P + 441), &t, "[지시] 리뷰 착수");
    push_gate_feed(&fx, t.id, 1);
    age_ticket(&t, x.id, 5_600);
    let before = counts(&t);
    assert_absorbed(&pair_return(&fx, Some(P + 441), &t), "pair", "queued");
    assert_eq!(counts(&t), before);
    // 재전송 1회로 회복(흡수는 1장·1회용) — 가역 방향의 증명.
    let again = pair_return(&fx, Some(P + 441), &t);
    assert_sent(&again);
    assert!(counts(&t).2 > before.2, "재전송은 쓴다(회복 경로)");
}

/// ★RF1 관문 feed 만(화면은 비모달 · 승인 feed 없음) — 승인으로 세지 않는다: 종전 A2 흡수.
/// (A2-F1 초판은 `approval_or_gate_pending` 으로 관문 feed 를 승인으로 세어 반사 창 밖 Return 을 썼다.)
#[test]
fn rf1_gate_feed_alone_is_not_approval() {
    let fx2 = fx("rf1-gfeed2");
    let t2 = pane(&fx2, "worker-1", P + 442);
    let x2 = pane(&fx2, "master", P + 443);
    typing_on(&t2);
    send_fallback(&fx2, Some(P + 443), &t2, "[보고] 완료");
    typing_off(&t2);
    push_gate_feed(&fx2, t2.id, 2);
    age_ticket(&t2, x2.id, 5_600);
    let before2 = counts(&t2);
    assert_absorbed(&pair_return(&fx2, Some(P + 443), &t2), "pair", "queued");
    assert_eq!(counts(&t2), before2);
    assert_eq!(bus_count(&fx2, "queue.return_absorb_bypassed"), 0);
}

/// ★RF1 정답 조작은 잃지 않는다 — 관문에서 `send-key Down` 뒤 `send-key Return`: Down(비제출 키)의 쓰기 뒤
/// D4 정산이 표를 지우므로 이어지는 Return 은 흡수되지 않고 쓴다(기본 포커스가 아니라 옮겨 간 선택지).
#[test]
fn rf1_gate_down_then_return_is_written() {
    let fx = fx("rf1-down");
    let t = pane(&fx, "worker-1", P + 450);
    let x = pane(&fx, "master", P + 451);
    paint_screen(&t, cys::first_run_gates::fixtures::TRUST_ECHO_THEN_DISCLAIMER);
    send_fallback_modal(&fx, Some(P + 451), &t, "[보고] 각성 완료");
    age_ticket(&t, x.id, 5_600);
    let down = rpc(&fx, Some(P + 451), "surface.send_key", json!({
        "surface_id": t.id, "key": "Down", "queued": false,
    }));
    assert_eq!(down["ok"], json!(true), "{down}");
    assert_eq!(ticket_of(&t, x.id), None, "비제출 키 쓰기 뒤 표 소거(D4)");
    let before = counts(&t);
    let resp = pair_return(&fx, Some(P + 451), &t);
    assert_sent(&resp);
    assert!(counts(&t).2 > before.2, "Down 뒤 Return 은 쓴다(쓰기 1회)");
}

/// ★RF1 대조(A2-F1 보존): 비관문 권한 창(모달)은 여전히 좁힌다 — 반사 창 밖 승인 Return 은 쓴다.
/// 술어 직접 핀 — `seat_approval_live` 진리표(관문 화면·관문 feed 는 false · 권한 창·승인 feed 는 true).
#[test]
fn rf1_seat_approval_live_truth_table() {
    use cys::first_run_gates::fixtures as fxs;
    let fx = fx("rf1-table");
    let t = pane(&fx, "worker-1", P + 460);
    let live = |t: &Arc<Surface>| crate::governance::seat_approval_live(&fx.daemon, t);
    // 권한 창(비관문 모달) → true(A2-F1 좁힘 유지).
    paint_screen(&t, fxs::LIVE_PERMISSION_PROMPT);
    assert!(live(&t), "권한 창은 승인이 살아 있다");
    // 관문 화면 3종 → false.
    for (id, s) in [
        ("disclaimer", fxs::TRUST_ECHO_THEN_DISCLAIMER),
        ("fullscreen", fxs::FEATURE_FULLSCREEN),
        ("trust-261", FOLDER_TRUST_NO_EXIT_FIRST),
        ("trust-241", fxs::FOLDER_TRUST),
        ("theme", fxs::THEME),
    ] {
        paint_screen(&t, s);
        assert!(!live(&t), "첫기동 관문({id})은 좁힘의 '승인'이 아니다");
    }
    // 관문 화면 + 승인 feed → false(관문 증거 우선).
    paint_screen(&t, fxs::TRUST_ECHO_THEN_DISCLAIMER);
    push_daemon_approval(&fx, t.id, 1);
    assert!(!live(&t), "관문 화면 위 승인 feed 는 좁힘 근거가 아니다");
    // 비관문 빈 화면 + 승인 feed → true(승인 feed 축 유지).
    paint_screen(&t, fxs::READY_SHELL);
    assert!(live(&t), "승인 feed 는 승인이 살아 있다");
    // + 관문 feed → false(거부 증거).
    push_gate_feed(&fx, t.id, 1);
    assert!(!live(&t), "관문 feed 는 거부 증거");
    // 어댑터 미등록 좌석(맨 셸)의 관문 문면 + 승인 feed → false(원시 화면으로도 관문을 본다).
    let bare = pane(&fx, "worker-2", P + 461);
    {
        let s = fxs::TRUST_ECHO_THEN_DISCLAIMER.replace('\n', "\r\n");
        let mut p = bare.parser.lock().unwrap();
        p.process(b"\x1b[2J\x1b[H");
        p.process(s.as_bytes());
    }
    push_daemon_approval(&fx, bare.id, 2);
    assert!(!live(&bare), "마커 없는 좌석도 관문 화면이면 false");
}

// ─────── ★B(0.14.42) 교차 소켓 Claimed 키 — 검증 신원이 없을 때만 자기신고 `from` 으로 표를 연다 ───────
//
// 설계 B(design-B-final · A2 표 구조 재사용): A2 표는 **검증 신원**(커널 peer pid → 이 데몬의 좌석)만 키로 썼다.
// 교차 소켓 발신자(HQ 의 CEO 가 `cys --socket <부서>.sock send --to master …` + `send-key … Return`)는 이 데몬의
// 좌석이 아니라 검증 신원이 없다 → 모달 거부 → 큐 폴백 → 짝 Return 이 **그대로 써져** 부서장의 질문·권한 창을
// 눌렀다(샌드박스 미니 S22 · 좌석 밖 발신 + CYS_SURFACE_ID=900: A2 트리 20/30 · 09-21 CEO→부서장 사례와 같은 모양).
// B 는 검증 신원이 **없을 때만** CLI 가 싣는 자기신고 `from` 을 `Claimed` 키로 쓴다. 검증 신원이 있으면 그것만
// 쓴다(로컬 좌석은 from 을 무엇으로 싣든 Verified). Verified(n) 과 Claimed(n) 은 서로 다른 키다.
// 오너 토큰을 실은(pane 무귀속) 호출자는 Claimed 를 만들지도 쓰지도 않는다.

/// 이 데몬에는 없는 번호 — 다른 데몬(HQ)의 CEO 좌석 id(자기신고 from).
const CEO_FROM: u64 = 900;

fn direct_from(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>, text: &str, extra: Value) -> Value {
    let mut p = json!({"surface_id": t.id, "text": text, "queued": false, "quiet": true});
    for (k, v) in extra.as_object().expect("extra 는 객체") {
        p[k] = v.clone();
    }
    rpc(fx, pid, "surface.send_text", p)
}

/// `cys send` 폴백(신 CLI) — 직접 전송이 타이핑 가드·초안 게이트로 거부되면 `queued:true` + `absorb_return:true`
/// 로 1회 재요청한다. CLI 는 두 요청 모두에 `from`(CYS_SURFACE_ID)을 싣는다.
fn fallback_from(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>, text: &str, extra: Value) -> Value {
    let first = direct_from(fx, pid, t, text, extra.clone());
    assert_eq!(first["ok"], json!(false), "전제: 직접 전송이 거부돼야 폴백이 일어난다: {first}");
    let msg = first["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains(cys::MSG_TYPING_GUARD), "전제: 폴백 대상 거부(타이핑 가드 문구): {first}");
    let mut p = json!({"surface_id": t.id, "text": text, "queued": true, "absorb_return": true, "quiet": true});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    let r2 = rpc(fx, pid, "surface.send_text", p);
    assert_eq!(r2["ok"], json!(true), "큐 전환은 성공해야 한다: {r2}");
    r2
}

/// 신 CLI 의 단일 `send-key Return` — 비큐 요청에 `from`(B5) 을 싣는다.
fn pair_return_from(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>, extra: Value) -> Value {
    let mut p = json!({"surface_id": t.id, "key": "Return", "queued": false, "pair_return": true});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    rpc(fx, pid, "surface.send_key", p)
}

/// ★B 핵심 재현(적색): 교차 소켓 CEO(검증 불가 pid + from=900) → 부서장(권한 창 전경) `send` 가 모달로 큐 전환
/// → 같은 셸 체인의 짝 Return → **흡수(쓰기 0)**. 종전(A2): 표 미발급(unverified) → Return 이 써져 창을 눌렀다.
#[test]
fn b_claimed_cross_socket_modal_pair_return_absorbed() {
    let fx = fx("b-claimed");
    std::env::set_var("CYS_RETURN_ABSORB_REFLEX_MS", "25000"); // 부하 러너에서도 '반사 창 안' 전제 고정
    let t = pane(&fx, "master", P + 700);
    paint_modal(&t);
    let first = direct_from(&fx, None, &t, "[CEO 지시] 착수", json!({"from": CEO_FROM}));
    assert!(
        first["error"]["message"].as_str().unwrap_or("").contains("[draft_gate:modal]"),
        "전제: 모달 전경 거부: {first}"
    );
    let r2 = fallback_from(&fx, None, &t, "[CEO 지시] 착수", json!({"from": CEO_FROM}));
    assert_eq!(r2["result"]["return_absorb"], json!(true), "교차 소켓 자기신고 from 으로 표 발급: {r2}");
    // 대조 ①: from 없는 요청(구 CLI send-key)은 표를 쓰지 못한다 — 종전처럼 쓴다.
    // 대조 ②: 다른 자기신고(901)도 쓰지 못한다.
    // (둘 다 쓰기를 만들므로 CEO 의 흡수 검사 **뒤**에 본다 — 쓰기가 창을 닫는 좌석 모형은 없지만 순서를 고정.)
    let before = counts(&t);
    let resp = pair_return_from(&fx, None, &t, json!({"from": CEO_FROM}));
    assert_absorbed(&resp, "pair", "queued");
    assert_eq!(counts(&t), before, "쓰기 0 — 창의 기본 선택지를 누르지 않는다");
    assert_eq!(qlen(&t), 1, "본문은 큐에 그대로(창이 닫힌 뒤 CR 포함 배달)");
    let ev = bus_last(&fx, "queue.return_absorbed").expect("흡수 이벤트");
    assert_eq!(ev["payload"]["from"], json!(cys::surface_ref(CEO_FROM)), "{ev}");
    assert_eq!(ev["payload"]["from_verified"], json!(false), "자기신고 키임을 싣는다: {ev}");
    // 1회용: 같은 자기신고의 두 번째 Return 은 쓴다(재전송 안내가 참).
    assert_sent(&pair_return_from(&fx, None, &t, json!({"from": CEO_FROM})));
    // 새 전환 뒤 대조 ①②(쓰인 CR 의 PTY 에코가 화면을 밀었을 수 있다 — 창을 다시 그린다).
    paint_modal(&t);
    let _ = fallback_from(&fx, None, &t, "[CEO 지시] 2", json!({"from": CEO_FROM}));
    assert_sent(&pair_return_from(&fx, None, &t, json!({})));
    assert_sent(&pair_return_from(&fx, None, &t, json!({"from": CEO_FROM + 1})));
    assert_absorbed(&pair_return_from(&fx, None, &t, json!({"from": CEO_FROM})), "pair", "queued");
}

/// Verified(n) 과 Claimed(n) 은 서로 다른 키다(B 지적 2·11): 교차 소켓이 로컬 좌석 P 의 번호를 자기신고해도
/// ③ P 의 Return(Verified) 을 흡수하지 않고 ② P 의 직접 send 성공이 Claimed(P) 를 지우지 않는다.
#[test]
fn b_claimed_and_verified_are_separate_namespaces() {
    let fx = fx("b-ns");
    std::env::set_var("CYS_RETURN_ABSORB_REFLEX_MS", "25000");
    let t = pane(&fx, "worker-1", P + 710);
    let p = pane(&fx, "worker-2", P + 711);
    typing_on(&t);
    let r2 = fallback_from(&fx, None, &t, "외부 본문", json!({"from": p.id}));
    assert_eq!(r2["result"]["return_absorb"], json!(true), "Claimed(p) 발급: {r2}");
    typing_off(&t);
    // ③ 로컬 P 의 Return 은 흡수되지 않는다(Verified(p) 표 없음).
    assert_sent(&pair_return(&fx, Some(P + 711), &t));
    // ② 로컬 P 의 직접 send 성공은 Claimed(p) 를 지우지 않는다(D2 는 자기 키만 소거).
    assert_eq!(direct(&fx, Some(P + 711), &t, "P 본문")["ok"], json!(true));
    assert_sent(&pair_return(&fx, Some(P + 711), &t));
    assert_eq!(counts(&t).0, 0, "P 본문 제출");
    // Claimed(p) 는 남아 있다 — 식별 불가 from=p 의 짝 Return 은 흡수.
    assert_absorbed(&pair_return_from(&fx, None, &t, json!({"from": p.id})), "pair", "queued");
}

/// 스푸핑(대조 · 수정 전후 모두 녹색): 로컬 좌석 Q 가 from=CEO 를 실어도 **Verified(q)** 만 연다 —
/// 식별 불가 from=CEO 의 Return 은 그 표를 쓰지 못한다.
#[test]
fn b_local_pane_spoofed_from_arms_verified_only() {
    let fx = fx("b-spoof");
    std::env::set_var("CYS_RETURN_ABSORB_REFLEX_MS", "25000");
    let t = pane(&fx, "worker-1", P + 720);
    let _q = pane(&fx, "worker-2", P + 721);
    typing_on(&t);
    let r2 = fallback_from(&fx, Some(P + 721), &t, "Q 본문", json!({"from": CEO_FROM}));
    assert_eq!(r2["result"]["return_absorb"], json!(true), "{r2}");
    typing_off(&t);
    assert_sent(&pair_return_from(&fx, None, &t, json!({"from": CEO_FROM})));
    assert_absorbed(&pair_return(&fx, Some(P + 721), &t), "pair", "queued");
}

/// 오너 토큰(pane 무귀속 GUI 등급) 호출자는 from 을 실어도 Claimed 를 만들지도 쓰지도 않는다(대조 · 전후 녹색).
#[test]
fn b_owner_token_caller_never_claims() {
    let fx = fx("b-owner");
    let tok = fx.daemon.operator_token.clone().expect("데몬 토큰");
    let t = pane(&fx, "worker-1", P + 730);
    typing_on(&t);
    let r2 = fallback_from(&fx, None, &t, "오너 문안", json!({"from": CEO_FROM, "owner_token": tok}));
    assert_eq!(r2["result"]["return_absorb"], json!(false), "오너 토큰 호출자는 Claimed 발급 0: {r2}");
    typing_off(&t);
    assert_sent(&pair_return_from(&fx, None, &t, json!({"from": CEO_FROM, "owner_token": tok})));
}

/// 결측형(대조 · 전후 녹색): from 이 결측·null·빈 문자열·숫자 아닌 문자열이면 Claimed 도 없다.
#[test]
fn b_missing_or_garbage_from_never_claims() {
    let fx = fx("b-missing");
    let t = pane(&fx, "worker-1", P + 740);
    for (i, extra) in [
        json!({}),
        json!({"from": null}),
        json!({"from": ""}),
        json!({"from": "inject(typing_guard fallback)"}),
        json!({"from": -3}),
    ]
    .into_iter()
    .enumerate()
    {
        typing_on(&t);
        let r2 = fallback_from(&fx, None, &t, &format!("본문{i}"), extra.clone());
        assert_eq!(r2["result"]["return_absorb"], json!(false), "결측은 값이 아니다({extra}): {r2}");
        typing_off(&t);
        assert_sent(&pair_return_from(&fx, None, &t, extra));
    }
}

/// B ⓒ(적색): 같은 자기신고 발신자의 직접 send 성공은 Claimed 표를 끝낸다 — 뒤따르는 Return 은 새 본문을
/// 제출해야 한다(흡수 금지). 비제출 키(Down)도 Claimed 표를 지운다(D4 정산 — 선택지 조작 뒤 Return 비흡수).
#[test]
fn b_claimed_ticket_cleared_by_direct_success_and_non_submit_key() {
    let fx = fx("b-clear");
    std::env::set_var("CYS_RETURN_ABSORB_REFLEX_MS", "25000");
    let t = pane(&fx, "worker-1", P + 750);
    let ceo = json!({"from": CEO_FROM});
    typing_on(&t);
    let r2 = fallback_from(&fx, None, &t, "본문", ceo.clone());
    assert_eq!(r2["result"]["return_absorb"], json!(true), "{r2}");
    typing_off(&t);
    assert_eq!(direct_from(&fx, None, &t, "새 본문", ceo.clone())["ok"], json!(true));
    let r = pair_return_from(&fx, None, &t, ceo.clone());
    assert_sent(&r);
    assert_eq!(counts(&t).0, 0, "새 본문 제출(흡수 금지)");
    // 비제출 키.
    typing_on(&t);
    let r2 = fallback_from(&fx, None, &t, "본문2", ceo.clone());
    assert_eq!(r2["result"]["return_absorb"], json!(true), "{r2}");
    typing_off(&t);
    let mut down = json!({"surface_id": t.id, "key": "Down"});
    down["from"] = json!(CEO_FROM);
    assert_eq!(rpc(&fx, None, "surface.send_key", down)["ok"], json!(true));
    assert_sent(&pair_return_from(&fx, None, &t, ceo.clone()));
    // 노브 0 이면 Claimed 도 발급 0.
    std::env::set_var("CYS_RETURN_ABSORB_SECS", "0");
    typing_on(&t);
    let r2 = fallback_from(&fx, None, &t, "본문3", ceo.clone());
    assert_eq!(r2["result"]["return_absorb"], json!(false), "{r2}");
    typing_off(&t);
    assert_sent(&pair_return_from(&fx, None, &t, ceo));
}
