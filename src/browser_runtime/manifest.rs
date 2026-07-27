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

#[cfg(test)]
mod tests {
    use serde_json::Value;

    /// 빌드 산출물 `browser-runtime.lock.json`의 위치. 스테이징 스크립트가
    /// `src-tauri/resources/browser-runtime/`에 쓴다(`scripts/runtime-stage.py:352`,
    /// `scripts/build-macos-signed.sh:89`, `scripts/build-windows-signed.ps1:42`).
    fn staged_lock_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src-tauri/resources/browser-runtime/browser-runtime.lock.json")
    }

    /// `Value`를 재귀 순회하며 f64로 역직렬화되는 수를 모두 모은다(경로와 함께).
    fn collect_floats(value: &Value, path: &str, found: &mut Vec<String>) {
        match value {
            Value::Number(number) => {
                if number.is_f64() {
                    found.push(format!("{path} = {number}"));
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    collect_floats(item, &format!("{path}[{index}]"), found);
                }
            }
            Value::Object(entries) => {
                for (key, item) in entries {
                    collect_floats(item, &format!("{path}.{key}"), found);
                }
            }
            _ => {}
        }
    }

    /// ★WS-6 요구4 · 설계 §8-3 — **lock.json float 금지 pin**.
    ///
    /// 매니페스트의 모든 수는 정수여야 한다. float이 섞이면 `float_roundtrip` 유무에 따라
    /// 정준화(`write_canonical`)의 `Number::to_string()`이 달라져 `runtime_id`가 바뀌고,
    /// 그 결과 브로커·supervisor의 runtime_id 대조가 무너진다(F9/F10).
    ///
    /// **파일 부재 시의 계약(명시)**: 이 파일은 릴리스 스테이징이 만드는 **빌드 산출물**이라
    /// 개발 체크아웃에는 없는 것이 정상이다(`.placeholder`만 존재). 따라서 부재는 **통과**로
    /// 계약한다 — skip이 아니라 "검사 대상이 없으므로 위반도 없다"는 판정이다. 파일이 존재하면
    /// **반드시** 검사하며, 이때 파싱 실패나 float 발견은 전부 실패다(조건부 무해화 금지).
    #[test]
    fn staged_lock_json_carries_no_floating_point_numbers() {
        let path = staged_lock_path();
        let input = match std::fs::read_to_string(&path) {
            Ok(input) => input,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // 계약: 빌드 산출물 부재 = 통과. 존재할 때만 검사 대상이 된다.
                return;
            }
            Err(error) => panic!("스테이징된 lock.json을 읽을 수 없다 ({}): {error}", path.display()),
        };
        let value: Value = serde_json::from_str(&input)
            .unwrap_or_else(|error| panic!("lock.json은 유효 JSON이어야 한다: {error}"));

        let mut floats = Vec::new();
        collect_floats(&value, "$", &mut floats);

        assert!(
            floats.is_empty(),
            "lock.json의 모든 수는 정수여야 한다(float은 정준화 표기를 흔들어 runtime_id를 깨뜨린다): {floats:?}"
        );
    }

    /// 위 pin이 실제로 float을 잡아내는지 확인하는 양성 대조군 — 순회기가 조용히
    /// 망가져도 본 테스트가 통과해버리는 사태(vacuous pass)를 막는다.
    #[test]
    fn the_float_scanner_actually_detects_nested_floats() {
        let value: Value = serde_json::from_str(
            r#"{"a":1,"b":{"c":[0,1,2.5]},"d":[{"e":3}],"f":"2.5","g":true}"#,
        )
        .unwrap();
        let mut floats = Vec::new();
        collect_floats(&value, "$", &mut floats);
        assert_eq!(floats, vec!["$.b.c[2] = 2.5".to_string()]);
    }
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
