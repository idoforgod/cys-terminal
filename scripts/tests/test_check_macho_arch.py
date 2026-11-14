"""Hermetic controls for scripts/check_macho_arch.py — the bundle architecture gate (0.15.0 · tracks T11, T11b ·
RULINGS BQ1(b), CA1–CA2, DB4–DB5 · owner order addendum 11: the Intel installer must never carry an Apple-silicon-only
python).

Every case builds SYNTHETIC Mach-O / fat headers in a temp dir (no real binary needed, no lipo, no network): thin
x86_64 / arm64, 32-bit (i386) and big-endian (ppc) thin headers, fat with both, fat without the target, fat64, the
CPU-subtype rule (x86_64h-only and arm64e-only do not satisfy; capability bits are masked), a header cut short, load
commands past the end, a fat slice outside the file, a Java class file that also starts with 0xCAFEBABE, plain text;
minimum macOS from LC_BUILD_VERSION and LC_VERSION_MIN_MACOSX; a fake cys.app whose runtime/python/bin/python3 is a
link to an arm64-only python3.12 (the owner's reported case); the same tree as a .tar.gz (the updater archive);
required entry points that are missing, dangling, looping or not Mach-O; an unreadable directory and file; and a
large archive whose single streaming pass is MEASURED (compressed bytes read ≤ archive size — any rewind of the
gzip stream would read more; RULINGS CA1). T11b (RULINGS DB4): a universal file whose fat table disagrees with a
slice's OWN header (CPU or subtype), a header / load commands that run past their slice, a load command past
sizeofcmds, a table listed out of offset order inside a streamed archive. T11b (RULINGS DB5): tar HARD-LINK members —
a required entry point packed as a hard link (target before it, `./` names, a chain), a hard link before its target,
a dangling one, a wrong-architecture target. A red case differs from its green control only in the planted fact.

    python3 scripts/tests/test_check_macho_arch.py
"""
import os
import re
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

X86_64, ARM64, ARM64_32, I386, PPC = 0x01000007, 0x0100000C, 0x0200000C, 7, 18
SUB = {X86_64: 3, ARM64: 0, ARM64_32: 1, I386: 3, PPC: 0}       # the ALL subtypes
X86_64H, ARM64E = 8, 2
LC_BUILD_VERSION, LC_VERSION_MIN_MACOSX = 0x32, 0x24
RT = "Contents/Resources/runtime/"
REQUIRE_APP = ["--require", RT + "python/bin/python3", "--require", RT + "node/bin/node",
               "--require", RT + "git/bin/git", "--require", RT + "uv/uv"]


def ver(s):
    p = [int(x) for x in s.split(".")] + [0, 0]
    return (p[0] << 16) | (p[1] << 8) | p[2]


def thin(cputype, minos=None, legacy=None, subtype=None, bits=64, endian="<"):
    """A Mach-O executable header (64- or 32-bit, either byte order) with optional minimum-OS load commands."""
    cmds, n = b"", 0
    if minos:
        cmds += struct.pack(endian + "IIIIII", LC_BUILD_VERSION, 24, 1, ver(minos), ver("15.0"), 0)
        n += 1
    if legacy:
        cmds += struct.pack(endian + "IIII", LC_VERSION_MIN_MACOSX, 16, ver(legacy), ver("10.15"))
        n += 1
    sub = SUB.get(cputype, 0) if subtype is None else subtype
    if bits == 64:
        hdr = struct.pack(endian + "IIIIIIII", 0xFEEDFACF, cputype, sub, 2, n, len(cmds), 0, 0)
    else:
        hdr = struct.pack(endian + "IIIIIII", 0xFEEDFACE, cputype, sub, 2, n, len(cmds), 0)
    return hdr + cmds + b"\0" * 64


def fat(slices, fat64=False, reverse_table=False):
    """A big-endian universal header; SLICES = [(cputype, thin bytes[, subtype])] at 4096-aligned offsets. The TABLE
    entry carries SLICES' cputype / subtype — the thin bytes may say something else (the DB4 controls).
    REVERSE_TABLE lists the entries in the opposite order of their offsets."""
    entry_fmt = ">IIQQII" if fat64 else ">IIIII"
    off, entries, body = 4096, [], b""
    for s in slices:
        cputype, data = s[0], s[1]
        sub = s[2] if len(s) > 2 else SUB.get(cputype, 0)
        pad = b"\0" * (-len(data) % 4096)
        entries.append(struct.pack(entry_fmt, cputype, sub, off, len(data), 12, 0) if fat64 else
                       struct.pack(entry_fmt, cputype, sub, off, len(data), 12))
        body += data + pad
        off += len(data) + len(pad)
    table = b"".join(reversed(entries) if reverse_table else entries)
    head = struct.pack(">II", 0xCAFEBABF if fat64 else 0xCAFEBABE, len(slices)) + table
    return head + b"\0" * (4096 - len(head)) + body


def with_slice_size(data, index, ssize):
    """DATA (a 32-bit fat file) with the SIZE field of table entry INDEX set to SSIZE."""
    b = bytearray(data)
    struct.pack_into(">I", b, 8 + 20 * index + 12, ssize)
    return bytes(b)


JAVA_CLASS = b"\xca\xfe\xba\xbe\x00\x00\x00\x34" + b"\x00" * 40        # Java 8 class: version where nfat would be
CUT_SHORT = b"\xcf\xfa\xed\xfe\x07\x00\x00\x01\x03\x00\x00\x00"           # thin magic + cputype, header cut after 12 bytes


class Gate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="macho-arch-")

    def tearDown(self):
        for dirpath, dirnames, _ in os.walk(self.tmp):
            for d in dirnames:
                try:
                    os.chmod(os.path.join(dirpath, d), 0o755)
                except OSError:
                    pass
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

    def app(self, python_cpu=X86_64, plist_min="10.13", python_sub=None):
        """A fake cys.app laid out like the real one (sidecars, runtime entry points, a universal dylib)."""
        a = "cys.app/Contents/"
        self.put(a + "Info.plist", (
            '<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" '
            '"http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0"><dict>'
            "<key>LSMinimumSystemVersion</key><string>%s</string></dict></plist>\n" % plist_min).encode(), 0o644)
        for exe in ("cys-app", "cys", "cysd"):
            self.put(a + "MacOS/" + exe, thin(X86_64, minos="10.13"))
        self.put(a + RT[len("Contents/"):] + "python/bin/python3.12", thin(python_cpu, minos="10.15", subtype=python_sub))
        self.link(a + RT[len("Contents/"):] + "python/bin/python3", "python3.12")
        self.put(a + RT[len("Contents/"):] + "python/lib/python3.12/os.py", b"import abc\n", 0o644)
        self.put(a + RT[len("Contents/"):] + "python/lib/python3.12/__pycache__/os.cpython-312.pyc",
                 b"\xcb\x0d\x0d\x0a" + b"\0" * 12, 0o644)
        self.put(a + RT[len("Contents/"):] + "node/bin/node", thin(X86_64, minos="11.0"))
        self.put(a + RT[len("Contents/"):] + "git/bin/git", thin(X86_64, legacy="10.13"))
        self.put(a + RT[len("Contents/"):] + "git/libexec/git-core/libSkiaSharp.dylib",
                 fat([(X86_64, thin(X86_64)), (ARM64, thin(ARM64))]))
        self.put(a + RT[len("Contents/"):] + "uv/uv", thin(X86_64, minos="10.12"))
        return os.path.join(self.tmp, "cys.app")

    def gate(self, *args):
        r = subprocess.run([sys.executable, SCRIPT] + list(args), capture_output=True, text=True, timeout=300)
        return r.returncode, r.stdout + r.stderr

    def one(self, data, target="x86_64-apple-darwin", name="f"):
        return self.gate("--target", target, self.put(name, data))

    # -- the owner's case and its green control --
    def test_intel_app_with_x86_64_python_passes_and_prints_entry_points(self):
        app = self.app()
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, app)
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
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, app)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL cys.app/Contents/Resources/runtime/python/bin/python3.12: arm64 (no slice that runs on every x86_64 Mac)", out)
        self.assertEqual(out.count("\nFAIL "), 1, out)
        self.assertIn("RESULT: FAIL", out)

    def test_the_same_app_fails_for_the_other_target_naming_every_thin_x86_64_file(self):
        rc, out = self.gate("--target", "aarch64-apple-darwin", self.app())
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
        plist = os.path.join(self.tmp, "cys.app/Contents/Info.plist")
        with open(plist, "rb") as f:
            data = f.read()
        with open(plist, "wb") as f:
            f.write(data.replace(b">10.13<", b">11.0<"))
        rc, out = self.gate("--target", "x86_64", app)
        self.assertEqual(rc, 0, out)
        self.assertNotIn("WARN (informational", out)

    # -- header shapes --
    def test_fat_containing_the_target_passes_fat_without_it_fails(self):
        both = fat([(X86_64, thin(X86_64)), (ARM64, thin(ARM64))])
        self.assertEqual(self.one(both, "x86_64-apple-darwin")[0], 0)
        self.assertEqual(self.one(both, "aarch64-apple-darwin")[0], 0)
        rc, out = self.one(fat([(ARM64, thin(ARM64)), (ARM64_32, thin(ARM64_32))]))
        self.assertEqual(rc, 1, out)
        self.assertIn("arm64+arm64_32 (no slice", out)

    def test_fat64_header_is_read(self):
        f64 = fat([(X86_64, thin(X86_64, minos="10.15")), (ARM64, thin(ARM64))], fat64=True)
        self.assertEqual(self.one(f64, "x86_64-apple-darwin")[0], 0)
        self.assertEqual(self.one(f64, "aarch64-apple-darwin")[0], 0)

    def test_thin_of_the_other_architecture_fails(self):
        self.put("d/x", thin(X86_64))
        self.put("d/a", thin(ARM64))
        rc, out = self.gate("--target", "x86_64-apple-darwin", os.path.join(self.tmp, "d"))
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL a: arm64 (no slice", out)
        self.assertNotIn("FAIL x:", out)

    def test_32_bit_and_big_endian_headers_are_read(self):
        rc, out = self.one(thin(I386, bits=32, legacy="10.6"))
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL f: i386 (no slice", out)
        rc, out = self.one(thin(PPC, bits=32, endian=">", legacy="10.4"), name="g")
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL g: ppc (no slice", out)

    # -- the CPU-subtype rule (RULINGS CA2(a)) --
    def test_x86_64h_only_does_not_satisfy_x86_64_but_beside_x86_64_it_does(self):
        rc, out = self.one(thin(X86_64, subtype=X86_64H))
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL f: x86_64h (no slice", out)
        rc, out = self.one(fat([(X86_64, thin(X86_64), X86_64H), (X86_64, thin(X86_64))]), name="g")
        self.assertEqual(rc, 0, out)

    def test_arm64e_only_does_not_satisfy_arm64_but_beside_arm64_it_does(self):
        rc, out = self.one(thin(ARM64, subtype=ARM64E), "aarch64-apple-darwin")
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL f: arm64e (no slice", out)
        rc, out = self.one(fat([(ARM64, thin(ARM64), 0x80000002), (ARM64, thin(ARM64))]), "aarch64-apple-darwin", "g")
        self.assertEqual(rc, 0, out)

    def test_capability_bits_of_the_subtype_are_masked(self):
        self.assertEqual(self.one(thin(X86_64, subtype=0x80000003))[0], 0)          # CPU_SUBTYPE_LIB64 | ALL
        self.assertEqual(self.one(thin(ARM64, subtype=0x00000001), "aarch64-apple-darwin", "g")[0], 0)  # arm64 v8

    # -- not measurable is not a pass (RULINGS CA2(c)) --
    def test_a_cut_short_thin_header_cannot_be_judged(self):
        self.put("d/ok", thin(X86_64))
        self.put("d/cut", CUT_SHORT)
        rc, out = self.gate("--target", "x86_64-apple-darwin", os.path.join(self.tmp, "d"))
        self.assertEqual(rc, 2, out)
        self.assertIn("CANNOT JUDGE cut: Mach-O header cut short", out)

    def test_load_commands_past_the_end_cannot_be_judged(self):
        data = bytearray(thin(X86_64, minos="10.15"))
        struct.pack_into("<I", data, 20, 1 << 16)                  # sizeofcmds far beyond the file
        rc, out = self.one(bytes(data))
        self.assertEqual(rc, 2, out)
        self.assertIn("run past the end of the file", out)

    def test_a_fat_slice_outside_the_file_and_a_cut_short_table_cannot_be_judged(self):
        data = fat([(X86_64, thin(X86_64)), (ARM64, thin(ARM64))])
        rc, out = self.one(data[:5000])                            # the second slice now lies beyond the end
        self.assertEqual(rc, 2, out)
        self.assertIn("lies outside the file", out)
        rc, out = self.one(struct.pack(">II", 0xCAFEBABE, 2) + b"\0" * 10, name="g")
        self.assertEqual(rc, 2, out)
        self.assertIn("fat table cut short", out)

    # -- T11b · RULINGS DB4: each slice's OWN header, within the slice's own bounds --
    def test_a_fat_table_that_says_x86_64_over_an_arm64_slice_fails(self):
        # the reviewer's M2: the table entry says x86_64/ALL, the bytes at its offset are a valid arm64 thin header
        rc, out = self.one(fat([(X86_64, thin(ARM64))]))
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL f: fat table entry 0 says x86_64 but the slice's own header is arm64", out)
        self.assertIn("FAIL f: arm64 (no slice that runs on every x86_64 Mac)", out)
        # beside a correct arm64 slice, for the arm64 target: still a mis-assembled file → FAIL
        rc, out = self.one(fat([(X86_64, thin(ARM64)), (ARM64, thin(ARM64))]), "aarch64-apple-darwin", "g")
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL g: fat table entry 0 says x86_64 but the slice's own header is arm64", out)
        # green control: the same two slices, table == headers
        self.assertEqual(self.one(fat([(X86_64, thin(X86_64)), (ARM64, thin(ARM64))]), name="h")[0], 0)

    def test_a_fat_subtype_that_differs_from_the_slice_header_fails(self):
        # table x86_64 (ALL), header x86_64h — the table would satisfy the target, the slice would not
        rc, out = self.one(fat([(X86_64, thin(X86_64, subtype=X86_64H), 3)]))
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL f: fat table entry 0 says x86_64 but the slice's own header is x86_64h", out)
        # capability bits differ only (masked on both sides) → not a mismatch
        self.assertEqual(self.one(fat([(X86_64, thin(X86_64, subtype=0x80000003), 3)]), name="g")[0], 0)

    def test_a_slice_header_or_its_load_commands_past_the_slice_cannot_be_judged(self):
        data = fat([(X86_64, thin(X86_64, minos="10.15")), (ARM64, thin(ARM64))])
        rc, out = self.one(with_slice_size(data, 0, 16))           # the 32-byte header does not fit in 16 bytes
        self.assertEqual(rc, 2, out)
        self.assertIn("Mach-O header runs past the end of fat slice 0", out)
        rc, out = self.one(with_slice_size(data, 0, 40), name="g")  # header fits, its 24 bytes of commands do not
        self.assertEqual(rc, 2, out)
        self.assertIn("load commands (24 bytes) run past the end of fat slice 0", out)
        self.assertEqual(self.one(with_slice_size(data, 0, 56), name="h")[0], 0)   # exactly header + commands: fits

    def test_a_load_command_past_sizeofcmds_cannot_be_judged(self):
        data = bytearray(thin(X86_64, minos="10.15"))
        struct.pack_into("<I", data, 32 + 4, 200)                  # LC_BUILD_VERSION claims 200 bytes of 24
        rc, out = self.one(bytes(data))
        self.assertEqual(rc, 2, out)
        self.assertIn("load command 0 (200 bytes) runs past sizeofcmds", out)

    def test_every_slice_is_read_forward_in_a_streamed_archive_whatever_the_table_order(self):
        # the table lists the arm64 slice (at the higher offset) first; the streamed member can only be read forward
        self.put("t/cys.app/Contents/MacOS/cys", fat([(X86_64, thin(X86_64, minos="10.15")),
                                                      (ARM64, thin(ARM64) + os.urandom(70000))], reverse_table=True))
        arc = self.make_tar("rev.tar.gz", os.path.join(self.tmp, "t", "cys.app"))
        for target in ("x86_64-apple-darwin", "aarch64-apple-darwin"):
            rc, out = self.gate("--target", target, arc)
            self.assertEqual(rc, 0, out)
            self.assertIn("arm64+x86_64 1", out)                   # table order, each name from the slice's header
        bad = fat([(X86_64, thin(ARM64) + os.urandom(70000)), (ARM64, thin(ARM64))], reverse_table=True)
        self.put("u/cys.app/Contents/MacOS/cys", bad)
        rc, out = self.gate("--target", "aarch64-apple-darwin", self.make_tar("rev-bad.tar.gz", os.path.join(self.tmp, "u", "cys.app")))
        self.assertEqual(rc, 1, out)
        self.assertIn("fat table entry 1 says x86_64 but the slice's own header is arm64", out)

    @unittest.skipIf(hasattr(os, "geteuid") and os.geteuid() == 0, "root reads everything")
    def test_an_unreadable_directory_or_file_cannot_be_judged(self):
        self.put("d/ok", thin(X86_64))
        self.put("d/locked/inner", thin(X86_64))
        os.chmod(os.path.join(self.tmp, "d/locked"), 0)
        rc, out = self.gate("--target", "x86_64-apple-darwin", os.path.join(self.tmp, "d"))
        self.assertEqual(rc, 2, out)
        self.assertIn("unreadable directory", out)
        os.chmod(os.path.join(self.tmp, "d/locked"), 0o755)
        os.chmod(os.path.join(self.tmp, "d/locked/inner"), 0)
        rc, out = self.gate("--target", "x86_64-apple-darwin", os.path.join(self.tmp, "d"))
        self.assertEqual(rc, 2, out)
        self.assertIn("CANNOT JUDGE locked/inner: unreadable", out)

    def test_java_class_and_text_are_not_mach_o(self):
        self.put("d/ok", thin(X86_64))
        self.put("d/Main.class", JAVA_CLASS, 0o644)
        self.put("d/readme.txt", b"hello\n", 0o644)
        rc, out = self.gate("--target", "x86_64-apple-darwin", os.path.join(self.tmp, "d"))
        self.assertEqual(rc, 0, out)
        self.assertIn("Mach-O files: 1 ", out + " ")
        self.assertNotIn("Main.class", out)

    # -- required entry points (RULINGS CA2(b)) --
    def test_missing_dangling_looping_or_non_mach_o_required_entry_points_fail(self):
        app = self.app()
        os.remove(os.path.join(app, "Contents/Resources/runtime/python/bin/python3.12"))     # python3 now dangles
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, app)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL required entry point cys.app/Contents/Resources/runtime/python/bin/python3: dangling link", out)
        self.put("cys.app/Contents/Resources/runtime/python/bin/python3.12", b"#!/bin/sh\n")   # not Mach-O
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, app)
        self.assertEqual(rc, 1, out)
        self.assertIn("python3: not a Mach-O file", out)
        os.remove(os.path.join(app, "Contents/Resources/runtime/python/bin/python3.12"))
        os.symlink("python3", os.path.join(app, "Contents/Resources/runtime/python/bin/python3.12"))  # a loop
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, app)
        self.assertEqual(rc, 1, out)
        self.assertIn("python3: link loop", out)
        os.remove(os.path.join(app, "Contents/Resources/runtime/uv/uv"))
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, app)
        self.assertIn("FAIL required entry point Contents/Resources/runtime/uv/uv: missing", out)

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
        self.assertEqual(rc, 1, out)
        self.assertIn("required entry point uv/uv: missing", out)

    # -- the updater archive: one streaming pass (RULINGS CA1) --
    def make_tar(self, name, src_dir, arcname="cys.app"):
        arc = os.path.join(self.tmp, name)
        with tarfile.open(arc, "w:gz") as t:
            t.add(src_dir, arcname=arcname)
        return arc

    def test_tar_gz_archive_red_and_green(self):
        self.app(python_cpu=ARM64)
        arc = self.make_tar("red.tar.gz", os.path.join(self.tmp, "cys.app"))
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, arc)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL cys.app/Contents/Resources/runtime/python/bin/python3.12: arm64", out)
        shutil.rmtree(os.path.join(self.tmp, "cys.app"))
        self.app()
        arc = self.make_tar("green.tar.gz", os.path.join(self.tmp, "cys.app"))
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, arc)
        self.assertEqual(rc, 0, out)
        self.assertRegex(out, r"runtime/python/bin/python3 -> python3\.12\s+x86_64\s+min 10\.15")

    def test_a_large_archive_is_read_in_one_linear_pass(self):
        # several thousand members; Mach-O files scattered so that archive order != sorted-name order, the python
        # link added LAST (after its target), fat binaries whose target slice sits late in the member
        arc = os.path.join(self.tmp, "big.tar.gz")
        n_text, step, n_macho = 4000, 66, 0
        with tarfile.open(arc, "w:gz") as t:
            def add(rel, data, mode=0o644):
                ti = tarfile.TarInfo("cys.app/" + rel)
                ti.size, ti.mode = len(data), mode
                t.addfile(ti, __import__("io").BytesIO(data))
            for i in range(n_text):
                add("Contents/Resources/runtime/python/lib/z%05d/m.py" % (n_text - i), (b"# %d\n" % i) * 40)
                if i % step == 0:
                    k = i // step
                    data = thin(X86_64, minos="10.15") if k % 2 else \
                        fat([(ARM64, thin(ARM64) + os.urandom(20000)), (X86_64, thin(X86_64, minos="10.15"))])
                    add("Contents/Resources/runtime/lib/a%03d.dylib" % (1000 - k), data, 0o755)
                    n_macho += 1
            for rel in ("Contents/MacOS/cys", "Contents/Resources/runtime/python/bin/python3.12",
                        "Contents/Resources/runtime/node/bin/node", "Contents/Resources/runtime/git/bin/git",
                        "Contents/Resources/runtime/uv/uv"):
                add(rel, thin(X86_64, minos="10.15"), 0o755)
            ti = tarfile.TarInfo("cys.app/Contents/Resources/runtime/python/bin/python3")
            ti.type, ti.linkname = tarfile.SYMTYPE, "python3.12"
            t.addfile(ti)
        self.assertGreater(n_macho, 50)
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, arc)
        self.assertEqual(rc, 0, out)
        m = re.search(r"archive read in one pass: (\d+) members, (\d+) of (\d+) compressed bytes", out)
        self.assertIsNotNone(m, out)
        members, read, size = (int(x) for x in m.groups())
        self.assertEqual(members, n_text + n_macho + 6, out)
        self.assertLessEqual(read, size, "the compressed stream was read more than once (a rewind) — RULINGS CA1")
        self.assertEqual(read, size, "every compressed byte must pass through the counting reader exactly once")
        self.assertIn("x86_64 ", out)
        self.assertRegex(out, r"python3 -> python3\.12\s+x86_64\s+min 10\.15")

    # -- T11b · RULINGS DB5: tar HARD-LINK members --
    def hard_tar(self, name, plan, python_cpu=X86_64, prefix=""):
        """The fake app as a .tar.gz whose runtime/python/bin/python3 is packed by PLAN instead of a symbolic link:
        PLAN = [("before"|"after", member rel, linkname rel)] — hard-link members written before or after the tree."""
        self.app(python_cpu=python_cpu)
        os.remove(os.path.join(self.tmp, "cys.app/Contents/Resources/runtime/python/bin/python3"))
        arc = os.path.join(self.tmp, name)
        py = "cys.app/Contents/Resources/runtime/python/bin/"

        def hard(t, rel, target):
            ti = tarfile.TarInfo(prefix + py + rel)
            ti.type, ti.linkname, ti.mode = tarfile.LNKTYPE, prefix + py + target, 0o755
            t.addfile(ti)
        with tarfile.open(arc, "w:gz") as t:
            for when, rel, target in plan:
                if when == "before":
                    hard(t, rel, target)
            t.add(os.path.join(self.tmp, "cys.app"), arcname=prefix + "cys.app")
            for when, rel, target in plan:
                if when == "after":
                    hard(t, rel, target)
        return arc

    def test_a_required_entry_point_packed_as_a_hard_link_is_found(self):
        # the reviewer's M3: python3.12 a regular member, python3 a hard link to it (after it, as every tar writes)
        arc = self.hard_tar("hard.tar.gz", [("after", "python3", "python3.12")])
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, arc)
        self.assertEqual(rc, 0, out)
        self.assertRegex(out, r"runtime/python/bin/python3 -> python3\.12\s+x86_64\s+min 10\.15")
        self.assertIn("x86_64 7", out)                             # the hard link is the same file, counted once
        # the same with `./` member names (tar's own normalisation)
        arc = self.hard_tar("hard-dot.tar.gz", [("after", "python3", "python3.12")], prefix="./")
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, arc)
        self.assertEqual(rc, 0, out)

    def test_a_chain_of_hard_links_resolves(self):
        arc = self.hard_tar("chain.tar.gz", [("after", "python3.1", "python3.12"), ("after", "python3", "python3.1")])
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, arc)
        self.assertEqual(rc, 0, out)
        self.assertRegex(out, r"runtime/python/bin/python3 -> python3\.12\s+x86_64")

    def test_a_hard_link_before_its_target_fails(self):
        # tar's own rule (tarfile, libarchive, GNU tar, the Rust tar crate): a hard link names an ALREADY archived
        # member — an extractor creates it from the file it already wrote; this one would be missing after extraction
        arc = self.hard_tar("early.tar.gz", [("before", "python3", "python3.12")])
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, arc)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL required entry point cys.app/Contents/Resources/runtime/python/bin/python3: hard link "
                      "before its target cys.app/Contents/Resources/runtime/python/bin/python3.12", out)

    def test_a_dangling_hard_link_fails(self):
        arc = self.hard_tar("dangling.tar.gz", [("after", "python3", "python3.13")])
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, arc)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL required entry point cys.app/Contents/Resources/runtime/python/bin/python3: dangling hard link", out)

    def test_a_hard_link_to_a_wrong_architecture_file_fails(self):
        arc = self.hard_tar("wrong.tar.gz", [("after", "python3", "python3.12")], python_cpu=ARM64)
        rc, out = self.gate("--target", "x86_64-apple-darwin", *REQUIRE_APP, arc)
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL cys.app/Contents/Resources/runtime/python/bin/python3.12: arm64 (no slice", out)
        self.assertRegex(out, r"runtime/python/bin/python3 -> python3\.12\s+arm64")

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
    def test_a_real_system_binary_shows_the_subtype_rule(self):
        # /bin/echo = x86_64 (ALL) + arm64e (Apple's pointer-authentication ABI): it satisfies x86_64 but, by the
        # rule, not a third-party arm64 target (macOS loads arm64e only for Apple's own code)
        rc, out = self.gate("--target", "x86_64-apple-darwin", "/bin/echo")
        self.assertEqual(rc, 0, out)
        rc, out = self.gate("--target", "aarch64-apple-darwin", "/bin/echo")
        self.assertEqual(rc, 1, out)
        self.assertIn("arm64e", out)


if __name__ == "__main__":
    unittest.main()
