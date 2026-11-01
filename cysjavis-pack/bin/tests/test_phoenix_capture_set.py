#!/usr/bin/env python3
"""0.14.57 P5 — the generation capture set comes from a predicate, with a control per pattern per run.

The ruling: an enumeration always lags and a new path then passes as lawful (`*soul.md` matched 0 of 70 generations,
the 2026-10-16 bytes are unrecoverable). So: a predicate over bounded roots, a positive control per pattern per run,
and a pattern matching 0 across all generations is a FAILURE, visible in the last 3 output lines, because the
phoenix-snapshot-6h job keeps only `2>&1 | tail -3`. Scope: what the tag captured plus soul.md at the pack root. No measured
need, no widening.

Whole runs go through the real CLI in a throw-away HOME with stdout and stderr merged into one pipe, as the job does.
  python3 test_phoenix_capture_set.py   exit 0 = all PASS
"""
import builtins
import contextlib
import fnmatch
import glob
import hashlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SNAP = os.path.normpath(os.path.join(HERE, "..", "javis_state_snapshot.py"))
EXCLUDES = ("*.db", "*.sqlite", "*-wal", "*-shm", "*.bak", "*.bak-*")

checks, bad = 0, []


def check(name, cond, detail=""):
    global checks
    checks += 1
    if not cond:
        bad.append(name)
    print(("PASS " if cond else "FAIL ") + name + (" | " + str(detail) if detail != "" else ""))


def w(path, text="x\n"):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def load():
    spec = importlib.util.spec_from_file_location("javis_state_snapshot_p5", SNAP)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


class World:
    """One machine: a base lane and a department, both with a pack, a state dir and a round."""

    def __init__(self, root, name):
        self.home = os.path.join(root, name)
        self.la = os.path.join(self.home, "AppData", "Local")
        self.state_root = self.la if os.name == "nt" else os.path.join(self.home, ".local", "state")
        self.state = os.path.join(self.state_root, "cys")
        self.pack = os.path.join(self.home, ".cys", "pack")
        self.dept = os.path.join(self.home, ".cys", "pack-dept-dept-1")
        self.gen_root = os.path.join(self.home, ".cys", "state-generations")
        for sd in (self.state, os.path.join(self.state_root, "cys-dept-dept-1")):
            for b in ("topology.json", "schedule_state.json", "event.seq"):
                w(os.path.join(sd, b), "{}\n")
        for pk, soul in ((self.pack, "base soul\n"), (self.dept, "dept soul\n")):
            w(os.path.join(pk, "soul.md"), soul)
            w(os.path.join(pk, "round", "SESSION_STATE.md"))
            w(os.path.join(pk, "round", "WORKER_TODO.md"))
            w(os.path.join(pk, "directives", "WORKER_DIRECTIVE.md"))
            w(os.path.join(pk, "memory", "one.md"))
        w(os.path.join(self.home, "_round", "SESSION_STATE.md"))
        w(os.path.join(self.home, ".claude", "projects", "p", "memory", "a.md"))
        with open(os.path.join(self.home, ".cys", "depts.json"), "w", encoding="utf-8") as f:
            json.dump({"depts": {"dept-1": {"pack_dir": self.dept}}}, f)

    def run(self, at=None, extra_env=None):
        """The job's pipe: one stream, read from the end. -> (rc, all lines, last 3)."""
        env = {"HOME": self.home, "USERPROFILE": self.home, "LOCALAPPDATA": self.la, "PATH": os.environ.get("PATH", ""),
               "CYS_PACK_DIR": self.pack, "LANG": "en_US.UTF-8", "PYTHONIOENCODING": "utf-8"}
        if at is not None:
            env["_CYS_SNAPSHOT_NOW"] = str(at)
        env.update(extra_env or {})
        p = subprocess.run([sys.executable, "-B", SNAP, "snapshot"], env=env, cwd=self.home,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
        lines = p.stdout.decode("utf-8", "replace").splitlines()
        return p.returncode, lines, lines[-3:]

    def run_patched(self, patch_code):
        """The job's pipe again, with one seam patched in the snapshot module. -> (last 3 lines)."""
        code = ("import sys\nsys.path.insert(0, %r)\nsys.dont_write_bytecode = True\nimport javis_state_snapshot as m\n%s\nm.do_snapshot()\n"
                % (os.path.dirname(SNAP), patch_code))
        env = {"HOME": self.home, "USERPROFILE": self.home, "LOCALAPPDATA": self.la, "PATH": os.environ.get("PATH", ""),
               "CYS_PACK_DIR": self.pack, "LANG": "en_US.UTF-8", "PYTHONIOENCODING": "utf-8", "_CYS_SNAPSHOT_NOW": "1790100000"}
        p = subprocess.run([sys.executable, "-B", "-c", code], env=env, cwd=self.home, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
        return p.stdout.decode("utf-8", "replace").splitlines()[-3:]

    def newest(self):
        g = sorted(n for n in os.listdir(self.gen_root) if n[:1].isdigit())[-1]
        with open(os.path.join(self.gen_root, g, "manifest.json"), encoding="utf-8") as f:
            return g, json.load(f)


def t_soul_captured(root):
    x = World(root, "soul")
    rc, lines, tail = x.run()
    g, man = x.newest()
    by_src = dict((e["source"], e["stored_as"]) for e in man["files"])
    for pk, want in ((x.pack, "base soul\n"), (x.dept, "dept soul\n")):
        src = os.path.join(pk, "soul.md")
        got = None
        if src in by_src:
            with open(os.path.join(x.gen_root, g, by_src[src]), encoding="utf-8") as f:
                got = f.read()
        check("T1 soul.md of %s is in the generation, same bytes" % os.path.basename(pk), got == want, got)
    hits = glob.glob(os.path.join(x.gen_root, g, "*soul.md"))
    check("T1 the ruling's own test: `*soul.md` over the generation matches", len(hits) == 2, [os.path.basename(h) for h in hits])
    check("T1 last line: PASS, every pattern, soul.md 2/2", tail[-1].startswith("[capture] PASS 패턴 5/5") and "soul.md 2/2" in tail[-1], tail[-1])


def t_predicate_not_enumeration(root):
    x = World(root, "pred")
    plants = [os.path.join(x.pack, "team-soul.md"), os.path.join(x.pack, "round", "NEW_NODE_TODO.md"),
              os.path.join(x.dept, "round", "CSO_STANDING_RULES_9.md"), os.path.join(x.home, ".claude", "projects", "q", "memory", "b.md"),
              os.path.join(x.home, ".claude-second", "projects", "z", "memory", "c.md")]
    for p in plants:
        w(p)
    x.run()
    srcs = set(e["source"] for e in x.newest()[1]["files"])
    missing = [p for p in plants if p not in srcs]
    check("T2 a new file in a pattern is captured with no code change", not missing, missing)


def t_exclusions(root):
    m = load()
    check("T3 CAPTURE_EXCLUDE_GLOBS is kept as it was", tuple(m.CAPTURE_EXCLUDE_GLOBS) == EXCLUDES, m.CAPTURE_EXCLUDE_GLOBS)
    x = World(root, "excl")
    decoys = [os.path.join(x.pack, n) for n in ("soul.md.bak", "soul.md.bak-20261101", "soul.bak-1.md", ".hidden.md", ".pack-state.json", "cache.db",
                                                "x.sqlite", "a-wal", "a-shm", "notes.txt", "NEW_LAW.md", "agents.json", "schedule.json")]
    decoys += [os.path.join(x.pack, "round", n) for n in ("log.jsonl", "SESSION_STATE.md.bak-1", "keep.db", "HANDOFF.md", "ledger.json",
                                                          "old.bak-1_TODO.md")]
    decoys += [os.path.join(x.state, n) for n in ("operator.token", "feed.jsonl", "transcripts.db", "transcripts.db-wal", "queue-state.json")]
    decoys += [os.path.join(x.home, "_round", "BASE_NOTE.md"), os.path.join(x.home, "_round", "BASE_TODO.md")]
    decoys += [os.path.join(x.pack, "directives", "NEW_ROLE_DIRECTIVE.md"), os.path.join(x.dept, "memory", "new.md")]
    decoys.append(os.path.join(x.pack, "round", "_evidence", "deep.md"))        # depth 1: no recursion
    decoys.append(os.path.join(x.pack, "bin", "tool.md"))                       # not a root
    for p in decoys:
        w(p)
    os.symlink(os.path.join(x.pack, "soul.md"), os.path.join(x.pack, "linked.md"))
    decoys.append(os.path.join(x.pack, "linked.md"))
    x.run()
    srcs = set(e["source"] for e in x.newest()[1]["files"])
    wrong = [p for p in decoys if p in srcs]
    check("T3 excluded names, dot files, other names, deeper files, other folders, links, the removed classes are not captured", not wrong, wrong)


def t_controls():
    m = load()
    got = m.capture_controls()
    check("T4 a positive control per pattern passes", set(got) == set(m.CAPTURE_PATTERNS) and all(got.values()), got)
    print("     patterns: %s" % ", ".join(m.CAPTURE_PATTERNS))
    real_roots = m.capture_roots
    for pat in [p for p in m.CAPTURE_PATTERNS if p != "registry"]:   # each control can fail: blind that pattern, only it may go red
        m.capture_roots = lambda *a, **k: [r for r in real_roots(*a, **k) if r[0] != pat]
        try:
            got = m.capture_controls()
        finally:
            m.capture_roots = real_roots
        check("T4 control of %s goes red when its pattern is blind" % pat, got[pat] is False and sum(got.values()) == 4, got)
    real = m.default_sources

    def no_registry(*a, **k):
        return [s for s in real(*a, **k) if os.path.basename(s) != "depts.json"]
    m.default_sources = no_registry
    try:
        check("T4 control of registry goes red when the registry is dropped", m.capture_controls()["registry"] is False)
    finally:
        m.default_sources = real
    saved = m.CAPTURE_EXCLUDE_GLOBS
    m.CAPTURE_EXCLUDE_GLOBS = ()
    try:
        check("T4 a widened predicate turns every control red", not any(m.capture_controls().values()))
    finally:
        m.CAPTURE_EXCLUDE_GLOBS = saved
    saved = m.CAPTURE_LANE_GLOBS
    m.CAPTURE_LANE_GLOBS = ("*.md",)
    try:
        check("T4 widening a pattern past its scope turns every control red", not any(m.capture_controls().values()))
    finally:
        m.CAPTURE_LANE_GLOBS = saved
    real_ref = m._capture_refused
    m._capture_refused = lambda *a, **k: False
    try:
        check("T4 a removed bound turns every control red", not any(m.capture_controls().values()))
    finally:
        m._capture_refused = real_ref
    m.IS_WINDOWS = True
    try:
        check("T4 the controls hold on the Windows state layout", all(m.capture_controls().values()))
    finally:
        m.IS_WINDOWS = os.name == "nt"


def t_zero_is_failure(root):
    x = World(root, "zero")
    shutil.rmtree(os.path.join(x.home, ".claude", "projects", "p", "memory"))
    rc, lines, tail = x.run(at=1790000000)
    check("T5a a pattern matching 0 across all generations is FAILURE on the last line",
          tail[-1].startswith("[capture] FAILURE") and "전 세대 0건 패턴 agent-memory" in tail[-1], tail[-1])
    check("T5b rc stays 0, the job discards it and the line is the signal", rc == 0, rc)
    check("T5c the generation is still made", x.newest()[1]["files"] != [])
    w(os.path.join(x.home, ".claude", "projects", "p", "memory", "first.md"))
    rc, lines, tail = x.run(at=1790000060)
    check("T5d the first file in the pattern clears it", tail[-1].startswith("[capture] PASS 패턴 5/5"), tail[-1])
    os.remove(os.path.join(x.home, ".claude", "projects", "p", "memory", "first.md"))
    rc, lines, tail = x.run(at=1790000120)
    check("T5e empty now but present in an older generation: not 0 across all", tail[-1].startswith("[capture] PASS 패턴 5/5")
          and "agent-memory=0" in tail[-1], tail[-1])


def t_visible_in_tail(root):
    x = World(root, "tail")
    os.remove(os.path.join(x.dept, "soul.md"))
    with open(os.path.join(x.home, ".cys", "depts.json"), "w", encoding="utf-8") as f:
        json.dump({"depts": {"dept-1": {"pack_dir": x.dept}, "broken": 42}}, f)      # a stderr line in the same stream
    for i in range(60):                               # enough generations that the [gc] line prints too
        rc, lines, tail = x.run(at=1790000000 + i * 3600)
    gc_seen = any(ln.startswith("[gc]") for ln in lines)
    check("T6a the run prints a registry line on stderr and a [gc] line", gc_seen and any("registry" in ln for ln in lines), lines[-4:])
    check("T6b tail -3 holds the FAILURE, as the last line", tail[-1].startswith("[capture] FAILURE") and "soul.md 없는 레인 1" in tail[-1], tail)
    check("T6c two [gc] lines push the generation line out, so the last line repeats its INCOMPLETE",
          not any(ln.startswith("[snapshot]") for ln in tail) and "INCOMPLETE" in tail[-1], tail)


def t_bound(root):
    """The covenant tree. Nothing in it is touched: not listed, not stat-ed, not opened. Temp home only, modelled tree."""
    m = load()
    home = os.path.realpath(os.path.join(root, "bound"))
    tree = os.path.join(home, "Desktop", "CYSjavis")
    w(os.path.join(tree, "pack", "soul.md"))
    w(os.path.join(tree, "pack", "round", "SESSION_STATE.md"))
    w(os.path.join(tree, "pack", "round", "A_TODO.md"))
    w(os.path.join(tree, "state", "topology.json"))
    w(os.path.join(tree, "linked", "projects", "x", "memory", "z.md"))
    w(os.path.join(home, ".cys", "pack", "soul.md"))
    w(os.path.join(home, ".cys", "pack", "round", "SESSION_STATE.md"))
    w(os.path.join(home, ".claude", "projects", "good", "memory", "kept.md"))
    os.makedirs(os.path.join(home, ".claude", "projects", "p"))
    os.symlink(os.path.join(tree, "pack", "round"), os.path.join(home, ".claude", "projects", "p", "memory"))
    os.symlink(os.path.join(tree, "linked"), os.path.join(home, ".claude-linked"))                 # the review's second path
    state_root = os.path.join(home, ".local", "state")
    os.makedirs(state_root)
    os.symlink(os.path.join(tree, "state"), os.path.join(state_root, "cys-dept-linked"))
    dj = os.path.join(home, ".cys", "depts.json")
    with open(dj, "w", encoding="utf-8") as f:
        json.dump({"depts": {"walled": {"pack_dir": os.path.join(tree, "pack")},
                             "wsock": {"socket": os.path.join(tree, "state", "cys.sock")}}}, f)    # the review's first path
    touched = []

    def seen(path):
        try:
            r = os.fspath(path)
        except TypeError:
            return
        if isinstance(r, str) and (r == tree or r.startswith(tree + os.sep)):
            touched.append(r)

    saved = {}

    def wrap(obj, name):
        real = getattr(obj, name)
        saved[(obj, name)] = real

        def spy(path=".", *a, **k):
            seen(path)
            return real(path, *a, **k)
        setattr(obj, name, spy)
    for obj, name in ((os, "listdir"), (os, "scandir"), (os, "stat"), (os, "lstat"), (os.path, "isdir"), (os.path, "isfile"),
                      (os.path, "exists"), (os.path, "islink"), (os.path, "getsize"), (os, "readlink")):
        wrap(obj, name)
    census = {}
    try:
        lanes = m.capture_lanes(home=home, env={"HOME": home}, depts_json=dj)
        srcs = m.default_sources(home=home, state_root=state_root, depts_json=dj, windows=False,
                                 env={"HOME": home}, lanes=lanes, census=census)
    finally:
        for (obj, name), real in saved.items():
            setattr(obj, name, real)
    named = [s for s in srcs if s == tree or s.startswith(tree + os.sep)]
    check("T7a a lane, a registry socket, a state link and two memory links into the tree: nothing in it is listed, stat-ed or read", not touched, touched[:4])
    check("T7b nothing from that tree is in the set, including the legacy declarations", not named, named)
    check("T7c the refused paths are recorded", len(census["notes"]["refused_roots"]) >= 3, census["notes"]["refused_roots"])
    check("T7d the base lane and an honest memory folder still are", os.path.join(home, ".cys", "pack", "soul.md") in srcs
          and os.path.join(home, ".claude", "projects", "good", "memory", "kept.md") in srcs)
    # an explicit source list: the copy loop never sees it
    out = io.StringIO()
    real_refused = m._capture_refused
    m._capture_refused = lambda path, home_=home: real_refused(path, home_)
    try:
        with contextlib.redirect_stdout(out):
            name = m.do_snapshot(sources=[os.path.join(tree, "pack", "soul.md"), os.path.join(home, ".cys", "pack", "soul.md")],
                                 gen_root=os.path.join(home, "gens-explicit"))
    finally:
        m._capture_refused = real_refused
    with open(os.path.join(home, "gens-explicit", name, "manifest.json"), encoding="utf-8") as f:
        man = json.load(f)
    check("T7e an explicit source in the tree is refused at the source entry", [e["source"] for e in man["files"]] == [os.path.join(home, ".cys", "pack", "soul.md")]
          and any("refused" in x for x in man["missing"]), man["missing"])
    check("T7f the guard treats a chain of links the same", m._capture_refused(os.path.join(home, ".claude-linked", "projects"), home)
          and not m._capture_refused(os.path.join(home, ".claude", "projects", "good", "memory"), home))


def t_dataless(root):
    m = load()
    F = type("St", (), {"st_flags": m.SF_DATALESS})
    N = type("St", (), {"st_flags": 0})
    O = type("St", (), {})
    check("T8a the flag is read from stat, and a platform without st_flags is not dataless",
          m._is_dataless(F()) and not m._is_dataless(N()) and not m._is_dataless(O()))
    td = os.path.join(root, "dataless")
    a, b = os.path.join(td, "a.md"), os.path.join(td, "b.md")
    w(a, "plain\n")
    w(b, "z" * 77)
    m._is_dataless = lambda st: st.st_size == 77
    opened = []
    real_open, real_os_open = builtins.open, os.open

    def spy(file, *x, **k):
        opened.append(str(file))
        return real_open(file, *x, **k)

    def spy_os(path, *x, **k):
        opened.append(str(path))
        return real_os_open(path, *x, **k)
    builtins.open = spy
    os.open = spy_os
    out = io.StringIO()
    gr = os.path.join(td, "gens")
    try:
        with contextlib.redirect_stdout(out):
            name = m.do_snapshot(sources=[a, b], gen_root=gr)
    finally:
        builtins.open = real_open
        os.open = real_os_open
    with open(os.path.join(gr, name, "manifest.json"), encoding="utf-8") as f:
        man = json.load(f)
    check("T8b a dataless file is never opened", b not in opened, [p for p in opened if p.endswith(".md")])
    check("T8c it is measured: size and time in the manifest, not in files",
          [e["size"] for e in man.get("dataless", [])] == [77] and [e["source"] for e in man["files"]] == [a], man.get("dataless"))
    check("T8d the generation says so", "INCOMPLETE" in out.getvalue() and "dataless 1건" in out.getvalue(), out.getvalue().strip())
    m2 = load()
    m2._is_dataless = lambda st: True
    home = os.path.realpath(os.path.join(root, "dataless-root"))
    w(os.path.join(home, "mem", "x.md"))
    notes = {}
    got = m2.capture_files([("agent-memory", os.path.join(home, "mem"), ("*.md",))], home=home, notes=notes)
    check("T8e a dataless root folder is not listed", got == [] and len(notes["refused_roots"]) == 1, notes)


def t_dataless_boundary(root):
    """Every reader goes through one boundary. No real SF_DATALESS file exists here: the detector seam `_is_dataless` flags a
    temp file by size, and an open spy proves the flagged path is never opened. No read of a real dataless file happens."""
    td = os.path.join(root, "boundary")
    flagged = 77
    opened = []
    real_open, real_os_open = builtins.open, os.open

    def spy(file, *a, **k):
        opened.append(str(file))
        return real_open(file, *a, **k)

    def spy_os(path, *a, **k):
        opened.append(str(path))
        return real_os_open(path, *a, **k)

    def spying(fn):
        del opened[:]
        builtins.open, os.open = spy, spy_os
        try:
            return fn()
        finally:
            builtins.open, os.open = real_open, real_os_open

    # the registry: both readers
    m = load()
    m._is_dataless = lambda st: st.st_size == flagged
    reg = os.path.join(td, "depts.json")
    w(reg, " " * flagged)
    entries, faults = spying(lambda: m._registry_pack_dirs(reg))
    check("T13a a flagged registry is not opened by _registry_pack_dirs, and it is a fault, not an empty list",
          reg not in opened and entries == [] and len(faults) == 1 and "DatalessFile" in faults[0], faults)
    os.makedirs(os.path.join(td, "state"))
    got = spying(lambda: m._dept_state_dirs(os.path.join(td, "state"), depts_json=reg, windows=False))
    check("T13b a flagged registry is not opened by _dept_state_dirs", reg not in opened and got == [], got)
    # GC's reader of the registry
    got = spying(lambda: m._lane_caps(home=td, depts_json=reg))
    check("T13c a flagged registry is not opened by the GC", reg not in opened, opened[:2])
    # a generation's manifest: the historical reader of the capture report and the GC's reader
    x = World(root, "boundary-world")
    x.run(at=1790000000)
    g, man = x.newest()
    mp = os.path.join(x.gen_root, g, "manifest.json")
    with open(mp, "a", encoding="utf-8") as f:
        f.write(" " * (flagged - (os.path.getsize(mp) % 1)))
    size = os.path.getsize(mp)
    m2 = load()
    m2._is_dataless = lambda st: st.st_size == size
    census = {"roots": [("agent-memory", os.path.join(x.pack, "memory"), ("*.md",))], "depts_json": "none", "packs": [], "notes": {}}
    line = spying(lambda: m2.capture_report(census, [], x.gen_root))
    check("T13d a flagged manifest is not opened by the historical reader", mp not in opened and "FAILURE" in line, line[:90])
    # a source that flips after selection, before the copy
    a, b = os.path.join(td, "a.md"), os.path.join(td, "b.md")
    w(a, "plain\n")
    w(b, "small\n")
    m3 = load()
    m3._is_dataless = lambda st: st.st_size == flagged
    real_now = m3._now_utc

    def flip():
        os.truncate(b, flagged)          # runs between the partition and the copy loop, opens nothing the spy would count
        return real_now()
    m3._now_utc = flip
    out = io.StringIO()
    gr = os.path.join(td, "gens")

    def run():
        with contextlib.redirect_stdout(out):
            return m3.do_snapshot(sources=[a, b], gen_root=gr)
    name = spying(run)
    with open(os.path.join(gr, name, "manifest.json"), encoding="utf-8") as f:
        man = json.load(f)
    check("T13e selected, then flagged before the copy: never opened", b not in opened, [p for p in opened if p.endswith(".md")])
    check("T13f it is measured, not stored, and the generation says so", [e["size"] for e in man.get("dataless", [])] == [flagged]
          and [e["source"] for e in man["files"]] == [a] and "복사 직전 dataless 1건" in out.getvalue(), out.getvalue().strip())
    # a source that flips between the stat and the open: the fstat stops it before the first byte
    c = os.path.join(td, "c.md")
    w(c, "text\n")
    m4 = load()
    calls = []
    m4._is_dataless = lambda st: calls.append(1) or len(calls) > 1
    try:
        m4.safe_open(c, binary=True).read()
        refused = False
    except m4.DatalessFile:
        refused = True
    check("T13g flipped between stat and open: refused before any read", refused and len(calls) == 2, calls)
    # the stored bytes are the hashed bytes: one fd
    m5 = load()
    gr5 = os.path.join(td, "gens5")
    with contextlib.redirect_stdout(io.StringIO()):
        name = m5.do_snapshot(sources=[a], gen_root=gr5)
    with open(os.path.join(gr5, name, "manifest.json"), encoding="utf-8") as f:
        e = json.load(f)["files"][0]
    with open(os.path.join(gr5, name, e["stored_as"]), "rb") as f:
        body = f.read()
    check("T13h the manifest's hash and size are of the stored bytes", hashlib.sha256(body).hexdigest() == e["sha256"] and len(body) == e["size"])
    gate = load()._dataless_gate()
    check("T13i the kernel policy is set on macOS, not applicable elsewhere", gate is (True if sys.platform == "darwin" else None), gate)


def t_live_churn(root):
    m = load()
    td = os.path.join(root, "churn")
    a, b = os.path.join(td, "a.md"), os.path.join(td, "b.md")
    w(a)
    w(b)
    real = m.safe_open
    m.safe_open = lambda p, **k: (_ for _ in ()).throw(FileNotFoundError(p)) if p == b else real(p, **k)
    with contextlib.redirect_stdout(io.StringIO()):
        name = m.do_snapshot(sources=[a, b], gen_root=os.path.join(td, "gens"))
    check("T9 a file gone between the walk and the copy costs that file, not the generation", bool(name))
    with open(os.path.join(td, "gens", name, "manifest.json"), encoding="utf-8") as f:
        man = json.load(f)
    check("T9 it is recorded", [e["source"] for e in man.get("vanished", [])] == [b] and [e["source"] for e in man["files"]] == [a], man.get("vanished"))


def t_churn_through_the_real_path(root):
    """The review's false PASS: the file was selected, it failed before the copy, and the report still credited it."""
    x = World(root, "churn-real")
    tail = x.run_patched('real = m.safe_open\n'
                         'def boom(p, **k):\n'
                         '    if p.replace("\\\\", "/").endswith("/memory/a.md"):\n'
                         '        raise FileNotFoundError(p)\n'
                         '    return real(p, **k)\n'
                         'm.safe_open = boom')
    g, man = x.newest()
    stored = [e["source"] for e in man["files"]]
    check("T9c the failed file is in vanished and not in files", len(man.get("vanished", [])) == 1
          and not any(s.endswith("/memory/a.md") for s in stored), man.get("vanished"))
    check("T9d the last line counts stored entries: agent-memory=0, FAILURE, 0 across all generations",
          tail[-1].startswith("[capture] FAILURE") and "전 세대 0건 패턴 agent-memory" in tail[-1] and "agent-memory=0" in tail[-1], tail[-1])
    check("T9e the last line says the copy failed", "복사 실패 1건" in tail[-1], tail[-1])


def t_false_zero(root):
    """The review's false zero: an unclassified historical path filled the seventh slot and stopped the scan."""
    x = World(root, "falsezero")
    x.run(at=1790000000)
    shutil.rmtree(os.path.join(x.home, ".claude", "projects", "p", "memory"))
    x.run(at=1790000060)
    g, man = x.newest()
    man["files"].append({"source": "/somewhere/else/unclassified.md", "stored_as": "unclassified.md", "sha256": "0", "size": 0})
    with open(os.path.join(x.gen_root, g, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(man, f)
    rc, lines, tail = x.run(at=1790000120)
    check("T11 an unclassified historical path does not stop the scan: the older generation counts",
          tail[-1].startswith("[capture] PASS 패턴 5/5") and "agent-memory=0" in tail[-1], tail[-1])


def t_credentials(root):
    """The review's credentials.json against the narrowed set, in a state root, a pack root and a round. Dummy data."""
    m = load()
    x = World(root, "creds")
    marker = "capture-test-marker-not-a-credential"
    plants = [os.path.join(x.state, "credentials.json"), os.path.join(x.pack, "credentials.json"),
              os.path.join(x.dept, "round", "credentials.json")]
    for p in plants:
        w(p, json.dumps({"api_key": marker}))
    rc, lines, tail = x.run()
    g, man = x.newest()
    srcs = [e["source"] for e in man["files"]]
    check("T12a credentials.json in a state root, a pack root and a round reaches no pattern", not set(plants) & set(srcs), set(plants) & set(srcs))
    copied = []
    for d, _s, names in os.walk(os.path.join(x.gen_root, g)):
        for n in names:
            with open(os.path.join(d, n), "rb") as f:
                if marker.encode() in f.read():
                    copied.append(n)
    check("T12b the dummy value is in no file of the generation", not copied, copied)
    opened = []
    real_open, real_os_open = builtins.open, os.open

    def spy(file, *a, **k):
        opened.append(str(file))
        return real_open(file, *a, **k)

    def spy_os(path, *a, **k):
        opened.append(str(path))
        return real_os_open(path, *a, **k)
    builtins.open, os.open = spy, spy_os
    try:
        got = m.capture_files([("state", x.state, tuple(m.DECLARATIVE_BASENAMES)), ("lane", x.pack, m.CAPTURE_LANE_GLOBS)], home=x.home)
    finally:
        builtins.open, os.open = real_open, real_os_open
    check("T12c the set is decided without opening any file", not opened and not set(plants) & set(got), opened[:3])


def main():
    root = os.path.realpath(tempfile.mkdtemp(prefix="p5-capture-"))
    try:
        t_soul_captured(root)
        t_predicate_not_enumeration(root)
        t_exclusions(root)
        t_controls()
        t_zero_is_failure(root)
        t_visible_in_tail(root)
        t_bound(root)
        t_dataless(root)
        t_dataless_boundary(root)
        t_live_churn(root)
        t_churn_through_the_real_path(root)
        t_false_zero(root)
        t_credentials(root)
    finally:
        shutil.rmtree(root, ignore_errors=True)
    print("\n=== %d/%d PASS ===" % (checks - len(bad), checks))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
