use super::{BrokerFailure, LaunchRequest, SupervisorLauncher};
use cys::browser_runtime::{BrokerHello, ParsedRuntimeState, RuntimePaths, RuntimeStateV2};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

pub(super) struct RealSupervisorLauncher;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeAttestation {
    attestation_schema: u16,
    runtime_id: String,
    target: String,
    supervisor_sha256: String,
    engine_sha256: String,
    chromium_tree_sha256: String,
    signature: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimePolicySnapshot {
    epoch: u64,
    allowed_runtime_ids: Vec<String>,
    signature: String,
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
        let mut child = Command::new(&paths.supervisor)
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
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                BrokerFailure::new(
                    "RUNTIME_START_FAILED",
                    format!("verified supervisor spawn failed: {error}"),
                )
            })?;
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
        let state = match ready_rx.recv_timeout(std::time::Duration::from_secs(25)) {
            Ok(Ok((count, line))) if count > 0 && count <= 64 * 1024 => {
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
            Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BrokerFailure::new(
                    "RUNTIME_START_FAILED",
                    "supervisor readiness timed out or returned an invalid frame",
                ));
            }
        };
        std::thread::spawn(move || {
            let _control_liveness = control;
            let _ = child.wait();
        });
        Ok(state)
    }
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
    if attestation.attestation_schema != 1
        || attestation.runtime_id != request.manifest.runtime_id
        || attestation.target != request.target.as_str()
        || attestation.supervisor_sha256 != assets.supervisor_sha256
        || attestation.engine_sha256 != assets.engine_sha256
        || attestation.chromium_tree_sha256 != assets.chromium_tree_sha256
        || attestation.signature.is_empty()
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
    if policy.epoch == 0
        || policy.signature.is_empty()
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
