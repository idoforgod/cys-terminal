import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HANDOFF = ROOT / "scripts" / "release-handoff.sh"


class ReleaseHandoffTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.work = Path(self.temporary.name)
        self.source = self.work / "source"
        self.source.mkdir()
        (self.source / "candidate.bin").write_bytes(b"signed-release-candidate")
        self.encrypted = self.work / "candidate.tgz.enc"
        self.unpacked = self.work / "unpacked"

    def run_handoff(
        self, *arguments: str, key: str = "x" * 48
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(HANDOFF), *arguments],
            cwd=ROOT,
            env={**os.environ, "RELEASE_HANDOFF_KEY": key},
            text=True,
            capture_output=True,
            check=False,
        )

    def test_encrypted_handoff_round_trip_never_stores_plaintext_in_artifact(self) -> None:
        packed = self.run_handoff("pack", str(self.source), str(self.encrypted))
        self.assertEqual(packed.returncode, 0, packed.stderr)
        self.assertNotIn(b"signed-release-candidate", self.encrypted.read_bytes())

        unpacked = self.run_handoff("unpack", str(self.encrypted), str(self.unpacked))
        self.assertEqual(unpacked.returncode, 0, unpacked.stderr)
        self.assertEqual(
            (self.unpacked / "candidate.bin").read_bytes(),
            b"signed-release-candidate",
        )

    def test_missing_short_or_wrong_key_fails_closed(self) -> None:
        missing = subprocess.run(
            ["bash", str(HANDOFF), "pack", str(self.source), str(self.encrypted)],
            cwd=ROOT,
            env={name: value for name, value in os.environ.items() if name != "RELEASE_HANDOFF_KEY"},
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertNotEqual(missing.returncode, 0)

        short = self.run_handoff("pack", str(self.source), str(self.encrypted), key="too-short")
        self.assertNotEqual(short.returncode, 0)

        packed = self.run_handoff("pack", str(self.source), str(self.encrypted))
        self.assertEqual(packed.returncode, 0, packed.stderr)
        wrong = self.run_handoff(
            "unpack", str(self.encrypted), str(self.unpacked), key="y" * 48
        )
        self.assertNotEqual(wrong.returncode, 0)
        self.assertFalse(self.unpacked.exists())


if __name__ == "__main__":
    unittest.main()
