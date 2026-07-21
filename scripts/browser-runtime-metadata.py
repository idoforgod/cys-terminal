#!/usr/bin/env python3
"""Build and verify signed Browser Runtime metadata without exposing keys."""

from __future__ import annotations

import argparse
import base64
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
import time


DOMAIN = b"cys.browser.chromium-tree.v1\0"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def hash_tree(root: Path) -> str:
    if not root.is_dir():
        raise SystemExit(f"Chromium tree root unavailable: {root}")
    digest = hashlib.sha256(DOMAIN)
    files = sorted(path for path in root.rglob("*") if path.is_file() and not path.is_symlink())
    if any(path.is_symlink() for path in root.rglob("*")):
        raise SystemExit("Chromium tree contains a symlink")
    for path in files:
        relative = path.relative_to(root).as_posix().encode()
        size = path.stat().st_size
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(size.to_bytes(8, "big"))
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
    return digest.hexdigest()


def canonical_runtime_id(manifest: dict) -> str:
    value = dict(manifest)
    value.pop("runtime_id", None)
    value.pop("signature", None)
    value.pop("attestation", None)
    canonical = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    return "sha256:" + hashlib.sha256(canonical).hexdigest()


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def minisign(path: Path, signature: Path, secret_key: Path) -> None:
    subprocess.run(
        ["minisign", "-Sm", str(path), "-s", str(secret_key), "-x", str(signature)],
        check=True,
    )


def verify(path: Path, signature: Path, public_key: Path) -> None:
    subprocess.run(
        ["minisign", "-Vm", str(path), "-p", str(public_key), "-x", str(signature)],
        check=True,
    )


def _public_key_line(data: bytes) -> bytes:
    lines = [line.strip() for line in data.splitlines() if line.strip()]
    if not lines or not lines[-1].startswith(b"RW"):
        raise SystemExit("invalid minisign public key material")
    return lines[-1]


def validate_trust_root(
    key_id: str,
    public_key: Path,
    trusted_keys_path: Path,
    tauri_config_path: Path,
    expires_at: int,
) -> None:
    trusted = json.loads(trusted_keys_path.read_text(encoding="utf-8"))
    if key_id in trusted.get("revoked_key_ids", []):
        raise SystemExit(f"Browser Runtime signing key is revoked: {key_id}")
    entry = next((item for item in trusted.get("keys", []) if item.get("key_id") == key_id), None)
    if entry is None:
        raise SystemExit(f"Browser Runtime signing key is not in the compiled trust root: {key_id}")
    try:
        not_after = int(datetime.fromisoformat(entry["not_after"].replace("Z", "+00:00")).timestamp())
    except (KeyError, TypeError, ValueError) as error:
        raise SystemExit(f"invalid Browser Runtime trust-root expiry: {error}") from error
    if expires_at >= not_after:
        raise SystemExit("metadata expiry must be earlier than the compiled signing-key expiry")

    configured = entry.get("pubkey", "")
    if not configured:
        tauri = json.loads(tauri_config_path.read_text(encoding="utf-8"))
        configured = tauri["plugins"]["updater"]["pubkey"]
    try:
        expected = _public_key_line(base64.b64decode(configured, validate=True))
    except (KeyError, TypeError, ValueError) as error:
        raise SystemExit(f"compiled Browser Runtime public key is invalid: {error}") from error
    actual = _public_key_line(public_key.read_bytes())
    if actual != expected:
        raise SystemExit("Browser Runtime public key does not match the compiled trust root")


def prepare(args: argparse.Namespace) -> None:
    root = args.resource_root.resolve(strict=True)
    manifest_path = root / "browser-runtime.lock.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assets = manifest["targets"][args.target]
    target_root = root / "runtime" / args.target
    suffix = ".exe" if args.target == "x86_64-pc-windows-msvc" else ""
    supervisor = target_root / "supervisor" / f"cys-browserd{suffix}"
    engine = target_root / "engine" / f"cys-browser-engine{suffix}"
    chromium_root = target_root / "chromium"
    chromium = target_root / assets["chromium_executable"]
    for path in (supervisor, engine, chromium):
        if not path.is_file() or path.is_symlink():
            raise SystemExit(f"pinned executable unavailable or linked: {path}")
    assets["supervisor_sha256"] = sha256_file(supervisor)
    assets["engine_sha256"] = sha256_file(engine)
    assets["chromium_tree_sha256"] = hash_tree(chromium_root)
    manifest["runtime_id"] = canonical_runtime_id(manifest)
    write_json(manifest_path, manifest)

    now = int(time.time())
    expires = int(args.expires_at)
    if expires <= now:
        raise SystemExit("metadata expiry must be in the future")
    validate_trust_root(
        args.key_id,
        args.public_key,
        args.trusted_keys,
        args.tauri_config,
        expires,
    )
    attestation = {
        "attestation_schema": 1,
        "key_id": args.key_id,
        "signed_at": now,
        "expires_at": expires,
        "runtime_id": manifest["runtime_id"],
        "target": args.target,
        "supervisor_sha256": assets["supervisor_sha256"],
        "engine_sha256": assets["engine_sha256"],
        "chromium_tree_sha256": assets["chromium_tree_sha256"],
    }
    policy = {
        "key_id": args.key_id,
        "signed_at": now,
        "expires_at": expires,
        "epoch": args.policy_epoch,
        "allowed_runtime_ids": [manifest["runtime_id"]],
    }
    attestation_path = root / "runtime-attestation.json"
    policy_path = root / "runtime-policy-snapshot.json"
    write_json(attestation_path, attestation)
    write_json(policy_path, policy)
    for path in (attestation_path, policy_path):
        signature = path.with_suffix(path.suffix + ".minisig")
        minisign(path, signature, args.secret_key)
        verify(path, signature, args.public_key)


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    hash_command = sub.add_parser("hash-tree")
    hash_command.add_argument("root", type=Path)
    prep = sub.add_parser("prepare")
    prep.add_argument("--resource-root", type=Path, required=True)
    prep.add_argument("--target", required=True)
    prep.add_argument("--key-id", required=True)
    prep.add_argument("--secret-key", type=Path, required=True)
    prep.add_argument("--public-key", type=Path, required=True)
    prep.add_argument("--trusted-keys", type=Path, required=True)
    prep.add_argument("--tauri-config", type=Path, required=True)
    prep.add_argument("--policy-epoch", type=int, required=True)
    prep.add_argument("--expires-at", type=int, required=True)
    args = parser.parse_args()
    if args.command == "hash-tree":
        print(hash_tree(args.root.resolve(strict=True)))
    else:
        prepare(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
