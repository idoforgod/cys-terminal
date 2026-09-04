#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_c80_runtime_seal.py — C80 동봉 런타임 봉인 매니페스트 회귀 하네스 (standalone).

★왜 이 파일이 존재하는가(부트 v2 §2-10 G5 · 티켓 C1):
  C76(코드서명 봉인)은 **macOS 전용**이다. Windows 설치본에는 sealed-resource 봉인이 없어
  설치 후 `runtime/**` 오염을 잡을 수단이 0이었다. C80 이 그 공백을 메운다. 실사고 형태는
  가상이 아니다 — 2026-09-04 실측으로 오너의 `/Applications/cys.app` 에 `npm i -g @openai/codex`
  가 **번들 안으로** 설치돼 봉인이 깨져 있었다(codesign: file added 11줄).

이 하네스가 결함 주입으로 못 박는 것
  ① 무결 트리 → PASS
  ② 오염(추가 파일) → WARN(부팅 비차단) · 원인 파일을 detail 에 지목
  ③ ★등급 계약: 파손이 FAIL 이 아니다. master 판정 2026-09-04 D1(레인 분리) — 발행 차단은
     릴리스 게이트가 FAIL 로 하고 기계 preflight 는 부팅을 막지 않는다. FAIL 로 바꾸면
     이미 파손된 기계가 READY→NOT READY 로 뒤집혀 부팅 자체가 막힌다(C76:5275 와 같은 판단).
  ④ ★구버전 무회귀: 매니페스트가 없는 v0.14.29 이하 설치본은 **SKIP**(정상)이지 경고가 아니다.
     이게 WARN 이 되면 기존 사용자 전원이 거짓 경보를 받는다.
  ④' ★그 반대 갈래(R1 codex #6): 0.14.30 **이상** 설치본에서 매니페스트만 없으면 **WARN** 이다.
     "부재=무조건 SKIP" 은 신규 설치에서 매니페스트만 소실된 경우를 정상으로 덮어, Windows 의
     유일한 변조 감지축을 조용히 없앤다(mac 은 코드서명이 남지만 Windows 는 0). 그래서 이
     하네스는 **양방향**으로 못 박는다 — 구버전 SKIP ∧ 신버전 WARN ∧ 버전 판독 불가 SKIP(판정 불가).
     판독처는 설치 형태가 정한다: macOS 는 Info.plist 의 CFBundleShortVersionString, Windows·
     비번들은 설치 루트의 `cys-installed-version.txt`(NSIS 훅이 전 게이트 통과 후에만 쓰는 마커).
  ⑤ node_modules 오염이면 npm prefix 원인 문구를 함께 낸다(실사고의 직접 원인 안내).
  ⑥ 자동 수정 0 — --fix 에서도 파일을 지우지 않는다(C76 과 같은 규율).
  ⑦ 판정 불가(대조 실패)는 통과가 아니라 SKIP.
  ⑧ 한 체크는 결과 행을 정확히 1개만 낸다(집의 규약).

라이브 무접촉: HOME·PATH·CYS_PACK_DIR 를 임시 스크래치로 바꿔치기하고, 검사 대상 번들도
픽스처다 — /Applications·~/.cys 실물은 읽지도 쓰지도 않는다.

    python3 cysjavis-pack/bin/tests/test_preflight_c80_runtime_seal.py
"""
import os
import shutil
import sys
import tempfile
import unittest
from unittest import mock

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)

import javis_preflight as pf          # noqa: E402 — 형제 모듈 직접 구동(서브프로세스 아님)
import javis_runtime_seal as rs       # noqa: E402

PASS, WARN, FAIL, SKIP = pf.PASS, pf.WARN, pf.FAIL, pf.SKIP

_SWAPPED_ENV = ("HOME", "PATH") + pf.PACK_DIR_ENV_KEYS


def wr(p, data, mode=0o644):
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8") as f:
        f.write(data)
    os.chmod(p, mode)
    return p


def plist_xml(version):
    """실제 배송 형식과 같은 XML plist. version=None 이면 CFBundleShortVersionString 키를 뺀다
    (실측 2026-09-04: /Applications/cys.app/Contents/Info.plist 는 XML plist · 값 "0.14.29")."""
    body = ""
    if version is not None:
        body = ("  <key>CFBundleShortVersionString</key>\n  <string>%s</string>\n" % version)
    return ('<?xml version="1.0" encoding="UTF-8"?>\n'
            '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" '
            '"http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
            '<plist version="1.0">\n<dict>\n'
            '  <key>CFBundleIdentifier</key>\n  <string>com.cysjavis.cys</string>\n'
            '%s</dict>\n</plist>\n' % body)


class C80RuntimeSealTests(unittest.TestCase):
    """픽스처: 가짜 .app 번들(Info.plist 로 진짜 번들 인증) + runtime 트리 + 매니페스트.
    PATH 선두에 픽스처 `cys` 를 놓아 _find_app_bundle 이 이 번들을 잡게 한다."""

    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="c80-rtseal-")
        self.saved = {k: os.environ.get(k) for k in _SWAPPED_ENV}
        self.home = os.path.join(self.root, "home")
        os.makedirs(self.home)

        self.bundle = os.path.join(self.root, "Applications", "cys.app")
        self.res = os.path.join(self.bundle, "Contents", "Resources")
        self.macos = os.path.join(self.bundle, "Contents", "MacOS")
        # ★번들 버전은 판정 재료다(매니페스트 부재 갈래) — 기본은 봉인 탑재 버전으로 둔다.
        self.plist = os.path.join(self.bundle, "Contents", "Info.plist")
        wr(self.plist, plist_xml(pf.Preflight.RUNTIME_SEAL_SINCE))
        # 번들 안 cys — PATH 선두에 두면 _app_bundle_of 가 이 번들을 해소한다(실사고와 같은 형상).
        wr(os.path.join(self.macos, "cys"), "#!/bin/sh\nexit 0\n", 0o755)
        # 축소 런타임 트리(심링크 포함 — 배송본과 동형).
        self.rt = os.path.join(self.res, "runtime")
        wr(os.path.join(self.rt, "python", "bin", "python3"), "ELF-ish\n", 0o755)
        wr(os.path.join(self.rt, "node", "lib", "node_modules", "npm", "bin", "npm-cli.js"),
           "#!/usr/bin/env node\n")
        os.makedirs(os.path.join(self.rt, "node", "bin"), exist_ok=True)
        os.symlink("../lib/node_modules/npm/bin/npm-cli.js",
                   os.path.join(self.rt, "node", "bin", "npm"))
        self.man = os.path.join(self.res, "runtime-manifest.json")

        os.environ["HOME"] = self.home
        os.environ["PATH"] = self.macos + os.pathsep + os.environ.get("PATH", "")
        for k in pf.PACK_DIR_ENV_KEYS:
            os.environ[k] = os.path.join(self.home, ".cys", "pack")

    def tearDown(self):
        for k, v in self.saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(self.root, ignore_errors=True)

    # ── 하네스 ──────────────────────────────────────────────────────────
    def _emit(self):
        with open(self.man, "w", encoding="utf-8") as f:
            f.write(rs.dumps(rs.build_manifest(self.rt, app_version="0.14.30", source="test")))

    def _row(self, fix=False):
        p = pf.Preflight(fix=fix, skips=[])
        p.c80_runtime_seal()
        self.assertEqual(1, len(p.results), "한 체크는 결과 행을 정확히 1개만 낸다: %r" % p.results)
        r = p.results[0]
        self.assertEqual("C80.runtime-seal", r["id"])
        return r["status"], r["detail"]

    # ── ① 무결 ─────────────────────────────────────────────────────────
    def test_clean_tree_passes(self):
        self._emit()
        st, detail = self._row()
        self.assertEqual(PASS, st, detail)
        self.assertIn("무결", detail)

    # ── ②③ 오염 → WARN (★FAIL 이 아니다) ────────────────────────────────
    def test_pollution_warns_and_never_fails(self):
        self._emit()
        wr(os.path.join(self.rt, "node", "lib", "node_modules", "@openai", "codex",
                        "package.json"), "{}\n")
        st, detail = self._row()
        self.assertEqual(WARN, st,
                         "봉인 파손은 WARN 이어야 한다 — FAIL 이면 파손된 기계의 부팅이 막힌다")
        self.assertNotEqual(FAIL, st)
        self.assertIn("추가 1건", detail)
        self.assertIn("@openai", detail, "원인 파일을 지목해야 안내다")
        self.assertIn("재설치", detail, "처방(통째 교체로 봉인 자연 복원)이 있어야 한다")

    # ── ⑤ npm prefix 원인 안내 ─────────────────────────────────────────
    def test_node_modules_pollution_names_the_npm_prefix_cause(self):
        self._emit()
        wr(os.path.join(self.rt, "node", "lib", "node_modules", "@scope", "x", "p.json"), "{}\n")
        st, detail = self._row()
        self.assertEqual(WARN, st)
        self.assertIn("npm i -g", detail,
                      "node_modules 오염이면 실사고의 직접 원인(전역 설치 prefix)을 말해야 한다")

    # ── ④ ★구버전 무회귀: 매니페스트 부재 + 구버전 = SKIP ────────────────
    def test_missing_manifest_on_old_version_is_skip_not_warn(self):
        """실사고 형상 그대로: 2026-09-04 오너 설치본 `/Applications/cys.app` = 0.14.29 · 매니페스트
        없음. 여기에 경고를 내면 기존 사용자 전원이 거짓 경보를 받는다."""
        wr(self.plist, plist_xml("0.14.29"))
        self.assertFalse(os.path.exists(self.man))
        st, detail = self._row()
        self.assertEqual(SKIP, st,
                         "구버전 설치본(매니페스트 없음)에 경고를 내면 기존 사용자 전원이 거짓 경보를 받는다")
        self.assertIn("0.14.29", detail, "판독한 버전을 말해야 근거가 추적된다")
        self.assertIn("0.14.30", detail, "왜 정상인지(봉인 도입 버전) 사유를 말해야 한다")

    # ── ④' ★반대 갈래(R1 codex #6): 매니페스트 부재 + 봉인 탑재 버전 = WARN ──
    def test_missing_manifest_on_sealed_version_warns(self):
        """0.14.30 설치본에 매니페스트만 없다 = 봉인이 실렸어야 하는데 사라진 상태.
        SKIP 으로 덮으면 Windows 의 유일한 변조 감지축이 조용히 없어진다."""
        self.assertFalse(os.path.exists(self.man))
        st, detail = self._row()
        self.assertEqual(WARN, st,
                         "봉인 탑재 버전에서 매니페스트 부재를 SKIP 하면 감지축 소실이 은폐된다")
        self.assertIn("0.14.30", detail)
        self.assertIn("판독처", detail, "어디서 버전을 읽었는지 말해야 추적 가능하다")
        self.assertIn("재설치", detail, "처방이 있어야 안내다")

    def test_missing_manifest_on_newer_version_warns(self):
        """경계 위쪽: 0.14.30 '이상'이지 '같음'이 아니다(0.15.0 도 WARN)."""
        wr(self.plist, plist_xml("0.15.0"))
        st, detail = self._row()
        self.assertEqual(WARN, st, detail)
        self.assertIn("0.15.0", detail)

    def test_missing_manifest_with_unreadable_version_is_skip_not_pass(self):
        """버전을 못 읽으면 조용히 정상이라 말하지 않는다 — 판정 불가(측정 불능은 통과가 아니다).
        WARN 으로 밀면 판독 실패한 개발 빌드 전부가 거짓 경보가 된다."""
        wr(self.plist, plist_xml(None))          # CFBundleShortVersionString 키 자체가 없다
        st, detail = self._row()
        self.assertEqual(SKIP, st, detail)
        self.assertIn("판정 불가", detail)
        self.assertIn("CFBundleShortVersionString", detail, "무엇을 못 읽었는지 지목해야 한다")

    def test_missing_manifest_with_corrupt_plist_is_skip_not_pass(self):
        wr(self.plist, "{not a plist")
        st, detail = self._row()
        self.assertEqual(SKIP, st, detail)
        self.assertIn("판정 불가", detail)

    # ── ④'' Windows·비번들 형상: 버전 판독처는 `cys-installed-version.txt` ──
    #
    # NSIS 훅이 전 게이트 통과 후에만 쓰는 마커(src-tauri/nsis-hooks.nsh:784 `cys_post_ok`).
    # `_find_app_bundle` 은 macOS 전용 폴백(/Applications/cys.app)을 갖고 있어 이 개발 기계에서
    # 라이브 번들을 집는다 — Windows 에는 그 경로가 아예 없으므로 None 으로 고정해 형상을 맞춘다
    # (라이브 무접촉 유지: 이 갈래에서 /Applications 를 읽지 않는다).
    def _win_shape(self, marker_text):
        d = os.path.join(self.root, "wininstall")
        wr(os.path.join(d, "cys"), "#!/bin/sh\nexit 0\n", 0o755)
        wr(os.path.join(d, "runtime", "python", "bin", "python3"), "ELF-ish\n", 0o755)
        if marker_text is not None:
            wr(os.path.join(d, "cys-installed-version.txt"), marker_text)
        os.environ["PATH"] = d + os.pathsep + os.environ["PATH"]
        return d

    def test_windows_marker_old_version_is_skip(self):
        self._win_shape("0.14.29")
        with mock.patch.object(pf.Preflight, "_find_app_bundle", lambda _self: None):
            st, detail = self._row()
        self.assertEqual(SKIP, st, detail)
        self.assertIn("cys-installed-version.txt", detail, "판독처를 말해야 한다")
        self.assertIn("0.14.29", detail)

    def test_windows_marker_sealed_version_warns(self):
        self._win_shape("0.14.30")           # NSIS 는 개행 없이 ${VERSION} 만 쓴다
        with mock.patch.object(pf.Preflight, "_find_app_bundle", lambda _self: None):
            st, detail = self._row()
        self.assertEqual(WARN, st, detail)
        self.assertIn("cys-installed-version.txt", detail)
        self.assertIn("0.14.30", detail)

    def test_windows_marker_absent_is_skip_not_pass(self):
        """마커가 없으면(포터블 복사·설치 실패 잔해) 버전을 모른다 → 판정 불가."""
        self._win_shape(None)
        with mock.patch.object(pf.Preflight, "_find_app_bundle", lambda _self: None):
            st, detail = self._row()
        self.assertEqual(SKIP, st, detail)
        self.assertIn("판정 불가", detail)

    def test_windows_marker_garbage_is_skip_not_pass(self):
        """판독은 됐지만 버전으로 해석 불가 → 통과도 경고도 아닌 판정 불가(fail-safe)."""
        self._win_shape("not-a-version")
        with mock.patch.object(pf.Preflight, "_find_app_bundle", lambda _self: None):
            st, detail = self._row()
        self.assertEqual(SKIP, st, detail)
        self.assertIn("판정 불가", detail)

    # ── 런타임 트리 자체가 없으면(개발 빌드) SKIP ───────────────────────
    def test_no_runtime_tree_is_skip(self):
        shutil.rmtree(self.rt)
        st, detail = self._row()
        self.assertEqual(SKIP, st, detail)

    # ── ⑦ 판정 불가는 통과가 아니라 SKIP ────────────────────────────────
    def test_unreadable_manifest_is_skip_not_pass(self):
        wr(self.man, "{not json")
        st, detail = self._row()
        self.assertEqual(SKIP, st, detail)
        self.assertIn("판정 불가", detail)

    def test_unknown_schema_is_skip_not_pass(self):
        wr(self.man, '{"schema": 99, "entries": {}}')
        st, _ = self._row()
        self.assertEqual(SKIP, st)

    # ── ⑥ 자동 수정 0 — --fix 에서도 파일을 지우지 않는다 ────────────────
    def test_fix_mode_never_deletes_anything(self):
        self._emit()
        polluted = wr(os.path.join(self.rt, "node", "lib", "node_modules",
                                   "@openai", "codex", "package.json"), "{}\n")
        before = sorted(rs.walk_entries(self.rt))
        st, _ = self._row(fix=True)
        after = sorted(rs.walk_entries(self.rt))
        self.assertEqual(WARN, st, "--fix 에서도 등급은 그대로 WARN")
        self.assertTrue(os.path.exists(polluted), "--fix 가 오염 파일을 지웠다 — 자동 삭제 0 위반")
        self.assertEqual(before, after, "--fix 가 런타임 트리를 변경했다 — 읽기 전용 위반")

    # ── 삭제·변경도 잡는가(3분류 전 갈래) ───────────────────────────────
    def test_missing_and_changed_are_reported(self):
        self._emit()
        os.remove(os.path.join(self.rt, "python", "bin", "python3"))
        wr(os.path.join(self.rt, "node", "lib", "node_modules", "npm", "bin", "npm-cli.js"),
           "tampered\n")
        st, detail = self._row()
        self.assertEqual(WARN, st)
        self.assertIn("누락 1건", detail)
        self.assertIn("변경 1건", detail)

    # ── 등재 확인 — 목록에 없으면 실행되지 않는다(껍데기 방지) ───────────
    def test_check_is_registered_before_the_fixed_last_slots(self):
        # run() 은 전 체크를 실제로 돌리므로 호출하지 않는다 — 소스에서 등재 순서만 읽는다.
        with open(os.path.join(BIN, "javis_preflight.py"), encoding="utf-8") as f:
            src = f.read()
        i80 = src.find("self.c80_runtime_seal,")
        i62 = src.find("self.c62_pack_heal_ledger,")
        self.assertGreater(i80, 0, "C80 이 run() 등재 목록에 없다 — 안 도는 체크는 게이트가 아니다")
        self.assertGreater(i62, i80, "C80 은 고정 마지막 슬롯(C62·C68) **앞**이어야 한다(§5-4 배선 규율)")


if __name__ == "__main__":
    unittest.main(verbosity=2)
