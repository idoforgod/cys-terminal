#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_hud_bridge_0144_specimen.py — 브리지 스크립트 단독 실행 검체 (0.14.44 B4 · B2 의 스크립트 쪽).

임시 포트(18671) · 임시 HUD_STATE_DIR · 임시 HOME · 가짜 HUD_CYS_BIN(cys 가 아님 — 이벤트 구독 자식은 sleep).
라이브(~/.cys · 8642 · 설치본)에는 닿지 않는다. 유닉스 전용(윈도우는 단위 시험이 판정).
  ① /health 200 + 칸  ② 수명줄 EOF → 1초 안 종료 · 자식 0  ③ SIGTERM → 1초 안 종료 · 자식 0
  ④ 환경변수 없는 기동은 수명줄 없이 산다  ⑤ 포트 점유 → 0.5초 안 종료 · 토큰 파일 불변 · 자식 0
  ⑥ 요청을 보내지 않는 접속은 30초에 정리(시간 제한 · 계수 1)  ⑦ HUD_REQ_TIMEOUT 손잡이(2초) · 0 이면 끔
"""
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.request
import warnings
warnings.simplefilter("ignore", ResourceWarning)

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
SCRIPT = os.path.join(BIN, "javis_hud_bridge.py")
PORT = 18671


def alive(pid):
    r = subprocess.run(["ps", "-p", str(pid), "-o", "pid="], capture_output=True, text=True)
    return bool(r.stdout.strip())


@unittest.skipIf(os.name == "nt", "유닉스 전용 검체")
class Specimen(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="hb0144-")
        self.state = os.path.join(self.tmp, "state")
        os.makedirs(self.state)
        self.home = os.path.join(self.tmp, "home")
        os.makedirs(self.home)
        self.childpid = os.path.join(self.tmp, "child.pid")
        self.fake = os.path.join(self.tmp, "fakecys")
        with open(self.fake, "w") as f:
            f.write('#!/bin/sh\n'
                    'for a in "$@"; do if [ "$a" = events ]; then echo $$ >> "%s"; exec sleep 300; fi; done\n'
                    'echo "{}"\n' % self.childpid)
        os.chmod(self.fake, 0o755)
        self.procs = []

    def tearDown(self):
        for p in self.procs:
            if p.poll() is None:
                p.kill()
                p.wait()
        for pid in self.child_pids():
            if alive(pid):
                try:
                    os.kill(pid, signal.SIGKILL)
                except OSError:
                    pass
        shutil.rmtree(self.tmp, ignore_errors=True)

    def env(self, **extra):
        e = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": self.home, "HUD_PORT": str(PORT),
             "HUD_STATE_DIR": self.state, "HUD_CYS_BIN": self.fake, "JAVIS_ROOT": os.path.join(self.tmp, "jr"),
             "HUD_PACK_VERSION": "t-0.0.0", "PYTHONDONTWRITEBYTECODE": "1"}
        e.update(extra)
        return e

    def start(self, stdin=subprocess.DEVNULL, **extra):
        p = subprocess.Popen([sys.executable, SCRIPT], env=self.env(**extra), stdin=stdin,
                             stdout=subprocess.DEVNULL, stderr=open(os.path.join(self.tmp, "err.log"), "ab"))
        self.procs.append(p)
        return p

    def health(self, timeout=3):
        with urllib.request.urlopen("http://127.0.0.1:%d/health" % PORT, timeout=timeout) as r:
            return r.status, json.loads(r.read())

    def wait_health(self, p, limit=8.0):
        end = time.monotonic() + limit
        while time.monotonic() < end:
            if p.poll() is not None:
                self.fail("브리지가 일찍 끝남 rc=%s: %s" % (p.returncode, self.errlog()))
            try:
                return self.health(1)
            except Exception:
                time.sleep(0.1)
        self.fail("health 없음: " + self.errlog())

    def errlog(self):
        try:
            return open(os.path.join(self.tmp, "err.log"), errors="replace").read()[-800:]
        except OSError:
            return ""

    def child_pids(self):
        try:
            return [int(x) for x in open(self.childpid).read().split()]
        except (OSError, ValueError):
            return []

    def wait_children(self, n=1, limit=6.0):
        end = time.monotonic() + limit
        while time.monotonic() < end:
            pids = [x for x in self.child_pids() if alive(x)]
            if len(pids) >= n:
                return pids
            time.sleep(0.1)
        self.fail("이벤트 구독 자식이 안 뜸: " + self.errlog())

    # ① -----------------------------------------------------------------
    def test_health_200(self):
        p = self.start()
        code, j = self.wait_health(p)
        self.assertEqual(code, 200)
        self.assertEqual(j["pid"], p.pid)
        self.assertEqual(j["pack_version"], "t-0.0.0")
        self.assertRegex(j["boot_id"], r"^[0-9a-f]{16}$")
        self.assertEqual(sorted(j["assets"]), ["office3d_html", "office_boot_js", "three_module_js"])
        self.assertEqual(j["timeouts"], 0)
        # 기존 주소는 그대로: /world 200
        with urllib.request.urlopen("http://127.0.0.1:%d/world" % PORT, timeout=3) as r:
            self.assertEqual(r.status, 200)
        # 로그 줄: 머리 + 시각 + 스레드 이름
        time.sleep(0.3)
        first = self.errlog().splitlines()[0]
        self.assertRegex(first, r"^\[hud-bridge\] \d{4}-\d\d-\d\dT\d\d:\d\d:\d\d \[MainThread\] http://127\.0\.0\.1:%d" % PORT)
        # 다른 기동은 다른 기동 식별자
        p.send_signal(signal.SIGTERM)
        p.wait(3)

    # ② -----------------------------------------------------------------
    def test_lifeline_eof_exits_within_1s_and_no_children(self):
        p = self.start(stdin=subprocess.PIPE, HUD_LIFELINE="stdin")
        self.wait_health(p)
        kids = self.wait_children(1)
        time.sleep(1.0)
        self.assertIsNone(p.poll(), "파이프가 열려 있는 동안 끝나면 안 된다(잘못된 조기 종료)")
        t0 = time.monotonic()
        p.stdin.close()
        p.wait(5)
        dt = time.monotonic() - t0
        self.assertLess(dt, 1.0, "수명줄 EOF 뒤 종료까지 %.2fs" % dt)
        time.sleep(0.5)
        self.assertEqual([k for k in kids if alive(k)], [], "자식이 남았다")

    # ③ -----------------------------------------------------------------
    def test_sigterm_exits_within_1s_and_no_children(self):
        p = self.start(stdin=subprocess.PIPE, HUD_LIFELINE="stdin")
        self.wait_health(p)
        kids = self.wait_children(1)
        t0 = time.monotonic()
        p.send_signal(signal.SIGTERM)
        p.wait(5)
        dt = time.monotonic() - t0
        self.assertLess(dt, 1.0, "SIGTERM 뒤 종료까지 %.2fs" % dt)
        self.assertEqual(p.returncode, 0)
        time.sleep(0.5)
        self.assertEqual([k for k in kids if alive(k)], [], "자식이 남았다")
        self.assertIn("shutdown: sigterm", self.errlog())

    # ④ -----------------------------------------------------------------
    def test_manual_start_without_lifeline_env_ignores_stdin_eof(self):
        p = self.start(stdin=subprocess.PIPE)      # HUD_LIFELINE 없음
        self.wait_health(p)
        p.stdin.close()
        time.sleep(2.0)
        self.assertIsNone(p.poll(), "환경변수 없는 수동 기동은 표준입력이 닫혀도 산다")
        p.send_signal(signal.SIGTERM)
        p.wait(3)

    # ⑤ -----------------------------------------------------------------
    def test_port_held_exits_fast_and_leaves_token_alone(self):
        tok = os.path.join(self.state, "token")
        with open(tok, "w") as f:
            f.write("ORIGINAL-TOKEN")
        before = (open(tok).read(), os.stat(tok).st_mtime_ns)
        s = socket.socket()
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind(("127.0.0.1", PORT))
        s.listen(5)
        try:
            t0 = time.monotonic()
            p = self.start(stdin=subprocess.PIPE, HUD_LIFELINE="stdin")
            rc = p.wait(5)
            dt = time.monotonic() - t0
        finally:
            s.close()
        self.assertNotEqual(rc, 0)
        self.assertLess(dt, 0.5, "포트 점유 시 종료까지 %.2fs (프로세스 기동 포함)" % dt)
        self.assertEqual((open(tok).read(), os.stat(tok).st_mtime_ns), before, "토큰 파일이 바뀌었다")
        self.assertEqual(self.child_pids(), [], "자식이 떴다")
        self.assertIn("포트 %d 를 잡지 못해" % PORT, self.errlog())

    # ⑥ -----------------------------------------------------------------
    def test_silent_connection_is_cut_at_30s_and_counted(self):
        p = self.start()
        self.wait_health(p)
        c = socket.create_connection(("127.0.0.1", PORT))
        t0 = time.monotonic()
        c.settimeout(40)
        data = c.recv(10)
        dt = time.monotonic() - t0
        c.close()
        self.assertEqual(data, b"", "서버가 닫아야 한다")
        self.assertGreaterEqual(dt, 29.0)
        self.assertLessEqual(dt, 32.0, "정리까지 %.1fs" % dt)
        self.assertEqual(self.health()[1]["timeouts"], 1)
        self.assertNotIn("timed out", self.errlog(), "시간 제한 접속은 로그를 찍지 않는다")

    # ⑦ -----------------------------------------------------------------
    def test_timeout_knob_short_and_off(self):
        p = self.start(HUD_REQ_TIMEOUT="2")
        self.wait_health(p)
        c = socket.create_connection(("127.0.0.1", PORT))
        t0 = time.monotonic()
        c.settimeout(10)
        self.assertEqual(c.recv(10), b"")
        self.assertLess(time.monotonic() - t0, 4.0)
        c.close()
        p.send_signal(signal.SIGTERM)
        p.wait(3)
        time.sleep(0.3)
        p2 = self.start(HUD_REQ_TIMEOUT="0")
        self.wait_health(p2)
        c = socket.create_connection(("127.0.0.1", PORT))
        c.settimeout(4)
        with self.assertRaises(socket.timeout):
            c.recv(10)                             # 끔 — 4초가 지나도 서버가 끊지 않는다
        c.close()
        p2.send_signal(signal.SIGTERM)
        p2.wait(3)


if __name__ == "__main__":
    unittest.main(verbosity=2)
