#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""A-ID S8: characterize the CSO outcome matrix at 2deb095c, without a daemon.

Frozen design R3 §3.3.1, lines 259–282. Exercise the real main(), including
JSON/exit aggregation, through a scripted subprocess boundary. No cys binary,
socket, queue, process group or seat is touched. Snapshot files and CYS_PACK_DIR
live only in a TemporaryDirectory. Production javis_reap_exited.py is unchanged.

Run: CYS_PACK_DIR="$(mktemp -d)" python3 -I bin/tests/test_reap_exited_matrix.py
"""
import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)
import javis_reap_exited as R  # noqa: E402

REF = "surface:42"
SURFACE = {"surface_ref": REF, "exited": True, "role": "worker", "title": "done"}
STATUS = ("cys", "status", "--json")
SCREEN = ("cys", "read-screen", "--surface", REF)
REAP = ("cys", "reap-surface", REF)
CLEAR = ("cys", "queue", "clear", REF)
LEGACY = ("cys", "close-surface", REF, "--reap")
SUCCESS = (0, "", "")
OLD_CLI = (2, "", "error: unrecognized subcommand 'reap-surface'")
OLD_DAEMON = (1, "", "error: method_not_found: unknown method: surface.reap")
GENERIC_ERROR = (1, "", "error: operation failed")
TRANSPORT_ERROR = (1, "", "error: connect failed")
NOT_FOUND = (1, "", "error: not_found: surface 42 not found")
INVALID_PARAMS = (1, "", "error: invalid_params: invalid surface")
# This list is independent of the implementation's DENY_REASONS constant.
DENIALS = ("caller_unresolved", "caller_role_forbidden", "active_surface",
           "agent_still_alive", "queue_not_empty", "daemon_ancestor",
           "grace_not_elapsed", "state_changed")


def status_reply(surfaces):
    return 0, json.dumps({"surfaces": surfaces}), ""


def denial(reason):
    return 7, "", "error: reap_denied: surface.reap denied: " + reason


class ScriptedRunner:
    """Deterministic (rc, stdout, stderr) replies; never launches a process.

    An extra command gets an inert success response so an erroneous fallback
    can reach its observable action/ok assertion. The complete call list is
    always checked separately, including calls after the scripted replies.
    """

    def __init__(self, replies):
        self.replies = {tuple(cmd): list(rows) for cmd, rows in replies.items()}
        self.calls = []
        self.options = []

    def __call__(self, cmd, **kwargs):
        key = tuple(cmd)
        self.calls.append(key)
        self.options.append(kwargs)
        rows = self.replies.get(key, [])
        result = rows.pop(0) if rows else SUCCESS
        if isinstance(result, Exception):
            raise result
        rc, stdout, stderr = result
        return SimpleNamespace(returncode=rc, stdout=stdout, stderr=stderr)


class TestReapExitedMatrix(unittest.TestCase):
    def invoke(self, replies, argv=()):
        runner = ScriptedRunner(replies)
        with tempfile.TemporaryDirectory(prefix="reap-matrix-") as td:
            env = {"HOME": td, "CYS_PACK_DIR": os.path.join(td, "pack")}
            stdout, stderr = io.StringIO(), io.StringIO()
            with mock.patch.dict(os.environ, env, clear=True), \
                    mock.patch.object(sys, "argv", [R.__file__] + list(argv)), \
                    mock.patch.object(R.subprocess, "run", side_effect=runner), \
                    contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                with self.assertRaises(SystemExit) as exited:
                    R.main()
            document = json.loads(stdout.getvalue())
            self.assertEqual(stderr.getvalue(), "")
            for row in document.get("results", []):
                if row["snapshot"] is not None:
                    path = row["snapshot"]
                    self.assertEqual(os.path.commonpath([td, path]), td)
                    with open(path, encoding="utf-8") as snapshot:
                        self.assertEqual(snapshot.read(), "final screen text")
            for options in runner.options:
                self.assertEqual(options, {"capture_output": True, "text": True,
                                           "timeout": 20})
        return document, exited.exception.code, runner.calls

    def assert_case(self, reply, action, ok, tail=(), retries=(),
                    legacy=SUCCESS, clear=SUCCESS, requery=None,
                    screen=(0, "final screen text", ""), detail=None):
        if requery is None:
            requery = status_reply([SURFACE])
        replies = {STATUS: [status_reply([SURFACE]), requery], SCREEN: [screen],
                   REAP: [reply] + list(retries), CLEAR: [clear], LEGACY: [legacy]}
        document, rc, calls = self.invoke(replies)
        self.assertEqual(len(document["results"]), 1)
        row = document["results"][0]
        # These outcome assertions discriminate the S8 fallback/retry mutants.
        self.assertEqual((row["action"], row["ok"]), (action, ok),
                         "script disposition must preserve the observed result")
        self.assertEqual((document["status"], rc),
                         ("ok", 0) if ok else ("partial-failure", 1))
        self.assertEqual(document["exited_found"], 1)
        self.assertEqual((row["surface_ref"], row["role"], row["title"]),
                         (REF, "worker", "done"))
        self.assertEqual(calls, [STATUS, SCREEN, REAP] + list(tail),
                         "exact command sequence: no extra fallback, clear, or retry")
        if detail is not None:
            self.assertEqual(row["detail"], detail)
        return row

    def test_status_unavailable_executes_nothing(self):
        for reply in ((1, "", "daemon down"), (0, "not json", ""),
                      (0, "{}", ""), (0, '{"surfaces": {}}', ""),
                      (0, "[]", "")):
            with self.subTest(reply=reply):
                document, rc, calls = self.invoke({STATUS: [reply]})
                self.assertEqual(document, {"status": "status-unavailable", "reaped": [],
                                           "note": "데몬 상태 조회 불가 — 미집행(fail-open)"})
                self.assertEqual(rc, 2)
                self.assertEqual(calls, [STATUS])

    def test_modern_success(self):
        self.assert_case(SUCCESS, "reaped", True, detail="")

    def test_deferred_grace_not_elapsed(self):
        self.assert_case(denial("grace_not_elapsed"), "deferred-grace_not_elapsed", True,
                         detail="")

    def test_deferred_state_changed(self):
        self.assert_case(denial("state_changed"), "deferred-state_changed", True, detail="")

    def test_queue_clear_success_retries_once(self):
        self.assert_case(denial("queue_not_empty"), "reaped-after-queue-clear", True,
                         tail=(CLEAR, REAP), retries=(SUCCESS,), detail="")

    def test_queue_clear_failure_does_not_retry(self):
        self.assert_case(denial("queue_not_empty"), "queue-clear-failed", False,
                         tail=(CLEAR,), clear=(1, "", "clear refused"), detail="clear refused")

    def test_queue_clear_retry_queue_denial_is_final(self):
        # A third success is bait for recursive retry: the correct caller never consumes it.
        self.assert_case(denial("queue_not_empty"), "reap-denied", False,
                         tail=(CLEAR, REAP), retries=(denial("queue_not_empty"), SUCCESS),
                         detail=denial("queue_not_empty")[2])

    def test_queue_clear_every_nonzero_retry_is_final(self):
        replies = [denial(reason) for reason in DENIALS]
        replies += [OLD_CLI, OLD_DAEMON, GENERIC_ERROR, TRANSPORT_ERROR, NOT_FOUND,
                    INVALID_PARAMS, denial("future_reason"), (127, "", "runner error")]
        for reply in replies:
            with self.subTest(retry=reply):
                self.assert_case(denial("queue_not_empty"), "reap-denied", False,
                                 tail=(CLEAR, REAP), retries=(reply,),
                                 requery=status_reply([]), detail=reply[2])

    def assert_manual_denial(self, reason):
        self.assert_case(denial(reason), "reap-denied", False,
                         requery=status_reply([]), detail=denial(reason)[2])

    def test_manual_caller_unresolved(self):
        self.assert_manual_denial("caller_unresolved")

    def test_manual_caller_role_forbidden(self):
        self.assert_manual_denial("caller_role_forbidden")

    def test_manual_active_surface(self):
        self.assert_manual_denial("active_surface")

    def test_manual_agent_still_alive(self):
        self.assert_manual_denial("agent_still_alive")

    def test_manual_daemon_ancestor(self):
        self.assert_manual_denial("daemon_ancestor")

    def test_recognized_denial_is_not_normalized_when_gone(self):
        # A confirming query would report absence, but recognized refusal is final.
        self.assert_case(denial("caller_role_forbidden"), "reap-denied", False,
                         requery=status_reply([]), detail=denial("caller_role_forbidden")[2])

    def test_old_cli_legacy_success(self):
        self.assert_case(OLD_CLI, "reaped-legacy", True, tail=(LEGACY,), detail="")

    def test_modern_cli_old_daemon_legacy_success(self):
        self.assert_case(OLD_DAEMON, "reaped-legacy", True, tail=(LEGACY,), detail="")

    def test_legacy_success_does_not_verify_disappearance(self):
        # Pin lines 173–175: exit 0 is reported success, not verified containment.
        # The still-present response is intentionally not queried (design line 268).
        self.assert_case(OLD_CLI, "reaped-legacy", True, tail=(LEGACY,),
                         requery=status_reply([SURFACE]), detail="")

    def test_old_cli_new_daemon_legacy_denied_target_present(self):
        # Inert design-consequence fixture, not evidence of the real new close gate.
        self.assert_case(OLD_CLI, "close-failed", False, tail=(LEGACY, STATUS),
                         legacy=(1, "", "error: close denied"), detail="error: close denied")

    def test_old_cli_new_daemon_legacy_denied_target_gone(self):
        self.assert_case(OLD_CLI, "already-gone", True, tail=(LEGACY, STATUS),
                         legacy=(1, "", "error: close denied"), requery=status_reply([]),
                         detail="")

    def test_modern_cli_old_daemon_legacy_failure(self):
        self.assert_case(OLD_DAEMON, "close-failed", False, tail=(LEGACY, STATUS),
                         legacy=(1, "", "legacy close refused"), detail="legacy close refused")

    def test_legacy_failure_unknown_status_stays_failed(self):
        self.assert_case(OLD_CLI, "close-failed", False, tail=(LEGACY, STATUS),
                         legacy=(1, "", "legacy close refused"),
                         requery=(1, "", "status unavailable"), detail="legacy close refused")

    def test_generic_exit_one_does_not_fallback(self):
        self.assert_case(GENERIC_ERROR, "close-failed", False, tail=(STATUS,),
                         detail=GENERIC_ERROR[2])

    def test_transport_error_does_not_fallback(self):
        self.assert_case(TRANSPORT_ERROR, "close-failed", False, tail=(STATUS,),
                         detail=TRANSPORT_ERROR[2])

    def test_not_found_does_not_fallback(self):
        self.assert_case(NOT_FOUND, "close-failed", False, tail=(STATUS,), detail=NOT_FOUND[2])

    def test_invalid_params_does_not_fallback(self):
        self.assert_case(INVALID_PARAMS, "close-failed", False, tail=(STATUS,),
                         detail=INVALID_PARAMS[2])

    def test_other_exit_two_is_not_capability_absence(self):
        self.assert_case((2, "", "error: invalid argument"), "close-failed", False,
                         tail=(STATUS,))

    def test_capability_text_requires_matching_exit_code(self):
        for reply in ((1, "", OLD_CLI[2]), (2, "", OLD_DAEMON[2]),
                      (7, "", OLD_DAEMON[2]), (1, OLD_DAEMON[2], "")):
            with self.subTest(reply=reply):
                self.assert_case(reply, "close-failed", False, tail=(STATUS,))

    def test_unknown_denial_requeries_present_target(self):
        self.assert_case(denial("future_reason"), "close-failed", False, tail=(STATUS,))

    def test_unknown_denial_requeries_gone_target(self):
        self.assert_case(denial("future_reason"), "already-gone", True, tail=(STATUS,),
                         requery=status_reply([]), detail="")

    def test_unclassified_error_gone_target_is_already_gone(self):
        self.assert_case(NOT_FOUND, "already-gone", True, tail=(STATUS,),
                         requery=status_reply([]), detail="")

    def test_unclassified_error_unknown_status_stays_failed(self):
        for reply in ((1, "", "daemon down"), (0, "not json", ""),
                      (0, '{"surfaces": null}', "")):
            with self.subTest(status=reply):
                self.assert_case(GENERIC_ERROR, "close-failed", False, tail=(STATUS,),
                                 requery=reply, detail=GENERIC_ERROR[2])

    def test_runner_exception_is_unclassified_not_capability_absence(self):
        self.assert_case(OSError("scripted transport failure"), "close-failed", False,
                         tail=(STATUS,), detail="runner error: scripted transport failure")

    def test_snapshot_failure_does_not_block_reap(self):
        for reply in ((1, "", "exited pane unreadable"), (0, "   ", "")):
            with self.subTest(screen=reply):
                row = self.assert_case(SUCCESS, "reaped", True, screen=reply)
                self.assertIsNone(row["snapshot"])

    def test_no_targets_is_ok(self):
        document, rc, calls = self.invoke({STATUS: [status_reply([])]})
        self.assertEqual(document, {"status": "ok", "exited_found": 0, "results": []})
        self.assertEqual(rc, 0)
        self.assertEqual(calls, [STATUS])

    def test_only_strict_exited_and_valid_refs_are_targeted(self):
        rows = [{"surface_ref": "surface:live", "exited": False},
                {"surface_ref": "surface:string", "exited": "true"},
                {"surface_ref": "surface:number", "exited": 1},
                {"surface_ref": "  ", "exited": True}, {"exited": True},
                {"surface_ref": 42, "exited": True}, "garbage", SURFACE]
        document, rc, calls = self.invoke({STATUS: [status_reply(rows)],
                                          SCREEN: [(0, "final screen text", "")],
                                          REAP: [SUCCESS]})
        self.assertEqual([(r["surface_ref"], r["action"], r["ok"])
                          for r in document["results"]], [(REF, "reaped", True)])
        self.assertEqual((document["status"], document["exited_found"], rc), ("ok", 1, 0))
        self.assertEqual(calls, [STATUS, SCREEN, REAP])

    def test_aggregate_deferred_and_success_is_ok(self):
        self.assert_aggregate(denial("state_changed"), "deferred-state_changed", True)

    def test_aggregate_any_false_row_is_partial_failure_and_continues(self):
        self.assert_aggregate(denial("active_surface"), "reap-denied", False)

    def assert_aggregate(self, first_reply, first_action, first_ok):
        second = dict(SURFACE, surface_ref="surface:43")
        third = dict(SURFACE, surface_ref="surface:44")
        screen2 = ("cys", "read-screen", "--surface", second["surface_ref"])
        reap2 = ("cys", "reap-surface", second["surface_ref"])
        screen3 = ("cys", "read-screen", "--surface", third["surface_ref"])
        reap3 = ("cys", "reap-surface", third["surface_ref"])
        document, rc, calls = self.invoke({
            STATUS: [status_reply([SURFACE, second, third])],
            SCREEN: [(0, "final screen text", "")], REAP: [first_reply],
            screen2: [(0, "final screen text", "")], reap2: [denial("grace_not_elapsed")],
            screen3: [(0, "final screen text", "")], reap3: [SUCCESS]})
        self.assertEqual([(r["action"], r["ok"]) for r in document["results"]],
                         [(first_action, first_ok), ("deferred-grace_not_elapsed", True),
                          ("reaped", True)])
        self.assertEqual((document["status"], rc),
                         ("ok", 0) if first_ok else ("partial-failure", 1))
        self.assertEqual(document["exited_found"], 3)
        self.assertEqual(calls, [STATUS, SCREEN, REAP, screen2, reap2, screen3, reap3])

    def test_dry_run_has_no_snapshot_or_reap_calls(self):
        document, rc, calls = self.invoke({STATUS: [status_reply([SURFACE])]}, ("--dry-run",))
        self.assertEqual(document, {"status": "ok", "exited_found": 1, "results": [
            {"surface_ref": REF, "role": "worker", "title": "done", "action": "dry-run",
             "snapshot": None, "ok": True}]})
        self.assertEqual(rc, 0)
        self.assertEqual(calls, [STATUS])


if __name__ == "__main__":
    unittest.main()
