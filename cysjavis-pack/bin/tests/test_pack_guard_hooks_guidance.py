#!/usr/bin/env python3
"""F10-B (0.14.50): pack-guard.sh carries the guidance sentence for an edit under hooks/ - and only there.

Pins (lane A's Rust test pins the const + CLI output; this file pins the hook side, per INTERFACE-F10-laneA.md section 2):
  1. the three literal strings are in pack-guard.sh: `~/.cys/local/hooks/<이벤트>.d/`, `cys pack-merge --file`, `--propose`
  2. the hook, run for an edit of a hooks/ file of a system-owned pack file, prints the sentence with `<rel>` = that path
  3. an edit outside hooks/ does not get the sentence (no spread of the text)
  4. the sentence in the hook is the SAME words as the const HOOKS_LOCAL_GUIDANCE in src/pack.rs (one origin; only <rel> differs)
  5. the hook stays non-blocking: exit 0, valid JSON additionalContext
POSIX only (bash hook); Windows runs pack-guard through run_bootstrap_health H-WIN-6."""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PACK = os.path.dirname(os.path.dirname(HERE))
ROOT = os.path.dirname(PACK)
HOOK = os.path.join(PACK, "hooks", "pack-guard.sh")
PACK_RS = os.path.join(ROOT, "src", "pack.rs")
BASH = shutil.which("bash") or "/bin/bash"
LITERALS = ("~/.cys/local/hooks/<이벤트>.d/", "cys pack-merge --file", "--propose")


def rust_const():
    with open(PACK_RS, encoding="utf-8") as f:
        m = re.search(r'pub const HOOKS_LOCAL_GUIDANCE: &str = "((?:[^"\\]|\\.)*)";', f.read())
    return m.group(1) if m else None


def run_hook(rel, own="system"):
    tmp = tempfile.mkdtemp(prefix="f10b-")
    try:
        binp = os.path.join(tmp, "bin")
        os.makedirs(binp)
        stub = os.path.join(binp, "cys")
        with open(stub, "w") as f:
            f.write('#!/bin/sh\ncase "$1" in pack-ownership) echo %s; exit 0;; esac\nexit 0\n' % own)
        os.chmod(stub, 0o755)
        pack = os.path.join(tmp, "pack")
        os.makedirs(os.path.join(pack, "hooks"))
        env = {k: v for k, v in os.environ.items() if not k.startswith("CYS_") and not k.startswith("_CYS_")}
        env.update({"HOME": os.path.join(tmp, "home"), "CYS_PACK_DIR": pack, "TMPDIR": os.path.join(tmp, "stamps"),
                    "PATH": binp + os.pathsep + "/usr/bin" + os.pathsep + "/bin"})
        payload = json.dumps({"tool_input": {"file_path": os.path.join(pack, rel)}, "session_id": "s1"})
        r = subprocess.run([BASH, HOOK], input=payload, capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
        return r
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def context_of(r):
    if not r.stdout.strip():
        return ""
    return json.loads(r.stdout)["hookSpecificOutput"]["additionalContext"]


class PackGuardHooksGuidance(unittest.TestCase):
    def test_1_literals_are_in_the_hook_source(self):
        with open(HOOK, encoding="utf-8") as f:
            src = f.read()
        for lit in LITERALS:
            self.assertIn(lit, src, "pack-guard.sh must carry %r" % lit)

    def test_2_edit_under_hooks_prints_the_sentence_with_the_path(self):
        r = run_hook("hooks/guard.sh")
        self.assertEqual(r.returncode, 0, r.stderr[-300:])
        ctx = context_of(r)
        for lit in LITERALS:
            self.assertIn(lit, ctx, "hook output lacks %r: %r" % (lit, ctx[:400]))
        self.assertIn("cys pack-merge --file hooks/guard.sh --propose", ctx)

    def test_3_edit_outside_hooks_does_not_get_the_sentence(self):
        r = run_hook("bin/javis_x.py")
        self.assertEqual(r.returncode, 0)
        ctx = context_of(r)
        self.assertIn("pack-guard", ctx, "control: the ordinary warning still prints")
        self.assertNotIn("~/.cys/local/hooks/<이벤트>.d/", ctx)
        self.assertNotIn("hooks/ 아래 vendor 파일의 수정은", ctx)

    def test_4_same_words_as_the_rust_const(self):
        const = rust_const()
        self.assertIsNotNone(const, "HOOKS_LOCAL_GUIDANCE not found in src/pack.rs")
        r = run_hook("hooks/guard.sh")
        ctx = context_of(r)
        self.assertIn(const.replace("<rel>", "hooks/guard.sh"), ctx,
                      "the hook's sentence differs from the one origin in src/pack.rs")

    def test_5_non_system_file_is_silent_and_exit_zero(self):
        r = run_hook("hooks/mine.sh", own="custom")
        self.assertEqual(r.returncode, 0)
        self.assertEqual(r.stdout.strip(), "", "a custom (non vendor) file must stay silent")


if __name__ == "__main__":
    if os.name == "nt":
        print("SKIP: POSIX-only (bash hook); Windows: run_bootstrap_health H-WIN-6")
        sys.exit(0)
    unittest.main(verbosity=2)
