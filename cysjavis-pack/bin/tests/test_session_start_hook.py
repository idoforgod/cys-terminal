#!/usr/bin/env python3
"""test_session_start_hook.py — session-start.sh 3상태 재대조·안내문 계약 핀 (WP-1·핀ⓒ).

가짜 JARVIS_DIR(디렉티브 파일)+PATH 스텁 cys로 hook을 sh 실행:
  ⓐ claim 성공 → 디렉티브 주입(현행)
  ⓑ 명시적 거부(claim_denied) → 디렉티브 대신 self-demote 지시·exit 0
  ⓒ 데몬-불가(스텁이 비0+무패턴/응답없음) → fail-open: 디렉티브 주입+고지 1줄
+ 안내문(role-less)이 javis_bootstrap.py 단일 진입점·exit 7 인계·인용 의무를 담는지
+ worker role은 재대조 미적용(무왕복) 핀.

★(0.14.31 · WP-4) 역할 **자동 복구** 확장:
  ⓓ 무역할 + `cys reclaim-role --auto` 가 역할을 돌려주면 → 그 역할 지침 주입 + 고지
  ⓔ 무역할 + `role=`(무결합) → 종전 안내문(무회귀). 두 왕복(surface-role·reclaim-role)은 실제로 났다
  ⓕ `cys surface-role` exit 2(판정 불가) → **reclaim 왕복 0**(모르는 상태에서 역할을 옮기지 않는다)
  ⓖ 역할명 형식 가드 — 데몬 유래 값이라도 [a-zA-Z0-9_-] 밖이면 채택하지 않는다
"""
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
HOOK = os.path.join(SELF, "..", "..", "hooks", "session-start.sh")
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def setup(tmp, claim_mode, reclaim_mode="none"):
    """claim_mode: ok | denied | dead(비0 무패턴) | silent(무한대기→timeout)

    reclaim_mode(★0.14.31 WP-4): 스텁 `cys` 의 surface-role·reclaim-role 응답 조합.
      none        surface-role 무출력 exit 0 · reclaim-role `role=`   (무결합 — 종전 경로)
      found       surface-role 무출력 exit 0 · reclaim-role `role=cso`(자동 복구 성공)
      undecidable surface-role **exit 2**   · reclaim-role `role=cso`(호출되면 안 된다)
      bogus       surface-role 무출력 exit 0 · reclaim-role 형식 위반 역할명
    """
    pack = os.path.join(tmp, "pack")
    bindir = os.path.join(tmp, "stubbin")
    os.makedirs(os.path.join(pack, "directives"), exist_ok=True)
    os.makedirs(bindir, exist_ok=True)
    for d in ("MASTER", "WORKER", "CSO", "REVIEWER"):
        with open(os.path.join(pack, "directives", "%s_DIRECTIVE.md" % d), "w",
                  encoding="utf-8") as f:
            f.write("DIRECTIVE-BODY-%s\n" % d)
    body = {"ok": "exit 0",
            "denied": "echo 'claim_denied: privileged role held by live surface' >&2; exit 1",
            "dead": "echo 'connect error' >&2; exit 1",
            "silent": "sleep 10"}[claim_mode]
    sr_body, rc_body = {
        "none":        ("exit 0", "echo 'role='; exit 0"),
        "found":       ("exit 0", "echo 'role=cso'; exit 0"),
        "undecidable": ("exit 2", "echo 'role=cso'; exit 0"),
        "bogus":       ("exit 0", "echo 'role=cso; rm -rf /'; exit 0"),
    }[reclaim_mode]
    with open(os.path.join(bindir, "cys"), "w", encoding="utf-8", newline="\n") as f:
        f.write("#!/bin/sh\necho \"cys $@\" >> \"%s/calls.log\"\n"
                "case \"$1\" in\n"
                "  claim-role) %s;;\n"
                "  surface-role) %s;;\n"
                "  reclaim-role) %s;;\n"
                "esac\nexit 0\n" % (tmp, body, sr_body, rc_body))
    os.chmod(os.path.join(bindir, "cys"), 0o755)
    env = dict(os.environ)
    env.update({"CYS_PACK_DIR": pack, "CYS_SURFACE_ID": "3",
                "PATH": bindir + os.pathsep + env.get("PATH", "")})
    env.pop("CYS_ROLE", None)
    return env


def run_hook(env, role=None):
    e = dict(env)
    if role:
        e["CYS_ROLE"] = role
    r = subprocess.run(["sh", HOOK], capture_output=True, text=True, encoding="utf-8",
                       env=e, stdin=subprocess.DEVNULL, timeout=30)
    return r.returncode, r.stdout, r.stderr


# ── 1. role-less 안내문 계약 ──
tmp = tempfile.mkdtemp(prefix="hook-t1-")
env = setup(tmp, "ok")
code, out, _ = run_hook(env)
check("1a 안내문 exit 0", code == 0)
check("1b 단일 진입점 스크립트", "javis_bootstrap.py" in out)
check("1c exit 7 인계 분기", "exit 7" in out and "인계" in out)
check("1d 인용 의무 명문", "인용" in out)
check("1e 산문 부트 지시 제거", "preflight.py --fix" not in out.replace("javis_preflight", ""))
shutil.rmtree(tmp)

# ── 2. ⓐ master claim 성공 → 디렉티브 주입 ──
tmp = tempfile.mkdtemp(prefix="hook-t2-")
env = setup(tmp, "ok")
code, out, _ = run_hook(env, role="master")
check("2a ⓐ성공: 디렉티브 주입", "DIRECTIVE-BODY-MASTER" in out)
check("2b ⓐ성공: self-demote 없음", "역할 주소 상실" not in out)
# ★R13 부트 브리지: 구 산문 §0만 아는 디렉티브 기계에도(hook=system층 전파) 스크립트 경로 고지
check("2c ★R13 부트 브리지 주입(javis_bootstrap 부재 시 생략)",
      "부트 브리지" not in out)  # 가짜 팩엔 bin/javis_bootstrap.py 없음 → 브리지 미주입(조건부 확인)

shutil.rmtree(tmp)

# ── 2x. ★R13 브리지 존재 케이스: 팩에 javis_bootstrap.py 있으면 master 주입에 브리지 동봉 ──
tmp = tempfile.mkdtemp(prefix="hook-t2x-")
env = setup(tmp, "ok")
_bs = os.path.join(env["CYS_PACK_DIR"], "bin")
os.makedirs(_bs, exist_ok=True)
open(os.path.join(_bs, "javis_bootstrap.py"), "w", encoding="utf-8").write("# stub\n")
code, out, _ = run_hook(env, role="master")
check("2x1 ★R13 브리지 주입", "부트 브리지" in out and "javis_bootstrap.py" in out)
check("2x2 브리지는 worker엔 미주입", "부트 브리지" not in run_hook(env, role="worker")[1])
shutil.rmtree(tmp)

# ── 3. ⓑ 명시적 거부 → self-demote·디렉티브 미주입 ──
tmp = tempfile.mkdtemp(prefix="hook-t3-")
env = setup(tmp, "denied")
code, out, _ = run_hook(env, role="master")
check("3a ⓑ거부: self-demote 지시", "역할 주소 상실" in out and "인계" in out)
check("3b ⓑ거부: 디렉티브 미주입", "DIRECTIVE-BODY-MASTER" not in out)
check("3c ⓑ거부: exit 0(hook 무해)", code == 0)
shutil.rmtree(tmp)

# ── 4. ⓒ 데몬-불가(비0·무패턴) → fail-open: 디렉티브 주입+고지 ──
tmp = tempfile.mkdtemp(prefix="hook-t4-")
env = setup(tmp, "dead")
code, out, _ = run_hook(env, role="master")
check("4a ⓒ불가: 디렉티브 주입(fail-open)", "DIRECTIVE-BODY-MASTER" in out)
check("4b ⓒ불가: 고지 1줄", "역할 재확인 불가" in out)
check("4c ⓒ불가: self-demote 없음", "역할 주소 상실" not in out)
shutil.rmtree(tmp)

# ── 5. ⓒ 무응답(timeout 상한) → fail-open ──
tmp = tempfile.mkdtemp(prefix="hook-t5-")
env = setup(tmp, "silent")
code, out, _ = run_hook(env, role="master")
check("5a ⓒ무응답: 디렉티브 주입(fail-open)", "DIRECTIVE-BODY-MASTER" in out)
shutil.rmtree(tmp)

# ── 6. worker는 재대조 미적용(권한 role 아님 — claim 왕복 0) ──
tmp = tempfile.mkdtemp(prefix="hook-t6-")
env = setup(tmp, "denied")
code, out, _ = run_hook(env, role="worker")
check("6a worker 디렉티브 주입", "DIRECTIVE-BODY-WORKER" in out)
calls = ""
if os.path.exists(os.path.join(tmp, "calls.log")):
    calls = open(os.path.join(tmp, "calls.log"), encoding="utf-8").read()
check("6b worker claim 왕복 0", "claim-role" not in calls)
shutil.rmtree(tmp)


# ══════════════════════════════════════════════════════════════════════════════
# ★(0.14.31 · WP-4) 무역할 좌석의 역할 자동 복구
# ══════════════════════════════════════════════════════════════════════════════

def calls_of(tmp):
    p = os.path.join(tmp, "calls.log")
    return open(p, encoding="utf-8").read() if os.path.exists(p) else ""


# ── 7. ⓓ 자동 복구 성공 → 그 역할 지침 주입 + 고지(안내문 대신) ──
tmp = tempfile.mkdtemp(prefix="hook-t7-")
env = setup(tmp, "ok", reclaim_mode="found")
code, out, _ = run_hook(env)
check("7a ⓓ복구: 역할 지침 주입", "DIRECTIVE-BODY-CSO" in out)
check("7b ⓓ복구: 고지 1줄", "역할 자동 복구" in out and "cso" in out)
check("7c ⓓ복구: 무역할 안내문 대신 지침", "javis_bootstrap.py" not in out)
check("7d ⓓ복구: exit 0", code == 0)
_c = calls_of(tmp)
check("7e ⓓ복구: 인자 계약(--auto --config --cwd)",
      "reclaim-role --auto --config" in _c and "--cwd" in _c, _c.strip().replace("\n", " | "))
shutil.rmtree(tmp)

# ── 8. ⓔ 무결합(role=) → 종전 안내문 그대로(무회귀) + 두 왕복은 실제로 났다 ──
tmp = tempfile.mkdtemp(prefix="hook-t8-")
env = setup(tmp, "ok", reclaim_mode="none")
code, out, _ = run_hook(env)
check("8a ⓔ무결합: 종전 안내문 유지", "javis_bootstrap.py" in out and code == 0)
check("8b ⓔ무결합: 역할 지침 미주입", "DIRECTIVE-BODY-" not in out)
check("8c ⓔ무결합: 고지 없음", "역할 자동 복구" not in out)
_c = calls_of(tmp)
check("8d ⓔ무결합: surface-role·reclaim-role 왕복 각 1",
      "surface-role" in _c and "reclaim-role" in _c)
shutil.rmtree(tmp)

# ── 9. ⓕ 판정 불가(surface-role exit 2) → reclaim 왕복 0 ──
tmp = tempfile.mkdtemp(prefix="hook-t9-")
env = setup(tmp, "ok", reclaim_mode="undecidable")
code, out, _ = run_hook(env)
_c = calls_of(tmp)
check("9a ⓕ판정불가: reclaim 왕복 0", "reclaim-role" not in _c, _c.strip().replace("\n", " | "))
check("9b ⓕ판정불가: 고지 1줄", "역할 판정 불가" in out)
check("9c ⓕ판정불가: 역할 지침 미주입(안내문 유지)",
      "DIRECTIVE-BODY-" not in out and "javis_bootstrap.py" in out)
check("9d ⓕ판정불가: exit 0", code == 0)
shutil.rmtree(tmp)

# ── 10. ⓖ 형식 가드 — 역할명이 [a-zA-Z0-9_-] 밖이면 채택하지 않는다 ──
tmp = tempfile.mkdtemp(prefix="hook-t10-")
env = setup(tmp, "ok", reclaim_mode="bogus")
code, out, _ = run_hook(env)
check("10a ⓖ형식가드: 미채택(안내문 유지)", "javis_bootstrap.py" in out)
check("10b ⓖ형식가드: 지침 미주입", "DIRECTIVE-BODY-" not in out)
check("10c ⓖ형식가드: 고지 없음", "역할 자동 복구" not in out)
shutil.rmtree(tmp)

# ── 11. 역할이 이미 있으면 자동 복구 경로에 들어가지 않는다(왕복 0) ──
tmp = tempfile.mkdtemp(prefix="hook-t11-")
env = setup(tmp, "ok", reclaim_mode="found")
code, out, _ = run_hook(env, role="worker")
_c = calls_of(tmp)
check("11a 역할 보유 시 reclaim 왕복 0", "reclaim-role" not in _c and "surface-role" not in _c)
check("11b 역할 보유 시 종전 지침 주입", "DIRECTIVE-BODY-WORKER" in out)
shutil.rmtree(tmp)

# ── 12. Windows 함정 회귀 — 재대조가 검증되지 않은 `timeout` 을 직접 부르지 않는다 ──
#   PortableGit 의 System32 timeout.exe 는 인자를 받으면 즉시 rc=1 이라, 재대조가 실행조차
#   되지 않은 채 '데몬 미응답'으로 접힌다. 판정은 소스에 있고 행위로는 mac 에서 못 재므로 소스 핀.
_hook_src = open(HOOK, encoding="utf-8").read()
_code_lines = [l for l in _hook_src.splitlines() if not l.lstrip().startswith("#")]
_code = "\n".join(_code_lines)
check("12a 훅이 bare `timeout N cys` 를 쓰지 않는다(cys_timeout_run 경유)",
      "timeout 2 cys" not in _code and "command -v timeout" not in _code)
check("12b 자동 복구 두 호출 모두 cys_timeout_run 경유",
      _code.count("cys_timeout_run") >= 3)
check("12c 훅에 ps·flock 없음(Windows 안전)",
      " ps " not in _code and "flock" not in _code)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
