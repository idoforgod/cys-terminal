#!/usr/bin/env python3
"""download_page_advisory.py - the ADVISORY layer of the homepage page check (H-04-R, 0.14.56). Mac only.

Not uploaded to the homepage server (head decision D-5). download_page_check.py loads it lazily (make_advisory, main) by
explicit path; nothing at its module level refers to this file (test_homepage_switch.SourceTests import contract).
adapter() returns the ONE advisory contract: a callable page_bytes -> list of (code, detail) str pairs. check_publish()
calls it last, gives it the page bytes only, and only appends its codes; an exception or a malformed return is a refusal
there. So this layer can add refusals and can never accept anything.
  index-parser    check() below: the HTML-parser validator of design v2 (worker-15 h04r_validator_v2.py, sha256-16 3ae3bf6e64428f40,
                  code unchanged except two constants: PINNED_SCRIPT_SRCS is empty because adapter() passes the profile's
                  pins (single source, D-3), and REPIN_CMD names the current CLI). Inline-script pins come from the
                  REVIEWED skeleton (render(tpl)), never from the page under test; it may fetch only pinned names.
  downloads-lint  downloads/index.html has no script: no active content at all, and exactly one link per release file
                  whose text is that file's name (a pin-time swap of two hrefs is refused before it becomes the skeleton).
crawl_closure() is the pin-time JS closure crawler (--pin-js), reading a local assets folder (no network).
Design: round/_evidence/0.14.56-P9/h04r-r3/H04R-DESIGN-r3.md s3 (rig file sha256-16 4442c585b3cb0476).
"""
import hashlib
import os
import re
from html.parser import HTMLParser

SLOTS = (
    ("mac-arm", "cys_{v}_aarch64.dmg", "Apple Silicon"),
    ("mac-x64", "cys_{v}_x64.dmg", "Intel"),
    ("win-x64", "cys_{v}_x64-setup.exe", None),
)
JS_SLOT = ("win-x64", "cys_{v}_x64-setup.zip")
SLOT_NAMES = {s[0] for s in SLOTS}
SLOT_INNER_CLASSES = {"dl-btn__label", "dl-btn__arch", "dl-btn__meta"}

# The only two literals of an inline script that may differ from the reviewed bytes.
HREF_LIT_RE = re.compile(r"'/downloads/(cys_\d+\.\d+\.\d+_x64-setup\.zip)'")
META_LIT_RE = re.compile(r"'(cys_\d+\.\d+\.\d+_x64-setup\.zip · \d+MB)'")
PAIR_RE = re.compile(r"^\s*(\S+)\s*·\s*(\d+)MB\s*$")

# sha256 of each reviewed executable inline script AFTER literal substitution
# (computed by skeleton_digest() from page-index-live-r2.html, sha16 bdbc94c0f1a4290c; the live page
# fetched 2026-10-28 23:04 KST carries the same bytes). A human updates this table on purpose.
PINNED_INLINE_SCRIPTS = {}          # filled by the specimen from the reviewed page (see pin_from())
PINNED_HANDLERS = {("link", "onload", "this.media='all'")}
# decision 1 (head 23:3x): external scripts pinned by NAME and by sha256 of the bytes fetched at check time.
PINNED_SCRIPT_SRCS = {}          # single source (D-3): adapter() always passes the profile's JS pins as srcs
REPIN_CMD = "%s %s - re-pin after review: python3 scripts/download_page_check.py --pin-js --page <reviewed-page.html> --assets <dir>"

EXEC_TYPES = {"", "text/javascript", "application/javascript", "module"}
VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr"}


def mib(n):
    return int(n // 1024 // 1024)


def skeleton(text):
    """Script bytes with the two version literals replaced -> (skeleton, href_values, meta_values)."""
    hrefs = [m.group(1) for m in HREF_LIT_RE.finditer(text)]
    metas = [m.group(1) for m in META_LIT_RE.finditer(text)]
    sk = META_LIT_RE.sub("'<META>'", HREF_LIT_RE.sub("'/downloads/<HREF>'", text))
    return sk, hrefs, metas


def skeleton_digest(text):
    return hashlib.sha256(skeleton(text)[0].encode("utf-8")).hexdigest()


class _Scan(HTMLParser):
    def __init__(self, html):
        super().__init__(convert_charrefs=False)
        self.html = html
        self.starts = [0]
        for i, ch in enumerate(html):
            if ch == "\n":
                self.starts.append(i + 1)
        self.stack = []          # [tag, slot_index_or_None, role]
        self.slots = []          # recognised slot anchors
        self.scripts = []        # (attrs, content, abs_start)
        self.errs = []
        self.cur_script = None
        self.meta_target = None  # (slot dict) while inside a dl-btn__meta span
        self.ctx = []            # noscript/template depth markers

    def _abs(self):
        line, col = self.getpos()
        return self.starts[line - 1] + col

    def _err(self, code, subj, detail):
        self.errs.append((code, subj, detail))

    def _slot_ctx(self):
        for t, si, role in reversed(self.stack):
            if si is not None:
                return si
        return None

    def _element(self, tag, attrs, selfclosing):
        pos = self._abs()
        names = [k for k, _ in attrs]
        d = {}
        for k, v in attrs:
            d.setdefault(k, v if v is not None else "")
        classes = (d.get("class") or "").split()
        dl_classes = [c for c in classes if c.startswith("dl-btn")]
        for k, v in attrs:
            v = v or ""
            if k.startswith("on") and (tag, k, v) not in PINNED_HANDLERS:
                self._err("HANDLER-UNPINNED", "%s@%d" % (tag, pos), "%s=%r" % (k, v[:80]))
            if "javascript:" in v.lower():
                self._err("JS-URL", "%s@%d" % (tag, pos), "%s=%r" % (k, v[:80]))
        if tag == "base":
            self._err("BASE-PRESENT", "base@%d" % pos, "a <base> element re-targets every relative link")
        relevant = ("data-dl" in d) or bool(dl_classes) or any(
            "/downloads/" in (v or "").lower() for _, v in attrs)
        role = None
        slot_index = None
        in_slot = self._slot_ctx()
        if relevant:
            dup = sorted({k for k in names if names.count(k) > 1})
            if dup:
                self._err("ATTR-DUP", "%s@%d" % (tag, pos), "duplicate attributes %s" % dup)
            if tag == "a" and "dl-btn" in classes and d.get("data-dl") in SLOT_NAMES and "href" in d \
                    and in_slot is None and len(dl_classes) == 1:
                if self.ctx:
                    self._err("SLOT-CONTEXT", d["data-dl"], "slot anchor inside <%s>" % self.ctx[-1])
                slot = {"slot": d["data-dl"], "href": d["href"], "pos": pos, "metas": [],
                        "arch": [], "labels": 0}
                self.slots.append(slot)
                slot_index = len(self.slots) - 1
                role = "slot"
            elif (in_slot is not None and tag == "span" and len(dl_classes) == 1
                  and dl_classes[0] in SLOT_INNER_CLASSES and "data-dl" not in d
                  and not any("/downloads/" in (v or "").lower() for _, v in attrs)):
                role = dl_classes[0]
                s = self.slots[in_slot]
                if role == "dl-btn__meta":
                    s["metas"].append({"parts": [], "bad": False, "pos": pos})
                    self.meta_target = s
                elif role == "dl-btn__arch":
                    s["arch"].append("")
                else:
                    s["labels"] += 1
            else:
                self._err("UNBOUND-DOWNLOAD-ELEMENT", "%s@%d" % (tag, pos),
                          "attrs=%r" % ([(k, (v or "")[:60]) for k, v in attrs],))
        if tag in ("noscript", "template") and not selfclosing:
            self.ctx.append(tag)
        if tag == "script":
            self.cur_script = {"attrs": d, "parts": [], "pos": None}
        if not selfclosing and tag not in VOID:
            self.stack.append([tag, slot_index, role])

    def handle_starttag(self, tag, attrs):
        self._element(tag, attrs, False)

    def handle_startendtag(self, tag, attrs):
        self._element(tag, attrs, True)

    def handle_endtag(self, tag):
        if tag == "script" and self.cur_script is not None:
            self.scripts.append(self.cur_script)
            self.cur_script = None
        if tag in ("noscript", "template") and self.ctx and self.ctx[-1] == tag:
            self.ctx.pop()
        if not any(t == tag for t, _, _ in self.stack):
            return
        while self.stack:
            t, si, role = self.stack.pop()
            if role == "dl-btn__meta":
                self.meta_target = None
            if t == tag:
                break
            if si is not None or role is not None:
                self._err("MARKUP-SHAPE", tag, "end tag </%s> closes an open <%s> slot part" % (tag, t))

    def handle_data(self, data):
        if self.cur_script is not None:
            if self.cur_script["pos"] is None:
                self.cur_script["pos"] = self._abs()
            self.cur_script["parts"].append(data)
            return
        if self.meta_target is not None:
            top = self.stack[-1] if self.stack else None
            m = self.meta_target["metas"][-1]
            if top is None or top[2] != "dl-btn__meta":
                m["bad"] = True
            m["parts"].append((self._abs(), data))
        elif self.stack and self.stack[-1][2] == "dl-btn__arch":
            si = self._slot_ctx()
            self.slots[si]["arch"][-1] += data

    def handle_entityref(self, name):
        if self.meta_target is not None:
            self.meta_target["metas"][-1]["bad"] = True

    handle_charref = handle_entityref


def check(html, ver, sizes, repair=False, pinned=None, srcs="default", fetch=None):
    """-> (ok, rows, errs, new_html). Structure faults refuse; meta text is repaired only if repair."""
    pinned = PINNED_INLINE_SCRIPTS if pinned is None else pinned
    srcs = PINNED_SCRIPT_SRCS if srcs == "default" else srcs
    sc = _Scan(html)
    sc.feed(html)
    sc.close()
    errs = list(sc.errs)
    rows, edits, claimed = [], [], {}

    # --- slots: presence, uniqueness, inner shape (M2) ---
    by = {}
    for s in sc.slots:
        by.setdefault(s["slot"], []).append(s)
    for name in SLOT_NAMES:
        n = len(by.get(name, []))
        if n == 0:
            errs.append(("MISSING-SLOT", name, "no <a class=dl-btn data-dl=%s href>" % name))
        elif n > 1:
            errs.append(("DUPLICATE-SLOT", name, "%d anchors" % n))
    for s in sc.slots:
        if len(s["metas"]) != 1:
            errs.append(("META-COUNT", s["slot"], "%d dl-btn__meta in the anchor" % len(s["metas"])))
        elif s["metas"][0]["bad"] or len(s["metas"][0]["parts"]) != 1:
            errs.append(("META-SHAPE", s["slot"], "meta is not one plain text node"))
        if len(s["arch"]) > 1:
            errs.append(("ARCH-COUNT", s["slot"], "%d dl-btn__arch" % len(s["arch"])))

    # --- scripts: closed world (M1) ---
    js = None
    for scr in sc.scripts:
        typ = (scr["attrs"].get("type") or "").strip().lower()
        if "src" in scr["attrs"]:
            name = scr["attrs"]["src"]
            if srcs is not None:
                data = fetch(name) if fetch else None
                if data is None:
                    errs.append(("SCRIPT-SRC-UNMEASURED", name, "bytes could not be read at check time"))
                    continue
                got = hashlib.sha256(data).hexdigest()
                if srcs.get(name) != got:
                    errs.append(("SCRIPT-SRC-UNPINNED", name, "sha256=%s; after review re-pin with: %s"
                                 % (got, REPIN_CMD % (name, got))))
            continue
        text = "".join(scr["parts"])
        if typ not in EXEC_TYPES:
            if "/downloads/" in text.lower():
                errs.append(("DOWNLOAD-REF-OUTSIDE", "script type=%s" % typ, "data block names /downloads/"))
            continue
        sk, hrefs, metas = skeleton(text)
        role = pinned.get(hashlib.sha256(sk.encode("utf-8")).hexdigest())
        if role is None:
            errs.append(("SCRIPT-UNPINNED", "inline@%s" % scr["pos"],
                         "executable inline script differs from every reviewed script"))
            continue
        if role == "download":
            if len(hrefs) != 1 or len(metas) != 1:
                errs.append(("JS-SLOT-COUNT", "download", "href %d / meta %d literals" % (len(hrefs), len(metas))))
                continue
            mm = META_LIT_RE.search(text)
            js = {"href": hrefs[0], "meta": metas[0],
                  "meta_span": (scr["pos"] + mm.start(1), scr["pos"] + mm.end(1))}
        elif hrefs or metas:
            errs.append(("JS-SLOT-COUNT", role, "a non-download script carries a download literal"))
    if js is None and not any(e[0] in ("SCRIPT-UNPINNED", "JS-SLOT-COUNT") for e in errs):
        errs.append(("JS-BRANCH-MISSING", "download", "the reviewed download script is absent"))

    if errs:
        return False, rows, errs, html

    # --- values: slot table, then text repair by offset span ---
    for slot, tmpl, arch_want in SLOTS:
        want_f = tmpl.format(v=ver)
        s = by[slot][0]
        hb = os.path.basename(s["href"])
        if s["href"] != "/downloads/" + want_f:
            errs.append(("SLOT-HREF-CONFLICT", slot, "href=%s want /downloads/%s" % (s["href"], want_f)))
            continue
        arch = s["arch"][0].strip() if s["arch"] else None
        if arch != arch_want:
            errs.append(("SLOT-LABEL-MISMATCH", slot, "arch=%r want %r" % (arch, arch_want)))
            continue
        claimed.setdefault(want_f, []).append(slot)
        pos, text = s["metas"][0]["parts"][0]
        rows.append(_text_row(slot, s["href"], want_f, sizes, pos, text, repair, edits, tmpl))
    want_z = JS_SLOT[1].format(v=ver)
    if js["href"] != want_z:
        errs.append(("JS-HREF-CONFLICT", "win-x64(js)", "href=%s want %s" % (js["href"], want_z)))
    else:
        pm = PAIR_RE.match(js["meta"])
        if pm.group(1) != js["href"]:
            errs.append(("JS-META-HREF-MISMATCH", "win-x64(js)", "href=%s meta=%s" % (js["href"], pm.group(1))))
        else:
            claimed.setdefault(want_z, []).append("win-x64(js)")
            st, en = js["meta_span"]
            rows.append(_text_row("win-x64(js)", "/downloads/" + js["href"], want_z, sizes, st, js["meta"],
                                  repair, edits, JS_SLOT[1]))
    for f in [t.format(v=ver) for _, t, _ in SLOTS] + [want_z]:
        if len(claimed.get(f, [])) != 1:
            errs.append(("ASSET-CLAIM-COUNT", f, "places %s" % claimed.get(f, [])))
    for r in rows:
        if r[-1] == "META-CROSS-SLOT":
            errs.append(("META-CROSS-SLOT", r[0], "meta names %s, a file of another slot/platform" % r[4]))
    if any(r[-1] == "META-UNPARSED" for r in rows):
        errs.append(("META-UNPARSED", "meta", "a meta text is not '<file> · <n>MB'"))
    if errs:
        return False, rows, errs, html
    out = html
    for st, en, rep in sorted(edits, reverse=True):
        out = out[:st] + rep + out[en:]
    if repair and edits:
        ok2, _, errs2, _ = check(out, ver, sizes, repair=False, pinned=pinned, srcs=srcs, fetch=fetch)   # A-5 re-validation
        if not ok2:
            return False, rows, [("REPAIR-REVALIDATION", "page", str(errs2))], html
    ok = all(r[-1] == "OK" or r[-1].endswith("->REPAIRED") for r in rows)
    return ok, rows, errs, out


def _same_slot(tmpl, name):
    """decision 2: is name this slot's own file at some version (normal version substitution)?"""
    pat = "^" + re.escape(tmpl).replace(re.escape("{v}"), r"\d+\.\d+\.\d+") + "$"
    return re.match(pat, name) is not None


def _text_row(slot, href, want_f, sizes, pos, text, repair, edits, tmpl):
    want_mb = mib(sizes[want_f])
    pm = PAIR_RE.match(text)
    if not pm:
        return (slot, href, want_f, want_mb, None, None, "META-UNPARSED")
    got_f, got_mb = pm.group(1), int(pm.group(2))
    if got_f != want_f and not _same_slot(tmpl, got_f):
        return (slot, href, want_f, want_mb, got_f, got_mb, "META-CROSS-SLOT")
    if got_f != want_f and got_mb != want_mb:
        state = "META-WRONG-ASSET"
    elif got_f != want_f:
        state = "META-WRONG-FILENAME"
    elif got_mb != want_mb:
        state = "META-STALE-SIZE"
    else:
        state = "OK"
    if repair and state != "OK":
        lead = len(text) - len(text.lstrip())
        trail = len(text.rstrip())
        edits.append((pos + lead, pos + trail, "%s · %dMB" % (want_f, want_mb)))
        state += "->REPAIRED"
    return (slot, href, want_f, want_mb, got_f, got_mb, state)


def pin_from(reviewed_html):
    """Build the pin table from a reviewed page: every executable inline script -> role."""
    sc = _Scan(reviewed_html)
    sc.feed(reviewed_html)
    sc.close()
    table = {}
    for scr in sc.scripts:
        typ = (scr["attrs"].get("type") or "").strip().lower()
        if "src" in scr["attrs"] or typ not in EXEC_TYPES:
            continue
        text = "".join(scr["parts"])
        sk, hrefs, metas = skeleton(text)
        table[hashlib.sha256(sk.encode("utf-8")).hexdigest()] = "download" if hrefs else "inline-%d" % len(table)
    return table


SUFFIXES = ("_aarch64.dmg", "_x64.dmg", "_x64-setup.exe", "_x64-setup.zip")
ACTIVE_TAGS = {"script", "base", "iframe", "frame", "frameset", "object", "embed", "form", "template", "svg", "math",
               "applet", "portal"}


def _decode(page):
    try:
        return page.decode("utf-8"), None
    except UnicodeDecodeError as e:
        return None, [("UNDECODABLE", "page is not UTF-8: %s" % e)]


def adapter(profile, tpl, ver, sizes_bytes, fetch, js_pins):
    from_skeleton = _render(tpl, ver, sizes_bytes, profile)
    pinned_names = set(js_pins)

    def guarded_fetch(name):                     # the advisory may never fetch a name outside the pinned table
        return fetch(name) if (name in pinned_names and fetch is not None) else None

    if profile.advisory == "index-parser":
        inline_pins = pin_from(from_skeleton.decode("utf-8"))

        def adv(page):
            text, err = _decode(page)
            if err:
                return err
            ok, rows, errs, _out = check(text, ver, sizes_bytes, repair=False, pinned=inline_pins,
                                            srcs=dict(js_pins), fetch=guarded_fetch)
            out = [(str(e[0]), str(e[1:])[:300]) for e in errs]
            if not ok and not out:                       # rows not all OK without an error row = still a refusal
                out = [("NOT-OK", str([r[-1] for r in rows])[:300])]
            return out
        return adv
    if profile.advisory == "downloads-lint":
        def adv(page):
            text, err = _decode(page)
            if err:
                return err
            return downloads_lint(text, ver)
        return adv
    raise ValueError("unknown advisory kind %r for profile %s" % (profile.advisory, profile.name))


def _render(tpl, ver, sizes_bytes, profile):
    out, pos = [], 0
    for m in re.finditer(rb"\{\{cys:([^}]*)\}\}", tpl):
        out.append(tpl[pos:m.start()])
        tok = m.group(1)
        if tok == b"VER":
            out.append(ver.encode("ascii"))
        else:
            suf = tok.split(b":", 1)[1].decode("ascii")
            out.append(str(sizes_bytes["cys_%s%s" % (ver, suf)] // (1024 * 1024)).encode("ascii"))
        pos = m.end()
    out.append(tpl[pos:])
    return b"".join(out)


class _DlScan(HTMLParser):
    def __init__(self):
        HTMLParser.__init__(self, convert_charrefs=True)
        self.errs, self.links, self._a = [], [], None

    def handle_starttag(self, tag, attrs):
        self._tag(tag, attrs)

    def handle_startendtag(self, tag, attrs):
        self._tag(tag, attrs)

    def _tag(self, tag, attrs):
        t = tag.lower()
        if t in ACTIVE_TAGS:
            self.errs.append(("ACTIVE-TAG", "<%s> at %s" % (t, self.getpos())))
        d = {}
        for k, v in attrs:
            k = (k or "").lower()
            v = v or ""
            if k.startswith("on"):
                self.errs.append(("HANDLER", "%s on <%s> at %s" % (k, t, self.getpos())))
            if k in ("href", "src", "action", "formaction", "xlink:href", "srcdoc", "data") and \
                    re.sub(r"[\s\x00-\x1f]", "", v).lower().startswith(("javascript:", "data:", "vbscript:")):
                self.errs.append(("SCRIPT-URL", "%s=%r on <%s>" % (k, v[:60], t)))
            if t == "meta" and k == "http-equiv":
                self.errs.append(("META-HTTP-EQUIV", "%r" % v))
            if t == "link" and k == "rel" and any(x in v.lower().split() for x in ("modulepreload", "preload",
                                                                                    "prefetch", "import")):
                self.errs.append(("LINK-PRELOAD", "%r" % v))
            d[k] = v
        if t == "a":
            self._a = {"href": d.get("href"), "text": []}
            self.links.append(self._a)

    def handle_endtag(self, tag):
        if tag.lower() == "a":
            self._a = None

    def handle_data(self, data):
        if self._a is not None:
            self._a["text"].append(data)


def downloads_lint(text, ver):
    sc = _DlScan()
    sc.feed(text)
    sc.close()
    errs = list(sc.errs)
    want = ["cys_%s%s" % (ver, s) for s in SUFFIXES]
    rel = [ln for ln in sc.links if ln["href"] and "/downloads/cys_" in ln["href"]]
    for f in want:
        hits = [ln for ln in rel if ln["href"] == "/downloads/" + f]
        if len(hits) != 1:
            errs.append(("LINK-COUNT", "%s: %d links (want 1)" % (f, len(hits))))
        elif "".join(hits[0]["text"]).strip() != f:
            errs.append(("LINK-TEXT", "%s: link text %r names another file" % (f, "".join(hits[0]["text"]).strip())))
    for ln in rel:
        if ln["href"] not in ["/downloads/" + f for f in want]:
            errs.append(("LINK-OTHER", "href %r is not one of this release's four files" % ln["href"]))
    return errs


# ---- pin time: the JS module closure (rig_crawl_closure.py r1, local files instead of GETs) ----
LIT = [re.compile(r'from\s*"(\./[A-Za-z0-9._-]+\.js)"'), re.compile(r'import\s*"(\./[A-Za-z0-9._-]+\.js)"'),
       re.compile(r'import\(\s*[`"\'](\./[A-Za-z0-9._-]+\.js)[`"\']\s*\)'),
       re.compile(r'[`"\'](\./[A-Za-z0-9._-]+\.js)[`"\']'),
       re.compile(r'[`"\'](/assets/[A-Za-z0-9._-]+\.js)[`"\']')]
DYN = re.compile(r'(?:import\(|new Worker\(|importScripts\(|\beval\(|new Function\()\s*([^`"\'\s)][^)]{0,40})\)')


def crawl_closure(page_bytes, assets_dir):
    page = page_bytes.decode("utf-8")
    roots = re.findall(r'<script[^>]*\ssrc="(\./assets/[A-Za-z0-9._-]+\.js)"', page) + \
        re.findall(r'<link[^>]*rel="modulepreload"[^>]*href="(\./assets/[A-Za-z0-9._-]+\.js)"', page)
    seen, todo, pins, refusals = {}, list(dict.fromkeys(roots)), {}, []
    while todo:
        name = todo.pop(0)
        if name in seen:
            continue
        try:
            b = open(os.path.join(assets_dir, name.split("/")[-1]), "rb").read()
        except OSError as e:
            refusals.append([name, "unreadable: %s" % e])
            seen[name] = None
            continue
        seen[name] = len(b)
        pins[name] = hashlib.sha256(b).hexdigest()
        t = b.decode("utf-8", "replace")
        for d in DYN.finditer(t):
            refusals.append([name, "non-literal loader %s" % d.group(0)[:60]])
        for rx in LIT:
            for s in rx.findall(t):
                child = "./assets/" + s.split("/")[-1]
                if child not in seen and child not in todo:
                    todo.append(child)
    return {"roots": roots, "pins": pins, "sizes": seen, "refusals": refusals}
