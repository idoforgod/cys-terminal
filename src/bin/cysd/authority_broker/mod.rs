//! BrowserAuthorityExtension: optional, lazy and isolated from CorePTY boot.

use cys::browser_runtime::{
    evaluate_compatibility, Compatibility, CompatibilityRequirement, EngineKey, ParsedRuntimeState,
    ProtocolRequirement, RuntimeManifest, RuntimeStateV2, TargetTriple,
};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorityKind {
    UserGesture,
    AuthorizedWorkerOperation,
    DirectUserCli,
    ReviewerVerificationJob,
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
                AuthorityKind::UserGesture
                    | AuthorityKind::AuthorizedWorkerOperation
                    | AuthorityKind::DirectUserCli
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

pub struct AuthorityBroker<L: SupervisorLauncher> {
    config: BrokerConfig,
    launcher: Arc<L>,
    inner: Mutex<BrokerInner>,
}

#[derive(Default)]
struct BrokerInner {
    live: Option<RuntimeStateV2>,
}

impl<L: SupervisorLauncher> AuthorityBroker<L> {
    pub fn new(config: BrokerConfig, launcher: Arc<L>) -> Self {
        Self {
            config,
            launcher,
            inner: Mutex::new(BrokerInner::default()),
        }
    }

    /// Passive inspection only. It never creates directories, repairs files or
    /// launches a process. Browser corruption is data, not a cysd boot error.
    pub fn probe(&self) -> BrokerStatus {
        if let Some(state) = self.inner.lock().unwrap().live.clone() {
            return self.status_for_v2(state);
        }
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
        self.status_for_v2(state)
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
            let status = self.status_for_v2(state);
            if matches!(status, BrokerStatus::Compatible { .. }) {
                return Ok(status);
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
        "ensure" => {
            let authority = verified_authority(daemon, params, caller_pid)?;
            serde_json::to_value(broker.ensure_shared(&authority)?).map_err(|error| {
                BrokerFailure::new(
                    "BROWSER_UNAVAILABLE",
                    format!("status encode failed: {error}"),
                )
            })
        }
        _ => Err(BrokerFailure::new(
            "METHOD_NOT_FOUND",
            format!("unknown Browser Runtime operation {operation}"),
        )),
    }
}

fn shared_broker(
) -> Result<&'static AuthorityBroker<runtime_launcher::RealSupervisorLauncher>, BrokerFailure> {
    match SHARED_BROKER.get_or_init(|| {
        BrokerConfig::bundled().map(|config| {
            AuthorityBroker::new(config, Arc::new(runtime_launcher::RealSupervisorLauncher))
        })
    }) {
        Ok(broker) => Ok(broker),
        Err(error) => Err(error.clone()),
    }
}

fn verified_authority(
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
        "user_gesture"
            if caller_surface.is_none()
                && process_basename(caller_pid).as_deref() == Some(app_name()) =>
        {
            let window = required_param(params, "window_label")?;
            let pane = required_param(params, "pane_nonce")?;
            let gesture = required_param(params, "gesture_id")?;
            (
                AuthorityKind::UserGesture,
                format!("PanePrepare:{window}:{pane}:{gesture}:shared-default/headless"),
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

fn app_name() -> &'static str {
    if cfg!(windows) {
        "cys-app.exe"
    } else {
        "cys-app"
    }
}

fn process_basename(pid: u32) -> Option<String> {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    system
        .process(Pid::from_u32(pid))?
        .exe()?
        .file_name()?
        .to_str()?
        .to_owned()
        .into()
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
}
