#!/usr/bin/env python3
"""0.14.57 P5 — the generation capture set comes from a predicate, with a control per pattern per run.

The ruling: an enumeration always lags and a new path then passes as lawful (`*soul.md` matched 0 of 70 generations,
the 2026-10-16 bytes are unrecoverable). So: a predicate over bounded roots, a positive control per pattern per run,
and a pattern matching 0 across all generations is a FAILURE, visible in the last 3 output lines, because the
phoenix-snapshot-6h job keeps only `2>&1 | tail -3`.

Whole runs go through the real CLI in a throw-away HOME with stdout and stderr merged into one pipe, as the job does.
  python3 test_phoenix_capture_set.py   exit 0 = all PASS
"""
import builtins
import contextlib
import fnmatch
import glob
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
    check("T1 last line: PASS, every pattern, soul.md 2/2", tail[-1].startswith("[capture] PASS 패턴 7/7") and "soul.md 2/2" in tail[-1], tail[-1])


def t_predicate_not_enumeration(root):
    x = World(root, "pred")
    plants = [os.path.join(x.pack, "NEW_LAW.md"), os.path.join(x.dept, "lane-config.json"),
              os.path.join(x.pack, "round", "HANDOFF_20270101.md"), os.path.join(x.dept, "round", "ledger.json"),
              os.path.join(x.pack, "directives", "NEW_ROLE_DIRECTIVE.md"), os.path.join(x.dept, "memory", "new.md"),
              os.path.join(x.state, "new-declared-state.json"), os.path.join(x.home, "_round", "BASE_NOTE.md"),
              os.path.join(x.home, ".claude", "projects", "q", "memory", "b.md")]
    for p in plants:
        w(p)
    x.run()
    srcs = set(e["source"] for e in x.newest()[1]["files"])
    missing = [p for p in plants if p not in srcs]
    check("T2 a path no list names is captured, in every root, with no code change", not missing, missing)


def t_exclusions(root):
    m = load()
    check("T3 CAPTURE_EXCLUDE_GLOBS is kept as it was", tuple(m.CAPTURE_EXCLUDE_GLOBS) == EXCLUDES, m.CAPTURE_EXCLUDE_GLOBS)
    x = World(root, "excl")
    decoys = [os.path.join(x.pack, n) for n in ("soul.md.bak", "soul.md.bak-20261101", "soul.bak-1.md", ".hidden.md", ".pack-state.json", "cache.db",
                                                "x.sqlite", "a-wal", "a-shm", "notes.txt")]
    decoys += [os.path.join(x.pack, "round", n) for n in ("log.jsonl", "SESSION_STATE.md.bak-1", "keep.db")]
    decoys += [os.path.join(x.state, n) for n in ("operator.token", "feed.jsonl", "transcripts.db", "transcripts.db-wal")]
    decoys.append(os.path.join(x.pack, "round", "_evidence", "deep.md"))        # depth 1: no recursion
    decoys.append(os.path.join(x.pack, "bin", "tool.md"))                       # not a root
    for p in decoys:
        w(p)
    os.symlink(os.path.join(x.pack, "soul.md"), os.path.join(x.pack, "linked.md"))
    decoys.append(os.path.join(x.pack, "linked.md"))
    x.run()
    srcs = set(e["source"] for e in x.newest()[1]["files"])
    wrong = [p for p in decoys if p in srcs]
    check("T3 excluded names, dot files, other suffixes, deeper files, other folders, links are not captured", not wrong, wrong)


def t_controls():
    m = load()
    got = m.capture_controls()
    check("T4 a positive control per pattern passes", set(got) == set(m.CAPTURE_PATTERNS) and all(got.values()), got)
    print("     patterns: %s" % ", ".join(m.CAPTURE_PATTERNS))
    for pat in m.CAPTURE_SUFFIXES:                  # each control can fail: blind that pattern, only it may go red
        saved = m.CAPTURE_SUFFIXES[pat]
        m.CAPTURE_SUFFIXES[pat] = (".no-such-suffix",)
        try:
            got = m.capture_controls()
        finally:
            m.CAPTURE_SUFFIXES[pat] = saved
        check("T4 control of %s goes red when its pattern is blind" % pat, got[pat] is False, got)
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
    for pk in (x.pack, x.dept):
        shutil.rmtree(os.path.join(pk, "memory"))
    rc, lines, tail = x.run(at=1790000000)
    check("T5a a pattern matching 0 across all generations is FAILURE on the last line",
          tail[-1].startswith("[capture] FAILURE") and "전 세대 0건 패턴 pack-memory" in tail[-1], tail[-1])
    check("T5b rc stays 0, the job discards it and the line is the signal", rc == 0, rc)
    check("T5c the generation is still made", x.newest()[1]["files"] != [])
    w(os.path.join(x.pack, "memory", "first.md"))
    rc, lines, tail = x.run(at=1790000060)
    check("T5d the first file in the pattern clears it", tail[-1].startswith("[capture] PASS 패턴 7/7"), tail[-1])
    os.remove(os.path.join(x.pack, "memory", "first.md"))
    rc, lines, tail = x.run(at=1790000120)
    check("T5e empty now but present in an older generation: not 0 across all", tail[-1].startswith("[capture] PASS 패턴 7/7")
          and "pack-memory=0" in tail[-1], tail[-1])


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
    m = load()
    home = os.path.realpath(os.path.join(root, "bound"))
    tree = os.path.join(home, "Desktop", "CYSjavis")
    w(os.path.join(tree, "pack", "soul.md"))
    w(os.path.join(tree, "pack", "round", "SESSION_STATE.md"))
    w(os.path.join(tree, "pack", "round", "A_TODO.md"))
    w(os.path.join(home, ".cys", "pack", "soul.md"))
    w(os.path.join(home, ".cys", "pack", "round", "SESSION_STATE.md"))
    os.makedirs(os.path.join(home, ".claude", "projects", "p"))
    os.symlink(os.path.join(tree, "pack", "round"), os.path.join(home, ".claude", "projects", "p", "memory"))
    dj = os.path.join(home, ".cys", "depts.json")
    with open(dj, "w", encoding="utf-8") as f:
        json.dump({"depts": {"walled": {"pack_dir": os.path.join(tree, "pack")}}}, f)
    seen = []
    real_ls, real_scandir, real_glob = m.os.listdir, m.os.scandir, m._glob.glob

    def ls(path="."):
        seen.append(os.path.realpath(path))
        return real_ls(path)

    def sd(path="."):
        seen.append(os.path.realpath(path))
        return real_scandir(path)
    m.os.listdir, m.os.scandir = ls, sd
    census = {}
    try:
        lanes = m.capture_lanes(home=home, env={"HOME": home}, depts_json=dj)
        srcs = m.default_sources(home=home, state_root=os.path.join(home, ".local", "state"), depts_json=dj, windows=False,
                                 env={"HOME": home}, lanes=lanes, census=census)
    finally:
        m.os.listdir, m.os.scandir = real_ls, real_scandir
    walked = [p for p in seen if p == tree or p.startswith(tree + os.sep)]
    named = [s for s in srcs if os.path.realpath(s).startswith(tree + os.sep)]
    check("T7a a lane registered inside ~/Desktop/CYSjavis is not walked, a link into it neither", not walked, walked)
    check("T7b nothing from that tree is in the set", not named, named)
    check("T7c the refused roots are recorded", len(census["notes"]["refused_roots"]) >= 1, census["notes"]["refused_roots"])
    check("T7d the base lane still is", os.path.join(home, ".cys", "pack", "soul.md") in srcs)


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
    got = m2.capture_files([("pack-memory", os.path.join(home, "mem"))], home=home, notes=notes)
    check("T8e a dataless root folder is not listed", got == [] and len(notes["refused_roots"]) == 1, notes)


def t_live_churn(root):
    m = load()
    td = os.path.join(root, "churn")
    a, b = os.path.join(td, "a.md"), os.path.join(td, "b.md")
    w(a)
    w(b)
    real = m._sha256
    m._sha256 = lambda p: (_ for _ in ()).throw(FileNotFoundError(p)) if p == b else real(p)
    with contextlib.redirect_stdout(io.StringIO()):
        name = m.do_snapshot(sources=[a, b], gen_root=os.path.join(td, "gens"))
    check("T9 a file gone between the walk and the copy costs that file, not the generation", bool(name))
    with open(os.path.join(td, "gens", name, "manifest.json"), encoding="utf-8") as f:
        man = json.load(f)
    check("T9 it is recorded", [e["source"] for e in man.get("vanished", [])] == [b] and [e["source"] for e in man["files"]] == [a], man.get("vanished"))


def t_oversize(root):
    x = World(root, "big")
    w(os.path.join(x.pack, "round", "HUGE.md"), "y" * 2048)
    m = load()
    m.CAPTURE_MAX_BYTES = 1024
    notes = {}
    got = m.capture_files([("round", os.path.join(x.pack, "round"))], home=x.home, notes=notes)
    check("T10 over the size bound: measured, listed, not captured", os.path.join(x.pack, "round", "HUGE.md") not in got
          and [e["size"] for e in notes["oversize"]] == [2048], notes["oversize"])


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
        t_live_churn(root)
        t_oversize(root)
    finally:
        shutil.rmtree(root, ignore_errors=True)
    print("\n=== %d/%d PASS ===" % (checks - len(bad), checks))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
