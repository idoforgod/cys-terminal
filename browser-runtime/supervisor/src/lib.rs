//! Native Browser Runtime supervisor.

use cys::browser_runtime::{
    EngineKey, RuntimeManifest, RuntimePaths, RuntimeStateV2, TargetTriple,
};
use std::fs::File;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupervisorError {
    code: &'static str,
    message: String,
}

impl SupervisorError {
    fn authority(message: impl Into<String>) -> Self {
        Self {
            code: "AUTHORITY_REJECTED",
            message: message.into(),
        }
    }

    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for SupervisorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SupervisorError {}

#[derive(Clone, Debug)]
pub struct SupervisorConfig {
    pub manifest_path: PathBuf,
    pub runtime_root: PathBuf,
    pub state_path: PathBuf,
    pub lock_path: PathBuf,
    pub target: TargetTriple,
}

impl SupervisorConfig {
    #[cfg(test)]
    fn for_test(root: PathBuf) -> Self {
        Self {
            manifest_path: root.join("manifest.json"),
            runtime_root: root.join("runtime"),
            state_path: root.join("state.json"),
            lock_path: root.join("supervisor.lock"),
            target: TargetTriple::current().unwrap_or(TargetTriple::Aarch64AppleDarwin),
        }
    }
}

#[derive(Debug)]
pub struct BrokerChannel {
    pub command_sequence: u64,
    pub challenge: [u8; 32],
    pub expected_runtime_id: String,
}

#[derive(Clone, Debug)]
pub struct EngineLaunchSpec {
    pub engine: PathBuf,
    pub chromium: PathBuf,
    pub runtime_id: String,
    pub chromium_revision: String,
    pub engine_key: EngineKey,
    pub engine_state_dir: PathBuf,
}

pub struct EngineProcess {
    pub pid: u32,
    pub process_group: u32,
    pub port: u16,
    child: Option<std::process::Child>,
}

impl std::fmt::Debug for EngineProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineProcess")
            .field("pid", &self.pid)
            .field("process_group", &self.process_group)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl EngineProcess {
    pub fn detached_for_test(pid: u32, process_group: u32, port: u16) -> Self {
        Self {
            pid,
            process_group,
            port,
            child: None,
        }
    }

    pub fn from_child(child: std::process::Child, process_group: u32, port: u16) -> Self {
        let pid = child.id();
        Self {
            pid,
            process_group,
            port,
            child: Some(child),
        }
    }

    pub fn wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        match self.child.as_mut() {
            Some(child) => child.try_wait(),
            None => Ok(None),
        }
    }
}

pub trait EngineLauncher {
    fn launch(&self, spec: &EngineLaunchSpec) -> Result<EngineProcess, SupervisorError>;
}

pub struct Supervisor;

pub struct RunningSupervisor {
    _lock: InstanceLock,
    pub state: RuntimeStateV2,
    pub engine: EngineProcess,
    state_path: PathBuf,
}

impl std::fmt::Debug for RunningSupervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningSupervisor")
            .field("state", &self.state)
            .field("engine", &self.engine)
            .finish_non_exhaustive()
    }
}

impl Drop for RunningSupervisor {
    fn drop(&mut self) {
        terminate_engine_group(&mut self.engine);
        if state_matches_instance(&self.state_path, &self.state.instance_id) {
            let _ = std::fs::remove_file(&self.state_path);
        }
    }
}

impl Supervisor {
    pub fn start<L: EngineLauncher>(
        config: SupervisorConfig,
        broker: Option<BrokerChannel>,
        launcher: &L,
    ) -> Result<RunningSupervisor, SupervisorError> {
        let Some(broker) = broker else {
            return Err(SupervisorError::authority(
                "an inherited cysd broker channel is required",
            ));
        };
        if broker.command_sequence == 0 || broker.challenge.iter().all(|byte| *byte == 0) {
            return Err(SupervisorError::authority(
                "invalid broker session challenge",
            ));
        }
        if broker.expected_runtime_id.is_empty() {
            return Err(SupervisorError::authority(
                "broker omitted expected runtime identity",
            ));
        }
        let parent = config.state_path.parent().ok_or_else(|| {
            SupervisorError::new("RUNTIME_PATH_REJECTED", "state path has no parent")
        })?;
        create_private_dir(parent)?;
        let lock = InstanceLock::acquire(&config.lock_path)?;
        let manifest_text = std::fs::read_to_string(&config.manifest_path).map_err(|error| {
            SupervisorError::new(
                "RUNTIME_NOT_FOUND",
                format!("manifest read failed: {error}"),
            )
        })?;
        let manifest = RuntimeManifest::parse(&manifest_text).map_err(map_runtime_error)?;
        if !manifest.release_ready {
            return Err(SupervisorError::new(
                "RUNTIME_DISABLED",
                "runtime manifest is not release-qualified",
            ));
        }
        if manifest.runtime_id != broker.expected_runtime_id {
            return Err(SupervisorError::new(
                "RUNTIME_INTEGRITY_FAILED",
                "broker and manifest runtime identities differ",
            ));
        }
        let assets = manifest.target(&config.target).map_err(map_runtime_error)?;
        let paths = RuntimePaths::resolve_existing(&config.runtime_root, &config.target, assets)
            .map_err(map_runtime_error)?;
        paths
            .verify_pinned_executables(assets)
            .map_err(map_runtime_error)?;
        let engine_key = EngineKey::shared_default();
        let engine = launcher.launch(&EngineLaunchSpec {
            engine: paths.engine,
            chromium: paths.chromium,
            runtime_id: manifest.runtime_id.clone(),
            chromium_revision: manifest.chromium.revision.clone(),
            engine_key: engine_key.clone(),
            engine_state_dir: parent.join("engine"),
        })?;
        if engine.pid == 0 || engine.process_group == 0 || engine.port == 0 {
            return Err(SupervisorError::new(
                "ENGINE_START_FAILED",
                "engine returned an invalid process identity or endpoint",
            ));
        }
        let state = RuntimeStateV2 {
            schema_version: 2,
            instance_id: random_instance_id()?,
            engine_generation: 1,
            supervisor_pid: std::process::id(),
            engine_pid: engine.pid,
            port: engine.port,
            supervisor_build_id: manifest.supervisor.build_id.clone(),
            engine_build_id: manifest.engine.build_id.clone(),
            runtime_id: manifest.runtime_id,
            attestation_id: format!("sha256:{}", "0".repeat(64)),
            policy_epoch: 1,
            policy_hash: format!("sha256:{}", "0".repeat(64)),
            protocol: manifest.browser_protocol,
            chromium_revision: manifest.chromium.revision,
            engine_key,
            profile_epoch: manifest.chromium.profile_schema_epoch,
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        write_state_atomic(&config.state_path, &state)?;
        Ok(RunningSupervisor {
            _lock: lock,
            state,
            engine,
            state_path: config.state_path,
        })
    }
}

struct InstanceLock {
    file: File,
}

impl InstanceLock {
    fn acquire(path: &std::path::Path) -> Result<Self, SupervisorError> {
        use std::fs::OpenOptions;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| SupervisorError::new("RUNTIME_START_FAILED", format!("lock open: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(SupervisorError::new(
                    "RUNTIME_ALREADY_RUNNING",
                    "another supervisor owns this EngineKey lock",
                ));
            }
        }
        Ok(Self { file })
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

fn map_runtime_error(error: cys::browser_runtime::BrowserError) -> SupervisorError {
    let code = match error.code() {
        cys::browser_runtime::BrowserErrorCode::RuntimePathRejected => "RUNTIME_PATH_REJECTED",
        cys::browser_runtime::BrowserErrorCode::RuntimeIntegrityFailed => {
            "RUNTIME_INTEGRITY_FAILED"
        }
        cys::browser_runtime::BrowserErrorCode::InvalidManifest => "INVALID_MANIFEST",
        _ => "RUNTIME_START_FAILED",
    };
    SupervisorError::new(code, error.to_string())
}

fn create_private_dir(path: &std::path::Path) -> Result<(), SupervisorError> {
    std::fs::create_dir_all(path)
        .map_err(|e| SupervisorError::new("RUNTIME_START_FAILED", format!("state dir: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(|e| {
            SupervisorError::new("RUNTIME_START_FAILED", format!("state dir mode: {e}"))
        })?;
    }
    Ok(())
}

fn write_state_atomic(
    path: &std::path::Path,
    state: &RuntimeStateV2,
) -> Result<(), SupervisorError> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let bytes = serde_json::to_vec(state)
        .map_err(|e| SupervisorError::new("RUNTIME_START_FAILED", format!("state encode: {e}")))?;
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .map_err(|e| SupervisorError::new("RUNTIME_START_FAILED", format!("state create: {e}")))?;
    file.write_all(&bytes)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|e| SupervisorError::new("RUNTIME_START_FAILED", format!("state write: {e}")))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| SupervisorError::new("RUNTIME_START_FAILED", format!("state publish: {e}")))?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

fn state_matches_instance(path: &std::path::Path, instance_id: &str) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| cys::browser_runtime::parse_runtime_state(&text).ok())
        .is_some_and(|state| matches!(state, cys::browser_runtime::ParsedRuntimeState::V2(v2) if v2.instance_id == instance_id))
}

fn random_instance_id() -> Result<String, SupervisorError> {
    let mut bytes = [0_u8; 16];
    #[cfg(unix)]
    {
        use std::io::Read;
        File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut bytes))
            .map_err(|e| SupervisorError::new("RUNTIME_START_FAILED", format!("CSPRNG: {e}")))?;
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Security::Cryptography::{
            BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        };
        if BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        ) != 0
        {
            return Err(SupervisorError::new(
                "RUNTIME_START_FAILED",
                "CNG random generation failed",
            ));
        }
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn terminate_engine_group(engine: &mut EngineProcess) {
    if engine.child.is_none() {
        return;
    }
    #[cfg(unix)]
    if engine.process_group > 1 {
        let _ = unsafe { libc::kill(-(engine.process_group as libc::pid_t), libc::SIGTERM) };
    }
    if let Some(child) = engine.child.as_mut() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        #[cfg(unix)]
        if engine.process_group > 1 {
            let _ = unsafe { libc::kill(-(engine.process_group as libc::pid_t), libc::SIGKILL) };
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub struct CommandEngineLauncher {
    startup_timeout: std::time::Duration,
}

impl Default for CommandEngineLauncher {
    fn default() -> Self {
        Self {
            startup_timeout: std::time::Duration::from_secs(20),
        }
    }
}

impl EngineLauncher for CommandEngineLauncher {
    fn launch(&self, spec: &EngineLaunchSpec) -> Result<EngineProcess, SupervisorError> {
        use std::process::{Command, Stdio};
        create_private_dir(&spec.engine_state_dir)?;
        let log_path = spec.engine_state_dir.join("engine.log");
        let stdout = private_log(&log_path)?;
        let stderr = stdout
            .try_clone()
            .map_err(|e| SupervisorError::new("ENGINE_START_FAILED", format!("log clone: {e}")))?;
        let mut command = Command::new(&spec.engine);
        command
            .arg("--headless")
            .env("CYS_BROWSER_V2_ENGINE_ROOT", &spec.engine_state_dir)
            .env("CYS_BROWSER_CHROMIUM_PATH", &spec.chromium)
            .env("CYS_BROWSER_RUNTIME_ID", &spec.runtime_id)
            .env("CYS_BROWSER_STRICT_RUNTIME", "1")
            .env_remove("PATH")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().map_err(|e| {
            SupervisorError::new("ENGINE_START_FAILED", format!("engine spawn failed: {e}"))
        })?;
        let pid = child.id();
        let process_group = pid;
        let state_path = spec.engine_state_dir.join("state.json");
        let deadline = std::time::Instant::now() + self.startup_timeout;
        loop {
            if let Ok(text) = std::fs::read_to_string(&state_path) {
                match cys::browser_runtime::parse_runtime_state(&text) {
                    Ok(cys::browser_runtime::ParsedRuntimeState::LegacyIncompatible(state))
                        if state.pid == pid && state.port != 0 =>
                    {
                        return Ok(EngineProcess::from_child(child, process_group, state.port));
                    }
                    Ok(cys::browser_runtime::ParsedRuntimeState::V2(state))
                        if state.engine_pid == pid && state.port != 0 =>
                    {
                        return Ok(EngineProcess::from_child(child, process_group, state.port));
                    }
                    _ => {}
                }
            }
            if let Some(status) = child.try_wait().map_err(|e| {
                SupervisorError::new("ENGINE_START_FAILED", format!("engine wait failed: {e}"))
            })? {
                return Err(SupervisorError::new(
                    "ENGINE_START_FAILED",
                    format!("engine exited before ready: {status}"),
                ));
            }
            if std::time::Instant::now() >= deadline {
                let mut process = EngineProcess::from_child(child, process_group, 1);
                terminate_engine_group(&mut process);
                return Err(SupervisorError::new(
                    "ENGINE_START_FAILED",
                    "engine readiness timed out",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

fn private_log(path: &std::path::Path) -> Result<File, SupervisorError> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|e| SupervisorError::new("ENGINE_START_FAILED", format!("engine log: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct FakeEngineLauncher(AtomicUsize);

    impl EngineLauncher for FakeEngineLauncher {
        fn launch(&self, _spec: &EngineLaunchSpec) -> Result<EngineProcess, SupervisorError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            unreachable!("missing broker authority must fail before engine launch")
        }
    }

    #[derive(Default)]
    struct StartingLauncher(AtomicUsize);

    impl EngineLauncher for StartingLauncher {
        fn launch(&self, _spec: &EngineLaunchSpec) -> Result<EngineProcess, SupervisorError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(EngineProcess::detached_for_test(424_242, 424_242, 53111))
        }
    }

    fn sha256(path: &std::path::Path) -> String {
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(path).unwrap();
        format!("{:x}", Sha256::digest(bytes))
    }

    #[cfg(unix)]
    fn qualified_fixture(config: &SupervisorConfig) -> String {
        use std::os::unix::fs::PermissionsExt;
        let target_root = config.runtime_root.join(config.target.as_str());
        let supervisor = target_root.join(format!(
            "supervisor/cys-browserd{}",
            config.target.executable_suffix()
        ));
        let engine = target_root.join(format!(
            "engine/cys-browser-engine{}",
            config.target.executable_suffix()
        ));
        let chromium_rel = if matches!(config.target, TargetTriple::X86_64PcWindowsMsvc) {
            "chromium/chrome.exe"
        } else {
            "chromium/Chromium.app/Contents/MacOS/Chromium"
        };
        let chromium = target_root.join(chromium_rel);
        for path in [&supervisor, &engine, &chromium] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, path.to_string_lossy().as_bytes()).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let arch = match config.target {
            TargetTriple::Aarch64AppleDarwin => "arm64",
            _ => "x86_64",
        };
        let mut value = serde_json::json!({
            "schema_version":1,"release_ready":true,"runtime_id":"sha256:placeholder",
            "browser_protocol":{"major":2,"min_minor":0,"max_minor":0,
              "capabilities":["scoped-ticket"],"required_capabilities":["scoped-ticket"]},
            "supervisor":{"build_id":"sup-test","rust_toolchain":"test"},
            "engine":{"build_id":"engine-test","bun_version":"test"},
            "playwright":{"version":"1.49.1"},
            "chromium":{"revision":"1148","version":"131.0.6778.33","major":131,
              "profile_schema_epoch":1,"license":"Chromium-BSD"},
            "targets":{}
        });
        value["targets"][config.target.as_str()] = serde_json::json!({
            "architecture":arch,
            "supervisor_sha256":sha256(&supervisor),
            "engine_sha256":sha256(&engine),
            "chromium_archive_url":"https://example.invalid/chromium.zip",
            "chromium_archive_sha256":"a".repeat(64),
            "chromium_tree_sha256":"b".repeat(64),
            "chromium_executable":chromium_rel,
            "license_files":["LICENSE.chromium"]
        });
        let runtime_id = RuntimeManifest::runtime_id_for_value(&value).unwrap();
        value["runtime_id"] = serde_json::Value::String(runtime_id.clone());
        std::fs::write(&config.manifest_path, serde_json::to_vec(&value).unwrap()).unwrap();
        runtime_id
    }

    fn channel(runtime_id: String) -> BrokerChannel {
        BrokerChannel {
            command_sequence: 1,
            challenge: [7; 32],
            expected_runtime_id: runtime_id,
        }
    }

    #[test]
    fn missing_broker_channel_fails_before_lock_state_or_engine() {
        let root =
            std::env::temp_dir().join(format!("cys-browserd-no-authority-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let config = SupervisorConfig::for_test(root.clone());
        let launcher = FakeEngineLauncher::default();

        let error = Supervisor::start(config.clone(), None, &launcher)
            .expect_err("direct supervisor start must fail closed");

        assert_eq!(error.code(), "AUTHORITY_REJECTED");
        assert_eq!(launcher.0.load(Ordering::SeqCst), 0);
        assert!(!config.state_path.exists());
        assert!(!config.lock_path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn same_engine_key_has_exactly_one_supervisor_writer() {
        let root =
            std::env::temp_dir().join(format!("cys-browserd-single-writer-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let config = SupervisorConfig::for_test(root.clone());
        let runtime_id = qualified_fixture(&config);
        let launcher = StartingLauncher::default();

        let running =
            Supervisor::start(config.clone(), Some(channel(runtime_id.clone())), &launcher)
                .expect("first supervisor owns the EngineKey");
        let second = Supervisor::start(config.clone(), Some(channel(runtime_id)), &launcher)
            .expect_err("second supervisor must fail before engine launch");

        assert_eq!(second.code(), "RUNTIME_ALREADY_RUNNING");
        assert_eq!(launcher.0.load(Ordering::SeqCst), 1);
        assert!(config.state_path.exists());
        drop(running);
        assert!(!config.state_path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
