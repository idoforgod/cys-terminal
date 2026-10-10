#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_inject_context_soul_l0.py - 0.14.50 B-057: the L0 soul block is never "banner + zero bytes".

Defect (PLAN-50 sheet B-057, MEASURED at v0.14.48): `cysjavis-pack/hooks/inject-context.sh` L0 block selected soul text
with `awk '/^## \\[/{p=1} p'`; today's soul files use plain `## title` headings, so the selector printed 0 bytes in every
lane and nothing said so. Head decision 2 (HEAD-DECISIONS-50): take the base-lane H1 form - every `## ` heading plus one
imperative pointer to read the whole file - with a LOUD line on zero headings / unreadable file; not H2/H3.

Pinned here (L0 block = from the banner line to the first empty line after it):
  T1  startup, plain `## ` headings  -> every heading listed in order, pointer with the resolved path and whole-file size
  T2  resume                         -> same block
  T3  clear / compact                -> no L0 block (unchanged: the block runs on startup/resume only)
  T4  only body text, no `## `       -> loud zero-heading line + pointer; never banner+nothing
  T5  unreadable soul (POSIX, non-root) -> loud unreadable line naming the path; no size claim
  T6  title list over 8,192 B        -> omission warning with count, list not cut silently, pointer kept
  T7  whole file over 32,768 B       -> bloat line; all headings still listed (titles are not truncated)
  T8  backslash in a heading         -> printed literally, and the output after it is not cut (printf %b, H-HOOK-1)
  T9  CRLF soul                      -> no carriage return in the injected titles
  T10 old `## [ANCHOR]` headings     -> listed too (no format is silently dropped)
  T11 body lines are NOT injected    -> H1 injects titles only (whole text comes from session-start.sh / the pointer)
  T12 lane resolution kept           -> `$CYS_PACK_DIR/soul.md` is the file named by the pointer

Live-untouched: scratch HOME / pack / cwd per case, stub `cys` (no daemon), PATH = stubs + /usr/bin:/bin.
    CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_inject_context_soul_l0.py
Red on the base: INJECT_HOOK_UNDER_TEST=<copy of the old hook placed next to _lib.sh> runs the same pins against it.
POSIX-only like the other inject-context pins (the hook is bash; Windows runs it through run_bootstrap_health).
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
HOOK = os.environ.get("INJECT_HOOK_UNDER_TEST") or os.path.normpath(
    os.path.join(SELF, "..", "..", "hooks", "inject-context.sh"))
BASH = shutil.which("bash") or "/bin/bash"

BANNER = "■ 불변 정체·절대규칙 (L0 · soul.md ANCHOR — 매 부팅 재확립)"
POINTER_HEAD = "개 절의 전문(금지선·오너 절대규칙 포함)은 제목만으로 지킬 수 없다 — 지금 바로 읽어라: cat "
ZERO_MARK = "절 제목을 0개 찾았다"
UNREADABLE_MARK = "soul 파일을 읽지 못했다"
TCAP_MARK = "— 목록을 생략한다. 아래 명령으로 전문을 읽어라."
BLOAT_MARK = "이 주입은 제목 목록이라 잘린 것은 없다"

PLAIN_SOUL = (
    "# soul - fixture\n"
    "\n"
    "intro line BODY-SENTINEL-INTRO\n"
    "## 금지선 — 절대 넘지 않는다\n"
    "- rule one BODY-SENTINEL-1\n"
    "## 오너 규칙 — 호칭과 보고\n"
    "- rule two BODY-SENTINEL-2\n"
    "## 세 번째 절\n"
    "text\n"
)


def _write_exec(path, body):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, 0o755)


class Case(object):
    def __init__(self):
        self.tmp = tempfile.mkdtemp(prefix="b057-")
        self.home = os.path.join(self.tmp, "home")
        self.pack = os.path.join(self.tmp, "pack")
        self.cwd = os.path.join(self.tmp, "work")
        self.bindir = os.path.join(self.tmp, "stubbin")
        for d in (self.home, self.pack, self.cwd, self.bindir):
            os.makedirs(d, exist_ok=True)
        # no daemon: `cys` stub answers nothing; ps/lsof stubs report one claude seat
        _write_exec(os.path.join(self.bindir, "cys"), "#!/bin/sh\nexit 0\n")
        _write_exec(os.path.join(self.bindir, "ps"), "#!/bin/sh\necho '9000 claude /x/claude'\n")
        _write_exec(os.path.join(self.bindir, "lsof"), "#!/bin/sh\nprintf 'n%s\\n' \"$PWD\"\n")
        self.soul = os.path.join(self.pack, "soul.md")

    def write_soul(self, text, newline="\n"):
        with open(self.soul, "w", encoding="utf-8", newline=newline) as f:
            f.write(text)

    def run(self, source="startup"):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith("CYS_") and not k.startswith("_CYS_")}
        env.update({"PATH": self.bindir + os.pathsep + "/usr/bin" + os.pathsep + "/bin",
                    "HOME": self.home, "CYS_PACK_DIR": self.pack, "CYS_ROOT": self.tmp,
                    "TMPDIR": self.tmp, "CYS_ROLE": "master"})
        payload = json.dumps({"source": source, "cwd": self.cwd})
        r = subprocess.run([BASH, HOOK], input=payload, capture_output=True, text=True,
                           encoding="utf-8", env=env, timeout=60)
        return r

    def cleanup(self):
        for root, dirs, files in os.walk(self.tmp):
            for n in dirs + files:
                try:
                    os.chmod(os.path.join(root, n), 0o700)
                except OSError:
                    pass
        shutil.rmtree(self.tmp, ignore_errors=True)


def l0_block(out):
    """Lines from the banner (exclusive) to the first empty line after it; None when there is no banner."""
    lines = out.split("\n")
    if BANNER not in lines:
        return None
    i = lines.index(BANNER)
    block = []
    for l in lines[i + 1:]:
        if l == "":
            break
        block.append(l)
    return block


class SoulL0(unittest.TestCase):
    def setUp(self):
        self.c = Case()

    def tearDown(self):
        self.c.cleanup()

    def _block(self, source="startup"):
        r = self.c.run(source)
        self.assertEqual(r.returncode, 0, "hook must exit 0: %r" % r.stderr[-400:])
        return r, l0_block(r.stdout)

    def _assert_pointer(self, block, n):
        ptr = [l for l in block if POINTER_HEAD in l]
        self.assertEqual(len(ptr), 1, "exactly one pointer line expected: %r" % block)
        size = os.path.getsize(self.c.soul)
        self.assertTrue(ptr[0].startswith("★위 %d" % n), "pointer heading count: %r" % ptr[0])
        self.assertTrue(ptr[0].endswith("cat %s  [전문 %dB]" % (self.c.soul, size)),
                        "pointer must name the resolved soul path and its whole-file size: %r" % ptr[0])

    def test_t1_startup_lists_every_plain_heading_and_points_to_the_file(self):
        self.c.write_soul(PLAIN_SOUL)
        r, block = self._block("startup")
        self.assertIsNotNone(block, "fixture: no L0 banner on startup\n" + r.stdout[:600])
        self.assertTrue(block, "★B-057: L0 banner followed by ZERO bytes (the 0.14.48 defect)")
        heads = [l for l in block if l.startswith("## ")]
        self.assertEqual(heads, ["## 금지선 — 절대 넘지 않는다", "## 오너 규칙 — 호칭과 보고", "## 세 번째 절"],
                         "every `## ` heading in file order: %r" % block)
        self._assert_pointer(block, 3)

    def test_t2_resume_same_block(self):
        self.c.write_soul(PLAIN_SOUL)
        _, block = self._block("resume")
        self.assertIsNotNone(block)
        self.assertEqual(len([l for l in block if l.startswith("## ")]), 3, "resume: %r" % block)
        self._assert_pointer(block, 3)

    def test_t3_clear_and_compact_have_no_l0_block(self):
        self.c.write_soul(PLAIN_SOUL)
        for src in ("clear", "compact"):
            with self.subTest(source=src):
                r, block = self._block(src)
                self.assertIsNone(block, "%s must not carry the L0 block (unchanged rule)" % src)
                self.assertNotIn(POINTER_HEAD, r.stdout)

    def test_t4_zero_headings_is_loud_not_empty(self):
        self.c.write_soul("# only a title\n\nbody without sections BODY-SENTINEL-Z\n")
        _, block = self._block("startup")
        self.assertIsNotNone(block)
        self.assertTrue(any(ZERO_MARK in l for l in block), "zero headings must be LOUD: %r" % block)
        self._assert_pointer(block, 0)
        self.assertFalse(any("BODY-SENTINEL" in l for l in block))

    def test_t5_unreadable_soul_is_loud(self):
        if os.name == "nt" or (hasattr(os, "geteuid") and os.geteuid() == 0):
            self.skipTest("permission bits are not enforced here (Windows or root)")
        self.c.write_soul(PLAIN_SOUL)
        os.chmod(self.c.soul, 0)
        _, block = self._block("startup")
        self.assertIsNotNone(block, "the file exists, so the block must run")
        loud = [l for l in block if UNREADABLE_MARK in l]
        self.assertEqual(len(loud), 1, "unreadable soul must be LOUD: %r" % block)
        self.assertTrue(loud[0].endswith(self.c.soul), "the loud line names the path: %r" % loud[0])
        self.assertFalse(any(POINTER_HEAD in l for l in block), "no size claim for a file that was not read")

    def test_t6_title_list_over_cap_is_named_not_cut(self):
        heads = ["## 절 %04d %s" % (i, "가" * 40) for i in range(120)]   # > 8,192 B of titles
        self.c.write_soul("# t\n" + "\n".join(heads) + "\n")
        _, block = self._block("startup")
        warn = [l for l in block if TCAP_MARK in l]
        self.assertEqual(len(warn), 1, "over-cap title list must be announced: %r" % block[:3])
        self.assertIn("(절 120개)", warn[0])
        self.assertFalse(any(l.startswith("## ") for l in block), "list omitted whole, never cut in the middle")
        self._assert_pointer(block, 120)

    def test_t7_whole_file_over_cap_reports_bloat_and_keeps_all_titles(self):
        body = ("본문 " * 2000 + "\n")
        self.c.write_soul("# t\n## A\n" + body * 3 + "## B\n" + body + "## C\n")   # > 32,768 B
        self.assertGreater(os.path.getsize(self.c.soul), 32768, "fixture")
        _, block = self._block("startup")
        self.assertEqual([l for l in block if l.startswith("## ")], ["## A", "## B", "## C"])
        self.assertTrue(any(BLOAT_MARK in l for l in block), "bloat line expected: %r" % block[-2:])
        self._assert_pointer(block, 3)

    def test_t8_backslash_heading_is_literal_and_does_not_cut_output(self):
        self.c.write_soul("# t\n## path C:\\cys\\new and \\c stop\n## after\n")
        r, block = self._block("startup")
        self.assertIn("## path C:\\cys\\new and \\c stop", block, "backslashes must print literally: %r" % block)
        self.assertIn("## after", block, "a `\\c` in a title must not cut the rest (printf %b)")
        self._assert_pointer(block, 2)

    def test_t9_crlf_soul_titles_have_no_carriage_return(self):
        self.c.write_soul(PLAIN_SOUL, newline="\r\n")
        _, block = self._block("startup")
        heads = [l for l in block if l.startswith("## ")]
        self.assertEqual(len(heads), 3, "CRLF headings must be found: %r" % block)
        self.assertFalse(any("\r" in l for l in heads), "carriage return leaked into titles: %r" % heads)

    def test_t10_old_bracket_headings_are_listed_too(self):
        self.c.write_soul("# t\n## [ABSOLUTE ANCHOR] one\nx\n## plain two\n")
        _, block = self._block("startup")
        self.assertEqual([l for l in block if l.startswith("## ")], ["## [ABSOLUTE ANCHOR] one", "## plain two"])

    def test_t11_body_text_is_not_injected(self):
        self.c.write_soul(PLAIN_SOUL)
        r, block = self._block("startup")
        self.assertFalse([l for l in block if "BODY-SENTINEL" in l], "H1 injects titles only: %r" % block)

    def test_t12_lane_soul_is_the_one_named(self):
        self.c.write_soul(PLAIN_SOUL)
        other = os.path.join(self.c.home, ".cys", "pack")
        os.makedirs(other, exist_ok=True)
        with open(os.path.join(other, "soul.md"), "w", encoding="utf-8") as f:
            f.write("# base\n## BASE-ONLY HEADING\n")
        _, block = self._block("startup")
        self.assertNotIn("## BASE-ONLY HEADING", block, "the lane soul must win over the base default")
        self._assert_pointer(block, 3)


if __name__ == "__main__":
    if os.name == "nt":
        print("SKIP: POSIX-only (bash hook; Windows runs inject-context through run_bootstrap_health)")
        sys.exit(0)
    unittest.main(verbosity=2)
