"""release-verify.py 의 밀폐 unittest — 네트워크·토큰·실자산 불요, 합성 픽스처만 사용.

★왜 이 테스트가 필요한가
  `scripts/release-verify.py` 는 **비가역 공개 발행 직전의 마지막 fail-closed 관문**이다
  (`release-publish.yml` 이 이걸로 죽으면 `gh release edit --draft=false` 가 실행되지 않는다).
  그런데 이 관문의 검사들은 **손으로 돌린 결함주입**으로만 실증돼 있었다 — 회귀 자산이 0이었다.
  실제로 2026-08-18 적대적 리뷰에서 fail-open 2건(플랫폼 키 집합 무단언 · 키→자산 결속 무단언)이
  발견됐다. 그 2건을 포함해 **"통과해선 안 될 입력"을 여기 전부 못박는다.**

★설계
  픽스처는 실물 13종의 **구조만** 축소 재현한다(수 KB). 실물 대조는 별도다 —
  `python3 scripts/release-verify.py --version 0.14.19 --release-dir ~/cys-release-backup/v0.14.19-assets`.
  각 테스트는 정상 픽스처를 만든 뒤 **한 곳만** 망가뜨리고, 그 사유가 나오는지 본다.
  경로는 전부 tempfile — 개인 경로·실 홈 디렉터리 금지(시크릿 스캔 계약, test_verify_win_crt.py 관례).

사용: python3 scripts/tests/test_release_verify.py
"""

import base64
import gzip
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
import zipfile

# ★하이픈 파일명(`release-verify.py`)은 `import` 문으로 못 부른다. 형제 테스트들이 쓰는
#   `sys.path.insert` + `import <모듈>` 관례를 쓸 수 없어 importlib 로 파일을 직접 적재한다.
#   (파일명을 바꾸는 쪽이 아니라 테스트가 맞추는 이유: 하이픈 명명은 scripts/ 의 다수파이고
#    워크플로가 그 경로를 문자열로 부른다 — 이름을 바꾸면 발행 레인이 끊긴다.)
_RV_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "release-verify.py")
_spec = importlib.util.spec_from_file_location("release_verify", _RV_PATH)
rv = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rv)

V = "0.14.19"
BASE = "https://github.com/idoforgod/cys-terminal/releases/download/v%s/" % V


def _sig_text(tag):
    """tauri updater 서명 형식 재현 — minisign 텍스트를 base64 로 감싼 한 줄."""
    body = "untrusted comment: signature from tauri secret key\n%s\n" % tag
    return base64.b64encode(body.encode()).decode()


def _dmg_bytes(payload):
    """UDIF dmg 재현 — 머리는 zlib(78 01), **끝 512B 가 'koly' 트레일러**."""
    return b"\x78\x01" + payload + b"koly" + b"\x00" * 508


def write_sums(root):
    """디렉터리 실물에서 SHA256SUMS.txt 를 다시 만든다(자기 자신 제외)."""
    rows = []
    for name in sorted(os.listdir(root)):
        if name == rv.SUMS_NAME:
            continue
        path = os.path.join(root, name)
        if not os.path.isfile(path) or os.path.islink(path):
            continue
        rows.append("%s  %s" % (rv.sha256_file(path), name))
    with open(os.path.join(root, rv.SUMS_NAME), "w", encoding="utf-8") as fh:
        fh.write("\n".join(rows) + "\n")


def build_fixture(root):
    """정상 통과하는 13종 묶음을 합성한다(실물 v0.14.19 의 구조를 축소 재현)."""
    exe_bytes = b"MZ\x90\x00" + b"windows-nsis-installer-payload" * 8

    def w(name, data):
        with open(os.path.join(root, name), "wb") as fh:
            fh.write(data)

    w("cys_%s_aarch64.dmg" % V, _dmg_bytes(b"macos-arm-disk-image" * 16))
    w("cys_%s_x64.dmg" % V, _dmg_bytes(b"macos-intel-disk-image" * 16))
    w("cys_%s_x64-setup.exe" % V, exe_bytes)
    w("cys_aarch64.app.tar.gz", gzip.compress(b"macos-arm-app-bundle" * 32))
    w("cys_x64.app.tar.gz", gzip.compress(b"macos-intel-app-bundle" * 32))
    w("pack.tar.gz", gzip.compress(b"cysjavis-pack-payload" * 32))
    w("pack-manifest.json", json.dumps({"files": [], "expires_at": "2027-01-01T00:00:00Z"}).encode())
    w("pack-manifest.json.minisig",
      b"untrusted comment: minisign signature\nRWQfixture\n")

    # zip 은 setup.exe **한 개**만, 바이트 동일하게 품어야 한다.
    with zipfile.ZipFile(os.path.join(root, "cys_%s_x64-setup.zip" % V), "w") as z:
        z.writestr("cys_%s_x64-setup.exe" % V, exe_bytes)

    sigs = {
        "cys_aarch64.app.tar.gz.sig": _sig_text("arm"),
        "cys_x64.app.tar.gz.sig": _sig_text("intel"),
        "cys_%s_x64-setup.exe.sig" % V: _sig_text("win"),
    }
    for name, text in sigs.items():
        w(name, (text + "\n").encode())

    platforms = {}
    for key, asset_tpl in rv.UPDATER_PLATFORMS.items():
        asset = asset_tpl.format(v=V)
        platforms[key] = {"signature": sigs[asset + ".sig"], "url": BASE + asset}
    w("latest.json", json.dumps({
        "version": V,
        "notes": "cys %s — 릴리스 안내" % V,
        "pub_date": "2026-08-17T14:36:35Z",
        "platforms": platforms,
    }, ensure_ascii=False).encode())

    write_sums(root)
    return root


class ReleaseVerifyTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = build_fixture(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    # ── 헬퍼 ──────────────────────────────────────────────────────────────
    def p(self, name):
        return os.path.join(self.root, name)

    def load_latest(self):
        return json.load(open(self.p("latest.json"), encoding="utf-8"))

    def save_latest(self, obj):
        with open(self.p("latest.json"), "w", encoding="utf-8") as fh:
            json.dump(obj, fh, ensure_ascii=False)

    def assert_pass(self):
        assets, platforms = rv.verify(V, self.root)
        return assets, platforms

    def assert_fail(self, needle):
        """비영 종료 사유에 needle 이 들어 있어야 한다 — '조용한 통과'를 막는 본체."""
        with self.assertRaises(rv.VerifyError) as cm:
            rv.verify(V, self.root)
        self.assertIn(needle, str(cm.exception),
                      "예상 사유 %r 가 아니라 %r 로 죽었다" % (needle, str(cm.exception)))
        return str(cm.exception)

    # ── 0. 기준선 ─────────────────────────────────────────────────────────
    def test_00_baseline_passes(self):
        assets, platforms = self.assert_pass()
        self.assertEqual(len(assets), 13)
        self.assertEqual(platforms, sorted(rv.UPDATER_PLATFORMS))

    # ── 1. 완전성(디렉터리 ↔ SUMS 양방향) ─────────────────────────────────
    def test_01_asset_missing_from_dir(self):
        os.remove(self.p("cys_%s_x64.dmg" % V))
        self.assert_fail("자산 누락")

    def test_02_unlisted_stowaway(self):
        open(self.p("stowaway.bin"), "wb").write(b"not-in-sums")
        self.assert_fail("SUMS 에 없는 자산이 섞여 있다")

    def test_03_symlink_rejected(self):
        os.symlink(self.p("pack.tar.gz"), self.p("linked.tar.gz"))
        self.assert_fail("일반 파일이 아닌 항목")

    def test_04_asset_and_sums_row_both_removed(self):
        """대조만으로는 절대 못 잡는 케이스 — REQUIRED_ASSETS 하한선이 유일한 포획 지점."""
        os.remove(self.p("cys_%s_aarch64.dmg" % V))
        write_sums(self.root)
        self.assert_fail("배포 필수 자산 누락")

    # ── 2. 체크섬·SUMS 형식 ───────────────────────────────────────────────
    def test_05_checksum_mismatch_on_byte_flip(self):
        raw = bytearray(open(self.p("pack.tar.gz"), "rb").read())
        raw[-1] ^= 0xFF                      # gzip CRC 꼬리를 흔든다
        open(self.p("pack.tar.gz"), "wb").write(bytes(raw))
        self.assert_fail("pack.tar.gz")      # 지문 또는 체크섬 — 어느 쪽이든 죽어야 한다

    def test_06_sums_digest_tampered(self):
        text = open(self.p(rv.SUMS_NAME), encoding="utf-8").read()
        first = text.splitlines()[0]
        flipped = ("b" if first[0] != "b" else "c") + first[1:]
        open(self.p(rv.SUMS_NAME), "w", encoding="utf-8").write(text.replace(first, flipped))
        self.assert_fail("체크섬 불일치")

    def test_07_sums_malformed_row(self):
        with open(self.p(rv.SUMS_NAME), "a", encoding="utf-8") as fh:
            fh.write("deadbeef  ../escape.bin\n")
        self.assert_fail("형식 오류")

    def test_08_sums_lists_itself(self):
        with open(self.p(rv.SUMS_NAME), "a", encoding="utf-8") as fh:
            fh.write("%s  %s\n" % ("0" * 64, rv.SUMS_NAME))
        self.assert_fail("자기 제외 규약 위반")

    # ── 3. 버전 토큰 ──────────────────────────────────────────────────────
    def test_09_version_token_mismatch(self):
        """구버전 자산이 섞여 든 묶음. ★REQUIRED_ASSETS 8종이 아닌 자산으로 찔러야
        3단계까지 도달한다 — 8종 중 하나면 2단계(배포 필수 자산 누락)가 먼저 잡는다."""
        stale = "cys_0.14.18_x64-setup.exe.sig"
        os.rename(self.p("cys_%s_x64-setup.exe.sig" % V), self.p(stale))
        write_sums(self.root)
        self.assert_fail("파일명 버전 토큰 불일치")

    def test_09b_required_asset_wins_over_token_check(self):
        """같은 사고라도 필수 8종이면 2단계가 먼저 잡는다 — 사유 순서를 못박는다."""
        os.rename(self.p("cys_%s_x64.dmg" % V), self.p("cys_0.14.18_x64.dmg"))
        write_sums(self.root)
        self.assert_fail("배포 필수 자산 누락")

    # ── 4. 컨테이너 지문(ASSET_SHAPES) ────────────────────────────────────
    #    ★SUMS 를 쓰레기에 맞춰 재생성해도 통과해선 안 된다.
    def test_10_zero_byte_asset(self):
        open(self.p("cys_%s_aarch64.dmg" % V), "wb").close()
        write_sums(self.root)
        self.assert_fail("0바이트")

    def test_11_dmg_without_koly_trailer(self):
        open(self.p("cys_%s_x64.dmg" % V), "wb").write(b"\x78\x01" + b"J" * 4096)
        write_sums(self.root)
        self.assert_fail("koly")

    def test_12_targz_is_junk(self):
        open(self.p("pack.tar.gz"), "wb").write(b"not a tarball")
        write_sums(self.root)
        self.assert_fail("gzip 매직이 아니다")

    def test_13_targz_truncated_midstream(self):
        """머리 매직은 멀쩡한데 중간에서 잘린 tar.gz — 끝까지 풀어야만 잡힌다."""
        raw = open(self.p("cys_aarch64.app.tar.gz"), "rb").read()
        open(self.p("cys_aarch64.app.tar.gz"), "wb").write(raw[: len(raw) // 2])
        write_sums(self.root)
        self.assert_fail("gzip 스트림이 끝까지 풀리지 않는다")

    def test_14_exe_not_pe(self):
        open(self.p("cys_%s_x64-setup.exe" % V), "wb").write(b"#!/bin/sh\necho pwned\n")
        write_sums(self.root)
        self.assert_fail("컨테이너 지문 불일치")

    def test_15_sig_not_base64(self):
        open(self.p("cys_x64.app.tar.gz.sig"), "wb").write(b"!!! not base64 !!!")
        latest = self.load_latest()
        for key in ("darwin-x86_64", "darwin-x86_64-app"):
            latest["platforms"][key]["signature"] = "!!! not base64 !!!"
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("base64 가 아니다")

    def test_16_json_asset_not_parseable(self):
        open(self.p("pack-manifest.json"), "wb").write(b"{ broken json,,,")
        write_sums(self.root)
        self.assert_fail("JSON 자산이 파싱되지 않는다")

    def test_17_unknown_extension_rejected(self):
        """배포 구성이 몰래 늘어나는 걸 막는다 — 사람이 상수를 고쳐야 통과한다."""
        open(self.p("cys_%s_x64.msi" % V), "wb").write(b"\xd0\xcf\x11\xe0msi")
        write_sums(self.root)
        self.assert_fail("확장자 규칙에 없는 자산")

    # ── 5. Windows zip ↔ exe ──────────────────────────────────────────────
    def test_18_zip_holds_different_bytes(self):
        with zipfile.ZipFile(self.p("cys_%s_x64-setup.zip" % V), "w") as z:
            z.writestr("cys_%s_x64-setup.exe" % V, b"MZ\x90\x00different-installer")
        write_sums(self.root)
        self.assert_fail("바이트 동일하지 않다")

    def test_19_zip_holds_extra_member(self):
        exe = "cys_%s_x64-setup.exe" % V
        with zipfile.ZipFile(self.p("cys_%s_x64-setup.zip" % V), "w") as z:
            z.writestr(exe, open(self.p(exe), "rb").read())
            z.writestr("README.txt", b"stowaway")
        write_sums(self.root)
        self.assert_fail("하나만 있어야 한다")

    # ── 6. 업데이터(latest.json) — ①②③④ ──────────────────────────────────
    def test_20_top_level_field_set_notes_deleted(self):
        latest = self.load_latest()
        del latest["notes"]
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("최상위 필드 집합 오류")

    def test_21_top_level_field_set_junk_injected(self):
        latest = self.load_latest()
        latest["injected_junk"] = "x"
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("최상위 필드 집합 오류")

    def test_22_macos_updater_lane_removed_entirely(self):
        """★fail-open 반증 케이스 1 — 초판이 exit 0 으로 통과시켰던 입력.

        macOS 업데이터 4종을 디렉터리·SUMS·latest.json 에서 통째로 지운다.
        = macOS 전 사용자의 앱 내 Update 가 죽은 묶음. 반드시 죽어야 한다.
        """
        for name in ("cys_aarch64.app.tar.gz", "cys_aarch64.app.tar.gz.sig",
                     "cys_x64.app.tar.gz", "cys_x64.app.tar.gz.sig"):
            os.remove(self.p(name))
        latest = self.load_latest()
        latest["platforms"] = {k: v for k, v in latest["platforms"].items()
                               if k.startswith("windows-")}
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("platforms 키 집합 오류")

    def test_23_updater_rows_all_repointed_to_windows_exe(self):
        """★fail-open 반증 케이스 2 — 초판이 exit 0 으로 통과시켰던 입력.

        6행 전부를 Windows 설치본으로 덮는다. = macOS 업데이터가 NSIS exe 를 받는 묶음.
        """
        latest = self.load_latest()
        exe = "cys_%s_x64-setup.exe" % V
        sig = open(self.p(exe + ".sig"), encoding="utf-8").read().strip()
        for key in latest["platforms"]:
            latest["platforms"][key] = {"signature": sig, "url": BASE + exe}
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("url 결속 위반")

    def test_24_single_row_repointed(self):
        latest = self.load_latest()
        latest["platforms"]["darwin-aarch64"]["url"] = BASE + "cys_x64.app.tar.gz"
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("url 결속 위반")

    def test_25_updater_signature_mismatch(self):
        latest = self.load_latest()
        latest["platforms"]["darwin-aarch64"]["signature"] = _sig_text("forged")
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("업데이터 서명 불일치")

    def test_26_updater_sig_file_removed_everywhere(self):
        """`.sig` 는 REQUIRED_ASSETS 밖 — latest.json 교차대조만이 유일한 포획 지점."""
        os.remove(self.p("cys_aarch64.app.tar.gz.sig"))
        write_sums(self.root)
        self.assert_fail("서명 파일이 없다")

    def test_27_latest_json_version_mismatch(self):
        latest = self.load_latest()
        latest["version"] = "0.14.18"
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("version 불일치")

    def test_28_pub_date_without_timezone(self):
        latest = self.load_latest()
        latest["pub_date"] = "2026-08-17T14:36:35"
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("타임존이 없다")

    def test_29_updater_row_extra_field(self):
        latest = self.load_latest()
        latest["platforms"]["windows-x86_64"]["with_elevated_task"] = False
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("업데이터 행 필드 집합 오류")

    def test_30_notes_emptied(self):
        latest = self.load_latest()
        latest["notes"] = "   "
        self.save_latest(latest)
        write_sums(self.root)
        self.assert_fail("notes 가 비어 있거나")

    # ── 7. 껍데기 입력 ────────────────────────────────────────────────────
    def test_31_sums_absent(self):
        os.remove(self.p(rv.SUMS_NAME))
        self.assert_fail("%s 가 없다" % rv.SUMS_NAME)

    def test_32_empty_dir(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(rv.VerifyError) as cm:
                rv.verify(V, d)
            self.assertIn("비어 있다", str(cm.exception))

    def test_33_missing_dir(self):
        with self.assertRaises(rv.VerifyError) as cm:
            rv.verify(V, os.path.join(self.root, "no-such-dir"))
        self.assertIn("릴리스 디렉터리가 없다", str(cm.exception))


class ExitCodeContractTests(unittest.TestCase):
    """★워크플로 계약 — `set -euo pipefail` 아래에서 종료코드가 곧 fail-closed 다."""

    def run_cli(self, *args):
        return subprocess.run([sys.executable, os.path.abspath(_RV_PATH)] + list(args),
                              capture_output=True, text=True)

    def test_exit_0_on_pass(self):
        with tempfile.TemporaryDirectory() as d:
            build_fixture(d)
            r = self.run_cli("--version", V, "--release-dir", d)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("릴리스 검증 통과", r.stdout)

    def test_exit_1_on_failure_with_reason(self):
        with tempfile.TemporaryDirectory() as d:
            build_fixture(d)
            os.remove(os.path.join(d, "latest.json"))
            r = self.run_cli("--version", V, "--release-dir", d)
            self.assertEqual(r.returncode, 1)
            self.assertIn("::error::", r.stderr)          # GitHub Actions 주석 형식
            self.assertIn("자산 누락", r.stderr)

    def test_exit_1_on_bad_version_arg(self):
        with tempfile.TemporaryDirectory() as d:
            r = self.run_cli("--version", "v0.14.19", "--release-dir", d)
            self.assertEqual(r.returncode, 1)
            self.assertIn("X.Y.Z", r.stderr)

    def test_exit_2_on_missing_arg(self):
        r = self.run_cli("--version", V)
        self.assertEqual(r.returncode, 2)                 # argparse 사용법 오류

    def test_print_assets_lists_all_13(self):
        with tempfile.TemporaryDirectory() as d:
            build_fixture(d)
            r = self.run_cli("--version", V, "--release-dir", d, "--print-assets")
            self.assertEqual(r.returncode, 0, r.stderr)
            listed = [ln for ln in r.stdout.splitlines() if not ln.startswith("✅")]
            self.assertEqual(len(listed), 13)


class ConstantsSanityTests(unittest.TestCase):
    """상수 자체의 자기정합 — 손으로 고치다 어긋나는 걸 잡는다."""

    def test_required_and_updater_cover_thirteen(self):
        covered = {a.format(v=V) for a in rv.REQUIRED_ASSETS}
        for asset_tpl in rv.UPDATER_PLATFORMS.values():
            asset = asset_tpl.format(v=V)
            covered.add(asset)
            covered.add(asset + ".sig")
        self.assertEqual(len(covered), 13,
                         "하한선 두 개가 덮는 자산이 13종이 아니다: %s" % sorted(covered))

    def test_every_covered_asset_has_a_shape_rule(self):
        covered = {a.format(v=V) for a in rv.REQUIRED_ASSETS}
        for asset_tpl in rv.UPDATER_PLATFORMS.values():
            asset = asset_tpl.format(v=V)
            covered.add(asset)
            covered.add(asset + ".sig")
        for name in sorted(covered):
            self.assertTrue(any(name.endswith(sfx) for sfx, _ in rv.ASSET_SHAPES),
                            "지문 규칙이 없는 자산: %s" % name)


if __name__ == "__main__":
    unittest.main(verbosity=2)
