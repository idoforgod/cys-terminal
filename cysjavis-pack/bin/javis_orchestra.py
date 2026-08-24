#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_orchestra — LLM 오케스트레이션의 결정론 도구 (절대지침 4차: LLM orchestrating 앵커).

master가 (a) "4개 노드 다 떴나"를 눈대중 판단, (b) 리뷰 프롬프트에 제약을 빠뜨림,
(c) 라운드 번호·완료조건을 머리로 셈 — 이 세 가지는 결정론으로 환원 가능하다. 이 도구가
그 사실을 산출한다(LLM 자연어 재추론 금지 — 출력만이 사실).

서브커맨드:
  check [--json]                4종 의무 노드(cso·worker·reviewer-gemini·reviewer-codex)
                                생존을 cys status로 판정.
                                --json: 완주 2축(ready ∧ awake)을 기계 판독 payload 1줄로
                                노출(U-25 1단 — **관측 전용**: exit 계약도 사람용 출력도
                                무변경). `awake`·`awake_pending` 이 null 이면 **미측정**이지
                                '각성 없음'이 아니다. 롤백은 `CYS_BOOT_GATES=0`(마스터) 또는
                                `CYS_AWAKE_AXIS=0`(축) — 어느 하나로 종전 1축 복귀.
                                exit 0=의무 역할 전원 생존 / 1=**노드 미기동**(실측 판정 —
                                status는 받았고 그중 부재 역할이 있다) /
                                2=**판정 불가**(cys 미설치·데몬 소실·status --json 비0/파손 —
                                cys_status()가 None). ★2와 1은 다른 사실이다: 2는 노드에 대해
                                아무것도 말하지 않는다(데몬 소실을 '노드 미기동'으로 오귀속하면
                                처방이 뒤집힌다 — 2는 `cys ping`·데몬 기동, 1은 `cys boot`).
                                소비자(javis_bootstrap ⑤ 등)는 두 갈래를 분기해야 한다.
  review-prompt --task T --scope S [--reviewer gemini|codex] [--round N] [--success X]
                                REVIEWER_DIRECTIVE §2 제약 + 형식 + 회신 채널을 항상 포함한
                                리뷰 의뢰 프롬프트를 출력(제약 누락 구조 차단). --success는
                                구현 위임과 동일한 평가 기준을 리뷰어에게도 투입(N6 영상 양방향 —
                                "구현할 때도 먹이고 리뷰할 때도 똑같이"). 생략 시 출력 바이트 동일.
  task-prompt  --task T --scope S [--success C] [--to ROLE] [--dont D]
                                위임 티켓 생성(절대지침 5차 work management 앵커):
                                ①위임 직전 대상 노드 생존을 결정론 확인(미기동=티켓 미출력 —
                                "워커 정상 작동 확인 후 작업 지시") ②WORKER §3
                                절대 강조 4규칙(품질·할루시네이션 방지·의도 합의·요약 금지)을
                                항상 주입 — 추출분이 마커 불완전하면 하드 폴백으로 강등·경고
                                (약화 전파·강조 누락 구조 차단). ③--dont 지정 시 무접촉
                                (절대 수정·삭제·리팩터 금지) 음의 경계를 주입(외과적 변경
                                4대 행동지침③ · 생략 시 출력 바이트 동일).
                                exit: 0=티켓 출력(stdout은 티켓만, 경고는 stderr) /
                                1=대상 미기동 / 2=확인 불가(데몬 다운·역할명 위반).
  phase-plan   --task T --phases "p1;p2;p3" --scope S [--success X] [--to ROLE] [--dont D]
                                Task를 세미콜론 분리 Phase로 분해해 각 Phase의 자기완결 티켓
                                (P1/P2/… · build_task_ticket 재사용으로 절대 강조 4규칙 포함)을
                                출력하고 round/PHASE-<task>.json 인덱스(상태 pending) 기록.
                                각 Phase는 독립 세션이 "이것만 보고도" 완수하게 자기완결(영상 N6).
                                코드는 claude -p raw subprocess를 띄우지 않는다 — Workflow
                                pipeline·cys 워커 순차 위임으로 실행(스킬 안내).
                                exit: 0=출력 / 2=phases 비었거나 역할명 위반.
  round-init   --task T                       라운드 장부 생성
  round-log    --task T --round N --evaluator E [--verdict V | --from-cmd CMD | --verdict-json J]
                                라운드 기록 append. --from-cmd는 기계검증 명령을 직접 실행해
                                exit code로 verdict 자동 기록(machine 평가자 규약 — 전사 금지).
                                exit: 0=기록(검증 통과 포함) / 1=기록됨·기계검증 실패
                                (기록 성공≠검증 통과 — 판정의 단일 진실은 gate-status).
  round-status --task T                       현재 라운드·10R 도달·최근 기록값 결정론 판정
  gate-status  --task T [--round N]           자율주행(앵커6 축1) 게이트 4자 수렴 결정론 판정:
                                해당 라운드에 gemini·codex·master·machine 4평가자의 승인
                                (PASS/수렴/approve/ok/green 접두) 기록이 전부 있어야 CONVERGED.
                                exit 0=수렴+임무 있음(다음 단계 자동 착수 가) / 1=미수렴 /
                                2=정직한 SKIP / **4=수렴했으나 오너 임무 미지정(자동 착수 금지)**.
  next-action                   자율주행(앵커6 축3) 다음 액션 결정론 추출: pack/round/
                                SESSION_STATE.md '## 다음 액션' 섹션의 첫 미완 항목을 출력.
                                ★임무 게이트(T1 2026-08-01): 큐에 항목이 있어도 **이 세션에
                                오너 임무 지정이 없으면 착수하지 않는다** — 큐는 master 자신이
                                쓴 파일이라 자기인가가 되기 때문이다(판정=javis_mission.gate).
                                exit 0=항목 있음+임무 지정됨(자율 착수 가)
                                / 1=큐 비음(전 작업 완료 — 정지·오너 보고)
                                / 2=SESSION_STATE 부재(신규 시작 — 오너 지시 대기)
                                / 3=항목 있음·임무 미지정 → 보고하고 멈춘다(자율 착수 금지).

의존성: 파이썬 표준 라이브러리 + PATH의 cys(check만 필요).
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
#   ._pth 는 표준 경로 계산을 우회해 **스크립트 폴더를 sys.path 에 넣지 않는다**
#   (2026-07-29 Windows 0.14.4 실측: `ModuleNotFoundError: No module named 'javis_scrub'`).
#   unix/mac 은 스크립트 폴더가 이미 sys.path[0] 이라 이 블록은 무동작(멱등).
#   ★append 인 이유는 MAJ#1 과 동일 — **발견이 목적이지 기존 항목의 precedence 를 강등하지 않는다**
#   (bin/ 을 stdlib 앞에 놓지 않아 미래의 이름충돌 shadowing 을 원천 차단).
#   선례(append 형태): javis_report.py:33-34.
#   (hooks/inject_gate.py:22 는 insert(0) + CYS_PACK_DIR 기반 경로 — 형태가 다르므로 선례 아님)
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

# ★로케일 비의존 I/O(W-A4 · 선례 javis_bootstrap.py R3/D-IMPL-3 · javis_detect.py G9 · javis_mission.py):
#   ANSI 코드페이지가 UTF-8 이 아닌 Windows(한국어 cp949·서구 cp1252·일본 cp932)에서 stdout 이
#   파이프로 캡처되면(부트 체인 ⑤ 가 check 출력을 캡처하는 경로가 정확히 그것) `✓`/`✗`/`⚠`/`↳`·`—`
#   또는 첫 한글 출력에서 UnicodeEncodeError 로 즉사한다 — 실측: PYTHONIOENCODING=cp949 에서
#   `--note-team-roster` 가 U+2014(—) position 66 크래시. 팀이 실제로 5개 다 떠 있어도 ⑤ 의
#   24회 재시도가 전부 같은 크래시로 죽어 부트 exit 6·완료 마커 미기록(허위 실패)이 됐다.
#   출력 인코딩만 고정한다 — 판정 로직·exit code 무접촉. errors="replace" 라 최악에도 '깨진
#   글자'일 뿐 크래시가 아니다. try/except 는 reconfigure 부재(구형 파이썬·비 TextIOWrapper
#   스트림) 허용 — 형태는 선례와 자구 동일(사본 드리프트 방지).
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

# 4차 앵커4-1: 프로젝트 상주 의무 노드(grok은 선택). 이것은 *표준(Tier-2 이상) 기본 로스터*다.
# ★check 가 실제로 검증하는 것은 effective_required_roles()(=감지 폴백 적용) — REQUIRED_ROLES 는
# 계약·문서용 표준 상수로 보존한다. agy/codex 미감지 시 리뷰어 슬롯은 Claude 대체로 치환된다.
REQUIRED_ROLES = ["cso", "worker", "reviewer-gemini", "reviewer-codex"]
OPTIONAL_ROLES = ["reviewer-grok"]
MAX_ROUNDS = 10  # 마스터 헌장 제9조: 잠근 합격 기준의 미달 항목 0 또는 10R 상한 도달 시 멈춘다

# ★리뷰어 슬롯 + 무구독 폴백(2026-06-14): agy(reviewer-gemini)·codex(reviewer-codex)는
# '기본 전제'일 뿐 절대 전제가 아니다 — 사용자가 다른 임무를 줄 수도, 구독·CLI가 없을 수도 있다.
# master 부트 후 리뷰어를 '호출하는 단계'에서 감지하지 못하면 멈추지 말고 곧바로 Claude 대체
# 리뷰어로 폴백한다. 감지는 LLM 자연어 재추론이 아니라 아래 결정론 함수만이 사실이다(§12).
# 각 슬롯: (네이티브 역할, 네이티브 agent, 대체 역할, 대체 agent=claude).
REVIEWER_SLOTS = [
    ("reviewer-gemini", "gemini", "reviewer-claude-1", "claude"),
    ("reviewer-codex",  "codex",  "reviewer-claude-2", "claude"),
]

# ─────────────────── B1: PLAN 테이블 정책 열 (의무/선택을 편성과 같은 소스에) ───────────────────
# ★재감사 B1: 의무 리뷰어가 지속 고장이면 종전 체인은 **영구 데드엔드**였다(자동 회복 0) —
#   ④ cys boot 가 비0 을 내면 javis_bootstrap 이 exit 4 로 죽고, 리뷰어 1종 고장이 팀 전체
#   기동 실패로 번졌다. 의무/선택 판정이 **편성 테이블 밖**(호출부 산문·주석)에 있었기 때문이다.
#   정책을 편성과 같은 행에 둔다 — 소비자는 산문을 읽지 않고 이 열을 읽는다.
#     Fatal   = 이 역할 기동 실패 = 부트 실패(exit 4). cso·worker(조직의 최소 실행 단위).
#     Degrade = 경고로 강등하고 ④-b·⑤ 를 계속한다. 리뷰어(대체 폴백·익명 peer-review 로 보완 가능).
#   ★{B1,B2} 동시 착륙 필수(하드 제약 3): B1 단독이면 데드엔드가 ④→⑤ 로 이동만 한다
#   (④ 는 통과하는데 ⑤ check 가 네이티브 리뷰어를 계속 요구해 영구 적색).
FAIL_FATAL = "Fatal"
FAIL_DEGRADE = "Degrade"
BOOT_PLAN = [
    ("cso", "claude", FAIL_FATAL),
    ("worker", "claude", FAIL_FATAL),
    ("reviewer-gemini", "gemini", FAIL_DEGRADE),
    ("reviewer-codex", "codex", FAIL_DEGRADE),
    ("reviewer-grok", "grok", FAIL_DEGRADE),
]


def plan_policy(role):
    """role → "Fatal"|"Degrade". PLAN 미등재 role 은 Degrade(보수 — 미지 역할이 부트를 죽이지 않게)."""
    for r, _agent, policy in BOOT_PLAN:
        if r == role:
            return policy
    return FAIL_DEGRADE


def plan_mandatory_roles():
    """Fatal 정책 역할 목록 — `cys boot --json` 의 `mandatory:true` 집합과 기계 대조된다(H-EXIT-4)."""
    return [r for r, _a, p in BOOT_PLAN if p == FAIL_FATAL]


# ─────────────────── 공유 술어 ③: slot_satisfied (B2 — 실충전자 라벨링) ───────────────────
def _slot_for(required):
    """required 리뷰어 역할이 속한 REVIEWER_SLOTS 행 반환(네이티브·대체 어느 이름으로도 조회)."""
    for nrole, nagent, srole, sagent in REVIEWER_SLOTS:
        if required in (nrole, srole):
            return nrole, nagent, srole, sagent
    return None


def slot_satisfied(required, live_roles):
    """★공유 술어 ③ — 의무 역할 `required` 가 라이브 좌석으로 충족되는가 + **누가 실제로 채웠는가**.

    반환 (satisfied: bool, filler: str|None, native: bool|None, why: str).

    ★B2 가 고치는 것: boot-reviewers 의 **2차 폴백**(네이티브 CLI 는 설치됐는데 각성 실패 →
      reviewer-claude-N 로 전환)이 발생하면 좌석은 대체 역할명으로 서고, ⑤check 는 여전히
      네이티브 역할명을 요구해 **영구 적색 + 재선언 불회복**이 됐다. 슬롯은 '네이티브 ∨ 대체'
      로 충족되며, 보고는 **실충전자를 라벨링**한다(정직한 강등 — 은닉 성공 금지).
    ★비-리뷰어(cso·worker)는 슬롯 개념이 없다 — 정확일치+worker 접두(role_matches_requirement)만.
    ★전제(하드 제약 7): G2(session-start role case) 착지 완료. 미착지 상태로 대체 좌석을 GREEN
      인정하면 '지침 없는 리뷰어 GREEN'(B6 동형 허위 성공)이 된다 — W1a 에서 이미 착지했다.
    """
    bn = _boot_node()
    match = (bn.role_matches_requirement if bn is not None
             else (lambda req, cand: req == cand or (req == "worker" and cand.startswith("worker"))))
    # ★결정론(자가치유 보호): live_roles 는 **집합**이라 순회 순서가 비결정적이다. worker 처럼
    #   복수 좌석이 가능한 요건에서 임의의 후보를 집으면, 죽은 worker-3 를 골라 '미기동' 오판을
    #   내고 그 오판이 결손>0 → 불필요한 스폰·재선언 churn 으로 번진다. 정렬로 못박고, 정확일치
    #   후보를 접두 후보보다 앞세운다(요건 이름 그대로의 좌석이 1순위 대표).
    #   ★등급 기반 최선 선택은 호출부(check_verdicts)가 한다 — 이 함수는 순수 이름공간 판정이다.
    natives = sorted(c for c in live_roles if match(required, c))
    if natives:
        cand = required if required in natives else natives[0]
        return True, cand, True, "네이티브 좌석 %s%s" % (
            cand, "" if len(natives) == 1 else " (동족 %d좌석 중 대표)" % len(natives))
    slot = _slot_for(required)
    if slot is None:
        return False, None, None, "부재(슬롯 없는 역할 — 정확일치 요건)"
    nrole, nagent, srole, sagent = slot
    for cand in live_roles:
        if cand == srole:
            return (True, srole, False,
                    "대체 좌석 %s(%s)가 슬롯 충전 — 네이티브 %s(%s) 부재"
                    % (srole, sagent, nrole, nagent))
    return False, None, None, "부재(네이티브 %s·대체 %s 모두 없음)" % (nrole, srole)


def _boot_node():
    """공유 술어 모듈 — 소비 불가 시 None(호출부가 명시 폴백. 조용한 접힘 금지)."""
    try:
        import javis_boot_node as _bn
        return _bn
    except Exception:
        return None


def _agents_json():
    try:
        with open(os.path.join(pack_dir(), "agents.json"), encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}


def reviewer_launch_binary(agent, agents=None):
    """agents.json 의 'cmd' 첫 토큰 = 그 agent 의 기동 바이너리(하드코딩 금지·진실원천)."""
    agents = agents if agents is not None else _agents_json()
    cmd = ((agents.get(agent) or {}).get("cmd") or "").strip()
    if not cmd:
        return None
    return os.path.expanduser(cmd.split()[0])


# 단일 오라클 캐시 — 프로세스 생애 1회 spawn (asked=질문했는가 / agents=결과 or None).
_CYS_AGENT_DETECT = {"asked": False, "agents": None}


def cys_agent_detect(timeout=10):
    """★(W4 · 재감사 §3 CS-1③ · B12) `cys agent-detect --json` = 어댑터 설치 감지의 **단일 오라클**.
    Rust 쪽이 한 곳에서 extract_bin(env-prefix 건너뛰기) + 틸드확장 + 실행권 + (Windows 후보
    순회)를 판정하므로, python 이 같은 규칙을 재발명하다 어긋나던 경로(구: Rust=exists() /
    python=os.access X_OK)를 없앤다.
    반환: {agent: {"installed": bool, ...}} · None = 판정 불가(cys 부재·구버전 cys 로 서브커맨드
    미지원·실행/파싱 실패) → **호출부가 자체 감지로 폴백**한다(감지가 죽어서 부트가 멈추면 안 된다).
    프로세스당 1회만 spawn 하고 결과를 캐시한다(로스터가 agent 마다 물어도 subprocess 1회)."""
    if _CYS_AGENT_DETECT["asked"]:
        return _CYS_AGENT_DETECT["agents"]
    _CYS_AGENT_DETECT["asked"] = True
    got = None
    cys = shutil.which("cys")
    if cys:
        try:
            r = subprocess.run([cys, "agent-detect", "--json"],
                               capture_output=True, timeout=timeout)
            if r.returncode == 0:
                d = json.loads((r.stdout or b"").decode("utf-8", "replace"))
                a = d.get("agents")
                if isinstance(a, dict):
                    got = a
        except Exception:
            got = None
    _CYS_AGENT_DETECT["agents"] = got
    return got


def detect_reviewer(agent, agents=None):
    """★결정론 1차 감지(오너 '가장 중요한 전제') — 그 리뷰어 CLI 가 *호출 가능*한가.
    ★1순위 = cys_agent_detect() (Rust 단일 오라클 · W4 CS-1③). 그 판정이 SOT 다.
    ★폴백(오라클 부재·실패·해당 agent 미수록) = 아래 자체 감지: 바이너리가 절대경로로 실재·
    실행가능(os.access X_OK)하거나 PATH(shutil.which)에서 해석되면 available. 폴백은 제거하지
    않는다 — 구버전 cys·cys 미설치 머신에서도 감지가 답을 내야 한다(하드 삭제 금지).
    ★오라클은 **실디스크 어댑터 정의에 대한 판정**이다. 그래서 주입된 `agents` 가 디스크 본과
    다르면(=합성 fixture 를 넣은 밀폐 self-test) 쓰지 않는다 — 주입을 무시하고 실환경을 보면
    테스트 밀폐가 깨진다. reviewer_roster 처럼 디스크에서 해소한 dict 는 같으므로 소비된다
    (로스터가 오라클을 못 쓰면 이 단일화가 무의미해진다).
    인증·구독 유무는 여기서 판정하지 않는다(미인증은 부트 시 set-status ack 부재로
    boot-reviewers 가 2차 폴백한다). claude 는 시스템 전제. 반환: (available, reason)."""
    if agents is None or agents == _agents_json():
        oracle = cys_agent_detect()
        if isinstance(oracle, dict):
            ent = oracle.get(agent)
            if isinstance(ent, dict) and isinstance(ent.get("installed"), bool):
                return ent["installed"], "cys agent-detect: %s" % (
                    ent.get("reason") or ("installed" if ent["installed"] else "missing"))
    binp = reviewer_launch_binary(agent, agents)
    if not binp:
        return False, "agents.json 에 %s.cmd 없음" % agent
    if os.path.sep in binp:
        ok = os.path.isfile(binp) and os.access(binp, os.X_OK)
        return ok, ("실행가능 %s" % binp if ok else "바이너리 부재/실행불가 %s" % binp)
    resolved = shutil.which(binp)
    return (bool(resolved), ("PATH 발견 %s" % resolved) if resolved else ("PATH 미발견 %s" % binp))


def reviewer_roster(detect=None, agents=None):
    """감지에 따른 *유효* 리뷰어 로스터. 각 항목: {role, agent, native, substituted_for, reason}.
    detect/agents 주입 가능(self-test 밀폐)."""
    detect = detect or detect_reviewer
    agents = agents if agents is not None else _agents_json()
    roster = []
    for nrole, nagent, srole, sagent in REVIEWER_SLOTS:
        ok, why = detect(nagent, agents)
        if ok:
            roster.append({"role": nrole, "agent": nagent, "native": True,
                           "substituted_for": None, "reason": why})
        else:
            roster.append({"role": srole, "agent": sagent, "native": False,
                           "substituted_for": nagent, "reason": why})
    return roster


def effective_required_roles(detect=None, agents=None):
    """check 가 검증할 유효 의무 역할 = cso·worker + 유효 리뷰어 로스터(감지 폴백 적용)."""
    return ["cso", "worker"] + [e["role"] for e in reviewer_roster(detect, agents)]


# ─────────────────── B18: 팀 구성 안내 문구의 단일 파생 소스 (H-DOC-2) ───────────────────
def team_roster_note(required=None):
    """훅 note·문서가 인용할 '완료 = 무엇이 떠야 하나' 한 줄. **하드코딩 금지**.

    ★B18(재감사 P3 · RC6): 훅 note 가 `master·cso·worker·reviewer×2 (5노드)` 를 **리터럴**로
      박아 놓고, 판정 술어(`REQUIRED_ROLES` / `effective_required_roles`)는 그와 무관하게
      진화했다 — 편성이 바뀌면 문서만 거짓이 되는 사본 드리프트(P3-A-120S 의 문서면과 동형).
      숫자·역할명을 **여기서 파생**해 소비처는 인용만 한다.
    ★금지 방향 ②(하드 제약 6): `REQUIRED_ROLES` 에 master 를 넣어 이 숫자를 맞추는 것은
      **금지**다 — check 의 required 집합이 master 를 요구하면 레거시 master(자기 좌석을
      스스로 세지 못하는 구 데몬 조합)에서 부트 전체가 사망한다. master 는 '선언한 자기 자신'
      이므로 required 밖에 있는 것이 정상이고, 안내 문구에서만 `+1` 로 합산한다.
    ★감지 미호출: `REQUIRED_ROLES`(표준 상수)만 읽는다 — 훅 발화 경로의 안내 1줄을 위해
      `cys agent-detect` 서브프로세스를 띄우지 않는다(발화 지연 0). 대체 슬롯 치환 가능성은
      문구로 고지한다(로스터 실체는 ⑤check 가 판정).
    """
    roles = ["master"] + list(REQUIRED_ROLES if required is None else required)
    return ("%s (필수 역할 전원+master — 총 %d노드 · 리뷰어는 미감지 시 Claude 대체 슬롯으로 치환)"
            % ("·".join(roles), len(roles)))


# ★팩 경로 env 키의 우선순위 목록(W14 S19). Rust 정본 `src/pack.rs::PACK_DIR_ENV_KEYS`와
# **같은 목록·같은 순서**여야 한다 — `tests/test_todo_shared_constants.py`가 기계 대조한다.
# 종전에는 이 목록이 3종으로 갈려 있었고, `cys todo-path`가 `AITERM_JARVIS_DIR`를 인식하지
# 못해 레거시 env 환경에서 **생성 위치와 스캔 위치가 갈려 파일이 보고기에 영영 보이지 않았다**.
PACK_DIR_ENV_KEYS = ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR")

def pack_dir():
    """팩 경로. 키 목록·순서는 `PACK_DIR_ENV_KEYS`(Rust `src/pack.rs`와 기계 대조)."""
    for key in PACK_DIR_ENV_KEYS:
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


def _cys_status_timeout_s():
    """cys_status 서브프로세스 상한 — javis_budget leaf `CYS_STATUS_TIMEOUT_S` 파생(W-A4).

    종전 하드코딩 10 은 예산 위반이었다: LEAF_FLOORS 의 냉시작 실측 하한이 12 고, 그 주석이
    이미 'orchestra.cys_status' 를 소비자로 명기하는데 실물만 10 으로 더 작았다(장부↔실물
    사본 드리프트). 데몬 냉시작·프로세스 표 refresh 로 status 가 10~12s 걸리는 창에서 정상
    데몬이 None(판정 불가)으로 접혀 check exit 2 오귀속을 만든다.
    ★짝 사본 배선 완료(W-B1 ④ · W-A4b 후속 · 2026-08-21): javis_boot_node.cys_status 도
    이제 같은 leaf 를 소비한다 — `timeout=budget("CYS_STATUS_TIMEOUT_S", 12)` (그 파일의
    budget() 헬퍼 경유 · import 실패 시 leaf 하한 12 명시 폴백). 두 사본이 '값이 우연히
    같은' 상태에서 '**같은 leaf 를 보는**' 상태로 승격됐다 — leaf 가 12 에서 움직이면
    이제 양쪽이 함께 움직인다(사본 드리프트 소멸 · tests/test_seat_latch_negation.py 가
    배선 사실을 기계 대조). cys_list_rows 등 나머지 cys RPC 하드코딩은 W-B1 범위 밖이다
    (티켓이 지정한 leaf 는 status 하나 — CYS_LIST_TIMEOUT_S 는 leaf 15 로 현행 12 와 값이
    달라, 배선이 곧 동작 변경이라 별도 티켓 소유).
    import 실패(부서 팩 결손·팩 스큐)는 leaf 하한 12 명시 폴백 — 예산 모듈 부재가 새 크래시
    지점이 되면 안 된다(선례: 같은 파일 _boot_node_outer_timeout · javis_bootstrap._budget_leaf).
    """
    try:
        import javis_budget as _b
        return float(_b.leaf("CYS_STATUS_TIMEOUT_S"))
    except Exception:
        return 12.0


def cys_status():
    cys = shutil.which("cys")
    if not cys:
        return None
    try:
        r = subprocess.run([cys, "status", "--json"], capture_output=True,
                           timeout=_cys_status_timeout_s())
        if r.returncode != 0:
            return None
        return json.loads(r.stdout.decode("utf-8", "replace"))
    except Exception:
        return None


# set-status 자기보고 신선도 임계(초). 이 안에 자기보고가 있으면 '살아 일하는 중'으로 본다.
STATUS_FRESH_SECS = 600


def live_roles(status):
    """role → alive(bool). 순수 함수(입력 status 만으로 판정).

    판정: agent_alive OR set-status 자기보고가 신선(age<=STATUS_FRESH_SECS·state 존재).
    ★부트스트랩 FAILURE 3 재발방지(2026-06-13): launch-agent 주입 실패로 agent 메타데이터가
    None 이거나 노드가 node 래퍼로 떠 agent_alive 가 구조적 false-negative 여도, 노드의
    set-status 자기보고(=디렉티브를 읽고 각성한 증거)를 결정론 신호로 인정해 '각성했는데 미기동'
    오판을 차단한다.
    ★단, '프로세스 존재'만으로는 생존 인정하지 않는다(codex R1 적대검증 결함5 반영): 빈 CLI(디렉티브
    미수신)를 READY 로 오인증하면 false-negative 가 false-positive 로 바뀐다. 부트 성공의 계약은
    어디까지나 set-status ack 다. 프로세스 탐침은 stuck pane 회수 판단(javis_boot_node.py --reclaim)
    에만 쓴다."""
    out = {}
    for s in status.get("surfaces", []):
        role = s.get("role")
        if not role or s.get("exited"):
            continue
        if s.get("agent_alive"):
            out[role] = True
            continue
        st = s.get("status") or {}
        age = st.get("age_secs")
        if isinstance(age, (int, float)) and age <= STATUS_FRESH_SECS and st.get("state"):
            out[role] = True
    return out


def _quiet_alive_roles(status, roles):
    """미확정 role 중 '생존추정'(각성이력+프로세스)인 것 → {role: True}.
    ★생존 술어는 javis_boot_node.quiet_but_alive 단일 정의를 공유한다(codex R2 결함2 — cmd_check 와
    reclaim 이 같은 상태를 반대로 해석하던 중복 로직 제거). status 를 surface_ref 에 결박해
    litter/exited row·과거이력 오인을 차단한다."""
    out = {}
    bn = _boot_node()
    if bn is None:
        return out
    for role in roles:
        if bn.quiet_but_alive(status, role):
            out[role] = True
    return out


def live_role_names(status):
    """status → 라이브(미exited) role 이름 집합. slot_satisfied·결손 판정의 공통 입력."""
    return {s.get("role") for s in status.get("surfaces", [])
            if s.get("role") and not s.get("exited")}


# ─────────────────── (U-25) 완주 정의 2축 — ready ∧ awake ───────────────────
# ★근본원인 R1: 지금까지 '완주'는 **프로세스가 존재하는가** 한 축이었다. 좌석이 살아 있고
#   이름공간이 맞으면 READY 로 집계됐고, 그 좌석이 **디렉티브를 읽고 입력을 받을 수 있는가**
#   (각성)는 라벨(생존추정)로만 남았다. 라벨은 exit 에 실리지 않으므로 소비부는 '팀을 세웠다'
#   로 읽고 티켓을 태웠다. 그래서 완주를 두 축으로 가른다:
#     · ready = 좌석이 있고 살아 있다  (= 종전 `satisfied` **그대로**. 값·exit 무변경)
#     · awake = 그 좌석이 **각성 확정** (fresh set-status ack 또는 awakened_at 래치)
#
# ★1단(이 변경)은 **무동작**이다: awake 는 additive 로 **관측만** 되고 exit·사람용 출력·
#   결손 판정(has)은 한 비트도 바뀌지 않는다. 의미 전환(exit 을 ready 축으로 명시 · 마커
#   payload 2축 승격 · GUI Warn)은 2단이며 이 커밋에 없다.
#
# ★★미측정(None) ≠ 미각성(False): 축이 꺼졌거나 구 팩 스큐로 값이 없으면 awake 는 **None**
#   이다. None 을 falsy 로 접어 '각성 안 됨'으로 읽는 소비자는 계약 위반이다 — 그 접힘이
#   정확히 BLOCK-4 형(롤백 스위치가 기저보다 파괴적인 상태를 만드는 것)이다. 축 off 에서
#   `awake_pending` 을 `[]`(빈 목록)이 아니라 `None` 으로 두는 이유도 같다: 빈 목록은
#   '재어 보니 전원 각성'이라는 **거짓 초록**이다(측정 불능은 통과가 아니다).

# 축 노브 — 이 축 하나만 끈다.
AWAKE_AXIS_ENV = "CYS_AWAKE_AXIS"
# 마스터 롤백 스위치 미러 — `javis_boot_node.BOOT_GATES_ENV` · Rust `cys::ENV_BOOT_GATES`.
# ★새 판정 축은 **태어날 때 마스터에 접는다**(BLOCK-3: 축마다 노브를 따로 두면 사고 순간에
#   사람이 조합을 만들지 못한다). 이름 드리프트는 `--self-test` 가 3자 대조로 잡는다.
BOOT_GATES_ENV = "CYS_BOOT_GATES"


def awake_axis_enabled():
    """★(U-25) awake 축 노출 판독(부작용 있음 — env 2회). ①마스터 `CYS_BOOT_GATES=0`
    ②축 `CYS_AWAKE_AXIS=0` 중 **어느 하나**라도 누르면 축이 꺼지고 종전 1축(ready) 판정으로
    즉시 복귀한다(판정 dict 에서 새 키가 아예 사라진다 = '되돌렸다'가 참이 되는 모양).

    극성은 형제 게이트(`javis_boot_node.gate_pending_axis_enabled`)와 같은 **엄격 비교**다 —
    `"false"`·`"off"`·빈 값 같은 느슨한 falsy 는 끄지 않는다(오타 한 글자로 안전장치가 조용히
    꺼지는 사고 방지). 기본이 '켜짐'인 이유: 1단에서 이 축은 **관측 전용**이라 켜져 있어도
    거동이 종전과 같고, 반대로 기본 꺼짐이면 2단 착지 때 켜는 것을 잊는 순간 완주 보고가
    다시 1축으로 접힌다 — 잊어서 위험해지는 방향이 아니라 **잊어도 오늘과 같은** 방향."""
    if os.environ.get(BOOT_GATES_ENV) == "0":
        return False
    return os.environ.get(AWAKE_AXIS_ENV) != "0"


def _awake_grade():
    """'각성 확정' 등급명 — `javis_boot_node.LIVENESS_AWAKE` 단일 정의를 소비한다.

    ★리터럴 금지 이유(무음 실패): 등급명을 문자열로 박아 두면 정본이 개명될 때 비교가
      **조용히 항상 False** 가 돼 전원이 '각성 미확인'으로 집계된다 — 적색도 예외도 없이
      판정만 틀린다. 구 팩 스큐(상수 부재)에서만 리터럴로 폴백한다(값 동일 · 무동작)."""
    bn = _boot_node()
    return getattr(bn, "LIVENESS_AWAKE", "awake_confirmed") if bn is not None \
        else "awake_confirmed"


def verdict_awake_from(grade, satisfied, awake_grade="awake_confirmed"):
    """★awake 축의 **순수 코어** — env·status·모듈 상태 비의존(진리표 대상).

    계약: ready(=satisfied) ∧ 등급이 각성확정 ⟺ awake. `alive_presumed`(agent_alive 단독·
    좌석 점유·quiet_but_alive)와 `unknown`(프로브 실패)은 **각성 미확인**이라 awake 가 아니다.
    ★그러나 그 좌석의 ready 는 **그대로 True** 다 — 강등은 라벨이지 실패가 아니라는 B6 계약을
      이 축이 뒤집으면 래치 배포 이전 기계의 건강한 팀이 전부 적색이 된다(역방향 회귀)."""
    return bool(satisfied) and grade == awake_grade


def verdict_awake(v):
    """판정 dict → awake(bool) | **None(미측정 — 축 off)**. 순수 코어의 dict 어댑터."""
    if not awake_axis_enabled():
        return None
    return verdict_awake_from(v.get("grade"), v.get("satisfied"), _awake_grade())


def verdict_axes(verdicts):
    """★(U-25) 판정 묶음 → **완주 2축 집계**. 부작용은 축 env 판독뿐(판정은 순수).

    반환 dict:
      `axis`           축이 켜져 있는가(False = awake 미측정)
      `ready`          전 의무 역할이 ready 인가 (= 종전 exit 0 조건 · 값 무변경)
      `ready_missing`  ready 미충족 역할(요건 순서 — 종전 `cmd_check` 의 `missing` 과 동일)
      `gated`          그중 첫기동 관문 보류(U-10 제4 등급 — 처방이 '기동'이 아니라 '사람 1회')
      `awake`          전 의무 역할이 각성 확정인가 | **None(미측정)**
      `awake_pending`  ready 이지만 각성 미확정인 역할 | **None(미측정)**

    ★`ready_missing`·`gated` 는 새 축이 아니다: `cmd_check` 가 종전에 **표현 루프 안에서**
      만들던 목록을 표현 이전으로 끌어낸 것뿐이다(같은 규칙을 사람용·기계용 두 벌로 적지
      않기 위해서 — 값·순서 동일 · 사람용 출력 바이트 무변경).
    ★`awake_pending` 이 ready 미충족 역할을 담지 않는 이유: 그쪽은 이미 `ready_missing` 이
      말하는 **다른 사실**(좌석이 없다)이고, 이 목록은 '좌석은 섰는데 아직 못 쓴다'는
      **경고 축**이다(2단에서 GUI 가 이 목록으로 Warn 을 낸다)."""
    bn = _boot_node()
    gated_grade = getattr(bn, "LIVENESS_GATED", "gate_pending") if bn is not None \
        else "gate_pending"
    awake_grade, on = _awake_grade(), awake_axis_enabled()
    ready_missing, gated, pending = [], [], []
    for r, v in verdicts.items():
        if not v.get("satisfied"):
            ready_missing.append(r)
            if v.get("grade") == gated_grade:
                gated.append(r)
            continue
        if not verdict_awake_from(v.get("grade"), True, awake_grade):
            pending.append(r)
    return {"axis": on,
            "ready": not ready_missing,
            "ready_missing": ready_missing,
            "gated": gated,
            "awake": (not ready_missing and not pending) if on else None,
            "awake_pending": pending if on else None}


def check_verdicts(status, detect=None, agents=None):
    """★check 판정의 순수 함수 코어 — (verdicts, roster). 데몬 왕복 0(status 주입).

    ★detect/agents 주입 (2026-08-22): `reviewer_roster` 와 **동일한 밀폐 규약**이다(그 함수는
      이미 "detect/agents 주입 가능(self-test 밀폐)"를 계약으로 갖고 있었는데, 이 함수만 그
      경로를 막고 있었다). 미주입=None 이면 종전과 완전히 같은 실감지 경로다 — 프로덕션 거동 불변.
      ☞ 왜 필요했나: `--self-test` 가 합성 status 로 `reviewer-gemini` 등급을 단언하는데, 로스터는
      **기계에 실제로 설치된 리뷰어 CLI 감지 결과**로 정해진다. agy·codex 가 없는 깨끗한 기계
      (=신규 사용자 대다수)에서는 required 에 reviewer-gemini 가 없어 `KeyError` 트레이스백으로
      self-test 가 죽었다. 밀폐 주입으로 테스트가 환경에 의존하지 않게 한다.

    verdicts: required role → {"satisfied","grade","filler","native","why"}
                              (+ 축 on 이면 "ready","awake" — U-25 1단 additive)
      grade ∈ awake_confirmed | alive_presumed | absent | unknown  (javis_boot_node.node_liveness)

    ★(U-25 1단) 완주 2축 additive — `satisfied` 는 **한 비트도 바뀌지 않는다**. 그것이 곧
      ready 축이고, 새 키 `ready` 는 그 별칭을 이름으로 드러낸 것이며, 새 축은 `awake` 뿐이다.
      축이 꺼지면(`CYS_BOOT_GATES=0` ∨ `CYS_AWAKE_AXIS=0`) 두 키는 **생기지 않는다** =
      종전 dict 모양 그대로다. 소비자는 `v.get("awake")` 로 읽고 **None 을 미측정으로**
      다뤄야 한다(False 로 접으면 롤백이 기저보다 파괴적이 된다 — BLOCK-4 형).

    ★B6 — 1차 통과 기준은 **fresh set-status ack(또는 awakened_at 래치)** 이고, `agent_alive`
      단독은 '생존추정'으로 **강등 라벨링**된다. 강등은 라벨이지 실패가 아니다(exit 는 불변):
      실패로 승격하면 래치 배포 이전 기계의 건강한 팀이 전부 적색이 되는 역방향 회귀다.
    ★B2 — 슬롯 충족은 `slot_satisfied`(네이티브 ∨ 대체) 단일 술어를 소비하고 **실충전자**를 남긴다.
    ★결손 판정(javis_bootstrap)·wakeup zombie 가드·reclaim 이 같은 함수를 소비한다(A1 클래스).
    """
    bn = _boot_node()
    roster = reviewer_roster(detect, agents)
    required = ["cso", "worker"] + [e["role"] for e in roster]
    live = live_role_names(status)
    # 등급 우선순위(높을수록 건강) — 동족 좌석이 여러 개일 때 **가장 건강한 좌석**이 요건을 대표한다.
    # ★왜: worker 가 3개 있고 그중 하나만 죽었을 때 죽은 좌석을 대표로 뽑으면 '미기동' 오판이 나고,
    #   그 오판이 결손>0 → 불필요한 스폰·재선언 churn(자가치유가 아니라 자가교란)으로 번진다.
    # ★(U-10) `gate_pending`(제4 등급) 자리 = **unknown 아래·absent 위**. 정수 재번호 없이 0.5.
    #   · unknown 아래인 이유: unknown 은 '판정 불가'라 충족측으로 접히지만(콜드스타트 fail-open),
    #     gate_pending 은 **확정적으로 미충족**이다. 동족 좌석 중 unknown 이 하나라도 있으면
    #     그쪽이 대표가 돼 종전대로 충족(위경보 0)이고, 관문 좌석이 단독이면 정직하게 미충족이다.
    #   · absent 위인 이유: 관문 보류는 '좌석이 있고 살아 있다'는 **신호가 있는** 상태다.
    #     absent(무신호)를 대표로 뽑으면 스폰을 유도해 살아 있는 pane 위에 중복 좌석이 생긴다.
    _RANK = {"awake_confirmed": 3, "alive_presumed": 2, "unknown": 1,
             "gate_pending": 0.5, "absent": 0}
    # ★(U-25) 축 상태·등급명은 **호출당 1회**만 읽는다 — 같은 호출 안에서 축이 반쯤 켜진
    #   판정 묶음이 나오면(역할마다 env 재판독) 그 자체가 비결정론이다.
    _awake_on, _awake_grade_name = awake_axis_enabled(), _awake_grade()
    verdicts = {}
    for r in required:
        sat, filler, native, why = slot_satisfied(r, live)
        if bn is None:
            grade, greason = (("alive_presumed", "공유 술어 소비 불가 — 좌석 존재로 추정")
                              if sat else ("absent", "공유 술어 소비 불가"))
        elif not sat:
            grade, greason = bn.node_liveness(status, filler or r)
        else:
            # 요건을 충족하는 **전 동족 좌석**을 평가해 최선 등급을 취한다(대표 선택의 결정론화).
            cands = sorted(c for c in live if bn.role_matches_requirement(r, c))
            if filler and filler not in cands:
                cands.append(filler)          # 대체 좌석(슬롯 폴백)도 후보에 포함
            best = max(((bn.node_liveness(status, c), c) for c in cands),
                       key=lambda t: _RANK.get(t[0][0], 0))
            (grade, greason), filler = best[0], best[1]
            # native = 요건 이름공간(정확일치·worker 접두)으로 충족됐는가. 대체 슬롯 좌석
            # (reviewer-claude-N)이 대표가 되면 False → 실충전자 라벨링이 켜진다(B2 정직 강등).
            native = bn.role_matches_requirement(r, filler)
        # 좌석은 있는데 각성/생존 신호가 전부 없으면(absent) 충족이 아니다 — 이름공간과 생존을
        # 함께 본다. 단 unknown(판정불가)은 좌석 존재 시 충족측으로 접는다(fail-open은 여기가
        # 정당하다: check 는 파괴 행위를 하지 않고, unknown 에서 적색을 내면 콜드스타트마다 위경보).
        # ★(U-10) `gate_pending`(제4 등급)도 **충족이 아니다** — 프로세스는 살아 있지만 첫기동
        #   관문에 갇혀 입력을 받지 못하는 좌석이다. 이걸 충족으로 접으면 관문에 갇힌 팀 전체가
        #   "정상 가동 중"으로 집계돼 지금보다 나빠진다(이 등급의 존재 이유).
        #   ★충족 아님 ≠ 죽음: 파괴 게이트(`javis_boot_node.node_alive` → reclaim kill)는 이
        #     등급을 **생존측**으로 읽는다. 두 축을 섞으면 살아 있는 pane 을 죽인다(치명 앵커 ④).
        #   ★구 팩 스큐 안전: 상수 부재(구 javis_boot_node)면 리터럴 폴백 — 그 경우 등급 자체가
        #     생산되지 않으므로 이 항은 무동작이다.
        _gated = getattr(bn, "LIVENESS_GATED", "gate_pending") if bn is not None else "gate_pending"
        satisfied = (bool(sat) and grade not in (bn.LIVENESS_ABSENT, _gated)
                     if bn is not None else bool(sat))
        verdicts[r] = {"satisfied": satisfied, "grade": grade, "filler": filler,
                       "native": native, "why": "%s · %s" % (why, greason)}
        if _awake_on:
            # ready = satisfied 의 **별칭**(값 동일 — 새 판정이 아니다) · awake = 새 축.
            verdicts[r]["ready"] = satisfied
            verdicts[r]["awake"] = verdict_awake_from(grade, satisfied, _awake_grade_name)
    return verdicts, roster


class VerdictDeficit(tuple):
    """`_shared_verdict_deficit` 의 반환 — **길이 2** 튜플 `(has, why)` + `.axes` 부가(U-25 1단).

    ★왜 3원소 튜플이 아닌가(판정 완화가 아니라 **호환 확장**이다): 이 반환을 그대로 2원소로
      푸는 소비자가 이 파일 **밖에** 있다 —
        · `javis_bootstrap._shared_verdict_deficit`(위임 래퍼 — 반환을 그대로 통과시킨다)
        · `javis_bootstrap.py` `_team_has_deficit` · 같은 파일 `--self-test` 2곳
        · `tests/test_seat_latch_negation.py` 3곳(정본을 **직접** 2원소로 푼다)
        · `tests/run_bootstrap_health.py` H-PRED-1 · H-SEAT-4AXIS
      길이 3 으로 늘리면 `has, why = f(...)` 가 전부 `ValueError` 로 죽는다. 그건 부트 ④ 의
      결손 산출이 죽는다는 뜻이고, 결손 산출이 죽으면 `cys boot` 호출 여부를 판정할 수 없다 —
      이 저장소가 실제로 사고를 낸 자리다. 그래서 **길이·동등성·인덱싱은 종전 그대로 두고**
      (`f(...) == (True, "…")` 도 여전히 참) 새 축은 속성으로만 얹는다.
      하드 3원소 승격은 위 소비자들을 **같은 커밋에서 함께 옮길 때**(2단)만 가능하다 —
      핀을 지우는 것이 아니라 이사시키는 조건이다.
    ★구 팩 스큐: 폴백(`javis_bootstrap._shared_verdict_deficit_fallback`)은 평범한 2원소
      튜플을 낸다. 그래서 소비자는 `getattr(r, "axes", None)` 로 읽어야 하고, 그 `None` 은
      **미측정**이지 '각성 없음'이 아니다."""

    # ★`__slots__` 를 쓰지 않는다: 가변길이 불변 내장(tuple)의 서브타입은 비어있지 않은
    #   `__slots__` 를 지원하지 않아 **import 시점에 TypeError** 로 모듈이 통째로 죽는다.
    #   (그 실패는 조용하지 않다 — 실측으로 확인하고 `__dict__` 경로로 되돌렸다.)

    def __new__(cls, has, why, axes=None):
        self = tuple.__new__(cls, (has, why))
        self.axes = axes
        return self

    @property
    def has(self):
        """결손 존재 여부(bool) | None(판정 불가) — `self[0]` 의 이름 있는 접근자."""
        return self[0]

    @property
    def why(self):
        """판정 사유 문자열 — `self[1]` 의 이름 있는 접근자."""
        return self[1]


def _shared_verdict_deficit(status, requery=None, tick_s=None, detect=None, agents=None):
    """★부트 경로 전용 결손 산출(W-B1 ③) — (결손 bool, 사유) | (None, 소비불가 사유).

    check_verdicts(⑤check 의 판정 코어)를 **소비만** 하고 그 satisfied 를 재정의하지 않는다.
    ⑤check 와의 유일한 의도적 차이 = **unknown 등급의 계상 방향**:
      · ⑤check: unknown = 충족측(fail-open — check 는 파괴 행위가 없고, 콜드스타트 창
        (watchdog 첫 틱 전 = 전 좌석 unknown)에서 적색을 내면 위경보·exit 6 라이브락이다)
      · 결손 산출(여기): unknown = **시한부 해소 후 잔존 시 결손**(스폰측 fail-open —
        `cys boot` 호출을 유도한다. 죽었는데 프로브만 실패한 좌석이 '충족'으로 접혀 boot 가
        영영 생략되는 잔여 B3 를 닫는다. 중복 스폰은 boot 락 + `cys boot` 자체의 Unknown
        시한부 해소 + seat_death_confirmed 죽음확정 게이트가 3중으로 방어한다).
    ★check_verdicts 본체에 넣지 않는 이유(감사 확정): 결손 판정과 ⑤check 가 같은 함수라
      **동시에** 뒤집혀, 데몬 콜드스타트 창에서 노드가 다 살아 있는데도 ⑤check 실패 →
      exit 6 라이브락이 된다. 그래서 갈래는 여기(결손 산출 전용 함수)다 — ⑤ satisfied 불변.

    시한부 해소는 `javis_boot_node.resolve_unknown_for_spawn`(워치독 1주기 대기 → 재조회 1회
    → 잔존 불명 = 결손 취급)을 **그대로 소비**한다 — `cys boot` 스폰 경로가 이미 쓰는 규약과
    동일(신술어 발명 금지). 대기 1주기는 **역할 수와 무관하게 1회**다(재조회 status 공유 —
    unknown 좌석이 N 개라고 5s×N 을 태우면 부트 경로 예산이 계약 없이 부푼다).

    ★소비 배선 현황(정직 표기 · 2026-08-21 배선 완료 · W-B3): 이 함수가 **정본**이고
      소비자는 `javis_bootstrap._shared_verdict_deficit` 위임 래퍼다(④ boot 호출 생략 판정).
      bootstrap 의 로컬 구현은 `_shared_verdict_deficit_fallback` 으로 개명돼 **구 팩 스큐
      (이 함수 부재·import 실패·위임 예외) 전용 폴백**으로만 남았고, 폴백 발동은 stderr 1줄로
      고지된다(조용한 강등 금지). 배선 시 주의 2건은 **둘 다 이행 완료**다:
      ①반환 계약 (bool|None, 사유) 3자(정본·래퍼·폴백) 동일 — 실측 대조 완료.
      ②run_bootstrap_health H-PRED-1 의 '결손↔check 차분 0' 계약은 seat_unknown corpus 의
      **의도된 차분**(check 충족 vs 결손>0)을 예외로 두도록 개정됐고, 배선 실재·폴백 실재·
      ⑤check satisfied 불변을 그 검체가 함께 핀한다(소비자 0 재발 시 적색).

    requery/tick_s 는 밀폐 테스트 주입(기본: cys_status 재조회 · 워치독 1주기 대기).
    ★detect/agents 주입(2026-08-22 적대검증 중대④): `check_verdicts` 가 이미 갖고 있던 밀폐
      주입 규약을 **형제 소비자에게도 뚫는다**. 이 경로만 주입이 막혀 있어서, 형제 소비자
      `javis_bootstrap._shared_verdict_deficit` 을 쓰는 `javis_bootstrap.py --self-test` 가
      **agy·codex 가 없는 깨끗한 기계에서 항상 FAIL** 했다(실측: 빈 HOME → exit 1). 그 FAIL 이
      뒤따르는 단언들보다 앞이라 신규 커버리지가 통째로 실행조차 되지 않았다.
      미주입=None 이면 종전과 완전히 같은 실감지 경로다 — **프로덕션 거동 불변**.

    ★(U-25 1단) 반환은 `VerdictDeficit` — **길이 2 그대로**(`has, why = …` 무접촉)이고
      `.axes` 로 완주 2축 집계가 따라온다. 결손 값(`has`)은 축 on/off 로 **바뀌지 않는다**:
      이 단계에서 awake 는 관측일 뿐 결손 판정에 참여하지 않는다(무동작 계약 · `--self-test`
      가 축 on/off 동치를 핀한다). 판정 자체가 불가능했던 경로는 `.axes` 도 **None(미측정)**
      이다 — 빈 집계를 내면 '재어 보니 이상 없음'이라는 거짓 초록이 된다."""
    bn = _boot_node()
    try:
        verdicts, _roster = check_verdicts(status, detect=detect, agents=agents)
    except Exception as e:
        return VerdictDeficit(None, "check_verdicts 소비 불가(%s: %s)"
                              % (type(e).__name__, e))
    if not verdicts:
        return VerdictDeficit(None, "check_verdicts 빈 판정(로스터 산출 실패)")
    axes = verdict_axes(verdicts)
    missing = [r for r, v in verdicts.items() if not v.get("satisfied")]
    if missing:
        return VerdictDeficit(
            True, ("공유 판정 결손(의무 %s / 부재 %s) — 결손 존재 [신호=check_verdicts 동일]"
                   % (", ".join(verdicts), ", ".join(missing))), axes)
    unknowns = [(r, v.get("filler") or r) for r, v in verdicts.items()
                if v.get("grade") == (bn.LIVENESS_UNKNOWN if bn is not None else "unknown")]
    if unknowns and bn is not None:
        # 워치독 1주기 대기·재조회는 **전 역할 공유 1회**(첫 역할만 tick 대기, 이후 0) —
        # 재조회 결과를 메모해 같은 status 로 전 unknown 을 재판정한다.
        fresh = {}

        def _requery():
            if "st" not in fresh:
                fresh["st"] = requery() if requery is not None else cys_status()
            return fresh["st"]

        residual = []
        for i, (req_role, seat_role) in enumerate(unknowns):
            grade, why = bn.resolve_unknown_for_spawn(
                seat_role, _requery, tick_s=(tick_s if i == 0 else 0))
            if grade == bn.LIVENESS_ABSENT:
                residual.append("%s(%s)" % (req_role, why))
        if residual:
            return VerdictDeficit(
                True, ("부트 경로 unknown 결손(판정불가 잔존: %s) — 결손 존재 "
                       "[⑤check satisfied 는 불변·시한부 해소=resolve_unknown_for_spawn]"
                       % "; ".join(residual)), axes)
        return VerdictDeficit(
            False, ("공유 판정 충족(의무 %s 전원 — unknown %d건 전부 시한부 해소로 생존 확인)"
                    " — 결손 0(재선언) [신호=check_verdicts+unknown 시한부 해소]"
                    % (", ".join(verdicts), len(unknowns))), axes)
    # ★등급명은 리터럴이 아니라 정본 상수를 쓴다(무음 실패 방지): 정본이 개명되면 리터럴
    #   비교는 **조용히 항상 False** 가 돼 '생존추정 고지'가 통째로 사라진다 — 적색도 예외도
    #   없이 진단만 없어진다. 구 팩 스큐(상수 부재)에서만 종전 리터럴로 폴백한다(값 동일).
    _presumed_grade = getattr(bn, "LIVENESS_PRESUMED", "alive_presumed") if bn is not None \
        else "alive_presumed"
    presumed = [r for r, v in verdicts.items()
                if v.get("satisfied") and v.get("grade") == _presumed_grade]
    note = ("" if not presumed
            else " · 생존추정(각성 미확인) %s — 재각성 권장이나 결손 아님" % ", ".join(presumed))
    return VerdictDeficit(
        False, ("공유 판정 충족(의무 %s 전원) — 결손 0(재선언)%s [신호=check_verdicts 동일]"
                % (", ".join(verdicts), note)), axes)


def _check_payload(verdicts, roster, alive_optional, axes):
    """`check --json` 의 기계 판독 payload(순수 구성 — 부작용 0).

    ★1단 계약: `exit` 은 **ready 축**(종전과 완전히 동일)이고 `awake`·`awake_pending` 은
      **관측 필드**다. 소비자는 이 둘이 `null` 이면 **미측정**으로 읽어야 한다(축 off·구 팩
      스큐) — null 을 '각성 없음'으로 접으면 롤백 스위치가 기저보다 파괴적이 된다(BLOCK-4 형).
    ★사람용 경로의 대체 리뷰어 고지는 여기서 산문으로 다시 내지 않는다(stdout 은 JSON 1줄만
      이어야 기계 소비가 성립). 대신 `roster` 가 `native`·`substituted_for`·`reason` 을
      그대로 실어 나른다 — 강등이 조용히 사라지지 않는다."""
    return {
        "schema": "orchestra-check/v1",
        "exit": 1 if axes["ready_missing"] else 0,
        "axis": axes["axis"],
        "ready": axes["ready"],
        "ready_missing": axes["ready_missing"],
        "gated": axes["gated"],
        "awake": axes["awake"],
        "awake_pending": axes["awake_pending"],
        "required": list(verdicts.keys()),
        "verdicts": verdicts,
        "roster": roster,
        "optional": {r: bool(alive_optional.get(r)) for r in OPTIONAL_ROLES},
        "why": None,
    }


def _check_unknown_payload(why):
    """exit 2(**판정 불가**) 의 payload — 전 판정 필드가 `null` 이다.

    ★모듈 docstring 의 계약을 그대로 형상화한 것이다: exit 2 는 **노드에 대해 아무것도 말하지
      않는다**(데몬 소실을 '노드 미기동'으로 오귀속하면 처방이 뒤집힌다). 그래서 `ready` 를
      `false` 로 두지 않는다 — false 는 '재어 보니 안 섰다'는 **측정 결과**이고, 여기서는
      측정 자체가 없었다. 측정 불능은 어떤 게이트에서도 통과가 아니지만 실패 판정도 아니다.
    ★`--json` 인데 산문만 나가면 소비자의 `json.loads` 가 죽는다 — 그 죽음은 부트 체인이
      데몬 소실을 진단하지 못하는 순간과 정확히 겹친다. 그래서 이 경로도 JSON 을 낸다(사람용
      처방 문안은 stderr 로 보내 잃지 않는다)."""
    return {
        "schema": "orchestra-check/v1",
        "exit": 2,
        "axis": awake_axis_enabled(),
        "ready": None,
        "ready_missing": None,
        "gated": None,
        "awake": None,
        "awake_pending": None,
        "required": None,
        "verdicts": None,
        "roster": None,
        "optional": None,
        "why": why,
    }


# ── check: 4종 의무 노드 생존 판정 ──
def cmd_check(args):
    status = cys_status()
    if status is None:
        _why = "[orchestra check] cys status 수집 실패(데몬 미가동?) — `cys ping` 확인 후 재실행"
        if getattr(args, "json", False):
            # ★`--json` 은 **전 exit 경로**에서 stdout 이 JSON 이어야 한다. 여기만 산문을 내면
            #   소비자의 json.loads 가 데몬 소실 순간에 죽는다 — 진단이 가장 필요한 순간이다.
            #   사람용 처방(`cys ping`)은 stderr 로 보내 잃지 않는다(조용한 강등 금지).
            print(json.dumps(_check_unknown_payload(_why), ensure_ascii=False, sort_keys=True))
            print(_why, file=sys.stderr)
            return 2
        print(_why)
        return 2
    # ★판정은 순수 함수(check_verdicts)에 있고 여기는 표현만 한다 — 같은 함수를 bootstrap 결손
    #   판정·wakeup zombie 가드·reclaim 이 소비하므로 판정 이원화가 구조적으로 불가능하다(A1 클래스).
    verdicts, roster = check_verdicts(status)
    required = list(verdicts.keys())
    alive_optional = live_roles(status)
    # ★(U-25 1단) 분류를 **표현 이전으로** 끌어낸다. 종전엔 missing·gated 를 아래 print 루프
    #   안에서 만들었다 — 같은 규칙을 사람용·기계용 두 벌로 적지 않기 위한 것이고, 목록의
    #   값·순서(요건 순서)는 종전과 동일하다(사람용 출력 바이트 무변경).
    axes = verdict_axes(verdicts)
    missing, gated = axes["ready_missing"], axes["gated"]
    if getattr(args, "json", False):
        # ★기계 소비 전용 경로: 사람용 산문('READY' 토큰 포함)은 **한 줄도 내지 않는다**.
        #   디렉티브·C03 핀이 인용하는 문안은 아래 기본 경로에 그대로 있다.
        #   ★`getattr` 기본값이 필요한 이유: `javis_idempotency._spy_cmd_check` 가 속성이 없는
        #     빈 Args 로 이 함수를 부른다(관찰 멱등 negative assertion). `args.json` 직접
        #     참조는 그 검체를 AttributeError 로 죽인다.
        print(json.dumps(_check_payload(verdicts, roster, alive_optional, axes),
                         ensure_ascii=False, sort_keys=True))
        return 1 if missing else 0
    print("LLM orchestrating 노드 점검 (4종 의무 + grok 선택):")
    # 리뷰어 대체 고지(2026-06-14 — 정직한 라벨링: 보편적이나 벤더 다양성은 약함)
    for e in roster:
        if not e["native"]:
            print("  ⚠ %s 미감지(%s) → %s(Claude 대체) — 보편적이나 벤더 다양성 약함, "
                  "페르소나/렌즈/익명화로 보완(REVIEWER_DIRECTIVE §6)"
                  % (e["substituted_for"], e["reason"], e["role"]))
    # ★missing·gated 는 위 `verdict_axes` 가 이미 산출했다(값·순서 동일) — 여기서는 표현만 한다.
    for r in required:
        v = verdicts[r]
        if not v["satisfied"]:
            # ★(U-10) 제4 등급은 '미기동'이 아니다: 좌석도 프로세스도 살아 있는데 첫기동 관문
            #   (테마·로그인방식·OAuth·폴더신뢰·면책·새기능안내)에 갇혀 입력을 못 받는 상태다.
            #   여기서 "→ `cys boot` 로 기동하라"고 적으면 **거짓 처방**이다(boot 은 이 좌석을
            #   스폰도 회수도 하지 않는다 — 그게 옳은 동작이다). 사람이 관문을 한 번 통과시켜야
            #   한다. 미충족 계상·exit 계약은 **그대로**(1) 두고 라벨과 처방만 정확하게 만든다.
            if r in gated:
                print("  ✗ %s — 첫기동 관문 보류 (%s)" % (r, v["why"]))
            else:
                print("  ✗ %s — 미기동 (%s)" % (r, v["why"]))
            continue
        # ★B2 실충전자 라벨링 — 대체 좌석이 슬롯을 채웠으면 그 사실을 숨기지 않는다.
        fill = "" if v["native"] in (True, None) or v["filler"] == r else \
               " ← 실충전자 %s(대체)" % v["filler"]
        if v["grade"] == _awake_grade():
            print("  ✓ %s — 각성 확정(%s)%s" % (r, v["why"], fill))
        elif v["grade"] == "unknown":
            print("  ✓ %s — 좌석 판정불가(프로브 실패 — 적색 아님·재확인 권장)%s" % (r, fill))
        else:
            # agent_alive 단독·좌석 점유·quiet_but_alive — 각성 확정이 아니다(B6 강등 라벨).
            print("  ✓ %s — 생존추정(각성 미확인 · 재각성 권장: %s)%s" % (r, v["why"], fill))
    for r in OPTIONAL_ROLES:
        print("  %s %s — %s" % ("✓" if alive_optional.get(r) else "·", r,
                                "생존" if alive_optional.get(r) else "미설치/미기동(선택)"))
    if missing:
        # ★B1 정책 열 소비: 부재가 전부 Degrade(리뷰어)면 처방은 boot-reviewers(대체 폴백 포함)다.
        only_degrade = all(plan_policy(m) == FAIL_DEGRADE for m in missing)
        howto = ("javis_orchestra.py boot-reviewers (리뷰어 감지·자동 폴백)" if only_degrade
                 else "cys boot")
        print("종합: 필수 %d/%d 생존 — 부재: %s → `%s`로 기동하라"
              % (len(required) - len(missing), len(required), ", ".join(missing), howto))
        if gated:
            # ★(U-10) 관문 보류는 기동 명령으로 풀리지 않는다 — 정직한 처방을 한 줄 더 얹는다.
            print("  ※ 관문 보류(%s): 기동 명령으로 풀리지 않는다. 해당 pane 에서 첫기동 관문을 "
                  "사람이 1회 통과시킨 뒤 재부트하라 — 좌석·프로세스는 살아 있다"
                  "(`cys read-screen --surface <ref>` 로 화면 확인)." % ", ".join(gated))
        return 1
    print("종합: %d종 의무 노드 전부 생존 — LLM orchestrating READY" % len(required))
    return 0


# ── boot-reviewers: 리뷰어 감지→기동, 미감지 시 Claude 대체 자동 폴백(멈춤 없음) ──
# ─────────────────── A12: 호출 exit 분류 (transient vs permanent) ───────────────────
# ★재감사 A12: 674행 '죽은 초기화'가 설계 의도 소실의 물증이었다 — 모든 비0 을 뭉개 재시도하면
#   영구 실패(스크립트 부재·인터프리터 깨짐)를 24회 재시도로 태우고 정확한 처방을 잃는다.
#     2   = 치명(데몬 다운·인자 오류)   → **영구**: 즉시 fail + 정확 처방(재시도 금지)
#     127 = 명령/스크립트 부재          → **영구**: 즉시 fail + 설치·경로 처방(재시도 금지)
#     124 = timeout(부트가 느림)        → **transient**: 재시도 가치 있음
#     그 밖 비0(1 등) = 실측 미확정      → transient(보수 — 종전 동작 보존)
EXIT_CLASS_PERMANENT = "permanent"
EXIT_CLASS_TRANSIENT = "transient"
EXIT_CLASS_OK = "ok"


def classify_call_exit(rc, target="호출"):
    """순수 판정: 서브프로세스 rc → (class, 처방). class ∈ ok|permanent|transient."""
    if rc == 0:
        return EXIT_CLASS_OK, "성공"
    if rc == 2:
        return EXIT_CLASS_PERMANENT, (
            "%s 치명(exit 2 — 데몬 다운 또는 인자 오류): 재시도는 무의미하다. "
            "`cys ping` 으로 데몬을 확인하고 인자를 점검하라." % target)
    if rc == 127:
        return EXIT_CLASS_PERMANENT, (
            "%s 부재(exit 127 — 스크립트/인터프리터 없음): 재시도는 무의미하다. "
            "팩 경로(CYS_PACK_DIR)와 python 해소를 점검하라." % target)
    if rc == 124:
        return EXIT_CLASS_TRANSIENT, "%s timeout(exit 124) — 예산 내 재시도 가치 있음" % target
    return EXIT_CLASS_TRANSIENT, "%s 실패(exit %s) — 실측 미확정, 보수적으로 재시도 대상" % (target, rc)


def _boot_node_outer_timeout():
    """_boot_one_node 가 씌우는 외부 상한 — javis_budget 파생(하드코딩 금지·B9 역전 해소)."""
    try:
        import javis_budget as _b
        return float(_b.boot_node_outer_s())
    except Exception:
        return 130.0


def _boot_one_node(role, agent, timeout=None):
    """javis_boot_node.py 로 단일 노드 결정론 부트 → (ok, rc, exit_class, 처방).

    ★B9 데드라인 전파: 외부 상한을 javis_budget 에서 파생하고 **같은 예산을 `--timeout` 으로
      하위에 전달**한다. 종전엔 외부 130s 가 내부(90s + 데드라인 무시 서브프로세스 80s)를 넘지
      못해 정상 진행 중인 부트를 잘랐다(예산 역전 2/3).
    ★A12: rc 를 분류해 영구 실패는 재시도하지 않고 처방을 낸다.
    """
    outer = float(timeout) if timeout else _boot_node_outer_timeout()
    inner = max(10.0, outer - 12.0)      # 하위 데드라인 < 외부 상한(잔여 granularity 확보)
    bn = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_boot_node.py")
    if not os.path.isfile(bn):
        cls, why = classify_call_exit(127, "javis_boot_node.py")
        return False, 127, cls, why
    try:
        r = subprocess.run([sys.executable, bn, "--role", role, "--agent", agent,
                            "--timeout", "%.0f" % inner], timeout=outer)
        rc = r.returncode
    except subprocess.TimeoutExpired:
        rc = 124
    except Exception:
        rc = 2
    cls, why = classify_call_exit(rc, "boot_node(%s)" % role)
    return rc == 0, rc, cls, why


def cmd_boot_reviewers(args):
    """★2026-06-14: master 부트 후 리뷰어(agy·codex)를 '호출하는 단계'.
    감지를 못하면 멈추지 말고 곧바로 Claude 대체 리뷰어로 폴백 기동한다.
    2층 감지: (1) 바이너리 미설치 → 즉시 대체(detect_reviewer). (2) 설치됐으나 부트가
    각성(set-status ack)에 실패(미인증·깨짐) → 대체로 2차 폴백. 절대 halt 하지 않는다."""
    roster = reviewer_roster()
    print("[boot-reviewers] 리뷰어 슬롯 기동 (미감지/각성실패 시 Claude 대체로 자동 폴백):")
    results = []
    fillers = []          # ★B2: 슬롯을 **실제로 채운** 역할 — check 재해소의 근거(라벨링 대상)
    for (nrole, nagent, srole, sagent), e in zip(REVIEWER_SLOTS, roster):
        role, agent = e["role"], e["agent"]
        if not e["native"]:
            print("  ⚠ %s 미감지(%s) — %s(Claude) 대체 기동" % (nagent, e["reason"], srole))
        if args.plan:
            print("  · PLAN %-18s ← %-8s [%s]%s" % (role, agent, plan_policy(nrole),
                  "" if e["native"] else " (대체: %s 부재)" % nagent))
            results.append("plan")
            fillers.append({"slot": nrole, "filler": role, "native": e["native"]})
            continue
        ok, rc, cls, why = _boot_one_node(role, agent)
        if not ok and e["native"]:
            if cls == EXIT_CLASS_PERMANENT:
                # ★A12: 영구 실패(스크립트 부재·데몬 다운)는 대체 폴백으로도 못 고친다 —
                #   같은 헬퍼를 다시 부르면 같은 영구 실패다. 정확 처방만 내고 재시도하지 않는다.
                print("  ✗ %-18s ← %s — 영구 실패: %s" % (role, agent, why))
                results.append("failed")
                fillers.append({"slot": nrole, "filler": None, "native": None})
                continue
            # 설치됐으나 각성 실패(미인증·깨짐) — 2차 폴백: Claude 대체로 전환
            print("  ⚠ %s 기동/각성 실패(%s) — %s(Claude) 대체로 2차 폴백" % (role, why, srole))
            role, agent = srole, sagent
            ok, rc, cls, why = _boot_one_node(srole, sagent)
        print("  %s %-18s ← %s%s" % ("✓" if ok else "✗", role, agent,
                                     "" if ok else " — %s" % why))
        results.append("awake" if ok else "failed")
        fillers.append({"slot": nrole, "filler": role if ok else None,
                        "native": (role == nrole) if ok else None})
    awoke = sum(1 for s in results if s in ("awake", "plan"))
    # ★B2 실충전자 고지 — ⑤check 는 slot_satisfied 로 같은 사실을 재해소한다(대체 좌석=슬롯 충족).
    for f in fillers:
        if f["filler"] and f["native"] is False:
            print("  ↳ 슬롯 %s 는 대체 좌석 %s 가 충전 — check 는 이 좌석으로 슬롯을 재해소한다"
                  " (영구 적색·재선언 불회복 차단)" % (f["slot"], f["filler"]))
    if args.plan:
        print("종합(PLAN): 리뷰어 %d슬롯 — 감지 폴백 적용 로스터 출력(기동 안 함)" % len(results))
        return 0
    print("종합: 리뷰어 %d/2 각성 (Claude 대체 포함)%s"
          % (awoke, "" if awoke >= 2 else " — 부족: master 가 점검·수동 재기동"))
    # ★B1 정책: 리뷰어는 Degrade 다 — 부족은 **경고**로 강등하고 상위 체인(④-b→⑤)을 계속시킨다.
    #   비0 을 유지하되 소비부(javis_bootstrap ④-b)는 이 exit 로 부트를 죽이지 않는다(exit 4=Fatal 만).
    return 0 if awoke >= 2 else 1


# ── review-prompt: 제약을 항상 포함한 리뷰 의뢰 프롬프트 ──
def extract_constraints():
    """REVIEWER_DIRECTIVE §2 '엄격 제약' 항목을 디렉티브에서 동적 추출(진실원천)."""
    p = os.path.join(pack_dir(), "directives", "REVIEWER_DIRECTIVE.md")
    try:
        text = open(p, encoding="utf-8", errors="replace").read()
    except OSError:
        return None
    # "## 2. 엄격 제약" 섹션의 '- ' 불릿만 추출 (다음 '## ' 전까지)
    m = re.search(r"##\s*2\.\s*엄격 제약.*?\n(.*?)(?:\n##\s|\Z)", text, re.S)
    if not m:
        return None
    bullets = [ln.strip() for ln in m.group(1).splitlines() if ln.strip().startswith("- ")]
    return bullets or None


def harvest_rejected(handoffs_dir, slug=None, max_items=12, max_chars=1200):
    """handoff 기각 결정 수확(T1/P0-1 attention-p0 · 순수 함수 — self-test 밀폐 검증).

    양형식 파싱(실물 21건 포맷 드리프트 실측 2026-07-13): '## Rejected' H2 섹션과
    '**Rejected**:' 인라인 둘 다 수용 — H2만 파싱하면 기록 절반이 무음 누락된다.
    면역 자격은 master 기록 handoff만(A1 — 워커 자기 기각의 리뷰어 포획 차단).
    상한(max_items·max_chars)은 워커/리뷰어 컨텍스트 60% 계약 보호. 커버리지 동반(무음 상한 금지).
    반환: (kept, coverage) — 디렉토리 부재 시 ([], None) = 호출부 라인 생략(회귀 0).
    """
    try:
        names = [n for n in os.listdir(handoffs_dir) if n.endswith(".md")]
    except OSError:
        return [], None
    # 최근 우선(mtime 내림차순) + slug 접두 우선
    names.sort(key=lambda n: -os.path.getmtime(os.path.join(handoffs_dir, n)))
    if slug:
        names = [n for n in names if n.startswith(slug)] + \
                [n for n in names if not n.startswith(slug)]
    items, hits = [], 0
    for name in names:
        try:
            text = open(os.path.join(handoffs_dir, name),
                        encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        # 기록자는 파일 '첫 줄' 헤더에서만 — 본문 괄호 오매칭이 거짓 면역(false immunity)을
        # 만드는 위험 방향 차단(성찰 D2). 첫 줄이 규격 밖이면 '?' = 비면역(fail-safe).
        _first = text.split("\n", 1)[0]
        m = re.search(r"^#\s*\S+.*\(([^)]*)\)", _first)
        recorder = (m.group(1).split("·")[-1].strip() if m else "?")
        found = []
        sec = re.search(r"(?m)^##\s*Rejected[^\n]*\n(.*?)(?=\n##\s|\Z)", text, re.S)
        if sec:
            body = " ".join(sec.group(1).split())
            if body and body != "없음":
                found.append(body)
        for im in re.finditer(r"(?m)^\s*(?:[-*]\s*)?\*\*Rejected\*\*:?\s*(.+)$", text):
            found.append(" ".join(im.group(1).split()))
        if not found:
            continue
        hits += 1
        for f in found:
            items.append({"src": name[:-3], "recorder": recorder,
                          "immune": "master" in recorder.lower(), "text": f})
    total, kept = 0, []
    for it in items:
        if len(kept) >= max_items:
            break
        t = it["text"][:200]
        if total + len(t) > max_chars:
            break
        total += len(t)
        kept.append(dict(it, text=t))
    coverage = {"files": len(names), "with_rejected": hits,
                "kept": len(kept), "dropped": max(0, len(items) - len(kept))}
    return kept, coverage


def load_invariants(path, max_lines=20):
    """_round/INVARIANTS.md 불변식 불릿 로드(T1/P0-3 · 순수 함수). 부재·공백 시 None
    = 호출부 라인 생략(회귀 0). writer=master 단독(계약 헤더) — 여기선 읽기만."""
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return None
    bullets = [ln.strip() for ln in text.splitlines() if ln.strip().startswith("- ")]
    return bullets[:max_lines] or None


def cmd_review_prompt(args):
    bullets = extract_constraints()
    if not bullets:
        # 디렉티브 추출 실패 시에도 제약 누락은 허용 불가 — REVIEWER_DIRECTIVE §2 원문과
        # 동기화한 최소 제약을 하드 폴백(잘림 없이 전문 보존).
        bullets = [
            "- 지정된 파일/범위만 검토한다. 무관 저장소·파일 배회 금지, 도구 남용 금지.",
            "- 서버·장시간 프로세스를 띄우지 않는다. 필요하면 의뢰자에게 요청한다.",
            "- 검토 대상을 직접 수정하지 않는다(의견 제시가 기본). 직접 생성·수정 의뢰를 "
            "받은 경우에만 계약(파일·범위)을 선합의하고 수행한다.",
        ]
    rnd = args.round
    # D4: --manifest/--phase가 있고 명시 --success가 없으면 매니페스트 평가기준·review_focus 해소
    success = getattr(args, "success", None)
    mfocus = []
    if success is None:
        success, mfocus = resolve_manifest_phase(getattr(args, "manifest", None), getattr(args, "phase", None))
    lines = []
    lines.append("[리뷰 의뢰 — 엄격 제약 준수 · 지정 범위만]")
    lines.append("검토 범위(이 파일/범위만, 무관 파일·repo 배회 금지): %s" % args.scope)
    lines.append("과업: %s" % args.task)
    # 평가 기준 양방향(영상 N3 — "구현할 때도 먹이고 리뷰할 때도 똑같이 먹임"): success가 있으면
    # 구현 위임(task-prompt --success)과 동일한 기준을 리뷰어에게도 투입한다. 없으면 라인 생략
    # (회귀 0 — 기존 출력 바이트 동일).
    if success:
        lines.append("평가 기준(구현 위임과 동일 기준 — 이 기준 대비 채점하라): %s" % success)
    if mfocus:
        lines.append("리뷰 초점(매니페스트 review_focus): %s" % ", ".join(mfocus))
    lines.append("")
    lines.append("엄격 제약 (REVIEWER_DIRECTIVE §2 — 위반 금지):")
    lines.extend("  " + b for b in bullets)
    # T1(attention-p0 · 2026-07-13): 기각 재주입(P0-1) + 불변식 주입(P0-3).
    # 소스 부재 시 아무것도 덧붙이지 않는다(기존 출력 바이트 동일 = 회귀 0 관례).
    # 역할 차등(A2): reviewer2(감사)에는 면역이 아니라 감사 자료로 제공 — 감사 독립성 보존.
    _root = os.environ.get("JAVIS_ROOT") or os.getcwd()
    _role = getattr(args, "reviewer_role", None) or "reviewer1"
    _rej, _cov = harvest_rejected(os.path.join(_root, "_round", "handoffs"),
                                  slug=getattr(args, "slug", None))
    if _rej:
        lines.append("")
        if _role == "reviewer2":
            lines.append("과거 기각 이력 (면역 아님 — 감사 자료: 기각 자체의 정당성까지 감사하라):")
        else:
            lines.append("이미 기각된 지적 (master 확정 기각만 면역 — 재제기하려면 당시 기각 근거를 반박하라):")
        for _it in _rej:
            _tag = "면역후보" if _it["immune"] else "참고(비면역 — 워커 자기기각은 면역 없음)"
            lines.append("  - [%s · %s · %s] %s" % (_it["src"], _it["recorder"], _tag, _it["text"]))
        lines.append("  (커버리지: handoff %d건 · 기각기록 %d건 · 주입 %d · 상한제외 %d)"
                     % (_cov["files"], _cov["with_rejected"], _cov["kept"], _cov["dropped"]))
    _inv = load_invariants(os.path.join(_root, "_round", "INVARIANTS.md"))
    if _inv:
        lines.append("")
        lines.append("프로젝트 불변식 (_round/INVARIANTS.md — 이와 충돌하는 지적은 불변식 반박 근거 필수. "
                     "불변식 뒤에 숨는 무지적도 금지):")
        lines.extend("  " + b for b in _inv)
    lines.append("")
    lines.append("리뷰 형식: [문제점] [논쟁점] [다음 단계 조언] — 각 지적에 파일:라인 또는 구체 근거.")
    lines.append("근거 없는 인상비평·칭찬만 하는 리뷰 금지. 결함을 찾는 것이 직무다.")
    # ★전환 게이트 §9-7-2 부수 2: 라운드 목표는 고정 향상률이 아니라 "잠근 합격 기준"이다
    # (운영계약 §7-1 통과 조건 · §7-6 고정 향상률 문구 전면 금지). 점수(0-100)는 §6-4가
    # 금지한다 — 평균·다수결 affordance 차단. 이 줄은 라운드 무관하게 항상 주입된다.
    lines.append("통과 조건: **잠근 합격 기준의 미달 항목 0** — 점수(0-100)·고정 향상률 목표는 금지. "
                 "판정은 verdict enum(ACCEPT|REVISE|BLOCK|ESCALATE) + evidence(file:line)로만 한다.")
    if rnd and rnd > 1:
        lines.append("라운드 %d: 직전 산출물을 해당 분야 최고 전문가 관점으로 재귀적 개선 관점에서 "
                     "평가한다(단순 코드수정 금지). 이 라운드의 종결 조건도 위 통과 조건과 같다." % rnd)
    lines.append("회신: `cys send --queued --to master \"[리뷰] ...\"` (자동 Return 배달 — "
                 "타이핑 가드 안전·send-key 불필요).")
    print("\n".join(lines))
    return 0


# ── task-prompt: 생존 게이트 + 절대 강조 4규칙을 항상 포함한 위임 티켓 ──
# 4규칙 무결성 마커 — 추출분이 이 전부를 포함해야 '완전한 4규칙'으로 인정한다.
# (부분 잘림·약화된 디렉티브가 티켓으로 전파되는 silent failure를 구조 차단 — 적대 검증 R1)
RULE_MARKERS = ("품질 절대우선", "할루시네이션 방지", "hallucination-guard", "몽상",
                "Garbage-in", "grill-me", "합의에 이를 때까지", "요약·압축 절대 금지",
                "전문용어·약호", "길이는 원문 수준", "충돌 시 상위 기준 절대 우선")


def extract_rules_from_text(text):
    """§N '절대 강조' 섹션의 불릿을 추출(순수 함수 — self-test가 밀폐 검증).

    - 헤더는 줄 시작 '## N.' + '절대 강조' (번호 하드코딩 안 함 — 절 번호 변경에 견딤)
    - 불릿 연속줄('- '로 시작하지 않는 들여쓰기 줄)은 직전 불릿에 합류 — 개행 wrap 잘림 방지
    - RULE_MARKERS 전부 포함해야 반환. 하나라도 빠지면 None(=폴백) — 약화 전파 차단
    """
    m = re.search(r"(?m)^##\s*\d+\.[^\n]*절대 강조[^\n]*\n(.*?)(?:\n##\s|\Z)", text, re.S)
    if not m:
        return None
    bullets = []
    for ln in m.group(1).splitlines():
        s = ln.strip()
        if s.startswith("- "):
            bullets.append(s)
        elif s and bullets:
            bullets[-1] += " " + s  # 연속줄 합류
    if not bullets:
        return None
    joined = "\n".join(bullets)
    if any(mark not in joined for mark in RULE_MARKERS):
        return None  # 부분 추출·약화 — 폴백이 안전
    return bullets


def extract_worker_rules():
    """WORKER_DIRECTIVE §'절대 강조 4규칙'을 디렉티브에서 동적 추출(진실원천)."""
    p = os.path.join(pack_dir(), "directives", "WORKER_DIRECTIVE.md")
    try:
        text = open(p, encoding="utf-8", errors="replace").read()
    except OSError:
        return None
    return extract_rules_from_text(text)


# WORKER §3 원문과 동기화한 하드 폴백(잘림 없이 전문 보존) — 추출 실패 시에도
# 절대 강조 4규칙 누락은 허용 불가(절대지침 5차: "task 시행을 명령할 때마다 절대 강조").
FALLBACK_RULES = [
    "- a) **품질 절대우선**: 조사의 깊이·폭·정확도가 절대 기준이다. 속도·토큰·편의는 "
    "이유가 될 수 없다.",
    "- b) **할루시네이션 방지**: 출처·근거·논리오류 분석·팩트체크가 필수인 작업·판단에는 "
    "전담 sub-skill(`cys skill show hallucination-guard`)을 반드시 사용해 검증 엄밀성·평가 "
    "신뢰성·환각 안전장치를 확보한다. 과장·거짓 확신·현실감 없는 출력 금지, 몽상·망상을 "
    "촉진하는 말 절대 금지. Garbage-in 차단 — 토대가 오염되면 아무리 다듬어도 거짓만 정교해진다.",
    "- c) **의도 합의**: 받은 지시의 의도 파악이 불충분하면 추측 진행 금지 — grill-me 스킬"
    "(`cys skill show grill-me`) 등으로 의뢰자(master)와 합의에 이를 때까지 질문을 반복한다.",
    "- d) **요약·압축 절대 금지**: 최종 결과물은 일반인도 이해하고 읽기 편하게 첨삭하되, 모든 "
    "분석·수치·표·단서를 하나도 빠뜨리지 않는다. 전문용어·약호·내부 검증 표시만 쉬운 말로 "
    "풀고 길이는 원문 수준을 유지한다.",
    "- **게이트**: 충돌 시 상위 기준 절대 우선. ②(b 할루시네이션 방지·검증)가 흔들리면 "
    "①③(그 위에 쌓는 나머지 실행)을 중단하고 master에 보고한다 — 토대 오염 위에 쌓지 마라.",
]


# ── 전제지식 자동주입(OpenMontage D6): 위임 티켓에 "어떤 증류 memory/스킬이 전제인지"를
# 이름·읽기명령·순서만 stitch한다(본문 아님 = progressive disclosure). normalize_slug·색인 파싱은
# javis_registry와 byte-동일 규칙(preflight는 orchestra를 import 안 함 → C39는 registry verify에 위임).
_PREREQ_HTML_COMMENT_RE = re.compile(r"<!--.*?-->", re.S)
_PREREQ_FENCED_CODE_RE = re.compile(r"```.*?```", re.S)
_PREREQ_INDEX_LINK_RE = re.compile(r"\]\(([^)\s]+\.md)\)")


def normalize_slug(ref):
    """ref → 표준 슬러그(javis_registry.normalize_slug와 byte-동일): lower·.md 제거·타입접두 제거."""
    s = (ref or "").strip().lower()
    if s.endswith(".md"):
        s = s[:-3]
    for t in ("feedback_", "user_", "project_", "reference_"):
        if s.startswith(t):
            s = s[len(t):]
            break
    return s


def parse_memory_index(memory_dir):
    """MEMORY.md 색인 → {정규화 슬러그: 파일명}. 주석·코드펜스 예시 제외(registry/memory 동일 규칙).
    색인 부재면 빈 dict — 호출부가 미해소를 인라인 표기한다(무음 드롭 금지)."""
    idx = os.path.join(memory_dir, "MEMORY.md")
    try:
        text = open(idx, encoding="utf-8", errors="replace").read()
    except OSError:
        return {}
    visible = _PREREQ_FENCED_CODE_RE.sub("", _PREREQ_HTML_COMMENT_RE.sub("", text))
    out = {}
    for m in _PREREQ_INDEX_LINK_RE.finditer(visible):
        fn = m.group(1)
        if "/" in fn or fn == "MEMORY.md":
            continue
        out[normalize_slug(fn)] = fn
    return out


def _split_csv(s):
    """쉼표 구분 인자 → 정리된 항목 리스트(빈 항목 제거)."""
    return [x.strip() for x in (s or "").split(",") if x.strip()]


def resolve_prereq_block(requires_skills, related_memory, memory_dir):
    """전제지식·읽기순서 블록 — 이름·읽기명령·순서만(본문 아님 = progressive disclosure).
    skill 먼저, 그다음 memory. 미해소 memory 슬러그는 무음 드롭 금지·인라인 표기한다.
    중복(같은 이름·정규화 슬러그)은 순서 보존하며 1회만 방출. 빈 입력이면 ""(티켓 무변)."""
    skills = [s for s in (requires_skills or []) if s]
    mems = [m for m in (related_memory or []) if m]
    if not skills and not mems:
        return ""
    index = parse_memory_index(memory_dir)
    lines = ["전제지식·읽기순서 (본문 아님 — 이름·읽기명령·순서만; 작업 전 읽어라):"]
    seen_sk = set()
    for name in skills:
        if name in seen_sk:  # 중복 skill은 순서 보존하며 1회만 방출(노이즈 차단)
            continue
        seen_sk.add(name)
        lines.append("  [skill] %s — cys skill show %s" % (name, name))
    seen_mem = set()
    for ref in mems:
        slug = normalize_slug(ref)
        if slug in seen_mem:  # 같은 슬러그로 정규화되는 ref는 1회만(collision collapse)
            continue
        seen_mem.add(slug)
        fn = index.get(slug)
        if fn:
            lines.append("  [memory] %s — cat %s" % (slug, os.path.join(memory_dir, fn)))
        else:
            lines.append("  [memory] (해소 불가: %s — 색인에 없음)" % ref)
    return "\n".join(lines)


def resolve_manifest_phase(manifest, phase_id):
    """타입드 워크플로우 매니페스트(D4) 단계 계약 해소 — javis_manifest phase에 위임.
    → (success_criteria.statement, review_focus[]). 부재·미지정·실패 시 (None, []) —
    호출부는 명시 --success를 우선(하위호환·byte-identical 보존)한다."""
    if not manifest or not phase_id:
        return None, []
    tool = os.path.join(pack_dir(), "bin", "javis_manifest.py")
    if not os.path.isfile(tool):
        return None, []
    try:
        r = subprocess.run([sys.executable, tool, "phase", manifest, "--phase", phase_id, "--json"],
                           capture_output=True, timeout=30)
        if r.returncode != 0:
            return None, []
        data = json.loads(r.stdout.decode("utf-8", "replace") or "{}")
        return (data.get("success") or None), (data.get("review_focus") or [])
    except Exception:
        return None, []


# ── todo 선언 블록 v1 (설계 DESIGN_declared-state.md §4-1 · 생산자 P3) ──────────────
# 유령 todo 사고의 근저원인은 소비자가 파일명·경로·mtime으로 소유권을 **추론**한 것이다.
# 추론을 없애려면 파일 자신이 소유를 **선언**해야 한다. 그런데 선언을 워커 손기재에 맡기면
# 실측상 오작성 6종 중 5종(따옴표·값 공백·키 누락·대문자 키·후행 주석)이 미선언으로 떨어진다
# — 그래서 티켓 발부 시점에 **완성된 한 줄**을 기계 생성해 동봉한다(손기동 자체를 줄인다).
DECL_VALUE_BAD_RE = re.compile(r"[^A-Za-z0-9._:-]+")
DECL_VALUE_OK_RE = re.compile(r"^[A-Za-z0-9._:-]+$")

# 파서는 단일 구현이다(재구현 금지 — ADR-2 2언어 파리티 계약의 Python 측). 생성한 선언을
# 파서에 되먹여 `counted`가 나오는지 확인하는 것이 유일한 계약 준수 증명이다.
# ⚠팩 부분갱신 스큐(ADR-2)로 파서가 없을 수도 있다 — 그때는 왕복 검증만 생략하고 문법 검사는
#   그대로 유지한다(여기서 예외를 던지면 팩 스큐 한 번에 위임 전체가 죽는다).
try:
    import javis_todo_decl as _decl                                       # noqa: E402
except Exception:                                                         # pragma: no cover
    _decl = None


def _pack_identity():
    """팩 **경로**와 **정체성(scope)** 을 한 번의 조회로 **함께** 확정한다.

    ★W14 S14 — 이것이 이 함수의 존재 이유다. 종전에는 티켓의 todo 경로가
    `${CYS_PACK_DIR:-$HOME/.cys/pack}/round/…` 문자열이라 **워커 셸에서 늦게** 전개되고,
    같은 티켓의 선언 `scope=`는 **master 프로세스에서 즉시** 확정됐다. 두 바인딩이 같다는
    보증이 아무 데도 없었다. 실측 재현 — master가 `scope=pack`으로 발부한 티켓을 워커가
    `pack-dept-dept-1` 팩에서 기록하면 소비자는 `excluded=[('worker','foreign-scope',0,2)]`,
    `pending_outside=[]`(주인이 명시한 처분이라 면제) → **false QUIET → 세션 주차**다.
    경로와 정체성은 **같은 시점·같은 값**에서 나와야 한다.
    """
    root = os.path.abspath(os.path.normpath(pack_dir()))
    return root, os.path.basename(root)


def decl_value_strict(raw):
    """**정체성 값**(owner·scope) 검증 — 접지 않고, 폴백도 없다. 위반이면 `None`.

    ★W14 S14 — 종전 `decl_value`는 허용 밖 문자를 `-`로 접고 남는 게 없으면 `"pack"`으로
    폴백했다. 팩 basename이 G4 문자집합 밖(예: `자비스`)이면 scope가 통째로 `pack`이 되어
    **그럴듯하지만 틀린 정체성**을 배포했다 — 그 선언을 받은 파일은 진짜 팩에서 `foreign-scope`
    로 **조용히 배제**된다. 유령을 막으려던 장치가 살아있는 작업을 지우는 정확한 형태다.

    같은 상황에서 스탬프 도구(`javis_todo_stamp.build_decl_line`)는 **정반대로** 시끄럽게
    실패했다(`선언 생성 실패(bad-token: …)`). 두 생산자가 반대로 행동하는데 어느 쪽이 정본인지
    계약이 없었고, master 심판은 **시끄러운 쪽**이다 — 추측한 정체성보다 없는 선언이 안전하다
    (선언이 없으면 소비자가 `unclaimed`로 fail-open 보고한다 · ADR-3).
    """
    s = (raw or "").strip()
    return s if DECL_VALUE_OK_RE.match(s) else None


def decl_slug(raw):
    """**진단 값**(lane) 전용 슬러그 — 여기서만 접는다. 정체성(owner·scope)에는 쓰지 마라.

    lane은 판정에 쓰이지 않는 사람용 표식이라(설계 §4-1 필드 표) 접어도 오배제를 낳지 않는다.
    남는 게 없으면 빈 문자열이고, 호출자는 그때 **키를 통째로 생략**한다(빈 값은 선언 전체를
    무효화한다 — G4 값은 1글자 이상).
    """
    return DECL_VALUE_BAD_RE.sub("-", (raw or "").strip()).strip("-.:_")


def todo_decl_line(to_role, task=None, today=None, scope=None):
    """티켓에 동봉할 선언 한 줄(v1). 반환 = `(line, None)` 또는 **`(None, 사유)`**.

    키 순서·필수 3종(owner·scope·status)은 설계 §4-1 고정. `scope`는 role 생존이 아니라
    **팩 정체성**이다 — owner 노드가 죽어도 선언은 파일에 남아 미완 작업이 노드 수명과
    분리된다. 값은 호출자가 `_pack_identity()`로 확정해 넘기며(경로와 **같은 바인딩**),
    생략 시 여기서 같은 함수로 조회한다.

    `lane`·`since`는 판정에 쓰지 않는 진단용이라, lane 슬러그가 비면 키를 통째로 생략한다.

    ★실패는 시끄럽다(W14 S14). 정체성 값이 G4를 어기면 접거나 폴백하지 않고 사유를 돌려준다.
    ★생성물은 **파서에 되먹여** 검증한다(스탬프 도구 `build_decl_line`과 같은 왕복 패턴) —
      문법 검사식을 여기 다시 적으면 그중 하나는 반드시 뒤처지고, 뒤처진 쪽이 소비자와 갈린다.
    """
    if scope is None:
        _, scope = _pack_identity()
    owner_v = decl_value_strict(to_role)
    scope_v = decl_value_strict(scope)
    if owner_v is None or scope_v is None:
        bad = "owner=%r" % to_role if owner_v is None else "scope=%r" % scope
        return None, ("선언 값이 G4 문자집합(`[A-Za-z0-9._:-]+`)을 벗어난다: %s" % bad)

    parts = ["owner=%s" % owner_v, "scope=%s" % scope_v]
    lane = decl_slug(task)[:48].strip("-.:_")
    if lane:
        parts.append("lane=%s" % lane)
    parts.append("status=active")
    parts.append("since=%s" % (today or time.strftime("%Y-%m-%d")))
    line = "<!-- javis:todo v1 %s -->" % " ".join(parts)

    if _decl is not None:                       # 파서 왕복 검증(계약 준수의 유일한 증명)
        d, diag = _decl.parse(line + "\n")
        if d is None:
            return None, "선언 생성 실패(%s: %s)" % (getattr(diag, "code", "?"), diag)
        if _decl.classify(d, scope_v, lambda s: True) != "counted":
            return None, "선언 생성 실패(counted 미달)"
    return line, None


def todo_file_name(role):
    """역할 → todo 파일명. 생산자 3곳(P1 `cys todo-path` · P2 cycle 저장검증 · P3 여기)이
    **같은 규칙**을 쓴다: 대문자화 + 하이픈→언더스코어."""
    return "%s_TODO.md" % role.upper().replace("-", "_")


def build_task_ticket(task, scope, success, to_role, rules, output_format=None, prereq_block="", dont=None, tier_hint=None, probes=None):
    """위임 티켓 본문 생성. rules는 필수 — 호출자가 추출 성패를 알고 명시 주입한다
    (기본값 경유의 무경고 폴백 경로 제거 · self-test는 rules 주입으로 밀폐 검증).
    tier_hint(R2 1단계): 권장 실행 등급 정보 1줄(강제 아님·None이면 라인 부재 → byte-identical).
    probes(P3 · 설계 §4 컴포넌트 C): 이 태스크의 필수 probe 이름 리스트. 지정 시 '필수 probe' 블록
    삽입, 빈/None이면 블록 부재 → 기존 티켓과 byte-identical(하위호환). E1 evidence-artifact 게이트와
    는 별개·보완 채널(E1=산출물 파일 `--evidence-artifact`, P3=`--evidence` 텍스트 증거범주·probe 영수증)."""
    bullets = rules
    lines = []
    lines.append("[작업 위임 — 절대 강조 4규칙 포함 · work management 앵커]")
    lines.append("작업: %s" % task)
    lines.append("범위(이 파일/범위만 — 무관 파일·repo 배회 금지): %s" % scope)
    # do/don't 쌍: scope=손댈 것(양) · dont=절대 손대지 말 것(음의 경계). 4대 행동지침③ 외과적
    # 변경을 위임 티켓에 기계적으로 주입한다. dont=None이면 라인 부재 → 기존 티켓 byte-identical.
    if dont:
        lines.append("무접촉(절대 건드리지 마라 — 아래 대상은 수정·삭제·리팩터·포맷 금지): %s" % dont)
    if success:
        lines.append("성공 기준(완료 보고는 이 기준 대비 검증 결과를 포함하라): %s" % success)
    if output_format:
        lines.append("산출 형식(이 형식·구조로 산출하라 — W8 4-part output-format): %s" % output_format)
    if tier_hint:
        lines.append("권장 실행 등급(정보·강제 아님 — 작업 난이도 참고용): %s" % tier_hint)
    lines.append("")
    lines.append("절대 강조 4규칙 (WORKER_DIRECTIVE §3 — 모든 작업에 적용·위반 금지):")
    lines.extend("  " + b for b in bullets)
    lines.append("")
    # ★W14 S14 — 경로와 선언 `scope`를 **같은 시점·같은 조회**에서 확정한다.
    #
    # 종전에는 경로가 `${CYS_PACK_DIR:-$HOME/.cys/pack}/round/…` 문자열이라 **워커 셸에서 늦게**
    # 전개되고 선언 scope는 **master 프로세스에서 즉시** 확정됐다. 두 바인딩이 같다는 보증이
    # 없었고, 갈리면 워커가 만든 파일이 자기 팩에서 `foreign-scope`로 **조용히 배제**된다
    # (실측: excluded=[('worker','foreign-scope',0,2)] · pending_outside=[] → false QUIET → park).
    # 지금은 master가 확정한 **절대경로**를 티켓에 박는다 — 발부자와 수행자가 같은 팩을 가리킨다.
    pack_root, scope_id = _pack_identity()
    todo_path = os.path.join(pack_root, "round", todo_file_name(to_role))
    lines.append("todo 영속: 이 작업을 \"%s\"에 "
                 "분해하고 세부 완료마다 체크박스를 갱신하라(진행%% 집계 원천). "
                 "경로는 발부 시점에 확정된 절대경로다 — 다른 팩에 만들면 집계에서 배제된다."
                 % todo_path)
    # ★todo 선언(설계 §4-1) — 집계기는 파일명·경로·mtime이 아니라 이 선언으로 귀속을 판정한다.
    # 위치 계약(첫 체크박스 이전)은 협상 대상이 아니다: 체크박스 뒤의 선언을 인정하면 본문에
    # 적힌 문구가 스스로를 무효화하는 자해 경로가 열린다(A2 회귀).
    decl, why = todo_decl_line(to_role, task, scope=scope_id)
    if decl is not None:
        lines.append("todo 선언(위 파일을 새로 만들 때 **첫 체크박스보다 위**, 머리말 첫 줄에 아래 한 줄을 "
                     "그대로 복사하라 — 집계기는 파일명이 아니라 이 선언으로 소유·귀속을 판정한다. "
                     "따옴표 추가·값에 공백·키 대문자화는 전부 '미선언'으로 떨어지니 한 글자도 고치지 마라. "
                     "이미 선언이 있는 파일이면 다시 넣지 마라 — 선언이 2개면 모호성으로 미선언 처리된다. "
                     "레인·스테이지가 끝나면 `status=active`를 `status=retired`로 바꿔 은퇴를 선언하라):")
        lines.append("  %s" % decl)
    else:
        # ★실패는 시끄럽다(S14). 접어서 그럴듯한 정체성(`scope=pack`)을 배포하지 않는다 —
        # 틀린 정체성은 살아있는 파일을 남의 레인으로 **조용히** 배제시키고, 그 배제는 QUIET
        # 불변식의 면제 대상이라 마지막 방어선조차 통과한다. 선언이 아예 없으면 소비자는
        # `unclaimed`로 fail-open 보고한다(ADR-3) = 시끄럽지만 안전한 쪽이다.
        sys.stderr.write("javis_orchestra: todo 선언을 생성하지 못했다 — %s "
                         "(role=%s scope=%s)\n" % (why, to_role, scope_id))
        lines.append("todo 선언: **생성 실패** — %s. 선언 없이 파일을 만들어라(집계기가 "
                     "`unclaimed`(미선언)로 시끄럽게 보고한다). 임의로 값을 고쳐 넣지 마라 — "
                     "틀린 `scope`는 이 파일을 '남의 레인'으로 **조용히** 배제시켜 진행률에서 "
                     "사라지게 만든다. 팩 이름을 G4 문자집합(`[A-Za-z0-9._:-]+`)으로 바로잡은 뒤 "
                     "`cys todo-path --emit-decl`로 다시 받아라." % why)
    lines.append("보고 채널: 완료·질문·충돌·막힘은 `cys send --queued --to master \"[보고] ...\"` "
                 "로 직접 push하라(--queued는 자동 Return 배달 — send-key 불필요·타이핑 가드 "
                 "안전). 즉시 끼어들어야 할 긴급 보고만 직접 send 후 `cys send-key --to master "
                 "Return`(가드 차단 시 --queued로 전환).")
    # done 증거 게이트(P3 · 설계 §2.2·§4 컴포넌트 C) — E1 산출물 파일 게이트와 나란히 공존하는
    # 별개·보완 채널: E1=검증 산출물 **파일**(--evidence-artifact), 여기=`--evidence` **텍스트**의
    # 증거 범주(negative-case/실데이터)와 probe 영수증 대조. 문구 중복·모순 없이 둘 다 명시한다.
    lines.append("done 증거 게이트(P3): `--evidence` 텍스트는 ①negative-case(고장 입력 검증) 또는 "
                 "②실데이터(합성 픽스처 아님) 검증 결과를 담을 것. probe를 실행했으면 `probe:<name>` "
                 "토큰을 `--evidence`에 명시하라(done 전이 시 probe 영수증 자동 대조). "
                 "※아래 E1은 검증 산출물 **파일**(`--evidence-artifact`) 채널 — 둘은 별개·보완이다.")
    # 필수 probe 블록은 probes 지정 시에만 삽입 — 미지정이면 라인 부재(하위호환·byte-identical).
    # ★--task 동반 필수(R1 major-a): 영수증 대조는 (probe명∧exit0∧최근성∧target 일치)이고
    #   relaxed probe(submit·ctx-compare·kill-preflight)의 target은 --task로만 바인딩된다 —
    #   --task 없는 무-task 영수증은 done 대조에서 대상 불일치로 거부된다. task-prompt는 작업
    #   서술만 알고 장부 task-id를 모르므로 `<task-id>` 플레이스홀더로 출력하고 워커가 치환한다.
    if probes:
        lines.append("필수 probe: 이 작업은 done 전 %s 각각 PASS 영수증 필수 "
                     "(`python3 ${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_actprobe.py <name> "
                     "--task <task-id> …` 실행 후 `--evidence`에 `probe:<name>` 토큰 포함). "
                     "<task-id>는 네 장부 태스크 id로 치환하라 — task-prompt는 작업 서술만 알고 "
                     "장부 id를 모른다. 미실행·FAIL 시 done 거부." % ", ".join(probes))
        lines.append("  ⚠ relaxed probe(submit·ctx-compare·kill-preflight)는 `--task` 없이 "
                     "실행하면 무-task 영수증이 되어 done 대조에서 대상 불일치로 거부된다 — "
                     "반드시 --task를 동반하라.")
    # E1 증거의 기계화(설계 §E1): 태스크 done 전이는 실제 검증 산출물 파일을 제출해야 통과(strict).
    lines.append("완료 증거(E1 evidence-artifact 게이트 · strict): 태스크를 done 처리할 때 검증 산출물"
                 " 파일(테스트 로그·빌드 출력 등, 권장 위치 `_round/evidence/<task-id>/`)을 만들고 "
                 "`javis_task.py set-status <id> done --evidence-artifact <경로>`로 제출하라 — "
                 "파일은 실존·비어있지않음·태스크 착수 이후 신선도를 기계 검사한다(검증 불가 시 "
                 "--skip-reason, skip_audit.jsonl 감사 기록).")
    if prereq_block:
        lines.append("")
        lines.append(prereq_block)
    return "\n".join(lines)


def cmd_task_prompt(args):
    # 역할명은 kebab-case만 — 오류 메시지·todo 파일명에 그대로 보간되므로 위생 처리(주입 차단).
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", args.to):
        print("[task-prompt] --to 역할명은 kebab-case(a-z0-9-)만 허용: %r" % args.to,
              file=sys.stderr)
        return 2
    # 생존 게이트 (절대지침 5차-1): "워커가 정상 작동하는 것을 확인한 후 작업 지시를 내린다"
    # — 이 확인은 눈대중이 아니라 cys status의 agent_alive로만 확정한다.
    # ★일회용(fresh) 경로 예외(D5/B2): --no-survival-gate면 생략한다 — 워커 surface는 실행 시점에
    #   schedule --fresh가 worker-fresh-*로 생성(+디렉티브 주입)하므로 지금 생존 확인은 의미가 없다.
    #   (raw pane 주입이 아니라 디렉티브 주입 워커이므로 무계약·치명 결함 위험 없음.)
    if not getattr(args, "no_survival_gate", False):
        status = cys_status()
        if status is None:
            print("[task-prompt] cys status 수집 실패(데몬 미가동?) — `cys ping` 확인 후 재실행. "
                  "대상 생존 미확인 상태로는 티켓을 내지 않는다.", file=sys.stderr)
            return 2
        if not live_roles(status).get(args.to):
            print("[task-prompt] 대상 '%s' 미기동 — 티켓 미출력. `cys boot`(4종 의무 기동) 또는 "
                  "`cys launch-agent --role %s --agent claude`로 기동 후 재실행하라."
                  % (args.to, args.to), file=sys.stderr)
            return 1
        # '정상 작동' 보조 신호: 장기 idle(기본 5분 — CYS_IDLE_SECONDS와 동기)은 hang일 수
        # 있다 — 차단은 아니고 경고만(지시 대기 중인 워커도 idle이므로 alive가 결정 기준,
        # idle은 §5 능동 점검 트리거). 같은 role의 죽은 stale surface는 건너뛴다.
        try:
            idle_thr = int(os.environ.get("CYS_IDLE_SECONDS", "300"))
        except ValueError:
            idle_thr = 300
        for s in status.get("surfaces", []):
            if s.get("role") == args.to and s.get("agent_alive"):
                idle = s.get("idle_secs")
                if isinstance(idle, (int, float)) and idle >= idle_thr:
                    print("[task-prompt] 주의: '%s' idle %d초 — hang 여부를 read-screen으로 "
                          "확인 후 전송하라(§5 능동 점검)." % (args.to, int(idle)), file=sys.stderr)
                break
    rules = extract_worker_rules()
    if rules is None:
        print("[task-prompt] 경고: WORKER_DIRECTIVE '절대 강조 4규칙' 추출 실패 또는 "
              "마커 불완전 — 하드 폴백(FALLBACK_RULES)으로 주입한다. 디렉티브를 점검하라"
              "(preflight C03).", file=sys.stderr)
        rules = FALLBACK_RULES
    # D4: 명시 --success가 없고 --manifest/--phase가 있으면 매니페스트 success_criteria 주입(명시 우선=하위호환)
    success = args.success
    if success is None:
        success, _ = resolve_manifest_phase(getattr(args, "manifest", None), getattr(args, "phase", None))
    prereq = resolve_prereq_block(
        _split_csv(getattr(args, "requires_skills", None)),
        _split_csv(getattr(args, "related_memory", None)),
        os.path.join(pack_dir(), "memory"))
    print(build_task_ticket(args.task, args.scope, success, args.to, rules=rules,
                            output_format=getattr(args, "output_format", None),
                            prereq_block=prereq, dont=getattr(args, "dont", None),
                            tier_hint=getattr(args, "tier", None),
                            probes=_split_csv(getattr(args, "probes", None))))
    return 0


# ── phase-plan: Task를 자기완결 Phase 티켓으로 분해 (영상 N6) ──
# 영상: Task=작업 통째, Phase=그 작업을 마치기 위해 나눈 단계들. 각 Phase는 독립 세션이
# "이것만 보고도" 완수하게 자기완결시켜 메인 컨텍스트를 보존한다(rule 인덱스 JSON + 페이지별 지침).
# ★실행은 코드가 `claude -p` raw subprocess를 띄우지 않는다(harness-creator PROMPT_RUNNER_ABSENT
# 철학·자원 거버넌스 충돌 회피) — 스킬이 Phase 티켓을 Workflow pipeline 또는 cys 워커 순차
# 위임으로 실행하도록 안내한다.
def phase_index_path(task):
    safe = re.sub(r"[^0-9A-Za-z가-힣_.-]", "_", task)[:80]
    return os.path.join(pack_dir(), "round", "PHASE-%s.json" % safe)


def cmd_phase_plan(args):
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", args.to):
        print("[phase-plan] --to 역할명은 kebab-case(a-z0-9-)만 허용: %r" % args.to,
              file=sys.stderr)
        return 2
    phases = [p.strip() for p in args.phases.split(";") if p.strip()]
    if not phases:
        print("[phase-plan] --phases가 비었거나 형식 오류(세미콜론 분리 비어있음): %r — "
              "예: --phases \"설계;구현;검증\"" % args.phases, file=sys.stderr)
        return 2
    # 4규칙 주입은 task-prompt와 동일 원천(추출→실패 시 하드 폴백). 위임 게이트(노드 생존)는
    # phase-plan이 즉시 위임하지 않으므로(스킬이 순차 위임) 적용하지 않는다 — 계획 산출 단계.
    rules = extract_worker_rules()
    if rules is None:
        print("[phase-plan] 경고: WORKER_DIRECTIVE '절대 강조 4규칙' 추출 실패 또는 "
              "마커 불완전 — 하드 폴백(전문)으로 강등 주입한다(약화 전파 차단).", file=sys.stderr)
        rules = FALLBACK_RULES
    prereq = resolve_prereq_block(
        _split_csv(getattr(args, "requires_skills", None)),
        _split_csv(getattr(args, "related_memory", None)),
        os.path.join(pack_dir(), "memory"))
    n = len(phases)
    tickets = []
    index = {"task": args.task, "scope": args.scope, "phases": []}
    for i, name in enumerate(phases, start=1):
        pid = "P%d" % i
        # 각 Phase는 자기완결 — 독립 세션이 이 티켓만 보고도 완수하도록 직전 Phase 산출물·
        # docs-diff 참조를 명시한다(영상: 페이지별 상세 지침·메인 컨텍스트 보존).
        prev = ("직전 Phase(%s) 산출물과 docs-diff(javis_docsdiff.py 변경 줄)를 참조하라."
                % ("P%d" % (i - 1)) if i > 1 else
                "이 작업의 첫 Phase다 — 컨텍스트의 구체화된 계획을 출발점으로 삼는다.")
        phase_task = "[%s/%d] %s — %s" % (pid, n, args.task, name)
        phase_scope = ("%s | 이 Phase만 독립 실행(자기완결): %s. %s 다른 Phase 작업·범위는 "
                       "건드리지 마라." % (args.scope, prev,
                       "산출물은 작업 폴더에 남기고 완료를 master에 push해 다음 Phase를 잇는다."))
        ticket = build_task_ticket(phase_task, phase_scope, args.success, args.to, rules=rules,
                                   prereq_block=prereq, dont=getattr(args, "dont", None))
        tickets.append((pid, name, ticket))
        index["phases"].append({"id": pid, "name": name, "status": "pending"})
    p = phase_index_path(args.task)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8") as f:
        json.dump(index, f, ensure_ascii=False, indent=2)
    # 사람이 읽는 티켓들(각 Phase 자기완결) + 기계 인덱스 경로를 출력.
    blocks = []
    blocks.append("[phase-plan] Task를 %d개 자기완결 Phase로 분해 — 인덱스: %s" % (n, p))
    blocks.append("실행: 코드는 claude -p를 띄우지 않는다. 아래 Phase 티켓을 Workflow "
                  "pipeline 또는 cys 워커로 순차 위임하라(각 Phase 독립 세션·메인 컨텍스트 보존).")
    for pid, name, ticket in tickets:
        blocks.append("")
        blocks.append("════════ %s · %s ════════" % (pid, name))
        blocks.append(ticket)
    print("\n".join(blocks))
    return 0


# ── round 장부 (결정론 라운드 추적) ──
def round_path(task):
    safe = re.sub(r"[^0-9A-Za-z가-힣_.-]", "_", task)[:80]
    return os.path.join(pack_dir(), "round", "ORCHESTRATION-%s.md" % safe)


def cmd_round_init(args):
    p = round_path(args.task)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    if os.path.exists(p):
        print("이미 존재: %s (round-status로 확인)" % p)
        return 0
    open(p, "w", encoding="utf-8").write(
        "# ORCHESTRATION 라운드 장부 — %s\n\n"
        "> 라운드 루프(운영계약 §6-5·§6-6). 완료조건: **잠근 합격 기준의 미달 항목 0**"
        "(외부 리뷰어 판정) 또는 %dR 상한 도달 — 먼저 온 것이 종결 사유다.\n"
        "> 자기채점 금지 · 점수(0-100) 금지(§6-4) — 판정은 producer≠evaluator(외부 리뷰어)의\n"
        "> verdict enum + evidence(file:line)다. 기록값 칸은 등급이 아니라 증거 발췌다.\n\n"
        "| 라운드 | 평가자 | 기록값 | 판정 |\n|---|---|---|---|\n" % (args.task, MAX_ROUNDS)
    )
    print("라운드 장부 생성: %s" % p)
    return 0


def _cell(s):
    """Markdown 표 셀 새니타이즈 — 파이프·개행이 표 구조(parse_rounds)를 깨지 않게."""
    return str(s).replace("|", "/").replace("\n", " ").strip()


def cmd_round_log(args):
    p = round_path(args.task)
    if not os.path.exists(p):
        cmd_round_init(args)
    # ★전환 게이트 §9-7-2 부수 1: --score 플래그를 제거했다(§6-4 점수 금지).
    #   기록값 칸의 기본은 "-" 이며, --from-cmd 경로에서만 기계검증 출력 꼬리를 담는다
    #   (등급이 아니라 증거 발췌다 — 평균·다수결 affordance 없음).
    score, verdict = "-", args.verdict
    machine_fail = False
    # machine 평가자의 결정론 기록(앵커6 축1): --from-cmd는 기계검증 명령을 이 도구가
    # 직접 실행해 exit code로 verdict를 자동 기록한다 — master(전환 이해당사자)의
    # 전사(轉寫)를 거치지 않는 producer≠evaluator 경로.
    if getattr(args, "from_cmd", None):
        try:
            # RC-6(D6): shell=True는 OS 기본 셸(unix=/bin/sh·Windows=cmd.exe)로 실행 — from_cmd는
            # OS중립 기계검증 명령(빌드·테스트) 전제다. bash 전용 문법을 넣으면 Windows cmd.exe에서
            # 실패하므로 RSI machine-eval 티켓은 OS중립 명령을 쓴다(저 consumer 영향·T3 실측 후 재판단).
            r = subprocess.run(args.from_cmd, shell=True, capture_output=True, timeout=1800)
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            # ★G8(cokacdir 성찰 2026-07-04 · _round/NODE_MEASURED_CONTRACT.md §2):
            #   exit 0은 PASS의 필요조건일 뿐이다 — ①stdout 에러형상(agy는 에러문을 stdout에
            #   싣는다, 계약 실측 #4·#6) ②LLM 노드 호출(agy/codex/claude)인데 빈 stdout
            #   (계약 §2·§4·§5)은 exit 0이어도 FAIL. 결정론 유닉스 명령의 무언 성공은 PASS 유지.
            first = next((l for l in tail.splitlines() if l.strip()), "")
            error_shaped = bool(re.match(r"\s*error\b", first, re.I))
            llm_cmd = bool(re.search(r"\b(agy|codex|claude)\b", args.from_cmd))
            if r.returncode != 0:
                verdict, machine_fail = "FAIL(exit %d)" % r.returncode, True
            elif error_shaped:
                verdict, machine_fail = "FAIL(exit 0·stdout 에러형상 — MEASURED_CONTRACT §2)", True
            elif llm_cmd and not tail:
                verdict, machine_fail = "FAIL(exit 0·LLM 빈 stdout — MEASURED_CONTRACT §2)", True
            else:
                verdict = "PASS(exit 0)"
            score = (tail.splitlines()[-1][:60] if tail else "-")
        except subprocess.TimeoutExpired:
            verdict, score, machine_fail = "FAIL(timeout 1800s)", "-", True
    elif evaluator_std(args.evaluator) == "machine":
        # ★G8: 경고→거부 격상 — machine 행은 --from-cmd 결정론 기록만(전사 금지 hard,
        #   MASTER §14). 스키마 미통과 기록이 게이트 신뢰를 갉는 경로를 닫는다.
        print("[round-log] 거부: machine 평가자는 --from-cmd 없이 기록 불가 — "
              "전사 금지(MASTER §14·G8). --from-cmd \"<명령>\"을 써라.", file=sys.stderr)
        return 2
    elif evaluator_std(args.evaluator) in ("gemini", "codex") and skip_reason(verdict) is None:
        # ★G8: 리뷰어 행은 타입 계약(_round/REVIEWER_VERDICT_CONTRACT.md) 강제 —
        #   verdict JSON이 javis_verdict 스키마(enum·evidence·score 금지)를 통과할 때만 기록.
        #   산문 전사·스키마 미통과는 거부. SKIP 행("SKIPPED: 사유")은 3-state 게이트 경로라 예외.
        vj = getattr(args, "verdict_json", None)
        if not vj:
            print("[round-log] 거부: 리뷰어(%s) 행은 --verdict-json <파일> 필수 — "
                  "산문 전사 금지(G8·REVIEWER_VERDICT_CONTRACT §2)." % args.evaluator,
                  file=sys.stderr)
            return 2
        try:
            import javis_verdict
            obj = json.load(open(vj, encoding="utf-8"))
            schema_errors, _lint, verdict_out = javis_verdict.validate_verdict(obj)
        except Exception as e:  # 모듈 부재·파일 없음·JSON 깨짐 전부 거부(fail-closed)
            print("[round-log] 거부: verdict JSON 검증 불가(%s) — fail-closed(G8)." % e,
                  file=sys.stderr)
            return 2
        if schema_errors:
            print("[round-log] 거부: verdict 스키마 미통과 — %s" % "; ".join(schema_errors),
                  file=sys.stderr)
            return 2
        # 기록 verdict = 검증기 출력 enum 그대로(R2 강등 반영). justification 산문은 셀에
        # 넣지 않는다(부정 어휘가 REJECT_MARKERS 게이트를 오작동). score 금지 계약 → "-".
        verdict, score = verdict_out, "-"
    with open(p, "a", encoding="utf-8") as f:
        f.write("| %d | %s | %s | %s |\n"
                % (args.round, _cell(args.evaluator), _cell(score), _cell(verdict)))
    print("기록: 라운드 %d · 평가자 %s · 기록값 %s · 판정 %s"
          % (args.round, _cell(args.evaluator), _cell(score), _cell(verdict)))
    # --from-cmd 검증 실패는 exit 1 — 기록은 성공했지만 && 체인이 "검증 통과"로
    # 오독하지 않게 한다(판정의 단일 진실은 gate-status).
    return 1 if machine_fail else 0


def parse_rounds(p):
    rows = []
    try:
        for ln in open(p, encoding="utf-8"):
            m = re.match(r"\|\s*(\d+)\s*\|\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|", ln)
            if m:
                rows.append({"round": int(m.group(1)), "evaluator": m.group(2),
                             "score": m.group(3), "verdict": m.group(4)})
    except OSError:
        return []
    return rows


def cmd_round_status(args):
    p = round_path(args.task)
    if not os.path.exists(p):
        print("라운드 장부 없음: %s — `round-init`로 생성" % p)
        return 1
    rows = parse_rounds(p)
    last = max((r["round"] for r in rows), default=0)
    print("라운드 현황 — %s" % args.task)
    print("  기록된 라운드: %d / 상한 %d" % (last, MAX_ROUNDS))
    if rows:
        r = rows[-1]
        print("  최근: 라운드 %d · 평가자 %s · 기록값 %s · 판정 %s"
              % (r["round"], r["evaluator"], r["score"], r["verdict"]))
    if last >= MAX_ROUNDS:
        print("  → %dR 상한 도달: 무한 루프 금지. 잠근 합격 기준에 미달이면 주인님께 "
              "격차를 보고하고 추가 라운드 여부를 여쭈어라(운영계약 §6-5)." % MAX_ROUNDS)
        return 0
    print("  → 다음 라운드 %d 진행 가능(잠근 합격 기준의 미달 항목 0 도달 전까지). "
          "외부 리뷰어가 verdict enum + evidence로 평가한다 — 점수·고정 향상률 금지." % (last + 1))
    return 0


# ── 자율주행 위임권 (앵커6) — 축1 게이트 4자 수렴 · 축3 다음 액션 큐 ──
# 축1: "게이트 4자 수렴(gemini+codex+master+기계검증)+커밋+SESSION_STATE 갱신 = 다음 단계
# 자동 착수". 수렴 여부를 LLM 눈대중이 아니라 round 장부의 기록으로만 판정한다.
GATE_EVALUATORS = ("gemini", "codex", "master", "machine")
# 표기 이주(2026-06-13): 구 Gemini CLI → Antigravity CLI(agy). 문서·라운드 기록이 'agy'로
# 표기해도 표준 평가자 'gemini'로 매핑한다(역할명 reviewer-gemini·어댑터 키 'gemini'와의
# 계약은 무변 — 식별자 층은 유지, 표기 층만 agy).
EVALUATOR_ALIASES = {"agy": "gemini"}
APPROVE_PREFIXES = ("pass", "수렴", "approve", "accept", "ok", "green", "승인")  # ★G8: enum ACCEPT 수용
# 부정 토큰이 하나라도 있으면 무조건 미승인 — 한국어 부정은 접미에 붙으므로("승인 불가"·
# "수렴 실패") 접두 매칭만으로는 게이트가 열린다(적대 검증 6차 R1 HIGH-1). 부정이 승인을
# 이긴다(안전 우선: 모호하면 닫힘).
REJECT_MARKERS = ("실패", "불가", "반려", "미달", "거부", "아님", "보류", "미흡", "미승인",
                  "fail", "reject", "deny", "denied", "no-go", "block", "not ")

# ── 3-state 게이트(OpenMontage D5): 의도적 SKIP을 None(기록없음)·False(미승인)과 구분한다.
# 안 돈 게이트가 PASS-by-absence(수렴)나 일반 미승인으로 위장하지 못하게 명시 상태로 기록.
SKIP_PREFIXES = ("skipped:", "skip:", "스킵:", "건너뜀:")


class Skip:
    """게이트의 3번째 상태 — 의도적 스킵(+사유). None도 bool도 아니다(isinstance로 판정)."""
    __slots__ = ("reason",)

    def __init__(self, reason):
        self.reason = reason


def skip_reason(verdict):
    """verdict가 'SKIPPED: <사유>' 형태면 사유 반환, 아니면 None. 빈 사유는 스킵 불인정(fail-closed)."""
    v = (verdict or "").strip()
    low = v.lower()
    for p in SKIP_PREFIXES:
        if low.startswith(p):
            return v[len(p):].strip() or None
    return None


# ── 무음실패 카탈로그(OpenMontage D5 2부): cys엔 guard.sh가 없다 — denylist는 CLAUDE.md §6
# 산문이다. 아래 SILENT_FAILURES가 그 산문을 손큐레이션한 source-of-record(런타임 prose 파싱
# 아님). render_catalog()가 결정론으로 .md 뷰를 파생한다. 무점수(수치 등급 키·값 없음). 각 행:
# {id, source(CLAUDE.md §ref), constraint, detection(위반 증명 아티팩트), kind}.
# kind=deterministic(기계 아티팩트로 증명)|heuristic(아티팩트 부재·수기 대조). 행 추가는 임베드
# .py라 cargo build 필요(CSO 공식 서명빌드). .md 재생성은 런타임(재컴파일 무관).
SILENT_FAILURES = [
    {"id": "SF-GATE-SCORE-FIELD",
     "source": "§6 리뷰어 verdict 타입 계약",
     "constraint": "verdict는 enum(ACCEPT|REVISE|BLOCK|ESCALATE)+evidence:file:line만 — 수치 score 금지(다수결·reward-hack 차단)",
     "detection": "verdict/round-log 레코드에 score 키 또는 0-100·0-1 수치 등급 값이 있으면 위반 — javis_verdict.py 스키마 게이트가 차단",
     "kind": "deterministic"},
    {"id": "SF-GATE-SKIPPED-AS-FALSE",
     "source": "§6 라운드 게이트·D5 3-state",
     "constraint": "의도적 SKIP은 None(미기록)·False(미승인)과 구분돼 명시 기록 — PASS-by-absence 위장 금지",
     "detection": "gate_verdicts가 'SKIPPED:' verdict를 Skip 인스턴스로 가로채는지(isinstance v,Skip) — False/None로 삼켜지면 위반; honest-skip만 남으면 gate-status exit 2",
     "kind": "deterministic"},
    {"id": "SF-DENY-CHARTER-EDIT",
     "source": "§6 denylist② charter 편집",
     "constraint": "soul.md·CLAUDE.md·*_DIRECTIVE.md·헌법 편집은 자율 금지 — 오너(owner) 토큰 승인 필수",
     "detection": "git diff 경로가 soul.md|CLAUDE.md|*_DIRECTIVE.md|directives/ 에 매칭되는데 owner 승인 토큰 레코드가 없으면 위반",
     "kind": "deterministic"},
    {"id": "SF-DENY-EXTERNAL-PUBLISH",
     "source": "§6 denylist③ 외부발행",
     "constraint": "외부발행/발송(git push·gh release·전송·공개)은 비가역 — 자율 금지·멈춰 승인(로컬커밋만 가역=허용)",
     "detection": "실행 명령이 git push|gh release|gh pr create/merge|publish/deploy|외부 전송 패턴에 매칭되는데 승인 없이 실행 로그에 있으면 경계 침범(R1·R2 preflight)",
     "kind": "deterministic"},
    {"id": "SF-DENY-IRREVERSIBLE-DELETE",
     "source": "§6 denylist④ 비가역 삭제",
     "constraint": "비가역 삭제/이동(rm·mv·chmod·git clean·truncate) 자율 금지 — 매 action 효과기반 preflight",
     "detection": "action 명령이 rm|mv|chmod|git clean|truncate 패턴에 매칭되는데 승인 없이 실행됐거나 preflight 로그가 비면 침범",
     "kind": "deterministic"},
    {"id": "SF-PLAN-DOWNGRADE",
     "source": "라우팅(tier 격하 금지)",
     "constraint": "라우터 판정 tier(slow>deliberate>fast)는 격상만 허용·격하 금지(과소발화가 안전)",
     "detection": "tier 격하를 증명할 필드-diff 아티팩트가 없어 결정론 탐지 불가 — 라우터 로그 대 실제 처리 모드 수기 대조(heuristic only)",
     "kind": "heuristic"},
    {"id": "SF-CONSENSUS-AVERAGE",
     "source": "§6 eval-driven·verdict 계약(독립 재유도)",
     "constraint": "리뷰어(agy·codex) 불일치는 다수결·평균 금지 — master 독립 재유도로만 해소",
     "detection": "agy.verdict≠codex.verdict인데 최종 확정 전 master 독립 재유도 레코드(별도 타임스탬프·증거)가 없으면 consensus-collapse",
     "kind": "deterministic"},
    {"id": "SF-PRODUCER-EQ-EVALUATOR",
     "source": "§6 eval-driven(producer≠evaluator)",
     "constraint": "측정 자기채점 금지 — 산출 노드(producer)와 채점 노드(evaluator) 분리, 채점=master LOCKED ref launcher·암호학적 핀",
     "detection": "eval 레코드의 producer_node_id==evaluator_node_id 이거나 LOCKED ref 핀(해시) 누락·불일치면 measurement 무효",
     "kind": "deterministic"},
    {"id": "SF-RETENTION-DELETE",
     "source": "§6 eval-driven(retention gate)",
     "constraint": "점수 올리려 콘텐츠·테스트 삭제하는 reward-hack 차단 — 이전 산출물·테스트 보존 강제",
     "detection": "라운드 N 항목집합이 N-1 집합을 포함하지 않으면(명시 deprecation 사유 없이) retention 위반·측정 무효",
     "kind": "deterministic"},
    {"id": "SF-ESCALATION-MISSING",
     "source": "§6 라운드 루프 8(10R escalation)",
     "constraint": "10R 도달인데 잠근 합격 기준에 미달이면 무한루프 금지 + 주인님께 격차 보고·판단 요청 필수",
     "detection": "기록 라운드>=10 AND 수렴 미달인데 SESSION_STATE에 ESCALATION 레코드+master→owner push가 없으면 위반",
     "kind": "deterministic"},
    {"id": "SF-DIRECTIVE-NOT-INJECTED",
     "source": "§3 워커 즉시 지침 주입",
     "constraint": "워커/리뷰어 생성 직후, 작업 티켓보다 선행해 DIRECTIVE 주입(각성) — 미주입 위임 금지(단일 sub-agent 수렴 치명에러)",
     "detection": "launch-agent 후 첫 task-prompt timestamp가 directive-ack push timestamp보다 앞서면 inject-skip 위반",
     "kind": "deterministic"},
    {"id": "SF-CROSSVERIFY-GATE-SWALLOWED",
     "source": "§8 품질 절대우선·§5② 교차검증 게이트",
     "constraint": "교차검증 게이트 실패 시 후속 ③공통분모·④대립비교·⑤결론 전면 중단+보고 — 통과로 흘리기 금지",
     "detection": "cross_verification_passed 플래그가 명시 True가 아닌데(누락 포함) ③④⑤ 산출물이 존재하면 swallowed-gate",
     "kind": "deterministic"},
    {"id": "SF-KILLSWITCH-IGNORED",
     "source": "§6 자율주행 메타안전(kill-switch)",
     "constraint": "오너 아무 입력=즉시 일시정지(kill-switch) · CSO 2-phase handshake 부재 시 self-clear 금지",
     "detection": "owner 입력 이벤트 timestamp 이후 autopilot 새 action 실행이 있으면 위반; self-clear에 대응 CSO handshake ack 레코드 없으면 unsafe-clear",
     "kind": "deterministic"},
    {"id": "SF-SUMMARY-COMPRESSION",
     "source": "§8 최종 산출물(요약금지)",
     "constraint": "최종 산출물 요약·압축 금지 — 분석·수치·표·단서 보존, 길이 원문 수준(쉬운 말 풀이 허용·항목 삭제 금지)",
     "detection": "최종본 길이·표·수치 개수가 직전 검증본 대비 현저히 감소하면 content-loss 의심 — 항목 삭제 여부는 수기 대조(heuristic)",
     "kind": "heuristic"},
    {"id": "SF-HALLUCINATION-NO-SOURCE",
     "source": "§8 환각방지·§5② 검색 선행",
     "constraint": "출처·근거 없는 단정 금지 — 모든 사실 주장은 인용/출처(URL·file:line) 동반(garbage-in 차단)",
     "detection": "사실 주장 문장에 출처 마커가 0이면 환각 의심 — 완결된 산문은 결정론 분리가 어려워 샘플 팩트체크 병행(heuristic)",
     "kind": "heuristic"},
    {"id": "SF-RENDER-RUNTIME-SWAP",
     "source": "영상 v2 §3 — OM CRITICAL 거버넌스(매니페스트 locked runtime ≠ 실제 렌더 = 위반)",
     "constraint": "edit가 고정한 render_runtime을 compose가 무음으로 교체 금지 — render_report.render_runtime이 edit_decisions의 고정값과 일치해야 한다(불일치·누락=무음 품질/포맷 강등)",
     "detection": "아키타입 매니페스트(D4) edit/compose phase의 field_present:render_runtime 게이트가 필드 부재를 1차 차단(check-criteria) + render_report.render_runtime != edit_decisions.render_runtime 값 대조는 video-verify 독립 노드(D1 verdict)",
     "kind": "deterministic"},
]

CATALOG_BANNER = (
    "<!-- 생성됨: `javis_orchestra.py silent-failure-catalog` 가 SILENT_FAILURES에서 결정론 파생.\n"
    "     손편집 금지 — 재생성: `javis_orchestra.py silent-failure-catalog`. "
    "드리프트는 preflight C38(WARN)·`--check`가 탐지. -->"
)


def render_catalog():
    """SILENT_FAILURES(소스-오브-레코드)에서 무음실패 카탈로그 .md를 결정론 파생한다.
    런타임 prose(CLAUDE.md §6) 파싱이 아니라 손큐레이션 튜플의 렌더 뷰다(재컴파일 회피)."""
    det = sum(1 for s in SILENT_FAILURES if s["kind"] == "deterministic")
    heu = sum(1 for s in SILENT_FAILURES if s["kind"] == "heuristic")
    lines = [
        "# SILENT_FAILURE_CATALOG — 무음실패 카탈로그 (OpenMontage D5)",
        "",
        CATALOG_BANNER,
        "",
        "> cys엔 guard.sh가 없다 — denylist는 CLAUDE.md §6 산문이다. 이 표는 그 산문을 각 항목의 "
        "**탐지절(위반 증명 아티팩트)**로 큐레이션한 것이다. 무점수(수치 등급 없음). source-of-record"
        "=javis_orchestra.py 내부 `SILENT_FAILURES`.",
        "",
        "| id | source (CLAUDE.md §ref) | constraint | DETECTION CLAUSE | kind |",
        "|---|---|---|---|---|",
    ]
    for sf in sorted(SILENT_FAILURES, key=lambda s: s["id"]):
        lines.append("| %s | %s | %s | %s | %s |" % (
            _cell(sf["id"]), _cell(sf["source"]), _cell(sf["constraint"]),
            _cell(sf["detection"]), _cell(sf["kind"])))
    lines += ["", "총 %d개 항목 — deterministic %d · heuristic %d." % (len(SILENT_FAILURES), det, heu), ""]
    return "\n".join(lines)


# ★WP-8(P-ORCH-5): 'block'을 부분문자열로 잡던 REJECT 게이트가 'unblocked'(정상어 — task
#   done 시 unblocked 의존자 보고 문맥)를 만나 승인 verdict를 오반려했다. 정상어는 화이트리스트로
#   먼저 지우고, 'block'은 단어 시작 경계(\bblock)로 검사해 'blocked'/'blockers'는 유지하되
#   'unblocked'/'roadblock' 등 선행결합어는 제외한다. 나머지 마커는 기존 부분문자열 매칭 유지.
_VERDICT_BENIGN = ("unblocked",)          # 'block' 포함하나 부정 신호 아님(정상어)
_BLOCK_WORD_RE = re.compile(r"\bblock")   # 단어 시작 경계 — 'un'/'road' 선행 복합어는 미매칭
_REJECT_MARKERS_SUBSTR = tuple(m for m in REJECT_MARKERS if m != "block")


def verdict_approved(verdict):
    """verdict 문자열의 승인 판정 — 부정 토큰 우선 차단, 그 다음 승인 접두(순수 함수)."""
    v = verdict.strip().lower()
    scan = v
    for w in _VERDICT_BENIGN:
        scan = scan.replace(w, " ")  # 정상어 제거 후 부정토큰 검사(부분문자열 오매칭 방지)
    if any(m in scan for m in _REJECT_MARKERS_SUBSTR) or _BLOCK_WORD_RE.search(scan):
        return False
    return any(v.startswith(p) for p in APPROVE_PREFIXES)


def evaluator_std(evaluator):
    """평가자 문자열 → 표준 평가자. 정확 일치 또는 구분자(:·-·공백) 접두만 인정 —
    'masterful'·'machinelearning' 류 오탐 차단(적대 검증 6차 R1 LOW-7).
    별칭(agy→gemini)도 같은 규칙으로 수용한다."""
    ev = evaluator.strip().lower()
    candidates = [(e, e) for e in GATE_EVALUATORS] + list(EVALUATOR_ALIASES.items())
    for name, std in candidates:
        if ev == name or ev.startswith(name + ":") or ev.startswith(name + "-") \
                or ev.startswith(name + " "):
            return std
    return None


def gate_verdicts(rows, rnd):
    """라운드 rnd의 평가자별 최종 verdict 승인 여부 — 순수 함수(self-test 박제).

    같은 평가자가 같은 라운드에 여러 번 기록하면 마지막 기록이 이긴다(재평가 허용).
    반환: {표준 평가자: bool|None}.
    """
    out = {e: None for e in GATE_EVALUATORS}
    for r in rows:
        if r["round"] != rnd:
            continue
        std = evaluator_std(r["evaluator"])
        if std:
            # SKIP은 verdict_approved 호출 *전*에 가로챈다 — 안 그러면 'SKIPPED: x'가
            # 승인접두도 부정토큰도 아니라 False(미승인)로 조용히 삼켜진다(D5 핵심).
            sr = skip_reason(r["verdict"])
            out[std] = Skip(sr) if sr is not None else verdict_approved(r["verdict"])
    return out


# gate-status exit 계약(결정론 환원 — 소비자는 이 코드만 본다):
#   0=수렴+임무 있음(자동 착수 가) · 1=미수렴 · 2=정직한 SKIP · 4=수렴했으나 임무 미지정(정지).
# ★4를 3이 아닌 값으로 둔 이유: 3은 `next-action` 이 이미 '임무 미지정'에 쓰고 있어, 같은 숫자를
#   다른 도구가 다른 의미로 쓰면 소비자 분기가 섞인다. 4는 이 도구에서 미사용이었다.
GATE_EXIT_NO_MISSION = 4


def cmd_gate_status(args):
    p = round_path(args.task)
    if not os.path.exists(p):
        print("[gate-status] 라운드 장부 없음: %s — round-init·round-log로 기록을 쌓아라"
              % p, file=sys.stderr)
        return 1
    rows = parse_rounds(p)
    rnd = args.round or max((r["round"] for r in rows), default=0)
    if rnd <= 0:
        print("[gate-status] 기록된 라운드 없음 — 미수렴", file=sys.stderr)
        return 1
    verdicts = gate_verdicts(rows, rnd)
    missing = [e for e, v in verdicts.items() if v is None]
    rejected = [e for e, v in verdicts.items() if v is False]
    skipped = [e for e, v in verdicts.items() if isinstance(v, Skip)]
    print("게이트 4자 수렴 판정 — %s (라운드 %d)" % (args.task, rnd))
    for e in GATE_EVALUATORS:
        v = verdicts[e]
        if isinstance(v, Skip):  # Skip은 truthy 객체라 명시 분기(아니면 ✓로 오표기)
            print("  ⊘ %s — SKIPPED: %s" % (e, v.reason))
        elif v is True:
            print("  ✓ %s — 승인" % e)
        elif v is None:
            print("  ✗ %s — 기록 없음" % e)
        else:
            print("  ✗ %s — 미승인" % e)
    if missing or rejected:
        print("종합: 미수렴 — %s%s. 자동 착수 불가(라운드 계속 또는 오너 보고)."
              % (("누락: " + ", ".join(missing)) if missing else "",
                 ((" / " if missing else "") + "미승인: " + ", ".join(rejected))
                 if rejected else ""))
        return 1
    if skipped:  # 누락·미승인 없이 의도적 스킵만 남음 → exit 2(미승인과 구분: 스킵 수용 판단)
        print("종합: 미수렴(정직한 SKIP) — %s. 스킵 수용 여부를 판단하라(미승인·누락과 구분)."
              % ", ".join("%s(%s)" % (e, verdicts[e].reason) for e in skipped))
        return 2
    # 보조 결정론(차단 아님): SESSION_STATE가 장부 마지막 기록보다 오래됐으면 "갱신" 요건
    # 미이행 가능성 경고 — 갱신은 전환 직전 수행이 규약이므로 순서상 이후일 수 있어 경고만.
    ss = os.path.join(pack_dir(), "round", "SESSION_STATE.md")
    try:
        if os.path.getmtime(ss) < os.path.getmtime(p):
            print("[gate-status] 주의: SESSION_STATE.md가 라운드 장부보다 오래됨 — 전환 전 "
                  "갱신 요건(축1)을 이행했는지 확인하라.", file=sys.stderr)
    except OSError:
        pass
    # ★T1(2026-08-01 실사고): 축1도 임무 게이트가 선행 조건이다 — 수렴은 '이 산출물이 통과했다'는
    #   뜻이지 '지금 달려도 된다'는 뜻이 아니다. 오너 임무가 없으면 여기서도 멈추고 보고한다.
    # ★적발 (c) 수리(2026-08-01 R2): 종전엔 임무 미지정을 감지하고도 **exit 0**(=CONVERGED)을
    #   냈다. 자연어 경고는 게이트가 아니다 — 소비자는 exit code 만 본다(결정론 환원 원칙).
    #   그래서 별도 exit 코드로 분기한다:
    #     0 = 수렴 + 임무 있음 → 자동 착수 가
    #     4 = **수렴했으나 임무 미지정** → 수렴 사실만 보고하고 정지(자동 착수 금지)
    #   1(미수렴)·2(정직한 SKIP)와 구분되므로 "왜 멈췄는가"가 exit 하나로 읽힌다.
    _mrc, _mv = _mission_gate()
    if _mrc != 0:
        print("종합: 수렴했으나 **임무 미지정 — 자동 착수 금지**(exit 4). 수렴 사실만 오너에게 "
              "보고하고 지시를 기다려라(§0-C 임무 게이트).")
        print("[gate-status] 임무 게이트 미통과: %s" % _mv.get("reason"), file=sys.stderr)
        return GATE_EXIT_NO_MISSION
    print("종합: GATE CONVERGED — 4자 수렴. 커밋+SESSION_STATE 갱신 후 다음 로드맵 단계를 "
          "자동 착수하라(앵커6 축1 — denylist 해당 시에만 정지).")
    # (RSI 자율추천 ii) 종료 게이트 — slow 작업 수렴(종료) 시 '더 나은 방법' 학습 1회 추천
    # (추천만·사람 승인·directive §4). gate-status는 폴링되므로 (task,round)당 1회 마커로 스팸 차단.
    _recommend_learn_once("gate", "%s R%d 종료 — 더 나은 방법론" % (args.task, rnd),
                          "gate-%s-%d" % (re.sub(r"[^A-Za-z0-9]+", "-", args.task), rnd))
    return 0


def _recommend_learn_once(reason, topic, marker_key):
    """RSI 학습 자율추천(best-effort) — marker_key당 1회 feed 추천(추천까지만 자율·착수 사람 승인·
    directive §4). cys 부재·데몬 미가동·오류·중복 마커는 무시(추천은 비핵심·핵심 판정 불간섭)."""
    import shutil
    learn_dir = os.path.join(pack_dir(), "round", "learn")
    marker = os.path.join(learn_dir, ".rec_" + marker_key)
    if os.path.exists(marker) or not shutil.which("cys"):
        return
    body = ('{"reason":"%s","topic":"%s","status":"awaiting_approval"} — '
            "feed 패널 또는 'cys feed reply <id> allow'로 승인 시에만 학습 착수. directive §4: 추천까지만 자율." % (reason, topic))
    try:
        os.makedirs(learn_dir, exist_ok=True)
        r = subprocess.run(["cys", "feed", "push", "--kind", "learn_proposal",
                            # 제목 포맷은 cysd RPC 생산자(handlers.rs learn_proposal)와 동일 규격 유지.
                            "--title", "[RSI 학습 추천] %s — %s" % (reason, topic), "--body", body],
                           capture_output=True, timeout=5)
        if r.returncode == 0:
            open(marker, "w").close()
    except Exception:
        pass


def next_action_items(text):
    """SESSION_STATE '## 다음 액션' 섹션의 **미완 항목 전량**(순서 보존) — 순수 함수.

    지원 형식: 'N. 항목' 번호 목록 · '- [ ] 항목' 체크박스 · '- 항목' 불릿.
    제외: '(없음)' 류 빈 표시 · 완료 체크(- [x]).

    ★섹션 종료 경계 = 다음 `## ` 헤딩 **또는 예약 블록**(`<!-- CYS:RESERVED:`).
      종전엔 `## ` 만 경계여서, 팩 기본 템플릿의 예약 블록
          <!-- CYS:RESERVED:restore_pointer __CYS__RESERVED__ -->
          - 복원 포인터: (없음)
      이 **큐 항목으로 계수**됐다. 실효가 치명적이었다 — 큐가 `1. (없음)`(=갓 설치·전 작업
      완료)인 상태에서도 `extract_next_action` 이 `'복원 포인터: (없음)'` 을 반환해 **exit 0
      (=자율 착수 가)** 이 났다. 즉 **한 번도 쓰지 않은 SESSION_STATE 로도 자율주행이 시동**됐다
      (T1 2026-08-01 검증 중 실측 발견 · 아래 self-test 로 박제).
      ※ `- 복원 포인터: (없음)` 자체는 '없음' 빈-표시 패턴에 걸리지 않는다 — 접두어가 붙어 있어
        `^[(]?\\s*없음` 매칭이 성립하지 않기 때문이다. 그래서 필터가 아니라 **경계**로 고쳐야 한다.
    """
    m = re.search(r"(?m)^##\s*다음 액션[^\n]*\n(.*?)(?:\n##\s|\n<!--\s*CYS:RESERVED:|\Z)",
                  text, re.S)
    if not m:
        return []
    out = []
    for ln in m.group(1).splitlines():
        s = ln.strip()
        if not s:
            continue
        item = None
        nm = re.match(r"^\d+\.\s+(.*)$", s)
        if nm:
            item = nm.group(1).strip()
        elif s.startswith("- "):
            item = s[2:].strip()
        if item is None:
            continue
        # 번호·불릿 공통: 체크박스 완료([x])는 건너뛰고 미완([ ])은 마커를 벗긴다 —
        # "1. [x] 끝난 일"이 다음 액션으로 반환되면 완료 작업 재실행 루프가 된다(6차 R1).
        if item.lower().startswith("[x]"):
            continue
        if item.startswith("[ ]"):
            item = item[3:].strip()
        # 빈 표시: '없음' 단독 또는 괄호/구두점 부가 설명만 빈 칸이다 — "없음 처리 로직
        # 구현" 같은 실제 과제명은 빈 칸이 아니다(시작-매칭 과확장 차단, 6차 R2).
        if item and not re.match(r"^[\(（]?\s*없음\s*[\)）.。\s]*([\(（].*)?$", item):
            out.append(item)
    return out


def extract_next_action(text):
    """첫 미완 항목 또는 None(구 계약 보존 — 소비자 다수)."""
    items = next_action_items(text)
    return items[0] if items else None


def _mission_gate():
    """(exit_code, verdict) — 판정의 단일 소유자는 `javis_mission.gate()` 다(사본 금지).
    모듈이 없으면 **fail-closed**: 임무 없음으로 접는다(팩 스큐가 자율주행을 열지 않는다).

    ★R4 탐지 가능성(2026-08-02): verdict 의 `anomalies` 를 **판정과 무관하게 항상** stderr 로
      흘린다. 동일 UID 의 원장 삭제·절단·창 축소 시도는 원리적으로 차단할 수 없으므로
      (보장 범위 SOT: docs/THREAT-MODEL-mission-gate.md), 남은 무기는 흔적이 master 눈에
      **반드시 닿는 것**이다. master 가 실제로 게이트를 보는 지점이 여기(next-action·gate-status)
      이므로 `javis_mission status` 에만 찍고 끝내면 아무도 안 본다. 은폐는 규약 위반이다.
    """
    try:
        import javis_mission as _m
        rc, v = _m.gate()
    except Exception as e:
        return 2, {"have_mission": False, "mission": None,
                   "reason": "javis_mission 미적재(%s) — fail-closed" % e}
    for _a in (v.get("anomalies") or []):
        print("[mission] ★이상징후(%s): %s — 오너에게 보고하라(은폐 금지)"
              % (_a.get("code"), _a.get("detail")), file=sys.stderr)
    return rc, v


def cmd_next_action(args):
    # ★exit 계약 v2 (2026-08-01 윈도우 실사고 T1 — 임무 게이트 신설):
    #   0 = 다음 액션 있음 **그리고** 이 세션에 오너 임무 지정이 있다 → 자율 착수 가
    #   1 = 빈 큐(전 작업 완료 — 정지·오너 보고)
    #   2 = SESSION_STATE 부재(신규 시작 — 오너 지시를 기다린다)
    #   3 = 큐에 항목은 있으나 **임무 미지정** → 자율 착수 금지. "대기 중인 작업 N건이 있습니다.
    #       이어서 하시겠습니까?"로 **보고하고 멈춘다**.
    # 왜 3이 필요한가: 구 계약은 exit 0(항목 있음)만 보고 달렸다. 그런데 큐는 master 자신이 쓴
    # SESSION_STATE 다 — 산출자가 자기 산출물로 착수 권한을 발급하는 자기인가였다. 오너가 임무를
    # 주지 않은 부팅에서 **이전 세션 잔무 큐**를 집어 무한 작업에 들어간 실사고의 직접 원인이다.
    # 이전 세션 잔무는 **보고 대상**이지 자동 착수 대상이 아니다.
    p = os.path.join(pack_dir(), "round", "SESSION_STATE.md")
    try:
        text = open(p, encoding="utf-8", errors="replace").read()
    except OSError:
        print("[next-action] SESSION_STATE 없음(신규 시작): %s — 오너 지시를 기다려라."
              % p, file=sys.stderr)
        return 2
    items = next_action_items(text)
    item = items[0] if items else None
    if item is None:
        print("[next-action] 다음 액션 큐 비어 있음 — 전 작업 완료. 자율 루프 정지·오너 보고.",
              file=sys.stderr)
        return 1
    mrc, mv = _mission_gate()
    if mrc != 0:                                  # 1=임무 없음 · 2=판독 불가(둘 다 착수 금지)
        n = len(items)
        # stdout 은 **오너에게 그대로 읽어줄 보고 문안**이다(master가 문장을 지어내지 않게).
        print("대기 중인 작업 %d건이 있습니다. 이어서 하시겠습니까? (첫 항목: %s)" % (n, item))
        print("[next-action] 임무 미지정 — 자율 착수 금지(exit 3). 사유: %s"
              % mv.get("reason"), file=sys.stderr)
        # ★적발 (b) 수리(2026-08-01 R2): 이 안내문의 유일한 독자는 **방금 차단당한 master 본인**
        #   이다. 여기에 `javis_mission.py set` 을 적어 두면 게이트가 자기 우회법을 가르치는 것이
        #   된다(그 명령은 이제 `cys feed push --wait` 오너 승인에 결박됐지만, 안내 자체를 두면
        #   "일단 쳐 보라"는 유인이 남는다). 해제 경로는 **오너 채널 하나**로만 안내한다.
        print("  이 큐는 이전 세션의 잔무다. **보고 대상이지 자동 착수 대상이 아니다.**\n"
              "  지금 할 일: 위 stdout 한 줄을 오너에게 그대로 보고하고 **멈춘다**.\n"
              "  해제는 오너가 이 세션에 임무를 말할 때만 일어난다(UserPromptSubmit 훅이 자동 기록).",
              file=sys.stderr)
        return 3
    print(item)
    return 0


def cmd_silent_failure_catalog(args):
    # exit 계약: 0=재생성 완료 또는 (--check) 정합 / 1=(--check) 드리프트(파일 부재·SILENT_FAILURES와 불일치).
    # 카탈로그는 런타임 아티팩트(pack/round/) — 재컴파일 무관, 테이블 행 변경만 cargo build(CSO).
    rendered = render_catalog()
    p = os.path.join(pack_dir(), "round", "SILENT_FAILURE_CATALOG.md")
    if getattr(args, "check", False):
        if not os.path.isfile(p):
            print("[silent-failure-catalog] 드리프트: 카탈로그 파일 없음 — 재생성 필요: %s" % p,
                  file=sys.stderr)
            return 1
        on_disk = open(p, encoding="utf-8", errors="replace").read()
        if on_disk != rendered:
            print("[silent-failure-catalog] 드리프트: 디스크 카탈로그가 SILENT_FAILURES와 불일치 — "
                  "재생성 필요: %s" % p, file=sys.stderr)
            return 1
        print("[silent-failure-catalog] 정합: %d개 항목 (SILENT_FAILURES 파생)" % len(SILENT_FAILURES))
        return 0
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8") as f:
        f.write(rendered)
    print("[silent-failure-catalog] 재생성: %s (%d개 항목)" % (p, len(SILENT_FAILURES)))
    return 0


def cmd_channel_health(args):
    """AGENTREACH OPP-02 — 콘텐츠 채널 per-channel 헬스(노드 헬스 check 의 짝).
    javis_channels.py 를 subprocess 호출(사람판=silence-first·기계판=--json). check 가
    부트 노드(cso/worker/agy/codex) 생존을, channel-health 가 콘텐츠 채널 도달성을 본다."""
    tool = os.path.join(pack_dir(), "bin", "javis_channels.py")
    if not os.path.isfile(tool):
        print("[channel-health] javis_channels.py 부재 — `cys init-pack`", file=sys.stderr)
        return 2
    chans = list(getattr(args, "channels", []) or [])
    flag = "--json" if getattr(args, "json", False) else "--silence-first"
    r = subprocess.run([sys.executable, tool, flag] + chans)
    return r.returncode


# ── guard-master-claim: misrouted-master 부트 가드 (Fix 2'·결정론·이중방어) ──
# 공유 데몬에 2번째 master를 선언하는 잔여 경로(수동 claim-role·명령팔레트)에 대한 이중방어.
# 데몬 내부의 privileged-role 점유 차단(cysd handlers.rs)이 1차 방어, 이 명령이 2차(부트 전 선검사).
def _surface_id_env():
    """내 surface id 문자열 반환(없으면 None). cys::env_compat 우선순위(AITERM_*→JAVIS_*→CYS_*)와 정합 —
    AITERM_SURFACE_ID를 먼저 보되, cysd가 실제 주입하는 CYS_SURFACE_ID(src/lib.rs ENV_SURFACE_ID)도 인식한다.
    셋 다 미설정(외부 셸 세션)이면 None → 호출부가 PASS(부팅 차단 회귀 방지·gemini D2)."""
    for k in ("AITERM_SURFACE_ID", "JAVIS_SURFACE_ID", "CYS_SURFACE_ID"):
        v = os.environ.get(k, "").strip()
        if v:
            return v
    return None


def _parse_ref(s):
    """'surface:31' 또는 '31' → 31(int). 파싱불가 → None."""
    s = (s or "").strip()
    if s.startswith("surface:"):
        s = s[len("surface:"):]
    try:
        return int(s)
    except (TypeError, ValueError):
        return None


def _cys_list_masters():
    """현재 데몬(상속 CYS_SOCKET)의 live(미exited) role=master surface id 리스트. cys list 실패 시 None.
    cys list 라인 형식: '{surface_ref}\\trole={role}\\tpid={pid}\\texited={bool}\\t{title}\\t{cwd}'."""
    cys = shutil.which("cys")
    if not cys:
        return None
    try:
        r = subprocess.run([cys, "list"], capture_output=True, timeout=10)
    except Exception:
        return None
    if r.returncode != 0:
        return None
    masters = []
    for line in r.stdout.decode("utf-8", "replace").splitlines():
        f = line.split("\t")
        if len(f) < 4:
            continue
        role = f[1][5:] if f[1].startswith("role=") else ""
        exited = f[3].strip().endswith("true")
        if role == "master" and not exited:
            sid = _parse_ref(f[0])
            if sid is not None:
                masters.append(sid)
    return masters


def guard_master_verdict(my_env, masters):
    """순수 판정(cys 의존 없음·self-test 핀). 반환 (code, kind).
    my_env=내 surface id 문자열(None=미설정) · masters=live master id 리스트(None=cys list 실패).
    ★결정론·false-block 회귀 금지(gemini D2): 미설정/파싱불가/조회실패는 전부 PASS(0). 오직 '내 id가
    유효하고 다른 유효 master가 존재'할 때만 MISROUTED(9)."""
    if my_env is None:
        return 0, "unset"
    my_id = _parse_ref(my_env)
    if my_id is None:
        return 0, "unparsed"        # set-but-malformed → 보수적 PASS(false-block 금지)
    if masters is None:
        return 0, "list_fail"       # 결정론 신호 부재 → 보수적 PASS
    if [m for m in masters if m != my_id]:
        return 9, "misrouted"
    return (0, "idempotent") if my_id in masters else (0, "no_master")


def cmd_guard_master_claim(args):
    """claim-role master 직전 결정론 선검사. AITERM/CYS_SURFACE_ID 미설정(외부 셸)→PASS(0).
    설정 시 내 surface 와 다른 live master 보유자가 있으면 MISROUTED_MASTER + exit 9.
    보유자=나(멱등)·master 부재→PASS(0)."""
    my_env = _surface_id_env()
    masters = _cys_list_masters() if (my_env is not None and _parse_ref(my_env) is not None) else None
    code, kind = guard_master_verdict(my_env, masters)
    if kind == "unset":
        print("[guard-master-claim] surface id env 미설정(외부 셸 세션) — PASS(부팅 차단 회귀 방지)")
    elif kind == "unparsed":
        print("[guard-master-claim] surface id env 파싱불가(%r) — PASS(false-block 회귀 방지)" % my_env)
    elif kind == "list_fail":
        print("[guard-master-claim] cys list 미수집(데몬 미응답?) — PASS(부팅 차단 회귀 방지)")
    elif kind == "misrouted":
        my_id = _parse_ref(my_env)
        holder = next(m for m in masters if m != my_id)
        print("MISROUTED_MASTER: 이 surface(surface:%d)는 이미 master(surface:%d)가 있는 공유 데몬에 떴습니다. "
              "2번째 master 선언 금지 — 격리된(전용 데몬) master 워크스페이스로 옮겨 다시 선언하세요." % (my_id, holder))
    elif kind == "idempotent":
        print("[guard-master-claim] 내가 이미 master 보유자(surface:%d) — 멱등 PASS" % _parse_ref(my_env))
    else:  # no_master
        print("[guard-master-claim] live master 부재 — claim 허용(PASS)")
    return code


def cmd_self_test(args):
    """순수 로직 자기검증 (cys 의존 없음) — preflight C19가 호출. assert 실패는 exit 1."""
    try:
        assert REQUIRED_ROLES == ["cso", "worker", "reviewer-gemini", "reviewer-codex"], \
            "4종 의무 노드 목록이 변형됐다"
        assert MAX_ROUNDS == 10, "라운드 상한은 10이어야 한다(앵커4 5-8)"
        # round_path 경로 탈출 방지: 악성 task가 round 디렉터리 밖으로 못 나간다(실효 검증).
        rnd_dir = os.path.realpath(os.path.join(pack_dir(), "round"))
        for evil in ("../../etc/passwd", "a/b ../x:일", "..\\..\\win", "/abs/x"):
            ep = os.path.realpath(os.path.dirname(round_path(evil)))
            assert ep == rnd_dir, "round_path 경로 탈출: %s → %s" % (evil, ep)
            assert os.sep not in os.path.basename(round_path(evil)).replace(
                "ORCHESTRATION-", "").replace(".md", "").replace("_", ""), "basename 분리자 잔존"
        # review-prompt 생성: 제약·형식이 항상 포함된다(폴백 포함)
        class _A:
            task, scope, reviewer, round = "T", "S", None, 2
        import io
        import contextlib
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            cmd_review_prompt(_A())
        out = buf.getvalue()
        # ★전환 게이트 §9-7-2: 고정 향상률 리터럴의 "존재 필수" 검사를 새 기준으로 교체.
        for must in ("엄격 제약", "배회 금지", "문제점", "회신",
                     "잠근 합격 기준의 미달 항목 0"):
            assert must in out, "review-prompt에 '%s' 누락" % must
        # T1(attention-p0) 밀폐 검증: 기각 재주입·불변식 — env 격리·복원(preflight C19 호출 안전)
        import tempfile as _tf
        _prev_root = os.environ.get("JAVIS_ROOT")
        try:
            with _tf.TemporaryDirectory() as _td:
                os.environ["JAVIS_ROOT"] = _td
                # (a) 소스 부재 = 주입 0 (회귀 0)
                _b0 = io.StringIO()
                with contextlib.redirect_stdout(_b0):
                    cmd_review_prompt(_A())
                _o0 = _b0.getvalue()
                assert "기각" not in _o0 and "불변식" not in _o0, "소스 부재인데 주입 발생(회귀)"
                # (b) 양형식 fixture → 수확·면역 판정·커버리지
                _hd = os.path.join(_td, "_round", "handoffs")
                os.makedirs(_hd)
                open(os.path.join(_hd, "a-gate.md"), "w", encoding="utf-8").write(
                    "# a-gate (2026-07-13 · master)\n## Rejected\n대안X | 이유Y\n## Files\n없음\n")
                open(os.path.join(_hd, "b-done.md"), "w", encoding="utf-8").write(
                    "# b-done (2026-07-13 · 워커)\n**Rejected**: 대안Z 기각.\n")
                _k, _c = harvest_rejected(_hd)
                assert _c["files"] == 2 and _c["with_rejected"] == 2, "양형식 수확 실패: %s" % _c
                _imm = {i["src"]: i["immune"] for i in _k}
                assert _imm.get("a-gate") is True and _imm.get("b-done") is False, \
                    "면역 판정 오류(A1): %s" % _imm
                # (c) 주입문·역할 차등(A2)
                _b1 = io.StringIO()
                with contextlib.redirect_stdout(_b1):
                    cmd_review_prompt(_A())
                assert "이미 기각된 지적" in _b1.getvalue() and "커버리지" in _b1.getvalue(), \
                    "reviewer1 주입 누락"

                class _A2(_A):
                    reviewer_role = "reviewer2"
                _b2 = io.StringIO()
                with contextlib.redirect_stdout(_b2):
                    cmd_review_prompt(_A2())
                assert "감사 자료" in _b2.getvalue(), "reviewer2 차등 주입 누락"
                # (d) 불변식 로드·주입 + 부재 시 None
                assert load_invariants(os.path.join(_td, "없음.md")) is None
                open(os.path.join(_td, "_round", "INVARIANTS.md"), "w", encoding="utf-8").write(
                    "# 불변식\n- 이 파일은 런타임에 불변이다.\n")
                _b3 = io.StringIO()
                with contextlib.redirect_stdout(_b3):
                    cmd_review_prompt(_A())
                assert "프로젝트 불변식" in _b3.getvalue(), "불변식 주입 누락"
        finally:
            if _prev_root is None:
                os.environ.pop("JAVIS_ROOT", None)
            else:
                os.environ["JAVIS_ROOT"] = _prev_root
        # live_roles 파싱
        lr = live_roles({"surfaces": [
            {"role": "cso", "agent_alive": True},
            {"role": "worker", "agent_alive": False},
        ]})
        assert lr == {"cso": True}, "live_roles 파싱 오류"
        # round-log 표 셀 새니타이즈: 파이프·개행이 제거된다
        assert _cell("a|b\nc") == "a/b c", "_cell 새니타이즈 오류"
        # task-prompt 티켓(밀폐 — rules 명시 주입, 설치본 디렉티브 상태와 무관):
        # 절대 강조 4규칙·게이트·todo(pack 앵커)·보고 채널이 항상 포함된다
        # ★W14 S14 — 종전 필수 토큰 `"${CYS_PACK_DIR"`는 **뺐다**. todo 경로를 워커 셸에서 늦게
        #   전개되는 문자열로 두는 것이 바로 이번에 없앤 이원 바인딩이다(경로=워커 시점 /
        #   선언 scope=master 시점 → foreign-scope 오배제 → false QUIET). pack 앵커 보장은
        #   아래 S14 블록이 **발부 시점 절대경로**(`_pack_identity()` + WORKER_TODO.md)로
        #   더 강하게 핀한다 — 토큰 존재 검사는 그 불변식과 정면으로 모순된다.
        ticket = build_task_ticket("T", "S", "C", "worker", rules=FALLBACK_RULES)
        for must in ("절대 강조 4규칙", "품질 절대우선", "할루시네이션 방지",
                     "hallucination-guard", "grill-me", "요약·압축 절대 금지", "게이트",
                     "성공 기준", "WORKER_TODO.md", "보고 채널",
                     "--queued", "완료 증거(E1 evidence-artifact 게이트", "--evidence-artifact",
                     "done 증거 게이트(P3)"):
            assert must in ticket, "task-prompt 티켓에 '%s' 누락" % must
        # P3 필수 probe 블록: probes 미지정이면 부재(하위호환·E1 블록은 그대로), 지정 시 목록·actprobe 명령
        assert "필수 probe" not in ticket, "probes 미지정인데 필수 probe 블록 존재(하위호환 위반)"
        tp_probe = build_task_ticket("T", "S", "C", "worker", rules=FALLBACK_RULES,
                                     probes=["submit", "artifact"])
        assert "필수 probe" in tp_probe and "submit, artifact" in tp_probe \
            and "javis_actprobe.py" in tp_probe, "probes 지정인데 필수 probe 블록 누락"
        # ★--task 동반(R1 major-a) + relaxed 경고(무-task 영수증 거부 회귀 배선)
        assert "--task <task-id>" in tp_probe, "probe 예시에 '--task <task-id>' 동반 누락(무-task 영수증 거부 회귀)"
        assert "relaxed probe" in tp_probe, "relaxed probe --task 경고 누락"
        # E1·P3 공존: probes 지정 시에도 E1 블록이 함께 존재(둘 다 명시)
        assert "완료 증거(E1 evidence-artifact 게이트" in tp_probe and "done 증거 게이트(P3)" in tp_probe, \
            "E1·P3 evidence 게이트 공존 실패"
        # 폴백 단독으로도 4규칙 마커 전부를 갖는다(디렉티브 부재 환경의 최후 방어선)
        fb = "\n".join(FALLBACK_RULES)
        for mark in RULE_MARKERS:
            assert mark in fb, "FALLBACK_RULES에 마커 '%s' 누락" % mark
        # --success 생략 시 성공 기준 라인이 사라진다(빈 값 주입 금지)
        assert "성공 기준" not in build_task_ticket("T", "S", None, "worker",
                                                  rules=FALLBACK_RULES), \
            "success 미지정인데 성공 기준 라인 존재"
        # todo 파일명은 역할명 대문자 변환(reviewer-gemini → REVIEWER_GEMINI_TODO.md)
        assert "REVIEWER_GEMINI_TODO.md" in build_task_ticket(
            "T", "S", None, "reviewer-gemini", rules=FALLBACK_RULES), "todo 파일명 역할 변환 오류"
        # ★todo 선언 v1(설계 §4-1) — 티켓이 문법 위반 선언을 배포하면 파서가 전건 미선언으로
        # 버려 배선 자체가 무의미해진다. 문법을 여기서 기계로 못박는다(손기재 오류 원천 차단).
        decl, why = todo_decl_line("reviewer-gemini", "유령 todo 결함 수정 (ghost fix)")
        assert decl is not None, "선언 생성 실패: %s" % why
        m_decl = re.fullmatch(r"<!-- javis:todo (v\d+) (.+) -->", decl)
        assert m_decl and m_decl.group(1) == "v1", "선언 접두·버전 토큰 형식 오류: %r" % decl
        toks = dict(t.split("=", 1) for t in m_decl.group(2).split())
        for k in ("owner", "scope", "status"):                      # G5 필수 키 3종
            assert k in toks, "선언 필수 키 '%s' 누락: %r" % (k, decl)
        for k, v in toks.items():                                   # G4 키·값 문법
            assert re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*", k), "선언 키 문법 위반: %r" % k
            assert re.fullmatch(r"[A-Za-z0-9._:-]+", v), "선언 값 문법 위반: %s=%r" % (k, v)
        assert toks["owner"] == "reviewer-gemini" and toks["status"] == "active", decl
        assert re.fullmatch(r"\d{4}-\d{2}-\d{2}", toks.get("since", "")), "since 날짜 형식 오류: %r" % decl
        # lane은 한글·공백을 지운 슬러그(G4 위반 방지) — 남는 게 없으면 키 자체를 생략한다
        assert toks.get("lane") == "todo-ghost-fix", "lane 슬러그 정규화 오류: %r" % decl
        assert "lane=" not in todo_decl_line("worker", "유령 결함")[0], \
            "슬러그가 빈 lane을 빈 값으로 배출(선언 전체 무효화 위험)"
        # ★W14 S14 — 정체성 값은 **접지 않는다**. 팩 이름이 G4 밖이면 그럴듯한 폴백을
        # 만들지 말고 시끄럽게 실패해야 한다(스탬프 도구와 같은 정책으로 수렴).
        bad_line, bad_why = todo_decl_line("worker", "t", scope="자비스")
        assert bad_line is None and bad_why, "G4 밖 scope를 조용히 접어 배포했다"
        assert todo_decl_line("워커", "t", scope="pack")[0] is None, "G4 밖 owner를 접어 배포했다"
        # 경로와 scope는 **같은 바인딩**에서 나온다 — 티켓의 todo 경로는 발부 시점 절대경로다.
        _pk_root, _pk_scope = _pack_identity()
        _tk = build_task_ticket("T", "S", None, "worker", rules=FALLBACK_RULES)
        assert os.path.join(_pk_root, "round", "WORKER_TODO.md") in _tk, \
            "티켓 todo 경로가 발부 시점 절대경로가 아니다(늦은 전개 = scope 이원 바인딩)"
        assert "scope=%s" % _pk_scope in _tk, "티켓 선언 scope가 발부 시점 팩과 다르다"
        assert "${CYS_PACK_DIR" not in _tk.split("todo 영속:")[1].split("\n")[0], \
            "todo 경로가 여전히 워커 셸에서 늦게 전개된다(S14 재발)"
        # 티켓 동봉 확인은 날짜 리터럴을 비교하지 않는다(자정 경계에서 since가 갈리는 위조 flake 방지)
        assert "<!-- javis:todo v1 owner=worker scope=" in build_task_ticket(
            "T", "S", None, "worker", rules=FALLBACK_RULES), "티켓에 todo 선언 한 줄 누락"
        # 추출기(순수 함수) 배터리 — 합성 디렉티브 텍스트로 밀폐 검증:
        synth = ("# W\n\n## 7. ★절대 강조 4규칙 — x\n머리말.\n"
                 + "\n".join(FALLBACK_RULES) + "\n\n## 8. 다음\n- 무관\n")
        got = extract_rules_from_text(synth)
        assert got and len(got) == len(FALLBACK_RULES), "추출 개수 불일치(머리말 혼입?)"
        # (a) 절 번호가 3이 아니어도 추출된다(번호 하드코딩 금지)
        # (b) 멀티라인 wrap: 불릿을 두 줄로 쪼개도 연속줄 합류로 마커가 보존된다
        wrapped = synth.replace("몽상·망상을 촉진하는 말 절대 금지.",
                                "\n  몽상·망상을 촉진하는 말 절대 금지.")
        gw = extract_rules_from_text(wrapped)
        assert gw and "몽상" in "\n".join(gw), "연속줄 합류 실패 — wrap 잘림"
        # (c) 약화된 디렉티브(마커 소실)는 추출 거부 → 폴백 강등(전파 차단)
        assert extract_rules_from_text(synth.replace("Garbage-in", "")) is None, \
            "약화 디렉티브가 추출을 통과(전파 위험)"
        # (d) 섹션 부재 → None
        assert extract_rules_from_text("# 없음\n## 1. 다른 절\n- x\n") is None, \
            "무관 텍스트에서 추출 오탐"
        # 자율주행(앵커6) — gate_verdicts 순수 배터리: 4자 수렴/누락/미승인/재평가 우선
        rows = [{"round": 1, "evaluator": "gemini", "score": "9", "verdict": "PASS 95"},
                {"round": 1, "evaluator": "codex-r1", "score": "9", "verdict": "수렴"},
                {"round": 1, "evaluator": "master", "score": "-", "verdict": "approve"},
                {"round": 1, "evaluator": "machine:cargo", "score": "159", "verdict": "green"}]
        assert all(gate_verdicts(rows, 1).values()), "4자 전원 승인인데 미수렴 판정"
        assert gate_verdicts(rows[:3], 1)["machine"] is None, "machine 누락 미검출"
        rows2 = rows + [{"round": 1, "evaluator": "codex", "score": "5", "verdict": "반려"}]
        assert gate_verdicts(rows2, 1)["codex"] is False, "재평가(마지막 기록 우선) 미반영"
        assert gate_verdicts(rows, 2) == {e: None for e in GATE_EVALUATORS}, \
            "다른 라운드 기록이 새 라운드에 새는 오염"
        # ★3-state 게이트(D5): SKIP을 None(누락)·False(미승인)·True(승인)와 구분
        assert skip_reason("SKIPPED: 호스트 오프라인") == "호스트 오프라인", "skip 사유 추출 실패"
        assert skip_reason("스킵: 사유") == "사유" and skip_reason("건너뜀: x") == "x", "한국어 skip 미인식"
        assert skip_reason("SKIPPED:") is None, "빈 사유 skip 인정(fail-closed 위반)"
        assert skip_reason("수렴") is None and skip_reason("반려") is None, "비-skip을 skip으로 오인"
        rows_sk = rows[:3] + [{"round": 1, "evaluator": "machine", "score": "-",
                               "verdict": "SKIPPED: 머신 평가 호스트 다운"}]
        gv = gate_verdicts(rows_sk, 1)
        assert isinstance(gv["machine"], Skip), "SKIP이 Skip으로 안 잡힘(verdict_approved에 먼저 삼켜짐)"
        assert gv["machine"] is not False and gv["machine"] is not None, "SKIP이 False/None과 혼동"
        assert gv["machine"].reason == "머신 평가 호스트 다운", "Skip 사유 보존 실패"
        # 사유에 'failed'가 있어도 SKIP은 미승인이 아니다(가로채기가 verdict_approved보다 먼저)
        assert isinstance(gate_verdicts(rows[:3] + [{"round": 1, "evaluator": "machine",
               "score": "-", "verdict": "SKIPPED: cargo build failed host"}], 1)["machine"], Skip), \
            "사유에 fail 포함 시 SKIP이 미승인으로 오분류"
        # ★부정 verdict 차단(6차 R1 HIGH-1): 한국어 부정 접미·영문 부정이 승인으로 새면
        # 가짜 GATE CONVERGED로 자율 전진한다 — 전부 False여야 한다.
        for neg in ("수렴 실패", "수렴 미달", "승인 불가", "승인 보류", "승인 거부",
                    "ok지만 반려", "pass 불가", "green 아님", "approve 거부", "PASS fail",
                    "ok — not yet", "미승인"):
            assert verdict_approved(neg) is False, "부정 verdict '%s'가 승인 오판" % neg
        for pos in ("PASS 95점", "수렴", "approve", "green", "승인."):
            assert verdict_approved(pos) is True, "정상 승인 '%s'가 거부 오판" % pos
        # ★평가자 구분자 강제(6차 R1 LOW-7): 가짜 접두는 무시, 구분자 변형은 인정
        assert evaluator_std("masterful-bot") is None, "'masterful' 오탐"
        assert evaluator_std("machinelearning") is None, "'machinelearning' 오탐"
        assert evaluator_std("machine:pytest") == "machine" and \
            evaluator_std("codex-r1") == "codex" and evaluator_std("gemini") == "gemini", \
            "정상 평가자 변형 매칭 실패"
        # 표기 이주 별칭: agy(Antigravity CLI) 기록도 표준 gemini로 — 'agycorp' 류는 거부
        assert evaluator_std("agy") == "gemini" and evaluator_std("agy:r2") == "gemini", \
            "agy 별칭 매핑 실패"
        assert evaluator_std("agycorp") is None, "'agycorp' 오탐"
        # 자율주행(앵커6) — extract_next_action 순수 배터리
        ss = ("# S\n## 다음 액션 큐\n1. (없음)\n\n## 기타\n- x\n")
        assert extract_next_action(ss) is None, "'(없음)' 빈 큐 오탐"
        ss2 = "# S\n## 다음 액션 큐\n1. 6차 블록 검증\n2. 다음\n"
        assert extract_next_action(ss2) == "6차 블록 검증", "번호 목록 첫 항목 추출 실패"
        ss3 = "# S\n## 다음 액션\n- [x] 끝난 일\n- [ ] 남은 일\n"
        assert extract_next_action(ss3) == "남은 일", "체크박스 미완 항목 추출 실패"
        assert extract_next_action("# S\n## 다른 절\n- x\n") is None, "섹션 부재 오탐"
        # ★번호+체크박스 혼용(6차 R1 MED-3): 완료([x])는 건너뛰고 미완([ ])은 마커 제거
        ss4 = "# S\n## 다음 액션 큐\n1. [x] 끝난 일\n2. [ ] 남은 일\n"
        assert extract_next_action(ss4) == "남은 일", "번호+[x] 완료 항목이 액션으로 반환"
        # ★'없음' 변형(6차 R1): 전각 괄호·부가 설명도 빈 칸이다
        for empty in ("1. （없음）\n", "1. 없음 (전 작업 완료)\n", "- (없음).\n"):
            assert extract_next_action("# S\n## 다음 액션 큐\n" + empty) is None, \
                "'없음' 변형 '%s'가 액션으로 반환" % empty.strip()
        # ★'없음' 시작-매칭 과확장 차단(6차 R2): "없음 처리 로직" 같은 실제 과제는 빈 칸 아님
        assert extract_next_action("# S\n## 다음 액션 큐\n1. 없음 처리 로직 구현\n") \
            == "없음 처리 로직 구현", "'없음'으로 시작하는 실제 과제가 silent skip"
        # ★T1(2026-08-01) 회귀 핀 — **예약 블록은 큐가 아니다**.
        #   구 코드는 섹션 경계가 `## ` 뿐이라, 팩 기본 템플릿의
        #   `<!-- CYS:RESERVED:restore_pointer -->` / `- 복원 포인터: (없음)` 을 큐 항목으로 읽었다.
        #   실효: 큐가 `1. (없음)`(갓 설치·전 작업 완료)인데도 exit 0(자율 착수 가)이 났다 —
        #   **한 번도 쓰지 않은 SESSION_STATE 로 자율주행이 시동**되는 경로였다.
        ss_res = ("# S\n## 다음 액션 큐\n1. (없음)\n\n"
                  "<!-- CYS:RESERVED:restore_pointer __CYS__RESERVED__ -->\n"
                  "- 복원 포인터: (없음)\n"
                  "<!-- /CYS:RESERVED:restore_pointer -->\n")
        assert extract_next_action(ss_res) is None, \
            "예약 블록(복원 포인터)이 다음 액션으로 반환 — 빈 큐가 자율 착수로 오판(T1 회귀)"
        # 팩 동봉 템플릿 실물로도 확인한다(문서와 코드가 같이 늙지 않게 · 부재 시 건너뜀)
        _tpl = os.path.join(pack_dir(), "round", "SESSION_STATE.md")
        if os.path.isfile(_tpl):
            _t = open(_tpl, encoding="utf-8", errors="replace").read()
            if "1. (없음)" in _t:
                assert extract_next_action(_t) is None, \
                    "팩 기본 SESSION_STATE 템플릿이 빈 큐인데 액션을 반환한다(T1 회귀)"
        # 계수는 추출과 **같은 필터**를 쓴다(보고 숫자와 착수 판정이 갈리지 않게)
        assert len(next_action_items(ss2)) == 2, "미완 항목 계수 불일치"
        assert len(next_action_items(ss4)) == 1, "완료([x]) 항목이 계수에 포함"
        # (e) 핀↔마커 패리티: 마커 소실로 폴백 강등될 때 안내하는 preflight C03(WORKER 핀)이
        # 같은 소실을 검출할 수 있어야 진단 루프가 닫힌다. javis_preflight가 같은 bin에
        # 있을 때만 검사(없는 환경에서는 자기 검증 불가 — 건너뜀).
        pf_path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                               "javis_preflight.py")
        if os.path.isfile(pf_path):
            import importlib.util
            spec = importlib.util.spec_from_file_location("_pf_parity", pf_path)
            _pf = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(_pf)
            worker_pins = [p for p, _ in _pf.CONTENT_PINS.get("WORKER_DIRECTIVE.md", [])]
            for mark in RULE_MARKERS:
                assert any(mark in pin or pin in mark for pin in worker_pins), \
                    "마커 '%s'가 WORKER C03 핀에 비커버 — 폴백 강등 원인을 preflight가 못 본다" % mark
        # ── 리뷰어 감지·무구독 폴백 배터리(2026-06-14 · 밀폐 가짜 감지기) ──
        # 표준 슬롯 계약 고정: agy/codex 네이티브 + claude 대체 2슬롯(변형 시 폴백 붕괴).
        assert [s[1] for s in REVIEWER_SLOTS] == ["gemini", "codex"], "표준 리뷰어 슬롯 변형"
        assert [s[3] for s in REVIEWER_SLOTS] == ["claude", "claude"], "대체 agent 는 claude 여야 함"
        # detect_reviewer: 절대경로 부재·미정의 agent 는 unavailable, 첫토큰만 본다(인자 무시 안 함)
        synth_ag = {"gemini": {"cmd": "/no/such/dir/agy --dangerously-skip-permissions"},
                    "codex": {"cmd": "codex --x"}, "claude": {"cmd": "bash /x/a.sh"}}
        assert detect_reviewer("gemini", synth_ag)[0] is False, "절대경로 부재 바이너리 available 오탐"
        assert detect_reviewer("zzz", synth_ag)[0] is False, "미정의 agent available 오탐"
        assert reviewer_launch_binary("gemini", synth_ag).endswith("/agy"), "cmd 첫토큰 추출 오류"
        # 미감지 → Claude 대체 로스터(멈춤 금지), 감지 → 네이티브 로스터
        no = lambda a, ag=None: (False, "테스트:미설치")
        yes = lambda a, ag=None: (True, "테스트:있음")
        rno = reviewer_roster(detect=no, agents=synth_ag)
        assert [e["role"] for e in rno] == ["reviewer-claude-1", "reviewer-claude-2"], \
            "미감지 시 Claude 대체 로스터 미생성(멈춤 위험)"
        assert all(e["agent"] == "claude" and not e["native"] for e in rno), "대체 슬롯 agent/native 오류"
        assert [e["substituted_for"] for e in rno] == ["gemini", "codex"], "대체 대상 추적 오류"
        ryes = reviewer_roster(detect=yes, agents=synth_ag)
        assert [e["role"] for e in ryes] == ["reviewer-gemini", "reviewer-codex"] and \
            all(e["native"] for e in ryes), "감지 시 네이티브 로스터 오류"
        # 혼합(gemini만 있음): 네이티브 1 + 대체 1
        mix = lambda a, ag=None: (a == "gemini", "mix")
        rmix = reviewer_roster(detect=mix, agents=synth_ag)
        assert [e["role"] for e in rmix] == ["reviewer-gemini", "reviewer-claude-2"], "혼합 로스터 오류"
        # effective_required_roles: 미감지 시 의무 역할이 Claude 대체로 치환(check 가 영영 부재 보고 안 함)
        assert effective_required_roles(detect=no, agents=synth_ag) == \
            ["cso", "worker", "reviewer-claude-1", "reviewer-claude-2"], "유효 의무역할 치환 오류"
        assert effective_required_roles(detect=yes, agents=synth_ag) == REQUIRED_ROLES, \
            "감지 시 유효 의무역할이 표준과 불일치"

        # ── B18: 팀 구성 안내 파생(H-DOC-2) — 리터럴 금지·master 는 required 밖 ──
        assert "master" not in REQUIRED_ROLES, \
            "REQUIRED_ROLES 에 master 가 들어갔다(금지 방향 ② — 레거시 master 부트 사망)"
        _note = team_roster_note()
        assert _note.startswith("master·"), "팀 구성 안내가 master 로 시작하지 않는다"
        assert "총 %d노드" % (len(REQUIRED_ROLES) + 1) in _note, \
            "노드 수가 REQUIRED_ROLES+1 파생이 아니다: %s" % _note
        for _r in REQUIRED_ROLES:
            assert _r in _note, "필수 역할 %s 가 안내에서 누락" % _r
        # 편성이 바뀌면 숫자·역할명이 **따라 움직인다**(사본 드리프트 불가능성 증명)
        _n3 = team_roster_note(required=["cso", "worker"])
        assert "총 3노드" in _n3 and "reviewer" not in _n3, "안내가 편성 변화를 따르지 않는다: %s" % _n3

        # ── 무음실패 카탈로그 배터리 (OpenMontage D5 2부 — render·무점수·드리프트) ──
        sf_ids = [s["id"] for s in SILENT_FAILURES]
        for must in ("SF-GATE-SCORE-FIELD", "SF-DENY-CHARTER-EDIT",
                     "SF-PLAN-DOWNGRADE", "SF-CONSENSUS-AVERAGE"):
            assert must in sf_ids, "필수 무음실패 id 누락: %s" % must
        assert len(sf_ids) == len(set(sf_ids)), "무음실패 id 중복"
        for s in SILENT_FAILURES:
            assert s["kind"] in ("deterministic", "heuristic"), "kind enum 위반: %s" % s["id"]
            assert not any(k in s for k in ("score", "grade", "rating")), \
                "무점수 위반 — 수치 등급 키: %s" % s["id"]
        pd = [s for s in SILENT_FAILURES if s["id"] == "SF-PLAN-DOWNGRADE"]
        assert pd and pd[0]["kind"] == "heuristic", "SF-PLAN-DOWNGRADE는 heuristic이어야 한다"
        cat = render_catalog()
        for sid in sf_ids:
            assert sid in cat, "카탈로그 렌더에 %s 누락" % sid
        assert "deterministic" in cat and "heuristic" in cat, "kind 표기 누락"
        # 무점수 트립와이어 — 알려진 등급 포맷(N/M·N점·0.x) 탐지용이지 망라적 탐지기는 아니다.
        # 구조적 보증은 위의 score/grade/rating 키 부재 + kind enum이 담당한다.
        assert not re.search(r"\d+\s*/\s*\d{1,3}|\b\d+\s*점\b|\b0\.\d+\b", cat), \
            "카탈로그에 수치 등급 토큰(무점수 위반)"
        assert render_catalog() == cat, "render_catalog 비결정론(2회 불일치)"
        # 쓰기·--check 왕복(격리 tempdir — 라이브 pack 미접촉; CYS_PACK_DIR 재지정·복원)
        import tempfile as _tf
        _saved_pd = os.environ.get("CYS_PACK_DIR")
        with _tf.TemporaryDirectory(prefix="javis-orch-sfc-") as _td:
            os.environ["CYS_PACK_DIR"] = _td
            try:
                _sink = io.StringIO()
                with contextlib.redirect_stdout(_sink), contextlib.redirect_stderr(_sink):
                    assert cmd_silent_failure_catalog(argparse.Namespace(check=False)) == 0, "카탈로그 쓰기 exit≠0"
                    _catp = os.path.join(_td, "round", "SILENT_FAILURE_CATALOG.md")
                    assert os.path.isfile(_catp), "카탈로그 파일 미생성"
                    assert cmd_silent_failure_catalog(argparse.Namespace(check=True)) == 0, "정합인데 --check 드리프트"
                    with open(_catp, "a", encoding="utf-8") as _f:
                        _f.write("\n변조행\n")
                    assert cmd_silent_failure_catalog(argparse.Namespace(check=True)) == 1, "변조를 --check가 못 잡음"
                    os.unlink(_catp)
                    assert cmd_silent_failure_catalog(argparse.Namespace(check=True)) == 1, "파일 부재를 --check가 못 잡음"
            finally:
                if _saved_pd is None:
                    os.environ.pop("CYS_PACK_DIR", None)
                else:
                    os.environ["CYS_PACK_DIR"] = _saved_pd

        # ── 전제지식 자동주입 배터리 (OpenMontage D6 — normalize_slug 핀·resolver·티켓 byte-동일) ──
        assert normalize_slug("feedback_decision-consult-cys-sot.md") == "decision-consult-cys-sot", \
            "normalize_slug 접두·.md 제거 규칙 드리프트(registry와 byte-동일이어야)"
        assert normalize_slug("Foo-Bar") == "foo-bar", "normalize_slug lower 규칙"
        assert normalize_slug("project_x") == "x" and normalize_slug("user_y") == "y", \
            "normalize_slug 타입접두 제거 규칙"
        assert _split_csv("a, b ,,c") == ["a", "b", "c"], "_split_csv 정리 규칙"
        assert _split_csv(None) == [] and _split_csv("") == [], "_split_csv 빈 입력"
        assert resolve_prereq_block([], [], "/nonexistent-dir") == "", "빈 입력 → 빈 블록(티켓 무변)"
        # 기존 티켓 byte-identical(prereq_block 기본 "") 회귀 — 무회귀 게이트
        _t1 = build_task_ticket("T", "S", "C", "worker", FALLBACK_RULES, output_format=None)
        _t2 = build_task_ticket("T", "S", "C", "worker", FALLBACK_RULES, output_format=None, prereq_block="")
        assert _t1 == _t2, "prereq_block='' 가 기존 티켓을 변형(byte-identical 깨짐)"
        assert "전제지식" not in _t1, "빈 prereq가 티켓에 누출"
        _t3 = build_task_ticket("T", "S", "C", "worker", FALLBACK_RULES, prereq_block="ZZZ-PREREQ-MARK")
        assert _t3.endswith("ZZZ-PREREQ-MARK"), "prereq_block append(끝) 실패"
        # do/don't 무접촉 필드(C3) — dont=None byte-identical 회귀 + 주입·위치 실증
        _tn = build_task_ticket("T", "S", "C", "worker", FALLBACK_RULES)
        assert _tn == _t1, "dont 기본값(None)이 기존 티켓을 변형(byte-identical 깨짐)"
        assert "무접촉" not in _tn, "dont 미지정 시 무접촉 라인 누출"
        _td = build_task_ticket("T", "S", "C", "worker", FALLBACK_RULES, dont="ZZZ-DONT-MARK")
        assert "무접촉" in _td and "ZZZ-DONT-MARK" in _td, "--dont 무접촉 라인 미주입"
        assert _td.index("무접촉(절대") > _td.index("범위(이 파일"), "무접촉 라인이 범위 앞에 옴"
        assert _td.index("무접촉(절대") < _td.index("절대 강조 4규칙 (WORKER"), \
            "무접촉 라인 위치 오류(범위 직후·4규칙 섹션 앞이어야 — do/don't 인접)"
        # tier_hint 무접촉(R2 1단계) — tier_hint=None byte-identical 회귀 + 주입·비강제 실증
        _tt = build_task_ticket("T", "S", "C", "worker", FALLBACK_RULES, tier_hint=None)
        assert _tt == _t1, "tier_hint 기본값(None)이 기존 티켓을 변형(byte-identical 깨짐)"
        assert "권장 실행 등급" not in _tt, "tier_hint 미지정 시 등급 라인 누출"
        _th = build_task_ticket("T", "S", "C", "worker", FALLBACK_RULES, tier_hint="heavy")
        assert "권장 실행 등급" in _th and "heavy" in _th, "--tier 등급 라인 미주입"
        # resolver 해소/미해소/주석제외 (격리 tempdir 색인)
        import tempfile as _tf2
        with _tf2.TemporaryDirectory(prefix="javis-orch-d6-") as _td2:
            _mdir = os.path.join(_td2, "memory")
            os.makedirs(_mdir)
            with open(os.path.join(_mdir, "MEMORY.md"), "w", encoding="utf-8") as _f:
                _f.write("# Memory Index\n- [Foo](feedback_foo-bar.md) — 후크\n"
                         "<!-- - [Hidden](feedback_hidden.md) — 주석은 무시 -->\n")
            _blk = resolve_prereq_block(["grill-me"], ["foo-bar", "no-such-mem"], _mdir)
            assert "[skill] grill-me — cys skill show grill-me" in _blk, "skill 읽기명령 누락"
            assert "[memory] foo-bar — cat" in _blk and "feedback_foo-bar.md" in _blk, \
                "해소된 memory 파일명·읽기명령 누락"
            assert "해소 불가: no-such-mem" in _blk, "미해소 슬러그 무음 드롭(인라인 표기 누락)"
            # 주석 strip 실증(비공허): 주석 속 hidden을 *요청*하면 색인에 없어 '해소 불가'여야 한다
            # — strip이 실패했다면 hidden이 색인에 잡혀 cat 경로로 해소돼 이 assert가 깨진다.
            _hb = resolve_prereq_block([], ["hidden"], _mdir)
            assert "해소 불가: hidden" in _hb and "feedback_hidden.md" not in _hb, \
                "주석 내 색인 예시가 실entry로 오탐(comment strip 실패)"
            # 중복 collapse(같은 이름·정규화 슬러그는 1회만)
            _dup = resolve_prereq_block(["grill-me", "grill-me"], ["foo-bar", "FEEDBACK_foo-bar.md"], _mdir)
            assert _dup.count("[skill] grill-me — cys skill show grill-me") == 1, "중복 skill 방출"
            assert _dup.count("[memory] foo-bar — cat") == 1, "중복 memory(같은 슬러그) 방출"

        # ── D4 매니페스트 배선 배터리 (resolve_manifest_phase·명시 --success 우선·review_focus) ──
        assert resolve_manifest_phase(None, None) == (None, []), "빈 입력 → (None,[])"
        assert resolve_manifest_phase("/nonexistent-manifest.json", "x") == (None, []), "부재 매니페스트 → (None,[])"
        import tempfile as _tf3
        with _tf3.TemporaryDirectory(prefix="javis-orch-d4-") as _td3:
            _mf = os.path.join(_td3, "workflow.json")
            with open(_mf, "w", encoding="utf-8") as _f:
                json.dump({"name": "w", "phases": [{"id": "g", "skill": "deep-research",
                          "success_criteria": {"statement": "출처 3개 확보",
                                               "checks": [{"kind": "citation_present"}]},
                          "review_focus": ["source-quality"]}]}, _f)
            _su, _fo = resolve_manifest_phase(_mf, "g")
            if _su is not None:  # javis_manifest 배포 시에만 해소(환경 독립 — 부재 시 (None,[]) 계약은 위에서 핀)
                assert _su == "출처 3개 확보", "매니페스트 success 해소 오류: %r" % _su
                assert _fo == ["source-quality"], "review_focus 해소 오류: %r" % _fo
            # cmd_review_prompt: 명시 --success가 매니페스트보다 우선(하위호환)
            class _RA:
                task, scope, reviewer, round, success, manifest, phase = "T", "S", None, 1, "명시기준ZZZ", _mf, "g"
            _rbuf = io.StringIO()
            with contextlib.redirect_stdout(_rbuf):
                cmd_review_prompt(_RA())
            assert "명시기준ZZZ" in _rbuf.getvalue(), "명시 --success가 리뷰 프롬프트에 미반영(하위호환 깨짐)"
        # ★Fix2' guard-master-claim 순수 판정 배터리(cys 의존 없음·결정론):
        assert _parse_ref("surface:7") == 7 and _parse_ref("7") == 7 and _parse_ref("x") is None, "_parse_ref 오류"
        assert guard_master_verdict(None, None) == (0, "unset"), "미설정인데 PASS/unset 아님(부팅 차단 회귀)"
        assert guard_master_verdict("31", [99, 31])[0] == 9, "타 master(99) 보유인데 exit9 아님"
        assert guard_master_verdict("surface:31", [99])[0] == 9, "타 master(surface 접두) 보유인데 exit9 아님"
        assert guard_master_verdict("31", [31]) == (0, "idempotent"), "내가 보유자(멱등)인데 PASS 아님"
        assert guard_master_verdict("31", []) == (0, "no_master"), "master 부재인데 PASS 아님"
        assert guard_master_verdict("31", None) == (0, "list_fail"), "cys list 실패인데 PASS/list_fail 아님(부팅 차단 회귀)"
        assert guard_master_verdict("notanumber", [99]) == (0, "unparsed"), "파싱불가 env가 false-block(회귀)"

        # ─────────── W2: B1 PLAN 정책 열 · B2 slot_satisfied · check_verdicts · A12 분류 ───────────
        # B1: 정책이 편성과 같은 행에 있고, Fatal 집합 = cso·worker(조직 최소 실행 단위)뿐이다.
        assert [r for r, _a, _p in BOOT_PLAN] == \
            ["cso", "worker", "reviewer-gemini", "reviewer-codex", "reviewer-grok"], \
            "BOOT_PLAN 편성 변형(cys boot PLAN 과 파리티 깨짐)"
        assert plan_mandatory_roles() == ["cso", "worker"], \
            "Fatal 집합 변형 — 리뷰어가 Fatal 이면 리뷰어 1종 고장이 팀 전체 부트를 죽인다(B1 재발)"
        assert plan_policy("reviewer-gemini") == FAIL_DEGRADE, "네이티브 리뷰어가 Degrade 아님"
        assert plan_policy("reviewer-grok") == FAIL_DEGRADE, "선택 리뷰어가 Degrade 아님"
        assert plan_policy("cso") == FAIL_FATAL and plan_policy("worker") == FAIL_FATAL, \
            "cso·worker 가 Fatal 아님(조직 최소 실행 단위 붕괴)"
        assert plan_policy("verifier") == FAIL_DEGRADE, "미지 role 이 Fatal 로 접힘(부트 사망 회귀)"
        # PLAN 정책 열 ↔ effective_required_roles ↔ 결손 구성 3자 대조(H-PRED-7):
        #   Fatal 역할은 전부 유효 의무 목록에 있어야 하고, 유효 의무 리뷰어는 슬롯으로 해소돼야 한다.
        _yes = lambda ag, agents=None: (True, "테스트 주입")   # noqa: E731
        _synth = {"gemini": {"cmd": "/x/agy"}, "codex": {"cmd": "/x/codex"}, "claude": {"cmd": "claude"}}
        _eff = effective_required_roles(detect=_yes, agents=_synth)
        assert set(plan_mandatory_roles()) <= set(_eff), \
            "Fatal 역할이 유효 의무 목록에서 빠짐(부트는 요구하는데 check 는 안 봄)"
        assert all(r in [p[0] for p in BOOT_PLAN] or r.startswith("reviewer-claude")
                   for r in _eff), "유효 의무 역할이 PLAN 에도 슬롯에도 없음(고아 요건)"
        # B2 slot_satisfied — 네이티브·대체·부재 3케이스 + 실충전자 라벨
        s_ok, s_fill, s_nat, _ = slot_satisfied("reviewer-gemini", {"reviewer-gemini"})
        assert (s_ok, s_fill, s_nat) == (True, "reviewer-gemini", True), "네이티브 좌석 충족 실패"
        s_ok, s_fill, s_nat, _ = slot_satisfied("reviewer-gemini", {"reviewer-claude-1"})
        assert (s_ok, s_fill, s_nat) == (True, "reviewer-claude-1", False), \
            "2차 폴백 대체 좌석이 슬롯을 충족하지 못함(B2 영구 적색 재발)"
        s_ok, s_fill, _, _ = slot_satisfied("reviewer-codex", {"reviewer-claude-1"})
        assert s_ok is False and s_fill is None, "슬롯 교차 충전(codex 슬롯을 gemini 대체가 채움)"
        assert slot_satisfied("cso", {"cso-1"})[0] is False, "cso-1 이 의무 cso 를 충족(G26 재발)"
        assert slot_satisfied("worker", {"worker-2"})[0] is True, "worker-N dedup 좌석 수용 실패"
        assert slot_satisfied("reviewer-gemini", {"reviewer-grok"})[0] is False, \
            "선택 리뷰어(grok)가 의무 슬롯을 충족(G26 재발)"
        # check_verdicts — B6 강등 라벨링(agent_alive 단독=생존추정)·좌석 empty=미충족
        def _st(rows):
            return {"surfaces": rows}
        _base = [{"role": "cso", "exited": False, "awakened_at": 1.0},
                 {"role": "worker-2", "exited": False, "status": {"age_secs": 3, "state": "working"}},
                 {"role": "reviewer-gemini", "exited": False, "agent_alive": True},
                 {"role": "reviewer-codex", "exited": False, "seat": "empty", "agent_alive": False}]
        # ★밀폐 주입(2026-08-22): 로스터를 감지 결과가 아니라 테스트 상수로 고정한다. 종전엔
        #   실감지를 타서 agy·codex 미설치 기계(신규 사용자 대다수)에서 required 에
        #   reviewer-gemini 가 없어 아래 단언이 KeyError 트레이스백으로 죽었다.
        _v, _ = check_verdicts(_st(_base), detect=_yes, agents=_synth)
        assert _v["cso"]["grade"] == "awake_confirmed", "래치 좌석이 각성확정 아님"
        assert _v["worker"]["satisfied"] and _v["worker"]["filler"] == "worker-2", \
            "worker-2 dedup 좌석이 worker 요건을 못 채움"
        assert _v["reviewer-gemini"]["grade"] == "alive_presumed", \
            "agent_alive 단독이 각성확정으로 계상(B6 오답 잔존)"
        assert _v["reviewer-gemini"]["satisfied"] is True, \
            "생존추정 강등이 **실패로 승격**됐다(래치 이전 기계 전원 적색 — 역방향 회귀)"
        assert _v["reviewer-codex"]["satisfied"] is False, "좌석 empty·무신호인데 충족으로 계상"
        # 대체 좌석만 있는 상태 → 슬롯 재해소로 충족(B2)
        _sub = [{"role": "cso", "exited": False, "awakened_at": 1.0},
                {"role": "worker", "exited": False, "awakened_at": 1.0},
                {"role": "reviewer-gemini", "exited": False, "awakened_at": 1.0},
                {"role": "reviewer-claude-2", "exited": False, "awakened_at": 1.0}]
        _v2, _ = check_verdicts(_st(_sub), detect=_yes, agents=_synth)   # ★밀폐 주입(위와 동일 사유)
        assert _v2["reviewer-codex"]["satisfied"] is True, "대체 좌석 재해소 실패(B2)"
        assert _v2["reviewer-codex"]["native"] is False, "실충전자 라벨(native=False) 누락"

        # ───── (U-25 1단) 완주 정의 2축 — ready ∧ awake · additive·무동작 ─────
        # ★계측 타당성(수리 전 적색 실측): `awake` 키·`.axes`·`check --json` 은 수리 전 코드에
        #   **존재하지 않았다** → 아래 단언은 전부 KeyError/AttributeError/SystemExit 로 적색이다.
        #   트리에 위반이 0 이라 탐지기가 고장나도 초록이 되는 항(반환 호환성 핀)은 **합성
        #   표본**(하드 3원소·1원소 튜플)으로 탐지 능력 자체를 시험한다.
        _axis_prev = {k: os.environ.get(k) for k in (AWAKE_AXIS_ENV, BOOT_GATES_ENV)}
        _globals_prev = {n: globals()[n] for n in ("cys_status", "reviewer_roster")}
        try:
            for _k in (AWAKE_AXIS_ENV, BOOT_GATES_ENV):
                os.environ.pop(_k, None)        # 기본(축 켜짐)을 명시적으로 만든다
            assert awake_axis_enabled() is True, "기본 상태에서 awake 축이 꺼져 있다"
            _bn_mod = _boot_node()
            if _bn_mod is not None:
                # 이름·등급 드리프트 3자 대조 — 사고 순간에 사람이 쥐는 손잡이가 갈라지면 안 된다.
                assert BOOT_GATES_ENV == _bn_mod.BOOT_GATES_ENV, \
                    "마스터 스위치 이름 드리프트(orchestra ↔ javis_boot_node)"
                assert _awake_grade() == _bn_mod.LIVENESS_AWAKE, \
                    "각성확정 등급명 드리프트(리터럴 잔존 — 비교가 조용히 항상 False 가 된다)"
            # ⓐ 판정 dict: ready = satisfied 별칭(값 동일) · awake = 각성확정만
            _v3, _r3 = check_verdicts(_st(_base), detect=_yes, agents=_synth)
            for _r, _vv in _v3.items():
                assert _vv["ready"] is _vv["satisfied"], \
                    "%s: ready 가 satisfied 별칭이 아니다(새 축이 종전 축을 바꿨다)" % _r
            assert _v3["cso"]["awake"] is True, "래치 좌석(각성확정)이 awake 가 아니다"
            assert _v3["reviewer-gemini"]["awake"] is False, \
                "생존추정(agent_alive 단독)이 awake 로 계상 — 각성 미확인을 완주로 세는 R1 재발"
            assert _v3["reviewer-gemini"]["satisfied"] is True, \
                "awake 축 신설이 ready 를 끌어내렸다(무동작 위반 — 래치 이전 기계 전원 적색)"
            # ⓑ 축 집계: ready_missing 은 종전 missing 과 같은 목록 · awake_pending 은 경고 축
            _ax = verdict_axes(_v3)
            assert _ax["ready_missing"] == [r for r, v in _v3.items() if not v["satisfied"]], \
                "ready_missing 이 종전 missing 목록·순서와 다르다(사람용 출력 파손)"
            assert _ax["ready"] is False and "reviewer-codex" in _ax["ready_missing"], \
                "좌석 empty 가 ready 집계에서 빠졌다"
            assert "reviewer-gemini" in _ax["awake_pending"], \
                "awake_pending 이 '살아 있으나 각성 미확정' 좌석을 짚지 못한다: %r" % (_ax,)
            assert "cso" not in _ax["awake_pending"], "각성확정 좌석이 awake_pending 에 섞였다"
            assert "reviewer-codex" not in _ax["awake_pending"], \
                "ready 미충족 좌석이 경고 축에 섞였다(ready_missing 과 사실이 겹친다)"
            assert _ax["awake"] is False, "각성 미확정 좌석이 있는데 awake 축이 참"
            # ⓒ 결손 산출: 종전 2원소 언팩 유지 + `.axes` 부가
            _st_ok = _st([{"role": "cso", "exited": False, "awakened_at": 1.0},
                          {"role": "worker", "exited": False, "awakened_at": 1.0},
                          {"role": "reviewer-gemini", "exited": False, "awakened_at": 1.0},
                          {"role": "reviewer-codex", "exited": False, "awakened_at": 1.0}])
            _d = _shared_verdict_deficit(_st_ok, requery=lambda: _st_ok, tick_s=0,
                                         detect=_yes, agents=_synth)
            _has, _why = _d                     # ★밖의 2원소 소비자 계약 그대로
            assert _has is False, "정상 팀이 결손>0(무동작 위반): %s" % _why

            def _deficit_shape_ok(val):
                """반환이 **종전 2원소 계약**을 지키는가 — 밖의 소비자 6곳이 이 모양을 푼다."""
                try:
                    _a, _b = val
                except ValueError:
                    return False
                return len(val) == 2 and tuple(val) == (_a, _b)

            assert _deficit_shape_ok(_d) is True, \
                "결손 반환의 길이·동등성이 바뀌었다 — 밖의 2원소 소비자가 ValueError 로 죽는다"
            # ★합성 표본(탐지 능력 시험): 위 핀은 트리에 위반이 0 이라 탐지기가 고장나도 초록이다.
            assert _deficit_shape_ok((True, "why", {"awake": True})) is False, \
                "합성 3원소 표본을 호환 술어가 통과시켰다 — 이 핀은 고장난 계측기다"
            assert _deficit_shape_ok((True,)) is False, "합성 1원소 표본을 통과시켰다"
            assert _d.axes is not None and _d.axes["ready"] is True, "결손 반환에 2축이 없다"
            assert _d.axes["awake"] is True and _d.axes["awake_pending"] == [], \
                "전원 각성 팀에서 awake 축이 참이 아니다: %r" % (_d.axes,)
            assert (_d.has, _d.why) == (_has, _why), "이름 있는 접근자가 튜플 원소와 다르다"
            # ★★무동작의 **핵심 핀** — '관측되지만 결손이 아니다'.
            #   전원 각성 fixture 로는 이 계약을 잴 수 없다(awake 를 결손에 끌어들여도 결과가
            #   같아서 변이가 살아남는다 — 실측으로 확인한 계측 구멍). 그래서 **ready 이지만
            #   각성 미확정인 좌석이 하나 섞인** fixture 로 잰다: awake 축은 그 좌석을 정확히
            #   짚어야 하고, 그럼에도 결손은 0 이어야 한다(1단에서 awake 는 exit·boot 호출
            #   판정에 참여하지 않는다).
            _st_presumed = _st([
                {"role": "cso", "exited": False, "awakened_at": 1.0},
                {"role": "worker", "exited": False, "awakened_at": 1.0},
                {"role": "reviewer-gemini", "exited": False, "agent_alive": True},
                {"role": "reviewer-codex", "exited": False, "awakened_at": 1.0}])
            _dp = _shared_verdict_deficit(_st_presumed, requery=lambda: _st_presumed, tick_s=0,
                                          detect=_yes, agents=_synth)
            assert _dp[0] is False, \
                "생존추정(각성 미확정) 좌석이 결손으로 계상됐다 — awake 축이 부트 판정에 " \
                "새어 들어갔다(1단 무동작 위반 · 불필요 boot churn): %s" % _dp[1]
            assert _dp.axes["awake"] is False \
                and _dp.axes["awake_pending"] == ["reviewer-gemini"], \
                "그 좌석을 awake 축이 짚지 못한다(관측 자체가 죽었다): %r" % (_dp.axes,)
            assert _dp.axes["ready"] is True and _dp.axes["ready_missing"] == [], \
                "각성 미확정이 ready 축까지 끌어내렸다(래치 이전 기계 전원 적색 회귀)"
            # 판정 불가 경로는 `.axes` 도 None(미측정) — 빈 집계는 '재어 보니 이상 없음' 거짓 초록

            def _detect_boom(_agent, _agents=None):
                raise RuntimeError("로스터 산출 실패 주입")

            _dbad = _shared_verdict_deficit({"surfaces": []}, requery=lambda: {"surfaces": []},
                                            tick_s=0, detect=_detect_boom, agents=_synth)
            assert _dbad[0] is None and _dbad.axes is None, \
                "판정 불가인데 축 집계가 값처럼 나왔다(미측정을 측정으로 위장): %r" % (_dbad,)
            # ⓓ `check --json` — 축 노출 · 사람용 산문 미출력 · exit 계약 무변경
            globals()["cys_status"] = lambda: _st_ok
            globals()["reviewer_roster"] = lambda detect=None, agents=None: [
                {"role": "reviewer-gemini", "agent": "gemini", "native": True,
                 "substituted_for": None, "reason": "테스트 주입"},
                {"role": "reviewer-codex", "agent": "codex", "native": True,
                 "substituted_for": None, "reason": "테스트 주입"}]

            class _CkJson(object):
                json = True

            class _CkBare(object):                # 속성 없는 Args(javis_idempotency 스파이 형상)
                pass

            _jb, _hb = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(_jb):
                _rc_j = cmd_check(_CkJson())
            with contextlib.redirect_stdout(_hb):
                _rc_h = cmd_check(_CkBare())
            _jout, _hout = _jb.getvalue(), _hb.getvalue()
            assert _rc_j == 0 and _rc_h == 0, \
                "정상 팀에서 exit 0 이 아니다(exit 계약 변형): json=%s human=%s" % (_rc_j, _rc_h)
            assert "READY" not in _jout, "--json stdout 에 사람용 산문이 섞였다(기계 소비 파손)"
            _pl = json.loads(_jout)               # 1줄 JSON — 산문 혼입이면 여기서 죽는다
            assert _pl["exit"] == 0 and _pl["ready"] is True and _pl["awake"] is True, \
                "payload 2축이 정상 팀을 정확히 말하지 않는다: %r" % (_pl,)
            assert _pl["awake_pending"] == [] and _pl["axis"] is True, \
                "축 on 인데 awake_pending 이 빈 목록이 아니다: %r" % (_pl,)
            assert "LLM orchestrating READY" in _hout, \
                "사람용 경로의 READY 문안이 사라졌다(디렉티브 5곳·preflight C03 핀 파손)"
            # ★exit 2(판정 불가)도 `--json` 이면 JSON 이어야 한다 — 여기만 산문이면 소비자의
            #   json.loads 가 **데몬 소실 순간에** 죽는다(진단이 가장 필요한 순간).
            #   그리고 exit 2 는 노드에 대해 아무것도 말하지 않으므로 전 판정 필드가 null 이다
            #   (ready=false 로 두면 '재어 보니 안 섰다'는 없는 측정을 지어내는 것이다).
            globals()["cys_status"] = lambda: None
            _nb, _neb = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(_nb), contextlib.redirect_stderr(_neb):
                _rc_n = cmd_check(_CkJson())
            _npl = json.loads(_nb.getvalue())
            assert _rc_n == 2, "데몬 소실인데 exit 2 가 아니다(판정 불가↔미기동 오귀속)"
            assert _npl["exit"] == 2 and _npl["ready"] is None \
                and _npl["awake"] is None and _npl["verdicts"] is None, \
                "exit 2 payload 가 없는 측정을 지어냈다: %r" % (_npl,)
            assert "cys ping" in _neb.getvalue(), \
                "판정 불가 처방(`cys ping`)이 사라졌다 — 조용한 강등"
            _nb2 = io.StringIO()
            with contextlib.redirect_stdout(_nb2):
                _rc_n2 = cmd_check(_CkBare())
            assert _rc_n2 == 2 and "cys ping" in _nb2.getvalue(), \
                "사람용 exit 2 문안이 stdout 에서 사라졌다(종전 계약 파손)"
            globals()["cys_status"] = lambda: _st_ok
            # ⓔ 롤백 1지점 — 마스터·축 **각각 하나**로 종전 1축 판정으로 복귀한다
            for _knob in (BOOT_GATES_ENV, AWAKE_AXIS_ENV):
                os.environ[_knob] = "0"
                try:
                    assert awake_axis_enabled() is False, "%s=0 이 축을 끄지 못한다" % _knob
                    _voff, _ = check_verdicts(_st(_base), detect=_yes, agents=_synth)
                    assert _voff == {r: {k: x for k, x in v.items()
                                         if k not in ("ready", "awake")}
                                     for r, v in _v3.items()}, \
                        "%s=0 의 판정 dict 가 종전 모양과 다르다(되돌림이 참이 아니다)" % _knob
                    _axoff = verdict_axes(_voff)
                    assert _axoff["awake"] is None and _axoff["awake_pending"] is None, \
                        "축 off 에서 awake 가 None(미측정)이 아니다 — []·False 는 거짓 초록이다"
                    assert _axoff["ready_missing"] == _ax["ready_missing"], \
                        "%s=0 이 ready 축까지 흔들었다(축 노브의 교차 오염)" % _knob
                    _doff = _shared_verdict_deficit(_st_ok, requery=lambda: _st_ok, tick_s=0,
                                                    detect=_yes, agents=_synth)
                    assert tuple(_doff) == tuple(_d), \
                        "축 on/off 로 결손 판정이 달라졌다(1단 무동작 계약 위반): %r" % (_doff,)
                    _jb2 = io.StringIO()
                    with contextlib.redirect_stdout(_jb2):
                        _rc2 = cmd_check(_CkJson())
                    _pl2 = json.loads(_jb2.getvalue())
                    assert _rc2 == _rc_j and _pl2["exit"] == 0, \
                        "%s=0 이 exit 계약을 흔들었다(롤백이 기저보다 파괴적)" % _knob
                    assert _pl2["awake"] is None and _pl2["awake_pending"] is None \
                        and _pl2["axis"] is False, \
                        "%s=0 인데 payload 가 축을 측정한 것처럼 말한다: %r" % (_knob, _pl2)
                finally:
                    os.environ.pop(_knob, None)
            assert awake_axis_enabled() is True, "킬스위치 복원 실패(검체 오염)"
        finally:
            globals().update(_globals_prev)
            for _k, _v in _axis_prev.items():
                if _v is None:
                    os.environ.pop(_k, None)
                else:
                    os.environ[_k] = _v
        # A12 exit 분류
        assert classify_call_exit(0)[0] == EXIT_CLASS_OK, "rc0 이 ok 아님"
        assert classify_call_exit(2)[0] == EXIT_CLASS_PERMANENT, "exit 2 가 영구 실패 아님"
        assert classify_call_exit(127)[0] == EXIT_CLASS_PERMANENT, "exit 127 이 영구 실패 아님"
        assert classify_call_exit(124)[0] == EXIT_CLASS_TRANSIENT, "exit 124 가 재시도 대상 아님"
        assert classify_call_exit(1)[0] == EXIT_CLASS_TRANSIENT, "exit 1 이 재시도 대상 아님(보수 이탈)"
        assert "재시도는 무의미" in classify_call_exit(127)[1], "영구 실패에 재시도 금지 처방 누락"
        assert _boot_node_outer_timeout() >= 100, "boot_node 외부 상한이 비정상(예산 파생 실패)"
    except AssertionError as e:
        print("javis_orchestra self-test FAIL: %s" % e, file=sys.stderr)
        return 1
    print("javis_orchestra self-test OK (W2: PLAN 정책열·slot_satisfied 3케이스·check_verdicts "
          "강등라벨·A12 exit 분류 + U-25 완주 2축(ready 별칭 불변·awake 관측·축 집계·"
          "결손 2원소 호환[합성 3원소 표본]·check --json·마스터/축 롤백 2노브) + "
          "4종 노드·라운드 상한·경로 탈출방지·제약 주입·"
          "4규칙 티켓 주입·do/don't 무접촉·파싱·셀 새니타이즈·무음실패 카탈로그·전제지식 주입·매니페스트 배선)")
    return 0


def main():
    # preflight 호환: `--self-test`는 subcommand 없이도 동작해야 한다(가로채기).
    if "--self-test" in sys.argv:
        return cmd_self_test(None)
    # ★B18(H-DOC-2): 훅 note·문서가 인용할 팀 구성 1줄. subcommand 공간을 늘리지 않고
    #   `--self-test` 와 동일한 가로채기 관례를 쓴다(javis_budget --note-check-window 대칭).
    if "--note-team-roster" in sys.argv:
        print(team_roster_note())
        return 0
    ap = argparse.ArgumentParser(description="LLM 오케스트레이션 결정론 도구(앵커4)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    ck = sub.add_parser("check", help="4종 의무 노드 생존 판정")
    ck.add_argument("--json", action="store_true",
                    help="완주 2축(ready ∧ awake)을 stdout 에 1줄 JSON 으로 출력(U-25 1단 — "
                         "관측 전용: exit 계약 무변경 · 사람용 산문 미출력). awake·awake_pending "
                         "이 null 이면 미측정(축 off·구 팩 스큐)이지 '각성 없음'이 아니다")

    br = sub.add_parser("boot-reviewers",
                        help="리뷰어(agy·codex) 감지→기동. 미감지/각성실패 시 Claude 대체로 자동 폴백(멈춤 없음)")
    br.add_argument("--plan", action="store_true", help="기동 없이 감지 결과 로스터만 출력(dry-run)")

    rp = sub.add_parser("review-prompt", help="제약 포함 리뷰 의뢰 프롬프트 생성")
    rp.add_argument("--task", required=True)
    rp.add_argument("--scope", required=True, help="검토 대상 파일/범위")
    rp.add_argument("--reviewer", choices=["gemini", "codex"], default=None)
    rp.add_argument("--round", type=int, default=1)
    rp.add_argument("--success", default=None,
                    help="평가 기준(구현 위임과 동일 — 리뷰어에게도 같은 기준 투입, N3 양방향)")
    rp.add_argument("--manifest", default=None,
                    help="워크플로우 매니페스트 경로 — 단계 평가기준·review_focus를 리뷰 프롬프트에 주입(D4·명시 --success 우선)")
    rp.add_argument("--phase", default=None, help="매니페스트 단계 id (--manifest와 함께·D4)")
    rp.add_argument("--reviewer-role", dest="reviewer_role",
                    choices=["reviewer1", "reviewer2"], default="reviewer1",
                    help="앵커 역할(T1·A2): reviewer1=적대검증(기각 이력=반박 조건부 필터) · "
                         "reviewer2=감사(기각 이력=면역 아닌 감사 자료)")
    rp.add_argument("--slug", default=None,
                    help="handoff 슬러그 접두 우선 필터(T1 기각 재주입 — 같은 작업 우선)")

    tp = sub.add_parser("task-prompt", help="생존 게이트 + 절대 강조 4규칙 포함 위임 티켓 생성")
    tp.add_argument("--task", required=True)
    tp.add_argument("--scope", required=True, help="작업 대상 파일/범위")
    tp.add_argument("--success", default=None, help="성공 기준 (완료 보고의 검증 기준)")
    tp.add_argument("--to", default="worker", help="위임 대상 역할 (기본 worker)")
    tp.add_argument("--output-format", default=None,
                    help="산출 형식·구조 (W8 4-part output-format 슬롯 — 예: 'JSON {필드}', '마크다운 표', '보고서 PDF')")
    tp.add_argument("--requires-skills", default=None,
                    help="전제 스킬(쉼표 구분) — 티켓에 읽기순서 블록 주입(D6 progressive disclosure)")
    tp.add_argument("--related-memory", default=None,
                    help="전제 증류 memory 슬러그(쉼표 구분) — MEMORY.md 색인 해소·미해소 인라인 표기(D6)")
    tp.add_argument("--manifest", default=None,
                    help="워크플로우 매니페스트(workflow.json) 경로 — 단계 success_criteria를 --success로 주입(D4·명시 --success 우선)")
    tp.add_argument("--phase", default=None, help="매니페스트 단계 id (--manifest와 함께·D4)")
    tp.add_argument("--dont", default=None,
                    help="무접촉(do-not-touch) — 워커가 절대 수정·삭제·리팩터·포맷하지 말 "
                         "파일/영역(외과적 변경 음의 경계·4대 행동지침③). 미지정 시 티켓 byte-동일")
    tp.add_argument("--tier", default=None,
                    help="권장 실행 등급 정보 1줄 주입(trivial/standard/heavy — 강제 아님·R2 1단계·"
                         "javis_route suggested_node와 정합). 미지정 시 티켓 byte-동일")
    tp.add_argument("--probes", default=None,
                    help="이 태스크의 필수 probe 목록(쉼표 구분 — 예: 'submit,artifact'). 지정 시 "
                         "티켓에 done 전 각 probe PASS 영수증 필수 블록 삽입(P3 · 설계 §2.2·§4 컴포넌트 C·"
                         "javis_actprobe.py 대조). 미지정 시 블록 부재 → 티켓 byte-동일(하위호환)")
    tp.add_argument("--no-survival-gate", action="store_true",
                    help="생존 게이트 생략(D5 일회용 fresh 경로 — 워커 surface가 실행 시점에 생성될 때만). "
                         "평시 위임엔 쓰지 마라(상시 워커 생존 확인이 안전).")

    pp = sub.add_parser("phase-plan",
                        help="Task를 자기완결 Phase 티켓으로 분해 (영상 N6 — Task/Phase 순차)")
    pp.add_argument("--task", required=True)
    pp.add_argument("--phases", required=True, help="세미콜론 분리 Phase 이름들 (예: \"설계;구현;검증\")")
    pp.add_argument("--scope", required=True, help="작업 대상 파일/범위")
    pp.add_argument("--success", default=None, help="성공 기준 (각 Phase 티켓에 동일 투입)")
    pp.add_argument("--to", default="worker", help="위임 대상 역할 (기본 worker)")
    pp.add_argument("--requires-skills", default=None,
                    help="전제 스킬(쉼표 구분) — 각 Phase 티켓에 주입(D6)")
    pp.add_argument("--related-memory", default=None,
                    help="전제 memory 슬러그(쉼표 구분) — 각 Phase 티켓에 주입(D6)")
    pp.add_argument("--dont", default=None,
                    help="무접촉(do-not-touch) — 각 Phase 티켓에 음의 경계 주입(외과적 변경·"
                         "4대 행동지침③). 미지정 시 티켓 byte-동일")

    ri = sub.add_parser("round-init"); ri.add_argument("--task", required=True)
    rl = sub.add_parser("round-log")
    rl.add_argument("--task", required=True); rl.add_argument("--round", type=int, required=True)
    rl.add_argument("--evaluator", required=True)   # --score 제거: §6-4 점수 금지 · §9-7-2 부수 1
    rl.add_argument("--verdict", default="")
    rl.add_argument("--from-cmd", dest="from_cmd", default=None,
                    help="기계검증 명령을 직접 실행해 exit code로 verdict 자동 기록"
                         "(machine 평가자 권장 — 전사 없는 producer≠evaluator 경로)")
    rl.add_argument("--verdict-json", dest="verdict_json", default=None,
                    help="★G8 리뷰어(gemini/agy/codex) 행 필수 — javis_verdict 스키마 통과 "
                         "verdict JSON 경로(미통과·부재 시 기록 거부, SKIP 행만 예외)")
    rs = sub.add_parser("round-status"); rs.add_argument("--task", required=True)

    gs = sub.add_parser("gate-status", help="자율주행 축1 — 4자 수렴 결정론 판정")
    gs.add_argument("--task", required=True)
    gs.add_argument("--round", type=int, default=None, help="생략 시 최신 라운드")

    sub.add_parser("next-action", help="자율주행 축3 — SESSION_STATE 다음 액션 큐 첫 미완 항목")

    sfc = sub.add_parser("silent-failure-catalog",
                         help="무음실패 카탈로그(D5) 런타임 재생성 — pack/round/SILENT_FAILURE_CATALOG.md")
    sfc.add_argument("--check", action="store_true",
                     help="재생성 없이 디스크 카탈로그가 SILENT_FAILURES와 정합인지 드리프트 검사(불일치=exit 1)")

    ch = sub.add_parser("channel-health",
                        help="콘텐츠 채널 per-channel 헬스(OPP-02) — 노드 check 의 짝(콘텐츠 채널 도달성)")
    ch.add_argument("--json", action="store_true", help="기계판(verdict 배열) — 기본은 silence-first")
    ch.add_argument("channels", nargs="*", help="부분집합(예: reddit x). 비우면 전체")

    sub.add_parser("guard-master-claim",
                   help="Fix2' misrouted-master 부트 가드 — claim-role master 직전 결정론 선검사"
                        "(surface id env 미설정=PASS·타 master 보유=exit 9)")

    args = ap.parse_args()
    return {
        "check": cmd_check,
        "boot-reviewers": cmd_boot_reviewers,
        "review-prompt": cmd_review_prompt,
        "task-prompt": cmd_task_prompt,
        "phase-plan": cmd_phase_plan,
        "round-init": cmd_round_init,
        "round-log": cmd_round_log,
        "round-status": cmd_round_status,
        "gate-status": cmd_gate_status,
        "next-action": cmd_next_action,
        "silent-failure-catalog": cmd_silent_failure_catalog,
        "channel-health": cmd_channel_health,
        "guard-master-claim": cmd_guard_master_claim,
    }[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
