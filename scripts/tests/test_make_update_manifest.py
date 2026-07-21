import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "make-update-manifest.sh"


class MakeUpdateManifestTests(unittest.TestCase):
    def test_x64_target_reads_its_own_bundle_and_emits_x64_fragment(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            bundle = work / "bundle"
            output = work / "output"
            macos = bundle / "macos"
            macos.mkdir(parents=True)
            (macos / "cys.app.tar.gz").write_bytes(b"intel-updater")
            (macos / "cys.app.tar.gz.sig").write_text("intel-signature\n")
            env = {
                **os.environ,
                "CYS_RELEASE_BUNDLE_ROOT": str(bundle),
                "CYS_RELEASE_OUTPUT_DIR": str(output),
            }

            result = subprocess.run(
                [
                    "sh",
                    str(SCRIPT),
                    "9.8.7",
                    "idoforgod",
                    "cys-terminal",
                    "x86_64-apple-darwin",
                ],
                cwd=ROOT,
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                (output / "cys_9.8.7_x64.app.tar.gz").read_bytes(), b"intel-updater"
            )
            self.assertEqual(
                (output / "cys_9.8.7_x64.app.tar.gz.sig").read_text(),
                "intel-signature\n",
            )
            fragment = json.loads(
                (output / "latest-darwin-x86_64.json").read_text(encoding="utf-8")
            )
            self.assertEqual(set(fragment["platforms"]), {"darwin-x86_64"})
            self.assertTrue(
                fragment["platforms"]["darwin-x86_64"]["url"].endswith(
                    "/cys_9.8.7_x64.app.tar.gz"
                )
            )


if __name__ == "__main__":
    unittest.main()
