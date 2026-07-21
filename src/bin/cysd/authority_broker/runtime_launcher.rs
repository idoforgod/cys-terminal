use super::{AuthenticatedEndpointSnapshot, BrokerFailure, LaunchRequest, SupervisorLauncher};
use cys::browser_runtime::{BrokerHello, ParsedRuntimeState, RuntimePaths, RuntimeStateV2};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub(super) struct RealSupervisorLauncher {
    managed: Arc<Mutex<HashMap<String, ManagedRuntime>>>,
}

const MAX_MANAGED_RUNTIMES: usize = 4;
type ManagedSessionKey = zeroize::Zeroizing<[u8; 32]>;

impl Default for RealSupervisorLauncher {
    fn default() -> Self {
        Self {
            managed: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Clone)]
struct ManagedRuntime {
    state: RuntimeStateV2,
    session_key: ManagedSessionKey,
    endpoint_path: std::path::PathBuf,
    supervisor_started_at: u64,
    engine_started_at: u64,
    alive: Arc<AtomicBool>,
}

impl RealSupervisorLauncher {
    fn prune_dead_managed(&self) {
        self.managed
            .lock()
            .unwrap()
            .retain(|_, runtime| runtime.alive.load(Ordering::SeqCst));
    }

    fn reserve_managed_slot(&self) -> Result<(), BrokerFailure> {
        self.prune_dead_managed();
        if self.managed.lock().unwrap().len() >= MAX_MANAGED_RUNTIMES {
            return Err(BrokerFailure::new(
                "RUNTIME_RESOURCE_LIMIT",
                "managed Browser Runtime registry is at its hard limit",
            ));
        }
        Ok(())
    }

    fn remove_managed_incarnation(
        &self,
        instance_id: &str,
        engine_generation: u64,
        alive: &Arc<AtomicBool>,
    ) {
        remove_managed_incarnation(&self.managed, instance_id, engine_generation, alive);
    }

    fn cleanup_failed_validation(&self, runtime: &ManagedRuntime) {
        self.remove_managed_incarnation(
            &runtime.state.instance_id,
            runtime.state.engine_generation,
            &runtime.alive,
        );
    }

    fn commit_managed_runtime(
        &self,
        runtime: ManagedRuntime,
        cancellation: &Arc<AtomicBool>,
    ) -> Result<(), BrokerFailure> {
        let instance_id = runtime.state.instance_id.clone();
        let engine_generation = runtime.state.engine_generation;
        let alive = runtime.alive.clone();
        self.managed
            .lock()
            .unwrap()
            .insert(instance_id.clone(), runtime);
        if cancellation.load(Ordering::SeqCst) {
            self.remove_managed_incarnation(&instance_id, engine_generation, &alive);
            return Err(BrokerFailure::new(
                "BROWSER_CANCELLED",
                "Browser launch was cancelled before managed-runtime commit",
            ));
        }
        Ok(())
    }
}

fn remove_managed_incarnation(
    managed: &Arc<Mutex<HashMap<String, ManagedRuntime>>>,
    instance_id: &str,
    engine_generation: u64,
    alive: &Arc<AtomicBool>,
) {
    let mut managed = managed.lock().unwrap();
    let remove = managed.get(instance_id).is_some_and(|runtime| {
        runtime.state.engine_generation == engine_generation && Arc::ptr_eq(&runtime.alive, alive)
    });
    if remove {
        managed.remove(instance_id);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedEngineEndpoint {
    schema_version: u16,
    pid: u32,
    port: u16,
    token: String,
    cast_token: String,
    runtime_id: String,
    process_start_time: u64,
    headless: bool,
    instance_id: String,
    engine_generation: u64,
    state_mac: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeAttestation {
    attestation_schema: u16,
    key_id: String,
    signed_at: i64,
    expires_at: i64,
    runtime_id: String,
    target: String,
    supervisor_sha256: String,
    engine_sha256: String,
    chromium_tree_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimePolicySnapshot {
    key_id: String,
    signed_at: i64,
    expires_at: i64,
    epoch: u64,
    allowed_runtime_ids: Vec<String>,
}

impl SupervisorLauncher for RealSupervisorLauncher {
    fn launch(&self, request: &LaunchRequest) -> Result<RuntimeStateV2, BrokerFailure> {
        // Transition A is intentionally disabled-safe. Release B must be built by
        // the pinned release lane after target attestation/policy verification;
        // a mutable disk manifest cannot enable Browser by itself.
        if option_env!("CYS_BROWSER_V2_RELEASE_QUALIFIED") != Some("1") {
            return Err(BrokerFailure::new(
                "RUNTIME_DISABLED",
                "this build does not carry Browser Runtime release qualification",
            ));
        }
        ensure_launch_not_cancelled(&request.cancellation)?;
        self.reserve_managed_slot()?;
        let assets = request
            .manifest
            .target(&request.target)
            .map_err(runtime_failure)?;
        let paths = RuntimePaths::resolve_existing(&request.runtime_root, &request.target, assets)
            .and_then(|paths| {
                paths.verify_pinned_executables(assets)?;
                Ok(paths)
            })
            .map_err(runtime_failure)?;
        let resource_root = request
            .manifest_path
            .parent()
            .ok_or_else(|| BrokerFailure::new("RUNTIME_NOT_FOUND", "manifest root unavailable"))?;
        let (attestation_id, policy_epoch, policy_hash) =
            verify_release_metadata(resource_root, request, assets)?;
        ensure_launch_not_cancelled(&request.cancellation)?;
        let session_key = random_session_key()?;
        let hello = BrokerHello::signed(
            std::process::id(),
            1,
            &request.authority.receipt_id,
            &request.authority.normalized_request_hash,
            &request.authority.subject_hash,
            &request.manifest.runtime_id,
            attestation_id,
            policy_epoch,
            policy_hash,
            session_key,
        )
        .map_err(runtime_failure)?;
        let state_parent = request.state_path.parent().ok_or_else(|| {
            BrokerFailure::new("RUNTIME_PATH_REJECTED", "v2 state path has no parent")
        })?;
        let lock_path = state_parent.join("supervisor.lock");
        let mut command = Command::new(&paths.supervisor);
        command
            .arg("supervise")
            .arg("--broker-handle")
            .arg("inherited-stdin")
            .arg("--manifest")
            .arg(&request.manifest_path)
            .arg("--runtime-root")
            .arg(&request.runtime_root)
            .arg("--state")
            .arg(&request.state_path)
            .arg("--lock")
            .arg(lock_path)
            .arg("--target")
            .arg(request.target.as_str())
            .env_remove("PATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = spawn_supervisor_cancellable(&mut command, &request.cancellation)?;
        let mut control = child.stdin.take().ok_or_else(|| {
            BrokerFailure::new(
                "RUNTIME_START_FAILED",
                "supervisor control pipe unavailable",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            BrokerFailure::new(
                "RUNTIME_START_FAILED",
                "supervisor readiness pipe unavailable",
            )
        })?;
        let write_result = control
            .write_all(&session_key)
            .and_then(|_| {
                hello
                    .to_line()
                    .map_err(std::io::Error::other)
                    .and_then(|line| control.write_all(&line))
            })
            .and_then(|_| control.flush());
        if let Err(error) = write_result {
            let _ = child.kill();
            let _ = child.wait();
            return Err(BrokerFailure::new(
                "RUNTIME_START_FAILED",
                format!("private broker handshake failed: {error}"),
            ));
        }
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout)
                .take(64 * 1024 + 1)
                .read_line(&mut line)
                .map(|count| (count, line));
            let _ = ready_tx.send(result);
        });
        let state = match wait_for_supervisor_readiness(
            &mut child,
            ready_rx,
            &request.cancellation,
            std::time::Duration::from_secs(25),
        )? {
            (count, line) if count > 0 && count <= 64 * 1024 => {
                match cys::browser_runtime::parse_runtime_state(&line) {
                    Ok(ParsedRuntimeState::V2(state)) => state,
                    Ok(ParsedRuntimeState::LegacyIncompatible(_)) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(BrokerFailure::new(
                            "PROTOCOL_MISMATCH",
                            "supervisor returned legacy state",
                        ));
                    }
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(runtime_failure(error));
                    }
                }
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BrokerFailure::new(
                    "RUNTIME_START_FAILED",
                    "supervisor readiness timed out or returned an invalid frame",
                ));
            }
        };
        let supervisor_started_at = process_start_time(state.supervisor_pid).ok_or_else(|| {
            BrokerFailure::new(
                "RUNTIME_START_FAILED",
                "supervisor process identity unavailable",
            )
        })?;
        let engine_started_at = process_start_time(state.engine_pid).ok_or_else(|| {
            BrokerFailure::new(
                "RUNTIME_START_FAILED",
                "engine process identity unavailable",
            )
        })?;
        if request.cancellation.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(BrokerFailure::new(
                "BROWSER_CANCELLED",
                "Browser launch was cancelled after readiness",
            ));
        }
        let alive = Arc::new(AtomicBool::new(true));
        let managed_runtime = ManagedRuntime {
            state: state.clone(),
            session_key: ManagedSessionKey::new(session_key),
            endpoint_path: request
                .state_path
                .parent()
                .expect("validated state parent")
                .join("engine/state.json"),
            supervisor_started_at,
            engine_started_at,
            alive: alive.clone(),
        };
        if let Err(error) = self.commit_managed_runtime(managed_runtime, &request.cancellation) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let managed = self.managed.clone();
        let instance_id = state.instance_id.clone();
        let engine_generation = state.engine_generation;
        let watcher_alive = alive.clone();
        std::thread::spawn(move || {
            let _control_liveness = control;
            let _ = child.wait();
            watcher_alive.store(false, Ordering::SeqCst);
            remove_managed_incarnation(&managed, &instance_id, engine_generation, &watcher_alive);
        });
        Ok(state)
    }

    fn validate_live(
        &self,
        state: &RuntimeStateV2,
    ) -> Result<AuthenticatedEndpointSnapshot, BrokerFailure> {
        let runtime = self
            .managed
            .lock()
            .unwrap()
            .get(&state.instance_id)
            .cloned()
            .ok_or_else(|| {
                BrokerFailure::new(
                    "UNAUTHENTICATED_STATE",
                    "runtime is not owned by this broker session",
                )
            })?;
        if runtime.state != *state {
            return Err(BrokerFailure::new(
                "ENGINE_EXITED",
                "managed runtime state was replaced by a newer incarnation",
            ));
        }
        if !runtime.alive.load(Ordering::SeqCst) {
            self.cleanup_failed_validation(&runtime);
            return Err(BrokerFailure::new(
                "ENGINE_EXITED",
                "managed runtime exited",
            ));
        }
        if process_start_time(state.supervisor_pid) != Some(runtime.supervisor_started_at)
            || process_start_time(state.engine_pid) != Some(runtime.engine_started_at)
        {
            self.cleanup_failed_validation(&runtime);
            return Err(BrokerFailure::new(
                "PROCESS_IDENTITY_MISMATCH",
                "managed runtime PID incarnation changed",
            ));
        }
        let validated = (|| {
            let input = std::fs::read(&runtime.endpoint_path).map_err(|error| {
                BrokerFailure::new(
                    "ENGINE_ENDPOINT_UNAVAILABLE",
                    format!("private engine endpoint unavailable: {error}"),
                )
            })?;
            let endpoint: AuthenticatedEngineEndpoint =
                serde_json::from_slice(&input).map_err(|error| {
                    BrokerFailure::new(
                        "RUNTIME_INTEGRITY_FAILED",
                        format!("invalid authenticated engine endpoint: {error}"),
                    )
                })?;
            verify_engine_endpoint(state, &runtime, &endpoint)?;
            authenticated_health(&endpoint)?;
            Ok(AuthenticatedEndpointSnapshot {
                schema_version: endpoint.schema_version,
                pid: endpoint.pid,
                port: endpoint.port,
                token: endpoint.token,
                cast_token: endpoint.cast_token,
                runtime_id: endpoint.runtime_id,
                process_start_time: endpoint.process_start_time,
                headless: endpoint.headless,
                instance_id: endpoint.instance_id,
                engine_generation: endpoint.engine_generation,
                state_mac: endpoint.state_mac,
            })
        })();
        if validated.is_err() {
            self.cleanup_failed_validation(&runtime);
        }
        validated
    }

    fn sign_private_request(
        &self,
        state: &RuntimeStateV2,
        body: &[u8],
    ) -> Result<String, BrokerFailure> {
        let runtime = self
            .managed
            .lock()
            .unwrap()
            .get(&state.instance_id)
            .cloned()
            .ok_or_else(|| {
                BrokerFailure::new(
                    "UNAUTHENTICATED_STATE",
                    "private engine request has no broker-owned runtime session",
                )
            })?;
        if runtime.state != *state {
            return Err(BrokerFailure::new(
                "ENGINE_EXITED",
                "private engine request targeted a replaced runtime",
            ));
        }
        if !runtime.alive.load(Ordering::SeqCst) {
            self.cleanup_failed_validation(&runtime);
            return Err(BrokerFailure::new(
                "ENGINE_EXITED",
                "private engine request targeted a stale runtime",
            ));
        }
        let mut signer =
            Hmac::<Sha256>::new_from_slice(runtime.session_key.as_ref()).map_err(|_| {
                BrokerFailure::new("RUNTIME_INTEGRITY_FAILED", "invalid private engine MAC key")
            })?;
        signer.update(b"cys.browser.private-control.v1\0");
        signer.update(body);
        Ok(signer
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}

fn ensure_launch_not_cancelled(cancelled: &Arc<AtomicBool>) -> Result<(), BrokerFailure> {
    if cancelled.load(Ordering::SeqCst) {
        return Err(BrokerFailure::new(
            "BROWSER_CANCELLED",
            "Browser launch was cancelled before process creation",
        ));
    }
    Ok(())
}

fn spawn_supervisor_cancellable(
    command: &mut Command,
    cancelled: &Arc<AtomicBool>,
) -> Result<std::process::Child, BrokerFailure> {
    ensure_launch_not_cancelled(cancelled)?;
    let mut child = command.spawn().map_err(|error| {
        BrokerFailure::new(
            "RUNTIME_START_FAILED",
            format!("verified supervisor spawn failed: {error}"),
        )
    })?;
    // Close the narrow check→spawn race: cancellation may win immediately
    // after the pre-spawn check, but it must never leave a detached child.
    if cancelled.load(Ordering::SeqCst) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(BrokerFailure::new(
            "BROWSER_CANCELLED",
            "Browser launch was cancelled during process creation",
        ));
    }
    Ok(child)
}

fn wait_for_supervisor_readiness(
    child: &mut std::process::Child,
    receiver: std::sync::mpsc::Receiver<std::io::Result<(usize, String)>>,
    cancelled: &Arc<AtomicBool>,
    timeout: std::time::Duration,
) -> Result<(usize, String), BrokerFailure> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(BrokerFailure::new(
                "BROWSER_CANCELLED",
                "Browser launch was cancelled while waiting for readiness",
            ));
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(BrokerFailure::new(
                "RUNTIME_START_FAILED",
                "supervisor readiness timed out",
            ));
        }
        match receiver.recv_timeout(std::time::Duration::from_millis(20)) {
            Ok(Ok(frame)) => return Ok(frame),
            Ok(Err(error)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BrokerFailure::new(
                    "RUNTIME_START_FAILED",
                    format!("supervisor readiness read failed: {error}"),
                ));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if child.try_wait().ok().flatten().is_some() {
                    return Err(BrokerFailure::new(
                        "RUNTIME_START_FAILED",
                        "supervisor exited before readiness",
                    ));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BrokerFailure::new(
                    "RUNTIME_START_FAILED",
                    "supervisor readiness channel disconnected",
                ));
            }
        }
    }
}

fn verify_engine_endpoint(
    state: &RuntimeStateV2,
    managed: &ManagedRuntime,
    endpoint: &AuthenticatedEngineEndpoint,
) -> Result<(), BrokerFailure> {
    if endpoint.schema_version != 2
        || endpoint.pid != state.engine_pid
        || endpoint.port != state.port
        || endpoint.runtime_id != state.runtime_id
        || endpoint.process_start_time != managed.engine_started_at
        || !endpoint.headless
        || endpoint.instance_id != state.instance_id
        || endpoint.engine_generation != state.engine_generation
        || endpoint.token.len() != 32
        || endpoint.cast_token.len() != 32
        || endpoint.token == endpoint.cast_token
    {
        return Err(BrokerFailure::new(
            "RUNTIME_INTEGRITY_FAILED",
            "engine endpoint identity does not match the managed runtime",
        ));
    }
    let payload = serde_json::to_vec(&json!([
        endpoint.schema_version,
        endpoint.pid,
        endpoint.port,
        endpoint.token,
        endpoint.cast_token,
        endpoint.runtime_id,
        endpoint.process_start_time,
        endpoint.headless,
        endpoint.instance_id,
        endpoint.engine_generation,
    ]))
    .map_err(|error| BrokerFailure::new("RUNTIME_INTEGRITY_FAILED", error.to_string()))?;
    let supplied = decode_mac(&endpoint.state_mac)?;
    let mut verifier = Hmac::<Sha256>::new_from_slice(managed.session_key.as_ref())
        .map_err(|_| BrokerFailure::new("RUNTIME_INTEGRITY_FAILED", "invalid engine MAC key"))?;
    verifier.update(b"cys.browser.engine-state.v1\0");
    verifier.update(&payload);
    verifier
        .verify_slice(&supplied)
        .map_err(|_| BrokerFailure::new("RUNTIME_INTEGRITY_FAILED", "engine endpoint MAC mismatch"))
}

fn authenticated_health(endpoint: &AuthenticatedEngineEndpoint) -> Result<(), BrokerFailure> {
    use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
    use std::time::Duration;
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, endpoint.port);
    let mut stream = TcpStream::connect_timeout(&address.into(), Duration::from_millis(500))
        .map_err(|error| {
            BrokerFailure::new("ENGINE_EXITED", format!("engine health connect: {error}"))
        })?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(1))).ok();
    let body = br#"{"verb":"status","args":{}}"#;
    write!(
        stream,
        "POST /{}/rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.token,
        endpoint.port,
        body.len()
    )
    .and_then(|_| stream.write_all(body))
    .map_err(|error| BrokerFailure::new("ENGINE_EXITED", format!("engine health write: {error}")))?;
    let mut response = Vec::new();
    stream
        .take(1024 * 1024)
        .read_to_end(&mut response)
        .map_err(|error| {
            BrokerFailure::new("ENGINE_EXITED", format!("engine health read: {error}"))
        })?;
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            BrokerFailure::new(
                "ENGINE_EXITED",
                "engine health returned an invalid HTTP response",
            )
        })?;
    if !response.starts_with(b"HTTP/1.1 200 ") && !response.starts_with(b"HTTP/1.0 200 ") {
        return Err(BrokerFailure::new(
            "ENGINE_EXITED",
            "engine health returned non-200",
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&response[split + 4..]).map_err(|error| {
            BrokerFailure::new("ENGINE_EXITED", format!("engine health JSON: {error}"))
        })?;
    if value
        .pointer("/result/pid")
        .and_then(serde_json::Value::as_u64)
        != Some(endpoint.pid as u64)
        || value
            .pointer("/result/runtime_id")
            .and_then(serde_json::Value::as_str)
            != Some(endpoint.runtime_id.as_str())
        || value
            .pointer("/result/process_start_time")
            .and_then(serde_json::Value::as_u64)
            != Some(endpoint.process_start_time)
    {
        return Err(BrokerFailure::new(
            "PROCESS_IDENTITY_MISMATCH",
            "engine health identity mismatch",
        ));
    }
    Ok(())
}

fn process_start_time(pid: u32) -> Option<u64> {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    system
        .process(Pid::from_u32(pid))
        .map(sysinfo::Process::start_time)
}

fn decode_mac(input: &str) -> Result<[u8; 32], BrokerFailure> {
    if input.len() != 64 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BrokerFailure::new(
            "RUNTIME_INTEGRITY_FAILED",
            "engine state MAC is malformed",
        ));
    }
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&input[index * 2..index * 2 + 2], 16).map_err(|_| {
            BrokerFailure::new("RUNTIME_INTEGRITY_FAILED", "engine state MAC is malformed")
        })?;
    }
    Ok(output)
}

fn verify_release_metadata(
    root: &std::path::Path,
    request: &LaunchRequest,
    assets: &cys::browser_runtime::TargetAssets,
) -> Result<(String, u64, String), BrokerFailure> {
    let attestation_bytes =
        std::fs::read(root.join("runtime-attestation.json")).map_err(|error| {
            BrokerFailure::new(
                "RUNTIME_INTEGRITY_FAILED",
                format!("runtime attestation unavailable: {error}"),
            )
        })?;
    let attestation: RuntimeAttestation =
        serde_json::from_slice(&attestation_bytes).map_err(|error| {
            BrokerFailure::new(
                "RUNTIME_INTEGRITY_FAILED",
                format!("invalid attestation: {error}"),
            )
        })?;
    let now = chrono::Utc::now().timestamp();
    if attestation.signed_at > now || now > attestation.expires_at {
        return Err(BrokerFailure::new(
            "RUNTIME_INTEGRITY_FAILED",
            "runtime attestation is outside its signed validity window",
        ));
    }
    let attestation_signature = std::fs::read(root.join("runtime-attestation.json.minisig"))
        .map_err(|error| {
            BrokerFailure::new(
                "RUNTIME_INTEGRITY_FAILED",
                format!("runtime attestation signature unavailable: {error}"),
            )
        })?;
    cys::packsig::verify_release_metadata_signature(
        &attestation.key_id,
        &attestation_bytes,
        &attestation_signature,
        now,
    )
    .map_err(|error| BrokerFailure::new("RUNTIME_INTEGRITY_FAILED", error))?;
    if attestation.attestation_schema != 1
        || attestation.runtime_id != request.manifest.runtime_id
        || attestation.target != request.target.as_str()
        || attestation.supervisor_sha256 != assets.supervisor_sha256
        || attestation.engine_sha256 != assets.engine_sha256
        || attestation.chromium_tree_sha256 != assets.chromium_tree_sha256
    {
        return Err(BrokerFailure::new(
            "RUNTIME_INTEGRITY_FAILED",
            "runtime attestation does not bind the selected target",
        ));
    }
    let policy_bytes =
        std::fs::read(root.join("runtime-policy-snapshot.json")).map_err(|error| {
            BrokerFailure::new(
                "RUNTIME_INTEGRITY_FAILED",
                format!("runtime policy unavailable: {error}"),
            )
        })?;
    let policy: RuntimePolicySnapshot = serde_json::from_slice(&policy_bytes).map_err(|error| {
        BrokerFailure::new(
            "RUNTIME_INTEGRITY_FAILED",
            format!("invalid runtime policy: {error}"),
        )
    })?;
    if policy.signed_at > now || now > policy.expires_at {
        return Err(BrokerFailure::new(
            "RUNTIME_REVOKED",
            "runtime policy is outside its signed validity window",
        ));
    }
    let policy_signature = std::fs::read(root.join("runtime-policy-snapshot.json.minisig"))
        .map_err(|error| {
            BrokerFailure::new(
                "RUNTIME_INTEGRITY_FAILED",
                format!("runtime policy signature unavailable: {error}"),
            )
        })?;
    cys::packsig::verify_release_metadata_signature(
        &policy.key_id,
        &policy_bytes,
        &policy_signature,
        now,
    )
    .map_err(|error| BrokerFailure::new("RUNTIME_INTEGRITY_FAILED", error))?;
    if policy.epoch == 0
        || !policy
            .allowed_runtime_ids
            .contains(&request.manifest.runtime_id)
    {
        return Err(BrokerFailure::new(
            "RUNTIME_REVOKED",
            "selected runtime is not allowed by current policy",
        ));
    }
    Ok((
        format!("sha256:{:x}", Sha256::digest(&attestation_bytes)),
        policy.epoch,
        format!("sha256:{:x}", Sha256::digest(&policy_bytes)),
    ))
}

fn random_session_key() -> Result<[u8; 32], BrokerFailure> {
    let hex = crate::channels::random_token_hex()
        .map_err(|error| BrokerFailure::new("RUNTIME_START_FAILED", error))?;
    let mut key = [0_u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).map_err(|_| {
            BrokerFailure::new("RUNTIME_START_FAILED", "CSPRNG returned malformed bytes")
        })?;
    }
    Ok(key)
}

fn runtime_failure(error: cys::browser_runtime::BrowserError) -> BrokerFailure {
    BrokerFailure::new(error.code().as_str(), error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_for(instance: usize) -> RuntimeStateV2 {
        RuntimeStateV2 {
            schema_version: 2,
            instance_id: format!("{instance:032x}"),
            engine_generation: instance as u64 + 1,
            supervisor_pid: 10,
            engine_pid: 11,
            port: 53111,
            supervisor_build_id: "sup".into(),
            engine_build_id: "eng".into(),
            runtime_id: format!("sha256:{}", "a".repeat(64)),
            attestation_id: format!("sha256:{}", "b".repeat(64)),
            policy_epoch: 1,
            policy_hash: format!("sha256:{}", "c".repeat(64)),
            protocol: cys::browser_runtime::ProtocolRange {
                major: 2,
                min_minor: 0,
                max_minor: 0,
                capabilities: ["scoped-ticket".into()].into_iter().collect(),
                required_capabilities: ["scoped-ticket".into()].into_iter().collect(),
            },
            chromium_revision: "1148".into(),
            engine_key: cys::browser_runtime::EngineKey::shared_default(),
            profile_epoch: 1,
            started_at: "2026-07-21T00:00:00Z".into(),
        }
    }

    #[test]
    fn managed_session_key_has_compiler_guaranteed_zeroize_contract() {
        fn assert_zeroize<T: zeroize::Zeroize>() {}
        assert_zeroize::<ManagedSessionKey>();
    }

    #[test]
    fn crash_loop_prunes_dead_session_keys_and_bounds_registry() {
        let launcher = RealSupervisorLauncher::default();
        for index in 0..(MAX_MANAGED_RUNTIMES * 4) {
            let state = state_for(index);
            launcher.managed.lock().unwrap().insert(
                state.instance_id.clone(),
                ManagedRuntime {
                    state,
                    session_key: ManagedSessionKey::new([index as u8; 32]),
                    endpoint_path: std::path::PathBuf::from("missing"),
                    supervisor_started_at: 1,
                    engine_started_at: 1,
                    alive: Arc::new(AtomicBool::new(false)),
                },
            );
            launcher.prune_dead_managed();
        }
        let managed = launcher.managed.lock().unwrap();
        assert!(managed.len() <= MAX_MANAGED_RUNTIMES);
        assert!(managed
            .values()
            .all(|runtime| runtime.alive.load(Ordering::SeqCst)));
    }

    #[test]
    fn late_exit_watcher_cannot_delete_a_replacement_incarnation() {
        let launcher = RealSupervisorLauncher::default();
        let instance_id = "f".repeat(32);
        let old_alive = Arc::new(AtomicBool::new(true));
        let mut old_state = state_for(1);
        old_state.instance_id = instance_id.clone();
        old_state.engine_generation = 1;
        launcher.managed.lock().unwrap().insert(
            instance_id.clone(),
            ManagedRuntime {
                state: old_state,
                session_key: ManagedSessionKey::new([1; 32]),
                endpoint_path: "old".into(),
                supervisor_started_at: 1,
                engine_started_at: 1,
                alive: old_alive.clone(),
            },
        );
        let replacement_alive = Arc::new(AtomicBool::new(true));
        let mut replacement_state = state_for(2);
        replacement_state.instance_id = instance_id.clone();
        replacement_state.engine_generation = 2;
        launcher.managed.lock().unwrap().insert(
            instance_id.clone(),
            ManagedRuntime {
                state: replacement_state,
                session_key: ManagedSessionKey::new([2; 32]),
                endpoint_path: "replacement".into(),
                supervisor_started_at: 2,
                engine_started_at: 2,
                alive: replacement_alive.clone(),
            },
        );

        launcher.remove_managed_incarnation(&instance_id, 1, &old_alive);

        let managed = launcher.managed.lock().unwrap();
        let retained = managed.get(&instance_id).expect("replacement retained");
        assert_eq!(retained.state.engine_generation, 2);
        assert!(Arc::ptr_eq(&retained.alive, &replacement_alive));
    }

    #[test]
    fn failed_validation_cleanup_cannot_delete_a_replacement_incarnation() {
        let launcher = RealSupervisorLauncher::default();
        let instance_id = "e".repeat(32);
        let old_alive = Arc::new(AtomicBool::new(true));
        let mut old_state = state_for(3);
        old_state.instance_id = instance_id.clone();
        old_state.engine_generation = 3;
        let stale_runtime = ManagedRuntime {
            state: old_state,
            session_key: ManagedSessionKey::new([3; 32]),
            endpoint_path: "stale".into(),
            supervisor_started_at: 3,
            engine_started_at: 3,
            alive: old_alive,
        };
        let replacement_alive = Arc::new(AtomicBool::new(true));
        let mut replacement_state = state_for(4);
        replacement_state.instance_id = instance_id.clone();
        replacement_state.engine_generation = 4;
        launcher.managed.lock().unwrap().insert(
            instance_id.clone(),
            ManagedRuntime {
                state: replacement_state,
                session_key: ManagedSessionKey::new([4; 32]),
                endpoint_path: "replacement".into(),
                supervisor_started_at: 4,
                engine_started_at: 4,
                alive: replacement_alive.clone(),
            },
        );

        launcher.cleanup_failed_validation(&stale_runtime);

        let managed = launcher.managed.lock().unwrap();
        let retained = managed.get(&instance_id).expect("replacement retained");
        assert_eq!(retained.state.engine_generation, 4);
        assert!(Arc::ptr_eq(&retained.alive, &replacement_alive));
    }

    #[test]
    fn cancellation_after_ready_leaves_no_managed_runtime_commit() {
        let launcher = RealSupervisorLauncher::default();
        let cancelled = Arc::new(AtomicBool::new(true));
        let runtime = ManagedRuntime {
            state: state_for(5),
            session_key: ManagedSessionKey::new([5; 32]),
            endpoint_path: "ready".into(),
            supervisor_started_at: 5,
            engine_started_at: 5,
            alive: Arc::new(AtomicBool::new(true)),
        };

        let error = launcher
            .commit_managed_runtime(runtime, &cancelled)
            .unwrap_err();

        assert_eq!(error.code, "BROWSER_CANCELLED");
        assert_eq!(launcher.managed.lock().unwrap().len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_reaps_supervisor_during_readiness_without_late_side_effect() {
        let root =
            std::env::temp_dir().join(format!("cys-browser-cancel-child-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let marker = root.join("late-spawn-marker");
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("sleep 0.25; printf late > \"$1\"; sleep 5")
            .arg("cys-browser-cancel-test")
            .arg(&marker)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut child = spawn_supervisor_cancellable(&mut command, &cancelled).unwrap();
        let stdout = child.stdout.take().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout)
                .take(64 * 1024 + 1)
                .read_line(&mut line)
                .map(|count| (count, line));
            let _ = ready_tx.send(result);
        });
        let cancel = cancelled.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            cancel.store(true, Ordering::SeqCst);
        });

        let error = wait_for_supervisor_readiness(
            &mut child,
            ready_rx,
            &cancelled,
            std::time::Duration::from_secs(2),
        )
        .unwrap_err();
        assert_eq!(error.code, "BROWSER_CANCELLED");
        assert!(
            child.try_wait().unwrap().is_some(),
            "cancelled child must be reaped"
        );
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            !marker.exists(),
            "cancelled readiness must prevent late child side effects"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
