"""verify_win_crt.py 게이트의 밀폐 unittest — 데몬·cys 미호출, 합성 PE 픽스처만 사용.

픽스처 경로는 전부 tempfile(mkdtemp) — 개인 경로·실 홈 디렉토리 금지(시크릿 스캔 계약).
실세계 대조는 v0.13.22 공개 설치본 370개 exe 전수(파싱 실패 0·정확 3건 검출)로 별도 실증됨
— 여기서는 파서의 구조 처리(PE32/PE32+/비PE)와 게이트 판정 로직(엄격/규칙/마커)을 핀한다.
"""

import os
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import verify_win_crt as gate  # noqa: E402


def make_pe(imports, pe32plus=True):
    """임포트 테이블을 가진 최소 유효 PE 바이트를 합성한다.

    단일 섹션(.idata, VA=0x1000)에 [descriptor 배열 | 종결 descriptor | DLL 이름들]을 배치.
    gate.imports_of 가 요구하는 필드(e_lfanew·COFF·optional magic·DataDirectory[1]·섹션 헤더)만
    정확히 채운 최소 구성이다.
    """
    va = 0x1000
    ndesc = len(imports)
    names_off = (ndesc + 1) * 20  # 섹션 내 상대: descriptor 들 뒤에 이름
    sec = bytearray()
    name_rvas = []
    names_blob = bytearray()
    for dll in imports:
        name_rvas.append(va + names_off + len(names_blob))
        names_blob += dll.encode("ascii") + b"\0"
    for rva in name_rvas:
        # ILT=1(비0 아무 값), TimeDateStamp=0, Forwarder=0, NameRVA, IAT=1
        sec += struct.pack("<IIIII", 1, 0, 0, rva, 1)
    sec += struct.pack("<IIIII", 0, 0, 0, 0, 0)  # 종결 descriptor
    sec += names_blob

    dos = bytearray(0x40)
    dos[:2] = b"MZ"
    struct.pack_into("<I", dos, 0x3C, 0x40)  # e_lfanew

    opt_size = 240 if pe32plus else 224
    coff = struct.pack("<HHIIIHH", 0x8664 if pe32plus else 0x14C, 1, 0, 0, 0, opt_size, 0)

    opt = bytearray(opt_size)
    struct.pack_into("<H", opt, 0, 0x20B if pe32plus else 0x10B)
    dd_off = 112 if pe32plus else 96  # optional 헤더 내 DataDirectory 시작
    struct.pack_into("<I", opt, dd_off - 4, 16)              # NumberOfRvaAndSizes
    struct.pack_into("<II", opt, dd_off + 8, va, len(sec))   # DataDirectory[1] = Import

    headers_len = 0x40 + 4 + 20 + opt_size + 40
    raw_off = (headers_len + 0x1FF) & ~0x1FF  # 512 정렬
    sec_hdr = bytearray(40)
    sec_hdr[:6] = b".idata"
    struct.pack_into("<I", sec_hdr, 8, len(sec))    # VirtualSize
    struct.pack_into("<I", sec_hdr, 12, va)         # VirtualAddress
    struct.pack_into("<I", sec_hdr, 16, len(sec))   # SizeOfRawData
    struct.pack_into("<I", sec_hdr, 20, raw_off)    # PointerToRawData

    blob = bytes(dos) + b"PE\0\0" + coff + bytes(opt) + bytes(sec_hdr)
    blob += b"\0" * (raw_off - len(blob)) + bytes(sec)
    return blob


class ParserTests(unittest.TestCase):
    def test_pe32plus_import_extraction(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "a.exe")
            with open(p, "wb") as f:
                f.write(make_pe(["KERNEL32.dll", "VCRUNTIME140.dll"]))
            self.assertEqual(gate.imports_of(p), ["KERNEL32.dll", "VCRUNTIME140.dll"])

    def test_pe32_import_extraction(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "b.exe")
            with open(p, "wb") as f:
                f.write(make_pe(["vcruntime140.dll"], pe32plus=False))
            self.assertEqual(gate.imports_of(p), ["vcruntime140.dll"])

    def test_non_pe_returns_none(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "c.exe")
            with open(p, "wb") as f:
                f.write(b"#!/bin/sh\necho not a pe\n")
            self.assertIsNone(gate.imports_of(p))

    def test_unmapped_import_dir_rva_is_parse_failure(self):
        # ★R1 codex high 핀: Import Directory 를 선언했는데 그 RVA 가 어느 섹션에도 매핑되지
        # 않는 조작·손상 PE — 종전엔 [](빈 임포트=통과)로 fail-open 이었다. None(파싱 실패
        # → exit 3)이어야 한다.
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "d.exe")
            blob = bytearray(make_pe(["VCRUNTIME140.dll"]))
            # DataDirectory[1] RVA 를 섹션 밖(0x9000)으로 조작
            e_lfanew = struct.unpack_from("<I", blob, 0x3C)[0]
            opt = e_lfanew + 4 + 20
            dd_off = opt + 112  # PE32+
            struct.pack_into("<I", blob, dd_off + 8, 0x9000)
            with open(p, "wb") as f:
                f.write(bytes(blob))
            self.assertIsNone(gate.imports_of(p))

    def test_lying_opt_size_is_parse_failure(self):
        # ★R3 codex high 핀: COFF 의 opt_size 를 DataDirectory[1] 이 담기지 않는 크기로 속인
        # 절단·조작 PE — 종전엔 어긋난 오프셋에서 쓰레기 섹션 헤더를 읽어 오판정 여지가 있었다.
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "f.exe")
            blob = bytearray(make_pe(["VCRUNTIME140.dll"]))
            e_lfanew = struct.unpack_from("<I", blob, 0x3C)[0]
            coff = e_lfanew + 4
            struct.pack_into("<H", blob, coff + 16, 100)  # PE32+ dd_off+16=128 > 100
            with open(p, "wb") as f:
                f.write(bytes(blob))
            self.assertIsNone(gate.imports_of(p))

    def test_unterminated_descriptor_array_is_parse_failure(self):
        # ★R3 codex high 핀: zero terminator 없이 descriptor 배열이 EOF 에 닿는 절단 PE —
        # 종전엔 수집분을 '정상 임포트'로 반환해 잘려나간 vcruntime descriptor 가 증발했다.
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "g.exe")
            blob = bytearray(make_pe(["KERNEL32.dll"]))
            # 섹션 원시영역을 '종결자 없는 descriptor 1개(20B)'로 재구성: name_rva 는 섹션 내
            # 자기 자신(+16, IAT=1 의 상위 0 바이트)을 가리켜 매핑은 성공하되 배열은 미종결.
            headers_len = 0x40 + 4 + 20 + 240 + 40
            raw_off = (headers_len + 0x1FF) & ~0x1FF
            va = 0x1000
            desc = struct.pack("<IIIII", 1, 0, 0, va + 16, 1)
            blob = blob[:raw_off] + desc  # 파일이 descriptor 끝에서 정확히 절단
            # 섹션 헤더의 크기 필드를 20 으로 축소(경계 정합)
            sec_hdr = 0x40 + 4 + 20 + 240
            struct.pack_into("<I", blob, sec_hdr + 8, 20)   # VirtualSize
            struct.pack_into("<I", blob, sec_hdr + 16, 20)  # SizeOfRawData
            with open(p, "wb") as f:
                f.write(bytes(blob))
            self.assertIsNone(gate.imports_of(p))

    def test_unmapped_descriptor_name_rva_is_parse_failure(self):
        # ★R1 codex high 핀: nonzero descriptor 의 name_rva 미매핑 — 종전엔 그 항목만 조용히
        # 건너뛰어 DLL 이름(잠재적 vcruntime)이 검사에서 증발했다. None 이어야 한다.
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "e.exe")
            blob = bytearray(make_pe(["VCRUNTIME140.dll", "KERNEL32.dll"]))
            # 섹션 원시 데이터에서 첫 descriptor 의 NameRVA(오프셋 +12)를 섹션 밖으로 조작
            headers_len = 0x40 + 4 + 20 + 240 + 40
            raw_off = (headers_len + 0x1FF) & ~0x1FF
            struct.pack_into("<I", blob, raw_off + 12, 0x9000)
            with open(p, "wb") as f:
                f.write(bytes(blob))
            self.assertIsNone(gate.imports_of(p))

    def test_no_import_table(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "d.exe")
            with open(p, "wb") as f:
                f.write(make_pe([]))
            self.assertEqual(gate.imports_of(p), [])


class GateLogicTests(unittest.TestCase):
    def test_strict_binary_rejected_even_with_dll_beside(self):
        # 정책 바이너리는 DLL 드롭으로도 통과 불가(무음 정책 회귀 봉쇄).
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "cysd.exe"), "wb") as f:
                f.write(make_pe(["VCRUNTIME140.dll"]))
            with open(os.path.join(d, "vcruntime140.dll"), "wb") as f:
                f.write(b"\0")
            violations, unparsed, allowed, total = gate.check_imports(d)
            self.assertEqual(total, 1)
            self.assertEqual(len(violations), 1)
            self.assertEqual(allowed, [])
            self.assertEqual(unparsed, [])

    def test_thirdparty_allowed_only_with_applocal_dll(self):
        with tempfile.TemporaryDirectory() as d:
            ok_dir = os.path.join(d, "withdll")
            bad_dir = os.path.join(d, "nodll")
            os.makedirs(ok_dir)
            os.makedirs(bad_dir)
            for dd in (ok_dir, bad_dir):
                with open(os.path.join(dd, "tool.exe"), "wb") as f:
                    f.write(make_pe(["vcruntime140.dll"]))
            with open(os.path.join(ok_dir, "vcruntime140.dll"), "wb") as f:
                f.write(b"\0")
            violations, _, allowed, total = gate.check_imports(d)
            self.assertEqual(total, 2)
            self.assertEqual(len(allowed), 1)
            self.assertIn("withdll", allowed[0])
            self.assertEqual(len(violations), 1)
            self.assertIn("nodll", violations[0][0])

    def test_vcruntime140_1_variant_is_caught(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "cys.exe"), "wb") as f:
                f.write(make_pe(["VCRUNTIME140_1.dll"]))
            violations, _, _, _ = gate.check_imports(d)
            self.assertEqual(len(violations), 1)

    def test_clean_static_binary_passes(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "cysd.exe"), "wb") as f:
                f.write(make_pe(["KERNEL32.dll", "ntdll.dll"]))
            violations, unparsed, allowed, total = gate.check_imports(d)
            self.assertEqual((len(violations), len(unparsed), len(allowed), total), (0, 0, 0, 1))

    def test_unparsable_exe_is_failclosed(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "broken.exe"), "wb") as f:
                f.write(b"MZ" + b"\0" * 8)  # 잘린 PE
            _, unparsed, _, _ = gate.check_imports(d)
            self.assertEqual(len(unparsed), 1)


class MarkerTests(unittest.TestCase):
    def test_all_markers_present(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "cysd.exe"), "wb") as f:
                f.write(b"pad" + b"".join(gate.FIX_MARKERS) + b"pad")
            missing, cysd = gate.check_markers(d)
            self.assertIsNotNone(cysd)
            self.assertEqual(missing, [])

    def test_missing_marker_detected(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "cysd.exe"), "wb") as f:
                f.write(b"pad-without-generation-marker")
            missing, _ = gate.check_markers(d)
            self.assertEqual(missing, ["cys-fix-w2-gen-0.14.4"])

    def test_cysd_absent_reports_all_missing(self):
        with tempfile.TemporaryDirectory() as d:
            missing, cysd = gate.check_markers(d)
            self.assertIsNone(cysd)
            self.assertEqual(len(missing), len(gate.FIX_MARKERS))


if __name__ == "__main__":
    unittest.main()
