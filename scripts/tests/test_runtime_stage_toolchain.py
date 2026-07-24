import ast
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
import zipfile


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "runtime-stage-toolchain.py"


def _call_name(func):
    if isinstance(func, ast.Attribute):
        return func.attr
    if isinstance(func, ast.Name):
        return func.id
    return None


class RuntimeStageToolchainTests(unittest.TestCase):
    def fixture(self, directory: str) -> tuple[Path, Path, bytes]:
        root = Path(directory)
        archive = root / "bun.zip"
        payload = struct.pack("<IIIIIIII", 0xFEEDFACF, 0x0100000C, 0, 2, 0, 0, 0, 0)
        with zipfile.ZipFile(archive, "w") as bundle:
            bundle.writestr("bun-darwin-aarch64/", b"")
            bundle.writestr("bun-darwin-aarch64/bun", payload)
        source = root / "sources.json"
        source.write_text(json.dumps({
            "schema_version": 1,
            "targets": {"aarch64-apple-darwin": {
                "architecture": "arm64",
                "bun_compiler_archive_url": "https://example.invalid/bun.zip",
                "bun_compiler_archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
                "bun_compiler_archive_bytes": archive.stat().st_size,
                "bun_compiler_member": "bun-darwin-aarch64/bun",
            }},
        }), encoding="utf-8")
        return archive, source, payload

    def run_extract(self, archive: Path, source: Path, output: Path):
        return subprocess.run([
            "python3", str(SCRIPT),
            "--source-manifest", str(source),
            "--target", "aarch64-apple-darwin",
            "--archive", str(archive),
            "--output", str(output),
        ], text=True, capture_output=True)

    def test_extracts_only_the_digest_pinned_target_compiler(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive, source, payload = self.fixture(directory)
            output = root / "compiler"

            result = self.run_extract(archive, source, output)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_bytes(), payload)
            self.assertTrue(output.stat().st_mode & 0o111)

    def test_rejects_same_size_compiler_archive_digest_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive, source, _ = self.fixture(directory)
            original = archive.read_bytes()
            tampered = bytearray(original)
            tampered[-1] ^= 0x01
            archive.write_bytes(tampered)
            output = root / "compiler"

            result = self.run_extract(archive, source, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Bun compiler archive digest mismatch", result.stderr)
            self.assertFalse(output.exists())

    def test_rejects_a_compiler_symlink_even_when_the_archive_pin_matches(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive, source, _ = self.fixture(directory)
            with zipfile.ZipFile(archive, "w") as bundle:
                bundle.writestr("bun-darwin-aarch64/", b"")
                link = zipfile.ZipInfo("bun-darwin-aarch64/bun")
                link.create_system = 3
                link.external_attr = 0o120777 << 16
                bundle.writestr(link, "../host-bun")
            manifest = json.loads(source.read_text(encoding="utf-8"))
            assets = manifest["targets"]["aarch64-apple-darwin"]
            assets["bun_compiler_archive_sha256"] = hashlib.sha256(archive.read_bytes()).hexdigest()
            assets["bun_compiler_archive_bytes"] = archive.stat().st_size
            source.write_text(json.dumps(manifest), encoding="utf-8")
            output = root / "compiler"

            result = self.run_extract(archive, source, output)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("not a regular file", result.stderr)
            self.assertFalse(output.exists())


class ToolchainHandleClosedBeforeReplaceTests(unittest.TestCase):
    """Source-level seal for the 5th-run Windows failure (unreproducible on macOS).

    The atomic rename previously ran INSIDE the NamedTemporaryFile with-block, so the
    write fd was still open and Windows refused the rename with WinError 32 — a
    self-inflicted share-lock that no retry could clear (POSIX allows renaming an open
    fd, which hid the defect on macOS). This asserts, structurally, that no .replace()
    rename lives inside the temp-file with-block; it must run only after the handle
    closes."""

    def _temp_with_node(self):
        tree = ast.parse(SCRIPT.read_text(encoding="utf-8"))
        temp_withs = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.With)
            and any(
                isinstance(item.context_expr, ast.Call)
                and _call_name(item.context_expr.func) == "NamedTemporaryFile"
                for item in node.items
            )
        ]
        self.assertEqual(
            len(temp_withs),
            1,
            "expected exactly one NamedTemporaryFile with-block in the staging script",
        )
        return temp_withs[0]

    def test_no_atomic_replace_runs_while_the_temp_write_handle_is_open(self):
        with_node = self._temp_with_node()
        replaces_inside = [
            node
            for node in ast.walk(with_node)
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and node.func.attr == "replace"
        ]
        self.assertEqual(
            replaces_inside,
            [],
            "temp .replace() must run AFTER the NamedTemporaryFile block closes the "
            "write handle — otherwise Windows fails the rename with WinError 32",
        )

    def test_the_extract_path_still_performs_an_atomic_replace(self):
        # Guard the seal against passing vacuously (e.g. the rename being removed).
        tree = ast.parse(SCRIPT.read_text(encoding="utf-8"))
        replaces = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and node.func.attr == "replace"
        ]
        self.assertTrue(
            replaces, "extract() must still atomically replace the output via .replace()"
        )


if __name__ == "__main__":
    unittest.main()
