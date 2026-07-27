use super::{AuthenticatedEndpointSnapshot, BrokerFailure, LaunchRequest, SupervisorLauncher};
use cys::browser_runtime::{BrokerHello, ParsedRuntimeState, RuntimePaths, RuntimeStateV2};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub(super) struct RealSupervisorLauncher {
    managed: Arc<Mutex<HashMap<String, ManagedRuntime>>>,
}

const MAX_MANAGED_RUNTIMES: usize = 4;
/// 살아있는 런타임을 부하 스파이크 1회로 고아로 만들지 않기 위한 연속 실패 한계.
/// 일시 실패는 이 횟수만큼 연속되어야 비로소 엔트리를 제거한다(성공 시 리셋).
const MAX_CONSECUTIVE_TRANSIENT_VALIDATION_FAILURES: u32 = 3;
/// 핸드셰이크 실패 경로: 엔진이 아직 없어 회수 대상이 없다 → 유예 0(즉시).
const HANDSHAKE_TERMINATION_GRACE: std::time::Duration = std::time::Duration::ZERO;
/// post-readiness 경로: EOF 실효 ≤100ms + supervisor Drop 내부 엔진 유예 2s + 여유.
const POST_READINESS_TERMINATION_GRACE: std::time::Duration = std::time::Duration::from_secs(3);
/// pre-readiness 경로: supervisor의 liveness 스레드는 readiness 이후에야 스폰되므로
/// (`supervisor/main.rs:44` 블로킹 → `:60` spawn) EOF가 무효다. 긴 유예는 회수 이득이
/// 0인 채로 취소 응답성만 해치고 자식의 늦은 부작용 창을 넓힌다.
const PRE_READINESS_TERMINATION_GRACE: std::time::Duration =
    std::time::Duration::from_millis(100);
type ManagedSessionKey = zeroize::Zeroizing<[u8; 32]>;

/// `validate_live` 실패의 성격. 일시 실패로 소유권을 파괴하면 supervisor는 계속 살아
/// flock을 점유하는데 세션 키는 메모리 전용이라 재인수가 불가능해 고아가 확정된다.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ValidationSeverity {
    /// 살아있는 정상 런타임에서도 부하·타이밍으로 발생 가능(TCP connect/read 타임아웃·비200).
    Transient,
    /// 런타임 신원 자체가 깨졌다(프로세스 부재·start_time 불일치·MAC 불일치·identity 불일치).
    Fatal,
}

type ValidationError = (BrokerFailure, ValidationSeverity);

fn transient(failure: BrokerFailure) -> ValidationError {
    (failure, ValidationSeverity::Transient)
}

fn fatal(failure: BrokerFailure) -> ValidationError {
    (failure, ValidationSeverity::Fatal)
}

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
    /// 연속 일시 실패 카운터(성공 시 0으로 리셋).
    transient_failures: Arc<AtomicU32>,
    /// evict 시 종료 계약용 control 파이프. 워처 스레드와 공유 보유하며,
    /// `take()` → drop(EOF)로 supervisor를 정상 종료 경로에 태운다.
    control: Arc<Mutex<Option<ChildStdin>>>,
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
    ) -> bool {
        remove_managed_incarnation(&self.managed, instance_id, engine_generation, alive)
    }

    /// 엔트리를 잊는 유일한 경로. 잊은 런타임을 살려두면 전역 flock을 영구 독점하므로
    /// 제거와 종료는 하나의 계약이다(§2-A evict 종료 계약).
    fn evict_managed_runtime(&self, runtime: &ManagedRuntime, reason: &str) {
        let removed = self.remove_managed_incarnation(
            &runtime.state.instance_id,
            runtime.state.engine_generation,
            &runtime.alive,
        );
        if removed {
            terminate_evicted_runtime(runtime, reason);
        }
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
) -> bool {
    let mut managed = managed.lock().unwrap();
    let remove = managed.get(instance_id).is_some_and(|runtime| {
        runtime.state.engine_generation == engine_generation && Arc::ptr_eq(&runtime.alive, alive)
    });
    if remove {
        managed.remove(instance_id);
    }
    remove
}

/// evict된 런타임을 종료 경로에 태운다: control 파이프를 닫으면(EOF) supervisor의
/// liveness 스레드(`supervisor/main.rs:60-70`)가 ≤100ms에 감지해 Drop을 완주시킨다
/// (엔진 그룹 회수·state.json 제거·flock 해제).
///
/// X2(reap 금지) 비위반: 이것은 타 데몬이 쥔 홀더가 아니라 **우리가 세션 키를 발급하고
/// control 파이프를 소유한 자기 자식**이다. 소유권 토큰 부재 문제가 성립하지 않으므로
/// X2의 적용 대상이 아니다.
///
/// 무음 종료 금지 계약: 종료 시 반드시 1줄을 남긴다.
fn terminate_evicted_runtime(runtime: &ManagedRuntime, reason: &str) {
    let closed = runtime.control.lock().unwrap().take().is_some();
    eprintln!(
        "cys-browserd: evicted managed runtime instance={} generation={} reason={reason} control_closed={closed}",
        runtime.state.instance_id, runtime.state.engine_generation
    );
}

/// 일시/치명 분리 판정. 일시 실패는 연속 카운터를 올리고 한계 도달 시에만 evict를 지시한다.
fn should_evict_after_validation_failure(
    runtime: &ManagedRuntime,
    severity: ValidationSeverity,
) -> bool {
    match severity {
        ValidationSeverity::Fatal => true,
        ValidationSeverity::Transient => {
            let consecutive = runtime.transient_failures.fetch_add(1, Ordering::SeqCst) + 1;
            consecutive >= MAX_CONSECUTIVE_TRANSIENT_VALIDATION_FAILURES
        }
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
        // identity 실패 경로도 반드시 자식을 회수한다(회수 누락 시 defunct 잔존).
        let supervisor_started_at = match process_start_time(state.supervisor_pid) {
            Some(started_at) => started_at,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BrokerFailure::new(
                    "RUNTIME_START_FAILED",
                    "supervisor process identity unavailable",
                ));
            }
        };
        let engine_started_at = match process_start_time(state.engine_pid) {
            Some(started_at) => started_at,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BrokerFailure::new(
                    "RUNTIME_START_FAILED",
                    "engine process identity unavailable",
                ));
            }
        };
        if request.cancellation.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(BrokerFailure::new(
                "BROWSER_CANCELLED",
                "Browser launch was cancelled after readiness",
            ));
        }
        let alive = Arc::new(AtomicBool::new(true));
        // control 파이프는 이제 워처 스레드와 managed 엔트리가 **함께** 보유한다.
        // 평시에는 워처가 붙들어 supervisor를 살려두고, evict 시에만 take()로 닫는다.
        let control = Arc::new(Mutex::new(Some(control)));
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
            transient_failures: Arc::new(AtomicU32::new(0)),
            control: control.clone(),
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
            // 워처가 Arc를 함께 보유해 성공 커밋 후 control 파이프 수명을 유지한다.
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
            self.evict_managed_runtime(&runtime, "managed runtime exited");
            return Err(BrokerFailure::new(
                "ENGINE_EXITED",
                "managed runtime exited",
            ));
        }
        if process_start_time(state.supervisor_pid) != Some(runtime.supervisor_started_at)
            || process_start_time(state.engine_pid) != Some(runtime.engine_started_at)
        {
            self.evict_managed_runtime(&runtime, "pid incarnation changed");
            return Err(BrokerFailure::new(
                "PROCESS_IDENTITY_MISMATCH",
                "managed runtime PID incarnation changed",
            ));
        }
        let validated = (|| -> Result<AuthenticatedEndpointSnapshot, ValidationError> {
            let input = std::fs::read(&runtime.endpoint_path).map_err(|error| {
                fatal(BrokerFailure::new(
                    "ENGINE_ENDPOINT_UNAVAILABLE",
                    format!("private engine endpoint unavailable: {error}"),
                ))
            })?;
            let endpoint: AuthenticatedEngineEndpoint =
                serde_json::from_slice(&input).map_err(|error| {
                    fatal(BrokerFailure::new(
                        "RUNTIME_INTEGRITY_FAILED",
                        format!("invalid authenticated engine endpoint: {error}"),
                    ))
                })?;
            verify_engine_endpoint(state, &runtime, &endpoint).map_err(fatal)?;
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
        match validated {
            Ok(snapshot) => {
                runtime.transient_failures.store(0, Ordering::SeqCst);
                Ok(snapshot)
            }
            Err((failure, severity)) => {
                if should_evict_after_validation_failure(&runtime, severity) {
                    self.evict_managed_runtime(&runtime, failure.code.as_str());
                }
                Err(failure)
            }
        }
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
            self.evict_managed_runtime(&runtime, "stale runtime on private request");
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

/// 실패·취소 경로 전용 종료기. `drop(control)`로 EOF를 유도해 supervisor가 스스로
/// Drop을 완주하게 하고(엔진 그룹·state.json 회수·flock 해제), grace 만료 시에만
/// SIGKILL로 격상한다. grace는 경로별로 다르다(위 3개 상수).
///
/// X2(reap 금지) 비위반: 대상은 우리가 방금 스폰하고 control 파이프를 소유한 자기
/// 자식이며, 타 데몬이 쥔 홀더가 아니다.
fn terminate_supervisor(
    child: &mut std::process::Child,
    control: &mut Option<ChildStdin>,
    grace: std::time::Duration,
) -> Option<std::process::ExitStatus> {
    drop(control.take());
    let deadline = std::time::Instant::now() + grace;
    while std::time::Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = child.kill();
    child.wait().ok()
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

/// 반환하는 실패의 성격 구분(§2-A): TCP connect/write/read·응답 형식·비200은 **일시**
/// (부하 스파이크로 살아있는 런타임에서도 발생), 신원 불일치만 **치명**이다.
fn authenticated_health(endpoint: &AuthenticatedEngineEndpoint) -> Result<(), ValidationError> {
    use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
    use std::time::Duration;
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, endpoint.port);
    let mut stream = TcpStream::connect_timeout(&address.into(), Duration::from_millis(500))
        .map_err(|error| {
            transient(BrokerFailure::new(
                "ENGINE_EXITED",
                format!("engine health connect: {error}"),
            ))
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
    .map_err(|error| {
        transient(BrokerFailure::new(
            "ENGINE_EXITED",
            format!("engine health write: {error}"),
        ))
    })?;
    let mut response = Vec::new();
    stream
        .take(1024 * 1024)
        .read_to_end(&mut response)
        .map_err(|error| {
            transient(BrokerFailure::new(
                "ENGINE_EXITED",
                format!("engine health read: {error}"),
            ))
        })?;
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            transient(BrokerFailure::new(
                "ENGINE_EXITED",
                "engine health returned an invalid HTTP response",
            ))
        })?;
    if !response.starts_with(b"HTTP/1.1 200 ") && !response.starts_with(b"HTTP/1.0 200 ") {
        return Err(transient(BrokerFailure::new(
            "ENGINE_EXITED",
            "engine health returned non-200",
        )));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&response[split + 4..]).map_err(|error| {
            transient(BrokerFailure::new(
                "ENGINE_EXITED",
                format!("engine health JSON: {error}"),
            ))
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
        return Err(fatal(BrokerFailure::new(
            "PROCESS_IDENTITY_MISMATCH",
            "engine health identity mismatch",
        )));
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

    fn test_runtime(
        state: &RuntimeStateV2,
        endpoint_path: std::path::PathBuf,
        session_key: [u8; 32],
        started_at: u64,
        control: Option<ChildStdin>,
    ) -> ManagedRuntime {
        ManagedRuntime {
            state: state.clone(),
            session_key: ManagedSessionKey::new(session_key),
            endpoint_path,
            supervisor_started_at: started_at,
            engine_started_at: started_at,
            alive: Arc::new(AtomicBool::new(true)),
            transient_failures: Arc::new(AtomicU32::new(0)),
            control: Arc::new(Mutex::new(control)),
        }
    }

    /// 실제 엔진처럼 대답하는 스크립트 서버. `script`의 각 원소가 한 요청의 성패다.
    fn spawn_scripted_health_server(
        script: Vec<bool>,
        pid: u32,
        runtime_id: String,
        started_at: u64,
    ) -> (u16, std::thread::JoinHandle<()>) {
        use std::net::{Ipv4Addr, TcpListener};
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("health listener");
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            for healthy in script {
                let (mut stream, _) = listener.accept().expect("health accept");
                // 요청을 끝까지 소비해야 close 시 RST가 나지 않는다(미소비 수신버퍼 → reset).
                let mut request = Vec::new();
                let mut scratch = [0_u8; 2048];
                loop {
                    match stream.read(&mut scratch) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => request.extend_from_slice(&scratch[..count]),
                    }
                    if let Some(split) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..split]).to_lowercase();
                        let length = head
                            .split("content-length:")
                            .nth(1)
                            .and_then(|rest| rest.split("\r\n").next())
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if request.len() >= split + 4 + length {
                            break;
                        }
                    }
                }
                let response = if healthy {
                    let body = format!(
                        "{{\"result\":{{\"pid\":{pid},\"runtime_id\":\"{runtime_id}\",\"process_start_time\":{started_at}}}}}"
                    );
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else {
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
                        .to_string()
                };
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (port, handle)
    }

    fn signed_endpoint_json(
        session_key: &[u8; 32],
        state: &RuntimeStateV2,
        started_at: u64,
    ) -> String {
        let token = "a".repeat(32);
        let cast_token = "b".repeat(32);
        let payload = serde_json::to_vec(&json!([
            2,
            state.engine_pid,
            state.port,
            token,
            cast_token,
            state.runtime_id,
            started_at,
            true,
            state.instance_id,
            state.engine_generation,
        ]))
        .unwrap();
        let mut signer = Hmac::<Sha256>::new_from_slice(session_key).unwrap();
        signer.update(b"cys.browser.engine-state.v1\0");
        signer.update(&payload);
        let state_mac: String = signer
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        serde_json::to_string(&json!({
            "schema_version": 2,
            "pid": state.engine_pid,
            "port": state.port,
            "token": token,
            "cast_token": cast_token,
            "runtime_id": state.runtime_id,
            "process_start_time": started_at,
            "headless": true,
            "instance_id": state.instance_id,
            "engine_generation": state.engine_generation,
            "state_mac": state_mac,
        }))
        .unwrap()
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cys-ws0-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn transient_failure_count(launcher: &RealSupervisorLauncher, instance_id: &str) -> u32 {
        launcher
            .managed
            .lock()
            .unwrap()
            .get(instance_id)
            .expect("runtime retained")
            .transient_failures
            .load(Ordering::SeqCst)
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
                    transient_failures: Arc::new(AtomicU32::new(0)),
                    control: Arc::new(Mutex::new(None)),
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
                transient_failures: Arc::new(AtomicU32::new(0)),
                control: Arc::new(Mutex::new(None)),
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
                transient_failures: Arc::new(AtomicU32::new(0)),
                control: Arc::new(Mutex::new(None)),
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
            transient_failures: Arc::new(AtomicU32::new(0)),
            control: Arc::new(Mutex::new(None)),
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
                transient_failures: Arc::new(AtomicU32::new(0)),
                control: Arc::new(Mutex::new(None)),
            },
        );

        launcher.evict_managed_runtime(&stale_runtime, "test");

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
            transient_failures: Arc::new(AtomicU32::new(0)),
            control: Arc::new(Mutex::new(None)),
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

    #[test]
    fn transient_health_failure_does_not_orphan_a_live_runtime() {
        let launcher = RealSupervisorLauncher::default();
        let pid = std::process::id();
        let started_at = process_start_time(pid).expect("self process start time");
        let mut state = state_for(7);
        state.supervisor_pid = pid;
        state.engine_pid = pid;
        let (port, server) = spawn_scripted_health_server(
            vec![false, false, true, false, false, false],
            pid,
            state.runtime_id.clone(),
            started_at,
        );
        state.port = port;
        let session_key = [7_u8; 32];
        let dir = scratch_dir("transient");
        let endpoint_path = dir.join("state.json");
        std::fs::write(
            &endpoint_path,
            signed_endpoint_json(&session_key, &state, started_at),
        )
        .unwrap();
        launcher.managed.lock().unwrap().insert(
            state.instance_id.clone(),
            test_runtime(&state, endpoint_path, session_key, started_at, None),
        );

        for attempt in 1..=2 {
            let error = launcher.validate_live(&state).unwrap_err();
            assert_eq!(error.code, "ENGINE_EXITED");
            assert_eq!(
                launcher.managed.lock().unwrap().len(),
                1,
                "transient failure {attempt} must not destroy ownership of a live runtime"
            );
            assert_eq!(transient_failure_count(&launcher, &state.instance_id), attempt);
        }

        launcher.validate_live(&state).expect("healthy validation");
        assert_eq!(
            transient_failure_count(&launcher, &state.instance_id),
            0,
            "a successful validation must reset the consecutive counter"
        );

        for attempt in 1..=2 {
            launcher.validate_live(&state).unwrap_err();
            assert_eq!(
                launcher.managed.lock().unwrap().len(),
                1,
                "post-reset transient failure {attempt} must not evict"
            );
        }
        let error = launcher.validate_live(&state).unwrap_err();
        assert_eq!(error.code, "ENGINE_EXITED");
        assert!(
            launcher.managed.lock().unwrap().is_empty(),
            "the third consecutive transient failure evicts"
        );

        server.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fatal_validation_failure_evicts_on_the_first_occurrence() {
        let launcher = RealSupervisorLauncher::default();
        let pid = std::process::id();
        let started_at = process_start_time(pid).expect("self process start time");
        let mut state = state_for(8);
        state.supervisor_pid = pid;
        state.engine_pid = pid;
        let dir = scratch_dir("fatal");
        launcher.managed.lock().unwrap().insert(
            state.instance_id.clone(),
            test_runtime(&state, dir.join("absent.json"), [8_u8; 32], started_at, None),
        );

        let error = launcher.validate_live(&state).unwrap_err();

        assert_eq!(error.code, "ENGINE_ENDPOINT_UNAVAILABLE");
        assert!(launcher.managed.lock().unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn identity_mismatch_evicts_without_counting_transient_failures() {
        let launcher = RealSupervisorLauncher::default();
        let mut state = state_for(10);
        state.supervisor_pid = std::process::id();
        state.engine_pid = std::process::id();
        let dir = scratch_dir("identity");
        // 기록된 start_time과 실제 프로세스의 start_time이 다르면 치명이다.
        launcher.managed.lock().unwrap().insert(
            state.instance_id.clone(),
            test_runtime(&state, dir.join("state.json"), [10_u8; 32], 1, None),
        );

        let error = launcher.validate_live(&state).unwrap_err();

        assert_eq!(error.code, "PROCESS_IDENTITY_MISMATCH");
        assert!(launcher.managed.lock().unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn eviction_closes_the_control_pipe_so_the_supervisor_can_exit() {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("read line; exit 0")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("fake supervisor");
        let control = child.stdin.take().expect("control pipe");
        let launcher = RealSupervisorLauncher::default();
        let state = state_for(9);
        let runtime = test_runtime(&state, "unused".into(), [9_u8; 32], 1, Some(control));
        launcher
            .managed
            .lock()
            .unwrap()
            .insert(state.instance_id.clone(), runtime.clone());

        launcher.evict_managed_runtime(&runtime, "test eviction");

        assert!(launcher.managed.lock().unwrap().is_empty());
        assert!(
            runtime.control.lock().unwrap().is_none(),
            "evict must take the control pipe"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "an evicted runtime must be terminated, not orphaned"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn eviction_never_terminates_a_replacement_incarnation() {
        let launcher = RealSupervisorLauncher::default();
        let instance_id = "d".repeat(32);
        let mut stale_state = state_for(11);
        stale_state.instance_id = instance_id.clone();
        stale_state.engine_generation = 11;
        let stale = test_runtime(&stale_state, "stale".into(), [11_u8; 32], 1, None);
        *stale.control.lock().unwrap() = None;
        let mut replacement_state = state_for(12);
        replacement_state.instance_id = instance_id.clone();
        replacement_state.engine_generation = 12;
        let replacement = test_runtime(&replacement_state, "new".into(), [12_u8; 32], 1, None);
        launcher
            .managed
            .lock()
            .unwrap()
            .insert(instance_id.clone(), replacement.clone());

        launcher.evict_managed_runtime(&stale, "stale");

        assert_eq!(launcher.managed.lock().unwrap().len(), 1);
        assert!(
            !replacement.control.lock().unwrap().is_some(),
            "control was None to begin with"
        );
        assert_eq!(
            launcher
                .managed
                .lock()
                .unwrap()
                .get(&instance_id)
                .unwrap()
                .state
                .engine_generation,
            12
        );
    }
}
