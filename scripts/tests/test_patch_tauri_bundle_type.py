import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PATCHER = ROOT / "scripts" / "patch-tauri-bundle-type.py"
UNKNOWN = b"__TAURI_BUNDLE_TYPE_VAR_UNK"
NSIS = b"__TAURI_BUNDLE_TYPE_VAR_NSS"


class PatchTauriBundleTypeTests(unittest.TestCase):
    def run_patcher(self, payload: bytes) -> tuple[subprocess.CompletedProcess[str], Path, bytes]:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        executable = Path(temporary.name) / "cys-app.exe"
        executable.write_bytes(payload)
        result = subprocess.run(
            [sys.executable, str(PATCHER), "--executable", str(executable), "--bundle-type", "nsis"],
            text=True,
            capture_output=True,
            check=False,
        )
        return result, executable, executable.read_bytes()

    def test_replaces_the_one_official_tauri_token_byte_for_byte(self) -> None:
        original = b"prefix\x00" + UNKNOWN + b"\x00suffix"

        result, _, patched = self.run_patcher(original)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(patched, original.replace(UNKNOWN, NSIS))
        self.assertEqual(len(patched), len(original))

    def test_missing_token_is_fail_closed_and_leaves_the_binary_unchanged(self) -> None:
        original = b"not-a-tauri-binary"

        result, _, patched = self.run_patcher(original)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected exactly one", result.stderr)
        self.assertEqual(patched, original)

    def test_duplicate_token_is_fail_closed_and_leaves_the_binary_unchanged(self) -> None:
        original = UNKNOWN + b"middle" + UNKNOWN

        result, _, patched = self.run_patcher(original)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected exactly one", result.stderr)
        self.assertEqual(patched, original)


if __name__ == "__main__":
    unittest.main()
