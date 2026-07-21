#!/usr/bin/env python3
"""Stage one release-qualified Browser Runtime target from pinned inputs."""

from __future__ import annotations

import argparse
import atexit
import hashlib
import json
import os
from pathlib import Path
from pathlib import PurePosixPath
import shutil
import stat
import struct
import tempfile
from urllib.parse import urlsplit
import zipfile


TREE_DOMAIN = b"cys.browser.chromium-tree.v1\0"
TARGETS = {
    "aarch64-apple-darwin": ("arm64", ""),
    "x86_64-apple-darwin": ("x86_64", ""),
    "x86_64-pc-windows-msvc": ("x86_64", ".exe"),
}


def fail(message: str) -> None:
    raise SystemExit(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def hash_tree(root: Path) -> str:
    digest = hashlib.sha256(TREE_DOMAIN)
    files = sorted(path for path in root.rglob("*") if path.is_file())
    for path in files:
        relative = path.relative_to(root).as_posix().encode()
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(path.stat().st_size.to_bytes(8, "big"))
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()


def require_sha256(path: Path, expected: str, label: str) -> None:
    actual = sha256_file(path)
    if actual != expected.lower():
        fail(f"{label} digest mismatch: expected {expected.lower()}, got {actual}")


def require_https_url(value: object, label: str) -> str:
    if not isinstance(value, str):
        fail(f"{label} must be an HTTPS URL")
    parsed = urlsplit(value)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.fragment
    ):
        fail(f"{label} must be an HTTPS URL without credentials or fragment")
    return value


def safe_archive_relative(value: object, label: str) -> PurePosixPath:
    if not isinstance(value, str) or not value or "\\" in value or value.startswith("/"):
        fail(f"{label} is not a safe archive-relative path")
    raw_parts = value.split("/")
    if any(part in ("", ".", "..") for part in raw_parts) or ":" in raw_parts[0]:
        fail(f"{label} is not a safe archive-relative path")
    return PurePosixPath(*raw_parts)


def executable_architecture(path: Path) -> str:
    data = path.read_bytes()[:4096]
    if data[:4] == b"\xcf\xfa\xed\xfe" and len(data) >= 8:
        cpu = struct.unpack("<I", data[4:8])[0]
        if cpu == 0x0100000C:
            return "arm64"
        if cpu == 0x01000007:
            return "x86_64"
    if data[:2] == b"MZ" and len(data) >= 0x40:
        pe_offset = struct.unpack("<I", data[0x3C:0x40])[0]
        if pe_offset + 6 <= len(data) and data[pe_offset : pe_offset + 4] == b"PE\0\0":
            machine = struct.unpack("<H", data[pe_offset + 4 : pe_offset + 6])[0]
            if machine == 0x8664:
                return "x86_64"
    fail(f"unsupported executable format: {path}")


def assert_target(path: Path, expected: str, label: str) -> None:
    if not path.is_file() or path.is_symlink():
        fail(f"{label} is not a regular unlinked file: {path}")
    actual = executable_architecture(path)
    if actual != expected:
        fail(f"{label} target mismatch: expected {expected}, got {actual}")


def runtime_id(manifest: dict) -> str:
    unsigned = dict(manifest)
    unsigned.pop("runtime_id", None)
    unsigned.pop("signature", None)
    unsigned.pop("attestation", None)
    payload = json.dumps(unsigned, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def checked_archive_members(
    archive: zipfile.ZipFile, max_files: int, max_uncompressed_bytes: int
) -> list[zipfile.ZipInfo]:
    members = archive.infolist()
    if max_files <= 0 or max_uncompressed_bytes <= 0:
        fail("Chromium archive limits must be positive")
    if len(members) > max_files:
        fail(f"Chromium archive file count exceeds limit {max_files}")
    total_size = sum(member.file_size for member in members)
    if total_size > max_uncompressed_bytes:
        fail(
            "Chromium archive uncompressed size exceeds limit "
            f"{max_uncompressed_bytes}: {total_size}"
        )
    seen: set[str] = set()
    for member in members:
        name = member.filename
        canonical_name = name[:-1] if name.endswith("/") else name
        raw_parts = canonical_name.split("/")
        if (
            not name
            or not canonical_name
            or "\0" in name
            or "\\" in name
            or name.startswith("/")
            or any(part in ("", ".", "..") for part in raw_parts)
            or ":" in raw_parts[0]
        ):
            fail(f"unsafe Chromium archive path: {name!r}")
        folded = name.rstrip("/").casefold()
        if folded in seen:
            fail(f"duplicate Chromium archive path: {name!r}")
        seen.add(folded)
        mode = (member.external_attr >> 16) & 0o170000
        if mode == stat.S_IFLNK:
            fail(f"Chromium archive entry is a symbolic link: {name!r}")
        if mode not in (0, stat.S_IFDIR, stat.S_IFREG):
            fail(f"Chromium archive entry is not a file or directory: {name!r}")
    return members


def extract_checked(
    archive_path: Path, destination: Path, max_files: int, max_uncompressed_bytes: int
) -> None:
    with zipfile.ZipFile(archive_path) as archive:
        members = checked_archive_members(archive, max_files, max_uncompressed_bytes)
        for member in members:
            output = destination.joinpath(*PurePosixPath(member.filename).parts)
            if member.is_dir():
                output.mkdir(parents=True, exist_ok=True)
                continue
            output.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(member) as source_file, output.open("xb") as output_file:
                shutil.copyfileobj(source_file, output_file)
            mode = (member.external_attr >> 16) & 0o777
            if mode:
                output.chmod(mode)


def stage(args: argparse.Namespace) -> None:
    source = json.loads(args.source_manifest.read_text(encoding="utf-8"))
    if source.get("schema_version") != 1:
        fail("unsupported Browser Runtime source manifest schema")
    if args.target not in TARGETS or args.target not in source.get("targets", {}):
        fail(f"unsupported or unpinned Browser Runtime target: {args.target}")
    if not args.source_revision or len(args.source_revision) != 40 or any(
        character not in "0123456789abcdef" for character in args.source_revision.lower()
    ):
        fail("source revision must be a full hexadecimal Git object id")

    expected_arch, suffix = TARGETS[args.target]
    target_source = source["targets"][args.target]
    if target_source.get("architecture") != expected_arch:
        fail("source manifest architecture does not match target")
    require_https_url(target_source.get("chromium_archive_url"), "Chromium archive provenance")
    require_https_url(source.get("chromium", {}).get("license_url"), "Chromium license provenance")
    assert_target(args.supervisor, expected_arch, "supervisor")
    assert_target(args.engine, expected_arch, "engine")
    expected_archive_bytes = target_source.get("chromium_archive_bytes")
    if not isinstance(expected_archive_bytes, int) or expected_archive_bytes <= 0:
        fail("Chromium archive byte size pin is invalid")
    actual_archive_bytes = args.chromium_archive.stat().st_size
    if actual_archive_bytes != expected_archive_bytes:
        fail(
            "Chromium archive byte size mismatch: "
            f"expected {expected_archive_bytes}, got {actual_archive_bytes}"
        )
    require_sha256(
        args.chromium_archive,
        target_source["chromium_archive_sha256"],
        "Chromium archive",
    )
    require_sha256(
        args.chromium_license,
        source["chromium"]["license_sha256"],
        "Chromium license",
    )

    max_files = target_source["chromium_max_files"]
    max_uncompressed_bytes = target_source["chromium_max_uncompressed_bytes"]
    with zipfile.ZipFile(args.chromium_archive) as archive:
        checked_archive_members(archive, max_files, max_uncompressed_bytes)

    final_output = args.output
    final_output.parent.mkdir(parents=True, exist_ok=True)
    temporary_parent = Path(
        tempfile.mkdtemp(prefix=f".{final_output.name}.stage-", dir=final_output.parent)
    )
    cleanup_pending = True

    def cleanup() -> None:
        if cleanup_pending:
            shutil.rmtree(temporary_parent, ignore_errors=True)

    atexit.register(cleanup)
    output = temporary_parent / "payload"
    target_root = output / "runtime" / args.target
    supervisor_output = target_root / "supervisor" / f"cys-browserd{suffix}"
    engine_output = target_root / "engine" / f"cys-browser-engine{suffix}"
    chromium_root = target_root / "chromium"
    license_root = target_root / "LICENSES"
    for directory in (supervisor_output.parent, engine_output.parent, chromium_root, license_root):
        directory.mkdir(parents=True, exist_ok=True)
    shutil.copy2(args.supervisor, supervisor_output)
    shutil.copy2(args.engine, engine_output)
    if suffix == "":
        supervisor_output.chmod(0o755)
        engine_output.chmod(0o755)
    extract_checked(args.chromium_archive, chromium_root, max_files, max_uncompressed_bytes)

    required_files = target_source.get("chromium_required_files")
    if not isinstance(required_files, list) or not required_files:
        fail("Chromium required-file list is empty")
    for required in required_files:
        relative = safe_archive_relative(required, "Chromium required file")
        required_path = chromium_root.joinpath(*relative.parts)
        if not required_path.is_file() or required_path.is_symlink():
            fail(f"Chromium required file missing: {required}")

    chromium_executable_relative = safe_archive_relative(
        target_source.get("chromium_executable"), "Chromium executable"
    )
    chromium_executable = target_root.joinpath(*chromium_executable_relative.parts)
    if suffix == "" and chromium_executable.is_file():
        chromium_executable.chmod(chromium_executable.stat().st_mode | 0o111)
    assert_target(chromium_executable, expected_arch, "Chromium")

    licenses = {
        "Chromium-LICENSE": args.chromium_license,
        "Playwright-LICENSE": args.playwright_root / "LICENSE",
        "Playwright-NOTICE": args.playwright_root / "NOTICE",
        "Playwright-ThirdPartyNotices.txt": args.playwright_root / "ThirdPartyNotices.txt",
    }
    license_files = []
    expected_playwright = source["playwright"]["license_files"]
    for output_name, input_path in licenses.items():
        expected = (
            source["chromium"]["license_sha256"]
            if output_name == "Chromium-LICENSE"
            else expected_playwright[input_path.name]
        )
        require_sha256(input_path, expected, output_name)
        shutil.copy2(input_path, license_root / output_name)
        license_files.append(f"LICENSES/{output_name}")

    lock = {
        "schema_version": 1,
        "release_ready": True,
        "runtime_id": "sha256:placeholder",
        "source_revision": args.source_revision.lower(),
        "source_manifest_sha256": sha256_file(args.source_manifest),
        "browser_protocol": {
            "major": 2,
            "min_minor": 0,
            "max_minor": 0,
            "capabilities": ["scoped-ticket", "paint-ack"],
            "required_capabilities": ["scoped-ticket"],
        },
        "supervisor": {
            "build_id": source["supervisor"]["build_id"],
            "rust_toolchain": source["toolchains"]["rust"],
        },
        "engine": {
            "build_id": source["engine"]["build_id"],
            "bun_version": source["toolchains"]["bun"],
        },
        "playwright": {"version": source["playwright"]["version"]},
        "chromium": {
            key: source["chromium"][key]
            for key in ("revision", "version", "major", "profile_schema_epoch", "license")
        },
        "targets": {
            args.target: {
                "architecture": expected_arch,
                "supervisor_sha256": sha256_file(supervisor_output),
                "engine_sha256": sha256_file(engine_output),
                "chromium_archive_url": target_source["chromium_archive_url"],
                "chromium_archive_sha256": target_source["chromium_archive_sha256"],
                "chromium_tree_sha256": hash_tree(chromium_root),
                "chromium_executable": target_source["chromium_executable"],
                "license_files": license_files,
            }
        },
    }
    lock["runtime_id"] = runtime_id(lock)
    output.mkdir(parents=True, exist_ok=True)
    (output / "browser-runtime.lock.json").write_text(
        json.dumps(lock, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    if final_output.exists():
        existing = {path.name for path in final_output.iterdir()}
        if existing - {".gitkeep"}:
            fail(f"Browser Runtime output is not empty: {final_output}")
        shutil.rmtree(final_output)
    output.replace(final_output)
    cleanup_pending = False
    shutil.rmtree(temporary_parent, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-manifest", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--chromium-archive", type=Path, required=True)
    parser.add_argument("--chromium-license", type=Path, required=True)
    parser.add_argument("--playwright-root", type=Path, required=True)
    parser.add_argument("--source-revision", required=True)
    parser.add_argument("--output", type=Path, required=True)
    stage(parser.parse_args())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
