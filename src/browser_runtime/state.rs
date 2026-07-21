use crate::browser_runtime::BrowserError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_STATE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyRuntimeState {
    pub pid: u32,
    pub port: u16,
    pub headless: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineMode {
    Headless,
    Headful,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EngineKey {
    pub realm: String,
    pub mode: EngineMode,
}

impl EngineKey {
    pub fn shared_default() -> Self {
        Self {
            realm: "shared-default".into(),
            mode: EngineMode::Headless,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProtocolRange {
    pub major: u16,
    pub min_minor: u16,
    pub max_minor: u16,
    #[serde(default)]
    pub capabilities: BTreeSet<String>,
    #[serde(default)]
    pub required_capabilities: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeStateV2 {
    pub schema_version: u16,
    pub instance_id: String,
    pub engine_generation: u64,
    pub supervisor_pid: u32,
    pub engine_pid: u32,
    pub port: u16,
    pub supervisor_build_id: String,
    pub engine_build_id: String,
    pub runtime_id: String,
    pub attestation_id: String,
    pub policy_epoch: u64,
    pub policy_hash: String,
    pub protocol: ProtocolRange,
    pub chromium_revision: String,
    pub engine_key: EngineKey,
    pub profile_epoch: u64,
    pub started_at: String,
}

impl RuntimeStateV2 {
    fn validate(&self) -> Result<(), BrowserError> {
        if self.schema_version != 2 {
            return Err(BrowserError::unsupported_state_schema(format!(
                "expected state schema 2, got {}",
                self.schema_version
            )));
        }
        if self.supervisor_pid == 0
            || self.engine_pid == 0
            || self.port == 0
            || self.engine_generation == 0
            || self.profile_epoch == 0
        {
            return Err(BrowserError::corrupt_state(
                "v2 state has a zero identity or endpoint",
            ));
        }
        if !is_hex_id(&self.instance_id, 32)
            || !is_sha256_id(&self.runtime_id)
            || !is_sha256_id(&self.attestation_id)
            || !is_sha256_id(&self.policy_hash)
        {
            return Err(BrowserError::corrupt_state(
                "v2 state contains a malformed identity",
            ));
        }
        if self.protocol.min_minor > self.protocol.max_minor || self.engine_key.realm.is_empty() {
            return Err(BrowserError::corrupt_state(
                "v2 state protocol or engine key is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParsedRuntimeState {
    V2(RuntimeStateV2),
    LegacyIncompatible(LegacyRuntimeState),
}

impl ParsedRuntimeState {
    pub const fn compatibility_code(&self) -> &'static str {
        match self {
            Self::V2(_) => "V2_REQUIRES_EVALUATION",
            Self::LegacyIncompatible(_) => "LEGACY_INCOMPATIBLE",
        }
    }
}

#[derive(Deserialize)]
struct LegacyWireState {
    pid: u32,
    port: u16,
    token: String,
    #[serde(default)]
    headless: Option<bool>,
}

pub fn parse_runtime_state(input: &str) -> Result<ParsedRuntimeState, BrowserError> {
    if input.len() > MAX_STATE_BYTES {
        return Err(BrowserError::corrupt_state("state exceeds 64 KiB"));
    }
    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| BrowserError::corrupt_state(format!("invalid state JSON: {e}")))?;
    match value.get("schema_version") {
        Some(version) => {
            let version = version.as_u64().ok_or_else(|| {
                BrowserError::corrupt_state("state schema_version is not an integer")
            })?;
            if version != 2 {
                return Err(BrowserError::unsupported_state_schema(format!(
                    "unsupported state schema {version}"
                )));
            }
            let state: RuntimeStateV2 = serde_json::from_value(value)
                .map_err(|e| BrowserError::corrupt_state(format!("invalid v2 state: {e}")))?;
            state.validate()?;
            Ok(ParsedRuntimeState::V2(state))
        }
        None => {
            let wire: LegacyWireState = serde_json::from_value(value)
                .map_err(|e| BrowserError::corrupt_state(format!("invalid legacy state: {e}")))?;
            if wire.pid == 0 || wire.port == 0 || wire.token.is_empty() {
                return Err(BrowserError::corrupt_state(
                    "legacy state has an invalid pid, port or token",
                ));
            }
            Ok(ParsedRuntimeState::LegacyIncompatible(LegacyRuntimeState {
                pid: wire.pid,
                port: wire.port,
                headless: wire.headless,
            }))
        }
    }
}

fn is_hex_id(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_sha256_id(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|v| is_hex_id(v, 64))
}
