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
import re
import struct
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


# ★(0.14.53 WP12 · D6-b) 공개 본문·latest.json notes 가 실어야 하는 SAC 고지 — release.yml releaseBody 의 SAC 블록과
#   같은 문면이다(그 일치는 test_51 이 release.yml 을 직접 읽어 잰다).
SAC_BLOCK = (
    "<!-- cys-win-sac: unsigned -->\n"
    "**Windows 스마트 앱 컨트롤(SAC):** 이 판의 cys 설치 파일과 cys 실행파일은 코드 서명이 없습니다. "
    "스마트 앱 컨트롤이 켜진 PC에서는 실행이 막힐 수 있으며, 이 판은 스마트 앱 컨트롤 적용 PC에서 검증하지 않았습니다.\n"
    "**Windows Smart App Control:** this release's cys installer and cys executables are not code-signed. "
    "They may be blocked on PCs with Smart App Control on, and this release was not verified on a Smart App Control PC.\n"
)
BODY_OK = "cys 터미널 v%s. 설치 안내.\n\n%s" % (V, SAC_BLOCK)


def _pe(magic=0x20B, nrva=16, security=(0, 0), e_lfanew=0x80, payload=b""):
    """최소 PE 머리(섹션 0개) — 데이터 디렉터리 4(보안)를 원하는 값으로. magic 0x20B=PE32+, 0x10B=PE32."""
    opt_size = (112 if magic == 0x20B else 96) + 8 * nrva
    hdr = bytearray(e_lfanew + 4 + 20 + opt_size)
    hdr[0:2] = b"MZ"
    struct.pack_into("<I", hdr, 0x3C, e_lfanew)
    hdr[e_lfanew:e_lfanew + 4] = b"PE\0\0"
    coff = e_lfanew + 4
    struct.pack_into("<HH", hdr, coff, 0x8664, 0)
    struct.pack_into("<H", hdr, coff + 16, opt_size)
    opt = coff + 20
    struct.pack_into("<H", hdr, opt, magic)
    struct.pack_into("<I", hdr, opt + (108 if magic == 0x20B else 92), nrva)
    if nrva >= 5:
        struct.pack_into("<II", hdr, opt + (112 if magic == 0x20B else 96) + 32, *security)
    return bytes(hdr) + payload


def _signed_pe(payload=b"windows-nsis-installer-payload" * 8):
    """보안 디렉터리가 파일 끝의 가짜 WIN_CERTIFICATE 16B 를 가리키는 PE — '서명된 설치 파일'의 구조 표본."""
    head_len = len(_pe())
    return _pe(security=(head_len + len(payload), 16), payload=payload + b"C" * 16)


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
    # ★(0.14.53 WP12) setup.exe 는 **정상 PE 머리**를 가져야 한다 — 8단계(D6-b)가 보안 디렉터리를 읽으며, 깨진 머리는
    #   '미서명'으로 기본값을 주지 않고 실패한다. 종전 바이트(MZ\x90\x00 + 쓰레기)는 이제 test_48a 의 음성 표본이다.
    exe_bytes = _pe(payload=b"windows-nsis-installer-payload" * 8)

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
        "notes": "cys %s — 릴리스 안내\n\n%s" % (V, SAC_BLOCK),
        "pub_date": "2026-08-17T14:36:35Z",
        "platforms": platforms,
    }, ensure_ascii=False).encode())

    write_sums(root)
    return root


class ReleaseVerifyTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = build_fixture(self._tmp.name)
        self.body = BODY_OK

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
        assets, platforms = rv.verify(V, self.root, self.body)
        return assets, platforms

    def assert_fail(self, needle):
        """비영 종료 사유에 needle 이 들어 있어야 한다 — '조용한 통과'를 막는 본체."""
        with self.assertRaises(rv.VerifyError) as cm:
            rv.verify(V, self.root, self.body)
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
                rv.verify(V, d, BODY_OK)
            self.assertIn("비어 있다", str(cm.exception))

    def test_33_missing_dir(self):
        with self.assertRaises(rv.VerifyError) as cm:
            rv.verify(V, os.path.join(self.root, "no-such-dir"), BODY_OK)
        self.assertIn("릴리스 디렉터리가 없다", str(cm.exception))

    # ── 8. Windows SAC 고지 ↔ 서명 상태 (0.14.53 WP12 · D6-b · 부서장 판정 0·4) ──────────
    def set_setup(self, data):
        """setup.exe 와 그것을 품은 zip 을 함께 바꾼다(6단계 바이트 동일성은 그대로 성립시킨다)."""
        name = "cys_%s_x64-setup.exe" % V
        open(self.p(name), "wb").write(data)
        with zipfile.ZipFile(self.p("cys_%s_x64-setup.zip" % V), "w") as z:
            z.writestr(name, data)
        write_sums(self.root)

    def set_notes(self, notes):
        latest = self.load_latest()
        latest["notes"] = notes
        self.save_latest(latest)
        write_sums(self.root)

    def test_40_body_without_sac_marker(self):
        self.body = "cys 터미널 v%s. 설치 안내." % V
        self.assert_fail("SAC 고지 마커")

    def test_41_body_with_two_markers(self):
        self.body = BODY_OK + "\n<!-- cys-win-sac: unsigned -->\n"
        self.assert_fail("SAC 마커가 2개")

    def test_42_body_signed_unverified_refused(self):
        self.body = BODY_OK.replace("cys-win-sac: unsigned", "cys-win-sac: signed-unverified")
        self.assert_fail("허용되지 않는다")

    def test_43_body_signed_verified_refused_with_any_run_id(self):
        for marker in ("cys-win-sac: signed-verified", "cys-win-sac: signed-verified run=123456789"):
            self.body = BODY_OK.replace("cys-win-sac: unsigned", marker)
            self.assert_fail("signed-verified")

    def test_44_body_marker_without_each_key_phrase(self):
        for phrase in rv.SAC_KEY_PHRASES:
            self.body = BODY_OK.replace(phrase, "")
            msg = self.assert_fail("핵심 문구")
            self.assertIn(phrase, msg)

    def test_45_signed_setup_refused_even_with_unsigned_text(self):
        self.set_setup(_signed_pe())
        self.assert_fail("Authenticode 서명이 있다")

    def test_46_notes_without_marker(self):
        self.set_notes("cys %s — 릴리스 안내" % V)
        self.assert_fail("latest.json notes 에 SAC 고지 마커")

    def test_47_notes_claiming_signed(self):
        self.set_notes("cys %s\n\n%s" % (V, SAC_BLOCK.replace("cys-win-sac: unsigned",
                                                                "cys-win-sac: signed-verified run=1")))
        self.assert_fail("signed-verified")

    def test_48_malformed_pe_fails_closed(self):
        """깨진 PE 머리는 '미서명'이 아니라 실패다 — 잘린·위조된 설치 파일이 미서명으로 읽혀 통과하는 길을 막는다."""
        legacy = b"MZ\x90\x00" + b"windows-nsis-installer-payload" * 8      # 종전 픽스처 바이트
        good = bytearray(_pe(payload=b"p" * 64))
        no_pe_sig = bytes(good[:0x80]) + b"XX\0\0" + bytes(good[0x84:])
        bad_magic = bytearray(good)
        struct.pack_into("<H", bad_magic, 0x80 + 24, 0x0107)
        cases = {
            "a-legacy-fixture": legacy,
            "b-e_lfanew-past-end": _pe(e_lfanew=0x80)[:0x90],
            "c-no-PE-signature": no_pe_sig,
            "d-four-data-directories": _pe(nrva=4, payload=b"p" * 64),
            "e-security-dir-past-end": _pe(security=(len(_pe()), 4096), payload=b"p" * 16),
            "f-bad-optional-magic": bytes(bad_magic),
        }
        for label, data in cases.items():
            with self.subTest(case=label):
                self.set_setup(data)
                self.assert_fail("정상 PE 가 아니다")

    def test_49_pe32_unsigned_passes(self):
        self.set_setup(_pe(magic=0x10B, payload=b"windows-nsis-installer-payload" * 8))
        self.assert_pass()

    def test_50_real_unsigned_launchers_parse_as_unsigned(self):
        """실물 표본(합성 아님) — pip 이 동봉하는 distlib Windows 런처(PE32·PE32+, 미서명)를 파서가 바르게 읽는가."""
        try:
            import pip  # noqa: F401
            d = os.path.join(os.path.dirname(pip.__file__), "_vendor", "distlib")
        except ImportError:
            self.skipTest("pip 미설치 — 실물 표본 없음")
        found = sorted(f for f in os.listdir(d) if f.endswith(".exe")) if os.path.isdir(d) else []
        if not found:
            self.skipTest("distlib 런처 없음 — 실물 표본 없음")
        for f in found:
            with self.subTest(launcher=f):
                off, size = rv.pe_security_directory(os.path.join(d, f))
                self.assertEqual(size, 0, "%s 에 보안 디렉터리가 있다고 읽었다" % f)

    def test_51_body_file_inside_release_dir_is_a_stowaway(self):
        """본문 파일을 릴리스 디렉터리 안에 두면 1단계 미등재 자산으로 실패한다 — 워크플로는 밖에 둬야 한다(G4.2)."""
        open(self.p("body.txt"), "w", encoding="utf-8").write(BODY_OK)
        self.assert_fail("SUMS 에 없는 자산")


class ExitCodeContractTests(unittest.TestCase):
    """★워크플로 계약 — `set -euo pipefail` 아래에서 종료코드가 곧 fail-closed 다."""

    def run_cli(self, *args, body=BODY_OK):
        """본문 파일은 릴리스 디렉터리 **밖**의 임시 파일로 넘긴다(워크플로와 같은 배치). body=None 이면 넘기지 않는다."""
        extra = []
        if body is not None:
            fd, path = tempfile.mkstemp(suffix="-release-body.txt")
            with os.fdopen(fd, "w", encoding="utf-8") as fh:
                fh.write(body)
            self.addCleanup(os.remove, path)
            extra = ["--body-file", path]
        return subprocess.run([sys.executable, os.path.abspath(_RV_PATH)] + list(args) + extra,
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

    def test_exit_2_without_body_file(self):
        """--body-file 은 필수다 — 빠뜨리면 SAC 대조를 건너뛰는 게 아니라 사용법 오류로 죽는다(G4.2)."""
        with tempfile.TemporaryDirectory() as d:
            build_fixture(d)
            r = self.run_cli("--version", V, "--release-dir", d, body=None)
            self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
            self.assertIn("--body-file", r.stderr)

    def test_exit_1_when_body_lacks_sac_line(self):
        with tempfile.TemporaryDirectory() as d:
            build_fixture(d)
            r = self.run_cli("--version", V, "--release-dir", d, body="cys 안내만 있는 본문")
            self.assertEqual(r.returncode, 1)
            self.assertIn("windows-sac", r.stderr)

    def test_print_assets_lists_all_13(self):
        with tempfile.TemporaryDirectory() as d:
            build_fixture(d)
            r = self.run_cli("--version", V, "--release-dir", d, "--print-assets")
            self.assertEqual(r.returncode, 0, r.stderr)
            listed = [ln for ln in r.stdout.splitlines() if not ln.startswith("✅")]
            self.assertEqual(len(listed), 13)


class ReleaseWorkflowSacTextTests(unittest.TestCase):
    """생산자(release.yml releaseBody)와 관문(check_sac_text)이 같은 문면에 합의하는가 — 태그 전에 잰다.

    태그 레인에서 처음 붉어지면 버전 번호 하나를 태운다. 본문은 tauri-action 이 latest.json notes 로도 옮기므로
    (실측 M2-b) 이 문면 하나가 8단계 두 대조(본문 · notes)의 출발점이다."""

    def test_52_release_yml_release_body_passes_the_gate(self):
        wf = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".github", "workflows", "release.yml")
        src = open(wf, encoding="utf-8").read()
        self.assertEqual(src.count("releaseBody:"), 1, "releaseBody 키가 정확히 하나여야 한다")
        m = re.search(r"\n( +)releaseBody: \|\n((?:\1 .*\n|\s*\n)+)", src)
        self.assertIsNotNone(m, "releaseBody 가 블록 스칼라(|)가 아니다 — SAC 블록을 여러 줄로 실어야 한다")
        indent = m.group(1) + "  "
        body = "\n".join(l[len(indent):] if l.startswith(indent) else l.strip() for l in m.group(2).split("\n"))
        body = body.replace("${{ github.ref_name }}", "v" + V)
        rv.check_sac_text(body, "release.yml releaseBody")
        self.assertIn(SAC_BLOCK.strip(), body.strip(), "release.yml 의 SAC 블록이 이 테스트의 SAC_BLOCK 과 다르다")

    def _step_run(self, wf_rel, name):
        """`- name: <name>` 스텝의 `run: |` 본문(들여쓰기 제거) — 모르는 꼴이면 실패한다(조용히 틀린 본문을 재지 않는다)."""
        wf = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", wf_rel)
        lines = open(wf, encoding="utf-8").read().split("\n")
        heads = [i for i, l in enumerate(lines) if l.strip() == "- name: " + name]
        self.assertEqual(len(heads), 1, "%s 에 스텝 %r 가 %d개" % (wf_rel, name, len(heads)))
        ind = lambda t: len(t) - len(t.lstrip(" "))
        dash = ind(lines[heads[0]])
        j = heads[0] + 1
        while j < len(lines) and lines[j].strip() != "run: |":
            self.assertGreater(ind(lines[j]) if lines[j].strip() else dash + 1, dash, "run: | 전에 스텝이 끝났다")
            j += 1
        key = ind(lines[j])
        block = []
        j += 1
        while j < len(lines) and not (lines[j].strip() and ind(lines[j]) <= key):
            block.append(lines[j])
            j += 1
        body_ind = min(ind(l) for l in block if l.strip())
        return "\n".join(l[body_ind:] for l in block) + "\n"

    def test_53_signing_mode_step_allows_only_unsigned(self):
        """SM(0.14.53 서명 잠금): 실제 워크플로 스텝 본문을 bash 로 돌린다 — 빈 값·unsigned 만 0, 나머지(test 포함)는 1."""
        run = self._step_run(".github/workflows/release.yml", "Windows signing mode (dormant - unsigned only in this release)")
        for value, want in (("", 0), ("unsigned", 0), ("test", 1), ("artifact", 1), ("ca", 1), ("signed", 1)):
            with self.subTest(WINSIGN_MODE=value):
                env = dict(os.environ, WINSIGN_MODE=value)
                r = subprocess.run(["bash", "-c", run], capture_output=True, text=True, env=env)
                self.assertEqual(r.returncode, want, r.stdout + r.stderr)
                if want == 0:
                    self.assertIn("mode=unsigned", r.stdout)
                else:
                    self.assertIn("not allowed in this release", r.stderr)


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
