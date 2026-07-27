import json
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
RELEASE_WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
PUBLISH_WORKFLOW = ROOT / ".github" / "workflows" / "release-publish.yml"
PACK_WORKFLOW = ROOT / ".github" / "workflows" / "pack-release.yml"
MACOS_BUILD = ROOT / "scripts" / "build-macos-signed.sh"
WINDOWS_SIGN = ROOT / "scripts" / "windows-authenticode.ps1"
BROWSER_RUNTIME_SOURCES = ROOT / "release" / "browser-runtime-sources.json"
WINDOWS_BUILD = ROOT / "scripts" / "build-windows-signed.ps1"
BROWSER_MAC_SIGN = ROOT / "scripts" / "runtime-stage-sign-macos.sh"
BROWSER_WINDOWS_SIGN = ROOT / "scripts" / "runtime-stage-sign-windows.ps1"


class ReleaseWorkflowTests(unittest.TestCase):
    def test_candidate_workflow_has_one_draft_assembler_and_no_public_publish(self) -> None:
        candidate = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        publish = PUBLISH_WORKFLOW.read_text(encoding="utf-8")

        self.assertNotIn("tauri-apps/tauri-action", candidate)
        self.assertIn("actions/upload-artifact@", candidate)
        self.assertIn("actions/download-artifact@", candidate)
        self.assertEqual(candidate.count("gh release create"), 1)
        self.assertIn("--draft", candidate)
        self.assertNotRegex(candidate, re.compile(r"gh release edit|--draft=false|--latest"))
        self.assertIn("workflow_dispatch", publish)
        self.assertIn("environment: release-production", publish)
        self.assertIn("release_bundle_sha256", publish)
        self.assertIn("gh release edit", publish)

    def test_public_repository_handoffs_are_ciphertext_only(self) -> None:
        candidate = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("scripts/release-handoff.sh pack", candidate)
        self.assertIn("scripts/release-handoff.sh unpack", candidate)
        self.assertIn("RELEASE_HANDOFF_KEY", candidate)
        self.assertNotIn("path: release-candidate/", candidate)
        self.assertNotRegex(candidate, re.compile(r"path:\s*\|[\s\S]{0,160}release-out/"))
        self.assertGreaterEqual(candidate.count(".tgz.enc"), 3)

    def test_every_external_action_is_pinned_to_the_locked_commit(self) -> None:
        lock_path = ROOT / "release" / "actions-lock.json"
        lock = json.loads(lock_path.read_text(encoding="utf-8"))["actions"]
        uses_pattern = re.compile(r"^\s*-?\s*uses:\s*([^\s#]+)", re.MULTILINE)
        for workflow in sorted((ROOT / ".github" / "workflows").glob("*.yml")):
            for use in uses_pattern.findall(workflow.read_text(encoding="utf-8")):
                if use.startswith("./"):
                    continue
                repository, separator, revision = use.rpartition("@")
                self.assertTrue(separator, f"missing revision: {workflow}: {use}")
                self.assertRegex(revision, r"^[0-9a-f]{40}$", f"floating action: {workflow}: {use}")
                self.assertIn(repository.lower(), lock, f"action missing from lock: {repository}")
                self.assertEqual(revision, lock[repository.lower()]["sha"])

    def test_only_explicit_publish_workflows_can_make_a_release_public(self) -> None:
        for workflow in sorted((ROOT / ".github" / "workflows").glob("*.yml")):
            body = workflow.read_text(encoding="utf-8")
            if workflow.name.endswith("-publish.yml"):
                self.assertIn("environment: release-production", body)
                self.assertIn("--draft=false", body)
                continue
            self.assertNotIn("--draft=false", body, f"public promotion leaked into {workflow.name}")

    def test_failed_candidate_upload_deletes_the_incomplete_draft(self) -> None:
        candidate = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        self.assertRegex(candidate, re.compile(r"if ! gh release upload[\s\S]+gh release delete"))
        pack = PACK_WORKFLOW.read_text(encoding="utf-8")
        self.assertRegex(pack, re.compile(r"if ! gh release upload[\s\S]+gh release delete"))

    def test_windows_crt_gate_is_wired_in_both_windows_lanes(self) -> None:
        # CRT 게이트(VCRUNTIME 임포트 스윕 + 3카테고리 수리 마커)가 릴리스·feasibility 양 레인에서
        # 조용히 소실되는 회귀를 차단한다. 게이트 자체의 판정 로직은 test_verify_win_crt.py 소관.
        candidate = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        feasibility = (ROOT / ".github" / "workflows" / "windows-build.yml").read_text(encoding="utf-8")
        for body, lane in ((candidate, "release.yml"), (feasibility, "windows-build.yml")):
            self.assertIn("scripts/verify_win_crt.py", body, f"CRT gate missing in {lane}")
        # 릴리스 레인은 서명 완료된 정규화 산출물(출하 바이트)을 검사해야 한다.
        self.assertIn("release-candidate/cys_", candidate)
        # 미서명 탈출구(ALLOW_UNSIGNED_WINDOWS)가 게이트를 우회하지 못하게 — 게이트 스텝은
        # platform 조건만 가져야 한다.
        gate_idx = candidate.index("Windows CRT gate")
        gate_block = candidate[gate_idx:gate_idx + 400]
        self.assertNotIn("ALLOW_UNSIGNED_WINDOWS", gate_block)

    def test_platform_signing_and_notarization_are_fail_closed(self) -> None:
        macos = MACOS_BUILD.read_text(encoding="utf-8")
        windows = WINDOWS_SIGN.read_text(encoding="utf-8")

        self.assertGreaterEqual(macos.count("xcrun notarytool submit"), 2)
        self.assertIn('xcrun stapler validate "$APP"', macos)
        self.assertIn('xcrun stapler validate "$DMG"', macos)
        self.assertNotRegex(macos, re.compile(r"(?:notarytool|stapler)[^\n]*\|\|\s*true"))

        for required in ("/fd SHA256", "/tr $env:WINDOWS_TIMESTAMP_URL", "/td SHA256"):
            self.assertIn(required, windows)
        self.assertIn("TimeStamperCertificate", windows)
        self.assertIn("Get-AuthenticodeSignature", windows)

    def test_browser_runtime_release_credentials_and_minisign_are_wired_for_every_target(self) -> None:
        candidate = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        for name in (
            "CYS_BROWSER_RUNTIME_SECRET_KEY",
            "CYS_BROWSER_RUNTIME_PUBLIC_KEY",
            "CYS_BROWSER_RUNTIME_KEY_ID",
            "CYS_BROWSER_RUNTIME_POLICY_EPOCH",
            "CYS_BROWSER_RUNTIME_EXPIRES_AT",
        ):
            self.assertGreaterEqual(candidate.count(name), 2, f"missing cross-platform wiring: {name}")
        self.assertIn("CYS_BROWSER_RUNTIME_SECRET_KEY_B64", candidate)
        self.assertIn("CYS_BROWSER_RUNTIME_PUBLIC_KEY_B64", candidate)
        # minisign(jedisct1) exposes only the short flag -v (no long --version); asserting
        # the long form pins a command that exits 2 on every runner. Track the working flag.
        self.assertRegex(candidate, re.compile(r"brew install minisign[\s\S]+minisign -v"))
        # choco의 minisign은 구버전이라 무암호 비밀키의 KDF를 못 읽어 -S 서명이 exit 2로 실패한다.
        # Windows는 공식 0.12 win64 zip을 다이제스트 고정 설치한다(무암호 키 KDF 호환·fail-closed).
        self.assertNotIn("choco install minisign", candidate)
        self.assertIn(
            "https://github.com/jedisct1/minisign/releases/download/0.12/minisign-0.12-win64.zip",
            candidate,
        )
        self.assertIn(
            "37b600344e20c19314b2e82813db2bfdcc408b77b876f7727889dbd46d539479", candidate
        )
        self.assertRegex(candidate, re.compile(r"Get-FileHash -Algorithm SHA256[\s\S]+\$exe -v"))

    def test_browser_runtime_is_built_and_staged_from_pinned_sources_before_signing(self) -> None:
        candidate = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        sources = json.loads(BROWSER_RUNTIME_SOURCES.read_text(encoding="utf-8"))

        self.assertEqual(sources["schema_version"], 1)
        self.assertEqual(set(sources["targets"]), {
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
        })
        for target, assets in sources["targets"].items():
            self.assertTrue(assets["chromium_archive_url"].startswith("https://"), target)
            self.assertRegex(assets["chromium_archive_sha256"], r"^[0-9a-f]{64}$")
            self.assertGreater(assets["chromium_archive_bytes"], 50_000_000)
            self.assertTrue(assets["chromium_required_files"])
            self.assertTrue(assets["bun_compiler_archive_url"].startswith("https://"), target)
            self.assertRegex(assets["bun_compiler_archive_sha256"], r"^[0-9a-f]{64}$")
            self.assertGreater(assets["bun_compiler_archive_bytes"], 20_000_000)
            self.assertTrue(assets["bun_compiler_member"])
        serialized = json.dumps(sources)
        self.assertNotIn("example.invalid", serialized)
        self.assertNotIn("placeholder", serialized.lower())

        self.assertIn("BROWSER_RUST_TOOLCHAIN: '1.95.0'", candidate)
        self.assertIn("BROWSER_BUN_VERSION: '1.3.8'", candidate)
        self.assertIn("cargo build --locked --release --target", candidate)
        for bun_target in ("bun-darwin-arm64", "bun-darwin-x64", "bun-windows-x64"):
            self.assertIn(bun_target, candidate)
        self.assertIn("scripts/runtime-stage.py", candidate)
        self.assertIn("scripts/runtime-stage-toolchain.py", candidate)
        self.assertIn("--compile-executable-path=\"$BUN_COMPILER\"", candidate)
        self.assertIn("release/browser-runtime-sources.json", candidate)
        self.assertIn("src-tauri/resources/browser-runtime", candidate)
        self.assertLess(
            candidate.index("Stage verified Browser Runtime"),
            candidate.index("Build, notarize, staple and normalize macOS candidate"),
        )

    def test_browser_runtime_platform_signing_precedes_final_metadata_hashes(self) -> None:
        macos = MACOS_BUILD.read_text(encoding="utf-8")
        windows = WINDOWS_BUILD.read_text(encoding="utf-8")
        mac_sign = BROWSER_MAC_SIGN.read_text(encoding="utf-8")
        win_sign = BROWSER_WINDOWS_SIGN.read_text(encoding="utf-8")

        self.assertLess(
            macos.index("runtime-stage-sign-macos.sh"),
            macos.index("browser-runtime-metadata.py prepare"),
        )
        self.assertLess(
            windows.index("runtime-stage-sign-windows.ps1"),
            windows.index("browser-runtime-metadata.py prepare"),
        )
        for required in ("codesign --force", "--timestamp", "--options runtime", "codesign --verify"):
            self.assertIn(required, mac_sign)
        self.assertIn("find \"$TARGET_ROOT\"", mac_sign)
        self.assertRegex(win_sign, re.compile(r"windows-authenticode\.ps1\"? -Mode Sign"))
        self.assertRegex(win_sign, re.compile(r"windows-authenticode\.ps1\"? -Mode Verify"))
        self.assertIn("*.exe", win_sign)
        self.assertIn("*.dll", win_sign)


if __name__ == "__main__":
    unittest.main()
