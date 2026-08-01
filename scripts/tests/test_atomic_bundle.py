#!/usr/bin/env python3
"""atomic_bundle.py 회귀 테스트 — 2026-08-01 '반쪽 번들' 사고의 재발 차단선.

전부 **격리 임시 디렉터리**에서 돈다. `/Applications` 는 읽지도 쓰지도 않는다.
실행: `python3 scripts/tests/test_atomic_bundle.py`
"""
import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import atomic_bundle as ab  # noqa: E402

EXECS = ("cys-app", "cysd", "cys")
# 코드서명·리소스는 픽스처 범위 밖 — 구조 계약만 본다(서명 경로는 Rust 쪽 실물 게이트가 담당).
VK = {"executables": EXECS, "resources": (), "codesign": False}


def make_bundle(root, name, marker, execs=EXECS):
    app = os.path.join(root, name)
    os.makedirs(os.path.join(app, "Contents", "MacOS"), exist_ok=True)
    os.makedirs(os.path.join(app, "Contents", "Resources"), exist_ok=True)
    with open(os.path.join(app, "Contents", "Info.plist"), "w") as f:
        f.write(f"<plist><dict><key>Marker</key><string>{marker}</string></dict></plist>")
    for e in execs:
        p = os.path.join(app, "Contents", "MacOS", e)
        with open(p, "w") as f:
            f.write(f"#!/bin/sh\necho {marker}\n")
        os.chmod(p, 0o755)
    return app


def marker_of(app):
    try:
        with open(os.path.join(app, "Contents", "Info.plist")) as f:
            return f.read()
    except OSError:
        return ""


def residue(parent, dest_name):
    """설치 위치에 남은 우리 잔해 — '부분 상태 잔존 금지'의 기계 판정."""
    return [n for n in os.listdir(parent) if n.startswith(dest_name) and n != dest_name]


class AtomicBundleContract(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="cys-atomic-py-")
        self.install_dir = os.path.join(self.root, "Applications")
        os.makedirs(self.install_dir)

    def tearDown(self):
        os.chmod(self.install_dir, 0o755)  # 권한 테스트가 남긴 상태 복원
        shutil.rmtree(self.root, ignore_errors=True)

    # ① 정상 교체
    def test_replaces_bundle_and_preserves_the_old_one(self):
        dest = make_bundle(self.install_dir, "cys.app", "OLD")
        staged = ab.staging_path(dest)
        ab.stage_copy(make_bundle(self.root, "new.app", "NEW"), staged)
        previous = ab.install_atomically(staged, dest, VK)
        self.assertIn("NEW", marker_of(dest), "목적지는 새 세대")
        self.assertIn("OLD", marker_of(previous), "옛 번들이 온전히 보존")
        self.assertEqual(ab.verify_bundle(dest, **VK), [], "교체 후 완본")
        self.assertEqual(residue(self.install_dir, "cys.app"), [], "설치 위치 잔해 0")

    # ② 완본 검증 실패 → 기존 번들 보존(목적지 무접촉)
    def test_incomplete_new_bundle_never_touches_the_installed_one(self):
        dest = make_bundle(self.install_dir, "cys.app", "OLD")
        staged = ab.staging_path(dest)
        ab.stage_copy(make_bundle(self.root, "new.app", "NEW"), staged)
        os.remove(os.path.join(staged, "Contents", "Info.plist"))  # 사고 상태 재현
        with self.assertRaises(ab.BundleError) as cm:
            ab.install_atomically(staged, dest, VK)
        self.assertIn("Info.plist", str(cm.exception))
        self.assertIn("기존 번들 무접촉", str(cm.exception), "무증상 실패 금지 — 상태를 말해야 한다")
        self.assertIn("OLD", marker_of(dest), "기존 번들 그대로")
        self.assertEqual(ab.verify_bundle(dest, **VK), [], "기존 번들은 여전히 완본")
        self.assertEqual(residue(self.install_dir, "cys.app"), [])

    # ②-b 사이드카 유실·0바이트·실행권한 유실도 각각 이름으로 잡힌다
    def test_verify_names_each_missing_part(self):
        app = make_bundle(self.root, "partial.app", "X")
        os.remove(os.path.join(app, "Contents", "MacOS", "cysd"))
        open(os.path.join(app, "Contents", "MacOS", "cys"), "w").close()
        os.chmod(os.path.join(app, "Contents", "MacOS", "cys-app"), 0o644)
        d = " | ".join(ab.verify_bundle(app, **VK))
        self.assertIn("MacOS/cysd 없음", d)
        self.assertIn("MacOS/cys 0바이트", d)
        self.assertIn("MacOS/cys-app 실행 권한 없음", d)
        # 끊어진 심볼릭 링크 = 없는 것(실행 시점에 실패할 파일을 통과시키지 않는다)
        app2 = make_bundle(self.root, "dangling.app", "X")
        p = os.path.join(app2, "Contents", "MacOS", "cysd")
        os.remove(p)
        os.symlink("/nonexistent/cysd", p)
        self.assertIn("MacOS/cysd 없음", " | ".join(ab.verify_bundle(app2, **VK)))

    # ③ 교체 도중 실패 → 반쪽 상태 미잔존 (진짜 EACCES 로 실패시킨다)
    def test_no_half_bundle_when_replace_fails(self):
        dest = make_bundle(self.install_dir, "cys.app", "OLD")
        staged = ab.staging_path(dest)
        ab.stage_copy(make_bundle(self.root, "new.app", "NEW"), staged)
        self.assertEqual(ab.verify_bundle(staged, **VK), [], "스테이징은 완본(②는 통과)")
        os.chmod(self.install_dir, 0o555)  # 부모 디렉터리 쓰기 금지 → rename/스왑이 EACCES
        try:
            with self.assertRaises(Exception):
                ab.install_atomically(staged, dest, VK)
        finally:
            os.chmod(self.install_dir, 0o755)
        self.assertIn("OLD", marker_of(dest), "기존 번들이 옛 세대 그대로")
        self.assertEqual(ab.verify_bundle(dest, **VK), [], "반쪽이 아니라 완본")
        self.assertEqual(residue(self.install_dir, "cys.app"), [], "잔해 0")

    # ③-b 폴백(rename 2단)의 실패·자동 원복
    def test_two_step_fallback_restores_on_failure(self):
        dest = make_bundle(self.install_dir, "cys.app", "OLD")
        ghost = os.path.join(self.install_dir, ".cys.app.deploy-staging-ghost")
        with self.assertRaises(ab.BundleError) as cm:
            ab._two_step_replace(ghost, dest)
        self.assertIn("원복 완료", str(cm.exception))
        self.assertIn("OLD", marker_of(dest), "옛 번들이 제자리로")
        self.assertEqual(residue(self.install_dir, "cys.app"), [], "대피본 잔해 0")

    # ③-c ★MEDIUM-1: **이중 실패**(원자 스왑 거부 → 대피 rename 마저 실패)에서도 계약 예외로만 나간다.
    #      실 `/Applications` 에서 App Management(TCC)가 개입하는 형태와 가장 닮은 갈래다.
    #      여기서 raw OSError 가 새면 ⓐ호출부가 '기존 번들이 어떤 상태인지'를 타입으로 알 수 없고
    #      ⓑ정본 Rust(`InstallError::SwapFailed` · `destination_intact()==true`)와 계약이 갈라진다.
    def test_park_failure_raises_contract_error_not_raw_oserror(self):
        dest = make_bundle(self.install_dir, "cys.app", "OLD")
        staged = ab.staging_path(dest)
        ab.stage_copy(make_bundle(self.root, "new.app", "NEW"), staged)
        os.chmod(self.install_dir, 0o555)  # 대피 rename 이 **진짜 EACCES** 로 실패한다(모킹 아님)
        try:
            with self.assertRaises(ab.BundleError) as cm:
                ab._two_step_replace(staged, dest)
        finally:
            os.chmod(self.install_dir, 0o755)
        self.assertIn("기존 번들 무접촉", str(cm.exception), "무증상 실패 금지 — 상태를 말해야 한다")
        self.assertIn(dest, str(cm.exception), "손댈 위치를 경로로 지목해야 한다")
        self.assertIn("OLD", marker_of(dest), "대피에 실패했으면 설치본은 손도 대지 못한 것이다")
        self.assertEqual(ab.verify_bundle(dest, **VK), [], "설치본은 여전히 완본")
        self.assertEqual(residue(self.install_dir, "cys.app"), [], "설치 위치 잔해 0")

    # ③-d 같은 이중 실패를 **공개 API 로** 통과시켜도 계약 예외뿐이다(폴백 진입은 스왑 미지원 FS 시늉).
    def test_install_atomically_wraps_the_double_failure_path(self):
        dest = make_bundle(self.install_dir, "cys.app", "OLD")
        staged = ab.staging_path(dest)
        ab.stage_copy(make_bundle(self.root, "new.app", "NEW"), staged)

        def refuse(a, b):  # ENOTSUP — RENAME_SWAP 을 못 쓰는 파일시스템(=폴백 진입 조건)
            raise OSError(45, "Operation not supported")

        real_swap = ab.swap_paths
        ab.swap_paths = refuse
        os.chmod(self.install_dir, 0o555)
        try:
            with self.assertRaises(ab.BundleError) as cm:
                ab.install_atomically(staged, dest, VK)
        finally:
            os.chmod(self.install_dir, 0o755)
            ab.swap_paths = real_swap
        self.assertIn("기존 번들 무접촉", str(cm.exception))
        self.assertIn("OLD", marker_of(dest))
        self.assertEqual(ab.verify_bundle(dest, **VK), [], "설치본은 여전히 완본")

    # ④-b ★HIGH-2 뼈대: ④의 기준을 ②와 **다르게 좁힐 수 있어야** 한다(정본 Rust
    #      `install_bundle_atomically_verifying` 와 같은 구멍). 좁힌 항목 때문에 원복되면 안 되고,
    #      좁히지 않으면 종전대로 원복돼야 한다 — 같은 픽스처로 양방향 대조.
    def test_post_verify_kwargs_narrows_only_the_after_swap_check(self):
        def fixture(tag):
            root = os.path.join(self.root, tag)
            os.makedirs(root)
            dest = make_bundle(root, "cys.app", "OLD")
            staged = ab.staging_path(dest)
            ab.stage_copy(make_bundle(root, "new.app", "NEW"), staged)
            # cysd 를 '스테이징 경로 안'을 가리키는 절대 링크로 → 스왑 전엔 실체가 있고(②통과),
            # 스왑 후엔 그 경로에 옛 번들이 들어와 끊긴다(④에서만 결손).
            real = os.path.join(staged, "Contents", "Resources", "real-cysd")
            with open(real, "w") as f:
                f.write("#!/bin/sh\n")
            os.chmod(real, 0o755)
            link = os.path.join(staged, "Contents", "MacOS", "cysd")
            os.remove(link)
            os.symlink(real, link)
            return dest, staged

        narrow = dict(VK, executables=("cys-app", "cys"))  # ④에서만 cysd 를 뺀다
        dest, staged = fixture("narrow")
        self.assertEqual(ab.verify_bundle(staged, **VK), [], "②는 넓은 기준으로도 통과")
        ab.install_atomically(staged, dest, VK, post_verify_kwargs=narrow)
        self.assertIn("NEW", marker_of(dest), "④에서 뺀 항목 때문에 원복되면 안 된다")

        dest2, staged2 = fixture("same")
        with self.assertRaises(ab.BundleError) as cm:  # 대조군: 같은 기준이면 종전대로 원복
            ab.install_atomically(staged2, dest2, VK)
        self.assertIn("되돌렸다", str(cm.exception))
        self.assertIn("OLD", marker_of(dest2), "게이트는 살아 있다")

    # ④ 교체 후 검증 실패 → 완전 원복.
    #    스테이징 안의 절대경로 심볼릭 링크를 쓴다: 교체 전엔 실체가 있고(②통과), 스왑 후엔
    #    그 경로에 옛 번들이 들어와 링크가 끊긴다(④실패) — 모킹 없는 결정론 재현.
    def test_rolls_back_when_post_swap_verification_fails(self):
        dest = make_bundle(self.install_dir, "cys.app", "OLD")
        staged = ab.staging_path(dest)
        ab.stage_copy(make_bundle(self.root, "new.app", "NEW"), staged)
        real = os.path.join(staged, "Contents", "Resources", "real-cysd")
        with open(real, "w") as f:
            f.write("#!/bin/sh\n")
        os.chmod(real, 0o755)
        link = os.path.join(staged, "Contents", "MacOS", "cysd")
        os.remove(link)
        os.symlink(real, link)
        self.assertEqual(ab.verify_bundle(staged, **VK), [], "교체 전에는 완본으로 보인다")
        with self.assertRaises(ab.BundleError) as cm:
            ab.install_atomically(staged, dest, VK)
        self.assertIn("되돌렸다", str(cm.exception))
        self.assertIn("OLD", marker_of(dest), "옛 번들이 제자리로")
        self.assertEqual(ab.verify_bundle(dest, **VK), [], "돌아온 번들은 완본")
        self.assertEqual(residue(self.install_dir, "cys.app"), [], "잔해 0")

    # 신규 설치(옛 번들 없음)
    def test_first_time_install(self):
        dest = os.path.join(self.install_dir, "cys.app")
        staged = ab.staging_path(dest)
        ab.stage_copy(make_bundle(self.root, "new.app", "NEW"), staged)
        self.assertIsNone(ab.install_atomically(staged, dest, VK))
        self.assertIn("NEW", marker_of(dest))

    # 다른 파일시스템은 복사 폴백 없이 거절(복사 폴백이 곧 사고의 기제)
    def test_cross_filesystem_is_refused(self):
        self.assertTrue(ab.same_filesystem(self.root, self.install_dir))
        self.assertFalse(ab.same_filesystem(self.root, "/nonexistent-volume-xyz"))

    # 복사 도구 rc 를 반드시 본다(rc 무시 = 반쪽 복사본을 완본으로 넘김)
    def test_stage_copy_reports_failure(self):
        with self.assertRaises(ab.BundleError):
            ab.stage_copy(os.path.join(self.root, "nope.app"), os.path.join(self.root, ".stage"))

    # RENAME_SWAP 자체의 계약(디렉터리 스왑 · 대상 부재 시 실패)
    def test_swap_paths_is_all_or_nothing(self):
        a = make_bundle(self.root, "A.app", "AAA")
        b = make_bundle(self.root, "B.app", "BBB")
        ab.swap_paths(a, b)
        self.assertIn("BBB", marker_of(a))
        self.assertIn("AAA", marker_of(b))
        with self.assertRaises(OSError):
            ab.swap_paths(a, os.path.join(self.root, "missing.app"))
        self.assertIn("BBB", marker_of(a), "실패한 스왑은 아무것도 바꾸지 않는다")

    # 스테이징 잔해 청소
    def test_sweep_stale_staging(self):
        dest = os.path.join(self.install_dir, "cys.app")
        for pid in ("111", "222"):
            os.makedirs(os.path.join(self.install_dir, f".cys.app.deploy-staging-{pid}"))
        make_bundle(self.install_dir, "cys.app", "KEEP")
        swept = ab.sweep_stale_staging(dest)
        self.assertEqual(len(swept), 2)
        self.assertIn("KEEP", marker_of(dest), "설치본은 건드리지 않는다")


if __name__ == "__main__":
    if sys.platform != "darwin":
        print("macOS 전용(renamex_np) — skip")
        sys.exit(0)
    unittest.main(verbosity=2)
