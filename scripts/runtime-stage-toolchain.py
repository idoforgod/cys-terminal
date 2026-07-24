#!/usr/bin/env python3
"""Verify and atomically extract the pinned Bun compile runtime."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path, PurePosixPath
import stat
import sys
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
            # ★진범(5차 CI): temp 쓰기 핸들은 이 with 블록 안에서만 열어둔다 — 블록을 나가며
            #   close된다. Windows는 열린 fd를 가진 파일의 rename을 sharing violation(WinError 32)
            #   으로 거부하므로, chmod·assert·replace는 반드시 블록 밖(핸들 close 후)에서 실행해야
            #   한다. 잠금 보유자는 AV가 아니라 우리 자신의 미close 핸들이었고(flush만 하고 close
            #   전 replace 호출), 그래서 재시도는 자기 핸들 상대라 결정론 실패했다. POSIX는 열린
            #   fd rename을 허용해 mac에선 무증상이었다.
            with tempfile.NamedTemporaryFile(
                prefix=f".{args.output.name}.", dir=args.output.parent, delete=False
            ) as temporary:
                temporary_path = Path(temporary.name)
                with archive.open(member) as payload:
                    while chunk := payload.read(1024 * 1024):
                        temporary.write(chunk)
                temporary.flush()
            # 여기서 temp 핸들은 닫혀 있다(위 with 블록 종료).
            temporary_path.chmod(0o755)
            expected_arch = runtime_stage.TARGETS[args.target][0]
            runtime_stage.assert_target(temporary_path, expected_arch, "Bun compiler")
            # retry 헬퍼는 진짜 AV/인덱서 간섭 대비로 유지(자기 핸들은 이미 위에서 close).
            runtime_stage.retry_on_windows_lock(lambda: temporary_path.replace(args.output))
        finally:
            # Best-effort temp cleanup. Only has work when the replace failed or an
            # earlier step raised (a successful replace leaves nothing to unlink). If a
            # primary exception is already propagating, swallow a residual lock rather
            # than mask it — the job still fails closed on the primary error.
            if temporary_path is not None:
                pending = sys.exc_info()[0]
                try:
                    runtime_stage.retry_on_windows_lock(
                        lambda: temporary_path.unlink(missing_ok=True)
                    )
                except PermissionError:
                    if pending is None:
                        raise


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
