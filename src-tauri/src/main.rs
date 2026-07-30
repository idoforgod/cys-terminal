//! cys UI shell — cysd 소켓의 얇은 클라이언트.
//! 코어/UI 분리: UI가 죽어도 세션(PTY)은 데몬에 살아있다. UI 재시작 = 재attach.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::UpdaterExt;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

type Stream = Box<dyn AsyncReadWrite>;
trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

// 멀티마스터 F3: attach 핸들 키를 (소켓 slug, surface_id) 복합키로 — 서로 다른 데몬이 같은
// surface_id를 독립 발급하므로 단독 키는 부서 간 PTY 스트림이 충돌한다.
struct Attachments(Mutex<HashMap<(String, u64), tauri::async_runtime::JoinHandle<()>>>);

/// 소켓 경로 → 짧은 결정론 식별자(이벤트명·attach 키용). 백엔드 단일 진실 — UI는 attach 반환값/
/// daemon-event 페이로드로 이 값을 전달받아 그대로 쓴다(독립 재계산 금지, 검증 mustFix).
fn sock_slug(socket: &std::path::Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    socket.to_string_lossy().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// 기본 소켓 — env(CYS_SOCKET) 누수 방지를 위해 명시적 기본 경로를 쓴다(멀티마스터 F3:
/// 앱이 CYS_SOCKET 걸린 셸에서 런칭돼도 단일 데몬 사용자 하위호환이 깨지지 않게).
fn default_socket() -> std::path::PathBuf {
    cys::socket_path()
}
/// UI workspace의 socket(Option) → 실제 경로. None = 기본 데몬(하위호환의 단일 결정요인).
fn resolve_socket(opt: &Option<String>) -> std::path::PathBuf {
    opt.as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_socket)
}

#[cfg(unix)]
async fn connect_to(socket: &std::path::Path) -> Result<Stream, String> {
    tokio::net::UnixStream::connect(socket)
        .await
        .map(|s| Box::new(s) as Stream)
        .map_err(|e| format!("cannot connect to cysd at {}: {e}", socket.display()))
}

#[cfg(windows)]
async fn connect_to(socket: &std::path::Path) -> Result<Stream, String> {
    use tokio::net::windows::named_pipe::ClientOptions;
    // ERROR_PIPE_BUSY(os error 231, "모든 파이프 인스턴스가 사용 중") busy-retry — 231은 데몬
    // 생존·listening 인스턴스 순간 소진(정상 혼잡)이므로 짧게 재시도하면 열린다(tokio 문서
    // 표준 패턴). 재시도 없는 1회 open 은 앱 기동 fan-out(daemon_status + pane별 attach +
    // event forwarder 동시 연결)에서 상시 "startup failed … os error 231"이 됐다(2026-07-10
    // Windows 실사고 — 워크스페이스/pane 렌더 전체 불능). 그 외 오류(파이프 부재 = 데몬
    // 다운 등)는 즉시 반환한다. 정책 상수는 CLI(cys)와 공용 단일 진실인 lib(cys::PIPE_BUSY_*).
    let name = socket.to_string_lossy().into_owned();
    let deadline = std::time::Instant::now() + cys::PIPE_BUSY_RETRY_DEADLINE;
    loop {
        match ClientOptions::new().open(&name) {
            Ok(s) => return Ok(Box::new(s) as Stream),
            Err(e)
                if e.raw_os_error() == Some(cys::PIPE_BUSY_ERROR)
                    && std::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(cys::PIPE_BUSY_RETRY_INTERVAL).await;
            }
            Err(e) => return Err(format!("cannot connect to cysd pipe: {e}")),
        }
    }
}

/// 기본 소켓 연결 (하위호환 wrapper).
async fn connect() -> Result<Stream, String> {
    connect_to(&default_socket()).await
}

/// 소켓별 영속 RPC 연결 풀 — 데몬(부서)마다 독립 연결 + 독립 락(데몬 간 직렬화 병목 제거).
type ConnCell = std::sync::Arc<tokio::sync::Mutex<Option<tokio::io::BufReader<Stream>>>>;
static RPC_POOL: std::sync::OnceLock<Mutex<HashMap<std::path::PathBuf, ConnCell>>> =
    std::sync::OnceLock::new();

/// 풀에서 소켓의 연결 셀을 얻는다 — 외부 std Mutex는 Arc 클론만 짧게 잡고 즉시 푼다(await 경계 안 넘김).
fn conn_cell(socket: &std::path::Path) -> ConnCell {
    let pool = RPC_POOL.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = pool.lock().unwrap();
    g.entry(socket.to_path_buf())
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(None)))
        .clone()
}

/// rpc_once 실패 단계: 전송 전(BeforeSend)은 데몬이 요청을 못 봤으므로 재시도 안전,
/// 전송 후(AfterSend)는 처리됐을 수 있어 비멱등 명령(create·send)의 맹목 재시도 금지.
enum RpcErr {
    BeforeSend(String),
    AfterSend(String),
}

async fn rpc_once(
    socket: &std::path::Path,
    conn: &mut Option<tokio::io::BufReader<Stream>>,
    line: &[u8],
) -> Result<String, RpcErr> {
    if conn.is_none() {
        *conn = Some(BufReader::new(
            connect_to(socket).await.map_err(RpcErr::BeforeSend)?,
        ));
    }
    let c = conn.as_mut().unwrap();
    c.get_mut()
        .write_all(line)
        .await
        .map_err(|e| RpcErr::BeforeSend(e.to_string()))?;
    c.get_mut()
        .flush()
        .await
        .map_err(|e| RpcErr::AfterSend(e.to_string()))?;
    let mut resp_line = String::new();
    let n = c
        .read_line(&mut resp_line)
        .await
        .map_err(|e| RpcErr::AfterSend(e.to_string()))?;
    if n == 0 {
        return Err(RpcErr::AfterSend("connection closed".into()));
    }
    Ok(resp_line)
}

/// 기본 소켓 RPC (하위호환 wrapper).
async fn rpc(method: &str, params: Value) -> Result<Value, String> {
    rpc_on(&default_socket(), method, params).await
}

/// 소켓 지정 RPC — 풀의 소켓별 연결을 잠가 직렬화(다른 데몬 RPC를 막지 않음).
async fn rpc_on(socket: &std::path::Path, method: &str, params: Value) -> Result<Value, String> {
    let resp = rpc_full(socket, method, params).await?;
    if resp["ok"].as_bool() == Some(true) {
        Ok(resp["result"].clone())
    } else {
        Err(resp["error"]["message"]
            .as_str()
            .unwrap_or("unknown error")
            .to_string())
    }
}

/// rpc_on의 전송·파싱 본체 — 데몬 응답 **전체**(ok/result/error.code)를 반환한다.
/// ★GUI 오퍼레이터 승인(오너 2026-07-15): feed_reply가 error.code(self_approval_denied 등)로
/// 재시도·UI 분류를 해야 하는데 rpc_on은 message만 올려 코드가 유실됐다 — 기존 호출부의 문자열
/// 계약(message만)은 rpc_on 래퍼로 그대로 보존하고, 코드가 필요한 곳만 이 함수를 직접 쓴다.
async fn rpc_full(socket: &std::path::Path, method: &str, params: Value) -> Result<Value, String> {
    let req = json!({"id": 1, "method": method, "params": params});
    let mut line = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    let cell = conn_cell(socket);
    let mut conn = cell.lock().await;
    let resp_line = match rpc_once(socket, &mut conn, &line).await {
        Ok(r) => r,
        Err(RpcErr::BeforeSend(_)) => {
            // 풀링된 연결이 끊겨 전송 자체가 실패 — 데몬이 요청을 못 봤으니 재시도 안전
            *conn = None;
            match rpc_once(socket, &mut conn, &line).await {
                Ok(r) => r,
                Err(RpcErr::BeforeSend(e)) | Err(RpcErr::AfterSend(e)) => {
                    *conn = None;
                    return Err(e);
                }
            }
        }
        Err(RpcErr::AfterSend(e)) => {
            // 데몬이 이미 처리했을 수 있음 — 중복 surface 생성·키 이중 주입을 막기 위해
            // 재전송하지 않고 에러를 그대로 올린다
            *conn = None;
            return Err(e);
        }
    };
    serde_json::from_str(resp_line.trim()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn daemon_status(socket: Option<String>) -> Result<Value, String> {
    rpc_on(&resolve_socket(&socket), "system.identify", json!({"caller": "ui"})).await
}

/// GUI(cys-app) 자기 버전 — 데몬 버전(system.identify .version)과 비교해 rename-swap 후
/// lame-duck 스큐(구 데몬 + 새 앱)를 UI 배지로 알리는 용도(P2 · 비차단·강제 재시작 없음).
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn list_surfaces(socket: Option<String>) -> Result<Value, String> {
    rpc_on(&resolve_socket(&socket), "surface.list", json!({})).await
}

/// org.status 브리지 — 사이드바 라이브 신호(B3)·command palette(07) 공유 소스.
#[tauri::command]
async fn org_status(socket: Option<String>) -> Result<Value, String> {
    rpc_on(&resolve_socket(&socket), "org.status", json!({})).await
}

/// 풀 비경유 일회성 RPC — org_fleet fan-out 전용. timeout 취소가 발생해도 이 연결만 드롭(폐기)되어
/// 공유 풀(conn_cell)을 desync로 오염시키지 않는다(같은 부서로 가는 send_key/org_status 응답 귀속 보호).
/// 적대검증 R-1 교정: rpc_on을 timeout으로 감싸면 취소 시 풀 연결이 미수신 응답을 남겨 후속 RPC가
/// stale 응답을 잘못 읽는다 — 일회성 연결은 드롭이 곧 연결 종료라 공유 상태를 건드리지 않는다.
async fn rpc_oneshot(socket: &std::path::Path, method: &str, params: Value) -> Result<Value, String> {
    let req = json!({"id": 1, "method": method, "params": params});
    let mut line = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    let mut stream = connect_to(socket).await?;
    stream.write_all(&line).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    let n = reader.read_line(&mut resp).await.map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("connection closed".into());
    }
    let resp: Value = serde_json::from_str(resp.trim()).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() == Some(true) {
        Ok(resp["result"].clone())
    } else {
        Err(resp["error"]["message"]
            .as_str()
            .unwrap_or("unknown error")
            .to_string())
    }
}

/// Tasks Control Center — 모든 부서의 모든 노드를 한 콜로 집계한다("부서 다중소켓 보드").
/// depts.json을 읽어 본부(기본 소켓)+각 부서 소켓에 org.status를 순회 호출하고, 부서 라벨을
/// 호출자(여기)에서 주입한다(단일 데몬은 자기가 어느 부서인지 모름 — socket_slug 사상과 동일).
/// 데몬은 outbound 클라이언트가 없어 집계는 이 Tauri 층(기존 rpc_on)에서 한다. 도달 실패 부서는
/// 드롭하지 않고 error로 표기한다(오너이 "부서가 죽었다"를 봐야 함). 부서 수가 적어(4~6) 순차
/// 호출이며 부서별 2초 timeout으로 hung 부서가 전체 함대를 막지 않는다.
#[tauri::command]
async fn org_fleet() -> Result<Value, String> {
    use std::time::Duration;
    // (소켓, name, display_name) — 본부 먼저, 그다음 depts.json 등록순.
    let mut targets: Vec<(std::path::PathBuf, String, String)> =
        vec![(default_socket(), "_hq".to_string(), "본부 · CEO".to_string())];
    if let Ok(reg) = list_depts() {
        if let Some(depts) = reg.get("depts").and_then(|d| d.as_object()) {
            for (name, meta) in depts {
                let sock = meta
                    .get("socket")
                    .and_then(|s| s.as_str())
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| dept_socket_path(name));
                let disp = meta
                    .get("display_name")
                    .and_then(|s| s.as_str())
                    .unwrap_or(name)
                    .to_string();
                targets.push((sock, name.clone(), disp));
            }
        }
    }
    let mut departments: Vec<Value> = Vec::new();
    for (sock, name, display_name) in targets {
        let slug = sock_slug(&sock);
        let socket_str = sock.to_string_lossy().to_string();
        // R-1 교정: 공유 풀(rpc_on) 대신 일회성 연결(rpc_oneshot) — timeout 취소가 풀을 오염시키지 않게.
        let call =
            tokio::time::timeout(Duration::from_secs(2), rpc_oneshot(&sock, "org.status", json!({})))
                .await;
        let base = json!({"name": name, "display_name": display_name,
                          "socket": socket_str, "socket_slug": slug});
        let entry = match call {
            Ok(Ok(status)) => {
                let mut o = base;
                let m = o.as_object_mut().unwrap();
                m.insert(
                    "surfaces".into(),
                    status.get("surfaces").cloned().unwrap_or_else(|| json!([])),
                );
                m.insert(
                    "paused".into(),
                    status.get("paused").cloned().unwrap_or(json!(false)),
                );
                o
            }
            Ok(Err(e)) => {
                let mut o = base;
                let m = o.as_object_mut().unwrap();
                m.insert("error".into(), json!(e));
                m.insert("surfaces".into(), json!([]));
                o
            }
            Err(_) => {
                let mut o = base;
                let m = o.as_object_mut().unwrap();
                m.insert("error".into(), json!("timeout"));
                m.insert("surfaces".into(), json!([]));
                o
            }
        };
        departments.push(entry);
    }
    Ok(json!({ "departments": departments }))
}

/// Tasks Control Center 실시간성: depts.json의 모든 부서 소켓에 이벤트 forwarder를 보장한다
/// (멱등 — 이미 도는 forwarder는 no-op). 앱 시작 시엔 기본 소켓 forwarder만 떠 있어(setup),
/// 이미 가동 중인 부서 데몬의 task.changed/status.changed가 UI로 안 흐를 수 있다 — 작업 탭이
/// 열릴 때 1회 호출해 전 부서 실시간 push를 보장한다.
#[tauri::command]
fn ensure_dept_forwarders(app: AppHandle) {
    if let Ok(reg) = list_depts() {
        if let Some(depts) = reg.get("depts").and_then(|d| d.as_object()) {
            for (name, meta) in depts {
                let sock = meta
                    .get("socket")
                    .and_then(|s| s.as_str())
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| dept_socket_path(name));
                spawn_event_forwarder(app.clone(), sock);
            }
        }
    }
}

#[tauri::command]
async fn control_dashboard() -> Result<Value, String> {
    rpc("control.dashboard", json!({})).await
}

#[tauri::command]
async fn control_hw() -> Result<Value, String> {
    rpc("control.hw", json!({})).await
}

#[tauri::command]
async fn control_analytics(window: Option<String>) -> Result<Value, String> {
    rpc("control.analytics", json!({ "window": window })).await
}

#[tauri::command]
async fn control_skills(window: Option<String>) -> Result<Value, String> {
    rpc("control.skills", json!({ "window": window })).await
}

#[tauri::command]
async fn control_cost_baseline(window: Option<String>) -> Result<Value, String> {
    rpc("control.cost_baseline", json!({ "window": window })).await
}

#[tauri::command]
async fn control_alerts() -> Result<Value, String> {
    rpc("control.alerts", json!({})).await
}

#[tauri::command]
async fn control_weekly() -> Result<Value, String> {
    rpc("control.weekly", json!({})).await
}

#[tauri::command]
async fn control_sessions(window: Option<String>, redact: Option<bool>) -> Result<Value, String> {
    rpc("control.sessions", json!({ "window": window, "redact": redact })).await
}

#[tauri::command]
async fn control_session_detail(session_id: String) -> Result<Value, String> {
    rpc("control.session_detail", json!({ "session_id": session_id })).await
}

#[tauri::command]
async fn control_session_star(session_id: String, starred: bool, note: Option<String>) -> Result<Value, String> {
    rpc("control.session_star", json!({ "session_id": session_id, "starred": starred, "note": note })).await
}

#[tauri::command]
async fn learn_status() -> Result<Value, String> {
    rpc("learn.status", json!({})).await
}

#[tauri::command]
async fn create_surface(
    socket: Option<String>,
    cwd: Option<String>,
    title: Option<String>,
    rows: u16,
    cols: u16,
) -> Result<Value, String> {
    rpc_on(
        &resolve_socket(&socket),
        "surface.create",
        json!({"cwd": cwd, "title": title, "rows": rows, "cols": cols}),
    )
    .await
}

/// 한글 IME 계측(디버그 전용): UI가 localStorage.cysImeDebug==="1"일 때만 호출 —
/// 입력 이벤트 시퀀스를 /tmp/cys-ime.log에 append해 유실 경로를 결정론으로 확정한다
/// (WKWebView 콘솔 접근이 어려운 환경의 실측 채널 · 2026-06-13 한글 4자→2자 유실 조사).
#[tauri::command]
fn log_ime(line: String) {
    use std::io::Write;
    // RC-10: /tmp 하드코딩 → OS중립 temp_dir(Windows엔 /tmp 없어 디버그 로그 무음 유실이던 것 수정).
    let log_path = std::env::temp_dir().join("cys-ime.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(f, "{ts} {line}");
    }
}

/// IME 디버그 게이트(파일/환경변수): 릴리스 빌드엔 devtools가 없어 localStorage.cysImeDebug를
/// 최종 사용자가 켤 수 없다 → ~/.cys/ime-debug 파일 존재 또는 CYS_IME_DEBUG=1이면 계측 활성.
#[tauri::command]
fn ime_debug_enabled() -> bool {
    std::env::var("CYS_IME_DEBUG").map(|v| v == "1").unwrap_or(false)
        || cys::home_dir().join(".cys/ime-debug").exists()
}

#[tauri::command]
async fn send_input(
    socket: Option<String>,
    surface_id: u64,
    data: String,
    queued: Option<bool>,
    clear_first: Option<bool>,
) -> Result<(), String> {
    // human=true: T3-13 타이핑 가드의 신호 — UI 키 입력을 '사람'으로 표시해
    // 원격 주입이 사람의 미완성 입력을 오염시키지 못하게 한다.
    // queued=true(전출 복원 주입 등 후속 지시)는 사람 타이핑이 아니므로 human=false —
    // human=true로 큐잉하면 last_human_input 갱신이 타이핑 가드를 3초 오염시킨다.
    // clear_first=true는 데몬 T3-13 권위 전달(Ctrl-U 정리→paste→지연 CR 원자 제출) —
    // raw "\r" 동봉은 Claude CLI가 paste로 삼켜 미제출된다(전출 e2e 실측). queued와 결합 불가.
    // 전출 지시도 사람의 클릭에서 발화하므로 human 유지(타이핑 가드 결정론 통과).
    let q = queued.unwrap_or(false);
    let cf = clear_first.unwrap_or(false);
    rpc_on(
        &resolve_socket(&socket),
        "surface.send_text",
        json!({"surface_id": surface_id, "text": data, "quiet": true, "human": !q,
               "queued": q, "clear_first": cf}),
    )
    .await
    .map(|_| ())
}

/// 전출(F6-2) 핸드오프 폴백 경로용 홈 디렉토리 — cwd가 루트류(/·C:\)인 pane은
/// 프로젝트 상대 경로(_round/handoffs)가 성립하지 않아 ~/.cys/transfers 로 폴백한다.
#[tauri::command]
fn home_dir_path() -> String {
    cys::home_dir().to_string_lossy().into_owned()
}

/// 클립보드 이미지 붙여넣기(F): base64 이미지를 임시 파일로 저장하고 절대경로를 반환한다.
/// UI가 이 경로를 셸 인용해 PTY로 타이핑한다(iTerm2 동작 — 붙여넣기로 이미지 경로 주입).
#[tauri::command]
fn save_pasted_image(data_b64: String, ext: String) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64.as_bytes())
        .map_err(|e| e.to_string())?;
    // ext는 UI가 MIME에서 유도(png/jpg/gif/webp) — 경로 조작 방지로 영숫자만 통과, 아니면 png.
    let safe_ext = if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext.as_str()
    } else {
        "png"
    };
    let dir = std::env::temp_dir().join("cys-paste");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("paste-{ms}.{safe_ext}"));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// 파일 트리 패널용 디렉토리 나열 — dirs 먼저, 이름순.
#[tauri::command]
fn list_dir(path: String) -> Result<Value, String> {
    let mut entries: Vec<(String, bool)> = std::fs::read_dir(&path)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (e.file_name().to_string_lossy().into_owned(), is_dir)
        })
        .collect();
    entries.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    Ok(json!(entries
        .into_iter()
        .map(|(name, is_dir)| json!({"name": name, "is_dir": is_dir}))
        .collect::<Vec<_>>()))
}

/// 파일을 시스템 기본 앱으로 연다 (macOS open / Windows start).
/// 실행형 파일(유닉스 실행비트·Windows 실행 확장자)은 open이 곧 실행일 수 있어
/// force 없이는 "executable_confirm" 에러로 거절한다 — UI가 확인 후 force로 재호출(fail-closed).
#[tauri::command]
fn open_path(path: String, force: Option<bool>) -> Result<(), String> {
    // 실재하는 로컬 경로만 허용 — URL 스킴·존재하지 않는 문자열이 OS 런처에 닿지 않게
    let meta = std::fs::metadata(&path).map_err(|e| format!("not a local path: {e}"))?;
    if !force.unwrap_or(false) && meta.is_file() {
        // 근본한계 명문화: '열기=실행'이 되는 타입의 완전 열거는 불가능하다(OS·설치 앱에 따라
        // 확장). 게이트 = 실행비트(unix) + 위험 확장자 목록(문서-실행형 포함) — 목록 밖 신종
        // 타입은 통과할 수 있으므로 신뢰 없는 파일은 Finder/탐색기에서 확인이 원칙이다.
        fn ext_of(path: &str) -> String {
            std::path::Path::new(path)
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default()
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o111 != 0 {
                return Err("executable_confirm".into());
            }
        }
        // macOS 문서-실행형: 실행비트 없이도 open이 설치·명령 실행으로 이어지는 타입
        #[cfg(target_os = "macos")]
        if ["pkg", "mpkg", "command", "terminal", "tool"].contains(&ext_of(&path).as_str()) {
            return Err("executable_confirm".into());
        }
        // Windows: 실행비트가 없어 확장자 게이트 — 스크립트·핸들러 실행형 전반
        #[cfg(windows)]
        if [
            "exe", "bat", "cmd", "com", "scr", "ps1", "msi", "vbs", "vbe", "js", "jse",
            "wsf", "wsh", "hta", "lnk", "reg", "jar", "pif", "scf", "cpl", "msc",
        ]
        .contains(&ext_of(&path).as_str())
        {
            return Err("executable_confirm".into());
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        let _ = ext_of; // linux 등에서 미사용 경고 억제(실행비트 게이트만 적용)
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&path).spawn();
    // explorer는 인자를 셸 파싱하지 않는다 — cmd /C start의 메타문자 주입 경로 제거
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(&path).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(&path).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// 전출(F6-2) 핸드오프 내용 검증용 텍스트 헤드 읽기 — 파일 실존≠내용 유효이므로
/// UI가 5필드(HANDOFF_CONTRACT)를 확인한다. 실재 파일만, 기본 64KB 캡(대파일 프리즈 방지).
#[tauri::command]
fn read_text_head(path: String, max_bytes: Option<u64>) -> Result<String, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("not a local path: {e}"))?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    let cap = max_bytes.unwrap_or(65536).min(1_048_576) as usize;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let head = &bytes[..bytes.len().min(cap)];
    Ok(String::from_utf8_lossy(head).into_owned())
}

/// 파일 관리자에서 해당 항목을 선택해 보여준다 (macOS Finder reveal / Windows explorer select).
/// open_path와 동일한 실재 경로 게이트 — URL 스킴·비존재 문자열 차단.
#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    std::fs::metadata(&path).map_err(|e| format!("not a local path: {e}"))?;
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("-R").arg(&path).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = {
        // xdg에는 reveal 표준이 없다 — 부모 폴더 열기로 폴백
        let parent = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        std::process::Command::new("xdg-open").arg(parent).spawn()
    };
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// HUD-2: 외부 URL HARD 화이트리스트 — https만·도메인 allowlist. 통과 시 Ok(spawn 없음·테스트 가능).
/// url crate 부재 → 수동 host 파싱(https:// strip → 첫 '/' 전 host, userinfo(@)·port(:) 제거 = 위장 host 차단).
/// 기본 목록은 코드 봉인, 사용자 도메인은 로컬 설정으로 확장(공개 배포에서 기관 도메인 하드코딩 제거):
/// ~/.cys/url-allow-hosts(줄당 1도메인 — GUI 경로) 또는 $CYS_URL_ALLOW_HOSTS(콤마 구분).
fn url_host_allowed(url: &str) -> Result<(), String> {
    let rest = url.strip_prefix("https://").ok_or_else(|| "https only".to_string())?;
    // authority는 첫 '/', '?'(query), '#'(fragment) 전까지(RFC 3986) — query/fragment 사칭 우회 차단.
    let authority = rest.split(|c: char| c == '/' || c == '?' || c == '#').next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or(authority); // userinfo(@) 제거 — 위장 host 차단
    let host = host.split(':').next().unwrap_or(host); // port 제거
    let extras = user_allow_hosts();
    if host_in_allowlist(host, &extras) {
        Ok(())
    } else {
        Err(format!("domain not allowed: {host}"))
    }
}

/// 순수 판정(테스트 핀) — 기본 allowlist + 사용자 확장 도메인, 정확일치 또는 서브도메인.
fn host_in_allowlist(host: &str, extras: &[String]) -> bool {
    const ALLOW: &[&str] = &["notebooklm.google.com", "github.com", "cysinsight.com"];
    ALLOW
        .iter()
        .map(|d| *d)
        .chain(extras.iter().map(|s| s.as_str()))
        .any(|d| !d.is_empty() && (host == d || host.ends_with(&format!(".{d}"))))
}

/// 사용자 확장 allowlist — 파일(~/.cys/url-allow-hosts, 줄당 1개) ∪ env(콤마 구분).
/// 로컬 사용자 자신의 동의 하에 자기 머신에서만 확장된다(원격 주입 경로 없음).
fn user_allow_hosts() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Ok(s) = std::fs::read_to_string(cys::home_dir().join(".cys/url-allow-hosts")) {
        out.extend(s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')));
    }
    if let Ok(env) = std::env::var("CYS_URL_ALLOW_HOSTS") {
        out.extend(env.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    }
    out
}

/// HUD-2: SOT 근거 URL을 시스템 브라우저로 연다 — 화이트리스트 통과 https만(비가역 외부개방의 최후 게이트).
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    url_host_allowed(&url)?;
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(&url).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// D5: cys 사이드카 바이너리 해소 — exe 옆(production 번들) 우선, 없으면 PATH 폴백(ensure_daemon 패턴).
fn resolve_sidecar(name: &str) -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from(name))
}

// ── CLI PATH 설치(명시 메뉴) — 가드/스크립트 순수 헬퍼 ─────────────────
#[derive(PartialEq, Debug)]
enum BundleKind {
    Canonical,    // /Applications/cys.app 또는 ~/Applications/cys.app
    Translocated, // Gatekeeper AppTranslocation 휘발 경로
    Backup,       // cys.app.bak-*/*.prev*
    NonStandard,  // 그 외(Downloads 등) — 경고와 함께 진행
}

/// 셸 단일따옴표 이스케이프(경로의 공백·특수문자·따옴표 안전).
fn sh_squote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// AppleScript 문자열 리터럴 이스케이프(큰따옴표). `osascript`의 `do shell script`는
/// 작은따옴표가 아니라 **큰따옴표 리터럴**을 요구한다 — 백슬래시·큰따옴표만 이스케이프하면 되고,
/// 내부 셸 경로 인용은 sh_squote(작은따옴표)가 따로 담당한다.
fn applescript_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `<bundle>/Contents/MacOS` 디렉토리를 분류한다.
fn classify_bundle_dir(macos_dir: &std::path::Path) -> BundleKind {
    let s = macos_dir.to_string_lossy();
    if s.contains("/AppTranslocation/") {
        return BundleKind::Translocated;
    }
    // macos_dir = <bundle>.app/Contents/MacOS → bundle = parent.parent
    let bundle = macos_dir.parent().and_then(|p| p.parent());
    if let Some(b) = bundle {
        let name = b
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.starts_with("cys.app.bak") || name.starts_with("cys.app.prev") {
            return BundleKind::Backup;
        }
        if name == "cys.app" {
            let parent = b
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            // ★/Volumes 가드: DMG·외장 마운트 안의 Applications 폴더/심링크(예: /Volumes/<dmg>/Applications/
            // cys.app)는 ends_with("/Applications")를 만족해도 Canonical 이 아니다 — 언마운트·이젝트 시
            // 죽은 경로가 되어 자기삭제·"손상됨" 결함이 재발한다(DMG 안 Applications 심링크 경유 실행 오판
            // 차단). 정규 /Applications·~/Applications 만 Canonical(둘 다 /Volumes 하위가 아니므로 불변).
            if !parent.starts_with("/Volumes/")
                && (parent == "/Applications" || parent.ends_with("/Applications"))
            {
                return BundleKind::Canonical;
            }
        }
    }
    BundleKind::NonStandard
}

/// launchd 자기등록 가드(순수): **Canonical(/Applications·~/Applications)만 허용**한다. 무음
/// autostart(GUI 시작 시 plist 무음 기록)는 명시 사용자설치(plan_cli_install: 사용자 액션+가시 경고)와
/// 위험 프로파일이 다르다 — NonStandard(~/Downloads·/Volumes/USB 등 휘발/이동 경로)가 plist
/// ProgramArguments 에 각인되면 언마운트·삭제 시 죽은 경로 데몬을 무한 스폰한다(Translocated·Backup 도
/// 동류). 그래서 plan_cli_install 이 NonStandard 를 경고만 하고 허용하는 것과 **의도적으로 divergence**해,
/// 자동등록은 Canonical 로 한정한다(비-Canonical 은 ensure_daemon 런타임 폴백=휘발성 데몬으로 안전).
fn autoregister_allowed(kind: &BundleKind) -> bool {
    matches!(kind, BundleKind::Canonical)
}

/// T2 부트 안전모드 판정 결과. autoregister 만 가리던 `autoregister_allowed` 보다 상위의 **부트 전면
/// 게이트**로, 데몬 기동·launchd 등록·팩/hook 쓰기 등 자기경로 부수효과 전체를 조건화한다.
#[derive(PartialEq, Debug, Clone, Copy)]
enum BootPathVerdict {
    Canonical,    // 정규 설치(/Applications·~/Applications) — 기존 부트 그대로 진행
    Translocated, // Gatekeeper AppTranslocation 휘발 경로 — 안전모드
    NonCanonical, // /Volumes(DMG 직실행)·Downloads·백업·개발 target/ 등 비정규 — 안전모드
}

/// 부트 경로 판정(순수): 실행 파일 경로와 escape env 플래그만으로 안전모드 진입 여부를 결정한다.
///
/// - `env_escape`(CYS_ALLOW_NONCANONICAL=1)이면 **무조건 Canonical** — 개발 빌드·CI·e2e 는 target/
///   등 비정규 경로에서 실행되므로 이 탈출구가 없으면 테스트 하네스 자신이 안전모드에 갇힌다.
/// - 그 외에는 `classify_bundle_dir` 4분류를 3분류로 접는다: Canonical→Canonical,
///   Translocated→Translocated, Backup·NonStandard(=/Volumes·Downloads·개발 target/ 포함)→NonCanonical.
///   판정 로직을 `classify_bundle_dir` 에 위임해 autoregister 가드와 divergence 하지 않게 한다
///   (동일 경로 → 동일 안전성 판단·단일 SOT).
///
/// exe_path 는 `.../Contents/MacOS/cys-app`(current_exe) — 그 parent 가 classify_bundle_dir 입력이다.
/// parent 가 없는 비정상 입력은 보수적으로 NonCanonical(정규 설치 근거 없음).
fn boot_path_verdict(exe_path: &std::path::Path, env_escape: bool) -> BootPathVerdict {
    if env_escape {
        return BootPathVerdict::Canonical;
    }
    let macos_dir = match exe_path.parent() {
        Some(d) => d,
        None => return BootPathVerdict::NonCanonical,
    };
    match classify_bundle_dir(macos_dir) {
        BundleKind::Canonical => BootPathVerdict::Canonical,
        BundleKind::Translocated => BootPathVerdict::Translocated,
        BundleKind::Backup | BundleKind::NonStandard => BootPathVerdict::NonCanonical,
    }
}

/// 실행 중 프로세스의 부트 판정(비순수 래퍼) — current_exe + CYS_ALLOW_NONCANONICAL env 를 읽어
/// `boot_path_verdict` 에 넘긴다. current_exe() 실패는 **fail-open(Canonical)**: 판정 근거가 전무할
/// 때 정규 설치를 안전모드로 오무력화하지 않는다("오탐=앱 무력화" 회피). 이 fail-open 은
/// maybe_autoregister_launchd 의 autoregister_allowed 가드(launchd 경로 독립 재검)로 방어심층이 유지된다.
#[cfg(target_os = "macos")]
fn current_boot_verdict() -> BootPathVerdict {
    let env_escape = std::env::var("CYS_ALLOW_NONCANONICAL")
        .map(|v| v == "1")
        .unwrap_or(false);
    match std::env::current_exe() {
        Ok(p) => boot_path_verdict(&p, env_escape),
        Err(_) => BootPathVerdict::Canonical,
    }
}

/// 프론트 **pull 경로**(emit-before-listen 레이스 회피 · reviewer1 major). setup 의 안전모드
/// `translocation-blocked` emit 은 프론트 listen 등록 전에 발화할 수 있고 Tauri v2 는 미등록 리스너에
/// 버퍼링하지 않아 안내가 유실될 수 있다. start() 초기에 이 커맨드를 조회해 Some(안내문구)=안전모드면
/// 즉시 표시한다(emit 은 벨트앤서스펜더로 유지). 데몬 무관 순수 조회라 daemon-ready 이전에도 응답한다.
/// Canonical·비-macOS 는 None(정상 부트).
#[tauri::command]
fn boot_verdict() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let v = current_boot_verdict();
        if v != BootPathVerdict::Canonical {
            return Some(translocation_guidance(v));
        }
    }
    None
}

/// 안전모드 사용자 안내 문구(순수). 자동 이동(자기 복사)은 오탐 시 파괴 위험이라 이번 범위에서
/// 구현하지 않고, 복구 절차만 안내한다 — 설계 폴백 경로가 항상 성립. GUI 에서는
/// `translocation-blocked` 이벤트로 stickyToast 에 실리고, 비-GUI(CI 등)에서는 stderr 로그로 나간다.
#[cfg(target_os = "macos")]
fn translocation_guidance(verdict: BootPathVerdict) -> String {
    let cause = match verdict {
        BootPathVerdict::Translocated => {
            "Safari 등에서 내려받은 DMG 안의 앱을 곧바로 열어 macOS가 cys.app을 임시 위치에서 실행 중입니다."
        }
        _ => "cys.app이 정규 설치 위치(Applications) 밖에서 실행 중입니다.",
    };
    format!(
        "{cause} 이 상태로는 백그라운드 서비스를 안전하게 등록할 수 없어 안전모드로 멈췄습니다.\n\n\
         다음 순서로 설치해 주세요:\n\
         1) Finder에서 cys.app을 응용 프로그램(Applications) 폴더로 드래그해 복사합니다.\n\
         2) 이미 설치된 구버전 cys.app이 실행 중이면 먼저 종료한 뒤 새 버전으로 교체합니다.\n\
         3) 그래도 '손상됨'으로 열리지 않으면 터미널에서 아래를 한 번 실행하세요:\n\
         \u{2003}xattr -d com.apple.quarantine /Applications/cys.app\n\n\
         설치 후 응용 프로그램 폴더의 cys.app을 다시 열면 정상 부팅됩니다."
    )
}

/// `do shell script` 본문: target_dir 생성 + cys·cysd 심볼릭 멱등 생성(`ln -sf`).
fn build_install_script(
    cys: &std::path::Path,
    cysd: &std::path::Path,
    target_dir: &str,
) -> String {
    format!(
        "mkdir -p {td} && ln -sf {c} {tc} && ln -sf {d} {tdd}",
        td = sh_squote(target_dir),
        c = sh_squote(&cys.to_string_lossy()),
        tc = sh_squote(&format!("{target_dir}/cys")),
        d = sh_squote(&cysd.to_string_lossy()),
        tdd = sh_squote(&format!("{target_dir}/cysd")),
    )
}

/// `which -a cys` 출력 → precedence 순 경로 리스트(공백줄 제거).
fn parse_which_a(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// 설치 계획(순수): 가드 판정 + 소스 경로 + osascript 인자 + 경고. osascript 실행은 포함하지 않는다.
struct CliInstallPlan {
    cys_src: std::path::PathBuf,
    cysd_src: std::path::PathBuf,
    osascript_arg: String, // `do shell script "..." with administrator privileges` (AppleScript 큰따옴표 리터럴)
    warnings: Vec<String>,
}

fn plan_cli_install(
    macos_dir: &std::path::Path,
    target_dir: &str,
) -> Result<CliInstallPlan, String> {
    match classify_bundle_dir(macos_dir) {
        BundleKind::Translocated => {
            return Err("cys.app이 Gatekeeper에 의해 임시 위치에서 실행 중입니다. \
Finder에서 cys.app을 Applications 폴더로 옮긴 뒤 다시 열고 시도하세요."
                .into());
        }
        BundleKind::Backup => {
            return Err("백업 번들에서 실행 중입니다. \
정규 cys.app(Applications)에서 실행한 뒤 시도하세요."
                .into());
        }
        BundleKind::Canonical | BundleKind::NonStandard => {}
    }
    let mut warnings = vec![];
    if classify_bundle_dir(macos_dir) == BundleKind::NonStandard {
        warnings.push(
            "cys.app이 표준 위치(Applications)가 아닌 곳에서 실행 중입니다. \
앱을 옮기면 심볼릭이 깨지니 Applications로 이동을 권장합니다."
                .into(),
        );
    }
    let cys_src = macos_dir.join("cys");
    let cysd_src = macos_dir.join("cysd");
    let script = build_install_script(&cys_src, &cysd_src, target_dir);
    // AppleScript `do shell script`는 큰따옴표 문자열 리터럴을 요구한다 — 작은따옴표로 감싸면
    // 실행 전 파스 단계에서 syntax error -2741로 거부된다(내부 셸 경로 인용은 build_install_script의
    // sh_squote가 담당). 따라서 바깥 래핑은 반드시 applescript_str(큰따옴표)여야 한다.
    let osascript_arg = format!(
        "do shell script {} with administrator privileges",
        applescript_str(&script)
    );
    Ok(CliInstallPlan {
        cys_src,
        cysd_src,
        osascript_arg,
        warnings,
    })
}

#[derive(serde::Serialize)]
struct InstallCliReport {
    ok: bool,
    target_dir: String,
    cys_link: String,
    cysd_link: String,
    source_cys: String,
    effective_cys: Option<String>, // which -a cys 1순위
    shadowed_by: Option<String>,   // /usr/local/bin/cys 앞을 가리는 다른 cys
    warnings: Vec<String>,
}

/// 명시 메뉴 트리거. macOS에서 cys·cysd를 /usr/local/bin에 1회 승격으로 심볼릭한다.
#[tauri::command]
fn install_cli_to_path() -> Result<InstallCliReport, String> {
    #[cfg(not(target_os = "macos"))]
    {
        return Err("이 기능은 macOS 전용입니다.".into());
    }
    #[cfg(target_os = "macos")]
    {
        let target_dir = "/usr/local/bin";
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let macos_dir = exe
            .parent()
            .ok_or("번들 디렉토리 해석 실패")?
            .to_path_buf();

        let plan = plan_cli_install(&macos_dir, target_dir)?;
        if !plan.cys_src.exists() || !plan.cysd_src.exists() {
            return Err("번들 내 cys/cysd 바이너리를 찾지 못했습니다.".into());
        }

        // osascript 1회 승격(cys·cysd 동시 → 단일 프롬프트).
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&plan.osascript_arg)
            .output()
            .map_err(|e| format!("osascript 실행 실패: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("-128") || err.contains("User canceled") {
                return Err("설치가 취소되었습니다.".into());
            }
            return Err(format!("심볼릭 생성 실패: {}", err.trim()));
        }

        // 검증: 로그인 PATH 기준 which -a cys.
        let which = std::process::Command::new("bash")
            .arg("-lc")
            .arg("which -a cys")
            .output()
            .ok();
        let entries = which
            .as_ref()
            .map(|o| parse_which_a(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default();
        let effective_cys = entries.first().cloned();
        let target_cys = format!("{target_dir}/cys");
        let shadowed_by = match &effective_cys {
            Some(p) if *p != target_cys => Some(p.clone()),
            _ => None,
        };

        let mut warnings = plan.warnings;
        if let Some(sh) = &shadowed_by {
            warnings.push(format!(
                "PATH 선행 위치의 다른 cys가 우선합니다: {sh} \
(예: dev deploy_gate의 /opt/homebrew/bin). 새로 설치한 {target_cys}는 그 뒤에 있습니다."
            ));
        }

        Ok(InstallCliReport {
            ok: true,
            target_dir: target_dir.to_string(),
            cys_link: target_cys,
            cysd_link: format!("{target_dir}/cysd"),
            source_cys: plan.cys_src.to_string_lossy().to_string(),
            effective_cys,
            shadowed_by,
            warnings,
        })
    }
}

/// 업데이트 재시작 후 자동복귀 마커 경로 — install_update(재시작 직전)가 쓰고, 재시작된 cys-app
/// setup이 읽는다. 두 프로세스가 공유하는 ~/.cys 아래에 둔다.
fn pending_restore_path() -> std::path::PathBuf {
    cys::home_dir().join(".cys/.pending-restore")
}

/// (T1) 마지막으로 팩반영·복원을 완료한 앱 버전 스탬프 경로. 홈페이지 수동 설치(.app 번들만 교체·
/// 복귀 마커 없음)를 '버전변경'으로 감지하는 진실원 — 인앱 업데이트(마커)와 수동 설치(스탬프) 두
/// 경로 모두에서 재시작 후 팩반영·복원이 돌게 한다. pending_restore_path와 같은 ~/.cys 아래에 둔다.
fn last_app_version_path() -> std::path::PathBuf {
    cys::home_dir().join(".cys/.last-app-version")
}

/// GUI 온보딩 완료 마커 — "이 GUI가 이 바이너리 버전에서 온보딩(팩+hook(+win: schtasks))을
/// **성공** 완료했는가". writer는 GUI 온보딩 성공 경로 단 하나다 — CLI autostart·잔존 schtasks·
/// ONLOGON 등 어떤 순서로 cysd가 먼저 돌아도 이 마커를 선점할 수 없다(0.12.52 cys-neo 회귀 시정:
/// 팩 마커(.pack-version) 기반 게이트를 CLI-선행 cysd 스윕이 선점 → ~/.claude hook 영구 미설치 →
/// "너는 마스터다" 부트스트랩 무력화). ★.pack-version(팩 최신 여부·install 계층 writer)·
/// .last-app-version(복원 필요 여부·L2 writer)과 질문·작성자가 전부 다르다 — 통합 금지:
/// .last-app-version은 --no-install-hook 경로(Apply)가 전진시키므로 "스탬프 있음=hook 있음"이 거짓.
fn gui_onboarded_path() -> std::path::PathBuf {
    cys::home_dir().join(".cys/.gui-onboarded")
}

/// GUI 온보딩 실행 여부 — 부작용 없는 순수 판정(단위테스트 대상). 마커 내용이 현재 바이너리
/// 버전과 정확히 일치할 때만 스킵. 부재·불일치·읽기 실패 = 실행(fail-open — 치유 방향).
fn needs_gui_onboard(marker: Option<&str>, current_version: &str) -> bool {
    marker.map(str::trim) != Some(current_version)
}

/// (T1) 재시작 후 팩반영·복원을 돌릴지 판정 — 부작용(파일·프로세스) 없는 순수 함수(단위테스트 대상).
#[derive(Debug, PartialEq, Eq)]
enum PendingUpdatePlan {
    /// 마커 없음 + 스탬프가 현재 버전과 일치 → 정상 정상상태, 아무 것도 안 함.
    Skip,
    /// 스탬프 부재 + 기존 설치 증거 없음(진짜 최초 설치) → 스탬프만 기록·팩반영·복원 스킵(복원할 topology 없음·온보딩이 팩 처리).
    RecordStampOnly,
    /// 마커 존재(인앱 업데이트) OR 스탬프≠현재버전(홈페이지 수동설치) → 팩반영 + 성공 시 조직 복원.
    Apply,
}

/// 발동 조건 = 마커 존재 OR 버전변경 감지. 마커가 최우선(구버전이 이 릴리스로 올라올 때 마커를 남김).
/// prior_state_exists = 기존 설치 증거(~/.cys/pack/.pack-version 존재). 스탬프 부재(≤0.12.50엔 스탬프
/// 파일 자체가 없다) 시 이 증거로 '전환기 기존 사용자의 홈페이지 수동설치'(Apply)와 '진짜 최초
/// 설치'(RecordStampOnly)를 가른다 — 오너가 홈페이지 설치본을 배포할 예정이라 이 경로가 실경로다.
fn decide_pending_update(
    marker_exists: bool,
    stamp: Option<&str>,
    current_version: &str,
    prior_state_exists: bool,
) -> PendingUpdatePlan {
    if marker_exists {
        return PendingUpdatePlan::Apply;
    }
    match stamp {
        // 스탬프 부재 + 기존 설치 증거 있음 = 전환기 기존 사용자(≤0.12.50)가 홈페이지로 0.12.51+ 설치 → 복원 필요.
        None if prior_state_exists => PendingUpdatePlan::Apply,
        None => PendingUpdatePlan::RecordStampOnly,
        Some(v) if v != current_version => PendingUpdatePlan::Apply,
        Some(_) => PendingUpdatePlan::Skip,
    }
}

/// 업데이트(인앱 재시작 OR 홈페이지 수동설치로 인한 버전변경)이면 두 가지를 한다:
///  ① 새 기능 배포 — 새 cys 바이너리에 embed된 팩(pack.rs include_str! + build.rs PACK_SKILLS)을
///     `cys init-pack --no-install-hook`으로 ~/.cys/pack에 반영한다. --no-install-hook: hook 등록은
///     최초 설치/launch-agent에서 끝나므로 매 업데이트마다 settings.json을 건드리지 않는다(.bak-cys
///     백업 파괴·활성 프로필 재직렬화 방지 — 적대검증 serious). force 없이 호출하므로 preserve-gate가
///     사용자 수정 파일을 보존하고 비수정·신규만 갱신한다.
///  ② 자동복귀 — 팩 반영 성공 시에만 조직 전체(본부+등록 부서) 노드를 복원(T2 spawn_org_restore).
///     init-pack 실패 시 마커·스탬프를 보존하고 복원을 보류해, 노드가 구 디렉티브로 조용히 각성하는
///     침묵 실패를 막는다(적대검증 fatal). restore는 멱등(run_restore).
fn maybe_apply_pending_update(app: &AppHandle) {
    let marker = pending_restore_path();
    let stamp_path = last_app_version_path();
    let current = env!("CARGO_PKG_VERSION");
    let marker_exists = marker.exists();
    let stamp = std::fs::read_to_string(&stamp_path)
        .ok()
        .map(|s| s.trim().to_string());
    // 기존 설치 증거 — 디스크 팩 버전 파일(check_pack_update:1711·install_pack_update:1895와 동일 SOT).
    let prior_state = cys::pack::pack_dir().join(".pack-version").exists();
    match decide_pending_update(marker_exists, stamp.as_deref(), current, prior_state) {
        PendingUpdatePlan::Skip => return,
        PendingUpdatePlan::RecordStampOnly => {
            // 최초 설치 — 복원할 topology가 없다. 스탬프만 기록해 다음 재시작을 정상상태로 만든다.
            let _ = std::fs::write(&stamp_path, current);
            return;
        }
        PendingUpdatePlan::Apply => {}
    }
    // ① 새 팩(새 기능) 반영 — 성공 여부를 검사한다(침묵 실패 차단).
    let mut init_cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" }));
    init_cmd.arg("init-pack").arg("--no-install-hook");
    no_console(&mut init_cmd);
    let pack_ok = init_cmd
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !pack_ok {
        // 실패 — 마커·스탬프를 보존(다음 재시작에 재시도)하고 노드 복원을 보류한다. 구 디렉티브로
        // 조용히 각성하는 것을 막고 사용자에게 알린다.
        let _ = app.emit(
            "update-error",
            "새 팩 반영(init-pack) 실패 — 노드 복원 보류, 다음 재시작에 재시도",
        );
        return;
    }
    // 성공 후에만 마커 제거 + 스탬프 전진 + 조직 복원. (마커 없는 버전변경 경로면 remove_file은 no-op.)
    let _ = std::fs::remove_file(&marker);
    let _ = std::fs::write(&stamp_path, current);
    spawn_org_restore(app.clone());
}

/// (T2) `cys restore --include-master`를 사이드카로 1회 실행한다. socket=Some이면 그 부서 소켓
/// 대상(CYS_SOCKET), None이면 기본(본부) 소켓. CYS_NO_AUTOSTART=1로 죽은 소켓에 빈 cysd가
/// autostart되는 것을 막는다(살아있는 대상에만 호출하므로 평시 무영향인 심층방어). 반환=성공 여부.
async fn run_sidecar_restore(socket: Option<std::path::PathBuf>) -> bool {
    tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" }));
        cmd.arg("restore").arg("--include-master");
        cmd.env("CYS_NO_AUTOSTART", "1"); // 죽은 소켓에 빈 데몬 autostart 금지(사이드카 CLI 가드)
        if let Some(sock) = socket {
            cmd.env(cys::ENV_SOCKET, sock);
        }
        no_console(&mut cmd);
        cmd.status().map(|s| s.success()).unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// ★TCC 처방(오너 2026-07-15 — EPERM 실사고 구조 수리): 서명이 바뀌는 업그레이드마다 macOS가
/// 폴더 접근 권한(TCC)을 리셋해 pane 자식(claude 등)이 작업 폴더 읽기에서 EPERM으로 죽는다.
/// ①GUI(UI 프로세스)가 기동 시 데스크톱/문서를 read_dir — 미결정 상태면 macOS 권한 팝업이 떠
///   선제 해결된다(UI 프로세스만 팝업 표시 가능 · CLI 자식은 팝업 없이 조용히 거부됨).
/// ②이미 거부된 상태(팝업 재유도 불가)면 perm-warning 이벤트 → 프론트 sticky 토스트로 설정
///   경로 안내. 매 기동 실행 — 저비용·멱등(허용 상태면 무음).
#[cfg(target_os = "macos")]
fn nudge_folder_permissions(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let home = cys::home_dir();
        for folder in ["Desktop", "Documents"] {
            let p = home.join(folder);
            let denied = tokio::task::spawn_blocking(move || {
                matches!(std::fs::read_dir(&p),
                         Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied)
            })
            .await
            .unwrap_or(false);
            if denied {
                let _ = app.emit("perm-warning", json!({"folder": folder}));
            }
        }
    });
}

/// (T2) 업데이트 후 조직 전체 복원 — setup 완료를 막지 않도록 백그라운드 태스크로 순차 실행하며
/// restore-progress를 emit한다(update-progress emit 스타일 동형). 본부=기본 소켓 사이드카 restore →
/// list_depts() 순회: 부서 데몬이 살아있으면 사이드카 restore(부서 소켓), 죽었으면 기존 launch 경로
/// (launch_dept_daemon)로 재기동한다 — 재기동된 부서 데몬은 콜드부트 auto-restore로 노드를 되살린다
/// (src/bin/cysd/main.rs). run_restore 멱등이라 콜드부트 복원과 겹쳐도 안전.
fn spawn_org_restore(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let _ = app.emit("restore-progress", json!({"phase": "start"}));
        // 본부(기본 소켓) — setup의 ensure_daemon으로 이미 가동 확정.
        let hq_ok = run_sidecar_restore(None).await;
        // ★WP-3 리바이버 게이트: base 데몬 dept 묘비 — 삭제-의도 부서는 재기동에서 제외(+생존 시 reap).
        // RPC 실패=빈 집합(보수적 fail-open: 묘비 부재=현행 거동 — 롤백 불변식 "부재=제약 없음").
        let tombs: std::collections::HashSet<String> =
            rpc_oneshot(&cys::socket_path(), "dept_tombstone.list", json!({}))
                .await
                .ok()
                .and_then(|v| {
                    v.get("dept_tombstones").and_then(|a| a.as_array()).map(|a| {
                        a.iter().filter_map(|x| x.as_str().map(String::from)).collect()
                    })
                })
                .unwrap_or_default();
        // 부서 순회 — 등록 부서(depts.json)만 대상(유령 부서 재-launch 차단).
        let mut ok = 0usize;
        let mut fail = 0usize;
        if let Ok(reg) = list_depts() {
            if let Some(depts) = reg.get("depts").and_then(|d| d.as_object()) {
                for (name, meta) in depts {
                    let sock = meta
                        .get("socket")
                        .and_then(|s| s.as_str())
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| dept_socket_path(name));
                    // 생존확인(org_fleet 동형·2초 timeout) — identify 응답 = 데몬 살아있음.
                    let alive = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        rpc_oneshot(&sock, "system.identify", json!({})),
                    )
                    .await
                    .map(|r| r.is_ok())
                    .unwrap_or(false);
                    // ★WP-3: 묘비 부서 — 재기동 금지. 생존이면 reap(정리 대기 프로세스 — 묘비가
                    // 부활을 이미 차단하므로 좀비 아님. teardown 실패=WARN·차회 부팅 재평가로 수렴).
                    if tombs.contains(name.as_str()) {
                        let mut detail = "삭제-의도 묘비 — 재기동 제외".to_string();
                        if alive {
                            let _ = stop_dept_daemon_by_socket(
                                sock.to_string_lossy().to_string(),
                            )
                            .await;
                            // ★R4(D-IMPL-4): teardown 함수는 실패를 삼키므로(무조건 Ok) 재프로브로
                            // 결과를 가시화 — 여전히 생존이면 WARN 라벨(차회 부팅 재시도가 수렴 경로).
                            let still = tokio::time::timeout(
                                std::time::Duration::from_secs(2),
                                rpc_oneshot(&sock, "system.identify", json!({})),
                            )
                            .await
                            .map(|r| r.is_ok())
                            .unwrap_or(false);
                            detail = if still {
                                "삭제-의도 묘비 — teardown 미확정(WARN·차회 시작 시 재시도)".into()
                            } else {
                                "삭제-의도 묘비 — 잔존 데몬 정리 완료".into()
                            };
                        }
                        let _ = app.emit(
                            "restore-progress",
                            json!({"phase": "skip", "dept": name, "detail": detail}),
                        );
                        continue;
                    }
                    let dept_ok = if alive {
                        run_sidecar_restore(Some(sock.clone())).await
                    } else {
                        // 죽은 부서 → 기존 launch 경로 재사용(콜드부트 auto-restore가 노드 부활).
                        launch_dept_daemon(app.clone(), name.clone()).await.is_ok()
                    };
                    if dept_ok {
                        ok += 1;
                    } else {
                        fail += 1;
                    }
                }
            }
        }
        if !hq_ok && ok == 0 && fail == 0 {
            // 본부 복원조차 못 돌고 부서도 없음 = 복원 경로 자체 실패 → 가시화(UI health 토스트).
            let _ = app.emit(
                "restore-progress",
                json!({"phase": "error", "detail": "본부 노드 복원 실행 실패"}),
            );
            return;
        }
        // hq_ok를 done에 실어 부서가 있을 때도 본부(HQ) 복원 실패가 묻히지 않게 한다(침묵 실패 차단 —
        // 이 작업의 목적). error 페이즈는 '본부 실패 + 부서 없음' 전면 실패만 담당(위).
        let _ = app.emit(
            "restore-progress",
            json!({"phase": "done", "hq_ok": hq_ok, "ok": ok, "fail": fail}),
        );
    });
}

/// D5/P1: UI 발 키 전송 — surface.send_key RPC 래퍼. send_input(send_text)과 달리 Return 등 키 전송 가능.
/// human 플래그 미사용(데몬 send_key 핸들러는 전부 프로그램 경로 — 읽지 않음).
#[tauri::command]
async fn send_key(socket: Option<String>, surface_id: u64, key: String) -> Result<(), String> {
    rpc_on(&resolve_socket(&socket), "surface.send_key",
        json!({"surface_id": surface_id, "key": key})).await.map(|_| ())
}

/// D5/SB-1: 스킬 버튼 보드 카탈로그 읽기(pack/board-catalog.json) — 정적 파일 read(데몬 무변경).
#[tauri::command]
fn read_board_catalog() -> Result<Value, String> {
    let path = cys::pack::pack_dir().join("board-catalog.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("board-catalog.json 없음 ({}): {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("카탈로그 파싱 실패: {e}"))
}

/// D6: 청중 프로파일(~/.cys/profile.json·사용자 로컬·pack 밖) audience 읽기 — 없으면 "custom"(전체보기 폴백·안전).
fn read_profile_audience() -> String {
    let path = cys::home_dir().join(".cys/profile.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("audience").and_then(|a| a.as_str()).map(String::from))
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "custom".to_string())
}

/// CC v2 WS-B: run_id 생성 — 산출물 dir·생애주기 추적의 결정론 키. ascii kebab만
/// (skillrun.rs run_started 검증과 정합 — 경로 성분 금지).
fn make_run_id(slug: Option<&str>, task: &str) -> String {
    let base: String = slug
        .unwrap_or(task)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.is_empty() { "skill".to_string() } else { base };
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{base}-{epoch}")
}

/// D5: 무계약 차단의 결정론 강제점 — task-prompt 티켓(성공기준·4규칙)을 생성한다(UI가 직접 워커에 명령 못 함).
/// --no-survival-gate(B2): fresh 경로는 surface를 실행 시점에 만들므로 지금 워커 생존 확인 불요.
/// D6: 청중 프로파일 audience를 scope에 주입 — 스킬이 Implications Domain 질문을 건너뛴다(custom=전체보기).
/// CC v2 WS-B: 반환이 {ticket, run_id}로 확장 — 산출물 위치를 run_id dir로 핀(실행↔산출물 결정론 연결).
#[tauri::command]
fn make_ticket(
    task: String,
    scope: String,
    success: String,
    to: String,
    slug: Option<String>,
) -> Result<Value, String> {
    let script = cys::pack::pack_dir().join("bin").join("javis_orchestra.py");
    let run_id = make_run_id(slug.as_deref(), &task);
    let out_fmt = format!(
        "산출물을 ~/.cys/_round/skill-out/{run_id}/ (절대경로) 아래에 저장하라(결정론 회수 위치·SB-6). \
         산출물에 '🔒 AI 보조 생성 · 오너 검수 전' 신뢰선 라벨을 부착하라(과대약속 금지)."
    );
    let audience = read_profile_audience();
    let scope_full = if audience != "custom" {
        format!("{scope} · 청중 프로파일: {audience}(이 청중 맞춤으로 산출·Implications Domain 질문 생략)")
    } else {
        scope.clone()
    };
    let mut orch_cmd = std::process::Command::new("python3");
    inject_runtime_path(&mut orch_cmd); // RC-5: 동봉 runtime(python3.exe) PATH 주입
    orch_cmd
        .arg(&script)
        .arg("task-prompt")
        .args(["--task", &task, "--scope", &scope_full, "--success", &success, "--to", &to])
        .arg("--no-survival-gate")
        .args(["--output-format", &out_fmt]);
    no_console(&mut orch_cmd);
    let output = orch_cmd
        .output()
        .map_err(|e| format!("javis_orchestra 실행 실패: {e}"))?;
    if !output.status.success() {
        return Err(format!("task-prompt 실패: {}", String::from_utf8_lossy(&output.stderr)));
    }
    Ok(json!({"ticket": String::from_utf8_lossy(&output.stdout).to_string(), "run_id": run_id}))
}

/// D5/SB-2: 보이는 일회용 워커로 스킬 실행 — cys skill run(schedule --fresh) spawn(새 RPC 0·invisible -p 금지).
/// CC v2 WS-B: run_id(make_ticket 발급)를 --run-id로 관통 — 데몬 run 생애주기 추적.
#[tauri::command]
fn run_skill(
    name: String,
    ticket: String,
    agent: Option<String>,
    close_after: Option<u64>,
    run_id: Option<String>,
) -> Result<Value, String> {
    if ticket.trim().is_empty() {
        return Err("ticket 비어 있음 — 무계약 실행 금지".into());
    }
    let cys = resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" });
    let mut cmd = std::process::Command::new(&cys);
    cmd.arg("skill").arg("run").arg(&name)
        .args(["--ticket", &ticket])
        .args(["--agent", agent.as_deref().unwrap_or("claude")]);
    if let Some(ca) = close_after {
        cmd.args(["--close-after", &ca.to_string()]);
    }
    if let Some(rid) = run_id.as_ref() {
        cmd.args(["--run-id", rid]);
    }
    cmd.stdin(std::process::Stdio::null());
    no_console(&mut cmd);
    cmd.spawn()
        .map_err(|e| format!("cys skill run 실행 실패 ({}): {e}", cys.display()))?;
    Ok(json!({"ok": true, "name": name, "run_id": run_id}))
}

/// CC v2 WS-B: 최근 스킬 run 목록(생애주기 카드) — 로컬 데몬 skill.runs 위임.
#[tauri::command]
async fn skill_runs(limit: Option<u64>) -> Result<Value, String> {
    rpc("skill.runs", json!({"limit": limit.unwrap_or(20)})).await
}

/// CC v2 WS-B(B5): 실행 전 자원 사전 게이트 — javis_resource_gate.py check --json.
/// exit 0=allow 1=soft(경고 후 진행 가능) 2=hard(차단). 스크립트 부재·실행 실패는 allow
/// (게이트가 보드를 죽이지 않는다 — fail-open, 게이트 자체는 사전 경고 장치).
#[tauri::command]
fn resource_gate_check() -> Result<Value, String> {
    let script = cys::pack::pack_dir().join("bin").join("javis_resource_gate.py");
    if !script.exists() {
        return Ok(json!({"exit_code": 0, "report": Value::Null}));
    }
    let mut cmd = std::process::Command::new("python3");
    inject_runtime_path(&mut cmd);
    cmd.arg(&script).arg("check").arg("--json");
    no_console(&mut cmd);
    match cmd.output() {
        Ok(out) => {
            let code = out.status.code().unwrap_or(0);
            let report =
                serde_json::from_slice::<Value>(&out.stdout).unwrap_or(Value::Null);
            Ok(json!({"exit_code": code, "report": report}))
        }
        Err(_) => Ok(json!({"exit_code": 0, "report": Value::Null})),
    }
}

/// CC v2 WS-A: 계정 rate limit 전 조직 병합 뷰 — org_fleet 동형 fan-out(본부+부서, 2s 타임아웃).
/// 병합 = (provider, account_id) 최신 updated_at 승자 · profiles 합집합. 부서 다운은 무시(로컬 우선).
#[tauri::command]
async fn usage_accounts_all() -> Result<Value, String> {
    use std::time::Duration;
    let mut targets: Vec<std::path::PathBuf> = vec![default_socket()];
    if let Ok(reg) = list_depts() {
        if let Some(depts) = reg.get("depts").and_then(|d| d.as_object()) {
            for (name, meta) in depts {
                let sock = meta
                    .get("socket")
                    .and_then(|s| s.as_str())
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| dept_socket_path(name));
                targets.push(sock);
            }
        }
    }
    let mut merged: std::collections::HashMap<(String, String), Value> =
        std::collections::HashMap::new();
    for sock in targets {
        let call = tokio::time::timeout(
            Duration::from_secs(2),
            rpc_oneshot(&sock, "usage.accounts", json!({})),
        )
        .await;
        let Ok(Ok(resp)) = call else { continue };
        for a in resp["accounts"].as_array().into_iter().flatten() {
            let key = (
                a["provider"].as_str().unwrap_or("").to_string(),
                a["account_id"].as_str().unwrap_or("").to_string(),
            );
            match merged.get_mut(&key) {
                None => {
                    merged.insert(key, a.clone());
                }
                Some(cur) => {
                    let cur_ts = cur["updated_at"].as_f64().unwrap_or(0.0);
                    let new_ts = a["updated_at"].as_f64().unwrap_or(0.0);
                    // profiles 합집합은 승자와 무관하게 유지
                    let mut profs: Vec<String> = cur["profiles"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .chain(a["profiles"].as_array().into_iter().flatten())
                        .filter_map(|p| p.as_str().map(String::from))
                        .collect();
                    profs.sort();
                    profs.dedup();
                    if new_ts > cur_ts {
                        *cur = a.clone();
                    }
                    cur["profiles"] = json!(profs);
                }
            }
        }
    }
    let mut accounts: Vec<Value> = merged.into_values().collect();
    accounts.sort_by(|x, y| {
        (x["provider"].as_str().unwrap_or(""), x["label"].as_str().unwrap_or(""))
            .cmp(&(y["provider"].as_str().unwrap_or(""), y["label"].as_str().unwrap_or("")))
    });
    Ok(json!({"accounts": accounts}))
}

/// D5/SB-6: 산출물 회수 결정론 위치(~/.cys/_round/skill-out) — make_ticket output_format과 정합.
#[tauri::command]
fn skill_out_dir() -> String {
    cys::home_dir()
        .join(".cys/_round/skill-out")
        .to_string_lossy()
        .to_string()
}

#[tauri::command]
async fn rename_surface(socket: Option<String>, surface_id: u64, title: String) -> Result<(), String> {
    rpc_on(
        &resolve_socket(&socket),
        "surface.rename",
        json!({"surface_id": surface_id, "title": title}),
    )
    .await
    .map(|_| ())
}

#[tauri::command]
async fn resize_surface(
    socket: Option<String>,
    surface_id: u64,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    rpc_on(
        &resolve_socket(&socket),
        "surface.resize",
        json!({"surface_id": surface_id, "rows": rows, "cols": cols}),
    )
    .await
    .map(|_| ())
}

#[tauri::command]
async fn close_surface(
    state: State<'_, Attachments>,
    socket: Option<String>,
    surface_id: u64,
) -> Result<(), String> {
    let sock = resolve_socket(&socket);
    let key = (sock_slug(&sock), surface_id);
    if let Some(handle) = state.0.lock().unwrap().remove(&key) {
        handle.abort();
    }
    rpc_on(&sock, "surface.close", json!({"surface_id": surface_id}))
        .await
        .map(|_| ())
}

#[tauri::command]
async fn feed_list(status: Option<String>) -> Result<Value, String> {
    rpc("feed.list", json!({"status": status})).await
}

/// ★GUI 오퍼레이터 승인(오너 2026-07-15): 기본 데몬 state 디렉토리의 operator.token을 읽는다 —
/// cysd state_dir(RC-13)의 기본 데몬 매핑과 동형(unix=기본 소켓의 부모 디렉토리,
/// windows=%LOCALAPPDATA%\cys). feed_reply는 기본 데몬 전용(rpc 기본 소켓)이라 부서 pipe 슬러그
/// 분기는 불필요. 매 호출 신선 재독(캐시 금지) — 데몬 재시작(churn)마다 토큰이 재발급되기 때문.
/// 부재·빈 파일=None(구 데몬 호환 — 첨부 없이 기존대로 호출).
fn read_operator_token() -> Option<String> {
    #[cfg(windows)]
    let dir = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").ok()?).join("cys");
    #[cfg(not(windows))]
    let dir = default_socket().parent()?.to_path_buf();
    let tok = std::fs::read_to_string(dir.join("operator.token")).ok()?;
    let tok = tok.trim().to_string();
    (!tok.is_empty()).then_some(tok)
}

#[tauri::command]
async fn feed_reply(request_id: String, decision: String) -> Result<(), String> {
    // ★GUI 오퍼레이터 승인(오너 2026-07-15): operator.token을 첨부해 §3.2 자기승인 가드의 GUI 오탐
    // (부서 생성 체인 pgid 각인 + surface 미귀속 fail-closed)을 면제한다. 첨부 지점은 이 Tauri 백엔드
    // 단 한 곳 — 공용 cys CLI 무첨부는 워커의 **우발적** 면제만 차단한다(의도적 동일사용자
    // 프로세스는 토큰 파일을 읽어 raw RPC로 우회 가능 — M11 수준·사고 방지용).
    async fn call(request_id: &str, decision: &str) -> Result<Value, String> {
        let mut params = json!({"request_id": request_id, "decision": decision});
        if let Some(tok) = read_operator_token() {
            params["operator_token"] = json!(tok);
        }
        rpc_full(&default_socket(), "feed.reply", params).await
    }
    let mut resp = call(&request_id, &decision).await?;
    if resp["ok"].as_bool() != Some(true)
        && resp["error"]["code"].as_str() == Some("self_approval_denied")
    {
        // 첫 호출의 파일 읽기와 데몬 재시작(토큰 회전)이 겹친 좁은 창 — 신선 재독으로 1회만 재시도.
        resp = call(&request_id, &decision).await?;
    }
    if resp["ok"].as_bool() == Some(true) {
        Ok(())
    } else {
        // UI가 사유를 분류·표시할 수 있게 코드를 보존해 반환(에러 은폐 제거의 짝).
        Err(format!(
            "{}: {}",
            resp["error"]["code"].as_str().unwrap_or("error"),
            resp["error"]["message"].as_str().unwrap_or("unknown error")
        ))
    }
}

/// Attach: 부서 소켓의 surface PTY 출력을 base64 이벤트로 webview에 스트리밍.
/// 이벤트명은 (소켓 slug, surface_id)로 데몬 간 충돌을 막고, 그 이름을 반환해 UI가 구독한다
/// (백엔드 단일 진실 — UI 독립 재계산 금지, 검증 mustFix).
#[tauri::command]
async fn attach_surface(socket: Option<String>, surface_id: u64) -> Result<Value, String> {
    // 이벤트명만 반환 — 실제 스트림은 start_surface_stream이 시작한다. UI가 이 이름으로 listen을
    // 먼저 등록한 뒤 start를 호출해야, 데몬이 attach 직후 보내는 초기 화면 snapshot(프롬프트)이
    // listen 등록 전에 emit돼 유실되는 race(런치 시 첫 pane 빈 화면)를 차단한다.
    let sock = resolve_socket(&socket);
    let slug = sock_slug(&sock);
    Ok(json!({
        "output_event": format!("surface-output-{slug}-{surface_id}"),
        "exited_event": format!("surface-exited-{slug}-{surface_id}"),
    }))
}

/// 실제 PTY 스트림 시작 — 이전 핸들 abort + connect + surface.attach + 초기 화면 snapshot + live 스트림.
/// UI는 attach_surface로 이벤트명을 받아 listen을 등록한 뒤 이 명령을 호출한다(snapshot 유실 방지).
#[tauri::command]
async fn start_surface_stream(
    app: AppHandle,
    state: State<'_, Attachments>,
    socket: Option<String>,
    surface_id: u64,
) -> Result<(), String> {
    let sock = resolve_socket(&socket);
    let slug = sock_slug(&sock);
    let key = (slug.clone(), surface_id);
    if let Some(prev) = state.0.lock().unwrap().remove(&key) {
        prev.abort();
    }
    let event_name = format!("surface-output-{slug}-{surface_id}");
    let event_exited = format!("surface-exited-{slug}-{surface_id}");
    let (en, ee) = (event_name.clone(), event_exited.clone());
    let handle = tauri::async_runtime::spawn(async move {
        let Ok(mut stream) = connect_to(&sock).await else {
            let _ = app.emit(&ee, ());
            return;
        };
        let req =
            json!({"id": 1, "method": "surface.attach", "params": {"surface_id": surface_id}});
        let mut line = serde_json::to_vec(&req).unwrap_or_default();
        line.push(b'\n');
        if stream.write_all(&line).await.is_err() {
            let _ = app.emit(&ee, ());
            return;
        }
        let mut reader = BufReader::new(stream);
        let mut ack = String::new();
        // ack 검증 — not_found 등 에러 ack에서 read 블록·무신호 죽은 pane이 되지 않게
        if reader.read_line(&mut ack).await.unwrap_or(0) == 0 {
            let _ = app.emit(&ee, ());
            return;
        }
        let ack_v: Value = serde_json::from_str(ack.trim()).unwrap_or(Value::Null);
        if ack_v["ok"].as_bool() != Some(true) {
            let _ = app.emit(&ee, ());
            return;
        }
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    if app.emit(&en, b64).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = app.emit(&ee, ());
    });
    state.0.lock().unwrap().insert(key, handle);
    Ok(())
}

/// 데몬 소켓이 준비될 때까지 connect를 폴링(수동 spawn 없음). `attempts`×100ms.
async fn wait_for_connect(attempts: u32) -> bool {
    for _ in 0..attempts {
        if connect().await.is_ok() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

/// 앱 첫 실행 시 cysd를 launchd에 자동등록(RunAtLoad·KeepAlive) — 재부팅 후에도 데몬 생존.
/// 수동 `cys daemon install`의 opt-in을 자동화한다(`cys::launchd`와 plist 포맷 단일화).
/// 반환값 = **launchd가 cysd 기동을 책임지는가**. true면 setter가 수동 spawn을 건너뛰고
/// launchd-owned cysd의 socket-ready를 폴링해야 한다(중복 spawn·flock 경합 방지 — codex BLOCKER).
#[cfg(target_os = "macos")]
async fn maybe_autoregister_launchd() -> bool {
    // 번들 동봉 cysd 절대경로(ensure_daemon과 동일 규칙) — current_exe()=.../Contents/MacOS/cys-app,
    // 그 parent 가 곧 <bundle>/Contents/MacOS(=classify_bundle_dir 입력)이자 형제 cysd 의 디렉터리다.
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let macos_dir = match exe.parent() {
        Some(d) => d,
        None => return false,
    };
    // ★번들 위치 가드: plist 를 쓰기 **전에** 실행 번들 위치를 분류해 **Canonical(/Applications·
    // ~/Applications)만** 자동등록한다. 무음 autostart 는 명시 사용자설치(plan_cli_install)와 위험
    // 프로파일이 달라(GUI 시작 시 plist 무음 기록) 더 엄격하다: 휘발/이동 경로 — Translocated
    // (/AppTranslocation/…)·Backup(cys.app.bak*/prev*)·NonStandard(~/Downloads·/Volumes/USB 등) — 가
    // plist ProgramArguments 에 각인되면 언마운트·삭제·앱 이동 시 죽은 경로 데몬을 무한 스폰한다(사용자
    // "손상됨"·앱 반복소실의 근본원인). 비-Canonical 은 자동등록만 skip 하고 ensure_daemon 런타임 폴백
    // (휘발성 데몬)으로 안전하게 흐른다.
    let kind = classify_bundle_dir(macos_dir);
    if !autoregister_allowed(&kind) {
        eprintln!(
            "[cys-app] launchd autoregister skipped: 비정규 실행 위치({kind:?}) — \
             Finder에서 cys.app을 Applications로 옮겨 다시 여세요"
        );
        return false;
    }
    // 형제 cysd가 없으면 보류(기존 동작 보존).
    let daemon = macos_dir.join("cysd");
    if !daemon.exists() {
        return false;
    }
    let running = connect().await.is_ok();
    match cys::launchd::register_if_absent(&daemon, running) {
        Ok(outcome) => {
            eprintln!("[cys-app] launchd autoregister: {outcome:?}");
            cys::launchd::launchd_will_serve(outcome)
        }
        Err(e) => {
            eprintln!("[cys-app] launchd autoregister skipped: {e}");
            false
        }
    }
}

/// 첫 기동 온보딩 공용 단계 — `cys init-pack`으로 팩 파일 + Claude SessionStart hook 등록.
/// install은 preserve, hook은 중복 dedup(already→skip·.bak-cys 무변경)이라 **멱등** — 반복 실행해도
/// 안전하다. 호출은 setup의 needs_gui_onboard 게이트(.gui-onboarded 마커)로 조건화된다(v4 · 2026-07-12):
/// 마커 부재(신선 머신·직전 실패)·버전 불일치(업그레이드)에만 실행 — 평시 부트 비용 제거.
/// Windows·macOS 온보딩이 공유한다(autostart는 OS별로 분리: Windows=schtasks·macOS=launchd).
/// 반환 = init-pack 성공 여부(★hook 등록 실패도 rc=1 — cys.rs run_init_pack). false면 호출자가
/// 마커를 기록하지 않아 다음 부트에 재시도된다(best-effort + 재시도 내장). 실패해도 세션은 진행.
#[cfg(any(windows, target_os = "macos"))]
fn onboard_init_pack(cys: &std::path::Path) -> bool {
    let mut init = std::process::Command::new(cys);
    init.arg("init-pack");
    no_console(&mut init);
    match init.status() {
        Ok(s) if s.success() => {
            eprintln!("[cys-app] onboarding: init-pack ok");
            true
        }
        Ok(s) => {
            eprintln!("[cys-app] onboarding: init-pack exited {s}");
            false
        }
        Err(e) => {
            eprintln!("[cys-app] onboarding: init-pack spawn failed: {e}");
            false
        }
    }
}

/// Windows 첫 기동 온보딩(RC-1) — 순정 Windows엔 hook 자동등록 경로가 없어 "너는 마스터다"
/// 부트스트랩(SessionStart hook)이 미발동했다(T1 증상①).
/// ① `onboard_init_pack`: 팩 + Claude hook 등록(멱등).
/// ② `cys daemon install`: 기존 schtasks ONLOGON 자동기동 등록 재사용(cys.rs:3139·/F 멱등).
#[cfg(windows)]
fn maybe_windows_onboard() -> bool {
    let cys = resolve_sidecar("cys.exe");
    let init_ok = onboard_init_pack(&cys);
    // ② autostart 등록 (기존 cys daemon install = schtasks ONLOGON 재사용, /F 멱등)
    let mut reg = std::process::Command::new(&cys);
    reg.arg("daemon").arg("install");
    no_console(&mut reg);
    let reg_ok = match reg.status() {
        Ok(s) if s.success() => {
            eprintln!("[cys-app] windows onboarding: daemon install (schtasks) ok");
            true
        }
        Ok(s) => {
            eprintln!("[cys-app] windows onboarding: daemon install exited {s}");
            false
        }
        Err(e) => {
            eprintln!("[cys-app] windows onboarding: daemon install spawn failed: {e}");
            false
        }
    };
    // 둘 다 성공해야 완료 — 부분 실패는 마커 미기록 → 다음 부트 재시도(멱등이라 안전).
    init_ok && reg_ok
}

/// macOS 첫 기동 온보딩 — Windows 온보딩의 대칭(RC-17·T5). macOS DMG 소비자는 launchd
/// 자동시작(maybe_autoregister_launchd)만 있고 hook 자동등록 경로가 없어 "너는 마스터다"
/// 부트스트랩이 미발동했다. autostart는 launchd가 담당하므로 여기서는 Windows와 대칭으로
/// 팩+Claude hook만 등록한다. init-pack 멱등 — 기존 사용자에 재실행돼도 무해(already→skip·.bak-cys 불변).
#[cfg(target_os = "macos")]
fn maybe_macos_onboard() -> bool {
    let cys = resolve_sidecar("cys");
    onboard_init_pack(&cys)
}

/// Windows: GUI(windows_subsystem)가 콘솔 바이너리(cys/cysd/python3)를 스폰할 때 콘솔 창이
/// 뜨지 않게 CREATE_NO_WINDOW 를 붙인다(검은 빈 Windows Terminal 창·ConPTY 오염 방지). 타 OS 무동작.
fn no_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// RC-5: GUI 직스폰(bash/python3)에 동봉 runtime PATH 주입. GUI(Explorer/Finder) 프로세스 PATH엔
/// runtime이 없어 순정 Windows서 bash/python3 lookup 실패 → ＋부서·티켓 무반응이었다(cysd PTY 자식만
/// 주입 수혜). cysd와 동일한 공용 로직(cys::runtime_prefixed_path) 사용 — 중복 구현 금지.
/// 타 OS는 exe_dir만 얹혀 사실상 무영향(제거 없음).
fn inject_runtime_path(cmd: &mut std::process::Command) {
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    {
        let cur = std::env::var("PATH").unwrap_or_default();
        if let Some(newp) = cys::runtime_prefixed_path(&exe_dir, &cur) {
            cmd.env("PATH", newp);
        }
    }
}

/// Ensure aitermd is running: try to connect, otherwise spawn the bundled/sibling binary.
async fn ensure_daemon() -> Result<(), String> {
    if connect().await.is_ok() {
        return Ok(());
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let daemon_name = if cfg!(windows) { "cysd.exe" } else { "cysd" };
    let candidate = exe_dir.as_ref().map(|d| d.join(daemon_name));
    let program = match candidate {
        Some(p) if p.exists() => p,
        _ => std::path::PathBuf::from(daemon_name), // fall back to PATH
    };
    let mut command = std::process::Command::new(&program);
    command.stdin(std::process::Stdio::null());
    // ★W1-b: 앱-스폰 데몬의 stdout/stderr 를 기본 데몬 로그(launchd StandardErrorPath 와 동일 파일 규약)에
    // O_APPEND 로 잇는다 — 과거 Stdio::null() 로 버려, 락 경쟁 패배·데드맨 판정 등 앱-스폰 데몬의 부트
    // 진단이 통째로 증발했다(launchd-스폰 데몬만 로그가 남았다). open 실패 시 기존 null() 폴백 —
    // 로그를 못 열어도 부트는 막지 않는다(fail-open).
    #[cfg(target_os = "macos")]
    {
        let log = cys::launchd::log_path();
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
        {
            Ok(f) => match f.try_clone() {
                Ok(f2) => {
                    command
                        .stdout(std::process::Stdio::from(f))
                        .stderr(std::process::Stdio::from(f2));
                }
                Err(_) => {
                    command
                        .stdout(std::process::Stdio::from(f))
                        .stderr(std::process::Stdio::null());
                }
            },
            Err(_) => {
                command
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
            }
        }
    }
    // launchd::log_path 는 mac 전용 경로 규약(#![cfg(target_os = "macos")])이라 그 외 OS(windows 포함)는
    // 기존 null() 을 유지한다 — 별도 로그 파일 규약이 정해지면 그때 동등 배선한다.
    #[cfg(not(target_os = "macos"))]
    {
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    no_console(&mut command);
    command
        .spawn()
        .map_err(|e| format!("failed to start cysd ({}): {e}", program.display()))?;
    if wait_for_connect(40).await {
        Ok(())
    } else {
        Err("cysd did not come up within 4s".into())
    }
}

/// Background: 한 데몬의 push 이벤트 스트림을 구독해 webview로 전달.
/// 데몬별 event forwarder 중복 spawn 방지 — restore가 ws마다 launch_dept_daemon을 재호출해도
/// socket당 forwarder 1개만 유지(태스크 누수·daemon-event 중복 emit 차단).
static FORWARDERS: std::sync::OnceLock<Mutex<std::collections::HashSet<std::path::PathBuf>>> =
    std::sync::OnceLock::new();

/// 데몬마다 spawn — 페이로드에 출처 socket_slug를 주입해 UI가 부서를 구분한다(멀티마스터 F3).
fn spawn_event_forwarder(app: AppHandle, socket: std::path::PathBuf) {
    // 멱등 가드: 이 socket의 forwarder가 이미 돌고 있으면 no-op.
    {
        let set = FORWARDERS.get_or_init(|| Mutex::new(std::collections::HashSet::new()));
        if !set.lock().unwrap().insert(socket.clone()) {
            return;
        }
    }
    let slug = sock_slug(&socket);
    tauri::async_runtime::spawn(async move {
        let mut after_seq: Option<u64> = None;
        let mut fails: u32 = 0;
        loop {
            let mut connected = false;
            let attempt: Result<(), String> = async {
                let mut stream = connect_to(&socket).await?;
                connected = true; // 연결 수립 — dead-socket 아님
                let req = json!({"id": 1, "method": "events.stream",
                                 "params": {"after_seq": after_seq}});
                let mut line = serde_json::to_vec(&req).unwrap_or_default();
                line.push(b'\n');
                stream.write_all(&line).await.map_err(|e| e.to_string())?;
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    if let Ok(mut v) = serde_json::from_str::<Value>(&l) {
                        if v["type"] == "event" {
                            if let Some(seq) = v["seq"].as_u64() {
                                after_seq = Some(seq);
                            }
                            if let Some(obj) = v.as_object_mut() {
                                obj.insert("socket_slug".into(), json!(slug));
                            }
                            let _ = app.emit("daemon-event", v);
                        }
                    }
                }
                Err("event stream closed".into())
            }
            .await;
            let _ = attempt;
            // dead-socket 회수: 연속 연결 실패(스트림 수립 실패)가 ~30s 넘으면 forwarder 종료.
            // 스트림 수립 후 종료(데몬 재시작 등)는 정상 재연결 대상이라 카운터를 리셋한다.
            if connected {
                fails = 0;
            } else {
                fails += 1;
                if fails >= 30 {
                    if let Some(set) = FORWARDERS.get() {
                        set.lock().unwrap().remove(&socket);
                    }
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

/// 부서 운용 정식 도구 cys-dept 경로(pack_dir/bin/cys-dept).
fn dept_tool() -> std::path::PathBuf {
    cys::pack::pack_dir().join("bin").join("cys-dept")
}

/// 부서 데몬 소켓 경로 — RC-4: 공용 규약(cys::dept_socket_path)에 위임.
/// Windows=named pipe `\\.\pipe\cys-dept-<name>`, unix=~/.local/state/cys-dept-<name>/cys.sock.
/// (구: HOME 직접사용 unix .sock 고정 → Windows named pipe 미대응·HOME 미설정 이중결함 RC-4/RC-7.)
fn dept_socket_path(name: &str) -> std::path::PathBuf {
    cys::dept_socket_path(name)
}

/// 새 부서 workspace 런칭 = 부서 데몬 spawn. 단일 진입점 cys-dept launch를 OS 호출해
/// 레지스트리·ACL 시드·CEO 승격을 일임한다(직접 cysd spawn 금지, 검증 mustFix). 성공 시
/// 그 데몬용 이벤트 forwarder를 추가 spawn하고 socket·slug·identify를 반환한다.
#[tauri::command]
async fn launch_dept_daemon(app: AppHandle, name: String) -> Result<Value, String> {
    let tool = dept_tool();
    let n = name.clone();
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(bash.exe) PATH 주입
        cmd.arg(&tool).arg("launch").arg(&n);
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    let sock = dept_socket_path(&name);
    spawn_event_forwarder(app.clone(), sock.clone());
    let mut info = rpc_on(&sock, "system.identify", json!({"caller": "ui"})).await?;
    if let Some(obj) = info.as_object_mut() {
        obj.insert("socket".into(), json!(sock.to_string_lossy()));
        obj.insert("socket_slug".into(), json!(sock_slug(&sock)));
    }
    Ok(info)
}

/// 새 부서 번호 백엔드 원자 발급. 번호 계산을 UI가 아닌 레지스트리 flock RMW에 일임해
/// lowest-unused 재사용 + 멀티창 충돌0을 보장한다. stdout 마지막 줄이 확정 name(dept-N).
/// ＋부서 자동화(패치5): `catalog_key`=Some(k) → `cys-dept create <k>`(카탈로그 기반 부서명·계정·미션·각성),
/// None → `cys-dept allocate`(레거시 무변경). create 경로는 레지스트리에서 display_name 을 조회해 반환한다.
#[tauri::command]
async fn allocate_dept_daemon(app: AppHandle, catalog_key: Option<String>) -> Result<Value, String> {
    let tool = dept_tool();
    let ck = catalog_key.clone();
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(bash.exe) PATH 주입
        cmd.arg(&tool);
        match &ck {
            Some(k) => {
                cmd.arg("create").arg(k);
            } // ＋부서 자동화: 카탈로그 키 기반 생성(stdout 마지막 줄=name)
            None => {
                cmd.arg("allocate");
            } // 레거시: 번호만 발급(회귀 무변경)
        }
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        // ＋부서 자동화(gemini R2 ①): create 경로는 exit code 를 'dept-create:<code>:<stderr>' 로 GUI 에 전달해
        //   보안 분기를 가능케 한다 — exit5(account dir 미존재)=계정누수 → 레거시 폴백 절대 금지(하드 에러)·
        //   exit4(키 부재)=에러·exit3(카탈로그 부재)=레거시 허용. 레거시 allocate(None) 경로는 평문 stderr 유지.
        if catalog_key.is_some() {
            let code = out.status.code().unwrap_or(-1);
            return Err(format!("dept-create:{code}:{stderr}"));
        }
        return Err(stderr);
    }
    let name = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .last()
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return Err("allocate: empty name".into());
    }
    let sock = dept_socket_path(&name);
    spawn_event_forwarder(app.clone(), sock.clone());
    let mut info = rpc_on(&sock, "system.identify", json!({"caller": "ui"})).await?;
    if let Some(obj) = info.as_object_mut() {
        obj.insert("socket".into(), json!(sock.to_string_lossy()));
        obj.insert("socket_slug".into(), json!(sock_slug(&sock)));
        obj.insert("name".into(), json!(name));
        // ＋부서 자동화: create 경로면 레지스트리(cys-dept reg_set_meta 가 기록)에서 display_name 조회 →
        // 탭 표시명. create stdout 은 name only(cys-dept 코어 재구현 금지)이므로 depts.json 이 표시명 진실원.
        if catalog_key.is_some() {
            if let Some(disp) = dept_display_name(&name) {
                obj.insert("display_name".into(), json!(disp));
            }
        }
    }
    Ok(info)
}

/// 부서 workspace 닫기 = 부서 데몬 teardown. cys-dept down에 일임(SIGTERM·소켓 정리·레지스트리·CEO 강등).
#[tauri::command]
async fn stop_dept_daemon(name: String) -> Result<(), String> {
    let tool = dept_tool();
    let _ = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(bash.exe) PATH 주입
        cmd.arg(&tool).arg("down").arg(&name);
        no_console(&mut cmd);
        cmd.output()
    })
    .await;
    Ok(())
}

/// 부서 레지스트리(depts.json) 조회 — restore가 등록된 부서(진실원)와 대조해 죽은 socket의 유령 ws를
/// 무비판 재-launch하지 않게 한다(옛 테스트 잔재·삭제된 부서 차단). 부재 시 빈 depts.
#[tauri::command]
fn list_depts() -> Result<Value, String> {
    let reg = std::env::var("CYS_DEPTS_JSON")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            cys::home_dir().join(".cys/depts.json")
        });
    match std::fs::read_to_string(&reg) {
        Ok(s) => serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()),
        Err(_) => Ok(json!({ "depts": {} })),
    }
}

/// 부서 레지스트리(depts.json)에서 표시명 조회 — cys-dept reg_set_meta 가 기록한 display_name.
/// create stdout 은 name only 이므로 표시명의 진실원은 레지스트리다. 부재/오류 시 None(=name 폴백).
fn dept_display_name(name: &str) -> Option<String> {
    let reg = std::env::var("CYS_DEPTS_JSON")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            cys::home_dir().join(".cys/depts.json")
        });
    let s = std::fs::read_to_string(&reg).ok()?;
    let v: Value = serde_json::from_str(&s).ok()?;
    v.get("depts")?
        .get(name)?
        .get("display_name")?
        .as_str()
        .map(|s| s.to_string())
}

/// 부서 카탈로그(dept-catalog.json) 조회 — ＋부서 선택 팝업용. cys-dept 와 동일 경로 규약
/// (CYS_DEPT_CATALOG 또는 $HOME/.cys/dept-catalog.json). 부재/손상 시 빈 departments 반환(팝업=레거시 폴백).
#[tauri::command]
fn read_dept_catalog() -> Result<Value, String> {
    let cat = std::env::var("CYS_DEPT_CATALOG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            cys::home_dir()
                .join(".cys/dept-catalog.json")
        });
    match std::fs::read_to_string(&cat) {
        Ok(s) => serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()),
        Err(_) => Ok(json!({ "departments": {} })),
    }
}

/// ★WP-3(BOOTSTRAP_HARDENING): 소켓 문자열에서 부서명 파생 — cys-dept-<name> 슬러그
/// (unix `.../cys-dept-<n>/cys.sock` · pipe `\\.\pipe\cys-dept-<n>` 공통 · cys-dept D8 파생과 동일 규약).
fn dept_name_from_socket(sock: &str) -> Option<String> {
    let norm = sock.replace('\\', "/");
    norm.split('/')
        .find_map(|seg| seg.strip_prefix("cys-dept-").map(str::to_string))
        .filter(|n| !n.is_empty())
}

/// ★WP-3 의도 선기록: 부서 삭제 클릭의 **제1행위** — base 데몬에 dept 묘비를 기록한다(견고
/// writer=데몬 RPC·topology.json 영속). 이후의 teardown(bash→python 체인·reg_remove)이 무음
/// 실패해도 리바이버(spawn_org_restore·프론트 복원)가 이 묘비를 게이트로 읽어 부활을 차단한다.
#[tauri::command]
async fn dept_tombstone_by_socket(socket: String) -> Result<Value, String> {
    let name = dept_name_from_socket(&socket)
        .ok_or_else(|| format!("부서명 파생 실패(비표준 소켓): {socket}"))?;
    rpc_oneshot(&cys::socket_path(), "dept_tombstone.set", json!({"name": name})).await
}

/// ★WP-3 리바이버 게이트 소스: base 데몬의 dept 묘비 목록(프론트 복원이 유령 판정에 사용).
#[tauri::command]
async fn dept_tombstones() -> Result<Vec<String>, String> {
    let v = rpc_oneshot(&cys::socket_path(), "dept_tombstone.list", json!({})).await?;
    Ok(v.get("dept_tombstones")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default())
}

/// ★WP-1 결정 e(설계 v1.1): "마스터 시작" — cys launch-agent --role master 배선. worker/cso와
/// 동일 메커니즘(앵커 준수: 시스템은 노드만 띄우고 지휘하지 않는다). CYS_SOCKET 제거로 항상
/// base 데몬 대상(부서 오염 불가 — 소켓 격리와 동일 축). 생성된 surface는 GUI 자동입양이 수용.
#[tauri::command]
async fn start_master(app: AppHandle) -> Result<(), String> {
    let cys = resolve_sidecar("cys");
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&cys);
        inject_runtime_path(&mut cmd);
        cmd.env_remove("CYS_SOCKET");
        cmd.arg("launch-agent").arg("--role").arg("master").arg("--agent").arg("claude");
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        // ★(W4 · B5) launch-agent 는 생성한 surface ref 를 stdout 으로 낸다 — 팀 부트 1차 경로
        //   (javis_bootstrap.py)가 ③claim-role 을 **이 pane 에 귀속**시키려면 그 값이 필요하다.
        //   없으면 GUI 는 surface 밖 프로세스라 claim 왕복이 exit 10(세션 컨텍스트 오류)로 죽는다.
        let sref = launched_surface_ref(&out.stdout);
        spawn_orchestra_boot(app, None, sref); // ★절대규칙: 마스터=팀 결정론 스폰(LLM 환각 무관)
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// `cys launch-agent` stdout → 생성된 surface ref("surface:N"). 진단 산문은 stderr 로 가므로
/// stdout 마지막 비어있지 않은 줄이 계약이다(cys.rs run_launch_agent_opts 의 `println!`).
fn launched_surface_ref(stdout: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("surface:"))
        .map(|l| l.to_string())
}

// ════════════════════════════════════════════════════════════════════════════
// 팀 부트 **단일 계약**(T-0147-7 W4 · B5·B15·B16 · H-DOC-8)
//
// ## 무엇이 틀렸었나
// 팀 부트 진입점이 셋(훅 → javis_bootstrap.py 체인 / GUI 버튼 → `cys boot` 직접 / 산문 §0)이었고,
// GUI 만 **체인을 건너뛰어** 자기만의 판정을 갖고 있었다:
//   · 판정 재료가 stdout **산문 문자열**("신규 기동 0"·"미설치")이었다 — 문구가 바뀌면 조용히
//     오작동하고(RC1 사본 드리프트), 정상 상황도 오경보로 읽혔다.
//   · `건강한 팀 + grok 미설치`(사실상 모든 기계)에서 재부트 한 번에 '팀 기동 실패' 토스트가
//     떴다(P3-B16 — 반복성 위경보). 반대로 `claude 만 설치 → 리뷰어 0` 은 **무경고**였다(R5).
//   · 플랫폼별 설치 힌트가 GUI 사본에만 없어서(P3-B15) macOS 명령을 Windows 사용자에게 안내했다.
//
// ## 계약(경로 2 · 계약 1 — 비평2 D-4)
//   1차: `javis_bootstrap.py run` — **훅과 같은 체인**이다(preflight→claim→④boot --json→⑤check).
//        판정은 그 체인의 **타입드 exit 공간**(0/3/4/6/7/8/9/10/11/64)을 소비한다.
//   폴백: python 해소 실패·스크립트 부재 시 `cys boot --json` **직접** 호출 — 이때도 판정은
//        **typed role 표**(outcome·mandatory·install_hint)로 하고, 강등 자체를 `boot-degraded`
//        이벤트로 **기록**한다(조용한 강등 금지). 두 경로가 같은 계약을 소비하므로 '진입점 통일'
//        없이도 판정 이원화가 소멸한다.
//
// ## 경고 규율(B16)
//   경고는 **mandatory(Fatal) 역할이 없을 때만** 낸다. 리뷰어·grok(Degrade)은 대체 폴백·
//   익명 peer-review 로 보완되므로 경고 대상이 아니다. busy(exit 75/11)는 '다른 런이 세우는 중'
//   이라 경고가 아니라 정보다. install_hint 는 **생산자 문구 그대로** 표출한다(사본 금지).
//   ★T-0147-3 정합: 새 신호도 sticky 기본 TTL(60s)로 자동 소멸한다 — '절대 안 사라지는 종류'를
//   만들지 않는다. 단 auto-dismiss 는 이 수리의 대체재가 아니다(오진 이력은 그대로 쌓이므로).
// ════════════════════════════════════════════════════════════════════════════

/// 팀 부트 결과의 사용자 신호 등급 — 조용한 실패·조용한 강등을 **타입으로** 불가능하게 만든다.
#[derive(Debug, Clone, PartialEq)]
enum BootSignal {
    /// 신호 없음(정상 완주 · Degrade-only 포함).
    Silent,
    /// 정보: 실패가 아니지만 사용자가 알아야 하는 상태(busy skip·부서 단독 각성).
    Info(String),
    /// 경고: 팀의 최소 실행 단위가 없다(Fatal) 또는 체인이 깨졌다.
    Warn(String),
}

/// GUI 가 팀 부트 1차 경로에 쓸 python 인터프리터 후보(우선순위).
/// ★후보를 2개로 둔 이유: 동봉 runtime 은 `python3` 를 제공하지만 순정 Windows CPython 은
/// `python` 만 있는 경우가 있다. 이 경로는 **비파괴**(부트 체인)이고 boot 락·싱글플라이트가
/// 중복을 막으므로 후보 확대가 안전하다 — 파괴 경로(reclaim)에서 후보를 넓히지 않는 보수
/// 판정(cys.rs escalate_reclaim)과는 위험 방향이 반대다.
const BOOT_PY_CANDIDATES: [&str; 2] = ["python3", "python"];

/// stdout 에서 **마지막 JSON 오브젝트**를 뽑는다 — 진행 산문과 기계 계약의 공존 규약
/// (python `javis_bootstrap._parse_boot_json` 과 동일 규칙: 마지막 `\n{`…부터 끝까지).
fn parse_last_json_object(s: &str) -> Option<Value> {
    let t = s.trim();
    let cand = match t.rfind("\n{") {
        Some(i) => &t[i + 1..],
        None if t.starts_with('{') => t,
        None => return None,
    };
    serde_json::from_str::<Value>(cand).ok().filter(|v| v.is_object())
}

/// `cys boot --json` role 표 → **Fatal 경고 문구**(없으면 None).
/// 규칙: `mandatory == true` 이고 `outcome ∈ {failed, missing}` 인 역할만 경고 대상이고,
/// `install_hint` 는 **그대로** 인용한다(플랫폼 분기는 생산자 = cys.rs `install_hint()` 소유).
fn boot_json_fatal_message(stdout: &str) -> Option<String> {
    let v = parse_last_json_object(stdout)?;
    let roles = v.get("roles")?.as_array()?;
    let bad: Vec<String> = roles
        .iter()
        .filter(|r| {
            r.get("mandatory").and_then(Value::as_bool).unwrap_or(false)
                && matches!(
                    r.get("outcome").and_then(Value::as_str),
                    Some("failed") | Some("missing")
                )
        })
        .map(|r| {
            let role = r.get("role").and_then(Value::as_str).unwrap_or("?");
            let outcome = r.get("outcome").and_then(Value::as_str).unwrap_or("?");
            match r.get("install_hint").and_then(Value::as_str) {
                Some(h) => format!("{role}={outcome} → {h}"),
                None => format!("{role}={outcome}"),
            }
        })
        .collect();
    if bad.is_empty() {
        return None;
    }
    Some(format!(
        "마스터는 시작됐으나 **의무 노드**가 빠졌습니다: {}. 팀 없이도 마스터 단독 사용은 가능합니다.",
        bad.join(" · ")
    ))
}

/// 폴백 경로(`cys boot --json` 직접)의 판정 — bare exit 신계약(0/1/75)과 typed role 표를 함께 읽는다.
fn cys_boot_signal(code: Option<i32>, stdout: &str) -> BootSignal {
    // busy(75) = 무스폰 skip. 다른 boot(훅 경로 등)가 팀을 세우는 중이므로 경고가 아니다.
    if code == Some(cys::EXIT_BOOT_BUSY) {
        return BootSignal::Info(
            "다른 팀 기동이 이미 진행 중이어서 이번 요청은 건너뜁니다(중복 스폰 방지) — 곧 팀이 올라옵니다."
                .into(),
        );
    }
    if let Some(msg) = boot_json_fatal_message(stdout) {
        return BootSignal::Warn(msg);
    }
    match code {
        Some(0) => BootSignal::Silent,
        // exit 1 = Fatal 이지만 role 표를 못 읽은 경우(스큐·파싱 실패) — fail-closed 로 경고한다.
        Some(c) => BootSignal::Warn(format!(
            "팀 기동이 실패했습니다(cys boot exit {c}) — `cys list` 로 노드 상태를 확인하세요."
        )),
        None => BootSignal::Warn("팀 기동 프로세스가 비정상 종료했습니다(시그널) — `cys list` 확인.".into()),
    }
}

/// 1차 경로(`javis_bootstrap.py run`)의 판정 — 체인의 **타입드 exit 공간**을 그대로 소비한다.
/// 근거 문서: `javis_bootstrap.py` 헤더 exit 표(0/3/4/5/6/7/8/9/10/11/64).
/// stderr 는 실패 상세(`[bootstrap] 단계 실패: <단계> (exit N)\n<detail>`)를 담으며, detail 에는
/// 생산자가 만든 install_hint 가 **그대로** 들어 있다(B15) — GUI 는 그 꼬리를 인용만 한다.
fn bootstrap_chain_signal(code: Option<i32>, stdout: &str, stderr: &str) -> BootSignal {
    let detail = stderr
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    match code {
        Some(0) => {
            // 완주(completed) 또는 부서 단독 각성(solo_awakening — CEO 티켓 부재). 둘 다 성공
            // 경로이므로 stdout 최종 JSON 을 읽어 구분한다(산문 매칭 금지).
            let solo = parse_last_json_object(stdout)
                .and_then(|v| v.get("solo_awakening").and_then(Value::as_bool))
                .unwrap_or(false);
            if solo {
                BootSignal::Info(
                    "부서장이 단독 각성했습니다 — 부서 팀 기동에는 CEO 티켓이 필요합니다\
                     (본부 master 에서 `javis_bootstrap.py issue-ticket --dept <부서>` 발급 후 재시도)."
                        .into(),
                )
            } else {
                BootSignal::Silent
            }
        }
        // 11 = 싱글플라이트 패자(정상 skip · 실패 아님) — 훅이 먼저 발화한 정상 상황이다.
        Some(11) => BootSignal::Info(
            "팀 기동이 이미 진행 중이어서 이번 요청은 건너뜁니다(중복 방지) — 곧 팀이 올라옵니다.".into(),
        ),
        // 7 = claim 정당거부: 살아있는 master 가 이미 있다(이 pane 은 master 가 아니다).
        Some(7) => BootSignal::Warn(
            "이미 다른 master 노드가 있어 이 탭은 master 로 등록되지 않았습니다 — 기존 master 탭을 사용하세요(조직당 master 1명)."
                .into(),
        ),
        Some(c) => BootSignal::Warn(format!(
            "팀 기동 체인이 {} 단계에서 멈췄습니다(javis_bootstrap exit {c}).\n{detail}",
            match c {
                3 => "데몬 확인(②ping)",
                4 => "팀 기동(④cys boot)",
                6 => "노드 생존 확인(⑤check)",
                8 => "레인↔팩 정합(⓪lane-pack)",
                9 => "자원 게이트(④′resource-gate)",
                10 => "세션 컨텍스트(③claim-role)",
                64 => "명령 사용오류(EX_USAGE — 배선 점검)",
                _ => "알 수 없는",
            }
        )),
        None => BootSignal::Warn(format!(
            "팀 기동 체인이 비정상 종료했습니다(시그널).\n{detail}"
        )),
    }
}

/// ★절대규칙(오너 2026-07-15): 모든 마스터(본부·부서장)는 CSO·워커·리뷰어2 팀을 반드시 갖는다.
/// 종전에는 이 팀 스폰이 마스터 LLM의 `cys boot`(디렉티브 §0 ④) 실행에 의존했는데, dept-master가
/// "부서장 스코프=단독 대기"를 **환각**해 boot를 건너뛰는 치명 실사고가 발생했다(2026-07-15). 산문
/// 의존을 제거하고 버튼 경로에서 팀 기동을 코드 결정론으로 강제한다 — 체인은 이미 가동 중인 역할을
/// 건너뛰고(멱등·boot 락·싱글플라이트로 직렬화) 마스터가 나중에 스스로 선언해도 중복이 없다.
/// ★(W4 · B5) 1차 경로가 `cys boot` 직접 호출에서 **`javis_bootstrap.py run` 체인**으로 바뀌었다 —
/// 위 [팀 부트 단일 계약] 주석 참조. fire-and-forget(체인 최악 예산이 길어 UI 무블록).
/// socket=Some 이면 그 부서 소켓 대상(+레인 팩 동반 주입 — G34), None 이면 본부.
/// surface_ref=Some 이면 체인의 ③claim-role 이 그 pane 에 귀속된다(없으면 exit 10 을 맞는다).
fn spawn_orchestra_boot(app: AppHandle, socket: Option<String>, surface_ref: Option<String>) {
    let cys = resolve_sidecar("cys");
    tokio::task::spawn_blocking(move || {
        // 레인 팩 유도(G34) — 부서 소켓이면 그 부서 팩이어야 한다. 본부는 기본 팩.
        let lane_pack: Option<std::path::PathBuf> = socket
            .as_deref()
            .and_then(|s| cys::pack::lane_pack_for_socket(std::path::Path::new(s)));
        let pack = lane_pack.clone().unwrap_or_else(cys::pack::pack_dir);
        let script = pack.join("bin").join("javis_bootstrap.py");

        // 공통 env 배선(두 경로가 **같은 레인**을 보게 한다).
        let apply_env = |cmd: &mut std::process::Command| {
            inject_runtime_path(cmd);
            match &socket {
                Some(s) => {
                    cmd.env("CYS_SOCKET", s);
                    if let Some(p) = &lane_pack {
                        cmd.env("CYS_PACK_DIR", p);
                    }
                }
                None => {
                    cmd.env_remove("CYS_SOCKET");
                }
            }
            if let Some(sref) = &surface_ref {
                cmd.env("CYS_SURFACE_ID", sref);
            }
            no_console(cmd);
        };

        // ── 1차: javis_bootstrap.py run(훅과 동일 체인) ──
        let why = if script.is_file() {
            let mut last_err = String::new();
            for py in BOOT_PY_CANDIDATES {
                let mut cmd = std::process::Command::new(py);
                apply_env(&mut cmd);
                cmd.arg(&script).arg("run");
                match cmd.output() {
                    Ok(o) => {
                        let signal = bootstrap_chain_signal(
                            o.status.code(),
                            &String::from_utf8_lossy(&o.stdout),
                            &String::from_utf8_lossy(&o.stderr),
                        );
                        emit_boot_signal(&app, signal);
                        return;
                    }
                    Err(e) => last_err = format!("{py}: {e}"),
                }
            }
            format!("python 인터프리터 해소 실패({last_err})")
        } else {
            format!("부트 체인 스크립트 부재({})", script.display())
        };

        // ── 폴백: cys boot --json 직접(경로 2·계약 1) + **typed 강등 신호**(조용한 강등 금지) ──
        let _ = app.emit(
            "boot-degraded",
            format!(
                "팀 부트 1차 경로(javis_bootstrap.py 체인)를 쓸 수 없어 `cys boot --json` 직접 호출로 \
                 강등했습니다 — 사유: {why}. 팀은 기동되지만 preflight·역할 등록·생존 확인 단계는 \
                 생략됩니다(팩·python 배선을 점검하세요)."
            ),
        );
        let mut cmd = std::process::Command::new(&cys);
        apply_env(&mut cmd);
        cmd.arg("boot").arg("--json");
        match cmd.output() {
            Ok(o) => emit_boot_signal(
                &app,
                cys_boot_signal(o.status.code(), &String::from_utf8_lossy(&o.stdout)),
            ),
            Err(e) => emit_boot_signal(
                &app,
                BootSignal::Warn(format!("팀 기동(cys boot) 실행 실패: {e}")),
            ),
        }
    });
}

/// 판정 → 이벤트. Silent 는 아무것도 내지 않는다(위경보 0). Info/Warn 은 각자 채널로 —
/// UI 는 둘 다 sticky 토스트로 받고 T-0147-3 의 TTL(60s)로 자동 소멸한다.
fn emit_boot_signal(app: &AppHandle, signal: BootSignal) {
    match signal {
        BootSignal::Silent => {}
        BootSignal::Info(m) => {
            let _ = app.emit("boot-info", m);
        }
        BootSignal::Warn(m) => {
            let _ = app.emit("boot-warning", m);
        }
    }
}

/// ★R8(WP-2·적대검증 W2): CEO 승격 대기(PENDING) 여부 — cys-dept가 기록한 상태 파일 존재 검사.
/// 프론트가 시작 시 1회+팔레트 온디맨드로 읽는다(신규 타이머 금지 — WINAUDIT 타이머 증식 방지).
#[tauri::command]
fn ceo_pending() -> bool {
    cys::home_dir().join(".cys/state/ceo-pending").exists()
}

/// ★R8: PENDING 해소 실행 — cys-dept promote-if-pending(대기형·자체 동의 게이트 feed --wait 경유).
/// GUI는 role-less(CYS_ROLE 제거 명시)라 단일소유 가드를 통과한다. async라 UI 무블록,
/// feed --wait의 timeout(deny/timeout=보류) 규약이 상한을 보장한다.
#[tauri::command]
async fn promote_pending_ceo() -> Result<String, String> {
    let tool = dept_tool();
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd);
        cmd.env_remove("CYS_SOCKET");
        cmd.env_remove("CYS_ROLE");
        cmd.arg(&tool).arg("promote-if-pending");
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let txt = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(txt.trim().to_string())
    } else {
        Err(txt.trim().to_string())
    }
}

/// ★CEO 승격 Allow 결함 수리(오너 2026-07-15): 승격 요청은 cys-dept가 `feed push --wait`로 만드는
/// 단명 프로세스인데, 오너가 Allow를 누를 무렵 그 대기자는 이미 timeout으로 죽어 있어 승격 행위가
/// 실행되지 않았다(버튼이 먹통). 결정을 대기자에서 분리 — Allow 시 GUI가 이 커맨드로 승격을 **직접**
/// 집행한다. `cys-dept promote-ceo`(오너 지명=consented 경로)는 feed 재질의 없이 directive를 교체한다.
/// role-less(CYS_ROLE 제거)로 단일소유 가드 통과·base 소켓(CYS_SOCKET 제거) 대상.
#[tauri::command]
async fn approve_ceo_promotion() -> Result<String, String> {
    let tool = dept_tool();
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd);
        cmd.env_remove("CYS_SOCKET");
        cmd.env_remove("CYS_ROLE");
        cmd.arg(&tool).arg("promote-ceo");
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let txt = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(txt.trim().to_string())
    } else {
        Err(txt.trim().to_string())
    }
}

/// ★조직 모델(오너 2026-07-15): 부서 탭의 "▶부서장" — 해당 부서 데몬에 master(부서장) 노드 기동.
/// start_master(base=CEO 자리)와 대칭·동일 메커니즘(launch-agent). CYS_SOCKET=부서 소켓으로
/// 그 부서 데몬이 pane을 spawn하므로 부서 팩 디렉티브(MASTER_DIRECTIVE)가 자동 주입되고,
/// claim도 그 부서 레지스트리 대상(데몬당 살아있는 마스터 1명 규칙은 부서별 독립 적용).
/// ★(W4 · G34 의 GUI 지점) **CYS_SOCKET 단독 주입 금지** — 부서 소켓만 주면 데몬·자식이 **본부 팩**을
/// 물려받아 ①그 부서 마스터 선언이 레인↔팩 가드에 `exit 8` 로 **영구 차단**되고 ②본부 팩을 교차
/// 서빙해 계정 격리가 붕괴하고 schedule 이 중복 발화한다(재현: 부서 데몬 사망 후 ▶부서장 클릭).
/// 그래서 (소켓, 팩) **쌍**을 함께 주입한다. 유도는 `cys::pack::lane_pack_for_socket` **단일 소스**를
/// 쓴다(CLI autostart 가 쓰는 그 함수 — 중복 구현 금지). 유도 불가·팩 미실재는 **Err**(토스트)로
/// 거부한다: 새 부서 팩을 자동 창설하지 않는다(부서 실체 없이 데몬만 뜨는 유령 레인 방지).
#[tauri::command]
async fn start_dept_master(app: AppHandle, socket: String) -> Result<(), String> {
    let cys = resolve_sidecar("cys");
    let lane_pack = cys::pack::lane_pack_for_socket(std::path::Path::new(&socket)).ok_or_else(|| {
        format!("부서 팩 경로를 유도할 수 없습니다(비표준 소켓: {socket}) — 부서를 정규 이름으로 재생성하세요.")
    })?;
    if !lane_pack.is_dir() {
        return Err(format!(
            "부서 팩이 없습니다({}) — 이 부서는 실체가 없습니다. 부서를 다시 생성한 뒤 부서장을 기동하세요\
             (본부 팩으로 부서 데몬을 띄우면 부서 부트가 영구 차단됩니다).",
            lane_pack.display()
        ));
    }
    let socket_boot = socket.clone(); // 아래 팀 부트용(첫 클로저가 socket을 move)
    let pack_env = lane_pack.clone();
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&cys);
        inject_runtime_path(&mut cmd);
        cmd.env("CYS_SOCKET", &socket);
        cmd.env("CYS_PACK_DIR", &pack_env); // ★G34: 소켓과 팩은 항상 쌍으로 간다
        cmd.arg("launch-agent").arg("--role").arg("master").arg("--agent").arg("claude");
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        // ★절대규칙: 부서장도 팀 결정론 스폰(환각 무관). 부서 레인은 ④-c CEO 티켓 게이트를 타므로
        //   티켓이 없으면 체인이 '단독 각성'(exit 0·solo_awakening)으로 강등되고, GUI 는 그 사실을
        //   boot-info 로 **명시**한다(조용한 무동작 금지 — 종전 GUI 는 티켓 게이트를 우회했다).
        let sref = launched_surface_ref(&out.stdout);
        spawn_orchestra_boot(app, Some(socket_boot), sref);
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 부서 데몬 teardown(socket 기준) — ws 이름 변경(rename)으로 name→socket 매핑이 끊겨도 정확히 종료.
/// cys-dept down-sock에 일임(레지스트리 역인덱스로 부서명 해석 후 teardown).
#[tauri::command]
async fn stop_dept_daemon_by_socket(socket: String) -> Result<(), String> {
    let tool = dept_tool();
    let _ = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(bash.exe) PATH 주입
        cmd.arg(&tool).arg("down-sock").arg(&socket);
        no_console(&mut cmd);
        cmd.output()
    })
    .await;
    Ok(())
}

/// ★기능2(2026-07-15): 부서 완전 폐역(purge) — teardown을 넘어 대화기억(state·transcripts.db)까지
/// 격리해 부활을 영구 차단한다. javis_org.py destroy 오케스트레이터에 일임(state·pack-dept
/// 2디렉토리를 ~/.local/state/cys-trash/ 로 격리·묘비 영구 존치·재발견 glob 절단). CSO 전용 게이트라
/// CYS_ROLE=cso 로 호출하고, base 레지스트리 대상이므로 CYS_SOCKET 은 제거한다(부서 소켓 오염 방지).
/// 실패는 Err 로 GUI 에 정직 표기(무음 삼킴 금지). stop_dept_daemon_by_socket 과 socket→name 규약 공유.
/// ★D2a(purge-safety 2026-07-16): --purge-workdir 는 GUI 에서 요청하지 않는다 — 실사고: 전 부서
/// 레지스트리 cwd=$HOME(공유 에이전트 작업 디렉토리)라 홈 전체 스냅샷(TCC .Trash 에서 사망)·성공 시
/// 홈 mv 파괴 경로였다. 백엔드 D1a 게이트(workdir_owned 선언제)가 이중 방어하나 GUI 계약도 정직하게
/// "작업 폴더 보존"으로 고정한다(모달 고지문과 동일 커밋 — 변경 결합).
#[tauri::command]
async fn purge_dept_daemon_by_socket(socket: String) -> Result<String, String> {
    let name = dept_name_from_socket(&socket)
        .ok_or_else(|| format!("부서명 파생 실패(비표준 소켓): {socket}"))?;
    let script = cys::pack::pack_dir().join("bin").join("javis_org.py");
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("python3");
        inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(python3.exe) PATH 주입
        cmd.env_remove("CYS_SOCKET"); // base 레지스트리 대상(부서 소켓 오염 방지)
        cmd.env("CYS_ROLE", "cso"); // destroy 는 CSO 전용 게이트(require_cso)
        cmd.arg(&script)
            .arg("destroy")
            .args(["--dept", &name])
            .arg("--purge")
            .arg("--purge-state");
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("javis_org destroy 실행 실패: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// ★기능2: 완전 삭제 확인 다이얼로그 프리뷰 — 격리될 state 디렉토리(대화기억)의 크기·최종 수정시각과
/// "이 부서가 마지막인가(→CEO 강등)"를 반환한다. 사용자가 무엇을 삭제하는지 읽고 결정하도록 하는 근거.
/// 읽기 전용(stat·registry 조회) — 부작용 없음.
#[tauri::command]
fn dept_purge_preview_by_socket(socket: String) -> Result<Value, String> {
    let name = dept_name_from_socket(&socket)
        .ok_or_else(|| format!("부서명 파생 실패(비표준 소켓): {socket}"))?;
    // state 디렉토리 = 부서 소켓의 부모(dept_socket_path 규약과 동일).
    let state_dir = dept_socket_path(&name)
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| dept_socket_path(&name));
    fn dir_size(p: &std::path::Path) -> u64 {
        let mut total = 0u64;
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                match e.file_type() {
                    Ok(ft) if ft.is_dir() => total += dir_size(&e.path()),
                    Ok(_) => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                    _ => {}
                }
            }
        }
        total
    }
    let (size_bytes, mtime_secs, exists) = match std::fs::metadata(&state_dir) {
        Ok(m) => {
            let mt = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (dir_size(&state_dir), mt, true)
        }
        Err(_) => (0, 0, false),
    };
    // 부서 수(depts.json) — 1이면 이 삭제가 마지막 → CEO 강등 고지.
    let dept_count = list_depts()
        .ok()
        .and_then(|r| r.get("depts").and_then(|d| d.as_object()).map(|o| o.len()))
        .unwrap_or(0);
    Ok(json!({
        "name": name,
        "state_dir": state_dir.to_string_lossy(),
        "exists": exists,
        "size_bytes": size_bytes,
        "mtime_secs": mtime_secs,
        "dept_count": dept_count,
        "is_last": dept_count <= 1,
    }))
}

/// A안(2026-07-11 오너 승인): 교대·설치 게이트가 세는 "지킬 세션" = **role 또는 agent 가 붙은
/// 살아있는 surface**만. 맨 셸 pane(role·agent 모두 없음)은 drain+restore 가 되살리므로 무손실
/// 자동 교대를 막지 않는다 — 종전 '살아있는 pane 전부' 기준은 기본 pane 1개만으로 자동 교대가
/// 영영 보류돼, 사용자가 taskkill 로 데몬을 죽여야 업데이트되던 실사고(2026-07-10 Windows)의 근원.
/// 한계(명시): 맨 pane에서 role 미claim 프로그램을 수동 실행 중이면 그 포그라운드 상태는 교대 시
/// 복원되지 않는다(pane 자체는 복원됨).
fn session_blocks_rotation(s: &Value) -> bool {
    if s["exited"].as_bool().unwrap_or(true) {
        return false;
    }
    let has = |k: &str| s[k].as_str().map(|v| !v.is_empty()).unwrap_or(false);
    has("role") || has("agent")
}

/// 부서 소켓의 살아있는 세션 수 — live_session_count(메인 데몬 전용·기본 소켓 하드코딩)의 부서판.
/// rotate_dept_daemon force 가드용. 판정 규칙은 live_session_count와 동일(session_blocks_rotation)하되
/// 대상만 부서 소켓으로 파라미터화(rpc_on). 조회 실패는 호출부에서 0으로 접어 보수적으로 처리.
async fn dept_live_session_count(sock: &std::path::Path) -> Result<u64, String> {
    let r = rpc_on(sock, "surface.list", json!({})).await?;
    let n = r["surfaces"]
        .as_array()
        .map(|a| a.iter().filter(|s| session_blocks_rotation(s)).count() as u64)
        .unwrap_or(0);
    Ok(n)
}

/// 부서 데몬 버전 스큐 세대교체(재기동) — 메인 rotate_daemon의 부서판. `cys-dept rotate <name>`에 일임한다:
/// 데몬 프로세스만 정지→새 on-disk cysd로 재기동하고 **레지스트리·phoenix 묘비·CEO는 건드리지 않는다**
/// (down=폐기와 결정적 차이 — CSO 단일소유 부서 생성/폐기 권한 불침범·rotate=순수 재기동). force 가드는
/// rotate_daemon 동형이되 대상이 부서 소켓이라 세션 카운트를 dept_live_session_count(부서소켓 surface.list)로
/// 산출한다(live_session_count는 메인 전용이라 재사용 불가). 반환=새 데몬 identify(+rotate_log) — UI 스큐 해소 판정.
#[tauri::command]
async fn rotate_dept_daemon(app: AppHandle, name: String, force: bool, skip_drain: bool) -> Result<Value, String> {
    let sock = dept_socket_path(&name);
    // 세션 가드(rotate_daemon 동형). ★F1(리뷰): force=false는 카운트 실패를 0으로 접지 않고
    // Err("live_sessions:unknown")로 보류한다 — 세션 보유 부서를 무확인 교대할 위험 차단(UI가 held 분류·다음
    // tick 재시도). Ok(0)만 진행. force=true(사용자 확인 완료)는 카운트 건너뜀.
    if !force {
        match dept_live_session_count(&sock).await {
            Ok(0) => {}
            Ok(n) => return Err(format!("live_sessions:{n}")),
            Err(_) => return Err("live_sessions:unknown".to_string()),
        }
    }
    // drain(best-effort): 교대 전 부서 노드에 저장 신호. 부서 소켓 대상(CYS_SOCKET)으로 cys drain 실행
    // (메인 rotate_daemon의 drain 동형·spawn_blocking 패턴 일치). cys drain 자체 watchdog로 hang 시에도 종료.
    // ★skip_drain: verified 재시작은 사전 `cys drain --verify`로 저장 확인됨 → 이중 drain 생략(회귀 0=false).
    if !skip_drain {
        let dsock = sock.to_string_lossy().into_owned();
        let _ = tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" }));
            cmd.env(cys::ENV_SOCKET, &dsock);
            cmd.arg("drain");
            no_console(&mut cmd);
            cmd.status()
        })
        .await;
    }
    // cys-dept rotate <name> — 프로세스 정지→새 바이너리 재기동(reg_upsert 메타보존·묘비 불변).
    // launch_dept_daemon의 bash+inject_runtime_path+no_console+spawn_blocking 패턴 동형.
    let tool = dept_tool();
    let n = name.clone();
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("bash");
        inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(bash.exe) PATH 주입
        cmd.arg(&tool).arg("rotate").arg(&n);
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    // 이벤트 포워더 재확립 + 새 데몬 identify(버전 확인·UI 스큐 해소 판정). launch_dept_daemon 반환 동형.
    spawn_event_forwarder(app.clone(), sock.clone());
    let mut info = rpc_on(&sock, "system.identify", json!({"caller": "ui"})).await?;
    if let Some(obj) = info.as_object_mut() {
        obj.insert("socket".into(), json!(sock.to_string_lossy()));
        obj.insert("socket_slug".into(), json!(sock_slug(&sock)));
        // rotate verb의 "rotated <name>: vX→vY" 확정 줄(검증 게이트) 전달 — 사람 로그·성공 판정 보조.
        if let Some(l) = String::from_utf8_lossy(&out.stdout)
            .lines()
            .rev()
            .find(|l| l.starts_with("rotated "))
        {
            obj.insert("rotate_log".into(), json!(l));
        }
    }
    // (T3) rotate는 graceful_kill로 노드 PTY를 동반 종료하고 새 데몬은 surface 0개로 뜬다. 콜드부트
    // auto-restore가 돌지만 실패할 수 있어(2026-07-12 dept-4 실사고: 콜드부트 복원 FAILED·미가시)
    // 사이드카 restore로 명시 복원한다(방금 rotate로 데몬은 살아있음·run_restore 멱등이라 이미 되살렸으면 no-op).
    // restore_ok를 반환 info에 실어 UI(manualRotateSkewed)가 복원 실패를 삼키지 않게 한다(dept-4 계열 가시화).
    let restore_ok = run_sidecar_restore(Some(sock.clone())).await;
    if let Some(obj) = info.as_object_mut() {
        obj.insert("restore_ok".into(), json!(restore_ok));
    }
    Ok(info)
}

/// 업데이트 체크·설치 공용 updater 핸들. CYS_UPDATE_MANIFEST_URL(테스트 전용 env)이 있으면 그
/// 엔드포인트로 오버라이드한다 — 패치 채널 E2E 실기기 검증용(Finder 런칭엔 env가 없어 프로덕션
/// 경로는 tauri.conf 기본 엔드포인트 그대로). ★서명 검증 불변: 설치는 baked pubkey로 .sig를
/// 검증하므로 엔드포인트 교체가 위조 패키지 설치를 허용하지 않는다.
fn build_updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    if let Some(u) = cys::env_compat("CYS_UPDATE_MANIFEST_URL") {
        let url: tauri::Url = u
            .parse()
            .map_err(|e| format!("CYS_UPDATE_MANIFEST_URL 파싱 실패: {e}"))?;
        return app
            .updater_builder()
            .endpoints(vec![url])
            .map_err(|e| e.to_string())?
            .build()
            .map_err(|e| e.to_string());
    }
    app.updater().map_err(|e| e.to_string())
}

/// 테스트 전용(패치 채널 E2E — 오너 2026-07-15): CYS_AUTOTEST_PATCH_INSTALL=1 env로 기동된
/// 경우에만 true — UI가 기동 직후 패치 설치를 무클릭 자동 발화한다(Finder 런칭엔 env 부재 →
/// 프로덕션 무영향).
#[tauri::command]
fn autotest_patch_install() -> bool {
    cys::env_compat("CYS_AUTOTEST_PATCH_INSTALL").as_deref() == Some("1")
}

/// 업데이트 확인: 새 버전이 있으면 (version, notes)를 반환, 없으면 null.
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<Option<Value>, String> {
    let updater = build_updater(&app)?;
    match updater.check().await.map_err(|e| e.to_string())? {
        Some(update) => Ok(Some(json!({
            "version": update.version,
            "current": update.current_version,
            "notes": update.body,
        }))),
        None => Ok(None),
    }
}

/// 기본 원격 pack-manifest.json URL — tauri.conf updater endpoint(latest.json)와 같은
/// release 'latest' 자산에 동봉된다(release.yml이 함께 업로드, DESIGN §5 파일맵).
fn default_pack_manifest_url() -> String {
    // Phase 2 릴리스 통합(2026-07-03): 배포 원본 = 공개 소스 repo. 구 repo는 전환기 미러.
    "https://github.com/idoforgod/cys-terminal/releases/latest/download/pack-manifest.json"
        .to_string()
}

/// 무중단 팩 업데이트 가용성 확인(DESIGN §7-④ 3축 게이트) — 원격 pack-manifest.json만 경량
/// 페치(curl)해 디스크 `.pack-version` 및 실행 바이너리 버전과 비교한다. ★pack.tar.gz·서명은
/// 받지 않는다(폴링 비용 최소화) — 실제 다운로드·서명검증·원자적 반영·reinject는
/// install_pack_update(사이드카 cys pack-update)가 전담한다(불가침).
/// 반환(★3상태 — UI가 'transient 장애'와 '확인된 no-update'를 구분해 fail-safe 상태보존):
///   - Ok(Some({pack_version, manifest_url, min_binary_version, binary_too_old}))
///       → 확인된 새 팩 있음. binary_too_old=false=무중단 가능(install_pack_update 경로) /
///         true=min_binary_version > 실행 바이너리 = 무중단 거부, 바이너리(재시작) 경로 안내.
///   - Ok(None)  → ① 정상 no-update(원격을 받아·파싱해 비교했고 디스크보다 새것이 아님) 또는
///                 ② 미서명/필수필드 부재 manifest의 fail-closed 거부(보안 경계 — 받았으나 신뢰 불가,
///                 설치 안 함). UI는 이때만 packUpdateAvailable을 해제한다(확인된 '새 팩 없음').
///   - Err(..)   → ★일시 fetch 장애(spawn/join·curl 실행·HTTP 비정상). UI의 기존 catch가
///                 packCheckFailed=true로 잡아 마지막 검증 상태를 보존하고 토스트는 띄우지 않는다
///                 (silent 폴링). '확인된 no-update'와 섞지 않는 게 핵심 — 일시 장애로
///                 packUpdateAvailable이 소거돼 배지가 사라지는 것을 막는다.
#[tauri::command]
async fn check_pack_update(manifest_url: Option<String>) -> Result<Option<Value>, String> {
    let url = manifest_url.unwrap_or_else(default_pack_manifest_url);
    // 경량 페치: manifest JSON만 stdout으로. blocking 풀에서 실행(install_pack_update curl 패턴 동형).
    let fetch_url = url.clone();
    // ★transient 실패(spawn/join·curl 실행·HTTP 비정상)는 Err로 돌린다 — UI catch가 상태보존(silent).
    //   Ok(None)으로 접으면 '확인된 no-update'와 구분 불가 → 일시 장애에 배지 소거(codex R2 #1).
    let joined = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("curl");
        cmd.args(["-fsSL", &fetch_url]);
        // startup + 6시간 폴링마다 실행 — GUI(무콘솔)가 콘솔 자식(curl)을 그냥 스폰하면
        // Win11(기본터미널=WT)에서 검은 창이 깜빡인다. 첫 실행 flash의 단일 최우선 원인.
        no_console(&mut cmd);
        cmd.output()
    })
    .await;
    let out = match joined {
        Ok(Ok(out)) if out.status.success() => out,
        Ok(Ok(out)) => return Err(format!("pack-manifest HTTP 실패(code {:?})", out.status.code())),
        Ok(Err(e)) => return Err(format!("curl 실행 실패: {e}")),
        Err(e) => return Err(format!("curl join 실패: {e}")),
    };
    // 미서명/필수필드 부재 manifest = packsig PackManifest 역직렬화 fail-closed(거부) = 보안 경계.
    //   받았으나 신뢰 불가 → '새 팩 없음'으로 취급(Ok(None), 설치 안 함). fetch 장애(Err·상태보존)와
    //   달리 재시도해도 동일하므로 unknown이 아닌 확정 거부 — UI는 packUpdateAvailable을 해제한다.
    let manifest: cys::packsig::PackManifest = match serde_json::from_slice(&out.stdout) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };
    let disk = std::fs::read_to_string(cys::pack::pack_dir().join(".pack-version"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    // 축1 반영 판정: remote가 디스크보다 strictly-newer 여야. ★여기서 false면 '확인된 no-update' = Ok(None).
    if !cys::pack::remote_is_newer(&manifest.pack_version, &disk) {
        return Ok(None);
    }
    // 축2 호환 게이트: min_binary_version ≤ 실행 바이너리(env CARGO_PKG_VERSION = 단일 버전선).
    let binary_too_old = pack_binary_too_old(&manifest.min_binary_version, env!("CARGO_PKG_VERSION"));
    Ok(Some(json!({
        "pack_version": manifest.pack_version,
        "min_binary_version": manifest.min_binary_version,
        "manifest_url": url,
        "binary_too_old": binary_too_old,
    })))
}

/// 무중단 호환 게이트(DESIGN §7-④ 축2) 순수 판정 — min_binary_version > 실행 바이너리면 true(무중단
/// 거부=바이너리 경로). 빈 값=제약 없음(false), 어느 쪽이든 파싱 실패=거부(true, 보수적).
/// cys.rs version_gates의 호환 게이트와 동일 의미 — 단위테스트 대상.
fn pack_binary_too_old(min_binary: &str, running: &str) -> bool {
    let min = min_binary.trim();
    if min.is_empty() {
        return false;
    }
    match (cys::pack::parse_semver(min), cys::pack::parse_semver(running)) {
        (Some(m), Some(r)) => m > r,
        _ => true,
    }
}

/// 데몬 핸드오프 정책(오너 결정): 살아있는 세션 0개면 데몬 종료까지 자동,
/// 있으면 거부하고 세션 수를 알려 UI가 확인을 받게 한다(force=true면 강행).
/// 반환: 종료된 세션 수.
#[tauri::command]
async fn live_session_count() -> Result<u64, String> {
    let r = rpc("surface.list", json!({})).await?;
    let n = r["surfaces"]
        .as_array()
        .map(|a| a.iter().filter(|s| session_blocks_rotation(s)).count() as u64)
        .unwrap_or(0);
    Ok(n)
}

/// 업데이트 다운로드·설치 후 데몬 핸드오프 + 재시작.
/// force=false: 살아있는 세션이 있으면 설치 전에 거부(UI가 확인 후 force=true로 재호출).
/// ★재배선(오너 2026-07-15): 본체 패치(인앱) 설치 경로 재활성화 — UI promptBinaryPatch가 호출한다
///   (구 T5 홈페이지 전용 정책의 실험적 개정 · 실기기 검증 대상). 아래 app.restart() 레이스 경고 참조.
#[tauri::command]
async fn install_update(app: AppHandle, force: bool) -> Result<(), String> {
    // 1) 세션 가드 (오너 정책: 없으면 자동·있으면 확인)
    let sessions = live_session_count().await.unwrap_or(0);
    if sessions > 0 && !force {
        return Err(format!("live_sessions:{sessions}"));
    }
    // 2) 업데이트 받아 설치 (.app 번들 교체 — 새 cysd/cys 동봉)
    let updater = build_updater(&app)?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("no update available")?;
    let _ = app.emit("update-progress", json!({"phase": "download"}));
    update
        .download_and_install(
            |chunk, total| {
                let _ = app.emit(
                    "update-progress",
                    json!({"phase": "download", "chunk": chunk, "total": total}),
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // 3) 데몬 핸드오프: 구 데몬을 정상 종료(SIGTERM — scoped 정리·소켓 제거)해야
    //    재시작 후 새 번들의 cysd가 뜬다. 종료 안 하면 구 데몬이 계속 세션을 들고 돈다.
    // drain(best-effort): 재시작 전 살아있는 노드에 저장 신호 + 유예를 준다. 노드 LLM 협조 의존이라
    // 무손실 보장은 아니며(마지막 미저장분은 손실 가능), 주 복원 경로는 재시작 후 resume이다.
    // spawn_blocking으로 tokio 워커 점유를 막는다(파일 내 launch_dept_daemon 패턴과 일치). cys drain은
    // 자체 watchdog(12s)로 hang 시에도 종료되므로 별도 timeout 없이 await해도 업데이트가 멈추지 않는다.
    let _ = app.emit("update-progress", json!({"phase": "drain"}));
    let _ = tokio::task::spawn_blocking(|| {
        let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" }));
        cmd.arg("drain");
        no_console(&mut cmd);
        cmd.status()
    })
    .await;
    let _ = app.emit("update-progress", json!({"phase": "handoff"}));
    // 재시작 후 자동복귀 예약 — 새 cys-app setup이 이 마커를 보고 cys restore로 노드를 resume 재런칭한다.
    let _ = std::fs::write(pending_restore_path(), "");
    stop_running_daemon().await;
    // 4) 앱 재시작 — setup의 ensure_daemon이 새 cysd를 자동 기동, maybe_restore_after_update가 노드 복원
    // ★재활성화 경고(현재 이 경로는 휴면 — 본체 업데이트는 홈페이지 전용 T5): single-instance 플러그인이
    // 등록돼 있어 restart()의 신 프로세스가 구 프로세스의 인스턴스 락과 레이스할 수 있다(신 인스턴스가
    // 죽어가는 구 인스턴스로 포워딩 후 종료 → 앱 미복귀). 이 경로를 되살릴 때 반드시 실기기 검증하라.
    app.restart();
}

/// 데몬 세대교체(업데이트 없이) — Windows rename-swap 후 lame-duck 스큐(구 데몬 + 새 앱)의
/// 지연 핸드오프 완결(P2 스큐 배지의 짝). NSIS 경로는 install_update의 핸드오프 코드가 실행될 수
/// 없어(인스톨러가 앱을 죽임) 디스크만 새 버전·프로세스는 구 버전으로 남는다 — 이 command가
/// install_update 3~4단계를 업데이트 없이 재현한다: drain → 복귀 마커 → 구 데몬 종료 →
/// 디스크의 새 cysd 기동. app.restart()가 없어 setup이 다시 돌지 않으므로
/// maybe_apply_pending_update(팩 반영 + cys restore 노드 복원)를 여기서 직접 수행한다.
/// ★update-progress는 emit하지 않는다 — drain/handoff 페이즈가 UI "업데이트 설치" sticky 토스트를
/// 만드는데 이 경로엔 재시작이 없어 영구 잔류한다. 진행 표시는 UI(checkVersionSkew/manualRotateSkewed) 토스트 담당.
/// force=false: 살아있는 세션이 있으면 거부(UI가 확인 후 force=true로 재호출) — install_update 가드 동형.
#[tauri::command]
async fn rotate_daemon(app: AppHandle, force: bool, skip_drain: bool) -> Result<(), String> {
    // ★F1(리뷰): force=false는 UI checkVersionSkew(무손실 자동 교대)만 호출한다 — 세션 카운트 실패를 0으로
    // 접으면 세션 보유 노드를 무확인 교대할 위험 → Err("live_sessions:unknown")로 보류(UI가 "held" 분류·다음
    // tick 재시도). Ok(0)만 진행. force=true(사용자 확인 완료 수동 경로)는 카운트 자체를 건너뛴다(무영향).
    if !force {
        match live_session_count().await {
            Ok(0) => {}
            Ok(n) => return Err(format!("live_sessions:{n}")),
            Err(_) => return Err("live_sessions:unknown".to_string()),
        }
    }
    // drain(best-effort): 교대 전 살아있는 노드에 저장 신호 + 유예 (install_update 3단계 동형).
    // ★skip_drain: verified 재시작 경로는 사전에 `cys drain --verify`로 저장을 확인했으므로 여기서
    // 이중 drain을 생략한다. 기존 무손실 자동교대·수동 '바로 재시작'은 skip_drain=false로 거동 불변(회귀 0).
    if !skip_drain {
        let _ = tokio::task::spawn_blocking(|| {
            let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" }));
            cmd.arg("drain");
            no_console(&mut cmd);
            cmd.status()
        })
        .await;
    }
    let _ = std::fs::write(pending_restore_path(), "");
    stop_running_daemon().await;
    ensure_daemon().await?;
    // init-pack이 blocking Command::status()라 blocking 풀에서 실행(위 drain 패턴과 일치).
    let app2 = app.clone();
    let _ = tokio::task::spawn_blocking(move || maybe_apply_pending_update(&app2)).await;
    Ok(())
}

/// [F5] drain --verify JSON 부재 시 실패 원인 분류 — ①구버전 미지원(clap unknown-flag) vs ②크래시/하드캡
/// 백스톱을 구분해 UI가 정직한 문구를 고르게 한다(거동=plain drain 폴백은 양쪽 동일, 문구만 다름).
/// 반환 Err 문자열 접두: "unsupported:"(①) / "verify_failed:"(②).
/// - ① 판정: clap은 미지의 인자에 exit 2 + stderr에 "unexpected argument"/usage를 낸다(run_drain_verify는
///   정상=0/1·백스톱=3만 내므로 exit 2는 clap 파싱 에러=미지원의 강신호).
/// - ② 그 외(백스톱 exit 3·시그널 사망·부분 stdout 등): 실행은 됐으나 결과를 못 냄 = 검증 실패(원인 미상).
fn classify_drain_verify_failure(exit_code: Option<i32>, stderr: &str) -> String {
    let unsupported = exit_code == Some(2)
        || stderr.contains("unexpected argument")
        || stderr.contains("unrecognized")
        || (stderr.contains("Usage:") && stderr.contains("--verify"));
    if unsupported {
        format!(
            "unsupported: cys drain --verify 미지원(구버전 바이너리) (stderr: {})",
            stderr.trim()
        )
    } else {
        format!(
            "verify_failed: drain --verify 실행 실패(exit={exit_code:?}, 크래시/하드캡 백스톱 가능) (stderr: {})",
            stderr.trim()
        )
    }
}

/// GUI verified 재시작용 — `cys drain --verify`를 실행해 노드별 체크포인트(SESSION_STATE) 저장 검증
/// 결과 JSON을 반환한다(all_saved·summary·nodes·pending_loss_warning). JSON 부재 시 [F5] 원인을 분류해
/// Err("unsupported:…"=구버전 미지원 / "verify_failed:…"=크래시·하드캡)로 신호 → UI가 문구를 분기하고
/// 양쪽 모두 plain drain 폴백(거동 동일). ★재시작하지 않는다(저장 검증만) — 재시작은 UI가 결과를 보고
/// rotate_daemon(skip_drain=true)로 진행.
#[tauri::command]
async fn drain_verify(timeout: u64) -> Result<Value, String> {
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd =
            std::process::Command::new(resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" }));
        cmd.arg("drain")
            .arg("--verify")
            .arg("--timeout")
            .arg(timeout.to_string());
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    // 결과 JSON은 exit code(전원 saved=0/아니면 1)와 무관하게 stdout에 방출된다 — JSON 파싱 성공이 진실원천.
    let stdout = String::from_utf8_lossy(&out.stdout);
    if let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) {
        return Ok(v);
    }
    // JSON 부재 = 구 바이너리 미지원(clap 에러) 또는 크래시/하드캡 → [F5] 원인 분류해 정직한 폴백 신호.
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(classify_drain_verify_failure(out.status.code(), &stderr))
}

/// P5: 무중단 팩 업데이트 UI 브리지(DESIGN-noshutdown-pack-update §2-②·§7-③/④).
/// UI "업데이트 버튼"이 호출 → `cys pack-update`(P4) 사이드카를 실행해 서명검증→디스크 반영→
/// 살아있는 노드 reinject를 시킨다. ★`app.restart()`를 **절대 호출하지 않는다** — cysd·cys-app·
/// 세션이 단 한 번도 죽지 않는 게 install_update(재시작)와의 핵심 차이(무중단).
/// 오케스트레이션은 cys(Rust)에 있고 cys CLI엔 AppHandle이 없으므로, **이 command가 사이드카를
/// 래핑**해(make_ticket/run_skill 패턴 동형) 성공 종료 후 자신이 `app.emit("pack-updated", …)`
/// 한다 — 프런트가 read_board_catalog 등 캐시 의존 호출을 재실행해 stale 캐시를 갱신(§2-② UI 브리지).
/// 인자: from(로컬 디렉터리) 우선, 없으면 manifest_url(원격) — cys pack-update의 --from/--manifest-url에 전달.
#[tauri::command]
async fn install_pack_update(
    app: AppHandle,
    manifest_url: Option<String>,
    from: Option<String>,
) -> Result<String, String> {
    let _ = app.emit("pack-progress", json!({"phase": "start"}));
    let cys = resolve_sidecar(if cfg!(windows) { "cys.exe" } else { "cys" });
    let mut cmd = std::process::Command::new(&cys);
    cmd.arg("pack-update");
    no_console(&mut cmd);
    match (&from, &manifest_url) {
        (Some(d), _) => {
            cmd.args(["--from", d]);
        }
        (None, Some(u)) => {
            cmd.args(["--manifest-url", u]);
        }
        (None, None) => return Err("from 또는 manifest_url 인자 필요".into()),
    }
    // 네트워크·디스크 작업이 길 수 있어 blocking 풀에서 실행(tokio 워커 점유 방지 — install_update drain 패턴).
    let out = tokio::task::spawn_blocking(move || cmd.output())
        .await
        .map_err(|e| format!("pack-update join 실패: {e}"))?
        .map_err(|e| format!("cys pack-update 실행 실패 ({}): {e}", cys.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    // 종료코드 구분: EXIT_REINJECT_DEGRADED = 디스크 팩은 반영됐으나 라이브 노드 reinject 실패
    // (일부 미각성) — 디스크는 성공이므로 pack-updated를 emit하되 update-warning을 함께 띄운다.
    // 그 외 비0 = 실제 실패(디스크 미반영) → 구 캐시 유지가 안전하므로 update-error만.
    let degraded = out.status.code() == Some(cys::pack::EXIT_REINJECT_DEGRADED);
    if !out.status.success() && !degraded {
        // ★실패 — "pack-updated"는 emit하지 않는다(구 캐시 유지가 stale 갱신보다 안전). update-error만.
        let _ = app.emit(
            "update-error",
            json!({"phase": "pack-update", "message": stderr.clone()}),
        );
        return Err(format!("pack-update 실패: {stderr}"));
    }
    // ★디스크 반영 성공(success 또는 degraded) — .pack-version을 읽어 새 팩 버전으로 브로드캐스트(§2-②/§7-③).
    //   read_board_catalog가 pack_dir의 정적 파일을 읽는 것과 동일 SOT(pack_dir).
    let pack_version = std::fs::read_to_string(cys::pack::pack_dir().join(".pack-version"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    // 사이드카 구조화 출력에서 reinject failed/deferred 집계 — 라이브 미각성을 사용자에게 경고.
    let (failed, deferred) = parse_reinject_counts(&stdout);
    if failed > 0 || deferred > 0 {
        // ★성공으로만 포장 금지 — 디스크는 갱신됐으나 라이브 노드 일부 미각성/보류를 경고한다.
        //   (app.restart는 여전히 미호출 — 무중단 불변식 유지.)
        let _ = app.emit(
            "update-warning",
            json!({
                "phase": "pack-update",
                "pack_version": pack_version,
                "reinject_failed": failed,
                "reinject_deferred": deferred,
                "message": format!(
                    "디스크 팩은 {pack_version} 로 갱신됐으나 reinject {failed} 실패·{deferred} 보류 — \
                     일부 노드 미각성(라이브 무중단 유지, 재시작 안 함). 다음 pack-update에서 재시도됩니다."
                ),
            }),
        );
    }
    let _ = app.emit(
        "pack-updated",
        json!({
            "pack_version": pack_version,
            "reinject_failed": failed,
            "reinject_deferred": deferred,
        }),
    );
    Ok(pack_version)
}

/// 사이드카(cys pack-update) stdout에서 `PACK_UPDATE_RESULT … failed=N deferred=N` 토큰을 파싱해
/// (failed, deferred)를 돌려준다. 토큰 부재(구버전 사이드카·reinject 스킵 등)면 (0,0) — 보수적.
/// 사람용 메시지와 독립한 안정 토큰(REINJECT_RESULT_PREFIX)만 신뢰한다.
fn parse_reinject_counts(stdout: &str) -> (u64, u64) {
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(cys::pack::REINJECT_RESULT_PREFIX) {
            let (mut failed, mut deferred) = (0u64, 0u64);
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("failed=") {
                    failed = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("deferred=") {
                    deferred = v.parse().unwrap_or(0);
                }
            }
            return (failed, deferred);
        }
    }
    (0, 0)
}

/// `ledger.list` 응답에서 scoped 프로세스 pid만 추린다.
/// windows 핸드오프(taskkill /F=TerminateProcess)는 데몬의 콘솔 이벤트 핸들러를
/// 못 깨워 shutdown_cleanup이 실행되지 않으므로, 데몬이 살아있는 동안 UI가
/// 직접 이 pid들을 ledger.kill로 회수해야 한다 (cysd shutdown_cleanup와 동일 선별).
/// (호출은 windows 경로 한정 — non-windows 빌드에선 테스트만 사용한다.)
#[cfg_attr(not(windows), allow(dead_code))]
fn scoped_pids_from_ledger_list(resp: &Value) -> Vec<u64> {
    resp["entries"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|e| e["scoped"].as_bool().unwrap_or(false))
                .filter_map(|e| e["pid"].as_u64())
                .collect()
        })
        .unwrap_or_default()
}

/// 구 데몬 정상 종료: system.identify로 pid를 받아 SIGTERM(unix)/taskkill(win).
async fn stop_running_daemon() {
    let pid = rpc("system.identify", json!({}))
        .await
        .ok()
        .and_then(|r| r["daemon_pid"].as_u64());
    if let Some(pid) = pid {
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        #[cfg(windows)]
        {
            // taskkill /F는 TerminateProcess라 데몬이 어떤 콘솔 이벤트도 못 받아
            // shutdown_cleanup이 실행되지 않는다 → ledger의 scoped 프로세스(=cys CLI의
            // 자식, 데몬 트리 밖이라 /T로도 닿지 않음)가 영구 고아로 남는다. 데몬이
            // 아직 살아있는 지금 직접 회수한 뒤 데몬을 종료한다 (unix SIGTERM 경로 대칭).
            if let Ok(r) = rpc("ledger.list", json!({})).await {
                for spid in scoped_pids_from_ledger_list(&r) {
                    let _ = rpc("ledger.kill", json!({ "pid": spid })).await;
                }
            }
            let mut kill = std::process::Command::new("taskkill");
            kill.args(["/PID", &pid.to_string(), "/F"]);
            no_console(&mut kill);
            let _ = kill.output();
        }
        // 종료·소켓 unlink 대기 (최대 3초)
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if connect().await.is_err() {
                break;
            }
        }
    }
}

fn main() {
    tauri::Builder::default()
        // ★최선두 등록 필수 — 두 번째 인스턴스는 다른 플러그인·setup이 돌기 전에 기존 창 포커스 후
        // 스스로 종료된다(Win11 cys-app.exe 프로세스 증식 이슈의 증상 차단 · 2026-07-12). 스폰 소스가
        // 무엇이든(설치기 재실행·바로가기 이중클릭·OS 재기동 복원) 단일 인스턴스가 보장된다.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .manage(Attachments(Mutex::new(HashMap::new())))
        .invoke_handler(tauri::generate_handler![
            dept_tombstone_by_socket,
            dept_tombstones,
            start_master,
            start_dept_master,
            ceo_pending,
            promote_pending_ceo,
            approve_ceo_promotion,
            daemon_status,
            list_surfaces,
            org_status,
            org_fleet,
            ensure_dept_forwarders,
            control_analytics,
            control_skills,
            control_cost_baseline,
            control_alerts,
            control_weekly,
            control_sessions,
            control_session_detail,
            control_session_star,
            control_dashboard,
            control_hw,
            learn_status,
            create_surface,
            send_input,
            save_pasted_image,
            log_ime,
            ime_debug_enabled,
            rename_surface,
            resize_surface,
            close_surface,
            attach_surface,
            start_surface_stream,
            feed_list,
            feed_reply,
            list_dir,
            open_path,
            reveal_path,
            read_text_head,
            home_dir_path,
            open_url,
            send_key,
            read_board_catalog,
            make_ticket,
            run_skill,
            skill_runs,
            resource_gate_check,
            usage_accounts_all,
            skill_out_dir,
            check_update,
            check_pack_update,
            live_session_count,
            install_update,
            autotest_patch_install,
            rotate_daemon,
            drain_verify,
            install_pack_update,
            launch_dept_daemon,
            allocate_dept_daemon,
            stop_dept_daemon,
            stop_dept_daemon_by_socket,
            purge_dept_daemon_by_socket,
            dept_purge_preview_by_socket,
            rotate_dept_daemon,
            list_depts,
            read_dept_catalog,
            install_cli_to_path,
            app_version,
            boot_verdict,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // ★T2 안전모드 게이트(translocation/비정규 경로 · 앱 자기삭제·"손상됨" 근본수리) —
                // 데몬 기동·launchd 등록·팩/hook 쓰기 등 **자기경로 부수효과 전체보다 먼저** 실행 번들
                // 위치를 판정한다. Canonical(정규 설치)이 아니면 부수효과를 전부 skip 하고 안내만 표시한
                // 뒤 조기 반환한다(자동 이동 없음 — 오탐 시 파괴 위험 회피, 안내 폴백이 항상 성립).
                // Canonical 이면 아래 기존 부트 흐름을 그대로 통과한다(정상 부트 무영향). 기존 설치본이
                // 실행 중이면 single-instance 플러그인이 새 인스턴스를 접고, 그와 별개로 이 게이트가
                // 비정규 인스턴스의 데몬 스폰 자체를 막는다(방어심층).
                #[cfg(target_os = "macos")]
                {
                    let verdict = current_boot_verdict();
                    if verdict != BootPathVerdict::Canonical {
                        let msg = translocation_guidance(verdict);
                        eprintln!(
                            "[cys-app] 안전모드: 비정규 실행 위치({verdict:?}) — 데몬·launchd·팩 등록 skip\n{msg}"
                        );
                        let _ = handle.emit("translocation-blocked", msg);
                        return;
                    }
                }
                // ★온보딩 게이트(v4) — GUI 전용 완료 마커(.gui-onboarded) 기준. 팩 마커(.pack-version)
                // 기준이던 v3는 CLI autostart·잔존 schtasks 등으로 cysd가 GUI보다 먼저 돈 머신에서
                // 게이트가 선점돼 ~/.claude hook이 영구 미설치됐다(0.12.52 cys-neo 실사고 — "너는
                // 마스터다" 부트스트랩 무력화). 이 마커는 GUI 온보딩 성공 경로만 기록하므로 프로세스
                // 순서와 무관하게 신선 머신 온보딩이 보장된다. 평시 부트 비용 = 마커 read 1회.
                #[allow(unused_variables)] // 온보딩 경로가 없는 OS(linux CI 등)에서만 미사용
                let needs_onboard = needs_gui_onboard(
                    std::fs::read_to_string(gui_onboarded_path()).ok().as_deref(),
                    env!("CARGO_PKG_VERSION"),
                );
                #[cfg(target_os = "macos")]
                let launchd_owns = maybe_autoregister_launchd().await;
                #[cfg(not(target_os = "macos"))]
                let launchd_owns = false;
                // ★신선 머신 부트 수리(오너 2026-07-15 — "daemon: connecting…" 영구 고착 실사고):
                // 종전에는 launchd 소유 시 5초 무응답이면 부트 시퀀스 전체를 영구 포기했다(재시도·
                // 폴백 전무 — 온보딩·이벤트 파이프까지 미실행). 최신 macOS는 앱이 등록한 LaunchAgent를
                // '백그라운드 항목' 사용자 승인까지 보류할 수 있고, 첫 실행 Gatekeeper 검증은 5초를
                // 훌쩍 넘긴다. 수리: ①launchd 5초 무응답 → 형제 spawn 폴백(CLI cys와 대칭 — 중복
                // spawn은 cysd 시동 잠금(healthy-holder 거부)이 단일 인스턴스 보장) ②그래도 실패면
                // 15초 간격 백그라운드 재시도(최대 20회 ≈ 5분 — 승인 지연·느린 첫 기동 흡수)
                // ③4회째부터 로그인 항목 안내 이벤트(daemon-retry-hint) — 생초보 가이드.
                let mut result = if launchd_owns {
                    if wait_for_connect(50).await {
                        Ok(())
                    } else {
                        eprintln!("[cys-app] launchd-owned cysd not ready in 5s — 형제 spawn 폴백");
                        ensure_daemon().await
                    }
                } else {
                    ensure_daemon().await
                };
                if result.is_err() {
                    for attempt in 1..=20u32 {
                        let _ = handle.emit(
                            "daemon-error",
                            format!("데몬 대기 중 — 재시도 {attempt}/20 (15초 간격)"),
                        );
                        if attempt == 4 {
                            let _ = handle.emit("daemon-retry-hint", ());
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                        if ensure_daemon().await.is_ok() {
                            result = Ok(());
                            break;
                        }
                    }
                }
                if let Err(e) = result {
                    let _ = handle.emit(
                        "daemon-error",
                        format!("{e} — 데몬을 시작하지 못했습니다. 시스템 설정 → 일반 → 로그인 항목에서 cys 백그라운드 항목을 허용한 뒤 앱을 다시 여세요."),
                    );
                    return;
                }
                let _ = handle.emit("daemon-ready", ());
                // event-forwarder를 먼저 띄워 init-pack 블로킹이 양방향 이벤트 파이프를 막지 않게 한다(반쪽 부팅 방지).
                spawn_event_forwarder(handle.clone(), default_socket());
                // RC-1: Windows 첫 기동 온보딩(팩+hook+autostart schtasks). 멱등.
                // 게이트(needs_onboard·위 캡처): 마커 부재(신선·직전 실패)·버전 불일치에만 실행 —
                // 평시 부트의 사이드카 스폰+전량 스윕+schtasks 재등록 비용 제거(Win11 이슈 실측).
                // 마커는 온보딩 **성공** 시에만 기록 — hook 등록 실패(init-pack rc=1)도 재시도로 수렴.
                // hook만 사후 유실된 상태(마커 무결)의 치유는 doctor --fix·버전 전이가 담당.
                #[cfg(windows)]
                if needs_onboard && maybe_windows_onboard() {
                    if let Err(e) = std::fs::write(gui_onboarded_path(), env!("CARGO_PKG_VERSION")) {
                        eprintln!("[cys-app] onboarding marker write failed (다음 부트 재시도): {e}");
                    }
                }
                // RC-17(T5): macOS 첫 기동 온보딩(팩+hook) — Windows 대칭(동일 게이트). autostart는 위 launchd.
                #[cfg(target_os = "macos")]
                if needs_onboard && maybe_macos_onboard() {
                    if let Err(e) = std::fs::write(gui_onboarded_path(), env!("CARGO_PKG_VERSION")) {
                        eprintln!("[cys-app] onboarding marker write failed (다음 부트 재시도): {e}");
                    }
                }
                // 업데이트 재시작 시: 새 팩(새 기능) 반영 + 노드 자동복귀(마커가 있을 때만).
                maybe_apply_pending_update(&handle);
                // ★TCC 처방(오너 2026-07-15): 폴더 권한 선제 트리거·거부 감지 안내.
                #[cfg(target_os = "macos")]
                nudge_folder_permissions(&handle);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running aiterm");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [F1] open_path 실행형 게이트 — 실행비트 파일은 force 없이 executable_confirm으로 거절(fail-closed),
    /// 비존재 경로는 metadata 게이트에서 거절(스폰 없음). force 경로는 실제 스폰이라 여기서 검사하지 않는다.
    #[test]
    fn open_path_gates_executable_and_missing() {
        let r = open_path("/definitely/not/a/real/path-xyz".into(), None);
        assert!(r.is_err() && !r.unwrap_err().contains("executable_confirm"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join("cys-openpath-test");
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join("run.sh");
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            let r = open_path(p.to_string_lossy().into_owned(), None);
            assert_eq!(r, Err("executable_confirm".to_string()));
        }
    }

    /// [F5] drain --verify 실패 분류 — 구버전 미지원(clap unknown-flag)과 크래시/하드캡을 구분한다.
    /// UI 문구 정직성: ①→"미지원" ②→"검증 실패(원인 미상)". 거동(plain drain 폴백)은 양쪽 동일.
    #[test]
    fn drain_verify_failure_classification() {
        // ① 구버전: clap exit 2 + unexpected argument → unsupported
        let e1 = classify_drain_verify_failure(
            Some(2),
            "error: unexpected argument '--verify' found\n\nUsage: cys drain [OPTIONS]",
        );
        assert!(e1.starts_with("unsupported:"), "구버전 미지원은 unsupported: {e1}");
        // ① exit 코드 미상이어도 stderr usage+--verify 패턴이면 unsupported
        let e1b = classify_drain_verify_failure(None, "Usage: cys drain --verify ...");
        assert!(e1b.starts_with("unsupported:"), "usage+--verify는 unsupported: {e1b}");
        // ② 하드캡 백스톱(exit 3) → verify_failed
        let e2 = classify_drain_verify_failure(Some(3), "");
        assert!(e2.starts_with("verify_failed:"), "exit3 백스톱은 verify_failed: {e2}");
        // ② 시그널 사망(code=None)·usage 무관 stderr → verify_failed
        let e2b = classify_drain_verify_failure(None, "thread 'main' panicked at ...");
        assert!(e2b.starts_with("verify_failed:"), "크래시는 verify_failed: {e2b}");
        // 정상 exit 1(부분 실패)은 JSON 경로라 여기 안 오지만, 분류가 오면 verify_failed(안전)
        let e2c = classify_drain_verify_failure(Some(1), "");
        assert!(e2c.starts_with("verify_failed:"), "exit1 무JSON은 verify_failed: {e2c}");
    }

    /// A안 회귀 박제(2026-07-11 오너 승인): 교대 게이트는 role/agent 붙은 세션만 지킨다.
    /// 맨 셸 pane(role·agent 없음)이 다시 게이트에 잡히면 기본 pane 1개만으로 자동 교대가
    /// 영영 보류돼 "taskkill 없인 데몬이 안 바뀐다" 실사고가 재발한다 — 그 회귀를 여기서 잡는다.
    #[test]
    fn bare_pane_does_not_block_rotation_but_role_or_agent_does() {
        let bare = json!({"exited": false, "role": null, "agent": null});
        let exited_agent = json!({"exited": true, "role": "worker", "agent": "claude"});
        let roled = json!({"exited": false, "role": "master", "agent": null});
        let agented = json!({"exited": false, "role": null, "agent": "claude"});
        let empty_strings = json!({"exited": false, "role": "", "agent": ""});
        assert!(!session_blocks_rotation(&bare), "맨 pane은 자동 교대를 막지 않는다");
        assert!(!session_blocks_rotation(&exited_agent), "죽은 세션은 세지 않는다");
        assert!(session_blocks_rotation(&roled), "role claim 세션은 보호");
        assert!(session_blocks_rotation(&agented), "agent 세션은 보호");
        assert!(!session_blocks_rotation(&empty_strings), "빈 문자열은 미부착으로 취급");
    }

    /// (T1) 재시작 후 팩반영·복원 발동 판정 — 마커(인앱 업데이트) OR 버전변경(홈페이지 수동설치).
    #[test]
    /// ★v4 GUI 온보딩 게이트 회귀 핀(0.12.52 cys-neo 실사고) — 마커가 현재 버전과 정확히 일치할
    /// 때만 스킵. 부재(신선 머신·직전 실패)·구버전·손상 = 실행(fail-open 치유 방향). 이 판정이
    /// .pack-version 등 팩 상태를 일절 보지 않는 것이 요점 — cysd 선행이 게이트를 선점 못 한다.
    #[test]
    fn needs_gui_onboard_only_skips_on_exact_version_match() {
        assert!(needs_gui_onboard(None, "0.12.53"), "마커 부재 = 온보딩(신선·직전 실패)");
        assert!(!needs_gui_onboard(Some("0.12.53"), "0.12.53"), "정확 일치 = 스킵");
        assert!(!needs_gui_onboard(Some("0.12.53\n"), "0.12.53"), "개행 trim 후 일치 = 스킵");
        assert!(needs_gui_onboard(Some("0.12.52"), "0.12.53"), "구버전 = 온보딩(업그레이드)");
        assert!(needs_gui_onboard(Some("garbage"), "0.12.53"), "손상 = 온보딩(fail-open)");
    }

    #[test]
    fn decide_pending_update_marker_or_version_change() {
        use PendingUpdatePlan::*;
        // 마커 최우선 — 스탬프·prior_state와 무관하게 Apply(구버전이 이 릴리스로 올라올 때 남긴 마커).
        assert_eq!(decide_pending_update(true, None, "0.12.51", false), Apply, "마커=Apply(스탬프 부재)");
        assert_eq!(decide_pending_update(true, Some("0.12.51"), "0.12.51", false), Apply, "마커=Apply(스탬프 동일해도)");
        // ★결함2: 스탬프 부재 × prior_state — 기존 설치 증거로 전환기 홈페이지 설치 vs 진짜 최초 설치를 가른다.
        assert_eq!(decide_pending_update(false, None, "0.12.51", true), Apply, "스탬프 부재+기존설치=전환기 홈페이지설치=Apply");
        assert_eq!(decide_pending_update(false, None, "0.12.51", false), RecordStampOnly, "스탬프 부재+기존설치 없음=진짜 최초설치");
        // 버전변경/동일은 prior_state와 무관(회귀 핀 — Some(stamp)이면 prior_state를 보지 않는다).
        assert_eq!(decide_pending_update(false, Some("0.12.50"), "0.12.51", false), Apply, "버전변경(홈페이지 수동설치)=Apply");
        assert_eq!(decide_pending_update(false, Some("0.12.50"), "0.12.51", true), Apply, "버전변경=Apply(prior_state 무관)");
        assert_eq!(decide_pending_update(false, Some("0.12.51"), "0.12.51", false), Skip, "동일 버전·마커 없음=Skip");
        assert_eq!(decide_pending_update(false, Some("0.12.51"), "0.12.51", true), Skip, "동일 버전=Skip(prior_state 무관)");
    }

    // HUD-2: open_url 화이트리스트 — https·허용 도메인만 통과, 위장 host(userinfo/서브도메인 사칭) 차단.
    #[test]
    fn open_url_whitelist_blocks_spoofed_and_nonhttps() {
        assert!(url_host_allowed("https://notebooklm.google.com/notebook/abc").is_ok());
        assert!(url_host_allowed("https://github.com/cys/repo").is_ok());
        assert!(url_host_allowed("https://www.cysinsight.com/").is_ok(), "홈페이지(본체 다운로드) 허용");
        assert!(url_host_allowed("https://cysinsight.com/download").is_ok(), "홈페이지 apex 허용");
        assert!(url_host_allowed("http://notebooklm.google.com/").is_err(), "http 차단");
        assert!(url_host_allowed("https://evil.com/notebooklm.google.com").is_err(), "경로 사칭 차단");
        assert!(url_host_allowed("https://notebooklm.google.com.evil.com/").is_err(), "서브도메인 사칭 차단");
        assert!(url_host_allowed("https://notebooklm.google.com@evil.example.com/").is_err(), "userinfo 사칭 차단");
        assert!(url_host_allowed("https://evil.com#.github.com/").is_err(), "fragment 사칭 차단");
        assert!(url_host_allowed("https://evil.com?.github.com").is_err(), "query 사칭 차단");
        assert!(url_host_allowed("https://evil.com?x=.github.com").is_err(), "query 파라미터 사칭 차단");
    }

    // 사용자 확장 allowlist(순수 판정) — 정확일치·서브도메인 허용, 사칭·빈 항목 차단.
    #[test]
    fn host_allowlist_user_extension() {
        let extras = vec!["example-inst.org".to_string()];
        assert!(host_in_allowlist("example-inst.org", &extras));
        assert!(host_in_allowlist("docs.example-inst.org", &extras), "확장 도메인 서브도메인 허용");
        assert!(!host_in_allowlist("example-inst.org.evil.com", &extras), "사칭 차단");
        assert!(!host_in_allowlist("evil.com", &extras));
        assert!(!host_in_allowlist("anything.com", &vec!["".to_string()]), "빈 확장 항목 무시");
    }

    // #3: 사이드카 stdout의 PACK_UPDATE_RESULT 토큰에서 reinject failed/deferred를 파싱해
    // update-warning 발화 판단에 쓴다. 토큰 부재(구버전·reinject 스킵)는 (0,0)으로 보수적 처리.
    #[test]
    fn parse_reinject_counts_reads_structured_token() {
        let out = "[pack-update] 팩 2.0.0 반영 완료 (3 written, 1 preserved). 노드 reinject 점검…\n\
                   [pack-update] reinject: 2 injected, 1 skipped, 3 deferred, 4 failed.\n\
                   PACK_UPDATE_RESULT pack_version=2.0.0 injected=2 skipped=1 deferred=3 failed=4\n";
        assert_eq!(parse_reinject_counts(out), (4, 3), "failed=4 deferred=3 파싱");

        // 토큰 부재 → (0,0) 보수적(경고 미발화).
        assert_eq!(parse_reinject_counts("아무 의미 없는 출력\n"), (0, 0));
        assert_eq!(parse_reinject_counts(""), (0, 0));

        // failed=0 deferred=0 → (0,0)(완전 성공, 경고 없음).
        assert_eq!(
            parse_reinject_counts("PACK_UPDATE_RESULT pack_version=2.0.0 injected=5 skipped=0 deferred=0 failed=0"),
            (0, 0)
        );
        // deferred만 있는 경우(busy 노드) — 경고 발화 대상.
        assert_eq!(
            parse_reinject_counts("PACK_UPDATE_RESULT pack_version=1.2.3 injected=0 skipped=0 deferred=2 failed=0"),
            (0, 2)
        );
    }

    // check_pack_update 호환 게이트(DESIGN §7-④ 축2): min_binary_version > 실행 바이너리 = 무중단 거부.
    #[test]
    fn pack_binary_too_old_gate() {
        // 빈 값 = 제약 없음 → 무중단 허용.
        assert!(!pack_binary_too_old("", "0.4.2"));
        assert!(!pack_binary_too_old("   ", "0.4.2"));
        // min ≤ running → 허용.
        assert!(!pack_binary_too_old("0.4.2", "0.4.2"), "동일 버전 허용");
        assert!(!pack_binary_too_old("0.4.1", "0.4.2"), "min < running 허용");
        // min > running → 거부(바이너리 경로).
        assert!(pack_binary_too_old("0.5.0", "0.4.2"), "min > running 거부");
        assert!(pack_binary_too_old("1.0.0", "0.4.2"));
        // 파싱 실패 = 거부(보수적).
        assert!(pack_binary_too_old("not-a-version", "0.4.2"));
        assert!(pack_binary_too_old("0.5.0", "garbage"));
    }

    // 회귀: windows 업데이트 핸드오프가 데몬을 taskkill /F로 하드킬하면 cysd의
    // shutdown_cleanup이 실행되지 않아 scoped 자식(cys CLI의 자식)이 영구 고아로
    // 남는다. 그 누수를 막으려면 데몬이 살아있을 때 UI가 ledger.list에서 scoped pid를
    // 정확히 추려 ledger.kill로 회수해야 한다 — 그 선별 로직을 고정한다.
    #[test]
    fn scoped_pids_from_ledger_list_picks_only_scoped_pids() {
        let resp = json!({
            "entries": [
                {"pid": 100, "scoped": true},
                {"pid": 200, "scoped": false}, // 비-scoped → 데몬이 생명주기 보장 안 함, 회수 제외
                {"pid": 300, "scoped": true},
            ]
        });
        let mut pids = scoped_pids_from_ledger_list(&resp);
        pids.sort_unstable();
        assert_eq!(
            pids,
            vec![100, 300],
            "scoped 항목만 회수 대상이어야 하고 비-scoped는 제외돼야 한다"
        );
    }

    // scoped 플래그가 없으면(기본값 누락) 보수적으로 회수 대상에서 빼 외부 프로세스
    // 오인 킬을 막는다. entries가 비었거나 누락돼도 패닉 없이 빈 목록을 돌려준다.
    #[test]
    fn scoped_pids_from_ledger_list_empty_and_missing_fields_are_safe() {
        assert!(scoped_pids_from_ledger_list(&json!({"entries": []})).is_empty());
        assert!(scoped_pids_from_ledger_list(&json!({})).is_empty());
        // scoped 키 누락 = false 취급, pid 누락 항목은 건너뛴다
        let resp = json!({"entries": [{"pid": 100}, {"scoped": true}]});
        assert!(scoped_pids_from_ledger_list(&resp).is_empty());
    }

    // 적대검증 R-1 회귀: org_fleet fan-out은 풀 비경유 rpc_oneshot을 쓴다. (a) 정상 소켓은 응답을
    // 파싱해 반환하고, (b) 무응답(hung) 소켓은 timeout으로 깨끗이 Err를 준다 — 일회성 연결이라
    // 취소가 공유 풀(conn_cell)을 오염시키지 않는다(같은 부서 send_key/org_status 응답 귀속 보호).
    #[cfg(unix)]
    #[test]
    fn rpc_oneshot_parses_response_and_times_out_on_hung_socket() {
        use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
        use tokio::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("cys-rpc-oneshot-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let ok_sock = dir.join("ok.sock");
        let hang_sock = dir.join("hang.sock");
        let _ = std::fs::remove_file(&ok_sock);
        let _ = std::fs::remove_file(&hang_sock);

        tauri::async_runtime::block_on(async {
            // (a) 응답 소켓 — 요청 1줄 소비 후 valid 프레임 반환
            let ok = UnixListener::bind(&ok_sock).unwrap();
            tauri::async_runtime::spawn(async move {
                if let Ok((mut s, _)) = ok.accept().await {
                    let (r, mut w) = s.split();
                    let mut br = BufReader::new(r);
                    let mut l = String::new();
                    let _ = br.read_line(&mut l).await;
                    let _ = w.write_all(b"{\"ok\":true,\"result\":{\"surfaces\":[]}}\n").await;
                    let _ = w.flush().await;
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            });
            // (b) hung 소켓 — accept만 하고 응답 없이 hold
            let hang = UnixListener::bind(&hang_sock).unwrap();
            tauri::async_runtime::spawn(async move {
                if let Ok((_s, _)) = hang.accept().await {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            });
            tokio::time::sleep(std::time::Duration::from_millis(50)).await; // bind 안정화

            // (a) 정상 응답 파싱
            let ok_res = rpc_oneshot(&ok_sock, "org.status", json!({})).await;
            assert!(ok_res.is_ok(), "정상 소켓 응답을 파싱해야 한다: {ok_res:?}");
            assert!(ok_res.unwrap()["surfaces"].is_array());

            // (b) hung 소켓은 timeout으로 Err — 취소가 깨끗이 일어난다(풀 비경유)
            let hung = tokio::time::timeout(
                std::time::Duration::from_millis(300),
                rpc_oneshot(&hang_sock, "org.status", json!({})),
            )
            .await;
            assert!(hung.is_err(), "무응답 소켓은 timeout(Elapsed)이어야 한다");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── CLI PATH 설치 헬퍼 ──────────────────────────────────────────
    #[test]
    fn sh_squote_escapes_spaces_and_quotes() {
        assert_eq!(sh_squote("/usr/local/bin"), "'/usr/local/bin'");
        assert_eq!(sh_squote("/Users/x/a b/cys.app"), "'/Users/x/a b/cys.app'");
        // 단일따옴표는 '\'' 시퀀스로 안전 이스케이프
        assert_eq!(sh_squote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn build_install_script_emits_idempotent_symlinks() {
        let cys = std::path::Path::new("/Applications/cys.app/Contents/MacOS/cys");
        let cysd = std::path::Path::new("/Applications/cys.app/Contents/MacOS/cysd");
        let s = build_install_script(cys, cysd, "/usr/local/bin");
        assert_eq!(
            s,
            "mkdir -p '/usr/local/bin' && \
ln -sf '/Applications/cys.app/Contents/MacOS/cys' '/usr/local/bin/cys' && \
ln -sf '/Applications/cys.app/Contents/MacOS/cysd' '/usr/local/bin/cysd'"
        );
    }

    #[test]
    fn classify_bundle_dir_distinguishes_canonical_translocated_backup_nonstandard() {
        use std::path::Path;
        assert_eq!(
            classify_bundle_dir(Path::new("/Applications/cys.app/Contents/MacOS")),
            BundleKind::Canonical
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Users/x/Applications/cys.app/Contents/MacOS")),
            BundleKind::Canonical
        );
        assert_eq!(
            classify_bundle_dir(Path::new(
                "/private/var/folders/aa/bb/AppTranslocation/CCCC/d/cys.app/Contents/MacOS"
            )),
            BundleKind::Translocated
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Applications/cys.app.bak-044/Contents/MacOS")),
            BundleKind::Backup
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Applications/cys.app.prev-210050/Contents/MacOS")),
            BundleKind::Backup
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Users/x/Downloads/cys.app/Contents/MacOS")),
            BundleKind::NonStandard
        );
    }

    #[test]
    fn classify_bundle_dir_volumes_applications_is_not_canonical() {
        use std::path::Path;
        // ★/Volumes 가드(reviewer1): DMG·외장 마운트 안의 Applications 폴더/심링크 경유 실행은
        // ends_with("/Applications")를 만족해도 Canonical 이 아니다(언마운트 시 죽은 경로 → 자기삭제 재발).
        assert_ne!(
            classify_bundle_dir(Path::new("/Volumes/cys 0.12.91/Applications/cys.app/Contents/MacOS")),
            BundleKind::Canonical,
            "/Volumes 하위 Applications 는 Canonical 오판 금지",
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Volumes/cys 0.12.91/Applications/cys.app/Contents/MacOS")),
            BundleKind::NonStandard,
        );
        // 정규 경로 불변(회귀 핀).
        assert_eq!(
            classify_bundle_dir(Path::new("/Applications/cys.app/Contents/MacOS")),
            BundleKind::Canonical,
            "/Applications 는 Canonical 불변",
        );
        assert_eq!(
            classify_bundle_dir(Path::new("/Users/x/Applications/cys.app/Contents/MacOS")),
            BundleKind::Canonical,
            "~/Applications 는 Canonical 불변",
        );
        // boot_path_verdict 도 델리게이션 결과로 안전모드 진입(비-Canonical=NonCanonical).
        assert_eq!(
            boot_path_verdict(
                Path::new("/Volumes/cys 0.12.91/Applications/cys.app/Contents/MacOS/cys-app"),
                false,
            ),
            BootPathVerdict::NonCanonical,
            "/Volumes 하위 Applications 는 부트 안전모드 진입",
        );
    }

    #[test]
    fn parse_which_a_returns_precedence_ordered_paths() {
        let out = "/Users/x/.local/bin/cys\n/opt/homebrew/bin/cys\n\n/usr/local/bin/cys\n";
        assert_eq!(
            parse_which_a(out),
            vec![
                "/Users/x/.local/bin/cys".to_string(),
                "/opt/homebrew/bin/cys".to_string(),
                "/usr/local/bin/cys".to_string(),
            ]
        );
    }

    #[test]
    fn plan_cli_install_refuses_translocated_and_backup() {
        // translocated → Err
        assert!(plan_cli_install(
            std::path::Path::new("/private/var/folders/x/AppTranslocation/Y/d/cys.app/Contents/MacOS"),
            "/usr/local/bin"
        ).is_err());
        // backup → Err
        assert!(plan_cli_install(
            std::path::Path::new("/Applications/cys.app.bak-044/Contents/MacOS"),
            "/usr/local/bin"
        ).is_err());
    }

    #[test]
    fn autoregister_allowed_only_canonical() {
        // 무음 launchd 자동등록은 plan_cli_install 보다 엄격하다(의도적 divergence): Canonical 만 허용.
        // NonStandard(~/Downloads·/Volumes 등)도 거부 — 휘발/이동 경로가 plist 에 각인되면 언마운트·삭제
        // 시 죽은 경로 데몬 무한 스폰(리뷰어1 F1). 비-Canonical 은 ensure_daemon 런타임 폴백으로 안전.
        assert!(autoregister_allowed(&BundleKind::Canonical), "정규 번들(/Applications·~/Applications)만 자동등록 허용");
        assert!(!autoregister_allowed(&BundleKind::Translocated), "임시 경로는 자동등록 거부");
        assert!(!autoregister_allowed(&BundleKind::Backup), "백업 번들은 자동등록 거부");
        assert!(!autoregister_allowed(&BundleKind::NonStandard), "비표준(Downloads·USB 등)도 자동등록 거부");
    }

    // ── T4: 부트 안전모드 감지 게이트 ─────────────────────────────────────
    #[test]
    fn boot_path_verdict_positive_and_negative_cases() {
        use std::path::Path;
        // 양성 3케이스(비-Canonical=안전모드 진입) — escape env 없음(false).
        assert_eq!(
            boot_path_verdict(
                Path::new("/private/var/folders/ab/AppTranslocation/CD12/d/cys.app/Contents/MacOS/cys-app"),
                false,
            ),
            BootPathVerdict::Translocated,
            "AppTranslocation 임시 경로 = Translocated",
        );
        assert_eq!(
            boot_path_verdict(
                Path::new("/Volumes/cys 0.12.91/cys.app/Contents/MacOS/cys-app"),
                false,
            ),
            BootPathVerdict::NonCanonical,
            "DMG(/Volumes) 직실행 = NonCanonical",
        );
        assert_eq!(
            boot_path_verdict(
                Path::new("/Users/x/Downloads/cys.app/Contents/MacOS/cys-app"),
                false,
            ),
            BootPathVerdict::NonCanonical,
            "임의(Downloads) 경로 = NonCanonical",
        );
        // 음성 2케이스(Canonical=정상 부트 그대로).
        assert_eq!(
            boot_path_verdict(
                Path::new("/Applications/cys.app/Contents/MacOS/cys-app"),
                false,
            ),
            BootPathVerdict::Canonical,
            "/Applications 정규 설치 = Canonical",
        );
        assert_eq!(
            boot_path_verdict(
                Path::new("/Users/x/dev/cys/target/release/cys-app"),
                true,
            ),
            BootPathVerdict::Canonical,
            "CYS_ALLOW_NONCANONICAL=1(escape env) = 무조건 Canonical(개발·CI 자기감금 방지)",
        );
    }

    #[test]
    fn boot_path_verdict_escape_overrides_and_user_applications() {
        use std::path::Path;
        // ~/Applications 도 Canonical(정규 allowlist).
        assert_eq!(
            boot_path_verdict(
                Path::new("/Users/x/Applications/cys.app/Contents/MacOS/cys-app"),
                false,
            ),
            BootPathVerdict::Canonical,
        );
        // escape env 는 translocation 경로마저 Canonical 로 덮는다(무조건 = 최우선 단락).
        assert_eq!(
            boot_path_verdict(
                Path::new("/private/var/folders/ab/AppTranslocation/x/cys.app/Contents/MacOS/cys-app"),
                true,
            ),
            BootPathVerdict::Canonical,
        );
        // escape 없는 개발 target/ 는 NonCanonical(하네스가 env 로 스스로 풀어야 함).
        assert_eq!(
            boot_path_verdict(
                Path::new("/Users/x/dev/cys/target/debug/cys-app"),
                false,
            ),
            BootPathVerdict::NonCanonical,
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn translocation_guidance_carries_recovery_steps() {
        // 안내는 ①Applications 드래그 ②구버전 종료·교체 ③xattr quarantine 제거를 모두 담아야 한다.
        let g = translocation_guidance(BootPathVerdict::Translocated);
        assert!(g.contains("Applications"), "① Applications 드래그 설치 안내 포함");
        assert!(g.contains("구버전") && g.contains("종료"), "② 구버전 종료·교체 안내 포함");
        assert!(
            g.contains("xattr -d com.apple.quarantine /Applications/cys.app"),
            "③ quarantine 제거 명령 포함",
        );
        // NonCanonical 도 동일 복구 절차를 안내한다(원인 문구만 일반화).
        let n = translocation_guidance(BootPathVerdict::NonCanonical);
        assert!(n.contains("xattr -d com.apple.quarantine /Applications/cys.app"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn boot_verdict_command_feeds_pull_path() {
        // 프론트 pull 경로(emit-before-listen 회피)의 백엔드 반쪽을 커맨드 레벨로 검증한다: start()가
        // invoke("boot_verdict")로 받는 값이 안전모드면 Some(안내)·정상이면 None 이어야 안내 표시가 성립.
        // 테스트 프로세스 exe(target/…/deps/cys_app-*)는 비정규 경로 → escape env 없으면 Some.
        // CYS_ALLOW_NONCANONICAL 은 이 커맨드 외 어떤 테스트도 읽지 않아 병렬 간섭 없음.
        std::env::remove_var("CYS_ALLOW_NONCANONICAL");
        let g = boot_verdict();
        assert!(g.is_some(), "비정규 실행(test 하네스 경로)에서 pull 은 안내 문구를 반환");
        assert!(
            g.unwrap().contains("xattr -d com.apple.quarantine /Applications/cys.app"),
            "pull 이 반환한 안내에 복구 명령 포함(프론트 stickyToast 본문)",
        );
        std::env::set_var("CYS_ALLOW_NONCANONICAL", "1");
        assert!(boot_verdict().is_none(), "escape env 에서는 None(정상 부트 — 안내 미표시)");
        std::env::remove_var("CYS_ALLOW_NONCANONICAL");
    }

    #[test]
    fn plan_cli_install_warns_on_nonstandard_but_proceeds() {
        let plan = plan_cli_install(
            std::path::Path::new("/Users/x/Downloads/cys.app/Contents/MacOS"),
            "/usr/local/bin"
        ).expect("nonstandard는 경고와 함께 진행");
        assert!(plan.osascript_arg.contains("with administrator privileges"));
        assert!(plan.warnings.iter().any(|w| w.contains("표준 위치")));
        assert_eq!(plan.cys_src, std::path::PathBuf::from("/Users/x/Downloads/cys.app/Contents/MacOS/cys"));
    }

    #[test]
    fn plan_cli_install_canonical_has_no_location_warning() {
        let plan = plan_cli_install(
            std::path::Path::new("/Applications/cys.app/Contents/MacOS"),
            "/usr/local/bin"
        ).expect("정규 번들은 진행");
        assert!(plan.warnings.iter().all(|w| !w.contains("표준 위치")));
        // osascript 인자는 do shell script + 승격 + 멱등 스크립트를 감싼다(AppleScript 큰따옴표 리터럴)
        assert!(plan.osascript_arg.starts_with("do shell script \""));
        assert!(plan.osascript_arg.ends_with("\" with administrator privileges"));
    }

    #[test]
    fn applescript_str_escapes_backslash_and_doublequote() {
        assert_eq!(applescript_str("/usr/local/bin"), "\"/usr/local/bin\"");
        assert_eq!(applescript_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(applescript_str("a\\b"), "\"a\\\\b\"");
    }

    // 회귀 가드: osascript 인자는 AppleScript 큰따옴표 리터럴로 감싸야 한다(작은따옴표면 -2741로
    // 모든 호출이 admin 프롬프트 전에 실패 = dead-on-arrival). 내부 셸 경로는 작은따옴표 유지.
    #[test]
    fn osascript_arg_wraps_shell_in_applescript_double_quotes() {
        let plan = plan_cli_install(
            std::path::Path::new("/Applications/cys.app/Contents/MacOS"),
            "/usr/local/bin",
        )
        .unwrap();
        assert!(plan.osascript_arg.starts_with("do shell script \""));
        assert!(plan.osascript_arg.ends_with("\" with administrator privileges"));
        assert!(!plan.osascript_arg.starts_with("do shell script '"));
        assert!(plan.osascript_arg.contains("'/usr/local/bin/cys'"));
        assert!(plan.osascript_arg.contains("ln -sf"));
    }

    /// ★D2a(purge-safety 2026-07-16) 회귀 트립와이어: GUI purge 는 --purge-workdir 를 절대
    /// 되살리지 않는다 — 전 부서 cwd=$HOME 현실에서 홈 스냅샷·격리(파괴) 경로였다(실사고).
    /// 재도입하려면 백엔드 D1a 게이트(workdir_owned)와 모달 고지문("작업 폴더 보존")을 함께 바꿔야
    /// 하며, 그 전에 이 테스트가 막는다.
    #[test]
    fn purge_dept_cmd_never_requests_workdir_purge() {
        let src = include_str!("main.rs");
        let start = src
            .find("async fn purge_dept_daemon_by_socket")
            .expect("purge_dept_daemon_by_socket 정의 소실 — 트립와이어 재배선 필요");
        let seg = &src[start..start + src[start..].find("\n#[tauri::command]").unwrap_or(src.len() - start)];
        assert!(
            !seg.contains("--purge-workdir"),
            "GUI purge 가 --purge-workdir 를 다시 요청함 — 홈 파괴 경로 재개방(실사고 2026-07-16 재발)"
        );
        assert!(seg.contains("--purge-state"), "purge 명령 골격 변형 — 트립와이어 재검토 필요");
    }

    // ══════════════ 팀 부트 단일 계약(W4 · B5·B15·B16 · G34 GUI) ══════════════

    /// B16: 경고는 **의무(Fatal) 역할이 빠졌을 때만** 난다. 리뷰어·grok(Degrade)은 대체 폴백으로
    /// 보완되므로 경고 대상이 아니다 — 이 규율이 깨지면 "건강한 팀 + grok 미설치"(사실상 전 기계)
    /// 에서 재부트마다 '팀 기동 실패' 위경보가 뜬다(P3-B16 실증).
    #[test]
    fn boot_warning_only_for_mandatory_failures() {
        let degrade_only = r#"cys boot — 편성 점검
· grok: CLI 'grok' 미설치 — 건너뜀
{"roles":[{"role":"cso","outcome":"launched","mandatory":true},
          {"role":"worker","outcome":"already_alive","mandatory":true},
          {"role":"reviewer-grok","outcome":"missing","mandatory":false,"install_hint":"npm i -g grok"},
          {"role":"reviewer-gemini","outcome":"failed","mandatory":false}],
 "summary":{"launched":1,"failed":1,"missing":1,"fatal_failed":0,"lock":"acquired"}}"#;
        assert_eq!(boot_json_fatal_message(degrade_only), None, "Degrade 실패가 경고를 만들었다");
        assert_eq!(cys_boot_signal(Some(0), degrade_only), BootSignal::Silent);

        let fatal = r#"{"roles":[{"role":"cso","outcome":"missing","mandatory":true,
                                  "install_hint":"winget install Anthropic.Claude"}],
                        "summary":{"fatal_failed":1}}"#;
        let msg = boot_json_fatal_message(fatal).expect("Fatal 인데 경고가 없다");
        // B15: install_hint 는 **생산자 문구 그대로** — GUI 가 플랫폼 사본을 만들지 않는다
        assert!(msg.contains("winget install Anthropic.Claude"), "install_hint 미표출: {msg}");
        assert!(msg.contains("cso=missing"), "역할·판정 미표출: {msg}");
        assert!(matches!(cys_boot_signal(Some(1), fatal), BootSignal::Warn(_)));
    }

    /// busy(exit 75)는 실패가 아니라 정보다 — 훅↔GUI 중첩 부트는 정상 시나리오다(G11·하드 제약 6-⑧).
    #[test]
    fn boot_busy_is_info_not_warning() {
        let busy = r#"{"roles":[{"role":"cso","outcome":"busy","mandatory":true}],
                       "summary":{"busy":5,"lock":"busy"}}"#;
        match cys_boot_signal(Some(cys::EXIT_BOOT_BUSY), busy) {
            BootSignal::Info(m) => assert!(m.contains("건너뜁니다"), "busy 안내 문구: {m}"),
            other => panic!("busy 가 정보 신호가 아니다: {other:?}"),
        }
        // 파싱 실패 + 미지 비0 은 fail-closed 로 경고한다(조용한 성공 금지)
        assert!(matches!(cys_boot_signal(Some(4), "산문만"), BootSignal::Warn(_)));
        assert!(matches!(cys_boot_signal(None, ""), BootSignal::Warn(_)));
        assert_eq!(cys_boot_signal(Some(0), "산문만"), BootSignal::Silent);
    }

    /// 1차 경로(체인)의 타입드 exit 소비 — 정상 완주는 무신호, 정상 skip·단독 각성은 정보,
    /// 단계 실패는 경고(단계명 + 생산자 detail 인용).
    #[test]
    fn bootstrap_chain_signal_consumes_typed_exit_space() {
        let done = r#"{"ok":true,"marker":"base 마커 기록","lane":"base"}"#;
        assert_eq!(bootstrap_chain_signal(Some(0), done, ""), BootSignal::Silent);
        let solo = r#"{"ok":true,"marker":"부서장 단독 각성(CEO 티켓 부재)","solo_awakening":true,"dept":"dept-2"}"#;
        match bootstrap_chain_signal(Some(0), solo, "") {
            BootSignal::Info(m) => assert!(m.contains("CEO 티켓"), "티켓 안내 누락: {m}"),
            other => panic!("단독 각성이 정보 신호가 아니다: {other:?}"),
        }
        assert!(matches!(bootstrap_chain_signal(Some(11), "", ""), BootSignal::Info(_)));
        match bootstrap_chain_signal(Some(7), "", "") {
            BootSignal::Warn(m) => assert!(m.contains("master"), "정당거부 처방 누락: {m}"),
            other => panic!("claim 정당거부가 경고가 아니다: {other:?}"),
        }
        // exit 4(④boot Fatal) — 단계명과 생산자 detail(install_hint 포함)을 인용한다
        let se = "[bootstrap] 단계 실패: ④boot (exit 1)\n의무(Fatal) 역할 기동 실패: cso=missing [claude 설치: curl -fsSL https://claude.ai/install.sh | bash]";
        match bootstrap_chain_signal(Some(4), "", se) {
            BootSignal::Warn(m) => {
                assert!(m.contains("팀 기동(④cys boot)"), "단계 라벨 누락: {m}");
                assert!(m.contains("claude.ai/install.sh"), "생산자 힌트 미인용: {m}");
            }
            other => panic!("체인 실패가 경고가 아니다: {other:?}"),
        }
        assert!(matches!(bootstrap_chain_signal(None, "", ""), BootSignal::Warn(_)));
    }

    /// ★B5 트립와이어: GUI 는 팀 부트 판정에 **stdout 산문 문자열**을 다시 쓰지 않는다.
    /// (구 판정 재료였던 "신규 기동 0"·"미설치" 매칭이 되살아나면 RC1 사본 드리프트가 재발한다.)
    #[test]
    fn gui_boot_diagnosis_has_no_prose_matching() {
        let src = include_str!("main.rs");
        let start = src.find("fn spawn_orchestra_boot").expect("팀 부트 함수 소실");
        let end = start + src[start..].find("\nfn emit_boot_signal").expect("배선 변형");
        let seg = &src[start..end];
        for banned in ["신규 기동 0", "\"미설치\"", "contains(\"미설치\")"] {
            assert!(!seg.contains(banned), "산문 문자열 매칭 재도입: {banned}");
        }
        assert!(seg.contains("javis_bootstrap.py"), "1차 경로가 체인이 아니다(B5 미착지)");
        assert!(seg.contains("boot-degraded"), "폴백 강등이 조용하다(typed 신호 부재)");
        assert!(seg.contains("--json"), "폴백이 typed 계약을 소비하지 않는다");
    }

    /// ★G34 GUI 지점: 부서 소켓 주입은 **레인 팩과 쌍**이어야 하고, 유도는 lib 단일 소스를 쓴다.
    #[test]
    fn dept_master_injects_lane_pack_pair() {
        let src = include_str!("main.rs");
        let start = src.find("async fn start_dept_master").expect("start_dept_master 소실");
        let end = start + src[start..].find("\n/// 부서 데몬 teardown").unwrap_or(600);
        let seg = &src[start..end];
        assert!(seg.contains("lane_pack_for_socket"), "레인 팩 유도 부재(중복 구현·미주입)");
        assert!(seg.contains("CYS_PACK_DIR"), "CYS_PACK_DIR 동반 주입 부재(G34 GUI 지점 미수리)");
        assert!(seg.contains("CYS_SOCKET"), "소켓 주입 소실");
        // 유도 함수가 lib 정본이다(GUI 사본 금지)
        let home = cys::home_dir();
        assert_eq!(
            cys::pack::lane_pack_for_socket(std::path::Path::new("/x/cys-dept-sales/cys.sock")),
            Some(home.join(".cys").join("pack-dept-sales"))
        );
        assert!(cys::pack::lane_pack_for_socket(std::path::Path::new("/x/cys/cys.sock")).is_none());
    }

    /// launch-agent stdout → surface ref 회수(1차 경로의 ③claim-role 귀속 전제).
    #[test]
    fn launched_surface_ref_reads_stdout_contract() {
        assert_eq!(
            launched_surface_ref(b"surface:42\n"),
            Some("surface:42".to_string())
        );
        // 진단 산문이 섞여도 stdout 의 surface 줄만 취한다(마지막 우선)
        assert_eq!(
            launched_surface_ref(b"noise\nsurface:7\ntrailing\n"),
            Some("surface:7".to_string())
        );
        assert_eq!(launched_surface_ref(b""), None);
        assert_eq!(launched_surface_ref(b"error: nope\n"), None);
    }
}
