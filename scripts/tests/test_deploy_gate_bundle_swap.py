#!/usr/bin/env python3
"""deploy_gate.py 번들 교체 경로 회귀 테스트 (ATOMIC-1 글루).

`atomic_bundle.py` 의 계약 자체는 test_atomic_bundle.py 가 덮는다. 여기서 검사하는 건
deploy_gate 가 그 계약을 **올바르게 쓰는가**다 — 특히 종전 사고 형태였던
"살아있는 /Applications/cys.app 안에 개별 파일을 쓰고 그 자리에서 재서명" 이 사라졌는지.

★격리: 모듈 전역(APP_BUNDLE·SRC)을 임시 디렉터리로 갈아끼워 돈다. `/Applications` 는 읽지도
  쓰지도 않으며, deploy_gate.main()·게이트·데몬 재시작 경로는 호출하지 않는다.
실행: `python3 scripts/tests/test_deploy_gate_bundle_swap.py`
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import atomic_bundle as ab      # noqa: E402
import deploy_gate as dg        # noqa: E402  (import 만으로는 아무것도 실행되지 않는다)

# 실제 Mach-O 여야 codesign 이 의미 있게 돈다 — stock 바이너리 둘로 세대를 구분한다.
OLD_BIN, NEW_BIN = "/bin/echo", "/bin/date"


def put_binary(src, dst):
    """★`shutil.copy2` 를 쓰지 않는다: stock 바이너리(/bin/echo 등)에는 SIP 의 `restricted` 플래그가
    붙어 있어 copy2 의 `chflags` 복제가 EPERM 으로 죽는다(실측). 내용 + 실행비트만 옮긴다."""
    shutil.copyfile(src, dst)
    os.chmod(dst, 0o755)


def sha(p):
    with open(p, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def make_installed_bundle(root, binary=OLD_BIN):
    """설치본 픽스처 — 완본 정의(Info.plist·실행물 3종·필수 리소스)를 만족하고 ad-hoc 서명까지 된 번들."""
    app = os.path.join(root, "cys.app")
    macos = os.path.join(app, "Contents", "MacOS")
    res = os.path.join(app, "Contents", "Resources")
    os.makedirs(macos, exist_ok=True)
    os.makedirs(os.path.join(res, "runtime", "python", "bin"), exist_ok=True)
    with open(os.path.join(app, "Contents", "Info.plist"), "w") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>'
                '<plist version="1.0"><dict>'
                "<key>CFBundleExecutable</key><string>cys-app</string>"
                "<key>CFBundleIdentifier</key><string>com.example.cysfixture</string>"
                "<key>CFBundlePackageType</key><string>APPL</string>"
                "</dict></plist>")
    for name in dg.BUNDLE_EXECUTABLES:
        put_binary(binary, os.path.join(macos, name))
    put_binary(binary, os.path.join(res, "runtime", "python", "bin", "python3"))
    for f in ("pack.tar.gz", "pack-manifest.json"):
        open(os.path.join(res, f), "w").close()
    subprocess.run(["codesign", "--force", "--deep", "--sign", "-", app],
                   capture_output=True, check=True)
    return app


def make_build_output(root, binary=NEW_BIN):
    src = os.path.join(root, "release")
    os.makedirs(src, exist_ok=True)
    for name in dg.BUNDLE_EXECUTABLES:
        put_binary(binary, os.path.join(src, name))
    return src


class DeployGateBundleSwap(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="cys-deploygate-")
        self.install_dir = os.path.join(self.root, "Applications")
        os.makedirs(self.install_dir)
        self.app = make_installed_bundle(self.install_dir)
        self._saved = (dg.APP_BUNDLE, dg.SRC)
        dg.APP_BUNDLE, dg.SRC = self.app, make_build_output(self.root)

    def tearDown(self):
        dg.APP_BUNDLE, dg.SRC = self._saved
        shutil.rmtree(self.root, ignore_errors=True)

    def residue(self):
        return [n for n in os.listdir(self.install_dir) if n != "cys.app"]

    def test_deploy_never_writes_into_the_live_bundle(self):
        """★핵심 회귀 핀: 스테이징 단계가 끝난 시점에 **설치본은 아직 옛 세대 그대로**여야 한다.
        종전 구현은 이 시점에 이미 설치본 안이 새 바이너리로 덮여 있었다(= 반쪽 번들 창)."""
        old_hash = sha(os.path.join(self.app, "Contents", "MacOS", "cys"))
        staged = dg.stage_new_bundle()
        self.assertNotEqual(staged.rstrip("/"), self.app.rstrip("/"))
        self.assertTrue(os.path.basename(staged).startswith(".cys.app.deploy-staging-"))
        self.assertEqual(sha(os.path.join(self.app, "Contents", "MacOS", "cys")), old_hash,
                         "스테이징 도중 설치본이 변하면 안 된다(부분 쓰기 금지)")
        self.assertNotEqual(sha(os.path.join(staged, "Contents", "MacOS", "cys")), old_hash,
                            "스테이징 사본에는 새 바이너리가 들어가야 한다")
        # ③④⑤ 교체 — 이때 비로소 설치본이 바뀐다.
        previous = dg.swap_bundle_into_place(staged)
        self.assertNotEqual(sha(os.path.join(self.app, "Contents", "MacOS", "cys")), old_hash)
        self.assertEqual(ab.verify_bundle(self.app), [], "교체 후 완본 + 서명 유효")
        self.assertEqual(sha(os.path.join(previous, "Contents", "MacOS", "cys")), old_hash,
                         "옛 번들이 온전히 보존")
        self.assertEqual(subprocess.run(
            ["codesign", "--verify", "--deep", "--strict", self.app],
            capture_output=True).returncode, 0, "교체된 번들이 codesign 검증을 통과해야 한다")

    def test_missing_build_artifact_aborts_before_touching_the_install(self):
        """빌드 산출물이 없으면 스테이징에서 죽고 설치본은 무접촉이다(무증상 성공 금지)."""
        os.remove(os.path.join(dg.SRC, "cys-app"))
        old_hash = sha(os.path.join(self.app, "Contents", "MacOS", "cys-app"))
        with self.assertRaises(RuntimeError) as cm:
            dg.stage_new_bundle()
        self.assertIn("빌드 산출물 누락", str(cm.exception))
        self.assertIn("설치본 무접촉", str(cm.exception))
        self.assertEqual(sha(os.path.join(self.app, "Contents", "MacOS", "cys-app")), old_hash)
        self.assertEqual(ab.verify_bundle(self.app), [], "설치본은 여전히 완본")

    def test_incomplete_staged_bundle_is_refused_and_install_survives(self):
        """스테이징이 완본이 아니면(사고 상태 재현: Info.plist 유실) 교체를 거절하고 설치본을 지킨다."""
        staged = dg.stage_new_bundle()
        os.remove(os.path.join(staged, "Contents", "Info.plist"))
        with self.assertRaises(ab.BundleError) as cm:
            dg.swap_bundle_into_place(staged)
        self.assertIn("Info.plist", str(cm.exception))
        self.assertIn("기존 번들 무접촉", str(cm.exception))
        self.assertEqual(ab.verify_bundle(self.app), [], "설치본은 완본 그대로")
        self.assertEqual(self.residue(), [os.path.basename(staged)],
                         "잔해는 숨김 스테이징 하나뿐(설치본 옆 반쪽 없음)")

    def test_rollback_restores_backup_through_the_same_contract(self):
        """롤백도 같은 계약(스테이징→검증→단일 스왑)을 탄다 — 종전의 비원자 2단 rename 제거 확인."""
        backup = os.path.join(self.root, "backup", "cys.app")
        os.makedirs(os.path.dirname(backup), exist_ok=True)
        subprocess.run(["ditto", "--rsrc", "--extattr", "--acl", self.app, backup], check=True)
        old_hash = sha(os.path.join(backup, "Contents", "MacOS", "cys"))
        # inventory 는 실제 코드와 같은 방식으로 만든다 — macOS 가 `.app` 에 자동으로 붙이는
        # `com.apple.provenance` 를 [] 로 가정하면 롤백 검증이 거짓 실패한다(실측).
        xattrs = subprocess.run(["xattr", self.app], capture_output=True,
                                text=True).stdout.splitlines()
        # 설치본을 새 세대로 바꿔 놓고(배포 성공 상태) 롤백을 부른다.
        dg.swap_bundle_into_place(dg.stage_new_bundle())
        self.assertNotEqual(sha(os.path.join(self.app, "Contents", "MacOS", "cys")), old_hash)
        dg.rollback({"bundle_bak": backup, "bundle_xattrs": xattrs, "targets": []})
        self.assertEqual(sha(os.path.join(self.app, "Contents", "MacOS", "cys")), old_hash,
                         "롤백 후 설치본은 백업 세대여야 한다")
        self.assertEqual(ab.verify_bundle(self.app), [], "롤백 후 완본")

    # ── 공용 픽스처 ────────────────────────────────────────────────────────
    def _backup_of_install(self):
        backup = os.path.join(self.root, "backup", "cys.app")
        os.makedirs(os.path.dirname(backup), exist_ok=True)
        subprocess.run(["ditto", "--rsrc", "--extattr", "--acl", self.app, backup], check=True)
        return backup

    def _write_bytecode(self, bundle):
        """**실행 이력이 있는 번들**의 상태 재현 — 동봉 python 이 남긴 `.pyc` 한 건.
        파일 '추가' 한 건으로 코드서명 봉인은 깨지지만 구조는 완본 그대로다(SEAL-1 이전 세대의 일상)."""
        d = os.path.join(bundle, "Contents", "Resources", "runtime", "python", "lib", "__pycache__")
        os.makedirs(d, exist_ok=True)
        p = os.path.join(d, "os.cpython-313.pyc")
        with open(p, "w") as f:
            f.write("bytecode")
        return p

    def _third_party_write_right_after_swap(self, mutate):
        """스왑이 끝난 **직후·재검증 전**에 설치본을 건드리는 제3자(구 데몬·훅)를 시늉한다.
        진짜 스왑은 그대로 돌리고 부작용만 한 번 얹는다 — 검증 로직은 손대지 않는다(모킹 아님).
        ★단발(one-shot)이어야 한다: 원복도 같은 스왑을 쓰므로, 매번 발동하면 되돌린 옛 번들까지 부순다."""
        real = ab.swap_paths
        fired = []

        def hooked(a, b):
            real(a, b)
            if not fired:
                fired.append(True)
                mutate(b)

        return real, hooked

    # ── ★HIGH-1: 롤백 안전망이 스스로를 막지 않는가(양방향) ────────────────
    def test_rollback_restores_a_backup_whose_seal_is_legitimately_broken(self):
        """봉인은 깨졌지만 **구조는 완본**인 백업으로 되돌릴 수 있어야 한다.
        실행 이력이 있는 설치본의 사본은 거의 항상 이 상태다(.pyc 한 건이면 봉인은 깨진다).
        종전 구현은 계약 ②의 codesign 기본값 때문에 이런 백업을 **절대** 복원하지 못했다 —
        배포가 막 실패한, 안전망이 가장 절실한 순간에 안전망이 없어지는 결함."""
        backup = self._backup_of_install()
        self._write_bytecode(backup)
        # 전제 확인: 봉인은 깨졌고(=종전 구현이 거절하던 상태) 구조는 완본이다.
        self.assertTrue(any("코드서명" in d for d in ab.verify_bundle(backup)), "백업 봉인은 깨져 있다")
        self.assertEqual(ab.verify_bundle(backup, codesign=False), [], "그러나 구조는 완본")
        old_hash = sha(os.path.join(backup, "Contents", "MacOS", "cys"))
        xattrs = subprocess.run(["xattr", self.app], capture_output=True, text=True).stdout.splitlines()

        dg.swap_bundle_into_place(dg.stage_new_bundle())  # 배포 성공 상태를 만들고
        self.assertNotEqual(sha(os.path.join(self.app, "Contents", "MacOS", "cys")), old_hash)
        dg.rollback({"bundle_bak": backup, "bundle_xattrs": xattrs, "targets": []})  # 되돌린다

        self.assertEqual(sha(os.path.join(self.app, "Contents", "MacOS", "cys")), old_hash,
                         "롤백 후 설치본은 백업 세대여야 한다")
        self.assertEqual(ab.verify_bundle(self.app, codesign=False), [], "복원본은 구조 완본")
        self.assertTrue(os.path.exists(os.path.join(
            self.app, "Contents", "Resources", "runtime", "python", "lib",
            "__pycache__", "os.cpython-313.pyc")), "백업 상태를 **그대로** 되돌렸는가(충실도)")

    def test_rollback_refuses_a_structurally_broken_backup(self):
        """반대 방향 — '봉인을 기준에서 뺐다'가 '아무거나 되돌린다'가 되면 게이트를 없앤 것이다.
        구조가 깨진 백업(사고 상태: Info.plist 결손)은 거절하고, 설치본은 손대지 않는다."""
        backup = self._backup_of_install()
        os.remove(os.path.join(backup, "Contents", "Info.plist"))
        dg.swap_bundle_into_place(dg.stage_new_bundle())
        new_hash = sha(os.path.join(self.app, "Contents", "MacOS", "cys"))
        with self.assertRaises(SystemExit) as cm:
            dg.rollback({"bundle_bak": backup, "bundle_xattrs": [], "targets": []})
        self.assertEqual(cm.exception.code, 1, "복원 실패는 hard fail(무증상 성공 금지)")
        self.assertEqual(sha(os.path.join(self.app, "Contents", "MacOS", "cys")), new_hash,
                         "거절했으면 설치본은 손대지 않은 그대로여야 한다")
        self.assertEqual(ab.verify_bundle(self.app, codesign=False), [], "설치본은 여전히 완본")

    # ── ★HIGH-2: 스왑~재검증 사이 제3자 쓰기(양방향) ───────────────────────
    def test_bytecode_written_right_after_swap_does_not_undo_the_upgrade(self):
        """실제 0.14.9→0.14.10 사용자 경로: 구 데몬(SEAL-1 env 없음)이 절대경로로 동봉 python 을
        부르면, 스왑 직후 그 경로는 **새 번들**을 가리키고 인터프리터가 `.pyc` 를 쓴다.
        종전 구현은 ④의 codesign 이 그걸 결손으로 읽어 **성공한 업그레이드를 자동 원복**했다."""
        staged = dg.stage_new_bundle()
        new_hash = sha(os.path.join(staged, "Contents", "MacOS", "cys"))
        real, hooked = self._third_party_write_right_after_swap(self._write_bytecode)
        ab.swap_paths = hooked
        try:
            dg.swap_bundle_into_place(staged)
        finally:
            ab.swap_paths = real
        self.assertEqual(sha(os.path.join(self.app, "Contents", "MacOS", "cys")), new_hash,
                         "업그레이드가 유지돼야 한다(자동 원복 금지)")
        self.assertEqual(ab.verify_bundle(self.app, codesign=False), [], "설치본은 구조 완본")
        ok, benign, detail = dg.seal_status(self.app)
        self.assertFalse(ok, "봉인은 실제로 깨져 있다(사실을 숨기지 않는다)")
        self.assertTrue(benign, f"이탈 갈래는 '바이트코드 캐시 추가' 하나뿐이어야 한다: {detail}")

    def test_real_defect_right_after_swap_still_triggers_the_automatic_undo(self):
        """반대 방향 — 진짜 결손(Info.plist 유실)이면 종전대로 자동 원복이 돈다(계약 ⑤ 유지)."""
        old_hash = sha(os.path.join(self.app, "Contents", "MacOS", "cys"))
        staged = dg.stage_new_bundle()

        def wreck(b):
            os.remove(os.path.join(b, "Contents", "Info.plist"))

        real, hooked = self._third_party_write_right_after_swap(wreck)
        ab.swap_paths = hooked
        try:
            with self.assertRaises(ab.BundleError) as cm:
                dg.swap_bundle_into_place(staged)
        finally:
            ab.swap_paths = real
        self.assertIn("되돌렸다", str(cm.exception))
        self.assertIn("Info.plist", str(cm.exception))
        self.assertEqual(sha(os.path.join(self.app, "Contents", "MacOS", "cys")), old_hash,
                         "옛 세대로 되돌아와야 한다")
        self.assertEqual(ab.verify_bundle(self.app, codesign=False), [], "돌아온 번들은 완본")

    def test_seal_status_separates_benign_bytecode_from_real_tampering(self):
        """봉인 이탈의 **갈래 판정** — 이 판정이 '무엇을 롤백할 것인가'를 정한다.
        추가(.pyc)만 양성이고 변조·소실은 종전대로 실패다(게이트를 넓히는 장치가 아니다).
        ※codesign 은 이탈을 **전부** 한 줄씩 보고한다(400건+1건 혼합 실측) — 하나라도 섞이면 양성이 아니다."""
        self.assertEqual(dg.seal_status(self.app)[:2], (True, True), "무결한 번들은 통과")
        self._write_bytecode(self.app)
        ok, benign, detail = dg.seal_status(self.app)
        self.assertFalse(ok)
        self.assertTrue(benign, detail)
        # 변조가 섞이면 더 이상 양성이 아니다.
        with open(os.path.join(self.app, "Contents", "Resources", "pack-manifest.json"), "w") as f:
            f.write("TAMPERED")
        ok2, benign2, detail2 = dg.seal_status(self.app)
        self.assertFalse(ok2)
        self.assertFalse(benign2, f"변조가 섞였는데 양성으로 접으면 게이트가 뚫린다: {detail2}")
        # 소실도 마찬가지다.
        os.remove(os.path.join(self.app, "Contents", "Resources", "pack-manifest.json"))
        os.remove(os.path.join(self.app, "Contents", "Resources", "pack.tar.gz"))
        self.assertFalse(dg.seal_status(self.app)[1], "소실은 양성이 아니다")

    def test_crash_recovery_sweeps_staging_but_keeps_the_install(self):
        for pid in ("101", "202"):
            os.makedirs(os.path.join(self.install_dir, f".cys.app.deploy-staging-{pid}"))
        dg.crash_recovery()
        self.assertEqual(self.residue(), [], "스테이징 잔해만 정리")
        self.assertEqual(ab.verify_bundle(self.app), [], "설치본은 무사")


if __name__ == "__main__":
    if sys.platform != "darwin":
        print("macOS 전용 — skip")
        sys.exit(0)
    unittest.main(verbosity=2)
