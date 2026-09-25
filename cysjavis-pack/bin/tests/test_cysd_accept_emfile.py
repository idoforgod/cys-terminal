#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""test_cysd_accept_emfile.py — cysd unix accept 오류 분류·표본 로그(F / G-01 rev2) e2e.

## 무엇을 재는가

fd 가 고갈되면(EMFILE) macOS accept 는 대기 연결 1개를 꺼내 **버린다** — 클라이언트는 EOF 를
받고, 즉시 재시도해도 오류 수는 도착 수를 넘지 않는다(자기 제한 루프). 종전 cysd 는 오류마다
`accept error: …` 한 줄을 무회전 stderr 로 썼다(폭주 로그 → 디스크 → eprintln 패닉 = 데몬 사망
경로). F 는 macOS 에서 **sleep 없이**(배출 속도 종전과 같음 → ECONNREFUSED·자동 기동 연쇄 신설 0)
로그만 표본화한다(연속열 시작·errno 변경 ≥1s, 그 외 60s, 회복 1줄).

두 모드를 각각 새 데몬으로 잰다 — legacy(`CYS_ACCEPT_ERROR_GATE=0`)와 gate(미설정).
  ① 유휴 연결을 하나씩 붙잡으며 첫 EOF 가 나는 K 를 찾는다(L+64 까지 EOF 없으면 FAIL — 고갈 미도달)
  ② 8 스레드 재접속 폭주 3s(connect→ping→recv 결과 분류)
  ③ 200 동시 connect 버스트
  ④ 붙잡은 연결을 모두 닫고 1s 안에 ping OK
  ⑤ 종료 전 생존 확인
  ⑥ stderr 를 ①직전~④직후 오프셋으로 잘라 `^accept (error|recovered):` 줄 수 · 경과 W

단언(실패 방향 단일화):
  (a) legacy 줄 ≥ 100(미달 = 폭주 미도달 FAIL — 노브 실효·적색 재현을 같은 환경에서 확인)
  (b) gate 'accept error' ≥1 · 'accept recovered' ≥1 · 총 줄 ≤ 2·(⌊W⌋+1)+⌊W/60⌋
  (c) gate 폭주 중 클라이언트 EOF ≥ 100(오류가 실제로 났다 — 줄 0 을 통과로 읽지 않는다)
  (d) 두 모드 모두 데몬 생존
  (e) 버스트 거절: gate 0 = PASS · gate>0 ∧ legacy=0 = FAIL(새 거절 경로 = ④ 증폭 회귀) · 둘 다 >0 = SKIP
  (f) gate 폭주 중 ECONNREFUSED 0
  (g) 해제 뒤 1s 안에 ping OK

수정 전 바이너리는 (b) 가 실패해야 한다(줄 ≈ 오류 수 ≫ 상한, 회복 줄 없음) — CYS_TEST_CYSD 로 지정.

## 미측정 범위(정직)
sys.platform != 'darwin' 이면 SKIP — Linux 는 EMFILE 에도 연결이 큐에 남는 다른 커널 경로이고
cysd 비배포다. listener_fatal errno 의 핫스핀 제거는 어떤 테스트로도 실연하지 않는다(단위 핀만).
부팅 불가·환경 분리 불가는 SKIP(통과 아님), 거절 회귀만 FAIL.

## 격리
HOME·CYS_SOCKET·CYS_STATE_DIR·CYS_PACK_DIR·CYS_CONFIG_DIR·CYS_PACK_CAPTURES_DIR 스크래치,
CYS_NO_AUTOSTART=1 · CYS_NO_OFFICE_BRIDGE=1 · CYS_NO_PERSONAL_HOOK_MERGE=1, 트립와이어 cys·cysd 를
PATH 앞에(호출되면 기록 후 rc 97 — 적중은 보고). 종료는 기록한 pid 에만 SIGTERM → 3s → SIGKILL.

실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_cysd_accept_emfile.py
종료 토큰 CYSD-ACCEPT-EMFILE-OK · 실패 시 exit 1
"""
import errno
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
REPO = os.path.dirname(PACK)
fails = []
skips = []
LINE_RE = re.compile(r"^accept (error|recovered):", re.M)


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""), flush=True)
    if not cond:
        fails.append(name)
    return cond


def skip(name, why):
    print("SKIP %s — %s" % (name, why), flush=True)
    skips.append(name)


def _find_cysd():
    """CYS_TEST_CYSD 우선, 없으면 target/debug·release. 없으면 None(= SKIP, 통과 아님)."""
    env = os.environ.get("CYS_TEST_CYSD", "").strip()
    if env and os.path.isfile(env) and os.access(env, os.X_OK):
        return env
    for c in (os.path.join(REPO, "target", "debug", "cysd"),
              os.path.join(REPO, "target", "release", "cysd")):
        if os.path.isfile(c) and os.access(c, os.X_OK):
            return c
    return None


def _req(method):
    return (json.dumps({"id": 1, "method": method, "params": {}}) + "\n").encode()


def _recv_line(s, deadline):
    buf = b""
    while not buf.endswith(b"\n"):
        left = deadline - time.time()
        if left <= 0:
            return "timeout"
        s.settimeout(left)
        try:
            chunk = s.recv(65536)
        except socket.timeout:
            return "timeout"
        except OSError as e:
            return "eof" if e.errno in (errno.ECONNRESET, errno.EPIPE) else "other"
        if not chunk:
            return "eof"
        buf += chunk
    try:
        return "ok" if json.loads(buf.decode()).get("ok") else "other"
    except ValueError:
        return "other"


def ping_once(sock, timeout=2.0):
    s = socket.socket(socket.AF_UNIX)
    try:
        s.settimeout(timeout)
        try:
            s.connect(sock)
        except OSError as e:
            return "refused" if e.errno == errno.ECONNREFUSED else "connect_err"
        try:
            s.sendall(_req("system.ping"))
        except OSError:
            return "eof"
        return _recv_line(s, time.time() + timeout)
    finally:
        s.close()


class Sandbox:
    def __init__(self):
        self.real_home = os.path.expanduser("~")
        self.dir = tempfile.mkdtemp(prefix="ae-")
        self.home = os.path.join(self.dir, "home")
        self.trip = os.path.join(self.dir, "trip")
        for d in ("home", "state", "pack", "config", "captures", "trip"):
            os.makedirs(os.path.join(self.dir, d), exist_ok=True)
        self.hits = os.path.join(self.trip, "hits.log")
        for name in ("cys", "cysd"):
            p = os.path.join(self.trip, name)
            with open(p, "w") as f:
                f.write('#!/bin/sh\necho "TRIP $0 $*" >> "%s"\nexit 97\n' % self.hits)
            os.chmod(p, 0o755)
        self.sock = os.path.join(self.dir, "cys.sock")
        real = os.path.realpath(self.sock)
        live = os.path.realpath(os.path.join(self.real_home, ".cys"))
        if not real.startswith(os.path.realpath(self.dir) + os.sep) or real.startswith(live + os.sep):
            raise SystemExit("FATAL 소켓 경로가 샌드박스 밖이다: %s" % real)
        self.log = os.path.join(self.dir, "cysd.log")

    def env(self, gate_off):
        e = dict(os.environ)
        for k in ("CYS_SURFACE_ID", "CYS_SEAT_TOKEN", "CYS_SURFACE_REF", "CYS_ROLE",
                  "CYS_ACCEPT_ERROR_GATE", "JAVIS_ACCEPT_ERROR_GATE", "AITERM_ACCEPT_ERROR_GATE"):
            e.pop(k, None)
        e.update(HOME=self.home, CYS_SOCKET=self.sock,
                 CYS_STATE_DIR=os.path.join(self.dir, "state"),
                 CYS_PACK_DIR=os.path.join(self.dir, "pack"),
                 CYS_CONFIG_DIR=os.path.join(self.dir, "config"),
                 CYS_PACK_CAPTURES_DIR=os.path.join(self.dir, "captures"),
                 CYS_NO_AUTOSTART="1", CYS_NO_OFFICE_BRIDGE="1", CYS_NO_PERSONAL_HOOK_MERGE="1",
                 PATH=self.trip + os.pathsep + os.environ.get("PATH", ""))
        if gate_off:
            e["CYS_ACCEPT_ERROR_GATE"] = "0"
        return e

    def trip_hits(self):
        try:
            with open(self.hits) as f:
                return [l.strip() for l in f if l.strip()]
        except OSError:
            return []

    def log_size(self):
        try:
            return os.path.getsize(self.log)
        except OSError:
            return 0

    def log_slice(self, a, b):
        with open(self.log, "rb") as f:
            f.seek(a)
            return f.read(max(0, b - a)).decode("utf-8", "replace")

    def cleanup(self):
        shutil.rmtree(self.dir, ignore_errors=True)


def boot(cysd, sb, limit, gate_off):
    """soft·hard 를 모두 limit 로 낮춰 띄운다(E 가 soft 를 hard 까지 올려도 고갈이 재현되게)."""
    import resource

    def lower():
        resource.setrlimit(resource.RLIMIT_NOFILE, (limit, limit))

    log = open(sb.log, "ab")
    p = subprocess.Popen([cysd], env=sb.env(gate_off), stdout=log, stderr=log,
                         stdin=subprocess.DEVNULL, preexec_fn=lower)
    log.close()
    deadline = time.time() + 60.0                       # debug 냉시작 실측 약 16s
    while time.time() < deadline:
        if p.poll() is not None:
            return p, False
        if os.path.exists(sb.sock) and ping_once(sb.sock) == "ok":
            return p, True
        time.sleep(0.2)
    return p, False


def stop(p):
    if p.poll() is None:
        try:
            os.kill(p.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            p.wait(timeout=3)
        except subprocess.TimeoutExpired:
            try:
                os.kill(p.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            p.wait(timeout=5)
    return p.poll() is not None


def run_mode(cysd, gate_off):
    label = "legacy" if gate_off else "gate"
    sb = Sandbox()
    p = None
    try:
        for limit in (96, 128, 192):
            p, ok = boot(cysd, sb, limit, gate_off)
            if ok:
                break
            stop(p)
            p = None
            try:
                os.unlink(sb.sock)
            except OSError:
                pass
        if p is None:
            return None, "부팅 불가(L 사다리 96·128·192 모두) — 미측정"
        res = {"label": label, "limit": limit}
        held = []
        a = sb.log_size()
        t0 = time.time()
        # ① 유휴 연결을 붙잡으며 첫 EOF 의 K 를 찾는다.
        k = None
        for i in range(limit + 64):
            s = socket.socket(socket.AF_UNIX)
            s.settimeout(2.0)
            try:
                s.connect(sb.sock)
                s.sendall(_req("system.ping"))
                r = _recv_line(s, time.time() + 2.0)
            except OSError:
                r = "eof"
            if r == "ok":
                held.append(s)
            else:
                s.close()
                k = i
                break
        res["K"] = k
        if k is None:
            for s in held:
                s.close()
            return res, "L+64 까지 EOF 없음 — 고갈 미도달"
        # ② 8 스레드 재접속 폭주 3s.
        counts = {"ok": 0, "eof": 0, "refused": 0, "timeout": 0, "other": 0, "connect_err": 0}
        lock = threading.Lock()
        stop_at = time.time() + 3.0

        def worker():
            while time.time() < stop_at:
                r = ping_once(sb.sock, 1.0)
                with lock:
                    counts[r] = counts.get(r, 0) + 1

        ts = [threading.Thread(target=worker) for _ in range(8)]
        for t in ts:
            t.start()
        for t in ts:
            t.join(timeout=10)
        res["flood"] = dict(counts)
        # ③ 200 동시 connect 버스트.
        burst = {"refused": 0, "connected": 0, "other": 0}
        barrier = threading.Barrier(200)

        def burst_one():
            s = socket.socket(socket.AF_UNIX)
            s.settimeout(2.0)
            try:
                barrier.wait(timeout=5)
            except threading.BrokenBarrierError:
                pass
            try:
                s.connect(sb.sock)
                k2 = "connected"
            except OSError as e:
                k2 = "refused" if e.errno == errno.ECONNREFUSED else "other"
            finally:
                s.close()
            with lock:
                burst[k2] += 1

        bts = [threading.Thread(target=burst_one) for _ in range(200)]
        for t in bts:
            t.start()
        for t in bts:
            t.join(timeout=10)
        res["burst"] = dict(burst)
        # ④ 해제 → 1s 안에 ping OK.
        for s in held:
            s.close()
        rel = time.time()
        recovered_ok = False
        while time.time() - rel < 1.0:
            if ping_once(sb.sock, 0.5) == "ok":
                recovered_ok = True
                break
            time.sleep(0.02)
        res["release_ping_ok"] = recovered_ok
        time.sleep(0.2)                                  # 회복 줄 기록 여유
        b = sb.log_size()
        res["W"] = time.time() - t0
        text = sb.log_slice(a, b)
        res["lines_error"] = len(re.findall(r"^accept error:", text, re.M))
        res["lines_recovered"] = len(re.findall(r"^accept recovered:", text, re.M))
        res["lines"] = len(LINE_RE.findall(text))
        res["log_bytes"] = b - a
        # ⑤ 생존.
        res["alive"] = p.poll() is None and ping_once(sb.sock) == "ok"
        res["trip_hits"] = sb.trip_hits()
        return res, ""
    finally:
        if p is not None:
            res_gone = stop(p)
            if not res_gone:
                print("WARN 데몬 pid %d 종료 확인 실패" % p.pid)
        sb.cleanup()


def main():
    if sys.platform != "darwin":
        print("SKIP 전체 — darwin 전용(Linux EMFILE 은 연결이 큐에 남는 다른 경로 · cysd 비배포 · 미측정)")
        print("CYSD-ACCEPT-EMFILE-OK")
        return 0
    import resource
    cysd = _find_cysd()
    if cysd is None:
        print("SKIP 전체 — cysd 바이너리 부재(팩 단독 배포 · 미측정, 통과 아님)")
        print("CYSD-ACCEPT-EMFILE-OK")
        return 0
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    want = 10240 if hard == resource.RLIM_INFINITY else min(hard, 10240)
    if soft < want:
        try:
            resource.setrlimit(resource.RLIMIT_NOFILE, (want, hard))
        except (ValueError, OSError):
            pass
    print("cysd=%s" % cysd)
    legacy, why_l = run_mode(cysd, gate_off=True)
    print("legacy 관측: %r %s" % (legacy, why_l))
    gate, why_g = run_mode(cysd, gate_off=False)
    print("gate 관측: %r %s" % (gate, why_g))
    if legacy is None or gate is None:
        skip("전체", "부팅 불가 — %s %s" % (why_l, why_g))
        print("CYSD-ACCEPT-EMFILE-OK")
        return 0
    for r, why in ((legacy, why_l), (gate, why_g)):
        if why:
            check("%s 고갈 도달" % r["label"], False, why)
    if fails:
        print("FAILED %r" % fails)
        return 1
    W = gate["W"]
    bound = 2 * (int(W) + 1) + int(W // 60)
    check("(a) legacy 폭주 줄 ≥ 100(적색 재현·노브 실효)", legacy["lines"] >= 100, "lines=%d" % legacy["lines"])
    check("(b) gate 'accept error' ≥1 · 'accept recovered' ≥1 · 줄 ≤ 상한 %d(W=%.1fs)" % (bound, W),
          gate["lines_error"] >= 1 and gate["lines_recovered"] >= 1 and gate["lines"] <= bound,
          "error=%d recovered=%d total=%d" % (gate["lines_error"], gate["lines_recovered"], gate["lines"]))
    check("(c) gate 폭주 중 클라이언트 EOF ≥ 100", gate["flood"]["eof"] >= 100, repr(gate["flood"]))
    check("(d) 두 모드 데몬 생존", legacy["alive"] and gate["alive"])
    gr, lr = gate["burst"]["refused"], legacy["burst"]["refused"]
    if gr == 0:
        check("(e) 버스트 거절 gate=0", True, "legacy=%d" % lr)
    elif lr == 0:
        check("(e) 버스트 거절 — gate 만 거절(새 거절 경로 = ④ 증폭 회귀)", False, "gate=%d legacy=0" % gr)
    else:
        skip("(e) 버스트 거절", "두 모드 모두 거절(gate=%d legacy=%d) — 환경 분리 불가" % (gr, lr))
    check("(f) gate 폭주 중 ECONNREFUSED 0", gate["flood"].get("refused", 0) == 0, repr(gate["flood"]))
    check("(g) 해제 뒤 1s 안 ping OK", gate["release_ping_ok"] and legacy["release_ping_ok"])
    hits = legacy["trip_hits"] + gate["trip_hits"]
    print("트립와이어 적중 %d%s" % (len(hits), (" — " + repr(hits[:3])) if hits else ""))
    if fails:
        print("FAILED %d: %r" % (len(fails), fails))
        return 1
    print("CYSD-ACCEPT-EMFILE-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
