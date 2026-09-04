#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_runtime_manifest_parity.py — mac·Windows 두 레인의 runtime-manifest 동일 스키마 파리티.

★왜 이 검체가 필요한가(master 판정 2026-09-04 D3 · 검체 1):
  두 OS 는 매니페스트를 **다른 자리에서·다른 트리로** 만든다 —
    · mac : `scripts/build-macos-signed.sh` 가 **.app 트리**에서(서명 직전 창).
            소스 트리로 뜨면 낡는다(inside-out 재서명·심링크 역참조·dedup 이 뒤따르기 때문).
    · Win : 인라인 prep 말미에서 **소스 트리**로(재서명·역참조가 없는 레인).
  생성 지점이 갈리면 스키마도 조용히 갈릴 수 있고, 그러면 판독기(preflight C80 · 릴리스
  게이트 ⑧)가 한쪽 레인에서만 동작하는 상태가 무증상으로 산다. 그 드리프트를 여기서 막는다.

봉인하는 것
  ① 같은 생성기 — 두 레인이 **동일한 `javis_runtime_seal.py emit`** 을 부른다(배선 판독).
  ② 같은 스키마 — mac 형상(심링크 다수)·Windows 형상(.exe·심링크 0)에서 뜬 두 산출이
     최상위 키 집합·schema 버전·항목 값 형태가 완전히 같다.
  ③ 같은 파일명 — 두 레인의 산출 파일명과 Rust 상수 `RUNTIME_MANIFEST` 의 basename 이 일치.
     (이름이 갈리면 판독기가 파일을 못 찾아 조용히 SKIP 된다 — 가장 비싼 무증상 실패다.)
  ④ 판독기 대칭 — 한쪽 형상에서 뜬 매니페스트를 다른 형상 트리에 대면 반드시 불일치를 낸다
     (스키마가 같다는 것이 "아무 트리나 통과한다"는 뜻이 아님을 못박는다).

파일 부재는 SKIP 이 아니라 실패다 — 배선이 사라진 것이 이 검체가 잡아야 할 사고다.

    python3 cysjavis-pack/bin/tests/test_runtime_manifest_parity.py
"""
import json
import os
import shutil
import sys
import tempfile
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
REPO = os.path.dirname(os.path.dirname(BIN))
sys.path.insert(0, BIN)

import javis_runtime_seal as rs  # noqa: E402

TOOL_REL = "cysjavis-pack/bin/javis_runtime_seal.py"
MAC_WIRING = os.path.join(REPO, "scripts", "build-macos-signed.sh")
WIN_WIRING = [os.path.join(REPO, ".github", "workflows", "release.yml"),
              os.path.join(REPO, ".github", "workflows", "windows-build.yml")]
APP_BUNDLE_RS = os.path.join(REPO, "src", "app_bundle.rs")


def read(p):
    with open(p, encoding="utf-8") as f:
        return f.read()


class ManifestSchemaParityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rtman-parity-")
        # mac 형상 — 심링크가 여럿(배송본 실측 158개)이고 확장자가 없다.
        self.mac = os.path.join(self.tmp, "mac", "runtime")
        os.makedirs(os.path.join(self.mac, "node", "bin"))
        os.makedirs(os.path.join(self.mac, "node", "lib", "node_modules", "npm", "bin"))
        os.makedirs(os.path.join(self.mac, "python", "bin"))
        self._w(os.path.join(self.mac, "python", "bin", "python3"), "ELF\n", 0o755)
        self._w(os.path.join(self.mac, "node", "lib", "node_modules", "npm", "bin",
                             "npm-cli.js"), "#!/usr/bin/env node\n")
        os.symlink("../lib/node_modules/npm/bin/npm-cli.js",
                   os.path.join(self.mac, "node", "bin", "npm"))
        # Windows 형상 — .exe 확장자, 심링크 0(7z 로 푼 PortableGit·embeddable python).
        self.win = os.path.join(self.tmp, "win", "runtime")
        os.makedirs(os.path.join(self.win, "python"))
        os.makedirs(os.path.join(self.win, "git", "bin"))
        self._w(os.path.join(self.win, "python", "python3.exe"), "MZ\n", 0o755)
        self._w(os.path.join(self.win, "git", "bin", "bash.exe"), "MZ\n", 0o755)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    @staticmethod
    def _w(p, data, mode=0o644):
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8") as f:
            f.write(data)
        os.chmod(p, mode)

    # ── ② 같은 스키마 ──────────────────────────────────────────────────
    def test_both_shapes_produce_the_same_schema(self):
        a = rs.build_manifest(self.mac, app_version="0.14.30", source="mac-app")
        b = rs.build_manifest(self.win, app_version="0.14.30", source="win-src")
        self.assertEqual(sorted(a), sorted(b), "최상위 키 집합이 두 레인에서 갈렸다")
        self.assertEqual(a["schema"], b["schema"])
        self.assertEqual(sorted(a["counts"]), sorted(b["counts"]))
        # 항목 값의 형태(키 집합)가 종류별로 같아야 한다.
        def shapes(m):
            out = {}
            for v in m["entries"].values():
                out.setdefault(v["t"], set()).update(v.keys())
            return out
        sa, sb = shapes(a), shapes(b)
        for t in set(sa) & set(sb):
            self.assertEqual(sa[t], sb[t], "항목 종류 %r 의 필드가 두 레인에서 갈렸다" % t)
        # mac 형상에는 심링크가 있고 Windows 형상에는 없다 — 그래도 스키마는 같다(위 단언).
        self.assertGreater(a["counts"]["symlinks"], 0, "mac 픽스처에 심링크가 없다 — 픽스처 결함")
        self.assertEqual(0, b["counts"]["symlinks"], "Windows 픽스처에 심링크가 생겼다 — 픽스처 결함")

    def test_serialization_is_valid_json_in_both_lanes(self):
        for root, src in ((self.mac, "mac-app"), (self.win, "win-src")):
            m = rs.build_manifest(root, app_version="0.14.30", source=src)
            parsed = json.loads(rs.dumps(m))
            self.assertEqual(m["digest"], parsed["digest"])
            self.assertEqual(rs.SCHEMA, parsed["schema"])

    # ── ④ 판독기 대칭 — 스키마가 같다고 아무 트리나 통과하지 않는다 ─────
    def test_cross_tree_comparison_must_fail(self):
        mac_m = rs.build_manifest(self.mac)
        d = rs.classify(mac_m, self.win)
        self.assertTrue(d["missing"] and d["added"],
                        "mac 매니페스트를 Windows 트리에 댔는데 불일치가 안 났다 — 판독기 무력")

    # ── ① 같은 생성기 배선 ─────────────────────────────────────────────
    def test_both_lanes_invoke_the_same_generator(self):
        self.assertTrue(os.path.isfile(MAC_WIRING), "mac 배선 파일이 사라졌다: %s" % MAC_WIRING)
        mac = read(MAC_WIRING)
        self.assertIn(TOOL_REL, mac, "mac 빌드가 봉인 생성기를 부르지 않는다")
        self.assertIn("emit", mac)
        for p in WIN_WIRING:
            self.assertTrue(os.path.isfile(p), "Windows 배선 파일이 사라졌다: %s" % p)
            src = read(p)
            self.assertIn(TOOL_REL, src, "%s 가 봉인 생성기를 부르지 않는다" % os.path.basename(p))
            self.assertIn("emit", src)

    def test_mac_generates_from_the_app_tree_not_the_source_tree(self):
        """★생성 지점 회귀 핀. mac 에서 소스 트리(src-tauri/runtime)로 뜨면 해시가 낡는다 —
        재서명·심링크 역참조·dedup 이 그 뒤에 오기 때문이다(2026-09-04 실측으로 확정)."""
        mac = read(MAC_WIRING)
        i = mac.find(TOOL_REL)
        self.assertGreater(i, 0)
        window = mac[i:i + 400]
        self.assertIn("Contents/Resources/runtime", window,
                      "mac emit 이 .app 트리를 대상으로 하지 않는다 — 낡은 해시가 배송된다")
        self.assertNotIn("--root src-tauri/runtime", window,
                         "mac emit 이 소스 트리를 대상으로 한다 — 서명·dedup 뒤 어긋난다")

    # ── ③ 같은 파일명 (Rust 상수까지) ───────────────────────────────────
    def test_manifest_basename_is_identical_everywhere(self):
        name = rs.MANIFEST_BASENAME
        self.assertEqual("runtime-manifest.json", name)
        self.assertIn(name, read(MAC_WIRING), "mac 산출 파일명이 다르다")
        for p in WIN_WIRING:
            self.assertIn(name, read(p), "%s 산출 파일명이 다르다" % os.path.basename(p))
        self.assertTrue(os.path.isfile(APP_BUNDLE_RS), "app_bundle.rs 가 사라졌다")
        rust = read(APP_BUNDLE_RS)
        self.assertIn('RUNTIME_MANIFEST: &str = "Resources/%s"' % name, rust,
                      "Rust 상수의 파일명이 생성기와 갈렸다 — 판독기가 파일을 못 찾는다")

    # ── ⑤ 설치본 검체가 **방금 설치한 트리**를 보는가 (R1 codex #9) ───────
    def test_windows_install_probe_is_pinned_to_the_installer_contract_path(self):
        """LOCALAPPDATA 재귀 검색의 첫 cysd.exe 를 쓰면 잔존 설치를 검증할 수 있다 —
        초록이 거짓이 되는 무증상 오검증이다. 매니페스트 실기 검증과 그 앞의 설치 스텝은
        installer 계약 경로(%LOCALAPPDATA%\\cys · installMode=currentUser + productName=cys)와
        버전 마커 동일성(cys-installed-version.txt == 이번 빌드)으로 고정돼야 한다."""
        wb = os.path.join(REPO, ".github", "workflows", "windows-build.yml")
        self.assertTrue(os.path.isfile(wb), "windows-build.yml 이 사라졌다")
        src = read(wb)
        i = src.find("- name: 자기완결 검증")
        j = src.find("- name: PTY 스모크")
        self.assertGreater(i, 0, "설치 스텝이 사라졌다 — 검체의 앵커가 없다")
        self.assertGreater(j, i, "PTY 스모크가 설치 스텝 뒤에 없다 — 스텝 순서 회귀")
        # 주석은 빼고 **실행되는 줄**만 센다 — 설명문에 이름이 적혀 있다고 배선은 아니다.
        window = "\n".join(l for l in src[i:j].split("\n") if not l.lstrip().startswith("#"))
        self.assertNotIn('Get-ChildItem "$env:LOCALAPPDATA" -Recurse', window,
                         "설치본 검체가 재귀 검색으로 되돌아갔다 — 잔존 설치를 검증할 수 있다")
        self.assertIn("Join-Path $env:LOCALAPPDATA 'cys'", window,
                      "installer 계약 경로(%LOCALAPPDATA%\\cys)로 고정돼 있지 않다")
        self.assertEqual(2, window.count("cys-installed-version.txt"),
                         "두 스텝 모두 버전 마커로 동일성을 확인해야 한다(방금 설치한 트리인가)")
        self.assertIn("CYS_INSTALL_DIR", window,
                      "매니페스트 검증 스텝이 앞 스텝의 확정값을 받지 않는다 — 두 스텝이 다른 트리를 볼 수 있다")
        # 마커 대조는 '있는지'가 아니라 '이번 빌드 버전과 같은지'여야 한다.
        self.assertEqual(2, window.count("$markerVer -ne $expVer"),
                         "마커 존재만 보고 값을 대조하지 않으면 잔존 설치를 걸러내지 못한다")

    def test_windows_lane_ships_the_manifest_as_a_bundle_resource(self):
        conf = os.path.join(REPO, "src-tauri", "tauri.windows.conf.json")
        self.assertTrue(os.path.isfile(conf))
        with open(conf, encoding="utf-8") as f:
            res = json.load(f)["bundle"]["resources"]
        self.assertIn("resources/%s" % rs.MANIFEST_BASENAME, res,
                      "Windows 번들이 매니페스트를 싣지 않는다 — 설치본에 파일이 없다")


if __name__ == "__main__":
    unittest.main(verbosity=2)
