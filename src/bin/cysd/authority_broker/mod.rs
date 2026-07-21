//! BrowserAuthorityExtension: optional, lazy and isolated from CorePTY boot.

use cys::browser_runtime::{
    evaluate_compatibility, Compatibility, CompatibilityRequirement, EngineKey, ParsedRuntimeState,
    ProtocolRequirement, RuntimeManifest, RuntimeStateV2, TargetTriple,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

mod runtime_launcher;

static SHARED_BROKER: OnceLock<
    Result<AuthorityBroker<runtime_launcher::RealSupervisorLauncher>, BrokerFailure>,
> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct BrokerConfig {
    manifest_path: PathBuf,
    runtime_root: PathBuf,
    state_path: PathBuf,
    target: TargetTriple,
}

impl BrokerConfig {
    pub fn bundled() -> Result<Self, BrokerFailure> {
        let executable = std::env::current_exe().map_err(|error| {
            BrokerFailure::new(
                "RUNTIME_NOT_FOUND",
                format!("cysd path unavailable: {error}"),
            )
        })?;
        let executable_dir = executable.parent().ok_or_else(|| {
            BrokerFailure::new("RUNTIME_NOT_FOUND", "cysd has no containing directory")
        })?;
        #[cfg(target_os = "macos")]
        let resource_root = executable_dir.join("../Resources/browser-runtime");
        #[cfg(windows)]
        let resource_root = executable_dir.join("browser-runtime");
        #[cfg(not(any(target_os = "macos", windows)))]
        let resource_root = executable_dir.join("browser-runtime");
        let state_root = cys::home_dir().join(".cys/browser/instances/v2/shared-default-headless");
        Ok(Self {
            manifest_path: resource_root.join("browser-runtime.lock.json"),
            runtime_root: resource_root.join("runtime"),
            state_path: state_root.join("state.json"),
            target: TargetTriple::current()
                .map_err(|error| BrokerFailure::new(error.code().as_str(), error.to_string()))?,
        })
    }

    #[cfg(test)]
    fn for_test(manifest_path: PathBuf, runtime_root: PathBuf, state_path: PathBuf) -> Self {
        Self {
            manifest_path,
            runtime_root,
            state_path,
            target: TargetTriple::current().unwrap_or(TargetTriple::Aarch64AppleDarwin),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LaunchRequest {
    pub manifest: RuntimeManifest,
    pub manifest_path: PathBuf,
    pub target: TargetTriple,
    pub runtime_root: PathBuf,
    pub state_path: PathBuf,
    pub authority: VerifiedAuthority,
    pub cancellation: Arc<AtomicBool>,
}

thread_local! {
    static BROWSER_REQUEST_CANCELLATION: std::cell::RefCell<Option<Arc<AtomicBool>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn with_browser_cancellation<T>(
    cancellation: Arc<AtomicBool>,
    task: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            BROWSER_REQUEST_CANCELLATION.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
    BROWSER_REQUEST_CANCELLATION.with(|slot| {
        *slot.borrow_mut() = Some(cancellation);
    });
    let _reset = Reset;
    task()
}

fn current_browser_cancellation() -> Arc<AtomicBool> {
    BROWSER_REQUEST_CANCELLATION
        .with(|slot| slot.borrow().clone())
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrokerFailure {
    pub code: String,
    pub message: String,
}

impl BrokerFailure {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

pub trait SupervisorLauncher: Send + Sync + 'static {
    fn launch(&self, request: &LaunchRequest) -> Result<RuntimeStateV2, BrokerFailure>;

    /// Return the exact immutable endpoint bytes authenticated for this live
    /// process incarnation. Broker callers must use this snapshot directly and
    /// must never re-read mutable disk state after validation.
    fn validate_live(
        &self,
        state: &RuntimeStateV2,
    ) -> Result<AuthenticatedEndpointSnapshot, BrokerFailure>;

    fn sign_private_request(
        &self,
        state: &RuntimeStateV2,
        body: &[u8],
    ) -> Result<String, BrokerFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorityKind {
    UserGesture,
    AuthorizedWorkerOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedAuthority {
    pub kind: AuthorityKind,
    pub caller_pid: u32,
    pub subject: String,
    pub receipt_id: String,
    pub normalized_request_hash: String,
    pub subject_hash: String,
}

impl VerifiedAuthority {
    #[cfg(test)]
    fn trusted_test() -> Self {
        Self {
            kind: AuthorityKind::UserGesture,
            caller_pid: std::process::id(),
            subject: "test-user-gesture".into(),
            receipt_id: "test-receipt".into(),
            normalized_request_hash: format!("sha256:{}", "8".repeat(64)),
            subject_hash: format!("sha256:{}", "9".repeat(64)),
        }
    }

    fn permits_shared_start(&self) -> bool {
        self.caller_pid != 0
            && !self.subject.is_empty()
            && !self.receipt_id.is_empty()
            && self.normalized_request_hash.starts_with("sha256:")
            && self.subject_hash.starts_with("sha256:")
            && matches!(
                self.kind,
                AuthorityKind::UserGesture | AuthorityKind::AuthorizedWorkerOperation
            )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BrokerStatus {
    NotRunning,
    Compatible { state: RuntimeStateV2 },
    Incompatible { reason: String },
    LegacyActive,
    DisabledSafe { code: String, message: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EmbedDescriptor {
    pub schema_version: u16,
    pub instance_id: String,
    pub engine_generation: u64,
    pub embed_generation: u64,
    pub parent_origin: String,
    pub pane_id: String,
    pub url: String,
    pub headless: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthenticatedEndpointSnapshot {
    pub(super) schema_version: u16,
    pub(super) pid: u32,
    pub(super) port: u16,
    pub(super) token: String,
    pub(super) cast_token: String,
    pub(super) runtime_id: String,
    pub(super) process_start_time: u64,
    pub(super) headless: bool,
    pub(super) instance_id: String,
    pub(super) engine_generation: u64,
    pub(super) state_mac: String,
}

pub struct AuthorityBroker<L: SupervisorLauncher> {
    config: BrokerConfig,
    launcher: Arc<L>,
    inner: Mutex<BrokerInner>,
}

#[derive(Default)]
struct BrokerInner {
    live: Option<RuntimeStateV2>,
    gesture_receipts: HashMap<String, GestureReceipt>,
    gui_peers: GuiPeerRegistry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GuiExecutableIdentity {
    pid: u32,
    start_time: u64,
    canonical_executable: String,
    code_sign_identity: String,
    executable_sha256: String,
}

#[derive(Default)]
struct GuiPeerRegistry {
    sessions: HashMap<u32, RegisteredGuiPeer>,
    used_registration_secrets: HashSet<String>,
    pending_challenges: HashMap<String, PendingGuiChallenge>,
}

struct RegisteredGuiPeer {
    session: String,
    identity: GuiExecutableIdentity,
}

struct PendingGuiChallenge {
    identity: GuiExecutableIdentity,
    expires_at_ms: u64,
}

const MAX_GUI_PEERS: usize = 4;
const GUI_CHALLENGE_TTL_MS: u64 = 5_000;

impl GuiPeerRegistry {
    fn issue_challenge(
        &mut self,
        identity: GuiExecutableIdentity,
        now_ms: u64,
    ) -> Result<String, BrokerFailure> {
        validate_gui_identity_shape(&identity)?;
        self.pending_challenges
            .retain(|_, challenge| challenge.expires_at_ms > now_ms);
        if self.pending_challenges.len() >= MAX_GUI_PEERS {
            return Err(BrokerFailure::new(
                "GUI_REGISTRATION_LIMIT",
                "GUI registration challenge registry is at its hard limit",
            ));
        }
        let challenge = crate::channels::random_token_hex()
            .map_err(|error| BrokerFailure::new("AUTHORITY_UNAVAILABLE", error))?;
        self.pending_challenges.insert(
            sha256_id(challenge.as_bytes()),
            PendingGuiChallenge {
                identity,
                expires_at_ms: now_ms.saturating_add(GUI_CHALLENGE_TTL_MS),
            },
        );
        Ok(challenge)
    }

    fn register(
        &mut self,
        one_time_secret: &str,
        identity: GuiExecutableIdentity,
        now_ms: u64,
    ) -> Result<String, BrokerFailure> {
        validate_hex_secret(one_time_secret, "GUI registration secret")?;
        validate_gui_identity_shape(&identity)?;
        let secret_hash = sha256_id(one_time_secret.as_bytes());
        if self.used_registration_secrets.contains(&secret_hash) {
            return Err(BrokerFailure::new(
                "GUI_REGISTRATION_REPLAY",
                "GUI one-time registration secret was already consumed",
            ));
        }
        let Some(challenge) = self.pending_challenges.remove(&secret_hash) else {
            return Err(BrokerFailure::new(
                "GUI_REGISTRATION_REPLAY",
                "GUI registration challenge is unknown or already consumed",
            ));
        };
        if challenge.expires_at_ms <= now_ms || challenge.identity != identity {
            return Err(BrokerFailure::new(
                "GUI_IDENTITY_MISMATCH",
                "GUI registration challenge is expired or bound to another process identity",
            ));
        }
        if self.sessions.contains_key(&identity.pid) {
            return Err(BrokerFailure::new(
                "GUI_ALREADY_REGISTERED",
                "GUI process incarnation is already registered",
            ));
        }
        if self.sessions.len() >= MAX_GUI_PEERS {
            return Err(BrokerFailure::new(
                "GUI_REGISTRATION_LIMIT",
                "GUI registration registry is at its hard limit",
            ));
        }
        let session = crate::channels::random_token_hex()
            .map_err(|error| BrokerFailure::new("AUTHORITY_UNAVAILABLE", error))?;
        self.used_registration_secrets.insert(secret_hash);
        self.sessions.insert(
            identity.pid,
            RegisteredGuiPeer {
                session: session.clone(),
                identity,
            },
        );
        Ok(session)
    }

    fn validate(
        &mut self,
        caller_pid: u32,
        session: &str,
        current: &GuiExecutableIdentity,
    ) -> Result<(), BrokerFailure> {
        validate_hex_secret(session, "GUI session")?;
        let Some(registered) = self.sessions.get(&caller_pid) else {
            return Err(BrokerFailure::new(
                "GUI_NOT_REGISTERED",
                "GUI peer has no broker-owned registration",
            ));
        };
        if registered.session != session || registered.identity != *current {
            self.sessions.remove(&caller_pid);
            return Err(BrokerFailure::new(
                "GUI_IDENTITY_MISMATCH",
                "GUI PID incarnation, canonical executable, code signature, or digest changed",
            ));
        }
        Ok(())
    }
}

fn validate_hex_secret(value: &str, label: &str) -> Result<(), BrokerFailure> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(BrokerFailure::new(
            "AUTHORITY_REJECTED",
            format!("{label} must be 32-byte lowercase hex"),
        ));
    }
    Ok(())
}

fn validate_gui_identity_shape(identity: &GuiExecutableIdentity) -> Result<(), BrokerFailure> {
    if identity.pid == 0
        || identity.start_time == 0
        || !std::path::Path::new(&identity.canonical_executable).is_absolute()
        || identity.code_sign_identity.is_empty()
        || !identity.executable_sha256.starts_with("sha256:")
        || identity.executable_sha256.len() != 71
    {
        return Err(BrokerFailure::new(
            "GUI_IDENTITY_REJECTED",
            "GUI executable identity is incomplete",
        ));
    }
    Ok(())
}

struct GestureReceipt {
    caller_pid: u32,
    window_label: String,
    pane_nonce: String,
    expires_at_ms: u64,
}

const USER_GESTURE_TTL_MS: u64 = 5_000;

impl<L: SupervisorLauncher> AuthorityBroker<L> {
    pub fn new(config: BrokerConfig, launcher: Arc<L>) -> Self {
        Self {
            config,
            launcher,
            inner: Mutex::new(BrokerInner::default()),
        }
    }

    fn register_gui_peer(
        &self,
        one_time_secret: &str,
        identity: GuiExecutableIdentity,
    ) -> Result<String, BrokerFailure> {
        self.inner
            .lock()
            .unwrap()
            .gui_peers
            .register(one_time_secret, identity, unix_millis())
    }

    fn issue_gui_challenge(
        &self,
        identity: GuiExecutableIdentity,
    ) -> Result<String, BrokerFailure> {
        self.inner
            .lock()
            .unwrap()
            .gui_peers
            .issue_challenge(identity, unix_millis())
    }

    fn validate_gui_peer(
        &self,
        caller_pid: u32,
        session: &str,
        current: &GuiExecutableIdentity,
    ) -> Result<(), BrokerFailure> {
        self.inner
            .lock()
            .unwrap()
            .gui_peers
            .validate(caller_pid, session, current)
    }

    fn issue_user_gesture_at(
        &self,
        caller_pid: u32,
        window_label: &str,
        pane_nonce: &str,
        now_ms: u64,
    ) -> Result<String, BrokerFailure> {
        validate_window_and_pane(window_label, pane_nonce)?;
        if caller_pid == 0 {
            return Err(BrokerFailure::new(
                "AUTHORITY_REJECTED",
                "trusted app process identity is unavailable",
            ));
        }
        let receipt = crate::channels::random_token_hex()
            .map_err(|error| BrokerFailure::new("AUTHORITY_UNAVAILABLE", error))?;
        let mut inner = self.inner.lock().unwrap();
        inner
            .gesture_receipts
            .retain(|_, value| value.expires_at_ms > now_ms);
        inner.gesture_receipts.insert(
            receipt.clone(),
            GestureReceipt {
                caller_pid,
                window_label: window_label.into(),
                pane_nonce: pane_nonce.into(),
                expires_at_ms: now_ms.saturating_add(USER_GESTURE_TTL_MS),
            },
        );
        Ok(receipt)
    }

    fn issue_user_gesture(
        &self,
        caller_pid: u32,
        window_label: &str,
        pane_nonce: &str,
    ) -> Result<String, BrokerFailure> {
        self.issue_user_gesture_at(caller_pid, window_label, pane_nonce, unix_millis())
    }

    fn consume_user_gesture_at(
        &self,
        caller_pid: u32,
        window_label: &str,
        pane_nonce: &str,
        receipt: &str,
        now_ms: u64,
    ) -> Result<String, BrokerFailure> {
        validate_window_and_pane(window_label, pane_nonce)?;
        if receipt.len() != 64 || !receipt.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(BrokerFailure::new(
                "AUTHORITY_REJECTED",
                "gesture receipt is malformed",
            ));
        }
        let mut inner = self.inner.lock().unwrap();
        let Some(expected) = inner.gesture_receipts.get(receipt) else {
            return Err(BrokerFailure::new(
                "AUTHORITY_REJECTED",
                "gesture receipt is missing or already consumed",
            ));
        };
        if expected.expires_at_ms <= now_ms {
            inner.gesture_receipts.remove(receipt);
            return Err(BrokerFailure::new(
                "AUTHORITY_REJECTED",
                "gesture receipt expired",
            ));
        }
        if expected.caller_pid != caller_pid
            || expected.window_label != window_label
            || expected.pane_nonce != pane_nonce
        {
            return Err(BrokerFailure::new(
                "AUTHORITY_REJECTED",
                "gesture receipt is not bound to this app window and pane",
            ));
        }
        inner.gesture_receipts.remove(receipt);
        Ok(format!(
            "PanePrepare:{window_label}:{pane_nonce}:{receipt}:shared-default/headless"
        ))
    }

    fn consume_user_gesture(
        &self,
        caller_pid: u32,
        params: &Value,
    ) -> Result<String, BrokerFailure> {
        let window = required_param(params, "window_label")?;
        let pane = required_hex_credential(params, "pane_nonce")?;
        let receipt = required_hex_credential(params, "gesture_receipt")?;
        self.consume_user_gesture_at(caller_pid, &window, &pane, &receipt, unix_millis())
    }

    /// Passive inspection only. It never creates directories, repairs files or
    /// launches a process. Browser corruption is data, not a cysd boot error.
    pub fn probe(&self) -> BrokerStatus {
        let mut inner = self.inner.lock().unwrap();
        if let Some(state) = inner.live.clone() {
            if self.launcher.validate_live(&state).is_ok() {
                return self.status_for_v2(state);
            }
            inner.live = None;
        }
        drop(inner);
        self.probe_disk()
    }

    fn probe_disk(&self) -> BrokerStatus {
        let input = match std::fs::read_to_string(&self.config.state_path) {
            Ok(input) => input,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return BrokerStatus::NotRunning;
            }
            Err(error) => {
                return BrokerStatus::DisabledSafe {
                    code: "CORRUPT_STATE".into(),
                    message: format!("Browser state unavailable: {error}"),
                };
            }
        };
        let parsed = match cys::browser_runtime::parse_runtime_state(&input) {
            Ok(parsed) => parsed,
            Err(error) => {
                return BrokerStatus::DisabledSafe {
                    code: error.code().as_str().into(),
                    message: error.to_string(),
                };
            }
        };
        let ParsedRuntimeState::V2(state) = parsed else {
            return BrokerStatus::LegacyActive;
        };
        let _ = state;
        BrokerStatus::DisabledSafe {
            code: "UNAUTHENTICATED_STATE".into(),
            message: "disk state has no broker-owned authenticated supervisor session".into(),
        }
    }

    pub fn ensure_shared(
        &self,
        authority: &VerifiedAuthority,
    ) -> Result<BrokerStatus, BrokerFailure> {
        if !authority.permits_shared_start() {
            return Err(BrokerFailure::new(
                "AUTHORITY_REJECTED",
                "authority does not permit shared-default Browser startup",
            ));
        }
        // This lock spans compatibility re-check and launch. Concurrent callers
        // wait for the first result and can never create a second supervisor.
        let mut inner = self.inner.lock().unwrap();
        if let Some(state) = inner.live.clone() {
            if self.launcher.validate_live(&state).is_ok() {
                let status = self.status_for_v2(state);
                if matches!(status, BrokerStatus::Compatible { .. }) {
                    return Ok(status);
                }
            }
            inner.live = None;
        }
        match self.probe_disk() {
            status @ BrokerStatus::Compatible { .. } => return Ok(status),
            BrokerStatus::NotRunning => {}
            BrokerStatus::LegacyActive => {
                return Err(BrokerFailure::new(
                    "LEGACY_ACTIVE",
                    "legacy Browser runtime is active; automatic kill is forbidden",
                ));
            }
            BrokerStatus::Incompatible { reason } => {
                return Err(BrokerFailure::new(
                    "PROTOCOL_MISMATCH",
                    format!("active Browser runtime is incompatible: {reason}"),
                ));
            }
            BrokerStatus::DisabledSafe { code, .. } if code == "UNAUTHENTICATED_STATE" => {}
            BrokerStatus::DisabledSafe { code, message } => {
                return Err(BrokerFailure::new(code, message));
            }
        }
        let manifest = self.load_manifest()?;
        if !manifest.release_ready {
            return Err(BrokerFailure::new(
                "RUNTIME_DISABLED",
                "bundled Browser Runtime is not release-qualified",
            ));
        }
        manifest
            .target(&self.config.target)
            .map_err(|error| BrokerFailure::new(error.code().as_str(), error.to_string()))?;
        let state = self.launcher.launch(&LaunchRequest {
            manifest,
            manifest_path: self.config.manifest_path.clone(),
            target: self.config.target.clone(),
            runtime_root: self.config.runtime_root.clone(),
            state_path: self.config.state_path.clone(),
            authority: authority.clone(),
            cancellation: current_browser_cancellation(),
        })?;
        let status = self.status_for_v2(state.clone());
        if !matches!(status, BrokerStatus::Compatible { .. }) {
            return Err(BrokerFailure::new(
                "RUNTIME_START_FAILED",
                format!("supervisor returned an incompatible live state: {status:?}"),
            ));
        }
        inner.live = Some(state);
        Ok(status)
    }

    /// Build a short-lived iframe descriptor from the broker-owned private
    /// engine endpoint. The control token is never a DTO field and callers
    /// cannot select a process, port, executable or runtime identity.
    pub fn prepare_embed(&self, params: &Value) -> Result<EmbedDescriptor, BrokerFailure> {
        let BrokerStatus::Compatible { state } = self.probe() else {
            return Err(BrokerFailure::new(
                "BROWSER_NOT_RUNNING",
                "a compatible Browser Runtime is not running",
            ));
        };
        let context = params.get("context").and_then(Value::as_str);
        let protocol_version = params.get("protocol_version").and_then(Value::as_u64);
        let embed_generation = params.get("embed_generation").and_then(Value::as_u64);
        let parent_origin = params.get("parent_origin").and_then(Value::as_str);
        if context != Some("default")
            || protocol_version != Some(2)
            || !embed_generation.is_some_and(|generation| generation > 0)
            || parent_origin != Some(packaged_parent_origin())
        {
            return Err(BrokerFailure::new(
                "EMBED_DESCRIPTOR_REJECTED",
                "embed context, protocol, generation or parent origin is invalid",
            ));
        }
        let embed_ticket = required_hex_credential(params, "embed_ticket")?;
        let pane_id = required_hex_credential(params, "pane_id")?;
        let endpoint = self.validated_engine_endpoint(&state)?;
        let embed_generation = embed_generation.unwrap();
        self.register_embed_ticket(
            &state,
            &endpoint,
            &json!({
                "ticket":embed_ticket,
                "runtimeId":state.runtime_id,
                "context":"default",
                "protocolVersion":2,
                "embedGeneration":embed_generation,
                "paneId":pane_id,
                "parentOrigin":packaged_parent_origin(),
            }),
        )?;
        let origin_query = match packaged_parent_origin() {
            "tauri://localhost" => "tauri%3A%2F%2Flocalhost",
            "http://tauri.localhost" => "http%3A%2F%2Ftauri.localhost",
            _ => unreachable!("packaged parent origin is fixed"),
        };
        let url = format!(
            "http://127.0.0.1:{}/{}/cast/?context=default&protocolVersion=2&embedGeneration={}&parentOrigin={}&embedTicket={}&paneId={}",
            endpoint.port,
            embed_ticket,
            embed_generation,
            origin_query,
            embed_ticket,
            pane_id,
        );
        Ok(EmbedDescriptor {
            schema_version: 1,
            instance_id: state.instance_id,
            engine_generation: state.engine_generation,
            embed_generation,
            parent_origin: packaged_parent_origin().into(),
            pane_id,
            url,
            headless: true,
        })
    }

    fn register_embed_ticket(
        &self,
        state: &RuntimeStateV2,
        endpoint: &AuthenticatedEndpointSnapshot,
        descriptor: &Value,
    ) -> Result<(), BrokerFailure> {
        use std::io::{Read, Write};
        use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
        use std::time::Duration;
        let body = serde_json::to_vec(descriptor)
            .map_err(|error| BrokerFailure::new("EMBED_DESCRIPTOR_REJECTED", error.to_string()))?;
        let request_mac = self.launcher.sign_private_request(state, &body)?;
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, endpoint.port);
        let mut stream = TcpStream::connect_timeout(&address.into(), Duration::from_secs(2))
            .map_err(|error| {
                BrokerFailure::new(
                    "ENGINE_ENDPOINT_UNAVAILABLE",
                    format!("embed registration connect failed: {error}"),
                )
            })?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
        write!(
            stream,
            "POST /{}/embed-ticket HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nX-CYS-Engine-Auth: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            endpoint.token,
            endpoint.port,
            request_mac,
            body.len()
        )
        .and_then(|_| stream.write_all(&body))
        .map_err(|error| {
            BrokerFailure::new(
                "ENGINE_RPC_FAILED",
                format!("embed registration failed: {error}"),
            )
        })?;
        let mut response = Vec::new();
        stream
            .take(64 * 1024)
            .read_to_end(&mut response)
            .map_err(|error| {
                BrokerFailure::new(
                    "ENGINE_RPC_FAILED",
                    format!("embed registration response failed: {error}"),
                )
            })?;
        if !response.starts_with(b"HTTP/1.1 200 ") && !response.starts_with(b"HTTP/1.0 200 ") {
            return Err(BrokerFailure::new(
                "EMBED_DESCRIPTOR_REJECTED",
                "engine rejected the one-time embed registration",
            ));
        }
        Ok(())
    }

    /// Proxy an already-authorized worker operation without disclosing the
    /// private port/control token to Python or the cys CLI adapter.
    pub fn proxy_operation(&self, params: &Value) -> Result<Value, BrokerFailure> {
        use std::io::{Read, Write};
        use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
        use std::time::Duration;

        let BrokerStatus::Compatible { state } = self.probe() else {
            return Err(BrokerFailure::new(
                "BROWSER_NOT_RUNNING",
                "a compatible Browser Runtime is not running",
            ));
        };
        let verb = params
            .get("verb")
            .and_then(Value::as_str)
            .filter(|verb| !verb.is_empty() && verb.len() <= 64)
            .ok_or_else(|| BrokerFailure::new("BAD_ARGS", "operation verb is required"))?;
        let args = params
            .get("args")
            .filter(|args| args.is_object())
            .ok_or_else(|| BrokerFailure::new("BAD_ARGS", "operation args must be an object"))?;
        let body = serde_json::to_vec(&json!({"verb":verb,"args":args})).map_err(|error| {
            BrokerFailure::new("BAD_ARGS", format!("operation encode failed: {error}"))
        })?;
        if body.len() > 1024 * 1024 {
            return Err(BrokerFailure::new(
                "BAD_ARGS",
                "operation request exceeds 1 MiB",
            ));
        }
        let endpoint = self.validated_engine_endpoint(&state)?;
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, endpoint.port);
        let mut stream = TcpStream::connect_timeout(&address.into(), Duration::from_secs(2))
            .map_err(|error| {
                BrokerFailure::new(
                    "ENGINE_ENDPOINT_UNAVAILABLE",
                    format!("private engine connection failed: {error}"),
                )
            })?;
        stream.set_read_timeout(Some(Duration::from_secs(65))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        let request = format!(
            "POST /{}/rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            endpoint.token,
            endpoint.port,
            body.len(),
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(&body))
            .map_err(|error| {
                BrokerFailure::new(
                    "ENGINE_RPC_FAILED",
                    format!("engine request failed: {error}"),
                )
            })?;
        let mut response = Vec::new();
        stream
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut response)
            .map_err(|error| {
                BrokerFailure::new(
                    "ENGINE_RPC_FAILED",
                    format!("engine response failed: {error}"),
                )
            })?;
        if response.len() > 16 * 1024 * 1024 {
            return Err(BrokerFailure::new(
                "ENGINE_RPC_FAILED",
                "engine response exceeds 16 MiB",
            ));
        }
        let split = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or_else(|| BrokerFailure::new("ENGINE_RPC_FAILED", "invalid HTTP response"))?;
        let head = std::str::from_utf8(&response[..split])
            .map_err(|_| BrokerFailure::new("ENGINE_RPC_FAILED", "invalid HTTP response head"))?;
        if !(head.starts_with("HTTP/1.1 200 ") || head.starts_with("HTTP/1.0 200 ")) {
            return Err(BrokerFailure::new(
                "ENGINE_RPC_FAILED",
                "engine returned a non-200 response",
            ));
        }
        serde_json::from_slice(&response[split + 4..]).map_err(|error| {
            BrokerFailure::new(
                "ENGINE_RPC_FAILED",
                format!("invalid engine JSON response: {error}"),
            )
        })
    }

    fn validated_engine_endpoint(
        &self,
        state: &RuntimeStateV2,
    ) -> Result<AuthenticatedEndpointSnapshot, BrokerFailure> {
        let endpoint = self.launcher.validate_live(state)?;
        if endpoint.schema_version != 2
            || endpoint.pid != state.engine_pid
            || endpoint.port != state.port
            || endpoint.runtime_id != state.runtime_id
            || endpoint.process_start_time == 0
            || !endpoint.headless
            || endpoint.instance_id != state.instance_id
            || endpoint.engine_generation != state.engine_generation
            || endpoint.state_mac.len() != 64
            || !endpoint
                .state_mac
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || endpoint.token.len() != 32
            || !endpoint
                .token
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || endpoint.cast_token.len() != 32
            || !endpoint
                .cast_token
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || endpoint.cast_token == endpoint.token
        {
            return Err(BrokerFailure::new(
                "RUNTIME_INTEGRITY_FAILED",
                "private engine endpoint does not match the selected runtime",
            ));
        }
        Ok(endpoint)
    }

    fn load_manifest(&self) -> Result<RuntimeManifest, BrokerFailure> {
        let input = std::fs::read_to_string(&self.config.manifest_path).map_err(|error| {
            BrokerFailure::new(
                "RUNTIME_NOT_FOUND",
                format!("Browser Runtime manifest unavailable: {error}"),
            )
        })?;
        RuntimeManifest::parse(&input)
            .map_err(|error| BrokerFailure::new(error.code().as_str(), error.to_string()))
    }

    fn status_for_v2(&self, state: RuntimeStateV2) -> BrokerStatus {
        let manifest_input = match std::fs::read_to_string(&self.config.manifest_path) {
            Ok(input) => input,
            Err(error) => {
                return BrokerStatus::DisabledSafe {
                    code: "RUNTIME_NOT_FOUND".into(),
                    message: format!("Browser Runtime manifest unavailable: {error}"),
                };
            }
        };
        let manifest = match RuntimeManifest::parse(&manifest_input) {
            Ok(manifest) => manifest,
            Err(error) => {
                return BrokerStatus::DisabledSafe {
                    code: error.code().as_str().into(),
                    message: error.to_string(),
                };
            }
        };
        if manifest.target(&self.config.target).is_err() {
            return BrokerStatus::DisabledSafe {
                code: "RUNTIME_NOT_FOUND".into(),
                message: format!("target {} is not bundled", self.config.target.as_str()),
            };
        }
        let requirement = CompatibilityRequirement::new(
            manifest.runtime_id.clone(),
            ProtocolRequirement::new(
                manifest.browser_protocol.major,
                manifest.browser_protocol.min_minor,
                manifest.browser_protocol.max_minor,
                manifest
                    .browser_protocol
                    .required_capabilities
                    .iter()
                    .cloned(),
            ),
            EngineKey::shared_default(),
        );
        match evaluate_compatibility(&ParsedRuntimeState::V2(state.clone()), &requirement) {
            Compatibility::Compatible => BrokerStatus::Compatible { state },
            Compatibility::Incompatible(issues) => BrokerStatus::Incompatible {
                reason: format!("{issues:?}"),
            },
        }
    }
}

/// cysd dispatch adapter. It is the only public entry to the Browser extension;
/// callers submit logical intent and never executable or state paths.
pub fn handle(
    daemon: &crate::state::Daemon,
    operation: &str,
    params: &Value,
    caller_pid: Option<u32>,
) -> Result<Value, BrokerFailure> {
    let broker = shared_broker()?;
    match operation {
        "probe" => serde_json::to_value(broker.probe()).map_err(|error| {
            BrokerFailure::new(
                "BROWSER_UNAVAILABLE",
                format!("status encode failed: {error}"),
            )
        }),
        "gui_challenge" => {
            let caller_pid = caller_pid.filter(|pid| *pid != 0).ok_or_else(|| {
                BrokerFailure::new("AUTHORITY_REJECTED", "peer process is unavailable")
            })?;
            if crate::handlers::resolve_caller_surface(daemon, caller_pid).is_some() {
                return Err(BrokerFailure::new(
                    "GUI_IDENTITY_REJECTED",
                    "a terminal surface process cannot register as the GUI shell",
                ));
            }
            let identity = PlatformGuiPeerVerifier.verify(caller_pid)?;
            let challenge = broker.issue_gui_challenge(identity)?;
            Ok(
                json!({"schema_version":1,"registration_challenge":challenge,"expires_in_ms":GUI_CHALLENGE_TTL_MS}),
            )
        }
        "register_gui" => {
            let caller_pid = caller_pid.filter(|pid| *pid != 0).ok_or_else(|| {
                BrokerFailure::new("AUTHORITY_REJECTED", "peer process is unavailable")
            })?;
            if crate::handlers::resolve_caller_surface(daemon, caller_pid).is_some() {
                return Err(BrokerFailure::new(
                    "GUI_IDENTITY_REJECTED",
                    "a terminal surface process cannot register as the GUI shell",
                ));
            }
            let one_time_secret = required_hex_credential(params, "registration_secret")?;
            let identity = PlatformGuiPeerVerifier.verify(caller_pid)?;
            let session = broker.register_gui_peer(&one_time_secret, identity)?;
            Ok(json!({"schema_version":1,"app_session":session}))
        }
        "issue_gesture" => {
            let caller_pid = require_registered_gui_caller(broker, daemon, params, caller_pid)?;
            let window = required_param(params, "window_label")?;
            let pane = required_hex_credential(params, "pane_nonce")?;
            let receipt = broker.issue_user_gesture(caller_pid, &window, &pane)?;
            Ok(json!({
                "schema_version":1,
                "gesture_receipt":receipt,
                "expires_in_ms":USER_GESTURE_TTL_MS,
                "window_label":window,
                "pane_nonce":pane,
            }))
        }
        "ensure" => {
            validate_shared_headless_scope(params)?;
            let authority = verified_authority(broker, daemon, params, caller_pid)?;
            serde_json::to_value(broker.ensure_shared(&authority)?).map_err(|error| {
                BrokerFailure::new(
                    "BROWSER_UNAVAILABLE",
                    format!("status encode failed: {error}"),
                )
            })
        }
        "operation" => {
            validate_shared_headless_scope(params)?;
            let authority = verified_authority(broker, daemon, params, caller_pid)?;
            if !matches!(authority.kind, AuthorityKind::AuthorizedWorkerOperation) {
                return Err(BrokerFailure::new(
                    "AUTHORITY_REJECTED",
                    "Browser operations require a peer-verified worker surface",
                ));
            }
            match broker.ensure_shared(&authority)? {
                BrokerStatus::Compatible { .. } => broker.proxy_operation(params),
                other => Err(BrokerFailure::new(
                    "BROWSER_UNAVAILABLE",
                    format!("Browser Runtime is not compatible: {other:?}"),
                )),
            }
        }
        "prepare_embed" => {
            require_registered_gui_caller(broker, daemon, params, caller_pid)?;
            serde_json::to_value(broker.prepare_embed(params)?).map_err(|error| {
                BrokerFailure::new(
                    "BROWSER_UNAVAILABLE",
                    format!("embed descriptor encode failed: {error}"),
                )
            })
        }
        _ => Err(BrokerFailure::new(
            "METHOD_NOT_FOUND",
            format!("unknown Browser Runtime operation {operation}"),
        )),
    }
}

fn packaged_parent_origin() -> &'static str {
    if cfg!(windows) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    }
}

fn require_registered_gui_caller<L: SupervisorLauncher>(
    broker: &AuthorityBroker<L>,
    daemon: &crate::state::Daemon,
    params: &Value,
    caller_pid: Option<u32>,
) -> Result<u32, BrokerFailure> {
    let caller_pid = caller_pid
        .filter(|pid| *pid != 0)
        .ok_or_else(|| BrokerFailure::new("AUTHORITY_REJECTED", "peer process is unavailable"))?;
    if crate::handlers::resolve_caller_surface(daemon, caller_pid).is_some() {
        return Err(BrokerFailure::new(
            "AUTHORITY_REJECTED",
            "Browser GUI RPCs are unavailable to terminal surface processes",
        ));
    }
    let session = required_hex_credential(params, "app_session")?;
    let current = PlatformGuiPeerVerifier.verify(caller_pid)?;
    broker.validate_gui_peer(caller_pid, &session, &current)?;
    Ok(caller_pid)
}

fn validate_shared_headless_scope(params: &Value) -> Result<(), BrokerFailure> {
    if params.get("realm").and_then(Value::as_str) != Some("shared-default")
        || params.get("mode").and_then(Value::as_str) != Some("headless")
    {
        return Err(BrokerFailure::new(
            "AUTHORITY_REJECTED",
            "shared Browser startup is fixed to shared-default/headless",
        ));
    }
    Ok(())
}

fn shared_broker(
) -> Result<&'static AuthorityBroker<runtime_launcher::RealSupervisorLauncher>, BrokerFailure> {
    match SHARED_BROKER.get_or_init(|| {
        BrokerConfig::bundled().map(|config| {
            AuthorityBroker::new(
                config,
                Arc::new(runtime_launcher::RealSupervisorLauncher::default()),
            )
        })
    }) {
        Ok(broker) => Ok(broker),
        Err(error) => Err(error.clone()),
    }
}

fn verified_authority<L: SupervisorLauncher>(
    broker: &AuthorityBroker<L>,
    daemon: &crate::state::Daemon,
    params: &Value,
    caller_pid: Option<u32>,
) -> Result<VerifiedAuthority, BrokerFailure> {
    let caller_pid = caller_pid
        .filter(|pid| *pid != 0)
        .ok_or_else(|| BrokerFailure::new("AUTHORITY_REJECTED", "peer process is unavailable"))?;
    let requested = params
        .get("authority_kind")
        .and_then(Value::as_str)
        .ok_or_else(|| BrokerFailure::new("AUTHORITY_REJECTED", "authority_kind is required"))?;
    let caller_surface = crate::handlers::resolve_caller_surface(daemon, caller_pid);
    let role = caller_surface.and_then(|surface_id| {
        daemon
            .get_surface(surface_id)
            .and_then(|surface| surface.role.lock().unwrap().clone())
    });
    let (kind, subject) = match requested {
        "user_gesture" if caller_surface.is_none() => {
            require_registered_gui_caller(broker, daemon, params, Some(caller_pid))?;
            (
                AuthorityKind::UserGesture,
                broker.consume_user_gesture(caller_pid, params)?,
            )
        }
        "authorized_worker_operation"
            if role.as_deref().is_some_and(|role| {
                role == "worker" || role.starts_with("worker-") || role.starts_with("worker_")
            }) =>
        {
            let task = required_param(params, "task_id")?;
            (
                AuthorityKind::AuthorizedWorkerOperation,
                format!("OperationStart:{task}:shared-default/headless"),
            )
        }
        "direct_user_cli" => {
            return Err(BrokerFailure::new(
                "DIRECT_USER_CONFIRMATION_REQUIRED",
                "TTY or a roleless cys process is not sufficient DirectUserCli authority",
            ));
        }
        "reviewer_verification_job" => {
            return Err(BrokerFailure::new(
                "AUTHORITY_REJECTED",
                "reviewer verification cannot start shared-default Browser Runtime",
            ));
        }
        _ => {
            return Err(BrokerFailure::new(
                "AUTHORITY_REJECTED",
                format!(
                    "caller role/process does not match requested Browser authority (role={})",
                    role.as_deref().unwrap_or("unregistered")
                ),
            ));
        }
    };
    let normalized = json!({
        "method":"browser.runtime.ensure",
        "authority_kind":requested,
        "caller_pid":caller_pid,
        "subject":subject,
    });
    let request_hash = sha256_id(&serde_json::to_vec(&normalized).unwrap_or_default());
    let subject_hash = sha256_id(subject.as_bytes());
    let receipt_id = crate::channels::random_token_hex()
        .map_err(|error| BrokerFailure::new("AUTHORITY_UNAVAILABLE", error))?;
    Ok(VerifiedAuthority {
        kind,
        caller_pid,
        subject,
        receipt_id,
        normalized_request_hash: request_hash,
        subject_hash,
    })
}

fn validate_window_and_pane(window: &str, pane: &str) -> Result<(), BrokerFailure> {
    if window.is_empty()
        || window.len() > 64
        || !window
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || pane.len() != 64
        || !pane.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(BrokerFailure::new(
            "AUTHORITY_REJECTED",
            "window label or pane identity is invalid",
        ));
    }
    Ok(())
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn required_hex_credential(params: &Value, key: &str) -> Result<String, BrokerFailure> {
    let value = required_param(params, key)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(BrokerFailure::new(
            "AUTHORITY_REJECTED",
            format!("{key} must be 32-byte lowercase hex"),
        ));
    }
    Ok(value)
}

fn required_param(params: &Value, key: &str) -> Result<String, BrokerFailure> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_owned)
        .ok_or_else(|| BrokerFailure::new("AUTHORITY_REJECTED", format!("{key} is required")))
}

fn sha256_id(input: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(input))
}

trait GuiPeerVerifier {
    fn verify(&self, pid: u32) -> Result<GuiExecutableIdentity, BrokerFailure>;
}

struct PlatformGuiPeerVerifier;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct WindowsSignerIdentity {
    leaf_thumbprint: String,
    spki_sha256: String,
    subject: String,
}

fn same_windows_signer(expected: &WindowsSignerIdentity, actual: &WindowsSignerIdentity) -> bool {
    !expected.leaf_thumbprint.is_empty()
        && expected
            .leaf_thumbprint
            .eq_ignore_ascii_case(&actual.leaf_thumbprint)
        && expected.spki_sha256.len() == 64
        && expected
            .spki_sha256
            .eq_ignore_ascii_case(&actual.spki_sha256)
        && !expected.subject.is_empty()
        && expected.subject == actual.subject
}

fn verify_windows_release_gui_hash(expected: &str, actual: &str) -> Result<String, BrokerFailure> {
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(BrokerFailure::new(
            "GUI_IDENTITY_REJECTED",
            "compiled Windows release GUI hash is missing or invalid",
        ));
    }
    if actual != format!("sha256:{expected}") {
        return Err(BrokerFailure::new(
            "GUI_IDENTITY_REJECTED",
            "unsigned Windows GUI does not match the release-pinned executable hash",
        ));
    }
    Ok(format!("windows-release-sha256:{expected}"))
}

impl GuiPeerVerifier for PlatformGuiPeerVerifier {
    fn verify(&self, pid: u32) -> Result<GuiExecutableIdentity, BrokerFailure> {
        inspect_gui_executable_identity(pid)
    }
}

fn inspect_gui_executable_identity(pid: u32) -> Result<GuiExecutableIdentity, BrokerFailure> {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    let process = system.process(Pid::from_u32(pid)).ok_or_else(|| {
        BrokerFailure::new("GUI_IDENTITY_REJECTED", "GUI peer process is unavailable")
    })?;
    let executable = process.exe().ok_or_else(|| {
        BrokerFailure::new(
            "GUI_IDENTITY_REJECTED",
            "GUI executable path is unavailable",
        )
    })?;
    let canonical = executable.canonicalize().map_err(|error| {
        BrokerFailure::new(
            "GUI_IDENTITY_REJECTED",
            format!("GUI executable path is not canonical: {error}"),
        )
    })?;
    let start_time = process.start_time();
    if start_time == 0 {
        return Err(BrokerFailure::new(
            "GUI_IDENTITY_REJECTED",
            "GUI process start time is unavailable",
        ));
    }
    let bytes = std::fs::read(&canonical).map_err(|error| {
        BrokerFailure::new(
            "GUI_IDENTITY_REJECTED",
            format!("GUI executable cannot be hashed: {error}"),
        )
    })?;
    let executable_sha256 = sha256_id(&bytes);
    let code_sign_identity = verify_gui_code_identity(&canonical, &executable_sha256)?;
    Ok(GuiExecutableIdentity {
        pid,
        start_time,
        canonical_executable: canonical.to_string_lossy().into_owned(),
        code_sign_identity,
        executable_sha256,
    })
}

fn explicit_gui_dev_mode() -> bool {
    cfg!(debug_assertions) && std::env::var("CYS_BROWSER_DEV").as_deref() == Ok("1")
}

fn verify_gui_code_identity(
    path: &std::path::Path,
    executable_sha256: &str,
) -> Result<String, BrokerFailure> {
    #[cfg(not(windows))]
    let _ = executable_sha256;
    if explicit_gui_dev_mode() {
        let path_text = path.to_string_lossy().replace('\\', "/");
        let local_target = path_text.ends_with("/target/debug/cys-app")
            || path_text.ends_with("/target/release/cys-app")
            || path_text.ends_with("/target/debug/cys-app.exe")
            || path_text.ends_with("/target/release/cys-app.exe");
        if !local_target {
            return Err(BrokerFailure::new(
                "GUI_IDENTITY_REJECTED",
                "explicit Browser dev mode accepts only a canonical local Cargo target cys-app",
            ));
        }
        return Ok("development:explicit-opt-in".into());
    }
    #[cfg(target_os = "macos")]
    {
        let expected = std::path::Path::new("/Applications/cys.app/Contents/MacOS/cys-app");
        if path != expected {
            return Err(BrokerFailure::new(
                "GUI_IDENTITY_REJECTED",
                "production GUI must be the canonical /Applications cys executable",
            ));
        }
        let requirement = "=designated => anchor apple generic and identifier \"com.cysjavis.terminal\" and certificate leaf[subject.OU] = \"Q43YA2NMF9\"";
        let status = std::process::Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict", "--verbose=2"])
            .arg(format!("-R{requirement}"))
            .arg(path)
            .status()
            .map_err(|error| {
                BrokerFailure::new(
                    "GUI_IDENTITY_REJECTED",
                    format!("codesign verification unavailable: {error}"),
                )
            })?;
        if !status.success() {
            return Err(BrokerFailure::new(
                "GUI_IDENTITY_REJECTED",
                "GUI designated code requirement did not verify",
            ));
        }
        return Ok("macos:com.cysjavis.terminal:Q43YA2NMF9".into());
    }
    #[cfg(windows)]
    {
        let expected = std::env::current_exe()
            .ok()
            .and_then(|current| current.parent().map(|parent| parent.join("cys-app.exe")))
            .and_then(|candidate| candidate.canonicalize().ok())
            .ok_or_else(|| {
                BrokerFailure::new(
                    "GUI_IDENTITY_REJECTED",
                    "canonical packaged GUI path unavailable",
                )
            })?;
        if path != expected {
            return Err(BrokerFailure::new(
                "GUI_IDENTITY_REJECTED",
                "production GUI is not the canonical packaged cys-app.exe",
            ));
        }
        if let Some(expected_hash) = option_env!("CYS_WINDOWS_GUI_SHA256") {
            return verify_windows_release_gui_hash(expected_hash, executable_sha256);
        }
        let cysd_path = std::env::current_exe()
            .map_err(|error| {
                BrokerFailure::new(
                    "GUI_IDENTITY_REJECTED",
                    format!("cysd executable unavailable: {error}"),
                )
            })?
            .canonicalize()
            .map_err(|error| {
                BrokerFailure::new(
                    "GUI_IDENTITY_REJECTED",
                    format!("canonical cysd path unavailable: {error}"),
                )
            })?;
        let expected_signer = windows_authenticode_identity(&cysd_path)?;
        let app_signer = windows_authenticode_identity(path)?;
        if !same_windows_signer(&expected_signer, &app_signer) {
            return Err(BrokerFailure::new(
                "GUI_IDENTITY_REJECTED",
                "GUI Authenticode signer does not match the signed cysd distribution identity",
            ));
        }
        return Ok(format!(
            "windows-authenticode:{}:{}",
            app_signer.leaf_thumbprint, app_signer.spki_sha256
        ));
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    Err(BrokerFailure::new(
        "GUI_IDENTITY_REJECTED",
        "production GUI identity verification is unsupported on this platform",
    ))
}

#[cfg(windows)]
fn windows_authenticode_identity(
    path: &std::path::Path,
) -> Result<WindowsSignerIdentity, BrokerFailure> {
    // Get-AuthenticodeSignature delegates trust evaluation to the Windows
    // Authenticode/WinVerifyTrust stack. `Valid` alone is insufficient: the
    // leaf certificate and DER SubjectPublicKeyInfo digest must match signed cysd.
    // The small DER encoder works on inbox Windows PowerShell 5 as well as pwsh;
    // relying on ExportSubjectPublicKeyInfo would fail closed on .NET Framework.
    let script = r#"
function Der-Length([int]$n) {
  if ($n -lt 128) { return [byte[]]@([byte]$n) }
  $b = New-Object 'System.Collections.Generic.List[byte]'
  while ($n -gt 0) { $b.Insert(0, [byte]($n -band 255)); $n = $n -shr 8 }
  return [byte[]](@([byte](0x80 -bor $b.Count)) + @($b.ToArray()))
}
function Der-Wrap([byte]$tag, [byte[]]$body) {
  return [byte[]](@($tag) + @(Der-Length $body.Length) + @($body))
}
function Der-Base128([UInt64]$n) {
  $b = New-Object 'System.Collections.Generic.List[byte]'
  $b.Insert(0, [byte]($n -band 127)); $n = $n -shr 7
  while ($n -gt 0) { $b.Insert(0, [byte](0x80 -bor ($n -band 127))); $n = $n -shr 7 }
  return [byte[]]$b.ToArray()
}
function Der-Oid([string]$oid) {
  [UInt64[]]$p = $oid.Split('.')
  if ($p.Length -lt 2 -or $p[0] -gt 2 -or ($p[0] -lt 2 -and $p[1] -gt 39)) { throw 'invalid OID' }
  $body = New-Object 'System.Collections.Generic.List[byte]'
  $body.AddRange([byte[]](Der-Base128 ([UInt64]($p[0] * 40 + $p[1]))))
  for ($i=2; $i -lt $p.Length; $i++) { $body.AddRange([byte[]](Der-Base128 $p[$i])) }
  return Der-Wrap 0x06 ([byte[]]$body.ToArray())
}
$s = Get-AuthenticodeSignature -LiteralPath $args[0]
if ($s.Status -ne 'Valid' -or $null -eq $s.SignerCertificate) { exit 9 }
$cert = $s.SignerCertificate
$oid = Der-Oid $cert.PublicKey.Oid.Value
$parameters = [byte[]]$cert.PublicKey.EncodedParameters.RawData
$algorithm = Der-Wrap 0x30 ([byte[]](@($oid) + @($parameters)))
$keyBits = Der-Wrap 0x03 ([byte[]](@(0) + @($cert.PublicKey.EncodedKeyValue.RawData)))
$spki = Der-Wrap 0x30 ([byte[]](@($algorithm) + @($keyBits)))
$sha = [System.Security.Cryptography.SHA256]::Create()
$spkiHash = ([BitConverter]::ToString($sha.ComputeHash($spki))).Replace('-','').ToLowerInvariant()
[PSCustomObject]@{leaf_thumbprint=$cert.Thumbprint;spki_sha256=$spkiHash;subject=$cert.Subject} | ConvertTo-Json -Compress
"#;
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .arg(path)
        .output()
        .map_err(|error| {
            BrokerFailure::new(
                "GUI_IDENTITY_REJECTED",
                format!("Authenticode verification unavailable: {error}"),
            )
        })?;
    if !output.status.success() || output.stdout.len() > 16 * 1024 {
        return Err(BrokerFailure::new(
            "GUI_IDENTITY_REJECTED",
            "WinVerifyTrust rejected the executable signature",
        ));
    }
    let identity: WindowsSignerIdentity =
        serde_json::from_slice(&output.stdout).map_err(|error| {
            BrokerFailure::new(
                "GUI_IDENTITY_REJECTED",
                format!("invalid Authenticode signer identity: {error}"),
            )
        })?;
    if identity.leaf_thumbprint.len() != 40
        || !identity
            .leaf_thumbprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || identity.spki_sha256.len() != 64
        || !identity
            .spki_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || identity.subject.is_empty()
    {
        return Err(BrokerFailure::new(
            "GUI_IDENTITY_REJECTED",
            "Authenticode signer identity is incomplete",
        ));
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Default)]
    struct CountingLauncher(AtomicUsize);

    impl SupervisorLauncher for CountingLauncher {
        fn launch(
            &self,
            request: &LaunchRequest,
        ) -> Result<cys::browser_runtime::RuntimeStateV2, BrokerFailure> {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(30));
            Ok(cys::browser_runtime::RuntimeStateV2 {
                schema_version: 2,
                instance_id: "00112233445566778899aabbccddeeff".into(),
                engine_generation: 1,
                supervisor_pid: 101,
                engine_pid: 102,
                port: 53111,
                supervisor_build_id: request.manifest.supervisor.build_id.clone(),
                engine_build_id: request.manifest.engine.build_id.clone(),
                runtime_id: request.manifest.runtime_id.clone(),
                attestation_id: format!("sha256:{}", "b".repeat(64)),
                policy_epoch: 1,
                policy_hash: format!("sha256:{}", "c".repeat(64)),
                protocol: request.manifest.browser_protocol.clone(),
                chromium_revision: request.manifest.chromium.revision.clone(),
                engine_key: cys::browser_runtime::EngineKey::shared_default(),
                profile_epoch: request.manifest.chromium.profile_schema_epoch,
                started_at: "2026-07-21T00:00:00Z".into(),
            })
        }

        fn sign_private_request(
            &self,
            _state: &RuntimeStateV2,
            _body: &[u8],
        ) -> Result<String, BrokerFailure> {
            Ok("0".repeat(64))
        }

        fn validate_live(
            &self,
            state: &RuntimeStateV2,
        ) -> Result<AuthenticatedEndpointSnapshot, BrokerFailure> {
            Ok(test_endpoint(state))
        }
    }

    struct CrashableLauncher {
        launches: AtomicUsize,
        live: std::sync::atomic::AtomicBool,
    }

    impl Default for CrashableLauncher {
        fn default() -> Self {
            Self {
                launches: AtomicUsize::new(0),
                live: std::sync::atomic::AtomicBool::new(true),
            }
        }
    }

    impl SupervisorLauncher for CrashableLauncher {
        fn launch(&self, request: &LaunchRequest) -> Result<RuntimeStateV2, BrokerFailure> {
            self.live.store(true, Ordering::SeqCst);
            let generation = self.launches.fetch_add(1, Ordering::SeqCst) as u64 + 1;
            let mut state = CountingLauncher::default().launch(request)?;
            state.engine_generation = generation;
            state.instance_id = format!("{generation:032x}");
            Ok(state)
        }

        fn validate_live(
            &self,
            state: &RuntimeStateV2,
        ) -> Result<AuthenticatedEndpointSnapshot, BrokerFailure> {
            if self.live.load(Ordering::SeqCst) {
                Ok(test_endpoint(state))
            } else {
                Err(BrokerFailure::new("ENGINE_EXITED", "managed engine exited"))
            }
        }

        fn sign_private_request(
            &self,
            _state: &RuntimeStateV2,
            _body: &[u8],
        ) -> Result<String, BrokerFailure> {
            Ok("0".repeat(64))
        }
    }

    fn test_endpoint(state: &RuntimeStateV2) -> AuthenticatedEndpointSnapshot {
        AuthenticatedEndpointSnapshot {
            schema_version: 2,
            pid: state.engine_pid,
            port: state.port,
            token: "0123456789abcdef0123456789abcdef".into(),
            cast_token: "fedcba9876543210fedcba9876543210".into(),
            runtime_id: state.runtime_id.clone(),
            process_start_time: 123,
            headless: true,
            instance_id: state.instance_id.clone(),
            engine_generation: state.engine_generation,
            state_mac: "e".repeat(64),
        }
    }

    #[derive(Default)]
    struct MismatchedEndpointLauncher(CountingLauncher);

    impl SupervisorLauncher for MismatchedEndpointLauncher {
        fn launch(&self, request: &LaunchRequest) -> Result<RuntimeStateV2, BrokerFailure> {
            self.0.launch(request)
        }

        fn validate_live(
            &self,
            state: &RuntimeStateV2,
        ) -> Result<AuthenticatedEndpointSnapshot, BrokerFailure> {
            let mut endpoint = test_endpoint(state);
            endpoint.runtime_id = format!("sha256:{}", "f".repeat(64));
            Ok(endpoint)
        }

        fn sign_private_request(
            &self,
            state: &RuntimeStateV2,
            body: &[u8],
        ) -> Result<String, BrokerFailure> {
            self.0.sign_private_request(state, body)
        }
    }

    fn write_manifest(config: &BrokerConfig) {
        let arch = match config.target {
            TargetTriple::Aarch64AppleDarwin => "arm64",
            _ => "x86_64",
        };
        let chromium_executable = if matches!(config.target, TargetTriple::X86_64PcWindowsMsvc) {
            "chromium/chrome.exe"
        } else {
            "chromium/Chromium.app/Contents/MacOS/Chromium"
        };
        let mut value = serde_json::json!({
            "schema_version":1,
            "release_ready":true,
            "runtime_id":"sha256:placeholder",
            "browser_protocol":{"major":2,"min_minor":0,"max_minor":0,
              "capabilities":["scoped-ticket"],"required_capabilities":["scoped-ticket"]},
            "supervisor":{"build_id":"sup-1","rust_toolchain":"1.88.0"},
            "engine":{"build_id":"eng-1","bun_version":"1.3.8"},
            "playwright":{"version":"1.49.1"},
            "chromium":{"revision":"1148","version":"131.0.6778.33","major":131,
              "profile_schema_epoch":1,"license":"Chromium-BSD"},
            "targets":{}
        });
        value["targets"][config.target.as_str()] = serde_json::json!({
            "architecture":arch,
            "supervisor_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "engine_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "chromium_archive_url":"https://example.invalid/chromium.zip",
            "chromium_archive_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "chromium_tree_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "chromium_executable":chromium_executable,
            "license_files":["LICENSE.chromium"]
        });
        let runtime_id = RuntimeManifest::runtime_id_for_value(&value).unwrap();
        value["runtime_id"] = serde_json::Value::String(runtime_id);
        std::fs::write(
            &config.manifest_path,
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn corrupt_browser_state_is_disabled_safe_without_spawn_or_write() {
        let root =
            std::env::temp_dir().join(format!("cys-browser-broker-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let state = root.join("state.json");
        std::fs::write(&state, b"{not-json").unwrap();
        let before = std::fs::read(&state).unwrap();
        let launcher = Arc::new(CountingLauncher::default());
        let broker = AuthorityBroker::new(
            BrokerConfig::for_test(
                root.join("manifest.json"),
                root.join("runtime"),
                state.clone(),
            ),
            launcher.clone(),
        );

        let status = broker.probe();

        assert!(
            matches!(status, BrokerStatus::DisabledSafe { code, .. } if code == "CORRUPT_STATE")
        );
        assert_eq!(launcher.0.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read(&state).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disk_state_alone_is_never_a_compatible_runtime_authority() {
        let root = std::env::temp_dir().join(format!(
            "cys-browser-broker-untrusted-disk-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = BrokerConfig::for_test(
            root.join("manifest.json"),
            root.join("runtime"),
            root.join("state.json"),
        );
        write_manifest(&config);
        let launcher = Arc::new(CountingLauncher::default());
        let seed = launcher
            .launch(&LaunchRequest {
                manifest: RuntimeManifest::parse(
                    &std::fs::read_to_string(&config.manifest_path).unwrap(),
                )
                .unwrap(),
                manifest_path: config.manifest_path.clone(),
                target: config.target.clone(),
                runtime_root: config.runtime_root.clone(),
                state_path: config.state_path.clone(),
                authority: VerifiedAuthority::trusted_test(),
                cancellation: Arc::new(AtomicBool::new(false)),
            })
            .unwrap();
        std::fs::write(&config.state_path, serde_json::to_vec(&seed).unwrap()).unwrap();
        let broker = AuthorityBroker::new(config, launcher);

        assert!(matches!(
            broker.probe(),
            BrokerStatus::DisabledSafe { code, .. } if code == "UNAUTHENTICATED_STATE"
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dead_managed_runtime_is_replaced_by_one_bounded_singleflight_launch() {
        let root = std::env::temp_dir().join(format!(
            "cys-browser-broker-crash-recovery-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = BrokerConfig::for_test(
            root.join("manifest.json"),
            root.join("runtime"),
            root.join("state.json"),
        );
        write_manifest(&config);
        let launcher = Arc::new(CrashableLauncher::default());
        let broker = AuthorityBroker::new(config, launcher.clone());
        let first = broker
            .ensure_shared(&VerifiedAuthority::trusted_test())
            .unwrap();
        let BrokerStatus::Compatible { state: first } = first else {
            panic!("first launch")
        };
        launcher.live.store(false, Ordering::SeqCst);

        let second = broker
            .ensure_shared(&VerifiedAuthority::trusted_test())
            .unwrap();
        let BrokerStatus::Compatible { state: second } = second else {
            panic!("restart")
        };
        assert_ne!(first.instance_id, second.instance_id);
        assert!(second.engine_generation > first.engine_generation);
        assert_eq!(launcher.launches.load(Ordering::SeqCst), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_authorized_ensure_is_singleflight() {
        let root = std::env::temp_dir().join(format!(
            "cys-browser-broker-singleflight-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = BrokerConfig::for_test(
            root.join("manifest.json"),
            root.join("runtime"),
            root.join("missing-state.json"),
        );
        write_manifest(&config);
        let launcher = Arc::new(CountingLauncher::default());
        let broker = Arc::new(AuthorityBroker::new(config, launcher.clone()));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let broker = broker.clone();
            threads.push(std::thread::spawn(move || {
                broker
                    .ensure_shared(&VerifiedAuthority::trusted_test())
                    .unwrap()
            }));
        }
        for thread in threads {
            assert!(matches!(
                thread.join().unwrap(),
                BrokerStatus::Compatible { .. }
            ));
        }

        assert_eq!(launcher.0.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_runtime_scope_is_exactly_shared_default_headless() {
        assert!(validate_shared_headless_scope(&json!({
            "realm":"shared-default", "mode":"headless"
        }))
        .is_ok());
        for denied in [
            json!({"realm":"shared-default", "mode":"headful"}),
            json!({"realm":"other", "mode":"headless"}),
            json!({"mode":"headless"}),
        ] {
            assert_eq!(
                validate_shared_headless_scope(&denied).unwrap_err().code,
                "AUTHORITY_REJECTED"
            );
        }
    }

    #[test]
    fn gui_registration_binds_pid_incarnation_canonical_binary_and_code_identity() {
        let mut registry = GuiPeerRegistry::default();
        let genuine = GuiExecutableIdentity {
            pid: 77,
            start_time: 101,
            canonical_executable: "/Applications/cys.app/Contents/MacOS/cys-app".into(),
            code_sign_identity: "macos:com.cysjavis.terminal:Q43YA2NMF9".into(),
            executable_sha256: format!("sha256:{}", "a".repeat(64)),
        };
        let one_time = registry.issue_challenge(genuine.clone(), 1_000).unwrap();
        let session = registry
            .register(&one_time, genuine.clone(), 1_001)
            .unwrap();
        registry.validate(77, &session, &genuine).unwrap();

        let basename_spoof = GuiExecutableIdentity {
            canonical_executable: "/tmp/cys-app".into(),
            code_sign_identity: "unsigned".into(),
            executable_sha256: format!("sha256:{}", "c".repeat(64)),
            ..genuine.clone()
        };
        assert_eq!(
            registry
                .validate(77, &session, &basename_spoof)
                .unwrap_err()
                .code,
            "GUI_IDENTITY_MISMATCH"
        );
        let second = registry.issue_challenge(genuine.clone(), 2_000).unwrap();
        let session = registry.register(&second, genuine.clone(), 2_001).unwrap();
        let pid_reuse = GuiExecutableIdentity {
            start_time: 102,
            ..genuine.clone()
        };
        assert_eq!(
            registry
                .validate(77, &session, &pid_reuse)
                .unwrap_err()
                .code,
            "GUI_IDENTITY_MISMATCH"
        );
        assert_eq!(
            registry
                .register(&one_time, genuine, 2_002)
                .unwrap_err()
                .code,
            "GUI_REGISTRATION_REPLAY"
        );
    }

    #[test]
    fn windows_valid_authenticode_from_a_different_signer_is_rejected() {
        let cysd = WindowsSignerIdentity {
            leaf_thumbprint: "AA11".into(),
            spki_sha256: "11".repeat(32),
            subject: "CN=CYS Insight".into(),
        };
        let genuine_app = cysd.clone();
        assert!(same_windows_signer(&cysd, &genuine_app));
        let attacker = WindowsSignerIdentity {
            leaf_thumbprint: "BB22".into(),
            spki_sha256: "22".repeat(32),
            subject: "CN=Other Valid Publisher".into(),
        };
        assert!(!same_windows_signer(&cysd, &attacker));
    }

    #[test]
    fn windows_unsigned_release_accepts_only_the_compiled_gui_digest() {
        let digest = "ab".repeat(32);
        let identity = verify_windows_release_gui_hash(&digest, &format!("sha256:{digest}"))
            .expect("the exact release GUI hash must be accepted");
        assert_eq!(identity, format!("windows-release-sha256:{digest}"));

        let mismatch =
            verify_windows_release_gui_hash(&digest, &format!("sha256:{}", "cd".repeat(32)))
                .expect_err("a different unsigned executable must be rejected");
        assert_eq!(mismatch.code, "GUI_IDENTITY_REJECTED");
    }

    #[test]
    fn trusted_app_gesture_receipt_is_window_bound_ttl_and_one_time() {
        let root =
            std::env::temp_dir().join(format!("cys-browser-broker-gesture-{}", std::process::id()));
        let broker = AuthorityBroker::new(
            BrokerConfig::for_test(
                root.join("manifest.json"),
                root.join("runtime"),
                root.join("state.json"),
            ),
            Arc::new(CountingLauncher::default()),
        );
        let pane = "a".repeat(64);
        let receipt = broker
            .issue_user_gesture_at(77, "main", &pane, 1_000)
            .unwrap();
        assert!(broker
            .consume_user_gesture_at(78, "main", &pane, &receipt, 1_001)
            .is_err());
        broker
            .consume_user_gesture_at(77, "main", &pane, &receipt, 1_001)
            .expect("exact trusted window receipt");
        assert!(broker
            .consume_user_gesture_at(77, "main", &pane, &receipt, 1_002)
            .is_err());
        let expired = broker
            .issue_user_gesture_at(77, "main", &pane, 2_000)
            .unwrap();
        assert!(broker
            .consume_user_gesture_at(77, "main", &pane, &expired, 2_000 + 5_001)
            .is_err());
    }

    struct EndpointSwapLauncher {
        endpoint_path: PathBuf,
        authenticated_port: u16,
        swapped_port: u16,
    }

    impl SupervisorLauncher for EndpointSwapLauncher {
        fn launch(&self, request: &LaunchRequest) -> Result<RuntimeStateV2, BrokerFailure> {
            CountingLauncher::default().launch(request)
        }

        fn validate_live(
            &self,
            state: &RuntimeStateV2,
        ) -> Result<AuthenticatedEndpointSnapshot, BrokerFailure> {
            // Model a state-file replacement after an authenticated snapshot.
            // The public broker boundary must retain the authenticated endpoint,
            // not read these attacker-controlled replacement bytes.
            std::fs::write(
                &self.endpoint_path,
                serde_json::to_vec(&json!({
                    "schema_version":2,"pid":state.engine_pid,"port":self.swapped_port,
                    "token":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "cast_token":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "runtime_id":state.runtime_id,"process_start_time":123,"headless":true,
                    "instance_id":state.instance_id,"engine_generation":state.engine_generation,
                    "state_mac":"c".repeat(64)
                }))
                .unwrap(),
            )
            .unwrap();
            Ok(AuthenticatedEndpointSnapshot {
                schema_version: 2,
                pid: state.engine_pid,
                port: self.authenticated_port,
                token: "0123456789abcdef0123456789abcdef".into(),
                cast_token: "fedcba9876543210fedcba9876543210".into(),
                runtime_id: state.runtime_id.clone(),
                process_start_time: 123,
                headless: true,
                instance_id: state.instance_id.clone(),
                engine_generation: state.engine_generation,
                state_mac: "e".repeat(64),
            })
        }

        fn sign_private_request(
            &self,
            _state: &RuntimeStateV2,
            _body: &[u8],
        ) -> Result<String, BrokerFailure> {
            Ok("0".repeat(64))
        }
    }

    #[test]
    fn prepare_embed_uses_the_authenticated_endpoint_snapshot_without_second_read() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        fn serve_once(listener: TcpListener) -> std::thread::JoinHandle<bool> {
            std::thread::spawn(move || {
                listener.set_nonblocking(true).unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while std::time::Instant::now() < deadline {
                    match listener.accept() {
                        Ok((mut socket, _)) => {
                            socket
                                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                                .ok();
                            let mut request = [0_u8; 4096];
                            let _ = socket.read(&mut request);
                            let body = r#"{"ok":true}"#;
                            write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                            return true;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(_) => return false,
                    }
                }
                false
            })
        }

        let root = std::env::temp_dir().join(format!(
            "cys-browser-broker-endpoint-race-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = BrokerConfig::for_test(
            root.join("manifest.json"),
            root.join("runtime"),
            root.join("instance/state.json"),
        );
        write_manifest(&config);
        let authenticated_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let swapped_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let authenticated_port = authenticated_listener.local_addr().unwrap().port();
        let swapped_port = swapped_listener.local_addr().unwrap().port();
        let endpoint_path = config
            .state_path
            .parent()
            .unwrap()
            .join("engine/state.json");
        std::fs::create_dir_all(endpoint_path.parent().unwrap()).unwrap();
        let launcher = Arc::new(EndpointSwapLauncher {
            endpoint_path: endpoint_path.clone(),
            authenticated_port,
            swapped_port,
        });
        let broker = AuthorityBroker::new(config, launcher);
        let status = broker
            .ensure_shared(&VerifiedAuthority::trusted_test())
            .unwrap();
        let BrokerStatus::Compatible { mut state } = status else {
            panic!("runtime must be compatible")
        };
        state.port = authenticated_port;
        broker.inner.lock().unwrap().live = Some(state.clone());
        std::fs::write(
            &endpoint_path,
            serde_json::to_vec(&json!({
                "schema_version":2,"pid":state.engine_pid,"port":authenticated_port,
                "token":"0123456789abcdef0123456789abcdef",
                "cast_token":"fedcba9876543210fedcba9876543210",
                "runtime_id":state.runtime_id,"process_start_time":123,"headless":true,
                "instance_id":state.instance_id,"engine_generation":state.engine_generation,
                "state_mac":"e".repeat(64)
            }))
            .unwrap(),
        )
        .unwrap();
        let authenticated = serve_once(authenticated_listener);
        let swapped = serve_once(swapped_listener);

        let descriptor = broker
            .prepare_embed(&json!({
                "context":"default","protocol_version":2,"embed_generation":1,
                "parent_origin":"tauri://localhost","embed_ticket":"a".repeat(64),
                "pane_id":"b".repeat(64)
            }))
            .unwrap();

        assert!(descriptor
            .url
            .starts_with(&format!("http://127.0.0.1:{authenticated_port}/")));
        assert!(
            authenticated.join().unwrap(),
            "authenticated endpoint was not used"
        );
        assert!(
            !swapped.join().unwrap(),
            "replacement endpoint must never be used"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn broker_returns_embed_descriptor_without_exposing_control_token_field() {
        let root =
            std::env::temp_dir().join(format!("cys-browser-broker-embed-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let config = BrokerConfig::for_test(
            root.join("manifest.json"),
            root.join("runtime"),
            root.join("instance/state.json"),
        );
        write_manifest(&config);
        let broker = AuthorityBroker::new(config.clone(), Arc::new(CountingLauncher::default()));
        let status = broker
            .ensure_shared(&VerifiedAuthority::trusted_test())
            .unwrap();
        let BrokerStatus::Compatible { mut state } = status else {
            panic!("runtime must be compatible")
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        state.port = listener.local_addr().unwrap().port();
        broker.inner.lock().unwrap().live = Some(state.clone());
        let engine_state = config
            .state_path
            .parent()
            .unwrap()
            .join("engine/state.json");
        std::fs::create_dir_all(engine_state.parent().unwrap()).unwrap();
        std::fs::write(
            &engine_state,
            serde_json::to_vec(&json!({
                "schema_version":2,
                "pid":state.engine_pid,
                "port":state.port,
                "token":"0123456789abcdef0123456789abcdef",
                "cast_token":"fedcba9876543210fedcba9876543210",
                "runtime_id":state.runtime_id,
                "process_start_time":123,
                "headless":true,
                "instance_id":state.instance_id,
                "engine_generation":state.engine_generation,
                "state_mac":"e".repeat(64)
            }))
            .unwrap(),
        )
        .unwrap();
        let expected_runtime_id = state.runtime_id.clone();
        let registration = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0_u8; 1024];
                let read = socket.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                let Some(split) = request.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let head = String::from_utf8_lossy(&request[..split]);
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= split + 4 + length {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request);
            assert!(
                request.starts_with("POST /0123456789abcdef0123456789abcdef/embed-ticket HTTP/1.1")
            );
            assert!(
                request.contains("\r\nX-CYS-Engine-Auth: ")
                    || request.contains("\r\nx-cys-engine-auth: ")
            );
            assert!(request.contains(&format!("\"runtimeId\":\"{expected_runtime_id}\"")));
            assert!(!request.contains("\"instanceId\""));
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: 26\r\nConnection: close\r\n\r\n{{\"ok\":true,\"result\":\"issued\"}}").unwrap();
        });
        let descriptor = broker
            .prepare_embed(&json!({
                "context":"default",
                "protocol_version":2,
                "embed_generation":7,
                "parent_origin":"tauri://localhost",
                "embed_ticket":"a".repeat(64),
                "pane_id":"b".repeat(64)
            }))
            .unwrap();
        let wire = serde_json::to_value(descriptor).unwrap();
        assert_eq!(wire["embed_generation"], 7);
        assert_eq!(wire["instance_id"], state.instance_id);
        assert!(wire["url"]
            .as_str()
            .unwrap()
            .starts_with(&format!("http://127.0.0.1:{}/", state.port)));
        assert!(wire["url"]
            .as_str()
            .unwrap()
            .contains(&format!("/{}/cast/", "a".repeat(64))));
        assert!(!wire["url"]
            .as_str()
            .unwrap()
            .contains("fedcba9876543210fedcba9876543210"));
        assert!(!wire["url"]
            .as_str()
            .unwrap()
            .contains("0123456789abcdef0123456789abcdef"));
        assert!(
            wire.get("token").is_none(),
            "control token must not be a DTO field"
        );
        assert!(
            wire.get("cast_token").is_none(),
            "cast token is private endpoint state, never an EmbedDescriptor field"
        );
        registration.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn embed_descriptor_fails_closed_on_engine_runtime_mismatch() {
        let root = std::env::temp_dir().join(format!(
            "cys-browser-broker-embed-mismatch-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = BrokerConfig::for_test(
            root.join("manifest.json"),
            root.join("runtime"),
            root.join("instance/state.json"),
        );
        write_manifest(&config);
        let broker = AuthorityBroker::new(
            config.clone(),
            Arc::new(MismatchedEndpointLauncher::default()),
        );
        let status = broker
            .ensure_shared(&VerifiedAuthority::trusted_test())
            .unwrap();
        let BrokerStatus::Compatible { state: _ } = status else {
            panic!("runtime must be compatible")
        };
        let error = broker.prepare_embed(&json!({
            "context":"default","protocol_version":2,"embed_generation":1,
            "parent_origin":"tauri://localhost","embed_ticket":"a".repeat(64),"pane_id":"b".repeat(64)
        })).unwrap_err();
        assert_eq!(error.code, "RUNTIME_INTEGRITY_FAILED");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_operation_is_proxied_without_returning_private_endpoint() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let root = std::env::temp_dir().join(format!(
            "cys-browser-broker-operation-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = BrokerConfig::for_test(
            root.join("manifest.json"),
            root.join("runtime"),
            root.join("instance/state.json"),
        );
        write_manifest(&config);
        let broker = AuthorityBroker::new(config.clone(), Arc::new(CountingLauncher::default()));
        let status = broker
            .ensure_shared(&VerifiedAuthority::trusted_test())
            .unwrap();
        let BrokerStatus::Compatible { mut state } = status else {
            panic!("runtime must be compatible")
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        state.port = listener.local_addr().unwrap().port();
        broker.inner.lock().unwrap().live = Some(state.clone());
        let engine_state = config
            .state_path
            .parent()
            .unwrap()
            .join("engine/state.json");
        std::fs::create_dir_all(engine_state.parent().unwrap()).unwrap();
        std::fs::write(
            &engine_state,
            serde_json::to_vec(&json!({
                "schema_version":2,"pid":state.engine_pid,"port":state.port,
                "token":"0123456789abcdef0123456789abcdef","runtime_id":state.runtime_id,
                "cast_token":"fedcba9876543210fedcba9876543210",
                "process_start_time":123,"headless":true,
                "instance_id":state.instance_id,"engine_generation":state.engine_generation,
                "state_mac":"e".repeat(64)
            }))
            .unwrap(),
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0_u8; 1024];
                let read = socket.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..read]);
                let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&bytes[..split]);
                let content_length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .unwrap();
                if bytes.len() >= split + 4 + content_length {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&bytes);
            assert!(request.starts_with("POST /0123456789abcdef0123456789abcdef/rpc HTTP/1.1"));
            assert!(request.contains(r#""verb":"snapshot""#));
            assert!(request.contains(r#""context":"default""#));
            let body = r#"{"ok":true,"result":{"kind":"snapshot"}}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });

        let reply = broker
            .proxy_operation(&json!({
                "verb":"snapshot", "args":{"context":"default"}
            }))
            .unwrap();

        assert_eq!(reply["result"]["kind"], "snapshot");
        assert!(reply.get("token").is_none());
        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
