#!/usr/bin/env python3
"""Compile the byte-extracted real method after writer/runner permission
inside a scripted-leaf Rust wrapper. This is NOT native Windows fault injection.
The draft freeze refuses execution until replaced with reviewed actual hashes.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys

SIGNATURE = b"    fn is_complete(&mut self) -> IoResult<Option<ExitStatus>> {\n"
NEXT_FUNCTION = b"    fn do_kill(&mut self) -> IoResult<()> {\n"
MARKER = b"/*__Q9B_EXACT_METHOD_BYTES__*/"
EXPECTED_TESTS = sorted([
    "q9b_model_tests::clone_error_returns_io_error_without_unwind_or_exit_api_call",
    "q9b_model_tests::clone_error_does_not_poison_proc_mutex",
    "q9b_model_tests::active_process_keeps_ok_none",
    "q9b_model_tests::terminal_process_keeps_exit_code",
    "q9b_model_tests::exit_code_api_failure_keeps_ok_none",
    "q9b_model_tests::prepoisoned_mutex_still_panics_before_native_leaves",
])


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise RuntimeError(reason)


def extract(source: bytes) -> tuple[bytes, dict]:
    # Deliberately narrow/fail-closed extractor for this frozen helper shape,
    # not a permissive Rust parser. Never normalize line endings or rewrite it.
    require(b"\r" not in source, "CRLF/non-LF source: freeze exact bytes; do not normalize silently")
    require(source.count(SIGNATURE) == 1, "is_complete signature must occur exactly once")
    require(source.count(NEXT_FUNCTION) == 1, "next-function anchor must occur exactly once")
    start = source.index(SIGNATURE)
    end = source.index(NEXT_FUNCTION)
    require(end > start, "invalid method boundaries")
    body = source[start:end]
    require(body.endswith(b"    }\n\n"), "unexpected closing/separator bytes")
    require(body.count(b"\n    fn ") == 0, "another method appeared inside extraction")
    require(body.count(b"{") == body.count(b"}"), "unexpected brace shape; review extractor rather than guessing")
    return body, {
        "source_sha256": digest(source),
        "method_sha256": digest(body),
        "byte_start_inclusive": start,
        "byte_end_exclusive": end,
        "byte_count": len(body),
        "line_start": source[:start].count(b"\n") + 1,
        "line_end_including_blank_separator": source[:end].count(b"\n"),
        "signature_hex": SIGNATURE.hex(),
        "next_function_anchor_hex": NEXT_FUNCTION.hex(),
        "normalization_or_method_rewrite": False,
    }


def process(command: list[str], cwd: pathlib.Path, timeout: int, stem: pathlib.Path) -> dict:
    try:
        got = subprocess.run(command, cwd=cwd, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, timeout=timeout, check=False)
        stdout, stderr, code, timed_out = got.stdout, got.stderr, got.returncode, False
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout or b""
        stderr = exc.stderr or b""
        code, timed_out = None, True
    stem.with_suffix(".stdout").write_bytes(stdout)
    stem.with_suffix(".stderr").write_bytes(stderr)
    return {"command": command, "returncode": code, "timed_out": timed_out,
            "stdout_sha256": digest(stdout), "stderr_sha256": digest(stderr)}


def run(freeze_path: pathlib.Path, execute: bool) -> int:
    require(execute, "execution disabled: --execute and a reviewed frozen manifest are both required")
    frozen_bytes = freeze_path.read_bytes()
    freeze = json.loads(frozen_bytes)
    require(freeze["status"] == "FROZEN_EXECUTION_APPROVED", "draft freeze refuses all compiler/test processes")
    require(freeze["scope"] == "q9b-source-body-leaf-model-only", "incorrect scope")
    require(freeze["authority_message"] == "q6ad11296.4352", "missing exact-location authority")
    driver_sha = digest(pathlib.Path(__file__).read_bytes())
    require(driver_sha == freeze["expected_driver_sha256"], "driver changed after freeze")
    repo = pathlib.Path(freeze["repository_root"]).resolve(strict=True)
    parent = pathlib.Path(freeze["output_parent"]).resolve(strict=True)
    require(parent == (repo / "target/u-tmp").resolve(strict=True), "output must stay in existing authorized target/u-tmp")
    require(re.fullmatch(r"[A-Za-z0-9_-]+", freeze["run_id"]) is not None, "unsafe run ID")
    output = parent / freeze["run_id"]
    output.mkdir()  # create-new: never overwrite an earlier red/green artifact
    record = {"scope": freeze["scope"], "freeze_sha256": digest(frozen_bytes), "driver_sha256": driver_sha,
              "state": "preparing", "native_fault_injection": False, "commands": []}

    def save() -> None:
        (output / "run-record.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")

    (output / "freeze.json").write_bytes(frozen_bytes)
    save()
    try:
        if freeze["source_kind"] == "git-blob":
            revision = freeze["source_revision"]
            require(re.fullmatch(r"[0-9a-f]{40}", revision) is not None, "full immutable Git revision required")
            require(freeze["source_relpath"] == "vendor/portable-pty/src/win/mod.rs", "unexpected source path")
            got = subprocess.run(["git", "show", f"{revision}:{freeze['source_relpath']}"],
                                 cwd=repo, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 timeout=30, check=False)
            require(got.returncode == 0, "pinned source could not be read; not assertion-red")
            source = got.stdout
        elif freeze["source_kind"] == "file":
            selected = pathlib.Path(freeze["source_path"]).resolve(strict=True)
            require(selected == (repo / "vendor/portable-pty/src/win/mod.rs").resolve(strict=True)
                    or selected.is_relative_to(parent), "source file is outside the declared repository/fixture")
            source = selected.read_bytes()
        else:
            raise RuntimeError("unsupported source kind")
        body, extraction = extract(source)
        require(extraction["source_sha256"] == freeze["expected_source_sha256"], "full source hash mismatch")
        require(extraction["method_sha256"] == freeze["expected_method_sha256"], "extracted method hash mismatch")
        template = pathlib.Path(freeze["template_path"]).read_bytes()
        require(digest(template) == freeze["expected_template_sha256"], "template hash mismatch")
        require(template.count(MARKER) == 1, "exactly one method insertion marker required")
        generated = template.replace(MARKER, body)
        require(generated.count(body) == 1 and MARKER not in generated, "exact-byte insertion failed")
        (output / "source.snapshot.rs").write_bytes(source)
        (output / "is_complete.extracted.rs").write_bytes(body)
        model_source = output / "model.rs"
        model_source.write_bytes(generated)
        record.update(state="source_prepared", extraction=extraction,
                      template_sha256=digest(template), generated_sha256=digest(generated),
                      method_inserted_byte_for_byte=True)
        save()

        rustc = freeze["rustc_program"]
        version = process([rustc, "--version", "--verbose"], output, 30, output / "rustc-version")
        record["commands"].append(version)
        require(version["returncode"] == 0 and not version["timed_out"], "rustc version unavailable")
        require(version["stdout_sha256"] == freeze["expected_rustc_version_stdout_sha256"], "compiler identity differs from freeze")
        binary = output / ("model-tests.exe" if sys.platform == "win32" else "model-tests")
        compile_result = process([rustc, "--edition=2018", "-C", "panic=unwind", "--test", str(model_source), "-o", str(binary)],
                                 output, 120, output / "compile")
        record["commands"].append(compile_result)
        if compile_result["timed_out"] or compile_result["returncode"] != 0:
            record["state"] = "compile_incomplete_or_error_NOT_assertion_red"
            save()
            return 2
        record["generated_test_binary_sha256"] = digest(binary.read_bytes())
        listing = process([str(binary), "--list"], output, 30, output / "test-list")
        record["commands"].append(listing)
        require(listing["returncode"] == 0 and not listing["timed_out"], "test listing failed")
        lines = (output / "test-list.stdout").read_text(encoding="utf-8").splitlines()
        names = sorted(line[:-6] for line in lines if line.endswith(": test"))
        require(names == EXPECTED_TESTS, "missing/extra test names; zero or partial execution is not evidence")
        record["listed_test_names"] = names
        record["state"] = "executing_source_body_model"
        save()
        suite = process([str(binary), "--test-threads=1", "--nocapture"], output, 30, output / "test-run")
        record["commands"].append(suite)
        record["state"] = "test_timeout_incomplete" if suite["timed_out"] else "test_process_completed"
        record["test_returncode"] = suite["returncode"]
        record["interpretation"] = "source-body/leaf-model only; read actual assertions/logs; no release or native-fault verdict"
        save()
        return 124 if suite["timed_out"] else int(suite["returncode"])
    except Exception as exc:
        record["state"] = "precondition_or_harness_error_NOT_assertion_red"
        record["error"] = str(exc)
        save()
        return 3


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", type=pathlib.Path, required=True)
    parser.add_argument("--execute", action="store_true", help="only after explicit integration/runner grant")
    args = parser.parse_args()
    try:
        return run(args.freeze, args.execute)
    except Exception as exc:
        print(f"Q9-b precondition rejection: {exc}", file=sys.stderr)
        return 3


if __name__ == "__main__":
    raise SystemExit(main())
