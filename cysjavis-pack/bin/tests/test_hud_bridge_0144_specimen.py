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

    def diag(self, p):
        """실패 메시지용 진단 — 프로세스 생존·rc·포트 상태·stderr/stdout 꼬리(빈 err.log 라도 살아 있었는지 죽었는지 구별)."""
        rc = p.poll()
        try:
            c = socket.create_connection(("127.0.0.1", PORT), timeout=1)
            c.close()
            port = "접속 됨"
        except OSError as e:
            port = "접속 실패(%s)" % e.__class__.__name__
        return "pid=%s %s rc=%s · 포트 %d %s · python=%s · stderr=%r" % (
            p.pid, "생존" if rc is None else "종료", rc, PORT, port, sys.version.split()[0], self.errlog())

    def wait_health(self, p, limit=25.0):
        end = time.monotonic() + limit
        while time.monotonic() < end:
            if p.poll() is not None:
                self.fail("브리지가 일찍 끝남: " + self.diag(p))
            try:
                return self.health(1)
            except Exception:
                time.sleep(0.1)
        self.fail("health 없음(%.0fs): %s" % (limit, self.diag(p)))

    def errlog(self):
        try:
            return open(os.path.join(self.tmp, "err.log"), errors="replace").read()[-800:]
        except OSError:
            return ""

    def wait_spool(self, p, pred, limit=20.0):
        """(0.14.56) Poll /health until pred(body) holds - the reader thread checks lanes asynchronously."""
        end, j = time.monotonic() + limit, None
        while time.monotonic() < end:
            if p.poll() is not None:
                self.fail("브리지가 일찍 끝남: " + self.diag(p))
            try:
                j = self.health(1)[1]
                if pred(j):
                    return j
            except Exception:
                pass
            time.sleep(0.1)
        self.fail("spool 조건 미충족(%.0fs): %r · %s" % (limit, j and j.get("spool"), self.errlog()))

    def rig(self, leaf=True):
        """(0.14.56) A rig pack: a COPY of bin/ (no tests/) at <tmp>/rig/pack - so the bridge's main lane is
        <tmp>/rig/pack/state/evt_spool.jsonl - and a dept pack <tmp>/rig/d7 whose javis_event.py answers `spool-path` with
        its formula. The fake cys answers `fleet --json` with the base row + dept-7 (pack_dir = that dept pack)."""
        pack = os.path.join(self.tmp, "rig", "pack")
        shutil.copytree(BIN, os.path.join(pack, "bin"), ignore=shutil.ignore_patterns("tests", "__pycache__"))
        leaf_file = os.path.join(pack, "bin", "javis_spool_path.py")
        if not leaf and os.path.exists(leaf_file):
            os.remove(leaf_file)
        d7 = os.path.join(self.tmp, "rig", "d7")
        os.makedirs(os.path.join(d7, "bin"))
        with open(os.path.join(d7, "bin", "javis_event.py"), "w") as f:
            f.write("import sys\nif sys.argv[1:] != ['spool-path']:\n    sys.exit(2)\nprint(%r)\n"
                    % os.path.join(d7, "state", "evt_spool.jsonl"))
        fj = os.path.join(self.tmp, "fleet.json")
        with open(fj, "w") as f:
            json.dump({"departments": [
                {"department": "본부 · CEO", "dept": "main", "socket": None, "pack_dir": None, "surfaces": []},
                {"department": "d7", "dept": "dept-7", "socket": os.path.join(self.tmp, "d7.sock"), "pack_dir": d7,
                 "surfaces": []}]}, f, ensure_ascii=False)
        fake = os.path.join(self.tmp, "fakecys-rig")
        with open(fake, "w") as f:
            f.write('#!/bin/sh\n'
                    'for a in "$@"; do if [ "$a" = events ]; then echo $$ >> "%s"; exec sleep 300; fi; done\n'
                    'for a in "$@"; do if [ "$a" = fleet ]; then cat "%s"; exit 0; fi; done\n'
                    'echo "{}"\n' % (self.childpid, fj))
        os.chmod(fake, 0o755)
        return pack, d7, fake

    def start_rig(self, pack, fake):
        """(0.14.56) The rig's own bridge copy under `python -I -B` (a bare leaf import would not resolve there)."""
        p = subprocess.Popen([sys.executable, "-I", "-B", os.path.join(pack, "bin", "javis_hud_bridge.py")],
                             env=self.env(HUD_CYS_BIN=fake, HUD_POLL="0.3"), stdin=subprocess.DEVNULL,
                             stdout=subprocess.DEVNULL, stderr=open(os.path.join(self.tmp, "err.log"), "ab"))
        self.procs.append(p)
        return p

    def offsets(self):
        return sorted(n for n in os.listdir(self.state) if n.endswith(".offset"))

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
        # (0.14.56 X-8 CARRIER · T8 on the real process) /health lists main, _bridge and _fleet. The fake cys answers `{}`
        # (no fleet), so the startup gate stays closed: `_fleet` waiting (no fault), spool_checked false, no lane read.
        j = self.wait_spool(p, lambda j: bool(j.get("spool")))
        self.assertEqual((j["spool_schema"], j["spool_checked"]), (1, False))
        lanes = {e["lane"]: e for e in j["spool"]}
        self.assertEqual(sorted(lanes), ["_bridge", "_fleet", "main"])
        self.assertEqual([(lanes[k]["state"], lanes[k]["fault"]) for k in ("_bridge", "_fleet")], [("ok", False), ("waiting", False)])
        self.assertIn(lanes["main"]["state"], ("ok", "absent", "unverified"))
        self.assertEqual((lanes["main"]["socket"], lanes["main"]["fault"], lanes["main"]["offset"]), (None, False, None))
        self.assertEqual(lanes["main"]["path"], os.path.join(os.path.dirname(BIN), "state", "evt_spool.jsonl"))
        # 기존 주소는 그대로: /world 200
        with urllib.request.urlopen("http://127.0.0.1:%d/world" % PORT, timeout=3) as r:
            self.assertEqual(r.status, 200)
        # 로그 줄: 머리 + 시각 + 스레드 이름. (0.14.56) 스풀 리더 스레드가 http 줄보다 먼저 한 줄 찍을 수 있다 — http 줄을 찾고
        # 모든 줄의 형식을 본다(종전 '첫 줄' 단언은 그 경쟁에 흔들린다).
        time.sleep(0.3)
        lines = self.errlog().splitlines()
        http = [ln for ln in lines if "http://127.0.0.1:%d" % PORT in ln]
        self.assertEqual(len(http), 1, lines)
        self.assertRegex(http[0], r"^\[hud-bridge\] \d{4}-\d\d-\d\dT\d\d:\d\d:\d\d \[MainThread\] http://127\.0\.0\.1:%d" % PORT)
        for ln in lines:
            self.assertRegex(ln, r"^\[hud-bridge\] \d{4}-\d\d-\d\dT\d\d:\d\d:\d\d \[[^\]]+\] ")
        # 다른 기동은 다른 기동 식별자
        p.send_signal(signal.SIGTERM)
        p.wait(3)

    # ①-b ---------------------------------------------------------------
    def test_health_opens_fast_even_when_reverse_name_lookup_is_slow(self):
        # CI 맥 러너 6/7 실패의 유력 후보(미확정) 재현: 표준 HTTPServer.server_bind 가 바인드 직후 socket.getfqdn 을 불러
        # 이름 해석이 느리면 listen 이 그만큼 늦다. 가짜 sitecustomize 로 getfqdn 을 12초 지연시켜도 /health 가 바로 열려야 한다.
        slow = os.path.join(self.tmp, "slowdns")
        os.makedirs(slow)
        with open(os.path.join(slow, "sitecustomize.py"), "w") as f:
            f.write("import socket, time\n_o = socket.getfqdn\n"
                    "def _slow(name=''):\n    time.sleep(12)\n    return _o(name)\nsocket.getfqdn = _slow\n")
        p = self.start(PYTHONPATH=slow)
        t0 = time.monotonic()
        self.wait_health(p, limit=6.0)
        self.assertLess(time.monotonic() - t0, 4.0)
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

    # ⑧ (0.14.56 X-8 CARRIER · ADDENDUM-1 Q3 specimens on the real script) --------------------------------------
    def test_rig_bridge_under_isolated_python_reads_its_own_lanes(self):
        # Q3(a): the leaf resolves by explicit path under `python -I`; the main lane is the rig pack's own spool; a dept lane
        # is read from its fleet pack_dir with its lane on each record; nothing is written under the rig's bin/ (3-2 (f)).
        pack, d7, fake = self.rig()
        p = self.start_rig(pack, fake)
        j = self.wait_spool(p, lambda j: j.get("spool_checked") is True)
        lanes = {e["lane"]: e for e in j["spool"]}
        self.assertEqual(sorted(lanes), ["_bridge", "_fleet", "dept-7", "main"])
        self.assertEqual((lanes["_bridge"]["state"], lanes["_fleet"]["state"]), ("ok", "ok"), self.errlog())
        self.assertEqual(lanes["main"]["path"], os.path.join(pack, "state", "evt_spool.jsonl"))
        self.assertEqual((lanes["dept-7"]["state"], lanes["dept-7"]["socket"], lanes["dept-7"]["path"]),
                         ("absent", os.path.join(self.tmp, "d7.sock"), os.path.join(d7, "state", "evt_spool.jsonl")))
        spool = os.path.join(d7, "state", "evt_spool.jsonl")
        os.makedirs(os.path.dirname(spool))
        with open(spool, "w") as f:
            f.write(json.dumps({"ts": time.time(), "type": "task.blocked", "payload": {"task": "RIG", "blocked_by": []}}) + "\n")
        size = os.path.getsize(spool)
        self.wait_spool(p, lambda j: any(e["lane"] == "dept-7" and e["state"] == "ok" and e["offset"] == size for e in j["spool"]))
        with urllib.request.urlopen("http://127.0.0.1:%d/world" % PORT, timeout=3) as r:
            self.assertEqual([(b["lane"], b["task"]) for b in json.loads(r.read())["blocked"]], [("dept-7", "RIG")])
        self.assertEqual(self.offsets(), ["evt_spool-dept-7.offset", "evt_spool-main.offset"])
        self.assertEqual([d for d, _, _ in os.walk(os.path.join(self.tmp, "rig")) if os.path.basename(d) == "__pycache__"], [])
        self.assertNotIn("Traceback", self.errlog())
        p.send_signal(signal.SIGTERM)
        p.wait(3)

    def test_rig_main_lane_agrees_with_its_own_producer_code(self):
        # Q3(b): the rig pack's own javis_event.py `spool-path` under -I prints exactly the path the bridge computes for that
        # pack, so the bridge's self-check verifies main (absent: no spool yet) - not unverified (rc 2) or mismatch.
        pack, d7, fake = self.rig()
        r = subprocess.run([sys.executable, "-I", "-B", os.path.join(pack, "bin", "javis_event.py"), "spool-path"],
                           capture_output=True, text=True, timeout=30, env=self.env())
        self.assertEqual((r.returncode, r.stdout), (0, os.path.join(pack, "state", "evt_spool.jsonl") + "\n"), r.stderr[-400:])
        p = self.start_rig(pack, fake)
        j = self.wait_spool(p, lambda j: j.get("spool_checked") is True)
        main = [e for e in j["spool"] if e["lane"] == "main"][0]
        self.assertEqual((main["state"], main["fault"], main["path"]),
                         ("absent", False, os.path.join(pack, "state", "evt_spool.jsonl")), self.errlog())
        p.send_signal(signal.SIGTERM)
        p.wait(3)

    def test_rig_bridge_without_the_leaf_keeps_serving_and_reports_leaf_missing(self):
        # Q3(c): no leaf -> no spool is read, `_bridge` and every listed lane leaf_missing (fault, path null), spool_checked
        # true; the HUD keeps serving and no exception escapes.
        pack, d7, fake = self.rig(leaf=False)
        p = self.start_rig(pack, fake)
        j = self.wait_spool(p, lambda j: len(j.get("spool") or []) == 4)
        self.assertIs(j["spool_checked"], True)
        self.assertEqual(sorted((e["lane"], e["state"], e["fault"], e["path"]) for e in j["spool"]),
                         [("_bridge", "leaf_missing", True, None), ("_fleet", "ok", False, None),
                          ("dept-7", "leaf_missing", True, None), ("main", "leaf_missing", True, None)])
        with urllib.request.urlopen("http://127.0.0.1:%d/world" % PORT, timeout=3) as r:
            self.assertEqual(r.status, 200)
        time.sleep(1.5)
        self.assertIsNone(p.poll())  # wallclock: aging the 1.5 s gives a crash time to show; the verdict on correct code does not depend on it; a crash later than 1.5 s is not seen
        self.assertEqual(self.offsets(), [])
        err = self.errlog()
        self.assertIn("spool leaf unavailable: FileNotFoundError", err)
        self.assertNotIn("Traceback", err)
        p.send_signal(signal.SIGTERM)
        p.wait(3)


if __name__ == "__main__":
    unittest.main(verbosity=2)
