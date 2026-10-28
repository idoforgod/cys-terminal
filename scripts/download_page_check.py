#!/usr/bin/env python3
"""download_page_check.py - the homepage download-page check (H-04-R, 0.14.56; design round/_evidence/0.14.56-P9/h04r-r3/
H04R-DESIGN-r3.md, converged at r3). Uploaded to the homepage server beside homepage-switch.py; also used on the Mac by
deploy-homepage.py, verify-release-remote.py and the pin workflow (main()).

Acceptance basis = a BYTE-LEVEL CLOSED WORLD (head ruling r1-(1)), one rule for every published page:
  page == render(SKELETON, release values) byte for byte. A skeleton is the reviewed page with declared literal slots:
      {{cys:VER}}             every occurrence of the version
      {{cys:SIZE:<suffix>}}   the MiB-floor size of cys_<V><suffix>
r3 (gemini-3 r2 major 1, head D-2): WHICH slots a page has is a per-page PROFILE, declared once in PROFILES below and
read by lint(), check_publish(), check_input() and pin_from() - nothing else names a slot count:
      index.html            9 VER + 4 SIZE (one per suffix), 8 pinned JS files, advisory "index-parser"
      downloads/index.html  12 VER + 0 SIZE, 0 JS, advisory "downloads-lint", 1 Defender section marker (owner rule)
r3 (gemini-3 r2 major 2, head D-5): the advisory layer has ONE call contract - a callable page_bytes -> list of
(code, detail) str pairs - built only by make_advisory() on the Mac. check_publish() can only APPEND its codes, after
the byte layer, and turns any exception or malformed return into a refusal. pin_from() REQUIRES an advisory (the
pin-time lint); the server never pins and never imports the advisory module (it is not uploaded).
Python 3.6+, stdlib only, no f-strings (homepage-switch.py's floor); single source of the pins (D-3).
"""
import collections
import hashlib
import re
import sys

if sys.version_info[:2] < (3, 6):
    sys.stderr.write("download_page_check: python3 >= 3.6 required, found %s\n" % sys.version.split()[0])
    sys.exit(3)

SUFFIXES = ("_aarch64.dmg", "_x64.dmg", "_x64-setup.exe", "_x64-setup.zip")
MARK_VER = b"{{cys:VER}}"
MARK_SIZE = dict((s, ("{{cys:SIZE:%s}}" % s).encode()) for s in SUFFIXES)
MARK_ANY = re.compile(rb"\{\{cys:[^}]*\}\}")
GRAMMAR = {"VER": re.compile(rb"[0-9]{1,4}\.[0-9]{1,4}\.[0-9]{1,4}"), "SIZE": re.compile(rb"[0-9]{1,6}")}
GRAMMAR_BYTES = {"VER": b"0123456789.", "SIZE": b"0123456789"}
NAME_RE = re.compile(r"^\./assets/[A-Za-z0-9._-]+\.js$")
MIDDOT = b" \xc2\xb7 "
DEFENDER_MARKER = b'data-cys-release-marker="windows-defender-guidance-v2"'   # verify-release-remote.py:66 (same bytes)
# index.html owner-rule counts the verifier asserts only AFTER publication (verify-release-remote.py:78-83 @9da2418d);
# as required literals a skeleton that breaks them is refused at pin time and at check time (R1-F1, same bytes).
INDEX_REQUIRED = ((b"dl-hero__macnote", 0), (b"dl-hero__winnote", 1), (b"App Translocation", 0))
REPIN_PAGE = ("python3 scripts/download_page_check.py --pin-page <reviewed-page.html> --profile %s --version %s "
              "--sizes-from <asset-dir> --out <new-skeleton>")
REPIN_JS = "python3 scripts/download_page_check.py --pin-js --page <reviewed-page.html> --assets <dir>  (changed: %s %s)"
CTX = 48

# name, rel (public path under the docroot = homepage-switch.py PAGES_ORDER key), skeleton_name (file uploaded beside
# this module), skeleton_sha256, ver_slots (exact), size_suffixes (exactly one slot each), js_pins, advisory (kind),
# required ((literal, exact count) checked on the skeleton at pin time and at check time)
Profile = collections.namedtuple("Profile", "name rel skeleton_name skeleton_sha256 ver_slots size_suffixes js_pins "
                                            "advisory required")

JS_PINS = {                    # the index page's whole module closure, crawled 2026-10-29 00:22:46 KST (live/closure.json)
    "./assets/main-e5a35f84.js": "55a5c84f43182d61d68bf8bae6bfa725bcd915f158a94a20bc42ef75b21e3a70",
    "./assets/site-data-yksQaPPU.js": "3f43b5255249fb5f834adf2a8d195305dd41f9d8f2fdad210a59edf08a1524d9",
    "./assets/i18n-e5a35f84.js": "4d208bb2e62c81b9827becf14cc0f1daa8c9c3f21cdb8ecfd80e8ef17704f3a0",
    "./assets/atlas-scene-core-CGx_Hj8_.js": "711247fe57b49529d8e9c2a0b6210bd9738b8157853bd4a7d98c1c0bb0bc25d5",
    "./assets/search-core-e5a35f84.js": "dc7be38372fc9b796c32e7a30230255eee13414d5c6dc73d1cfa12a0484756f0",
    "./assets/share-core-DrF7S8Yr.js": "1688f2687c09255672ecde1f8aa10919da3680ac1165cb73e9acea5e35b00689",
    "./assets/reading-core-zBtetisa.js": "36896b7f7c186c1ecc05b8dcda568de9ad30be0063e41528857d252f21b528a3",
    "./assets/i18n-core-7d3fbee3.js": "7d3fbee31b926fffea79bc06f4a588960474cf93c0573b68905d2896c4e47994",
}

PROFILES = {                    # the ONLY place a slot count, a skeleton pin or a JS pin is written (D-2, D-3)
    "index.html": Profile("index", "index.html", "index.skeleton",
                          "68c7b582a1bdefc1b6cf8bc1c8c7ec3843acda50882d0ec0f6713c65a1635e6c",   # rig/skeleton-index-20261029.tpl
                          9, SUFFIXES, JS_PINS, "index-parser", INDEX_REQUIRED),
    "downloads/index.html": Profile("downloads", "downloads/index.html", "downloads-index.skeleton",
                                    "1ed35f22695a9a0660536556127b03669cfe717b606cd5565a85967a98b4575c",
                                    # rig/skeleton-downloads-20261029.tpl
                                    12, (), {}, "downloads-lint", ((DEFENDER_MARKER, 1),)),
}


def sha256(b):
    return hashlib.sha256(b).hexdigest()


def mib(n):
    """MiB floor - the page's own size convention (deploy-homepage.py mib())."""
    return n // (1024 * 1024)


def parts(tpl):
    """Template bytes -> [bytes, ('VER',), ('SIZE', suffix), bytes, ...]. Unknown marks are an error."""
    out, pos = [], 0
    for m in MARK_ANY.finditer(tpl):
        out.append(tpl[pos:m.start()])
        tok = m.group(0)
        if tok == MARK_VER:
            out.append(("VER",))
        else:
            suf = [s for s, mk in MARK_SIZE.items() if mk == tok]
            if not suf:
                raise ValueError("unknown slot mark %r at %d" % (tok, m.start()))
            out.append(("SIZE", suf[0]))
        pos = m.end()
    out.append(tpl[pos:])
    return out


def lint(tpl, profile):
    """Refusals for a skeleton under its profile: exact slot counts, unambiguous slot delimiters, required literals."""
    bad = []
    try:
        ps = parts(tpl)
    except ValueError as e:
        return [str(e)]
    kinds = [p for p in ps if isinstance(p, tuple)]
    if kinds.count(("VER",)) != profile.ver_slots:
        bad.append("VER slots %d (profile %s wants %d)" % (kinds.count(("VER",)), profile.name, profile.ver_slots))
    for s in SUFFIXES:
        want = 1 if s in profile.size_suffixes else 0
        if kinds.count(("SIZE", s)) != want:
            bad.append("SIZE slot %s occurs %d times (profile %s wants %d)" % (s, kinds.count(("SIZE", s)),
                                                                             profile.name, want))
    for lit, n in profile.required:
        if tpl.count(lit) != n:
            bad.append("required literal %r occurs %d times (profile %s wants %d)" % (lit, tpl.count(lit),
                                                                                     profile.name, n))
    for i, p in enumerate(ps):
        if isinstance(p, tuple):
            g = GRAMMAR_BYTES[p[0]]
            before, after = ps[i - 1], ps[i + 1]
            if not before or not after:
                bad.append("slot %r at part %d is not between two fixed byte runs" % (p, i))
                continue
            if before[-1:] in [bytes([c]) for c in g] or after[:1] in [bytes([c]) for c in g]:
                bad.append("slot %r at part %d touches a byte its grammar accepts" % (p, i))
    return bad


def render(tpl, ver, sizes_mib):
    out = []
    for p in parts(tpl):
        if isinstance(p, bytes):
            out.append(p)
        elif p[0] == "VER":
            out.append(ver.encode("ascii"))
        else:
            out.append(str(int(sizes_mib[p[1]])).encode("ascii"))
    return b"".join(out)


def _ctx(b, off):
    return b[max(0, off - CTX):off + CTX]


def first_diff(a, b):
    n = min(len(a), len(b))
    for i in range(n):
        if a[i] != b[i]:
            return i
    return n if len(a) != len(b) else -1


def match(page, tpl):
    """Structure-only: does page = skeleton with SOME grammar-valid slot values? -> (values, None) or (None, (off, msg))."""
    pos, vals = 0, []
    for p in parts(tpl):
        if isinstance(p, bytes):
            if page[pos:pos + len(p)] != p:
                off = pos + first_diff(page[pos:pos + len(p)], p)
                return None, (off, "fixed bytes differ from the skeleton")
            pos += len(p)
        else:
            m = GRAMMAR[p[0]].match(page, pos)
            if not m:
                return None, (pos, "slot %s: no value of its grammar here" % (p,))
            vals.append((p, m.group(0).decode("ascii"), pos))
            pos = m.end()
    if pos != len(page):
        return None, (pos, "trailing bytes after the skeleton")
    return vals, None


def _skeleton_errors(tpl, profile):
    if sha256(tpl) != profile.skeleton_sha256:
        return [("SKELETON-PIN", "%s skeleton sha256 %s != pinned %s" % (profile.name, sha256(tpl),
                                                                        profile.skeleton_sha256))]
    bad = lint(tpl, profile)
    if bad:
        return [("SKELETON-LINT", "%s: %s" % (profile.name, "; ".join(bad)))]
    return []


def _advisory_codes(advisory, page):
    """The ONE advisory call contract. Only ever returns refusals; anything but a list of (str, str) is a refusal."""
    try:
        got = advisory(page)
    except Exception as e:                                   # fail closed: an advisory that cannot answer refuses
        return [("ADVISORY-ERROR", "advisory raised %s: %s" % (type(e).__name__, e))]
    if not isinstance(got, list):
        return [("ADVISORY-BAD-RETURN", "advisory returned %s, not a list" % type(got).__name__)]
    out = []
    for item in got:
        if isinstance(item, tuple) and len(item) == 2 and isinstance(item[0], str) and isinstance(item[1], str) \
                and item[0]:
            out.append(("ADVISORY-" + item[0], item[1]))
        else:
            out.append(("ADVISORY-BAD-ITEM", "advisory item %r is not a (code, detail) str pair" % (item,)))
    return out


def check_publish(page, tpl, profile, ver, sizes_bytes, fetch, advisory=None):
    """The gate for a page that is about to be (or has been) published. -> list of (code, detail); empty = pass.
    The advisory layer runs last and can only add entries (it never sees the list built before it)."""
    errs = _skeleton_errors(tpl, profile)
    if errs:
        return errs
    try:
        want = render(tpl, ver, dict((s, mib(sizes_bytes["cys_%s%s" % (ver, s)])) for s in profile.size_suffixes))
    except (KeyError, ValueError, UnicodeEncodeError) as e:
        return [("EXPECTED-UNMEASURED", "cannot compute the expected %s: %s" % (profile.rel, e))]
    off = first_diff(page, want)
    if off != -1:
        return [("PAGE-DIFF", "%s: first differing byte at offset %d; page[%d:]=%r ; expected[%d:]=%r ; re-pin (our "
                 "reviewed commit, never the owner): %s" % (profile.rel, off, max(0, off - CTX), _ctx(page, off),
                                                            max(0, off - CTX), _ctx(want, off),
                                                            REPIN_PAGE % (profile.name, ver)))]
    errs = check_js(profile.js_pins, fetch)
    if advisory is not None:
        errs = errs + _advisory_codes(advisory, page)
    return errs


def check_js(js_pins, fetch):
    """The pinned JS module closure: each PINNED name (never a name read from a page) must read back its pinned sha256."""
    errs = []
    for name in sorted(js_pins):
        if not NAME_RE.match(name):
            errs.append(("SCRIPT-NAME", "pinned name %r is not ./assets/<file>.js" % name))
            continue
        b = fetch(name) if fetch is not None else None
        if b is None:
            errs.append(("SCRIPT-UNMEASURED", "%s could not be read - unmeasured is never a pass" % name))
        elif sha256(b) != js_pins[name]:
            errs.append(("SCRIPT-SHA", "%s sha256 %s != pinned %s ; re-pin after review: %s"
                         % (name, sha256(b), js_pins[name], REPIN_JS % (name, sha256(b)))))
    return errs


def check_input(page, tpl, profile):
    """The live page a release starts from: it must be the reviewed skeleton (else someone edited the homepage and
    rendering would silently revert that edit). Slot values may be anything their grammar allows - they are replaced."""
    errs = _skeleton_errors(tpl, profile)
    if errs:
        return None, errs
    vals, err = match(page, tpl)
    if err:
        off, msg = err
        return None, [("LIVE-DRIFT", "%s: %s at offset %d: %r ; the live page is not the pinned skeleton - review the "
                       "change, then re-pin: %s" % (profile.rel, msg, off, _ctx(page, off),
                                                    REPIN_PAGE % (profile.name, "<old>")))]
    return vals, []


def _skeleton_from(page, ver, sizes_mib, profile):
    """Mechanical slotting under a profile (no advisory). Public callers use pin_from()."""
    if MARK_ANY.search(page):
        raise ValueError("the page already contains a slot mark")
    t = page
    for s in profile.size_suffixes:
        lit = b"cys_" + ver.encode() + s.encode() + MIDDOT + str(sizes_mib[s]).encode() + b"MB"
        if t.count(lit) < 1:
            raise ValueError("no meta literal %r" % lit)
        t = t.replace(lit, b"cys_" + ver.encode() + s.encode() + MIDDOT + MARK_SIZE[s] + b"MB")
    left = re.findall(rb"(?<![0-9])[0-9]{1,6}MB", t)
    if left:
        raise ValueError("size tokens remain outside the declared meta literals of profile %s: %r" % (profile.name, left))
    t = t.replace(ver.encode(), MARK_VER)
    bad = lint(t, profile)
    if bad:
        raise ValueError("lint: " + "; ".join(bad))
    if render(t, ver, sizes_mib) != page:
        raise ValueError("round trip failed")
    return t


def pin_from(page, ver, sizes_mib, profile, advisory):
    """Pin time (a reviewed commit, on the Mac): the reviewed page -> skeleton bytes. The advisory is REQUIRED: a lint
    refusal means no pin (r2 s2.6 step 2, now enforced here, not only by the CLI)."""
    if not callable(advisory):
        raise TypeError("pin_from needs the pin-time advisory (make_advisory(...)); got %r" % (advisory,))
    t = _skeleton_from(page, ver, sizes_mib, profile)
    codes = _advisory_codes(advisory, page)
    if codes:
        raise ValueError("advisory: " + "; ".join("%s %s" % c for c in codes))
    return t


def make_advisory(profile, tpl, ver, sizes_bytes, fetch, js_pins=None):
    """Mac only. Loads the advisory module (NOT uploaded to the server) lazily, by explicit path beside this file, and
    returns the one-contract adapter: page_bytes -> [(code, detail), ...]."""
    return _advisory_module().adapter(profile, tpl, ver, sizes_bytes, fetch,
                                      profile.js_pins if js_pins is None else js_pins)


def main(argv):
    """Mac-side pin workflow (never run on the server). Exit 0 = pinned, 1 = refused, 2 = usage."""
    import argparse
    import json
    import os
    ap = argparse.ArgumentParser(prog="download_page_check.py")
    ap.add_argument("--pin-page", metavar="HTML")
    ap.add_argument("--pin-js", action="store_true")
    ap.add_argument("--profile", choices=sorted(set(p.name for p in PROFILES.values())))
    ap.add_argument("--version")
    ap.add_argument("--sizes-from", metavar="ASSET_DIR")
    ap.add_argument("--js-pins", metavar="JSON", help="new JS pins (from --pin-js) used by the pin-time advisory")
    ap.add_argument("--old-skeleton", metavar="FILE", help="the skeleton being replaced: its diff is printed (required)")
    ap.add_argument("--first-pin", action="store_true", help="no previous skeleton exists (the only way to omit the diff)")
    ap.add_argument("--out", metavar="NEW_SKELETON")
    ap.add_argument("--page", metavar="HTML")
    ap.add_argument("--assets", metavar="DIR")
    try:
        a = ap.parse_args(argv)
    except SystemExit:
        return 2
    if a.pin_js:
        if not a.page or not a.assets:
            sys.stderr.write("--pin-js needs --page and --assets\n")
            return 2
        mod = _advisory_module()
        res = mod.crawl_closure(open(a.page, "rb").read(), a.assets)
        sys.stdout.write(json.dumps(res, indent=1, sort_keys=True) + "\n")
        return 1 if res["refusals"] else 0
    if not (a.pin_page and a.profile and a.version and a.out):
        sys.stderr.write("--pin-page needs --profile, --version and --out\n")
        return 2
    if bool(a.old_skeleton) == bool(a.first_pin):            # a re-pin always shows its diff (r2 s2.6 step 2, R1-F2)
        sys.stderr.write("--pin-page needs exactly one of --old-skeleton (re-pin: the diff is printed) or --first-pin\n")
        return 2
    prof = [p for p in PROFILES.values() if p.name == a.profile][0]
    page = open(a.pin_page, "rb").read()
    sizes_bytes = {}
    for s in prof.size_suffixes:
        if not a.sizes_from:
            sys.stderr.write("profile %s has SIZE slots: --sizes-from is required\n" % prof.name)
            return 2
        n = "cys_%s%s" % (a.version, s)
        sizes_bytes[n] = os.path.getsize(os.path.join(a.sizes_from, n))
    sizes_mib = dict((s, mib(sizes_bytes["cys_%s%s" % (a.version, s)])) for s in prof.size_suffixes)
    js_pins = json.load(open(a.js_pins))["pins"] if a.js_pins else prof.js_pins
    if js_pins or prof.js_pins:
        # A page with JS: the pins used for this pin must equal a FRESH crawl of the reviewed assets - whether they come
        # from --js-pins (a re-pin after a bundle change) or from the current table. Disagreement = no pin.
        if not a.assets:
            sys.stderr.write("profile %s has JS pins: --assets (the reviewed .js files) is required\n" % prof.name)
            return 2
        fresh = _advisory_module().crawl_closure(page, a.assets)
        if fresh["refusals"] or fresh["pins"] != js_pins:
            diff = sorted(set(fresh["pins"]) ^ set(js_pins)) + sorted(
                n for n in set(fresh["pins"]) & set(js_pins) if fresh["pins"][n] != js_pins[n])
            sys.stderr.write("REFUSED (no pin written): JS-PINS-DISAGREE between %s and a fresh crawl of %s: %s; "
                             "crawl refusals %s - run --pin-js, review, then pass its output as --js-pins\n"
                             % ("--js-pins" if a.js_pins else "the current JS_PINS", a.assets, diff, fresh["refusals"]))
            return 1
    try:
        tpl = _skeleton_from(page, a.version, sizes_mib, prof)
        adv = make_advisory(prof._replace(skeleton_sha256=sha256(tpl), js_pins=js_pins), tpl, a.version,
                            sizes_bytes, _local_fetch(a.assets), js_pins)
        pin_from(page, a.version, sizes_mib, prof, adv)
    except (ValueError, TypeError) as e:
        sys.stderr.write("REFUSED (no pin written): %s\n" % e)
        return 1
    try:
        with open(a.out, "xb") as f:                             # exclusive create: never overwrites a file
            f.write(tpl)
    except OSError as e:
        sys.stderr.write("REFUSED (no pin written): %s\n" % e)
        return 1
    if a.old_skeleton:
        import difflib
        old = open(a.old_skeleton, "rb").read().decode("utf-8", "replace").splitlines(True)
        new = tpl.decode("utf-8", "replace").splitlines(True)
        sys.stdout.writelines(difflib.unified_diff(old, new, "old-skeleton", "new-skeleton", n=1))
    sys.stdout.write("PINNED profile=%s skeleton=%s sha256=%s bytes=%d VER=%d SIZE=%d\n"
                     % (prof.name, a.out, sha256(tpl), len(tpl), tpl.count(MARK_VER),
                        sum(tpl.count(m) for m in MARK_SIZE.values())))
    sys.stdout.write("constant for the reviewed commit: PROFILES[%r].skeleton_sha256 = %r\n" % (prof.rel, sha256(tpl)))
    if js_pins != prof.js_pins:
        sys.stdout.write("constant for the reviewed commit: JS_PINS = %s\n" % json.dumps(js_pins, indent=1, sort_keys=True))
    return 0


def _advisory_module():
    import importlib.util
    import os
    p = os.path.join(os.path.dirname(os.path.abspath(__file__)), "download_page_advisory.py")
    spec = importlib.util.spec_from_file_location("download_page_advisory", p)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _local_fetch(assets_dir):
    import os

    def fetch(name):
        if assets_dir is None or not NAME_RE.match(name):
            return None
        try:
            with open(os.path.join(assets_dir, name.split("/")[-1]), "rb") as f:
                return f.read()
        except OSError:
            return None
    return fetch


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
