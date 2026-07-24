#!/usr/bin/env python3
"""test_ceo_pending_gate.py — cys-dept CEO 부트 게이트·PENDING 상태 기계 핀 (WP-2).

가짜 HOME(팩 directives·registry)+스텁 cys($HOME/.local/bin — cys-dept PATH prepend 1순위)로
실 데몬 무접촉 검증:
  1) 마커 無 + 승격 시도 → PENDING·디렉티브 무교체(사고 R2 봉쇄) + truthful exit 5
     (af6fcb6 D-2 post-verify: 미승격인데 exit 0이면 GUI가 "승격 완료"로 오보 — 부서 흐름
     불파괴는 내부 ceo_promote의 return 0이 담당, 지명 서브커맨드는 진실 보고)
  2) 마커 생성 후 promote-if-pending(대기형) → 동의 게이트 경유 승격·PENDING 해소
  3) --request-only → 무변조·알림만·exit 0 (부트 ⑦ 비대기 계약)
  4) 단일소유 가드: master 세션 대기형=exit 7 / --request-only=허용
  5) 이미 승격 상태에서 재호출 → stale PENDING 청소·멱등
  6) 조건 미충족(부서 0) → no-op
(ceo_demote의 PENDING 청소는 down 경로 통합시험 영역 — 본 파일은 승격 축만.)

★W1 invert-merge 갱신(2026-07-24 · 설계 DD-1 파급): 승격은 이제 replace 가 아니라 compose(invert-merge) —
  MASTER_DIRECTIVE = [표준 base 전문] + CEO-OVERLAY sentinel 구간. 따라서 승격 후 md 는 base+overlay 이지
  overlay 단독이 아니다 → 2b·4c·5c 를 "base 보존 + overlay sentinel" 계약으로 갱신(약화 아님·오히려 base
  보존을 추가 검증). 2e 는 오너 2026-07-17 자동승격 정책이 --wait 동의 게이트를 폐지한 상태를 반영해 갱신
  (이 assertion 은 W1 착수 전 이미 baseline RED 였음 — W1 과 무관한 선존 drift 정합화). PENDING 게이트·
  단일소유 가드·feed 프롬프트·exit 코드 계약은 전부 불변.
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


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def setup(tmp, ndepts=1):
    home = os.path.join(tmp, "home")
    pack = os.path.join(home, ".cys", "pack", "directives")
    bindir = os.path.join(home, ".local", "bin")  # cys-dept PATH prepend 1순위
    os.makedirs(pack, exist_ok=True)
    os.makedirs(bindir, exist_ok=True)
    with open(os.path.join(pack, "MASTER_DIRECTIVE.md"), "w", encoding="utf-8") as f:
        f.write("STANDARD-MASTER\n")
    with open(os.path.join(pack, "CEO_TEMPLATE.md"), "w", encoding="utf-8") as f:
        f.write("CEO-TEMPLATE\n")
    reg = os.path.join(home, ".cys", "depts.json")
    with open(reg, "w", encoding="utf-8") as f:
        json.dump({"depts": {("d%d" % i): {} for i in range(ndepts)}}, f)
    # 스텁 cys: feed push=승인(exit 0)·status=실패(reinject skip 경로)·전 호출 기록
    stub = os.path.join(bindir, "cys")
    with open(stub, "w", encoding="utf-8", newline="\n") as f:
        f.write("#!/bin/sh\necho \"cys $@\" >> \"%s/calls.log\"\n"
                "case \"$1\" in status) exit 1;; esac\nexit 0\n" % tmp)
    os.chmod(stub, 0o755)
    env = dict(os.environ)
    env.update({"HOME": home, "CYS_DEPTS_JSON": reg,
                "PATH": bindir + os.pathsep + env.get("PATH", "")})
    for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR"):
        env.pop(k, None)
    return env, home


def run(env, *args, role=None):
    e = dict(env)
    if role:
        e["CYS_ROLE"] = role
    r = subprocess.run(["bash", DEPT] + list(args), capture_output=True, text=True,
                       encoding="utf-8", env=e, timeout=60)
    return r.returncode, r.stdout + r.stderr


def paths(home):
    d = os.path.join(home, ".cys", "pack", "directives", "MASTER_DIRECTIVE.md")
    return (d, d + ".pre-ceo",
            os.path.join(home, ".cys", "state", "ceo-pending"),
            os.path.join(home, ".cys", ".master-bootstrapped"))


def md(home):
    return open(paths(home)[0], encoding="utf-8").read()


# ── 1. 마커 無 → PENDING·무교체 (실사고 R2 봉쇄) ──
tmp = tempfile.mkdtemp(prefix="ceo-t1-")
env, home = setup(tmp)
code, out = run(env, "promote-ceo")
mdp, pre, pend, marker = paths(home)
check("1a 승격 시도 truthful exit 5(미승격 오보 차단·D-2 post-verify)", code == 5,
      "exit=%d %s" % (code, out[-150:]))
check("1b PENDING 기록", os.path.exists(pend))
check("1c 디렉티브 무교체", md(home) == "STANDARD-MASTER\n")
check("1d .pre-ceo 미생성", not os.path.exists(pre))
check("1e fail-visible(feed 알림)", "feed push" in open(os.path.join(tmp, "calls.log"), encoding="utf-8").read())

# ── 2. 마커 생성 → promote-if-pending(대기형) → 동의 경유 승격·PENDING 해소 ──
with open(marker, "w", encoding="utf-8") as f:
    json.dump({"orchestra_check": "exit 0"}, f)
code, out = run(env, "promote-if-pending")
check("2a 대기형 exit 0", code == 0, out[-150:])
_m2 = md(home)
check("2b 승격됨(invert-merge·base 보존 + CEO-OVERLAY sentinel)",
      "STANDARD-MASTER" in _m2 and "CEO-TEMPLATE" in _m2 and "CEO-OVERLAY BEGIN" in _m2,
      "md=%r" % _m2[:120])
check("2c .pre-ceo 보존 헌법", os.path.exists(pre) and open(pre, encoding="utf-8").read() == "STANDARD-MASTER\n")
check("2d PENDING 해소", not os.path.exists(pend))
calls = open(os.path.join(tmp, "calls.log"), encoding="utf-8").read()
check("2e 자동승격 정책(오너 2026-07-17 — --wait 동의 게이트 폐지·완료 알림 push)",
      "feed push --wait" not in calls and "feed push" in calls,
      "auto-promote는 --wait 미사용(W1 무관 선존 drift 정합화)")

# ── 5. 이미 승격 + stale PENDING → 재호출이 청소·멱등 ──
with open(pend, "w", encoding="utf-8") as f:
    f.write("stale\n")
code, out = run(env, "promote-ceo")
check("5a 멱등 exit 0", code == 0)
check("5b stale PENDING 청소", not os.path.exists(pend))
_m5 = md(home)
check("5c 디렉티브 불변(compose 멱등·정확히 1 sentinel·base+overlay 잔존)",
      _m5.count("CEO-OVERLAY BEGIN") == 1 and "STANDARD-MASTER" in _m5 and "CEO-TEMPLATE" in _m5)
shutil.rmtree(tmp)

# ── 3. --request-only: 무변조·알림만 (부트 ⑦ 계약) ──
tmp = tempfile.mkdtemp(prefix="ceo-t3-")
env, home = setup(tmp)
mdp, pre, pend, marker = paths(home)
run(env, "promote-ceo")                     # PENDING 상태 만들기(마커 無)
with open(marker, "w", encoding="utf-8") as f:
    f.write("{}")
code, out = run(env, "promote-if-pending", "--request-only", role="master")
check("3a request-only exit 0(master 세션 허용)", code == 0, out[-150:])
check("3b 무변조(디렉티브 표준 유지)", md(home) == "STANDARD-MASTER\n")
check("3c PENDING 유지(해소는 대기형/lifecycle)", os.path.exists(pend))
calls = open(os.path.join(tmp, "calls.log"), encoding="utf-8").read()
check("3d 비대기(--wait 없는 알림)", "CEO 승격 대기" in out or "feed push --title CEO 승격 대기" in calls)

# ── 4. 단일소유 가드: master 대기형=차단 / CSO·role-less=허용 ──
code, out = run(env, "promote-if-pending", role="master")
check("4a master 대기형 exit 7", code == 7, "exit=%d" % code)
check("4b 차단 시 무변조", md(home) == "STANDARD-MASTER\n")
code, out = run(env, "promote-if-pending", role="cso")
_m4 = md(home)
check("4c cso 대기형 허용·승격(compose·base 보존+overlay)",
      code == 0 and "CEO-OVERLAY BEGIN" in _m4 and "STANDARD-MASTER" in _m4)
shutil.rmtree(tmp)

# ── 6. 조건 미충족(부서 0·마커 有) → no-op ──
tmp = tempfile.mkdtemp(prefix="ceo-t6-")
env, home = setup(tmp, ndepts=0)
mdp, pre, pend, marker = paths(home)
with open(marker, "w", encoding="utf-8") as f:
    f.write("{}")
os.makedirs(os.path.dirname(pend), exist_ok=True)
with open(pend, "w", encoding="utf-8") as f:
    f.write("pending\n")
code, out = run(env, "promote-if-pending")
check("6a 부서 0 no-op", code == 0 and "no-op" in out)
check("6b 무교체", md(home) == "STANDARD-MASTER\n")
shutil.rmtree(tmp)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
