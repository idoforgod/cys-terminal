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


if __name__ == "__main__":
    unittest.main()
