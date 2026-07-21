#!/usr/bin/env python3
"""Assemble one exact, deterministic cys release candidate.

The input directory is an untrusted hand-off from target build jobs.  This
command is the single public interface that turns those files into the exact
release asset set.  It never signs, uploads, or reads credentials.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
import re
import shutil
import sys
import tempfile
import zipfile
from datetime import datetime
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = ROOT / "release" / "assets-policy.json"
VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")


class ReleaseError(ValueError):
    pass


def load_policy(path: Path, version: str) -> dict[str, object]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    if raw.get("windows_release_mode") != "unsigned":
        raise ReleaseError("Windows release mode must be unsigned")
    for field in ("input_assets", "generated_assets", "install_assets"):
        if not isinstance(raw.get(field), list):
            raise ReleaseError(f"policy field {field!r} must be a list")
        raw[field] = [str(item).format(version=version) for item in raw[field]]
    return raw


def exact_input_files(input_dir: Path, expected: set[str]) -> dict[str, Path]:
    if not input_dir.is_dir():
        raise ReleaseError(f"input directory does not exist: {input_dir}")
    found: dict[str, Path] = {}
    duplicates: list[str] = []
    for path in input_dir.rglob("*"):
        if not path.is_file():
            continue
        if path.name in found:
            duplicates.append(path.name)
        else:
            found[path.name] = path
    if duplicates:
        raise ReleaseError(f"duplicate asset names: {sorted(set(duplicates))}")
    missing = sorted(expected - found.keys())
    extra = sorted(found.keys() - expected)
    if missing or extra:
        raise ReleaseError(f"asset set mismatch: missing={missing}; extra={extra}")
    for name, path in found.items():
        if path.is_symlink():
            raise ReleaseError(f"symlink input is forbidden: {name}")
        if path.stat().st_size == 0:
            raise ReleaseError(f"empty input is forbidden: {name}")
    return found


def write_flat_windows_zip(exe: Path, destination: Path) -> None:
    info = zipfile.ZipInfo(exe.name, date_time=(2020, 1, 1, 0, 0, 0))
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = 0o100644 << 16
    info.create_system = 3
    with zipfile.ZipFile(destination, "w", allowZip64=True) as archive:
        archive.writestr(info, exe.read_bytes())


def updater_platforms(version: str, repository: str, stage: Path) -> dict[str, dict[str, str]]:
    base = f"https://github.com/{repository}/releases/download/v{version}"

    def row(asset: str, signature_asset: str) -> dict[str, str]:
        signature = (stage / signature_asset).read_text(encoding="utf-8").strip()
        if not signature:
            raise ReleaseError(f"empty updater signature: {signature_asset}")
        return {"signature": signature, "url": f"{base}/{asset}"}

    return {
        "darwin-aarch64": row(
            f"cys_{version}_aarch64.app.tar.gz",
            f"cys_{version}_aarch64.app.tar.gz.sig",
        ),
        "darwin-x86_64": row(
            f"cys_{version}_x64.app.tar.gz",
            f"cys_{version}_x64.app.tar.gz.sig",
        ),
        "windows-x86_64": row(
            f"cys_{version}_x64-setup.exe",
            f"cys_{version}_x64-setup.exe.sig",
        ),
    }


def write_checksums(stage: Path) -> None:
    rows = []
    for path in sorted(stage.iterdir(), key=lambda item: item.name):
        if path.name == "SHA256SUMS.txt":
            continue
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        rows.append(f"{digest}  {path.name}\n")
    (stage / "SHA256SUMS.txt").write_text("".join(rows), encoding="utf-8")


def write_site_bundle(version: str, release_dir: Path, site_output: Path, policy: dict[str, object]) -> None:
    if site_output.exists():
        raise ReleaseError(f"site output already exists: {site_output}")
    site_output.parent.mkdir(parents=True, exist_ok=True)
    safe_version = html.escape(version)
    install_assets = [str(item) for item in policy["install_assets"]]
    links = "\n".join(
        f'      <li><a href="/downloads/{html.escape(name)}">{html.escape(name)}</a></li>'
        for name in install_assets
    )
    index = f"""<!doctype html>
<html lang="ko">
<head><meta charset="utf-8"><title>cys {safe_version} 다운로드</title></head>
<body>
  <main>
    <h1>cys {safe_version} 다운로드</h1>
    <ul>
{links}
    </ul>
    <section {policy['defender_marker']}>
      <h2>Windows Defender 안내</h2>
      <p>Windows 설치 파일은 의도적으로 Authenticode 서명되지 않았습니다. 따라서 Chrome, SmartScreen 또는 Defender에서 “알 수 없는 게시자” 경고가 나타날 수 있습니다.</p>
      <p>EXE 직접 다운로드가 차단되면 ZIP을 받아 압축을 푼 뒤 같은 설치 파일을 사용할 수 있습니다. 실행 전 반드시 SHA256SUMS.txt와 다운로드 파일의 SHA-256을 대조하세요.</p>
      <p>해시가 일치할 때만 Windows 보안의 보호 기록에서 허용 여부를 직접 판단하세요. Defender나 브라우저의 보호 기능을 끄지 마세요.</p>
    </section>
  </main>
</body>
</html>
"""
    recovery_links = "\n".join(
        f'      <li><a href="/downloads/{html.escape(name)}">{html.escape(name)}</a></li>'
        for name in install_assets
    )
    recovery = f"""<!doctype html>
<html lang="ko">
<head><meta charset="utf-8"><title>cys {safe_version} 복구</title></head>
<body>
  <main {policy['recovery_marker']}>
    <h1>cys {safe_version} 복구 다운로드</h1>
    <ul>
{recovery_links}
    </ul>
  </main>
</body>
</html>
"""

    with tempfile.TemporaryDirectory(prefix="release-site-", dir=site_output.parent) as temporary:
        stage = Path(temporary)
        downloads = stage / "downloads"
        recovery_dir = downloads / "recovery"
        recovery_dir.mkdir(parents=True)
        for path in release_dir.iterdir():
            if path.is_file():
                shutil.copyfile(path, downloads / path.name)
        (downloads / "index.html").write_text(index, encoding="utf-8")
        (recovery_dir / "index.html").write_text(recovery, encoding="utf-8")
        os.replace(stage, site_output)


def assemble(
    version: str,
    input_dir: Path,
    output_dir: Path,
    repository: str,
    policy_path: Path,
    site_output: Path | None,
    pub_date: str,
) -> None:
    if not VERSION_RE.fullmatch(version):
        raise ReleaseError(f"version must be X.Y.Z, got {version!r}")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ReleaseError(f"invalid GitHub repository: {repository!r}")
    try:
        parsed_pub_date = datetime.fromisoformat(pub_date.replace("Z", "+00:00"))
    except ValueError as error:
        raise ReleaseError(f"pub-date must be RFC3339: {pub_date!r}") from error
    if parsed_pub_date.tzinfo is None:
        raise ReleaseError("pub-date must include a timezone")

    policy = load_policy(policy_path, version)
    policy_repository = str(policy.get("repository", ""))
    if repository != policy_repository:
        raise ReleaseError(
            f"repository does not match release policy: {repository!r} != {policy_repository!r}"
        )
    inputs = exact_input_files(input_dir, set(policy["input_assets"]))
    expected_output = set(policy["input_assets"]) | set(policy["generated_assets"])
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    if output_dir.exists():
        raise ReleaseError(f"output already exists: {output_dir}")

    with tempfile.TemporaryDirectory(prefix="release-stage-", dir=output_dir.parent) as temporary:
        stage = Path(temporary)
        for name, source in inputs.items():
            shutil.copyfile(source, stage / name)

        exe_name = f"cys_{version}_x64-setup.exe"
        write_flat_windows_zip(stage / exe_name, stage / f"cys_{version}_x64-setup.zip")
        latest = {
            "version": version,
            "notes": f"cys {version}",
            "pub_date": pub_date,
            "platforms": updater_platforms(version, repository, stage),
        }
        (stage / "latest.json").write_text(
            json.dumps(latest, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        write_checksums(stage)

        actual_output = {path.name for path in stage.iterdir() if path.is_file()}
        if actual_output != expected_output:
            raise ReleaseError(
                "assembled asset set mismatch: "
                f"missing={sorted(expected_output - actual_output)}; "
                f"extra={sorted(actual_output - expected_output)}"
            )
        os.replace(stage, output_dir)
    if site_output is not None:
        write_site_bundle(version, output_dir, site_output, policy)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--site-output", type=Path)
    parser.add_argument("--pub-date", required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        assemble(
            args.version,
            args.input,
            args.output,
            args.repository,
            args.policy,
            args.site_output,
            args.pub_date,
        )
    except (OSError, UnicodeError, json.JSONDecodeError, ReleaseError) as error:
        print(f"release assembly failed: {error}", file=sys.stderr)
        return 1
    print(f"release assembly complete: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
