import hashlib
import importlib.util
import base64
from datetime import datetime, timezone
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "browser-runtime-metadata.py"
SPEC = importlib.util.spec_from_file_location("browser_runtime_metadata", SCRIPT)
module = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(module)


class BrowserRuntimeMetadataTests(unittest.TestCase):
    def test_minisign_uses_noninteractive_unencrypted_release_key(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = root / "payload.json"
            signature = root / "payload.json.minisig"
            secret_key = root / "release.key"

            with mock.patch.object(module.subprocess, "run") as run:
                module.minisign(payload, signature, secret_key)

            run.assert_called_once_with(
                [
                    "minisign",
                    "-W",
                    "-Sm",
                    str(payload),
                    "-s",
                    str(secret_key),
                    "-x",
                    str(signature),
                ],
                check=True,
            )

    def test_tree_hash_binds_relative_path_size_and_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            executable = root / "bin" / "chromium"
            executable.write_bytes(b"v1")
            first = module.hash_tree(root)
            self.assertRegex(first, r"^[0-9a-f]{64}$")
            executable.write_bytes(b"v2")
            self.assertNotEqual(module.hash_tree(root), first)

    def test_runtime_id_is_canonical_and_binds_target_hashes(self):
        manifest = {"runtime_id": "placeholder", "targets": {"x": {"engine_sha256": "a"}}}
        first = module.canonical_runtime_id(manifest)
        reordered = {"targets": {"x": {"engine_sha256": "a"}}, "runtime_id": "other"}
        self.assertEqual(module.canonical_runtime_id(reordered), first)
        reordered["targets"]["x"]["engine_sha256"] = "b"
        self.assertNotEqual(module.canonical_runtime_id(reordered), first)

    def test_release_public_key_must_match_active_compiled_trust_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            public = root / "release.pub"
            public.write_text("untrusted comment\nRWTESTKEY\n", encoding="utf-8")
            encoded = base64.b64encode(public.read_bytes()).decode()
            trusted = root / "trusted.json"
            trusted.write_text(json.dumps({
                "keys": [{"key_id": "ACTIVE", "pubkey": "", "not_after": "2030-01-01T00:00:00Z"}],
                "revoked_key_ids": [],
            }), encoding="utf-8")
            tauri = root / "tauri.json"
            tauri.write_text(json.dumps({"plugins": {"updater": {"pubkey": encoded}}}), encoding="utf-8")
            expiry = int(datetime(2029, 1, 1, tzinfo=timezone.utc).timestamp())

            module.validate_trust_root("ACTIVE", public, trusted, tauri, expiry)
            public.write_text("untrusted comment\nRWFORGED\n", encoding="utf-8")
            with self.assertRaises(SystemExit):
                module.validate_trust_root("ACTIVE", public, trusted, tauri, expiry)
            with self.assertRaises(SystemExit):
                module.validate_trust_root("UNKNOWN", public, trusted, tauri, expiry)


if __name__ == "__main__":
    unittest.main()
