//! ★0.14.42 P5 — 대화 승인 **1회용 팀 생성 토큰**의 데몬 다리(검증·소비 = 데몬 · 설계 §6-5).
//!
//! 설계 정본: `팀만들기-확인창-무반응-수정설계안-최종-20260923.md` §6-5 · §8-2 · §11 R6·R7·R8 · §13 P5.
//!
//! ## 판정은 팩 모듈 하나가 소유한다 — 데몬은 부를 뿐이다(사본 금지)
//! 원장 형식·전이 규칙·만료·결박·찢긴 꼬리 봉인·락(`<원장>.lock` fcntl.flock/msvcrt)·오너 문구의 정의처는
//! `cysjavis-pack/bin/javis_teamtoken.py` 다. 이 원장은 훅(발급 · 파이썬)과 데몬(소비)이 **함께 쓰는**
//! 파일이라, Rust 사본을 하나 두는 순간 두 필자가 서로 다른 규칙으로 읽고 쓴다(배달 원장
//! `delivery.rs` ↔ `javis_mission.py` 는 한 방향 쓰기인데도 교차 테스트로 겨우 붙들고 있다). 본문해시
//! (정규형 sha256)도 P3 가 "Rust 로 재구현 금지 — `--body-b64` 를 넘겨라" 로 못박았다.
//! 선례: 데몬이 팩 파이썬을 자식으로 부르는 경로가 이미 셋이다(`boot_supervisor` 의 `javis_bootstrap.py`
//! · `main.rs` phoenix · hud bridge). 인터프리터 해소(`bundled_python3`)·바이트코드 봉인·autostart 봉인을
//! 같게 쓴다. `dispatch` 는 `spawn_blocking` 풀에서 돌므로 이 동기 자식 대기는 런타임을 막지 않는다.
//!
//! ## 데몬이 보태는 것 — 파이썬이 알 수 없는 두 사실
//!   ① 호출 좌석 = 커널 peer pid 의 조상 체인(`handlers::resolve_caller_surface`). env·인자 자기신고는
//!      믿지 않는다 — 좌석을 신고하는 인자(`surface`·`surface_id`)는 거절한다(`hook.decide` 규약).
//!   ② 제안의 **현재** 본문·대기 여부 = 데몬 메모리(`feed_items`). feed.jsonl 파일은 믿지 않는다.
//!
//! ## 인가 규칙(실패 방향 = 인가 없음)
//! 자식 종료코드 0 **이고** JSON `ok:true` **이고** 기대한 성공 코드일 때만 통과([`judge`]). 비0·출력
//! 판독 불가·시간 초과·스폰 실패·스크립트 부재·파이썬 부재는 전부 거부다(P3 계약 "0 이 아닌 모든 값은
//! 인가 없음"). 거부는 P3 사유 코드를 **그대로** 싣는다 — `owner_gui_required` 로 뭉뚱그리지 않는다(R7).
//!
//! ## 2단 권한(P3 설계 공백 해소 — 그대로 따른다)
//!   생성: `inspect`(제안 id) → 데몬이 대기·현재 본문 확인 → `consume --phase create`(원자 1회)
//!         → `cys-dept` 가 생성 → `settle created|failed`.
//!   allow: `verify --phase allow`(비소비) → 데몬 해소 → **해소 성공 뒤에만** `consume --phase allow`.
//!
//! ★정직한 한계(설계 §12-1 · §14-1): 사고 방지 층이다. 같은 UID 로 원장을 직접 쓰는 프로세스는 막지
//!   못한다 — 원장은 위조의 감사 흔적이지 사전 차단이 아니다.

use crate::handlers::Reply;
use crate::state::Daemon;
use base64::Engine as _;
use cys::{err_response, ok_response, SpawnPolicy as _};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 자식 한 번의 수명 상한 — 원장 락 대기 상한(`LOCK_TIMEOUT_S` = 5초)의 세 배. 넘기면 죽이고 거부한다.
/// (CLI 의 RPC 무진행 상한 40초 안에 allow 경로의 두 번 호출이 들어간다.)
const CHILD_TIMEOUT: Duration = Duration::from_secs(15);
/// ★(0.14.42 fatal-fix R4-N3) 오너 문구 조회(`messages` — 원장 락을 잡지 않는다)의 수명 상한. 거부 1건의 최악 경로가
/// inspect 15 + consume 15 + 문구 15 = 45초로 CLI 무진행 상한(40초)을 넘어, CLI 가 daemon_unreachable 로 포기한 뒤
/// 데몬이 뒤늦게 토큰을 소비할 수 있었다(생성 없이 소진). 문구 조회는 짧게 묶는다(15 + 15 + 5 = 35초).
const MESSAGES_TIMEOUT: Duration = Duration::from_secs(5);
/// ★(0.14.42 fatal-fix R3-F3 · R4-N3) 자식 출력 한 벌의 보관 상한 — 넘는 바이트는 **읽어서 버린다**(파이프가 차서
/// 자식이 쓰기에서 막히지 않게). 실모듈의 가장 큰 출력(`messages`)은 약 7.5KB 다.
const OUTPUT_CAP: usize = 256 * 1024;
/// 자식 종료 뒤 판독 스레드를 기다리는 상한 — 손자가 파이프를 물고 있어도 요청 스레드를 무기한 붙들지 않는다.
const DRAIN_JOIN: Duration = Duration::from_secs(2);

/// 사유 코드 글자 규칙 — 자식이 내놓은 코드를 응답 코드로 실어도 되는 모양(`[a-z_]{1,48}`).
fn code_shape_ok(c: &str) -> bool {
    !c.is_empty() && c.len() <= 48 && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

/// 거부 1건 — 응답 `error.code`·`error.message` 로 그대로 나간다.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub code: String,
    pub message: String,
}

/// 파이썬 1회 호출의 판정.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub authorized: bool,
    pub code: String,
    pub message: String,
    pub detail: String,
    pub json: Value,
}

impl Verdict {
    fn refusal(&self) -> Refusal {
        Refusal { code: self.code.clone(), message: compose(&self.message, &self.detail) }
    }
}

/// 오너 문구 + 기술 사유 1줄(무안내 거부 금지 — 둘 중 있는 것은 전부 싣는다).
fn compose(owner: &str, detail: &str) -> String {
    match (owner.trim(), detail.trim()) {
        ("", "") => "팀 생성 토큰 판정 실패(사유 미상)".to_string(),
        (o, "") => o.to_string(),
        ("", d) => d.to_string(),
        (o, d) => format!("{o} ({d})"),
    }
}

/// ★순수 판정 — (종료코드, stdout, 기대 성공 코드) → 인가 여부. 테스트가 파이썬 없이 잰다.
///
/// 통과 = `exit == Some(0)` ∧ `ok:true` ∧ `code == expected`. 거부 JSON(비0 ∧ `ok:false` ∧ 코드 모양
/// 정상)은 그 사유 코드를 그대로 싣고, 그 밖의 어긋남(0 인데 `ok:false` · 비0 인데 `ok:true` · 판독 불가
/// · 기대와 다른 성공 코드)은 `internal_error` 로 접는다 — 어긋난 결과는 믿지 않는다.
pub fn judge(exit: Option<i32>, stdout: &str, expected: &str) -> Verdict {
    let line = stdout.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    let parsed: Value = serde_json::from_str(line).unwrap_or(Value::Null);
    let s = |k: &str| parsed.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let code = s("code");
    let ok_flag = parsed.get("ok").and_then(Value::as_bool);
    if exit == Some(0) && ok_flag == Some(true) && code == expected {
        return Verdict { authorized: true, code, message: s("message"), detail: s("detail"), json: parsed };
    }
    if exit.is_some_and(|c| c != 0) && ok_flag == Some(false) && code_shape_ok(&code) {
        return Verdict { authorized: false, code, message: s("message"), detail: s("detail"), json: parsed };
    }
    let head: String = line.chars().take(200).collect();
    Verdict {
        authorized: false,
        code: "internal_error".into(),
        message: String::new(),
        detail: format!("토큰 판정 결과를 믿을 수 없다(exit={exit:?} · 기대={expected} · 출력={head:?})"),
        json: parsed,
    }
}

fn script_path() -> PathBuf {
    #[cfg(test)]
    {
        tests::script_override().unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("cysjavis-pack")
                .join("bin")
                .join("javis_teamtoken.py")
        })
    }
    #[cfg(not(test))]
    {
        cys::pack::pack_dir().join("bin").join("javis_teamtoken.py")
    }
}

fn python() -> String {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    crate::bundled_python3(&exe_dir).unwrap_or_else(|| "python3".to_string())
}

fn internal(detail: String) -> Verdict {
    Verdict { authorized: false, code: "internal_error".into(), message: String::new(), detail, json: Value::Null }
}

/// `javis_teamtoken.py <args>` 1회 — 훅과 **같은 원장**을 보게 소켓·상태 루트를 명시한다.
///
/// env 규약(P3 "호출자는 훅과 같은 CYS_SOCKET/CYS_STATE_DIR 로 돌아야 한다"):
///   `CYS_SOCKET` = 이 데몬의 소켓(pane 에 싣는 값과 같은 문자열 — 레인 키가 같아진다) ·
///   `CYS_STATE_DIR` = `delivery::pack_state_dir()`(배달 원장과 같은 루트 = 훅이 읽는 루트 · 테스트
///   빌드에서는 실 HOME 대신 격리 루트) · 좌석 env 는 지운다(좌석은 `--surface` 인자로만 — 데몬이
///   커널 신원으로 정한 값).
fn run(socket: &Path, args: &[String], expected: &str) -> Verdict {
    run_with(socket, args, expected, CHILD_TIMEOUT)
}

/// ★(0.14.42 fatal-fix R3-F3 · R4-N3) 파이프 판독 스레드 — 자식이 **도는 동안** 출력을 비운다(상한 `OUTPUT_CAP` 까지 보관 ·
/// 나머지는 읽어서 버림). 종전 `run` 은 자식이 끝난 **뒤에야** 읽어, 파이프 용량(Windows 익명 파이프는 수 KB · 맥·리눅스 64KB)을
/// 넘게 쓰는 자식이 쓰기에서 막혀 15초 뒤 kill 되고 internal_error 로 접혔다(거부마다 +15초 · 오너 문구 소실 · 사유는
/// '원장 락 경합?'으로 오보). 스레드 생성 실패는 빈 출력으로 접는다(판정 = 인가 없음 · fail-closed).
fn drain<R: std::io::Read + Send + 'static>(mut r: R) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new().name("cysd-teamtoken-drain".into()).spawn(move || {
        let mut kept: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let room = OUTPUT_CAP.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n.min(room)]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = tx.send(kept);
    });
    rx
}

fn drained(rx: Option<std::sync::mpsc::Receiver<Vec<u8>>>) -> String {
    let bytes = rx.and_then(|r| r.recv_timeout(DRAIN_JOIN).ok()).unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn run_with(socket: &Path, args: &[String], expected: &str, timeout: Duration) -> Verdict {
    let script = script_path();
    if !script.is_file() {
        return internal(format!("토큰 모듈 부재: {} — 팩이 0.14.42 미만일 수 있다", script.display()));
    }
    // SEAL-1: 바이트코드 봉인 팩토리(`python_command`) — 번들 python 이 `.pyc` 를 쓰면 코드서명이 깨진다.
    let mut cmd = cys::python_command(python());
    cmd.arg(&script)
        .args(args)
        .env(cys::ENV_SOCKET, socket.to_string_lossy().as_ref())
        .env("CYS_STATE_DIR", crate::delivery::pack_state_dir())
        .env_remove(cys::ENV_SURFACE_ID)
        .env_remove("AITERM_SURFACE_ID")
        .env_remove("CYS_SURFACE_REF")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // 유계 자식(부모가 wait 로 붙든다 — 분리 금지 · Windows 콘솔 창만 숨김) + 라이벌 데몬 autostart 봉인.
    cmd.spawn_policy(cys::ChildLifetime::Attached).no_autostart();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return internal(format!("토큰 모듈 실행 실패({e})")),
    };
    // 도는 동안 비운다(위 `drain`) — 끝난 뒤 읽으면 파이프가 찬 자식이 영영 끝나지 않는다.
    let out_rx = child.stdout.take().map(drain);
    let err_rx = child.stderr.take().map(drain);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let detail = format!("토큰 모듈이 {}초 안에 끝나지 않았다(원장 락 경합 또는 인터프리터 지연)", timeout.as_secs());
                // ★(fatal-fix R4-N3) 시간 초과 갈래도 데몬 로그에 1줄 — 종전엔 무기록이었다.
                let tail: String = drained(err_rx).chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
                eprintln!("cysd: team.token {expected} 판정 불가 — {detail} / stderr: {tail}");
                return internal(detail);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return internal(format!("토큰 모듈 대기 실패({e})"));
            }
        }
    };
    let out = drained(out_rx);
    let v = judge(status.code(), &out, expected);
    if !v.authorized && v.code == "internal_error" {
        let err = drained(err_rx);
        let tail: String = err.chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
        eprintln!("cysd: team.token {expected} 판정 불가 — {} / stderr: {tail}", v.detail);
    }
    v
}

fn arg(k: &str, v: &str) -> String {
    // `--opt=값` 한 덩어리로 넘긴다 — 값이 `-` 로 시작해도 argparse 가 옵션으로 오인하지 않는다.
    format!("--{k}={v}")
}

/// P3 오너 문구(단일 출처)에서 사유 코드의 1줄을 꺼낸다 — 데몬이 스스로 판정한 거부(좌석 미상·제안
/// 부재·토큰 모양)에도 같은 문구를 싣기 위해서다. 조회 실패는 빈 문자열(기술 사유는 따로 실린다).
fn owner_text(socket: &Path, code: &str) -> String {
    let v = run_with(socket, &["messages".to_string()], "messages", MESSAGES_TIMEOUT);
    if !v.authorized {
        return String::new();
    }
    v.json["messages"][code].as_str().unwrap_or("").to_string()
}

fn refuse(socket: &Path, code: &str, detail: &str) -> Refusal {
    Refusal { code: code.to_string(), message: compose(&owner_text(socket, code), detail) }
}

/// `team_token` 인자 — `None` = 토큰 경로를 시도하지 않은 호출(키 부재·null). 문자열이 아닌 값은
/// 빈 토큰으로 접는다(→ `token_missing` · 결측은 값이 아니다).
pub fn token_param(params: &Value) -> Option<String> {
    match params.get("team_token") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => Some(String::new()),
    }
}

/// 토큰 모양 선검사 — 모양 밖이면 원장을 열 필요도 없이 미발급이다(argv 에 옵션 모양이 실리지 않게).
fn check_token(socket: &Path, token: &str) -> Result<(), Refusal> {
    if token.is_empty() {
        return Err(refuse(socket, "token_missing", "토큰이 주어지지 않았다"));
    }
    if !cys::team_spec::team_token_shape_ok(token) {
        return Err(refuse(socket, "token_unknown", "토큰 형식(32자 소문자 hex)이 아니다 — 발급된 적 없는 값"));
    }
    Ok(())
}

fn seat(socket: &Path, caller_sid: Option<u64>) -> Result<u64, Refusal> {
    caller_sid.ok_or_else(|| {
        refuse(
            socket,
            "surface_unknown",
            "이 호출은 어느 좌석(pane)에도 귀속되지 않는다 — 토큰은 발급된 좌석의 pane 안에서만 쓴다(커널 peer 신원)",
        )
    })
}

fn b64(s: &str) -> String {
    base64::engine::general_purpose::URL_SAFE.encode(s.as_bytes())
}

/// 이 제안 id 의 대기 중 팀 제안 본문(데몬 메모리의 **현재** 값).
fn pending_team_body(daemon: &Daemon, proposal_id: &str) -> Option<String> {
    let items = daemon.feed_items.lock().unwrap();
    items
        .iter()
        .find(|i| i.request_id == proposal_id)
        .filter(|i| i.kind == cys::team_spec::KIND && i.status == "pending")
        .map(|i| i.body.clone())
}

fn inspect_token(socket: &Path, token: &str) -> Result<Verdict, Refusal> {
    let v = run(socket, &["inspect".into(), arg("token", token)], "inspected");
    if v.authorized {
        Ok(v)
    } else {
        Err(v.refusal())
    }
}

/// `team.token.consume` — **생성 단계** 원자 1회 소비. 통과 = 이 좌석이 이 제안(현재 본문)으로 부서를
/// 1회 만들어도 된다는 데몬의 답. 응답 `spec_b64` = `cys-dept` 가 만들 명세(데몬 메모리의 현재 본문).
fn consume_create(daemon: &Daemon, params: &Value, caller_sid: Option<u64>) -> Result<Value, Refusal> {
    let sock = daemon.socket_path.as_path();
    let token = token_param(params).unwrap_or_default();
    check_token(sock, &token)?;
    let sid = seat(sock, caller_sid)?;
    let ins = inspect_token(sock, &token)?;
    let pid = ins.json["proposal_id"].as_str().unwrap_or("").to_string();
    // 판정 순서는 P3(`_judge_token`)와 같다 — **토큰 상태가 먼저**다. 이미 쓴 토큰의 재실행에 '제안 부재'로
    // 답하면(카드가 정리된 뒤) 사유가 바뀌고 cys-dept 의 멱등 보고가 끊긴다(샌드박스 E2E 실측 2026-09-23).
    let state = ins.json["state"].as_str().unwrap_or("");
    if state != "issued" {
        return Err(refuse(sock, "token_consumed", &format!("이미 소비된 토큰(상태 {state}) — 생성은 토큰 1개당 1회")));
    }
    // 제안의 대기·본문은 파일이 아니라 데몬 메모리가 정본이다.
    let body = pending_team_body(daemon, &pid).ok_or_else(|| {
        refuse(sock, "proposal_not_pending", &format!("토큰의 제안({pid})이 승인 피드에 대기 중이 아니다 — 이미 처리됐거나 거둬졌다"))
    })?;
    let spec = cys::team_spec::parse_body(&body).map_err(|e| refuse(sock, "proposal_body_invalid", &e))?;
    let body_b64 = b64(&body);
    let c = run(
        sock,
        &[
            "consume".into(),
            arg("token", &token),
            arg("proposal", &pid),
            arg("surface", &sid.to_string()),
            arg("body-b64", &body_b64),
            arg("phase", "create"),
        ],
        "consumed",
    );
    if !c.authorized {
        return Err(c.refusal());
    }
    Ok(json!({"code": "consumed", "proposal_id": pid, "surface": sid, "spec_b64": body_b64,
              "display": spec.display}))
}

/// `team.token.settle` — 생성 결과 기록(created 만 allow 권한을 무장한다 · failed 는 종결).
fn settle(daemon: &Daemon, params: &Value, caller_sid: Option<u64>) -> Result<Value, Refusal> {
    let sock = daemon.socket_path.as_path();
    let token = token_param(params).unwrap_or_default();
    check_token(sock, &token)?;
    let sid = seat(sock, caller_sid)?;
    let ins = inspect_token(sock, &token)?;
    let pid = ins.json["proposal_id"].as_str().unwrap_or("").to_string();
    let outcome = params.get("outcome").and_then(Value::as_str).unwrap_or("").to_string();
    let mut args = vec![
        "settle".to_string(),
        arg("token", &token),
        arg("proposal", &pid),
        arg("surface", &sid.to_string()),
        arg("outcome", &outcome),
    ];
    if let Some(dept) = params.get("dept").and_then(Value::as_str) {
        args.push(arg("dept", dept));
    }
    if let Some(code) = params.get("code").and_then(Value::as_i64) {
        args.push(arg("code", &code.to_string()));
    }
    let v = run(sock, &args, "settled");
    if !v.authorized {
        return Err(v.refusal());
    }
    Ok(json!({"code": "settled", "outcome": outcome, "proposal_id": pid,
              "grant_expires_at": v.json.get("grant_expires_at").cloned().unwrap_or(Value::Null)}))
}

/// `team.token.inspect` — 조회(인가 아님). `same_seat` = 호출 좌석(커널 신원)이 토큰 좌석과 같은가 —
/// `cys-dept` 의 멱등 재실행(이미 만든 팀 보고)이 이 답을 쓴다.
fn inspect(daemon: &Daemon, params: &Value, caller_sid: Option<u64>) -> Result<Value, Refusal> {
    let sock = daemon.socket_path.as_path();
    let token = token_param(params).unwrap_or_default();
    check_token(sock, &token)?;
    let v = inspect_token(sock, &token)?;
    let surface = v.json["surface"].as_str().unwrap_or("").to_string();
    let same_seat = caller_sid.is_some_and(|s| s.to_string() == surface);
    Ok(json!({"code": "inspected", "state": v.json["state"], "proposal_id": v.json["proposal_id"],
              "surface": surface, "same_seat": same_seat, "expires_at": v.json["expires_at"],
              "grant_expires_at": v.json["grant_expires_at"]}))
}

/// `team.token.*` 진입(dispatch 가 `channel.*` 처럼 위임한다). 좌석은 호출측이 커널 신원으로 도출해 넘긴다.
pub fn handle(daemon: &Arc<Daemon>, sub: &str, params: &Value, id: &Value, caller_sid: Option<u64>) -> Reply {
    if params.get("surface").is_some() || params.get("surface_id").is_some() {
        return Reply::Single(err_response(
            id,
            "invalid_params",
            "team.token: 좌석은 신고할 수 없다 — 데몬이 호출 프로세스(커널 peer pid)로 도출한다",
        ));
    }
    let r = match sub {
        "consume" => consume_create(daemon, params, caller_sid),
        "settle" => settle(daemon, params, caller_sid),
        "inspect" => inspect(daemon, params, caller_sid),
        other => {
            return Reply::Single(err_response(id, "method_not_found", &format!("unknown method: team.token.{other}")))
        }
    };
    match r {
        Ok(v) => Reply::Single(ok_response(id, v)),
        Err(f) => Reply::Single(err_response(id, &f.code, &f.message)),
    }
}

/// feed.reply `allow` 의 토큰 검증(비소비) — 무장된 allow 권한(settle created)·좌석·제안·본문 결박.
pub fn verify_allow(
    daemon: &Daemon,
    request_id: &str,
    body: &str,
    caller_sid: Option<u64>,
    token: &str,
) -> Result<(), Refusal> {
    let sock = daemon.socket_path.as_path();
    check_token(sock, token)?;
    let sid = seat(sock, caller_sid)?;
    let v = run(sock, &allow_args("verify", token, request_id, sid, body), "verified");
    if v.authorized {
        Ok(())
    } else {
        Err(v.refusal())
    }
}

/// 해소 **성공 뒤에만** — allow 권한을 닫는다(1회). 실패해도 해소는 이미 끝났다(두 번째 해소는 데몬이
/// `item already resolved` 로 막는다 — 멱등).
pub fn consume_allow(daemon: &Daemon, request_id: &str, body: &str, caller_sid: Option<u64>, token: &str) -> Verdict {
    let Some(sid) = caller_sid else {
        return internal("좌석 미상 — allow 권한을 닫지 못했다".into());
    };
    run(daemon.socket_path.as_path(), &allow_args("consume", token, request_id, sid, body), "consumed")
}

fn allow_args(verb: &str, token: &str, request_id: &str, sid: u64, body: &str) -> Vec<String> {
    vec![
        verb.to_string(),
        arg("token", token),
        arg("proposal", request_id),
        arg("surface", &sid.to_string()),
        arg("body-b64", &b64(body)),
        arg("phase", "allow"),
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static SCRIPT_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    pub(crate) fn script_override() -> Option<PathBuf> {
        SCRIPT_OVERRIDE.with(|c| c.borrow().clone())
    }

    const OKJ: &str = r#"{"ok": true, "code": "consumed", "message": "", "detail": "create 단계 1회 소비", "exit": 0}"#;

    #[test]
    fn judge_only_rc0_ok_true_and_expected_code_authorizes() {
        assert!(judge(Some(0), OKJ, "consumed").authorized);
        // 기대와 다른 성공 코드(verify 결과를 consume 으로 오인) — 인가 아님.
        assert!(!judge(Some(0), OKJ, "verified").authorized);
        // rc 가 비0 이면 ok:true 여도 인가 아님(P3: 0 이 아닌 모든 값은 인가 없음).
        for rc in [Some(1), Some(2), Some(3), Some(4), None] {
            let v = judge(rc, OKJ, "consumed");
            assert!(!v.authorized, "rc={rc:?}");
            assert_eq!(v.code, "internal_error", "어긋난 결과는 internal_error 로 접는다(rc={rc:?})");
        }
        // rc 0 인데 ok:false — 어긋남.
        let v = judge(Some(0), r#"{"ok": false, "code": "token_unknown"}"#, "consumed");
        assert!(!v.authorized);
        assert_eq!(v.code, "internal_error");
    }

    #[test]
    fn judge_carries_refusal_code_and_owner_message_verbatim() {
        let out = "진단 줄\n{\"ok\": false, \"code\": \"token_expired\", \"message\": \"확인이 오래 걸려 승인이 만료됐습니다\", \"detail\": \"TTL 초과\", \"exit\": 1}\n";
        let v = judge(Some(1), out, "consumed");
        assert!(!v.authorized);
        assert_eq!(v.code, "token_expired");
        let r = v.refusal();
        assert!(r.message.contains("만료됐습니다") && r.message.contains("TTL 초과"), "{}", r.message);
        // 기반 고장(exit 4) — 사유 코드 그대로(인가 아님).
        let v = judge(Some(4), r#"{"ok": false, "code": "ledger_corrupt", "message": "m", "detail": "d"}"#, "consumed");
        assert_eq!((v.authorized, v.code.as_str()), (false, "ledger_corrupt"));
    }

    #[test]
    fn judge_folds_garbage_and_odd_codes_to_internal_error() {
        for out in ["", "not json", "[]", "{\"ok\": false}", "{\"ok\": false, \"code\": \"Token-Unknown\"}",
                    "{\"ok\": false, \"code\": \"x y\"}"] {
            let v = judge(Some(1), out, "consumed");
            assert!(!v.authorized, "{out:?}");
            assert_eq!(v.code, "internal_error", "{out:?}");
        }
    }

    #[test]
    fn values_travel_as_single_equals_args() {
        // 값이 '-' 로 시작해도 argparse 옵션으로 오인되지 않게 `--k=v` 한 덩어리.
        assert_eq!(arg("dept", "-x"), "--dept=-x");
        let a = allow_args("verify", "t", "tp-1", 3, "{}");
        assert!(a.iter().skip(1).all(|s| s.starts_with("--") && s.contains('=')), "{a:?}");
    }

    /// ★(0.14.42 fatal-fix R3-F3 · R4-N3) 파이프 용량을 넘게 쓰는 자식도 막히지 않는다 — 종전 `run` 은 자식이 끝난 뒤에야
    /// 읽어서, stderr 300KB 를 쓰는 자식이 쓰기에서 막혀 15초 뒤 kill · internal_error 로 접혔다(수정 전 실측: 15.0s · 적색).
    #[test]
    fn chatty_child_does_not_deadlock_on_full_pipes() {
        crate::delivery::tests::isolate_state_dir_for_thread("tt-chatty");
        let dir = std::env::temp_dir().join(format!("cys-tt-chatty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("javis_teamtoken.py");
        std::fs::write(
            &script,
            "import sys\nsys.stderr.write('x' * 300000)\nsys.stderr.flush()\nsys.stdout.write('y' * 200000 + '\\n')\n\
             print('{\"ok\": true, \"code\": \"inspected\", \"message\": \"\", \"detail\": \"\", \"exit\": 0}')\n",
        )
        .unwrap();
        SCRIPT_OVERRIDE.with(|c| *c.borrow_mut() = Some(script.clone()));
        let t0 = Instant::now();
        let v = run(Path::new("/tmp/cys-tt-chatty.sock"), &["inspect".into()], "inspected");
        let took = t0.elapsed();
        SCRIPT_OVERRIDE.with(|c| *c.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(took < Duration::from_secs(10), "파이프가 찬 자식이 막혔다: {took:?} · {} {}", v.code, v.detail);
        assert!(v.authorized, "큰 출력 뒤 마지막 줄 JSON 판정 실패: {} {}", v.code, v.detail);
    }

    /// 스크립트 부재(구 팩) = 인가 없음(internal_error) — 토큰 경로가 조용히 열리지 않는다.
    #[test]
    fn missing_module_is_fail_closed() {
        crate::delivery::tests::isolate_state_dir_for_thread("tt-nomod");
        SCRIPT_OVERRIDE.with(|c| *c.borrow_mut() = Some(PathBuf::from("/nonexistent/javis_teamtoken.py")));
        let v = run(Path::new("/tmp/cys-tt-nomod.sock"), &["inspect".into(), arg("token", &"0".repeat(32))], "inspected");
        SCRIPT_OVERRIDE.with(|c| *c.borrow_mut() = None);
        assert!(!v.authorized);
        assert_eq!(v.code, "internal_error");
        assert!(v.detail.contains("부재"), "{}", v.detail);
    }
}
