#!/usr/bin/env python3
"""S7: isolated harness close/cleanup contract; no cys/cysd or live endpoint.

All import-time cleanup hooks and external process operations are intercepted.
Run directly with python3 -I; no third-party dependencies.
"""
import contextlib
import importlib.util
import io
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.dont_write_bytecode = True
HARNESS = os.path.join(os.path.dirname(__file__), "..", "javis_phoenix_harness.py")


class HarnessCloseTests(unittest.TestCase):
    def setUp(self):
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        # Canonical path: on macOS the temp dir is reached through a symlink (/tmp, /var), and an unresolved path would make
        # _harness_paths() refuse with aliased_harness_root before the check a test targets (release run 39747056302).
        self.tmp = os.path.realpath(self.stack.enter_context(tempfile.TemporaryDirectory(prefix="ph-close-")))
        spec = importlib.util.spec_from_file_location("s7_harness", HARNESS)
        self.h = importlib.util.module_from_spec(spec)
        with patch("atexit.register"), patch("signal.signal"):
            spec.loader.exec_module(self.h)
        h = self.h
        h.HARN_DIR = os.path.join(os.path.realpath(self.tmp), "scratch")
        h.HARN_SOCK = os.path.join(h.HARN_DIR, "cys.sock")
        h.HARN_STATE = os.path.join(h.HARN_DIR, "state")
        h.LIVE_STATE = os.path.join(self.tmp, "live")
        h.LIVE_SOCK = os.path.join(h.LIVE_STATE, "cys.sock")
        h.LIVE_LEDGER_ROOT = os.path.join(self.tmp, "ledger")
        os.makedirs(h.HARN_STATE)
        self.token = "S7_SYNTHETIC_DO_NOT_LOG"
        with open(os.path.join(h.HARN_DIR, "operator.token"), "w") as f:
            f.write(self.token)
        self.present = True
        self.error = True
        self.requests = []
        self.connections = []
        self.cli = []
        self.closed_sockets = 0
        self.reply_filter = lambda req, reply: reply
        self.raw_reply = None
        self.keep_present = False
        self.fragment_size = 65536
        self.real_capture = getattr(h, "_capture_owned", None)
        self.real_discovery = h.harness_daemon_pids
        h._CLOSE_TIMEOUT = 0.04
        h._CLEANUP_TIMEOUT = 0.08
        self.stack.enter_context(patch.object(h, "harness_ping", return_value=True))
        self.stack.enter_context(patch.object(h, "harness_daemon_pids", return_value=[]))
        self.stack.enter_context(patch.object(h, "_capture_owned", create=True))
        self.stack.enter_context(patch.object(h, "cys", side_effect=self.fake_cys))
        self.stack.enter_context(patch("socket.socket", side_effect=self.fake_socket))
        self.stack.enter_context(patch("subprocess.run", side_effect=AssertionError("external process forbidden")))
        self.stack.enter_context(patch("subprocess.Popen", side_effect=AssertionError("external process forbidden")))
        self.stack.enter_context(patch("os.kill", side_effect=AssertionError("real kill forbidden")))
        self.stack.enter_context(patch("os.killpg", side_effect=AssertionError("real killpg forbidden")))
        self.output = self.stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
        self.stack.enter_context(contextlib.redirect_stderr(io.StringIO()))

    def fake_cys(self, *args, **kwargs):
        self.cli.append(args)
        if args[0] == "list":
            return SimpleNamespace(returncode=0, stdout="surface:7 pid=700 exited=false\n" if self.present else "", stderr="")
        if args[0] == "close-surface":
            return SimpleNamespace(returncode=1, stdout="", stderr="close_denied")
        raise AssertionError("unexpected CLI verb: " + str(args[0]))

    def fake_socket(self, *args):
        test = self
        class Socket:
            def settimeout(self, timeout):
                test.assertGreater(timeout, 0)
            def connect(self, endpoint):
                test.connections.append(endpoint)
                test.assertEqual(endpoint, test.h.HARN_SOCK, "endpoint escaped scratch")
            def sendall(self, wire):
                req = json.loads(wire)
                test.requests.append(req)
                if req["method"] == "surface.list":
                    result = {"surfaces": [{"surface_id": 7, "pid": 700}] if test.present else []}
                    reply = {"id": req["id"], "ok": True, "result": result}
                elif req["method"] == "surface.close":
                    if test.error:
                        reply = {"id": req["id"], "ok": False, "error": {"code": "close_denied"}}
                    else:
                        if not test.keep_present:
                            test.present = False
                        reply = {"id": req["id"], "ok": True, "result": {"surface_id": 7, "closed": True}}
                elif req["method"] == "system.ping":
                    raise ConnectionRefusedError(61, "fixture down")
                else:
                    raise AssertionError("unexpected RPC")
                reply = test.reply_filter(req, reply)
                self.reply = test.raw_reply if test.raw_reply is not None else (json.dumps(reply) + "\n").encode()
            def recv(self, size):
                size = min(size, test.fragment_size)
                result, self.reply = self.reply[:size], self.reply[size:]
                return result
            def close(self):
                test.closed_sockets += 1
        return Socket()

    def test_close_error_not_reported_closed(self):
        closed = self.h.close_all_surfaces()
        self.assertEqual(closed, [], "refused close must never enter closed")
        self.assertEqual(getattr(self.h, "_cleanup_counts", {}).get("close_failure", 0), 1)

    def test_raw_owner_route_and_rotation(self):
        self.error = False
        closed = self.h.close_all_surfaces()
        self.assertFalse(any(c[0] == "close-surface" for c in self.cli), "public CLI close is forbidden")
        self.assertEqual(closed, ["surface:7"])
        closes = [r for r in self.requests if r["method"] == "surface.close"]
        self.assertEqual(closes[0]["params"], {"surface_id": 7, "owner_token": self.token})
        self.present = True
        with open(os.path.join(self.h.HARN_DIR, "operator.token"), "w") as f:
            f.write("SECOND_SYNTHETIC_TOKEN")
        self.assertTrue(self.h._close_surface("surface:7")["ok"])
        closes = [r for r in self.requests if r["method"] == "surface.close"]
        self.assertEqual(closes[-1]["params"]["owner_token"], "SECOND_SYNTHETIC_TOKEN")
        self.assertEqual(self.h._cleanup_counts["close_success"], 2)
        self.assertNotIn(self.token, self.output.getvalue() + json.dumps(self.h._last_close))

    def test_inherited_endpoint_ignored(self):
        self.error = False
        with patch.dict(os.environ, {"CYS_SOCKET": self.h.LIVE_SOCK,
                                    "JAVIS_SOCKET": self.h.LIVE_SOCK,
                                    "AITERM_SOCKET": self.h.LIVE_SOCK,
                                    "OWNER_TOKEN": "FOREIGN_TOKEN"}):
            self.assertEqual(self.h.close_all_surfaces(), ["surface:7"])
        self.assertTrue(self.connections)
        self.assertEqual(set(self.connections), {self.h.HARN_SOCK})
        self.assertEqual([r["params"]["owner_token"] for r in self.requests
                          if r["method"] == "surface.close"], [self.token])

    def test_error_then_absent_stays_failure(self):
        def reply_filter(req, reply):
            if req["method"] == "surface.close":
                self.present = False
                reply["error"]["message"] = self.token
            return reply
        self.reply_filter = reply_filter
        self.assertFalse(self.h._close_surface("surface:7")["ok"])
        self.assertEqual(self.h._cleanup_counts["close_failure"], 1)
        self.assertNotIn(self.token, json.dumps(self.h._last_close) + self.output.getvalue())

    def test_success_requires_absence(self):
        self.error = False
        self.keep_present = True
        start = time.monotonic()
        self.assertFalse(self.h._close_surface("surface:7")["ok"])
        self.assertLess(time.monotonic() - start, 0.5)
        self.assertEqual(self.h._cleanup_counts["close_failure"], 1)

    def test_inventory_compares_exact_ids(self):
        self.error = False
        def reply_filter(req, reply):
            if req["method"] == "surface.list" and not self.present:
                reply["result"]["surfaces"] = [{"surface_id": 70, "pid": 701}]
            return reply
        self.reply_filter = reply_filter
        self.assertTrue(self.h._close_surface("surface:7")["ok"])

    def test_valid_fragmented_reply(self):
        self.error = False
        self.fragment_size = 3
        self.assertTrue(self.h._close_surface("surface:7")["ok"])

    def test_two_endpoints_colliding_sid_and_swapped_token(self):
        # Client containment model only: real daemon credential refusal belongs to S11.
        h = self.h
        other_dir = os.path.join(os.path.realpath(self.tmp), "other")
        os.makedirs(other_dir)
        other_sock = os.path.join(other_dir, "cys.sock")
        endpoint_a = h.HARN_SOCK
        states = {endpoint_a: {"token": self.token, "present": True},
                  other_sock: {"token": "OTHER_SYNTHETIC_TOKEN", "present": True}}
        test = self
        class EndpointSocket:
            def settimeout(self, timeout): pass
            def connect(self, endpoint):
                test.assertEqual(endpoint, endpoint_a, "only selected harness endpoint may be contacted")
                self.state = states[endpoint]
            def sendall(self, wire):
                req = json.loads(wire)
                reply = {"id": req["id"], "ok": True}
                if req["method"] == "surface.close":
                    if req["params"].get("owner_token") != self.state["token"]:
                        reply.update(ok=False, error={"code": "close_denied"})
                    else:
                        self.state["present"] = False
                        reply["result"] = {"surface_id": 7, "closed": True}
                else:
                    reply["result"] = {"surfaces": [{"surface_id": 7, "pid": 700}] if self.state["present"] else []}
                self.wire = (json.dumps(reply) + "\n").encode()
            def recv(self, size):
                wire, self.wire = self.wire[:size], self.wire[size:]
                return wire
            def close(self): pass
        path = os.path.join(h.HARN_DIR, "operator.token")
        with patch("socket.socket", return_value=EndpointSocket()), patch.dict(os.environ, {"CYS_SOCKET": other_sock}):
            with open(path, "w") as f:
                f.write(states[other_sock]["token"])
            self.assertFalse(h._close_surface("surface:7")["ok"])
            self.assertTrue(states[endpoint_a]["present"])
            self.assertTrue(states[other_sock]["present"])
            with open(path, "w") as f:
                f.write(self.token)
            self.assertTrue(h._close_surface("surface:7")["ok"])
            self.assertFalse(states[endpoint_a]["present"])
            self.assertTrue(states[other_sock]["present"])
            self.assertEqual(h._cleanup_counts["close_failure"], 1)

    def test_wire_rejections_close_socket(self):
        for value in (b"{}\n", b"not json\n", b"{}", b"", b"x" * 1100):
            with self.subTest(reply_size=len(value)), patch.object(self.h, "_RPC_MAX_BYTES", 1024):
                self.raw_reply = value
                before = self.closed_sockets
                with self.assertRaises(self.h._HarnessError):
                    self.h._harness_rpc("surface.list")
                self.assertEqual(self.closed_sockets, before + 1)

    def test_socket_creation_failure_is_counted(self):
        with patch("socket.socket", side_effect=OSError("unavailable")):
            self.assertFalse(self.h._close_surface("surface:7")["ok"])
        self.assertEqual(self.h._cleanup_counts["close_failure"], 1)

    def test_close_envelope_requires_true_and_matching_sid(self):
        self.error = False
        variants = [lambda r: dict(r, ok=1), lambda r: dict(r, id="wrong"),
                    lambda r: dict(r, result={"surface_id": 8, "closed": True}),
                    lambda r: dict(r, result={"surface_id": 7, "closed": False}),
                    lambda r: dict(r, result={"surface_id": 7, "closed": True}, error={})]
        for mutate in variants:
            self.present = True
            self.reply_filter = lambda req, rep: mutate(rep) if req["method"] == "surface.close" else rep
            self.assertFalse(self.h._close_surface("surface:7")["ok"])
        self.assertEqual(self.h._cleanup_counts["close_failure"], len(variants))

    def test_slow_fragments_use_one_deadline(self):
        class Slow:
            def settimeout(self, timeout): pass
            def connect(self, endpoint): pass
            def sendall(self, request): pass
            def recv(self, size):
                time.sleep(0.015)
                return b" "
            def close(self): self.closed = True
        sock = Slow()
        with patch("socket.socket", return_value=sock):
            start = time.monotonic()
            with self.assertRaisesRegex(self.h._HarnessError, "timeout"):
                self.h._harness_rpc("surface.list")
            self.assertLess(time.monotonic() - start, 0.25)
        self.assertTrue(sock.closed)

    def test_missing_empty_and_fifo_token(self):
        token_path = os.path.join(self.h.HARN_DIR, "operator.token")
        for kind in ("missing", "empty", "fifo"):
            if os.path.lexists(token_path):
                os.unlink(token_path)
            if kind == "empty":
                open(token_path, "w").close()
            elif kind == "fifo":
                if os.name == "nt":
                    continue
                os.mkfifo(token_path)
            self.assertFalse(self.h._close_surface("surface:7")["ok"])
        self.assertFalse(any(r["method"] == "surface.close" for r in self.requests))

    def test_path_escapes_refused_before_connection(self):
        h = self.h
        outside = os.path.join(self.tmp, "second-endpoint")
        os.makedirs(outside)
        for name in ("state", "cys.sock", "operator.token"):
            path = os.path.join(h.HARN_DIR, name)
            backup = path + ".original"
            existed = os.path.lexists(path)
            if existed:
                os.rename(path, backup)
            os.symlink(outside, path)
            try:
                with self.assertRaises(h._HarnessError):
                    h._harness_paths()
            finally:
                os.unlink(path)
                if existed:
                    os.rename(backup, path)
        original = h.HARN_DIR
        alias = os.path.join(self.tmp, "alias")
        os.symlink(outside, alias)
        h.HARN_DIR, h.HARN_SOCK, h.HARN_STATE = alias, os.path.join(alias, "cys.sock"), os.path.join(alias, "state")
        with self.assertRaisesRegex(h._HarnessError, "aliased_harness_root"):
            h._harness_paths()
        self.assertEqual(self.connections, [])
        h.HARN_DIR = original

    def test_live_optin_does_not_authorize_raw_route(self):
        self.h.HARN_DIR = self.h.LIVE_STATE
        self.h.HARN_SOCK = self.h.LIVE_SOCK
        with patch.dict(os.environ, {"CYS_PHOENIX_ALLOW_LIVE": "1"}):
            with self.assertRaisesRegex(self.h._HarnessError, "live_overlap"):
                self.h._harness_paths()
        self.assertEqual(self.connections, [])

    def test_explicit_foreign_endpoint_refused(self):
        with self.assertRaisesRegex(self.h._HarnessError, "foreign_endpoint"):
            self.h._harness_paths(self.h.LIVE_SOCK)
        self.assertFalse(self.h._inside(self.h.HARN_DIR + "-other", self.h.HARN_DIR))

    def test_windows_mapping_and_explicit_transport_refusal(self):
        h = self.h
        local = os.path.join(h.HARN_DIR, "localapp")
        self.assertEqual(h._windows_token_dir(r"\\.\pipe\cys-x.1", local), os.path.join(local, "cys", "cys-x1"))
        self.assertEqual(h._windows_token_dir(r"\\.\pipe\cys", local), os.path.join(local, "cys"))
        h.HARN_SOCK = r"\\.\pipe\cys-harness-7"
        with patch.object(os, "name", "nt"), patch.dict(os.environ, {"LOCALAPPDATA": local}):
            self.assertEqual(h._harness_paths()[1], os.path.join(local, "cys", "cys-harness-7", "operator.token"))
            outcome = h._close_surface("surface:7")
            self.assertEqual(outcome["reason"], "windows_transport_unsupported")
            self.assertFalse(outcome["ok"])
        self.assertEqual(self.connections, [])

    def test_launch_is_not_cleanup_success(self):
        h = self.h
        h._owned_processes[700] = (10, 20)
        info = lambda pid: ((10, 20), 1)
        with patch.object(h, "_process_info", side_effect=info), patch("os.kill", return_value=None) as kill:
            h.teardown()
            self.assertTrue(kill.called)
            self.assertEqual(h._cleanup_counts["fallback_cleanup_success"], 0,
                             "successful signal/launch is not observed cleanup completion")
            report = h.residual_report()
            self.assertEqual(report["owned_process_pids"], [700])
            self.assertFalse(report["clean"])
            with self.assertRaises(SystemExit):
                h.cmd_down(None)

    def test_fallback_success_preserves_close_failure(self):
        h = self.h
        h._owned_processes[700] = (10, 20)
        alive = {700: ((10, 20), 1)}
        with patch.object(h, "_process_info", side_effect=lambda pid: alive.get(pid)), \
                patch("os.kill", side_effect=lambda pid, sig: alive.pop(pid, None)):
            h.teardown()
            self.assertEqual(h._cleanup_counts["close_failure"], 1)
            self.assertEqual(h._cleanup_counts["close_success"], 0)
            self.assertEqual(h._cleanup_counts["fallback_cleanup_success"], 1)
            self.assertTrue(h.residual_report()["clean"])
            self.present = False
            h.teardown()
            self.assertEqual(h._cleanup_counts["close_failure"], 1)
            self.assertEqual(h._cleanup_counts["fallback_cleanup_success"], 1)

    def test_parent_exit_retains_descendant_and_reused_pid_is_not_killed(self):
        h = self.h
        infos = {700: ((10, 20), 1), 701: ((11, 21), 700), 702: ((12, 22), 701)}
        ps = SimpleNamespace(returncode=0, stdout="700 1\n701 700\n702 701\n")
        with patch.object(h, "_capture_owned", self.real_capture), \
                patch.object(h, "_process_info", side_effect=lambda pid: infos.get(pid)), \
                patch("subprocess.run", return_value=ps):
            h._capture_owned([700])
        self.assertEqual(set(h._owned_processes), {700, 701, 702})
        infos.pop(700)
        infos[701] = ((99, 99), 1)  # same PID, different process; NOT ours
        infos[702] = ((12, 22), 1)  # original grandchild now reparented
        with patch.object(h, "_process_info", side_effect=lambda pid: infos.get(pid)), \
                patch("os.kill", return_value=None) as kill:
            self.assertEqual(h.residual_report()["owned_process_pids"], [702])
            self.assertFalse(h._residual_zero())
            h.teardown()
            self.assertEqual([c.args[0] for c in kill.call_args_list], [702])

    def test_unobservable_process_blocks_clean(self):
        h = self.h
        h._owned_processes[700] = (10, 20)
        with patch.object(h, "_process_info", side_effect=h._HarnessError("probe_unavailable")):
            report = h.residual_report()
        self.assertFalse(report["clean"])
        self.assertTrue(report["unobserved_owned"])

    def test_invalid_ping_cannot_prove_absence(self):
        with patch.object(self.h, "_harness_rpc", return_value="not-pong"):
            report = self.h.residual_report()
        self.assertFalse(report["clean"])
        self.assertIn("endpoint_unobserved", report["unobserved_owned"])

    def test_teardown_final_deadline_returns_residuals(self):
        h = self.h
        report = {"clean": False, "unobserved_owned": [], "harness_daemon_pids": [700]}
        with patch.object(h, "close_all_surfaces", return_value=[]), \
                patch.object(h, "_residual_snapshot", return_value=report), \
                patch("time.monotonic", side_effect=[0.0, 0.0, 0.05, 0.09]):
            self.assertEqual(h.teardown(), [700])
        self.assertEqual(h._cleanup_counts["fallback_cleanup_success"], 0)

    def test_failed_process_capture_remains_unobserved(self):
        h = self.h
        with patch.object(h, "_capture_owned", self.real_capture), \
                patch.object(h, "_process_info", return_value=((10, 20), 1)), \
                patch("subprocess.run", side_effect=subprocess.TimeoutExpired("ps", 0.1)):
            h._capture_owned([700])
        with patch.object(h, "_process_info", return_value=None):
            self.assertFalse(h.residual_report()["clean"])
        self.assertEqual(h._ownership_errors, {"ownership_capture_incomplete"})

    def test_discovery_handoff_never_adopts_reused_pid(self):
        h = self.h
        old, new = ((10, 20), 1), ((99, 99), 1)
        def run(cmd, **kw):
            if cmd[0] == "pgrep":
                return SimpleNamespace(returncode=0, stdout="700\n")
            self.assertEqual(cmd, ["lsof", "-p", "700", "-Fn"])
            return SimpleNamespace(returncode=0, stdout="n" + h.HARN_SOCK + "\n")
        with patch("subprocess.run", side_effect=run), \
                patch.object(h, "_process_info", side_effect=[old, old]):
            self.assertEqual(self.real_discovery(), [700])
        self.assertEqual(h._owned_processes[700], old[0])
        with patch.object(h, "_process_info", return_value=new), \
                patch.object(h, "_capture_owned", self.real_capture), \
                patch.object(h, "close_all_surfaces", return_value=[]), patch("os.kill") as kill:
            h._capture_owned([700])
            h.teardown()
            self.assertFalse(kill.called, "reused PID after lsof must never become an owned kill target")
            self.assertFalse(h.residual_report()["clean"], "unobserved original descendants cannot mean zero")
        self.assertEqual(h._owned_processes[700], old[0])

    def test_discovery_identity_change_during_lsof_refused(self):
        h = self.h
        results = [SimpleNamespace(returncode=0, stdout="700\n"),
                   SimpleNamespace(returncode=0, stdout="n" + h.HARN_SOCK + "\n")]
        with patch("subprocess.run", side_effect=results), \
                patch.object(h, "_process_info", side_effect=[((10, 20), 1), ((99, 99), 1)]):
            with self.assertRaisesRegex(h._HarnessError, "daemon_identity_changed"):
                self.real_discovery()
        self.assertNotIn(700, h._owned_processes)
        self.assertIn("daemon_identity_changed", h._ownership_errors)

    def test_caller_exception_still_closes(self):
        h = self.h
        def cli(*args, **kw):
            if args[0] == "new-surface":
                return SimpleNamespace(returncode=0, stdout="surface:7", stderr="")
            if args[0] == "watch":
                return SimpleNamespace(returncode=0, stdout="marker", stderr="")
            raise RuntimeError("scripted observation failure")
        with patch.object(h, "_fresh_harness"), patch.object(h, "guard_isolation"), \
                patch.object(h, "cys", side_effect=cli), patch("time.sleep"):
            with self.assertRaises(RuntimeError):
                h.cmd_crash_agent(None)
            with self.assertRaises(RuntimeError):
                h._q_watch()
        self.assertEqual(h._cleanup_counts["close_failure"], 2)
        self.assertEqual(sum(r["method"] == "surface.close" for r in self.requests), 2)

    def test_crash_agent_and_watch_use_real_close_route(self):
        h = self.h
        def cli(*args, **kw):
            if args[0] == "new-surface":
                return SimpleNamespace(returncode=0, stdout="surface:7", stderr="")
            if args[0] == "list":
                return SimpleNamespace(returncode=0, stdout="surface:7 pid=700 exited=true", stderr="")
            if args[0] in ("watch", "send", "send-key"):
                return SimpleNamespace(returncode=0, stdout="marker", stderr="")
            raise AssertionError("unexpected public CLI close")
        with patch.object(h, "_fresh_harness"), patch.object(h, "guard_isolation"), \
                patch.object(h, "cys", side_effect=cli), patch("os.kill"), patch("time.sleep"):
            crash = h.cmd_crash_agent(None)
            watch = h._q_watch()
        self.assertTrue(crash["ok"])
        self.assertFalse(crash["cleanup_close"]["ok"])
        self.assertEqual(watch["verdict"], "WORKS(마커 출력 시 블로킹 해제)")
        self.assertFalse(watch["cleanup_close"]["ok"])
        self.assertEqual(h._cleanup_counts["close_failure"], 2)
        self.assertEqual(sum(r["method"] == "surface.close" for r in self.requests), 2)


class WindowsExclusionTests(unittest.TestCase):
    def load_windows(self):
        spec = importlib.util.spec_from_file_location("s7_windows_harness", HARNESS)
        h = importlib.util.module_from_spec(spec)
        with patch("os.name", "nt"), patch("atexit.register") as register, patch("signal.signal") as signals:
            spec.loader.exec_module(h)
        return h, register.call_count, signals.call_count

    def test_windows_import_installs_no_unix_exit_hooks(self):
        _, registrations, signals = self.load_windows()
        self.assertEqual((registrations, signals), (0, 0), "S7-WINDOWS: no Unix teardown hooks on Windows")

    def test_windows_cli_excluded_before_binary_resolution(self):
        h, _, _ = self.load_windows()
        err = io.StringIO()
        with patch("os.name", "nt"), patch.object(sys, "argv", ["harness", "down"]), \
                patch.object(h, "_resolve_cys", return_value="inert-cys") as cys_resolver, \
                patch.object(h, "_resolve_cysd", return_value="inert-cysd") as daemon_resolver, \
                patch.object(h, "cmd_down", return_value={}) as down, contextlib.redirect_stderr(err):
            code = None
            try:
                h.main()
            except SystemExit as exc:
                code = exc.code
            self.assertEqual(code, 2, "S7-WINDOWS: explicit unsupported CLI result")
            cys_resolver.assert_not_called()
            daemon_resolver.assert_not_called()
            down.assert_not_called()
        self.assertIn("Windows", err.getvalue())
        self.assertIn("javis_phoenix_win_smoke.py", err.getvalue())


@unittest.skipUnless(os.environ.get("S7_NATIVE_DUMMY") == "1" and sys.platform == "darwin",
                     "macOS-only opt-in native tree; Linux orphan reaping and Windows are unmeasured")
class NativeOwnedCleanupTests(unittest.TestCase):
    def test_actual_reparented_grandchild_cleanup(self):
        spec = importlib.util.spec_from_file_location("s7_native_harness", HARNESS)
        h = importlib.util.module_from_spec(spec)
        with patch("atexit.register"), patch("signal.signal"):
            spec.loader.exec_module(h)
        with tempfile.TemporaryDirectory(prefix="ph-native-") as tmp:
            tmp = os.path.realpath(tmp)
            h.HARN_DIR = os.path.join(tmp, "scratch")
            h.HARN_SOCK = os.path.join(h.HARN_DIR, "cys.sock")
            h.HARN_STATE = os.path.join(h.HARN_DIR, "state")
            h.LIVE_STATE = os.path.join(tmp, "never-live")
            h.LIVE_SOCK = os.path.join(h.LIVE_STATE, "cys.sock")
            h.LIVE_LEDGER_ROOT = os.path.join(tmp, "never-ledger")
            os.makedirs(h.HARN_STATE)
            with open(os.path.join(h.HARN_DIR, "operator.token"), "w") as f:
                f.write("S7_DUMMY_ONLY")
            # Each descendant has its own <=6 second lifetime, even if this test fails.
            child_code = """
import os, subprocess, sys, time
p = subprocess.Popen([sys.executable, '-I', '-c', 'import time; time.sleep(6)'],
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
with open('grandchild.pid', 'w') as f: f.write(str(p.pid))
end = time.monotonic() + 5
while not os.path.exists('leave') and time.monotonic() < end: time.sleep(.01)
"""
            root_code = """
import os, subprocess, sys, time
p = subprocess.Popen([sys.executable, '-I', '-c', CHILD],
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
with open('child.pid', 'w') as f: f.write(str(p.pid))
p.wait(timeout=6)
""".replace("CHILD", repr(child_code))
            root = subprocess.Popen([sys.executable, "-I", "-c", root_code], cwd=h.HARN_DIR,
                                    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL, start_new_session=True)
            evidence = {"fixture": "python-only root/child/grandchild", "native": sys.platform}
            try:
                deadline = time.monotonic() + 3
                paths = [os.path.join(h.HARN_DIR, name) for name in ("child.pid", "grandchild.pid")]
                while not all(os.path.exists(p) and os.path.getsize(p) for p in paths):
                    self.assertLess(time.monotonic(), deadline, "dummy tree readiness deadline")
                    time.sleep(.01)
                pids = []
                for path in paths:
                    with open(path) as f:
                        pids.append(int(f.read()))
                child, grandchild = pids
                h._capture_owned([root.pid])
                self.assertEqual(set(h._owned_processes), {root.pid, child, grandchild})
                self.assertEqual(h._ownership_errors, set())
                evidence["before"] = [{"pid": p, "start": list(i)} for p, i in h._owned_processes.items()]
                def rpc(method, params=None, deadline=None):
                    if method == "surface.list":
                        return {"surfaces": [{"surface_id": 7, "pid": child}]} if h._process_info(child) else {"surfaces": []}
                    if method == "surface.close":
                        raise h._HarnessError("rpc_error")
                    raise h._HarnessError("unavailable")  # no actual endpoint exists
                with patch.object(h, "_harness_rpc", side_effect=rpc), \
                        patch.object(h, "harness_daemon_pids", return_value=[]):
                    self.assertFalse(h._close_surface("surface:7")["ok"])
                    open(os.path.join(h.HARN_DIR, "leave"), "w").close()
                    root.wait(timeout=2)  # root itself reaps the child before exiting
                    before_cleanup = h.residual_report()
                    evidence["after_parent_exit"] = before_cleanup
                    self.assertIn(grandchild, before_cleanup["owned_process_pids"])
                    self.assertFalse(before_cleanup["clean"])
                    h.teardown()
                    after = h.residual_report()
                    evidence["after_cleanup"] = after
                    self.assertTrue(after["clean"], after)
                    self.assertEqual(after["owned_process_pids"], [])
                    self.assertEqual(after["close_failure"], 1)
                    self.assertEqual(after["close_success"], 0)
                    self.assertEqual(after["fallback_cleanup_success"], 1)
                    self.assertIsNone(h._process_info(grandchild))
            finally:
                # Original identities only, never name/group killing; finite child
                # lifetime also protects a failure before the capture completed.
                for pid, identity in h._owned_processes.items():
                    info = h._process_info(pid)
                    if info and info[0] == identity:
                        try:
                            os.kill(pid, h.signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                if root.poll() is None:
                    root.kill()  # our direct Popen, not an arbitrary PID
                root.wait(timeout=2)
                evidence_path = os.environ.get("S7_NATIVE_EVIDENCE")
                if evidence_path:
                    with open(evidence_path, "w", encoding="utf-8") as f:
                        json.dump(evidence, f, indent=2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
