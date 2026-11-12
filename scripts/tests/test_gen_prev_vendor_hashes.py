"""Hermetic controls for scripts/gen_prev_vendor_hashes.py — the previous-release hash table generator and its
`--check` freshness gate (0.15.0 「스스로 해결」 T2c · M11 다 · CONTRACTS C8 8.5 / O-C8-5 (i) · testplan H-11da-A2-01-01).

Why: with the install record missing or corrupt, the table is the only authority that tells "old vendor bytes" from
"a user edit" (design §4-11 다, appendix A-2 #1). A stale table silently turns old vendor copies into user edits, so
`--check` must go red when the table no longer matches the repository, and must refuse — never pass — when it cannot
see the window (a shallow clone, no tags). Each red case has a green control that differs only in the planted change.

Every repository here is a fresh temp git repository (no network; global and system git config ignored). The
generator is copied into it as scripts/gen_prev_vendor_hashes.py, so it reads and writes that repository only.

    python3 scripts/tests/test_gen_prev_vendor_hashes.py
"""
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
# Mutation testing: GEN_PREV_VENDOR_UNDER_TEST=<mutant path> runs the same cases against that script instead.
SCRIPT = os.environ.get("GEN_PREV_VENDOR_UNDER_TEST") or os.path.normpath(os.path.join(HERE, "..", "gen_prev_vendor_hashes.py"))
OUT_REL = os.path.join("src", "pack_prev_vendor.rs")

ENV = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
ENV.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1", GIT_TERMINAL_PROMPT="0",
           GIT_AUTHOR_NAME="t2c", GIT_AUTHOR_EMAIL="t2c@example.invalid",
           GIT_COMMITTER_NAME="t2c", GIT_COMMITTER_EMAIL="t2c@example.invalid")


def sha(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


class Repo:
    def __init__(self, root):
        self.root = root
        os.makedirs(os.path.join(root, "scripts"))
        os.makedirs(os.path.join(root, "src"))
        shutil.copyfile(SCRIPT, os.path.join(root, "scripts", "gen_prev_vendor_hashes.py"))
        self.git("-c", "init.defaultBranch=main", "init", "-q")

    def git(self, *a):
        r = subprocess.run(["git", "-C", self.root, "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false",
                            "-c", "core.excludesFile=" + os.devnull, *a],
                           env=ENV, capture_output=True, text=True, timeout=60)
        if r.returncode != 0:
            raise AssertionError("git %s failed: %s" % (" ".join(a), r.stderr))
        return r.stdout.strip()

    def write(self, rel, text):
        p = os.path.join(self.root, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)

    def commit(self, files, tag=None):
        """FILES: pack-relative paths (written under cysjavis-pack/), except `docs/…` (outside the pack)."""
        for rel, text in files.items():
            self.write(rel if rel.startswith("docs/") else "cysjavis-pack/" + rel, text)
        self.git("add", "-A")
        self.git("commit", "-q", "--allow-empty", "-m", "c")
        if tag:
            self.git("tag", tag)
        return self.git("rev-parse", "HEAD")

    def gen(self, *args, cwd=None):
        r = subprocess.run([sys.executable, os.path.join(self.root, "scripts", "gen_prev_vendor_hashes.py"), *args],
                           cwd=cwd or self.root, env=ENV, capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout + r.stderr

    def table(self):
        with open(os.path.join(self.root, OUT_REL), encoding="utf-8") as f:
            return f.read()


def rows_of(text):
    out = {}
    for m in re.finditer(r'^    \("([^"]+)", &\[(.*)\]\),$', text, re.M):
        out[m.group(1)] = re.findall(r'"([0-9a-f]{64})"', m.group(2))
    return out


class Generator(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="gen-prev-vendor-")
        self.repo = Repo(os.path.join(self.tmp, "repo"))

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def history(self):
        """v0.13.9 (outside) → v0.14.0 → pack-v0.14.1 → v0.15.0 → HEAD tagged v0.15.1 (this release)."""
        r = self.repo
        r.commit({"bin/a.py": "A1", "bin/same.py": "S", "tests/t.py": "T1", ".hidden": "H1",
                  "bin/__pycache__/x.pyc": "P1"}, tag="v0.13.9")
        r.commit({"bin/a.py": "A2", "tests/t.py": "T2", ".hidden": "H2", "bin/__pycache__/x.pyc": "P2"}, tag="v0.14.0")
        r.commit({"bin/a.py": "A3"}, tag="pack-v0.14.1")
        r.commit({"bin/a.py": "A4"}, tag="v0.15.0")
        r.git("tag", "v0.15.0-rc1")          # not a release tag name
        r.git("tag", "release-0.16.0")       # not a release tag name
        r.commit({"bin/a.py": "A5", "bin/new.py": "N"}, tag="v0.15.1")

    # -- the window and the rows --
    def test_window_rows_and_predicate(self):
        self.history()
        rc, out = self.repo.gen()
        self.assertEqual(rc, 0, out)
        text = self.repo.table()
        self.assertRegex(text, r"(?m)^// WINDOW\s+:\s+3 resolvable release tags")
        self.assertIn("v0.14.0 .. v0.15.0", text)
        rows = rows_of(text)
        # v0.13.9 (below 0.14.0) and v0.15.1 (the tag on HEAD itself) are outside; pack-v0.14.1 is inside
        self.assertEqual(rows.get("bin/a.py"), [sha("A2"), sha("A3"), sha("A4")], text)
        self.assertNotIn("bin/same.py", rows, "unchanged across the window -> no row")
        self.assertIn("//   + bin/new.py", text, "new in this release -> listed as absent")
        for excluded in ("tests/t.py", ".hidden", "bin/__pycache__/x.pyc"):
            self.assertNotIn(excluded, rows, "build.rs predicate excludes %s" % excluded)
            self.assertNotIn(excluded, text)

    def test_window_is_widened_beyond_0_14(self):
        # design §4-11 다: the table must not stay frozen at 0.14.x — a 0.15.x release enters the window
        self.repo.commit({"bin/a.py": "A1"}, tag="v0.15.0")
        self.repo.commit({"bin/a.py": "A2"})
        rc, out = self.repo.gen()
        self.assertEqual(rc, 0, out)
        self.assertEqual(rows_of(self.repo.table()).get("bin/a.py"), [sha("A1")])

    # -- --check: fresh, then stale --
    def test_check_fresh_then_stale_after_a_pack_edit_then_fresh_again(self):
        self.history()
        self.assertEqual(self.repo.gen()[0], 0)
        rc, out = self.repo.gen("--check")
        self.assertEqual(rc, 0, out)
        self.assertIn("FRESH", out)
        # a file unchanged since the releases is edited: the old bytes must enter the table (0.14.56 incident shape)
        self.repo.commit({"bin/same.py": "S-edited"})
        rc, out = self.repo.gen("--check")
        self.assertEqual(rc, 1, out)
        self.assertIn("STALE", out)
        self.assertIn("bin/same.py", out, "the diff hint names the missing row")
        self.assertEqual(self.repo.gen()[0], 0)
        rc, out = self.repo.gen("--check")
        self.assertEqual(rc, 0, out)
        self.assertEqual(rows_of(self.repo.table()).get("bin/same.py"), [sha("S")])

    def test_check_stays_fresh_when_the_release_tag_is_pushed_on_head_then_stale_after_the_next_commit(self):
        self.repo.commit({"bin/a.py": "A1"}, tag="v0.14.0")
        head = self.repo.commit({"bin/a.py": "A2"})
        self.assertEqual(self.repo.gen()[0], 0)
        self.repo.git("tag", "v0.15.0", head)          # the release tag on the checked commit
        rc, out = self.repo.gen("--check")
        self.assertEqual(rc, 0, out)
        self.repo.commit({"docs/notes.md": "next"})     # next commit: v0.15.0 is now a previous release
        rc, out = self.repo.gen("--check")
        self.assertEqual(rc, 1, out)
        self.assertIn("STALE", out)

    def test_check_ignores_only_the_provenance_line(self):
        self.history()
        self.assertEqual(self.repo.gen()[0], 0)
        p = os.path.join(self.repo.root, OUT_REL)
        text = self.repo.table()
        with open(p, "w", encoding="utf-8") as f:
            f.write(re.sub(r"(?m)^// generated from : \S+$", "// generated from : 0000", text))
        rc, out = self.repo.gen("--check")
        self.assertEqual(rc, 0, out)
        with open(p, "w", encoding="utf-8") as f:
            f.write(text.replace(sha("A3"), sha("tampered"), 1))
        rc, out = self.repo.gen("--check")
        self.assertEqual(rc, 1, out)

    def test_check_with_no_committed_table_is_stale(self):
        self.history()
        rc, out = self.repo.gen("--check")
        self.assertEqual(rc, 1, out)

    # -- cannot see the window: refused, never fresh --
    def test_shallow_clone_is_refused(self):
        self.history()
        self.assertEqual(self.repo.gen()[0], 0)
        clone = os.path.join(self.tmp, "shallow")
        r = subprocess.run(["git", "clone", "-q", "--depth", "1", "file://" + self.repo.root, clone],
                           env=ENV, capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        os.makedirs(os.path.join(clone, "scripts"), exist_ok=True)
        os.makedirs(os.path.join(clone, "src"), exist_ok=True)
        shutil.copyfile(SCRIPT, os.path.join(clone, "scripts", "gen_prev_vendor_hashes.py"))
        shutil.copyfile(os.path.join(self.repo.root, OUT_REL), os.path.join(clone, OUT_REL))
        r = subprocess.run([sys.executable, os.path.join(clone, "scripts", "gen_prev_vendor_hashes.py"), "--check"],
                           cwd=clone, env=ENV, capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("shallow clone", r.stdout + r.stderr)

    def test_no_release_tag_is_refused(self):
        self.repo.commit({"bin/a.py": "A1"}, tag="v0.13.9")
        self.repo.commit({"bin/a.py": "A2"})
        rc, out = self.repo.gen("--check")
        self.assertEqual(rc, 2, out)
        self.assertIn("no release tag", out)

    def test_not_a_git_repository_is_refused(self):
        bare = os.path.join(self.tmp, "plain")
        os.makedirs(os.path.join(bare, "scripts"))
        shutil.copyfile(SCRIPT, os.path.join(bare, "scripts", "gen_prev_vendor_hashes.py"))
        env = dict(ENV, GIT_CEILING_DIRECTORIES=self.tmp)
        r = subprocess.run([sys.executable, os.path.join(bare, "scripts", "gen_prev_vendor_hashes.py"), "--check"],
                           cwd=bare, env=env, capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("REFUSED", r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
