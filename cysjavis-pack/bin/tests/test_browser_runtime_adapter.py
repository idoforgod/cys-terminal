#!/usr/bin/env python3
"""Browser v2 lifecycle boundary tests.

Production Python is an intent adapter.  Source spawning is available only when
the developer explicitly opts into it, so a packaged install can never fall
back to a user Bun/Chrome by accident.
"""

import importlib.util
import os
from pathlib import Path
import sys
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).resolve().parents[1] / "javis_browser.py"
SPEC = importlib.util.spec_from_file_location("javis_browser_runtime_adapter", MODULE_PATH)
browser = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(browser)


class BrowserRuntimeAdapterTests(unittest.TestCase):
    def test_production_ensure_delegates_to_cysd_without_process_spawn(self):
        completed = mock.Mock(returncode=0, stdout='{"status":"COMPATIBLE"}\n', stderr="")
        with mock.patch.dict(os.environ, {"CYS_BROWSER_DEV": "0"}, clear=False), \
             mock.patch.object(browser, "_which", return_value="/opt/cys/bin/cys"), \
             mock.patch.object(browser.subprocess, "run", return_value=completed) as run, \
             mock.patch.object(browser.subprocess, "Popen") as popen:
            state, error = browser.ensure_browserd(headless=True)

        self.assertIsNone(error)
        self.assertEqual(state["transport"], "broker")
        self.assertEqual(
            run.call_args.args[0],
            ["/opt/cys/bin/cys", "browser-runtime-ensure", "--headless"],
        )
        popen.assert_not_called()

    def test_production_stop_never_signals_engine_pid(self):
        with mock.patch.dict(os.environ, {"CYS_BROWSER_DEV": "0"}, clear=False), \
             mock.patch.object(browser.os, "kill") as kill:
            ok, message = browser.stop_browserd()

        self.assertFalse(ok)
        self.assertIn("cysd", message)
        kill.assert_not_called()

    def test_source_spawn_requires_explicit_development_opt_in(self):
        with mock.patch.dict(os.environ, {"CYS_BROWSER_DEV": "1"}, clear=False), \
             mock.patch.object(browser, "_live_state", return_value={"pid": 7, "port": 8, "token": "t"}), \
             mock.patch.object(browser.subprocess, "run") as run:
            state, error = browser.ensure_browserd(headless=True)

        self.assertIsNone(error)
        self.assertEqual(state["transport"], "dev-direct")
        run.assert_not_called()

    def test_production_headful_only_operation_fails_before_broker_call(self):
        with mock.patch.dict(os.environ, {"CYS_BROWSER_DEV": "0"}, clear=False), \
             mock.patch.object(browser, "_cys_command") as broker_call, \
             mock.patch.object(browser, "audit"):
            exit_code = browser.guard_headful_required(
                "observe", {"url": "https://example.com", "profile": "agent"}
            )

        self.assertEqual(exit_code, browser.EXIT_START_FAIL)
        broker_call.assert_not_called()


if __name__ == "__main__":
    unittest.main()
