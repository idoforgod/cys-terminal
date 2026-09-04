#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_mission_boot_command_filter.py — 임무 게이트 층0-c(기동 명령문) 회귀 핀 (0.14.30 A4 #2 · PREP #2).

## 재현 대상 (2026-09-03 21:19:23 실사고)
임무 대장이 **기동 명령문**으로 덮였다 —

    {"mission": "cys launch-agent --role master --agent claude",
     "source": "prompt", "reason": "잔여문 45자 — 오너 임무로 인정"}

이것은 노드를 띄우는 셸 한 줄이지 "이 세션에서 무엇을 하라"는 임무가 아니다. 그런데 기존 세 층
어디에도 걸리지 않는다: 층1(배달 원장)은 데몬 주입이 아니라 원장에 없고, 층2(라벨)는 `[` 로
시작하지 않으며, 층0(harness)은 XML 마커가 없어 잔여문 45자가 살아남는다. 결과는 **자율 착수
게이트 개방**(gate exit 0) — 임무를 지정받지 않은 세션이 자기 기동 명령을 근거로 일을 시작한다.

## 이 파일이 못박는 것 (밀폐: CYS_STATE_DIR 격리 · 실 데몬·실 사용자 대장 무접촉)
  1) 실사고 문자열 → mission=null · source=boot_command · reason 실재 · 게이트 무개방
  2) 진행 중 오너 임무를 기동 명령문이 **덮지 않는다**(반대 방향 사고 차단)
  3) 통과 corpus — 명령을 **인용한** 한글 지시·평문 임무·영어 산문·열거 밖 서브커맨드·다중 줄
  4) 접힘 corpus — cys 기동 계열 5종 + 에이전트 CLI 기동 2종
  5) 층0(harness)·층2(라벨) 판정이 이 축 신설로 변하지 않는다(독립성)
  6) ★음성 대조 — 같은 판정 함수에 실사고 문자열의 **한글 1자 추가본**을 넣으면 통과한다
     (규칙이 '전체 일치 + 한글 0자' 라는 사실의 실증 · 규칙이 넓어지면 이 핀이 깨진다)

## 양방향 의무 (층0 과 동일 · 2026-08-22 master 지시의 계승)
접힘 표만 늘리면 "전부 접는 구현"이, 통과 표만 늘리면 "아무것도 안 접는 구현"이 만점을 받는다.
**3·4 두 표가 동시에 green 일 때만** 이 파일의 green 이 증거가 된다.

실행(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_mission_boot_command_filter.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 MISSION-BOOT-CMD-OK.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
MISSION = os.path.join(BIN, "javis_mission.py")
PY = sys.executable or "python3"
sys.path.insert(0, BIN)

import javis_mission as jm  # noqa: E402 — 순수 판정 함수 직접 핀

INCIDENT = "cys launch-agent --role master --agent claude"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def make_env(tmp):
    env = dict(os.environ)
    env["CYS_STATE_DIR"] = os.path.join(tmp, "state")   # 밀폐 — 실 사용자 대장 무접촉
    env["CYS_SURFACE_ID"] = "7"
    env.pop("CYS_MISSION", None)                        # env 임무는 게이트를 열어 버린다
    env.pop("AITERM_SURFACE_ID", None)
    return env


def record(env, prompt):
    r = subprocess.run([PY, MISSION, "record"],
                       input=json.dumps({"prompt": prompt}, ensure_ascii=False),
                       capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
    return r.returncode, r.stderr


def ledger(env):
    r = subprocess.run([PY, MISSION, "path"], capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=30)
    p = (r.stdout or "").strip()
    if not p or not os.path.isfile(p):
        return None, p
    with open(p, encoding="utf-8") as f:
        return json.load(f), p


def status(env):
    return subprocess.run([PY, MISSION, "status"], capture_output=True, text=True,
                          encoding="utf-8", env=env, timeout=30).returncode


# ── 1. 실사고 문자열 e2e ────────────────────────────────────────────────────────
tmp = tempfile.mkdtemp(prefix="mission-bc1-")
env = make_env(tmp)
record(env, INCIDENT)
rec, path = ledger(env)
check("1a 대장 생성(판정 근거를 남긴다)", isinstance(rec, dict), "path=%s" % path)
check("1b ★mission=null(기동 명령문이 임무가 되지 않는다)",
      isinstance(rec, dict) and rec.get("mission") is None, "mission=%r" % (rec or {}).get("mission"))
check("1c source=boot_command(판정 근거가 대장에 남는다)",
      (rec or {}).get("source") == "boot_command", "source=%r" % (rec or {}).get("source"))
check("1d reason 비어 있지 않음 + 축 이름 포함",
      "기동 명령문" in ((rec or {}).get("reason") or ""), repr((rec or {}).get("reason")))
check("1e ★게이트 무개방(자율 착수 금지)", status(env) != 0)
shutil.rmtree(tmp, ignore_errors=True)

# ── 2. 진행 중 오너 임무를 덮지 않는다(반대 방향 사고 차단) ─────────────────────
tmp = tempfile.mkdtemp(prefix="mission-bc2-")
env = make_env(tmp)
record(env, "홈페이지 릴리스 검증 착수해줘")
rec_before, _ = ledger(env)
record(env, INCIDENT)
rec_after, _ = ledger(env)
check("2a 오너 임무가 먼저 기록됐다",
      (rec_before or {}).get("mission") == "홈페이지 릴리스 검증 착수해줘",
      repr((rec_before or {}).get("mission")))
check("2b ★기동 명령문이 진행 중 오너 임무를 덮지 않는다",
      (rec_after or {}).get("mission") == "홈페이지 릴리스 검증 착수해줘",
      repr((rec_after or {}).get("mission")))
shutil.rmtree(tmp, ignore_errors=True)

# ── 3. 통과 corpus(과잉 차단 0) — 순수 함수 + e2e 1종 ──────────────────────────
PASS_CORPUS = (
    (INCIDENT + " 가 왜 임무로 잡혔는지 조사해", "명령을 인용한 한글 지시"),
    ("cys boot 실행하고 결과 보고해", "명령 + 한글 지시"),
    ("부서 문서 정리 착수해줘", "평범한 오너 임무"),
    ("run cys boot and report the result", "영어 산문(전체 일치 아님)"),
    ("cys send --to master 'hi'", "열거 밖 서브커맨드(조회·발신 계열)"),
    ("cys status --json", "관측 명령(열거 밖)"),
    (INCIDENT + "\n그리고 상태를 알려줘", "다중 줄"),
)
for p, why in PASS_CORPUS:
    check("3 통과: %s" % why, not jm.boot_command_origin(p)[0], repr(p[:60]))

tmp = tempfile.mkdtemp(prefix="mission-bc3-")
env = make_env(tmp)
record(env, "cys boot 실행하고 결과 보고해")
rec, _ = ledger(env)
check("3e2e 통과 corpus 1종은 e2e 로도 임무 기록",
      (rec or {}).get("mission") == "cys boot 실행하고 결과 보고해" and
      (rec or {}).get("source") == "prompt", repr(rec and {k: rec.get(k) for k in ("mission", "source")}))
shutil.rmtree(tmp, ignore_errors=True)

# ── 4. 접힘 corpus ─────────────────────────────────────────────────────────────
FOLD_CORPUS = (
    (INCIDENT, "★실사고 문자열"),
    ("cys boot", "인자 없는 기동"),
    ("cys claim-role worker", "좌석 선점"),
    ("cys new-surface --role worker --cwd /tmp", "좌석 생성"),
    ("cys node-recover --role cso", "노드 복구"),
    ("claude --dangerously-skip-permissions", "에이전트 CLI 기동"),
    ("codex --dangerously-bypass-approvals-and-sandbox", "타 에이전트 CLI 기동"),
)
for p, why in FOLD_CORPUS:
    check("4 접힘: %s" % why, jm.boot_command_origin(p)[0], repr(p[:60]))

# ── 5. 층 독립성 — 이 축이 층0/층2 판정을 바꾸지 않는다 ────────────────────────
check("5a 층0(harness)은 기동 명령문을 접지 않는다(축 분리)",
      not jm.harness_origin(INCIDENT)[0])
check("5b 층0 실측 알림은 여전히 층0 이 접는다(회귀 0)",
      jm.harness_origin("<task-notification><status>completed</status></task-notification>")[0])
check("5c 라벨 push 는 층0-c 대상이 아니다(층2 소관)",
      not jm.boot_command_origin("[보고] 완료했습니다")[0])

# ── 6. ★음성 대조 — 규칙이 '전체 일치 + 한글 0자' 임을 실증 ────────────────────
check("6a 한글 1자만 붙여도 통과(규칙이 넓어지면 이 핀이 깨진다)",
      not jm.boot_command_origin(INCIDENT + " 해줘")[0])
check("6b 앞에 산문 한 단어만 붙어도 통과(전체 일치 규칙)",
      not jm.boot_command_origin("please " + INCIDENT)[0])
check("6c 상한 초과(400자+)는 이 축 대상 아님",
      not jm.boot_command_origin("cys boot " + "-x" * 300)[0])

print("\n=== %d/%d PASS ===" % (26 - len(fails), 26))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("MISSION-BOOT-CMD-OK")
