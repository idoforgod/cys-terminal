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
  ⑤ ★셸 짝(`hooks/_lib.sh cys_resolve_role`)과 **같은 캐시 파일·같은 형식**을 읽고 쓴다
  ⑥ 캐시 위생 — 미래 시각·손상 형식·심링크는 신뢰하지 않는다
  ⑦ 신원 키 순서 CYS_→JAVIS_→AITERM_(Rust `env_compat` 미러) · 자릿수 상한 19
  ⑧ 소비처 단조 거부 — javis_org.require_cso · javis_snapshot.is_master · completion_guard._role
  ⑨ ★음성 대조: 부모 프로세스 env 는 조회로 오염되지 않는다(CYS_NO_AUTOSTART 누출 0)
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 ROLE-AUTHORITY-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_role_authority.py
"""
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
    log3 = os.path.join(_tmproot, "memo.log")
    e = base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(0, "cso\n", log=log3))
    code = ("import sys; sys.path.insert(0, %r); import javis_role as R;"
            "print(R.resolve_role(), R.resolve_role(), R.resolve_role())" % BIN)
    subprocess.run([sys.executable, "-c", code], capture_output=True, text=True,
                   timeout=60, env=e)
    n = len(open(log3, encoding="utf-8").read().strip().split("\n"))
    check("4e 프로세스 메모 — 3회 질의에 조회 1회", n == 1, "조회 %d회" % n)


# ── ⑥ 캐시 위생 ──────────────────────────────────────────────────────────────
def test_cache_hygiene():
    def seeded(line, mode="file"):
        e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master")
        env2 = dict(os.environ); env2.update(e)
        old = dict(os.environ)
        try:
            os.environ.clear(); os.environ.update(e)
            JR.reset_cache()
            path = JR._cache_path("7")
        finally:
            os.environ.clear(); os.environ.update(old)
            JR.reset_cache()
        if mode == "file":
            with open(path, "w", encoding="utf-8") as f:
                f.write(line)
            os.chmod(path, 0o600)
        else:
            target = path + ".target"
            with open(target, "w", encoding="utf-8") as f:
                f.write(line)
            os.symlink(target, path)
        return e

    now = int(time.time())
    for label, line, mode in (("6a 미래 시각 캐시는 신선이 아니다", "%d cso\n" % (now + 9999), "file"),
                              ("6b 손상 형식(공백 없음) 캐시는 무시", "noSpaceLine\n", "file"),
                              ("6c ★심링크 캐시는 판독하지 않는다", "%d cso\n" % now, "symlink")):
        e = seeded(line, mode)
        e["CYS_BIN"] = stub_dir(2, "")
        got = resolve(e)
        check(label, got == "master\tenv-cys-role", got)


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


def test_org_gate():
    rc, _e = _org_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="cso", CYS_BIN=stub_dir(0, "worker\n")))
    check("8a ★org: env=cso 인데 데몬=worker → 거부(승계 후 stale 로 부서 mutation 하던 길)",
          rc == 3, "rc=%s" % rc)
    rc, _e = _org_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="cso", CYS_BIN=stub_dir(0, "\n")))
    check("8b org: env=cso 인데 데몬이 '역할 없음' → 거부", rc == 3, "rc=%s" % rc)
    rc, _e = _org_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="cso", CYS_BIN=stub_dir(2, "")))
    check("8c ★org: 판정 불가 → 종전 그대로 통과(게이트에서 안 막힌다)", rc != 3, "rc=%s" % rc)
    rc, _e = _org_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="worker", CYS_BIN=stub_dir(0, "cso\n")))
    check("8d ★org: 데몬=cso 인데 env=worker → **여전히 거부**(단조 거부 — 새 허용 0)",
          rc == 3, "rc=%s" % rc)
    rc, _e = _org_rc(base_env(CYS_ROLE="cso", CYS_BIN=stub_dir(0, "worker\n")))
    check("8e org: 주소 없음 → 종전 env 판정만(통과)", rc != 3, "rc=%s" % rc)


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

    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                    CYS_BIN=stub_dir(0, "worker\n")), state)
    check("8f ★snapshot: 데몬=worker 면 env=master 도 대장 일치도 통과시키지 않는다",
          rc == 1 and "not-master" in out, "rc=%s out=%r" % (rc, out.strip()))
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                    CYS_BIN=stub_dir(0, "master\n")), state)
    check("8g snapshot: 데몬=master → 통과", rc == 0, "rc=%s out=%r" % (rc, out.strip()))
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="master",
                                    CYS_BIN=stub_dir(2, "")), state)
    check("8h snapshot: 판정 불가 → 종전 env 절 그대로 통과", rc == 0, "rc=%s" % rc)
    rc, out = _snapshot_rc(base_env(CYS_SURFACE_ID="7", CYS_BIN=stub_dir(0, "\n")), state)
    check("8i snapshot: 권위 무역할이어도 대장 일치 절은 살아 있다(종전 거부 없음)",
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


def main():
    try:
        test_three_states()
        test_env_compat_keys()
        test_fallback_is_cys_role_only()
        test_cache_and_backoff()
        test_cache_hygiene()
        test_cross_layer_cache()
        test_no_autostart_and_env_purity()
        test_org_gate()
        test_snapshot_gate()
        test_guard_label()
    finally:
        shutil.rmtree(_tmproot, ignore_errors=True)
    if fails:
        print("test_role_authority FAIL (%d): %s" % (len(fails), fails))
        return 1
    print("ROLE-AUTHORITY-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
