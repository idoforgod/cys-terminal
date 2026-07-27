use super::{AuthenticatedEndpointSnapshot, BrokerFailure, LaunchRequest, SupervisorLauncher};
use cys::browser_runtime::{BrokerHello, ParsedRuntimeState, RuntimePaths, RuntimeStateV2};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{ChildStderr, ChildStdin, Command, ExitStatus, Stdio};
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
    /// 살아있는 정상 런타임에서도 부하·타이밍으로 발생 가능(TCP connect/read 타임아웃·비200
    /// **+ endpoint 파일의 일시적 read I/O 오류** — fd 고갈·EIO·NFS 지연은 내용 손상이 아니다).
    Transient,
    /// 런타임 신원 자체가 깨졌다(프로세스 부재·start_time 불일치·MAC 불일치·identity 불일치
    /// **+ endpoint 파일 부재(NotFound)·JSON 파싱 실패 + health connect의 ConnectionRefused**
    /// — 모두 재시도로 회복되지 않는다).
    Fatal,
}

/// health TCP connect 실패의 성격 판정(F-2).
///
/// `ConnectionRefused` = "그 포트에 리스너가 없다" → 파일 I/O의 `NotFound`와 동형인 **회복 불가**
/// 신호다(부하가 아무리 심해도 살아있는 엔진이 자기 포트를 refuse하지 않는다). 나머지
/// (timeout·reset·unreachable·interrupted 등)는 살아있는 런타임에서도 부하 스파이크로 발생하므로
/// 종전대로 일시다 — `endpoint_read_io_error_is_transient_except_not_found`와 정확히 대칭이다.
fn health_connect_severity(kind: std::io::ErrorKind) -> ValidationSeverity {
    if kind == std::io::ErrorKind::ConnectionRefused {
        ValidationSeverity::Fatal
    } else {
        ValidationSeverity::Transient
    }
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
    /// post-readiness 사망 사인 채집기(워처 스레드와 공유).
    stderr: Arc<StderrTap>,
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
    /// 제거와 종료 **요청**은 하나의 계약이다(§2-A evict 종료 계약). 단 종료는
    /// best-effort이며 EOF 무시 supervisor는 잔존한다 — `terminate_evicted_runtime` 주석의
    /// 잔존물 계약을 볼 것.
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
/// ★**evict 종료는 best-effort다 — EOF를 무시하는 supervisor는 잔존한다**(적대 검증 4-2).
/// 이 함수는 control 파이프만 닫고 끝나며 `try_wait` 폴링도 SIGKILL 격상도 하지 않는다.
/// `terminate_supervisor`가 하는 격상을 여기서 하지 않는 이유는 **구조적 불가가 아니라
/// 비용·복잡도 때문의 보류**다(ADV-1 정정 — 종전 주석은 "구조적으로 불가"라고 단정했으나
/// 틀렸다): 성공 커밋 후 `Child` 핸들은 워처 스레드(`launch()`의 `child.wait()` 루프)가 단독
/// 소유하지만, SIGTERM/SIGKILL은 `Child`가 아니라 **pid만** 있으면 보낼 수 있고(`libc::kill`),
/// pid 재사용 오살을 막을 재료(`state.supervisor_pid` ↔ `supervisor_started_at`)는 같은 파일의
/// `validate_live`가 이미 쓰고 있다. 하려면 워처와의 종료 경합·grace 타이머·플랫폼 분기를
/// 감당해야 해서 리스(WS-11)와 함께 다루기로 미룬 것이다. 현재 evict는 여전히 "종료 요청"이지
/// "종료 보장"이 아니다 — 아래 잔존물 계약이 그 대가다.
///
/// **잔존물 계약**(§4-3의 pre-readiness 잔존물 계약과 같은 등급의 인정된 한계):
/// EOF를 무시하거나 wedge된 supervisor는 flock을 쥔 채 장부에서만 사라진다 = 잔여 고아.
/// 회수는 ①supervisor 자신의 liveness 스레드(EOF ≤100ms 감지) ②엔진 런처 자기 타임아웃
/// ③프로세스 사망 시 flock 자동 해제 + 다음 ensure의 stale 수렴에 의존한다. 이 창은
/// 리스(WS-11) 도입 전까지 완전 봉합 불가다. 계약을 회귀로 고정한 테스트:
/// `evicting_an_eof_ignoring_supervisor_leaves_it_alive_by_contract`.
///
/// 무음 종료 금지 계약: 종료 시 반드시 1줄을 남긴다.
fn terminate_evicted_runtime(runtime: &ManagedRuntime, reason: &str) {
    let closed = runtime.control.lock().unwrap().take().is_some();
    eprintln!(
        "cys-browserd: evicted managed runtime instance={} generation={} reason={reason} control_closed={closed}",
        runtime.state.instance_id, runtime.state.engine_generation
    );
}

/// supervisor stderr 링버퍼 상한. 다중 라인을 보존해 "필터를 통과하는 마지막 완결 라인"을
/// 뽑을 수 있어야 한다(자기유발 라인 하나에 진짜 사인이 밀려나면 안 된다).
const STDERR_TAP_CAPACITY: usize = 8 * 1024;
/// tap 스레드가 EOF에 도달했음을 기다리는 **유계** 상한. 무한 join은 금지다
/// (ensure 경로의 broker Mutex 안에서 데드락을 만든다).
const STDERR_TAP_EOF_WAIT: std::time::Duration = std::time::Duration::from_millis(200);
/// 우리가 control 파이프를 닫아서 supervisor가 내는 소리는 사인이 아니다.
/// (`supervisor/main.rs:64-66`, `SupervisorError` Display = `{code}: {message}`)
const SELF_INFLICTED_SIGNS: [&str; 4] = [
    "broker liveness pipe closed",
    "broker liveness pipe failed",
    "unexpected command on startup-only broker channel",
    "AUTHORITY_REJECTED: broker liveness",
];

/// supervisor stderr를 논블로킹으로 채집하는 링버퍼. 스레드는 detach이며
/// supervisor stderr의 EOF(= supervisor 사망)로 스스로 끝난다.
struct StderrTap {
    ring: Mutex<VecDeque<u8>>,
    rolled: AtomicBool,
    eof: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl StderrTap {
    fn spawn(stderr: Option<ChildStderr>) -> Arc<Self> {
        let (eof_tx, eof_rx) = std::sync::mpsc::sync_channel(1);
        let tap = Arc::new(Self {
            ring: Mutex::new(VecDeque::new()),
            rolled: AtomicBool::new(false),
            eof: Mutex::new(stderr.as_ref().map(|_| eof_rx)),
        });
        let Some(mut stderr) = stderr else {
            return tap;
        };
        let sink = tap.clone();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 1024];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => sink.push(&buffer[..count]),
                }
            }
            let _ = eof_tx.send(());
        });
        tap
    }

    fn push(&self, bytes: &[u8]) {
        let mut ring = self.ring.lock().unwrap();
        ring.extend(bytes.iter().copied());
        while ring.len() > STDERR_TAP_CAPACITY {
            ring.pop_front();
            self.rolled.store(true, Ordering::SeqCst);
        }
    }

    /// EOF 신호를 **유계**로 기다린다. 데이터가 커널 파이프에 있으나 tap 스레드가 아직
    /// 스케줄되지 않은 창 때문에, 이 대기 없이는 가장 흔한 즉사 실패에서 사인이 확률적으로 빈다.
    fn await_eof(&self, timeout: std::time::Duration) -> bool {
        let mut slot = self.eof.lock().unwrap();
        match slot.as_ref() {
            None => true,
            Some(receiver) => {
                let reached = receiver.recv_timeout(timeout).is_ok();
                if reached {
                    *slot = None;
                }
                reached
            }
        }
    }

    /// "필터를 통과하는 마지막 완결 라인"(filter-then-last). 순서를 뒤집으면
    /// 자기유발 라인이 마지막일 때 진짜 사인까지 함께 소실된다.
    fn last_sign(&self) -> Option<String> {
        let bytes: Vec<u8> = self.ring.lock().unwrap().iter().copied().collect();
        let text = String::from_utf8_lossy(&bytes);
        // 완결 라인만 본다: 마지막 개행 이후의 조각은 아직 끝나지 않은 라인이다.
        let complete = match text.rfind('\n') {
            Some(index) => &text[..index],
            None => return None,
        };
        let mut lines: Vec<&str> = complete.lines().collect();
        if self.rolled.load(Ordering::SeqCst) && !lines.is_empty() {
            // 링이 굴렀다면 첫 라인은 앞이 잘렸을 수 있다.
            lines.remove(0);
        }
        lines
            .into_iter()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !is_self_inflicted_sign(line))
            .map(normalize_sign)
            .next_back()
    }

    /// 사망 사인 채집: EOF 유계 대기 후 추출.
    fn death_sign(&self) -> Option<String> {
        self.await_eof(STDERR_TAP_EOF_WAIT);
        self.last_sign()
    }
}

fn is_self_inflicted_sign(line: &str) -> bool {
    SELF_INFLICTED_SIGNS
        .iter()
        .any(|marker| line.contains(marker))
}

/// `cys-browserd: ` 접두는 **로그 전용**이다 — 배너 message에서는 제거한다(120자 절단 대응).
fn normalize_sign(line: &str) -> String {
    line.strip_prefix("cys-browserd: ")
        .unwrap_or(line)
        .trim()
        .to_string()
}

/// 자기유발 SIGKILL(우리 격상)은 사인이 아니므로 표시하지 않는다.
fn exit_annotation(status: Option<ExitStatus>) -> Option<String> {
    let status = status?;
    if let Some(code) = status.code() {
        return Some(format!("exit {code}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        return match status.signal() {
            Some(9) | None => None,
            Some(signal) => Some(format!("signal {signal}")),
        };
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// 배너는 120자에서 절단되므로 **사인을 선두**에 둔다. 산문 해설은 넣지 않는다.
fn supervisor_death_message(sign: Option<String>, status: Option<ExitStatus>) -> String {
    match (sign, exit_annotation(status)) {
        (Some(sign), Some(exit)) => format!("{sign} ({exit})"),
        (Some(sign), None) => sign,
        (None, Some(exit)) => format!("supervisor left no stderr sign ({exit})"),
        (None, None) => "supervisor left no stderr sign".to_string(),
    }
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
            // 사인 채집: stderr를 버리면 실패 원인이 침묵 속에 소멸한다(진단 블랙홀).
            // `.env_remove("PATH")`는 보안 봉인이므로 불가침이다.
            .stderr(Stdio::piped());
        let mut child = spawn_supervisor_cancellable(&mut command, &request.cancellation)?;
        let stderr_tap = StderrTap::spawn(child.stderr.take());
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
            let mut control = Some(control);
            let status = terminate_supervisor(&mut child, &mut control, HANDSHAKE_TERMINATION_GRACE);
            let message = match stderr_tap.death_sign() {
                Some(sign) => supervisor_death_message(Some(sign), status),
                None => format!("private broker handshake failed: {error}"),
            };
            return Err(BrokerFailure::new("RUNTIME_START_FAILED", message));
        }
        // 핸드셰이크가 끝난 시점부터 control은 종료기·워처가 공유하는 자원이다.
        let mut control = Some(control);
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
            &mut control,
            ready_rx,
            &request.cancellation,
            std::time::Duration::from_secs(25),
        )? {
            (count, line) if count > 0 && count <= 64 * 1024 => {
                match cys::browser_runtime::parse_runtime_state(&line) {
                    Ok(ParsedRuntimeState::V2(state)) => state,
                    Ok(ParsedRuntimeState::LegacyIncompatible(_)) => {
                        terminate_supervisor(&mut child, &mut control, POST_READINESS_TERMINATION_GRACE);
                        return Err(BrokerFailure::new(
                            "PROTOCOL_MISMATCH",
                            "supervisor returned legacy state",
                        ));
                    }
                    Err(error) => {
                        terminate_supervisor(&mut child, &mut control, POST_READINESS_TERMINATION_GRACE);
                        return Err(runtime_failure(error));
                    }
                }
            }
            (0, _) => {
                // supervisor가 readiness 프레임 없이 죽었다 — 가장 흔한 즉사 실패다.
                let status =
                    terminate_supervisor(&mut child, &mut control, PRE_READINESS_TERMINATION_GRACE);
                return Err(BrokerFailure::new(
                    "SUPERVISOR_EXIT_PRE_READY",
                    supervisor_death_message(stderr_tap.death_sign(), status),
                ));
            }
            _ => {
                terminate_supervisor(&mut child, &mut control, PRE_READINESS_TERMINATION_GRACE);
                return Err(BrokerFailure::new(
                    // §5-0-A-1 코드 길이 상한 26자 — `SUPERVISOR_READINESS_OVERFLOW`(29자)에서 개명.
                    "SUPERVISOR_READY_OVERFLOW",
                    "readiness frame exceeded 64 KiB",
                ));
            }
        };
        // identity 실패 경로도 반드시 자식을 회수한다(회수 누락 시 defunct 잔존).
        let supervisor_started_at = match process_start_time(state.supervisor_pid) {
            Some(started_at) => started_at,
            None => {
                terminate_supervisor(&mut child, &mut control, POST_READINESS_TERMINATION_GRACE);
                return Err(BrokerFailure::new(
                    "RUNTIME_START_FAILED",
                    "supervisor process identity unavailable",
                ));
            }
        };
        let engine_started_at = match process_start_time(state.engine_pid) {
            Some(started_at) => started_at,
            None => {
                terminate_supervisor(&mut child, &mut control, POST_READINESS_TERMINATION_GRACE);
                return Err(BrokerFailure::new(
                    "RUNTIME_START_FAILED",
                    "engine process identity unavailable",
                ));
            }
        };
        if request.cancellation.load(Ordering::SeqCst) {
            terminate_supervisor(&mut child, &mut control, POST_READINESS_TERMINATION_GRACE);
            return Err(BrokerFailure::new(
                "BROWSER_CANCELLED",
                "Browser launch was cancelled after readiness",
            ));
        }
        let alive = Arc::new(AtomicBool::new(true));
        // control 파이프는 이제 워처 스레드와 managed 엔트리가 **함께** 보유한다.
        // 평시에는 워처가 붙들어 supervisor를 살려두고, evict 시에만 take()로 닫는다.
        let control = Arc::new(Mutex::new(control));
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
            stderr: stderr_tap.clone(),
        };
        if let Err(error) = self.commit_managed_runtime(managed_runtime, &request.cancellation) {
            let mut owned = control.lock().unwrap().take();
            terminate_supervisor(&mut child, &mut owned, POST_READINESS_TERMINATION_GRACE);
            return Err(error);
        }
        let managed = self.managed.clone();
        let instance_id = state.instance_id.clone();
        let engine_generation = state.engine_generation;
        let watcher_alive = alive.clone();
        std::thread::spawn(move || {
            // 워처가 Arc를 함께 보유해 성공 커밋 후 control 파이프 수명을 유지한다.
            let _control_liveness = control;
            let status = child.wait().ok();
            watcher_alive.store(false, Ordering::SeqCst);
            // post-readiness 사망의 사인 소비처 — 이게 없으면 정상 운영에서 더 흔한
            // 엔진 크래시류의 원인이 여전히 0이다.
            eprintln!(
                "cys-browserd: managed runtime exited instance={instance_id} generation={engine_generation} {}",
                supervisor_death_message(stderr_tap.death_sign(), status)
            );
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
            // 죽은 런타임의 사인을 호출자에게도 전달한다(사인 우선 형식).
            return Err(BrokerFailure::new(
                "ENGINE_EXITED",
                match runtime.stderr.last_sign() {
                    Some(sign) => sign,
                    None => "managed runtime exited".to_string(),
                },
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
                let failure = BrokerFailure::new(
                    "ENGINE_ENDPOINT_FAILED",
                    format!("private engine endpoint unavailable: {error}"),
                );
                // ★적대 검증 4-3: 파일 I/O 실패를 일괄 치명으로 두면 fd 고갈·순간 EIO·
                // NFS 지연 **1회**가 살아있는 브라우저의 소유권을 파괴한다 — WS-0이 막으려던
                // 결함(health 타임아웃 1회 = 고아 확정)과 정확히 동형이다. 회복 불가가
                // 확정된 `NotFound`(엔진이 endpoint를 지웠거나 쓰지 못했다)만 치명이고,
                // 나머지 io 오류는 일시로 분류해 연속 3회 한계에 맡긴다.
                if error.kind() == std::io::ErrorKind::NotFound {
                    fatal(failure)
                } else {
                    transient(failure)
                }
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

    /// ★ADV-1: 소유 장부(`managed`)에 **같은 세대의 그 인스턴스**가 아직 있는가.
    /// evict(치명 1회·일시 연속 3회)가 일어났으면 엔트리가 사라져 false가 되고, 브로커는
    /// 그때만 `inner.live`를 버린다 — 1·2회째 일시 실패에서는 세션이 살아남아야 다음 호출이
    /// 발생하고 3스트라이크가 실제로 셈해진다(세대 대조로 교체된 신규 런타임 오인식 차단).
    fn retains_runtime(&self, state: &RuntimeStateV2) -> bool {
        self.managed
            .lock()
            .unwrap()
            .get(&state.instance_id)
            .is_some_and(|runtime| runtime.state.engine_generation == state.engine_generation)
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

/// §5-0-A-1 계약: 에러 코드는 **26자 이내**여야 배너
/// `BROWSER_DISABLED_SAFE [<CODE>]: <message>`가 120자 절단(`ui/src/webpane.ts:386`) 안에서
/// 사유를 보존한다. 소스 텍스트를 직접 스캔하는 트립와이어로, 두 모듈의 테스트가 공유한다.
///
/// 판정 규칙: `"` 바로 뒤부터 `[A-Z0-9_]`가 이어지고 곧바로 `"`로 닫히는 리터럴만 후보다
/// (따옴표 parity에 의존하지 않아 `\"` 이스케이프가 섞여도 어긋나지 않는다). Rust 상수 이름
/// 같은 비-리터럴 식별자는 애초에 후보가 아니며, 에러 코드가 아닌 것으로 확인된 리터럴은
/// 아래 명시 제외 목록에 둔다.
#[cfg(test)]
pub(super) fn over_long_screaming_case_literals(source: &str) -> Vec<&str> {
    const MAX_ERROR_CODE_LEN: usize = 26;
    /// 에러 코드가 아님이 확인된 SCREAMING_SNAKE 리터럴(컴파일 타임 env 이름).
    const NOT_ERROR_CODES: [&str; 1] = ["CYS_BROWSER_V2_RELEASE_QUALIFIED"];
    let bytes = source.as_bytes();
    let mut offenders: Vec<&str> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_uppercase()
                || bytes[end] == b'_'
                || bytes[end].is_ascii_digit())
        {
            end += 1;
        }
        if end > start && end < bytes.len() && bytes[end] == b'"' {
            let token = &source[start..end];
            if token.contains('_')
                && token.len() > MAX_ERROR_CODE_LEN
                && !NOT_ERROR_CODES.contains(&token)
                && !offenders.contains(&token)
            {
                offenders.push(token);
            }
            index = end + 1;
        } else {
            index += 1;
        }
    }
    offenders
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
        // 아직 아무도 stdin을 가져가지 않았다 — 여기서 꺼내 닫아야 EOF가 전달된다.
        let mut control = child.stdin.take();
        terminate_supervisor(&mut child, &mut control, PRE_READINESS_TERMINATION_GRACE);
        return Err(BrokerFailure::new(
            "BROWSER_CANCELLED",
            "Browser launch was cancelled during process creation",
        ));
    }
    Ok(child)
}

/// pre-readiness 구간 전용 대기 루프. 모든 실패 경로는 `terminate_supervisor`로
/// 통일하며, 이 구간의 grace는 `PRE_READINESS_TERMINATION_GRACE`다(EOF 무효 구간이라
/// 긴 유예는 취소 응답성만 해친다 — 취소 회귀 테스트의 계약이기도 하다).
fn wait_for_supervisor_readiness(
    child: &mut std::process::Child,
    control: &mut Option<ChildStdin>,
    receiver: std::sync::mpsc::Receiver<std::io::Result<(usize, String)>>,
    cancelled: &Arc<AtomicBool>,
    timeout: std::time::Duration,
) -> Result<(usize, String), BrokerFailure> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            terminate_supervisor(child, control, PRE_READINESS_TERMINATION_GRACE);
            return Err(BrokerFailure::new(
                "BROWSER_CANCELLED",
                "Browser launch was cancelled while waiting for readiness",
            ));
        }
        if std::time::Instant::now() >= deadline {
            terminate_supervisor(child, control, PRE_READINESS_TERMINATION_GRACE);
            return Err(BrokerFailure::new(
                // §5-0-A-1 코드 길이 상한 26자 — `SUPERVISOR_READINESS_TIMEOUT`(28자)에서 개명.
                "SUPERVISOR_READY_TIMEOUT",
                "readiness timed out",
            ));
        }
        match receiver.recv_timeout(std::time::Duration::from_millis(20)) {
            Ok(Ok(frame)) => return Ok(frame),
            Ok(Err(error)) => {
                terminate_supervisor(child, control, PRE_READINESS_TERMINATION_GRACE);
                return Err(BrokerFailure::new(
                    "RUNTIME_START_FAILED",
                    format!("supervisor readiness read failed: {error}"),
                ));
            }
            // try_wait 기반 "exited before readiness" 분기는 삭제했다(부활 금지):
            // 자식이 죽으면 stdout EOF로 read_line이 0을 반환해 (0, _) 분기가 받는다.
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                terminate_supervisor(child, control, PRE_READINESS_TERMINATION_GRACE);
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

/// 반환하는 실패의 성격 구분(§2-A): TCP write/read·응답 형식·비200과 timeout·reset·unreachable
/// 계열 connect 실패는 **일시**(부하 스파이크로 살아있는 런타임에서도 발생), 신원 불일치와
/// `ConnectionRefused`만 **치명**이다.
///
/// ★F-2: `ConnectionRefused`는 "그 포트에 리스너가 없다"는 **회복 불가 신호**로, 파일 I/O의
/// `NotFound`(위 endpoint 읽기에서 유일하게 치명으로 좁힌 그것)와 정확히 동형이다 — 부하가
/// 아무리 심해도 살아있는 엔진이 자기 포트 연결을 refuse하지는 않는다. 이걸 일시로 두면
/// supervisor 생존 + engine 단독 사망 시 evict에 `validate_live` 3회가 필요한데, 이 함수는
/// **사용자 행동(probe·ensure)으로만 구동**되므로 회복이 2클릭에서 4클릭으로 늘어난다.
/// 대칭 근거: `endpoint_read_io_error_is_transient_except_not_found`(파일 I/O 쪽 같은 논리).
fn authenticated_health(endpoint: &AuthenticatedEngineEndpoint) -> Result<(), ValidationError> {
    use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
    use std::time::Duration;
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, endpoint.port);
    let mut stream = TcpStream::connect_timeout(&address.into(), Duration::from_millis(500))
        .map_err(|error| {
            (
                BrokerFailure::new("ENGINE_EXITED", format!("engine health connect: {error}")),
                health_connect_severity(error.kind()),
            )
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
            stderr: StderrTap::spawn(None),
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
                    stderr: StderrTap::spawn(None),
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
                stderr: StderrTap::spawn(None),
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
                stderr: StderrTap::spawn(None),
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
            stderr: StderrTap::spawn(None),
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
                stderr: StderrTap::spawn(None),
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
            stderr: StderrTap::spawn(None),
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
            &mut None,
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

    /// ⚠범위 고지(ADV-1): 이 테스트는 `validate_live`를 **직접** 연속 호출한다 — 3스트라이크
    /// 산술만 검증하며, 프로덕션 호출자가 그 연속 호출을 실제로 만들어내는지는 증명하지 않는다
    /// (종전에는 첫 실패에 `inner.live`가 비워져 두 번째 호출 자체가 없었고, 이 테스트가 그
    /// 사실을 은폐했다). 프로덕션 규약은
    /// `authority_broker::tests::three_strike_eviction_is_reachable_through_the_production_probe_path`
    /// 가 브로커 공개 API로 고정한다 — 둘은 한 쌍이며 따로 지우지 말 것.
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

    /// endpoint 파일 **부재**(`NotFound`)는 회복 불가이므로 첫 실패에 축출한다.
    /// ★적대 검증 4-3 이후 이 테스트의 의미는 "모든 파일 I/O 오류가 치명"이 아니라
    /// "**NotFound만** 치명"으로 좁혀졌다 — 짝이 되는 음성 테스트는
    /// `endpoint_read_io_error_is_transient_and_keeps_a_live_runtime`이다.
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

        assert_eq!(error.code, "ENGINE_ENDPOINT_FAILED");
        assert!(launcher.managed.lock().unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// ★적대 검증 4-3: endpoint read의 **일시적** I/O 오류(fd 고갈·순간 EIO·NFS 지연) 1회로
    /// 살아있는 브라우저의 소유권이 파괴되면 안 된다 — WS-0이 막으려던 결함과 동형이다.
    /// 픽스처는 endpoint_path를 디렉터리로 만들어 `NotFound`가 아닌 io 오류를 결정론적으로 낸다.
    /// ⚠위 `transient_health_failure_does_not_orphan_a_live_runtime`과 같은 범위 고지(ADV-1):
    /// 직접 3회 호출은 산술 검증이고, 프로덕션 경로 성립은 브로커측 3스트라이크 테스트가 맡는다.
    #[test]
    fn endpoint_read_io_error_is_transient_and_keeps_a_live_runtime() {
        let launcher = RealSupervisorLauncher::default();
        let pid = std::process::id();
        let started_at = process_start_time(pid).expect("self process start time");
        let mut state = state_for(13);
        state.supervisor_pid = pid;
        state.engine_pid = pid;
        let dir = scratch_dir("endpoint-io");
        // 디렉터리를 읽으면 NotFound가 아닌 io 오류(EISDIR)가 난다 — "내용 손상"이 아니다.
        let endpoint_path = dir.join("endpoint-as-a-directory");
        std::fs::create_dir_all(&endpoint_path).unwrap();
        assert_ne!(
            std::fs::read(&endpoint_path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound,
            "픽스처 전제: 이 오류는 NotFound가 아니어야 한다"
        );
        launcher.managed.lock().unwrap().insert(
            state.instance_id.clone(),
            test_runtime(&state, endpoint_path, [13_u8; 32], started_at, None),
        );

        for attempt in 1..=(MAX_CONSECUTIVE_TRANSIENT_VALIDATION_FAILURES - 1) {
            let error = launcher.validate_live(&state).unwrap_err();
            assert_eq!(error.code, "ENGINE_ENDPOINT_FAILED");
            assert_eq!(
                launcher.managed.lock().unwrap().len(),
                1,
                "endpoint read I/O 오류 {attempt}회로는 살아있는 런타임을 축출하지 않는다"
            );
            assert_eq!(
                transient_failure_count(&launcher, &state.instance_id),
                attempt,
                "일시 실패로 계수되어야 한다(치명이면 카운터가 남지 않는다)"
            );
        }

        // 한계까지 연속되면 비로소 축출된다 — 일시 분류가 "무한 관용"은 아니다.
        launcher.validate_live(&state).unwrap_err();
        assert!(launcher.managed.lock().unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// ★F-2 회귀 핀: engine health의 `ConnectionRefused`("그 포트에 리스너가 없다")는 **치명**이다.
    /// supervisor 생존 + engine 단독 사망일 때 종전에는 evict에 `validate_live` 3회가 필요했는데,
    /// 이 함수는 사용자 행동(probe·ensure)으로만 구동되므로 회복이 2클릭에서 4클릭으로 늘어났다.
    /// 픽스처는 bind 직후 닫은 포트로 ECONNREFUSED를 결정론적으로 낸다.
    #[test]
    fn engine_connection_refused_is_fatal_on_the_first_occurrence() {
        use std::net::{Ipv4Addr, TcpListener};
        let launcher = RealSupervisorLauncher::default();
        let pid = std::process::id();
        let started_at = process_start_time(pid).expect("self process start time");
        let mut state = state_for(21);
        state.supervisor_pid = pid;
        state.engine_pid = pid;
        // 닫힌 포트 = 리스너 부재. 연결 시도는 즉시 ECONNREFUSED로 떨어진다.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("port reservation");
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        state.port = port;
        let session_key = [21_u8; 32];
        let dir = scratch_dir("health-refused");
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

        let error = launcher.validate_live(&state).unwrap_err();
        assert_eq!(error.code, "ENGINE_EXITED");
        assert!(
            error.message.contains("health connect"),
            "connect 단계 실패여야 한다(픽스처 전제): {}",
            error.message
        );
        assert!(
            launcher.managed.lock().unwrap().is_empty(),
            "ECONNREFUSED 1회로 즉시 축출돼야 한다 — 3회를 요구하면 회복이 2클릭에서 4클릭이 된다"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// F-2의 **대조군**: refused만 치명이고 나머지 TCP 오류(timeout·reset·unreachable)는 일시로
    /// 남아 연속 3회 한계에 맡겨진다. timeout을 치명으로 넓히면 부하 스파이크 1회가 살아있는
    /// 런타임을 파괴한다(WS-0이 막으려던 바로 그 결함).
    #[test]
    fn only_connection_refused_is_a_fatal_health_connect_error() {
        use std::io::ErrorKind;
        assert_eq!(
            health_connect_severity(ErrorKind::ConnectionRefused),
            ValidationSeverity::Fatal
        );
        for kind in [
            ErrorKind::TimedOut,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::Interrupted,
            ErrorKind::WouldBlock,
            ErrorKind::AddrNotAvailable,
            ErrorKind::PermissionDenied,
        ] {
            assert_eq!(
                health_connect_severity(kind),
                ValidationSeverity::Transient,
                "{kind:?}는 살아있는 런타임에서도 발생 가능 — 일시로 남아야 한다"
            );
        }

        // 판정이 실제 축출 규약에 어떻게 꽂히는지까지 못 박는다: timeout은 3회, refused는 1회.
        let started_at = process_start_time(std::process::id()).expect("self start time");
        let dir = scratch_dir("health-severity");
        let timeout_state = state_for(22);
        let timeout_runtime = test_runtime(
            &timeout_state,
            dir.join("timeout.json"),
            [22_u8; 32],
            started_at,
            None,
        );
        for attempt in 1..MAX_CONSECUTIVE_TRANSIENT_VALIDATION_FAILURES {
            assert!(
                !should_evict_after_validation_failure(
                    &timeout_runtime,
                    health_connect_severity(ErrorKind::TimedOut)
                ),
                "timeout {attempt}회로는 축출되지 않는다"
            );
        }
        assert!(
            should_evict_after_validation_failure(
                &timeout_runtime,
                health_connect_severity(ErrorKind::TimedOut)
            ),
            "timeout은 연속 3회에서 비로소 축출된다"
        );

        let refused_state = state_for(23);
        let refused_runtime = test_runtime(
            &refused_state,
            dir.join("refused.json"),
            [23_u8; 32],
            started_at,
            None,
        );
        assert!(
            should_evict_after_validation_failure(
                &refused_runtime,
                health_connect_severity(ErrorKind::ConnectionRefused)
            ),
            "refused는 첫 회에 축출된다"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// JSON 파싱 실패는 내용 손상이라 재시도로 회복되지 않는다 — 치명 유지(4-3 경계의 반대편).
    #[test]
    fn corrupt_endpoint_json_stays_fatal_on_the_first_occurrence() {
        let launcher = RealSupervisorLauncher::default();
        let pid = std::process::id();
        let started_at = process_start_time(pid).expect("self process start time");
        let mut state = state_for(14);
        state.supervisor_pid = pid;
        state.engine_pid = pid;
        let dir = scratch_dir("endpoint-corrupt");
        let endpoint_path = dir.join("state.json");
        std::fs::write(&endpoint_path, b"{not json").unwrap();
        launcher.managed.lock().unwrap().insert(
            state.instance_id.clone(),
            test_runtime(&state, endpoint_path, [14_u8; 32], started_at, None),
        );

        let error = launcher.validate_live(&state).unwrap_err();

        assert_eq!(error.code, "RUNTIME_INTEGRITY_FAILED");
        assert!(
            launcher.managed.lock().unwrap().is_empty(),
            "손상된 내용은 재시도로 회복되지 않는다"
        );
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

    #[cfg(unix)]
    #[test]
    fn terminate_supervisor_reclaims_an_eof_honoring_child_within_grace() {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("read line; exit 0")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("eof-honoring child");
        let mut control = child.stdin.take();

        let started = std::time::Instant::now();
        let status = terminate_supervisor(
            &mut child,
            &mut control,
            std::time::Duration::from_millis(1500),
        )
        .expect("terminated child must be reaped");

        assert!(
            status.success(),
            "an EOF-honoring child exits on its own — SIGKILL must not be needed"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(1500),
            "EOF must reclaim the child before the grace expires"
        );
        assert!(control.is_none(), "the control pipe must be taken and closed");
    }

    #[cfg(unix)]
    #[test]
    fn terminate_supervisor_escalates_to_sigkill_for_an_eof_ignoring_child() {
        use std::os::unix::process::ExitStatusExt;
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("trap \"\" TERM; sleep 30 </dev/null")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("eof-ignoring child");
        let mut control = child.stdin.take();

        let status = terminate_supervisor(
            &mut child,
            &mut control,
            std::time::Duration::from_millis(100),
        )
        .expect("escalated child must be reaped");

        assert_eq!(
            status.signal(),
            Some(9),
            "a child that ignores EOF must be escalated to SIGKILL"
        );
    }

    /// 가짜 supervisor 픽스처: stderr를 tap으로 채집하고 종료까지 회수한 뒤
    /// (사인, 종료 상태)를 돌려준다 — 생산 경로와 동일한 순서다.
    #[cfg(unix)]
    fn tapped_fixture(script: &str) -> (Option<String>, Option<ExitStatus>) {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("fixture supervisor");
        let tap = StderrTap::spawn(child.stderr.take());
        let mut control = child.stdin.take();
        let status = terminate_supervisor(
            &mut child,
            &mut control,
            std::time::Duration::from_millis(1500),
        );
        (tap.death_sign(), status)
    }

    #[cfg(unix)]
    #[test]
    fn silent_immediate_death_is_classified_with_its_exit_status() {
        let (sign, status) = tapped_fixture("exit 70");

        assert_eq!(sign, None);
        assert_eq!(status.and_then(|status| status.code()), Some(70));
        assert_eq!(
            supervisor_death_message(sign, status),
            "supervisor left no stderr sign (exit 70)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn supervisor_sign_leads_the_message_and_fits_the_banner() {
        let (sign, status) = tapped_fixture(
            "echo 'cys-browserd: RUNTIME_ALREADY_RUNNING: another supervisor owns this EngineKey lock' >&2; exit 70",
        );

        let message = supervisor_death_message(sign, status);
        assert_eq!(
            message,
            "RUNTIME_ALREADY_RUNNING: another supervisor owns this EngineKey lock (exit 70)"
        );
        assert!(
            message.len() <= 120,
            "the sign must survive the 120-char banner truncation"
        );
        assert!(
            !message.contains("cys-browserd: "),
            "the log-only prefix must not enter the banner message"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_slow_living_supervisor_never_blocks_the_sign_snapshot() {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 30 </dev/null")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("slow supervisor");
        let tap = StderrTap::spawn(child.stderr.take());

        let started = std::time::Instant::now();
        let sign = tap.death_sign();
        let elapsed = started.elapsed();

        assert_eq!(sign, None);
        assert!(
            elapsed < std::time::Duration::from_millis(600),
            "the EOF wait must be bounded, not an unbounded join (took {elapsed:?})"
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    #[cfg(unix)]
    #[test]
    fn oversized_stderr_keeps_the_last_complete_line() {
        let (sign, _) = tapped_fixture(
            "i=0; while [ $i -lt 400 ]; do printf 'noise %s aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\\n' $i >&2; i=$((i+1)); done; echo 'ENGINE_EXITED: engine exited: signal 11' >&2; exit 70",
        );

        assert_eq!(
            sign.as_deref(),
            Some("ENGINE_EXITED: engine exited: signal 11")
        );
    }

    #[cfg(unix)]
    #[test]
    fn self_inflicted_liveness_lines_are_not_mistaken_for_a_sign() {
        let (sign, _) = tapped_fixture(
            "echo 'cys-browserd: ENGINE_EXITED: engine exited: signal 11' >&2; \
             echo 'cys-browserd: AUTHORITY_REJECTED: broker liveness pipe closed' >&2; exit 77",
        );

        assert_eq!(
            sign.as_deref(),
            Some("ENGINE_EXITED: engine exited: signal 11"),
            "filter-then-last: our own pipe close must not bury the real sign"
        );
    }

    #[cfg(unix)]
    #[test]
    fn incomplete_trailing_line_is_not_reported_as_a_sign() {
        let (sign, _) = tapped_fixture("printf 'partial line without newline' >&2; exit 1");

        assert_eq!(sign, None, "only complete lines are signs");
    }

    #[cfg(unix)]
    #[test]
    fn tap_eof_synchronisation_collects_the_sign_every_time() {
        for iteration in 0..100 {
            let (sign, status) = tapped_fixture("echo 'RUNTIME_START_FAILED: boom' >&2; exit 3");
            assert_eq!(
                sign.as_deref(),
                Some("RUNTIME_START_FAILED: boom"),
                "sign collection must be deterministic (iteration {iteration})"
            );
            assert_eq!(status.and_then(|status| status.code()), Some(3));
        }
    }

    #[cfg(unix)]
    #[test]
    fn self_inflicted_sigkill_is_suppressed_from_the_message() {
        use std::os::unix::process::ExitStatusExt;
        let killed = ExitStatus::from_raw(9);
        assert_eq!(exit_annotation(Some(killed)), None);
        assert_eq!(
            supervisor_death_message(Some("SOME_SIGN: detail".into()), Some(killed)),
            "SOME_SIGN: detail"
        );
    }

    #[test]
    fn termination_grace_is_split_by_readiness_phase() {
        // pre-readiness에서 supervisor는 liveness 스레드가 없어 EOF가 무효다 —
        // 긴 유예는 회수 이득 0에 취소 응답성만 해친다(취소 회귀 테스트의 계약).
        assert_eq!(HANDSHAKE_TERMINATION_GRACE, std::time::Duration::ZERO);
        assert!(PRE_READINESS_TERMINATION_GRACE < std::time::Duration::from_millis(250));
        assert!(POST_READINESS_TERMINATION_GRACE >= std::time::Duration::from_secs(3));
    }

    /// stale 화신을 evict해도 **교체 화신의 control 파이프는 건드리지 않는다**.
    ///
    /// ★적대 검증 BLOCK-2: 종전 판본은 픽스처가 양쪽 control을 `None`으로 만들어 놓고
    /// `!replacement.control...is_some()`을 단언했다 — "None이 None임"을 확인할 뿐이라
    /// 명제를 검증할 수 없는 자기충족 테스트였다. 이제 교체 화신에 **실제 자식 프로세스의
    /// stdin 파이프**를 물리고, evict 후에도 그 파이프가 살아있으며(`is_some()`) 자식이
    /// 계속 실행 중임을 단언한다.
    #[cfg(unix)]
    #[test]
    fn eviction_never_terminates_a_replacement_incarnation() {
        let launcher = RealSupervisorLauncher::default();
        let instance_id = "d".repeat(32);

        // 교체 화신 = 살아있는 자식. stdin EOF를 감지하면 즉시 종료하는 진짜 supervisor 모사.
        let mut replacement_child = Command::new("/bin/sh")
            .arg("-c")
            .arg("read line; exit 0")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("replacement supervisor");
        let replacement_control = replacement_child.stdin.take().expect("control pipe");

        let mut stale_state = state_for(11);
        stale_state.instance_id = instance_id.clone();
        stale_state.engine_generation = 11;
        let stale = test_runtime(&stale_state, "stale".into(), [11_u8; 32], 1, None);
        let mut replacement_state = state_for(12);
        replacement_state.instance_id = instance_id.clone();
        replacement_state.engine_generation = 12;
        let replacement = test_runtime(
            &replacement_state,
            "new".into(),
            [12_u8; 32],
            1,
            Some(replacement_control),
        );
        launcher
            .managed
            .lock()
            .unwrap()
            .insert(instance_id.clone(), replacement.clone());

        launcher.evict_managed_runtime(&stale, "stale");

        assert_eq!(launcher.managed.lock().unwrap().len(), 1);
        assert!(
            replacement.control.lock().unwrap().is_some(),
            "stale evict가 교체 화신의 control 파이프를 빼앗으면 살아있는 런타임을 무음 종료시킨다"
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
        // 파이프가 살아있다는 사실을 프로세스 관측으로 이중 확인한다(구조체 필드만이 아니라).
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            replacement_child.try_wait().unwrap().is_none(),
            "교체 화신은 stale evict 후에도 계속 살아 있어야 한다"
        );

        // 대조군: 교체 화신을 실제로 evict하면 그때는 파이프가 닫히고 자식이 회수된다.
        launcher.evict_managed_runtime(&replacement, "replacement");
        assert!(replacement.control.lock().unwrap().is_none());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while replacement_child.try_wait().unwrap().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "EOF를 존중하는 자식은 evict로 회수돼야 한다"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// ★적대 검증 4-2 ②: **evict 종료의 한계를 계약으로 박제**한다(고치는 게 아니라 정직한 기록).
    ///
    /// `terminate_evicted_runtime`은 control 파이프만 닫고 끝난다 — `try_wait` 폴링도 SIGKILL
    /// 격상도 없다(성공 커밋 후 `Child` 핸들은 워처 스레드가 단독 소유하므로 구조적으로 불가).
    /// 따라서 EOF를 무시하거나 wedge된 supervisor는 **장부에서만 사라지고 프로세스는 잔존**한다.
    /// 이 테스트가 깨진다면 격상 핸들이 도입돼 한계가 해소된 것이므로 계약 문서와 함께 갱신하라.
    #[cfg(unix)]
    #[test]
    fn evicting_an_eof_ignoring_supervisor_leaves_it_alive_by_contract() {
        // stdin을 아예 읽지 않는 자식 = EOF 무시(wedge된 supervisor 모사).
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("trap \"\" TERM; sleep 30")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("eof-ignoring supervisor");
        let control = child.stdin.take().expect("control pipe");
        let launcher = RealSupervisorLauncher::default();
        let state = state_for(15);
        let runtime = test_runtime(&state, "unused".into(), [15_u8; 32], 1, Some(control));
        launcher
            .managed
            .lock()
            .unwrap()
            .insert(state.instance_id.clone(), runtime.clone());

        launcher.evict_managed_runtime(&runtime, "eof-ignoring");

        assert!(
            launcher.managed.lock().unwrap().is_empty(),
            "장부에서는 사라진다"
        );
        assert!(
            runtime.control.lock().unwrap().is_none(),
            "종료 요청(파이프 닫기)은 반드시 수행된다"
        );
        // 잔존물 계약: 유예를 충분히 주어도 이 프로세스는 죽지 않는다.
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(
            child.try_wait().unwrap().is_none(),
            "evict 종료는 best-effort — EOF 무시 supervisor는 잔존한다(§4-3 잔존물 계약 등급의 인정된 한계)"
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    /// §5-0-A-1 계약: 신설·개명 에러 코드는 **26자 이내**여야 배너
    /// `BROWSER_DISABLED_SAFE [<CODE>]: <message>`가 120자 절단(`ui/src/webpane.ts:386`) 안에
    /// 사유를 보존한다. 소스 자체를 스캔하는 트립와이어라 이 파일에 긴 코드를 새로 넣으면 깨진다.
    #[test]
    fn every_error_code_literal_in_this_module_fits_the_26_char_cap() {
        let offenders = super::over_long_screaming_case_literals(include_str!("runtime_launcher.rs"));
        assert!(
            offenders.is_empty(),
            "에러 코드는 26자 이내여야 한다(배너 120자 절단): {offenders:?}"
        );
    }
}
