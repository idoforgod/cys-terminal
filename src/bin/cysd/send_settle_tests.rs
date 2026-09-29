//! ★(0.14.42 · S21-SETTLE) 직접 `cys send` 의 **제출 정착** — RPC 수준 회귀 핀.
//!
//! 【무엇을 고정하나】 기계 제출 CR(`send-key Return` → writer `SubmitAfterGap`)이 writer 에 넘어간 뒤 실제로 쓰이기
//! 전(최소 간격 150ms)이나 쓰인 직후(분리 창)에 에이전트 좌석으로 들어온 다음 직접 본문은 **writer FIFO 에서 그 CR
//! 바로 뒤에 붙어** `\r`+본문 한 덩이로 읽힌다 — 실 claude 는 그 덩이를 붙여넣기로 읽어 CR 을 줄바꿈으로 바꾸고
//! 두 본문을 한 초안으로 병합한다(4cf1490a 실 claude T13 · 조용한 유실). v0.14.41 의 S21 N=8 직접 수락 84/88 이
//! 바로 이 붙음(같은 read 조각에 `\rM|…`)으로 만든 수치였고, 0.14.42 A2 가 맨 CR 적체를 없애자 그 착시가 사라져
//! 직접 수락이 절반으로 줄었다(S21 13.2%). 여기 핀은 그 둘을 한 번에 닫는 규칙이다:
//!   ① **분리 보류** — 진행 중인 기계 제출 CR 이 있거나 방금 쓴 CR 의 분리 창 안이면 Text 본문을 받지 않는다
//!      (쓰기 0 · 사유 `submit_settling` · 정착 증명 ` [settle:<ms>]`).
//!   ② **정착 증명** — 이미 나던 D-12 거부(계수·화면 점유)라도 원인이 진행 중인 기계 제출이면 증명을 붙인다 →
//!      신 CLI 는 예산 안에서 다시 보낸다(큐 10초 간격으로 밀려나지 않는다). 사람 초안·모달에는 붙이지 않는다.
//! 【범위】 에이전트 좌석(agent_meta) · 유닉스 · 킬 스위치(`CYS_SEND_SETTLE=0` · `<state_dir>/send-settle-off`) 꺼짐일
//! 때만. 셸 좌석·윈도우·끔 상태는 종전 바이트 그대로다(아래 음성 대조).
//!
//! 관측 축은 return_absorb_tests 와 같다: 핸들러가 PTY 쓰기를 writer 에 넘기면 `apply_pending_input` 이 **반드시**
//! 변이 세대(`input_gen`)를 올린다 — "세대 불변" = "이 요청은 아무것도 쓰지 않았다". 이 파일의 행동 검체는 **기존
//! 공개 API 만** 쓴다(dispatch · create_surface · caller_cache · agent_meta) — 구현 전에도 컴파일되어 RED 를 실제로
//! 관측할 수 있게 하기 위함이다.
#![cfg(test)]

use crate::handlers::{dispatch, Reply};
use crate::state::{Daemon, Surface};
use cys::Request;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard};
use std::time::{Duration, Instant};

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
        std::env::remove_var("CYS_SEND_SETTLE");
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fx(tag: &str) -> Fx {
    // CYS_PACK_DIR·CYS_SEND_SETTLE 는 프로세스 전역이다 — handlers ACL 검체·governance 큐 검체와 **같은 락**.
    let g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::delivery::tests::isolate_state_dir_for_thread(tag);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cys-settle-{tag}-{}-{}-{n}",
        std::process::id(),
        crate::state::now_epoch() as u64
    ));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
    std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);
    std::env::remove_var("CYS_SEND_SETTLE");
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
    fx.daemon.caller_cache.lock().unwrap().insert(
        pid,
        crate::state::CallerCacheEntry::new(
            Some(s.id),
            crate::state::now_epoch(),
            None,
            fx.daemon.caller_gen.load(Ordering::Relaxed),
        ),
    );
    s
}

/// 에이전트 좌석(claude 어댑터 등록) — 정착 규칙의 적용 대상.
fn agent_pane(fx: &Fx, role: &str, pid: u32) -> Arc<Surface> {
    let s = pane(fx, role, pid);
    *s.agent_meta.lock().unwrap() = Some(("claude".into(), "claude".into()));
    s
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
    (s.pending_input_bytes.load(Ordering::Relaxed), human, s.input_gen.load(Ordering::Acquire))
}

fn qlen(s: &Arc<Surface>) -> usize {
    s.pending_queue.lock().unwrap().len()
}

fn direct(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>, text: &str) -> Value {
    rpc(fx, pid, "surface.send_text", json!({
        "surface_id": t.id, "text": text, "queued": false, "quiet": true,
    }))
}

/// 신 CLI 의 단일 `send-key Return` — `pair_return:true`.
fn pair_return(fx: &Fx, pid: Option<u32>, t: &Arc<Surface>) -> Value {
    rpc(fx, pid, "surface.send_key", json!({
        "surface_id": t.id, "key": "Return", "queued": false, "pair_return": true,
    }))
}

fn msg(resp: &Value) -> String {
    resp["error"]["message"].as_str().unwrap_or("").to_string()
}

/// 정착 증명 힌트(ms) — 거부 문구의 ` [settle:<ms>]`. 없으면 None(수기 파서 — lib 파서와 독립 대조).
fn settle_hint(resp: &Value) -> Option<u64> {
    let m = msg(resp);
    let i = m.rfind("[settle:")? + "[settle:".len();
    m[i..].split(']').next()?.parse().ok()
}

fn last_denied(fx: &Fx) -> Option<Value> {
    fx.daemon.bus.tail(400).into_iter().filter(|e| e["name"] == "queue.draft_gate_denied").last()
}

const P: u32 = 996_600;

// ─────────────────────────── 적색→녹색 ───────────────────────────

/// ① 분리 보류: X 의 제출 CR 이 writer 에 넘어간 직후(150ms 최소 간격 안) Y 의 본문은 받지 않는다 — 쓰기 0 ·
/// 큐 적재 0 · 사유 submit_settling · 정착 증명 힌트. CR 이 쓰이고 분리 창이 지나면 같은 본문이 직접 통과한다.
/// 종전(0.14.42 A2 트리): 셸 화면 축이 Unknown 이라 Y 가 곧바로 통과해 writer FIFO 에서 X 의 대기 CR 바로 뒤에 붙었다.
#[test]
fn settle_hold_denies_text_while_submit_cr_inflight_then_admits() {
    let fx = fx("hold");
    let t = agent_pane(&fx, "worker-1", P);
    let _x = pane(&fx, "worker-2", P + 1);
    let _y = pane(&fx, "worker-3", P + 2);
    let t0 = Instant::now();
    assert_eq!(direct(&fx, Some(P + 1), &t, "M|x|AAA")["ok"], json!(true));
    let xr = pair_return(&fx, Some(P + 1), &t);
    assert_eq!(xr["result"]["sent"], json!(true), "X 자기 본문의 Return 은 종전처럼 쓴다: {xr}");
    let before = counts(&t);
    let y1 = direct(&fx, Some(P + 2), &t, "M|y|BBB");
    assert_eq!(y1["ok"], json!(false), "대기 CR 뒤에 본문을 붙이면 안 된다(붙음 = 실 claude 병합): {y1}");
    let m = msg(&y1);
    assert!(m.contains(cys::MSG_TYPING_GUARD), "구 CLI 도 --queued 로 1회 전환하는 문구 접두: {m}");
    assert!(m.contains("[draft_gate:submit_settling]"), "사유 태그: {m}");
    let h = settle_hint(&y1).expect("정착 증명 힌트");
    assert!((1..=1000).contains(&h), "힌트는 '곧 빈다'(≤1s): {h}");
    assert_eq!(counts(&t), before, "보류는 쓰기 0(세대 불변)");
    assert_eq!(qlen(&t), 0, "보류는 적재하지 않는다(재시도는 CLI 몫)");
    let ev = last_denied(&fx).expect("거부 이벤트");
    assert_eq!(ev["payload"]["reason"], json!("submit_settling"), "{ev}");
    assert_eq!(ev["payload"]["kind"], json!("text"), "{ev}");
    assert!(ev["payload"]["settle_ms"].is_u64(), "증명 힌트는 이벤트에도(가산 키): {ev}");

    // CLI 정착 재시도 흉내 — 힌트만큼 쉬고 다시 보낸다(쓰기 0 이 확정된 명시 거부라 중복 주입 없음).
    let mut last = y1;
    let deadline = Instant::now() + Duration::from_secs(3);
    while last["ok"] != json!(true) && Instant::now() < deadline {
        let hint = settle_hint(&last).expect("재시도 대상 거부는 전부 증명이 있어야 한다");
        std::thread::sleep(Duration::from_millis(hint.clamp(5, 300)));
        last = direct(&fx, Some(P + 2), &t, "M|y|BBB");
    }
    assert_eq!(last["ok"], json!(true), "CR 이 쓰이고 분리 창이 지나면 직접 통과: {last}");
    let el = t0.elapsed();
    assert!(
        el >= Duration::from_millis(225),
        "Y 본문은 X 의 CR(본문 +150ms) 과 분리 창(80ms) 뒤에만 넘어간다: {el:?}"
    );
    assert_eq!(qlen(&t), 0, "재시도 성공은 큐를 쓰지 않는다");
}

/// ② 정착 증명(계수 축): X 의 새 기계 본문이 짝 Return 을 기다리는 중(발신자 검증 · 사람 바이트 0)이면 Y 의
/// pending_input 거부에 증명을 붙인다 — CLI 가 곧바로 큐(10초 간격)로 밀려나지 않고 짝 Return 뒤를 기다린다.
/// 종전: 증명 없음 → 곧바로 `--queued`.
#[test]
fn settle_proof_on_pending_machine_body_awaiting_its_return() {
    let fx = fx("pair-proof");
    let t = agent_pane(&fx, "worker-1", P + 10);
    let _x = pane(&fx, "worker-2", P + 11);
    let _y = pane(&fx, "worker-3", P + 12);
    assert_eq!(direct(&fx, Some(P + 11), &t, "M|x|AAA")["ok"], json!(true));
    let before = counts(&t);
    let y = direct(&fx, Some(P + 12), &t, "M|y|BBB");
    assert_eq!(y["ok"], json!(false), "{y}");
    assert!(msg(&y).contains("[draft_gate:pending_input]"), "사유는 종전 그대로: {y}");
    assert!(settle_hint(&y).is_some(), "짝 Return 을 기다리는 기계 본문 = 정착 증명: {y}");
    assert_eq!(counts(&t), before, "쓰기 0");
    let ev = last_denied(&fx).expect("거부 이벤트");
    assert!(ev["payload"]["settle_ms"].is_u64(), "{ev}");
}

// ─────────────────────────── 음성 대조(치명 방향) ───────────────────────────

/// 사람 초안 앞에서는 증명이 없다 — 재시도 0회로 곧바로 큐(종전 바이트 · 이벤트 페이로드 불변).
#[test]
fn no_settle_proof_on_human_draft() {
    let fx = fx("human");
    let t = agent_pane(&fx, "worker-1", P + 20);
    let _y = pane(&fx, "worker-3", P + 22);
    let r = rpc(&fx, Some(P + 20), "surface.send_text", json!({
        "surface_id": t.id, "text": "abc", "human": true, "quiet": true,
    }));
    assert_eq!(r["ok"], json!(true));
    *t.last_human_input.lock().unwrap() = None; // 타이핑 가드 창 밖 — D-12 계수 축만 남긴다
    let y = direct(&fx, Some(P + 22), &t, "M|y|BBB");
    assert_eq!(y["ok"], json!(false), "{y}");
    assert!(msg(&y).contains("[draft_gate:pending_input]"), "{y}");
    assert_eq!(settle_hint(&y), None, "사람 초안에는 정착 증명을 붙이지 않는다: {y}");
    let ev = last_denied(&fx).expect("거부 이벤트");
    assert!(ev["payload"].get("settle_ms").is_none(), "증명 없는 거부의 페이로드는 종전 그대로: {ev}");
}

/// 셸 좌석(agent_meta 없음)은 무변경 — 셸은 `\r`+다음 명령 한 덩이를 줄 단위로 읽으므로 분리가 필요 없다.
#[test]
fn shell_seat_unchanged_no_hold() {
    let fx = fx("shell");
    let t = pane(&fx, "worker-1", P + 30);
    let _x = pane(&fx, "worker-2", P + 31);
    let _y = pane(&fx, "worker-3", P + 32);
    assert_eq!(direct(&fx, Some(P + 31), &t, "echo a")["ok"], json!(true));
    assert_eq!(pair_return(&fx, Some(P + 31), &t)["result"]["sent"], json!(true));
    let y = direct(&fx, Some(P + 32), &t, "echo b");
    assert_eq!(y["ok"], json!(true), "셸 좌석은 종전처럼 곧바로 통과: {y}");
}

/// 킬 스위치(env) — `CYS_SEND_SETTLE=0` 이면 종전 동작(보류·증명 0).
#[test]
fn kill_switch_env_restores_previous_behavior() {
    let fx = fx("kill-env");
    std::env::set_var("CYS_SEND_SETTLE", "0");
    let t = agent_pane(&fx, "worker-1", P + 40);
    let _x = pane(&fx, "worker-2", P + 41);
    let _y = pane(&fx, "worker-3", P + 42);
    assert_eq!(direct(&fx, Some(P + 41), &t, "M|x|AAA")["ok"], json!(true));
    let yp = direct(&fx, Some(P + 42), &t, "M|y|BBB");
    assert_eq!(settle_hint(&yp), None, "끔이면 증명 0: {yp}");
    assert_eq!(pair_return(&fx, Some(P + 41), &t)["result"]["sent"], json!(true));
    let y = direct(&fx, Some(P + 42), &t, "M|y|BBB");
    assert_eq!(y["ok"], json!(true), "끔이면 보류 0(종전): {y}");
}

/// 킬 스위치(센티널) — 데몬 상태 디렉터리의 `send-settle-off` 가 있으면 재기동 없이 종전 동작.
#[test]
fn kill_switch_sentinel_restores_previous_behavior() {
    let fx = fx("kill-file");
    let sdir = crate::state::state_dir(&fx.daemon.socket_path);
    std::fs::create_dir_all(&sdir).unwrap();
    std::fs::write(sdir.join("send-settle-off"), b"").unwrap();
    let t = agent_pane(&fx, "worker-1", P + 50);
    let _x = pane(&fx, "worker-2", P + 51);
    let _y = pane(&fx, "worker-3", P + 52);
    assert_eq!(direct(&fx, Some(P + 51), &t, "M|x|AAA")["ok"], json!(true));
    assert_eq!(pair_return(&fx, Some(P + 51), &t)["result"]["sent"], json!(true));
    let y = direct(&fx, Some(P + 52), &t, "M|y|BBB");
    let _ = std::fs::remove_file(sdir.join("send-settle-off"));
    assert_eq!(y["ok"], json!(true), "센티널이면 보류 0(종전): {y}");
}

/// 권위 경로(clear_first)·사람 키 경로는 보류 대상이 아니다 — cycle-agent `/clear`(② 무clear)·오너 타이핑 무변경.
#[test]
fn clear_first_and_human_keys_are_not_held() {
    let fx = fx("exempt");
    let t = agent_pane(&fx, "worker-1", P + 60);
    let _x = pane(&fx, "worker-2", P + 61);
    assert_eq!(direct(&fx, Some(P + 61), &t, "M|x|AAA")["ok"], json!(true));
    assert_eq!(pair_return(&fx, Some(P + 61), &t)["result"]["sent"], json!(true));
    // 사람 키(GUI term.onData 모양) — 게이트 밖(종전 그대로).
    let h = rpc(&fx, Some(P + 60), "surface.send_text", json!({
        "surface_id": t.id, "text": "q", "human": true, "quiet": true,
    }));
    assert_eq!(h["ok"], json!(true), "사람 키는 정착 보류 대상이 아니다: {h}");
    // clear_first(원자 Inject) — ClearFirst 팔은 정착 규칙 밖(사람 초안 판정만 종전대로).
    let c = rpc(&fx, Some(P + 61), "surface.send_text", json!({
        "surface_id": t.id, "text": "/clear", "clear_first": true, "quiet": true,
    }));
    assert!(!msg(&c).contains("submit_settling"), "clear_first 에 정착 보류 없음: {c}");
    assert_eq!(settle_hint(&c), None, "clear_first 에 정착 증명 없음: {c}");
}

// ─────────────────────────── writer 표식(판정의 재료) ───────────────────────────

/// writer 표식: send_key 제출 CR 을 넘기는 순간 '진행 중'(계수 1) → writer 가 최소 간격 뒤 CR 을 쓰면 계수 0 ·
/// 쓴 시각이 찍힌다. 비제출 키(Down)는 표식을 올리지 않는다.
#[test]
fn writer_marks_submit_cr_inflight_until_written() {
    let fx = fx("writer");
    let t = agent_pane(&fx, "worker-1", P + 70);
    let _x = pane(&fx, "worker-2", P + 71);
    let obs = || t.inject_track.submit_settle_obs(crate::state::settle_mono_ms(), 2000);
    assert_eq!(obs(), crate::state::SubmitSettleObs::default(), "빈 좌석은 표식 없음");
    // 비제출 키는 다른 좌석에서 본다(Down 의 ESC 바이트는 계수에 쌓여 뒤 직접 본문을 막는다 — 종전 규칙).
    let k = agent_pane(&fx, "worker-3", P + 72);
    let d = rpc(&fx, Some(P + 71), "surface.send_key", json!({"surface_id": k.id, "key": "Down"}));
    assert_eq!(d["ok"], json!(true), "{d}");
    let ko = k.inject_track.submit_settle_obs(crate::state::settle_mono_ms(), 2000);
    assert!(!ko.inflight && ko.pending == 0 && ko.since_written_ms.is_none(), "비제출 키는 제출 CR 이 아니다: {ko:?}");
    assert_eq!(direct(&fx, Some(P + 71), &t, "M|x|AAA")["ok"], json!(true));
    assert_eq!(pair_return(&fx, Some(P + 71), &t)["result"]["sent"], json!(true));
    let o = obs();
    assert!(o.inflight && o.pending == 1, "인계 직후 진행 중: {o:?}");
    let deadline = Instant::now() + Duration::from_secs(3);
    while obs().inflight && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let o = obs();
    assert!(!o.inflight && o.pending == 0, "writer 가 CR 을 쓰면 진행 중이 끝난다: {o:?}");
    assert!(o.since_written_ms.is_some(), "쓴 시각이 찍힌다: {o:?}");
}

/// 신선도 상한: 계수가 남아도(writer 가 PTY 닫힘으로 끝난 경우 등) 상한 뒤에는 '진행 중' 이 아니다 — 영구 보류 0.
#[test]
fn stale_inflight_marker_expires() {
    let track = crate::state::InjectTrack::default();
    track.submit_handed();
    let now = crate::state::settle_mono_ms();
    assert!(track.submit_settle_obs(now, 2000).inflight, "막 넘긴 CR 은 진행 중");
    let later = track.submit_settle_obs(now + 2001, 2000);
    assert!(!later.inflight && later.pending == 0, "상한 뒤에는 진행 중으로 보지 않는다: {later:?}");
    track.submit_hand_failed();
    track.submit_hand_failed(); // 0 아래로 내려가지 않는다
    assert_eq!(track.submit_settle_obs(now, 2000).pending, 0);
}

/// 정착 증명 거부 이벤트는 (좌석, 발신자)당 1초에 1건 — CLI 재시도가 이벤트 링을 채우지 않는다. 응답(증명 힌트)은
/// 매번 그대로다. 발신자가 다르면 따로 센다. 증명 없는 거부는 종전처럼 요청마다 1건(위 사람 초안 검체·d12 핀).
#[test]
fn settle_proven_denial_events_are_rate_limited_per_sender() {
    let fx = fx("evrate");
    let t = agent_pane(&fx, "worker-1", P + 80);
    let _x = pane(&fx, "worker-2", P + 81);
    let _y = pane(&fx, "worker-3", P + 82);
    let _z = pane(&fx, "worker-4", P + 83);
    assert_eq!(direct(&fx, Some(P + 81), &t, "M|x|AAA")["ok"], json!(true)); // 짝 Return 을 기다리는 기계 본문
    let n0 = fx.daemon.bus.tail(400).iter().filter(|e| e["name"] == "queue.draft_gate_denied").count();
    for _ in 0..5 {
        let y = direct(&fx, Some(P + 82), &t, "M|y|BBB");
        assert!(settle_hint(&y).is_some(), "응답은 매번 증명을 싣는다: {y}");
    }
    let z = direct(&fx, Some(P + 83), &t, "M|z|CCC");
    assert!(settle_hint(&z).is_some(), "{z}");
    let evs: Vec<Value> = fx.daemon.bus.tail(400).into_iter().filter(|e| e["name"] == "queue.draft_gate_denied").collect();
    assert_eq!(evs.len() - n0, 2, "Y 5회 → 1건 · Z 1회 → 1건: {evs:?}");
}

// ═══════════ ★(0.14.42 · 수정 2회차 FV1-1) kill-switch pause 와 정착 재시도 ═══════════
//
// 【무엇이 틀렸었나】 정착 증명은 pause 를 보지 않았다. pause 중 경쟁으로 거부된 발신도 증명을 받아 신 CLI 가 예산
// (기본 3s · 최대 10s) 안에서 **재시도로 좌석에 직접** 썼고, 뒤따른 짝 Return 이 그 본문을 제출했다(S93 5/5 ·
// pause 응답 뒤 309~490ms 기록). 수정 전(bc954dc2)에는 같은 거부가 곧바로 `--queued` 로 넘어가 pause 동안 동결됐다.
// 직접 send 첫 요청은 원래부터 pause 판정 대상이 아니다(pause 중 '보고' 허용 — 종전 그대로). 막는 것은 **경쟁으로
// 거부된 발신이 재시도로 pause 를 뚫는 창**뿐이다: ① pause 중 거부에는 증명을 붙이지 않는다(CLI 재시도 0회 → 종전 큐
// 전환) ② 증명을 받은 뒤 pause 가 걸린 재시도(`settle_retry:true`)는 줄이 비어 있어도 쓰지 않고 증명 없이 거부한다.

/// ① pause 중에는 어떤 거부에도 정착 증명이 없다 — 계수 축(짝 Return 을 기다리는 기계 본문)·분리 보류 둘 다.
/// 분리 보류 거부 자체(쓰기 0)는 유지한다. resume 뒤에는 증명이 돌아온다(영구화 0).
#[test]
fn fv1_paused_daemon_gives_no_settle_proof_but_keeps_hold() {
    let fx = fx("fv1-pause-proof");
    let t = agent_pane(&fx, "worker-1", P + 90);
    let _x = pane(&fx, "worker-2", P + 91);
    let _y = pane(&fx, "worker-3", P + 92);
    assert_eq!(direct(&fx, Some(P + 91), &t, "M|x|AAA")["ok"], json!(true)); // 짝 Return 을 기다리는 기계 본문
    fx.daemon.paused.store(true, Ordering::SeqCst);
    let before = counts(&t);
    let yp = direct(&fx, Some(P + 92), &t, "M|y|BBB");
    assert_eq!(yp["ok"], json!(false), "{yp}");
    assert!(msg(&yp).contains(cys::MSG_TYPING_GUARD), "CLI 가 --queued 로 1회 전환하는 문구 접두: {yp}");
    assert!(msg(&yp).contains("[draft_gate:pending_input]"), "사유는 종전 그대로: {yp}");
    assert_eq!(settle_hint(&yp), None, "pause 중 거부에 정착 증명이 붙었다 — CLI 가 재시도로 pause 를 뚫는다: {yp}");
    assert_eq!(cys::send_settle_hint_ms(&msg(&yp)), None, "lib 파서로도 증명 없음: {yp}");
    let ev = last_denied(&fx).expect("거부 이벤트");
    assert!(ev["payload"].get("settle_ms").is_none(), "pause 중 거부 이벤트에 증명 키가 없다: {ev}");
    assert_eq!(counts(&t), before, "쓰기 0");
    // 분리 보류 — X 의 짝 Return(send-key 는 pause 판정 대상이 아니다 · 종전 그대로) 직후 Y 는 여전히 보류(쓰기 0).
    assert_eq!(pair_return(&fx, Some(P + 91), &t)["result"]["sent"], json!(true));
    let before = counts(&t);
    let yh = direct(&fx, Some(P + 92), &t, "M|y|BBB");
    assert_eq!(yh["ok"], json!(false), "분리 보류는 pause 와 무관하게 유지(대기 CR 뒤에 붙이지 않는다): {yh}");
    assert!(msg(&yh).contains("[draft_gate:submit_settling]"), "{yh}");
    assert_eq!(settle_hint(&yh), None, "pause 중 보류 거부에도 증명 없음: {yh}");
    let ev = last_denied(&fx).expect("보류 이벤트");
    assert_eq!(ev["payload"]["reason"], json!("submit_settling"), "{ev}");
    assert!(ev["payload"].get("settle_ms").is_none(), "{ev}");
    assert_eq!(counts(&t), before, "보류는 쓰기 0");
    assert_eq!(qlen(&t), 0, "보류는 적재하지 않는다(큐 전환은 CLI 몫)");
    // resume 뒤 — 증명이 돌아온다(다음 짝 Return 대기 본문 위에서).
    fx.daemon.paused.store(false, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(400)); // X 의 CR 이 쓰이고 분리 창이 지난다
    assert_eq!(direct(&fx, Some(P + 91), &t, "M|x|CCC")["ok"], json!(true));
    let yr = direct(&fx, Some(P + 92), &t, "M|y|BBB");
    assert!(settle_hint(&yr).is_some(), "resume 뒤에는 증명이 돌아온다(pause 영구화 0): {yr}");
}

/// ② 증명을 받은 뒤 pause 가 걸린 **정착 재시도**(`settle_retry:true`)는 줄이 비어 있어도 쓰지 않는다 — 증명 없는
/// 타이핑 가드 거부(`[draft_gate:paused]`)라 CLI 는 종전처럼 `--queued` 로 1회 넘기고 본문은 pause 동안 동결된다.
/// 대조: pause 가 아니면 같은 재시도가 쓰이고, pause 중에도 **첫 요청**(재시도 아님)은 종전처럼 직접 쓰인다(보고 경로).
#[test]
fn fv1_settle_retry_refused_while_paused_even_on_free_line() {
    let fx = fx("fv1-pause-retry");
    let t = agent_pane(&fx, "worker-1", P + 100);
    let t2 = agent_pane(&fx, "worker-4", P + 103);
    let _y = pane(&fx, "worker-3", P + 102);
    let retry = |t: &Arc<Surface>| {
        rpc(&fx, Some(P + 102), "surface.send_text", json!({
            "surface_id": t.id, "text": "M|y|BBB", "queued": false, "quiet": true, "settle_retry": true,
        }))
    };
    fx.daemon.paused.store(true, Ordering::SeqCst);
    let before = counts(&t);
    let r = retry(&t);
    assert_eq!(r["ok"], json!(false), "pause 중 정착 재시도가 빈 줄에 직접 쓰였다(pause 를 뚫는다): {r}");
    let m = msg(&r);
    assert!(m.contains(cys::MSG_TYPING_GUARD), "CLI 의 --queued 1회 전환 문구 접두: {m}");
    assert!(m.contains("[draft_gate:paused]"), "사유 태그: {m}");
    assert_eq!(cys::send_settle_hint_ms(&m), None, "증명 없음 → 재시도 0회: {m}");
    assert_eq!(counts(&t), before, "쓰기 0(세대 불변)");
    assert_eq!(qlen(&t), 0, "적재 0(큐 전환은 CLI 몫)");
    let ev = last_denied(&fx).expect("거부 이벤트");
    assert_eq!(ev["payload"]["reason"], json!("paused"), "{ev}");
    assert!(ev["payload"].get("settle_ms").is_none(), "{ev}");
    // pause 중 첫 요청(재시도 아님)은 종전 그대로 직접 쓴다 — 직접 send 는 원래 pause 판정 대상이 아니다.
    let first = direct(&fx, Some(P + 102), &t2, "M|y|DDD");
    assert_eq!(first["ok"], json!(true), "pause 중 첫 직접 send(보고)는 종전처럼 통과: {first}");
    // 대조: resume 뒤 같은 재시도는 쓰인다.
    fx.daemon.paused.store(false, Ordering::SeqCst);
    let r2 = retry(&t);
    assert_eq!(r2["ok"], json!(true), "pause 가 아니면 재시도는 종전처럼 쓴다: {r2}");
    assert_ne!(counts(&t), before, "쓰기 1");
}

// ═══════════ ★(0.14.42 · 수정 2회차 F1) 제출 CR 보류 — 인계 뒤 뜬 질문·선택 창을 누르지 않는다 ═══════════
//
// 【무엇이 틀렸었나】 정착 재시도는 경쟁에서 진 본문을 앞 제출 CR 바로 뒤(≈80~120ms)에 직접 넣고, 그 짝 Return 의 CR 은
// writer 가 최소 간격(150ms) 뒤에 쓴다. 에이전트가 앞 제출에 반응해 그 사이에 권한 창을 띄우면 CR 이 '1. Yes' 를
// 누른다(S92 x=90~250ms 26/31 · 수정 전 0/31). Modal 판정은 Text 팔뿐이고, writer 의 SubmitAfterGap arm 은 간격을
// 잔 뒤 화면을 다시 보지 않고 CR 을 썼다. 1인 발신의 같은 창(F3 · 본문→CR 사이 창)도 같은 자리에서 열려 있었다.
// 【규칙】 writer 가 제출 CR 을 쓰기 **직전** 화면을 다시 본다 — 질문·선택 창이 전경이고 커서행이 우리 기계 본문으로
// 설명되지 않으면 그 CR 을 쓰지 않는다(이벤트 `queue.submit_withheld`). 승인 조작은 막지 않는다: 인계 시점에 이미 창이
// 보였고 줄 위에 우리 기계 본문이 없으면(= 보이는 창에 대한 Return · 선택지 조작 뒤 Return) 탐침을 걸지 않는다.

/// vt100 파서에 화면을 직접 먹인다(PTY 프로그램 무관) — governance 검체 `paint` 와 같은 규율.
fn paint(s: &Arc<Surface>, lines: &[&str], row: u16, col: u16) {
    let mut p = s.parser.lock().unwrap_or_else(|e| e.into_inner());
    p.process(b"\x1b[2J\x1b[H");
    for (i, l) in lines.iter().enumerate() {
        p.process(format!("\x1b[{};1H{}", i + 1, l).as_bytes());
    }
    p.process(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
}

/// 가짜 에이전트·실 claude 권한 창 모양 — 커서는 `❯ 1. Yes` 행 끝.
const DIALOG: [&str; 4] = ["Do you want to proceed?", "❯ 1. Yes", "  2. No", "  Esc to cancel"];

fn settle_obs(t: &Arc<Surface>) -> crate::state::SubmitSettleObs {
    t.inject_track.submit_settle_obs(crate::state::settle_mono_ms(), 2000)
}

/// writer 가 인계된 제출 CR 요청을 소비할 때까지(최대 3s) — 소비 뒤 관측을 돌려준다.
fn wait_submit_consumed(t: &Arc<Surface>) -> crate::state::SubmitSettleObs {
    let deadline = Instant::now() + Duration::from_secs(3);
    while settle_obs(t).inflight && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    settle_obs(t)
}

fn withheld(fx: &Fx) -> Vec<Value> {
    fx.daemon.bus.tail(400).into_iter().filter(|e| e["name"] == "queue.submit_withheld").collect()
}

/// 로그인 초기 출력이 픽스처 화면을 덮지 않게 안정화한 에이전트 좌석(governance `probe_seat` 와 같은 규율).
fn agent_pane_settled(fx: &Fx, role: &str, pid: u32) -> Arc<Surface> {
    let s = agent_pane(fx, role, pid);
    std::thread::sleep(Duration::from_millis(600));
    s
}

/// 적색→녹색: X 의 본문이 쓰인 뒤·짝 Return 앞에 권한 창이 떴다(렌더 지연 창) — X 의 CR 은 쓰이지 않는다.
/// 종전: writer 가 간격을 잔 뒤 화면을 보지 않고 CR 을 써 '1. Yes' 를 눌렀다(오승인).
#[test]
fn f1_submit_cr_withheld_when_dialog_rose_after_own_body() {
    let fx = fx("f1-withhold");
    let t = agent_pane_settled(&fx, "worker-1", P + 110);
    let _x = pane(&fx, "worker-2", P + 111);
    assert_eq!(direct(&fx, Some(P + 111), &t, "M|x|AAA")["ok"], json!(true));
    std::thread::sleep(Duration::from_millis(60)); // 본문 PTY 에코가 파서에 닿은 뒤 화면을 덮는다
    paint(&t, &DIALOG, 1, 8);
    let xr = pair_return(&fx, Some(P + 111), &t);
    assert_eq!(xr["result"]["sent"], json!(true), "Return 요청 응답은 종전 그대로(쓰기 결정은 writer 의 몫): {xr}");
    let o = wait_submit_consumed(&t);
    assert!(!o.inflight, "writer 가 CR 요청을 소비했다: {o:?}");
    assert_eq!(o.since_written_ms, None, "창이 뜬 좌석에 제출 CR 을 썼다 — '1. Yes' 오승인: {o:?}");
    let ev = withheld(&fx);
    assert_eq!(ev.len(), 1, "보류 사실 1건: {ev:?}");
    assert_eq!(ev[0]["payload"]["surface_ref"], json!(cys::surface_ref(t.id)), "{ev:?}");
    assert_eq!(ev[0]["payload"]["armed_by"], json!("machine_body"), "{ev:?}");
}

/// 적색→녹색(재개 · S94 실측): 좌석 캐시(watchdog 5초 틱)가 에이전트가 앉기 **전** 틱의 `Empty` 로 낡은 좌석 —
/// 에이전트 미확인(set_meta 가 내린 `agent_seen=false`). 종전에는 탐침을 걸지 않아(생존 술어 거짓) 같은 창 경합에서
/// CR 이 '1. Yes' 를 눌렀다(S94 좌석 empty 시행만 오승인). 커서가 선택지 행이면 쓰지 않는다.
#[test]
fn f1_submit_cr_withheld_on_stale_empty_seat_before_first_agent_sighting() {
    let fx = fx("f1-stale");
    let t = agent_pane_settled(&fx, "worker-1", P + 115);
    t.seat_cache.store(crate::governance::SeatState::Empty.as_u8(), Ordering::Relaxed);
    t.agent_seen.store(false, Ordering::Relaxed);
    let _x = pane(&fx, "worker-2", P + 116);
    assert_eq!(direct(&fx, Some(P + 116), &t, "M|x|AAA")["ok"], json!(true));
    std::thread::sleep(Duration::from_millis(60));
    paint(&t, &DIALOG, 1, 8);
    assert_eq!(pair_return(&fx, Some(P + 116), &t)["result"]["sent"], json!(true));
    let o = wait_submit_consumed(&t);
    assert!(!o.inflight, "writer 가 CR 요청을 소비했다: {o:?}");
    assert_eq!(o.since_written_ms, None, "낡은 Empty 좌석에서 창 위 제출 CR 을 썼다 — '1. Yes' 오승인: {o:?}");
    let ev = withheld(&fx);
    assert_eq!(ev.len(), 1, "보류 사실 1건: {ev:?}");
}

fn named(fx: &Fx, name: &str) -> Vec<Value> {
    fx.daemon.bus.tail(400).into_iter().filter(|e| e["name"] == name).collect()
}

/// X 의 본문 → 권한 창 → X 의 짝 Return(보류)까지 만든 좌석. 반환 = 보류 뒤 관측(쓴 시각 없음 확인 끝).
fn withheld_seat(fx: &Fx, pid: u32) -> Arc<Surface> {
    let t = agent_pane_settled(fx, "worker-1", pid);
    let _x = pane(fx, "worker-2", pid + 1);
    assert_eq!(direct(fx, Some(pid + 1), &t, "M|x|AAA")["ok"], json!(true));
    std::thread::sleep(Duration::from_millis(60));
    paint(&t, &DIALOG, 1, 8);
    assert_eq!(pair_return(fx, Some(pid + 1), &t)["result"]["sent"], json!(true));
    let o = wait_submit_consumed(&t);
    assert_eq!(o.since_written_ms, None, "전제: 창 위 제출 CR 은 보류된다: {o:?}");
    assert_eq!(withheld(fx).len(), 1, "전제: 보류 1건");
    t
}

/// 입력줄이 우리 본문 그대로인 유휴 composer(창이 닫힌 뒤의 화면).
fn paint_own_composer(t: &Arc<Surface>) {
    paint(t, &["────────", "❯ M|x|AAA", "────────"], 1, 11);
}

/// 적색→녹색(재개 · S91 dialog 실측): 보류한 제출 CR 은 **버림이 아니라 미룸**이다. 창이 닫히고 입력줄이 그 기계 본문
/// 그대로면 watchdog 틱이 그 CR 을 **한 번** 다시 쓴다. 종전(88a63e22)에는 본문이 입력줄에 남아 큐 틱이 입력줄 점유로
/// 그 좌석을 통째로 세웠다(S91 dialog 폭풍: 150초 동안 큐 배달 0 · 수정 전 판 10건).
#[test]
fn f1_withheld_submit_is_resubmitted_once_after_dialog_closes() {
    let fx = fx("f1-resubmit");
    let t = withheld_seat(&fx, P + 180);
    // 창이 아직 떠 있으면 다시 쓰지 않는다(기록은 남긴다).
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(settle_obs(&t).since_written_ms, None, "창이 떠 있는데 보류 CR 을 다시 썼다 — 오승인");
    assert!(named(&fx, "queue.submit_resubmitted").is_empty());
    // 창이 닫히고 입력줄이 우리 본문 그대로 — 한 번 다시 쓴다.
    paint_own_composer(&t);
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "창이 닫힌 뒤에도 보류 CR 을 다시 쓰지 않았다 — 입력줄 잔여로 큐가 선다: {o:?}");
    let ev = named(&fx, "queue.submit_resubmitted");
    assert_eq!(ev.len(), 1, "재제출 1건: {ev:?}");
    assert_eq!(ev[0]["payload"]["surface_ref"], json!(cys::surface_ref(t.id)), "{ev:?}");
    // 한 번뿐이다(기록 소비) — TUI 가 그 CR 을 삼켜 입력줄이 그대로여도 다시 쓰지 않는다(5초 틱마다 CR 을 되풀이하는 폭주 0).
    std::thread::sleep(Duration::from_millis(200));
    paint_own_composer(&t);
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(named(&fx, "queue.submit_resubmitted").len(), 1, "보류 CR 을 두 번 썼다");
}

/// 음성 대조: kill-switch pause 중에는 다시 쓰지 않고(기록 유지 — resume 뒤 재개), 사람 손이 닿은 줄은 버린다(사람 몫).
#[test]
fn f1_withheld_submit_waits_for_resume_and_yields_to_human() {
    let fx = fx("f1-resubmit-guard");
    let t = withheld_seat(&fx, P + 190);
    paint_own_composer(&t);
    fx.daemon.paused.store(true, Ordering::SeqCst);
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(settle_obs(&t).since_written_ms, None, "pause 중에 보류 CR 을 썼다 — kill-switch 약화");
    fx.daemon.paused.store(false, Ordering::SeqCst);
    // 사람이 본문 뒤에 손을 댔다 — 그 줄은 이제 사람 몫이다(쓰지 않고 기록을 버린다).
    *t.last_human_input.lock().unwrap() = Some(Instant::now());
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(settle_obs(&t).since_written_ms, None, "사람 손이 닿은 줄에 보류 CR 을 썼다");
    let d = named(&fx, "queue.submit_withheld_dropped");
    assert_eq!(d.len(), 1, "버린 사실 1건: {d:?}");
    assert_eq!(d[0]["payload"]["reason"], json!("human"), "{d:?}");
    *t.last_human_input.lock().unwrap() = None;
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    std::thread::sleep(Duration::from_millis(300));
    assert!(named(&fx, "queue.submit_resubmitted").is_empty(), "버린 기록이 되살아났다");
}

/// 음성 대조: 보류 뒤 새 기계 본문이 들어왔으면(데몬 주입의 병합 제출 · 다른 send) 그 기록은 낡았다 — 버린다.
#[test]
fn f1_withheld_submit_dropped_after_newer_machine_body() {
    let fx = fx("f1-resubmit-newer");
    let t = withheld_seat(&fx, P + 200);
    t.inject_track.note_body("M|y|BBB", None);
    paint(&t, &["────────", "❯ M|x|AAA", "────────"], 1, 11);
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(settle_obs(&t).since_written_ms, None, "낡은 보류 기록으로 CR 을 썼다");
    let d = named(&fx, "queue.submit_withheld_dropped");
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["payload"]["reason"], json!("newer_body"), "{d:?}");
}

/// 음성 대조(③ 방향): 에이전트를 **본 뒤**의 `Empty`(사망 · 종료 통지 전) 좌석은 종전 — 창 잔상이 있어도 쓴다.
#[test]
fn f1_seat_empty_after_agent_was_seen_is_unchanged() {
    let fx = fx("f1-deadseat");
    let t = agent_pane_settled(&fx, "worker-1", P + 117);
    t.seat_cache.store(crate::governance::SeatState::Empty.as_u8(), Ordering::Relaxed);
    t.agent_seen.store(true, Ordering::Relaxed);
    let _x = pane(&fx, "worker-2", P + 118);
    assert_eq!(direct(&fx, Some(P + 118), &t, "M|x|AAA")["ok"], json!(true));
    std::thread::sleep(Duration::from_millis(60));
    paint(&t, &DIALOG, 1, 8);
    assert_eq!(pair_return(&fx, Some(P + 118), &t)["result"]["sent"], json!(true));
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "사망 좌석(에이전트를 본 뒤 Empty)은 종전처럼 쓴다: {o:?}");
    assert!(withheld(&fx).is_empty(), "{:?}", withheld(&fx));
}

/// 음성 대조(치명 방향 — 워커 hang): 이미 보이는 창에 대한 Return(master 의 승인 조작)은 쓴다.
#[test]
fn f1_approval_return_on_visible_dialog_is_written() {
    let fx = fx("f1-approve");
    let t = agent_pane_settled(&fx, "worker-1", P + 120);
    let _m = pane(&fx, "master", P + 121);
    paint(&t, &DIALOG, 1, 8);
    let r = pair_return(&fx, Some(P + 121), &t);
    assert_eq!(r["result"]["sent"], json!(true), "{r}");
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "보이는 창에 대한 Return 을 막았다 — 승인 수단 소실(워커 hang): {o:?}");
    assert!(withheld(&fx).is_empty(), "승인 조작에 보류 이벤트: {:?}", withheld(&fx));
}

/// 음성 대조: 선택지 조작(`Down`) 뒤 Return 도 승인 조작이다 — 쓴다(키가 입력 세대를 올려 본문 소유가 끊긴다).
#[test]
fn f1_selector_navigation_then_return_is_written() {
    let fx = fx("f1-nav");
    let t = agent_pane_settled(&fx, "worker-1", P + 130);
    let _m = pane(&fx, "master", P + 131);
    paint(&t, &DIALOG, 1, 8);
    assert_eq!(rpc(&fx, Some(P + 131), "surface.send_key", json!({"surface_id": t.id, "key": "Down"}))["ok"], json!(true));
    paint(&t, &["Do you want to proceed?", "  1. Yes", "❯ 2. No", "  Esc to cancel"], 2, 7);
    let r = pair_return(&fx, Some(P + 131), &t);
    assert_eq!(r["result"]["sent"], json!(true), "{r}");
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "선택지 조작 뒤 Return 을 막았다: {o:?}");
    assert!(withheld(&fx).is_empty(), "{:?}", withheld(&fx));
}

/// 음성 대조(조용한 유실 방향): 본문이 선택지 어휘(`1. Yes …`)로 시작해도 커서행이 **우리 본문**이면 composer 다 — 쓴다.
#[test]
fn f1_composer_body_that_looks_like_a_choice_is_submitted() {
    let fx = fx("f1-quote");
    let t = agent_pane_settled(&fx, "worker-1", P + 140);
    let _x = pane(&fx, "worker-2", P + 141);
    assert_eq!(direct(&fx, Some(P + 141), &t, "1. Yes please")["ok"], json!(true));
    std::thread::sleep(Duration::from_millis(60));
    paint(&t, &["────────", "❯ 1. Yes please", "────────"], 1, 15);
    let r = pair_return(&fx, Some(P + 141), &t);
    assert_eq!(r["result"]["sent"], json!(true), "{r}");
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "composer 의 우리 본문을 창으로 오인해 CR 을 막았다: {o:?}");
    assert!(withheld(&fx).is_empty(), "{:?}", withheld(&fx));
}

/// 음성 대조: 평범한 유휴 composer(우리 본문) — 종전처럼 쓴다.
#[test]
fn f1_plain_composer_submit_is_written() {
    let fx = fx("f1-plain");
    let t = agent_pane_settled(&fx, "worker-1", P + 150);
    let _x = pane(&fx, "worker-2", P + 151);
    assert_eq!(direct(&fx, Some(P + 151), &t, "M|x|AAA")["ok"], json!(true));
    std::thread::sleep(Duration::from_millis(60));
    paint(&t, &["────────", "❯ M|x|AAA", "────────"], 1, 11);
    assert_eq!(pair_return(&fx, Some(P + 151), &t)["result"]["sent"], json!(true));
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "{o:?}");
    assert!(withheld(&fx).is_empty(), "{:?}", withheld(&fx));
}

/// 킬 스위치(`CYS_SEND_SETTLE=0`) — 제출 CR 보류도 끈다(0.14.42 A2 트리 동작 = 한 노브 롤백).
#[test]
fn f1_kill_switch_disables_submit_cr_withhold() {
    let fx = fx("f1-kill");
    std::env::set_var("CYS_SEND_SETTLE", "0");
    let t = agent_pane_settled(&fx, "worker-1", P + 160);
    let _x = pane(&fx, "worker-2", P + 161);
    assert_eq!(direct(&fx, Some(P + 161), &t, "M|x|AAA")["ok"], json!(true));
    std::thread::sleep(Duration::from_millis(60));
    paint(&t, &DIALOG, 1, 8);
    assert_eq!(pair_return(&fx, Some(P + 161), &t)["result"]["sent"], json!(true));
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "끔이면 종전처럼 쓴다: {o:?}");
    assert!(withheld(&fx).is_empty());
}

/// 셸 좌석(agent_meta 없음)은 무변경 — 모달 판정 대상이 아니다.
#[test]
fn f1_shell_seat_submit_cr_unchanged() {
    let fx = fx("f1-shell");
    let t = pane(&fx, "worker-1", P + 170);
    let _x = pane(&fx, "worker-2", P + 171);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(direct(&fx, Some(P + 171), &t, "echo a")["ok"], json!(true));
    std::thread::sleep(Duration::from_millis(60));
    paint(&t, &DIALOG, 1, 8);
    assert_eq!(pair_return(&fx, Some(P + 171), &t)["result"]["sent"], json!(true));
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "셸 좌석은 종전처럼 쓴다: {o:?}");
    assert!(withheld(&fx).is_empty());
}
