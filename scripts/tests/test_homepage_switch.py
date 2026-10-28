"""homepage-switch.py hermetic tests (B-438, 0.14.53) - temp folders only, no network, no homepage server.

What is pinned here (BAR WP7-B438-0.14.53 r1):
  B2  the switch order (DESIGN s6), one journal record pair per move (fsynced), inode identity 16/16, an in-flight reader keeps
      the old bytes, and every kill point leaves a fail-safe public state (each name old-complete or new-complete; an index never
      flips before the files it names).
  B3  fail closed before any public change: G-dev, G-ts, G-live-old, G-pages, G-stale, G-disk, G-outside, G-versioned-new, the
      staging verify (SUMS specimens incl. "13 lines with an old name" and a duplicate), and a too-old interpreter.
  B4  publication uses only os.link + os.unlink and os.rename (AST scan of the final bytes, with positive controls).
  B5  SIGKILL after every primitive call -> resume ends NEW and rollback ends OLD; SIGHUP mid-loop completes; rollback never
      writes over a live file.
  F1  (codex r1) a planned file changed in place keeps its inode: a resume (forward or rollback), a repeated switch over a
      finished one, and the rollback's old-assets-back step stop before any further public move; each with its control.
The 42 tests of r1 only ever resumed over unchanged bytes, so inode identity alone passed them - see ContentRuleTests.
The fixture is the real 14-asset layout shrunk to a few KB per file. Every path is a tempfile path (no personal paths).

Use: python3 scripts/tests/test_homepage_switch.py
"""

import ast
import atexit
import contextlib
import errno
import hashlib
import importlib.util
import io
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import tokenize
import types
import unittest

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
HS_PATH = os.path.normpath(os.path.join(HERE, "..", "homepage-switch.py"))
DPC_PATH = os.path.normpath(os.path.join(HERE, "..", "download_page_check.py"))
# H-04-R: the switch loads download_page_check.py from beside itself (the release template uploads both). The tests run a
# BYTE-IDENTICAL copy of the switch beside a copy of the module whose PROFILES block holds fixture profiles (the fixture
# pages below are small stand-ins with the slot structure), written by write_page_module(). SourceTests reads HS_PATH.
RUN_DIR = tempfile.mkdtemp(prefix="hs-run-")
atexit.register(shutil.rmtree, RUN_DIR, True)
RUN_PATH = os.path.join(RUN_DIR, "homepage-switch.py")
shutil.copyfile(HS_PATH, RUN_PATH)
_spec = importlib.util.spec_from_file_location("homepage_switch", RUN_PATH)
hs = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(hs)

OLD, NEW, TS = "9.9.8", "9.9.9", "20991231-235959"
EXPECTED_ORDER = (
    ["downloads/cys_%s%s" % (NEW, s) for s in ("_aarch64.dmg", "_x64.dmg", "_x64-setup.exe", "_x64-setup.exe.sig",
                                               "_x64-setup.zip")]
    + ["downloads/" + n for n in ("cys_aarch64.app.tar.gz", "cys_aarch64.app.tar.gz.sig", "cys_x64.app.tar.gz",
                                  "cys_x64.app.tar.gz.sig", "pack.tar.gz", "pack-manifest.json.minisig",
                                  "pack-manifest.json", "latest.json", "SHA256SUMS.txt")]
    + ["downloads/index.html", "index.html"]
)
PRIMITIVE_CALLS = 5 * 2 + 11          # 5 link+unlink, 9 + 2 renames

RUNNER = r'''
import importlib.util, os, signal, sys
spec = importlib.util.spec_from_file_location("hs", sys.argv[1])
hs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hs)
mode, n, argv = sys.argv[2], int(sys.argv[3]), sys.argv[4:]
count = [0]
real = {"link": hs._do_link, "unlink": hs._do_unlink, "rename": hs._do_rename}
def wrap(kind):
    def f(*a):
        if mode == "race" and kind in ("link", "rename") and a[1].endswith(os.environ["RACE_DST"]):
            with open(a[1], "wb") as g:
                g.write(b"FOREIGN")
        real[kind](*a)
        count[0] += 1
        if count[0] == n:
            if mode == "kill":
                os.kill(os.getpid(), signal.SIGKILL)
            if mode == "hup":
                os.kill(os.getpid(), signal.SIGHUP)
    return f
hs._do_link, hs._do_unlink, hs._do_rename = wrap("link"), wrap("unlink"), wrap("rename")
sys.exit(hs.main(argv))
'''


def sha(b):
    return hashlib.sha256(b).hexdigest()


def rd(path):
    with open(path, "rb") as f:
        return f.read()


def fsha(path):
    return sha(rd(path))


def put(path, data):
    d = os.path.dirname(path)
    if not os.path.isdir(d):
        os.makedirs(d)
    with open(path, "wb") as f:
        f.write(data)


def asset_bytes(ver):
    """name -> bytes for one release, with real cross-bindings (SUMS, latest.json, pack-manifest digest)."""
    a = {}
    for n in hs.versioned_names(ver):
        a[n] = (("%s|%s|" % (n, ver)) * 300).encode()
    for n in ("cys_aarch64.app.tar.gz", "cys_x64.app.tar.gz", "pack.tar.gz"):
        a[n] = (("%s|%s|" % (n, ver)) * 400).encode()
    for n in ("cys_aarch64.app.tar.gz.sig", "cys_x64.app.tar.gz.sig", "pack-manifest.json.minisig"):
        a[n] = ("sig of %s %s" % (n, ver)).encode()
    a["latest.json"] = json.dumps({
        "version": ver, "notes": "릴리스 노트 — 한글 (non-ASCII on purpose: Python 3.6 under the C locale)",
        "platforms": {"darwin-aarch64": {"url": "https://example.invalid/releases/download/v%s/cys_aarch64.app.tar.gz" % ver,
                                         "signature": "s"},
                      "windows-x86_64": {"url": "https://example.invalid/releases/download/v%s/cys_%s_x64-setup.exe"
                                         % (ver, ver), "signature": "s"}}}, ensure_ascii=False).encode("utf-8")
    a["pack-manifest.json"] = json.dumps({"digest": sha(a["pack.tar.gz"]), "pack_version": ver}).encode()
    a["SHA256SUMS.txt"] = "".join("%s  %s\n" % (sha(a[n]), n) for n in sorted(a)).encode()
    return a


PAGE_SUFFIXES = ("_aarch64.dmg", "_x64.dmg", "_x64-setup.exe", "_x64-setup.zip")
BUNDLE = b"/* fixture bundle, pinned */\n"          # <docroot>/assets/main.js, the fixture index page's pinned JS


def pages(ver):
    """Stand-in pages with the slot structure of the real ones: index = S-slot + 3 cards (href + meta) + the JS branch
    literals (9 VER, 4 SIZE) + one pinned module script; downloads = 4 release links (4 VER) + the SUMS link."""
    md = u" \u00b7 "
    root = (u'<html><body><p class="dl-hero__sub">v%s%smacOS%sWindows</p>' % (ver, md, md)
            + u"".join(u'<a class="dl-btn" href="/downloads/cys_%s%s"><span class="dl-btn__meta">cys_%s%s%s0MB</span></a>'
                       % (ver, s, ver, s, md) for s in PAGE_SUFFIXES[:3])
            + u"<script>var h='/downloads/cys_%s_x64-setup.zip';var m='cys_%s_x64-setup.zip%s0MB';</script>" % (ver, ver, md)
            + u'<script type="module" src="./assets/main.js"></script></body></html>\n').encode("utf-8")
    dl = (u"<html><body>" + u"".join(u'<a href="/downloads/cys_%s%s">x</a>' % (ver, s) for s in PAGE_SUFFIXES)
          + u'<a href="/downloads/SHA256SUMS.txt">sums</a></body></html>\n').encode("utf-8")
    return {"index.html": root, "downloads/index.html": dl}


_dspec = importlib.util.spec_from_file_location("download_page_check_repo", DPC_PATH)
dpc = importlib.util.module_from_spec(_dspec)
_dspec.loader.exec_module(dpc)
_ZERO = dict((s, 0) for s in PAGE_SUFFIXES)
FX_IX = dpc.PROFILES["index.html"]._replace(skeleton_sha256="", js_pins={"./assets/main.js": sha(BUNDLE)}, required=())
FX_DL = dpc.PROFILES["downloads/index.html"]._replace(skeleton_sha256="", ver_slots=4, required=())
TPL_IX = dpc._skeleton_from(pages(OLD)["index.html"], OLD, _ZERO, FX_IX)
TPL_DL = dpc._skeleton_from(pages(OLD)["downloads/index.html"], OLD, {}, FX_DL)


def page_module_source(ix_pin=None, drop_downloads=False):
    """The repo module with ONLY its PROFILES block replaced by the fixture profiles."""
    src = rd(DPC_PATH).decode("utf-8")
    assert src.count("PROFILES = {") == 1
    a = src.index("PROFILES = {")
    b = src.index("\n}\n", a) + 3
    block = ("PROFILES = {   # TEST FIXTURE PROFILES (test_homepage_switch.page_module_source replaced this block only)\n"
             "    \"index.html\": Profile(\"index\", \"index.html\", \"index.skeleton\", %r, 9, SUFFIXES, %r,\n"
             "                          \"index-parser\", ()),\n" % (ix_pin or sha(TPL_IX), dict(FX_IX.js_pins)))
    if not drop_downloads:
        block += ("    \"downloads/index.html\": Profile(\"downloads\", \"downloads/index.html\", "
                  "\"downloads-index.skeleton\", %r, 4, (), {}, \"downloads-lint\", ()),\n" % sha(TPL_DL))
    return src[:a] + block + "}\n" + src[b:]


def write_page_module(text=None):
    with open(os.path.join(RUN_DIR, "download_page_check.py"), "w", encoding="utf-8") as f:
        f.write(page_module_source() if text is None else text)


write_page_module()


class Site(object):
    """One fixture: live OLD site, step-1 backup, NEW staging - exactly the state the template leaves before the switch."""

    def __init__(self, root, ts=TS):
        self.root = root
        self.ts = ts
        self.home = os.path.join(root, "home")
        self.docroot = os.path.join(self.home, "domains", "x", "public_html")
        self.backup = os.path.join(self.home, "cys-site-backup-" + ts)
        self.stage = os.path.join(self.home, "cys-site-staging-%s-%s" % (NEW, ts))
        self.rb = os.path.join(self.home, "cys-site-staging-rb-" + ts)
        self.trash = os.path.join(self.home, "cys-site-trash-" + ts)
        self.old_dir = os.path.join(self.home, "cys-site-backup-old-assets-" + ts)
        self.old, self.new = asset_bytes(OLD), asset_bytes(NEW)
        self.old_pages, self.new_pages = pages(OLD), pages(NEW)
        dl = os.path.join(self.docroot, "downloads")
        for n, b in self.old.items():
            put(os.path.join(dl, n.replace(NEW, OLD)), b)
        for rel, b in self.old_pages.items():
            put(os.path.join(self.docroot, rel), b)
        for rel, b in self.old_pages.items():
            put(os.path.join(self.backup, rel), b)
        for n in hs.VERSIONLESS_ORDER:
            shutil.copy2(os.path.join(dl, n), os.path.join(self.backup, "downloads", n))
        for n, b in self.new.items():
            put(os.path.join(self.stage, "downloads", n), b)
        put(os.path.join(self.stage, "pages", "index.html"), self.new_pages["index.html"])
        put(os.path.join(self.stage, "pages", "downloads-index.html"), self.new_pages["downloads/index.html"])
        put(os.path.join(self.stage, "pages", "index.skeleton"), TPL_IX)          # H-04-R: uploaded with the stage
        put(os.path.join(self.stage, "pages", "downloads-index.skeleton"), TPL_DL)
        put(os.path.join(self.docroot, "assets", "main.js"), BUNDLE)            # pinned JS the switch never moves
        self.write_expect()
        put(os.path.join(self.stage, "pages.json"), json.dumps(dict(
            (rel, {"pre": sha(self.old_pages[rel]), "want": sha(self.new_pages[rel])}) for rel in self.old_pages)).encode())

    def write_expect(self, ts=None):
        put(os.path.join(self.stage, "expect.json"), json.dumps({
            "ts": ts or self.ts, "new_ver": NEW, "old_ver": OLD, "sums_self": sha(self.new["SHA256SUMS.txt"]),
            "assets": [{"name": n, "size": len(b), "sha256": sha(b)} for n, b in sorted(self.new.items())]}).encode())

    def tree(self):
        out = {}
        for d, _, files in os.walk(self.docroot):
            for f in files:
                p = os.path.join(d, f)
                out[os.path.relpath(p, self.docroot)] = fsha(p)
        return out

    def old_tree(self):
        t = dict(("downloads/" + n.replace(NEW, OLD), sha(b)) for n, b in self.old.items())
        t.update((rel, sha(b)) for rel, b in self.old_pages.items())
        t["assets/main.js"] = sha(BUNDLE)
        return t

    def new_tree(self, old_moved=False):
        t = {} if old_moved else dict(("downloads/" + n, sha(self.old[n])) for n in hs.versioned_names(OLD))
        t.update(("downloads/" + n, sha(b)) for n, b in self.new.items())
        t.update((rel, sha(b)) for rel, b in self.new_pages.items())
        t["assets/main.js"] = sha(BUNDLE)
        return t

    def args(self, cmd):
        if cmd == "switch":
            return ["switch", "--stage", self.stage, "--docroot", self.docroot, "--ts", self.ts, "--backup", self.backup]
        if cmd == "rollback":
            return ["rollback", "--docroot", self.docroot, "--backup", self.backup, "--stage", self.stage, "--rb-stage",
                    self.rb, "--trash", self.trash, "--ts", self.ts, "--old-back", "%s=%s" % (OLD, self.old_dir)]
        if cmd == "preflight":
            return ["preflight", "--stage", self.stage, "--docroot", self.docroot, "--ts", self.ts]
        if cmd == "move-old":
            return ["move-old", "--docroot", self.docroot, "--journal", os.path.join(self.stage, "journal.jsonl"),
                    "--move", "%s=%s" % (OLD, self.old_dir)]
        raise ValueError(cmd)

    def journal(self, folder=None):
        return hs.Journal(os.path.join(folder or self.stage, "journal.jsonl")).records()


def run(args, extra=None):
    p = subprocess.run([sys.executable, "-B", RUN_PATH] + args, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return p.returncode, p.stdout.decode("utf-8", "replace") + p.stderr.decode("utf-8", "replace")


def run_wrapped(runner, mode, n, args, env=None):
    e = dict(os.environ)
    e.update(env or {})
    p = subprocess.run([sys.executable, "-B", runner, RUN_PATH, mode, str(n)] + args, stdout=subprocess.PIPE,
                       stderr=subprocess.PIPE, env=e)
    return p.returncode


def fail_safe_problems(site):
    """DESIGN s6: every public name is old-complete or new-complete, and an index never flips before what it names."""
    t = site.tree()
    bad = []
    old = dict(("downloads/" + n.replace(NEW, OLD), sha(b)) for n, b in site.old.items())
    old.update((rel, sha(b)) for rel, b in site.old_pages.items())
    new = dict(("downloads/" + n, sha(b)) for n, b in site.new.items())
    new.update((rel, sha(b)) for rel, b in site.new_pages.items())
    old["assets/main.js"] = new["assets/main.js"] = sha(BUNDLE)     # unchanged by the switch; changed bytes = FOREIGN
    state = {}
    for rel, h in t.items():
        if h == new.get(rel):
            state[rel] = "NEW"
        elif h == old.get(rel):
            state[rel] = "OLD"
        else:
            bad.append("FOREIGN %s" % rel)

    def is_new(n):
        return state.get("downloads/" + n) == "NEW"

    for tar in ("cys_aarch64.app.tar.gz", "cys_x64.app.tar.gz"):
        if is_new(tar + ".sig") and not is_new(tar):
            bad.append("%s.sig new before its tar" % tar)
    if is_new("latest.json") and not all(is_new(x) for x in ("cys_aarch64.app.tar.gz", "cys_x64.app.tar.gz",
                                                             "cys_aarch64.app.tar.gz.sig", "cys_x64.app.tar.gz.sig")):
        bad.append("latest.json new before the tarballs")
    if is_new("pack-manifest.json") and not (is_new("pack.tar.gz") and is_new("pack-manifest.json.minisig")):
        bad.append("pack-manifest new before pack.tar.gz/minisig")
    if is_new("SHA256SUMS.txt") and not all(is_new(n) for n in site.new if n != "SHA256SUMS.txt"):
        bad.append("SUMS new before every file it lists")
    for rel in ("downloads/index.html", "index.html"):
        if state.get(rel) == "NEW":
            if not is_new("SHA256SUMS.txt"):
                bad.append("%s new before SUMS" % rel)
        ver = NEW if state.get(rel) == "NEW" else OLD
        body = rd(os.path.join(site.docroot, rel)).decode()
        for s in hs.VERSIONED_SUFFIXES:
            if ("cys_%s%s" % (ver, s)) in body and ("downloads/cys_%s%s" % (ver, s)) not in t:
                bad.append("%s links absent cys_%s%s" % (rel, ver, s))
    return bad


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="hs-test-")
        self.site = Site(os.path.join(self.tmp, "s"))
        self.runner = os.path.join(self.tmp, "runner.py")
        put(self.runner, RUNNER.encode())

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def fresh(self, name):
        return Site(os.path.join(self.tmp, name))

    def assertUntouched(self, site, before):
        self.assertEqual(site.tree(), before, "public tree changed although the call stopped")
        self.assertFalse([r for r in site.journal() if r.get("ev") == "begin"], "a journal 'begin' was written")


class SumsRuleTests(unittest.TestCase):
    """DESIGN s5 specimens - the rule is set equality; the count is a consequence."""

    def setUp(self):
        self.a = asset_bytes(NEW)
        self.names = sorted(self.a)

    def lines(self, names):
        return "".join("%s  %s\n" % ("0" * 64, n) for n in names)

    def test_good_13(self):
        self.assertEqual(hs.sums_verdict(self.lines([n for n in self.names if n != "SHA256SUMS.txt"]), self.names), [])

    def test_14_lists_itself(self):
        self.assertTrue(hs.sums_verdict(self.lines(self.names), self.names))

    def test_12_missing_one(self):
        self.assertTrue(hs.sums_verdict(self.lines([n for n in self.names if n not in ("SHA256SUMS.txt", "latest.json")]),
                                        self.names))

    def test_13_with_an_old_name(self):
        rows = [n for n in self.names if n != "SHA256SUMS.txt"]
        rows[rows.index("cys_%s_x64.dmg" % NEW)] = "cys_%s_x64.dmg" % OLD
        self.assertEqual(len(rows), 13)
        bad = hs.sums_verdict(self.lines(rows), self.names)
        self.assertTrue(any("not in the asset set" in b for b in bad), bad)

    def test_13_with_a_duplicate(self):
        rows = [n for n in self.names if n not in ("SHA256SUMS.txt", "pack.tar.gz")] + ["latest.json"]
        self.assertEqual(len(rows), 13)
        bad = hs.sums_verdict(self.lines(rows), self.names)
        self.assertTrue(any("duplicate" in b for b in bad), bad)

    def test_bad_line_form(self):
        rows = self.lines([n for n in self.names if n != "SHA256SUMS.txt"]).replace("  latest.json", " *latest.json")
        self.assertTrue(hs.sums_verdict(rows, self.names))

    def test_self_hash_pinned(self):
        text = self.lines([n for n in self.names if n != "SHA256SUMS.txt"])
        self.assertTrue(hs.sums_verdict(text, self.names, sums_sha="a" * 64, sums_self="b" * 64))


class OrderAndForwardTests(Base):
    def test_order_table_is_design_s6(self):
        ex = hs.load_expect(os.path.join(self.site.stage, "expect.json"))
        self.assertEqual([it["dst"] for it in hs.forward_items(self.site.stage, ex)], EXPECTED_ORDER)

    def test_forward_switch_new_tree_journal_and_inodes(self):
        s = self.site
        staged_ino = {}
        for n in s.new:
            staged_ino["downloads/" + n] = os.lstat(os.path.join(s.stage, "downloads", n)).st_ino
        staged_ino["index.html"] = os.lstat(os.path.join(s.stage, "pages", "index.html")).st_ino
        staged_ino["downloads/index.html"] = os.lstat(os.path.join(s.stage, "pages", "downloads-index.html")).st_ino
        rc, out = run(s.args("switch"))
        self.assertEqual(rc, 0, out)
        self.assertEqual(s.tree(), s.new_tree())
        same = [rel for rel, ino in staged_ino.items() if os.lstat(os.path.join(s.docroot, rel)).st_ino == ino]
        self.assertEqual(len(same), 16, "inode identity %d/16" % len(same))
        j = s.journal()
        self.assertEqual([r["ev"] for r in j][0], "begin")
        self.assertEqual(j[-1]["ev"], "end")
        befores = [r["dst"] for r in j if r.get("ev") == "move" and r.get("phase") == "before"]
        afters = [r["dst"] for r in j if r.get("ev") == "move" and r.get("phase") == "after"]
        self.assertEqual(befores, EXPECTED_ORDER)
        self.assertEqual(afters, EXPECTED_ORDER)
        rc, out = run(s.args("switch"))
        self.assertEqual((rc, json.loads(out)["moved"]), (0, 0), "second call must be a no-op")
        self.assertEqual(fail_safe_problems(s), [])

    def test_journal_fsync_per_record(self):
        s = self.site
        real, calls = os.fsync, [0]

        def counting(fd):
            calls[0] += 1
            return real(fd)

        os.fsync = counting
        try:
            rc = hs.main(s.args("switch"))
        finally:
            os.fsync = real
        self.assertEqual(rc, 0)
        n = len(s.journal())
        self.assertEqual(n, 2 + 2 * 16)
        self.assertGreaterEqual(calls[0], n, "every journal record must be fsynced")

    def test_in_flight_reader_keeps_old_bytes(self):
        s = self.site
        p = os.path.join(s.docroot, "downloads", "cys_aarch64.app.tar.gz")
        f = open(p, "rb")
        try:
            head = f.read(7)
            rc, out = run(s.args("switch"))
            self.assertEqual(rc, 0, out)
            self.assertEqual(sha(head + f.read()), sha(s.old["cys_aarch64.app.tar.gz"]))
        finally:
            f.close()
        self.assertEqual(fsha(p), sha(s.new["cys_aarch64.app.tar.gz"]))

    def test_move_old_and_rollback_after_full_run(self):
        s = self.site
        self.assertEqual(run(s.args("switch"))[0], 0)
        rc, out = run(s.args("move-old"))
        self.assertEqual(rc, 0, out)
        self.assertEqual(s.tree(), s.new_tree(old_moved=True))
        self.assertEqual(run(s.args("move-old"))[0], 0, "move-old is idempotent")
        rc, out = run(s.args("rollback"))
        self.assertEqual(rc, 0, out)
        self.assertEqual(s.tree(), s.old_tree())
        out_dir = os.path.join(s.trash, "rolled-back-new-assets")
        self.assertEqual(sorted(os.listdir(out_dir)), sorted(hs.versioned_names(NEW)))
        self.assertEqual(run(s.args("rollback"))[0], 0, "rollback is idempotent")
        self.assertEqual(s.tree(), s.old_tree())

    def test_rollback_renames_from_copies_and_keeps_in_flight_reader(self):
        s = self.site
        self.assertEqual(run(s.args("switch"))[0], 0)
        live = os.path.join(s.docroot, "downloads", "cys_x64.app.tar.gz")
        ino_new = os.lstat(live).st_ino
        f = open(live, "rb")
        try:
            head = f.read(5)
            rc, out = run(s.args("rollback"))
            self.assertEqual(rc, 0, out)
            self.assertEqual(sha(head + f.read()), sha(s.new["cys_x64.app.tar.gz"]), "rollback wrote over a live file")
        finally:
            f.close()
        plan = json.loads(rd(os.path.join(s.rb, "plan.json")).decode())
        for it in plan["items"]:
            self.assertEqual(os.lstat(os.path.join(s.docroot, it["dst"])).st_ino, it["src_ino"], it["dst"])
        self.assertNotEqual(os.lstat(live).st_ino, ino_new)
        for n in hs.VERSIONLESS_ORDER:                       # the backup itself survives for a second attempt
            self.assertTrue(os.path.isfile(os.path.join(s.backup, "downloads", n)), n)


class GuardTests(Base):
    """B3: each guard stops before any public change (no journal 'begin', public tree byte-identical)."""

    def stop(self, args, site=None, want=3):
        site = site or self.site
        before = site.tree()
        rc, out = run(args)
        self.assertEqual(rc, want, out)
        self.assertUntouched(site, before)
        return out

    def test_g_dev(self):
        s = self.site
        real = hs._st_dev
        dl = os.path.realpath(os.path.join(s.docroot, "downloads"))
        hs._st_dev = lambda p: real(p) + (1 if os.path.realpath(p) == dl else 0)
        before = s.tree()
        try:
            self.assertEqual(hs.main(s.args("switch")), 3)
            self.assertEqual(hs.main(s.args("preflight")), 3)
        finally:
            hs._st_dev = real
        self.assertUntouched(s, before)

    def test_g_ts_expect_and_plan(self):
        s = self.site
        s.write_expect(ts="20990101-000000")
        self.stop(s.args("switch"))
        s.write_expect()
        put(os.path.join(s.stage, "plan.json"), json.dumps({"ts": "20990101-000000", "items": []}).encode())
        self.stop(s.args("switch"))

    def test_g_live_old(self):
        s = self.site
        put(os.path.join(s.docroot, "downloads", "pack.tar.gz"), b"someone changed the live file after the backup")
        self.stop(s.args("switch"))

    def test_g_pages(self):
        s = self.site
        put(os.path.join(s.docroot, "index.html"), b"<html>edited by hand meanwhile</html>")
        self.stop(s.args("switch"))
        s2 = self.fresh("pages-pre")
        pj = os.path.join(s2.stage, "pages.json")
        d = json.loads(rd(pj).decode())
        d["index.html"]["pre"] = "0" * 64           # the page was built from other bytes than the live ones
        put(pj, json.dumps(d).encode())
        self.assertIn("G-pages", self.stop(s2.args("switch"), site=s2))

    def test_g_versioned_new_exists(self):
        s = self.site
        p = os.path.join(s.docroot, "downloads", "cys_%s_x64.dmg" % NEW)
        put(p, b"a different x64.dmg already public")
        self.stop(s.args("switch"))
        self.assertEqual(rd(p), b"a different x64.dmg already public")

    def test_g_outside(self):
        s = self.site
        inside = os.path.join(s.docroot, "cys-site-staging-%s-%s" % (NEW, s.ts))
        shutil.copytree(s.stage, inside)
        for cmd in ("preflight", "switch"):
            args = [inside if x == s.stage else x for x in s.args(cmd)]
            self.assertEqual(run(args)[0], 3)

    def test_g_disk(self):
        s = self.site
        shutil.rmtree(os.path.join(s.stage, "downloads"))
        os.makedirs(os.path.join(s.stage, "downloads"))
        real = hs._free_bytes
        hs._free_bytes = lambda p: 10
        try:
            self.assertEqual(hs.main(s.args("preflight")), 3)
        finally:
            hs._free_bytes = real

    def test_g_stale_moves_not_deletes_not_reuses(self):
        s = self.site
        stale = os.path.join(s.home, "cys-site-staging-%s-20990101-000000" % NEW)
        put(os.path.join(stale, "downloads", "cys_%s_x64.dmg" % NEW), b"corrupt leftover")
        own_rb = os.path.join(s.home, "cys-site-staging-rb-" + s.ts)
        os.makedirs(own_rb)
        rc, out = run(s.args("preflight"))
        self.assertEqual(rc, 0, out)
        self.assertFalse(os.path.exists(stale))
        moved = os.path.join(s.trash, "stale-staging", os.path.basename(stale))
        self.assertEqual(rd(os.path.join(moved, "downloads", "cys_%s_x64.dmg" % NEW)), b"corrupt leftover")
        self.assertTrue(os.path.isdir(own_rb) and os.path.isdir(s.stage), "own staging folders must stay")
        self.assertEqual(json.loads(out)["stale_moved"], [os.path.basename(stale)])

    def test_g_stale_refuses_an_unfinished_switch_or_rollback(self):
        s = self.site
        for kind, label in (("cys-site-staging-%s-20990101-000000" % NEW, "switch"), ("cys-site-staging-rb-20990101-000001", "rollback")):
            other = os.path.join(s.home, kind)
            put(os.path.join(other, "plan.json"), json.dumps({"ts": "x", "items": []}).encode())
            hs.Journal(os.path.join(other, "journal.jsonl")).add(ev="begin", label=label, todo=[])
            done = os.path.join(s.home, "cys-site-staging-%s-20990102-000000" % NEW)       # a finished one may move
            put(os.path.join(done, "plan.json"), json.dumps({"ts": "y", "items": []}).encode())
            hs.Journal(os.path.join(done, "journal.jsonl")).add(ev="begin", label="switch", todo=[])
            hs.Journal(os.path.join(done, "journal.jsonl")).add(ev="end", label="switch", moved=16)
            rc, out = run(s.args("preflight"))
            self.assertEqual(rc, 3, out)
            self.assertIn("did not finish", out)
            self.assertTrue(os.path.isdir(other) and os.path.isdir(done), "nothing may move when one folder is unfinished")
            shutil.rmtree(other)
            rc, out = run(s.args("preflight"))
            self.assertEqual(rc, 0, out)
            self.assertFalse(os.path.exists(done), "a finished staging folder is moved to trash")

    def test_status_reports_plan_and_loop_time(self):
        s = self.site
        rc, out = run(["status", "--stage", s.stage, "--docroot", s.docroot])
        self.assertEqual((rc, json.loads(out)["plan"]), (0, False))
        self.assertEqual(run(s.args("switch"))[0], 0)
        self.assertEqual(run(s.args("move-old"))[0], 0)
        d = json.loads(run(["status", "--stage", s.stage, "--docroot", s.docroot])[1])
        self.assertTrue(d["plan"])
        j = s.journal()
        b = [r["t_ns"] for r in j if r.get("ev") == "begin"][-1]
        e = [r["t_ns"] for r in j if r.get("ev") == "end"][-1]
        self.assertEqual(d["loop_ns"], e - b, "loop time is the switch loop, not the later move-old records")

    def test_staging_verify_flipped_byte(self):
        s = self.site
        p = os.path.join(s.stage, "downloads", "cys_%s_aarch64.dmg" % NEW)
        b = bytearray(rd(p))
        b[100] ^= 1
        put(p, bytes(b))
        self.assertEqual(run(["verify-stage", "--stage", s.stage])[0], 3)
        self.stop(s.args("switch"))

    def test_staging_verify_short_and_extra(self):
        s = self.site
        p = os.path.join(s.stage, "downloads", "cys_x64.app.tar.gz")
        put(p, rd(p)[:-1])
        self.assertEqual(run(["verify-stage", "--stage", s.stage])[0], 3)
        s2 = self.fresh("extra")
        put(os.path.join(s2.stage, "downloads", "cys_%s_x64.dmg.part" % NEW), b"x")
        self.assertEqual(run(["verify-stage", "--stage", s2.stage])[0], 3)
        self.stop(s2.args("switch"), site=s2)

    def test_staging_verify_sums_with_old_name(self):
        s = self.site
        p = os.path.join(s.stage, "downloads", "SHA256SUMS.txt")
        text = rd(p).decode().replace("cys_%s_x64.dmg" % NEW, "cys_%s_x64.dmg" % OLD)
        put(p, text.encode())
        s.new["SHA256SUMS.txt"] = text.encode()
        s.write_expect()                     # even with the pin re-made, the name set must catch it
        rc, out = run(["verify-stage", "--stage", s.stage])
        self.assertEqual(rc, 3, out)
        self.assertIn("not in the asset set", out)

    def test_expect_with_another_asset_set_stops(self):
        s = self.site
        ex = json.loads(rd(os.path.join(s.stage, "expect.json")).decode())
        ex["assets"].append({"name": "cys_%s_extra.pkg" % NEW, "size": 1, "sha256": "0" * 64})
        put(os.path.join(s.stage, "expect.json"), json.dumps(ex).encode())
        rc, out = run(["verify-stage", "--stage", s.stage])
        self.assertEqual(rc, 3, out)
        self.assertIn("asset set", out)
        self.stop(s.args("switch"))

    def test_verify_stage_good(self):
        rc, out = run(["verify-stage", "--stage", self.site.stage])
        self.assertEqual(rc, 0, out)
        self.assertEqual(json.loads(out)["sums_lines"], 13)

    def test_verify_backup(self):
        s = self.site
        self.assertEqual(run(["verify-backup", "--backup", s.backup, "--docroot", s.docroot])[0], 0)
        put(os.path.join(s.backup, "downloads", "latest.json"), b"{}")
        self.assertEqual(run(["verify-backup", "--backup", s.backup, "--docroot", s.docroot])[0], 3)

    def test_need_lists_missing_and_short(self):
        s = self.site
        os.unlink(os.path.join(s.stage, "downloads", "pack.tar.gz"))
        p = os.path.join(s.stage, "downloads", "latest.json")
        put(p, rd(p)[:10])
        rc, out = run(["need", "--stage", s.stage])
        self.assertEqual((rc, sorted(out.split())), (0, ["latest.json", "pack.tar.gz"]))

    def test_unknown_public_state(self):
        s = self.site
        self.assertEqual(run(s.args("switch"))[0], 0)
        p = os.path.join(s.docroot, "downloads", "latest.json")
        os.unlink(p)
        put(p, b"replaced by someone else")
        rc, out = run(s.args("switch"))
        self.assertEqual(rc, 5, out)

    def test_too_old_interpreter_stops_with_3(self):
        code = ("import sys\nsys.version_info = (3, 5, 9, 'final', 0)\nsys.argv = ['hs', 'status', '--stage', '.']\n"
                "src = open(%r).read()\nexec(compile(src, %r, 'exec'), {'__name__': '__main__'})\n" % (HS_PATH, HS_PATH))
        p = subprocess.run([sys.executable, "-B", "-c", code], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.assertEqual(p.returncode, 3)
        self.assertIn(b">= 3.6 required", p.stderr)


class KillResumeRollbackTests(Base):
    """B5/M10/M11: SIGKILL after every primitive call; SIGHUP mid-loop; the race on a new name (M13)."""

    def test_sigkill_every_point_resume_new_rollback_old(self):
        for n in range(1, PRIMITIVE_CALLS + 1):
            a = self.fresh("k%da" % n)
            rc = run_wrapped(self.runner, "kill", n, a.args("switch"))
            self.assertEqual(rc, -signal.SIGKILL, "kill point %d" % n)
            self.assertEqual(fail_safe_problems(a), [], "prefix after %d calls is not fail-safe" % n)
            st = run(["status", "--stage", a.stage, "--docroot", a.docroot])
            self.assertEqual(st[0], 0, st[1])
            rc, out = run(a.args("switch"))
            self.assertEqual(rc, 0, "resume after %d: %s" % (n, out))
            self.assertEqual(a.tree(), a.new_tree(), "resume after %d" % n)
            b = self.fresh("k%db" % n)
            self.assertEqual(run_wrapped(self.runner, "kill", n, b.args("switch")), -signal.SIGKILL)
            rc, out = run(b.args("rollback"))
            self.assertEqual(rc, 0, "rollback after %d: %s" % (n, out))
            self.assertEqual(b.tree(), b.old_tree(), "rollback after %d" % n)

    def test_sigkill_during_rollback_then_rollback_again(self):
        for n in (1, 6, 11):
            s = self.fresh("rk%d" % n)
            self.assertEqual(run(s.args("switch"))[0], 0)
            self.assertEqual(run_wrapped(self.runner, "kill", n, s.args("rollback")), -signal.SIGKILL)
            rc, out = run(s.args("rollback"))
            self.assertEqual(rc, 0, out)
            self.assertEqual(s.tree(), s.old_tree(), "rollback re-run after kill %d" % n)

    def test_sighup_mid_loop_completes(self):
        s = self.site
        rc = run_wrapped(self.runner, "hup", 8, s.args("switch"))
        self.assertEqual(rc, 0)
        self.assertEqual(s.tree(), s.new_tree())

    def test_new_name_race_is_never_clobbered(self):
        s = self.site
        dst = "downloads/cys_%s_x64.dmg" % NEW
        rc = run_wrapped(self.runner, "race", 0, s.args("switch"), env={"RACE_DST": dst})
        self.assertEqual(rc, 4)
        self.assertEqual(rd(os.path.join(s.docroot, dst)), b"FOREIGN", "a new name was overwritten")
        self.assertTrue(os.path.isfile(os.path.join(s.stage, dst)), "the staged file must stay")


class PartialMoveFailureTests(Base):
    """R3-F1: a failed source unlink must not hide a destination name that the link already created."""

    def failed_move(self, engine, fault, prelinked=False, prior=0):
        s = self.fresh("partial-%s-%s-%s-%s" % (engine, fault, prelinked, prior))
        name = hs.versioned_names(NEW)[0]
        src = os.path.join(s.stage, "downloads", name)
        dst = os.path.join(s.docroot, "downloads", name)
        if prelinked:
            # A previous attempt stopped between link and unlink. The next call must see it.
            ex = hs.load_expect(os.path.join(s.stage, "expect.json"), s.ts)
            page_table = hs._load_json(os.path.join(s.stage, "pages.json"), "pages.json")
            items = hs.build_forward_plan(s.stage, s.docroot, s.backup, ex, page_table)
            hs._write_json_atomic(os.path.join(s.stage, "plan.json"), {"ts": s.ts, "items": items})
            os.link(src, dst)
        real_link, real_unlink, real_lstat = hs._do_link, hs._do_unlink, hs._lstat
        state = {"failed": False, "links": 0, "observed_after_error": []}

        def fail(message):
            state["failed"] = True
            raise OSError(errno.EACCES, message)

        def link(a, b):
            if fault == "before-link":
                fail("link refused before changing a name")
            real_link(a, b)
            state["links"] += 1
            if fault == "link-raises-after":
                fail("link created the name, then reported an error")

        def unlink(path):
            self.assertEqual(path, src, "fault must target the first source unlink")
            if fault == "vanished":
                real_unlink(dst)
            elif fault == "replaced":
                real_unlink(dst)
                put(dst, b"FOREIGN")
            fail("source unlink failed after link")

        def observe(path):
            if state["failed"] and path == dst:
                state["observed_after_error"].append(path)
                if fault == "unreadable":
                    raise OSError(errno.EIO, "destination observation unavailable")
                if fault == "other-device":
                    got = real_lstat(path)
                    return types.SimpleNamespace(st_mode=got.st_mode, st_ino=got.st_ino, st_dev=got.st_dev + 1)
            return real_lstat(path)

        hs._do_link, hs._do_unlink, hs._lstat = link, unlink, observe
        buf = io.StringIO()
        try:
            with contextlib.redirect_stdout(buf):
                if engine == "execute":
                    rc = hs.main(s.args("switch"))
                else:
                    try:
                        hs.move_files([(src, dst)], hs.Journal(os.path.join(s.stage, "pairs.jsonl")),
                                      "fixture-public-move", prior=prior)
                        rc = 0
                    except hs.Stop as exc:
                        rc = exc.code
        finally:
            hs._do_link, hs._do_unlink, hs._lstat = real_link, real_unlink, real_lstat
        self.assertTrue(os.path.isfile(src), "failed unlink must leave the source")
        self.assertEqual(state["links"], 0 if prelinked or fault == "before-link" else 1)
        return rc, dst, state, s

    def check_both(self, fault, want, prelinked=False):
        for engine in ("execute", "move_files"):
            with self.subTest(engine=engine):
                rc, dst, state, s = self.failed_move(engine, fault, prelinked)
                # Establish the filesystem premise BEFORE judging the exit code (including on the old tree).
                if fault in ("before-link", "vanished"):
                    self.assertFalse(os.path.lexists(dst))
                elif fault == "replaced":
                    self.assertEqual(rd(dst), b"FOREIGN", "foreign bytes must not be changed")
                else:
                    self.assertEqual(rd(dst), s.new[os.path.basename(dst)])
                self.assertEqual(rc, want, "%s/%s chose the wrong stop code" % (engine, fault))
                self.assertTrue(state["observed_after_error"], "must re-observe the destination after failure")

    def test_first_link_then_unlink_failure_is_partial(self):
        self.check_both("unlink", hs.STOP_PARTIAL)

    def test_link_error_after_name_creation_is_partial(self):
        self.check_both("link-raises-after", hs.STOP_PARTIAL)

    def test_successful_link_then_vanished_name_is_unknown(self):
        self.check_both("vanished", hs.STOP_UNKNOWN)

    def test_successful_link_then_foreign_name_is_unknown(self):
        self.check_both("replaced", hs.STOP_UNKNOWN)

    def test_unreadable_destination_is_unknown(self):
        self.check_both("unreadable", hs.STOP_UNKNOWN)

    def test_destination_on_unexpected_device_is_unknown(self):
        self.check_both("other-device", hs.STOP_UNKNOWN)

    def test_failure_before_link_is_clean(self):
        self.check_both("before-link", hs.STOP_CLEAN)

    def test_resume_pending_unlink_is_partial(self):
        self.check_both("unlink", hs.STOP_PARTIAL, prelinked=True)

    def test_move_files_keeps_prior_change_classification(self):
        rc, dst, state, _s = self.failed_move("move_files", "before-link", prior=1)
        self.assertEqual(rc, hs.STOP_PARTIAL)
        self.assertFalse(os.path.lexists(dst))
        self.assertTrue(state["observed_after_error"])


    def two_call_prelinked_failure(self, replace_destination):
        s = self.site
        name = hs.versioned_names(NEW)[0]
        src = os.path.join(s.stage, "downloads", name)
        dst = os.path.join(s.docroot, "downloads", name)
        journal = hs.Journal(os.path.join(s.stage, "two-call.jsonl"))
        real_link, real_unlink = hs._do_link, hs._do_unlink

        def first_unlink(path):
            self.assertEqual(path, src)
            raise OSError(errno.EACCES, "first source unlink fails")

        hs._do_unlink = first_unlink
        try:
            with self.assertRaises(hs.Stop) as first:
                hs.move_files([(src, dst)], journal, "two-call-public-move")
        finally:
            hs._do_unlink = real_unlink
        self.assertEqual(os.stat(src).st_ino, os.stat(dst).st_ino,
                         "call one must physically create the prelinked pair")
        self.assertEqual(rd(dst), s.new[name])
        self.assertEqual(first.exception.code, hs.STOP_PARTIAL)

        def no_second_link(*args):
            self.fail("the retry must consume the prelinked witness, not create a new link")

        def second_unlink(path):
            self.assertEqual(path, src)
            real_unlink(dst)
            if replace_destination:
                put(dst, b"FOREIGN")
            raise OSError(errno.EACCES, "retry source unlink fails after destination changes")

        hs._do_link, hs._do_unlink = no_second_link, second_unlink
        try:
            with self.assertRaises(hs.Stop) as retry:
                hs.move_files([(src, dst)], journal, "two-call-public-move")
        finally:
            hs._do_link, hs._do_unlink = real_link, real_unlink
        self.assertEqual(rd(src), s.new[name], "source must survive both failed unlinks")
        if replace_destination:
            self.assertEqual(rd(dst), b"FOREIGN")
        else:
            self.assertFalse(os.path.lexists(dst))
        self.assertEqual(retry.exception.code, hs.STOP_UNKNOWN,
                         "prelinked retry lost the acknowledged first-call name change")

    def test_two_call_prelinked_replaced_destination_is_not_clean(self):
        self.two_call_prelinked_failure(replace_destination=True)

    def test_two_call_prelinked_vanished_destination_is_not_clean(self):
        self.two_call_prelinked_failure(replace_destination=False)


class ContentRuleTests(Base):
    """F1 (codex r1): same-size one-byte corruption at offset 100 with the inode kept, after an interrupted forward run, an
    interrupted rollback, a finished switch, and in the old-assets folder. Each test runs its untouched control first: the
    same kill and the same call without the flip must still end rc 0."""

    def flip(self, path, at=100):
        ino = os.lstat(path).st_ino
        with open(path, "r+b") as f:
            f.seek(at)
            b = f.read(1)
            f.seek(at)
            f.write(bytes([b[0] ^ 1]))
        self.assertEqual(os.lstat(path).st_ino, ino, "the corruption must keep the inode - that is the F1 case")

    def begins(self, site, folder=None):
        return len([r for r in site.journal(folder) if r.get("ev") == "begin"])

    def states(self, site, folder, *extra):
        rc, out = run(["status", "--stage", folder, "--docroot", site.docroot] + list(extra))
        self.assertEqual(rc, 0, out)
        return json.loads(out)["states"]

    def test_forward_resume_refuses_a_staged_file_changed_in_place(self):
        for corrupt in (False, True):
            s = self.fresh("fwd-%d" % corrupt)
            self.assertEqual(run_wrapped(self.runner, "kill", 2, s.args("switch")), -signal.SIGKILL)
            p = os.path.join(s.stage, "downloads", "cys_x64.app.tar.gz")
            if corrupt:
                self.flip(p)
            before, begins = s.tree(), self.begins(s)
            rc, out = run(s.args("switch"))
            if not corrupt:
                self.assertEqual(rc, 0, "control: " + out)
                self.assertEqual(s.tree(), s.new_tree())
                continue
            self.assertEqual(rc, 5, out)             # one new installer is public already: not "before any public change"
            self.assertIn("downloads/cys_x64.app.tar.gz", out)
            self.assertEqual(s.tree(), before, "the resume made a public move")
            self.assertEqual(self.begins(s), begins, "the resume opened a move loop")
            self.assertEqual(fail_safe_problems(s), [])
            self.assertEqual(self.states(s, s.stage)["downloads/cys_x64.app.tar.gz"], "changed")
            rc, out = run(s.args("rollback"))        # the way back stays open
            self.assertEqual(rc, 0, out)
            self.assertEqual(s.tree(), s.old_tree())

    def test_rollback_resume_refuses_a_copy_changed_in_place(self):
        for corrupt in (False, True):
            s = self.fresh("rb-%d" % corrupt)
            self.assertEqual(run(s.args("switch"))[0], 0)
            self.assertEqual(run_wrapped(self.runner, "kill", 1, s.args("rollback")), -signal.SIGKILL)
            p = os.path.join(s.rb, "downloads", "cys_x64.app.tar.gz")
            if corrupt:
                self.flip(p)
            before, begins = s.tree(), self.begins(s, s.rb)
            rc, out = run(s.args("rollback"))
            if not corrupt:
                self.assertEqual(rc, 0, "control: " + out)
                self.assertEqual(s.tree(), s.old_tree())
                continue
            self.assertEqual(rc, 5, out)
            self.assertIn("downloads/cys_x64.app.tar.gz", out)
            self.assertEqual(s.tree(), before, "the rollback resume made a public move")
            self.assertEqual(self.begins(s, s.rb), begins, "the rollback resume opened a move loop")
            self.assertNotIn(fsha(p), s.tree().values(), "the corrupted copy is public")
            self.assertEqual(self.states(s, s.rb, "--allow-already")["downloads/cys_x64.app.tar.gz"], "changed")
            self.assertIn("at " + p, out, "the stop must say where the bytes were checked")
            # the way out the template prints (R1-1): set the rollback plan aside -> copies are rebuilt from the backup
            os.rename(os.path.join(s.rb, "plan.json"), os.path.join(s.rb, "plan.json.changed-copy"))
            rc, out = run(s.args("rollback"))
            self.assertEqual(rc, 0, out)
            self.assertEqual(s.tree(), s.old_tree())

    def test_link_without_unlink_and_changed_bytes_is_not_finished(self):
        for corrupt in (False, True):
            s = self.fresh("pending-%d" % corrupt)
            self.assertEqual(run_wrapped(self.runner, "kill", 1, s.args("switch")), -signal.SIGKILL)   # linked, not unlinked
            name = "cys_%s_aarch64.dmg" % NEW
            p = os.path.join(s.stage, "downloads", name)
            self.assertEqual(os.lstat(p).st_ino, os.lstat(os.path.join(s.docroot, "downloads", name)).st_ino)
            if corrupt:
                self.flip(p)                          # the same inode is the public name too
            before, begins = s.tree(), self.begins(s)
            rc, out = run(s.args("switch"))
            if not corrupt:
                self.assertEqual(rc, 0, "control: " + out)
                self.assertEqual(s.tree(), s.new_tree())
                continue
            self.assertEqual(rc, 5, out)
            self.assertEqual(s.tree(), before, "the resume made a public move")
            self.assertEqual(self.begins(s), begins, "the resume opened a move loop")
            self.assertTrue(os.path.exists(p), "the staged name was unlinked")
            self.assertEqual(self.states(s, s.stage)["downloads/" + name], "changed")

    def test_finished_switch_with_a_public_file_changed_in_place_is_not_done(self):
        for corrupt in (False, True):
            s = self.fresh("done-%d" % corrupt)
            self.assertEqual(run(s.args("switch"))[0], 0)
            p = os.path.join(s.docroot, "downloads", "cys_x64.app.tar.gz")
            if corrupt:
                self.flip(p)
            before = s.tree()
            rc, out = run(s.args("switch"))
            if not corrupt:
                self.assertEqual((rc, json.loads(out)["moved"]), (0, 0), "control: " + out)
                continue
            self.assertEqual(rc, 5, out)
            self.assertEqual(s.tree(), before)
            self.assertEqual(self.states(s, s.stage)["downloads/cys_x64.app.tar.gz"], "changed")

    def test_a_planned_file_on_another_filesystem_is_not_published(self):
        s = self.site
        target = os.path.join(s.stage, "downloads", "pack.tar.gz")
        real = hs._lstat

        def other_dev(p):
            st = real(p)
            if st is not None and p == target:
                v = list(st)
                v[2] += 1                                # st_dev; the inode number stays the planned one
                return os.stat_result(v)
            return st

        before = s.tree()
        hs._lstat = other_dev
        try:
            rc = hs.main(s.args("switch"))
        finally:
            hs._lstat = real
        self.assertEqual(rc, 3)
        self.assertUntouched(s, before)
        self.assertEqual(hs.main(s.args("switch")), 0, "control: the same call on the real filesystem")
        self.assertEqual(s.tree(), s.new_tree())

    def test_rollback_old_back_refuses_an_old_asset_changed_in_place(self):
        for corrupt in (False, True):
            s = self.fresh("oldback-%d" % corrupt)
            self.assertEqual(run(s.args("switch"))[0], 0)
            self.assertEqual(run(s.args("move-old"))[0], 0)
            p = os.path.join(s.old_dir, "cys_%s_x64.dmg" % OLD)
            if corrupt:
                self.flip(p)
            before = s.tree()
            rc, out = run(s.args("rollback"))
            if not corrupt:
                self.assertEqual(rc, 0, "control: " + out)
                self.assertEqual(s.tree(), s.old_tree())
                continue
            self.assertEqual(rc, 3, out)
            self.assertIn("cys_%s_x64.dmg" % OLD, out)
            self.assertEqual(s.tree(), before, "rollback moved something although an old asset was changed")
            self.assertTrue(os.path.isfile(p))


class RollbackPreflightTests(Base):
    """R3-FIX (codex r2 R2-1, R2-2): every check of a rollback (and of move-old) runs before its first public move, on every
    retry path. Each test runs its untouched control first, then the fault; the fault call must make zero publication calls
    (_do_link / _do_unlink / _do_rename, counted in-process) and leave the public tree byte-identical."""

    flip = ContentRuleTests.flip

    def counted(self, args):
        calls, real = [], (hs._do_link, hs._do_unlink, hs._do_rename)

        def wrap(f):
            def g(*a):
                calls.append(a)
                return f(*a)
            return g

        hs._do_link, hs._do_unlink, hs._do_rename = (wrap(f) for f in real)
        buf = io.StringIO()
        try:
            with contextlib.redirect_stdout(buf):
                rc = hs.main(args)
        finally:
            hs._do_link, hs._do_unlink, hs._do_rename = real
        return rc, len(calls), buf.getvalue()

    def switched_and_moved_old(self, name):
        s = self.fresh(name)
        self.assertEqual(run(s.args("switch"))[0], 0)
        self.assertEqual(run(s.args("move-old"))[0], 0)
        return s

    def test_old_installer_put_back_then_changed_is_checked_on_retry(self):
        # kill 2 = the first old installer linked AND its source unlinked (codex's case); kill 1 = linked, not yet unlinked
        for kill in (2, 1):
            for corrupt in (False, True):
                s = self.switched_and_moved_old("oldback-retry-%d-%d" % (kill, corrupt))
                self.assertEqual(run_wrapped(self.runner, "kill", kill, s.args("rollback")), -signal.SIGKILL)
                p = os.path.join(s.docroot, "downloads", "cys_%s_aarch64.dmg" % OLD)
                self.assertEqual(os.path.exists(os.path.join(s.old_dir, os.path.basename(p))), kill == 1)
                if corrupt:
                    self.flip(p)
                before = s.tree()
                rc, n, out = self.counted(s.args("rollback"))
                if not corrupt:
                    self.assertEqual(rc, 0, "control kill %d: %s" % (kill, out))
                    self.assertEqual(s.tree(), s.old_tree())
                    continue
                self.assertEqual(rc, 5, "kill %d: %s" % (kill, out))
                self.assertEqual(n, 0, "kill %d: publication calls before the check" % kill)
                self.assertEqual(s.tree(), before)
                self.assertIn("old installers do not match the old SUMS", out)
                self.assertIn("at " + p, out)

    def test_stale_rollback_plan_ts_stops_before_old_installers_move(self):
        for bad in (False, True):
            s = self.switched_and_moved_old("rb-ts-%d" % bad)
            if bad:
                put(os.path.join(s.rb, "plan.json"), json.dumps({"ts": "20990101-000000", "items": []}).encode())
            before = s.tree()
            rc, n, out = self.counted(s.args("rollback"))
            if not bad:
                self.assertEqual(rc, 0, "control: " + out)
                self.assertEqual(s.tree(), s.old_tree())
                continue
            self.assertEqual((rc, n), (3, 0), out)
            self.assertIn("G-ts", out)
            self.assertEqual(s.tree(), before)
            self.assertEqual(len(os.listdir(s.old_dir)), 5, "an old installer left the old-assets folder")

    def test_changed_versionless_backup_stops_before_old_installers_move(self):
        for bad in (False, True):
            s = self.switched_and_moved_old("rb-bk-%d" % bad)
            if bad:
                self.flip(os.path.join(s.backup, "downloads", "latest.json"))
            before = s.tree()
            rc, n, out = self.counted(s.args("rollback"))
            if not bad:
                self.assertEqual(rc, 0, "control: " + out)
                self.assertEqual(s.tree(), s.old_tree())
                continue
            self.assertEqual((rc, n), (3, 0), out)
            self.assertIn("old SUMS line", out)
            self.assertEqual(s.tree(), before)

    def test_unknown_public_file_stops_before_old_installers_move(self):
        for bad in (False, True):
            s = self.switched_and_moved_old("rb-unknown-%d" % bad)
            p = os.path.join(s.docroot, "downloads", "latest.json")
            if bad:
                os.unlink(p)
                put(p, b"replaced by someone else")
            before = s.tree()
            rc, n, out = self.counted(s.args("rollback"))
            if not bad:
                self.assertEqual(rc, 0, "control: " + out)
                self.assertEqual(s.tree(), s.old_tree())
                continue
            self.assertEqual((rc, n), (3, 0), out)
            self.assertIn("public state not recognised", out)
            self.assertEqual(s.tree(), before)

    def test_new_names_way_out_is_checked_before_any_move(self):
        for bad in (False, True):
            s = self.switched_and_moved_old("rb-out-%d" % bad)
            if bad:
                put(os.path.join(s.trash, "rolled-back-new-assets", "cys_%s_x64.dmg" % NEW), b"other bytes already here")
            before = s.tree()
            rc, n, out = self.counted(s.args("rollback"))
            if not bad:
                self.assertEqual(rc, 0, "control: " + out)
                self.assertEqual(s.tree(), s.old_tree())
                continue
            self.assertEqual((rc, n), (3, 0), out)
            self.assertIn("already exists with other bytes", out)
            self.assertEqual(s.tree(), before)

    def test_move_old_checks_every_pair_before_the_first_move(self):
        for bad in (False, True):
            s = self.fresh("mvold-pre-%d" % bad)
            self.assertEqual(run(s.args("switch"))[0], 0)
            if bad:                                  # the 2nd pair in name order conflicts; the 1st would move first
                put(os.path.join(s.old_dir, "cys_%s_x64-setup.exe" % OLD), b"other bytes already here")
            before = s.tree()
            rc, n, out = self.counted(s.args("move-old"))
            if not bad:
                self.assertEqual(rc, 0, "control: " + out)
                self.assertEqual(s.tree(), s.new_tree(old_moved=True))
                continue
            self.assertEqual((rc, n), (3, 0), out)
            self.assertEqual(s.tree(), before)


class PreflightStaleCheckFirstTests(Base):
    """R3-FIX census follow-up: G-stale checks every stale staging folder before it moves the first one to trash, and never
    moves a folder that holds the docroot."""

    def test_every_stale_folder_is_checked_before_the_first_move(self):
        for bad in (False, True):
            s = self.fresh("stale-pre-%d" % bad)
            first = os.path.join(s.home, "cys-site-staging-%s-20990101-000000" % NEW)    # sorted first
            second = os.path.join(s.home, "cys-site-staging-%s-20990102-000000" % NEW)
            for d in (first, second):
                put(os.path.join(d, "downloads", "x"), b"leftover")
            real = hs._st_dev
            if bad:                                  # the SECOND folder fails G-dev
                hs._st_dev = lambda p: real(p) + (1 if os.path.realpath(p) == os.path.realpath(second) else 0)
            try:
                rc = hs.main(s.args("preflight"))
            finally:
                hs._st_dev = real
            if not bad:
                self.assertEqual(rc, 0)
                self.assertFalse(os.path.exists(first) or os.path.exists(second), "control: both moved to trash")
                continue
            self.assertEqual(rc, 3)
            self.assertTrue(os.path.isdir(first) and os.path.isdir(second), "a stale folder moved before every check")

    def test_a_stale_folder_holding_the_docroot_is_never_moved(self):
        s = self.site
        holder = os.path.join(s.home, "cys-site-staging-%s-20990101-000000" % NEW)
        inside = os.path.join(holder, "public_html")
        shutil.copytree(s.docroot, inside)
        args = [inside if x == s.docroot else x for x in s.args("preflight")]
        rc, out = run(args)
        self.assertEqual(rc, 3, out)
        self.assertIn("G-outside", out)
        self.assertTrue(os.path.isdir(inside), "the folder holding the docroot was moved")
        self.assertEqual(run(s.args("preflight"))[0], 0, "control: the same preflight with the real docroot")


class FolderOutsideDocrootTests(Base):
    """F3 (codex r1), helper side: the folders rollback and move-old write into are checked against the docroot before the
    first mkdir. The template checks the whole remote layout read-only before its first write; these are the second line."""

    def refused(self, site, args, made):
        before = site.tree()
        rc, out = run(args)
        self.assertEqual(rc, 3, out)
        self.assertIn("G-outside", out)
        self.assertEqual(site.tree(), before, "public tree changed")
        if made:
            self.assertFalse(os.path.exists(made), "%s was created inside the docroot" % made)

    def test_rollback_trash_inside_the_docroot(self):
        s = self.site
        self.assertEqual(run(s.args("switch"))[0], 0)
        inside = os.path.join(s.docroot, "cys-site-trash-" + s.ts)
        self.refused(s, [inside if x == s.trash else x for x in s.args("rollback")], inside)
        self.assertEqual(run(s.args("rollback"))[0], 0, "control: the same rollback with the trash outside")

    def test_rollback_backup_inside_the_docroot(self):
        s = self.site
        self.assertEqual(run(s.args("switch"))[0], 0)
        inside = os.path.join(s.docroot, "cys-site-backup-" + s.ts)
        shutil.copytree(s.backup, inside)
        self.refused(s, [inside if x == s.backup else x for x in s.args("rollback")], None)
        self.assertEqual(run(s.args("rollback"))[0], 0, "control: the same rollback with the backup outside")

    def test_rollback_old_assets_folder_inside_the_docroot(self):
        s = self.site
        self.assertEqual(run(s.args("switch"))[0], 0)
        self.assertEqual(run(s.args("move-old"))[0], 0)
        inside = os.path.join(s.docroot, "cys-site-backup-old-assets-" + s.ts)
        os.rename(s.old_dir, inside)
        spec = "%s=%s" % (OLD, s.old_dir)
        self.refused(s, ["%s=%s" % (OLD, inside) if x == spec else x for x in s.args("rollback")], None)
        os.rename(inside, s.old_dir)
        self.assertEqual(run(s.args("rollback"))[0], 0, "control: the same rollback with the folder outside")
        self.assertEqual(s.tree(), s.old_tree())

    def test_move_old_destination_inside_the_docroot(self):
        s = self.site
        self.assertEqual(run(s.args("switch"))[0], 0)
        inside = os.path.join(s.docroot, "cys-site-backup-old-assets-" + s.ts)
        spec = "%s=%s" % (OLD, s.old_dir)
        self.refused(s, ["%s=%s" % (OLD, inside) if x == spec else x for x in s.args("move-old")], inside)
        self.assertEqual(run(s.args("move-old"))[0], 0, "control: the same move with the folder outside")
        self.assertEqual(s.tree(), s.new_tree(old_moved=True))


class PageCheckTests(Base):
    """H-04-R D3 (design r3 s5.2/s6): both pages against their profiles; pins read only from the uploaded module."""

    def rehash_page(self, s, rel, flat, data):
        put(os.path.join(s.stage, "pages", flat), data)
        pj = json.loads(rd(os.path.join(s.stage, "pages.json")))
        pj[rel]["want"] = fsha(os.path.join(s.stage, "pages", flat))
        put(os.path.join(s.stage, "pages.json"), json.dumps(pj).encode())

    def tearDown(self):
        write_page_module()                         # every test that swaps the module copy gets the fixture one back
        super(PageCheckTests, self).tearDown()

    def test_first_call_refuses_a_changed_staged_page_of_either_profile(self):
        for rel, flat in (("index.html", "index.html"), ("downloads/index.html", "downloads-index.html")):
            s = Site(os.path.join(self.tmp, "bad-" + flat))
            self.rehash_page(s, rel, flat, rd(os.path.join(s.stage, "pages", flat)).replace(b"</body>", b"<i>x</i></body>"))
            before = s.tree()
            rc, out = run(s.args("switch"))
            self.assertEqual((rc, s.tree() == before, os.path.exists(os.path.join(s.stage, "plan.json"))), (3, True, False),
                             out)
            self.assertIn("%s PAGE-DIFF" % rel, out)

    def test_first_call_refuses_a_changed_pinned_bundle(self):
        s = Site(os.path.join(self.tmp, "js"))
        put(os.path.join(s.docroot, "assets", "main.js"), b"/* changed */")
        before = s.tree()
        rc, out = run(s.args("switch"))
        self.assertEqual((rc, s.tree() == before), (3, True), out)
        self.assertIn("SCRIPT-SHA", out)

    def test_verify_stage_checks_both_pages_bytes(self):
        s = Site(os.path.join(self.tmp, "vs"))
        self.assertEqual(run(["verify-stage", "--stage", s.stage])[0], 0)
        for flat in ("index.html", "downloads-index.html"):
            s = Site(os.path.join(self.tmp, "vs-" + flat))
            put(os.path.join(s.stage, "pages", flat), b"<html>other</html>\n")
            rc, out = run(["verify-stage", "--stage", s.stage])
            self.assertEqual(rc, 3, out)

    def test_resume_after_index_published_finishes(self):
        s = Site(os.path.join(self.tmp, "done"))
        runner = os.path.join(self.tmp, "runner.py")
        put(runner, RUNNER.encode())
        run_wrapped(runner, "kill", PRIMITIVE_CALLS, s.args("switch"))
        rc, out = run(s.args("switch"))
        self.assertEqual((rc, json.loads(out)["moved"]), (0, 0), out)

    def test_resume_with_public_moves_and_changed_bundle_is_4(self):
        s = Site(os.path.join(self.tmp, "pj"))
        runner = os.path.join(self.tmp, "runner.py")
        put(runner, RUNNER.encode())
        run_wrapped(runner, "kill", 10, s.args("switch"))
        put(os.path.join(s.docroot, "assets", "main.js"), b"/* changed */")
        before = s.tree()
        rc, out = run(s.args("switch"))
        self.assertEqual((rc, s.tree() == before), (4, True), out)
        self.assertIn("this call moved nothing", out)

    def test_resume_with_public_moves_and_a_staged_page_changed_is_5(self):
        for flat in ("index.html", "downloads-index.html"):
            s = Site(os.path.join(self.tmp, "pt-" + flat))
            runner = os.path.join(self.tmp, "runner.py")
            put(runner, RUNNER.encode())
            run_wrapped(runner, "kill", 10, s.args("switch"))
            with open(os.path.join(s.stage, "pages", flat), "ab") as f:
                f.write(b"<!-- x -->")
            before = s.tree()
            rc, out = run(s.args("switch"))
            self.assertEqual((rc, s.tree() == before), (5, True), out)

    def test_resume_plan_written_no_move_changed_bundle_is_3(self):
        s = Site(os.path.join(self.tmp, "po"))
        ex = hs.load_expect(os.path.join(s.stage, "expect.json"), s.ts)
        items = hs.build_forward_plan(s.stage, s.docroot, s.backup, ex, json.loads(rd(os.path.join(s.stage, "pages.json"))))
        hs._write_json_atomic(os.path.join(s.stage, "plan.json"), {"ts": s.ts, "items": items})
        put(os.path.join(s.docroot, "assets", "main.js"), b"/* changed */")
        before = s.tree()
        rc, out = run(s.args("switch"))
        self.assertEqual((rc, s.tree() == before), (3, True), out)

    def test_an_unusable_module_stops_preflight_before_any_move_and_switch_before_the_plan(self):
        s = Site(os.path.join(self.tmp, "pf"))
        self.assertEqual(run(s.args("preflight"))[0], 0)
        write_page_module(page_module_source() + "\ndef broken(:\n")      # stands in for a 3.6 parse failure
        s = Site(os.path.join(self.tmp, "pf-bad"))
        stale = os.path.join(s.home, "cys-site-staging-%s-20990101-000000" % NEW)
        put(os.path.join(stale, "downloads", "x"), b"leftover")
        home_before = sorted(os.listdir(s.home))
        rc, out = run(s.args("preflight"))
        self.assertEqual((rc, os.path.isdir(stale), sorted(os.listdir(s.home)) == home_before), (3, True, True), out)
        self.assertIn("page-check module unusable", out)
        s2 = Site(os.path.join(self.tmp, "sw-bad"))
        before = s2.tree()
        rc, out = run(s2.args("switch"))
        self.assertEqual((rc, s2.tree() == before, os.path.exists(os.path.join(s2.stage, "plan.json"))), (3, True, False),
                         out)

    def test_the_switch_follows_the_module_pin(self):
        write_page_module(page_module_source(ix_pin="0" * 64))
        s = Site(os.path.join(self.tmp, "pin"))
        before = s.tree()
        rc, out = run(s.args("switch"))
        self.assertEqual((rc, s.tree() == before), (3, True), out)
        self.assertIn("SKELETON-PIN", out)

    def test_profiles_must_cover_pages_order(self):
        write_page_module(page_module_source(drop_downloads=True))
        s = Site(os.path.join(self.tmp, "np"))
        before = s.tree()
        rc, out = run(s.args("switch"))
        self.assertEqual((rc, s.tree() == before), (3, True), out)
        self.assertIn("PAGES_ORDER", out)


class AssetSetCouplingTests(unittest.TestCase):
    """R2: the asset set lives in three places - this file's tables, the homepage template's arrays (checked against this
    file at run time: load_expect refuses any other set) and verify-release-remote.py (the public verify, step 6). This pins
    the third copy so that a release adding or renaming an asset cannot change one list and not the other."""

    def test_same_set_as_the_public_verifier(self):
        path = os.path.normpath(os.path.join(HERE, "..", "verify-release-remote.py"))
        spec = importlib.util.spec_from_file_location("verify_release_remote", path)
        vrr = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(vrr)                  # module level is constants and functions only (no network)
        for ver in ("0.14.53", "1.2.3"):
            self.assertEqual(set(hs.asset_names(ver)) - {hs.SUMS_NAME}, vrr.expected_sums_names(ver), ver)
        self.assertEqual(len(hs.asset_names("0.14.53")), 14)

    def test_order_table_covers_exactly_the_asset_set(self):
        ex = {"new_ver": NEW, "assets": [{"name": n} for n in hs.asset_names(NEW)]}
        moved = [it["dst"] for it in hs.forward_items("/s", ex) if it["dst"].startswith("downloads/")
                 and it["dst"] != "downloads/index.html"]
        self.assertEqual(sorted(os.path.basename(m) for m in moved), sorted(hs.asset_names(NEW)))


class SourceTests(unittest.TestCase):
    """B4 (publication primitives) and B3 (Python 3.6 floor) over the final bytes, each with a positive control."""

    SRC = rd(HS_PATH).decode("utf-8")
    ALLOWED_DIRECT = {"_do_link", "_do_unlink", "_do_rename", "_write_json_atomic", "_copy_into_stage"}

    @staticmethod
    def publication_problems(src):
        tree = ast.parse(src)
        bad = []
        for node in ast.walk(tree):
            if isinstance(node, (ast.Import, ast.ImportFrom)):
                mods = [a.name for a in node.names] if isinstance(node, ast.Import) else [node.module or ""]
                for m in mods:
                    if m.split(".")[0] in ("shutil", "subprocess", "pty", "distutils"):
                        bad.append("imports %s" % m)
        for fn in [n for n in ast.walk(tree) if isinstance(n, ast.FunctionDef)]:
            for node in ast.walk(fn):
                if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and \
                        isinstance(node.func.value, ast.Name) and node.func.value.id == "os":
                    if node.func.attr in ("system", "popen", "spawnv", "spawnl", "execv", "execvp", "replace"):
                        bad.append("os.%s in %s" % (node.func.attr, fn.name))
                    if node.func.attr in ("link", "unlink", "rename", "remove") and fn.name not in SourceTests.ALLOWED_DIRECT:
                        bad.append("direct os.%s in %s" % (node.func.attr, fn.name))
        for tok in ("'mv'", '"mv"', "'cp'", '"cp"', "copyfile", "copy2", "copytree"):
            if tok in src:
                bad.append("token %s" % tok)
        return bad

    @staticmethod
    def floor_problems(src):
        """Python 3.6 floor: ast feature_version is best-effort (3.9 accepts := under (3,6); 3.13 accepts f-strings under
        (3,5) - measured), so the newer syntax and API are also checked by name."""
        bad = []
        try:
            ast.parse(src, feature_version=(3, 6))
        except SyntaxError as e:
            bad.append("feature_version (3,6): %s" % e)
        tree = ast.parse(src)
        later = {"NamedExpr": "3.8 walrus", "Match": "3.10 match", "TryStar": "3.11 except*", "TypeAlias": "3.12 type"}
        for node in ast.walk(tree):
            k = type(node).__name__
            if k in later:
                bad.append(later[k])
            if k == "JoinedStr":
                bad.append("f-string (kept out so that 3.5 reaches the version check)")
            if isinstance(node, (ast.FunctionDef, ast.Lambda)) and getattr(node.args, "posonlyargs", []):
                bad.append("3.8 positional-only parameters")
            if isinstance(node, ast.ImportFrom) and node.module == "__future__" and \
                    any(a.name == "annotations" for a in node.names):
                bad.append("3.7 from __future__ import annotations")
            if isinstance(node, ast.Attribute) and node.attr in (
                    "time_ns", "monotonic_ns", "fromisoformat", "removeprefix", "removesuffix", "nullcontext", "isascii",
                    "cached_property", "parse_intermixed_args", "prod", "isqrt", "unraisablehook", "reconfigure"):
                bad.append("3.7+ API .%s" % node.attr)
            if isinstance(node, ast.Name) and node.id in ("breakpoint", "dataclass", "dataclasses", "contextvars"):
                bad.append("3.7+ API %s" % node.id)
            if isinstance(node, ast.keyword) and node.arg in ("capture_output", "text"):
                bad.append("3.7+ keyword %s=" % node.arg)
            if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and \
                    node.func.attr == "add_subparsers" and any(k.arg == "required" for k in node.keywords):
                bad.append("3.7+ add_subparsers(required=)")
        for t in tokenize.generate_tokens(io.StringIO(src).readline):
            if t.type == tokenize.STRING and t.string[:2].lower() in ("f'", 'f"', "rf", "fr"):
                bad.append("f-string token")
        return bad

    FLOOR_FILES = (HS_PATH, DPC_PATH)          # every file the homepage server loads (H-04-R: the uploaded checker too)
    TOP_IMPORTS = {"collections", "hashlib", "re", "sys"}

    @staticmethod
    def import_contract(src):
        """download_page_check.py: module-level imports from a 3.6 stdlib whitelist only; nothing at module level names the
        Mac-only advisory module (it is not uploaded; make_advisory/main load it inside the function)."""
        tree, bad, todo = ast.parse(src), [], None
        todo = list(tree.body)
        while todo:
            n = todo.pop()
            if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda, ast.ClassDef)):
                continue
            todo.extend(ast.iter_child_nodes(n))
            mods = [a.name for a in n.names] if isinstance(n, ast.Import) else (
                [n.module or ""] if isinstance(n, ast.ImportFrom) else [])
            bad += ["module-level import %s" % m for m in mods if m.split(".")[0] not in SourceTests.TOP_IMPORTS]
            if isinstance(getattr(n, "value", None), str) and "download_page_advisory" in n.value:
                bad.append("module-level reference to the advisory module")
        return bad

    def test_publication_primitives_only(self):
        for p in self.FLOOR_FILES:
            with self.subTest(p=os.path.basename(p)):
                self.assertEqual(self.publication_problems(rd(p).decode("utf-8")), [])

    def test_publication_scan_positive_controls(self):
        for inject in ("import shutil\n", "import subprocess\n"):
            for p in self.FLOOR_FILES:
                self.assertTrue(self.publication_problems(inject + rd(p).decode("utf-8")), (inject, p))
        mutated = self.SRC.replace("    os.rename(src, dst)\n\n\ndef _st_dev", "    os.rename(src, dst)\n\n\ndef _x(a, b):\n"
                                   "    os.rename(a, b)\n\n\ndef _st_dev", 1)
        self.assertNotEqual(mutated, self.SRC)
        self.assertTrue(self.publication_problems(mutated))

    def test_python36_floor(self):
        for p in self.FLOOR_FILES:
            with self.subTest(p=os.path.basename(p)):
                self.assertEqual(self.floor_problems(rd(p).decode("utf-8")), [])

    def test_page_check_import_contract(self):
        self.assertEqual(self.import_contract(rd(DPC_PATH).decode("utf-8")), [])
        src = rd(DPC_PATH).decode("utf-8")
        for inject in ("import html.parser\n", "import os\n", "import download_page_advisory\n",
                       "ADV = 'download_page_advisory.py'\n", "try:\n    import importlib\nexcept ImportError:\n    pass\n"):
            self.assertTrue(self.import_contract(inject + src), inject)

    def test_the_switch_holds_no_page_pin(self):
        """D-3: pins, slot counts and skeleton names live only in download_page_check.py."""
        self.assertEqual(re.findall(r"\b[0-9a-f]{64}\b", self.SRC), [])
        for name in ("SKELETON_SHA256", "JS_PINS", "index.skeleton", "downloads-index.skeleton", "ver_slots"):
            self.assertNotIn(name, self.SRC)

    def test_page_profiles_cover_pages_order(self):
        self.assertEqual(sorted(dpc.PROFILES), sorted(rel for rel, _flat in hs.PAGES_ORDER))
        self.assertEqual(rd(RUN_PATH), rd(HS_PATH))               # the tests run the switch's own bytes

    def test_python36_floor_positive_controls(self):
        for inject in ("if (zz := 1):\n    pass\n", "zz = f'{1}'\n", "import time\nzz = time.time_ns()\n",
                       "def zz(a, /):\n    pass\n", "from __future__ import annotations\n",
                       "import argparse\nargparse.ArgumentParser().add_subparsers(dest='c', required=True)\n",
                       "xx = 1\nzz = f'{xx=}'\n"):
            for p in self.FLOOR_FILES:
                self.assertTrue(self.floor_problems(inject + rd(p).decode("utf-8")), (inject, p))


if __name__ == "__main__":
    unittest.main(verbosity=2)
