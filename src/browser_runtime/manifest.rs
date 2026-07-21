use crate::browser_runtime::{BrowserError, ProtocolRange};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const MAX_MANIFEST_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum TargetTriple {
    #[serde(rename = "aarch64-apple-darwin")]
    Aarch64AppleDarwin,
    #[serde(rename = "x86_64-apple-darwin")]
    X86_64AppleDarwin,
    #[serde(rename = "x86_64-pc-windows-msvc")]
    X86_64PcWindowsMsvc,
}

impl TargetTriple {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Aarch64AppleDarwin => "aarch64-apple-darwin",
            Self::X86_64AppleDarwin => "x86_64-apple-darwin",
            Self::X86_64PcWindowsMsvc => "x86_64-pc-windows-msvc",
        }
    }

    pub const fn executable_suffix(&self) -> &'static str {
        match self {
            Self::X86_64PcWindowsMsvc => ".exe",
            _ => "",
        }
    }

    pub fn current() -> Result<Self, BrowserError> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Ok(Self::Aarch64AppleDarwin),
            ("macos", "x86_64") => Ok(Self::X86_64AppleDarwin),
            ("windows", "x86_64") => Ok(Self::X86_64PcWindowsMsvc),
            (os, arch) => Err(BrowserError::invalid_manifest(format!(
                "unsupported Browser Runtime target {arch}-{os}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SupervisorPin {
    pub build_id: String,
    pub rust_toolchain: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnginePin {
    pub build_id: String,
    pub bun_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaywrightPin {
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChromiumPin {
    pub revision: String,
    pub version: String,
    pub major: u32,
    pub profile_schema_epoch: u64,
    pub license: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetAssets {
    pub architecture: String,
    pub supervisor_sha256: String,
    pub engine_sha256: String,
    pub chromium_archive_url: String,
    pub chromium_archive_sha256: String,
    pub chromium_tree_sha256: String,
    pub chromium_executable: String,
    pub license_files: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub schema_version: u16,
    pub release_ready: bool,
    pub runtime_id: String,
    pub browser_protocol: ProtocolRange,
    pub supervisor: SupervisorPin,
    pub engine: EnginePin,
    pub playwright: PlaywrightPin,
    pub chromium: ChromiumPin,
    pub targets: BTreeMap<TargetTriple, TargetAssets>,
}

impl RuntimeManifest {
    pub fn parse(input: &str) -> Result<Self, BrowserError> {
        if input.len() > MAX_MANIFEST_BYTES {
            return Err(BrowserError::invalid_manifest("manifest exceeds 256 KiB"));
        }
        let value: Value = serde_json::from_str(input)
            .map_err(|e| BrowserError::invalid_manifest(format!("invalid JSON: {e}")))?;
        let expected_id = Self::runtime_id_for_value(&value)?;
        let manifest: Self = serde_json::from_value(value)
            .map_err(|e| BrowserError::invalid_manifest(format!("invalid schema: {e}")))?;
        manifest.validate(&expected_id)?;
        Ok(manifest)
    }

    pub fn runtime_id_for_value(value: &Value) -> Result<String, BrowserError> {
        let mut value = value.clone();
        let object = value
            .as_object_mut()
            .ok_or_else(|| BrowserError::invalid_manifest("manifest root must be a JSON object"))?;
        object.remove("runtime_id");
        object.remove("signature");
        object.remove("attestation");
        let mut canonical = String::new();
        write_canonical(&value, &mut canonical)?;
        Ok(format!("sha256:{:x}", Sha256::digest(canonical.as_bytes())))
    }

    pub fn target(&self, target: &TargetTriple) -> Result<&TargetAssets, BrowserError> {
        self.targets.get(target).ok_or_else(|| {
            BrowserError::invalid_manifest(format!("target {} is not pinned", target.as_str()))
        })
    }

    fn validate(&self, expected_id: &str) -> Result<(), BrowserError> {
        if self.schema_version != 1 {
            return Err(BrowserError::invalid_manifest(format!(
                "unsupported manifest schema {}",
                self.schema_version
            )));
        }
        if self.runtime_id != expected_id {
            return Err(BrowserError::runtime_integrity(format!(
                "runtime_id mismatch: expected {expected_id}, got {}",
                self.runtime_id
            )));
        }
        if self.browser_protocol.major != 2
            || self.browser_protocol.min_minor > self.browser_protocol.max_minor
            || self.targets.is_empty()
            || self.supervisor.build_id.is_empty()
            || self.engine.build_id.is_empty()
            || self.engine.bun_version.is_empty()
            || self.playwright.version.is_empty()
            || self.chromium.revision.is_empty()
            || self.chromium.license.is_empty()
            || self.chromium.profile_schema_epoch == 0
        {
            return Err(BrowserError::invalid_manifest(
                "manifest identity or protocol is incomplete",
            ));
        }
        for (target, assets) in &self.targets {
            let expected_arch = match target {
                TargetTriple::Aarch64AppleDarwin => "arm64",
                TargetTriple::X86_64AppleDarwin | TargetTriple::X86_64PcWindowsMsvc => "x86_64",
            };
            if assets.architecture != expected_arch
                || !is_sha256(&assets.supervisor_sha256)
                || !is_sha256(&assets.engine_sha256)
                || !is_sha256(&assets.chromium_archive_sha256)
                || !is_sha256(&assets.chromium_tree_sha256)
                || !assets.chromium_archive_url.starts_with("https://")
                || !safe_relative_path(&assets.chromium_executable)
                || assets.license_files.is_empty()
                || assets.license_files.iter().any(|p| !safe_relative_path(p))
            {
                return Err(BrowserError::invalid_manifest(format!(
                    "target {} has incomplete or unsafe pinned assets",
                    target.as_str()
                )));
            }
        }
        Ok(())
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.starts_with('\\')
        && value
            .split(['/', '\\'])
            .all(|part| !matches!(part, "" | "." | ".."))
}

fn write_canonical(value: &Value, output: &mut String) -> Result<(), BrowserError> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(v) => output.push_str(if *v { "true" } else { "false" }),
        Value::Number(v) => output.push_str(&v.to_string()),
        Value::String(v) => output.push_str(
            &serde_json::to_string(v)
                .map_err(|e| BrowserError::invalid_manifest(format!("string encoding: {e}")))?,
        ),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write_canonical(value, output)?;
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut keys: Vec<_> = values.keys().collect();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).map_err(|e| {
                        BrowserError::invalid_manifest(format!("key encoding: {e}"))
                    })?,
                );
                output.push(':');
                write_canonical(&values[key], output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}
