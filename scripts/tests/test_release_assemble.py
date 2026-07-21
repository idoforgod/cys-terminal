import hashlib
import json
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
ASSEMBLER = ROOT / "scripts" / "release-assemble.py"
VERIFIER = ROOT / "scripts" / "release-verify.py"
ASSETS_POLICY = ROOT / "release" / "assets-policy.json"


class ReleaseAssembleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.addCleanup(self.tempdir.cleanup)
        self.work = Path(self.tempdir.name)
        self.inputs = self.work / "inputs"
        self.output = self.work / "output"
        self.inputs.mkdir()

    def write_candidate_inputs(self, version: str) -> None:
        payloads = {
            f"cys_{version}_aarch64.dmg": b"arm-dmg",
            f"cys_{version}_x64.dmg": b"intel-dmg",
            f"cys_{version}_x64-setup.exe": b"signed-windows-installer",
            f"cys_{version}_aarch64.app.tar.gz": b"arm-updater",
            f"cys_{version}_aarch64.app.tar.gz.sig": b"arm-signature",
            f"cys_{version}_x64.app.tar.gz": b"intel-updater",
            f"cys_{version}_x64.app.tar.gz.sig": b"intel-signature",
            f"cys_{version}_x64-setup.exe.sig": b"windows-updater-signature",
            "pack.tar.gz": b"pack",
            "pack-manifest.json": b"{}\n",
            "pack-manifest.json.minisig": b"pack-signature",
        }
        for name, content in payloads.items():
            (self.inputs / name).write_bytes(content)

    def run_assembler(
        self,
        version: str,
        *,
        site_output: Path | None = None,
        policy: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        command = [
                "python3",
                str(ASSEMBLER),
                "--version",
                version,
                "--input",
                str(self.inputs),
                "--output",
                str(self.output),
                "--repository",
                "idoforgod/cys-terminal",
                "--pub-date",
                "2026-07-21T00:00:00Z",
            ]
        if site_output is not None:
            command.extend(["--site-output", str(site_output)])
        if policy is not None:
            command.extend(["--policy", str(policy)])
        return subprocess.run(
            command,
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_valid_candidate_becomes_exact_release_bundle(self) -> None:
        version = "9.8.7"
        self.write_candidate_inputs(version)

        result = self.run_assembler(version)

        self.assertEqual(result.returncode, 0, result.stderr)
        expected = {
            f"cys_{version}_aarch64.dmg",
            f"cys_{version}_x64.dmg",
            f"cys_{version}_x64-setup.exe",
            f"cys_{version}_x64-setup.zip",
            f"cys_{version}_aarch64.app.tar.gz",
            f"cys_{version}_aarch64.app.tar.gz.sig",
            f"cys_{version}_x64.app.tar.gz",
            f"cys_{version}_x64.app.tar.gz.sig",
            f"cys_{version}_x64-setup.exe.sig",
            "latest.json",
            "pack.tar.gz",
            "pack-manifest.json",
            "pack-manifest.json.minisig",
            "SHA256SUMS.txt",
        }
        self.assertEqual({path.name for path in self.output.iterdir()}, expected)

        zip_path = self.output / f"cys_{version}_x64-setup.zip"
        with zipfile.ZipFile(zip_path) as archive:
            self.assertEqual(archive.namelist(), [f"cys_{version}_x64-setup.exe"])
            self.assertEqual(
                archive.read(f"cys_{version}_x64-setup.exe"),
                (self.output / f"cys_{version}_x64-setup.exe").read_bytes(),
            )

        latest = json.loads((self.output / "latest.json").read_text())
        self.assertEqual(latest["version"], version)
        self.assertEqual(latest["pub_date"], "2026-07-21T00:00:00Z")
        self.assertEqual(
            set(latest["platforms"]),
            {"darwin-aarch64", "darwin-x86_64", "windows-x86_64"},
        )

        checksum_rows = (self.output / "SHA256SUMS.txt").read_text().splitlines()
        self.assertEqual(len(checksum_rows), len(expected) - 1)
        checksummed_names = {row.split("  ", 1)[1] for row in checksum_rows}
        self.assertEqual(checksummed_names, expected - {"SHA256SUMS.txt"})
        for row in checksum_rows:
            expected_hash, filename = row.split("  ", 1)
            self.assertEqual(
                expected_hash,
                hashlib.sha256((self.output / filename).read_bytes()).hexdigest(),
            )

    def test_policy_explicitly_records_unsigned_windows_distribution(self) -> None:
        policy = json.loads(ASSETS_POLICY.read_text(encoding="utf-8"))

        self.assertEqual(policy["windows_release_mode"], "unsigned")

    def test_assembler_rejects_a_windows_release_mode_policy_drift(self) -> None:
        version = "9.8.7"
        self.write_candidate_inputs(version)
        policy = json.loads(ASSETS_POLICY.read_text(encoding="utf-8"))
        policy["windows_release_mode"] = "signed"
        policy_path = self.work / "drifted-policy.json"
        policy_path.write_text(json.dumps(policy), encoding="utf-8")

        result = self.run_assembler(version, policy=policy_path)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Windows release mode must be unsigned", result.stderr)

    def test_target_handoffs_may_be_nested_but_are_assembled_by_asset_name(self) -> None:
        version = "9.8.7"
        self.write_candidate_inputs(version)
        handoffs = [self.inputs / "mac-arm", self.inputs / "mac-intel", self.inputs / "windows"]
        for directory in handoffs:
            directory.mkdir()
        for index, path in enumerate(list(self.inputs.iterdir())):
            if path.is_file():
                path.rename(handoffs[index % len(handoffs)] / path.name)

        result = self.run_assembler(version)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.output / f"cys_{version}_x64-setup.zip").is_file())

    def test_site_bundle_has_download_recovery_and_defender_contracts(self) -> None:
        version = "9.8.7"
        site = self.work / "site"
        self.write_candidate_inputs(version)

        result = self.run_assembler(version, site_output=site)

        self.assertEqual(result.returncode, 0, result.stderr)
        downloads = site / "downloads"
        index = (downloads / "index.html").read_text(encoding="utf-8")
        recovery = (downloads / "recovery" / "index.html").read_text(encoding="utf-8")
        self.assertIn('data-cys-release-marker="windows-defender-guidance-v1"', index)
        self.assertIn("Windows Defender", index)
        self.assertIn("Authenticode 서명되지 않았습니다", index)
        self.assertIn("SmartScreen", index)
        self.assertIn("ZIP", index)
        self.assertIn("SHA256SUMS.txt", index)
        self.assertIn("보호 기능을 끄지 마세요", index)
        self.assertNotIn("xattr -d", index)
        self.assertIn('data-cys-release-marker="release-recovery-v1"', recovery)
        for filename in (
            f"cys_{version}_aarch64.dmg",
            f"cys_{version}_x64.dmg",
            f"cys_{version}_x64-setup.exe",
            f"cys_{version}_x64-setup.zip",
        ):
            self.assertIn(filename, index)
        release_assets = {path.name for path in self.output.iterdir()}
        hosted_assets = {
            path.name
            for path in downloads.iterdir()
            if path.is_file() and path.name != "index.html"
        }
        self.assertEqual(hosted_assets, release_assets)

    def test_missing_or_extra_candidate_asset_fails_closed(self) -> None:
        version = "9.8.7"
        self.write_candidate_inputs(version)
        (self.inputs / "pack.tar.gz").unlink()
        (self.inputs / "unexpected.bin").write_bytes(b"unexpected")

        result = self.run_assembler(version)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing=['pack.tar.gz']", result.stderr)
        self.assertIn("extra=['unexpected.bin']", result.stderr)
        self.assertFalse(self.output.exists())

    def test_duplicate_target_asset_name_fails_closed(self) -> None:
        version = "9.8.7"
        self.write_candidate_inputs(version)
        duplicate_dir = self.inputs / "other-target"
        duplicate_dir.mkdir()
        (duplicate_dir / "pack.tar.gz").write_bytes(b"different-pack")

        result = self.run_assembler(version)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("duplicate asset names: ['pack.tar.gz']", result.stderr)
        self.assertFalse(self.output.exists())

    def test_post_assembly_tampering_is_rejected(self) -> None:
        version = "9.8.7"
        self.write_candidate_inputs(version)
        assembled = self.run_assembler(version)
        self.assertEqual(assembled.returncode, 0, assembled.stderr)
        (self.output / f"cys_{version}_aarch64.dmg").write_bytes(b"tampered")

        verified = subprocess.run(
            [
                "python3",
                str(VERIFIER),
                "--version",
                version,
                "--release-dir",
                str(self.output),
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertNotEqual(verified.returncode, 0)
        self.assertIn("checksum mismatch", verified.stderr)

    def test_checksum_consistent_updater_redirect_is_rejected(self) -> None:
        version = "9.8.7"
        self.write_candidate_inputs(version)
        assembled = self.run_assembler(version)
        self.assertEqual(assembled.returncode, 0, assembled.stderr)

        latest_path = self.output / "latest.json"
        latest = json.loads(latest_path.read_text(encoding="utf-8"))
        latest["platforms"]["darwin-aarch64"]["url"] = (
            "https://attacker.invalid/cys.app.tar.gz"
        )
        latest_path.write_text(
            json.dumps(latest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        checksum_path = self.output / "SHA256SUMS.txt"
        rows = checksum_path.read_text(encoding="utf-8").splitlines()
        new_digest = hashlib.sha256(latest_path.read_bytes()).hexdigest()
        checksum_path.write_text(
            "\n".join(
                f"{new_digest}  latest.json" if row.endswith("  latest.json") else row
                for row in rows
            )
            + "\n",
            encoding="utf-8",
        )

        verified = subprocess.run(
            [
                "python3",
                str(VERIFIER),
                "--version",
                version,
                "--release-dir",
                str(self.output),
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertNotEqual(verified.returncode, 0)
        self.assertIn("updater URL mismatch", verified.stderr)


if __name__ == "__main__":
    unittest.main()
