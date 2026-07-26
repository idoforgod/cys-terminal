#!/usr/bin/env python3
"""test_dept_account_dir.py — CU-5A `dept_account_dir` 복구 규칙 동적 검증(샌드박스 HOME/REG).

설계 정본 DESIGN_scope-first-class.md §4 CU-5A.1 + SIM_REPORT SIM-1(복구 규칙 v2 채택 근거).
우선순위: ①depts.json `account_dir` → ②부서 팩 agents.json CLAUDE_CONFIG_DIR 리터럴
(`$` 미전개 템플릿 제외·디렉터리 실재 시에만 채택 → 성공 시 ①로 역기입 자기치유).
`resolve_default_base` 재산출은 기각(SIM-1) — 이 테스트는 그 기각도 함께 핀으로 고정한다.

전부 mktemp 샌드박스(가짜 HOME·가짜 REG). 실 데몬·라이브 팩(~/.cys/pack*)·실 depts.json 무접촉:
cys-dept를 `sock`(순수 echo 아암)으로 **source**해 함수만 취하고, 함수를 직접 호출해 실측한다.

분기:
  D1 레지스트리 有(dir 실재)            → exit 0 · 그 값 · agents.json 미조회
  D2 레지스트리 無 + agents 리터럴 실재  → exit 0 · 리터럴 값 · depts.json 역기입(자기치유)
  D3 레지스트리 有(dir 부재)            → exit 5 fail-loud(stdout 무출력·사유 stderr)
  D4 양쪽 無                            → exit 1 · 무출력(신규·온보딩 경로 = 종전 spawn)
  D5 agents 리터럴이 `$` 템플릿          → 미채택(base 템플릿 오채택 방어 · SIM-1)
  D6 agents 리터럴 dir 미실재            → 미채택 + WARN
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
DEPT = os.path.join(SELF, "..", "cys-dept")
fails = []

# 함수만 취하는 하네스: `sock` 아암은 dept_sock echo 1줄(레지스트리·python·데몬 무접촉)이라
# source 부작용이 없다. 이후 set +e 로 exit code를 직접 회수한다.
HARNESS = r"""
source "$DEPT" sock __probe__ >/dev/null
set +e
out="$(dept_account_dir "$NAME" 2>"$ERRF")"; rc=$?
printf '%s\n' "rc=$rc"
printf '%s\n' "out=$out"
"""


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def sandbox(depts, agents=None, mkdirs=()):
    """가짜 HOME 생성: depts.json + (선택)부서 팩 agents.json + 실재시킬 디렉터리들."""
    tmp = tempfile.mkdtemp(prefix="dad-")
    home = os.path.join(tmp, "home")
    os.makedirs(os.path.join(home, ".cys"), exist_ok=True)
    for d in mkdirs:
        os.makedirs(d.replace("@HOME@", home), exist_ok=True)
    reg = os.path.join(home, ".cys", "depts.json")
    with open(reg, "w", encoding="utf-8") as f:
        json.dump({"depts": json.loads(json.dumps(depts).replace("@HOME@", home))}, f)
    if agents:
        for dept, conf in agents.items():
            pk = os.path.join(home, ".cys", "pack-dept-%s" % dept)
            os.makedirs(pk, exist_ok=True)
            with open(os.path.join(pk, "agents.json"), "w", encoding="utf-8") as f:
                f.write(json.dumps(conf, ensure_ascii=False).replace("@HOME@", home))
    return tmp, home, reg


def call(tmp, home, reg, name):
    """dept_account_dir <name> 실측 → (rc, stdout값, stderr)."""
    errf = os.path.join(tmp, "err.txt")
    env = dict(os.environ)
    env.update({"HOME": home, "CYS_DEPTS_JSON": reg, "DEPT": os.path.abspath(DEPT),
                "NAME": name, "ERRF": errf})
    for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_ACCOUNT_DIR"):
        env.pop(k, None)
    r = subprocess.run(["bash", "-c", HARNESS], capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=60)
    rc, out = -1, ""
    for line in r.stdout.splitlines():
        if line.startswith("rc="):
            rc = int(line[3:])
        elif line.startswith("out="):
            out = line[4:]
    err = open(errf, encoding="utf-8").read() if os.path.exists(errf) else ""
    return rc, out, err + r.stderr


# ── D1: 레지스트리 有(dir 실재) → 그 값 채택. agents.json은 다른 값이어도 무시(①이 권위) ──
tmp, home, reg = sandbox(
    {"dept-1": {"socket": "s", "pack_dir": "p", "role": "dept-master",
                "account_dir": "@HOME@/.cys/claude-reg-dept-1"}},
    agents={"dept-1": {"claude": {"cmd": "claude",
                                  "env": {"CLAUDE_CONFIG_DIR": "@HOME@/.cys/claude-other"}}}},
    mkdirs=("@HOME@/.cys/claude-reg-dept-1", "@HOME@/.cys/claude-other"))
rc, out, err = call(tmp, home, reg, "dept-1")
check("D1 레지스트리 우선 exit 0", rc == 0, "rc=%d err=%s" % (rc, err.strip()[:120]))
check("D1 값=레지스트리 account_dir(agents 리터럴 무시)",
      out == os.path.join(home, ".cys", "claude-reg-dept-1"), out)
shutil.rmtree(tmp)

# ── D2: 레지스트리 無 → agents.json 리터럴 복구 + depts.json 역기입(자기치유) ──
tmp, home, reg = sandbox(
    {"dept-2": {"socket": "s", "pack_dir": "p", "role": "dept-master"}},
    agents={"dept-2": {"claude": {"cmd": "claude",
                                  "env": {"CLAUDE_CONFIG_DIR": "@HOME@/.cys/claude-cat-dept-2"}}}},
    mkdirs=("@HOME@/.cys/claude-cat-dept-2",))
rc, out, err = call(tmp, home, reg, "dept-2")
want2 = os.path.join(home, ".cys", "claude-cat-dept-2")
check("D2 agents 리터럴 복구 exit 0", rc == 0, "rc=%d err=%s" % (rc, err.strip()[:120]))
check("D2 값=agents 리터럴", out == want2, out)
check("D2 역기입(자기치유) — depts.json account_dir 기록",
      json.load(open(reg, encoding="utf-8"))["depts"]["dept-2"].get("account_dir") == want2)
check("D2 복구 고지 stderr(stdout 오염 0)", "자기치유" in err and "자기치유" not in out)
shutil.rmtree(tmp)

# ── D2b: 레거시 cmd 인라인 리터럴(seed_agents_account 하위호환 경로)도 동일 복구 ──
tmp, home, reg = sandbox(
    {"dept-2": {"socket": "s", "pack_dir": "p", "role": "dept-master"}},
    agents={"dept-2": {"claude": {"cmd": 'CLAUDE_CONFIG_DIR="@HOME@/.cys/claude-legacy" claude'}}},
    mkdirs=("@HOME@/.cys/claude-legacy",))
rc, out, err = call(tmp, home, reg, "dept-2")
check("D2b 레거시 cmd 리터럴 복구", rc == 0 and out == os.path.join(home, ".cys", "claude-legacy"),
      "rc=%d out=%s" % (rc, out))
shutil.rmtree(tmp)

# ── D3: 레지스트리 有 + dir 부재 → fail-loud exit 5(조용한 무주입 금지) ──
tmp, home, reg = sandbox(
    {"dept-3": {"socket": "s", "pack_dir": "p", "role": "dept-master",
                "account_dir": "@HOME@/.cys/GONE"}})
rc, out, err = call(tmp, home, reg, "dept-3")
check("D3 fail-loud exit 5", rc == 5, "rc=%d" % rc)
check("D3 stdout 무출력(비격리 경로 미산출)", out == "", out)
check("D3 사유 stderr 명시", "실재하지 않음" in err and "fail-loud" in err, err.strip()[:160])
shutil.rmtree(tmp)

# ── D4: 양쪽 無 → exit 1·무출력(신규·온보딩 = 종전 spawn 경로 무변경) ──
tmp, home, reg = sandbox({"dept-4": {"socket": "s", "pack_dir": "p", "role": "dept-master"}})
rc, out, err = call(tmp, home, reg, "dept-4")
check("D4 출처 없음 exit 1", rc == 1, "rc=%d" % rc)
check("D4 무출력", out == "", out)
# ★기각 규칙 핀(SIM-1): resolve_default_base 재산출로 값을 지어내면 안 된다 —
#   base(~/.cys/claude-default)가 실재해도 dept-4의 계정으로 파생하지 않는다.
os.makedirs(os.path.join(home, ".cys", "claude-default"), exist_ok=True)
os.makedirs(os.path.join(home, ".cys", "claude-default-dept-4"), exist_ok=True)
rc2, out2, _ = call(tmp, home, reg, "dept-4")
check("D4b resolve_default_base 재산출 기각(SIM-1) — base 실재해도 무출력",
      rc2 == 1 and out2 == "", "rc=%d out=%s" % (rc2, out2))
shutil.rmtree(tmp)

# ── D5: agents 리터럴이 미전개 `$` 템플릿 → 미채택(base 템플릿 오채택 방어) ──
tmp, home, reg = sandbox(
    {"dept-5": {"socket": "s", "pack_dir": "p", "role": "dept-master"}},
    agents={"dept-5": {"claude": {"cmd": "claude",
                                  "env": {"CLAUDE_CONFIG_DIR": "${CYS_ACCOUNT_DIR:-$HOME/.claude}"}}}})
rc, out, err = call(tmp, home, reg, "dept-5")
check("D5 `$` 템플릿 미채택(exit 1·무출력)", rc == 1 and out == "", "rc=%d out=%s" % (rc, out))
check("D5 역기입 없음(오염 0)",
      json.load(open(reg, encoding="utf-8"))["depts"]["dept-5"].get("account_dir") is None)
shutil.rmtree(tmp)

# ── D6: agents 리터럴 dir 미실재 → 미채택 + WARN(무주입) ──
tmp, home, reg = sandbox(
    {"dept-6": {"socket": "s", "pack_dir": "p", "role": "dept-master"}},
    agents={"dept-6": {"claude": {"cmd": "claude",
                                  "env": {"CLAUDE_CONFIG_DIR": "@HOME@/.cys/claude-missing"}}}})
rc, out, err = call(tmp, home, reg, "dept-6")
check("D6 리터럴 dir 미실재 → 미채택", rc == 1 and out == "", "rc=%d out=%s" % (rc, out))
check("D6 WARN 가시화", "디렉터리 미실재" in err, err.strip()[:160])
shutil.rmtree(tmp)

# ── D7: 손상 depts.json → crash 없이 강등(exit 1) ──
tmp, home, reg = sandbox({"dept-7": {"socket": "s", "pack_dir": "p", "role": "dept-master"}})
with open(reg, "w", encoding="utf-8") as f:
    f.write("{ this is not json")
rc, out, err = call(tmp, home, reg, "dept-7")
check("D7 손상 레지스트리 — crash 0·무출력 강등", rc == 1 and out == "", "rc=%d out=%s" % (rc, out))
shutil.rmtree(tmp)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
