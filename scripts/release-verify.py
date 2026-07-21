#!/usr/bin/env python3
"""Verify a fully assembled release directory without network or credentials."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import zipfile
from datetime import datetime
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = ROOT / "release" / "assets-policy.json"
VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
CHECKSUM_RE = re.compile(r"^([0-9a-f]{64})  ([A-Za-z0-9_.-]+)$")


class VerificationError(ValueError):
    pass


def rendered_policy(path: Path, version: str) -> dict[str, object]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    for field in ("input_assets", "generated_assets", "install_assets"):
        raw[field] = [str(item).format(version=version) for item in raw[field]]
    return raw


def verify_release(version: str, release_dir: Path, policy_path: Path) -> list[str]:
    if not VERSION_RE.fullmatch(version):
        raise VerificationError(f"version must be X.Y.Z, got {version!r}")
    policy = rendered_policy(policy_path, version)
    expected = set(policy["input_assets"]) | set(policy["generated_assets"])
    if not release_dir.is_dir():
        raise VerificationError(f"release directory does not exist: {release_dir}")
    files: dict[str, Path] = {}
    for path in release_dir.iterdir():
        if not path.is_file() or path.is_symlink():
            raise VerificationError(f"non-regular release entry: {path.name}")
        files[path.name] = path
    actual = set(files)
    if actual != expected:
        raise VerificationError(
            f"release set mismatch: missing={sorted(expected - actual)}; "
            f"extra={sorted(actual - expected)}"
        )

    checksum_path = files["SHA256SUMS.txt"]
    rows = checksum_path.read_text(encoding="utf-8").splitlines()
    checksums: dict[str, str] = {}
    for row in rows:
        match = CHECKSUM_RE.fullmatch(row)
        if not match:
            raise VerificationError(f"invalid checksum row: {row!r}")
        digest, filename = match.groups()
        if filename in checksums:
            raise VerificationError(f"duplicate checksum row: {filename}")
        checksums[filename] = digest
    expected_checksums = expected - {"SHA256SUMS.txt"}
    if set(checksums) != expected_checksums:
        raise VerificationError(
            f"checksum set mismatch: missing={sorted(expected_checksums - set(checksums))}; "
            f"extra={sorted(set(checksums) - expected_checksums)}"
        )
    for filename, expected_digest in checksums.items():
        actual_digest = hashlib.sha256(files[filename].read_bytes()).hexdigest()
        if actual_digest != expected_digest:
            raise VerificationError(f"checksum mismatch: {filename}")

    exe_name = f"cys_{version}_x64-setup.exe"
    zip_name = f"cys_{version}_x64-setup.zip"
    with zipfile.ZipFile(files[zip_name]) as archive:
        if archive.namelist() != [exe_name]:
            raise VerificationError(
                f"Windows ZIP must contain one exact root member {exe_name!r}, "
                f"got {archive.namelist()!r}"
            )
        if archive.read(exe_name) != files[exe_name].read_bytes():
            raise VerificationError("Windows ZIP member is not byte-identical to setup EXE")

    latest = json.loads(files["latest.json"].read_text(encoding="utf-8"))
    if not isinstance(latest, dict):
        raise VerificationError("latest.json must contain an object")
    if set(latest) != {"version", "notes", "pub_date", "platforms"}:
        raise VerificationError("latest.json field set mismatch")
    if latest.get("version") != version:
        raise VerificationError("latest.json version mismatch")
    if latest.get("notes") != f"cys {version}":
        raise VerificationError("latest.json notes mismatch")
    pub_date = latest.get("pub_date")
    if not isinstance(pub_date, str):
        raise VerificationError("latest.json pub_date must be a string")
    try:
        parsed_pub_date = datetime.fromisoformat(pub_date.replace("Z", "+00:00"))
    except ValueError as error:
        raise VerificationError("latest.json pub_date is not RFC3339") from error
    if parsed_pub_date.tzinfo is None:
        raise VerificationError("latest.json pub_date must include a timezone")
    expected_platforms = {"darwin-aarch64", "darwin-x86_64", "windows-x86_64"}
    platforms = latest.get("platforms")
    if not isinstance(platforms, dict) or set(platforms) != expected_platforms:
        raise VerificationError("latest.json platform set mismatch")
    repository = str(policy.get("repository", ""))
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise VerificationError("release policy repository is invalid")
    base_url = f"https://github.com/{repository}/releases/download/v{version}"
    updater_assets = {
        "darwin-aarch64": (
            f"cys_{version}_aarch64.app.tar.gz",
            f"cys_{version}_aarch64.app.tar.gz.sig",
        ),
        "darwin-x86_64": (
            f"cys_{version}_x64.app.tar.gz",
            f"cys_{version}_x64.app.tar.gz.sig",
        ),
        "windows-x86_64": (
            f"cys_{version}_x64-setup.exe",
            f"cys_{version}_x64-setup.exe.sig",
        ),
    }
    for platform, (asset, signature_asset) in updater_assets.items():
        row = platforms[platform]
        if not isinstance(row, dict) or set(row) != {"signature", "url"}:
            raise VerificationError(f"updater row field set mismatch: {platform}")
        expected_url = f"{base_url}/{asset}"
        if row.get("url") != expected_url:
            raise VerificationError(f"updater URL mismatch: {platform}")
        expected_signature = files[signature_asset].read_text(encoding="utf-8").strip()
        if not expected_signature or row.get("signature") != expected_signature:
            raise VerificationError(f"updater signature mismatch: {platform}")
    return sorted(expected_checksums)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--release-dir", required=True, type=Path)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--print-assets", action="store_true")
    args = parser.parse_args()
    try:
        assets = verify_release(args.version, args.release_dir, args.policy)
    except (
        OSError,
        UnicodeError,
        json.JSONDecodeError,
        zipfile.BadZipFile,
        VerificationError,
    ) as error:
        print(f"release verification failed: {error}", file=sys.stderr)
        return 1
    if args.print_assets:
        print("\n".join(assets))
    else:
        print(f"release verified: {len(assets)} checksummed assets")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
