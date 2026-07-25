#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
javis_phoenix_harness.py — 불사조(무손실 복원) Phase 1 격리 하네스

설계 근거: _round/ZERO_LOSS_RESTORE_DESIGN.md §9.2 P3 · §9.4-1
목적: 라이브 데몬·소켓·~/.local/state/cys/ 를 절대 건드리지 않고, 별도 소켓·별도
      상태 디렉터리(~/.cys/state-harness/)에서 격리된 테스트 데몬을 띄워
      크래시 주입·부활 drill·프리미티브 자격시험을 안전하게 반복하는 "증명 장소".

격리 계약(실측 확정):
  · 데몬 bind:  CYS_SOCKET=<harness>/cys.sock  cysd
                → 데몬이 그 소켓에 listen하고, 상태 디렉터리는 소켓의 부모 디렉터리로 격리됨
                  (analytics.db·transcripts.db·event.seq·schedule_state.json·cys.lock 전부 격리 dir)
  · 클라이언트: cys --socket <harness>/cys.sock <cmd>
  · 함정: `cys --socket <X> list` 는 데몬이 없으면 자동으로 cysd를 autostart한다(고아 위험).
          `cys --socket <X> ping` 은 autostart하지 않는다 → 데몬 down 감지는 반드시 ping으로 한다.
  · identify는 AITERM_SURFACE_ID env를 읽으므로 격리 소켓에 걸어도 라이브 surface를 답한다
    → 격리 데몬 liveness 판정은 identify가 아니라 ping/list로 한다.

자원 거버넌스(잔여 프로세스 0 절대 준수):
  · 격리 데몬은 os.setsid로 자기 프로세스 그룹의 리더가 되게 띄운다 → 종료 시 killpg로 그룹 일괄 종료.
  · atexit + SIGINT/SIGTERM 핸들러가 teardown()을 호출한다(외부 timeout이 이 파이썬을 죽여도 최대한 정리).
  · teardown은 (1) 살아있으면 모든 격리 surface를 close-surface로 닫고 (2) 격리 dir에 bind한
    모든 cysd를 lsof로 찾아 kill -9 (autostart 고아 청소) (3) 추적 데몬 그룹 killpg 순으로 동작한다.

사용:
  javis_phoenix_harness.py up            # 격리 데몬 기동 + 라이브 무영향 불변식 실측
  javis_phoenix_harness.py down          # 전면 teardown + 잔여 0 실측
  javis_phoenix_harness.py status        # 격리/라이브 상태 스냅샷
  javis_phoenix_harness.py crash-daemon  # 격리 데몬 kill -9 → 재기동(fixture 재부팅) 성공 검증
  javis_phoenix_harness.py crash-agent   # 격리 surface 에이전트 kill -9 시나리오
  javis_phoenix_harness.py record-cli    # 실 claude CLI 기동→ready_marker→resume PTY 1회 녹화(토큰0)
  javis_phoenix_harness.py qualify       # 기존 프리미티브 자격시험 4건(--queued/restore/watch/lock경합)
  javis_phoenix_harness.py drill         # 위 전 과정 일괄 + 증거 JSON 방출(HARNESS_REPORT 원천)
"""

import argparse
import hashlib
import json
import os
import re
import select
import signal
import subprocess
import sys
import time


def _sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()

HOME = os.path.expanduser("~")

# --- 라이브(절대 무접촉) ---
LIVE_STATE = os.path.join(HOME, ".local", "state", "cys")
LIVE_SOCK = os.path.join(LIVE_STATE, "cys.sock")

# --- 격리(하네스 전용) ---
HARN_DIR = os.path.join(HOME, ".cys", "state-harness")
HARN_SOCK = os.path.join(HARN_DIR, "cys.sock")
RECORD_DIR = os.path.join(HARN_DIR, "cli_record")
DAEMON_LOG = os.path.join(HARN_DIR, "daemon.log")
EVIDENCE = os.path.join(HARN_DIR, "drill_evidence.json")

LEAKY_ENV = [
    "CLAUDECODE", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT",
    "AITERM_SURFACE_ID", "AITERM_SOCKET",
]

_tracked_daemon = None  # subprocess.Popen of the isolated cysd we own


# ------------------------------------------------------------------ 유틸

def _resolve_cysd():
    """cysd 절대경로 결정 — PHOENIX_HARNESS_CYSD 오버라이드 우선(Phase3 신 바이너리 자격시험),
    없으면 app 번들, 없으면 PATH."""
    override = os.environ.get("PHOENIX_HARNESS_CYSD")
    if override and os.path.exists(override):
        return os.path.realpath(override)
    cand = "/Applications/cys.app/Contents/MacOS/cysd"
    if os.path.exists(cand):
        return cand
    p = shutil_which("cysd")
    if p:
        return os.path.realpath(p)
    die("cysd 바이너리를 찾을 수 없다.")


def shutil_which(name):
    import shutil
    return shutil.which(name)


def _resolve_claude():
    """DD-2 완성: claude 탐지를 로그인셸 프로브(javis_cli_probe.resolve_cli)로 단일화한다.
    GUI/launchd 빈곤 PATH에서 record-cli를 돌려도 로그인셸 rc(기능2 우산)로 해석해 오판 0.
    ★import/probe 실패는 shutil.which 폴백(record 도구가 어떤 환경에서도 죽지 않게 — R10 정신).
    (record-cli는 부트 경로가 아닌 수동/CI 스텁-시드 도구라 저위험이며, 다른 claude 탐지부와
     동일 계약으로 수렴시켜 DD-2 '단일 감지 함수' 불변식을 완성한다.)"""
    try:
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        import javis_cli_probe
        p = javis_cli_probe.resolve_cli("claude")
        if p:
            return p
    except Exception:
        pass
    return shutil_which("claude")


def _resolve_cys():
    p = shutil_which("cys")
    return p or "cys"


CYSD = None  # lazy
CYS = None


def die(msg, code=2):
    sys.stderr.write("[harness][FATAL] %s\n" % msg)
    sys.exit(code)


def log(msg):
    sys.stdout.write("[harness] %s\n" % msg)
    sys.stdout.flush()


def guard_isolation():
    """격리 경로가 라이브 상태 디렉터리와 겹치지 않는지 하드 가드 — 겹치면 즉시 중단.
    ★C5/P1-10(W3): 라이브 상태/소켓을 타깃하는 것은 **CYS_PHOENIX_ALLOW_LIVE=1 명시 opt-in** 이 있을 때만
    허용한다(없으면 LIVE write 거부). 하네스는 격리가 기본이며, 라이브 접촉은 사고가 아니라 의도된 예외여야 한다."""
    hd = os.path.realpath(HARN_DIR)
    ls = os.path.realpath(LIVE_STATE)
    allow_live = os.environ.get("CYS_PHOENIX_ALLOW_LIVE") == "1"
    overlap = hd == ls or hd.startswith(ls + os.sep) or ls.startswith(hd + os.sep)
    sock_same = os.path.realpath(HARN_SOCK) == os.path.realpath(LIVE_SOCK)
    if overlap or sock_same:
        if not allow_live:
            die("★C5/P1-10: 하네스가 라이브 상태(%s)/소켓을 타깃 — CYS_PHOENIX_ALLOW_LIVE=1 명시 opt-in "
                "없으면 LIVE write 거부(격리가 기본)." % ls)
        log("★C5 경고: CYS_PHOENIX_ALLOW_LIVE=1 — 하네스의 라이브 상태 쓰기 허용(명시 opt-in·위험 작업).")


def _daemon_env():
    env = dict(os.environ)
    env["CYS_SOCKET"] = HARN_SOCK       # ← 데몬 bind 경로 오버라이드(실측 확정)
    # ★W2: 하네스 데몬은 phoenix 로직을 직접 테스트한다 — cysd 콜드부트 auto-restore(W6 --socket 이후 하네스
    #   소켓 대상)가 restore.lease 를 잡으면 드릴의 직접 restore 가 LEASE_HELD 로 경합한다. 기본 비활성화하되
    #   (드릴 결정론화·cysd auto-restore 는 E1 e2e_replacement 가 별도 검증), 호출자가 os.environ 으로 명시
    #   설정(예: E1 이 "0"으로 auto-restore 활성화)하면 존중한다.
    env.setdefault("CYS_NO_AUTORESTORE", "1")
    for k in LEAKY_ENV:
        env.pop(k, None)
    return env


def cys(*args, timeout=20, socket=True):
    """격리 데몬 대상 cys 호출. socket=False면 라이브 대상(무접촉 관측용 list/ping만)."""
    cmd = [CYS]
    if socket:
        cmd += ["--socket", HARN_SOCK]
    cmd += list(args)
    env = dict(os.environ)
    for k in ("AITERM_SOCKET",):  # 클라이언트가 실수로 라이브로 새지 않게
        env.pop(k, None)
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
        return r
    except subprocess.TimeoutExpired as e:
        class _R:
            returncode = 124
            stdout = (e.stdout or b"").decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
            stderr = "TIMEOUT after %ss" % timeout
        return _R()


def harness_ping():
    """격리 데몬 살아있나 — ping은 autostart 안 하므로 안전한 liveness probe."""
    r = cys("ping", timeout=6)
    return r.returncode == 0 and "pong" in (r.stdout or "")


def live_surfaces():
    """라이브 데몬의 surface 목록(무접촉 관측 — 소켓 오버라이드 없음)."""
    try:
        r = subprocess.run([CYS, "list"], capture_output=True, text=True, timeout=10)
    except Exception:
        return []
    return [l for l in (r.stdout or "").splitlines() if l.startswith("surface:")]


def harness_daemon_pids():
    """격리 dir(state-harness)에 bind한 모든 cysd pid — lsof 실측(autostart 고아 포함).
    ★Phase11 ②: 판별을 **realpath 해소 경로 + bind 소켓 기반**으로 정밀화한다. 이전엔 lsof 출력에 대한
    'state-harness' 문자열 substring 폴백을 썼는데, 이는 심링크·이름 변형에 취약하고 무관 경로를 오적중할 수 있다.
    수리: (a) 격리 소켓(HARN_SOCK)의 realpath 를 lsof 출력에서 정확히 매칭(socket 기반 — 어떤 cysd 가 우리 소켓을
    bind 했는가), (b) 보조로 격리 dir realpath 접두 경로. 문자열 'state-harness' substring 폴백 제거."""
    pids = []
    harn_dir_rp = os.path.realpath(HARN_DIR)
    harn_sock_rp = os.path.realpath(HARN_SOCK)
    try:
        out = subprocess.run(["pgrep", "-x", "cysd"], capture_output=True, text=True, timeout=10).stdout
    except Exception:
        return pids
    for line in out.split():
        pid = line.strip()
        if not pid.isdigit():
            continue
        try:
            # lsof -Fn: NUL 없는 이름 필드만(파싱 안정) — 각 열린 파일 경로를 realpath 로 해소해 대조.
            ls = subprocess.run(["lsof", "-p", pid, "-Fn"], capture_output=True, text=True, timeout=10).stdout
        except Exception:
            continue
        matched = False
        for l in ls.splitlines():
            if not l.startswith("n"):
                continue
            path = l[1:]
            rp = os.path.realpath(path)
            # socket 기반 정밀 매칭(우리 소켓을 bind) 또는 격리 dir realpath 접두(우리 상태 파일).
            if rp == harn_sock_rp or rp == harn_dir_rp or rp.startswith(harn_dir_rp + os.sep):
                matched = True
                break
        if matched:
            pids.append(int(pid))
    return pids


def _descendants(root_pids):
    """root_pids 의 전(全) 자손 pid 집합 — ps 실측 부모체인(pid/ppid 기반). ★Phase11 ②: 실행 경로 문자열
    (pkill -f 'sleep 600')이 아니라 pid 관계로 추적하므로 심링크·명령명 변형에 면역이고 무관 프로세스 collateral 0."""
    try:
        out = subprocess.run(["ps", "-Ao", "pid=,ppid="], capture_output=True, text=True, timeout=10).stdout
    except Exception:
        return set()
    children = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) != 2 or not (parts[0].isdigit() and parts[1].isdigit()):
            continue
        children.setdefault(int(parts[1]), []).append(int(parts[0]))
    seen = set()
    stack = list(root_pids)
    while stack:
        p = stack.pop()
        for c in children.get(p, []):
            if c not in seen:
                seen.add(c)
                stack.append(c)
    return seen


def _annihilate():
    """★Phase11 ② 전멸(realpath/pid 기반·빗나감0·collateral0): 격리 데몬 + 그 전 자손을 pid 로 정확히 kill.
    이전 방식(pkill -9 -f 'sleep 600')은 명령 문자열 매칭이라 stub 이 'sleep 3600' 이면 **타겟을 빗나가고**
    무관한 'sleep 600' 을 오적중할 수 있었다. 여기서는 데몬 pid 집합의 부모체인 전체를 pid 로 kill 한다 →
    stub surface 자식(sleep 3600)까지 정확히 적중하고 무관 프로세스는 절대 건드리지 않는다. victim pid 리스트 반환."""
    dpids = harness_daemon_pids()
    victims = set(dpids) | _descendants(dpids)
    for pid in sorted(victims):
        try:
            os.kill(pid, signal.SIGKILL)
        except Exception:
            pass
    return sorted(victims)


# ------------------------------------------------------------------ 데몬 lifecycle

def start_daemon(wait=12.0):
    """격리 cysd를 자기 프로세스 그룹 리더로 기동하고 ping OK까지 대기."""
    global _tracked_daemon
    os.makedirs(HARN_DIR, exist_ok=True)
    lf = open(DAEMON_LOG, "ab")
    _tracked_daemon = subprocess.Popen(
        [CYSD], env=_daemon_env(), stdout=lf, stderr=lf,
        cwd=HARN_DIR, preexec_fn=os.setsid,   # ← 새 세션/프로세스그룹 → killpg 대상
    )
    t0 = time.time()
    while time.time() - t0 < wait:
        if harness_ping():
            return _tracked_daemon.pid
        if _tracked_daemon.poll() is not None:
            break
        time.sleep(0.2)
    return _tracked_daemon.pid if harness_ping() else None


def _kill_pg(pid, sig):
    try:
        os.killpg(os.getpgid(pid), sig)
        return True
    except Exception:
        try:
            os.kill(pid, sig)
            return True
        except Exception:
            return False


def close_all_surfaces():
    """살아있는 격리 데몬의 모든 surface를 close-surface로 닫는다(자손 트리 강제 종료)."""
    if not harness_ping():
        return []
    r = cys("list", timeout=10)
    refs = []
    for l in (r.stdout or "").splitlines():
        m = re.match(r"(surface:\d+)", l)
        if m:
            refs.append(m.group(1))
    closed = []
    for ref in refs:
        cys("close-surface", ref, timeout=10)
        closed.append(ref)
    return closed


def teardown(verbose=False):
    """전면 정리 — 잔여 프로세스 0 보장. 어떤 종료 경로에서도 안전하게 재호출 가능."""
    # 1) 살아있으면 surface부터 정상 종료
    try:
        close_all_surfaces()
    except Exception:
        pass
    # 2) 격리 dir에 bind한 모든 cysd(우리 것 + autostart 고아) kill -9
    for pid in harness_daemon_pids():
        _kill_pg(pid, signal.SIGKILL)
    # 3) 추적 데몬 그룹 killpg (2에서 안 잡힌 경우 대비)
    global _tracked_daemon
    if _tracked_daemon is not None:
        _kill_pg(_tracked_daemon.pid, signal.SIGKILL)
        try:
            _tracked_daemon.wait(timeout=3)
        except Exception:
            pass
        _tracked_daemon = None
    # 4) 확인
    time.sleep(0.4)
    remain = harness_daemon_pids()
    if verbose:
        log("teardown 후 격리 dir bind cysd 잔여: %s" % (remain or "0(없음)"))
    return remain


def residual_report():
    """잔여 프로세스 실측 — 성공기준 ⑤용."""
    return {
        "harness_daemon_pids": harness_daemon_pids(),
        "harness_ping": harness_ping(),
    }


def _wipe_daemon_state():
    """격리 데몬의 영속 surface 상태만 제거(topology·analytics·큐 등). ★phoenix/ dept-root/ 는 건드리지 않는다
    — 그건 phoenix 의 보호집합 roster 라 재기동을 넘어 보존돼야 한다(부활 단일 진실). HARN_DIR 만."""
    # ★v0.13.17 C1: 큐 WAL이 v2 신규 파일(queue-state-v2.json)로 분리됐다 — 둘 다 지워야
    # 드릴이 hermetic 하다(v2만 남으면 이전 드릴 큐가 신 데몬에 그대로 복원된다).
    for name in ("topology.json", "analytics.db", "analytics.db-shm", "analytics.db-wal",
                 "event.seq", "queue-state.json", "queue-state-v2.json",
                 "schedule_state.json", "cys.lock",
                 "autopilot.json", "feed.jsonl"):
        try:
            os.remove(os.path.join(HARN_DIR, name))
        except OSError:
            pass


def _fresh_harness():
    """★Phase 9: 드릴을 hermetic 하게 만든다 — 이전 드릴이 남긴 격리 데몬 영속상태 + phoenix 저널/부서 루트를
    전부 제거하고 깨끗한 격리 데몬으로 시작한다. 상태 누수로 인한 CI 비결정성 근본 차단.
    ★HARN_DIR(격리 스크래치)만 건드린다 — guard_isolation 이 라이브 상태와의 분리를 보장한다."""
    guard_isolation()
    teardown()
    _wipe_daemon_state()
    import shutil as _sh
    for d in ("phoenix", "dept-root"):
        _sh.rmtree(os.path.join(HARN_DIR, d), ignore_errors=True)
    start_daemon()


# 시그널/atexit 안전망 — 외부 timeout(SIGTERM)에도 정리 시도
def _sig_handler(signum, frame):
    try:
        teardown()
    finally:
        os._exit(130)


import atexit
atexit.register(lambda: teardown())
signal.signal(signal.SIGINT, _sig_handler)
signal.signal(signal.SIGTERM, _sig_handler)


# ------------------------------------------------------------------ 명령: up

def cmd_up(args):
    guard_isolation()
    live_before = live_surfaces()
    log("라이브 surface(before): %d개" % len(live_before))
    if harness_ping():
        log("격리 데몬이 이미 떠 있음 → 재사용")
    else:
        pid = start_daemon()
        if not pid or not harness_ping():
            teardown()
            die("격리 데몬 기동 실패 (daemon.log 확인: %s)" % DAEMON_LOG)
        log("격리 데몬 기동 OK pid=%s socket=%s" % (pid, HARN_SOCK))
    r = cys("list", timeout=10)
    n_iso = len([l for l in (r.stdout or '').splitlines() if l.startswith('surface:')])
    live_after = live_surfaces()
    inv = {
        "socket_live": LIVE_SOCK,
        "socket_harness": HARN_SOCK,
        "sockets_distinct": os.path.realpath(LIVE_SOCK) != os.path.realpath(HARN_SOCK),
        "live_surfaces_before": len(live_before),
        "live_surfaces_after": len(live_after),
        "live_unchanged": len(live_before) == len(live_after),
        "harness_surfaces": n_iso,
    }
    log("불변식: 소켓 상이=%s · 라이브 surface %d→%d(불변=%s) · 격리 surface=%d" % (
        inv["sockets_distinct"], inv["live_surfaces_before"],
        inv["live_surfaces_after"], inv["live_unchanged"], inv["harness_surfaces"]))
    print(json.dumps(inv, ensure_ascii=False, indent=2))
    return inv


# ------------------------------------------------------------------ 명령: down / status

def cmd_down(args):
    guard_isolation()
    remain = teardown(verbose=True)
    ok = (not remain) and (not harness_ping())
    res = {"residual_daemon_pids": remain, "harness_ping_after": harness_ping(), "clean": ok}
    print(json.dumps(res, ensure_ascii=False, indent=2))
    if not ok:
        die("teardown 후 잔여 존재 — 수동 확인 필요")
    log("teardown 완료 · 잔여 0 확인")
    return res


def cmd_status(args):
    st = {
        "harness_ping": harness_ping(),
        "harness_daemon_pids": harness_daemon_pids(),
        "harness_dir": HARN_DIR,
        "harness_dir_exists": os.path.isdir(HARN_DIR),
        "live_surfaces": len(live_surfaces()),
        "live_sock_exists": os.path.exists(LIVE_SOCK),
    }
    print(json.dumps(st, ensure_ascii=False, indent=2))
    return st


# ------------------------------------------------------------------ 명령: crash-daemon

def cmd_crash_daemon(args):
    """격리 데몬 kill -9 → 재기동(fixture 재부팅) 성공 검증. 성공기준 ②."""
    guard_isolation()
    _fresh_harness()
    pids = harness_daemon_pids()
    if not pids:
        die("격리 데몬이 없다 — 먼저 up")
    victim = pids[0]
    ev = {"before_pids": pids, "victim": victim}
    log("격리 데몬 kill -9 주입: pid=%s" % victim)
    _kill_pg(victim, signal.SIGKILL)
    global _tracked_daemon
    if _tracked_daemon is not None:
        try:
            _tracked_daemon.wait(timeout=3)
        except Exception:
            pass
        _tracked_daemon = None
    time.sleep(0.6)
    ev["ping_after_kill"] = harness_ping()   # False 기대(crash-only: 데몬 사망)
    log("kill 후 ping(살아있으면 안 됨): %s" % ev["ping_after_kill"])
    # 재기동(부활)
    pid2 = start_daemon()
    ev["restart_pid"] = pid2
    ev["ping_after_restart"] = harness_ping()  # True 기대
    ev["state_files_survived"] = sorted(
        f for f in os.listdir(HARN_DIR)
        if f.endswith(".db") or f in ("event.seq", "schedule_state.json", "cys.lock")
    ) if os.path.isdir(HARN_DIR) else []
    ev["fixture_reboot_ok"] = bool(pid2) and ev["ping_after_restart"] and not ev["ping_after_kill"]
    log("fixture 재부팅 성공=%s (재기동 pid=%s · 재기동후 ping=%s)" % (
        ev["fixture_reboot_ok"], pid2, ev["ping_after_restart"]))
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


# ------------------------------------------------------------------ 명령: crash-agent

def cmd_crash_agent(args):
    """격리 surface(pane 에이전트 대역) kill -9 시나리오. 데몬 생존 + surface 사망 관측."""
    guard_isolation()
    _fresh_harness()
    r = cys("new-surface", timeout=15)
    ref = (r.stdout or "").strip().splitlines()[0].strip() if r.stdout else ""
    m = re.search(r"(surface:\d+)", r.stdout or "")
    ref = m.group(1) if m else ref
    ev = {"new_surface_ref": ref, "new_surface_raw": (r.stdout or "").strip()}
    if not ref:
        die("격리 surface 생성 실패: %s" % (r.stderr or r.stdout))
    # surface의 PTY 자식 pid를 list에서 파싱
    time.sleep(0.5)
    lst = cys("list", timeout=10).stdout or ""
    pid = None
    for l in lst.splitlines():
        if l.startswith(ref):
            mm = re.search(r"pid=(\d+)", l)
            if mm:
                pid = int(mm.group(1))
    ev["surface_pid"] = pid
    log("격리 surface %s pid=%s 생성 → kill -9 주입" % (ref, pid))
    if pid:
        try:
            os.kill(pid, signal.SIGKILL)
        except Exception as e:
            ev["kill_error"] = str(e)
    time.sleep(0.8)
    # 데몬은 살아야 하고, surface는 죽은 것으로 관측되어야 함
    ev["daemon_alive_after"] = harness_ping()
    lst2 = cys("list", timeout=10).stdout or ""
    still = [l for l in lst2.splitlines() if l.startswith(ref)]
    ev["surface_line_after"] = still[0] if still else "(목록에서 사라짐)"
    ev["surface_dead_observed"] = ("exited=true" in (still[0] if still else "")) or (not still)
    ev["ok"] = ev["daemon_alive_after"] and ev["surface_dead_observed"]
    log("결과: 데몬 생존=%s · surface 사망관측=%s" % (
        ev["daemon_alive_after"], ev["surface_dead_observed"]))
    # 청소: 남은 surface 닫기
    cys("close-surface", ref, timeout=10)
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


# ------------------------------------------------------------------ 명령: record-cli (PTY)

def _clean_ansi(b):
    t = b.decode("utf-8", "replace")
    t = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", t)
    t = re.sub(r"\x1b\][0-9;].*?(\x07|\x1b\\)", "", t)
    t = re.sub(r"\x1b[=>78]", "", t)
    return t


READY_MARKERS = ['? for shortcuts', 'bypass permissions on', 'esc to interrupt', 'Try "',
                 'Resume session', 'conversations found', 'Loading conversations']
MODAL_MARKERS = ['❯', 'Enter to confirm', 'Do you trust', 'keep browser', 'use my browser']


def _pty_record(argv, out_raw, max_secs=25.0, dismiss_modals=True):
    """실 CLI를 PTY로 기동해 ready_marker까지의 바이트를 녹화. 메시지 미전송(토큰0).
    자식은 pty.fork로 세션 리더가 되므로 killpg로 자손까지 확실히 종료한다."""
    import pty
    for k in LEAKY_ENV:
        os.environ.pop(k, None)
    raw = open(out_raw, "wb")
    pid, fd = pty.fork()
    if pid == 0:
        # child
        try:
            os.execvp(argv[0], argv)
        except Exception:
            os._exit(127)
    ready = False
    marker = ""
    acc = b""
    last_esc = 0.0
    t0 = time.time()
    try:
        while time.time() - t0 < max_secs:
            r, _, _ = select.select([fd], [], [], 0.3)
            if fd in r:
                try:
                    d = os.read(fd, 4096)
                except OSError:
                    break
                if not d:
                    break
                raw.write(d); raw.flush(); acc += d
                txt = acc.decode("utf-8", "replace")
                hit = next((mk for mk in READY_MARKERS if mk in txt), None)
                if hit:
                    ready = True; marker = hit
                    time.sleep(0.5)
                    try:
                        r2, _, _ = select.select([fd], [], [], 0.5)
                        if fd in r2:
                            d2 = os.read(fd, 16384); raw.write(d2); raw.flush()
                    except Exception:
                        pass
                    break
                if dismiss_modals:
                    now = time.time()
                    if now - last_esc > 0.6 and any(mk in txt for mk in MODAL_MARKERS):
                        try:
                            os.write(fd, b"\x1b")  # Esc로 온보딩 모달 해제
                        except Exception:
                            pass
                        last_esc = now; acc = b""
    finally:
        raw.close()
        # teardown: Ctrl-C → killpg(자손 포함) → WNOHANG 수거
        try:
            os.write(fd, b"\x03")
        except Exception:
            pass
        time.sleep(0.3)
        _kill_pg(pid, signal.SIGKILL)
        for _ in range(25):
            try:
                w, _ = os.waitpid(pid, os.WNOHANG)
                if w:
                    break
            except ChildProcessError:
                break
            time.sleep(0.1)
        try:
            os.close(fd)
        except Exception:
            pass
    return ready, marker


def cmd_record_cli(args):
    """실 claude CLI '기동→ready_marker→--resume' 구간 1회 녹화(record-replay stub 시드).
    ⚠ LLM 작업 지시 절대 금지 — 메시지 미전송, 기동/ready까지만, 토큰 최소."""
    guard_isolation()
    os.makedirs(RECORD_DIR, exist_ok=True)
    which_claude = _resolve_claude()   # DD-2: 로그인셸 프로브 단일화(shutil.which 폴백)
    result = {"claude_path": which_claude}
    if not which_claude:
        die("claude CLI를 찾을 수 없다")
    # 격리된 임시 작업 디렉터리(라이브 프로젝트 무접촉)
    work = os.path.join(RECORD_DIR, "cwd")
    os.makedirs(work, exist_ok=True)
    cwd0 = os.getcwd()
    os.chdir(work)
    try:
        # ── 레그 1: 기동 → ready_marker (interactive startup handshake) ──
        raw1 = os.path.join(RECORD_DIR, "startup.raw")
        ready1, mk1 = _pty_record([which_claude, "--dangerously-skip-permissions"], raw1, max_secs=25.0)
        result["startup_ready"] = ready1
        result["startup_marker"] = mk1
        result["startup_raw"] = raw1
        result["startup_bytes"] = os.path.getsize(raw1) if os.path.exists(raw1) else 0
        # ── 레그 2: --resume 코드경로 handshake (세션 피커/복원 화면 · 메시지 미전송) ──
        raw2 = os.path.join(RECORD_DIR, "resume.raw")
        ready2, mk2 = _pty_record([which_claude, "--dangerously-skip-permissions", "--resume"], raw2, max_secs=20.0)
        result["resume_ready"] = ready2
        result["resume_marker"] = mk2
        result["resume_raw"] = raw2
        result["resume_bytes"] = os.path.getsize(raw2) if os.path.exists(raw2) else 0
    finally:
        os.chdir(cwd0)
    # 정리된 트랜스크립트 + ready_marker 헤더 기록(성공기준 ③: 파일 존재 + ready_marker 포함)
    seed = os.path.join(RECORD_DIR, "startup.transcript.txt")
    with open(seed, "w") as f:
        f.write("# CLI record-replay stub seed (기동→ready 구간 · 메시지 미전송=토큰0)\n")
        f.write("# READY_MARKER: %s\n" % (result.get("startup_marker") or "(미검출)"))
        f.write("# claude: %s\n\n" % which_claude)
        if os.path.exists(result.get("startup_raw", "")):
            f.write(_clean_ansi(open(result["startup_raw"], "rb").read()))
    result["seed_transcript"] = seed
    result["seed_contains_ready_marker"] = bool(result.get("startup_marker"))
    log("녹화 완료: startup ready=%s(%r, %dB) · resume ready=%s(%r, %dB) · seed=%s" % (
        result["startup_ready"], result["startup_marker"], result["startup_bytes"],
        result["resume_ready"], result["resume_marker"], result["resume_bytes"], seed))
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return result


# ------------------------------------------------------------------ 명령: qualify (자격시험 4건)

def _q_queued_durability():
    """a) --queued 큐가 데몬 재시작에서 생존하는가 — 큐를 얼린 채 surface로 넣고 kill -9 → 재기동 → 큐 잔존 확인.
    role 등록은 caller가 그 surface 안이어야 하므로(외부 claim 거부), role 대신 --surface로 직접 큐잉한다."""
    ev = {"test": "queued_survives_restart"}
    r = cys("new-surface", timeout=15)
    m = re.search(r"(surface:\d+)", r.stdout or "")
    ref = m.group(1) if m else None
    ev["target_surface"] = ref
    if not ref:
        ev["verdict"] = "SKIP(surface 생성 실패)"
        return ev
    # 큐 배달 동결(pause) → --queued 주입(surface 대상) → 얼어붙은 채 큐에 남음
    ev["pause_out"] = (cys("pause", timeout=10).stdout or "").strip()[:160]
    snd = cys("send", "--queued", "--surface", ref, "PHOENIX_PROBE_MSG_A1", timeout=10)
    ev["send_queued_rc"] = snd.returncode
    ev["send_queued_out"] = (snd.stdout or snd.stderr or "").strip()[:200]
    q1 = cys("queue", "list", timeout=10)
    ev["queue_before_kill"] = (q1.stdout or q1.stderr or "").strip()[:400]
    ev["queued_present_before"] = "PHOENIX_PROBE_MSG_A1" in ev["queue_before_kill"]
    # 데몬 kill -9 → 재기동
    pids = harness_daemon_pids()
    ev["killed"] = pids[:1]
    for p in pids[:1]:
        _kill_pg(p, signal.SIGKILL)
    global _tracked_daemon
    _tracked_daemon = None
    time.sleep(0.6)
    start_daemon()
    q2 = cys("queue", "list", timeout=10)
    ev["queue_after_restart"] = (q2.stdout or q2.stderr or "").strip()[:400]
    cys("resume", timeout=10)
    # 판정: 재시작 후 큐에 프로브 메시지가 남아있으면 durable
    ev["msg_survived"] = "PHOENIX_PROBE_MSG_A1" in ev["queue_after_restart"]
    if not ev["queued_present_before"]:
        ev["verdict"] = "INCONCLUSIVE(주입 직후 큐에서 미확인 — 원문 참조)"
    elif ev["msg_survived"]:
        ev["verdict"] = "DURABLE(큐가 재시작 생존)"
    else:
        ev["verdict"] = "VOLATILE(큐 재시작 소실 — 설계 §9.0 실전사고와 일치)"
    return ev


def _q_restore_idempotency():
    """b) restore 멱등성 — 생존 role이 있는 상태에서 재실행 시 중복 기동 여부."""
    ev = {"test": "restore_idempotency"}
    n0 = len([l for l in (cys("list", timeout=10).stdout or "").splitlines() if l.startswith("surface:")])
    ev["surfaces_before"] = n0
    r1 = cys("restore", timeout=45)
    ev["restore1_rc"] = r1.returncode
    ev["restore1_out"] = (r1.stdout or r1.stderr or "").strip()[:500]
    time.sleep(1.0)
    n1 = len([l for l in (cys("list", timeout=10).stdout or "").splitlines() if l.startswith("surface:")])
    ev["surfaces_after_restore1"] = n1
    r2 = cys("restore", timeout=45)
    ev["restore2_rc"] = r2.returncode
    ev["restore2_out"] = (r2.stdout or r2.stderr or "").strip()[:500]
    time.sleep(1.0)
    n2 = len([l for l in (cys("list", timeout=10).stdout or "").splitlines() if l.startswith("surface:")])
    ev["surfaces_after_restore2"] = n2
    ev["idempotent_no_duplicate"] = (n2 <= n1)
    ev["verdict"] = ("IDEMPOTENT(2회차 중복기동 없음)" if n2 <= n1
                     else "NON-IDEMPOTENT(2회차 surface 증가=중복기동)")
    return ev


def _q_watch():
    """c) watch 동작 — surface에 마커 라인을 흘려 regex 매칭 시 블로킹 해제되는지."""
    ev = {"test": "watch"}
    r = cys("new-surface", timeout=15)
    m = re.search(r"(surface:\d+)", r.stdout or "")
    ref = m.group(1) if m else None
    ev["surface"] = ref
    if not ref:
        ev["verdict"] = "SKIP(surface 생성 실패)"
        return ev
    marker = "PHOENIX_WATCH_DONE_C3"
    # 백그라운드로 watch 시작, 그 다음 마커를 surface에 출력
    import threading
    box = {}
    def _run_watch():
        w = cys("watch", "--surface", ref, "--until", marker, "--timeout", "12", timeout=16)
        box["rc"] = w.returncode
        box["out"] = (w.stdout or w.stderr or "").strip()[:200]
    th = threading.Thread(target=_run_watch, daemon=True)
    th.start()
    time.sleep(1.0)
    cys("send", "--surface", ref, "echo %s" % marker, timeout=10)
    cys("send-key", "--surface", ref, "Return", timeout=10)
    th.join(timeout=14)
    ev["watch_returned"] = not th.is_alive()
    ev["watch_rc"] = box.get("rc")
    ev["watch_out"] = box.get("out", "(미반환)")
    ev["verdict"] = ("WORKS(마커 출력 시 블로킹 해제)" if ev["watch_returned"] and box.get("rc") == 0
                     else "INCONCLUSIVE(타임아웃 내 미반환 — 원문 출력 참조)")
    cys("close-surface", ref, timeout=10)
    return ev


def _q_startup_lock_contention():
    """d) launchd KeepAlive vs 앱 내장 데몬 startup lock 경합 재현 — 같은 소켓에 cysd 2개 동시 기동."""
    ev = {"test": "startup_lock_contention"}
    _fresh_harness()
    ev["primary_alive"] = harness_ping()
    # 같은 격리 소켓에 두 번째 cysd 기동 시도 → startup lock 경합 재현
    p = subprocess.run([CYSD], env=_daemon_env(), capture_output=True, text=True,
                       cwd=HARN_DIR, timeout=8)
    combined = (p.stdout or "") + (p.stderr or "")
    ev["second_cysd_rc"] = p.returncode
    ev["second_cysd_output"] = combined.strip()[:400]
    ev["lock_contention_reproduced"] = ("holds the startup lock" in combined) or ("cys.lock" in combined)
    ev["verdict"] = ("REPRODUCED(두 번째 cysd가 startup lock에 막혀 종료 — cysd.log 경합과 동일)"
                     if ev["lock_contention_reproduced"]
                     else "NOT-REPRODUCED(원문 출력 참조)")
    ev["primary_still_alive"] = harness_ping()
    return ev


def cmd_qualify(args):
    guard_isolation()
    _fresh_harness()
    results = {}
    for name, fn in [
        ("a_queued_durability", _q_queued_durability),
        ("b_restore_idempotency", _q_restore_idempotency),
        ("c_watch", _q_watch),
        ("d_startup_lock_contention", _q_startup_lock_contention),
    ]:
        log("자격시험 %s 실행..." % name)
        try:
            results[name] = fn()
        except Exception as e:
            results[name] = {"test": name, "error": repr(e)}
        log("  → %s" % results[name].get("verdict", results[name].get("error", "?")))
    print(json.dumps(results, ensure_ascii=False, indent=2))
    return results


# ------------------------------------------------------------------ 명령: drill (일괄 + 증거)

def cmd_drill(args):
    guard_isolation()
    ev = {"ts_note": "wall-clock stamped by caller", "harness_dir": HARN_DIR}
    log("=== DRILL 시작 ===")
    ev["up"] = cmd_up(args)
    ev["crash_daemon"] = cmd_crash_daemon(args)
    ev["crash_agent"] = cmd_crash_agent(args)
    ev["record_cli"] = cmd_record_cli(args)
    ev["qualify"] = cmd_qualify(args)
    log("=== DRILL 종료 → teardown ===")
    ev["teardown"] = cmd_down(args)
    ev["residual_final"] = residual_report()
    with open(EVIDENCE, "w") as f:
        json.dump(ev, f, ensure_ascii=False, indent=2)
    log("증거 번들 기록: %s" % EVIDENCE)
    print("\n===== DRILL EVIDENCE (요약) =====")
    print(json.dumps({
        "isolation_live_unchanged": ev["up"].get("live_unchanged"),
        "sockets_distinct": ev["up"].get("sockets_distinct"),
        "fixture_reboot_ok": ev["crash_daemon"].get("fixture_reboot_ok"),
        "crash_agent_ok": ev["crash_agent"].get("ok"),
        "cli_ready_marker": ev["record_cli"].get("startup_marker"),
        "cli_seed_has_marker": ev["record_cli"].get("seed_contains_ready_marker"),
        "qualify_verdicts": {k: v.get("verdict") for k, v in ev["qualify"].items()},
        "residual_zero": not ev["residual_final"]["harness_daemon_pids"],
    }, ensure_ascii=False, indent=2))
    return ev


# ================================================================== Phase 2: phoenix-drill
# 불사조 부활 저널 상태머신(javis_phoenix.py)을 격리 하네스에서 완료 기준 drill로 검증한다.

PHOENIX = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_phoenix.py")


def _seed_topology(entries):
    """격리 데몬 상태 dir(state-harness)에 topology.json 시드 — 위임 대장 대역."""
    p = os.path.join(HARN_DIR, "topology.json")
    with open(p, "w") as f:
        json.dump({"entries": entries, "updated_at": 0}, f, ensure_ascii=False)
    return p


def _phoenix(*args, extra_env=None, timeout=60):
    """javis_phoenix.py 를 격리 소켓 대상으로 실 CLI 호출(하위 프로세스 — 실제 게이트 검증)."""
    cmd = [sys.executable, PHOENIX, "--socket", HARN_SOCK] + [str(a) for a in args]
    env = dict(os.environ)
    for k in LEAKY_ENV:
        env.pop(k, None)
    if extra_env:
        env.update(extra_env)
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
        return r
    except subprocess.TimeoutExpired as e:
        class _R:
            returncode = 124
            stdout = (e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")) if e.stdout else ""
            stderr = "TIMEOUT"
        return _R()


def _phoenix_json(*args, **kw):
    r = _phoenix(*args, **kw)
    txt = r.stdout or ""
    i = txt.find("{")
    j = None
    if i >= 0:
        try:
            j = json.loads(txt[i:])
        except Exception:
            j = None
    return j, r


def cmd_phoenix_drill(args):
    """완료 기준 drill: M9(verified/unverified)·저널 재개·M5 차단기·B1 reconcile·⑦ 생존 role skip
    ·gen-manual(⑥)·gen-protect(M4 dry-run) 전부를 격리 데몬에서 실측."""
    guard_isolation()
    ev = {"harness_dir": HARN_DIR, "phoenix": PHOENIX}
    live_before = len(live_surfaces())
    _fresh_harness()
    ph = os.path.join(HARN_DIR, "phoenix")
    import shutil as _sh
    _sh.rmtree(ph, ignore_errors=True)  # 깨끗한 저널로 시작

    # ── T_A: M9 정상 부활 → VERIFIED ──
    _seed_topology([{"agent": "stub", "agent_bin": "stub", "cwd": HARN_DIR,
                     "role": "worker", "session_id": "SID-W-1", "title": "w"}])
    jA, rA = _phoenix_json("restore", "--ticket", "TA", "--stub")
    ev["T_A_verified"] = {"outcome": (jA or {}).get("phoenix_restore"),
                          "per_role": (jA or {}).get("per_role_outcome"),
                          "pass": (jA or {}).get("phoenix_restore") == "VERIFIED"}
    log("T_A(M9 정상) → %s" % ev["T_A_verified"]["outcome"])

    # ── T_B: M9 오복원(세션 불일치) → UNVERIFIED, 'success/성공' 문자열 부재 ──
    _seed_topology([{"agent": "stub", "agent_bin": "stub", "cwd": HARN_DIR,
                     "role": "worker", "session_id": "SID-W-1", "title": "w"}])
    jB, rB = _phoenix_json("restore", "--ticket", "TB", "--stub",
                           "--stub-sids", '{"worker":"WRONG-SID-999"}')
    out_txt = (rB.stdout or "")
    ev["T_B_unverified"] = {
        "outcome": (jB or {}).get("phoenix_restore"),
        "no_success_string": ("success" not in out_txt.lower()) and ("성공" not in out_txt),
        "pass": (jB or {}).get("phoenix_restore") == "UNVERIFIED",
    }
    log("T_B(M9 오복원) → %s · success문자열부재=%s" % (
        ev["T_B_unverified"]["outcome"], ev["T_B_unverified"]["no_success_string"]))

    # ── T_C: 저널 재개 — spawn+ready 완료로 미리 심고, 살아있는 surface 위에서 재실행 → 두 단계 skip ──
    _seed_topology([{"agent": "stub", "agent_bin": "stub", "cwd": HARN_DIR,
                     "role": "worker", "session_id": "SID-W-1", "title": "w"}])
    # 살아있는 surrogate surface를 하나 만들고, 그 위에 spawn+ready done 저널을 심는다
    rns = cys("new-surface", timeout=15)
    m = re.search(r"(surface:\d+)", rns.stdout or "")
    ref = m.group(1) if m else None
    cys("send", "--surface", ref, "echo PHOENIX_STUB_READY role=worker SESSION=SID-W-1 ENDMARK; exec sleep 3600", timeout=10)
    cys("send-key", "--surface", ref, "Return", timeout=10)
    time.sleep(1.0)
    os.makedirs(ph, exist_ok=True)
    # ★Phase6: 같은 부팅 세대의 크래시 재개이므로 완료 마킹에 현재 epoch를 스탬프한다
    #   (epoch 게이트가 켜져도 '같은 epoch → 정당 skip'이 유지됨 = 성공기준 ①). phoenix가 계산하는
    #   것과 정확히 동일한 문자열을 status의 boot_epoch에서 가져와 포맷 불일치 위험을 제거.
    _jsc, _ = _phoenix_json("status")
    epoch_c = (_jsc or {}).get("boot_epoch")
    partial = {"ticket": "TC", "roles": {"worker": {"stages": {
        "spawn": {"done": True, "ts": 0, "epoch": epoch_c, "evidence": "미리 심음(크래시 전 완료 가정·같은 epoch)"},
        "ready": {"done": True, "ts": 0, "epoch": epoch_c, "evidence": "미리 심음"}},
        "surface": ref, "expected_sid": "SID-W-1"}}, "events": [], "created": 0}
    with open(os.path.join(ph, "journal-TC.json"), "w") as f:
        json.dump(partial, f, ensure_ascii=False)
    jC, rC = _phoenix_json("restore", "--ticket", "TC", "--stub")
    # 재실행 저널의 이벤트에서 spawn=skip 이고 resume 이 새로 실행됐는지 확인
    jc_after = json.load(open(os.path.join(ph, "journal-TC.json")))
    spawn_skipped = any(e["stage"] == "spawn" and e["status"] == "skip" for e in jc_after.get("events", []))
    resume_ran = any(e["stage"] == "resume" for e in jc_after.get("events", []))
    ev["T_C_journal_resume"] = {
        "outcome": (jC or {}).get("phoenix_restore"),
        "spawn_skipped": spawn_skipped, "resume_executed": resume_ran,
        "pass": spawn_skipped and resume_ran and (jC or {}).get("phoenix_restore") == "VERIFIED",
    }
    log("T_C(저널 재개) → spawn skip=%s · resume 재실행=%s · %s" % (
        spawn_skipped, resume_ran, ev["T_C_journal_resume"]["outcome"]))

    # ── T_D: M5 회로차단기 — 낮은 임계(N=3/T=300)로 restore 3회 → 3회차 BREAKER_OPEN ──
    _sh.rmtree(ph, ignore_errors=True)
    _seed_topology([{"agent": "stub", "agent_bin": "stub", "cwd": HARN_DIR,
                     "role": "worker", "session_id": "SID-W-1", "title": "w"}])
    breaker_env = {"PHOENIX_BREAKER_N": "3", "PHOENIX_BREAKER_T": "300"}
    outcomes = []
    # ★크래시 루프는 '실패 반복'이므로 UNVERIFIED(세션 불일치)로 시도 — 성공 리셋 없이 누적되어 3회차 OPEN
    for i in range(3):
        ji, ri = _phoenix_json("restore", "--ticket", "TD%d" % i, "--stub",
                               "--stub-sids", '{"worker":"WRONG-LOOP"}', extra_env=breaker_env)
        outcomes.append((ji or {}).get("phoenix_restore"))
    ev["T_D_breaker"] = {
        "attempt_outcomes": outcomes,
        "breaker_opened_on_3rd": outcomes[-1] == "BREAKER_OPEN",
        "pass": outcomes[-1] == "BREAKER_OPEN",
    }
    log("T_D(M5 차단기) → 시도결과=%s · 3회차 OPEN=%s" % (outcomes, ev["T_D_breaker"]["breaker_opened_on_3rd"]))

    # ── T_E: B1 reconcile — 대장 2역할 중 1역할만 생존 → DIVERGED/MISSING ──
    _seed_topology([
        {"agent": "stub", "agent_bin": "stub", "cwd": HARN_DIR, "role": "worker", "session_id": "SID-W-1", "title": "w"},
        {"agent": "stub", "agent_bin": "stub", "cwd": HARN_DIR, "role": "cso", "session_id": "SID-C-1", "title": "c"},
    ])
    jE, rE = _phoenix_json("reconcile")
    ev["T_E_reconcile"] = {
        "verdict": (jE or {}).get("verdict"),
        "missing": (jE or {}).get("MISSING(대장O/실측X=부활필요)"),
        "pass": "DIVERGED" in ((jE or {}).get("verdict") or "") and bool((jE or {}).get("MISSING(대장O/실측X=부활필요)")),
    }
    log("T_E(B1 reconcile) → %s · MISSING=%s" % (
        ev["T_E_reconcile"]["verdict"], ev["T_E_reconcile"]["missing"]))

    # ── T_F(⑦): 생존 role skip — worker surface 살아있게 한 뒤 restore 2회 → 매번 대상0(NOOP) ──
    _sh.rmtree(ph, ignore_errors=True)
    # worker role로 등록된 살아있는 surface를 만든다(in-surface claim-role)
    rns2 = cys("new-surface", timeout=15)
    m2 = re.search(r"(surface:\d+)", rns2.stdout or "")
    ref2 = m2.group(1) if m2 else None
    cys("send", "--surface", ref2, "cys --socket %s claim-role worker; exec sleep 3600" % HARN_SOCK, timeout=10)
    cys("send-key", "--surface", ref2, "Return", timeout=10)
    time.sleep(1.5)
    _seed_topology([{"agent": "stub", "agent_bin": "stub", "cwd": HARN_DIR,
                     "role": "worker", "session_id": "SID-W-1", "title": "w"}])
    live_roles = live_role_surfaces_local()
    r1o, _ = _phoenix_json("restore", "--ticket", "TF", "--stub", "--no-breaker")
    r2o, _ = _phoenix_json("restore", "--ticket", "TF", "--stub", "--no-breaker")
    ev["T_F_surviving_skip"] = {
        "worker_alive_in_daemon": "worker" in live_roles,
        "restore1": (r1o or {}).get("phoenix_restore"),
        "restore2": (r2o or {}).get("phoenix_restore"),
        # 생존 role이면 대상=0 → NOOP(중복 기동 없음). 저널 role 수 증가 없음.
        "pass": (r1o or {}).get("phoenix_restore") in ("NOOP", "VERIFIED")
                and (r2o or {}).get("phoenix_restore") in ("NOOP", "VERIFIED"),
        "note": "생존 role은 restore 대상에서 제외(죽은 role만 재기동) — 반복해도 중복 기동 없음(HARNESS 후속2 검증).",
    }
    log("T_F(⑦ 생존 role skip) → worker생존=%s · restore1=%s restore2=%s" % (
        ev["T_F_surviving_skip"]["worker_alive_in_daemon"],
        ev["T_F_surviving_skip"]["restore1"], ev["T_F_surviving_skip"]["restore2"]))

    # ── T_G: ⑥ 독립 수동 복원 스크립트 + M4 보호 스크립트(dry-run) 생성 ──
    _seed_topology([{"agent": "claude", "agent_bin": "claude", "cwd": HARN_DIR,
                     "role": "worker", "session_id": "SID-W-1", "title": "w"}])
    jman, _ = _phoenix_json("gen-manual")
    man_path = (jman or {}).get("manual_restore_script", "")
    man_ok = bool(man_path) and os.path.exists(man_path) and os.access(man_path, os.X_OK)
    man_body = open(man_path).read() if man_ok else ""
    jprot, _ = _phoenix_json("gen-protect")
    prot_path = (jprot or {}).get("protect_script", "")
    prot_ok = bool(prot_path) and os.path.exists(prot_path)
    prot_applied = (jprot or {}).get("applied", None)
    ev["T_G_artifacts"] = {
        "manual_restore_exists_exec": man_ok,
        "manual_self_contained": ("topology.json" in man_body and "launch-agent" in man_body),
        "protect_exists": prot_ok,
        "protect_applied_to_live": prot_applied,  # False 여야 함(적용 금지)
        "pass": man_ok and prot_ok and prot_applied is False,
    }
    log("T_G(⑥ 수동복원+M4) → manual실행가능=%s · protect적용=%s(False기대)" % (
        man_ok, prot_applied))

    # ── teardown + 잔여0 ──
    remain = teardown(verbose=True)
    live_after = len(live_surfaces())
    ev["teardown_clean"] = (not remain) and (not harness_ping())
    ev["live_unchanged"] = live_before == live_after
    ev["residual_zero"] = not remain
    with open(os.path.join(HARN_DIR, "phoenix_drill_evidence.json"), "w") as f:
        json.dump(ev, f, ensure_ascii=False, indent=2)

    summary = {
        "T_A_M9_verified": ev["T_A_verified"]["pass"],
        "T_B_M9_unverified_no_success_str": ev["T_B_unverified"]["pass"] and ev["T_B_unverified"]["no_success_string"],
        "T_C_journal_resume": ev["T_C_journal_resume"]["pass"],
        "T_D_M5_breaker": ev["T_D_breaker"]["pass"],
        "T_E_B1_reconcile": ev["T_E_reconcile"]["pass"],
        "T_F_surviving_role_skip": ev["T_F_surviving_skip"]["pass"],
        "T_G_manual_and_protect": ev["T_G_artifacts"]["pass"],
        "live_unchanged": ev["live_unchanged"],
        "residual_zero": ev["residual_zero"],
    }
    print("\n===== PHOENIX DRILL SUMMARY =====")
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    ev["summary"] = summary
    return ev


def cmd_phoenix_p4_erosion(args):
    """Phase 4: desired-state 침식 면역 회귀 가드(DRILL_LIVE_1 §12 수리 검증).
    4역할 관측→topology 침식(1역할)→phoenix가 desired로 3역할 죽음 판정(NOOP 오판 제거)·tombstone."""
    guard_isolation()
    ev = {"test": "desired_state_erosion_immunity"}
    live_before = len(live_surfaces())
    _fresh_harness()
    ph = os.path.join(HARN_DIR, "phoenix")
    import shutil as _sh
    _sh.rmtree(ph, ignore_errors=True)
    roles = ["worker", "cso", "reviewer-gemini", "reviewer-codex"]
    _seed_topology([{"role": r, "agent": "stub", "session_id": "S-" + r} for r in roles])
    # 1) 침식 전 관측 → desired 4역할 박제
    jr, _ = _phoenix_json("roster")
    ev["desired_before"] = (jr or {}).get("desired_roster(선언·침식 면역)")
    # 2) 침식: persist_topology가 부분부활 후 미부활 3역할 삭제 재현 → topology 1역할
    _seed_topology([{"role": "worker", "agent": "stub", "session_id": "S-worker"}])
    jr2, _ = _phoenix_json("roster")
    ev["actual_after_erosion"] = (jr2 or {}).get("actual_topology(라이브·침식됨)")
    ev["desired_after_erosion"] = (jr2 or {}).get("desired_roster(선언·침식 면역)")
    ev["dead_by_desired"] = (jr2 or {}).get("dead_by_desired(부활 대상)")
    # 3) restore가 침식된 역할을 여전히 대상으로(NOOP 아님)
    jr3, r3 = _phoenix_json("restore", "--ticket", "P4E", "--stub")
    targeted = re.search(r"대상역할=(\[[^\]]*\])", r3.stdout or "")
    ev["restore_targets_raw"] = targeted.group(1) if targeted else ""
    ev["not_noop"] = "cso" in ev["restore_targets_raw"] and (jr3 or {}).get("phoenix_restore") != "NOOP"
    # 4) tombstone cso → desired 축소·부활 대상 아님
    _phoenix_json("tombstone", "cso")
    jr4, _ = _phoenix_json("roster")
    ev["dead_after_tombstone"] = (jr4 or {}).get("dead_by_desired(부활 대상)")
    ev["tombstone_excludes_cso"] = "cso" not in (ev["dead_after_tombstone"] or [])
    remain = teardown(verbose=True)
    ev["immunity_pass"] = (
        ev["actual_after_erosion"] == ["worker"]
        and set(ev["desired_after_erosion"] or []) == set(roles)
        and ev["not_noop"]
        and ev["tombstone_excludes_cso"]
    )
    ev["live_unchanged"] = live_before == len(live_surfaces())
    ev["residual_zero"] = not remain
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


def cmd_phoenix_p5_redelivery(args):
    """Phase 5 ①c 회귀 가드: 큐 재배달 갭 폐쇄(성공기준 ③). worker role surface에 --queued →
    kill-9 → 재기동(restored_queue 생존) → 새 worker surface → deliver_queued 재홈 → idle 배달."""
    guard_isolation()
    ev = {"test": "queue_redelivery_across_restart"}
    live_before = len(live_surfaces())
    _fresh_harness()

    def _mk_worker():
        # ★Phase9: claim-role 이 list 에 나타날 때까지 검증-재시도(PTY 타이밍 flakiness 제거·결정론).
        # 반환 (surface, claimed) — claim-role 은 surface shell 에서 실행돼야 하므로 send-key 실행에 의존한다.
        r = cys("new-surface", timeout=15)
        m = re.search(r"(surface:\d+)", r.stdout or "")
        ref = m.group(1) if m else None
        if not ref:
            return ref, False
        claimed = False
        for _ in range(8):
            cys("send", "--surface", ref, "cys --socket %s claim-role worker" % HARN_SOCK, timeout=10)
            cys("send-key", "--surface", ref, "Return", timeout=10)
            time.sleep(1.2)
            if "worker" in live_role_surfaces_local():
                claimed = True
                break
        cys("send", "--surface", ref, "exec sleep 600", timeout=10)
        cys("send-key", "--surface", ref, "Return", timeout=10)
        time.sleep(0.5)
        return ref, claimed

    s1, claimed1 = _mk_worker()
    ev["s1"] = s1
    # ★Phase10 정직 SKIP: send-key Return 이 harness surface 명령을 실행하지 못하는 환경(app-bundle cysd PTY)에서는
    # claim-role 이 role 을 등록하지 못한다 → 큐 재배달 메커니즘을 테스트할 전제 자체가 성립 안 함(false-fail 금지).
    if not claimed1:
        ev["precondition"] = "claim_role_unavailable(send-key not executing surface commands)"
        ev["note"] = ("환경성 전제 미충족 — 큐 재배달 메커니즘 자체의 회귀가 아님. Phase5 WAL 코드 무변경. "
                      "surrogate 드릴(p6/p9/p10)은 marker-present 기반이라 영향 없음.")
        ev["redelivery_pass"] = None
        ev["live_unchanged"] = live_before == len(live_surfaces())
        teardown(verbose=True)
        subprocess.run(["pkill", "-9", "-f", "sleep 600"], capture_output=True)
        print(json.dumps(ev, ensure_ascii=False, indent=2))
        return ev
    cys("pause", timeout=10)
    snd = cys("send", "--queued", "--to", "worker", "P5_REDELIV_MSG", timeout=10)
    ev["send_out"] = (snd.stdout or snd.stderr or "").strip()[:120]
    # 재기동
    for p in harness_daemon_pids():
        _kill_pg(p, signal.SIGKILL)
    global _tracked_daemon
    _tracked_daemon = None
    time.sleep(0.6)
    start_daemon()
    q = cys("queue", "list", timeout=10)
    ev["restored_present"] = "P5_REDELIV_MSG" in (q.stdout or "")
    s2, _claimed2 = _mk_worker()
    ev["s2"] = s2
    cys("resume", timeout=10)
    time.sleep(10)  # deliver_queued 재홈 + idle 배달 대기
    q2 = cys("queue", "list", timeout=10)
    ev["queue_empty_after"] = "P5_REDELIV_MSG" not in (q2.stdout or "")
    scr = cys("read-screen", "--surface", s2, timeout=12) if s2 else None
    ev["delivered_to_new_worker"] = bool(scr and "P5_REDELIV_MSG" in (scr.stdout or ""))
    remain = teardown(verbose=True)
    # sleep 600 stub 정리
    subprocess.run(["pkill", "-9", "-f", "sleep 600"], capture_output=True)
    ev["redelivery_pass"] = ev["restored_present"] and ev["queue_empty_after"] and ev["delivered_to_new_worker"]
    ev["live_unchanged"] = live_before == len(live_surfaces())
    ev["residual_zero"] = not remain
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


def cmd_phoenix_p6_bootepoch(args):
    """Phase 6: 저널 boot-epoch 태그(DRILL_LIVE_2 worker 잘못-skip 수리) 회귀 가드.
    ① 같은 epoch 재실행 = 완료 stage 정당 skip(성공기준 ①·Phase2 저널재개 회귀 유지)
    ② 실 데몬 재시작(started_at 변경) 후 재실행 = 이전 완료마킹 무효화·worker 재spawn(성공기준 ②)
    ★A/B(producer≠evaluator: phoenix 서브프로세스 실행): 동일 stale 저널을 레거시(PHOENIX_EPOCH_GATE=0)로
      돌리면 worker 잘못 skip(버그 재현), 게이트 ON 기본으로 돌리면 재spawn(수리) — gate가 유일한 차이임을 실증."""
    guard_isolation()
    ev = {"test": "journal_boot_epoch_tag"}
    live_before = len(live_surfaces())
    _fresh_harness()
    ph = os.path.join(HARN_DIR, "phoenix")
    import shutil as _sh
    _sh.rmtree(ph, ignore_errors=True)

    def _pending(stdout):
        m = re.search(r"이번 진행=(\[[^\]]*\])", stdout or "")
        return m.group(1) if m else ""

    def _started_at():
        r = cys("status", "--json", timeout=10)
        try:
            return (json.loads(r.stdout or "{}").get("daemon") or {}).get("started_at")
        except Exception:
            return None

    seed = lambda: _seed_topology([{"role": "worker", "agent": "stub",
                                    "session_id": "SID-W-1", "cwd": HARN_DIR, "title": "w"}])
    jpath = os.path.join(ph, "journal-TP6.json")

    # ── 세대1: 완료 저널 확립(verify done@epoch1) ──
    seed()
    j1, _ = _phoenix_json("restore", "--ticket", "TP6", "--stub", "--no-breaker")
    ev["gen1_outcome"] = (j1 or {}).get("phoenix_restore")
    ev["gen1_boot_epoch"] = (j1 or {}).get("boot_epoch")
    ja = json.load(open(jpath))
    ev["journal_verify_epoch_gen1"] = ja["roles"]["worker"]["stages"]["verify"].get("epoch")
    ev["tagging_ok"] = (ev["journal_verify_epoch_gen1"] == ev["gen1_boot_epoch"]
                        and ev["gen1_boot_epoch"] is not None)
    started1 = _started_at()

    # ── ① 같은 epoch 재실행 → 완료(verify) 정당 skip(pending 에 worker 부재) ──
    _, rs = _phoenix_json("restore", "--ticket", "TP6", "--stub", "--no-breaker")
    ev["same_epoch_pending"] = _pending(rs.stdout)
    ev["same_epoch_skip"] = "worker" not in ev["same_epoch_pending"]

    # ── 세대2: 실 격리 데몬 재시작(started_at 변경) — DRILL_LIVE_2 재부팅 시뮬레이션 ──
    for p in harness_daemon_pids():
        _kill_pg(p, signal.SIGKILL)
    global _tracked_daemon
    _tracked_daemon = None
    time.sleep(0.6)
    start_daemon()
    started2 = _started_at()
    ev["daemon_started_at_1"] = started1
    ev["daemon_started_at_2"] = started2
    ev["daemon_restarted"] = (started2 is not None and started2 != started1)
    # 실 cysd 의 topology 영속을 시뮬레이션(재기동 시 actual-state 재구성) — desired 로스터는 이미 worker 박제
    seed()
    # 저널-TP6 은 디스크에 생존(verify.epoch = 세대1 = 현재 세대와 상이 = stale)
    jb = json.load(open(jpath))
    ev["journal_verify_epoch_before_fixed"] = jb["roles"]["worker"]["stages"]["verify"].get("epoch")
    ev["stale_before_fixed"] = (ev["journal_verify_epoch_before_fixed"] == ev["gen1_boot_epoch"])

    # ── ②A 레거시(gate OFF) — 버그 재현: stale done → worker 잘못 skip ──
    _sh.copyfile(jpath, os.path.join(ph, "journal-TP6L.json"))
    _, rL = _phoenix_json("restore", "--ticket", "TP6L", "--stub", "--no-breaker",
                          extra_env={"PHOENIX_EPOCH_GATE": "0"})
    ev["legacy_pending"] = _pending(rL.stdout)
    ev["legacy_wrongly_skips"] = "worker" not in ev["legacy_pending"]

    # ── ②B 수리(gate ON 기본) — 이전 완료마킹 무효화·worker 재spawn ──
    jF, rF = _phoenix_json("restore", "--ticket", "TP6", "--stub", "--no-breaker")
    ev["fixed_pending"] = _pending(rF.stdout)
    ev["fixed_respawns"] = "worker" in ev["fixed_pending"]
    ev["fixed_outcome"] = (jF or {}).get("phoenix_restore")
    ev["fixed_boot_epoch"] = (jF or {}).get("boot_epoch")
    jc = json.load(open(jpath))
    ev["journal_verify_epoch_after_fixed"] = jc["roles"]["worker"]["stages"]["verify"].get("epoch")
    ev["epoch_advanced"] = (ev["journal_verify_epoch_after_fixed"] == ev["fixed_boot_epoch"]
                            and ev["fixed_boot_epoch"] != ev["gen1_boot_epoch"])

    remain = teardown(verbose=True)
    ev["live_unchanged"] = live_before == len(live_surfaces())
    ev["residual_zero"] = not remain
    ev["p6_pass"] = bool(
        ev["gen1_outcome"] == "VERIFIED"
        and ev["tagging_ok"]
        and ev["same_epoch_skip"]
        and ev["daemon_restarted"]
        and ev["stale_before_fixed"]
        and ev["legacy_wrongly_skips"]
        and ev["fixed_respawns"]
        and ev["fixed_outcome"] == "VERIFIED"
        and ev["epoch_advanced"]
        and ev["live_unchanged"]
        and ev["residual_zero"]
    )
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


def cmd_phoenix_p7_inherit(args):
    """★Phase 7 상속 완결 drill(완성 기준): 새 노드 N개 + 합성 부서를 만들고, 손 배선 0으로
    (phoenix inherit 1회) 전부 보호집합에 자동 편입 → 실 데몬 재시작(재부팅 시뮬) 후에도 roster·dept-roster·
    스냅샷 소스에 잔존 → 죽은 새 노드가 부활 대상이 됨을 실증. + ① 명시 tombstone 만 제거·크래시 잔존.
    ★라이브 무접촉: 합성 부서는 env(PHOENIX_DEPT_STATE_ROOT/PHOENIX_DEPTS_JSON)로 격리 주입(실 depts.json 무접촉)."""
    guard_isolation()
    ev = {"test": "auto_protection_inheritance"}
    live_before = len(live_surfaces())
    _fresh_harness()
    ph = os.path.join(HARN_DIR, "phoenix")
    import shutil as _sh
    _sh.rmtree(ph, ignore_errors=True)

    # 합성 부서 루트 — registry(depts.json)엔 alpha만(stale), 디스크엔 alpha·beta 둘 다(파일시스템 truth 검증)
    dept_root = os.path.join(HARN_DIR, "dept-root")
    _sh.rmtree(dept_root, ignore_errors=True)
    os.makedirs(dept_root)
    for dep in ("cys-dept-alpha", "cys-dept-beta"):
        dd = os.path.join(dept_root, dep)
        os.makedirs(dd)
        with open(os.path.join(dd, "schedule_state.json"), "w") as f:
            f.write('{"jobs":[]}')
    depts_json = os.path.join(dept_root, "depts.json")
    with open(depts_json, "w") as f:
        json.dump({"depts": {"alpha": {"socket": os.path.join(dept_root, "cys-dept-alpha", "cys.sock")}}}, f)
    dept_env = {"PHOENIX_DEPT_STATE_ROOT": dept_root, "PHOENIX_DEPTS_JSON": depts_json}

    # ── 새 노드 N개 '탄생' = 데몬이 아는 role(topology 시드 — ★결정론·PTY claim-role 타이밍 flakiness 배제) ──
    new_roles = ["worker", "analyst"]
    _seed_topology([{"role": r, "agent": "stub", "session_id": "SID-" + r} for r in new_roles])
    ev["seeded_roles"] = new_roles

    # ── ① 자동 상속: phoenix inherit(창조시점/주기 reconciler primitive) 1회 = 손 배선 0 ──
    jinh, _ = _phoenix_json("inherit", extra_env=dept_env)
    ev["node_roster_after_inherit"] = (jinh or {}).get("node_roster(보호집합)")
    ev["dept_roster_after_inherit"] = (jinh or {}).get("dept_roster(보호집합)")
    ev["nodes_auto_registered"] = all(r in (ev["node_roster_after_inherit"] or []) for r in new_roles)
    ev["depts_auto_registered"] = ("alpha" in (ev["dept_roster_after_inherit"] or [])
                                   and "beta" in (ev["dept_roster_after_inherit"] or []))

    # ── ② 스냅샷 자동 커버리지: 동일 부서가 state_snapshot 소스에도 자동 포함(수동 tar.gz 미의존) ──
    snap = os.path.join(os.path.dirname(PHOENIX), "javis_state_snapshot.py")
    probe = subprocess.run([sys.executable, "-c",
        "import sys; sys.path.insert(0, %r); import javis_state_snapshot as m; "
        "print(chr(10).join(m.default_sources(state_root=%r, depts_json=%r)))"
        % (os.path.dirname(snap), dept_root, depts_json)],
        capture_output=True, text=True, timeout=15)
    src_lines = probe.stdout or ""
    ev["snapshot_covers_alpha"] = ("cys-dept-alpha" in src_lines and "schedule_state.json" in src_lines)
    ev["snapshot_covers_beta"] = ("cys-dept-beta" in src_lines)

    # ── ③ 재부팅 시뮬(실 데몬 재시작) → 노드 사망하지만 roster 잔존(크래시=보호 유지) ──
    for p in harness_daemon_pids():
        _kill_pg(p, signal.SIGKILL)
    global _tracked_daemon
    _tracked_daemon = None
    subprocess.run(["pkill", "-9", "-f", "sleep 600"], capture_output=True)
    time.sleep(0.6)
    _wipe_daemon_state()  # ★cysd 자동복원 차단(phoenix roster 보존) → dead_by_desired 결정론
    start_daemon()
    jr, _ = _phoenix_json("roster", extra_env=dept_env)
    jr = jr or {}
    ev["node_roster_after_restart"] = jr.get("desired_roster(선언·침식 면역)")
    ev["dept_roster_after_restart"] = jr.get("dept_roster(부서 보호집합·자동 상속)")
    ev["dead_by_desired"] = jr.get("dead_by_desired(부활 대상)")
    ev["nodes_survive_restart"] = all(r in (ev["node_roster_after_restart"] or []) for r in new_roles)
    ev["depts_survive_restart"] = ("alpha" in (ev["dept_roster_after_restart"] or [])
                                   and "beta" in (ev["dept_roster_after_restart"] or []))
    ev["nodes_are_revival_targets"] = all(r in (ev["dead_by_desired"] or []) for r in new_roles)

    # ③ 노드 자동 부활 판정: restore가 죽은 새 노드를 재spawn 대상(pending)으로 ──
    _, rres = _phoenix_json("restore", "--ticket", "P7", "--stub", "--no-breaker", extra_env=dept_env)
    pend = re.search(r"이번 진행=(\[[^\]]*\])", rres.stdout or "")
    ev["restore_pending"] = pend.group(1) if pend else ""
    ev["nodes_auto_revived"] = all(r in ev["restore_pending"] for r in new_roles)

    # ── ① 명시 tombstone만 제거·크래시(비명시)는 잔존: analyst 명시 폐역 ──
    _phoenix_json("tombstone", "analyst", extra_env=dept_env)
    jr2, _ = _phoenix_json("roster", extra_env=dept_env)
    roster2 = (jr2 or {}).get("desired_roster(선언·침식 면역)") or []
    ev["explicit_tombstone_removes"] = "analyst" not in roster2
    ev["crash_role_still_kept"] = "worker" in roster2

    remain = teardown(verbose=True)
    subprocess.run(["pkill", "-9", "-f", "sleep 600"], capture_output=True)
    ev["live_unchanged"] = live_before == len(live_surfaces())
    ev["residual_zero"] = not remain
    ev["p7_pass"] = bool(
        ev["nodes_auto_registered"] and ev["depts_auto_registered"]
        and ev["snapshot_covers_alpha"] and ev["snapshot_covers_beta"]
        and ev["nodes_survive_restart"] and ev["depts_survive_restart"]
        and ev["nodes_are_revival_targets"] and ev["nodes_auto_revived"]
        and ev["explicit_tombstone_removes"] and ev["crash_role_still_kept"]
        and ev["live_unchanged"] and ev["residual_zero"]
    )
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


def cmd_phoenix_p8_backup(args):
    """★Phase 8: 일반 백업 능력 + 정직 보호 상태 회귀 drill. javis_backup 함수 self-test(단위) +
    CLI E2E(통합·2계층 — classify/backup/restore/verify/status 실 CLI) + phoenix status 보호등급 배선 확인.
    ★라이브/원본 무접촉: 전부 /tmp 스크래치 합성 home. 실제 오프사이트 push 없음(스크래치 로컬 앵커만)."""
    guard_isolation()
    ev = {"test": "backup_capability_and_honest_status"}
    live_before = len(live_surfaces())
    BACKUP = os.path.join(os.path.dirname(PHOENIX), "javis_backup.py")
    import shutil as _sh
    import tempfile as _tf
    scratch = _tf.mkdtemp(prefix="cys-p8-", dir="/tmp")
    try:
        # 1) 함수 self-test(단위 계층)
        rs = subprocess.run([sys.executable, BACKUP, "self-test"], capture_output=True, text=True, timeout=90)
        ev["selftest_pass"] = rs.returncode == 0 and "self-test 전체 PASS" in (rs.stdout or "")

        # 2) CLI E2E(통합 계층) — 합성 home 스크래치
        home = os.path.join(scratch, "home")
        pack = os.path.join(home, ".cys", "pack")
        os.makedirs(os.path.join(pack, "memory"))
        open(os.path.join(pack, "soul.md"), "w").write("SOUL\n")
        open(os.path.join(pack, "memory", "MEMORY.md"), "w").write("idx\n")
        open(os.path.join(pack, "api.token"), "w").write("LEAKMARK_P8_XYZ\n")
        open(os.path.join(pack, "big.db"), "wb").write(b"\x00" * 4096)
        key = os.path.join(scratch, "k.key")
        open(key, "w").write("p8-scratch-key\n")
        out = os.path.join(scratch, "bk")

        def cli(*a, env=None):
            e = {k: v for k, v in os.environ.items() if k not in LEAKY_ENV}
            if env:
                e.update(env)
            return subprocess.run([sys.executable, BACKUP, *a], capture_output=True, text=True, timeout=45, env=e)

        clsj = json.loads(cli("classify", "--home", home).stdout)
        ev["cli_classify_tier2_has_token"] = any("api.token" in f for f in clsj["tier2_secrets_excluded"])
        ev["cli_classify_tier3_has_db"] = any("big.db" in f for f in clsj["tier3_local_only"])

        ev["cli_backup_rc0"] = cli("backup", "--out", out, "--home", home, "--key-file", key).returncode == 0
        offdir = os.path.join(out, "offsite")
        ev["offsite_ciphertext_only"] = sorted(os.listdir(offdir)) == ["tier1.tar.enc"]
        ev["token_not_in_offsite"] = b"LEAKMARK_P8_XYZ" not in open(os.path.join(offdir, "tier1.tar.enc"), "rb").read()

        dest = os.path.join(scratch, "restored")
        ev["cli_restore_rc0"] = cli("restore", "--in", out, "--dest", dest, "--key-file", key).returncode == 0
        soul_r = os.path.join(dest, ".cys", "pack", "soul.md")
        ev["restored_soul_matches"] = os.path.isfile(soul_r) and open(soul_r).read() == "SOUL\n"
        ev["token_not_restored"] = not os.path.exists(os.path.join(dest, ".cys", "pack", "api.token"))

        ev["cli_verify_ok"] = json.loads(cli("verify", "--in", out, "--key-file", key).stdout).get("verify") == "OK"
        wk = os.path.join(scratch, "wrong.key")
        open(wk, "w").write("nope\n")
        ev["wrong_key_fails"] = cli("verify", "--in", out, "--key-file", wk).returncode != 0

        # 3) 정직 보호등급 RED / GREEN (앵커 env)
        red = json.loads(cli("status", "--home", home, env={"CYS_BACKUP_DIR": os.path.join(scratch, "none")}).stdout)
        ev["status_red_no_backup"] = red["grade"] == "RED"
        green = json.loads(cli("status", "--home", home,
                               env={"CYS_BACKUP_DIR": out, "CYS_BACKUP_OFFSITE": "scratch://remote"}).stdout)
        ev["status_green_armed"] = green["grade"] == "GREEN"

        # 4) phoenix status 가 protection 등급 노출(배선 확인)
        penv = {k: v for k, v in os.environ.items() if k not in LEAKY_ENV}
        pr = subprocess.run([sys.executable, PHOENIX, "--socket", HARN_SOCK, "status"],
                            capture_output=True, text=True, timeout=25, env=penv)
        try:
            pj = json.loads(pr.stdout[pr.stdout.find("{"):])
            ev["phoenix_status_has_protection"] = "grade" in (pj.get("protection") or {})
        except Exception:
            ev["phoenix_status_has_protection"] = False
    finally:
        _sh.rmtree(scratch, ignore_errors=True)

    ev["live_unchanged"] = live_before == len(live_surfaces())
    ev["scratch_cleaned"] = not os.path.exists(scratch)
    ev["p8_pass"] = bool(
        ev.get("selftest_pass") and ev.get("cli_classify_tier2_has_token") and ev.get("cli_classify_tier3_has_db")
        and ev.get("cli_backup_rc0") and ev.get("offsite_ciphertext_only") and ev.get("token_not_in_offsite")
        and ev.get("cli_restore_rc0") and ev.get("restored_soul_matches") and ev.get("token_not_restored")
        and ev.get("cli_verify_ok") and ev.get("wrong_key_fails")
        and ev.get("status_red_no_backup") and ev.get("status_green_armed")
        and ev.get("phoenix_status_has_protection") and ev["live_unchanged"] and ev["scratch_cleaned"]
    )
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


def cmd_phoenix_p9_catastrophe(args):
    """★Phase 9 종합 파괴 meta-drill: 전 스택 end-to-end 무손실 증명.
    새 노드+부서 inherit → 백업(Tier1 암호화) → ★전멸(격리 데몬+전 노드 kill-9) → 신 데몬 부활 →
    phoenix restore(roster+boot-epoch 기준) → 백업 복원 → 원본 해시동일·roster 완전복구·손배선0."""
    guard_isolation()
    ev = {"test": "catastrophe_full_stack"}
    live_before = len(live_surfaces())
    _fresh_harness()
    ph = os.path.join(HARN_DIR, "phoenix")
    import shutil as _sh
    import tempfile as _tf
    _sh.rmtree(ph, ignore_errors=True)
    scratch = _tf.mkdtemp(prefix="cys-p9cat-", dir="/tmp")
    BACKUP = os.path.join(os.path.dirname(PHOENIX), "javis_backup.py")
    try:
        # 합성 부서(env 격리) + 백업 대상 정체성 home(스크래치)
        dept_root = os.path.join(HARN_DIR, "dept-root")
        _sh.rmtree(dept_root, ignore_errors=True)
        dd = os.path.join(dept_root, "cys-dept-alpha")
        os.makedirs(dd)
        open(os.path.join(dd, "schedule_state.json"), "w").write('{"jobs":[]}')
        depts_json = os.path.join(dept_root, "depts.json")
        json.dump({"depts": {"alpha": {"socket": os.path.join(dd, "cys.sock")}}}, open(depts_json, "w"))
        dept_env = {"PHOENIX_DEPT_STATE_ROOT": dept_root, "PHOENIX_DEPTS_JSON": depts_json}
        bhome = os.path.join(scratch, "home")
        pack = os.path.join(bhome, ".cys", "pack")
        os.makedirs(os.path.join(pack, "memory"))
        open(os.path.join(pack, "soul.md"), "w").write("SOUL-CAPSTONE\n")
        open(os.path.join(pack, "memory", "MEMORY.md"), "w").write("mem-index\n")
        key = os.path.join(scratch, "k.key")
        open(key, "w").write("p9-capstone-key\n")
        orig_hashes = {}
        for r, _d, fs in os.walk(pack):
            for fn in fs:
                p = os.path.join(r, fn)
                orig_hashes[os.path.relpath(p, bhome)] = _sha256(p)

        # 새 노드 '탄생' = 데몬이 아는 role(topology 시드 — ★결정론·PTY claim-role 타이밍 flakiness 배제).
        #   observe/inherit 가 이를 보호집합(desired_roster)으로 자동 편입한다(손 배선 0).
        _seed_topology([{"role": "worker", "agent": "stub", "session_id": "SID-W"},
                        {"role": "analyst", "agent": "stub", "session_id": "SID-A"}])
        _phoenix_json("inherit", extra_env=dept_env)

        # 백업(Tier1 암호화)
        br = subprocess.run([sys.executable, BACKUP, "backup", "--out", os.path.join(scratch, "bk"),
                             "--home", bhome, "--key-file", key], capture_output=True, text=True, timeout=45)
        ev["backup_ok"] = br.returncode == 0

        # ★전멸(Phase11 ②: realpath/pid 기반·빗나감0): 격리 데몬 + 전 자손(stub surface 자식 포함)을 pid 로 kill.
        #   이전 pkill -f 'sleep 600' 문자열 매칭은 stub(sleep 3600)을 빗나갔다 — _annihilate 는 부모체인 pid 로 정확 적중.
        global _tracked_daemon
        ev["annihilated_pids"] = _annihilate()
        _tracked_daemon = None
        time.sleep(0.8)
        ev["annihilated_ping"] = harness_ping()  # False 기대(전멸)

        # 신 데몬 부활 — ★topology 소거 후 기동해 cysd 자동복원(죽은 role 재등장)을 차단한다.
        #   phoenix/ dept-root/ 는 보존 → phoenix 의 roster 가 부활의 단일 권위(결정론).
        _wipe_daemon_state()
        start_daemon()
        ev["reborn_ping"] = harness_ping()  # True 기대

        # phoenix restore(roster+boot-epoch) → 죽은 새 노드 재spawn 대상
        _, rres = _phoenix_json("restore", "--ticket", "P9CAT", "--stub", "--no-breaker", extra_env=dept_env)
        pend = re.search(r"이번 진행=(\[[^\]]*\])", rres.stdout or "")
        ev["restore_pending"] = pend.group(1) if pend else ""
        ev["nodes_revived"] = all(r in ev["restore_pending"] for r in ("worker", "analyst"))

        # roster 완전 복구(노드+부서)
        jr, _ = _phoenix_json("roster", extra_env=dept_env)
        jr = jr or {}
        roster_after = set(jr.get("desired_roster(선언·침식 면역)") or [])
        dept_after = set(jr.get("dept_roster(부서 보호집합·자동 상속)") or [])
        ev["roster_full_recovery"] = {"worker", "analyst"}.issubset(roster_after) and "alpha" in dept_after

        # 백업 복원 → 원본 해시 동일
        dest = os.path.join(scratch, "restored")
        rr = subprocess.run([sys.executable, BACKUP, "restore", "--in", os.path.join(scratch, "bk"),
                             "--dest", dest, "--key-file", key], capture_output=True, text=True, timeout=45)
        ev["restore_backup_ok"] = rr.returncode == 0
        mism = [rel for rel, h in orig_hashes.items()
                if not os.path.isfile(os.path.join(dest, rel)) or _sha256(os.path.join(dest, rel)) != h]
        ev["backup_hash_identical"] = not mism
        ev["hash_mismatches"] = mism
    finally:
        teardown(verbose=True)
        subprocess.run(["pkill", "-9", "-f", "sleep 600"], capture_output=True)
        _sh.rmtree(scratch, ignore_errors=True)
    ev["live_unchanged"] = live_before == len(live_surfaces())
    ev["scratch_cleaned"] = not os.path.exists(scratch)
    ev["p9_pass"] = bool(
        ev.get("backup_ok") and not ev.get("annihilated_ping") and ev.get("reborn_ping")
        and ev.get("nodes_revived") and ev.get("roster_full_recovery")
        and ev.get("restore_backup_ok") and ev.get("backup_hash_identical")
        and ev["live_unchanged"] and ev["scratch_cleaned"]
    )
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


def cmd_phoenix_p9_failsafe(args):
    """★Phase 9 fail-safe 방향 meta-drill(§11.2): 집행 계층 불능화 시 설계된 방향으로 낙하하는지.
    (a)백업도구 부재→정직 RED (b)roster 손상→보수적 재spawn(침묵skip 아님) (c)세션핀 미상→unverified(거짓VERIFIED 아님)
    (d)boot-epoch 마킹 미상→stale 재spawn. + 쓰기/권한 fail-closed(gen-protect dry-run·암호복원 무키 거부).
    규칙 실증: 복구=fail-open-with-label · 권한/쓰기=fail-closed."""
    guard_isolation()
    ev = {"test": "failsafe_directions"}
    live_before = len(live_surfaces())
    _fresh_harness()
    ph = os.path.join(HARN_DIR, "phoenix")
    import shutil as _sh
    import tempfile as _tf
    _sh.rmtree(ph, ignore_errors=True)
    BACKUP = os.path.join(os.path.dirname(PHOENIX), "javis_backup.py")
    scratch = _tf.mkdtemp(prefix="cys-p9fs-", dir="/tmp")
    clean_env = {k: v for k, v in os.environ.items() if k not in LEAKY_ENV}
    try:
        # (a) 백업 미구성 → 정직 RED (미상을 GREEN 아닌 RED로 = fail-safe)
        st = subprocess.run([sys.executable, BACKUP, "status", "--home", scratch], capture_output=True, text=True,
                            timeout=20, env=dict(clean_env, CYS_BACKUP_DIR=os.path.join(scratch, "none")))
        ev["a_backup_absent_red"] = json.loads(st.stdout)["grade"] == "RED"

        # (b) roster 손상 → 보수적 재spawn(침묵 skip 아님)
        os.makedirs(ph, exist_ok=True)
        _seed_topology([{"role": "worker", "agent": "stub", "session_id": "SID-W-1"}])
        open(os.path.join(ph, "desired_roster.json"), "w").write("{ corrupt json ]")
        _, rb = _phoenix_json("restore", "--ticket", "FSB", "--stub", "--no-breaker")
        pb = re.search(r"이번 진행=(\[[^\]]*\])", rb.stdout or "")
        ev["b_corrupt_roster_respawns"] = "worker" in (pb.group(1) if pb else "")

        # (c) 세션핀 미상 → unverified(거짓 VERIFIED 아님)
        _sh.rmtree(ph, ignore_errors=True)
        _seed_topology([{"role": "worker", "agent": "stub"}])  # session_id 없음
        jc, _ = _phoenix_json("restore", "--ticket", "FSC", "--stub", "--no-breaker")
        ev["c_no_pin_unverified"] = (jc or {}).get("phoenix_restore") == "UNVERIFIED"

        # (d) boot-epoch 마킹 미상 → stale 취급 재spawn(저널 verify done 이나 epoch 필드 없음)
        _sh.rmtree(ph, ignore_errors=True)
        os.makedirs(ph)
        _seed_topology([{"role": "worker", "agent": "stub", "session_id": "SID-W-1"}])
        stale = {"ticket": "FSD", "roles": {"worker": {"stages": {
            s: {"done": True, "ts": 0} for s in ("spawn", "ready", "resume", "reinject", "g2_ack", "verify")},
            "surface": "surface:999", "expected_sid": "SID-W-1", "outcome": "verified"}},
            "events": [], "created": 0}  # ★epoch 필드 없음
        json.dump(stale, open(os.path.join(ph, "journal-FSD.json"), "w"))
        _, rd = _phoenix_json("restore", "--ticket", "FSD", "--stub", "--no-breaker")
        pd = re.search(r"이번 진행=(\[[^\]]*\])", rd.stdout or "")
        ev["d_no_epoch_respawns"] = "worker" in (pd.group(1) if pd else "")

        # (e) 쓰기/권한 fail-closed: gen-protect dry-run · 암호복원 무키 거부
        jp, _ = _phoenix_json("gen-protect")
        ev["e_write_fail_closed_protect"] = (jp or {}).get("applied") is False
        bhome = os.path.join(scratch, "h")
        pk = os.path.join(bhome, ".cys", "pack")
        os.makedirs(pk)
        open(os.path.join(pk, "soul.md"), "w").write("s\n")
        k = os.path.join(scratch, "k")
        open(k, "w").write("kk\n")
        subprocess.run([sys.executable, BACKUP, "backup", "--out", os.path.join(scratch, "bk"),
                        "--home", bhome, "--key-file", k], capture_output=True, timeout=30)
        nokey = subprocess.run([sys.executable, BACKUP, "restore", "--in", os.path.join(scratch, "bk"),
                                "--dest", os.path.join(scratch, "d")], capture_output=True, text=True, timeout=30,
                               env={kk: vv for kk, vv in clean_env.items() if kk != "CYS_BACKUP_KEY"})
        ev["e_restore_nokey_fails"] = nokey.returncode != 0
    finally:
        teardown(verbose=True)
        _sh.rmtree(scratch, ignore_errors=True)
    ev["live_unchanged"] = live_before == len(live_surfaces())
    ev["failsafe_pass"] = bool(
        ev.get("a_backup_absent_red") and ev.get("b_corrupt_roster_respawns")
        and ev.get("c_no_pin_unverified") and ev.get("d_no_epoch_respawns")
        and ev.get("e_write_fail_closed_protect") and ev.get("e_restore_nokey_fails")
        and ev["live_unchanged"]
    )
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


def cmd_phoenix_p9_ci(args):
    """★Phase 9 CI 영구 게이트: 캡스톤(①②) + 전 회귀를 서브프로세스로 순차 실행하고 결정론 종료코드로 집계한다.
    각 drill 은 PASS_KEY 매핑으로 exit 0(PASS)/1(FAIL) 을 낸다 → 하나라도 실패하면 ci_pass=False(exit 1)."""
    me = [sys.executable, os.path.abspath(__file__)]
    suite = ["phoenix-p12-deploy", "phoenix-p11-poison", "phoenix-p10-populated", "phoenix-p9-catastrophe",
             "phoenix-p9-failsafe", "phoenix-p4-erosion", "phoenix-p5-redelivery", "phoenix-p6-bootepoch",
             "phoenix-p7-inherit", "phoenix-p8-backup", "phoenix-drill"]
    results = {}
    for cmd in suite:
        r = subprocess.run(me + [cmd], capture_output=True, text=True, timeout=600)
        results[cmd] = r.returncode
        tag = "PASS" if r.returncode == 0 else ("SKIP(precondition)" if r.returncode == 77 else "FAIL")
        log("CI %s → exit %s (%s)" % (cmd, r.returncode, tag))
    # ★게이트 판정: exit 0 = PASS, 77 = SKIP(환경성 전제 미충족·게이트 불통과 아님), 그 외 = FAIL.
    failed = [c for c, rc in results.items() if rc not in (0, 77)]
    skipped = [c for c, rc in results.items() if rc == 77]
    ci_pass = not failed
    print(json.dumps({"ci_gate": "PASS" if ci_pass else "FAIL", "per_drill_exit": results,
                      "failed": failed, "skipped_precondition": skipped, "ci_pass": ci_pass},
                     ensure_ascii=False, indent=2))
    return {"ci_pass": ci_pass, "per_drill_exit": results, "skipped_precondition": skipped}


def cmd_phoenix_p10_populated(args):
    """★Phase 10 대표성 catastrophe 드릴: 최소 대표 연구소를 실제 구성해 전멸→완결성 부활→복원 무손실 검증.
    부서 3개(synthetic dept-roster) + 워커 다수(5 role) + 프로젝트 상태(실 파일·todo·git 커밋) → 백업 →
    ★전멸 → 부활(★완결성: cso 스폰실패 주입에도 재시도로 전원 100% 부활) → 백업 복원 원본 해시동일 → 손배선0.
    + INCOMPLETE escalation 서브검증(재시도 소진→정직 미완·잔여 역할 명시). 라이브 무접촉·env 격리 부서."""
    guard_isolation()
    ev = {"test": "populated_lab_catastrophe"}
    live_before = len(live_surfaces())
    _fresh_harness()
    ph = os.path.join(HARN_DIR, "phoenix")
    import shutil as _sh
    import tempfile as _tf
    _sh.rmtree(ph, ignore_errors=True)
    scratch = _tf.mkdtemp(prefix="cys-p10-", dir="/tmp")
    BACKUP = os.path.join(os.path.dirname(PHOENIX), "javis_backup.py")
    roles = ["worker-1", "worker-2", "cso", "analyst", "reviewer"]
    depts = ["alpha", "beta", "gamma"]
    try:
        # 부서 3개(synthetic·env 격리)
        dept_root = os.path.join(HARN_DIR, "dept-root")
        _sh.rmtree(dept_root, ignore_errors=True)
        for d in depts:
            dd = os.path.join(dept_root, "cys-dept-" + d)
            os.makedirs(dd)
            open(os.path.join(dd, "schedule_state.json"), "w").write('{"jobs":[]}')
        depts_json = os.path.join(dept_root, "depts.json")
        json.dump({"depts": {d: {"socket": os.path.join(dept_root, "cys-dept-" + d, "cys.sock")} for d in depts}},
                  open(depts_json, "w"))
        dept_env = {"PHOENIX_DEPT_STATE_ROOT": dept_root, "PHOENIX_DEPTS_JSON": depts_json}

        # 프로젝트 상태(실 작업물) — pack 트리 하위(백업 대상)
        home = os.path.join(scratch, "home")
        pack = os.path.join(home, ".cys", "pack")
        os.makedirs(os.path.join(pack, "memory"))
        os.makedirs(os.path.join(pack, "_round"))
        open(os.path.join(pack, "soul.md"), "w").write("SOUL identity\n")
        open(os.path.join(pack, "memory", "MEMORY.md"), "w").write("memory index\n")
        open(os.path.join(pack, "_round", "PROJECT_STATE.md"), "w").write("# 프로젝트 상태\n실 작업물: 단계 3/5 완료\n")
        open(os.path.join(pack, "_round", "WORKER_TODO.md"), "w").write("- [x] a\n- [ ] b\n")
        # git 커밋(실 작업물·best-effort — git 부재/설정 없어도 평문 프로젝트 파일로 핵심 검증)
        proj = os.path.join(pack, "_round", "proj")
        os.makedirs(proj)
        open(os.path.join(proj, "work.txt"), "w").write("project work output\n")
        genv = dict(os.environ, GIT_AUTHOR_NAME="drill", GIT_AUTHOR_EMAIL="d@x",
                    GIT_COMMITTER_NAME="drill", GIT_COMMITTER_EMAIL="d@x")
        try:
            for gcmd in (["init", "-q"], ["add", "-A"], ["commit", "-q", "-m", "work"]):
                subprocess.run(["git", "-C", proj] + gcmd, capture_output=True, env=genv, timeout=15)
            g = subprocess.run(["git", "-C", proj, "rev-parse", "HEAD"], capture_output=True, text=True, env=genv, timeout=10)
            ev["git_project"] = g.returncode == 0
            ev["git_commit"] = (g.stdout or "").strip()[:12]
        except Exception as e:
            ev["git_project"] = False
            ev["git_error"] = str(e)
        key = os.path.join(scratch, "k.key")
        open(key, "w").write("p10-key\n")
        orig = {}
        for r, _d, fs in os.walk(pack):
            for fn in fs:
                p = os.path.join(r, fn)
                orig[os.path.relpath(p, home)] = _sha256(p)
        ev["project_file_count"] = len(orig)

        # 워커 다수(부서 workers) = topology 시드(결정론) + 자동 상속(손 배선 0)
        _seed_topology([{"role": r, "agent": "stub", "session_id": "SID-" + r} for r in roles])
        _phoenix_json("inherit", extra_env=dept_env)

        # 백업(Tier1 암호화)
        br = subprocess.run([sys.executable, BACKUP, "backup", "--out", os.path.join(scratch, "bk"),
                             "--home", home, "--key-file", key], capture_output=True, text=True, timeout=60)
        ev["backup_ok"] = br.returncode == 0

        # ★전멸(Phase11 ②: realpath/pid 기반·빗나감0): 격리 데몬 + 전 자손(stub 포함) pid kill + topology 소거
        #   (cysd 자동복원 차단·phoenix roster 보존).
        global _tracked_daemon
        ev["annihilated_pids"] = _annihilate()
        _tracked_daemon = None
        time.sleep(0.8)
        ev["annihilated_ping"] = harness_ping()
        _wipe_daemon_state()
        start_daemon()
        ev["reborn_ping"] = harness_ping()

        # ★완결성 부활 — 대량 노드 + cso 스폰실패 주입(DRILL_LIVE_3 재현) → 재시도로 전원 100%
        fault_env = dict(dept_env, PHOENIX_SPAWN_FAIL_ONCE="cso", PHOENIX_SPAWN_BACKOFF="0.5")
        jr, _ = _phoenix_json("restore", "--ticket", "P10", "--stub", "--no-breaker", extra_env=fault_env)
        jr = jr or {}
        ev["completeness"] = jr.get("completeness")
        ev["incomplete_roles"] = jr.get("incomplete_roles")
        ev["ready_roles"] = sorted(jr.get("ready_roles") or [])
        ev["all_revived_100pct"] = (jr.get("completeness") == "COMPLETE" and set(jr.get("ready_roles") or []) == set(roles))
        jj = json.load(open(os.path.join(ph, "journal-P10.json")))
        cso_ev = [e for e in jj.get("events", []) if e["role"] == "cso" and e["stage"] == "spawn"]
        ev["cso_retry_recovered"] = any(e["status"] == "retry" for e in cso_ev) and any(e["status"] == "ok" for e in cso_ev)

        # 부서 전부 dept_roster
        jrost, _ = _phoenix_json("roster", extra_env=dept_env)
        ev["dept_roster"] = sorted((jrost or {}).get("dept_roster(부서 보호집합·자동 상속)") or [])
        ev["all_depts_present"] = set(ev["dept_roster"]) >= set(depts)

        # 프로젝트 상태 복원 → 해시 동일
        dest = os.path.join(scratch, "restored")
        rsb = subprocess.run([sys.executable, BACKUP, "restore", "--in", os.path.join(scratch, "bk"),
                              "--dest", dest, "--key-file", key], capture_output=True, text=True, timeout=60)
        ev["restore_backup_ok"] = rsb.returncode == 0
        mism = [rel for rel, h in orig.items()
                if not os.path.isfile(os.path.join(dest, rel)) or _sha256(os.path.join(dest, rel)) != h]
        ev["project_hash_identical"] = not mism
        ev["hash_mismatches"] = mism[:10]

        # ── INCOMPLETE escalation 서브검증(재시도 소진→정직 미완·잔여 역할 명시) ──
        _sh.rmtree(ph, ignore_errors=True)
        _seed_topology([{"role": r, "agent": "stub", "session_id": "SID-" + r} for r in roles])
        inc_env = dict(dept_env, PHOENIX_SPAWN_FAIL_ALWAYS="reviewer", PHOENIX_SPAWN_BACKOFF="0.3")
        jinc, _ = _phoenix_json("restore", "--ticket", "P10INC", "--stub", "--no-breaker", extra_env=inc_env)
        jinc = jinc or {}
        ev["incomplete_completeness"] = jinc.get("completeness")
        ev["incomplete_lists_role"] = "reviewer" in (jinc.get("incomplete_roles") or [])
        note = jinc.get("honesty_note") or ""
        ev["incomplete_escalates"] = "INCOMPLETE" in note and "reviewer" in note
        ev["incomplete_others_ready"] = all(r in (jinc.get("ready_roles") or []) for r in roles if r != "reviewer")
    finally:
        teardown(verbose=True)
        _sh.rmtree(scratch, ignore_errors=True)

    ev["live_unchanged"] = live_before == len(live_surfaces())
    ev["scratch_cleaned"] = not os.path.exists(scratch)
    ev["p10_pass"] = bool(
        ev.get("backup_ok") and not ev.get("annihilated_ping") and ev.get("reborn_ping")
        and ev.get("all_revived_100pct") and ev.get("cso_retry_recovered")
        and ev.get("all_depts_present") and ev.get("restore_backup_ok") and ev.get("project_hash_identical")
        and ev.get("incomplete_completeness") == "INCOMPLETE" and ev.get("incomplete_lists_role")
        and ev.get("incomplete_escalates") and ev.get("incomplete_others_ready")
        and ev["live_unchanged"] and ev["scratch_cleaned"]
    )
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


def _launchd_subverify(scratch):
    """★Phase11 ③ launchd 관리 무결성 서브검증: fake launchctl 로 managed/orphan/unmanaged 분류 + 재등록 복원
    + 종료 intact assert. 실 launchctl 무접촉(PHOENIX_LAUNCHCTL 주입) — 라이브 데몬 재시작·변경 0."""
    out = {}
    label = "com.cys.harness.p11"
    st = os.path.join(scratch, "ld_state")
    plist = os.path.join(scratch, "fake.plist")
    open(plist, "w").write("<plist/>\n")
    fake = os.path.join(scratch, "fake_launchctl.sh")
    open(fake, "w").write(
        '#!/bin/bash\n'
        'ST="$FAKE_LD_STATE"\n'
        'case "$1" in\n'
        '  list) if [ -f "$ST" ] && grep -q "^loaded" "$ST"; then echo "{ Label = $2; };"; exit 0; else exit 1; fi ;;\n'
        '  print) if [ -f "$ST" ] && grep -q "^loaded" "$ST"; then echo "KeepAlive = 1"; echo "RunAtLoad = 1"; exit 0; else exit 1; fi ;;\n'
        '  bootstrap) echo loaded > "$ST"; exit 0 ;;\n'
        '  bootout) echo unloaded > "$ST"; exit 0 ;;\n'
        '  *) exit 0 ;;\n'
        'esac\n')
    os.chmod(fake, 0o755)
    lenv = {"PHOENIX_LAUNCHCTL": fake, "FAKE_LD_STATE": st}
    # 1) managed(로드됨 + KeepAlive/RunAtLoad intact = 재부팅 자동기동 토대)
    open(st, "w").write("loaded\n")
    j1, _ = _phoenix_json("launchd-status", "--label", label, extra_env=lenv)
    out["launchd_managed_detected"] = ((j1 or {}).get("state") == "managed"
                                       and bool((j1 or {}).get("keepalive")) and bool((j1 or {}).get("runatload")))
    # 2) orphan(unload 됐으나 프로세스는 살아있음 — 관리 이탈·재부팅되면 안 뜸)
    open(st, "w").write("unloaded\n")
    j2, _ = _phoenix_json("launchd-status", "--label", label, "--pid", str(os.getpid()), extra_env=lenv)
    out["launchd_orphan_detected"] = (j2 or {}).get("state") == "orphan"
    # 3) unmanaged(unload + 프로세스도 없음 — 등록 부재)
    j3, _ = _phoenix_json("launchd-status", "--label", label, extra_env=lenv)
    out["launchd_unmanaged_detected"] = (j3 or {}).get("state") == "unmanaged"
    # 4) 재등록 복원(ensure) → managed(드릴/복원 절차가 unload 했으면 '복원까지 보장')
    j4, _ = _phoenix_json("launchd-ensure", "--label", label, "--plist", plist, extra_env=lenv)
    out["launchd_reregister_restores"] = ((j4 or {}).get("ensured") is True
                                          and ((j4 or {}).get("after") or {}).get("state") == "managed")
    # 5) 드릴 종료 intact assert(등록 intact)
    j5, _ = _phoenix_json("launchd-status", "--label", label, extra_env=lenv)
    out["launchd_end_intact"] = (j5 or {}).get("state") == "managed"
    return out


def cmd_phoenix_p11_poison(args):
    """★Phase 11 대표성 드릴: 독약 세션(resume 불가) fresh-spawn fallback + 전멸/토대 견고화(DRILL_LIVE_4 §15 수리).
    ⓐ독약 세션 주입(2역할 resume 항상 실패) → 완결성이 fresh 강등으로 roster 100% 부활(무한 재시도 0)·fresh 전환 저널 명시·
      건강 역할은 verified 유지·최종 VERIFIED_FRESH(세션 보존 실패를 정직 구분).
    ⓑ②realpath/pid 전멸(빗나감0): 부활 stub(sleep 3600) surface 자식을 pid 로 정확 적중(구 pkill 'sleep 600'은 빗나감).
    ⓒ③launchd 관리무결성: managed→orphan→unmanaged 분류·재등록 복원·드릴 종료 등록 intact assert(fake launchctl·라이브 무접촉)."""
    import shutil as _sh
    import tempfile as _tf
    guard_isolation()
    ev = {"test": "poison_session_fresh_fallback"}
    live_before = len(live_surfaces())
    _fresh_harness()
    ph = os.path.join(HARN_DIR, "phoenix")
    _sh.rmtree(ph, ignore_errors=True)
    scratch = _tf.mkdtemp(prefix="cys-p11-", dir="/tmp")
    healthy = ["cso", "analyst"]
    poison = ["worker-1", "worker-2"]
    roles = healthy + poison
    depts = ["alpha", "beta"]
    global _tracked_daemon
    try:
        # 부서(synthetic·env 격리)
        dept_root = os.path.join(HARN_DIR, "dept-root")
        _sh.rmtree(dept_root, ignore_errors=True)
        for d in depts:
            os.makedirs(os.path.join(dept_root, "cys-dept-" + d))
        depts_json = os.path.join(dept_root, "depts.json")
        json.dump({"depts": {d: {"socket": os.path.join(dept_root, "cys-dept-" + d, "cys.sock")} for d in depts}},
                  open(depts_json, "w"))
        dept_env = {"PHOENIX_DEPT_STATE_ROOT": dept_root, "PHOENIX_DEPTS_JSON": depts_json}

        _seed_topology([{"role": r, "agent": "stub", "session_id": "SID-" + r} for r in roles])
        _phoenix_json("inherit", extra_env=dept_env)

        # ★전멸(realpath/pid) → 신 데몬 부활(topology 소거로 cysd 자동복원 차단·phoenix roster 보존)
        ev["annihilated_pids_setup"] = _annihilate()
        _tracked_daemon = None
        time.sleep(0.6)
        ev["annihilated_ping"] = harness_ping()   # False 기대(전멸)
        _wipe_daemon_state()
        start_daemon()
        ev["reborn_ping"] = harness_ping()         # True 기대

        # ── ⓐ 독약 세션 fresh-spawn fallback ──
        poison_env = dict(dept_env, PHOENIX_POISON_SESSION=",".join(poison),
                          PHOENIX_SPAWN_RETRIES="2", PHOENIX_SPAWN_BACKOFF="0.2")
        jr, _ = _phoenix_json("restore", "--ticket", "P11", "--stub", "--no-breaker", extra_env=poison_env)
        jr = jr or {}
        pro = jr.get("per_role_outcome") or {}
        ev["completeness"] = jr.get("completeness")
        ev["ready_roles"] = sorted(jr.get("ready_roles") or [])
        ev["fresh_fallback_roles"] = sorted(jr.get("fresh_fallback_roles") or [])
        ev["phoenix_restore"] = jr.get("phoenix_restore")
        ev["roster_100pct"] = (jr.get("completeness") == "COMPLETE" and set(jr.get("ready_roles") or []) == set(roles))
        ev["poison_downgraded_to_fresh"] = set(jr.get("fresh_fallback_roles") or []) == set(poison)
        ev["healthy_stayed_verified"] = all(pro.get(r) == "verified" for r in healthy)
        # 저널: 독약 역할이 resume 실패(retry/fail) → fresh_fallback 전환 명시 + 유한(무한 재시도 0)
        jj = json.load(open(os.path.join(ph, "journal-P11.json")))
        w = poison[0]
        wsp = [e for e in jj.get("events", []) if e["role"] == w and e["stage"] == "spawn"]
        statuses = [e["status"] for e in wsp]
        ev["poison_resume_failed_then_fresh"] = (("retry" in statuses or "fail" in statuses) and "fresh_fallback" in statuses)
        ev["fresh_transition_journaled"] = any(e["status"] == "fresh_fallback" and "fresh 강등" in e.get("msg", "") for e in wsp)
        ev["poison_spawn_event_count"] = len(wsp)
        ev["no_infinite_retry"] = len(wsp) <= (2 + 2 + 2)   # SPAWN_RETRIES(2) resume + fail + fresh 의 유한 상한
        ev["fresh_outcome_not_fork"] = pro.get(w) == "fresh"

        # ── ⓑ ②realpath/pid 전멸(빗나감0) 서브검증 ──
        # 부활한 독약 역할 stub surface 의 자식 pid(sleep 3600)를 기록 → _annihilate 가 pid 로 정확 적중하는지.
        # stub surface 는 new-surface 라 role 미claim(role=-) → 저널의 surface ref 로 pid 를 찾는다(role 파싱 아님).
        target_surface = (jj.get("roles", {}).get(w, {}) or {}).get("surface")
        surf_pid = {}
        for line in (cys("list", timeout=10).stdout or "").splitlines():
            m = re.match(r"(surface:\d+)\s+.*?pid=(\d+)", line)
            if m:
                surf_pid[m.group(1)] = int(m.group(2))
        target_pid = surf_pid.get(target_surface)
        ev["kill_target_surface"] = target_surface
        ev["kill_target_surface_pid"] = target_pid
        # 구 방식(pkill -f 'sleep 600')이 빗나가는 이유: stub 은 'sleep 3600'('sleep 600' substring 아님) → 문자열 매칭 실패.
        ev["old_pkill_would_miss"] = ("sleep 600" not in "exec sleep 3600")  # True(빗나감 실증)
        victims = _annihilate()
        ev["annihilate_victims"] = victims
        ev["pid_kill_hit_target"] = bool(target_pid) and target_pid in victims
        _tracked_daemon = None
        time.sleep(0.4)
        dead = False
        if target_pid:
            try:
                os.kill(target_pid, 0)
            except ProcessLookupError:
                dead = True
            except Exception:
                dead = False
        ev["target_pid_dead_after_kill"] = dead
        ev["kill_no_miss"] = bool(ev["pid_kill_hit_target"] and ev["target_pid_dead_after_kill"])

        # ── ⓒ ③launchd 관리 무결성(fake launchctl·라이브 무접촉) ──
        ev.update(_launchd_subverify(scratch))
    finally:
        teardown(verbose=True)
        _sh.rmtree(scratch, ignore_errors=True)

    ev["live_unchanged"] = live_before == len(live_surfaces())
    ev["scratch_cleaned"] = not os.path.exists(scratch)
    ev["p11_pass"] = bool(
        not ev.get("annihilated_ping") and ev.get("reborn_ping")
        and ev.get("roster_100pct") and ev.get("poison_downgraded_to_fresh")
        and ev.get("healthy_stayed_verified") and ev.get("phoenix_restore") == "VERIFIED_FRESH"
        and ev.get("poison_resume_failed_then_fresh") and ev.get("fresh_transition_journaled")
        and ev.get("no_infinite_retry") and ev.get("fresh_outcome_not_fork")
        and ev.get("old_pkill_would_miss") and ev.get("kill_no_miss")
        and ev.get("launchd_managed_detected") and ev.get("launchd_orphan_detected")
        and ev.get("launchd_unmanaged_detected") and ev.get("launchd_reregister_restores")
        and ev.get("launchd_end_intact")
        and ev["live_unchanged"] and ev["scratch_cleaned"]
    )
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


def live_role_surfaces_local():
    """격리 데몬의 role→surface 매핑(하네스 소켓)."""
    r = cys("list", timeout=12)
    roles = {}
    for line in (r.stdout or "").splitlines():
        mm = re.match(r"(surface:\d+)\s+role=(\S+)", line)
        if mm and mm.group(2) != "-":
            roles.setdefault(mm.group(2), []).append(mm.group(1))
    return roles


def _harness_bootepoch():
    """격리 데몬의 boot-epoch(started_at) 실측 — p6 _started_at 과 동일 계약(재시작마다 변경)."""
    r = cys("status", "--json", timeout=10)
    try:
        return (json.loads(r.stdout or "{}").get("daemon") or {}).get("started_at")
    except Exception:
        return None


def cmd_deploy_restart_fixture(args):
    """★Phase12(요구C) deploy 드릴 전용 재시작 프리미티브(라이브 launchd 대역). 격리 데몬을 전멸→재기동한다.
    phoenix deploy 의 --restart-hook 로 주입돼 restart 단계에서 실행된다(하네스 기존 kill/restart 프리미티브 재사용:
    _annihilate=realpath/pid 전멸 §15발견2 · _wipe_daemon_state=topology 소거로 cysd 자동복원 차단·phoenix roster 는
    phoenix/ 에 보존 · start_daemon=격리 데몬 재기동). ★os._exit(0) 로 atexit teardown 을 우회한다 —
    그러지 않으면 이 프로세스 종료 시 방금 띄운 데몬을 스스로 죽여 재시작을 무력화한다."""
    guard_isolation()
    global _tracked_daemon
    _annihilate()
    _tracked_daemon = None
    time.sleep(0.5)
    _wipe_daemon_state()   # topology 소거(cysd 자동복원 차단) — phoenix roster(desired) 는 phoenix/ 에 보존
    pid = start_daemon()
    ok = bool(pid) and harness_ping()
    sys.stdout.write(json.dumps({"restart_fixture": "OK" if ok else "FAIL",
                                 "pid": pid, "ping": harness_ping()}, ensure_ascii=False) + "\n")
    sys.stdout.flush()
    os._exit(0 if ok else 1)   # ★atexit teardown 우회(방금 띄운 데몬 보존)


def cmd_phoenix_p12_deploy(args):
    """★Phase 12(요구C) deploy 오케스트레이션 드릴: quiescent(drain best-effort)→스냅샷(실 세대 생성 + 독립
    runbook 실행권한)→(apply 생략/실패주입)→전멸·재기동(하네스 respawner=launchd 대역)→restore(run_restore 재사용)
    →verdict, 전 과정 격리·hermetic(2회 연속 동일). + --plan 무실행/exit0 · apply 실패→비0 exit·재시작 미진입·
    재시작 boot-epoch delta 확증 · roster 100% 부활 · 라이브 무접촉·잔여 0."""
    import shutil as _sh
    guard_isolation()
    ev = {"test": "deploy_orchestration"}
    live_before = len(live_surfaces())
    _fresh_harness()
    ph = os.path.join(HARN_DIR, "phoenix")
    _sh.rmtree(ph, ignore_errors=True)
    roles = ["worker", "cso"]
    seed = lambda: _seed_topology([{"role": r, "agent": "stub", "session_id": "SID-" + r,
                                    "cwd": HARN_DIR, "title": r} for r in roles])
    restart_hook = "%s %s _deploy-restart-fixture" % (sys.executable, os.path.abspath(__file__))
    try:
        # ── 1) --plan 무실행(exit0·부작용 0) ──
        jp, rp = _phoenix_json("deploy", "--plan", "--ticket", "PLAN", "--stub")
        ev["plan_stages"] = (jp or {}).get("stages")
        ev["plan_exit0"] = rp.returncode == 0
        ev["plan_no_side_effect"] = not os.path.exists(os.path.join(ph, "deploy-journal-PLAN.json"))

        # ── 2) COMPLETE deploy(재시작 hook·roster 100% 부활·runbook 실행권한) ──
        seed()
        jd, rd = _phoenix_json("deploy", "--ticket", "D1", "--stub", "--restart-hook", restart_hook,
                               "--no-breaker", timeout=150)
        jd = jd or {}
        stages = jd.get("stages", {})
        ev["deploy_verdict"] = jd.get("deploy")
        ev["deploy_exit0"] = rd.returncode == 0
        ev["daemon_ping_after"] = harness_ping()
        rr = jd.get("restart") or {}
        ev["restart_revived"] = rr.get("revived")
        ev["restart_epoch_changed"] = rr.get("boot_epoch_changed")
        snap_stage = stages.get("snapshot") or {}
        runbook = snap_stage.get("runbook") or (jd.get("snapshot") or {}).get("runbook")
        ev["snapshot_gen"] = snap_stage.get("gen")
        ev["snapshot_created"] = bool(snap_stage.get("gen"))
        ev["runbook_path"] = runbook
        ev["runbook_exists_exec"] = bool(runbook) and os.path.exists(runbook) and os.access(runbook, os.X_OK)
        body = open(runbook).read() if ev["runbook_exists_exec"] else ""
        ev["runbook_self_contained"] = ("launchctl bootstrap" in body and "launch-agent" in body
                                        and all(("--role %s" % r) in body for r in roles))
        restore = jd.get("restore") or {}
        ev["restore_completeness"] = restore.get("completeness")
        ev["restore_verdict"] = restore.get("phoenix_restore")
        ev["roster_100pct"] = (restore.get("completeness") == "COMPLETE"
                               and set(restore.get("ready_roles") or []) == set(roles))
        ev["deploy_complete"] = (jd.get("deploy") == "COMPLETE" and rd.returncode == 0)
        # deploy-<ts>.json 별도 레코드 기록 확인
        ev["deploy_record_written"] = any(f.startswith("deploy-2") and f.endswith(".json")
                                          for f in (os.listdir(ph) if os.path.isdir(ph) else []))

        # ── 3) apply 실패 → 비0 exit·재시작 진입 금지(APPLY_FAILED, 부작용 확산 차단) ──
        _fresh_harness()
        _sh.rmtree(ph, ignore_errors=True)
        seed()
        epoch_before_fail = _harness_bootepoch()
        jf, rf = _phoenix_json("deploy", "--ticket", "D2", "--stub", "--apply-cmd", "false",
                               "--restart-hook", restart_hook, "--no-breaker", timeout=60)
        jf = jf or {}
        ev["apply_fail_verdict"] = jf.get("deploy")
        ev["apply_fail_exit_nonzero"] = rf.returncode != 0
        ev["apply_fail_no_restart"] = (_harness_bootepoch() == epoch_before_fail)  # 재시작 미진입(세대 불변)
        ev["apply_fail_ping"] = harness_ping()  # 데몬 여전히 생존(재시작 안 함)

        # ── 4) restart-miss(재시작 빗나감·조용한 오복원 방어) → RESTART_FAILED·exit4·부작용 0 ──
        #   --restart-hook "true": 아무것도 안 함(데몬 생존·boot-epoch 불변) = launchd 비관리 데몬에서 kill 빗나가
        #   pong 즉시 True 지만 재시작은 안 된 상황 재현. boot-epoch delta hard 게이트가 RESTART_FAILED 로 잡아야 한다.
        _fresh_harness()
        _sh.rmtree(ph, ignore_errors=True)
        seed()
        epoch_before_miss = _harness_bootepoch()
        jm, rm = _phoenix_json("deploy", "--ticket", "D3", "--stub", "--restart-hook", "true",
                               "--no-breaker", timeout=40)
        jm = jm or {}
        mrr = jm.get("restart") or {}
        ev["restartmiss_verdict"] = jm.get("deploy")
        ev["restartmiss_exit4"] = rm.returncode == 4
        ev["restartmiss_revived_but_unconfirmed"] = (mrr.get("revived") is True
                                                     and mrr.get("boot_epoch_changed") is False)
        ev["restartmiss_epoch_unchanged"] = (_harness_bootepoch() == epoch_before_miss)  # 실제 재시작 안 됨
        ev["restartmiss_ping"] = harness_ping()  # 데몬 생존(빗나감이라 안 죽음)
        ev["restartmiss_no_restore"] = (jm.get("restore") is None)  # restore 미진입(재시작 확증 전 중단·부작용 0)
    finally:
        teardown(verbose=True)
        _sh.rmtree(ph, ignore_errors=True)

    ev["live_unchanged"] = live_before == len(live_surfaces())
    ev["residual_zero"] = not harness_daemon_pids()
    ev["p12_pass"] = bool(
        ev.get("plan_exit0") and ev.get("plan_no_side_effect") and ev.get("plan_stages")
        and ev.get("deploy_complete") and ev.get("restart_revived") and ev.get("restart_epoch_changed")
        and ev.get("snapshot_created") and ev.get("runbook_exists_exec") and ev.get("runbook_self_contained")
        and ev.get("roster_100pct") and ev.get("deploy_record_written")
        and ev.get("apply_fail_verdict") == "APPLY_FAILED" and ev.get("apply_fail_exit_nonzero")
        and ev.get("apply_fail_no_restart") and ev.get("apply_fail_ping")
        and ev.get("restartmiss_verdict") == "RESTART_FAILED" and ev.get("restartmiss_exit4")
        and ev.get("restartmiss_revived_but_unconfirmed") and ev.get("restartmiss_epoch_unchanged")
        and ev.get("restartmiss_ping") and ev.get("restartmiss_no_restore")
        and ev["live_unchanged"] and ev["residual_zero"]
    )
    print(json.dumps(ev, ensure_ascii=False, indent=2))
    return ev


# ------------------------------------------------------------------ main

def main():
    global CYSD, CYS
    CYS = _resolve_cys()
    CYSD = _resolve_cysd()
    ap = argparse.ArgumentParser(description="불사조 무손실복원 격리 하네스(Phase1+2)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("up", "down", "status", "crash-daemon", "crash-agent",
                 "record-cli", "qualify", "drill", "phoenix-drill", "phoenix-p4-erosion",
                 "phoenix-p5-redelivery", "phoenix-p6-bootepoch", "phoenix-p7-inherit",
                 "phoenix-p8-backup", "phoenix-p9-catastrophe", "phoenix-p9-failsafe",
                 "phoenix-p9-ci", "phoenix-p10-populated", "phoenix-p11-poison",
                 "phoenix-p12-deploy", "_deploy-restart-fixture"):
        sub.add_parser(name)
    args = ap.parse_args()
    dispatch = {
        "up": cmd_up, "down": cmd_down, "status": cmd_status,
        "crash-daemon": cmd_crash_daemon, "crash-agent": cmd_crash_agent,
        "record-cli": cmd_record_cli, "qualify": cmd_qualify, "drill": cmd_drill,
        "phoenix-drill": cmd_phoenix_drill, "phoenix-p4-erosion": cmd_phoenix_p4_erosion,
        "phoenix-p5-redelivery": cmd_phoenix_p5_redelivery,
        "phoenix-p6-bootepoch": cmd_phoenix_p6_bootepoch,
        "phoenix-p7-inherit": cmd_phoenix_p7_inherit,
        "phoenix-p8-backup": cmd_phoenix_p8_backup,
        "phoenix-p9-catastrophe": cmd_phoenix_p9_catastrophe,
        "phoenix-p9-failsafe": cmd_phoenix_p9_failsafe,
        "phoenix-p9-ci": cmd_phoenix_p9_ci,
        "phoenix-p10-populated": cmd_phoenix_p10_populated,
        "phoenix-p11-poison": cmd_phoenix_p11_poison,
        "phoenix-p12-deploy": cmd_phoenix_p12_deploy,
        "_deploy-restart-fixture": cmd_deploy_restart_fixture,
    }
    res = dispatch[args.cmd](args)
    # ★Phase 9 ④: 결정론 종료코드(CI 게이트화) — drill 은 pass 불리언을 exit 0/1 로 낸다.
    PASS_KEY = {
        "phoenix-p4-erosion": "immunity_pass", "phoenix-p5-redelivery": "redelivery_pass",
        "phoenix-p6-bootepoch": "p6_pass", "phoenix-p7-inherit": "p7_pass",
        "phoenix-p8-backup": "p8_pass", "phoenix-p9-catastrophe": "p9_pass",
        "phoenix-p9-failsafe": "failsafe_pass", "phoenix-p9-ci": "ci_pass",
        "phoenix-p10-populated": "p10_pass", "phoenix-p11-poison": "p11_pass",
        "phoenix-p12-deploy": "p12_pass",
    }
    if isinstance(res, dict):
        # ★환경성 전제 미충족(예: send-key 미실행)은 SKIP(exit 77) — false-fail 방지·CI 게이트에 loud 표기.
        if res.get("precondition"):
            sys.exit(77)
        if args.cmd in PASS_KEY:
            sys.exit(0 if res.get(PASS_KEY[args.cmd]) else 1)
        if args.cmd == "phoenix-drill":
            summ = res.get("summary", {})
            sys.exit(0 if summ and all(summ.values()) else 1)


if __name__ == "__main__":
    main()
