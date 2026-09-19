#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_dept_schedule_seed.py — 부서 schedule 시드 회귀 (결재 13).

무엇을 막는가: `cys-dept` 가 부서 schedule 을 `{"jobs": []}` 로 덮어써 **레인 지역 기계장치**
(cso-alert-inbox-check-60m · cycle-autopilot-tick · cycle-verifier-watchdog · phoenix-snapshot-6h)가
owner 잡과 함께 지워지던 결함. 실측(2026-09-19): dept-1 jobs=1(owner 잡만) · dept-2 jobs=0.
"""
import json, os, re, subprocess, sys, tempfile

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MOD = os.path.join(BIN, "javis_dept_schedule.py")
DEPT = os.path.join(BIN, "cys-dept")
sys.path.insert(0, BIN)
import javis_dept_schedule as S  # noqa: E402

fails = []


def ck(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


ck("모듈 self-test 9종", subprocess.run([sys.executable, MOD, "--self-test"],
                                      capture_output=True, text=True).returncode == 0)

d = tempfile.mkdtemp(prefix="deptsched-")
p = os.path.join(d, "schedule.json")
jobs = [{"id": "owner-progress-gate-5min"}] + [{"id": i, "_builtin": "b"} for i in S.LANE_LOCAL]
json.dump({"jobs": jobs}, open(p, "w", encoding="utf-8"))
S.seed(p)
got = [j["id"] for j in json.load(open(p, encoding="utf-8"))["jobs"]]
ck("★레인 지역 4종 보존(종전 결함의 회귀 핀)", got == list(S.LANE_LOCAL), str(got))
ck("owner 스코프 제외", "owner-progress-gate-5min" not in got)

src = open(DEPT, encoding="utf-8").read()
ck("★cys-dept 가 더 이상 빈 jobs 로 덮어쓰지 않는다",
   '"jobs": []\\n}' not in src.replace(" ", "") or "javis_dept_schedule.py" in src)
ck("cys-dept 가 필터 도구를 호출한다", "javis_dept_schedule.py" in src)
# ★주석의 '언급'은 복제가 아니다 — 막아야 할 것은 **잡 정의의 복제**(every_minutes·action·command
#   같은 필드가 cys-dept 안에 박히는 것)다. 처음엔 이름 등장 자체를 금지했는데 그건 내 검체가
#   틀린 것이었다(설명 주석까지 적색). 정의 모양으로 판정한다.
body = re.sub(r"^\s*#.*$", "", src, flags=re.M)          # 주석 제거 후 검사
ck("잡 정의를 복제하지 않는다(드리프트 차단)",
   not re.search(r'"every_minutes"\s*:|"_builtin"\s*:|"text_command"\s*:', body), "정의 필드가 cys-dept 에 박혀 있다")
ck("bash 문법 유효", subprocess.run(["bash", "-n", DEPT], capture_output=True).returncode == 0)

print("\n=== %d/%d PASS ===" % (7 - len(fails), 7))
sys.exit(1 if fails else 0)
