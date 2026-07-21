import os
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts" / "release-credentials-check.py"


class ReleaseCredentialsTests(unittest.TestCase):
    def test_windows_unsigned_release_needs_no_authenticode_credentials(self) -> None:
        clean_env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("WINDOWS_")
            and not key.startswith("TAURI_SIGNING_")
            and not key.startswith("CYS_BROWSER_RUNTIME_")
        }
        clean_env.update({
            "CYS_WINDOWS_RELEASE_MODE": "unsigned-acknowledged",
            "TAURI_SIGNING_PRIVATE_KEY": "updater-key",
            "CYS_BROWSER_RUNTIME_SECRET_KEY": "/private/runtime.key",
            "CYS_BROWSER_RUNTIME_PUBLIC_KEY": "/private/runtime.pub",
            "CYS_BROWSER_RUNTIME_KEY_ID": "39E60A702949D6C3",
            "CYS_BROWSER_RUNTIME_POLICY_EPOCH": "1",
            "CYS_BROWSER_RUNTIME_EXPIRES_AT": "1893455999",
        })

        result = subprocess.run(
            ["python3", str(CHECKER), "windows"],
            cwd=ROOT,
            env=clean_env,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("WINDOWS_CERTIFICATE", result.stdout + result.stderr)
        self.assertNotIn("WINDOWS_EXPECTED_PUBLISHER", result.stdout + result.stderr)

    def test_windows_unsigned_release_requires_exact_operator_acknowledgement(self) -> None:
        env = {
            **os.environ,
            "CYS_WINDOWS_RELEASE_MODE": "unsigned",
            "TAURI_SIGNING_PRIVATE_KEY": "updater-key",
            "CYS_BROWSER_RUNTIME_SECRET_KEY": "/private/runtime.key",
            "CYS_BROWSER_RUNTIME_PUBLIC_KEY": "/private/runtime.pub",
            "CYS_BROWSER_RUNTIME_KEY_ID": "39E60A702949D6C3",
            "CYS_BROWSER_RUNTIME_POLICY_EPOCH": "1",
            "CYS_BROWSER_RUNTIME_EXPIRES_AT": "1893455999",
        }

        result = subprocess.run(
            ["python3", str(CHECKER), "windows"],
            cwd=ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("CYS_WINDOWS_RELEASE_MODE", result.stderr)

    def test_macos_release_fails_closed_when_notary_or_signing_inputs_are_missing(self) -> None:
        clean_env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("APPLE_")
            and not key.startswith("TAURI_SIGNING_")
            and not key.startswith("CYS_BROWSER_RUNTIME_")
        }

        result = subprocess.run(
            ["python3", str(CHECKER), "macos"],
            cwd=ROOT,
            env=clean_env,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("APPLE_CERTIFICATE_B64", result.stderr)
        self.assertIn("APPLE_SIGNING_IDENTITY", result.stderr)
        self.assertIn("APPLE_PASSWORD", result.stderr)
        self.assertIn("CYS_BROWSER_RUNTIME_PUBLIC_KEY", result.stderr)

    def test_satisfied_contract_never_prints_secret_values(self) -> None:
        redaction_sentinel = "redaction-sentinel-value"
        env = {
            **os.environ,
            "CYS_WINDOWS_RELEASE_MODE": "unsigned-acknowledged",
            "TAURI_SIGNING_PRIVATE_KEY": redaction_sentinel,
            "CYS_BROWSER_RUNTIME_SECRET_KEY": "/private/runtime.key",
            "CYS_BROWSER_RUNTIME_PUBLIC_KEY": "/private/runtime.pub",
            "CYS_BROWSER_RUNTIME_KEY_ID": "39E60A702949D6C3",
            "CYS_BROWSER_RUNTIME_POLICY_EPOCH": "1",
            "CYS_BROWSER_RUNTIME_EXPIRES_AT": "1893455999",
        }

        result = subprocess.run(
            ["python3", str(CHECKER), "windows"],
            cwd=ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn(redaction_sentinel, result.stdout + result.stderr)
        self.assertIn("values redacted", result.stdout)

    def test_windows_unsigned_release_fails_closed_without_operator_acknowledgement(self) -> None:
        env = {
            **os.environ,
            "TAURI_SIGNING_PRIVATE_KEY": "present",
            "CYS_BROWSER_RUNTIME_SECRET_KEY": "/private/runtime.key",
            "CYS_BROWSER_RUNTIME_PUBLIC_KEY": "/private/runtime.pub",
            "CYS_BROWSER_RUNTIME_KEY_ID": "39E60A702949D6C3",
            "CYS_BROWSER_RUNTIME_POLICY_EPOCH": "1",
            "CYS_BROWSER_RUNTIME_EXPIRES_AT": "1893455999",
        }

        result = subprocess.run(
            ["python3", str(CHECKER), "windows"],
            cwd=ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("CYS_WINDOWS_RELEASE_MODE", result.stderr)


if __name__ == "__main__":
    unittest.main()
