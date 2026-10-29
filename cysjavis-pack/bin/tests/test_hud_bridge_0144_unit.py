#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_hud_bridge_0144_unit.py — 0.14.44 B4(브리지 스크립트 묶음) 단위 시험.

지키는 것
  ① 윈도우(와 sys.platform 이 cygwin/msys)에서는 B4 의 1·5·6(시간 제한 · 포트 먼저 · 신호/수명줄/자식 표준입력)이 종전대로 —
     `HUD_WIN_NEW=1` 일 때만 새 동작. 운영체제 이름을 바꿔치기해 판정한다.
  ② 스크립트 소스 고정: 신호·부모 번호로 부모를 확인하는 호출 0건 · 프로세스 그룹 신호 0건 · 콘솔 숨김 플래그 1건 ·
     고정 임시 파일 이름 1건 · `def pick_ctx(node):` 존재.
  ③ 로그 줄 머리(`[hud-bridge] `) 유지 + 시각·스레드 이름. ④ /health 본문의 칸. ⑤ 응답 전송은 try 밖(B4-2).
  ⑥ (0.14.56 X-8 CARRIER · DESIGN-r3 §1·§2 · PLAN contract 3) 레인별 스풀 리더 — T3 시작 게이트(첫 non-null fleet · 60 초
     fleet_unavailable 래치 · spool_checked) · T7 자기점검 자식의 바이트 모드 디코드 · T8 /health 가 main·_bridge·_fleet 를 늘 싣는다 ·
     골든 /health 픽스처(fixtures/office_spool_health_golden.json — Rust spool_watch 시험이 같은 파일을 include_str! 한다) ·
     leaf(Q3 (a) `-I` 에서 명시 경로 적재 · (c) leaf 부재 = leaf_missing 이고 HUD 는 산다) · 이력(첫 관측 EOF · 부재 레인 0 ·
     레인별 오프셋 · 보관 이름 rename · 삭제 0) · 귀속(파일의 레인 · 남의 레인 정식 키 버림 · 레인별 blocked).
     골든 갱신(설계가 바뀐 경우에만): python3 test_hud_bridge_0144_unit.py --write-golden
실행: python3 test_hud_bridge_0144_unit.py
"""
import ast
import contextlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
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
        with mock.patch.dict(os.environ, {"HUD_PACK_VERSION": "9.9.9"}), mock.patch.object(HB, "_SPOOL", None):
            j = json.loads(HB.health_body())
        self.assertEqual(sorted(j), ["assets", "boot_id", "ok", "pack_version", "pid", "spool", "spool_checked",
                                     "spool_schema", "subs", "timeouts"])   # (0.14.56) spool* 셋은 additive(PLAN contract 3)
        self.assertIsNone(j["subs"], "감독자가 없는 시험에서는 subs 가 null 이다")
        self.assertEqual((j["spool_schema"], j["spool_checked"], j["spool"]), (1, False, None),
                         "리더가 없는 시험에서는 spool 이 null 이다(main() 은 서버보다 먼저 리더를 건넨다)")
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


# ═══════════════════════════════════════════════════════════════════════════
# ⑥ 0.14.56 X-8 CARRIER — the per-lane spool reader (DESIGN-r3 §1·§2 · §7 T3 T7 T8 · PLAN contract 3)
#   Every rig lives in a temp dir: <tmp>/main is the bridge's own pack (lane main), dept packs sit beside it, the bridge
#   state dir is <tmp>/bridge-state. A rig pack's bin/javis_event.py is a stub that answers `spool-path` the way a
#   producer does, so these tests do not depend on the producer module. archive_fx is replaced (its module-level path
#   is the repo pack's state dir). Nothing here touches a live spool, a live fleet or the real `cys`.
# ═══════════════════════════════════════════════════════════════════════════
GOLDEN = os.path.join(SELF, "fixtures", "office_spool_health_golden.json")
ENTRY_KEYS = sorted(["lane", "socket", "state", "fault", "path", "bytes", "archives", "archive_bytes", "offset",
                     "foreign_key_dropped"])
T0 = time.time()   # records stamped T0 are fresh (inside BACKLOG_FX_SECS) for the whole run


def spool_of(pack):
    return os.path.join(pack, "state", "evt_spool.jsonl")


def stub_pack(pack, prints=None, rc=0, raw=None, sleep=0):
    """A rig pack whose bin/javis_event.py answers `spool-path` like a producer: it writes `raw` bytes (default: `prints`,
    else the leaf formula for this pack, plus a newline) to stdout and exits `rc`, after `sleep` seconds."""
    os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
    if raw is None:
        raw = ((spool_of(pack) if prints is None else prints) + "\n").encode("utf-8")
    with open(os.path.join(pack, "bin", "javis_event.py"), "w", encoding="utf-8") as f:
        f.write("import sys, time\n"
                "if sys.argv[1:] != ['spool-path']:\n"
                "    sys.exit(2)\n"
                "time.sleep(%r)\n"
                "sys.stdout.buffer.write(%r)\n"
                "sys.stdout.flush()\n"
                "sys.exit(%d)\n" % (sleep, raw, rc))
    return pack


def read_bytes(path):
    with open(path, "rb") as f:
        return f.read()


def put(path, *entries):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "ab") as f:
        for e in entries:
            f.write((json.dumps(e, ensure_ascii=False, sort_keys=True) + "\n").encode("utf-8"))


def fleet(*rows, base_surfaces=()):
    """A `cys fleet --json` body: the base row first (dept main, socket null, pack_dir null), then one row per
    (dept, socket, pack_dir, [(role, surface_ref, surface_id), ...])."""
    def surf(ss):
        return [{"role": r, "surface_ref": ref, "surface_id": sid} for (r, ref, sid) in ss]
    deps = [{"department": "본부 · CEO", "dept": "main", "socket": None, "pack_dir": None,
             "surfaces": surf(base_surfaces)}]
    for dept, sock, pack, ss in rows:
        deps.append({"department": dept, "dept": dept, "socket": sock, "pack_dir": pack, "surfaces": surf(ss)})
    return {"departments": deps}


class Clock:
    def __init__(self, t=5000.0):
        self.t = t

    def __call__(self):
        return self.t


class _Spool(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="hb-spool-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.state = os.path.join(self.tmp, "bridge-state")
        self.main = stub_pack(os.path.join(self.tmp, "main"))
        self.clock = Clock()
        self.world = HB.World()
        self.log = io.StringIO()
        self.fx = []
        p = mock.patch.object(HB, "archive_fx", lambda ts, fr: self.fx.append(fr))
        p.start()
        self.addCleanup(p.stop)
        self.addCleanup(setattr, HB, "_SPOOL", getattr(HB, "_SPOOL", None))

    def reader(self, **kw):
        kw.setdefault("root", self.main)
        kw.setdefault("state_dir", self.state)
        kw.setdefault("clock", self.clock)
        with contextlib.redirect_stderr(self.log):
            return HB.SpoolReader(self.world, HB.Hub(), HB.Coalescer(), **kw)

    def tick(self, rd, n=1):
        with contextlib.redirect_stderr(self.log):
            for _ in range(n):
                rd.tick()

    def merge(self, body):
        with contextlib.redirect_stderr(self.log):
            self.world.merge_fleet(body, None)

    def lane(self, rd, name):
        hits = [e for e in rd.health()[1] if e["lane"] == name]
        self.assertEqual(len(hits), 1, (name, rd.health()[1]))
        return hits[0]

    def offsets(self):
        try:
            return sorted(n for n in os.listdir(self.state) if n.endswith(".offset"))
        except OSError:
            return []

    def read_offset(self, lane):
        with open(os.path.join(self.state, "evt_spool-%s.offset" % lane)) as f:
            return json.load(f)["offset"]          # (slice H) the v2 offset record


class SpoolStartupGate(_Spool):
    """T3 (DESIGN-r3 §2): no lane is read before the first merge_fleet that carried a NON-NULL fleet; none within 60 s ->
    `_fleet` latches fleet_unavailable (fault); spool_checked = (first fleet + one full self-check pass over every lane it
    lists) or a terminal startup fault (leaf_missing, fleet_unavailable)."""

    def test_nothing_is_read_and_no_offset_moves_until_the_first_non_null_fleet(self):
        put(spool_of(self.main), {"ts": T0, "type": "task.blocked", "payload": {"task": "OLD", "blocked_by": ["x"]}})
        rd = self.reader()
        self.tick(rd)
        f = self.lane(rd, "_fleet")
        self.assertEqual((rd.health()[0], f["state"], f["fault"]), (False, "waiting", False))
        self.assertEqual(self.lane(rd, "main")["state"], "ok", "main is self-checked at start (its pack is ROOT)")
        self.merge(None)       # a failed `cys fleet --json`
        self.merge({})         # an empty answer is not a fleet either (merge_fleet keeps the previous world)
        self.assertEqual(self.world.fleet_gen, 0)
        self.clock.t += HB.SPOOL_FLEET_WAIT_SECS - 0.1
        self.tick(rd)
        self.assertEqual((rd.health()[0], self.lane(rd, "_fleet")["state"]), (False, "waiting"))
        self.clock.t += 0.1
        self.tick(rd)
        f = self.lane(rd, "_fleet")
        self.assertEqual((f["state"], f["fault"]), ("fleet_unavailable", True))
        self.assertTrue(rd.health()[0], "fleet_unavailable is a terminal startup fault - spool_checked latches")
        self.assertEqual(self.world.blocked, [], "no lane is read before the gate opens")
        self.assertEqual(self.offsets(), [], "no offset moves before the gate opens")
        self.assertIsNone(self.lane(rd, "main")["offset"])
        self.merge(fleet())    # the first non-null fleet opens the gate; first sight = EOF (no replay)
        self.tick(rd)
        f = self.lane(rd, "_fleet")
        self.assertEqual((rd.health()[0], f["state"], f["fault"]), (True, "ok", False))
        self.assertEqual(self.world.blocked, [])
        self.assertEqual(self.offsets(), ["evt_spool-main.offset"])
        self.assertEqual(self.read_offset("main"), os.path.getsize(spool_of(self.main)))
        put(spool_of(self.main), {"ts": T0, "type": "task.blocked", "payload": {"task": "NEW", "blocked_by": ["y"]}})
        self.tick(rd)
        self.assertEqual([b["task"] for b in self.world.blocked], ["NEW"])

    def test_spool_checked_waits_for_one_full_pass_over_every_lane_of_the_first_fleet(self):
        d5 = stub_pack(os.path.join(self.tmp, "d5"))
        rd = self.reader()
        self.tick(rd)
        self.assertFalse(rd.health()[0])
        self.merge(fleet(("dept-5", "/s/d5.sock", d5, []), ("dept-7", "/s/d7.sock", None, [])))
        calls, real = [], HB.spool_self_check
        with mock.patch.object(HB, "spool_self_check", side_effect=lambda *a, **k: (calls.append(a[0]), real(*a, **k))[1]):
            self.tick(rd)
        checked, entries = rd.health()
        self.assertTrue(checked)
        self.assertEqual(sorted(calls), sorted([self.main, d5]), "every lane of the first fleet is checked once (no pack -> no child)")
        self.assertEqual([(e["lane"], e["state"], e["fault"]) for e in entries],
                         [("main", "absent", False), ("dept-5", "absent", False), ("dept-7", "unknown_pack", True),
                          ("_bridge", "ok", False), ("_fleet", "ok", False)])

    def test_a_failed_fleet_call_after_a_good_one_keeps_the_lane_set(self):
        d5 = stub_pack(os.path.join(self.tmp, "d5"))
        rd = self.reader()
        self.merge(fleet(("dept-5", "/s/d5.sock", d5, [])))
        self.tick(rd)
        before = [(e["lane"], e["state"]) for e in rd.health()[1]]
        self.merge(None)
        self.clock.t += HB.SPOOL_FLEET_WAIT_SECS * 3
        self.tick(rd, 2)
        self.assertEqual([(e["lane"], e["state"]) for e in rd.health()[1]], before)

    def test_a_missing_leaf_latches_spool_checked_at_start(self):
        rd = self.reader(leaf_path=os.path.join(self.tmp, "no-leaf.py"))
        checked, entries = rd.health()
        self.assertTrue(checked, "leaf_missing is a terminal startup fault (K9)")
        self.assertEqual([(e["lane"], e["state"]) for e in entries],
                         [("main", "leaf_missing"), ("_bridge", "leaf_missing"), ("_fleet", "waiting")])


class SpoolSelfCheckDecode(_Spool):
    """T7 (DESIGN-r3 §1 · r2 gemini2-3): the self-check child is `python -I -B <P>/bin/javis_event.py spool-path` in BYTES
    mode, stdout decoded with .decode("utf-8", "surrogateescape").strip(); rc 2 = unverified; any other rc, a timeout or a
    child that does not run = unverified; another path = mismatch. No exception leaves."""

    def test_a_non_ascii_path_and_a_crlf_round_trip(self):
        pack = os.path.join(self.tmp, "팩-ü")
        stub_pack(pack, raw=(spool_of(pack) + "\r\n").encode("utf-8"))
        self.assertEqual(HB.spool_self_check(pack, spool_of(pack)), ("verified", None))

    def test_bytes_that_are_not_utf8_are_a_mismatch_and_never_raise(self):
        pack = os.path.join(self.tmp, "p")
        stub_pack(pack, raw=b"\xff\xfe" + spool_of(pack).encode("utf-8") + b"\n")
        check, why = HB.spool_self_check(pack, spool_of(pack))
        self.assertEqual(check, "mismatch")
        self.assertIn("\\udcff", why, "the undecodable byte is kept as a surrogate escape, not replaced or raised")

    def test_another_path_is_a_mismatch_and_a_lexically_equal_path_is_not(self):
        pack = stub_pack(os.path.join(self.tmp, "p"), prints="/elsewhere/state/evt_spool.jsonl")
        self.assertEqual(HB.spool_self_check(pack, spool_of(pack))[0], "mismatch")
        stub_pack(pack, prints=os.path.join(pack, "bin", "..", "state", "evt_spool.jsonl"))
        self.assertEqual(HB.spool_self_check(pack, spool_of(pack)), ("verified", None))

    def test_rc_2_other_rc_timeout_and_no_child_are_unverified(self):
        pack = os.path.join(self.tmp, "p")
        stub_pack(pack, rc=2)
        self.assertEqual(HB.spool_self_check(pack, spool_of(pack)),
                         ("unverified", "spool-path rc 2 (older pack without the subcommand)"))
        stub_pack(pack, rc=7)
        self.assertEqual(HB.spool_self_check(pack, spool_of(pack)), ("unverified", "spool-path rc 7"))
        stub_pack(pack, sleep=30)
        t = time.monotonic()
        self.assertEqual(HB.spool_self_check(pack, spool_of(pack), timeout=0.5), ("unverified", "spool-path timed out"))
        self.assertLess(time.monotonic() - t, 10.0)  # wallclock: guard spool_self_check returns after its 0.5 s timeout; the stub child sleeps 30 s
        self.assertEqual(HB.spool_self_check(pack, spool_of(pack), python=os.path.join(self.tmp, "no-python")),
                         ("unverified", "spool-path did not run (FileNotFoundError)"))

    def test_an_unverified_lane_is_read_with_the_formula_and_logged_once(self):
        d5 = stub_pack(os.path.join(self.tmp, "d5"), rc=2)
        rd = self.reader()
        self.merge(fleet(("dept-5", "/s/d5.sock", d5, [])))
        self.tick(rd)
        e = self.lane(rd, "dept-5")
        self.assertEqual((e["state"], e["fault"], e["path"], e["offset"]), ("unverified", False, spool_of(d5), 0))
        put(spool_of(d5), {"ts": T0, "type": "task.blocked", "payload": {"task": "U", "blocked_by": []}})
        for _ in range(3):
            self.clock.t += HB.SPOOL_RECHECK_SECS
            self.tick(rd)
        self.assertEqual([(b["lane"], b["task"]) for b in self.world.blocked], [("dept-5", "U")])
        self.assertEqual(self.log.getvalue().count("spool lane dept-5: unverified"), 1,
                         "one stderr line per change, not one per re-check")

    def test_the_call_is_bytes_mode_isolated_bounded_and_hidden(self):
        fn = next(n for n in ast.walk(ast.parse(SRC)) if isinstance(n, ast.FunctionDef) and n.name == "spool_self_check")
        calls = [n for n in ast.walk(fn) if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute)
                 and isinstance(n.func.value, ast.Name) and n.func.value.id == "subprocess"]
        self.assertEqual(len(calls), 1)
        kw = {k.arg for k in calls[0].keywords if k.arg}
        self.assertTrue(kw.isdisjoint({"text", "universal_newlines", "encoding", "errors"}), kw)
        self.assertTrue({"capture_output", "timeout"} <= kw, kw)
        self.assertIn("NOWIN", [k.value.id for k in calls[0].keywords if k.arg is None and isinstance(k.value, ast.Name)])
        self.assertEqual([e.value for e in calls[0].args[0].elts if isinstance(e, ast.Constant)], ["-I", "-B", "spool-path"])
        decodes = [n for n in ast.walk(fn) if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr == "decode"]
        self.assertEqual([[a.value for a in d.args] for d in decodes], [["utf-8", "surrogateescape"]])
        self.assertEqual(HB.SPOOL_CHECK_TIMEOUT_SECS, 5.0)


class SpoolHealthAlwaysLists(_Spool):
    """T8 (DESIGN-r3 §4-2 row 2): every /health body lists main, _bridge and _fleet with an explicit state, in every phase,
    and every entry carries every key of PLAN contract 3 with its type; `fault` is true exactly for the fault states."""

    def body(self, rd):
        HB._SPOOL = rd
        j = json.loads(HB.health_body())
        self.assertEqual(j["spool_schema"], 1)
        self.assertIsInstance(j["spool_checked"], bool)
        names = [e["lane"] for e in j["spool"]]
        self.assertIn("main", names)
        self.assertEqual(names[-2:], ["_bridge", "_fleet"])
        self.assertEqual((names.count("_bridge"), names.count("_fleet")), (1, 1))
        for e in j["spool"]:
            self.assertEqual(sorted(e), ENTRY_KEYS, e)
            self.assertIs(e["fault"], e["state"] in HB.SPOOL_FAULT_STATES, e)
            self.assertTrue(e["socket"] is None or isinstance(e["socket"], str), e)
            self.assertTrue(e["path"] is None or isinstance(e["path"], str), e)
            for k in ("bytes", "offset"):
                self.assertTrue(e[k] is None or (type(e[k]) is int and e[k] >= 0), e)
            for k in ("archives", "archive_bytes", "foreign_key_dropped"):
                self.assertTrue(type(e[k]) is int and e[k] >= 0, e)
        return j

    def states(self, j):
        return [(e["lane"], e["state"]) for e in j["spool"]]

    def test_every_phase_of_one_reader(self):
        rd = self.reader()
        self.assertEqual(self.states(self.body(rd)), [("main", "unverified"), ("_bridge", "ok"), ("_fleet", "waiting")])
        self.tick(rd)
        self.assertEqual(self.states(self.body(rd)), [("main", "absent"), ("_bridge", "ok"), ("_fleet", "waiting")])
        self.clock.t += HB.SPOOL_FLEET_WAIT_SECS
        self.tick(rd)
        self.assertEqual(self.states(self.body(rd))[-1], ("_fleet", "fleet_unavailable"))
        d5 = stub_pack(os.path.join(self.tmp, "d5"), prints="/other/state/evt_spool.jsonl")
        self.merge(fleet(("dept-5", "/s/d5.sock", d5, []), ("dept-6", "/s/d6.sock", os.path.join(self.tmp, "gone"), [])))
        self.tick(rd)
        self.assertEqual(self.states(self.body(rd)),
                         [("main", "absent"), ("dept-5", "mismatch"), ("dept-6", "pack_missing"), ("_bridge", "ok"), ("_fleet", "ok")])

    def test_a_missing_leaf_marks_every_listed_lane_and_bridge(self):
        rd = self.reader(leaf_path=os.path.join(self.tmp, "no-leaf.py"))
        self.merge(fleet(("dept-5", "/s/d5.sock", stub_pack(os.path.join(self.tmp, "d5")), [])))
        self.tick(rd)
        j = self.body(rd)
        self.assertTrue(j["spool_checked"])
        self.assertEqual([(e["lane"], e["state"], e["fault"], e["path"]) for e in j["spool"]],
                         [("main", "leaf_missing", True, None), ("dept-5", "leaf_missing", True, None),
                          ("_bridge", "leaf_missing", True, None), ("_fleet", "ok", False, None)])

    def test_over_cap_and_key_collisions_are_listed_faults_never_a_silent_skip(self):
        packs = [stub_pack(os.path.join(self.tmp, "d%d" % i)) for i in range(4)]
        rd = self.reader(cap=3)
        self.merge(fleet(("dept-1", "/s/1", packs[0], []), ("dept-2", "/s/2", packs[1], []), ("dept-3", "/s/3", packs[2], []),
                         ("main", "/s/m", packs[3], []), ("dept-9", "/s/9a", None, []), ("Dept.9", "/s/9b", None, [])))
        self.tick(rd)
        j = self.body(rd)
        self.assertEqual([(e["lane"], e["socket"], e["state"]) for e in j["spool"]],
                         [("main", None, "lane_key_collision"), ("dept-1", "/s/1", "absent"), ("dept-2", "/s/2", "absent"),
                          ("dept-3", "/s/3", "over_cap"), ("dept-9", "/s/9a", "lane_key_collision"),
                          ("dept-9", "/s/9b", "lane_key_collision"), ("main", "/s/m", "lane_key_collision"),
                          ("_bridge", None, "ok"), ("_fleet", None, "ok")])
        self.assertEqual(self.offsets(), ["evt_spool-dept-1.offset", "evt_spool-dept-2.offset"],
                         "a collided or over-cap lane is not read and gets no offset")

    def test_health_answers_when_the_spool_snapshot_raises(self):
        class Boom:
            def health(self):
                raise RuntimeError("model")
        HB._SPOOL = Boom()
        j = json.loads(HB.health_body())
        self.assertEqual((j["ok"], j["spool_schema"], j["spool_checked"], j["spool"]), (True, 1, False, None))

    def test_main_hands_the_reader_to_health_before_its_thread_starts(self):
        main = next(n for n in ast.parse(SRC).body if isinstance(n, ast.FunctionDef) and n.name == "main")
        assign = [n for n in ast.walk(main) if isinstance(n, ast.Assign)
                  and any(isinstance(t, ast.Name) and t.id == "_SPOOL" for t in n.targets)]
        self.assertEqual(len(assign), 1)
        self.assertTrue(any(isinstance(n, ast.Global) and "_SPOOL" in n.names for n in ast.walk(main)))
        starts = [n.lineno for n in ast.walk(main) if isinstance(n, ast.Call) and any(
            k.arg == "target" and isinstance(k.value, ast.Name) and k.value.id == "spool_loop" for k in n.keywords)]
        serve = [n.lineno for n in ast.walk(main) if isinstance(n, ast.Attribute) and n.attr == "serve_forever"]
        self.assertEqual(len(starts), 1)
        self.assertLess(assign[0].lineno, starts[0])
        self.assertLess(assign[0].lineno, serve[0])


def golden_spool_body():
    """The fixed world of fixtures/office_spool_health_golden.json, built through the real functions in a temp rig: main ok
    (one archive, one foreign key dropped), dept-5 mismatch (with its socket string), dept-7 unknown_pack, _bridge ok,
    _fleet ok. Machine independence: every path field has the rig directory replaced by "/RIG" and os.sep by "/";
    the clock is fixed and every record is old, so no field depends on time."""
    tmp = tempfile.mkdtemp(prefix="hb-golden-")
    saved = getattr(HB, "_SPOOL", None)
    try:
        main = stub_pack(os.path.join(tmp, "main"))
        d5 = stub_pack(os.path.join(tmp, "dept-5"), prints="/elsewhere/state/evt_spool.jsonl")
        put(spool_of(main), {"ts": 1.0, "type": "task_progress", "payload": {"task": "G", "stage": "s", "pct": 1}})
        with open(os.path.join(main, "state", "evt_spool.20261028T000000Z.jsonl"), "wb") as f:
            f.write(b'{"payload": {"task": "A"}, "ts": 0.5, "type": "task.unblocked"}\n')
        world = HB.World()
        with contextlib.redirect_stderr(io.StringIO()), mock.patch.object(HB, "archive_fx", lambda ts, fr: None):
            world.merge_fleet(fleet(("dept-5", "/Users/x/.local/state/cys-dept-dept-5/cys.sock", d5, []),
                                    ("dept-7", "/Users/x/.local/state/cys-dept-dept-7/cys.sock", None, [])), None)
            rd = HB.SpoolReader(world, HB.Hub(), HB.Coalescer(), root=main,
                                state_dir=os.path.join(tmp, "bridge-state"), clock=Clock())
            rd.tick()       # the first fleet: one full pass; main first sight = EOF
            put(spool_of(main), {"ts": 2.0, "type": "task.blocked", "key": "dept-5@surface:3",
                                 "payload": {"task": "F", "blocked_by": []}})
            rd.tick()       # a dept-5 key in main's file: dropped, counted
            HB._SPOOL = rd
            body = json.loads(HB.health_body())
    finally:
        HB._SPOOL = saved
        shutil.rmtree(tmp, True)
    out = {k: body[k] for k in ("spool_schema", "spool_checked", "spool")}
    for e in out["spool"]:
        if e["path"] is not None:
            e["path"] = e["path"].replace(tmp, "/RIG").replace(os.sep, "/")
    return out


GOLDEN_DOC = ("0.14.56 X-8 CARRIER golden /health spool fields (PLAN contract 3). Written by "
              "cysjavis-pack/bin/tests/test_hud_bridge_0144_unit.py --write-golden from the bridge's own health_body() over a "
              "fixed temp-rig world; \"/RIG\" stands for that rig directory. Python test: equality of spool_schema, "
              "spool_checked and spool. cysd spool_watch's Rust test include_str!s this file.")


class SpoolGoldenHealth(unittest.TestCase):
    """The cross-language /health contract (DESIGN-r3 §9 item 5): the bridge's health_body() for a fixed world equals the
    golden fixture that the Rust spool_watch test reads, so a drift on either side fails in CI, not at runtime."""

    def test_health_body_spool_fields_equal_the_golden_fixture(self):
        with open(GOLDEN, encoding="utf-8") as f:
            want = json.load(f)
        self.assertEqual(golden_spool_body(), {k: want[k] for k in ("spool_schema", "spool_checked", "spool")})

    def test_the_fixture_is_the_world_the_rust_side_expects(self):
        with open(GOLDEN, encoding="utf-8") as f:
            want = json.load(f)
        got = [(e["lane"], e["state"], e["fault"]) for e in want["spool"]]
        self.assertEqual(got, [("main", "ok", False), ("dept-5", "mismatch", True), ("dept-7", "unknown_pack", True),
                               ("_bridge", "ok", False), ("_fleet", "ok", False)])
        self.assertTrue(all(e["socket"] is None or isinstance(e["socket"], str) for e in want["spool"]))
        self.assertEqual([e["lane"] for e in want["spool"] if isinstance(e["socket"], str)], ["dept-5", "dept-7"])


class SpoolLeaf(_Spool):
    """ADDENDUM-1 Q3 specimens: (a) under `python -I` the bridge resolves the leaf by explicit path (a bare import does
    not resolve there - negative control); (c) a missing or broken leaf -> leaf_missing, no spool read, no exception."""

    def test_a_isolated_python_resolves_the_leaf_by_explicit_path(self):
        code = ("import runpy, sys; g = runpy.run_path(sys.argv[1], run_name='hb_probe'); "
                "m, err = g['_load_spool_leaf'](); print(err if m is None else m.spool_path_for_pack('/p'))")
        r = subprocess.run([sys.executable, "-I", "-B", "-c", code, HB.__file__], capture_output=True, text=True,
                           timeout=60, cwd=self.tmp)
        self.assertEqual((r.returncode, r.stdout.strip()), (0, os.path.join("/p", "state", "evt_spool.jsonl")), r.stderr[-800:])
        r2 = subprocess.run([sys.executable, "-I", "-B", "-c", "import javis_spool_path"], capture_output=True, text=True,
                            timeout=60, cwd=BIN)
        self.assertNotEqual(r2.returncode, 0, "negative control: a bare import must not resolve under -I")
        self.assertIn("ModuleNotFoundError", r2.stderr)

    def test_c_a_missing_or_broken_leaf_is_leaf_missing_and_nothing_is_read(self):
        put(spool_of(self.main), {"ts": T0, "type": "task.blocked", "payload": {"task": "L", "blocked_by": []}})
        d5 = stub_pack(os.path.join(self.tmp, "d5"))
        for name, text in (("absent", None), ("syntax", "def spool_path_for_pack(:\n"), ("nofunc", "X = 1\n")):
            with self.subTest(name):
                leaf = os.path.join(self.tmp, "leaf-%s.py" % name)
                if text is not None:
                    with open(leaf, "w") as f:
                        f.write(text)
                rd = self.reader(leaf_path=leaf)
                self.merge(fleet(("dept-5", "/s/d5.sock", d5, [])))
                self.tick(rd, 3)
                checked, entries = rd.health()
                self.assertTrue(checked)
                self.assertEqual([(e["lane"], e["state"], e["path"]) for e in entries if e["lane"] != "_fleet"],
                                 [("main", "leaf_missing", None), ("dept-5", "leaf_missing", None), ("_bridge", "leaf_missing", None)])
                self.assertEqual((self.world.blocked, self.offsets()), ([], []))
        for err in ("FileNotFoundError", "SyntaxError", "AttributeError"):
            self.assertIn("spool leaf unavailable: %s" % err, self.log.getvalue())
        self.assertNotIn("Traceback", self.log.getvalue())

    def test_a_leaf_that_appears_later_is_loaded_on_the_recheck_and_the_fault_clears(self):
        leaf = os.path.join(self.tmp, "late-leaf.py")
        rd = self.reader(leaf_path=leaf)
        self.merge(fleet())
        self.tick(rd)
        shutil.copy(HB.SPOOL_LEAF_PATH, leaf)
        self.clock.t += HB.SPOOL_RECHECK_SECS - 1
        self.tick(rd)
        self.assertEqual(self.lane(rd, "_bridge")["state"], "leaf_missing", "a fault clears on a check, never by itself")
        self.clock.t += 1
        self.tick(rd)
        self.assertEqual((self.lane(rd, "_bridge")["state"], self.lane(rd, "main")["state"]), ("ok", "absent"))


class SpoolHistory(_Spool):
    """History (DESIGN-r2 3-3 · CEO E5 invariants): None for a missing offset; first sight persists EOF (no replay) or 0
    (an absent lane's first batch is read); offsets per lane; the pre-0.14.56 evt_spool.offset is never touched."""

    def test_load_offset_is_none_when_missing_empty_unreadable_or_negative(self):
        """(slice H) The v2 record round-trips; anything else - missing, empty, garbage, a plain integer (e59aae92's format),
        negative, an offset without the identity, a non-finite last_ts, a directory - is None = first sight (no replay)."""
        p = os.path.join(self.tmp, "o")
        self.assertIsNone(HB._load_offset_record(p))
        good = {"offset": 12, "head": b'{"ts": 1}\n', "tail": b'{"ts": 2}\n', "prev_head": None, "last_ts": 2.5}
        self.assertTrue(HB._save_offset_record(p, good))
        self.assertEqual(HB._load_offset_record(p), good)

        def rec(**kw):
            d = {"v": 2, "offset": 12, "head": "eyJ0cyI6IDF9Cg==", "tail": "", "prev_head": None, "last_ts": None}
            d.update(kw)
            return json.dumps(d)
        for text in ("", "garbage", "12\n", "0", rec(offset=-4), rec(head=""), rec(v=1), rec(last_ts=float("nan")),
                     rec(head="!!not base64!!"), rec(offset=True)):
            with open(p, "w") as f:
                f.write(text)
            self.assertIsNone(HB._load_offset_record(p), repr(text))
        os.makedirs(p + ".d")
        self.assertIsNone(HB._load_offset_record(p + ".d"))

    def test_an_absent_lane_starts_at_0_and_its_first_batch_is_read(self):
        d5 = stub_pack(os.path.join(self.tmp, "d5"))
        rd = self.reader()
        self.merge(fleet(("dept-5", "/s/d5.sock", d5, [])))
        self.tick(rd)
        self.assertEqual((self.lane(rd, "dept-5")["state"], self.read_offset("dept-5")), ("absent", 0))
        put(spool_of(d5), {"ts": T0, "type": "task.blocked", "payload": {"task": "FIRST", "blocked_by": []}})
        self.tick(rd)
        self.assertEqual([(b["lane"], b["task"]) for b in self.world.blocked], [("dept-5", "FIRST")])
        self.assertEqual(self.lane(rd, "dept-5")["state"], "ok")
        self.assertEqual(self.read_offset("dept-5"), os.path.getsize(spool_of(d5)))

    def test_a_restart_resumes_from_the_persisted_offset_and_a_bad_offset_is_first_sight(self):
        put(spool_of(self.main), {"ts": T0, "type": "task.blocked", "payload": {"task": "A", "blocked_by": []}})
        self.merge(fleet())
        self.tick(self.reader())
        self.assertEqual(self.world.blocked, [], "first sight is EOF: A is not replayed")
        put(spool_of(self.main), {"ts": T0, "type": "task.blocked", "payload": {"task": "B", "blocked_by": []}})
        self.tick(self.reader())          # a restarted bridge: the persisted offset, not first sight again
        self.assertEqual([b["task"] for b in self.world.blocked], ["B"])
        with open(os.path.join(self.state, "evt_spool-main.offset"), "w") as f:
            f.write("not a number")
        self.world.blocked = []
        self.tick(self.reader())
        self.assertEqual(self.world.blocked, [], "an unreadable offset is first sight (EOF), never a replay from 0")
        self.assertEqual(self.read_offset("main"), os.path.getsize(spool_of(self.main)))

    def test_offsets_are_per_lane_and_the_pre_0_14_56_offset_is_left_untouched(self):
        os.makedirs(self.state)
        with open(os.path.join(self.state, "evt_spool.offset"), "w") as f:
            f.write("777")
        rd = self.reader()
        self.merge(fleet(("dept-5", "/s/d5.sock", stub_pack(os.path.join(self.tmp, "d5")), [])))
        self.tick(rd, 2)
        self.assertEqual(self.offsets(), ["evt_spool-dept-5.offset", "evt_spool-main.offset", "evt_spool.offset"])
        with open(os.path.join(self.state, "evt_spool.offset")) as f:
            self.assertEqual(f.read(), "777")

    def test_the_reader_routes_the_leftover_before_it_keeps_offset_0_and_deletes_nothing(self):
        put(spool_of(self.main), *({"ts": 1.0, "type": "task.unblocked", "payload": {"task": "o%d" % i}} for i in range(3)))
        original = read_bytes(spool_of(self.main))
        late = {"ts": T0, "type": "task.blocked", "payload": {"task": "LATE", "blocked_by": []}}
        real_rotate, real_save, saves = HB.maybe_rotate_spool, HB._save_offset_record, []

        def landing(path, off, now=None):     # a producer append lands between the last tail and the rename
            put(path, late)
            return real_rotate(path, off, now)

        def save(p, ident):
            saves.append((os.path.basename(p), ident["offset"], [b["task"] for b in self.world.blocked]))
            return real_save(p, ident)
        rd = self.reader()
        self.merge(fleet())
        self.tick(rd)                         # first sight: EOF (below the default threshold - nothing rotates yet)
        self.clock.t += HB.ROTATE_START_GRACE_SECS + 1   # (slice H) past the start grace (F1-C G-a)
        with mock.patch.object(HB, "SPOOL_ROTATE_BYTES", 10), mock.patch.object(HB, "maybe_rotate_spool", landing), \
                mock.patch.object(HB, "_save_offset_record", save):
            self.tick(rd)                     # drained and above the (patched) threshold: rotate
        self.assertEqual(saves, [("evt_spool-main.offset", 0, ["LATE"])], "the leftover is routed BEFORE offset 0 is kept")
        names = os.listdir(os.path.join(self.main, "state"))
        self.assertEqual(len(names), 1, names)
        self.assertRegex(names[0], r"^evt_spool\.\d{8}T\d{6}Z\.jsonl$")
        with open(os.path.join(self.main, "state", names[0]), "rb") as f:
            self.assertEqual(f.read(), original + (json.dumps(late, ensure_ascii=False, sort_keys=True) + "\n").encode())
        e = self.lane(rd, "main")
        self.assertEqual((e["state"], e["bytes"], e["archives"], e["archive_bytes"], e["offset"]),
                         ("absent", None, 1, len(original) + len(json.dumps(late, sort_keys=True)) + 1, 0))


class SpoolArchive(unittest.TestCase):
    """Rotation (ADDENDUM-1 Q5 · DESIGN-r2 3-3): rename to evt_spool.<UTC>.jsonl in the lane's state dir, never over an
    existing file (-2 .. -99, then skip), any other OSError -> skip and retry next drain; nothing is ever deleted."""
    NOW = 1761699723.0

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="hb-arch-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.path = spool_of(self.tmp)
        put(self.path, {"ts": 1.0, "type": "task.unblocked", "payload": {"task": "a"}})
        self.data = read_bytes(self.path)
        self.name = "evt_spool.%s.jsonl" % time.strftime("%Y%m%dT%H%M%SZ", time.gmtime(self.NOW))
        p = mock.patch.object(HB, "SPOOL_ROTATE_BYTES", 5)
        p.start()
        self.addCleanup(p.stop)

    def files(self):
        out = {}
        for n in sorted(os.listdir(os.path.dirname(self.path))):
            with open(os.path.join(os.path.dirname(self.path), n), "rb") as f:
                out[n] = f.read()
        return out

    def test_rotation_renames_to_a_utc_archive_and_keeps_every_byte(self):
        self.assertEqual(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW), (True, [], 0))
        self.assertEqual(self.files(), {self.name: self.data})
        self.assertEqual(HB.spool_archives(os.path.dirname(self.path)), (1, len(self.data)))

    def test_a_taken_name_gets_a_suffix_and_a_full_set_is_skipped(self):
        d = os.path.dirname(self.path)
        with open(os.path.join(d, self.name), "wb") as f:
            f.write(b"older archive\n")
        self.assertTrue(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW)[0])
        self.assertEqual(self.files(), {self.name: b"older archive\n", self.name[:-6] + "-2.jsonl": self.data})
        put(self.path, {"ts": 2.0, "type": "task.unblocked", "payload": {"task": "b"}})
        live = read_bytes(self.path)
        for n in range(3, 100):
            with open(os.path.join(d, self.name[:-6] + "-%d.jsonl" % n), "wb") as f:
                f.write(b"x%d\n" % n)
        before = self.files()
        self.assertEqual(HB.maybe_rotate_spool(self.path, len(live), now=self.NOW), (False, [], len(live)))
        self.assertEqual(self.files(), before, "every name taken: skipped, nothing overwritten or removed")
        self.assertEqual(HB.spool_archives(d)[0], 99)

    def test_a_failed_link_is_skipped_and_the_next_drain_retries(self):
        with mock.patch.object(HB.os, "link", side_effect=PermissionError("no hard links here")):
            self.assertEqual(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW), (False, [], len(self.data)))
        self.assertEqual(self.files(), {"evt_spool.jsonl": self.data})
        self.assertTrue(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW)[0])
        self.assertEqual(self.files(), {self.name: self.data})

    @unittest.skipIf(os.name == "nt", "the link + unlink pair is the POSIX branch")
    def test_a_failed_unlink_takes_back_only_the_name_it_just_made(self):
        real = os.unlink

        def unlink(p, *a, **k):
            if p == self.path:
                raise PermissionError("busy")
            return real(p, *a, **k)
        with mock.patch.object(HB.os, "unlink", side_effect=unlink):
            self.assertFalse(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW)[0])
        self.assertEqual(self.files(), {"evt_spool.jsonl": self.data})

    # -- head ruling 2026-10-29 (rotation ACCEPT, conditions 1-5): G1 (4b) G2 (5) G3 (3b) - RED at 3cd23f35, GREEN after --

    @unittest.skipIf(os.name == "nt", "the link + unlink pair is the POSIX branch")
    def test_g1_an_unlink_that_removed_src_but_raised_never_costs_the_last_name(self):
        """4b - a cleanup may not become a data loss (E5): an unlink(src) that removed src and still raised (a remove retried
        by a network filesystem) must not make the take-back remove the ONLY remaining name. The rename is then complete."""
        real = os.unlink

        def unlink(p, *a, **k):
            if p == self.path:
                real(p)
                raise FileNotFoundError(2, "removed on the server, error on the client")
            return real(p, *a, **k)
        with mock.patch.object(HB.os, "unlink", side_effect=unlink):
            rotated = HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW)
        self.assertEqual(self.files(), {self.name: self.data}, "the bytes survive under exactly one name, the archive")
        self.assertEqual(rotated, (True, [], 0), "src is gone and dst holds the bytes: the rename is complete")

    @unittest.skipIf(os.name == "nt", "the link + unlink pair is the POSIX branch")
    def test_g1_a_producer_new_src_after_a_misreported_unlink_keeps_the_archive(self):
        """4b: unlink(src) removed src and raised, and a producer had already created a NEW src (O_CREAT append). src and dst
        are then different files - dst holds the old bytes and is their only name: keep it, the rename is complete."""
        real = os.unlink
        fresh = b'{"ts": 3.0, "type": "task.unblocked", "payload": {"task": "new"}}\n'

        def unlink(p, *a, **k):
            if p == self.path:
                real(p)
                with open(p, "wb") as f:
                    f.write(fresh)
                raise FileNotFoundError(2, "removed on the server, error on the client")
            return real(p, *a, **k)
        with mock.patch.object(HB.os, "unlink", side_effect=unlink):
            self.assertEqual(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW), (True, [], 0))
        self.assertEqual(self.files(), {self.name: self.data, "evt_spool.jsonl": fresh})

    @unittest.skipIf(os.name == "nt", "the link + unlink pair is the POSIX branch")
    def test_g1_an_unknown_state_skips_and_never_resets_the_offset(self):
        """4a + 4b (reflection R1 on G1): unlink(src) failed AND src cannot even be compared - the state is unknown. Treating
        it as a finished rename would reset the offset to 0 on a src that still holds the old bytes (re-read as new events).
        Skip instead: both names kept (one inode), offset unchanged, nothing removed."""
        real = os.unlink

        def unlink(p, *a, **k):
            if p == self.path:
                raise PermissionError("busy")
            return real(p, *a, **k)
        with mock.patch.object(HB.os, "unlink", side_effect=unlink), \
                mock.patch.object(HB.os.path, "samefile", side_effect=PermissionError("stat refused")):
            self.assertEqual(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW), (False, [], len(self.data)))
        self.assertEqual(self.files(), {"evt_spool.jsonl": self.data, self.name: self.data})

    def test_g2_every_rotation_writes_one_readable_log_line(self):
        """5 - the operator can read every rotation: archive name, bytes, the spool's directory, nothing deleted."""
        lines = []
        with mock.patch.object(HB, "_log", side_effect=lines.append):
            self.assertTrue(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW)[0])
        self.assertEqual(len(lines), 1, lines)
        for part in (self.name, "%d B" % len(self.data), os.path.dirname(self.path), "nothing deleted"):
            self.assertIn(part, lines[0])

    def test_g2_a_skipped_rotation_is_logged_once_per_reason(self):
        """5 - a skipped rotation is readable too, once per (spool, reason) - a drain every second must not flood the log."""
        lines = []
        with mock.patch.object(HB, "_log", side_effect=lines.append), \
                mock.patch.object(HB.os, "link", side_effect=PermissionError("no hard links here")):
            for _ in range(3):
                self.assertFalse(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW)[0])
        self.assertEqual(len(lines), 1, lines)
        self.assertIn("skipped", lines[0])
        self.assertIn("PermissionError", lines[0])
        self.assertEqual(self.files(), {"evt_spool.jsonl": self.data})

    def test_g2_every_name_taken_is_a_readable_skip(self):
        """5 - the other skip reason (all 99 names of this second taken) is logged too, once."""
        d = os.path.dirname(self.path)
        for n in range(1, 100):
            with open(os.path.join(d, self.name if n == 1 else self.name[:-6] + "-%d.jsonl" % n), "wb") as f:
                f.write(b"x\n")
        lines = []
        with mock.patch.object(HB, "_log", side_effect=lines.append):
            for _ in range(2):
                self.assertFalse(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW)[0])
        self.assertEqual(len(lines), 1, lines)
        self.assertIn("taken", lines[0])

    @unittest.skipIf(os.name == "nt", "a second hard link is the POSIX crash shape")
    def test_g3_two_names_of_one_inode_are_one_archive(self):
        """3b - a crash between link and unlink leaves two names of ONE inode; the next drain renames again. /health must
        count that as one archive of its real size, not two."""
        d = os.path.dirname(self.path)
        os.link(self.path, os.path.join(d, "evt_spool.20200101T000000Z.jsonl"))
        self.assertTrue(HB.maybe_rotate_spool(self.path, len(self.data), now=self.NOW)[0])
        self.assertEqual(HB.spool_archives(d), (1, len(self.data)))

    def test_windows_uses_a_plain_rename(self):
        calls = []
        with mock.patch.object(HB.os, "name", "nt"), mock.patch.object(HB.os, "rename", side_effect=lambda s, d: calls.append((s, d))), \
                mock.patch.object(HB.os, "link", side_effect=AssertionError("no link on Windows")):
            HB._rename_no_overwrite("a", "b")
        self.assertEqual(calls, [("a", "b")])

    def test_the_bridge_moves_or_removes_files_only_in_the_rename_and_its_own_heat_file(self):
        found = {}
        for fn in ast.walk(ast.parse(SRC)):
            if not isinstance(fn, ast.FunctionDef):
                continue
            for n in ast.walk(fn):
                if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and isinstance(n.func.value, ast.Name) \
                        and n.func.value.id in ("os", "shutil") and n.func.attr in (
                            "remove", "unlink", "replace", "rename", "renames", "rmdir", "rmtree", "move", "truncate", "ftruncate"):
                    found.setdefault(fn.name, []).append(n.func.attr)
        self.assertEqual({k: sorted(v) for k, v in found.items()},
                         {"_rename_no_overwrite": ["rename", "unlink", "unlink"], "persist_heat": ["replace"],
                          "_save_offset_record": ["replace", "unlink", "unlink"]},
                         "pin: os.replace + os.remove(.old) in maybe_rotate_spool deleted a spool; now only the non-overwriting "
                         "rename (link+unlink, its take-back, Windows rename), presence_heat.json's own atomic replace and "
                         "(slice H) the offset record's own tmp + replace - `<record>.tmp-<pid>` never matches evt_spool*.jsonl, "
                         "even when HUD_STATE_DIR defaults to the spool's own directory")


class SpoolAttribution(_Spool):
    """Attribution (DESIGN-r3 §2 · DESIGN 3(C)): a record's lane is the lane of its file; an explicit key of another lane is
    DROPPED and counted; bare keys and role_key are scoped to the lane; world.blocked acts per (lane, task)."""
    BASE = [("master", "surface:1", 1), ("worker", "surface:2", 2)]
    D5 = [("master", "surface:1", 1), ("worker", "surface:3", 3)]

    def setUp(self):
        super().setUp()
        self.d5 = stub_pack(os.path.join(self.tmp, "d5"))
        self.rd = self.reader()
        self.merge(fleet(("dept-5", "/s/d5.sock", self.d5, self.D5), base_surfaces=self.BASE))
        self.tick(self.rd)      # both spools absent: first sight = 0

    def test_a_role_shared_by_two_lanes_goes_to_the_lane_of_the_file(self):
        rec = {"ts": T0, "type": "run.started", "payload": {"agent": "worker", "task": "W"}}
        put(spool_of(self.d5), rec)
        self.tick(self.rd)
        self.assertEqual(list(self.world.run), ["dept-5@surface:3"])
        put(spool_of(self.main), rec)          # negative control: the same line in main's file is main's worker
        self.tick(self.rd)
        self.assertEqual(sorted(self.world.run), ["dept-5@surface:3", "main@surface:2"])
        self.assertIsNone(self.world.role_key("worker"), "the whole-fleet ladder cannot tell the two workers apart")

    def test_an_explicit_key_of_another_lane_is_dropped_and_counted(self):
        put(spool_of(self.d5), {"ts": T0, "type": "task.blocked", "key": "main@surface:2", "payload": {"task": "X", "blocked_by": []}},
            {"ts": T0, "type": "task_progress", "key": "MAIN@surface:2", "payload": {"task": "X", "stage": "s", "pct": 5}},
            {"ts": T0, "type": "run.started", "key": "ghost@surface:99", "payload": {"agent": "worker", "task": "X"}})
        self.tick(self.rd)
        self.assertEqual((self.world.blocked, self.world.progress, self.world.run, self.fx), ([], {}, {}, []))
        self.assertEqual((self.lane(self.rd, "dept-5")["foreign_key_dropped"], self.lane(self.rd, "main")["foreign_key_dropped"]), (3, 0))
        put(spool_of(self.main), {"ts": T0, "type": "task.blocked", "key": "main@surface:2", "payload": {"task": "X", "blocked_by": []}})
        self.tick(self.rd)
        self.assertEqual([(b["lane"], b["key"]) for b in self.world.blocked], [("main", "main@surface:2")])

    def test_a_bare_key_and_an_unknown_same_lane_key_stay_inside_the_lane(self):
        put(spool_of(self.d5), {"ts": T0, "type": "task_progress", "key": "surface:1", "payload": {"task": "P1", "stage": "s", "pct": 1}},
            {"ts": T0, "type": "task_progress", "key": "dept-5@surface:99", "payload": {"agent": "worker", "task": "P2", "stage": "s", "pct": 1}})
        self.tick(self.rd)
        self.assertEqual({k: v["task"] for k, v in self.world.progress.items()},
                         {"dept-5@surface:1": "P1", "dept-5@surface:3": "P2"})
        self.assertIsNone(self.world.unique_full_key_for_ref("surface:1"), "fleet-wide, surface:1 is ambiguous")

    def test_blocked_and_unblocked_act_on_the_same_lane_and_task_only(self):
        blk = {"ts": T0, "type": "task.blocked", "payload": {"task": "T", "blocked_by": ["d"]}}
        put(spool_of(self.main), blk)
        put(spool_of(self.d5), blk, {"ts": T0, "type": "task.blocked", "payload": {"task": "T", "blocked_by": ["d", "e"]}})
        self.tick(self.rd)
        self.assertEqual(sorted((b["lane"], tuple(b["blocked_by"])) for b in self.world.blocked),
                         [("dept-5", ("d", "e")), ("main", ("d",))])
        put(spool_of(self.d5), {"ts": T0, "type": "task.unblocked", "payload": {"task": "T"}})
        self.tick(self.rd)
        self.assertEqual([b["lane"] for b in self.world.blocked], ["main"])

    def test_pack_dir_never_reaches_the_office_page(self):
        body = fleet(("dept-5", "/s/d5.sock", self.d5, self.D5), base_surfaces=self.BASE)
        body["departments"][1]["surfaces"][0]["status"] = {"task": "changed", "state": "working", "age_secs": 1}
        with contextlib.redirect_stderr(self.log):
            patches, _ = self.world.merge_fleet(body, None)
        for frame in (self.world.snapshot(), patches):
            text = json.dumps(frame, ensure_ascii=False)
            self.assertNotIn(self.d5, text)
            self.assertNotIn("pack_dir", text)


def _write_golden():
    body = golden_spool_body()
    with open(GOLDEN, "w", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(dict([("_doc", GOLDEN_DOC)] + list(body.items())), ensure_ascii=False, indent=1) + "\n")
    print("wrote %s" % GOLDEN)


class _SpoolH(_Spool):
    """(0.14.56 slice H · CEO E5 "never re-read as new", head ruling E5-F1-F2 78cd2297 + GO amendments) - every routed task is
    recorded over the REAL route_spool, so a replay shows up as a task routed twice, and a skipped new record as a task missing."""

    def setUp(self):
        super().setUp()
        self.spool = spool_of(self.main)
        self.routed = []
        real = HB.route_spool

        def record(entry, *a, **kw):
            self.routed.append((entry.get("payload") or {}).get("task"))
            return real(entry, *a, **kw)
        p = mock.patch.object(HB, "route_spool", record)
        p.start()
        self.addCleanup(p.stop)

    @staticmethod
    def ev(task, k):
        return {"ts": T0 + k, "type": "task.unblocked", "payload": {"task": task}}

    def first_sight(self, *tasks):
        """Old records A.. are on disk; the reader starts (first sight = EOF, nothing replayed) and returns."""
        put(self.spool, *(self.ev(t, i) for i, t in enumerate(tasks)))
        rd = self.reader()
        self.merge(fleet())
        self.tick(rd)
        self.assertEqual(self.routed, [], "first sight is EOF")
        return rd

    def consume(self, rd, *pairs):
        put(self.spool, *(self.ev(t, k) for t, k in pairs))
        self.tick(rd)
        self.assertEqual(self.routed, [t for t, _ in pairs])
        del self.routed[:]

    def past_grace(self):
        self.clock.t += getattr(HB, "ROTATE_START_GRACE_SECS", 0) + 1

    def archives(self):
        return sorted(n for n in os.listdir(os.path.dirname(self.spool)) if HB._SPOOL_ARCHIVE_RE.match(n))


class SpoolIdentityH(_SpoolH):
    """F2 routes (a)-(d), rows 2/6/7 of DESIGN-NOTE-H-F2-F1C 012b8d62 and the legacy record. At e59aae92 each of these re-reads
    (or misreads) a file at the live name, because tail_spool turns an absent/short file into offset 0 and _drain saves it."""

    def test_h_a_an_absent_moment_then_the_same_file_is_not_replayed(self):
        rd = self.first_sight("A", "B")
        self.consume(rd, ("C", 10))
        os.rename(self.spool, self.spool + ".away")      # the swap's gap between its two renames: nothing at the live name
        self.tick(rd)
        os.rename(self.spool + ".away", self.spool)
        put(self.spool, self.ev("D", 20))
        self.tick(rd)
        self.assertEqual(self.routed, ["D"], "the same file came back: only the new record is routed")

    @unittest.skipUnless(os.name == "posix" and os.geteuid() != 0, "needs a directory a non-root user cannot search")
    def test_h_b_a_transient_stat_error_holds_the_offset(self):
        rd = self.first_sight("A", "B")
        self.consume(rd, ("C", 10))
        d = os.path.dirname(self.spool)
        os.chmod(d, 0)
        try:
            self.tick(rd)
        finally:
            os.chmod(d, 0o755)
        put(self.spool, self.ev("D", 20))
        self.tick(rd)
        self.assertEqual(self.routed, ["D"])

    def test_h_c_an_install_copy_shorter_than_the_offset_is_not_replayed_and_a_new_record_counts_once(self):
        rd = self.first_sight("A", "B")
        copy = read_bytes(self.spool)                     # init-pack's copy_dir_all took the spool here
        self.consume(rd, ("C", 10), ("E", 30))            # the bridge kept reading the OLD tree inside the window
        with open(self.spool + ".new", "wb") as f:
            f.write(copy)
        os.replace(self.spool + ".new", self.spool)       # the swap: the copy (new inode) is the live file now
        put(self.spool, self.ev("F", 40))                 # a producer appends before the bridge's next poll
        self.tick(rd)
        self.assertEqual(self.routed, ["F"], "A/B are not replayed and F, appended after the swap, is routed once")
        self.assertIn("older copy", self.log.getvalue())

    def test_h_d_a_restore_that_rewinds_the_file_in_place_is_not_replayed(self):
        rd = self.first_sight("A", "B")
        copy = read_bytes(self.spool)
        self.consume(rd, ("C", 10), ("E", 30))
        with open(self.spool, "r+b") as f:                # tar restore: same inode, older bytes
            f.truncate(0)
            f.write(copy)
        put(self.spool, self.ev("F", 40))
        self.tick(rd)
        self.assertEqual(self.routed, ["F"])

    def test_h_residual_an_earlier_ts_appended_after_a_swap_is_skipped_not_replayed_and_counted(self):
        """Head add (b): the named residual of the ts filter - a record whose ts is not newer than the newest already read
        (here earlier; == falls on the same side) lands after the swap. It is skipped from view, never replayed, and the
        one resync line counts it (head add (a))."""
        rd = self.first_sight("A", "B")
        copy = read_bytes(self.spool)
        self.consume(rd, ("C", 10), ("E", 30))
        with open(self.spool + ".new", "wb") as f:
            f.write(copy)
        os.replace(self.spool + ".new", self.spool)
        put(self.spool, self.ev("EARLY", 20), self.ev("SAME", 30), self.ev("F", 40))
        self.tick(rd)
        self.assertEqual(self.routed, ["F"], "A/B not replayed; EARLY (ts 20) and SAME (ts == 30) skipped; F routed once")
        lines = [ln for ln in self.log.getvalue().splitlines() if "routed 1 newer record(s), skipped 4" in ln]
        self.assertEqual(len(lines), 1, self.log.getvalue())

    def test_h_c2_a_copy_plus_appends_longer_than_the_offset_is_not_misread_at_the_old_offset(self):
        """The copy grew past the old offset before the poll: byte N is no longer where the reader stopped (tail anchor)."""
        rd = self.first_sight("A", "B")
        copy = read_bytes(self.spool)
        self.consume(rd, ("C", 10), ("E", 30))
        with open(self.spool + ".new", "wb") as f:
            f.write(copy)
        os.replace(self.spool + ".new", self.spool)
        put(self.spool, *(self.ev("F%d" % i, 40 + i) for i in range(5)))   # now longer than the old offset
        self.assertGreater(os.path.getsize(self.spool), self.lane(rd, "main")["offset"])
        self.tick(rd)
        self.assertEqual(self.routed, ["F%d" % i for i in range(5)])

    @unittest.skipUnless(hasattr(os, "mkfifo"), "needs a FIFO")
    def test_h_r1_1_a_fifo_at_the_live_name_never_blocks_the_reader(self):
        """R1-1: a plain open() of a FIFO blocks until a writer appears - the spool thread must hold the lane instead."""
        rd = self.first_sight("A")
        self.consume(rd, ("B", 1))
        off = self.lane(rd, "main")["offset"]
        os.rename(self.spool, self.spool + ".away")
        os.mkfifo(self.spool)
        t = threading.Thread(target=self.tick, args=(rd,), daemon=True)
        t.start()
        t.join(5)
        alive = t.is_alive()
        if alive:                                         # unblock the stuck open so the daemon thread can end
            fd = os.open(self.spool, os.O_WRONLY | os.O_NONBLOCK)
            os.close(fd)
        self.assertFalse(alive, "the reader blocked on a FIFO at the live name")
        self.assertEqual(self.lane(rd, "main")["offset"], off, "held: the offset is kept")
        os.remove(self.spool)
        os.rename(self.spool + ".away", self.spool)
        put(self.spool, self.ev("C", 2))
        self.tick(rd)
        self.assertEqual(self.routed, ["C"])

    def test_h_first_sight_counts_the_skipped_records_as_consumed(self):
        """First sight skips to EOF without routing: a rewind right after it must still treat A/B as consumed (last_ts)."""
        rd = self.first_sight("A", "B")
        a_only = read_bytes(self.spool).split(b"\n")[0] + b"\n"
        with open(self.spool, "r+b") as f:
            f.truncate(0)
            f.write(a_only)
        put(self.spool, self.ev("F", 40))
        self.tick(rd)
        self.assertEqual(self.routed, ["F"])

    def test_h_row2_a_restore_seen_mid_write_holds_then_rewinds(self):
        rd = self.first_sight("A", "B")
        copy = read_bytes(self.spool)
        self.consume(rd, ("C", 10), ("E", 30))
        with open(self.spool, "r+b") as f:
            f.truncate(0)
            f.write(copy[:20])                            # less than one record, no newline: cannot be judged yet
        self.tick(rd)
        self.assertEqual(self.routed, [])
        with open(self.spool, "ab") as f:
            f.write(copy[20:])
        put(self.spool, self.ev("F", 40))
        self.tick(rd)
        self.assertEqual(self.routed, ["F"], "the half-written restore was held, not read from 0")

    def test_h_row7_a_different_file_at_the_live_name_is_read_from_0(self):
        rd = self.first_sight("A")
        self.consume(rd, ("B", 10))
        os.remove(self.spool)
        put(self.spool, *(self.ev("N%d" % i, 50 + i) for i in range(6)))   # longer than the old offset, other first bytes
        self.tick(rd)
        self.assertEqual(self.routed, ["N%d" % i for i in range(6)])
        self.assertIn("different file", self.log.getvalue())

    def test_h_row6_a_restored_archive_older_than_the_last_rotation_is_not_replayed(self):
        """GO amendment (1): "came back" is judged against EVERY archive in the spool dir (the CEO's directory predicate)."""
        rd = self.first_sight("A")                        # under the real threshold: nothing rotates at first sight
        self.past_grace()                                 # the grace counts from the reader's start
        with mock.patch.object(HB, "SPOOL_ROTATE_BYTES", 10):
            self.consume(rd, ("X1", 1))
            self.tick(rd)                                 # drained above 10 B: rotation 1
            self.consume(rd, ("Y1", 2))
            self.tick(rd)                                 # rotation 2
        arch = self.archives()
        self.assertEqual(len(arch), 2, arch)
        d = os.path.dirname(self.spool)                   # by content: `...Z-2.jsonl` sorts BEFORE `...Z.jsonl`
        (first,) = [os.path.join(d, n) for n in arch if b'"task": "A"' in read_bytes(os.path.join(d, n))]
        with open(self.spool, "wb") as f:                 # the FIRST archive (older than the last rotation) restored
            f.write(read_bytes(first))
        put(self.spool, self.ev("W", 3))
        self.tick(rd)
        self.assertEqual(self.routed, ["W"], "A/X1 (archive 1) are not replayed; W is routed once")
        self.assertEqual(self.archives(), arch, "both archives are still there")

    def test_h_row6_prev_head_covers_an_archive_that_lives_only_in_prev(self):
        """TOCTOU case C: the archive is in <pack>.prev only, its pre-rotation copy came back under the live name."""
        rd = self.first_sight("A")
        self.past_grace()
        with mock.patch.object(HB, "SPOOL_ROTATE_BYTES", 10):
            self.consume(rd, ("X1", 1))
            self.tick(rd)                                 # rotation
        (only,) = self.archives()
        elsewhere = os.path.join(self.tmp, "prev-" + only)
        os.rename(os.path.join(os.path.dirname(self.spool), only), elsewhere)   # test-only: the archive is not in the dir
        with open(self.spool, "wb") as f:
            f.write(read_bytes(elsewhere))
        put(self.spool, self.ev("W", 3))
        self.tick(rd)
        self.assertEqual(self.routed, ["W"])

    def test_h_a_legacy_integer_offset_is_first_sight_never_a_resume(self):
        os.makedirs(self.state)
        with open(os.path.join(self.state, "evt_spool-main.offset"), "w") as f:
            f.write("5")
        put(self.spool, self.ev("A", 0), self.ev("B", 1))
        rd = self.reader()
        self.merge(fleet())
        self.tick(rd)
        self.assertEqual(self.routed, [], "a record without the identity is first sight (EOF), never a resume at 5")
        self.assertEqual(self.lane(rd, "main")["offset"], os.path.getsize(self.spool))

    @unittest.skipUnless(os.name == "posix", "POSIX file modes")
    def test_h_the_offset_record_keeps_its_file_mode(self):
        """GO amendment (2): tmp + os.replace must not turn the record into 0600 (mkstemp) - first write = open(...,"w")."""
        um = os.umask(0)
        os.umask(um)
        rd = self.first_sight("A")
        opath = os.path.join(self.state, "evt_spool-main.offset")
        self.assertEqual(os.stat(opath).st_mode & 0o777, 0o666 & ~um, "first write: the mode open(path, 'w') gives")
        os.chmod(opath, 0o640)
        self.consume(rd, ("B", 1))
        self.assertEqual(os.stat(opath).st_mode & 0o777, 0o640, "a save keeps the existing file's mode")

    @unittest.skipUnless(os.name == "posix", "POSIX file modes")
    def test_h_an_archive_keeps_the_spool_mode(self):
        rd = self.first_sight("A")
        self.past_grace()
        os.chmod(self.spool, 0o640)
        with mock.patch.object(HB, "SPOOL_ROTATE_BYTES", 10):
            self.consume(rd, ("B", 1))
            self.tick(rd)
        (only,) = self.archives()
        self.assertEqual(os.stat(os.path.join(os.path.dirname(self.spool), only)).st_mode & 0o777, 0o640)


class SpoolRotationGateH(_SpoolH):
    """F1-C (head ruling E5-F1-F2): no rotation while a pack install can be in its copy->swap window - G-a start grace,
    G-b a fresh `.pack-staging-init-*` sibling of the pack; the condition-5 line still appears once the rotation happens."""

    def rotate_ready(self):
        rd = self.first_sight("A")
        self.consume(rd, ("B", 1))                        # offset now above the patched 10 B threshold
        return rd

    def test_h_ga_no_rotation_inside_the_start_grace_then_it_rotates_with_the_condition_5_line(self):
        with mock.patch.object(HB, "SPOOL_ROTATE_BYTES", 10):
            rd = self.rotate_ready()
            self.tick(rd, 3)
            self.assertEqual(self.archives(), [], "inside the grace nothing rotates")
            self.assertEqual(self.log.getvalue().count("start grace"), 1, "one line per deferral episode")
            self.past_grace()
            self.tick(rd)
        self.assertEqual(len(self.archives()), 1)
        self.assertIn("spool rotated:", self.log.getvalue())

    def test_h_gb_a_fresh_install_staging_defers_rotation_until_it_is_gone(self):
        rd = self.first_sight("A")                        # under the real threshold
        self.past_grace()
        staging = os.path.join(self.tmp, ".pack-staging-init-4242")   # sibling of the main pack (<tmp>/main)
        os.makedirs(staging)
        with mock.patch.object(HB, "SPOOL_ROTATE_BYTES", 10):
            self.tick(rd, 2)
            self.assertEqual(self.archives(), [])
            self.assertIn("pack install in progress (.pack-staging-init-4242)", self.log.getvalue())
            os.rmdir(staging)                             # the swap renamed it into place
            self.tick(rd)
        self.assertEqual(len(self.archives()), 1)

    def test_h_gb_stale_staging_residue_is_ignored_and_logged_once(self):
        staging = os.path.join(self.tmp, ".pack-staging-init-77")
        os.makedirs(staging)
        old = time.time() - getattr(HB, "STAGING_FRESH_SECS", 600) - 100
        os.utime(staging, (old, old))
        rd = self.first_sight("A")                        # under the real threshold
        self.past_grace()
        with mock.patch.object(HB, "SPOOL_ROTATE_BYTES", 10):
            self.tick(rd)                                 # drained above 10 B: one rotation attempt
        self.assertEqual(len(self.archives()), 1, "a crashed install's leftover does not stop rotation forever")
        self.assertEqual(self.log.getvalue().count("residue"), 1)

    def test_h_post_check_logs_an_install_that_appeared_during_the_rename(self):
        real = HB.archive_spool

        def racing(path, now=None):
            os.makedirs(os.path.join(self.tmp, ".pack-staging-init-9"), exist_ok=True)   # the copy started meanwhile
            return real(path, now)
        rd = self.first_sight("A")
        self.past_grace()
        with mock.patch.object(HB, "SPOOL_ROTATE_BYTES", 10), mock.patch.object(HB, "archive_spool", racing):
            self.tick(rd)
        self.assertEqual(len(self.archives()), 1)
        self.assertIn("may have raced a pack install", self.log.getvalue())


if __name__ == "__main__":
    if sys.argv[1:] == ["--write-golden"]:
        _write_golden()
        sys.exit(0)
    unittest.main(verbosity=2)
