#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""test_cysd_nofile_e2e.py — cysd fd soft 한도 자체 상향(E · RLIMIT_NOFILE) **로컬 수용 검체**.

## 무엇을 재는가

launchd·Finder·수동 기동 cysd 는 soft 256 을 물려받는다. 연결이 256 을 넘으면 XNU accept 가
EMFILE 로 대기 연결을 **버리고** 클라이언트는 EOF 를 받는다(서버는 요청을 읽지 않았으니 부작용
0 인 거짓 실패). E 수정은 기동 때(시작 락 뒤) soft 를 min(8192, hard, kern.maxfilesperproc) 로
올린다. 이 검체는 `ulimit -Sn 256` 셸로 **실데몬**을 띄워 그 효과를 소켓으로 실연한다.

  E1 동시 연결 320개를 모두 쥔 채 각각 system.ping → ok 320 · 나머지 0 · 'accept error' 0
  E2 stderr 에 '[cysd] fd-limit: soft 256 -> {기대값}' 정확히 1줄, 순서 = 마커 < fd-limit < lane
  E3 pane(surface.create)이 올린 soft·같은 hard 를 상속
  E4 롤백 두 채널(CYS_NOFILE_RAISE=0 · ~/.cys/nofile-raise-off) → E1 이 다시 적색 + 'disabled by …'
  E5 never-lower — `ulimit -Sn 9000` 기동은 'unchanged soft 9000', pane 도 9000

`--red-only` 는 E1 하나만 돌려 **적색 재현**(eof ≥1 ∧ accept error ≥1)을 확인한다 — 수정 전
바이너리(CYS_TEST_CYSD=<구 빌드>)로 TDD 적색 증거를 남길 때 쓴다(재현되면 exit 0).

## 왜 CI 레인 밖인가(lane-parity UNREGISTERED_OK 사유)

debug cysd 바이너리 빌드·실데몬·동시 소켓 320개·soft 256 기동 셸 재현이 필요하다. ci-branch 에는
`cargo build` 가 없고 pack-release 는 cys 만 빌드한다. 한도 상향·상속·never-lower·롤백 두 채널은
fdlimit.rs 의 자식 프로세스 검체 C1 이 `cargo test --bin cysd` 에서 매 푸시 잰다 — 이 파일이
덧붙이는 것은 소켓 EMFILE 실연뿐이다.

## 격리(라이브 무접촉)

HOME·CYS_SOCKET·CYS_STATE_DIR·CYS_PACK_DIR·CYS_CONFIG_DIR·CYS_PACK_CAPTURES_DIR 전부 mkdtemp,
CYS_NO_AUTOSTART=1 · CYS_NO_OFFICE_BRIDGE=1 · CYS_NO_PERSONAL_HOOK_MERGE=1, 트립와이어 cys·cysd 를
PATH 앞에(호출되면 hits.log 에 기록 후 rc 97 — 적중 수를 보고한다). 종료는 기록한 pid 에만
SIGTERM → 3s → SIGKILL(패턴 kill 금지). 단계마다 하드 타임아웃 10s, 부팅 대기 60s.

실행: python3 scripts/tests/test_cysd_nofile_e2e.py [--red-only]   (CYS_TEST_CYSD 로 바이너리 지정)
출력: PASS/FAIL/SKIP 행 · 실패 시 exit 1 · 종료 토큰 CYSD-NOFILE-E2E-OK
"""
import errno
import json
import os
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
N_CONN = 320
PHASE_TIMEOUT = 10.0
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


def host_cap():
    """macOS kern.maxfilesperproc(판독 실패 = None · clamp 없음)."""
    if sys.platform != "darwin":
        return None
    try:
        out = subprocess.run(["/usr/sbin/sysctl", "-n", "kern.maxfilesperproc"],
                             capture_output=True, text=True, timeout=5)
        v = int(out.stdout.strip())
        return v if v > 0 else None
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return None


def fmt_lim(v, resource):
    return "unlimited" if v == resource.RLIM_INFINITY else str(v)


def expected_soft(hard, cap, resource):
    w = 8192
    if hard != resource.RLIM_INFINITY:
        w = min(w, hard)
    if cap is not None:
        w = min(w, cap)
    return w


class Sandbox:
    def __init__(self):
        self.real_home = os.path.expanduser("~")
        self.dir = tempfile.mkdtemp(prefix="nf-")
        self.home = os.path.join(self.dir, "home")
        self.trip = os.path.join(self.dir, "trip")
        for d in ("home", "home/.cys", "state", "pack", "config", "captures", "trip"):
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

    def env(self, extra=None):
        e = dict(os.environ)
        for k in ("CYS_SURFACE_ID", "CYS_SEAT_TOKEN", "CYS_SURFACE_REF", "CYS_ROLE",
                  "CYS_NOFILE_RAISE", "JAVIS_NOFILE_RAISE", "AITERM_NOFILE_RAISE"):
            e.pop(k, None)
        e.update(HOME=self.home, CYS_SOCKET=self.sock,
                 CYS_STATE_DIR=os.path.join(self.dir, "state"),
                 CYS_PACK_DIR=os.path.join(self.dir, "pack"),
                 CYS_CONFIG_DIR=os.path.join(self.dir, "config"),
                 CYS_PACK_CAPTURES_DIR=os.path.join(self.dir, "captures"),
                 CYS_NO_AUTOSTART="1", CYS_NO_OFFICE_BRIDGE="1", CYS_NO_PERSONAL_HOOK_MERGE="1",
                 PATH=self.trip + os.pathsep + os.environ.get("PATH", ""))
        e.update(extra or {})
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
    """`/bin/sh -c 'ulimit -Sn <soft> && exec "$0"' cysd` — stderr 는 PIPE 로 받아 필요한 줄만 보관."""

    def __init__(self, cysd, sb, soft, extra_env=None):
        self.sb = sb
        self.lines = []           # marker·fd-limit·lane 줄 + 앞 200줄
        self.accept_errors = 0
        self.total = 0
        self.overflow = False
        self._lock = threading.Lock()
        self.p = subprocess.Popen(
            ["/bin/sh", "-c", 'ulimit -Sn %s && exec "$0"' % soft, cysd],
            env=sb.env(extra_env), stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL)
        self.t = threading.Thread(target=self._reader, daemon=True)
        self.t.start()

    def _reader(self):
        n = 0
        for raw in iter(self.p.stdout.readline, b""):
            self.total += len(raw)
            if self.total > LOG_CAP_BYTES:
                self.overflow = True
                break
            line = raw.decode("utf-8", "replace").rstrip("\n")
            with self._lock:
                if "accept error" in line:
                    self.accept_errors += 1
                elif n < 200 or line.startswith("[cysd] v") or "fd-limit" in line or "[cysd] lane:" in line:
                    self.lines.append(line)
            n += 1

    def snapshot(self):
        with self._lock:
            return list(self.lines), self.accept_errors

    def wait_ready(self):
        deadline = time.time() + BOOT_TIMEOUT
        while time.time() < deadline:
            if self.p.poll() is not None:
                return False
            if os.path.exists(self.sb.sock) and ping(self.sb.sock, 2.0) == "ok":
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


def _req(method, params=None):
    return (json.dumps({"id": 1, "method": method, "params": params or {}}) + "\n").encode()


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


def ping(sock_path, timeout):
    try:
        s = socket.socket(socket.AF_UNIX)
        s.settimeout(timeout)
        s.connect(sock_path)
    except OSError:
        return "connect_refused"
    try:
        s.sendall(_req("system.ping"))
        return read_reply(s, time.time() + timeout)[0]
    except OSError:
        return "eof"
    finally:
        s.close()


def rpc(sock_path, method, params, timeout=5.0):
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(timeout)
    s.connect(sock_path)
    try:
        s.sendall(_req(method, params))
        return read_reply(s, time.time() + timeout)
    finally:
        s.close()


def connect_retry(sock_path, budget=2.0):
    deadline = time.time() + budget
    while True:
        s = socket.socket(socket.AF_UNIX)
        try:
            s.settimeout(2.0)
            s.connect(sock_path)
            return s
        except OSError as e:
            s.close()
            if e.errno in (errno.ECONNREFUSED, errno.EAGAIN, errno.ENOENT) and time.time() < deadline:
                time.sleep(0.01)
                continue
            return None


def flood(d, stop_on_red):
    """E1 — N_CONN 개를 순차로 열어 모두 쥔 채 각각 ping. 분류 = ok·connect_refused·eof·timeout(+기타)."""
    res = {"ok": 0, "connect_refused": 0, "eof": 0, "timeout": 0, "other": 0}
    conns = []
    t0 = time.time()
    try:
        for _ in range(N_CONN):
            if time.time() - t0 > PHASE_TIMEOUT:
                break
            s = connect_retry(d.sb.sock)
            if s is None:
                res["connect_refused"] += 1
            else:
                conns.append(s)
        t1 = time.time()
        for s in conns:
            if time.time() - t1 > PHASE_TIMEOUT:
                res["timeout"] += 1
                continue
            try:
                s.sendall(_req("system.ping"))
                k = read_reply(s, min(time.time() + 5.0, t1 + PHASE_TIMEOUT))[0]
            except OSError:
                k = "eof"
            res[k if k in res else "other"] += 1
            if stop_on_red and res["eof"] >= 1 and d.snapshot()[1] >= 1:
                break                                    # 적색 확인 즉시 중단(G-01 스핀 봉쇄)
    finally:
        for s in conns:
            try:
                s.close()
            except OSError:
                pass
    res["attempted"] = sum(v for k, v in res.items() if k != "attempted")
    return res


def lines_with(lines, needle):
    return [l for l in lines if needle in l]


def pane_limits(sb, sock_path):
    """E3 — surface.create 로 띄운 pane 이 본 soft·hard(ulimit -Sn / -Hn)."""
    out = os.path.join(sb.dir, "nofile.txt")
    cmd = "/bin/sh -c 'ulimit -Sn > %s; ulimit -Hn >> %s'" % (out, out)
    kind, rec = rpc(sock_path, "surface.create", {"cmd": cmd, "cwd": sb.dir, "rows": 24, "cols": 80})
    if kind != "ok":
        return None, "surface.create %s %r" % (kind, rec)
    deadline = time.time() + 5.0
    while time.time() < deadline:
        try:
            with open(out) as f:
                vals = f.read().split()
            if len(vals) >= 2:
                return vals[:2], ""
        except OSError:
            pass
        time.sleep(0.05)
    return None, "5s 안에 pane 결과 파일이 생기지 않았다"


def run_case(cysd, label, soft, extra_env=None, home_marker=False):
    sb = Sandbox()
    if home_marker:
        open(os.path.join(sb.home, ".cys", "nofile-raise-off"), "w").close()
    d = Daemon(cysd, sb, soft, extra_env)
    try:
        if not d.wait_ready():
            lines, _ = d.snapshot()
            check("%s 부팅" % label, False, "60s 안에 ping OK 없음 — 로그 앞부분: %r" % lines[:5])
            return None
        return sb, d
    except BaseException:
        d.stop()
        sb.cleanup()
        raise


def finish(sb, d, label):
    gone = d.stop()
    hits = sb.trip_hits()
    check("%s 종료·청소(pid 소멸)" % label, gone)
    check("%s 트립와이어 적중 0" % label, not hits, repr(hits[:3]))
    sb.cleanup()


def main():
    if sys.platform == "win32":
        print("SKIP 전체 — unix 전용(RLIMIT_NOFILE)")
        return 0
    import resource
    red_only = "--red-only" in sys.argv[1:]
    cysd = find_cysd()
    if cysd is None:
        print("SKIP 전체 — cysd 바이너리 부재(미측정 · 통과 아님)")
        return 0
    soft0, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    cap = host_cap()
    client_soft = 4096 if hard == resource.RLIM_INFINITY else min(hard, 4096)
    if cap is not None:
        client_soft = min(client_soft, cap)
    if soft0 < client_soft:
        resource.setrlimit(resource.RLIMIT_NOFILE, (client_soft, hard))
    want = expected_soft(hard, cap, resource)
    print("cysd=%s · 테스트 hard=%s · cap=%s · 기대 soft=%d" % (cysd, fmt_lim(hard, resource), cap, want))

    if red_only:
        case = run_case(cysd, "RED", "256")
        if case is None:
            return 1
        sb, d = case
        try:
            r = flood(d, stop_on_red=True)
            _, acc = d.snapshot()
            print("RED 관측: %r · accept error %d" % (r, acc))
            check("RED 적색 재현(eof ≥1 ∧ 'accept error' ≥1)", r["eof"] >= 1 and acc >= 1)
        finally:
            finish(sb, d, "RED")
        print("CYSD-NOFILE-E2E-RED-REPRODUCED" if not fails else "FAILED %r" % fails)
        return 1 if fails else 0

    # ── E1·E2·E3: soft 256 기동 → 상향 ──────────────────────────────────────
    case = run_case(cysd, "E1", "256")
    if case is not None:
        sb, d = case
        try:
            r = flood(d, stop_on_red=False)
            lines, acc = d.snapshot()
            print("E1 관측: %r · accept error %d" % (r, acc))
            check("E1 동시 연결 %d 전부 ok" % N_CONN,
                  r["ok"] == N_CONN and r["attempted"] == N_CONN
                  and r["eof"] == r["timeout"] == r["connect_refused"] == r["other"] == 0 and acc == 0,
                  repr(r))
            fl = lines_with(lines, "[cysd] fd-limit:")
            exact = "[cysd] fd-limit: soft 256 -> %d" % want
            check("E2 fd-limit 줄 정확히 1개 · 'soft 256 -> %d'" % want,
                  len(fl) == 1 and fl[0].startswith(exact), repr(fl))
            try:
                im = next(i for i, l in enumerate(lines) if l.startswith("[cysd] v"))
                ifl = next(i for i, l in enumerate(lines) if "[cysd] fd-limit:" in l)
                ilane = next(i for i, l in enumerate(lines) if l.startswith("[cysd] lane:"))
                check("E2 순서 마커 < fd-limit < lane(락 뒤 위치)", im < ifl < ilane, "%d %d %d" % (im, ifl, ilane))
            except StopIteration:
                check("E2 순서 마커 < fd-limit < lane(락 뒤 위치)", False, "줄 누락: %r" % lines[:8])
            vals, why = pane_limits(sb, sb.sock)
            check("E3 pane 상속 soft=%d · hard=%s" % (want, fmt_lim(hard, resource)),
                  vals == [str(want), fmt_lim(hard, resource)], "%r %s" % (vals, why))
        finally:
            finish(sb, d, "E1")

    # ── E4: 롤백 두 채널 → 적색 복귀 ─────────────────────────────────────────
    for label, extra, marker, needle in (
            ("E4-env", {"CYS_NOFILE_RAISE": "0"}, False, "disabled by CYS_NOFILE_RAISE"),
            ("E4-marker", None, True, "disabled by ~/.cys/nofile-raise-off")):
        case = run_case(cysd, label, "256", extra, marker)
        if case is None:
            continue
        sb, d = case
        try:
            r = flood(d, stop_on_red=True)
            lines, acc = d.snapshot()
            print("%s 관측: %r · accept error %d" % (label, r, acc))
            check("%s 롤백 로그 '%s'" % (label, needle), bool(lines_with(lines, needle)),
                  repr(lines_with(lines, "fd-limit")))
            check("%s 롤백이면 E1 이 다시 적색(eof ≥1 ∧ accept error ≥1)" % label,
                  r["eof"] >= 1 and acc >= 1, repr(r))
        finally:
            finish(sb, d, label)

    # ── E5: never-lower ─────────────────────────────────────────────────────
    ceiling = hard if cap is None else min(hard, cap)
    if ceiling < 9000:
        check("E5 전제(호스트 min(hard,cap) ≥ 9000)", False, "hard=%s cap=%s" % (fmt_lim(hard, resource), cap))
    else:
        case = run_case(cysd, "E5", "9000")
        if case is not None:
            sb, d = case
            try:
                lines, _ = d.snapshot()
                check("E5 'unchanged soft 9000'", bool(lines_with(lines, "fd-limit: unchanged soft 9000")),
                      repr(lines_with(lines, "fd-limit")))
                vals, why = pane_limits(sb, sb.sock)
                check("E5 pane 도 9000", bool(vals) and vals[0] == "9000", "%r %s" % (vals, why))
            finally:
                finish(sb, d, "E5")

    if fails:
        print("FAILED %d: %r" % (len(fails), fails))
        return 1
    print("CYSD-NOFILE-E2E-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
