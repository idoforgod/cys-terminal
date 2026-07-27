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
// GUI가 부팅 시 브로커(cysd)에 등록한 세션. 브로커 등록 대장은 데몬 메모리에 있어 데몬
// 재시작 시 소실되므로, 낡은 세션 거부를 만나면 비우고 재등록할 수 있어야 한다 → 재설정
// 불가한 OnceLock 금지(v0.13.10 실사고: 됐다안됐다 간헐 실패), RwLock<Option<_>>로 보관.
static BROWSER_APP_SESSION: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// 브로커(cysd) 등록 대장 소실 시 GUI가 issue_gesture 거부로 받는 메시지의 판별 부분열.
/// 원문: src/bin/cysd/authority_broker/mod.rs 의 BrokerFailure "GUI peer has no broker-owned
/// registration"(코드 GUI_NOT_REGISTERED). rpc()는 error.message만 올리므로 이 부분열로 판별한다.
/// 문자열 정합은 broker_registration_lost_marker_matches_broker_source 테스트가 보장.
const BROKER_REGISTRATION_LOST_MARKER: &str = "no broker-owned registration";

/// GUI가 **분기**하는 브로커 실패 코드(자가 회복 판정). 문자열 정합은
/// `gui_branch_codes_exist_in_broker_source` 트립와이어가 컴파일 시점 임베드로 보장한다.
const GUI_NOT_REGISTERED_CODE: &str = "GUI_NOT_REGISTERED";
const GUI_IDENTITY_MISMATCH_CODE: &str = "GUI_IDENTITY_MISMATCH";
/// 브로커가 만료·미발급·기소비를 한 code로 묶어 보내는 등록 challenge 거부 사인.
const GUI_REGISTRATION_REPLAY_CODE: &str = "GUI_REGISTRATION_REPLAY";
/// **만료 전용 code(정공법)** — 아직 브로커에 없다. 브로커 담당 워커가 만료 분기를 이 code로
/// 분리하면 GUI는 아래 문자열 폴백을 지우고 곧바로 이 상수만 보면 된다(이관 지점을 미리 고정).
const GUI_CHALLENGE_EXPIRED_CODE: &str = "GUI_CHALLENGE_EXPIRED";
/// ★임시 조치(한계 명시): 브로커는 만료를 `GUI_REGISTRATION_REPLAY` 하나로 뭉쳐 보내고 사유만
/// message 괄호에 부기한다(authority_broker/mod.rs `(사유: 만료)`). 만료는 "틀렸다"가 아니라
/// "느렸다"이므로 **재발급→재등록이 정답인 회복 가능 조건**인데, code만 보면 미발급·기소비와
/// 구별할 수 없어 자가회복 분기를 못 탄다. 그래서 당분간 message 부분열로 만료를 식별한다.
///
/// 한계(정직한 기록): ①message는 계약이 아니다 — 브로커가 문구를 손보면 이 분기는 **무증상으로
/// 죽는다**(`expiry_marker_exists_in_broker_source` 트립와이어가 컴파일 시점 임베드로 그것만 막는다).
/// ②한국어 리터럴 의존이라 로케일·번역 도입 시 즉시 파손. ③미발급·기소비는 여전히 회복 대상이
/// 아니다(재시도해도 같은 실패 — 의도된 보수성).
/// → 근본 수리는 브로커측 `GUI_CHALLENGE_EXPIRED` 신설이다(**브로커 담당 워커 조치 필요**).
const BROKER_CHALLENGE_EXPIRED_MARKER: &str = "(사유: 만료)";
/// 데몬 응답이 아예 없는 전송 계층 실패(연결 끊김·직렬화) — 데몬 code가 존재하지 않는 경우.
const RPC_TRANSPORT_FAILED_CODE: &str = "RPC_TRANSPORT_FAILED";
/// 데몬이 error.code 없이 message만 준 경우(구 데몬 스큐) — 배너 형식을 깨지 않기 위한 자리표.
const RPC_UNCODED_ERROR_CODE: &str = "RPC_UNCODED_ERROR";
/// ok 응답인데 GUI가 요구하는 필드가 없거나 형식이 어긋난 경우(데몬 스큐·계약 위반).
const RPC_MALFORMED_RESPONSE_CODE: &str = "RPC_MALFORMED_RESPONSE";
/// GUI 자체가 embed 요청을 만들지 못한 경우(pane_nonce·ticket·generation·origin 검증 실패) —
/// RPC를 보내기도 전의 **로컬** 거부라 데몬 code가 존재하지 않는다(ADV-2).
const EMBED_INTENT_INVALID_CODE: &str = "EMBED_INTENT_INVALID";

static NATIVE_BROWSER_ACTIVATIONS: std::sync::OnceLock<Mutex<NativeActivationRegistry>> =
    std::sync::OnceLock::new();

// 신뢰된 네이티브 Browser 활성화 창의 arm→consume 유효시간. 2s는 부팅 직후 부하에서 첫 클릭의
// arm→consume이 초과해 "trusted native Browser activation expired"를 유발했다(v0.13.8 실사고).
// 10s로 상향해도 활성화는 여전히 one-time·window-bound 단일 소모라 보안 성질(재생 불가)은 불변이다.
const NATIVE_ACTIVATION_TTL_MS: u64 = 10_000;
const TAURI_BROWSER_ENSURE_DEADLINE_SECS: u64 =
    cys::browser_runtime::ENSURE_WORKER_DEADLINE.as_secs() + 5;

/// GUI peer 등록 체인(`gui_challenge`·`register_gui`) RPC 1콜의 **읽기 상한**.
/// ★F-3(성찰3): 등록은 `GUI_REGISTRATION_FLIGHT` 뮤텍스로 직렬화되는데 체인 전체
/// (`register_browser_gui_peer`→`rpc_on_full`→`rpc_full`→`rpc_once`)에 read 상한이 없었다 —
/// 연결은 됐지만 응답하지 않는 cysd 하나가 뮤텍스를 **영구 점유**하면 부트 재시도뿐 아니라
/// 신뢰 클릭 경로(`ensure_browserd_cast`)까지 통째로 막힌다(single-flight 도입이 만든 결합).
/// 상한은 `ensure`의 소켓 RPC 상한과 같은 계열의 보수적 10s다(등록은 브로커 메모리 조작이라
/// runtime cold start 같은 긴 작업이 아니다). 상한 초과는 `AfterSend`로 처리돼 풀 연결이
/// 폐기되므로(`rpc_full`의 `*conn = None`) 취소가 공유 풀을 stale 응답으로 오염시키지 않는다.
const GUI_REGISTRATION_RPC_DEADLINE_SECS: u64 = 10;

#[derive(Default)]
struct NativeActivationRegistry {
    windows: HashMap<String, NativeWindowActivation>,
}

struct NativeWindowActivation {
    bootstrap_capability: String,
    pending: Option<PendingNativeActivation>,
}

struct PendingNativeActivation {
    _nonce: String,
    expires_at_ms: u64,
}

impl NativeActivationRegistry {
    fn install_window(&mut self, label: &str, capability: &str) -> Result<(), String> {
        validate_lower_hex(capability, "Browser bootstrap capability")?;
        if label.is_empty() || label.len() > 64 || self.windows.contains_key(label) {
            return Err("invalid or duplicate Browser activation window".into());
        }
        self.windows.insert(
            label.into(),
            NativeWindowActivation {
                bootstrap_capability: capability.into(),
                pending: None,
            },
        );
        Ok(())
    }

    fn arm(&mut self, label: &str, capability: &str, now_ms: u64) -> Result<(), String> {
        validate_lower_hex(capability, "Browser bootstrap capability")?;
        let window = self
            .windows
            .get_mut(label)
            .ok_or_else(|| "Browser activation window is not registered".to_string())?;
        if window.bootstrap_capability != capability {
            window.pending = None;
            return Err("Browser bootstrap capability mismatch".into());
        }
        window.pending = Some(PendingNativeActivation {
            _nonce: random_secret_hex()?,
            expires_at_ms: now_ms.saturating_add(NATIVE_ACTIVATION_TTL_MS),
        });
        Ok(())
    }

    fn consume(&mut self, label: &str, now_ms: u64) -> Result<(), String> {
        let window = self
            .windows
            .get_mut(label)
            .ok_or_else(|| "Browser activation window is not registered".to_string())?;
        let pending = window
            .pending
            .take()
            .ok_or_else(|| "trusted native Browser activation is required".to_string())?;
        if pending.expires_at_ms <= now_ms {
            return Err("trusted native Browser activation expired".into());
        }
        Ok(())
    }
}

fn unix_millis_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn validate_lower_hex(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} must be 32-byte lowercase hex"));
    }
    Ok(())
}

fn random_secret_hex() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| format!("OS CSPRNG unavailable: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// invoke 초기화 스크립트에 push_str 연결되므로 **반드시 `;`로 시작**해야 한다 — 선행
/// ipc-protocol.js가 `})()`(식)로 끝나 ASI가 발동하지 않고, `(`로 시작하면 호출 연쇄로
/// 삼켜져 인터셉터가 통째로 미설치된다(v0.13.6 실사고). 경계 계약은 동파일 테스트
/// `activation_script_survives_ipc_protocol_concatenation`이 봉쇄한다.
/// 표식 `__CYS_BROWSER_ACTIVATION__`은 진단 전용(installed→armed-ok|arm-failed:|skip-*) —
/// skip-*는 인터셉터가 살아있으나 클릭이 특정 가드(untrusted·inactive-ua·not-element·nomatch)에서
/// 탈락한 지점을 특정한다(설치됨=installed 상태에서 클릭 미인터셉트 원인 세분화).
/// 권위 없음(판정자는 백엔드 consume). capability는 클로저 격리 유지. arm 실패도 클릭을
/// 재생한다 — 무반응 버튼 대신 백엔드의 정확한 거부 사유가 UI 배관으로 노출되게.
///
/// 이 스크립트는 invoke init(ipc-protocol) 단계에서 실행 — core.js가 `.invoke`를 정의하기
/// **이전**이다(tauri 2.11.2 실측 순서). 설치 시점에 `__TAURI_INTERNALS__`를 접촉하면
/// `undefined.bind` TypeError로 리스너가 미등록된다(v0.13.7 실사고: 표식 `installed` 정지·
/// 클릭 영영 미인터셉트). invoke는 반드시 클릭 시점 지연 조회 — 설치 시점 코드
/// (리스너 등록 전)는 `__TAURI_INTERNALS__`를 일절 접촉하지 않는다.
fn browser_activation_initialization_script(capability: &str) -> String {
    format!(
        r##";(() => {{
  'use strict';
  const capability = '{capability}';
  const mark = (s) => {{ try {{ window.__CYS_BROWSER_ACTIVATION__ = s; }} catch (_) {{}} }};
  mark('installed');
  const nativeClick = HTMLElement.prototype.click;
  document.addEventListener('click', (event) => {{
    // 각 조기 return에 skip-* 진단 표식 — 클릭이 어느 가드에서 탈락하는지 실사고 특정용.
    // ★skip-untrusted만은 마커를 덮지 않는다: arm 성공 후 nativeClick 재생(합성 클릭)은
    // isTrusted=false라 이 가드로 매번 재진입한다. mark('armed-ok')가 재생보다 먼저 찍히므로
    // 무조건 덮으면 최종 상태가 armed-ok→skip-untrusted로 오염된다. armed 접두면 기록을 보류해
    // armed-ok가 최종 상태로 남게 한다(진단 진실 보존). 마커 값은 UI 노출되므로 id/tagName만.
    if (!event.isTrusted) {{ if (!String(window.__CYS_BROWSER_ACTIVATION__ || '').startsWith('armed')) mark('skip-untrusted'); return; }}
    const ua = navigator.userActivation;
    if (ua && ua.isActive === false) {{ mark('skip-inactive-ua'); return; }}
    const raw = event.target;
    if (!(raw instanceof Element)) {{ mark('skip-not-element'); return; }}
    const target = raw.closest("#btn-browser, [data-browser-reconnect='true']");
    if (!target) {{ mark('skip-nomatch:' + (raw.id || raw.tagName || '?')); return; }}
    event.preventDefault();
    event.stopImmediatePropagation();
    const internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== 'function') {{
      mark('arm-failed: __TAURI_INTERNALS__.invoke unavailable at click time');
      return nativeClick.call(target);
    }}
    internals.invoke('arm_browser_native_activation', {{ bootstrapCapability: capability }})
      .then(() => {{ mark('armed-ok'); nativeClick.call(target); }})
      .catch((e) => {{ mark('arm-failed: ' + e); nativeClick.call(target); }});
  }}, true);
}})();"##
    )
}

fn browser_session_params(mut params: Value) -> Result<Value, String> {
    // read 가드는 즉시 drop하고 String만 clone해 이후(비동기 rpc 포함) 가드를 들고 있지 않는다.
    let session = BROWSER_APP_SESSION
        .read()
        .unwrap()
        .clone()
        .ok_or_else(|| "Browser GUI peer registration is unavailable".to_string())?;
    let object = params
        .as_object_mut()
        .ok_or_else(|| "Browser RPC params must be an object".to_string())?;
    object.insert("app_session".into(), Value::String(session));
    Ok(params)
}

/// GUI peer 등록의 **단일 비행(single-flight) 게이트**. 부트 재시도 태스크와 클릭 경로가 동시에
/// 등록을 시도할 수 있는데, `GUI_ALREADY_REGISTERED` 폐기(WS-4)로 두 번째 등록이 이제 **교체**로
/// 성공한다 — 즉 늦게 도착한 등록이 살아있는 세션 S1을 S2로 갈아치우고, 그 사이 날아간 RPC가
/// `GUI_IDENTITY_MISMATCH`를 맞는다(종전 c232a1e에서는 두 번째가 거부돼 S1이 보존됐다).
/// 이 뮤텍스가 등록을 직렬화하고, 아래 `ensure_browser_gui_peer`가 "이미 세션이 있으면 손대지
/// 않는다"를 강제해 그 창을 닫는다.
static GUI_REGISTRATION_FLIGHT: std::sync::OnceLock<tauri::async_runtime::Mutex<()>> =
    std::sync::OnceLock::new();

fn gui_registration_flight() -> &'static tauri::async_runtime::Mutex<()> {
    GUI_REGISTRATION_FLIGHT.get_or_init(|| tauri::async_runtime::Mutex::new(()))
}

/// **살아있는 세션은 절대 교체하지 않는** 등록 진입점. 세션이 없을 때만 실제 등록을 수행한다.
/// 부트 재시도·클릭 지연등록·세션 소실 회복 — 모든 경로가 이걸 쓴다. 강제 재등록이 필요한 곳은
/// 먼저 `BROWSER_APP_SESSION`을 None으로 비운 뒤 호출한다(= 자기 세션을 스스로 무효화한 경우만).
async fn ensure_browser_gui_peer() -> Result<(), RpcFailure> {
    ensure_browser_gui_peer_on(
        &default_socket(),
        std::time::Duration::from_secs(GUI_REGISTRATION_RPC_DEADLINE_SECS),
    )
    .await
}

/// `ensure_browser_gui_peer`의 소켓·상한 주입형 — 프로덕션 경로는 항상 기본 소켓 +
/// `GUI_REGISTRATION_RPC_DEADLINE_SECS`를 쓰고, 테스트만 무응답 소켓 + 짧은 상한을 꽂아
/// "상한 내 실패 + 뮤텍스 해제"를 실측한다(10s를 그대로 기다리면 스위트가 느려진다).
async fn ensure_browser_gui_peer_on(
    socket: &std::path::Path,
    deadline: std::time::Duration,
) -> Result<(), RpcFailure> {
    let _flight = gui_registration_flight().lock().await;
    // 비행 대기 중 다른 경로가 등록을 마쳤을 수 있다 — 락 **획득 후** 재확인이 핵심이다
    // (획득 전 확인만 하면 두 태스크가 나란히 통과해 교체 경합이 그대로 남는다).
    if BROWSER_APP_SESSION.read().unwrap().is_some() {
        return Ok(());
    }
    register_browser_gui_peer_on(socket, deadline).await
}

/// 등록 체인도 code 보존형(`rpc_on_full_deadline`) — `GUI_REGISTRATION_LIMIT`·`GUI_IDENTITY_REJECTED`·
/// `GUI_REGISTRATION_REPLAY`가 message만 남고 소멸하면 등록 실패 진단이 반쪽이 된다(WS-1).
///
/// 만료(`is_challenge_expired`)는 **1회만** 재발급→재등록한다: challenge 발급과 소비 사이에 부팅
/// 부하가 끼면 TTL을 넘길 수 있는데, 그건 "느렸다"이지 "틀렸다"가 아니라 재시도가 정답이다.
/// 두 번째 만료는 그대로 실패시킨다(무한루프 금지 — 재시도 예산은 정확히 1회).
///
/// ★F-3: 체인의 두 RPC는 각각 `GUI_REGISTRATION_RPC_DEADLINE_SECS` 읽기 상한을 갖는다 —
/// 무응답 cysd가 뮤텍스를 영구 점유하는 hang 경로를 끊는다(만료 재시도는 브로커가 **응답한**
/// 뒤에만 일어나므로 상한이 곱해지지 않는다: 최악도 challenge 10s + register 10s).
async fn register_browser_gui_peer_on(
    socket: &std::path::Path,
    deadline: std::time::Duration,
) -> Result<(), RpcFailure> {
    match register_browser_gui_peer_once_on(socket, deadline).await {
        Err(failure) if failure.is_challenge_expired() => {
            eprintln!(
                "[cys-app] Browser GUI registration challenge expired ([{}] {}) — 재발급 후 1회 재시도",
                failure.code, failure.message
            );
            register_browser_gui_peer_once_on(socket, deadline).await
        }
        other => other,
    }
}

async fn register_browser_gui_peer_once_on(
    socket: &std::path::Path,
    deadline: std::time::Duration,
) -> Result<(), RpcFailure> {
    let challenge = rpc_on_full_deadline(
        socket,
        "browser.runtime.gui_challenge",
        json!({}),
        Some(deadline),
    )
    .await?;
    let registration_secret = challenge["registration_challenge"]
        .as_str()
        .ok_or_else(|| {
            RpcFailure::malformed("cysd returned no Browser GUI registration challenge")
        })?;
    let registration = rpc_on_full_deadline(
        socket,
        "browser.runtime.register_gui",
        json!({"registration_secret":registration_secret}),
        Some(deadline),
    )
    .await?;
    let session = registration["app_session"]
        .as_str()
        .filter(|value| value.len() == 64)
        .ok_or_else(|| RpcFailure::malformed("cysd returned an invalid Browser GUI session"))?
        .to_string();
    // 덮어쓰기: 데몬 재시작으로 대장이 갱신되면 낡은 세션을 새 세션으로 교체한다(OnceLock의
    // 실패-무시 set 대체). write 가드는 이 대입 문에서 즉시 drop된다(await 경계 안 넘김).
    *BROWSER_APP_SESSION.write().unwrap() = Some(session);
    Ok(())
}

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

///
/// `read_deadline`(Some)은 **응답 대기 상한**이다. 초과는 `AfterSend`로 분류해 호출자
/// (`rpc_full`)가 풀 연결을 폐기하게 만든다 — 미수신 응답이 남은 연결을 재사용하면 다음 RPC가
/// stale 프레임을 읽는다(적대검증 R-1). None은 종전 무한 대기(기존 호출부 전부).
async fn rpc_once(
    socket: &std::path::Path,
    conn: &mut Option<tokio::io::BufReader<Stream>>,
    line: &[u8],
    read_deadline: Option<std::time::Duration>,
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
    let read = c.read_line(&mut resp_line);
    let n = match read_deadline {
        Some(limit) => match tokio::time::timeout(limit, read).await {
            Ok(result) => result.map_err(|e| RpcErr::AfterSend(e.to_string()))?,
            Err(_) => {
                return Err(RpcErr::AfterSend(format!(
                    "cysd did not answer in {}s",
                    limit.as_secs()
                )))
            }
        },
        None => read.await.map_err(|e| RpcErr::AfterSend(e.to_string()))?,
    };
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
    rpc_full_deadline(socket, method, params, None).await
}

/// `rpc_full`의 읽기 상한 주입형. 상한 초과는 `AfterSend`로 처리되어 이 함수가 풀 연결을
/// 폐기(`*conn = None`)하므로, 바깥에서 `tokio::time::timeout`으로 감쌌을 때와 달리 취소된
/// 연결이 미수신 응답을 남기지 않는다(적대검증 R-1이 금지한 패턴을 피하는 이유).
async fn rpc_full_deadline(
    socket: &std::path::Path,
    method: &str,
    params: Value,
    read_deadline: Option<std::time::Duration>,
) -> Result<Value, String> {
    let req = json!({"id": 1, "method": method, "params": params});
    let mut line = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    let cell = conn_cell(socket);
    let mut conn = cell.lock().await;
    let resp_line = match rpc_once(socket, &mut conn, &line, read_deadline).await {
        Ok(r) => r,
        Err(RpcErr::BeforeSend(_)) => {
            // 풀링된 연결이 끊겨 전송 자체가 실패 — 데몬이 요청을 못 봤으니 재시도 안전
            *conn = None;
            match rpc_once(socket, &mut conn, &line, read_deadline).await {
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

/// 데몬 실패의 **사인(code)을 보존**하는 실패 값 — `rpc_on`/`rpc_oneshot`이 message만 올려
/// 배너에서 사인이 소멸하던 진단 블랙홀(WS-1 F1·F2)의 수리 단위다. 배너 문구는 기존 대괄호
/// 규약 `BROWSER_DISABLED_SAFE [<CODE>]: <message>`(`[NATIVE_ACTIVATION_REQUIRED]` 선례,
/// `ui/src/castdiag.test.ts` 픽스처)의 확장이며 **신설 형식이 아니다**.
#[derive(Debug, Clone, PartialEq)]
struct RpcFailure {
    code: String,
    message: String,
}

impl RpcFailure {
    /// 전송 계층 실패(연결 끊김·직렬화) — 데몬 응답 자체가 없어 code가 존재하지 않는다.
    fn transport(message: impl Into<String>) -> Self {
        Self {
            code: RPC_TRANSPORT_FAILED_CODE.to_string(),
            message: message.into(),
        }
    }

    /// ok 응답인데 GUI가 요구하는 필드가 없는 경우 — 데몬 실패가 아니라 계약 어긋남이다.
    fn malformed(message: impl Into<String>) -> Self {
        Self {
            code: RPC_MALFORMED_RESPONSE_CODE.to_string(),
            message: message.into(),
        }
    }

    /// `{"ok":false,"error":{...}}` 응답에서 code/message를 뽑는다. code 부재(구 데몬 스큐)는
    /// 자리표 코드로 채워 배너 형식을 유지한다 — message는 기존 계약(`unknown error`)과 동일.
    fn from_error_response(resp: &Value) -> Self {
        Self {
            code: resp["error"]["code"]
                .as_str()
                .filter(|code| !code.is_empty())
                .unwrap_or(RPC_UNCODED_ERROR_CODE)
                .to_string(),
            message: resp["error"]["message"]
                .as_str()
                .unwrap_or("unknown error")
                .to_string(),
        }
    }

    /// GUI 배너 1줄. 사인을 선두에 실어 절단(`castFailureReason`)에도 code가 살아남게 한다.
    fn banner(&self) -> String {
        format!("BROWSER_DISABLED_SAFE [{}]: {}", self.code, self.message)
    }

    /// 데몬 재시작으로 브로커 등록 대장이 소실된 상황인가 — **code 우선**, 구 데몬(코드 미탑재·
    /// 스큐) 대비 marker 문자열 폴백 유지. `GUI_IDENTITY_MISMATCH`도 재등록으로 회복 가능한
    /// 동일 부류다(대장에서 자기 세션이 제거된 뒤이므로 재등록이 정확한 처방).
    fn is_gui_registration_lost(&self) -> bool {
        self.code == GUI_NOT_REGISTERED_CODE
            || self.code == GUI_IDENTITY_MISMATCH_CODE
            || self.is_challenge_expired()
            || self.message.contains(BROKER_REGISTRATION_LOST_MARKER)
    }

    /// 등록 challenge가 **만료**되어 거부됐는가 — 재발급→재등록으로 회복 가능한 유일한 replay 사유.
    /// code 우선(브로커가 전용 code를 내면 그것만 보고), 그 전까지는 message 부분열 폴백이다
    /// (한계는 `BROKER_CHALLENGE_EXPIRED_MARKER` 주석 참조).
    fn is_challenge_expired(&self) -> bool {
        self.code == GUI_CHALLENGE_EXPIRED_CODE
            || (self.code == GUI_REGISTRATION_REPLAY_CODE
                && self.message.contains(BROKER_CHALLENGE_EXPIRED_MARKER))
    }
}

/// 데몬 전체 응답 → `result` 또는 `{code,message}`. `rpc_on`/`rpc_oneshot`의 unwrap과 동일한
/// 판정(ok==true)이며 실패 측만 code를 보존한다.
fn unwrap_rpc_result(resp: Value) -> Result<Value, RpcFailure> {
    if resp["ok"].as_bool() == Some(true) {
        Ok(resp["result"].clone())
    } else {
        Err(RpcFailure::from_error_response(&resp))
    }
}

/// `rpc_on`의 code 보존형 — **연결 모델은 그대로 풀(rpc_full) 경유**다. 브라우저 체인 중
/// 풀을 쓰던 호출(issue_gesture·prepare_embed)과 등록 체인(gui_challenge·register_gui)만
/// 이 함수로 승격한다. 기존 `rpc`/`rpc_on` 소비자는 무영향.
async fn rpc_on_full(
    socket: &std::path::Path,
    method: &str,
    params: Value,
) -> Result<Value, RpcFailure> {
    rpc_on_full_deadline(socket, method, params, None).await
}

/// `rpc_on_full`의 읽기 상한 주입형 — 등록 체인(single-flight 뮤텍스 아래) 전용.
async fn rpc_on_full_deadline(
    socket: &std::path::Path,
    method: &str,
    params: Value,
    read_deadline: Option<std::time::Duration>,
) -> Result<Value, RpcFailure> {
    let resp = rpc_full_deadline(socket, method, params, read_deadline)
        .await
        .map_err(RpcFailure::transport)?;
    unwrap_rpc_result(resp)
}

/// 기본 소켓 code 보존형 RPC (`rpc`의 대응물).
async fn rpc_coded(method: &str, params: Value) -> Result<Value, RpcFailure> {
    rpc_on_full(&default_socket(), method, params).await
}

#[tauri::command]
async fn daemon_status(socket: Option<String>) -> Result<Value, String> {
    rpc_on(
        &resolve_socket(&socket),
        "system.identify",
        json!({"caller": "ui"}),
    )
    .await
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
    // 기존 9개 호출부의 문자열 계약(message만)을 그대로 보존하는 얇은 래퍼.
    rpc_oneshot_checked(socket, method, params)
        .await
        .map_err(|failure| failure.message)
}

/// `rpc_oneshot`의 전송·파싱 본체 — 데몬 응답 **전체**(ok/result/error.code)를 반환한다.
/// ★`rpc_full` 직사용 금지(설계 §3-1): `rpc_full`은 소켓별 풀 셀을 응답까지 점유하므로 ensure
/// 40s 동안 기본 소켓의 전 GUI RPC가 직렬 대기하는 동결을 만든다. 일회성 연결은 드롭이 곧 연결
/// 종료라 공유 상태를 건드리지 않는다.
async fn rpc_oneshot_full(
    socket: &std::path::Path,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let req = json!({"id": 1, "method": method, "params": params});
    let mut line = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    let mut stream = connect_to(socket).await?;
    stream.write_all(&line).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    let n = reader
        .read_line(&mut resp)
        .await
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("connection closed".into());
    }
    serde_json::from_str(resp.trim()).map_err(|e| e.to_string())
}

/// 일회성 연결 + code 보존 — 브라우저 체인(ensure·cancel) 전용.
async fn rpc_oneshot_checked(
    socket: &std::path::Path,
    method: &str,
    params: Value,
) -> Result<Value, RpcFailure> {
    let resp = rpc_oneshot_full(socket, method, params)
        .await
        .map_err(RpcFailure::transport)?;
    unwrap_rpc_result(resp)
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
    let mut targets: Vec<(std::path::PathBuf, String, String)> = vec![(
        default_socket(),
        "_hq".to_string(),
        "본부 · CEO".to_string(),
    )];
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
        let call = tokio::time::timeout(
            Duration::from_secs(2),
            rpc_oneshot(&sock, "org.status", json!({})),
        )
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
    rpc(
        "control.sessions",
        json!({ "window": window, "redact": redact }),
    )
    .await
}

#[tauri::command]
async fn control_session_detail(session_id: String) -> Result<Value, String> {
    rpc(
        "control.session_detail",
        json!({ "session_id": session_id }),
    )
    .await
}

#[tauri::command]
async fn control_session_star(
    session_id: String,
    starred: bool,
    note: Option<String>,
) -> Result<Value, String> {
    rpc(
        "control.session_star",
        json!({ "session_id": session_id, "starred": starred, "note": note }),
    )
    .await
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
    std::env::var("CYS_IME_DEBUG")
        .map(|v| v == "1")
        .unwrap_or(false)
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
            "exe", "bat", "cmd", "com", "scr", "ps1", "msi", "vbs", "vbe", "js", "jse", "wsf",
            "wsh", "hta", "lnk", "reg", "jar", "pif", "scf", "cpl", "msc",
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

/// ~/.cys/viewer/state.json 을 읽어 pid 생존이면 {port, token} 반환, 스테일/부재면 None.
/// 사이드카(javis_view_bridge.py)가 {pid, port, token, ts} 를 원자 기록한다.
fn read_live_view_state(path: &std::path::Path) -> Option<Value> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let pid = v.get("pid").and_then(|p| p.as_i64())?;
    let port = v.get("port").and_then(|p| p.as_u64())?;
    let token = v.get("token").and_then(|t| t.as_str())?.to_string();
    // pid 생존 확인 — kill(pid, 0): 0=생존(권한 있음), ESRCH=사멸. (windows는 생존 가정 — 스테일이면 재기동 대기 타임아웃이 흡수)
    #[cfg(unix)]
    if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    {
        return None;
    }
    let _ = pid;
    Some(json!({"port": port, "token": token}))
}

/// ensure_view_bridge 임계구역 락 — 레이아웃 복원 시 web pane 2+개가 동시에 이 커맨드를
/// invoke 하면 check-then-spawn 사이 갭에서 사이드카가 이중 spawn 되고, state.json 원자 기록의
/// 패자가 고아로 영구 상주한다(view bridge는 유휴종료 없음). 이 프로세스 안의 동시 invoke를
/// 직렬화해 첫 호출만 spawn하고 나머지는 락 획득 후 live state를 재확인해 즉시 반환하게 한다.
/// ※한계: 프로세스 밖 동시성(앱 인스턴스 2개)은 이 락 범위 밖 — 별도 파일락이 필요하다(범위 외).
static VIEW_BRIDGE_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

/// 뷰어 사이드카 확보(§8-1#13) — 살아있으면 {port, token} 즉시 반환, 아니면 detached spawn 후
/// state.json(생존 pid) 을 최대 10초 대기해 반환. lazy 기동(부트체인 무접점)·읽기 전용 loopback.
#[tauri::command]
fn ensure_view_bridge() -> Result<Value, String> {
    let state_path = cys::home_dir().join(".cys/viewer/state.json");
    // 락 밖 빠른 경로 — 이미 살아있으면 직렬화 없이 즉시 반환.
    if let Some(v) = read_live_view_state(&state_path) {
        return Ok(v);
    }
    // 임계구역 진입: spawn 은 프로세스 내 한 번에 하나만.
    let lock = VIEW_BRIDGE_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner()); // poison 무시(임계구역 상태 무보유)
                                                                 // 락 획득 후 재확인(double-checked) — 대기 중 다른 invoke가 이미 기동했으면 spawn 생략.
    if let Some(v) = read_live_view_state(&state_path) {
        return Ok(v);
    }
    let script = cys::pack::pack_dir()
        .join("bin")
        .join("javis_view_bridge.py");
    if !script.exists() {
        return Err(format!("view bridge 스크립트 없음: {}", script.display()));
    }
    let mut cmd = std::process::Command::new("python3");
    inject_runtime_path(&mut cmd); // 동봉 runtime(python3) PATH 주입
    cmd.arg(&script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    no_console(&mut cmd);
    cmd.spawn()
        .map_err(|e| format!("view bridge 기동 실패: {e}"))?;
    // state 대기(0.2s 간격·최대 10s) — 사이드카가 0-bind 포트 확정 후 state.json 을 원자 기록한다.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if let Some(v) = read_live_view_state(&state_path) {
            return Ok(v);
        }
    }
    Err("view bridge state 대기 타임아웃(10s)".into())
}

/// 인증 정보가 없는 구형 Tauri 진입점. Browser v2의 검증된 pane/gesture identity를 만들 수
/// 없으므로 disabled-safe로 거부한다. 실제 지구본 버튼은 `ensure_browserd_cast`를 사용한다.
#[tauri::command]
fn browser_open(url: Option<String>) -> Result<Value, String> {
    let target = url
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "about:blank".into());
    Err(format!(
        "BROWSER_DISABLED_SAFE [USER_GESTURE_REQUIRED]: legacy browser_open cannot authorize or navigate {target}; use the in-pane Browser reconnect action"
    ))
}

fn browser_user_gesture_intent(
    window_label: &str,
    pane_nonce: &str,
    gesture_receipt: &str,
) -> Result<Value, String> {
    if window_label.is_empty()
        || window_label.len() > 64
        || !window_label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid Browser window label".into());
    }
    let valid_credential = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if !valid_credential(pane_nonce) || !valid_credential(gesture_receipt) {
        return Err("Browser pane/gesture receipt must be 32-byte lowercase hex".into());
    }
    Ok(json!({
        "authority_kind":"user_gesture",
        "realm":"shared-default",
        "mode":"headless",
        "window_label":window_label,
        "pane_nonce":pane_nonce,
        "gesture_receipt":gesture_receipt,
    }))
}

#[tauri::command]
fn arm_browser_native_activation(
    window: tauri::WebviewWindow,
    bootstrap_capability: String,
) -> Result<(), String> {
    NATIVE_BROWSER_ACTIVATIONS
        .get()
        .ok_or_else(|| "Browser native activation registry is unavailable".to_string())?
        .lock()
        .unwrap()
        .arm(window.label(), &bootstrap_capability, unix_millis_now())
        .map_err(|error| {
            eprintln!(
                "[cys-app] native activation arm rejected: {error} (window={})",
                window.label()
            );
            error
        })
}

fn browser_embed_intent(
    pane_id: &str,
    embed_ticket: &str,
    embed_generation: u64,
    parent_origin: &str,
) -> Result<Value, String> {
    let valid_credential = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    let expected_origin = if cfg!(windows) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    };
    if !valid_credential(pane_id)
        || !valid_credential(embed_ticket)
        || embed_generation == 0
        || parent_origin != expected_origin
    {
        return Err("invalid Browser embed request".into());
    }
    Ok(json!({
        "context":"default",
        "protocol_version":2,
        "embed_generation":embed_generation,
        "parent_origin":parent_origin,
        "embed_ticket":embed_ticket,
        "pane_id":pane_id,
    }))
}

/// 지구본 버튼의 active Browser 진입점. 검증된 사용자 제스처를 `cysd`의 lazy
/// BrowserAuthorityExtension에 제출하고, 호환 runtime이 준비된 뒤 짧은 수명의
/// `EmbedDescriptor`만 반환한다. Tauri는 실행 파일·PID·port·control token·process lock을
/// 읽거나 소유하지 않는다. cold start는 bounded RPC timeout 안에서 일어나며 앱 부트/복원
/// 경로에서는 이 명령을 호출하지 않는다.
#[tauri::command]
async fn ensure_browserd_cast(
    window: tauri::WebviewWindow,
    pane_nonce: String,
    request_id: String,
    embed_generation: u64,
    parent_origin: String,
    embed_ticket: String,
) -> Result<Value, String> {
    let valid_credential = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if !valid_credential(&request_id) {
        return Err("invalid Browser request identity".into());
    }
    NATIVE_BROWSER_ACTIVATIONS
        .get()
        .ok_or_else(|| "BROWSER_DISABLED_SAFE: native activation registry unavailable".to_string())?
        .lock()
        .unwrap()
        .consume(window.label(), unix_millis_now())
        .map_err(|error| {
            eprintln!(
                "[cys-app] native activation consume rejected: {error} (window={})",
                window.label()
            );
            format!("BROWSER_DISABLED_SAFE [NATIVE_ACTIVATION_REQUIRED]: {error}")
        })?;
    // 지연 재시도(fail-closed는 유지, 무한루프 없음): 부팅 시 1회 등록이 데몬 교체·부팅 경합으로
    // 실패해 세션이 비어 있으면, 신뢰된 클릭 소모 직후 여기서 등록한다. 이미 등록됐으면 통과.
    // ★ensure로 위임 — 부트 재시도 태스크와 이 클릭이 겹칠 때 살아있는 세션을 교체하지 않는다.
    ensure_browser_gui_peer()
        .await
        .map_err(|failure| failure.banner())?;
    let issue_params = browser_session_params(json!({
        "window_label":window.label(),
        "pane_nonce":pane_nonce,
    }))?;
    let issued = match rpc_coded("browser.runtime.issue_gesture", issue_params).await {
        Ok(value) => value,
        // 브로커 등록 대장은 데몬 메모리 — 데몬 재시작 시 소실되므로 낡은 세션 거부를 만나면 1회
        // 재등록·재시도로 자가 회복한다(v0.13.10 실사고: 됐다안됐다 간헐 실패). one-time activation은
        // 이미 consume된 뒤이므로 재시도에 새 클릭이 필요 없다(consume은 함수 초입 1회 — 재시도는 rpc만).
        // ★판정은 code 우선(GUI_NOT_REGISTERED·GUI_IDENTITY_MISMATCH) + marker 문자열 폴백
        // (is_gui_registration_lost). identity mismatch는 종전 marker 매칭에 걸리지 않아 클릭 1회를
        // 통째로 잃었다(브로커가 대장에서 세션을 제거한 직후라 재등록이 정확한 처방이다).
        Err(failure) if failure.is_gui_registration_lost() => {
            // 자기 세션이 브로커에서 무효해졌음을 확인한 경로다 — 여기서만 세션을 비우고(강제
            // 재등록의 전제) ensure를 부른다. 같은 순간 다른 경로가 이미 새 세션을 만들었다면
            // ensure가 그것을 재사용한다(교체 금지).
            *BROWSER_APP_SESSION.write().unwrap() = None;
            ensure_browser_gui_peer()
                .await
                .map_err(|failure| failure.banner())?;
            let retry_params = browser_session_params(json!({
                "window_label":window.label(),
                "pane_nonce":pane_nonce,
            }))?;
            // 재시도도 실패하면 그 에러로 fail-closed(무한루프 금지 — 재등록·재시도는 각 1회 한정).
            rpc_coded("browser.runtime.issue_gesture", retry_params)
                .await
                .map_err(|failure| failure.banner())?
        }
        Err(failure) => return Err(failure.banner()),
    };
    let gesture_receipt = issued["gesture_receipt"].as_str().ok_or_else(|| {
        "BROWSER_DISABLED_SAFE: broker returned no activation receipt".to_string()
    })?;
    let mut intent = browser_user_gesture_intent(window.label(), &pane_nonce, &gesture_receipt)?;
    intent
        .as_object_mut()
        .ok_or_else(|| "invalid Browser launch intent".to_string())?
        .insert("request_id".into(), Value::String(request_id.clone()));
    let ensure = tokio::time::timeout(
        std::time::Duration::from_secs(TAURI_BROWSER_ENSURE_DEADLINE_SECS),
        rpc_oneshot_checked(
            &default_socket(),
            "browser.runtime.ensure",
            browser_session_params(intent)?,
        ),
    )
    .await;
    match ensure {
        Ok(Ok(_)) => {}
        Ok(Err(failure)) => return Err(failure.banner()),
        Err(_) => {
            if let Err(failure) = cancel_browser_request(&pane_nonce, &request_id).await {
                // 취소 실패는 배너를 덮지 않는다(원인은 timeout) — 사인만 로그로 남긴다.
                eprintln!(
                    "[cys-app] browser cancel after ensure timeout failed: [{}] {}",
                    failure.code, failure.message
                );
            }
            return Err(format!(
                "BROWSER_DISABLED_SAFE [RUNTIME_START_TIMEOUT]: broker did not answer in {}s",
                TAURI_BROWSER_ENSURE_DEADLINE_SECS
            ));
        }
    }
    let embed = browser_embed_intent(&pane_nonce, &embed_ticket, embed_generation, &parent_origin)?;
    rpc_coded(
        "browser.runtime.prepare_embed",
        browser_session_params(embed)?,
    )
    .await
    .map_err(|failure| failure.banner())
}

async fn cancel_browser_request(
    pane_nonce: &str,
    request_id: &str,
) -> Result<Value, RpcFailure> {
    rpc_oneshot_checked(
        &default_socket(),
        "browser.runtime.cancel",
        browser_session_params(json!({
            "pane_nonce":pane_nonce,
            "request_id":request_id,
        }))
        .map_err(RpcFailure::malformed)?,
    )
    .await
}

#[tauri::command]
async fn cancel_browserd_cast(pane_nonce: String, request_id: String) -> Result<Value, String> {
    let valid_credential = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if !valid_credential(&pane_nonce) || !valid_credential(&request_id) {
        return Err("invalid Browser cancellation identity".into());
    }
    // 취소 실패는 브라우저 비활성화 사건이 아니므로 BROWSER_DISABLED_SAFE 배너를 쓰지 않는다 —
    // 대괄호 사인 규약만 공유한다.
    cancel_browser_request(&pane_nonce, &request_id)
        .await
        .map_err(|failure| format!("[{}]: {}", failure.code, failure.message))
}

/// 복원용 passive descriptor 조회. `prepare_embed`은 cysd의 이미 호환 판정된 live runtime에만
/// descriptor를 발급하며 절대 ensure/spawn하지 않는다. live면
/// `{alive:true,descriptor:EmbedDescriptor}`, 부재·손상·불일치면 `{alive:false}`다.
#[tauri::command]
async fn browserd_state(
    pane_nonce: String,
    embed_generation: u64,
    parent_origin: String,
    embed_ticket: String,
) -> Value {
    // ★ADV-2: 조기 return 2곳도 사인을 싣는다 — 특히 아래 `browser_session_params` 실패는
    // `BROWSER_APP_SESSION`이 비어 있다는 뜻(= GUI peer 미등록)이고, 부팅 등록 실패·데몬 교체
    // 직후의 **가장 흔한 실제 고장**이다. 여기서 침묵하면 사용자는 종전과 글자 하나 다르지 않은
    // "브라우저 꺼짐 — 클릭해 재연결"만 보고, `castStateReason` 배선이 최빈 경로에서 죽는다.
    let embed = match browser_embed_intent(
        &pane_nonce,
        &embed_ticket,
        embed_generation,
        &parent_origin,
    ) {
        Ok(embed) => embed,
        Err(message) => {
            return json!({
                "alive":false,
                "code":EMBED_INTENT_INVALID_CODE,
                "message":message,
            })
        }
    };
    let embed = match browser_session_params(embed) {
        Ok(embed) => embed,
        // 데몬이 준 code가 아니라 GUI가 판정한 등록 부재다 — 브로커의 동명 사인과 같은 처방
        // (클릭 → ensure_browser_gui_peer 재등록)으로 수렴하므로 같은 code를 쓴다.
        Err(message) => {
            return json!({
                "alive":false,
                "code":GUI_NOT_REGISTERED_CODE,
                "message":message,
            })
        }
    };
    // ★WS-1 code 전파의 논리적 연장(설계 §7-0-A 3): 종전에는 모든 Err를 `{alive:false}`로 뭉개
    // passive 경로(pane 복원·5s 재시도)에서 실패 사유가 통째로 소실됐다 — UNSUPPORTED·
    // GUI_IDENTITY_MISMATCH 같은 사인이 사용자에게 영영 도달하지 못한다. code/message를 함께 싣되
    // alive 계약은 불변이라 기존 소비자(`state.alive` 판정)는 무영향이다.
    match rpc_on_full(&default_socket(), "browser.runtime.prepare_embed", embed).await {
        Ok(descriptor) => json!({"alive":true,"descriptor":descriptor}),
        Err(failure) => json!({
            "alive":false,
            "code":failure.code,
            "message":failure.message,
        }),
    }
}

/// 파일 관리자에서 해당 항목을 선택해 보여준다 (macOS Finder reveal / Windows explorer select).
/// open_path와 동일한 실재 경로 게이트 — URL 스킴·비존재 문자열 차단.
#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    std::fs::metadata(&path).map_err(|e| format!("not a local path: {e}"))?;
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn();
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
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| "https only".to_string())?;
    // authority는 첫 '/', '?'(query), '#'(fragment) 전까지(RFC 3986) — query/fragment 사칭 우회 차단.
    let authority = rest
        .split(|c: char| c == '/' || c == '?' || c == '#')
        .next()
        .unwrap_or("");
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
        out.extend(
            s.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#')),
        );
    }
    if let Ok(env) = std::env::var("CYS_URL_ALLOW_HOSTS") {
        out.extend(
            env.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );
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
fn build_install_script(cys: &std::path::Path, cysd: &std::path::Path, target_dir: &str) -> String {
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
        let macos_dir = exe.parent().ok_or("번들 디렉토리 해석 실패")?.to_path_buf();

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
    let mut init_cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) {
        "cys.exe"
    } else {
        "cys"
    }));
    init_cmd.arg("init-pack").arg("--no-install-hook");
    no_console(&mut init_cmd);
    let pack_ok = init_cmd.status().map(|s| s.success()).unwrap_or(false);
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
        let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) {
            "cys.exe"
        } else {
            "cys"
        }));
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
                    v.get("dept_tombstones")
                        .and_then(|a| a.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
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
                            let _ = stop_dept_daemon_by_socket(sock.to_string_lossy().to_string())
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
    rpc_on(
        &resolve_socket(&socket),
        "surface.send_key",
        json!({"surface_id": surface_id, "key": key}),
    )
    .await
    .map(|_| ())
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
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.is_empty() {
        "skill".to_string()
    } else {
        base
    };
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
        .args([
            "--task",
            &task,
            "--scope",
            &scope_full,
            "--success",
            &success,
            "--to",
            &to,
        ])
        .arg("--no-survival-gate")
        .args(["--output-format", &out_fmt]);
    no_console(&mut orch_cmd);
    let output = orch_cmd
        .output()
        .map_err(|e| format!("javis_orchestra 실행 실패: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "task-prompt 실패: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
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
    cmd.arg("skill")
        .arg("run")
        .arg(&name)
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
    let script = cys::pack::pack_dir()
        .join("bin")
        .join("javis_resource_gate.py");
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
            let report = serde_json::from_slice::<Value>(&out.stdout).unwrap_or(Value::Null);
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
        (
            x["provider"].as_str().unwrap_or(""),
            x["label"].as_str().unwrap_or(""),
        )
            .cmp(&(
                y["provider"].as_str().unwrap_or(""),
                y["label"].as_str().unwrap_or(""),
            ))
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
async fn rename_surface(
    socket: Option<String>,
    surface_id: u64,
    title: String,
) -> Result<(), String> {
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
async fn allocate_dept_daemon(
    app: AppHandle,
    catalog_key: Option<String>,
) -> Result<Value, String> {
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
        .unwrap_or_else(|_| cys::home_dir().join(".cys/depts.json"));
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
        .unwrap_or_else(|_| cys::home_dir().join(".cys/depts.json"));
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
        .unwrap_or_else(|_| cys::home_dir().join(".cys/dept-catalog.json"));
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
    rpc_oneshot(
        &cys::socket_path(),
        "dept_tombstone.set",
        json!({"name": name}),
    )
    .await
}

/// ★WP-3 리바이버 게이트 소스: base 데몬의 dept 묘비 목록(프론트 복원이 유령 판정에 사용).
#[tauri::command]
async fn dept_tombstones() -> Result<Vec<String>, String> {
    let v = rpc_oneshot(&cys::socket_path(), "dept_tombstone.list", json!({})).await?;
    Ok(v.get("dept_tombstones")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
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
        cmd.arg("launch-agent")
            .arg("--role")
            .arg("master")
            .arg("--agent")
            .arg("claude");
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        spawn_orchestra_boot(app, None); // ★절대규칙: 마스터=4노드 팀 결정론 스폰(LLM 환각 무관)
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// `cys boot` 기계판독 요약 줄의 파싱 결과(결함 A 수리 · 2026-07-25).
#[derive(Debug, Clone, PartialEq, Eq)]
struct BootMachineSummary {
    /// 판정에는 쓰지 않는다(신규 기동 0 = 전원 가동 중인 정상 no-op boot일 수 있다 — 이 값을
    /// 판정에 넣은 것이 원 오탐이었다). 계약 완결성·디버깅·회귀 핀용으로만 파싱한다.
    #[allow(dead_code)]
    launched: u32,
    failed: u32,
    /// 의무 4종(cso·worker·reviewer-gemini·reviewer-codex)이 CLI 미발견으로 skip된 수.
    /// 선택 노드 grok의 미설치는 여기에 **포함되지 않는다** — 이것이 오탐 제거의 핵심.
    mandatory_missing: u32,
}

/// `boot-machine: launched=N failed=N mandatory_missing=N` 줄 파싱(순수 함수).
/// ★계약(단일 진실): 이 형식의 **생성자**는 `src/bin/cys.rs::boot_machine_line()` — 형식 변경은
/// 반드시 양쪽 동시 수정. 줄이 없으면(=구 cys 사이드카와의 버전 스큐) None → 호출자가 구
/// 휴리스틱으로 폴백한다. 알 수 없는 키는 무시(전방 호환), 세 키가 다 모여야 Some.
/// 마지막 매칭 줄을 채택 — 재실행 로그가 앞에 섞여도 최신 요약이 이긴다.
fn parse_boot_machine_line(stdout: &str) -> Option<BootMachineSummary> {
    let body = stdout
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("boot-machine:"))?;
    let (mut launched, mut failed, mut mandatory_missing) = (None, None, None);
    for tok in body.split_whitespace() {
        let Some((k, v)) = tok.split_once('=') else {
            continue;
        };
        let Ok(n) = v.parse::<u32>() else { continue };
        match k {
            "launched" => launched = Some(n),
            "failed" => failed = Some(n),
            "mandatory_missing" => mandatory_missing = Some(n),
            _ => {}
        }
    }
    Some(BootMachineSummary {
        launched: launched?,
        failed: failed?,
        mandatory_missing: mandatory_missing?,
    })
}

/// 의무 노드 CLI 부재 안내(자가 진단 가능한 구체 원인).
const BOOT_WARN_MISSING_CLI: &str = "마스터는 시작됐으나 팀(CSO·워커·리뷰어) 기동에 실패했습니다 — claude CLI가 설치돼 있는지 확인하세요(설치: curl -fsSL https://claude.ai/install.sh | bash 후 재시도). 팀 없이도 마스터 단독 사용은 가능합니다.";
/// 원인 중립 안내 — 기동 실패·비정상 종료는 CLI 미설치라 단정할 수 없다(오진 문구 금지).
const BOOT_WARN_PARTIAL: &str = "팀 일부 노드 기동에 실패했습니다 — `cys list`로 현황을 확인하세요. 팀 없이도 마스터 단독 사용은 가능합니다.";

/// `cys boot` 결과 → 경고 배너 문구(Some) 또는 무경고(None). 순수 함수 — 회귀 핀.
///
/// ★결함 A 수리(2026-07-25): 종전 판정은 사람용 요약 줄의 문자열 힌트("신규 기동 0" ∧ "미설치")
/// 였는데, 팀 5노드가 전부 살아 있어도 **선택 노드 grok의 "미설치" 한 줄** 때문에 조건이 참이 돼
/// 오탐 배너가 떴다(게다가 문구는 "claude CLI 확인"으로 오진). 이제 cys가 찍는 기계 줄의
/// `mandatory_missing`(의무 4종만 셈)으로 판정한다.
/// 기계 줄이 없으면(구 사이드카) 구 휴리스틱을 그대로 폴백 — 배너 자체가 사라지지는 않게.
fn boot_warning_message(stdout: &str, exit_ok: bool) -> Option<&'static str> {
    match parse_boot_machine_line(stdout) {
        Some(m) => {
            if m.mandatory_missing > 0 {
                Some(BOOT_WARN_MISSING_CLI)
            } else if !exit_ok || m.failed > 0 {
                Some(BOOT_WARN_PARTIAL)
            } else {
                None
            }
        }
        None => {
            let launched_zero = stdout.contains("신규 기동 0");
            let has_missing = stdout.contains("미설치");
            if !exit_ok || (launched_zero && has_missing) {
                Some(BOOT_WARN_MISSING_CLI)
            } else {
                None
            }
        }
    }
}

/// ★결함 B 수리(2026-07-25): 배너 **소멸 신호**의 canonical 발행자를 버튼 레인에도 배선한다.
/// 배경: boot-warning 배너의 유일한 소멸 신호는 feed `formation-complete`(ui/src/bootbanner.ts
/// W3/T3)인데, 그 발행자 `javis_formation.py ensure`는 훅 레인(javis_bootstrap.py)과 부서 생성
/// 레인(cys-dept `_fire_formation`)에만 배선돼 있었다. 버튼 경로(spawn_orchestra_boot)에는
/// formation 호출이 0줄이라 한 번 뜬 배너가 팀이 정상화돼도 영영 사라지지 않았다
/// (라이브 증거 2026-07-25: ~/.cys/state/formation/ 빈 디렉토리 · feed.jsonl formation-* 0건).
///
/// 소켓 규약 — **javis_bootstrap.py `_run_formation_ensure` 와 바이트 동일 규약**:
///   본부(None) = `--socket` 인자 없음 + CYS_SOCKET 제거 / 부서(Some) = `--socket <소켓경로>` + env.
/// ⚠`resolve_socket()`이 만드는 기본 소켓 **경로**를 본부에서 넘기면 안 된다 —
/// javis_formation.py `_sanitize_key()`가 None→"base", 경로→"_Users_..._cys.sock"로 다른 키를
/// 만들어 상태 파일과 **싱글플라이트 락 키가 레인마다 갈라진다**(훅 레인과 동시 실행 시 상호배제
/// 소멸 = 중복 스폰). 인자 형식은 소켓 **경로**가 맞다(usage `ensure --socket <S>`, cys-dept가
/// 부서 소켓 경로를 그대로 전달) — 본부만 "인자 생략"이 규약이다.
/// 배너 스코프(sock_slug)는 formation feed를 중계하는 데몬이 결정하므로 이 인자와 무관하다.
///
/// 전부 best-effort — 팩 부재·실행 실패·타임아웃 어느 것도 배너 로직이나 부트 결과를 바꾸지 않는다.
fn fire_formation_ensure(socket: &Option<String>) {
    let script = cys::pack::pack_dir()
        .join("bin")
        .join("javis_formation.py");
    if !script.exists() {
        return; // 팩 미설치/미시드 = 구동작 보존(무해 skip · cys-dept _fire_formation 과 동일)
    }
    let mut cmd = std::process::Command::new("python3");
    inject_runtime_path(&mut cmd); // RC-5: 동봉 runtime(python3.exe) PATH 주입
    match socket {
        Some(s) => {
            cmd.env("CYS_SOCKET", s);
            cmd.arg(&script).arg("ensure").arg("--socket").arg(s);
        }
        None => {
            cmd.env_remove("CYS_SOCKET");
            cmd.arg(&script).arg("ensure");
        }
    }
    // ★부트 레인 강제 표면화(2026-07-26): 표면화가 '전이 시에만'으로 좁혀지면서(주기 10분 잡의 토스트
    // 스팸 차단) **앱 재시작**이라는 구멍이 생겼다 — UI 의 배너 중복 억제 상태(bootbanner.ts
    // lastFormationKind)는 인메모리라 새 앱 세션에서 초기화되는데, 상태 파일이 이미 complete 인 레인은
    // 전이가 없어 formation-complete 를 다시 발행하지 않는다 → 새 세션에서 뜬 boot-warning 배너에
    // 소멸 신호가 영영 오지 않는다. 부트는 세션당 1회뿐이라 스팸이 아니다(주기 잡에는 절대 금지).
    cmd.arg("--force-surface");
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    no_console(&mut cmd);
    // 상한 감시(cys.rs 헤드리스 호출과 동일한 try_wait 폴링 패턴) — ensure 내부 _boot_node 는
    // 노드당 200s 를 쓸 수 있어 무한 대기가 가능하다. 이 스레드는 boot 이후의 꼬리라 UI 를 막지
    // 않지만, 좀비 python 이 남지 않도록 60s 에서 kill 한다(다음 boot 가 멱등하게 재시도한다).
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[formation] ensure 기동 실패(무해 skip): {e}");
            return;
        }
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // zombie 수거
                    eprintln!("[formation] ensure 60초 타임아웃 — 종료(다음 boot 가 멱등 재시도)");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => {
                eprintln!("[formation] ensure 대기 실패(무해 skip): {e}");
                break;
            }
        }
    }
}

/// ★절대규칙(오너 2026-07-15): 모든 마스터(본부·부서장)는 CSO·워커·리뷰어2 팀을 반드시 갖는다.
/// 종전에는 이 팀 스폰이 마스터 LLM의 `cys boot`(디렉티브 §0 ④) 실행에 의존했는데, dept-master가
/// "부서장 스코프=단독 대기"를 **환각**해 boot를 건너뛰는 치명 실사고가 발생했다(2026-07-15). 산문
/// 의존을 제거하고 버튼 경로에서 `cys boot`를 코드 결정론으로 강제한다 — cys boot는 이미 가동 중인
/// 역할을 건너뛰고(멱등·boot 락으로 동시 boot 직렬화) 마스터가 나중에 스스로 boot해도 중복 없음.
/// ★관측성(적대검증 D-8): 종전 `let _ = status()`는 실패를 삼켜 claude 미설치 등으로 팀이 0개여도
/// 사용자가 몰랐다(원 증상 재현·더 나쁨). 진짜 결손이면 boot-warning 이벤트로 승격한다 — 승격
/// 판정은 `boot_warning_message()`(기계 줄 기반)이며, 종전의 "신규 기동 0" 휴리스틱은 구 사이드카
/// 폴백으로만 남았다(결함 A 2026-07-25). 배너의 **소멸** 신호는 이어지는 `fire_formation_ensure()`
/// 가 만든다(결함 B — 버튼 레인에 formation 발행자가 없어 배너가 영구 잔류했다).
/// fire-and-forget(최대 300s라 UI 무블록). socket=Some이면 그 부서 소켓 대상, None이면 본부.
fn spawn_orchestra_boot(app: AppHandle, socket: Option<String>) {
    let cys = resolve_sidecar("cys");
    // ★W3(T3 FIX-1): 배너를 소켓 스코프로 발화 — payload 에 소켓 slug 동봉. UI 가 `boot-warn:<slug>`
    // 스코프 배너를 만들고, formation feed 이벤트의 socket_slug 와 일치할 때만 소멸/갱신한다(다른
    // 부서 데몬의 formation-complete 가 base 배너를 오소멸하는 C6 위반 차단). slug 파생은 formation
    // feed 의 socket_slug(spawn_event_forwarder)와 **동일 함수·동일 경로**라 결정론적으로 일치한다
    // (본부=default_socket / 부서=Some(socket)).
    let slug = sock_slug(&resolve_socket(&socket));
    tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&cys);
        inject_runtime_path(&mut cmd);
        match &socket {
            Some(s) => {
                cmd.env("CYS_SOCKET", s);
            }
            None => {
                cmd.env_remove("CYS_SOCKET");
            }
        }
        cmd.arg("boot");
        no_console(&mut cmd);
        match cmd.output() {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                // 판정은 기계 줄(boot-machine:) 기반 — 사람용 문구 휴리스틱은 폴백으로만 남는다.
                // (구 판정은 선택 노드 grok 의 "미설치" 한 줄에 오탐했다 — boot_warning_message 주석.)
                if let Some(message) = boot_warning_message(&stdout, o.status.success()) {
                    let _ = app.emit(
                        "boot-warning",
                        json!({"slug": slug.clone(), "message": message}),
                    );
                }
            }
            Err(e) => {
                let _ = app.emit(
                    "boot-warning",
                    json!({"slug": slug.clone(), "message": format!("팀 기동(cys boot) 실행 실패: {e}")}),
                );
            }
        }
        // ★결함 B 수리: 경고 여부·boot 성패와 **무관하게** 편성 상태 발행자를 돌린다 — 이 호출이
        // 배너의 유일한 소멸 신호(feed formation-complete)를 만든다. boot 가 no-op(전원 가동)이어도
        // 반드시 필요하다: 이전에 뜬 배너를 걷어내는 건 boot 결과가 아니라 formation 상태다.
        fire_formation_ensure(&socket);
    });
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
#[tauri::command]
async fn start_dept_master(app: AppHandle, socket: String) -> Result<(), String> {
    let cys = resolve_sidecar("cys");
    let socket_boot = socket.clone(); // 아래 orchestra boot용(첫 클로저가 socket을 move)
    let out = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&cys);
        inject_runtime_path(&mut cmd);
        cmd.env("CYS_SOCKET", &socket);
        cmd.arg("launch-agent")
            .arg("--role")
            .arg("master")
            .arg("--agent")
            .arg("claude");
        no_console(&mut cmd);
        cmd.output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        spawn_orchestra_boot(app, Some(socket_boot)); // ★절대규칙: 부서장도 4노드 팀 결정론 스폰(환각 무관)
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
async fn rotate_dept_daemon(
    app: AppHandle,
    name: String,
    force: bool,
    skip_drain: bool,
) -> Result<Value, String> {
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
            let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) {
                "cys.exe"
            } else {
                "cys"
            }));
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
        Ok(Ok(out)) => {
            return Err(format!(
                "pack-manifest HTTP 실패(code {:?})",
                out.status.code()
            ))
        }
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
    let binary_too_old =
        pack_binary_too_old(&manifest.min_binary_version, env!("CARGO_PKG_VERSION"));
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
    match (
        cys::pack::parse_semver(min),
        cys::pack::parse_semver(running),
    ) {
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
        let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) {
            "cys.exe"
        } else {
            "cys"
        }));
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
            let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) {
                "cys.exe"
            } else {
                "cys"
            }));
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
        let mut cmd = std::process::Command::new(resolve_sidecar(if cfg!(windows) {
            "cys.exe"
        } else {
            "cys"
        }));
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
    let bootstrap_capability = random_secret_hex().expect("Browser activation CSPRNG unavailable");
    let mut activation_registry = NativeActivationRegistry::default();
    activation_registry
        .install_window("main", &bootstrap_capability)
        .expect("Browser activation registry initialization failed");
    NATIVE_BROWSER_ACTIVATIONS
        .set(Mutex::new(activation_registry))
        .unwrap_or_else(|_| panic!("Browser activation registry already initialized"));
    let activation_script = browser_activation_initialization_script(&bootstrap_capability);
    tauri::Builder::default()
        .append_invoke_initialization_script(activation_script)
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
            ensure_view_bridge,
            browser_open,
            arm_browser_native_activation,
            ensure_browserd_cast,
            cancel_browserd_cast,
            browserd_state,
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
                // ★부트 등록 재시도(설계 §6-3): 종전에는 1회 실패를 로그만 찍고 흘려보내 GUI가
                // 세션 없이 부팅했다(첫 클릭이 통째로 지연 재등록에 소모 — 실패하면 그대로 죽음).
                // 재시도는 **부트 임계 경로 밖**에서만 한다 — spawn으로 떼어내 daemon-ready·
                // event-forwarder·온보딩을 45초간 막지 않는다.
                if let Err(failure) = ensure_browser_gui_peer().await {
                    eprintln!(
                        "[cys-app] Browser GUI registration failed closed: [{}] {} — 15s 간격 3회 재시도",
                        failure.code, failure.message
                    );
                    let retry_handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        let mut last = failure;
                        for attempt in 1..=3u32 {
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            // ★조기 종료(이 브랜치가 신설한 경합의 봉합): 15s 잠든 사이 클릭 경로가
                            // 이미 등록을 마쳤을 수 있다. `GUI_ALREADY_REGISTERED` 폐기(WS-4) 이후
                            // 두 번째 등록은 거부가 아니라 **교체**라, 그대로 재시도하면 살아있는
                            // 세션 S1을 S2로 갈아치워 in-flight RPC가 GUI_IDENTITY_MISMATCH를 맞는다.
                            // (ensure 안에도 같은 확인이 있지만 여기서 먼저 끊어 남은 재시도 예산과
                            //  최종 실패 배너까지 함께 소멸시킨다 — 성공했는데 배너가 뜨는 모순 방지.)
                            if BROWSER_APP_SESSION.read().unwrap().is_some() {
                                eprintln!(
                                    "[cys-app] Browser GUI registration already recovered elsewhere — 부트 재시도 {attempt}/3 종료"
                                );
                                return;
                            }
                            match ensure_browser_gui_peer().await {
                                Ok(()) => {
                                    eprintln!(
                                        "[cys-app] Browser GUI registration recovered on retry {attempt}/3"
                                    );
                                    return;
                                }
                                Err(failure) => {
                                    eprintln!(
                                        "[cys-app] Browser GUI registration retry {attempt}/3 failed: [{}] {}",
                                        failure.code, failure.message
                                    );
                                    last = failure;
                                }
                            }
                        }
                        // 최종 실패만 배너로 소재화한다 — 사인이 선두에 실린 대괄호 규약.
                        let _ = retry_handle.emit("browser-disabled-safe", last.banner());
                    });
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

    /// ★불변식 박제(결함 A 수리 2026-07-25) ①: 기계 줄 정상 파싱 — 생성자는
    /// src/bin/cys.rs::boot_machine_line(). 이 리터럴이 그쪽 테스트와 **글자 그대로** 같아야 한다.
    #[test]
    fn boot_machine_line_parses_cys_contract_format() {
        let out = "cys boot — LLM orchestrating 편성 점검\n\
                   boot-machine: launched=3 failed=1 mandatory_missing=2\n\
                   boot 완료: 신규 기동 3 · 실패 1 · 현황은 `cys list`로 확인 (role 열)\n";
        assert_eq!(
            parse_boot_machine_line(out),
            Some(BootMachineSummary {
                launched: 3,
                failed: 1,
                mandatory_missing: 2
            })
        );
    }

    /// ②오탐 케이스 박제 — 원 결함의 실측 stdout(2026-07-25): 팀 5노드 전원 가동이라 신규 기동 0,
    /// 선택 노드 grok 만 "미설치". 구 휴리스틱은 여기서 경고를 띄웠다(오탐). 이제 무경고여야 한다.
    #[test]
    fn boot_warning_silent_when_only_optional_grok_missing() {
        let out = "· claude: 역할 'cso' 이미 가동 중 — 건너뜀\n\
                   · claude: 역할 'worker' 이미 가동 중 — 건너뜀\n\
                   · gemini: 역할 'reviewer-gemini' 이미 가동 중 — 건너뜀\n\
                   · codex: 역할 'reviewer-codex' 이미 가동 중 — 건너뜀\n\
                   · grok: CLI 'grok' 미설치 — 건너뜀 (선택 노드 — 미설치면 건너뜀이 정상)\n\
                   boot-machine: launched=0 failed=0 mandatory_missing=0\n\
                   boot 완료: 신규 기동 0 · 실패 0 · 현황은 `cys list`로 확인 (role 열)\n";
        assert_eq!(boot_warning_message(out, true), None);
        // 구 휴리스틱이 참이 되는 입력임을 함께 박제(회귀 시 이 줄이 먼저 깨진다).
        assert!(out.contains("신규 기동 0") && out.contains("미설치"));
    }

    /// ③진성 결손 — 의무 4종이 claude 미설치로 빠지면 경고 + CLI 설치 안내(원인 구체).
    /// 반대로 failed>0·exit≠0 은 원인 중립 문구여야 한다(claude 미설치라 단정하면 오진).
    #[test]
    fn boot_warning_distinguishes_missing_cli_from_launch_failure() {
        let missing = "boot-machine: launched=0 failed=0 mandatory_missing=2\n";
        assert_eq!(boot_warning_message(missing, true), Some(BOOT_WARN_MISSING_CLI));
        let failed = "boot-machine: launched=1 failed=2 mandatory_missing=0\n";
        assert_eq!(boot_warning_message(failed, true), Some(BOOT_WARN_PARTIAL));
        // exit≠0 인데 기계 줄은 깨끗한 경우(락 경합 후 비정상 종료 등)도 중립 경고.
        let clean = "boot-machine: launched=0 failed=0 mandatory_missing=0\n";
        assert_eq!(boot_warning_message(clean, false), Some(BOOT_WARN_PARTIAL));
    }

    /// ④기계 줄 부재(구 cys 사이드카 버전 스큐) → None → 구 휴리스틱 폴백이 살아 있어야 한다.
    /// 스큐 상태에서 배너가 통째로 사라지면 C6(침묵 금지) 위반이라 폴백은 의도적으로 남긴다.
    #[test]
    fn boot_machine_line_absent_falls_back_to_legacy_heuristic() {
        let legacy = "· claude: CLI 'claude' 미설치 — 건너뜀 (설치: …)\n\
                      boot 완료: 신규 기동 0 · 실패 0 · 현황은 `cys list`로 확인 (role 열)\n";
        assert_eq!(parse_boot_machine_line(legacy), None);
        assert_eq!(boot_warning_message(legacy, true), Some(BOOT_WARN_MISSING_CLI));
        // 정상 구버전 출력(미설치 없음)은 폴백에서도 무경고.
        let legacy_ok = "boot 완료: 신규 기동 4 · 실패 0 · 현황은 `cys list`로 확인 (role 열)\n";
        assert_eq!(boot_warning_message(legacy_ok, true), None);
        // 손상된 기계 줄(키 결손)도 None → 폴백(전방호환: 모르는 키만 있으면 무시).
        assert_eq!(parse_boot_machine_line("boot-machine: launched=1\n"), None);
    }

    #[test]
    fn tauri_browser_deadline_outlives_cysd_worker_deadline() {
        assert!(
            TAURI_BROWSER_ENSURE_DEADLINE_SECS
                > cys::browser_runtime::ENSURE_WORKER_DEADLINE.as_secs()
        );
    }

    #[test]
    fn native_browser_activation_is_window_bound_ttl_and_single_consume() {
        let mut registry = NativeActivationRegistry::default();
        let capability = "a".repeat(64);
        registry.install_window("main", &capability).unwrap();
        assert!(registry.arm("other", &capability, 1_000).is_err());
        assert!(registry.arm("main", &"b".repeat(64), 1_000).is_err());
        registry.arm("main", &capability, 1_000).unwrap();
        registry.consume("main", 1_001).unwrap();
        assert!(registry.consume("main", 1_002).is_err());
        registry.arm("main", &capability, 2_000).unwrap();
        assert!(registry
            .consume("main", 2_000 + NATIVE_ACTIVATION_TTL_MS + 1)
            .is_err());
    }

    #[test]
    fn initialization_script_keeps_capability_lexical_and_requires_trusted_activation() {
        let capability = "c".repeat(64);
        let script = browser_activation_initialization_script(&capability);
        assert!(script.contains("event.isTrusted"));
        // 방어적 userActivation 가드(API 부재 시 통과) — isTrusted가 1차 게이트.
        assert!(script.contains("navigator.userActivation"));
        assert!(script.contains("isActive"));
        assert!(script.contains("#btn-browser"));
        assert!(script.contains("[data-browser-reconnect='true']"));
        assert!(script.contains("arm_browser_native_activation"));
        assert_eq!(script.matches(&capability).count(), 1);
        assert!(!script.contains("window.browser"));
        assert!(!script.contains("globalThis.browser"));
        assert!(script.starts_with(';'), "push_str 연결 경계 — 파스 분리자 필수");
        assert!(script.contains("__CYS_BROWSER_ACTIVATION__"));
        assert!(script.contains("arm-failed"));
        // 가드 탈락 진단 표식 — 각 조기 return 지점을 skip-*로 특정한다.
        assert!(script.contains("skip-untrusted"));
        assert!(script.contains("skip-inactive-ua"));
        assert!(script.contains("skip-not-element"));
        assert!(script.contains("skip-nomatch:"));
        // replay(합성 클릭)가 armed-ok를 덮지 않도록 armed 접두 가드가 존재해야 한다.
        assert!(script.contains("startsWith('armed')"));
    }

    /// tauri Builder::append_invoke_initialization_script는 push_str 문자열 연결이고, 선행
    /// 기본 스크립트(ipc-protocol.js)는 `})()` + 개행(식)으로 끝난다(2.11.2 실측). 뒤 스크립트가
    /// 식-연속 토큰으로 시작하면 ASI 미발동으로 `})()\n(...)` 호출 연쇄가 되어 **통째로 삼켜진다**
    /// (v0.13.6 실사고 — 인터셉터 영구 미설치). 파서 의존성 없이 술어로 경계를 봉쇄한다:
    /// `)` 뒤에 식을 잇는 시작 토큰이면 위험. 안전 충분조건은 첫 바이트 `;`(빈 문장 분리자).
    fn concatenation_swallow_hazard(appended: &str) -> bool {
        matches!(
            appended.trim_start().chars().next(),
            Some('(' | '[' | '`' | '.' | '+' | '-' | '*' | '/' | ',' | '?')
        )
    }

    #[test]
    fn activation_script_survives_ipc_protocol_concatenation() {
        let script = browser_activation_initialization_script(&"c".repeat(64));
        assert!(
            !concatenation_swallow_hazard(&script),
            "활성화 스크립트가 ipc-protocol.js 말미의 식에 흡수된다"
        );
        // trim 이전의 실제 첫 바이트가 `;`여야 결정론이 완성된다(공백 시작도 불허)
        assert_eq!(script.as_bytes().first(), Some(&b';'));
    }

    #[test]
    fn concatenation_detector_catches_the_original_swallow_bug() {
        // 음성 대조군(계측 타당성 게이트) — v0.13.6 실사고 형태(`;` 부재)를 탐지기가 실제로
        // 잡는지 스스로 증명한다. 이 테스트가 없으면 탐지기 자체의 무력화를 알 수 없다.
        let current = browser_activation_initialization_script(&"c".repeat(64));
        let buggy = current
            .strip_prefix(';')
            .expect("양성 전제: 현행 스크립트는 ;로 시작");
        assert!(
            concatenation_swallow_hazard(buggy),
            "탐지기가 원본 결함(호출 연쇄 삼킴)을 잡지 못하면 계측 무효"
        );
    }

    /// tauri 2.11.2 실측 초기화 순서상 활성화 스크립트는 invoke init(ipc-protocol) 단계에서
    /// 실행되고, `__TAURI_INTERNALS__.invoke`는 그보다 뒤인 core.js에서야 정의된다. 따라서
    /// **설치 시점(리스너 등록 전) 코드가 `__TAURI_INTERNALS__`를 접촉하면** `undefined.bind`
    /// TypeError로 리스너가 미등록된다(v0.13.7 실사고). 술어: 스크립트에서 `addEventListener`
    /// 첫 등장 이전 구간에 `__TAURI_INTERNALS__`가 포함되면 위험(true). invoke 조회는 반드시
    /// 클릭 핸들러(=`addEventListener` 콜백) 안에서 지연 수행돼야 한다.
    fn install_time_internals_capture(script: &str) -> bool {
        match script.find("addEventListener") {
            Some(idx) => script[..idx].contains("__TAURI_INTERNALS__"),
            // addEventListener가 아예 없으면 리스너 미등록 — 접촉 여부와 무관하게 위험 취급.
            None => true,
        }
    }

    #[test]
    fn activation_script_defers_invoke_lookup_past_install_time() {
        let script = browser_activation_initialization_script(&"c".repeat(64));
        assert!(
            !install_time_internals_capture(&script),
            "설치 시점(리스너 등록 전)에 __TAURI_INTERNALS__를 접촉하면 core.js 이전 실행이라 \
             undefined.bind TypeError로 리스너가 미등록된다 — invoke는 클릭 시점 지연 조회여야 한다"
        );
    }

    #[test]
    fn install_time_capture_detector_catches_the_v0137_bug() {
        // 음성 대조군(계측 타당성 게이트) — v0.13.7 실사고 형태(설치 시점에 invoke를 bind하는
        // 라인)를 탐지기가 실제로 잡는지 스스로 증명한다. 현행 스크립트의 `mark('installed');`
        // 직후에 실사고 라인을 합성 삽입해 addEventListener 이전 구간에 __TAURI_INTERNALS__를
        // 넣는다. 이 테스트가 없으면 탐지기 자체가 무력화돼도 알 수 없다.
        let current = browser_activation_initialization_script(&"c".repeat(64));
        let buggy = current.replacen(
            "mark('installed');",
            "mark('installed');\n  const invoke = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);",
            1,
        );
        assert_ne!(buggy, current, "양성 전제: 합성 삽입이 실제로 일어나야 한다");
        assert!(
            install_time_internals_capture(&buggy),
            "탐지기가 v0.13.7 실사고(설치 시점 __TAURI_INTERNALS__.invoke 접촉)를 잡지 못하면 계측 무효"
        );
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

    #[test]
    fn browser_user_gesture_intent_matches_broker_public_contract() {
        let intent = browser_user_gesture_intent(
            "main",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        assert_eq!(intent["authority_kind"], "user_gesture");
        assert_eq!(intent["realm"], "shared-default");
        assert_eq!(intent["mode"], "headless");
        assert_eq!(intent["window_label"], "main");
        assert_eq!(intent["pane_nonce"], "a".repeat(64));
        assert_eq!(intent["gesture_receipt"], "b".repeat(64));
        assert!(browser_user_gesture_intent("main", "short", &"b".repeat(64)).is_err());
        let embed = browser_embed_intent(
            &"a".repeat(64),
            &"b".repeat(64),
            7,
            if cfg!(windows) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            },
        )
        .unwrap();
        assert_eq!(embed["context"], "default");
        assert_eq!(embed["protocol_version"], 2);
        assert_eq!(embed["embed_generation"], 7);
        assert!(embed.get("token").is_none());
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
        assert!(
            e1.starts_with("unsupported:"),
            "구버전 미지원은 unsupported: {e1}"
        );
        // ① exit 코드 미상이어도 stderr usage+--verify 패턴이면 unsupported
        let e1b = classify_drain_verify_failure(None, "Usage: cys drain --verify ...");
        assert!(
            e1b.starts_with("unsupported:"),
            "usage+--verify는 unsupported: {e1b}"
        );
        // ② 하드캡 백스톱(exit 3) → verify_failed
        let e2 = classify_drain_verify_failure(Some(3), "");
        assert!(
            e2.starts_with("verify_failed:"),
            "exit3 백스톱은 verify_failed: {e2}"
        );
        // ② 시그널 사망(code=None)·usage 무관 stderr → verify_failed
        let e2b = classify_drain_verify_failure(None, "thread 'main' panicked at ...");
        assert!(
            e2b.starts_with("verify_failed:"),
            "크래시는 verify_failed: {e2b}"
        );
        // 정상 exit 1(부분 실패)은 JSON 경로라 여기 안 오지만, 분류가 오면 verify_failed(안전)
        let e2c = classify_drain_verify_failure(Some(1), "");
        assert!(
            e2c.starts_with("verify_failed:"),
            "exit1 무JSON은 verify_failed: {e2c}"
        );
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
        assert!(
            !session_blocks_rotation(&bare),
            "맨 pane은 자동 교대를 막지 않는다"
        );
        assert!(
            !session_blocks_rotation(&exited_agent),
            "죽은 세션은 세지 않는다"
        );
        assert!(session_blocks_rotation(&roled), "role claim 세션은 보호");
        assert!(session_blocks_rotation(&agented), "agent 세션은 보호");
        assert!(
            !session_blocks_rotation(&empty_strings),
            "빈 문자열은 미부착으로 취급"
        );
    }

    /// ★v4 GUI 온보딩 게이트 회귀 핀(0.12.52 cys-neo 실사고) — 마커가 현재 버전과 정확히 일치할
    /// 때만 스킵. 부재(신선 머신·직전 실패)·구버전·손상 = 실행(fail-open 치유 방향). 이 판정이
    /// .pack-version 등 팩 상태를 일절 보지 않는 것이 요점 — cysd 선행이 게이트를 선점 못 한다.
    #[test]
    fn needs_gui_onboard_only_skips_on_exact_version_match() {
        assert!(
            needs_gui_onboard(None, "0.12.53"),
            "마커 부재 = 온보딩(신선·직전 실패)"
        );
        assert!(
            !needs_gui_onboard(Some("0.12.53"), "0.12.53"),
            "정확 일치 = 스킵"
        );
        assert!(
            !needs_gui_onboard(Some("0.12.53\n"), "0.12.53"),
            "개행 trim 후 일치 = 스킵"
        );
        assert!(
            needs_gui_onboard(Some("0.12.52"), "0.12.53"),
            "구버전 = 온보딩(업그레이드)"
        );
        assert!(
            needs_gui_onboard(Some("garbage"), "0.12.53"),
            "손상 = 온보딩(fail-open)"
        );
    }

    #[test]
    fn decide_pending_update_marker_or_version_change() {
        use PendingUpdatePlan::*;
        // 마커 최우선 — 스탬프·prior_state와 무관하게 Apply(구버전이 이 릴리스로 올라올 때 남긴 마커).
        assert_eq!(
            decide_pending_update(true, None, "0.12.51", false),
            Apply,
            "마커=Apply(스탬프 부재)"
        );
        assert_eq!(
            decide_pending_update(true, Some("0.12.51"), "0.12.51", false),
            Apply,
            "마커=Apply(스탬프 동일해도)"
        );
        // ★결함2: 스탬프 부재 × prior_state — 기존 설치 증거로 전환기 홈페이지 설치 vs 진짜 최초 설치를 가른다.
        assert_eq!(
            decide_pending_update(false, None, "0.12.51", true),
            Apply,
            "스탬프 부재+기존설치=전환기 홈페이지설치=Apply"
        );
        assert_eq!(
            decide_pending_update(false, None, "0.12.51", false),
            RecordStampOnly,
            "스탬프 부재+기존설치 없음=진짜 최초설치"
        );
        // 버전변경/동일은 prior_state와 무관(회귀 핀 — Some(stamp)이면 prior_state를 보지 않는다).
        assert_eq!(
            decide_pending_update(false, Some("0.12.50"), "0.12.51", false),
            Apply,
            "버전변경(홈페이지 수동설치)=Apply"
        );
        assert_eq!(
            decide_pending_update(false, Some("0.12.50"), "0.12.51", true),
            Apply,
            "버전변경=Apply(prior_state 무관)"
        );
        assert_eq!(
            decide_pending_update(false, Some("0.12.51"), "0.12.51", false),
            Skip,
            "동일 버전·마커 없음=Skip"
        );
        assert_eq!(
            decide_pending_update(false, Some("0.12.51"), "0.12.51", true),
            Skip,
            "동일 버전=Skip(prior_state 무관)"
        );
    }

    // HUD-2: open_url 화이트리스트 — https·허용 도메인만 통과, 위장 host(userinfo/서브도메인 사칭) 차단.
    #[test]
    fn open_url_whitelist_blocks_spoofed_and_nonhttps() {
        assert!(url_host_allowed("https://notebooklm.google.com/notebook/abc").is_ok());
        assert!(url_host_allowed("https://github.com/cys/repo").is_ok());
        assert!(
            url_host_allowed("https://www.cysinsight.com/").is_ok(),
            "홈페이지(본체 다운로드) 허용"
        );
        assert!(
            url_host_allowed("https://cysinsight.com/download").is_ok(),
            "홈페이지 apex 허용"
        );
        assert!(
            url_host_allowed("http://notebooklm.google.com/").is_err(),
            "http 차단"
        );
        assert!(
            url_host_allowed("https://evil.com/notebooklm.google.com").is_err(),
            "경로 사칭 차단"
        );
        assert!(
            url_host_allowed("https://notebooklm.google.com.evil.com/").is_err(),
            "서브도메인 사칭 차단"
        );
        assert!(
            url_host_allowed("https://notebooklm.google.com@evil.example.com/").is_err(),
            "userinfo 사칭 차단"
        );
        assert!(
            url_host_allowed("https://evil.com#.github.com/").is_err(),
            "fragment 사칭 차단"
        );
        assert!(
            url_host_allowed("https://evil.com?.github.com").is_err(),
            "query 사칭 차단"
        );
        assert!(
            url_host_allowed("https://evil.com?x=.github.com").is_err(),
            "query 파라미터 사칭 차단"
        );
    }

    // 사용자 확장 allowlist(순수 판정) — 정확일치·서브도메인 허용, 사칭·빈 항목 차단.
    #[test]
    fn host_allowlist_user_extension() {
        let extras = vec!["example-inst.org".to_string()];
        assert!(host_in_allowlist("example-inst.org", &extras));
        assert!(
            host_in_allowlist("docs.example-inst.org", &extras),
            "확장 도메인 서브도메인 허용"
        );
        assert!(
            !host_in_allowlist("example-inst.org.evil.com", &extras),
            "사칭 차단"
        );
        assert!(!host_in_allowlist("evil.com", &extras));
        assert!(
            !host_in_allowlist("anything.com", &vec!["".to_string()]),
            "빈 확장 항목 무시"
        );
    }

    // #3: 사이드카 stdout의 PACK_UPDATE_RESULT 토큰에서 reinject failed/deferred를 파싱해
    // update-warning 발화 판단에 쓴다. 토큰 부재(구버전·reinject 스킵)는 (0,0)으로 보수적 처리.
    #[test]
    fn parse_reinject_counts_reads_structured_token() {
        let out = "[pack-update] 팩 2.0.0 반영 완료 (3 written, 1 preserved). 노드 reinject 점검…\n\
                   [pack-update] reinject: 2 injected, 1 skipped, 3 deferred, 4 failed.\n\
                   PACK_UPDATE_RESULT pack_version=2.0.0 injected=2 skipped=1 deferred=3 failed=4\n";
        assert_eq!(
            parse_reinject_counts(out),
            (4, 3),
            "failed=4 deferred=3 파싱"
        );

        // 토큰 부재 → (0,0) 보수적(경고 미발화).
        assert_eq!(parse_reinject_counts("아무 의미 없는 출력\n"), (0, 0));
        assert_eq!(parse_reinject_counts(""), (0, 0));

        // failed=0 deferred=0 → (0,0)(완전 성공, 경고 없음).
        assert_eq!(
            parse_reinject_counts(
                "PACK_UPDATE_RESULT pack_version=2.0.0 injected=5 skipped=0 deferred=0 failed=0"
            ),
            (0, 0)
        );
        // deferred만 있는 경우(busy 노드) — 경고 발화 대상.
        assert_eq!(
            parse_reinject_counts(
                "PACK_UPDATE_RESULT pack_version=1.2.3 injected=0 skipped=0 deferred=2 failed=0"
            ),
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
                    let _ = w
                        .write_all(b"{\"ok\":true,\"result\":{\"surfaces\":[]}}\n")
                        .await;
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
            classify_bundle_dir(Path::new(
                "/Applications/cys.app.prev-210050/Contents/MacOS"
            )),
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
            classify_bundle_dir(Path::new(
                "/Volumes/cys 0.12.91/Applications/cys.app/Contents/MacOS"
            )),
            BundleKind::Canonical,
            "/Volumes 하위 Applications 는 Canonical 오판 금지",
        );
        assert_eq!(
            classify_bundle_dir(Path::new(
                "/Volumes/cys 0.12.91/Applications/cys.app/Contents/MacOS"
            )),
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
            std::path::Path::new(
                "/private/var/folders/x/AppTranslocation/Y/d/cys.app/Contents/MacOS"
            ),
            "/usr/local/bin"
        )
        .is_err());
        // backup → Err
        assert!(plan_cli_install(
            std::path::Path::new("/Applications/cys.app.bak-044/Contents/MacOS"),
            "/usr/local/bin"
        )
        .is_err());
    }

    #[test]
    fn autoregister_allowed_only_canonical() {
        // 무음 launchd 자동등록은 plan_cli_install 보다 엄격하다(의도적 divergence): Canonical 만 허용.
        // NonStandard(~/Downloads·/Volumes 등)도 거부 — 휘발/이동 경로가 plist 에 각인되면 언마운트·삭제
        // 시 죽은 경로 데몬 무한 스폰(리뷰어1 F1). 비-Canonical 은 ensure_daemon 런타임 폴백으로 안전.
        assert!(
            autoregister_allowed(&BundleKind::Canonical),
            "정규 번들(/Applications·~/Applications)만 자동등록 허용"
        );
        assert!(
            !autoregister_allowed(&BundleKind::Translocated),
            "임시 경로는 자동등록 거부"
        );
        assert!(
            !autoregister_allowed(&BundleKind::Backup),
            "백업 번들은 자동등록 거부"
        );
        assert!(
            !autoregister_allowed(&BundleKind::NonStandard),
            "비표준(Downloads·USB 등)도 자동등록 거부"
        );
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
            boot_path_verdict(Path::new("/Users/x/dev/cys/target/release/cys-app"), true,),
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
                Path::new(
                    "/private/var/folders/ab/AppTranslocation/x/cys.app/Contents/MacOS/cys-app"
                ),
                true,
            ),
            BootPathVerdict::Canonical,
        );
        // escape 없는 개발 target/ 는 NonCanonical(하네스가 env 로 스스로 풀어야 함).
        assert_eq!(
            boot_path_verdict(Path::new("/Users/x/dev/cys/target/debug/cys-app"), false,),
            BootPathVerdict::NonCanonical,
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn translocation_guidance_carries_recovery_steps() {
        // 안내는 ①Applications 드래그 ②구버전 종료·교체 ③xattr quarantine 제거를 모두 담아야 한다.
        let g = translocation_guidance(BootPathVerdict::Translocated);
        assert!(
            g.contains("Applications"),
            "① Applications 드래그 설치 안내 포함"
        );
        assert!(
            g.contains("구버전") && g.contains("종료"),
            "② 구버전 종료·교체 안내 포함"
        );
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
        assert!(
            g.is_some(),
            "비정규 실행(test 하네스 경로)에서 pull 은 안내 문구를 반환"
        );
        assert!(
            g.unwrap()
                .contains("xattr -d com.apple.quarantine /Applications/cys.app"),
            "pull 이 반환한 안내에 복구 명령 포함(프론트 stickyToast 본문)",
        );
        std::env::set_var("CYS_ALLOW_NONCANONICAL", "1");
        assert!(
            boot_verdict().is_none(),
            "escape env 에서는 None(정상 부트 — 안내 미표시)"
        );
        std::env::remove_var("CYS_ALLOW_NONCANONICAL");
    }

    #[test]
    fn plan_cli_install_warns_on_nonstandard_but_proceeds() {
        let plan = plan_cli_install(
            std::path::Path::new("/Users/x/Downloads/cys.app/Contents/MacOS"),
            "/usr/local/bin",
        )
        .expect("nonstandard는 경고와 함께 진행");
        assert!(plan.osascript_arg.contains("with administrator privileges"));
        assert!(plan.warnings.iter().any(|w| w.contains("표준 위치")));
        assert_eq!(
            plan.cys_src,
            std::path::PathBuf::from("/Users/x/Downloads/cys.app/Contents/MacOS/cys")
        );
    }

    #[test]
    fn plan_cli_install_canonical_has_no_location_warning() {
        let plan = plan_cli_install(
            std::path::Path::new("/Applications/cys.app/Contents/MacOS"),
            "/usr/local/bin",
        )
        .expect("정규 번들은 진행");
        assert!(plan.warnings.iter().all(|w| !w.contains("표준 위치")));
        // osascript 인자는 do shell script + 승격 + 멱등 스크립트를 감싼다(AppleScript 큰따옴표 리터럴)
        assert!(plan.osascript_arg.starts_with("do shell script \""));
        assert!(plan
            .osascript_arg
            .ends_with("\" with administrator privileges"));
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
        assert!(plan
            .osascript_arg
            .ends_with("\" with administrator privileges"));
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
        let seg = &src[start
            ..start
                + src[start..]
                    .find("\n#[tauri::command]")
                    .unwrap_or(src.len() - start)];
        assert!(
            !seg.contains("--purge-workdir"),
            "GUI purge 가 --purge-workdir 를 다시 요청함 — 홈 파괴 경로 재개방(실사고 2026-07-16 재발)"
        );
        assert!(
            seg.contains("--purge-state"),
            "purge 명령 골격 변형 — 트립와이어 재검토 필요"
        );
    }

    /// [C6] 데몬 재시작으로 브로커 등록 대장이 소실되면 broker가 GUI에 내려보내는 거부 메시지의
    /// 판별 부분열(BROKER_REGISTRATION_LOST_MARKER)이 실제 브로커 소스 문자열에 포함되는지 확인한다.
    /// rpc()는 error.message만 GUI로 올리므로(main.rs rpc_on), 이 부분열 매칭이 어긋나면
    /// ensure_browserd_cast의 자가 회복(1회 재등록·재시도)이 발동하지 못한다(v0.13.10 실사고 재발).
    /// include_str!는 컴파일 시점 임베드라 런타임 경로 의존이 없다(purge 트립와이어와 동일 관례).
    #[test]
    fn broker_registration_lost_marker_matches_broker_source() {
        let broker_src = include_str!("../../src/bin/cysd/authority_broker/mod.rs");
        assert!(
            broker_src.contains("GUI peer has no broker-owned registration"),
            "브로커 GUI_NOT_REGISTERED 메시지 원문 변경 — 판별 마커 재배선 필요"
        );
        assert!(
            "GUI peer has no broker-owned registration".contains(BROKER_REGISTRATION_LOST_MARKER),
            "판별 마커가 브로커 메시지의 부분열이 아님 — 자가 회복 미발동 위험"
        );
    }

    /// [WS-1] GUI가 **분기**하는 브로커 code 상수가 실제 브로커 소스에 실재하는지 확인한다.
    /// 브로커가 code를 개명·삭제하면 자가 회복(재등록·재시도)이 조용히 죽는다 — marker 폴백이
    /// 남아 있어 무증상으로 넘어가므로 컴파일 시점에 잡아야 한다.
    /// 음성 대조군(존재하지 않는 code)을 함께 둬 이 단언이 공허 참이 아님을 증명한다.
    #[test]
    fn gui_branch_codes_exist_in_broker_source() {
        let broker_src = include_str!("../../src/bin/cysd/authority_broker/mod.rs");
        for code in [GUI_NOT_REGISTERED_CODE, GUI_IDENTITY_MISMATCH_CODE] {
            assert!(
                broker_src.contains(&format!("\"{code}\"")),
                "브로커 소스에 {code} 가 없음 — GUI 자가 회복 분기가 죽는다(코드 개명·삭제 의심)"
            );
        }
        // 음성 대조군: 실재하지 않는 code는 잡히면 안 된다(부분열 오탐 방지 포함).
        for absent in ["\"GUI_NOT_REGISTERED_V2\"", "\"GUI_IDENTITY_MISMATCHED\""] {
            assert!(
                !broker_src.contains(absent),
                "음성 대조군 {absent} 이 브로커 소스에 존재 — 트립와이어 판정이 무의미해짐"
            );
        }
    }

    /// [WS-1] 배너 대괄호 규약(`BROWSER_DISABLED_SAFE [<CODE>]: <message>`)이 UI 픽스처와
    /// 동일한 형식인지 컴파일 시점에 대조한다. UI가 새 형식을 만들거나 Rust가 규약을 바꾸면
    /// castdiag/webpane의 진단 보강이 원문 통과로 무력화된다.
    #[test]
    fn banner_bracket_convention_matches_ui_fixture() {
        let castdiag_src = include_str!("../../ui/src/castdiag.test.ts");
        assert!(
            castdiag_src.contains("BROWSER_DISABLED_SAFE [NATIVE_ACTIVATION_REQUIRED]"),
            "UI 픽스처의 대괄호 배너 규약 소실 — Rust 배너 형식과 어긋남"
        );
        let failure = RpcFailure {
            code: GUI_IDENTITY_MISMATCH_CODE.to_string(),
            message: "GUI PID incarnation changed".to_string(),
        };
        assert_eq!(
            failure.banner(),
            "BROWSER_DISABLED_SAFE [GUI_IDENTITY_MISMATCH]: GUI PID incarnation changed"
        );
        // 음성 대조군: 종전의 code 없는 형식(`BROWSER_DISABLED_SAFE: `)으로 회귀하지 않는다.
        assert!(!failure.banner().starts_with("BROWSER_DISABLED_SAFE: "));
    }

    /// [WS-1] 데몬 응답 → {code,message} 분해. code 부재(구 데몬 스큐)에도 배너 형식이 깨지지
    /// 않아야 하고, ok 응답의 result 계약은 기존 rpc_on/rpc_oneshot과 동일해야 한다.
    #[test]
    fn unwrap_rpc_result_preserves_code_and_message() {
        let ok = unwrap_rpc_result(json!({"ok":true,"result":{"port":9222}}));
        assert_eq!(ok.unwrap()["port"], json!(9222));

        let coded = unwrap_rpc_result(
            json!({"ok":false,"error":{"code":"GUI_REGISTRATION_LIMIT","message":"too many GUI peers"}}),
        )
        .unwrap_err();
        assert_eq!(coded.code, "GUI_REGISTRATION_LIMIT");
        assert_eq!(coded.message, "too many GUI peers");

        // code 부재·빈 문자열은 자리표로 채운다(배너 형식 유지). message 폴백은 기존 계약 유지.
        let uncoded = unwrap_rpc_result(json!({"ok":false,"error":{"message":"boom"}})).unwrap_err();
        assert_eq!(uncoded.code, RPC_UNCODED_ERROR_CODE);
        assert_eq!(uncoded.message, "boom");
        let empty = unwrap_rpc_result(json!({"ok":false,"error":{"code":""}})).unwrap_err();
        assert_eq!(empty.code, RPC_UNCODED_ERROR_CODE);
        assert_eq!(empty.message, "unknown error");
    }

    /// [WS-1] 자가 회복 판정 — code 우선 + marker 문자열 폴백. `GUI_IDENTITY_MISMATCH`가
    /// 종전 marker 매칭에 걸리지 않아 클릭 1회를 잃던 결함의 회귀 핀이다.
    #[test]
    fn gui_registration_lost_matches_code_first_then_marker() {
        let not_registered = RpcFailure {
            code: GUI_NOT_REGISTERED_CODE.to_string(),
            message: "GUI peer has no broker-owned registration".into(),
        };
        assert!(not_registered.is_gui_registration_lost());

        // 종전 결함: identity mismatch 는 message 에 marker 가 없어 회복 분기를 못 탔다.
        let mismatch = RpcFailure {
            code: GUI_IDENTITY_MISMATCH_CODE.to_string(),
            message: "GUI PID incarnation, canonical executable, code signature, or digest changed"
                .into(),
        };
        assert!(!mismatch.message.contains(BROKER_REGISTRATION_LOST_MARKER));
        assert!(mismatch.is_gui_registration_lost());

        // 구 데몬 스큐(code 미탑재) — marker 폴백이 살아 있어야 한다.
        let legacy = RpcFailure {
            code: RPC_UNCODED_ERROR_CODE.to_string(),
            message: "GUI peer has no broker-owned registration".into(),
        };
        assert!(legacy.is_gui_registration_lost());

        // 무관 실패는 재등록 루프를 돌리지 않는다(오발동 금지).
        let unrelated = RpcFailure {
            code: "RUNTIME_START_FAILED".into(),
            message: "supervisor exited before ready".into(),
        };
        assert!(!unrelated.is_gui_registration_lost());
    }

    /// 만료 challenge 자가회복 구멍의 회귀 핀. 브로커가 만료를 `GUI_IDENTITY_MISMATCH`에서
    /// `GUI_REGISTRATION_REPLAY`로 재분류하면서 GUI 회복 분기가 끊겼다 — 만료는 "느렸다"이지
    /// "틀렸다"가 아니라 재발급·재등록이 정답인데 그 경로를 못 탔다.
    #[test]
    fn expired_challenge_is_recoverable_but_replay_is_not() {
        let expired = RpcFailure {
            code: GUI_REGISTRATION_REPLAY_CODE.to_string(),
            message:
                "GUI registration challenge is unknown or already consumed (사유: 만료)".into(),
        };
        assert!(expired.is_challenge_expired(), "만료 식별");
        assert!(
            expired.is_gui_registration_lost(),
            "만료는 재시도로 회복 가능 — 회복 대상에 포함돼야 한다"
        );

        // 미발급·기소비는 같은 code지만 회복 불가(재시도해도 같은 실패) — 오발동 금지.
        let consumed = RpcFailure {
            code: GUI_REGISTRATION_REPLAY_CODE.to_string(),
            message: "GUI registration challenge is unknown or already consumed (사유: 미발급 또는 기소비)"
                .into(),
        };
        assert!(!consumed.is_challenge_expired());
        assert!(!consumed.is_gui_registration_lost());

        // 브로커가 전용 code를 신설하면(정공법) 문자열 없이도 곧바로 잡혀야 한다.
        let coded = RpcFailure {
            code: GUI_CHALLENGE_EXPIRED_CODE.to_string(),
            message: "whatever the broker writes".into(),
        };
        assert!(coded.is_challenge_expired());
        assert!(coded.is_gui_registration_lost());
    }

    /// 임시 조치(message 부분열 식별)의 유일한 안전장치 — 브로커가 만료 문구를 손대면 이 분기는
    /// 무증상으로 죽는다. 컴파일 시점 임베드로 그 순간을 red로 만든다.
    /// (브로커가 `GUI_CHALLENGE_EXPIRED` 전용 code를 신설하면 이 테스트와 마커를 함께 폐기한다.)
    #[test]
    fn expiry_marker_exists_in_broker_source() {
        let broker_src = include_str!("../../src/bin/cysd/authority_broker/mod.rs");
        assert!(
            broker_src.contains(BROKER_CHALLENGE_EXPIRED_MARKER),
            "브로커 소스에 만료 마커 {BROKER_CHALLENGE_EXPIRED_MARKER} 가 없다 — \
             GUI 만료 자가회복 분기가 죽었다(브로커측 전용 code 신설로 이관했다면 이 테스트도 폐기)"
        );
        assert!(
            broker_src.contains(&format!("\"{GUI_REGISTRATION_REPLAY_CODE}\"")),
            "브로커 소스에 {GUI_REGISTRATION_REPLAY_CODE} 가 없다 — code 개명·삭제 의심"
        );
    }

    /// ★4-1 회귀 핀: **재등록이 살아있는 세션을 무효화하지 않는다**.
    /// `GUI_ALREADY_REGISTERED` 폐기(WS-4)로 두 번째 등록이 거부가 아니라 교체가 됐다 — 부트
    /// 재시도와 클릭 경로가 겹치면 S1이 S2로 갈아치워져 in-flight RPC가 GUI_IDENTITY_MISMATCH를
    /// 맞는다. `ensure_browser_gui_peer`는 살아있는 세션을 보면 **RPC 자체를 하지 않는다**.
    /// (가드를 지우면: 데몬 부재 시 Err로, 데몬 존재 시 세션 교체로 — 어느 쪽이든 red가 된다.)
    /// 전역 `BROWSER_APP_SESSION`을 건드리는 테스트끼리의 직렬화 가드 — 병렬 실행에서 서로의
    /// 세션 상태를 덮어써 무증상 flake가 되는 것을 막는다(테스트 전용·프로덕션 무관).
    static SESSION_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn ensure_does_not_replace_a_live_session() {
        let _guard = SESSION_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let live = "a".repeat(64);
        *BROWSER_APP_SESSION.write().unwrap() = Some(live.clone());
        let result = tauri::async_runtime::block_on(ensure_browser_gui_peer());
        let after = BROWSER_APP_SESSION.read().unwrap().clone();
        *BROWSER_APP_SESSION.write().unwrap() = None; // 전역 상태 원복(다른 테스트 오염 금지)
        assert!(result.is_ok(), "살아있는 세션이면 등록을 시도조차 하지 않는다: {result:?}");
        assert_eq!(after.as_deref(), Some(live.as_str()), "세션이 교체되면 안 된다");
    }

    /// ★ADV-2 회귀 핀: passive 조회의 **조기 return 2곳**도 사인을 싣는다. 특히 세션 부재
    /// (= GUI peer 미등록)는 부팅 등록 실패·데몬 교체 직후의 최빈 고장인데, 종전에는 bare
    /// `{alive:false}`라 `castStateReason`이 최빈 경로에서 아무 것도 표시하지 못했다.
    #[test]
    fn browserd_state_early_returns_carry_a_diagnostic_sign() {
        let _guard = SESSION_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let nonce = "a".repeat(64);
        let ticket = "b".repeat(64);
        let origin = if cfg!(windows) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };

        // ① embed intent 자체가 불성립(부모 origin 불일치) — 로컬 거부 사인.
        let bad = tauri::async_runtime::block_on(browserd_state(
            nonce.clone(),
            1,
            "https://evil.example".into(),
            ticket.clone(),
        ));
        assert_eq!(bad["alive"], json!(false), "alive 계약은 불변이어야 한다");
        assert_eq!(bad["code"], json!(EMBED_INTENT_INVALID_CODE));
        assert!(
            bad["message"].as_str().is_some_and(|m| !m.is_empty()),
            "사유 문구가 실려야 한다: {bad}"
        );

        // ② 세션 부재(=GUI peer 미등록) — 최빈 실제 고장. RPC를 보내기 전에 끊긴다.
        *BROWSER_APP_SESSION.write().unwrap() = None;
        let unregistered =
            tauri::async_runtime::block_on(browserd_state(nonce, 1, origin.into(), ticket));
        assert_eq!(unregistered["alive"], json!(false));
        assert_eq!(
            unregistered["code"],
            json!(GUI_NOT_REGISTERED_CODE),
            "미등록은 브로커 동명 사인으로 올라와야 클릭 재등록 처방과 수렴한다: {unregistered}"
        );
        assert!(unregistered["message"]
            .as_str()
            .is_some_and(|m| m.contains("registration")));
    }

    /// ★F-3 회귀 핀: 등록 RPC는 **연결은 됐지만 응답하지 않는** cysd 앞에서 상한 내에 실패하고,
    /// `GUI_REGISTRATION_FLIGHT` 뮤텍스를 즉시 놓아줘 다음 시도(=신뢰 클릭 경로)가 진행된다.
    /// 상한이 없으면 `rpc_once`의 `read_line`이 영원히 블록해 뮤텍스가 영구 점유되고, 이 브랜치가
    /// 도입한 single-flight 직렬화 때문에 클릭 경로까지 통째로 봉쇄된다(회귀 조건).
    #[cfg(unix)]
    #[test]
    fn gui_registration_times_out_on_hung_socket_and_releases_the_flight() {
        use tokio::net::UnixListener;
        let _guard = SESSION_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("cys-gui-reg-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let hang_sock = dir.join("hang.sock");
        let _ = std::fs::remove_file(&hang_sock);
        *BROWSER_APP_SESSION.write().unwrap() = None; // 세션 없음 = 실제 등록을 시도하는 상태

        let deadline = std::time::Duration::from_millis(300);
        tauri::async_runtime::block_on(async {
            // 무응답 소켓: accept만 하고 응답을 주지 않는다(= hang한 cysd).
            let hang = UnixListener::bind(&hang_sock).unwrap();
            tauri::async_runtime::spawn(async move {
                let mut held = Vec::new();
                while let Ok((s, _)) = hang.accept().await {
                    held.push(s); // 연결 유지 — EOF를 주면 timeout이 아니라 즉시 실패가 된다
                }
            });
            tokio::time::sleep(std::time::Duration::from_millis(50)).await; // bind 안정화

            for attempt in 1..=2 {
                let started = std::time::Instant::now();
                let result = ensure_browser_gui_peer_on(&hang_sock, deadline).await;
                let elapsed = started.elapsed();
                let failure = result.expect_err("무응답 데몬에서 등록이 성공할 수 없다");
                assert_eq!(
                    failure.code, RPC_TRANSPORT_FAILED_CODE,
                    "상한 초과는 전송 계층 실패로 올라온다 (attempt {attempt}): {failure:?}"
                );
                assert!(
                    failure.message.contains("did not answer"),
                    "상한 초과 사유가 message에 남아야 한다 (attempt {attempt}): {failure:?}"
                );
                assert!(
                    elapsed < deadline * 10,
                    "등록은 상한 안에서 끝나야 한다 (attempt {attempt}): {elapsed:?}"
                );
                // 뮤텍스 해제 확인 — 잠긴 채로 남으면 다음 시도(클릭 경로)가 영영 못 들어온다.
                assert!(
                    gui_registration_flight().try_lock().is_ok(),
                    "실패 후 single-flight 뮤텍스가 해제돼야 한다 (attempt {attempt})"
                );
            }
        });
        assert!(
            BROWSER_APP_SESSION.read().unwrap().is_none(),
            "실패한 등록이 세션을 남기면 안 된다"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// [WS-1] 배너 선두 사인은 UI 절단(castFailureReason maxLen=200) 안에 반드시 남아야 한다.
    /// 브로커 최장 code + 전형적 message 로 상한을 잰다.
    #[test]
    fn banner_code_survives_ui_truncation_budget() {
        let failure = RpcFailure {
            code: "GUI_IDENTITY_MISMATCH".into(),
            message: "GUI PID incarnation, canonical executable, code signature, or digest changed"
                .into(),
        };
        let banner = failure.banner();
        // 사인은 앞에서 46자 안에 끝난다("BROWSER_DISABLED_SAFE [" 23 + code 21 + "]" 1 + ":" 1).
        let sign_end = banner.find(']').expect("대괄호 규약 이탈");
        assert!(sign_end < 120, "사인이 기본 절단(120) 밖으로 밀림: {sign_end}");
        assert!(banner.len() <= 200, "배너 전문이 cast 절단(200)을 초과: {}", banner.len());
    }
}
