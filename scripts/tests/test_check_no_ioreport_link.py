"""check-no-ioreport-link.sh 게이트의 밀폐 검체 — 가짜 `otool`(PATH 맨 앞)로 종료 코드 0/1/2 세 갈래를 잰다(0.14.43 성찰 R2F-PK · A4 m2).

왜 존재하는가: 이 게이트는 cysd 산출물이 비공개 IOReport 를 강하게 링크하지 않았음을 `otool -L` 로 확인한다(ci-branch 맥 레인 · release 맥 레그가 부른다). 그런데 이 저장소의 교리 —
"통과만 보이는 게이트는 늘 초록인 검사와 구별되지 않는다"(release.yml 의 version-check 음성 대조) — 와 달리 **실패 갈래(링크 발견 → 1)를 재는 검체가 저장소에 없었다**(1회차의 19갈래 확인은
증거 폴더의 일회성 도구였다). awk 패턴 한 글자가 바뀌어 늘 통과하게 돼도 붉어지는 곳이 없었다. 그리고 '맥인데 otool 이 없다'는 건너뜀(통과)이 아니라 판정 불가(2)여야 같은 스크립트의 다른 갈래와 맞는다.

이 검체는 진짜 otool·진짜 바이너리·진짜 uname 에 기대지 않는다: 임시 폴더에 가짜 `uname`·`otool` 과 스크립트가 쓰는 도구(awk·wc·tr·head)의 링크만 두고 PATH 를 그 폴더 하나로 한정한다 —
그래서 '맥인데 otool 이 없다'(PATH 에서 otool 을 뺀 것)와 '맥이 아니다'(가짜 uname)를 이 기계가 맥이든 리눅스든 같은 결과로 재현한다. 실제 otool 이 있는 맥에서는 마지막에 실제 Mach-O 로 양성 대조 하나를 더 한다.

    python3 scripts/tests/test_check_no_ioreport_link.py
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
# 돌연변이 검증용: IOREPORT_GATE_UNDER_TEST=<변이본 경로> — 제품 대신 그 스크립트를 대상으로 같은 핀을 돌린다(수정 전 판·변이본에서 이 검체가 붉은지 재는 데 쓴다).
SCRIPT = os.environ.get("IOREPORT_GATE_UNDER_TEST") or os.path.normpath(os.path.join(HERE, "..", "check-no-ioreport-link.sh"))
BASH = shutil.which("bash") or "/bin/bash"

FAKE_UNAME = '#!/bin/sh\nprintf \'%s\\n\' "${FAKE_UNAME:-Darwin}"\n'
FAKE_OTOOL = '#!/bin/sh\nprintf \'%s\\n\' "$FAKE_OTOOL_OUT"\nexit "${FAKE_OTOOL_RC:-0}"\n'

# `otool -L` 출력의 모양: 첫 줄 `<경로>:`(칸 0) 뒤에 탭으로 들여쓴 의존 라이브러리 줄들. fat 바이너리는 `<경로> (architecture <arch>):` 머리가 아키텍처마다 칸 0 으로 나온다.
LIBSYS = "\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)"
CF = "\t/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 3000.0.0)"
IOR = "\t/System/Library/PrivateFrameworks/IOReport.framework/Versions/A/IOReport (compatibility version 1.0.0, current version 1.0.0)"
CLEAN = "cysd:\n" + LIBSYS + "\n" + CF
LINKED = "cysd:\n" + LIBSYS + "\n" + IOR + "\n" + CF


def _write_exec(path, text):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    os.chmod(path, 0o755)


class IoreportLinkGate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="ioreport-gate-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.bin = os.path.join(self.tmp, "bin")
        os.mkdir(self.bin)
        for tool in ("awk", "wc", "tr", "head"):   # 스크립트가 쓰는 외부 도구 — PATH 를 이 폴더 하나로 한정하므로 링크로 건네준다
            real = shutil.which(tool)
            self.assertIsNotNone(real, "검체가 쓰는 도구 %s 가 이 기계에 없다" % tool)
            os.symlink(real, os.path.join(self.bin, tool))
        self.target = os.path.join(self.tmp, "cysd")
        with open(self.target, "wb") as f:
            f.write(b"\xcf\xfa\xed\xfe")   # 가짜 otool 은 내용을 읽지 않는다 — 파일의 실재만 필요하다

    def run_gate(self, args="default", uname="Darwin", otool_out=None, otool_rc=0, with_otool=True):
        _write_exec(os.path.join(self.bin, "uname"), FAKE_UNAME)
        otool = os.path.join(self.bin, "otool")
        if os.path.lexists(otool):
            os.remove(otool)
        if with_otool:
            _write_exec(otool, FAKE_OTOOL)
        argv = [self.target] if args == "default" else list(args)
        env = {"PATH": self.bin, "FAKE_UNAME": uname, "FAKE_OTOOL_OUT": otool_out if otool_out is not None else CLEAN, "FAKE_OTOOL_RC": str(otool_rc)}
        r = subprocess.run([BASH, SCRIPT] + argv, capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
        return r.returncode, r.stdout, r.stderr

    # ── exit 0 — 통과 / 건너뜀 ──
    def test_clean_binary_passes_with_exit_0(self):
        rc, out, err = self.run_gate(otool_out=CLEAN)
        self.assertEqual(rc, 0, out + err)
        self.assertIn("통과", out)
        self.assertIn("링크된 라이브러리 2개 중 IOReport 없음", out)

    def test_path_named_ioreport_in_the_header_line_is_not_a_hit(self):
        # 첫 줄(`<경로>:`)과 fat 의 `(architecture …):` 머리는 칸 0 이라 의존 줄이 아니다 — 경로 이름에 ioreport 가 있어도 거짓 양성이 없다.
        rc, out, err = self.run_gate(otool_out="/work/ioreport-build/cysd (architecture arm64):\n" + LIBSYS + "\n" + CF)
        self.assertEqual(rc, 0, out + err)

    def test_not_macos_is_skipped_with_exit_0_even_without_otool(self):
        for uname in ("Linux", "MINGW64_NT-10.0-26100", "FreeBSD"):
            with self.subTest(uname=uname):
                rc, out, err = self.run_gate(uname=uname, with_otool=False)
                self.assertEqual(rc, 0, out + err)
                self.assertIn("건너뜀", out)
                self.assertIn("macOS 가 아니다(%s)" % uname, out)

    # ── exit 1 — 링크 발견 ──
    def test_linked_binary_fails_with_exit_1_and_prints_the_offending_line(self):
        rc, out, err = self.run_gate(otool_out=LINKED)
        self.assertEqual(rc, 1, out + err)
        self.assertIn("IOReport 링크 발견", err)
        self.assertIn(IOR.strip(), err, "otool -L 의 해당 줄이 출력돼야 한다")
        self.assertNotIn("통과", out)

    def test_match_is_case_insensitive_and_covers_a_dylib_name(self):
        rc, out, err = self.run_gate(otool_out="cysd:\n" + LIBSYS + "\n\t/usr/lib/libioreport.dylib (compatibility version 1.0.0, current version 1.0.0)")
        self.assertEqual(rc, 1, out + err)
        self.assertIn("libioreport.dylib", err)

    def test_fat_binary_with_the_link_in_one_slice_fails(self):
        fat = "cysd (architecture x86_64):\n" + LIBSYS + "\ncysd (architecture arm64):\n" + LIBSYS + "\n" + IOR
        rc, out, err = self.run_gate(otool_out=fat)
        self.assertEqual(rc, 1, out + err)

    # ── exit 2 — 판정 불가(측정 불능은 통과가 아니다) ──
    def test_macos_without_otool_is_undecidable_exit_2_not_a_skip(self):
        # ★R2F-PK(A4 m2): 종전에는 이 갈래가 건너뜀(통과 · exit 0)이었다. 맥에서 도구가 없다는 것은 '링크 없음'이 아니라 '재지 못했다'다.
        rc, out, err = self.run_gate(uname="Darwin", with_otool=False)
        self.assertEqual(rc, 2, out + err)
        self.assertIn("otool 이 없다", out + err)
        self.assertNotIn("건너뜀", out + err, "맥에서 otool 부재를 건너뜀(통과)으로 접었다")
        self.assertNotIn("통과", out)

    def test_missing_argument_is_exit_2_even_off_macos(self):
        for args in ([], [""]):
            for uname in ("Darwin", "Linux"):
                with self.subTest(args=args, uname=uname):
                    rc, out, err = self.run_gate(args=args, uname=uname)
                    self.assertEqual(rc, 2, out + err)
                    self.assertIn("인자 누락", err)

    def test_missing_file_is_exit_2(self):
        rc, out, err = self.run_gate(args=[os.path.join(self.tmp, "no-such-binary")])
        self.assertEqual(rc, 2, out + err)
        self.assertIn("파일이 없다", err)

    def test_otool_failure_is_exit_2(self):
        rc, out, err = self.run_gate(otool_out="otool: error: boom", otool_rc=1)
        self.assertEqual(rc, 2, out + err)
        self.assertIn("otool -L 이 실패했다", err)

    def test_output_without_dependency_lines_is_exit_2(self):
        # Mach-O 가 아니거나 출력 형식이 바뀌면 의존 줄이 0건이다 — '링크 없음'이 아니라 판정 불가.
        rc, out, err = self.run_gate(otool_out="cysd: is not an object file")
        self.assertEqual(rc, 2, out + err)
        self.assertIn("의존 라이브러리 줄이 없다", err)

    # ── 양성 대조(실제 otool·실제 Mach-O) — 가짜가 아닌 도구에서도 통과해야 한다. 맥에서만 ──
    @unittest.skipUnless(sys.platform == "darwin" and shutil.which("otool") and os.path.exists("/bin/ls"), "실제 otool 이 있는 맥에서만")
    def test_real_otool_on_a_real_macho_passes(self):
        r = subprocess.run([BASH, SCRIPT, "/bin/ls"], capture_output=True, text=True, encoding="utf-8", timeout=60)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("IOReport 없음", r.stdout)


if __name__ == "__main__":
    unittest.main()
