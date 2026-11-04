#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_f58_learning_loop.py — F58 T-11: the learning loop's store, staging and per-entry revert.

What it checks (DESIGN-F58-final.md §3.4-3.5, §4.2-4.4, §6.2-6.6; rows T1-19c, T1-19f, T1-18c):
  store   — `--approved` is gone; only the closed low-risk shape (provisional, verdict improved, every string of
            desc/name/body free of commands, paths, addresses, code and action words, a plain https citation,
            body <= 2,048 bytes) is written to memory. Everything else is STAGED: exit 8, a `staged` ledger line
            and a round record, nothing written to memory, the gate not run. A gate bundle that declares a
            high-risk operation or any target path is staged before the gate. `human_signed` in the bundle is
            always overwritten with false before rsi-gate.sh reads it.
  revert  — `javis_learn revert --entry <name>` removes exactly the recorded entry and its one index line
            through `javis_memory.py remove` (same MEMORY.md lock as add/update, index first, compensation),
            moves the entry to memory/.learned-trash/<UTC>/, and refuses (exit 7) whenever the recorded bytes
            or line no longer match. A lost ledger line (exit 5) is recovered only on an exact trash + index match.
  Known boundary written as a test: 「보관 폴더는 매주 새로 시작한다」 is stored (lexical detector, not meaning).

Every case runs the real javis_learn.py, javis_memory.py and rsi-gate.sh, copied into a temp bin; the gate is
wrapped by a recorder that logs each input it receives. HOME, the pack and the round dir are temp folders.
Run: python3 cysjavis-pack/bin/tests/test_f58_learning_loop.py
"""
import argparse
import contextlib
import glob
import hashlib
import importlib.util
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
PACK = os.path.dirname(BIN)
ROOT = os.path.dirname(PACK)
COPY = ("javis_learn.py", "javis_memory.py", "javis_rsi.py", "javis_skillscan.py", "skillscan_rules.json")
ROUND = "r1"
SHA_A = "a" * 64
STAGED_RC = 8
INDEX0 = "# MEMORY.md\n\n## 색인\n\n"
GATE_WRAPPER = ('#!/usr/bin/env bash\ninp="$(cat)"\nprintf \'%s\\n\' "$inp" >> "$F58_GATE_LOG"\n'
                'printf \'%s\' "$inp" | bash "$(dirname "$0")/rsi-gate.real.sh"\n')
DRIVER = r'''
import importlib.util, sys
spec = importlib.util.spec_from_file_location("javis_learn", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
flags = sys.argv[2].split(",")
if "noprecheck" in flags:
    m.bundle_precheck = lambda gi: []
if "nooverwrite" in flags:
    m.STORE_GATE_OVERRIDE = {}
if "ledgerfail" in flags:
    _orig = m._append_ledger
    def _bad(entry):
        if entry.get("event") == "reverted":
            raise OSError("injected ledger failure")
        return _orig(entry)
    m._append_ledger = _bad
sys.argv = ["javis_learn.py"] + sys.argv[3:]
sys.exit(m.main())
'''
LOCK_HOLDER = r'''
import importlib.util, sys
spec = importlib.util.spec_from_file_location("javis_memory", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
with m.FileLock(sys.argv[2]):
    print("held", flush=True)
    sys.stdin.readline()
'''


def sha(b):
    return hashlib.sha256(b).hexdigest()


def plain_pattern(**kw):
    p = {"domain": "build cache", "condition": "the build cache is older than the source",
         "action": "keep the cache folder", "rationale": "a rebuild is expected",
         "evidence_ref": "https://example.org/a"}
    p.update(kw)
    return p


def v2_pattern(**kw):
    p = plain_pattern()
    p.update({"behavioral_claim": "a stale cache is followed by a rebuild",
              "falsifier": "no rebuild after the cache went stale",
              "maturity": {"first_seen": "2024-05-01", "adoption_evidence": "three teams",
                           "known_failures": []}})
    p.update(kw)
    return p


def bundle(**kw):
    b = {"producer_model_family": "claude", "target_paths": [], "operations": [],
         "dimensions": {"source": {"fetch_log": True, "canonical": False, "distinct_sources": 1},
                        "fact_check": {"cross_checked": True},
                        "evidence": {"quote": "", "context_entailment": "support"},
                        "logic": {"verdict_json": "{\"verdict\":\"PASS\"}"},
                        "quality": {"eval_improved": True}},
         "verdicts": [{"dimension": "fact_check", "model_family": "gemini", "verdict": "PASS"},
                      {"dimension": "logic", "model_family": "codex", "verdict": "PASS"}]}
    b.update(kw)
    return b


def would_be_body(pattern, state="provisional", rnd=ROUND, verdict="improved"):
    """The body store writes (javis_learn.py cmd_store); checked against the staged record's entry_sha256."""
    return json.dumps({"pattern": pattern, "state": state, "round": rnd, "verdict": verdict,
                       "evidence_ref": pattern.get("evidence_ref")}, ensure_ascii=False, indent=2)


class Rig(object):
    def __init__(self, verdict="improved", rounds=(ROUND,)):
        self.d = tempfile.mkdtemp(prefix="f58-t11-")
        self.bin = os.path.join(self.d, "bin")
        os.makedirs(self.bin)
        for f in COPY:
            if os.path.exists(os.path.join(BIN, f)):
                shutil.copy2(os.path.join(BIN, f), os.path.join(self.bin, f))
        shutil.copy2(os.path.join(BIN, "rsi-gate.sh"), os.path.join(self.bin, "rsi-gate.real.sh"))
        with open(os.path.join(self.bin, "rsi-gate.sh"), "w", encoding="utf-8", newline="\n") as f:
            f.write(GATE_WRAPPER)
        self.home = os.path.join(self.d, "home")
        self.pack = os.path.join(self.d, "pack")
        self.mdir = os.path.join(self.pack, "memory")
        self.index = os.path.join(self.mdir, "MEMORY.md")
        self.round_dir = os.path.join(self.d, "round")
        self.stub = os.path.join(self.d, "stub")
        for p in (self.home, self.mdir, os.path.join(self.round_dir, "learn"), self.stub):
            os.makedirs(p)
        with open(self.index, "w", encoding="utf-8", newline="\n") as f:
            f.write(INDEX0)
        self.cys_log = os.path.join(self.d, "cys-calls.log")
        stub = os.path.join(self.stub, "cys")
        with open(stub, "w", encoding="utf-8", newline="\n") as f:
            f.write('#!/bin/sh\nprintf "%%s\\n" "$*" >> "%s"\nexit 0\n' % self.cys_log)
        os.chmod(stub, 0o755)
        self.gate_log = os.path.join(self.d, "gate.log")
        env = {k: v for k, v in os.environ.items()
               if not (k.startswith("CYS_") or k.startswith("JAVIS_") or k.startswith("AITERM_")
                       or k == "CLAUDE_CONFIG_DIR")}
        env.update({"HOME": self.home, "CYS_PACK_DIR": self.pack, "CYS_ROUND_DIR": self.round_dir,
                    "F58_GATE_LOG": self.gate_log,
                    "PATH": self.stub + os.pathsep + env.get("PATH", "/usr/bin:/bin")})
        self.env = env
        self.n = 0
        st = {"rounds": {r: {"round": r, "verdict": verdict, "stored": [], "harness": []} for r in rounds},
              "discovery": {"capability": 0, "perspective": 0, "knowledge": 0}}
        self.write_json(os.path.join(self.round_dir, "learn", "learn_state.json"), st)

    def close(self):
        shutil.rmtree(self.d, ignore_errors=True)

    def write_json(self, path, obj):
        with open(path, "w", encoding="utf-8") as f:
            json.dump(obj, f, ensure_ascii=False)

    def tmpjson(self, obj):
        self.n += 1
        p = os.path.join(self.d, "in-%d.json" % self.n)
        self.write_json(p, obj)
        return p

    def learn(self, args, extra_env=None):
        env = dict(self.env)
        env.update(extra_env or {})
        return subprocess.run([sys.executable, "-B", os.path.join(self.bin, "javis_learn.py")] + list(args),
                              cwd=self.d, env=env, capture_output=True, text=True, encoding="utf-8")

    def driver(self, flags, args):
        drv = os.path.join(self.d, "driver.py")
        with open(drv, "w", encoding="utf-8") as f:
            f.write(DRIVER)
        return subprocess.run([sys.executable, "-B", drv, os.path.join(self.bin, "javis_learn.py"), flags]
                              + list(args), cwd=self.d, env=self.env, capture_output=True, text=True,
                              encoding="utf-8")

    def memory(self, args):
        return subprocess.run([sys.executable, "-B", os.path.join(self.bin, "javis_memory.py"), "--dir", self.mdir]
                              + list(args), cwd=self.d, env=self.env, capture_output=True, text=True,
                              encoding="utf-8")

    def store(self, pattern, gi=None, name=None, state="provisional", rnd=ROUND, extra=(), extra_env=None,
              raw_pattern=None):
        pf = os.path.join(self.d, "pat-%d.json" % (self.n + 1))
        self.n += 1
        if raw_pattern is not None:
            with open(pf, "w", encoding="utf-8") as f:
                f.write(raw_pattern)
        else:
            self.write_json(pf, pattern)
        args = ["store", "--round", rnd, "--pattern", pf, "--type", "feedback", "--state", state,
                "--gate-input", self.tmpjson(bundle() if gi is None else gi), "--json"]
        if name:
            args += ["--name", name]
        return self.learn(args + list(extra), extra_env)

    def snapshot(self):
        out = {}
        for root, _dirs, files in os.walk(self.mdir):
            for fn in files:
                if fn.endswith(".lock"):
                    continue
                p = os.path.join(root, fn)
                with open(p, "rb") as f:
                    out[os.path.relpath(p, self.mdir)] = f.read()
        return out

    def read_index(self):
        with open(self.index, "rb") as f:
            return f.read()

    def ledger(self):
        p = os.path.join(self.round_dir, "learn", "ledger.jsonl")
        if not os.path.exists(p):
            return []
        with open(p, encoding="utf-8") as f:
            return [json.loads(x) for x in f if x.strip()]

    def state(self):
        with open(os.path.join(self.round_dir, "learn", "learn_state.json"), encoding="utf-8") as f:
            return json.load(f)

    def gate_calls(self):
        if not os.path.exists(self.gate_log):
            return []
        with open(self.gate_log, encoding="utf-8") as f:
            return [json.loads(x) for x in f if x.strip()]

    def stored_rec(self, name):
        for r in self.state()["rounds"].values():
            for s in r.get("stored", []):
                if s.get("name") == name:
                    return s
        return None

    def trash_files(self):
        return sorted(glob.glob(os.path.join(self.mdir, ".learned-trash", "*", "*")))


class RigCase(unittest.TestCase):
    def rig(self, **kw):
        r = Rig(**kw)
        self.addCleanup(r.close)
        return r

    def assertStaged(self, rig, res, *reasons):
        self.assertEqual(res.returncode, STAGED_RC, "want exit 8 STAGED; rc=%d\nstdout=%s\nstderr=%s"
                         % (res.returncode, res.stdout[-800:], res.stderr[-800:]))
        staged = [e for e in rig.ledger() if e.get("event") == "staged"]
        self.assertTrue(staged, "no staged ledger line")
        rec = staged[-1]
        self.assertEqual(rec.get("gate"), "not_run")
        self.assertTrue(re.match(r"^[0-9a-f]{64}$", rec.get("entry_sha256", "")), rec)
        joined = "\n".join(rec.get("reasons", []))
        for want in reasons:
            self.assertIn(want, joined, "reason %r missing from %r" % (want, rec.get("reasons")))
        return rec

    def assertStored(self, rig, res, name=None):
        self.assertEqual(res.returncode, 0, "want stored; rc=%d\nstdout=%s\nstderr=%s"
                         % (res.returncode, res.stdout[-800:], res.stderr[-800:]))
        files = [f for f in os.listdir(rig.mdir) if f.endswith(".md") and f != "MEMORY.md"]
        self.assertTrue(files, "no memory file written")
        return files


# ───────────────────────────── §4.4 T-11n: the detector on the real store path ─────────────────────────────

NAMED = [
    # (label, pattern-or-callable, expect, reasons, store kwargs)
    ("plain note", plain_pattern(action="When the build cache is older than the source, a rebuild is "
                                        "expected, so keep the cache folder."), "stored", (), {}),
    ("one word crosses", plain_pattern(action="the next run rebuilds it; keep"), "staged", ("run", ";"), {}),
    ("codex example", plain_pattern(action="Use PowerShell to run Remove-Item recursively on the archive"),
     "staged", ("powershell", "run", "Remove-"), {}),
    ("powershell form", plain_pattern(action="Remove-Item -Recurse C:\\archive"), "staged",
     ("Remove-", "-Recurse", "drive"), {}),
    ("del switches", plain_pattern(action="del /s /q archive"), "staged", ("del", "/s", "/q"), {}),
    ("rd switch", plain_pattern(action="rd /s archive"), "staged", ("rd",), {}),
    ("cmd switch", plain_pattern(action="cmd /c cleanup"), "staged", ("cmd",), {}),
    ("invoke url", plain_pattern(action="Invoke-WebRequest https://x"), "staged", ("Invoke-", "://"), {}),
    ("rm rf home", plain_pattern(action="rm -rf ~/old"), "staged", ("rm",), {}),
    # F58 r1fix m1 (impl review r1, worker-2 measured on 040e783f: string_problems() == [] for both): fullwidth
    # look-alikes of listed words and characters are read after NFKC, so they stage like their ASCII forms.
    ("fullwidth rm rf home", plain_pattern(action="ｒｍ －ｒｆ ～／ｏｌｄ"), "staged",
     ("command rm", "flag -rf", "home path ~/"), {}),
    ("fullwidth path and operators", plain_pattern(action="a／b ｜ c；d"), "staged",
     ("path 'a/b'", "operator |", "operator ;"), {}),
    ("korean erase", plain_pattern(action="보관 폴더를 통째로 지워라"), "staged", ("지워",), {}),
    ("korean overwrite", plain_pattern(action="설정 파일을 덮어쓴다"), "staged", ("덮어",), {}),
    ("names soul.md", plain_pattern(rationale="see soul.md first"), "staged", ("soul.md",), {}),
    ("token in round", plain_pattern(), "staged", ("curl",), {"rnd": "r7 curl"}),
    ("forbidden extra key", plain_pattern(shell="x"), "staged", ("shell",), {}),
    ("forbidden maturity key", v2_pattern(maturity={"first_seen": "2024-05-01", "adoption_evidence": "three teams",
                                                    "known_failures": [], "run": "x"}), "staged", ("run",), {}),
    ("forbidden known_failures value", v2_pattern(maturity={
        "first_seen": "2024-05-01", "adoption_evidence": "three teams",
        "known_failures": [{"source_url": "https://ex.org/pm", "snapshot_sha256": SHA_A, "summary": "rm"}]}),
     "staged", ("rm",), {}),
    ("evidence_ref key not exempt elsewhere", v2_pattern(maturity={
        "first_seen": "2024-05-01", "adoption_evidence": "three teams", "known_failures": [],
        "evidence_ref": "https://x/$(id)"}), "staged", ("$(",), {}),
    ("citation file:", plain_pattern(evidence_ref="file:///etc/x"), "staged", ("evidence_ref",), {}),
    ("citation space", plain_pattern(evidence_ref="https://x/a b"), "staged", ("evidence_ref",), {}),
    ("citation command", plain_pattern(evidence_ref="https://x/$(id)"), "staged", ("evidence_ref",), {}),
    ("known miss stored", plain_pattern(action="보관 폴더는 매주 새로 시작한다"), "stored", (), {}),
    ("near-miss words stored", plain_pattern(action="Curly brace style is preferred by the runner, delegate "
                                                    "the shellfish naming"), "stored", (), {}),
    ("default name with own prefix stored", plain_pattern(domain="cache notes"), "stored", (), {}),
    ("default name hides nothing", plain_pattern(domain="rsi gate"), "staged", ("rsi-",), {}),
    ("caller name checked whole", plain_pattern(), "staged", ("rsi-",), {"name": "rsi-gate-note"}),
    # mutation run u5-mutants: each case below is the only one that names its list (SAFETY_KEYWORDS, the file
    # names) — without them, dropping that list passed every test. (An undecodable --name is checked in PureAndRows:
    # on the store path the earlier, unchanged conflict-scan step stops on it first.)
    ("safety word", plain_pattern(rationale="the autopilot rule applies"), "staged", ("safety autopilot",), {}),
    ("names settings.json", plain_pattern(rationale="see settings.json"), "staged", ("file settings.json",), {}),
]


class DetectorOnStorePath(RigCase):
    def test_named_cases(self):
        for label, pat, expect, reasons, kw in NAMED:
            with self.subTest(case=label):
                rig = self.rig(rounds=(kw.get("rnd", ROUND),))
                before = rig.snapshot()
                res = rig.store(pat, name=kw.get("name"), rnd=kw.get("rnd", ROUND))
                if expect == "stored":
                    self.assertStored(rig, res)
                    self.assertEqual(len(rig.gate_calls()), 1)
                else:
                    self.assertStaged(rig, res, *reasons)
                    self.assertEqual(rig.snapshot(), before, "memory changed for a staged entry")
                    self.assertEqual(rig.gate_calls(), [], "gate ran for a staged entry")

    def test_confirmed_plain_is_staged(self):
        rig = self.rig()
        gi = bundle(conflict_audit={"reviewer": "reviewer2", "verdict": "PASS"})
        before = rig.snapshot()
        res = rig.store(v2_pattern(), gi=gi, state="confirmed")
        self.assertStaged(rig, res, "confirmed")
        self.assertEqual(rig.snapshot(), before)

    def test_body_size_boundary(self):
        base = would_be_body(plain_pattern(rationale=""))
        for size, expect in ((2048, "stored"), (2049, "staged")):
            with self.subTest(size=size):
                rig = self.rig()
                pat = plain_pattern(rationale="a" * (size - len(base.encode("utf-8"))))
                self.assertEqual(len(would_be_body(pat).encode("utf-8")), size)
                res = rig.store(pat)
                if expect == "stored":
                    self.assertStored(rig, res)
                else:
                    rec = self.assertStaged(rig, res, "2049")
                    self.assertEqual(rec["entry_sha256"], sha(would_be_body(pat).encode("utf-8")))

    def test_non_utf8_body_is_staged(self):
        rig = self.rig()
        raw = json.dumps(plain_pattern()).replace("a rebuild is expected", "a rebuild \\udc80 is expected")
        res = rig.store(None, raw_pattern=raw)
        self.assertStaged(rig, res, "undecodable")
        self.assertEqual(rig.gate_calls(), [])


# ───────────────────────────── §3.5 T-11a..o: composition, signature, fake approval ─────────────────────────────

class StoreComposition(RigCase):
    def test_11a_closed_shape_stored_without_feed(self):
        rig = self.rig()
        res = rig.store(plain_pattern())
        files = self.assertStored(rig, res)
        self.assertEqual(len(files), 1)
        idx = rig.read_index().decode("utf-8")
        self.assertEqual(idx.count("(%s)" % files[0]), 1)
        calls = open(rig.cys_log, encoding="utf-8").read() if os.path.exists(rig.cys_log) else ""
        self.assertNotIn("feed", calls)
        rec = rig.stored_rec("build-cache") or rig.stored_rec("rsi-build-cache")
        self.assertIsNotNone(rec, "no stored record")
        with open(os.path.join(rig.mdir, rec["entry_file"]), "rb") as f:
            self.assertEqual(sha(f.read()), rec["entry_sha256"])
        self.assertEqual(idx.split("\n").count(rec["index_line"]), 1)

    def test_11c1_declared_operation_is_staged_before_gate(self):
        for label, gi in (("network", bundle(operations=["network_socket"])),
                          ("network signed", bundle(operations=["network_socket"], human_signed=True)),
                          ("target paths", bundle(target_paths=["docs/x.md"]))):
            with self.subTest(case=label):
                rig = self.rig()
                before = rig.snapshot()
                res = rig.store(plain_pattern(), gi=gi)
                self.assertStaged(rig, res, "bundle declares")
                self.assertEqual(rig.gate_calls(), [])
                self.assertEqual(rig.snapshot(), before)

    def test_11c2_signature_is_overwritten(self):
        rig = self.rig()
        res = rig.store(plain_pattern(), gi=bundle(human_signed=True))
        self.assertStored(rig, res)
        calls = rig.gate_calls()
        self.assertEqual(len(calls), 1)
        self.assertIs(calls[0].get("human_signed"), False)

    def test_11c2_control_the_overwrite_bites(self):
        gi = bundle(operations=["network_socket"], human_signed=True)
        rig = self.rig()
        res = rig.driver("noprecheck", ["store", "--round", ROUND, "--pattern", rig.tmpjson(plain_pattern()),
                                        "--type", "feedback", "--gate-input", rig.tmpjson(gi)])
        self.assertEqual(res.returncode, 2, res.stderr)
        self.assertIn("고위험(인간서명 필요) 무서명", res.stderr)
        rig2 = self.rig()
        res2 = rig2.driver("noprecheck,nooverwrite", ["store", "--round", ROUND, "--pattern",
                                                      rig2.tmpjson(plain_pattern()), "--type", "feedback",
                                                      "--gate-input", rig2.tmpjson(gi)])
        self.assertEqual(res2.returncode, 0, res2.stderr)
        self.assertIs(rig2.gate_calls()[0].get("human_signed"), True)

    def test_11c3_non_closed_entries_staged_and_listed(self):
        rig = self.rig()
        cases = {"soul-note": plain_pattern(rationale="edit soul.md"),
                 "curl-note": plain_pattern(action="curl the archive"),
                 "win-note": plain_pattern(action="Remove-Item -Recurse C:\\archive")}
        before = rig.snapshot()
        for name, pat in cases.items():
            self.assertStaged(rig, rig.store(pat, name=name))
        res = rig.store(v2_pattern(), name="conf-note", state="confirmed",
                        gi=bundle(conflict_audit={"reviewer": "reviewer2", "verdict": "PASS"}))
        self.assertStaged(rig, res, "confirmed")
        self.assertEqual(rig.snapshot(), before)
        st = rig.learn(["status"])
        self.assertEqual(st.returncode, 0, st.stderr)
        for name in list(cases) + ["conf-note"]:
            self.assertIn(name, st.stdout)
        staged = rig.state()["rounds"][ROUND].get("staged", [])
        self.assertEqual(sorted(s["name"] for s in staged), sorted(list(cases) + ["conf-note"]))
        for s in staged:
            self.assertTrue(s.get("reasons"), s)

    def test_11d_no_fake_approval(self):
        rig = self.rig()
        res = rig.store(plain_pattern(), extra=["--approved"])
        self.assertEqual(res.returncode, 2)
        self.assertIn("--approved", res.stderr)
        fake_env = {"CYS_LEARN_APPROVED": "1", "JAVIS_LEARN_APPROVED": "1", "LEARN_APPROVED": "1",
                    "HUMAN_SIGNED": "1", "RSI_HUMAN_SIGNED": "1"}
        for label, gi in (("plain", bundle()), ("fields", bundle(human_signed=True, approved=True,
                                                                    consent="allow", owner_approved=True))):
            with self.subTest(case=label):
                r2 = self.rig()
                self.assertStaged(r2, r2.store(plain_pattern(action="curl it"), gi=gi, extra_env=fake_env), "curl")
                r3 = self.rig()
                self.assertStaged(r3, r3.store(plain_pattern(), gi=dict(gi, operations=["shell_exec"]),
                                               extra_env=fake_env), "bundle declares")

    def test_11b_deficient_evidence_not_stored_not_staged(self):
        rig = self.rig(verdict="flat")
        res = rig.store(plain_pattern())
        self.assertEqual(res.returncode, 2)
        self.assertFalse([e for e in rig.ledger() if e.get("event") == "staged"])
        rig2 = self.rig()
        bad = plain_pattern()
        del bad["condition"]
        res2 = rig2.store(bad)
        self.assertEqual(res2.returncode, 2)
        self.assertFalse([e for e in rig2.ledger() if e.get("event") == "staged"])
        rig3 = self.rig()
        gi = bundle()
        gi["dimensions"]["quality"]["eval_improved"] = False
        before = rig3.snapshot()
        res3 = rig3.store(plain_pattern(), gi=gi)
        self.assertEqual(res3.returncode, 2, res3.stderr)
        self.assertEqual(len(rig3.gate_calls()), 1)
        self.assertFalse([e for e in rig3.ledger() if e.get("event") == "staged"])
        self.assertEqual(rig3.snapshot(), before)

    def test_11o_staged_entry_edited_gets_a_new_record(self):
        rig = self.rig()
        before = rig.snapshot()
        a = self.assertStaged(rig, rig.store(plain_pattern(action="run it"), name="edit-note"))
        b = self.assertStaged(rig, rig.store(plain_pattern(action="run it twice"), name="edit-note"))
        self.assertNotEqual(a["entry_sha256"], b["entry_sha256"])
        self.assertEqual(len([e for e in rig.ledger() if e.get("event") == "staged"]), 2)
        self.assertEqual(rig.snapshot(), before)


class PureAndRows(unittest.TestCase):
    def load(self):
        spec = importlib.util.spec_from_file_location("javis_learn_f58", os.path.join(BIN, "javis_learn.py"))
        m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    def test_promotion_allowed_reads_no_approval(self):
        m = self.load()
        self.assertTrue(m.promotion_allowed("improved", False, "provisional")[0])
        self.assertFalse(m.promotion_allowed("improved", True, "confirmed")[0])
        self.assertFalse(m.promotion_allowed("flat", False, "provisional")[0])

    def test_undecodable_name_is_not_the_closed_shape(self):
        m = self.load()
        body = {"pattern": plain_pattern(), "state": "provisional", "round": ROUND, "verdict": "improved",
                "evidence_ref": "https://example.org/a"}
        entry = {"state": "provisional", "desc": "[provisional] build cache: keep the cache folder",
                 "name": "build-cache", "body": body, "body_text": json.dumps(body, ensure_ascii=False, indent=2)}
        self.assertEqual(m.closed_shape_problems(entry), [])
        self.assertIn("name: undecodable", m.closed_shape_problems(dict(entry, name="note\udc80x")))

    def test_safety_keywords_match_overrides_rs(self):
        m = self.load()
        with open(os.path.join(ROOT, "src", "overrides.rs"), encoding="utf-8") as f:
            src = f.read()
        mo = re.search(r"pub const SAFETY_KEYWORDS: &\[&str\] = &\[(.*?)\];", src, re.S)
        self.assertIsNotNone(mo)
        rust = tuple(re.findall(r'"((?:[^"\\]|\\.)*)"', mo.group(1)))
        self.assertGreater(len(rust), 10)
        self.assertEqual(tuple(m.SAFETY_KEYWORDS), rust)

    def test_propose_records_without_a_feed_claim(self):
        rig = Rig()
        try:
            res = rig.learn(["propose", "--reason", "stuck", "--topic", "x", "--json"])
            self.assertEqual(res.returncode, 0, res.stderr)
            ev = rig.ledger()[-1]
            self.assertEqual(ev["status"], "recorded")
            self.assertNotIn("awaiting_approval", res.stdout)
        finally:
            rig.close()

    def test_learn_registers_no_skill(self):
        with open(os.path.join(BIN, "javis_learn.py"), encoding="utf-8") as f:
            src = f.read()
        for needle in ('"skill", "new"', "'skill', 'new'", "cys skill", ".learned.json"):
            self.assertNotIn(needle, src)


# ───────────────────────────── §6.6 T-11e'..q: per-entry revert ─────────────────────────────

class Revert(RigCase):
    def stored(self, rig, name):
        res = rig.store(plain_pattern(domain=name.replace("-", " ")), name=name)
        self.assertStored(rig, res)
        rec = rig.stored_rec(name)
        self.assertIsNotNone(rec)
        return rec

    def revert(self, rig, name, flags=None):
        if flags:
            return rig.driver(flags, ["revert", "--entry", name, "--json"])
        return rig.learn(["revert", "--entry", name, "--json"])

    def reverted_lines(self, rig):
        return [e for e in rig.ledger() if e.get("event") == "reverted"]

    def test_11e_preexisting_content_kept(self):
        rig = self.rig()
        user = INDEX0 + "- [one](feedback_one.md) — one\n- [two](feedback_two.md) — two\nfree text line\n"
        with open(rig.index, "w", encoding="utf-8", newline="\n") as f:
            f.write(user)
        for n in ("one", "two"):
            with open(os.path.join(rig.mdir, "feedback_%s.md" % n), "w", encoding="utf-8", newline="\n") as f:
                f.write("---\nname: %s\ndescription: %s\nmetadata:\n  type: feedback\n---\n\nuser note\n" % (n, n))
        before = rig.snapshot()
        rec = self.stored(rig, "cache-a")
        res = self.revert(rig, "cache-a")
        self.assertEqual(res.returncode, 0, res.stderr)
        after = rig.snapshot()
        trash = [k for k in after if k.startswith(".learned-trash")]
        self.assertEqual(len(trash), 1)
        self.assertTrue(trash[0].endswith(rec["entry_file"]))
        del after[trash[0]]
        self.assertEqual(after, before)
        self.assertEqual(len(self.reverted_lines(rig)), 1)
        self.assertEqual(rig.memory(["verify"]).returncode, 0)

    def test_11f_later_user_edit_wins(self):
        # "json value" keeps a valid learning body, so only the sha check can refuse it (mutation M15 survived the
        # "appended text" edit alone: that one is also refused by the body-scope check).
        for edit in ("appended text", "json value"):
            with self.subTest(edit=edit):
                rig = self.rig()
                rec = self.stored(rig, "cache-a")
                p = os.path.join(rig.mdir, rec["entry_file"])
                with open(p, "rb") as f:
                    raw = f.read()
                if edit == "appended text":
                    raw += b"\nuser addition\n"
                else:
                    self.assertEqual(raw.count(b"a rebuild is expected"), 1)   # the body only (desc has the action)
                    raw = raw.replace(b"a rebuild is expected", b"a rebuild is expected soon")
                with open(p, "wb") as f:
                    f.write(raw)
                before = rig.snapshot()
                ino = os.stat(rig.index).st_ino
                res = self.revert(rig, "cache-a")
                self.assertEqual(res.returncode, 7, res.stderr)
                self.assertEqual(rig.snapshot(), before)
                # refused at step 2: the index was never rewritten (a rewrite + restore keeps bytes, not the inode)
                self.assertEqual(os.stat(rig.index).st_ino, ino)
                self.assertEqual(self.reverted_lines(rig), [])

    def test_11g_selective(self):
        rig = self.rig()
        a = self.stored(rig, "cache-a")
        b = self.stored(rig, "cache-b")
        with open(os.path.join(rig.mdir, b["entry_file"]), "rb") as f:
            b_bytes = f.read()
        self.assertEqual(self.revert(rig, "cache-a").returncode, 0)
        idx = rig.read_index().decode("utf-8")
        self.assertNotIn(a["index_line"], idx.split("\n"))
        self.assertEqual(idx.split("\n").count(b["index_line"]), 1)
        with open(os.path.join(rig.mdir, b["entry_file"]), "rb") as f:
            self.assertEqual(f.read(), b_bytes)
        self.assertFalse(os.path.exists(os.path.join(rig.mdir, a["entry_file"])))

    def test_11j_concurrent_add_and_remove_both_orders(self):
        for order in ("add-first", "remove-first"):
            with self.subTest(order=order):
                rig = self.rig()
                a = self.stored(rig, "cache-a")
                holder_py = os.path.join(rig.d, "holder.py")
                with open(holder_py, "w", encoding="utf-8") as f:
                    f.write(LOCK_HOLDER)
                holder = subprocess.Popen([sys.executable, "-B", holder_py, os.path.join(rig.bin, "javis_memory.py"),
                                           rig.index], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
                                          env=rig.env)
                self.assertEqual(holder.stdout.readline().strip(), "held")
                base = [sys.executable, "-B", os.path.join(rig.bin, "javis_memory.py"), "--dir", rig.mdir]
                add = base + ["add", "--type", "feedback", "--name", "note-b", "--desc", "b", "--body", "b body"]
                rem = base + ["remove", "--name", a["entry_file"], "--expect-sha", a["entry_sha256"],
                              "--expect-line", a["index_line"]]
                first, second = (add, rem) if order == "add-first" else (rem, add)
                p1 = subprocess.Popen(first, env=rig.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                time.sleep(0.3)
                p2 = subprocess.Popen(second, env=rig.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                time.sleep(0.5)
                holder.stdin.write("go\n")
                holder.stdin.flush()
                holder.wait(timeout=10)
                o1, e1 = p1.communicate(timeout=20)
                o2, e2 = p2.communicate(timeout=20)
                self.assertEqual((p1.returncode, p2.returncode), (0, 0), (e1, e2))
                idx = rig.read_index().decode("utf-8").split("\n")
                self.assertNotIn(a["index_line"], idx)
                self.assertEqual(sum(1 for x in idx if "(feedback_note-b.md)" in x), 1)
                self.assertTrue(os.path.exists(os.path.join(rig.mdir, "feedback_note-b.md")))
                self.assertEqual(len(rig.trash_files()), 1)
                self.assertEqual(rig.memory(["verify"]).returncode, 0, rig.memory(["verify"]).stdout)

    def load_memory(self, rig):
        spec = importlib.util.spec_from_file_location("javis_memory_rig", os.path.join(rig.bin, "javis_memory.py"))
        m = importlib.util.module_from_spec(spec)
        with contextlib.redirect_stderr(io.StringIO()):
            spec.loader.exec_module(m)
        return m

    def remove_in_process(self, m, rig, rec):
        ns = argparse.Namespace(name=rec["entry_file"], expect_sha=rec["entry_sha256"], expect_line=rec["index_line"])
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return m.cmd_remove(rig.mdir, ns)

    def test_11k_failure_at_each_boundary(self):
        def boom(*_a, **_k):
            raise OSError("injected")
        for step in ("makedirs", "index-write", "move", "move-and-restore"):
            with self.subTest(step=step):
                rig = self.rig()
                rec = self.stored(rig, "cache-a")
                before = rig.snapshot()
                m = self.load_memory(rig)
                if step == "makedirs":
                    m._make_trash_dir = boom
                elif step == "index-write":
                    m._write_bytes_atomic = boom
                elif step == "move":
                    m._move_entry = boom
                else:
                    m._move_entry = boom
                    real = m._write_bytes_atomic
                    calls = []

                    def first_only(path, data, _real=real, _calls=calls):
                        _calls.append(path)
                        if len(_calls) > 1:
                            raise OSError("injected restore failure")
                        return _real(path, data)
                    m._write_bytes_atomic = first_only
                rc = self.remove_in_process(m, rig, rec)
                after = {k: v for k, v in rig.snapshot().items() if not k.startswith(".learned-trash")}
                if step != "move-and-restore":
                    self.assertEqual(rc, 3)
                    self.assertEqual(after, before)
                else:
                    self.assertEqual(rc, 4)
                    self.assertTrue(os.path.exists(os.path.join(rig.mdir, rec["entry_file"])))
                    self.assertNotIn(rec["index_line"], rig.read_index().decode("utf-8").split("\n"))
                    v = rig.memory(["verify"])
                    self.assertEqual(v.returncode, 1)
                    self.assertIn(rec["entry_file"], v.stdout)

    def exit5(self, rig, name="cache-a"):
        rec = self.stored(rig, name)
        res = self.revert(rig, name, flags="ledgerfail")
        self.assertEqual(res.returncode, 5, res.stdout + res.stderr)
        self.assertFalse(os.path.exists(os.path.join(rig.mdir, rec["entry_file"])))
        self.assertNotIn(rec["index_line"], rig.read_index().decode("utf-8").split("\n"))
        self.assertEqual(self.reverted_lines(rig), [])
        return rec

    def test_11k_ledger_failure_then_recovery(self):
        rig = self.rig()
        rec = self.exit5(rig)
        st = rig.learn(["status"])
        self.assertIn("stored-but-missing", st.stdout)
        res = self.revert(rig, "cache-a")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        lines = self.reverted_lines(rig)
        self.assertEqual(len(lines), 1)
        self.assertTrue(lines[0].get("recovered"))
        self.assertEqual(lines[0].get("trash"), rig.trash_files()[0])
        self.assertTrue(lines[0]["trash"].endswith(rec["entry_file"]))

    def test_11q_recovery_controls(self):
        for case, want in (("moved-away", "no matching trash file"), ("bytes-changed", "hash differs"),
                           ("line-restored", "index still has the line"), ("two-copies", "several matches")):
            with self.subTest(case=case):
                rig = self.rig()
                rec = self.exit5(rig)
                t = rig.trash_files()[0]
                if case == "moved-away":
                    os.rename(t, os.path.join(rig.d, "moved"))
                elif case == "bytes-changed":
                    with open(t, "ab") as f:
                        f.write(b"x")
                elif case == "line-restored":
                    with open(rig.index, "ab") as f:
                        f.write((rec["index_line"] + "\n").encode("utf-8"))
                else:
                    other = os.path.join(rig.mdir, ".learned-trash", "20000101T000000Z")
                    os.makedirs(other)
                    shutil.copy2(t, os.path.join(other, os.path.basename(t)))
                res = self.revert(rig, "cache-a")
                self.assertEqual(res.returncode, 7, res.stdout + res.stderr)
                self.assertIn(want, res.stdout + res.stderr)
                self.assertEqual(self.reverted_lines(rig), [])

    def test_second_revert_claims_nothing_new(self):
        rig = self.rig()
        self.stored(rig, "cache-a")
        self.assertEqual(self.revert(rig, "cache-a").returncode, 0)
        res = self.revert(rig, "cache-a")
        self.assertEqual(res.returncode, 7, res.stdout + res.stderr)
        self.assertEqual(len(self.reverted_lines(rig)), 1)
        self.assertIn("reverted", rig.learn(["status"]).stdout)

    def test_entry_edited_inside_the_window_is_refused(self):
        # The second hash (step 5) is what refuses an entry changed after the step-2 checks. The race itself is the
        # stated residue; this injects the edit deterministically right after the index write.
        rig = self.rig()
        rec = self.stored(rig, "cache-a")
        index_before = rig.read_index()
        m = self.load_memory(rig)
        real = m._write_bytes_atomic
        entry = os.path.join(rig.mdir, rec["entry_file"])

        def write_then_edit(path, data, _real=real):
            _real(path, data)
            if path == rig.index and data != index_before:
                with open(entry, "ab") as f:
                    f.write(b"edited in the window\n")
        m._write_bytes_atomic = write_then_edit
        rc = self.remove_in_process(m, rig, rec)
        self.assertEqual(rc, 7)
        self.assertEqual(rig.read_index(), index_before)
        with open(entry, "rb") as f:
            self.assertTrue(f.read().endswith(b"edited in the window\n"))
        self.assertEqual(rig.trash_files(), [])

    def test_11l_uncoordinated_index_edit_refused(self):
        rig = self.rig()
        rec = self.stored(rig, "cache-a")
        idx = rig.read_index().decode("utf-8").replace(rec["index_line"] + "\n", "")
        with open(rig.index, "w", encoding="utf-8", newline="\n") as f:
            f.write(idx)
        before = rig.snapshot()
        res = self.revert(rig, "cache-a")
        self.assertEqual(res.returncode, 7)
        self.assertEqual(rig.snapshot(), before)

    def test_11p_user_entry_refused(self):
        rig = self.rig()
        r = rig.memory(["add", "--type", "feedback", "--name", "mine", "--desc", "mine", "--body", "my note"])
        self.assertEqual(r.returncode, 0, r.stderr)
        p = os.path.join(rig.mdir, "feedback_mine.md")
        with open(p, "rb") as f:
            s = sha(f.read())
        line = "- [mine](feedback_mine.md) — mine"
        before = rig.snapshot()
        res = rig.memory(["remove", "--name", "feedback_mine.md", "--expect-sha", s, "--expect-line", line])
        self.assertEqual(res.returncode, 7, res.stdout + res.stderr)
        self.assertEqual(rig.snapshot(), before)

    def test_crlf_index_keeps_every_other_byte(self):
        rig = self.rig()
        rec = self.stored(rig, "cache-a")
        crlf = rig.read_index().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
        with open(rig.index, "wb") as f:
            f.write(crlf)
        res = self.revert(rig, "cache-a")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertEqual(rig.read_index(), crlf.replace((rec["index_line"] + "\r\n").encode("utf-8"), b""))

    @unittest.skipUnless(os.name == "nt", "T-11m: a file held open blocks os.replace only on Windows")
    def test_11m_windows_held_index_and_entry(self):
        holder_src = "import sys\nf = open(sys.argv[1], 'rb')\nprint('held', flush=True)\nsys.stdin.readline()\n"
        for target in ("index", "entry"):
            with self.subTest(target=target):
                rig = self.rig()
                rec = self.stored(rig, "cache-a")
                hp = os.path.join(rig.d, "hold.py")
                with open(hp, "w", encoding="utf-8") as f:
                    f.write(holder_src)
                path = rig.index if target == "index" else os.path.join(rig.mdir, rec["entry_file"])
                # r2fix B1: Popen as a context manager closes the holder's stdin and stdout and reaps it, so the case
                # leaves no ResourceWarning (unittest.main shows warnings; one between ' ... ' and 'ok' broke the
                # windows-health line check). Closing stdin also ends the holder if an assertion below fails.
                with subprocess.Popen([sys.executable, "-B", hp, path], stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, text=True) as h:
                    self.assertEqual(h.stdout.readline().strip(), "held")
                    before = rig.snapshot()
                    try:
                        res = self.revert(rig, "cache-a")
                    finally:
                        h.stdin.write("x\n")
                        h.stdin.flush()
                        h.wait(timeout=10)
                self.assertEqual(res.returncode, 3, res.stdout + res.stderr)
                after = {k: v for k, v in rig.snapshot().items() if not k.startswith(".learned-trash")}
                self.assertEqual(after, before)


if __name__ == "__main__":
    unittest.main(verbosity=2)
