#!/usr/bin/env python3
"""설치 도우미(Install cys.app) 원자 설치 회귀 테스트 — 스파이크 DEMO2/DEMO3 를 CI 로 편입.

무엇을 못박는가:
  · `atomic_bundle.py install <src> <dst>` CLI 래퍼가 정본 ATOMIC-1 을 그대로 집행한다(로직 이중구현 없음).
  · 신규 설치(단일 rename) · 업그레이드(renamex_np SWAP) 후에도 목적지는 **완본 + 서명 유효**.
  · DEMO3(Gatekeeper 승계): ad-hoc 서명이 ditto 스테이징 → 원자 교체를 거쳐 **codesign --deep --strict
    통과**로 승계된다(공증 티켓/spctl 은 실서명·공증이 필요 → CI 서명 잡 몫, 여기선 봉인 승계만 실증).
  · 설치본에 반쪽/잔해가 남지 않고, 소스는 읽기 전용으로 무접촉.
  · 소스가 .app 이 아니면 exit 2 로 명시 실패(무음 반쪽설치 금지).

전부 격리 임시 디렉터리에서 돈다. `/Applications` 는 읽지도 쓰지도 않는다.
실행: `python3 scripts/tests/test_installer_atomic.py`
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
WRAPPER = os.path.join(HERE, "..", "atomic_bundle.py")
EXECS = ("cys-app", "cysd", "cys")
# 정본 verify 의 필수 리소스 목록(REQUIRED_RESOURCES)과 동일 — 픽스처가 이걸 갖춰야 ② 완본 검증 통과.
RESOURCES = (
    "runtime/python/bin/python3",
    "pack.tar.gz",
    "pack-manifest.json",
)

INFO_PLIST = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>com.cysjavis.terminal.test</string>
  <key>CFBundleExecutable</key><string>cys-app</string>
  <key>CFBundleName</key><string>cys</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>Marker</key><string>%s</string>
</dict></plist>
"""


def make_signed_bundle(root, name, marker):
    """cys.app 픽스처 — 필수 실행물(실 Mach-O)·필수 리소스·Info.plist 구비 후 ad-hoc(--deep) 서명."""
    app = os.path.join(root, name)
    macos = os.path.join(app, "Contents", "MacOS")
    res = os.path.join(app, "Contents", "Resources")
    os.makedirs(macos, exist_ok=True)
    os.makedirs(res, exist_ok=True)
    with open(os.path.join(app, "Contents", "Info.plist"), "w") as f:
        f.write(INFO_PLIST % marker)
    for e in EXECS:  # 실 Mach-O 여야 codesign 이 깔끔 → 시스템 /bin/echo 내용만 복사(제한 flags 승계 회피)
        p = os.path.join(macos, e)
        shutil.copyfile("/bin/echo", p)
        os.chmod(p, 0o755)
    for rel in RESOURCES:
        p = os.path.join(res, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as f:
            f.write(f"placeholder:{marker}:{rel}\n")
    subprocess.run(["/usr/bin/codesign", "--force", "--deep", "--sign", "-", app],
                   check=True, capture_output=True, text=True)
    return app


def codesign_ok(path, deep=False):
    args = ["/usr/bin/codesign", "--verify", "--strict"]
    if deep:
        args.append("--deep")
    return subprocess.run(args + [path], capture_output=True).returncode == 0


def marker_of(app):
    try:
        with open(os.path.join(app, "Contents", "Info.plist")) as f:
            return f.read()
    except OSError:
        return ""


def residue(parent, dest_name):
    return [n for n in os.listdir(parent) if n.startswith(dest_name) and n != dest_name]


def run_install(src, dest):
    return subprocess.run([sys.executable, WRAPPER, "install", src, dest],
                          capture_output=True, text=True)


class InstallerAtomicContract(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="cys-installer-test-")
        self.apps = os.path.join(self.root, "Applications")
        os.makedirs(self.apps)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    # ── 신규 설치: 단일 rename → 완본 + 서명 유효 + 소스 무접촉 + 잔해 0 ──
    def test_fresh_install_is_atomic_and_signed(self):
        src = make_signed_bundle(self.root, "src-cys.app", "SRC1")
        dest = os.path.join(self.apps, "cys.app")
        r = run_install(src, dest)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(os.path.isfile(os.path.join(dest, "Contents", "Info.plist")))
        self.assertIn("SRC1", marker_of(dest))
        # DEMO3 승계: 원자 교체 후에도 서명이 유효(deep 포함).
        self.assertTrue(codesign_ok(dest), "설치본 codesign --verify --strict 실패")
        self.assertTrue(codesign_ok(dest, deep=True), "설치본 codesign --deep --strict 실패(봉인 승계 깨짐)")
        self.assertEqual(residue(self.apps, "cys.app"), [], "설치 위치에 스테이징/반쪽 잔해가 남음")
        self.assertTrue(codesign_ok(src), "소스가 설치 과정에서 변조됨(읽기 전용 위반)")

    # ── 업그레이드: renamex_np SWAP → 새 세대로 교체 + 서명 유효 + 잔해 0 ──
    def test_upgrade_swaps_generation_atomically(self):
        dest = make_signed_bundle(self.apps, "cys.app", "OLD")
        self.assertIn("OLD", marker_of(dest))
        src = make_signed_bundle(self.root, "src-cys.app", "NEW")
        r = run_install(src, dest)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("NEW", marker_of(dest), "업그레이드 후 새 세대로 교체되지 않음")
        self.assertNotIn("OLD", marker_of(dest))
        self.assertTrue(codesign_ok(dest, deep=True), "업그레이드 후 봉인 승계 깨짐")
        self.assertEqual(residue(self.apps, "cys.app"), [], "업그레이드 후 옛 번들/스테이징 잔해가 남음")
        self.assertTrue(codesign_ok(src), "소스가 업그레이드 과정에서 변조됨")

    # ── 소스가 .app 이 아니면 명시 실패(exit 2) — 무음 반쪽설치 금지 ──
    def test_nonbundle_source_fails_loud(self):
        bogus = os.path.join(self.root, "not-an-app")
        os.makedirs(bogus)
        dest = os.path.join(self.apps, "cys.app")
        r = run_install(bogus, dest)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertFalse(os.path.exists(dest), "실패했는데 목적지에 무언가 생성됨")

    # ── 반쪽/변조 소스(Info.plist 없음)는 봉인 게이트에서 거부 · 목적지 무접촉 ──
    def test_broken_source_is_rejected_before_touching_dest(self):
        src = make_signed_bundle(self.root, "src-cys.app", "BROKEN")
        os.remove(os.path.join(src, "Contents", "Info.plist"))  # 봉인·구조 동시 파손
        dest = make_signed_bundle(self.apps, "cys.app", "KEEP")
        r = run_install(src, dest)
        self.assertNotEqual(r.returncode, 0, "반쪽 소스가 설치를 통과했다")
        self.assertIn("KEEP", marker_of(dest), "실패 시 기존 번들이 보존되지 않음")
        self.assertEqual(residue(self.apps, "cys.app"), [], "실패 후 설치 위치에 잔해가 남음")


if __name__ == "__main__":
    if sys.platform != "darwin":
        print("SKIP: macOS 전용(codesign/renamex_np)")
        raise SystemExit(0)
    unittest.main(verbosity=2)
