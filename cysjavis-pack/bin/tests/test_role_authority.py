#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_role_authority.py — 역할 해소 데몬 권위 전환 회귀 핀 (0.14.31 P6 · 감사 codex E).

무엇을 막는가: 좌석 승계(claim-role·takeover)는 데몬 roles 맵만 바꾸고 pane 의 `CYS_ROLE` env 는
**낡은 채로 남긴다**. 팩의 파이썬 결정 지점들이 그 env 를 직접 읽으면 옛 신원으로 판정한다
(정본 IMPLEMENTATION-PLAN.md §8: "`CYS_ROLE` env 를 권위로 쓰지 않는다 — 데몬 조회 우선").
0.14.31 부터 판정은 `bin/javis_role.py` 한 곳이 하고, 소비처는 **단조 거부**(monotone deny)로
합성한다 — 종전이 막던 것은 그대로 막고, **데몬이 아니라고 말하는 것만 더 막는다**.

★왜 단조 거부인가(적대 검토 반영 · 이 파일 §5 가 그 반례를 든다): `javis_org.destroy_dept` 가
  허용한 뒤 하위 `cys-dept down` 이 거부하면, destroy 는 down 실패에도 pack/workdir 격리를
  best-effort 로 **계속 진행한다**(javis_org.py:511-524) — 살아 있는 부서의 팩·작업 폴더가
  이동되는 반파괴다. '데몬 답으로 갈아끼우기'는 새 허용을 만들어 그 조합을 늘린다. 그래서
  어느 층도 종전보다 더 허용하지 않는다.

핀 목록
  ① 해소기 3상 — 데몬 답 / 판정 불가(rc≠0)→env / 둘 다 없음→빈 역할
  ② ★ⓑ(권위 있는 무역할 rc0+빈줄)과 ⓒ(판정 불가 rc≠0)는 **다른 사실**이다 — 뭉개면 데몬 사망이
     '무역할'로 읽힌다
  ③ ★주소 부재(surface id 없음)는 무역할 판정이 아니다 — 조회 자체를 하지 않는다(정상 위임 경로
     `CYS_ROLE=cso python3 javis_org.py apply …` 와 기존 하네스 다수가 그 형상이다)
  ④ 캐시 60s·실패 백오프 30s·프로세스 메모 — 훅 1런에 왕복 최대 1회(부트체인 ④ 회피)
  ⑤ ★셸 짝(`hooks/_lib.sh cys_resolve_role`)과 **같은 전용 디렉터리·같은 레코드 문법**을 읽고 쓴다
  ⑥ 캐시 위생 — 미래 시각·손상 형식·심링크는 신뢰하지 않는다
  ⑦ 신원 키 순서 CYS_→JAVIS_→AITERM_(Rust `env_compat` 미러) · 자릿수 상한 19
  ⑧ 소비처 단조 거부 — javis_org.require_cso · javis_snapshot.is_master · completion_guard._role
  ⑨ ★음성 대조: 부모 프로세스 env 는 조회로 오염되지 않는다(CYS_NO_AUTOSTART 누출 0)
  ⑩ ★R1(blocking): **부모 허용 + 자식 거부 = 격리 0** — `cys-dept down` 이 거부 rc(7·2)로 끝나면
     `javis_org.destroy_dept` 는 pack/workdir 을 **한 개도 옮기지 않는다**(살아 있는 부서의
     반파괴 봉인). 양성 대조(rc 0 → 실제로 옮긴다)를 같이 둬서 공허한 단언이 되지 않게 한다.
  ⑪ ★R1: 신원 문법이 Rust `parse_surface_ref` 의 **부분집합**이다 — 여러 줄·공백만·`+n` 은
     조회 없이 거절(유효 surface 아래 '권위 무역할'을 캐시해 정상 CSO 를 거짓 거부하던 길 차단)
  ⑫ ★R1: 캐시 기질 — 0700 전용 디렉터리 · FIFO 무매달림 · 신뢰 못 할 디렉터리는 캐시 끔
  ⑭ ★R1: 슬러그 충돌은 캐시 미스로 강등 · 프로세스 메모는 신원으로 키가 잡힌다
  ⑬ ★R1: **같은 바이트 → 같은 판정** — 레코드 문법 10종을 두 층에 동시에 먹여 차분 대조한다
     (종전엔 `"<ts> cso "` 가 셸에선 `cso `, 파이썬에선 `cso` 였다 · codex 위임 산출 · 전량 리뷰)
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 ROLE-AUTHORITY-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_role_authority.py
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
sys.path.insert(0, BIN)

import javis_role as JR  # noqa: E402  (해소 규칙 SOT)

LIB = os.path.normpath(os.path.join(BIN, "..", "hooks", "_lib.sh"))
fails = []
_tmproot = tempfile.mkdtemp(prefix="role-authority-")
_n = [0]


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def stub_dir(rc=0, out="", log=None):
    """`cys` 스텁 1개만 있는 디렉터리. 자식이 받은 CYS_NO_AUTOSTART 를 log 에 append 한다."""
    _n[0] += 1
    d = os.path.join(_tmproot, "stub%d" % _n[0])
    os.makedirs(d, exist_ok=True)
    p = os.path.join(d, "cys")
    body = ["#!/bin/sh"]
    if log:
        body.append('printf "%%s\\n" "${CYS_NO_AUTOSTART:-<unset>}" >> "%s"' % log)
    body.append('[ "$1" = "surface-role" ] || exit 1')
    body.append("printf '%%s' '%s'" % out.replace("'", "'\\''"))
    body.append("exit %d" % rc)
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(body) + "\n")
    os.chmod(p, 0o755)
    return p


def base_env(**over):
    """ambient 신원·캐시를 전부 걷어낸 밀폐 env(케이스마다 새 TMPDIR)."""
    env = {k: v for k, v in os.environ.items()
           if k not in ("CYS_ROLE", "CYS_SURFACE_ROLE", "CYS_SURFACE_ID", "JAVIS_SURFACE_ID",
                        "AITERM_SURFACE_ID", "CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET",
                        "CYS_BIN", "CYS_NO_AUTOSTART", "TMPDIR")}
    env["TMPDIR"] = tempfile.mkdtemp(dir=_tmproot)
    env.update({k: v for k, v in over.items() if v is not None})
    return env


def resolve(env):
    """자식 프로세스에서 해소 1회 — "role\\tsource" 를 돌려준다(메모 오염 0)."""
    r = subprocess.run([sys.executable, os.path.join(BIN, "javis_role.py")],
                       capture_output=True, text=True, encoding="utf-8",
                       timeout=60, env=env, cwd=BIN)
    return (r.stdout or "").strip("\n")


def sh_resolve(env, interp="sh"):
    """셸 짝으로 해소 1회 — "role\\tsource"."""
    script = ('. "%s" >/dev/null 2>&1; cys_resolve_role; '
              'printf "%%s\\t%%s\\n" "$CYS_RESOLVED_ROLE" "$CYS_RESOLVED_ROLE_SOURCE"') % LIB
    r = subprocess.run([interp, "-c", script], capture_output=True, text=True,
                       encoding="utf-8", timeout=60, env=env)
    return (r.stdout or "").strip("\n")


def sh_sock_id(env, interp="sh"):
    """셸 짝이 계산한 종단점 신원 — 파이썬 `_sock_id()` 와 글자 그대로 같아야 한다."""
    script = ('. "%s" >/dev/null 2>&1; cys_role_sock_id_init; '
              'printf "%%s" "$CYS_ROLE_SOCK_ID"') % LIB
    r = subprocess.run([interp, "-c", script], capture_output=True, text=True,
                       encoding="utf-8", timeout=60, env=env)
    return r.stdout or ""


def _identity(env, sid):
    """주어진 env 에서 (캐시 경로, sockid, epoch) — **해소기 자신의 규칙**으로 계산한다.

    검체가 규칙을 다시 구현하면 규칙이 바뀔 때 검체가 조용히 빗나간다(그래서 재구현하지 않는다).
    """
    old_env = dict(os.environ)
    try:
        os.environ.clear()
        os.environ.update(env)
        JR.reset_cache()
        return JR._cache_path(sid), JR._sock_id(), JR._boot_epoch()
    finally:
        os.environ.clear()
        os.environ.update(old_env)
        JR.reset_cache()


def JR_cache_path_of(env, sid):
    """해소기 규칙으로 계산한 캐시 파일 경로(디렉터리 생성 포함)."""
    return _identity(env, sid)[0]


def _seed_record(env, sid, role, ts=None, epoch=None, sockid=None, mode="file", path=None):
    """**문법적으로 유효한 4필드 레코드**를 캐시 자리에 심는다.

    ★R2(major · reviewer-codex): 종전 6a/6c 는 두 필드짜리 옛 레코드를 심어서 **문법 거절**이
      먼저 걸렸다 — 미래 시각·심링크 방어를 통째로 지워도 검체가 녹색이었다(공허한 단언).
      여기서는 '그 한 가지 결함만 있는' 레코드를 심고, 같은 레코드의 정상판이 실제로 캐시
      히트가 되는 **양성 대조**를 함께 둔다.
    """
    cpath, sk, ep = _identity(env, sid)
    if path is None:
        path = cpath
    line = "%d %s %s %s\n" % (int(time.time()) if ts is None else ts, role,
                              ep if epoch is None else epoch,
                              sk if sockid is None else sockid)
    d = os.path.dirname(path)
    if d and not os.path.isdir(d):
        os.makedirs(d, mode=0o700, exist_ok=True)
    if mode == "file":
        with open(path, "w", encoding="utf-8") as f:
            f.write(line)
        os.chmod(path, 0o600)
    else:
        target = path + ".target"
        with open(target, "w", encoding="utf-8") as f:
            f.write(line)
        os.symlink(target, path)
    return path, line


# ── ① 3상 + ② ⓑ/ⓒ 구분 + ③ 주소 부재 ────────────────────────────────────────
def test_three_states():
    got = resolve(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n")))
    check("1a 데몬 답이 stale env 를 이긴다", got == "cso\tdaemon", got)

    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(2, ""))
    check("1b 판정 불가(rc=2) → env 폴백(= 현행 동작)", resolve(e) == "master\tenv-cys-role")

    e = base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(2, ""))
    check("1c 둘 다 없음 → 빈 역할", resolve(e) == "\tnone")

    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="cso", CYS_BIN=stub_dir(0, "\n"))
    check("2a ★권위 있는 무역할(rc0+빈줄)이 stale env 를 덮는다", resolve(e) == "\tdaemon-none")

    e = base_env(CYS_ROLE="cso", CYS_BIN=stub_dir(0, "\n"))
    check("3a ★주소 부재는 무역할 판정이 아니다 — 조회 없이 env",
          resolve(e) == "cso\tenv-cys-role")

    for i, (label, ov) in enumerate((("빈 surface", {}), ("비숫자 surface", {"CYS_SURFACE_ID": "abc"}),
                                     ("20자리(u64 초과)", {"CYS_SURFACE_ID": "1" * 20}))):
        log = os.path.join(_tmproot, "noquery%d.log" % i)
        resolve(base_env(CYS_ROLE="cso", CYS_BIN=stub_dir(0, "cso\n", log=log), **ov))
        check("3b-%d ★%s → 데몬 조회 0회" % (i + 1, label), not os.path.exists(log),
              "조회 로그가 생겼다" if os.path.exists(log) else "")

    e = base_env(CYS_SURFACE_ID="surface:12", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n"))
    check("3c surface:<n> 접두 수용", resolve(e) == "cso\tdaemon")
    e = base_env(CYS_SURFACE_ID="1" * 19, CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n"))
    check("3d 19자리(u64 안전)는 조회한다", resolve(e) == "cso\tdaemon")


# ── ⑦ 신원 키 순서(Rust env_compat 미러) ─────────────────────────────────────
def test_env_compat_keys():
    e = base_env(JAVIS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n"))
    check("7a JAVIS_SURFACE_ID 단독으로도 조회", resolve(e) == "cso\tdaemon")
    e = base_env(AITERM_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n"))
    check("7b AITERM_SURFACE_ID 단독으로도 조회", resolve(e) == "cso\tdaemon")
    e = base_env(CYS_SURFACE_ID="7", AITERM_SURFACE_ID="bad", CYS_ROLE="master",
                 CYS_BIN=stub_dir(0, "cso\n"))
    check("7c CYS_ 가 구 키보다 우선", resolve(e) == "cso\tdaemon")


# ── ★C: 폴백은 CYS_ROLE 하나뿐(현행이 거부하던 것을 새로 허용하지 않는다) ─────
def test_fallback_is_cys_role_only():
    got = resolve(base_env(CYS_SURFACE_ID="7", CYS_SURFACE_ROLE="cso", CYS_ROLE="worker",
                           CYS_BIN=stub_dir(2, "")))
    check("C ★CYS_SURFACE_ROLE 은 폴백이 아니다(=cso 로 승격되지 않는다)",
          got == "worker\tenv-cys-role", got)


# ── ④ 캐시·백오프·메모 ───────────────────────────────────────────────────────
def test_cache_and_backoff():
    log = os.path.join(_tmproot, "cachehit.log")
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n", log=log))
    check("4a 첫 호출은 데몬", resolve(e) == "cso\tdaemon")
    e2 = dict(e); e2["CYS_BIN"] = stub_dir(2, "", log=log)   # 데몬 사망
    check("4b 신선 캐시가 판정 불가를 메운다", resolve(e2) == "cso\tcache")
    n = len(open(log, encoding="utf-8").read().strip().split("\n"))
    check("4c 캐시 히트는 자식을 다시 부르지 않는다", n == 1, "조회 %d회" % n)

    log2 = os.path.join(_tmproot, "backoff.log")
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(2, "", log=log2))
    resolve(e); resolve(e)
    n = len(open(log2, encoding="utf-8").read().strip().split("\n"))
    check("4d ★실패 백오프 — 두 번째 호출은 조회하지 않는다(데몬 사망 시 전 pane 정지 차단)",
          n == 1, "조회 %d회" % n)

    # 프로세스 메모: 한 프로세스에서 두 번 물어도 조회는 1회
    # ★R2(major · reviewer-codex): 종전 4e 는 **디스크 캐시로도 통과**했다(메모 만료 검사를
    #   지워도 녹색). 그래서 아래 memo_* 검체는 캐시 디렉터리를 신뢰 불가로 만들어
    #   **디스크 캐시를 끈 상태**에서 재고, 시각은 가짜 시계로 통제한다(sleep 0).
    log3 = os.path.join(_tmproot, "memo.log")
    e = base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(0, "cso\n", log=log3))
    code = ("import sys; sys.path.insert(0, %r); import javis_role as R;"
            "print(R.resolve_role(), R.resolve_role(), R.resolve_role())" % BIN)
    subprocess.run([sys.executable, "-c", code], capture_output=True, text=True,
                   timeout=60, env=e)
    n = len(open(log3, encoding="utf-8").read().strip().split("\n"))
    check("4e 프로세스 메모 — 3회 질의에 조회 1회", n == 1, "조회 %d회" % n)


def _no_disk_cache(env):
    """캐시 디렉터리 자리에 **정규 파일**을 놓아 디스크 캐시를 끈다(두 층 공통 규칙).

    메모만 남기므로 '메모가 실제로 왕복을 아끼는가'를 디스크 캐시의 도움 없이 잴 수 있다.
    """
    with open(os.path.join(env["TMPDIR"], "cys-role-authority.d"), "w") as f:
        f.write("not a directory\n")
    return env


_MEMO_DRIVER = """
import json, sys
sys.path.insert(0, %r)
import javis_role as R


class Clock(object):
    def __init__(self):
        self.w = 1000000.0
        self.m = 500.0

    def time(self):
        return self.w

    def monotonic(self):
        return self.m


C = Clock()
R.time = C
out = []
for step in json.loads(sys.argv[1]):
    C.w += step[0]
    C.m += step[1]
    out.append(list(R.resolve_role_detail()))
print(json.dumps(out))
"""


def _memo_run(env, steps):
    """가짜 시계로 (벽시계 증분, 단조시계 증분) 열을 따라 해소를 반복한다."""
    r = subprocess.run([sys.executable, "-c", _MEMO_DRIVER % BIN, json.dumps(steps)],
                       capture_output=True, text=True, encoding="utf-8", timeout=60,
                       env=env, cwd=BIN)
    try:
        return json.loads(r.stdout or "[]"), (r.stderr or "")
    except Exception:
        return [], (r.stdout or "") + (r.stderr or "")


def test_memo_lifetime():
    """★R2: 메모의 **수명**을 잰다 — 종전 검체는 키만 재고 수명은 재지 않아, 만료 항을 지워도
    (=R1 이전의 영구 메모로 되돌려도) 전건 녹색이었다(reviewer-claude 실증)."""
    log = os.path.join(_tmproot, "memo-ttl.log")
    e = _no_disk_cache(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                CYS_BIN=stub_dir(0, "cso\n", log=log)))
    got, err = _memo_run(e, [[0, 0], [30, 30], [29, 29], [2, 2]])
    n = len(open(log, encoding="utf-8").read().strip().split("\n")) if os.path.exists(log) else 0
    check("4f ★메모는 60초에 만료한다(+59s 재사용 · +61s 재해소)",
          [x[1] for x in got] == ["daemon", "daemon", "daemon", "daemon"] and n == 2,
          "src=%s 조회 %d회 %s" % ([x[1] for x in got], n, err[:120]))

    # 시계 역행: 단조시계는 1초만 흘렀는데 벽시계를 500초 되돌린다 → 메모를 버려야 한다.
    log = os.path.join(_tmproot, "memo-rollback.log")
    e = _no_disk_cache(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                CYS_BIN=stub_dir(0, "cso\n", log=log)))
    got, err = _memo_run(e, [[0, 0], [-500, 1]])
    n = len(open(log, encoding="utf-8").read().strip().split("\n")) if os.path.exists(log) else 0
    check("4g ★벽시계 역행은 메모를 무효화한다(옛 권위가 560초 더 사는 길 차단)",
          n == 2, "조회 %d회 %s" % (n, err[:120]))

    # 서스펜드: 벽시계만 1시간 흐르고 단조시계는 멈춘 플랫폼(macOS) → 메모를 버려야 한다.
    log = os.path.join(_tmproot, "memo-suspend.log")
    e = _no_disk_cache(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                CYS_BIN=stub_dir(0, "cso\n", log=log)))
    got, err = _memo_run(e, [[0, 0], [3600, 0]])
    n = len(open(log, encoding="utf-8").read().strip().split("\n")) if os.path.exists(log) else 0
    check("4h ★단조시계가 서스펜드를 세지 않아도 벽시계 만료가 메모를 끊는다",
          n == 2, "조회 %d회 %s" % (n, err[:120]))

    # 59초 된 디스크 레코드를 메모가 60초 더 살리지 않는다(승계 반영 상한 119s 회귀 차단).
    log = os.path.join(_tmproot, "memo-cachecap.log")
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "worker\n", log=log))
    _seed_record(e, "7", "cso", ts=1000000 - 59)
    got, err = _memo_run(e, [[0, 0], [1, 1]])
    n = len(open(log, encoding="utf-8").read().strip().split("\n")) if os.path.exists(log) else 0
    check("4i ★59초 된 캐시 히트의 메모는 1초짜리다(다 된 캐시를 60초 되살리지 않는다)",
          [x[1] for x in got] == ["cache", "daemon"] and n == 1,
          "src=%s 조회 %d회 %s" % ([x[1] for x in got], n, err[:120]))


# ── ⑥ 캐시 위생 ──────────────────────────────────────────────────────────────
def test_cache_hygiene():
    now = int(time.time())

    # ★양성 대조 먼저: **아무 결함도 없는** 4필드 레코드는 실제로 캐시 히트다. 이 줄이 없으면
    #   아래 세 음성 대조는 '문법이 거절해서' 통과하는 공허한 단언이 될 수 있다(R2 · codex).
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master")
    _seed_record(e, "7", "cso", ts=now)
    e["CYS_BIN"] = stub_dir(2, "")
    check("6-pos 양성 대조: 유효한 4필드 레코드는 캐시 히트다", resolve(e) == "cso\tcache")

    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master")
    _seed_record(e, "7", "cso", ts=now + 9999)          # 유일한 결함 = 미래 시각
    e["CYS_BIN"] = stub_dir(2, "")
    check("6a 미래 시각 캐시는 신선이 아니다(그 한 가지만 어긋난 유효 레코드)",
          resolve(e) == "master\tenv-cys-role")

    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master")
    _identity(e, "7")                                    # 캐시 디렉터리 생성
    with open(JR_cache_path_of(e, "7"), "w", encoding="utf-8") as f:
        f.write("noSpaceLine\n")
    e["CYS_BIN"] = stub_dir(2, "")
    check("6b 손상 형식(공백 없음) 캐시는 무시", resolve(e) == "master\tenv-cys-role")

    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master")
    _seed_record(e, "7", "cso", ts=now, mode="symlink")  # 유일한 결함 = 심링크
    e["CYS_BIN"] = stub_dir(2, "")
    check("6c ★심링크 캐시는 판독하지 않는다(내용은 유효한 레코드다)",
          resolve(e) == "master\tenv-cys-role")

    # 소유자가 다른 캐시 파일은 신뢰하지 않는다 — root 아니면 만들 수 없으므로 건너뛴다.
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master")
    _seed_record(e, "7", "cso", ts=now, sockid="/other/daemon.sock")
    e["CYS_BIN"] = stub_dir(2, "")
    check("6d 다른 데몬 신원의 레코드는 이 좌석의 권위가 아니다",
          resolve(e) == "master\tenv-cys-role")


# ── ⑤ 셸 짝과의 캐시 파리티 ──────────────────────────────────────────────────
def test_cross_layer_cache():
    e = base_env(CYS_SURFACE_ID="12", CYS_ROLE="master", CYS_BIN=stub_dir(0, "reviewer-codex\n"))
    got = sh_resolve(e)
    check("5a 셸이 데몬 답을 쓴다", got == "reviewer-codex\tdaemon", got)
    e2 = dict(e); e2["CYS_BIN"] = stub_dir(2, "")
    got = resolve(e2)
    check("5b ★파이썬이 셸의 캐시를 읽는다", got == "reviewer-codex\tcache", got)

    e = base_env(CYS_SURFACE_ID="12", CYS_ROLE="master", CYS_BIN=stub_dir(0, "planner\n"))
    resolve(e)
    e2 = dict(e); e2["CYS_BIN"] = stub_dir(2, "")
    got = sh_resolve(e2)
    check("5c ★셸이 파이썬의 캐시를 읽는다", got == "planner\tcache", got)
    got = sh_resolve(e2, "bash")
    check("5d bash 인터프리터에서도 동형", got == "planner\tcache", got)


# ── ⑨ 음성 대조: 부모 env 무오염 + 자식에 CYS_NO_AUTOSTART ───────────────────
def test_no_autostart_and_env_purity():
    log = os.path.join(_tmproot, "noauto.log")
    e = base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(0, "cso\n", log=log))
    code = ("import sys, os; sys.path.insert(0, %r); import javis_role as R; R.resolve_role();"
            "print('LEAK' if 'CYS_NO_AUTOSTART' in os.environ else 'CLEAN')" % BIN)
    r = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True,
                       timeout=60, env=e)
    check("9a ★자식 조회에 CYS_NO_AUTOSTART=1 (역할 조회가 데몬을 낳지 않는다)",
          open(log, encoding="utf-8").read().strip() == "1",
          repr(open(log, encoding="utf-8").read()))
    check("9b ★부모 프로세스 env 는 오염되지 않는다", "CLEAN" in r.stdout, r.stdout.strip())


# ── ⑧ 소비처 단조 거부 ───────────────────────────────────────────────────────
def _org_rc(env, extra_args=None):
    """javis_org.py apply 를 부작용 없는 형태로 — 게이트에서 exit 3 이면 3, 아니면 그 밖."""
    env = dict(env)
    env.setdefault("CYS_DEPT_CATALOG", os.path.join(_tmproot, "no-catalog.json"))
    env.setdefault("CYS_DEPTS_JSON", os.path.join(_tmproot, "no-depts.json"))
    r = subprocess.run([sys.executable, os.path.join(BIN, "javis_org.py"), "apply",
                        os.path.join(_tmproot, "no-such-manifest.json")],
                       capture_output=True, text=True, encoding="utf-8", timeout=60, env=env)
    return r.returncode, (r.stderr or "")


GATE_MSG = "★CSO 전용"


def test_org_gate():
    rc, e = _org_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="cso", CYS_BIN=stub_dir(0, "worker\n")))
    check("8a ★org: env=cso 인데 데몬=worker → 거부(승계 후 stale 로 부서 mutation 하던 길)",
          rc == 3 and GATE_MSG in e, "rc=%s" % rc)
    rc, e = _org_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="cso", CYS_BIN=stub_dir(0, "\n")))
    check("8b org: env=cso 인데 데몬이 '역할 없음' → 거부", rc == 3 and GATE_MSG in e, "rc=%s" % rc)
    # ★R1(reviewer-codex): '통과'를 `rc != 3` 으로만 재면 **무관한 크래시도 통과로 읽힌다**.
    #   없는 매니페스트의 정확한 rc(=2 · v_schema 이전 파일 판독 실패)와 게이트 문면 부재를 함께 잰다.
    rc, e = _org_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="cso", CYS_BIN=stub_dir(2, "")))
    check("8c ★org: 판정 불가 → 종전 그대로 통과(게이트 무발화 · rc=2 는 매니페스트 부재)",
          rc == 2 and GATE_MSG not in e, "rc=%s err=%r" % (rc, e.strip()[:120]))
    # ★I5 재핀(판정관 T3a·T3b · 2026-09-08 · **의도적 계약 변경**): 종전 8d 는 "데몬=cso 인데
    #   env=worker → 여전히 거부" 를 핀했다 — 그것이 정본 §8("`CYS_ROLE` env 를 권위로 쓰지
    #   않는다 — 승계 후 stale")의 미달 지점 그 자체다. 승계로 정당하게 CSO 가 된 좌석은 자기
    #   env 를 고칠 수 없어(SessionStart 는 부모 env 를 못 고친다) 부서 mutation 을 **영구히**
    #   못 썼다. 이제 **살아 있는 데몬의 직접 응답**이 stale env 를 이긴다.
    e_live = base_env(CYS_SURFACE_ID="7", CYS_ROLE="worker", CYS_BIN=stub_dir(0, "cso\n"))
    rc, e = _org_rc(e_live)
    check("8d ★org: 데몬 직접 응답 cso 는 stale env=worker 를 이긴다(게이트 무발화 · rc=2)",
          rc == 2 and GATE_MSG not in e, "rc=%s err=%r" % (rc, e.strip()[:120]))
    # ★같은 env 로 한 번 더 — 첫 호출이 남긴 **신선 캐시** 때문에 판정이 뒤집히면(요동) 안 된다.
    rc, e = _org_rc(e_live)
    check("8d-2 ★두 번째 호출도 같은 판정이다(신선 캐시가 통과를 되돌리지 않는다)",
          rc == 2 and GATE_MSG not in e, "rc=%s err=%r" % (rc, e.strip()[:120]))
    # ★음성 대조(새 허용의 근거를 좁힌 증거 · codex 설계 비평 (g)): **캐시만으로는 통과하지
    #   못한다**. 같은 uid 의 아무 프로세스나 쓸 수 있는 파일 한 줄이 부서 lifecycle mutation 을
    #   열면 §3-3(막는 쪽으로만 틀린다)이 거짓이 된다 — 통과 근거는 `daemon` 직접 응답뿐이다.
    e_forge = base_env(CYS_SURFACE_ID="7", CYS_ROLE="worker", CYS_BIN=stub_dir(2, ""))
    _seed_record(e_forge, "7", "cso")
    rc, e = _org_rc(e_forge)
    check("8d-3 ★위조된 신선 캐시 `cso` 만으로는 통과하지 못한다(데몬 사망 · env=worker)",
          rc == 3 and GATE_MSG in e, "rc=%s err=%r" % (rc, e.strip()[:120]))
    rc, e = _org_rc(base_env(CYS_ROLE="cso", CYS_BIN=stub_dir(0, "worker\n")))
    check("8e org: 주소 없음 → 종전 env 판정만(게이트 무발화 · rc=2)",
          rc == 2 and GATE_MSG not in e, "rc=%s err=%r" % (rc, e.strip()[:120]))


def _snapshot_rc(env, state):
    env = dict(env); env["CYS_STATE_DIR"] = state
    r = subprocess.run([sys.executable, os.path.join(BIN, "javis_snapshot.py"), "is-master"],
                       capture_output=True, text=True, encoding="utf-8", timeout=60,
                       env=env, cwd=BIN)
    return r.returncode, (r.stdout or "")


def test_snapshot_gate():
    import json
    state = os.path.join(_tmproot, "snapstate")
    os.makedirs(state, exist_ok=True)
    with open(os.path.join(state, "mission.json"), "w", encoding="utf-8") as f:
        json.dump({"schema": 1, "mission": None, "surface": "7"}, f)

    # ★R1(reviewer-codex): 종전 8f 는 env-master 와 대장 일치를 **한 픽스처에 섞어** 놓아서
    #   데몬 절이 env 절 뒤로 밀려도 통과했다. 두 신호를 갈라 각각 잰다.
    nostate = os.path.join(_tmproot, "snapstate-empty")
    os.makedirs(nostate, exist_ok=True)          # 대장 없음 = 대장 절 단독으로는 불통과
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                    CYS_BIN=stub_dir(0, "worker\n")), nostate)
    check("8f-1 ★snapshot: env=master 단독 신호를 데몬=worker 가 끊는다",
          rc == 1 and "daemon role is not master" in out, "rc=%s out=%r" % (rc, out.strip()))
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(0, "worker\n")), state)
    check("8f-2 ★snapshot: 대장 일치 단독 신호도 데몬=worker 가 끊는다(env 절 없이)",
          rc == 1 and "daemon role is not master" in out, "rc=%s out=%r" % (rc, out.strip()))
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                    CYS_BIN=stub_dir(0, "worker\n")), state)
    check("8f-3 snapshot: 두 신호가 함께여도 데몬 절이 앞선다",
          rc == 1 and "not-master" in out, "rc=%s out=%r" % (rc, out.strip()))
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(2, "")), nostate)
    check("8f-4 음성 대조: 신호가 하나도 없으면 데몬과 무관하게 불통과",
          rc == 1 and "no surface id" not in out, "rc=%s out=%r" % (rc, out.strip()))
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                    CYS_BIN=stub_dir(0, "master\n")), state)
    check("8g snapshot: 데몬=master → 통과", rc == 0, "rc=%s out=%r" % (rc, out.strip()))
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                    CYS_BIN=stub_dir(2, "")), state)
    check("8h snapshot: 판정 불가 → 종전 env 절 그대로 통과", rc == 0, "rc=%s" % rc)
    # ★R2 재핀(blocking · reviewer-codex · 의도적 기본값 변경): 종전 8i 는 **권위 있는 무역할**
    #   앞에서 대장 절이 master 를 되살리는 것을 '종전 거부 없음'이라며 핀했다. 그것이 정본 §8
    #   ("`CYS_ROLE` env 를 권위로 쓰지 않는다 — 승계 후 stale")의 표적 그 자체다: 데몬이
    #   **확정적으로** '이 좌석에 역할이 없다'고 답한 것은 판정 불가가 아니라 사실이다.
    #   실패 방향은 여전히 생산 skip(exit 0)이지 좌석 사망이 아니다(§3-3).
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                    CYS_BIN=stub_dir(0, "\n")), nostate)
    check("8i-1 ★권위 무역할은 stale env master 를 끊는다(대장 없음)",
          rc == 1 and "daemon knows no role" in out, "rc=%s out=%r" % (rc, out.strip()))
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(0, "\n")), state)
    check("8i-2 ★권위 무역할은 stale 대장 일치도 끊는다(env 없음)",
          rc == 1 and "daemon knows no role" in out, "rc=%s out=%r" % (rc, out.strip()))
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                    CYS_BIN=stub_dir(0, "\n")), state)
    check("8i-3 ★두 신호가 함께여도 권위 무역할이 앞선다",
          rc == 1 and "daemon knows no role" in out, "rc=%s out=%r" % (rc, out.strip()))
    # 양성 대조 — 이 거부가 '권위 있는 답'에만 걸린다는 증거(판정 불가는 종전 그대로 통과).
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(2, "")), state)
    check("8i-4 판정 불가는 여전히 대장 절을 살린다(새 거부를 만들지 않는다)",
          rc == 0, "rc=%s out=%r" % (rc, out.strip()))


def test_guard_label():
    code = ("import sys; sys.path.insert(0, %r); import javis_completion_guard as G;"
            "print(G._role())" % BIN)

    def label(env):
        r = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True,
                           timeout=60, env=env, cwd=BIN)
        return (r.stdout or "").strip()

    check("8j guard: 데몬 답이 라벨이 된다",
          label(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                         CYS_BIN=stub_dir(0, "cso\n"))) == "cso")
    check("8k ★guard: 권위 무역할이면 stale env 가 아니라 주소로 귀속(라벨은 비지 않는다)",
          label(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                         CYS_BIN=stub_dir(0, "\n"))) == "surface:7")
    check("8l guard: 판정 불가 → 종전 env 라벨",
          label(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                         CYS_BIN=stub_dir(2, ""))) == "master")
    # ★R1(reviewer-codex): 권위 무역할의 주소 라벨은 **해소기의 정규 신원**으로 만든다 —
    #   이 파일의 지역 파서는 JAVIS_ 키를 모르고 구두점을 지워 `surface:unknown`·
    #   `surface:surface12` 같은 틀린 라벨을 냈다.
    got = label(base_env(JAVIS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "\n")))
    check("8m ★guard: JAVIS_SURFACE_ID 로도 주소 라벨이 선다", got == "surface:7", got)
    got = label(base_env(CYS_SURFACE_ID="surface:12", CYS_ROLE="master",
                         CYS_BIN=stub_dir(0, "\n")))
    check("8n ★guard: `surface:<n>` 접두도 정규 신원으로 읽는다", got == "surface:12", got)
    # ★R1: 권위가 아닌 출처(env 폴백)에서는 **종전 표현식 그대로** — 공백조차 다듬지 않는다.
    got = label(base_env(CYS_SURFACE_ID="7", CYS_ROLE=" cso ", CYS_BIN=stub_dir(2, "")))
    check("8o guard: 판정 불가면 env 라벨(출력 strip 비교 · 원문 보존은 8p 가 잰다)",
          got == "cso", got)
    code2 = ("import sys; sys.path.insert(0, %r); import javis_completion_guard as G;"
             "print(repr(G._role()))" % BIN)
    r = subprocess.run([sys.executable, "-c", code2], capture_output=True, text=True, timeout=60,
                       env=base_env(CYS_SURFACE_ID="7", CYS_ROLE=" cso ", CYS_BIN=stub_dir(2, "")),
                       cwd=BIN)
    check("8p ★guard: env 폴백 라벨은 raw 그대로(' cso ')",
          (r.stdout or "").strip() == "' cso '", (r.stdout or "").strip())
    # ★0.14.31 성찰 G15: 같은 좌석은 **두 갈래(권위 무역할 / 판정 불가)에서 같은 주소 라벨**을 낸다.
    #   종전 폴백 갈래는 지역 파서라 `surface:surface12`·`surface:007` 을 냈다(원장에 두 행위자).
    for _sid, _want in (("surface:12", "surface:12"), ("007", "surface:7")):
        _a = label(base_env(CYS_SURFACE_ID=_sid, CYS_BIN=stub_dir(0, "\n")))     # 권위 무역할
        _b = label(base_env(CYS_SURFACE_ID=_sid, CYS_BIN=stub_dir(2, "")))        # 판정 불가·env 없음
        check("8q ★guard(G15): %r → 권위 갈래 라벨 %s" % (_sid, _want), _a == _want, _a)
        check("8r ★guard(G15): %r → 폴백 갈래도 같은 라벨 %s" % (_sid, _want), _b == _want, _b)



# ── ⑩ ★반파괴 봉인: 부모 허용 + 자식 거부 → 격리 0 ───────────────────────────
def _destroy_rig(name, down_rc):
    """가짜 HOME 에 팩·작업 폴더·레지스트리를 세우고 `cys-dept` 스텁을 꽂는다(라이브 무접촉).

    ★`javis_org` 는 HOME 에서 `~/.cys/pack-dept-<n>` · `~/.local/state/cys-trash` 를 **모듈
      로드 시** 계산한다 → 자식 프로세스의 HOME 만 바꾸면 전부 가짜 트리로 들어온다.
    """
    _n[0] += 1
    root = os.path.join(_tmproot, "destroy%d" % _n[0])
    home = os.path.join(root, "home")
    pack = os.path.join(home, ".cys", "pack-dept-%s" % name)
    work = os.path.join(home, "work-%s" % name)
    for d in (pack, work, os.path.join(home, ".local", "state")):
        os.makedirs(d, exist_ok=True)
    open(os.path.join(pack, "marker"), "w").write("pack\n")
    open(os.path.join(work, "marker"), "w").write("work\n")
    depts = os.path.join(root, "depts.json")
    with open(depts, "w", encoding="utf-8") as f:
        json.dump({"depts": {name: {"cwd": work, "workdir_owned": True,
                                    "socket": "/dev/null", "mission_key": None}}}, f)
    dept_bin = os.path.join(root, "cys-dept-stub")
    with open(dept_bin, "w", encoding="utf-8", newline="\n") as f:
        f.write("#!/bin/sh\necho '[stub] refused' >&2\nexit %d\n" % down_rc)
    os.chmod(dept_bin, 0o755)
    env = base_env(CYS_SURFACE_ID="7", CYS_ROLE="cso", CYS_BIN=stub_dir(0, "cso\n"))
    env["HOME"] = home
    env["CYS_DEPTS_JSON"] = depts
    env["CYS_DEPT_CATALOG"] = os.path.join(root, "catalog.json")
    env["CYS_DEPT_BIN"] = dept_bin
    r = subprocess.run([sys.executable, os.path.join(BIN, "javis_org.py"), "destroy",
                        "--dept", name, "--purge", "--purge-workdir"],
                       capture_output=True, text=True, encoding="utf-8", timeout=90,
                       env=env, cwd=BIN)
    trash = os.path.join(home, ".local", "state", "cys-trash")
    moved = sorted(os.listdir(trash)) if os.path.isdir(trash) else []
    try:
        acts = json.loads(r.stdout or "{}").get("targets", {}).get(name, [])
    except Exception:
        acts = []
    return {"rc": r.returncode, "err": r.stderr or "", "actions": [a[0] for a in acts],
            "pack_alive": os.path.isdir(pack), "work_alive": os.path.isdir(work),
            "trash": moved}


def test_destroy_half_op():
    for rc_in, label in ((7, "단일소유 거부"), (2, "인자 검증 거부")):
        g = _destroy_rig("halfop%d" % rc_in, rc_in)
        ok = (g["pack_alive"] and g["work_alive"] and not g["trash"]
              and "quarantine_pack" not in g["actions"]
              and "quarantine_workdir" not in g["actions"]
              and "down" in g["actions"] and g["rc"] != 0)
        check("10a-%d ★자식 down 이 %s(rc=%d) → pack/workdir 이동 0"
              % (rc_in, label, rc_in), ok,
              "actions=%s pack=%s work=%s trash=%s rc=%s"
              % (g["actions"], g["pack_alive"], g["work_alive"], g["trash"], g["rc"]))
        check("10b-%d 거부 사유가 정직하게 보고된다" % rc_in,
              "거부" in g["err"] and "살아 있다" in g["err"], g["err"].strip()[:200])
    # ★양성 대조 — 이 단언이 공허하지 않다는 증거(rc 0 이면 실제로 옮긴다)
    g = _destroy_rig("halfop0", 0)
    check("10c 양성 대조: down rc=0 이면 pack/workdir 은 실제로 격리된다",
          (not g["pack_alive"]) and (not g["work_alive"])
          and "quarantine_pack" in g["actions"] and "quarantine_workdir" in g["actions"],
          "actions=%s pack=%s work=%s trash=%s" % (g["actions"], g["pack_alive"],
                                                   g["work_alive"], g["trash"]))
    # ★rc 3(teardown 완료·state 격리만 실패)은 종전 best-effort 계약 그대로(핀 불변 확인)
    g = _destroy_rig("halfop3", 3)
    check("10d 음성 대조: rc=3(teardown 완료)은 종전대로 best-effort 격리를 계속한다",
          "quarantine_pack" in g["actions"] and "quarantine_workdir" in g["actions"],
          "actions=%s" % g["actions"])


# ── ⑪ 신원 문법이 Rust 의 부분집합인가(조회 0 · 캐시 오염 0) ─────────────────
def test_identity_grammar():
    cases = (("11a 여러 줄 신원", {"CYS_SURFACE_ID": "7\njunk"}),
             ("11b 공백만 있는 1순위 키는 구 키로 넘어가지 않는다",
              {"CYS_SURFACE_ID": " ", "JAVIS_SURFACE_ID": "7"}),
             ("11c 선두 + (Rust 는 받지만 우리는 거절)", {"CYS_SURFACE_ID": "+7"}),
             ("11d 접두 뒤 공백", {"CYS_SURFACE_ID": "surface: 7"}),
             ("11e 65자 초과 원값", {"CYS_SURFACE_ID": "7" + " " * 70}))
    for i, (label, ov) in enumerate(cases):
        log = os.path.join(_tmproot, "ident%d.log" % i)
        got = resolve(base_env(CYS_ROLE="cso", CYS_BIN=stub_dir(0, "cso\n", log=log), **ov))
        check("%s → 조회 0회 · env 폴백" % label,
              got == "cso\tenv-cys-role" and not os.path.exists(log),
              "%s query=%s" % (got, os.path.exists(log)))
    # 정규화: 선두 0 은 같은 캐시 키로 접힌다(두 층 공통 규칙)
    e = base_env(CYS_SURFACE_ID="007", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n"))
    check("11f 선두 0 신원도 조회한다", resolve(e) == "cso\tdaemon")
    d = os.path.join(e["TMPDIR"], "cys-role-authority.d")
    names = sorted(os.listdir(d)) if os.path.isdir(d) else []
    check("11g ★선두 0 은 캐시 키에서 정규화된다(007 과 7 이 갈리지 않는다)",
          len(names) == 1 and names[0].startswith("role-7-"), str(names))


# ── ⑫ 캐시 기질: 전용 디렉터리 · FIFO · 신뢰 불가 디렉터리 ──────────────────
def test_cache_substrate():
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n"))
    resolve(e)
    d = os.path.join(e["TMPDIR"], "cys-role-authority.d")
    mode = os.stat(d).st_mode & 0o777 if os.path.isdir(d) else -1
    check("12a ★캐시는 0700 전용 디렉터리 안에 있다(tmp 루트에 흩뿌리지 않는다)",
          os.path.isdir(d) and (mode & 0o077) == 0, "mode=%o" % mode if mode >= 0 else "없음")

    # FIFO 를 캐시 자리에 심어도 매달리지 않는다(타임아웃이 증인) · 판정은 데몬 답
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n"))
    os.makedirs(os.path.join(e["TMPDIR"], "cys-role-authority.d"), mode=0o700, exist_ok=True)
    env2 = dict(os.environ); old = dict(os.environ)
    try:
        os.environ.clear(); os.environ.update(e); JR.reset_cache()
        cpath = JR._cache_path("7")
    finally:
        os.environ.clear(); os.environ.update(old); JR.reset_cache()
    try:
        os.mkfifo(cpath, 0o600)
    except Exception:
        cpath = ""
    if cpath:
        got = resolve(e)
        check("12b ★FIFO 캐시는 판독을 매달지 않는다(무시하고 데몬에 묻는다)",
              got == "cso\tdaemon", got)

    # 신뢰 못 할 캐시 디렉터리(누구나 쓰기) → 캐시를 끄고 매번 데몬에 묻는다(죽지 않는다)
    log = os.path.join(_tmproot, "openperm.log")
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n", log=log))
    bad = os.path.join(e["TMPDIR"], "cys-role-authority.d")
    os.makedirs(bad, exist_ok=True)
    os.chmod(bad, 0o777)
    got1 = resolve(e)
    got2 = resolve(e)
    n = len(open(log, encoding="utf-8").read().strip().split("\n")) if os.path.exists(log) else 0
    check("12c ★group/other 쓰기 가능한 디렉터리는 신뢰하지 않는다 — 캐시 끔·매번 조회",
          got1 == "cso\tdaemon" and got2 == "cso\tdaemon" and n == 2,
          "%s/%s 조회 %d회" % (got1, got2, n))

    # 셸 짝도 같은 판정을 한다(두 층 규칙 동일)
    check("12d 셸 짝도 열린 디렉터리에서 daemon 으로 답한다", sh_resolve(e) == "cso\tdaemon")


def test_record_grammar_parity():
    # ★데몬 판정 불가(rc=2) 고정 — env 와 다른 답은 같은 바이트의 캐시 히트뿐이다.
    cases = (("신선 캐시", b"%s cso - %s\n", "cso\tcache"),
             ("권위 있는 무역할", b"%s - - %s\n", "\tcache-none"),
             ("이중 공백으로 빈 epoch", b"%s cso  - %s\n", "master\tenv-cys-role"),
             ("소켓 신원 뒤 쓰레기", b"%s cso - %s extra\n", "master\tenv-cys-role"),
             ("시각 선두 0", b"0%s cso - %s\n", "master\tenv-cys-role"),
             ("시각 12자리 초과", b"%s0000 cso - %s\n", "master\tenv-cys-role"),
             ("역할 안 공백", b"%s c so - %s\n", "master\tenv-cys-role"),
             ("CRLF 줄 끝", b"%s cso - %s\r\n", "cso\tcache"),
             ("첫 줄만 판독", b"%s cso - %s\nSECOND LINE\n", "cso\tcache"),
             ("역할 64자 초과", b"%s " + b"r" * 65 + b" - %s\n", "master\tenv-cys-role"))
    disagreed = []
    for i, (label, line, expected) in enumerate(cases, 1):
        e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(2, ""))
        # ★경로·신원은 **해소기 자신의 규칙**으로 얻는다(검체가 슬러그를 재구현하면 규칙이 바뀔 때
        #   조용히 빗나간다 — R2 에서 `tr -cs` 로 접기를 넣자 지역 람다가 즉시 어긋났다).
        cpath, sockid, _ep = _identity(e, "7")
        with open(cpath, "wb") as f:
            f.write(line % (str(int(time.time())).encode(), sockid.encode()))
        got = resolve(e)
        sh_got = sh_resolve(e)
        check("13-%d ★레코드 문법 파리티: %s" % (i, label),
              got == expected and sh_got == expected,
              "python=%r sh=%r expected=%r" % (got, sh_got, expected))
        if got != expected or sh_got != expected:
            disagreed.append(str(i))
    if disagreed:
        print("NOTE ★레코드 기대값 불일치 행: %s" % ", ".join(disagreed))


# ── ⑭ 슬러그 충돌 · 메모 키(codex R1 "검체가 못 잡는 결함" 목록의 나머지 둘) ─────────
def test_slug_collision_and_memo_key():
    # ★파일명은 겹치지만(슬러그가 손실 치환) 레코드의 소켓 신원이 달라 **권위가 넘어가지 않는다**.
    root = os.path.join(_tmproot, "slug"); os.makedirs(os.path.join(root, "a"), exist_ok=True)
    sock_a = os.path.join(root, "a", "b.sock")      # …/a/b.sock
    sock_b = os.path.join(root, "a_b.sock")         # …/a_b.sock  → 같은 슬러그
    e1 = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_SOCKET=sock_a,
                  CYS_BIN=stub_dir(0, "cso\n"))
    check("14a 데몬 A 의 답이 캐시된다", resolve(e1) == "cso\tdaemon")
    e2 = dict(e1); e2["CYS_SOCKET"] = sock_b; e2["CYS_BIN"] = stub_dir(2, "")
    got = resolve(e2)
    d = os.path.join(e1["TMPDIR"], "cys-role-authority.d")
    names = sorted(n for n in os.listdir(d) if not n.endswith(".fail")) if os.path.isdir(d) else []
    # 파일명은 **하나뿐**(두 소켓이 같은 슬러그로 접힌다)인데도 B 는 A 의 역할을 못 받는다 —
    # 레코드에 실린 소켓 신원이 다르기 때문이다(그 강등이 이 핀의 전부다).
    check("14b ★슬러그가 겹쳐도 다른 데몬의 역할을 권위로 읽지 않는다(캐시 미스로 강등)",
          got == "master\tenv-cys-role" and len(names) == 1,
          "%s files=%s" % (got, names))

    # ★프로세스 메모는 **신원으로 키가 잡힌다** — 신원이 바뀌면 재해소한다(영구 메모 금지).
    sa = stub_dir(0, "cso\n"); sb = stub_dir(0, "worker\n")
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=sa)
    code = ("import os, sys; sys.path.insert(0, %r); import javis_role as R;"
            "a=R.resolve_role();"
            "os.environ['CYS_BIN']=%r; b=R.resolve_role();"           # 같은 신원 → 메모 재사용
            "os.environ['CYS_SURFACE_ID']='8'; c=R.resolve_role();"   # 신원 변경 → 재해소
            "print(a, b, c)" % (BIN, sb))
    r = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True,
                       timeout=60, env=e, cwd=BIN)
    check("14c ★메모는 신원으로 키가 잡힌다(같은 신원=재사용 · 다른 신원=재해소)",
          (r.stdout or "").strip() == "cso cso worker",
          "%r %s" % ((r.stdout or "").strip(), (r.stderr or "").strip()[:120]))


# ── ⑮ R2: 종단점 신원 — 자르지도 접지도 않는다(codex R2 major) ───────────────
def _cache_files(env):
    d = os.path.join(env["TMPDIR"], "cys-role-authority.d")
    return sorted(os.listdir(d)) if os.path.isdir(d) else []


def test_endpoint_identity():
    """소켓 신원이 **다른 종단점을 같은 이름으로 접던** 세 길을 각각 막았는지 잰다.

    실패 방향은 전부 '디스크 캐시 끔 = 매번 데몬 조회'다(오판이 아니라 왕복 1회).
    """
    # ⓐ 상대 경로 — cwd 마다 다른 소켓이다. 신원 미지 → 캐시 파일 0 · 매번 조회.
    log = os.path.join(_tmproot, "relsock.log")
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_SOCKET="cys.sock",
                 CYS_BIN=stub_dir(0, "cso\n", log=log))
    got1, got2 = resolve(e), resolve(e)
    n = len(open(log, encoding="utf-8").read().strip().split("\n")) if os.path.exists(log) else 0
    check("15a ★상대 소켓 경로는 신원이 아니다 — 디스크 캐시 끔(파일 0·매번 조회)",
          got1 == "cso\tdaemon" and got2 == "cso\tdaemon" and n == 2 and _cache_files(e) == [],
          "%s/%s 조회 %d회 files=%s" % (got1, got2, n, _cache_files(e)))

    # ⓑ 한 프로세스 안에서 cwd 만 바꿔도 **메모가 넘어가지 않는다**(codex R2: 메모 키에 문맥).
    log = os.path.join(_tmproot, "relsock-memo.log")
    d1 = os.path.join(_tmproot, "cwd1"); d2 = os.path.join(_tmproot, "cwd2")
    os.makedirs(d1, exist_ok=True); os.makedirs(d2, exist_ok=True)
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_SOCKET="cys.sock",
                 CYS_BIN=stub_dir(0, "cso\n", log=log))
    code = ("import os, sys; sys.path.insert(0, %r); import javis_role as R;"
            "os.chdir(%r); a=R.resolve_role();"
            "os.chdir(%r); b=R.resolve_role();"
            "os.chdir(%r); c=R.resolve_role();"
            "print(a, b, c)" % (BIN, d1, d2, d1))
    r = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True,
                       timeout=60, env=e, cwd=BIN)
    n = len(open(log, encoding="utf-8").read().strip().split("\n")) if os.path.exists(log) else 0
    check("15b ★상대 소켓에서 cwd 가 바뀌면 메모를 재사용하지 않는다(다른 종단점이다)",
          (r.stdout or "").strip() == "cso cso cso" and n == 3,
          "%r 조회 %d회 %s" % ((r.stdout or "").strip(), n, (r.stderr or "")[:120]))

    # ⓒ 말미 개행 — 셸이 `$( )` 로 개행을 먹고 **다른 종단점의 이름표**를 달던 길(codex R2).
    sock = os.path.join(_tmproot, "nl.sock")
    e_nl = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_SOCKET=sock + "\n",
                    CYS_BIN=stub_dir(0, "cso\n"))
    sh_got = sh_resolve(e_nl)
    e_plain = dict(e_nl); e_plain["CYS_SOCKET"] = sock; e_plain["CYS_BIN"] = stub_dir(2, "")
    py_got = resolve(e_plain)
    # (같은 TMPDIR 을 공유하므로 개행 **없는** 쪽의 정당한 실패표식 `.fail` 은 남는다 —
    #  금지되는 것은 개행 있는 쪽이 남기는 **역할 레코드**다.)
    _recs = [f for f in _cache_files(e_nl) if not f.endswith(".fail")]
    check("15c ★말미 개행 소켓의 답이 개행 없는 종단점의 권위가 되지 않는다(두 층 공통)",
          sh_got == "cso\tdaemon" and py_got == "master\tenv-cys-role" and _recs == [],
          "sh=%r py=%r recs=%s" % (sh_got, py_got, _recs))

    # ⓓ 512 초과 — 자르면 서로 다른 긴 경로가 한 신원이 된다. 자르지 않고 캐시를 끈다.
    longsock = "/" + ("a" * 600) + "/cys.sock"
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_SOCKET=longsock,
                 CYS_BIN=stub_dir(0, "cso\n"))
    py_got = resolve(e)
    sh_got = sh_resolve(e)
    check("15d ★512 초과 신원은 절단이 아니라 캐시 끔(두 층 동형)",
          py_got == "cso\tdaemon" and sh_got == "cso\tdaemon" and _cache_files(e) == [],
          "py=%r sh=%r files=%s" % (py_got, sh_got, _cache_files(e)))

    # ⓔ ★I7 재핀(판정관 T5c · **의도적 계약 변경**): `default:<XDG>:<HOME>` 은 이제 성분을
    #   **퍼센트 이스케이프**해 단사다(`%`→`%25` 먼저, `:`→`%3A`). 종전에는 `:` 하나로 캐시를
    #   통째로 껐는데, Windows 의 드라이브 지정 HOME(`C:\Users\x`)은 **늘** 거기에 걸려
    #   디스크 캐시도 `.fail` 백오프도 없이 훅마다 2s 데몬 왕복을 물었다(§7 ④ 방향 소실).
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n"))
    e["HOME"] = "/h:x"
    sh_got = sh_resolve(e)
    e2 = dict(e); e2["CYS_BIN"] = stub_dir(2, "")
    py_got = resolve(e2)
    check("15e ★`:` 가 든 기본 신원도 단사로 표현된다 — 캐시가 서고 두 층이 그 파일을 공유한다",
          sh_got == "cso\tdaemon" and py_got == "cso\tcache" and len(_cache_files(e)) == 1,
          "sh=%r py=%r files=%s" % (sh_got, py_got, _cache_files(e)))

    # ★단사성 — 종전에 **한 문자열로 접히던** 두 문맥이 서로 다른 신원이 된다(권위가 넘어가지
    #   않는다). 두 층이 같은 값을 내야 계약이 참이다.
    idents = []
    for xdg, home in (("/x:state", "/h"), ("/x", "state:/h")):
        ee = base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(2, ""))
        ee["XDG_STATE_HOME"] = xdg
        ee["HOME"] = home
        idents.append((_identity(ee, "7")[1], sh_sock_id(ee)))
    check("15e-2 ★모호했던 두 문맥이 서로 다른 신원이 된다(두 층 동형)",
          idents[0][0] and idents[0][0] == idents[0][1] and idents[1][0] == idents[1][1]
          and idents[0][0] != idents[1][0], "%r vs %r" % (idents[0], idents[1]))

    # ★Windows 정규 종단점(named pipe)에도 신원이 선다 — 그래야 `.fail` 백오프가 산다.
    for ep in ("\\\\.\\pipe\\cys", "\\\\?\\pipe\\cys-dept-one"):
        ee = base_env(CYS_SURFACE_ID="7", CYS_SOCKET=ep, CYS_BIN=stub_dir(2, ""))
        py_id, sh_id = _identity(ee, "7")[1], sh_sock_id(ee)
        check("15e-3 ★named pipe 종단점의 신원(두 층 동형) %s" % ep,
              py_id == ep and sh_id == ep, "py=%r sh=%r" % (py_id, sh_id))
    # 음성 대조: 이름이 빈 접두·`/` 가 섞인 값은 여전히 신원 미지다(pipe 인정이 만능이 아니다).
    for bad in ("\\\\.\\pipe\\", "\\\\.\\pipe\\a/b", "C:\\x", "rel.sock"):
        ee = base_env(CYS_SURFACE_ID="7", CYS_SOCKET=bad, CYS_BIN=stub_dir(2, ""))
        py_id, sh_id = _identity(ee, "7")[1], sh_sock_id(ee)
        check("15e-4 음성대조 신원 미지 %r" % bad, py_id == "" and sh_id == "",
              "py=%r sh=%r" % (py_id, sh_id))


def test_slug_parity_nonascii():
    """★R2(codex 위임 차분 프로브가 잡은 결함): 비-ASCII 소켓 경로에서 **두 층이 같은 파일**을 쓴다.

    종전 슬러그는 파이썬이 UTF-8 **바이트**를, macOS `tr` 가 **글자**를 세어 같은 소켓이 두 파일이
    됐다(밑줄 14 vs 6) — '두 층이 캐시를 공유한다'는 계약이 거짓이었다. 지금은 연속 치환을 접어
    (`tr -cs`) 어떤 입력에서도 결과가 같다.
    """
    sock = os.path.join(_tmproot, "한글", "소켓.sock")
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_SOCKET=sock,
                 CYS_BIN=stub_dir(0, "cso\n"))
    sh_got = sh_resolve(e)
    e2 = dict(e); e2["CYS_BIN"] = stub_dir(2, "")
    py_got = resolve(e2)
    check("15f ★비-ASCII 소켓 경로에서도 파이썬이 셸의 캐시를 읽는다(같은 파일명)",
          sh_got == "cso\tdaemon" and py_got == "cso\tcache" and len(_cache_files(e)) == 1,
          "sh=%r py=%r files=%s" % (sh_got, py_got, _cache_files(e)))

    # 신원 미지에서는 **함수 스스로** 경로를 내지 않는다(셸 짝 rc 1 과 동형).
    e = base_env(CYS_SURFACE_ID="7", CYS_SOCKET="relative.sock")
    check("15g ★신원 미지면 `_cache_path` 자체가 빈 값이다(호출측 방어에만 기대지 않는다)",
          _identity(e, "7")[0] == "", repr(_identity(e, "7")[0]))


def test_concurrent_writers():
    """★R2(codex): 동시 기록자 — 원자 교체라 **찢긴 레코드가 관측되지 않는다**.

    두 층(파이썬 8 · 셸 4)을 같은 캐시 자리에 동시에 붙인다. 각 판정은 `daemon`(첫 기록자) 또는
    `cache`(뒤이은 기록자) 중 하나여야 하고, 최종 파일은 **완결된 4필드 레코드 1줄**이어야 한다.
    (임시 파일에 쓰고 rename 하는 대신 대상 파일에 직접 쓰면 반쯤 쓰인 줄이 읽힌다.)
    """
    import threading
    e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "cso\n"))
    outs, lock = [], threading.Lock()

    def worker(shell):
        got = sh_resolve(e) if shell else resolve(e)
        with lock:
            outs.append(got)

    threads = [threading.Thread(target=worker, args=(i % 3 == 0,)) for i in range(12)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    ok = all(x in ("cso\tdaemon", "cso\tcache") for x in outs)
    path, sk, ep = _identity(e, "7")
    line = open(path, encoding="utf-8").read() if os.path.exists(path) else ""
    parts = line.rstrip("\n").split(" ", 3)
    check("16a ★동시 기록자 12 — 찢긴 레코드 0 · 판정은 daemon|cache 뿐",
          ok and len(parts) == 4 and parts[1] == "cso" and parts[3] == sk and parts[2] == ep,
          "outs=%s line=%r" % (sorted(set(outs)), line))
    leftovers = [f for f in _cache_files(e) if ".tmp" in f or f.endswith(".d")]
    check("16b 임시 잔재 0(전용 디렉터리는 매번 치운다)", leftovers == [], str(leftovers))


# ── ⑰ 수렴 R2: 최종 리뷰 blocking 2 + minor 3 의 회귀 핀 ─────────────────────
def _legacy_path(env, sid):
    """I2 이전 이름의 캐시 경로 — 해소기 자신의 규칙으로 계산한다(검체 재구현 금지)."""
    old_env = dict(os.environ)
    try:
        os.environ.clear()
        os.environ.update(env)
        JR.reset_cache()
        return JR._cache_path_legacy(sid)
    finally:
        os.environ.clear()
        os.environ.update(old_env)
        JR.reset_cache()


def _qcount(log):
    if not os.path.exists(log):
        return 0
    body = open(log, encoding="utf-8").read().strip()
    return len(body.split("\n")) if body else 0


def test_convergence_r2_backoff():
    """R2a(blocking · reviewer-codex): `.fail` 백오프가 **직접 확인**의 조회를 지워, 같은 uid 가
    쓸 수 있는 표식 한 줄이 stale env 를 통과 근거로 만들었다.

    신원은 I7 이 새로 표현하게 된 `:` 든 HOME 이다 — base 는 그 신원을 표현하지 못해 백오프
    자체가 없었고(매번 데몬에 물어 거부), HEAD 는 표현하면서 백오프까지 되살려 **거부가 허용으로**
    바뀌었다. 지금은 확인 경로가 표식을 무시하고 묻는다(표식은 계속 쓴다 · 일반 경로는 그대로).
    """
    home = os.path.join(_tmproot, "h:x")
    os.makedirs(home, exist_ok=True)
    log = os.path.join(_tmproot, "r2a.log")
    env = base_env(CYS_SURFACE_ID="7", CYS_ROLE="cso", HOME=home,
                   CYS_BIN=stub_dir(0, "worker\n", log=log))
    cpath, _sk, _ep = _identity(env, "7")
    check("R2a-0 전제: `:` 든 HOME 도 신원이 있다(I7)", cpath != "", repr(cpath))
    _seed_record(env, "7", "-", path=cpath + ".fail")
    got = resolve(env)
    check("R2a-1 양성대조: **일반** 해소는 백오프를 존중한다(조회 0 · env 폴백)",
          got == "cso\tenv-cys-role" and _qcount(log) == 0, "%r 조회 %d" % (got, _qcount(log)))
    rc, e = _org_rc(env)
    check("R2a-2 ★`.fail` 이 신선해도 직접 확인은 데몬에 묻는다(stale env 통과 차단)",
          rc == 3 and GATE_MSG in e and _qcount(log) == 1,
          "rc=%s 조회 %d err=%r" % (rc, _qcount(log), e.strip()[:100]))


def test_convergence_r2_legacy_slug():
    """R2b(blocking · reviewer-codex): I2 의 슬러그 변경이 고아로 만든 **거부 레코드**.

    `/tmp/a__b.sock` 에서 base 파이썬은 `role-7-_tmp_a__b.sock` 에 썼고 새 규칙은
    `role-7-_tmp_a_b.sock` 을 읽는다 — 만료 전 `worker` 레코드가 통째로 안 보이면 판정이
    'cache worker(거부)' 에서 'env cso(허용)' 으로 **뒤집힌다**. 이제 새 이름이 비면 옛 이름도
    한 번 읽는다(권위는 여전히 레코드의 sockid 원문 대조가 가른다).
    """
    sock = "/tmp/a__b.sock"
    env = base_env(CYS_SURFACE_ID="7", CYS_SOCKET=sock, CYS_ROLE="cso", CYS_BIN=stub_dir(2, ""))
    legacy = _legacy_path(env, "7")
    new = _identity(env, "7")[0]
    check("R2b-0 전제: 두 이름이 실제로 다르다(고아가 생기는 조건)",
          legacy and new and os.path.basename(legacy) != os.path.basename(new),
          "legacy=%r new=%r" % (os.path.basename(legacy or ""), os.path.basename(new or "")))
    _seed_record(env, "7", "worker", path=legacy)
    got = resolve(env)
    check("R2b-1 ★옛 이름의 신선한 `worker` 레코드는 여전히 권위다(거부→허용 뒤집기 차단)",
          got == "worker\tcache", got)
    # 음성 대조 ①: 옛 이름이어도 레코드의 sockid 가 다르면 **캐시 미스**다(별칭 금지).
    env2 = base_env(CYS_SURFACE_ID="7", CYS_SOCKET=sock, CYS_ROLE="cso", CYS_BIN=stub_dir(2, ""))
    _seed_record(env2, "7", "worker", path=_legacy_path(env2, "7"), sockid="/tmp/other.sock")
    check("R2b-2 음성대조: 옛 이름이어도 sockid 가 다르면 캐시 미스",
          resolve(env2) == "cso\tenv-cys-role", resolve(env2))
    # 음성 대조 ②: 옛 이름의 `.fail` 은 **되살리지 않는다** — 조회를 지우는 방향이라 허용 쪽이다.
    log = os.path.join(_tmproot, "r2b.log")
    env3 = base_env(CYS_SURFACE_ID="7", CYS_SOCKET=sock, CYS_ROLE="cso",
                    CYS_BIN=stub_dir(0, "worker\n", log=log))
    _seed_record(env3, "7", "-", path=_legacy_path(env3, "7") + ".fail")
    got = resolve(env3)
    check("R2b-3 ★옛 이름의 `.fail` 은 백오프로 인정하지 않는다(조회는 그대로 간다)",
          got == "worker\tdaemon" and _qcount(log) == 1, "%r 조회 %d" % (got, _qcount(log)))


_MEMO_HOME_DRIVER = """
import json, os, sys
sys.path.insert(0, %r)
import javis_role as R
out = []
for home in json.loads(sys.argv[1]):
    os.environ["HOME"] = home
    out.append(list(R.resolve_role_detail()))
print(json.dumps(out))
"""


def test_convergence_r2_memo_key():
    """R2c(minor · reviewer-codex 잔여 I6): 표현 **불가**한 기본 신원 둘이 한 메모를 공유했다.

    `_sock_id()` 가 둘 다 "" 를 내므로 키가 같았다 — 한 프로세스가 HOME 을 A→B 로 바꾸면
    B 데몬이 답할 참이던 것을 A 의 답으로 대신했다. 지금은 소켓 지정이 없을 때 **입력 원값**을
    키에 함께 싣는다.
    """
    log = os.path.join(_tmproot, "r2c.log")
    env = _no_disk_cache(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                  CYS_BIN=stub_dir(0, "cso\n", log=log)))
    a = "/tmp/" + "a/" * 300
    b = "/tmp/" + "b/" * 300
    check("R2c-0 전제: 두 HOME 모두 상한을 넘어 신원이 없다",
          _identity(dict(env, HOME=a), "7")[1] == "" and _identity(dict(env, HOME=b), "7")[1] == "",
          "")
    r = subprocess.run([sys.executable, "-c", _MEMO_HOME_DRIVER % BIN, json.dumps([a, b])],
                       capture_output=True, text=True, encoding="utf-8", timeout=60,
                       env=env, cwd=BIN)
    check("R2c-1 ★표현 불가한 두 HOME 은 메모를 공유하지 않는다(조회 2회)",
          _qcount(log) == 2, "조회 %d회 out=%r err=%r" % (_qcount(log), r.stdout.strip()[:80],
                                                        r.stderr.strip()[:120]))


def test_convergence_r2_length_unit():
    r"""R2d(minor · reviewer-codex): 길이 단위 파리티 — `${#var}` 는 dash 가 바이트·bash/zsh 가
    글자였다. `\.\pipe\` + `한`*200 은 209자·609바이트라 파이썬·bash·zsh 는 신원을 인정하고
    dash 만 거절했다(같은 좌석에서 캐시·백오프가 층마다 켜졌다 꺼졌다). 지금은 네 층이 바이트다.
    """
    long_pipe = "\\\\.\\pipe\\" + "\ud55c" * 200
    env = base_env(CYS_SURFACE_ID="7", CYS_SOCKET=long_pipe, CYS_ROLE="cso",
                   LC_ALL="en_US.UTF-8", LANG="en_US.UTF-8")
    py = _identity(env, "7")[1]
    layers = {"py": py}
    for interp in ("sh", "bash", "zsh", "dash"):
        if shutil.which(interp):
            layers[interp] = sh_sock_id(env, interp)
    check("R2d-1 ★609바이트 종단점은 어느 층에서도 신원이 아니다(바이트 단위 통일)",
          all(v == "" for v in layers.values()), repr(layers))
    # 양성 대조: 같은 문자열의 짧은 판(3글자·9바이트)은 네 층 모두 **같은 신원**이다.
    short_pipe = "\\\\.\\pipe\\" + "\ud55c" * 3
    env = base_env(CYS_SURFACE_ID="7", CYS_SOCKET=short_pipe, CYS_ROLE="cso",
                   LC_ALL="en_US.UTF-8", LANG="en_US.UTF-8")
    py = _identity(env, "7")[1]
    got = {"py": py}
    for interp in ("sh", "bash", "zsh", "dash"):
        if shutil.which(interp):
            got[interp] = sh_sock_id(env, interp)
    check("R2d-2 양성대조: 상한 안의 비-ASCII pipe 는 네 층이 같은 신원을 낸다",
          py == short_pipe and len(set(got.values())) == 1, repr(got))


def test_convergence_r2_snapshot_none():
    """R2e(minor · reviewer-claude — **반박 후 문면만 정정**): 확인 답이 `daemon-none` 일 때
    `is_master()` 가 대장 절로 흘러 stale 대장으로 True 를 낼 수 있다는 지적은 **재현되지
    않는다**(`is_authoritative("daemon-none")` 이 참이고 역할 ""≠master 라 그 앞에서 끊긴다).
    다만 사유 문면이 'not master' 로 나가 세 형제(org·dept)와 어긋났으므로 전용 절을 세웠다 —
    판정은 그대로 False 이고, 이 핀이 그 사실을 고정한다.
    """
    import json as _json
    state = os.path.join(_tmproot, "r2e-state")
    os.makedirs(state, exist_ok=True)
    with open(os.path.join(state, "mission.json"), "w", encoding="utf-8") as f:
        _json.dump({"schema": 1, "mission": None, "surface": "7"}, f)
    # 디스크 캐시=master(신선) · env=worker · 데몬 확인=권위 무역할 → 대장이 일치해도 불통과.
    env = base_env(CYS_SURFACE_ID="7", CYS_ROLE="worker", CYS_BIN=stub_dir(0, "\n"))
    _seed_record(env, "7", "master")
    rc, out = _snapshot_rc(env, state)
    check("R2e-1 ★캐시=master + 확인=권위 무역할 → 불통과(대장 일치해도)",
          rc == 1 and "daemon knows no role" in out, "rc=%s out=%r" % (rc, out.strip()))


def test_reflect_g10_snapshot_backoff():
    """★0.14.31 성찰 G10(major): 관측 훅(`is_master`)은 **살아 있는 백오프** 안에서 데몬 타임아웃을
    반복하지 않는다(보류 = 생산 skip) · 위조 표식으로 권한 상승 0 · 표식 만료 뒤 정상 생성 ·
    8h(첫 실패는 종전대로 env 절 통과)는 불변. 권한 변이 경로(require_cso)는 R2a-2 가 따로 핀한다."""
    import json
    state = os.path.join(_tmproot, "g10state")
    os.makedirs(state, exist_ok=True)
    with open(os.path.join(state, "mission.json"), "w", encoding="utf-8") as f:
        json.dump({"schema": 1, "mission": None, "surface": "7"}, f)
    nostate = os.path.join(_tmproot, "g10state-empty")
    os.makedirs(nostate, exist_ok=True)

    def _fresh_fail(env, age=0):
        cpath, _sk, _ep = _identity(env, "7")
        _seed_record(env, "7", "-", ts=int(time.time()) - age, path=cpath + ".fail")

    # (i) 살아 있는 백오프 + env master + 데몬 worker → 보류(조회 0 · 타임아웃 0 · 상승 0)
    log = os.path.join(_tmproot, "g10-1.log")
    env = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "worker\n", log=log))
    _fresh_fail(env)
    rc, out = _snapshot_rc(env, nostate)
    check("G10-1 ★백오프 안 env master: 보류(rc 1 · 사유 backoff) · 데몬 조회 0",
          rc == 1 and "backoff" in out and _qcount(log) == 0,
          "rc=%s out=%r q=%d" % (rc, out.strip(), _qcount(log)))
    # (ii) 위조 표식 + env master + 데몬이 master 라고 답할 상황 → **여전히 보류**(표식은 허용 근거가 아니다)
    log2 = os.path.join(_tmproot, "g10-2.log")
    env2 = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "master\n", log=log2))
    _fresh_fail(env2)
    rc, out = _snapshot_rc(env2, state)
    check("G10-2 ★위조 표식은 통과 근거가 아니다(보류 · 조회 0)",
          rc == 1 and "backoff" in out and _qcount(log2) == 0, "rc=%s out=%r" % (rc, out.strip()))
    # (iii) 표식 만료(31s) + 데몬 master → 조회 1회 · 정상 생성(회복)
    log3 = os.path.join(_tmproot, "g10-3.log")
    env3 = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "master\n", log=log3))
    _fresh_fail(env3, age=31)
    rc, out = _snapshot_rc(env3, state)
    check("G10-3 데몬 회복(표식 만료) → 조회 1회 · 정상 생성", rc == 0 and _qcount(log3) == 1,
          "rc=%s out=%r q=%d" % (rc, out.strip(), _qcount(log3)))
    # (iv) 권위 캐시 master + env 없음 + 살아 있는 백오프 → 확인 갈래 (a) 도 보류(조회 0)
    log4 = os.path.join(_tmproot, "g10-4.log")
    env4 = base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(0, "worker\n", log=log4))
    _seed_record(env4, "7", "master")
    _fresh_fail(env4)
    rc, out = _snapshot_rc(env4, state)
    check("G10-4 ★캐시 master 갈래도 백오프 안에서는 보류(조회 0 · 캐시는 통과 근거가 아니다)",
          rc == 1 and "backoff" in out and _qcount(log4) == 0, "rc=%s out=%r" % (rc, out.strip()))
    # (v) 8h 불변: 표식 없음 + env master + 데몬 판정 불가 → 첫 실패는 조회 1회 · 종전 env 절 통과
    log5 = os.path.join(_tmproot, "g10-5.log")
    env5 = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(2, "", log=log5))
    rc, out = _snapshot_rc(env5, state)
    check("G10-5 양성 대조(8h 불변): 표식 없는 첫 실패는 조회 1회 · env 절 통과",
          rc == 0 and _qcount(log5) == 1, "rc=%s out=%r q=%d" % (rc, out.strip(), _qcount(log5)))
    # (vi) 손상·미래·신원 불일치 표식은 백오프가 아니다(순수 판독의 검증 규율 · codex minor)
    for _lab, _kw in (("미래 시각", {"ts": int(time.time()) + 3600}),
                      ("신원 불일치", {"sockid": "/tmp/other.sock"}),
                      ("세대 불일치", {"epoch": "zz"})):
        _l = os.path.join(_tmproot, "g10-6-%s.log" % _lab)
        _e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "worker\n", log=_l))
        _cp, _sk, _ep = _identity(_e, "7")
        _seed_record(_e, "7", "-", path=_cp + ".fail", **_kw)
        rc, out = _snapshot_rc(_e, nostate)
        check("G10-6 %s 표식은 백오프가 아니다(확인이 데몬에 묻는다 · 조회 1)" % _lab,
              rc == 1 and "backoff" not in out and _qcount(_l) == 1,
              "rc=%s out=%r q=%d" % (rc, out.strip(), _qcount(_l)))
    # (vi-b) **문법이 깨진** 표식(옛 2필드 레코드·쓰레기 한 줄)도 백오프가 아니다 —
    #   위 3종은 4필드 문법을 지키고 값만 틀린 것이라, 문법 축은 따로 잰다(codex minor).
    for _lab, _raw in (("옛 2필드", "%d -\n" % int(time.time())),
                       ("쓰레기 한 줄", "not a record at all\n"),
                       ("빈 파일", "")):
        _l = os.path.join(_tmproot, "g10-6b-%s.log" % _lab)
        _e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(0, "worker\n", log=_l))
        _cp, _sk, _ep = _identity(_e, "7")
        with open(_cp + ".fail", "w", encoding="utf-8") as _f:
            _f.write(_raw)
        rc, out = _snapshot_rc(_e, nostate)
        check("G10-6b %s 표식은 백오프가 아니다(조회 1)" % _lab,
              rc == 1 and "backoff" not in out and _qcount(_l) == 1,
              "rc=%s out=%r q=%d" % (rc, out.strip(), _qcount(_l)))

    # (vii) ★훅의 실제 형상(codex 설계 비평 · 누락 검체): **같은 TMPDIR·같은 좌석**에서 새
    #   프로세스 3회 — 첫 실패는 조회 1회·생성 / 30s 안 둘째는 조회 0회·보류(반복 타임아웃 0)
    #   / 표식 만료 + 데몬 회복은 조회 1회·재생성. 종전 검체는 호출마다 TMPDIR 이 달라
    #   "연속 훅 호출" 이라는 이 항목의 전제 자체를 재지 못했다.
    shared = os.path.join(_tmproot, "g10-hookseq")
    os.makedirs(shared, exist_ok=True)

    def _seq_env(rc_, out_, tag):
        _l = os.path.join(_tmproot, "g10-7%s.log" % tag)
        _e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(rc_, out_, log=_l))
        _e["TMPDIR"] = shared                      # ★캐시 디렉터리를 세 호출이 공유한다
        return _e, _l

    e7a, l7a = _seq_env(2, "", "a")
    rc, out = _snapshot_rc(e7a, state)
    check("G10-7a 훅① 표식 없음 → 조회 1회 · 종전 env 판정으로 생성",
          rc == 0 and _qcount(l7a) == 1, "rc=%s out=%r q=%d" % (rc, out.strip(), _qcount(l7a)))
    e7b, l7b = _seq_env(2, "", "b")
    rc, out = _snapshot_rc(e7b, state)
    check("G10-7b 훅② 30s 안 → 조회 0회 · 보류(★반복 타임아웃 0 = 이 항목의 목적)",
          rc == 1 and "backoff" in out and _qcount(l7b) == 0,
          "rc=%s out=%r q=%d" % (rc, out.strip(), _qcount(l7b)))
    _cp7, _sk7, _ep7 = _identity(e7b, "7")
    _seed_record(e7b, "7", "-", ts=int(time.time()) - 31, path=_cp7 + ".fail")
    e7c, l7c = _seq_env(0, "master\n", "c")
    rc, out = _snapshot_rc(e7c, state)
    check("G10-7c 훅③ 표식 만료 + 데몬 회복 → 조회 1회 · 재생성(복구가 막히지 않는다)",
          rc == 0 and _qcount(l7c) == 1, "rc=%s out=%r q=%d" % (rc, out.strip(), _qcount(l7c)))

    # (viii) ★순수 판독 계약(codex minor): 술어 호출만으로 0700 캐시 디렉터리를 **만들지 않는다**.
    nomk = os.path.join(_tmproot, "g10-nomkdir")
    os.makedirs(nomk, exist_ok=True)
    e8 = base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(2, ""))
    e8["TMPDIR"] = nomk
    _code8 = ("import os, sys; sys.path.insert(0, %r); import javis_role as R;"
              "print(R.fail_backoff_active());"
              "print(os.path.isdir(os.path.join(os.environ['TMPDIR'], R.CACHE_DIR_NAME)))" % BIN)
    _r8 = subprocess.run([sys.executable, "-c", _code8], capture_output=True, text=True,
                         timeout=60, env=e8, cwd=BIN)
    check("G10-8 ★술어는 순수 판독 — 표식 없음(False) · 캐시 디렉터리 생성 0",
          (_r8.stdout or "").split() == ["False", "False"],
          "%r %r" % ((_r8.stdout or "").strip(), (_r8.stderr or "")[-160:]))

    # (ix) ★프로세스 안 표식(`_LIVE_FAIL_MONO`)을 술어에서 뺀 것의 회귀 핀(codex minor):
    #   **신원을 표현할 수 없는** 좌석(상대 경로 소켓 = 디스크 표식이 아예 안 써진다)에서
    #   같은 프로세스가 `is_master()` 를 두 번 불러도 판정은 종전 그대로다(True True).
    #   종전 술어는 이 자리에서 둘째 호출을 **보류**시켰다 — 아끼는 타임아웃 0의 새 거부였다.
    e9 = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_SOCKET="cys.sock",
                  CYS_BIN=stub_dir(2, ""))
    _code9 = ("import sys; sys.path.insert(0, %r); import javis_snapshot as S;"
              "print(S.is_master()[0], S.is_master()[0])" % BIN)
    _r9 = subprocess.run([sys.executable, "-c", _code9], capture_output=True, text=True,
                         timeout=60, env=e9, cwd=BIN)
    check("G10-9 ★신원 미표현 좌석의 같은 프로세스 재호출은 종전 판정 그대로(True True)",
          (_r9.stdout or "").split() == ["True", "True"],
          "%r %r" % ((_r9.stdout or "").strip(), (_r9.stderr or "")[-160:]))


def main():
    try:
        test_three_states()
        test_env_compat_keys()
        test_fallback_is_cys_role_only()
        test_cache_and_backoff()
        test_memo_lifetime()
        test_cache_hygiene()
        test_cross_layer_cache()
        test_no_autostart_and_env_purity()
        test_org_gate()
        test_snapshot_gate()
        test_guard_label()
        test_destroy_half_op()
        test_identity_grammar()
        test_cache_substrate()
        test_record_grammar_parity()
        test_slug_collision_and_memo_key()
        test_endpoint_identity()
        test_slug_parity_nonascii()
        test_concurrent_writers()
        test_convergence_r2_backoff()
        test_convergence_r2_legacy_slug()
        test_convergence_r2_memo_key()
        test_convergence_r2_length_unit()
        test_convergence_r2_snapshot_none()
        test_reflect_g10_snapshot_backoff()
    finally:
        shutil.rmtree(_tmproot, ignore_errors=True)
    if fails:
        print("test_role_authority FAIL (%d): %s" % (len(fails), fails))
        return 1
    print("ROLE-AUTHORITY-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
