"""Hermetic controls for scripts/check_macho_arch.py — the bundle architecture gate (0.15.0 · track T11 ·
RULINGS BQ1(b) · owner order addendum 11: the Intel installer must never carry an Apple-silicon-only python).

Every case builds SYNTHETIC Mach-O / fat headers in a temp dir (no real binary needed, no lipo, no network):
thin x86_64, thin arm64, fat with both, fat without the target, fat64, a header cut short, a Java class file that
also starts with 0xCAFEBABE, plain text; minimum macOS from LC_BUILD_VERSION and from LC_VERSION_MIN_MACOSX; a
fake cys.app whose runtime/python/bin/python3 is a link to an arm64-only python3.12 (the owner's reported case);
the same tree as a .tar.gz (the updater archive); --require, --quiet and the "cannot judge" exits. A red case
differs from its green control only in the planted architecture.

    python3 scripts/tests/test_check_macho_arch.py
"""
import io
import os
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
# Mutation testing: MACHO_ARCH_GATE_UNDER_TEST=<mutant path> runs the same cases against that script instead.
SCRIPT = os.environ.get("MACHO_ARCH_GATE_UNDER_TEST") or os.path.normpath(os.path.join(HERE, "..", "check_macho_arch.py"))

X86_64, ARM64, ARM64_32 = 0x01000007, 0x0100000C, 0x0200000C
LC_BUILD_VERSION, LC_VERSION_MIN_MACOSX = 0x32, 0x24


def ver(s):
    p = [int(x) for x in s.split(".")] + [0, 0]
    return (p[0] << 16) | (p[1] << 8) | p[2]


def thin(cputype, minos=None, legacy=None):
    """A little-endian 64-bit Mach-O executable header with an optional minimum-OS load command."""
    cmds = b""
    n = 0
    if minos:
        cmds += struct.pack("<IIIIII", LC_BUILD_VERSION, 24, 1, ver(minos), ver("15.0"), 0)
        n += 1
    if legacy:
        cmds += struct.pack("<IIII", LC_VERSION_MIN_MACOSX, 16, ver(legacy), ver("10.15"))
        n += 1
    hdr = struct.pack("<IIIIIIII", 0xFEEDFACF, cputype, 3, 2, n, len(cmds), 0, 0)
    return hdr + cmds + b"\0" * 64


def fat(slices, fat64=False):
    """A big-endian universal header with the given (cputype, thin bytes) slices at 4096-aligned offsets."""
    n = len(slices)
    entry = 32 if fat64 else 20
    off = 4096
    table, body = b"", b""
    for cputype, data in slices:
        if fat64:
            table += struct.pack(">IIQQII", cputype, 3, off, len(data), 12, 0)
        else:
            table += struct.pack(">IIIII", cputype, 3, off, len(data), 12)
        body += data + b"\0" * (4096 - len(data) % 4096)
        off += len(data) + (4096 - len(data) % 4096)
    head = struct.pack(">II", 0xCAFEBABF if fat64 else 0xCAFEBABE, n) + table
    return head + b"\0" * (4096 - len(head)) + body


JAVA_CLASS = b"\xca\xfe\xba\xbe\x00\x00\x00\x34" + b"\x00" * 40        # Java 8 class: version where nfat would be
CUT_SHORT = b"\xcf\xfa\xed\xfe\x07"                                       # thin magic, header cut after 5 bytes


class Gate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="macho-arch-")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def put(self, rel, data, mode=0o755):
        p = os.path.join(self.tmp, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "wb") as f:
            f.write(data)
        os.chmod(p, mode)
        return p

    def link(self, rel, target):
        p = os.path.join(self.tmp, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        os.symlink(target, p)

    def app(self, python_cpu=X86_64, plist_min="10.13"):
        """A fake cys.app laid out like the real one (sidecars, runtime entry points, a universal dylib)."""
        a = "cys.app/Contents/"
        self.put(a + "Info.plist", (
            '<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" '
            '"http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0"><dict>'
            "<key>LSMinimumSystemVersion</key><string>%s</string></dict></plist>\n" % plist_min).encode(), 0o644)
        for exe in ("cys-app", "cys", "cysd"):
            self.put(a + "MacOS/" + exe, thin(X86_64, minos="10.13"))
        rt = a + "Resources/runtime/"
        self.put(rt + "python/bin/python3.12", thin(python_cpu, minos="10.15"))
        self.link(rt + "python/bin/python3", "python3.12")
        self.put(rt + "python/lib/python3.12/os.py", b"import abc\n", 0o644)
        self.put(rt + "python/lib/python3.12/__pycache__/os.cpython-312.pyc", b"\xcb\x0d\x0d\x0a" + b"\0" * 12, 0o644)
        self.put(rt + "node/bin/node", thin(X86_64, minos="11.0"))
        self.put(rt + "git/bin/git", thin(X86_64, legacy="10.13"))
        self.put(rt + "git/libexec/git-core/libSkiaSharp.dylib", fat([(X86_64, thin(X86_64)), (ARM64, thin(ARM64))]))
        self.put(rt + "uv/uv", thin(X86_64, minos="10.12"))
        return os.path.join(self.tmp, "cys.app")

    def gate(self, *args):
        r = subprocess.run([sys.executable, SCRIPT] + list(args), capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout + r.stderr

    # -- the owner's case and its green control --
    def test_intel_app_with_x86_64_python_passes_and_prints_entry_points(self):
        app = self.app()
        rc, out = self.gate("--target", "x86_64-apple-darwin", app)
        self.assertEqual(rc, 0, out)
        self.assertIn("RESULT: PASS", out)
        self.assertIn("x86_64 7", out)                 # count per architecture: 7 thin x86_64
        self.assertIn("x86_64+arm64 1", out)           # and one universal dylib
        self.assertRegex(out, r"runtime/python/bin/python3 -> python3\.12\s+x86_64\s+min 10\.15")
        self.assertRegex(out, r"runtime/node/bin/node\s+x86_64\s+min 11\.0")
        self.assertRegex(out, r"runtime/git/bin/git\s+x86_64\s+min 10\.13")      # from LC_VERSION_MIN_MACOSX
        self.assertRegex(out, r"Contents/MacOS/cysd\s+x86_64\s+min 10\.13")

    def test_intel_app_with_arm64_only_python_fails_naming_the_file(self):
        app = self.app(python_cpu=ARM64)
        rc, out = self.gate("--target", "x86_64-apple-darwin", app)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL cys.app/Contents/Resources/runtime/python/bin/python3.12: arm64 (no x86_64 slice)", out)
        self.assertIn("RESULT: FAIL — 1 Mach-O file(s) without x86_64", out)

    def test_the_same_app_fails_for_the_other_target_naming_every_thin_x86_64_file(self):
        app = self.app()
        rc, out = self.gate("--target", "aarch64-apple-darwin", app)
        self.assertEqual(rc, 1, out)
        self.assertEqual(out.count("FAIL cys.app/"), 7, out)            # the universal dylib is not named
        self.assertNotIn("FAIL cys.app/Contents/Resources/runtime/git/libexec/git-core/libSkiaSharp.dylib", out)

    def test_min_os_newer_than_ls_minimum_is_reported_not_failed(self):
        app = self.app(plist_min="10.13")
        rc, out = self.gate("--target", "x86_64", app)
        self.assertEqual(rc, 0, out)
        self.assertIn("LSMinimumSystemVersion = 10.13", out)
        self.assertIn("WARN (informational", out)
        self.assertIn("python3 10.15", out)
        self.assertIn("node 11.0", out)
        app2 = os.path.join(self.tmp, "cys.app/Contents/Info.plist")
        with open(app2, "rb") as f:
            data = f.read()
        with open(app2, "wb") as f:
            f.write(data.replace(b">10.13<", b">11.0<"))
        rc, out = self.gate("--target", "x86_64", app)
        self.assertEqual(rc, 0, out)
        self.assertNotIn("WARN (informational", out)

    # -- header shapes --
    def test_fat_containing_the_target_passes_fat_without_it_fails(self):
        self.put("both", fat([(X86_64, thin(X86_64)), (ARM64, thin(ARM64))]))
        rc, out = self.gate("--target", "x86_64-apple-darwin", os.path.join(self.tmp, "both"))
        self.assertEqual(rc, 0, out)
        rc, out = self.gate("--target", "aarch64-apple-darwin", os.path.join(self.tmp, "both"))
        self.assertEqual(rc, 0, out)
        self.put("armonly", fat([(ARM64, thin(ARM64)), (ARM64_32, thin(ARM64_32))]))
        rc, out = self.gate("--target", "x86_64-apple-darwin", os.path.join(self.tmp, "armonly"))
        self.assertEqual(rc, 1, out)
        self.assertIn("arm64+arm64_32 (no x86_64 slice)", out)

    def test_fat64_header_is_read(self):
        self.put("f64", fat([(X86_64, thin(X86_64, minos="10.15")), (ARM64, thin(ARM64))], fat64=True))
        rc, out = self.gate("--target", "x86_64-apple-darwin", os.path.join(self.tmp, "f64"))
        self.assertEqual(rc, 0, out)
        rc, out = self.gate("--target", "aarch64-apple-darwin", os.path.join(self.tmp, "f64"))
        self.assertEqual(rc, 0, out)

    def test_thin_of_the_other_architecture_fails(self):
        self.put("x", thin(X86_64))
        self.put("a", thin(ARM64))
        rc, out = self.gate("--target", "x86_64-apple-darwin", self.tmp)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL a: arm64 (no x86_64 slice)", out)
        self.assertNotIn("FAIL x:", out)

    def test_a_cut_short_header_fails(self):
        self.put("ok", thin(X86_64))
        self.put("cut", CUT_SHORT)
        rc, out = self.gate("--target", "x86_64-apple-darwin", self.tmp)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL cut: Mach-O header cut short", out)

    def test_a_fat_table_cut_short_fails(self):
        self.put("ok", thin(X86_64))
        self.put("fatcut", struct.pack(">II", 0xCAFEBABE, 2) + b"\0" * 10)
        rc, out = self.gate("--target", "x86_64-apple-darwin", self.tmp)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL fatcut: Mach-O header cut short", out)

    def test_java_class_and_text_are_not_mach_o(self):
        self.put("ok", thin(X86_64))
        self.put("Main.class", JAVA_CLASS, 0o644)
        self.put("readme.txt", b"hello\n", 0o644)
        rc, out = self.gate("--target", "x86_64-apple-darwin", self.tmp)
        self.assertEqual(rc, 0, out)
        self.assertIn("Mach-O files: 1 ", out + " ")
        self.assertNotIn("Main.class", out)

    # -- the updater archive --
    def test_tar_gz_archive_is_read_member_by_member(self):
        self.app(python_cpu=ARM64)
        arc = os.path.join(self.tmp, "cys.app.tar.gz")
        with tarfile.open(arc, "w:gz") as t:
            t.add(os.path.join(self.tmp, "cys.app"), arcname="cys.app")
        rc, out = self.gate("--target", "x86_64-apple-darwin", arc)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL cys.app/Contents/Resources/runtime/python/bin/python3.12: arm64", out)
        shutil.rmtree(os.path.join(self.tmp, "cys.app"))
        os.remove(arc)
        self.app()
        with tarfile.open(arc, "w:gz") as t:
            t.add(os.path.join(self.tmp, "cys.app"), arcname="cys.app")
        rc, out = self.gate("--target", "x86_64-apple-darwin", arc)
        self.assertEqual(rc, 0, out)
        self.assertRegex(out, r"runtime/python/bin/python3 -> python3\.12\s+x86_64\s+min 10\.15")

    # -- the runtime tree as prep-mac-runtime.sh checks it --
    def test_require_entry_points_on_a_runtime_tree(self):
        self.app()
        rt = os.path.join(self.tmp, "cys.app/Contents/Resources/runtime")
        req = ["--require", "python/bin/python3", "--require", "node/bin/node",
               "--require", "git/bin/git", "--require", "uv/uv"]
        rc, out = self.gate("--target", "x86_64-apple-darwin", "--quiet", *req, rt)
        self.assertEqual(rc, 0, out)
        self.assertEqual(out.strip().splitlines()[-1][:13], "RESULT: PASS ")
        self.assertNotIn("entry points", out)          # --quiet
        os.remove(os.path.join(rt, "uv/uv"))
        rc, out = self.gate("--target", "x86_64-apple-darwin", "--quiet", *req, rt)
        self.assertEqual(rc, 2, out)
        self.assertIn("required entry point uv/uv is missing", out)

    # -- cannot judge is never a pass --
    def test_cannot_judge_cases_exit_2(self):
        empty = os.path.join(self.tmp, "empty")
        os.makedirs(empty)
        self.assertEqual(self.gate("--target", "x86_64-apple-darwin", empty)[0], 2)
        self.assertEqual(self.gate("--target", "x86_64-apple-darwin", os.path.join(self.tmp, "nope"))[0], 2)
        self.put("x", thin(X86_64))
        self.assertEqual(self.gate("--target", "powerpc", os.path.join(self.tmp, "x"))[0], 2)
        bad = self.put("broken.tar.gz", b"\x1f\x8b\x08\x00garbage", 0o644)
        self.assertEqual(self.gate("--target", "x86_64-apple-darwin", bad)[0], 2)

    def test_a_failure_wins_over_cannot_judge_across_inputs(self):
        self.put("bad/a", thin(ARM64))
        rc, out = self.gate("--target", "x86_64-apple-darwin", os.path.join(self.tmp, "bad"), os.path.join(self.tmp, "nope"))
        self.assertEqual(rc, 1, out)

    @unittest.skipUnless(sys.platform == "darwin", "a real universal system binary exists on macOS only")
    def test_a_real_universal_system_binary_passes_both_targets(self):
        for target in ("x86_64-apple-darwin", "aarch64-apple-darwin"):
            rc, out = self.gate("--target", target, "/bin/echo")
            self.assertEqual(rc, 0, out)


if __name__ == "__main__":
    unittest.main()
