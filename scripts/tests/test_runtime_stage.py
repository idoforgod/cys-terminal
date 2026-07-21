import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
import zipfile


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "runtime-stage.py"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def macho(architecture: str) -> bytes:
    cpu = {"arm64": 0x0100000C, "x86_64": 0x01000007}[architecture]
    return struct.pack("<IIIIIIII", 0xFEEDFACF, cpu, 0, 2, 0, 0, 0, 0) + b"fixture"


class RuntimeStageTests(unittest.TestCase):
    def fixture(self, directory: str, target: str = "aarch64-apple-darwin") -> dict[str, Path]:
        root = Path(directory)
        architecture = "arm64" if target.startswith("aarch64") else "x86_64"
        supervisor = root / "supervisor"
        engine = root / "engine"
        supervisor.write_bytes(macho(architecture))
        engine.write_bytes(macho(architecture))
        supervisor.chmod(0o755)
        engine.chmod(0o755)

        archive = root / "chromium.zip"
        executable = "chrome-mac/Chromium.app/Contents/MacOS/Chromium"
        with zipfile.ZipFile(archive, "w") as bundle:
            info = zipfile.ZipInfo(executable)
            info.external_attr = 0o100755 << 16
            bundle.writestr(info, macho(architecture))
            bundle.writestr("chrome-mac/resources.pak", b"resource")

        chromium_license = root / "chromium-LICENSE"
        chromium_license.write_text("Chromium fixture license\n", encoding="utf-8")
        playwright = root / "playwright"
        playwright.mkdir()
        for name in ("LICENSE", "NOTICE", "ThirdPartyNotices.txt"):
            (playwright / name).write_text(f"Playwright {name}\n", encoding="utf-8")

        source = {
            "schema_version": 1,
            "toolchains": {"rust": "1.95.0", "bun": "1.3.8"},
            "supervisor": {"build_id": "cys-browserd-0.1.0"},
            "engine": {"build_id": "cys-browser-engine-0.1.0"},
            "playwright": {
                "version": "1.49.1",
                "license_files": {
                    name: sha256(playwright / name)
                    for name in ("LICENSE", "NOTICE", "ThirdPartyNotices.txt")
                },
            },
            "chromium": {
                "revision": "1148",
                "version": "131.0.6778.33",
                "major": 131,
                "profile_schema_epoch": 1,
                "license": "Chromium-BSD",
                "license_url": "https://example.invalid/chromium-LICENSE",
                "license_sha256": sha256(chromium_license),
            },
            "targets": {
                target: {
                    "architecture": architecture,
                    "chromium_archive_url": "https://example.invalid/chromium.zip",
                    "chromium_archive_sha256": sha256(archive),
                    "chromium_archive_bytes": archive.stat().st_size,
                    "chromium_max_files": 20,
                    "chromium_max_uncompressed_bytes": 4096,
                    "chromium_executable": f"chromium/{executable}",
                    "chromium_required_files": [
                        executable,
                        "chrome-mac/resources.pak",
                    ],
                }
            },
        }
        source_path = root / "sources.json"
        source_path.write_text(json.dumps(source), encoding="utf-8")
        return {
            "root": root,
            "source": source_path,
            "supervisor": supervisor,
            "engine": engine,
            "archive": archive,
            "chromium_license": chromium_license,
            "playwright": playwright,
        }

    def run_stage(self, fixture: dict[str, Path], output: Path, target: str = "aarch64-apple-darwin"):
        return subprocess.run(
            [
                "python3",
                str(SCRIPT),
                "--source-manifest",
                str(fixture["source"]),
                "--target",
                target,
                "--supervisor",
                str(fixture["supervisor"]),
                "--engine",
                str(fixture["engine"]),
                "--chromium-archive",
                str(fixture["archive"]),
                "--chromium-license",
                str(fixture["chromium_license"]),
                "--playwright-root",
                str(fixture["playwright"]),
                "--source-revision",
                "a" * 40,
                "--output",
                str(output),
            ],
            text=True,
            capture_output=True,
        )

    def refresh_archive_pin(self, fixture: dict[str, Path], target: str = "aarch64-apple-darwin"):
        source = json.loads(fixture["source"].read_text(encoding="utf-8"))
        assets = source["targets"][target]
        assets["chromium_archive_sha256"] = sha256(fixture["archive"])
        assets["chromium_archive_bytes"] = fixture["archive"].stat().st_size
        fixture["source"].write_text(json.dumps(source), encoding="utf-8")

    def test_stages_a_digest_bound_runtime_through_the_public_cli(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertEqual(result.returncode, 0, result.stderr)
            target = output / "runtime" / "aarch64-apple-darwin"
            supervisor = target / "supervisor" / "cys-browserd"
            engine = target / "engine" / "cys-browser-engine"
            chromium = target / "chromium/chrome-mac/Chromium.app/Contents/MacOS/Chromium"
            self.assertTrue(os.access(supervisor, os.X_OK))
            self.assertTrue(os.access(engine, os.X_OK))
            self.assertTrue(os.access(chromium, os.X_OK))
            lock = json.loads((output / "browser-runtime.lock.json").read_text(encoding="utf-8"))
            assets = lock["targets"]["aarch64-apple-darwin"]
            self.assertEqual(assets["supervisor_sha256"], sha256(supervisor))
            self.assertEqual(assets["engine_sha256"], sha256(engine))
            self.assertEqual(assets["chromium_archive_sha256"], sha256(fixture["archive"]))
            self.assertEqual(lock["source_revision"], "a" * 40)
            self.assertEqual(
                set(assets["license_files"]),
                {
                    "LICENSES/Chromium-LICENSE",
                    "LICENSES/Playwright-LICENSE",
                    "LICENSES/Playwright-NOTICE",
                    "LICENSES/Playwright-ThirdPartyNotices.txt",
                },
            )
            for relative in assets["license_files"]:
                self.assertTrue((target / relative).is_file())

    def test_rejects_archive_path_traversal_without_partial_output(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            with zipfile.ZipFile(fixture["archive"], "a") as bundle:
                bundle.writestr("../../outside", b"escaped")
            source = json.loads(fixture["source"].read_text(encoding="utf-8"))
            source["targets"]["aarch64-apple-darwin"]["chromium_archive_sha256"] = sha256(
                fixture["archive"]
            )
            source["targets"]["aarch64-apple-darwin"]["chromium_archive_bytes"] = fixture[
                "archive"
            ].stat().st_size
            fixture["source"].write_text(json.dumps(source), encoding="utf-8")
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unsafe Chromium archive path", result.stderr)
            self.assertFalse(output.exists())
            self.assertFalse((fixture["root"] / "outside").exists())

    def test_rejects_noncanonical_archive_dot_segments(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            with zipfile.ZipFile(fixture["archive"], "a") as bundle:
                bundle.writestr("chrome-mac/./alias.pak", b"alias")
            self.refresh_archive_pin(fixture)
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unsafe Chromium archive path", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_a_manifest_executable_path_outside_the_target_tree(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            source = json.loads(fixture["source"].read_text(encoding="utf-8"))
            source["targets"]["aarch64-apple-darwin"]["chromium_executable"] = str(
                fixture["supervisor"]
            )
            fixture["source"].write_text(json.dumps(source), encoding="utf-8")
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Chromium executable is not a safe archive-relative path", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_archive_expansion_beyond_manifest_limits(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            with zipfile.ZipFile(fixture["archive"], "a") as bundle:
                bundle.writestr("chrome-mac/oversized.pak", b"x" * 5000)
            source = json.loads(fixture["source"].read_text(encoding="utf-8"))
            target = source["targets"]["aarch64-apple-darwin"]
            target["chromium_archive_sha256"] = sha256(fixture["archive"])
            target["chromium_archive_bytes"] = fixture["archive"].stat().st_size
            fixture["source"].write_text(json.dumps(source), encoding="utf-8")
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("uncompressed size exceeds", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_target_skew_atomically(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            executable = "chrome-mac/Chromium.app/Contents/MacOS/Chromium"
            with zipfile.ZipFile(fixture["archive"], "w") as bundle:
                info = zipfile.ZipInfo(executable)
                info.external_attr = 0o100755 << 16
                bundle.writestr(info, macho("x86_64"))
                bundle.writestr("chrome-mac/resources.pak", b"resource")
            source = json.loads(fixture["source"].read_text(encoding="utf-8"))
            target = source["targets"]["aarch64-apple-darwin"]
            target["chromium_archive_sha256"] = sha256(fixture["archive"])
            target["chromium_archive_bytes"] = fixture["archive"].stat().st_size
            fixture["source"].write_text(json.dumps(source), encoding="utf-8")
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Chromium target mismatch", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_manifest_archive_size_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            source = json.loads(fixture["source"].read_text(encoding="utf-8"))
            source["targets"]["aarch64-apple-darwin"]["chromium_archive_bytes"] += 1
            fixture["source"].write_text(json.dumps(source), encoding="utf-8")
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("archive byte size mismatch", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_same_size_archive_digest_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            original = fixture["archive"].read_bytes()
            tampered = bytearray(original)
            tampered[-1] ^= 0x01
            fixture["archive"].write_bytes(tampered)
            self.assertEqual(fixture["archive"].stat().st_size, len(original))
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Chromium archive digest mismatch", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_non_https_source_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            source = json.loads(fixture["source"].read_text(encoding="utf-8"))
            source["targets"]["aarch64-apple-darwin"][
                "chromium_archive_url"
            ] = "http://mirror.invalid/chromium.zip"
            fixture["source"].write_text(json.dumps(source), encoding="utf-8")
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("HTTPS", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_an_archive_missing_a_manifest_required_file(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            source = json.loads(fixture["source"].read_text(encoding="utf-8"))
            source["targets"]["aarch64-apple-darwin"]["chromium_required_files"].append(
                "chrome-mac/icudtl.dat"
            )
            fixture["source"].write_text(json.dumps(source), encoding="utf-8")
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("required file missing", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_an_archive_symlink_even_when_it_stays_internal(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            with zipfile.ZipFile(fixture["archive"], "a") as bundle:
                bundle.writestr("chrome-mac/Framework/Versions/1/resource.pak", b"linked")
                link = zipfile.ZipInfo("chrome-mac/Framework/Resources")
                link.create_system = 3
                link.external_attr = 0o120777 << 16
                bundle.writestr(link, "Versions/1")
            source = json.loads(fixture["source"].read_text(encoding="utf-8"))
            target = source["targets"]["aarch64-apple-darwin"]
            target["chromium_archive_sha256"] = sha256(fixture["archive"])
            target["chromium_archive_bytes"] = fixture["archive"].stat().st_size
            fixture["source"].write_text(json.dumps(source), encoding="utf-8")
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("symbolic link", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_an_archive_link_that_escapes_its_parent(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            with zipfile.ZipFile(fixture["archive"], "a") as bundle:
                link = zipfile.ZipInfo("chrome-mac/Framework/escape")
                link.create_system = 3
                link.external_attr = 0o120777 << 16
                bundle.writestr(link, "../outside")
            self.refresh_archive_pin(fixture)
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("symbolic link", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_a_dangling_archive_link(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            with zipfile.ZipFile(fixture["archive"], "a") as bundle:
                link = zipfile.ZipInfo("chrome-mac/Framework/Missing")
                link.create_system = 3
                link.external_attr = 0o120777 << 16
                bundle.writestr(link, "does-not-exist")
            self.refresh_archive_pin(fixture)
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("symbolic link", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_an_archive_link_cycle(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            with zipfile.ZipFile(fixture["archive"], "a") as bundle:
                for name, target in (("A", "B"), ("B", "A")):
                    link = zipfile.ZipInfo(f"chrome-mac/Framework/{name}")
                    link.create_system = 3
                    link.external_attr = 0o120777 << 16
                    bundle.writestr(link, target)
            self.refresh_archive_pin(fixture)
            output = fixture["root"] / "output"

            result = self.run_stage(fixture, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("symbolic link", result.stderr)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
