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
    // 다운 등)는 즉시 반환한다. 정책(상수·jitter)은 CLI(cys)와 공용 단일 진실인
    // lib(cys::PIPE_BUSY_* · next_busy_delay). WaitNamedPipeW 커널 대기는 blocking이라
    // async 레인에선 쓰지 않는다(tokio named pipe 표준 = sleep 재시도) — 간격만 jitter 로
    // 분산해 fan-out 동시 재시도의 위상 충돌을 깬다.
    let name = socket.to_string_lossy().into_owned();
    let deadline = std::time::Instant::now() + cys::PIPE_BUSY_RETRY_DEADLINE;
    let mut delay = cys::PIPE_BUSY_RETRY_INTERVAL;
    loop {
        match ClientOptions::new().open(&name) {
            Ok(s) => return Ok(Box::new(s) as Stream),
            Err(e)
                if e.raw_os_error() == Some(cys::PIPE_BUSY_ERROR)
                    && std::time::Instant::now() < deadline =>
            {
                delay = cys::next_busy_delay(delay, cys::rand01_cheap());
                tokio::time::sleep(delay).await;
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

/// 앱 마우스 킬스위치 게이트(파일/환경변수 — ime_debug_enabled 와 동형 · 2026-08-12 R3 확정):
/// USER-MANUAL 이 안내하던 localStorage.cysAllowAppMouse 는 릴리스 빌드에 devtools 가 없어
/// 최종 사용자가 설정할 수단 자체가 없었다('현장 롤백 채널' 불변식이 출고 산출물에서 허구).
/// → ~/.cys/allow-app-mouse 파일 존재 또는 CYS_ALLOW_APP_MOUSE=1 이면 트래킹 스트리핑·입력측
/// 마우스 필터를 모두 우회해 TUI 앱(vim mouse=a 등)이 마우스를 갖는다(새 pane 부터 적용).
#[tauri::command]
fn app_mouse_enabled() -> bool {
    std::env::var("CYS_ALLOW_APP_MOUSE").map(|v| v == "1").unwrap_or(false)
        || cys::home_dir().join(".cys/allow-app-mouse").exists()
}

/// Windows 휠 가드 롤백 게이트(파일/환경변수 — 위 두 게이트와 완전 동형 · C-4):
/// Windows 에서 alt 화면 TUI(Claude Code fullscreen 등)에 휠을 굴리면 xterm 이 노치당
/// 방향키를 합성해 쏘는 문제를 UI 측 술어(shouldSuppressWheelWin)로 억제하는데, 그 억제가
/// 누군가의 워크플로를 깨뜨렸을 때 되돌릴 탈출구가 필요하다. 릴리스 빌드엔 devtools 가 없어
/// localStorage 는 최종 사용자의 롤백 수단이 될 수 없다(ime_debug_enabled 주석과 같은 함정) →
/// ~/.cys/win-wheel-guard-off 파일 존재 또는 CYS_WIN_WHEEL_GUARD_OFF=1 이면 가드를 끈다.
/// ★기존 allow-app-mouse 킬스위치를 이 탈출구로 재사용하면 안 된다 — 그것은 입·출력 양측을
/// 열어 Windows ConPTY 결함 1호(마우스 보고가 리터럴로 타이핑되는 현상)를 되살린다.
/// 그래서 '출력측 휠 억제만' 끄는 전용 게이트를 따로 둔다.
///
/// ★두 수단의 **적용 시점이 다르다**(사용자 문서에 같은 내용이 명시돼 있다 — USER-MANUAL
/// §4.6b · env 표): 파일은 pane 을 만들 때마다 stat 하므로 **새 pane 부터 즉시** 반영되지만,
/// env 는 이 GUI 프로세스가 **기동 시 상속한 값**만 보므로 터미널에서 set/setx 해도 이미 떠
/// 있는 GUI 에는 반영되지 않는다(=GUI 재시작 필요). Windows 는 GUI 를 탐색기·바로가기로
/// 띄우는 것이 보통이라 실질 권장 수단은 **파일**이다. 그 파일을 PowerShell 에서 만드는
/// 정본 명령은 `New-Item -ItemType File -Force $HOME\.cys\win-wheel-guard-off` 다
/// (`touch` 는 PowerShell·cmd 에 존재하지 않는다 — 안내 문안에 쓰지 마라).
///
/// ★이름은 **술어형**이다(2026-08-17 개명 — 성찰3 설계렌즈 note): 종전 `win_wheel_guard_off`
/// 는 불리언 질의인데 명령형 동작("가드를 꺼라")으로 오독될 여지가 있었고, 형제 게이트 둘
/// (`app_mouse_enabled`·`ime_debug_enabled`)이 모두 술어형이라 doc 이 주장한 '완전 동형'이
/// 이름에서만 깨져 있었다. 커맨드는 이 바이너리에 UI 와 함께 묶여 나가므로(ui/dist 임베드)
/// 외부 호환 부담이 없다 — 호출부는 ui/src/main.ts 의 invoke 문자열 하나뿐이다.
/// 사용자 표면(env `CYS_WIN_WHEEL_GUARD_OFF` · 파일 `~/.cys/win-wheel-guard-off`)은 **불변**이다.
#[tauri::command]
fn win_wheel_guard_disabled() -> bool {
    std::env::var("CYS_WIN_WHEEL_GUARD_OFF").map(|v| v == "1").unwrap_or(false)
        || cys::home_dir().join(".cys/win-wheel-guard-off").exists()
}

#[tauri::command]
async fn send_input(
    socket: Option<String>,
    surface_id: u64,
    data: String,
    queued: Option<bool>,
    clear_first: Option<bool>,
    // ★R5: 이 문안을 **UI 코드가 조립했는가**(전출 지시·재기동 명령·경로 삽입 = true) —
    // 사용자가 자판으로 친 실키(sendRaw/붙여넣기)는 false(미지정)다. 아래 본문 주석 참조.
    machine_origin: Option<bool>,
) -> Result<(), String> {
    // human=true: T3-13 타이핑 가드의 신호 — UI 키 입력을 '사람'으로 표시해
    // 원격 주입이 사람의 미완성 입력을 오염시키지 못하게 한다.
    // queued=true(전출 복원 주입 등 후속 지시)는 사람 타이핑이 아니므로 human=false —
    // human=true로 큐잉하면 last_human_input 갱신이 타이핑 가드를 3초 오염시킨다.
    // clear_first=true는 데몬 T3-13 권위 전달(Ctrl-U 정리→paste→지연 CR 원자 제출) —
    // raw "\r" 동봉은 Claude CLI가 paste로 삼켜 미제출된다(전출 e2e 실측). queued와 결합 불가.
    // 전출 지시도 사람의 클릭에서 발화하므로 human 유지(타이핑 가드 결정론 통과).
    //
    // ★R4(2026-08-02): `human`은 클라이언트 자기신고라 데몬이 그것만으로 **배달 원장 기록을
    // 억제하지 않는다**(원시 소켓 1줄로 임무 게이트가 열리던 N3 관통). 억제 근거는 데몬이
    // 발급·0600 보관하는 operator.token 뿐이다 — 이 pane 이 붙어 있는 **그 데몬의** 토큰을
    // 읽어 첨부한다(부서 데몬은 자기 state 디렉토리에 자기 토큰을 갖는다). 첨부 실패(구 데몬·
    // 권한)면 데몬이 fail-closed 로 **기록**한다: 오너 키 입력이 원장에 남아도 원장 단위는
    // 키 조각(term.onData)이고 훅은 프롬프트 전문을 해시하므로 매치되지 않는다(피해 경미).
    // 매 호출 신선 재독(캐시 금지) — 데몬 재기동마다 토큰이 재발급된다.
    //
    // ★★R5(2026-08-02): 토큰이 증명하는 것은 **'사람이 앉은 GUI 세션'**이지 **'사람이 친 문장'**이
    // 아니다. 이 커맨드는 두 종류를 함께 나른다 —
    //   ⓐ 사용자가 자판으로 친 실키(`sendRaw` ← `term.onData`·붙여넣기) → machine_origin 없음
    //   ⓑ UI 코드가 조립한 문안(전출 지시 전문·`launchCmd`·`restartNode`·`injectRawToPane`)
    //      → 호출부가 `machineOrigin: true` 를 넘긴다
    // R4 배선은 ⓑ 에도 토큰을 붙여 **배달 원장 기록을 억제**했고, 그래서 GUI 가 자동 주입한
    // 문안이 훅에게 **오너 임무**로 보였다(실측 rc=0·흔적 0 — 자율 착수 권한 오발급). 이제
    // 표식을 그대로 데몬에 전달하고, 데몬은 표식이 있으면 토큰이 유효해도 기록한다
    // (`handlers.rs::surface.send_text` · origin=`gui_auto`).
    // ⓐ 는 **반드시 무기록**이어야 한다 — 기록되면 오너 문장이 자기 해시와 매치돼 임무를 영영
    // 줄 수 없다(온보딩 사망). 두 경로를 여기서 섞지 말 것.
    let q = queued.unwrap_or(false);
    let cf = clear_first.unwrap_or(false);
    let mo = machine_origin.unwrap_or(false);
    let sock = resolve_socket(&socket);
    let mut params = json!({"surface_id": surface_id, "text": data, "quiet": true, "human": !q,
                            "queued": q, "clear_first": cf, "machine_origin": mo});
    let tok = read_operator_token_for(&sock);
    // ★★결함#6-b 잔여분(2026-08-22 · 오너 실사고 후속) — `owner_token` = **ACL 등급 전용** 키.
    //
    // 무엇이 남아 있었나: #6-b 1차 수리는 데몬 ACL 에 `owner` 등급을 신설해 오너 GUI 를 external
    // 과 구별했지만, 등급 판정 근거가 `operator_token` 이었다. 그 키는 아래 `!q && !mo` 분기에서만
    // 붙으므로 **UI 가 조립한 주입**(전출 지시 전문·`launchCmd`·`restartNode`·`injectRawToPane`·
    // queued 후속 지시)은 여전히 external 로 남아 **부서 워커 pane 에서만 차단**됐다. 오너 절대
    // 규칙("모든 노드의 프롬프트 창을 오너가 컨트롤")에 비추면 같은 결함의 잔여분이다.
    //
    // 왜 `operator_token` 을 그대로 넓히지 않았나(중요): 그 키에는 **면제**가 매달려 있다 —
    // 배달 원장 무기록(R4/R5)과 `feed.reply` §3.2 자기승인 우회. 첨부 범위를 넓히면 그 면제들이
    // 함께 넓어질 위험이 생긴다(= v0.14.22 가 고친 '통과하면 안 되는 승인' 부류의 재발 경로).
    // 데몬 전수 조사 결과 오늘은 `human_verified = human && !machine_origin && …` 의 `!machine_origin`
    // 곱 덕분에 실제 동작이 안 바뀌지만, 그건 **한 줄의 논리곱에 기댄 안전**이다. 키를 나누면
    // 그 의존이 사라진다 — 아래 두 블록은 **서로 다른 조건**을 갖고, 아래 블록은 종전 그대로다.
    //   · `owner_token`  : 모든 GUI pane 쓰기(실키·machine_origin·queued) — 소비자는 ACL 등급 하나.
    //   · `operator_token`: `!queued && !machine_origin` 일 때만 — 원장·승인 면제의 근거(불변).
    // 값이 같은 비밀인 이유는 별도 비밀의 발급·회전·배포 경로를 새로 만드는 것 자체가 새 실패
    // 모드이기 때문이다. 분리한 것은 비밀이 아니라 **면제 범위**다.
    if let Some(t) = &tok {
        params["owner_token"] = json!(t);
    }
    // 표식이 붙은 자동 주입에는 `operator_token` 을 아예 붙이지 않는다(데몬도 표식으로 무시하지만,
    // 첨부 자체를 하지 않는 편이 "이 키는 사람 실키 전용"이라는 계약을 코드 한 곳에서 더 분명히
    // 만든다). ★이 조건을 넓히지 마라 — 넓히면 원장 무기록·자기승인 면제가 함께 넓어진다.
    if !q && !mo {
        if let Some(t) = &tok {
            params["operator_token"] = json!(t);
        }
    }
    rpc_on(&sock, "surface.send_text", params).await.map(|_| ())
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
/// 동류). 예전에는 plan_cli_install 이 NonStandard 를 경고만 하고 허용해 의도적 divergence 가 있었으나,
/// **D5(2026-08-23)로 plan_cli_install 도 NonStandard 를 거부하면서 두 판정은 Canonical 만 허용으로 수렴했다**.
/// 게이트는 여전히 둘이다 — 가리는 행위가 다르기 때문이다(이쪽은 무음 plist 기록, 저쪽은 명시 심볼릭 생성).
/// 비-Canonical 은 자동등록만 skip 하고 ensure_daemon 런타임 폴백(휘발성 데몬)으로 안전하게 흐른다.
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

// ── ATOMIC-1 짝: 설치본 자기 무결성(반쪽 번들) 기동 점검 ────────────────────────
//
// ★2026-08-01 실사고: `/Applications/cys.app/Contents/Info.plist` 가 사라진 **반쪽 번들**이
//   남았고, 사용자에게 보이는 얼굴은 "손상되었기 때문에 열 수 없습니다" 한 줄뿐이었다.
//   교체를 수행하는 주체가 우리 코드가 아닐 때(Finder 드래그의 '바꾸기' · tauri-plugin-updater)
//   교체 자체는 계약화할 수 없다 — 그래서 **설치 후/기동 시 검증**으로 덮는다.
//
// ★왜 안전모드처럼 부트를 멈추지 않고 '알리기만' 하는가(의도적 divergence):
//   translocation 게이트가 멈추는 이유는 부수효과가 **능동적으로 해롭기** 때문이다(휘발 경로가
//   launchd plist 에 각인되면 죽은 경로 데몬을 무한 스폰한다). 반쪽 번들은 그런 종류가 아니라
//   '기능이 빠진' 상태다. 여기서 부트를 멈추면 ⓐ 오탐 1건이 정상 사용자의 앱을 통째로 무력화하고
//   ⓑ 아직 동작하는 기능까지 뺏는다. 사고의 진짜 피해는 "고장을 아무도 말해주지 않은 것"이었으므로,
//   처방도 거기에 맞춘다 — **정확히 무엇이 빠졌는지 말하고 복구 절차를 준다.**
#[cfg(target_os = "macos")]
fn bundle_integrity_guidance() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    // 레이아웃 기반 탐지(cys::app_bundle) — Info.plist 실재를 번들 확증 조건으로 쓰지 않는다.
    // 그걸 조건으로 걸면 **사고 상태에서 정확히 탐지가 꺼진다**(가장 파괴적인 손상이 무판정).
    let bundle = cys::app_bundle::enclosing_bundle(&exe)?;
    let defects = cys::app_bundle::verify(&bundle, &cys::app_bundle::VerifySpec::installed());
    if defects.is_empty() {
        return None;
    }
    Some(cys::app_bundle::damaged_bundle_guidance(&bundle, &defects))
}

// ── SEAL-DIAG: 설치 후 코드서명 봉인 자가진단(macOS 전용 · ★advisory 전용) ──────────
//
// 위 `bundle_integrity_guidance` 가 **구조 결손**(반쪽 번들 = stat 몇 번)을 보는 데 반해,
// 이쪽은 **봉인 파손**(codesign)을 본다. 둘은 같은 사고(2026-08-01)의 다른 절반이고,
// 비용도 성질도 달라서 발화 방식이 다르다:
//   · 구조 결손 = 즉시(수 ms) → setup 안에서 그대로 판정.
//   · 봉인 파손 = `codesign --deep`(초 단위) → **별도 스레드**로 내보내 부트 경로에서 완전히 뗀다.
//
// ★절대 계약: 어떤 판정에서도 기동을 막거나 지연시키지 않는다. `std::thread::spawn` 은 즉시
//   반환하고, 이 스레드는 tauri 런타임과 무관하게 돈다 — setup 이 이 검사를 기다리는 지점이
//   한 곳도 없다(그게 곧 "부트 차단 없음"의 기계적 근거다).
//
// ★알림 채널은 기존 `bundle-damaged` 스티키 토스트를 **재사용**한다(새 이벤트를 만들지 않는다):
//   ⓐ 사용자에게 하는 말이 정확히 같다 — "설치본이 온전하지 않습니다 — 재설치 필요".
//   ⓑ 새 이벤트는 프론트(ui/src/main.ts + 빌드 산출물 ui/dist)까지 함께 고쳐야 발화하는데,
//      그 연쇄가 끊기면 **알림이 조용히 사라진다**(이 기능의 유일한 임무가 알리는 것인데).
//   두 판정이 동시에 참이면 나중 것이 토스트를 덮지만, 둘 다 결론이 "재설치"라 안내는 어긋나지 않는다.
//
// ★pull 백스톱(F3 격차1): emit(push)은 프론트가 listen 을 걸기 **전**이면 그대로 유실된다
//   (emit-before-listen 레이스 — 웹뷰 리로드·기동 타이밍이 대표 경로). 그래서 Broken 판정은
//   emit 과 별개로 전역 캐시(SEAL_BROKEN_CACHE)에 적재하고, 프론트 기동 pull(`bundle_integrity`)
//   이 구조 결손 안내와 **합산**해 회수한다 — push 가 유실된 기동에도 알림이 기계적으로 성립한다.

/// 스로틀 마커 경로 — `~/.cys/state/selfdiag-<version>`.
/// 버전을 파일명에 넣는 이유: 업데이트되면 **새 번들이므로 다시 봐야 한다**(마커 삭제 로직 불요).
#[cfg(target_os = "macos")]
fn seal_selfdiag_marker() -> std::path::PathBuf {
    cys::home_dir()
        .join(".cys/state")
        .join(format!("selfdiag-{}", env!("CARGO_PKG_VERSION")))
}

/// 마커 내용으로 "이번 기동에는 검사를 건너뛰어도 되는가"를 판정한다(순수 함수 = 회귀 핀 대상).
///
/// 마커에는 **직전 판정**을 적는다(존재 여부가 아니라). 그래야 요구 두 개를 동시에 만족한다:
/// ⓐ 평시(무결·판정불가)에는 버전당 한 번만 돌아 매 기동 `codesign --deep` 비용을 물지 않는다.
/// ⓑ **파손이 확인된 뒤에는 마커를 무시하고 매 기동 다시 보고 다시 알린다** — 고쳐질 때까지
///    침묵하면 안 되고, 재설치로 고쳐졌는지도 다시 봐야 알 수 있다.
/// 미지의 문자열은 "모른다 → 다시 본다"로 읽는다(fail-open toward checking).
#[cfg(target_os = "macos")]
fn seal_selfdiag_skips(marker: Option<&str>) -> bool {
    matches!(marker.map(str::trim), Some("intact") | Some("undetermined"))
}

/// SEAL-DIAG pull 캐시 — 봉인 자가진단의 **Broken 안내문만** 담는다(위 ★pull 백스톱 주석).
/// Intact/Undetermined 는 저장하지 않는다: 알릴 것이 없는 판정이 캐시에 남으면
/// 정상 기동·개발 빌드에서 "재설치" 오보가 pull 로 새어 나간다.
#[cfg(target_os = "macos")]
static SEAL_BROKEN_CACHE: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();

#[cfg(target_os = "macos")]
fn seal_broken_cache() -> &'static Mutex<Option<String>> {
    SEAL_BROKEN_CACHE.get_or_init(|| Mutex::new(None))
}

/// 판정 → pull 캐시에 적재할 안내문(순수 함수 = 회귀 핀 대상).
///
/// 문구는 `app_bundle::seal_broken_notice` 산출물 **그대로**다(push 와 동문 — 이원화 금지):
/// push 가 유실된 기동에서 pull 이 다른 문구를 보이면 같은 고장이 두 얼굴을 갖게 된다.
/// Broken 외 판정은 None — 캐시에 아무것도 적재하지 않는다.
#[cfg(target_os = "macos")]
fn seal_cache_payload(
    bundle: &std::path::Path,
    verdict: &cys::app_bundle::SealVerdict,
) -> Option<String> {
    match verdict {
        cys::app_bundle::SealVerdict::Broken {
            culprits,
            self_inflicted,
        } => Some(cys::app_bundle::seal_broken_notice(
            bundle,
            culprits,
            *self_inflicted,
        )),
        _ => None,
    }
}

/// 봉인 자가진단을 **백그라운드 스레드로 내보낸다**(호출 즉시 반환 — 부트 무영향).
///
/// 판정별 처리:
///   · Intact        → 마커 `intact` 기록 · 알림 없음(정상은 말이 없어야 한다).
///   · Undetermined  → 마커 `undetermined` 기록 · **무음 skip** + stderr 디버그 한 줄.
///                     (미서명 개발 빌드·codesign 부재·중첩서명 불만 — 파손으로 오보하지 않는다.)
///   · Broken        → 마커 기록 **안 함**(다음 기동에 다시 본다) · 사용자 알림 발화.
#[cfg(target_os = "macos")]
fn spawn_seal_selfdiag(handle: tauri::AppHandle) {
    std::thread::spawn(move || {
        // 번들 미탐지(cargo run·비번들 설치) = 검사 대상 없음 → 완전 무음.
        let Some(bundle) = std::env::current_exe()
            .ok()
            .and_then(|exe| cys::app_bundle::enclosing_bundle(&exe))
        else {
            return;
        };
        let marker = seal_selfdiag_marker();
        if seal_selfdiag_skips(std::fs::read_to_string(&marker).ok().as_deref()) {
            return;
        }
        let verdict = cys::app_bundle::verify_seal_deep(&bundle);
        // 마커 쓰기는 best-effort — 실패해도 다음 기동에 한 번 더 도는 것뿐이라 무해하다.
        let record = |token: &str| {
            if let Some(dir) = marker.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&marker, token);
        };
        match &verdict {
            cys::app_bundle::SealVerdict::Intact => record("intact"),
            cys::app_bundle::SealVerdict::Undetermined(why) => {
                eprintln!("[cys-app] 봉인 자가진단 판정 불가(무음 skip) — {why}");
                record("undetermined");
            }
            cys::app_bundle::SealVerdict::Broken { .. } => {
                let msg = seal_cache_payload(&bundle, &verdict)
                    .expect("Broken 판정은 항상 안내문을 산출한다(seal_cache_payload 계약)");
                // ★캐시 적재는 emit 보다 먼저다: emit 이 listen 전에 나가 유실돼도
                //   프론트 기동 pull(bundle_integrity)이 이 값을 회수한다(F3 격차1의 기계 보장).
                *seal_broken_cache().lock().unwrap() = Some(msg.clone());
                eprintln!("[cys-app] 코드서명 봉인 파손 감지 — 재설치가 필요합니다\n{msg}");
                let _ = handle.emit("bundle-damaged", msg);
            }
        }
    });
}

/// pull 응답 합산(순수 함수 = 회귀 핀 대상): 구조 결손 안내 ⊕ 캐시된 봉인 파손 안내.
///
/// 덮어쓰기 의미론 계약(위 SEAL-DIAG ★알림 채널 주석)을 pull 에도 그대로 보존한다 —
/// push 경로에서는 봉인 판정이 codesign(초 단위) 탓에 **나중에** 도착해 토스트를 덮으므로,
/// 둘 다 참이면 pull 도 봉인 쪽을 돌려준다. 어느 쪽이 이겨도 결론은 "재설치" 하나라
/// 안내가 어긋나지 않는다. 한쪽만 참이면 그것을, 둘 다 없으면 None(무음).
#[cfg(target_os = "macos")]
fn merge_integrity_pull(
    structural: Option<String>,
    seal_broken: Option<String>,
) -> Option<String> {
    seal_broken.or(structural)
}

/// 프론트 pull 경로(`boot_verdict` 와 같은 이유 — emit-before-listen 레이스 회피).
/// 구조 결손(즉시 판정)과 **캐시된 봉인 파손**(백그라운드 자가진단이 적재 · F3 격차1)을
/// 합산해 돌려준다. 정상 설치본·번들 밖 실행·비 macOS 는 None.
#[tauri::command]
fn bundle_integrity() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let seal = seal_broken_cache().lock().unwrap().clone();
        return merge_integrity_pull(bundle_integrity_guidance(), seal);
    }
    #[allow(unreachable_code)]
    None
}

/// (BLOCK-1) 설치 시 백업 파일명에 박을 타임스탬프(순수). **셸에서 `date` 를 부르지 않는다** —
/// 승격 스크립트가 만드는 이름이 실행 환경(로케일·TZ·PATH 상의 date 구현)에 따라 달라지면
/// "무엇을 어디로 백업했는지"를 Rust 가 사용자에게 정확히 보고할 수 없다. Rust 가 이름을 먼저
/// 확정해 스크립트에 박아 넣어야 보고 문구와 실제 파일명이 **하나의 진실**이 된다.
/// 사람이 읽는 값이 아니라 충돌 회피용 접미사이므로 epoch 초로 충분하다(외부 크레이트 의존 없음).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn backup_stamp(now: std::time::SystemTime) -> String {
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

/// (BLOCK-1) 대상 경로 → 백업 경로(순수). 스크립트가 만드는 이름과 사용자에게 보고하는 이름이
/// 갈라지지 않도록 **생성처를 하나로** 둔다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn backup_path_for(target: &str, stamp: &str) -> String {
    format!("{target}.cys-backup-{stamp}")
}

/// (BLOCK-1) 설치 전 관측 → **백업이 발생할** (원본, 백업본) 경로 쌍(순수).
/// `build_install_script` 의 셸 조건(`-e && ! -L`)과 **같은 판정**을 Rust 쪽에서 한 번 더 계산한다 —
/// 사용자에게 "무엇을 어디로 옮겼는지" 보고하려면 스크립트가 무엇을 할지 미리 알아야 하기 때문이다.
/// 두 판정이 어긋나면 보고가 거짓이 되므로 조건을 바꿀 때는 반드시 함께 바꾼다.
/// dangling 심볼릭(present=true·is_symlink=true)은 셸 `-e` 가 거짓이라 백업되지 않는다 — 여기서도
/// `!is_symlink` 로 같이 제외된다(죽은 링크는 보존할 값이 없다).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn plan_install_backups(probes: &[LinkProbe], stamp: &str) -> Vec<(String, String)> {
    probes
        .iter()
        .filter(|p| p.present && !p.is_symlink)
        .map(|p| (p.path.clone(), backup_path_for(&p.path, stamp)))
        .collect()
}

/// `do shell script` 본문: target_dir 생성 + cys·cysd 심볼릭 생성.
///
/// ★BLOCK-1(2026-08-25) **파괴 금지 → 백업**: 예전 본문은 `ln -sf` 하나였다. `ln -sf` 는 대상이
/// **일반 파일이어도** unlink 후 심볼릭으로 갈아 끼운다 — 즉 Homebrew·수동 빌드가 깔아 둔 실체
/// `/usr/local/bin/cys` 를 아무 말 없이 파괴했다. 해제 경로는 정확히 같은 파일을 "남의 설치본
/// 파괴이고 되돌릴 수 없다"며 지켰는데(`decide_cli_uninstall` 의 SkipNotSymlink) 설치 경로만
/// 정반대 가드였다. 그래서 **지우지 않고 옮긴다**: 심볼릭이 아닌 무언가가 있으면
/// `<경로>.cys-backup-<stamp>` 로 mv 한 뒤에 링크를 만든다. 확인 모달을 두지 않는 근거가 바로
/// 이것이다 — **잃는 것이 없어야 1클릭이 정당하다**.
///
/// 조건이 `[ -e X ] && [ ! -L X ]` 인 이유: 이미 심볼릭이면(우리 것이든 남의 것이든) mv 분기가 돌지
/// 않으므로 **반복 설치가 멱등이고 백업이 쌓이지 않는다**. dangling 심볼릭은 `-e` 가 거짓이라 역시
/// 백업 없이 그대로 갈아 끼워진다. mv 가 실패하면 `fi` 가 실패 상태를 물려주고 `&&` 가 끊겨 링크를
/// 만들지 않는다 — 백업에 실패한 채 원본을 덮는 일은 없다.
///
/// `ln -sfn` 인 이유: BSD ln 에서 대상이 **디렉터리를 가리키는 심볼릭**이면 `-f` 만으로는 그 링크를
/// 갈아 끼우지 않고 그 디렉터리 **안에** 새 링크를 만든다. 그러면 root 권한 쓰기가 target_dir 밖으로
/// 새어 나간다(예: `/usr/local/bin/cys` 가 `/etc` 심볼릭이면 `/etc/cys` 가 생긴다).
/// `-n`(대상 심볼릭을 따라가지 않음)이 그 누출을 구조적으로 막는다.
fn build_install_script(
    cys: &std::path::Path,
    cysd: &std::path::Path,
    target_dir: &str,
    stamp: &str,
) -> String {
    let link = |src: &std::path::Path, name: &str| -> String {
        let dst = format!("{target_dir}/{name}");
        let bak = backup_path_for(&dst, stamp);
        format!(
            "if [ -e {d} ] && [ ! -L {d} ]; then mv {d} {b}; fi && ln -sfn {s} {d}",
            d = sh_squote(&dst),
            b = sh_squote(&bak),
            s = sh_squote(&src.to_string_lossy()),
        )
    };
    format!(
        "mkdir -p {td} && {c} && {d}",
        td = sh_squote(target_dir),
        c = link(cys, "cys"),
        d = link(cysd, "cysd"),
    )
}

/// `which -a cys` 출력 → precedence 순 **절대경로** 리스트.
///
/// ★MINOR-7(2026-08-25): 절대경로(`/` 로 시작)만 받는다. 예전에는 "빈 줄이 아니면 경로"였다.
/// `-lc` 로 로그인 셸을 태우면 rc 파일 배너·경고나 셸마다 다른 which 구현의 비-경로 출력이
/// stdout 에 섞인다(zsh: `cys: aliased to ...`, `cys: shell built-in command`, bash: `cys not found`).
/// 그 잡음을 경로로 격상하면 목록 1순위가 잡음이 되어 **installed 여야 할 설치가
/// installed_shadowed 로 오판**되고, 존재하지도 않는 "앞을 가리는 cys" 를 지우라고 안내하게 된다.
fn parse_which_a(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| l.starts_with('/'))
        .collect()
}

/// (MINOR-6) `plan_cli_install` **전용** 엄격 판정(순수): 번들이 정확히 `/Applications/cys.app`
/// 또는 `<홈>/Applications/cys.app` 인가.
///
/// `classify_bundle_dir` 은 `parent.ends_with("/Applications")` 로 판정한다 — 그래서
/// `/tmp/Applications/cys.app` · `~/Downloads/Applications/cys.app` 처럼 **사용자가 아무 때나 만들 수
/// 있는** 디렉터리도 Canonical 로 통과시킨다. 그 상태로 설치하면 root 소유
/// `/usr/local/bin/cys` 가 사용자 쓰기 가능한 경로를 가리키게 되어, D5 가 막으려던 바로 그 구멍
/// (파일을 바꿔 끼우면 다음 `sudo cys` 가 남의 코드를 돌린다)이 그대로 열린다.
///
/// ★그런데 `classify_bundle_dir` 자체는 건드리지 않는다 — `autoregister_allowed`·`boot_path_verdict`
/// 가 같은 함수를 쓰므로 고치면 부트 전면 게이트까지 함께 좁아지는 산탄총 수술이 된다.
/// 비가역·root 권한이 걸린 **설치만** 이 엄격 판정을 추가로 통과해야 한다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn strict_install_bundle_ok(macos_dir: &std::path::Path, home: &std::path::Path) -> bool {
    if macos_dir.file_name().map(|n| n != "MacOS").unwrap_or(true) {
        return false;
    }
    let Some(contents) = macos_dir.parent() else {
        return false;
    };
    if contents.file_name().map(|n| n != "Contents").unwrap_or(true) {
        return false;
    }
    let Some(bundle) = contents.parent() else {
        return false;
    };
    if bundle.file_name().map(|n| n != "cys.app").unwrap_or(true) {
        return false;
    }
    let Some(parent) = bundle.parent() else {
        return false;
    };
    parent == std::path::Path::new("/Applications") || parent == home.join("Applications")
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
    stamp: &str,
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
        // ★D5(2026-08-23) NonStandard 경고 → **거부** 승격. 예전에는 경고만 달고 진행했으나,
        // 그 결과는 root 소유 심볼릭(/usr/local/bin/cys)이 **사용자가 쓸 수 있는 임의 경로**
        // (~/Downloads·USB 등)를 가리키는 상태다 — 그 경로의 파일을 바꿔 끼우는 것만으로 다음
        // sudo cys 실행이 남의 코드를 돌린다. 게다가 앱을 옮기면 즉시 죽은 링크가 되고 자가치유도
        // 불가능하다. 경고는 읽히지 않고 지나가므로 게이트로 올린다(autoregister_allowed 와 판정 수렴).
        BundleKind::NonStandard => {
            let bundle = macos_dir
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| macos_dir.to_string_lossy().to_string());
            return Err(format!(
                "cys.app이 표준 위치(Applications)가 아닌 곳에서 실행 중입니다: {bundle}\n\
Finder에서 cys.app을 Applications 폴더로 옮긴 뒤 다시 열고 시도하세요."
            ));
        }
        BundleKind::Canonical => {}
    }
    // ★MINOR-6(2026-08-25): Canonical 이어도 여기서 한 번 더 좁힌다. classify_bundle_dir 의
    // ends_with("/Applications") 판정은 /tmp/Applications·~/Downloads/Applications 처럼 사용자가
    // 직접 만든 디렉터리를 통과시키므로, D5 의 거부 근거(root 링크가 사용자 쓰기 가능 경로를
    // 가리키면 안 된다)가 실제로는 성립하지 않았다. 설치 경로에만 엄격 판정을 덧댄다.
    if !strict_install_bundle_ok(macos_dir, &cys::home_dir()) {
        let bundle = macos_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| macos_dir.to_string_lossy().to_string());
        return Err(format!(
            "cys.app이 표준 Applications 폴더가 아닌 곳에서 실행 중입니다: {bundle}\n\
정확히 /Applications 또는 홈 폴더의 Applications 안에 있는 cys.app에서만 설치할 수 있습니다. \
Finder에서 cys.app을 Applications 폴더로 옮긴 뒤 다시 열고 시도하세요."
        ));
    }
    // 가드를 모두 통과한 Canonical 경로만 여기까지 온다 — 위치 경고는 더 이상 존재하지 않는다.
    let warnings: Vec<String> = vec![];
    let cys_src = macos_dir.join("cys");
    let cysd_src = macos_dir.join("cysd");
    let script = build_install_script(&cys_src, &cysd_src, target_dir, stamp);
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

/// (D6) 자식 프로세스 실행에 기한을 건다. std 에는 프로세스 타임아웃이 없어 spawn + try_wait 폴링 +
/// 기한 초과 kill 로 직접 만든다 — 로그인 셸은 rc(nvm·conda·사내 프로필)가 매달리면 **무기한**
/// 블록하고, 동기 tauri 커맨드가 그대로 굳어 버튼이 영영 돌아오지 않는다(사용자에겐 '앱이 멈춤').
///
/// ★MAJOR-3(2026-08-25) **파이프 제거**: 예전 구현은 stdout 을 파이프로 받아 별도 스레드에서
/// `read_to_string` 으로 드레인하고 마지막에 그 스레드를 **기한 없이** join 했다. `read_to_string` 은
/// 파이프의 write-end 가 **전부** 닫혀야 EOF 를 본다 — 로그인 셸이 띄운 손자(ssh-agent·gpg-agent·
/// 백그라운드 잡 등)가 stdout 을 물고 있으면 부모를 kill 해도 EOF 가 오지 않아 join 이 무기한
/// 블록한다. 즉 **타임아웃이 있는데도 커맨드가 굳었다**. 기존 테스트가 초록이던 이유는
/// `sh -c "sleep 30"` 이 exec 대체되어 손자가 아예 없는 유일한 형태였기 때문이다.
///
/// 그래서 파이프와 드레인 스레드를 없애고 자식 stdout 을 **임시 파일**로 리다이렉트한다
/// (`Stdio::from(File)`). wait 또는 kill 이 끝난 뒤 그 파일을 읽으므로 EOF 의존이 사라지고 손자
/// 문제와 무관해진다. 파이프 버퍼(64KB) 포화 데드락도 같이 사라진다 — 파일은 차지 않는다.
/// 임시 파일 이름은 pid + 나노초 + 프로세스 내 시퀀스로 충돌을 피하고, 어느 경로로 빠져나가든
/// 반드시 지운다.
///
/// 반환은 `Ok((정상종료 여부, stdout))` / `Err(사유)` — **실패를 빈 출력으로 접지 않는다**. 이 구분이
/// 없으면 '측정 실패'와 '결과 없음'이 같은 값이 되어 D3 의 unverified 판정이 불가능해진다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn run_capture_with_timeout(
    program: &str,
    args: &[&str],
    limit: std::time::Duration,
) -> Result<(bool, String), String> {
    run_capture_with_timeout_in(&std::env::temp_dir(), program, args, limit)
}

/// 위 함수의 본체. 임시 파일을 놓을 디렉터리를 인자로 받는 이유는 **정리 검증을 결정론으로 만들기
/// 위해서**다 — 공유 TMPDIR 을 세면 병렬 테스트끼리 같은 pid 아래 파일을 만들어 카운트가 흔들린다.
/// 프로덕션은 언제나 `std::env::temp_dir()` 하나로 들어온다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn run_capture_with_timeout_in(
    tmp_dir: &std::path::Path,
    program: &str,
    args: &[&str],
    limit: std::time::Duration,
) -> Result<(bool, String), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let out_path = tmp_dir.join(format!(
        "cys-probe-{}-{}-{}.out",
        std::process::id(),
        nanos,
        PROBE_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let sink = std::fs::File::create(&out_path)
        .map_err(|e| format!("{program} 출력 임시파일 생성 실패({}): {e}", out_path.display()))?;
    let spawned = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(sink))
        .stderr(std::process::Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&out_path);
            return Err(format!("{program} 실행 실패: {e}"));
        }
    };
    let deadline = std::time::Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Ok(st.success()),
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // 좀비 수거
                    break Err(format!("{program} 타임아웃({}초 초과)", limit.as_secs_f32()));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => break Err(format!("{program} 상태 확인 실패: {e}")),
        }
    };
    // 자식이 끝났거나(kill 포함) 상태 확인이 실패한 **그 시점까지 쓰인 것**이 관측값이다.
    // 손자가 아직 물고 있어도 기다리지 않는다 — 기다리는 순간 타임아웃이 무의미해진다.
    let stdout = std::fs::read_to_string(&out_path).unwrap_or_default();
    let _ = std::fs::remove_file(&out_path);
    status.map(|ok| (ok, stdout))
}

/// (D3) which 프로브의 관측 결과. 실행 실패·타임아웃을 '빈 목록'으로 접으면 측정불능이 통과로
/// 둔갑하므로 성공/실패를 **타입으로** 갈라 둔다(예전 `.ok()` + `unwrap_or_default()` 가 정확히
/// 그 사고였다 — which 가 죽어도 shadowed_by=None + "설치 완료" 토스트가 나갔다).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum WhichProbe {
    /// which -a 가 끝까지 돌았고 precedence 순 경로 목록을 얻었다(비어 있을 수 있다).
    Completed(Vec<String>),
    /// 실행 실패·타임아웃 — 측정 자체를 못 했다(사유 동봉).
    Unmeasured(String),
}

/// (D3) 설치 등급 판정 결과. status 는 정확히 셋뿐이다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct InstallVerdict {
    status: &'static str,
    effective_cys: Option<String>,
    shadowed_by: Option<String>,
    warnings: Vec<String>,
}

/// (D3) 설치 등급 판정(순수). 헌장 원칙 "측정 불능은 어떤 게이트에서도 통과가 아니다" 를 타입으로
/// 강제한다 — 심볼릭 생성이 성공해도 **측정한 셸의 PATH 1순위가 우리 링크임을 확인했을 때만**
/// installed 다.
///
/// ★MAJOR-4(2026-08-25) 문구 정직화: 예전 주석·경고문은 "PATH 1순위"·"로그인 셸 PATH" 라는 **전칭**
/// 주장을 했지만 실제로 잰 것은 `bash -lc` 하나였다(macOS 10.15+ 기본 로그인 셸은 zsh 다). 이제
/// 무엇으로 쟀는지를 `shell_name` 으로 받아 문구에 밝힌다 — 잰 적 없는 것을 잰 척하지 않는다.
///
/// ★MINOR-9(2026-08-25) **unverified 두 분기의 신호**: `unverified` 는 원인이 둘인데 예전에는
/// UI 가 그것을 구분할 방법이 없어 "검증 명령 실패 또는 응답 없음" 으로 단정 오안내했다.
/// status 는 계약(3값)이라 늘리지 않고 **경고문 첫 구절을 안정된 판별자로** 고정한다:
/// - `"PATH 확인 결과: …"` → 측정은 **정상**이었고 그 셸 PATH 에서 cys 를 못 찾았다(원인=PATH 구성).
/// - `"PATH 확인 실패: …"`  → 검증 명령 자체를 못 돌렸다(실행 실패·타임아웃. 사유 동봉).
///   UI·문서는 이 두 접두로 문구를 갈라야 한다.
///
/// - `installed`          : which -a cys 1순위 == target_cys
/// - `installed_shadowed` : 링크는 생겼으나 앞을 가리는 다른 cys 가 있다(사용자가 치는 cys 는 그쪽)
/// - `unverified`         : 위 두 분기 — 어느 쪽도 '설치 완료'로 올리지 않는다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn classify_install_status(
    probe: &WhichProbe,
    target_cys: &str,
    shell_name: &str,
) -> InstallVerdict {
    match probe {
        WhichProbe::Completed(entries) => match entries.first() {
            Some(first) if first == target_cys => InstallVerdict {
                status: "installed",
                effective_cys: Some(first.clone()),
                shadowed_by: None,
                warnings: vec![],
            },
            Some(first) => InstallVerdict {
                status: "installed_shadowed",
                effective_cys: Some(first.clone()),
                shadowed_by: Some(first.clone()),
                warnings: vec![format!(
                    "PATH 확인 결과: 심볼릭은 만들었지만 로그인 셸({shell_name}) PATH 앞쪽의 다른 cys가 \
우선합니다: {first}. 새 터미널에서 'cys'를 치면 {target_cys}가 아니라 그쪽이 실행됩니다 — \
그 파일을 지우거나 PATH에서 /usr/local/bin을 앞으로 옮긴 뒤 다시 확인하세요."
                )],
            },
            None => InstallVerdict {
                status: "unverified",
                effective_cys: None,
                shadowed_by: None,
                warnings: vec![format!(
                    "PATH 확인 결과: 검증 명령은 정상 실행됐지만 로그인 셸({shell_name}) PATH에서 cys를 \
찾지 못했습니다(PATH에 {target_cys}의 폴더가 없을 수 있습니다). 새 터미널을 열어 \
'which -a cys'로 확인하세요."
                )],
            },
        },
        WhichProbe::Unmeasured(reason) => InstallVerdict {
            status: "unverified",
            effective_cys: None,
            shadowed_by: None,
            warnings: vec![format!(
                "PATH 확인 실패: 심볼릭은 만들었지만 로그인 셸({shell_name})로 'which -a cys'를 \
실행하지 못했습니다: {reason}. 새 터미널에서 'which -a cys'로 직접 확인하세요."
            )],
        },
    }
}

#[derive(serde::Serialize)]
struct InstallCliReport {
    /// status 파생값이다 — **두 개의 진실을 만들지 않는다**. 부분 성공(그림자·측정불능)은 false.
    ok: bool,
    /// (D3) 등급. 정확히 "installed" | "installed_shadowed" | "unverified" 셋 중 하나.
    /// UI 는 이 값으로 토스트 등급을 나눈다 — installed 만 성공이고 나머지 둘은 경고다.
    /// serde rename 없음 = JS 에서 `r.status`(snake_case 그대로).
    status: String,
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

        // (BLOCK-1) 백업 이름은 Rust 가 먼저 확정한다 — 셸에서 date 를 부르면 실제 파일명과
        // 사용자에게 보고하는 이름이 갈라질 수 있다.
        let stamp = backup_stamp(std::time::SystemTime::now());
        let plan = plan_cli_install(&macos_dir, target_dir, &stamp)?;
        if !plan.cys_src.exists() || !plan.cysd_src.exists() {
            return Err("번들 내 cys/cysd 바이너리를 찾지 못했습니다.".into());
        }

        // (BLOCK-1c) 승격 **전** 관측. 사후에 보면 이미 심볼릭으로 바뀐 뒤라 "원래 무엇이 있었는지"를
        // 알 수 없다. 여기서 잡은 것만이 스크립트가 백업으로 옮길 대상이다.
        let pre_probes: Vec<LinkProbe> = ["cys", "cysd"]
            .iter()
            .map(|n| probe_link(&format!("{target_dir}/{n}")))
            .collect();
        let expected_backups = plan_install_backups(&pre_probes, &stamp);

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

        // 검증: **사용자의 로그인 셸($SHELL, 없으면 bash) 기준** which -a cys — 전 시스템 PATH 가
        // 아니라 "그 셸에서 무엇이 잡히는가" 하나만 잰다(MAJOR-4). **측정 실패를 성공으로 접지 않는다**(D3) —
        // 예전 코드는 `.ok()` + `unwrap_or_default()` 로 실행 실패를 빈 목록으로 삼켜 '그림자 없음'과
        // 구분이 불가능했고, 그 상태로 ok:true "설치 완료" 토스트가 나갔다. 기한 5초(D6):
        // 로그인 셸 rc 가 매달려도 버튼은 반드시 돌아온다.
        let target_cys = format!("{target_dir}/cys");
        // (MAJOR-4) 사용자가 **실제로 쓰는** 셸로 잰다. macOS 10.15+ 기본 로그인 셸은 zsh 이므로
        // bash 로 재면 사용자의 PATH 가 아니라 '설치돼 있지도 않을 수 있는 다른 셸'의 PATH 를 재게
        // 되고, 그 결과로 나온 판정을 "PATH 1순위" 라고 부르는 것은 거짓이 된다.
        let login_shell = std::env::var("SHELL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/bin/bash".to_string());
        let shell_name = std::path::Path::new(&login_shell)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| login_shell.clone());
        let probe = match run_capture_with_timeout(
            &login_shell,
            &["-lc", "which -a cys"],
            std::time::Duration::from_secs(5),
        ) {
            // which 는 못 찾으면 비정상 종료하고 아무것도 출력하지 않는다 — 그건 '측정 실패'가 아니라
            // '측정 결과 없음'이므로 Completed(빈 목록)으로 넘겨 unverified 로 떨어지게 한다.
            Ok((_, stdout)) => WhichProbe::Completed(parse_which_a(&stdout)),
            Err(reason) => WhichProbe::Unmeasured(reason),
        };
        let verdict = classify_install_status(&probe, &target_cys, &shell_name);

        let mut warnings = plan.warnings;
        // (BLOCK-1c) 백업이 정말로 생겼는지 **재관측**해서 보고한다 — 스크립트의 자기신고를 믿지 않는다.
        // 사용자는 자기 파일이 어디로 갔고 어떻게 되돌리는지를 반드시 알아야 한다.
        for (orig, bak) in &expected_backups {
            if std::path::Path::new(bak).exists() {
                warnings.push(format!(
                    "{orig}에 심볼릭이 아닌 파일이 있어 지우지 않고 {bak}로 백업한 뒤 링크를 만들었습니다. \
되돌리려면 'sudo mv {bak} {orig}', 필요 없으면 'sudo rm {bak}' 를 실행하세요."
                ));
            } else {
                warnings.push(format!(
                    "{orig}의 기존 파일을 {bak}로 백업하려 했으나 백업본을 확인하지 못했습니다 — \
'ls -l {orig}'로 현재 상태를 직접 확인하세요."
                ));
            }
        }
        warnings.extend(verdict.warnings);

        Ok(InstallCliReport {
            ok: verdict.status == "installed",
            status: verdict.status.to_string(),
            target_dir: target_dir.to_string(),
            cys_link: target_cys,
            cysd_link: format!("{target_dir}/cysd"),
            source_cys: plan.cys_src.to_string_lossy().to_string(),
            effective_cys: verdict.effective_cys,
            shadowed_by: verdict.shadowed_by,
            warnings,
        })
    }
}

// ── CLI PATH 해제(비가역) — 관측/판정 순수 헬퍼 ─────────────────
/// 링크 한 경로의 파일시스템 관측값. **순수 판정부는 이 값만 본다** — 판정 안에서 파일시스템을
/// 다시 조회하면(`exists()`·`canonicalize()`) 대상이 사라진 **dangling 심볼릭**에서 판정이 실패한다.
/// 그런데 그게 바로 청소가 가장 필요한 경우다(앱을 이미 지웠고 root 소유 링크만 남은 상태). 그래서
/// 관측(symlink_metadata·read_link)과 판정을 갈라 두고, 판정은 문자열/불리언만 본다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone)]
struct LinkProbe {
    path: String,
    /// symlink_metadata 성공 여부 — 링크 자체를 보므로 dangling 링크도 true 다.
    present: bool,
    is_symlink: bool,
    /// read_link 결과. 대상 파일이 없어도 경로 문자열은 읽힌다.
    link_target: Option<String>,
}

/// 해제 판정 결과(순수). **비가역 삭제**이므로 '지운다'는 결론은 두 가드를 모두 통과할 때만 나온다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(PartialEq, Debug)]
enum UninstallAction {
    /// 심볼릭 + 대상이 cys.app 번들 안의 cys/cysd → 제거해도 되는 우리 링크.
    Remove,
    /// 경로가 없다 — 이미 해제된 상태(할 일 없음).
    SkipAbsent,
    /// ★일반 파일·디렉터리 — 절대 건드리지 않는다. 다른 도구(Homebrew·수동 빌드)가 설치한 실체
    /// 바이너리를 지우는 것은 남의 설치본 파괴이고 되돌릴 수 없다.
    SkipNotSymlink,
    /// 심볼릭이지만 대상이 cys.app 번들 밖 — 우리가 만든 링크가 아니므로 남긴다.
    SkipForeignTarget,
}

/// 심볼릭 대상이 **cys.app 번들 안의 cys/cysd** 인가(순수). 문자열만 본다 — 대상이 이미 삭제된
/// dangling 링크에서도 판정이 서야 하기 때문이다. `cys.app.bak-*`·`*.prev*` 나 타 앱 번들
/// (`Other.app/Contents/MacOS/cys`)은 마커가 어긋나 자동으로 거부된다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn links_into_cys_bundle(target: &str) -> bool {
    const MARK: &str = "/cys.app/Contents/MacOS/";
    match target.split_once(MARK) {
        Some((_, tail)) => tail == "cys" || tail == "cysd",
        None => false,
    }
}

/// 한 경로의 해제 판정(순수). 가드는 둘이고 순서가 곧 안전성이다: ①심볼릭이 아니면 즉시 포기
/// ②심볼릭이어도 대상이 우리 번들이 아니면 포기.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn decide_cli_uninstall(p: &LinkProbe) -> UninstallAction {
    if !p.present {
        return UninstallAction::SkipAbsent;
    }
    if !p.is_symlink {
        return UninstallAction::SkipNotSymlink;
    }
    match p.link_target.as_deref() {
        Some(t) if links_into_cys_bundle(t) => UninstallAction::Remove,
        _ => UninstallAction::SkipForeignTarget,
    }
}

/// `do shell script` 본문: 확인된 심볼릭만 **경로를 하나씩 박아** 제거한다. 경로 전개(와일드카드)·
/// `rm -r` 없음 — 삭제 범위가 새는 사고를 구조적으로 막는다. 인용은 설치와 같은 sh_squote 재사용.
///
/// ★MAJOR-2(2026-08-25) **TOCTOU 봉합**: `probe_link` → `decide_cli_uninstall` 은 비특권 **사전
/// 관측**이고 실제 집행은 root 다. 그 사이에는 사용자가 관리자 비밀번호를 치는 **시간 제한 없는
/// 창**이 있다 — 그 창에서 링크가 실체 파일로 바뀌거나 대상이 남의 경로로 바뀌면 예전 본문
/// (`rm -f <경로>`)은 그것을 그대로 root 권한으로 지웠다. 비특권 관측은 특권 집행의 가드가 될 수
/// 없다. 그래서 **스크립트 자신이 집행 직전에 다시 검사한다**:
///   ① `[ -L ]` — 심볼릭이 아니면 지우지 않는다(남의 실체 파일 보호. SkipNotSymlink 와 같은 규칙)
///   ② `readlink` 결과가 `*/cys.app/Contents/MacOS/{cys,cysd}` 가 아니면 지우지 않는다
/// 두 조건은 Rust 순수 판정 `links_into_cys_bundle` 과 **같은 마커**를 본다 — 한쪽만 고치면 안 된다.
/// `case` 안의 `*` 는 파일명 전개가 아니라 **문자열 패턴 대조**다(rm 인자는 여전히 리터럴 인용 경로).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn build_uninstall_script(paths: &[String]) -> String {
    paths
        .iter()
        .map(|p| {
            let q = sh_squote(p);
            format!(
                "if [ -L {q} ]; then case \"$(readlink {q})\" in \
*/cys.app/Contents/MacOS/cys|*/cys.app/Contents/MacOS/cysd) rm -f {q};; esac; fi"
            )
        })
        .collect::<Vec<String>>()
        .join("; ")
}

/// 해제 계획(순수): 관측 목록 → 제거 대상 + 건너뛴 사유(사용자 고지용) + osascript 인자.
/// 제거 대상이 하나도 없으면 `osascript_arg` 는 None 이다 — 지울 것도 없는데 관리자 프롬프트를
/// 띄우는 '헛 승격'을 막는다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct CliUninstallPlan {
    remove: Vec<String>,
    skipped: Vec<String>,
    osascript_arg: Option<String>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn plan_cli_uninstall(probes: &[LinkProbe]) -> CliUninstallPlan {
    let mut remove: Vec<String> = vec![];
    let mut skipped: Vec<String> = vec![];
    for p in probes {
        match decide_cli_uninstall(p) {
            UninstallAction::Remove => remove.push(p.path.clone()),
            UninstallAction::SkipAbsent => {
                skipped.push(format!("{} — 없음(이미 해제된 상태)", p.path));
            }
            UninstallAction::SkipNotSymlink => {
                skipped.push(format!(
                    "{} — 심볼릭이 아니라 실제 파일입니다. 다른 도구가 설치한 것일 수 있어 건드리지 않았습니다.",
                    p.path
                ));
            }
            UninstallAction::SkipForeignTarget => {
                skipped.push(format!(
                    "{} — cys.app 번들이 아닌 곳({})을 가리키는 링크라 건드리지 않았습니다.",
                    p.path,
                    p.link_target.as_deref().unwrap_or("대상 읽기 실패")
                ));
            }
        }
    }
    // AppleScript `do shell script` 는 큰따옴표 리터럴을 요구한다(작은따옴표면 파스 단계 -2741).
    // 설치 경로와 동일한 규약 — 바깥은 applescript_str, 내부 경로는 sh_squote.
    let osascript_arg = if remove.is_empty() {
        None
    } else {
        Some(format!(
            "do shell script {} with administrator privileges",
            applescript_str(&build_uninstall_script(&remove))
        ))
    };
    CliUninstallPlan {
        remove,
        skipped,
        osascript_arg,
    }
}

/// 설치 상태(순수): 두 링크 관측 → UI 버튼 라벨을 가르는 상태 하나. 판정은 해제 판정을 그대로
/// 재사용한다 — '해제' 라벨은 **실제로 지울 것이 있을 때만** 떠야 하기 때문이다(라벨과 행동 일치).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(PartialEq, Debug)]
enum CliLinkState {
    /// 우리 링크가 하나도 없다 → "셸에 cys 설치"
    Absent,
    /// cys·cysd 둘 다 우리 번들 심볼릭 → "셸 cys 해제"
    Ours,
    /// 한쪽만 우리 것(중단된 설치·부분 삭제 잔재) → 해제로 청소 가능
    Partial,
    /// 파일은 있으나 우리 것이 아니다(실체 파일·타 대상 링크) → 설치 라벨 유지 + 고지
    Foreign,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn classify_cli_links(probes: &[LinkProbe]) -> CliLinkState {
    let ours = probes
        .iter()
        .filter(|p| decide_cli_uninstall(p) == UninstallAction::Remove)
        .count();
    let foreign = probes
        .iter()
        .filter(|p| {
            matches!(
                decide_cli_uninstall(p),
                UninstallAction::SkipNotSymlink | UninstallAction::SkipForeignTarget
            )
        })
        .count();
    match (ours, foreign) {
        (0, 0) => CliLinkState::Absent,
        (0, _) => CliLinkState::Foreign,
        (n, _) if n == probes.len() => CliLinkState::Ours,
        _ => CliLinkState::Partial,
    }
}

/// 파일시스템 관측(얇은 래퍼 — 판정 없음). symlink_metadata 는 링크 자체를 보므로 대상이 사라진
/// dangling 링크도 present=true 로 잡힌다. `/usr/local/bin` 자체가 없는 기계(클린 macOS)에서도
/// Err 가 아니라 present=false 로 매끄럽게 떨어진다.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn probe_link(path: &str) -> LinkProbe {
    let p = std::path::Path::new(path);
    let md = std::fs::symlink_metadata(p).ok();
    let is_symlink = md
        .as_ref()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    LinkProbe {
        path: path.to_string(),
        present: md.is_some(),
        is_symlink,
        link_target: if is_symlink {
            std::fs::read_link(p)
                .ok()
                .map(|t| t.to_string_lossy().to_string())
        } else {
            None
        },
    }
}

#[derive(serde::Serialize)]
struct UninstallCliReport {
    /// 계획한 제거가 전부 실측으로 확인됐는가(지울 것이 없었던 경우도 true).
    ok: bool,
    removed: Vec<String>,
    /// "경로 — 사유" 형식. 왜 안 지웠는지를 사용자가 읽을 수 있어야 한다.
    skipped: Vec<String>,
    warnings: Vec<String>,
}

/// (D4a) 명시 메뉴 트리거. `/usr/local/bin` 의 cys·cysd 심볼릭을 osascript 1회 승격으로 제거한다.
/// **비가역**이므로 순수 판정(plan_cli_uninstall)이 Remove 로 결론낸 경로만, 이름을 박아 지운다.
/// 앱만 지우고 root 소유 링크가 남아 죽은 명령이 PATH 에 눌러앉던 결함(설치는 있는데 해제가 없었다)
/// 의 수리다.
#[tauri::command]
fn uninstall_cli_from_path() -> Result<UninstallCliReport, String> {
    #[cfg(not(target_os = "macos"))]
    {
        return Err("이 기능은 macOS 전용입니다.".into());
    }
    #[cfg(target_os = "macos")]
    {
        let target_dir = "/usr/local/bin";
        let probes: Vec<LinkProbe> = ["cys", "cysd"]
            .iter()
            .map(|n| probe_link(&format!("{target_dir}/{n}")))
            .collect();
        let plan = plan_cli_uninstall(&probes);
        let Some(arg) = plan.osascript_arg.clone() else {
            // 지울 것이 없다 = 이미 해제됐거나 우리 링크가 아니다. 승격 프롬프트를 띄우지 않는다.
            return Ok(UninstallCliReport {
                ok: true,
                removed: vec![],
                skipped: plan.skipped,
                warnings: vec![],
            });
        };

        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&arg)
            .output()
            .map_err(|e| format!("osascript 실행 실패: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("-128") || err.contains("User canceled") {
                return Err("해제가 취소되었습니다.".into());
            }
            return Err(format!("심볼릭 제거 실패: {}", err.trim()));
        }

        // 사후 재관측 — 산출자의 자기신고를 믿지 않는다. 정말 사라졌을 때만 removed 로 보고한다.
        let mut removed: Vec<String> = vec![];
        let mut warnings: Vec<String> = vec![];
        for path in &plan.remove {
            if probe_link(path).present {
                warnings.push(format!(
                    "{path} 가 아직 남아 있습니다 — 터미널에서 'sudo rm {path}' 로 지우세요."
                ));
            } else {
                removed.push(path.clone());
            }
        }
        Ok(UninstallCliReport {
            ok: warnings.is_empty(),
            removed,
            skipped: plan.skipped,
            warnings,
        })
    }
}

#[derive(serde::Serialize)]
struct CliInstallStatusReport {
    /// macOS 전용 기능이다. UI 는 이 값 하나로 버튼 노출 여부를 정할 수 있다(false 면 숨김).
    platform_supported: bool,
    /// true 면 UI 라벨은 '해제', false 면 '설치'.
    installed: bool,
    /// "absent" | "ours" | "partial" | "foreign" | "unsupported".
    state: String,
    cys_link: String,
    cysd_link: String,
    /// 설치도 해제도 아닌 상태(실체 파일·타 대상 링크)의 사유 — 사용자 고지용.
    notes: Vec<String>,
}

/// (D4b) 읽기전용 상태 조회. **승격하지 않는다** — 심볼릭 메타데이터만 본다. UI 는 Control Center
/// 를 열 때 1회, 설치·해제 직후 1회만 호출한다(폴링 금지 — 타이머 증식 차단 원칙).
/// non-macOS 에서 Err 를 던지지 않는 이유: CC 를 열 때마다 실패 토스트가 뜨기 때문이다. 대신
/// platform_supported=false 로 답한다(install/uninstall 쪽 non-macOS Err 는 심층방어로 존치).
#[tauri::command]
fn cli_install_status() -> Result<CliInstallStatusReport, String> {
    let target_dir = "/usr/local/bin";
    let cys_link = format!("{target_dir}/cys");
    let cysd_link = format!("{target_dir}/cysd");
    #[cfg(not(target_os = "macos"))]
    {
        return Ok(CliInstallStatusReport {
            platform_supported: false,
            installed: false,
            state: "unsupported".into(),
            cys_link,
            cysd_link,
            notes: vec![],
        });
    }
    #[cfg(target_os = "macos")]
    {
        let probes: Vec<LinkProbe> = vec![probe_link(&cys_link), probe_link(&cysd_link)];
        let state = classify_cli_links(&probes);
        let notes: Vec<String> = probes
            .iter()
            .filter_map(|p| match decide_cli_uninstall(p) {
                UninstallAction::SkipNotSymlink => Some(format!(
                    "{} — 심볼릭이 아닌 실제 파일이 이미 있습니다(다른 도구 설치본일 수 있어 자동으로 제거하지 않습니다).",
                    p.path
                )),
                UninstallAction::SkipForeignTarget => Some(format!(
                    "{} — cys.app 번들 밖({})을 가리키는 링크입니다.",
                    p.path,
                    p.link_target.as_deref().unwrap_or("대상 읽기 실패")
                )),
                _ => None,
            })
            .collect();
        Ok(CliInstallStatusReport {
            platform_supported: true,
            installed: matches!(state, CliLinkState::Ours | CliLinkState::Partial),
            state: match state {
                CliLinkState::Absent => "absent",
                CliLinkState::Ours => "ours",
                CliLinkState::Partial => "partial",
                CliLinkState::Foreign => "foreign",
            }
            .to_string(),
            cys_link,
            cysd_link,
            notes,
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
    // ★U-19 도달성 앵커(2026-08-24): 인앱 업데이트는 **항상** `--no-install-hook` 이다. 즉
    //   `pack.rs setup_isolated_config_dir` 의 `if !install_hooks { return; }` **아래**에 놓인
    //   시드·치유는 **업데이트로 올라온 사용자 전원에게 영영 도달하지 않는다**(신규 설치에만
    //   닿는다 = `agents.json` 값 수정이 기존 기계에 안 닿는 K-1 과 같은 계열의 도달성 결함).
    //   첫기동 관문 시드(`seed_first_run_gates`)는 그래서 훅과 **독립 플래그**로 제어되며 그
    //   조기 return **위**에 있다 — 순서가 뒤집히면 H-SEED-U19 가 적색이 된다.
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

/// ★GUI 오퍼레이터 승인(오너 2026-07-15 · R4 2026-08-02 소켓 인지 확장):
/// **지정한 소켓의 데몬**이 쓴 state 디렉토리에서 operator.token 을 읽는다.
/// cysd `state::state_dir`(RC-13) 미러 — unix=소켓의 부모 디렉토리,
/// windows=%LOCALAPPDATA%\cys(기본 데몬) 또는 그 하위 pipe 슬러그 디렉토리(부서 데몬).
///
/// ★부서 데몬 분기가 R4 에서 필요해진 이유: `send_input` 은 부서 워크스페이스 pane 에도 쓰이는데
/// (`socket: Some(...)`), 기본 데몬의 토큰을 붙이면 부서 데몬에서 **불일치**가 되어 오너 GUI
/// 키 입력이 배달 원장에 기록된다(불변식 ② 이탈). 토큰은 반드시 그 pane 이 붙은 데몬의 것이어야 한다.
/// 매 호출 신선 재독(캐시 금지) — 데몬 재시작(churn)마다 토큰이 재발급되기 때문.
/// 부재·빈 파일=None(구 데몬 호환 — 첨부 없이 호출).
fn read_operator_token_for(socket: &std::path::Path) -> Option<String> {
    #[cfg(windows)]
    let dir = {
        // cysd state::pipe_slug 미러: `\\.\pipe\<name>` 의 마지막 컴포넌트에서 파일시스템
        // 안전 문자만 남긴다. 기본 데몬(`cys`)은 루트 유지, 부서는 슬러그 하위(격리).
        let root = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").ok()?).join("cys");
        let last = socket
            .to_string_lossy()
            .rsplit(|c| c == '\\' || c == '/')
            .next()
            .unwrap_or("")
            .to_string();
        let slug: String = last
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if slug.is_empty() || slug == "cys" {
            root
        } else {
            root.join(slug)
        }
    };
    #[cfg(not(windows))]
    let dir = socket.parent()?.to_path_buf();
    let tok = std::fs::read_to_string(dir.join("operator.token")).ok()?;
    let tok = tok.trim().to_string();
    (!tok.is_empty()).then_some(tok)
}

/// 기본 데몬(feed_reply 전용) 토큰 — 소켓 인지 판을 기본 소켓으로 부른다(사본 금지).
fn read_operator_token() -> Option<String> {
    read_operator_token_for(&default_socket())
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
    // ~/Applications)만** 자동등록한다. 명시 사용자설치(plan_cli_install)와 **같은 기준**이다
    // — D5(2026-08-23)로 저쪽도 NonStandard 를 거부하면서 판정이 수렴했다: 휘발/이동 경로 — Translocated
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
/// 주입 수혜). 타 OS는 exe_dir만 얹혀 사실상 무영향(제거 없음).
///
/// ★SEAL-1(2026-08-01 실사고): PYTHONDONTWRITEBYTECODE 도 함께 얹는다. **이 함수가
/// 하는 일이 곧 "자식이 번들 python 을 쓰게 만드는 것"**이므로, 번들 python 이 자기 번들에
/// `__pycache__/*.pyc` 를 써서 코드서명 봉인을 깨는 경로와 호출부 집합이 정확히 같다
/// (직스폰 python3 + bash 경유로 python 을 부르는 곳 전부). 호출부마다 한 줄씩 더하면
/// 새 스폰이 생길 때 또 빠진다 — 배선의 단일 지점에 둔다. 근거·대안 비교는 lib.rs
/// `ENV_PY_NO_BYTECODE` 주석. python 이 아닌 자식(cys/bash)에게는 무해한 무시 변수다.
///
/// ★W-B2(감사 blocker #4 의 GUI 절반): 손수 PATH·SEAL-1 두 키만 얹던 이 함수를 pane 스폰
/// (state.rs)·스케줄 발화(schedule.rs)와 **같은 공용 규약**(`cys::spawn_env_pairs_from_process`)
/// 소비로 교체한다 — GUI 직스폰 env 키 집합이 pane 스폰 runtime 규약 키 집합의 **상위집합**이
/// 된다(회귀 핀 `gui_spawn_env_matches_pane_spawn_env`). 종전 누락 2종이 이걸로 닫힌다:
///   · PYTHONUTF8=1 — 한국어 Windows(cp949)에서 GUI 직스폰 부트 체인(`javis_orchestra.py
///     check`)이 '✓' 한 글자에 UnicodeEncodeError 로 즉사하던 무보호 경로. pane 경로는
///     state.rs literal(RC-6)로 이미 막혀 있었고, **훅이 죽은 사용자가 정확히 이 GUI 직스폰
///     경로로 몰리므로** 두 결함이 같은 사람에게 겹쳤다.
///   · HOME←USERPROFILE backfill — Windows 비로그인 bash.exe 자식의
///     `${CYS_PACK_DIR:-$HOME/.cys/pack}` 이 `/.cys/pack` 으로 붕괴하던 경로(W1a 와 동일
///     기제의 GUI 절반 · HOME 이 이미 있으면 무접촉이라 mac/unix 는 무변경).
///
/// 순서 계약: 무조건 쌍 2종(SEAL-1·UTF-8)을 규약 **앞에** 상수로 한 번 더 얹는다 —
/// current_exe 실패(이론상)에도 이 둘만은 잃지 않는 방어선이다. `spawn_env_pairs` 가 같은
/// 키를 다시 얹지만 std `Command` env 는 맵이라 나중 주입이 이기고 값이 동일("1")해 무해하다.
/// 호출부들이 이 함수 **뒤에** 거는 CYS_SOCKET/CYS_ROLE 등 명시 env(env/env_remove)는 규약과
/// 키가 겹치지 않아 종전 순서 의미가 그대로 보존된다.
fn inject_runtime_path(cmd: &mut std::process::Command) {
    cmd.env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON);
    cmd.env(cys::ENV_PY_UTF8, cys::PY_UTF8_ON);
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    {
        // pane(state.rs)·스케줄(schedule.rs)과 동일 SOT — PATH 선두주입·HOME backfill·
        // SEAL-1·UTF-8 이 한 규약에서 나온다(사본 금지 · 검체 H-WIN-8 의 GUI 확장).
        for (k, v) in cys::spawn_env_pairs_from_process(&exe_dir) {
            cmd.env(k, v);
        }
    }
}

/// Ensure aitermd is running: try to connect, otherwise spawn the bundled/sibling binary.
async fn ensure_daemon() -> Result<(), String> {
    if connect().await.is_ok() {
        return Ok(());
    }
    // ★A3(성찰 확정): 완전 초기화 진행 중에는 데몬을 스폰하지 않는다 — 이 경로가 리셋 도중
    // cysd 를 되살리는 주 통로였다(부트 재시도 루프·재시작·drain 사이드카가 모두 여기로 온다).
    // fail-open 판정(TTL·pid 생존)이라 리셋이 비정상 종료해도 다음 기동은 정상이다.
    if cys::factory_reset::reset_in_progress() {
        return Err("완전 초기화가 진행 중 — 데몬 기동을 보류한다(완료 후 앱을 다시 실행하라)".into());
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
    // ★(U-11) 3분기: 성공 / **관문 보류**(pane 은 살아 있다) / 실패.
    //   보류에서 팀 부트를 이어가면 안 된다 — 체인이 그 pane 에 claim-role 을 귀속시키고 지침을
    //   주입하는데, 관문 창에 붙여넣는 순간 그 Return 이 실측상 면책 창의 `No, exit` 을 눌러
    //   마스터를 종료시킨다. 좌석은 이미 보존됐으니(닫지 않았다) 사람이 그 pane 에서 관문을
    //   통과시키면 그대로 쓴다 — 사용자에게는 그 처방만 올린다.
    if out.status.code() == Some(cys::EXIT_GATE_PENDING) {
        return Err(format!(
            "마스터 pane 은 떴고 프로세스도 살아 있으나 **첫기동 관문**에 갇혀 있습니다(pane 은 \
             닫지 않았습니다). 그 pane 에서 관문을 1회 통과시킨 뒤 다시 시작하세요 — 순서는 \
             테마 → 로그인방식 → OAuth → 폴더신뢰 → 면책 → 새기능안내이고, ★면책 창의 기본 \
             선택은 `No, exit` 이라 그대로 Enter 를 누르면 종료됩니다(아래 방향키 1회 뒤 Enter).\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
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

/// ★D4(v4 · 결정 D4): CEO 승격 드리프트 — [.pre-ceo 존재 ∧ md≠라이브 CEO_TEMPLATE] 여부.
/// 팔레트 'CEO 승격 재실행(템플릿 전진 적용)' 항목의 노출 게이트로, 템플릿 전진 릴리스 직후
/// "이미 승격된 md 가 구본화된" 상태를 감지한다(스펙 §3 R2·`.pristine` 등가 판정의 위경보 교정 짝).
/// 판정은 순수 함수로 분리(회귀 핀 대상) — 판독 불가(파일 부재·IO 실패)는 노출 억제(보수적 false):
/// 비정형 승격 상태의 진단·안내는 preflight C03 의 관할이지 팔레트가 아니다.
/// 경로 규약 = cys-dept `ceo_promote`(`$PACK_DEFAULT/directives/…`)와 동일 파일 쌍 · ceo_pending 관례
/// (동기 fn·프론트 온디맨드 조회 — 신규 타이머 금지).
fn ceo_drift_verdict(pre_ceo_exists: bool, md: Option<&[u8]>, template: Option<&[u8]>) -> bool {
    pre_ceo_exists && matches!((md, template), (Some(m), Some(t)) if m != t)
}

#[tauri::command]
fn ceo_promotion_drift() -> bool {
    let dir = cys::pack::pack_dir().join("directives");
    ceo_drift_verdict(
        dir.join("MASTER_DIRECTIVE.md.pre-ceo").exists(),
        std::fs::read(dir.join("MASTER_DIRECTIVE.md")).ok().as_deref(),
        std::fs::read(dir.join("CEO_TEMPLATE.md")).ok().as_deref(),
    )
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
    // ★(U-11) 부서장도 같은 3분기 — 보류 pane 에 팀 부트를 이어 붙이면 그 주입이 관문 창의
    //   Return 이 된다(= 부서장 사망). 좌석은 보존돼 있으니 처방만 올린다.
    if out.status.code() == Some(cys::EXIT_GATE_PENDING) {
        return Err(format!(
            "부서장 pane 은 떴고 프로세스도 살아 있으나 **첫기동 관문**에 갇혀 있습니다(pane 은 \
             닫지 않았습니다). 그 pane 에서 관문을 1회 통과시킨 뒤 다시 시작하세요 — ★면책 창의 \
             기본 선택은 `No, exit` 이라 그대로 Enter 를 누르면 종료됩니다(아래 방향키 1회 뒤 Enter).\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
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

/// ★완전 초기화(팩토리 리셋) 프리뷰 — 읽기 전용(쓰기 0). 코어·인벤토리는 CLI `cys factory-reset`
/// 과 동일한 `cys::factory_reset`(DESIGN-factory-reset.md) — GUI 는 표시·확인만 담당한다.
/// 라이선스·미등록 파일(오너 배치 *.env 등) 보존이 코어 계약이라 GUI 가 따로 지킬 것이 없다.
/// ★P0-2: 종전엔 집계 5개만 반환해 GUI 사용자가 **무엇이 사라지는지 볼 방법이 없었다**
/// (같은 기능의 CLI `--plan` 은 전 경로를 찍는다 — 정보 비대칭). 이제 전 항목·강조 표식·
/// report_only·사전점검·중단흔적·세션 수를 넘겨 모달이 승인 전에 다 보여준다.
/// 또 `fn` 이라 대용량 재귀 stat 동안 창이 굳었다 — 실행 커맨드와 같은 `spawn_blocking` 으로.
#[tauri::command]
async fn factory_reset_preview() -> Result<Value, String> {
    let live_sessions = live_session_count().await.unwrap_or(0);
    let dept_count = list_depts()
        .ok()
        .and_then(|r| r.get("depts").and_then(|d| d.as_object()).map(|o| o.len()))
        .unwrap_or(0);
    tokio::task::spawn_blocking(move || {
        let roots =
            cys::factory_reset::ResetRoots::live().ok_or("홈 디렉토리를 해석할 수 없다")?;
        let plan = cys::factory_reset::build_plan(
            &roots,
            &cys::factory_reset::ResetOptions {
                purge_license: false,
                purge_local: false,
                purge_round: false,
            },
        );
        Ok(json!({
            "quarantine_count": plan.quarantine.len(),
            "total_bytes": plan.quarantine_total_bytes(),
            "trash_dir": plan.trash_dir.to_string_lossy(),
            "quarantine": plan.quarantine.iter().map(|i| json!({
                "path": i.path.to_string_lossy(),
                "label": i.label,
                "size_bytes": i.size_bytes,
                "outside_state": i.outside_state,
            })).collect::<Vec<_>>(),
            "kept": plan.keep.iter().map(|k| json!({
                "path": k.path.to_string_lossy(), "label": k.label,
            })).collect::<Vec<_>>(),
            "strip_profiles": plan.strip_settings.len(),
            "report_only": plan.report_only,
            "live_sessions": live_sessions,
            "dept_count": dept_count,
            "trash_root_ready": plan.trash_root_ready.is_ok(),
            "trash_root_error": plan.trash_root_ready.as_ref().err(),
            "interrupted_prior": plan.interrupted_prior.iter()
                .map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// ★완전 초기화 실행 — 데몬 전멸(launchd 해제 우선·전멸 실측이 격리 하드 게이트) 후
/// cys-trash/factory-reset-<UTC>/ 로 격리 + manifest + 훅·statusLine·스킬 심링크 해제.
/// 진행은 `reset-progress` 이벤트({phase:"step",detail}), 결과는 반환 JSON(restore-progress 관례의
/// invoke-반환 변형 — 장시간 단계가 kill 대기 ~12s 뿐이라 3-phase emit 전부는 과함).
/// 실패는 Err 로 정직 표기(무음 삼킴 금지). 완료 후 앱 상태는 재시작 전까지 반쪽(데몬 없음)이므로
/// UI 는 곧장 종료 안내 모달로 이어야 한다(factory_reset_quit_app).
#[tauri::command]
async fn factory_reset_execute(
    app: AppHandle,
    purge_license: bool,
    purge_local: bool,
) -> Result<Value, String> {
    // ★가드 대칭(CLI 와 동일): pane 셸에서 앱을 띄웠다면 이 프로세스도 surface env 를 상속한다 —
    // 그 경우 리셋이 자기 세션을 끊으므로 거부한다(CLI run_factory_reset 과 같은 근거).
    if std::env::var("CYS_SURFACE_ID").map(|v| !v.is_empty()).unwrap_or(false) {
        return Err(
            "cys surface 안에서 기동된 앱에서는 완전 초기화를 실행할 수 없다 — 앱을 독립 실행하라"
                .into(),
        );
    }
    tokio::task::spawn_blocking(move || {
        let roots =
            cys::factory_reset::ResetRoots::live().ok_or("홈 디렉토리를 해석할 수 없다")?;
        let plan = cys::factory_reset::build_plan(
            &roots,
            &cys::factory_reset::ResetOptions {
                purge_license,
                purge_local,
                purge_round: false,
            },
        );
        // ★P0-6: 격리 목적지를 못 쓰는 상태면 데몬을 건드리기 전에 거부한다.
        if let Err(e) = &plan.trash_root_ready {
            return Err(format!("격리 폴더를 쓸 수 없어 초기화를 시작하지 않는다: {e}"));
        }
        let mut progress = |phase: &str, detail: &str| {
            let _ = app.emit("reset-progress", json!({"phase": phase, "detail": detail}));
        };
        // ★P0-1: RAII 센티널 — 조기 return·패닉에도 Drop 이 해제한다(잔존 시 데몬 기동 불가).
        let _sentinel = cys::factory_reset::ResetSentinel::arm();
        cys::factory_reset::stop_daemons_and_unregister(&plan, &mut progress).map_err(|e| {
            // ★P0-6: 정지 단계가 이미 남긴 비가역 부수효과를 숨기지 않는다.
            format!("{e}\n{}", cys::factory_reset::stop_side_effects_note())
        })?;
        let rep = cys::factory_reset::execute_quarantine(
            &plan,
            &roots,
            &cys::factory_reset::live_pid_is_cysd,
            &cys::factory_reset::live_any_cysd_running,
            &mut progress,
        )?;
        Ok(json!({
            "ok": rep.ok(),
            "trash_dir": rep.trash_dir.to_string_lossy(),
            "moved": rep.moved.len(),
            "failed": rep.failed.iter().map(|(p, e)| json!({
                "path": p.to_string_lossy(), "error": e,
            })).collect::<Vec<_>>(),
            "kept": rep.kept.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
            "stripped": rep.stripped,
            "revived_warning": rep.revived_warning,
            "deferred": rep.deferred.iter().map(|(p, e)| json!({
                "path": p.to_string_lossy(), "error": e,
            })).collect::<Vec<_>>(),
            "skipped_absent": rep.skipped_absent,
            "manifest_written": rep.manifest_written,
            "report_path": rep.trash_dir.join("REPORT.txt").to_string_lossy(),
            "interrupted_prior": rep.interrupted_prior.iter()
                .map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 완전 초기화 후 앱 종료 — `app.restart()` 는 single-instance 락 레이스(install_update 주석의
/// 재활성화 경고)가 있어 쓰지 않는다. 정직한 종료 + "다시 실행하면 온보딩" 안내가 계약.
#[tauri::command]
fn factory_reset_quit_app(app: AppHandle) {
    app.exit(0);
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
    // 2-b) ★설치 후 검증(ATOMIC-1 계약 ④의 대체 집행 · 2026-08-01 실사고).
    //   교체를 수행한 주체는 tauri-plugin-updater 이고 우리는 그 내부를 못 고친다. 실측된 결함:
    //   ⓐ `rename(현재 .app → TempDir)` 후 최종 rename 이 실패해도 **되돌리는 코드가 없다**
    //      (백업이 TempDir 이라 프로세스 종료와 함께 증발 → 앱 영구 소실),
    //   ⓑ 권한상승 폴백은 `rm -rf '<app>' && mv …` 로 **삭제가 먼저**다(이동 실패 = 앱 전소),
    //   ⓒ 교체 후 하는 일이 `touch` 뿐 — **검증이 0건**이다.
    //   우리가 덮을 수 있는 건 ⓒ다. 여기서 완본이 아니면 **재시작하지 않고 큰 소리로 실패**한다:
    //   깨진 번들로 재시작하면 다음 기동은 Gatekeeper 가 막아 사용자는 원인 없는 "손상되었기 때문에
    //   열 수 없습니다"만 보게 된다(무증상 성공 금지). 구 프로세스는 계속 살아 있으므로 사용자는
    //   최소한 안내를 읽고 재설치할 수 있다.
    #[cfg(target_os = "macos")]
    if let Some(msg) = bundle_integrity_guidance() {
        eprintln!("[cys-app] 업데이트 설치 후 검증 실패 — 재시작을 중단합니다\n{msg}");
        let _ = app.emit("bundle-damaged", msg.clone());
        return Err(msg);
    }
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
    // ★SEAL-1 층3: tauri 런타임(=스레드) 생성 전에 프로세스 env 봉인. GUI 는 이 앱의 **뿌리
    // 프로세스**라 여기서 심으면 cysd·pane·팩 python 까지 전부 상속으로 덮인다(lib.rs SOT).
    // `inject_runtime_path`(층1·2)는 그대로 둔다 — 그쪽은 명시 계약이라 회귀 핀이 걸린다.
    cys::seal_python_bytecode_in_process();
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
            ceo_promotion_drift,
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
            app_mouse_enabled,
            win_wheel_guard_disabled,
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
            factory_reset_preview,
            factory_reset_execute,
            factory_reset_quit_app,
            rotate_dept_daemon,
            list_depts,
            read_dept_catalog,
            install_cli_to_path,
            uninstall_cli_from_path,
            cli_install_status,
            app_version,
            boot_verdict,
            // ATOMIC-1 짝: 설치본이 '반쪽 번들'인지 기동 시 스스로 확인해 복구 절차를 준다.
            bundle_integrity,
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
                    // ★설치본 자기 무결성(ATOMIC-1 짝 · 2026-08-01 실사고): 정규 설치 위치인데도
                    // 구성요소가 빠진 '반쪽 번들'인지 확인한다. 위 안전모드와 달리 **부트를 멈추지
                    // 않는다** — 이유는 bundle_integrity_guidance 주석. 침묵만 깬다(사고의 실제 피해는
                    // 고장이 아니라 '아무도 말해주지 않은 것'이었다). stat 몇 번이라 부트 비용 무시 가능.
                    if let Some(msg) = bundle_integrity_guidance() {
                        eprintln!("[cys-app] 설치본 무결성 결손 — 재설치가 필요합니다\n{msg}");
                        let _ = handle.emit("bundle-damaged", msg);
                    }
                    // ★SEAL-DIAG(같은 사고의 다른 절반): 구조는 멀쩡한데 **봉인만** 깨진 사본을
                    // 스스로 발견해 알린다. 위 검사와 달리 codesign 이 초 단위라 별도 스레드로
                    // 내보낸다 — 이 호출은 즉시 반환하고 아래 부트 시퀀스는 조금도 기다리지 않는다
                    // (advisory 전용 · 어떤 판정도 기동을 막지 않는다). 버전당 1회 스로틀,
                    // 단 파손 확인 시에는 고쳐질 때까지 매 기동 다시 본다(spawn_seal_selfdiag).
                    spawn_seal_selfdiag(handle.clone());
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
                        // ★P1-3: 완전 초기화가 진행 중이면 데몬은 **일부러** 없는 것이다.
                        // 이 루프가 그걸 모르고 "로그인 항목을 허용하세요"라는 엉뚱한 처방을
                        // 완료 모달과 나란히 띄웠다. 사유만 바로잡고 **재시도 루프만 접는다** —
                        // ★여기서 return 하면 아래 event-forwarder·온보딩까지 통째로 건너뛰어
                        // 그 앱 세션의 이벤트 파이프가 재실행 전까지 영구 사망한다(감사 확정).
                        if cys::factory_reset::reset_in_progress() {
                            let _ = handle.emit(
                                "daemon-error",
                                "완전 초기화가 진행 중입니다 — 끝난 뒤 앱을 종료했다가 다시 실행하세요.".to_string(),
                            );
                            break;
                        }
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
                    // ★P1-3: 리셋 중이면 원인이 다르다 — 처방도 달라야 한다.
                    let msg = if cys::factory_reset::reset_in_progress() {
                        "완전 초기화가 진행 중입니다 — 끝난 뒤 앱을 종료했다가 다시 실행하세요.".to_string()
                    } else {
                        format!("{e} — 데몬을 시작하지 못했습니다. 시스템 설정 → 일반 → 로그인 항목에서 cys 백그라운드 항목을 허용한 뒤 앱을 다시 여세요.")
                    };
                    let _ = handle.emit("daemon-error", msg);
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

    /// ★SEAL-DIAG 스로틀 회귀 핀: **파손은 마커로 침묵시킬 수 없다.**
    /// 스로틀의 목적은 평시 `codesign --deep` 비용 절감이지 고장 은폐가 아니다 —
    /// 이 구분이 무너지면 첫 기동에 파손을 한 번 알리고 그 뒤로는 영원히 조용해진다.
    #[cfg(target_os = "macos")]
    #[test]
    fn seal_selfdiag_throttle_never_silences_a_broken_seal() {
        // 평시(무결·판정불가)는 버전당 1회로 접는다.
        assert!(seal_selfdiag_skips(Some("intact")));
        assert!(seal_selfdiag_skips(Some("undetermined")));
        assert!(seal_selfdiag_skips(Some("  intact\n")), "공백·개행 관용");

        // 마커 없음 = 아직 안 봤다 → 본다.
        assert!(!seal_selfdiag_skips(None));
        // ★파손 기록은 skip 대상이 아니다(애초에 기록하지 않지만, 손으로 심어도 다시 본다).
        assert!(!seal_selfdiag_skips(Some("broken")));
        // 미지의 값 = "모른다" → 건너뛰지 않고 다시 본다(fail-open toward checking).
        assert!(!seal_selfdiag_skips(Some("")));
        assert!(!seal_selfdiag_skips(Some("ok")));

        // 마커는 버전에 묶인다 — 업데이트되면 새 번들이므로 자동으로 다시 본다.
        assert!(seal_selfdiag_marker()
            .to_string_lossy()
            .ends_with(&format!("selfdiag-{}", env!("CARGO_PKG_VERSION"))));
    }

    /// ★SEAL-DIAG pull 캐시 회귀 핀 ①(F3 격차1): **Broken 만 적재 대상이고, 문구는
    /// push 가 쓰는 seal_broken_notice 산출물과 바이트 동일하다(이원화 0).**
    /// Intact/Undetermined 가 안내문을 만들면 정상 기동·개발 빌드에서 "재설치" 오보가
    /// pull 로 새고, 문구가 갈라지면 push 유실 기동에서 같은 고장이 두 얼굴을 갖는다.
    /// (전역 SEAL_BROKEN_CACHE 는 건드리지 않는다 — 병렬 테스트의
    ///  bundle_integrity_is_silent_outside_a_bundle… 가 빈 캐시를 전제하기 때문.)
    #[cfg(target_os = "macos")]
    #[test]
    fn seal_cache_stores_only_broken_and_speaks_the_canonical_notice() {
        use cys::app_bundle::SealVerdict;
        let bundle = std::path::Path::new("/Applications/cys.app");
        // 알릴 것 없는 판정 → 적재 없음(오보 차단).
        assert_eq!(seal_cache_payload(bundle, &SealVerdict::Intact), None);
        assert_eq!(
            seal_cache_payload(bundle, &SealVerdict::Undetermined("codesign 부재".into())),
            None
        );
        // Broken → 반드시 Some 이며, push 문구와 바이트 동일(이원화 금지 계약).
        let culprits = vec!["Contents/Resources/pack/__pycache__/x.pyc".to_string()];
        let got = seal_cache_payload(
            bundle,
            &SealVerdict::Broken {
                culprits: culprits.clone(),
                self_inflicted: true,
            },
        )
        .expect("Broken 판정은 항상 안내문을 산출해야 한다");
        assert_eq!(
            got,
            cys::app_bundle::seal_broken_notice(bundle, &culprits, true),
            "push(emit)와 pull(캐시) 문구가 갈라졌다 — 이원화 금지 계약 위반"
        );
    }

    /// ★SEAL-DIAG pull 캐시 회귀 핀 ②(F3 격차1): 합산은 **어느 파손 판정도 떨어뜨리지
    /// 않는다.** 둘 다 참이면 봉인 쪽을 돌려준다 — push 에서 codesign(초 단위)이 나중에
    /// 도착해 토스트를 덮는 순서의 pull 판 보존(덮어쓰기 의미론 계약 · 결론은 양쪽 다
    /// "재설치"라 안내 불일치 없음). 여기가 무너지면 push 유실 기동에서 봉인 파손이
    /// 구조 무결(None)에 가려 침묵한다.
    #[cfg(target_os = "macos")]
    #[test]
    fn bundle_integrity_merge_never_drops_a_damage_verdict() {
        let s = || Some("구조 결손 안내".to_string());
        let b = || Some("봉인 파손 안내".to_string());
        // 무고장 = 무음(오보 없음).
        assert_eq!(merge_integrity_pull(None, None), None);
        // 한쪽만 참 → 그 판정이 그대로 살아 나간다(어느 쪽도 소실 금지).
        assert_eq!(merge_integrity_pull(s(), None), s());
        assert_eq!(merge_integrity_pull(None, b()), b());
        // 둘 다 참 → 봉인(나중 도착)이 덮는다 — push 토스트 덮어쓰기 순서와 동일.
        assert_eq!(merge_integrity_pull(s(), b()), b());
    }

    /// ★D4 팔레트 노출 게이트 회귀 핀: 드리프트 = [.pre-ceo 존재 ∧ md≠라이브 CEO_TEMPLATE] **만**이다.
    /// 특히 R2 실측 교정 두 가지를 고정한다 — ⓐ md==템플릿(정상 승격 최신)은 .pre-ceo 가 있어도
    /// 비노출(위경보 0) ⓑ 판독 불가(부재·IO 실패)는 보수적 비노출(비정형 상태 안내는 C03 관할).
    #[test]
    fn ceo_drift_verdict_gate_matrix() {
        let old = Some("CEO v1".as_bytes());
        let new = Some("CEO v2".as_bytes());
        // 유일한 노출 케이스: 승격 상태(.pre-ceo)에서 템플릿 전진으로 md 가 구본화됨.
        assert!(ceo_drift_verdict(true, old, new));
        // 정상 승격 최신(md==템플릿) → 비노출.
        assert!(!ceo_drift_verdict(true, new, new));
        // 미승격(.pre-ceo 부재) → md 가 달라도 비노출(주권 편집·미승격 머신).
        assert!(!ceo_drift_verdict(false, old, new));
        // 판독 불가(어느 쪽이든) → 비노출(보수) — 템플릿 소실 시 promote-ceo 재실행 권유 금지.
        assert!(!ceo_drift_verdict(true, None, new));
        assert!(!ceo_drift_verdict(true, old, None));
        assert!(!ceo_drift_verdict(true, None, None));
    }

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

    // ★BLOCK-1(2026-08-25): 예전 본문은 `ln -sf` 하나였고, 그것은 대상이 일반 파일이어도 unlink 후
    // 심볼릭으로 갈아 끼운다 = 남의 실체 설치본을 말없이 파괴했다. 이제 백업(mv) 뒤에 링크를 만든다.
    #[test]
    fn build_install_script_backs_up_real_files_and_uses_ln_sfn() {
        let cys = std::path::Path::new("/Applications/cys.app/Contents/MacOS/cys");
        let cysd = std::path::Path::new("/Applications/cys.app/Contents/MacOS/cysd");
        let s = build_install_script(cys, cysd, "/usr/local/bin", "1700000000");
        assert_eq!(
            s,
            "mkdir -p '/usr/local/bin' && \
if [ -e '/usr/local/bin/cys' ] && [ ! -L '/usr/local/bin/cys' ]; \
then mv '/usr/local/bin/cys' '/usr/local/bin/cys.cys-backup-1700000000'; fi && \
ln -sfn '/Applications/cys.app/Contents/MacOS/cys' '/usr/local/bin/cys' && \
if [ -e '/usr/local/bin/cysd' ] && [ ! -L '/usr/local/bin/cysd' ]; \
then mv '/usr/local/bin/cysd' '/usr/local/bin/cysd.cys-backup-1700000000'; fi && \
ln -sfn '/Applications/cys.app/Contents/MacOS/cysd' '/usr/local/bin/cysd'"
        );
        // 백업(mv)이 링크 생성보다 **먼저** 와야 한다 — 순서가 뒤집히면 이미 파괴된 뒤다.
        assert!(
            s.find("mv '/usr/local/bin/cys'").unwrap()
                < s.find("ln -sfn '/Applications/cys.app/Contents/MacOS/cys'").unwrap(),
            "백업이 링크 생성보다 뒤에 있다: {s}"
        );
        // ★BSD ln: -n 이 없으면 대상이 '디렉터리를 가리키는 심볼릭'일 때 그 디렉터리 안에 링크를
        // 만든다 = root 권한 쓰기가 target_dir 밖으로 샌다.
        assert!(!s.contains("ln -sf '"), "ln -sf(-n 없음) 회귀: {s}");
        assert!(s.contains("ln -sfn "), "ln -sfn 이어야 한다: {s}");
        // 셸에서 date 를 부르지 않는다(스크립트 이름과 보고 문구가 갈라지면 안 된다).
        assert!(!s.contains("date"), "타임스탬프는 Rust 가 박는다: {s}");
    }

    // ★BLOCK-1 **실행** 검증: 문자열 단언만으로는 셸 의미론(ln -sf 가 일반 파일을 unlink 한다)을
    // 잡을 수 없다. 임시 디렉터리를 target_dir 로 삼아 실제 /bin/sh 로 돌려 결과를 관측한다.
    // (root 불필요 — 승격은 osascript 의 몫이고 스크립트 본문 의미론만 검사한다.)
    #[cfg(unix)]
    #[test]
    fn install_script_backs_up_foreign_file_instead_of_destroying_it() {
        let base = std::env::temp_dir().join(format!("cys-install-script-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let bin = base.join("bin");
        let src = base.join("src");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("cys"), "OURS").unwrap();
        std::fs::write(src.join("cysd"), "OURSD").unwrap();
        // 남의 설치본(Homebrew·수동 빌드)이 이미 **실체 파일**로 있다.
        std::fs::write(bin.join("cys"), "FOREIGN-BINARY").unwrap();

        let td = bin.to_string_lossy().to_string();
        let script = build_install_script(&src.join("cys"), &src.join("cysd"), &td, "T1");
        let st = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .status()
            .unwrap();
        assert!(st.success(), "설치 스크립트 실패: {script}");

        // ① 남의 파일은 사라지지 않고 백업으로 살아 있다(예전 `ln -sf` 는 여기서 파괴했다).
        let bak = bin.join("cys.cys-backup-T1");
        assert_eq!(
            std::fs::read_to_string(&bak).unwrap_or_default(),
            "FOREIGN-BINARY",
            "남의 실체 파일이 백업되지 않고 파괴됐다"
        );
        // ② 우리 링크가 생겼다.
        assert!(
            std::fs::symlink_metadata(bin.join("cys"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "심볼릭이 만들어지지 않았다"
        );
        // ③ 반복 설치는 멱등 — 이미 심볼릭이면 mv 분기가 돌지 않아 백업이 쌓이지 않는다.
        let script2 = build_install_script(&src.join("cys"), &src.join("cysd"), &td, "T2");
        assert!(std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&script2)
            .status()
            .unwrap()
            .success());
        assert!(
            !bin.join("cys.cys-backup-T2").exists(),
            "심볼릭 재설치가 백업을 쌓았다(멱등성 깨짐)"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // (BLOCK-1c) 사용자에게 무엇이 백업될지 보고하려면 Rust 가 스크립트와 **같은 판정**을 해야 한다.
    #[test]
    fn plan_install_backups_matches_shell_condition() {
        let ours = "/Applications/cys.app/Contents/MacOS/cys";
        let b = plan_install_backups(
            &[
                probe("/usr/local/bin/cys", true, false, None), // 실체 파일 → 백업
                probe("/usr/local/bin/cysd", true, true, Some(ours)), // 심볼릭 → 백업 없음(멱등)
            ],
            "S",
        );
        assert_eq!(
            b,
            vec![(
                "/usr/local/bin/cys".to_string(),
                "/usr/local/bin/cys.cys-backup-S".to_string()
            )]
        );
        // 부재 + dangling 심볼릭은 둘 다 백업 대상이 아니다(셸 `-e` 가 거짓인 경우와 일치).
        assert!(plan_install_backups(
            &[
                probe("/usr/local/bin/cys", false, false, None),
                probe("/usr/local/bin/cysd", true, true, None),
            ],
            "S"
        )
        .is_empty());
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

    // ★MINOR-7: stdout 잡음을 경로로 격상하면 목록 1순위가 잡음이 되어 installed 가
    // installed_shadowed 로 오판되고, 존재하지도 않는 "앞을 가리는 cys" 를 지우라고 안내하게 된다.
    #[test]
    fn parse_which_a_rejects_non_absolute_path_noise() {
        let out = "cys: aliased to /Applications/cys.app/Contents/MacOS/cys\n\
zsh: no such file or directory: cys\n\
/usr/local/bin/cys\n\
cys is a shell builtin\n\
\x20 /opt/homebrew/bin/cys  \n\
\n";
        assert_eq!(
            parse_which_a(out),
            vec![
                "/usr/local/bin/cys".to_string(),
                "/opt/homebrew/bin/cys".to_string(),
            ],
            "절대경로가 아닌 줄은 경로가 아니다"
        );
        // 잡음만 있는 출력은 빈 목록 → classify_install_status 가 unverified 로 떨어뜨린다.
        assert!(parse_which_a("cys not found\nsome rc banner\n").is_empty());
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
            "/usr/local/bin",
            "S"
        ).is_err());
        // backup → Err
        assert!(plan_cli_install(
            std::path::Path::new("/Applications/cys.app.bak-044/Contents/MacOS"),
            "/usr/local/bin",
            "S"
        ).is_err());
    }

    #[test]
    fn autoregister_allowed_only_canonical() {
        // 무음 launchd 자동등록은 Canonical 만 허용한다. 예전에는 plan_cli_install 이 NonStandard 를
        // 경고만 하고 허용해 의도적 divergence 였으나, D5(2026-08-23)로 저쪽도 거부하면서 판정이 수렴했다.
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

    /// ★ATOMIC-1 짝 회귀 핀(2026-08-01 실사고): 반쪽 번들 기동 점검이 **거짓 경보를 내지 않고**
    /// **진짜 결손은 이름으로 말하는지**를 커맨드 층위에서 고정한다.
    /// 실행 프로세스(test 하네스)는 번들 밖이라 `bundle_integrity()` 는 None 이어야 한다 —
    /// 여기서 Some 이 나오면 모든 개발 빌드가 손상 경보를 띄운다(가장 비싼 오탐).
    #[cfg(target_os = "macos")]
    #[test]
    fn bundle_integrity_is_silent_outside_a_bundle_and_names_defects_inside_one() {
        assert!(
            bundle_integrity().is_none(),
            "번들 밖 실행(개발 빌드·테스트 하네스)에서는 어떤 경보도 내면 안 된다"
        );
        // 번들 안 판정은 순수 함수 조합으로 검사한다(프로세스 exe 를 옮길 수는 없으므로).
        // 사고 상태(Info.plist 유실)를 그대로 만들어, 탐지가 꺼지지 않고 결손을 지목하는지 본다.
        let base = std::env::temp_dir().join(format!("cys-app-integ-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app = base.join("cys.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let exe = app.join("Contents/MacOS/cys-app");
        std::fs::write(&exe, b"x").unwrap();
        // Info.plist 없음 = 실사고 최종 상태.
        let found = cys::app_bundle::enclosing_bundle(&exe).expect("레이아웃만으로 번들을 지목해야 한다");
        let defects = cys::app_bundle::verify(&found, &cys::app_bundle::VerifySpec::installed());
        assert!(
            defects.contains(&cys::app_bundle::BundleDefect::InfoPlistMissing),
            "가장 파괴적인 손상이 '판정 불가'로 새면 안 된다: {defects:?}"
        );
        let msg = cys::app_bundle::damaged_bundle_guidance(&found, &defects);
        assert!(msg.contains("Info.plist") && msg.contains("휴지통"), "원인 + 복구 절차");
        let _ = std::fs::remove_dir_all(&base);
    }

    // ★D5(2026-08-23): NonStandard 는 '경고 후 진행'에서 **거부**로 승격됐다. root 소유 심볼릭이
    // 사용자 쓰기 가능한 임의 경로를 가리키는 상태를 만들지 않는다. 이 테스트는 그 승격을 못박는다.
    #[test]
    fn plan_cli_install_refuses_nonstandard_and_names_actual_path() {
        // CliInstallPlan 은 Debug 를 파생하지 않으므로(경로만 담는 내부 계획) match 로 받는다.
        let err = match plan_cli_install(
            std::path::Path::new("/Users/x/Downloads/cys.app/Contents/MacOS"),
            "/usr/local/bin",
            "S"
        ) {
            Ok(_) => panic!("nonstandard 는 거부되어야 한다 — 경고 후 진행으로 되돌아갔다"),
            Err(e) => e,
        };
        // 사용자가 원인을 알 수 있게 **실제 현재 경로**와 다음 조치를 함께 준다.
        assert!(err.contains("/Users/x/Downloads/cys.app"), "실제 번들 경로를 알려야 한다: {err}");
        assert!(err.contains("Applications"), "다음 조치(Applications로 이동) 안내가 있어야 한다: {err}");
    }

    // ★MINOR-6(2026-08-25): D5 의 거부 근거("root 링크가 사용자 쓰기 가능 경로를 가리키면 안 된다")는
    // classify_bundle_dir 의 ends_with("/Applications") 판정 때문에 실제로는 성립하지 않았다 —
    // /tmp/Applications · ~/Downloads/Applications 같은 **사용자가 직접 만드는** 디렉터리가 Canonical
    // 로 통과했다. classify_bundle_dir 은 autoregister·boot 게이트가 공유하므로 건드리지 않고,
    // 설치 전용 엄격 판정을 덧댄다.
    #[test]
    fn strict_install_bundle_ok_accepts_only_the_two_real_applications_dirs() {
        use std::path::Path;
        let home = Path::new("/Users/tester");
        assert!(strict_install_bundle_ok(
            Path::new("/Applications/cys.app/Contents/MacOS"),
            home
        ));
        assert!(strict_install_bundle_ok(
            Path::new("/Users/tester/Applications/cys.app/Contents/MacOS"),
            home
        ));
        // ★반례: classify_bundle_dir 은 이 셋을 전부 Canonical 로 본다(그게 결함이었다).
        for bad in [
            "/tmp/Applications/cys.app/Contents/MacOS",
            "/Users/x/Downloads/Applications/cys.app/Contents/MacOS",
            "/Applications/Utilities/Applications/cys.app/Contents/MacOS",
            "/Users/other/Applications/cys.app/Contents/MacOS",
        ] {
            assert_eq!(
                classify_bundle_dir(Path::new(bad)),
                BundleKind::Canonical,
                "전제 확인: 이 경로는 classify_bundle_dir 상 Canonical 이다 — {bad}"
            );
            assert!(
                !strict_install_bundle_ok(Path::new(bad), home),
                "사용자가 만들 수 있는 Applications 유사 경로는 거부되어야 한다: {bad}"
            );
        }
        // 번들 구조가 어긋난 입력도 거부(cys.app 이 아니거나 Contents/MacOS 가 아님).
        assert!(!strict_install_bundle_ok(
            Path::new("/Applications/Other.app/Contents/MacOS"),
            home
        ));
        assert!(!strict_install_bundle_ok(Path::new("/Applications/cys.app"), home));
    }

    #[test]
    fn plan_cli_install_refuses_applications_lookalike_directories() {
        for bad in [
            "/tmp/Applications/cys.app/Contents/MacOS",
            "/Users/x/Downloads/Applications/cys.app/Contents/MacOS",
            "/Applications/Utilities/Applications/cys.app/Contents/MacOS",
        ] {
            let r = plan_cli_install(std::path::Path::new(bad), "/usr/local/bin", "S");
            let err = match r {
                Ok(_) => panic!("Applications 유사 경로가 설치를 통과했다: {bad}"),
                Err(e) => e,
            };
            assert!(err.contains("Applications"), "다음 조치 안내가 있어야 한다: {err}");
        }
        // 진짜 /Applications 는 그대로 통과(회귀 핀).
        assert!(plan_cli_install(
            std::path::Path::new("/Applications/cys.app/Contents/MacOS"),
            "/usr/local/bin",
            "S"
        )
        .is_ok());
        // 이 기계의 실제 홈 Applications 도 통과해야 한다(엄격 판정이 과하게 잠기지 않았는가).
        let home_bundle = cys::home_dir().join("Applications/cys.app/Contents/MacOS");
        assert!(
            plan_cli_install(&home_bundle, "/usr/local/bin", "S").is_ok(),
            "~/Applications 설치가 막혔다: {}",
            home_bundle.display()
        );
    }

    #[test]
    fn plan_cli_install_canonical_has_no_location_warning() {
        let plan = plan_cli_install(
            std::path::Path::new("/Applications/cys.app/Contents/MacOS"),
            "/usr/local/bin",
            "S"
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
            "S",
        )
        .unwrap();
        assert!(plan.osascript_arg.starts_with("do shell script \""));
        assert!(plan.osascript_arg.ends_with("\" with administrator privileges"));
        assert!(!plan.osascript_arg.starts_with("do shell script '"));
        assert!(plan.osascript_arg.contains("'/usr/local/bin/cys'"));
        assert!(plan.osascript_arg.contains("ln -sfn"));
    }

    // ── D3: 설치 등급(installed / installed_shadowed / unverified) ────────
    #[test]
    fn classify_install_status_installed_only_when_target_is_first() {
        let v = classify_install_status(
            &WhichProbe::Completed(vec!["/usr/local/bin/cys".into()]),
            "/usr/local/bin/cys",
            "zsh",
        );
        assert_eq!(v.status, "installed");
        assert_eq!(v.shadowed_by, None);
        assert!(v.warnings.is_empty(), "성공에는 경고가 붙지 않는다");
        assert_eq!(v.effective_cys.as_deref(), Some("/usr/local/bin/cys"));
    }

    #[test]
    fn classify_install_status_shadowed_when_other_cys_precedes() {
        let v = classify_install_status(
            &WhichProbe::Completed(vec![
                "/opt/homebrew/bin/cys".into(),
                "/usr/local/bin/cys".into(),
            ]),
            "/usr/local/bin/cys",
            "zsh",
        );
        assert_eq!(v.status, "installed_shadowed");
        assert_eq!(v.shadowed_by.as_deref(), Some("/opt/homebrew/bin/cys"));
        assert!(v.warnings.iter().any(|w| w.contains("/opt/homebrew/bin/cys")),
            "무엇이 가리는지 경로를 알려야 한다");
        // ★MAJOR-4: 잰 셸을 밝힌다(예전 문구는 'PATH 앞쪽'이라는 전칭 주장이었다).
        assert!(v.warnings[0].contains("zsh"), "어느 셸 기준인지 밝혀야 한다: {}", v.warnings[0]);
    }

    // ★헌장: "측정 불능은 어떤 게이트에서도 통과가 아니다". which 가 죽거나 매달려도 '설치 완료'로
    // 올라가면 안 된다 — 예전 `.ok()`+`unwrap_or_default()` 경로가 정확히 그 사고였다.
    #[test]
    fn classify_install_status_unverified_on_measurement_failure_or_empty() {
        let failed = classify_install_status(
            &WhichProbe::Unmeasured("zsh 5초 타임아웃".into()),
            "/usr/local/bin/cys",
            "zsh",
        );
        assert_eq!(failed.status, "unverified");
        assert_eq!(failed.shadowed_by, None);
        assert!(failed.warnings.iter().any(|w| w.contains("타임아웃")), "사유를 사용자에게 전달");

        // 측정은 됐지만 PATH 에서 cys 를 못 찾은 경우도 '확인됨'이 아니다.
        let empty = classify_install_status(&WhichProbe::Completed(vec![]), "/usr/local/bin/cys", "zsh");
        assert_eq!(empty.status, "unverified");
        assert_eq!(empty.effective_cys, None);

        // ★MINOR-9: 같은 unverified 라도 원인이 둘이다. UI·문서가 "검증 명령 실패 또는 응답 없음"
        // 으로 **단정**했던 오안내의 뿌리 — 경고문 접두를 안정된 판별자로 고정해 분기를 노출한다.
        assert!(
            failed.warnings[0].starts_with("PATH 확인 실패:"),
            "측정 실패 분기의 신호가 없다: {}",
            failed.warnings[0]
        );
        assert!(
            empty.warnings[0].starts_with("PATH 확인 결과:"),
            "측정 성공 + 미발견 분기의 신호가 없다: {}",
            empty.warnings[0]
        );
        assert!(
            empty.warnings[0].contains("정상 실행됐지만"),
            "검증 명령이 실패했다고 오단정하면 안 된다: {}",
            empty.warnings[0]
        );
        // ★MAJOR-4: 무엇으로 쟀는지(로그인 셸 이름)를 밝힌다 — 전칭 'PATH 1순위' 주장 금지.
        for v in [&failed, &empty] {
            assert!(
                v.warnings[0].contains("zsh"),
                "어느 셸로 쟀는지 밝혀야 한다: {}",
                v.warnings[0]
            );
        }
    }

    // ── D6: which 검증 타임아웃(무기한 hang 차단) ──────────────────────
    #[test]
    fn run_capture_with_timeout_returns_stdout_and_kills_hung_child() {
        let (ok, out) = run_capture_with_timeout(
            "/bin/sh",
            &["-c", "echo hi"],
            std::time::Duration::from_secs(5),
        )
        .expect("정상 종료는 Ok");
        assert!(ok);
        assert_eq!(out.trim(), "hi");

        // 매달린 자식은 기한 초과로 kill 되고, 호출자는 반드시 돌아온다(빈 성공으로 접히지 않는다).
        let started = std::time::Instant::now();
        let hung = run_capture_with_timeout(
            "/bin/sh",
            &["-c", "sleep 30"],
            std::time::Duration::from_millis(300),
        );
        assert!(hung.is_err(), "기한 초과는 Err(측정 실패)여야 한다");
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "기한 안에 돌아와야 한다");

        // 실행 자체가 불가능한 경우도 Err — 조용한 빈 결과가 아니다.
        assert!(run_capture_with_timeout(
            "/nonexistent/cys-probe",
            &[],
            std::time::Duration::from_secs(1)
        )
        .is_err());
    }

    // ★MAJOR-3(2026-08-25) 회귀 트립와이어 — **실제로 손자를 띄운다**.
    // 예전 구현(파이프 + 드레인 스레드 무기한 join)은 이 두 시나리오에서 무기한 블록했다:
    // read_to_string 은 write-end 가 **전부** 닫혀야 EOF 를 보는데, 자식을 kill 해도 stdout 을 물고
    // 있는 손자가 살아 있으면 EOF 가 오지 않는다. 기존 D6 테스트가 초록이던 이유는
    // `sh -c "sleep 30"` 이 exec 대체되어 손자가 아예 없는 유일한 형태였기 때문이다.
    #[cfg(unix)]
    #[test]
    fn run_capture_with_timeout_returns_even_when_grandchild_holds_stdout() {
        // ① 자식은 즉시 끝나지만 손자가 stdout 을 계속 물고 있다.
        let started = std::time::Instant::now();
        let (ok, out) = run_capture_with_timeout(
            "/bin/sh",
            &["-c", "sleep 12 & echo alive"],
            std::time::Duration::from_secs(5),
        )
        .expect("자식이 정상 종료했으므로 Ok");
        assert!(ok, "자식은 0으로 끝났다");
        assert_eq!(out.trim(), "alive", "자식이 남긴 출력은 그대로 읽혀야 한다");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "손자가 stdout 을 물고 있어도 즉시 돌아와야 한다(경과 {:?})",
            started.elapsed()
        );

        // ② 자식도 매달리고 손자도 stdout 을 물고 있다 — kill 뒤에도 EOF 는 오지 않는다.
        let started = std::time::Instant::now();
        let hung = run_capture_with_timeout(
            "/bin/sh",
            &["-c", "sleep 12 & sleep 12"],
            std::time::Duration::from_secs(1),
        );
        assert!(hung.is_err(), "기한 초과는 Err(측정 실패)여야 한다");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(6),
            "kill 뒤 손자 EOF 를 기다리면 안 된다(경과 {:?})",
            started.elapsed()
        );
    }

    // 임시 파일 누수 금지 — 프로브는 어느 경로로 빠져나가든(정상·타임아웃·spawn 실패) 자기 흔적을
    // 남기지 않는다. 수천 번 호출돼도 TMPDIR 이 차면 안 된다. 격리 디렉터리로 재서 병렬 테스트와
    // 경합하지 않게 한다.
    #[cfg(unix)]
    #[test]
    fn run_capture_with_timeout_cleans_up_its_temp_file() {
        let dir = std::env::temp_dir().join(format!("cys-probe-cleanup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let a = run_capture_with_timeout_in(&dir, "/bin/sh", &["-c", "echo x"], std::time::Duration::from_secs(5));
        assert_eq!(a.expect("정상 종료").1.trim(), "x");
        // 타임아웃 + 손자가 stdout 을 물고 있는 경로에서도 파일은 지워져야 한다.
        assert!(run_capture_with_timeout_in(
            &dir,
            "/bin/sh",
            &["-c", "sleep 12 & sleep 12"],
            std::time::Duration::from_millis(300)
        )
        .is_err());
        assert!(run_capture_with_timeout_in(
            &dir,
            "/nonexistent/cys-probe",
            &[],
            std::time::Duration::from_secs(1)
        )
        .is_err());

        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(left.is_empty(), "프로브 임시 파일이 남았다: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── D4a: 해제 가드(비가역) ────────────────────────────────────────
    #[test]
    fn links_into_cys_bundle_matches_only_bundle_binaries() {
        assert!(links_into_cys_bundle("/Applications/cys.app/Contents/MacOS/cys"));
        assert!(links_into_cys_bundle("/Users/x/Applications/cys.app/Contents/MacOS/cysd"));
        // 백업 번들·타 앱·임의 경로는 우리 링크가 아니다.
        assert!(!links_into_cys_bundle("/Applications/cys.app.bak-044/Contents/MacOS/cys"));
        assert!(!links_into_cys_bundle("/Applications/Other.app/Contents/MacOS/cys"));
        assert!(!links_into_cys_bundle("/opt/homebrew/bin/cys"));
        // 번들 안이어도 cys/cysd 가 아닌 파일은 대상이 아니다.
        assert!(!links_into_cys_bundle("/Applications/cys.app/Contents/MacOS/cys-app"));
    }

    fn probe(path: &str, present: bool, is_symlink: bool, target: Option<&str>) -> LinkProbe {
        LinkProbe {
            path: path.into(),
            present,
            is_symlink,
            link_target: target.map(|t| t.to_string()),
        }
    }

    // ★반례 4종: 일반 파일 / 타 앱 번들 지시 링크 / 부재 / dangling 링크.
    // dangling(대상 파일이 이미 없는 링크)은 **반드시 제거 대상**이다 — 앱을 지운 뒤 남은 죽은
    // 명령이 정확히 그 상태이고, 그게 이 기능이 존재하는 이유다.
    #[test]
    fn decide_cli_uninstall_removes_only_our_symlink() {
        // ① 일반 파일(다른 도구가 설치한 실체 바이너리) — 절대 지우지 않는다
        assert_eq!(
            decide_cli_uninstall(&probe("/usr/local/bin/cys", true, false, None)),
            UninstallAction::SkipNotSymlink
        );
        // ② 심볼릭이지만 대상이 우리 번들 밖
        assert_eq!(
            decide_cli_uninstall(&probe(
                "/usr/local/bin/cys",
                true,
                true,
                Some("/opt/homebrew/Cellar/cys/1.0/bin/cys")
            )),
            UninstallAction::SkipForeignTarget
        );
        // ③ 부재 — 할 일 없음
        assert_eq!(
            decide_cli_uninstall(&probe("/usr/local/bin/cysd", false, false, None)),
            UninstallAction::SkipAbsent
        );
        // ④ dangling(앱 삭제 후 남은 잔재) — 제거 대상
        assert_eq!(
            decide_cli_uninstall(&probe(
                "/usr/local/bin/cys",
                true,
                true,
                Some("/Applications/cys.app/Contents/MacOS/cys")
            )),
            UninstallAction::Remove
        );
        // 대상 경로를 읽지 못한 링크도 지우지 않는다(모르면 손대지 않는다)
        assert_eq!(
            decide_cli_uninstall(&probe("/usr/local/bin/cys", true, true, None)),
            UninstallAction::SkipForeignTarget
        );
    }

    // ★MAJOR-2: 비특권 사전 관측(probe_link)은 root 집행의 가드가 될 수 없다 — 그 사이에 사용자가
    // 비밀번호를 치는 시간 제한 없는 창이 있다. 승격 스크립트 **자신이** 재검증해야 한다.
    #[test]
    fn build_uninstall_script_reverifies_symlink_and_target_before_rm() {
        let s = build_uninstall_script(&[
            "/usr/local/bin/cys".to_string(),
            "/usr/local/bin/cysd".to_string(),
        ]);
        // ① 심볼릭 재검사 ② readlink 대조 — 둘 다 스크립트 문자열 안에 있어야 한다.
        assert!(s.contains("[ -L '/usr/local/bin/cys' ]"), "-L 재검사 누락: {s}");
        assert!(s.contains("[ -L '/usr/local/bin/cysd' ]"), "-L 재검사 누락: {s}");
        assert!(s.contains("readlink '/usr/local/bin/cys'"), "readlink 대조 누락: {s}");
        assert!(s.contains("readlink '/usr/local/bin/cysd'"), "readlink 대조 누락: {s}");
        assert!(
            s.contains("*/cys.app/Contents/MacOS/cys|*/cys.app/Contents/MacOS/cysd"),
            "번들 마커 대조 누락(links_into_cys_bundle 과 같은 마커여야 한다): {s}"
        );
        // 가드 없는 일괄 rm(예전 본문)이 남아 있으면 회귀다.
        assert!(
            !s.contains("rm -f '/usr/local/bin/cys' '/usr/local/bin/cysd'"),
            "가드 없는 일괄 rm 회귀: {s}"
        );
        assert!(!s.contains("rm -r"), "재귀 삭제 금지: {s}");
        // 지우는 대상은 여전히 리터럴 인용 경로 하나씩(경로 전개 없음).
        assert!(s.contains("rm -f '/usr/local/bin/cys'"));
        assert!(s.contains("rm -f '/usr/local/bin/cysd'"));
    }

    // ★MAJOR-2 **실행** 검증: 관측 이후 대상이 바뀐 상황(TOCTOU 창이 닫힌 뒤의 세계)을 만들어 놓고
    // 스크립트를 돌린다. 예전 본문(`rm -f <경로들>`)은 셋을 전부 지웠다.
    #[cfg(unix)]
    #[test]
    fn uninstall_script_refuses_non_symlink_and_foreign_target_at_execution_time() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!("cys-uninstall-script-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        // ① 사전 관측 뒤 실체 파일로 바뀐 경로(남의 설치본) — 살아남아야 한다.
        let real = base.join("real");
        std::fs::write(&real, "FOREIGN-BINARY").unwrap();
        // ② 남의 번들을 가리키게 바뀐 심볼릭 — 살아남아야 한다.
        let foreign = base.join("foreign");
        symlink("/opt/homebrew/Cellar/cys/1.0/bin/cys", &foreign).unwrap();
        // ③ 진짜 우리 링크(대상이 이미 없는 dangling) — 지워져야 한다. 이게 기능의 존재 이유다.
        let ours = base.join("ours");
        symlink("/Applications/cys.app/Contents/MacOS/cys", &ours).unwrap();

        let paths: Vec<String> = [&real, &foreign, &ours]
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        let script = build_uninstall_script(&paths);
        let st = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .status()
            .unwrap();
        assert!(st.success(), "해제 스크립트 실패: {script}");

        assert_eq!(
            std::fs::read_to_string(&real).unwrap_or_default(),
            "FOREIGN-BINARY",
            "심볼릭이 아닌 실체 파일을 root 권한으로 지웠다"
        );
        assert!(
            std::fs::symlink_metadata(&foreign).is_ok(),
            "남의 번들을 가리키는 링크를 지웠다"
        );
        assert!(
            std::fs::symlink_metadata(&ours).is_err(),
            "우리 링크(dangling)는 지워져야 한다 — 가드가 과하게 잠겼다"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn plan_cli_uninstall_removes_ours_and_explains_skips() {
        let plan = plan_cli_uninstall(&[
            probe("/usr/local/bin/cys", true, true, Some("/Applications/cys.app/Contents/MacOS/cys")),
            probe("/usr/local/bin/cysd", true, false, None),
        ]);
        assert_eq!(plan.remove, vec!["/usr/local/bin/cys".to_string()]);
        assert_eq!(plan.skipped.len(), 1);
        assert!(plan.skipped[0].contains("/usr/local/bin/cysd"));
        assert!(plan.skipped[0].contains("심볼릭이 아니라"), "왜 안 지웠는지 읽을 수 있어야 한다");
        let arg = plan.osascript_arg.expect("제거 대상이 있으면 승격 인자 생성");
        // 설치와 동일 규약: 바깥은 AppleScript 큰따옴표(작은따옴표면 -2741), 내부 경로는 sh_squote.
        assert!(arg.starts_with("do shell script \""));
        assert!(arg.ends_with("\" with administrator privileges"));
        assert!(arg.contains("'/usr/local/bin/cys'"));
        assert!(!arg.contains("'/usr/local/bin/cysd'"), "건드리지 않기로 한 경로가 스크립트에 새면 안 된다");
    }

    #[test]
    fn plan_cli_uninstall_never_elevates_when_nothing_to_remove() {
        let plan = plan_cli_uninstall(&[
            probe("/usr/local/bin/cys", false, false, None),
            probe("/usr/local/bin/cysd", false, false, None),
        ]);
        assert!(plan.remove.is_empty());
        assert!(plan.osascript_arg.is_none(), "지울 것이 없으면 관리자 프롬프트를 띄우지 않는다");
        assert_eq!(plan.skipped.len(), 2);
    }

    // ── D4b: 버튼 라벨을 가르는 상태 판정 ─────────────────────────────
    #[test]
    fn classify_cli_links_maps_absent_ours_partial_foreign() {
        let ours = "/Applications/cys.app/Contents/MacOS/cys";
        assert_eq!(
            classify_cli_links(&[
                probe("/usr/local/bin/cys", false, false, None),
                probe("/usr/local/bin/cysd", false, false, None),
            ]),
            CliLinkState::Absent
        );
        assert_eq!(
            classify_cli_links(&[
                probe("/usr/local/bin/cys", true, true, Some(ours)),
                probe("/usr/local/bin/cysd", true, true, Some("/Applications/cys.app/Contents/MacOS/cysd")),
            ]),
            CliLinkState::Ours
        );
        assert_eq!(
            classify_cli_links(&[
                probe("/usr/local/bin/cys", true, true, Some(ours)),
                probe("/usr/local/bin/cysd", false, false, None),
            ]),
            CliLinkState::Partial
        );
        assert_eq!(
            classify_cli_links(&[
                probe("/usr/local/bin/cys", true, false, None),
                probe("/usr/local/bin/cysd", false, false, None),
            ]),
            CliLinkState::Foreign
        );
    }

    // 라벨과 행동의 일치 가드: '해제' 라벨(installed=true)은 실제로 지울 것이 있을 때만 뜬다.
    #[test]
    fn install_label_state_agrees_with_uninstall_plan() {
        for probes in [
            vec![
                probe("/usr/local/bin/cys", true, false, None),
                probe("/usr/local/bin/cysd", false, false, None),
            ],
            vec![
                probe("/usr/local/bin/cys", true, true, Some("/opt/homebrew/bin/cys")),
                probe("/usr/local/bin/cysd", false, false, None),
            ],
            vec![
                probe("/usr/local/bin/cys", true, true, Some("/Applications/cys.app/Contents/MacOS/cys")),
                probe("/usr/local/bin/cysd", false, false, None),
            ],
        ] {
            let state = classify_cli_links(&probes);
            let installed = matches!(state, CliLinkState::Ours | CliLinkState::Partial);
            let plan = plan_cli_uninstall(&probes);
            assert_eq!(
                installed,
                !plan.remove.is_empty(),
                "'해제' 라벨과 실제 제거 대상이 어긋났다: {state:?}"
            );
        }
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

    /// ★완전 초기화 트립와이어(DESIGN-factory-reset.md §7): GUI 커맨드는 ①직접 rm 을 만들지
    /// 않고(격리 독트린 — 삭제는 lib 코어의 mv·manifest 경로뿐) ②lib 코어(cys::factory_reset)에
    /// 위임하며 ③부서 purge 실사고의 --purge-workdir 류 작업폴더 파괴 경로를 절대 열지 않는다.
    #[test]
    fn factory_reset_cmd_delegates_and_never_deletes_directly() {
        let src = include_str!("main.rs");
        let start = src
            .find("async fn factory_reset_execute")
            .expect("factory_reset_execute 정의 소실 — 트립와이어 재배선 필요");
        let seg = &src[start..start + src[start..].find("\n#[tauri::command]").unwrap_or(src.len() - start)];
        for banned in ["rm -rf", "remove_dir_all", "remove_file", "--purge-workdir"] {
            assert!(
                !seg.contains(banned),
                "factory_reset_execute 가 '{banned}' 를 포함 — 격리 독트린 위반(직접 삭제 금지)"
            );
        }
        assert!(
            seg.contains("cys::factory_reset::execute_quarantine"),
            "factory_reset_execute 가 lib 코어 위임을 벗어남 — 이중 구현 금지(계약: DESIGN-factory-reset.md)"
        );
        assert!(
            seg.contains("stop_daemons_and_unregister"),
            "정지 단계 소실 — 살아있는 데몬 밑에서 격리하는 경로가 열린다"
        );
        // ★P0-1: GUI 실행 경로도 RAII 센티널을 무장해야 한다(누락 시 리셋 중 데몬이 되살아나고,
        // 실패 시 해제자가 없어 데몬 기동이 최대 15분 막힌다 — 시뮬레이션 확정 결함).
        assert!(
            seg.contains("ResetSentinel::arm()"),
            "GUI 실행 경로에 센티널 무장이 없다(P0-1)"
        );
        // ★P0-6: 격리 목적지 사전 점검 없이 데몬을 죽이면 안 된다.
        assert!(
            seg.contains("trash_root_ready"),
            "격리 폴더 사전 점검이 빠졌다 — 데몬만 죽이고 실패하는 경로가 열린다(P0-6)"
        );
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

    /// ★SEAL-1 GUI 직스폰 층 회귀 핀(2026-08-01 실사고): `inject_runtime_path` 는 "자식이
    /// 번들 python 을 쓰게 만드는" 배선의 단일 지점이다 — 여기서 ENV_PY_NO_BYTECODE=1 이
    /// 빠지면 GUI 직스폰(bash/python3) 자식이 번들 안에 `__pycache__/*.pyc` 를 써서
    /// 코드서명 봉인을 깬다(다음 실행이 Gatekeeper 에 차단). lib.rs 의 python_command·
    /// spawn_env_pairs 핀과 **별개 층**이다 — 둘이 남아도 이 함수가 쌍을 잃으면 GUI 경로가 샌다.
    #[test]
    fn gui_direct_spawns_never_write_bytecode_into_the_bundle() {
        let mut cmd = std::process::Command::new("true");
        inject_runtime_path(&mut cmd);
        let got = cmd
            .get_envs()
            .find(|(k, _)| *k == std::ffi::OsStr::new(cys::ENV_PY_NO_BYTECODE))
            .and_then(|(_, v)| v)
            .map(|v| v.to_string_lossy().into_owned());
        assert_eq!(
            got.as_deref(),
            Some(cys::PY_NO_BYTECODE_ON),
            "inject_runtime_path 가 바이트코드 쓰기 차단 쌍을 잃었다 — GUI 직스폰 python 이 번들을 오염시킨다"
        );
    }
    /// ★W-B2 회귀 핀(감사 blocker #4 의 GUI 절반): GUI 직스폰 env 키 집합 ⊇ pane 스폰
    /// **runtime 규약** env 키 집합(PATH·HOME backfill·PYTHONDONTWRITEBYTECODE·PYTHONUTF8).
    /// 종전 누락 2종(PYTHONUTF8·HOME backfill) 탓에 한국어 Windows(cp949) GUI 직스폰
    /// 부트 체인이 UnicodeEncodeError 로 즉사했다 — 목표는 누락 0 이고 이 핀이 그걸 박제한다.
    ///
    /// ★범위 한계(정직 고지): pane 스폰(state.rs `create_surface_with_env`)은 위 규약에 더해
    /// pane **정체성** env(TERM·LANG·CYS_SURFACE_ID/REF·CYS_SOCKET·CYS_PACK_DIR 등)를 얹지만,
    /// 그것들은 PTY/surface 소속 표식이라 "직스폰 CLI 자식"인 GUI 경로의 대칭 대상이 아니다 —
    /// 이 핀의 pane 측 기준은 **공용 규약(cys::spawn_env_pairs)** 이다. state.rs 는 bin
    /// 크레이트라 여기서 심볼 참조가 불가능하고(W-B2 는 state.rs 무접촉 제약) pane literal
    /// ("PYTHONUTF8","1")은 아래 ③에서 literal 거울로 핀한다.
    #[test]
    fn gui_spawn_env_matches_pane_spawn_env() {
        // ① 규약 완전집합 핀: 조건을 강제(PATH 변경 유발·HOME 부재·USERPROFILE 실재)하면
        //    4키 전부 나와야 한다 — 여기서 키가 빠지면 "규약 소비" 자체가 반쪽이 된다.
        let full = cys::spawn_env_pairs(
            std::path::Path::new("/nonexistent-exe-dir-for-pin"),
            "/usr/bin:/bin",
            None,
            Some("C:\\Users\\x"),
        );
        let full_keys: std::collections::BTreeSet<&str> =
            full.iter().map(|(k, _)| k.as_str()).collect();
        for want in ["PATH", "HOME", cys::ENV_PY_NO_BYTECODE, cys::ENV_PY_UTF8] {
            assert!(
                full_keys.contains(want),
                "공용 규약(spawn_env_pairs)에서 {want} 키가 사라졌다 — GUI/pane 대칭의 기반 소실"
            );
        }

        // ② 상위집합 핀: **같은 프로세스 env** 아래에서 GUI 직스폰(inject_runtime_path)이
        //    pane 규약 키를 하나도 빠뜨리지 않는다(W-B2 이전: PYTHONUTF8·HOME 2종 누락).
        let mut cmd = std::process::Command::new("true");
        inject_runtime_path(&mut cmd);
        let gui_keys: std::collections::BTreeSet<String> = cmd
            .get_envs()
            .filter_map(|(k, v)| v.map(|_| k.to_string_lossy().into_owned()))
            .collect();
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .expect("테스트 바이너리 exe_dir 조회 실패");
        let pane_pairs = cys::spawn_env_pairs_from_process(&exe_dir);
        let missing: Vec<&str> = pane_pairs
            .iter()
            .map(|(k, _)| k.as_str())
            .filter(|k| !gui_keys.contains(*k))
            .collect();
        assert!(
            missing.is_empty(),
            "GUI 직스폰 env 가 pane 규약 키를 누락했다: {missing:?} — cp949 즉사/HOME 붕괴 재발"
        );

        // ③ 무조건 쌍 2종은 값까지 literal 로 핀 — pane 측 state.rs literal("PYTHONUTF8","1")
        //    주입과의 파리티(중복 주입 무해 근거 = 값 동일)를 상수 우회 없이 못박는다.
        for (k, want) in [("PYTHONDONTWRITEBYTECODE", "1"), ("PYTHONUTF8", "1")] {
            let got = cmd
                .get_envs()
                .find(|(ek, _)| *ek == std::ffi::OsStr::new(k))
                .and_then(|(_, v)| v)
                .map(|v| v.to_string_lossy().into_owned());
            assert_eq!(
                got.as_deref(),
                Some(want),
                "GUI 직스폰 무조건 쌍 {k}={want} 소실 — 값이 다르면 pane literal 과의 무해 중복 근거도 무너진다"
            );
        }
    }
}
