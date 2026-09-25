#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""test_cysd_dispatch_storm_e2e.py — 동시 조회 폭주에서 cysd 가 죽지 않는가(FATAL-1) **로컬 수용 검체**.

## 무엇을 재는가

E(fd soft 자체 상향)는 launchd·GUI 기동(soft 256)에서 연결 수를 묶던 **사실상의 입장 상한**을 없앴다.
그 뒤 surface.list 가 약 350 이상 동시에 들어오면 cysd 가 스택 오버플로로 SIGABRT 했다(리뷰 실측 rc=-6 ·
debug · 크래시 12건 모두 sysinfo `refresh_processes_specifics` → rayon join → 워커 스택 초과). 원인은
sysinfo 기본 feature `multithread` 가 요청마다 rayon 작업을 전역 풀에 넣고, join 대기 중인 워커가 다른
요청의 작업을 훔쳐 같은 스택에 중첩 실행하는 것이다. 수정은 두 겹이다:
  (b) sysinfo `multithread` 제거(Cargo.toml) — 중첩 자체가 없다.
  (a) dispatch 입장 상한 `DISPATCH_INFLIGHT_MAX`(main.rs) — 초과분은 대기, `system.ping` 만 면제.

`ulimit -Sn 256` 셸로 **실데몬**을 띄우고(E 켬 = launchd 와 같은 조건) pane 4개를 만든 뒤, 영속 연결
K 개(기본 500)가 surface.list 를 SECS 초(기본 10) 반복한다. 폭주 중에는 0.5s 마다 새 연결로 ping 한다.

  S1 데몬 생존 — 폭주 뒤 프로세스가 살아 있고 ping OK
  S2 로그에 'overflowed its stack'·'fatal runtime error' 0줄
  S3 surface.list 응답 ok ≥ K(연결마다 평균 1건 이상 — 조용히 버려진 폭주를 통과로 읽지 않는다)
  S4 폭주 중 ping 표본 ≥ 3 이고 전부 ok(입장 면제 — 생존 프로브가 줄 뒤에 서지 않는다)

`--red-only` 는 S1 만 판정해 **적색 재현**(데몬 사망)을 확인한다 — 수정 전 바이너리
(CYS_TEST_CYSD=<구 빌드>)로 TDD 적색 증거를 남길 때 쓴다(재현되면 exit 0, 생존하면 exit 1).

## 왜 CI 레인 밖인가(lane-parity UNREGISTERED_OK 사유)

debug cysd 빌드·실데몬·동시 영속 연결 500·soft 256 기동 셸 재현·10s 폭주가 필요하다. ci-branch 에는
`cargo build` 가 없고 pack-release 는 cys 만 빌드한다. 판정의 CI 몫(sysinfo 가 rayon 에 의존하지 않음 ·
dispatch 가 입장 게이트를 거침 · 게이트가 동시 실행을 상한 안으로 묶고 거절하지 않음 · ping 면제)은
cysd `fatal1_admission_tests` 가 `cargo test --bin cysd` 에서 매 푸시 잰다.

## 미측정 범위(정직)

release 빌드는 재지 않았다(프레임이 작아 수정 전 임계가 더 높을 수 있다). 적색 재현은 debug 에서만
확인했다. 입장 상한 **단독**(multithread 가 되살아난 상태)의 방어력은 이 검체가 재지 않는다.

## 격리(라이브 무접촉)

HOME·CYS_SOCKET·CYS_STATE_DIR·CYS_PACK_DIR·CYS_CONFIG_DIR·CYS_PACK_CAPTURES_DIR 전부 mkdtemp,
CYS_NO_AUTOSTART=1 · CYS_NO_OFFICE_BRIDGE=1 · CYS_NO_PERSONAL_HOOK_MERGE=1, 트립와이어 cys·cysd 를
PATH 앞에(호출되면 hits.log 에 기록 후 rc 97 — 적중 수를 보고한다). 종료는 기록한 pid 에만
SIGTERM → 3s → SIGKILL(패턴 kill 금지). pane pid 도 기록한 것만 정리한다.

실행: python3 scripts/tests/test_cysd_dispatch_storm_e2e.py [--red-only]
      (CYS_TEST_CYSD 로 바이너리 지정 · CYS_STORM_K · CYS_STORM_SECS 로 규모 조정)
출력: PASS/FAIL/SKIP 행 · 실패 시 exit 1 · 종료 토큰 CYSD-DISPATCH-STORM-E2E-OK
"""
import errno
import json
import os
import resource
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

SELF = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(SELF))
K = int(os.environ.get("CYS_STORM_K", "500"))
SECS = float(os.environ.get("CYS_STORM_SECS", "10"))
BOOT_SOFT = 256
BOOT_TIMEOUT = 60.0
LOG_CAP_BYTES = 8 * 1024 * 1024
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""), flush=True)
    if not cond:
        fails.append(name)
    return cond


def skip(name, why):
    print("SKIP %s — %s" % (name, why), flush=True)


def find_cysd():
    """CYS_TEST_CYSD 우선, 없으면 워크트리 target/debug/cysd. 없으면 None(= SKIP, 통과 아님)."""
    env = os.environ.get("CYS_TEST_CYSD", "").strip()
    if env and os.path.isfile(env) and os.access(env, os.X_OK):
        return env
    c = os.path.join(REPO, "target", "debug", "cysd")
    return c if os.path.isfile(c) and os.access(c, os.X_OK) else None


class Sandbox:
    def __init__(self):
        self.real_home = os.path.expanduser("~")
        self.dir = tempfile.mkdtemp(prefix="ds-")
        self.home = os.path.join(self.dir, "home")
        self.trip = os.path.join(self.dir, "trip")
        for d in ("home", "home/.cys", "state", "pack", "config", "captures", "trip", "cwd"):
            os.makedirs(os.path.join(self.dir, d), exist_ok=True)
        self.hits = os.path.join(self.trip, "hits.log")
        for name in ("cys", "cysd"):
            p = os.path.join(self.trip, name)
            with open(p, "w") as f:
                f.write('#!/bin/sh\necho "TRIP $0 $*" >> "%s"\nexit 97\n' % self.hits)
            os.chmod(p, 0o755)
        self.sock = os.path.join(self.dir, "cysd.sock")
        real = os.path.realpath(self.sock)
        live_roots = [os.path.realpath(os.path.join(self.real_home, ".cys")),
                      os.path.realpath(os.path.join(self.real_home, ".local", "state", "cys"))]
        if not real.startswith(os.path.realpath(self.dir) + os.sep) or \
                any(real.startswith(r + os.sep) for r in live_roots):
            raise SystemExit("FATAL 소켓 경로가 샌드박스 밖이다: %s" % real)

    def env(self):
        e = dict(os.environ)
        for k in ("CYS_SURFACE_ID", "CYS_SEAT_TOKEN", "CYS_SURFACE_REF", "CYS_ROLE",
                  "CYS_NOFILE_RAISE", "JAVIS_NOFILE_RAISE", "AITERM_NOFILE_RAISE",
                  "RAYON_NUM_THREADS"):
            e.pop(k, None)
        e.update(HOME=self.home, CYS_SOCKET=self.sock,
                 CYS_STATE_DIR=os.path.join(self.dir, "state"),
                 CYS_PACK_DIR=os.path.join(self.dir, "pack"),
                 CYS_CONFIG_DIR=os.path.join(self.dir, "config"),
                 CYS_PACK_CAPTURES_DIR=os.path.join(self.dir, "captures"),
                 CYS_NO_AUTOSTART="1", CYS_NO_OFFICE_BRIDGE="1", CYS_NO_PERSONAL_HOOK_MERGE="1",
                 PYTHONDONTWRITEBYTECODE="1",
                 PATH=self.trip + os.pathsep + os.environ.get("PATH", ""))
        return e

    def trip_hits(self):
        try:
            with open(self.hits) as f:
                return [l.strip() for l in f if l.strip()]
        except OSError:
            return []

    def cleanup(self):
        shutil.rmtree(self.dir, ignore_errors=True)


class Daemon:
    """`/bin/sh -c 'ulimit -Sn 256 && exec "$0"' cysd` — stderr 는 PIPE 로 받아 치명 줄만 보관."""

    def __init__(self, cysd, sb):
        self.sb = sb
        self.fatal = []
        self.fdlimit = []
        self.total = 0
        self._lock = threading.Lock()
        self.p = subprocess.Popen(
            ["/bin/sh", "-c", 'ulimit -Sn %d && exec "$0"' % BOOT_SOFT, cysd],
            env=sb.env(), stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL)
        self.t = threading.Thread(target=self._reader, daemon=True)
        self.t.start()

    def _reader(self):
        for raw in iter(self.p.stdout.readline, b""):
            self.total += len(raw)
            if self.total > LOG_CAP_BYTES:
                continue                                 # 계속 읽어 파이프가 막히지 않게 한다(보관만 중단)
            line = raw.decode("utf-8", "replace").rstrip("\n")
            low = line.lower()
            with self._lock:
                if "overflowed its stack" in low or "fatal runtime error" in low or "panicked" in low:
                    self.fatal.append(line)
                elif "fd-limit" in line:
                    self.fdlimit.append(line)

    def snapshot(self):
        with self._lock:
            return list(self.fatal), list(self.fdlimit)

    def wait_ready(self):
        deadline = time.time() + BOOT_TIMEOUT
        while time.time() < deadline:
            if self.p.poll() is not None:
                return False
            if os.path.exists(self.sb.sock) and rpc_once(self.sb.sock, "system.ping", {}, 2.0)[0] == "ok":
                return True
            time.sleep(0.2)
        return False

    def stop(self):
        pid = self.p.pid
        if self.p.poll() is None:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.p.wait(timeout=3)
            except subprocess.TimeoutExpired:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.p.wait(timeout=5)
        gone = self.p.poll() is not None
        self.t.join(timeout=2)
        return gone


def _req(method, params):
    return (json.dumps({"id": 1, "method": method, "params": params}) + "\n").encode()


def read_reply(s, deadline):
    buf = b""
    while not buf.endswith(b"\n"):
        left = deadline - time.time()
        if left <= 0:
            return "timeout", None
        s.settimeout(left)
        try:
            chunk = s.recv(65536)
        except socket.timeout:
            return "timeout", None
        except OSError as e:
            return ("eof" if e.errno in (errno.ECONNRESET, errno.EPIPE) else "error:%s" % e.errno), None
        if not chunk:
            return "eof", None
        buf += chunk
    try:
        rec = json.loads(buf.decode())
    except ValueError:
        return "error:json", None
    return ("ok" if rec.get("ok") else "error:rpc"), rec


def rpc_on(s, method, params, timeout):
    try:
        s.sendall(_req(method, params))
    except OSError as e:
        return ("eof" if e.errno in (errno.EPIPE, errno.ECONNRESET) else "error:%s" % e.errno), None
    return read_reply(s, time.time() + timeout)


def rpc_once(sock_path, method, params, timeout):
    s = socket.socket(socket.AF_UNIX)
    try:
        s.settimeout(timeout)
        try:
            s.connect(sock_path)
        except OSError as e:
            return ("refused" if e.errno == errno.ECONNREFUSED else "connect:%s" % e.errno), None
        return rpc_on(s, method, params, timeout)
    finally:
        s.close()


def pid_alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def storm(sb, d):
    counts = {}
    lock = threading.Lock()
    stop_at = time.time() + SECS
    barrier = threading.Barrier(K)

    def worker():
        s = None
        try:
            barrier.wait(timeout=30)
        except threading.BrokenBarrierError:
            pass
        while time.time() < stop_at:
            try:
                if s is None:
                    s = socket.socket(socket.AF_UNIX)
                    s.settimeout(30)
                    s.connect(sb.sock)
                k, _ = rpc_on(s, "surface.list", {}, 30)
            except OSError as e:
                k = "oserr:%s" % e.errno
            if k != "ok" and s is not None:
                try:
                    s.close()
                except OSError:
                    pass
                s = None
            with lock:
                counts[k] = counts.get(k, 0) + 1
            if k != "ok":
                time.sleep(0.2)                          # 거절·사망 뒤 헛돌이 방지(판정은 counts 로)
        if s is not None:
            s.close()

    pings = []

    def pinger():
        time.sleep(1.0)                                  # 폭주가 차오른 뒤부터 표본
        while time.time() < stop_at - 0.5:
            t0 = time.time()
            k, _ = rpc_once(sb.sock, "system.ping", {}, 5.0)
            pings.append((k, round((time.time() - t0) * 1000.0, 1)))
            if d.p.poll() is not None:
                break
            time.sleep(0.5)

    ts = [threading.Thread(target=worker, daemon=True) for _ in range(K)]
    pt = threading.Thread(target=pinger, daemon=True)
    for t in ts:
        t.start()
    pt.start()
    peak_threads = 0
    while any(t.is_alive() for t in ts) and time.time() < stop_at + 40:
        if d.p.poll() is None:
            try:
                out = subprocess.run(["/bin/ps", "-M", "-p", str(d.p.pid)],
                                     capture_output=True, text=True, timeout=5).stdout
                peak_threads = max(peak_threads, len(out.splitlines()) - 1)
            except (OSError, subprocess.TimeoutExpired):
                pass
        time.sleep(0.5)
    pt.join(timeout=10)
    return counts, pings, peak_threads


def main():
    red_only = "--red-only" in sys.argv[1:]
    if sys.platform != "darwin":
        skip("전체", "darwin 전용 검체(launchd soft 256 재현) — 미측정, 통과 아님")
        print("CYSD-DISPATCH-STORM-E2E-OK")
        return 0
    cysd = find_cysd()
    if cysd is None:
        skip("전체", "cysd 바이너리 부재(cargo build --bin cysd 선행) — 미측정, 통과 아님")
        return 1
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    want = 10240 if hard == resource.RLIM_INFINITY else min(hard, 10240)
    if soft < want:
        try:
            resource.setrlimit(resource.RLIMIT_NOFILE, (want, hard))
        except (ValueError, OSError):
            pass
    if resource.getrlimit(resource.RLIMIT_NOFILE)[0] < K + 64:
        skip("전체", "검체 프로세스 fd 한도 < K+64 — 폭주를 만들 수 없다(미측정)")
        return 1
    print("cysd=%s K=%d SECS=%.0f boot_soft=%d red_only=%s" % (cysd, K, SECS, BOOT_SOFT, red_only), flush=True)
    sb = Sandbox()
    d = None
    pane_pids = []
    try:
        d = Daemon(cysd, sb)
        if not d.wait_ready():
            check("부팅", False, "rc=%r" % d.p.poll())
            return 1
        for _ in range(4):
            k, rec = rpc_once(sb.sock, "surface.create",
                              {"cmd": "/bin/cat", "cwd": os.path.join(sb.dir, "cwd"), "rows": 24, "cols": 80}, 10.0)
            if k == "ok":
                pid = (rec.get("result") or {}).get("pid")
                if isinstance(pid, int) and pid > 0:
                    pane_pids.append(pid)
        check("준비 — pane 4개 생성", len(pane_pids) == 4, "pids=%r" % pane_pids)
        counts, pings, peak_threads = storm(sb, d)
        time.sleep(0.5)
        rc = d.p.poll()
        alive = rc is None and rpc_once(sb.sock, "system.ping", {}, 5.0)[0] == "ok"
        fatal, fdlimit = d.snapshot()
        ok = counts.get("ok", 0)
        print("관측: rc=%r counts=%r peak_threads=%d fd-limit=%r" % (rc, counts, peak_threads, fdlimit), flush=True)
        print("ping 표본(%d): %r" % (len(pings), pings[:40]), flush=True)
        if fatal:
            print("치명 줄: %r" % fatal[:5], flush=True)
        if red_only:
            if alive:
                print("FAIL 적색 미재현 — 데몬 생존(ok=%d)" % ok)
                return 1
            print("RED-REPRODUCED 데몬 사망 rc=%r · 치명 줄 %d" % (rc, len(fatal)))
            return 0
        check("S1 데몬 생존 · 폭주 뒤 ping OK", alive, "rc=%r" % rc)
        check("S2 스택 오버플로·치명 런타임 오류 0줄", not fatal, repr(fatal[:3]))
        check("S3 surface.list ok ≥ K(%d)" % K, ok >= K, "ok=%d" % ok)
        check("S4 폭주 중 ping 표본 ≥3 · 전부 ok(입장 면제)",
              len(pings) >= 3 and all(k == "ok" for k, _ in pings), repr(pings[:10]))
        hits = sb.trip_hits()
        print("트립와이어 적중 %d%s" % (len(hits), (" — " + repr(hits[:3])) if hits else ""))
    finally:
        for pid in pane_pids:
            if pid_alive(pid):
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        if d is not None and not d.stop():
            print("WARN 데몬 pid %d 종료 확인 실패" % d.p.pid)
        sb.cleanup()
    if fails:
        print("FAILED %d: %r" % (len(fails), fails))
        return 1
    print("CYSD-DISPATCH-STORM-E2E-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
