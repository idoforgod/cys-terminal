#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_c11b_seal.py — C11b cys-dept 심링크의 코드서명 봉인 보호 회귀 하네스 (standalone).

★왜 이 파일이 존재하는가(2026-08-28 실사고 · W3c):
  종전 C11b 는 링크를 `dirname(shutil.which("cys"))` 에 만들었는데 페인 PATH 선두가
  `/Applications/cys.app/Contents/MacOS` 라서 **앱 번들 안에** 심링크가 생겼고, codesign
  봉인이 깨졌다(spctl "a sealed resource is missing or invalid" — C76 이 검출하는 그 파손의
  생산자가 preflight 자신이었다). 게다가 which("cys-dept") realpath 일치 PASS 단락이 번들 안
  링크를 영원히 PASS 로 덮어 이행이 구조적으로 불가능했다. 이 하네스는 재설계 4원칙
  (① ~/.local/bin 고정·번들 안 생성 거부 ② base 팩 타깃 고정 ③ 레거시 능동 unlink
  ④ Windows SKIP)을 결함 주입으로 못 박는다 — "PASS 가 나온다"가 아니라 **판정이 실제로
  뒤집히고 파일계가 실제로 이행되는지**를 잰다(계측 타당성 규약은 phase1 하네스와 동일).

라이브 무접촉: HOME·PATH·CYS_PACK_DIR 전부 임시 스크래치로 바꿔치기하고 쓰기는 그 안에서만
일어난다. --fix(mode=fix)는 픽스처 경로에만 작용한다 — /Applications·~/.cys 실물은 읽지도 않는다.

    python3 cysjavis-pack/bin/tests/test_preflight_c11b_seal.py
"""
import os
import shutil
import sys
import tempfile
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)

import javis_preflight as pf  # noqa: E402  — 형제 모듈 직접 구동(서브프로세스 아님)

PASS, WARN, FAIL, SKIP, FIXED = pf.PASS, pf.WARN, pf.FAIL, pf.SKIP, pf.FIXED

# setUp 이 바꿔치기하는 env 전부 — tearDown 원복 대상(레인 격리 키 4종 포함).
_SWAPPED_ENV = ("HOME", "PATH") + pf.PACK_DIR_ENV_KEYS


def wr(p, data, mode=0o644):
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8") as f:
        f.write(data)
    os.chmod(p, mode)
    return p


class C11bSealFixture(unittest.TestCase):
    """공통 픽스처: 가짜 번들(Info.plist 포함 — _app_bundle_of 가 진짜 번들로 인증) +
    base/dept 팩 + 스크래치 HOME. 각 테스트가 PATH·팩 env 를 자기 시나리오로 조립한다."""

    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="c11b-seal-")
        self.saved = {k: os.environ.get(k) for k in _SWAPPED_ENV}
        self.home = os.path.join(self.root, "home")
        os.makedirs(self.home)
        # base 팩 — 실사고와 동일하게 0644(실행비트 없음)로 두어 +x 보강 경로까지 함께 잰다.
        self.base_pack = os.path.join(self.home, ".cys", "pack")
        self.base_script = wr(os.path.join(self.base_pack, "bin", "cys-dept"),
                              "#!/bin/sh\necho base\n", 0o644)
        # dept 팩 — 타깃 flap 회귀용(base 와 다른 실체).
        self.dept_pack = os.path.join(self.home, ".cys", "pack-dept-t1")
        self.dept_script = wr(os.path.join(self.dept_pack, "bin", "cys-dept"),
                              "#!/bin/sh\necho dept\n", 0o755)
        # 가짜 앱 번들 — 페인 PATH 선두가 이 MacOS 디렉터리였던 것이 실사고의 조건.
        self.bundle = os.path.join(self.root, "Applications", "cys.app")
        self.macos = os.path.join(self.bundle, "Contents", "MacOS")
        wr(os.path.join(self.bundle, "Contents", "Info.plist"),
           "<plist><dict/></plist>")
        self.fake_cys = wr(os.path.join(self.macos, "cys"), "#!/bin/sh\nexit 0\n", 0o755)
        os.environ["HOME"] = self.home
        os.environ["PATH"] = self.macos  # 기본: 실사고 형상(번들이 PATH 선두이자 유일)
        for k in pf.PACK_DIR_ENV_KEYS:
            os.environ.pop(k, None)
        os.environ["CYS_PACK_DIR"] = self.base_pack
        self.link = os.path.join(self.home, ".local", "bin", "cys-dept")

    def tearDown(self):
        for k, v in self.saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(self.root, ignore_errors=True)

    def run_c11b(self, fix):
        p = pf.Preflight(fix=fix, skips=[])
        p.c11b_cys_dept_path()
        self.assertEqual(len(p.results), 1, p.results)
        return p.results[0]

    def plant_incident_link(self):
        """실사고 재현: 번들 Contents/MacOS 안에 base 팩 스크립트로의 심링크."""
        legacy = os.path.join(self.macos, "cys-dept")
        os.symlink(self.base_script, legacy)
        return legacy

    # ── ① 링크는 번들 밖(~/.local/bin)·타깃은 base 팩 ──
    def test_fix_migrates_incident_link_out_of_bundle(self):
        legacy = self.plant_incident_link()
        r = self.run_c11b(fix=True)
        self.assertEqual(r["status"], FIXED, r)
        # 봉인 파손 원인(번들 안 링크)이 능동 unlink 됐다 — 지우면 봉인이 복구되는 '추가 파일'.
        self.assertFalse(os.path.lexists(legacy), "번들 안 레거시 링크가 남아 있으면 봉인 파손 지속")
        # 새 링크는 ~/.local/bin(없던 디렉터리 생성 포함)이고 번들 밖이다.
        self.assertTrue(os.path.islink(self.link), r)
        self.assertFalse(pf.Preflight._in_app_bundle(self.link))
        self.assertEqual(os.path.realpath(self.link), os.path.realpath(self.base_script))
        # PATH 해소 전제(+x 보강)와 PATH 안내 문구.
        self.assertTrue(os.stat(self.base_script).st_mode & 0o111, "+x 보강이 빠지면 which 해소 불가")
        self.assertIn(".local/bin", r["detail"])

    # ── ③ 회귀 핀: realpath 일치 PASS 단락이 번들 안 링크를 덮으면 이행이 영원히 안 된다 ──
    def test_report_mode_never_passes_on_inbundle_link(self):
        legacy = self.plant_incident_link()
        # 실사고의 PASS 조건 그대로: which("cys-dept")=번들 안 링크 · realpath==팩 스크립트.
        os.chmod(self.base_script, 0o755)  # which 가 잡도록 X_OK 보장(단락 조건 완성)
        self.assertEqual(os.path.realpath(shutil.which("cys-dept")),
                         os.path.realpath(self.base_script))
        r = self.run_c11b(fix=False)
        self.assertEqual(r["status"], WARN, "번들 안 해소를 PASS 로 덮으면 이행 불가(실사고 재발)")
        self.assertIn(legacy, r["detail"])
        # report 모드는 무변경(관찰이 상태를 바꾸지 않는다).
        self.assertTrue(os.path.lexists(legacy))
        self.assertFalse(os.path.lexists(self.link))

    # ── ④ Windows 는 SKIP + 무변경 ──
    def test_windows_skips_without_touching_fs(self):
        legacy = self.plant_incident_link()
        real_name = pf.os.name
        pf.os.name = "nt"
        try:
            r = self.run_c11b(fix=True)
        finally:
            pf.os.name = real_name
        self.assertEqual(r["status"], SKIP, r)
        self.assertTrue(os.path.lexists(legacy), "SKIP 인데 파일계를 만지면 안 된다")
        self.assertFalse(os.path.lexists(self.link))

    # ── ② base 팩 타깃 고정 — dept 레인 --fix 가 타깃을 플랩시키지 않는다 ──
    def test_dept_lane_pins_target_to_base_pack(self):
        os.environ["CYS_PACK_DIR"] = self.dept_pack  # dept 레인에서 실행된 상황
        r = self.run_c11b(fix=True)
        self.assertEqual(r["status"], FIXED, r)
        self.assertEqual(os.path.realpath(self.link), os.path.realpath(self.base_script),
                         "dept 레인에서도 타깃은 base 팩이어야 한다(flap 방지)")
        self.assertNotEqual(os.path.realpath(self.link), os.path.realpath(self.dept_script))

    def test_base_pack_dir_pure_mapping(self):
        os.environ["CYS_PACK_DIR"] = self.dept_pack
        self.assertEqual(pf.base_pack_dir(), os.path.join(self.home, ".cys", "pack"))
        os.environ["CYS_PACK_DIR"] = self.base_pack
        self.assertEqual(pf.base_pack_dir(), os.path.normpath(self.base_pack))

    # ── ③ 번들 '안을 가리키는' 레거시 링크도 치유 대상 ──
    def test_legacy_link_pointing_into_bundle_is_unlinked(self):
        outer_bin = os.path.join(self.root, "bin")
        os.makedirs(outer_bin)
        legacy = os.path.join(outer_bin, "cys-dept")
        os.symlink(self.fake_cys, legacy)  # 번들 밖 위치 → 번들 안 타깃
        os.environ["PATH"] = outer_bin + os.pathsep + self.macos
        r = self.run_c11b(fix=True)
        self.assertEqual(r["status"], FIXED, r)
        self.assertFalse(os.path.lexists(legacy), "번들 안을 가리키는 링크는 unlink 대상")
        self.assertEqual(os.path.realpath(self.link), os.path.realpath(self.base_script))

    # ── 오타깃(dept 팩) 레거시 — flap 의 실제 기제 — 도 치유 대상 ──
    def test_mistargeted_legacy_link_is_unlinked(self):
        outer_bin = os.path.join(self.root, "bin")
        os.makedirs(outer_bin)
        legacy = os.path.join(outer_bin, "cys-dept")
        os.symlink(self.dept_script, legacy)  # 구 구현이 dept 레인에서 만든 형상
        os.environ["PATH"] = outer_bin + os.pathsep + self.macos
        r = self.run_c11b(fix=True)
        self.assertEqual(r["status"], FIXED, r)
        self.assertFalse(os.path.lexists(legacy), "PATH 상류의 dept 타깃 링크가 남으면 flap 지속")

    # ── 번들 안 '실파일'은 자동 삭제 금지(수동 안내만) ──
    def test_real_file_inside_bundle_never_auto_deleted(self):
        stray = wr(os.path.join(self.macos, "cys-dept"), "#!/bin/sh\necho stray\n", 0o755)
        r = self.run_c11b(fix=True)
        self.assertEqual(r["status"], WARN, r)
        self.assertTrue(os.path.isfile(stray), "실파일은 preflight 가 지우지 않는다")
        self.assertTrue(os.path.islink(self.link), "링크 설치 자체는 진행된다")
        self.assertIn("봉인 복구", r["detail"])  # 수동 rm = 봉인 복구 안내(C76 문구와 동일 축)

    # ── ~/.local/bin 의 실파일도 덮지 않는다(종전 불변식 유지) ──
    def test_local_bin_real_file_not_clobbered(self):
        wr(self.link, "#!/bin/sh\necho mine\n", 0o755)
        r = self.run_c11b(fix=True)
        self.assertEqual(r["status"], WARN, r)
        self.assertFalse(os.path.islink(self.link), "실파일을 심링크로 바꿔치기하면 안 된다")
        with open(self.link, encoding="utf-8") as f:
            self.assertIn("mine", f.read())

    # ── ① 생성 거부 불변식: 링크 위치가 번들 안이면 어떤 모드에서도 만들지 않는다 ──
    def test_creation_refused_when_home_is_inside_bundle(self):
        os.environ["HOME"] = os.path.join(self.macos, "home")  # Contents/MacOS 아래 HOME
        r = self.run_c11b(fix=True)
        self.assertEqual(r["status"], WARN, r)
        self.assertIn("생성 거부", r["detail"])
        self.assertFalse(
            os.path.lexists(os.path.join(self.macos, "home", ".local", "bin", "cys-dept")),
            "번들 안에는 어떤 경로로도 링크를 만들지 않는다")

    # ── 순수 판정 헬퍼 행렬 ──
    def test_in_app_bundle_helper_matrix(self):
        f = pf.Preflight._in_app_bundle
        self.assertTrue(f("/Applications/cys.app/Contents/MacOS/cys-dept"))
        self.assertTrue(f("/Applications/cys.app/anything"))
        self.assertTrue(f("/weird/Contents/MacOS/cys-dept"))  # .app 이름이 없어도 명시 거부
        self.assertFalse(f(os.path.join(self.home, ".local", "bin", "cys-dept")))
        self.assertFalse(f("/opt/homebrew/bin/cys-dept"))

    # ── 정상 정착 후 재실행은 PASS(멱등) ──
    def test_idempotent_pass_after_migration(self):
        self.plant_incident_link()
        self.assertEqual(self.run_c11b(fix=True)["status"], FIXED)
        r = self.run_c11b(fix=False)
        self.assertEqual(r["status"], PASS, r)
        self.assertIn(".local/bin", r["detail"])  # PATH 안내가 남는다


if __name__ == "__main__":
    unittest.main(verbosity=2)
