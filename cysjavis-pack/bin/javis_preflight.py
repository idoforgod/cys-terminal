#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_preflight — CYSJavis 부트 결정론 프리플라이트 (절대지침의 기계 검증부).

마스터 부트 시퀀스 ⓪단계에서 반드시 실행된다. 이 스크립트가 수행하는
존재 검증·번호/역할 매핑·범위 검사·hook 등록 검사는 LLM이 자연어로 재추론하지
않는다 — 이 출력만이 유일한 사실이다 (할루시네이션 구조 차단 = 결정론 환원).

사용:
    python3 javis_preflight.py [--fix] [--json] [--skip <ID> ...]

종료 코드: 0 = FAIL 없음(WARN 허용), 1 = FAIL 존재.
의존성: 파이썬 표준 라이브러리만 (네트워크·LLM 호출 없음).
"""

import argparse
import contextlib
import errno
import hashlib
import json
import math
import os
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unicodedata

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
#   ._pth 는 표준 경로 계산을 우회해 스크립트 폴더를 sys.path 에 넣지 않는다(javis_bootstrap.py:94
#   선례·test_import_guard 계약). append 인 이유: 발견이 목적이고 stdlib precedence 를 강등하지 않는다.
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

# ★공용 크로스플랫폼 락·원자쓰기(W1a 신설 javis_lock → W3 소비 이관 · G16).
#   import 실패(팩 스큐·부분 배포)는 preflight 를 죽이지 않는다 — None 이면 `_settings_rmw` 가
#   mkstemp 인라인 폴백으로 강등한다(직렬화만 상실·등록은 계속).
try:
    import javis_lock as _lock
except Exception:
    _lock = None

PASS, FAIL, WARN, FIXED, SKIP = "PASS", "FAIL", "WARN", "FIXED", "SKIP"
# OPP-17 Mutation 게이트 status — dry/safe 미리보기·무변경진단·비가역 차단(WARN-first).
DRYRUN, SAFE_GAP, BLOCKED = "DRYRUN", "SAFE-GAP", "BLOCKED"

DIRECTIVES = [
    "MASTER_DIRECTIVE.md",
    "WORKER_DIRECTIVE.md",
    "CSO_DIRECTIVE.md",
    "REVIEWER_DIRECTIVE.md",
]

# 절대지침 핵심 조항의 내용 핀 — 디렉티브가 약화·소실되면 결정론으로 검출된다.
CONTENT_PINS = {
    # ★원자 전환 게이트 §9-7-2 부수 4 · §9-7-7 (2026-08-01 재산정)
    #   재산정 규칙: 역할당 20~30핀 · 최대/최소 비 ≤ 1.5배(구 53 대 1 = 53배 격차 해소).
#   근거·전수표: _work/absolute-rules-audit-20260731/gate-staging/pins/CONTENT_PINS.md
    #   각 핀은 해당 역할 헌장의 조문 또는 운영계약의 집행 배선에 1:1로 대응한다.
    #   구 문안 핀(구 종료 기준 리터럴 · 고정 향상률 · 구 호칭 · 구 Phase 보고 문구)은 전량 폐기.
    "MASTER_DIRECTIVE.md": [
        # 헌장 제0조 — 정지 경계
        ("정지 경계", "다섯 정지의 상주 배치(마스터 헌장 제0조)"),
        ("로컬 커밋은 가역", "외부 발행≠로컬 커밋 구분(제0조 1항 denylist)"),
        ("팩 템플릿 강제 복원은 절대 금지", "부트 ⓪ 절대 금지 1줄 — 무백업 소실 차단(계약 §1-1·§11-13)"),
        # 헌장 제1조 — 정체와 호칭 (구체 호칭은 주인님 주권이라 정책 라인만 핀)
        ("호칭의 정의처는 마스터 헌장 제1조", "호칭 단일 정의처 참조 규정(제1조)"),
        # 헌장 제2조·계약 §1 — 부트·라우팅
        ("javis_bootstrap.py", "부트 실행 주체 단일 계약(계약 §1-1)"),
        ("javis_preflight", "부트 ⓪ 결정론 프리플라이트 편입(계약 §1-1·§1-5)"),
        ("lane-path", "레인 격리 경로 규약 — base 경로 하드코딩 금지(계약 §1-6)"),
        # 헌장 제3조 — 역할 경계
        ("4종 의무 노드", "의무 노드 4종 상시(제3조 확정값)"),
        ("javis_orchestra.py", "기동 확인·라운드·게이트 결정론 도구 편입"),
        ("task-prompt", "위임 티켓 결정론 생성기 의무(계약 §1-4)"),
        ("수기 티켓 위임은 금지", "티켓 도구 우회 차단 고유 핀(계약 §1-4)"),
        # 헌장 제4조 — 품질 불가침 (절대 강조 4규칙)
        ("절대 강조 4규칙", "위임 시마다 4규칙 절대 강조"),
        ("a) **품질 절대우선**", "4규칙 a 불릿 고유 핀 — 교차참조 겹침 무력화 방지"),
        ("hallucination-guard", "환각방지 전담 sub-skill 가동(계약 §6-0)"),
        ("몽상", "몽상·망상 촉진 절대 금지"),
        ("Garbage-in", "토대 오염 차단 — 다듬어도 거짓만 정교해진다"),
        ("grill-me", "의도 합의 — 합의까지 질문 반복"),
        ("길이는 원문 수준", "요약·압축 절대 금지·길이 보존"),
        ("충돌 시 상위 기준 절대 우선", "검증 토대 붕괴 시 결론 도출 중단 게이트(제0조 5항)"),
        # 헌장 제9조·제10조 — 검증·라운드·RSI (구 종료 기준 리터럴 대체)
        ("잠근 합격 기준의 미달 항목 0", "라운드 통과·종료 기준(제9조·계약 §7-1) — 구 리터럴 대체"),
        ("keep-or-discard", "라운드 종결 방식 — 고정 향상률 목표 금지의 대체(계약 §7-6)"),
        ("gate-status", "4자 수렴 결정론 판정 도구 편입(계약 §6-8)"),
        ("GATE CONVERGED", "수렴 시에만 자동 전환 — 눈대중 차단"),
        # 헌장 제5조·제6조 — 자율·승인
        ("next-action", "축3 다음 액션 큐 결정론 추출 도구 편입(계약 §9-3)"),
        ("javis_resource_gate.py", "②등급 자원 게이트 선행 의무(제6조·계약 §5-1)"),
        ("가장 좋은 옵션", "무지성 승인이 아니라 최선 옵션 확인 후 승인(계약 §4-5)"),
        # 헌장 제7조·제11조 — 컨텍스트·영속
        ("60%", "컨텍스트 임계 명문화(제7조)"),
        ("MASTER_TODO.md", "master 자신의 todo 영속(제11조·계약 §8-1)"),
        ("Phase 종료 시 주인님께 1줄 push", "Phase 보고 의무(계약 §9-2) — 구 문안 대체"),
        ("품질 게이트를 무르게 하지 않는다", "자율화=전환 주체만·게이트 엄격성 불변(제5조)"),
    ],
    "WORKER_DIRECTIVE.md": [
        ("정지 경계", "다섯 정지의 상주 배치(워커 헌장 제0조)"),
        ("호칭의 정의처는 마스터 헌장 제1조", "호칭 단일 정의처 참조 규정"),
        ("충돌 시 헌장 > 운영계약 > 이 디렉티브", "정의처 우선순위 1줄 — 배치본 성격 고지"),
        ("WORKER_TODO.md", "워커 todo 영속(워커 헌장 제10조)"),
        ("60%", "컨텍스트 임계 명문화(워커 헌장 제11조)"),
        ("set-status", "컨텍스트·태스크 상태 자기보고 의무"),
        ("--queued", "자동 Return 배달 인지(계약 §2-1)"),
        # 아래 핀들은 orchestra RULE_MARKERS 와 패리티를 이룬다(orchestra --self-test 가
        # 기계 검증) — 마커 소실로 task-prompt 가 폴백 강등될 때 C03 이 같은 원인을 가리킨다.
        ("절대 강조 4규칙", "4규칙 기본 계약 — 티켓 누락 시에도 준수"),
        ("a) **품질 절대우선**", "4규칙 a 불릿 고유 핀 — 추출 원천 약화 전파 차단"),
        ("할루시네이션 방지", "4규칙 b 핀 — 마커 패리티"),
        ("hallucination-guard", "환각방지 전담 sub-skill 사용"),
        ("몽상", "몽상·망상 촉진 절대 금지 — 추출 원천 핀"),
        ("Garbage-in", "토대 오염 차단 — 추출 원천 핀"),
        ("grill-me", "의도 합의 스킬"),
        ("합의에 이를 때까지", "의도 합의 핵심 술어 — 합의까지 질문 반복"),
        ("요약·압축 절대 금지", "4규칙 d 핀 — 마커 패리티"),
        ("전문용어·약호", "일반인 첨삭 — 전문용어·약호만 쉬운 말로"),
        ("길이는 원문 수준", "요약·압축 금지·길이 보존 — 추출 원천 핀"),
        ("충돌 시 상위 기준 절대 우선", "검증 토대 붕괴 시 중단·보고 게이트"),
        ("cys run --", "서버 생명주기 강제 종료 — 프로세스 그룹 원장 등록(워커 헌장 제3조)"),
        ("javis_task.py", "위임 태스크 체크아웃·상태 전이 결정론 도구"),
        ("--evidence", "done 전이 증거 게이트 — 부재 시 거부"),
        ("잠근 합격 기준의 미달 항목 0", "라운드 통과·종료 기준 — 구 리터럴 대체"),
        ("top goal", "RSI 목표 4필드 수립·보고 의무(마스터 헌장 제10조)"),
    ],
    "CSO_DIRECTIVE.md": [
        ("정지 경계", "다섯 정지의 상주 배치(CSO 헌장 제0조)"),
        ("살아있는 타 노드·세션의 종료", "오살 금지 denylist 항 — 효과 기반 판정"),
        ("호칭의 정의처는 마스터 헌장 제1조", "호칭 단일 정의처 참조 규정"),
        ("충돌 시 헌장 > 운영계약 > 이 디렉티브", "정의처 우선순위 1줄 — 배치본 성격 고지"),
        ("cys status --json", "노드·헬스 관측 실재 명령(계약 §10 대응표)"),
        ("health_recent", "헬스 관측 필드 고유 핀 — 부재 명령 승계 차단"),
        ("javis_orchestra.py check", "좌석 생존 등급 판정 실재 명령"),
        ("javis_report_gate.py", "게이트 대장 조회 실재 명령"),
        ("능동 점검", "능동 점검 의무 — 이벤트 구동+정기 60분(CSO 헌장 제3조)"),
        ("CYS_IDLE_SECONDS", "idle 5분 임계 명시(계약 §2-5)"),
        ("--queued", "자동 Return 배달 인지(계약 §2-1)"),
        ("javis_resource_gate.py", "사전 자원 게이트 판정자(계약 §5-1)"),
        ("cys run --", "서버 생명주기 — 프로세스 그룹 강제 종료가 기본 동작"),
        ("백프레셔", "큐 적체 행동 규칙(계약 §5-3)"),
        ("자가치유 주기 잡 생존", "CSO 단독 책임 — 순환 의존 차단(CSO 헌장 제8조·계약 §2-8)"),
        ("last_fired", "잡 생존 판정 필드 고유 핀 — 미발화 배수 판정"),
        ("cys cycle-agent", "clear 상호 집행 핸드셰이크 도구(계약 §3-3)"),
        ("60%", "컨텍스트 임계 명문화(CSO 헌장 제6조)"),
        ("CSO_TODO.md", "CSO todo 영속(CSO 헌장 제9조)"),
        ("SESSION_STATE", "복원 정본 갱신 의무(계약 §8-2·§8-5)"),
        ("AUTOPILOT_PAUSED", "kill-switch 2경로 파일 규약(계약 §9-4)"),
        ("오살 금지", "회생 가능 노드 종료 금지 원칙(CSO 헌장 제5조)"),
        # v0.4 집합 통일(재정 R-03) — pause 중 허용 범위 = 생명 유지 목록 그 자체. 이 두 핀이 빠지면
        # 정지 중 살아있는 타 노드 종료 금지가 문서에서 소리 없이 사라진다.
        ("일시정지 중 허용 범위 = 바로 위 생명 유지 목록", "pause 중 허용 범위 = 생명 유지 목록 그 자체(CSO 헌장 제0조 4항 · 계약 §9-4 · v0.4 집합 통일)"),
        ("exited=true", "산 노드/죽은 노드 구분의 결정론 판정 기준 — 판정 불능은 산 노드로 보류"),
    ],
    "REVIEWER_DIRECTIVE.md": [
        ("정지 경계", "다섯 정지의 상주 배치(리뷰어 헌장 제0조)"),
        ("호칭의 정의처는 마스터 헌장 제1조", "호칭 단일 정의처 참조 규정"),
        ("충돌 시 헌장 > 운영계약 > 이 디렉티브", "정의처 우선순위 1줄 — 배치본 성격 고지"),
        ("producer", "producer ≠ evaluator 분리 원칙(리뷰어 헌장 제1조)"),
        ("무관한 repo 및 파일 배회 금지", "엄격 제약 1(계약 §6-3)"),
        ("도구 남용 금지", "엄격 제약 2"),
        ("지정 파일과 task만 검토", "엄격 제약 3"),
        ("서버 기동/상태 변경 명령 금지", "엄격 제약 4 — 자원 오염 차단"),
        ("REVIEWER_VERDICT_CONTRACT.md", "verdict 정본 스키마 참조(계약 §6-4)"),
        ("ACCEPT | REVISE | BLOCK | ESCALATE", "verdict enum 4종 — 그 외 값 금지"),
        ("evidence", "근거 필수 — 근거 없는 YES는 검증이 아니다"),
        ("file:line", "근거 형식 고유 핀"),
        ("score` 필드 금지", "점수(0-100) 금지 — 평균·다수결 affordance 차단"),
        ("다수결·judge auto-pick 금지", "합의 대체 금지 원칙"),
        ("독립 재유도", "불일치 결착 방식 — master 원출처 재유도"),
        ("anonymized peer-review", "고위험 포인트 2차 교차반박"),
        ("steelman antithesis", "ACCEPT 전 최강 반론 1개 구성 의무"),
        ("대상 커밋 해시", "리비전 바인딩 — 대상이 바뀌면 검증이 무효(계약 §6-6·§6-7)"),
        ("잠근 합격 기준의 미달 항목 0", "라운드 종료 기준 — 구 리터럴 대체"),
        ("REVIEWER_TODO.md", "리뷰어 todo 영속(리뷰어 헌장 제8조)"),
    ],
    # ── CEO_TEMPLATE.md — 라이브 CEO 템플릿 파일 자체의 내용 핀(스펙 v4 §D2 · cid 는
    #    :파일명 첫 토큰 규칙으로 C03.pin.ceo 자연 파생). 승격 '표지' 술어(MARKER_PINS)와
    #    앞 3핀을 공유하되 상수는 분리 유지한다 — 여기는 파일 검사 전용 집합이다. ──
    "CEO_TEMPLATE.md": [
        ("master of master", "CEO 정체 선언 — 거버넌스 머리글(구·신 템플릿 공통)"),
        ("단일소유 강제", "부서 수명주기 단일소유 강제 절(구·신 템플릿 공통)"),
        ("exit 7", "단일소유 가드 exit 7 계약(구·신 공통 · 약핀 — 단독 판정 금지)"),
        # ★Wave2 대기 핀: 아래 문구는 Wave2 합성 서문(scripts/gen_ceo_template.py 재합성)이
        #   넣을 문구라 **당장 repo 템플릿엔 없어 FAIL — 정상**이며 Wave2 재합성 후 green 이
        #   된다. 이 핀은 라이브 템플릿 파일 검사 전용 — 승격 '표지' 술어(MARKER_PINS)에는
        #   절대 편입하지 않는다(R3 A7: 구 템플릿에 부재라 구판 승격 표지가 사멸 → 위경보 재발).
        ("직접 구현은 §1-A 사소 예외 없이 금지",
         "CEO 직접 구현 금지 — 직할/부서 위임 판단 트리(합성 서문 · Wave2 재합성 후 green)"),
    ],
}

# ── CEO 승격 '표지' 핀(MARKER_PINS) — C03 승격 상태 판정 전용 부분집합 ──
# ★불변식: 표지 핀은 **구·신 양쪽 CEO_TEMPLATE 에 실존하는 핀만** 편입한다(R3 A7 실측 —
#   4번째 핀 '직접 구현…'은 구 6KB 템플릿에 부재(grep 0건)라 표지 술어에 넣으면 구판 승격
#   표지가 사멸해 '비정형 승격 상태' FAIL 위경보가 재발한다). 신 템플릿 개정 시 이 불변식은
#   gen_ceo_template.py --check 가 단언한다(스펙 §D2 — 구·신 양쪽 템플릿 실존 필수).
# ★약핀 주석: "exit 7" 은 MASTER_DIRECTIVE 에도 등장하는 약핀이라 단독 판정 금지 —
#   표지 판정은 언제나 3핀 **전수**를 요구한다.
MARKER_PINS = ("master of master", "단일소유 강제", "exit 7")

# C03 소실 FAIL 의 현행 복원 절차 문안(사용자 수정본이 의심될 때의 처방 — 무수정 배포본에는
# _c03_cause_line 의 원인 분류가 이 문안을 대체한다 · R1 시나리오10 오진 차단).
_C03_RESTORE_GUIDE = ("★복원 절차(운영계약 §9-7-6·§11-13): "
                      "①먼저 백업한다 `cp <파일> <파일>.bak-$(date +%Y%m%dT%H%M%S)` "
                      "②주인님께 보고하고 지시에 따라 복구한다. 팩 템플릿 강제 복원은 "
                      "사용자 수정을 무백업으로 덮어쓰는 비가역 조작이라 절대 금지다.")

ROLES = ["master", "worker", "cso", "reviewer"]

# Harness Creator 툴체인(1st-party) 핀 — 2026-06-12 통합 시점 커밋.
# 스킬(pack/skills/harness-creator)은 임베드 배포되지만 이미터·검증기·게놈 툴체인은
# 6MB+ 개발 저장소라 클론 설치한다. 해석 순서: $CYS_HARNESS_HOME → ~/.cys/harness-creator
# → ~/Desktop/CYSjavis/cys-harness-creator(로컬 원본).
HARNESS_REPO = "https://github.com/idoforgod/cys-harness-creator"
HARNESS_PIN = "98a36f4b9aee761f208aa559c2e1f7c755f7c9a6"
HARNESS_KEY_FILES = ("emit_orchestrator.py", "validate_harness.py", "warrant.py",
                     "genome/soul.md")

# NotebookLM SOT 도구(nlm) 핀 — ★2026-09-03 PyPI 정식판으로 전환(PREP #12).
# 종전 핀은 git 커밋 6d41c75(v0.7.3)였다. 그 리비전은 **구 호스트 전용**이라 `--fix` 가 그것을
# 재설치하면 로그인이 되지 않는 버전이 깔린다(CSO 실측: 0.7.3 `nlm login` exit 1). 즉 자동 수리
# 경로가 도구를 고장 난 상태로 되돌리는 형상이었다 — 핀은 '고정'이지 '옛것 유지'가 아니다.
# 지금은 PyPI 에 정식 배포본이 있다(실측 2026-09-03: latest 0.10.0 · 총 128 릴리스 ·
# `curl -s https://pypi.org/pypi/notebooklm-mcp-cli/json`). 이 머신 설치본도 `nlm version 0.10.0`.
# 최소 버전은 0.9.3 — 그 이하는 구 호스트 인증 경로가 남아 있어 login 이 흔들린다(하향 금지).
NLM_MIN_VERSION = (0, 9, 3)
NLM_PIN = "notebooklm-mcp-cli==0.10.0"

TODO_FILES = ["MASTER_TODO.md", "CSO_TODO.md", "WORKER_TODO.md", "REVIEWER_TODO.md"]

# 한국 법령 전용 MCP(korean-law-mcp) 핀 — 2026-06-12 감사(v4.4.1, npm) · 기본 채택.
# k-skill의 korean-law-search(프록시 경유)를 대체하는 전용 경로 — 인용 검증·판례 생사
# 확인(citator)·행위시법 판단 등 환각 방지 기능 내장. 키는 법제처 무료 OC(사람 단계).
KLAW_MIN_VERSION = (4, 4, 1)
KLAW_PIN = "korean-law-mcp@4.4.1"

# ── Serena 코드-의미 인덱스 MCP(uvx 온디맨드 채택 — 미설치) · 2026-06-25 기본 채택 ──
# 심볼 단위 nav(get_symbols_overview/find_symbol/find_referencing_symbols)로 통째-Read·
# 전체-Grep을 대체해 code-nav 슬라이스의 토큰을 줄인다(산문/SOT/설교/markdown=비코드 0).
# 등록은 기계(--fix), 노드 활성화·신뢰(enable/trust)는 사람 전용 단계(denylist).
SERENA_PKG     = "serena-agent"
SERENA_PIN     = "serena-agent==1.5.3"   # server.json 공개판(uvx 해석). 1.5.4.dev0(로컬 dev·PyPI 미배포) 금지
SERENA_PYTHON  = "3.13"                   # server.json runtimeArguments -p 3.13 (requires-python >=3.11,<3.15)
SERENA_CONTEXT = "claude-code"            # Claude 노드용 shipped context(무료 심볼-tool steering). desktop-app 디폴트는 오답
SERENA_PROJECT = os.environ.get("CYS_SERENA_PROJECT") or os.path.expanduser("~/Desktop/CYSjavis")  # 절대경로(노드 spawn·cwd 미보장)·env override·배포 SOT 개인경로 0
# stdio per-node 런치 args — S1+S2 등록 + S8 메모리격리(no-onboarding/no-memories) +
# S4 stray-dashboard 차단(--enable/open-web-dashboard false)을 한 entry로 통합.
SERENA_STDIO_ARGS = [
    "--python", SERENA_PYTHON, "--from", SERENA_PIN, "serena", "start-mcp-server",
    "--context", SERENA_CONTEXT,
    "--transport", "stdio",
    "--project", SERENA_PROJECT,
    "--mode", "no-onboarding",            # S8: 온보딩 write-burst 차단
    "--mode", "no-memories",              # S8: 메모리 tool 전부 drop(javis_memory FileLock SOT 보호)
    "--enable-web-dashboard", "false",    # S4: stray 24282 리스너 제거(stdio 라이브니스=노드 자체)
    "--open-web-dashboard", "false",      # S4: 브라우저 spawn 방지
]

# cys-video-creator 영상 자동제작 스킬(1st-party 32종) — pack 임베드로 배포되고, C26이
# 네이티브 Claude Code(/goal) 발견을 위해 프로필 skills/ 로 심링크한다. 대표 7기둥 +
# 하위 + 공통 규약. 새 스킬 추가 시 이 목록과 pack.rs 임베드 불변식을 함께 갱신한다.
VIDEO_SKILLS = [
    "youtube-video-pipeline", "suite-runtime-keys", "cost-preview-confirm",
    "script-writer", "script-writer-research", "script-writer-structure",
    "script-writer-factcheck", "script-writer-voice-prep",
    "voice-clone-elevenlabs", "voice-clone-elevenlabs-chunk", "voice-clone-elevenlabs-synth-qc",
    "heygen-avatar-render", "heygen-avatar-render-api", "heygen-avatar-render-gate",
    "media-gen", "media-gen-image", "media-gen-edit", "media-gen-video",
    "media-gen-upscale", "media-gen-thumbnail",
    "video-stitch", "video-stitch-compositing", "video-stitch-broll", "video-stitch-captions",
    "audio-post", "audio-post-music", "audio-post-mix",
    "video-verify", "video-verify-visual", "video-verify-timing",
    "video-verify-audio-sync", "video-verify-final-gate",
]
# 영상 파이프라인이 채택하는 공식 벤더 스킬 — `npx skills add`는 cwd의 .agents/skills/에
# 프로젝트-로컬 설치한다(글로벌 아님). 그래서 preflight가 자동 실행하지 않고(엉뚱한 cwd
# 오염 방지) 영상 작업 폴더에서 사람이 1회 실행하는 단계로 안내한다(드리프트 방지·정직성).
VIDEO_VENDOR_COMMANDS = [
    "npx skills add heygen-com/hyperframes   # HyperFrames 모션그래픽 15종",
    "npx skills add elevenlabs/skills        # ElevenLabs 음성",
    "gh skill install heygen-com/skills heygen-video   # HeyGen(선택)",
]
VIDEO_RUNTIME_KEYS = ["ELEVENLABS_API_KEY", "HEYGEN_API_KEY", "FAL_KEY"]

# appbuild 웹/앱 빌드 스킬(1st-party 20종·워커 필수) — 스펙 기반 기획→감독관 검증→자율빌드.
# pack 임베드 배포 + C27이 프로필 심링크 + 코드선행 금지 hook(PreToolUse) 등록.
# 새 스킬 추가 시 이 목록·pack.rs 임베드 불변식을 함께 갱신한다.
APPBUILD_SKILLS = [
    "appbuild", "appbuild-plan", "appbuild-plan-interview",
    "appbuild-plan-debate", "appbuild-plan-quick",
    "appbuild-screen-spec", "appbuild-screen-spec-flow", "appbuild-screen-spec-detail",
    "appbuild-tasks", "appbuild-tasks-slice", "appbuild-tasks-order",
    "appbuild-supervisor", "appbuild-supervisor-collect", "appbuild-supervisor-verify",
    "appbuild-supervisor-fix", "appbuild-supervisor-gate",
    "appbuild-orchestrate", "appbuild-orchestrate-delegate",
    "appbuild-orchestrate-verify", "appbuild-orchestrate-route",
]
APPBUILD_HOOK = "appbuild-gate.sh"  # PreToolUse 코드선행 금지 게이트

# grill-me 최소 질문(결정론 floor) 게이트 — C55가 엔진 self-test·hook·등록·SKILL 핀 검증.
# (제품 절대규칙: grill-me는 합의 전 최소 20·복잡30 결정 브랜치를 강제 해소)
GRILL_ENGINE = "grill_gate.py"
GRILL_HOOK = "grill-gate.sh"            # PreToolUse check(gatekeeper) — distinct<floor면 deny
GRILL_HOOK_EVENT = "PreToolUse"
GRILL_HOOK_MATCHER = "Edit|Write|NotebookEdit"  # Bash 제외(인터뷰 중 탐색 자유)
GRILL_COUNT_HOOK = "grill-count.sh"     # PostToolUse count(evaluator) — distinct 누적
GRILL_COUNT_EVENT = "PostToolUse"
GRILL_COUNT_MATCHER = "AskUserQuestion"
GRILL_ARM_HOOK = "grill-arm.sh"         # PreToolUse(Skill) — grill-me 발동 시 자동 무장(begin)
GRILL_ARM_EVENT = "PreToolUse"
GRILL_ARM_MATCHER = "Skill"
GRILL_STOP_HOOK = "grill-stop.sh"       # Stop — floor 미충족 턴 종료 차단(무쓰기 flow 봉인)
GRILL_STOP_EVENT = "Stop"
GRILL_STOP_MATCHER = ""
# (GATE check hook, count evaluator hook) 쌍 — 둘 다 없으면 게이트가 fail-closed/무력.
# + (arm, stop) 쌍(2026-07-16) — 없으면 무장이 LLM 자발 의존으로 회귀(강제 약화).
GRILL_HOOKS = (
    (GRILL_HOOK, GRILL_HOOK_EVENT, GRILL_HOOK_MATCHER),
    (GRILL_COUNT_HOOK, GRILL_COUNT_EVENT, GRILL_COUNT_MATCHER),
    (GRILL_ARM_HOOK, GRILL_ARM_EVENT, GRILL_ARM_MATCHER),
    (GRILL_STOP_HOOK, GRILL_STOP_EVENT, GRILL_STOP_MATCHER),
)
GRILL_SKILL_PINS = ["AskUserQuestion", "grill_gate", "최소 깊이"]  # pack SKILL 본문 핀
GRILL_AGENTS_DIR = os.path.join(os.path.expanduser("~"), ".agents", "skills")
GRILL_AGENTS_PINS = ["AskUserQuestion", "grill_gate", "20 distinct", "30 for complex"]

# C28 자기교정·영속성 hook(외부 메모리 아키텍처 접목 이관) — (스크립트, [(event, matcher)…]).
# inject/save 는 .config 구체계에서 패키지로 이관, reflect-scan·commit-nudge 는 신규.
SELFCORR_HOOKS = [
    ("inject-context.sh", [("SessionStart", None)]),
    ("save-state.sh", [("Stop", None), ("PreCompact", None)]),
    ("reflect-scan.sh", [("Stop", None), ("SessionEnd", None)]),
    ("commit-memory-nudge.sh", [("PostToolUse", "Bash")]),
    # ★결정론 부트스트랩 발화(제품 절대요구): "너는 마스터다" 선언 입력 시 LLM 재량과
    # 무관하게 하네스가 javis_bootstrap.py(팀 5노드 기동)를 발화 — 산문 계약의 코드 결정론 격상.
    ("role-bootstrap.sh", [("UserPromptSubmit", None)]),
    # ★W-C1(커스텀 생존 2026-07-17): vendor(system·임베드) 팩 파일 수정 감지 → 치유 예고 +
    # 영속 경로 안내(additionalContext WARN — BLOCK 아님·자기발화 봉쇄 금지 경계 준수).
    ("pack-guard.sh", [("PostToolUse", "Write|Edit|MultiEdit")]),
]

# ★훅 **본체** — 실재 전용(등록 대상 아님 · 부트 v2 A2 분할 2026-09-04).
#   `role-bootstrap.sh` 는 자기완결 **런처**이고 실제 부트 본체는 `role-bootstrap-legacy.sh` 다.
#   본체가 없으면 런처는 고지 1줄을 내고 `exit 0` 한다 — 즉 **훅은 정상 종료하는데 부트만 안
#   난다**. 종전에는 어떤 preflight 축도 본체 실재를 재지 않아 이 상태가 **무관측**이었다
#   (부서 팩 복제 목록 결손·부분 배포에서 확정적으로 재현되는 갈래다).
#   ★`SELFCORR_HOOKS` 에 넣으면 안 되는 이유: 그 목록은 C28 이 settings.json 에 **등록**하는
#     집합이다. 본체를 등록하면 같은 이벤트에 훅이 둘(런처+본체) 달려 선언 1건이 두 번 처리되고,
#     본체는 런처가 넘겨주던 `$1` 없이 직접 불려 stdin 계약으로 되돌아간다. 그래서 **실재만**
#     요구하는 목록을 따로 둔다.
#   티어: owner 가 각성 훅(`AWAKENING_SCRIPTS`)이면 **FAIL**(C08·각성 훅 미등록과 대칭) —
#   본체 부재는 등록 결손과 **같은 결과**(부트 발화 0)를 낳으므로 보고 크기도 같아야 한다.
HOOK_BODY_FILES = [
    (os.path.join("hooks", "role-bootstrap-legacy.sh"), "role-bootstrap.sh"),
]

# ★npm_config_prefix **번들 오염** 처방 문안 — 정본은 Rust `npm_prefix_bundle_warning`
#   (src/lib.rs)이고, 그 함수의 독스트링이 "pane 첫 줄 고지와 **preflight 가 같은 문자열**을
#   쓴다 · 문안을 두 벌 두면 한쪽만 고쳐져 사용자가 서로 다른 처방을 받는다"를 계약으로 못박는다.
#   python 에서 Rust 를 호출할 수단이 없으므로 문장을 **옮겨 두되**, 드리프트는 검체가 잡는다
#   (`test_preflight_npm_prefix.py` ⑥ — Rust 원본이 이 레인에 들어오면 핵심 문장 대조가 켜진다).
#   ★값을 덮지 않는다는 것이 계약이다(사용자 설정) — 그래서 이 축은 WARN 이고 처방도 '권함'이다.
NPM_PREFIX_BUNDLE_WARNING = (
    "npm_config_prefix 가 설치본 안을 가리킵니다 — npm 전역 설치가 설치본을 변경하면 "
    "코드서명·봉인이 깨져 다음 실행이 차단될 수 있습니다(그래도 값은 덮지 않습니다 — "
    "사용자 설정입니다). 설치본 밖(예: $HOME/.local · Windows 는 %LOCALAPPDATA%\\cys-npm)으로 "
    "옮기시길 권합니다."
)

# ★소망상태 매니페스트의 파이썬 측 — **각성 티어**(awakening tier · A9 · W3).
#   "없으면 부트 발화 자체가 사라지는" 훅 집합이다: SessionStart(=/clear 후 지침 재주입) +
#   UserPromptSubmit(=마스터 선언 부트 발화). Rust 측 정본은 `src/pack.rs AWAKENING_HOOKS` 이고
#   **집합 대조는 bin/tests/run_bootstrap_health.py H-SEED-1** 이 한다(언어 경계=기계 대조·A11 규율).
#   등록 주체: session-start.sh=C08(FAIL) · role-bootstrap.sh=C28(★A21로 FAIL 티어 격상).
#   ※ 이 목록은 '소망상태'이고 SELFCORR_HOOKS 는 'C28 이 등록하는 집합'이다 — 교집합이
#     role-bootstrap 이며, H-SEED-1 이 그 포함관계까지 단언한다(집행자 누락 방지).
# ★U-21 · 선언 timeout 표(초) — Rust `src/pack.rs` 상수의 파이썬 사본이며 **H-SEED-1 이
#   4-튜플로 기계 대조**한다. 키는 (스크립트, 이벤트).
#
#   왜 필요한가: Claude Code 훅의 `UserPromptSubmit` **기본 timeout 은 30초**다(다른 이벤트는 600초).
#   그리고 timeout 은 지연이 아니라 **취소 + 출력 폐기**다 — role-bootstrap 의 stdout 은
#   additionalContext(팀 기동 사실 + 착수 규율)라서, 30초를 넘긴 순간 부트 체인이 에러도 없이
#   **조용히 사라진다**. 훅 자신의 데드라인 합만 해도 2+5+5+5+10 = 27초라 여유가 3초뿐이고,
#   인터프리터 냉시작이 4회 얹히는 Windows 에서는 상한을 넘는다(mac 만 멀쩡한 그 계열).
#
#   값은 새로 발명하지 않고 **하네스가 다른 모든 이벤트에 쓰는 기본값(600)** 에 맞춘다.
#
# ★상한을 올린 대가와 그 봉인(결함 7 · 2026-08-24 이종 리뷰어): 선언 timeout 을 올리면 훅
#   절단(오살)은 막히지만, 훅 **안**의 데드라인 없는 블로킹 읽기는 상한이 그대로 20배가 된다
#   — `role-bootstrap.sh` 의 `INPUT=$(cat)` 이 그 자리였다(사람 프롬프트 먹통의 상한 = 600초).
#   그래서 그 읽기에 전용 데드라인(`CYS_HOOK_STDIN_TIMEOUT_S`)을 걸었다. **이 표를 올릴 때는
#   반드시 훅 내부 블로킹 지점에 데드라인이 있는지 함께 확인한다** — 선언 timeout 은 상한을
#   '늘리는' 노브이지 '지키는' 노브가 아니다.
HOOK_TIMEOUT_PLATFORM_DEFAULT_UPS_S = 30
HOOK_TIMEOUT_S = {
    ("role-bootstrap.sh", "UserPromptSubmit"): 600,
}

# ★U-21 롤백 스위치(축 1지점) — Rust `pack::hook_timeout_axis_legacy_from` 의 파이썬 미러.
#   축 전용 노브 하나 + **마스터 `CYS_BOOT_GATES=0`** 접기. 사고 순간에 사람이 조합을 만들 수는 없다.
ENV_HOOK_TIMEOUT_V1 = "CYS_HOOK_TIMEOUT_V1"


def hook_timeout_axis_legacy_from(master_env, axis_env):
    """순수 코어(진리표 대상) — `"0"`/`"1"` 엄격 비교(형제 축과 동일 규약)."""
    return master_env == "0" or axis_env == "1"


def hook_timeout_axis_legacy():
    return hook_timeout_axis_legacy_from(os.environ.get("CYS_BOOT_GATES"),
                                         os.environ.get(ENV_HOOK_TIMEOUT_V1))


def hook_timeout_for(script_name, event):
    """이 훅의 **유효 선언 timeout**. 축이 종전이면 None(= 선언 없음 → 판정·쓰기 모두 종전)."""
    if hook_timeout_axis_legacy():
        return None
    return HOOK_TIMEOUT_S.get((script_name, event))


def hook_timeout_satisfied(entry_timeout, declared):
    """선언 충족 판정(순수). **선언은 하한이지 동등이 아니다.**

    · declared None → 아무것도 단언하지 않는다(종전 판정과 동일). 선언하지 않은 축을 '0이어야
      한다'로 읽으면 사용자가 손으로 넣은 값이 전부 불일치가 되어 **우리가 그것을 지운다**.
    · declared 있음 → 엔트리가 그 값 **이상**이어야 충족. 동등(==)을 기각한 이유: 우리보다 큰
      값을 우리 값으로 내리는 것은 취소 시각을 앞당기는 **오살 방향**이다.
    · 키 부재는 **불충족**. 하네스 기본값(30s)을 우리가 읽을 길이 없으므로 '없음=안전'으로 접으면
      원 결함(무음 취소)이 그대로 남는다.
    ★완화가 아니다 — 종전 판정은 timeout 을 아예 보지 않았고(무조건 충족) 이 술어는 더 엄격하다.
    """
    if declared is None:
        return True
    if not isinstance(entry_timeout, (int, float)) or isinstance(entry_timeout, bool):
        return False
    return entry_timeout >= declared


AWAKENING_HOOKS = [
    # (스크립트, [(event, matcher, timeout)…]) — ★timeout 은 **matcher 뒤**(Rust 필드 순서 계약).
    ("session-start.sh", [("SessionStart", None, HOOK_TIMEOUT_S.get(
        ("session-start.sh", "SessionStart")))]),
    ("role-bootstrap.sh", [("UserPromptSubmit", None, HOOK_TIMEOUT_S.get(
        ("role-bootstrap.sh", "UserPromptSubmit")))]),
]
AWAKENING_SCRIPTS = {script for script, _ in AWAKENING_HOOKS}

# work management 앵커(절대지침 5차) 4규칙 b·c의 전담 sub-skill — C22가 존재·본문을 검증한다.
WORK_SKILLS = ["hallucination-guard", "grill-me"]

# 하네스 엔지니어링 운영 스킬 — C29가 3프로필에 자동 심링크(VIDEO/APPBUILD와 동일 규약).
HARNESS_SKILLS = ["harness-engineering"]
# 스킬 본문 핀 — frontmatter만 남기고 본문이 비워지면(전담 기능 소실) 결정론 검출한다.
WORK_SKILL_PINS = {
    # "원출처까지 간다"는 본문 고유 문구 — "출처 진실성"은 frontmatter description과
    # 겹쳐 순서 1 단독 삭제를 못 잡는다(적대 검증 R3).
    "hallucination-guard": ["원출처까지 간다", "근거 적합성", "논리 오류 분석", "팩트체크 판정"],
    "grill-me": ["가정 명시", "분기 질문", "모서리 사냥", "합의 선언"],
}

# 외부 에이전트 운영체계(거버넌스 점유형 스킬 모음)의 결정론 감지 시그니처 — C23.
# 충돌 정의: cysjavis가 배선된 프로필(우리 SessionStart hook 등록)과 **같은 프로필**에
# 동거할 때만 충돌이다 — 전용 프로필 분리 설치는 격리 수칙 준수로 보고 경고하지 않는다.
# (2026-06-12 gstack 감사: 금지가 아니라 'WARN + 격리 수칙 안내'가 목적.)
FOREIGN_AGENT_OS = {
    "gstack": {
        "skills_dir": "gstack",
        "claude_md_markers": ("skills/gstack", "/gstack-upgrade", "/land-and-deploy",
                              "open-gstack-browser"),
        "hook_marker": "gstack",
        "guide": ("격리 수칙: ①cysjavis 프로필이 아닌 전용 CLAUDE_CONFIG_DIR로 이동 "
                  "②CLAUDE.md의 gstack 섹션 제거 ③/ship·/land-and-deploy·"
                  "/gstack-upgrade 사용 금지(커밋 핀 수동 갱신만) ④hook 미등록(클론만)"),
    },
}

# 핀은 '오너 호칭' 규정 라인의 존재다 — 구체 호칭("주인님" 기본값)은 오너가 자유로이
# 바꿀 수 있어야 하므로 특정 단어를 핀으로 삼지 않는다(오너 주권과 결정론의 양립).
SOUL_MARKER = "오너 호칭"
SOUL_PLACEHOLDER = "(이름/호칭을 적어라)"
SOUL_APPEND = (
    "\n## 호칭 (절대지침 — preflight 자동 보강)\n\n"
    '- **오너 호칭: master는 오너를 "주인님"으로 호칭한다** (오너가 다른 호칭을 원하면 이 줄을 수정하라)\n'
)

# 자율주행 위임권(앵커6) — soul이 권한을 부여해야 MASTER §14가 발효된다(이 절이 없으면
# master는 자율주행하지 않는다). 오너가 회수·축소하려면 soul의 이 절을 수정·삭제한다.
# ★자동 재주입 금지(적대 검증 6차 H-2): 아래 골격은 --fix가 쓰지 않는다 — 오너가 권한을
# 다시 부여할 때 수동 복사하는 표준 문안일 뿐이다(부여 주체는 오너뿐).
SOUL_AUTOPILOT_MARKER = "자율주행 위임권"
SOUL_AUTOPILOT_TEMPLATE = """
## 자율주행 위임권 (Autonomous Pilot Mandate — 오너가 master에 부여)

- master는 승인된 로드맵을 오너 수동개입 없이 **자율 완주**할 권한을 가진다
  (MASTER_DIRECTIVE §14 — 3축: 진행권·컨텍스트 수명주기·재기동 루프).
- **★시동 조건 — 이 권한은 오너가 그 세션에 임무를 지정했을 때만 발효한다**(MASTER_DIRECTIVE
  §0-C 임무 게이트). 임무 없는 부팅에서는 대기 중인 작업을 **보고만 하고 멈춘다** — 자율주행은
  "준 일을 끝까지 하는 권한"이지 "줄 일을 스스로 찾는 권한"이 아니다.
- **정지 경계는 위 금지선(denylist)뿐이다**: 로드맵 이탈 새 범위·soul/CLAUDE/디렉티브 변경·
  외부 발행/발송·비가역 삭제·오너 명시 보유 결정권 — 여기서만 멈춰 오너 승인을 받는다.
  로컬 커밋은 가역이므로 허용된다.
- **kill-switch**: 오너의 어떤 입력이든 자율주행을 즉시 일시정지시킨다 — 오너가 항상 우선이다.
- (오너가 이 권한을 회수·축소하려면 이 절을 수정하라 — 이 절이 없으면 master는 자율주행하지 않는다.)
"""

# 자율주행 메모리 상주(앵커6 — 🔒색인 상주 필수) — C25가 파일 존재+본문 핀+색인 등재를
# 검증한다. 본문 핀: 권한·경계 실질이 비워지면(frontmatter만 잔존) 검출(WORK_SKILL_PINS 선례).
AUTOPILOT_MEMORY_FILE = "feedback_autonomous-pilot-mandate.md"
# 핀은 본문 고유 문구만 — "denylist"·"kill-switch"는 frontmatter description과 겹쳐
# 본문 문장 단독 삭제를 못 잡는다(6차 R2 N-4 — 스킬 핀 R3 교훈과 동일 계열).
AUTOPILOT_MEMORY_PINS = ["축1", "축2", "축3", "로드맵 이탈", "오너 아무 입력=즉시 일시정지",
                         "How to apply"]
AUTOPILOT_MEMORY_INDEX_LINE = (
    "- [자율 진행 권한 템플릿](feedback_autonomous-pilot-mandate.md) — **기본 미부여**. "
    "오너가 soul.md에 직접 부여했을 때만 발효하는 3축 계약·denylist에서만 정지·"
    "kill-switch 최우선 (🔒상주 필수 — 제거 금지)"
)


# ★팩 경로 env 키 목록·순서의 단일 상수(A11 · W3). 이 목록이 계약이다 — 먼저 발견되는
#   비어있지 않은 값이 이긴다. 같은 목록·같은 순서를 갖는 구현: src/pack.rs `PACK_DIR_ENV_KEYS`(Rust) ·
#   javis_report · javis_orchestra · javis_todo_stamp · javis_bootstrap · 이 파일(Python).
#   기계 대조: bin/tests/test_todo_shared_constants.py(2언어 전 구현) — 한 곳만 고치면 테스트가 멈춘다.
# ★A11 실측 교정: 종전 이 함수의 목록은 3키(AITERM_PACK_DIR 누락)였는데 docstring 은 "pack.rs 의
#   4단 폴백을 그대로 미러링한다"고 **거짓 주장**했다(재감사 A11: 주장 자체가 자기 증거). 레거시
#   env(AITERM_PACK_DIR)만 설정된 기계에서 preflight 는 홈 기본 팩을, Rust·orchestra 는 레거시 팩을
#   봐서 **검사 대상과 실사용 팩이 갈렸다**. 목록을 4키로 맞추고 주장을 참으로 만든다.
PACK_DIR_ENV_KEYS = ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR")

# ★병합 원장 kind 계약 미러 — 정본(SOT)은 src/pack.rs `LEDGER_KINDS`(유일 등재소).
#   C62/C68 은 이 kind 문자열 계약을 독립 재구현해 왔고, T4 신 kind 'adopted' 분류 누락이
#   실제로 wakeup 일일 재배달 소음(C68)으로 실현됐다(0.14.29 성찰 차단). 미러가 정본과 갈리면
#   bin/tests/test_todo_shared_constants.py 의 census 핀이 멈추고, 신 kind 의 C62/C68 분류
#   미결정은 bin/tests/test_preflight_c62_c68_ledger.py 의 행동 census 가 멈춘다.
MERGE_LEDGER_KINDS = ("healed", "new-pending", "kept-drift", "merged",
                      "conflicted", "quarantined", "adopted")
# C68 기한 제외 kind — kept-drift·merged(at-rest 보존)·adopted(복권 확정 = 스윕 대기 정상 체류).
C68_EXEMPT_KINDS = ("kept-drift", "merged", "adopted")


def pack_dir():
    """pack 위치 결정 — src/pack.rs pack_dir()의 4단 폴백(PACK_DIR_ENV_KEYS)을 그대로 미러링한다."""
    for key in PACK_DIR_ENV_KEYS:
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


def base_pack_dir():
    """C11b 심링크 타깃 전용 — 항상 **base 팩**을 돌려준다(기본 배치에선 `~/.cys/pack`).

    dept 레인에서 pack_dir()(= `~/.cys/pack-dept-<name>` · src/pack.rs lane_pack_for_socket ·
    javis_org.py create_dept 와 동일 규칙)을 링크 타깃으로 잡으면 base↔dept 레인이 번갈아
    --fix 를 돌 때마다 ~/.local/bin/cys-dept 타깃이 플랩한다(REFLECTION-RIPPLE Mac-15).
    부서 팩은 base 의 **형제 디렉터리**이므로 형제 `pack` 으로 승격한다 — 홈 리터럴을 다시
    쓰지 않는 이유는 env 로 재배치된 팩 루트(레인 격리 규약)를 그대로 따라가기 위해서다."""
    p = os.path.normpath(pack_dir())
    if os.path.basename(p).startswith("pack-dept-"):
        return os.path.join(os.path.dirname(p), "pack")
    return p


# ── B7 레인 분리(T-0147-2 §2 층3 · idle-standby-v5 D2/D3) ────────────────────
# 델타게이트 상태(대장·카운터·seen-store·배지)는 **데몬(레인)별로 분리**돼야 한다. 공유하면
# 여러 데몬이 같은 counters 를 밀어 stall 카운터가 배속 증가하고, 한 레인의 배지가 다른 레인의
# 판정처럼 보인다(설계 §0-7 실측: dept-3 데몬이 base 와 report_gate 공유).
#
# ★진짜 계약은 env 이름이 아니라 **파생 규칙**이다: 팩 디렉터리 basename → state_dir.
#   `pack-dept-<id>` → `$HOME/.cys/state/report_gate-<id>` / 그 밖 → `$HOME/.cys/state/report_gate`.
#   파생이 필요한 전 site 가 이 헬퍼 하나만 쓴다(샷건 서저리 봉인). 현재 파생 site:
#     ⓐ C16 배선 bake(신규 append + 마이그레이션 삽입)   ⓑ C69 대장 데드맨 검사
#   신규 site 는 반드시 이 함수를 경유한다.
GATE_STATE_ENV = "CYS_REPORT_GATE_DIR"


def gate_state_dir_for_pack(pack=None):
    """팩 정체 → 델타게이트 레인 state_dir(리터럴 경로). javis_report_gate.default_state_dir 과 동형."""
    base = os.path.basename(os.path.normpath(pack or pack_dir()))
    m = re.match(r"pack-dept-(.+)$", base)
    root = os.path.join(os.path.expanduser("~"), ".cys", "state")
    return os.path.join(root, "report_gate-%s" % m.group(1)) if m \
        else os.path.join(root, "report_gate")


def c03_fingerprint_path():
    """C03 상태 지문 원장 위치(스펙 v4 A6) — 기존 state 관용(~/.cys/state ·
    gate_state_dir_for_pack 과 동일 루트)을 따른다. base 팩 전용이다 — dept/CEO 팩은
    c03_content_pins 의 early-return 으로 승격 로직 자체에 진입하지 않아 레인 분리 불요."""
    return os.path.join(os.path.expanduser("~"), ".cys", "state",
                        "preflight-c03-fingerprint.json")


def gate_command_with_lane(command, state_dir):
    """게이트 잡 command 에 인라인 env 프리픽스를 **삽입만** 한다(재생성 금지).

    ★기존 토큰을 절대 잃지 않는 것이 계약이다: 실측상 일부 레인의 잡은 `run --shadow` 를 보유하고
      있고, 템플릿 재생성 방식은 그것을 조용히 소실시킨다(idle-standby-v5 D2 3항). 그래서
      "앞에 붙이기"만 하고 나머지 문자열은 바이트 그대로 둔다. 멱등(이미 있으면 무동작)."""
    if GATE_STATE_ENV in (command or ""):
        return command
    return '%s="%s" %s' % (GATE_STATE_ENV, state_dir, command)


def _cys_hook_cmd(script_name):
    """Claude settings.json hook 명령 문자열(단일 진실 — 모든 등록부 공용).
    Windows: git-bash `bash`로 명시 호출 + **정슬래시 + 따옴표**. 미따옴표 역슬래시 경로는 bash가
    escape로 먹어 경로가 파괴되고(C:\\Users\\...\\hooks\\cys-hook.sh → C:Userscys.cys/packhookscys-hook.sh
    → No such file), 따옴표 없이는 공백·역슬래시가 깨진다(실측 회귀 — 전 hook No such file 폭주).
    Unix: 기존 `sh <abs>` 무변경(회귀0 — 기존 install 문자열·matcher 그대로 유지)."""
    script = os.path.join(pack_dir(), "hooks", script_name)
    if os.name == "nt":
        return 'bash "%s"' % script.replace("\\", "/")
    return "sh " + script


# ★G10(W3): '우리 훅인가' 판정의 소유 술어. 종전 술어는 `script_name in c and "hooks" in c` 라는
#   **부분문자열 2개**였다 — 사용자가 자기 훅을 `~/myhooks/inject-context.sh`(또는 어떤 경로든
#   'hooks' 문자열을 포함하는 곳)에 두면 우리 등록기가 그것을 '우리 파손 엔트리'로 보고 **무음 삭제**
#   했다(사용자 설정 파괴 · 되돌릴 근거도 안 남는다). 판정은 두 조건의 **동시 충족**으로 좁힌다:
#     ⓐ 경로 꼬리가 정확히 `/hooks/<script_name>` (구성요소 경계 — `/myhooks/…` 는 탈락)
#     ⓑ 그 경로가 **cys 관리 루트** 아래(현재 팩 접두 또는 알려진 cys 레거시 루트)
#   ⓑ 없이 ⓐ만 쓰면 `~/mytools/hooks/inject-context.sh` 같은 제3자 훅이 여전히 삭제 대상이 되고,
#   ⓐ 없이 ⓑ만 쓰면 원 결함(부분문자열)이 남는다. 구 cys 경로(.config/cysjavis 등)의 죽은 엔트리는
#   ⓑ의 레거시 루트 목록으로 계속 회수한다(회귀 0 — 그 청소가 중복 append 방지의 본래 목적).
_CYS_LEGACY_HOOK_ROOTS = ("/.cys/", "/.config/cysjavis/", "/.aiterm/", "/cysjavis-pack/")


def _hook_cmd_paths(cmd):
    """hook command 문자열에서 경로 후보를 뽑는다(정슬래시 정규화). `sh <abs>` / `bash "<abs>"` /
    env 접두(VAR=x sh …) 등 배포된 형태를 모두 커버 — 토큰 단위로 훑고 따옴표만 벗긴다."""
    norm = (cmd or "").replace("\\", "/")
    out = []
    for tok in norm.replace('"', " ").replace("'", " ").split():
        if "/" in tok:
            out.append(tok)
    return out


def _hook_entry_is_ours(cmd, script_name, pack_hooks_prefix):
    """순수 판정: 이 hook command 가 **우리 팩의 script_name 훅**을 가리키나(G10 소유 술어)."""
    tail = "/hooks/" + script_name
    for path in _hook_cmd_paths(cmd):
        if not path.endswith(tail):
            continue                     # ⓐ 경로 꼬리 정확 매치(구성요소 경계)
        if path.startswith(pack_hooks_prefix):
            return True                  # ⓑ-1 현재 팩(레인 팩 포함) 접두
        if any(root in path for root in _CYS_LEGACY_HOOK_ROOTS):
            return True                  # ⓑ-2 알려진 cys 레거시 루트(죽은 엔트리 회수)
    return False


def _prune_stale_hook_entries(arr, script_name, desired):
    """event hook 배열에서 **우리 팩의** script_name 훅을 참조하되 desired와 다른(구·파손)
    엔트리를 제거한다. return (정리된 리스트, desired 존재 여부). 비-cys 엔트리·사용자 동명 훅·
    타 스크립트·정상 엔트리는 보존 → in-place 업그레이드 시 파손 항목만 교체."""
    prefix = (os.path.join(pack_dir(), "hooks") + os.sep).replace("\\", "/")
    kept, have = [], False
    for entry in arr:
        if not isinstance(entry, dict):
            kept.append(entry)
            continue
        cmds = [h.get("command", "") for h in entry.get("hooks", []) if isinstance(h, dict)]
        ours = any(_hook_entry_is_ours(c, script_name, prefix) for c in cmds)
        if not ours:
            kept.append(entry)
        elif desired in cmds:
            kept.append(entry)
            have = True
        # else: 우리 hook이나 desired와 불일치(구·파손 역슬래시·미따옴표) → 제거(교체 유도)
    return kept, have


def _utf8_env(extra=None):
    """자식 프로세스 텍스트 I/O를 UTF-8로 고정한 env (AgentReach utf8_subprocess 계약 클린룸 포트).

    부모 로케일(Windows cp949/cp936)이 아니라 UTF-8로 디코드/인코드하도록 PYTHONUTF8/
    PYTHONIOENCODING을 강제한다. 엔진(skills/insane-search/engine/proc.py)을 import 하지 않고
    독립 재구현한다 — preflight(pack/bin)는 엔진을 import 하면 안 된다(레이어 역전·배포 경계).
    불변식: 멱등(이미 적용된 env에 재적용해도 동일)·비파괴(운영자 명시 LC_ALL/LANG은 setdefault로
    보존)·순수(os.environ 미변경, copy만). 부작용 0(PHIL-04)."""
    env = os.environ.copy()
    env["PYTHONUTF8"] = "1"
    env["PYTHONIOENCODING"] = "utf-8"
    env.setdefault("LC_ALL", "C.UTF-8")   # 운영자 명시값은 보존(fail-safe)
    env.setdefault("LANG", "C.UTF-8")
    if extra:
        env.update(extra)
    return env


def heartbeat_verdict(mtime, now_ts, max_age):
    """[C79·R6 W0-5] 검증자 heartbeat 신선도 판정(순수) — (state, age).

    state ∈ absent(파일 부재) | fresh(age ≤ max_age) | stale. 부등호는 autopilot 게이트6
    (`(now_ts - hb) <= HEARTBEAT_MAX_AGE`)과 동일하게 ≤ 다(경계 판정 드리프트 금지).
    미래 mtime(시계 스큐)은 age<0 ≤ max_age 로 fresh — 방금 touch 된 파일을 노화로
    오판해 불필요한 재기동을 트리거하지 않는 보수측."""
    if mtime is None:
        return "absent", None
    age = now_ts - mtime
    return ("fresh" if age <= max_age else "stale"), age


def _cycle_autopilot_mod():
    """javis_cycle_autopilot 형제 모듈 import — HEARTBEAT 경로·HEARTBEAT_MAX_AGE 의 SOT.

    경로·수치를 여기 복제하면 autopilot 개정 때 조용히 드리프트한다 — import 가 유일한
    무드리프트 채널이다(_SELF_DIR 이 sys.path 에 있어 pack/bin·레포 양쪽에서 성립).
    import 는 상수 정의·경로 해석뿐(파일 쓰기 0 — 부작용 없음). 실패=None(C79 가 WARN)."""
    try:
        import javis_cycle_autopilot as _ap
        return _ap
    except Exception:  # noqa: BLE001 — 팩 스큐·부분 배포는 체크를 죽이지 않는다
        return None


def is_dept_pack():
    """부서/CEO pack 컨텍스트인가 — pack_dir이 기본(~/.cys/pack)이 아니면 부서/CEO 데몬이다.
    부서장·CEO의 MASTER_DIRECTIVE는 표준 핀이 없는 게 정상이라 C03 표준 핀 검사를 면제한다
    (멀티마스터 정식화 F1 — 부서 운영 중 C03 영구 FAIL→`--force` 복원이 CEO 디렉티브를 파괴하는 것 차단)."""
    default = os.path.join(os.path.expanduser("~"), ".cys/pack")
    try:
        return os.path.realpath(pack_dir()) != os.path.realpath(default)
    except OSError:
        return False


def _path_under_tempdir(p):
    """p 가 임시 디렉터리(/tmp·/private/tmp·$TMPDIR·/var/folders·/private/var/folders) 아래인가.
    임시 pack 예방 가드(discover_claude_settings)와 C57 temp-훅 청소기의 공용 술어.
    symlink 정규화 위해 양변 realpath 비교(macOS /tmp→/private/tmp·/var→/private/var)."""
    try:
        rp = os.path.realpath(p)
    except OSError:
        return False
    roots = ["/tmp", "/private/tmp", "/var/folders", "/private/var/folders",
             os.environ.get("TMPDIR")]
    try:
        roots.append(tempfile.gettempdir())
    except Exception:
        pass
    for r in roots:
        if not r:
            continue
        try:
            rr = os.path.realpath(r).rstrip("/")
        except OSError:
            rr = r.rstrip("/")
        if rp == rr or rp.startswith(rr + os.sep):
            return True
    return False


# ── 부서 판정 술어 통일표 (2언어 · G3 축1 확정 — H-SEED-6 파리티 핀이 기계 대조) ──────────
# 같은 "부서인가" 질문에 **새 술어를 추가하지 마라**(제3 술어 중복 = 드리프트 원천 — C28 부서
# 게이트 신설안은 이 이유로 기각·2026-08 확정). 현존 술어와 소관:
#   Python ① `_discover_isolation_block._pack_is_dept` — basename 'pack-dept-' 접두(2026-06-30).
#             소관: 훅 **등록(쓰기) 금지/좁힘** 게이트(C28 포함 모든 등록기가 resolve_registration_
#             targets 경유로 이 게이트를 소비한다 — C28 에 별도 게이트 불요).
#          ② `is_dept_pack` — pack_dir ≠ 기본(~/.cys/pack). CEO·임시 팩 포함 **광의**(C03 면제용).
#          ③ C56 `_dept_hooks_in` — 훅 command 의 '/pack-dept-' 경로 앵커. 소관: 글로벌 누수
#             invariant **탐지**(+레거시 청소 — 기존 거동 유지).
#   Rust   ④ `pack::dept_scope_of` — basename `pack-dept-` 접두(①과 동일 규칙 · 파리티 핀 대상).
#             소관: config 시드 표적 판정·hooks-prune 게이트·init-pack 부서 게이트.
#          ⑤ `factory_reset::command_points_into_pack` — `<base>/pack` 경계+`-dept-` 꼬리(리셋 광의).
# **제거 엔진은 `cys hooks-prune`(Rust `strip_hooks_pointing_into_pack`) 단일**이다 — 부서 잔존
# 훅의 신규 제거 배선은 파이썬에 늘리지 않는다(teardown 은 cys-dept down 이 hooks-prune 을 호출).
def _discover_isolation_block():
    """등록 금지(격리) 컨텍스트인가 — (사유|None, 좁힌 대상|None).

    ★G1(W3) sentinel 의 근거: 종전 이 판정은 `[]` 를 반환했고, 호출부가 `discover() or [~/.claude]`
      로 폴백해 **금지가 통째로 무효화**됐다(부서·임시 팩이 실 글로벌 settings 에 자기 훅을 등록).
      `[]`(순수 미발견 — 신규 머신) 와 `금지`(격리 팩)는 **정반대 처방**이므로 융합하면 안 된다:
      전자는 폴백 생성이 정답, 후자는 등록 0 이 정답이다. 그래서 판정을 여기서 분리해 낸다.
    """
    _acct = os.environ.get("CYS_ACCOUNT_DIR")
    # 축A 근본복원(2026-06-30): 부서 데몬 컨텍스트(CYS_ACCOUNT_DIR이 부서 전용 dir·basename에
    # 'dept-')에서는 home-glob을 생략하고 자기 account_dir settings.json에만 hook을 등록한다.
    # home-glob을 그대로 두면 부서 preflight가 CEO(CEO 프로필 config)·타부서 config에까지
    # hook을 append해 dept-3·dept-4처럼 무한 재발한다(부서장 config 공유와 무관한 절차 버그).
    _acct_is_dept = bool(_acct and "dept-" in os.path.basename(os.path.normpath(_acct)))
    # R2 강화(2026-06-30): CYS_ACCOUNT_DIR 누락 엣지에서도 pack_dir이 pack-dept-* 면 부서로 판별 →
    # home-glob 진입을 차단해 CEO·타부서 settings 재누수를 env 비의존으로 원천 봉쇄.
    _pack_is_dept = "pack-dept-" in os.path.basename(os.path.normpath(pack_dir()))
    if _acct_is_dept or _pack_is_dept:
        if _acct and os.path.isdir(_acct):
            # 좁힌 대상(자기 account dir)만 허용 — 글로벌은 금지 상태 그대로다.
            return ("부서 팩/계정 컨텍스트 — 자기 account dir 한정 등록",
                    [os.path.join(_acct, "settings.json")])
        return ("부서 팩 컨텍스트인데 account_dir 미상 — 글로벌 등록 절대 금지"
                "(부서 settings 는 cys-dept 가 생성)", [])
    # 2026-07-02 근본복원(temp-pack 누수): grill embed/스냅샷 하네스가 CYS_PACK_DIR=/tmp/snap_grill_* 로
    # preflight --fix 를 돌리면 home-glob을 타고 실 글로벌 settings에 /tmp 세션훅을 등록 → temp가 비거나
    # 사라지며 "No such file" 무한재발. dept 가드의 짝: 임시 pack은 실 config에 절대 등록 금지
    # (스냅샷/테스트 부작용 0). 이미 등록된 잔해 청소는 C57.
    if _path_under_tempdir(pack_dir()):
        return ("임시 팩 컨텍스트(%s) — 실 config 등록 절대 금지" % pack_dir(), [])
    return (None, None)


def discover_claude_settings():
    """$HOME 직하 .claude* **프로필 디렉터리**의 settings.json 전부(사전순) + 실사용 config dir.

    반환은 **항상 리스트**다(읽기 전용 소비자 계약 — raise 없음·None 없음). 등록(쓰기) 소비자는
    `resolve_registration_targets()` 를 써야 한다 — 그쪽만 '금지'와 '미발견'을 구분한다(G1).

    · ★R4(W3): `CLAUDE_CONFIG_DIR`(현재 세션의 **실사용** config dir)이 있으면 **최우선**으로 넣는다.
      종전엔 이 env 를 아예 보지 않아, 실제로 훅이 필요한 그 디렉터리가 등록 대상에서 빠질 수 있었다
      (등록≠가동 갭의 절반 — A21 재검증 R4).
    · ★G7(W3): 후보 기준은 **디렉터리 존재**다. 종전 `isfile(settings.json)` 게이트는 파일이 아직
      없는 프로필을 영구 미배선으로 굳혔다(등록기는 makedirs+create 를 이미 한다 — cys.rs
      `discover_claude_settings` 와 동일 규칙).
    · cys 계정 config dir(${CYS_ACCOUNT_DIR:-dirname(pack_dir())/claude})을 append 한다 —
      agents.json claude.env 의 CLAUDE_CONFIG_DIR 해석(C31)과 같은 규약이다.
    · agy/codex 는 Claude-config 노드가 아니므로 미대상.
    """
    reason, narrow = _discover_isolation_block()
    if reason is not None:
        return list(narrow or [])
    home = os.path.expanduser("~")
    found = []
    seen = set()

    def _add(path):
        try:
            key = os.path.realpath(path)
        except OSError:
            key = path
        if key in seen:
            return
        seen.add(key)
        found.append(path)

    # ★R4: 실사용 config dir 최우선(디렉터리 존재 시에만 — 없는 경로를 만들지 않는다).
    ccd = os.environ.get("CLAUDE_CONFIG_DIR", "").strip()
    if ccd and os.path.isdir(ccd):
        _add(os.path.join(ccd, "settings.json"))
    try:
        names = os.listdir(home)
    except OSError:
        names = []
    for n in sorted(names):
        if n == ".claude" or n.startswith(".claude-"):
            d = os.path.join(home, n)
            if os.path.isdir(d):                     # ★G7: 디렉터리 존재 기준
                _add(os.path.join(d, "settings.json"))
    # cys 계정 config dir(master + ~/.cys/claude 공유 claude-adapter 리뷰어) 포함.
    # env-first: dept 데몬은 CYS_ACCOUNT_DIR=~/.cys/claude-<key>로 기동되므로 자기 dir이 정답
    # (하드코딩 ~/.cys/claude는 dept 오타깃). 디렉터리 존재 시에만 포함(미기동 노드 config dir 생성 방지).
    try:
        account_dir = os.environ.get("CYS_ACCOUNT_DIR") or os.path.join(
            os.path.dirname(os.path.normpath(pack_dir())), "claude")
        if account_dir and os.path.isdir(account_dir):
            _add(os.path.join(account_dir, "settings.json"))
    except Exception:
        pass  # 부재/이상 dir → home-glob만 반환(preflight 부트 게이트라 crash 금지)
    return found


def resolve_registration_targets():
    """훅 **등록(쓰기)** 대상 해소 — `(targets, forbidden_reason)`.

    · `forbidden_reason` 이 not-None 이면 **글로벌 폴백 금지**다(targets 는 좁힌 목록 또는 빈 목록).
    · None 이면 targets 는 발견 목록이며, 순수 미발견(신규 머신)일 때만 기본 프로필
      `~/.claude/settings.json` **한 건**으로 폴백한다(등록기가 생성한다).
    ★G1: 이 반환 계약이 '금지'와 '미발견'의 융합을 절단한다 — 종전 `discover() or [기본]` 관용구는
      금지 컨텍스트에서도 기본 프로필로 폴백해 격리를 무효화했다(H-EXIT-8).
    """
    reason, narrow = _discover_isolation_block()
    if reason is not None:
        return (list(narrow or []), reason)
    targets = discover_claude_settings()
    if targets:
        return (targets, None)
    return ([os.path.join(os.path.expanduser("~"), ".claude", "settings.json")], None)


# ── settings.json read-modify-write 의 단일 소유자 (G16 · W3) ────────────────────
# ★실측 정정(N13 · 2026-08-24): 이 자리는 "settings.json 은 **3-writer 대상**" 이라고 단언하고
#   있었다. 그 수는 틀렸다 — 전수로 세면 **5-writer** 다. 거짓 단언을 남기는 것이 가장 싸고
#   가장 나쁘므로 세어서 적는다(경로·락 여부는 검증 가능해야 하니 `파일:라인`으로 남긴다):
#     ① cysjavis-pack/bin/javis_preflight.py:_settings_rmw  — 락 O(`<settings>.cys-lock`)
#        · 등록기 4종(_register_hook/_register_statusline/_register_event_hook/
#          _register_appbuild_hook = C08/C27/C28/C32/C33/C41)이 전부 이 하나를 경유한다.
#     ② cysjavis-pack/bin/javis_guard_register.py:_atomic_write(호출 `:502`·`:535`)
#        — 락 X · mkstemp+replace O(교차 파손은 없고 lost-update 만 남는다)
#     ③ cysjavis-pack/bin/javis_dept_migrate.py:_register_hook(부서 account settings 백필)
#        — 락 X · **고정 `.tmp`**(아래가 preflight 에서 걷어낸 바로 그 교차 파손 형태가 그대로 있다)
#     ④ src/pack.rs:merge_desired_hooks(Rust 시드·init-pack)                — 락 X · write_atomic O
#     ⑤ src/factory_reset.rs:strip_settings_matching(외과 제거·부서 해제)   — 락 X · write_atomic O
#   즉 **락으로 직렬화되는 writer 는 5 중 1**이고, 나머지 넷은 원자 교체까지만 한다.
#   ★남은 위험을 정직하게: 원자 교체는 반쪽 JSON 을 막을 뿐 **lost update** 를 막지 못한다
#     (동시 RMW 두 벌 중 뒤에 쓰는 쪽이 앞의 등록을 통째로 덮는다). 그 창을 닫으려면 락을
#     5 writer 전원이 공유해야 하고, Windows 에는 `LockFileEx` 승격이 필요하다 — 반경이 커서
#     이번 태그에 넣지 않는다(릴리스 노트 항목). 여기서 하는 일은 **수를 사실로 되돌리는 것**뿐이다.
# ★결함(G16): 그런데 preflight 의 네 등록기가 각자
#   `open(path + ".tmp")` → `os.replace` 를 재구현했고 tmp 이름이 **고정**이었다: 동시 writer 가
#   서로의 임시 파일에 써서 **교차 파손**(반쪽 JSON)을 만들고, 그러면 그 뒤 모든 등록기가
#   "파싱 실패 — 덮어쓰기 거부"로 수리를 영구 포기한다(A8 재검증이 지목한 지배 실패 모드).
#   Rust 측 원자화는 W2(A8rs)에서 착지했고, python 측 락·mkstemp 유틸은 W1a(javis_lock)에서
#   신설됐다 — W3 은 그 유틸의 **소비 이관**이다(신설 1회 + 소비 이관 = 원자 단위·비평1 #19).
# 계약: ①레인 무관 **파일별 락**(`<settings>.cys-lock`) — 이 락을 잡는 writer 끼리만 직렬화된다
#   (위 실측표의 ①뿐이고 ②~⑤는 아직 이 락을 잡지 않는다) ②symlink 거부
#   ③파싱 실패 = 거부(빈 dict 로 대체 금지) ④최초 1회 백업 보존 ⑤mkstemp+replace 원자 교체.
#   락 사용 불가(백엔드 부재)여도 **쓰기는 진행**한다 — 직렬화 상실은 열화이고, 등록 자체를
#   포기하면 훅이 사라진다(가용성 우선·조용하지 않게 사유를 반환값에 싣지 않고 stderr 로 남긴다).
def _settings_rmw(settings_path, mutate, indent=2):
    """settings.json 을 락 아래에서 읽고-바꾸고-원자적으로 쓴다.

    `mutate(data) -> None|str` : data(dict)를 제자리 수정. 문자열을 반환하면 그 사유로 중단(무쓰기).
    반환: None=성공 / 문자열=실패 사유(호출자가 FAIL·WARN 으로 보고).
    """
    if os.path.islink(settings_path):
        return "symlink 거부(실파일만 허용): %s" % settings_path
    d = os.path.dirname(settings_path)
    if d:
        try:
            os.makedirs(d, exist_ok=True)
        except OSError as e:
            return "설정 디렉터리 생성 실패: %s (%s)" % (d, e)
    lock = None
    if _lock is not None:
        try:
            lock = _lock.FileLock(settings_path + ".cys-lock", owner="preflight-settings",
                                  blocking=True, timeout=10.0, soft=True)
            lock.acquire()
            if lock.status != _lock.ACQUIRED:
                sys.stderr.write("[preflight] settings 락 미획득(%s: %s) — 직렬화 없이 진행: %s\n"
                                 % (lock.status, lock.detail, settings_path))
        except Exception as e:      # 락 인프라 고장이 등록을 죽이지 않는다
            sys.stderr.write("[preflight] settings 락 사용 불가(%s) — 직렬화 없이 진행\n" % e)
            lock = None
    try:
        data = {}
        if os.path.isfile(settings_path):
            try:
                with open(settings_path, encoding="utf-8") as f:
                    data = json.load(f)
            except (OSError, ValueError) as e:
                return ("기존 settings.json 파싱 실패 — 덮어쓰기 거부(수동 복구 필요): %s (%s)"
                        % (settings_path, e))
            if not isinstance(data, dict):
                return "settings.json 루트가 객체가 아님 — 거부: %s" % settings_path
            # 최초 백업만 보존 — 재실행이 정상 백업을 손상 상태로 덮어쓰는 것을 차단.
            backup = settings_path + ".bak-preflight"
            if not os.path.exists(backup):
                try:
                    shutil.copy2(settings_path, backup)
                except OSError as e:
                    return "백업 생성 실패(쓰기 중단): %s (%s)" % (backup, e)
        err = mutate(data)
        if err:
            return err
        body = json.dumps(data, ensure_ascii=False, indent=indent)
        if _lock is not None:
            _lock.atomic_write_text(settings_path, body)
        else:                       # 팩 스큐(javis_lock 부재) — mkstemp 인라인 폴백
            fd, tmp = tempfile.mkstemp(dir=d or ".", prefix=".tmp-settings-")
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                f.write(body)
            os.replace(tmp, settings_path)
        return None
    finally:
        if lock is not None:
            try:
                lock.release()
            except Exception:
                pass


class Preflight:
    def __init__(self, fix, skips, mode="report", allow_irreversible=False):
        # OPP-17: mode ∈ report(관찰만)|fix(집행)|dry(미리보기)|safe(무변경+갭만).
        # self.fix 는 *집행 모드일 때만* True — dry/safe 에선 False 라 기존 50+ `if self.fix and …`
        # 가역 부작용 분기(c04 soul·c07 hook·c08 settings·c10 todo·c32 statusline·c33 event_hooks
        # 등)가 self.fix=False 로 **일괄 비집행**된다. may_mutate() 게이트는 *비가역 외부설치*
        # (denylist external_install)만 명시 미리보기/차단한다(아래 게이트 docstring 참조).
        # back-compat: 호출자가 mode 미지정 시 fix 인자로 report/fix 결정(기존 시그니처 보존).
        if mode == "report" and fix:
            mode = "fix"
        self.mode = mode
        self.fix = (mode == "fix")
        self.allow_irreversible = allow_irreversible
        # planned: may_mutate() 가 기록하는 *비가역 외부설치* 계획 버퍼. 가역 로컬 변경(soul/hook/
        # settings/todo 등)은 self.fix=False 로 일괄 비집행되므로 이 버퍼에 기록되지 않는다(정직 범위).
        self.planned = []
        self.skips = set(skips)
        self.results = []
        self._init_pack_ran = None  # None=미시도, True/False=시도 결과
        # report 모드 병렬화용 sink 격리: 병렬 워커 스레드는 자기 버퍼에 add() 하고
        # run() 이 원래 순서로 재조립한다(직렬 경로는 sink=None 으로 self.results 직행).
        self._local = threading.local()

    def add(self, cid, status, detail):
        sink = getattr(self._local, "sink", None)
        target = self.results if sink is None else sink
        target.append({"id": cid, "status": status, "detail": detail})

    def skipped(self, cid):
        if cid in self.skips:
            self.add(cid, SKIP, "skipped by --skip")
            return True
        return False

    # ── OPP-17 비가역 외부설치 Mutation 게이트 — "관찰이 상태를 바꾸지 않는다"(PHIL-04) 동형 ──
    # ★범위 정직(적대검증 REVISE 교정): may_mutate() 는 **비가역 외부설치(denylist external_install
    # — npm install -g·git clone) 전용 게이트**다. 가역적 로컬 변경(soul/hook/settings/todo/
    # statusline/event_hooks 등 50+ `if self.fix` 분기)은 may_mutate() 를 거치지 않고, dry/safe
    # 모드에서 self.fix=False 로 **일괄 비집행**된다(자연 게이팅). 따라서 dry/safe 의 무변경
    # 보장은 전 사이트에 성립하나, `--json` planned 미리보기 충실성은 비가역 외부설치 2건에
    # 한정된다 — "단일 Mutation 게이트·전 사이트 1:1 미리보기" 는 의도적으로 *주장하지 않는다*.
    # 비가역만 게이트한 이유: 가역(.bak·kill·재실행 가능)은 dry/safe 비집행으로 충분하고, 비가역은
    # 사전 확인·denylist 차단이 *반드시* 필요하기 때문(자율주행 denylist ④ 정합).
    def may_mutate(self, cid, kind, target, summary, denylist_class=None):
        """이 *비가역 외부설치* 부작용을 지금 집행해도 되는가? 계획 기록 + 모드별 분기. 집행=True."""
        self.planned.append({"cid": cid, "kind": kind, "target": target,
                             "summary": summary, "denylist_class": denylist_class})
        if self.mode == "dry":
            self.add(cid, DRYRUN, "[dry-run] Would %s → %s" % (kind, target))
            return False
        if self.mode == "safe":
            self.add(cid, SAFE_GAP, "[safe] 누락: %s (무변경 — 수리 보류)" % summary)
            return False
        if self.mode == "fix":
            if denylist_class and not self.allow_irreversible:
                # 첫 도입 WARN-first(BLOCK 아님) — 부트 ⓪ 표준 호출 회귀(NOT READY) 방지.
                self.add(cid, WARN, "비가역 변경(%s) 보류 — --allow-irreversible 없이 자동집행 안 함: %s"
                         % (denylist_class, target))
                return False
            return True
        return False  # report 모드: 관찰만, 부작용 없음

    # ── 공용 수리: cys init-pack — 누락 항목 재설치 + 비수정 파일 신버전 갱신 + **수정된
    #    system 파일은 제자리 보존(kept-drift — 벤더 미전진)·벤더 전진 시 자동 3-way 병합**(충돌 =
    #    vendor 본 채택 + 수정본 <rel>.user 보존·C62 원장 보고). 보안 잠금(trusted-keys.json)만
    #    즉시 치유. 불가침은 user-owned(디렉티브·soul·CLAUDE·schedule)·seed-once(memory/·round
    #    상태)뿐이다.
    #    (구 문구 "사용자 수정본 불가침"은 user-owned에만 참 — 오독이 실사고를 낳아 시정.) ──
    def repair_via_init_pack(self):
        if self._init_pack_ran is not None:
            return self._init_pack_ran
        cys = shutil.which("cys")
        if not cys:
            self._init_pack_ran = False
            return False
        try:
            r = subprocess.run(
                [cys, "init-pack", "--no-install-hook"],
                capture_output=True, timeout=30,
            )
            self._init_pack_ran = r.returncode == 0
        except Exception:
            self._init_pack_ran = False
        return self._init_pack_ran

    # ── C01 pack 디렉터리 ──
    def c01_pack_dir(self):
        cid = "C01.pack-dir"
        if self.skipped(cid):
            return
        d = pack_dir()
        if os.path.isdir(d):
            self.add(cid, PASS, d)
            return
        if self.fix and self.repair_via_init_pack() and os.path.isdir(d):
            self.add(cid, FIXED, "%s (cys init-pack로 생성)" % d)
            return
        self.add(cid, FAIL, "%s 없음 — `cys init-pack` 실행 필요" % d)

    # ── C02 디렉티브 4종 존재·비어있지 않음 ──
    def c02_directives(self):
        cid = "C02.directives"
        if self.skipped(cid):
            return
        missing = []
        for f in DIRECTIVES:
            p = os.path.join(pack_dir(), "directives", f)
            if not (os.path.isfile(p) and os.path.getsize(p) > 0):
                missing.append(f)
        if missing and self.fix and self.repair_via_init_pack():
            missing = [
                f for f in missing
                if not os.path.isfile(os.path.join(pack_dir(), "directives", f))
            ]
            if not missing:
                self.add(cid, FIXED, "누락 디렉티브 재설치 완료")
                return
        if missing:
            self.add(cid, FAIL, "누락/빈 파일: %s" % ", ".join(missing))
        else:
            self.add(cid, PASS, "4종 디렉티브 존재·비공백")

    # ── C03 내용 핀 — 두 축 분리 재구성(스펙 v4 §D3(ii)+A6 · base 팩 한정 승격 로직) ──
    #
    # [핀 축] C03.pin.<role> — 절대지침 조항이 문서에 살아있는가. master 는 승격 상태
    #   결정론 판정(표지 = 바이트 등가 > 영수증 > 표지 3핀 폴백)을 포함한다.
    # [건전성 축] C03.demote-guard — 승격 표지 시 항상, 핀 결과와 **독립**으로 .pre-ceo
    #   (강등 백업)의 건전성을 검사한다(R2 E2 봉합: 핀 축 ① 조기 PASS 가 강등-불능 검출을
    #   영구 차폐하던 결함).
    # ★C03 에는 --fix 경로가 절대 없다(계약): 디렉티브·.pre-ceo 는 user-owned 헌법 파일이라
    #   preflight 가 편집하지 않는다 — 가시화만(부트 ⓪ '팩 템플릿 강제 복원 절대 금지' 동일 원칙).

    @staticmethod
    def _c03_read_bytes(path):
        """바이트 등가(cmp 동형 — 개행 정규화 없음) 판정용 원시 읽기. 부재/불가 = None."""
        try:
            with open(path, "rb") as f:
                return f.read()
        except OSError:
            return None

    @staticmethod
    def _c03_cmp_label(a, b):
        """cmp 결과 라벨 — 비정형 FAIL detail 의 진단 첨부용(파괴적 조치 대신 관찰 제공)."""
        if a is None or b is None:
            return "판정 불능(파일 부재)"
        return "등가" if a == b else "부등"

    def _c03_receipt_hash(self, receipt_path):
        """승격 영수증(directives/.ceo-template-applied) 해시 추출 — cys-dept `_swap` 이
        기록하고 `ceo_demote` 가 삭제한다(스펙 결정 D3 · pack 트리 내부 = Tier1 백업 원자성,
        ~/.cys/state 금지). 내용 규약 = 적용 시점 md sha256 hex. 관용 파서: 본문에서 첫
        64자리 hex 토큰만 취한다(개행·JSON 래핑 허용). 없으면 None."""
        try:
            with open(receipt_path, encoding="utf-8", errors="replace") as f:
                text = f.read()
        except OSError:
            return None
        m = re.search(r"\b[0-9a-fA-F]{64}\b", text)
        return m.group(0).lower() if m else None

    def _c03_cause_line(self, f, live_b):
        """FAIL 원인 분류 1줄(R1 시나리오10 — 오진 처방 차단): live==.pristine/directives/<f>
        등가면 벤더 동기 문제(사용자 과실 아님 — '사용자 소실 복원 절차' 처방은 오진)라 전용
        문안을 반환하고, 부등/판정 불능이면 None(호출부가 현행 복원 절차 문안 유지).
        .pristine 미러는 pack.rs:1324 커스터마이즈 절충 원장이 실존을 보장한다."""
        pristine_b = self._c03_read_bytes(
            os.path.join(pack_dir(), ".pristine", "directives", f))
        if live_b is None or pristine_b is None or live_b != pristine_b:
            return None
        line = ("원인 분류: 무수정 배포본(live==.pristine) — 핀 기준 선행"
                "(벤더 동기 필요·사용자 과실 아님)")
        if os.path.isfile(os.path.join(pack_dir(), "directives", f + ".new")):
            line += (" · .new 병치 — `cys pack-merge --file directives/%s --take-new`"
                     "(무수정본 한정)" % f)
        return line

    def _c03_promotion_state(self):
        """승격 판정에 필요한 파일 전부를 1회 읽어 상태 dict 로 환원한다(순수 관찰).

        승격 '표지' 판정 우선순위(스펙 결정 D3):
          ① md == 라이브 CEO_TEMPLATE 바이트 등가
          ② 승격 영수증 해시 == md sha256 — stale 영수증(부등)은 판정 근거로
             미사용·경고 없이 무시(R3 시나리오3)
          ③ 폴백: md 가 표지 3핀(MARKER_PINS) 전수 포함 ∧ .pre-ceo 존재"""
        d = os.path.join(pack_dir(), "directives")
        md_p = os.path.join(d, "MASTER_DIRECTIVE.md")
        st = {"tmpl_p": os.path.join(d, "CEO_TEMPLATE.md"),
              "pre_p": md_p + ".pre-ceo", "new_p": md_p + ".new",
              "receipt_p": os.path.join(d, ".ceo-template-applied"),
              "pristine_p": os.path.join(pack_dir(), ".pristine", "directives",
                                         "MASTER_DIRECTIVE.md")}
        md_b = self._c03_read_bytes(md_p)
        tmpl_b = self._c03_read_bytes(st["tmpl_p"])
        pre_b = self._c03_read_bytes(st["pre_p"])
        st.update(md_b=md_b, tmpl_b=tmpl_b, pre_b=pre_b,
                  new_b=self._c03_read_bytes(st["new_p"]),
                  pristine_b=self._c03_read_bytes(st["pristine_p"]))
        st["md_text"] = md_b.decode("utf-8", "replace") if md_b is not None else None
        st["pre_text"] = pre_b.decode("utf-8", "replace") if pre_b is not None else None
        st["md_sha"] = hashlib.sha256(md_b).hexdigest() if md_b is not None else None
        st["pre_sha"] = hashlib.sha256(pre_b).hexdigest() if pre_b is not None else None
        byte_equal = md_b is not None and tmpl_b is not None and md_b == tmpl_b
        receipt = self._c03_receipt_hash(st["receipt_p"])
        receipt_equal = (receipt is not None and st["md_sha"] is not None
                        and receipt == st["md_sha"])
        marker_pins_ok = (st["md_text"] is not None
                        and all(p in st["md_text"] for p in MARKER_PINS))
        st.update(byte_equal=byte_equal, receipt_equal=receipt_equal,
                  marker_pins_ok=marker_pins_ok, pre_exists=pre_b is not None)
        st["marker"] = byte_equal or receipt_equal or (marker_pins_ok and st["pre_exists"])
        st["marker_via"] = ("md==라이브 CEO_TEMPLATE 바이트 등가" if byte_equal
                           else "승격 영수증 해시 등가" if receipt_equal
                           else "표지 3핀+.pre-ceo 폴백" if st["marker"] else None)
        # 퇴화 = .pre-ceo 가 CEO 템플릿 사본으로 오염(동시 승격 레이스 잔재 · R1 E3) —
        # 강등하면 템플릿을 재적용하게 되어 강등 경로 자체가 파괴된 상태.
        st["degenerate"] = st["pre_exists"] and tmpl_b is not None and pre_b == tmpl_b
        st["classification"] = None   # 주축 분류 — _c03_master_axis 가 채운다(지문 성분).
        st["guard_bits"] = None       # demote-guard 3항 비트 — _c03_demote_guard 가 채운다.
        return st

    def _c03_master_axis(self, st, pins):
        """[핀 축·master] (status, detail) 반환 + st['classification'] 기록.

        판정 우선순위(R1 시나리오5 — md 직접/우회 이중 판정 충돌 소멸):
          ① md 표준 핀 전수 → PASS(승격 무관 — D2 합성본 자연 통과·사용자 주권 편집 허용.
             '개행 변경도 비정형 판정됨' 문구는 ③ 비정형 FAIL 쪽에 둔다 — 스펙 §D3(ii)①)
          ② 승격 표지 → 검사 표면을 .pre-ceo 로 승격 상태 세분(정상/구판/구본화/파괴/소실)
          ③ .pre-ceo 존재 ∧ 표지 전부 실패 → '비정형 승격 상태' FAIL(파괴적 조치 금지)
          ④ 그 외 → 현행 소실 FAIL(+원인 분류 1줄)"""
        if st["md_text"] is None:
            st["classification"] = "unreadable"
            return (FAIL, "MASTER_DIRECTIVE.md 읽기 불가 (C02 먼저 해결)")
        lost_md = [label for pin, label in pins if pin not in st["md_text"]]
        if not lost_md:
            st["classification"] = "pins-pass"
            return (PASS, "MASTER_DIRECTIVE.md 핀 %d개 전부 존재"
                          "(승격 여부 무관 — 사용자 주권 편집 허용)" % len(pins))
        promote_hint = ("신 템플릿 재적용: promote-ceo 재실행 — 오너 role-less 셸 또는 CSO "
                        "seat CLI `bash ~/.cys/pack/bin/cys-dept promote-ceo`"
                        "(CEO/master pane 의 exit 7 거부는 계약)")
        rebuild_hint = ".pristine/directives/MASTER_DIRECTIVE.md 로 .pre-ceo 재건(오너 승인)"
        if st["marker"]:
            if not st["pre_exists"]:
                # ④ 사분면(R1 시나리오6): 등가/영수증 승격인데 강등 백업이 없다 — demote 로
                #   자연 회복 불가한 고착 상태라 전용 FAIL. demote-guard 의 부재 WARN 과
                #   중복 발화는 의도(독립 축).
                st["classification"] = "promoted-backup-missing"
                return (FAIL, "복원 백업 소실(강등 불능) — 승격 상태(%s)인데 "
                              "MASTER_DIRECTIVE.md.pre-ceo 부재. 파괴적 조치 금지 — %s"
                        % (st["marker_via"], rebuild_hint))
            prefix = "승격 상태 — 표준 핀은 MASTER_DIRECTIVE.md.pre-ceo에서 검사"
            if st["degenerate"]:
                # ★연동 규칙(R3 conflicts2): 퇴화 검출 시 '구본화' 분류를 **억제**하고 전용
                #   분류로 승격한다 — 퇴화 머신에서도 .new 병치 시 구본화 술어(.pre-ceo 핀
                #   부족 ∧ 동일 핀이 .new 에 존재)가 참이 되어 '승격 백업 파괴'가 '구본화'로
                #   오표기되는 것을 차단.
                st["classification"] = "promoted-backup-destroyed"
                return (FAIL, "%s: 승격 백업 파괴(강등 불능) — cmp(.pre-ceo, CEO_TEMPLATE.md) "
                              "바이트 등가(퇴화 — 동시 승격 레이스 잔재). 파괴적 조치 금지 — %s"
                        % (prefix, rebuild_hint))
            lost_pre = [(pin, label) for pin, label in pins if pin not in st["pre_text"]]
            legacy_tail = "" if st["byte_equal"] else \
                " · 정상 승격(구판 템플릿 — md≠라이브 CEO_TEMPLATE). %s" % promote_hint
            if not lost_pre:
                if st["byte_equal"]:
                    st["classification"] = "promoted-current"
                    return (PASS, "%s: 핀 %d개 전부 존재(%s)"
                            % (prefix, len(pins), st["marker_via"]))
                # 구판 템플릿 승격 = 정상 상태의 WARN(FAIL 아님 — 템플릿 전진 릴리스 직후
                # 전 기승격 머신 위경보 차단 · R2 시나리오6) + 재적용 안내.
                st["classification"] = "promoted-legacy"
                return (WARN, "정상 승격(구판 템플릿) — md≠라이브 CEO_TEMPLATE(%s). "
                              "%s: 핀 %d개 전부 존재. %s"
                        % (st["marker_via"], prefix, len(pins), promote_hint))
            new_text = (st["new_b"].decode("utf-8", "replace")
                        if st["new_b"] is not None else None)
            if new_text is not None and all(pin in new_text for pin, _ in lost_pre):
                # 구본화(R1 E1): 승격 유지 중 벤더 전진으로 .pre-ceo 만 낡았다 — 신본 채택
                # 대기 분류로 결정론 안내(매번 수동 진단 반복 차단).
                st["classification"] = "promoted-stale-backup"
                return (FAIL, "%s: 구본화 — 신본 채택 대기: 소실 핀(%s) 전부가 "
                              "MASTER_DIRECTIVE.md.new 에 존재. D1(a)형 갱신(순서 역전 금지): "
                              "① `cp MASTER_DIRECTIVE.md.new MASTER_DIRECTIVE.md.pre-ceo` "
                              "② `cys pack-merge --file directives/MASTER_DIRECTIVE.md "
                              "--keep-mine`(keep-mine 이 .new 를 삭제)%s"
                        % (prefix, "; ".join(l for _, l in lost_pre), legacy_tail))
            st["classification"] = "promoted-backup-pins-lost"
            if st["pristine_b"] is not None and st["pre_b"] == st["pristine_b"]:
                # 검사 표면(.pre-ceo)이 무수정 배포본 — 사용자 과실 아님. 주의: 승격 중
                # MASTER 에 --take-new 를 처방하지 않는다(A12 가드 — 재사용 금지 경로).
                cause = ("원인 분류: 무수정 배포본(.pre-ceo==.pristine) — 핀 기준 선행"
                         "(벤더 동기 필요·사용자 과실 아님)")
            else:
                cause = _C03_RESTORE_GUIDE
            return (FAIL, "%s: 소실된 조항: %s — %s%s"
                    % (prefix, "; ".join(l for _, l in lost_pre), cause, legacy_tail))
        if st["pre_exists"]:
            # ③ 비정형 승격 상태 — 파괴적 조치 금지·cmp 진단 첨부(R1 시나리오4).
            st["classification"] = "anomalous-promotion"
            return (FAIL, "비정형 승격 상태 — .pre-ceo 존재 ∧ md 표준 핀 실패 ∧ 승격 표지"
                          "(바이트 등가·영수증·표지 3핀) 전부 실패. ★파괴적 조치 금지(주인님께 "
                          "보고 후 지시 대기). 사용자 주권 편집은 허용이나 개행 변경도 비정형 "
                          "판정됨(바이트 등가 기준·정규화 없음). cmp: md vs .pre-ceo=%s · "
                          "md vs .pristine(MASTER)=%s · md vs 라이브 CEO_TEMPLATE=%s. "
                          "CEO_TEMPLATE 전진 직후라면 promote-ceo 재실행으로 해소(재스왑·"
                          ".pre-ceo 보존). 소실된 조항: %s"
                    % (self._c03_cmp_label(st["md_b"], st["pre_b"]),
                       self._c03_cmp_label(st["md_b"], st["pristine_b"]),
                       self._c03_cmp_label(st["md_b"], st["tmpl_b"]),
                       "; ".join(lost_md)))
        # ④ 비승격 일반 소실 — 원인 분류 1줄(무수정 배포본이면 복원 절차 처방을 대체).
        st["classification"] = "pins-fail"
        cause = self._c03_cause_line("MASTER_DIRECTIVE.md", st["md_b"])
        return (FAIL, "MASTER_DIRECTIVE.md에서 소실된 조항: %s — %s"
                % ("; ".join(lost_md), cause or _C03_RESTORE_GUIDE))

    def _c03_demote_guard(self, st):
        """[건전성 축] C03.demote-guard — 승격 표지 시 항상 (status, detail)|None 반환 +
        st['guard_bits'] 기록. 핀 축 결과와 독립이다(R2 E2).

        ★WARN 등급 유지 근거 = 부트 비치명 계약(R1 E3 의 '전용 FAIL' 요구를 R2 가 하향한
          근거의 명문화): preflight FAIL 은 부트를 차단(exit 1)하는데, 강등 백업 건전성은
          '지금 부트를 막을 사유'가 아니라 '강등 시점에 터질 잠복 위험'의 상시 가시화다 —
          FAIL 로 두면 승격 함대 전체의 부트가 관측 목적에 볼모로 잡힌다. 같은 상태를 핀
          축이 FAIL 로 병행 발화할 수 있고(④ 백업 소실 등) 그 중복은 의도다(독립 축 —
          모순 아닌 중복 신호 · R3 minor).
        guard_bits = [pre 존재, 퇴화, 핀 전수] (1/0 · 부재 단락 시 후속 = None) — 지문 성분."""
        if not st["marker"]:
            return None    # 비승격 — 건전성 축 해당 없음(행 미발화 · 소음 0)
        if not st["pre_exists"]:
            st["guard_bits"] = [0, None, None]
            return (WARN, "승격 표지(%s)인데 .pre-ceo 부재 — 강등 불능. 후속 검사(퇴화·핀) "
                          "단락. .pristine/directives/MASTER_DIRECTIVE.md 로 재건(오너 승인). "
                          "핀 축 '복원 백업 소실' FAIL 과의 중복 발화는 의도(독립 축)"
                    % st["marker_via"])
        pins_lost = [(pin, label) for pin, label in CONTENT_PINS["MASTER_DIRECTIVE.md"]
                     if pin not in st["pre_text"]]
        st["guard_bits"] = [1, 1 if st["degenerate"] else 0, 0 if pins_lost else 1]
        if st["degenerate"]:
            return (WARN, "승격 백업 파괴 — cmp(.pre-ceo, CEO_TEMPLATE.md) 바이트 등가(퇴화)"
                          "·강등 불능 — .pristine/directives/MASTER_DIRECTIVE.md 로 재건"
                          "(오너 승인)")
        if pins_lost:
            new_text = (st["new_b"].decode("utf-8", "replace")
                        if st["new_b"] is not None else None)
            if new_text is not None and all(pin in new_text for pin, _ in pins_lost):
                return (WARN, "구본화 — 신본 채택 대기: .pre-ceo 소실 핀(%s) 전부가 "
                              "MASTER_DIRECTIVE.md.new 에 존재. D1(a)형 갱신: "
                              "① `cp MASTER_DIRECTIVE.md.new MASTER_DIRECTIVE.md.pre-ceo` "
                              "② `cys pack-merge --file directives/MASTER_DIRECTIVE.md "
                              "--keep-mine`" % "; ".join(l for _, l in pins_lost))
            return (WARN, ".pre-ceo 표준 핀 부족: %s — 강등 시 조항 소실 위험"
                    % "; ".join(l for _, l in pins_lost))
        return (PASS, "승격 백업 건전 — .pre-ceo 존재·비퇴화(≠CEO_TEMPLATE)·표준 핀 전수")

    def _c03_fingerprint_dedupe(self, fp):
        """상태 지문 dedupe(스펙 A6 · ①폭주 앵커) — 반환 True = 동일 지문(detail 1줄 축약).

        지문 = [md sha256, 주축 분류, .pre-ceo sha256|'absent', demote-guard 3항 비트].
        demote-guard 축도 지문에 편입돼 .pre-ceo 소실/퇴화 전이는 **항상 새 지문**이 되어
        축약을 뚫는다(즉시 전문 재발화 · R3 minor2). 오너 통보(heartbeat/fleet digest 인용)는
        지문 변화 시에만 — 소비처가 이 파일의 changed_at 을 참조한다(배선은 티켓 범위 밖).
        ★report 모드에서도 기록한다: 이 파일은 설정이 아니라 관측 dedupe 원장(기계 소유·
          가역·~/.cys/state)이라 OPP-17 비가역 게이트 범위 밖이고, 스펙 A6 이 '동일 지문
          축약'을 매 부트 결정론으로 요구한다. 쓰기 실패는 무시(부트 게이트 crash 금지)."""
        path = c03_fingerprint_path()
        try:
            with open(path, encoding="utf-8") as f:
                old = json.load(f)
        except (OSError, ValueError):
            old = None
        same = isinstance(old, dict) and old.get("fingerprint") == fp
        if not same:
            try:
                os.makedirs(os.path.dirname(path), exist_ok=True)
                fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=".c03-fp-")
                with os.fdopen(fd, "w", encoding="utf-8") as f:
                    json.dump({"fingerprint": fp, "changed_at": int(time.time())},
                              f, ensure_ascii=False)
                os.replace(tmp, path)   # 원자 교체(반쪽 JSON 차단 — G16 관용)
            except OSError:
                pass
        return same

    def c03_content_pins(self):
        # ★dept 면제 early-return 선행 불변(멀티마스터 F1) — 어떤 신설 로직도 이 가드보다
        #   앞서지 않는다(부서/CEO 커스텀 디렉티브에 표준 핀·승격 판정을 들이대지 않는다).
        if is_dept_pack():
            self.add("C03.pin", WARN,
                     "부서/CEO pack(%s) — 표준 디렉티브 핀 검사 면제(CEO/부서장 커스텀 디렉티브가 정상)"
                     % pack_dir())
            return
        # [핀 축·master + 건전성 축 + 상태 지문] — base 팩 한정 승격 로직.
        mcid, gcid = "C03.pin.master", "C03.demote-guard"
        st = self._c03_promotion_state()
        mrow = None
        if not self.skipped(mcid):
            mrow = self._c03_master_axis(st, CONTENT_PINS["MASTER_DIRECTIVE.md"])
        grow = None
        if not self.skipped(gcid):
            grow = self._c03_demote_guard(st)
        if mrow is not None:
            fp = [st["md_sha"] or "unreadable", st["classification"],
                  st["pre_sha"] or "absent", st["guard_bits"]]
            same = self._c03_fingerprint_dedupe(fp)
            self.add(mcid, mrow[0],
                     ("[지문 동일 — 축약] %s (md=%s) — 전문은 지문 변화 시 재발화"
                      % (st["classification"], (st["md_sha"] or "?")[:12]))
                     if same else mrow[1])
            if grow is not None:
                self.add(gcid, grow[0],
                         ("[지문 동일 — 축약] demote-guard bits=%s" % (st["guard_bits"],))
                         if same else grow[1])
        elif grow is not None:
            # master 축이 --skip 됐어도 건전성 축은 독립 발화(지문 dedupe 없이 전문).
            self.add(gcid, grow[0], grow[1])
        # [배달 축] C03.boot-contract — 아래 참조. 핀/건전성 축과 독립(둘의 skip 여부 무관).
        self._c03_boot_contract(st)
        # [핀 축 — 그 외 역할 + CEO 템플릿] (master 는 위 승격 축 판정이 대체)
        for f, pins in CONTENT_PINS.items():
            if f == "MASTER_DIRECTIVE.md":
                continue
            cid = "C03.pin.%s" % f.split("_")[0].lower()
            if self.skipped(cid):
                continue
            live_b = self._c03_read_bytes(os.path.join(pack_dir(), "directives", f))
            if live_b is None:
                self.add(cid, FAIL, "%s 읽기 불가 (C02 먼저 해결)" % f)
                continue
            text = live_b.decode("utf-8", "replace")
            lost = [label for pin, label in pins if pin not in text]
            if lost:
                cause = self._c03_cause_line(f, live_b)
                self.add(cid, FAIL, "%s에서 소실된 조항: %s — %s"
                         % (f, "; ".join(lost), cause or _C03_RESTORE_GUIDE))
            else:
                self.add(cid, PASS, "%s 핀 %d개 전부 존재" % (f, len(pins)))

    def _c03_boot_contract(self, st):
        """[배달 축] C03.boot-contract — P0-3 기계 래치(`result.retry_eligible`)의 **소비 권한
        행**(§0-A session_error 행)이 *살아있는* MASTER_DIRECTIVE 에 실재하는가.

        ★왜 별도 축인가(R3-DELIVERY-1 · 2026-08-26 적대검증): `directives/*_DIRECTIVE.md` 는
          pack.rs `ownership()` 상 **Ownership::User** 라, 팩 업데이트는 디스크≠임베드이면
          매니페스트 해시가 일치해도·`--force` 여도 본문을 덮지 않는다(`decide_file_action` 의
          User 분기 = `Keep{new_pending}`) — 신본은 `<rel>.new` 병치 + `cys pack-merge` 로만
          도달한다. 반대로 그 행을 소비하도록 안내하는 훅(role-bootstrap.sh·session-start.sh)은
          **System 등급이라 강제 치유로 전원에게 도달**한다. 결손이 관측되지 않으면 배포되는
          것은 침묵이 아니라 '재실행 금지 vs 1회 재실행'의 이중 진실이다.
        ★WARN 고정(FAIL 금지): 이 행의 부재는 '지금 파손'이 아니라 '병합 대기'다 — 같은 규칙을
          훅 브리지가 **자기완결**로 싣고 전원에게 도달하므로(두 훅의 R3-DELIVERY-1 문단) 기계
          거동은 성립한다. CONTENT_PINS 에 편입해 FAIL 로 만들면 병합 전 전 함대가 상시
          NOT READY 가 되어 관측 목적이 부트 준비 판정을 볼모로 잡는다(demote-guard 의 WARN
          유지 근거와 동형). 대신 해소 명령을 detail 에 결정론으로 싣는다.
        """
        cid = "C03.boot-contract"
        if self.skipped(cid):
            return
        text = st.get("md_text")
        if text is None:
            self.add(cid, WARN, "MASTER_DIRECTIVE.md 판독 불가 — 배달 판정 불가"
                                "(C02·C03.pin 먼저 해결)")
            return
        # 소비면의 최소 술어 2개: 래치 필드명 + 그 행이 다루는 상태명.
        lost = [k for k in ("retry_eligible", "session_error") if k not in text]
        if not lost:
            self.add(cid, PASS,
                     "§0-A session_error 행(P0-3 래치 소비면) 실재 — 훅 브리지와 동일 규칙")
            return
        new_p = st.get("new_p")
        if new_p and os.path.isfile(new_p):
            fix = ("해소: `cys pack-merge --file directives/MASTER_DIRECTIVE.md`"
                   "(무수정본이면 `--take-new`) — vendor 신본이 이미 %s 로 병치돼 있다"
                   % os.path.basename(new_p))
        elif st.get("marker"):
            fix = ("해소(승격 기계): CEO_TEMPLATE 재합성/팩 갱신 후 `cys-dept promote-ceo` 재실행 "
                   "— 승격 md 는 CEO 템플릿 사본이라 표준 전문이 그 경유로만 갱신된다")
        else:
            fix = "해소: `cys init-pack` 으로 vendor 신본(.new) 병치 재생성 후 `cys pack-merge`"
        self.add(cid, WARN,
                 "살아있는 MASTER_DIRECTIVE 에 §0-A session_error 행이 없다(부재 술어: %s) — "
                 "user-owned 헌법 파일이라 팩 갱신이 도달하지 않은 상태다. 기계 거동은 훅 브리지"
                 "(자기완결 문단)가 유지하므로 부트는 정상이나, 디렉티브와 주입문이 서로 다른 "
                 "지시를 하는 이중 진실이 남는다. %s" % (", ".join(lost), fix))

    # ── C04 soul.md 호칭 규정 ──
    def c04_soul(self):
        cid = "C04.soul"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "soul.md")
        if not os.path.isfile(p):
            if self.fix and self.repair_via_init_pack() and os.path.isfile(p):
                pass  # 재설치됨 — 아래 호칭 검사로 계속
            else:
                self.add(cid, FAIL, "soul.md 없음")
                return
        text = open(p, encoding="utf-8", errors="replace").read()
        # 2개 정책 마커: ①오너 호칭 ②자율주행 위임권(앵커6 — soul이 권한을 부여해야
        # MASTER §14 발효). 둘 다 --fix로 기본 골격을 보강할 수 있다(내용은 오너 주권).
        fixed = []
        if SOUL_MARKER not in text:
            if not self.fix:
                self.add(cid, FAIL,
                         "soul.md에 '오너 호칭' 규정 부재 — --fix로 기본값(주인님) 보강 가능")
                return
            if SOUL_PLACEHOLDER in text:
                text = text.replace(
                    SOUL_PLACEHOLDER,
                    '(이름을 적어라)\n- **오너 호칭: master는 오너를 "주인님"으로 호칭한다** (수정 가능)',
                    1,
                )
            else:
                text += SOUL_APPEND
            fixed.append("호칭 규정(기본 주인님)")
        # 자율주행 절은 '권한 부여 조항'이라 --fix가 자동 재주입하지 않는다(적대 검증 6차
        # H-2: 오너가 권한 회수 의사로 절을 삭제하면 다음 부트의 의무 --fix가 권한을 자동
        # 복원해 "절 부재=자율주행 안 함" 상태가 도달 불가능해진다 — 부여 주체는 오너뿐).
        # 절 부재는 유효한 '자율주행 비활성' 상태 — WARN으로 알리고 부트는 막지 않는다.
        autopilot_note = ""
        if SOUL_AUTOPILOT_MARKER not in text:
            autopilot_note = (" · 자율주행 위임권 절 부재 — 자율주행 비활성(MASTER §14 미발효). "
                              "부여하려면 오너가 soul.md에 절을 직접 추가하라"
                              "(표준 문안: 이 스크립트의 SOUL_AUTOPILOT_TEMPLATE)")
        if fixed:
            open(p, "w", encoding="utf-8").write(text)
            self.add(cid, FIXED, "soul.md 보강: %s%s" % (", ".join(fixed), autopilot_note))
        elif autopilot_note:
            self.add(cid, WARN, "호칭 규정 존재%s" % autopilot_note)
        else:
            self.add(cid, PASS, "호칭 규정 + 자율주행 위임권 절 존재 (내용은 오너 주권)")

    # ── 공용 수리: 파손 JSON을 백업 후 템플릿으로 복원 ──
    # 파싱이 죽은 파일은 '유효한 사용자 수정'이 아니다 — .broken 백업을 남기고
    # init-pack 템플릿으로 되살리는 것이 안전한 결정론 수리다(내용 손실 없음: 백업 보존).
    def restore_broken_json(self, path):
        if not self.fix:
            return False
        if os.path.islink(path):
            return False  # symlink 거부 — 링크 너머 실파일 훼손 차단(TOCTOU 방어)
        try:
            if os.path.isfile(path):
                shutil.move(path, path + ".broken-preflight")
        except OSError:
            return False
        self._init_pack_ran = None  # 파일을 치웠으니 재시도 허용
        return self.repair_via_init_pack() and os.path.isfile(path)

    # ── C05 agents.json 역할 매핑 ──
    def c05_agents(self):
        cid = "C05.agents-json"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "agents.json")
        if not os.path.isfile(p) and self.fix and self.repair_via_init_pack():
            pass
        fixed_broken = False
        try:
            data = json.load(open(p, encoding="utf-8"))
        except (OSError, ValueError) as e:
            if self.restore_broken_json(p):
                try:
                    data = json.load(open(p, encoding="utf-8"))
                    fixed_broken = True
                except (OSError, ValueError) as e2:
                    self.add(cid, FAIL, "agents.json 복원 후에도 파싱 실패: %s" % e2)
                    return
            else:
                self.add(cid, FAIL, "agents.json 파싱 실패: %s — --fix로 백업·복원 가능" % e)
                return
        problems = []
        for a in ("claude", "gemini", "codex"):
            if not isinstance(data.get(a), dict) or "cmd" not in data[a]:
                problems.append("어댑터 %s 누락/불완전" % a)
        roles = data.get("_roles", {})
        for r in ROLES:
            f = roles.get(r)
            if not f:
                problems.append("_roles.%s 매핑 누락" % r)
            elif not os.path.isfile(os.path.join(pack_dir(), f)):
                problems.append("_roles.%s → %s 파일 없음" % (r, f))
        if problems:
            self.add(cid, FAIL, "; ".join(problems))
        elif fixed_broken:
            self.add(cid, FIXED, "파손 agents.json 백업(.broken-preflight) 후 템플릿 복원")
        else:
            self.add(cid, PASS, "어댑터 3종 + 역할 매핑 4종 정합")

    # ── C06 acl.json / schedule.json 파싱 ──
    def c06_json_files(self):
        cid = "C06.json-parse"
        if self.skipped(cid):
            return
        problems = []
        fixed = []
        for f in ("acl.json", "schedule.json"):
            p = os.path.join(pack_dir(), f)
            if not os.path.isfile(p):
                if self.fix and self.repair_via_init_pack() and os.path.isfile(p):
                    pass
                else:
                    problems.append("%s 없음" % f)
                    continue
            try:
                json.load(open(p, encoding="utf-8"))
            except (OSError, ValueError) as e:
                if self.restore_broken_json(p):
                    try:
                        json.load(open(p, encoding="utf-8"))
                        fixed.append(f)
                        continue
                    except (OSError, ValueError):
                        pass
                problems.append("%s 파싱 실패: %s" % (f, e))
        if problems:
            self.add(cid, FAIL, "; ".join(problems))
        elif fixed:
            self.add(cid, FIXED, "파손 복원: %s (.broken-preflight 백업)" % ", ".join(fixed))
        else:
            self.add(cid, PASS, "acl.json·schedule.json 정상")

    # ── C07 hook 스크립트 존재·실행권한 ──
    def c07_hook_script(self):
        cid = "C07.hook-script"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "hooks", "session-start.sh")
        if not os.path.isfile(p):
            if self.fix and self.repair_via_init_pack() and os.path.isfile(p):
                pass
            else:
                self.add(cid, FAIL, "hooks/session-start.sh 없음")
                return
        if os.name == "posix":
            mode = os.stat(p).st_mode
            if not mode & stat.S_IXUSR:
                if self.fix:
                    os.chmod(p, mode | 0o755)
                    self.add(cid, FIXED, "실행권한 부여(755)")
                    return
                self.add(cid, WARN, "실행권한 없음 (sh 명시 호출이라 동작은 하나 권장 755)")
                return
        self.add(cid, PASS, p)

    # ── C08 SessionStart hook 등록 (Claude 설정) ──
    def _hook_registered(self, settings_path):
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        # 정확히 desired 형태(정슬래시+따옴표)만 '등록됨'으로 인정 — 구·파손(역슬래시·미따옴표)
        # 엔트리는 미등록으로 보아 _register_hook 이 교체하게 한다(멱등: 재실행 시 True).
        desired = _cys_hook_cmd("session-start.sh")
        for entry in data.get("hooks", {}).get("SessionStart", []):
            for h in entry.get("hooks", []):
                if h.get("command", "") == desired:
                    return True
        return False

    def _register_hook(self, settings_path):
        """hook 등록. 성공=None, 실패=사유 문자열 (호출자가 FAIL로 보고).

        안전장치는 `_settings_rmw`(G16 단일 소유자) 계약에 위임한다 — symlink 거부·파싱 실패
        거부(빈 dict 대체 금지)·최초 1회 백업·**파일별 락 + mkstemp 원자 교체**.
        """
        cmd = _cys_hook_cmd("session-start.sh")

        def _mutate(data):
            arr = data.setdefault("hooks", {}).setdefault("SessionStart", [])
            # reconcile: 구·파손 엔트리 제거 후 desired 하나만 보장(중복·잔존 파손 차단).
            kept, have = _prune_stale_hook_entries(arr, "session-start.sh", cmd)
            if not have:
                kept.append({"hooks": [{"type": "command", "command": cmd}]})
            arr[:] = kept

        return _settings_rmw(settings_path, _mutate)

    def c08_hook_registered(self):
        cid = "C08.hook-registered"
        if self.skipped(cid):
            return
        # ★G1: 등록 대상은 sentinel 계약으로 받는다 — '금지'(격리 팩)와 '미발견'(신규 머신)은
        #   처방이 정반대다(등록 0 vs 기본 프로필 생성). 폴백은 resolve 가 소유한다.
        targets, forbidden = resolve_registration_targets()
        if forbidden and not targets:
            self.add(cid, SKIP, "등록 대상 없음 — %s" % forbidden)
            return
        unregistered = [t for t in targets if not self._hook_registered(t)]
        if not unregistered:
            self.add(cid, PASS, "%d개 프로필 전부 hook 등록됨" % len(targets))
            return
        if self.fix:
            done, errs = [], []
            for t in unregistered:
                err = self._register_hook(t)
                if err:
                    errs.append(err)
                else:
                    done.append(t)
            if errs:
                self.add(cid, FAIL, "; ".join(errs)
                         + (" | 등록 성공: %s" % ", ".join(done) if done else ""))
            else:
                self.add(cid, FIXED, "hook 등록: %s" % ", ".join(done))
        else:
            self.add(cid, FAIL, "hook 미등록 프로필: %s" % ", ".join(unregistered))

    # ── C32 statusline 래퍼 등록 (Claude 설정 — T5 Phase 2-A claude rate limit 채널) ──
    # claude의 5h/주간 rate limit 잔량은 로컬 파일에 없다 — 유일한 무간섭 채널이 statusline
    # stdin JSON이다. cys-statusline.sh가 매 메시지마다 usage.report로 push해 pane 배지에
    # 5h/7d를 띄운다. 부가 기능이라(ctx 배지는 없어도 작동) 미설치는 WARN(READY 미차단).
    def _statusline_registered(self, settings_path):
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        sl = data.get("statusLine")
        if not (isinstance(sl, dict) and "cys-statusline.sh" in sl.get("command", "")):
            return False
        # Windows 구 파손 형태(역슬래시 경로)는 미등록으로 보아 재등록(statusLine은 단일 overwrite라 중복 없음).
        return not (os.name == "nt" and "\\" in sl.get("command", ""))

    def _register_statusline(self, settings_path):
        """statusLine 등록. 성공=None, 실패=사유 문자열. 기존 statusLine은 CYS_PREV_STATUSLINE로
        래핑해 체인 보존(덮어쓰기 금지) — _register_hook과 동일한 symlink 거부·파싱 거부·최초
        백업·원자적 쓰기 철학."""
        base = _cys_hook_cmd("cys-statusline.sh")   # 정슬래시+따옴표(Windows) / sh <abs>(unix)

        def _mutate(data):
            # 기존 statusLine(우리 것이 아니면) → CYS_PREV_STATUSLINE로 보존 체인(사람용 줄 위임).
            prev = data.get("statusLine")
            prev_cmd = prev.get("command", "") if isinstance(prev, dict) else ""
            if prev_cmd and "cys-statusline.sh" not in prev_cmd:
                cmd = "CYS_PREV_STATUSLINE=%s %s" % (shlex.quote(prev_cmd), base)
            else:
                cmd = base
            data["statusLine"] = {"type": "command", "command": cmd}

        return _settings_rmw(settings_path, _mutate)   # G16: 락+mkstemp 단일 소유자

    def c32_statusline(self):
        cid = "C32.statusline"
        if self.skipped(cid):
            return
        script = os.path.join(pack_dir(), "hooks", "cys-statusline.sh")
        if not os.path.isfile(script):
            if not (self.fix and self.repair_via_init_pack() and os.path.isfile(script)):
                self.add(cid, FAIL, "hooks/cys-statusline.sh 없음 — `cys init-pack` 또는 --fix")
                return
        targets, forbidden = resolve_registration_targets()   # ★G1 sentinel
        if forbidden and not targets:
            self.add(cid, SKIP, "등록 대상 없음 — %s" % forbidden)
            return
        unregistered = [t for t in targets if not self._statusline_registered(t)]
        if not unregistered:
            self.add(cid, PASS,
                     "%d개 프로필 statusLine 등록됨 (claude 재시작 후 5h/7d rate limit 배지 적용)"
                     % len(targets))
            return
        if self.fix:
            done, errs = [], []
            for t in unregistered:
                err = self._register_statusline(t)
                errs.append(err) if err else done.append(t)
            if errs:
                self.add(cid, FAIL, "; ".join(errs)
                         + (" | 등록 성공: %s" % ", ".join(done) if done else ""))
            else:
                self.add(cid, FIXED, "statusLine 등록: %s — ★claude 재시작 후 적용" % ", ".join(done))
        else:
            self.add(cid, WARN, "statusLine 미등록 프로필: %s — --fix로 설치(claude 재시작 후 적용)"
                     % ", ".join(unregistered))

    # ── C33 툴 이벤트 hook (T7 E1-④ — events 테이블 적재) ──
    # PreToolUse/PostToolUse에 hooks/cys-hook.sh 등록 → 툴·스킬·에이전트 호출·exit_code를
    # cysd events 테이블에 적재(E3 스킬 TOP·반복실패 토대). hook은 fail-open(에이전트 무차단)이라
    # 무관 작업 불간섭. C32와 동일 규약(체인보존·symlink/파손 거부·원자적). FAIL 없음(미등록=WARN).
    EVENT_HOOK = "cys-hook.sh"
    EVENT_HOOK_EVENTS = ("PreToolUse", "PostToolUse", "PermissionRequest")

    def c33_event_hooks(self):
        cid = "C33.event-hooks"
        if self.skipped(cid):
            return
        script = os.path.join(pack_dir(), "hooks", self.EVENT_HOOK)
        if not os.path.isfile(script):
            if not (self.fix and self.repair_via_init_pack() and os.path.isfile(script)):
                self.add(cid, WARN, "hooks/%s 없음 — `cys init-pack` 또는 --fix" % self.EVENT_HOOK)
                return
        targets, forbidden = resolve_registration_targets()   # ★G1 sentinel
        if forbidden and not targets:
            self.add(cid, SKIP, "등록 대상 없음 — %s" % forbidden)
            return
        # 프로필×이벤트 단위로 미등록 항목 수집
        pending = [(t, ev) for t in targets for ev in self.EVENT_HOOK_EVENTS
                   if not self._event_hook_registered(t, ev, self.EVENT_HOOK)]
        if not pending:
            self.add(cid, PASS,
                     "%d개 프로필 PreToolUse/PostToolUse 이벤트 hook 등록됨 (claude 재시작 후 적용)"
                     % len(targets))
            return
        if self.fix:
            done, errs = [], []
            for t, ev in pending:
                err = self._register_event_hook(t, ev, self.EVENT_HOOK, matcher="")
                errs.append("%s/%s: %s" % (os.path.basename(os.path.dirname(t)), ev, err)) \
                    if err else done.append("%s/%s" % (os.path.basename(os.path.dirname(t)), ev))
            if errs:
                self.add(cid, WARN, "; ".join(errs)
                         + (" | 등록 성공: %s" % ", ".join(done) if done else ""))
            else:
                self.add(cid, FIXED, "이벤트 hook 등록: %s — ★claude 재시작 후 적용" % ", ".join(done))
        else:
            self.add(cid, WARN, "이벤트 hook 미등록: %d건 — --fix로 설치(claude 재시작 후 적용)"
                     % len(pending))

    # ── C56 dept-hook 누수 invariant (결정론 재발방지 — 예방 가드 _pack_is_dept의 짝) ──
    # invariant: 비-부서 글로벌 settings(discover_claude_settings 반환)에 pack-dept-* 훅은 0이어야.
    # 부서 config는 discover의 _acct/_pack 가드가 애초에 제외 → 자기 dept 훅은 안전(검사 대상 아님).
    # report=FAIL(누수 N 탐지) · fix=청소(base 보존·빈 블록 제거·백업·원자적 쓰기). 수동 #2의 코드화.
    def _dept_hooks_in(self, settings_path):
        """결정론: settings.json hooks command 경로에 '/pack-dept-' 포함한 (event,bi,hi) 목록.
        마커는 예방 가드(discover/_pack_is_dept)의 'pack-dept-'와 동일 — 명명부서(pack-dept-<custom>)도
        탐지(가드의 짝). 경로경계 '/' 앵커로 비앵커 substring 오탐 제거. base(/pack/)·CEO(/pack-ceo/) 미매치."""
        try:
            with open(settings_path) as f:
                data = json.load(f)
        except Exception:
            return []
        hroot = data.get("hooks")
        if not isinstance(hroot, dict):  # 청소기와 대칭(무raise 계약·malformed 입력 부트크래시 방지)
            return []
        out = []
        for ev, blocks in hroot.items():
            if not isinstance(blocks, list):
                continue
            for bi, blk in enumerate(blocks):
                hooks = blk.get("hooks", []) if isinstance(blk, dict) else []
                for hi, h in enumerate(hooks):
                    if "/pack-dept-" in (h.get("command", "") if isinstance(h, dict) else ""):
                        out.append((ev, bi, hi))
        return out

    def _strip_dept_hooks(self, settings_path):
        """백업 후 dept 훅 제거(base 보존·빈 블록 제거·원자적). 반환 (removed, err)."""
        try:
            with open(settings_path) as f:
                data = json.load(f)
        except Exception as e:
            return (0, str(e))
        H = data.get("hooks")
        if not isinstance(H, dict):
            return (0, "hooks 루트가 객체 아님")
        removed = 0
        for ev in list(H.keys()):
            blocks = H[ev] if isinstance(H[ev], list) else []
            nb = []
            for blk in blocks:
                hooks = blk.get("hooks", []) if isinstance(blk, dict) else []
                kept = [h for h in hooks
                        if "/pack-dept-" not in (h.get("command", "") if isinstance(h, dict) else "")]
                removed += len(hooks) - len(kept)
                if kept:
                    b2 = dict(blk); b2["hooks"] = kept; nb.append(b2)
            H[ev] = nb
        if removed:
            try:
                bak = settings_path + ".bak-deptleak"
                if not os.path.exists(bak):
                    shutil.copy(settings_path, bak)
                # 프로세스 고유 tmp(동시 --fix 레이스 방지) + 권한 보존 후 원자적 replace
                fd, tmp = tempfile.mkstemp(dir=os.path.dirname(settings_path) or ".",
                                           prefix=".deptleak.", suffix=".tmp")
                try:
                    with os.fdopen(fd, "w") as f:
                        json.dump(data, f, indent=2, ensure_ascii=False)
                    shutil.copystat(settings_path, tmp)
                    os.replace(tmp, settings_path)
                except Exception:
                    if os.path.exists(tmp):
                        os.unlink(tmp)
                    raise
            except Exception as e:
                return (0, str(e))
        return (removed, "")

    def c56_dept_hook_leak(self):
        cid = "C56.dept-hook-leak"
        if self.skipped(cid):
            return
        # 부서 컨텍스트(account 또는 pack이 dept)면 글로벌 누수 탐지 대상 아님 — master/CEO preflight 소관.
        _acct = os.environ.get("CYS_ACCOUNT_DIR")
        if _acct and "dept-" in os.path.basename(os.path.normpath(_acct)):
            self.add(cid, SKIP, "부서 컨텍스트(account=dept) — 누수 탐지는 master/CEO preflight 소관")
            return
        if "pack-dept-" in os.path.basename(os.path.normpath(pack_dir())):
            self.add(cid, SKIP, "부서 pack 컨텍스트 — 글로벌 검사 대상 아님")
            return
        targets = discover_claude_settings()
        if not targets:
            self.add(cid, WARN, "~/.claude*/settings.json 미발견")
            return
        leaks = {t: self._dept_hooks_in(t) for t in targets}
        leaks = {t: v for t, v in leaks.items() if v}
        if not leaks:
            self.add(cid, PASS, "%d개 글로벌 settings에 dept 훅 누수 0 (invariant 충족)" % len(targets))
            return
        total = sum(len(v) for v in leaks.values())
        summary = ", ".join("%s:%d" % (os.path.basename(os.path.dirname(t)), len(v))
                            for t, v in leaks.items())
        if self.fix:
            done, errs = [], []
            for t in leaks:
                n, err = self._strip_dept_hooks(t)
                if err:
                    errs.append("%s: %s" % (os.path.basename(os.path.dirname(t)), err))
                else:
                    done.append("%s(-%d)" % (os.path.basename(os.path.dirname(t)), n))
            if errs:
                self.add(cid, WARN, "일부 청소 실패: %s | 성공: %s"
                         % ("; ".join(errs), ", ".join(done)))
            else:
                self.add(cid, FIXED, "dept 훅 누수 %d개 제거(base 보존·백업): %s — ★claude 재시작 후 적용"
                         % (total, ", ".join(done)))
        else:
            self.add(cid, FAIL, "글로벌 settings dept 훅 누수 %d개 탐지: %s (--fix로 청소)"
                     % (total, summary))

    # ── C57 temp-pack hook 누수 invariant (C56 dept 누수의 짝 — 2026-07-02 근본복원) ──
    # invariant: 어떤 settings(hooks)에도 command 경로가 임시 디렉터리(/tmp·$TMPDIR·/var/folders)인
    # hook은 0이어야 한다. grill embed/스냅샷 하네스가 temp pack으로 preflight를 돌려 실 config에 등록한
    # /tmp 세션훅이 temp dir이 비거나 사라지며 SessionStart "No such file" 무한재발한 계열의 코드화 청소.
    # 예방(discover_claude_settings temp 가드)의 짝 — 이미 누수된 잔해를 제거한다. report=FAIL·fix=청소.
    def _temp_hooks_in(self, settings_path):
        """결정론: settings.json hooks command 의 sh|bash 스크립트 경로가 temp dir 아래인 (event,bi,hi)."""
        try:
            with open(settings_path) as f:
                data = json.load(f)
        except Exception:
            return []
        hroot = data.get("hooks")
        if not isinstance(hroot, dict):
            return []
        out = []
        for ev, blocks in hroot.items():
            if not isinstance(blocks, list):
                continue
            for bi, blk in enumerate(blocks):
                hooks = blk.get("hooks", []) if isinstance(blk, dict) else []
                for hi, h in enumerate(hooks):
                    cmd = h.get("command", "") if isinstance(h, dict) else ""
                    m = re.search(r"(?:sh|bash)\s+(\S+)", cmd)
                    if m and _path_under_tempdir(os.path.expanduser(m.group(1))):
                        out.append((ev, bi, hi))
        return out

    def _strip_temp_hooks(self, settings_path):
        """백업 후 temp 훅 제거(비-temp 보존·빈 블록 제거·원자적). 반환 (removed, err)."""
        try:
            with open(settings_path) as f:
                data = json.load(f)
        except Exception as e:
            return (0, str(e))
        H = data.get("hooks")
        if not isinstance(H, dict):
            return (0, "hooks 루트가 객체 아님")

        def _is_temp(h):
            cmd = h.get("command", "") if isinstance(h, dict) else ""
            m = re.search(r"(?:sh|bash)\s+(\S+)", cmd)
            return bool(m and _path_under_tempdir(os.path.expanduser(m.group(1))))

        removed = 0
        for ev in list(H.keys()):
            blocks = H[ev] if isinstance(H[ev], list) else []
            nb = []
            for blk in blocks:
                hooks = blk.get("hooks", []) if isinstance(blk, dict) else []
                kept = [h for h in hooks if not _is_temp(h)]
                removed += len(hooks) - len(kept)
                if kept:
                    b2 = dict(blk)
                    b2["hooks"] = kept
                    nb.append(b2)
            H[ev] = nb
        if removed:
            try:
                bak = settings_path + ".bak-temphook"
                if not os.path.exists(bak):
                    shutil.copy(settings_path, bak)
                fd, tmp = tempfile.mkstemp(dir=os.path.dirname(settings_path) or ".",
                                           prefix=".temphook.", suffix=".tmp")
                try:
                    with os.fdopen(fd, "w") as f:
                        json.dump(data, f, indent=2, ensure_ascii=False)
                    shutil.copystat(settings_path, tmp)
                    os.replace(tmp, settings_path)
                except Exception:
                    if os.path.exists(tmp):
                        os.unlink(tmp)
                    raise
            except Exception as e:
                return (0, str(e))
        return (removed, "")

    def c57_temp_hook_leak(self):
        cid = "C57.temp-hook-leak"
        if self.skipped(cid):
            return
        targets = discover_claude_settings()
        if not targets:
            self.add(cid, WARN, "~/.claude*/settings.json 미발견(temp-pack 컨텍스트면 정상)")
            return
        leaks = {t: self._temp_hooks_in(t) for t in targets}
        leaks = {t: v for t, v in leaks.items() if v}
        if not leaks:
            self.add(cid, PASS, "%d개 settings에 temp 훅 누수 0 (invariant 충족)" % len(targets))
            return
        total = sum(len(v) for v in leaks.values())
        summary = ", ".join("%s:%d" % (os.path.basename(os.path.dirname(t)), len(v))
                            for t, v in leaks.items())
        if self.fix:
            done, errs = [], []
            for t in leaks:
                n, err = self._strip_temp_hooks(t)
                if err:
                    errs.append("%s: %s" % (os.path.basename(os.path.dirname(t)), err))
                else:
                    done.append("%s(-%d)" % (os.path.basename(os.path.dirname(t)), n))
            if errs:
                self.add(cid, WARN, "일부 청소 실패: %s | 성공: %s"
                         % ("; ".join(errs), ", ".join(done)))
            else:
                self.add(cid, FIXED, "temp 훅 누수 %d개 제거(비-temp 보존·백업): %s — ★claude 재시작 후 적용"
                         % (total, ", ".join(done)))
        else:
            self.add(cid, FAIL, "글로벌 settings temp 훅 누수 %d개 탐지: %s (--fix로 청소)"
                     % (total, summary))

    # ── C09 round 핵심 문서 ──
    def c09_round_core(self):
        cid = "C09.round-core"
        if self.skipped(cid):
            return
        missing = []
        for f in ("SESSION_STATE.md", "RECOVERY.md"):
            p = os.path.join(pack_dir(), "round", f)
            if not os.path.isfile(p):
                missing.append(f)
        if missing and self.fix and self.repair_via_init_pack():
            missing = [
                f for f in missing
                if not os.path.isfile(os.path.join(pack_dir(), "round", f))
            ]
            if not missing:
                self.add(cid, FIXED, "round 핵심 문서 재설치")
                return
        if missing:
            self.add(cid, FAIL, "누락: %s" % ", ".join(missing))
        else:
            self.add(cid, PASS, "SESSION_STATE.md·RECOVERY.md 존재")

    # ── C10 전 노드 TODO 영속 파일 (절대지침 7) ──
    def c10_todo_files(self):
        cid = "C10.todo-files"
        if self.skipped(cid):
            return
        rdir = os.path.join(pack_dir(), "round")
        missing = [f for f in TODO_FILES if not os.path.isfile(os.path.join(rdir, f))]
        if not missing:
            self.add(cid, PASS, "4개 노드 TODO 전부 존재")
            return
        if self.fix:
            os.makedirs(rdir, exist_ok=True)
            for f in missing:
                node = f.replace("_TODO.md", "")
                open(os.path.join(rdir, f), "w", encoding="utf-8").write(
                    "# %s_TODO — 영속 todo (절대지침 7)\n\n"
                    "> 세부 완료마다 갱신·디스크 영속. 세션 clear/재시작 후 이 파일부터 읽고 복원한다.\n\n"
                    "- [ ] (작업을 추가하라)\n" % node
                )
            self.add(cid, FIXED, "생성: %s" % ", ".join(missing))
        else:
            self.add(cid, FAIL, "누락: %s — --fix로 생성 가능" % ", ".join(missing))

    # ── C11 cys 바이너리 ──
    def c11_cys_binary(self):
        cid = "C11.cys-binary"
        if self.skipped(cid):
            return
        p = shutil.which("cys")
        if p:
            self.add(cid, PASS, p)
        else:
            self.add(cid, FAIL, "PATH에 cys 없음 — cys 터미널 설치/PATH 확인 필요")

    # ── C11b cys-dept PATH 노출 (Fix3' — CEO fan-out `cys-dept list`가 풀경로 없이 동작) ──
    #
    # cys·cysd는 컴파일 바이너리지만 cys-dept는 팩 bash 스크립트라 PATH에 없다 → CEO 디렉티브
    # (CEO_TEMPLATE.md `cys-dept list` fan-out)·CSO reap·javis_org.py create_dept 의 bare
    # `cys-dept` 호출이 풀경로 없이 실패한다(GUI/백엔드는 pack_dir()/bin 풀경로라 무관).
    #
    # ★2026-08-28 실사고(코드서명 봉인 파손)로 재설계 — 종전 구현은 링크를 `dirname(which("cys"))`
    #   ("cys 와 같은 PATH dir")에 만들었는데, 페인 PATH **선두**가 `/Applications/cys.app/
    #   Contents/MacOS` 라서 링크가 **앱 번들 안에** 생겼고 codesign 봉인이 깨졌다(spctl
    #   "a sealed resource is missing or invalid" — C76 이 검출하는 그 파손의 생산자가 바로 이
    #   검사였다). 게다가 which("cys-dept") realpath 일치 PASS 단락이 번들 안 링크를 영원히
    #   PASS 로 덮어 번들 밖으로의 이행이 구조적으로 불가능했다. 재설계 4원칙:
    #   ① 링크 위치는 `~/.local/bin` 고정(없으면 생성) — *.app 번들 안(특히 Contents/MacOS)에는
    #      절대 만들지 않는다. `/usr/local/bin` 도 금지(root 소유·sudo 필요·D5 규약 위반).
    #      해소 보장: unix 페인 PATH 는 `~/.local/bin` 을 말미에 무조건 append 한다(src/lib.rs
    #      compose_unix_pane_path — bare 소비자 3곳 전부 페인에서 실행되므로 해소된다).
    #   ② 링크 타깃은 **base 팩**(base_pack_dir()/bin/cys-dept)으로 고정 — dept 레인의 --fix 가
    #      pack_dir()(=pack-dept-*)을 타깃으로 잡으면 레인이 바뀔 때마다 타깃이 플랩한다.
    #   ③ 레거시 자기치유: 번들 안에 존재하거나·번들 안을 가리키거나·base 팩이 아닌 곳을
    #      가리키는 기존 `cys-dept` 심링크는 fix 모드에서 **먼저 unlink** 한다(번들 안 링크는
    #      '추가된 파일'이라 지우는 것만으로 봉인이 복구된다 — C76 복구 문구와 동일 실측).
    #   ④ Windows(os.name == "nt")는 SKIP — 심링크에 관리자 권한/개발자 모드가 필요하고
    #      cys-dept 는 unix 페인 전용이다.
    #   가역·멱등·자가치유(실파일은 덮지 않음) — 비가역 외부설치가 아니므로 may_mutate 불요
    #   (reversible local · unlink 대상도 이 검사가 만든 재생성 가능한 심링크뿐).

    @staticmethod
    def _in_app_bundle(path):
        """조상 성분에 `*.app` 이 있거나 `Contents/MacOS` 아래인가(문자 판정·심링크 해소 없음).
        링크 **위치**의 봉인 위반을 묻는 판정이라 realpath 를 쓰지 않는다 — "번들 안을
        가리키는가"(타깃 판정)가 필요한 호출부는 realpath 를 먼저 적용해 넘긴다."""
        parts = [p for p in os.path.normpath(os.path.abspath(path)).split(os.sep) if p]
        dirs = parts[:-1]  # 마지막 성분(링크 자신)은 위치가 아니다
        if any(c.endswith(".app") for c in dirs):
            return True
        return any(a == "Contents" and b == "MacOS" for a, b in zip(dirs, dirs[1:]))

    def c11b_cys_dept_path(self):
        cid = "C11b.cys-dept-path"
        if self.skipped(cid):
            return
        if os.name == "nt":
            self.add(cid, SKIP, "Windows — 심링크 권한 제약·cys-dept 는 unix 페인 전용(미해당)")
            return
        src = os.path.join(base_pack_dir(), "bin", "cys-dept")
        if not os.path.isfile(src):
            self.add(cid, WARN, "base 팩에 bin/cys-dept 없음(%s) — `cys init-pack` 재실행" % src)
            return
        real_src = os.path.realpath(src)
        link = os.path.join(os.path.expanduser("~"), ".local", "bin", "cys-dept")
        if self._in_app_bundle(link):
            # 정상 기계에선 나올 수 없는 형상(HOME 이 번들 안?)이지만, 어떤 경로로도 번들 안
            # 생성은 거부한다 — 이 가드가 이 사고의 재발 불변식이다.
            self.add(cid, WARN, "링크 위치 %s 가 앱 번들 안 — 생성 거부(봉인 보호 불변식)" % link)
            return
        # 레거시 후보: ⓐ 현재 셸이 해소하는 cys-dept ⓑ 종전 구현의 생성 위치(cys 와 같은 dir)
        # ⓒ 실사용 번들의 Contents/MacOS(페인 밖 실행이라 ⓐⓑ의 PATH 가 번들을 못 볼 때의 그물)
        cys = shutil.which("cys")
        bundle = self._app_bundle_of(cys) if cys else None
        cands, seen = [], set()
        for cand in (shutil.which("cys-dept"),
                     os.path.join(os.path.dirname(cys), "cys-dept") if cys else None,
                     os.path.join(bundle, "Contents", "MacOS", "cys-dept") if bundle else None):
            if not cand:
                continue
            cand = os.path.abspath(cand)
            if cand != link and cand not in seen:
                seen.add(cand)
                cands.append(cand)
        legacy, manual = [], []
        for cand in cands:
            if os.path.islink(cand):
                if (self._in_app_bundle(cand)                           # 번들 안에 존재(봉인 파손)
                        or self._in_app_bundle(os.path.realpath(cand))  # 번들 안을 가리킴
                        or os.path.realpath(cand) != real_src):         # 오타깃(dept 팩 flap 등)
                    legacy.append(cand)
            elif os.path.isfile(cand) and self._in_app_bundle(cand):
                manual.append(cand)  # 번들 안 '실파일' — 자동 삭제 금지(수동 안내만)
        ok = self._symlink_ok(link, src)
        blocked = os.path.exists(link) and not os.path.islink(link)  # 실파일 — 덮지 않는다
        if ok and not legacy and not manual:
            self.add(cid, PASS, "%s → %s (unix 페인 PATH 말미에 ~/.local/bin 자동 append — "
                                "페인 밖 셸은 PATH 에 ~/.local/bin 추가 필요)" % (link, src))
            return
        if not self.fix:
            probs = []
            if legacy:
                probs.append("레거시 링크 정리 필요(번들 안 링크는 봉인 파손·오타깃은 flap): %s"
                             % ", ".join(legacy))
            if manual:
                probs.append("번들 안 실파일 %s — 수동 삭제 시 봉인 복구" % ", ".join(manual))
            if blocked:
                probs.append("%s 실파일 존재 — 자동 심링크 보류(수동 확인)" % link)
            elif not ok:
                probs.append("~/.local/bin/cys-dept 미설치")
            self.add(cid, WARN, " · ".join(probs)
                     + " — --fix 로 이관(레거시 unlink → ~/.local/bin 심링크·base 팩 타깃)")
            return
        # fix: 봉인 우선 — 레거시 unlink 를 생성보다 먼저, 실패해도 나머지 치유는 계속한다.
        notes = []
        for cand in legacy:
            try:
                os.unlink(cand)
                notes.append("레거시 unlink: %s" % cand)
            except OSError as e:
                # 번들 안 삭제는 App Management(TCC)에 막힐 수 있다 — '추가된 파일'이라
                # 지우면 봉인이 복구되므로 아래 WARN 이 수동 rm 을 안내한다.
                notes.append("레거시 unlink 실패(%s): %s" % (e, cand))
        try:
            # PATH 해소(which/셸)는 대상이 실행가능해야 한다 — 팩 cys-dept가 0644면 심링크해도
            # `cys-dept`가 안 잡힌다. 실행비트를 보강(가역·멱등)한 뒤 심링크한다(Fix1' 유지).
            st = os.stat(src).st_mode
            if not (st & 0o111):
                os.chmod(src, st | 0o111)
            if not ok and not blocked:
                os.makedirs(os.path.dirname(link), exist_ok=True)
                if os.path.islink(link):
                    os.unlink(link)  # stale/오타깃 심링크 교체
                os.symlink(src, link)
                notes.append("%s → %s 심링크(+x 보강)" % (link, src))
        except OSError as e:
            self.add(cid, WARN, "심링크/실행비트 실패(%s 쓰기권한?): %s — %s"
                     % (os.path.dirname(link), e, "; ".join(notes) or "무진행"))
            return
        leftovers = [c for c in legacy if os.path.lexists(c)] + manual
        if blocked:
            self.add(cid, WARN, "%s 실파일 존재 — 자동 심링크 보류(수동 확인)%s"
                     % (link, (" · " + "; ".join(notes)) if notes else ""))
        elif leftovers:
            self.add(cid, WARN, "이관 부분 완료 — 번들 안 잔재는 수동 rm 필요(추가 파일이라 "
                                "지우면 봉인 복구): %s · %s"
                     % (", ".join(leftovers), "; ".join(notes) or "무진행"))
        else:
            self.add(cid, FIXED, "%s (페인 PATH 말미 ~/.local/bin 으로 해소 — "
                                 "src/lib.rs compose_unix_pane_path)" % ("; ".join(notes) or "정합 확인"))

    # ── C12 cysd 데몬 생존 ──
    def c12_daemon(self):
        cid = "C12.daemon"
        if self.skipped(cid):
            return
        cys = shutil.which("cys")
        if not cys:
            self.add(cid, SKIP, "cys 부재로 판정 불가 (C11 먼저)")
            return

        def ping():
            try:
                return subprocess.run(
                    [cys, "ping"], capture_output=True, timeout=5
                ).returncode == 0
            except Exception:
                return False

        if ping():
            self.add(cid, PASS, "cys ping OK")
            return
        cysd = shutil.which("cysd")
        if self.fix and cysd:
            log = open("/tmp/cysd-preflight.log", "ab") if os.name == "posix" else subprocess.DEVNULL
            subprocess.Popen(
                [cysd], stdout=log, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            for _ in range(10):
                time.sleep(0.5)
                if ping():
                    self.add(cid, FIXED, "cysd 기동 후 ping OK")
                    return
            self.add(cid, FAIL, "cysd 기동 시도했으나 ping 실패 — /tmp/cysd-preflight.log 확인")
        else:
            self.add(cid, FAIL, "데몬 다운 — `cysd > /tmp/cysd.log 2>&1 &` 후 재실행 (--fix로 자동 기동 가능)")

    # ── C13 프로젝트 CLAUDE.md (git 루트에서만) ──
    def c13_claude_md(self):
        cid = "C13.claude-md"
        if self.skipped(cid):
            return
        if not os.path.isdir(".git"):
            self.add(cid, SKIP, "cwd가 git 루트 아님")
            return
        if os.path.isfile("CLAUDE.md"):
            self.add(cid, PASS, "프로젝트 CLAUDE.md 존재")
            return
        tpl = os.path.join(pack_dir(), "CLAUDE.md.template")
        if self.fix and os.path.isfile(tpl):
            shutil.copy2(tpl, "CLAUDE.md")
            self.add(cid, FIXED, "CLAUDE.md.template → ./CLAUDE.md 배치")
        else:
            self.add(cid, WARN, "프로젝트 CLAUDE.md 없음 (hook이 전역 커버하므로 권장 수준) — --fix로 배치 가능")

    # ── C14 프리플라이트 자기 존재 (pack 영구 편입 확인) ──
    def c14_self(self):
        cid = "C14.preflight-self"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "bin", "javis_preflight.py")
        if os.path.isfile(p):
            self.add(cid, PASS, p)
            return
        if self.fix:
            os.makedirs(os.path.dirname(p), exist_ok=True)
            shutil.copy2(os.path.abspath(__file__), p)
            os.chmod(p, 0o755)
            self.add(cid, FIXED, "자기 복제로 pack에 편입: %s" % p)
        else:
            self.add(cid, FAIL, "pack/bin/javis_preflight.py 없음 — `cys init-pack` 또는 --fix")

    # ── C15 진행% 보고기 javis_report.py (앵커3-A6) ──
    def c15_report_tool(self):
        cid = "C15.report-tool"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "bin", "javis_report.py")
        if not os.path.isfile(p):
            if self.fix and self.repair_via_init_pack() and os.path.isfile(p):
                self.add(cid, FIXED, "javis_report.py 재설치")
            else:
                self.add(cid, FAIL, "pack/bin/javis_report.py 없음 — `cys init-pack` 또는 --fix")
            return
        self.add(cid, PASS, p)

    # ── C16 5분 주기 보고 스케줄 job (앵커3-A6) ──
    def c16_report_schedule(self):
        cid = "C16.report-schedule"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "schedule.json")
        try:
            data = json.load(open(p, encoding="utf-8"))
        except (OSError, ValueError) as e:
            self.add(cid, FAIL, "schedule.json 읽기/파싱 실패: %s (C06 먼저)" % e)
            return
        jobs = data.get("jobs", [])
        # 절대지침 "매 5분" — every_minutes는 5 이하만 충족(더 자주는 명세 이상, 더 길면 위반).
        # 5분 보고 체계는 두 형태를 허용한다(체계 존재 보장이 의도·형태는 확장):
        #   ①구 push 보고 잡: action=="push" to=="master" text|text_command (하위호환)
        #   ②델타게이트 잡: action=="command" command에 'javis_report_gate.py' 포함
        #     (하트비트 델타게이트로 마이그레이션 — 게이트가 javis_report를 소비·판정·배달 소유).
        # 게이트 잡을 인식하지 못하면 --fix가 부팅마다 구 push 잡을 재생성해 마이그레이션을
        # 되돌리고 구/신 이중발화를 만든다 — 그래서 두 형태를 모두 report 체계로 판정한다.
        def is_push_report(j):
            return (isinstance(j.get("every_minutes"), int)
                    and j.get("action") == "push" and j.get("to") == "master"
                    and (j.get("text") or j.get("text_command")))

        def is_gate_report(j):
            return (isinstance(j.get("every_minutes"), int)
                    and j.get("action") == "command"
                    and "javis_report_gate.py" in (j.get("command") or ""))

        def is_report(j):
            return is_push_report(j) or is_gate_report(j)
        rep = [j for j in jobs if is_report(j) and 1 <= j.get("every_minutes") <= 5]
        too_slow = [j for j in jobs if is_report(j) and j.get("every_minutes") > 5]
        if rep:
            j = rep[0]
            if is_gate_report(j):
                mode = "command(하트비트 델타게이트)"
            elif j.get("text_command"):
                mode = "text_command(결정론 직접산출)"
            else:
                mode = "text(master 산출)"
            # ★B7 레인 분리 마이그레이션(T-0147-2 §2 층3): 게이트 잡이 있어도 레인 env 가
            #   배선되지 않았으면 여러 데몬이 같은 state 를 밟는다. --fix 는 **토큰 보존 삽입**만
            #   한다(재생성 금지 — `run --shadow` 같은 기존 인자 소실 차단).
            unwired = [x for x in jobs if is_gate_report(x)
                       and GATE_STATE_ENV not in (x.get("command") or "")]
            if unwired:
                lane = gate_state_dir_for_pack()
                if self.fix:
                    for x in unwired:
                        x["command"] = gate_command_with_lane(x["command"], lane)
                    data["jobs"] = jobs
                    open(p, "w", encoding="utf-8").write(
                        json.dumps(data, ensure_ascii=False, indent=2))
                    self.add(cid, FIXED,
                             "게이트 잡 %d건에 레인 배선 삽입(%s=%s) — 데몬별 state 분리"
                             % (len(unwired), GATE_STATE_ENV, lane))
                else:
                    self.add(cid, FAIL,
                             "게이트 잡에 레인 배선(%s) 부재 — 다중 데몬 state 공유 위험. "
                             "--fix로 삽입 가능(기존 인자 보존)" % GATE_STATE_ENV)
                return
            self.add(cid, PASS, "5분 보고 job 존재: %s (every_minutes=%s ≤5, %s)"
                     % (j.get("id"), j.get("every_minutes"), mode))
            return
        if too_slow and not self.fix:
            j = too_slow[0]
            self.add(cid, FAIL, "보고 주기가 너무 김: %s (every_minutes=%s > 5) — 절대지침 5분 위반"
                     % (j.get("id"), j.get("every_minutes")))
            return
        if self.fix:
            # ★reviewer1 P1 교정: 보고 잡 전무 시 추가하는 잡은 구 push 보고 잡이 아니라 델타게이트
            #   잡(action:command)이다 — 구 push 잡 부활은 이 프로젝트가 제거하는 대상 그 자체다.
            jobs.append({
                "id": "owner-progress-gate-5min",
                "every_minutes": 5,
                "action": "command",
                # ★B7: 레인 state_dir 을 **리터럴로 bake** 한다(런타임 env 오염에 비의존).
                "command": gate_command_with_lane(
                    'python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_report_gate.py" run',
                    gate_state_dir_for_pack()),
                "if_absent": "skip",
            })
            data["jobs"] = jobs
            open(p, "w", encoding="utf-8").write(
                json.dumps(data, ensure_ascii=False, indent=2))
            self.add(cid, FIXED, "5분 보고 job(owner-progress-gate-5min·델타게이트) 추가")
        else:
            self.add(cid, FAIL, "5분 주기 master 보고 job 부재 — --fix로 추가 가능")

    # ── 공용: pack/bin 도구 존재 확보 + --self-test 실행 ──
    def _check_bin_tool(self, cid, fname, extra_files=()):
        """bin 도구의 존재(누락 시 init-pack 수리)·자기검증을 결정론으로 판정한다."""
        p = os.path.join(pack_dir(), "bin", fname)
        missing = [f for f in (fname,) + tuple(extra_files)
                   if not os.path.isfile(os.path.join(pack_dir(), "bin", f))]
        if missing and self.fix and self.repair_via_init_pack():
            missing = [f for f in missing
                       if not os.path.isfile(os.path.join(pack_dir(), "bin", f))]
        if missing:
            self.add(cid, FAIL, "pack/bin 누락: %s — `cys init-pack` 또는 --fix"
                     % ", ".join(missing))
            return None
        if os.name == "posix" and not os.stat(p).st_mode & stat.S_IXUSR and self.fix:
            os.chmod(p, 0o755)
        try:
            r = subprocess.run([sys.executable, p, "--self-test"],
                               capture_output=True, timeout=30, env=_utf8_env())
        except Exception as e:
            self.add(cid, FAIL, "%s --self-test 실행 불가: %s" % (fname, e))
            return None
        if r.returncode != 0:
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            self.add(cid, FAIL, "%s --self-test 실패: %s" % (fname, tail[-400:]))
            return None
        return p

    # ── C17 3단 사고 라우팅 결정론 엔진 (사고 모드 §1) ──
    def c17_route_engine(self):
        cid = "C17.route-engine"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_route.py",
                                 extra_files=("route_triggers.json",))
        if p:
            self.add(cid, PASS, "%s self-test OK (로직 배터리 + 트리거 구조 검증)" % p)

    # ── C19 LLM 오케스트레이션 결정론 도구 (앵커4) ──
    def c19_orchestra_engine(self):
        cid = "C19.orchestra-engine"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_orchestra.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (4종 노드·라운드·제약 주입 검증)" % p)

    # ── C41 스킬 보안·품질 결정론 게이트 (SkillSpector 규칙 stdlib 포트) ──
    def c41_skillscan(self):
        cid = "C41.skillscan"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_skillscan.py",
                                 extra_files=("skillscan_rules.json",))
        if p:
            self.add(cid, PASS, "%s self-test OK (포트 규칙 + fixture recall + verdict 검증)" % p)

    # ── C42 MCP 거버넌스 결정론 게이트 (tool-poisoning·rug-pull) ──
    def c42_mcpgate(self):
        cid = "C42.mcpgate"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_mcpgate.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (TP1~3·RP1~3 검증)" % p)

    # ── C34 자기기술 능력 레지스트리 (OpenMontage D2 — 하드코딩 목록 폐기·파생 카탈로그) ──
    def c34_registry(self):
        cid = "C34.registry"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_registry.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (능력 카탈로그 파생·orphan lint·무점수)" % p)

    # ── C35 채점식 provider 선택 엔진 (OpenMontage P5 — deny-by-default·무료우선) ──
    def c35_select(self):
        cid = "C35.select"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_select.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (7차원 채점·deny-by-default·무료우선)" % p)

    # ── C36 리뷰어 verdict 스키마검증 + CHAI lint (OpenMontage D1 — 4자수렴 기계검증부) ──
    def c36_verdict(self):
        cid = "C36.verdict"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_verdict.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (verdict 계약·점수금지·CHAI R2 강등)" % p)

    # ── C37 의사결정 로그(OpenMontage D3 — Options Considered·rejected_because·근거·무점수) ──
    def c37_adr_engine(self):
        cid = "C37.adr-engine"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_adr.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (결정근거 rationale·커버리지 게이트·동거 ledger)" % p)

    # ── C38 무음실패 카탈로그 (OpenMontage D5 2부 — 런타임-write 미러[C10/C16/C25]·드리프트 WARN·--fix 재생성) ──
    # _check_bin_tool 아님: 검사 대상은 bin 도구가 아니라 round/ 런타임 아티팩트다. SILENT_FAILURES
    # 단일 source-of-record를 보존하려 렌더/드리프트 판정은 javis_orchestra에 위임(subprocess).
    def c38_silent_failure_catalog(self):
        cid = "C38.silent-failure-catalog"
        if self.skipped(cid):
            return
        orch = os.path.join(pack_dir(), "bin", "javis_orchestra.py")
        cat = os.path.join(pack_dir(), "round", "SILENT_FAILURE_CATALOG.md")
        if not os.path.isfile(orch):
            self.add(cid, WARN, "javis_orchestra.py 부재 — 무음실패 카탈로그 검사 보류")
            return
        try:
            if self.fix:
                r = subprocess.run([sys.executable, orch, "silent-failure-catalog"],
                                   capture_output=True, timeout=30, env=_utf8_env())
                if r.returncode == 0:
                    self.add(cid, FIXED, "무음실패 카탈로그 재생성: %s" % cat)
                else:
                    tail = (r.stderr or r.stdout or b"").decode("utf-8", "replace").strip()
                    self.add(cid, WARN, "무음실패 카탈로그 재생성 실패: %s" % tail[-200:])
                return
            r = subprocess.run([sys.executable, orch, "silent-failure-catalog", "--check"],
                               capture_output=True, timeout=30, env=_utf8_env())
            if r.returncode == 0:
                self.add(cid, PASS, "무음실패 카탈로그 정합 (런타임 파생·D5 거버넌스)")
            else:
                self.add(cid, WARN, "무음실패 카탈로그 드리프트/부재 — `javis_preflight.py --fix` 또는 "
                         "`javis_orchestra.py silent-failure-catalog`로 재생성")
        except Exception as e:
            # WARN-only 계약 유지: 카탈로그 검사 실패(타임아웃·OSError)가 전체 preflight를 죽이지
            # 않게 한다(형제 검사 C18·_check_bin_tool의 except Exception 패턴 미러).
            self.add(cid, WARN, "무음실패 카탈로그 검사 실행 불가 — 보류: %s" % e)

    # ── C39 전제지식 고아 lint (OpenMontage D6 — requires_skills/related_memory 슬러그 해소·WARN-only) ──
    # registry verify에 위임(normalize_slug·색인 규칙 단일 source-of-record 보존 — preflight는
    # orchestra/registry를 import 안 함). orphan 문제만 골라 WARN(드리프트·점수 위반은 registry 몫).
    # orphan 탐지 *로직* 정합은 C34(registry --self-test, synthetic orphan-ref/mem)가 핀하고,
    # C39는 그 로직을 라이브 pack 데이터에 적용하는 표면이다(C19↔orchestra 쌍과 동형).
    def c39_prereq_orphan_lint(self):
        cid = "C39.prereq-orphan-lint"
        if self.skipped(cid):
            return
        reg = os.path.join(pack_dir(), "bin", "javis_registry.py")
        if not os.path.isfile(reg):
            self.add(cid, WARN, "javis_registry.py 부재 — 전제지식 고아 lint 보류")
            return
        try:
            # --root로 lint 대상을 preflight가 보는 pack에 핀(env 재유도 분기 차단).
            r = subprocess.run([sys.executable, reg, "verify", "--root", pack_dir(), "--json"],
                               capture_output=True, timeout=30, env=_utf8_env())
            data = json.loads((r.stdout or b"").decode("utf-8", "replace") or "{}")
        except Exception as e:
            self.add(cid, WARN, "전제지식 고아 lint 실행 불가 — 보류: %s" % e)
            return
        orphans = [p for p in data.get("problems", []) if p.startswith("orphan ")]
        if orphans:
            self.add(cid, WARN, "전제지식 고아 %d건(requires_skills/related_memory 색인 미해소) — %s"
                     % (len(orphans), " · ".join(orphans[:3])))
        else:
            self.add(cid, PASS, "전제지식 고아 0 — requires_skills/related_memory 색인 해소 정합")

    # ── C40 워크플로우 매니페스트 도구 (OpenMontage D4 — 신규 *옵션* 도구·WARN-only) ──
    # _check_bin_tool 아님: 그건 부재·self-test 실패를 FAIL로 만든다. 매니페스트는 opt-in
    # (없으면 resolve exit 4 → README 디스패치 폴백)이라 boot-blocker가 아니다 → WARN.
    def c40_workflow_manifest(self):
        cid = "C40.workflow-manifest"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "bin", "javis_manifest.py")
        if not os.path.isfile(p):
            self.add(cid, WARN, "javis_manifest.py 부재 — 타입드 워크플로우 매니페스트 미설치(opt-in·README 폴백)")
            return
        try:
            r = subprocess.run([sys.executable, p, "--self-test"],
                               capture_output=True, timeout=30, env=_utf8_env())
        except Exception as e:
            self.add(cid, WARN, "javis_manifest.py --self-test 실행 불가 — 보류: %s" % e)
            return
        if r.returncode == 0:
            self.add(cid, PASS, "javis_manifest.py self-test OK (매니페스트 계약·무점수·콘텐츠 checks·exit 4 폴백)")
        else:
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            self.add(cid, WARN, "javis_manifest.py self-test 실패(도구 점검 필요) — %s" % tail[-200:])

    # ── C18 장기기억 증류 결정론 도구 + 색인↔파일 정합 (§10 증류 게이트) ──
    def c18_memory_engine(self):
        cid = "C18.memory-engine"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_memory.py")
        if not p:
            return
        # 실 데이터 정합 — MEMORY.md 색인과 메모리 파일의 기계검증.
        # 자동 수리 없음: 기억 내용은 오너·노드 소관이라 preflight가 임의 재작성하지 않는다.
        try:
            r = subprocess.run([sys.executable, p, "verify", "--json"],
                               capture_output=True, timeout=15, env=_utf8_env())
        except Exception as e:
            self.add(cid, FAIL, "javis_memory verify 실행 불가: %s" % e)
            return
        if r.returncode == 0:
            self.add(cid, PASS, "self-test OK + 장기기억 색인↔파일 정합")
        else:
            tail = (r.stdout or b"").decode("utf-8", "replace").strip()
            self.add(cid, FAIL, "장기기억 부정합 — 수동 복구 필요: %s" % tail[-400:])

    # ── C20 보조: nlm 버전 탐지 / 설치 / MCP 등록 ──
    @staticmethod
    def _nlm_version():
        nlm = shutil.which("nlm")
        if not nlm:
            return None, None
        try:
            out = subprocess.run([nlm, "--version"], capture_output=True,
                                 timeout=15).stdout.decode("utf-8", "replace")
            m = re.search(r"(\d+)\.(\d+)\.(\d+)", out)
            return nlm, (tuple(int(x) for x in m.groups()) if m else None)
        except Exception:
            return nlm, None

    @staticmethod
    def _install_nlm():
        """uv → pipx → pip 폴백으로 핀 버전 설치. 성공 여부 반환."""
        candidates = []
        if shutil.which("uv"):
            candidates.append(["uv", "tool", "install", "--force", NLM_PIN])
        if shutil.which("pipx"):
            candidates.append(["pipx", "install", "--force", NLM_PIN])
        candidates.append([sys.executable, "-m", "pip", "install", "--user",
                           "--upgrade", NLM_PIN])
        for cmd in candidates:
            try:
                if subprocess.run(cmd, capture_output=True, timeout=600).returncode == 0:
                    return True
            except Exception:
                continue
        return False

    def _register_mcp(self, mcp_path, name, binary, env=None, args=None):
        """프로젝트 .mcp.json에 MCP 서버 등록(merge). 성공=None, 실패=사유.
        binary는 PATH에서 절대경로로 해석해 박는다. env는 그대로 기입
        (값에 ${VAR}를 쓰면 Claude Code가 세션 환경변수로 전개한다).
        args는 list[str](argv 토큰) — uvx 온디맨드 런치처럼 서브커맨드 체인이
        필요한 stdio 서버용. truthy일 때만 기입(env 경로와 동형, back-compat)."""
        if os.path.islink(mcp_path):
            return "symlink 거부: %s" % mcp_path
        server = shutil.which(binary)
        if not server:
            return "%s 실행파일 미발견 (설치 먼저)" % binary
        data = {}
        if os.path.isfile(mcp_path):
            try:
                data = json.load(open(mcp_path, encoding="utf-8"))
            except (OSError, ValueError) as e:
                return "기존 .mcp.json 파싱 실패 — 덮어쓰기 거부: %s" % e
            if not isinstance(data, dict):
                return ".mcp.json 루트가 객체가 아님 — 거부"
            backup = mcp_path + ".bak-preflight"
            if not os.path.exists(backup):
                shutil.copy2(mcp_path, backup)
        entry = {"command": server}
        if env:
            entry["env"] = env
        if args:
            entry["args"] = args
        data.setdefault("mcpServers", {})[name] = entry
        # 원자적 쓰기 — settings.json 쓰기와 동일 사유(파손 시 수리 불능 차단).
        tmp = mcp_path + ".tmp"
        open(tmp, "w", encoding="utf-8").write(
            json.dumps(data, ensure_ascii=False, indent=2))
        os.replace(tmp, mcp_path)
        return None

    @staticmethod
    def _mcp_registered(mcp_path, name):
        """mcpServers에 정확한 서버 키가 있는가 — 전체 JSON 부분문자열 검사는
        무관한 값(경로·URL)에 오탐해 --fix가 실제 등록을 영영 건너뛴다."""
        if not os.path.isfile(mcp_path):
            return False
        try:
            cfg = json.load(open(mcp_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        return isinstance(cfg, dict) and name in cfg.get("mcpServers", {})

    # ── C43 보조: Serena 활성화(enable/trust) 탐지·write + sub-stage 탐지기 ──
    # 등록(.mcp.json)은 기계가, 활성화(노드 .claude.json enable/trust)는 사람 전용(denylist).
    # 기계는 gap을 탐지해 WARN만 내고, _serena_enable_approved() 토큰이 있을 때만 write한다.
    @staticmethod
    def _serena_nodes():
        # 워커 프로필 디렉터리 외부화(공개 배포에서 개인 프로필명 제거):
        # $CYS_WORKER_PROFILE_DIR → ~/.cys/worker-profile-dir 파일(1줄) → ~/.claude-worker 기본
        wp = os.environ.get("CYS_WORKER_PROFILE_DIR", "")
        if not wp:
            try:
                wp = open(os.path.expanduser("~/.cys/worker-profile-dir"), encoding="utf-8").read().strip()
            except OSError:
                wp = ""
        wp = os.path.expanduser(wp or "~/.claude-worker")
        return [("master", os.path.expanduser("~/.cys/claude/.claude.json"), False),
                ("worker", os.path.join(wp, ".claude.json"), True)]

    @staticmethod
    def _mcp_enabled(config_path, project_root, name):
        """(enabled_bool, trust_bool) — 노드 .claude.json의 project별 활성화·신뢰 상태.
        ★R5(리뷰 minor): 판독은 `_read_json_tolerant`(O_NONBLOCK + fstat 정규 재확인) — 종전 무가드 `open` 은
        `.claude.json` 이 writer 없는 FIFO 면 preflight 를 영구 정지시켰다(`isfile` 선검사가 있어도 그 사이 교체되면
        TOCTOU). 판독 실패는 종전과 같은 (False, False)."""
        d = _read_json_tolerant(config_path)
        if not isinstance(d, dict):
            return (False, False)
        pe = (d.get("projects", {}) or {}).get(project_root, {}) or {}
        en = (name in (pe.get("enabledMcpjsonServers", []) or [])) \
            or bool(pe.get("enableAllProjectMcpServers"))
        return (en, bool(pe.get("hasTrustDialogAccepted")))

    def _serena_activation_gaps(self):
        """읽기전용 탐지 — .claude.json 미변경. enable/trust 미충족 항목 리스트."""
        gaps = []
        for label, path, needs_trust in self._serena_nodes():
            if not os.path.isfile(path):
                continue
            enabled, trust = self._mcp_enabled(path, SERENA_PROJECT, "serena")
            if not enabled:
                gaps.append("%s: enabledMcpjsonServers += 'serena'" % label)
            if needs_trust and not trust:
                gaps.append("%s: hasTrustDialogAccepted=true" % label)
        return gaps

    @staticmethod
    def _serena_enable_approved():
        """1회 오너 승인 토큰 — sentinel 파일 OR env. 없으면 절대 .claude.json write 금지."""
        if os.environ.get("CYS_SERENA_ENABLE_APPROVED") == "1":
            return True
        return os.path.isfile(os.path.expanduser("~/.cys/state/serena-enable-approved"))

    def _enable_mcp_server(self, config_path, project_root, name, set_trust=False):
        """None=ok/no-change, str=사유. denylist write — _serena_enable_approved() True일 때만 호출.
        ★R4(리뷰 minor · lost update): 이 파일은 `--seed-trust`/C58 이 커밋하는 바로 그 `.claude.json` 이다
        (`_serena_nodes()` 대상에 `~/.cys/claude/.claude.json` = C58 레지스트리 config 가 들어 있다) — 무잠금
        tmp+os.replace 는 방금 커밋된 신뢰 플래그를 조용히 잃는다. 시더와 **같은 잠금**(`.claude.json.seed-lock`)을
        비차단으로 잡고 그 아래에서 읽기~쓰기를 한다. 못 잡으면 **쓰지 않고 사유를 돌려준다**(호출자가 WARN 문면에
        싣는다 · 막는 쪽으로만 틀린다). 잠금 파일은 이 함수가 호출되는 경로(fix + 오너 승인 토큰)에서만 생긴다 —
        report/dry/safe 는 호출하지 않는다.
        한계(정직): ①claude 자신은 이 잠금을 모른다(advisory) — 그 창은 종전과 같다 ②커밋은 여전히 tmp+os.replace 라
        시더의 무손실 CAS(교환/link)가 아니다(같은 잠금을 쓰는 기록자끼리는 직렬화되므로 이 라운드의 결함은 닫힌다 ·
        CAS 승격은 백로그)."""
        if os.path.islink(config_path):
            return "symlink 거부: %s" % config_path
        lock_path = os.path.join(os.path.dirname(config_path) or ".", SEED_TRUST_LOCK_NAME)
        try:
            # 시더(`seed_trust._acquire`)와 **같은 열기**: O_NOFOLLOW — 잠금 파일이 심링크면 다른 inode 를 잠가
            # 시더와 직렬화되지 않는다(직렬화 실패 = 이 수정이 막으려던 lost update 가 그대로 남는다).
            lf = os.fdopen(_open_nofollow(lock_path, os.O_RDWR | os.O_CREAT), "r+")
        except OSError as e:
            return "시드 잠금 열기 실패 — 쓰기 보류: %s" % e
        try:
            got = _try_lock_nb(lf)
            if got is not True:
                return ("시드 잠금 %s — 쓰기 보류(%s · 다른 시더가 이 .claude.json 을 커밋 중)"
                        % ("경합" if got is False else "기구 미가용", lock_path))
            # ★R5(리뷰 minor): 이 판독은 **시드 잠금을 쥔 채** 일어난다 — 무가드 `open` 이 FIFO 에서 막히면
            #   preflight --fix 가 잠금을 든 채로 영구 정지한다(같은 라운드가 시더에서 닫은 결함과 같은 계급).
            #   `_read_json_tolerant` 는 O_NONBLOCK + fstat 정규 재확인이라 막히지 않고, 판독 실패는 무쓰기 사유다.
            data = _read_json_tolerant(config_path)
            if not isinstance(data, dict):
                return "판독/파싱 실패 — 거부(비정규 파일·손상·IO): %s" % config_path
            pr = data.setdefault("projects", {}).setdefault(project_root, {})
            ml = pr.setdefault("enabledMcpjsonServers", [])
            changed = False
            if name not in ml:
                ml.append(name); changed = True
            if set_trust and not pr.get("hasTrustDialogAccepted"):
                pr["hasTrustDialogAccepted"] = True; changed = True
            if not changed:
                return None
            backup = config_path + ".bak-preflight"
            if not os.path.exists(backup):
                shutil.copy2(config_path, backup)
            tmp = config_path + ".tmp"
            open(tmp, "w", encoding="utf-8").write(
                json.dumps(data, ensure_ascii=False, indent=2))
            os.replace(tmp, config_path)
            return None
        finally:
            try:
                lf.close()
            except OSError:
                pass

    @staticmethod
    def _serena_reviewer_gap():
        """S6 sub-stage(detect-only, never write): codex serena-ro 등록·read-only yml 존재 탐지."""
        toml = os.path.expanduser("~/.codex/config.toml")
        yml = os.path.join(pack_dir(), "resources", "contexts", "cys-codex-readonly.yml")
        miss = []
        if not (os.path.isfile(toml)
                and "serena-ro" in open(toml, encoding="utf-8", errors="replace").read()):
            miss.append("codex serena-ro 미등록")
        if not os.path.isfile(yml):
            miss.append("cys-codex-readonly.yml 부재")
        return (" · 리뷰어RO: " + ", ".join(miss)) if miss else ""

    @staticmethod
    def _serena_memory_isolated():
        """S8: serena args에 no-memories/no-onboarding 또는 project.yml excluded_tools(메모리)."""
        try:
            ent = json.load(open(".mcp.json", encoding="utf-8")) \
                .get("mcpServers", {}).get("serena", {})
            a = ent.get("args", []) or []
            if "no-memories" in a or "no-onboarding" in a:
                return True
        except (OSError, ValueError):
            pass
        pyml = os.path.join(SERENA_PROJECT, ".serena", "project.yml")
        if os.path.isfile(pyml):
            try:
                txt = open(pyml, encoding="utf-8", errors="replace").read()
                if "write_memory" in txt or "onboarding" in txt:
                    return True
            except OSError:
                pass
        return False

    def _serena_governance_gap(self):
        """S4/S8 sub-stage(detect-only WARN): probe·schedule job·메모리 격리 탐지. auto-register 금지."""
        miss = []
        probe = os.path.join(pack_dir(), "bin", "javis_serena_probe.py")
        if not os.path.isfile(probe):
            miss.append("probe 부재")
        else:
            try:
                rc = subprocess.run([sys.executable, probe, "--self-test"],
                                    capture_output=True, timeout=30, env=_utf8_env()).returncode
                if rc != 0:
                    miss.append("probe --self-test 실패")
            except Exception:
                miss.append("probe --self-test 실행불가")
        sched = os.path.join(pack_dir(), "schedule.json")
        try:
            jobs = json.load(open(sched, encoding="utf-8")).get("jobs", [])
            if not any(j.get("id") == "serena-heartbeat" for j in jobs):
                miss.append("serena-heartbeat job 부재(cys schedule add — 사람단계)")
        except (OSError, ValueError):
            miss.append("schedule.json 읽기불가")
        if not self._serena_memory_isolated():
            miss.append("메모리 미격리(S8 runbook)")
        return (" · 거버넌스: " + ", ".join(miss)) if miss else ""

    # ── C43 Serena 코드-의미 인덱스 MCP 채택 (등록 + 활성화 탐지 + reviewer/거버넌스 sub-stage) ──
    # 단일 c43_serena에 등록(기계)·활성화(사람·denylist 탐지)·reviewer RO(S6)·거버넌스/격리(S4/S8)를
    # sub-stage로 통합한다(별도 c43_ def 금지 = 중복 충돌). 등록≠활성: enable/trust는 사람 전용
    # (승인 토큰 있을 때만 write). 모든 사람-게이트는 WARN(never FAIL/block). C34~C42 점유 → C43.
    def c43_serena(self):
        cid = "C43.serena"
        if self.skipped(cid):
            return
        fixed = []
        # (a) 게이트: uvx PATH 필수 (Serena는 온디맨드, 미설치)
        uvx = shutil.which("uvx")
        if not uvx:
            self.add(cid, FAIL,
                     "uvx 미발견 — Serena 심볼 네비게이션 불가. uv/uvx 설치 후 재시도")
            return
        # (b) MCP 등록 — 등록 .mcp.json 디렉터리 = enable-key root 일치 필수(§1.4). CYSjavis는
        #     NOT git(실측)이라 c24의 .git 게이트만으론 영영 미등록 → cwd가 SERENA_PROJECT면
        #     .git 없이도 등록(P0.5 결정: 등록 스코프를 SERENA_PROJECT cwd로 확장, 무관 cwd는 제외).
        mcp_note = ""
        mcp_err = False
        cwd = os.path.abspath(".")
        if cwd == SERENA_PROJECT or os.path.exists(".git"):
            if not self._mcp_registered(".mcp.json", "serena"):
                if self.fix:
                    err = self._register_mcp(".mcp.json", "serena", "uvx",
                                             args=list(SERENA_STDIO_ARGS))
                    if err:
                        mcp_note = " · MCP 등록 실패: %s" % err
                        mcp_err = True
                    else:
                        fixed.append("./.mcp.json에 serena 등록(uvx args)")
                else:
                    mcp_note = " · ./.mcp.json MCP 미등록(--fix로 등록 가능)"
        else:
            mcp_note = (" · 등록 스코프 밖(cwd≠SERENA_PROJECT·.git 없음 · §1.5 P0.5) — "
                        "SERENA_PROJECT cwd에서 preflight 실행 필요")
        # (c) S5 Layer-0 assert: --context claude-code 무료 steering args가 실제 등록됐는지
        if self._mcp_registered(".mcp.json", "serena"):
            try:
                a = json.load(open(".mcp.json", encoding="utf-8")) \
                    .get("mcpServers", {}).get("serena", {}).get("args", []) or []
                if "claude-code" not in a:
                    mcp_note += " · ⚠ --context claude-code 누락(S5 steering inert)"
            except (OSError, ValueError):
                pass
        # (d) 활성화·신뢰 — 사람 전용(denylist). 기계는 상태만 알리고 명시 토큰 있을 때만 write.
        gaps = self._serena_activation_gaps()   # 읽기전용 탐지, .claude.json 미변경
        if gaps and self.fix and self._serena_enable_approved():
            why = []
            for label, path, needs_trust in self._serena_nodes():
                if os.path.isfile(path):
                    r = self._enable_mcp_server(path, SERENA_PROJECT, "serena",
                                                set_trust=needs_trust)
                    if r:
                        why.append("%s: %s" % (label, r))   # ★R4: 사유를 버리지 않는다(잠금 경합·권한 오류가 침묵하던 것)
            gaps = self._serena_activation_gaps()   # 재탐지(멱등 검증)
            if not gaps:
                fixed.append("노드 .claude.json serena 활성화(승인 토큰)")
            elif why:
                gaps = gaps + ["활성화 보류 — " + "; ".join(why)]
        # (e) reviewer(codex) read-only 탐지 — S6 sub-stage (detect-only, never write)
        rev_note = self._serena_reviewer_gap()
        # (f) 거버넌스·격리 탐지 — S4/S8 sub-stage (detect-only WARN)
        gov_note = self._serena_governance_gap()
        suffix = (" · " + "; ".join(fixed)) if fixed else ""
        tail = mcp_note + rev_note + gov_note + suffix
        if mcp_err:
            self.add(cid, WARN, "serena(uvx)%s" % tail)
            return
        if gaps:
            self.add(cid, WARN,
                     "serena 등록됨%s · 미활성: %s — 사람 단계(노드 .claude.json 편집·cys feed push 승인)"
                     % (tail, "; ".join(gaps)))
            return
        self.add(cid, FIXED if fixed else PASS, "serena(uvx) · 등록+활성 OK%s" % tail)

    # ── C44 Serena crossover eval 하베스터 게이트 (S7) ──
    # 분석기 javis_serena_eval.py 의 --self-test(기계 게이트)는 FAIL 할 수 있으나(코드 결함),
    # 루브릭 작성·핀·측정 실행은 사람-게이트(master STEP1 PREP + worker serena 마운트=human-hold)
    # → 미populate/미핀은 WARN-not-FAIL. C43 다음 free id = C44(C34~C42·C43 점유).
    def c44_serena_eval(self):
        cid = "C44.serena-eval"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_serena_eval.py")
        if not p:
            return  # _check_bin_tool 이 이미 FAIL 등록(harness 결함)
        rubric = os.path.join(SERENA_PROJECT, "_round", "SERENA_EVAL_RUBRIC.json")
        if not os.path.isfile(rubric):
            self.add(cid, WARN, "eval harness self-test OK · 루브릭 부재"
                     "(_round/SERENA_EVAL_RUBRIC.json) — 사람단계(master STEP1 PREP)")
            return
        try:
            rb = json.load(open(rubric, encoding="utf-8"))
            tasks = rb.get("tasks", []) or []
            unpinned = [t.get("id") for t in tasks if not t.get("ground_truth_diff_sha")]
        except (OSError, ValueError) as e:
            self.add(cid, WARN, "eval harness self-test OK · 루브릭 파싱 실패: %s" % e)
            return
        if unpinned:
            self.add(cid, WARN,
                     "eval harness self-test OK · 루브릭 미populate(%d task ground_truth_diff_sha=null)·미핀 "
                     "— 측정 선행=worker serena 마운트(human-hold) + master cys attest pin" % len(unpinned))
            return
        self.add(cid, PASS, "eval harness self-test OK · 루브릭 populate·존재(측정은 master LOCKED launcher)")

    # ── C45 semver strictly-newer 비교 도구 (AgentReach PHIL-07 — 신규 *옵션* advisory·WARN-only) ──
    # _check_bin_tool 아님: 그건 부재·self-test 실패를 FAIL로 만든다. javis_semver.py 는 순수
    # advisory(재시작·발행 0행동)이고 즉시 소비자가 적은 신규 opt-in 도구라 boot-blocker가
    # 아니다 → 부재=WARN(C40 패턴 동형). self-test 가 strictly-newer 불변식(반사·반대칭·전이·
    # main-ahead 회귀) 박제가 깨지지 않았나를 결정론으로 잠근다(PHIL-03 도구 *건강* 핀).
    def c45_semver_selftest(self):
        cid = "C45.semver-selftest"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "bin", "javis_semver.py")
        if not os.path.isfile(p):
            self.add(cid, WARN, "javis_semver.py 부재 — strictly-newer 버전 비교 advisory 미설치"
                     "(opt-in·자율주행 ESCALATE 게이트 보완)")
            return
        try:
            r = subprocess.run([sys.executable, p, "--self-test"],
                               capture_output=True, timeout=30, env=_utf8_env())
        except Exception as e:
            self.add(cid, WARN, "javis_semver.py --self-test 실행 불가 — 보류: %s" % e)
            return
        if r.returncode == 0:
            self.add(cid, PASS, "javis_semver.py self-test OK (strictly-newer 불변식 박제·"
                     "main-ahead 거부·fail-safe·무점수·advisory only)")
        else:
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            self.add(cid, WARN, "javis_semver.py self-test 실패(도구 점검 필요) — %s" % tail[-200:])

    # ── C46 bias_check CI 게이트 실배선 (AgentReach OPP-16 GO조건 — 계약 박제 aspirational→실배선) ──
    # bias_check.py(engine No-Site-Name 린터)는 SKILL.md·주석 참조뿐 어디서도 호출 안 됨(grep 0건)
    # 이라 "계약을 테스트로 박제"가 권고에 머물렀다. preflight C-check가 engine 에 대해 직접
    # 호출해 게이트로 강제한다 — PHIL-07 성립의 실배선부. 신규 utf8 린트 *규칙 자체*는 bias_check.py
    # 소관(다른 작업), 본 C46는 preflight가 그 린터를 게이트로 *돌리는* 배선만이다. WARN-first
    # (부재·위반 모두 WARN) — 규칙이 안정화 중이라 boot-blocker로 만들지 않는다(SkillSpector 선례).
    def c46_bias_check(self):
        cid = "C46.bias-check"
        if self.skipped(cid):
            return
        engine = os.path.join(pack_dir(), "skills", "insane-search", "engine")
        bc = os.path.join(engine, "bias_check.py")
        if not os.path.isfile(bc):
            self.add(cid, WARN, "bias_check.py 부재(%s) — No-Site-Name CI 린터 미설치(opt-in)" % bc)
            return
        try:
            # --root = 스킬 루트(engine 의 부모). bias_check 가 engine/·references/ 를 스캔한다.
            skill_root = os.path.dirname(engine)
            r = subprocess.run([sys.executable, bc, "--root", skill_root],
                               capture_output=True, timeout=30, env=_utf8_env())
        except Exception as e:
            self.add(cid, WARN, "bias_check 실행 불가 — 보류: %s" % e)
            return
        if r.returncode == 0:
            self.add(cid, PASS, "bias_check OK (engine No-Site-Name·인코딩 린터 게이트 통과)")
        else:
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            self.add(cid, WARN, "bias_check 위반 검출(규칙 안정화 중 WARN-first) — %s" % tail[-300:])

    # ── C47 URL→자막/전사 단일 채널 글루 (OPP-09 — 존재·자기검증 결정론) ──
    # AGENTREACH OPP-09: transcribe_channel.py 부재면 "URL→자막/전사 단일 채널"이 없어 에이전트가
    # 매번 산문으로 자막/ASR 분기를 재추론(환각 표면)한다. build.rs 가 skills/ 를 자동 walk 임베드하므로
    # PACK 수동 등재는 불요 — 여기선 존재 + --self-test 만 결정론 검증한다(LLM 재추론 금지).
    def c47_transcribe_channel(self):
        cid = "C47.transcribe-channel"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "skills", "transcription", "bin", "transcribe_channel.py")
        if not os.path.isfile(p):
            self.add(cid, WARN, "transcribe_channel.py 부재(%s) — URL→자막/전사 단일 채널 미설치(OPP-09)" % p)
            return
        try:
            r = subprocess.run([sys.executable, p, "--self-test"],
                               capture_output=True, timeout=30, env=_utf8_env())
        except Exception as e:
            self.add(cid, WARN, "transcribe_channel --self-test 실행 불가 — 보류: %s" % e)
            return
        if r.returncode == 0:
            self.add(cid, PASS, "transcribe_channel self-test OK (자막우선·ASR폴백·channel_trace 박제)")
        else:
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            self.add(cid, FAIL, "transcribe_channel --self-test 실패: %s" % tail[-400:])

    # ── C48 콘텐츠 채널 의존성 dormant/absent 디스크 신호 (OPP-10 — 부작용0·결정론) ──
    # AGENTREACH OPP-10: insane-search 우회 의존성(curl_cffi/yt-dlp/playwright)을
    # "잠자는(dormant·디스크 흔적 있음) vs 부재(absent·흔적 없음)"로 부작용0 디스크 신호로
    # 선분류한다. engine/disk_signal.py(stdlib only·네트워크0·import 미실행)를 서브프로세스로
    # 호출 — preflight 계약(표준 라이브러리만·네트워크0) 불변. ABSENT=WARN(우회 의존성은 graceful
    # degrade·선택사항이라 부트 비차단), READY_DORMANT=PASS, UNKNOWN=WARN(런타임 probe 필요).
    # --fix 자동 pip install 금지 — 설치는 CSO 승인·OPP-17 Mutation 게이트 경유.
    def c48_content_channel_deps(self):
        cid = "C48.content-channel-deps"
        if self.skipped(cid):
            return
        engine = os.path.join(pack_dir(), "skills", "insane-search", "engine")
        ds = os.path.join(engine, "disk_signal.py")
        if not os.path.isfile(ds):
            self.add(cid, WARN, "disk_signal.py 부재(%s) — dormant/absent 디스크 신호 미설치(OPP-10)" % ds)
            return
        # stdlib only·네트워크0·import 미실행(find_spec) — preflight 계약 준수.
        driver = (
            "import json,sys; sys.path.insert(0, %r); "
            "import disk_signal as d; print(json.dumps(d.content_dep_signals()))"
            % engine
        )
        try:
            r = subprocess.run([sys.executable, "-c", driver],
                               capture_output=True, timeout=20, env=_utf8_env())
        except Exception as e:
            self.add(cid, WARN, "disk_signal 실행 불가 — 보류: %s" % e)
            return
        if r.returncode != 0:
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            self.add(cid, WARN, "disk_signal 신호 판독 실패(런타임 probe 필요) — %s" % tail[-300:])
            return
        try:
            sigs = json.loads((r.stdout or b"{}").decode("utf-8", "replace"))
        except Exception as e:
            self.add(cid, WARN, "disk_signal 출력 파싱 실패 — %s" % e)
            return
        dormant, absent, unknown = [], [], []
        for dep, sig in sorted(sigs.items()):
            av = (sig or {}).get("avail")
            if av == "ready_dormant":
                dormant.append(dep)
            elif av == "absent":
                absent.append(dep)
            else:
                unknown.append(dep)
        detail = "dormant=%s absent=%s unknown=%s" % (
            ",".join(dormant) or "-", ",".join(absent) or "-", ",".join(unknown) or "-")
        if absent or unknown:
            # 우회 의존성은 선택사항·graceful degrade → FAIL 아닌 WARN(부트 비차단).
            self.add(cid, WARN,
                     "콘텐츠 채널 의존성 일부 미흔적/판독불가(우회 graceful degrade·부트 비차단) — %s "
                     "· 설치는 CSO 승인·OPP-17 게이트 경유(자동 pip install 금지)" % detail)
        else:
            self.add(cid, PASS, "콘텐츠 채널 의존성 전부 dormant(디스크 흔적 존재) — %s" % detail)

    # ── C49 콘텐츠 채널 per-channel 헬스 doctor (AGENTREACH OPP-02) ──
    # javis_channels.py 가 배선됐고 self-test(네트워크0·집계/verdict/permutation/429비종결/tier
    # enum 박제)를 통과하나만 결정론 검증. 실제 채널 타격은 cron(OPP-06 watch)이 담당 —
    # C49 는 부트에서 네트워크 안 침(부트 결정론·속도 보존). coverage_battery 함정 봉인:
    # self-test 는 battery 부재 시 UNKNOWN graceful 을 박제하므로 배포 머신(tests-제외)에서도 통과.
    def c49_channel_health(self):
        cid = "C49.channel-health"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_channels.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (2-pass·tier enum·429비종결·permutation·트랩봉인 박제) "
                     "— 채널 생존은 cron watch 참조(배선만 보증)" % p)

    # ── C50 silence-first 콘텐츠 채널 watch (AGENTREACH OPP-06) ──
    # javis_channel_watch.py 배선·self-test(네트워크0·diff/2-strike/silence-first/snapshot
    # round-trip 박제) 결정론 검증. 채널건강≠노드건강(javis_report 와 별개 층위·중복 회피).
    def c50_channel_watch(self):
        cid = "C50.channel-watch"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_channel_watch.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (2-strike·diff·silence-first·atomic snapshot 박제) "
                     "— cron 미등록은 사람 결정(자율 설치 안 함)" % p)

    # ── C51 클린룸 벤더링 무결성 게이트 (AGENTREACH OPP-19 — NEVER-modify-upstream 자동강제) ──
    # javis_cleanroom.py 의 self-test 가 vendor 변이검증(1바이트 tamper→DRIFTED·삭제→MISSING·
    # 미핀→UNPINNED·snapshot 승인게이트 exit3)을 박제하나만 결정론 검증. 라이브 트리 vendor-check
    # 는 빌드/SOT 머신 소관(REPO_ROOT 부재 시 환각 회피) — C51 은 "도구·자기공격 박제됐나"만 본다.
    def c51_cleanroom_vendor(self):
        cid = "C51.cleanroom-vendor"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_cleanroom.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (벤더링 5상태 분류·1바이트 변이→DRIFTED 자기공격·"
                     "snapshot owner 승인게이트 박제)" % p)

    # ── C52 THIRD_PARTY/NOTICE 라이선스 추적 게이트 (AGENTREACH OPP-20 — AGPL copyleft 오너 결정) ──
    # 동일 javis_cleanroom.py self-test 가 라이선스 변이검증(MIT vendored=ACCEPT·AGPL embed=
    # ESCALATE·unknown SPDX=BLOCK·SPDX 정규화)을 박제하나만 검증. AGPL 은 오너 결정 대상 →
    # copyleft 추적·ESCALATE 큐잉(부트 비차단). C51 과 동일 도구라 self-test 1회로 양쪽 보증.
    def c52_license_gate(self):
        cid = "C52.license-gate"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "bin", "javis_cleanroom.py")
        if not os.path.isfile(p):
            self.add(cid, FAIL, "javis_cleanroom.py 부재 — C51 과 동일 도구(`cys init-pack`)")
            return
        # C51 이 이미 self-test 를 돌렸으므로 여기선 존재만 재확인(중복 subprocess 회피·외과적).
        self.add(cid, PASS, "javis_cleanroom.py license self-test 박제 OK (MIT=ACCEPT·AGPL embed="
                 "ESCALATE·unknown SPDX=BLOCK·정규화) — AGPL copyleft 추적(오너 결정·ESCALATE 큐잉)")

    # ── C53 관찰 명령 부작용 금지 멱등성 봉인 (AGENTREACH OPP-21) ──
    # javis_idempotency.py self-test 가 spy/AST 배터리를 결정론 실행: cmd_check 관찰멱등
    # (calls∩MUTATE=∅ negative assertion)·C12.daemon fix=False Popen 0·coverage_battery
    # 관찰전용(POST/--cookies/yt-dlp 다운로드 토큰 AST 부재)·표면커버리지(cys actions⊆OBSERVE∪MUTATE).
    def c53_idempotency(self):
        cid = "C53.idempotency"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_idempotency.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (관찰 멱등성 봉인 — cmd_check negative assertion·"
                     "C12.daemon fix=False Popen 0·coverage_battery AST 관찰전용·표면커버리지)" % p)

    def _register_nlm_mcp(self, mcp_path):
        return self._register_mcp(mcp_path, "notebooklm-mcp", "notebooklm-mcp")

    # ── C20 NotebookLM SOT 도구 (nlm CLI + MCP 등록 + 인증) ──
    # 자동화 경계(제품 기본 절차): 설치·MCP 등록은 기계가 수행(--fix),
    # Google 로그인은 사람 전용 단계 — "빠진 것을 기계가 알려주는" 수준으로
    # 정확한 명령을 안내한다(부트 비차단 WARN).
    def c20_nlm_sot(self):
        cid = "C20.nlm-sot"
        if self.skipped(cid):
            return
        nlm, ver = self._nlm_version()
        fixed = []
        # (a) 설치·버전 하한
        if nlm is None or ver is None or ver < NLM_MIN_VERSION:
            cur = ".".join(map(str, ver)) if ver else "미설치/판독불가"
            if self.fix and self._install_nlm():
                nlm, ver = self._nlm_version()
            if nlm and ver and ver >= NLM_MIN_VERSION:
                fixed.append("nlm %s 설치(핀)" % ".".join(map(str, ver)))
            else:
                self.add(cid, FAIL,
                         "nlm %s — SOT 도구 미비. --fix(uv/pipx/pip 자동 설치) 또는 "
                         "`uv tool install '%s'`" % (cur, NLM_PIN))
                return
        # (b) MCP 등록 (git 루트에서만 — C13과 동일 스코프. worktree는 .git이 파일)
        mcp_note = ""
        mcp_err = False
        if os.path.exists(".git"):
            registered = self._mcp_registered(".mcp.json", "notebooklm-mcp")
            if not registered:
                if self.fix:
                    err = self._register_nlm_mcp(".mcp.json")
                    if err:
                        mcp_note = " · MCP 등록 실패: %s" % err
                        mcp_err = True
                    else:
                        fixed.append("./.mcp.json에 notebooklm-mcp 등록")
                else:
                    mcp_note = " · ./.mcp.json MCP 미등록(--fix로 등록 가능)"
        # (c) 인증 — 사람 전용 단계: 기계는 상태와 다음 명령만 정확히 알린다
        auth_ok = False
        try:
            auth_ok = subprocess.run([nlm, "login", "--check"], capture_output=True,
                                     timeout=45).returncode == 0
        except Exception:
            pass
        ver_s = ".".join(map(str, ver))
        suffix = (" · " + "; ".join(fixed)) if fixed else ""
        if not auth_ok:
            self.add(cid, WARN,
                     "nlm %s 설치됨%s · Google 미인증 — 사람 단계: `nlm login` 실행 필요%s"
                     % (ver_s, mcp_note, suffix))
            return
        if mcp_err:
            # 등록 실패를 PASS 본문에 접어 넣으면 READY가 MCP 계층 파손을 가린다.
            self.add(cid, WARN, "nlm %s · 인증 OK%s%s" % (ver_s, mcp_note, suffix))
            return
        self.add(cid, FIXED if fixed else PASS,
                 "nlm %s · 인증 OK%s%s" % (ver_s, mcp_note, suffix))

    # ── C21 Harness Creator 툴체인 (1st-party 메타스킬의 도구 본체) ──
    # 스킬은 pack 임베드로 자동 배포 — 이 검사는 스킬이 호출하는 TOOLS_ROOT의 존재를
    # 결정론 검증하고, 신규 머신에서는 --fix가 핀 커밋을 자동 클론한다.
    @staticmethod
    def _harness_root():
        cands = []
        env = os.environ.get("CYS_HARNESS_HOME", "")
        if env:
            cands.append(env)
        home = os.path.expanduser("~")
        cands.append(os.path.join(home, ".cys/harness-creator"))
        cands.append(os.path.join(home, "Desktop/CYSjavis/cys-harness-creator"))
        for d in cands:
            if all(os.path.isfile(os.path.join(d, f)) for f in HARNESS_KEY_FILES):
                return d
        return None

    def c21_harness_creator(self):
        cid = "C21.harness-creator"
        if self.skipped(cid):
            return
        root = self._harness_root()
        if root:
            self.add(cid, PASS, "TOOLS_ROOT=%s (핵심 도구 %d종 존재)"
                     % (root, len(HARNESS_KEY_FILES)))
            return
        dst = os.path.join(os.path.expanduser("~"), ".cys/harness-creator")
        if self.mode in ("dry", "safe"):
            # OPP-17: git clone 은 external_install(전역 디렉터리 신설·사실상 비가역) → 미리보기/무변경.
            self.may_mutate(cid, "subprocess_install", "git clone %s → %s" % (HARNESS_REPO, dst),
                            "harness-creator 툴체인 git clone(핀 %s)" % HARNESS_PIN[:8],
                            denylist_class="external_install")
            return
        if self.fix and shutil.which("git") and self.may_mutate(
                cid, "subprocess_install", "git clone %s → %s" % (HARNESS_REPO, dst),
                "harness-creator 툴체인 git clone(핀 %s)" % HARNESS_PIN[:8],
                denylist_class="external_install"):
            try:
                ok = subprocess.run(["git", "clone", HARNESS_REPO, dst],
                                    capture_output=True, timeout=300).returncode == 0
                if ok:
                    # 핀은 검증돼야 핀이다 — checkout rc와 HEAD==핀을 기계 확인하지
                    # 않으면 핀 부재(force-push·레포 교체) 시 조용히 moving HEAD로
                    # 남아 FIXED가 거짓 핀 주장이 된다(공급망 표면).
                    co = subprocess.run(["git", "-C", dst, "checkout", HARNESS_PIN],
                                        capture_output=True, timeout=60).returncode
                    head = subprocess.run(
                        ["git", "-C", dst, "rev-parse", "HEAD"],
                        capture_output=True, timeout=15).stdout.decode().strip()
                    ok = co == 0 and head == HARNESS_PIN
            except Exception:
                ok = False
            if ok and self._harness_root():
                self.add(cid, FIXED, "%s 클론(핀 %s 검증)" % (dst, HARNESS_PIN[:8]))
                return
        dirty = " (기존 %s 불완전 — 제거 후 재시도 필요)" % dst if os.path.isdir(dst) else ""
        self.add(cid, FAIL,
                 "harness-creator 툴체인 미설치%s — --fix(git 자동 클론) 또는 "
                 "`git clone %s %s && git -C %s checkout %s`"
                 % (dirty, HARNESS_REPO, dst, dst, HARNESS_PIN[:8]))

    # ── C22 work management 스킬 2종 (앵커5-4b·c — 환각방지·의도 합의) ──
    # 절대 강조 4규칙의 b(hallucination-guard)·c(grill-me)가 가리키는 전담 sub-skill이
    # 실재해야 지침이 공수표가 되지 않는다. 누락 시 init-pack 임베드로 수리한다.
    def _skill_indexable(self, name):
        p = os.path.join(pack_dir(), "skills", name, "SKILL.md")
        if not (os.path.isfile(p) and os.path.getsize(p) > 0):
            return False
        # 실파서(cys.rs compose_directive)는 read_to_string이라 전 파일 UTF-8 유효 +
        # name: 값 비어있지 않음을 요구한다 — 동일 규칙로 판정(거짓 PASS 차단).
        # 줄 분리도 rust str::lines와 동일하게 \n 기준(bare-CR 파일 parity — splitlines 금지).
        try:
            head = open(p, encoding="utf-8", newline="").read().split("\n")[:10]
        except (OSError, UnicodeDecodeError):
            return False
        # rust는 첫 10줄에서 마지막 name: 이 이긴다(덮어쓰기 루프) — first-match로
        # 판정하면 'name: foo' 뒤 빈 'name:'이 있는 파일을 rust는 떨구는데 여기는
        # 통과시키는 거짓 PASS가 난다(parity 위반).
        name = None
        for ln in head:
            if ln.startswith("name:"):
                name = ln[5:].strip()
        return bool(name)

    def _work_skill_problem(self, name):
        """None=건전, 문자열=결함 사유. 색인성 + 본문 핀(전담 기능 실재)을 함께 판정."""
        if not self._skill_indexable(name):
            return "%s(누락/색인 불가)" % name
        p = os.path.join(pack_dir(), "skills", name, "SKILL.md")
        try:
            text = open(p, encoding="utf-8").read()
        except (OSError, UnicodeDecodeError):
            return "%s(읽기 실패)" % name
        lost = [pin for pin in WORK_SKILL_PINS.get(name, []) if pin not in text]
        if lost:
            return "%s(본문 핀 소실: %s)" % (name, "·".join(lost))
        return None

    def c22_work_skills(self):
        cid = "C22.work-skills"
        if self.skipped(cid):
            return
        problems = [pr for s in WORK_SKILLS if (pr := self._work_skill_problem(s))]
        repaired = []
        if problems and self.fix and self.repair_via_init_pack():
            still = [pr for s in WORK_SKILLS if (pr := self._work_skill_problem(s))]
            repaired = [pr for pr in problems if pr not in still]
            problems = still
        if problems:
            self.add(cid, FAIL,
                     "work 스킬 결함: %s — `cys init-pack` 또는 --fix"
                     "(파일이 존재하되 깨진/약화된 경우 init-pack은 보존한다 — "
                     "강제 복원 플래그는 사용자 수정을 전량 vendor 본으로 되돌리는 광범위 조작이라(<rel>.user 백업은 남는다) "
                     "쓰지 않는다. 백업 선행 + 주인님 보고 후 복구 · 운영계약 §11-13)"
                     % "; ".join(problems))
        elif repaired:
            self.add(cid, FIXED, "init-pack 수리 완료: %s" % "; ".join(repaired))
        else:
            self.add(cid, PASS,
                     "work management 스킬 2종(%s) 존재·색인 가능·본문 핀 건재"
                     % ", ".join(WORK_SKILLS))

    # ── C23 거버넌스 충돌 감시 (외부 에이전트 운영체계 동거 감지) ──
    # 사용자가 나중에 gstack류를 추가 설치해도 "아무도 모르는" 상황을 차단한다 —
    # 부트마다 결정론 감지 → WARN + 격리 수칙 안내 (금지·자동 제거 없음: 설치는 오너 주권).
    def c23_governance_conflict(self):
        cid = "C23.governance-conflict"
        if self.skipped(cid):
            return
        findings = []
        for settings_path in discover_claude_settings():
            profile = os.path.dirname(settings_path)
            # 충돌 조건 = cysjavis 배선 프로필(우리 hook 등록)과의 '동거'만
            if not self._hook_registered(settings_path):
                continue
            for name, sig in FOREIGN_AGENT_OS.items():
                signals = []
                if os.path.isdir(os.path.join(profile, "skills", sig["skills_dir"])):
                    signals.append("skills/%s 설치" % sig["skills_dir"])
                cmd_path = os.path.join(profile, "CLAUDE.md")
                if os.path.isfile(cmd_path):
                    try:
                        text = open(cmd_path, encoding="utf-8", errors="replace").read()
                        hits = [m for m in sig["claude_md_markers"] if m in text]
                        if hits:
                            signals.append("CLAUDE.md 점유 마커(%s)" % ", ".join(hits[:2]))
                    except OSError:
                        pass
                try:
                    stext = open(settings_path, encoding="utf-8", errors="replace").read()
                    if sig["hook_marker"] in stext:
                        signals.append("hook 등록")
                except OSError:
                    pass
                if signals:
                    findings.append("%s@%s: %s — %s"
                                    % (name, profile, "; ".join(signals), sig["guide"]))
        if findings:
            self.add(cid, WARN, "외부 운영체계 동거 감지 — " + " | ".join(findings))
        else:
            self.add(cid, PASS, "cysjavis 배선 프로필에 외부 운영체계 점유 신호 없음")

    # ── C24 한국 법령 전용 MCP (korean-law-mcp — k-skill law 프록시 경로 대체) ──
    # 자동화 경계는 C20과 동일: 설치·MCP 등록은 기계(--fix), 법제처 OC 키 발급만
    # 사람 단계로 정확히 안내한다(부트 비차단 WARN).
    @staticmethod
    def _klaw_version():
        """(cli경로|None, 버전튜플|None) — 설치·재설치 후 동일 경로로 재탐침한다."""
        cli = shutil.which("korean-law")
        if cli is None:
            return None, None
        try:
            out = subprocess.run([cli, "--version"], capture_output=True,
                                 timeout=15).stdout.decode("utf-8", "replace")
            m = re.search(r"(\d+)\.(\d+)\.(\d+)", out)
            return cli, (tuple(int(x) for x in m.groups()) if m else None)
        except Exception:
            return cli, None

    def c24_korean_law_mcp(self):
        cid = "C24.korean-law-mcp"
        if self.skipped(cid):
            return
        fixed = []
        cli, ver = self._klaw_version()
        # 버전 게이트는 C20과 동형으로 빈틈없이(else-망라) — ver 판독불가가
        # FAIL 없이 통과하던 무성 폴스루를 차단하고, 설치 후 버전을 재탐침한다.
        if cli is None or ver is None or ver < KLAW_MIN_VERSION:
            cur = ".".join(map(str, ver)) if ver else "미설치/판독불가"
            if self.mode in ("dry", "safe"):
                # OPP-17: npm install -g 은 external_install(전역 환경 변경·사실상 비가역) → 미리보기/무변경.
                self.may_mutate(cid, "subprocess_install", "npm install -g %s" % KLAW_PIN,
                                "korean-law MCP CLI 전역 설치(핀 %s)" % KLAW_PIN,
                                denylist_class="external_install")
                return
            if self.fix and shutil.which("npm") and self.may_mutate(
                    cid, "subprocess_install", "npm install -g %s" % KLAW_PIN,
                    "korean-law MCP CLI 전역 설치(핀 %s)" % KLAW_PIN,
                    denylist_class="external_install"):
                try:
                    if subprocess.run(["npm", "install", "-g", KLAW_PIN],
                                      capture_output=True, timeout=600).returncode == 0:
                        cli, ver = self._klaw_version()
                except Exception:
                    pass
            if cli and ver and ver >= KLAW_MIN_VERSION:
                fixed.append("%s 설치(핀)" % KLAW_PIN)
            else:
                self.add(cid, FAIL,
                         "korean-law %s — 법령 MCP 미비. --fix(npm 자동 설치) 또는 "
                         "`npm install -g %s`" % (cur, KLAW_PIN))
                return
        # 키 — 사람 전용 단계 (등록 전에 판정: 발견된 변수명을 등록에 그대로 쓴다)
        key_var = next((v for v in ("LAW_OC", "LAW_OC_ID") if os.environ.get(v)), None)
        # MCP 등록 (git 루트 — C20과 동일 스코프·worktree는 .git이 파일.
        #  ${변수}는 Claude Code가 세션 env에서 전개)
        mcp_note = ""
        mcp_err = False
        if os.path.exists(".git"):
            if not self._mcp_registered(".mcp.json", "korean-law-mcp"):
                if self.fix:
                    err = self._register_mcp(
                        ".mcp.json", "korean-law-mcp", "korean-law-mcp",
                        env={"LAW_OC": "${%s}" % (key_var or "LAW_OC")})
                    if err:
                        mcp_note = " · MCP 등록 실패: %s" % err
                        mcp_err = True
                    else:
                        fixed.append("./.mcp.json에 korean-law-mcp 등록")
                else:
                    mcp_note = " · ./.mcp.json MCP 미등록(--fix로 등록 가능)"
        suffix = (" · " + "; ".join(fixed)) if fixed else ""
        if not key_var:
            hint = "사람 단계: open.law.go.kr 가입·OC 발급 후 `export LAW_OC=<키>`"
            for rc in ("~/.zshrc", "~/.zshenv"):
                p = os.path.expanduser(rc)
                try:
                    if os.path.isfile(p) and "LAW_OC" in open(p, encoding="utf-8",
                                                              errors="replace").read():
                        hint = "%s에 키 라인 존재 — 현 프로세스 미로드(셸 재기동 필요)" % rc
                        break
                except OSError:
                    pass
            self.add(cid, WARN, "korean-law 설치됨%s · OC 키 미설정 — %s%s"
                     % (mcp_note, hint, suffix))
            return
        if mcp_err:
            self.add(cid, WARN, "korean-law-mcp · OC 키 확인%s%s" % (mcp_note, suffix))
            return
        self.add(cid, FIXED if fixed else PASS,
                 "korean-law-mcp · OC 키 확인%s%s" % (mcp_note, suffix))

    # ── C25 자율주행 메모리 상주 (앵커6 — 🔒색인 상주 필수) ──
    # feedback_autonomous-pilot-mandate.md가 파일로 존재하고 MEMORY.md 색인에 등재돼야
    # 모든 노드 기동 시 자율주행 권한·경계가 자동 주입된다(빠지면 매 단계 수동개입 대기로
    # 자율주행 무력화). 파일은 init-pack 임베드로, 색인 줄은 lock 하에 결정론 append로 수리.
    # ★실행 순서: C18(memory verify)보다 먼저 돌아야 같은 런에서 수리→정합 순이 된다.
    @staticmethod
    def _index_registered(itext):
        """색인 등재 판정 — javis_memory verify의 index_links와 동일 기준(링크 타깃,
        HTML 주석·코드펜스 제외). raw substring은 산문 언급을 등재로 오판한다(6차 R1)."""
        visible = re.sub(r"```.*?```", "", re.sub(r"<!--.*?-->", "", itext, flags=re.S),
                         flags=re.S)
        return ("](%s)" % AUTOPILOT_MEMORY_FILE) in visible

    def c25_autopilot_memory(self):
        cid = "C25.autopilot-memory"
        if self.skipped(cid):
            return
        mdir = os.path.join(pack_dir(), "memory")
        fpath = os.path.join(mdir, AUTOPILOT_MEMORY_FILE)
        idx = os.path.join(mdir, "MEMORY.md")
        fixed = []
        if not os.path.isfile(fpath):
            if self.fix and self.repair_via_init_pack() and os.path.isfile(fpath):
                fixed.append("메모리 파일 재설치")
            else:
                self.add(cid, FAIL, "memory/%s 없음 — `cys init-pack` 또는 --fix"
                         % AUTOPILOT_MEMORY_FILE)
                return
        # 본문 핀: 권한·경계의 실질이 비워지면(frontmatter만 잔존) 상주가 공수표다.
        try:
            ftext = open(fpath, encoding="utf-8", errors="replace").read()
        except OSError:
            self.add(cid, FAIL, "memory/%s 읽기 불가" % AUTOPILOT_MEMORY_FILE)
            return
        lost = [pin for pin in AUTOPILOT_MEMORY_PINS if pin not in ftext]
        if lost:
            self.add(cid, FAIL, "자율주행 메모리 본문 핀 소실: %s — ★복원 절차"
                     "(운영계약 §9-7-6·§11-13): ①먼저 백업 ②주인님께 보고 후 지시에 따라 복구. "
                     "팩 템플릿 강제 복원은 무백업 소실을 낳는 비가역 조작이라 절대 금지다."
                     % "·".join(lost))
            return
        try:
            itext = open(idx, encoding="utf-8", errors="replace").read()
        except OSError:
            self.add(cid, FAIL, "memory/MEMORY.md 읽기 불가 (C01 먼저)")
            return
        if not self._index_registered(itext):
            if not self.fix:
                self.add(cid, FAIL,
                         "MEMORY.md 색인에 자율주행 메모리 미등재(🔒상주 필수) — --fix로 등재 가능")
                return
            # javis_memory와 동일한 lock 규약(O_CREAT|O_EXCL + stale 회수)으로 색인 1줄
            # append — 결정론 도구의 기계 등재이지 LLM 손편집이 아니다.
            lock = idx + ".lock"
            acquired = False
            for _ in range(2):
                try:
                    fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                    acquired = True
                    break
                except FileExistsError:
                    try:  # 죽은 프로세스의 만료 잠금(30초+)은 회수한다 — javis_memory와 동일
                        if time.time() - os.path.getmtime(lock) > 30:
                            os.unlink(lock)
                            continue
                    except OSError:
                        pass
                    break
            if not acquired:
                self.add(cid, FAIL, "MEMORY.md 잠금 경합(활성 lock) — 잠시 후 재실행. "
                         "30초+ 방치된 %s는 자동 회수된다" % lock)
                return
            try:
                with open(idx, "a", encoding="utf-8") as f:
                    if not itext.endswith("\n"):
                        f.write("\n")
                    f.write(AUTOPILOT_MEMORY_INDEX_LINE + "\n")
            finally:
                os.close(fd)
                os.unlink(lock)
            fixed.append("색인 등재(lock append)")
        if fixed:
            self.add(cid, FIXED, "자율주행 메모리 상주 수리: %s" % ", ".join(fixed))
        else:
            self.add(cid, PASS, "자율주행 메모리 파일 존재·본문 핀 건재·색인 등재(🔒상주)")

    # ── C26 영상 자동제작 스킬(cys-video-creator) 기본 탑재 ──
    # 비차단(영상 제작은 옵트인 능력) — 절대 FAIL 없음. 핵심(우리 스킬을 프로필에 심링크)은
    # 결정론 FIXED, 런타임 전제(도구·벤더스킬·키)는 WARN(정보). 자동화 경계는 C20/C24와 동일:
    # 우리 스킬 배선·벤더 스킬 설치는 기계(--fix), API 키 발급은 사람 단계.
    def c26_video_creator(self):
        cid = "C26.video-creator"
        if self.skipped(cid):
            return
        fixed, warns = [], []
        # (a) 우리 스킬이 pack에 임베드·설치됐는지(데몬 install 산출물) 확인
        missing = [s for s in VIDEO_SKILLS
                   if not os.path.isfile(os.path.join(pack_dir(), "skills", s, "SKILL.md"))]
        if missing:
            warns.append("pack 스킬 %d종 미설치(%s…) — init-pack 재실행 필요"
                         % (len(missing), missing[0]))
        # (b) 네이티브 Claude Code(/goal) 발견용 프로필 심링크 (기계 --fix)
        _home = os.path.expanduser("~")
        profiles = sorted(os.path.join(_home, d) for d in os.listdir(_home)
                          if (d == ".claude" or d.startswith(".claude-"))
                          and os.path.isdir(os.path.join(_home, d)))
        linked_profiles = 0
        for prof in profiles:
            sdir = os.path.join(prof, "skills")
            need = [s for s in VIDEO_SKILLS if s not in missing
                    and not self._symlink_ok(os.path.join(sdir, s),
                                             os.path.join(pack_dir(), "skills", s))]
            if not need:
                linked_profiles += 1
                continue
            if self.fix:
                try:
                    os.makedirs(sdir, exist_ok=True)
                    for s in need:
                        link = os.path.join(sdir, s)
                        target = os.path.join(pack_dir(), "skills", s)
                        if os.path.islink(link) or os.path.exists(link):
                            if os.path.islink(link):
                                os.unlink(link)
                            else:
                                continue  # 실디렉(사용자 보유) — 덮지 않음
                        os.symlink(target, link)
                    linked_profiles += 1
                    fixed.append("%s/skills ← 영상 스킬 심링크" % os.path.basename(prof))
                except OSError as e:
                    warns.append("%s 심링크 실패: %s" % (os.path.basename(prof), e))
            else:
                warns.append("%s/skills 영상 스킬 미배선(--fix로 심링크)" % os.path.basename(prof))
        # (c) 도구 — Node 22+·FFmpeg (WARN만, 영상 제작 시 필요)
        node_major = self._node_major()
        if node_major is None or node_major < 22:
            warns.append("Node 22+ 필요(HyperFrames 렌더) — 현재 %s"
                         % (node_major or "미설치"))
        if not shutil.which("ffmpeg"):
            warns.append("FFmpeg 미설치(HyperFrames·합성 필요)")
        # (d) 공식 벤더 스킬 — `npx skills add`는 cwd의 .agents/skills/에 프로젝트-로컬 설치라
        # 자동 실행하지 않는다(엉뚱한 cwd 오염 방지). 영상 작업 폴더에서 1회 실행 안내.
        warns.append("벤더 스킬은 영상 작업 폴더에서 1회: " + " · ".join(VIDEO_VENDOR_COMMANDS))
        # (e) 런타임 키 — 사람 단계(WARN 비차단)
        miss_keys = [k for k in VIDEO_RUNTIME_KEYS if not os.environ.get(k)]
        if miss_keys:
            warns.append("API 키 미설정: %s — 사람 단계(`export <KEY>=...`), 영상 제작 시 필요"
                         % ", ".join(miss_keys))
        # 판정: WARN 있으면 WARN(비차단), 없으면 PASS/FIXED
        detail = "영상 스킬 %d종 · 프로필 %d/%d 배선" % (
            len(VIDEO_SKILLS) - len(missing), linked_profiles, len(profiles) or 0)
        if fixed:
            detail += " · " + "; ".join(fixed)
        if warns:
            self.add(cid, WARN, detail + " · 전제: " + " | ".join(warns))
        else:
            self.add(cid, FIXED if fixed else PASS, detail)

    @staticmethod
    def _symlink_ok(link, target):
        return os.path.islink(link) and os.path.realpath(link) == os.path.realpath(target)

    @staticmethod
    def _node_major():
        node = shutil.which("node")
        if not node:
            return None
        try:
            out = subprocess.run([node, "-v"], capture_output=True,
                                 timeout=15).stdout.decode("utf-8", "replace")
            m = re.search(r"v(\d+)\.", out)
            return int(m.group(1)) if m else None
        except Exception:
            return None

    # ── C27 appbuild 웹/앱 빌드 스킬 + 코드선행 금지 hook (워커 필수) ──
    # 비차단(빌드는 옵트인)이되, 핵심은 결정론으로: 우리 20종을 프로필 심링크 + 게이트 hook을
    # PreToolUse로 등록(hook은 .appbuild 밖에선 fail-open이라 무관 작업 불간섭). 도구·키 불요
    # (cysjavis 자체 엔진 사용). FAIL 없음.
    @staticmethod
    def _appbuild_hook_registered(settings_path):
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        if not isinstance(data, dict):
            return False
        desired = _cys_hook_cmd(APPBUILD_HOOK)
        for entry in data.get("hooks", {}).get("PreToolUse", []):
            for h in entry.get("hooks", []):
                if h.get("command", "") == desired:
                    return True
        return False

    def _register_appbuild_hook(self, settings_path):
        """PreToolUse(Edit|Write|NotebookEdit)로 게이트 hook 등록. 성공=None, 실패=사유."""
        cmd = _cys_hook_cmd(APPBUILD_HOOK)

        def _mutate(data):
            arr = data.setdefault("hooks", {}).setdefault("PreToolUse", [])
            kept, have = _prune_stale_hook_entries(arr, APPBUILD_HOOK, cmd)
            if not have:
                kept.append({"matcher": "Edit|Write|NotebookEdit",
                             "hooks": [{"type": "command", "command": cmd}]})
            arr[:] = kept

        return _settings_rmw(settings_path, _mutate)   # G16: 락+mkstemp 단일 소유자

    # ── C28 자기교정·영속성 hook 등록 헬퍼 (event 일반화) ──
    @staticmethod
    def _event_hook_registered(settings_path, event, script_name, declared_timeout=None):
        """event 에 pack 경로의 script_name 이 **선언 timeout 을 충족한 채** 등록돼 있나
        (구 .config 경로는 미인정). 판정 = `command 동등 ∧ 선언 timeout 충족`(U-21).
        `declared_timeout=None`(기본) 이면 종전 판정 그대로 — command 축 단독이다."""
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        if not isinstance(data, dict):
            return False
        desired = _cys_hook_cmd(script_name)
        for entry in data.get("hooks", {}).get(event, []):
            for h in entry.get("hooks", []):
                if h.get("command", "") == desired and hook_timeout_satisfied(
                        h.get("timeout"), declared_timeout):
                    return True
        return False

    def _register_event_hook(self, settings_path, event, script_name, matcher=None,
                             timeout=None):
        """event 에 pack/hooks/script_name 등록. 성공=None, 실패=사유. 멱등은 호출부.
        _register_appbuild_hook 과 동일 규약(symlink 거부·파싱실패 거부·백업·원자적).

        ★U-21 **불일치 엔트리 교체 경로**: 명령은 이미 있고 **선언 timeout 만 미달**이면 append 가
        아니라 그 hook 객체의 `timeout` 하나만 올린다. append 로 처리하면 같은 명령이 두 번 실려
        **매 프롬프트마다 훅이 2회 발화**한다(큐 폭주 방향). 그리고 `max(기존, 선언)` 이라
        사용자가 더 크게 잡아둔 값은 **내리지 않는다**(오살 금지 — 한 방향으로만 여는 축)."""
        cmd = _cys_hook_cmd(script_name)

        def _mutate(data):
            arr = data.setdefault("hooks", {}).setdefault(event, [])
            kept, have = _prune_stale_hook_entries(arr, script_name, cmd)
            if have and timeout is not None:
                # 교체 경로 — 우리 명령과 **바이트 동등**인 hook 객체의 timeout 키 하나만 손댄다.
                for entry in kept:
                    if not isinstance(entry, dict):
                        continue
                    for h in entry.get("hooks", []):
                        if not isinstance(h, dict) or h.get("command", "") != cmd:
                            continue
                        cur = h.get("timeout")
                        if not isinstance(cur, (int, float)) or isinstance(cur, bool):
                            cur = None
                        h["timeout"] = timeout if cur is None else max(cur, timeout)
            if not have:
                entry = {"hooks": [{"type": "command", "command": cmd}]}
                if timeout is not None:
                    entry["hooks"][0]["timeout"] = timeout
                if matcher is not None:
                    entry["matcher"] = matcher
                kept.append(entry)
            arr[:] = kept

        return _settings_rmw(settings_path, _mutate)   # G16: 락+mkstemp 단일 소유자

    def c27_appbuild(self):
        cid = "C27.appbuild"
        if self.skipped(cid):
            return
        fixed, warns = [], []
        # (a) 우리 스킬이 pack에 설치됐는지
        missing = [s for s in APPBUILD_SKILLS
                   if not os.path.isfile(os.path.join(pack_dir(), "skills", s, "SKILL.md"))]
        if missing:
            warns.append("pack 스킬 %d종 미설치(%s…) — init-pack 재실행"
                         % (len(missing), missing[0]))
        # (b) 프로필 심링크 (네이티브/goal 발견 — C26과 동일 규약)
        _home = os.path.expanduser("~")
        profiles = sorted(os.path.join(_home, d) for d in os.listdir(_home)
                          if (d == ".claude" or d.startswith(".claude-"))
                          and os.path.isdir(os.path.join(_home, d)))
        linked = 0
        for prof in profiles:
            sdir = os.path.join(prof, "skills")
            need = [s for s in APPBUILD_SKILLS if s not in missing
                    and not self._symlink_ok(os.path.join(sdir, s),
                                             os.path.join(pack_dir(), "skills", s))]
            if not need:
                linked += 1
                continue
            if self.fix:
                try:
                    os.makedirs(sdir, exist_ok=True)
                    for s in need:
                        link = os.path.join(sdir, s)
                        if os.path.islink(link):
                            os.unlink(link)
                        elif os.path.exists(link):
                            continue
                        os.symlink(os.path.join(pack_dir(), "skills", s), link)
                    linked += 1
                    fixed.append("%s/skills ← appbuild 심링크" % os.path.basename(prof))
                except OSError as e:
                    warns.append("%s 심링크 실패: %s" % (os.path.basename(prof), e))
            else:
                warns.append("%s/skills appbuild 미배선(--fix)" % os.path.basename(prof))
        # (c) 게이트 hook 존재·실행권한
        hook_path = os.path.join(pack_dir(), "hooks", APPBUILD_HOOK)
        if not os.path.isfile(hook_path):
            warns.append("게이트 hook 미설치 — init-pack 재실행")
        elif os.name == "posix":
            mode = os.stat(hook_path).st_mode
            if not mode & stat.S_IXUSR and self.fix:
                os.chmod(hook_path, mode | 0o755)
                fixed.append("게이트 hook 실행권한")
        # (d) PreToolUse 게이트 hook 등록 (결정론 — .appbuild 밖 fail-open이라 안전)
        if os.path.isfile(hook_path):
            targets, forbidden = resolve_registration_targets()   # ★G1 sentinel(폴백 소유자)
            if forbidden and not targets:
                warns.append("등록 대상 없음 — %s" % forbidden)
                targets = []
            reg = 0
            for t in targets:
                if self._appbuild_hook_registered(t):
                    reg += 1
                    continue
                if self.fix:
                    err = self._register_appbuild_hook(t)
                    if err:
                        warns.append("hook 등록 실패(%s): %s" % (os.path.basename(t), err))
                    else:
                        reg += 1
                        fixed.append("%s에 게이트 hook 등록" % os.path.basename(t))
                else:
                    warns.append("게이트 hook 미등록(--fix)")
        # 판정 (FAIL 없음)
        detail = "appbuild 스킬 %d종 · 프로필 %d/%d 배선 · 코드선행 금지 hook" % (
            len(APPBUILD_SKILLS) - len(missing), linked, len(profiles) or 0)
        if fixed:
            detail += " · " + "; ".join(fixed)
        if warns:
            self.add(cid, WARN, detail + " · " + " | ".join(warns))
        else:
            self.add(cid, FIXED if fixed else PASS, detail)

    def c28_self_correction(self):
        cid = "C28.self-correction"
        if self.skipped(cid):
            return
        # ★`fails` 를 여기서 만든다 — (a) 실재 검사에서도 **각성 티어 FAIL** 이 나올 수 있다
        #   (훅 본체 부재). 종전엔 (b) 등록 루프 직전에 만들어 (a)는 WARN 밖에 못 냈다.
        fixed, warns, fails = [], [], []
        # (a) hook 스크립트 4종 + javis_reflect.py 존재·실행권한
        rels = [os.path.join("hooks", s) for s, _ in SELFCORR_HOOKS]
        rels.append(os.path.join("bin", "javis_reflect.py"))
        for rel in rels:
            p = os.path.join(pack_dir(), rel)
            if not os.path.isfile(p):
                if self.fix and self.repair_via_init_pack() and os.path.isfile(p):
                    pass
                else:
                    warns.append("%s 미설치 — init-pack 재실행" % rel)
                    continue
            if os.name == "posix":
                mode = os.stat(p).st_mode
                if not mode & stat.S_IXUSR and self.fix:
                    os.chmod(p, mode | 0o755)
                    fixed.append("%s 실행권한" % os.path.basename(p))
        # (a-2) ★훅 **본체** 실재(부트 v2 A2 · 등록 대상 아님 — 위 HOOK_BODY_FILES 주석 참조)
        for _rel, _owner in HOOK_BODY_FILES:
            _p = os.path.join(pack_dir(), _rel)
            if not os.path.isfile(_p):
                if self.fix and self.repair_via_init_pack() and os.path.isfile(_p):
                    pass
                else:
                    (fails if _owner in AWAKENING_SCRIPTS else warns).append(
                        "%s 부재 — 훅 본체가 없으면 `%s`(런처)는 고지 1줄만 내고 **부트를 발화하지 "
                        "않는다**(훅 자체는 exit 0 이라 무관측이었다). init-pack 재실행·pack-update "
                        "로 복구하라" % (_rel, _owner))
                    continue
            if os.name == "posix":
                _mode = os.stat(_p).st_mode
                if not _mode & stat.S_IXUSR and self.fix:
                    os.chmod(_p, _mode | 0o755)
                    fixed.append("%s 실행권한" % os.path.basename(_p))

        # (b) 이벤트별 등록 (멱등 — 구 .config 경로는 미인정이라 패키지 경로로 신규 등록)
        # ★G1 sentinel: 격리(부서/임시) 팩은 글로벌 폴백 없이 등록 0 — 폴백은 resolve 가 소유한다.
        targets, forbidden = resolve_registration_targets()
        if forbidden and not targets:
            # ★"등록 대상이 없다"는 사실이 **파일 실재 사실을 지우지 않는다** — 두 축은 별개다.
            #   종전엔 여기서 즉시 SKIP 해 (a)·(a-2)가 이미 찾은 결손(훅 스크립트·reflect 엔진·
            #   훅 **본체** 부재)이 통째로 버려졌다. 격리 팩·부서 팩처럼 등록이 금지된 컨텍스트가
            #   바로 팩 복제 결손이 실제로 발생하는 곳인데, 거기서 preflight 가 SKIP(초록에 가까움)
            #   이었다 — 관측이 가장 필요한 자리에서 관측이 꺼져 있었다.
            _note = "등록 대상 없음 — %s" % forbidden
            _base = "자기교정·영속성 hook 파일 실재(등록은 이 컨텍스트에서 금지)"
            if fails:
                self.add(cid, FAIL,
                         _base + " · ★각성 훅 본체/스크립트 결손(부트 발화 불가 — C08 대칭 FAIL): "
                         + " | ".join(fails[:6])
                         + (" · 기타: " + " | ".join(warns[:3]) if warns else "")
                         + " · " + _note)
            elif warns:
                self.add(cid, WARN, _base + " · " + " | ".join(warns[:6]) + " · " + _note)
            elif fixed:
                self.add(cid, FIXED, _base + " · " + "; ".join(fixed[:6]) + " · " + _note)
            else:
                self.add(cid, SKIP, _note)
            return
        # ★A21(W3) 훅별 중요도 티어: **각성 훅**(role-bootstrap→UserPromptSubmit) 미등록은
        #   C08(session-start)과 **대칭으로 FAIL** 이다. 종전엔 C28 전체가 WARN 이라, 부트 발화의
        #   유일한 트리거가 빠져 있어도 preflight 가 초록에 가까웠다(C08=FAIL vs C28=WARN 비대칭 —
        #   재감사 A21 확증). 나머지 자기교정 훅(inject·save·reflect·nudge·pack-guard)은 종전대로 WARN.
        for t in targets:
            for script_name, events in SELFCORR_HOOKS:
                if not os.path.isfile(os.path.join(pack_dir(), "hooks", script_name)):
                    continue
                tier_fatal = script_name in AWAKENING_SCRIPTS
                for event, matcher in events:
                    # ★U-21: 판정·쓰기 모두 **같은 선언값**을 본다(둘이 갈리면 매 런 재쓰기).
                    _to = hook_timeout_for(script_name, event)
                    # ★판정 2축을 **보고에서 가른다** — 두 사실은 다르다:
                    #   ① command 미등록 = 훅이 아예 발화하지 않는다 → 종전 티어 유지(각성=FAIL).
                    #   ② command 는 있는데 선언 timeout 미달 = 훅은 발화하되 **중간에 취소**된다.
                    #      이걸 "미등록" 이라 부르면 거짓 보고이고, 각성 FAIL 티어에 실으면
                    #      **위경고**다(H-SEED-2 가 지키는 계약: 등록됐는데 FAIL 금지).
                    #   ★티어는 보고 크기일 뿐이고, 실제 교정은 --fix 경로가 두 축 모두 동일하게
                    #     수행한다(WARN 강등이 수리를 미루지 않는다).
                    _cmd_ok = self._event_hook_registered(t, event, script_name)
                    if _cmd_ok and self._event_hook_registered(t, event, script_name, _to):
                        continue
                    if self.fix:
                        err = self._register_event_hook(t, event, script_name, matcher,
                                                        timeout=_to)
                        if err:
                            (fails if (tier_fatal and not _cmd_ok) else warns).append(
                                "%s/%s %s 실패: %s"
                                % (os.path.basename(t), event,
                                   "timeout 교정" if _cmd_ok else "등록", err))
                        else:
                            fixed.append(
                                "%s←%s(%s)%s" % (os.path.basename(t), script_name, event,
                                                 " timeout=%ss" % _to if _cmd_ok else ""))
                    elif _cmd_ok:
                        warns.append(
                            "%s %s(%s) 선언 timeout 미달 — %ss 필요(--fix로 교정). 훅은 발화하되 "
                            "하네스가 중간에 **취소하고 출력을 폐기**한다"
                            % (os.path.basename(t), script_name, event, _to))
                    else:
                        (fails if tier_fatal else warns).append(
                            "%s %s(%s) 미등록%s" % (os.path.basename(t), script_name, event,
                                                   "(--fix로 등록)" if tier_fatal else "(--fix)"))
        detail = "자기교정·영속성 hook(inject·save·reflect-scan·commit-nudge·role-bootstrap·pack-guard) 6종 + reflect 엔진"
        if fixed:
            shown = "; ".join(fixed[:6]) + (" …+%d" % (len(fixed) - 6) if len(fixed) > 6 else "")
            detail += " · " + shown
        if fails:
            self.add(cid, FAIL,
                     detail + " · ★각성 훅 결손(미등록 또는 **본체 부재** — 부트 발화 불가 · "
                     "C08 대칭 FAIL): "
                     + " | ".join(fails[:6])
                     + (" · 기타: " + " | ".join(warns[:3]) if warns else ""))
        elif warns:
            self.add(cid, WARN, detail + " · " + " | ".join(warns[:6]))
        else:
            self.add(cid, FIXED if fixed else PASS, detail)

    def c29_harness_engineering(self):
        cid = "C29.harness-engineering"
        if self.skipped(cid):
            return
        fixed, warns = [], []
        # (a) 우리 스킬이 pack에 설치됐는지 (build.rs 임베드 → init-pack 산출물)
        missing = [s for s in HARNESS_SKILLS
                   if not os.path.isfile(os.path.join(pack_dir(), "skills", s, "SKILL.md"))]
        if missing:
            warns.append("pack 스킬 미설치(%s) — init-pack 재실행" % ", ".join(missing))
        # (b) 프로필 심링크 (네이티브 스킬 발견 — C26/C27과 동일 규약)
        _home = os.path.expanduser("~")
        profiles = sorted(os.path.join(_home, d) for d in os.listdir(_home)
                          if (d == ".claude" or d.startswith(".claude-"))
                          and os.path.isdir(os.path.join(_home, d)))
        linked = 0
        for prof in profiles:
            sdir = os.path.join(prof, "skills")
            need = [s for s in HARNESS_SKILLS if s not in missing
                    and not self._symlink_ok(os.path.join(sdir, s),
                                             os.path.join(pack_dir(), "skills", s))]
            if not need:
                linked += 1
                continue
            if self.fix:
                try:
                    os.makedirs(sdir, exist_ok=True)
                    for s in need:
                        link = os.path.join(sdir, s)
                        if os.path.islink(link):
                            os.unlink(link)
                        elif os.path.exists(link):
                            continue  # 실디렉(사용자 보유) — 덮지 않음
                        os.symlink(os.path.join(pack_dir(), "skills", s), link)
                    linked += 1
                    fixed.append("%s/skills ← 하네스 스킬 심링크" % os.path.basename(prof))
                except OSError as e:
                    warns.append("%s 심링크 실패: %s" % (os.path.basename(prof), e))
            else:
                warns.append("%s/skills 하네스 스킬 미배선(--fix)" % os.path.basename(prof))
        # 판정 (FAIL 없음 — 하네스 운영은 옵트인 능력)
        detail = "하네스 스킬 %d종 · 프로필 %d/%d 배선" % (
            len(HARNESS_SKILLS) - len(missing), linked, len(profiles) or 0)
        if fixed:
            detail += " · " + "; ".join(fixed)
        if warns:
            self.add(cid, WARN, detail + " · " + " | ".join(warns))
        else:
            self.add(cid, FIXED if fixed else PASS, detail)

    # ── C30 git 결정론 점검 (2026-06-14 — git 온보딩) ──
    # git은 기여자 clone·harness-creator(C21) 툴체인 자동설치·RSI 자기개선 push에 필요하다.
    # 일반 .dmg 사용자 기본 기능엔 불필요 → 부재는 FAIL이 아니라 WARN(기능별 필수).
    def c30_git(self):
        cid = "C30.git"
        if self.skipped(cid):
            return
        p = shutil.which("git")
        if p:
            self.add(cid, PASS, "%s (기여자 clone·harness-creator·RSI 자기개선에 사용)" % p)
        else:
            self.add(cid, WARN,
                     "git 미설치 — 기여자 clone·harness-creator(C21)·RSI 자기개선이 막힌다. "
                     "설치: macOS `xcode-select --install`(또는 brew install git) · "
                     "Windows git-scm.org · Linux `apt/dnf install git`. "
                     "(일반 .dmg 사용자 기본기능엔 불필요 — 기능별 필수)")

    # ── C31 config dir 격리 + 오염 감지 (2026-06-15) ──
    # cys 마스터는 전용 CLAUDE_CONFIG_DIR(~/.cys/claude)로 격리 기동돼 사용자 ~/.claude 의
    # 외부 터미널 체계·구 지침 오염에 영향받지 않는다. 이 체크는 ①격리 라우터 설치 확인 ②사용자
    # 프로필 오염 감지(경고만 — 자동삭제 절대 안 함, 사용자 데이터 불가침)다.
    def c31_config_isolation(self):
        cid = "C31.config-isolation"
        if self.skipped(cid):
            return
        home = os.path.expanduser("~")
        cfg = os.path.join(os.path.dirname(os.path.normpath(pack_dir())), "claude")
        router = os.path.join(cfg, "CLAUDE.md")
        if not os.path.isfile(router):
            self.add(cid, WARN,
                     "cys 전용 config dir 라우터 부재(%s) — `cys init-pack` 재실행 권장 "
                     "(격리 없으면 사용자 ~/.claude 오염에 노출)" % router)
            return
        # 사용자 ~/.claude* 에 외부 터미널 체계 명령을 쓰는 구체계/구 지침 잔재 감지 (패턴은 레거시 식별자 유지)
        contaminated = []
        try:
            entries = [n for n in os.listdir(home)
                       if n == ".claude" or n.startswith(".claude-")]
        except OSError:
            entries = []
        cmux_cmd = re.compile(r"cmux (send|launch|new-split|identify|list-workspaces|notify)|cmux\.app")
        for n in entries:
            for fn in ("CLAUDE.md", "soul.md", "CSO_DIRECTIVE.md", "MASTER_DIRECTIVE.md"):
                p = os.path.join(home, n, fn)
                try:
                    t = open(p, encoding="utf-8", errors="replace").read()
                except OSError:
                    continue
                # cys 치환 선언("cmux 아님"/"치환")이 있으면 신체계 — 오염 아님
                if cmux_cmd.search(t) and ("cmux 아님" not in t) and ("치환" not in t):
                    contaminated.append(p)
        if contaminated:
            self.add(cid, WARN,
                     "사용자 프로필에 외부 터미널 체계/구 지침 %d건 감지 — cys는 전용 config dir로 격리돼 "
                     "영향 없으나, 정리하려면 **백업 후 직접 제거**(cys는 자동삭제 안 함): %s"
                     % (len(contaminated), ", ".join(contaminated[:3])))
            return
        self.add(cid, PASS, "격리 config dir 라우터 설치됨 · 사용자 프로필 외부 체계 오염 없음")

    # ── C54 god-file 회귀 방지 LOC-cap (cysd 코어 파일 줄수 ceiling 감시) ──
    def c54_loc_cap(self):
        cid = "C54.loc-cap"
        if self.skipped(cid):
            return
        # cys-terminal 소스 경로 추정: CYS_REPO_DIR env 우선, 없으면 관용 경로 후보.
        # 경로 부재면 SKIP(이 preflight는 pack 부트용 — repo 부재 환경에서 FAIL 금지).
        repo = os.environ.get("CYS_REPO_DIR")
        candidates = [repo] if repo else []
        candidates += [os.path.join(os.path.expanduser("~"), "dev", "cys-terminal")]
        root = next((c for c in candidates if c and os.path.isdir(
            os.path.join(c, "src", "bin", "cysd"))), None)
        if not root:
            self.add(cid, SKIP, "cys-terminal 소스 경로 미발견(CYS_REPO_DIR 미설정·repo 부재) — pack 단독 부트 정상")
            return
        # 대상 파일별 ceiling — 회귀 경보용 초기 상한(현재값+여유). 순수화로 줄면 따라 낮춰
        # god-file을 한 방향으로 압박(후행: ceiling 점진 강화). 형식: (모듈상대경로, ceiling).
        caps = [
            ("src/bin/cysd/handlers.rs", 5300),
            ("src/bin/cysd/governance.rs", 2000),
            ("src/bin/cysd/state.rs", 2700),
        ]
        over = []
        for rel, cap in caps:
            p = os.path.join(root, rel)
            try:
                with open(p, encoding="utf-8") as f:
                    n = f.read().count("\n") + 1  # 마지막 줄 EOF 보정
            except OSError:
                continue  # 파일 부재(리네임 등)는 건너뜀 — 존재 검증은 별 체크 소관
            if n > cap:
                over.append("%s %d줄 > ceiling %d" % (rel, n, cap))
        if over:
            # 외부발행·삭제 없는 경보라 --fix 무관(자동수리 불가) — WARN로 보고.
            self.add(cid, WARN, "god-file ceiling 초과(순수화로 분리 권장): " + "; ".join(over))
        else:
            self.add(cid, PASS, "cysd 코어 파일 LOC-cap 이내(%d개 감시)" % len(caps))

    # ── C55 grill-me 최소 질문 게이트 (제품 절대규칙) ──
    # grill-me가 합의 전 floor(20·복잡30) 결정 브랜치를 강제 해소하도록 하는 인프라:
    # 엔진(grill_gate.py)·hook(grill-gate.sh, PreToolUse deny)·SKILL 핀(pack+메인)을 검증.
    # 등록은 결정론(마커 밖 fail-open이라 무관·무해 작업을 막지 않음 — 안전).
    def c55_grill_gate(self):
        cid = "C55.grill-gate"
        if self.skipped(cid):
            return
        fixed, warns, fails = [], [], []
        pd = pack_dir()
        # (a) 엔진 존재 + self-test 통과(producer≠evaluator 분리 회귀보호)
        engine = os.path.join(pd, "bin", GRILL_ENGINE)
        if not os.path.isfile(engine):
            fails.append("엔진 %s 미설치 — `cys init-pack`" % GRILL_ENGINE)
        else:
            try:
                r = subprocess.run([sys.executable, engine, "--self-test"],
                                   capture_output=True, text=True, timeout=30,
                                   env=_utf8_env())
                if r.returncode != 0:
                    fails.append("grill_gate self-test 실패(rc=%d): %s"
                                 % (r.returncode, (r.stderr or "").strip()[:120]))
            except (OSError, subprocess.SubprocessError) as e:
                warns.append("grill_gate self-test 미실행: %s" % e)
        # (b)(c) 두 hook(check=PreToolUse·count=PostToolUse) 존재·실행권·등록.
        # ★count(evaluator) 미배선이면 distinct가 영원히 0 → fail-CLOSED 마비라 FAIL로 강제.
        #   check(gatekeeper)는 마커 밖 fail-open이라 미등록 시 WARN(강제 약화일 뿐 마비 아님).
        targets, forbidden = resolve_registration_targets()   # ★G1 sentinel(폴백 소유자)
        if forbidden and not targets:
            self.add(cid, SKIP, "등록 대상 없음 — %s" % forbidden)
            return
        for hname, hevent, hmatcher in GRILL_HOOKS:
            hook = os.path.join(pd, "hooks", hname)
            if not os.path.isfile(hook):
                fails.append("hook %s 미설치 — `cys init-pack`" % hname)
                continue
            if os.name == "posix":
                mode = os.stat(hook).st_mode
                if not mode & stat.S_IXUSR and self.fix:
                    os.chmod(hook, mode | 0o755)
                    fixed.append("%s 실행권한" % hname)
            unreg = 0
            for t in targets:
                if self._event_hook_registered(t, hevent, hname):
                    continue
                if self.fix:
                    err = self._register_event_hook(t, hevent, hname, matcher=hmatcher)
                    if err:
                        warns.append("%s 등록 실패(%s): %s"
                                     % (hname, os.path.basename(t), err))
                    else:
                        fixed.append("%s 등록(%s)"
                                     % (hname, os.path.basename(os.path.dirname(t))))
                else:
                    unreg += 1
            if unreg:
                msg = "%s %d/%d 프로필 미등록(--fix로 등록)" % (hname, unreg, len(targets))
                (fails if hname == GRILL_COUNT_HOOK else warns).append(msg)
        # (d) pack SKILL 본문 핀(게이트 지시가 비워지면 검출)
        sp = os.path.join(pd, "skills", "grill-me", "SKILL.md")
        if os.path.isfile(sp):
            try:
                text = open(sp, encoding="utf-8").read()
                lost = [p for p in GRILL_SKILL_PINS if p not in text]
                if lost:
                    fails.append("pack grill-me 핀 소실: %s" % "·".join(lost))
            except (OSError, UnicodeDecodeError) as e:
                warns.append("pack grill-me 읽기 실패: %s" % e)
        # (f) ★지침 조항 핀(절대규칙 드리프트 감시 — WARN 전용·자동수리 금지:
        #     *_DIRECTIVE는 guard 보호 헌법파일이라 preflight가 편집하지 않는다. 가시화만.)
        clause_pins = [
            ("MASTER_DIRECTIVE.md", ["todo 이중화"]),
            ("CEO_TEMPLATE.md", ["todo 이중화"]),
            ("CSO_DIRECTIVE.md", ["exited surface 자동 reap", "즉시성"]),
        ]
        for dname, pins in clause_pins:
            dp = os.path.join(pd, "directives", dname)
            if not os.path.isfile(dp):
                continue   # 지침 자체의 존재는 별도 검사(C03) 소관
            try:
                dtext = open(dp, encoding="utf-8").read()
            except (OSError, UnicodeDecodeError) as e:
                warns.append("%s 읽기 실패: %s" % (dname, e))
                continue
            lost = [p for p in pins if p not in dtext]
            if lost:
                warns.append("%s 절대규칙 핀 소실(pack 소스 동기 필요): %s"
                             % (dname, "·".join(lost)))
        # (e) 메인 .agents 핀 — Skill 도구가 실제 로드하는 사본(pack과 별개 SOT·C22 사각 교정)
        ap = os.path.join(GRILL_AGENTS_DIR, "grill-me", "SKILL.md")
        if os.path.isfile(ap):
            try:
                text = open(ap, encoding="utf-8").read()
                lost = [p for p in GRILL_AGENTS_PINS if p not in text]
                if lost:
                    warns.append(".agents grill-me 핀 소실(수동 동기화 필요): %s"
                                 % "·".join(lost))
            except (OSError, UnicodeDecodeError) as e:
                warns.append(".agents grill-me 읽기 실패: %s" % e)
        # 판정
        tail = (" · " + "; ".join(warns)) if warns else ""
        if fails:
            self.add(cid, FAIL, "grill-gate 결함: %s%s" % ("; ".join(fails), tail))
        elif fixed:
            self.add(cid, FIXED, "grill-gate 정비: %s%s" % ("; ".join(fixed), tail))
        elif warns:
            self.add(cid, WARN, "grill-gate: %s" % "; ".join(warns))
        else:
            self.add(cid, PASS, "grill-gate 인프라 건재(엔진 self-test·hook·"
                     "PreToolUse 등록·SKILL 핀 pack+메인)")

    # ── C58 트러스트 하드닝 (cysjavis 가 좌석을 띄운 (config_dir, cwd) 쌍의 폴더 신뢰) ──
    # 배경(감사 2026-09-06 에러4 ③ · WP-2): claude 는 첫기동 시 `.claude.json` projects[getcwd()].hasTrustDialogAccepted
    #   가 true 가 아니면 "폴더 신뢰" 관문(2.1.261 기본 "No, exit")을 띄운다 — dept-3 계정 dir 의 false 플래그가 좌석을 죽였다.
    #   종전 C58 의 워크스페이스 판정(_round/ 존재 AND CLAUDE.md 의 cys 토큰)은 실제 좌석 cwd(/Users/<owner>)에 CLAUDE.md 가
    #   없어 **발화 0** 이었다(수리기가 있어도 한 번도 고치지 못함).
    # ★0.14.31 스코프(플랜 WP-2 · 정본): 판정 근거 = **레지스트리 쌍** — 우리 데몬이 claude 좌석을 스폰한 (claude_config_dir,
    #   cwd) 기록(본부 topology.json + depts.json 각 부서 topology.json + depts.json (account_dir, cwd)) · 대상 config =
    #   레지스트리 config(depts.json account_dir · topology claude_config_dir · dir 존재 시 .claude.json 부재도 대상 — dept-2
    #   실측 "항목 없음"). 항목 부재도 갭.
    #   ★R1 의도적 축소(리뷰 R1 · 종전 헤더가 말한 '개인 alias 프로필(~/.claude-<profile>) 갭' 은 이제 **범위 밖**): 마커
    #   판정을 없앴으므로 레지스트리에 없는 프로필엔 어느 cwd 를 신뢰해야 하는지 알 근거가 없다 — 추정 귀속 0 원칙상 대상이
    #   아니다(hook 배선 프로필 루프는 이 이유로 제거 · 그 config 가 레지스트리에 있으면 ①' 로 잡힌다).
    # ★보안 스코프(절대·티켓 §②): blanket 신뢰 금지 — 등재 외 워크스페이스·stale 경로(dir 부재)는 무변경.
    #   합집합 살포 금지: 부서 A 의 cwd 를 부서 B 의 config 에 넣지 않는다(쌍 단위 · codex 10). 부서 컨텍스트 preflight 는
    #   자기 계정 config 만 본다(_scope_registry — 본부·타 부서 config 판정·수리 금지).
    # ★판정 키(R1 · codex): claude 가 읽는 키는 **정확한 getcwd 문자열**(POSIX realpath · claude_project_key) 하나다 —
    #   꼬리 슬래시·심링크 별칭 키가 true 여도 claude 는 그 항목을 읽지 않는다 → 갭 판정·시드 모두 정확 키만 본다(별칭 무접촉).
    # 쓰기는 `--seed-trust` 와 **같은 경로**(seed_trust: 이미 신뢰면 무프로브 · 라이브 claude 0 확인 · 비차단 잠금 · 교체 직전
    #   재읽기 대조 · 원자 교체 · 되읽기 · .bak-preflight 1회) — C58 도 라이브 세션의 .claude.json 을 덮지 않는다. trust 는 가역
    #   로컬 변경이라 may_mutate(비가역 외부설치) 게이트가 아니라 self.fix 게이트로 집행(dry/safe 에선 self.fix=False → 탐지만).
    # 판정 정직성(§3-3): 레지스트리 출처가 0(topology·depts.json 어느 것도 판독 못 함)이면 PASS 가 아니라 SKIP — 볼 수 없었던
    #   것을 '갭 없음' 으로 말하지 않는다(종전: 부서 컨텍스트에서 빈 레지스트리 → PASS "0쌍 출처 0" 침묵 오판 · 리뷰 R1).
    def _registry(self):
        """cysjavis_registry() 1회 캐시 — 한 preflight 실행 안에서 판독 일관(읽기 전용)."""
        cache = self.__dict__.get("_registry_cache")
        if cache is None:
            cache = cysjavis_registry()
            self.__dict__["_registry_cache"] = cache
        return cache

    # ★R2(리뷰): 종전 `_is_cysjavis_workspace`(0.14.30 의 _round/·CLAUDE.md 마커 판정 → R1 에서 레지스트리 동일성 판정으로
    #   고쳐 둔 것)는 **삭제** — 생산 호출자 0 이었고 동일성(_path_identity) 판정이 아래 정확 키 판정과 갈릴 수 있었다.
    #   워크스페이스 판정은 `_trust_gap_workspaces` 하나(등재 쌍 → 정확 키)다.
    def _trust_gap_workspaces(self, config_path):
        """읽기전용 탐지 — .claude.json 미변경. 그 config 에 등재된 좌석 cwd 중 projects[claude_project_key(cwd)] 의
        hasTrustDialogAccepted 가 True 가 아닌 것 — **항목 부재도 갭** · 별칭 키(꼬리 슬래시·심링크)의 true 는 인정하지 않는다
        (claude 는 정확 키만 읽는다 · R1). dir 부재 cwd 는 제외(stale)."""
        config_dir = os.path.dirname(config_path)
        registered = self._registry()["pairs"].get(_path_identity(config_dir), {})
        if not registered:
            return []
        # 판독은 **막히지 않는 관용 판독**(`_read_json_tolerant` = O_NONBLOCK + fstat 정규 재확인)이다. R4 에서
        #   codex major(writer 없는 FIFO `.claude.json` 이 preflight 를 영구 정지)를 닫으려고 시더 가드
        #   (`_read_claude_json_bytes`)로 바꿨으나, 그것은 **심링크를 거부**한다 → ★R5(리뷰 minor): `.claude.json` 을
        #   심링크로 둔 설치는 대상 문서에 플래그가 이미 있어도 data=None 이 되어 등재 cwd 전부가 **영구 거짓 갭**
        #   이 되고, `--fix` 는 시더가 같은 이유로 REFUSE 해 매 실행 WARN 이 반복됐다(자가 치유 불가). 탐지는 읽기
        #   전용이므로 심링크를 따라가고(FIFO 정지 차단은 그대로), **쓰기 정책은 시더가 따로 판정**한다 — 심링크
        #   문서에 진짜 갭이 있으면 그때 seed_trust 가 REFUSE 1줄을 남긴다(사람이 관문에서 1회 신뢰).
        #   판독 불가(비정규·손상·IO)의 판정은 종전 None 과 같다: projects 를 못 봤으므로 등재 cwd 전부가 갭.
        data = _read_json_tolerant(config_path)
        projs = data.get("projects") if isinstance(data, dict) else None
        if not isinstance(projs, dict):
            projs = {}
        gaps = []
        for cwd in registered.values():
            if not os.path.isdir(cwd):
                continue
            ent = projs.get(claude_project_key(cwd))
            if not (isinstance(ent, dict) and ent.get("hasTrustDialogAccepted") is True):
                gaps.append(cwd)
        return sorted(set(gaps))

    def _seed_journals(self, cfg_dir):
        """읽기 전용 — 그 config dir 의 미해결 교환 저널(`.claude.json.seed-intent-*`) 이름들 → (names, err).
        열거 실패는 ([], errno 이름) 이고 그것은 **'저널 없음' 이 아니다**(★R6 codex: PASS·갭 없음으로 접지 않는다)."""
        try:
            return sorted(n for n in os.listdir(cfg_dir) if n.startswith(SEED_TRUST_INTENT_PREFIX)), None
        except OSError as e:
            return [], (errno.errorcode.get(e.errno, str(e.errno)) if e.errno else str(e))

    def _seed_pair(self, cfg_dir, ws):
        """C58 의 **유일한 쓰기 진입점** → (rc, verdict, reason) 또는 None(report 모드).
        `self.fix` 가드를 이 한 자리에 둔다 — 호출 지점이 둘(갭 수리 · 저널 판정)로 늘어도 report 모드 읽기 전용이
        호출자 수만큼 흩어지지 않는다(★R6). 예외는 WARN 1줄로 접는다: run() 의 fut.result() 로 올라가면 preflight
        전체가 죽는다(R3 · lone surrogate UnicodeEncodeError 재현)."""
        if not self.fix:
            return None
        try:
            return seed_trust(cfg_dir, ws, backup=True)
        except Exception as e:  # noqa: BLE001 — 수리 1건의 예외가 점검 전체를 멈추면 안 된다
            return SEED_TRUST_ERROR, "ERROR", "예외 %s: %s" % (type(e).__name__, e)

    def _survey_journals(self, reg, previous=None):
        """스코프 안 모든 config dir 의 저널 상태를 **읽기 전용**으로 훑는다 → {identity: {dir, names, err, verdict}}.
        `previous` 를 주면 그 판정(verdict)을 이어받는다 — 수리 **뒤** 재열거용.
        ★R6(codex 위임 반례 ②): 수리가 **새 저널을 남길 수도** 있다(교환은 성립했는데 청소가 실패한 정상 결과).
        수리 전 스냅샷에 그 config 가 없었다는 이유로 재열거를 건너뛰면 FIXED 를 내며 잔존을 감춘다 — 그래서
        재열거는 스냅샷 유무와 무관하게 **전 대상**을 다시 훑는다."""
        out = {}
        for k, cfg_dir in sorted(reg["configs"].items(), key=lambda kv: kv[1]):
            if k in out or not os.path.isdir(cfg_dir):
                continue
            names, err = self._seed_journals(cfg_dir)
            prev = (previous or {}).get(k) or {}
            if names or err or prev.get("verdict") is not None:
                out[k] = {"dir": cfg_dir, "names": names, "err": err, "verdict": prev.get("verdict")}
        return out

    def _existing_registered_cwd(self, reg, key):
        """그 config 에 등재된 cwd 중 **실재하는** 첫 것(정렬 고정) 또는 None — 저널 판정의 호출 인자."""
        return next((w for w in sorted(reg["pairs"].get(key, {}).values()) if os.path.isdir(w)), None)

    def c58_trust_harden(self):
        cid = "C58.trust-harden"
        if self.skipped(cid):
            return
        reg = self._registry()
        n_pairs = sum(len(v) for v in reg["pairs"].values())
        unreadable = reg.get("unreadable") or []
        stats = "출처 %d · config %d · 쌍 %d · 판독불가 %d · scope=%s" % (
            len(reg["sources"]), len(reg["configs"]), n_pairs, len(unreadable), reg.get("scope", "?"))
        # ★R6(리뷰 codex major): 중단된 교환은 **신뢰 플래그가 이미 활성**이라 갭이 0 이다 — 갭만 보는 판정은
        #   미해결 저널(상대의 더 새 문서가 displaced 에 고립된 상태)을 안고 PASS 를 냈고 `--fix` 에서도
        #   seed_trust 가 아예 불리지 않아 저널 판정이 돌지 않았다. 저널 점검은 갭과 **독립**이며 쌍 0(SKIP)
        #   보다도 **앞**이다(등재 cwd 가 없는 config 의 저널도 보여야 한다 · codex R6). 열거는 읽기 전용이다.
        journals = self._survey_journals(reg)
        if n_pairs == 0:
            # ★R1(codex): 판정할 쌍이 0 이면 PASS 가 아니라 SKIP — config 가 있다는 사실은 워크스페이스 신뢰에 대해 아무것도
            #   증명하지 않는다. 출처 0(판독 없음)·임시 팩·계정 미상 부서·판독불가 파일 전부 이 경로.
            tail = (" · 판독불가: " + " | ".join(unreadable)) if unreadable else ""
            residual, _resolved = _journal_lines(journals)
            if residual:
                # 쌍이 0 이어도 미해결 저널은 **관측된 사실**이다 — SKIP(판정 불가)로 덮지 않는다.
                self.add(cid, WARN, "cysjavis 레지스트리 쌍 0(%s · 트러스트 판정 불가) · %s%s"
                         % (stats, " | ".join(residual), tail))
                return
            self.add(cid, SKIP, "cysjavis 레지스트리에 판정할 (config, cwd) 쌍 0(%s) — 트러스트 판정 불가(PASS 아님 · 데몬이 "
                     "claude 좌석을 기록한 뒤 재판정)%s" % (stats, tail))
            return
        targets = []
        missing_cfg = []
        seen = set()
        # 대상 config = 레지스트리 config(depts.json account_dir · topology claude_config_dir) — cys-dept/데몬이 좌석을 띄운
        #   계정 dir 그 자체(codex 12). dir 존재 시 .claude.json 부재도 대상(시드가 생성한다). 부서 컨텍스트는 자기 계정만(스코프).
        #   ★R1: config dir 자체가 부재인데 쌍이 등재돼 있으면 침묵 통과가 아니라 WARN 줄(수리 대상은 아니다 — 지워진 계정 dir 을
        #   preflight 가 되살리지 않는다 · 등재 stale 가능성 고지).
        for k, cfg_dir in sorted(reg["configs"].items(), key=lambda kv: kv[1]):
            if k in seen:
                continue
            seen.add(k)
            if not os.path.isdir(cfg_dir):
                if reg["pairs"].get(k):
                    missing_cfg.append("%s(config dir 부재 · 등재 cwd %d — stale 등재?)" % (cfg_dir, len(reg["pairs"][k])))
                continue
            targets.append(os.path.join(cfg_dir, ".claude.json"))
        set_lines = []
        gap_lines = list(missing_cfg)
        for cfg in targets:
            key = _path_identity(os.path.dirname(cfg))
            gaps = self._trust_gap_workspaces(cfg)
            if not gaps:
                # 갭 0 인데 저널이 있으면 --fix 는 **기존 단일 경로**(seed_trust ⑬ 회수)를 한 번 돌린다. 갭이 있는
                #   config 는 아래 수리 호출이 같은 회수를 하므로 여기서 중복 호출하지 않는다(codex R6).
                entry = journals.get(key)
                if entry and entry["names"] and self.fix:
                    ws = self._existing_registered_cwd(reg, key)
                    if ws:
                        entry["verdict"] = self._seed_pair(entry["dir"], ws)
                continue
            if self.fix:
                for ws in gaps:
                    # ★쓰기는 --seed-trust 와 같은 경로 — 라이브 claude(그 config) 존재·검증 불가·잠금 경합·동시 변경은
                    #   전부 보류로 접힌다(막는 쪽으로만 틀린다).
                    rc, verdict, reason = self._seed_pair(os.path.dirname(cfg), ws)
                    if rc == SEED_TRUST_OK:
                        set_lines.append("trust set: %s / %s (%s)" % (cfg, ws, reason))
                    else:
                        gap_lines.append("%s / %s: 세팅 보류 — %s %s" % (cfg, ws, verdict, reason))
            else:
                for ws in gaps:
                    gap_lines.append("trust gap: %s / %s (--fix로 세팅)" % (cfg, ws))
        if self.fix:
            # rc 만으로 '정리 완료' 를 단정하지 않는다 — 수리 뒤 **재열거**가 잔존의 정본이다(codex R6 · 위임 반례 ②).
            journals = self._survey_journals(reg, journals)
        residual, resolved = _journal_lines(journals)
        gap_lines.extend(residual)
        set_lines.extend(resolved)
        if unreadable:
            gap_lines.append("판독불가 레지스트리 파일: " + " | ".join(unreadable))
        if set_lines:
            detail = "cysjavis 좌석 (config, cwd) 트러스트 세팅 — " + " | ".join(set_lines)
            if gap_lines:
                detail += " · 잔존: " + " | ".join(gap_lines)
            self.add(cid, FIXED if not gap_lines else WARN, detail)
        elif gap_lines:
            self.add(cid, WARN, "트러스트 갭 — " + " | ".join(gap_lines))
        else:
            self.add(cid, PASS, "cysjavis 좌석 트러스트 OK(갭 없음 · %d config 점검 · %s)" % (len(targets), stats))

    # ── C59 역할별 Bash denylist guard 배선 검증 (WP-2 · 감사 X-1·H-HOOK-3) ──
    # 감사 2026-07-06: 워커 역할 프로필에 Bash denylist guard 부재(X-1),
    # master 역할 프로필은 개인경로 guard 직접배선(H-HOOK-3). guard.sh를 팩 hooks/로
    # 편입한 뒤, 두 역할 프로필의 PreToolUse에 팩경로 guard 배선 존재를 결정론 검증한다.
    # 이전엔 guard 배선 검사 자체가 없어 배선 누락이 침묵 통과("skip정상")했다 → 부재 hard-fail.
    # 검증만 수행(자동 배선 안 함): 잘못된 Bash guard는 전 Bash를 마비시키므로 배선은 의도적
    # 수동 행위여야 한다(외과적 — settings enforcement 변경은 Tier C 정지경계).
    # ★PII 하드게이트(secret-scan HANDLE) 대응: 역할 프로필 dotdir 실명을 공개 리포에 박지 않는다.
    #   공급 경로: ①env CYS_GUARD_ROLE_PROFILES(콤마구분) ②<pack>/guard-profiles.txt(로컬 파일 —
    #   리포·임베드 미포함, 오너 머신 전용) ③둘 다 없으면 검증 대상 없음 skip(PASS) — 소비자
    #   설치본엔 역할 프로필 자체가 없어 의미 동일.
    @staticmethod
    def _guard_role_profiles():
        env = os.environ.get("CYS_GUARD_ROLE_PROFILES", "")
        names = tuple(x.strip() for x in env.split(",") if x.strip())
        if names:
            return names
        try:
            fp = os.path.join(pack_dir(), "guard-profiles.txt")
            with open(fp, encoding="utf-8") as f:
                return tuple(ln.strip() for ln in f if ln.strip() and not ln.startswith("#"))
        except OSError:
            return ()

    @staticmethod
    def _guard_wired(settings_path):
        """PreToolUse 에 팩경로 guard.sh(hooks/guard.sh) 배선이 있으면 True."""
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        for entry in data.get("hooks", {}).get("PreToolUse", []):
            if not isinstance(entry, dict):
                continue
            for h in entry.get("hooks", []):
                if isinstance(h, dict) and "hooks/guard.sh" in h.get("command", "").replace("\\", "/"):
                    return True
        return False

    def c59_guard_wiring(self):
        cid = "C59.guard-wiring"
        if self.skipped(cid):
            return
        # 1) 팩에 guard.sh 실체 존재(+실행권한) — 배선 대상이 있어야 배선이 의미
        gp = os.path.join(pack_dir(), "hooks", "guard.sh")
        if not os.path.isfile(gp):
            self.add(cid, FAIL, "hooks/guard.sh 없음 — 팩 편입 필요(WP-2)")
            return
        if os.name == "posix" and not (os.stat(gp).st_mode & stat.S_IXUSR):
            if self.fix:
                os.chmod(gp, os.stat(gp).st_mode | 0o755)
            else:
                self.add(cid, FAIL,
                         "hooks/guard.sh 실행권한 없음 — 직접 실행(shebang) 배선이라 755 필수(--fix로 부여)")
                return
        # 2) 역할 프로필(master·워커)별 PreToolUse guard 배선 존재 검증
        profiles = self._guard_role_profiles()
        if not profiles:
            self.add(cid, PASS, "역할 프로필 명단 미공급(env/guard-profiles.txt) — guard 배선 검증 skip")
            return
        targets = [s for s in discover_claude_settings()
                   if os.path.basename(os.path.dirname(s)) in profiles]
        if not targets:
            self.add(cid, PASS, "master·워커 역할 프로필 미설치 — guard 배선 대상 없음")
            return
        missing = [s for s in targets if not self._guard_wired(s)]
        if missing:
            names = ", ".join(os.path.basename(os.path.dirname(s)) for s in missing)
            self.add(cid, FAIL,
                     "역할 프로필 Bash guard 배선 부재: %s — PreToolUse에 hooks/guard.sh 배선 필요"
                     " (감사 X-1·H-HOOK-3)" % names)
            return
        self.add(cid, PASS,
                 "master·워커 guard 배선 OK (%d 프로필 검증 · %s)" % (len(targets), gp))

    # ── C60 결정론 게이트 '배선' 검증 (G4 · cokacdir 성찰 2026-07-04) ──
    # C41/C42는 게이트 도구의 존재·self-test만 본다 — "게이트를 짓고 문에 안 달았다"(G4)를
    # 여기서 닫는다: ①재주입 포이즌 게이트(G3) 배선 ②memory 스캐너 로드 가능(fail-closed는
    # 부트 게이트인 여기 — 런타임 memory 쓰기는 생명선 WARN 유지, G14 층 분리) ③skillscan
    # 집행 스캔 실행 ④mcpgate 스냅샷 diff(rug-pull).
    def c60_gate_wiring(self):
        cid = "C60.gate-wiring"
        if self.skipped(cid):
            return
        probs, warns = [], []
        # (a) G3 재주입 게이트 배선 — hook이 게이트를 실제로 경유하는가
        hook = os.path.join(pack_dir(), "hooks", "inject-context.sh")
        gate = os.path.join(pack_dir(), "hooks", "inject_gate.py")
        try:
            hook_txt = open(hook, encoding="utf-8").read()
        except OSError:
            hook_txt = ""
        if not (os.path.isfile(gate) and "inject_gate.py" in hook_txt):
            probs.append("재주입 포이즌 게이트 미배선(hooks/inject_gate.py + inject-context.sh _gate)")
        # (b) memory 포이즌 스캐너 로드 가능 — 다운이면 부트 FAIL(fail-closed 층)
        r = subprocess.run(
            [sys.executable, "-c",
             "import sys; sys.path.insert(0, %r); import javis_memory as m; "
             "sys.exit(0 if m._skillscan is not None else 3)" % os.path.join(pack_dir(), "bin")],
            capture_output=True, timeout=60)
        if r.returncode != 0:
            probs.append("memory 포이즌 스캐너 다운(_skillscan=None · fail-open 상태)")
        # (c) skillscan 집행 스캔(전 스킬 정적·~6s 실측) — BLOCK verdict는 정지경계 정책
        #     (feedback_skillscan-gate-policy)에 따라 WARN+명시 목록(처분은 master/CSO).
        #     ★승인 저장소(2026-07-04 master 승인): _round/skillscan_acknowledged.json —
        #     fingerprint 핀 일치 시만 면제. 스킬 내용 변경=핀 불일치=자동 재차단.
        acked_note = ""
        try:
            scan_tool = os.path.join(pack_dir(), "bin", "javis_skillscan.py")
            r = subprocess.run([sys.executable, scan_tool, "all", "--json"],
                               capture_output=True, text=True, timeout=120)
            data = json.loads(r.stdout or "{}")
            blocked = data.get("blocked") or []
            if blocked:
                ack_p = os.path.join(os.environ.get("JAVIS_ROOT") or os.getcwd(),
                                     "_round", "skillscan_acknowledged.json")
                try:
                    acks = json.load(open(ack_p, encoding="utf-8"))
                except (OSError, json.JSONDecodeError):
                    acks = {}
                residual, acked = [], []
                for s in blocked:
                    fp = None
                    if s in acks:
                        rc = subprocess.run(
                            [sys.executable, scan_tool, "card",
                             os.path.join(pack_dir(), "skills", s), "--json"],
                            capture_output=True, text=True, timeout=60)
                        try:
                            fp = json.loads(rc.stdout).get("fingerprint")
                        except (json.JSONDecodeError, ValueError):
                            fp = None
                    if fp and fp == acks[s].get("fingerprint"):
                        acked.append(s)
                    else:
                        residual.append(s)
                if residual:
                    warns.append("skillscan BLOCK %d건(미승인/핀 불일치): %s — 정지경계 정책 "
                                 "검토(master/CSO)" % (len(residual), ", ".join(sorted(residual)[:8])))
                if acked:
                    acked_note = " · BLOCK 승인 %d건(핀 일치)" % len(acked)
        except Exception as e:
            probs.append("skillscan 집행 스캔 실행 불가(%s)" % e)
        # (d) mcpgate rug-pull diff — 승인 스냅샷 저장소 기반(스냅샷 없으면 미가동 경고)
        store = os.path.join(os.environ.get("JAVIS_ROOT") or os.getcwd(), "_round", "mcp_approved")
        snaps = sorted(f for f in (os.listdir(store) if os.path.isdir(store) else [])
                       if f.endswith(".json"))
        if not snaps:
            warns.append("mcpgate 승인 스냅샷 0 — MCP 등록 시 snapshot 의무화 미가동")
        else:
            changed = []
            for f in snaps[:10]:
                skill = os.path.join(pack_dir(), "skills", f[:-5])
                r = subprocess.run(
                    [sys.executable, os.path.join(pack_dir(), "bin", "javis_mcpgate.py"),
                     "diff", skill, "--store", store, "--json"],
                    capture_output=True, text=True, timeout=60)
                if r.returncode != 0:
                    changed.append(f[:-5])
            if changed:
                probs.append("mcpgate diff 변경 감지(rug-pull 의심): %s" % ", ".join(changed))
        if probs:
            self.add(cid, FAIL, " | ".join(probs + warns))
        elif warns:
            self.add(cid, WARN, " | ".join(warns) + acked_note)
        else:
            self.add(cid, PASS, "게이트 배선 OK(재주입·memory·skillscan 집행·mcpgate diff)" + acked_note)

    # ── C61 doc-code SOT 대조 확장 (G15) — 스킬 한정(:2286)을 넘어 SESSION_STATE·directive가
    #     명명한 파일 실재를 대조(드리프트=WARN — 산문은 계획·과거를 적을 수 있어 FAIL 아님) ──
    def c61_doc_code_sot(self):
        cid = "C61.doc-code-sot"
        if self.skipped(cid):
            return
        root = os.environ.get("JAVIS_ROOT") or os.getcwd()
        docs = [os.path.join(root, "_round", "SESSION_STATE.md")]
        ddir = os.path.join(pack_dir(), "directives")
        if os.path.isdir(ddir):
            docs += [os.path.join(ddir, f) for f in sorted(os.listdir(ddir))
                     if f.endswith(".md")]
        tok_re = re.compile(r"`([^`\n]{3,120})`")
        missing, seen, checked = [], set(), 0
        for doc in docs:
            try:
                # SESSION_STATE는 고정 헤더부만(날짜 진행로그 제외 — inject-context 발췌와 동일 규칙)
                lines = open(doc, encoding="utf-8").read().split("\n")
                if doc.endswith("SESSION_STATE.md"):
                    kept, keep = [], True
                    for ln in lines:
                        if ln.startswith("## "):
                            keep = not re.search(r"\[20[0-9][0-9]", ln)
                        if keep:
                            kept.append(ln)
                    lines = kept
                text = "\n".join(lines)
            except OSError:
                continue
            for tok in tok_re.findall(text):
                if tok in seen:
                    continue
                seen.add(tok)
                # 경로형 토큰만: 구분자 포함 + 파일 확장자, 글롭/플레이스홀더/URL 제외.
                # ★말줄임(...)·중간 ~(범위 표기 3.1~3.9)은 산문 표기지 경로가 아님(실측 오탐 2건).
                if ("/" not in tok or any(c in tok for c in "*<>{}$|;\" ")
                        or tok.startswith("http") or not re.search(r"\.(py|sh|md|json|jsonl)$", tok)
                        or "..." in tok or "~" in tok[1:]):
                    continue
                p = os.path.expanduser(tok)
                if not os.path.isabs(p):
                    # 해석 루트 = 프로젝트 + 팩 + ★doc_sot_roots.txt(교차 repo 참조 — 개인경로는
                    #   팩 코드가 아니라 프로젝트 소유 설정 파일에 둔다: pack scan gate 관례)
                    roots_f = os.path.join(root, "_round", "doc_sot_roots.txt")
                    extra = []
                    try:
                        extra = [ln.strip() for ln in open(roots_f, encoding="utf-8")
                                 if ln.strip() and not ln.startswith("#")]
                    except OSError:
                        pass
                    cands = [os.path.join(root, p), os.path.join(pack_dir(), p)] \
                        + [os.path.join(os.path.expanduser(r), p) for r in extra]
                else:
                    cands = [p]
                checked += 1
                if not any(os.path.exists(c) for c in cands):
                    missing.append(tok)
        if missing:
            self.add(cid, WARN, "doc-code 드리프트 %d/%d건 — 문서가 명명한 파일 부재: %s"
                     % (len(missing), checked, ", ".join(missing[:10])))
        else:
            self.add(cid, PASS, "doc-code SOT 대조 OK(경로형 토큰 %d건 실재)" % checked)

    # ── C62 팩 치유 원장 가시화 (2026-07-12 치유 원복 사고 시정) ──
    # init-pack이 수정된 system 파일을 임베드로 되돌리면(healed) 수정본은 <rel>.user로,
    # user-owned 신버전은 <rel>.new로 병치되고 .merge-pending.json 원장에 기록되는데, 이
    # 원장을 아무도 읽지 않아 라이브 수정 소실이 무통보로 반복됐다(로컬·배포 사용자 양쪽
    # 실측 사고). 부트 ⓪ 출력에 병합 대기를 올려 치유 발생을 관측 가능하게 한다.
    # (현행 0.14.29: healed는 벤더 전진 충돌 시에만 발생 — 벤더 미전진 드리프트는 kind
    #  kept-drift로 제자리 보존·기한 검사 제외는 C68 참조.)
    # 체크 목록 '마지막' 고정: 같은 런의 --fix(repair_via_init_pack)가 남긴 신규 원장까지
    # 이 런에서 보여야 한다. 읽기 전용(report 병렬 안전)·WARN(READY 미차단).
    def c62_pack_heal_ledger(self):
        cid = "C62.pack-heal-ledger"
        if self.skipped(cid):
            return
        ledger = os.path.join(pack_dir(), ".merge-pending.json")
        if not os.path.isfile(ledger):
            self.add(cid, PASS, "병합 대기 0건 (원장 없음)")
            return
        try:
            with open(ledger, encoding="utf-8") as f:
                pending = json.load(f)
            if not isinstance(pending, dict):
                raise ValueError("원장 루트가 객체가 아님")
        except Exception as e:
            self.add(cid, WARN, "병합 원장 파싱 실패(%s) — %s 수동 확인" % (e, ledger))
            return
        healed = sorted(r for r, v in pending.items()
                        if isinstance(v, dict) and v.get("kind") == "healed")
        newp = sorted(r for r, v in pending.items()
                      if isinstance(v, dict) and v.get("kind") == "new-pending")
        # ★T3(D2): Merge3 폴백(conflicted)·백업 실패 격리(quarantined) 가시화 — 2026-07-12 사고
        # 시정 채널(C62)이 신 충돌 클래스를 못 보는 사각의 봉인. kept-drift·merged(at-rest)는
        # 조치 불요 보존 상태라 WARN "판정 집합" 비대상(발동 조건 불변 — 부트 요약 1줄·pack-plan
        # 이 담당)이며, ★T4: WARN 발동 시 상세 문자열에만 정보 병기한다(단독 존재 = PASS 유지 ·
        # at-rest state 가 붙은 conflicted 항목도 "(at-rest)" 병기 — 제외 아님 · master 결정).
        conflicted = sorted(r + (" (at-rest)" if v.get("state") == "at-rest" else "")
                            for r, v in pending.items()
                            if isinstance(v, dict) and v.get("kind") == "conflicted")
        quarantined = sorted(r for r, v in pending.items()
                             if isinstance(v, dict) and v.get("kind") == "quarantined")
        if not healed and not newp and not conflicted and not quarantined:
            self.add(cid, PASS, "병합 대기 0건")
            return
        parts = []
        if healed:
            parts.append("★원복(healed) %d건 — 라이브 수정이 배포 원본으로 되돌려짐(수정본은 <파일>.user 보존): %s%s"
                         % (len(healed), ", ".join(healed[:8]),
                            " 외 %d건" % (len(healed) - 8) if len(healed) > 8 else ""))
        if newp:
            parts.append("신버전 대기(.new) %d건: %s%s"
                         % (len(newp), ", ".join(newp[:8]),
                            " 외 %d건" % (len(newp) - 8) if len(newp) > 8 else ""))
        if conflicted:
            parts.append("★병합 충돌(conflicted) %d건 — vendor 본 적용·내 수정본 <파일>.user·조상 <파일>.base 보존: %s%s"
                         % (len(conflicted), ", ".join(conflicted[:8]),
                            " 외 %d건" % (len(conflicted) - 8) if len(conflicted) > 8 else ""))
        if quarantined:
            parts.append("★격리(quarantined) %d건 — 판독 불가 + 백업 실패로 손대지 않음(파일 그대로 · UTF-8 회복 후 재스윕): %s%s"
                         % (len(quarantined), ", ".join(quarantined[:8]),
                            " 외 %d건" % (len(quarantined) - 8) if len(quarantined) > 8 else ""))
        # ★T4: at-rest kind(kept-drift·merged) 정보 병기 — WARN 발동 시 상세에만(판정 집합 불변).
        atrest_kd = sum(1 for v in pending.values()
                        if isinstance(v, dict) and v.get("kind") == "kept-drift")
        atrest_mg = sum(1 for v in pending.values()
                        if isinstance(v, dict) and v.get("kind") == "merged")
        if atrest_kd or atrest_mg:
            parts.append("(at-rest) kept-drift %d건·merged %d건 — 조치 불요 보존 상태(정보 병기)"
                         % (atrest_kd, atrest_mg))
        self.add(cid, WARN, "; ".join(parts)
                 + " — `cys pack-merge`로 검토(가치 있는 수정은 vendor 승격 제보)"
                 + " · 방금 원복된 파일의 원커맨드 복원: `cys pack-rollback --file <파일>`")

    # ── C68 병합 원장 체류 기한 게이트 (★W-D1 커스텀 생존 2026-07-17) ──
    # 고지 채널은 실측으로 반증됐다(원장 항목 9주 체류 — C62 WARN·init-pack 보고 줄이 있었는데도).
    # 소비를 강제한다: 기한 초과 항목이 있으면 WARN + master 에게 wakeup 큐로 "병합 검토 위임"
    # 티켓 신호를 push(코얼레싱·멱등 — javis_wakeup 재사용). master 는 앵커대로 직접 병합하지
    # 않고 워커에 검토를 위임·승인만 한다. WARN 전용(READY 미차단)·--fix 비대상(스윕 트리거 아님).
    def c68_merge_pending_age(self):
        cid = "C68.merge-pending-age"
        if self.skipped(cid):
            return
        ledger = os.path.join(pack_dir(), ".merge-pending.json")
        if not os.path.isfile(ledger):
            self.add(cid, PASS, "병합 대기 0건")
            return
        try:
            with open(ledger, encoding="utf-8") as f:
                pending = json.load(f)
            if not isinstance(pending, dict):
                raise ValueError("원장 루트가 객체가 아님")
        except Exception as e:
            self.add(cid, WARN, "병합 원장 파싱 실패(%s) — C62 참조" % e)
            return
        try:
            max_days = float(os.environ.get("CYS_MERGE_PENDING_MAX_DAYS", "14"))
        except ValueError:
            max_days = 14.0
        now = time.time()

        # ★T3(D1): 영속 kind 는 기한 개념이 없다(체류가 정상 상태) — kept-drift·merged(∨ state
        # at-rest)는 stale 산식에서 제외한다. 제외 없이는 fingerprint(일 단위 oldest)가 매일
        # 새 멱등키를 만들어 wakeup 큐 일일 재배달 폭주가 확정된다(성찰 4렌즈 공통 실측).
        # conflicted·quarantined·healed·new-pending 은 조치 가능(actionable)이라 기한 유지.
        # ★0.14.29 성찰 차단 수리: T4 신설 adopted(복권 확정 — 다음 스윕에 kept-drift 정규화·
        # state 무부여)도 제외 — 워커가 할 일이 없는 상태(pack-merge 표시 "↩ 복권됨"뿐)를
        # actionable 로 계상하면 스윕 0회(버전 무변경+init-pack 무실행) 14일 후 C62 PASS ∧
        # C68 WARN 비대칭 + fingerprint(일 단위 oldest) 일일 갱신 = wakeup 큐 일일 재배달
        # 소음원이 된다(픽스처 실측). 복권 완료 = 스윕 대기 정상 체류.
        # 제외 kind 집합 = 모듈 상수 C68_EXEMPT_KINDS(정본 src/pack.rs LEDGER_KINDS 의 부분집합
        # — census 핀 대조 대상)로 단일화.
        def _exempt(v):
            return (v.get("kind") in C68_EXEMPT_KINDS
                    or v.get("state") == "at-rest")

        stale = sorted(
            (rel, (now - float(v.get("ts", now))) / 86400.0)
            for rel, v in pending.items()
            if isinstance(v, dict) and not _exempt(v)
            and (now - float(v.get("ts", now))) / 86400.0 > max_days
        )
        exempt_n = sum(1 for v in pending.values() if isinstance(v, dict) and _exempt(v))
        if not stale:
            self.add(cid, PASS, "병합 대기 %d건 — 전부 기한(%.0f일) 이내%s"
                     % (len(pending), max_days,
                        " (영속 kind 기한 제외 %d건)" % exempt_n if exempt_n else ""))
            return
        # 소비 강제 신호: master wakeup 큐 enqueue(멱등 키=원장 지문 — 같은 잔존 상태로 재부트해도 1건).
        # ★모드 계약 준수(C28 관례와 동일 `self.fix` 게이트): report=관찰만·safe/dry=무변경이므로
        # 큐 적재(가역 부작용)는 --fix(부트 ⓪ 표준 호출)에서만 집행한다. 다른 모드는 WARN 관찰만.
        oldest = max(d for _, d in stale)
        fingerprint = "%d-%d" % (len(stale), int(oldest))
        enq = "관찰만(--fix 에서 master 큐 적재)"
        wakeup = os.path.join(pack_dir(), "bin", "javis_wakeup.py")
        # ★cwd 의존 방어(launchd cwd=/ 오염 사고 계열 · 2026-07-15 실측): javis_wakeup 의 큐 루트는
        # `JAVIS_ROOT or os.getcwd()` 라, 부트가 워크스페이스 밖(cwd=/ 등)에서 실행되면 엉뚱한 곳에
        # 큐를 만든다. 루트가 결정론으로 확정될 때만 적재하고, 아니면 WARN 관찰만(무해측).
        wk_root = os.environ.get("JAVIS_ROOT") or os.getcwd()
        root_ok = os.path.isdir(os.path.join(wk_root, "_round"))
        if self.fix and not root_ok:
            enq = "큐 적재 보류(워크스페이스 루트 미확정: %s — JAVIS_ROOT 미설정·cwd 에 _round 부재)" % wk_root
        if self.fix and root_ok and os.path.isfile(wakeup):
            try:
                r = subprocess.run(
                    [sys.executable, wakeup, "enqueue", "--to", "master",
                     "--task", "merge-review",
                     "--reason", "병합 원장 기한 초과 %d건(최장 %.0f일) — 워커에 pack-merge 검토 위임 필요"
                                 % (len(stale), oldest),
                     "--idempotency-key", "merge-review-" + fingerprint],
                    capture_output=True, text=True, timeout=10, env=_utf8_env())
                enq = "wakeup enqueue %s" % ("OK" if r.returncode == 0 else "실패(%d)" % r.returncode)
            except Exception as e:
                enq = "wakeup enqueue 예외(%s)" % e
        shown = ", ".join("%s(%.0f일)" % (rel, d) for rel, d in stale[:6])
        self.add(cid, WARN,
                 "병합 대기 기한(%.0f일) 초과 %d건: %s%s — master: 워커에 `cys pack-merge` 검토 위임(직접 병합 금지) · %s"
                 % (max_days, len(stale), shown,
                    " 외 %d건" % (len(stale) - 6) if len(stale) > 6 else "", enq))

    # ── C65 cys drain --verify 능력 체크 (기능1 이월분 · 재시작 전 저장검증 feature-detect) ──
    # GUI 저장후재시작 흐름이 `cys drain --verify`에 의존한다. 번들 cys가 미지원(구버전 스큐)이면 GUI가
    # plain drain 으로 폴백해야 하며 그 스큐를 부트에서 표면화한다. ★F4(reviewer1): shutil.which("cys")만
    # 쓰면 PATH 바이너리와 GUI 번들 sidecar 가 달라 오진할 수 있어, **번들 sidecar 후보를 우선 탐지**하고
    # PATH 는 폴백으로 두며, **실제 검사한 경로를 메시지에 명시**한다(스큐 시 진단 가능). WARN 전용(차단 금지).
    def c65_drain_verify(self):
        cid = "C65.drain-verify"
        if self.skipped(cid):
            return
        # 번들 sidecar 우선(GUI 실제 사용 바이너리) → CYS_BIN(env) → PATH 순 후보. 첫 존재 파일 채택.
        candidates = []
        if os.environ.get("CYS_BIN"):
            candidates.append(os.environ["CYS_BIN"])
        candidates += [
            "/Applications/cys.app/Contents/MacOS/cys",
            os.path.expanduser("~/Applications/cys.app/Contents/MacOS/cys"),
            os.path.expanduser("~/.local/bin/cys"),
            "/opt/homebrew/bin/cys",
        ]
        w = shutil.which("cys")
        if w:
            candidates.append(w)
        cys = next((c for c in candidates if c and os.path.isfile(c)), None)
        if not cys:
            self.add(cid, WARN, "cys 바이너리 미발견(번들 sidecar·CYS_BIN·PATH 모두) — drain --verify 능력 확인 불가")
            return
        try:
            r = subprocess.run([cys, "drain", "--help"], capture_output=True, text=True, timeout=15)
            help_text = (r.stdout or "") + (r.stderr or "")
            if "--verify" in help_text:
                self.add(cid, PASS, "cys drain --verify 지원 (GUI 저장후재시작 검증 흐름 가용 · 검사=%s)" % cys)
            else:
                self.add(cid, WARN,
                         "cys drain --verify 미지원(구버전 스큐) — GUI가 plain drain 으로 폴백. "
                         "최신 cys 로 갱신 권장(rotate/재설치) · 검사=%s" % cys)
        except Exception as e:
            self.add(cid, WARN, "cys drain --help 실행 실패(%s) — 능력 확인 불가 · 검사=%s" % (e, cys))

    # ── C66 스킬보드 카탈로그 무결성 (WARN-only·부트 비차단·--fix 무동작=카탈로그는 오너 주권) ──
    def c66_board_catalog(self):
        cid = "C66.board-catalog"
        if self.skipped(cid):
            return
        try:
            data = json.load(open(os.path.join(pack_dir(), "board-catalog.json"), encoding="utf-8"))
        except (OSError, ValueError) as e:
            self.add(cid, WARN, "board-catalog.json 읽기/파싱 실패(%s) — 카탈로그 무결성 확인 불가" % e)
            return
        if not isinstance(data, dict):
            self.add(cid, WARN, "board-catalog.json 스키마 예상 밖(객체 아님) — 무결성 확인 불가")
            return
        names = []
        for dom in data.get("domains", []):
            if isinstance(dom, dict):
                for s in dom.get("skills", []):
                    if isinstance(s, dict) and s.get("name"):
                        names.append(s["name"])
        for act in data.get("actions", []):
            if isinstance(act, dict) and act.get("name"):
                names.append(act["name"])
        names = list(dict.fromkeys(names))  # 중복 제거·순서 보존
        # 설치 루트 = pack/skills + ~/.claude*/skills — 보드 카탈로그 스킬은 claude 프로필
        # skills에 설치돼 있다(실측 2026-07-16: pack 단일 루트는 설치 스킬을 미설치로 오탐).
        roots = [os.path.join(pack_dir(), "skills")]
        home = os.path.expanduser("~")
        try:
            for nm in sorted(os.listdir(home)):
                if nm == ".claude" or nm.startswith(".claude-"):
                    d = os.path.join(home, nm, "skills")
                    if os.path.isdir(d):
                        roots.append(d)
        except OSError:
            pass
        missing = [n for n in names
                   if not any(os.path.isdir(os.path.join(r, n)) for r in roots)]
        if missing:
            self.add(cid, WARN, "카탈로그 참조 스킬 미설치(전 루트 부재 %d종): %s"
                     % (len(missing), ", ".join(missing)))
        else:
            self.add(cid, PASS, "board-catalog 참조 스킬 %d종 전부 설치됨" % len(names))

    # ── C67 학습 기록 배선 (WARN-only·부트 비차단·--fix 무동작) ──
    def c67_learn_wiring(self):
        cid = "C67.learn-wiring"
        if self.skipped(cid):
            return
        p = os.path.join(os.path.expanduser("~"), ".cys", "state", "learn", "state.json")
        msg = "학습 기록 미배선 — RSI 라운드가 cys learn-checkpoint로 push하면 CC 학습 탭에 표시"
        if not os.path.isfile(p):
            self.add(cid, WARN, "%s (%s 없음)" % (msg, p))
            return
        try:
            age_days = (time.time() - os.path.getmtime(p)) / 86400.0
        except OSError as e:
            self.add(cid, WARN, "%s (mtime 조회 실패: %s)" % (msg, e))
            return
        if age_days > 30:
            self.add(cid, WARN, "%s (마지막 갱신 %.0f일 전)" % (msg, age_days))
        else:
            self.add(cid, PASS, "학습 기록 배선됨 (마지막 갱신 %.1f일 전)" % age_days)

    # ── C69 하트비트 게이트 대장 최신성 (WARN-only·부트 비차단 — 데드맨 2차 · DESIGN §C6) ──
    # 게이트가 5분마다 대장에 append하므로 대장 mtime이 ≤15분이면 게이트 생존이다. 정체는
    # ①게이트 사망(스크립트/인터프리터 부재) ②kill-switch pause(정상) 둘 중 하나 — pause면
    # 스케줄 발화가 동결이라 정체가 정상이므로 `cys gate-check` exit 4=pause면 skip한다(오경보 금지).
    # ★비차단 필수: preflight는 부트 시퀀스 ⓪라 이 검사의 버그가 부트를 막아선 안 된다 → 전부 WARN.
    def c69_gate_ledger(self):
        cid = "C69.gate-ledger"
        if self.skipped(cid):
            return
        # pause 존중 — pause 중이면 대장 정체가 정상이므로 검사 자체를 건너뛴다(gate-check exit 4).
        cys = shutil.which("cys") or os.environ.get("CYS_BIN")
        if cys:
            try:
                r = subprocess.run([cys, "gate-check"], capture_output=True, timeout=10)
                if r.returncode == 4:
                    self.add(cid, PASS, "kill-switch pause 중 — 게이트 대장 최신성 검사 skip(정체 정상)")
                    return
            except Exception:
                pass  # gate-check 실패는 무시하고 대장 검사 계속(비차단)
        # ★B7 파생 site ⓑ — 레인별 대장을 본다. 종전에는 env/기본값만 해석해, 격리 이후 부서
        #   레인이 **본사 대장**을 감시하는 split-brain 이 됐다(idle-standby-v5 D2 ⓑ 실측).
        ledger = os.path.join(gate_state_dir_for_pack(), "ledger.jsonl")
        if os.environ.get(GATE_STATE_ENV):
            ledger = os.path.join(os.environ[GATE_STATE_ENV], "ledger.jsonl")
        if not os.path.isfile(ledger):
            self.add(cid, WARN, "게이트 대장 부재(%s) — 델타게이트 미배선/미가동일 수 있음(비차단)" % ledger)
            return
        try:
            age_min = (time.time() - os.path.getmtime(ledger)) / 60.0
        except OSError as e:
            self.add(cid, WARN, "게이트 대장 mtime 조회 실패(%s) — 최신성 확인 불가" % e)
            return
        if age_min > 15:
            self.add(cid, WARN,
                     "게이트 대장 정체 %.0f분(>15분) — 게이트 사망 의심(스크립트/인터프리터 점검). "
                     "pause가 아니면 CSO 점검 필요" % age_min)
        else:
            self.add(cid, PASS, "게이트 대장 최신(%.1f분 전) — 델타게이트 생존" % age_min)

    # ── C70 launchd 스테일 잡 탐지 (macOS · 탐지·보고 전용 — 자동 수정 절대 금지) ──
    # W2(DESIGN_triple-fix_20260718 §W2): 본부 데몬 launchd 잡이 ①penalty box(재시작 폭풍
    # 억제) ②last exit code=78(EX_CONFIG — plist config 부적합) ③program 경로가 소멸한 backup
    # 번들로 유추 채택(inferred stale — /Applications/cys.app 아님) 상태에 빠지면 오피스·승인
    # 채널이 조용히 죽는다. 이 체크는 그 3징후를 `launchctl print` 출력에서 탐지해 WARN 보고만
    # 한다(WARN은 exit 0 불변 — 부트 게이트 NOT READY 미차단, 탐지·보고 전용 규약).
    # ★--fix 에서도 이 체크는 절대 자동 수정하지 않는다(bootout·bootstrap 재부트스트랩을
    # 자동화하면 매 업데이트 세션마다 sibling 스폰·데몬 대학살이 파생된다 — 2R 판정). 수리는
    # 오너 지정 정지창의 CSO 집행 런북(§W2)으로만. launchctl 부재·권한 실패·잡 미등록은
    # 오탐 방지로 SKIP(스테일이 아니라 판정 불가 상태).
    def c70_launchd_job(self):
        cid = "C70.launchd-job"
        if self.skipped(cid):
            return
        if sys.platform != "darwin":
            self.add(cid, SKIP, "macOS 아님 — launchd 미해당")
            return
        launchctl = shutil.which("launchctl")
        if not launchctl:
            self.add(cid, SKIP, "launchctl 부재 — 판정 불가")
            return
        label = "gui/%d/com.cysjavis.cysd" % os.getuid()
        try:
            r = subprocess.run([launchctl, "print", label],
                               capture_output=True, timeout=10, env=_utf8_env())
        except Exception as e:
            self.add(cid, SKIP, "launchctl print 실행 실패 — 판정 불가: %s" % e)
            return
        out = ((r.stdout or b"").decode("utf-8", "replace")
               + (r.stderr or b"").decode("utf-8", "replace"))
        # 잡 미등록(bootstrap 안 됨)·권한 거부는 스테일이 아니라 '판정 불가'다 → SKIP(오탐 금지).
        # launchctl print 는 미등록 잡에 nonzero + "Could not find service" 를 낸다(잡 이름 미출현).
        if r.returncode != 0 and "com.cysjavis.cysd" not in out:
            self.add(cid, SKIP,
                     "launchd 잡 미등록/조회 불가(rc=%d) — 스테일 판정 대상 아님" % r.returncode)
            return
        low = out.lower()
        symptoms = []
        if "penalty box" in low:
            symptoms.append("penalty box(재시작 억제)")
        # `last exit code = 78: EX_CONFIG` (공백 변주 허용, 78 뒤는 ':' 등 비단어 경계).
        if re.search(r"last exit code\s*=\s*78\b", low):
            symptoms.append("last exit code=78(EX_CONFIG)")
        # program 경로가 /Applications/cys.app 이 아니면 소멸 번들 유추 채택(inferred stale).
        m = re.search(r"^\s*program\s*=\s*(.+?)\s*$", out, re.MULTILINE | re.IGNORECASE)
        if m and "/Applications/cys.app/" not in m.group(1):
            symptoms.append("program=%s (inferred stale — /Applications/cys.app 아님)"
                            % m.group(1).strip())
        if symptoms:
            self.add(cid, WARN,
                     "launchd 스테일 징후 — %s · 수리는 오너 정지창 CSO 런북(DESIGN §W2)으로만, "
                     "자동 재부트스트랩 금지(탐지·보고 전용)" % "; ".join(symptoms))
        else:
            self.add(cid, PASS,
                     "launchd 잡 정상(penalty box·EX_CONFIG·inferred stale 징후 없음)")

    # ── C71 어댑터 스키마 완결성 (B20) — 결손 키의 **의미**를 말한다(무음 퇴화 차단) ──
    # C05 는 "cmd 존재 + 역할 매핑"만 본다. 그런데 어댑터의 선택 키 결손은 조용히 기능을
    # 퇴화시킨다: ready_marker 없음→readiness 를 시간 폴백으로 때움(느리고 부정확),
    # clear_cmd 없음→cycle-agent 가 컨텍스트를 못 비움, resume_arg 없음→restore 가 대화기억
    # 없이 fresh 기동(=세션 만료 시 맥락 소실), approval_patterns 없음→승인 프롬프트 무감지.
    # 어느 것도 그 자체로 오류가 아니다(grok 처럼 원래 없는 게 정상인 어댑터가 있다) → 기본
    # PASS + detail 에 결손 의미를 명시하고, **resume_arg 결손만 WARN** 으로 올린다(복원 계층의
    # 맥락 소실이 가장 비싸고 사용자가 모르는 채 겪는 손실이라서). WARN 남발은 신호를 죽인다.
    OPTIONAL_KEY_MEANING = (
        ("ready_marker", "ready_marker 없음→시간 폴백 사용", False),
        ("clear_cmd", "clear_cmd 없음→cycle-agent 미지원(clear 불가)", False),
        ("resume_arg", "resume_arg 없음→session resume 미지원(restore는 fresh 기동)", True),
        ("approval_patterns", "approval_patterns 없음→승인 프롬프트 자동감지 없음", False),
    )
    # ★U-12(2026-08-23): 3 = `first_run_gates` 봉투 추가분. 목록은 **누적**이다 — 구 스키마
    #   기계(사용자가 수정해 둔 _schema=2 디스크본)를 미지 버전으로 오경보하지 않기 위해
    #   1·2 를 남긴다. 판정 자체는 종전과 동일(알려진 값이면 무경고 · 미지면 WARN).
    KNOWN_AGENTS_SCHEMA = (1, 2, 3)

    def c71_agents_schema(self):
        cid = "C71.agents-schema"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "agents.json")
        try:
            data = json.load(open(p, encoding="utf-8"))
        except (OSError, ValueError) as e:
            # 부재·손상 수리는 C05 의 책임(백업·복원 경로 보유) — 여기서 중복 수리하지 않는다.
            self.add(cid, SKIP, "agents.json 로드 불가(%s) — C05 먼저 해결" % e)
            return
        if not isinstance(data, dict):
            self.add(cid, FAIL, "agents.json 최상위가 객체가 아니다(%s)" % type(data).__name__)
            return
        fails, warns, notes = [], [], []
        # _schema: 없으면 구 파일 → 무시(하위호환). 있으면 알려진 값인지만 본다.
        schema = data.get("_schema")
        if schema is not None and schema not in self.KNOWN_AGENTS_SCHEMA:
            warns.append("미지 agents.json 스키마 버전 %s — 팩 갱신 확인 권장" % schema)
        agents = [(k, v) for k, v in sorted(data.items()) if not k.startswith("_")]
        if not agents:
            self.add(cid, FAIL, "어댑터 항목 0개 — agents.json 이 메타 키만 갖고 있다")
            return
        for name, spec in agents:
            if not isinstance(spec, dict):
                fails.append("%s: 어댑터 정의가 객체가 아니다(%s)" % (name, type(spec).__name__))
                continue
            cmd = spec.get("cmd")
            if not (isinstance(cmd, str) and cmd.strip()):
                fails.append("%s: 필수 키 cmd 누락/빈 값 — 기동 불가" % name)
            missing = [(key, why, warn) for key, why, warn in self.OPTIONAL_KEY_MEANING
                       if key not in spec]
            for key, why, warn in missing:
                (warns if warn else notes).append("%s: %s" % (name, why))
        if fails:
            self.add(cid, FAIL, "; ".join(fails + warns + notes))
        elif warns:
            self.add(cid, WARN, "; ".join(warns + notes))
        else:
            detail = "어댑터 %d종 스키마 정합(cmd 전건 존재%s)" % (
                len(agents), ", _schema=%s" % schema if schema is not None else "")
            if notes:
                detail += " · 선택 키 결손(정상 가능): " + "; ".join(notes)
            self.add(cid, PASS, detail)

    # ═══ Phase 1 Wave B2 신규 4종 (C72~C75 · DESIGN-DECISIONS §3 · 조건 18①·D4·05④·02④) ═══
    # 전부 read-only·부작용 0·self.results/planned 미참조 — report 스레드풀 안전(§5-4 규율).
    # cid 는 C72부터(C63·C64 결번 재사용 금지 — T0 §5-4).

    # C72 상호 정합 핀 — Phase1 게이트 3점 세트의 '세대 문자열'(C03 content-pin 문법 답습).
    # 핀이 있는 구성요소는 현 세대, 없는 것은 구세대다 — 어느 쪽이 구세대인지 명시(조건 18①).
    C72_TASK_PINS = (
        ("verify-spec missing(6)", "checkout exit6 게이트 stderr 고정 문면(T1)"),
        (".verify-gate-activated", "grandfather 활성 마커 상수(T1)"),
        ("EXIT_VERIFY = 6", "exit 6 배정(D9)"),
        ("--demote-calibrated", "calibrated 강등 내부 경로(B2 · 조건 07②)"),
    )
    C72_GUARD_PINS = (
        ("SKIPPED_NO_SPEC", "판정 enum 10종 세대(B1)"),
        (".guard-claim.", "태스크 바인딩 파일 계약(B1 §2-1)"),
        ("verify-budget", "verifier tax 샤드 소비부(B1/B2 §3)"),
        ("_claim_liveness", "claim 생존 판정=태스크 상태 기반(E2-1 · R-02 교정 세대)"),
        (".guard-silence.", "침묵 탐지기 상태 파일(E2-1c — 무장≠발동 표면화)"),
        ("budget-init", "verifier tax 활성화 절차(E2-2 · R-07)"),
    )
    C72_SCHEMA_MODE_ENUM = ["command", "procedural", "probe", "waiver"]
    _C72_RESERVED_RE = re.compile(r"RESERVED_ID_SUFFIXES\s*=\s*\(([^)]*)\)")

    def c72_phase1_gate_set(self):
        cid = "C72.phase1-gate-set"
        if self.skipped(cid):
            return
        bin_dir = os.path.join(pack_dir(), "bin")
        task_p = os.path.join(bin_dir, "javis_task.py")
        guard_p = os.path.join(bin_dir, "javis_completion_guard.py")
        schema_p = os.path.join(pack_dir(), "schemas", "verify_spec_schema.json")
        probs = []

        def _read(p):
            try:
                with open(p, encoding="utf-8") as f:
                    return f.read()
            except OSError:
                return None

        task_txt = _read(task_p)
        if task_txt is None:
            probs.append("javis_task.py 판독 불가(%s) — 게이트 본체 소실" % task_p)
        else:
            for pin, why in self.C72_TASK_PINS:
                if pin not in task_txt:
                    probs.append("javis_task.py 구세대 — 마커 %r 부재(%s)" % (pin, why))
        guard_txt = _read(guard_p)
        if guard_txt is None:
            probs.append("javis_completion_guard.py 부재/판독 불가(%s) — guard 구성요소 "
                         "구세대/미배포(3중 부재 회귀 · 조건 18①)" % guard_p)
        else:
            for pin, why in self.C72_GUARD_PINS:
                if pin not in guard_txt:
                    probs.append("javis_completion_guard.py 구세대 — 마커 %r 부재(%s)"
                                 % (pin, why))
        try:
            with open(schema_p, encoding="utf-8") as f:
                schema_doc = json.load(f)
            if not isinstance(schema_doc, dict):
                probs.append("verify_spec_schema.json 구조 손상(최상위 비객체)")
            elif schema_doc.get("MODE_ENUM") != self.C72_SCHEMA_MODE_ENUM:
                probs.append("verify_spec_schema.json 구세대 — MODE_ENUM %r ≠ 기대 %r"
                             % (schema_doc.get("MODE_ENUM"), self.C72_SCHEMA_MODE_ENUM))
        except (OSError, ValueError) as e:
            probs.append("verify_spec_schema.json 부재/손상(%s) — %s" % (schema_p, e))
        # 상호 해시급 정합: 예약 접미 튜플이 두 파일에서 동일해야 한다(F5 네임스페이스 계약).
        if task_txt is not None and guard_txt is not None:
            mt = self._C72_RESERVED_RE.search(task_txt)
            mg = self._C72_RESERVED_RE.search(guard_txt)
            norm = lambda m: re.sub(r"\s+", "", m.group(1)) if m else None  # noqa: E731
            if norm(mt) is None or norm(mg) is None or norm(mt) != norm(mg):
                probs.append("javis_task↔guard RESERVED_ID_SUFFIXES 불일치(%r vs %r) — "
                             "두 구성요소가 서로 다른 세대(찢김)"
                             % (norm(mt), norm(mg)))
        if probs:
            self.add(cid, FAIL, " | ".join(probs) + " — 팩 원자쌍 재배포로 세대 일치 필요")
            return
        # 찢김 없음 — 운영 마커 표면화(WARN 층): guard-disarmed·stale CYCLE(TTL 초과).
        warns = []
        root = os.environ.get("JAVIS_ROOT") or os.getcwd()
        tasks_dir = os.path.join(root, "_round", "tasks")
        try:
            names = os.listdir(tasks_dir)
        except OSError:
            names = []
        disarmed = sorted(n[:-len(".guard-disarmed")] for n in names
                          if n.endswith(".guard-disarmed"))
        if disarmed:
            warns.append("guard-disarmed %d건(회로차단기 개방 — 재무장=master 마커 삭제): %s"
                         % (len(disarmed), ", ".join(disarmed[:8])))
        # G7e: TTL 은 guard 모듈 상수를 흡수(C75 형제 import 전례) — 수기 복제(30*60)는
        # guard 측 TTL 변경 시 조용히 어긋나는 드리프트 창이라 제거. import 불가 = 1800 폴백.
        try:
            import javis_completion_guard as _jcg
            cycle_ttl = getattr(_jcg, "CYCLE_TTL_SEC", 1800)
        except Exception:  # noqa: BLE001 — 부재/찢김은 위 guard 핀 층이 이미 FAIL 로 표면화
            cycle_ttl = 1800
        stale_cyc = []
        for n in names:
            if not n.startswith("CYCLE_IN_PROGRESS."):
                continue
            with contextlib.suppress(OSError):
                age = time.time() - os.stat(os.path.join(tasks_dir, n)).st_mtime
                if age > cycle_ttl:
                    stale_cyc.append("%s(%.0f분)" % (n, age / 60))
        if stale_cyc:
            warns.append("stale CYCLE 마커(TTL %d분 초과 — 고아 clear 흔적·guard 는 무시 진행): %s"
                         % (cycle_ttl // 60, ", ".join(sorted(stale_cyc)[:8])))
        # ★E2-1(c) 침묵 탐지기 표면화 — guard 런타임 경보의 **부트 짝**.
        #   "무장했는데 연속 N회 아무것도 검증하지 않았다"는 상태는 런타임에서는 무음이고
        #   정상 동작과 구분되지 않는다(R-02 침묵형 실패). guard 가 남긴 상태 파일을 부트에서
        #   읽어 WARN 으로 올린다 — 경보 코얼레싱(6h)으로 런타임 알림이 이미 삼켜진 뒤에도
        #   부트 진단에는 남는다.
        silent = []
        for n in names:
            if not (n.startswith(".guard-silence.") and n.endswith(".json")):
                continue
            try:
                with open(os.path.join(tasks_dir, n), encoding="utf-8") as f:
                    rec = json.load(f)
            except (OSError, ValueError):
                continue
            if not isinstance(rec, dict):
                continue
            streak = int(rec.get("streak", 0) or 0)
            thr = int(rec.get("threshold", 0) or 0) or 5
            if streak >= thr:
                silent.append("%s(연속 %d회·사유 %s·최근 %s)"
                              % (n[len(".guard-silence."):-len(".json")], streak,
                                 rec.get("last_reason"), rec.get("last_ts")))
        if silent:
            warns.append("guard 침묵 지속(무장인데 검증 0회 — 무장≠발동): %s "
                         "· claim 소유/바인딩 확인(javis_task checkout --claim-surface"
                         " <워커 sid> [--claim-pid <워커 pid>])" % ", ".join(sorted(silent)[:8]))
        # ★E2-2(R-07) verifier tax 활성/비활성 1줄 — 정보성이지만 **말하게** 만든다.
        #   부재=무제한 통과(기본 OFF)라 "압력이 없다"는 관측은 tax 가 켜져 있을 때만
        #   의미가 있다. 비활성 상태에서 샤드를 관찰하라는 지시는 영원히 생기지 않는 파일을
        #   보라는 말이고, 그 침묵은 '정상'으로 오독된다.
        budget_dir = os.path.join(root, "_round", "verify-budget")
        tax_active = os.path.isdir(budget_dir)
        tax_line = ("verifier tax: %s"
                    % ("활성(policy.json %s)"
                       % ("있음" if os.path.isfile(os.path.join(budget_dir, "policy.json"))
                          else "없음 — 기본 60회/300초")
                       if tax_active else "비활성(OFF · 무제한 통과) — 활성화는 "
                                          "`javis_completion_guard.py budget-init`"))
        if os.environ.get("CYS_COMPLETION_GUARD") == "1" and not tax_active:
            warns.append("무장 pane 인데 " + tax_line
                         + " · 이 상태에서 verify-budget 샤드는 생성되지 않는다(관찰 불가)")
        # G6(성찰 2): 무장 env 조합 점검 — 이 프로세스가 무장 pane 에서 돌 때만 의미(평시
        # master 프리플라이트는 비무장 → 무발화). 조합 = cap 미설정 ∧ self-cap 0: grill 블록
        # K 가 하네스 Stop 지갑(기본 8+1=9)을 선점하면 guard escalation(N=3) 도달 전 소진
        # (9−K<N 선점 산술 — guard _warn_armed_combo 런타임 경보의 프리플라이트 짝).
        if (os.environ.get("CYS_COMPLETION_GUARD") == "1"
                and not os.environ.get("CLAUDE_CODE_STOP_HOOK_BLOCK_CAP", "").strip()
                and os.environ.get("GRILL_STOP_SELF_CAP", "0").strip() in ("", "0")):
            warns.append("무장 env 조합 위험(G6 — 선점 산술 9−K<N): CYS_COMPLETION_GUARD=1 ∧ "
                         "CLAUDE_CODE_STOP_HOOK_BLOCK_CAP 미설정 ∧ GRILL_STOP_SELF_CAP=0 — "
                         "cap 상향+self-cap 활성 원자 조합 적용 요망(OT-2)")
        if warns:
            self.add(cid, WARN, "게이트 3점 세대 정합 OK · " + " | ".join(warns)
                     + " · " + tax_line)
        else:
            self.add(cid, PASS, "Phase1 게이트 3점 세트 세대 정합(javis_task 마커 %d·guard 마커 "
                                "%d·schema MODE_ENUM·예약 접미 상호 일치) · disarmed 0 · "
                                "stale CYCLE 0 · guard 침묵 0 · %s"
                                % (len(self.C72_TASK_PINS), len(self.C72_GUARD_PINS), tax_line))

    # C73 — 워커 프로필 settings 훅 무결성(D4: 블록 압박 하 모델의 훅 자가제거 실측 대응).
    # 기대 집합 마커 = $PACK/state/guard-hook-expected.json (없으면 SKIP — OT-2 등록 전 상태).
    # 스키마: {"schema_version":1, "profiles":[{"settings":"<abs settings.json>",
    #          "sha256":"<hex·선택>", "must_contain":["<부분문자열>", ...]}]}
    def c73_guard_hook_integrity(self):
        cid = "C73.guard-hook-integrity"
        if self.skipped(cid):
            return
        exp_p = os.path.join(pack_dir(), "state", "guard-hook-expected.json")
        if not os.path.isfile(exp_p):
            self.add(cid, SKIP, "기대 집합 마커 부재(%s) — guard 훅 라이브 등록 전(OT-2) 정상"
                     % exp_p)
            return
        try:
            with open(exp_p, encoding="utf-8") as f:
                exp = json.load(f)
            profiles = exp["profiles"]
            assert isinstance(profiles, list)
        except Exception as e:  # noqa: BLE001 — 마커 손상은 WARN(감시 불능 표면화·부트 비차단)
            self.add(cid, WARN, "기대 집합 마커 손상(%s) — 훅 무결성 감시 불능: %s" % (exp_p, e))
            return
        # G7g 판정 서열: must_contain(등록 실존)이 1급 신호 — 통과 시 sha 불일치는 WARN 강등.
        # settings 는 정당 사유(권한 추가·모델 변경 등)로도 바뀌는 파일이라 sha 단독 FAIL 은
        # 오경보 양산 경로다. ★갱신 책임 명문: settings 를 정당 변경한 주체가 같은 변경에서
        # guard-hook-expected.json 의 sha256 을 재계산·갱신할 책임을 진다(방치 = WARN 잔존).
        probs, warns, oks = [], [], 0
        import hashlib
        for ent in profiles:
            if not isinstance(ent, dict) or not ent.get("settings"):
                probs.append("마커 항목 형식 오류: %r" % (ent,))
                continue
            sp = os.path.expanduser(str(ent["settings"]))
            try:
                raw = open(sp, "rb").read()
            except OSError:
                probs.append("%s: settings 부재/판독 불가 — 등록 소실 의심(D4)" % sp)
                continue
            text = raw.decode("utf-8", errors="replace")
            missing = [m for m in (ent.get("must_contain") or []) if str(m) not in text]
            if missing:
                probs.append("%s: 등록 부재 %s — 훅 자가제거 의심(D4 실측: 블록 압박 하 "
                             "update-config 자발 시도)" % (sp, ", ".join(map(repr, missing))))
                continue
            want_sha = str(ent.get("sha256") or "").lower()
            if want_sha:
                got = hashlib.sha256(raw).hexdigest()
                if got != want_sha:
                    warns.append("%s: 체크섬 불일치(기대 %s… ≠ 실측 %s…) — must_contain 은 "
                                 "전부 존재: 정당 설정 변경 가능성 → WARN 강등(G7g · 변경 "
                                 "주체가 기대 마커 sha256 갱신 책임)" % (sp, want_sha[:12],
                                                                        got[:12]))
                    continue
            oks += 1
        if probs:
            self.add(cid, FAIL, " | ".join(probs + warns))
        elif warns:
            self.add(cid, WARN, " | ".join(warns))
        else:
            self.add(cid, PASS, "워커 프로필 settings 훅 무결성 OK(%d/%d 프로필 — 등록 존재·"
                                "체크섬 일치)" % (oks, len(profiles)))

    # C74 — brief 게이트 2경로 배선(조건 05④·21③·28): ①checkout --brief hard 경로 코드
    # ②인세션 경고 훅(brief-lint-warn.sh) 등록. 등록 대상표 = env CYS_BRIEF_WARN_PROFILES
    # (콤마) 또는 $PACK/state/brief-warn-expected.txt(프로필 dotdir basename 목록) — 표 부재
    # 시 전 프로필 스캔으로 존재 여부만 본다. 미등록 = WARN(라이브 등록은 OT-2 — FAIL 아님).
    def c74_brief_gate_paths(self):
        cid = "C74.brief-gate-paths"
        if self.skipped(cid):
            return
        probs, warns = [], []
        task_p = os.path.join(pack_dir(), "bin", "javis_task.py")
        lint_p = os.path.join(pack_dir(), "bin", "javis_brief_lint.py")
        hook_p = os.path.join(pack_dir(), "hooks", "brief-lint-warn.sh")
        try:
            task_txt = open(task_p, encoding="utf-8").read()
        except OSError:
            task_txt = ""
        if not ("--brief" in task_txt and "javis_brief_lint" in task_txt):
            probs.append("checkout --brief hard 경로 코드 부재(javis_task.py grep — T1 회귀)")
        if not os.path.isfile(lint_p):
            probs.append("javis_brief_lint.py 부재 — 두 경로 모두 엔진 소실")
        if not os.path.isfile(hook_p):
            probs.append("hooks/brief-lint-warn.sh 부재 — 인세션 경고 경로 실체 소실")
        # 등록 대상표(분리 독립 — guard 대상표와 별개 · §1-3 R1-contract)
        env_names = os.environ.get("CYS_BRIEF_WARN_PROFILES", "")
        names = tuple(x.strip() for x in env_names.split(",") if x.strip())
        if not names:
            with contextlib.suppress(OSError):
                with open(os.path.join(pack_dir(), "state", "brief-warn-expected.txt"),
                          encoding="utf-8") as f:
                    names = tuple(ln.strip() for ln in f
                                  if ln.strip() and not ln.startswith("#"))
        settings = discover_claude_settings()

        def _registered(sp):
            try:
                return "brief-lint-warn.sh" in open(sp, encoding="utf-8").read()
            except OSError:
                return False

        if names:
            targets = [s for s in settings
                       if os.path.basename(os.path.dirname(s)) in names]
            miss = [os.path.basename(os.path.dirname(s)) for s in targets
                    if not _registered(s)]
            absent = [n for n in names if n not in
                      {os.path.basename(os.path.dirname(s)) for s in settings}]
            if miss or absent:
                warns.append("경고 훅 미등록(대상표 기준): %s — 라이브 등록은 OT-2 승인 라인"
                             % ", ".join(sorted(set(miss) | set(absent))))
        else:
            reg = [s for s in settings if _registered(s)]
            if not reg:
                warns.append("경고 훅 등록 0 프로필(대상표 미공급·전 프로필 스캔) — "
                             "인세션 경고 경로 무음 비활성(라이브 등록=OT-2)")
        if probs:
            self.add(cid, FAIL, " | ".join(probs + warns))
        elif warns:
            self.add(cid, WARN, "hard 경로 코드·엔진·훅 실체 OK · " + " | ".join(warns))
        else:
            self.add(cid, PASS, "brief 게이트 2경로 OK(checkout --brief 코드 + 경고 훅 등록)")

    # C75 — 배포 직전 열린+무spec 태스크 재스캔(조건 02④ — U22 스냅샷 재사용 금지·매 실행
    # 재실측). 라이브 _round/tasks **읽기 전용**. 5건 초과 = strict 승격 부적격 FAIL.
    # T1 descope 이연 2건 수용처: grandfathered 목록(02② HUD 목록 대체 표면화) +
    # 유예-중 waiver 목록(16③ 매일 경보 대체 표면화).
    def c75_verify_spec_rescan(self):
        cid = "C75.verify-spec-rescan"
        if self.skipped(cid):
            return
        root = os.environ.get("JAVIS_ROOT") or os.getcwd()
        tasks_dir = os.path.join(root, "_round", "tasks")
        if not os.path.isdir(tasks_dir):
            self.add(cid, SKIP, "%s 부재 — 태스크 보드 없는 환경" % tasks_dir)
            return
        try:
            import javis_task as _jt  # 형제 모듈 — OPEN_STATUSES·waiver 판정 재사용(읽기 전용)
        except Exception as e:  # noqa: BLE001
            self.add(cid, WARN, "javis_task import 실패(%s) — 재스캔 불능(버전 찢김 의심)" % e)
            return
        open_statuses = set(getattr(_jt, "OPEN_STATUSES", ["backlog", "todo", "in_progress",
                                                           "in_review", "blocked"]))
        reserved = tuple(getattr(_jt, "RESERVED_FILE_SUFFIXES",
                                 (".guard.json", ".esc-bundle.json")))
        no_spec, grandfathered, waiver_grace, parse_fail = [], [], [], 0
        gf_closed = 0
        try:
            names = sorted(os.listdir(tasks_dir))
        except OSError as e:
            self.add(cid, WARN, "태스크 디렉터리 판독 불가(%s)" % e)
            return
        for n in names:
            if not n.endswith(".json") or n.startswith(".") or n.endswith(reserved):
                continue
            try:
                with open(os.path.join(tasks_dir, n), encoding="utf-8") as f:
                    t = json.load(f)
            except (OSError, ValueError):
                parse_fail += 1
                continue
            if not isinstance(t, dict) or "status" not in t:
                continue
            tid = t.get("id") or n[:-5]
            is_open = t.get("status") in open_statuses
            gf = t.get("grandfathered") is True
            if gf:
                # G3: grandfathered 목록은 **열린 태스크 한정** — closed(done·cancelled)는
                # 승격 리스크 표면이 아니므로 '(closed N건)' 분리 집계만(목록 소음 제거).
                if is_open:
                    grandfathered.append(tid)
                else:
                    gf_closed += 1
            spec = t.get("verify_spec")
            if is_open and (not isinstance(spec, dict) or not spec):
                # G3(과계상 수리): grandfathered=true 는 초과 계수에서 **제외** — 게이트
                # 설계(§1-2)가 이미 WARN 통과로 면제한 구세대분이라 strict 승격 부적격
                # 산술(>5 FAIL)에 재계상하면 이중 계상이다. 별도 목록 표면화는 유지(위).
                if not gf:
                    no_spec.append(tid)
            elif is_open and isinstance(spec, dict) and spec.get("mode") == "waiver":
                with contextlib.suppress(Exception):
                    rec = _jt._load_waiver(spec.get("waiver_ref") or "")
                    if rec is not None and _jt._waiver_state(rec)[0] == "grace":
                        waiver_grace.append(tid)
        detail = ("열린+무spec(비-grandfathered) %d건%s · grandfathered(열린) %d건%s%s · "
                  "waiver 유예-중 %d건%s%s"
                  % (len(no_spec),
                     "(%s)" % ", ".join(no_spec[:10]) if no_spec else "",
                     len(grandfathered),
                     "(%s)" % ", ".join(grandfathered[:10]) if grandfathered else "",
                     "(closed %d건 분리 집계)" % gf_closed if gf_closed else "",
                     len(waiver_grace),
                     "(%s)" % ", ".join(waiver_grace[:10]) if waiver_grace else "",
                     " · 파싱 실패 %d건" % parse_fail if parse_fail else ""))
        # ★E1-4(BLOCKER F12/R-25 · 무게이트 발효 강등): 심각도는 **strict 를 실제로 쫓고 있을
        #   때만** FAIL 이다. 근거(실측): 이 검사가 무조건 FAIL 을 내면, 팩 install 직후의
        #   라이브 보드(T0 실측 33건 · verify_spec 은 아직 0건)에서 부트 진단이 즉시 FAIL 로
        #   전환된다 — 오너 승인 접점 없이 `preflight` 가 READY→NOT READY 로 뒤집힌다.
        #   게다가 그 시점 게이트 모드는 출하 기본 `warn` 이라 **막고 있는 것이 아무것도 없다**:
        #   "strict 승격 부적격"은 참이지만 승격을 시도하지도 않은 상태의 거짓 경보다.
        #   판정: strict 추적 중(JAVIS_VERIFY_GATE=strict ∨ grandfather 마커 존재) → FAIL
        #         그 밖(warn/off = 출하 기본) → WARN(정보성 · 종료코드는 FAIL 수만 본다).
        strict_pursued = (os.environ.get("JAVIS_VERIFY_GATE", "").strip().lower() == "strict"
                          or os.path.isfile(os.path.join(tasks_dir,
                                                         getattr(_jt, "GATE_MARKER", ""))))
        if len(no_spec) > 5:
            if strict_pursued:
                self.add(cid, FAIL, "strict 승격 부적격(조건 02④ — 열린+무spec %d건 > 5): "
                                    "grandfather 완료 전 배포 금지 · %s"
                         % (len(no_spec), detail))
            else:
                self.add(cid, WARN, "strict 승격 부적격(조건 02④ — 열린+무spec %d건 > 5) — "
                                    "다만 현재 게이트는 출하 기본(warn/off)이라 차단 중인 것은 "
                                    "없다. strict 승격 전 해소 필요(그때 FAIL 로 승격) · %s"
                         % (len(no_spec), detail))
        else:
            self.add(cid, PASS, "strict 승격 재스캔 OK(≤5) — " + detail)

    # ── C76 앱 번들 코드서명 봉인 (2026-08-01 실사고 M3 — 무증상 보균 탐지·WARN-only) ──
    #
    # 실사고: 번들 안 Python 런타임이 **실행 중** Contents/Resources/runtime/python/lib/
    # python3.12/**/__pycache__/*.pyc 를 번들 *안에* 만들어 codesign 봉인(sealed resources)을
    # 스스로 깼다. 로컬은 이미 실행 중이라 증상이 0이다(무증상 보균) — 그러나 그 번들을
    # 사용자가 브라우저로 받아 설치하면 quarantine 이 붙고 첫 실행 Gatekeeper 전체 재검증에서
    # "손상되었기 때문에 열 수 없습니다"로 차단된다(공증·staple 은 정상인데도). 릴리스 검증이
    # curl 사본(quarantine 없음)만 봐서 이 경로를 한 번도 재현하지 못했다.
    # ★등급 WARN 고정(부팅 비차단): 봉인 파손은 *배포 시* 사고이지 이 기계의 부팅 불능이
    #   아니다 — 여기서 FAIL 을 내면 이미 파손된 기계가 READY→NOT READY 로 뒤집혀 부팅
    #   자체가 막힌다(C75 E1-4 선례와 동일 판단). 대신 detail 이 원인 파일과 복구 절차를 말한다.
    # ★판정 불가(비 macOS·번들 미발견·codesign 부재·타임아웃)는 전부 SKIP — 거짓 경보 금지.
    # ★--fix 에서도 자동 수정 없음: 번들 안 파일 삭제는 App Management(TCC)에 막힐 수 있고,
    #   수정(modified)·누락(missing) 파손은 지워서 복구되지 않는다(통째 재설치만이 복구). 추가
    #   (added) 파손 중 C11b 가 스스로 만든 심링크만은 C11b --fix 가 unlink 로 자가치유한다
    #   (생산자=치유자) — 그 밖의 추가 파일은 정체 미상인 채 지우는 자동화가 위험해 수동 안내만.
    # ★복구 문구 2갈래(2026-08-28 실측 교정 — 종전 "지워도 added 가 missing 으로 바뀔 뿐"은
    #   added 에 대해 거짓): '추가된' 파일은 서명 목록(CodeResources)에 없던 파일이라 **그
    #   파일만 지우면 봉인이 원상 복구**된다 — 이 Mac 의 Contents/MacOS/cys-dept 심링크를 rm
    #   한 뒤 spctl accepted 실측(재설치 불요). missing 은 서명 목록에 *있는* 파일이 사라진
    #   갈래라 added 삭제로는 생기지 않는다. 수정·누락이 하나라도 섞이면 종전대로 통째 재설치
    #   만이 복구다(부분 되채움은 TCC 에 막히고 세대 혼합 번들이 될 뿐이다).
    APP_SEAL_RECOVERY = ("수정·누락 파손의 복구는 번들 통째 재설치뿐 — 새 DMG의 cys.app 을 임시 "
                         "폴더에 스테이징(ditto --rsrc --extattr --acl) 후 `mv` 로 /Applications "
                         "교체(/Applications 안 부분 수정은 App Management 보호에 막힘)")
    APP_SEAL_RECOVERY_ADDED = ("파손이 전부 '추가된 파일'(file added)이라 그 파일만 지우면 "
                               "봉인이 복구된다 — 재설치 불요(2026-08-28 실측: 번들 안 cys-dept "
                               "심링크 rm 후 spctl accepted). 삭제가 권한에 막히면(App "
                               "Management) 통째 재설치로")

    @staticmethod
    def _app_bundle_of(path):
        """실행 파일 경로에서 자기 앱 번들 루트(…/*.app)를 찾는다. Contents/Info.plist 로 진짜
        번들임을 확증(이름만 .app 인 디렉토리 오탐 차단). 번들 밖이면 None."""
        p = os.path.realpath(path)          # 심링크(/usr/local/bin/cys)를 풀어야 번들이 보인다
        while True:
            parent = os.path.dirname(p)
            if not p or parent == p:
                return None
            if p.endswith(".app") and os.path.isfile(os.path.join(p, "Contents", "Info.plist")):
                return p
            p = parent

    def _find_app_bundle(self):
        """이 기계가 실제로 쓰는 cys 의 앱 번들(심링크 해소). 없으면 표준 설치 경로. 둘 다 없으면 None.
        (분리 이유: 회귀 하네스가 라이브 /Applications 를 건드리지 않고 픽스처 번들을 주입한다.)"""
        cys = shutil.which("cys")
        if cys:
            b = self._app_bundle_of(cys)
            if b:
                return b
        if os.path.isdir("/Applications/cys.app/Contents"):
            return "/Applications/cys.app"
        return None

    # ── C77 임무 게이트 (2026-08-01 실사고 T1 — 임무 없는 부팅의 자율 착수 차단) ──
    def c77_mission_gate(self):
        """`javis_mission.py` 는 '자율 착수해도 되는가'의 **단일 판정처**다. 이 파일이 팩에서
        빠지면 `next-action` 이 영구 exit 3(fail-closed)으로 접혀 자율주행이 통째로 죽는다 —
        조용한 기능 소실이므로 존재+자기검증을 결정론으로 못 박는다(누락 시 init-pack 수리)."""
        cid = "C77.mission-gate"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_mission.py")
        if not p:
            return
        # 배선 확인: next-action 이 실제로 임무 게이트를 소비하는가(도구만 있고 미배선 차단).
        orch = os.path.join(pack_dir(), "bin", "javis_orchestra.py")
        try:
            src = open(orch, encoding="utf-8", errors="replace").read()
        except OSError as e:
            self.add(cid, FAIL, "javis_orchestra.py 판독 불가: %s" % e)
            return
        if "javis_mission" not in src or "return 3" not in src:
            self.add(cid, FAIL,
                     "next-action 이 임무 게이트를 소비하지 않는다(도구는 있으나 미배선) — "
                     "임무 없는 부팅에서 잔무 큐 자율 착수가 되살아난다")
            return
        self.add(cid, PASS, "%s self-test OK + next-action 임무 게이트 배선(exit 3) 확인" % p)

    # ── C78 JavisRadio 도구 존재 + 자기검증 (RADIO_SPEC_v4 · 2026-08-13) ──
    # ★WARN 티어(READY 미차단): radio 는 파일럿 단계 기능이라 부재·self-test 실패가 부트를
    #   통째로 막으면 안 된다(C62·C76 선례의 'WARN(READY 미차단)' 규율).
    #   그러나 **조용히** 없으면 안 된다 — 이 노드에서 self-test 가 실패하면 AA33(a) 능력
    #   게이트가 open 시점에 이 노드의 radio 등록을 거부하고(exit 3), §4.5 heartbeat 백스톱은
    #   '스크립트 부재로 재기동 불능'인 워커에게 재기동 지시를 무한 반복한다. 부트 ⓪ 출력에
    #   상태를 올려 그 난청을 개통 **전에** 관측 가능하게 한다.
    #   읽기 전용(self.results/planned 미참조 — report 병렬 워커 안전)·--fix 무관(자동수리 불가).
    #   cid 는 C78(C63·C64 결번 재사용 금지 — T0 §5-4).
    def c78_radio(self):
        cid = "C78.radio"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "bin", "javis_radio.py")
        if not os.path.isfile(p):
            self.add(cid, WARN, "pack/bin/javis_radio.py 부재 — 이 노드는 radio 미개통 "
                                "(AA33(a) 능력 게이트에서 등록 거부된다) · `cys init-pack` 으로 설치")
            return
        try:
            r = subprocess.run([sys.executable, p, "--self-test"],
                               capture_output=True, timeout=60, env=_utf8_env())
        except Exception as e:
            self.add(cid, WARN, "javis_radio.py --self-test 실행 불가: %s "
                                "— radio 능력 미검증(READY 미차단)" % e)
            return
        if r.returncode != 0:
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            self.add(cid, WARN, "javis_radio.py --self-test 실패(exit %d): %s "
                                "— 이 노드는 radio 능력 게이트에서 등록 거부된다(READY 미차단)"
                     % (r.returncode, tail[-300:]))
            return
        self.add(cid, PASS, "%s self-test OK (명명 대조·진위 게이트·쿨다운/차단기·seq/로테이션·"
                            "커서·close 시퀀스)" % p)

    # ── C81 npm_config_prefix 번들 오염 — **데몬 판정의 소비자**(부트 v2 A6 · W-B #2 협업) ──
    #
    # ★이 검사에는 술어가 없다. 있는 것은 **판독과 문안**뿐이다. 판정 주체는 데몬이고
    #   (`cys status --json` → `result.daemon.npm_prefix_polluted` · bool · 항상 존재 ·
    #   호출마다 재평가), preflight 는 그 bool 을 소비만 한다. 같은 술어를 python 으로 다시
    #   구현하면 경로 정규화·Windows 대소문자·형제 접두(`cys.app-old`) 같은 함정이 **두 벌**이
    #   되고, 두 판정이 갈리는 순간 사용자는 pane 고지와 preflight 에서 **서로 다른 처방**을
    #   받는다 — 정본 문안 함수가 독스트링으로 금지한 바로 그 상태다.
    # ★티어 WARN: 데몬은 이 값을 **덮지 않는다**(사용자 설정이다). 부트를 막을 사안이 아니다.
    # ★미측정 규약(이 파일의 계급 구분 그대로): 키가 없으면 SKIP 이다. 키 부재를 '깨끗함'으로
    #   접으면 구 데몬 전 기계에서 이 축이 **거짓 초록**이 되고, FAIL 로 접으면 부트 v2 미배선
    #   스큐가 부트를 막는다. 재지 못한 것은 결손도 통과도 아니다.
    def c81_npm_prefix_polluted(self):
        cid = "C81.npm-prefix-polluted"
        if self.skipped(cid):
            return
        cys = shutil.which("cys")
        if not cys:
            self.add(cid, SKIP, "PATH 에 cys 없음 — 데몬 판정 조회 불가(C11 소관)")
            return
        try:
            r = subprocess.run([cys, "status", "--json"], capture_output=True,
                               text=True, timeout=15, env=_utf8_env())
        except Exception as e:  # noqa: BLE001
            self.add(cid, SKIP, "cys status --json 실행 불가(%s) — 미측정" % e)
            return
        if r.returncode != 0:
            self.add(cid, SKIP, "cys status --json rc=%d — 미측정(데몬 미가동 가능)" % r.returncode)
            return
        try:
            payload = json.loads(r.stdout or "{}")
        except Exception:  # noqa: BLE001
            self.add(cid, SKIP, "cys status --json 판독 불가(JSON 아님) — 미측정")
            return
        daemon = (payload.get("result") or {}).get("daemon") or {}
        if "npm_prefix_polluted" not in daemon:
            self.add(cid, SKIP, "daemon.npm_prefix_polluted 키 부재 — 구 데몬·부트 v2 미배선(미측정). "
                                "키 부재를 '깨끗함'으로 접지 않는다")
            return
        if daemon.get("npm_prefix_polluted") is True:
            self.add(cid, WARN, NPM_PREFIX_BUNDLE_WARNING)
        else:
            self.add(cid, PASS, "npm_config_prefix 번들 오염 없음(데몬 판정)")

    # ── C79 cycle-verifier heartbeat (R6 W0-5) — 전 등급 WARN 티어(READY 미차단) ──
    # ★FAIL 금지 근거: 검증자 pane 은 전자동 사이클 **live** 단계의 전제(autopilot 게이트6)
    #   일 뿐, S0 shadow 단계 전에는 미필수다 — 부트 비치명. 부재·노화·수리 실패 전부
    #   WARN 강등(C78 radio 의 'WARN(READY 미차단)' 규율 동형). cid 는 C79(결번 재사용 금지).
    #   report 모드에선 읽기 전용(mtime 조회만 — 병렬 워커 안전) · --fix 에서만 subprocess.
    def c79_cycle_verifier_heartbeat(self):
        cid = "C79.cycle-verifier-heartbeat"
        if self.skipped(cid):
            return
        ap = _cycle_autopilot_mod()
        if ap is None:
            self.add(cid, WARN, "javis_cycle_autopilot import 불가(팩 스큐?) — heartbeat 판정 "
                                "보류(READY 미차단) · `cys init-pack` 으로 팩 정합 복구")
            return
        try:
            hb_mtime = os.path.getmtime(ap.HEARTBEAT)
        except OSError:
            hb_mtime = None
        state, age = heartbeat_verdict(hb_mtime, time.time(), ap.HEARTBEAT_MAX_AGE)
        if state == "fresh":
            self.add(cid, PASS, "verifier heartbeat 신선(%.1fs ≤ %.0fs) — %s"
                     % (age, ap.HEARTBEAT_MAX_AGE, ap.HEARTBEAT))
            return
        why = ("verifier heartbeat 부재(%s)" % ap.HEARTBEAT) if state == "absent" else \
            "verifier heartbeat 노화(%.1fs > %.0fs)" % (age, ap.HEARTBEAT_MAX_AGE)
        if self.fix:
            script = os.path.abspath(ap.__file__)
            try:
                # --ensure = 멱등 게이트(P0-1): 이 서브프로세스 안에서 실재+신선을 재판정하고
                # 건강하면 no-op — preflight 와 워치독 잡이 겹쳐 떠도 중복 pane 0.
                r = subprocess.run([sys.executable, script, "bootstrap-verifier", "--ensure"],
                                   capture_output=True, text=True, timeout=60, env=_utf8_env())
            except Exception as e:  # noqa: BLE001
                self.add(cid, WARN, "%s — bootstrap-verifier --ensure 실행 불가(%s) · "
                                    "WARN 강등(부트 비치명 — S0 shadow 전 미필수·FAIL 금지)"
                         % (why, e))
                return
            if r.returncode == 0:
                self.add(cid, FIXED, "%s → bootstrap-verifier --ensure 수행(heartbeat 는 워처 "
                                     "기동 후 touch 주기 %.0fs 내 생성)"
                         % (why, ap.HEARTBEAT_TOUCH_SECS))
            else:
                tail = ((r.stdout or "") + (r.stderr or "")).strip()[-200:]
                self.add(cid, WARN, "%s — --ensure 실패(exit %d): %s · WARN 강등(부트 비치명 — "
                                    "검증자는 S0 shadow 전 미필수·FAIL 금지)"
                         % (why, r.returncode, tail))
            return
        self.add(cid, WARN, "%s — 수리: javis_cycle_autopilot.py bootstrap-verifier --ensure "
                            "또는 --fix (WARN 티어 — S0 shadow 전 미필수)" % why)

    def c76_app_seal(self):
        cid = "C76.app-seal"
        if self.skipped(cid):
            return
        if sys.platform != "darwin":
            self.add(cid, SKIP, "macOS 아님 — 코드서명 봉인 검사 미해당")
            return
        bundle = self._find_app_bundle()
        if not bundle:
            self.add(cid, SKIP, "앱 번들 미발견(비번들 설치·개발 빌드) — 검사 대상 없음")
            return
        tool = "/usr/bin/codesign"
        if not os.path.exists(tool):
            self.add(cid, SKIP, "%s 부재 — 판정 불가" % tool)
            return
        # --verify --strict = Gatekeeper 가 보는 최상위 봉인 판정. --verbose 는 **필수**:
        # 없으면 "a sealed resource is missing or invalid" 한 줄뿐이라 원인 파일을 못 말한다(실측).
        # --deep 은 쓰지 않는다(이번 파손은 최상위 sealed resource · deep 은 느리다).
        try:
            r = subprocess.run([tool, "--verify", "--strict", "--verbose", bundle],
                               capture_output=True, timeout=60, env=_utf8_env())
        except subprocess.TimeoutExpired:
            self.add(cid, SKIP, "codesign 시간초과(60s) — 판정 불가")
            return
        except Exception as e:  # noqa: BLE001
            self.add(cid, SKIP, "codesign 실행 실패(%s) — 판정 불가" % e)
            return
        if r.returncode == 0:
            self.add(cid, PASS, "코드서명 봉인 무결 — %s" % bundle)
            return
        out = ((r.stdout or b"").decode("utf-8", "replace")
               + (r.stderr or b"").decode("utf-8", "replace"))
        added, modified, missing, other = [], [], [], []
        for line in out.splitlines():
            ln = line.strip()
            if not ln:
                continue
            for pfx, bucket in (("file added: ", added), ("file modified: ", modified),
                                ("file missing: ", missing)):
                if ln.startswith(pfx):
                    bucket.append(ln[len(pfx):].strip())
                    break
            else:
                if not ln.startswith("--prepared:") and not ln.startswith("--validated:"):
                    other.append(ln)
        culprits = added + modified + missing
        if not culprits:
            # codesign 은 실패했으나 봉인 파손 문장이 아니다(미서명 개발 번들 등) — 손상을
            # 확증하지 못했으므로 원문만 남긴다(거짓 단정 금지).
            self.add(cid, WARN, "codesign 검증 실패(rc=%d) — 봉인 파손 파일은 특정되지 않음: %s "
                                "(미서명 개발 빌드면 정상)"
                     % (r.returncode, (" | ".join(other))[:300] or "(출력 없음)"))
            return
        rel = [c[len(bundle):].lstrip("/") if c.startswith(bundle) else c for c in culprits[:3]]
        pycache = " ★전부 __pycache__ — 번들 안 Python 런타임이 실행 중 스스로 생성(자기유발)" \
            if all("__pycache__" in c for c in culprits) else ""
        # 추가-전용 파손만 '파일 삭제 = 복구' 갈래가 참이다 — 수정·누락이 섞이면 재설치 문구로.
        recovery = (self.APP_SEAL_RECOVERY_ADDED if added and not modified and not missing
                    else self.APP_SEAL_RECOVERY)
        self.add(cid, WARN,
                 "코드서명 봉인 파손 — %s · 추가 %d건·수정 %d건·누락 %d건(예: %s)%s · 이 번들을 "
                 "브라우저로 배포하면 받는 쪽 첫 실행에서 Gatekeeper 가 '손상되었기 때문에 열 수 "
                 "없습니다'로 차단한다(공증·staple 정상이어도) · %s"
                 % (bundle, len(added), len(modified), len(missing), ", ".join(rel), pycache,
                    recovery))

    # ── C80 동봉 런타임 봉인 매니페스트 (부트 v2 §2-10 G5 · 명세의 "C-SEAL") ──
    #
    # 왜 C76 과 별개인가: C76(코드서명)은 **macOS 전용**이다. Windows 설치본에는 sealed-resource
    # 봉인이 아예 없어 설치 후 `runtime/**` 오염을 잡을 수단이 0이었다 — 이 체크가 그 공백을
    # 메운다. mac 에서는 코드서명이 여전히 권위이고(서명 키 없이는 위조 불가) 이건 보조층이다.
    #
    # ★등급 WARN 고정(master 판정 2026-09-04 D1 · 레인 분리): 발행 차단은 릴리스 게이트
    #   (release-gate-gatekeeper.sh)가 FAIL 로 하고, **기계 preflight 는 부팅을 막지 않는다**.
    #   C76 이 같은 판단을 이미 문서화했다(:5275 "이미 파손된 기계가 READY→NOT READY 로
    #   뒤집혀 부팅 자체가 막힌다"). 실제로 2026-09-04 오너 머신이 파손 상태였다 — FAIL 등급이면
    #   그 기계가 그날로 부팅 불능 판정을 받았을 것이다.
    # ★자동 수정 0: C76 과 같다. 번들 안 파일 삭제는 App Management(TCC)에 막힐 수 있고,
    #   무엇인지 모르는 파일을 지우는 자동화가 더 위험하다. 처방만 말한다.
    # ★구버전은 SKIP: 매니페스트는 v0.14.30 부터 실린다. 그 이전 설치본에 없는 것은 정상이며
    #   경고할 일이 아니다(거짓 경보 금지 — C76 의 '판정 불가는 SKIP' 규율과 동형).
    #   ★단 "부재=무조건 SKIP" 은 틀렸다(R1 codex #6): 0.14.30 **이상** 설치본에서 매니페스트만
    #   사라진 경우까지 정상으로 덮으면, Windows 에서 유일한 변조 감지축이 조용히 없어진다
    #   (mac 은 코드서명이 남지만 Windows 는 남는 게 0). 그래서 부재를 만나면 **설치본 버전을
    #   판독해** 갈래를 나눈다 — 구버전이면 SKIP, 봉인 탑재 버전이면 WARN, 판독 불가면
    #   '판정 불가' SKIP(측정 불능은 통과가 아니다 · R1 codex #7 에서 세운 같은 규율).
    RUNTIME_SEAL_RECOVERY = ("복구는 v0.14.30 이상으로 재설치 — 설치기가 .app(또는 설치 폴더)을 "
                             "**통째로 교체**하므로 오염 파일이 함께 사라지고 봉인이 자연 복원된다"
                             "(부분 삭제 불요·권장하지 않음). 상세 진단은 `cys doctor`")

    # 이 봉인(runtime-manifest)이 실리기 시작한 버전. 이보다 낮은 설치본에 없는 것은 정상이다.
    RUNTIME_SEAL_SINCE = "0.14.30"

    def _installed_version_near(self, root):
        """설치본 버전 판독 → (버전문자열|None, 판독처 설명). 판독처는 **설치 형태**가 정한다.

        · macOS 번들: `<bundle>/Contents/Info.plist` 의 `CFBundleShortVersionString`
          (실측 2026-09-04: 오너 설치본 `/Applications/cys.app` = "0.14.29" · XML plist).
        · 그 외(Windows NSIS 포함): 설치 루트의 `cys-installed-version.txt`.
          이 마커는 NSIS 훅이 **모든 게이트를 통과한 뒤에만** 쓴다(src-tauri/nsis-hooks.nsh:784
          `cys_post_ok`) — 즉 '마지막으로 실제 반영된 버전'이다. 판정 재료로서는 정보성이지만
          (WINDOWS-UPGRADE-ATOMICITY-CHECKLIST.md:140), 여기서 묻는 것은 '설치가 성공한 버전이
          무엇인가' 하나뿐이라 이 용도에는 정확하다.
        판독 실패는 거짓말하지 않고 None 을 돌려준다 — 호출자가 '판정 불가'로 표현한다."""
        bundle = self._app_bundle_of(root)
        if bundle:
            plist = os.path.join(bundle, "Contents", "Info.plist")
            try:
                import plistlib
                with open(plist, "rb") as f:
                    d = plistlib.load(f)
                v = d.get("CFBundleShortVersionString") if isinstance(d, dict) else None
            except Exception as e:  # noqa: BLE001 — 손상 plist·바이너리 형식 등
                return None, "%s 판독 실패(%s)" % (plist, e)
            if not isinstance(v, str) or not v.strip():
                return None, "%s 에 CFBundleShortVersionString 없음" % plist
            return v.strip(), plist
        marker = os.path.join(os.path.dirname(root), "cys-installed-version.txt")
        try:
            with open(marker, encoding="utf-8", errors="replace") as f:
                v = f.read().strip()
        except OSError as e:
            return None, "%s 판독 실패(%s)" % (marker, e)
        if not v:
            return None, "%s 가 비어 있음" % marker
        return v, marker

    def _runtime_seal_expected(self, ver):
        """이 버전의 설치본은 매니페스트를 **싣고 있어야 하는가**. True/False/None(판정 불가).

        비교는 결정론 도구(`javis_semver`)에 위임한다 — 문자열 대소 비교를 여기서 재발명하면
        "0.14.9 vs 0.14.30" 같은 사전식 함정을 다시 만든다. `compare(기준선, 설치본)` 의
        MAIN_AHEAD = 설치본이 기준선보다 낮다 = 구버전(SKIP).
        ★알려진 경계: `0.14.30-rc1` 은 semver 상 0.14.30 미만이라 구버전으로 접힌다 —
        정식 릴리스에만 걸리는 게이트라는 뜻이고, 거짓 경보보다 이쪽이 안전하다(fail-safe)."""
        if not ver:
            return None
        try:
            import javis_semver as _sv
        except Exception:  # noqa: BLE001 — 팩에서 빠졌으면 판정 불가(추측 금지)
            return None
        verdict, _ev = _sv.compare(self.RUNTIME_SEAL_SINCE, ver)
        if verdict == _sv.INCOMPARABLE:
            return None
        return verdict != _sv.MAIN_AHEAD

    def _find_runtime_seal_pair(self):
        """(매니페스트 경로, runtime 트리 경로) 또는 None. 설치 형태별 후보를 전부 본다."""
        cands = []
        bundle = self._find_app_bundle()
        if bundle:                                   # macOS .app
            res = os.path.join(bundle, "Contents", "Resources")
            cands.append((os.path.join(res, "runtime-manifest.json"),
                          os.path.join(res, "runtime")))
        cys = shutil.which("cys")
        if cys:
            # 설치 루트 기준(Windows NSIS: %LOCALAPPDATA%\cys · 그 외 비번들 설치).
            # ★realpath 로 푼다 — 심링크 그림자(예: ~/.local/bin/cys)를 설치 루트로 오인하면
            #   런타임 트리를 못 찾아 조용히 SKIP 된다.
            d = os.path.dirname(os.path.realpath(cys))
            cands.append((os.path.join(d, "runtime-manifest.json"), os.path.join(d, "runtime")))
            cands.append((os.path.join(d, "resources", "runtime-manifest.json"),
                          os.path.join(d, "runtime")))
        for man, root in cands:
            if os.path.isfile(man) and os.path.isdir(root):
                return man, root
        # 매니페스트는 없는데 런타임 트리는 있는가 = 구버전 설치본(정상)인지 구분해 돌려준다.
        for _man, root in cands:
            if os.path.isdir(root):
                return None, root
        return None, None

    def c80_runtime_seal(self):
        cid = "C80.runtime-seal"
        if self.skipped(cid):
            return
        try:
            import javis_runtime_seal as _seal
        except Exception as e:  # noqa: BLE001
            self.add(cid, SKIP, "javis_runtime_seal 판독 불가(%s) — 판정 불가" % e)
            return
        man, root = self._find_runtime_seal_pair()
        if not root:
            self.add(cid, SKIP, "동봉 런타임 트리 미발견(비번들 설치·개발 빌드) — 검사 대상 없음")
            return
        if not man:
            # 부재를 만나면 설치본 버전으로 갈래를 나눈다(R1 codex #6) — 무조건 SKIP 은
            # "신규 설치에서 매니페스트만 소실"을 정상으로 덮어 감지축을 통째로 없앤다.
            ver, where = self._installed_version_near(root)
            expected = self._runtime_seal_expected(ver)
            if expected is None:
                self.add(cid, SKIP, "runtime-manifest 부재 + 설치본 버전 판독 불가(%s) — "
                                    "판정 불가(통과가 아니다) · 트리: %s" % (where, root))
                return
            if not expected:
                self.add(cid, SKIP, "runtime-manifest 부재 — 설치본 %s 는 봉인 도입(v%s) 이전이라 "
                                    "정상 · 판독처: %s · 트리: %s"
                         % (ver, self.RUNTIME_SEAL_SINCE, where, root))
                return
            self.add(cid, WARN,
                     "runtime-manifest 부재 — 설치본 %s 는 봉인 탑재 버전(v%s 이상)인데 매니페스트가 "
                     "없다: 설치 후 runtime/** 변조를 잡을 축이 사라진 상태다(Windows 에는 코드서명 "
                     "봉인이 없어 이것이 유일한 축) · 판독처: %s · 트리: %s · %s"
                     % (ver, self.RUNTIME_SEAL_SINCE, where, root, self.RUNTIME_SEAL_RECOVERY))
            return
        try:
            m = _seal.load_manifest(man)
            d = _seal.classify(m, root)
        except Exception as e:  # noqa: BLE001
            self.add(cid, SKIP, "봉인 대조 실패(%s) — 판정 불가" % e)
            return
        total = len(d["missing"]) + len(d["added"]) + len(d["changed"])
        if total == 0:
            self.add(cid, PASS, "동봉 런타임 봉인 무결 — 항목 %d개 일치(%s)"
                     % (len(m.get("entries") or {}), root))
            return
        # 원인 파일을 말한다 — "손상되었습니다" 한 줄은 안내가 아니다(C76 과 같은 규율).
        sample = (d["added"] + d["changed"] + d["missing"])[:3]
        npm_hint = ""
        if any("node_modules" in p or p.startswith("node/bin/") for p in
               (d["added"] + d["changed"])):
            npm_hint = (" ★node_modules 오염 — `npm i -g <패키지>` 가 동봉 node 의 기본 prefix"
                        "(=번들 안)로 설치된 흔적이다. 전역 설치는 번들 밖 prefix 로 하라")
        self.add(cid, WARN,
                 "동봉 런타임 봉인 파손 — 추가 %d건·변경 %d건·누락 %d건(예: %s)%s · 이 설치본을 "
                 "그대로 재배포하면 받는 쪽에서 무결성 검증이 깨진다 · %s"
                 % (len(d["added"]), len(d["changed"]), len(d["missing"]),
                    ", ".join(sample), npm_hint, self.RUNTIME_SEAL_RECOVERY))

    def run(self):
        # 의도된 호출 순서(불변식). C25를 C18보다 먼저: C25의 --fix(파일 설치·색인 등재)가
        # 정합을 만든 뒤 C18이 verify해야 같은 런에서 FAIL/FIXED 플랩(NOT READY 헛사이클)이
        # 없다(6차 R1). report 모드 병렬 실행도 결과를 이 순서 그대로 재조립한다.
        # ★가드: 체크 함수는 self.results/self.planned를 읽지 말 것 — report 병렬 워커에선
        # 결과가 thread-local sink에 있어 self.results가 비어 있다(읽으면 조용한 오답).
        checks = [
            self.c01_pack_dir, self.c02_directives, self.c03_content_pins,
            self.c04_soul, self.c05_agents, self.c06_json_files,
            self.c07_hook_script, self.c08_hook_registered, self.c09_round_core,
            self.c10_todo_files, self.c11_cys_binary, self.c11b_cys_dept_path,
            self.c12_daemon, self.c13_claude_md, self.c14_self,
            self.c15_report_tool, self.c16_report_schedule, self.c17_route_engine,
            self.c25_autopilot_memory, self.c18_memory_engine,
            self.c19_orchestra_engine, self.c20_nlm_sot, self.c21_harness_creator,
            self.c22_work_skills, self.c23_governance_conflict,
            self.c24_korean_law_mcp, self.c26_video_creator, self.c27_appbuild,
            self.c28_self_correction, self.c29_harness_engineering, self.c30_git,
            self.c31_config_isolation, self.c32_statusline, self.c33_event_hooks,
            self.c34_registry, self.c35_select, self.c36_verdict,
            self.c37_adr_engine, self.c38_silent_failure_catalog,
            self.c39_prereq_orphan_lint, self.c40_workflow_manifest,
            self.c41_skillscan, self.c42_mcpgate, self.c43_serena,
            self.c44_serena_eval, self.c45_semver_selftest, self.c46_bias_check,
            self.c47_transcribe_channel, self.c48_content_channel_deps,
            self.c49_channel_health, self.c50_channel_watch,
            self.c51_cleanroom_vendor, self.c52_license_gate, self.c53_idempotency,
            self.c54_loc_cap, self.c55_grill_gate, self.c56_dept_hook_leak,
            self.c57_temp_hook_leak, self.c58_trust_harden, self.c59_guard_wiring,
            self.c60_gate_wiring, self.c61_doc_code_sot, self.c65_drain_verify,
            self.c66_board_catalog, self.c67_learn_wiring, self.c69_gate_ledger,
            self.c70_launchd_job, self.c71_agents_schema,
            # Phase 1 Wave B2(C72~C75) — 마지막 고정 슬롯(C62·C68) 앞(§5-4 배선 규율).
            self.c72_phase1_gate_set, self.c73_guard_hook_integrity,
            self.c74_brief_gate_paths, self.c75_verify_spec_rescan,
            # C76(2026-08-01 실사고 M3) — 앱 번들 코드서명 봉인. 읽기 전용·WARN-only.
            self.c76_app_seal,
            # C77(2026-08-01 실사고 T1) — 임무 게이트 도구 존재 + next-action 배선.
            self.c77_mission_gate,
            # C78(2026-08-13 RADIO_SPEC_v4) — radio 도구 존재 + self-test. WARN-only.
            # 마지막 고정 슬롯(C62·C68) **앞**에 둔다(§5-4 배선 규율).
            self.c78_radio,
            # C79(R6 W0-5) — cycle-verifier heartbeat 신선도. WARN-only(S0 shadow 전 미필수).
            self.c79_cycle_verifier_heartbeat,
            # C81(부트 v2 A6 · W-B #2 협업) — 데몬이 판정한 npm_config_prefix 번들 오염 **소비**.
            #   WARN-only(값을 덮지 않는 것이 계약 — 부트를 막지 않는다).
            self.c81_npm_prefix_polluted,
            # C80(2026-09-04 부트 v2 §2-10) — 동봉 런타임 봉인 매니페스트. 읽기 전용·WARN-only.
            # Windows 에는 코드서명 봉인(C76)이 없어 이 체크가 유일한 변조 탐지다.
            # 번호 규율: C63·C64 는 결번 재사용 금지라 다음 자유 번호는 C80(§5-4).
            self.c80_runtime_seal,
            # C62는 마지막 고정 — 같은 런의 --fix가 남긴 치유 원장까지 이 런에서 보이게.
            # C68은 C62 직후(원장 소비 강제 게이트 — 같은 런의 최신 원장 기준으로 기한 판정).
            self.c62_pack_heal_ledger,
            self.c68_merge_pending_age,
        ]
        # --fix/dry/safe 는 공유 상태(repair_via_init_pack 메모이즈·settings.json 원자적
        # 쓰기·planned 버퍼)를 갖는 변이 경로라 전면 직렬 유지. report 모드만 병렬화한다.
        if self.mode != "report":
            for check in checks:
                check()
            return self.results
        # report 모드: 부작용0·공유 가변상태0(Phase 0 증명)인 독립 self-test 를 bounded pool
        # 로 병렬 실행하고, 각 결과를 원래 인덱스에 되꽂아 run() 호출 순서로 재조립한다
        # (출력 바이트 = 직렬과 동일). bounded=부팅 시점 자원 경합·resource_gate trip 방지.
        import concurrent.futures
        # IO 바운드(subprocess 대기 지배)라 최소 2 보장 — 저코어(≤7) 머신에서 워커=1이면
        # 직렬+풀 오버헤드 순손실. 상한 4는 부팅 시점 자원 경합·resource_gate trip 방지.
        max_workers = min(4, max(2, (os.cpu_count() or 4) // 4))
        bufs = [None] * len(checks)
        with concurrent.futures.ThreadPoolExecutor(max_workers=max_workers) as ex:
            fut_index = {ex.submit(self._run_check_isolated, c): i
                         for i, c in enumerate(checks)}
            for fut in concurrent.futures.as_completed(fut_index):
                bufs[fut_index[fut]] = fut.result()
        for buf in bufs:
            self.results.extend(buf)
        return self.results

    def _run_check_isolated(self, check):
        """병렬 워커: 이 체크의 add() 를 스레드-로컬 버퍼로 격리 수집해 반환한다."""
        buf = []
        self._local.sink = buf
        try:
            check()
        finally:
            self._local.sink = None
        return buf


# ═══════════════════════════════════════════════════════════════════════════════════════
# WP-2(0.14.31 · 감사 2026-09-06 에러4 원천봉쇄) — 폴더 신뢰 사전 주입 `--seed-trust`
# ═══════════════════════════════════════════════════════════════════════════════════════
# 왜: dept-3 계정 dir 의 hasTrustDialogAccepted=false 가 첫기동 "폴더 신뢰" 관문을 열었고, claude 2.1.261 은 그
#   관문의 기본 포커스가 "No, exit" 라 코퍼스 자동통과가 버전 핀으로 보류(옳은 보류)된 채 좌석이 죽었다(에러4 ①·②).
#   이 도구는 cys-dept 가 계정 dir 을 만든 직후·claude 기동 **前** 에 그 (config_dir, cwd) **한 쌍만** 신뢰로 박는다.
# 불변: ①한 쌍만(합집합 살포 금지 · codex 10) ②hasCompletedOnboarding 무접촉(테마 관문은 코퍼스 자동통과 대상)
#   ③이미 신뢰(정확 키 true)면 **프로세스 프로브 없이** already-trusted(R1 — 가동 중 부서의 rotate/launch 재사용 경로가
#     매번 'REFUSE live-claude' WARN 을 내던 것) ④쓰기가 필요할 때만 라이브 claude(그 CLAUDE_CONFIG_DIR) 0 확인 — 검증
#     불가는 **거부**(--force-unverified 만 그 단계를 넘긴다 · 잠금·대조는 넘기지 못한다 · 양성 관측 n>0 은 강행으로도
#     못 넘는다) ⑤비차단 파일 잠금(phoenix _try_lock_nb 동형 · fcntl 은 Windows ImportError) — 기구 미가용도 거부
#   ⑥교체 **직전** 존재+바이트 대조(값싼 선검사) ⑦같은 dir mkstemp+fsync+교체 前 임시파일 되읽기 → **무손실 커밋(R2·R3)**:
#     기존 파일 = 원자 **교환**(darwin renamex_np RENAME_SWAP · linux renameat2 RENAME_EXCHANGE) 뒤 교환되어 나온 옛 inode
#     (= 교환 순간 .claude.json 에 있던 것 그대로)를 읽어 캡처 바이트와 대조 — 같으면 폐기(우리 계획의 전제가 참이었다) ·
#     다르면 **되교환**(상대 inode 를 그대로 복원) 후 REFUSE concurrent-change(되교환 뒤 우리 inode 에 제3의 쓰기가 앉았으면
#     .conflict-<utc> 로 보존) · 부재 파일 = `os.link`(원자 create-if-absent · 생겨나 있으면 REFUSE · 하드링크 미지원 FS 는
#     REFUSE link-failed(<errno>) — 내용보다 이름이 먼저 공개되는 O_EXCL 경로는 R3 에서 제거: 부분 쓰기가 영구 손상 문서를 남긴다) ·
#     ★R3(리뷰 codex BLOCK): 교환 기구 부재(Windows · 교환 미지원 FS)는 **REFUSE exchange-unavailable** — 대조~교체
#     창을 '고지' 로 넘기지 않는다(그 창의 기록자는 Rust 시더처럼 이 잠금을 모른다 · C43 `_enable_mcp_server` 는 R4 에서
#     같은 잠금 아래로 들어왔다). ★R4 정정(리뷰 major): **강행 플래그로도 열리지 않는다** — `--force-unverified` 는 프로브
#     단계만 넘기고, os.replace 폴백은 코드에 **없다**(R3 에서 삭제 · self-test 가 `"os.replace(" not in seed_src` 로 핀).
#   ⑧교체 後 되읽기 — 불일치/판독 실패에도 **롤백하지 않는다**(R1 · codex: 잠금은 우리끼리의 advisory 라 그 사이 쓴 claude/
#     C43/Rust 시더의 내용을 원본으로 되돌리면 그쪽 데이터를 파괴한다 · 임시파일 검증을 통과한 내용이 커밋됐으므로 남는
#     쪽이 안전) ⑨.claude.json 심링크/정션·**비정규 파일** 거부(모든 읽기 O_NOFOLLOW|O_NONBLOCK + 연 뒤 fstat 정규 재확인 —
#     writer 없는 FIFO 가 부트 경로를 영구 정지시키지 못한다 · R4 · 교체 직전 재검 · 교환되어 나온 inode 도 재검)
#   ⑩라이브 claude 프로브는 **기존 문서가 있을 때만**(R2 · 리뷰): 프로브가 지키는 것은 라이브 claude 가 메모리에 든 기존
#     .claude.json 이다 — 파일이 없으면(fresh fork = cys-dept allocate/create 의 주경로) 지킬 것이 없고 최악은 '동시 기동한
#     claude 의 첫 쓰기가 우리 플래그 1개를 덮음'(= 종전 상태 · 2차 방어). Windows 는 env 미노출로 config 귀속이 원리적으로
#     불가해 종전엔 hub 좌석(node/claude 상존) 아래의 **모든** 시드가 REFUSE unverified 였다(WP-2 가 Windows 에서 inert) →
#     이 규칙으로 신규 부서 경로는 Windows 에서도 기본값으로 동작 · 기존 문서+플래그 부재(0.14.30 이전 dir 의 rotate) 는
#     Windows 에선 교환 기구가 없어 **어떤 플래그로도 시드되지 않는다**(REFUSE exchange-unavailable · 강행 무관 · R4 정정 —
#     종전 이 자리의 '1회 수동 시드(--force-unverified)' 안내는 실행하면 영원히 실패하는 명령이었다). 그 복구는 도구가 아니라
#     **사람**이 한다: 관문에서 1회 신뢰하면 claude 자신이 플래그를 쓰고, 이후 시드는 already-trusted(무프로브·무커밋)로 끝난다
#     (WP-1 관문 보류가 그 관문을 사람에게 보이게 한다).
#   ⑪잠금 아래에서 죽은 시더의 임시파일(.claude.json.seed-<8자>)과 **우리 payload 만 든 displaced**(교환 직전 사망 · 이름에
#     payload sha256 16자가 박혀 있어 바이트로 판별 · R3)를 청소 — 교환 뒤 옛 inode 가 앉은 displaced(낯선 바이트)는 무접촉.
#   ⑫darwin 프로브의 argv/env 경계는 **`ps` 2회**(argv 만 · argv+env)의 접두 대조로 확정한다(R3 · 리뷰 codex: `ps -E` 한
#     줄엔 구분자가 없어 인자 속 `CLAUDE_CONFIG_DIR=/other` 가 env 값을 가렸다). 접두가 안 맞거나 argv 목록에 없는 claude
#     형상 줄은 unresolved(검증된 0 으로 흡수하지 않는다). ★R4(리뷰 codex major): env **값** 경계는 여전히 휴리스틱이라
#     ⓐ양성은 분할 前 **원문 바이트 대조**(`_ps_env_value_present`)로도 본다(꼬리·머리 공백 · 값 속 ' NAME=' 대상에서도
#     관측이 살아 있어야 강행이 라이브 좌석을 못 넘는다 — 종전 `' NAME=' 대상은 스캔 前 None` 조기 반환은 관측 기회 자체를
#     없애 강행에 문을 열어 뒀다 · 삭제) ⓑ대상이 그런 모호 형상이거나(`_ps_target_ambiguous`) 출력에 레코드 아닌 조각이
#     있으면(`_ps_lines_torn` — env 값 속 줄바꿈이 레코드를 쪼갠 것) claude 형상 줄의 **불일치를 '검증된 0' 으로 삼지 않는다**.
#     보통 대상 + 정상 ps 출력의 판정은 종전과 완전히 같다(검증된 0 보존 = WP-2 의 주경로).
#     ★R5(리뷰 codex major): env 값의 **끝은 원리적으로 확정 불가**다(구분자가 없다) — 이름이 셸 식별자가 아니면
#     (`BAD-NAME=`·`BASH_FUNC_f%%=`) 값이 길게 잡히고, 값 안에 ' X=y' 가 있으면 짧게 잡힌다. 그래서 판정을 값 하나가
#     아니라 **값 후보 전부**(`_ps_env_value_candidates` = `CLAUDE_CONFIG_DIR=` 뒤의 공백으로 끝나는 모든 접두)로 하고,
#     대조는 표기가 아니라 **파일시스템 동일성 3값**(`_same_dir` — True/False/None)으로 한다: 하나라도 True 면 양성 ·
#     하나라도 None(조회 불가 = EACCES/ESTALE)이면 미해결 · 전부 False 라야 검증된 0. 대소문자·유니코드 별칭도 여기서
#     닫힌다(macOS 는 대소문자·정규화 보존이라 `realpath` 표기 비교로는 같은 디렉터리가 '다름' 이 됐다). 대상이 기본
#     config(~/.claude)인데 그 줄에 겉보기 지정이 있으면 미해결이다 — 같은 직렬화가 `OTHER="x CLAUDE_CONFIG_DIR=…"`
#     에서도 나오므로 지정의 존재가 증거가 아니다(codex).
# ⑬시더가 **중단된 교환**을 남길 수 있다(마감 감시 `os._exit`·SIGKILL): 교환은 성립했는데 검증 전에 끊기면 상대의 더
#   새 문서가 displaced 이름에 남고 우리 payload 가 활성이 된다. 종전엔 다음 실행이 `already-trusted` 로 조용히 OK 를
#   내 그 문서가 영영 고립됐다(★R5 리뷰 codex major). 이제 교환 **직전**에 의도 저널(`.claude.json.seed-intent-*` ·
#   displaced 이름 + 원본/payload sha256 · fsync + dir fsync)을 남기고, 다음 실행이 잠금 아래 **가장 먼저** 두 파일의
#   바이트로 판정한다: 우리 payload 뿐이면 저널만 회수 · 원본과 같으면(활성 문서가 유효할 때만) 폐기 · **낯선 바이트면
#   무접촉 REFUSE `interrupted-transaction`**. 자동 재시드는 하지 않는다(활성 쪽이 더 새로울 수도 있어 이름만으로 선후를
#   알 수 없다). 저널을 못 쓰면 교환하지 않는다.
# 정직한 한계: 프로브 뒤 claude 기동 창(우리 플래그 1개가 덮일 수 있음 = 종전 상태 · 2차 방어가 받는다)은 남는다.
#   ★R5: R4 가 고지했던 '라이브 claude 쪽 값이 손실 형상' 잔여는 값 후보 × 파일시스템 동일성으로 **닫았다**(별칭 표기·
#   비식별자 변수명·값 속 ' NAME=' 전부). 대신 새 한계가 있다(정직): 후보 대조는 **보수적 양성**이라 다른 변수 값의
#   일부가 우연히 우리 대상 디렉터리를 가리키면 과잉 거부가 된다(가용성 손해 · 안전 방향). 레코드 조각도 우연히 숫자로
#   시작하면 레코드로 보인다. `--force-unverified` 는 여전히 '미해결' 을 넘긴다(양성은 못 넘는다) — 그 플래그는 사람이
#   명시적으로 감수하는 예외이고 cys-dept 는 쓰지 않는다. 두 경우의 결과는 '종전 상태'(claude 첫 쓰기가 우리 플래그 1개를 덮음)이고 2차 방어(관문 보류)가 받는다. 교환
#   기구가 없는 플랫폼(Windows · 미지원 FS)의 기존 문서는 **항상 거부**된다(R3 · 강행 플래그도 열지 않는다 — 우회 경로 0).
#   어느 경로에도 상대 데이터 파괴는 없다(롤백 0 · 백업은 캡처한 바이트로 배타 생성 · 되교환은 상대 inode 그대로 · os.replace
#   폴백 0). ★R4 시간 상한: CLI(`--seed-trust`)는 `CYS_SEED_TRUST_TIMEOUT`(기본 20초 · 0 이하면 끔 · 비유한값·상한 초과는
#   기본/상한으로 접는다)의 마감 감시로 부트 호출자가 I/O 에 무한정 잡히지 않는다 — 초과는 REFUSE timeout(rc 2). ★R5 정정:
#   그 1줄은 **커밋 여부를 단정하지 않는다**(공개 전이면 무쓰기지만, 교환 뒤 검증 전이면 커밋은 성립했다) — 판정은 다음
#   실행의 의도 저널(⑬)이 한다. 잠금은 프로세스 종료로 OS 가 푼다.
# 실패 방향: 전부 REFUSE(rc 2)/ERROR(rc 1) — 부분 쓰기 0 · 좌석 접촉 0. 호출자(cys-dept)는 fail-open(WARN 1줄 + 계속):
#   거부된 시드 = 종전과 같은 상태이고 2차 방어(restore 준비 판정의 관문 보류)가 뒤에 있다.
# 키 정책(R1 · codex): claude 가 읽는 키는 process.cwd() = getcwd() **정확 문자열 하나**(POSIX 심링크 해소 물리 경로 =
#   realpath · Windows abspath) = claude_project_key(cwd). 꼬리 슬래시·심링크 별칭 키는 claude 가 읽지 않으므로 '이미
#   신뢰' 판정에도 쓰지 않고 손대지도 않는다(종전 '동일성 같은 기존 키 재사용' 은 claude 가 안 읽는 키를 true 로 만들고
#   정확 키를 false 로 남겼다). 파일시스템 동일성(_path_identity·_same_dir)은 **레지스트리 등재 판정·프로세스 env 대조**
#   전용. ★R5(리뷰 codex major): macOS 의 `realpath` 는 **저장된 표기**를 돌려주지 않는다(대소문자 별칭 · 유니코드 정규화
#   차) — 그런 cwd 로 좌석을 띄우면 자식의 getcwd()(= claude 의 키)와 우리 키가 갈려 시드가 조용히 무효였다. darwin 은
#   `fcntl(F_GETPATH)` 로 저장 표기를 되살리되 **구조가 같을 때만** 채택한다(펌링크·마운트 별칭은 종전 realpath 유지 ·
#   정규화는 우리가 하지 않는다). 실측 오라클: 자식 getcwd()/node process.cwd() 와 4형 전수 일치(2026-09-07).
SEED_TRUST_OK, SEED_TRUST_ERROR, SEED_TRUST_REFUSE = 0, 1, 2
SEED_TRUST_LOCK_NAME = ".claude.json.seed-lock"
SEED_TRUST_TMP_PREFIX = ".claude.json.seed-"
SEED_TRUST_DISPLACED_PREFIX = ".claude.json.displaced-"   # 교환 전에 옮겨 두는 이름 — 교환 뒤 옛 inode 가 여기 앉는다
SEED_TRUST_CONFLICT_PREFIX = ".claude.json.conflict-"     # 되교환 창의 제3 쓰기 보존 — 자동 삭제 0
# ★R6(codex 위임 반례 ①): 파이썬 정규식 `$` 는 **문자열 끝 개행 앞**에도 일치한다 — 파일 이름과 sha256 필드는
#   개행을 담을 수 있으므로(POSIX 파일명은 개행 허용) 전부 `\Z`(진짜 끝)로 못 박는다. 종전엔 `<64hex>\n` 이
#   '유효한 지문' 으로 통과해 형식 위반 저널이 회수 경로로 들어갔다.
_SEED_TMP_LITTER_RE = re.compile(r"^\.claude\.json\.seed-[A-Za-z0-9_]{8}\Z")   # mkstemp 접미 8자만(잠금 -lock · displaced · conflict 제외)
# ★R3: displaced 이름 = <prefix><payload sha256 64hex>-<utc>-<pid>[-n]. 청소는 파일 바이트의 sha256 이 이름의 지문과 **같을
#   때만** = 'payload 와 바이트 동등한 잔재' 의 회수(소유·출처 증명이 아니다 · 지문이 다른 낯선 inode 는 무접촉). conflict-* 와
#   지문 없는 구형 displaced 이름은 청소 대상이 아니다.
_SEED_DISPLACED_RE = re.compile(r"^\.claude\.json\.displaced-([0-9a-f]{64})-")
# ★R5(리뷰 codex major · 중단된 트랜잭션): 교환 **직전**에 기록하고 처분이 끝나면 지우는 의도 저널. 이름은 mkstemp 잔재
#   정규식(`_SEED_TMP_LITTER_RE` = seed-<8자>)과 겹치지 않아 잔재 청소가 지우지 못한다.
SEED_TRUST_INTENT_PREFIX = ".claude.json.seed-intent-"
_SEED_INTENT_MAX_BYTES = 8192
_SEED_HEX64_RE = re.compile(r"^[0-9a-f]{64}\Z")
_SEED_DISPLACED_DIGEST_LEN = 64
_CLAUDE_EXE_NAMES = ("claude", "claude.exe", "claude.cmd")
_CLAUDE_INSTALL_MARKER = "/claude/versions/"        # 공식 설치 레이아웃 ~/.local/share/claude/versions/<ver>(직접 exec 형상)
_CLAUDE_NPM_MARKER = "/claude-code/"                # npm @anthropic-ai/claude-code/cli.js(node 인터프리터 형상)
_JS_SUFFIXES = (".js", ".mjs", ".cjs")
_JS_RUNTIME_NAMES = ("node", "node.exe", "bun", "bun.exe")   # strict 형상에서 claude-code .js 번들을 실행할 수 있는 argv[0]
_PS_ENV_SPLIT_RE = re.compile(r"\s+(?=[A-Za-z_][A-Za-z0-9_]*=)")
# Windows: Win32_Process 는 환경변수를 노출하지 않는다 → claude 실행 형상 전역 계수(자기·부모 제외). ★R1(codex): claude.exe/
#   claude 는 **이름만으로** 센다(CommandLine 이 null/빈 프로세스가 사라져 '검증된 0' 이 되던 것) · node 는 CommandLine 에
#   claude 가 있거나 CommandLine 을 못 읽으면(null) 미지 = 계수. 파이프라인 오류는 rc≠0 으로(ErrorActionPreference Stop).
_WIN_CLAUDE_COUNT_PS_TMPL = (
    "$ErrorActionPreference='Stop'; "
    "Get-CimInstance Win32_Process -Filter \"Name='claude.exe' OR Name='claude' OR Name='node.exe' OR Name='node'\" | "
    "Where-Object { $_.ProcessId -ne $PID -and $_.ProcessId -ne %d -and "
    "($_.Name -like 'claude*' -or -not $_.CommandLine -or $_.CommandLine -match 'claude') } | "
    "Measure-Object | Select-Object -ExpandProperty Count")


def _path_identity(p):
    """파일시스템 동일성 키(등재 판정·프로세스 env 대조 전용 — 쓰기 키도 '이미 신뢰' 키도 아니다): realpath + normcase
    (Windows 만 대소문자 접음) · 꼬리 구분자 제거. 판독 실패는 원문 그대로(비교 실패 = 불일치 방향).
    ★R4(codex 위임 반례): 비-str(bytes·None)은 **그대로** 돌려준다 — 종전엔 `realpath` 는 통과하고 그 뒤 `rstrip("\\/")`
    가 예외 밖에서 TypeError 를 냈다(순수 판정 함수가 호출자를 죽였다)."""
    if not isinstance(p, str):
        return p
    try:
        r = os.path.normcase(os.path.realpath(p))
    except (OSError, ValueError, TypeError):
        return p
    stripped = r.rstrip("\\/")
    return stripped or r


def _stat_ident(path):
    """(동일성 키|None, 상태) — 상태 ∈ {"ok","absent","unknown"}. 심링크는 따라간다(가리키는 **디렉터리**의 동일성).
    ★R5(리뷰 codex D1): 조회 실패를 '다르다' 로 읽지 않기 위해 **부재**(ENOENT·ENOTDIR·ENAMETOOLONG·ELOOP = 그 경로로
    도달하는 디렉터리가 없다 = 증명된 다름)와 **판정 불가**(EACCES·ESTALE·EIO…)를 나눈다."""
    if not isinstance(path, str) or not path:
        return None, "unknown"
    if len(path) > _MAX_PATH_PROBE:
        return None, "absent"       # 어떤 파일시스템도 이 길이의 이름을 갖지 않는다(값 후보 열거가 만드는 긴 문자열)
    try:
        st = os.stat(path)
    except (FileNotFoundError, NotADirectoryError):
        return None, "absent"
    except ValueError:
        return None, "absent"       # 널 바이트 등 — 경로가 될 수 없는 문자열
    except OSError as e:
        # 이름이 너무 길다/심링크 루프 = 그 경로로 도달하는 디렉터리가 없다(증명된 다름) · 권한·ESTALE·IO 는 판정 불가
        if e.errno in (getattr(errno, "ENAMETOOLONG", None), getattr(errno, "ELOOP", None)):
            return None, "absent"
        return None, "unknown"
    return (st.st_dev, st.st_ino), "ok"


def _same_dir(a, b):
    """두 경로 문자열이 **같은 디렉터리**인가 → True · False · **None(판정 불가)**.
    ★R5(리뷰 codex major D1): 종전엔 `_path_identity(a) == _path_identity(b)` 하나로 봤다 — macOS 는 대소문자 보존·무시
    (그리고 유니코드 정규화 보존)라 `realpath` 가 표기를 고치지 못해 **같은 디렉터리의 별칭 표기**가 '다름' 이 됐고, 그
    별칭으로 도는 라이브 claude 가 '검증된 0' 이 되어 라이브 config 에 썼다. 이제 표기 비교가 실패하면 **파일시스템
    동일성**(st_dev, st_ino)으로 본다. 그리고 codex D1 의 핵심: 조회가 **실패**하면(권한·ESTALE) 결코 '다르다' 로 접지
    않는다 — None 을 내고 호출자가 미해결로 처리한다(강행 플래그도 미해결을 넘지 못하는 것이 계약의 목표).
    순수하지 않다(stat) — 순수 판정부(`_count_claude_in_ps_lines`)는 이 함수를 **주입점**으로 받는다.
    조회 실패의 분류는 `_stat_ident` 가 한다: 부재류(ENOENT·ENOTDIR·ENAMETOOLONG·ELOOP)는 **증명된 다름**이고
    나머지(EACCES·ESTALE·EIO…)만 None 이다(★R5 · codex 위임 반례가 지적한 문서-코드 불일치 정정)."""
    if not isinstance(a, str) or not isinstance(b, str) or not a or not b:
        return None
    if a == b:
        return True
    # 순서 주의(성능): 값 후보 대조는 한 줄에 수백 번 돈다 — 값싼 `stat` 2회를 **먼저** 보고, 비싼 표기 정규화
    #   (`realpath` = 구성요소마다 lstat)는 **둘 다 부재**일 때만 한다(그때만 표기 동치가 판단 근거다).
    ka, sa = _stat_ident(a)
    kb, sb = _stat_ident(b)
    if ka is not None and kb is not None:
        return ka == kb
    if sa == "absent" and sb == "absent":
        return _path_identity(a) == _path_identity(b)
    if (ka is not None and sb == "absent") or (kb is not None and sa == "absent"):
        return False               # 한쪽은 실재하고 한쪽은 부재 = 증명된 다름
    return None                    # 판정 불가(권한·ESTALE·IO) — '검증된 0' 으로 흡수 금지


_MAX_PATH_PROBE = 4096            # 값 후보 stat 의 상한(PATH_MAX 상한선) — 이보다 길면 syscall 없이 '부재'
_F_GETPATH_DARWIN = 50            # darwin fcntl.h: F_GETPATH — 열린 fd 의 전체 경로(커널이 기억하는 저장 표기)
_MAXPATHLEN = 1024


def _fgetpath(path):
    """darwin 전용 **저장 표기 오라클** — `fcntl(fd, F_GETPATH)` 은 `getcwd(3)`/libuv `uv_cwd`(= node `process.cwd()`)
    와 같은 출처(vnode 이름)를 돌려준다. 실패·미지원·비-darwin 은 None(호출자는 종전 realpath 유지).
    실측(2026-09-07 · macOS 27 · APFS): `/Users/cys/Desktop/cysjavis` → `/Users/cys/Desktop/CYSjavis`(자식 getcwd 와 일치 ·
    realpath 는 별칭 표기를 그대로 둔다) · NFC/NFD 로 만든 디렉터리를 반대 형태로 열어도 자식 getcwd 와 일치(realpath 는 4형
    중 2형에서 불일치). 정규화는 **우리가 하지 않는다**(APFS 는 정규화 보존 · codex D1)."""
    if sys.platform != "darwin":
        return None
    try:
        import fcntl
    except ImportError:
        return None
    try:
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_CLOEXEC", 0))
    except (OSError, ValueError):
        return None
    try:
        buf = fcntl.fcntl(fd, _F_GETPATH_DARWIN, b"\0" * _MAXPATHLEN)
    except (OSError, ValueError, OverflowError, AttributeError):
        return None
    finally:
        try:
            os.close(fd)
        except OSError:
            pass
    try:
        got = os.fsdecode(buf.split(b"\0", 1)[0])
    except (ValueError, UnicodeError):
        return None
    return got or None


def _same_path_shape(a, b):
    """두 절대경로가 **같은 구조**(구성요소 수 동일 · 각 구성요소가 대소문자·유니코드 정규화만 다름)인가 — 순수.
    저장 표기 오라클의 결과를 '표기만 고친 것' 으로 한정해 채택하기 위한 방벽이다(펌링크·마운트 별칭처럼 **구조가
    다른** 경로는 채택하지 않는다 · codex D1)."""
    if not isinstance(a, str) or not isinstance(b, str):
        return False
    pa, pb = a.split(os.sep), b.split(os.sep)
    if len(pa) != len(pb):
        return False
    for x, y in zip(pa, pb):
        if x == y:
            continue
        try:
            if unicodedata.normalize("NFC", x).casefold() != unicodedata.normalize("NFC", y).casefold():
                return False
        except (TypeError, ValueError):
            return False
    return True


def claude_project_key(cwd):
    """claude 가 .claude.json projects 에 쓰는 정확한 키 — getcwd() 규약: POSIX realpath · Windows abspath.
    ★R5(리뷰 codex major D1): macOS 에서 `realpath` 는 **저장된 표기**를 돌려주지 않는다(대소문자 별칭·유니코드 정규화 차)
    — 그런 cwd 로 좌석을 띄우면 자식의 `getcwd()`(= claude 가 쓰는 키)는 저장 표기라서 우리가 박은 키를 **claude 가 읽지
    않았다**(기능이 조용히 inert · 관문이 다시 뜬다). darwin 은 `F_GETPATH` 오라클로 표기를 되살리되, **구조가 같을 때만**
    채택한다(`_same_path_shape` + samestat 재확인) — 오라클이 없거나 구조가 다르면 종전 realpath 그대로다(거부가 아니라
    종전 동작 유지: 여기서 거부하면 좌석이 관문 앞에 서는 것이 기본값이 된다).
    알려진 한계(★R5 · codex 위임 반례): **실행 전용 디렉터리**(mode 0111)는 자식의 chdir/getcwd 는 되지만 우리의 읽기
    전용 open 이 EACCES 라 오라클을 못 얻는다 → 그 별칭에서는 종전 표기로 시드한다(claude 가 안 읽는 키 = 관문이 다시
    뜬다 = **거부 방향**이지 좌석 사망이 아니다). 그런 워크스페이스에서는 claude 자신도 파일을 못 읽는다."""
    if os.name == "nt":
        return os.path.abspath(cwd)
    physical = os.path.realpath(cwd)
    got = _fgetpath(physical)
    if not got or not os.path.isabs(got) or got == physical or not _same_path_shape(got, physical):
        return physical
    try:
        if not os.path.samestat(os.stat(got), os.stat(physical)):
            return physical
    except (OSError, ValueError):
        return physical
    return got


def _default_claude_config_dir():
    """CLAUDE_CONFIG_DIR 미설정 claude 의 config dir(~/.claude) — env 없는 claude 프로세스의 귀속 대상(codex R1)."""
    return os.path.join(os.path.expanduser("~"), ".claude")


def _is_link_like(path):
    """심링크 또는 Windows 정션(py3.12 os.path.isjunction)."""
    try:
        if os.path.islink(path):
            return True
        isj = getattr(os.path, "isjunction", None)
        return bool(isj and isj(path))
    except (OSError, ValueError):
        return True   # 판정 불가 = 거부 방향


def _try_lock_nb(f):
    """비차단 배타 락 1회 시도(unix·Windows 통합 · javis_phoenix._try_lock_nb 동형 사본 — import 결합 회피).
    반환 True(획득)·False(다른 보유)·None(락 기구 미가용). unix=fcntl.flock(LOCK_EX|NB) · Windows=msvcrt.locking
    (LK_NBLCK, byte0). 둘 다 프로세스 사망 시 OS 가 자동 해제(stale lock 없음). ★여기서는 None 도 거부다(seed_trust)."""
    if os.name == "nt":
        try:
            import msvcrt
        except Exception:
            return None
        try:
            f.seek(0)
            msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
            return True
        except OSError:
            return False
        except Exception:
            return None
    try:
        import fcntl
    except Exception:
        return None
    try:
        fcntl.flock(f.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        return True
    except OSError:
        return False
    except Exception:
        return None


def trust_plan(data, cwd_key):
    """순수 계획 — (new_data, changed, used_key). projects[cwd_key].hasTrustDialogAccepted=True **정확 키 하나만** 건드린다
    (온보딩 플래그·다른 항목·별칭 키 무접촉 · 깊은 복사 후 변경). `projects` 키 부재면 생성 · 항목 부재면 생성. 이미 정확 키가
    dict 이고 True 면 changed=False. 비-dict 최상위 / **존재하는** 비-object projects(명시 null 포함) / 존재하는 비-object 항목
    (명시 null 포함) → ValueError(호출자 ERROR · 무쓰기). 부재 파일은 호출자가 {} 를 넘긴다(None 은 '있는 비-object')."""
    if not isinstance(data, dict):
        raise ValueError("최상위 비-object")
    new = json.loads(json.dumps(data))
    if "projects" in new:
        projs = new["projects"]
        if not isinstance(projs, dict):
            raise ValueError("projects 비-object")
    else:
        projs = new["projects"] = {}
    if cwd_key in projs:
        ent = projs[cwd_key]           # 명시 null 도 '있는 비-object' — 대체하지 않는다(codex R2: 손상 항목 무접촉)
        if not isinstance(ent, dict):
            raise ValueError("projects[%s] 비-object" % cwd_key)
    else:
        ent = projs[cwd_key] = {}
    if ent.get("hasTrustDialogAccepted") is True:
        return new, False, cwd_key
    ent["hasTrustDialogAccepted"] = True
    return new, True, cwd_key


def _trusted_exact(data, cwd_key):
    """순수 판정 — data(파싱된 .claude.json)에서 정확 키 항목이 dict 이고 hasTrustDialogAccepted is True 인가. 형상이
    어떻든 예외 0(되읽기 형 검사 · codex R1: {"projects":[1]} 같은 유효 JSON 에 AttributeError 를 내지 않는다)."""
    if not isinstance(data, dict):
        return False
    projs = data.get("projects")
    if not isinstance(projs, dict):
        return False
    ent = projs.get(cwd_key)
    return isinstance(ent, dict) and ent.get("hasTrustDialogAccepted") is True


def _run_capture(cmd, timeout=15):
    """subprocess 러너 — (rc, stdout, stderr). 예외도 rc 127 로 정규화(판정 불가 → 호출자가 None 으로 접는다)."""
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout or "", p.stderr or ""
    except Exception as e:  # noqa: BLE001 — 러너는 예외를 올리지 않는다
        return 127, "", "runner error: %s" % e


def _is_claude_command(tokens, strict=False):
    """argv 토큰열이 claude 실행 형상인가 — **구조** 판정: 어느 토큰의 basename 이 claude/claude.exe/claude.cmd 이거나
    경로에 claude-code(npm cli.js)가 있을 때만. 인자 문자열 속 '.claude' 경로(예: --config …/claude-default-dept-3)로는
    매칭되지 않는다(codex 6 — 검사기 자기매칭 차단).
    ★R2(리뷰 · Rust governance.rs argv[0] 승격 미러): strict=True 는 **argv[0] 의 basename** 이 claude 이름이거나 어느 토큰이
    claude-code(npm cli.js) 경로일 때만 — `tail -f …/logs/claude`·`less ~/.local/bin/claude`·`grep … claude`·
    `zsh -lc '…; claude'` 같은 **인자에 claude 가 실리는 비-에이전트**를 제외한다. env 가 안 보이는 줄(darwin ps -E 가 env 를
    숨기는 Apple 플랫폼 바이너리 = 바로 그 tail/less/grep/zsh)은 strict 로만 unresolved 후보가 된다 — 종전엔 그런 프로세스
    1개가 함대 전체의 시드를 REFUSE unverified 로 돌렸다(실측: `tail -f <dir>/logs/claude` 1건 → rc 2)."""
    for i, t in enumerate(tokens):
        raw = t.strip("\"'")
        if not raw:
            continue
        norm = raw.replace("\\", "/")
        low = norm.lower()
        if strict:
            # argv[0]: 실행파일 이름 또는 공식 설치 경로 · argv[0] 이 JS 런타임(node/bun)이면 그 뒤 어느 토큰이든 claude-code
            #   패키지의 .js 번들(`node --inspect …/claude-code/cli.js` 도 형상 · R3 codex) · `tail -f /x/claude-code/debug.log`
            #   같은 인자 속 패키지 세그먼트(.js 아님)나 다른 인터프리터(python3 x.py …cli.js)는 형상 아님(codex R2)
            if i == 0:
                base0 = os.path.basename(norm).lower()
                if base0 in _CLAUDE_EXE_NAMES or _CLAUDE_INSTALL_MARKER in low:
                    return True
                if _CLAUDE_NPM_MARKER in low and low.endswith(_JS_SUFFIXES):
                    return True                                   # 번들 직접 exec(shebang)
                if base0 not in _JS_RUNTIME_NAMES:
                    return False
                continue
            if _CLAUDE_NPM_MARKER in low and low.endswith(_JS_SUFFIXES):
                return True
            continue
        if "claude-code" in low or _CLAUDE_INSTALL_MARKER in low:
            return True
        if os.path.basename(norm).lower() in _CLAUDE_EXE_NAMES:
            return True
    return False


def _ps_argv_map(argv_lines):
    """`ps -ax -ww -o pid=,command=`(env 없음) 출력 → {pid: argv 문자열}. 파싱 불가 줄은 무시 · 같은 pid 가 둘이면(비정상)
    None 으로 남겨 '경계 미확정' 이 되게 한다(순수)."""
    out = {}
    for line in argv_lines or ():
        s_ = line.strip()
        parts = s_.split(None, 1)
        if len(parts) == 2 and parts[0].isdigit():
            out[parts[0]] = None if parts[0] in out else parts[1]
    return out


def _ps_env_value_present(text, name, value):
    """`NAME=<value>` 가 **바이트 그대로** 세그먼트 경계에 실려 있는가 — 휴리스틱 분할 前 원문 대조(★R4 · 리뷰 codex
    major 의 "preserve exact path bytes where possible"). 분할기(`_PS_ENV_SPLIT_RE`)는 값의 꼬리 공백을 구분자로 먹고
    값 속 ' NAME=' 에서 값을 자른다 — 그래서 **양성**은 분할 결과가 아니라 원문에서 찾는다. 그래야 꼬리 공백·내부
    ' NAME=' 대상에서도 관측이 살아 있고 `--force-unverified` 가 관측된 라이브 claude 를 넘지 못한다.
    경계: 앞은 문자열 머리 또는 공백(needle 이 `NAME=` 로 시작하므로 그 자리는 분할기도 나눈다) · 뒤는 문자열 끝 또는
    **아무 공백**. ★R5(리뷰 codex major): 종전엔 뒤 경계를 '다음 `NAME=` 분할점' 으로 요구해서, 뒤따르는 변수 이름이
    셸 식별자가 아니면(`CLAUDE_CONFIG_DIR=/target BAD-NAME=x`, `BASH_FUNC_f%%=…`) **양성 관측 자체가 사라졌다**(→
    검증된 0 → 라이브 config 에 쓰기). 참값은 언제나 공백으로 끝나는 접두이므로 '공백이면 경계' 가 필요조건으로 옳다.
    대가는 보수적 과잉 양성(값이 `/target archive` 인데 대상이 `/target` 이면 양성 = 거부 방향 · 정직하게 고지).
    순수 함수 — I/O·정규화 0(동일성 비교는 호출자가 따로 한다)."""
    if not isinstance(text, str) or not isinstance(value, str) or not value:
        return False
    needle = name + "=" + value
    i = text.find(needle)
    while i >= 0:
        if i == 0 or text[i - 1].isspace():
            j = i + len(needle)
            if j == len(text) or text[j].isspace():
                return True
        i = text.find(needle, i + 1)
    return False


_PS_VALUE_CANDIDATE_MAX = 512   # 개수 상한(길이 상한이 먼저 걸리는 것이 정상 · 개수로 잘리면 미해결)


def _ps_env_candidates_truncated(text, name="CLAUDE_CONFIG_DIR", limit=_PS_VALUE_CANDIDATE_MAX):
    """값 후보 열거가 **상한에서 잘렸는가**(★R5 · codex 위임 반례): 별칭 경로에 공백이 30개면 참값이 상한 밖으로 밀려
    후보가 '전부 불일치' 로 보였다 — 잘린 열거의 불일치는 증명이 아니므로 '검증된 0' 이 될 수 없다(미해결)."""
    return len(_ps_env_value_candidates(text, name, limit + 1)) > limit


def _ps_env_value_candidates(text, name="CLAUDE_CONFIG_DIR", limit=_PS_VALUE_CANDIDATE_MAX):
    """`NAME=` 가 세그먼트 머리에 실린 자리마다 그 뒤의 **공백으로 끝나는 모든 접두**를 값 후보로 낸다(순수 · 중복 제거 ·
    개수 상한 `_PS_VALUE_CANDIDATE_MAX`=512 · 길이 상한 `_MAX_PATH_PROBE`=4096. ★R6(리뷰 minor): 종전 docstring 의
    '상한 24' 는 R5 에서 512 로 올린 뒤 남은 사문이었다 — 이 상한은 '검증된 0' 과 '미해결' 을 가르는 안전 파라미터
    (`_ps_env_candidates_truncated`)라 오독이 곧 판정 오독이다).
    왜 접두 전부인가(★R5 · 리뷰 codex D2): `ps -E` 한 줄에는 env 구분자가 없어 값의 끝을 **원리적으로** 알 수 없다 —
    종전 판독기(`_PS_ENV_SPLIT_RE`)는 다음 `NAME=` 를 경계로 삼는데, ⓐ 이름이 셸 식별자가 아니면(`BAD-NAME=`,
    `BASH_FUNC_f%%=`) 경계를 못 보고 값이 **길게** 잡히고, ⓑ 값 안에 ` X=y` 가 있으면 값이 **짧게** 잡힌다. 둘 다
    같은 디렉터리를 가리키는 라이브 claude 를 '불일치' 로 만들어 '검증된 0' 을 냈다. 후보 전체를 파일시스템 동일성
    (`_same_dir`)으로 대조하면 두 방향이 함께 닫힌다(맞는 후보 하나면 양성 = 거부 방향).
    한계(정직): 후보 대조는 '이 줄이 그 디렉터리를 쓸 수 있다' 는 **보수적 양성**이지 정확 귀속의 증명이 아니다 —
    다른 변수 값의 일부가 우연히 우리 대상과 같은 디렉터리를 가리키면 과잉 거부가 된다(가용성 손해 · 안전 방향)."""
    out = []
    if not isinstance(text, str) or not isinstance(name, str):
        return out
    seen = set()
    needle = name + "="
    i = text.find(needle)
    while i >= 0 and len(out) < limit:
        if i == 0 or text[i - 1].isspace():
            rest = text[i + len(needle):]
            for j in range(len(rest) + 1):
                if len(out) >= limit or j > _MAX_PATH_PROBE:
                    break          # 길이 상한은 **소진**이다(그보다 긴 접두는 어떤 FS 에도 없다) — 절단이 아니다
                if j == len(rest) or rest[j].isspace():
                    c = rest[:j]
                    if c and c not in seen:
                        seen.add(c)
                        out.append(c)
        i = text.find(needle, i + 1)
    return out


def _ps_lines_torn(lines, name="CLAUDE_CONFIG_DIR"):
    """★R4(codex 설계 비평 잔여 major · 실증): ps 출력에 **레코드가 아닌 조각**(pid 로 시작하지 않는 줄)이 있고 그 조각이
    `name=` 를 담고 있는가. env 값 하나에 줄바꿈이 들어 있으면 `splitlines()` 가 그 프로세스의 레코드를 둘로 쪼갠다 —
    뒤 조각은 pid 가 없어 통째로 버려지고, 앞 조각만 보고 '검증된 0' 이 되던 구멍이다(codex 재현: `71 claude OTHER=x` +
    개행 + `CLAUDE_CONFIG_DIR=… HOME=/x` → 종전 (0,1,0)).
    **잃어버린 귀속만** 잡는다: 조각에 `CLAUDE_CONFIG_DIR=` 가 없으면 그 조각이 감춘 config 귀속도 없다(그 키는 ps 가 찍는
    고정 문자열이라 값 안의 줄바꿈으로 쪼개지지 않는다). 그래서 형상만 이상한 줄(파싱 불가 litter)은 판정을 오염시키지
    않는다 — 종전 R1 반례의 `junk` 줄이 그것이다. 하나라도 걸리면 그 출력 **전체**를 신뢰하지 않는다(정상
    `ps -o pid=,command=` 출력엔 헤더도 접힘도 없고 `-ww` 로 절단도 없어 전 줄이 pid 로 시작한다).
    한계(정직): 조각이 우연히 숫자로 시작하면 레코드로 보인다 — 그것까지는 못 막는다."""
    needle = name + "="
    for line in lines:
        s_ = line.strip()
        if not s_ or s_.split(None, 1)[0].isdigit():
            continue
        if needle in s_:
            return True
    return False


def _ps_target_ambiguous(config_dir):
    """★R4(리뷰 codex major): 대상 경로가 ps 판독기를 통과하면 **원문 그대로 복원되지 않는** 형상인가. 판정은 손으로
    고른 문자 목록이 아니라 **판독기와 같은 연산**(`_PS_ENV_SPLIT_RE` 분할 + `NAME=` 접두 제거 + `.strip()`)으로 한다:
      ①분할 — 경로 안 ' NAME=' 토큰(`/w/a X=y`)이면 값이 반으로 잘린다
      ②머리·꼬리 공백류 — `.strip()` 이 깎아 `CLAUDE_CONFIG_DIR=/a/b ` 가 `/a/b` 로 보인다(꼬리만이 아니다 · 머리 공백도
        같은 방향으로 틀린다)
      ③`splitlines()` 가 나누는 문자(개행·CR·수직탭·폼피드·정보구분자·NEL·U+2028/9) — ps 출력의 줄 자체가 끊긴다
    어느 쪽이든 **일치가 불일치로** 보여 그 줄이 '검증된 0' 이 된다(→ 라이브 config 에 쓴다). 원문과 동일성 키 양쪽을
    보고 하나라도 모호하면 모호다. 판정은 **불일치만** 접는다 — 양성(`_ps_env_value_present` 의 원문 대조)은 그대로
    양성이어야 `--force-unverified` 가 관측된 라이브 claude 를 넘지 못한다."""
    if not isinstance(config_dir, str):
        return True          # ★R4(codex 위임 반례): None/bytes 는 귀속 자체가 불가 — '검증된 0' 이 되면 안 된다
    for p in (config_dir, _path_identity(config_dir)):
        if not isinstance(p, str):
            continue
        if p.splitlines()[:1] != [p]:
            return True
        segs = _PS_ENV_SPLIT_RE.split("CLAUDE_CONFIG_DIR=" + p)
        if len(segs) != 1 or segs[0][len("CLAUDE_CONFIG_DIR="):].strip() != p:
            return True
    return False


def _count_claude_in_ps_lines(lines, config_dir, self_pids=(), argv_lines=None, same_dir=None):
    """darwin `ps -ww -E -o pid=,command=` 출력 순수 판정 — (count, parsed_lines, unresolved). 각 줄 = pid + [argv…] +
    [NAME=value …]. self_pids(자기·부모)는 제외.
    ★R3(리뷰 codex major): `ps -E` 한 줄엔 argv 와 env 사이 구분자가 없어 **인자 속** `CLAUDE_CONFIG_DIR=/other` 가 env 값을
    가렸다(첫 세그먼트 채택 → 검증된 0). 두 모드:
    ① argv_lines(같은 ps 의 env 없는 출력)가 있으면 그 pid 의 argv 문자열이 -E 줄의 **접두**여야 하고(경계 = 그 뒤 공백 1개)
       env 는 그 접미 전체다(실측: -E 줄 == argv 줄 + " " + env · env 비노출이면 == argv 줄). 판정 = argv 가 claude 실행 형상
       AND (접미의 CLAUDE_CONFIG_DIR= 값 중 하나가 config_dir 와 동일성 일치 · 또는 CLAUDE_CONFIG_DIR= 가 없고 config_dir 가
       기본 ~/.claude). 접두가 안 맞거나 argv 목록에 없는(두 호출 사이에 생긴 · pid 중복) claude 형상 줄과 env 비노출
       (접미 없음) argv[0] 엄격 형상은 unresolved.
    ② argv_lines 가 없으면(구분자 없는 모드 — 주입 검체 전용): `_PS_ENV_SPLIT_RE` 휴리스틱 경계. claude 형상(env 세그먼트가
       있으면 any-token · 없으면 argv[0] 엄격) 줄은 **검증된 0 이 되지 않는다** — CLAUDE_CONFIG_DIR= 어느 하나라도 일치하면
       양성 · 없고 기본 config 대상이면 양성 · 그 밖은 전부 unresolved(인자 속 NAME= 가 env 를 가릴 수 있으므로 · R3 codex).
       비-claude 형상 줄만 0.
    ★R4(리뷰 codex major): ①**양성**은 분할 결과가 아니라 원문 대조(`_ps_env_value_present`)로도 본다 — 꼬리/머리 공백·
       내부 ' NAME=' 대상에서도 관측이 살아 있어야 `--force-unverified` 가 관측된 라이브 claude 를 넘지 못한다.
       ②대상이 모호 형상(`_ps_target_ambiguous`)이거나 출력에 레코드 아닌 조각이 있으면(`_ps_lines_torn` — env 값 속
       줄바꿈이 레코드를 쪼갠 것) claude 형상 줄의 **불일치를 '검증된 0' 으로 삼지 않는다**(모드① 도 동일).
       보통 대상 + 정상 출력에서는 종전 판정 그대로다(검증된 0 보존 — 그것이 WP-2 의 주경로).
    ★R5(리뷰 codex major D1·D2): 값의 끝을 판독기로 확정할 수 없다는 사실을 판정에 반영한다 — 값 **후보 전부**
       (`_ps_env_value_candidates`)를 파일시스템 동일성 3값 판정(`same_dir` 주입점 · 기본 `_same_dir`)으로 대조한다:
       하나라도 True 면 양성 · 하나라도 None(조회 불가)이면 미해결 · 전부 False 라야 그 줄이 '검증된 0' 에 든다.
       대소문자/유니코드 별칭·비식별자 변수명·값 속 ' NAME=' 이 이 한 규칙으로 함께 닫힌다. 대상이 기본 config
       (~/.claude)인데 그 줄에 겉보기 지정이 있으면 **미해결**이다 — 그 지정이 다른 변수 값의 일부일 수 있어
       (직렬화가 같다) 그 프로세스가 실제로 기본 config 를 쓸 가능성을 배제할 수 없다(codex D2).
       판정 로직 자체는 순수하고, 파일시스템 조회는 `same_dir` 주입점 하나로 모인다."""
    lines = list(lines)
    same_dir = same_dir or _same_dir
    ident = _path_identity(config_dir)
    default_verdict = same_dir(config_dir, _default_claude_config_dir())
    ambiguous = _ps_target_ambiguous(config_dir) or _ps_lines_torn(lines)
    argv_map = _ps_argv_map(argv_lines) if argv_lines is not None else None
    n = 0
    parsed = 0
    unresolved = 0
    def _carries(text):
        """이 줄이 **우리 대상**을 env 값으로 달고 있는가(원문 바이트 · 세그먼트 경계 · 순수)."""
        return (_ps_env_value_present(text, "CLAUDE_CONFIG_DIR", config_dir)
                or _ps_env_value_present(text, "CLAUDE_CONFIG_DIR", ident))

    def _attribute(text):
        """claude 형상 줄의 귀속 → True(우리 대상 · 양성) · False(검증된 음성) · None(판정 불가 · 미해결)."""
        if _carries(text):
            return True
        cands = _ps_env_value_candidates(text)
        if not cands:
            # 지정이 안 보인다 = 기본 config 사용(양성/음성은 대상이 기본인가로 갈린다). 단 **찢긴 출력·모호 대상**
            # 에서는 값이 다른 조각에 있을 수 있어 '안 보인다' 를 근거로 쓰지 않는다(R4 핀 보존).
            if ambiguous:
                return None
            return True if default_verdict is True else (False if default_verdict is False else None)
        unknown = _ps_env_candidates_truncated(text)   # ★R5(codex 위임 반례): 잘린 열거의 불일치는 증명이 아니다
        for c in cands:
            v = same_dir(c, config_dir)
            if v is True:
                return True
            if v is None:
                unknown = True
        if unknown or ambiguous or default_verdict is not False:
            return None
        return False
    for line in lines:
        s_ = line.lstrip()          # ★R4(codex 위임 반례): rstrip 금지 — 줄 끝 공백은 **마지막 env 값의 바이트**일 수 있다
        if not s_:
            continue
        parts = s_.split(None, 1)
        if len(parts) < 2 or not parts[0].isdigit():
            # 레코드가 아닌 조각(찢긴 출력의 뒷조각). 그 조각이 우리 대상을 달고 있으면 어느 pid 의 것인지 알 수 없다 →
            # '검증된 0' 금지(★R4 codex 위임 반례: claude 설치 경로 안의 줄바꿈이 argv 자체를 쪼개는 경우도 여기서 잡힌다).
            if _carries(s_):
                unresolved += 1
            continue
        parsed += 1
        if parts[0] in self_pids:
            continue
        rest = parts[1]
        if argv_map is not None:
            a = argv_map.get(parts[0])
            if a is None or not (rest == a or rest.startswith(a + " ")):
                # argv 를 모른다(두 호출 사이에 생긴 프로세스 · pid 중복) · 접두 불일치 = 경계 미확정 → claude 형상이면 unresolved.
                # ★R4(codex 위임 반례): 그 줄이 **우리 대상 바이트**를 달고 있으면 경계를 몰라도 관측이다 — claude 형상이면
                #   양성(강행이 못 넘는다) · 아니면 미해결(귀속 불가). 경계를 모르니 argv 속 값도 env 로 본다(거부 방향).
                shaped = _is_claude_command(_PS_ENV_SPLIT_RE.split(rest)[0].split())
                if _carries(rest) and shaped:
                    n += 1                 # 형상 + 우리 대상 바이트 = 관측(양성) — 강행이 넘지 못한다
                elif _carries(rest) or shaped:
                    unresolved += 1        # 둘 중 하나만 = 귀속 불가(검증된 0 금지)
                continue
            cmd_tokens = a.split()
            env_str = rest[len(a) + 1:] if len(rest) > len(a) else ""
            env_segs = [e for e in (_PS_ENV_SPLIT_RE.split(env_str) if env_str else []) if e]
            if not env_segs:
                if _is_claude_command(cmd_tokens, strict=True):
                    unresolved += 1
                continue
            if not _is_claude_command(cmd_tokens):
                continue
            verdict = _attribute(env_str)         # ★R5: 원문 대조 → 값 후보 × 파일시스템 동일성(3값)
            if verdict is True:
                n += 1
            elif verdict is None:
                unresolved += 1        # 모호 대상/찢긴 출력/별칭 조회 불가 — '검증된 0' 으로 흡수 금지
            continue
        segs = _PS_ENV_SPLIT_RE.split(rest)
        cmd_tokens = segs[0].split()
        if len(segs) == 1:
            # env 비노출 줄: argv[0]/claude-code 엄격 형상만 unresolved(R2) — 인자 속 claude 토큰(tail/less/grep)은 형상 아님
            if _is_claude_command(cmd_tokens, strict=True):
                unresolved += 1
            continue
        if not _is_claude_command(cmd_tokens):
            continue
        if _attribute(rest) is True:
            n += 1
        else:
            unresolved += 1                # 구분자 없는 모드의 claude 형상은 검증된 0 이 될 수 없다(R3 codex)
    return n, parsed, unresolved


def _count_claude_procfs(config_dir, proc_root="/proc"):
    """linux /proc/<pid>/{environ,cmdline} 스캔 — (count|None, detail). 권한 밖(EACCES)은 **소유자를 확인**한다(codex R1):
    같은 uid 인데 못 읽으면(비덤프 프로세스) 미해결 · 다른 uid 는 범위 외(cys-dept 와 claude 는 같은 사용자) · uid 판정 불가도
    미해결. 스캔 중 종료(ENOENT)=무시 · 그 외 판독 실패=미해결. ★양성 관측 n>0 은 미해결보다 먼저 반환한다(--force-unverified 가
    관측된 라이브 claude 를 넘지 못하게)."""
    # ★R5(리뷰 codex D1): 표기 비교 하나가 아니라 3값 파일시스템 동일성(_same_dir) — 조회 불가(EACCES/ESTALE)는
    #   '다름' 이 아니라 **미해결**이다(검증된 0 금지). /proc 은 값 경계가 NUL 로 확정돼 있어 후보 열거는 필요 없다.
    default_verdict = _same_dir(config_dir, _default_claude_config_dir())
    try:
        names = os.listdir(proc_root)
    except OSError as e:
        return None, "linux: %s 판독 불가(%s)" % (proc_root, e)
    self_pids = {str(os.getpid()), str(os.getppid())}
    getuid = getattr(os, "getuid", None)
    my_uid = getuid() if getuid else None
    n = scanned = unresolved = 0
    for pid in names:
        if not pid.isdigit() or pid in self_pids:
            continue
        pdir = os.path.join(proc_root, pid)
        try:
            with open(os.path.join(pdir, "environ"), "rb") as f:
                env = f.read()
            with open(os.path.join(pdir, "cmdline"), "rb") as f:
                cmd = f.read()
        except PermissionError:
            try:
                owner = os.stat(pdir).st_uid
            except OSError:
                owner = None
            if my_uid is None or owner is None or owner == my_uid:
                unresolved += 1
            continue
        except FileNotFoundError:
            continue
        except OSError:
            unresolved += 1
            continue
        scanned += 1
        # ★R5(리뷰 codex major): 디코드는 **파일시스템 규약**(os.fsdecode = surrogateescape)이어야 한다. 종전
        #   `decode("utf-8","replace")` 는 비-UTF8 바이트를 U+FFFD 로 바꿔, 같은 디렉터리를 가리키는 라이브 claude 가
        #   불일치로 보이고 '검증된 0'(→ 라이브 config 에 쓰기)이 됐다 — 파이썬 경로 문자열은 그 바이트를 서로게이트로
        #   보존하므로 양쪽 표기가 갈렸다. fsdecode 는 왕복 보존이라 대상 문자열과 바이트 단위로 대조된다.
        if not _is_claude_command([os.fsdecode(t) for t in cmd.split(b"\0") if t]):
            continue
        cfg_val = None
        for e in env.split(b"\0"):
            if e.startswith(b"CLAUDE_CONFIG_DIR="):
                cfg_val = os.fsdecode(e[len(b"CLAUDE_CONFIG_DIR="):])
                break
        if cfg_val is not None:
            v = _same_dir(cfg_val, config_dir)
            if v is True:
                n += 1
            elif v is None:
                unresolved += 1        # 별칭 조회 불가(EACCES/ESTALE) → 검증된 0 금지. /proc 은 NUL 경계라 값 자체는
                #                        확정이다 — ps 의 '겉보기 지정' 모호(codex D2)는 여기서 발생하지 않는다.
        elif default_verdict is True:
            n += 1
        elif default_verdict is None:
            unresolved += 1
    if n > 0:
        return n, "linux: /proc %d건(미해결 %d)" % (scanned, unresolved)
    if unresolved:
        return None, "linux: /proc 미해결 %d건(판독 실패·같은 uid 권한 거부)" % unresolved
    if scanned == 0:
        return None, "linux: /proc 판독 0건"
    return 0, "linux: /proc %d건" % scanned


def claude_procs_for_config(config_dir, runner=None, os_name=None, platform=None, proc_root="/proc"):
    """그 CLAUDE_CONFIG_DIR 로 도는 claude 프로세스 수 → (count|None, detail). **None = 검증 불가**(호출자는 거부).
    darwin: `ps -ax -ww -E`(전 프로세스 · env 노출 · -ww 로 절단 0) 구조 매칭 — claude 형상인데 env 비노출 줄이 있고 양성 0 이면
    None(귀속 불가 · codex R1) · linux: /proc 스캔 · nt: Win32_Process 는 env 미노출 → claude/node 실행 형상 전역 계수 0 만
    '검증된 음성', ≥1 은 config dir 귀속 불가 → None(codex 4·5: 다부서 상시 운용 중 Windows 는 --force-unverified 없이는
    거부되며 2차 방어(관문 보류)가 받는다 — 고지된 degraded mode). 분기 조건은 os_name/platform 인자뿐(rc 로 플랫폼을 추정하지
    않는다). ★양성 n>0 은 미해결보다 우선 반환(강행 플래그가 관측된 라이브 claude 를 넘지 못하게)."""
    os_name = os_name or os.name
    platform = platform or sys.platform
    runner = runner or _run_capture
    if os_name == "nt":
        rc, out, _e = runner(["powershell", "-NoProfile", "-NonInteractive", "-Command",
                              _WIN_CLAUDE_COUNT_PS_TMPL % os.getpid()])
        if rc != 0:
            return None, "windows: powershell 실패(rc=%s)" % rc
        lines = [ln.strip() for ln in (out or "").splitlines() if ln.strip()]
        if not lines or not lines[-1].isdigit():
            return None, "windows: powershell 출력 비숫자"
        cnt = int(lines[-1])
        if cnt == 0:
            return 0, "windows: claude/node 프로세스 0(전역 · 검증된 음성)"
        return None, "windows: claude/node 프로세스 %d — config dir 귀속 불가(Win32_Process env 미노출)" % cnt
    if platform.startswith("linux"):
        return _count_claude_procfs(config_dir, proc_root)
    if platform == "darwin":
        # -ax: 전 프로세스(tty 없는 좌석·데몬 자식 포함 — 기본은 **자기 세션만** 보여 라이브 claude 를 놓친다 · 실측 2026-09-06)
        # -ww: 폭 절단 0(env 는 줄 끝에 실린다) · -E: 환경 노출(같은 사용자 프로세스만 — cys-dept 와 claude 는 같은 사용자)
        # ★R3: argv 만 보는 ps 를 **먼저** 찍어 -E 줄의 argv/env 경계를 접두 대조로 확정한다(인자 속 NAME=value 가 env 를
        #   가리지 못하게). 두 호출 사이에 생긴 프로세스는 argv 를 모르므로 claude 형상이면 unresolved(거부 방향).
        #   env 값 사이의 경계는 여전히 ' NAME=' 휴리스틱이다 — 대상 경로 자체가 그 형상(`/tmp/work X=y`)이면 값이 잘린다.
        # ★R4(codex 설계 비평): 그 경우 종전엔 **스캔 前에** None 을 돌려줬는데, `--force-unverified` 는 unverified 를 넘기므로
        #   그 대상에서는 라이브 claude 를 '관측' 할 기회 자체가 사라져 강행이 살아 있는 좌석의 config 를 덮을 수 있었다.
        #   이제는 스캔을 **하고** 분류한다: 양성은 원문 바이트 대조(`_ps_env_value_present`)로 그대로 관측되고(강행 불가),
        #   불일치는 `_ps_target_ambiguous` 가 unresolved 로 접어 None(거부)이 된다 — 판정은 같거나 더 안전하고, claude 가
        #   하나도 없으면 '검증된 0'(정당한 음성)이 나온다.
        rc0, out0, _e0 = runner(["ps", "-ax", "-ww", "-o", "pid=,command="])
        if rc0 != 0 or not out0.strip():
            return None, "darwin: ps -ax(argv) 실패(rc=%s)" % rc0
        rc, out, _e = runner(["ps", "-ax", "-ww", "-E", "-o", "pid=,command="])
        if rc != 0 or not out.strip():
            return None, "darwin: ps -axE 실패(rc=%s)" % rc
        argv_lines = out0.splitlines()
        n, parsed, unresolved = _count_claude_in_ps_lines(out.splitlines(), config_dir,
                                                          {str(os.getpid()), str(os.getppid())}, argv_lines=argv_lines)
        if parsed == 0:
            return None, "darwin: ps 출력 파싱 0줄"
        n_argv = len(_ps_argv_map(argv_lines))
        if n > 0:
            return n, "darwin: ps -E %d줄(argv 대조 %d · 미확정 claude 형상 %d)" % (parsed, n_argv, unresolved)
        if unresolved:
            return None, "darwin: claude 형상 %d건의 env 비노출/경계 미확정 — config 귀속 불가" % unresolved
        return 0, "darwin: ps -E %d줄(argv 대조 %d)" % (parsed, n_argv)
    return None, "미지원 플랫폼(%s/%s)" % (os_name, platform)


def _open_nofollow(path, flags, mode=0o600):
    """심링크 무추종 open(가능한 플랫폼에서) — Windows 는 O_NOFOLLOW 부재 → 호출자의 _is_link_like 검사가 방벽."""
    return os.open(path, flags | getattr(os, "O_NOFOLLOW", 0), mode)


def _open_unblocking_ro(path, nofollow=False):
    """★R4(리뷰 codex major): **막히지 않는** 읽기 전용 open → (fd, st). `O_NONBLOCK` 으로 writer 없는 FIFO 의 무한 대기를
    막고(정규 파일엔 무해), 연 **뒤** `fstat` 으로 정규 파일을 재확인한다 — lstat 선검사와 open 사이에 FIFO 로 바뀌는
    TOCTOU 까지 닫는다. 비정규(FIFO·장치·디렉터리)는 ValueError.
    한계(정직): 이것이 닫는 것은 **FIFO open 대기**다 — 응답 없는 원격 FS(NFS)의 조회·read 지연 같은 '무한정 느림' 은
    닫지 못한다(그 층의 상한은 `--seed-trust` CLI 의 마감 감시 `CYS_SEED_TRUST_TIMEOUT`). Windows 는 O_NONBLOCK/
    O_NOFOLLOW 가 없어 getattr 0 — 명명 파이프는 열리더라도 fstat 이 정규가 아니라 거부된다.
    ★R5(리뷰 minor): `os.open` 은 Windows 에서 `O_BINARY` 를 주지 않으면 CRT 기본이 텍스트 모드다(CRLF 변환 · 0x1A
    EOF 절단) — 종전 `open(path,"rb")` 에서 이 함수로 옮기며 바이트 정확성을 잃었다(sha256/CAS 계약과 같은 계층).
    getattr 0 이라 POSIX 는 무영향."""
    flags = os.O_RDONLY | getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_BINARY", 0)
    if nofollow:
        flags |= getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(path, flags)
    try:
        st = os.fstat(fd)
        if not stat.S_ISREG(st.st_mode):
            raise ValueError("정규 파일이 아니다(FIFO/장치/디렉터리) 거부: %s" % path)
    except BaseException:
        os.close(fd)
        raise
    return fd, st


def _read_claude_json_bytes(cfg):
    """(existed, raw, st) — 부재 = (False, b"", None). 심링크/정션은 ValueError · IO 는 OSError 를 올린다(호출자 ERROR)."""
    if not os.path.lexists(cfg):
        return False, b"", None
    if _is_link_like(cfg):
        raise ValueError("symlink 거부: %s" % cfg)
    if not stat.S_ISREG(os.lstat(cfg).st_mode):
        raise ValueError("정규 파일이 아니다(FIFO/장치/디렉터리) 거부: %s" % cfg)   # FIFO 는 open 에서 막힌다 — 열기 전에 거른다(R2)
    fd, _st0 = _open_unblocking_ro(cfg, nofollow=True)   # ★R4: 선검사~open 사이 FIFO 교체(TOCTOU)도 막는다
    with os.fdopen(fd, "rb") as f:
        raw = f.read()
        st = os.fstat(f.fileno())
    return True, raw, st


def _parse_claude_json(existed, raw):
    """부재 → {} · 빈 파일(0B/공백) → {}(기존 파일은 유지·시드된 유효 문서로) · 그 외 JSON. 파싱 실패는 ValueError."""
    if not existed or not raw.strip():
        return {}
    return json.loads(raw.decode("utf-8-sig"))


class _ExchangeUnavailable:
    """`_exchange_paths` 의 '기구 부재' 반환값 — falsy · 사유(why)를 호출별로 담는다(R3 codex: 전역 가변 상태 대신 호출별 진단)."""
    __slots__ = ("why",)

    def __init__(self, why):
        self.why = why

    def __bool__(self):
        return False

    def __repr__(self):
        return "ExchangeUnavailable(%s)" % self.why


def _exchange_paths(a, b):
    """두 경로의 **원자 교환**(a ↔ b) → True(교환됨) · falsy `_ExchangeUnavailable(why)`(기구 부재 — 호출자는 REFUSE) ·
    OSError(실 오류). darwin: renamex_np(from, to, RENAME_SWAP=0x2)(macOS 10.12+ · APFS/HFS+) · linux: renameat2(AT_FDCWD,
    from, AT_FDCWD, to, RENAME_EXCHANGE=2)(커널 3.15+ · glibc 2.28+ 심볼) · 그 외(Windows 포함) 부재. 미지원 FS(ENOTSUP/
    EOPNOTSUPP/EINVAL/ENOSYS/EXDEV)도 부재(사유 = errno 이름 · EINVAL 은 '미지원 플래그' 와 '인자 오류' 가 겹친다). ctypes 는
    함수 안에서만 import(Windows 경로는 import 전에 부재)."""
    if os.name != "posix":
        return _ExchangeUnavailable("platform:%s" % os.name)
    try:
        import ctypes
        import ctypes.util
        libc = ctypes.CDLL(ctypes.util.find_library("c") or None, use_errno=True)
    except Exception as e:  # noqa: BLE001 — 기구 부재는 거부 사유이지 실패가 아니다
        return _ExchangeUnavailable("ctypes:%s" % type(e).__name__)
    if sys.platform == "darwin":
        fn = getattr(libc, "renamex_np", None)
        if fn is None:
            return _ExchangeUnavailable("renamex_np 심볼 부재")
        fn.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
        fn.restype = ctypes.c_int
        rc = fn(os.fsencode(a), os.fsencode(b), 0x2)
    elif sys.platform.startswith("linux"):
        fn = getattr(libc, "renameat2", None)
        if fn is None:
            return _ExchangeUnavailable("renameat2 심볼 부재(glibc<2.28)")
        fn.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
        fn.restype = ctypes.c_int
        rc = fn(-100, os.fsencode(a), -100, os.fsencode(b), 2)     # AT_FDCWD = -100
    else:
        return _ExchangeUnavailable("platform:%s" % sys.platform)
    if rc == 0:
        return True
    err = ctypes.get_errno()
    if err in (errno.ENOTSUP, errno.EOPNOTSUPP, errno.EINVAL, errno.ENOSYS, errno.EXDEV):
        return _ExchangeUnavailable(errno.errorcode.get(err, str(err)))
    raise OSError(err, "%s: exchange(%s <-> %s)" % (os.strerror(err), a, b))


def _unlink_quiet(path):
    try:
        os.unlink(path)
    except OSError:
        pass


# 디렉터리 fsync 가 **원리적으로 없는** 플랫폼/FS 의 errno — 이것만 strict 에서 통과시킨다(EIO·ENOSPC 는 통과 못 한다).
_DIR_FSYNC_UNSUPPORTED = tuple(e for e in (getattr(errno, "EINVAL", None), getattr(errno, "ENOTSUP", None),
                                           getattr(errno, "EOPNOTSUPP", None), getattr(errno, "ENOSYS", None))
                               if e is not None)


def _fsync_dir(path, strict=False):
    """디렉터리 fsync(rename/link 의 내구성 · POSIX 만).
    기본(strict=False)은 **부수 보장**이라 실패를 무시한다 — 커밋이 이미 성립한 뒤의 내구성 강화용.
    ★R6(리뷰 codex major): 의도 저널은 '이름이 교환보다 먼저 내구적' 이어야 하는 **의무**다 — 거기서 이 함수가
    EIO 까지 삼키면 저널 없이 교환을 여는 것과 같아진다. `strict=True` 는 **open 실패를 언제나 전파**하고
    (codex R6: open 단계의 EINVAL 은 'fsync 미지원' 의 증거가 아니다), fsync 단계만 `_DIR_FSYNC_UNSUPPORTED`
    를 통과시킨다. 그 통과의 대가는 정직하게 고지한다 — 디렉터리 fsync 가 없는 FS 에서 전원 장애까지의 저널
    내구성은 보장되지 않는다(프로세스 사망까지는 보장된다: 이름은 이미 보인다).
    Windows(os.name != posix)는 종전대로 무동작이다 — 여기서 예외를 올리면 `exchange-unavailable` REFUSE 에
    닿기도 전에 ERROR 로 끝나 계약이 바뀐다(codex R6)."""
    if os.name != "posix":
        return
    try:
        dfd = os.open(path, os.O_RDONLY)
    except OSError:
        if strict:
            raise
        return
    try:
        os.fsync(dfd)
    except OSError as e:
        if strict and e.errno not in _DIR_FSYNC_UNSUPPORTED:
            raise
    finally:
        os.close(dfd)


def _lexists_strict(path):
    """`os.lstat` 로 확정하는 존재 여부 → True(있다) · False(**증명된** 부재) · None(조회 실패 = 모른다).
    ★R6(리뷰 codex): `os.path.lexists` 는 EACCES/EIO/ESTALE 를 '없다' 로 접는다 — 회수 판정에서 그 접힘은 곧
    '지워도 된다' 가 된다(결측은 값이 아니다). 부재류 분류는 `_stat_ident` 와 같은 집합이다."""
    try:
        os.lstat(path)
        return True
    except OSError as e:
        absent = (getattr(errno, "ENOENT", None), getattr(errno, "ENOTDIR", None),
                  getattr(errno, "ELOOP", None), getattr(errno, "ENAMETOOLONG", None))
        return False if e.errno in absent else None
    except ValueError:
        return False                 # 널 바이트 등 — 경로가 될 수 없는 문자열


_EXCLUSIVE_NAME_MAX_TRIES = 512   # 후보 상한 — 이름에 이미 초 단위 utc + pid 가 들어 있어 정상 경합은 한 자리다


def _exclusive_name(prefix, lexists=None):
    """충돌 없는 보존 이름 — <prefix><utc>-<pid>[-<n>] · 이미 있으면 n 증가(기존 보존 파일을 덮지 않는다).
    ★R7(리뷰 claude minor): 존재 판정을 `os.path.lexists` 가 아니라 3값 `_lexists_strict` 로 한다 — lexists 는
    EACCES/EIO/ESTALE 를 '없다' 로 접는데, 이 함수의 반환값은 곧 `os.rename`/`os.replace` 의 **대상**이라(교환의
    displaced · 되교환 창의 conflict) 접힌 '부재' 가 말없는 덮어쓰기 = 사람이 병합해야 할 낯선 문서의 파괴가 된다.
    '있다(True)' 와 '모른다(None)' 는 **둘 다** 다음 후보로 넘긴다(결측은 값이 아니다 · `_lexists_strict` 와 같은 규율).
    상한까지 **증명된 부재**를 못 찾으면 OSError — 호출자는 전부 '보존 이름을 못 얻었다 = 쓰지 않는다' 로 접는다."""
    lexists = lexists or _lexists_strict
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    base = "%s%s-%d" % (prefix, stamp, os.getpid())
    for n in range(_EXCLUSIVE_NAME_MAX_TRIES):
        cand = base if n == 0 else "%s-%d" % (base, n)
        if lexists(cand) is False:              # 증명된 부재에서만 그 이름을 쓴다(True·None 은 다음 후보)
            return cand
    raise OSError(errno.EEXIST, "보존 이름 확정 실패(후보 %d 가 전부 존재하거나 조회 불가): %s"
                  % (_EXCLUSIVE_NAME_MAX_TRIES, base))


def _payload_digest(payload_b):
    """displaced 이름에 박는 payload 지문 — sha256 64hex(순수)."""
    return hashlib.sha256(payload_b).hexdigest()[:_SEED_DISPLACED_DIGEST_LEN]


def _sweep_stale_seed_tmp(config_dir):
    """잠금 보유 중에만 호출 — 죽은 시더(SIGKILL)의 잔재를 치운다 → 청소 수. 대상 ①mkstemp 잔재(.claude.json.seed-<8자>)
    ②★R3 displaced(.claude.json.displaced-<sha256>-… · 교환 직전 사망 = 우리 payload 만 든 파일) 중 **바이트의 sha256 이 이름의
    지문과 같은 것** — 이는 'payload 와 바이트 동등' 의 회수이지 소유 증명이 아니다: 교환 뒤 옛 inode(낯선 바이트)가 앉은
    displaced 는 지문이 달라 무접촉(낯선 데이터의 유일 사본일 수 있다) · 심링크/정션/FIFO/비정규/판독 불가 후보도 무접촉.
    잔여(고지): 해시~unlink 사이에 그 inode 를 fd 로 잡고 고쳐 쓰는 기록자는 이 판정이 못 본다(_restore_foreign 과 같은
    in-place 기록자 한계 · 알려진 기록자 중 없음). 잠금 파일·.bak-*·.conflict-*·지문 없는 구형 displaced 는 대상이 아니다.
    실패는 무시(청소는 부수 효과)."""
    swept = 0
    try:
        names = os.listdir(config_dir)
    except OSError:
        return 0
    # ★R6(리뷰 codex '추가로 놓친 것' ②): displaced 지문 청소는 **활성 문서가 유효할 때만** 한다. payload 를
    #   displaced 로 rename 한 뒤 저널 공개 전에 죽으면 저널 없는 payload 잔재가 남는데, 그 내용은 '원본 + 플래그'
    #   라 그 사이 활성이 잘렸다면 그것이 유일한 완전한 문서다(회수 ⓑ 와 같은 손실). mkstemp 잔재(.seed-<8자>)는
    #   공개된 적 없는 순수 임시파일이라 이 조건과 무관하게 치운다.
    healthy = None                     # 지연 판정 — displaced 후보가 없으면 판독하지 않는다
    for n in names:
        path = os.path.join(config_dir, n)
        if _SEED_TMP_LITTER_RE.match(n) and n != SEED_TRUST_LOCK_NAME:
            try:
                os.unlink(path)
                swept += 1
            except OSError:
                pass
            continue
        m = _SEED_DISPLACED_RE.match(n)
        if not m:
            continue
        if healthy is None:
            healthy = _active_document_healthy(os.path.join(config_dir, ".claude.json"))
        if not healthy:
            continue                                       # 유일한 완전한 사본일 수 있다 — 무접촉(사람이 병합)
        try:
            if _is_link_like(path) or not stat.S_ISREG(os.lstat(path).st_mode):
                continue
            fd, _st = _open_unblocking_ro(path, nofollow=True)   # ★R4: 선검사~open 사이 FIFO 교체도 막힘 0
            with os.fdopen(fd, "rb") as f:
                data = f.read()
        except (OSError, ValueError):
            continue
        if _payload_digest(data) != m.group(1):
            continue                                       # 낯선 inode — 보존(수동 병합 대상)
        try:
            os.unlink(path)
            swept += 1
        except OSError:
            pass
    return swept


def _digest_of_regular(path):
    """정규 파일(무추종)의 sha256 → 64hex · 판독 불가/비정규/심링크는 None. 잠금 아래 호출."""
    try:
        if _is_link_like(path) or not stat.S_ISREG(os.lstat(path).st_mode):
            return None
        fd, _st = _open_unblocking_ro(path, nofollow=True)
        with os.fdopen(fd, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()
    except (OSError, ValueError):
        return None


def _write_seed_intent(config_dir, displaced_name, captured_digest, payload_digest):
    """교환 **전** 의도 저널을 내구적으로 공개 → 저널 경로. 실패는 OSError(호출자는 교환하지 않는다).
    ★R5(리뷰 codex major): 교환~검증 창에서 시더가 죽으면(마감 감시 `os._exit`·SIGKILL) 상대의 **더 새 문서**가
    displaced 이름에 남고 우리 payload 가 활성이 된다 — 다음 실행은 플래그가 이미 있으니 `already-trusted` 로 조용히
    OK 를 냈다(고립된 사용자 문서 · 아무도 모른다). 저널이 그 창을 **내구적으로** 표시한다.
    담는 것: displaced 이름(basename) · 우리가 읽은 원본 바이트의 sha256 · payload sha256 · pid/utc(진단).
    지문 하나로는 트랜잭션 단계를 증명할 수 없으므로(codex D4) 회수는 **두 파일의 증거**로 판정한다."""
    payload = json.dumps({"v": 1, "displaced": displaced_name, "captured_sha256": captured_digest,
                          "payload_sha256": payload_digest, "pid": os.getpid(),
                          "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())},
                         ensure_ascii=True).encode("utf-8")
    fd, tmp = tempfile.mkstemp(prefix=SEED_TRUST_TMP_PREFIX, dir=config_dir)
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(payload)
            f.flush()
            os.fsync(f.fileno())          # 내구성 실패는 여기서 예외 = 교환 안 함(codex D4)
        path = _exclusive_name(os.path.join(config_dir, SEED_TRUST_INTENT_PREFIX))
        os.link(tmp, path)                # 완성된 내용을 한 번에 공개(부분 저널 0)
    except BaseException:
        _unlink_quiet(tmp)
        raise
    _unlink_quiet(tmp)
    try:
        _fsync_dir(config_dir, strict=True)   # 저널의 **이름**이 교환보다 먼저 내구적이어야 한다(★R6: 실패를 삼키지 않는다)
    except OSError:
        _unlink_quiet(path)                   # 내구성을 못 세웠다 = 창을 열지 않는다 — 표시도 남기지 않는다
        raise
    return path


def _read_seed_intent(path):
    """저널 판독 → dict(검증 통과) 또는 None(부재·비정규·심링크·손상·형식 위반). 관대하지 않다 — 형식이 어긋나면
    '모른다' 이고 호출자는 **보존 + 거부** 로 간다."""
    try:
        if _is_link_like(path) or not stat.S_ISREG(os.lstat(path).st_mode):
            return None
        fd, st_ = _open_unblocking_ro(path, nofollow=True)
        with os.fdopen(fd, "rb") as f:
            raw = f.read(_SEED_INTENT_MAX_BYTES + 1)
    except (OSError, ValueError):
        return None
    if len(raw) > _SEED_INTENT_MAX_BYTES:
        return None
    try:
        rec = json.loads(raw.decode("utf-8"))
    except (ValueError, UnicodeDecodeError):
        return None
    if not isinstance(rec, dict):
        return None
    d = rec.get("displaced")
    if not isinstance(d, str) or os.path.basename(d) != d or not _SEED_DISPLACED_RE.match(d):
        return None                        # 경로 탈출(`../victim`)·타 네임스페이스 참조 금지(codex D4)
    for k in ("captured_sha256", "payload_sha256"):
        v = rec.get(k)
        if not isinstance(v, str) or not _SEED_HEX64_RE.match(v):
            return None
    return rec


def _active_document_healthy(cfg):
    """활성 `.claude.json` 이 **유효한 문서**인가(존재 + 실제로 파싱된 JSON 객체). displaced 를 지워도 되는지의
    전제다 — 활성이 없거나 깨졌으면 displaced 가 유일한 유효 사본일 수 있다(codex D4: 지문 동등만으로 삭제를
    인가하지 않는다).
    ★R6(리뷰 codex major): 종전엔 `_parse_claude_json` 을 썼다 — 그것은 **0바이트/공백을 `{}` 로** 바꾼다(시더의
    '빈 문서에도 시드한다' 관용). 그 관용을 **보존 사본 삭제의 근거**로 쓰면 중단 뒤 잘린 활성 문서가 '건강' 이
    되어 유일한 완전한 사본을 지운다. 여기서는 빈 바이트·공백·비객체 JSON 전부 **불건강**이다(거부 방향).
    판독 실패(권한·IO·심링크·비정규)도 불건강 — '못 봤다' 를 '건강하다' 로 읽지 않는다.
    `utf-8-sig` 디코딩은 시더와 같게 유지한다(BOM 있는 실 문서를 불건강으로 오판하지 않는다 · codex R6)."""
    try:
        existed, raw, _st = _read_claude_json_bytes(cfg)
    except (OSError, ValueError):
        return False
    if not existed or not raw.strip():
        return False
    try:
        return isinstance(json.loads(raw.decode("utf-8-sig")), dict)
    except (ValueError, UnicodeDecodeError):
        return False


def _active_digest(cfg):
    """활성 `.claude.json` 의 바이트 sha256(64hex) → 부재/판독 실패는 None(모른다)."""
    try:
        existed, raw, _st = _read_claude_json_bytes(cfg)
    except (OSError, ValueError):
        return None
    return _payload_digest(raw) if existed else None


def _copy_is_redundant(cfg, digests):
    """보존 사본을 지워도 **잃는 것이 없다** 는 증명 — 활성 문서의 바이트가 주어진 지문(우리가 읽은 원본 · 우리 payload)
    중 하나와 같은가. 판독 실패·불일치는 False(모르면 보존한다).
    ★R7(리뷰 codex): 지문 동등은 '예전에 우리가 만든 것과 같다' 만 증명할 뿐 **활성 문서에 그 데이터가 있다** 는
    증거가 아니다 — 중단 뒤 외부가 활성을 `{}` 로 재생성하면 종전 규칙은 유일한 사용자 사본을 삭제했다."""
    d = _active_digest(cfg)
    return d is not None and d in digests


def _preserve_copy(path):
    """보존 사본을 **자동 삭제되지 않는 이름**(`.claude.json.conflict-*`)으로 옮긴다 → 새 basename 또는 None(실패).
    ★R7(리뷰 codex major): `.displaced-<지문>-*` 는 지문 청소의 대상이라 '보호' 가 다음 실행까지 살아남지 못한다 —
    활성 문서가 (우리든 claude 든) 새로 만들어져 '건강' 해지는 순간 그 사본은 지워도 되는 잔재가 된다. conflict
    네임스페이스는 어느 경로에서도 자동 삭제되지 않으므로 **이름을 옮기는 것 자체가 내구적 보호 표시**다(별도 marker
    파일을 만들지 않는 이유: 그 표시 자체가 또 하나의 내구성·청소 대상이 된다). 바이트는 건드리지 않는다(rename 하나).
    잔여(고지): 이름 선택은 예약이 아니다 — 잠금 밖의 제3자가 그 마이크로초에 같은 이름을 만들면 rename 이 덮는다
    (그 이름은 우리 pid·utc 를 담고 알려진 기록자 중 만드는 자가 없다)."""
    try:
        target = _exclusive_name(os.path.join(os.path.dirname(path), SEED_TRUST_CONFLICT_PREFIX))
        os.rename(path, target)
    except OSError:
        return None
    return os.path.basename(target)


def _release_uncommitted_copy(cfg, dpath, jpath):
    """교환이 **성립하지 않은** 자리에서 우리 payload 사본(displaced)과 의도 저널을 처분 → 사유 꼬리.
    ★R7(리뷰 codex): 종전 근거 '교환 안 됨 = 사본을 지워도 안전' 은 성립하지 않는다 — 교환이 ENOENT 로 실패하는
    바로 그 형상(그 사이 외부가 활성을 지웠다)에서 이 사본(원본 + 플래그)이 **유일한 완전한 문서**다.
    판정은 '활성 문서가 유효한 문서로 있는가' 하나다: 있으면(누가 썼든) 우리 사본은 그 문서보다 낡은 스냅샷이므로
    지운다 — 여기서 바이트 동등까지 요구하면 커밋 순간 끼어든 정상 기록자마다 conflict 파일이 쌓인다(Windows 는
    기존 문서 경로가 **언제나** 이 자리라 잔재가 상시화된다). 없거나 깨졌으면 보존 이름으로 옮긴다.
    잔여(고지): 외부가 지운 뒤 그 마이크로초에 다시 만들면(healthy) 낡은 스냅샷으로 보고 지운다 — 저널로 보호되는
    **공개된** 사본(회수 ⓑⓒ)과 달리 이 사본은 공개된 적이 없고 창은 syscall 두 개 폭이다."""
    if _active_document_healthy(cfg):
        _unlink_quiet(dpath)
        _unlink_quiet(jpath)
        return ""
    kept = _preserve_copy(dpath)
    _unlink_quiet(jpath)                       # 사본 처분이 끝났다 = 이 트랜잭션의 표시는 소임을 다했다
    if kept:
        return " · 사본 보존(%s — 활성 .claude.json 이 유효하지 않다 · 사람이 병합)" % kept
    return " · 사본 보존(%s — 보존 이름으로 옮기지 못해 그 이름 그대로 둔다)" % os.path.basename(dpath)


def _orphan_recovery_guard(config_dir, cfg):
    """잠금 아래 · 저널 회수 **뒤** · 청소/계획 **앞**: 저널 없는 보존 사본(`.claude.json.displaced-<지문>-*`)이 있는데
    활성 문서가 없거나 유효하지 않으면 → 사본을 **자동 삭제되지 않는 이름으로 옮긴 뒤** REFUSE. 반환: None(계속) 또는
    (rc, verdict, reason).
    ★R7(리뷰 codex major '재시도가 보호된 회수 사본을 지워도 되는 잔재로 바꾼다'): 종전엔 이 상태에서 그대로 진행해
    **활성 부재 = link 경로**로 '플래그만 든 새 문서' 를 만들고 OK 를 냈다. 다음 실행은 그 새 문서를 '건강' 으로 읽고
    지문 일치 사본을 청소한다 — 두 번의 성공적인 재시도가 사용자 필드를 영구히 지운다. 여기서 멈추는 이유는 그
    **대체 문서 생성**이 손실을 인가하기 때문이다.
    거부는 1회로 끝난다(다음 실행에는 displaced 가 없다 = 이 가드가 통과한다) — 영구 부트 마찰이 아니다. 호출자
    (cys-dept)는 fail-open WARN 이고 관문 1회 수동 신뢰가 2차 방어다."""
    R = SEED_TRUST_REFUSE
    try:
        orphans = sorted(n for n in os.listdir(config_dir) if _SEED_DISPLACED_RE.match(n))
    except OSError as e:
        return (R, "REFUSE", "orphan-scan-failed(%s — 보존 사본의 유무를 관측할 수 없다 · 무접촉 · 사람이 확인)"
                % (errno.errorcode.get(e.errno, str(e.errno)) if e.errno else e))
    if not orphans or _active_document_healthy(cfg):
        return None                            # 사본이 없으면 활성 문서를 **읽지도 않는다**(주경로 판독 횟수 불변)
    # 대상은 **지문 청소가 지울 수 있는 것**뿐이다(바이트의 sha256 = 이름의 지문 = 우리 payload 동등). 지문이 다른
    #   displaced(교환 뒤 옛 inode = 낯선 문서)는 어느 경로에서도 자동 삭제되지 않으므로 이미 안전하고, 그것 때문에
    #   막으면 사람이 손대기 전까지 **매번** 거부하게 된다(1회 계약 위반 · 부트 마찰).
    at_risk = []
    for n in orphans:
        m = _SEED_DISPLACED_RE.match(n)
        if m and _digest_of_regular(os.path.join(config_dir, n)) == m.group(1):
            at_risk.append(n)
    if not at_risk:
        return None
    kept = [(_preserve_copy(os.path.join(config_dir, n)) or n) for n in at_risk]
    return (R, "REFUSE", "unresolved-recovery(활성 .claude.json 이 없거나 유효하지 않은데 중단된 교환의 보존 사본 %d건이 "
                         "있다 — 그 사본이 유일한 완전한 문서일 수 있어 대체 문서를 만들지 않는다 · 자동 삭제되지 않는 "
                         "이름으로 보존: %s · 사람이 병합한 뒤 그 파일을 지운다)" % (len(at_risk), ", ".join(kept)))


def _journal_lines(journals):
    """C58 저널 점검의 사람이 읽는 줄 → (잔존, 회수됨). 잔존·판독 불가·회수 완료를 **다른 문장**으로 낸다 —
    침묵도 뭉뚱그림도 없다(★R6 리뷰 codex major: 미해결 트랜잭션을 안고 PASS 를 내던 자리)."""
    residual, resolved = [], []
    for entry in sorted(journals.values(), key=lambda e: e["dir"]):
        got = entry.get("verdict")
        tail = (" — 판정 %s %s" % (got[1], got[2])) if got is not None else ""
        if entry["err"]:
            residual.append("%s: 교환 저널 상태 판독 불가(%s — '저널 없음' 이 아니다)%s" % (entry["dir"], entry["err"], tail))
        elif entry["names"]:
            more = (" 외 %d" % (len(entry["names"]) - 1)) if len(entry["names"]) > 1 else ""
            residual.append("%s: 미해결 교환 저널 %d건(%s%s — 중단된 신뢰 교환 · 사람이 .claude.json* 을 병합한 뒤 저널을 "
                            "지운다)%s" % (entry["dir"], len(entry["names"]), entry["names"][0], more, tail))
        elif got is not None:
            resolved.append("중단된 교환 저널 회수: %s (%s %s)" % (entry["dir"], got[1], got[2]))
    return residual, resolved


def _recover_interrupted_seed(config_dir, cfg, note=None):
    """잠금 아래 · 잔재 청소와 'already-trusted' **앞**에서 중단된 교환 트랜잭션을 판정한다 → None(계속) 또는
    (rc, verdict, reason). 판정은 저널 + **두 파일의 바이트**로 한다(codex D4):
      ⓐ 저널이 가리키는 displaced 부재 → 활성 문서가 유효하면 저널만 회수(교환 전 사망이거나 처분까지 끝난 뒤) ·
        활성이 없거나 깨졌으면 **거부**(무엇이 사라졌는지 모른다)
      ⓑ displaced 바이트 == payload → 우리 payload 뿐(교환 전 사망 · 되교환 뒤) = 낯선 데이터 아님 → 저널 회수
        (파일 자체는 지문 청소가 회수)
      ⓒ displaced 바이트 == 우리가 읽은 원본 → 교환은 성립했고 그것은 우리 계획의 전제였던 문서다 → 활성 문서가
        유효할 때만 폐기하고 저널 회수(활성이 깨졌으면 거부·보존)
      ★R7(리뷰 codex): ⓑⓒ 의 '활성이 유효하다' 는 **삭제의 충분조건이 아니다** — 그 사이 외부가 활성을 `{}` 로
        재생성하면 '건강' 이 통과한다. 활성 바이트가 저널의 두 지문 중 하나와 같아야(= 사본에만 있는 데이터가
        없어야) 지우고, 아니면 **자동 삭제되지 않는 이름으로 옮긴 뒤**(`_preserve_copy`) 계속한다(사유에 남긴다).
      ⓓ 그 밖(낯선 바이트) → **REFUSE interrupted-transaction** · 두 파일 무접촉 · 사람이 병합한다
      ⓔ 저널 자체가 판독 불가/형식 위반 → REFUSE(무접촉)
    자동 재시드(displaced 를 새 원본으로 삼아 다시 커밋)는 **하지 않는다** — 활성 쪽이 더 새로울 수도 있어 이름만으로
    선후를 알 수 없다(codex D4)."""
    R = SEED_TRUST_REFUSE
    preserved = []                             # 지울 수 없어 보존 이름으로 옮긴 사본(★R7) — 사유에 남긴다
    try:
        names = sorted(n for n in os.listdir(config_dir) if n.startswith(SEED_TRUST_INTENT_PREFIX))
    except OSError as e:
        # ★R6(리뷰 codex major): 종전엔 `return None`(계속)이라 열거가 막힌 dir 에서 미해결 저널이 **무시**됐고
        #   그 실행이 `already-trusted OK` 를 냈다. 이 자리는 config dir 생성·잠금 획득 **뒤**라 ENOENT 조차
        #   '정상적인 초기 부재' 가 아니라 '그 사이 사라졌다' 는 뜻이다(codex R6) — 전부 거부한다.
        return (R, "REFUSE", "journal-scan-failed(%s — 중단된 교환의 흔적을 관측할 수 없다 · 무접촉 · 사람이 확인)"
                % (errno.errorcode.get(e.errno, str(e.errno)) if e.errno else e))
    for n in names:
        jpath = os.path.join(config_dir, n)
        rec = _read_seed_intent(jpath)
        if rec is None:
            return (R, "REFUSE", "interrupted-transaction(저널 %s 판독 불가/형식 위반 — 중단된 교환의 흔적이다 · "
                                 "이 디렉터리의 .claude.json* 를 사람이 확인·병합한 뒤 저널을 지운다)" % n)
        dpath = os.path.join(config_dir, rec["displaced"])
        here = _lexists_strict(dpath)         # ★R6(codex): 조회 실패를 '부재' 로 접지 않는다 — 그 접힘은 '지워도 된다' 가 된다
        if here is None:
            return (R, "REFUSE", "interrupted-transaction(저널 %s 가 가리키는 %s 의 존재를 조회할 수 없다 — 무접촉 · "
                                 "사람이 확인)" % (n, rec["displaced"]))
        if here is False:
            if _active_document_healthy(cfg):
                _unlink_quiet(jpath)
                continue
            return (R, "REFUSE", "interrupted-transaction(저널 %s 가 가리키는 %s 가 없고 활성 .claude.json 도 유효하지 "
                                 "않다 — 사람이 확인)" % (n, rec["displaced"]))
        d = _digest_of_regular(dpath)
        if d is None:
            return (R, "REFUSE", "interrupted-transaction(%s 판독 불가 — 무접촉 · 사람이 확인)" % rec["displaced"])
        if d == rec["payload_sha256"]:
            # ★R5(codex 위임 반례 · 데이터 손실): 이 잔재는 '우리 payload' 지만 그 내용은 **원본 + 플래그** 다 —
            #   활성 문서가 그 사이 사라지거나 깨졌다면 이것이 **유일한 완전한 문서**다. 저널을 지우면 곧바로
            #   지문 청소(⑪)가 삭제해 원본 필드까지 잃는다 → 활성이 유효할 때만 회수 대상으로 넘긴다.
            if not _active_document_healthy(cfg):
                return (R, "REFUSE", "interrupted-transaction(활성 .claude.json 이 유효하지 않고 %s 가 유일한 완전한 "
                                     "문서일 수 있다 — 무접촉 · 사람이 확인)" % rec["displaced"])
            # ★R7(리뷰 codex): '건강' 은 '원본 데이터가 그 안에 있다' 가 아니다 — 중단 뒤 외부가 활성을 `{}` 로
            #   재생성하면 종전엔 저널을 지웠고 지문 청소가 유일한 사용자 사본을 삭제했다. 활성 바이트가 원본이나
            #   우리 payload 와 같을 때만 '지워도 잃는 것이 없다' 가 증명된다 · 아니면 보존 이름으로 옮긴다.
            if not _copy_is_redundant(cfg, (rec["captured_sha256"], rec["payload_sha256"])):
                kept = _preserve_copy(dpath)
                if kept is None:
                    return (R, "REFUSE", "interrupted-transaction(%s 를 보존 이름으로 옮기지 못했다 — 활성 문서가 "
                                         "원본이 아니어서 지울 수도 없다 · 무접촉 · 사람이 확인)" % rec["displaced"])
                preserved.append(kept)
            _unlink_quiet(jpath)                 # 우리 payload 뿐 — 파일은 지문 청소(⑪)가 회수한다
            continue
        if d == rec["captured_sha256"]:
            if not _active_document_healthy(cfg):
                return (R, "REFUSE", "interrupted-transaction(교환 뒤 활성 .claude.json 이 유효하지 않다 — %s 가 유일한 "
                                     "유효 사본일 수 있어 지우지 않는다 · 사람이 확인)" % rec["displaced"])
            if not _copy_is_redundant(cfg, (rec["captured_sha256"], rec["payload_sha256"])):
                # ★R7: 교환 뒤 제3자가 활성을 다시 썼다 — 옛 원본에만 있는 필드를 잃을 수 있으므로 지우지 않고 옮긴다.
                kept = _preserve_copy(dpath)
                if kept is None:
                    return (R, "REFUSE", "interrupted-transaction(%s 를 보존 이름으로 옮기지 못했다 — 교환 뒤 활성 문서가 "
                                         "다시 쓰였다 · 무접촉 · 사람이 확인)" % rec["displaced"])
                preserved.append(kept)
                _unlink_quiet(jpath)
                continue
            try:
                os.unlink(dpath)
            except OSError:
                # ★R6(codex): 데이터 판정은 끝났지만(활성 = 우리 문서 · displaced = 바이트 동일한 옛 원본) **정리는
                #   못 했다**. 저널을 남겨 다음 실행이 다시 시도하고 C58 이 그 잔존을 WARN 으로 드러낸다 —
                #   '회수 성공 = 저널 0' 을 단정하지 않는다. 여기서 REFUSE 로 올리지 않는 이유: 데이터는 안전하고
                #   FS 일시 오류로 부트 경로를 매번 막는 것이 더 나쁘다(치명위험 ④ 방향).
                continue
            _unlink_quiet(jpath)
            continue
        return (R, "REFUSE", "interrupted-transaction(교환 뒤 검증 전에 중단됐다 — 상대의 더 새 문서가 %s 에 있고 활성 "
                             ".claude.json 은 우리 문서다 · 어느 쪽이 최신인지 도구가 판정하지 않는다 · 사람이 병합한 뒤 "
                             "%s 를 지운다)" % (rec["displaced"], n))
    if preserved and note is not None:
        note.append("recovery-preserved(%s — 활성 문서가 원본이 아니어서 지우지 않고 옮겼다 · 사람이 병합)"
                    % ", ".join(preserved))
    return None


def _restore_foreign(tmp, cfg, payload_b, note):
    """교환으로 드러난 '낯선 inode'(교환 순간 .claude.json 에 있던 것이 우리가 읽은 원본이 아니다) 복원 → (rc, verdict, reason).
    되교환으로 상대 inode 를 그대로 되돌린다(무손실). 되교환 뒤 tmp(우리 inode)가 우리 payload 그대로면 폐기 · 아니면(그 마이크로초
    창에 제3의 쓰기가 우리 inode 에 앉았다) .claude.json.conflict-<utc>-<pid> 로 보존해 사유에 적는다. 되교환 자체가 실패하면 상대
    inode 는 displaced 이름(tmp 인자)에 그대로 있다 — **지우지 않는다** · ERROR. 어느 경로에도 상대 데이터 파괴 0.
    잔여(고지 · codex R2): 되교환 창(마이크로초)에 우리 inode 를 **쓰기용 fd 로 잡고 있던** 기록자의 in-place 쓰기는 대조 뒤·unlink 앞
    에 앉으면 잃는다 — 알려진 기록자(claude 는 프로브가 배제 · C43 _enable_mcp_server · Rust write_atomic_mode)는 전부 tmp+rename
    이라 이 형상이 아니다."""
    R, E = SEED_TRUST_REFUSE, SEED_TRUST_ERROR
    try:
        back = _exchange_paths(tmp, cfg)
    except OSError as e:
        back = e
    if back is not True:
        # 상대 inode 는 displaced 이름(tmp)에 그대로 있다 — 지우지 않는다(이름 그대로 보존 · 수동 병합)
        return E, "ERROR", ("되교환 실패(%s) — .claude.json 은 우리 문서, 상대 내용은 %s 에 보존(수동 병합)" % (back, tmp))
    try:
        conflict = _exclusive_name(os.path.join(os.path.dirname(cfg), SEED_TRUST_CONFLICT_PREFIX))
    except OSError:
        conflict = None                      # ★R7: 보존 이름을 못 얻었다 — 옮기지 않고 그 자리(displaced 이름)에 둔다
    try:
        _e, mine, _s = _read_claude_json_bytes(tmp)
        intact = _e and mine == payload_b
    except (ValueError, OSError):
        intact = False
    if intact:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        return R, "REFUSE", ("concurrent-change(교체 순간 다른 기록자 감지 — 상대 inode 무손실 복원 · 재시도 가능)%s"
                             % ((" · " + " · ".join(note)) if note else ""))
    if conflict is None:
        conflict = tmp
    else:
        try:
            os.replace(tmp, conflict)
        except OSError:
            conflict = tmp
    return R, "REFUSE", ("concurrent-change(교체 순간 다른 기록자 감지 — 상대 inode 복원 · 되교환 창의 제3 쓰기는 %s 에 보존 · "
                         "수동 병합)" % conflict)


def seed_trust(config_dir, cwd, force_unverified=False, proc_counter=None, backup=False,
               lock_fn=None, _pre_write_hook=None):
    """(config_dir, cwd) 한 쌍 신뢰 주입 → (rc, verdict, reason). rc: 0 OK(seeded|already-trusted) · 2 REFUSE
    (live-claude|unverified|lock-busy|lock-unavailable|concurrent-change|exchange-unavailable|link-failed) · 1 ERROR
    (usage|구조|IO|되읽기). 절차는 파일 머리 주석 ①~⑫. backup=True 면 기존 파일을 .bak-preflight 로 1회 보존(캡처 바이트
    배타 생성 · C58 --fix 경로). proc_counter/lock_fn/_pre_write_hook 은 테스트 주입점(기본 = 실물). ★R2: 프로브는 기존
    문서가 있을 때만 · 커밋은 교환/link 무손실 CAS. ★R3: 교환 기구 부재는 **무조건** REFUSE(강행 플래그도 못 연다 ·
    os.replace 0) · O_EXCL 폴백 제거."""
    proc_counter = proc_counter or claude_procs_for_config
    lock_fn = lock_fn or _try_lock_nb
    E, R, OK_ = SEED_TRUST_ERROR, SEED_TRUST_REFUSE, SEED_TRUST_OK
    if not config_dir or not cwd:
        return E, "ERROR", "usage(--config/--cwd 비어 있음)"
    if not os.path.isabs(config_dir) or not os.path.isabs(cwd):
        return E, "ERROR", "절대경로만 허용(config=%s cwd=%s)" % (config_dir, cwd)
    if ".pristine" in _path_identity(config_dir).replace("\\", "/").split("/"):
        return E, "ERROR", ".pristine 은 무접촉(바이너리 임베드 재생성 영역)"
    if os.path.lexists(config_dir) and (_is_link_like(config_dir) or not os.path.isdir(config_dir)):
        return E, "ERROR", "config dir 이 실제 디렉터리가 아니다(심링크/파일): %s" % config_dir
    if not os.path.isdir(cwd):
        # ★R2(리뷰): 부재 cwd 에 키를 박지 않는다(C58 헤더 'stale 경로 무변경' 과 같은 규칙 · claude 는 그 폴더에서 뜰 수 없다).
        return E, "ERROR", "cwd 가 존재하는 디렉터리가 아니다(stale 경로 무변경): %s" % cwd
    cfg = os.path.join(config_dir, ".claude.json")
    key = claude_project_key(cwd)
    lock_path = os.path.join(config_dir, SEED_TRUST_LOCK_NAME)
    lf = None

    def _acquire():
        """잠금 파일 열기+비차단 잠금 → None(획득) 또는 (rc, verdict, reason)."""
        nonlocal lf
        try:
            lf = os.fdopen(_open_nofollow(lock_path, os.O_RDWR | os.O_CREAT), "r+")
        except OSError as e:
            return E, "ERROR", "잠금 파일 열기 실패: %s" % e
        got = lock_fn(lf)
        if got is False:
            return R, "REFUSE", "lock-busy(%s — 다른 시드/수리 진행 중)" % lock_path
        if got is None:
            return R, "REFUSE", "lock-unavailable(잠금 기구 미가용 — 무잠금 쓰기 금지)"
        return None

    def _read_plan():
        """읽기+파싱+계획 → (existed, raw, st, new, changed) 또는 (rc, verdict, reason) 오류 튜플(길이 3)."""
        try:
            existed, raw, st = _read_claude_json_bytes(cfg)
        except ValueError as e:
            return E, "ERROR", str(e)
        except OSError as e:
            return E, "ERROR", "읽기 실패 — 거부: %s" % e
        try:
            data = _parse_claude_json(existed, raw)
        except (ValueError, UnicodeDecodeError) as e:
            return E, "ERROR", "파싱 실패 — 거부(손상 .claude.json 은 손대지 않는다): %s" % e
        try:
            new, changed, _k = trust_plan(data, key)
        except ValueError as e:
            return E, "ERROR", "구조 거부: %s" % e
        return existed, raw, st, new, changed

    try:
        if not os.path.isdir(config_dir):
            # 부재 dir = 부재 문서 = 지킬 기존 문서 없음(⑩) — 프로브 없이 만든다(cys-dept 는 어차피 mkdir -p 뒤에 부른다).
            try:
                os.makedirs(config_dir, exist_ok=True)
            except OSError as e:
                return E, "ERROR", "config dir 생성 실패: %s" % e
            if _is_link_like(config_dir) or not os.path.isdir(config_dir):
                return E, "ERROR", "config dir 이 실제 디렉터리가 아니다(심링크/파일): %s" % config_dir
        # ② 잠금 → 읽기 → 계획 — 이미 신뢰면 프로세스 프로브 없이 종료(R1)
        err = _acquire()
        if err:
            return err
        note = []
        broken = _recover_interrupted_seed(config_dir, cfg, note)   # ⑬ ★R5: 중단된 교환 — 청소·판정보다 **먼저**
        if broken:
            return broken
        orphan = _orphan_recovery_guard(config_dir, cfg)            # ⑭ ★R7: 저널 없는 보존 사본 + 불건강 활성 = 대체 문서 금지
        if orphan:
            return orphan
        swept = _sweep_stale_seed_tmp(config_dir)                                   # ⑪ 잠금 아래 잔재 청소
        state = _read_plan()
        if len(state) == 3:
            return state
        existed, raw, orig_st, new, changed = state
        if swept:
            note.append("stale-tmp swept %d" % swept)
        if not changed:
            return OK_, "OK", "already-trusted(key=%s)%s" % (key, (" · " + " · ".join(note)) if note else "")
        if existed:
            # ③⑩ 라이브 claude(그 CLAUDE_CONFIG_DIR) 0 확인 — **기존 문서가 있을 때만**(프로브가 지키는 것이 그 문서다)
            count, pdetail = proc_counter(config_dir)
            if count is None:
                if not force_unverified:
                    return R, "REFUSE", "unverified(%s) — --force-unverified 없이는 거부" % pdetail
                note.append("force-unverified(%s)" % pdetail)
            elif count > 0:
                return R, "REFUSE", "live-claude(n=%d · %s)" % (count, pdetail)
            else:
                note.append("probe=%s" % pdetail)          # 무엇이 0 을 검증했는지 사유에 남긴다(감사 · 스텁/실물 구분)
        else:
            note.append("no-probe(.claude.json 부재 — 보호할 기존 문서 없음)")
        # ⑦ 같은 dir mkstemp + fsync + 교체 前 임시파일 되읽기 → ⑥ 교체 직전 존재+바이트 대조 → 백업 → 무손실 커밋
        tmp = None
        intent = None
        try:
            # ★R3(리뷰): 직렬화·인코딩도 try 안 — 기존 문서의 고아 서로게이트 이스케이프(`"\ud800"`)는 json.loads 는 받지만
            #   ensure_ascii=False 출력의 utf-8 인코딩이 UnicodeEncodeError 를 낸다(종전: try 밖 → C58 --fix 경유 시 preflight
            #   전체 중단). ASCII 이스케이프로 재직렬화하면 원문과 같은 `\ud800` 이스케이프가 그대로 남는다(JS 는 읽는다).
            try:
                payload = json.dumps(new, ensure_ascii=False, indent=2)
                payload_b = payload.encode("utf-8")
            except UnicodeEncodeError:
                payload = json.dumps(new, ensure_ascii=True, indent=2)
                payload_b = payload.encode("utf-8")
                note.append("ascii-escaped(lone surrogate · JSON 값 보존)")
            fd, tmp = tempfile.mkstemp(prefix=SEED_TRUST_TMP_PREFIX, dir=config_dir)
            with os.fdopen(fd, "wb") as f:
                f.write(payload_b)
                f.flush()
                os.fsync(f.fileno())
            if orig_st is not None:
                try:
                    os.chmod(tmp, stat.S_IMODE(orig_st.st_mode))   # 캡처한 권한(mutable 원본 재판독 금지 · 0B 파일도 보존)
                except OSError as e:
                    return E, "ERROR", "권한 보존 실패 — 커밋 안 함: %s" % e     # 권한을 잃은 문서를 공개하지 않는다(codex R2)
            with open(tmp, "rb") as f:
                if json.loads(f.read().decode("utf-8")) != new:
                    return E, "ERROR", "임시파일 되읽기 불일치 — 커밋 안 함"
            if _pre_write_hook is not None:
                _pre_write_hook()
            # ⑥ 교체 직전 대조 — 존재+바이트(부재↔0B 구분 · codex R1). 심링크/정션 스왑도 여기서 거부. (값싼 선검사 — 진짜
            #   무손실 보장은 아래 교환/link 가 한다)
            try:
                cur_existed, cur, _cst = _read_claude_json_bytes(cfg)
            except ValueError as e:
                return E, "ERROR", "%s(교체 직전 스왑 감지)" % e
            except OSError as e:
                return E, "ERROR", "재확인 실패 — 거부: %s" % e
            if cur_existed != existed or cur != raw:
                return R, "REFUSE", "concurrent-change(.claude.json 이 읽기 이후 변경됨 — clobber 방지 · 재시도 가능)"
            if backup and existed:
                bak = cfg + ".bak-preflight"
                try:
                    bfd = os.open(bak, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
                                  stat.S_IMODE(orig_st.st_mode) if orig_st is not None else 0o600)
                except FileExistsError:
                    bfd = None                      # 1회 보존 계약 — 이미 있으면 그대로
                except OSError as e:
                    return E, "ERROR", "백업 실패 — 거부: %s" % e
                if bfd is not None:
                    with os.fdopen(bfd, "wb") as bf:  # 캡처한 바이트로(mutable 원본 copy2 금지 · codex R1)
                        bf.write(raw)
            # ⑦ 무손실 커밋(R2)
            if existed:
                # 교환은 **displaced 이름** 아래에서만 한다(codex R2): 교환 뒤 옛 inode 가 앉는 이름이 mkstemp 잔재(.seed-<8자>)와
                #   다른 네임스페이스여야 ⑪ 청소가 낯선 데이터를 지울 수 없고, 교환 직후 죽어도 상대 문서는 그 이름으로 남는다.
                # ★R3: 이름에 payload 지문(sha256 16자)을 박는다 — 교환 **직전**에 죽으면 우리 payload 만 든 displaced 가 남는데,
                #   다음 시더의 ⑪ 청소가 바이트 지문으로 그것만 골라 치운다(낯선 inode 는 지문 불일치 = 무접촉).
                try:
                    displaced = _exclusive_name(os.path.join(config_dir, SEED_TRUST_DISPLACED_PREFIX
                                                             + _payload_digest(payload_b) + "-"))
                except OSError as e:                      # ★R7: 보존 이름을 확정 못 하면 아무것도 옮기지 않는다
                    return E, "ERROR", "보존 이름 확정 실패 — 교환하지 않는다(무변경): %s" % e
                # ⑬ ★R5(리뷰 codex major): 교환~검증 창을 **내구적으로** 표시한다. 저널을 못 쓰면 교환하지 않는다
                #   (표시 없는 창을 열지 않는다 = 고립된 사용자 문서 0).
                # ★R7(리뷰 codex major): 저널은 **rename 보다 먼저** 공개한다. 종전 순서(rename → 저널)에는
                #   '저널 없는 payload 사본' 이 남는 창이 있었고, 그 사본은 보호 근거를 잃은 채 남아 다음 실행이
                #   만든 새 문서가 '건강' 판정을 주는 순간 청소 대상이 됐다(사용자 필드 소실). 저널이 먼저면 그
                #   창에서 남는 것은 '가리키는 파일이 없는 저널' 이고 회수 ⓐ 가 그것을 안전하게 접는다.
                try:
                    intent = _write_seed_intent(config_dir, os.path.basename(displaced),
                                                _payload_digest(raw), _payload_digest(payload_b))
                except OSError as e:
                    return E, "ERROR", "의도 저널 기록 실패 — 교환하지 않는다(무변경): %s" % e
                try:
                    os.rename(tmp, displaced)             # 같은 dir · 우리 payload 만 이동(아직 공개 전)
                except OSError as e:
                    _unlink_quiet(intent)                 # 창이 열리지 않았다(무변경) — 표시 회수
                    return E, "ERROR", "쓰기 실패(보존 이름 준비): %s" % e
                tmp = None
                # ★displaced 는 finally 가 절대 치우지 않는다(교환 직후 인터럽트가 와도 옛 inode = 낯선 데이터일 수 있다 · codex R2).
                #   치우는 곳은 '교환이 안 됐다' 가 확정된 자리(syscall 예외 · 기구 부재 · 폴백 실패)와 '옛 inode == 원본' 이 확정된 자리뿐.
                try:
                    swapped = _exchange_paths(displaced, cfg)
                except OSError as e:                      # syscall -1 = 교환 안 됨 → displaced 는 우리 payload 그대로
                    # ★R7(리뷰 codex): '교환 안 됨 = 사본을 지워도 안전' 은 성립하지 않는다 — 그 사이 외부가 활성을
                    #   지웠다면(교환이 ENOENT 로 실패하는 바로 그 형상) 이 사본이 유일한 완전한 문서다.
                    kept = _release_uncommitted_copy(cfg, displaced, intent)
                    return E, "ERROR", "쓰기 실패(교환): %s%s" % (e, kept)
                if swapped:
                    # displaced 에는 교환 순간 .claude.json 에 있던 inode 가 그대로 있다 — 그것이 우리가 읽은 원본인가?
                    try:
                        p_existed, prev, _pst = _read_claude_json_bytes(displaced)
                        foreign = (not p_existed) or prev != raw
                    except (ValueError, OSError):
                        foreign = True                    # 심링크/정션/비정규 파일이 끼어들었거나 판독 불가 = 낯선 것 → 복원
                    if foreign:
                        got = _restore_foreign(displaced, cfg, payload_b, note)    # displaced 처분은 그 안에서 확정
                        if got[0] != E:
                            _unlink_quiet(intent)         # 되교환까지 끝나 창이 닫혔다(실패면 저널을 남겨 증거로 둔다)
                        return got
                    note.append("commit=exchange")            # ★R4: 커밋은 교환으로 이미 성립 — 아래 청소 실패가 이 사실을 못 뒤집는다
                    try:
                        os.unlink(displaced)                  # 옛 원본(= 우리 계획의 전제 · 바이트 동일) 폐기
                        _unlink_quiet(intent)                 # 처분 완료 뒤에만 표시를 지운다(순서 고정 · codex D4)
                    except OSError as e:                      # 잔재는 남지만 커밋은 성공(리뷰 minor: 종전엔 ERROR 로 보고했다)
                        note.append("displaced-left(%s · 저널이 남아 다음 실행이 회수한다)"
                                    % (errno.errorcode.get(e.errno, str(e.errno)) if e.errno else e))
                    _fsync_dir(config_dir)
                else:
                    # ★R3(리뷰 codex BLOCK): 기구 부재(Windows · 미지원 FS)는 **REFUSE** — 대조~os.replace 창의 기록자(C43
                    #   _enable_mcp_server · Rust 시더 · 이 잠금을 모르는 모든 도구)는 덮이고 되읽기는 OK 를 돌려주므로 '고지' 로는
                    #   무손실 계약을 만족하지 못한다. 강행 플래그로도 열지 않는다(귀속 불확실을 감수하는 것과 알려진 손실을
                    #   감수하는 것은 다른 예외 · codex R3) — 사람이 관문에서 1회 신뢰하면 claude 가 스스로 플래그를 쓴다(2차 방어).
                    # ★R7: 같은 규율 — 활성 문서가 **유효할 때만** 우리 사본을 지운다(아니면 보존 이름으로 옮긴다).
                    kept = _release_uncommitted_copy(cfg, displaced, intent)
                    return R, "REFUSE", ("exchange-unavailable(%s — 원자 교환 없이는 대조~교체 창을 닫을 수 없다 · 기존 문서 무접촉 · "
                                         "관문에서 1회 수동 신뢰 뒤 already-trusted)%s%s"
                                         % (getattr(swapped, "why", None) or "기구 부재",
                                            (" · " + " · ".join(note)) if note else "", kept))
            else:
                try:
                    os.link(tmp, cfg)                     # 원자 create-if-absent(완성된 내용을 한 번에 공개)
                except FileExistsError:
                    return R, "REFUSE", "concurrent-change(부재였던 .claude.json 이 교체 순간 생겨났다 — 상대 내용 보존 · 재시도 가능)"
                except OSError as e:
                    # ★R3(리뷰 codex major): 하드링크 미지원 FS 의 O_EXCL 폴백은 **이름이 내용보다 먼저 공개**돼 ENOSPC/중단이
                    #   잘린 .claude.json 을 영구히 남겼다(이후 모든 시드가 '손상' 으로 거부) → 폴백 없이 REFUSE. errno 는 사유에만
                    #   (EPERM/EXDEV/ENOTSUP = 미지원 FS · ENOSPC/EIO/EACCES = 실패 — 어느 쪽이든 공개 0).
                    return R, "REFUSE", ("link-failed(%s — 완성된 내용의 원자 공개(hard link) 실패 · .claude.json 생성 0 · 하드링크 없는 "
                                         "FS 면 관문에서 1회 수동 신뢰)%s" % (errno.errorcode.get(e.errno, str(e.errno)) if e.errno else e,
                                                                          (" · " + " · ".join(note)) if note else ""))
                note.append("commit=link")                    # ★R4: 공개(link)가 성립한 뒤라 아래 청소 실패는 ERROR 가 아니다
                try:
                    os.unlink(tmp)
                except OSError as e:
                    note.append("tmp-left(%s)" % (errno.errorcode.get(e.errno, str(e.errno)) if e.errno else e))
                tmp = None
                _fsync_dir(config_dir)
        except (OSError, ValueError, UnicodeDecodeError) as e:
            return E, "ERROR", "쓰기 실패: %s" % e
        finally:
            if tmp:                                       # mkstemp 잔재만(displaced 는 여기서 손대지 않는다)
                _unlink_quiet(tmp)
        # ⑧ 되읽기 — 형 검사 전수 · **롤백 0**(R1): 다른 기록자가 그 사이 썼다면 그 내용을 보존한다.
        try:
            b_existed, braw, _bst = _read_claude_json_bytes(cfg)
            back = _parse_claude_json(b_existed, braw)
        except (OSError, ValueError, UnicodeDecodeError) as e:
            return E, "ERROR", ("되읽기 실패: %s — 파일은 커밋 상태로 둔다(임시파일 검증 통과분 · 롤백은 동시 기록자 내용을 "
                                "파괴할 수 있어 하지 않는다 · 수동 확인)%s" % (e, (" · " + " · ".join(note)) if note else ""))
        tail = (" · " + " · ".join(note)) if note else ""
        # 판정은 **엄격 True**(_trusted_exact · `is True`) 가 먼저다 — dict 동등 비교는 1/1.0 == True 라 다른 기록자가 쓴 숫자
        #   플래그를 통과시킨다(codex R1 반례 07).
        if not _trusted_exact(back, key):
            return R, "REFUSE", ("concurrent-change(post-commit — 교체 후 다른 기록자가 다시 썼고 플래그가 없다 · 상대 내용 보존 · "
                                 "재시도 가능)%s" % tail)
        if back == new:
            return OK_, "OK", "seeded(key=%s)%s" % (key, tail)
        return OK_, "OK", "seeded(key=%s · 교체 후 다른 기록자가 갱신했으나 플래그 보존)%s" % (key, tail)
    finally:
        if lf is not None:
            try:
                lf.close()
            except OSError:
                pass


_SEED_TRUST_TIMEOUT_ENV = "CYS_SEED_TRUST_TIMEOUT"
_SEED_TRUST_TIMEOUT_DEFAULT = 20.0
_SEED_TRUST_EXIT_GRACE = 2.0      # 마감 감시 발화 뒤 '진단 출력' 에 허용하는 시간 — 지나면 무조건 종료(codex D5)


def _seed_trust_timeout_secs(env=None):
    """마감 감시 상한(초) — 미설정 = 기본 20 · 0 이하 = 끔(None) · 비수치 = 기본(파싱 실패로 감시를 잃지 않는다).
    롤백 노브지 게이트 노브가 아니다(끄면 종전 동작 = 무한 대기).
    ★R5(리뷰 codex minor): **비유한값**(`nan`·`inf`·`1e309`)은 계약("0 이하면 끔") 밖인데 종전엔 nan → None(감시
    상실) · inf → `threading.Timer(inf)`(사실상 감시 상실 · 내부 오버플로로 타이머 스레드가 죽을 수도) 였다 →
    `math.isfinite` 아니면 **기본값**으로 되돌린다(감시를 잃지 않는 방향)."""
    raw = (os.environ if env is None else env).get(_SEED_TRUST_TIMEOUT_ENV, "")
    if not raw:
        return _SEED_TRUST_TIMEOUT_DEFAULT
    try:
        v = float(raw)
    except (TypeError, ValueError):
        return _SEED_TRUST_TIMEOUT_DEFAULT
    if not math.isfinite(v):
        return _SEED_TRUST_TIMEOUT_DEFAULT
    if v <= 0:
        return None
    # ★R5(리뷰 codex D5): 유한이어도 `threading.TIMEOUT_MAX` 를 넘으면 타이머 스레드가 OverflowError 로 죽어 감시가
    #   조용히 사라진다(`1e300`) → 상한으로 접는다(그 값은 사실상 '끄기' 지만 예외 없이 그렇게 된다).
    return min(v, threading.TIMEOUT_MAX)


def _start_seed_deadline(secs, emit):
    """★R4(codex 잔여 지적): 부트 호출자(cys-dept launch/allocate/create · rotate 는 **데몬을 내린 뒤** 여기 온다)가
    시드 I/O 에 무한정 잡히지 않게 하는 마감 감시 → 취소 함수. 데몬 스레드 타이머(Windows 안전 · SIGALRM 미사용) ·
    초과하면 emit() 로 REFUSE 1줄을 찍고 `os._exit(2)`(블로킹 syscall 안에서는 그것만 통한다).
    안전성: 종료 시점의 상태 집합은 SIGKILL 과 같다 — 공개(link/교환) 전이면 `.claude.json` 미생성/무변경이고,
    뒤면 **교환은 성립했으나 검증·청소가 끝나지 않았을 수 있다**(★R5 정정: 종전 주석의 "뒤면 이미 커밋된 상태다" 는
    거짓이었다 — 교환~검증 창에서 죽으면 상대의 더 새 문서가 displaced 에 남는다). 그 창은 이제 **의도 저널**
    (`.claude.json.seed-intent-*`)이 표시하고 다음 실행이 회수하거나 REFUSE 한다(seed_trust ⑬) · 잠금은 프로세스
    종료로 OS 가 푼다 · 호출자는 fail-open(WARN 1줄 + 계속)이라 좌석은 죽지 않는다(2차 방어 = 관문 보류).
    ★R5(리뷰 minor · 취소 경쟁): `seed_trust` 가 **반환한 뒤** `cancel()` 전에 타이머가 `_fire` 에 들어가면
    `Timer.cancel()` 은 무효라 커밋된 실행이 `REFUSE timeout`(rc 2)으로 뒤집혔다 — 이제 `_fire` 와 취소는 같은
    잠금 아래 `done` 플래그를 보고, 먼저 잡은 쪽만 이긴다(취소가 이기면 `_fire` 는 조용히 반환)."""
    if not secs:
        return lambda: None
    state = {"done": False}
    guard = threading.Lock()

    def _claim():
        """취소/발화 중 **먼저 온 하나**만 True — 반환 뒤 취소 사이의 창(리뷰 R5)을 닫는다."""
        with guard:
            if state["done"]:
                return False
            state["done"] = True
            return True

    def _fire():
        if not _claim():
            return                      # 정상 경로가 이미 끝났다 — 커밋된 실행을 timeout 으로 뒤집지 않는다
        # ★R5(리뷰 codex D5): `emit()` 이 막힌 stdout 에 걸리면 `finally` 는 실행되지 않는다(예외가 아니라 '돌아오지
        #   않음') → 종료 자체를 별도 타이머로 보증한다(진단 1줄은 최선 노력 · 종료는 보장).
        hard = threading.Timer(_SEED_TRUST_EXIT_GRACE, lambda: os._exit(SEED_TRUST_REFUSE))
        hard.daemon = True
        try:
            hard.start()
        except RuntimeError:
            os._exit(SEED_TRUST_REFUSE)   # ★R5(codex 위임 반례): 종료 보증이 없으면 진단을 포기한다 — emit 이 막히면
            #                               부트 호출자가 영원히 붙잡힌다(이 층의 계약은 '반드시 끝난다' 다)
        try:
            emit()
        finally:
            os._exit(SEED_TRUST_REFUSE)
    t = threading.Timer(secs, _fire)
    t.daemon = True
    try:
        t.start()
    except RuntimeError:
        return None                     # 감시 스레드를 못 띄웠다 — 호출자는 시드를 시작하지 않는다(codex D5)

    def cancel():
        _claim()                        # 발화가 이미 시작됐으면 False — 그쪽이 os._exit 한다(취소 불가가 정상)
        t.cancel()
    return cancel


def _seed_trust_emit(json_mode, rc, verdict, reason, config, cwd):
    """stdout 1줄(사람/JSON 공통 형식) — 마감 감시와 정상 경로가 **같은 문면**을 쓴다."""
    if json_mode:
        line = json.dumps({"verdict": verdict, "reason": reason, "config": config, "cwd": cwd, "rc": rc},
                          ensure_ascii=False)
    else:
        line = "seed-trust: %s %s config=%s cwd=%s" % (verdict, reason, config, cwd)
    try:
        print(line)
    except UnicodeEncodeError:   # 경로/사유 속 고아 서로게이트(surrogateescape) — 진단 출력이 또 예외를 내면 안 된다(R3 codex)
        print(line.encode("ascii", "backslashreplace").decode("ascii"))
    try:
        sys.stdout.flush()       # 마감 감시는 이 뒤에 os._exit 한다(버퍼 유실 금지)
    except (OSError, ValueError):
        pass


def _seed_trust_main(argv):
    """`--seed-trust --config DIR --cwd DIR [--force-unverified] [--json]` — stdout 1줄
    `seed-trust: <OK|REFUSE|ERROR> <reason> config=… cwd=…` · rc 0/2/1. argparse 앞 가로채기(--self-test 관례).
    ★R4: `CYS_SEED_TRUST_TIMEOUT`(기본 20초) 마감 감시 — 부트 경로가 응답 없는 FS 에 잡히면 REFUSE timeout(rc 2)."""
    ap = argparse.ArgumentParser(prog="javis_preflight.py --seed-trust",
                                 description="폴더 신뢰 사전 주입 — (config_dir, cwd) 한 쌍만")
    ap.add_argument("--seed-trust", action="store_true", help=argparse.SUPPRESS)
    ap.add_argument("--config", required=True, metavar="DIR", help="CLAUDE_CONFIG_DIR(계정 dir · 절대경로)")
    ap.add_argument("--cwd", required=True, metavar="DIR", help="신뢰할 워크스페이스(좌석 cwd · 절대경로)")
    ap.add_argument("--force-unverified", action="store_true",
                    help="라이브 claude 귀속 불가 플랫폼에서도 프로브 단계만 넘긴다(잠금·재읽기 대조·되읽기·양성 관측 거부·"
                         "교환 기구 부재 거부는 그대로)")
    ap.add_argument("--json", action="store_true", help="JSON 1줄 출력")
    try:
        args = ap.parse_args(argv)
    except SystemExit as e:
        return 0 if e.code == 0 else SEED_TRUST_ERROR
    secs = _seed_trust_timeout_secs()
    cancel = _start_seed_deadline(secs, lambda: _seed_trust_emit(
        args.json, SEED_TRUST_REFUSE, "REFUSE",
        # ★R5(리뷰 codex D5): 종전 문면의 '공개 전이면 무쓰기' 는 **거짓일 수 있다** — 교환 뒤 검증 전에 끊기면 커밋은
        #   성립했고 상대 문서가 displaced 에 남는다. 결과를 단정하지 않고, 그 판정은 다음 실행의 의도 저널이 한다.
        "timeout(%gs 안에 끝나지 않았다 — 응답 없는 파일시스템/판독 대기 · 부트 호출자를 붙잡지 않으려 중단 · **커밋 여부는 "
        "이 줄로 단정하지 않는다**: 중단된 교환은 다음 실행이 의도 저널로 판정한다 · 상한은 %s)"
        % (secs, _SEED_TRUST_TIMEOUT_ENV), args.config, args.cwd))
    if cancel is None:
        _seed_trust_emit(args.json, SEED_TRUST_REFUSE, "REFUSE",
                         "watchdog-unavailable(마감 감시 스레드를 띄우지 못했다 — 시간 상한 없이 부트 경로를 붙잡지 "
                         "않는다 · 무쓰기 · %s=0 으로 감시를 끄면 종전 동작)" % _SEED_TRUST_TIMEOUT_ENV,
                         args.config, args.cwd)
        return SEED_TRUST_REFUSE
    try:
        rc, verdict, reason = seed_trust(args.config, args.cwd, force_unverified=args.force_unverified)
    finally:
        cancel()
    _seed_trust_emit(args.json, rc, verdict, reason, args.config, args.cwd)
    return rc


# ── cysjavis 레지스트리 판독(C58 스코프 · 읽기 전용) ──
def _read_json_tolerant(path):
    """JSON 파일 관용 판독 — 부재·IO·파싱 실패·**비정규 파일**은 None(BOM 허용). ★R4(리뷰 codex major): 종전 무가드
    `open` 은 writer 없는 FIFO 에서 영구 블록했다(C58 이 레지스트리·config 를 읽다 preflight 전체가 멈춘다) → 공용
    `_open_unblocking_ro`. 심링크는 **계속 따라간다**(레지스트리를 심링크로 두는 설치를 깨지 않는다 · 대상이 정규
    파일이 아니면 그때 None)."""
    try:
        fd, _st = _open_unblocking_ro(path)
        with os.fdopen(fd, "rb") as f:
            raw = f.read()
    except (OSError, ValueError):
        return None
    try:
        return json.loads(raw.decode("utf-8-sig"))
    except (ValueError, UnicodeDecodeError):
        return None


_TOPOLOGY_CLAUDE_AGENT = "claude"
_WIN_PIPE_PREFIX = "\\\\.\\pipe\\"


def _abs_str(v):
    """레지스트리 경로 값 검증 — 비어 있지 않은 **절대경로** 문자열만(상대경로는 preflight 호출자의 cwd 를 빌려 귀속되는
    추정이 된다 · codex R1)."""
    return isinstance(v, str) and bool(v) and os.path.isabs(v)


def _is_root_cwd(cwd):
    """cys.rs sanitize_launch_cwd 의 루트 판정 미러(순수): 꼬리 `/`·`\\` 를 전부 벗긴 뒤 비어 있거나 `X:`(드라이브 루트)면 루트."""
    if not isinstance(cwd, str):
        return False
    t = cwd.rstrip("/\\")
    return t == "" or (len(t) == 2 and t[1] == ":" and t[0].isalpha())


def _resolve_catalog_cwd(cwd, home=None):
    """★R3(리뷰 codex major): depts.json 의 **카탈로그 원값** cwd(create 가 기록 · 기동 입력)를 기동기 `cys-dept resolve_dept_cwd`
    와 같은 규칙으로 해석한다 — 루트("/"·"///"·"\\"·"C:\\") → home · 존재하지 않는 dir → home(기동기의 $HOME 폴백 미러) ·
    존재하면 물리 경로가 루트인 것(→ home) 만 걸러내고 원값 유지(소비자의 claude_project_key 가 realpath 를 낸다). 종전엔 카탈로그
    `cwd="/"` 가 좌석·시드는 $HOME 인데 C58 은 "/" 를 별개 갭으로 잡고 --fix 가 claude 가 결코 뜨지 않는 쌍을 시드했다.
    topology 의 cwd 는 **기록된 좌석 상태**라 이 해석을 받지 않는다(관측 쌍 보존 · codex R3). 상대경로·None 은 호출자
    (_abs_str)가 이미 제외. home 은 주입점(기본 expanduser · Windows 네이티브 python 과 Git Bash $HOME 의 표기 차는 Not-tested)."""
    home = home or os.path.expanduser("~")
    if not isinstance(cwd, str) or not cwd:
        return cwd
    if _is_root_cwd(cwd):
        return home
    if not os.path.isabs(cwd):
        return cwd
    if not os.path.isdir(cwd):
        return home
    try:
        if _is_root_cwd(os.path.realpath(cwd)):
            return home
    except (OSError, ValueError):
        pass
    return cwd


def _topology_pairs(topology_path, reg=None):
    """topology.json entries[] → [(claude_config_dir, cwd)] — 둘 다 절대경로 문자열인 항목만(config 부재 항목은 레인
    기본값으로 **추정 귀속하지 않는다** · codex 11). ★R1: `agent` 가 정확히 "claude" 인 항목만 — codex/gemini 좌석은 그
    config 로 claude 를 띄우지 않고(본부 reviewer-codex cwd 가 영구 미수리 WARN 을 만들던 것), null(미입양 빈 셸)의 장래
    에이전트는 **역할**이 정하므로(javis_formation ROLE_AGENT — reviewer-codex 빈 셸은 codex 가 된다) claude 의도의 기록이
    아니다 · 키 부재(레거시)도 미지 = 제외. reg 를 주면 존재하나 판독 불가한 파일을 reg["unreadable"] 에 남긴다."""
    t = _read_json_tolerant(topology_path)
    out = []
    if isinstance(t, dict) and "entries" not in t:
        entries = []                     # 키 부재 = 빈 로스터(판독은 됐다)
    else:
        entries = t.get("entries") if isinstance(t, dict) else None
    if not isinstance(entries, list):
        # ★R2(리뷰 codex): `{"entries": 1}` 같은 형상 이상은 TypeError 로 preflight 전체를 죽이는 게 아니라 판독불가 1건이다.
        #   명시 `null`·수·문자열·객체도 형상 이상(키 부재만 빈 로스터).
        if reg is not None and os.path.isfile(topology_path):
            reg.setdefault("unreadable", []).append(topology_path)
        return out
    for e in entries:
        if not isinstance(e, dict):
            continue
        if e.get("agent") != _TOPOLOGY_CLAUDE_AGENT:
            continue
        c, w = e.get("claude_config_dir"), e.get("cwd")
        if _abs_str(c) and _abs_str(w):
            out.append((c, w))
    return out


def _pipe_slug(socket_path):
    """src/lib.rs pipe_slug 미러 — 마지막 경로 컴포넌트(역슬래시·슬래시 양쪽)에서 영숫자·-·_ 만."""
    last = re.split(r"[\\/]", socket_path or "")[-1]
    return "".join(ch for ch in last if ch.isalnum() or ch in "-_")


def _win_state_root():
    """Windows cysd 영속 루트 = %LOCALAPPDATA%\\cys (src/bin/cysd/state.rs state_dir). LOCALAPPDATA 부재면 Rust 는 상대 "."
    를 쓴다(데몬 cwd 기준 — 판독 측에서 위치를 알 수 없다) → None(미해결 · 두 번째 위치를 발명하지 않는다 · codex R1)."""
    base = os.environ.get("LOCALAPPDATA")
    if not base:
        return None
    return os.path.join(base, "cys")


def _unix_state_root(platform=None):
    """lib.rs default_socket_path 의 dirs::state_dir 미러 — linux 만 $XDG_STATE_HOME(절대경로) 우선 · darwin 등은 dirs 가
    None 을 돌려 home 폴백 = ~/.local/state."""
    if (platform or sys.platform).startswith("linux"):
        xdg = os.environ.get("XDG_STATE_HOME")
        if xdg and os.path.isabs(xdg):
            return xdg
    return os.path.join(os.path.expanduser("~"), ".local", "state")


def _hub_state_dir(os_name=None, platform=None):
    """본부 데몬 state dir(topology.json 위치) 또는 None(미해결). Windows = %LOCALAPPDATA%\\cys(기본 파이프 `\\\\.\\pipe\\cys`
    슬러그 'cys' 는 루트 그대로 · state.rs) · unix = <state root>/cys. ★CYS_SOCKET 은 의도적으로 쓰지 않는다 — 부서 좌석의
    CYS_SOCKET 은 부서 소켓이라 본부 위치가 아니고, cys-dept 는 시드 호출 시 CYS_SOCKET 을 벗긴다."""
    if (os_name or os.name) == "nt":
        return _win_state_root()
    return os.path.join(_unix_state_root(platform), "cys")


def _dept_state_dir(name, sock, os_name=None, platform=None):
    """부서 데몬 state dir 또는 None(미해결). unix 소켓 경로 = dirname(socket) **그대로**(부모가 사라졌어도 무관한 폴백으로
    돌리지 않는다 · codex R1) · named pipe(`\\\\.\\pipe\\…`) 또는 Windows = %LOCALAPPDATA%\\cys\\<pipe_slug>(state.rs RC-13 ·
    슬러그 빈/`cys` 는 루트) · 소켓 미기록 = cys-dept dept_sock 규약 ~/.local/state/cys-dept-<name>(bash 는 XDG 를 모른다)."""
    sock = sock if isinstance(sock, str) else ""
    is_pipe = sock.startswith(_WIN_PIPE_PREFIX) or sock.startswith("//./pipe/")
    if is_pipe or (os_name or os.name) == "nt":
        root = _win_state_root()
        if root is None:
            return None
        slug = _pipe_slug(sock) if sock else "cys-dept-%s" % name
        return root if (not slug or slug == "cys") else os.path.join(root, slug)
    if sock:
        return os.path.dirname(sock) or None
    return os.path.join(os.path.expanduser("~"), ".local", "state", "cys-dept-%s" % name)


def _depts_json_path():
    """cys-dept 와 같은 env 이름(CYS_DEPTS_JSON) — 하네스 격리 주입점."""
    return os.environ.get("CYS_DEPTS_JSON") or os.path.join(os.path.expanduser("~"), ".cys", "depts.json")


def _scope_registry(reg, reason, narrow):
    """격리 컨텍스트에 따른 레지스트리 스코프(순수). reason None = 전체 · 부서 컨텍스트(narrow = [<acct>/settings.json]) =
    **그 계정 config 의 쌍만**(타 부서·본부 config 는 보이지 않는다 — 부서 preflight 가 남의 config 를 판정·수리하지 않게) ·
    계정 미상 부서/임시 팩 = 빈 레지스트리(sources 도 비운다 — '판독했으나 대상 없음' 이 아니라 '판정 밖'). ★R1: 종전엔 사유가
    있으면 무조건 빈 레지스트리 → 부서 컨텍스트 C58 이 자기 갭도 못 보고 PASS(감사 에러4 ③ 재현)."""
    if reason is None:
        return reg
    if not narrow:
        return {"pairs": {}, "configs": {}, "sources": [], "unreadable": [], "scope": "none"}
    keep = {_path_identity(os.path.dirname(t)) for t in narrow if isinstance(t, str) and t}
    return {"pairs": {k: v for k, v in reg["pairs"].items() if k in keep},
            "configs": {k: v for k, v in reg["configs"].items() if k in keep},
            "sources": list(reg["sources"]), "unreadable": list(reg.get("unreadable") or []), "scope": "account"}


def cysjavis_registry():
    """cysjavis 레지스트리(읽기 전용) → {"pairs": {config_identity: {cwd_identity: cwd}}, "configs": {config_identity:
    config_dir}, "sources": [판독한 파일…], "scope": full|account|none}. 출처 = 본부 topology.json(_hub_state_dir) +
    depts.json 각 부서 topology.json(_dept_state_dir) + depts.json account_dir(config 대상) + ★R1 depts.json (account_dir, cwd)
    쌍(create 가 기록한 등재 cwd — 추정 아님) + 본부 mission.json(0.14.30 스키마엔 cwd 가 없다 — claude_config_dir·cwd 문자열
    쌍이 있을 때만 채택 · 추정 귀속 0). 격리 컨텍스트는 _scope_registry 로 접는다(부서 = 자기 계정만 · 임시 팩 = 빈)."""
    reg = {"pairs": {}, "configs": {}, "sources": [], "unreadable": [], "scope": "full"}
    reason, narrow = _discover_isolation_block()
    if reason is not None and not narrow:
        return _scope_registry(reg, reason, narrow)

    def add_cfg(cfg):
        if _abs_str(cfg):
            reg["configs"].setdefault(_path_identity(cfg), cfg)

    def add_pair(cfg, cwd):
        if _abs_str(cfg) and _abs_str(cwd):
            add_cfg(cfg)
            reg["pairs"].setdefault(_path_identity(cfg), {}).setdefault(_path_identity(cwd), cwd)

    home = os.path.expanduser("~")
    hub_sd = _hub_state_dir()
    if hub_sd is None:
        reg["unreadable"].append("<hub state dir 미해결(Windows LOCALAPPDATA 부재)>")
    else:
        hub_topo = os.path.join(hub_sd, "topology.json")
        pairs = _topology_pairs(hub_topo, reg)
        if pairs:
            reg["sources"].append(hub_topo)
        for c, w in pairs:
            add_pair(c, w)
    mission_path = os.path.join(home, ".cys", "state", "mission.json")
    mission = _read_json_tolerant(mission_path)
    if (isinstance(mission, dict) and _abs_str(mission.get("cwd")) and _abs_str(mission.get("claude_config_dir"))):
        add_pair(mission["claude_config_dir"], mission["cwd"])
        reg["sources"].append(mission_path)
    depts_path = _depts_json_path()
    depts = _read_json_tolerant(depts_path)
    if isinstance(depts, dict):
        reg["sources"].append(depts_path)      # 판독한 출처(등재 0·depts 키 부재/형상 이상이어도 object 면 출처) — '판정 불가' 와
                                               #   '대상 없음' 을 구분(codex R1 반례 10)
    elif os.path.isfile(depts_path):
        reg["unreadable"].append(depts_path)   # 존재하는데 object 가 아니다(null·[]·손상)
    dmap = depts.get("depts") if isinstance(depts, dict) else None
    for name, meta in (dmap.items() if isinstance(dmap, dict) else ()):
        if not isinstance(meta, dict):
            continue
        acct = meta.get("account_dir")
        if _abs_str(acct):
            add_cfg(acct)
            if isinstance(meta.get("cwd"), str) and meta["cwd"]:
                # ★R1: create 가 기록한 등재 cwd — topology 부재/미기록에도 쌍(추정 아님) · ★R3: 카탈로그 원값은 기동기 규칙으로
                #   **먼저** 해석(루트 "C:\\" 는 posix 에선 isabs 가 아니라 _abs_str 이 먼저 버리면 못 잡는다 · codex R3) —
                #   루트/부재 → home · 상대값은 그대로 남아 _abs_str 이 제외 · depts.json 자체는 원값 그대로(등재 계약 불변)
                add_pair(acct, _resolve_catalog_cwd(meta["cwd"]))
        sd = _dept_state_dir(name, meta.get("socket"))
        if sd is None:
            reg["unreadable"].append("<dept %s state dir 미해결(Windows LOCALAPPDATA 부재)>" % name)
            continue
        topo = os.path.join(sd, "topology.json")
        pairs = _topology_pairs(topo, reg)
        if pairs:
            reg["sources"].append(topo)
        for c, w in pairs:
            add_pair(c, w)
    return _scope_registry(reg, reason, narrow)


class _Nowhere(object):
    """핀 문자열의 **부재 위치** — 어떤 대소 비교에도 False(정렬 불가). 결측은 값이 아니다."""
    __slots__ = ()

    def __lt__(self, other):
        return False

    def __le__(self, other):
        return False

    def __gt__(self, other):
        return False

    def __ge__(self, other):
        return False

    def __repr__(self):
        return "<핀 문자열 부재>"


_NOWHERE = _Nowhere()


class _PinSrc(str):
    """self-test 정적 핀의 소스 문자열 — `.index()/.rindex()` 가 부재에 **예외 대신** `_NOWHERE` 를 돌려준다.
    ★R7(리뷰 claude minor): 순서 핀은 `src.index(a) < src.index(b)` 꼴인데 핀 문자열이 사라지면(= 잡으라는 바로 그
    회귀) `ValueError: substring not found` 가 self-test **전체**를 죽여 `결과: PASS n / FAIL m` 줄조차 못 냈다
    (실측 C58 뮤테이션: 나머지 97 핀의 상태가 트레이스백 하나에 가려졌다). 이제 그 핀 **하나만** FAIL 이 된다.
    포함 검사(`in`)·`count()` 는 str 그대로다."""
    __slots__ = ()

    def index(self, sub, *a):
        i = str.find(self, sub, *a)
        return i if i >= 0 else _NOWHERE

    def rindex(self, sub, *a):
        i = str.rfind(self, sub, *a)
        return i if i >= 0 else _NOWHERE


def _pin_src(obj):
    """핀 대상의 소스 → `_PinSrc`. self-test 안의 모든 `inspect.getsource` 는 이 경유다(순서 핀 안전)."""
    import inspect
    return _PinSrc(inspect.getsource(obj))


def _st_raises(exc, fn, *a, **kw):
    """핀용 — 그 호출이 정확히 그 예외를 내는가(부작용 없는 순수 함수에만 쓴다). 다른 예외·무예외는 False."""
    try:
        fn(*a, **kw)
    except exc:
        return True
    except Exception:
        return False
    return False


def _self_test():
    """--self-test 가로채기(팩 bin 도구 관례 — _check_bin_tool 이 부르는 그 형태와 동형).

    범위는 **순수 판정 핀만**이다(네트워크·데몬·subprocess 0): C79 heartbeat 판정 함수 +
    WARN 강등·배선의 정적 계약. 전체 체크 배터리는 self-test 대상이 아니다 — 그것은
    preflight 실행 자체가 검증한다(부작용 있는 체크를 여기서 돌리면 '관찰이 상태를
    바꾸는' PHIL-04 위반이 된다)."""
    fails, total = [], [0]

    def check(name, cond):
        total[0] += 1
        print("  %s  %s" % ("PASS" if cond else "FAIL", name))
        if not cond:
            fails.append(name)

    print("== javis_preflight.py self-test ==")
    try:
        now = 1000000.0
        check("heartbeat 부재 → absent", heartbeat_verdict(None, now, 90.0) == ("absent", None))
        check("age 10s → fresh", heartbeat_verdict(now - 10, now, 90.0) == ("fresh", 10.0))
        check("경계 age==max_age → fresh(게이트6 부등호 ≤ 동일)",
              heartbeat_verdict(now - 90, now, 90.0)[0] == "fresh")
        check("age 200s → stale", heartbeat_verdict(now - 200, now, 90.0)[0] == "stale")
        check("미래 mtime(시계 스큐) → fresh(보수측)",
              heartbeat_verdict(now + 5, now, 90.0)[0] == "fresh")
        src = _pin_src(Preflight.c79_cycle_verifier_heartbeat)
        check("C79 는 FAIL 을 내지 않는다(WARN 강등 계약 — 부트 비치명)",
              "self.add(cid, FAIL" not in src)
        check("C79 --fix 는 bootstrap-verifier --ensure(멱등)로 수리",
              '"bootstrap-verifier", "--ensure"' in src)
        check("C79 report 모드는 읽기 전용(fix 분기 밖 subprocess 없음)",
              src.index("if self.fix:") < src.index("subprocess.run("))
        run_src = _pin_src(Preflight.run)
        check("C79 run() 배선(마지막 고정 슬롯 C62 앞)",
              "c79_cycle_verifier_heartbeat" in run_src
              and run_src.index("c79_cycle_verifier_heartbeat") < run_src.index("c62_pack_heal_ledger"))
        # ── WP-2(0.14.31) --seed-trust · C58 스코프 — 순수 판정·정적 계약 핀 ──
        # ★R1 재핀 고지: 아래 핀 중 9a4cda4(이 WP 1차 커밋)가 신설한 것만 리뷰 R1 반영으로 바뀌었다(기준선 v0.14.30 핀 무변경):
        #   별칭 키 재사용 → 정확 키 정책 · 프로브→잠금 순서 → 이미 신뢰 무프로브 · 롤백 → 롤백 0 · ps 판정 3튜플(unresolved).
        print("-- WP-2 seed-trust --")
        nd, ch, k = trust_plan({}, "/w/a")
        check("trust_plan 부재({}) → 최소 문서 {projects:{cwd:{hasTrustDialogAccepted:true}}}",
              ch and k == "/w/a" and nd == {"projects": {"/w/a": {"hasTrustDialogAccepted": True}}})
        part = {"hasCompletedOnboarding": False, "theme": "dark",
                "projects": {"/w/other": {"hasTrustDialogAccepted": False, "x": 1},
                             "/w/a": {"allowedTools": ["Bash"]}}}
        snap = json.loads(json.dumps(part))
        nd, ch, k = trust_plan(part, "/w/a")
        check("trust_plan 부분 → 그 항목 한 키만 추가 · 다른 키·다른 항목·온보딩 플래그 무접촉 · 입력 불변",
              ch and k == "/w/a" and part == snap
              and nd["hasCompletedOnboarding"] is False and nd["theme"] == "dark"
              and nd["projects"]["/w/other"] == {"hasTrustDialogAccepted": False, "x": 1}
              and nd["projects"]["/w/a"] == {"allowedTools": ["Bash"], "hasTrustDialogAccepted": True})
        nd, ch, k = trust_plan({"projects": {"/w/a": {"hasTrustDialogAccepted": True}}}, "/w/a")
        check("trust_plan 이미 true(정확 키) → changed=False(멱등 · 무쓰기)", not ch and k == "/w/a")
        nd, ch, k = trust_plan({"projects": {"/w/a/": {"hasTrustDialogAccepted": True}}}, "/w/a")
        check("trust_plan 별칭 키(꼬리 슬래시) true 는 '이미 신뢰' 가 아니다 — 정확 키 생성 · 별칭 무접촉(R1 · claude 는 정확 키만 읽는다)",
              ch and k == "/w/a" and nd["projects"]["/w/a/"] == {"hasTrustDialogAccepted": True}
              and nd["projects"]["/w/a"] == {"hasTrustDialogAccepted": True})
        bad = 0
        for data in ([], {"projects": []}, {"projects": None}, None, {"projects": {"/w/a": None}}, {"projects": {"/w/a": 1}}):
            try:
                trust_plan(data, "/w/a")
            except ValueError:
                bad += 1
        check("trust_plan 비-object 최상위/None · 존재하는 비-object projects(명시 null 포함) · 비-object 항목(명시 null 포함) → ValueError(무쓰기) 6/6",
              bad == 6)
        plan_src = _pin_src(trust_plan)
        check("trust_plan 은 hasCompletedOnboarding 리터럴을 모른다(테마 관문 무접촉 계약)",
              '"hasCompletedOnboarding"' not in plan_src and "'hasCompletedOnboarding'" not in plan_src)
        check("trust_plan 은 _path_identity(동일성)를 쓰지 않는다 — 정확 키 정책(R1)", "_path_identity" not in plan_src)
        check("_trusted_exact: 형상 무관 예외 0(list projects · 비-dict 항목 · 비-dict 최상위 → False)",
              not _trusted_exact({"projects": [1]}, "/w/a") and not _trusted_exact([1], "/w/a")
              and not _trusted_exact({"projects": {"/w/a": 1}}, "/w/a") and not _trusted_exact({"projects": {"/w/a": {"hasTrustDialogAccepted": 1}}}, "/w/a")
              and _trusted_exact({"projects": {"/w/a": {"hasTrustDialogAccepted": True}}}, "/w/a"))
        check("_is_claude_command: 실행 형상만(basename claude/claude.exe/claude.cmd · claude-code cli.js)",
              _is_claude_command(["/Users/o/.local/bin/claude", "--continue"])
              and _is_claude_command(["C:\\Users\\o\\AppData\\claude.exe"])
              and _is_claude_command(["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"])
              and not _is_claude_command(["python3", "javis_preflight.py", "--seed-trust", "--config",
                                          "/Users/o/.cys/claude-default-dept-3"])
              and not _is_claude_command(["/usr/bin/python3", "-m", "notebooklm"]))
        ps_lines = [
            "  91703 /Users/o/.local/bin/claude CLAUDE_CONFIG_DIR=/w/cfg HOME=/Users/o",
            "  92160 /Users/o/.local/share/uv/tools/x/bin/python CLAUDE_CONFIG_DIR=/w/cfg HOME=/Users/o",
            "  93000 /Users/o/.local/bin/claude CLAUDE_CONFIG_DIR=/w/other HOME=/Users/o",
            "  94000 /bin/sh /tmp/x/claude CLAUDE_CONFIG_DIR=/w/cfg/",
            "  95000 python3 javis_preflight.py --seed-trust --config /w/cfg CLAUDE_CONFIG_DIR=/w/cfg",
            "  96000 /Users/o/.local/bin/claude CLAUDE_CONFIG_DIR=/w/cfg",
        ]
        check("ps -E 판정(구분자 없는 모드): 그 config 의 claude 만 양성(uv python 제외 · 검사기 자기 제외 · self_pid 제외 · 꼬리 / 허용) · 타 config claude 는 0 이 아니라 unresolved(R3) = (2, 6, 1)",
              _count_claude_in_ps_lines(ps_lines, "/w/cfg", self_pids={"96000"}) == (2, 6, 1))
        check("ps -E 판정: 공백 포함 env 값 보존(다음 NAME= 직전까지)",
              _count_claude_in_ps_lines(["  1 /usr/bin/claude CLAUDE_CONFIG_DIR=/w/My Dir HOME=/x"], "/w/My Dir") == (1, 1, 0))
        check("ps -E 판정: claude 형상인데 env 세그먼트 0 → unresolved(0 으로 흡수 금지 · R1 codex)",
              _count_claude_in_ps_lines(["  7 /Users/o/.local/bin/claude --continue", "  8 /usr/bin/python3 x.py"], "/w/cfg") == (0, 2, 1))
        check("ps -E 판정: env 는 보이는데 CLAUDE_CONFIG_DIR 없음 → 기본 ~/.claude 대상이면 양성 · 아니면 unresolved(R3 · 구분자 없는 모드는 claude 형상에 검증된 0 을 주지 않는다)",
              _count_claude_in_ps_lines(["  7 /Users/o/.local/bin/claude HOME=/x"], "/w/cfg") == (0, 1, 1)
              and _count_claude_in_ps_lines(["  7 /Users/o/.local/bin/claude HOME=/x"], _default_claude_config_dir()) == (1, 1, 0)
              and _count_claude_in_ps_lines(["  7 /Users/o/.local/bin/claude HOME=/x"], "/w/cfg", argv_lines=["7 /Users/o/.local/bin/claude"]) == (0, 1, 0))
        amb = "  7 claude -p CLAUDE_CONFIG_DIR=/w/other CLAUDE_CONFIG_DIR=/w/cfg HOME=/x"
        check("ps -E 판정(R3 codex): 경계 없는 줄은 CLAUDE_CONFIG_DIR= 세그먼트 어느 하나라도 일치하면 양성 · 불일치는 전부 unresolved(인자가 env 를 가릴 수 있다)",
              _count_claude_in_ps_lines([amb], "/w/cfg") == (1, 1, 0)
              and _count_claude_in_ps_lines([amb], "/w/zzz") == (0, 1, 1)
              and _count_claude_in_ps_lines(["  7 claude CLAUDE_CONFIG_DIR=/w/other HOME=/x"], "/w/cfg") == (0, 1, 1)
              and _count_claude_in_ps_lines(["  7 python3 x.py CLAUDE_CONFIG_DIR=/w/other HOME=/x"], "/w/cfg") == (0, 1, 0))
        argv_l = ["7 claude -p CLAUDE_CONFIG_DIR=/w/other", "8 claude -p CLAUDE_CONFIG_DIR=/w/cfg", "9 /bin/sh /x/claude", "10 claude"]
        env_l = ["7 claude -p CLAUDE_CONFIG_DIR=/w/other CLAUDE_CONFIG_DIR=/w/cfg HOME=/x", "8 claude -p CLAUDE_CONFIG_DIR=/w/cfg HOME=/x",
                 "9 /bin/sh /x/claude", "10 claude"]
        check("ps -E 판정(R3): argv 목록으로 경계 확정 — 인자 속 CLAUDE_CONFIG_DIR= 는 env 가 아니다(7 양성 · 8 은 기본 config) · env 비노출 sh 0 · 경계만 있는 claude(10) unresolved",
              _count_claude_in_ps_lines(env_l, "/w/cfg", argv_lines=argv_l) == (1, 4, 1)
              # ★R5 재핀(리뷰 codex D2 · 이 WP 의 R3 자기 핀): 대상이 **기본 config** 일 때, 겉보기 `CLAUDE_CONFIG_DIR=` 가
              #   있는 claude 줄은 '검증된 음성' 이 될 수 없다 — 같은 직렬화가 `OTHER="x CLAUDE_CONFIG_DIR=/other"`(그 프로세스는
              #   기본 config 사용)에서도 나오므로 지정의 존재 자체가 증거가 아니다. 7 번이 음성 → 미해결로 바뀐다(1,4,1 → 1,4,2).
              and _count_claude_in_ps_lines(env_l, _default_claude_config_dir(), argv_lines=argv_l) == (1, 4, 2)
              and _count_claude_in_ps_lines(["11 claude HOME=/x"], "/w/cfg", argv_lines=argv_l) == (0, 1, 1)
              and _count_claude_in_ps_lines(["8 claude --new HOME=/x"], "/w/cfg", argv_lines=argv_l) == (0, 1, 1)
              and _count_claude_in_ps_lines(["8 claude -p CLAUDE_CONFIG_DIR=/w/cfg2 HOME=/x"], "/w/cfg", argv_lines=argv_l) == (0, 1, 1)
              and _count_claude_in_ps_lines(["8 claude -p CLAUDE_CONFIG_DIR=/w/cfg HOME=/x CLAUDE_CONFIG_DIR=/w/other"], "/w/cfg", argv_lines=argv_l) == (0, 1, 0)
              # ★R4 재핀(codex 위임 반례 · 이 WP 자기 핀): pid 중복으로 argv 경계를 모르는 줄에 **우리 대상 바이트**가 보이면
              #   종전 unresolved 대신 **양성**이다(강행이 못 넘는다 · 경계를 모르니 argv 속 값도 env 로 본다 = 거부 방향).
              #   경계를 아는 줄에서 인자 속 값이 env 가 아니라는 R3 계약은 위 6줄이 그대로 지킨다.
              and _count_claude_in_ps_lines(["8 claude -p CLAUDE_CONFIG_DIR=/w/cfg HOME=/x"], "/w/cfg", argv_lines=["8 claude -p CLAUDE_CONFIG_DIR=/w/cfg", "8 claude"]) == (1, 1, 0)
              and _count_claude_in_ps_lines(["8 claude -p CLAUDE_CONFIG_DIR=/w/other HOME=/x"], "/w/cfg", argv_lines=["8 x", "8 y"]) == (0, 1, 1))
        check("_is_claude_command strict(R3): JS 런타임 argv[0] 뒤 어느 위치의 claude-code .js 도 형상(node --inspect …/cli.js) · 다른 인터프리터는 아님",
              _is_claude_command(["node", "--inspect", "/x/claude-code/cli.js"], strict=True)
              and _is_claude_command(["bun", "run", "/x/claude-code/cli.mjs"], strict=True)
              and not _is_claude_command(["python3", "x.py", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"], strict=True)
              and not _is_claude_command(["node", "/x/claude-code/cli.js.map"], strict=True))
        check("claude_procs_for_config darwin(R3→R4): 대상 경로에 ' NAME=' 형상이 있어도 스캔은 한다 — 불일치는 모호로 접혀 None(값 절단 오판 방지) · 형상 없는 출력이면 정당한 0",
              claude_procs_for_config("/tmp/work X=y", runner=lambda c: (0, "7 claude HOME=/x", ""), os_name="posix", platform="darwin")[0] is None
              and claude_procs_for_config("/tmp/work X=y", runner=lambda c: (0, "7 claude CLAUDE_CONFIG_DIR=/o HOME=/x", ""), os_name="posix", platform="darwin")[0] is None
              and claude_procs_for_config("/tmp/work X=y", runner=lambda c: (0, "7 python3 x.py HOME=/x", ""), os_name="posix", platform="darwin")[0] == 0)
        check("claude_procs_for_config nt: powershell 실패 → None(검증 불가)",
              claude_procs_for_config("/w/cfg", runner=lambda c: (1, "", "e"), os_name="nt")[0] is None)
        check("claude_procs_for_config nt: 전역 0 → 0(검증된 음성)",
              claude_procs_for_config("/w/cfg", runner=lambda c: (0, "\n 0 \n", ""), os_name="nt")[0] == 0)
        check("claude_procs_for_config nt: 전역 ≥1 → None(config 귀속 불가)",
              claude_procs_for_config("/w/cfg", runner=lambda c: (0, "3\n", ""), os_name="nt")[0] is None)
        check("nt 필터: claude 실행파일은 이름만으로 · node 는 CommandLine 부재(null)도 계수 · ErrorActionPreference Stop(R1 codex)",
              "$_.Name -like 'claude*'" in _WIN_CLAUDE_COUNT_PS_TMPL and "-not $_.CommandLine" in _WIN_CLAUDE_COUNT_PS_TMPL
              and "$ErrorActionPreference='Stop'" in _WIN_CLAUDE_COUNT_PS_TMPL)
        check("claude_procs_for_config darwin: ps 실패 → None",
              claude_procs_for_config("/w/cfg", runner=lambda c: (1, "", ""), os_name="posix",
                                      platform="darwin")[0] is None)
        def _two_ps(env_text):
            """주입 러너(R3): -E 없는 호출엔 각 줄의 env 세그먼트를 벗긴 argv 줄을 돌려준다(실 ps 의 접두 관계 재현)."""
            def runner(cmd):
                if "-E" in cmd:
                    return 0, env_text, ""
                return 0, "\n".join(l.split(None, 1)[0] + " " + _PS_ENV_SPLIT_RE.split(l.split(None, 1)[1])[0]
                                     for l in env_text.splitlines() if len(l.split(None, 1)) == 2), ""
            return runner
        check("claude_procs_for_config darwin: 주입 ps 출력 계수(96000 은 실 pid 가 아니므로 포함 = 3)",
              claude_procs_for_config("/w/cfg", runner=_two_ps("\n".join(ps_lines)), os_name="posix", platform="darwin")[0] == 3)
        check("claude_procs_for_config darwin: 양성 0 + env 비노출 claude 형상 → None · 양성 ≥1 이면 unresolved 보다 우선 n(강행 불가 방향)",
              claude_procs_for_config("/w/cfg", runner=_two_ps("  7 /Users/o/.local/bin/claude\n"),
                                      os_name="posix", platform="darwin")[0] is None
              and claude_procs_for_config("/w/cfg", runner=_two_ps("  7 /Users/o/.local/bin/claude\n  8 /Users/o/.local/bin/claude CLAUDE_CONFIG_DIR=/w/cfg\n"),
                                          os_name="posix", platform="darwin")[0] == 1)
        check("claude_procs_for_config darwin(R3): ps 2회 — argv 만(-o) 먼저 · 그 다음 -E · argv ps 실패 → None",
              '["ps", "-ax", "-ww", "-o", "pid=,command="]' in _pin_src(claude_procs_for_config)
              and '["ps", "-ax", "-ww", "-E", "-o", "pid=,command="]' in _pin_src(claude_procs_for_config)
              and _pin_src(claude_procs_for_config).index('"-ww", "-o"') < _pin_src(claude_procs_for_config).index('"-ww", "-E"')
              and claude_procs_for_config("/w/cfg", runner=lambda c: (1, "", "") if "-E" not in c else (0, "7 claude HOME=/x", ""),
                                          os_name="posix", platform="darwin")[0] is None)
        check("claude_procs_for_config darwin(R3): 한 줄에 CLAUDE_CONFIG_DIR= 가 둘인 라이브 claude(인자+env)는 argv 대조로 양성 — 검증된 0 이 되지 않는다(codex 재현)",
              claude_procs_for_config("/w/cfg", runner=lambda c: (0, "7 claude -p CLAUDE_CONFIG_DIR=/w/other" + ("" if "-E" not in c else " CLAUDE_CONFIG_DIR=/w/cfg HOME=/x"), ""),
                                      os_name="posix", platform="darwin")[0] == 1)
        check("claude_procs_for_config 미지원 플랫폼 → None",
              claude_procs_for_config("/w/cfg", runner=lambda c: (0, "", ""), os_name="posix",
                                      platform="freebsd")[0] is None)
        procfs_src = _pin_src(_count_claude_procfs)
        check("linux /proc: PermissionError 는 소유자 확인(같은 uid/판정 불가 = 미해결) · 양성 n 이 미해결보다 먼저 반환(R1 codex)",
              "st_uid" in procfs_src and procfs_src.index("if n > 0:") < procfs_src.index("if unresolved:"))
        seed_src = _pin_src(seed_trust)
        check("seed_trust: 이미 신뢰(정확 키 true) 는 프로세스 프로브 **앞**에서 반환(R1 — 가동 중 부서 재사용 경로 WARN 0)",
              seed_src.index("already-trusted") < seed_src.index("proc_counter(config_dir)"))
        # ★R2 재핀(리뷰 · 이 WP 의 9a4cda4/2f4be59 핀만): 프로브 위치 = 기존 문서가 있을 때만(makedirs·잠금·계획 뒤 `if existed:`) ·
        #   커밋 = 교환/link 무손실 CAS(os.replace 는 기구 부재 폴백에만).
        check("seed_trust: 프로브는 **기존 문서가 있을 때만**(`if existed:` 분기 안 · makedirs·잠금·계획 뒤) · 부재 문서는 no-probe 고지(R2)",
              seed_src.index("os.makedirs(config_dir") < seed_src.index("err = _acquire()") < seed_src.index("state = _read_plan()")
              < seed_src.index("if existed:\n") < seed_src.index("proc_counter(config_dir)")
              and "no-probe(.claude.json 부재" in seed_src)
        check("seed_trust: 잠금 기구 미가용(None)도 거부(무잠금 쓰기 금지)", "lock-unavailable" in seed_src)
        check("seed_trust: 임시파일 되읽기 → 교체 직전 존재+바이트 대조 → 백업(캡처 바이트 · O_EXCL) → displaced 이름으로 교환 순서 · 부재는 os.link · rename-over 0(R3)",
              seed_src.index("임시파일 되읽기 불일치") < seed_src.index("cur_existed != existed or cur != raw")
              < seed_src.index("os.O_EXCL") < seed_src.index("os.rename(tmp, displaced)") < seed_src.index("_exchange_paths(displaced, cfg)")
              < seed_src.index('"exchange-unavailable(') and "os.link(tmp, cfg)" in seed_src and "os.replace(" not in seed_src)
        check("seed_trust(R3 codex BLOCK): 교환 기구 부재 = 무조건 REFUSE exchange-unavailable(강행 플래그도 못 연다) · 하드링크 실패 = REFUSE link-failed · O_EXCL 배타 생성 폴백 0",
              '"exchange-unavailable(' in seed_src and "link-failed(" in seed_src and "excl-create" not in seed_src
              and seed_src.count("os.O_EXCL") == 1 and "_ExchangeUnavailable" in _pin_src(_exchange_paths)
              and not _exchange_paths.__globals__.get("_EXCHANGE_LAST_UNAVAILABLE"))
        check("seed_trust(R3): payload 직렬화·인코딩은 try 안(lone surrogate → ASCII 이스케이프 재직렬화 · UnicodeEncodeError 전파 0) · displaced 이름에 payload 지문",
              seed_src.index("try:\n") < seed_src.index("json.dumps(new, ensure_ascii=False") and "ensure_ascii=True" in seed_src
              and "_payload_digest(payload_b)" in seed_src)
        check("seed_trust: 교환 뒤 옛 inode 가 원본과 다르면 되교환(_restore_foreign) · 교체 後 되읽기 불일치/실패에 롤백 0(R1) · copy2 0",
              "_restore_foreign(displaced, cfg, payload_b, note)" in seed_src and "_rollback_file" not in seed_src
              and "shutil.copy2" not in seed_src and "커밋 상태로 둔다" in seed_src and "_trusted_exact(back, key)" in seed_src)
        rf_src = _pin_src(_restore_foreign)
        check("_restore_foreign: 되교환 실패 시 displaced 를 지우지 않는다(unlink 는 되교환 성공+payload 동일 분기에만) · 제3 쓰기는 conflict 이름 보존",
              rf_src.count("os.unlink(tmp)") == 1 and rf_src.index("back is not True") < rf_src.index("os.unlink(tmp)")
              and "SEED_TRUST_CONFLICT_PREFIX" in rf_src and "_exclusive_name(" in rf_src)
        ex_src = _pin_src(_exchange_paths)
        check("_exchange_paths: darwin renamex_np(c_char_p,c_char_p,c_uint · 0x2) · linux renameat2(c_int,c_char_p,c_int,c_char_p,c_uint · AT_FDCWD -100 · 2) · 비-posix None · ctypes 지연 import",
              "renamex_np" in ex_src and "0x2)" in ex_src and "renameat2" in ex_src and "-100, os.fsencode(a), -100" in ex_src
              and ex_src.index('if os.name != "posix":') < ex_src.index("import ctypes")
              and "errno.ENOTSUP, errno.EOPNOTSUPP, errno.EINVAL, errno.ENOSYS, errno.EXDEV" in ex_src)
        _d = hashlib.sha256(b"x").hexdigest()
        check("잔재 청소(R3): displaced 는 이름의 payload 지문(sha256 64hex)과 바이트 sha256 이 같을 때만 · conflict/구형 displaced/잠금 무접촉 · 비정규/링크 무접촉",
              _SEED_DISPLACED_RE.match(".claude.json.displaced-%s-20260906T000000Z-1" % _d)
              and not _SEED_DISPLACED_RE.match(".claude.json.displaced-20260906T000000Z-1")
              and not _SEED_DISPLACED_RE.match(".claude.json.displaced-%s-20260906T000000Z-1" % _d[:16])
              and not _SEED_DISPLACED_RE.match(".claude.json.conflict-%s-20260906T000000Z-1" % _d)
              and _payload_digest(b"x") == _d
              and "_payload_digest(data) != m.group(1)" in _pin_src(_sweep_stale_seed_tmp)
              and "_is_link_like(path)" in _pin_src(_sweep_stale_seed_tmp))
        check("잔재 청소: mkstemp 접미 8자만 · 잠금(-lock)·displaced·conflict 는 대상 아님(교환은 displaced 이름 아래에서만)",
              _SEED_TMP_LITTER_RE.match(".claude.json.seed-abc12_xy") and not _SEED_TMP_LITTER_RE.match(SEED_TRUST_LOCK_NAME)
              and not _SEED_TMP_LITTER_RE.match(".claude.json.displaced-20260906T000000Z-1")
              and not _SEED_TMP_LITTER_RE.match(".claude.json.conflict-20260906T000000Z-1")
              and not _SEED_TMP_LITTER_RE.match(".claude.json.seed-abc12_xy9"))
        check("_read_claude_json_bytes: 심링크/정션 + 비정규 파일(FIFO 등) 거부(열기 전 lstat S_ISREG)",
              "S_ISREG" in _pin_src(_read_claude_json_bytes))
        # ── ★R4(리뷰 반영 4) 핀 — 비정규 파일 무한대기 · ps 꼬리 모호 대상 · 커밋 뒤 청소 · C43 잠금 · CLI 마감 감시 ──
        rd_src = _pin_src(_read_claude_json_bytes)
        check("_read_claude_json_bytes(R4): 실제 open 도 _open_unblocking_ro(O_NOFOLLOW|O_NONBLOCK + fstat 정규 재확인) — lstat~open TOCTOU 도 막힘 0",
              "_open_unblocking_ro(cfg, nofollow=True)" in rd_src and "_open_nofollow(cfg" not in rd_src
              and "O_NONBLOCK" in _pin_src(_open_unblocking_ro)
              and "S_ISREG" in _pin_src(_open_unblocking_ro))
        tol_src = _pin_src(_read_json_tolerant)
        check("_read_json_tolerant(R4 codex major): 무가드 open 0 — _open_unblocking_ro + ValueError 도 None(FIFO 레지스트리/config 에서 preflight 정지 0) · 심링크 추종은 유지",
              "_open_unblocking_ro(path)" in tol_src and 'open(path, "rb")' not in tol_src
              and "except (OSError, ValueError)" in tol_src and "nofollow" not in tol_src)
        check("_sweep_stale_seed_tmp(R4): 후보 판독도 _open_unblocking_ro",
              "_open_unblocking_ro(path, nofollow=True)" in _pin_src(_sweep_stale_seed_tmp))
        if hasattr(os, "mkfifo"):
            _fd = tempfile.mkdtemp(prefix="pfseal-")
            try:
                _fifo = os.path.join(_fd, "f")
                os.mkfifo(_fifo)
                _bad = 0
                try:
                    _open_unblocking_ro(_fifo)          # writer 없는 FIFO — 종전 open 은 여기서 영구 대기했다
                except ValueError:
                    _bad = 1
                _reg = os.path.join(_fd, "r.json")
                with open(_reg, "w", encoding="utf-8") as _f:
                    _f.write('{"a": 1}')
                _lnk = os.path.join(_fd, "link.json")
                os.symlink(_reg, _lnk)
                check("R4 실측: writer 없는 FIFO 는 ValueError(막힘 0) · _read_json_tolerant(FIFO)=None · 정규/심링크 판독은 그대로",
                      _bad == 1 and _read_json_tolerant(_fifo) is None and _read_json_tolerant(_reg) == {"a": 1}
                      and _read_json_tolerant(_lnk) == {"a": 1})
            finally:
                shutil.rmtree(_fd, ignore_errors=True)
        check("_ps_target_ambiguous(R4 codex major): 판독기 파생 판정 — 머리·꼬리 공백류 · 줄 나누는 문자 · 경로 속 ' NAME=' 은 모호 · 보통 경로·내부 공백·내부 '=' 는 아니다",
              _ps_target_ambiguous("/w/cfg ") and _ps_target_ambiguous(" /w/cfg") and _ps_target_ambiguous("/w/a\nb")
              and _ps_target_ambiguous("/w/a\r") and _ps_target_ambiguous("/w/cfg\t") and _ps_target_ambiguous("/w/a X=y")
              and not _ps_target_ambiguous("/w/cfg") and not _ps_target_ambiguous("/w/my cfg")
              and not _ps_target_ambiguous("/w/a=b") and not _ps_target_ambiguous("/w/a\tb"))
        check("_open_unblocking_ro(R5 minor · R6 핀): Windows 텍스트 모드(CRLF 변환·0x1A EOF 절단) 회귀 금지 — flags 에 O_BINARY "
              "를 싣는다(POSIX 는 getattr 0 이라 무영향 · macOS 에서 행위로는 관측 불가하므로 소스 핀이 유일한 회귀 장치)",
              'getattr(os, "O_BINARY", 0)' in _pin_src(_open_unblocking_ro)
              and _pin_src(_open_unblocking_ro).index("flags = os.O_RDONLY")
              < _pin_src(_open_unblocking_ro).index('getattr(os, "O_BINARY", 0)')
              < _pin_src(_open_unblocking_ro).index("fd = os.open(path, flags)"))
        check("_ps_env_value_present(R5 codex major · R6 핀): 뒤 경계는 **아무 공백**이다 — 뒤따르는 변수 이름이 셸 식별자가 "
              "아니어도(BAD-NAME= · BASH_FUNC_f%%=) 양성 관측이 살아 있어야 --force-unverified 가 라이브 claude 를 못 넘는다",
              _ps_env_value_present("CLAUDE_CONFIG_DIR=/w/a BAD-NAME=x", "CLAUDE_CONFIG_DIR", "/w/a")
              and _ps_env_value_present("CLAUDE_CONFIG_DIR=/w/a BASH_FUNC_f%%=() {", "CLAUDE_CONFIG_DIR", "/w/a")
              and _ps_env_value_present("CLAUDE_CONFIG_DIR=/w/a 1BAD=x", "CLAUDE_CONFIG_DIR", "/w/a"))
        check("_ps_env_value_present(R4 codex major): 분할 前 원문 대조 — 꼬리 공백·값 속 ' NAME=' 도 양성 · 경계 밖 부분일치는 음성",
              _ps_env_value_present("CLAUDE_CONFIG_DIR=/w/account  HOME=/x", "CLAUDE_CONFIG_DIR", "/w/account ")
              and _ps_env_value_present("CLAUDE_CONFIG_DIR=/w/a X=y HOME=/x", "CLAUDE_CONFIG_DIR", "/w/a X=y")
              and _ps_env_value_present("A=1 CLAUDE_CONFIG_DIR=/w/a", "CLAUDE_CONFIG_DIR", "/w/a")
              and not _ps_env_value_present("CLAUDE_CONFIG_DIR=/w/account2 HOME=/x", "CLAUDE_CONFIG_DIR", "/w/account")
              and not _ps_env_value_present("XCLAUDE_CONFIG_DIR=/w/a", "CLAUDE_CONFIG_DIR", "/w/a")
              and not _ps_env_value_present("CLAUDE_CONFIG_DIR=/w/a", "CLAUDE_CONFIG_DIR", ""))
        check("_ps_lines_torn(R4 codex 실증): CLAUDE_CONFIG_DIR= 를 담은 비-레코드 조각만 출력 전체를 모호로 만든다 · 정상 출력·빈 줄·형상 litter 는 아니다",
              _ps_lines_torn(["71 claude OTHER=x", "CLAUDE_CONFIG_DIR=/w/account  HOME=/x"])
              and not _ps_lines_torn(["71 claude", "", "  ", "72 node x.js"])
              and not _ps_lines_torn(["71 claude HOME=/x", "junk", "72 python"]))
        _amb_t = "/w/account "
        check("ps -E 판정(R4 codex major): 꼬리 공백 대상도 **양성은 원문 대조로 관측**(강행 불가) · 불일치는 검증된 0 이 되지 않는다 · 보통 대상의 검증된 0 은 불변",
              _count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=/w/account  HOME=/x"], _amb_t, argv_lines=["71 claude"]) == (1, 1, 0)
              and _count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=/w/other HOME=/x"], _amb_t, argv_lines=["71 claude"]) == (0, 1, 1)
              and _count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=/w/a X=y HOME=/x"], "/w/a X=y", argv_lines=["71 claude"]) == (1, 1, 0)
              and _count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=/w/other HOME=/x"], "/w/account", argv_lines=["71 claude"]) == (0, 1, 0)
              and _count_claude_in_ps_lines(["71 python3 x.py CLAUDE_CONFIG_DIR=/w/other HOME=/x"], _amb_t, argv_lines=["71 python3 x.py"]) == (0, 1, 0))
        check("ps -E 판정(R4 codex 실증): env 값 속 줄바꿈이 레코드를 쪼갠 출력은 보통 대상에서도 검증된 0 이 되지 않는다(조각 자체 1 + 찢김 전역 모호 1)",
              _count_claude_in_ps_lines(["71 claude OTHER=x", "CLAUDE_CONFIG_DIR=/w/account  HOME=/x"], "/w/account",
                                        argv_lines=["71 claude"]) == (0, 1, 2)
              # ★R4(codex 위임 반례): claude 설치 경로 속 줄바꿈이 **argv 자체**를 쪼개 남은 레코드가 claude 형상이 아니어도,
              #   우리 대상을 단 조각이 있으면 검증된 0 이 아니다(찢김 전역 모호는 claude 형상 줄에만 걸리므로 이것이 필요하다).
              and _count_claude_in_ps_lines(["71 /w/inst", "part/claude CLAUDE_CONFIG_DIR=/w/account HOME=/x"], "/w/account",
                                            argv_lines=["71 /w/inst"]) == (0, 1, 1)
              # 줄 끝 공백은 마지막 env 값의 바이트다(rstrip 금지) — 꼬리 공백 대상이 마지막 변수여도 양성으로 관측된다
              and _count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=/w/account "], "/w/account ",
                                            argv_lines=["71 claude"]) == (1, 1, 0)
              and _ps_target_ambiguous(None) and _ps_target_ambiguous(b"/w/a")
              and _count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=/w/a HOME=/x"], None, argv_lines=["71 claude"]) == (0, 1, 1))
        def _amb_runner(cmd):
            return (0, "71 claude CLAUDE_CONFIG_DIR=/w/other HOME=/x", "") if "-E" in cmd else (0, "71 claude", "")
        check("claude_procs_for_config darwin(R4): 꼬리 공백 대상 + 불일치 claude → None(귀속 불가 = 거부 방향 · 종전엔 0 을 돌려줘 라이브 config 에 썼다)",
              claude_procs_for_config(_amb_t, runner=_amb_runner, os_name="posix", platform="darwin")[0] is None)
        def _namekv_runner(cmd):
            return ((0, "71 claude CLAUDE_CONFIG_DIR=/w/a X=y HOME=/x", "") if "-E" in cmd else (0, "71 claude", ""))
        check("claude_procs_for_config darwin(R4 codex): ' NAME=' 대상의 선차단 조기 반환 삭제 — 스캔해서 **양성 1** 을 관측한다(강행이 라이브 좌석을 못 넘는다)",
              claude_procs_for_config("/w/a X=y", runner=_namekv_runner, os_name="posix", platform="darwin")[0] == 1
              and "env 경계 형상" not in _pin_src(claude_procs_for_config))
        check("seed_trust(R4): 커밋 확정(note commit=)이 잔재 청소(unlink) **앞** — 청소 실패가 성공을 ERROR 로 뒤집지 않는다(양 커밋 경로)",
              seed_src.index('note.append("commit=exchange")') < seed_src.index("os.unlink(displaced)")
              and seed_src.index('note.append("commit=link")') < seed_src.index("os.unlink(tmp)")
              and "displaced-left(" in seed_src and "tmp-left(" in seed_src)
        mcp_src = _pin_src(Preflight._enable_mcp_server)
        check("C43(R4 리뷰 minor): _enable_mcp_server 는 시더와 같은 잠금(.claude.json.seed-lock) 아래에서만 읽기~쓰기 — 못 잡으면 무쓰기 사유 반환",
              "SEED_TRUST_LOCK_NAME" in mcp_src and "_try_lock_nb(lf)" in mcp_src and "_open_nofollow(lock_path" in mcp_src
              and mcp_src.index("_try_lock_nb(lf)") < mcp_src.index("_read_json_tolerant(config_path)")
              and mcp_src.index("_try_lock_nb(lf)") < mcp_src.index("os.replace(tmp, config_path)")
              # ★R5(리뷰 minor · 이 WP 의 R4 자기 핀 재핀): 잠금 아래 판독도 막히지 않는 가드 경유(무가드 open 0) —
              #   FIFO 에서 막히면 preflight --fix 가 **시드 잠금을 쥔 채** 영구 정지한다.
              and "json.load(open(" not in mcp_src and "_read_json_tolerant(config_path)" in mcp_src
              and "json.load(open(" not in _pin_src(Preflight._mcp_enabled)
              and "쓰기 보류" in mcp_src
              and "self._enable_mcp_server(path, SERENA_PROJECT" in _pin_src(Preflight.c43_serena)
              and "활성화 보류 — " in _pin_src(Preflight.c43_serena))
        check("--seed-trust 마감 감시(R4): 기본 20초 · env 로 조정 · 0 이하 = 끔 · 비수치는 기본 유지",
              _seed_trust_timeout_secs({}) == 20.0 and _seed_trust_timeout_secs({_SEED_TRUST_TIMEOUT_ENV: "1.5"}) == 1.5
              and _seed_trust_timeout_secs({_SEED_TRUST_TIMEOUT_ENV: "0"}) is None
              and _seed_trust_timeout_secs({_SEED_TRUST_TIMEOUT_ENV: "-3"}) is None
              and _seed_trust_timeout_secs({_SEED_TRUST_TIMEOUT_ENV: "zzz"}) == 20.0)
        check("_same_dir(★R5 codex major D1): 표기가 아니라 파일시스템 동일성 3값 — 같은 문자열/실재 동일성 True · 부재류는 "
              "증명된 다름 False · 조회 불가는 None(검증된 0 흡수 금지)",
              _same_dir("/w/a", "/w/a") is True and _same_dir(os.getcwd(), os.path.join(os.getcwd(), ".")) is True
              and _same_dir("/w/a", "/w/b") is False and _same_dir(None, "/w/a") is None
              and _same_dir("/" + "z" * 5000, os.getcwd()) is False)
        check("claude_project_key(★R5 codex major D1): darwin 저장 표기 오라클(F_GETPATH) — 구조가 같을 때만 채택 · "
              "오라클 부재/구조 불일치/samestat 불일치는 종전 realpath",
              "_fgetpath(physical)" in _pin_src(claude_project_key)
              and "_same_path_shape(got, physical)" in _pin_src(claude_project_key)
              and "samestat" in _pin_src(claude_project_key)
              and _same_path_shape("/a/CYSjavis/x", "/a/cysjavis/x") and not _same_path_shape("/a/b", "/a/b/c")
              and not _same_path_shape("/a/other", "/a/b"))
        check("값 후보(★R5 codex major D2): `NAME=` 뒤 공백으로 끝나는 접두 전부 · 길이 상한까지 **소진**(절단 아님) · 개수 "
              "상한에 걸리면 절단(미해결)",
              _ps_env_value_candidates("CLAUDE_CONFIG_DIR=/a b HOME=/h") == ["/a", "/a b", "/a b HOME=/h"]
              and not _ps_env_candidates_truncated("CLAUDE_CONFIG_DIR=/a b HOME=/h")
              and _ps_env_candidates_truncated("CLAUDE_CONFIG_DIR=" + " ".join("t%d=x" % i for i in range(_PS_VALUE_CANDIDATE_MAX + 5)))
              and _ps_env_value_candidates("XCLAUDE_CONFIG_DIR=/a") == [])
        check("중단된 트랜잭션(★R5 codex major D4 · ★R7 재핀): 의도 저널은 **rename 보다 먼저** 내구적으로 공개되고"
              "(못 쓰면 교환 0) 회수는 잠금 아래 **청소·already-trusted 앞** · 자동 재시드 0",
              # ★R7 재핀(리뷰 codex major · plan §8 '의도적 기본값 변경만 재핀'): ⓐ 회수 호출에 note 인자가 붙어
              #   닫는 괄호까지 박은 종전 핀 문자열이 못 쓰게 됐고 ⓑ already-trusted 사유에 꼬리가 붙었으며
              #   ⓒ **저널 < rename** 이 이 라운드의 계약이다(종전은 rename < 저널이어도 통과했다).
              seed_src.index("_recover_interrupted_seed(config_dir, cfg") < seed_src.index("_sweep_stale_seed_tmp(config_dir)")
              < seed_src.index('"already-trusted(key=%s)%s" % (key')     # 본문 반환 지점(머리 주석 언급이 아니라)
              and seed_src.index("_write_seed_intent(config_dir") < seed_src.index("os.rename(tmp, displaced)")
              < seed_src.index("_exchange_paths(displaced, cfg)")
              and "의도 저널 기록 실패" in seed_src
              and "seed_trust(" not in _pin_src(_recover_interrupted_seed)
              and "os.link(tmp, path)" in _pin_src(_write_seed_intent)
              # ★R6 재핀(리뷰 codex major M3): 종전 핀은 `_fsync_dir(config_dir)` 문자열만 봤다 — 그 헬퍼는 open/fsync
              #   실패를 **전부 삼켜서**(EIO 포함) '이름이 먼저 내구적' 이라는 의무를 세우지 못한 채 성공을 돌려줬다.
              #   이제 strict 를 요구하고, 내구성 실패 시 공개한 저널 이름을 회수하는지까지 본다.
              and "_fsync_dir(config_dir, strict=True)" in _pin_src(_write_seed_intent)
              and "_unlink_quiet(path)" in _pin_src(_write_seed_intent))
        fs_src = _pin_src(_fsync_dir)
        check("_fsync_dir(R6 리뷰 codex major): strict 는 **open 실패를 언제나 전파**하고(open 단계 EINVAL 은 fsync 미지원의 "
              "증거가 아니다) fsync 단계만 미지원 errno 를 통과 · Windows(비-posix)는 종전대로 무동작",
              'if os.name != "posix":' in fs_src and fs_src.index('if os.name != "posix":') < fs_src.index("os.open(")
              and "if strict:\n            raise" in fs_src
              and "e.errno not in _DIR_FSYNC_UNSUPPORTED" in fs_src
              and getattr(errno, "EIO", None) not in _DIR_FSYNC_UNSUPPORTED
              and getattr(errno, "ENOSPC", None) not in _DIR_FSYNC_UNSUPPORTED
              and getattr(errno, "EACCES", None) not in _DIR_FSYNC_UNSUPPORTED)
        check("_lexists_strict(R6 리뷰 codex): 조회 실패는 '부재' 가 아니다 — 부재류만 False · 나머지 OSError 는 None",
              _lexists_strict(os.path.join(os.path.dirname(os.path.abspath(__file__)), "\x00nope")) is False
              and _lexists_strict(os.path.abspath(__file__)) is True
              and _lexists_strict(os.path.join(os.path.abspath(__file__), "under-a-file")) is False
              and "os.lstat(path)" in _pin_src(_lexists_strict)
              and "os.path.lexists" not in _pin_src(_recover_interrupted_seed))
        check("_active_document_healthy(R6 리뷰 codex major): 잘린 활성 문서는 **건강이 아니다** — 보존 사본 삭제의 근거로 "
              "`_parse_claude_json` 의 '빈 파일 → {}' 관용을 쓰지 않는다",
              "_parse_claude_json(" not in _pin_src(_active_document_healthy)   # 호출 0(주석의 이름 언급은 무관)
              and "raw.strip()" in _pin_src(_active_document_healthy)
              and "utf-8-sig" in _pin_src(_active_document_healthy))
        check("_sweep_stale_seed_tmp(R6 리뷰 codex): displaced 지문 청소는 활성 문서가 유효할 때만(저널 없는 payload 잔재가 "
              "유일한 완전한 사본일 수 있다) · mkstemp 잔재는 무관하게 청소",
              "_active_document_healthy(" in _pin_src(_sweep_stale_seed_tmp)
              and _pin_src(_sweep_stale_seed_tmp).index("_SEED_TMP_LITTER_RE.match(n)")
              < _pin_src(_sweep_stale_seed_tmp).index("_active_document_healthy("))
        check("_exclusive_name(★R7 리뷰 claude minor): 보존 이름은 **증명된 부재**에서만 확정한다 — 3값 "
              "`_lexists_strict`(True·None 은 다음 후보) · `os.path.lexists` 0 · 상한 소진은 OSError(쓰지 않는다)",
              "os.path.lexists(" not in _pin_src(_exclusive_name)   # 호출 0(독스트링의 이름 언급은 무관)
              and "_lexists_strict" in _pin_src(_exclusive_name)
              and _exclusive_name("/p/x-", lexists=lambda c: False).endswith("-%d" % os.getpid())
              and _exclusive_name("/p/x-", lexists=lambda c: c.count("-") < 3).endswith("-1")
              and _exclusive_name("/p/x-", lexists=lambda c: None if c.count("-") < 3 else False).endswith("-1")
              and _st_raises(OSError, _exclusive_name, "/p/x-", lexists=lambda c: None)
              and _st_raises(OSError, _exclusive_name, "/p/x-", lexists=lambda c: True))
        og_src = _pin_src(_orphan_recovery_guard)
        check("_orphan_recovery_guard(★R7 리뷰 codex major): 저널 없는 **지문 일치** 보존 사본 + 불건강 활성 = 대체 문서 "
              "생성 금지 — 사본을 자동 삭제되지 않는 이름으로 옮긴 뒤 REFUSE(1회) · 회수 뒤·청소 앞",
              seed_src.index("_recover_interrupted_seed(config_dir, cfg") < seed_src.index("_orphan_recovery_guard(config_dir, cfg)")
              < seed_src.index("_sweep_stale_seed_tmp(config_dir)")
              and '"REFUSE", "unresolved-recovery(' in og_src and "_preserve_copy(" in og_src
              and "_digest_of_regular(" in og_src            # 청소가 지울 수 있는 것만 막는다(무한 거부 0)
              and "orphan-scan-failed" in og_src             # 열거 실패는 '사본 없음' 이 아니다
              and og_src.index("os.listdir(config_dir)") < og_src.index("_active_document_healthy(cfg)"))
        check("_preserve_copy(★R7): 보호는 **네임스페이스 이동**이다 — conflict 접두는 어느 자동 삭제 경로에도 없다"
              "(지문 청소·잔재 청소가 그 이름을 지우지 못한다) · 바이트는 rename 하나로 불변",
              "SEED_TRUST_CONFLICT_PREFIX" in _pin_src(_preserve_copy) and "os.rename(path, target)" in _pin_src(_preserve_copy)
              and "SEED_TRUST_CONFLICT_PREFIX" not in _pin_src(_sweep_stale_seed_tmp)
              and not _SEED_TMP_LITTER_RE.match(SEED_TRUST_CONFLICT_PREFIX + "x")
              and not _SEED_DISPLACED_RE.match(SEED_TRUST_CONFLICT_PREFIX + "a" * 64 + "-1"))
        rec_src = _pin_src(_recover_interrupted_seed)
        check("회수 ⓑⓒ(★R7 리뷰 codex): '건강' 은 '원본이 그 안에 있다' 가 아니다 — 활성 바이트가 저널의 두 지문 중 "
              "하나와 같을 때만 사본을 지운다(아니면 보존 이름으로 이동) · 판독 실패는 보존 방향",
              rec_src.count("_copy_is_redundant(cfg, (rec[\"captured_sha256\"], rec[\"payload_sha256\"]))") == 2
              and rec_src.count("_preserve_copy(dpath)") == 2
              and "_active_digest(cfg)" in _pin_src(_copy_is_redundant)
              and _copy_is_redundant.__doc__ is not None
              and _active_digest(os.path.join(os.path.dirname(os.path.abspath(__file__)), "no-such-file")) is None)
        check("_release_uncommitted_copy(★R7 리뷰 codex): 교환이 성립하지 않은 자리(syscall 실패 · 기구 부재)에서 "
              "사본 삭제는 **활성 문서가 유효할 때만** — 아니면 보존 이름으로",
              seed_src.count("_release_uncommitted_copy(cfg, displaced, intent)") == 2
              and "_active_document_healthy(cfg)" in _pin_src(_release_uncommitted_copy)
              and "_preserve_copy(dpath)" in _pin_src(_release_uncommitted_copy))
        check("self-test 러너(★R7 리뷰 claude minor): 순서 핀의 소스는 `_pin_src` 경유 — 핀 문자열이 사라지면 그 핀 "
              "하나만 FAIL 이고 결과 줄은 반드시 나온다(종전엔 ValueError 로 전체 중단)",
              (_PinSrc("abc").index("zz") < 5) is False and (5 < _PinSrc("abc").index("zz")) is False
              and _PinSrc("abcabc").index("b") == 1 and _PinSrc("abcabc").rindex("b") == 4
              and _PinSrc("abc").rindex("zz") is _NOWHERE
              and "except Exception as e" in _pin_src(_self_test)
              and "self-test 중단" in _pin_src(_self_test))
        check("_recover_interrupted_seed(R6 리뷰 codex major): 저널 열거 실패는 '저널 없음' 이 아니라 REFUSE",
              "journal-scan-failed" in _pin_src(_recover_interrupted_seed)
              # 문(statement)으로서의 `return None` 이 열거 앞에 0(주석 속 이름 언급은 무관)
              and "\n        return None" not in _pin_src(_recover_interrupted_seed).split("for n in names:")[0])
        dl_src = _pin_src(_start_seed_deadline)
        main_seed_src = _pin_src(_seed_trust_main)
        check("--seed-trust 마감 감시(R4): 데몬 타이머(SIGALRM 0 · Windows 안전) · 초과는 REFUSE 1줄 뒤 os._exit(2) · 정상 경로는 취소",
              "threading.Timer" in dl_src and "t.daemon = True" in dl_src and "os._exit(SEED_TRUST_REFUSE)" in dl_src
              and "signal" not in dl_src
              and "_start_seed_deadline(secs" in main_seed_src and "cancel()" in main_seed_src
              and main_seed_src.index("_start_seed_deadline(secs") < main_seed_src.index("seed_trust(args.config"))
        gapread_src = _pin_src(Preflight._trust_gap_workspaces)
        check("C58 갭 탐지 판독(★R5 재핀 · 이 WP 의 R4 자기 핀): 막히지 않는 관용 판독(_read_json_tolerant) — FIFO 정지 0 이면서 "
              "심링크는 따라간다(시더 가드는 심링크를 거부해 영구 거짓 갭을 만들었다) · 쓰기 정책은 시더가 따로 판정",
              "_read_json_tolerant(config_path)" in gapread_src and "_read_claude_json_bytes(" not in gapread_src
              and "open(" not in gapread_src)
        check("_is_claude_command strict(env 비노출 줄): argv[0] 이름/설치 경로 · argv[0..1] claude-code .js 만 — tail/less/grep 인자 속 claude 제외(R2)",
              not _is_claude_command(["tail", "-f", "/x/logs/claude"], strict=True)
              and not _is_claude_command(["less", "/Users/o/.local/bin/claude"], strict=True)
              and not _is_claude_command(["tail", "-f", "/x/claude-code/debug.log"], strict=True)
              and not _is_claude_command(["python3", "x.py", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"], strict=True)
              and _is_claude_command(["/Users/o/.local/bin/claude", "--continue"], strict=True)
              and _is_claude_command(["/Users/o/.local/share/claude/versions/2.1.263", "--effort"], strict=True)
              and _is_claude_command(["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"], strict=True)
              and _is_claude_command(["tail", "-f", "/x/logs/claude"]))
        check("ps -E 판정: env 비노출 tail/less 줄은 unresolved 가 아니다 · env 비노출 claude 줄은 unresolved(R2)",
              _count_claude_in_ps_lines(["  7 tail -f /x/logs/claude", "  8 less /Users/o/.local/bin/claude"], "/w/cfg") == (0, 2, 0)
              and _count_claude_in_ps_lines(["  7 /Users/o/.local/bin/claude --continue"], "/w/cfg") == (0, 1, 1))
        seed_body = _PinSrc(seed_src.split('"""', 2)[2])   # 시그니처·docstring 뒤 본문
        check("seed_trust: --force-unverified 는 프로세스 확인 단계만 넘긴다(본문 참조 1회 · 프로브 뒤 · mkstemp 앞 · 교환 기구 부재 거부는 못 넘는다 R3)",
              seed_body.count("force_unverified") == 1
              and seed_body.index("proc_counter(config_dir)") < seed_body.index("force_unverified")
              < seed_body.index("tempfile.mkstemp("))
        check("seed_trust: cwd 부재(dir 아님) → ERROR(stale 경로 무변경 · R2) — 잠금·계획 앞",
              "cwd 가 존재하는 디렉터리가 아니다" in seed_src
              and seed_src.index("cwd 가 존재하는 디렉터리가 아니다") < seed_src.index("_acquire()"))
        check("seed_trust: 모든 .claude.json **읽기**(초기·교체 직전·되읽기)는 _read_claude_json_bytes(O_NOFOLLOW·정션·비정규 거부) 경로 · 직접 read open 0(R2 재핀: O_EXCL 배타 생성은 쓰기)",
              seed_src.count("_read_claude_json_bytes(cfg)") == 3 and "open(cfg, os.O_RDONLY" not in seed_src
              and 'open(cfg, "r' not in seed_src and "open(cfg)" not in seed_src)
        check("C58: 워크스페이스 판정은 _trust_gap_workspaces 하나(_is_cysjavis_workspace 삭제 · R2) · CLAUDE.md/_round 마커 요구 0",
              not hasattr(Preflight, "_is_cysjavis_workspace")
              and "CLAUDE.md" not in _pin_src(Preflight._trust_gap_workspaces)
              and "_round" not in _pin_src(Preflight._trust_gap_workspaces)
              and "_registry()" in _pin_src(Preflight._trust_gap_workspaces))
        gap_src = _pin_src(Preflight._trust_gap_workspaces)
        check("C58 갭 판정은 정확 키(claude_project_key) — 별칭 true 불인정(R1)",
              "claude_project_key(cwd)" in gap_src and "_path_identity(ws)" not in gap_src)
        c58_src = _pin_src(Preflight.c58_trust_harden)
        pair_src = _pin_src(Preflight._seed_pair)
        # ★R6 재핀(리뷰 codex major M4 · 근거는 plan §8 '기존 핀 테스트를 일괄 수정하지 않는다 — 반례를 추가하고,
        #   **의도적 기본값 변경만 재핀**' 이다 · ★R7 정정: 종전 이 자리가 인용하던 `CONTRACTS §B-(f)` 는 실재하지
        #   않는다(§B 는 번호 1~13 뿐 · 2026-09-07 실측 grep 0건)): 저널 판정 호출이 생겨 C58 안의
        #   `seed_trust(` **개수**로 읽기 전용을 재던 종전 핀은 더 못 쓴다. 대신 **구조로** 더 강하게 못 박는다 —
        #   쓰기 진입점은 `_seed_pair` 하나이고 `self.fix` 가드가 그 안에 있다(호출자 수와 무관하게 read-only 보존).
        check("C58 쓰기 진입점은 _seed_pair 하나 — c58 본문에 직접 seed_trust 호출 0(R6 재핀)",
              "seed_trust(" not in c58_src and "self._seed_pair(" in c58_src)
        check("C58 report 모드는 읽기 전용 — _seed_pair 가 self.fix 가드를 **먼저** 통과해야 seed_trust 에 닿는다(R6 재핀)",
              "if not self.fix:" in pair_src and pair_src.index("if not self.fix:") < pair_src.index("seed_trust(")
              and "backup=True" in pair_src and pair_src.count("seed_trust(") == 1)
        check("C58(R3): --fix 의 seed_trust 예외는 WARN 1줄로 접힌다(preflight 전체 중단 0)",
              pair_src.index("try:") < pair_src.index("seed_trust(") < pair_src.index("except Exception"))
        check("C58: 판정할 쌍 0 → SKIP(PASS 아님 · 판정 정직성 R1) · hook 배선 프로필 루프(스코프 ①) 제거",
              "self.add(cid, SKIP" in c58_src and c58_src.index("self.add(cid, SKIP") < c58_src.index("targets = []")
              and "discover_claude_settings" not in c58_src and "_hook_registered" not in c58_src)
        check("C58: 등재 쌍이 있는데 config dir 부재 → 침묵 통과 아님(WARN 줄 · 되살리기 0)",
              "config dir 부재" in c58_src and c58_src.index("config dir 부재") < c58_src.index("self._seed_pair("))
        check("C58(R6 리뷰 codex major): 미해결 교환 저널 점검은 갭과 **독립**이고 쌍 0(SKIP)보다 앞이다 — 읽기 전용 열거",
              c58_src.index("journals = self._survey_journals(reg)") < c58_src.index("self.add(cid, SKIP")
              and "self._seed_journals(cfg_dir)" in _pin_src(Preflight._survey_journals)
              and "os.listdir" in _pin_src(Preflight._seed_journals)
              and "seed_trust" not in _pin_src(Preflight._seed_journals))
        check("C58(R6 · codex 위임 반례 ②): 저널 잔존 판정의 정본은 수리 뒤 **전 대상 재열거**다 — rc 로도, 수리 전 "
              "스냅샷 유무로도 단정하지 않는다(수리가 새 저널을 남기는 정상 결과가 있다)",
              c58_src.count("self._survey_journals(reg") == 2
              and c58_src.index("journals = self._survey_journals(reg, journals)") > c58_src.index("for cfg in targets:")
              and c58_src.index("journals = self._survey_journals(reg, journals)") < c58_src.rindex("_journal_lines(journals)")
              and "previous" in _pin_src(Preflight._survey_journals))
        check("registry(R3 codex): depts.json 카탈로그 cwd 만 기동기 규칙으로 해석 — 루트('/' · '///' · '\\\\' · 'C:\\') → home · 부재 dir → home · 존재 dir 원값 · topology 는 무해석(관측 쌍 보존)",
              _resolve_catalog_cwd("/", home="/h") == "/h" and _resolve_catalog_cwd("///", home="/h") == "/h"
              and _resolve_catalog_cwd("\\\\", home="/h") == "/h" and _resolve_catalog_cwd("C:\\", home="/h") == "/h"
              and _resolve_catalog_cwd("c:", home="/h") == "/h" and _resolve_catalog_cwd("/nonexistent/zz/yy", home="/h") == "/h"
              and _resolve_catalog_cwd(os.getcwd(), home="/h") == os.getcwd() and _resolve_catalog_cwd(None, home="/h") is None
              and _resolve_catalog_cwd("rel/x", home="/h") == "rel/x"
              and "_resolve_catalog_cwd(meta[\"cwd\"])" in _pin_src(cysjavis_registry)
              and "_resolve_catalog_cwd" not in _pin_src(_topology_pairs))
        main_src = _pin_src(main)
        check("main: --seed-trust 가로채기가 argparse 앞",
              main_src.index("--seed-trust") < main_src.index("argparse.ArgumentParser("))
        tp_src = _pin_src(_topology_pairs)
        check("registry: topology 항목은 절대경로 문자열 쌍만(config 부재 추정 귀속 0 · 상대경로 0) · agent 정확히 'claude' 만(R1)",
              "_abs_str(c) and _abs_str(w)" in tp_src and 'e.get("agent") != _TOPOLOGY_CLAUDE_AGENT' in tp_src
              and _TOPOLOGY_CLAUDE_AGENT == "claude")
        check("registry: topology entries 비-list(명시 null·수·객체)는 판독불가 1건 · 키 부재는 빈 로스터 · 예외 0(R2 codex)",
              '"entries" not in t' in tp_src and "isinstance(entries, list)" in tp_src)
        full = {"pairs": {"a": {"x": "/x"}, "b": {"y": "/y"}}, "configs": {"a": "/A", "b": "/B"}, "sources": ["s"],
                "unreadable": [], "scope": "full"}
        sc = _scope_registry(full, "부서", ["/A/settings.json"]) if _path_identity("/A") == "a" else None
        check("_scope_registry: reason None 전체 · narrow [] 빈(none) · 부서 = 그 계정만(account)",
              _scope_registry(full, None, None) is full
              and _scope_registry(full, "임시", [])["pairs"] == {} and _scope_registry(full, "임시", [])["scope"] == "none"
              and _scope_registry({"pairs": {_path_identity("/A"): {"x": "/x"}, _path_identity("/B"): {"y": "/y"}},
                                   "configs": {_path_identity("/A"): "/A", _path_identity("/B"): "/B"}, "sources": ["s"]},
                                  "부서", ["/A/settings.json"])["pairs"] == {_path_identity("/A"): {"x": "/x"}})
        check("registry 격리: cysjavis_registry 는 _discover_isolation_block 을 _scope_registry 로 접는다(무조건 빈 반환 0 · R1)",
              "_scope_registry(reg, reason, narrow)" in _pin_src(cysjavis_registry))
        check("state dir: pipe_slug 미러(마지막 컴포넌트 · 영숫자-_ · 'cys'/빈 = 루트) · LOCALAPPDATA 부재 = None(위치 발명 0 · R1)",
              _pipe_slug(r"\\.\pipe\cys-dept-dept-3") == "cys-dept-dept-3" and _pipe_slug(r"\\.\pipe\cys") == "cys"
              and _pipe_slug("") == "")
        check("state dir: unix 소켓은 dirname 그대로(부모 부재에도 폴백 0) · 미기록 = ~/.local/state/cys-dept-<name>(cys-dept 규약)",
              _dept_state_dir("d", "/nonexistent/parent/cys.sock", os_name="posix") == "/nonexistent/parent"
              and _dept_state_dir("d", None, os_name="posix").endswith(os.path.join(".local", "state", "cys-dept-d")))
    except Exception as e:
        # ★R7(리뷰 claude minor): 핀 표현식 하나가 예외로 죽어도 **결과 줄은 반드시 낸다** — 종전엔 트레이스백이
        #   self-test 전체를 삼켜 나머지 핀의 상태가 가려졌다(회귀 진단이 트레이스백 1개로 축소).
        total[0] += 1
        fails.append("self-test 중단(%s: %s) — 이 지점 이후의 핀은 미평가" % (type(e).__name__, e))
        print("  FAIL  %s" % fails[-1])
    print("결과: PASS %d / FAIL %d" % (total[0] - len(fails), len(fails)))
    return 1 if fails else 0


def main():
    # --self-test 가로채기 — argparse 앞(팩 bin 도구 관례: 인자 스키마와 독립인 자기검증 채널).
    if "--self-test" in sys.argv[1:]:
        return _self_test()
    # ★WP-2(0.14.31): --seed-trust 가로채기 — 같은 관례(argparse 앞·인자 스키마 독립). cys-dept 가 계정 dir 생성
    #   직후·claude 기동 前 호출한다. 전체 체크 배터리를 돌리지 않는다(부트 핫패스 · 부작용 = 그 .claude.json 1개).
    if "--seed-trust" in sys.argv[1:]:
        return _seed_trust_main(sys.argv[1:])
    ap = argparse.ArgumentParser(description="CYSJavis 결정론 부트 프리플라이트")
    ap.add_argument("--fix", action="store_true", help="수리 가능한 항목 자동 수리")
    # OPP-17: --fix 의 시스템 변경을 단일 Mutation 게이트로 수렴.
    ap.add_argument("--dry-run", action="store_true",
                    help="변경 없이 --fix 가 무엇을 할지 미리보기('[dry-run] Would …')")
    ap.add_argument("--safe", action="store_true",
                    help="시스템 무변경 — 무엇이 빠졌는지만(생산/공용 머신)")
    ap.add_argument("--allow-irreversible", action="store_true",
                    help="--fix 에서 전역 설치(npm -g·git clone) 집행 허용(기본=WARN-first 보류)")
    ap.add_argument("--json", action="store_true", help="JSON 출력")
    ap.add_argument("--skip", action="append", default=[], metavar="ID",
                    help="해당 검사 건너뜀 (예: --skip C12.daemon)")
    args = ap.parse_args()

    if args.safe and (args.dry_run or args.fix):
        ap.error("--safe 는 --dry-run/--fix 와 동시 사용 불가")
    if args.dry_run and args.fix:
        ap.error("--dry-run 은 --fix 와 동시 사용 불가")
    mode = ("safe" if args.safe else "dry" if args.dry_run
            else "fix" if args.fix else "report")

    pf = Preflight(fix=args.fix, skips=args.skip, mode=mode,
                   allow_irreversible=args.allow_irreversible)
    results = pf.run()
    fails = sum(1 for r in results if r["status"] == FAIL)
    warns = sum(1 for r in results if r["status"] == WARN)
    # dry/safe: "변경했나"가 아니라 "변경이 필요한가"를 보고 — planned 비어있지 않으면 변경 예정.
    planned_change = any(p["cid"] for p in pf.planned)

    if args.json:
        print(json.dumps(
            {"ok": fails == 0, "fails": fails, "warns": warns,
             "mode": mode, "planned": pf.planned,
             "pack_dir": pack_dir(), "checks": results},
            ensure_ascii=False, indent=2,
        ))
    else:
        for r in results:
            print("[%s] %s — %s" % (r["status"], r["id"], r["detail"]))
        print("─" * 60)
        if mode in ("dry", "safe"):
            tag = "DRY-RUN(미리보기)" if mode == "dry" else "SAFE(무변경 진단)"
            print("preflight[%s]: 비가역 외부설치 예정 %d건 · FAIL %d · WARN %d · 검사 %d"
                  % (tag, len(pf.planned), fails, warns, len(results)))
            if pf.planned:
                print("위 [DRYRUN]/[SAFE-GAP] 항목 = 비가역 external_install 변경 대상(--allow-irreversible 주의).")
            print("※ 가역 로컬 변경(soul/hook/settings/todo 등)은 이 모드에서 self.fix=False 로 "
                  "일괄 비집행 — 개별 미리보기는 비가역 외부설치 항목에 한정된다.")
        else:
            verdict = "READY (프로젝트 시작 준비 완료)" if fails == 0 else "NOT READY"
            print("preflight: %s — FAIL %d · WARN %d · 검사 %d"
                  % (verdict, fails, warns, len(results)))
            if fails:
                print("FAIL 항목을 수리하고 재실행하라. 이 출력 외의 추론으로 READY를 선언하지 마라.")
    # 종료코드: dry/safe = 0(변경 불필요)·2(변경 예정)·1(진단 FAIL). report/fix = 기존 계약 불변.
    if mode in ("dry", "safe"):
        if fails:
            return 1
        return 2 if planned_change else 0
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
