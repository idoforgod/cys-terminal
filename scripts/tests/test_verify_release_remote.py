import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
ASSEMBLER = ROOT / "scripts" / "release-assemble.py"
REMOTE_VERIFIER = ROOT / "scripts" / "verify-release-remote.sh"


class VerifyReleaseRemoteTests(unittest.TestCase):
    def setUp(self) -> None:
        self.version = "9.8.7"
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.work = Path(self.temporary.name)
        self.inputs = self.work / "inputs"
        self.release = self.work / "release"
        self.site = self.work / "site"
        self.inputs.mkdir()
        payloads = {
            f"cys_{self.version}_aarch64.dmg": b"arm-dmg",
            f"cys_{self.version}_x64.dmg": b"intel-dmg",
            f"cys_{self.version}_x64-setup.exe": b"signed-windows-installer",
            f"cys_{self.version}_aarch64.app.tar.gz": b"arm-updater",
            f"cys_{self.version}_aarch64.app.tar.gz.sig": b"arm-signature",
            f"cys_{self.version}_x64.app.tar.gz": b"intel-updater",
            f"cys_{self.version}_x64.app.tar.gz.sig": b"intel-signature",
            f"cys_{self.version}_x64-setup.exe.sig": b"windows-updater-signature",
            "pack.tar.gz": b"pack",
            "pack-manifest.json": b"{}\n",
            "pack-manifest.json.minisig": b"pack-signature",
        }
        for name, content in payloads.items():
            (self.inputs / name).write_bytes(content)
        assembled = subprocess.run(
            [
                "python3",
                str(ASSEMBLER),
                "--version",
                self.version,
                "--input",
                str(self.inputs),
                "--output",
                str(self.release),
                "--site-output",
                str(self.site),
                "--repository",
                "idoforgod/cys-terminal",
                "--pub-date",
                "2026-07-21T00:00:00Z",
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(assembled.returncode, 0, assembled.stderr)

    def run_verifier(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                "bash",
                str(REMOTE_VERIFIER),
                "--version",
                self.version,
                "--release-dir",
                str(self.release),
                "--site-root",
                self.site.as_uri(),
            ],
            cwd=ROOT,
            env={**os.environ, "CYS_RELEASE_VERIFY_ALLOW_FILE": "1"},
            text=True,
            capture_output=True,
            check=False,
        )

    def test_complete_remote_site_is_verified_byte_for_byte(self) -> None:
        result = self.run_verifier()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("remote release verified", result.stdout)

    def test_missing_defender_marker_blocks_remote_promotion(self) -> None:
        index = self.site / "downloads" / "index.html"
        index.write_text(
            index.read_text(encoding="utf-8").replace(
                'data-cys-release-marker="windows-defender-guidance-v1"', ""
            ),
            encoding="utf-8",
        )

        result = self.run_verifier()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing the Defender marker", result.stderr)

    def test_remote_byte_tampering_blocks_promotion(self) -> None:
        target = self.site / "downloads" / f"cys_{self.version}_x64-setup.exe"
        target.write_bytes(b"tampered")

        result = self.run_verifier()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("remote checksum mismatch", result.stderr)


if __name__ == "__main__":
    unittest.main()
