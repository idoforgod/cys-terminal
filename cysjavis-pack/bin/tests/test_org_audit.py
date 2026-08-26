#!/usr/bin/env python3
"""test_org_audit.py — javis_org.py org-audit(T10·P3-3 최소안) 수렴/미수렴 exit 계약 핀.

가짜 HOME + PATH 선두 스텁 cys(상태 JSON 배달)로 실 데몬 무접촉 검증:
  A) 승격 완료(.pre-ceo)·PENDING 無·base/dept 데몬 생존·5역할 전원 agent → exit 0(수렴)
     + 출력 스키마 {ceo, depts:{name:{daemon_alive, master_seat, team_state}}}
  B) worker 좌석 agent 부재 → exit 1(미수렴)·team_state=partial·missing에 worker
  C) ceo-pending 잔존 → exit 1(미수렴)·ceo.state=pending (팀 complete여도)
  D) 부서 데몬 사망 → exit 1·daemon_alive=false·team_state=unknown
  E) 부서 0 + 미승격 → exit 0(표준 master 상태 = 수렴)
  RO) read-only 계약: 실행 전후 HOME 트리 파일 집합 불변(영속 쓰기 0 — 정본 이원화 금지)
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(SELF, "..", "javis_org.py")
PY = sys.executable or "python3"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


FULL_SURFACES = {"surfaces": [
    {"role": "master", "agent": "claude", "agent_alive": True, "exited": False},
    {"role": "cso", "agent": "claude", "exited": False},
    {"role": "worker", "agent": "claude", "exited": False},
    {"role": "reviewer-gemini", "agent": "gemini", "exited": False},
    {"role": "reviewer-codex", "agent": "codex", "exited": False},
]}


def setup(tmp, *, ndepts=1, promoted=True, pending=False, dept_surfaces=FULL_SURFACES,
          dept_alive=True):
    home = os.path.join(tmp, "home")
    directives = os.path.join(home, ".cys", "pack", "directives")
    bindir = os.path.join(tmp, "stubbin")
    os.makedirs(directives, exist_ok=True)
    os.makedirs(bindir, exist_ok=True)
    with open(os.path.join(directives, "MASTER_DIRECTIVE.md"), "w", encoding="utf-8") as f:
        f.write("CEO-BODY\n" if promoted else "STANDARD-MASTER\n")
    with open(os.path.join(directives, "CEO_TEMPLATE.md"), "w", encoding="utf-8") as f:
        f.write("CEO-BODY\n")
    if promoted:
        with open(os.path.join(directives, "MASTER_DIRECTIVE.md.pre-ceo"), "w",
                  encoding="utf-8") as f:
            f.write("STANDARD-MASTER\n")
    with open(os.path.join(home, ".cys", ".master-bootstrapped"), "w", encoding="utf-8") as f:
        f.write("{}")
    state = os.path.join(home, ".cys", "state")
    os.makedirs(state, exist_ok=True)
    if pending:
        with open(os.path.join(state, "ceo-pending"), "w", encoding="utf-8") as f:
            f.write("pending\n")
    reg = os.path.join(home, ".cys", "depts.json")
    depts = {("d%d" % i): {"socket": os.path.join(tmp, "cys-dept-d%d" % i, "cys.sock")}
             for i in range(ndepts)}
    with open(reg, "w", encoding="utf-8") as f:
        json.dump({"depts": depts}, f)
    # 상태 배달 파일 — base 는 항상 생존, dept 는 dept_alive 로 제어
    with open(os.path.join(tmp, "base-status.json"), "w", encoding="utf-8") as f:
        json.dump({"surfaces": []}, f)
    if dept_alive:
        with open(os.path.join(tmp, "dept-status.json"), "w", encoding="utf-8") as f:
            json.dump(dept_surfaces, f)
    # 스텁 cys: `--socket …` = 부서(dept-status.json 부재 시 exit 1=사망) / 그 외 = base
    stub = os.path.join(bindir, "cys")
    with open(stub, "w", encoding="utf-8", newline="\n") as f:
        f.write("#!/bin/sh\n"
                "if [ \"$1\" = \"--socket\" ]; then\n"
                "  [ -f \"%s/dept-status.json\" ] || exit 1\n"
                "  cat \"%s/dept-status.json\"; exit 0\n"
                "fi\ncat \"%s/base-status.json\"\n" % (tmp, tmp, tmp))
    os.chmod(stub, 0o755)
    env = dict(os.environ)
    # PYTHONDONTWRITEBYTECODE: RO 핀의 관측 대상은 **audit 자신의 영속**이다 — macOS 시스템
    # python 이 가짜 HOME 의 Library/Caches 에 남기는 .pyc 인터프리터 캐시는 판정 잡음이라 차단.
    env.update({"HOME": home, "CYS_DEPTS_JSON": reg, "PYTHONDONTWRITEBYTECODE": "1",
                "PATH": bindir + os.pathsep + env.get("PATH", "")})
    for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_STATE_DIR",
              "CYS_FORMATION_EXTERNAL_ROLES", "CYS_DEPT_CATALOG"):
        env.pop(k, None)
    return env, home


def audit(env):
    r = subprocess.run([PY, SCRIPT, "org-audit"], capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=60)
    try:
        data = json.loads(r.stdout)
    except Exception:
        data = {}
    return r.returncode, data, r.stderr


def tree(home):
    """RO 핀 관측역 = $HOME/.cys — 정본 이원화(org-state.json 등 신설 영속)가 생긴다면 여기다.
    인터프리터 캐시(Library/Caches) 등 audit 밖 잡음은 관측 대상이 아니다."""
    out = set()
    for root, _dirs, files in os.walk(os.path.join(home, ".cys")):
        for fn in files:
            out.add(os.path.join(root, fn))
    return out


# ── A. 수렴: 승격·PENDING 無·데몬 생존·5역할 전원 ──
tmp = tempfile.mkdtemp(prefix="orgaudit-a-")
env, home = setup(tmp)
before = tree(home)
code, data, err = audit(env)
after = tree(home)
check("A1 수렴 exit 0", code == 0, "exit=%d err=%s" % (code, err[-200:]))
check("A2 audit=converged", data.get("audit") == "converged")
d0 = (data.get("depts") or {}).get("d0") or {}
check("A3 스키마: daemon_alive/master_seat/team_state",
      d0.get("daemon_alive") is True and d0.get("master_seat") == "agent"
      and d0.get("team_state") == "complete", json.dumps(d0, ensure_ascii=False))
check("A4 ceo 축: state=ceo·promoted", (data.get("ceo") or {}).get("state") == "ceo"
      and (data.get("ceo") or {}).get("promoted") is True)
check("A-RO read-only(영속 쓰기 0 — HOME 트리 불변)", before == after,
      "diff=%r" % sorted(after ^ before))
shutil.rmtree(tmp)

# ── B. worker agent 부재 → 미수렴·partial ──
surf_b = {"surfaces": [s for s in FULL_SURFACES["surfaces"] if s["role"] != "worker"]
          + [{"role": "worker", "agent": None, "exited": False}]}
tmp = tempfile.mkdtemp(prefix="orgaudit-b-")
env, home = setup(tmp, dept_surfaces=surf_b)
code, data, err = audit(env)
d0 = (data.get("depts") or {}).get("d0") or {}
check("B1 미수렴 exit 1", code == 1, "exit=%d" % code)
check("B2 team_state=partial·missing=worker",
      d0.get("team_state") == "partial" and d0.get("missing") == ["worker"],
      json.dumps(d0, ensure_ascii=False))
shutil.rmtree(tmp)

# ── C. PENDING 잔존 → 미수렴(팀 complete여도) ──
tmp = tempfile.mkdtemp(prefix="orgaudit-c-")
env, home = setup(tmp, pending=True)
code, data, err = audit(env)
check("C1 pending 미수렴 exit 1", code == 1, "exit=%d" % code)
check("C2 ceo.state=pending", (data.get("ceo") or {}).get("state") == "pending")
check("C3 dept 축은 수렴 유지(원인 국지화)",
      ((data.get("depts") or {}).get("d0") or {}).get("converged") is True)
shutil.rmtree(tmp)

# ── D. 부서 데몬 사망 → 미수렴·unknown ──
tmp = tempfile.mkdtemp(prefix="orgaudit-d-")
env, home = setup(tmp, dept_alive=False)
code, data, err = audit(env)
d0 = (data.get("depts") or {}).get("d0") or {}
check("D1 데몬 사망 미수렴 exit 1", code == 1, "exit=%d" % code)
check("D2 daemon_alive=false·team_state=unknown",
      d0.get("daemon_alive") is False and d0.get("team_state") == "unknown",
      json.dumps(d0, ensure_ascii=False))
shutil.rmtree(tmp)

# ── E. 부서 0 + 미승격 = 표준 master 상태 → 수렴 ──
tmp = tempfile.mkdtemp(prefix="orgaudit-e-")
env, home = setup(tmp, ndepts=0, promoted=False)
code, data, err = audit(env)
check("E1 부서 0·미승격 수렴 exit 0", code == 0, "exit=%d err=%s" % (code, err[-200:]))
check("E2 ceo.state=master", (data.get("ceo") or {}).get("state") == "master")
shutil.rmtree(tmp)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
