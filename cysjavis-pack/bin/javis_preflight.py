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
import hashlib
import json
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
        ("능동 점검", "주기 10분 능동 점검 의무(CSO 헌장 제3조)"),
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

# NotebookLM SOT 도구(nlm) 핀 — 2026-06-12 감사 커밋(v0.7.3).
# PyPI에 0.7.3+가 배포되면 "notebooklm-mcp-cli>=0.7.3" 핀으로 전환하라.
# (PyPI 0.7.2 이하는 질의 짧은답 누락·auth 오판·silent failure 미수정 — 핀 하향 금지)
NLM_MIN_VERSION = (0, 7, 3)
NLM_PIN = ("notebooklm-mcp-cli @ git+https://github.com/jacob-bd/notebooklm-mcp-cli"
           "@6d41c75e21dae89d7bf6f43a71e3095239a28281")

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

# ★소망상태 매니페스트의 파이썬 측 — **각성 티어**(awakening tier · A9 · W3).
#   "없으면 부트 발화 자체가 사라지는" 훅 집합이다: SessionStart(=/clear 후 지침 재주입) +
#   UserPromptSubmit(=마스터 선언 부트 발화). Rust 측 정본은 `src/pack.rs AWAKENING_HOOKS` 이고
#   **집합 대조는 bin/tests/run_bootstrap_health.py H-SEED-1** 이 한다(언어 경계=기계 대조·A11 규율).
#   등록 주체: session-start.sh=C08(FAIL) · role-bootstrap.sh=C28(★A21로 FAIL 티어 격상).
#   ※ 이 목록은 '소망상태'이고 SELFCORR_HOOKS 는 'C28 이 등록하는 집합'이다 — 교집합이
#     role-bootstrap 이며, H-SEED-1 이 그 포함관계까지 단언한다(집행자 누락 방지).
AWAKENING_HOOKS = [
    ("session-start.sh", [("SessionStart", None)]),
    ("role-bootstrap.sh", [("UserPromptSubmit", None)]),
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


def pack_dir():
    """pack 위치 결정 — src/pack.rs pack_dir()의 4단 폴백(PACK_DIR_ENV_KEYS)을 그대로 미러링한다."""
    for key in PACK_DIR_ENV_KEYS:
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


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
# ★결함(G16): settings.json 은 **3-writer 대상**이다 — python preflight(C08/C27/C28/C32/C33/C41) ·
#   Rust 시드/init-pack · 부서 마이그레이션. 그런데 preflight 의 네 등록기가 각자
#   `open(path + ".tmp")` → `os.replace` 를 재구현했고 tmp 이름이 **고정**이었다: 동시 writer 가
#   서로의 임시 파일에 써서 **교차 파손**(반쪽 JSON)을 만들고, 그러면 그 뒤 모든 등록기가
#   "파싱 실패 — 덮어쓰기 거부"로 수리를 영구 포기한다(A8 재검증이 지목한 지배 실패 모드).
#   Rust 측 원자화는 W2(A8rs)에서 착지했고, python 측 락·mkstemp 유틸은 W1a(javis_lock)에서
#   신설됐다 — W3 은 그 유틸의 **소비 이관**이다(신설 1회 + 소비 이관 = 원자 단위·비평1 #19).
# 계약: ①레인 무관 **파일별 락**(`<settings>.cys-lock`)으로 3-writer 직렬화 ②symlink 거부
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
    #    system 파일은 임베드로 치유**(수정본 <rel>.user 보존·C62 원장 보고). 불가침은
    #    user-owned(디렉티브·soul·CLAUDE·schedule)·seed-once(memory/·round 상태)뿐이다.
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
    def c11b_cys_dept_path(self):
        cid = "C11b.cys-dept-path"
        if self.skipped(cid):
            return
        # cys·cysd는 컴파일 바이너리(scripts/deploy_gate.py가 target/release→/opt/homebrew/bin 복사)지만
        # cys-dept는 팩 bash 스크립트(~/.cys/pack/bin/cys-dept)라 PATH에 없다 → CEO 디렉티브의
        # `cys-dept list` fan-out이 풀경로 없이 실패한다(GUI/백엔드는 pack_dir()/bin 풀경로라 무관).
        # cys 옆(=같은 PATH dir)에 팩 스크립트로의 심링크를 둬 노출한다 — 가역·멱등·자가치유(스킬 심링크와
        # 동일 규약: 실파일은 덮지 않음). 비가역 외부설치가 아니므로 may_mutate 불요(reversible local).
        src = os.path.join(pack_dir(), "bin", "cys-dept")
        if not os.path.isfile(src):
            self.add(cid, WARN, "팩에 bin/cys-dept 없음 — `cys init-pack` 재실행")
            return
        existing = shutil.which("cys-dept")
        if existing and os.path.realpath(existing) == os.path.realpath(src):
            self.add(cid, PASS, existing)
            return
        cys = shutil.which("cys")
        if not cys:
            self.add(cid, SKIP, "cys 부재로 PATH dir 판정 불가 (C11 먼저)")
            return
        link = os.path.join(os.path.dirname(cys), "cys-dept")  # cys와 같은 PATH dir
        if self._symlink_ok(link, src):
            self.add(cid, PASS, link)
            return
        if os.path.exists(link) and not os.path.islink(link):
            self.add(cid, WARN, "%s 실파일 존재 — 자동 심링크 보류(수동 확인)" % link)
            return
        if self.fix:
            try:
                # PATH 해소(which/셸)는 대상이 실행가능해야 한다 — 팩 cys-dept가 0644면 심링크해도
                # `cys-dept`가 안 잡힌다. 실행비트를 보강(가역·멱등)한 뒤 심링크한다(Fix1' 실행비트 의존 해소).
                st = os.stat(src).st_mode
                if not (st & 0o111):
                    os.chmod(src, st | 0o111)
                if os.path.islink(link):
                    os.unlink(link)  # stale/오타깃 심링크 교체
                os.symlink(src, link)
                self.add(cid, FIXED, "%s → %s 심링크(PATH 노출·+x 보강)" % (link, src))
            except OSError as e:
                self.add(cid, WARN, "심링크/실행비트 실패(%s 쓰기권한?): %s" % (os.path.dirname(link), e))
        else:
            self.add(cid, WARN, "cys-dept PATH 미노출 — --fix로 %s 심링크 생성" % link)

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
        """(enabled_bool, trust_bool) — 노드 .claude.json의 project별 활성화·신뢰 상태."""
        try:
            d = json.load(open(config_path, encoding="utf-8"))
        except (OSError, ValueError):
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
        """None=ok/no-change, str=사유. denylist write — _serena_enable_approved() True일 때만 호출."""
        if os.path.islink(config_path):
            return "symlink 거부: %s" % config_path
        try:
            data = json.load(open(config_path, encoding="utf-8"))
        except (OSError, ValueError) as e:
            return "파싱 실패 — 거부: %s" % e
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
            for label, path, needs_trust in self._serena_nodes():
                if os.path.isfile(path):
                    self._enable_mcp_server(path, SERENA_PROJECT, "serena",
                                            set_trust=needs_trust)
            gaps = self._serena_activation_gaps()   # 재탐지(멱등 검증)
            if not gaps:
                fixed.append("노드 .claude.json serena 활성화(승인 토큰)")
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
                     "강제 복원 플래그는 사용자 수정을 무백업으로 덮어쓰는 비가역 조작이라 "
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
    def _event_hook_registered(settings_path, event, script_name):
        """event 에 pack 경로의 script_name 이 등록돼 있나 (구 .config 경로는 미인정)."""
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        if not isinstance(data, dict):
            return False
        desired = _cys_hook_cmd(script_name)
        for entry in data.get("hooks", {}).get(event, []):
            for h in entry.get("hooks", []):
                if h.get("command", "") == desired:
                    return True
        return False

    def _register_event_hook(self, settings_path, event, script_name, matcher=None):
        """event 에 pack/hooks/script_name 등록. 성공=None, 실패=사유. 멱등은 호출부.
        _register_appbuild_hook 과 동일 규약(symlink 거부·파싱실패 거부·백업·원자적)."""
        cmd = _cys_hook_cmd(script_name)

        def _mutate(data):
            arr = data.setdefault("hooks", {}).setdefault(event, [])
            kept, have = _prune_stale_hook_entries(arr, script_name, cmd)
            if not have:
                entry = {"hooks": [{"type": "command", "command": cmd}]}
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
        fixed, warns = [], []
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
        # (b) 이벤트별 등록 (멱등 — 구 .config 경로는 미인정이라 패키지 경로로 신규 등록)
        # ★G1 sentinel: 격리(부서/임시) 팩은 글로벌 폴백 없이 등록 0 — 폴백은 resolve 가 소유한다.
        targets, forbidden = resolve_registration_targets()
        if forbidden and not targets:
            self.add(cid, SKIP, "등록 대상 없음 — %s" % forbidden)
            return
        # ★A21(W3) 훅별 중요도 티어: **각성 훅**(role-bootstrap→UserPromptSubmit) 미등록은
        #   C08(session-start)과 **대칭으로 FAIL** 이다. 종전엔 C28 전체가 WARN 이라, 부트 발화의
        #   유일한 트리거가 빠져 있어도 preflight 가 초록에 가까웠다(C08=FAIL vs C28=WARN 비대칭 —
        #   재감사 A21 확증). 나머지 자기교정 훅(inject·save·reflect·nudge·pack-guard)은 종전대로 WARN.
        fails = []
        for t in targets:
            for script_name, events in SELFCORR_HOOKS:
                if not os.path.isfile(os.path.join(pack_dir(), "hooks", script_name)):
                    continue
                tier_fatal = script_name in AWAKENING_SCRIPTS
                for event, matcher in events:
                    if self._event_hook_registered(t, event, script_name):
                        continue
                    if self.fix:
                        err = self._register_event_hook(t, event, script_name, matcher)
                        if err:
                            (fails if tier_fatal else warns).append(
                                "%s/%s 등록 실패: %s" % (os.path.basename(t), event, err))
                        else:
                            fixed.append("%s←%s(%s)"
                                         % (os.path.basename(t), script_name, event))
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
                     detail + " · ★각성 훅 미등록(부트 발화 불가 — C08 대칭 FAIL): "
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

    # ── C58 트러스트 하드닝 (개인 alias 프로필 config가 cysjavis 워크스페이스를 자동 신뢰) ──
    # 배경(실측): 개인 alias(claude-<profile> 등·config=~/.claude-<profile>)로 Claude 기동 시
    #   그 config의 .claude.json에서 cysjavis 워크스페이스 hasTrustDialogAccepted=False/부재면
    #   시작 시 "Ignoring N permissions.allow entries … workspace has not been trusted" 경고가
    #   flash하고 permissions.allow가 무시된다. 패키지 표준 경로(launch-agent→~/.cys/claude)는
    #   신뢰 프롬프트 자동확인이라 무경고지만, 개인 alias config는 독립 보장이 없어 갭 발생.
    #   C43 serena의 _enable_mcp_server(set_trust=)는 MCP 활성 시에만 조건부라 이 갭을 못 메운다.
    # ★보안 스코프(절대·티켓 §②): cysjavis가 관리하는 워크스페이스에만 신뢰 세팅 — 임의
    #   워크스페이스 blanket 신뢰 금지(신뢰 다이얼로그는 악성 .claude/settings.local.json 방어
    #   장치라 남용 시 보안구멍). 2중 스코프: ①대상 config = cysjavis 배선 프로필(우리 hook
    #   등록)의 .claude.json만 ②대상 워크스페이스 = 결정론 cysjavis 마커(_round/ 시스템 폴더 +
    #   CLAUDE.md의 cys 마커 토큰) 보유 project 항목만. 마커 없는 항목·stale 경로는 무변경.
    # C43과 독립(MCP 활성 무관). trust는 가역 로컬 변경이라 may_mutate(비가역 외부설치) 게이트가
    # 아니라 self.fix 게이트로 집행(dry/safe에선 self.fix=False → 탐지만). C57 다음 free id = C58.
    CYSJAVIS_WS_CLAUDEMD_MARKERS = ("cys ", "CYSjavis", "cysjavis", "claim-role")

    def _is_cysjavis_workspace(self, path):
        """결정론 판정 — 이 워크스페이스 경로가 cysjavis 관리 대상인가.
        마커: ①_round/ 시스템 폴더 존재 AND ②CLAUDE.md 존재 + cys 마커 토큰 포함.
        존재하지 않는 stale 항목·마커 부재는 False(blanket 신뢰 차단 · 티켓 §②)."""
        try:
            if not os.path.isdir(path):
                return False
            if not os.path.isdir(os.path.join(path, "_round")):
                return False
            cmd = os.path.join(path, "CLAUDE.md")
            if not os.path.isfile(cmd):
                return False
            text = open(cmd, encoding="utf-8", errors="replace").read()
            return any(m in text for m in self.CYSJAVIS_WS_CLAUDEMD_MARKERS)
        except OSError:
            return False

    def _trust_gap_workspaces(self, config_path):
        """읽기전용 탐지 — .claude.json 미변경. 신뢰 갭(cysjavis 워크스페이스인데
        hasTrustDialogAccepted가 True 아님) 워크스페이스 경로 리스트."""
        try:
            data = json.load(open(config_path, encoding="utf-8"))
        except (OSError, ValueError):
            return []
        gaps = []
        for ws, ent in (data.get("projects", {}) or {}).items():
            if not isinstance(ent, dict):
                continue
            if not self._is_cysjavis_workspace(ws):
                continue
            if not ent.get("hasTrustDialogAccepted"):
                gaps.append(ws)
        return gaps

    def _set_workspace_trust(self, config_path, workspaces):
        """denylist write — 지정 cysjavis 워크스페이스 항목의 hasTrustDialogAccepted만 True.
        None=ok/no-change, str=사유. 원자 쓰기·다른 필드 무변경·멱등·낙관적 동시성 가드."""
        if os.path.islink(config_path):
            return "symlink 거부: %s" % config_path
        try:
            with open(config_path, "rb") as f:
                raw = f.read()
        except OSError as e:
            return "읽기 실패 — 거부: %s" % e
        try:
            data = json.loads(raw.decode("utf-8"))
        except (ValueError, UnicodeDecodeError) as e:
            return "파싱 실패 — 거부: %s" % e
        if not isinstance(data, dict):
            return "최상위 비-object — 거부"
        projs = data.get("projects")
        if not isinstance(projs, dict):
            return "projects 부재/비-object — 거부"
        changed = False
        for ws in workspaces:
            ent = projs.get(ws)
            if not isinstance(ent, dict):
                continue
            # 세팅 직전 마커 재검증(TOCTOU·blanket 신뢰 2차 차단): 갭 목록 산출 이후
            # 마커가 사라진 항목은 손대지 않는다.
            if not self._is_cysjavis_workspace(ws):
                continue
            if not ent.get("hasTrustDialogAccepted"):
                ent["hasTrustDialogAccepted"] = True
                changed = True
        if not changed:
            return None  # 멱등: 이미 전부 True → 무동작
        # 낙관적 동시성 가드(티켓 §④): 우리가 읽은 이후 라이브 세션이 config를 갱신했으면
        # os.replace로 그 갱신을 clobber하지 않도록 건너뛰고 보고한다(감지·경고).
        try:
            with open(config_path, "rb") as f:
                if f.read() != raw:
                    return "동시 변경 감지(라이브 세션?) — clobber 방지 위해 건너뜀"
        except OSError as e:
            return "재확인 실패 — 거부: %s" % e
        backup = config_path + ".bak-preflight"
        if not os.path.exists(backup):
            shutil.copy2(config_path, backup)
        tmp = config_path + ".tmp"
        # 라이브 config는 indent=2 pretty(실측) — 동일 포맷 유지로 다른 필드 텍스트 churn 0.
        with open(tmp, "w", encoding="utf-8") as f:
            f.write(json.dumps(data, ensure_ascii=False, indent=2))
        os.replace(tmp, config_path)
        return None

    def c58_trust_harden(self):
        cid = "C58.trust-harden"
        if self.skipped(cid):
            return
        # 대상 config = cysjavis 배선 프로필(우리 hook 등록)의 .claude.json만(스코프 ①).
        targets = []
        for settings_path in discover_claude_settings():
            if not self._hook_registered(settings_path):
                continue
            cfg = os.path.join(os.path.dirname(settings_path), ".claude.json")
            if os.path.isfile(cfg):
                targets.append(cfg)
        if not targets:
            self.add(cid, PASS, "cysjavis 배선 프로필 .claude.json 없음 — 트러스트 대상 없음")
            return
        set_lines = []
        gap_lines = []
        for cfg in targets:
            gaps = self._trust_gap_workspaces(cfg)
            if not gaps:
                continue
            if self.fix:
                err = self._set_workspace_trust(cfg, gaps)
                if err:
                    gap_lines.append("%s: 세팅 보류 — %s" % (cfg, err))
                    continue
                remain = self._trust_gap_workspaces(cfg)   # 재탐지(멱등 검증)
                for ws in gaps:
                    if ws not in remain:
                        set_lines.append("trust set: %s / %s" % (cfg, ws))
                for ws in remain:
                    gap_lines.append("%s / %s: 세팅 후 갭 잔존" % (cfg, ws))
            else:
                for ws in gaps:
                    gap_lines.append("trust gap: %s / %s (--fix로 세팅)" % (cfg, ws))
        if set_lines:
            detail = "cysjavis 워크스페이스 트러스트 세팅 — " + " | ".join(set_lines)
            if gap_lines:
                detail += " · 잔존: " + " | ".join(gap_lines)
            self.add(cid, FIXED if not gap_lines else WARN, detail)
        elif gap_lines:
            self.add(cid, WARN, "트러스트 갭 — " + " | ".join(gap_lines))
        else:
            self.add(cid, PASS,
                     "cysjavis 워크스페이스 트러스트 OK(갭 없음 · %d config 점검)" % len(targets))

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
        if not healed and not newp:
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
        stale = sorted(
            (rel, (now - float(v.get("ts", now))) / 86400.0)
            for rel, v in pending.items()
            if isinstance(v, dict) and (now - float(v.get("ts", now))) / 86400.0 > max_days
        )
        if not stale:
            self.add(cid, PASS, "병합 대기 %d건 — 전부 기한(%.0f일) 이내" % (len(pending), max_days))
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
    KNOWN_AGENTS_SCHEMA = (1, 2)

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
    # ★--fix 에서도 자동 수정 없음: 번들 안 파일 삭제는 App Management(TCC)에 막히고, 막히지
    #   않아도 added 가 missing 으로 바뀔 뿐 봉인은 복구되지 않는다(통째 재설치만이 복구).
    APP_SEAL_RECOVERY = ("복구는 번들 통째 재설치뿐 — 새 DMG의 cys.app 을 임시 폴더에 스테이징"
                         "(ditto --rsrc --extattr --acl) 후 `mv` 로 /Applications 교체"
                         "(/Applications 안 부분 수정은 App Management 보호에 막힘)")

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
        self.add(cid, WARN,
                 "코드서명 봉인 파손 — %s · 추가 %d건·수정 %d건·누락 %d건(예: %s)%s · 이 번들을 "
                 "브라우저로 배포하면 받는 쪽 첫 실행에서 Gatekeeper 가 '손상되었기 때문에 열 수 "
                 "없습니다'로 차단한다(공증·staple 정상이어도) · %s"
                 % (bundle, len(added), len(modified), len(missing), ", ".join(rel), pycache,
                    self.APP_SEAL_RECOVERY))

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


def _self_test():
    """--self-test 가로채기(팩 bin 도구 관례 — _check_bin_tool 이 부르는 그 형태와 동형).

    범위는 **순수 판정 핀만**이다(네트워크·데몬·subprocess 0): C79 heartbeat 판정 함수 +
    WARN 강등·배선의 정적 계약. 전체 체크 배터리는 self-test 대상이 아니다 — 그것은
    preflight 실행 자체가 검증한다(부작용 있는 체크를 여기서 돌리면 '관찰이 상태를
    바꾸는' PHIL-04 위반이 된다)."""
    import inspect
    fails, total = [], [0]

    def check(name, cond):
        total[0] += 1
        print("  %s  %s" % ("PASS" if cond else "FAIL", name))
        if not cond:
            fails.append(name)

    print("== javis_preflight.py self-test ==")
    now = 1000000.0
    check("heartbeat 부재 → absent", heartbeat_verdict(None, now, 90.0) == ("absent", None))
    check("age 10s → fresh", heartbeat_verdict(now - 10, now, 90.0) == ("fresh", 10.0))
    check("경계 age==max_age → fresh(게이트6 부등호 ≤ 동일)",
          heartbeat_verdict(now - 90, now, 90.0)[0] == "fresh")
    check("age 200s → stale", heartbeat_verdict(now - 200, now, 90.0)[0] == "stale")
    check("미래 mtime(시계 스큐) → fresh(보수측)",
          heartbeat_verdict(now + 5, now, 90.0)[0] == "fresh")
    src = inspect.getsource(Preflight.c79_cycle_verifier_heartbeat)
    check("C79 는 FAIL 을 내지 않는다(WARN 강등 계약 — 부트 비치명)",
          "self.add(cid, FAIL" not in src)
    check("C79 --fix 는 bootstrap-verifier --ensure(멱등)로 수리",
          '"bootstrap-verifier", "--ensure"' in src)
    check("C79 report 모드는 읽기 전용(fix 분기 밖 subprocess 없음)",
          src.index("if self.fix:") < src.index("subprocess.run("))
    run_src = inspect.getsource(Preflight.run)
    check("C79 run() 배선(마지막 고정 슬롯 C62 앞)",
          "c79_cycle_verifier_heartbeat" in run_src
          and run_src.index("c79_cycle_verifier_heartbeat") < run_src.index("c62_pack_heal_ledger"))
    print("결과: PASS %d / FAIL %d" % (total[0] - len(fails), len(fails)))
    return 1 if fails else 0


def main():
    # --self-test 가로채기 — argparse 앞(팩 bin 도구 관례: 인자 스키마와 독립인 자기검증 채널).
    if "--self-test" in sys.argv[1:]:
        return _self_test()
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
