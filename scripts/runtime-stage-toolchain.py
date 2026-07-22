#!/usr/bin/env python3
"""Verify and atomically extract the pinned Bun compile runtime."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path, PurePosixPath
import stat
import tempfile
import zipfile


RUNTIME_STAGE = Path(__file__).with_name("runtime-stage.py")
SPEC = importlib.util.spec_from_file_location("cys_runtime_stage", RUNTIME_STAGE)
if SPEC is None or SPEC.loader is None:
    raise SystemExit("Browser Runtime stage verifier is unavailable")
runtime_stage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime_stage)


def extract(args: argparse.Namespace) -> None:
    source = json.loads(args.source_manifest.read_text(encoding="utf-8"))
    if source.get("schema_version") != 1:
        runtime_stage.fail("unsupported Browser Runtime source manifest schema")
    assets = source.get("targets", {}).get(args.target)
    if not isinstance(assets, dict) or args.target not in runtime_stage.TARGETS:
        runtime_stage.fail(f"unsupported or unpinned Browser Runtime target: {args.target}")
    runtime_stage.require_https_url(assets.get("bun_compiler_archive_url"), "Bun compiler provenance")
    expected_bytes = assets.get("bun_compiler_archive_bytes")
    if not isinstance(expected_bytes, int) or args.archive.stat().st_size != expected_bytes:
        runtime_stage.fail("Bun compiler archive byte size mismatch")
    runtime_stage.require_sha256(
        args.archive, assets.get("bun_compiler_archive_sha256", ""), "Bun compiler archive"
    )
    member_path = runtime_stage.safe_archive_relative(
        assets.get("bun_compiler_member"), "Bun compiler member"
    )

    with zipfile.ZipFile(args.archive) as archive:
        members = archive.infolist()
        names = {PurePosixPath(member.filename.rstrip("/")) for member in members}
        allowed = {member_path, member_path.parent}
        if names != allowed:
            runtime_stage.fail("Bun compiler archive has an unexpected file set")
        member = archive.getinfo(member_path.as_posix())
        mode = (member.external_attr >> 16) & 0o170000
        if mode not in (0, stat.S_IFREG):
            runtime_stage.fail("Bun compiler archive member is not a regular file")
        if member.file_size <= 0 or member.file_size > 128 * 1024 * 1024:
            runtime_stage.fail("Bun compiler executable size is invalid")

        args.output.parent.mkdir(parents=True, exist_ok=True)
        if args.output.exists():
            runtime_stage.fail("Bun compiler output already exists")
        temporary_path = None
        try:
            with tempfile.NamedTemporaryFile(
                prefix=f".{args.output.name}.", dir=args.output.parent, delete=False
            ) as temporary:
                temporary_path = Path(temporary.name)
                with archive.open(member) as payload:
                    while chunk := payload.read(1024 * 1024):
                        temporary.write(chunk)
                temporary.flush()

            temporary_path.chmod(0o755)
            expected_arch = runtime_stage.TARGETS[args.target][0]
            runtime_stage.assert_target(temporary_path, expected_arch, "Bun compiler")
            temporary_path.replace(args.output)
        finally:
            if temporary_path is not None:
                temporary_path.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-manifest", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    extract(parser.parse_args())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
