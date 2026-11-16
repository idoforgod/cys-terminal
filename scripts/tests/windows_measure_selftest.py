"""Self-test of the TW1 Windows measurement driver (release 0.15.0; impl RULINGS CW7, EQ1-EQ6).

Not named test_*.py on purpose (RULINGS EQ2 / D5): scripts/lane-parity-rehearsal.sh stage 3 refuses an unregistered
test_*.py, and this unit edits no existing file. It runs on a developer machine and as the FIRST step of the measurement
job (.github/workflows/windows-measure.yml): a driver whose own rules fail measures nothing.

What it pins (the rules of the record - plan impl/plans/TW1.md 3.7, each by a test below):
  M  the matrix: every design 8-2 gate, every CW7 item and every EQ5 row exists exactly once; the class counts; the
     validator refuses each malformed shape;
  V  a probe that raised, timed out, was skipped or never started is NOT-MEASURED, never a pass; a missing fact is never
     met; a control that did not discriminate is NOT-MEASURED; an unmet check with discriminating controls is FAIL;
     an environment limit is NOT-MEASURABLE-HERE; an assumption contradicted is a FINDING and changes no verdict;
  D  decisions before a probe runs: class NO, phase 2, probe not built, unit absent / disagreeing / unknown, wrong state;
  S  state hand-off: a probe that could not restore -> "unknown" -> the next stateful probe is not run;
  T  time limits with an injected clock (no real sleep);
  R  the record: environment first, findings counted, sub-row FAIL counts, job conclusion (red on FAIL anywhere or a
     failed set-up; never red for findings or NOT-MEASURED alone), Markdown order;
  P  privacy: the redaction of paths / account names and the environment allow-list;
  W  the workflow file: triggers, permissions, time limits, pins, retention, concurrency, no secret, cleanup always();
  K  (TW2, RULINGS LH6) a kill by a REMEMBERED pid acts only while that pid still names the process that was recorded
     (creation time + image); a tree is walked only from a verified root and only to children not older than their
     parent; every successful stop closes the pid's registration; the early copy of the record is redacted;
  SH (TW4) stage s3_repair_flow's second repair flow on ONE general shell file: its candidates, its evidence folder
     repair-flow-sh/ and its ONE finding line for a commit (rc 0, 94 then 0, two 94s, a refusal), a stop before the
     commit, no general file, no time left; the Python flow's facts, evidence list and state are the same whatever
     it does; the line and the folder are redacted (also across the 200-character cut) and reach the record.
  PO (TW5, RULINGS LH36) the job t9-libtests also runs PO1's po1_ tests in its two test steps (one more filter word
     each; the steps' shell otherwise as before); through the REAL probe and the REAL matrix rows a red po1_ test
     fails the T9 rows and turns the job red, a green one passes, and logs without any po1_ test are judged as before.

Standard library only. Run: python3 scripts/tests/windows_measure_selftest.py
"""

import copy
import json
import os
import re
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DRIVER = os.path.join(REPO, "scripts", "windows-measure")
sys.path.insert(0, DRIVER)

import tw1_record as R  # noqa: E402

MATRIX = os.path.join(DRIVER, "matrix.json")
WORKFLOW = os.path.join(REPO, ".github", "workflows", "windows-measure.yml")


def load():
    with open(MATRIX, encoding="utf-8") as f:
        return json.load(f)


def ran(facts):
    return {"status": "ran", "facts": facts}


def row_with(checks, **kw):
    r = {"id": "X1", "title": "t", "group": "g", "source": "s", "class": "YES", "probe": "p", "checks": checks}
    r.update(kw)
    return r


C = lambda cid, role, fact, op, **kw: dict({"id": cid, "role": role, "fact": fact, "op": op}, **kw)  # noqa: E731


class MatrixTests(unittest.TestCase):
    def test_real_matrix_is_valid(self):
        m = R.load_matrix(MATRIX)
        self.assertEqual(len(m["rows"]), 78)

    def test_class_counts_match_the_approved_plan(self):
        m = load()
        counts = {}
        for r in m["rows"]:
            counts[r["class"]] = counts.get(r["class"], 0) + 1
        # push 2 (RULINGS GS2, GL4, GB5): +7 YES rows (IN-PLACE-HELD, MANIFEST-HELD, PACK-PREV, APP-RESTORE,
        # T5C-JUDGES, T5C-DANGLING, T5C-COMPGEN), +2 PARTLY (T5C-CP949, T2G-COPY), R-T3D-IDENT WAIT -> YES
        # push 3 (RULINGS HB5, HD2): +1 YES (T5C-HANGUL), +1 WAIT (T9-JOURNAL-TAKE)
        # RULINGS HK2 (2) / HL1: +1 PARTLY (I-CLEAR-CHAIN)
        # push 4 (T6 in the tree from d00f4075): G15 WAIT -> PARTLY (timing now, no-false-revert with T5b);
        # I-AUTOOFF, R-T0-TS, R-T6-TICKSHELL WAIT -> YES (driven now; the first two stop at commit until T5b)
        # push 5 (RULINGS JG1): +1 PARTLY (W3-BOOTSTEP, recorded)
        self.assertEqual(counts, {"YES": 35, "PARTLY": 28, "NO": 8, "WAIT": 7})

    def test_every_required_row_exists_once(self):
        ids = [r["id"] for r, _ in R.iter_rows(load())]
        for req in R.REQUIRED_ROWS:
            self.assertEqual(ids.count(req), 1, req)
        self.assertEqual(len(ids), len(set(ids)))

    def test_eq5_rows_name_their_unit_or_primitive(self):
        m = {r["id"]: r for r, _ in R.iter_rows(load())}
        self.assertIn("T3d", m["R-T3D-IDENT"]["needs_units"])
        self.assertIn("T2g", m["EC2.product"]["needs_units"])
        self.assertIn("T2g", m["EC4.product"]["needs_units"])
        self.assertIn("T2g", m["EC5"]["needs_units"])
        self.assertIn("T5c", m["DY1.product"]["needs_units"])
        self.assertEqual(m["T7C.capture"]["class"], "NO")

    def test_rows_that_wait_for_a_fix_name_it_and_still_record(self):
        """RULINGS FS / FT / run-1 ruling (3): a row whose verdict needs a fix that is not merged is NOT-MEASURED
        "feature absent" - and its observation is still recorded (run_when_absent)."""
        m = {r["id"]: r for r, _ in R.iter_rows(load())}
        for rid, unit in (("CW7-LISTED", "T9a"), ("G01.nodelete", "T9a"), ("G01.variants-nodelete", "T9a"),
                          ("CW7-HELD.nopartial", "T9a"), ("I-LISTEN.b", "T9a"), ("CW7-ENC.conflict-text", "T2g"),
                          ("CW7-ENC.conflict-shell", "T2g"), ("DY3", "T5c")):
            self.assertIn(unit, m[rid]["needs_units"], rid)
            self.assertTrue(m[rid].get("run_when_absent"), rid)
            dec = R.decide_row(m[rid], load()["probes"], "updated", 1, {unit: {"merge_log": False}})
            self.assertEqual(dec[1], R.NM, rid)
            self.assertTrue(dec[2].startswith(R.R_FEATURE_ABSENT), rid)

    def _errors_after(self, mutate):
        m = load()
        mutate(m)
        return R.validate_matrix(m)

    def test_removing_a_required_row_is_refused(self):
        e = self._errors_after(lambda m: m["rows"].pop(next(i for i, r in enumerate(m["rows"]) if r["id"] == "G09")))
        self.assertTrue(any("required row missing: G09" in x for x in e), e)

    def test_duplicate_id_is_refused(self):
        e = self._errors_after(lambda m: m["rows"].append(copy.deepcopy(m["rows"][0])))
        self.assertTrue(any("duplicated" in x for x in e), e)

    def test_wait_without_unit_is_refused(self):
        def mut(m):
            r = next(r for r in m["rows"] if r["id"] == "T9-JOURNAL-TAKE")
            r.pop("needs_units")
        self.assertTrue(any("needs 'needs_units'" in x for x in self._errors_after(mut)))

    def test_no_without_fail_safe_is_refused(self):
        def mut(m):
            next(r for r in m["rows"] if r["id"] == "G09").pop("fail_safe")
        self.assertTrue(any("needs 'fail_safe'" in x for x in self._errors_after(mut)))

    def test_unknown_op_and_role_are_refused(self):
        def mut(m):
            r = next(r for r in m["rows"] if r["id"] == "G07")
            r["checks"][0]["op"] = "looks_fine"
            r["checks"][1]["role"] = "vibe"
        e = self._errors_after(mut)
        self.assertTrue(any("op 'looks_fine'" in x for x in e), e)
        self.assertTrue(any("role 'vibe'" in x for x in e), e)

    def test_a_probe_row_without_checks_is_refused(self):
        def mut(m):
            next(r for r in m["rows"] if r["id"] == "G07")["checks"] = []
        self.assertTrue(any("at least one check" in x for x in self._errors_after(mut)))

    def test_environment_check_needs_a_reason_and_assumption_its_source(self):
        def mut(m):
            r = next(r for r in m["rows"] if r["id"] == "G07")
            r["checks"].append({"id": "e", "role": "environment", "fact": "x", "op": "true"})
            r["checks"].append({"id": "a", "role": "assumption", "fact": "x", "op": "true"})
        e = self._errors_after(mut)
        self.assertTrue(any("needs the reason" in x for x in e), e)
        self.assertTrue(any("needs the assumption" in x for x in e), e)

    def test_top_row_count_is_pinned(self):
        e = self._errors_after(lambda m: m.__setitem__("expected_top_rows", 76))
        self.assertTrue(any("expected_top_rows" in x for x in e), e)

    def test_bad_probe_table_is_refused(self):
        def mut(m):
            m["probes"]["s0_env"]["time_limit_s"] = 0
            m["probes"]["s1_base"]["requires_state"] = "whatever"
        e = self._errors_after(mut)
        self.assertTrue(any("time_limit_s" in x for x in e), e)
        self.assertTrue(any("unknown state" in x for x in e), e)

    def test_every_probe_in_order_exists_and_every_row_probe_is_ordered(self):
        m = load()
        for p in m["order"]:
            self.assertIn(p, m["probes"])
        used = {r.get("probe") for r, _ in R.iter_rows(m) if r.get("probe")}
        self.assertLessEqual(used, set(m["order"]))

    def test_not_built_rows_say_so_and_do_not_pass(self):
        m = load()
        for r, _ in R.iter_rows(m):
            if r.get("probe") is None and r["class"] != "NO":
                run, v, reason = R.decide_row(r, m["probes"], "updated", 1, {})
                self.assertFalse(run, r["id"])
                self.assertEqual(v, R.NM, r["id"])


class CheckTests(unittest.TestCase):
    def test_missing_fact_is_missing_never_met(self):
        for op in R.OPS:
            c = {"id": "c", "role": "check", "fact": "absent_fact", "op": op, "value": 1, "other": "o"}
            st, _ = R.evaluate_check(c, {"o": 1})
            self.assertEqual(st, "missing", op)

    def test_fact_comparison_with_a_none_side_is_missing(self):
        c = {"id": "c", "role": "check", "fact": "a", "op": "eq_fact", "other": "b"}
        self.assertEqual(R.evaluate_check(c, {"a": None, "b": None})[0], "missing")
        self.assertEqual(R.evaluate_check(c, {"a": "x", "b": None})[0], "missing")
        self.assertEqual(R.evaluate_check(c, {"a": "x", "b": "x"})[0], "met")
        self.assertEqual(R.evaluate_check(c, {"a": "x", "b": "y"})[0], "unmet")

    def test_version_comparison(self):
        c = {"id": "c", "role": "identity", "fact": "a", "op": "version_gt_fact", "other": "b"}
        self.assertEqual(R.evaluate_check(c, {"a": "0.15.0", "b": "0.14.56"})[0], "met")
        self.assertEqual(R.evaluate_check(c, {"a": "0.14.56", "b": "0.14.56"})[0], "unmet")
        self.assertEqual(R.evaluate_check(c, {"a": "0.14.9", "b": "0.14.56"})[0], "unmet")
        self.assertEqual(R.evaluate_check(c, {"a": "x", "b": "0.14.56"})[0], "missing")

    def test_numbers_and_booleans_are_not_confused(self):
        c = {"id": "c", "role": "check", "fact": "a", "op": "le", "value": 2}
        self.assertEqual(R.evaluate_check(c, {"a": True})[0], "missing")
        self.assertEqual(R.evaluate_check(c, {"a": 2})[0], "met")
        t = {"id": "t", "role": "check", "fact": "a", "op": "true"}
        self.assertEqual(R.evaluate_check(t, {"a": 1})[0], "unmet")
        self.assertEqual(R.evaluate_check(t, {"a": True})[0], "met")

    def test_collection_ops(self):
        e = {"id": "c", "role": "check", "fact": "a", "op": "empty"}
        self.assertEqual(R.evaluate_check(e, {"a": []})[0], "met")
        self.assertEqual(R.evaluate_check(e, {"a": ["x"]})[0], "unmet")
        self.assertEqual(R.evaluate_check(e, {"a": None})[0], "missing")
        i = {"id": "c", "role": "check", "fact": "a", "op": "in", "value": [0, 2]}
        self.assertEqual(R.evaluate_check(i, {"a": 1})[0], "unmet")
        self.assertEqual(R.evaluate_check(i, {"a": 2})[0], "met")
        n = {"id": "c", "role": "check", "fact": "a", "op": "not_false"}
        self.assertEqual(R.evaluate_check(n, {"a": None})[0], "met")
        self.assertEqual(R.evaluate_check(n, {"a": False})[0], "unmet")


class VerdictTests(unittest.TestCase):
    ROW = row_with([
        C("id1", "identity", "marker", "eq_fact", other="ver"),
        C("cmp", "complete", "holder", "true"),
        C("ctl", "control", "ctl_park", "none"),
        C("main", "check", "live", "eq_fact", other="vendor"),
        C("asm", "assumption", "gap", "le", value=2, assumption="checklist P1"),
    ])
    GOOD = {"marker": "0.15.0", "ver": "0.15.0", "holder": True, "ctl_park": None, "live": "a", "vendor": "a", "gap": 1}

    def v(self, facts=None, outcome=None):
        return R.compute_verdict(self.ROW, outcome if outcome is not None else ran(facts))

    def test_pass_only_when_everything_is_met(self):
        self.assertEqual(self.v(self.GOOD)["verdict"], R.PASS)

    def test_probe_that_did_not_run_is_not_measured(self):
        for st in ("error", "timeout", "skipped", "not_started"):
            res = self.v(outcome={"status": st, "facts": dict(self.GOOD), "reason": "x", "limit_s": 5})
            self.assertEqual(res["verdict"], R.NM, st)
        self.assertEqual(R.compute_verdict(self.ROW, None)["verdict"], R.NM)

    def test_identity_failure_is_not_measured(self):
        f = dict(self.GOOD, marker="0.14.56")
        res = self.v(f)
        self.assertEqual(res["verdict"], R.NM)
        self.assertIn("identity", res["reason"])

    def test_incomplete_record_is_not_measured(self):
        f = dict(self.GOOD)
        del f["holder"]
        self.assertEqual(self.v(f)["verdict"], R.NM)

    def test_control_that_did_not_discriminate_is_not_measured_even_if_the_main_check_fails(self):
        f = dict(self.GOOD, ctl_park="sha", live="b")
        res = self.v(f)
        self.assertEqual(res["verdict"], R.NM)
        self.assertIn("control did not discriminate", res["reason"])

    def test_unmet_main_check_with_good_controls_is_fail(self):
        res = self.v(dict(self.GOOD, live="b"))
        self.assertEqual(res["verdict"], R.FAIL)

    def test_missing_main_observation_is_not_measured(self):
        f = dict(self.GOOD)
        del f["live"]
        self.assertEqual(self.v(f)["verdict"], R.NM)

    def test_assumption_contradicted_is_a_finding_and_keeps_the_verdict(self):
        res = self.v(dict(self.GOOD, gap=5))
        self.assertEqual(res["verdict"], R.PASS)
        self.assertEqual(len(res["findings"]), 1)
        self.assertEqual(res["findings"][0]["assumption"], "checklist P1")
        res = self.v(dict(self.GOOD, gap=5, live="b"))
        self.assertEqual(res["verdict"], R.FAIL)
        self.assertEqual(len(res["findings"]), 1)

    def test_environment_limit_is_not_measurable_here_and_unobserved_environment_is_not_measured(self):
        row = row_with([C("env", "environment", "um_finished", "true", reason="did not finish"),
                        C("x", "check", "um_exit", "present")])
        self.assertEqual(R.compute_verdict(row, ran({"um_finished": False, "um_exit": 0}))["verdict"], R.NMH)
        self.assertEqual(R.compute_verdict(row, ran({"um_exit": 0}))["verdict"], R.NM)
        self.assertEqual(R.compute_verdict(row, ran({"um_finished": True, "um_exit": 0}))["verdict"], R.PASS)

    def test_no_real_row_passes_on_an_empty_observation(self):
        m = load()
        for r, _ in R.iter_rows(m):
            if r.get("probe"):
                res = R.compute_verdict(r, ran({}))
                self.assertNotEqual(res["verdict"], R.PASS, r["id"])
                self.assertNotEqual(res["verdict"], R.FAIL, r["id"])

    def test_no_real_row_passes_when_its_probe_did_not_run(self):
        m = load()
        for r, _ in R.iter_rows(m):
            for st in ("error", "timeout", "skipped", "not_started"):
                self.assertEqual(R.compute_verdict(r, {"status": st, "facts": {"x": 1}})["verdict"], R.NM)

    def test_vendor_first_row_fails_its_control_when_the_untouched_file_is_parked(self):
        m = {r["id"]: r for r, _ in R.iter_rows(load())}
        row = m["CW7-LISTED"]
        facts = {"pre_update_L1_sha": "u1", "pre_update_L2_sha": "u2", "pre_update_L3_sha": "u3",
                 "manifest_present_before_install": True, "L1_in_snapshot_sha": "u1",
                 "L1_listed": 0, "L2_listed": 0, "L3_listed": 0, "CL_listed": 0,
                 "CL_live_sha": "v", "CL_vendor_sha": "v", "CL_park_sha": None,
                 "L1_live_sha": "v1", "L1_vendor_sha": "v1", "L1_park_sha": "u1", "L1_user_sha": "u1",
                 "L1_pending_kind": "vendor-first",
                 "L2_live_sha": "v2", "L2_vendor_sha": "v2", "L2_park_sha": "u2", "L2_user_sha": "u2",
                 "L2_pending_kind": "vendor-first", "L3_live_sha": "u3", "L3_user_sha": "u3"}
        self.assertEqual(R.compute_verdict(row, ran(facts))["verdict"], R.PASS)
        bad = dict(facts, CL_park_sha="something")
        self.assertEqual(R.compute_verdict(row, ran(bad))["verdict"], R.NM)
        not_parked = dict(facts, L1_park_sha=None)
        self.assertEqual(R.compute_verdict(row, ran(not_parked))["verdict"], R.NM)  # None side = not an observation
        user_live = dict(facts, L1_live_sha="u1")
        self.assertEqual(R.compute_verdict(row, ran(user_live))["verdict"], R.FAIL)
        lost_before = dict(facts, pre_update_L1_sha="v-other")   # the edit was not on disk when the update began
        self.assertEqual(R.compute_verdict(row, ran(lost_before))["verdict"], R.NM)
        no_base = dict(facts, manifest_present_before_install=False)
        self.assertEqual(R.compute_verdict(row, ran(no_base))["verdict"], R.NM)

    def test_update_row_refuses_a_candidate_that_is_not_newer(self):
        m = {r["id"]: r for r, _ in R.iter_rows(load())}
        row = m["CW7-UPD"]
        facts = {"cand_marker": "0.14.56", "candidate_version": "0.14.56", "cand_cys_version": "0.14.56.0",
                 "cand_cysd_version": "0.14.56.0", "candidate_version4": "0.14.56.0", "cand_hash_match_cys": True,
                 "cand_hash_match_cysd": True, "base_version": "0.14.56", "marker_before": "0.14.56",
                 "installer_final_exit": 0, "failure_file_present": False, "pack_version_after": "0.14.56",
                 "manifest_missing": [], "cli_absent_max_consecutive": 0}
        res = R.compute_verdict(row, ran(facts))
        self.assertEqual(res["verdict"], R.NM)
        self.assertIn("cand-newer", res["reason"])


class DecisionTests(unittest.TestCase):
    PROBES = {"p": {"stage": "S3", "time_limit_s": 5, "requires_state": "updated", "leaves_state": "same"}}

    def d(self, row, state="updated", phase=1, units=None):
        return R.decide_row(row, self.PROBES, state, phase, units or {})

    def test_class_no_is_not_measurable_here(self):
        self.assertEqual(self.d(row_with([], **{"class": "NO", "why": "w"}))[1], R.NMH)

    def test_phase_two_is_not_run_in_phase_one(self):
        r = row_with([C("x", "check", "a", "true")], phase=2)
        self.assertEqual(self.d(r)[:2], (False, R.NM))
        self.assertEqual(self.d(r, phase=2)[0], True)

    def test_probe_not_built(self):
        run, v, reason = self.d(row_with([], probe=None))
        self.assertEqual((run, v, reason), (False, R.NM, R.R_PROBE_NOT_BUILT))

    def test_units(self):
        r = row_with([C("x", "check", "a", "true")], needs_units=["T6"])
        absent = {"T6": {"has_marker": True, "behaviour": False, "merge_log": False}}
        present = {"T6": {"has_marker": True, "behaviour": True, "merge_log": True}}
        disagree = {"T6": {"has_marker": True, "behaviour": True, "merge_log": False}}
        unknown = {"T6": {"has_marker": True, "behaviour": None, "merge_log": True}}
        self.assertEqual(self.d(r, units=absent)[:2], (False, R.NM))
        self.assertTrue(self.d(r, units=absent)[2].startswith(R.R_FEATURE_ABSENT))
        self.assertEqual(self.d(r, units=present), (True, None, None))
        self.assertTrue(self.d(r, units=disagree)[2].startswith(R.R_UNIT_DISAGREE))
        self.assertTrue(self.d(r, units=unknown)[2].startswith(R.R_UNIT_UNKNOWN))
        self.assertTrue(self.d(r, units={})[2].startswith(R.R_UNIT_UNKNOWN))
        info = dict(r, run_when_absent=True)
        self.assertEqual(self.d(info, units=absent)[:2], (True, R.NM))

    def test_an_absent_unit_is_named_even_where_the_probe_is_not_built(self):
        r = row_with([], probe=None, needs_units=["T7b"], **{"class": "WAIT", "why": "w"})
        run, v, reason = self.d(r, units={"T7b": {"has_marker": True, "behaviour": False, "merge_log": False}})
        self.assertEqual((run, v), (False, R.NM))
        self.assertTrue(reason.startswith(R.R_FEATURE_ABSENT), reason)
        run, v, reason = self.d(r, units={"T7b": {"has_marker": True, "behaviour": True, "merge_log": True}})
        self.assertEqual(reason, R.R_PROBE_NOT_BUILT)

    def test_unit_status_table(self):
        s = R.unit_status
        self.assertEqual(s("T2g", {"T2g": {"merge_log": True}}), "present")
        self.assertEqual(s("T2g", {"T2g": {"merge_log": False}}), "absent")
        self.assertEqual(s("T2g", {"T2g": {"merge_log": None}}), "unknown")
        self.assertEqual(s("T6", {"T6": {"has_marker": True, "behaviour": False, "merge_log": None}}), "absent")

    def test_wrong_state_is_not_run(self):
        run, v, reason = self.d(row_with([C("x", "check", "a", "true")]), state="unknown")
        self.assertFalse(run)
        self.assertIn("state", reason)

    def test_run_when_absent_keeps_the_decision_verdict(self):
        r = row_with([C("x", "check", "a", "true")], needs_units=["T2g"], run_when_absent=True)
        units = {"T2g": {"merge_log": False}}
        dec = self.d(r, units=units)
        res = R.row_result(r, None, dec, ran({"a": True}))
        self.assertEqual(res["verdict"], R.NM)
        self.assertTrue(res["reason"].startswith(R.R_FEATURE_ABSENT))
        self.assertEqual(res["info_checks"][0]["state"], "met")


class StateTests(unittest.TestCase):
    def test_transitions(self):
        stateful = {"leaves_state": "updated", "read_only": False}
        ro = {"leaves_state": "same", "read_only": True}
        self.assertEqual(R.state_after(stateful, "ran", True, "base"), "updated")
        self.assertEqual(R.state_after(stateful, "ran", False, "base"), "unknown")
        self.assertEqual(R.state_after(stateful, "error", None, "base"), "unknown")
        self.assertEqual(R.state_after(stateful, "timeout", None, "base"), "unknown")
        self.assertEqual(R.state_after(ro, "timeout", None, "updated"), "updated")
        self.assertEqual(R.state_after(ro, "ran", None, "updated"), "updated")
        self.assertEqual(R.state_after(stateful, "skipped", None, "base"), "base")

    def test_unknown_state_stops_the_following_stateful_probes(self):
        m = load()
        state = R.state_after(m["probes"]["s1_base"], "timeout", None, "clean")
        self.assertEqual(state, "unknown")
        r = next(r for r in m["rows"] if r["id"] == "CW7-UPD")
        self.assertEqual(R.decide_row(r, m["probes"], state, 1, {})[:2], (False, R.NM))
        g07 = next(r for r in m["rows"] if r["id"] == "G07")
        self.assertTrue(R.decide_row(g07, m["probes"], state, 1, {})[0], "a probe needing no state still runs")


class TimeTests(unittest.TestCase):
    class Clock:
        def __init__(self):
            self.t = 100.0
            self.sleeps = 0

        def now(self):
            return self.t

        def sleep(self, s):
            self.sleeps += 1
            self.t += s

    def test_done_before_the_limit(self):
        c = self.Clock()
        res = R.wait_until(lambda: "ok" if c.t >= 105 else None, c.now, c.sleep, limit_s=30, step_s=1)
        self.assertEqual(res[0], "done")
        self.assertEqual(res[1], "ok")
        self.assertAlmostEqual(res[2], 5.0)

    def test_timeout_at_the_limit(self):
        c = self.Clock()
        res = R.wait_until(lambda: None, c.now, c.sleep, limit_s=10, step_s=3)
        self.assertEqual(res[0], "timeout")
        self.assertGreaterEqual(res[2], 10)
        self.assertLess(res[2], 13.0001)

    def test_deadline(self):
        c = self.Clock()
        d = R.Deadline(c.now, 20)
        self.assertFalse(d.expired())
        c.t += 19.5
        self.assertAlmostEqual(d.remaining(), 0.5)
        c.t += 1
        self.assertTrue(d.expired())
        self.assertEqual(d.remaining(), 0.0)

    def test_longest_false_run(self):
        self.assertEqual(R.longest_false_run([True, False, False, True, False]), 2)
        self.assertEqual(R.longest_false_run([]), 0)
        self.assertEqual(R.longest_false_run([True, None, True]), 0)


class PrivacyTests(unittest.TestCase):
    ENV = {
        "USERPROFILE": "X:\\profiles\\someone",
        "LOCALAPPDATA": "X:\\profiles\\someone\\AppData\\Local",
        "RUNNER_TEMP": "D:\\a\\_temp",
        "GITHUB_REPOSITORY": "someowner/somerepo",
        "GITHUB_REPOSITORY_OWNER": "someowner",
        "GITHUB_ACTOR": "someactor",
        "USERNAME": "someone",
        "GITHUB_SHA": "abc123",
        "GH_TOKEN": "ghs_secretvalue",
        "PATH": "C:\\x",
    }

    def test_paths_and_names_are_replaced_in_every_form(self):
        mp = R.redaction_map(self.ENV)
        obj = {
            "a": "X:\\profiles\\someone\\AppData\\Local\\cys\\cys.exe",
            "b": "x:/PROFILES/SOMEONE/.cys/pack",
            "c": ["https://example/someowner/somerepo/releases", "by someactor"],
            "d": json.dumps({"p": "X:\\profiles\\someone\\x"}),
            "k\\profiles\\someone": 1,
        }
        out = json.dumps(R.redact(obj, mp))
        for needle in ("someone", "someowner", "someactor", "somerepo"):
            self.assertNotIn(needle.lower(), out.lower(), needle)
        self.assertIn("%LOCALAPPDATA%", out)
        self.assertIn("%USERPROFILE%", out)

    def test_environment_values_only_from_the_allow_list(self):
        snap = R.env_snapshot(self.ENV)
        self.assertEqual(snap["values"], {"GITHUB_SHA": "abc123"})
        self.assertIn("GH_TOKEN", snap["names_only"])
        self.assertNotIn("ghs_secretvalue", json.dumps(snap))
        for name in ("GITHUB_REPOSITORY", "GITHUB_ACTOR", "USERPROFILE", "USERNAME", "GH_TOKEN"):
            self.assertNotIn(name, R.ENV_VALUE_ALLOW)

    # ── TW2 (RULINGS LH6 + EQ6 / HC6): the early copy of the record, made BEFORE the clean-up stops anything ──
    NEEDLES = ("someone", "someowner", "someactor", "somerepo")

    def _all_bytes_under(self, root):
        out = b""
        for dp, _dns, fns in os.walk(root):
            for fn in fns:
                with open(os.path.join(dp, fn), "rb") as fh:
                    out += fn.encode("utf-8") + b"\n" + fh.read() + b"\n"
        return out

    def test_the_early_copy_is_redacted_in_every_form(self):
        """The same forms as test_paths_and_names_are_replaced_in_every_form, written as the files a run leaves in
        the record folder (a JSON observation, a probe log, a daemon's raw output in a nested evidence folder)."""
        import tempfile
        import tw1_main as M
        mp = R.redaction_map(self.ENV)
        obj = {"a": "X:\\profiles\\someone\\AppData\\Local\\cys\\cys.exe", "b": "x:/PROFILES/SOMEONE/.cys/pack",
               "c": ["https://example/someowner/somerepo/releases", "by someactor"],
               "d": json.dumps({"p": "X:\\profiles\\someone\\x"}), "k\\profiles\\someone": 1}
        with tempfile.TemporaryDirectory() as d:
            src, dest = os.path.join(d, "tw1-record"), os.path.join(d, "tw1-record-early")
            os.makedirs(os.path.join(src, "evidence", "s2_update"))
            os.makedirs(os.path.join(src, "logs"))
            with open(os.path.join(src, "tw1-record.json"), "w", encoding="utf-8") as fh:
                json.dump(obj, fh)
            with open(os.path.join(src, "logs", "s2_update.log"), "w", encoding="utf-8", newline="") as fh:
                fh.write("run as someone in X:\\profiles\\someone\r\nSOMEONE again\r\n")
            with open(os.path.join(src, "evidence", "s2_update", "new-cysd.out"), "wb") as fh:
                fh.write("데몬 X:/profiles/someone/.cys/pack 시작\n".encode("utf-8"))
            os.makedirs(os.path.join(src, "evidence", "s3_repair_flow", "repair-flow-sh"))    # TW4's folder
            with open(os.path.join(src, "evidence", "s3_repair_flow", "repair-flow-sh", "commit.json"), "w",
                      encoding="utf-8") as fh:
                json.dump({"args": ["repair", "commit", "R-2", "--json"], "result": {"rc": 96, "out": "", "err": (
                    "X:\\profiles\\someone\\.cys\\pack\\hooks\\x.sh: 셸 문법 검사 실패 — /x/profiles/SOMEONE/x.sh "
                    "by someactor")}}, fh)
            sm = M.redacted_copy(src, dest, mp)
            self.assertEqual((sm["copied"], sm["redacted"]), (4, 4), sm)
            blob = self._all_bytes_under(dest).lower()
            for needle in self.NEEDLES:
                self.assertNotIn(needle.encode("utf-8"), blob, needle)
            self.assertIn(b"%userprofile%", blob)
            with open(os.path.join(dest, "logs", "s2_update.log"), "rb") as fh:
                self.assertIn(b"\r\n", fh.read())          # line endings kept: a copy, not a rewrite
            with open(os.path.join(src, "logs", "s2_update.log"), encoding="utf-8") as fh:
                self.assertIn("someone", fh.read())         # the source is not touched (the clean-up redacts it later)

    def test_the_early_copy_leaves_out_what_it_cannot_redact(self):
        """A file that is not UTF-8 text cannot be redacted as text: it is LEFT OUT of the early copy (named in the
        summary), never copied as it is. A file still being written may end in half a character: that tail is dropped
        and the rest is redacted. No mapping, or a destination inside the record folder: nothing is copied."""
        import tempfile
        import tw1_main as M
        mp = R.redaction_map(self.ENV)
        with tempfile.TemporaryDirectory() as d:
            src, dest = os.path.join(d, "rec"), os.path.join(d, "early")
            os.makedirs(os.path.join(src, "evidence"))
            with open(os.path.join(src, "evidence", "holder-defender.out"), "wb") as fh:
                fh.write(b"\xff\xfe s\x00someone X:\\profiles\\someone")          # UTF-16 / cp1252 text: not UTF-8
            with open(os.path.join(src, "evidence", "busy-cysd.out"), "wb") as fh:
                fh.write("pack at X:\\profiles\\someone\\.cys 완".encode("utf-8")[:-1])  # cut inside the last character
            sm = M.redacted_copy(src, dest, mp)
            self.assertEqual(sm["left_out_not_utf8"], [os.path.join("evidence", "holder-defender.out")])
            self.assertEqual(sm["tail_dropped"], [os.path.join("evidence", "busy-cysd.out")])
            self.assertFalse(os.path.exists(os.path.join(dest, "evidence", "holder-defender.out")))
            with open(os.path.join(dest, "evidence", "busy-cysd.out"), encoding="utf-8") as fh:
                self.assertEqual(fh.read(), "pack at %USERPROFILE%\\.cys ")
            self.assertNotIn(b"someone", self._all_bytes_under(dest).lower())
            for bad_mapping, bad_dest in (([], os.path.join(d, "early2")), (mp, os.path.join(src, "inside"))):
                sm = M.redacted_copy(src, bad_dest, bad_mapping)
                self.assertEqual(sm["copied"], 0)
                self.assertTrue(sm.get("refused"), sm)
                self.assertFalse(os.path.exists(os.path.join(bad_dest, "evidence")))

    def test_the_early_copy_command_uses_the_clean_ups_mapping(self):
        """`tw1_main.py early-copy` reads the mapping the run stored for the clean-up (ctx.json); it always exits 0
        and writes early-copy.json (what was copied, what was left out, what the copy lacks) into the copy."""
        import tempfile
        from unittest import mock
        import tw1_main as M
        with tempfile.TemporaryDirectory() as d:
            work, out, dest = os.path.join(d, "tw1"), os.path.join(d, "tw1-record"), os.path.join(d, "tw1-record-early")
            os.makedirs(work)
            os.makedirs(os.path.join(out, "logs"))
            with open(os.path.join(work, "ctx.json"), "w", encoding="utf-8") as fh:
                json.dump({"redaction": R.redaction_map(self.ENV)}, fh)
            with open(os.path.join(out, "logs", "s0_env.log"), "w", encoding="utf-8") as fh:
                fh.write("X:\\profiles\\someone by someactor\n")
            with mock.patch.dict(os.environ, {}, clear=True):
                rc = M.main(["early-copy", "--work", work, "--out", out, "--dest", dest])
            self.assertEqual(rc, 0)
            with open(os.path.join(dest, "early-copy.json"), encoding="utf-8") as fh:
                note = json.load(fh)
            self.assertEqual(note["copied"], 1)
            self.assertTrue(note["lacks"])
            blob = self._all_bytes_under(dest).lower()
            self.assertNotIn(b"someone", blob)
            self.assertNotIn(b"someactor", blob)
            # no stored mapping and none in the environment: nothing is copied, still exit 0
            os.remove(os.path.join(work, "ctx.json"))
            dest2 = os.path.join(d, "early-2")
            with mock.patch.dict(os.environ, {}, clear=True):
                self.assertEqual(M.main(["early-copy", "--work", work, "--out", out, "--dest", dest2]), 0)
            self.assertEqual(sorted(os.listdir(dest2)), ["early-copy.json"])


class RecordTests(unittest.TestCase):
    def results(self):
        rows = [{"id": "A", "verdict": R.PASS, "findings": []},
                {"id": "B", "verdict": R.NM, "reason": R.R_FEATURE_ABSENT + ": T6", "findings": []},
                {"id": "C", "verdict": R.NM, "reason": R.R_PROBE_NOT_BUILT, "findings": []},
                {"id": "D", "verdict": R.NMH, "reason": "no", "findings": [{"row": "D", "check": "x", "assumption": "a", "observed": "o"}]}]
        subs = [{"id": "A.1", "parent": "A", "verdict": R.PASS, "findings": []}]
        return rows, subs

    def test_shape_and_counts(self):
        rows, subs = self.results()
        rec = R.build_record({"commit": "x"}, {"windows": "w"}, {"failed": False, "reasons": []}, rows, subs)
        keys = list(rec)
        self.assertLess(keys.index("environment"), keys.index("rows"))
        s = rec["summary"]
        self.assertEqual(s["top"][R.PASS], 1)
        self.assertEqual(s["feature_absent"], 1)
        self.assertEqual(s["probe_not_built"], 1)
        self.assertEqual(s["findings"], 1)
        self.assertIn("findings 1", s["line"])
        self.assertEqual(R.job_conclusion(rec)[0], "green", "findings and NOT-MEASURED alone never turn it red")

    def test_fail_anywhere_turns_the_job_red(self):
        rows, subs = self.results()
        subs[0]["verdict"] = R.FAIL
        rec = R.build_record({}, {}, {"failed": False}, rows, subs)
        self.assertEqual(R.job_conclusion(rec)[0], "red")
        rows, subs = self.results()
        rows[0]["verdict"] = R.FAIL
        self.assertEqual(R.job_conclusion(R.build_record({}, {}, {"failed": False}, rows, subs))[0], "red")

    def test_failed_setup_turns_the_job_red(self):
        rows, subs = self.results()
        rec = R.build_record({}, {}, {"failed": True, "reasons": ["x"]}, rows, subs)
        self.assertEqual(R.job_conclusion(rec)[0], "red")
        self.assertIn("set-up FAILED", rec["summary"]["line"])

    def test_no_record_is_red(self):
        self.assertEqual(R.job_conclusion(None)[0], "red")
        self.assertEqual(R.job_conclusion({})[0], "red")

    def test_markdown_puts_the_summary_and_findings_before_the_environment(self):
        rows, subs = self.results()
        md = R.render_markdown(R.build_record({}, {"windows": "w"}, {"failed": False}, rows, subs))
        self.assertLess(md.index("PASS 1"), md.index("## Findings"))
        self.assertLess(md.index("## Findings"), md.index("## Environment"))
        self.assertLess(md.index("## Environment"), md.index("## Rows"))
        self.assertIn("**D / x**", md)

    def test_aggregate(self):
        self.assertEqual(R.aggregate({"class": "NO"}, None, [R.FAIL]), R.NMH)
        self.assertEqual(R.aggregate({"class": "YES"}, None, [R.PASS, R.PASS]), R.PASS)
        self.assertEqual(R.aggregate({"class": "YES"}, None, [R.PASS, R.NMH]), R.NMH)
        self.assertEqual(R.aggregate({"class": "YES"}, R.PASS, [R.PASS, R.NM]), R.NM)
        self.assertEqual(R.aggregate({"class": "YES"}, R.PASS, [R.NM, R.FAIL]), R.FAIL)
        self.assertEqual(R.aggregate({"class": "YES"}, None, []), R.NM)

    def test_setup_evaluation(self):
        spec = [{"probe": "s0", "checks": [C("a", "identity", "x", "true")]}]
        self.assertTrue(R.evaluate_setup(spec, {})["failed"])
        self.assertTrue(R.evaluate_setup(spec, {"s0": ran({"x": False})})["failed"])
        self.assertTrue(R.evaluate_setup(spec, {"s0": ran({})})["failed"])
        self.assertFalse(R.evaluate_setup(spec, {"s0": ran({"x": True})})["failed"])

    def test_real_setup_refuses_a_same_version_candidate(self):
        m = load()
        facts = {"base_installer_obtained": True, "base_sha_pinned_match": True, "base_sha_sums_match": True,
                 "candidate_obtained": True, "candidate_version": "0.14.56", "candidate_version_filename": "0.14.56",
                 "base_version": "0.14.56"}
        res = R.evaluate_setup(m["setup"][:1], {"s0_env": ran(facts)})
        self.assertTrue(res["failed"])
        self.assertTrue(any("cand-newer" in r for r in res["reasons"]))
        ok = dict(facts, candidate_version="0.15.0", candidate_version_filename="0.15.0")
        self.assertFalse(R.evaluate_setup(m["setup"][:1], {"s0_env": ran(ok)})["failed"])

    def test_a_run_where_nothing_ran_has_no_pass_and_is_red(self):
        m = load()
        rows, subs = [], []
        for r in m["rows"]:
            sub_v = []
            for s in r.get("subrows", []):
                dec = R.decide_row(s, m["probes"], "clean", 1, {})
                res = R.row_result(s, r["id"], dec, {"status": "not_started", "reason": "x"})
                subs.append(res)
                sub_v.append(res["verdict"])
            dec = R.decide_row(r, m["probes"], "clean", 1, {}) if (r.get("probe") or not r.get("subrows")) else (True, None, None)
            res = R.row_result(r, None, dec, {"status": "not_started", "reason": "x"}) if dec[1] is not None or r.get("probe") else \
                dict(R.row_result(r, None, (False, R.NM, "aggregate"), None), verdict=R.aggregate(r, None, sub_v))
            rows.append(res)
        setup = R.evaluate_setup(m["setup"], {})
        rec = R.build_record({}, {}, setup, rows, subs)
        self.assertEqual(rec["summary"]["top"][R.PASS], 0)
        self.assertEqual(rec["summary"]["sub"][R.PASS], 0)
        self.assertEqual(R.job_conclusion(rec)[0], "red")


class BuildRunTests(unittest.TestCase):
    def test_selects_the_newest_push_run_of_this_commit(self):
        runs = [{"id": 1, "head_sha": "a", "event": "push", "created_at": "2026-11-14T10:00:00Z", "status": "completed"},
                {"id": 2, "head_sha": "a", "event": "push", "created_at": "2026-11-14T11:00:00Z", "status": "in_progress"},
                {"id": 3, "head_sha": "a", "event": "workflow_dispatch", "created_at": "2026-11-14T12:00:00Z"},
                {"id": 4, "head_sha": "b", "event": "push", "created_at": "2026-11-14T13:00:00Z"}]
        self.assertEqual(R.select_build_run(runs, "a")["id"], 2)
        self.assertEqual(R.select_build_run(runs[2:3], "a")["id"], 3)
        self.assertIsNone(R.select_build_run(runs, "c"))
        self.assertIsNone(R.select_build_run(None, "a"))

    def test_wait_decision(self):
        self.assertEqual(R.build_wait_decision(None), "absent")
        self.assertEqual(R.build_wait_decision({"status": "queued"}), "wait")
        self.assertEqual(R.build_wait_decision({"status": "completed", "conclusion": "failure"}), "found")


class ManifestHelperTests(unittest.TestCase):
    def test_eol_normalized(self):
        self.assertEqual(R.eol_normalized(b"a\r\nb\r\n"), b"a\nb\n")
        self.assertEqual(R.eol_normalized(b"a\rb"), b"a\rb")
        self.assertIsNone(R.eol_normalized(None))

    def test_manifest_delta(self):
        d = R.manifest_delta({"a": "1", "b": "2", "c": "3"}, {"b": "2", "c": "9", "d": "4"})
        self.assertEqual(d, {"missing": ["a"], "added": ["d"], "changed": ["c"]})
        self.assertIsNone(R.manifest_delta(None, {}))
        self.assertIsNone(R.manifest_delta({}, None))

    def test_entries_before_follows_the_ntfs_order(self):
        names = ["agents.json", ".install-manifest.json", "acl.json", "bin", "CHARTER-RATIONALE.md", "bench", "_x"]
        self.assertEqual(R.entries_before(names, "agents.json"), [".install-manifest.json", "acl.json"])
        self.assertEqual(R.entries_before(names, "bin"), [".install-manifest.json", "acl.json", "agents.json", "bench"])


class DriverShapeTests(unittest.TestCase):
    def test_every_ordered_probe_has_a_function_and_back(self):
        import tw1_probes as P
        m = load()
        self.assertEqual(set(m["order"]), set(P.PROBES))

    def test_scenario_lists_are_disjoint_and_the_following_version_marks_them(self):
        sc = load()["scenario"]
        pools = [sc[k] for k in sc["tnv_mark_pools"]]
        self.assertEqual(set(sc["tnv_mark_pools"]), {"listed_changed", "listed_unchanged", "general_changed_py",
                                                      "general_changed_sh", "general_changed_md", "general_cmd"})
        self.assertTrue(all(len(v) == 2 and v[0] for v in sc["g01_variants"]))
        self.assertIn(["control", []], sc["g01_variants"])
        flat = [x for p in pools for x in p]
        self.assertEqual(len(flat), len(set(flat)))
        self.assertGreaterEqual(len(sc["listed_changed"]), 3)
        self.assertGreaterEqual(len(sc["general_changed_py"]), 3)

    def test_following_version_preparation(self):
        import tnv_prepare as T
        self.assertEqual(T.bump_patch("0.15.0"), "0.15.1")
        with self.assertRaises(ValueError):
            T.bump_patch("0.15.0-rc1")
        toml = '[package]\nname = "cys-terminal"\nversion = "0.15.0"\n[dependencies]\nx = { version = "1" }\n'
        out, old = T.set_package_version(toml, "0.15.1")
        self.assertEqual(old, "0.15.0")
        self.assertIn('version = "0.15.1"', out)
        self.assertIn('x = { version = "1" }', out)
        lock = '[[package]]\nname = "cys-terminal"\nversion = "0.15.0"\n\n[[package]]\nname = "other"\nversion = "0.15.0"\n'
        lock2, n = T.set_lock_version(lock, "cys-terminal", "0.15.0", "0.15.1")
        self.assertEqual(n, 1)
        self.assertIn('name = "other"\nversion = "0.15.0"', lock2)
        self.assertEqual(T.append_marker(b"a\r\nb", "# m"), b"a\nb\n# m\n")
        self.assertIn("cmd", load()["scenario"]["next_version_marker"])


class Push2HelperTests(unittest.TestCase):
    """The pure helpers measurement push 2 added to the probes (the Windows calls themselves run only on the runner)."""

    def test_vendor_rels_follow_build_rs(self):
        import tw1_probes as P
        rels = P.vendor_rels()
        self.assertGreaterEqual(len(rels), 250)                     # build.rs refuses a pack under 250 entries
        self.assertEqual(rels, sorted(rels))
        self.assertFalse([r for r in rels if any(c.startswith(".") or c in ("tests", "__pycache__") for c in r.split("/"))])
        self.assertIn("hooks/session-start.sh", rels)
        self.assertIn("bin/javis_event.py", rels)

    def test_deny_of_reads_the_last_json_line(self):
        import tw1_probes as P
        deny = '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"r"}}'
        self.assertEqual(P._deny_of("Active code page: 949\n" + deny + "\n"), (True, "r"))
        self.assertEqual(P._deny_of(""), (False, None))
        self.assertEqual(P._deny_of('{"hookSpecificOutput":{"additionalContext":"x"}}'), (False, None))
        self.assertEqual(P._deny_of("{not json"), (False, None))

    def test_spellings(self):
        import tw1_probes as P
        self.assertEqual(P._msys("X:\\profiles\\x\\.cys\\pack"), "/x/profiles/x/.cys/pack")
        self.assertEqual(P._fwd("D:\\a\\_temp"), "D:/a/_temp")

    def test_staging_leftovers_and_held_relations(self):
        import tempfile
        import tw1_probes as P
        with tempfile.TemporaryDirectory() as d:
            for n in ("pack", "pack.prev", "pack-captures", ".pack-staging-init-12", "x.superseded-prev"):
                os.makedirs(os.path.join(d, n))
            self.assertEqual(P.staging_leftovers(d), [".pack-staging-init-12", "x.superseded-prev"])
        self.assertIsNone(P.staging_leftovers(os.path.join(HERE, "no-such-folder")))
        sc = load()["scenario"]
        self.assertEqual({k: f(sc) for k, f in P._HELD_REL.items()},
                         {"bash": "hooks/save-state.sh", "python": "bin/javis_event.py", "early": "agents.json"})


class Run2CorrectionTests(unittest.TestCase):
    """The driver's corrections after run 2 (RULINGS HC3-HC7, HD2-HD5, HB5) - pure logic and the rows' decisions."""

    def _row(self, rid):
        return {r["id"]: r for r, _ in R.iter_rows(load())}[rid]

    def _verdict(self, rid, facts):
        return R.compute_verdict(self._row(rid), {"status": "ran", "facts": facts})

    def test_line_endings_and_lines(self):
        self.assertEqual(R.eol_kind(b"a\r\nb\r\n"), "crlf")
        self.assertEqual(R.eol_kind(b"a\nb\n"), "lf")
        self.assertEqual(R.eol_kind(b"a\r\nb\n"), "mixed")
        self.assertEqual(R.eol_kind(b"ab"), "none")
        self.assertIsNone(R.eol_kind(None))
        self.assertEqual(R.text_lines(b"a\r\nb\n"), ["a", "b", ""])

    def test_added_lines_and_presence(self):
        base = b"one\ntwo\nthree\n"
        user = b"one\r\ntwo\r\nthree\r\n# mine\r\n"           # CRLF + one appended line
        vendor = b"one\nTWO\nthree\n"
        self.assertEqual(R.added_lines(base, user), ["# mine"])
        self.assertEqual(R.added_lines(base, vendor), ["TWO"])
        self.assertEqual(R.added_lines(base, base.replace(b"\n", b"\r\n")), [])   # a line-ending-only edit adds nothing
        merged = b"one\r\nTWO\r\nthree\r\n# mine\r\n"
        self.assertTrue(R.lines_present(["# mine", "TWO"], merged))
        self.assertFalse(R.lines_present(["# mine"], vendor))
        self.assertIsNone(R.lines_present(None, merged))

    def test_conflicting_edit_changes_a_line_the_vendor_changed(self):
        base = b"a\nb\nc\n"
        vendor = b"a\nB2\nc\n"
        out, idx = R.conflicting_edit(base, vendor, "  # mine")
        self.assertEqual(idx, 1)
        self.assertEqual(out, b"a\nb  # mine\nc\n")
        out, why = R.conflicting_edit(base, b"a\nb\nc\nd\n", " x")      # the vendor only inserted a line
        self.assertIsNone(out)
        self.assertIn("no existing", why)

    def test_edited_outcome_is_structural(self):
        import tw1_probes as P
        base, user, vendor = b"x\ny\n", b"x\r\ny\r\nmine\r\n", b"x\nY\n"
        merged = b"x\r\nY\r\nmine\r\n"
        o = P.edited_outcome(merged, user, base, vendor, None)
        self.assertEqual((o["live_eol"], o["user_eol"]), ("crlf", "crlf"))
        self.assertTrue(o["user_added_in_live"] and o["vendor_added_in_live"] and o["vendor_changed"])
        self.assertIsNone(o["new_is_vendor"])
        lost = P.edited_outcome(vendor, user, base, vendor, vendor)       # the user's line lost, .new = vendor
        self.assertFalse(lost["user_added_in_live"])
        self.assertTrue(lost["new_is_vendor"])

    def test_aggregate_reason_names_the_sub_row(self):
        r = R.aggregate_reason(R.PASS, "all checks met", [("DY3.dotdot", R.FAIL), ("x", R.PASS)], R.FAIL)
        self.assertIn("DY3.dotdot", r)
        self.assertIn("all checks met", r)
        self.assertEqual(R.aggregate_reason(R.PASS, "all checks met", [("x", R.PASS)], R.PASS), "all checks met")

    def test_redact_tree_rewrites_text_only(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, "a"))
            with open(os.path.join(d, "a", "cysd.out"), "w", encoding="utf-8") as fh:
                fh.write("pack at X:\\profiles\\runner-x\\.cys\\pack by runner-x\n")
            with open(os.path.join(d, "b.bin"), "wb") as fh:
                fh.write(b"\xff\xfe runner-x")
            n = R.redact_tree(d, R.redaction_map({"USERPROFILE": "X:\\profiles\\runner-x", "USERNAME": "runner-x"}))
            self.assertEqual(n, 1)
            with open(os.path.join(d, "a", "cysd.out"), encoding="utf-8") as fh:
                txt = fh.read()
            self.assertNotIn("runner-x", txt)
            self.assertIn("%USERPROFILE%", txt)
            with open(os.path.join(d, "b.bin"), "rb") as fh:
                self.assertEqual(fh.read(), b"\xff\xfe runner-x")

    def test_kept_and_merged_rows_apply_only_to_their_kind(self):
        kept = {"CF1_listed": 1, "CF1_vendor_changed": True, "CF1_conflict_line": 3, "manifest_present_before_install": True,
                "CF1_pending_kind": "user-kept", "CF1_user_has_cr": True, "CF1_live_sha": "u", "CF1_user_sha": "u",
                "CF1_new_is_vendor": True, "CF1_pending_eol": None, "CF1_pending_reason": "merge-conflict"}
        self.assertEqual(self._verdict("CW7-ENC.conflict-text", kept)["verdict"], R.PASS)
        merged = dict(kept, CF1_pending_kind="merged")
        v = self._verdict("CW7-ENC.conflict-text", merged)
        self.assertEqual(v["verdict"], R.NM)
        self.assertIn("record incomplete", v["reason"])
        g = {"G1_listed": 1, "CG_listed": 1, "CG_live_sha": "v", "CG_vendor_sha": "v", "CG_new_sha": None,
             "manifest_present_before_install": True, "G1_vendor_changed": True, "G1_pending_kind": "merged",
             "G1_new_sha": None, "G1_capture_is_user": True, "G1_snapshot_is_user": True, "G1_user_added_in_live": True,
             "G1_vendor_added_in_live": True, "G1_live_eol": "lf", "G1_user_eol": "lf"}
        self.assertEqual(self._verdict("CW7-GENERAL", g)["verdict"], R.PASS)
        self.assertEqual(self._verdict("CW7-GENERAL", dict(g, G1_pending_kind="user-kept"))["verdict"], R.NM)
        self.assertEqual(self._verdict("CW7-GENERAL", dict(g, G1_user_added_in_live=False))["verdict"], R.FAIL)

    def test_dotdot_gap_is_a_finding_not_red_and_closing_it_is_not_red(self):
        base = {"dotdot_junction_created": True, "dotdot_bash_lands": "physical", "dotdot_python_lands": "lexical",
                "q_dotdot_native": 0, "q_dotdot_msys": 1, "dotdot_native_protects": None, "dotdot_removed": True}
        open_gap = self._verdict("DY3.dotdot", dict(base, dotdot_msys_protects=False))
        self.assertEqual(open_gap["verdict"], R.PASS)
        self.assertEqual(len(open_gap["findings"]), 1)
        self.assertIn("HD5", open_gap["findings"][0]["assumption"])
        closed = self._verdict("DY3.dotdot", dict(base, q_dotdot_msys=0, dotdot_msys_protects=True))
        self.assertEqual((closed["verdict"], closed["findings"]), (R.PASS, []))

    def test_cp949_control_failing_says_call_form(self):
        f = {"cp949_active": True, "cp949_ctl_deny": False, "cp949_ctl_exit": 2, "cp949_exit": 2, "cp949_deny": False}
        v = self._verdict("T5C-CP949.utf8", f)
        self.assertEqual(v["verdict"], R.NMH)
        self.assertIn("CALL FORM", v["reason"])

    def test_compgen_empty_is_absent_on_windows_and_new_rows(self):
        c = [x for x in self._row("T5C-COMPGEN")["checks"] if x["id"] == "child-empty"][0]
        self.assertEqual(c["value"], "None")
        dec = R.decide_row(self._row("T9-JOURNAL-TAKE"), load()["probes"], "updated", 1, {"T9": {"merge_log": False}})
        self.assertEqual(dec[1], R.NM)
        self.assertTrue(dec[2].startswith(R.R_FEATURE_ABSENT))
        v = self._verdict("T5C-HANGUL", {"hangul_pack_ready": False})
        self.assertEqual(v["verdict"], R.NMH)
        self.assertIn("Hangul", v["reason"])
        self.assertEqual(load()["unit_markers"]["T7b"], ["repair", "gate", "--lane", "pack"])


class ReleaseBlockingRowTests(unittest.TestCase):
    """The evidence rule for pack files and the rows RULINGS HK2 (2) / HL1 make blocking (WIN-13, WIN-14, WIN-02,
    I-CLEAR-CHAIN, G04 / G08)."""

    def _row(self, rid):
        return {r["id"]: r for r, _ in R.iter_rows(load())}[rid]

    def _verdict(self, rid, facts):
        return R.compute_verdict(self._row(rid), {"status": "ran", "facts": facts})

    def test_pack_file_evidence_carries_no_body(self):
        import tw1_probes as P
        base = b"a\nACCOUNT-LINE secret-name\nc\n"
        vendor = b"a\nACCOUNT-LINE acct-a\nc\n"
        user = R.conflicting_edit(base, vendor, "  # tw1 conflicting user edit")[0].replace(b"\n", b"\r\n")
        ev = P.edited_evidence("bin/x.sh", user, vendor, user, base, vendor)
        text = json.dumps(ev, ensure_ascii=False)
        self.assertNotIn("secret-name", text)                 # the original line is never copied
        self.assertNotIn("acct-a", text)                      # nor the vendor's line
        self.assertIn("# tw1 conflicting user edit", text)    # only the driver's own marker
        self.assertEqual(ev["live"]["crlf"], 3)
        self.assertEqual(ev["user_added"][0]["in_live"], True)
        self.assertEqual(set(ev["live"]), {"sha256", "size", "crlf", "lf", "cr"})
        src = open(os.path.join(HERE, "..", "windows-measure", "tw1_probes.py"), encoding="utf-8").read()
        self.assertNotIn("b64encode", src)                    # no file body in any encoding

    def test_statusline_input_has_what_the_product_reads(self):
        import tw1_probes as P
        d = P._statusline_input("X:/p/tw1/s.jsonl", "sid", 41)
        for k in ("transcript_path", "context_window", "rate_limits", "model"):
            self.assertIn(k, d)
        self.assertEqual(d["context_window"]["used_percentage"], 41)
        doc = {"departments": [{"surfaces": [{"surface_id": 7, "usage": {"source": "statusline"}}]}]}
        self.assertEqual(P._find_surface(doc, 7)["usage"]["source"], "statusline")
        self.assertIsNone(P._find_surface(doc, 8))

    def test_failure_record_parsing(self):
        import tempfile
        import tw1_probes as P

        class Fake:
            def __init__(self, d):
                self.d = d

            def exe(self, n):
                return os.path.join(self.d, n)
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "cys-install-failure.txt"), "w", encoding="utf-8") as fh:
                fh.write("cys installer: critical executable verification FAILED (exit 3)\nunrecoverable: cys.exe\n"
                         "rolled-back-to-previous:\nnot-updated:\nplacement-refused: cys.exe(vacate-locked)\n")
            ff = P._failure_file(Fake(d))
            self.assertTrue(ff["first"].startswith("cys installer: critical executable verification FAILED"))
            self.assertEqual(ff["tokens"]["placement-refused"], "cys.exe(vacate-locked)")
            self.assertEqual(ff["tokens"]["not-updated"], "")
            self.assertIsNone(P._failure_file(Fake(os.path.join(d, "none"))))

    def test_win14_covers_the_fourteen_changed_tools(self):
        sc = load()["scenario"]["cp949_tools"]
        self.assertEqual(len(sc), 14)
        names = {t["name"] for t in sc}
        g14 = {s["id"] for s in self._row("G14")["subrows"]}
        self.assertEqual(g14, {"G14." + n for n in names} | {"G14.%s.pipe" % n for n in names} | {"G14.env"})
        doc = {t["name"]: {c[0]: c[2] for c in t["cases"]} for t in sc}
        self.assertEqual(doc["javis_preflight"], {"help": 0, "hangul-arg": 2})
        self.assertEqual(doc["javis_mission"], {"help": 64, "hangul-arg": 64})
        self.assertEqual(doc["javis_dept_schedule"], {"help": 2, "hangul-path": 0})
        self.assertNotIn([], [t["cases"] for t in sc])

    def test_win14_pipe_form(self):
        """Run 3 asked for the pipe form: cp949 strict streams; the documented exit codes; an encode error is a check
        AND a finding naming the character; the daemon-spawned tools are measured without PYTHONUTF8 as well."""
        import tw1_probes as P
        for n in ("javis_teamtoken", "javis_bootstrap"):
            self.assertIn(("t14pd", "the daemon's direct spawn (no PYTHONUTF8)", False), P.pipe_launchers(n))
        self.assertEqual([x[0] for x in P.pipe_launchers("javis_report")], ["t14p"])
        sub = next(s for s in self._row("G14")["subrows"] if s["id"] == "G14.javis_teamtoken.pipe")
        ids = {c["id"] for c in sub["checks"]}
        self.assertTrue({"help-rc", "hangul-arg-no-encode-error", "daemon-help-rc", "daemon-hangul-arg-encodable"} <= ids)
        facts = {"t14p_javis_report_present": True, "t14p_javis_report_launchers": ["x"], "t14p_javis_report_non_cp949_chars": []}
        for case, rc in (("help", 0), ("hangul-arg", 2)):
            facts.update({"t14p_javis_report_%s_rc" % case: rc, "t14p_javis_report_%s_traceback" % case: False,
                          "t14p_javis_report_%s_encode_error" % case: False, "t14p_javis_report_%s_encode_error_char" % case: None})
        self.assertEqual(self._verdict("G14.javis_report.pipe", facts)["verdict"], R.PASS)
        facts["t14p_javis_report_help_encode_error"] = True
        facts["t14p_javis_report_help_encode_error_char"] = "\\u2713"
        v = self._verdict("G14.javis_report.pipe", facts)
        self.assertEqual(v["verdict"], R.FAIL)
        self.assertTrue(any("2713" in str(x["observed"]) for x in v["findings"]), v["findings"])
        self.assertIn('errors="strict"', P._CP949_PIPE_WRAPPER)

    def test_p5_rests_on_a_state_that_blocks(self):
        """Run 3: the checklist's ACL recipe did not stop the installer; the verdict needs the rename control."""
        row = next(s for s in self._row("G02")["subrows"] if s["id"] == "G02.P5")
        ctl = [c for c in row["checks"] if c["role"] == "control"]
        self.assertEqual([c["fact"] for c in ctl], ["p5_lock_cand_rename_refused"])
        ok = {"p5_lock_cand_base_ok": True, "p5_lock_cand_state_set": True, "p5_lock_cand_rename_refused": False}
        v = self._verdict("G02.P5", ok)
        self.assertEqual(v["verdict"], R.NM)
        self.assertIn("control did not discriminate", v["reason"])
        ref = next(s for s in self._row("G02")["subrows"] if s["id"] == "G02.P5ref")
        self.assertEqual(ref["kind"], "record")
        self.assertIn("p5_acl_base_exit", {c.get("fact") for c in ref["checks"]} | {c.get("other") for c in ref["checks"]})

    def test_t6_rows_and_their_gates(self):
        for rid, probe in (("R-T6-TICKSHELL", "s3_t6_jobs"), ("I-AUTOOFF", "s3_repair_flow"), ("R-T0-TS", "s3_repair_flow")):
            self.assertEqual(self._row(rid)["probe"], probe)
        self.assertEqual(self._row("I-AUTOOFF")["needs_units"], ["T6", "T5b"])
        self.assertTrue(self._row("I-AUTOOFF").get("run_when_absent"))
        subs = {s["id"]: s for s in self._row("G16")["subrows"]}
        self.assertEqual(subs["G16.product"]["needs_units"], ["T6", "T5b"])
        self.assertEqual(subs["G16.propagation"]["needs_units"], ["T6", "T7b"])
        g15 = {s["id"]: s for s in self._row("G15")["subrows"]}
        self.assertEqual(g15["G15.timing"]["needs_units"], ["T6"])
        self.assertEqual(g15["G15.no-false-revert"]["needs_units"], ["T6", "T5b"])
        m = load()
        self.assertLess(m["order"].index("s3_t6_jobs"), m["order"].index("s3_repair_flow"))
        self.assertLess(m["order"].index("s3_repair_flow"), m["order"].index("s5_checklist"))

    def test_t9_rows(self):
        subs = {s["id"]: s for s in self._row("T9-JOURNAL-TAKE")["subrows"]}
        self.assertEqual(set(subs), {"T9-JOURNAL-TAKE.prim", "T9-JOURNAL-TAKE.libtests", "T9-JOURNAL-TAKE.verb",
                                     "T9-JOURNAL-TAKE.shipped"})
        self.assertNotIn("needs_units", subs["T9-JOURNAL-TAKE.prim"])
        self.assertEqual(subs["T9-JOURNAL-TAKE.libtests"]["needs_units"], ["T9"])
        self.assertEqual(subs["T9-JOURNAL-TAKE.shipped"]["class"], "NO")
        import tw1_probes as P
        self.assertEqual(len(P.T9_EXPECTED_TESTS), 22)

    def test_push5_probe_fixes(self):
        """RULINGS JG3: the four probe faults of run 4."""
        import tw1_probes as P
        # G02.P9 judges only the installer's own <bin>.prev*.exe
        p9 = next(s for s in self._row("G02")["subrows"] if s["id"] == "G02.P9")
        prev = next(c for c in p9["checks"] if c["id"] == "prev")
        self.assertEqual((prev["fact"], prev["op"]), ("p9_prev_left", "empty"))
        with open(P.__file__, encoding="utf-8") as fh:
            src = fh.read()
        self.assertIn('f["p9_prev_left"] = _exes(pr, "*.prev*.exe")', src)
        # the seat command line keeps a spaced argument one word
        self.assertEqual(P._pane_quote("tw1 repair flow", "native"), "'tw1 repair flow'")
        self.assertEqual(P._pane_quote("it's", "bash"), "'it'\\''s'")
        self.assertEqual(P._pane_quote("--note", "bash"), "--note")
        # T9's presence by the tree: a source marker decides over the merge log
        units = {"T9": {"merge_log": False, "behaviour": True, "has_marker": True, "source_marker": True}}
        self.assertEqual(R.unit_status("T9", units), "present")
        units["T9"]["behaviour"] = None
        self.assertEqual(R.unit_status("T9", units), "unknown")
        units = {"T6": {"merge_log": False, "behaviour": True, "has_marker": True}}
        self.assertEqual(R.unit_status("T6", units), "disagree")
        self.assertEqual(load()["unit_source_markers"]["T9"][0], "src/pack.rs")
        # T9-JOURNAL-TAKE.prim: the timed series, the holders, a control; at-once is an assumption (minor #141)
        prim = next(s for s in self._row("T9-JOURNAL-TAKE")["subrows"] if s["id"] == "T9-JOURNAL-TAKE.prim")
        roles = {c["id"]: c["role"] for c in prim["checks"]}
        self.assertEqual(roles["at-once"], "assumption")
        self.assertEqual(roles["within-30"], "check")
        self.assertEqual(P.TAKE_SERIES_AT_S[0], 0.0)
        self.assertEqual(P.TAKE_SERIES_AT_S[-1], 30.0)

    def test_jg2_and_w3_rows(self):
        import tw1_probes as P
        rw = next(s for s in self._row("T5C-JUDGES")["subrows"] if s["id"] == "T5C-JUDGES.runtime-write")
        self.assertEqual(rw["kind"], "record")
        w3 = self._row("W3-BOOTSTEP")
        self.assertEqual((w3["kind"], w3["probe"]), ("record", "s7_w3_bootstep"))
        self.assertEqual([x[0] for x in P.W3_LEGS], ["d0", "da", "db", "dc"])
        self.assertNotIn("PYTHONUTF8", dict(P.W3_LEGS[0][1]))
        self.assertEqual(dict(P.W3_LEGS[1][1]), {"PYTHONUTF8": "1"})
        out = ("[PASS] C01 \u2014 ok\n[WARN] C02 \u2014 x\nTraceback (most recent call last):\n"
               "UnicodeEncodeError: 'cp949' codec can't encode character '\\u2014' in position 3\n")
        fx = P.w3_output_facts(out)
        self.assertEqual((fx["result_lines_dash"], fx["traceback"], fx["encode_error_char"], fx["summary"]),
                         (2, True, "\\u2014", False))
        fx = P.w3_output_facts("[PASS] C01 \\u2014 ok\npreflight: READY \u2014 FAIL 0\n")
        self.assertEqual((fx["result_lines_escaped"], fx["summary"]), (1, True))
        m = load()
        self.assertLess(m["order"].index("s7_w3_bootstep"), m["order"].index("s5_checklist"))

    def test_clear_chain_candidate_must_be_no_weaker(self):
        cases = {"missing": [None, False], "good1": ["T1", True], "good2": ["T2", True], "stale": ["T2", True],
                 "failing": ["T2", True]}
        f = {"cc_base_registered": True, "cc_base_surface": "surface:4", "cc_cand_registered": True,
             "cc_cand_surface": "surface:9", "cc_cand_registered_before_registrar": True,
             "cc_cand_missing_session_is": None, "cc_cand_good1_session_is": "T1", "cc_cand_good2_session_is": "T2",
             "cc_cand_good2_source": "statusline", "cc_cand_stale_session_is": "T2", "cc_cand_failing_hook_exit": 0,
             "cc_cand_failing_session_is": "T2"}
        f.update({"cc_same_as_base_" + k: True for k in cases})
        self.assertEqual(self._verdict("I-CLEAR-CHAIN.payload", f)["verdict"], R.PASS)
        weaker = dict(f, cc_cand_failing_session_is="T3", cc_same_as_base_failing=False)
        self.assertEqual(self._verdict("I-CLEAR-CHAIN.payload", weaker)["verdict"], R.FAIL)
        dec = R.decide_row(self._row("I-CLEAR-CHAIN.verifier"), load()["probes"], "updated", 1, {})
        self.assertEqual(dec[1], R.NMH)
        self.assertIn("agent CLI", dec[2])

    def test_checklist_steps_are_built_and_p8_stated(self):
        g02 = {s["id"]: s for s in self._row("G02")["subrows"]}
        for p in ("P4", "P5", "P6a", "P6b", "P7", "P9", "P10a", "P10b", "P11"):
            self.assertEqual(g02["G02." + p]["probe"], "s5_checklist", p)
        self.assertEqual(g02["G02.P8"]["class"], "NO")
        self.assertEqual(self._row("G08")["probe"], "s5_checklist")
        self.assertEqual(self._row("G04")["probe"], "s7_primitives")
        self.assertIsNone(self._row("G12")["probe"])
        self.assertIn("HQ", self._row("G12")["why"])

    def test_win13_row_and_its_folder(self):
        import tw1_probes as P
        self.assertEqual(len(P.WIN13_USER), 20)
        self.assertTrue(any("\uac00" <= ch <= "\ud7a3" for ch in P.WIN13_USER) and " " in P.WIN13_USER)
        v = self._verdict("G13", {"folder_ok": False})
        self.assertEqual(v["verdict"], R.NMH)
        self.assertEqual(self._row("G13")["probe"], "s6_hangul")


class FakeWindows:
    """TW2: an injected process table [{pid, ppid, name, path, created}] and the one OS primitive the driver's kills go
    through (`_os_terminate(pid, created)`: the handle-level check of the creation time, then TerminateProcess). New
    processes get a fresh pid and a later creation time; a pid can be handed out again (reuse) by `reuse`."""

    def __init__(self, procs=()):
        self.procs = [dict(p) for p in procs]
        self.killed = []
        self.next_pid = 5000
        self.clock = 50_000

    def processes(self):
        return [dict(p) for p in self.procs]

    def os_terminate(self, pid, created=None):
        for p in self.procs:
            if p["pid"] == pid and (created is None or p["created"] == created):
                self.procs.remove(p)
                self.killed.append(pid)
                return True
        return False

    def identity(self, pid):
        for p in self.procs:
            if p["pid"] == pid:
                return {"created": p["created"], "image": p["path"]}
        return None

    def spawn(self, path, ppid=1, pid=None):
        self.clock += 10
        if pid is None:
            self.next_pid += 4
            pid = self.next_pid
        p = {"pid": pid, "ppid": ppid, "name": re.split(r"[\\/]", path)[-1], "path": path, "created": self.clock}
        self.procs.append(p)
        return p

    def popen(self):
        world = self

        class _Popen:
            def __init__(self, cmd, **_kw):
                self.pid = world.spawn(str(cmd[0]))["pid"]
        return _Popen


INST = "X:\\inst"
CYSD = INST + "\\cysd.exe"


class KillIdentityTests(unittest.TestCase):
    """TW2 (RULINGS LH6): a kill by a remembered pid acts only on the process that was recorded; the registry is kept
    true; every kill site of the driver is in the census below."""

    def setUp(self):
        import tempfile
        from unittest import mock
        import tw1_probes as P
        import tw1_main as M
        self.P, self.M, self.mock = P, M, mock
        self.tmp = tempfile.TemporaryDirectory()
        self.work = os.path.join(self.tmp.name, "tw1")
        self.out = os.path.join(self.tmp.name, "tw1-record")
        os.makedirs(self.work)
        self.saved = (dict(P._STARTED), dict(P._REGISTRY), list(P.KILL_REFUSED))
        P._STARTED.clear()
        P._REGISTRY.update(path=None, probe=None)
        del P.KILL_REFUSED[:]

    def tearDown(self):
        P = self.P
        P._STARTED.clear()
        P._STARTED.update(self.saved[0])
        P._REGISTRY.clear()
        P._REGISTRY.update(self.saved[1])
        P.KILL_REFUSED[:] = self.saved[2]
        self.tmp.cleanup()

    def world(self, procs=()):
        w = FakeWindows(procs)
        mock = self.mock
        for name, fn in (("processes", w.processes), ("_os_terminate", w.os_terminate), ("proc_identity", w.identity)):
            pt = mock.patch.object(self.P, name, fn)
            pt.start()
            self.addCleanup(pt.stop)
        return w

    def probe(self, pid="s3_repair_flow"):
        return self.P.Probe(pid, self.work, self.out, {"paths": {"install_dir": INST, "pack_dir": INST + "\\pack",
                                                                  "local_dir": INST + "\\local"}})

    def register(self, entries):
        with open(os.path.join(self.work, "pids.jsonl"), "a", encoding="utf-8") as fh:
            for e in entries:
                fh.write(json.dumps(e) + "\n")

    def open_registrations(self):
        return self.P.registry_read(os.path.join(self.work, "pids.jsonl"))

    # ── the pure rules ──
    def test_identity_verdicts(self):
        P = self.P
        reg = {"created": 1000, "image": "X:\\Inst\\cysd.exe"}
        self.assertEqual(P.identity_verdict(reg, {"created": 1000, "image": "\\\\?\\x:\\inst\\CYSD.EXE"}), "ours")
        self.assertEqual(P.identity_verdict(reg, {"created": 1001, "image": "X:\\Inst\\cysd.exe"}), "pid-reused")
        self.assertEqual(P.identity_verdict(reg, {"created": 1000, "image": "C:\\runner\\Runner.Worker.exe"}), "pid-reused")
        self.assertEqual(P.identity_verdict(reg, None), "gone")
        self.assertEqual(P.identity_verdict({"created": None, "image": "x"}, {"created": 5, "image": "x"}), "unverifiable")
        self.assertEqual(P.identity_verdict(None, {"created": 5, "image": "x"}), "unverifiable")
        self.assertEqual(P.identity_verdict(reg, {"created": None, "image": "X:\\Inst\\cysd.exe"}), "unverifiable")
        self.assertEqual(P.identity_verdict({"created": 1000, "image": None}, {"created": 1000, "image": "y"}), "ours")
        self.assertFalse(P.same_process(reg, None))

    def test_a_tree_is_walked_only_from_a_verified_root_and_only_to_children_not_older_than_their_parent(self):
        P = self.P
        procs = [
            {"pid": 100, "ppid": 1, "name": "cysd.exe", "path": CYSD, "created": 1000},
            {"pid": 101, "ppid": 100, "name": "bash.exe", "path": "b", "created": 1010},
            {"pid": 102, "ppid": 101, "name": "claude.exe", "path": "c", "created": 1020},
            # older than 100: the child of an EARLIER holder of pid 100 (Windows keeps a dead parent's number)
            {"pid": 103, "ppid": 100, "name": "Runner.Worker.exe", "path": "C:\\runner\\Runner.Worker.exe", "created": 500},
            {"pid": 104, "ppid": 103, "name": "node.exe", "path": "n", "created": 600},
            {"pid": 105, "ppid": 1, "name": "other.exe", "path": "o", "created": 1005},
            {"pid": 106, "ppid": 102, "name": "x.exe", "path": "x", "created": None},   # unreadable: never walked
        ]
        self.assertEqual(sorted(P.tree_of(100, 1000, procs)), [101, 102])
        plan = P.plan_tree_kill(100, {"created": 1000, "image": CYSD}, procs)
        self.assertEqual(plan["root"], "ours")
        self.assertEqual([k[0] for k in plan["kill"]], [102, 101, 100])      # descendants first, the root last
        self.assertEqual(plan["kill"][0][1], 1020)                           # each kill carries its creation time
        for expect, state in (({"created": 999, "image": CYSD}, "pid-reused"), (None, "unverifiable")):
            plan = P.plan_tree_kill(100, expect, procs)
            self.assertEqual((plan["root"], plan["kill"]), (state, []))
        self.assertEqual(P.plan_tree_kill(77, {"created": 1, "image": None}, procs)["root"], "gone")
        # a root the caller still holds a handle to (Popen) cannot have lost its pid: walked even after it exited
        plan = P.plan_tree_kill(100, {"created": 1000, "image": CYSD, "pinned": True}, procs[1:])
        self.assertEqual((plan["root"], [k[0] for k in plan["kill"]]), ("pinned-exited", [102, 101]))
        # keep (a probe's timeout spares the run's registered processes) - by identity, not by number
        keep = {101: {"pid": 101, "created": 1010, "image": "b"}, 102: {"pid": 102, "created": 7, "image": "c"}}
        plan = P.plan_tree_kill(100, {"created": 1000, "image": CYSD}, procs, keep=keep)
        self.assertEqual([k[0] for k in plan["kill"]], [102, 100])
        self.assertEqual(plan["spared"], [101])

    def test_registry_entries_are_the_open_registrations(self):
        lines = [json.dumps({"pid": 10, "purpose": "a", "created": 1}), "not json",
                 json.dumps({"pid": 11, "purpose": "b", "created": 2}), json.dumps({"pid": 10, "stopped": True}),
                 json.dumps({"pid": 11, "purpose": "b again", "created": 3})]
        reg = self.P.registry_entries(lines)
        self.assertEqual(sorted(reg), [11])
        self.assertEqual(reg[11]["created"], 3)

    # ── the clean-up (K1) ──
    def test_a_registered_pid_whose_creation_time_differs_is_not_killed_and_is_reported(self):
        w = self.world([
            {"pid": 200, "ppid": 1, "name": "Runner.Worker.exe", "path": "C:\\runner\\bin\\Runner.Worker.exe", "created": 9000},
            {"pid": 201, "ppid": 200, "name": "node.exe", "path": "C:\\runner\\node.exe", "created": 9100},
        ])
        self.register([{"pid": 200, "purpose": "t6-storm daemon 0", "probe": "s3_t6_jobs", "created": 2000, "image": CYSD}])
        rep = self.M.stop_registered(self.work)
        self.assertEqual(w.killed, [])
        self.assertEqual(rep["stopped_registered"], [])
        self.assertEqual(len(rep["not_killed"]), 1)
        nk = rep["not_killed"][0]
        self.assertEqual((nk["pid"], nk["why"]), (200, "pid reused, not ours"))
        self.assertEqual(nk["registered"]["created"], 2000)
        self.assertEqual(nk["live"]["created"], 9000)
        self.assertIn("Runner.Worker.exe", nk["live"]["image"])

    def test_a_matching_registered_pid_is_killed_with_its_descendants(self):
        w = self.world([
            {"pid": 300, "ppid": 1, "name": "cysd.exe", "path": CYSD, "created": 3000},
            {"pid": 301, "ppid": 300, "name": "bash.exe", "path": INST + "\\runtime\\git\\bin\\bash.exe", "created": 3100},
            {"pid": 302, "ppid": 300, "name": "svc.exe", "path": "C:\\Windows\\svc.exe", "created": 100},
            {"pid": 400, "ppid": 1, "name": "other.exe", "path": "C:\\other.exe", "created": 4500},
        ])
        self.register([{"pid": 300, "purpose": "new daemon", "probe": "s2_update", "created": 3000, "image": CYSD},
                       {"pid": 400, "purpose": "holder", "probe": "s1_base", "created": 4000, "image": "X:\\h.exe"},
                       {"pid": 500, "purpose": "gone holder", "probe": "s1_base", "created": 5000, "image": "X:\\h.exe"},
                       {"pid": 600, "purpose": "stopped", "probe": "s1_base", "created": 6000, "image": "X:\\h.exe"},
                       {"pid": 600, "stopped": True},
                       {"pid": 700, "purpose": "registered before TW2 (no identity)", "probe": "s1_base"}])
        w.spawn("C:\\runner\\x.exe", pid=700)
        rep = self.M.stop_registered(self.work)
        self.assertEqual(sorted(w.killed), [300, 301])
        self.assertEqual(rep["stopped_registered"][0]["pid"], 300)
        self.assertEqual(sorted(rep["stopped_registered"][0]["killed"]), [300, 301])
        self.assertEqual([g["pid"] for g in rep["gone"]], [500])
        self.assertEqual({(n["pid"], n["why"]) for n in rep["not_killed"]},
                         {(400, "pid reused, not ours"),
                          (700, "identity unknown (none recorded, or the live process cannot be read) - not killed")})
        self.assertEqual({p["pid"] for p in w.procs}, {302, 400, 700})

    # ── kills inside a probe (K9-K24) ──
    def test_a_remembered_pid_in_a_probe_is_killed_only_while_it_names_the_started_process(self):
        P = self.P
        w = self.world()
        self.probe()
        with self.mock.patch.object(P.subprocess, "Popen", w.popen()):
            hp = P.start_detached([INST + "\\runtime\\python\\python.exe", "-c", "hold"])
        self.assertEqual(P._STARTED[hp]["created"], w.identity(hp)["created"])   # taken while the handle was open
        w.procs = [p for p in w.procs if p["pid"] != hp]                       # the holder ended by itself ...
        stranger = w.spawn("C:\\runner\\Runner.Listener.exe", pid=hp)             # ... and Windows gave its pid away
        w.spawn("C:\\runner\\child.exe", ppid=hp)
        self.assertEqual(P.kill_tree(hp), [])
        self.assertEqual(w.killed, [])
        self.assertEqual(P.KILL_REFUSED[-1]["pid"], hp)
        self.assertEqual(P.KILL_REFUSED[-1]["root"], "pid-reused")
        self.assertIn(stranger, w.procs)
        # the started process itself: killed with its child
        with self.mock.patch.object(P.subprocess, "Popen", w.popen()):
            hp2 = P.start_detached([INST + "\\runtime\\python\\python.exe", "-c", "hold"])
        kid = w.spawn("C:\\x\\kid.exe", ppid=hp2)["pid"]
        self.assertEqual(sorted(P.kill_tree(hp2)), sorted([hp2, kid]))
        # no identity known for this pid at all: refused, never a kill by number
        w.spawn("C:\\runner\\y.exe", pid=4242)
        self.assertEqual(P.kill_tree(4242), [])
        self.assertEqual(P.KILL_REFUSED[-1]["root"], "unverifiable")

    def test_a_pid_reused_between_the_listing_and_the_kill_is_not_killed(self):
        """The kill itself re-reads the creation time through the handle it terminates with: a process that ended
        after the snapshot, its pid given to another process before TerminateProcess, is not killed."""
        P = self.P
        w = self.world([{"pid": 60, "ppid": 1, "name": "cysd.exe", "path": CYSD, "created": 6},
                        {"pid": 61, "ppid": 60, "name": "bash.exe", "path": "b", "created": 7}])
        snapshot = w.processes()

        def listing_then_reuse():
            w.procs = [p for p in w.procs if p["pid"] != 61]
            w.spawn("C:\\runner\\Runner.Worker.exe", pid=61)              # 61 ended and was reused after the listing
            return [dict(p) for p in snapshot]
        with self.mock.patch.object(P, "processes", listing_then_reuse):
            killed = P.kill_tree(60, expect={"created": 6, "image": CYSD})
        self.assertEqual(killed, [60])
        self.assertIn(61, [p["pid"] for p in w.procs])

    def test_a_kill_across_probes_uses_the_registered_identity(self):
        P = self.P
        w = self.world([{"pid": 900, "ppid": 1, "name": "cysd.exe", "path": CYSD, "created": 9}])
        self.register([{"pid": 900, "purpose": "base daemon", "probe": "s1_base", "created": 9, "image": CYSD}])
        pr = self.probe("s2_update")
        self.assertTrue(pr.still_ours(900))
        self.assertEqual(P.kill_tree(900), [900])
        w.spawn("C:\\runner\\z.exe", pid=901)
        self.register([{"pid": 901, "purpose": "base daemon", "probe": "s1_base", "created": 1, "image": CYSD}])
        self.assertFalse(pr.still_ours(901))
        self.assertEqual(P.kill_tree(901), [])

    def test_every_successful_stop_closes_the_registration(self):
        """(2) the registry is kept true wherever the driver itself stops a process it registered - also a stop by name
        (`_kill_product`) or by the storm; a stop that failed, or of a pid that is not registered, writes nothing."""
        P = self.P
        w = self.world([{"pid": 30, "ppid": 1, "name": "cysd.exe", "path": CYSD, "created": 3},
                        {"pid": 31, "ppid": 1, "name": "python.exe", "path": INST + "\\runtime\\python\\python.exe",
                         "created": 4},
                        {"pid": 32, "ppid": 1, "name": "cys.exe", "path": INST + "\\cys.exe", "created": 5}])
        pr = self.probe("s6_variants")
        pr.register_pid(30, "v_x base daemon")
        pr.register_pid(31, "v_x holder python")
        self.assertEqual(self.open_registrations()[30]["created"], 3)
        with self.mock.patch.object(P.time, "sleep", lambda _s: None):
            killed = P._kill_product(INST)
        self.assertEqual(sorted(killed), [30, 31, 32])
        self.assertEqual(self.open_registrations(), {})
        with open(os.path.join(self.work, "pids.jsonl"), encoding="utf-8") as fh:
            self.assertEqual(sum(1 for ln in fh if '"stopped"' in ln), 2)      # 32 was never registered: no line
        self.assertFalse(P.terminate(30))                                        # already gone: nothing to close

    def test_the_restart_storm_leaves_no_stale_registered_pid(self):
        P = self.P
        w = self.world([{"pid": 40, "ppid": 1, "name": "cysd.exe", "path": CYSD, "created": 40}])
        pr = self.probe("s3_repair_flow")
        pr.register_pid(40, "new daemon")
        answer = {"code": 0, "out": "ok", "err": "", "secs": 0.0, "timed_out": False}
        with self.mock.patch.object(P.subprocess, "Popen", w.popen()), \
                self.mock.patch.object(P, "run", lambda *a, **k: dict(answer)), \
                self.mock.patch.object(P.time, "sleep", lambda _s: None):
            answers, _launch, _ready = P._restart_storm(pr, INST + "\\cys.exe", "flow-storm", n=4)
        self.assertEqual(len(answers), 4)
        daemons = [p for p in w.procs if p["name"] == "cysd.exe"]
        self.assertEqual(len(daemons), 1)
        reg = self.open_registrations()
        self.assertEqual(sorted(reg), [daemons[0]["pid"]])                     # only the one still running
        self.assertEqual(reg[daemons[0]["pid"]]["created"], daemons[0]["created"])
        self.assertEqual(len(w.killed), 4)

    # ── (3) the census of every kill site (a new one must be looked at, and this list changed with it) ──
    KILL_SITES = {
        "tw1_main.py": [
            "killed = P.kill_tree(proc.pid, keep=keep, expect=P.popen_identity(proc)) if P.IS_WIN else []",
            "res = P.kill_tree_report(pid, expect=P.registered_identity(entries[pid]))",
            "if P.terminate(pid):",
        ],
        "tw1_probes.py": [
            "plan[\"killed\"] = [pid for pid, created in plan[\"kill\"] if terminate(pid, created)]",
            "return kill_tree_report(root_pid, keep=keep, expect=expect)[\"killed\"]",
            "return bool(_k32.TerminateProcess(h, 1))",
            "if not _os_terminate(int(pid), created):",
            "return [p[\"pid\"] for p in procs if (p.get(\"name\") or \"\").lower() == name.lower() and terminate(p[\"pid\"], p.get(\"created\"))]",
            "terminate(p[\"pid\"], p.get(\"created\"))",
            "f[\"um_killed\"] = kill_tree(proc.pid, expect=popen_identity(proc))",
            "terminate(a)",
            "terminate(p, created.get(p))",
            "kill_tree(hp)",
            "kill_tree(pid)",
            "kill_tree(hp)",
            "kill_tree(x)",
            "if terminate(p[\"pid\"], p.get(\"created\")):",
            "kill_tree(hp)",
            "kill_tree(hp)",
            "kill_tree(con.pid, expect=popen_identity(con))",
            "terminate(d)",
            "kill_tree(hp)",
            "kill_tree(ep)",
            "kill_tree(hp)",
            "kill_tree(proc.pid, expect=popen_identity(proc))",
            "stop_named(\"cysd.exe\")",
        ],
    }

    def test_every_kill_site_is_in_the_census(self):
        pat = re.compile(r"\b(?:kill_tree|kill_tree_report|terminate|TerminateProcess|stop_named|_os_terminate)\(")
        for fn, want in self.KILL_SITES.items():
            with open(os.path.join(DRIVER, fn), encoding="utf-8") as fh:
                calls = [ln.strip() for ln in fh if pat.search(ln) and not re.match(r"\s*(def |#|\"|_k32\.\w+\.arg)", ln)]
            self.assertEqual(calls, want, fn)
        with open(os.path.join(DRIVER, "tw1_probes.py"), encoding="utf-8") as fh:
            src = fh.read()
        for banned in ("taskkill", "Stop-Process", "os.kill(", "wmic "):
            self.assertNotIn(banned, src)


def split_native(line):
    """The words of a native pane's command line as `_pane_quote` writes them (single quotes, '' for a quote)."""
    out, i = [], 0
    while i < len(line):
        if line[i] == " ":
            i += 1
            continue
        if line[i] == "'":
            j, buf = i + 1, ""
            while True:
                if line[j] == "'" and line[j + 1:j + 2] == "'":
                    buf, j = buf + "'", j + 2
                    continue
                if line[j] == "'":
                    break
                buf, j = buf + line[j], j + 1
            out.append(buf)
            i = j + 1
        else:
            j = line.find(" ", i)
            j = len(line) if j < 0 else j
            out.append(line[i:j])
            i = j
    return out


class FakeProduct:
    """TW4: cys.exe and its panes as the repair flows meet them. `run` is the driver's runner: the local query answers
    per file, `new-surface` makes a seat, `send` to a seat runs the typed `cys repair …` line (the words after the
    answer file of `_seat_cys`'s line) and writes the seat's answer file - as the pane's seat_cys.py does. The repair
    verbs keep a small ledger: begin takes the live bytes as the pre-image; a commit with rc 0 applies (the live bytes
    stay); revert of an applied repair writes the pre-image back. `commits` = per kind ("sh" / "py") the answers of the
    successive commits; the Python flow's default is run 7c's 94."""
    F94 = "repair.commit: %s: 검사하는 동안 수리 기록이 바뀌었습니다 — 확정을 멈춥니다(다시 `cys repair commit`)"

    def __init__(self, pack, commits=None, query=None, begin_class=None, record_rc=None, answer_missing=()):
        self.pack = pack
        self.commits = {"sh": list((commits or {}).get("sh", [])), "py": list((commits or {}).get("py", []))}
        self.query = dict(query or {})              # rel -> (exit code, {boot_critical, protection})
        self.begin_class = dict(begin_class or {})  # rel -> classification begin answers (default general)
        self.record_rc = dict(record_rc or {})      # kind -> (rc, message)
        self.answer_missing = set(answer_missing)   # verbs of the SHELL repair whose seat never writes an answer file
        self.calls, self.ledger, self.surfaces = [], {}, 0

    def _rel_of(self, path):
        return os.path.relpath(path, self.pack).replace(os.sep, "/")

    def _verb(self, args):
        verb = args[1]
        self.calls.append(tuple(args))
        if verb == "begin":
            rel = args[args.index("--files") + 1]
            rid = "R-%d" % (len(self.ledger) + 1)
            with open(os.path.join(self.pack, *rel.split("/")), "rb") as fh:
                pre = fh.read()
            self.ledger[rid] = {"rel": rel, "pre": pre, "applied": False,
                                "kind": "sh" if rel.endswith(".sh") else "py"}
            return 0, json.dumps({"repair": rid, "classification": self.begin_class.get(rel, "general"),
                                  "state": "in_progress"}) + "\n", ""
        rid = args[2]
        rec = self.ledger[rid]
        if verb == "record":
            rc, msg = self.record_rc.get(rec["kind"], (0, json.dumps({"repair": rid, "state": "in_progress"})))
            return (rc, msg + "\n", "") if rc == 0 else (rc, "", msg + "\n")
        if verb == "attest-verify":
            return 0, "%s 검증 서명(pass) — 상태 진행 중\n" % rid, ""
        if verb == "commit":
            queue = self.commits[rec["kind"]]
            rc, msg = queue.pop(0) if queue else ((94, self.F94 % rid) if rec["kind"] == "py" else (0, "{}"))
            if rc == 0:
                rec["applied"] = True
                return 0, msg + "\n", ""
            return rc, "", msg + "\n"
        if verb == "revert":
            if not rec["applied"]:
                return 95, "", "repair.revert: %s: 적용되지 않은 수리입니다\n" % rid
            with open(os.path.join(self.pack, *rec["rel"].split("/")), "wb") as fh:
                fh.write(rec["pre"])
            rec["applied"] = False
            return 0, "%s 되돌렸습니다\n" % rid, ""
        return 0, "{}\n", ""

    def run(self, cmd, timeout, env=None, cwd=None, stdin_path=None):
        args = [str(c) for c in cmd][1:]
        res = {"cmd": [str(c) for c in cmd], "code": 0, "out": "", "err": "", "secs": 0.0, "timed_out": False}
        if args[:2] == ["repair", "boot-critical"]:
            rel = self._rel_of(args[args.index("--path") + 1])
            code, doc = self.query.get(rel, (1, {"boot_critical": False, "protection": False}))
            res.update(code=code, out=json.dumps(dict(doc, rel=rel)) + "\n")
        elif args[:1] == ["new-surface"]:
            self.surfaces += 1
            res["out"] = "surface:%d\n" % self.surfaces
        elif args[:1] == ["send"]:
            words = split_native(args[3])
            outp, rargs = words[6], words[7:]
            silent = (rargs[:1] == ["repair"] and rargs[1] in self.answer_missing and rargs[1] != "begin"
                      and self.ledger.get(rargs[2], {}).get("kind") == "sh")
            if rargs[:1] == ["repair"] and not silent:
                rc, out, err = self._verb(rargs)
                with open(outp, "w", encoding="utf-8") as fh:
                    json.dump({"rc": rc, "out": out, "err": err}, fh)
            elif rargs[:1] == ["repair"]:
                self.calls.append(tuple(rargs))
        return res

    def verbs(self, kind):
        return [c[1] for c in self.calls if c[1] == "begin" and c[3].endswith(".sh") == (kind == "sh")
                or c[1] != "begin" and c[2] in self.ledger and self.ledger[c[2]]["kind"] == kind]


SH_PRE = b"#!/bin/sh\n# brief lint warn (test copy)\nexit 0\n"


class ShellRepairFlowTests(unittest.TestCase):
    """TW4: the second repair flow of stage s3_repair_flow - ONE general shell file - through the REAL driver functions
    (probe_s3_repair_flow_stage -> probe_s3_repair_flow unchanged, then _repair_flow_sh with _flow_seats, _seat_cys,
    _pane_quote, _boot_critical_exit) over an injected runner (FakeProduct). Its outcome is one finding and the folder
    repair-flow-sh/; the Python flow's facts, its evidence list and the stage's state are the same whatever it does."""

    def setUp(self):
        import tempfile
        from unittest import mock
        import tw1_probes as P
        self.P, self.mock = P, mock
        self.tmp = tempfile.TemporaryDirectory()
        root = self.tmp.name
        self.work, self.out = os.path.join(root, "tw1"), os.path.join(root, "tw1-record")
        self.inst, self.pack = os.path.join(root, "inst"), os.path.join(root, "pack")
        os.makedirs(self.work)
        self.files = {"bin/javis_report.py": b"print('report')\n", P.SH_FLOW_CANDIDATES[0]: SH_PRE,
                      P.SH_FLOW_CANDIDATES[1]: b"#!/usr/bin/env bash\necho find\n",
                      P.SH_FLOW_CANDIDATES[2]: b"#!/usr/bin/env bash\necho stop\n"}
        for rel, data in self.files.items():
            os.makedirs(os.path.dirname(self.live(rel)), exist_ok=True)
            with open(self.live(rel), "wb") as fh:
                fh.write(data)
        self.saved = (dict(P._REGISTRY),)
        self.clock = [1000.0]           # a fake clock that only sleep moves: a missing answer costs no real time

        def sleep(s):
            self.clock[0] += max(float(s), 0.001)
        for name, fn in (("sleep", sleep), ("monotonic", lambda: self.clock[0])):
            pt = mock.patch.object(P.time, name, fn)
            pt.start()
            self.addCleanup(pt.stop)

    def tearDown(self):
        self.P._REGISTRY.clear()
        self.P._REGISTRY.update(self.saved[0])
        self.tmp.cleanup()

    def live(self, rel):
        return os.path.join(self.pack, *rel.split("/"))

    def read(self, rel):
        with open(self.live(rel), "rb") as fh:
            return fh.read()

    def probe(self, mapping=()):
        return self.P.Probe("s3_repair_flow", self.work, self.out, {
            "paths": {"install_dir": self.inst, "pack_dir": self.pack, "local_dir": os.path.join(self.tmp.name, "local")},
            "selection": {"CG": "bin/javis_report.py"}, "redaction": [list(x) for x in mapping]})

    def stage(self, product, fn=None, mapping=()):
        """Run the stage (or `fn`) with the injected runner -> the probe."""
        pr = self.probe(mapping)
        with self.mock.patch.object(self.P, "run", product.run):
            (fn or self.P.PROBES["s3_repair_flow"])(pr)
        return pr

    def sh_evidence(self):
        d = os.path.join(self.out, "evidence", "s3_repair_flow", "repair-flow-sh")
        return sorted(os.listdir(d)) if os.path.isdir(d) else []

    def sh_json(self, name):
        with open(os.path.join(self.out, "evidence", "s3_repair_flow", "repair-flow-sh", name), encoding="utf-8") as fh:
            return json.load(fh)

    def finding(self, pr):
        sh = [f for f in pr.findings if f["row"] == "repair-flow-sh"]
        self.assertEqual(len(sh), 1, pr.findings)
        return sh[0]

    # ── the static choice ──
    def test_the_candidates_are_general_looking_shell_files_outside_every_scenario_pool(self):
        P = self.P
        sc = load()["scenario"]
        pools = {x for k in sc["tnv_mark_pools"] for x in sc[k]} | {sc["query_listed_rel"]}
        self.assertTrue(P.SH_FLOW_CANDIDATES)
        for rel in P.SH_FLOW_CANDIDATES:
            self.assertTrue(rel.endswith(".sh"), rel)
            self.assertNotIn(rel, pools, rel)
            with open(os.path.join(REPO, "cysjavis-pack", *rel.split("/")), "rb") as fh:   # the pack this driver ships with
                data = fh.read()
            self.assertIsNone(P.sh_candidate_refusal(rel, data, pools), rel)
        with open(os.path.join(REPO, "cysjavis-pack", *P.SH_FLOW_CANDIDATES[0].split("/")), "rb") as fh:
            self.assertEqual(P.sh_flow_shell(fh.read().split(b"\n")[0].decode()), "sh")   # the first one is sh.exe's
        self.assertEqual(self.P.PROBES["s3_repair_flow"], P.probe_s3_repair_flow_stage)

    def test_pure_rules(self):
        P = self.P
        self.assertEqual(P.sh_flow_shell("#!/bin/sh"), "sh")
        self.assertEqual(P.sh_flow_shell("#!/usr/bin/env bash"), "bash")
        self.assertEqual(P.sh_flow_shell("#!/bin/bash -e"), "bash")
        self.assertIn("another leg", P.sh_candidate_refusal("a.sh", b"#!/bin/sh\n", {"a.sh"}))
        self.assertIn("absent", P.sh_candidate_refusal("a.sh", None, set()))
        self.assertIn("#!", P.sh_candidate_refusal("a.sh", b"echo x\n", set()))
        self.assertIn("CR", P.sh_candidate_refusal("a.sh", b"#!/bin/sh\r\nx\r\n", set()))
        self.assertIsNone(P.sh_candidate_refusal("a.sh", b"#!/bin/sh\nx\n", set()))
        ok = json.dumps({"boot_critical": False, "protection": False})
        self.assertIsNone(P.sh_query_refusal(1, ok + "\n"))
        self.assertIn("exit 0", P.sh_query_refusal(0, json.dumps({"boot_critical": True, "protection": False})))
        self.assertIn("exit 2", P.sh_query_refusal(2, ""))
        self.assertIn("protection=True", P.sh_query_refusal(1, json.dumps({"boot_critical": False, "protection": True})))
        self.assertIn("protection=None", P.sh_query_refusal(1, "not json"))          # an unreadable answer is not general

        def scripted(*rcs):
            seq = list(rcs)

            def commit(n):
                if not seq:
                    return None
                rc = seq.pop(0)
                return None if rc == "budget" else {"rc": rc, "out": "", "err": "m%d" % n}
            return commit
        self.assertEqual([a["rc"] for a in P.sh_commit_attempts(scripted(0))], [0])
        self.assertEqual([a["rc"] for a in P.sh_commit_attempts(scripted(94, 0))], [94, 0])
        self.assertEqual([a["rc"] for a in P.sh_commit_attempts(scripted(94, 94, 0))], [94, 94])   # never a third
        self.assertEqual([a["rc"] for a in P.sh_commit_attempts(scripted(96, 0))], [96])           # only 94 is retried
        self.assertEqual([a["rc"] for a in P.sh_commit_attempts(scripted(94, "budget"))], [94])
        self.assertEqual(P.sh_commit_attempts(scripted("budget")), [])
        self.assertEqual(P.sh_commit_attempts(scripted(94, 0))[1]["message"], "m2")

    # ── outcomes ──
    def test_a_committed_shell_repair_is_recorded_and_reverted_by_the_product(self):
        os.makedirs(os.path.join(self.inst, "runtime", "git", "usr", "bin"))
        open(os.path.join(self.inst, "runtime", "git", "usr", "bin", "sh.exe"), "wb").close()
        fp = FakeProduct(self.pack, commits={"sh": [(0, '{"repair":"R-2","state":"probation"}')]})
        pr = self.stage(fp)
        rel = self.P.SH_FLOW_CANDIDATES[0]
        self.assertEqual(fp.verbs("sh"), ["begin", "record", "attest-verify", "commit", "revert"])
        self.assertEqual(self.sh_evidence(), ["a-new-surface-reviewer-codex.json", "a-new-surface-worker.json",
                                              "attest.json", "begin.json", "candidates.json", "commit.json",
                                              "record.json", "revert.json", "summary.json"])
        for name, verb in (("begin", "begin"), ("record", "record"), ("attest", "attest-verify"), ("commit", "commit")):
            ev = self.sh_json(name + ".json")
            self.assertEqual(sorted(ev), ["args", "result"])                         # the shape of repair-flow/*.json
            self.assertEqual(ev["args"][:2], ["repair", verb])
            self.assertEqual(sorted(ev["result"]), ["err", "out", "rc"])
        self.assertEqual(self.sh_json("begin.json")["args"], ["repair", "begin", "--files", rel, "--trigger",
                                                              "owner-order", "--json"])
        self.assertEqual(self.sh_json("attest.json")["args"][:3], ["repair", "attest-verify", "R-2"])
        fd = self.finding(pr)
        self.assertEqual(fd["check"], "commit")
        self.assertTrue(fd["observed"].startswith('repair-flow-sh: commit rc=0 {"repair":"R-2","state":"probation"}'),
                        fd["observed"])
        self.assertIn("file %s, shell sh (runtime\\git\\usr\\bin\\sh.exe, present: True)" % rel, fd["observed"])
        self.assertIn("revert rc=0", fd["observed"])
        self.assertIn("pack file back to its pre-image: yes (the product's revert)", fd["observed"])
        self.assertEqual(self.read(rel), SH_PRE)
        s = self.sh_json("summary.json")
        self.assertTrue(s["committed"] and s["live_after_commit_is_edited"] and s["reverted_by_product"])
        self.assertEqual(s["edited_sha"], self.P.sha256_bytes(SH_PRE + self.P.SH_FLOW_EDIT))   # one comment line, LF
        self.assertTrue(pr.restored)
        self.assertTrue(all(e.startswith("evidence/s3_repair_flow/repair-flow-sh/") for e in fd["evidence"]))
        self.assertFalse([e for e in pr.evidence if "repair-flow-sh" in e])           # not on any row's evidence list

    def test_a_94_then_a_0_is_recorded_as_two_attempts(self):
        fp = FakeProduct(self.pack, commits={"sh": [(94, FakeProduct.F94 % "R-2"), (0, '{"state":"probation"}')]})
        pr = self.stage(fp)
        self.assertEqual(fp.verbs("sh"), ["begin", "record", "attest-verify", "commit", "commit", "revert"])
        self.assertEqual((self.sh_json("commit.json")["result"]["rc"], self.sh_json("commit-2.json")["result"]["rc"]),
                         (94, 0))
        obs = self.finding(pr)["observed"]
        self.assertTrue(obs.startswith("repair-flow-sh: commit rc=94 repair.commit: R-2: 검사하는 동안 수리 기록이 바뀌었습니다"),
                        obs)
        self.assertIn(" | retried: commit rc=0 ", obs)
        self.assertEqual([a["rc"] for a in self.sh_json("summary.json")["commit_attempts"]], [94, 0])

    def test_two_94s_stop_after_the_second_attempt_and_the_driver_puts_the_file_back(self):
        fp = FakeProduct(self.pack, commits={"sh": [(94, FakeProduct.F94 % "R-2")] * 3})
        pr = self.stage(fp)
        self.assertEqual(fp.verbs("sh"), ["begin", "record", "attest-verify", "commit", "commit"])   # no third, no revert
        obs = self.finding(pr)["observed"]
        self.assertEqual(obs.count("commit rc=94"), 2)
        self.assertIn("pack file back to its pre-image: yes (the driver)", obs)
        self.assertEqual(self.read(self.P.SH_FLOW_CANDIDATES[0]), SH_PRE)
        self.assertTrue(pr.restored)

    def test_a_refused_commit_names_the_rc_and_the_products_sentence(self):
        msg = ("repair.commit: R-2: 살아 있는 파일의 검사가 실패해 확정을 거부합니다 — hooks/brief-lint-warn.sh: 셸 문법 검사 "
               "실패(종료 Some(2)) — sh: line 3: syntax error near unexpected token")
        fp = FakeProduct(self.pack, commits={"sh": [(96, msg)]})
        pr = self.stage(fp)
        self.assertEqual(fp.verbs("sh"), ["begin", "record", "attest-verify", "commit"])
        fd = self.finding(pr)
        self.assertEqual(fd["check"], "commit")
        self.assertTrue(fd["observed"].startswith("repair-flow-sh: commit rc=96 " + msg[:200]), fd["observed"])
        self.assertEqual(self.read(self.P.SH_FLOW_CANDIDATES[0]), SH_PRE)
        self.assertEqual(self.sh_json("commit.json")["result"]["rc"], 96)

    def test_a_flow_stopped_before_the_commit_names_the_step(self):
        fp = FakeProduct(self.pack, record_rc={"sh": (95, "repair.record: R-2: 기록할 수 없습니다")})
        pr = self.stage(fp)
        self.assertEqual(fp.verbs("sh"), ["begin", "record"])
        fd = self.finding(pr)
        self.assertEqual(fd["check"], "stopped")
        self.assertTrue(fd["observed"].startswith("repair-flow-sh: stopped at record rc=95 repair.record: R-2: 기록할 수 없습니다"),
                        fd["observed"])
        self.assertEqual(self.read(self.P.SH_FLOW_CANDIDATES[0]), SH_PRE)

    def test_a_commit_without_an_answer_is_reverted_by_the_product_first(self):
        fp = FakeProduct(self.pack, answer_missing={"commit"})
        pr = self.stage(fp)
        obs = self.finding(pr)["observed"]
        self.assertTrue(obs.startswith("repair-flow-sh: commit rc=None "), obs)
        self.assertIn("revert", fp.verbs("sh"))
        self.assertEqual(self.read(self.P.SH_FLOW_CANDIDATES[0]), SH_PRE)

    def test_no_general_shell_file_is_a_finding_and_touches_nothing(self):
        c = self.P.SH_FLOW_CANDIDATES
        os.remove(self.live(c[2]))
        fp = FakeProduct(self.pack, query={c[0]: (0, {"boot_critical": True, "protection": False}),
                                           c[1]: (1, {"boot_critical": False, "protection": True})})
        pr = self.stage(fp)
        self.assertEqual(fp.verbs("sh"), [])
        self.assertEqual(fp.surfaces, 1 + 1)                     # the Python flow's two seats only
        fd = self.finding(pr)
        self.assertEqual(fd["check"], "no-general-shell-file")
        self.assertTrue(fd["observed"].startswith("repair-flow-sh: no general shell file found ("), fd["observed"])
        for rel, why in ((c[0], "exit 0"), (c[1], "protection=True"), (c[2], "absent")):
            self.assertIn("%s: " % rel, fd["observed"])
            self.assertIn(why, fd["observed"])
        self.assertEqual(self.sh_evidence(), ["candidates.json", "summary.json"])
        self.assertEqual((self.read(c[0]), self.read(c[1])), (self.files[c[0]], self.files[c[1]]))

    def test_the_first_general_candidate_is_taken_and_its_shell_named(self):
        c = self.P.SH_FLOW_CANDIDATES
        fp = FakeProduct(self.pack, query={c[0]: (0, {"boot_critical": True, "protection": False})})
        pr = self.stage(fp)
        self.assertEqual([x[3] for x in fp.calls if x[1] == "begin"][-1], c[1])
        self.assertIn("file %s, shell bash (runtime\\git\\usr\\bin\\bash.exe, present: False)" % c[1],
                      self.finding(pr)["observed"])

    def test_begin_answering_another_class_stops_before_any_edit(self):
        c0 = self.P.SH_FLOW_CANDIDATES[0]
        fp = FakeProduct(self.pack, begin_class={c0: "boot_critical"})
        writes = []
        real = self.P.write_bytes
        with self.mock.patch.object(self.P, "write_bytes", lambda p, b: (writes.append(p), real(p, b))):
            pr = self.stage(fp)
        self.assertEqual(fp.verbs("sh"), ["begin"])
        self.assertNotIn(self.live(c0), writes)
        obs = self.finding(pr)["observed"]
        self.assertIn("stopped at begin rc=0 begin classified the file 'boot_critical', not general", obs)

    def test_it_does_not_start_without_time_left_in_the_stage(self):
        fp = FakeProduct(self.pack)
        pr = self.probe()
        with self.mock.patch.object(self.P, "run", fp.run):
            self.P._repair_flow_sh(pr, self.P.time.monotonic() - 1500)
        self.assertEqual(fp.calls, [])
        fd = self.finding(pr)
        self.assertEqual(fd["check"], "not-run")
        self.assertIn("time limit", fd["observed"])

    # ── the Python flow is the same whatever the shell flow does ──
    def outcomes(self):
        c = self.P.SH_FLOW_CANDIDATES
        return {
            "rc0": dict(commits={"sh": [(0, "{}")]}),
            "94-then-0": dict(commits={"sh": [(94, "x"), (0, "{}")]}),
            "94-twice": dict(commits={"sh": [(94, "x"), (94, "x")]}),
            "refused-96": dict(commits={"sh": [(96, "refused")]}),
            "stopped-at-record": dict(record_rc={"sh": (95, "no")}),
            "no-general-file": dict(query={r: (0, {"boot_critical": True, "protection": False}) for r in c}),
            "no-commit-answer": dict(answer_missing={"commit"}),
        }

    def fresh_run_folders(self, name):
        self.work, self.out = os.path.join(self.tmp.name, name, "tw1"), os.path.join(self.tmp.name, name, "tw1-record")
        os.makedirs(self.work)
        for rel, data in self.files.items():
            with open(self.live(rel), "wb") as fh:
                fh.write(data)

    def test_the_python_flows_facts_evidence_and_state_are_unchanged_by_every_outcome(self):
        P = self.P
        self.fresh_run_folders("python-flow-alone")
        base = self.stage(FakeProduct(self.pack), fn=P.probe_s3_repair_flow)          # the Python flow alone
        self.assertEqual(base.facts.get("flow_commit_rc"), 94)                     # run 7c's answer
        self.assertEqual(base.facts.get("flow_target"), "bin/javis_report.py")
        want = (copy.deepcopy(base.facts), list(base.evidence), base.restored)
        cases = dict(self.outcomes())
        cases["the-shell-flow-raises"] = dict()
        for name, kw in sorted(cases.items()):
            with self.subTest(outcome=name):
                self.fresh_run_folders(name)
                if name == "the-shell-flow-raises":
                    real = P._flow_seats

                    def seats(pr, tag, folder="repair-flow"):
                        if folder != "repair-flow":
                            raise RuntimeError("boom")
                        return real(pr, tag)
                    with self.mock.patch.object(P, "_flow_seats", seats):
                        pr = self.stage(FakeProduct(self.pack))
                    self.assertEqual(self.finding(pr)["check"], "driver-error")
                    self.assertIn("RuntimeError: boom", self.finding(pr)["observed"])
                else:
                    pr = self.stage(FakeProduct(self.pack, **kw))
                    self.finding(pr)
                self.assertEqual(pr.facts, want[0])
                self.assertEqual(pr.evidence, want[1])
                self.assertEqual(pr.restored, want[2])
                for rel, data in self.files.items():
                    self.assertEqual(self.read(rel), data, rel)

    def test_a_shell_file_left_changed_makes_the_state_unknown(self):
        """The one way the shell flow can move the stage's state: its file could not be put back (a later stage would
        measure another pack - plan 3.7 rule 8). Never the other way."""
        P = self.P
        real = P.write_bytes
        rel = P.SH_FLOW_CANDIDATES[0]

        def no_write_back(p, b):
            if p == self.live(rel) and b == SH_PRE:
                raise PermissionError("held")
            return real(p, b)
        with self.mock.patch.object(P, "write_bytes", no_write_back):
            pr = self.stage(FakeProduct(self.pack, commits={"sh": [(96, "refused")]}))
        self.assertIs(pr.restored, False)
        self.assertIn("pack file back to its pre-image: NO", self.finding(pr)["observed"])
        self.assertEqual(self.sh_json("summary.json")["write_back_errors"], ["held"] * 3)

    def test_a_raising_python_flow_keeps_its_error_and_the_shell_flow_does_not_run(self):
        P = self.P
        fp = FakeProduct(self.pack)
        pr = self.probe()
        with self.mock.patch.object(P, "run", fp.run), \
                self.mock.patch.object(P, "probe_s3_repair_flow", side_effect=RuntimeError("python flow")):
            with self.assertRaises(RuntimeError):
                P.probe_s3_repair_flow_stage(pr)
        self.assertEqual(fp.calls, [])
        self.assertEqual(self.finding(pr)["check"], "not-run")

    def test_the_stage_never_raises_for_the_shell_flow(self):
        """Defence in depth: whatever escapes the shell flow becomes its finding; the stage keeps the Python flow's
        observation (status 'ran')."""
        P = self.P
        self.fresh_run_folders("python-flow-alone")
        base = self.stage(FakeProduct(self.pack), fn=P.probe_s3_repair_flow)
        self.fresh_run_folders("shell-flow-escapes")
        with self.mock.patch.object(P, "_repair_flow_sh", side_effect=OSError("escaped")):
            pr = self.stage(FakeProduct(self.pack))
        self.assertEqual((pr.facts, pr.evidence, pr.restored), (base.facts, base.evidence, base.restored))
        fd = self.finding(pr)
        self.assertEqual(fd["check"], "driver-error")
        self.assertIn("OSError: escaped", fd["observed"])

    # ── the record and the observation ──
    def test_the_finding_reaches_the_observation_and_the_record(self):
        import tw1_main as M
        from types import SimpleNamespace
        P = self.P
        with open(os.path.join(self.work, "ctx.json"), "w", encoding="utf-8") as fh:
            json.dump({"paths": {"install_dir": self.inst, "pack_dir": self.pack}}, fh)
        fd = {"row": "repair-flow-sh", "check": "commit", "assumption": "a", "observed": "repair-flow-sh: commit rc=0 ok"}

        def ok_probe(pr):
            pr.facts["x"] = 1
            pr.findings.append(dict(fd))

        def raising_probe(pr):
            pr.findings.append(dict(fd, check="not-run"))
            raise RuntimeError("x")
        for pid, fn, status in (("s3_repair_flow", ok_probe, "ran"), ("s3_repair_flow", raising_probe, "error")):
            with self.mock.patch.dict(P.PROBES, {pid: fn}):
                M.cmd_probe(SimpleNamespace(id=pid, work=self.work, out=self.out))
            with open(os.path.join(self.work, "obs", pid + ".json"), encoding="utf-8") as fh:
                obs = json.load(fh)
            self.assertEqual(obs["status"], status)
            self.assertEqual(len(obs["findings"]), 1)
        with self.mock.patch.dict(P.PROBES, {"s3_eg2": lambda pr: None}):            # no finding: no key, as before
            M.cmd_probe(SimpleNamespace(id="s3_eg2", work=self.work, out=self.out))
        with open(os.path.join(self.work, "obs", "s3_eg2.json"), encoding="utf-8") as fh:
            self.assertNotIn("findings", json.load(fh))
        outcomes = {"s3_eg2": {"status": "ran", "facts": {}}, "s3_repair_flow": {"status": "ran", "findings": [fd]},
                    "s3_t6_jobs": {"status": "timeout", "facts": {}}}
        extra = R.probe_findings(["s3_eg2", "s3_t6_jobs", "s3_repair_flow"], outcomes)
        self.assertEqual(extra, [dict(fd, probe="s3_repair_flow")])
        rows = [{"id": "D", "verdict": R.NMH, "findings": [{"row": "D", "check": "x", "assumption": "a", "observed": "o"}]},
                {"id": "A", "verdict": R.PASS, "findings": []}]
        rec = R.build_record({}, {}, {"failed": False, "reasons": []}, rows, [], extra)
        self.assertEqual([f["row"] for f in rec["findings"]], ["D", "repair-flow-sh"])   # after the rows' findings
        self.assertEqual(rec["summary"]["findings"], 2)
        self.assertEqual(R.job_conclusion(rec), ("green", []))                    # a finding never turns it red
        self.assertIn("- **repair-flow-sh / commit** - assumption: a - observed: repair-flow-sh: commit rc=0 ok",
                      R.render_markdown(rec))
        same = R.build_record({}, {}, {"failed": False, "reasons": []}, rows, [])
        self.assertEqual([r["verdict"] for r in rec["rows"]], [r["verdict"] for r in same["rows"]])

    # ── privacy (RULINGS EQ6 / HC6): the new folder and the line, in every form of the existing test ──
    def test_the_shell_flow_evidence_and_finding_are_redacted_in_every_form(self):
        import tw1_main as M
        mp = R.redaction_map(PrivacyTests.ENV)
        forms = ("X:\\profiles\\someone\\.cys\\pack\\hooks\\brief-lint-warn.sh", "x:/PROFILES/SOMEONE/.cys/pack",
                 json.dumps("X:\\profiles\\someone\\x"), "/x/profiles/someone/.cys", "https://example/someowner/somerepo",
                 "by someactor")
        # the account name across character 200 (the finding's cut) and across character 400 (the summary's cut)
        msg = "e" * 195 + "someone" + "g" * 193 + "someone" + " — " + " ".join(forms)
        fp = FakeProduct(self.pack, commits={"sh": [(94, msg), (96, msg)]},
                         record_rc={"py": (0, json.dumps({"note": " ".join(forms)}))})
        pr = self.stage(fp, mapping=mp)
        d = os.path.join(self.out, "evidence", "s3_repair_flow", "repair-flow-sh")
        blob = PrivacyTests._all_bytes_under(None, d).lower()
        for needle in PrivacyTests.NEEDLES + ("someo",):       # "someo": a needle cut before it was redacted
            self.assertNotIn(needle.encode("utf-8"), blob, needle)
        self.assertIn(b"%username%", blob)
        fd = self.finding(pr)
        line = json.dumps(fd, ensure_ascii=False).lower()
        for needle in PrivacyTests.NEEDLES + ("someo",):       # "someo": a needle cut before it was redacted
            self.assertNotIn(needle, line, needle)
        self.assertIn("e" * 195 + "%user", line)                # redacted first, then cut at 200 characters
        rec = R.redact(R.build_record({}, {}, {"failed": False, "reasons": []}, [], [],
                                      R.probe_findings(["s3_repair_flow"], {"s3_repair_flow": {"findings": [fd]}})), mp)
        text = (json.dumps(rec, ensure_ascii=False) + R.render_markdown(rec)).lower()
        for needle in PrivacyTests.NEEDLES:
            self.assertNotIn(needle, text, needle)
        early = os.path.join(self.tmp.name, "tw1-record-early")
        M.redacted_copy(self.out, early, mp)
        self.assertNotIn(b"someone", PrivacyTests._all_bytes_under(None, early).lower())
        self.assertTrue(os.path.isfile(os.path.join(early, "evidence", "s3_repair_flow", "repair-flow-sh", "commit-2.json")))


_PO1_LIB_OK = ("test pack::tests::po1_install_core_child ... ignored, child of the PO1 library specimen\n",
               "test pack::tests::po1_install_core_lines_follow_the_caller ... ok\n")
_PO1_CYS = ("po1_a_the_snapshot_line_goes_to_stderr_and_the_record_stays_line_0",
            "po1_b_the_pending_summary_and_the_healed_line_go_to_stderr",
            "po1_c_the_m11_kept_user_file_lines_go_to_stderr", "po1_c_the_tombstone_lines_go_to_stderr",
            "po1_d_the_unreadable_file_line_goes_to_stderr", "po1_init_pack_keeps_the_snapshot_and_pending_lines_on_stdout")


def _cargo_log(lines, failures="", rc=0, failed=None):
    """A `cargo test` log as the step writes it (compile lines, libtest's lines, its closing line, the exit line)."""
    st = [ln.rsplit(" ... ", 1)[-1].split(",")[0].strip() for ln in lines if ln.startswith("test ")]
    n_ok, n_ign = st.count("ok"), st.count("ignored")
    n_failed = st.count("FAILED") if failed is None else failed
    return ("   Compiling cys-terminal v0.15.1 (D:\\a\\x\\x)\n    Finished `test` profile [unoptimized + debuginfo]\n"
            "     Running unittests (target\\debug\\deps\\x.exe)\n\nrunning %d tests\n%s%s\ntest result: %s. %d passed; "
            "%d failed; %d ignored; 0 measured; 600 filtered out; finished in 9.10s\n\nTW1-CARGO-EXIT=%d\n"
            % (len(lines), "".join(lines), failures, "FAILED" if n_failed else "ok", n_ok, n_failed, n_ign, rc))


class PO1OnTheT9StepsTests(unittest.TestCase):
    """TW5 (RULINGS LH36): PO1's po1_ tests ride on the two test steps of the job t9-libtests. Their result reaches the
    conclusion the way a pack_update_ test's does: the step's exit line and libtest's `test ... FAILED` lines, read by
    the REAL probe s7_t9_libtests (the artifact download injected), judged by the REAL rows T9-JOURNAL-TAKE.libtests /
    .verb. They are not in the expected lists: a tree without PO1 is judged as before."""

    ROWS = ("T9-JOURNAL-TAKE.libtests", "T9-JOURNAL-TAKE.verb")

    def setUp(self):
        import tempfile
        import tw1_probes as P
        self.P = P
        self.tmp = tempfile.TemporaryDirectory()
        self.work, self.out = os.path.join(self.tmp.name, "tw1"), os.path.join(self.tmp.name, "tw1-record")
        os.makedirs(self.work)
        self.saved = dict(P._REGISTRY)

    def tearDown(self):
        self.P._REGISTRY.clear()
        self.P._REGISTRY.update(self.saved)
        self.tmp.cleanup()

    def t9_lines(self, lib):
        if lib:
            return ["test pack::tests::%s ... ok\n" % t for t in self.P.T9_EXPECTED_TESTS]
        return ["test tests::%s ... ok\n" % t for t in self.P.T9_VERB_EXPECTED]

    def judge(self, lib_log, verb_log):
        """-> (facts, {row id: verdict result}, job conclusion) through the real probe and the real rows."""
        from unittest import mock
        P = self.P

        def fetch(run_id, name, dest, token, work):
            self.assertEqual(name, "tw1-t9-libtests")
            os.makedirs(dest, exist_ok=True)
            for fn, text in (("t9-present.txt", "present\n"), ("t9-libtests.log", lib_log), ("t9-verbtests.log", verb_log)):
                with open(os.path.join(dest, fn), "w", encoding="utf-8", newline="\n") as fh:
                    fh.write(text)
            return True, "fixture"
        pr = P.Probe("s7_t9_libtests", self.work, self.out, {})
        with mock.patch.object(P, "fetch_artifact", fetch):
            P.PROBES["s7_t9_libtests"](pr)
        rows = {r["id"]: r for r, _ in R.iter_rows(load())}
        res = {rid: R.compute_verdict(rows[rid], ran(pr.facts)) for rid in self.ROWS}
        subs = [dict(v, id=rid, parent="T9-JOURNAL-TAKE") for rid, v in res.items()]
        top = [{"id": "T9-JOURNAL-TAKE", "verdict": R.worst([v["verdict"] for v in res.values()]), "findings": []}]
        rec = R.build_record({}, {}, {"failed": False, "reasons": []}, top, subs)
        return pr.facts, res, R.job_conclusion(rec)[0]

    def cys_lines(self, failed=()):
        out = ["test tests::po1_command_child ... ignored, child of the PO1 real-process specimens\n"]
        out += ["test tests::%s ... %s\n" % (t, "FAILED" if t in failed else "ok") for t in _PO1_CYS]
        out += ["test b360_tests::po1_census_no_bare_stdout_in_the_pack_update_reach ... ok\n",
                "test b360_tests::po1_census_catches_each_mutation ... ok\n"]
        return out

    def test_the_probe_records_the_filters_the_steps_run(self):
        facts, _, _ = self.judge(_cargo_log(self.t9_lines(True)), _cargo_log(self.t9_lines(False)))
        self.assertEqual(facts["t9lib_filters"], ["t9_", "m15_da_", "m15_da2_", "po1_"])
        self.assertEqual(facts["t9verb_filters"], ["pack_update_", "b428_u9_", "po1_"])

    def test_green_po1_tests_pass_the_rows(self):
        facts, res, concl = self.judge(_cargo_log(self.t9_lines(True) + list(_PO1_LIB_OK)),
                                       _cargo_log(self.t9_lines(False) + self.cys_lines()))
        self.assertEqual({k: v["verdict"] for k, v in res.items()}, dict.fromkeys(self.ROWS, R.PASS), res)
        self.assertEqual(concl, "green")
        self.assertEqual((facts["t9lib_passed"], facts["t9lib_ignored"]), (len(self.P.T9_EXPECTED_TESTS) + 1, 1))
        self.assertEqual((facts["t9verb_passed"], facts["t9verb_ignored"]), (len(self.P.T9_VERB_EXPECTED) + 8, 1))

    def test_logs_without_any_po1_test_are_judged_as_before(self):
        """A tree without PO1: the word po1_ selects nothing, nothing is missing, the rows pass as before."""
        facts, res, concl = self.judge(_cargo_log(self.t9_lines(True)), _cargo_log(self.t9_lines(False)))
        self.assertEqual({k: v["verdict"] for k, v in res.items()}, dict.fromkeys(self.ROWS, R.PASS), res)
        self.assertEqual(concl, "green")
        self.assertEqual((facts["t9lib_expected_missing"], facts["t9verb_expected_missing"]), ([], []))

    def test_a_red_library_po1_test_fails_the_libtests_row_and_the_job(self):
        lib = self.t9_lines(True) + [_PO1_LIB_OK[0], "test pack::tests::po1_install_core_lines_follow_the_caller ... FAILED\n"]
        fail = ("\nfailures:\n\n---- pack::tests::po1_install_core_lines_follow_the_caller stdout ----\n\nthread "
                "'pack::tests::po1_install_core_lines_follow_the_caller' panicked at src\\pack.rs:19700:9:\nall-tx: "
                "\"[init-pack] x\" is not on stderr\n\n\nfailures:\n    pack::tests::po1_install_core_lines_follow_the_caller\n")
        facts, res, concl = self.judge(_cargo_log(lib, fail, rc=101),
                                       _cargo_log(self.t9_lines(False) + self.cys_lines()))
        self.assertEqual(res["T9-JOURNAL-TAKE.libtests"]["verdict"], R.FAIL, res)
        self.assertIn("po1_install_core_lines_follow_the_caller", res["T9-JOURNAL-TAKE.libtests"]["reason"])
        self.assertIn("exit0", res["T9-JOURNAL-TAKE.libtests"]["reason"])
        self.assertEqual(res["T9-JOURNAL-TAKE.verb"]["verdict"], R.PASS)
        self.assertEqual(concl, "red")

    def test_a_red_cys_po1_test_fails_the_verb_row_and_the_job(self):
        bad = "po1_a_the_snapshot_line_goes_to_stderr_and_the_record_stays_line_0"
        facts, res, concl = self.judge(_cargo_log(self.t9_lines(True) + list(_PO1_LIB_OK)),
                                       _cargo_log(self.t9_lines(False) + self.cys_lines(failed=(bad,)), rc=101))
        self.assertEqual(res["T9-JOURNAL-TAKE.verb"]["verdict"], R.FAIL, res)
        self.assertEqual(facts["t9verb_failed_names"], [bad])
        self.assertEqual(res["T9-JOURNAL-TAKE.libtests"]["verdict"], R.PASS)
        self.assertEqual(concl, "red")

    def test_a_split_form_failure_still_fails_by_the_exit_line(self):
        """On the runner a panic message can sit between `test X ... ` and `FAILED` (TW3, WIN_LOG_STEP9): the FAILED
        name is then not read from that line, but the step's exit line (101) still fails the row."""
        lines = self.t9_lines(False) + self.cys_lines()[:1] + [
            "test tests::po1_d_the_unreadable_file_line_goes_to_stderr ... \nthread 'x' panicked at src\\bin\\cys.rs:1:1:\n"
            "FAILED\n"] + self.cys_lines()[2:]
        facts, res, concl = self.judge(_cargo_log(self.t9_lines(True) + list(_PO1_LIB_OK)), _cargo_log(lines, rc=101, failed=1))
        self.assertEqual(res["T9-JOURNAL-TAKE.verb"]["verdict"], R.FAIL, res)
        self.assertIn("exit0", res["T9-JOURNAL-TAKE.verb"]["reason"])
        self.assertEqual(concl, "red")


class WorkflowTests(unittest.TestCase):
    ALLOWED_USES = {"actions/checkout@v4", "actions/setup-python@v5", "actions/upload-artifact@v4",
                    "dtolnay/rust-toolchain@stable", "swatinem/rust-cache@v2"}

    @classmethod
    def setUpClass(cls):
        with open(WORKFLOW, encoding="utf-8") as f:
            cls.text = f.read().replace("\r\n", "\n")  # .gitattributes checks out LF; a CRLF checkout must not void the step
        cls.jobs = cls._jobs(cls.text)

    @staticmethod
    def _jobs(text):
        """{job: body text} from the top-level `jobs:` block (two-space job keys)."""
        lines = text.split("\n")
        start = lines.index("jobs:")
        jobs, cur = {}, None
        for ln in lines[start + 1:]:
            m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", ln)
            if m:
                cur = m.group(1)
                jobs[cur] = []
            elif re.match(r"^\S", ln):
                break
            elif cur:
                jobs[cur].append(ln)
        return {k: "\n".join(v) for k, v in jobs.items()}

    def test_triggers(self):
        self.assertIn("'fix/tw1-measure-**'", self.text)
        self.assertIn("'fix/*-candidate'", self.text)
        self.assertRegex(self.text, r"(?m)^  workflow_dispatch:\s*$")
        self.assertRegex(self.text, r"(?m)^      build_run_id:\s*$")
        self.assertNotRegex(self.text, r"(?m)^\s+tags:")
        self.assertNotIn("pull_request_target", self.text)
        self.assertNotIn("workflow_run", self.text)

    def test_permissions_are_read_only(self):
        block = re.search(r"(?m)^permissions:\n((?:  .*\n)+)", self.text).group(1)
        self.assertEqual(sorted(x.strip() for x in block.strip().split("\n")), ["actions: read", "contents: read"])
        self.assertNotRegex(self.text, r":\s*write\b")

    def test_no_secret_and_no_input_in_a_run_body(self):
        self.assertNotIn("secrets.", self.text)
        bodies = [m.group(1) for m in re.finditer(r"(?ms)^\s+run: \|\n(.*?)(?=^\s+- |\Z)", self.text)]
        bodies += re.findall(r"(?m)^\s+run: (?!\|)(.*)$", self.text)
        self.assertTrue(bodies)
        for b in bodies:
            self.assertNotIn("${{ inputs.", b)
            self.assertNotIn("${{ github.event", b)
            self.assertNotIn("${{ needs.", b)

    def test_every_job_has_a_time_limit(self):
        self.assertEqual(set(self.jobs), {"wait-build", "tnv-build", "t9-libtests", "measure"})
        for name, body in self.jobs.items():
            self.assertRegex(body, r"(?m)^    timeout-minutes: \d+", name)

    def test_actions_are_pinned_the_existing_way(self):
        uses = set(re.findall(r"uses:\s*(\S+)", self.text))
        self.assertTrue(uses, "no action found")
        self.assertLessEqual(uses, self.ALLOWED_USES)

    def test_only_the_following_version_job_writes_a_cache(self):
        for name, body in self.jobs.items():
            if name != "tnv-build":
                self.assertNotIn("rust-cache", body, name)
                self.assertNotIn("actions/cache", body, name)
        self.assertIn("swatinem/rust-cache@v2", self.jobs["tnv-build"])

    def test_artifacts_kept_fourteen_days(self):
        uploads = re.findall(r"uses: actions/upload-artifact@v4\n((?:\s+.*\n){1,8})", self.text)
        self.assertTrue(uploads)
        for u in uploads:
            self.assertIn("retention-days: 14", u)

    def test_one_run_per_branch(self):
        self.assertRegex(self.text, r"(?m)^concurrency:\n  group: .*\$\{\{ github\.ref \}\}.*\n  cancel-in-progress: true")

    def test_measure_job_shape(self):
        body = self.jobs["measure"]
        steps = re.findall(r"(?m)^      - (?:name|uses): (.*)$", body)
        self.assertIn("self-test", " ".join(steps[:3]).lower(), steps[:3])
        self.assertIn("scripts/tests/windows_measure_selftest.py", body)
        self.assertRegex(body, r"(?ms)- name: Stop everything this run started.*?if: always\(\)")
        self.assertRegex(body, r"(?ms)- name: Upload the record.*?if: always\(\)")
        self.assertRegex(body, r"(?ms)- name: Conclusion.*?if: always\(\)")
        self.assertLess(body.index("Stop everything this run started"), body.index("Upload the record"))
        # TW2 (RULINGS LH6): a redacted copy of the record is made and uploaded right after Measure, BEFORE the clean-up
        self.assertRegex(body, r"(?ms)- name: Copy the record, redacted, before anything is stopped.*?if: always\(\)"
                               r".*?tw1_main\.py early-copy --work .*? --out .*? --dest \"\$env:RUNNER_TEMP\\tw1-record-early\"")
        self.assertRegex(body, r"(?ms)- name: Upload the early copy of the record.*?if: always\(\).*?name: tw1-record-early\n"
                               r"\s+path: \$\{\{ runner\.temp \}\}/tw1-record-early\n")
        self.assertLess(body.index("- name: Measure"), body.index("Copy the record, redacted"))
        self.assertLess(body.index("Copy the record, redacted"), body.index("Upload the early copy of the record"))
        self.assertLess(body.index("Upload the early copy of the record"), body.index("Stop everything this run started"))
        self.assertEqual(re.findall(r"(?m)^\s+path: \$\{\{ runner\.temp \}\}/(\S+)$", body), ["tw1-record-early", "tw1-record"])
        self.assertLess(body.index("Upload the record"), body.index("- name: Conclusion"))
        self.assertIn("needs: [wait-build, tnv-build, t9-libtests]", body)
        self.assertIn("TW1_T9_RESULT: ${{ needs.t9-libtests.result }}", body)

    def test_t9_job_shape(self):
        """RULINGS HV4: T9's protocol tests and the verb's complete command run on a Windows runner, only when T9 is in
        the tree; the logs are uploaded always under the name the probe reads."""
        import tw1_probes as P
        body = self.jobs["t9-libtests"]
        self.assertIn("runs-on: windows-latest", body)
        self.assertIn("cargo test --lib -- %s >" % " ".join(P.T9_LIB_TEST_FILTERS), body)
        self.assertIn("cargo test --bin cys -- %s >" % " ".join(P.T9_VERB_FILTERS), body)
        self.assertEqual(body.count("if: steps.detect.outputs.present == 'true'"), 2)
        self.assertRegex(body, r"(?ms)- name: Upload the T9 test logs.*?if: always\(\).*?name: tw1-t9-libtests")
        with open(os.path.join(HERE, "..", "windows-measure", "tw1_probes.py"), encoding="utf-8") as fh:
            self.assertIn('"tw1-t9-libtests"', fh.read())
        for t in P.T9_EXPECTED_TESTS:
            self.assertTrue(any(t.startswith(x) for x in P.T9_LIB_TEST_FILTERS), t)
        for t in P.T9_UNIX_ONLY:
            self.assertFalse(any(x in t for x in P.T9_LIB_TEST_FILTERS), t)

    def test_the_t9_steps_also_run_the_po1_tests_and_nothing_else_moved(self):
        """TW5 (RULINGS LH36): each test step is exactly as before plus the filter word po1_ - same condition, same
        shell, same rc capture into the exit line, same log; no continue-on-error and no `|| true` in the job."""
        import tw1_probes as P
        self.assertEqual(P.PO1_FILTER, "po1_")
        self.assertEqual(P.T9_LIB_TEST_FILTERS, ("t9_", "m15_da_", "m15_da2_", "po1_"))
        self.assertEqual(P.T9_VERB_FILTERS, ("pack_update_", "b428_u9_", "po1_"))
        body = self.jobs["t9-libtests"]
        for name, cmd, log in (
                ("T9 protocol tests (library)", "cargo test --lib -- t9_ m15_da_ m15_da2_ po1_", "t9-libtests.log"),
                ("Pack-update verb, complete command, test-signed pack (binary tests)",
                 "cargo test --bin cys -- pack_update_ b428_u9_ po1_", "t9-verbtests.log")):
            step = re.search(r"(?ms)^      - name: %s\n(.*?)(?=^      - |\Z)" % re.escape(name), body)
            self.assertIsNotNone(step, name)
            self.assertEqual(step.group(1), "        if: steps.detect.outputs.present == 'true'\n"
                                            "        shell: bash\n"
                                            "        run: |\n"
                                            "          rc=0\n"
                                            "          %s > t9-out/%s 2>&1 || rc=$?\n"
                                            "          echo \"TW1-CARGO-EXIT=$rc\" >> t9-out/%s\n" % (cmd, log, log))
        self.assertNotIn("continue-on-error", body)
        self.assertNotIn("|| true", body)
        for t in P.T9_EXPECTED_TESTS + P.T9_VERB_EXPECTED:      # PO1's tests are never required: a tree without them
            self.assertNotIn(P.PO1_FILTER, t)                   # is judged as before

    def test_cargo_log_summary(self):
        import tw1_probes as P
        log = ("running 3 tests\ntest pack::tests::t9_a ... ok\ntest pack::tests::t9_b ... FAILED\n"
               "test pack::tests::m15_da_c ... ignored\n\ntest result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; "
               "230 filtered out; finished in 1.00s\n\nTW1-CARGO-EXIT=101\n")
        sm = P.cargo_log_summary(log)
        self.assertEqual((sm["exit"], sm["passed"], sm["failed"], sm["ignored"]), (101, 1, 1, 1))
        self.assertEqual(sm["ran"], {"t9_a": "ok", "t9_b": "FAILED", "m15_da_c": "ignored"})
        self.assertTrue(sm["compiled"])
        sm = P.cargo_log_summary("error[E0425]: cannot find value\nerror: could not compile `cys-terminal`\nTW1-CARGO-EXIT=101\n")
        self.assertFalse(sm["compiled"])
        self.assertIsNone(sm["passed"])

    def test_no_literal_account_or_release_url(self):
        self.assertNotRegex(self.text, r"github\.com/[A-Za-z0-9_.-]+/")
        self.assertNotIn("releases/download", self.text)
        self.assertNotRegex(self.text, r"[A-Za-z]:\\Users\\")

    def test_measured_tools_run_without_utf8_mode_in_the_environment(self):
        self.assertNotRegex(self.text, r"(?m)^\s+PYTHONUTF8:", "PYTHONUTF8 in the job environment would leak into the measured product")

    def test_the_repositorys_workflow_census_accepts_this_file(self):
        """scripts/tests/test_c8_ci_consumer.py reads EVERY workflow (an action allow-list, cargo routes). This unit
        edits no existing file, so its workflow must pass that census as it is."""
        sys.path.insert(0, HERE)
        try:
            import test_c8_ci_consumer as c8
        except Exception as e:  # noqa: BLE001
            self.skipTest("the census module could not be imported here: %s" % e)
        from pathlib import Path
        problems = [p for p in c8.unread_cargo_routes(Path(REPO)) if "windows-measure.yml" in p]
        self.assertEqual(problems, [])

    def test_optional_yaml_load(self):
        try:
            import yaml  # noqa: F401
        except ImportError:
            self.skipTest("PyYAML not installed (the CI Python has none); the text-level checks above stand")
        doc = yaml.safe_load(self.text)
        self.assertIn("jobs", doc)


if __name__ == "__main__":
    unittest.main(verbosity=2)
