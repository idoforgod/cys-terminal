#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_f58_corrections_manifest.py — F58 T-9: the correction manifest against the tree.

What it checks (DESIGN-F58-final.md §7.1, r2 §5.3 and §9.1 T-9):
  The manifest `cysjavis-pack/round/self-service-corrections.json` is the machine form of the
  F58 clause table (69 rows designed; T1-12d withdrawn by head decision, 68 in the manifest). For every row, on this tree:
    replace (new does not contain old): count(old) == 0 and count(new) == k_before, and the same
        on the string-continuation-normalized text: count(norm(old)) == 0, count(norm(new)) == k_norm.
        (T1-05: raw k_before 1, k_norm 2 — its test pin at state.rs spells the Rust `\\` continuation
        with other whitespace, so the two are equal only as the runtime string.)
    prefix (class 3: new = scope prefix + old): count(old) == k_before, count(new) == 1, and the one
        occurrence of old lies inside the occurrence of new.
    delete (new is empty): count(old) == 0 on the exact old substring.
  Generated rows (CEO_TEMPLATE.md mirrors) are judged on the committed CEO_TEMPLATE.md, which must
  equal header fragment + separator + MASTER_DIRECTIVE.md byte for byte (the drift half of
  gen_ceo_template.py --check). The owner-restored clause keeps 6,918 B and sha256 ef982427….
  Staleness: when the base commit is in this checkout, every row's file at the base has the recorded
  blob id and count(old) == k_before there. When it is not (a shallow checkout) the test prints
  UNMEASURED for that half; the tip predicates still run.
  The checker itself is shown to fail on an altered copy of the manifest (negative controls).

Read-only: repo tree only (no live pack, no HOME).
    python3 cysjavis-pack/bin/tests/test_f58_corrections_manifest.py
"""
import ast
import copy
import hashlib
import json
import os
import re
import subprocess
import sys
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
PACK = os.path.dirname(os.path.dirname(TESTS_DIR))
ROOT = os.path.dirname(PACK)
MANIFEST = os.path.join(PACK, "round", "self-service-corrections.json")
CEO = "cysjavis-pack/directives/CEO_TEMPLATE.md"
MASTER = "cysjavis-pack/directives/MASTER_DIRECTIVE.md"
HEADER = "scripts/ceo_template_header.md"
PREFIX_IDS = {"T3-01", "T3-01m", "T3-02", "T3-03", "T3-03m", "T3-04", "T3-04m", "T3-05",
              "T3-05m", "T3-06", "T3-07n", "T3-07nm"}
OWNER_CLAUSE_HEAD = "## [CEO 와 master 의 차이"
OWNER_CLAUSE_BYTES = 6918
OWNER_CLAUSE_SHA16 = "ef982427f25c5b2c"


def norm(s):
    """Rust string continuation: a backslash-newline and the spaces/tabs after it are removed."""
    return re.sub(r"\\\n[ \t]*", "", s)


def read(root, rel):
    with open(os.path.join(root, rel), "r", encoding="utf-8", newline="") as f:
        return f.read()


def load_manifest(path=MANIFEST):
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def check_rows(manifest, root):
    """Return a list of problem strings for the tip predicates of every row (empty = pass)."""
    problems = []
    cache = {}
    ncache = {}
    for r in manifest["rows"]:
        p = r["repo_path"]
        if p not in cache:
            try:
                cache[p] = read(root, p)
            except OSError as e:
                problems.append("%s: cannot read %s (%s)" % (r["id"], p, e))
                cache[p] = None
        t = cache[p]
        if t is None:
            continue
        if p not in ncache:
            ncache[p] = norm(t)
        nt = ncache[p]
        old, new, op = r["old"], r["new"], r["op"]
        c_old, c_new = t.count(old), (t.count(new) if new else 0)
        if op == "replace":
            n_old, n_new = nt.count(norm(old)), nt.count(norm(new))
            if c_old != 0 or c_new != r["k_before"] or n_old != 0 or n_new != r["k_norm"]:
                problems.append("%s replace: raw old %d (want 0) new %d (want %d); norm old %d (want 0) new %d (want %d)"
                                % (r["id"], c_old, c_new, r["k_before"], n_old, n_new, r["k_norm"]))
        elif op == "prefix":
            inside = False
            if c_new == 1:
                i = t.find(new)
                j = t.find(old)
                inside = i <= j and j + len(old) <= i + len(new)
            if c_old != r["k_before"] or c_new != 1 or not inside:
                problems.append("%s prefix: old %d (want %d) new %d (want 1) old inside new %s"
                                % (r["id"], c_old, r["k_before"], c_new, inside))
        elif op == "delete":
            if c_old != 0:
                problems.append("%s delete: old %d (want 0)" % (r["id"], c_old))
        else:
            problems.append("%s: unknown op %r" % (r["id"], op))
    return problems


def separator():
    src = read(ROOT, "scripts/gen_ceo_template.py")
    for n in ast.walk(ast.parse(src)):
        if isinstance(n, ast.Assign) and any(getattr(t, "id", "") == "SEPARATOR" for t in n.targets):
            v = n.value  # SEPARATOR = "<text>".encode("utf-8")
            if isinstance(v, ast.Call) and isinstance(v.func, ast.Attribute) and v.func.attr == "encode":
                v = v.func.value
            return ast.literal_eval(v)
    raise AssertionError("SEPARATOR not found in scripts/gen_ceo_template.py")


def owner_clause(text):
    i = text.find(OWNER_CLAUSE_HEAD)
    if i < 0:
        return None
    m = re.compile(r"(?m)^## \[").search(text, i + 1)
    return text[i:m.start() if m else len(text)].encode("utf-8")


def git(*args):
    return subprocess.run(["git", "-C", ROOT] + list(args), capture_output=True)


class ManifestShape(unittest.TestCase):
    def setUp(self):
        self.assertTrue(os.path.isfile(MANIFEST), "manifest missing: %s" % MANIFEST)
        self.m = load_manifest()

    def test_header_and_counts(self):
        m = self.m
        self.assertEqual(m["schema"], 1)
        self.assertEqual(m["release"], "0.14.57")
        self.assertEqual(m["base_commit"], "31f878782d966510089df8e71ddc8fb6cc289184")
        rows = m["rows"]
        self.assertEqual(len(rows), 68)  # 69 designed; T1-12d withdrawn by head decision
        self.assertEqual([w["id"] for w in m["withdrawn"]], ["T1-12d"])
        self.assertNotIn("T1-12d", {r["id"] for r in rows})
        ids = [r["id"] for r in rows]
        self.assertEqual(len(ids), len(set(ids)), "duplicate row ids")
        ops = {}
        for r in rows:
            ops[r["op"]] = ops.get(r["op"], 0) + 1
        self.assertEqual(ops, {"replace": 55, "prefix": 12, "delete": 1})
        self.assertEqual({r["id"] for r in rows if r["op"] == "prefix"}, PREFIX_IDS)
        self.assertEqual([r["id"] for r in rows if r["op"] == "delete"], ["T1-19f"])
        self.assertEqual(sorted(r["k_before"] for r in rows).count(2), 2)
        self.assertEqual({r["id"] for r in rows if r["k_before"] == 2}, {"T1-04", "T1-08"})
        self.assertEqual({r["id"] for r in rows if r["k_before"] != r["k_norm"]}, {"T1-05"})
        self.assertEqual(sum(1 for r in rows if r["generated"]), 9)
        for r in rows:
            if r["generated"]:
                self.assertEqual(r["repo_path"], CEO, r["id"])

    def test_owner_class_and_roles(self):
        rows = self.m["rows"]
        user = [r for r in rows if r["owner_class"] == "user"]
        self.assertEqual(len(user), 27)
        for r in rows:
            if r["owner_class"] == "user":
                self.assertTrue(r["roles"], r["id"])
                self.assertIsNotNone(r["file"], r["id"])
                self.assertEqual("cysjavis-pack/" + r["file"], r["repo_path"])
            else:
                self.assertEqual(r["roles"], [], r["id"])
        per = {role: sum(1 for r in rows if role in r["roles"]) for role in ("master", "worker", "cso", "reviewer")}
        self.assertEqual(per, {"master": 19, "worker": 16, "cso": 3, "reviewer": 2})


class ManifestAgainstTree(unittest.TestCase):
    def setUp(self):
        self.assertTrue(os.path.isfile(MANIFEST), "manifest missing: %s" % MANIFEST)
        self.m = load_manifest()

    def test_tip_predicates(self):
        problems = check_rows(self.m, ROOT)
        self.assertEqual(problems, [], "%d of %d rows fail:\n%s" % (len(problems), len(self.m["rows"]), "\n".join(problems)))

    def test_ceo_template_regenerated(self):
        want = (read(ROOT, HEADER) + separator() + read(ROOT, MASTER)).encode("utf-8")
        got = read(ROOT, CEO).encode("utf-8")
        self.assertEqual(hashlib.sha256(got).hexdigest(), hashlib.sha256(want).hexdigest(),
                         "CEO_TEMPLATE.md is not header + separator + MASTER (run scripts/gen_ceo_template.py)")

    def test_owner_clause_unchanged(self):
        blk = owner_clause(read(ROOT, CEO))
        self.assertIsNotNone(blk, "owner clause heading not found")
        self.assertEqual(len(blk), OWNER_CLAUSE_BYTES)
        self.assertEqual(hashlib.sha256(blk).hexdigest()[:16], OWNER_CLAUSE_SHA16)

    def test_rows_not_stale_at_base(self):
        base = self.m["base_commit"]
        if git("cat-file", "-e", base + "^{commit}").returncode != 0:
            print("\nT-9 staleness half UNMEASURED: base commit %s is not in this checkout" % base[:12])
            return
        cache = {}
        problems = []
        for r in self.m["rows"]:
            p = r["repo_path"]
            if p not in cache:
                blob = git("rev-parse", base + ":" + p).stdout.decode().strip()
                text = git("show", base + ":" + p).stdout.decode("utf-8")
                cache[p] = (blob, text)
            blob, text = cache[p]
            if blob != r["base_blob"]:
                problems.append("%s: base blob %s != recorded %s" % (r["id"], blob[:12], r["base_blob"][:12]))
            if text.count(r["old"]) != r["k_before"]:
                problems.append("%s: base count(old) %d != k_before %d" % (r["id"], text.count(r["old"]), r["k_before"]))
        self.assertEqual(problems, [], "\n".join(problems))


class CheckerNegativeControls(unittest.TestCase):
    """The checker must fail on altered manifests; otherwise a green tip proves nothing."""

    def setUp(self):
        self.m = load_manifest()
        self.base_problems = check_rows(self.m, ROOT)

    def _alter(self, rid, field, value):
        m = copy.deepcopy(self.m)
        for r in m["rows"]:
            if r["id"] == rid:
                r[field] = value(r[field]) if callable(value) else value
        return m

    def test_altered_new_text_fails(self):
        # one replace row's new text altered by one character (T-10 step 9 shape)
        m = self._alter("T1-01+T2-01", "new", lambda s: s[:-1] + ("x" if s[-1] != "x" else "y"))
        self.assertGreater(len(check_rows(m, ROOT)), len(self.base_problems))

    def test_wrong_k_before_fails(self):
        m = self._alter("T1-04", "k_before", 1)
        self.assertGreater(len(check_rows(m, ROOT)), len(self.base_problems))

    def test_prefix_and_delete_controls_fail(self):
        m = self._alter("T3-02", "new", lambda s: "X" + s)
        self.assertGreater(len(check_rows(m, ROOT)), len(self.base_problems))
        # a delete row whose old text IS still in the file must fail ("def " is in javis_learn.py)
        m = self._alter("T1-19f", "old", "def ")
        self.assertGreater(len(check_rows(m, ROOT)), len(self.base_problems))


if __name__ == "__main__":
    unittest.main(verbosity=2)
