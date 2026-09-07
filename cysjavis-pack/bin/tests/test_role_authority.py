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
    rc, e = _org_rc(base_env(CYS_SURFACE_ID="7", CYS_ROLE="worker", CYS_BIN=stub_dir(0, "cso\n")))
    check("8d ★org: 데몬=cso 인데 env=worker → **여전히 거부**(단조 거부 — 새 허용 0)",
          rc == 3 and GATE_MSG in e, "rc=%s" % rc)
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
    import re

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
    slug = lambda value: re.sub(rb"[^A-Za-z0-9._-]", b"_", value.encode()).decode()[:80]
    disagreed = []
    for i, (label, line, expected) in enumerate(cases, 1):
        e = base_env(CYS_SURFACE_ID="7", CYS_ROLE="master", CYS_BIN=stub_dir(2, ""))
        d = os.path.join(e["TMPDIR"], "cys-role-authority.d")
        os.makedirs(d, mode=0o700)
        sockid = ("default:%s:%s" % (e.get("XDG_STATE_HOME", ""), e.get("HOME", "")))[:512]
        cpath = os.path.join(d, "role-%s-%s" % (slug(e["CYS_SURFACE_ID"]), slug(sockid)))
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
        test_destroy_half_op()
        test_identity_grammar()
        test_cache_substrate()
        test_record_grammar_parity()
        test_slug_collision_and_memo_key()
    finally:
        shutil.rmtree(_tmproot, ignore_errors=True)
    if fails:
        print("test_role_authority FAIL (%d): %s" % (len(fails), fails))
        return 1
    print("ROLE-AUTHORITY-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
