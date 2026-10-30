#!/usr/bin/env python3
"""test_download_page_check.py - controls for scripts/download_page_check.py + scripts/download_page_advisory.py (H-04-R).

Design: round/_evidence/0.14.56-P9/h04r-r3/H04R-DESIGN-r3.md (converged at r3). A page passes only if it is byte for byte
the reviewed skeleton with its declared slots filled (per-page PROFILES); the JS closure is pinned by name and sha256;
the advisory layer can only add refusals; pin_from requires the advisory.
Fixtures (scripts/tests/fixtures/download_page/, PUBLIC-repo safe): the reviewed index skeleton SANITIZED (same-length
replacement of the owner's contact address and a local path, so every offset is unchanged), the reviewed downloads
skeleton (clean, unchanged), patches.json (the 10-27 page and the codex fixtures as byte ops, each with the sha256 of its
sanitized result), and 8 tiny SYNTHETIC JS files with the real names and the real import graph. Fixture profiles carry
the fixture pins; the production PROFILES are checked for shape, never for these pins.
The r2 cases below the marker "r2 cases (verbatim)" are the lines of the reviewed r2 specimen, byte for byte, except the
one line that built SK_BASE with the r2 API.
Run: PYTHONDONTWRITEBYTECODE=1 python3 scripts/tests/test_download_page_check.py      (stdlib only, no network)
"""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import shutil
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPTS = os.path.normpath(os.path.join(HERE, ".."))
FIX = os.path.join(HERE, "fixtures", "download_page")
ASSETS = os.path.join(FIX, "assets")


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


dpc = _load("download_page_check", os.path.join(SCRIPTS, "download_page_check.py"))
V, V2 = "0.14.54", "0.14.56"
SIZES_MIB = {"_aarch64.dmg": 261, "_x64.dmg": 260, "_x64-setup.exe": 137, "_x64-setup.zip": 137}
SK_INDEX = open(os.path.join(FIX, "index-sanitized.skeleton"), "rb").read()
SD = open(os.path.join(FIX, "downloads-index.skeleton"), "rb").read()
PATCHES = json.load(open(os.path.join(FIX, "patches.json"), encoding="utf-8"))
JS_PINS = dict(("./assets/" + n, hashlib.sha256(open(os.path.join(ASSETS, n), "rb").read()).hexdigest())
               for n in sorted(os.listdir(ASSETS)))
FX_IX = dpc.PROFILES["index.html"]._replace(skeleton_sha256=hashlib.sha256(SK_INDEX).hexdigest(), js_pins=JS_PINS)
FX_DL = dpc.PROFILES["downloads/index.html"]      # the reviewed downloads skeleton is clean: production pin, real bytes
IX, DLP = FX_IX, FX_DL


def _apply(base, ops):
    out, pos = [], 0
    for off, dl, ins in ops:
        out.append(base[pos:off])
        out.append(ins.encode("latin-1"))
        pos = off + dl
    out.append(base[pos:])
    return b"".join(out)


def _patched(key, base):
    p = PATCHES[key]
    page = _apply(base, p["ops"])
    assert hashlib.sha256(page).hexdigest() == p["sha256"], ("fixture drift", key)
    return page


NOW = dpc.render(SK_INDEX, V, SIZES_MIB)            # the 2026-10-29 live page, sanitized
OLD27 = _patched("old27", NOW)                       # the 2026-10-27 copy, sanitized
CODEX = {"js_wrong_selected_button.html": _patched("js_wrong_selected_button", OLD27),
         "duplicate_reordered_anchor.html": _patched("duplicate_reordered_anchor", OLD27),
         "h04-wrong-card.html": _patched("h04_wrong_card", OLD27)}
SK28 = dpc._skeleton_from(NOW, V, SIZES_MIB, FX_IX)
SK27 = dpc._skeleton_from(OLD27, V, SIZES_MIB, FX_IX)
P9 = "codex-fixtures"
FETCHED = []


def bytes_for(ver, mibs=SIZES_MIB):
    return dict(("cys_%s%s" % (ver, s), mibs[s] * 1024 * 1024 + 7) for s in dpc.SUFFIXES)


def local_fetch(name, override=None):
    FETCHED.append(name)
    if override and name in override:
        return override[name]
    p = os.path.join(ASSETS, os.path.basename(name))
    try:
        return open(p, "rb").read()
    except OSError:
        return None


# ======== r2 helpers (verbatim r2 lines 72-101) ========
def sub1(b, old, new):
    old, new = old.encode("utf-8") if isinstance(old, str) else old, new.encode("utf-8") if isinstance(new, str) else new
    assert b.count(old) == 1, ("anchor count", old[:60], b.count(old))
    return b.replace(old, new)


def rd(*p):
    return open(os.path.join(*p), "rb").read()


DL_S = NOW.index(b"<script>", 14700)
DL_E = NOW.index(b"</script>", DL_S) + len(b"</script>")
DL_SCRIPT = NOW[DL_S:DL_E]
WIN_A = '<a class="dl-btn" data-dl="win-x64"\n         href="/downloads/cys_0.14.54_x64-setup.exe">'
WIN_META = '<span class="dl-btn__meta" lang="en">cys_0.14.54_x64-setup.exe · 137MB</span>'
BRANCH_HEAD = "if(isEdgeIE){var w=document.querySelector('.dl-btn[data-dl=\"win-x64\"]');if(w){"
WRONG = DL_SCRIPT.replace(b"0.14.54", b"0.0.1")
INJ = '<svg><title><div><a data-dl="win-x64" class="dl-btn" href="/downloads/wrong.exe"><span class="dl-btn__meta">' \
      'wrong.exe · 1MB</span></a><script>document.querySelector("a[data-dl=mac-arm]").href="/downloads/wrong.exe";' \
      '</script></div></title></svg>'

cases = []


def case(name, page, expect, codes=(), mode="pub", sk="28", ver=V, mibs=SIZES_MIB, js=None, js_override=None,
         at=None, advisory=True, src_pins=None):
    cases.append(dict(name=name, page=page, expect=expect, codes=set(codes), mode=mode, sk=sk, ver=ver, mibs=mibs,
                      js=JS_PINS if js is None else js, js_override=js_override, at=at, advisory=advisory,
                      src_pins=src_pins))

def rd(*p):                                          # r2 read a file; here: the codex fixtures rebuilt from patches.json
    return CODEX[p[-1]]


# ======== r2 cases (verbatim r2 lines 103-231; one line changed: SK_BASE) ========
# ---- baselines ----
case("B1 reviewed 10-27 copy vs its own skeleton - BYTE LAYER ONLY: its bundle main-DIZ3GFio.js was never captured, so "
     "the JS axis and the advisory layer are not exercised for this copy (run 1: the advisory layer refused it as "
     "SCRIPT-SRC-UNMEASURED, which is the right answer)", OLD27, "accept", sk="27", js={}, src_pins={}, advisory=False)
case("B2 live page 2026-10-29 00:22 (= 10-28 23:04 capture)", NOW, "accept")
case("C5 the version bump, rendered: 0.14.54 -> 0.14.56", NOW.replace(b"0.14.54", b"0.14.56"), "accept", ver=V2)
# ---- the two round-3 majors (codex-r3 fixtures, unmodified, built on the 10-27 copy) ----
case("M1 codex js_wrong_selected_button", rd(P9, "codex-r3", "js_wrong_selected_button.html"), "refuse", {"PAGE-DIFF"},
     sk="27", js={})
case("M2 codex duplicate_reordered_anchor", rd(P9, "codex-r3", "duplicate_reordered_anchor.html"), "refuse",
     {"PAGE-DIFF"}, sk="27", js={})
# ---- v1 M1 family (on the live page) ----
case("M1b reassignment of w inside the branch", sub1(NOW, BRANCH_HEAD, BRANCH_HEAD +
     "w=document.querySelector('a[data-dl=\"mac-arm\"]');"), "refuse", {"PAGE-DIFF"})
case("M1c a second inline script rewrites a download href", sub1(NOW, "</body>", "<script>document.querySelector("
     "'a[data-dl=\"mac-arm\"]').setAttribute('href','/downloads/cys_0.14.54_x64-setup.exe');</script>\n</body>"),
     "refuse", {"PAGE-DIFF"})
case("M1d inline onclick on the Windows meta span", sub1(NOW, WIN_META, WIN_META.replace('lang="en">',
     'lang="en" onclick="this.parentNode.href=\'/x.exe\'">')), "refuse", {"PAGE-DIFF"})
case("M1e <base href>", sub1(NOW, "<head>", '<head><base href="https://example.invalid/">'), "refuse", {"PAGE-DIFF"})
case("M1f unknown external script name", sub1(NOW, "./assets/main-e5a35f84.js", "./assets/main-XXXX.js"), "refuse",
     {"PAGE-DIFF"})
case("M1g JS meta literal names the exe", sub1(NOW, "m.textContent='cys_0.14.54_x64-setup.zip · 137MB'",
     "m.textContent='cys_0.14.54_x64-setup.exe · 137MB'"), "refuse", {"PAGE-DIFF"})
# ---- v1 M2 family ----
case("M2b legit reordered attributes (v1: accept) -> r2 REFUSE: not the reviewed bytes", sub1(NOW, WIN_A,
     '<a href="/downloads/cys_0.14.54_x64-setup.exe" data-dl="win-x64" class="dl-btn">'), "refuse", {"PAGE-DIFF"})
case("M2c single-quoted attributes (v1: accept) -> r2 REFUSE", sub1(NOW, WIN_A,
     "<a class='dl-btn' data-dl='win-x64' href='/downloads/cys_0.14.54_x64-setup.exe'>"), "refuse", {"PAGE-DIFF"})
case("M2d upper-case tag/attributes (v1: accept) -> r2 REFUSE", sub1(NOW, WIN_A,
     '<A CLASS="dl-btn" DATA-DL="win-x64" HREF="/downloads/cys_0.14.54_x64-setup.exe">'), "refuse", {"PAGE-DIFF"})
case("M2e extra <button class=dl-btn data-dl=win-x64>", sub1(NOW, WIN_A,
     '<button class="dl-btn" data-dl="win-x64">x</button>' + WIN_A), "refuse", {"PAGE-DIFF"})
case("M2f extra plain anchor to /downloads/", sub1(NOW, WIN_A, '<a href="/downloads/evil.exe">get</a>' + WIN_A),
     "refuse", {"PAGE-DIFF"})
case("M2g duplicate slot inside <noscript>", sub1(NOW, WIN_A, '<noscript><a data-dl="win-x64" class="dl-btn" '
     'href="/downloads/x.exe"><span class="dl-btn__meta">x · 1MB</span></a></noscript>' + WIN_A), "refuse",
     {"PAGE-DIFF"})
case("M2h duplicated href attribute on a slot", sub1(NOW, WIN_A, WIN_A[:-1] +
     ' href="/downloads/cys_0.14.54_x64.dmg">'), "refuse", {"PAGE-DIFF"})
# ---- carried dispositions ----
C1 = rd(P9, "codex-r2", "h04-wrong-card.html")
case("C1/F4 codex-r2 metadata swap - as the release's input page", C1, "refuse", {"LIVE-DRIFT"}, mode="in", sk="27")
case("C1/F4 codex-r2 metadata swap - as a page to publish", C1, "refuse", {"PAGE-DIFF"}, sk="27", js={})
STALE = sub1(NOW, "cys_0.14.54_aarch64.dmg · 261MB", "cys_0.14.54_aarch64.dmg · 250MB")
case("C2 stale mac-arm size - as the release's input page: replaced by the render (repair)", STALE, "repair",
     mode="in")
case("C2 stale mac-arm size - as a page to publish", STALE, "refuse", {"PAGE-DIFF"})
STALE_JS = sub1(NOW, "'cys_0.14.54_x64-setup.zip · 137MB'", "'cys_0.14.54_x64-setup.zip · 99MB'")
case("C3 stale JS meta size - as the input page (repair)", STALE_JS, "repair", mode="in")
case("C3 stale JS meta size - as a page to publish", STALE_JS, "refuse", {"PAGE-DIFF"})
C4 = sub1(NOW, 'href="/downloads/cys_0.14.54_aarch64.dmg"', 'href="/downloads/cys_0.14.54_x64.dmg"')
case("C4 mac-arm href -> x64 dmg (input)", C4, "refuse", {"LIVE-DRIFT"}, mode="in")
case("C4 mac-arm href -> x64 dmg (publish)", C4, "refuse", {"PAGE-DIFF"})
# ---- v2 cases ----
case("X1 main bundle: same name, different bytes", NOW, "refuse", {"SCRIPT-SHA"},
     js_override={"./assets/main-e5a35f84.js": b"/* evil */"})
case("X1b modulepreload chunk i18n: same name, different bytes (gemini-2 major 1-(2), gemini-3 major 1-(4))", NOW,
     "refuse", {"SCRIPT-SHA"}, js_override={"./assets/i18n-e5a35f84.js": b"/* evil */"})
case("X1c dynamic chunk atlas-scene-core changed", NOW, "refuse", {"SCRIPT-SHA"},
     js_override={"./assets/atlas-scene-core-CGx_Hj8_.js": b"/* evil */"})
case("X1d second-level dynamic chunk reading-core changed (imported by i18n)", NOW, "refuse", {"SCRIPT-SHA"},
     js_override={"./assets/reading-core-zBtetisa.js": b"/* evil */"})
case("X2 a pinned chunk unreadable", NOW, "refuse", {"SCRIPT-UNMEASURED"},
     js_override={"./assets/site-data-yksQaPPU.js": None})
case("X3 new bundle name in the page", sub1(NOW, "main-e5a35f84.js\"", "main-NEWNAME1.js\""), "refuse", {"PAGE-DIFF"})
F1 = sub1(NOW, "cys_0.14.54_aarch64.dmg · 261MB", "cys_0.14.53_aarch64.dmg · 261MB")
case("F1 same slot, other version (input: repaired by the render)", F1, "repair", mode="in")
case("F1 same slot, other version (publish)", F1, "refuse", {"PAGE-DIFF"})
case("F2 mac-arm meta = x64 dmg (input)", sub1(NOW, "cys_0.14.54_aarch64.dmg · 261MB", "cys_0.14.54_x64.dmg · 261MB"),
     "refuse", {"LIVE-DRIFT"}, mode="in")
case("F3 mac-arm meta = Windows exe (input)", sub1(NOW, "cys_0.14.54_aarch64.dmg · 261MB",
     "cys_0.14.54_x64-setup.exe · 261MB"), "refuse", {"LIVE-DRIFT"}, mode="in")
# ---- gemini-3 PoCs ----
POS_WIN = NOW.index(WIN_A.encode())
case("G3-b1 svg/title RCDATA breakout before the Windows anchor", sub1(NOW, WIN_A, INJ + WIN_A), "refuse",
     {"PAGE-DIFF"}, at=POS_WIN + 1)   # predicted POS_WIN before run 1; both pages hold '<' there, the first diff is +1
case("G3-b2-1a module script (0.0.1) before the sync download script", sub1(NOW, DL_SCRIPT,
     WRONG.replace(b"<script>", b'<script type="module">') + b"\n" + DL_SCRIPT), "refuse", {"PAGE-DIFF"}, at=DL_S + 7)
case("G3-b2-1b active 0.0.1 script + inert <template> copy", sub1(NOW, DL_SCRIPT, WRONG + b"<template>" + DL_SCRIPT +
     b"</template>"), "refuse", {"PAGE-DIFF"})
case("G3-b2-1c download script moved into <head>", sub1(sub1(NOW, DL_SCRIPT, b""), "</head>", DL_SCRIPT + b"</head>"),
     "refuse", {"PAGE-DIFF"})
case("G3-b2-1d download script inside <template>", sub1(NOW, DL_SCRIPT, b"<template>" + DL_SCRIPT + b"</template>"),
     "refuse", {"PAGE-DIFF"})
case("G3-b2-1d' download script inside <noscript>", sub1(NOW, DL_SCRIPT, b"<noscript>" + DL_SCRIPT + b"</noscript>"),
     "refuse", {"PAGE-DIFF"})
case("G3-m1-1 self-closing <template/> + relative downloads/ href", sub1(NOW, WIN_A, '<template/><a class="dl-btn" '
     'data-dl="win-x64" href="downloads/evil.exe"></a>' + WIN_A), "refuse", {"PAGE-DIFF"})
case("G3-m1-2 <script type=text/ecmascript> rewrites a button", sub1(NOW, "</body>", "<script type=\"text/ecmascript\">"
     "document.querySelector('.dl-btn').href='/down'+'loads/evil.exe'</script></body>"), "refuse", {"PAGE-DIFF"})
case("G3-m1-2' <script type=importmap> remaps i18n", sub1(NOW, "<head>", '<head><script type="importmap">{"imports":'
     '{"./assets/i18n-e5a35f84.js":"https://evil.invalid/x.js"}}</script>'), "refuse", {"PAGE-DIFF"})
case("G3-m1-3 <svg><script href=/evil.js>", sub1(NOW, "</body>", '<svg><script href="/evil.js"></script></svg></body>'),
     "refuse", {"PAGE-DIFF"})
case("G3-m1-5a <iframe srcdoc>", sub1(NOW, "</body>", '<iframe srcdoc="<script>parent.document.links[0].href=1'
     '</script>"></iframe></body>'), "refuse", {"PAGE-DIFF"})
case("G3-m1-5b <meta http-equiv=refresh>", sub1(NOW, "<head>", '<head><meta http-equiv="refresh" content="0; '
     'url=/evil.exe">'), "refuse", {"PAGE-DIFF"})
case("G3-m1-5c <svg><a xlink:href>", sub1(NOW, "</body>", '<svg><a xlink:href="/evil.exe"><text>x</text></a></svg>'
     '</body>'), "refuse", {"PAGE-DIFF"})
case("G3-m1-5d <form action>", sub1(NOW, "</body>", '<form action="/evil.exe"><button>x</button></form></body>'),
     "refuse", {"PAGE-DIFF"})
# ---- gemini-2 PoCs ----
case("G2-1 parameterized MIME text/javascript;charset=utf-8", sub1(NOW, "</body>", '<script type="text/javascript;'
     'charset=utf-8">void 0</script></body>'), "refuse", {"PAGE-DIFF"})
case("G2-3 duplicate download script, the FIRST one wrong", sub1(NOW, DL_SCRIPT, WRONG + DL_SCRIPT), "refuse",
     {"PAGE-DIFF"})
case("G2-3b duplicate Windows anchor, the SECOND one wrong (gemini-2 major 1-(3) fixture note)", sub1(NOW, WIN_A + "\n",
     WIN_A + "\n" + '<a class="dl-btn" data-dl="win-x64" href="/downloads/wrong.exe"></a>' + "\n"), "refuse",
     {"PAGE-DIFF"})
case("X1e modulepreload chunk site-data: same name, different bytes", NOW, "refuse", {"SCRIPT-SHA"},
     js_override={"./assets/site-data-yksQaPPU.js": b"/* evil */"})
case("G2-m1 script src with a path (../../) - and it is never fetched", sub1(NOW, "./assets/main-e5a35f84.js",
     "../../etc/main-e5a35f84.js"), "refuse", {"PAGE-DIFF"})
# ---- slot injection (a slot value carries markup) ----
case("S1 a VER slot carrying markup (publish)", sub1(NOW, '"/downloads/cys_0.14.54_x64-setup.exe"',
     '"/downloads/cys_0.14.54"><script>x</script><a href="_x64-setup.exe"'), "refuse", {"PAGE-DIFF"})
case("S1 a VER slot carrying markup (input)", sub1(NOW, '"/downloads/cys_0.14.54_x64-setup.exe"',
     '"/downloads/cys_0.14.54"><script>x</script><a href="_x64-setup.exe"'), "refuse", {"LIVE-DRIFT"}, mode="in")
case("S2 a SIZE slot carrying markup (input)", sub1(NOW, "cys_0.14.54_x64.dmg · 260MB", "cys_0.14.54_x64.dmg · 260<b>MB"),
     "refuse", {"LIVE-DRIFT"}, mode="in")
case("S3 the S-slot (dl-hero__sub) version is stale (publish) - the S-slot is now a declared slot",
     sub1(NOW, "v0.14.54 · macOS", "v0.14.53 · macOS"), "refuse", {"PAGE-DIFF"})
# ---- advisory layer: adds refusals, never acceptances ----
WITH_BASE = sub1(NOW, "<head>", '<head><base href="https://example.invalid/">')
SK_BASE = dpc._skeleton_from(WITH_BASE, V, SIZES_MIB, FX_IX)   # r2: dpc.pin_from(WITH_BASE, V, SIZES_MIB) (r2 API)
case("A1 a page equal to a skeleton that was (wrongly) pinned with <base>: the byte layer passes, the advisory layer "
     "refuses", WITH_BASE, "refuse", {"ADVISORY-BASE-PRESENT"}, sk="BASE")


# ======== runner (the r3 specimen's, on the product module) ========
SKS = {"28": SK28, "27": SK27, "BASE": SK_BASE}


def prof_for(c):
    return FX_IX._replace(skeleton_sha256=dpc.sha256(SKS[c["sk"]]), js_pins=dict(c["js"]))


def run_r2_case(c):
    sk, prof = SKS[c["sk"]], prof_for(c)
    FETCHED[:] = []
    if c["mode"] == "in":
        vals, errs = dpc.check_input(c["page"], sk, prof)
        if errs:
            return "refuse", {e[0] for e in errs}, errs
        out = dpc.render(sk, c["ver"], c["mibs"])
        want = dpc.render(SK28 if c["sk"] == "28" else sk, c["ver"], c["mibs"])
        got = "repair" if c["page"] != out else "accept"
        return (got if out == want else "WRONG-OUTPUT"), set(), []
    fetch = (lambda cc: (lambda n: local_fetch(n, cc["js_override"])))(c)
    adv = dpc.make_advisory(prof, sk, c["ver"], bytes_for(c["ver"], c["mibs"]), fetch) if c["advisory"] else None
    errs = dpc.check_publish(c["page"], sk, prof, c["ver"], bytes_for(c["ver"], c["mibs"]), fetch, adv)
    return ("refuse" if errs else "accept"), {e[0] for e in errs}, errs


def met_r2(c, got, codes, errs):
    met = got == c["expect"] and (c["codes"] <= codes if c["codes"] else True)
    if met and c["at"] is not None and "PAGE-DIFF" in codes:
        d = [e for e in errs if e[0] == "PAGE-DIFF"][0][1]
        met = ("offset %d;" % c["at"]) in d
    if met and any(n for n in FETCHED if n not in JS_PINS):
        met = False
    return met


# ======== r3 cases ========
DL = dpc.render(SD, V, {})                     # == the live downloads page (the downloads skeleton is clean)
SB = bytes_for(V)
TMP = None


def lfetch(n):
    return local_fetch(n)


def adv_ix(ver=V):
    return dpc.make_advisory(FX_IX, SK28, ver, bytes_for(ver), lfetch)


def adv_dl(ver=V):
    return dpc.make_advisory(FX_DL, SD, ver, {}, None)


def pub_dl(page, ver=V):
    return dpc.check_publish(page, SD, FX_DL, ver, {}, None, adv_dl(ver))


def codes(errs):
    return {e[0] for e in errs}


DL_SWAP = sub1(sub1(sub1(DL, '<a href="/downloads/cys_0.14.54_aarch64.dmg">', '<a href="/downloads/cys_0.14.54_TMP">'),
                    '<a href="/downloads/cys_0.14.54_x64.dmg">', '<a href="/downloads/cys_0.14.54_aarch64.dmg">'),
               '<a href="/downloads/cys_0.14.54_TMP">', '<a href="/downloads/cys_0.14.54_x64.dmg">')
DL_NODEF = DL[:DL.index(b'    <section data-cys-release-marker="windows-defender-guidance-v2">')] + \
    DL[DL.index(b"    </section>\n  </main>") + len(b"    </section>\n"):]
SK28_8 = SK28.replace(dpc.MARK_VER, V.encode(), 1)
SD_11 = SD.replace(dpc.MARK_VER, V.encode(), 1)
SD_13 = SD.replace(b"<h2>macOS", b"<h2>{{cys:VER}} macOS", 1)
NOW_NOWIN = NOW.replace(b"dl-hero__winnote", b"dl-hero__note")
SK28_NOWIN = SK28.replace(b"dl-hero__winnote", b"dl-hero__note")


def _boom(page):
    raise RuntimeError("advisory crashed")


HOSTILE = [("empty list", lambda p: []), ("('OK','accept')", lambda p: [("OK", "accept")]), ("None", lambda p: None),
           ("'PASS'", lambda p: "PASS"), ("True", lambda p: True), ("raises", _boom),
           ("3-tuple item", lambda p: [("A", "b", "c")]), ("generator", lambda p: (x for x in [])),
           ("empty code", lambda p: [("", "x")]), ("bytes code", lambda p: [(b"OK", "x")])]


def cli(argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        rc = dpc.main(argv)
    return rc, out.getvalue(), err.getvalue()


def fresh(name):
    return os.path.join(tempfile.mkdtemp(dir=TMP), name)


class Fixtures(unittest.TestCase):
    def test_fixtures_are_the_sanitized_reviewed_bytes(self):
        self.assertEqual(dpc.sha256(NOW), "bfd3156e89799be3fed564fec0525074df9e4429588f7ff7d583c0c2701ee956")
        self.assertEqual(SK28, SK_INDEX)
        self.assertEqual(dpc.sha256(DL), "3be907e6a0bd5d4d392205ff2a4f10c31cd0f751ff0435cd6c50b4f65e10e90c")   # the 2026-10-29 live downloads page
        self.assertEqual(len(cases), 60)

    def test_production_profiles_shape(self):
        self.assertEqual(sorted(dpc.PROFILES), ["downloads/index.html", "index.html"])
        ix, dl = dpc.PROFILES["index.html"], dpc.PROFILES["downloads/index.html"]
        self.assertEqual((ix.ver_slots, ix.size_suffixes, ix.advisory), (9, dpc.SUFFIXES, "index-parser"))
        self.assertEqual((dl.ver_slots, dl.size_suffixes, dl.js_pins, dl.advisory), (12, (), {}, "downloads-lint"))
        self.assertEqual(len(ix.js_pins), 8)
        for name, h in list(ix.js_pins.items()) + [("index", ix.skeleton_sha256), ("downloads", dl.skeleton_sha256)]:
            self.assertRegex(h, "^[0-9a-f]{64}$", name)
        self.assertEqual(sorted(ix.js_pins), sorted(JS_PINS))       # the fixture closure has the production names
        # the production downloads pin = the reviewed bytes (the downloads skeleton carries no personal data, so the fixture
        # IS the reviewed file): a downloads re-pin commit refreshes fixtures/download_page/downloads-index.skeleton too.
        self.assertEqual(dl.skeleton_sha256, dpc.sha256(SD))


class R2CarriedCases(unittest.TestCase):
    def test_r2_cases(self):
        for c in cases:
            with self.subTest(case=c["name"][:70]):
                got, cs, errs = run_r2_case(c)
                self.assertTrue(met_r2(c, got, cs, errs), "%s: got %s %s %s" % (c["name"], got, sorted(cs),
                                                                              str(errs[:1])[:300]))


class Profiles(unittest.TestCase):
    def test_DL9_downloads_page_pins_under_its_profile(self):
        self.assertEqual(dpc.sha256(dpc.pin_from(DL, V, {}, FX_DL, adv_dl())), FX_DL.skeleton_sha256)

    def test_DL9b_downloads_page_refused_under_the_index_profile(self):
        with self.assertRaisesRegex(ValueError, "no meta literal"):
            dpc.pin_from(DL, V, SIZES_MIB, FX_IX, adv_ix())

    def test_DL1_DL2_publish_live_and_bumped(self):
        self.assertEqual(pub_dl(DL), [])
        self.assertEqual(pub_dl(dpc.render(SD, V2, {}), V2), [])

    def test_DL3_DL4_DL5_DL6_publish_refusals(self):
        for page, ver in ((DL_SWAP, V), (dpc.render(SD, V2, {}).replace(b"<code>cys_0.14.56_x64.dmg",
                                                                          b"<code>cys_0.14.54_x64.dmg"), V2),
                          (sub1(DL, "</main>", "<script>x()</script></main>"), V), (DL_NODEF, V)):
            self.assertEqual(codes(pub_dl(page, ver)), {"PAGE-DIFF"})

    def test_DL7_input_repairs_twelve_stale_slots(self):
        vals, errs = dpc.check_input(dpc.render(SD, "0.14.53", {}), SD, FX_DL)
        self.assertEqual((errs, len(vals), dpc.render(SD, V, {}) == DL), ([], 12, True))

    def test_DL8_input_edited_outside_a_slot(self):
        vals, errs = dpc.check_input(sub1(DL, "<h2>macOS", "<h2>NEW macOS"), SD, FX_DL)
        self.assertEqual((vals, codes(errs)), (None, {"LIVE-DRIFT"}))

    def test_DL10_pin_refuses_a_downloads_page_without_the_defender_section(self):
        with self.assertRaisesRegex(ValueError, "required literal"):
            dpc.pin_from(DL_NODEF, V, {}, FX_DL, adv_dl())

    def test_DL11_pin_time_advisory_refuses_swapped_hrefs(self):
        with self.assertRaisesRegex(ValueError, "LINK-TEXT"):
            dpc.pin_from(DL_SWAP, V, {}, FX_DL, adv_dl())

    def test_DL12_cross_profile(self):
        self.assertEqual(codes(dpc.check_publish(DL, SD, FX_IX, V, SB, lfetch)), {"SKELETON-PIN"})
        self.assertEqual(codes(dpc.check_publish(NOW, SD, FX_DL, V, {}, None)), {"PAGE-DIFF"})

    def test_LN1_LN2_LN3_exact_slot_counts(self):
        self.assertTrue(any("VER slots 8" in x for x in dpc.lint(SK28_8, FX_IX)))
        self.assertTrue(any("VER slots 11" in x for x in dpc.lint(SD_11, FX_DL)))
        self.assertTrue(any("VER slots 13" in x for x in dpc.lint(SD_13, FX_DL)))
        self.assertEqual((dpc.lint(SK28, FX_IX), dpc.lint(SD, FX_DL)), ([], []))
        self.assertTrue(dpc.lint(SK28, FX_DL) and dpc.lint(SD, FX_IX))

    def test_LN4_index_owner_rule_literals(self):
        self.assertTrue(any("required literal" in x for x in dpc.lint(SK28_NOWIN, FX_IX)))
        self.assertEqual(codes(dpc.check_publish(NOW_NOWIN, SK28_NOWIN, FX_IX._replace(
            skeleton_sha256=dpc.sha256(SK28_NOWIN)), V, SB, lfetch)), {"SKELETON-LINT"})
        with self.assertRaisesRegex(ValueError, "required literal"):
            dpc.pin_from(NOW_NOWIN, V, SIZES_MIB, FX_IX, adv_ix())

    def test_CL1_check_time_lint(self):
        errs = dpc.check_publish(dpc.render(SD_11, V, {}), SD_11, FX_DL._replace(skeleton_sha256=dpc.sha256(SD_11)), V,
                                 {}, None)
        self.assertEqual(codes(errs), {"SKELETON-LINT"})


class AdvisoryContract(unittest.TestCase):
    def test_AC1_AC2_real_adapters_answer_empty_on_the_reviewed_pages(self):
        self.assertEqual((adv_ix()(NOW), adv_dl()(DL)), ([], []))

    def test_AC3_an_advisory_can_add_a_refusal(self):
        self.assertEqual(codes(dpc.check_publish(NOW, SK28, FX_IX, V, SB, lfetch, lambda p: [("X", "y")])),
                         {"ADVISORY-X"})

    def test_AC4_real_index_adapter_refuses_a_base_page_pinned_by_mistake(self):
        prof = FX_IX._replace(skeleton_sha256=dpc.sha256(SK_BASE))
        errs = dpc.check_publish(WITH_BASE, SK_BASE, prof, V, SB, lfetch, dpc.make_advisory(prof, SK_BASE, V, SB, lfetch))
        self.assertIn("ADVISORY-BASE-PRESENT", codes(errs))

    def test_AC5_refuse_only_property(self):
        viol, n = [], 0
        pages = [(c["name"][:40], c["page"], SKS[c["sk"]], prof_for(c), c["ver"], bytes_for(c["ver"], c["mibs"]),
                  (lambda cc: (lambda nm: local_fetch(nm, cc["js_override"])))(c)) for c in cases if c["mode"] == "pub"]
        pages += [(nm, pg, SD, FX_DL, vr, {}, None) for nm, pg, vr in (
            ("DL1", DL, V), ("DL2", dpc.render(SD, V2, {}), V2), ("DL3", DL_SWAP, V), ("DL6", DL_NODEF, V),
            ("DL5", sub1(DL, "</main>", "<script>x()</script></main>"), V))]
        for name, page, sk, prof, ver, sizes, fetch in pages:
            base = dpc.check_publish(page, sk, prof, ver, sizes, fetch, None)
            for hname, h in HOSTILE:
                n += 1
                got = dpc.check_publish(page, sk, prof, ver, sizes, fetch, h)
                if base and (not got or not codes(base) <= codes(got)):
                    viol.append((name, hname, "refusal lost"))
                if not base and (got == []) != (hname == "empty list"):
                    viol.append((name, hname, "accepted page: %s" % sorted(codes(got))))
        self.assertEqual((n, viol), (560, []))

    def test_AC6_a_raising_advisory_refuses(self):
        self.assertEqual(codes(dpc.check_publish(NOW, SK28, FX_IX, V, SB, lfetch, _boom)), {"ADVISORY-ERROR"})

    def test_AC7_the_advisory_sees_only_the_page_bytes(self):
        seen = []
        self.assertEqual(dpc.check_publish(NOW, SK28, FX_IX, V, SB, lfetch, lambda *a: seen.append(a) or []), [])
        self.assertEqual([len(a) for a in seen], [1])
        self.assertEqual(seen[0][0], NOW)


class PinWorkflow(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.sizes = fresh("sizes")
        os.makedirs(cls.sizes)
        for s in dpc.SUFFIXES:
            with open(os.path.join(cls.sizes, "cys_%s%s" % (V, s)), "wb") as f:
                f.truncate(SIZES_MIB[s] * 1024 * 1024 + 7)            # sparse: logical size only
        cls.pages = {}
        for nm, b in (("index.html", NOW), ("dl.html", DL), ("base.html", WITH_BASE), ("old.skeleton", SK28)):
            cls.pages[nm] = fresh(nm)
            with open(cls.pages[nm], "wb") as f:
                f.write(b)
        cls.pins = fresh("pins.json")
        with open(cls.pins, "w") as f:
            json.dump({"pins": JS_PINS}, f)

    def pin_index(self, page, out, extra=(), assets=ASSETS, pins=None):
        return cli(["--pin-page", page, "--profile", "index", "--version", V, "--sizes-from", self.sizes,
                    "--assets", assets, "--js-pins", pins or self.pins, "--out", out] + list(extra))

    def test_P1_pin_from_refuses_the_base_page(self):
        prof = FX_IX._replace(skeleton_sha256=dpc.sha256(SK_BASE))
        with self.assertRaisesRegex(ValueError, "BASE-PRESENT"):
            dpc.pin_from(WITH_BASE, V, SIZES_MIB, FX_IX, dpc.make_advisory(prof, SK_BASE, V, SB, lfetch))

    def test_P2_pin_from_requires_an_advisory(self):
        with self.assertRaisesRegex(TypeError, "pin-time advisory"):
            dpc.pin_from(NOW, V, SIZES_MIB, FX_IX, None)

    def test_P3_cli_pins_the_reviewed_index_page(self):
        o = fresh("p3.skeleton")
        rc, out, err = self.pin_index(self.pages["index.html"], o, ["--old-skeleton", self.pages["old.skeleton"]])
        self.assertEqual((rc, open(o, "rb").read() == SK28, dpc.sha256(SK28) in out), (0, True, True), err)

    def test_P4_cli_refuses_the_base_page_and_writes_nothing(self):
        o = fresh("p4.skeleton")
        rc, out, err = self.pin_index(self.pages["base.html"], o, ["--old-skeleton", self.pages["old.skeleton"]])
        self.assertEqual((rc, os.path.exists(o), "BASE-PRESENT" in err), (1, False, True), err)

    def test_P5_cli_never_overwrites_out(self):
        o = fresh("p5.skeleton")
        with open(o, "wb") as f:
            f.write(b"keep me")
        rc, out, err = cli(["--pin-page", self.pages["dl.html"], "--profile", "downloads", "--version", V, "--out", o,
                            "--first-pin"])
        self.assertEqual((rc, open(o, "rb").read()), (1, b"keep me"))

    def test_P6_cli_pin_js_crawls_the_closure(self):
        rc, out, err = cli(["--pin-js", "--page", self.pages["index.html"], "--assets", ASSETS])
        res = json.loads(out)
        self.assertEqual((rc, res["pins"], res["refusals"]), (0, JS_PINS, []))

    def test_P7_cli_pins_the_downloads_page(self):
        o = fresh("p7.skeleton")
        rc, out, err = cli(["--pin-page", self.pages["dl.html"], "--profile", "downloads", "--version", V, "--out", o,
                            "--old-skeleton", os.path.join(FIX, "downloads-index.skeleton")])
        self.assertEqual((rc, dpc.sha256(open(o, "rb").read())), (0, FX_DL.skeleton_sha256), err)

    def test_P8_cli_pin_js_refuses_an_unreadable_chunk(self):
        part = fresh("assets")
        shutil.copytree(ASSETS, part)
        os.remove(os.path.join(part, "reading-core-zBtetisa.js"))
        rc, out, err = cli(["--pin-js", "--page", self.pages["index.html"], "--assets", part])
        self.assertEqual((rc, "reading-core" in out), (1, True))

    def test_P9_cli_index_without_sizes_is_usage(self):
        o = fresh("p9.skeleton")
        rc, out, err = cli(["--pin-page", self.pages["index.html"], "--profile", "index", "--version", V, "--out", o,
                            "--first-pin"])
        self.assertEqual((rc, os.path.exists(o)), (2, False))

    def test_P10_cli_needs_exactly_one_of_old_skeleton_and_first_pin(self):
        res = []
        for extra in ([], ["--first-pin", "--old-skeleton", self.pages["old.skeleton"]]):
            o = fresh("p10.skeleton")
            rc, out, err = cli(["--pin-page", self.pages["dl.html"], "--profile", "downloads", "--version", V,
                                "--out", o] + extra)
            res.append((rc, os.path.exists(o)))
        self.assertEqual(res, [(2, False), (2, False)])

    def _rotated(self):
        """A re-pin after a bundle change: main renamed (and so its bytes' place in the page), reading-core changed."""
        assets = fresh("assets")
        shutil.copytree(ASSETS, assets)
        os.rename(os.path.join(assets, "main-e5a35f84.js"), os.path.join(assets, "main-R0TATED1.js"))
        with open(os.path.join(assets, "reading-core-zBtetisa.js"), "ab") as f:
            f.write(b"/* changed */\n")
        page = NOW.replace(b"./assets/main-e5a35f84.js", b"./assets/main-R0TATED1.js")
        p = fresh("rotated.html")
        with open(p, "wb") as f:
            f.write(page)
        rc, out, err = cli(["--pin-js", "--page", p, "--assets", assets])
        pins = fresh("pins-new.json")
        with open(pins, "w") as f:
            f.write(out)
        return assets, page, p, pins, rc, json.loads(out)

    def test_J1_js_pins_from_a_fresh_pin_js_are_accepted(self):
        assets, page, p, pins, rc0, res = self._rotated()
        self.assertEqual(rc0, 0)
        self.assertIn("./assets/main-R0TATED1.js", res["pins"])
        self.assertNotEqual(res["pins"]["./assets/reading-core-zBtetisa.js"], JS_PINS["./assets/reading-core-zBtetisa.js"])
        o = fresh("j1.skeleton")
        rc, out, err = self.pin_index(p, o, ["--old-skeleton", self.pages["old.skeleton"]], assets=assets, pins=pins)
        self.assertEqual((rc, open(o, "rb").read() == dpc._skeleton_from(page, V, SIZES_MIB, FX_IX)), (0, True), err)
        self.assertIn("main-R0TATED1.js", out[out.index("JS_PINS"):])          # the constant for the reviewed commit

    def test_J2_stale_js_pins_disagreeing_with_the_assets_are_refused(self):
        assets, page, p, pins, rc0, res = self._rotated()
        o = fresh("j2.skeleton")
        rc, out, err = self.pin_index(p, o, ["--old-skeleton", self.pages["old.skeleton"]], assets=assets)
        self.assertEqual((rc, os.path.exists(o), "JS-PINS-DISAGREE" in err), (1, False, True), err)

    def test_J3_the_current_table_must_also_match_the_assets(self):
        o = fresh("j3.skeleton")                         # no --js-pins: the production JS_PINS vs the fixture assets
        rc, out, err = cli(["--pin-page", self.pages["index.html"], "--profile", "index", "--version", V,
                            "--sizes-from", self.sizes, "--assets", ASSETS, "--out", o,
                            "--old-skeleton", self.pages["old.skeleton"]])
        self.assertEqual((rc, os.path.exists(o), "the current JS_PINS" in err), (1, False, True), err)

    def test_J4_a_page_with_js_needs_assets(self):
        o = fresh("j4.skeleton")
        rc, out, err = cli(["--pin-page", self.pages["index.html"], "--profile", "index", "--version", V,
                            "--sizes-from", self.sizes, "--js-pins", self.pins, "--out", o, "--first-pin"])
        self.assertEqual((rc, os.path.exists(o)), (2, False))


class VerifierCoupling(unittest.TestCase):
    """D-6: the verifier keeps its own owner-rule literals; this pins them equal to PROFILES."""

    def test_required_literals_equal_the_verifier_constants(self):
        vrr = _load("verify_release_remote_coupling", os.path.join(SCRIPTS, "verify-release-remote.py"))
        self.assertEqual(dpc.DEFENDER_MARKER, vrr.DEFENDER_MARKER.encode("utf-8"))
        self.assertEqual(dpc.PROFILES["downloads/index.html"].required,
                         ((vrr.DEFENDER_MARKER.encode("utf-8"), vrr.DEFENDER_MARKER_EXPECT),))
        self.assertEqual(dpc.PROFILES["index.html"].required,
                         ((vrr.MACNOTE_CLASS.encode("utf-8"), vrr.MACNOTE_EXPECT),
                          (vrr.WINNOTE_CLASS.encode("utf-8"), vrr.WINNOTE_EXPECT),
                          (vrr.TRANSLOCATION_TOKEN.encode("utf-8"), vrr.TRANSLOCATION_EXPECT)))


class RealPinsAreNotFixturePins(unittest.TestCase):
    """A fixture can never be shipped as a pin: the production index skeleton pin and every production JS pin differ from
    the fixture ones. (The downloads skeleton fixture IS the reviewed file - it carries no personal data - so its
    production pin equals the fixture's sha256 by construction; test_production_profiles_shape asserts exactly that.)"""

    def test_real_pins_differ_from_fixture_pins(self):
        real = dpc.PROFILES["index.html"]
        self.assertNotEqual(real.skeleton_sha256, FX_IX.skeleton_sha256)
        self.assertEqual(set(real.js_pins.values()) & set(JS_PINS.values()), set())
        self.assertNotIn(b"{{cys:", NOW)                     # fixture pages are rendered, never skeletons


def _fixture_skeleton_dir(index_bytes=None):
    d = fresh("skel")
    os.makedirs(d)
    for name, b in (("index.skeleton", SK_INDEX if index_bytes is None else index_bytes), ("downloads-index.skeleton", SD)):
        with open(os.path.join(d, name), "wb") as f:
            f.write(b)
    return d


FX_PROFILES = {"index.html": FX_IX, "downloads/index.html": FX_DL}


class DeployPageStep(unittest.TestCase):
    """scripts/deploy-homepage.py: --skeleton-dir is required and checked before any credential or network use."""

    @classmethod
    def setUpClass(cls):
        cls.dep = _load("deploy_homepage_under_test", os.path.join(SCRIPTS, "deploy-homepage.py"))

    def refuse_before_anything(self, extra):
        def tripwire(*a, **k):
            raise AssertionError("reached credentials or network")
        saved = (self.dep.load_env, self.dep.curl)
        self.dep.load_env = self.dep.curl = tripwire
        err = io.StringIO()
        try:
            with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
                rc = self.dep.main(["deploy-homepage.py", V2, fresh("assets")] + extra)
        finally:
            self.dep.load_env, self.dep.curl = saved
        return rc, err.getvalue()

    def test_apply_is_refused_before_credentials_and_network(self):
        """PKG-3B M1 (HEAD-RULING-PKG3B-R1 1): --apply FTP-deleted every other version's assets and
        docs/HOMEPAGE_0.14.56.md forbids that route. It is refused with exit 2 before any credential load or network
        call. Every stub below records; with the refusal removed the run reaches load_env, ftp_list, fetch_bytes and
        ftp_put, so the zero-calls assertion (first) is the one that turns red."""
        calls = []
        names = ("load_env", "curl", "ftp_list", "ftp_put", "ftp_delete", "sftp_put", "fetch", "fetch_bytes")
        saved = {n: getattr(self.dep, n) for n in names + ("load_page_check", "load_skeletons", "build_pages")}

        def rec(name, ret):
            def f(*a, **k):
                calls.append(name)
                return ret
            return f
        stubs = {"load_env": {"FTP_HOST": "h", "FTP_USER": "u", "FTP_PASS": "p"}, "curl": None,
                 "ftp_list": [], "ftp_put": True, "ftp_delete": True, "sftp_put": True, "fetch": "", "fetch_bytes": b"p"}
        for n in names:
            setattr(self.dep, n, rec(n, stubs[n]))
        self.dep.load_page_check = lambda: dpc
        self.dep.load_skeletons = lambda *a, **k: ({}, [])
        self.dep.build_pages = lambda *a, **k: ({"index.html": b"i", "downloads/index.html": b"d"}, [], None)
        assets = fresh("assets-apply")
        os.makedirs(assets)
        for f in ("cys_%s_aarch64.dmg", "cys_%s_x64.dmg", "cys_%s_x64-setup.exe", "cys_%s_x64-setup.zip"):
            with open(os.path.join(assets, f % V2), "wb") as fh:
                fh.write(b"x")
        err = io.StringIO()
        try:
            with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
                rc = self.dep.main(["deploy-homepage.py", V2, assets, "--skeleton-dir", fresh("skel-unused"), "--apply"])
        finally:
            for n, v in saved.items():
                setattr(self.dep, n, v)
        self.assertEqual(calls, [], "--apply reached credentials or the network")
        self.assertEqual(rc, 2, err.getvalue())
        self.assertIn("docs/HOMEPAGE_0.14.56.md", err.getvalue())
        self.assertIn("SSH", err.getvalue())
        self.assertEqual(len(err.getvalue().strip().splitlines()), 1, err.getvalue())

    def test_usage_no_longer_advertises_apply_and_nothing_deletes(self):
        doc = self.dep.__doc__
        usage = doc[doc.index("사용:"):]
        self.assertNotIn("--apply  ", usage)
        self.assertFalse([l for l in usage.splitlines() if "--apply" in l and "#" in l and "집행" in l], usage)
        with open(os.path.join(SCRIPTS, "deploy-homepage.py"), encoding="utf-8") as f:
            src = f.read()
        self.assertEqual(src.count("ftp_delete("), 1, "ftp_delete must have no caller (only its def remains)")

    def test_missing_flag_refuses(self):
        rc, err = self.refuse_before_anything([])
        self.assertEqual(rc, 1, err)
        self.assertIn("--skeleton-dir", err)

    def test_missing_folder_refuses(self):
        rc, err = self.refuse_before_anything(["--skeleton-dir", os.path.join(TMP, "no-such-folder")])
        self.assertEqual(rc, 1, err)

    def test_unreadable_skeleton_refuses(self):
        d = _fixture_skeleton_dir()
        os.chmod(os.path.join(d, "downloads-index.skeleton"), 0)
        try:
            rc, err = self.refuse_before_anything(["--skeleton-dir", d])
        finally:
            os.chmod(os.path.join(d, "downloads-index.skeleton"), 0o600)
        self.assertEqual(rc, 1, err)
        self.assertIn("downloads-index.skeleton", err)

    def test_skeleton_whose_sha256_differs_from_its_pin_refuses(self):
        rc, err = self.refuse_before_anything(["--skeleton-dir", _fixture_skeleton_dir()])   # sanitized != real pin
        self.assertEqual(rc, 1, err)
        self.assertIn("index.skeleton", err)

    def test_build_pages_bumps_both_pages(self):
        skel, bad = self.dep.load_skeletons(_fixture_skeleton_dir(), dpc, FX_PROFILES)
        self.assertEqual(bad, [])
        new, refusals, macrep = self.dep.build_pages({"index.html": NOW, "downloads/index.html": DL}, V2, bytes_for(V2),
                                                     skel, lfetch, dpc, FX_PROFILES)
        self.assertEqual(refusals, [])
        self.assertEqual((new["index.html"].count(V2.encode()), V.encode() in new["index.html"],
                          new["downloads/index.html"].count(V2.encode())), (9, False, 12))

    def test_build_pages_refuses_an_edited_live_page_and_a_non_utf8_one(self):
        skel, bad = self.dep.load_skeletons(_fixture_skeleton_dir(), dpc, FX_PROFILES)
        _, refusals, _ = self.dep.build_pages({"index.html": NOW, "downloads/index.html": sub1(DL, "<h2>macOS",
                                                                                              "<h2>NEW macOS")},
                                              V2, bytes_for(V2), skel, lfetch, dpc, FX_PROFILES)
        self.assertTrue(any(r.startswith("downloads/index.html LIVE-DRIFT") for r in refusals), refusals)
        _, refusals, _ = self.dep.build_pages({"index.html": NOW.replace(b"</body>", b"\xff</body>"),
                                               "downloads/index.html": DL}, V2, bytes_for(V2), skel, lfetch, dpc,
                                              FX_PROFILES)
        self.assertTrue(any("UTF-8" in r for r in refusals), refusals)


class VerifierPageAxis(unittest.TestCase):
    """scripts/verify-release-remote.py axis 2: both pages byte for byte; no folder / unreadable / pin mismatch = FAIL."""

    @classmethod
    def setUpClass(cls):
        cls.vrr = _load("verify_release_remote_under_test", os.path.join(SCRIPTS, "verify-release-remote.py"))

    def verdict(self, served, skeleton_dir, ver=V, profiles=FX_PROFILES, clen=lambda u: 1024 * 1024):
        site = self.vrr.SITE

        def get_bytes(url):
            if url == site + "/":
                return served["index.html"]
            if url == site + "/downloads/":
                return served["downloads/index.html"]
            if url.startswith(site + "/assets/"):
                return local_fetch("." + url[len(site):])
            raise AssertionError(url)
        saved = (self.vrr.get_bytes, self.vrr.clen, self.vrr.PAGE_PROFILES)
        self.vrr.get_bytes, self.vrr.clen, self.vrr.PAGE_PROFILES = get_bytes, clen, profiles
        try:
            return self.vrr.pages_verdict(ver, skeleton_dir)
        finally:
            self.vrr.get_bytes, self.vrr.clen, self.vrr.PAGE_PROFILES = saved

    def served(self, ver=V):
        return {"index.html": dpc.render(SK_INDEX, ver, dict((s, 1) for s in dpc.SUFFIXES)),
                "downloads/index.html": dpc.render(SD, ver, {})}

    def test_the_reviewed_pages_pass(self):
        self.assertEqual(self.verdict(self.served(), _fixture_skeleton_dir()), (True, "2종 일치(바이트 단위)"))

    def test_old_pages_still_served_after_a_release_fail_both(self):
        ok, detail = self.verdict(self.served(V), _fixture_skeleton_dir(), ver=V2)
        self.assertFalse(ok)
        self.assertIn("index.html PAGE-DIFF", detail)
        self.assertIn("downloads/index.html PAGE-DIFF", detail)

    def test_no_folder_unreadable_and_pin_mismatch_fail(self):
        self.assertEqual(self.verdict(self.served(), None)[0], False)
        ok, detail = self.verdict(self.served(), os.path.join(TMP, "no-such-folder"))
        self.assertEqual((ok, "판독 불가" in detail), (False, True))
        d = _fixture_skeleton_dir()
        os.chmod(os.path.join(d, "index.skeleton"), 0)
        try:
            ok, detail = self.verdict(self.served(), d)
        finally:
            os.chmod(os.path.join(d, "index.skeleton"), 0o600)
        self.assertEqual((ok, "판독 불가" in detail), (False, True))
        ok, detail = self.verdict(self.served(), _fixture_skeleton_dir(), profiles=dpc.PROFILES)   # real pins
        self.assertEqual((ok, "SKELETON-PIN" in detail), (False, True))

    def test_skeleton_dir_argument_contract(self):
        """parse_args keeps its strict flag rule: --skeleton-dir takes exactly one folder value, given once."""
        pa = self.vrr.parse_args
        self.assertEqual(pa(["x", V, "--skeleton-dir", "/s"]), (V, None, False))
        self.assertEqual(pa(["x", V, "--skeleton-dir", "/s", "--no-prev"]), (V, None, True))
        for argv in (["x", V, "--skeleton-dir"], ["x", V, "--skeleton-dir", "--no-prev"],
                     ["x", V, "--skeleton-dir", "/a", "--skeleton-dir", "/b"]):
            self.assertIsInstance(pa(argv), str, argv)
        self.assertEqual(self.vrr.skeleton_dir_arg(["x", V, "--skeleton-dir", "/s"]), "/s")
        self.assertIsNone(self.vrr.skeleton_dir_arg(["x", V]))

    def test_missing_content_length_is_unmeasured(self):
        ok, detail = self.verdict(self.served(), _fixture_skeleton_dir(), clen=lambda u: None)
        self.assertEqual((ok, "측정 불능" in detail), (False, True))

    def test_a_crash_inside_the_axis_fails_it(self):
        """The axis never passes on a crash: a profile table without one page (KeyError) and a Content-Length probe that
        raises both end in (False, '페이지 검사 중단: <type>: ...'), never in a pass or a traceback."""
        partial = {"index.html": FX_PROFILES["index.html"]}
        ok, detail = self.verdict(self.served(), _fixture_skeleton_dir(), profiles=partial)
        self.assertEqual((ok, detail.startswith("페이지 검사 중단: KeyError")), (False, True), detail)

        def raising_clen(url):
            raise RuntimeError("probe died")
        ok, detail = self.verdict(self.served(), _fixture_skeleton_dir(), clen=raising_clen)
        self.assertEqual((ok, detail), (False, "페이지 검사 중단: RuntimeError: probe died"))


def setUpModule():
    global TMP
    TMP = tempfile.mkdtemp(prefix="dpc-test-")


def tearDownModule():
    shutil.rmtree(TMP, ignore_errors=True)


if __name__ == "__main__":
    unittest.main(verbosity=1)
