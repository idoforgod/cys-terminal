#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_hud_bridge_0144_unit.py — 0.14.44 B4(브리지 스크립트 묶음) 단위 시험.

지키는 것
  ① 윈도우(와 sys.platform 이 cygwin/msys)에서는 B4 의 1·5·6(시간 제한 · 포트 먼저 · 신호/수명줄/자식 표준입력)이 종전대로 —
     `HUD_WIN_NEW=1` 일 때만 새 동작. 운영체제 이름을 바꿔치기해 판정한다.
  ② 스크립트 소스 고정: 신호·부모 번호로 부모를 확인하는 호출 0건 · 프로세스 그룹 신호 0건 · 콘솔 숨김 플래그 1건 ·
     고정 임시 파일 이름 1건 · `def pick_ctx(node):` 존재.
  ③ 로그 줄 머리(`[hud-bridge] `) 유지 + 시각·스레드 이름. ④ /health 본문의 칸. ⑤ 응답 전송은 try 밖(B4-2).
실행: python3 test_hud_bridge_0144_unit.py
"""
import contextlib
import io
import json
import os
import re
import sys
import threading
import unittest
from unittest import mock

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)
import javis_hud_bridge as HB                                            # noqa: E402

SRC = open(os.path.join(BIN, "javis_hud_bridge.py"), encoding="utf-8").read()


def code_only(src):
    import tokenize
    toks = tokenize.generate_tokens(io.StringIO(src).readline)
    return " ".join(t.string for t in toks if t.type != tokenize.COMMENT)


class WindowsFake(unittest.TestCase):
    def _env(self, **kw):
        base = {k: v for k, v in os.environ.items() if k not in ("HUD_WIN_NEW", "HUD_REQ_TIMEOUT")}
        base.update(kw)
        return mock.patch.dict(os.environ, base, clear=True)

    def test_mac_defaults(self):
        with self._env(), mock.patch.object(HB.os, "name", "posix"), mock.patch.object(HB.sys, "platform", "darwin"):
            self.assertFalse(HB._is_windows())
            self.assertFalse(HB._legacy_win())
            self.assertEqual(HB._req_timeout(), 30.0)
            self.assertEqual(HB._nostdin(), {"stdin": HB.subprocess.DEVNULL})

    def test_timeout_knob(self):
        with mock.patch.object(HB.os, "name", "posix"), mock.patch.object(HB.sys, "platform", "darwin"):
            with self._env(HUD_REQ_TIMEOUT="0"):
                self.assertIsNone(HB._req_timeout(), "0 = 끔")
            with self._env(HUD_REQ_TIMEOUT="7.5"):
                self.assertEqual(HB._req_timeout(), 7.5)
            with self._env(HUD_REQ_TIMEOUT="abc"):
                self.assertEqual(HB._req_timeout(), 30.0, "잘못된 값은 기본")

    def test_windows_variants_are_legacy(self):
        for name, plat in (("nt", "win32"), ("posix", "cygwin"), ("posix", "msys"), ("nt", "cygwin")):
            with self._env(), mock.patch.object(HB.os, "name", name), mock.patch.object(HB.sys, "platform", plat):
                self.assertTrue(HB._is_windows(), (name, plat))
                self.assertTrue(HB._legacy_win(), (name, plat))
                self.assertIsNone(HB._req_timeout(), "윈도우 종전은 시간 제한 없음 %s/%s" % (name, plat))
                self.assertEqual(HB._nostdin(), {}, "윈도우 종전은 자식 표준입력 그대로 %s/%s" % (name, plat))

    def test_win_new_switch_turns_new_behavior_on(self):
        with self._env(HUD_WIN_NEW="1"), mock.patch.object(HB.os, "name", "nt"), mock.patch.object(HB.sys, "platform", "win32"):
            self.assertTrue(HB._is_windows())
            self.assertFalse(HB._legacy_win())
            self.assertEqual(HB._req_timeout(), 30.0)
            self.assertEqual(HB._nostdin(), {"stdin": HB.subprocess.DEVNULL})
        with self._env(HUD_WIN_NEW="0"), mock.patch.object(HB.os, "name", "nt"), mock.patch.object(HB.sys, "platform", "win32"):
            self.assertTrue(HB._legacy_win(), "1 이외의 값은 켜지 않는다")

    def test_main_on_windows_legacy_binds_late_and_installs_no_signal_or_lifeline(self):
        # main() 을 끝까지 돌리지 않고, 윈도우 종전 경로가 B4-5/6 의 호출을 하지 않는지만 본다.
        class Stop(Exception):
            pass
        calls = []
        def fake_bind():
            calls.append("bind_first")
            raise Stop()
        class FakeSrv:
            def __init__(self, *a, **k):
                calls.append("bind_late")
                raise Stop()
        with self._env(LIFE=""), mock.patch.dict(os.environ, {"HUD_LIFELINE": "stdin"}), \
                mock.patch.object(HB.os, "name", "nt"), mock.patch.object(HB.sys, "platform", "win32"), \
                mock.patch.object(HB, "_bind_or_exit", fake_bind), \
                mock.patch.object(HB.signal, "signal", side_effect=lambda *a: calls.append("signal")), \
                mock.patch.object(HB.threading, "Thread", side_effect=lambda *a, **k: (calls.append("thread:%s" % k.get("name")), mock.Mock())[1]), \
                mock.patch.object(HB, "_BridgeServer", FakeSrv), \
                mock.patch.object(HB, "World", side_effect=Stop):
            with self.assertRaises(Stop):
                HB.main()
        self.assertEqual(calls, [], "윈도우 종전: 포트를 먼저 잡지 않고 신호 처리기·수명줄 스레드도 없다: %r" % calls)

    def test_main_on_posix_binds_first_then_signal_then_lifeline(self):
        class Stop(Exception):
            pass
        calls = []
        def fake_bind():
            calls.append("bind_first")
            return mock.Mock()
        with self._env(HUD_LIFELINE="stdin"), mock.patch.object(HB.os, "name", "posix"), mock.patch.object(HB.sys, "platform", "darwin"), \
                mock.patch.object(HB, "_bind_or_exit", fake_bind), \
                mock.patch.object(HB.signal, "signal", side_effect=lambda *a: calls.append("signal")), \
                mock.patch.object(HB.threading, "Thread", side_effect=lambda *a, **k: (calls.append("thread:%s" % k.get("name")), mock.Mock())[1]), \
                mock.patch.object(HB, "World", side_effect=Stop):
            with self.assertRaises(Stop):
                HB.main()
        self.assertEqual(calls, ["bind_first", "signal", "thread:lifeline"])

    def test_lifeline_not_started_without_env(self):
        class Stop(Exception):
            pass
        calls = []
        with self._env(), mock.patch.object(HB.os, "name", "posix"), mock.patch.object(HB.sys, "platform", "darwin"), \
                mock.patch.object(HB, "_bind_or_exit", lambda: mock.Mock()), \
                mock.patch.object(HB.signal, "signal", side_effect=lambda *a: calls.append("signal")), \
                mock.patch.object(HB.threading, "Thread", side_effect=lambda *a, **k: (calls.append("thread"), mock.Mock())[1]), \
                mock.patch.object(HB, "World", side_effect=Stop):
            with self.assertRaises(Stop):
                HB.main()
        self.assertEqual(calls, ["signal"], "환경변수 없는 수동 기동은 수명줄 스레드를 만들지 않는다")


class SourcePins(unittest.TestCase):
    def setUp(self):
        self.code = code_only(SRC)

    def test_no_parent_probe_calls(self):
        self.assertNotIn("os.kill(", SRC, "신호로 부모를 확인하는 코드 금지(윈도우에서 콘솔 이벤트/강제 종료)")
        self.assertNotIn("getppid", SRC, "부모 번호 폴링 금지")
        self.assertNotIn("killpg", SRC, "프로세스 그룹 신호 금지")

    def test_creationflags_and_tmp_and_pick_ctx_pins(self):
        self.assertEqual(self.code.count("creationflags"), 1, "콘솔 숨김 플래그는 NOWIN 하나만")
        n = 0
        rx = re.compile(r'[A-Za-z_][A-Za-z0-9_.]*\s*\+\s*["\']\.tmp["\']')
        for ln in self.code.splitlines():
            if rx.search(ln):
                n += 1
        self.assertEqual(n, 1, "고정 임시 파일 이름 개수")
        self.assertIn("def pick_ctx(node):", SRC)

    def test_every_child_spawn_has_nostdin(self):
        spawns = [m.start() for m in re.finditer(r"subprocess\.(run|Popen)\(", SRC)]
        self.assertGreaterEqual(len(spawns), 5)
        for pos in spawns:
            seg = SRC[pos:pos + 400]
            seg = seg.split("\n\n")[0]
            end = seg.find("**NOWIN")
            self.assertGreater(end, 0, seg)
            self.assertIn("_nostdin()", seg[:end + 10], "자식 스폰에 표준입력 차단이 빠졌다: %s" % seg[:160])

    def test_asset_response_is_sent_outside_try(self):
        i = SRC.index("if path in self.routes:")
        blk = SRC[i:i + 2600]
        t = blk.index("try:")
        exc = blk.index("except OSError:")
        self.assertNotIn("self._send(200", blk[t:exc], "응답 전송이 try 안에 남았다(끊긴 응답 뒤에 404 덧붙임)")
        self.assertIn("return self._send(200, ctype, body, cache)", blk[exc:])


class LogAndHealth(unittest.TestCase):
    def test_log_line_head_time_and_thread(self):
        buf = io.StringIO()
        with contextlib.redirect_stderr(buf):
            t = threading.Thread(target=HB._log, args=("hello",), name="probe-thread")
            t.start()
            t.join()
        line = buf.getvalue()
        self.assertTrue(line.startswith("[hud-bridge] "), line)
        self.assertRegex(line, r"^\[hud-bridge\] \d{4}-\d\d-\d\dT\d\d:\d\d:\d\d \[probe-thread\] hello\n$")

    def test_log_never_raises_on_broken_stderr(self):
        class Broken(io.StringIO):
            def write(self, s):
                raise OSError("closed")
        with contextlib.redirect_stderr(Broken()):
            HB._log("x")

    def test_health_body_fields(self):
        with mock.patch.dict(os.environ, {"HUD_PACK_VERSION": "9.9.9"}):
            j = json.loads(HB.health_body())
        self.assertEqual(sorted(j), ["assets", "boot_id", "ok", "pack_version", "pid", "timeouts"])
        self.assertTrue(j["ok"])
        self.assertEqual(j["pid"], os.getpid())
        self.assertEqual(j["boot_id"], HB.BOOT_ID)
        self.assertEqual(j["pack_version"], "9.9.9")
        self.assertEqual(sorted(j["assets"]), ["office3d_html", "office_boot_js", "three_module_js"])
        self.assertTrue(all(isinstance(v, bool) for v in j["assets"].values()))
        env = {k: v for k, v in os.environ.items() if k != "HUD_PACK_VERSION"}
        with mock.patch.dict(os.environ, env, clear=True):
            self.assertIsNone(json.loads(HB.health_body())["pack_version"])

    def test_health_assets_reflect_missing_file(self):
        with mock.patch.object(HB, "WEB_DIR", "/nonexistent-web-dir-0144"):
            j = json.loads(HB.health_body())
        self.assertEqual(j["assets"], {"office3d_html": False, "office_boot_js": False, "three_module_js": False})

    def test_timeout_log_error_counts_and_is_silent(self):
        h = HB.Handler.__new__(HB.Handler)
        before = HB._STATS["timeouts"]
        buf = io.StringIO()
        with contextlib.redirect_stderr(buf):
            h.log_error("Request timed out: %r", TimeoutError("x"))
            h.log_error("other %s", "y")
        self.assertEqual(HB._STATS["timeouts"], before + 1)
        self.assertEqual(buf.getvalue(), "", "시간 제한 접속은 줄을 찍지 않는다")

    def test_handle_error_silences_timeout_and_broken_pipe_but_logs_real_errors(self):
        srv = HB._BridgeServer.__new__(HB._BridgeServer)
        before = HB._STATS["timeouts"]
        buf = io.StringIO()
        with contextlib.redirect_stderr(buf):
            for exc in (TimeoutError("w"), BrokenPipeError(), ConnectionResetError(), ConnectionAbortedError()):
                try:
                    raise exc
                except Exception:
                    srv.handle_error(None, ("127.0.0.1", 1))
        self.assertEqual(buf.getvalue(), "", "끊긴 쓰기·연결은 줄도 추적도 없다")
        self.assertEqual(HB._STATS["timeouts"], before + 1, "시간 제한(TimeoutError)만 계수")
        with contextlib.redirect_stderr(buf):
            try:
                raise ValueError("boom")
            except Exception:
                srv.handle_error(None, ("127.0.0.1", 1))
        self.assertIn("[hud-bridge] ", buf.getvalue())
        self.assertIn("요청 처리 예외", buf.getvalue())
        self.assertIn("ValueError", buf.getvalue(), "진짜 예외는 추적이 남는다")

    def test_server_bind_does_no_reverse_name_lookup(self):
        with mock.patch("socket.getfqdn", side_effect=AssertionError("getfqdn 호출 금지")):
            srv = HB._BridgeServer(("127.0.0.1", 0), HB.Handler)
            try:
                self.assertEqual(srv.server_name, "127.0.0.1")
                self.assertEqual(srv.server_port, srv.server_address[1])
            finally:
                srv.server_close()

    def test_lifeline_reads_without_buffer_lock(self):
        i = SRC.index("def _lifeline():")
        body = SRC[i:i + 600]
        self.assertIn("os.read(0, 1)", body)
        self.assertNotIn("sys.stdin.buffer", body)


if __name__ == "__main__":
    unittest.main(verbosity=2)
