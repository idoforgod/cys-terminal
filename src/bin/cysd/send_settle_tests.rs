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
    /// ★(R3C-1) 보류 기록 시계의 가상 경과 — 재제출 틱에 `Instant::now() + 이 값`을 싣는다(큰 `Instant` 뺄셈 없이 분·시간
    /// 경과를 모사 · 부팅 직후 CI 에서도 underflow 패닉 없음).
    vclock: std::cell::Cell<Duration>,
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
    Fx { daemon, dir, vclock: std::cell::Cell::new(Duration::ZERO), _g: g }
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

// ═══════════ ★(0.14.42 · 수정 3회차 FV2-1) 접힌 줄·여러 줄 기계 본문의 제출 CR 을 창으로 오인하지 않는다 ═══════════
//
// 【무엇이 틀렸었나 — 적대 검증 2회차 FV2-1 · 실 claude 2.1.282 · Full 등급 좌석 15/15 보류(베이스 0/9)】 보류 탐침의
// '우리 본문이면 창이 아니다' 예외는 **커서행이 선두 마커로 시작할 때만** 성립했다. 실 claude 는 본문이 한 줄에 다 들어가지
// 않으면(긴 줄이 접힘 · 여러 줄) 커서를 마커 없는 **연속행**(2칸 들여쓰기)에 둔다 → 예외 불성립. 그런데 화면의 모달 어휘는
// 본문 첫 줄(`❯ 1. …`)이나 위에 남은 이전 메시지 에코(`❯ 1. …`)에서 나오고, `modal_left_behind` 는 입력줄이 빈 대기
// 프롬프트일 때만 과거 서명을 면제한다 → 창이 없는데 제출 Return 을 보류했다. 재제출도 같은 술어라 `wait_dialog` 에
// 영구히 머물렀고(상한 = 큐 TTL 6h) 그동안 그 좌석의 큐·H0 생산자·후속 직접 send 가 모두 섰다.
// 【규칙】 ① composer 설명은 **입력 블록 전체**(선두 마커 행 ~ 커서 · 연속행 이음 · 공백 무시 · claude 붙여넣기 자리표시
// `[Pasted text #N …]` 인정)로 본다. ② 입력 블록의 주인이 우리 기계 본문이면 그 블록 **위**의 서명은 전경이 아니다 —
// 블록 **아래**에 그려진 창 서명만 창이다. ③ 재제출 대기에 상한(기록 수명)을 둔다(넘으면 사유 이벤트와 함께 기록을 버린다).
// 창을 누르는 보호(본문 뒤·Return 앞에 뜬 창 · 커서가 창 행)는 그대로다(아래 음성 대조).

const RULE78: &str = "──────────────────────────────────────────────────────────────────────────────";

fn wcol(c: char) -> usize {
    if c.is_ascii() {
        1
    } else {
        2
    }
}

/// 한 논리 줄을 표시 폭 `width` 로 접는다 — 단어 경계(공백) 우선, 한 단어가 폭을 넘으면 글자 경계(한글 무공백·긴 토큰).
fn fold(line: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let (mut cur, mut cw) = (String::new(), 0usize);
    for word in line.split(' ') {
        let ww: usize = word.chars().map(wcol).sum();
        if !cur.is_empty() && cw + 1 + ww <= width {
            cur.push(' ');
            cur.push_str(word);
            cw += 1 + ww;
            continue;
        }
        if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cw = 0;
        }
        for c in word.chars() {
            if cw + wcol(c) > width {
                out.push(std::mem::take(&mut cur));
                cw = 0;
            }
            cur.push(c);
            cw += wcol(c);
        }
    }
    out.push(cur);
    out
}

/// 실 claude 2.1.282 입력 상자 형상(fatal-v2 실측 screen-end) — 이력 행들 · 괘선 · 첫 행 `❯ ` · 접힌/여러 줄 연속행은
/// 2칸 들여쓰기 · 괘선 · 상태줄 · (선택) 입력 상자 아래에 더 그린 행들. 커서는 본문 끝(마지막 composer 행의 글자 뒤).
/// 반환 = (행들, 커서 행, 커서 열).
fn claude_screen(history: &[&str], composer: &str, below_extra: &[&str]) -> (Vec<String>, u16, u16) {
    let mut rows: Vec<String> = history.iter().map(|s| s.to_string()).collect();
    rows.push(RULE78.to_string());
    let mut first = true;
    let (mut crow, mut ccol) = (0usize, 0usize);
    for logical in composer.split('\n') {
        for seg in fold(logical, 74) {
            let prefix = if first { "❯ " } else { "  " };
            first = false;
            ccol = 2 + seg.chars().map(wcol).sum::<usize>();
            rows.push(format!("{prefix}{seg}"));
            crow = rows.len() - 1;
        }
    }
    rows.push(RULE78.to_string());
    rows.push("  ⏸ manual mode on".to_string());
    rows.extend(below_extra.iter().map(|s| s.to_string()));
    assert!(rows.len() <= 24, "픽스처가 24행 좌석을 넘는다: {}", rows.len());
    (rows, crow as u16, ccol as u16)
}

fn paint_rows(t: &Arc<Surface>, sc: &(Vec<String>, u16, u16)) {
    let lines: Vec<&str> = sc.0.iter().map(|s| s.as_str()).collect();
    paint(t, &lines, sc.1, sc.2);
}

const W1: &str = "1. W1BODY please review the following status report carefully and reply with a short \
                  acknowledgement when done please review the following status report carefully and reply \
                  with a short acknowledgement when done";

/// 발신 좌석의 본문 → (에코가 파서에 닿은 뒤) 화면 `sc` → 짝 Return. 반환 = CR 소비 뒤 관측.
fn body_then_return(fx: &Fx, t: &Arc<Surface>, from: u32, body: &str, sc: &(Vec<String>, u16, u16)) -> crate::state::SubmitSettleObs {
    let r = direct(fx, Some(from), t, body);
    assert_eq!(r["ok"], json!(true), "전제: 본문 직접 send 통과: {r}");
    std::thread::sleep(Duration::from_millis(60));
    paint_rows(t, sc);
    assert_eq!(pair_return(fx, Some(from), t)["result"]["sent"], json!(true));
    let o = wait_submit_consumed(t);
    assert!(!o.inflight, "writer 가 CR 요청을 소비했다: {o:?}");
    o
}

/// 적색→녹색(FV2-1 W1·W1d·W1dL·W1dC·W1dS*): 번호로 시작하는 긴 본문이 접혀 커서가 연속행 — 창 없음. 종전: 보류(15/15).
#[test]
fn fv2_wrapped_numbered_body_on_full_seat_is_submitted() {
    let fx = fx("fv2-w1");
    let t = agent_pane_settled(&fx, "worker-1", P + 300);
    let _x = pane(&fx, "worker-2", P + 301);
    let sc = claude_screen(&[], W1, &[]);
    assert!(sc.0.len() >= 5, "전제: 본문이 접혔다(연속행 ≥2): {:?}", sc.0);
    let o = body_then_return(&fx, &t, P + 301, W1, &sc);
    assert!(o.since_written_ms.is_some(), "창이 없는데 접힌 우리 본문의 제출 CR 을 보류했다(FV2-1): {o:?}");
    assert!(withheld(&fx).is_empty(), "{:?}", withheld(&fx));
}

/// 적색→녹색(FV2-1 W2d): 위에 이전 메시지 에코 `❯ 1. …` 가 남은 화면 + 번호 없는 긴 본문(접힘). 종전: 보류.
#[test]
fn fv2_wrapped_body_under_numbered_echo_is_submitted() {
    let fx = fx("fv2-w2");
    let t = agent_pane_settled(&fx, "worker-1", P + 302);
    let _x = pane(&fx, "worker-2", P + 303);
    let body = "W2LONG please review the following status report carefully and reply with a short \
                acknowledgement when done please review the following status report carefully";
    let sc = claude_screen(&["❯ 1. W2SHORT item", "", "⏺ ok", "", "✻ Cooked for 0s · done 7:23 PM"], body, &[]);
    let o = body_then_return(&fx, &t, P + 303, body, &sc);
    assert!(o.since_written_ms.is_some(), "이력 에코 서명으로 접힌 우리 본문의 제출 CR 을 보류했다(FV2-1 W2d): {o:?}");
    assert!(withheld(&fx).is_empty(), "{:?}", withheld(&fx));
}

/// 적색→녹색(FV2-1 M1·M1d): 3줄 번호 목록 — 줄마다 연속행 · 커서는 마지막 줄 끝. 종전: 보류.
#[test]
fn fv2_multiline_numbered_body_is_submitted() {
    let fx = fx("fv2-m1");
    let t = agent_pane_settled(&fx, "worker-1", P + 304);
    let _x = pane(&fx, "worker-2", P + 305);
    let body = "1. MLBODY first item of the report\n2. second item of the report\n3. third item of the report";
    let sc = claude_screen(&[], body, &[]);
    let o = body_then_return(&fx, &t, P + 305, body, &sc);
    assert!(o.since_written_ms.is_some(), "여러 줄 번호 목록 본문의 제출 CR 을 보류했다(FV2-1 M1): {o:?}");
    assert!(withheld(&fx).is_empty(), "{:?}", withheld(&fx));
}

/// 적색→녹색: 800자 초과 또는 줄바꿈 2개 초과 붙여넣기는 실 claude 가 `[Pasted text #N +M lines]` 로 접어 그린다(2.1.282
/// 바이너리 실측 문자열 · 임계 O5=800자 · 줄바꿈 2). 이력에 번호 에코가 있으면 종전은 보류했다.
/// (검체 좌석은 입력을 읽지 않는 `sleep` 이라 tty 입력 버퍼(≈1KiB)를 넘는 본문은 writer 를 막는다 — 줄 수 축으로 만든다.)
#[test]
fn fv2_pasted_text_placeholder_under_numbered_echo_is_submitted() {
    let fx = fx("fv2-paste");
    let t = agent_pane_settled(&fx, "worker-1", P + 306);
    let _x = pane(&fx, "worker-2", P + 307);
    let line = "PASTEBODY status line";
    let body = format!("{line} a\n{line} b\n{line} c\n{line} d");
    assert_eq!(body.matches('\n').count(), 3, "전제: 줄바꿈 3 > 2 → 자리표시");
    let sc = claude_screen(&["❯ 1. earlier numbered request", "", "⏺ ok"], "[Pasted text #1 +3 lines] ", &[]);
    let o = body_then_return(&fx, &t, P + 307, &body, &sc);
    assert!(o.since_written_ms.is_some(), "붙여넣기 자리표시 composer 의 제출 CR 을 보류했다: {o:?}");
    assert!(withheld(&fx).is_empty(), "{:?}", withheld(&fx));
}

/// 적색→녹색: 공백 없는 한글 본문은 글자 경계에서 접힌다(단어 안 줄바꿈) — 이음은 공백 무시로 대조한다.
#[test]
fn fv2_wrapped_korean_body_split_midword_is_submitted() {
    let fx = fx("fv2-ko");
    let t = agent_pane_settled(&fx, "worker-1", P + 308);
    let _x = pane(&fx, "worker-2", P + 309);
    let body = "1. 다음상태보고서를주의깊게검토하고끝나면짧은확인응답을보내주세요".repeat(3);
    let sc = claude_screen(&["❯ 1. 앞선 번호 메시지"], &body, &[]);
    assert!(sc.0.len() >= 6, "전제: 한글 본문이 단어 안에서 접혔다: {:?}", sc.0);
    let o = body_then_return(&fx, &t, P + 309, &body, &sc);
    assert!(o.since_written_ms.is_some(), "단어 안에서 접힌 한글 본문의 제출 CR 을 보류했다: {o:?}");
    assert!(withheld(&fx).is_empty(), "{:?}", withheld(&fx));
}

/// 실 claude 권한 창(입력 상자를 대체) — 커서 행 선택 가능.
fn claude_dialog(cursor_on_last_option: bool) -> (Vec<String>, u16, u16) {
    let rows: Vec<String> = [
        "⏺ Bash(rm -rf build)",
        RULE78,
        " Bash command",
        "",
        "   rm -rf build",
        "",
        " Do you want to proceed?",
        " ❯ 1. Yes",
        "   2. Yes, and don't ask again for rm commands in this project",
        "   3. No, and tell Claude what to do differently (esc)",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if cursor_on_last_option {
        (rows, 9, 55)
    } else {
        (rows, 7, 9)
    }
}

/// 음성 대조(치명 방향 — 오승인): 접힌 우리 본문 뒤·Return 앞에 권한 창이 입력 상자를 대체했다 — 커서가 선택지 행이든
/// 그 아래 선택지(들여쓴 연속행 모양)든 보류한다(블록 이음이 창 행을 우리 본문으로 설명하지 않는다).
#[test]
fn fv2_dialog_replacing_wrapped_body_is_still_withheld() {
    for (i, last) in [false, true].into_iter().enumerate() {
        let fx = fx("fv2-dlg");
        let t = agent_pane_settled(&fx, "worker-1", P + 310 + 2 * i as u32);
        let _x = pane(&fx, "worker-2", P + 311 + 2 * i as u32);
        let o = body_then_return(&fx, &t, P + 311 + 2 * i as u32, W1, &claude_dialog(last));
        assert_eq!(o.since_written_ms, None, "창 위 제출 CR 을 썼다 — 오승인(커서 마지막 선택지={last}): {o:?}");
        assert_eq!(withheld(&fx).len(), 1, "보류 1건(커서 마지막 선택지={last})");
    }
}

/// 음성 대조(② 의 경계): 입력 블록의 주인이 우리 본문이어도 블록 **아래**에 창 서명이 그려져 있으면 창이다(보류).
#[test]
fn fv2_dialog_drawn_below_own_composer_is_still_withheld() {
    let fx = fx("fv2-below");
    let t = agent_pane_settled(&fx, "worker-1", P + 320);
    let _x = pane(&fx, "worker-2", P + 321);
    let sc = claude_screen(&[], W1, &[" Do you want to proceed?", " ❯ 1. Yes", "   2. No"]);
    let o = body_then_return(&fx, &t, P + 321, W1, &sc);
    assert_eq!(o.since_written_ms, None, "우리 composer 아래 그려진 창 위에 제출 CR 을 썼다: {o:?}");
    assert_eq!(withheld(&fx).len(), 1);
}

/// 권한 창으로 보류된 접힌 번호 본문 좌석(재제출 검체 전제).
fn withheld_wrapped_seat(fx: &Fx, pid: u32) -> Arc<Surface> {
    let t = agent_pane_settled(fx, "worker-1", pid);
    let _x = pane(fx, "worker-2", pid + 1);
    let o = body_then_return(fx, &t, pid + 1, W1, &claude_dialog(false));
    assert_eq!(o.since_written_ms, None, "전제: 창 위 제출 CR 은 보류된다: {o:?}");
    assert_eq!(withheld(fx).len(), 1, "전제: 보류 1건");
    t
}

/// 적색→녹색(FV2-1 재제출 wait_dialog 영구): 창이 닫히고 접힌 우리 번호 본문이 입력 상자에 돌아왔다 — 한 번 다시 쓴다.
/// 종전: 본문 첫 줄의 `❯ 1.` 서명을 창으로 보아 `wait_dialog` 에 영구히 머물렀다(상한 = 큐 TTL).
#[test]
fn fv2_withheld_wrapped_body_is_resubmitted_after_dialog_closes() {
    let fx = fx("fv2-resub");
    let t = withheld_wrapped_seat(&fx, P + 330);
    paint_rows(&t, &claude_screen(&["❯ 1. earlier numbered request", "", "⏺ ok"], W1, &[]));
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "창이 닫힌 뒤 접힌 우리 본문의 보류 CR 을 다시 쓰지 않았다(wait_dialog 영구): {o:?}");
    assert_eq!(named(&fx, "queue.submit_resubmitted").len(), 1);
}

/// 적색→녹색: 같은 재제출이 대체화면(실 claude 2.1.282 는 alt-screen 상주 · fatal-v2 surface.list alt_screen=true)에서도
/// 된다 — 레이아웃 양성 증거를 '빈 대기 프롬프트로 본 화면'에서 잰다(접힌 본문 연속행은 입력 상자 아래 꼬리가 아니다).
#[test]
fn fv2_withheld_wrapped_body_is_resubmitted_on_alt_screen() {
    let fx = fx("fv2-resub-alt");
    let t = withheld_wrapped_seat(&fx, P + 340);
    t.parser.lock().unwrap_or_else(|e| e.into_inner()).process(b"\x1b[?1049h");
    t.alt_screen.store(true, Ordering::Relaxed);
    *t.last_output.lock().unwrap() = Instant::now() - Duration::from_secs(30);
    paint_rows(&t, &claude_screen(&[], W1, &[]));
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "대체화면에서 접힌 우리 본문의 보류 CR 을 다시 쓰지 않았다: {o:?}");
    assert_eq!(named(&fx, "queue.submit_resubmitted").len(), 1);
}

/// 적색→녹색(FV2-1 fix ⓒ): 재제출 대기에 상한 — 창(또는 창으로 보이는 화면)이 오래 남으면 기록을 버리고 사유 1건.
/// 종전 상한은 큐 TTL(6h)뿐이었다. 쓰기는 0 이다(상한은 쓰지 않는 방향으로만 끝낸다).
/// (R3C-1: 상한은 창이 연달아 열린 시간만 잰다 — 창이 11분 떠 있는 것을 가상 시계로 모사한다.)
#[test]
fn fv2_withheld_record_dropped_at_wait_cap() {
    let fx = fx("fv2-cap");
    let t = withheld_wrapped_seat(&fx, P + 350);
    // 창은 그대로 떠 있다 — 상한 전에는 기다린다(기록 유지 · 쓰기 0).
    tick(&fx);
    assert!(t.inject_track.withheld().is_some(), "상한 전인데 기록을 버렸다");
    elapse(&fx, Duration::from_secs(11 * 60)); // 창이 11분 떠 있다(그동안 5초 틱마다 wait_dialog)
    tick(&fx);
    assert!(t.inject_track.withheld().is_none(), "상한(10분)을 넘긴 보류 기록이 남았다 — 큐 TTL 까지 wait_dialog");
    assert_eq!(settle_obs(&t).since_written_ms, None, "상한은 쓰지 않는 방향으로만 끝낸다");
    let d = named(&fx, "queue.submit_withheld_dropped");
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["payload"]["reason"], json!("wait_cap"), "{d:?}");
    assert!(
        d[0]["payload"]["dialog_open_ms"].as_u64().is_some_and(|ms| ms >= 600_000),
        "버림 이벤트가 창이 열려 있던 시간(≥10분)을 싣지 않았다: {d:?}"
    );
}

/// 적색→녹색(② 재제출 판정 일치): 입력 블록의 주인이 우리 본문이면 블록 **위** 서명은 전경이 아니다 — 입력 상자 아래에
/// 괘선·상태줄 대신 레이아웃 증거가 아닌 안내 행(실 claude 2.1.282 실측 `paste again to expand`)만 보이는 프레임에서도
/// 재제출은 보류 탐침과 같은 판정을 쓴다(공유 전경 술어로 재면 이력 에코 `❯ 1. …` 가 '과거'로 면제되지 않아 wait_prompt).
#[test]
fn fv2_resubmit_ignores_signatures_above_own_composer_without_trailer() {
    let fx = fx("fv2-resub-notrail");
    let t = withheld_wrapped_seat(&fx, P + 360);
    let (mut rows, cr, cc) = claude_screen(&["❯ 1. earlier numbered request", "", "⏺ ok"], W1, &[]);
    rows.truncate(cr as usize + 1); // 입력 상자 아래 괘선·상태줄 없음
    rows.push("  paste again to expand".to_string());
    assert!(
        cys::readiness::modal_foreground(&rows.join("\n"), Some("❯")).is_some(),
        "형상 대조 실패: 이 화면은 공유 전경 술어에서 양성이어야 한다(아래 판정이 무의미해진다)"
    );
    paint_rows(&t, &(rows, cr, cc));
    crate::governance::resubmit_withheld_submits(&fx.daemon);
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "블록 위 이력 서명 때문에 재제출이 멈췄다(탐침과 판정 불일치): {o:?}");
    assert_eq!(named(&fx, "queue.submit_resubmitted").len(), 1);
}

// ═══════════ ★(0.14.42 · 수정 4회차 R3C-1) 보류 기록의 창 대기 상한은 '창이 열려 있는 시간'만 잰다 ═══════════
//
// 【무엇이 틀렸었나】 수정 3회차(d59a1f5e)의 수명 상한 10분이 문서('창이 10분 안에 닫히지 않으면')와 달리 보류 시각부터
// **모든** 대기에 걸렸다 — 창이 몇 초 만에 승인돼 닫혀도 에이전트가 그 작업을 10분 넘게 이어 가면(`wait_prompt`) 기록을
// `wait_cap` 으로 버렸고, kill-switch pause 동안(틱이 곧바로 반환해 경과만 쌓임)에도 같았다. 상한 검사가 상태 검사보다
// 앞이었다. 결과: 작업이 끝난 뒤 본문이 입력 상자에 미제출로 남고(CLI 는 이미 OK) 그 좌석 큐가 입력줄 점유로 섰다
// (실 claude LT2 2/2 · 상한만 끈 진단판은 재제출·도달).
// 【규칙】 상한은 재제출 틱이 창(질문·선택 창 ∨ 승인 대기)을 **연달아 열린 것으로 본** 시간에만 걸린다. 창이 닫힌 것을 본
// 틱에서 그 시계는 0 으로 돌아가고(다음 창은 새 창), pause 동안은 멈춘다. 창이 닫힌 뒤 프롬프트 경계 복귀를 기다리는
// 동안의 외곽 상한은 큐 TTL(큐 항목과 같은 pause 크레딧)이다. 그 긴 대기 중에도 사람 손·새 기계 본문·빈 줄이면 종전대로
// 버리고, 재제출은 기록 1건당 한 번이다.

/// 시간 경과 모사 — 재제출 틱의 가상 시계를 `d` 만큼 앞으로 민다(= 직전 틱 뒤로 `d` 가 흘렀다).
fn elapse(fx: &Fx, d: Duration) {
    fx.vclock.set(fx.vclock.get() + d);
}

/// 가상 시계의 재제출 틱 1회(watchdog 이 부르는 것과 같은 본체).
fn tick_now(fx: &Fx) {
    crate::governance::resubmit_withheld_submits_at(&fx.daemon, Instant::now() + fx.vclock.get());
}

/// 창이 닫혔고 에이전트가 그 작업을 하는 중(스피너) — 입력 상자에는 우리 본문이 그대로 있다.
fn paint_working_own_composer(t: &Arc<Surface>) {
    paint(t, &["✻ Cogitating… (esc to interrupt)", "────────", "❯ M|x|AAA", "────────"], 2, 11);
}

fn tick(fx: &Fx) {
    tick_now(fx);
    std::thread::sleep(Duration::from_millis(150));
}

fn dropped(fx: &Fx) -> Vec<Value> {
    named(fx, "queue.submit_withheld_dropped")
}

/// 창이 2초 만에 승인돼 닫히고, 에이전트가 그 작업을 11분 한 뒤 유휴 — 기록은 남아 있다가 한 번 재제출된다.
fn long_turn_after_quick_approval(fx: &Fx, pid: u32) -> Arc<Surface> {
    let t = withheld_seat(fx, pid);
    tick(fx); // 창이 떠 있다 — 기다린다
    elapse(fx, Duration::from_secs(2));
    paint_working_own_composer(&t); // 2초 뒤 승인 — 창이 닫히고 작업 시작
    tick(fx);
    assert!(named(fx, "queue.submit_resubmitted").is_empty(), "전제: 작업 중에는 다시 쓰지 않는다");
    assert!(t.inject_track.withheld().is_some(), "전제: 작업 중 기록 유지");
    elapse(fx, Duration::from_secs(11 * 60)); // 작업 11분(그동안 5초 틱마다 wait_prompt)
    tick(fx);
    assert!(
        t.inject_track.withheld().is_some(),
        "창은 2초 만에 닫혔는데 작업 대기 11분에 기록을 버렸다(창 대기 상한이 작업 대기에 산입): {:?}",
        dropped(fx)
    );
    assert!(dropped(fx).is_empty(), "{:?}", dropped(fx));
    assert_eq!(settle_obs(&t).since_written_ms, None, "작업 중에 보류 CR 을 썼다");
    t
}

/// 적색→녹색(R3C-1 결과 1): 창 2초 승인 뒤 에이전트 11분 작업 → 유휴가 되면 한 번 재제출(두 번째는 없다).
#[test]
fn r3c_long_agent_turn_after_quick_approval_resubmits_once() {
    let fx = fx("r3c-longturn");
    let t = long_turn_after_quick_approval(&fx, P + 400);
    paint_own_composer(&t); // 작업이 끝나 유휴 — 입력 상자는 우리 본문 그대로
    tick_now(&fx);
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "장기 작업 뒤 유휴인데 보류 CR 을 다시 쓰지 않았다(조용한 미도달): {o:?}");
    assert_eq!(named(&fx, "queue.submit_resubmitted").len(), 1);
    // 1회성 — 그 CR 을 TUI 가 삼켜 입력줄이 그대로여도 다시 쓰지 않는다.
    paint_own_composer(&t);
    tick(&fx);
    elapse(&fx, Duration::from_secs(60));
    tick(&fx);
    assert_eq!(named(&fx, "queue.submit_resubmitted").len(), 1, "보류 CR 을 두 번 썼다(폭주)");
    assert!(dropped(&fx).is_empty(), "{:?}", dropped(&fx));
}

/// 적색→녹색(R3C-1 결과 3 · FV1-1 'pause 동안 동결 · resume 뒤 재개'): pause 11분 동안 창이 닫혔고 입력줄은 우리 본문
/// 그대로 — pause 중에는 쓰지 않고, resume 뒤 한 번 재제출한다(pause 시간은 창 대기 상한에 들지 않는다).
#[test]
fn r3c_long_pause_then_resume_resubmits_once() {
    let fx = fx("r3c-longpause");
    let t = withheld_seat(&fx, P + 410);
    tick(&fx); // 창이 떠 있다
    fx.daemon.paused.store(true, Ordering::SeqCst);
    elapse(&fx, Duration::from_secs(11 * 60));
    paint_own_composer(&t); // pause 중에 창이 닫혔다
    tick(&fx); // pause 중 틱(5초마다) — 아무것도 쓰지 않는다
    assert_eq!(settle_obs(&t).since_written_ms, None, "pause 중에 보류 CR 을 썼다 — kill-switch 약화");
    assert!(named(&fx, "queue.submit_resubmitted").is_empty());
    assert!(t.inject_track.withheld().is_some(), "pause 중 기록 유지");
    fx.daemon.paused.store(false, Ordering::SeqCst);
    tick_now(&fx);
    let o = wait_submit_consumed(&t);
    assert!(
        o.since_written_ms.is_some(),
        "resume 뒤 유휴 입력줄(우리 본문)인데 재제출하지 않았다 — pause 시간이 상한에 산입: {:?}",
        dropped(&fx)
    );
    assert_eq!(named(&fx, "queue.submit_resubmitted").len(), 1);
    assert!(dropped(&fx).is_empty(), "{:?}", dropped(&fx));
}

/// 적색→녹색 + 상한 보존: 창이 6분 열린 채 pause 30분(창 그대로) → resume 뒤에도 기록 유지(pause 는 창 시계를 멈춘다) →
/// 창이 5분 더 열려 누적 11분이면 `wait_cap` 으로 버린다(쓰기 0). 상한 자체(창이 10분 안에 닫히지 않으면)는 그대로다.
#[test]
fn r3c_pause_freezes_dialog_clock_and_cap_still_fires_on_open_window() {
    let fx = fx("r3c-freeze");
    let t = withheld_seat(&fx, P + 420);
    tick(&fx);
    elapse(&fx, Duration::from_secs(6 * 60));
    tick(&fx); // 창이 6분째 열려 있다
    assert!(t.inject_track.withheld().is_some(), "6분 — 상한 전");
    fx.daemon.paused.store(true, Ordering::SeqCst);
    elapse(&fx, Duration::from_secs(30 * 60));
    tick(&fx); // pause 중 틱
    fx.daemon.paused.store(false, Ordering::SeqCst);
    tick(&fx); // resume — 창은 여전히 열려 있다(창 시계 약 6분)
    assert!(
        t.inject_track.withheld().is_some(),
        "창이 열린 시간은 약 6분인데 pause 30분을 더해 기록을 버렸다: {:?}",
        dropped(&fx)
    );
    assert!(dropped(&fx).is_empty(), "{:?}", dropped(&fx));
    elapse(&fx, Duration::from_secs(5 * 60));
    tick(&fx); // 창 누적 약 11분
    assert!(t.inject_track.withheld().is_none(), "창이 11분 열려 있었는데 기록이 남았다(상한 소실)");
    let d = dropped(&fx);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["payload"]["reason"], json!("wait_cap"), "{d:?}");
    assert_eq!(settle_obs(&t).since_written_ms, None, "상한은 쓰지 않는 방향으로만 끝낸다");
    assert!(named(&fx, "queue.submit_resubmitted").is_empty());
}

/// 적색→녹색: 상한은 **한 창**이 10분 안에 닫히지 않을 때다 — 8분 창이 닫히고(작업) 다음 창이 8분 열려 있어도 버리지
/// 않는다. 두 번째 창이 닫히고 유휴가 되면 한 번 재제출한다.
#[test]
fn r3c_dialog_clock_restarts_when_window_closes() {
    let fx = fx("r3c-rewin");
    let t = withheld_seat(&fx, P + 430);
    tick(&fx);
    elapse(&fx, Duration::from_secs(8 * 60));
    tick(&fx); // 첫 창 8분
    paint_working_own_composer(&t); // 첫 창이 닫혔다
    tick(&fx);
    paint(&t, &DIALOG, 1, 8); // 두 번째 창
    tick(&fx);
    elapse(&fx, Duration::from_secs(8 * 60));
    tick(&fx); // 두 번째 창 8분(보류 뒤 16분)
    assert!(
        t.inject_track.withheld().is_some(),
        "어느 창도 10분을 넘지 않았는데 기록을 버렸다: {:?}",
        dropped(&fx)
    );
    paint_own_composer(&t);
    tick_now(&fx);
    let o = wait_submit_consumed(&t);
    assert!(o.since_written_ms.is_some(), "창이 닫히고 유휴인데 재제출하지 않았다: {o:?} {:?}", dropped(&fx));
    assert_eq!(named(&fx, "queue.submit_resubmitted").len(), 1);
}

/// 음성 대조(종전대로 버림): 긴 작업 대기 중에도 사람 손 · 새 기계 본문 · 빈 줄이면 쓰지 않고 그 사유로 버린다.
#[test]
fn r3c_long_wait_still_yields_to_human_newer_body_and_empty_line() {
    for (i, case) in ["human", "newer_body", "line_empty"].into_iter().enumerate() {
        let fx = fx("r3c-yield");
        let t = long_turn_after_quick_approval(&fx, P + 440 + 2 * i as u32);
        match case {
            "human" => {
                *t.last_human_input.lock().unwrap() = Some(Instant::now());
                paint_own_composer(&t);
            }
            "newer_body" => {
                t.inject_track.note_body("M|y|BBB", None);
                paint_own_composer(&t);
            }
            _ => paint(&t, &["────────", "❯ ", "────────"], 1, 2),
        }
        tick(&fx);
        assert_eq!(settle_obs(&t).since_written_ms, None, "{case}: 보류 CR 을 썼다");
        assert!(named(&fx, "queue.submit_resubmitted").is_empty(), "{case}");
        let d = dropped(&fx);
        assert_eq!(d.len(), 1, "{case}: {d:?}");
        assert_eq!(d[0]["payload"]["reason"], json!(case), "{case}: {d:?}");
        assert!(t.inject_track.withheld().is_none(), "{case}: 기록이 남았다");
    }
}

/// 외곽 상한(큐 TTL)은 창이 닫힌 뒤 대기에도 그대로다 — pause 가 아닌 대기가 TTL 을 넘으면 `expired` 로 버린다(쓰기 0).
/// 큐 항목과 같은 pause 크레딧: 긴 pause(TTL 초과) 뒤 resume 한 유휴 입력줄은 재제출한다.
#[test]
fn r3c_queue_ttl_bounds_post_close_wait_with_pause_credit() {
    let ttl = crate::state::queue_ttl_default_secs();
    assert!(ttl > 0, "전제: 큐 TTL 켜짐");
    // (가) 작업이 TTL 을 넘게 이어졌다 — 버린다.
    let fx1 = fx("r3c-ttl");
    let t = long_turn_after_quick_approval(&fx1, P + 460);
    elapse(&fx1, Duration::from_secs(ttl));
    tick(&fx1);
    let d = dropped(&fx1);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["payload"]["reason"], json!("expired"), "{d:?}");
    assert_eq!(settle_obs(&t).since_written_ms, None);
    drop(fx1);
    // (나) TTL 을 넘는 pause — pause 크레딧으로 버리지 않고 resume 뒤 재제출.
    let fx2 = fx("r3c-ttl-pause");
    let t = withheld_seat(&fx2, P + 470);
    paint_own_composer(&t);
    fx2.daemon.paused.store(true, Ordering::SeqCst);
    tick(&fx2);
    elapse(&fx2, Duration::from_secs(ttl + 3600));
    tick(&fx2);
    fx2.daemon.paused.store(false, Ordering::SeqCst);
    tick_now(&fx2);
    let o = wait_submit_consumed(&t);
    assert!(
        o.since_written_ms.is_some(),
        "TTL 을 넘는 pause 뒤 유휴 입력줄에서 재제출하지 않았다(큐 항목은 pause 크레딧으로 산다): {:?}",
        dropped(&fx2)
    );
    assert_eq!(named(&fx2, "queue.submit_resubmitted").len(), 1);
}
