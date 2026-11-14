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
  W  the workflow file: triggers, permissions, time limits, pins, retention, concurrency, no secret, cleanup always().

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
        self.assertEqual(len(m["rows"]), 77)

    def test_class_counts_match_the_approved_plan(self):
        m = load()
        counts = {}
        for r in m["rows"]:
            counts[r["class"]] = counts.get(r["class"], 0) + 1
        # push 2 (RULINGS GS2, GL4, GB5): +7 YES rows (IN-PLACE-HELD, MANIFEST-HELD, PACK-PREV, APP-RESTORE,
        # T5C-JUDGES, T5C-DANGLING, T5C-COMPGEN), +2 PARTLY (T5C-CP949, T2G-COPY), R-T3D-IDENT WAIT -> YES
        # push 3 (RULINGS HB5, HD2): +1 YES (T5C-HANGUL), +1 WAIT (T9-JOURNAL-TAKE)
        # RULINGS HK2 (2) / HL1: +1 PARTLY (I-CLEAR-CHAIN)
        self.assertEqual(counts, {"YES": 32, "PARTLY": 26, "NO": 8, "WAIT": 11})

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
            r = next(r for r in m["rows"] if r["id"] == "G15")
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
        self.assertEqual(g14, {"G14." + n for n in names})
        doc = {t["name"]: {c[0]: c[2] for c in t["cases"]} for t in sc}
        self.assertEqual(doc["javis_preflight"], {"help": 0, "hangul-arg": 2})
        self.assertEqual(doc["javis_mission"], {"help": 64, "hangul-arg": 64})
        self.assertEqual(doc["javis_dept_schedule"], {"help": 2, "hangul-path": 0})
        self.assertNotIn([], [t["cases"] for t in sc])

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
        self.assertEqual(set(self.jobs), {"wait-build", "tnv-build", "measure"})
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
        self.assertLess(body.index("Upload the record"), body.index("- name: Conclusion"))
        self.assertIn("needs: [wait-build, tnv-build]", body)

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
