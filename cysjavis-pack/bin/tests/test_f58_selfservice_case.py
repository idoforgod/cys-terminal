#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_f58_selfservice_case.py — F58 T-1, T-2a..T-2p, T-3: the self-service case tool `javis_selfservice.py`.

What it checks (DESIGN-F58-final.md §2; DESIGN-F58-r2.md §2, §8.2; DESIGN-F58-r1.md C3, §3, §6, §7):
  T-1   the 59 classified report rows (fixture `fixtures/self_service_reports.json`, parsed from r1 section 6)
        each select, through `match_entry`, the seeded lever-table entry built from that row; its verdict,
        first lever class and risky flag equal the row's. Stated limit: the table is built from the same rows
        (P-6), so T-1 checks provenance and encoding, not generalisation. Three rows also run open/triage
        through the CLI.
  T-2   the case state machine and the report gate (r2 T-2a..T-2f, final T-2g..T-2p), one method each, plus:
        an ask_once attempt without consent (exit 6, nothing written), NO_MATCH (exit 8), unknown case (3),
        the three probe values (unknown is never success), stored values never read, re-plan after regressed.
  T-3   no secret in any case file: env tokens, the HOME literal, an approval signature in probe output, a
        plain env value (no env dump) and the evidence file's content: 0 hits in every file under the case.

Every case runs the real tool from this checkout with a temp HOME and a temp pack (CYS_PACK_DIR) that holds
the test's own lever table; probes are `{python} -c` stubs reading a per-test state file in the pack, so
each probe's exit and output is set by the test; the timeout probe sleeps past a 1 s timeout. A stub `cys`
on PATH answers `--version`. Children run with -B.
Run: python3 cysjavis-pack/bin/tests/test_f58_selfservice_case.py
"""
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
PACK = os.path.dirname(BIN)
TOOL = os.path.join(BIN, "javis_selfservice.py")
LEVERS = os.path.join(PACK, "round", "self_service_levers.json")
FIXTURE = os.path.join(TESTS_DIR, "fixtures", "self_service_reports.json")

SF, SMPD, PDNL, ON = ("self_fixable", "self_mitigable_product_defect", "product_defect_no_lever", "owner_needed")
VERDICT_OF = {"S": SF, "S+R": SMPD, "R": PDNL, "O": ON}
RC_OK, RC_NOT_FIXED, RC_USAGE, RC_NO_CASE, RC_REDIRECT, RC_NO_ATTEMPT, RC_NEEDS_YES, RC_DECLINED, RC_NO_MATCH = range(9)
SR_SENTENCE = ("지금 쓰시는 데는 문제없게 막았습니다. 남는 원인은 제품 안에 있어 보입니다 — "
               "고친 기록을 붙여 제조사 보고 초안을 만들어 둘까요?")
NO_CASE_ID = "20990101T000000Z-abcdef"
PASS_TEXT, FAIL_TEXT = "PROBE-PASS", "PROBE-FAIL"
STUB = ("import json,sys,time\n"
        "s=json.load(open(sys.argv[1],encoding='utf-8')).get(sys.argv[2],{})\n"
        "time.sleep(s.get('sleep',0))\n"
        "sys.stdout.write(s.get('out',''))\n"
        "sys.stdout.flush()\n"
        "sys.exit(s.get('exit',0))\n")
STATE_OF = {"pass": {"exit": 0, "out": PASS_TEXT + "\n"},
            "fail": {"exit": 1, "out": FAIL_TEXT + "\n"},
            "timeout": {"exit": 0, "out": PASS_TEXT + "\n", "sleep": 4}}


def sha(b):
    return hashlib.sha256(b).hexdigest()


def probe(pid, timeout_s=20, predicate=None, argv=None):
    return {"argv": argv or ["{python}", "-c", STUB, "{pack}/probe_state.json", pid], "timeout_s": timeout_s,
            "ok_exit": 0, "predicate": predicate or {"kind": "contains", "text": PASS_TEXT}}


def lever(lid, cls="T1", risky=False, applicable=True, rb="rb"):
    return {"id": lid, "class": cls, "action": "set the widget option " + lid, "risky": risky,
            "applicable": applicable, "not_applicable_reason": None if applicable else "not on this OS",
            "readback_probe": rb}


def entry(eid, verdict, levers, symptom="sym", defect="def", match=None):
    e = {"id": eid, "source_rows": ["test-" + eid], "title": "test entry " + eid,
         "match": match or [eid + " broken"], "verdict": verdict, "symptom_probe": symptom}
    if symptom == "none":
        e["symptom_probe_reason"] = "test: no probe"
    if verdict in (SMPD, PDNL):
        e["defect_probe"] = defect
        if defect == "none":
            e["defect_probe_reason"] = "test: no probe"
    e["levers"] = levers
    return e


def load_tool():
    spec = importlib.util.spec_from_file_location("javis_selfservice_under_test", TOOL)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


class Rig(object):
    """Temp HOME + temp pack (its own lever table and probe state) + stub `cys`; runs the real tool."""

    def __init__(self, entries=None, table=None, timeouts=None, extra_probes=None, cys_version="cys 0.14.57-test",
                 pack_version="0.14.57-test\n", extra_env=None):
        self.d = tempfile.mkdtemp(prefix="f58-ss-")
        self.home = os.path.join(self.d, "home")
        self.pack = os.path.join(self.d, "pack")
        self.stub = os.path.join(self.d, "stub")
        for p in (self.home, os.path.join(self.pack, "round"), self.stub):
            os.makedirs(p)
        if table is None:
            timeouts = timeouts or {}
            probes = {pid: probe(pid, timeouts.get(pid, 20)) for pid in ("sym", "def", "rb", "rb2")}
            probes.update(extra_probes or {})
            table = {"schema": 1, "classes": {"T1": "setting"}, "probes": probes, "entries": entries or []}
        self.write_json(os.path.join(self.pack, "round", "self_service_levers.json"), table)
        self.probe_state = {}
        self.set(sym="fail", **{"def": "fail", "rb": "fail", "rb2": "fail"})
        if pack_version is not None:
            with open(os.path.join(self.pack, ".pack-version"), "w", encoding="utf-8") as f:
                f.write(pack_version)
        if cys_version is not None:
            with open(os.path.join(self.stub, "cys"), "w", encoding="utf-8", newline="\n") as f:
                f.write("#!/bin/sh\necho '%s'\n" % cys_version)
            os.chmod(os.path.join(self.stub, "cys"), 0o755)
            with open(os.path.join(self.stub, "cys.cmd"), "w", encoding="utf-8") as f:
                f.write("@echo %s\r\n" % cys_version)
        env = {k: v for k, v in os.environ.items()
               if not (k.startswith("CYS_") or k.startswith("JAVIS_") or k == "CLAUDE_CONFIG_DIR")}
        path = self.stub if cys_version is None else self.stub + os.pathsep + env.get("PATH", "/usr/bin:/bin")
        env.update({"HOME": self.home, "USERPROFILE": self.home, "CYS_PACK_DIR": self.pack, "PATH": path})
        env.update(extra_env or {})
        self.env = env
        self.n = 0

    def close(self):
        shutil.rmtree(self.d, ignore_errors=True)

    def write_json(self, path, obj):
        with open(path, "w", encoding="utf-8") as f:
            json.dump(obj, f, ensure_ascii=False)

    def set(self, **values):
        for pid, v in values.items():
            self.probe_state[pid] = STATE_OF[v] if isinstance(v, str) else v
        self.write_json(os.path.join(self.pack, "probe_state.json"), self.probe_state)

    def run(self, *args):
        return subprocess.run([sys.executable, "-B", TOOL] + list(args), cwd=self.d, env=self.env,
                              capture_output=True, text=True, encoding="utf-8")

    def open(self, symptom):
        r = self.run("open", "--symptom", symptom)
        assert r.returncode == 0, "open rc=%d\n%s\n%s" % (r.returncode, r.stdout, r.stderr)
        cid = r.stdout.strip().splitlines()[0].strip()
        assert re.match(r"^\d{8}T\d{6}Z-[0-9a-f]{6}$", cid), r.stdout
        return cid

    def case_dir(self, cid):
        return os.path.join(self.home, ".cys", "local", "self-service", cid)

    def case(self, cid):
        return load_json(os.path.join(self.case_dir(cid), "case.json"))

    def attempts(self, cid):
        p = os.path.join(self.case_dir(cid), "attempts.json")
        if not os.path.exists(p):
            return []
        doc = load_json(p)
        return doc["attempts"] if isinstance(doc, dict) else doc

    def report_file(self, cid):
        return os.path.join(self.case_dir(cid), "vendor-report.txt")

    def report_text(self, cid):
        with open(self.report_file(cid), encoding="utf-8") as f:
            return f.read()

    def evidence(self, content="evidence: the widget setting was changed\n"):
        self.n += 1
        p = os.path.join(self.d, "evidence-%d.txt" % self.n)
        with open(p, "w", encoding="utf-8") as f:
            f.write(content)
        return p

    def snapshot(self, cid):
        out = {}
        root = self.case_dir(cid)
        for dp, _dn, fns in os.walk(root):
            for fn in fns:
                p = os.path.join(dp, fn)
                with open(p, "rb") as f:
                    out[os.path.relpath(p, root)] = f.read()
        return out


class Base(unittest.TestCase):
    maxDiff = None

    def rig(self, *a, **kw):
        r = Rig(*a, **kw)
        self.addCleanup(r.close)
        return r

    def rc(self, res, want, msg=""):
        self.assertEqual(res.returncode, want, "%s want rc %d, got %d\nstdout=%s\nstderr=%s"
                         % (msg, want, res.returncode, res.stdout[-1500:], res.stderr[-1500:]))
        return res

    def state(self, rig, cid, want):
        c = rig.case(cid)
        self.assertEqual(c["state"], want, "state %r, want %r; history=%r" % (c["state"], want, c.get("history")))
        return c

    def triaged(self, rig, symptom):
        cid = rig.open(symptom)
        self.rc(rig.run("triage", "--case", cid), RC_OK, "triage")
        return cid

    def attempt(self, rig, cid, lid, want=RC_OK):
        return self.rc(rig.run("attempt", "--case", cid, "--lever", lid, "--action", "changed the option " + lid,
                               "--evidence", rig.evidence()), want, "attempt " + lid)

    def no_report_file(self, rig, cid):
        self.assertFalse(os.path.exists(rig.report_file(cid)), "vendor-report.txt was written")
        self.assertFalse(os.path.exists(os.path.join(rig.case_dir(cid), "report-evidence")), "report-evidence")

    def to_mwr(self, rig_kw=None, defect_at_verify="fail"):
        """T-2g state: S+R entry, L1 applied, symptom pass, defect probe still failing -> mitigated_with_residual."""
        kw = {"entries": [entry("wid", SMPD, [lever("L1")])]}
        kw.update(rig_kw or {})
        rig = self.rig(**kw)
        rig.set(sym="fail", **{"def": "fail", "rb": "pass"})
        cid = self.triaged(rig, "the wid broken since update")
        rig.set(sym="pass", **{"def": defect_at_verify})
        self.attempt(rig, cid, "L1")
        self.state(rig, cid, "attempting")
        self.rc(rig.run("verify", "--case", cid), RC_OK, "verify")
        return rig, cid

    def reported(self, rig, cid):
        self.rc(rig.run("consent", "--case", cid, "--report", "--yes"), RC_OK, "consent report yes")
        self.rc(rig.run("report", "--case", cid), RC_OK, "report")
        self.assertTrue(os.path.exists(rig.report_file(cid)))
        return self.state(rig, cid, "reported")


# ─────────────────────────────── T-1 ───────────────────────────────

class T1Rows(Base):
    def rows(self):
        return load_json(FIXTURE)["rows"]

    def test_1_fixture_has_59_rows_with_the_planned_counts(self):
        rows = self.rows()
        self.assertEqual(len(rows), 59)
        self.assertEqual(len({r["id"] for r in rows}), 59)
        counts = {k: sum(1 for r in rows if r["verdict"] == k) for k in ("S", "S+R", "R", "O")}
        self.assertEqual(counts, {"S": 12, "S+R": 28, "R": 16, "O": 3})
        for r in rows:
            for k in ("id", "item", "verdict", "first_answer", "first_action", "vendor_report"):
                self.assertTrue(isinstance(r.get(k), str) and r[k], (r.get("id"), k))
            self.assertTrue(r["item"].startswith(r["id"]), r["id"])
            self.assertIn(r["lever_class"], ("T1", "T2", "T3", "T4", "T5", "T6", "T7"), r["id"])
            self.assertIs(type(r["risky"]), bool, r["id"])

    def test_1_every_row_selects_its_own_seeded_entry(self):
        m = load_tool()
        table = load_json(LEVERS)
        rows = self.rows()
        self.assertEqual(len(table["entries"]), 59)
        self.assertEqual(sorted(e["source_rows"][0] for e in table["entries"]), sorted(r["id"] for r in rows))
        for r in rows:
            e = m.match_entry(table, r["item"])
            self.assertIsNotNone(e, r["id"])
            self.assertEqual(e["source_rows"], [r["id"]], r["id"])
            self.assertEqual(e["verdict"], VERDICT_OF[r["verdict"]], r["id"])
            first = e["levers"][0]
            self.assertEqual(first["class"], r["lever_class"], r["id"])
            self.assertIs(first["risky"], r["risky"], r["id"])
            self.assertEqual(first["action"], r["first_action"], r["id"])

    def test_1_seed_entries_follow_the_schema(self):
        table = load_json(LEVERS)
        self.assertEqual(table["schema"], 1)
        self.assertEqual(sorted(table["classes"]), ["T1", "T2", "T3", "T4", "T5", "T6", "T7"])
        items = {r["id"]: r["item"].lower() for r in self.rows()}
        for e in table["entries"]:
            self.assertTrue(2 <= len(e["match"]) <= 4, e["id"])
            for s in e["match"]:
                self.assertEqual(s, s.lower(), e["id"])
                self.assertIn(s, items[e["source_rows"][0]], e["id"])
            self.assertEqual(e["symptom_probe"], "none", e["id"])
            self.assertTrue(e.get("symptom_probe_reason"), e["id"])
            if e["verdict"] in (SMPD, PDNL):
                self.assertIn("defect_probe", e, e["id"])
                if e["defect_probe"] == "none":
                    self.assertTrue(e.get("defect_probe_reason"), e["id"])
            else:
                self.assertNotIn("defect_probe", e, e["id"])
            lv = e["levers"][0]
            self.assertEqual(lv["id"], "L1")
            self.assertIs(lv["applicable"], True)
            self.assertEqual(lv["readback_probe"], "none")

    def test_1_three_rows_end_to_end_through_the_cli(self):
        table = load_json(LEVERS)
        rows = self.rows()
        picks = [next(r for r in rows if r["verdict"] == v) for v in ("S", "S+R", "O")]
        rig = self.rig(table=table)
        by_row = {e["source_rows"][0]: e for e in table["entries"]}
        for r in picks:
            cid = self.triaged(rig, r["item"])
            c = self.state(rig, cid, "triaged")
            self.assertEqual(c["entry"], by_row[r["id"]]["id"], r["id"])
            self.assertEqual(c["entry_source"], "matched")
            self.assertEqual(c["verdict"], VERDICT_OF[r["verdict"]])
            lv = c["plan"]["levers"][0]
            self.assertEqual(lv["class"], r["lever_class"])
            self.assertIs(lv["ask_once"], r["risky"])
            sym = [o for o in c["observations"] if o["role"] == "symptom"]
            self.assertTrue(sym and all(o["value"] == "unknown" for o in sym), sym)
            if r["verdict"] == "S+R":
                self.assertEqual(c["defect_probe"]["value"], "unknown")
            else:
                self.assertIsNone(c.get("defect_probe"))


# ─────────────────────────────── T-2 ───────────────────────────────

class T2Cases(Base):
    def test_2a_failed_repair_with_readback_reaches_product_defect_and_reports(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1")])])
        rig.set(sym="fail", rb="pass")
        cid = self.triaged(rig, "the wid broken today")
        self.state(rig, cid, "triaged")
        self.attempt(rig, cid, "L1")
        self.state(rig, cid, "exhausted")
        a = rig.attempts(cid)[-1]
        self.assertEqual((a["before"]["value"], a["after"]["value"], a["readback"]["value"], a["verify"]),
                         ("fail", "fail", "pass", "fail"))
        self.rc(rig.run("report", "--case", cid), RC_NO_ATTEMPT, "report before retriage")
        self.no_report_file(rig, cid)
        self.rc(rig.run("retriage", "--case", cid), RC_OK, "retriage")
        self.state(rig, cid, "product_defect")
        self.rc(rig.run("report", "--case", cid), RC_NEEDS_YES, "report without consent")
        self.no_report_file(rig, cid)
        c = self.reported(rig, cid)
        self.assertEqual(c["from_state"], "product_defect")
        text = rig.report_text(cid)
        self.assertIn("root_cause: unproven", text)
        for h in (a["before"]["out_sha256"], a["after"]["out_sha256"], a["readback"]["out_sha256"],
                  a["evidence"]["sha256"]):
            self.assertRegex(h, r"^[0-9a-f]{64}$")
            self.assertIn(h, text)
        self.assertIn("product_defect", text)
        self.assertTrue(os.path.exists(os.path.join(rig.case_dir(cid), "report-evidence", "attempts.json")))

    def test_2b_unknown_probe_timeout_is_never_success(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1"), lever("L2")])], timeouts={"sym": 1})
        rig.set(sym="timeout", rb="pass")
        cid = self.triaged(rig, "wid broken")
        self.attempt(rig, cid, "L1")
        self.assertEqual(rig.attempts(cid)[-1]["verify"], "unknown")
        self.state(rig, cid, "attempting")
        self.rc(rig.run("verify", "--case", cid), RC_NOT_FIXED, "verify with unknown")
        self.state(rig, cid, "attempting")
        self.rc(rig.run("recheck"), RC_OK, "recheck sweep")
        self.state(rig, cid, "attempting")
        self.attempt(rig, cid, "L2")
        self.state(rig, cid, "exhausted")
        self.rc(rig.run("retriage", "--case", cid), RC_OK)
        c = self.state(rig, cid, "unresolved")
        self.assertEqual(c["reason"], "unknown_probe")
        self.rc(rig.run("report", "--case", cid), RC_NEEDS_YES)
        self.reported(rig, cid)
        self.assertNotIn("fixed", [h["to"] for h in rig.case(cid)["history"]])

    def test_2b_unknown_probe_unparseable_output_is_never_success(self):
        skip = probe("symj", predicate={"kind": "json_path_eq", "path": "result", "value": "ok"})
        rig = self.rig(entries=[entry("wid", SF, [lever("L1")], symptom="symj")], extra_probes={"symj": skip})
        rig.set(symj={"exit": 0, "out": "SKIP: socket checks are not available on this OS\n"}, rb="pass")
        cid = self.triaged(rig, "wid broken")
        self.attempt(rig, cid, "L1")
        self.assertEqual(rig.attempts(cid)[-1]["verify"], "unknown")
        self.state(rig, cid, "exhausted")
        self.rc(rig.run("verify", "--case", cid), RC_NOT_FIXED)
        self.state(rig, cid, "exhausted")
        self.rc(rig.run("retriage", "--case", cid), RC_OK)
        self.assertEqual(self.state(rig, cid, "unresolved")["reason"], "unknown_probe")

    def test_2c_no_applicable_lever_exhausts_at_triage_then_reports(self):
        rig = self.rig(entries=[entry("wid", PDNL, [lever("L1", applicable=False)])])
        rig.set(sym="fail", **{"def": "fail"})
        cid = self.triaged(rig, "wid broken")
        c = self.state(rig, cid, "exhausted")
        self.assertEqual([h["to"] for h in c["history"]][-2:], ["triaged", "exhausted"])
        self.assertEqual(c["defect_probe"]["value"], "fail")
        self.rc(rig.run("report", "--case", cid), RC_NO_ATTEMPT, "exhausted -> retriage first")
        self.rc(rig.run("retriage", "--case", cid), RC_OK)
        self.assertEqual(self.state(rig, cid, "unresolved")["reason"], "no_lever")
        self.rc(rig.run("report", "--case", cid), RC_NEEDS_YES)
        self.no_report_file(rig, cid)
        self.reported(rig, cid)

    def test_2d_untried_applicable_lever_redirects_without_a_file(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1")])])
        cid = self.triaged(rig, "wid broken")
        r = self.rc(rig.run("report", "--case", cid), RC_REDIRECT)
        self.assertIn("L1", r.stdout)
        self.no_report_file(rig, cid)
        self.state(rig, cid, "triaged")

    def test_2e_declined_risky_lever_is_not_success_and_ends_unresolved(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1"), lever("L2", risky=True)])])
        cid = self.triaged(rig, "wid broken")
        self.attempt(rig, cid, "L1")
        self.state(rig, cid, "attempting")
        self.rc(rig.run("consent", "--case", cid, "--lever", "L2", "--no"), RC_OK)
        c = self.state(rig, cid, "exhausted")
        self.assertEqual({lv["id"]: lv["status"] for lv in c["plan"]["levers"]}["L2"], "declined")
        self.rc(rig.run("verify", "--case", cid), RC_NOT_FIXED)
        self.rc(rig.run("retriage", "--case", cid), RC_OK)
        c = self.state(rig, cid, "unresolved")
        self.assertEqual(c["reason"], "declined")
        self.assertNotIn("fixed", [h["to"] for h in c["history"]])

    def _hand_edit_pass(self, rig, cid):
        p = os.path.join(rig.case_dir(cid), "attempts.json")
        doc = load_json(p)
        lst = doc["attempts"] if isinstance(doc, dict) else doc
        for a in lst:
            a["verify"] = "pass"
            for k in ("after", "readback", "before"):
                if isinstance(a.get(k), dict):
                    a[k]["value"] = "pass"
        rig.write_json(p, doc)
        cp = os.path.join(rig.case_dir(cid), "case.json")
        c = load_json(cp)
        for o in c.get("observations", []):
            o["value"] = "pass"
        rig.write_json(cp, c)

    def test_2f_hand_edited_pass_is_never_read_by_verify_report_or_retriage(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1"), lever("L2")])])
        rig.set(sym="fail", rb="fail")
        # attempting: the Jarvis writes verify/after/readback "pass" by hand while the symptom still fails
        a = self.triaged(rig, "wid broken")
        self.attempt(rig, a, "L1")
        self._hand_edit_pass(rig, a)
        self.rc(rig.run("verify", "--case", a), RC_NOT_FIXED, "verify after hand edit")
        self.state(rig, a, "attempting")
        self.rc(rig.run("report", "--case", a), RC_REDIRECT, "report after hand edit")
        self.no_report_file(rig, a)
        # exhausted: the same hand edit after both levers failed
        b = self.triaged(rig, "wid broken")
        self.attempt(rig, b, "L1")
        self.attempt(rig, b, "L2")
        self.state(rig, b, "exhausted")
        self._hand_edit_pass(rig, b)
        self.rc(rig.run("verify", "--case", b), RC_NOT_FIXED)
        self.state(rig, b, "exhausted")
        self.rc(rig.run("report", "--case", b), RC_NO_ATTEMPT)
        self.rc(rig.run("retriage", "--case", b), RC_OK)
        c = self.state(rig, b, "unresolved")
        self.assertEqual(c["reason"], "no_effect_shown", "a stored readback pass was read")
        self.no_report_file(rig, b)

    def test_2f_hand_edited_pass_is_never_read_by_recheck(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1")])])
        cid = self.triaged(rig, "wid broken")
        rig.set(sym="pass")
        self.attempt(rig, cid, "L1")
        self.rc(rig.run("verify", "--case", cid), RC_OK)
        self.state(rig, cid, "fixed")
        rig.set(sym="fail")
        self._hand_edit_pass(rig, cid)
        self.rc(rig.run("recheck", "--case", cid), RC_NOT_FIXED)
        self.state(rig, cid, "regressed")

    def test_2g_mitigated_with_residual_reports_after_consent(self):
        rig, cid = self.to_mwr()
        c = self.state(rig, cid, "mitigated_with_residual")
        self.assertEqual(c["residual"]["value"], "fail")
        self.assertRegex(c["residual"]["out_sha256"], r"^[0-9a-f]{64}$")
        self.assertTrue(c["residual"]["at"])
        r = self.rc(rig.run("report", "--case", cid), RC_NEEDS_YES)
        self.assertIn(SR_SENTENCE, r.stdout)
        self.no_report_file(rig, cid)
        self.reported(rig, cid)
        text = rig.report_text(cid)
        self.assertIn("set the widget option L1", text)
        self.assertIn("root_cause: unproven", text)
        self.assertRegex(text, r"defect probe[^\n]*\bdef\b[^\n]*\bfail\b")
        self.assertIn(c["residual"]["out_sha256"], text)

    def test_2h_pure_self_fixable_is_fixed_and_redirects(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1")])])
        cid = self.triaged(rig, "wid broken")
        rig.set(sym="pass")
        self.attempt(rig, cid, "L1")
        self.rc(rig.run("verify", "--case", cid), RC_OK)
        self.state(rig, cid, "fixed")
        self.rc(rig.run("report", "--case", cid), RC_REDIRECT)
        self.no_report_file(rig, cid)

    def test_2i_residual_passes_gives_fixed(self):
        rig, cid = self.to_mwr(defect_at_verify="pass")
        self.state(rig, cid, "fixed")
        self.rc(rig.run("report", "--case", cid), RC_REDIRECT)
        self.no_report_file(rig, cid)

    def test_2j_residual_unknown_is_mitigated_never_fixed(self):
        rig, cid = self.to_mwr(rig_kw={"timeouts": {"def": 1}}, defect_at_verify="timeout")
        c = self.state(rig, cid, "mitigated_with_residual")
        self.assertEqual(c["residual"]["value"], "unknown")
        self.rc(rig.run("report", "--case", cid), RC_NEEDS_YES)
        self.reported(rig, cid)

    def test_2j_defect_probe_none_is_mitigated_never_fixed(self):
        rig, cid = self.to_mwr(rig_kw={"entries": [entry("wid", SMPD, [lever("L1")], defect="none")]})
        self.assertEqual(self.state(rig, cid, "mitigated_with_residual")["residual"]["value"], "unknown")

    def test_2k_fully_declined_plan_redirects_with_the_gui_sentence(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1", risky=True), lever("L2", risky=True)])])
        cid = self.triaged(rig, "wid broken")
        self.rc(rig.run("consent", "--case", cid, "--lever", "L1", "--no"), RC_OK)
        self.state(rig, cid, "triaged")
        self.rc(rig.run("consent", "--case", cid, "--lever", "L2", "--no"), RC_OK)
        self.state(rig, cid, "exhausted")
        self.rc(rig.run("retriage", "--case", cid), RC_OK)
        self.assertEqual(self.state(rig, cid, "unresolved")["reason"], "all_declined")
        r = self.rc(rig.run("report", "--case", cid), RC_REDIRECT)
        self.assertIn("피드백 보내기", r.stdout)
        self.assertIn("declined every fix", r.stdout)
        self.no_report_file(rig, cid)
        self.rc(rig.run("consent", "--case", cid, "--report", "--yes"), RC_OK)
        self.rc(rig.run("report", "--case", cid), RC_REDIRECT, "H10: consent does not open the gate")
        self.no_report_file(rig, cid)
        self.assertEqual(rig.attempts(cid), [])

    def test_2l_user_declines_the_report(self):
        rig, cid = self.to_mwr()
        self.rc(rig.run("consent", "--case", cid, "--report", "--no"), RC_OK)
        self.assertTrue(rig.case(cid).get("report_declined_at"))
        before = rig.snapshot(cid)
        for _ in range(2):
            r = self.rc(rig.run("report", "--case", cid), RC_DECLINED)
            self.assertNotIn(SR_SENTENCE, r.stdout)
            self.assertNotIn("?", r.stdout)
            self.no_report_file(rig, cid)
        self.assertEqual(rig.snapshot(cid), before, "report 7 wrote something")
        self.state(rig, cid, "mitigated_with_residual")

    def test_2m_reported_is_idempotent(self):
        rig, cid = self.to_mwr()
        self.rc(rig.run("consent", "--case", cid, "--report", "--yes"), RC_OK)
        r1 = self.rc(rig.run("report", "--case", cid), RC_OK)
        snap = rig.snapshot(cid)
        h = sha(snap["vendor-report.txt"])
        r2 = self.rc(rig.run("report", "--case", cid), RC_OK)
        path = os.path.realpath(rig.report_file(cid))
        self.assertTrue(any(os.path.realpath(x.split(": ", 1)[-1].strip()) == path for x in r1.stdout.splitlines()))
        self.assertTrue(any(os.path.realpath(x.split(": ", 1)[-1].strip()) == path for x in r2.stdout.splitlines()))
        self.assertEqual(rig.snapshot(cid), snap, "second report wrote something")
        self.assertEqual(sha(rig.snapshot(cid)["vendor-report.txt"]), h)

    def test_2n_mitigation_regresses(self):
        rig, cid = self.to_mwr()
        rig.set(sym="fail")
        self.rc(rig.run("recheck"), RC_NOT_FIXED, "recheck with a regressed case")
        self.state(rig, cid, "regressed")
        self.rc(rig.run("report", "--case", cid), RC_NO_ATTEMPT)

    def test_2o_reported_from_mitigated_then_recheck_regresses(self):
        rig, cid = self.to_mwr()
        self.reported(rig, cid)
        h = sha(rig.snapshot(cid)["vendor-report.txt"])
        rig.set(sym="fail")
        self.rc(rig.run("recheck", "--case", cid), RC_NOT_FIXED)
        self.state(rig, cid, "regressed")
        self.assertEqual(sha(rig.snapshot(cid)["vendor-report.txt"]), h, "the written report must stay")
        self.rc(rig.run("report", "--case", cid), RC_NO_ATTEMPT)

    def test_2o_reported_from_mitigated_then_recheck_fixed(self):
        rig, cid = self.to_mwr()
        self.reported(rig, cid)
        rig.set(sym="pass", **{"def": "pass"})
        self.rc(rig.run("recheck", "--case", cid), RC_OK)
        self.state(rig, cid, "fixed")
        self.assertTrue(os.path.exists(rig.report_file(cid)))

    def test_2o_reported_from_mitigated_residual_still_failing_stays_reported(self):
        rig, cid = self.to_mwr()
        self.reported(rig, cid)
        rig.set(sym="pass", **{"def": "fail"})
        self.rc(rig.run("recheck", "--case", cid), RC_OK)
        self.state(rig, cid, "reported")

    def test_2p_reported_from_product_defect_and_unresolved_stays_on_recheck(self):
        rig = self.rig(entries=[entry("pd", SF, [lever("L1")]),
                                entry("un", PDNL, [lever("L1", applicable=False)])])
        rig.set(sym="fail", rb="pass", **{"def": "fail"})
        pd = self.triaged(rig, "pd broken")
        self.attempt(rig, pd, "L1")
        self.rc(rig.run("retriage", "--case", pd), RC_OK)
        self.state(rig, pd, "product_defect")
        self.reported(rig, pd)
        un = self.triaged(rig, "un broken")
        self.rc(rig.run("retriage", "--case", un), RC_OK)
        self.state(rig, un, "unresolved")
        self.reported(rig, un)
        for cid, frm in ((pd, "product_defect"), (un, "unresolved")):
            for value in ("fail", "pass"):
                rig.set(sym=value)
                n0 = len(rig.case(cid)["observations"])
                h = sha(rig.snapshot(cid)["vendor-report.txt"])
                self.rc(rig.run("recheck", "--case", cid), RC_OK, "recheck %s %s" % (frm, value))
                c = self.state(rig, cid, "reported")
                self.assertEqual(c["from_state"], frm)
                obs = c["observations"][n0:]
                self.assertTrue(obs and obs[-1]["role"] == "symptom" and obs[-1]["value"] == value, obs)
                snap = rig.snapshot(cid)
                r = self.rc(rig.run("report", "--case", cid), RC_OK)
                self.assertIn("vendor-report.txt", r.stdout)
                self.assertEqual(rig.snapshot(cid), snap)
                self.assertEqual(sha(snap["vendor-report.txt"]), h)

    # ── the extra cells named in the U3 spec ──

    def test_attempt_on_ask_once_lever_without_consent_is_6_and_writes_nothing(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1", risky=True)])])
        cid = self.triaged(rig, "wid broken")
        ev = rig.evidence()
        before = rig.snapshot(cid)
        self.rc(rig.run("attempt", "--case", cid, "--lever", "L1", "--action", "x", "--evidence", ev), RC_NEEDS_YES)
        self.assertEqual(rig.snapshot(cid), before, "exit 6 wrote something")
        self.rc(rig.run("consent", "--case", cid, "--lever", "L1", "--yes"), RC_OK)
        self.rc(rig.run("attempt", "--case", cid, "--lever", "L1", "--action", "x", "--evidence", ev), RC_OK)
        self.assertEqual(len(rig.attempts(cid)), 1)

    def test_no_match_is_8_lists_every_entry_and_changes_nothing(self):
        rig = self.rig(entries=[entry("alpha", SF, [lever("L1")]), entry("beta", SF, [lever("L1")])])
        cid = rig.open("something entirely different happened")
        before = rig.snapshot(cid)
        r = self.rc(rig.run("triage", "--case", cid), RC_NO_MATCH)
        for eid in ("alpha", "beta"):
            self.assertRegex(r.stdout, r"(?m)^%s\s+test entry %s$" % (eid, eid))
        self.assertEqual(rig.snapshot(cid), before, "NO_MATCH changed the case")
        self.state(rig, cid, "opened")
        self.rc(rig.run("triage", "--case", cid, "--entry", "nope"), RC_USAGE)
        self.assertEqual(rig.snapshot(cid), before)
        self.rc(rig.run("triage", "--case", cid, "--entry", "beta"), RC_OK)
        c = self.state(rig, cid, "triaged")
        self.assertEqual((c["entry"], c["entry_source"]), ("beta", "chosen"))

    def test_match_prefers_the_highest_score_then_table_order(self):
        m = load_tool()
        table = {"entries": [{"id": "a", "match": ["disk", "slow"]}, {"id": "b", "match": ["disk", "slow", "fan"]},
                             {"id": "c", "match": ["disk"]}]}
        self.assertEqual(m.match_entry(table, "The DISK is Slow and the FAN is loud")["id"], "b")
        self.assertEqual(m.match_entry(table, "disk is slow")["id"], "a")
        self.assertEqual(m.match_entry(table, "disk")["id"], "a")
        self.assertIsNone(m.match_entry(table, "printer jam"))

    def test_unknown_case_is_3(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1")])])
        ev = rig.evidence()
        for bad in (NO_CASE_ID, "../../etc", "x"):
            for args in (("triage", "--case", bad), ("verify", "--case", bad), ("retriage", "--case", bad),
                         ("report", "--case", bad), ("status", "--case", bad), ("recheck", "--case", bad),
                         ("consent", "--case", bad, "--report", "--yes"),
                         ("attempt", "--case", bad, "--lever", "L1", "--action", "x", "--evidence", ev)):
                self.rc(rig.run(*args), RC_NO_CASE, " ".join(args))

    def test_probe_values_unknown_pass_fail(self):
        jp = {"kind": "json_path_eq", "path": "result.state", "value": "ok"}
        probes = {"nobin": probe("x", argv=[os.path.join(os.sep, "nonexistent-f58", "bin", "probe")]),
                  "badjson": probe("badjson", predicate=jp), "goodjson": probe("goodjson", predicate=jp),
                  "nojsonkey": probe("nojsonkey", predicate=jp), "slow": probe("slow", timeout_s=1),
                  "exit0text": probe("exit0text"), "exit1pass": probe("exit1pass"),
                  "notc": probe("notc", predicate={"kind": "not_contains", "text": "ERROR"}),
                  "plain": probe("plain", predicate={"kind": "none"})}
        cases = {"none": "unknown", "ghost": "unknown", "nobin": "unknown", "badjson": "unknown",
                 "nojsonkey": "unknown", "slow": "unknown", "goodjson": "pass", "exit0text": "fail",
                 "exit1pass": "fail", "notc": "pass", "plain": "fail"}
        rig = self.rig(entries=[entry(k, SF, [lever("L1")], symptom=k) for k in cases], extra_probes=probes)
        rig.set(badjson={"exit": 0, "out": "SKIP\n"}, goodjson={"exit": 0, "out": '{"result": {"state": "ok"}}'},
                nojsonkey={"exit": 0, "out": '{"result": {}}'}, slow="timeout",
                exit0text={"exit": 0, "out": FAIL_TEXT}, exit1pass={"exit": 1, "out": PASS_TEXT},
                notc={"exit": 0, "out": "all good"}, plain={"exit": 3, "out": ""})
        for k, want in cases.items():
            cid = self.triaged(rig, k + " broken")
            got = [o for o in rig.case(cid)["observations"] if o["role"] == "symptom"][-1]
            self.assertEqual(got["value"], want, k)
            self.assertIn(got["value"], ("pass", "fail", "unknown"))

    def test_open_records_versions_and_a_before_observation(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1")])])
        cid = rig.open("wid broken")
        c = self.state(rig, cid, "opened")
        self.assertEqual(c["versions"]["cys"], "cys 0.14.57-test")
        self.assertEqual(c["versions"]["pack"], "0.14.57-test")
        self.assertTrue(c["versions"]["os"])
        self.assertTrue(c["versions"]["arch"])
        obs = c["observations"]
        self.assertEqual(len(obs), 1, "r2 2.2: a before observation is taken at open")
        self.assertEqual((obs[0]["role"], obs[0]["probe"], obs[0]["value"]), ("symptom", "sym", "fail"))
        nocys = self.rig(entries=[entry("wid", SF, [lever("L1")])], cys_version=None, pack_version=None)
        c2 = nocys.case(nocys.open("nothing matches this"))
        self.assertEqual((c2["versions"]["cys"], c2["versions"]["pack"]), ("unknown", "unknown"))
        self.assertEqual(c2["observations"][0]["value"], "unknown")

    def test_every_state_change_is_in_history(self):
        rig, cid = self.to_mwr()
        self.reported(rig, cid)
        hist = rig.case(cid)["history"]
        self.assertEqual([h["to"] for h in hist],
                         ["opened", "triaged", "attempting", "mitigated_with_residual", "reported"])
        prev = None
        for h in hist:
            self.assertEqual(sorted(h), ["at", "command", "evidence_hashes", "from", "to"])
            self.assertEqual(h["from"], prev)
            prev = h["to"]
        self.assertTrue(any(re.match(r"^[0-9a-f]{64}$", x) for x in hist[-1]["evidence_hashes"]))

    def test_regressed_triage_replans_and_keeps_old_attempts(self):
        rig, cid = self.to_mwr()
        rig.set(sym="fail")
        self.rc(rig.run("recheck", "--case", cid), RC_NOT_FIXED)
        self.state(rig, cid, "regressed")
        self.rc(rig.run("triage", "--case", cid), RC_OK)
        c = self.state(rig, cid, "triaged")
        self.assertEqual(len(rig.attempts(cid)), 1)
        self.assertEqual(c["plan"]["levers"][0]["status"], "pending")
        r = self.rc(rig.run("report", "--case", cid), RC_REDIRECT)
        self.assertIn("L1", r.stdout)

    def test_wrong_state_exits(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1")])])
        cid = rig.open("wid broken")
        self.rc(rig.run("verify", "--case", cid), RC_NO_ATTEMPT, "verify in opened")
        self.rc(rig.run("report", "--case", cid), RC_NO_ATTEMPT, "report in opened")
        self.rc(rig.run("retriage", "--case", cid), RC_REDIRECT, "retriage in opened")
        self.rc(rig.run("attempt", "--case", cid, "--lever", "L1", "--action", "x", "--evidence", rig.evidence()),
                RC_REDIRECT, "attempt in opened")
        self.rc(rig.run("triage", "--case", cid), RC_OK)
        self.rc(rig.run("verify", "--case", cid), RC_NO_ATTEMPT, "verify in triaged")
        r = self.rc(rig.run("consent", "--case", cid, "--report", "--yes"), RC_REDIRECT, "report consent in triaged")
        self.assertIn("triaged", r.stdout)
        self.rc(rig.run("attempt", "--case", cid, "--lever", "L9", "--action", "x", "--evidence", rig.evidence()),
                RC_USAGE, "unknown lever")
        self.rc(rig.run("attempt", "--case", cid, "--lever", "L1", "--action", "x", "--evidence",
                        os.path.join(rig.d, "missing.txt")), RC_USAGE, "missing evidence")
        self.assertEqual(rig.attempts(cid), [])
        rig.set(sym="pass")
        self.attempt(rig, cid, "L1")
        self.rc(rig.run("verify", "--case", cid), RC_OK)
        self.state(rig, cid, "fixed")
        self.rc(rig.run("verify", "--case", cid), RC_NOT_FIXED, "verify in fixed")
        self.rc(rig.run("triage", "--case", cid), RC_REDIRECT, "triage in fixed")

    def test_retriage_with_symptom_gone_says_run_verify(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1")])])
        cid = self.triaged(rig, "wid broken")
        self.attempt(rig, cid, "L1")
        self.state(rig, cid, "exhausted")
        rig.set(sym="pass")
        r = self.rc(rig.run("retriage", "--case", cid), RC_NOT_FIXED)
        self.assertIn("run verify", r.stdout)
        self.state(rig, cid, "exhausted")
        self.rc(rig.run("verify", "--case", cid), RC_OK)
        self.state(rig, cid, "fixed")

    def test_consent_decline_needs_an_ask_once_lever(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1"), lever("L2")])])
        cid = self.triaged(rig, "wid broken")
        self.rc(rig.run("consent", "--case", cid, "--lever", "L1", "--no"), RC_USAGE)
        self.assertEqual(rig.case(cid)["plan"]["levers"][0]["status"], "pending")

    def test_status_prints_state_and_plan(self):
        rig = self.rig(entries=[entry("wid", SF, [lever("L1"), lever("L2", risky=True)])])
        cid = self.triaged(rig, "wid broken")
        r = self.rc(rig.run("status", "--case", cid), RC_OK)
        self.assertIn("triaged", r.stdout)
        self.assertIn(SF, r.stdout)
        self.assertRegex(r.stdout, r"L1[^\n]*pending")
        self.assertRegex(r.stdout, r"L2[^\n]*ask_once")
        r = self.rc(rig.run("status"), RC_OK)
        self.assertIn(cid, r.stdout)


# ─────────────────────────────── T-3 ───────────────────────────────

class T3Secrets(Base):
    def test_3_no_secret_in_any_case_file(self):
        tokens = {"F58_API_TOKEN": "tok-A1b2C3d4E5f6G7h8", "F58_MY_SECRET": "s3cr3t-value-xyz-123",
                  "F58_SERVICE_PASSWORD": "pw-0987654321-q", "F58_GH_AUTH": "authval-ZZZ-777-yyy",
                  "F58_SESSION_COOKIE": "cookie-abcdefgh-1234", "F58_SIGNING_KEY": "keyval-55aa66bb77"}
        plain = {"F58_PLAIN_MARKER": "plain-marker-6b1d0e9f"}
        sig = "a3f1c2e4b5d6978899aabbccddeeff0011223344"
        env = dict(tokens)
        env.update(plain)
        rig = self.rig(entries=[entry("wid", SMPD, [lever("L1")])], extra_env=env)
        home = rig.home
        tv = list(tokens.values())

        def out(word):
            return {"exit": 0 if word == PASS_TEXT else 1,
                    "out": "home=%s/.cys/pack\nauth %s\n{\"signature\": \"%s\"}\nsig=%s\n%s\n"
                           % (home, " ".join(tv), sig, sig, word)}

        rig.set(sym=out(FAIL_TEXT), **{"def": out(FAIL_TEXT), "rb": out(PASS_TEXT)})
        cid = rig.open("wid broken in %s/.cys token %s cookie %s" % (home, tv[0], tv[4]))
        self.rc(rig.run("triage", "--case", cid), RC_OK)
        ev_dir = os.path.join(home, "evidence-dir")
        os.makedirs(ev_dir)
        ev = os.path.join(ev_dir, "ev.txt")
        with open(ev, "w", encoding="utf-8") as f:
            f.write("EVIDENCE-CONTENT-7f3a %s %s\n" % (tv[1], sig))
        rig.set(sym=out(PASS_TEXT))
        self.rc(rig.run("attempt", "--case", cid, "--lever", "L1", "--action",
                        "edited %s/.cys/agents.json with %s" % (home, tv[2]), "--evidence", ev), RC_OK)
        self.rc(rig.run("verify", "--case", cid), RC_OK)
        self.state(rig, cid, "mitigated_with_residual")
        self.rc(rig.run("consent", "--case", cid, "--report", "--yes"), RC_OK)
        self.rc(rig.run("report", "--case", cid, "--hypothesis", "maybe %s under %s" % (tv[3], home),
                        "--observed", "probe line %s" % tv[5]), RC_OK)
        snap = rig.snapshot(cid)
        self.assertIn("vendor-report.txt", snap)
        self.assertIn("case.json", snap)
        self.assertIn("attempts.json", snap)
        self.assertTrue(any(k.startswith("obs") for k in snap))
        self.assertTrue(any(k.startswith("report-evidence") for k in snap))
        needles = tv + [home, sig, plain["F58_PLAIN_MARKER"], "EVIDENCE-CONTENT-7f3a"]
        hits = [(name, n) for name, data in snap.items() for n in needles if n.encode("utf-8") in data]
        self.assertEqual(hits, [], "secret or HOME literal found in case files")
        joined = b"\n".join(snap.values())
        self.assertIn(b"<redacted>", joined, "positive control: scrubbed values are marked")
        self.assertIn(b"~/.cys", joined, "positive control: HOME written as ~")


if __name__ == "__main__":
    unittest.main(verbosity=2)
