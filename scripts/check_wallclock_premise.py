#!/usr/bin/env python3
"""Wall-clock premise gate - fails when a test outside a frozen baseline rests its verdict on wall-clock timing.

Why this exists: the 0.14.56 SLEEP-PREMISE census (round/_evidence/0.14.56-R2/census/, pin 9da2418d) found 332 tests whose
verdict rests on a wall-clock premise: a sleep assumed long enough (or short enough), a measured duration asserted
against a bound, or a short bounded wait whose expiry fails the test. v0.14.55 was spent on one of them
(state::tests::inject_track_handoff_pending_until_an_arm_ends, release attempt 3). The census was a one-off evidence tool,
so nothing stopped the next premise from landing. This gate ports the census classifier rule for rule (scan.py,
classify.py, build_report.py; declared in 00-population-and-selectors.md and 02-addendum-pass2.md) and turns it into a
ratchet:
  * a PREMISE test (own, or inherited through a PREMISE helper) whose id is not in the baseline fails the check;
  * a baseline id that no longer exists or no longer classifies as a premise also fails ("remove from baseline"),
    so the baseline only shrinks.

Rules (identical to the census; the census is the source of truth and this port is proven equivalent to it at the pin):
  population   src/bin/cysd/**/*.rs · src/bin/cys.rs + src/bin/cys/**/*.rs · src/*.rs · cysjavis-pack/**/*.py
               (cysjavis-pack/**/*.sh is scanned for tokens only; shell has no test units). The census listed files with
               `git ls-files`; this gate walks src/ and cysjavis-pack/ on disk (same selectors, works on a plain tree).
  selectors    applied to code with comments and literals blanked (Rust lexer / Python tokenize), census §3 + pass 2.
  units        Rust `#[test]` / `#[tokio::test]` fns; Python `def test_*` and `*self_test*` fns.
  helpers      the enclosing fn of every SUPPORT-region token (cfg(test) code outside test fns; Python test files
               outside test units), classified with the same rubric; tests calling one by name inherit its class.
  classes      PREMISE-SLEEP · PREMISE-WINDOW · PREMISE-DEADLINE (premises) · GUARD · CLOCK-ONLY (not premises).
  overrides    the two id-keyed rules the census itself applies (CENSUS_OVERRIDES below), each only while the code it
               was evidence about is unchanged; printed whenever one fires or is withheld.

Test identity = repo-relative path + qualified name, never a line number:
  Rust    src/bin/cysd/state.rs::tests::some_test      (enclosing inline `mod` blocks of that file + fn name)
  Python  cysjavis-pack/tests/test_x.py::TestX.test_y  (enclosing classes / functions + def name)

Escape: a comment ON the deciding line printed for the test, with a non-empty reason, accepts that premise:
  Rust    assert!(done);  // wallclock: guard <reason>
  Python  self.assertTrue(done)  # wallclock: guard <reason>
Every accepted annotation is printed and counted. An annotation without a reason is not accepted.

Exit codes: 0 clean · 1 violations (new premise test, or stale baseline entry) · 2 cannot judge (missing or unreadable
baseline, unparsable source, scanner self-check failure). 2 is never a pass.

    python3 scripts/check_wallclock_premise.py [--root DIR] [--baseline FILE] [--write-baseline FILE]
"""
import argparse
import ast
import bisect
import hashlib
import io
import os
import re
import sys
import tokenize

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_ROOT = os.path.normpath(os.path.join(HERE, ".."))
BASELINE_REL = "scripts/wallclock-premise-baseline.txt"
EXIT_CLEAN, EXIT_VIOLATION, EXIT_CANNOT_JUDGE = 0, 1, 2
PREMISE_PRIORITY = {"PREMISE-SLEEP": 0, "PREMISE-WINDOW": 1, "PREMISE-DEADLINE": 2}
SCAN_TOPS = ("src", "cysjavis-pack")


class CannotJudge(Exception):
    """The tree could not be judged. Never read as a pass."""


# ======================= population (census scan.py area(), 00 §1) =======================
def area(p):
    if p.startswith("src/bin/cysd/") and p.endswith(".rs"):
        return "R-cysd"
    if (p == "src/bin/cys.rs" or p.startswith("src/bin/cys/")) and p.endswith(".rs"):
        return "R-cys"
    if re.fullmatch(r"src/[^/]+\.rs", p):
        return "R-lib"
    if p.startswith("cysjavis-pack/") and p.endswith(".py"):
        return "P-py"
    if p.startswith("cysjavis-pack/") and p.endswith(".sh"):
        return "P-sh"
    return None


class Tree:
    """The files under judgement: a directory on disk, or (scanner self-check) an in-memory {path: text} mapping."""

    def __init__(self, root=None, files=None):
        self.root, self.files = root, files
        if files is not None:
            self.paths = sorted(p for p in files if area(p))
            return
        found = []
        for top in SCAN_TOPS:
            for d, dirs, names in os.walk(os.path.join(root, top)):
                dirs[:] = sorted(x for x in dirs if x != ".git")
                for n in names:
                    rel = os.path.relpath(os.path.join(d, n), root).replace(os.sep, "/")
                    if area(rel):
                        found.append(rel)
        self.paths = sorted(found)

    def read(self, rel, strict):
        if self.files is not None:
            return self.files[rel]
        try:
            # census: Rust files are read strictly as UTF-8, Python and shell with errors="replace"
            with open(os.path.join(self.root, rel), encoding="utf-8", errors="strict" if strict else "replace") as f:
                return f.read()
        except (OSError, UnicodeDecodeError) as e:
            raise CannotJudge("cannot read %s: %s" % (rel, e))


# ======================= Rust lexer (census scan.py blank_rust, verbatim; also records `//` comments) =======================
def blank_rust(src, comments=None):
    out = list(src)
    n, i = len(src), 0

    def blank(a, b):
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            if comments is not None:
                comments.append((i, src[i:j]))
            blank(i, j); i = j; continue
        if src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1; j += 2
                elif src.startswith("*/", j):
                    depth -= 1; j += 2
                else:
                    j += 1
            blank(i, j); i = j; continue
        m = re.match(r'(b?r)(#*)"', src[i:i + 300]) if c in "rb" and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")) else None
        if m:
            hashes = m.group(2)
            start = i + m.end()
            end = src.find('"' + hashes, start)
            end = n if end < 0 else end + 1 + len(hashes)
            blank(i + m.end(), end - 1 - len(hashes)); i = end; continue
        if c == '"' or (c == "b" and i + 1 < n and src[i + 1] == '"' and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_"))):
            j = i + (2 if c == "b" else 1)
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            blank(i + 1, j); i = j + 1; continue
        if c == "'":
            # char literal vs lifetime
            if i + 1 < n and src[i + 1] == "\\":
                j = i + 2
                while j < n and src[j] != "'":
                    j += 1
                blank(i + 1, j); i = j + 1; continue
            if i + 2 < n and src[i + 2] == "'":
                blank(i + 1, i + 2); i = i + 3; continue
            i += 1; continue
        i += 1
    return "".join(out)


def find_close(code, open_idx):
    """Index of the '}' matching code[open_idx], or -1 when the braces never balance."""
    depth = 0
    for k in range(open_idx, len(code)):
        if code[k] == "{":
            depth += 1
        elif code[k] == "}":
            depth -= 1
            if depth == 0:
                return k
    return -1


def match_brace(code, open_idx):
    """census scan.py match_brace: the matching '}' or len(code)-1 when unbalanced."""
    k = find_close(code, open_idx)
    return len(code) - 1 if k < 0 else k


TEST_ATTR = re.compile(r"#\[\s*(?:[A-Za-z_]\w*::)*test\s*(?:\([^\]]*\))?\s*\]")
CFG_TEST = re.compile(r"#\[\s*cfg\s*\(([^\]]*\btest\b[^\]]*)\)\s*\]")
ATTR = re.compile(r"\s*#!?\[[^\]]*(?:\[[^\]]*\][^\]]*)*\]")
MOD_BLOCK = re.compile(r"\bmod\s+(\w+)\s*\{")

R_SEL = {
    "S-sleep": re.compile(r"\bsleep(?:_until)?\s*\("),
    "S-now": re.compile(r"Instant::now\b|SystemTime::now\b|\bnow_epoch\s*\("),
    "S-elapsed": re.compile(r"\.elapsed\s*\(|\bduration_since\s*\(|\bsaturating_duration_since\s*\("),
    "S-timeout": re.compile(r"\brecv_timeout\s*\(|\bwait_timeout(?:_while|_ms)?\s*\(|\btimeout\s*\(|\bdeadline\b"),
    "S-durassert": re.compile(r"Duration::from_(?:millis|secs|micros|nanos|secs_f64|secs_f32)\b"),
    "S-iotimeout": re.compile(r"\bset_(?:read|write)_timeout\s*\("),
    "S-nonblock": re.compile(r"\bset_nonblocking\s*\(\s*true"),
}
P_SEL = {
    "P-sleep": re.compile(r"\bsleep\s*\("),
    "P-clock": re.compile(r"\btime\.(?:monotonic|time|perf_counter)(?:_ns)?\s*\(|\bdatetime\.(?:datetime\.)?now\s*\("),
    "P-timeout": re.compile(r"\btimeout\s*=|\bdeadline\b"),
}
SH_SEL = {"SH-sleep": re.compile(r"\bsleep\s+[0-9.]"), "SH-timeout": re.compile(r"\btimeout\s+[0-9]")}


class RustFile:
    """One Rust file: source, blanked code, line tables, consts and `//` comments by line (each computed once)."""

    def __init__(self, rel, src):
        self.rel, self.src = rel, src
        found = []
        self.code = blank_rust(src, found)
        self.lines_c, self.lines_s = self.code.split("\n"), src.split("\n")
        self.starts = [0]
        for ln in self.lines_c:
            self.starts.append(self.starts[-1] + len(ln) + 1)
        self.nl = [k - 1 for k in self.starts[1:-1]]
        self.comments = {self.line_of(i): text for i, text in found}
        self.consts = {}
        for m in re.finditer(r"\bconst\s+([A-Z_][A-Z0-9_]*)\s*:\s*[^=;]+=\s*([^;]+);", self.code):
            self.consts[m.group(1)] = m.group(2).strip()
        self.mods = []
        for m in MOD_BLOCK.finditer(self.code):
            ob = m.end() - 1
            self.mods.append((ob, match_brace(self.code, ob), m.group(1)))

    def line_of(self, idx):
        return bisect.bisect_left(self.nl, idx) + 1

    def qual(self, idx, name):
        return "::".join([n for a, b, n in sorted(self.mods) if a <= idx <= b] + [name])


def scan_rust(rf, test_module_files, problems):
    """census scan.py scan_rust: cfg(test) regions and test fns (start/end = body braces)."""
    code, path = rf.code, rf.rel
    regions = []  # (start, end, kind, meta)
    for m in CFG_TEST.finditer(code):
        k = m.end()
        while True:
            a = ATTR.match(code, k)
            if not a:
                break
            k = a.end()
        rest = code[k:k + 400]
        mm = re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*([;{])", rest)
        if mm:
            if mm.group(2) == ";":
                test_module_files.add((os.path.dirname(path), mm.group(1)))
                continue
            ob = k + mm.end() - 1
            regions.append((ob, match_brace(code, ob), "mod", mm.group(1)))
            continue
        ob = code.find("{", k)
        semi = code.find(";", k)
        if ob >= 0 and (semi < 0 or ob < semi):
            regions.append((ob, match_brace(code, ob), "item", None))
    tests = []
    attr_count = 0
    for m in TEST_ATTR.finditer(code):
        attr_count += 1
        k = m.end()
        while True:
            a = ATTR.match(code, k)
            if not a:
                break
            k = a.end()
        fm = re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(\w+)", code[k:k + 400])
        if not fm:
            problems.append("%s:%d: #[test] attribute not followed by an fn (lexer control)" % (path, rf.line_of(m.start())))
            continue
        name = fm.group(1)
        ob = code.find("{", k + fm.end())
        cb = find_close(code, ob) if ob >= 0 else -1
        if cb < 0:
            problems.append("%s:%d: test fn %s has no balanced body (lexer control)" % (path, rf.line_of(k + fm.start(1)), name))
            continue
        tests.append({"name": name, "fn_line": rf.line_of(k + fm.start(1)), "start": ob, "end": cb, "qual": rf.qual(ob, name)})
    return regions, tests, attr_count


def classify_region(idx, regions, tests, file_is_test_module):
    for t in tests:
        if t["start"] <= idx <= t["end"]:
            return "TEST", t
    if file_is_test_module:
        return "SUPPORT", None
    for a, b, kind, meta in regions:
        if a <= idx <= b:
            return "SUPPORT", None
    return "PRODUCTION", None


# ======================= Python (census scan.py blank_py; also records comments) =======================
# The census out/ was produced with Python 3.9, where an f-string is ONE STRING token and is blanked whole. From 3.12
# tokenize splits it (FSTRING_START / MIDDLE / END with the {expression} tokens in between) and the census code blanked
# only FSTRING_MIDDLE, so the same scan.py yields 4662 tokens instead of 4658 on 3.12+ (f"{time.time_ns()}" in
# production code). This port blanks the whole f-string (and t-string) span on every version, which is the census's
# declared rule ("literals blanked") and reproduces its recorded output on 3.9, 3.12 and 3.14 alike.
_FSTRING_START = {getattr(tokenize, n) for n in ("FSTRING_START", "TSTRING_START") if hasattr(tokenize, n)}
_FSTRING_END = {getattr(tokenize, n) for n in ("FSTRING_END", "TSTRING_END") if hasattr(tokenize, n)}


def blank_py(src, comments=None):
    lines = src.split("\n")
    out = [list(l) for l in lines]
    try:
        toks = list(tokenize.generate_tokens(io.StringIO(src).readline))
    except (tokenize.TokenError, IndentationError, SyntaxError):
        return None

    def blank(start, end):
        (sr, sc), (er, ec) = start, end
        for r in range(sr, er + 1):
            row = out[r - 1]
            a = sc if r == sr else 0
            b = ec if r == er else len(row)
            for k in range(a, min(b, len(row))):
                row[k] = " "

    depth, f_start = 0, None
    for t in toks:
        if t.type == tokenize.COMMENT and comments is not None and depth == 0:
            comments[t.start[0]] = t.string
        if t.type in _FSTRING_START:
            if depth == 0:
                f_start = t.start
            depth += 1
        elif t.type in _FSTRING_END:
            depth -= 1
            if depth == 0:
                blank(f_start, t.end)
        elif t.type in (tokenize.STRING, tokenize.COMMENT) and depth == 0:
            blank(t.start, t.end)
    return "\n".join("".join(r) for r in out)


def is_py_unit(name):
    return name.startswith("test_") or bool(re.search(r"self_?test", name, re.I))


class PyFile:
    """One Python file: source, blanked code, AST, qualified names and comments by line (each computed once)."""

    def __init__(self, rel, src):
        self.rel, self.src = rel, src
        self.comments = {}
        self.blanked = blank_py(src, self.comments)
        self.lines_s = src.split("\n")
        self.tree = None
        if self.blanked is None:
            return
        try:
            self.tree = ast.parse(src)
        except SyntaxError as e:
            self.error = "ast %s" % e
            return
        self.code = self.blanked or src
        self.lines_c = self.code.split("\n")
        self.fns = [n for n in ast.walk(self.tree) if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))]
        self.by_pos = {}
        for n in self.fns:  # census classify_py keeps the LAST walk-order match of (lineno, name)
            self.by_pos[(n.lineno, n.name)] = n
        self.quals = {}
        stack = [(self.tree, [])]
        while stack:
            node, prefix = stack.pop()
            for ch in ast.iter_child_nodes(node):
                if isinstance(ch, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                    path = prefix + [ch.name]
                    if not isinstance(ch, ast.ClassDef):
                        self.quals[(ch.lineno, ch.name)] = ".".join(path)
                    stack.append((ch, path))
                else:
                    stack.append((ch, prefix))


# ======================= classifier (census classify.py, verbatim rules) =======================
TEARDOWN = re.compile(r"exited|try_wait|\bkill|pid_alive|\breap|\beof\b|\bEOF\b|terminate|send_signal|SIGKILL|SIGTERM|is_alive\(\)|poll\(\)\s*is\s*None|waitpid")
ASSERT_RS = re.compile(r"\b(?:debug_)?assert(?:_eq|_ne)?!\s*\(|\.expect\s*\(|panic!\s*\(")
ASSERT_ONLY_RS = re.compile(r"\b(?:debug_)?assert(?:_eq|_ne)?!\s*\(|panic!\s*\(")
DUR = re.compile(r"(?:Duration::from_(millis|secs|micros|nanos|secs_f64|secs_f32)|\b(ms|secs|millis)|\b(sleep_ms))\s*\(\s*([^()]*(?:\([^()]*\)[^()]*)*)\)")
WAIT_KINDS_RS = ("deadline-loop", "recv_timeout", "wait_timeout", "wait_timeout_while", "wait_timeout_ms", "timeout", "set_read_timeout", "set_write_timeout")
PY_ASSERT = re.compile(r"\bassert\b|self\.assert|self\.fail\(|raise AssertionError")


def eval_num(expr, consts, depth=0):
    e = expr.strip().replace("_", "") if re.fullmatch(r"[\d_]+(?:\.\d+)?", expr.strip()) else expr.strip()
    e = re.sub(r"\bas\s+\w+", "", e).strip()
    if depth > 4:
        return None
    if re.fullmatch(r"\d+(?:\.\d+)?", e):
        return float(e)
    m = re.fullmatch(r"([A-Z_][A-Z0-9_]*)", e)
    if m and m.group(1) in consts:
        return eval_num(consts[m.group(1)].replace("_", "") if re.fullmatch(r"[\d_]+", consts[m.group(1)]) else consts[m.group(1)], consts, depth + 1)
    m = re.fullmatch(r"(.+?)\s*([*+\-/])\s*(.+)", e)
    if m:
        a, b = eval_num(m.group(1), consts, depth + 1), eval_num(m.group(3), consts, depth + 1)
        if a is not None and b is not None:
            return {"*": a * b, "+": a + b, "-": a - b, "/": a / b if b else None}[m.group(2)]
    return None


def dur_ms(text, consts):
    """all durations on a line -> list of (ms or None, raw)."""
    out = []
    for m in DUR.finditer(text):
        unit = m.group(1) or m.group(2) or m.group(3)
        raw = m.group(0)
        v = eval_num(m.group(4), consts)
        if v is None:
            out.append((None, raw)); continue
        mult = {"millis": 1, "ms": 1, "sleep_ms": 1, "secs": 1000, "secs_f64": 1000, "secs_f32": 1000, "micros": 0.001, "nanos": 1e-6}[unit]
        out.append((v * mult, raw))
    return out


def rust_loops(code, a, b):
    """loops (while / loop / for) in code[a:b] -> list of dicts with header text, body span."""
    res = []
    for m in re.finditer(r"\b(while|loop|for)\b", code[a:b]):
        s = a + m.start()
        ob = code.find("{", s)
        if ob < 0 or ob > b:
            continue
        hdr = code[s:ob]
        if ";" in hdr:  # not a block header
            continue
        cb = match_brace(code, ob)
        res.append({"kind": m.group(1), "s": s, "ob": ob, "cb": cb, "hdr": hdr})
    return res


def classify_rust(rf, fn_line, end_line):
    """census classify.py classify_rust -> (class, reasons, constructs, deciding line or None)."""
    code, consts, lines_c, starts = rf.code, rf.consts, rf.lines_c, rf.starts
    a, b = starts[fn_line - 1], starts[end_line]
    body_lines = range(fn_line, end_line + 1)
    loops = rust_loops(code, a, b)
    clock_vars = set()
    for ln in body_lines:
        for m in re.finditer(r"let\s+(?:mut\s+)?(\w+)\s*(?::[^=]+)?=\s*[^;]*(?:\.elapsed\s*\(|duration_since\s*\()", lines_c[ln - 1]):
            clock_vars.add(m.group(1))
    max_delay = 0.0
    for ln in body_lines:
        t = lines_c[ln - 1]
        if re.search(r"delay_ms|cr_delay_ms|gap_ms|GAP|sleep|DataAfter", t):
            for v, _ in dur_ms(t, consts):
                if v:
                    max_delay = max(max_delay, v)
            for m in re.finditer(r"(?:delay_ms|cr_delay_ms|min_gap_ms)\s*:\s*([A-Z_0-9]+|\d[\d_]*)", t):
                v = eval_num(m.group(1).replace("_", "") if m.group(1)[0].isdigit() else m.group(1), consts)
                if v:
                    max_delay = max(max_delay, v)
    constructs = []

    def in_poll_loop(idx):
        for L in loops:
            if L["ob"] <= idx <= L["cb"]:
                txt = code[L["s"]:L["cb"]]
                if re.search(r"elapsed\s*\(|Instant::now|deadline|\bend\b|timeout|\bbreak\b|return\b", txt):
                    return L
        return None

    first_sleep_bare = None
    for ln in body_lines:
        t = lines_c[ln - 1]
        idx0 = starts[ln - 1]
        for m in re.finditer(r"\bsleep(?:_until)?\s*\(", t):
            L = in_poll_loop(idx0 + m.start())
            d = dur_ms(t[m.start():], consts)
            dv = d[0][0] if d else None
            if L:
                constructs.append({"kind": "poll-pacing", "line": ln, "ms": dv})
            else:
                constructs.append({"kind": "bare-sleep", "line": ln, "ms": dv})
                if first_sleep_bare is None:
                    first_sleep_bare = ln
    for L in loops:
        txt = code[L["s"]:L["cb"] + 1]
        hdr = L["hdr"]
        if not re.search(r"elapsed\s*\(|Instant::now|deadline|\bend\b|timeout|\bsleep", txt):
            continue
        if not re.search(r"\bsleep|recv_timeout|wait_timeout|elapsed\s*\(|Instant::now\s*\(\)\s*[<>]", txt):
            continue
        bound = None
        braw = None
        lnL = code.count("\n", 0, L["s"]) + 1
        m = re.search(r"elapsed\s*\(\s*\)\s*<=?\s*(.+)$", hdr.strip()) or re.search(r"Instant::now\s*\(\s*\)\s*<=?\s*(\w+)", hdr)
        if m:
            seg = m.group(1)
            ds = dur_ms(seg, consts)
            if ds:
                bound, braw = ds[0]
            else:
                var = re.match(r"\s*(\w+)", seg)
                if var:
                    for ln in body_lines:
                        mm = re.search(r"let\s+(?:mut\s+)?" + re.escape(var.group(1)) + r"\b[^=]*=\s*(.+);", lines_c[ln - 1])
                        if mm:
                            ds = dur_ms(mm.group(1), consts)
                            if ds:
                                bound, braw = ds[0]
                                break
        else:
            mf = re.search(r"for\s+\w+\s+in\s+0\s*\.\.=?\s*([\w_]+)", hdr)
            if mf:
                n = eval_num(mf.group(1).replace("_", "") if mf.group(1)[0].isdigit() else mf.group(1), consts)
                sl = [dur_ms(code[s.start():s.start() + 120], consts) for s in re.finditer(r"\bsleep\s*\(", txt)]
                sv = [x[0][0] for x in sl if x and x[0][0]]
                if n and sv:
                    bound, braw = n * sum(sv), f"{int(n)} x {sum(sv)} ms"
            if bound is None:
                mm = re.search(r"elapsed\s*\(\s*\)\s*>=?\s*([^{]+)\{", txt) or re.search(r"Instant::now\s*\(\s*\)\s*>=?\s*(\w+)", txt)
                if mm:
                    ds = dur_ms(mm.group(1), consts)
                    if ds:
                        bound, braw = ds[0]
                    else:
                        var = re.match(r"\s*(\w+)", mm.group(1))
                        if var:
                            for ln in body_lines:
                                m2 = re.search(r"let\s+(?:mut\s+)?" + re.escape(var.group(1)) + r"\b[^=]*=\s*(.+);", lines_c[ln - 1])
                                if m2:
                                    ds = dur_ms(m2.group(1), consts)
                                    if ds:
                                        bound, braw = ds[0]
                                        break
        if bound is None:  # pass 2b fallback: first `Instant::now() + <dur>` in the function body
            for ln in body_lines:
                mm = re.search(r"Instant::now\s*\(\s*\)\s*\+\s*(.+)", lines_c[ln - 1])
                if mm:
                    ds = dur_ms(mm.group(1), consts)
                    if ds and ds[0][0] is not None:
                        bound, braw = ds[0][0], ds[0][1] + " (fallback: Instant::now()+ at :%d)" % ln
                        break
        constructs.append({"kind": "deadline-loop", "line": lnL, "ms": bound, "raw": braw, "teardown": bool(TEARDOWN.search(txt))})
    for ln in body_lines:
        t = lines_c[ln - 1]
        for m in re.finditer(r"\b(recv_timeout|wait_timeout(?:_while|_ms)?|timeout|set_read_timeout|set_write_timeout)\s*\(", t):
            d = dur_ms(t[m.end():], consts)
            ctx = "\n".join(lines_c[max(ln - 4, 0):ln + 2])
            c = {"kind": m.group(1), "line": ln, "ms": d[0][0] if d else None, "raw": d[0][1] if d else None,
                 "teardown": bool(TEARDOWN.search(ctx))}
            # pass 2: a read timeout is inert when the socket is nonblocking (macOS accepted-socket inheritance)
            if m.group(1) == "set_read_timeout" and re.search(r"set_nonblocking\s*\(\s*true", code[a:b]) and not re.search(r"set_nonblocking\s*\(\s*false", code[a:b]):
                c["ms"], c["raw"], c["inert_nonblocking"] = 0.0, (c.get("raw") or "") + " [inert: nonblocking listener, effective 0 ms]", True
            constructs.append(c)
    for ln in body_lines:
        t = lines_c[ln - 1]
        if not ASSERT_ONLY_RS.search(t):
            continue
        uses_clock = re.search(r"\.elapsed\s*\(|duration_since\s*\(", t) or any(re.search(r"\b" + re.escape(v) + r"\b", t) for v in clock_vars)
        if uses_clock and re.search(r"[^<>=!-][<>]=?[^<>=]", t.replace("->", "")):
            ds = dur_ms(t, consts)
            constructs.append({"kind": "window-assert", "line": ln, "ms": ds[0][0] if ds else None, "raw": ds[0][1] if ds else None})
    asserts_after = []
    if first_sleep_bare is not None:
        for ln in range(first_sleep_bare + 1, end_line + 1):
            if ASSERT_RS.search(lines_c[ln - 1]):
                asserts_after.append(ln)
    reasons = []
    cls = None
    decide = None
    bare = [c for c in constructs if c["kind"] == "bare-sleep"]
    if bare and asserts_after:
        cls = "PREMISE-SLEEP"
        decide = asserts_after[0]  # addendum §3: the deciding line of a PREMISE-SLEEP is the first assert after it
        reasons.append(f"bare sleep at :{bare[0]['line']} ({bare[0]['ms']} ms) then assert at :{asserts_after[0]}")
    wins = [c for c in constructs if c["kind"] == "window-assert"]
    if wins:
        if cls is None:
            cls, decide = "PREMISE-WINDOW", wins[0]["line"]
        reasons.append(f"real-clock duration compared in assert at :{wins[0]['line']} (bound {wins[0].get('raw')})")
    waits = [c for c in constructs if c["kind"] in WAIT_KINDS_RS]
    tight = []
    for w in waits:
        why = []
        if w.get("teardown"):
            why.append("awaits OS-teardown state")
        if w["ms"] is not None and w["ms"] < 1000:
            why.append(f"bound {w['ms']:.0f} ms < 1 s")
        if w["ms"] is not None and max_delay and w["ms"] < 10 * max_delay:
            why.append(f"bound {w['ms']:.0f} ms < 10x explicit delay {max_delay:.0f} ms")
        w["tight_because"] = why
        if why:
            tight.append(w)
    if tight:
        if cls is None:
            cls, decide = "PREMISE-DEADLINE", tight[0]["line"]
        reasons.append(f"{tight[0]['kind']} at :{tight[0]['line']} bound {tight[0].get('raw') or tight[0]['ms']} — " + "; ".join(tight[0]["tight_because"]))
    if cls is None and (waits or bare):
        cls = "GUARD"
        if waits:
            w = waits[0]
            reasons.append(f"{w['kind']} at :{w['line']} bound {w.get('raw') or w['ms']} (>= 1 s, >= 10x explicit delays {max_delay:.0f} ms, no teardown)" + ("" if w["ms"] is not None else " [bound unresolved]"))
        else:
            reasons.append(f"bare sleep at :{bare[0]['line']} with no assert after it")
    if cls is None:
        cls = "CLOCK-ONLY"
        reasons.append("clock reads/injection or Duration values only; no sleep, no wait, no real-clock window assert")
    if any(c["kind"] == "poll-pacing" for c in constructs) and not waits:
        reasons.append("poll-pacing sleep inside a loop whose bound was not recognised")
    return cls, reasons, constructs, decide


def classify_py(pf, name, fn_line):
    """census classify.py classify_py -> (class, reasons, constructs, deciding line or None)."""
    src, lines_c = pf.src, pf.lines_c
    fn = pf.by_pos.get((fn_line, name))
    constructs, reasons = [], []
    loops = [n for n in ast.walk(fn) if isinstance(n, (ast.While, ast.For))] if fn else []

    def loop_of(ln):
        for L in loops:
            if L.lineno <= ln <= L.end_lineno:
                seg = "\n".join(lines_c[L.lineno - 1:L.end_lineno])
                if re.search(r"time\.(monotonic|time|perf_counter)|deadline|timeout|\bbreak\b|return\b", seg):
                    return L
        return None

    def num(node):
        try:
            return float(ast.literal_eval(node))
        except Exception:
            return None

    calls = [n for n in ast.walk(fn) if isinstance(n, ast.Call)] if fn else []
    bare_lines = []
    for c in calls:
        f = c.func
        cname = f.attr if isinstance(f, ast.Attribute) else (f.id if isinstance(f, ast.Name) else "")
        if cname == "sleep":
            v = num(c.args[0]) if c.args else None
            L = loop_of(c.lineno)
            k = "poll-pacing" if L else "bare-sleep"
            constructs.append({"kind": k, "line": c.lineno, "ms": None if v is None else v * 1000})
            if k == "bare-sleep":
                bare_lines.append(c.lineno)
        for kw in c.keywords:
            if kw.arg == "timeout":
                v = num(kw.value)
                ctx = "\n".join(lines_c[max(c.lineno - 3, 0):c.lineno + 1])
                constructs.append({"kind": "timeout=", "line": c.lineno, "ms": None if v is None else v * 1000, "raw": ast.get_source_segment(src, kw.value),
                                   "call": cname, "teardown": bool(TEARDOWN.search(ctx))})
    for L in loops:
        seg = "\n".join(lines_c[L.lineno - 1:L.end_lineno])
        if re.search(r"time\.(monotonic|time|perf_counter)|deadline", seg) and re.search(r"sleep|monotonic|time\.time", seg):
            m = re.search(r"(?:deadline|end|until|limit)\s*=\s*time\.(?:monotonic|time|perf_counter)\(\)\s*\+\s*([\d.]+)", "\n".join(lines_c[fn.lineno - 1:fn.end_lineno]))
            m2 = re.search(r"time\.(?:monotonic|time|perf_counter)\(\)\s*-\s*\w+\s*<=?\s*([\d.]+)", seg)
            v = float((m or m2).group(1)) * 1000 if (m or m2) else None
            constructs.append({"kind": "deadline-loop", "line": L.lineno, "ms": v, "raw": (m or m2).group(0) if (m or m2) else None, "teardown": bool(TEARDOWN.search(seg))})
    body = range(fn.lineno, fn.end_lineno + 1) if fn else range(0)
    clockv = set()
    for ln in body:
        m = re.search(r"(\w+)\s*=\s*time\.(?:monotonic|time|perf_counter)\(\)\s*-", lines_c[ln - 1])
        if m:
            clockv.add(m.group(1))
    for ln in body:
        t = lines_c[ln - 1]
        if PY_ASSERT.search(t) and (re.search(r"time\.(monotonic|time|perf_counter)\(\)\s*-", t) or any(re.search(r"\b" + re.escape(v) + r"\b", t) for v in clockv)):
            if re.search(r"assertLess|assertGreater|assertLessEqual|assertGreaterEqual|[<>]", t):
                constructs.append({"kind": "window-assert", "line": ln, "ms": None, "raw": t.strip()[:80]})
    asserts_after = []
    if bare_lines:
        for ln in range(min(bare_lines) + 1, (fn.end_lineno if fn else 0) + 1):
            if PY_ASSERT.search(lines_c[ln - 1]):
                asserts_after.append(ln)
    cls = None
    decide = None
    bare = [c for c in constructs if c["kind"] == "bare-sleep"]
    if bare and asserts_after:
        cls, decide = "PREMISE-SLEEP", asserts_after[0]
        reasons.append(f"bare sleep at :{bare[0]['line']} ({bare[0]['ms']} ms) then assert at :{asserts_after[0]}")
    wins = [c for c in constructs if c["kind"] == "window-assert"]
    if wins:
        if cls is None:
            cls, decide = "PREMISE-WINDOW", wins[0]["line"]
        reasons.append(f"real-clock duration compared in assert at :{wins[0]['line']}")
    waits = [c for c in constructs if c["kind"] in ("timeout=", "deadline-loop")]
    tight = []
    for w in waits:
        why = []
        if w.get("teardown"):
            why.append("awaits OS-teardown state")
        floor = 10000 if w.get("call") in ("run", "communicate", "wait", "check_output", "call", "check_call") and not w.get("teardown") else 1000
        if w["ms"] is not None and w["ms"] < floor:
            why.append(f"bound {w['ms']:.0f} ms < {floor // 1000} s ({'process-completion' if floor == 10000 else 'in-process'} floor x10)")
        w["tight_because"] = why
        if why:
            tight.append(w)
    if tight:
        if cls is None:
            cls, decide = "PREMISE-DEADLINE", tight[0]["line"]
        reasons.append(f"{tight[0]['kind']} at :{tight[0]['line']} bound {tight[0].get('raw') or tight[0]['ms']} — " + "; ".join(tight[0]["tight_because"]))
    if cls is None and (waits or bare):
        cls = "GUARD"
        if waits:
            w = waits[0]; reasons.append(f"{w['kind']} at :{w['line']} bound {w.get('raw') or w['ms']} (>= 1 s, no teardown)" + ("" if w["ms"] is not None else " [bound unresolved]"))
        else:
            reasons.append(f"bare sleep at :{bare[0]['line']} with no assert after it")
    if cls is None:
        cls = "CLOCK-ONLY"; reasons.append("clock reads only; no sleep, no wait, no real-clock window assert")
    return cls, reasons, constructs, decide


# ======================= census overrides (the two id-keyed rules the census itself applies) =======================
# Each one is evidence about one specific body at the pin, so it applies only while that evidence still describes the
# code: the runner-failed helper while its body (comments and literals blanked, whitespace collapsed) is unchanged, the
# child-process parent while its source still names the child. A fixed helper therefore leaves the premise set (and its
# callers' baseline entries go stale) instead of staying a premise by name forever. At the pin both conditions hold.
CENSUS_OVERRIDES = {
    # classify.py RUNNER_FAILED_HELPERS: rubric §4 "PREMISE-DEADLINE ... OR it has already failed on a runner".
    "runner_failed_helpers": {
        ("src/bin/cysd/agent_queue_tests.rs", "wait_until"): {
            "body_sha16": "b957b23de4f35924",
            "reason": "already failed on a runner: windows-health cysd-win 3 runs, `panicked at agent_queue_tests.rs:84:9: not within 10 s: resumed attempt 1 settled` (bound 10 s; GUARD by the 10x rule, PREMISE by the failed-on-a-runner clause)",
        },
    },
    # build_report.py child-process inheritance: the parent test runs the child test as a subprocess and asserts on its
    # result, which name-call attribution cannot see. The parent inherits the child's inherited premise.
    "child_process_parents": [
        ("src/bin/cys/consumer_tests.rs", "p2_recorder_peer", "p2_base_dept_marker_recorder",
         "consumer_tests.rs runs b360_tests::p2_consumer_tests::p2_recorder_peer as a child process and asserts on it"),
    ],
}


def body_sha16(code_text):
    return hashlib.sha256(" ".join(code_text.split()).encode("utf-8")).hexdigest()[:16]


# ======================= the census pipeline over one tree =======================
def rust_fns(code):
    out = []
    for m in re.finditer(r"\bfn\s+(\w+)", code):
        ob = code.find("{", m.end())
        semi = code.find(";", m.end())
        if ob < 0 or (0 <= semi < ob):
            continue
        out.append((m.group(1), m.start(), ob, match_brace(code, ob)))
    return out


def _stem(f):
    return os.path.splitext(os.path.basename(f))[0]


def analyze(tree):
    """Run scan.py -> classify.py -> build_report.py (premise rows only) on a Tree. Returns a dict of results."""
    problems = []
    pop = {k: [] for k in ("R-cysd", "R-cys", "R-lib", "P-py", "P-sh")}
    for p in tree.paths:
        pop[area(p)].append(p)
    if not tree.paths:
        raise CannotJudge("empty population: no %s files under %s (wrong --root?)" % ("/".join(SCAN_TOPS), tree.root))
    tok_rows, hits = [], []
    rust_files = pop["R-cysd"] + pop["R-cys"] + pop["R-lib"]
    rfs, scanned, test_module_files = {}, {}, set()
    for p in rust_files:
        rfs[p] = RustFile(p, tree.read(p, strict=True))
        scanned[p] = scan_rust(rfs[p], test_module_files, problems)
    tm_paths = set()
    for d, name in test_module_files:
        for cand in (f"{d}/{name}.rs", f"{d}/{name}/mod.rs"):
            if cand in scanned:
                tm_paths.add(cand)
    rust_tests = 0
    for p in rust_files:
        rf = rfs[p]
        regions, tests, attr_count = scanned[p]
        rust_tests += len(tests)
        is_tm = p in tm_paths
        per_test = {}
        for sel, rx in R_SEL.items():
            for m in rx.finditer(rf.code):
                ln = rf.line_of(m.start())
                if sel == "S-durassert" and "assert" not in rf.lines_c[ln - 1]:
                    continue
                reg, t = classify_region(m.start(), regions, tests, is_tm)
                tok_rows.append({"file": p, "area": area(p), "line": ln, "sel": sel, "region": reg})
                if reg == "TEST":
                    per_test.setdefault(t["name"], []).append((ln, sel))  # census keys TEST tokens by bare fn name
        for t in tests:
            if t["name"] in per_test:
                hits.append({"file": p, "area": area(p), "test": t["name"], "qual": t["qual"], "fn_line": t["fn_line"],
                             "end_line": rf.line_of(t["end"]), "n_tokens": len(per_test[t["name"]])})
    pfs = {}
    py_units = 0
    for p in pop["P-py"]:
        pf = PyFile(p, tree.read(p, strict=False))
        if pf.blanked is None:
            problems.append("%s: tokenize failed (unparsable source)" % p); continue
        if pf.tree is None:
            problems.append("%s: %s (unparsable source)" % (p, pf.error)); continue
        pfs[p] = pf
        units = [(n.lineno, n.end_lineno, n.name) for n in pf.fns if is_py_unit(n.name)]
        py_units += len(units)
        is_testfile = bool(re.search(r"(^|/)test_[^/]*\.py$|_test\.py$|/tests/", p))
        clines = pf.blanked.split("\n")
        per = {}
        for sel, rx in P_SEL.items():
            for i, cl in enumerate(clines, 1):
                for m in rx.finditer(cl):
                    inner = [u for u in units if u[0] <= i <= u[1]]
                    if inner:
                        u = min(inner, key=lambda u: u[1] - u[0])  # innermost unit
                        reg = "TEST"
                        per.setdefault((u[0], u[2], u[1]), []).append((i, sel))
                    else:
                        reg = "SUPPORT" if is_testfile else "PRODUCTION"
                    tok_rows.append({"file": p, "area": "P-py", "line": i, "sel": sel, "region": reg})
        for (l0, name, l1), toks in sorted(per.items()):
            hits.append({"file": p, "area": "P-py", "test": name, "qual": pf.quals[(l0, name)], "fn_line": l0, "end_line": l1, "n_tokens": len(toks)})
    for p in pop["P-sh"]:
        is_t = "test" in os.path.basename(p)
        for i, l in enumerate(tree.read(p, strict=False).split("\n"), 1):
            code = re.sub(r"(^|\s)#.*$", "", l)
            for sel, rx in SH_SEL.items():
                for m in rx.finditer(code):
                    tok_rows.append({"file": p, "area": "P-sh", "line": i, "sel": sel, "region": "TEST" if is_t else "PRODUCTION"})
    if problems:
        raise CannotJudge("; ".join(problems[:10]) + (" (+%d more)" % (len(problems) - 10) if len(problems) > 10 else ""))

    # ---- classify.py pass 1: every hit ----
    for h in hits:
        try:
            if h["area"] == "P-py":
                cls, reasons, cons, dec = classify_py(pfs[h["file"]], h["test"], h["fn_line"])
            else:
                cls, reasons, cons, dec = classify_rust(rfs[h["file"]], h["fn_line"], h["end_line"])
        except Exception as e:  # the census never hides a classifier error; a gate cannot judge past one
            raise CannotJudge("classifier error on %s::%s: %r" % (h["file"], h["qual"], e))
        h.update({"class": cls, "reasons": reasons, "constructs": cons, "decide": dec})

    # ---- classify.py pass 2: helper attribution ----
    by_file = {}
    for t in tok_rows:
        if t["region"] == "SUPPORT":
            by_file.setdefault(t["file"], []).append(t)
    helpers = {}
    all_tests = {p: scanned[p][1] for p in rust_files}
    for f in rust_files:
        if f not in by_file:
            continue
        rf = rfs[f]
        fns = rust_fns(rf.code)
        for t in by_file[f]:
            idx = rf.starts[t["line"] - 1]
            enc = [x for x in fns if x[2] <= idx <= x[3]]
            if not enc:
                continue
            name, fs, ob, cb = min(enc, key=lambda x: x[3] - x[2])
            key = (f, name, rf.line_of(fs))
            helpers.setdefault(key, {"file": f, "test": name, "qual": rf.qual(ob, name), "fn_line": key[2], "end_line": rf.line_of(cb), "area": t["area"], "tokens": 0,
                                     "body": rf.code[ob:cb + 1]})
            helpers[key]["tokens"] += 1
    for f, ts in by_file.items():
        if not f.endswith(".py"):
            continue
        pf = pfs[f]
        for t in ts:
            enc = [n for n in pf.fns if n.lineno <= t["line"] <= n.end_lineno]
            if not enc:
                continue
            n = min(enc, key=lambda n: n.end_lineno - n.lineno)
            key = (f, n.name, n.lineno)
            helpers.setdefault(key, {"file": f, "test": n.name, "qual": pf.quals[(n.lineno, n.name)], "fn_line": n.lineno, "end_line": n.end_lineno, "area": "P-py", "tokens": 0,
                                     "body": "\n".join(pf.lines_c[n.lineno - 1:n.end_lineno])})
            helpers[key]["tokens"] += 1
    runner_failed = CENSUS_OVERRIDES["runner_failed_helpers"]
    overrides_fired = []
    hrows = []
    for key, hh in sorted(helpers.items()):
        try:
            if hh["area"] == "P-py":
                cls, reasons, cons, dec = classify_py(pfs[hh["file"]], hh["test"], hh["fn_line"])
            else:
                cls, reasons, cons, dec = classify_rust(rfs[hh["file"]], hh["fn_line"], hh["end_line"])
        except Exception as e:  # census: report, never hide
            cls, reasons, cons, dec = "UNCLASSIFIED", [f"classifier error: {e!r}"], [], None
        ov = runner_failed.get((hh["file"], hh["test"]))
        if ov and body_sha16(hh["body"]) != ov["body_sha16"]:
            overrides_fired.append("runner-failed override NOT applied to %s::%s: body changed since the census pin (%s != %s); classified by the rules as %s"
                                   % (hh["file"], hh["qual"], body_sha16(hh["body"]), ov["body_sha16"], cls))
        elif ov:
            reasons = [ov["reason"]] + reasons
            if not cls.startswith("PREMISE"):
                waits = [c for c in cons if c["kind"] in WAIT_KINDS_RS + ("timeout=",)]
                dec = waits[0]["line"] if waits else (cons[0]["line"] if cons else hh["fn_line"])
            cls = "PREMISE-DEADLINE"
            overrides_fired.append("runner-failed helper %s::%s forced PREMISE-DEADLINE (census classify.py RUNNER_FAILED_HELPERS)" % (hh["file"], hh["qual"]))
        del hh["body"]
        hh.update({"class": cls, "reasons": reasons, "constructs": cons, "decide": dec, "callers": [], "via": []})
        hrows.append(hh)
    py_tests = {}
    for f in {h["file"] for h in hrows if h["area"] == "P-py"}:
        pf = pfs[f]
        py_tests[f] = [(n.name, n.lineno, n.end_lineno, pf.quals[(n.lineno, n.name)]) for n in pf.fns if is_py_unit(n.name)]

    def callers_of(h):
        out = []
        name = h["test"]
        if h["area"] == "P-py":
            L = pfs[h["file"]].lines_s
            rx = re.compile(r"(?<![\w.])(?:self\.|cls\.)?" + re.escape(name) + r"\s*\(")
            for tn, l0, l1, q in py_tests.get(h["file"], []):
                body = "\n".join(L[l0:l1])  # census: raw source lines, the def line excluded
                if name in body and rx.search(body):
                    out.append({"file": h["file"], "test": tn, "fn_line": l0, "qual": q})
            return out
        same = re.compile(r"(?<![\w])" + re.escape(name) + r"\s*\(")
        cross = re.compile(r"([\w:]+)::" + re.escape(name) + r"\s*\(")
        hstem = _stem(h["file"])
        hdir = os.path.basename(os.path.dirname(h["file"]))
        for f, tests in all_tests.items():
            code = rfs[f].code
            for t in tests:
                body = code[t["start"]:t["end"]]
                if name not in body:
                    continue
                if f == h["file"]:
                    if same.search(body):
                        out.append({"file": f, "test": t["name"], "fn_line": rfs[f].line_of(t["start"]), "qual": t["qual"]})
                else:
                    for m in cross.finditer(body):
                        if hstem in m.group(1).split("::") or (hstem == "mod" and hdir in m.group(1).split("::")):
                            out.append({"file": f, "test": t["name"], "fn_line": rfs[f].line_of(t["start"]), "qual": t["qual"]}); break
        return out

    for h in hrows:
        h["callers"] = callers_of(h)
    # helper -> helper (same file) fixpoint, census order and round cap kept exactly
    same_file = {}
    for h in hrows:
        if h["area"] != "P-py":
            same_file.setdefault(h["file"], []).append(h)
    changed, rounds = True, 0
    while changed and rounds < 5:
        changed, rounds = False, rounds + 1
        for h in hrows:
            if h["area"] == "P-py":
                continue
            rf = rfs[h["file"]]
            rx = re.compile(r"(?<![\w])" + re.escape(h["test"]) + r"\s*\(")
            for g in same_file[h["file"]]:
                if g is h:
                    continue
                gbody = rf.code[rf.starts[g["fn_line"] - 1]:rf.starts[g["end_line"]]]
                if h["test"] in gbody and rx.search(gbody) and g["test"] not in h["via"]:
                    h["via"].append(g["test"])
                    for c in g["callers"]:
                        if c not in h["callers"]:
                            h["callers"].append(c); changed = True
    inh = []
    for h in hrows:
        if not h["class"].startswith("PREMISE"):
            continue
        for c in h["callers"]:
            inh.append({"file": c["file"], "test": c["test"], "qual": c["qual"], "fn_line": c["fn_line"], "helper": h, "class": h["class"]})
    # build_report.py child-process inheritance
    child_rows = []
    hit_names = {(r["file"], r["test"]) for r in hits}
    for cfile, child, parent, note in CENSUS_OVERRIDES["child_process_parents"]:
        peer = next((r for r in inh if r["file"] == cfile and r["test"] == child), None)
        if peer is None or (cfile, parent) in hit_names or any(r["file"] == cfile and r["test"] == parent for r in inh):
            continue
        ptest = next((t for t in all_tests.get(cfile, []) if t["name"] == parent), None)
        if ptest is None or child not in rfs[cfile].src[ptest["start"]:ptest["end"] + 1]:
            overrides_fired.append("child-process override NOT applied: %s::%s no longer exists or no longer names %s" % (cfile, parent, child))
            continue
        child_rows.append({"file": cfile, "test": parent, "qual": ptest["qual"], "fn_line": rfs[cfile].line_of(ptest["start"]),
                           "helper": peer["helper"], "class": peer["class"], "via_child": "%s::%s" % (cfile, peer["qual"]), "note": note})
        overrides_fired.append("child-process parent %s::%s inherits %s from %s::%s (census build_report.py)" % (cfile, ptest["qual"], peer["class"], cfile, peer["qual"]))
    files = dict(rfs)
    files.update(pfs)
    all_ids = {"%s::%s" % (p, t["qual"]) for p in rust_files for t in all_tests[p]}
    all_ids |= {"%s::%s" % (p, pf.quals[(n.lineno, n.name)]) for p, pf in pfs.items() for n in pf.fns if is_py_unit(n.name)}
    return {"hits": hits, "helpers": hrows, "inherited": inh, "child": child_rows, "tokens": tok_rows, "files": files, "all_ids": all_ids,
            "rust_tests": rust_tests, "py_units": py_units, "population": {k: len(v) for k, v in pop.items()},
            "overrides_fired": overrides_fired}


# ======================= premise rows per test id, escape annotations =======================
ANNOT_RS = re.compile(r"^//+\s*wallclock:\s*guard\b(.*)$")
ANNOT_PY = re.compile(r"^#+\s*wallclock:\s*guard\b(.*)$")


def annotation(files, path, line):
    """(found, reason) for a `wallclock: guard <reason>` comment on path:line."""
    fd = files.get(path)
    if fd is None or line is None:
        return False, ""
    text = fd.comments.get(line)
    if text is None:
        return False, ""
    m = (ANNOT_PY if path.endswith(".py") else ANNOT_RS).match(text.strip())
    if not m:
        return False, ""
    return True, m.group(1).strip(" \t:-")


def quote(files, path, line):
    fd = files.get(path)
    if fd is None or line is None or not (1 <= line <= len(fd.lines_s)):
        return ""
    return fd.lines_s[line - 1].strip()[:200]


def premise_tests(res):
    """{test id: {"file", "rows": [...]}}. A row = one premise the test rests on (own, inherited, child process)."""
    files = res["files"]
    out = {}

    def add(file, qual, row):
        tid = "%s::%s" % (file, qual)
        ent = out.setdefault(tid, {"id": tid, "file": file, "rows": []})
        found, reason = annotation(files, row["decide_file"], row["decide_line"])
        row["annotated"], row["accepted"] = found, (reason if found and reason else None)
        row["quote"] = quote(files, row["decide_file"], row["decide_line"])
        ent["rows"].append(row)

    for h in res["hits"]:
        if h["class"].startswith("PREMISE"):
            add(h["file"], h["qual"], {"kind": "own", "class": h["class"], "decide_file": h["file"], "decide_line": h["decide"],
                                        "reason": h["reasons"][0], "via": None})
    for r in res["inherited"] + res["child"]:
        hp = r["helper"]
        via = "%s::%s" % (hp["file"], hp["qual"]) + (" via child process %s" % r["via_child"] if r.get("via_child") else "")
        add(r["file"], r["qual"], {"kind": "child" if r.get("via_child") else "inherited", "class": r["class"], "decide_file": hp["file"],
                                    "decide_line": hp["decide"], "reason": hp["reasons"][0], "via": via})
    for ent in out.values():
        ent["rows"].sort(key=lambda r: (r["kind"] != "own", PREMISE_PRIORITY[r["class"]], r["via"] or ""))
        live = [r for r in ent["rows"] if not r["accepted"]]
        ent["live"] = live[0] if live else None
    return out


# ======================= scanner self-check (positive and negative controls, in memory) =======================
SELF_CHECK_FILES = {
    "src/bin/cysd/selfcheck_tests.rs": (
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    use std::time::{Duration, Instant};\n"
        "    fn settle(rx: &std::sync::mpsc::Receiver<u8>) -> u8 {\n"
        "        rx.recv_timeout(Duration::from_millis(200)).expect(\"settled\")\n"
        "    }\n"
        "    #[test]\n"
        "    fn sleeps_then_asserts() {\n"
        "        let brace = \"{\";\n"
        "        std::thread::sleep(Duration::from_millis(50));\n"
        "        assert!(brace.len() == 1);\n"
        "    }\n"
        "    #[test]\n"
        "    fn guarded_by_a_long_deadline() {\n"
        "        let deadline = Instant::now() + Duration::from_secs(60);\n"
        "        while Instant::now() < deadline {\n"
        "            std::thread::sleep(Duration::from_millis(10));\n"
        "            break;\n"
        "        }\n"
        "    }\n"
        "    #[test]\n"
        "    fn inherits_from_a_helper() {\n"
        "        let (_tx, rx) = std::sync::mpsc::channel();\n"
        "        let _ = settle(&rx);\n"
        "    }\n"
        "}\n"),
    "cysjavis-pack/tests/test_selfcheck.py": (
        "import time\n"
        "import unittest\n\n\n"
        "class T(unittest.TestCase):\n"
        "    def test_sleeps_then_asserts(self):\n"
        "        time.sleep(0.2)  # a comment with sleep( in it\n"
        "        self.assertEqual(1, 1)\n"),
}
SELF_CHECK_EXPECT = {
    "src/bin/cysd/selfcheck_tests.rs::tests::sleeps_then_asserts": "PREMISE-SLEEP",
    "src/bin/cysd/selfcheck_tests.rs::tests::inherits_from_a_helper": "PREMISE-DEADLINE",
    "cysjavis-pack/tests/test_selfcheck.py::T.test_sleeps_then_asserts": "PREMISE-SLEEP",
}


def self_check():
    res = analyze(Tree(files=SELF_CHECK_FILES))
    got = {tid: e["live"]["class"] for tid, e in premise_tests(res).items() if e["live"]}
    guard = [h for h in res["hits"] if h["qual"] == "tests::guarded_by_a_long_deadline" and h["class"] == "GUARD"]
    if got != SELF_CHECK_EXPECT or not guard or res["rust_tests"] != 3:
        raise CannotJudge("scanner self-check failed: expected %r and one GUARD hit, got %r (GUARD hits %d, rust tests %d)"
                          % (SELF_CHECK_EXPECT, got, len(guard), res["rust_tests"]))


# ======================= baseline and verdict =======================
BASELINE_HEADER = (
    "# Wall-clock premise baseline for scripts/check_wallclock_premise.py: tests tolerated until fixed.\n"
    "# One test id per line (repo-relative path :: qualified test name), sorted. The check fails on a premise test\n"
    "# that is not listed, and on a listed id that is no longer a premise (remove it: this list only shrinks).\n"
    "# Regenerate: python3 scripts/check_wallclock_premise.py --write-baseline scripts/wallclock-premise-baseline.txt\n"
)


def read_baseline(path):
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except (OSError, UnicodeDecodeError) as e:
        raise CannotJudge("baseline missing or unreadable: %s (%s)" % (path, e))
    return [(i, s.strip()) for i, s in enumerate(text.split("\n"), 1) if s.strip() and not s.strip().startswith("#")]


def row_text(r):
    cls = r["class"] if r["kind"] == "own" else "INHERITED:%s via %s" % (r["class"], r["via"])
    return '%s:%s  %s  "%s"  (%s)' % (r["decide_file"], r["decide_line"], cls, r["quote"], r["reason"])


def summary(n_premise, n_base_ok, n_new, n_stale, n_acc, n_rej, verdict):
    return ("summary: premise tests %d (baselined %d, new %d); stale baseline entries %d; accepted annotations %d; "
            "rejected annotations %d -> %s" % (n_premise, n_base_ok, n_new, n_stale, n_acc, n_rej, verdict))


def run(root, baseline_path, write_path):
    self_check()
    res = analyze(Tree(root=root))
    tests = premise_tests(res)
    for note in res["overrides_fired"]:
        print("census override: %s" % note)
    accepted = [(tid, r) for tid, e in sorted(tests.items()) for r in e["rows"] if r["accepted"]]
    rejected = [(tid, r) for tid, e in sorted(tests.items()) for r in e["rows"] if r["annotated"] and not r["accepted"]]
    for tid, r in accepted:
        print('ACCEPTED  %s:%s  %s  %s  reason: "%s"' % (r["decide_file"], r["decide_line"], tid, r["class"], r["accepted"]))
    live = {tid: e for tid, e in tests.items() if e["live"]}
    if write_path:
        with open(write_path, "w", encoding="utf-8", newline="\n") as f:
            f.write(BASELINE_HEADER + "".join(tid + "\n" for tid in sorted(live)))
        print("wrote %d premise test ids to %s" % (len(live), write_path))
        print(summary(len(live), 0, 0, 0, len(accepted), len(rejected), "baseline written (exit 0)"))
        return EXIT_CLEAN
    entries = read_baseline(baseline_path)
    listed = {tid for _, tid in entries}
    new = sorted(tid for tid in live if tid not in listed)
    stale = []
    for ln, tid in entries:
        if tid in live:
            continue
        if tid in tests:
            r = tests[tid]["rows"][0]
            why = "every premise is accepted by an annotation (first at %s:%s)" % (r["decide_file"], r["decide_line"])
        else:
            hit = next((h for h in res["hits"] if "%s::%s" % (h["file"], h["qual"]) == tid), None)
            if hit:
                why = "no longer a premise (now %s at %s:%d)" % (hit["class"], hit["file"], hit["fn_line"])
            elif tid in res["all_ids"]:
                why = "no longer a premise (no timing token left)"
            else:
                why = "no such test any more"
        stale.append((ln, tid, why))
    for tid in new:
        r = live[tid]["live"]
        note = "  [annotation on this line has no reason: not accepted]" if r["annotated"] else ""
        print("NEW       %s  %s%s" % (tid, row_text(r), note))
    for ln, tid, why in stale:
        print("STALE     %s:%d  %s  %s - remove from baseline" % (os.path.relpath(baseline_path, root) if os.path.isabs(baseline_path) else baseline_path, ln, tid, why))
    for tid, r in rejected:
        if tid not in new:
            print('REJECTED  %s:%s  %s  annotation without a reason is not accepted' % (r["decide_file"], r["decide_line"], tid))
    bad = bool(new or stale)
    print(summary(len(live), len(live) - len(new), len(new), len(stale), len(accepted), len(rejected),
                  "FAIL (exit 1)" if bad else "PASS (exit 0)"))
    return EXIT_VIOLATION if bad else EXIT_CLEAN


def main(argv=None):
    ap = argparse.ArgumentParser(description="Fail when a test outside the baseline rests its verdict on a wall-clock premise.")
    ap.add_argument("--root", default=DEFAULT_ROOT, help="tree to judge (default: the repo root, the parent of scripts/)")
    ap.add_argument("--baseline", help="baseline file (default: <root>/%s)" % BASELINE_REL)
    ap.add_argument("--write-baseline", metavar="FILE", help="write the current premise test ids to FILE and exit")
    a = ap.parse_args(argv)
    for stream in (sys.stdout, sys.stderr):  # quoted source lines are not ASCII; a cp1252 console must not crash the gate
        try:
            stream.reconfigure(errors="backslashreplace")
        except (AttributeError, ValueError):
            pass
    root = os.path.abspath(a.root)
    baseline = a.baseline or os.path.join(root, BASELINE_REL)
    try:
        if sys.version_info < (3, 8):
            raise CannotJudge("python >= 3.8 is required (ast end_lineno)")
        return run(root, baseline, a.write_baseline)
    except CannotJudge as e:
        print("cannot judge: %s" % e, file=sys.stderr)
        print(summary(0, 0, 0, 0, 0, 0, "CANNOT JUDGE (exit 2): %s" % str(e)[:300]))
        return EXIT_CANNOT_JUDGE


if __name__ == "__main__":
    sys.exit(main())
