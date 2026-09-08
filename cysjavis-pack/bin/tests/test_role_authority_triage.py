#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_role_authority_triage.py — P6-role-authority 독립 판정관 회귀 핀 (0.14.31).

이 파일은 phase1d 잔여 쟁점을 **판정관이 직접 재현한** 최소 반례만 담는다.
각 핀은 기준 커밋 a7883b8(HEAD)에서 **적색**이며, 옆에 붙은 양성 대조는 녹색이어야 한다
(양성 대조가 없으면 "전부 막으면 통과"하는 공허한 핀이 된다).

  T1 게이트 제어 상태 봉인의 잔여 구멍 (reviewer-claude major "R2-1 mitigation incomplete")
     `_is_gate_state_path` 는 Write/Edit/직접 리다이렉트만 잡는다. reviewer 의 write-shell
     판정은 **denylist** 라 `sh -c '... > <cache>'` · `eval "... > <cache>"` 가 통과한다
     (CSO 는 allowlist 라 같은 문자열이 막힌다 = 구멍은 reviewer 경로 한쪽이다).
     귀결: 집행 대상이 남의 좌석 역할 캐시에 `cache-none` 을 심어 CSO 의
     `cys-dept launch/down/rotate`(exit 7)·`javis_org apply/destroy`(exit 3)를 잠근다.

  T2 두 층 슬러그 파리티 (reviewer-claude major / reviewer-codex minor #5)
     파이썬 `_slug` 는 입력의 `_` 를 보존하고 셸 `tr -cs` 는 **접는다**.
     같은 종단점이 두 파일이 되어 "두 층이 캐시를 공유한다"는 계약이 거짓이다.

  T3 데몬이 확정한 역할이 stale env 에 진다 (reviewer-codex blocking #1 · 정본 §8)
     `javis_org.require_cso`·`javis_snapshot.is_master`·`cys-dept`(:1267)는 데몬이
     **권위 있게** cso/master 라고 답해도 env 가 낡았으면 거부하고, cys-dept 는 데몬을
     **묻지도 않는다**. 실패 방향은 거부(§3-3)지만 정본 §8 의 목표는 미달이다.

  T4 프로세스 메모 키가 기본 종단점 입력을 빠뜨린다 (reviewer-codex major #2)
     소켓 지정이 없으면 신원은 `default:<XDG_STATE_HOME>:<HOME>` 인데 `_memo_key()` 는
     그 둘을 담지 않는다 — 한 프로세스에서 HOME 을 바꾸면 앞 데몬의 답이 재사용된다.

실행: CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" \
      CYS_PROBE_RUNS="$JAVIS_ROOT/probe_runs.jsonl" \
      python3 cysjavis-pack/bin/tests/test_role_authority_triage.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 ROLE-AUTHORITY-TRIAGE-OK.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
REPO = os.path.dirname(PACK)
LIB = os.path.join(PACK, "hooks", "_lib.sh")
GATE = os.path.join(PACK, "hooks", "role-capability-gate.sh")
DEPT = os.path.join(BIN, "cys-dept")
sys.path.insert(0, BIN)

import javis_role as JR  # noqa: E402

fails = []
_root = tempfile.mkdtemp(prefix="role-triage-")


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" - " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def equal(name, actual, expected):
    check(name, actual == expected, "got=%r want=%r" % (actual, expected))


# ── T1 게이트 제어 상태 봉인의 잔여 구멍 ─────────────────────────────────────
CACHE_REC = "/tmp/%s/role-7-default" % JR.CACHE_DIR_NAME


def gate(role, command):
    """게이트를 **종단 실행**한다 — deny 이면 True."""
    env = dict(os.environ)
    env["CYS_SURFACE_ROLE"] = role
    env["CYS_ROLE"] = role
    env.pop("CYS_SURFACE_ID", None)       # 데몬 조회 없이 env 역할로 판정
    payload = json.dumps({"tool_name": "Bash", "tool_input": {"command": command}})
    p = subprocess.run(["bash", GATE], input=payload, capture_output=True,
                       text=True, env=env, timeout=30)
    return '"permissionDecision":"deny"' in p.stdout


def t1():
    forge = '1757300000 - 12345 /tmp/d.sock'
    # 기준선(이미 녹색): 직접 리다이렉트는 두 역할 모두 막힌다.
    check("T1-base-py reviewer 직접 리다이렉트 deny",
          gate("reviewer-codex", 'echo "%s" > %s' % (forge, CACHE_REC)))
    check("T1-base-cso CSO 직접 리다이렉트 deny",
          gate("cso", 'echo "%s" > %s' % (forge, CACHE_REC)))
    # ★적색 예상: 같은 리다이렉트를 한 겹 감싸면 reviewer 경로가 통과한다.
    check("T1a reviewer `sh -c` 리다이렉트로 역할 캐시 위조 deny",
          gate("reviewer-codex", "sh -c 'echo \"%s\" > %s'" % (forge, CACHE_REC)))
    check("T1b reviewer `eval` 리다이렉트로 역할 캐시 위조 deny",
          gate("reviewer-codex", 'eval "echo x > %s"' % CACHE_REC))
    check("T1c reviewer `sh -c` 로 capgate 예산 카운터 위조 deny",
          gate("reviewer-codex", "sh -c 'echo 9 > /tmp/cys-capgate-role-3-x-y'"))
    # ★양성 대조(녹색 유지): 게이트 제어 상태가 **아닌** tmp 경로는 여전히 허용이다
    #   — T1a~c 가 "sh -c 를 통째로 막는다"로 고쳐지면 이 줄이 적색이 된다.
    check("T1d 양성대조 reviewer `sh -c` 로 일반 tmp 쓰기는 allow",
          not gate("reviewer-codex", "sh -c 'echo x > /tmp/reviewer-note.md'"))
    check("T1e 양성대조 CSO 는 같은 문자열을 이미 막는다(구멍은 reviewer 한쪽)",
          gate("cso", "sh -c 'echo x > %s'" % CACHE_REC))


# ── T2 두 층 슬러그 파리티 ───────────────────────────────────────────────────
def sh_slug(value):
    script = '. "$1"; cys_role_slug "$2"'
    p = subprocess.run(["/bin/sh", "-c", script, "slug", LIB, value],
                       capture_output=True, text=True, timeout=20)
    return p.stdout


def sh_cache_base(sockid, sid, tmpdir):
    """셸 짝의 **생산 진입점**으로 캐시 파일명을 얻는다(지역 재구현 금지)."""
    script = ('. "$1"; CYS_SOCKET="$2"; export CYS_SOCKET; '
              'cys_role_sock_id_init; p="$(cys_role_cache_path "$3")" || exit 9; '
              'printf "%s" "${p##*/}"')
    env = {"HOME": tmpdir, "TMPDIR": tmpdir, "PATH": "/usr/bin:/bin",
           "LC_ALL": "C", "XDG_STATE_HOME": tmpdir}
    p = subprocess.run(["/bin/sh", "-c", script, "cache", LIB, sockid, sid],
                       capture_output=True, text=True, env=env, timeout=20)
    return p.stdout


def py_cache_base(sockid, sid, tmpdir):
    saved = {k: os.environ.get(k) for k in ("CYS_SOCKET", "TMPDIR", "HOME", "XDG_STATE_HOME")}
    try:
        os.environ["CYS_SOCKET"] = sockid
        os.environ["TMPDIR"] = tmpdir
        os.environ["HOME"] = tmpdir
        os.environ["XDG_STATE_HOME"] = tmpdir
        return os.path.basename(JR._cache_path(sid))
    finally:
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def t2():
    # ★적색 예상: 입력에 `_` 가 있으면 두 층의 슬러그가 갈린다.
    for value in ("/tmp/a__b.sock", "/tmp/_role.sock", "/tmp/x/__y.sock"):
        equal("T2a 슬러그 파리티 %s" % value, JR._slug(value), sh_slug(value))
    # 양성 대조(이미 녹색): `_` 가 없으면 갈리지 않는다 — 이 핀이 파리티 자체를
    # 부정하는 것이 아니라 `_` 하나를 지목한다는 증거.
    for value in ("/tmp/a/b.sock", "default:/x:/y", "/tmp/한글/소켓.sock"):
        equal("T2b 양성대조 슬러그 파리티 %s" % value, JR._slug(value), sh_slug(value))
    # 생산 진입점 대조: 같은 종단점 → 같은 캐시 파일명이어야 계약이 참이다.
    box = os.path.join(_root, "t2box")
    os.makedirs(box, exist_ok=True)
    equal("T2c 캐시 파일명 파리티 /tmp/a__b.sock",
          py_cache_base("/tmp/a__b.sock", "7", box),
          sh_cache_base("/tmp/a__b.sock", "7", box))
    equal("T2d 양성대조 캐시 파일명 파리티 /tmp/a/b.sock",
          py_cache_base("/tmp/a/b.sock", "7", box),
          sh_cache_base("/tmp/a/b.sock", "7", box))


# ── T3 데몬 확정 역할이 stale env 에 진다 ────────────────────────────────────
class _Stub(object):
    """`resolve_role_detail` 을 고정 답으로 바꾼다(디스크·데몬 무접촉)."""

    def __init__(self, role, src):
        self.role, self.src = role, src

    def __enter__(self):
        self._saved = JR.resolve_role_detail
        JR.resolve_role_detail = lambda: (self.role, self.src)
        return self

    def __exit__(self, *a):
        JR.resolve_role_detail = self._saved
        return False


def _require_cso_rc(env_role, role, src):
    import javis_org
    saved = os.environ.get("CYS_ROLE")
    try:
        if env_role is None:
            os.environ.pop("CYS_ROLE", None)
        else:
            os.environ["CYS_ROLE"] = env_role
        with _Stub(role, src):
            try:
                javis_org.require_cso()
                return 0
            except SystemExit as e:
                return int(e.code or 0)
    finally:
        if saved is None:
            os.environ.pop("CYS_ROLE", None)
        else:
            os.environ["CYS_ROLE"] = saved


def t3():
    # ★적색 예상: 데몬이 권위 있게 cso 라고 답했는데 env 가 낡아서 거부된다.
    equal("T3a require_cso: 데몬 권위 cso + stale env=worker → 통과",
          _require_cso_rc("worker", "cso", JR.SOURCE_DAEMON), 0)
    equal("T3b require_cso: 데몬 권위 cso + env 없음 → 통과",
          _require_cso_rc(None, "cso", JR.SOURCE_DAEMON), 0)
    # 양성 대조(녹색 유지): 데몬이 아니라고 하면 env 가 cso 여도 여전히 거부다.
    equal("T3c 양성대조 require_cso: 데몬 권위 worker + env=cso → 거부",
          _require_cso_rc("cso", "worker", JR.SOURCE_DAEMON), 3)
    equal("T3d 양성대조 require_cso: 판정 불가 + env=worker → 거부",
          _require_cso_rc("worker", "worker", JR.SOURCE_ENV_ROLE), 3)

    import javis_snapshot
    saved = os.environ.get("CYS_ROLE")
    saved_sid = os.environ.get("CYS_SURFACE_ID")
    try:
        os.environ["CYS_ROLE"] = "worker"
        os.environ["CYS_SURFACE_ID"] = "12"   # 주소는 있다 — 갈리는 것은 대장뿐이다
        with _Stub("master", JR.SOURCE_DAEMON):
            ok, why = javis_snapshot.is_master()
        check("T3e is_master: 데몬 권위 master + stale env=worker → True", ok, str(why))
        with _Stub("worker", JR.SOURCE_DAEMON):
            ok2, _ = javis_snapshot.is_master()
        check("T3f 양성대조 is_master: 데몬 권위 worker → False", not ok2)
    finally:
        for k, v in (("CYS_ROLE", saved), ("CYS_SURFACE_ID", saved_sid)):
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def t3_dept():
    """cys-dept lifecycle 게이트는 데몬을 **묻기도 전에** env 로 exit 7 한다(:1267)."""
    box = os.path.join(_root, "dept")
    home = os.path.join(box, "home")
    tmp = os.path.join(box, "tmp")
    fakebin = os.path.join(home, ".local", "bin")
    log = os.path.join(box, "stub.log")
    for d in (fakebin, tmp, os.path.join(home, ".cys")):
        os.makedirs(d, exist_ok=True)
    reg = os.path.join(home, ".cys", "depts.json")
    with open(reg, "w", encoding="utf-8") as f:
        f.write('{"depts":{}}')
    open(log, "w").close()
    stub = os.path.join(fakebin, "cys")
    with open(stub, "w", encoding="utf-8") as f:
        f.write('#!/bin/sh\nprintf "%s\\n" "$*" >> "$STUB_LOG"\nprintf "cso\\n"\nexit 0\n')
    os.chmod(stub, 0o700)
    py = os.path.join(fakebin, "python3")
    if not os.path.exists(py):
        os.symlink(sys.executable, py)
    env = {"HOME": home, "TMPDIR": tmp, "PATH": "%s:/usr/bin:/bin:/usr/sbin:/sbin" % fakebin,
           "CYS_DEPTS_JSON": reg, "CYS_PACK_DIR": PACK,
           "CYS_STATE_DIR": os.path.join(home, ".cys", "state"),
           "XDG_STATE_HOME": os.path.join(home, ".local", "state"),
           "CYS_PY": sys.executable, "PYTHONDONTWRITEBYTECODE": "1", "LC_ALL": "C",
           "STUB_LOG": log, "CYS_SURFACE_ID": "12", "CYS_ROLE": "worker"}
    p = subprocess.run(["/usr/bin/env", "-i"] + ["%s=%s" % (k, v) for k, v in env.items()]
                       + [DEPT, "down", "no-such-dept-triage"],
                       env={}, cwd=home, capture_output=True, text=True, timeout=30)
    with open(log, encoding="utf-8") as f:
        calls = [ln.strip() for ln in f if ln.strip()]
    # ★적색 예상: `surface-role` 조회가 **한 번도** 나가지 않는다(env 로 먼저 죽는다).
    check("T3g cys-dept lifecycle 게이트는 데몬에게 먼저 묻는다",
          any(c == "surface-role" for c in calls),
          "rc=%s calls=%r" % (p.returncode, calls))


# ── T4 프로세스 메모 키가 기본 종단점 입력을 빠뜨린다 ────────────────────────
def t4():
    box = os.path.join(_root, "memo")
    home_a = os.path.join(box, "A")
    home_b = os.path.join(box, "B")
    tmp = os.path.join(box, "tmp")
    for d in (home_a, home_b, tmp):
        os.makedirs(d, exist_ok=True)
    keys = ("CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET", "CYS_SURFACE_ID",
            "HOME", "XDG_STATE_HOME", "TMPDIR")
    saved = {k: os.environ.get(k) for k in keys}
    saved_q = JR._query_daemon
    saved_memo = JR._MEMO
    seen = []
    try:
        for k in ("CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET"):
            os.environ.pop(k, None)
        os.environ["CYS_SURFACE_ID"] = "7"
        os.environ["TMPDIR"] = tmp
        os.environ["XDG_STATE_HOME"] = ""

        def q():
            h = os.environ.get("HOME", "")
            seen.append(h)
            return ("cso" if h == home_a else "worker"), True

        JR._query_daemon = q
        JR._MEMO = None
        os.environ["HOME"] = home_a
        r1 = JR.resolve_role_detail()
        os.environ["HOME"] = home_b
        r2 = JR.resolve_role_detail()
        equal("T4a 첫 해소는 A 데몬의 답", r1[0], "cso")
        # ★적색 예상: HOME 이 바뀌어 디스크 신원이 달라졌는데 메모가 A 의 답을 되돌린다.
        equal("T4b HOME 이 바뀌면 새 종단점에 다시 묻는다", r2[0], "worker")
        check("T4c 두 종단점 = 조회 2회", len(seen) == 2, "queries=%r" % (seen,))
    finally:
        JR._query_daemon = saved_q
        JR._MEMO = saved_memo
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


# ── T5 Windows 정규 종단점이 두 층의 디스크 캐시를 통째로 끈다 ───────────────
#   러너 없이도 판정 가능하다: 종단점 **생산자**가 이 저장소 안에 있다
#   (`bin/cys-dept:48` MINGW/MSYS/CYGWIN → `\\.\pipe\cys-dept-<n>` · `src/lib.rs:385`
#    Windows 기본 소켓 `\\.\pipe\cys`), 신원 규칙은 순수 함수다(javis_role.py:241).
#   귀결: 캐시도 `.fail` 백오프도 사라져 훅/도구 프로세스마다 데몬 왕복 1회 —
#   데몬 무응답이면 매 호출 2s 정지다(§7 ④ 방향 · P6 이전에는 왕복 자체가 없었다).
WIN_ENDPOINTS = (r"\\.\pipe\cys", r"\\.\pipe\cys-dept-one")


def py_sock_id(**env):
    keys = ("CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET", "HOME", "XDG_STATE_HOME")
    saved = {k: os.environ.get(k) for k in keys}
    try:
        for k in keys:
            os.environ.pop(k, None)
        os.environ.update(env)
        return JR._sock_id()
    finally:
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def sh_sock_id(**env):
    script = ('. "$1"; cys_role_sock_id_init; printf "%s" "$CYS_ROLE_SOCK_ID"')
    e = {"PATH": "/usr/bin:/bin", "LC_ALL": "C"}
    e.update(env)
    p = subprocess.run(["/bin/sh", "-c", script, "sockid", LIB],
                       capture_output=True, text=True, env=e, timeout=20)
    return p.stdout


def t5():
    for ep in WIN_ENDPOINTS:
        check("T5a 파이썬: Windows named pipe 종단점은 신원이 있다 %s" % ep,
              py_sock_id(CYS_SOCKET=ep) != "", "sock_id=%r" % py_sock_id(CYS_SOCKET=ep))
        check("T5b 셸: Windows named pipe 종단점은 신원이 있다 %s" % ep,
              sh_sock_id(CYS_SOCKET=ep) != "", "sock_id=%r" % sh_sock_id(CYS_SOCKET=ep))
    win_home = "C:\\Users\\x"
    check("T5c 파이썬: 드라이브 지정 HOME 의 기본 종단점도 신원이 있다",
          py_sock_id(HOME=win_home, XDG_STATE_HOME=win_home + "\\AppData") != "")
    # 양성 대조(녹색 유지): unix 종단점은 종전대로 신원이 있다.
    check("T5d 양성대조 unix 절대 경로 종단점", py_sock_id(CYS_SOCKET="/tmp/d.sock") != "")


def main():
    try:
        t1()
        t2()
        t3()
        t3_dept()
        t4()
        t5()
    finally:
        shutil.rmtree(_root, ignore_errors=True)
    if fails:
        print("FAILED: %s" % ", ".join(fails))
        return 1
    print("ROLE-AUTHORITY-TRIAGE-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
