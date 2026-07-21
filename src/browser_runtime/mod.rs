//! Browser v2 runtime contract.
//!
//! Callers see typed state and compatibility decisions. Filesystem shape,
//! legacy parsing and runtime validation stay behind this module boundary.

mod compatibility;
mod error;
mod manifest;
mod path;
mod private_protocol;
mod state;

pub use compatibility::{
    evaluate_compatibility, Compatibility, CompatibilityIssue, CompatibilityRequirement,
    ProtocolRequirement,
};
pub use error::{BrowserError, BrowserErrorCode};
pub use manifest::{
    ChromiumPin, EnginePin, PlaywrightPin, RuntimeManifest, SupervisorPin, TargetAssets,
    TargetTriple,
};
pub use path::RuntimePaths;
pub use private_protocol::{BrokerHello, VerifiedBrokerHello};
pub use state::{
    parse_runtime_state, EngineKey, EngineMode, LegacyRuntimeState, ParsedRuntimeState,
    ProtocolRange, RuntimeStateV2,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_state_is_parsed_but_never_compatible() {
        let parsed = parse_runtime_state(r#"{"pid":42,"port":53111,"token":"legacy"}"#)
            .expect("legacy state remains diagnosable");

        assert!(matches!(parsed, ParsedRuntimeState::LegacyIncompatible(_)));
        assert_eq!(parsed.compatibility_code(), "LEGACY_INCOMPATIBLE");
    }

    #[test]
    fn v2_state_requires_exact_runtime_and_compatible_protocol() {
        let parsed = parse_runtime_state(
            r#"{
              "schema_version":2,
              "instance_id":"00112233445566778899aabbccddeeff",
              "engine_generation":3,
              "supervisor_pid":42,
              "engine_pid":43,
              "port":53111,
              "supervisor_build_id":"sup-1",
              "engine_build_id":"eng-1",
              "runtime_id":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "attestation_id":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
              "policy_epoch":7,
              "policy_hash":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
              "protocol":{"major":2,"min_minor":0,"max_minor":2,
                "capabilities":["scoped-ticket","paint-ack"],
                "required_capabilities":["scoped-ticket"]},
              "chromium_revision":"1148",
              "engine_key":{"realm":"shared-default","mode":"headless"},
              "profile_epoch":1,
              "started_at":"2026-07-21T00:00:00Z"
            }"#,
        )
        .expect("valid v2 state");
        let required = CompatibilityRequirement::new(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ProtocolRequirement::new(2, 1, 3, ["scoped-ticket", "paint-ack"]),
            EngineKey::shared_default(),
        );

        assert_eq!(
            evaluate_compatibility(&parsed, &required),
            Compatibility::Compatible
        );
    }

    #[test]
    fn manifest_runtime_id_binds_target_hashes_and_licenses() {
        let mut value = serde_json::json!({
            "schema_version": 1,
            "release_ready": true,
            "runtime_id": "sha256:placeholder",
            "browser_protocol": {"major":2,"min_minor":0,"max_minor":0,
              "capabilities":["scoped-ticket"],"required_capabilities":["scoped-ticket"]},
            "supervisor": {"build_id":"sup-1","rust_toolchain":"1.88.0"},
            "engine": {"build_id":"eng-1","bun_version":"1.3.8"},
            "playwright": {"version":"1.49.1"},
            "chromium": {"revision":"1148","version":"131.0.6778.33","major":131,
              "profile_schema_epoch":1,"license":"Chromium-BSD"},
            "targets": {
              "aarch64-apple-darwin": {
                "architecture":"arm64",
                "supervisor_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "engine_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "chromium_archive_url":"https://example.invalid/chromium-1148-mac-arm64.zip",
                "chromium_archive_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "chromium_tree_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                "chromium_executable":"chromium/Chromium.app/Contents/MacOS/Chromium",
                "license_files":["LICENSE.chromium","THIRD_PARTY_NOTICES.chromium"]
              }
            }
        });
        let id = RuntimeManifest::runtime_id_for_value(&value).expect("canonical manifest id");
        value["runtime_id"] = serde_json::Value::String(id.clone());
        let manifest = RuntimeManifest::parse(&serde_json::to_string(&value).unwrap())
            .expect("release-qualified manifest");

        assert_eq!(manifest.runtime_id, id);
        assert_eq!(
            manifest
                .target(&TargetTriple::Aarch64AppleDarwin)
                .unwrap()
                .architecture,
            "arm64"
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_resolver_rejects_symlinked_runtime_executable() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "cys-browser-runtime-path-test-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        let target_root = root.join("aarch64-apple-darwin");
        std::fs::create_dir_all(target_root.join("supervisor")).unwrap();
        std::fs::create_dir_all(target_root.join("engine")).unwrap();
        std::fs::create_dir_all(target_root.join("chromium/Chromium.app/Contents/MacOS")).unwrap();
        let outside = root.join("outside-browserd");
        std::fs::write(&outside, b"not the bundled supervisor").unwrap();
        symlink(&outside, target_root.join("supervisor/cys-browserd")).unwrap();
        std::fs::write(target_root.join("engine/cys-browser-engine"), b"engine").unwrap();
        std::fs::write(
            target_root.join("chromium/Chromium.app/Contents/MacOS/Chromium"),
            b"chromium",
        )
        .unwrap();
        let assets = TargetAssets {
            architecture: "arm64".into(),
            supervisor_sha256: "a".repeat(64),
            engine_sha256: "b".repeat(64),
            chromium_archive_url: "https://example.invalid/chromium.zip".into(),
            chromium_archive_sha256: "c".repeat(64),
            chromium_tree_sha256: "d".repeat(64),
            chromium_executable: "chromium/Chromium.app/Contents/MacOS/Chromium".into(),
            license_files: vec!["LICENSE.chromium".into()],
        };

        let error =
            RuntimePaths::resolve_existing(&root, &TargetTriple::Aarch64AppleDarwin, &assets)
                .expect_err("symlinked supervisor must never be executable authority");
        assert_eq!(error.code(), BrowserErrorCode::RuntimePathRejected);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_broker_hello_is_parent_bound_mac_verified_and_single_sequence() {
        let session_key = [0x5a; 32];
        let hello = BrokerHello::signed(
            4242,
            1,
            "receipt-1",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            7,
            "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            session_key,
        )
        .expect("broker can create a private hello");

        let verified = hello
            .verify(4242, session_key)
            .expect("the inherited peer and MAC agree");
        assert_eq!(verified.command_sequence, 1);
        assert_eq!(verified.policy_epoch, 7);
        assert!(
            hello.verify(7, session_key).is_err(),
            "forged parent is rejected"
        );
        assert!(
            hello.verify(4242, [0x33; 32]).is_err(),
            "wrong session key is rejected"
        );
    }
}
