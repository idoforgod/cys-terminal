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


# ── T11 · RULINGS BQ1(d) · 오너 지시 추가 11 — install-core.sh 의 동봉 python 실행 가능성 검사 ──────────────
# install-core.sh 자체는 Team ID(Q43YA2NMF9)·공증 게이트 때문에 픽스처로 끝까지 돌릴 수 없다. 그래서 그 파일의
# `# >>> T11-PY-RUNNABLE` … `# <<< T11-PY-RUNNABLE` 블록을 **그대로** 떼어 /bin/bash 로 돌리고, 기계 조건
# (칩 · Rosetta 2 · macOS 버전)을 인자로 주입해 합성 Mach-O 헤더에 대해 판정·문장·종료 코드를 잰다.
# 순서(진위 ①②③ → 이 검사 → python exec)와 설치 도우미의 승격 조건(exit 1 만)은 소스 핀으로 잰다.
CORE = os.path.join(HERE, "..", "installer-app", "install-core.sh")
APPLET = os.path.join(HERE, "..", "installer-app", "installer.applescript")
X86_64, ARM64 = 0x01000007, 0x0100000C
SENT_INTEL = ("이 설치 파일은 Apple 실리콘(M1·M2 등) Mac 전용이라 이 인텔 Mac에서는 설치할 수 없습니다 — "
              "www.cysinsight.com 에서 인텔용 설치 파일(이름에 x64가 들어간 .dmg)을 받아 다시 설치해 주세요.")
SENT_AS = ("이 설치 파일은 인텔 Mac용이라 Rosetta 2가 없는 이 Apple 실리콘(M1·M2 등) Mac에서는 설치할 수 없습니다 — "
           "www.cysinsight.com 에서 Apple 실리콘용 설치 파일(이름에 aarch64가 들어간 .dmg)을 받아 다시 설치해 주세요.")


def _ver(s):
    p = [int(x) for x in s.split(".")] + [0, 0]
    return (p[0] << 16) | (p[1] << 8) | p[2]


def _thin(cputype, minos=None, legacy=None):
    import struct
    cmds, n = b"", 0
    if minos:
        cmds += struct.pack("<IIIIII", 0x32, 24, 1, _ver(minos), _ver("15.0"), 0)
        n += 1
    if legacy:
        cmds += struct.pack("<IIII", 0x24, 16, _ver(legacy), _ver("10.15"))
        n += 1
    return struct.pack("<IIIIIIII", 0xFEEDFACF, cputype, 3, 2, n, len(cmds), 0, 0) + cmds + b"\0" * 64


def _fat(slices):
    import struct
    table, body, off = b"", b"", 4096
    for cputype, data in slices:
        pad = b"\0" * (4096 - len(data) % 4096)
        table += struct.pack(">IIIII", cputype, 3, off, len(data), 12)
        body += data + pad
        off += len(data) + len(pad)
    head = struct.pack(">II", 0xCAFEBABE, len(slices)) + table
    return head + b"\0" * (4096 - len(head)) + body


class PythonRunnableCheck(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="cys-pyrun-")
        with open(CORE, encoding="utf-8") as f:
            src = f.read()
        i, j = src.index("# >>> T11-PY-RUNNABLE"), src.index("# <<< T11-PY-RUNNABLE")
        self.driver = os.path.join(self.root, "driver.sh")
        with open(self.driver, "w", encoding="utf-8") as f:
            f.write("#!/bin/bash\nset -euo pipefail\n" + src[i:j] + '\nic_python_runnable "$1" "$2" "$3" "$4"\n')

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def py(self, data, name="python3.12"):
        p = os.path.join(self.root, "runtime", "python", "bin", name)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "wb") as f:
            f.write(data)
        os.chmod(p, 0o755)
        link = os.path.join(os.path.dirname(p), "python3")
        if name != "python3" and not os.path.lexists(link):
            os.symlink(name, link)            # 실제 번들처럼 python3 → python3.12 링크를 통해 읽는다
        return link if name != "python3" else p

    def run_check(self, py, hw, rosetta, os_ver):
        bash = "/bin/bash" if os.path.exists("/bin/bash") else "bash"
        r = subprocess.run([bash, self.driver, py, hw, rosetta, os_ver], capture_output=True, text=True, timeout=60)
        return r.returncode, r.stdout, r.stderr.strip()

    def test_arm64_only_python_on_an_intel_mac_says_one_korean_sentence_and_exits_5(self):
        rc, out, err = self.run_check(self.py(_thin(ARM64, minos="11.0")), "x86_64", "no", "13.6")
        self.assertEqual((rc, out, err), (5, "", SENT_INTEL))
        self.assertNotIn("Bad CPU", err)

    def test_x86_64_python_on_an_intel_mac_runs(self):
        rc, out, err = self.run_check(self.py(_thin(X86_64, minos="10.15")), "x86_64", "no", "10.15.7")
        self.assertEqual((rc, err), (0, ""))

    def test_intel_python_on_apple_silicon_needs_rosetta(self):
        py = self.py(_thin(X86_64, minos="10.15"))
        rc, out, err = self.run_check(py, "arm64", "no", "14.5")
        self.assertEqual((rc, err), (5, SENT_AS))
        rc, out, err = self.run_check(py, "arm64", "yes", "14.5")
        self.assertEqual((rc, err), (0, ""), "an Intel python runs on Apple silicon with Rosetta 2")

    def test_arm64_and_universal_pythons_on_apple_silicon_run(self):
        self.assertEqual(self.run_check(self.py(_thin(ARM64, minos="11.0")), "arm64", "no", "14.5")[0], 0)
        uni = self.py(_fat([(X86_64, _thin(X86_64, minos="10.15")), (ARM64, _thin(ARM64, minos="11.0"))]), "python3")
        self.assertEqual(self.run_check(uni, "arm64", "no", "14.5")[0], 0)
        self.assertEqual(self.run_check(uni, "x86_64", "no", "10.15")[0], 0, "the x86_64 slice's own minimum (10.15) applies")

    def test_macos_older_than_the_python_minimum_names_both_versions(self):
        rc, out, err = self.run_check(self.py(_thin(X86_64, minos="10.15")), "x86_64", "no", "10.14.6")
        self.assertEqual(rc, 5)
        self.assertEqual(err, "이 Mac의 macOS 버전(10.14.6)이 낮아 cys를 설치할 수 없습니다 — "
                              "macOS 10.15 이상으로 업데이트한 뒤 다시 설치해 주세요.")
        rc, out, err = self.run_check(self.py(_thin(X86_64, legacy="10.13"), "python3"), "x86_64", "no", "10.12.6")
        self.assertEqual(rc, 5, "LC_VERSION_MIN_MACOSX is read too")
        self.assertIn("macOS 10.13 이상", err)

    def test_unreadable_header_does_not_block(self):
        for data in (b"#!/bin/sh\necho hi\n", b"\xcf\xfa\xed\xfe\x07"):
            p = self.py(data, "python3")
            self.assertEqual(self.run_check(p, "x86_64", "no", "13.0")[:2], (0, ""))
            os.remove(p)

    def test_order_authenticity_first_then_this_check_then_exec_and_exit_5_is_not_escalated(self):
        with open(CORE, encoding="utf-8") as f:
            src = f.read()
        i_spctl = src.index('/usr/sbin/spctl --assess --type execute "$SRC"')
        i_teamid = src.index('[ "$SRC_TEAM" = "$EXPECTED_TEAM_ID" ]')
        i_sign = src.index('/usr/bin/codesign --verify --strict "$SRC"')
        i_block = src.index("# >>> T11-PY-RUNNABLE")
        i_call = src.index('ic_python_runnable "$PY" "$IC_HW" "$IC_ROSETTA" "$IC_OS" || exit 5')
        i_exec = src.index('exec "$PY" "$WRAPPER" install "$SRC" "$DEST"')
        self.assertTrue(i_sign < i_teamid < i_spctl < i_block < i_call < i_exec,
                        "order must stay: signature -> Team ID -> notarisation -> (read-only) python check -> exec")
        self.assertIn("5=이 Mac 에서 소스 동봉 python 을 실행할 수 없음", src)
        block = src[i_block:src.index("# <<< T11-PY-RUNNABLE")]
        import re
        runs_python = re.compile(r'(^\s*|[;&|(]\s*|\$\(\s*)(exec\s+)?"?\$\{?(py|PY)\}?"?(\s|$)')
        for line in block.splitlines():
            code = line.split("#", 1)[0] if line.lstrip().startswith("#") else line
            self.assertIsNone(runs_python.search(code), "the check must never run the bundled python: " + line)
        with open(APPLET, encoding="utf-8") as f:
            applet = f.read()
        self.assertIn("if errNum is 1 then", applet, "only exit 1 may trigger the administrator retry")


if __name__ == "__main__":
    if sys.platform != "darwin":
        print("SKIP: macOS 전용(codesign/renamex_np)")
        raise SystemExit(0)
    unittest.main(verbosity=2)
