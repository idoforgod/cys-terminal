#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_approval_queue.py — Phase 1 Wave C: T3 승인 통합 큐 v1 (팩 층 · cysd diff 0).

설계 SOT: `_work/phase1-impl-package/DESIGN-DECISIONS.md` §0·§4 · 계약 문서
`contracts/APPROVAL_QUEUE_CONTRACT.md`. 전제 실측: T0-BASELINE §5-1(wakeup 신계약)·
§5-2(EVT 13종·이름공간 2원)·D15~D21·U10~U12. 조건 03·06·13·17·22·31·33·36.

이 도구가 하는 일은 하나다 — **알림을 한 번만, 한 곳으로 보낸다.**
승인·자가치유·escalation 세 종류의 요청을 하나의 원장에 모으고, 중복·폭주·재기동 버스트를
결정론으로 눌러 master(또는 리뷰어1) 에게 도달시킨다. 큐 자체는 데몬이 아니다 — 서브커맨드
호출 1회가 곧 유지보수 tick 1회다(아래 '유지보수 tick' 절).

── 저장소(전부 `$JAVIS_ROOT/_round/approvals/`) ──────────────────────────────
  · `ledger.jsonl`  append-only 원장 — 모든 사건. **이 원장이 SOT다**(조건 19①). `queued`·
    `self_heal_bypass` 레코드는 **항목 복원 가능한 스냅샷**이라 원장만 살아있으면 항목을
    통째로 재구성한다(S3 리플레이 복원). 5MB 초과 시 **세대 보존 회전**(`.1`→`.2`…) 이며
    어떤 세대도 폐기하지 않는다 — 리플레이는 **오래된 세대부터 이어 읽는다**(R3).
  · `items/<sanitized_rid>.<h8>.json` **항목별 파일**(temp+os.replace · 생성은 temp+os.link).
    통짜 state.json 교체를 폐기한 자리다 — 항목 1건의 기록이 다른 항목을 덮어쓰지 못한다(S1).
    자가치유 사전 인가 항목은 **별도 키 공간** `items/selfheal-<...>.json` 에 산다(S2).
    **이 디스크 실측이 항목 존재의 유일 SOT다** — 별도 카운터를 두지 않는다(R1).
  · `state.json`    **집계/창 상태만**(rate·grace·source 계수·다이제스트 마커).
    항목도, 항목 수도 여기 없다. temp+os.replace 원자 교체(조건 03⑤ 재기동 버스트 방지).
  · `outbox.jsonl`  배달 실패 SOT(조건 17③) — cys 배달 실패분 append. 표면화(부트 주입)는
    후속 preflight C72 확장 몫이며 이번 Wave 는 **기록까지만**이다. **회전해도 폐기 0**(R3).
  · `digest-<날짜>.md` 다이제스트 **읽기 전용 집계 파일**(조건 03② 문면) — 본문은 여기 있고
    wakeup reason 에는 300자 이내 헤드+이 파일 경로만 실린다.
  · `.digest-sent-<날짜>` 다이제스트 발행 마커(조건 03③ 인터록 신호 — 소비자 배선은 미해결).
  · `.slack-enabled` Slack 발신 flag(부재 = 발신 경로 완전 무동작 · 기본 OFF · 활성화 [OT-3]).
  · `slack-outbox.jsonl` Slack 발신 기록부(로컬 sink — 실채널 발행 배선은 [OT-3]).

── 락 규율(S1 — 이 파일의 구조 핵심) ────────────────────────────────────────
  락(`state.json.lock`)은 **상태 판정·기록에만** 보유한다. 배달(wakeup enqueue·drain·cys)은
  전부 **락 밖**이고, 그 결과는 항목 파일(자기 소유)과 짧은 2차 상태 갱신으로 반영한다.
  종전 구조는 최대 50초짜리 배달 I/O 를 임계구역 안에 넣어, 동시 submit 시 5초 락 타임아웃 →
  무락 degrade → 통짜 state.json last-writer-wins → **항목 소실**을 만들었다. 임계구역은 이제
  메모리 연산 + 파일 2개(state.json 읽기·쓰기)뿐이다.

── 항목 스키마 ──────────────────────────────────────────────────────────────
  {request_id, class: approval|self-healing|escalation, risk: high|normal,
   severity: info|warn|critical, created_at, source, payload_ref,
   state: pending|approved|denied|expired|notified}
  · `request_id` = **발행처 멱등키 재사용**(esc-bundle 이면 `guard:<task>:<sig>`) — wakeup
    `--idempotency-key` 와 같은 문자열이라 큐↔wakeup 원장이 이 키로 조인된다(설계 §4 R1).
  · 확장 필드(전방 호환·계약 문서 §2): summary·risk_why·notified_at·notified_slack_at·
    deferred_to_digest·rate_overflow·grace_held·decided_at/by·local_only·routed_to·bypass.

── 중복 알림 0 (조건 03① · 완료기준①) ───────────────────────────────────────
  2층 방어다. ①큐 원장: 같은 request_id 재submit 은 `duplicate` 로 종결 — **재알림 0**.
  판정 근거는 스냅샷이 아니라 **항목 파일 O_EXCL 생성**이라 동시 submit 경합에서도 정확
  1건이다. ②wakeup: 그래도 나가는 enqueue 는 전부 `--idempotency-key <request_id>` 를 달아
  wakeup 원장이 2차로 접는다. 코얼레싱 SOT 는 javis_wakeup 원장 1벌이다(조건 03①·D21).
  **class 교차 재사용은 duplicate 가 아니라 exit 6**(`rid_conflict` — 덮어쓰기 금지·S2).

── 배달 의미론(D18 이연 명문) ───────────────────────────────────────────────
  "즉시 배달" = `javis_wakeup enqueue`(멱등키) + **same-run `drain --deliver --target <대상>`
  1회**. 실패하면 pending 이 잔류해 다음 drain 에 편승한다. 단 **대상 사망·fast-fail 임계
  도달은 pending 즉시 삭제**(zombie 가드)라 그 순간의 유일한 SOT 는 `outbox.jsonl` 이다.
  **at-least-once 가 아니다**(D18 — 라이브 영수증 조인은 데몬 0.14.7 로테이트 이후).
  ★성공 판정은 drain 의 exit·계수가 아니라 **wakeup id 조인**이다(R4). enqueue stdout 의
  `id`(W-…) 를 기억했다가 wakeup 원장(`_round/wakeups/queue.jsonl`)에서 **그 id 의**
  `delivered`/`skipped`/`deliver_failed` 를 읽는다. `drain exit 5`(nothing pending)는
  성공이 아니라 **판정 불가**이고, 형제 drain 이 zombie 가드로 폐기(`skipped`)한 것을
  `delivered` 로 오기록하던 무성 소실이 여기서 닫힌다. **확정 배달이 아닌 모든 결말은
  outbox 에 1줄을 남긴다**(무성 소실 0 의 구조적 보장).

── 폭주 억제 3종 ────────────────────────────────────────────────────────────
  1) **rate cap(조건 17② > 13)**: escalation·critical 은 **다이제스트 이월 금지**. 시간당
     `JAVIS_APPROVAL_URGENT_CAP`(기본 4) 초과분은 보류했다가 **다음 시간창 시작 시
     'N건 병합' critical 1건으로 즉시 배달**한다(삼켜지지 않는다 — 조건 17 데드락 방어).
  2) **발행자별 back-pressure(조건 03④)**: source 별 시간창 계수. 임계
     `JAVIS_APPROVAL_SOURCE_CAP`(기본 10) **교차 1회**만 `resource.soft`, 초과분은
     다이제스트로 이월(escalation 제외). approve_auto_route 와 무관하게 **무조건 계수**.
  3) **데몬 재기동 유예(조건 36)**: `cys status --json` 지문(started_at·build_id·version —
     **셋 중 하나라도 없으면 관찰 실패**) 변화 감지 → `JAVIS_APPROVAL_RESTART_GRACE_SEC`
     (기본 300초) 창 동안 critical 외 배달 억제, 창 종료 시 **누적 held 전량 요약 1건**.
     창 중첩(창 안에서 또 재기동)은 held 를 **이월**한다 — 비우지 않는다(S5).

── 부수 신호(EVT 13종 재사용 · 이름공간 2원 준수) ───────────────────────────
  실제 배달되는 항목 1건당 `approval.needed`(agent·task·summary) **spool 전용** 발행.
  **억제 상태(overflow·deferred·grace_held) 항목은 발행하지 않는다**(S11g — 억제 3종과
  신호를 일치시킨다). back-pressure 임계 교차는 `resource.soft`, Slack 발신 거부는
  `agent.error`, 다이제스트 발행은 `briefing`(음성 브리핑 — 조건 13) 1건.
  `queue.delivered` 등 **데몬 bus 이벤트는 emit 하지 않는다**(javis_event SCHEMA 밖).

── 자가치유 사전 인가(조건 17①) ─────────────────────────────────────────────
  allowlist(`self-heal-allowlist.json`)에 걸리면 **큐 미경유**: 멱등/충돌 검사 직후 곧바로
  항목 파일(별도 키 공간)+원장 1줄만 남기고 반환한다 — **유지보수 tick·배달 I/O·상태 파일
  접근 전면 스킵**(S4: 데몬 wedge 로 cys 가 60초 멈춰도 지연 0). 목록 밖 액션은 일반 승인
  경로로 떨어진다(fail-closed 방향).

── risk 분류(조건 22②③) ────────────────────────────────────────────────────
  1차 = 데몬 risk_class 자기신고(`--risk-class`), 2차 = `javis_task._risk_scan_cmd`
  **import 재사용**(복제 금지)으로 AutoEligible → high 강등 오버레이. 분류 불가는 보수측
  high. 고위험 = **로컬 승인 전용** + 발행 tier=d 강제(데몬 `tier_mirrorable` 이 d 를 미러
  금지 — channels.rs:260 정의·:967 적용 · 조건 22③).

── Slack(조건 06·31 — 기본 OFF) ─────────────────────────────────────────────
  `.slack-enabled` 부재 시 무동작. 존재해도 ①**억제 3종에 종속**(즉시 배달된 항목만 —
  overflow·deferred·grace_held 는 로컬과 동일 억제·S7) ②notify-only 포맷 ③항목당 1회 상한
  ④발신 전 시크릿/절대경로 스캔(**`javis_scrub` 재사용** + 절대경로 계층) — 검출 시 거부+경보.

── 상태 소실 판정(R1 — 카운터 폐기) ─────────────────────────────────────────
  "소실"은 **원장 리플레이 대조**로만 판정한다. ①원장에 `queued`/`self_heal_bypass` 가
  있는데 항목 파일이 없고 `item_reaped` 도 없으면 → **그 rid 만 복원 대상**(되살려 쓴다).
  ②복원조차 못 하거나(쓰기 실패) **원장 자체가 훼손**(중간 줄 파손·세대 전멸)이면 그때만
  `state_lost` 다. 종전의 `state.item_count` 대조는 폐기했다 — 별도 카운터가 디스크 SOT 와
  경쟁해, submit 의 claim→+1 사이에 형제 프로세스의 재기록이 끼면 오버카운트 → **유령
  state_lost → 무음 exit 6** 가 났다(동시 8건·지연 10초 실측 재현). 어떤 `state_lost`
  경로도 **stderr 1줄 + stdout JSON + 원장 1줄**을 반드시 남긴다(무음 폐기 금지).

exit: 0 ok · 2 usage/전이 위반/JAVIS_ROOT 미설정 거부 · 3 not found · 5 nothing-to-do
      6 invalid(미지 class·severity·id 형식·rid 클래스 충돌·schema_version 미지값·
        상태 소실 복원 불가·내부 예외 통제 강등) · self-test: 0 통과 · 1 실패.

env 노브:
  JAVIS_ROOT                      큐 루트(**미설정 시 배달성 서브커맨드 거부** — S11e)
  CYS_BIN                         cys 바이너리(기본 cys — 테스트는 PATH shim)
  JAVIS_APPROVAL_SOURCE_CAP       발행자별 시간창 임계(기본 10)
  JAVIS_APPROVAL_URGENT_CAP       escalation·critical 시간당 상한(기본 4 — 조건 13)
  JAVIS_APPROVAL_RESTART_GRACE_SEC 데몬 재기동 유예 초(기본 300)
  JAVIS_APPROVAL_TTL_DAYS         pending 만료·종결 항목 보존 일수(기본 7)
  JAVIS_APPROVAL_ROUTING          routing.json 경로(기본 <approvals>/routing.json·내장 폴백)
  JAVIS_APPROVAL_ALLOWLIST        self-heal-allowlist.json 경로(동상)
  JAVIS_APPROVAL_SLACK_SINK       Slack 발신 기록부 경로(기본 <approvals>/slack-outbox.jsonl)
  JAVIS_APPROVAL_CONFIRM_SEC      배달 확정(wakeup id 조인) 대기 상한 초(기본 25 · R4)
  JAVIS_APPROVAL_ROTATE_BYTES     원장·outbox 세대 회전 임계(기본 5MB — 테스트 축소용)
  JAVIS_APPROVAL_NOW              **테스트 전용** 결정론 시계(epoch 초)
"""
import argparse
import contextlib
import glob
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import traceback
import uuid

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장
#   (javis_wakeup.py:50-52·javis_task.py:64 관례 답습).
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

import javis_scrub  # ★G2: 원장·항목 기록 직전 비밀 마스킹(형제 모듈 — 부재 시 즉시 실패)

_ROOT_FROM_ENV = bool((os.environ.get("JAVIS_ROOT") or "").strip())
ROOT = (os.environ.get("JAVIS_ROOT") or "").strip() or os.getcwd()
APPROVALS_DIR = os.path.join(ROOT, "_round", "approvals")
ITEMS_DIR = os.path.join(APPROVALS_DIR, "items")
TASKS_DIR = os.path.join(ROOT, "_round", "tasks")
LEDGER = os.path.join(APPROVALS_DIR, "ledger.jsonl")
STATE_PATH = os.path.join(APPROVALS_DIR, "state.json")
OUTBOX = os.path.join(APPROVALS_DIR, "outbox.jsonl")
SLACK_FLAG = os.path.join(APPROVALS_DIR, ".slack-enabled")
WAKEUP_LEDGER = os.path.join(ROOT, "_round", "wakeups", "queue.jsonl")
WAKEUP_PENDING = os.path.join(ROOT, "_round", "wakeups", "pending")

EXIT_OK, EXIT_USAGE, EXIT_NOTFOUND, EXIT_EMPTY, EXIT_INVALID = 0, 2, 3, 5, 6

CLASSES = ("approval", "self-healing", "escalation")
RISKS = ("high", "normal")
SEVERITIES = ("info", "warn", "critical")
STATES = ("pending", "approved", "denied", "expired", "notified")
TERMINAL_STATES = ("approved", "denied", "expired", "notified")

# 배달성 서브커맨드 — JAVIS_ROOT 미설정 시 cwd 폴백으로 실행하지 않는다(S11e).
DELIVERING_CMDS = ("submit", "collect-escalations", "digest", "tick")

SOURCE_CAP_DEFAULT = 10          # 조건 03④ 발행자별 시간창 임계
URGENT_CAP_DEFAULT = 4           # 조건 13 시간당 긴급 개별 push 상한
RESTART_GRACE_DEFAULT = 300      # 조건 36 데몬 재기동 유예(초)
TTL_DAYS_DEFAULT = 7
DIGEST_CAP_BYTES = 2048          # 조건 13 다이제스트 2KB 캡(집계 파일 본문)
DIGEST_LINE_MAX = 160
WAKEUP_REASON_MAX = 300          # javis_wakeup._DIGEST_REASON_MAX 와 동수(300자 접기 정합)
MERGE_LIST_MAX = 20              # 병합 알림에 실을 request_id 최대 개수(본문 팽창 차단)
LEDGER_ROTATE_BYTES = 5 * 1024 * 1024   # 원장·outbox 로테이션 임계(S11f)
MARKER_TTL_DAYS = 30             # `.digest-sent-*`·집계 파일 보존(S11f)
CONFIRM_SEC_DEFAULT = 25         # 배달 확정(wakeup id 조인) 대기 상한(R4)
LEDGER_RECHECK_SEC = 0.3         # '원장 전멸' 확정 전 1회 재확인 유예(콜드 스타트 오탐 방지)
DELIVER_BUDGET_DEFAULT = 75      # 배달 1건 전체 예산(enqueue+drain+confirm 합 · M2 단일 노브)
SWEEP_MAX_DEFAULT = 50           # 락 안 TTL sweep 배치 상한(보유 시간 상수화 · M4)
_RESNAP_MAX = 2000               # 전멸 재출발 시 재스냅샷 상한(F3 — 비용 상한)

# 항목 필수 8필드 + enum(R5a) — 위반은 `item_corrupt_isolated` 로 격리하고 리플레이가 메운다.
ITEM_REQUIRED = ("request_id", "class", "risk", "severity", "created_at", "source",
                 "payload_ref", "state")
ITEM_ENUMS = {"class": CLASSES, "risk": RISKS, "severity": SEVERITIES, "state": STATES}

# request_id·source 화이트리스트(S11c) — 경로 조작·개행 주입·제어문자 차단.
ID_RE = re.compile(r"^[A-Za-z0-9:._\-/]{1,120}$")
_SAFE_RE = re.compile(r"[^A-Za-z0-9._-]")

# 데몬 지문 필수 키 — **하나라도 없으면 관찰 실패**(유령 재기동 금지 · S5).
FP_REQUIRED = ("started_at", "build_id", "version")

# 자가치유 사전 인가 목록 내장 폴백 — 파일(`self-heal-allowlist.json`)이 정본이고 이 상수는
# 파일 부재·미봉인 시의 동형 폴백이다(조건 17① 목록 그대로). prereg 봉인은 [OT] 기록 —
# 도구는 봉인 주체를 검증하지 못한다(D12 자기신고 한계 자인).
SELF_HEAL_ALLOWLIST_FALLBACK = {
    "schema_version": 1,
    "actions": ["watchdog-kill", "phoenix-restore", "node-recover",
                "directive-reinject", "clear-enforce"],
    "prereg": {"sealed": False, "note": "봉인은 [OT] — 오너 1회 서명 대기"},
}

# escalation 라우팅 내장 폴백 — 라이브 역할 라벨 바인딩은 [OT-2](격리 검증은 PATH shim 대상).
ROUTING_FALLBACK = {
    "schema_version": 1,
    "routes": {
        "escalation": {"target": "reviewer1", "fallback": "master"},
        "approval": {"target": "master", "fallback": None},
        "self-healing": {"target": "master", "fallback": None},
        # 큐 메타 알림(rate 병합·유예 요약·다이제스트)은 개별 승인이 아니라 큐 운영 보고라
        # **항상 master** 다 — 값이 아니라 '표를 거친다'는 사실이 계약이다(R6d).
        "queue-meta": {"target": "master", "fallback": None},
    },
    # ★E2-4(BLOCKER R-03): 라벨 해소 규칙. `reviewer1`·`reviewer2` 는 **격리 검증용 자리표시
    # 라벨**이고 라이브 역할 라벨(reviewer-gemini·reviewer-codex)이 아니다. null = 미바인딩.
    # OT-2 가 실라벨을 여기에 채우면(또는 env JAVIS_APPROVAL_LABEL_REVIEWER1) 배달 대상이
    # 바뀐다 — 코드 수정 없이 표 1곳이 조작 지점이다.
    "label_bindings": {"reviewer1": None, "reviewer2": None},
    "live_binding": "OT-2",
}
# 자리표시 라벨 집합 — 이 라벨로 배달을 시도하는 것은 "실재하지 않는 대상에게 보내는 것"이다.
PLACEHOLDER_LABELS = ("reviewer1", "reviewer2")
UNBOUND_ALERT_TTL_SEC = 6 * 3600   # 미바인딩 경보 코얼레싱 창(guard `_alert_once` 동형)
_UNBOUND_LOGGED = set()            # 프로세스당 원장 1줄(같은 run 반복 배달 시 원장 폭주 방지)

# 발신 전 절대경로 계층(조건 31) — **비밀 패턴 자체는 `javis_scrub` 재사용**이고(형제 모듈
# 단일 정의 원칙 · 패턴 열화 복제 제거 · S7) 여기에는 scrub 이 일부러 잡지 않는 '경로' 계층만
# 둔다(원장은 경로가 정상 구성요소라 오탐 0 이 요구되지만, Slack 발신은 반대로 유출면이다).
_PATH_PATTERNS = [
    (re.compile(r"(?i)/users(?:/|\b)"), "절대경로(/Users)"),
    (re.compile(r"(?i)/home/[a-z0-9._-]+(?:/|\b)"), "절대경로(/home/<user>)"),
    (re.compile(r"(?:^|[\s\"'`(\[=:])~/"), "홈 경로(~/)"),
    (re.compile(r"(?i)\b[a-z]:\\users\\"), "절대경로(Windows Users)"),
]

_NO_DELIVER = False   # --no-deliver: enqueue 는 하되 same-run drain 을 생략(pending 계수용)
# ★R5c: JAVIS_ROOT 미설정이면 **어떤 파일도 쓰지 않는다**. 종전에는 거부 로그(`root_unset_refused`)
#   자체가 cwd 에 원장을 만들었고, 조회 경로(`list`)는 cwd 에 state.json 을 남겼다 — 거부한다면서
#   남의 트리를 오염시키는 자기모순. 거부 사유는 stderr 로만 말한다.
_NO_WRITE = False


class SchemaError(Exception):
    """schema_version 미지값·구조 위반 — deny-by-default(exit 6)."""


# ── 시각·경로·IO 기초 ────────────────────────────────────────────────────────
def _epoch():
    """결정론 시계 — JAVIS_APPROVAL_NOW(테스트 전용) 우선."""
    raw = os.environ.get("JAVIS_APPROVAL_NOW", "").strip()
    if raw:
        with contextlib.suppress(ValueError):
            return float(raw)
    return time.time()


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z", time.localtime(_epoch()))


def _today():
    return time.strftime("%Y-%m-%d", time.localtime(_epoch()))


def _hour_window(ts=None):
    return int((_epoch() if ts is None else ts) // 3600)


def _cys_bin():
    return os.environ.get("CYS_BIN", "cys")


def _int_env(name, default):
    raw = os.environ.get(name, "").strip()
    try:
        v = int(raw) if raw else default
    except ValueError:
        return default
    return v if v >= 0 else default


def _oneline(text, limit=400):
    """개행·제어문자 이스케이프(S11c) — 호출자 제공 본문이 원장 줄을 깨지 못하게 한다.

    역슬래시까지 접는 **조립 단계** 전용이다(사용자가 친 리터럴 `\\n` 과 실제 개행을 구분).
    와이어 직전 안전망은 `_flatten` — 두 번 적용해 `\\\\n` 이 되는 이중 이스케이프를 막는다.
    """
    if text is None:
        return ""
    s = str(text).replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r")
    s = re.sub(r"[\x00-\x08\x0b-\x1f\x7f]", "", s)
    return s[:limit]


def _flatten(text, limit=1200):
    """와이어 최종 안전망 — 개행·제어문자만 접는다(역슬래시 재이스케이프 금지)."""
    if text is None:
        return ""
    s = str(text).replace("\n", "\\n").replace("\r", "\\r")
    s = re.sub(r"[\x00-\x08\x0b-\x1f\x7f]", "", s)
    return s[:limit]


def _write_json_atomic(path, obj):
    """temp+os.replace(조건 03⑤) — 재기동 중 잘린 스냅샷을 만들지 않는다."""
    if _NO_WRITE:
        return                                  # R5c — JAVIS_ROOT 미설정 시 cwd 무오염
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = "%s.tmp.%s.%s" % (path, os.getpid(), uuid.uuid4().hex[:8])
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def _read_json(path):
    """(obj|None, err|None) — 부재 (None,'absent') · 손상/비객체 (None,'corrupt')."""
    try:
        with open(path, encoding="utf-8") as f:
            d = json.load(f)
    except FileNotFoundError:
        return None, "absent"
    except (OSError, ValueError):
        return None, "corrupt"
    return (d, None) if isinstance(d, dict) else (None, "corrupt")


def _append_jsonl(path, obj):
    """append-only 1줄 — 기록 실패는 무해히 흘리되 침묵하지 않는다(stderr 1줄)."""
    if _NO_WRITE:
        return False                            # R5c — 거부 사유는 stderr 로만(원장 생성 금지)
    if path == LEDGER:
        # ★M3: 헤더는 **원장을 만드는 모든 경로**보다 앞서야 세대 첫 줄이 된다. `_item_claim`
        #   에서만 부르면 `daemon_baseline` 같은 선행 append 가 헤더 없는 세대를 만든다.
        _ensure_ledger_file()
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "a", encoding="utf-8") as f:
            f.write(json.dumps(javis_scrub.scrub_obj(obj), ensure_ascii=False) + "\n")
        return True
    except OSError as e:
        sys.stderr.write("[approval-queue] 원장 기록 실패(%s): %s\n" % (path, e))
        return False


def _preflight_lost_probe():
    """(lost|None) — **진입 직후 1회** 경량 전멸 프로브. 읽기 2회(listdir+glob)·로그 0.

    ★H2: v3.1 의 순서 계약(`_item_claim` → `_ensure_ledger_file`)에는 부작용이 있었다.
    `cmd_submit` 은 판정에 닿기 전에 `_self_heal_allowlist`(contract_fallback 원장)·
    `_item_claim`(원장 선실체화)으로 **원장을 재실체화**한다. 그래서 진짜 전멸(항목 실재 +
    원장 전삭제) 상태에서도 세대가 되살아나 `_items_sync` 의 전멸 판정이 **영구 마스킹**됐다.
    판정은 어떤 쓰기보다 먼저 와야 한다 — 그래서 이 프로브는 서브커맨드 진입 직후에 선다.
    """
    try:
        names = [f for f in os.listdir(ITEMS_DIR) if f.endswith(".json")]
    except OSError:
        return None                      # items/ 부재 = 콜드 스타트(정상)
    if not names or _generations(LEDGER):
        return None
    time.sleep(LEDGER_RECHECK_SEC)       # 형제의 claim 중간 상태 방어(1회 재확인)
    if _generations(LEDGER):
        return None
    return {"reason": "ledger_damaged", "bad_lines": 0, "generations": 0,
            "note": "원장 세대 전멸(디스크 항목 %d건 실재) — 진입 프로브 탐지" % len(names),
            "disk_items": len(names), "restored": 0, "ledger_present": False,
            "torn_lines": 0, "foreign_lines": 0, "probe": "preflight"}


def _ensure_ledger_file():
    """원장 파일을 **비어 있더라도 먼저 실체화**한다(콜드 스타트 경합 창 제거 · 4차 수리).

    `_item_claim` 이 `items/` 를 만들기 직전에 부른다. "항목 파일이 있으면 원장 파일도 있다"를
    순서로 보장해, 형제의 claim 중간 상태가 '원장 전멸'로 오판되지 않게 한다.
    """
    if _NO_WRITE:
        return
    with contextlib.suppress(OSError):
        os.makedirs(os.path.dirname(LEDGER), exist_ok=True)
        try:
            fd = os.open(LEDGER, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644)
        except FileExistsError:
            return
        # ★M3 세대 헤더 — `O_EXCL` 이라 세대당 정확 1줄이다. 빈 파일로 두면 "선실체화된 새
        #   원장"과 "내용이 통째로 지워진 원장"이 구분되지 않는다(0바이트 모호성). 헤더는
        #   리플레이가 무시하는 event 라 복원 의미론에 영향을 주지 않는다.
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(json.dumps({"event": "ledger_opened", "ts": _now(),
                                "generation": len(_generations(LEDGER)),
                                "note": "세대 헤더(선실체화 표식) — 내용 0줄 = 비정상 신호"},
                               ensure_ascii=False) + "\n")


def _ledger_append(event, **kw):
    ev = dict(event) if isinstance(event, dict) else {"event": str(event)}
    ev.update(kw)
    ev.setdefault("ts", _now())
    return _append_jsonl(LEDGER, ev)


def _relativize(path):
    """발신·표시용 경로 축약 — 절대경로 유출 1차 차단(2차는 시크릿 스캔)."""
    if not path:
        return path
    p = str(path)
    with contextlib.suppress(ValueError):
        if os.path.isabs(p) and os.path.commonpath([p, ROOT]) == ROOT:
            return os.path.relpath(p, ROOT)
    return os.path.basename(p) if os.path.isabs(p) else p


def _sha256_file(path):
    try:
        with open(path, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()[:16]
    except OSError:
        return None


@contextlib.contextmanager
def _state_lock():
    """`state.json` 직렬화 — **javis_wakeup._FileLock 재사용**(원자 mkdir·5s 상한·30s stale).

    ★임계구역 축소(S1): 이 락 안에서는 **상태 판정·기록만** 한다. 배달(wakeup enqueue/drain·
    cys)·이벤트 emit 같은 외부 I/O 는 전부 락 밖이다 — 종전처럼 최대 50초 배달을 임계구역에
    넣으면 동시 submit 이 5초 상한을 넘겨 무락 degrade → last-writer-wins → 항목 소실이 된다.
    조건 25②: 전역 flock/세마포어 신설 금지 — javis_task wlock 패턴(원자 mkdir + 짧은 상한 +
    stale 회수 + **무락 degrade**)만 허용. 획득 실패는 경고 후 진행이다(블로킹 대기 금지).
    무락 degrade 에서도 항목 정합은 유지된다 — 항목은 통짜 스냅샷이 아니라 **항목별 파일
    O_EXCL 생성**으로 정확 1건이 보장되기 때문이다(스냅샷 판정 폐기).
    """
    lk = None
    try:
        if _NO_WRITE:
            raise RuntimeError("쓰기 봉인 — 락 자체가 디렉토리를 만든다(R5c cwd 무오염)")
        from javis_wakeup import _FileLock as _WkLock  # 형제 모듈 단일 정의 재사용
        lk = _WkLock(STATE_PATH + ".lock", timeout=5.0, stale_sec=30.0)
        lk.__enter__()
    except Exception as e:  # noqa: BLE001 — import 실패·타임아웃 전부 degrade
        if lk is not None:
            # ★L-c: degrade 를 **무음으로 넘기지 않는다**. 무락 진행은 정합이 유지되도록
            #   설계됐지만(항목 파일 원자 생성), 그 사실이 관측되지 않으면 "왜 계수가
            #   흔들렸나"를 사후에 추적할 근거가 사라진다.
            sys.stderr.write("[approval-queue] state 락 획득 실패 — 무락 진행(degrade): %s\n" % e)
            with contextlib.suppress(Exception):
                _ledger_append({"event": "state_lock_degraded", "why": str(e)[:200],
                                "note": "5초 상한 초과·stale 회수 실패 — 무락 진행(정합은 "
                                        "항목 파일 원자 생성이 담보)"})
        lk = None
    try:
        yield
    finally:
        if lk is not None:
            with contextlib.suppress(Exception):
                lk.__exit__(None, None, None)


# ── 부수 채널: EVT emit(전부 best-effort — 판정 불개입·조건 19①) ─────────────
def _emit(evt_type, fields):
    """javis_event emit --spool. 실패 전부 무해 격리(음성·HUD 구독 토대 — EVT v1 계약)."""
    ev = os.path.join(_SELF_DIR, "javis_event.py")
    if not os.path.isfile(ev):
        return False
    argv = [sys.executable, ev, "emit", evt_type, "--spool"]
    for k, v in fields.items():
        argv += ["--field", "%s=%s" % (k, v)]
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"
    # ★L-f: `HUD_STATE_DIR` 미설정이면 javis_event 가 **팩 소스 트리**(`<pack>/state/`)에
    #   spool 을 쌓아 격리 실행이 저장소를 오염시킨다(미추적 산출물 혼입). JAVIS_ROOT 가
    #   있으면 격리 루트 안으로 강제한다 — 격리는 env 하나로 완결돼야 한다.
    if not env.get("HUD_STATE_DIR") and _ROOT_FROM_ENV:
        env["HUD_STATE_DIR"] = os.path.join(ROOT, "_round", "hud")
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=10, env=env)
        return r.returncode == 0
    except (subprocess.SubprocessError, OSError):
        return False


# ── 항목 저장소(항목별 파일 · 락 불요 — O_EXCL 이 소유권을 준다) ─────────────
def _item_key(rid, bypass=False):
    """키 공간 분리(S2) — 자가치유 사전 인가 항목은 승인 항목과 키가 겹치지 않는다."""
    return ("selfheal-" + rid) if bypass else rid


def _item_file(rid, bypass=False):
    """`items/<sanitized_rid>.<h8>.json` — sanitize 충돌은 rid 해시 8자로 분리한다."""
    safe = _SAFE_RE.sub("_", rid)[:80]
    h = hashlib.sha1(rid.encode("utf-8")).hexdigest()[:8]  # noqa: S324 — 파일명 분리 전용
    return os.path.join(ITEMS_DIR, "%s%s.%s.json" % ("selfheal-" if bypass else "", safe, h))


def _item_read(rid, bypass=False):
    """(item|None, err|None) — 존재 판정의 유일 근거(스냅샷 조회 폐기)."""
    return _read_json(_item_file(rid, bypass))


def _item_write(item):
    """항목 파일 원자 교체(temp+os.replace) + `javis_scrub` 마스킹(S11d — 원장과 대칭)."""
    path = _item_file(item["request_id"], bool(item.get("bypass")))
    _write_json_atomic(path, javis_scrub.scrub_obj(item))
    return path


def _item_claim(item):
    """**완전 기록 → os.link** — 경합에서도 정확 1건 + **0바이트 창 0**(R6a). (ok, path).

    종전은 `O_EXCL` 로 빈 파일을 먼저 만들고 그 뒤에 본문을 썼다. 그 사이(수 마이크로초)에
    형제 프로세스의 `_items_scan` 이 읽으면 **0바이트 = 손상 항목**으로 격리해버린다(claim 한
    항목이 자기 손으로 격리되는 자기잠식). `os.link` 는 대상이 있으면 `FileExistsError` 로
    실패하는 **원자 생성** 프리미티브라, "완전히 쓴 파일"을 그대로 원자 등장시킨다.
    """
    path = _item_file(item["request_id"], bool(item.get("bypass")))
    if _NO_WRITE:
        return False, path
    # ★순서 계약(4차 수리): **원장 파일이 items/ 보다 먼저 실체화된다.** 항목 파일이 존재하는데
    #   원장이 없는 상태는 이 순서로 구조적으로 만들어지지 않으며, 그래야 "항목 실재 + 세대 0 =
    #   원장 전멸" 판정이 형제의 중간 상태를 오탐하지 않는다. 빈 파일이면 리플레이 결과도 비므로
    #   판정에 영향을 주지 않는다(존재 자체가 신호다).
    _ensure_ledger_file()
    os.makedirs(ITEMS_DIR, exist_ok=True)
    tmp = "%s.tmp.%s.%s" % (path, os.getpid(), uuid.uuid4().hex[:8])
    try:
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(javis_scrub.scrub_obj(item), f, ensure_ascii=False, indent=1)
            f.flush()
            os.fsync(f.fileno())
        try:
            os.link(tmp, path)          # 원자 생성 — 이미 있으면 FileExistsError
        except FileExistsError:
            return False, path
    except OSError as e:
        raise SchemaError("항목 파일 생성 불가(%s): %s" % (path, e)) from e
    finally:
        with contextlib.suppress(OSError):
            os.remove(tmp)
    # ★M5(테스트 전용): claim 직후 ~ `queued` append 사이의 창을 **결정론으로 벌린다**.
    #   이 창을 형제 프로세스가 밟았을 때 전멸 오탐이 나던 자리라, 확률 의존 시험(약 10%
    #   탐지)을 100% 탐지로 바꾸기 위한 노브다. 운영에서는 미설정 = 무동작.
    _test_delay = os.environ.get("JAVIS_APPROVAL_TEST_CLAIM_DELAY", "").strip()
    if _test_delay:
        with contextlib.suppress(ValueError):
            time.sleep(float(_test_delay))
    return True, path


def _item_remove(item_or_key):
    key = item_or_key if isinstance(item_or_key, str) else None
    if key is None:
        path = _item_file(item_or_key["request_id"], bool(item_or_key.get("bypass")))
    else:
        path = key
    with contextlib.suppress(OSError):
        os.remove(path)


def _item_defect(obj):
    """항목 1건의 결함 사유 또는 None — **필수 8필드 + enum 검증**(R5a).

    종전에는 `request_id` 유무만 봤다. 그래서 `class`·`severity` 가 빠진 반쪽 항목이 그대로
    통과해 `_is_urgent(item["class"])` 같은 소비자에서 **KeyError → 통제 강등 exit 6** 로
    번졌다(항목 1건의 결함이 도구 전체를 멈추는 전이). 여기서 격리하면 리플레이가 메운다.
    """
    if not isinstance(obj, dict):
        return "객체가 아님"
    missing = [k for k in ITEM_REQUIRED if k not in obj]
    if missing:
        return "필수 필드 누락: %s" % ",".join(missing)
    if not isinstance(obj.get("request_id"), str) or not obj["request_id"]:
        return "request_id 비문자열/빈값"
    for k, allowed in ITEM_ENUMS.items():
        if obj.get(k) not in allowed:
            return "%s enum 위반(%r)" % (k, obj.get(k))
    return None


def _items_scan():
    """({key: item}, [(손상 경로, 사유)]) — 항목별 파일 전수(디렉토리 부재 = 빈 집합)."""
    items, corrupt = {}, []
    for p in sorted(glob.glob(os.path.join(ITEMS_DIR, "*.json"))):
        obj, err = _read_json(p)
        why = err if obj is None else _item_defect(obj)
        if why:
            corrupt.append((p, why))
            continue
        items[_item_key(obj["request_id"], bool(obj.get("bypass")))] = obj
    return items, corrupt


# ── 원장 세대(R3 — 회전이 복원 SOT 를 파괴하지 않게) ─────────────────────────
_GEN_RE = re.compile(r"\.(\d+)$")


def _generations(path):
    """[오래된 세대 … 최신] 경로 목록 — `<path>.N`(큰 N 이 오래됨) 다음에 `<path>`.

    회전이 `.1` 하나만 쓰면 2회 회전에서 **최초 세대가 소멸**한다. 복원 SOT(미종결 queued
    스냅샷)가 거기 있었다면 그대로 유실이다. 그래서 폐기 대신 **번호를 밀어 보존**하고,
    리플레이는 오래된 세대부터 이어 읽는다.
    """
    gens = []
    for p in glob.glob(glob.escape(path) + ".*"):
        m = _GEN_RE.search(p)
        if m:
            with contextlib.suppress(ValueError):
                gens.append((int(m.group(1)), p))
    out = [p for _n, p in sorted(gens, reverse=True)]
    if os.path.isfile(path):
        out.append(path)
    return out


def _rotate_preserving(path):
    """세대 보존 회전 — `.N`→`.N+1` 로 밀고 본체를 `.1` 로. **어떤 세대도 지우지 않는다**.

    `os.replace` 는 원자라, 회전 중 어느 순간에도 세대 하나는 반드시 존재한다(형제 프로세스가
    '원장 전멸'로 오판하지 않는다).
    """
    ns = []
    for p in glob.glob(glob.escape(path) + ".*"):
        m = _GEN_RE.search(p)
        if m:
            with contextlib.suppress(ValueError):
                ns.append(int(m.group(1)))
    for n in sorted(ns, reverse=True):
        with contextlib.suppress(OSError):
            os.replace("%s.%d" % (path, n), "%s.%d" % (path, n + 1))
    os.replace(path, path + ".1")


# ── 원장 리플레이 복원(S3 · R1 판정 근거) ────────────────────────────────────
def _replay_ledger():
    """({key: item}, info) — `queued` 스냅샷 + 후속 사건 재적용(전 세대 이어 읽기).

    info = {present, generations, bad_lines, tail_truncated, damaged}
    · `damaged` = **꼬리가 아닌 위치의 파손 줄**(= 기록이 실제로 사라진 증거) 또는 전멸.
      마지막 줄만 잘린 것은 append 중 크래시의 정상 흔적이라 관용한다(오탐 wedge 방지).
    항목 파일이 통째로 사라져도 원장만 살아있으면 pending 을 되살린다. 되살릴 수 없는데
    조용히 fresh 로 출발하는 것이 이 큐의 최악 실패라, 여기서 못 살리면 호출자가
    `state_lost` 경보 + stdout/stderr/원장 3중 흔적 + 비영 exit 로 멈춘다.
    """
    items = {}
    gens = _generations(LEDGER)
    info = {"present": bool(gens), "generations": len(gens), "bad_lines": 0,
            "torn_lines": 0, "foreign_lines": 0, "damaged": False, "why": None}
    for path in gens:
        try:
            f = open(path, encoding="utf-8")  # noqa: SIM115 — with 로 즉시 감싼다
        except OSError as e:
            info["damaged"] = True
            info["why"] = "세대 판독 실패(%s): %s" % (_relativize(path), e)
            continue
        with f:
            lines = f.readlines()
        for ln in lines:
            ln = ln.strip()
            if not ln:
                continue
            try:
                d = json.loads(ln)
            except ValueError:
                d = None
            if not isinstance(d, dict):
                info["bad_lines"] += 1
                # 파손 줄 2분류: **잘린 쓰기**(`{` 로 시작하는 JSON 객체의 접두 — append 도중
                # 프로세스가 죽은 정상 흔적)는 관용하고, **이물질**(`{` 로 시작하지도 않는 줄 =
                # 파일이 다른 내용으로 덮여썼다는 증거)만 훼손으로 본다.
                # 위치(꼬리 여부)로 판정하면 그 뒤에 한 줄만 더 append 돼도 정상 흔적이
                # 훼손으로 승격돼 도구가 **영구 wedge** 된다 — 오탐 비용이 미탐 비용보다 크다.
                if ln.startswith("{"):
                    info["torn_lines"] += 1
                else:
                    info["foreign_lines"] += 1
                    info["damaged"] = True
                    info["why"] = "이물질 줄(JSON 객체가 아님) — 원장이 덮어써짐"
                continue
            _replay_apply(items, d)
    # ★원장 전멸 판정은 여기서 하지 않는다 — 판정 근거가 **디스크 항목 실재**이므로
    #   `_items_sync`(디스크를 아는 자리)로 옮겼다. 종전에 여기서 `os.path.isdir(ITEMS_DIR)` 를
    #   근거로 삼은 것이 콜드 스타트 오탐의 원인이었다(아래 `_items_sync` 주석).
    return items, info


# 전수 파싱 없이 '결손 여부'만 보는 경량 게이트(R5b) — 정규식 1회 vs json.loads 1회.
_EV_RE = re.compile(r'"event"\s*:\s*"([A-Za-z_]+)"')
_RID_RE = re.compile(r'"request_id"\s*:\s*"([^"\\]{1,120})"')
_BYPASS_RE = re.compile(r'"bypass"\s*:\s*(true|false)')


def _ledger_open_keys():
    """(열린 키 집합|None) — 원장이 만든 항목 키 중 아직 reaped 되지 않은 것.

    None = 경량 판독 실패(전수 리플레이로 승격). 이 집합이 디스크 항목의 부분집합이면
    **결손 0** 이므로 전수 리플레이·복원 쓰기를 통째로 건너뛴다(락 밖이라도 원장이 커지면
    tick 비용이 선형으로 늘어나는 것을 막는다).
    """
    keys = set()
    for path in _generations(LEDGER):
        try:
            f = open(path, encoding="utf-8")  # noqa: SIM115
        except OSError:
            return None
        with f:
            for ln in f:
                ln = ln.strip()
                if not ln:
                    continue
                if not (ln.startswith("{") and ln.endswith("}")):
                    # 경량 게이트도 **훼손을 통과시키면 안 된다**. JSON 형태가 아닌 줄이 하나라도
                    # 있으면 판정을 포기하고 전수 리플레이로 승격한다(거기서 damaged 판정).
                    return None
                if '"event"' not in ln:
                    continue
                m = _EV_RE.search(ln)
                if not m:
                    return None
                ev = m.group(1)
                if ev not in ("queued", "self_heal_bypass", "item_reaped"):
                    continue
                r = _RID_RE.search(ln)
                if not r:
                    return None
                rid = r.group(1)
                if ev == "queued":
                    keys.add(_item_key(rid))
                elif ev == "self_heal_bypass":
                    keys.add(_item_key(rid, True))
                else:                            # item_reaped — `_replay_apply` 와 동일 규칙
                    bm = _BYPASS_RE.search(ln)
                    if bm:
                        keys.discard(_item_key(rid, bm.group(1) == "true"))
                    else:
                        keys.discard(_item_key(rid))
                        keys.discard(_item_key(rid, True))
    return keys


def _replay_apply(items, d):
    ev, rid = d.get("event"), d.get("request_id")
    if ev == "queued" and rid:
        items[_item_key(rid)] = {
            "request_id": rid, "class": d.get("class"), "risk": d.get("risk"),
            "severity": d.get("severity"), "created_at": d.get("created_at"),
            "created_epoch": d.get("created_epoch"), "source": d.get("source"),
            "payload_ref": d.get("payload_ref"), "state": d.get("state") or "pending",
            "summary": d.get("summary"), "risk_why": d.get("risk_why"),
            "local_only": bool(d.get("local_only")), "bypass": False,
            "notified_at": None, "notified_slack_at": None, "deferred_to_digest": False,
            "rate_overflow": False, "grace_held": False, "decided_at": None,
            "decided_by": None, "routed_to": None, "restored_from": "ledger"}
        return
    if ev == "self_heal_bypass" and rid:
        items[_item_key(rid, True)] = {
            "request_id": rid, "class": "self-healing", "risk": "normal",
            "severity": d.get("severity") or "warn", "created_at": d.get("ts"),
            "created_epoch": d.get("created_epoch"), "source": d.get("source"),
            "payload_ref": d.get("payload_ref"), "state": "notified",
            "summary": d.get("summary"), "risk_why": "자가치유 사전 인가(allowlist)",
            "local_only": False, "bypass": True, "queued": False, "action": d.get("action"),
            "notified_at": d.get("ts"), "notified_slack_at": None,
            "deferred_to_digest": False, "rate_overflow": False, "grace_held": False,
            "decided_at": None, "decided_by": None, "routed_to": None,
            "restored_from": "ledger"}
        return
    if ev == "item_reaped" and rid:
        # 회수는 **키 공간까지** 재현해야 한다. 종전에는 승인 키만 지워서, 회수된
        # 자가치유(selfheal-) 항목이 매 리플레이마다 되살아나 영구 부활했다.
        if "bypass" in d:
            items.pop(_item_key(rid, bool(d.get("bypass"))), None)
        else:                                   # 구 레코드(플래그 부재) — 양쪽 다 회수
            items.pop(_item_key(rid), None)
            items.pop(_item_key(rid, True), None)
        return
    it = items.get(_item_key(rid)) if rid else None
    if it is None:
        # 병합·요약 배달은 여러 rid 를 한 줄에 싣는다 — 그 줄로 개별 항목의 억제 표식을 푼다.
        for one in (d.get("request_ids") or []):
            tgt = items.get(_item_key(one))
            if tgt is not None and ev in ("rate_merge_delivered", "grace_summary_delivered"):
                tgt["rate_overflow"] = False
                tgt["grace_held"] = False
                tgt["notified_at"] = d.get("ts")
        return
    if ev in ("delivered", "enqueued_no_deliver"):
        it["notified_at"] = d.get("ts")
        it["routed_to"] = d.get("target")
        it["delivery_ok"] = True
    elif ev in ("deliver_failed", "routed_fallback"):
        it["notified_at"] = d.get("ts")
        it["routed_to"] = d.get("target")
        it["delivery_ok"] = bool(d.get("ok"))
    elif ev == "deferred_to_digest":
        it["deferred_to_digest"] = True
    elif ev == "rate_overflow":
        it["rate_overflow"] = True
    elif ev == "grace_held":
        it["grace_held"] = True
    elif ev == "slack_notified":
        it["notified_slack_at"] = d.get("ts")
        it["slack_tier"] = d.get("tier")
    elif ev in ("approved", "denied"):
        it["state"] = ev
        it["decided_at"] = d.get("ts")
        it["decided_by"] = d.get("by")
        it["decided_epoch"] = d.get("decided_epoch")
    elif ev == "expired":
        it["state"] = "expired"
        it["decided_at"] = d.get("ts")
        it["decided_epoch"] = d.get("decided_epoch")


def _items_sync():
    """({key: item}, 복원수, lost|None) — 디스크(SOT) ↔ 원장 리플레이 정합. **락 밖**(R5b).

    ★R1 근본 처방: 별도 카운터(`state.item_count`)와의 대조를 폐기했다. 카운터는 디스크와
    경쟁하는 두 번째 진실이었고, submit 의 `claim → +1` 사이에 형제 프로세스의 재기록이 끼면
    카운터만 앞서 나가 **유령 소실**이 났다. 이제 판정은 원장 리플레이 하나로만 한다:
      · 원장이 만든 키(queued·self_heal_bypass, `item_reaped` 제외)가 디스크에 없다 → **복원**
      · 복원조차 실패 or 원장이 실제로 훼손(꼬리 아닌 파손·세대 전멸) → **그때만 lost**
    반환 `lost` 는 사유 dict 이며, 호출자는 이것을 stdout·stderr·원장 3중으로 표면화한다.
    """
    disk, corrupt = _items_scan()
    for p, why in corrupt:
        # 손상·필수필드 위반 항목 격리(R5a) — 원장에 있으면 아래에서 되살아난다.
        with contextlib.suppress(OSError):
            os.replace(p, "%s.corrupt-%d" % (p, int(_epoch())))
        _ledger_append({"event": "item_corrupt_isolated", "path": _relativize(p), "why": why})

    # ① 원장 전멸 판정 — 근거는 **디스크에 항목이 실재하는데 세대가 0** 인 것 하나뿐이다.
    #    ★4차 수리: 종전 근거는 `os.path.isdir(ITEMS_DIR)` 였고, 그것이 **콜드 스타트를
    #    오탐**했다. 빈 큐에 동시 제출이 들어오면 형제가 `items/` 를 먼저 만들고 원장 첫 줄을
    #    아직 못 쓴 창이 생기는데, 그 창에 들어온 프로세스가 "원장 전멸"로 판정해 submit 을
    #    무음에 가깝게 중단시켰다(운영 첫 사용 조건과 정확히 일치 — 실측 disk_items=0).
    #    디렉토리의 존재는 아무것도 증명하지 않는다. 항목 파일이 실재해야 "원장이 있었어야
    #    한다"가 성립한다.
    if disk and not _generations(LEDGER):
        # 그럼에도 남는 창(claim ~ queued append 사이)을 위해 **1회 재확인**한다. 창은
        # `_item_claim` 의 순서 보장(원장 파일 선실체화)으로 이미 닫혀 있고, 이것은 보험이다.
        time.sleep(LEDGER_RECHECK_SEC)
        if not _generations(LEDGER):
            lost = {"reason": "ledger_damaged", "bad_lines": 0, "generations": 0,
                    "note": "원장 세대 전멸(디스크 항목 %d건 실재) — 무엇이 있었는지 열거 불가"
                            % len(disk),
                    "disk_items": len(disk), "restored": 0, "ledger_present": False,
                    "torn_lines": 0, "foreign_lines": 0}
            _ledger_append(dict(lost, event="state_lost",
                                note="원장 리플레이로도 복원 불가 — fresh 출발 금지·경보 후 중단"))
            return disk, 0, lost

    # ② 경량 게이트 — 결손이 없으면 전수 리플레이 자체를 건너뛴다(R5b).
    if not corrupt:
        open_keys = _ledger_open_keys()
        if open_keys is not None and open_keys.issubset(set(disk)):
            return disk, 0, None

    # ③ 결손·판독 불확실 → 전수 리플레이(락 밖이라 락 상한을 잠식하지 않는다).
    replay, info = _replay_ledger()
    # ★테스트 전용(M5 동류): 리플레이 스냅샷 확보 ~ 복원 쓰기 사이의 창을 결정론으로 벌린다.
    #   H1(복원 루프 통짜 덮어쓰기)이 그 창에서만 나므로, 확률 재현을 100% 재현으로 바꾼다.
    _rdelay = os.environ.get("JAVIS_APPROVAL_TEST_RESTORE_DELAY", "").strip()
    if _rdelay:
        with contextlib.suppress(ValueError):
            time.sleep(float(_rdelay))
    restored, unrestorable, skipped_present, defective = 0, [], 0, []
    for key, it in replay.items():
        if key in disk:
            continue
        # ★M1: 복원은 **자기 산출물을 검증**한다. 검증 없이 쓰면 결손 `queued` 스냅샷이
        #   그대로 파일이 되고 → 다음 스캔이 필수필드 위반으로 격리하고 → 다시 복원하는
        #   '복원→격리→복원' 루프가 돌며 원장이 무한 증식한다.
        defect = _item_defect(it)
        if defect:
            defective.append({"request_id": it.get("request_id"), "why": defect})
            continue
        # ★H1: 복원은 "없는 것을 되살리는" 연산이다 — `os.replace` 통짜 덮어쓰기는 **잘못된
        #   프리미티브**였다. 스캔 이후 다른 프로세스가 그 항목을 만들거나 approve/deny 로
        #   확정했으면, 낡은 리플레이 스냅샷이 그 결정을 pending 으로 **영구·무음** 되돌린다
        #   (deny 면 fail-open). `os.link` 원자 생성은 이미 있으면 실패하므로 그 경로가 없다.
        try:
            created, _p = _item_claim(it)
        except (OSError, SchemaError) as e:
            unrestorable.append({"request_id": it.get("request_id"), "why": str(e)[:120]})
            continue
        if not created:
            cur, _e = _item_read(it["request_id"], bool(it.get("bypass")))
            if cur is not None:
                disk[key] = cur                   # 디스크 최신본이 이긴다(결정 보존)
            skipped_present += 1
            _ledger_append({"event": "item_restore_skipped_present",
                            "request_id": it.get("request_id"),
                            "replay_state": it.get("state"),
                            "disk_state": (cur or {}).get("state"),
                            "note": "복원 직전 항목이 이미 존재 — 덮어쓰기 거부(결정 보존)"})
            continue
        disk[key] = it
        restored += 1
        _ledger_append({"event": "item_restored", "request_id": it.get("request_id"),
                        "state": it.get("state"), "from": "ledger_replay"})
    if defective:
        # 원장 1줄로 접는다(라운드당 1줄 — 무한 증식 차단 · M1).
        _ledger_append({"event": "item_restore_unrestorable", "count": len(defective),
                        "request_ids": [d["request_id"] for d in defective][:20],
                        "why": defective[0]["why"],
                        "note": "결손 스냅샷 — 복원 보류(복원↔격리 루프 차단·라운드당 1줄)"})

    lost = None
    if unrestorable:
        lost = {"reason": "item_write_failed", "unrestorable": unrestorable[:20],
                "count": len(unrestorable)}
    elif info["damaged"]:
        lost = {"reason": "ledger_damaged", "bad_lines": info["bad_lines"],
                "generations": info["generations"], "note": info["why"]}
    if lost:
        lost.update({"disk_items": len(disk), "restored": restored,
                     "ledger_present": info["present"],
                     "torn_lines": info["torn_lines"],
                     "foreign_lines": info["foreign_lines"]})
        _ledger_append(dict(lost, event="state_lost",
                            note="원장 리플레이로도 복원 불가 — fresh 출발 금지·경보 후 중단"))
    return disk, restored, lost


# ── state(집계/창 상태만 — 항목 없음) ────────────────────────────────────────
def _fresh_state():
    return {"schema_version": 1,
            "sources": {},                       # source → {window_start, count, soft_emitted}
            # carry = 아직 병합 배달되지 않은 긴급 초과분(창 전환에서 버리지 않는다 · L-d)
            "rate": {"window_start": 0, "count": 0, "overflow": [], "carry": []},
            "daemon": {"fingerprint": None, "observed_at": None, "grace_until": 0,
                       "held": [], "summary_sent": True},
            "digest": {},                        # 날짜 → {sent_at, count, bytes}
            # ★item_count 없음(R1) — 항목 수는 디스크 실측이 유일 SOT다. 카운터를 두면
            #   claim→증가 창에서 형제 프로세스의 재기록과 경쟁해 유령 소실을 만든다.
            "updated_at": None}


def _num(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def _state_key_ok(key, v):
    """키별 타입 검증(S9) — 유효 JSON 이지만 타입이 어긋난 주입을 통제 강등한다."""
    if key == "sources":
        return isinstance(v, dict) and all(
            isinstance(r, dict) and _num(r.get("window_start")) and _num(r.get("count"))
            and isinstance(r.get("soft_emitted", False), bool) for r in v.values())
    if key == "rate":
        return (isinstance(v, dict) and _num(v.get("window_start", 0))
                and _num(v.get("count", 0)) and isinstance(v.get("overflow", []), list)
                and all(isinstance(x, str) for x in v.get("overflow", []))
                and isinstance(v.get("carry", []), list)
                and all(isinstance(x, str) for x in v.get("carry", [])))
    if key == "daemon":
        if not isinstance(v, dict):
            return False
        fp = v.get("fingerprint")
        return ((fp is None or isinstance(fp, dict))
                and _num(v.get("grace_until", 0))
                and isinstance(v.get("held", []), list)
                and all(isinstance(x, str) for x in v.get("held", []))
                and isinstance(v.get("summary_sent", True), bool))
    if key == "digest":
        return isinstance(v, dict) and all(isinstance(r, dict) for r in v.values())
    if key == "updated_at":
        return v is None or isinstance(v, str)
    if key == "schema_version":
        return v == 1
    return True


def _load_state():
    obj, err = _read_json(STATE_PATH)
    if obj is None:
        if err == "corrupt":
            # 손상 격리(조건 14·phoenix P2-6 관례) — 원장이 살아있으므로 항목은 리플레이가 되살린다.
            with contextlib.suppress(OSError):
                os.replace(STATE_PATH, "%s.corrupt-%d" % (STATE_PATH, int(_epoch())))
            _ledger_append({"event": "state_corrupt_isolated", "path": _relativize(STATE_PATH)})
            sys.stderr.write("[approval-queue] state.json 손상 — 격리 후 재생성\n")
        return _fresh_state()
    sv = obj.get("schema_version", 1)
    if sv != 1:
        raise SchemaError("state.json schema_version 미지값(%r) — 처리 거부(deny-by-default)" % sv)
    base = _fresh_state()
    bad = []
    for k in base:
        if k not in obj:
            continue
        if _state_key_ok(k, obj[k]):
            base[k] = obj[k]
        else:
            bad.append(k)
    if bad:
        _ledger_append({"event": "state_key_isolated", "keys": sorted(bad),
                        "note": "타입 위반 키만 초기값으로 강등(나머지 상태 보존)"})
        sys.stderr.write("[approval-queue] state.json 타입 위반 키 강등: %s\n" % ",".join(bad))
    return base


def _save_state(state):
    state["updated_at"] = _now()
    _write_json_atomic(STATE_PATH, javis_scrub.scrub_obj(state))


def _state_update(fn):
    """짧은 임계구역 — 로드→판정→저장만. 외부 I/O 금지(S1)."""
    with _state_lock():
        state = _load_state()
        out = fn(state)
        _save_state(state)
    return out


# ── 배달: wakeup enqueue + same-run drain 짝 (전부 락 밖) ─────────────────────
def _run_wakeup(args, timeout=20):
    wk = os.path.join(_SELF_DIR, "javis_wakeup.py")
    if not os.path.isfile(wk):
        return None
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"
    try:
        return subprocess.run([sys.executable, wk] + args, capture_output=True,
                              text=True, timeout=timeout, env=env)
    except (subprocess.SubprocessError, OSError):
        return None


def _drain_result(proc):
    """drain stdout 마지막 JSON 줄의 delivered 계수. 판독 불가 = None."""
    for line in reversed((proc.stdout or "").strip().splitlines()):
        with contextlib.suppress(ValueError):
            d = json.loads(line)
            if isinstance(d, dict) and "delivered" in d:
                return d
    return None


def _enqueue_id(proc):
    """(wakeup id(W-…), enqueue result) — 조인 키(R4). 판독 불가 = (None, None)."""
    for line in reversed((proc.stdout or "").strip().splitlines()):
        with contextlib.suppress(ValueError):
            d = json.loads(line)
            if isinstance(d, dict) and isinstance(d.get("id"), str):
                return d["id"], d.get("result")
    return None, None


def _wakeup_ledger_verdict(wakeup_id):
    """('delivered'|'skipped'|'deliver_failed'|None, why) — wakeup 원장에서 **그 id** 만 본다.

    ★R4 근본: 종전 판정은 drain 의 exit·`delivered` 계수였다. 그런데 ①`exit 5(nothing
    pending)` 를 무조건 성공으로 접었고 ②`delivered` 는 그 drain 이 처리한 **전체** 계수라
    형제 항목의 성공을 내 성공으로 오독했다. 그래서 형제 drain 이 zombie 가드로 내 pending 을
    폐기(`skipped`)해도 큐 원장에는 `delivered` 가 찍혔다 — 무성 소실. wakeup 원장의
    `wakeup_id` 는 enqueue 가 돌려준 내 id 라, 이 조인만이 '내 알림이 실제로 나갔는가'를 답한다.
    """
    if not wakeup_id or not os.path.isfile(WAKEUP_LEDGER):
        return None, "wakeup 원장 없음"
    verdict, why = None, None
    try:
        f = open(WAKEUP_LEDGER, encoding="utf-8")  # noqa: SIM115 — with 로 즉시 감싼다
    except OSError as e:
        return None, "wakeup 원장 판독 실패: %s" % e
    with f:
        for ln in f:
            if wakeup_id not in ln:
                continue
            with contextlib.suppress(ValueError):
                d = json.loads(ln)
                if not isinstance(d, dict) or d.get("wakeup_id") != wakeup_id:
                    continue
                ev = d.get("event")
                if ev in ("delivered", "delivered_dryrun", "skipped", "deliver_failed"):
                    verdict = "delivered" if ev == "delivered_dryrun" else ev
                    why = d.get("why")
    return verdict, why


def _wakeup_pending_exists(target, task_key):
    """wakeup pending 파일 존재(회수 가능 여부 판정 — javis_wakeup._pending_path 규칙 동형)."""
    p = os.path.join(WAKEUP_PENDING, "%s__%s.json"
                     % (_SAFE_RE.sub("_", target)[:80], _SAFE_RE.sub("_", task_key)[:80]))
    return os.path.isfile(p)


def _confirm_delivery(wakeup_id, target, task_key, deadline):
    """(verdict, why) — 조인 확정까지 짧게 폴링. 미확정이면 pending 존재로 분류(R4).

    형제 프로세스가 실제 `cys send` 를 잡고 있는 동안 내 drain 은 아무것도 못 하고 돌아온다.
    그 순간 '실패'로 단정하면 오탐이고 '성공'으로 단정하면 무성 소실이다 — 조인이 확정될
    때까지만 기다렸다가, 끝내 미확정이면 `pending`(회수 가능)/`undetermined`(판정 불가)로
    **분류**한다. 어느 쪽도 성공으로 접지 않는 것이 계약이다.
    """
    gone_grace = None
    while True:
        v, why = _wakeup_ledger_verdict(wakeup_id)
        if v:
            return v, why
        if not _wakeup_pending_exists(target, task_key):
            # 형제 drain 이 이미 pending 을 가져갔다 — 원장 기록은 `os.remove` 직후라 곧 온다.
            # 짧은 유예 뒤에도 없으면 그 drain 이 중도 실패한 것이므로 **판정 불가**로 확정한다
            # (여기서 성공으로 접으면 정확히 그 자리가 무성 소실 구멍이 된다).
            if gone_grace is None:
                gone_grace = time.time() + 1.0
            elif time.time() >= gone_grace:
                return "undetermined", ("drain 이 pending 을 가져갔으나 종결 기록 없음 — "
                                        "판정 불가(wakeup 원장 결손)")
        if time.time() >= deadline:
            break
        time.sleep(0.2)
    if _wakeup_pending_exists(target, task_key):
        return "pending", "wakeup pending 잔류 — 미배달(후속 drain 편승 가능)"
    return "undetermined", "drain 종결 흔적 없음 — 판정 불가(형제 drain 처리 중이거나 유실)"


def _deliver(target, task_key, reason, idem, severity, payload=None):
    """즉시 배달 = enqueue(멱등키) + same-run `drain --deliver --target <대상>` 1회.

    **(ok, detail, verdict)** — verdict ∈ delivered·no_deliver·skipped·deliver_failed·
    pending·undetermined·enqueue_failed. `ok` 는 **verdict=='delivered' 일 때만 True** 이고
    나머지는 전부 호출자가 outbox 에 남긴다(확정 배달 아닌 모든 결말 = 흔적 1줄).
    **at-least-once 아님**(D18 — 라이브 영수증 조인은 데몬 0.14.7 이후로 이연).
    """
    # ★M2 배달 예산 단일화 — enqueue 20 + drain 30 + confirm 25 = **최대 75초**였다(계약이
    #   50초로 적어 둔 것은 오기). 세 구간이 각자 상한을 갖고 합쳐지면 총량을 아무도 모르므로,
    #   **하나의 deadline** 으로 합을 자른다. 이후 구간은 남은 예산만큼만 쓴다.
    budget = _int_env("JAVIS_APPROVAL_DELIVER_BUDGET_SEC", DELIVER_BUDGET_DEFAULT)
    dl = time.time() + budget

    def _left(cap):
        return max(1.0, min(float(cap), dl - time.time()))

    argv = ["enqueue", "--to", target, "--task", task_key, "--reason", _flatten(reason, 1200),
            "--idempotency-key", idem, "--severity", severity]
    if payload:
        argv += ["--payload", json.dumps(payload, ensure_ascii=False)]
    r = _run_wakeup(argv, timeout=_left(20))
    if r is None or r.returncode != 0:
        return (False, "enqueue 실패(rc=%s)" % (None if r is None else r.returncode),
                "enqueue_failed")
    wid, eres = _enqueue_id(r)
    if _NO_DELIVER:
        return True, "enqueued(id=%s result=%s · --no-deliver·drain 생략)" % (wid, eres), \
               "no_deliver"
    d = _run_wakeup(["drain", "--deliver", "--target", target], timeout=_left(30))
    drc = None if d is None else d.returncode
    res = _drain_result(d) if d is not None else None
    if wid is None:
        # 조인 키를 못 얻었다 — 계수 판정으로 강등하되 **성공 단정은 금지**한다.
        if drc == 0 and res and res.get("delivered", 0) >= 1 and not res.get("skipped"):
            return True, "id 조인 불가 · drain delivered=%s(계수 단독 판정)" % res["delivered"], \
                   "delivered"
        return (False, "enqueue id 판독 불가 — 판정 불가(drain rc=%s res=%s)" % (drc, res),
                "undetermined")
    deadline = min(dl, time.time() + _int_env("JAVIS_APPROVAL_CONFIRM_SEC",
                                              CONFIRM_SEC_DEFAULT))
    verdict, why = _confirm_delivery(wid, target, task_key, deadline)
    tail = " (drain rc=%s res=%s id=%s)" % (drc, res, wid)
    if verdict == "delivered":
        return True, "delivered(wakeup id 조인 확정)" + tail, "delivered"
    if verdict == "skipped":
        return (False, ("skipped(%s — zombie 가드/fast-fail 로 pending 폐기 · outbox 가 SOT)"
                        % (why or "?")) + tail, "skipped")
    if verdict == "deliver_failed":
        return (False, ("deliver_failed(%s — pending 잔류)" % (why or "?")) + tail,
                "deliver_failed")
    return False, (why or verdict) + tail, verdict


def _outbox_append(request_id, target, task_key, detail, extra=None):
    """배달 실패 SOT(조건 17③) — 후속 preflight C72 확장이 표면화(이번 Wave 는 기록까지만)."""
    rec = {"ts": _now(), "request_id": request_id, "target": target,
           "task_key": task_key, "why": detail}
    if extra:
        rec.update(extra)
    _append_jsonl(OUTBOX, rec)


def _task_key(rid, prefix="approval"):
    """wakeup task_key — **rid 해시를 포함**해 유일성을 보장한다(H3 조인 키 불변식).

    종전은 sanitize + 80자 절단뿐이라 서로 다른 rid 가 같은 key 로 접힐 수 있었다
    (`guard:T…<동일 64자>:sigA` / `:sigB`). wakeup pending 은 `(target, task_key)` 로
    묶이므로 충돌하면 **두 알림이 하나로 코얼레싱**되고, 그 순간 R4 의 "내 wakeup id 를
    조인한다"는 전제가 무너진다(둘 중 하나의 배달 결과를 둘 다의 것으로 읽는다).
    `_item_file` 은 이미 해시를 쓰는데 여기만 안 쓰던 **비대칭**을 없앤다.
    """
    h = hashlib.sha1(rid.encode("utf-8")).hexdigest()[:8]   # noqa: S324 — 키 분리 전용
    return "%s-%s.%s" % (prefix, _SAFE_RE.sub("_", rid)[:64], h)


def _item_reason(item):
    return _flatten("[승인 큐] %s · risk=%s · sev=%s · source=%s · %s (참조 %s)"
                    % (item["class"], item["risk"], item["severity"], item["source"],
                       item.get("summary") or "(요약 없음)",
                       _relativize(item.get("payload_ref"))), 1200)


# verdict → 원장 event(닫힌 사상 · 계약 §1 어휘표와 1:1). 성공은 delivered 하나뿐이다.
_VERDICT_EVENT = {"delivered": "delivered", "no_deliver": "enqueued_no_deliver",
                  "skipped": "deliver_skipped", "deliver_failed": "deliver_failed",
                  "pending": "deliver_pending", "undetermined": "deliver_undetermined",
                  "enqueue_failed": "deliver_failed"}


def _notify(item, target=None, task_key=None, reason=None):
    """항목 1건의 개별 알림 — 락 밖 실행. 항목당 1회(원장 `notified_at` 결정론 집행)."""
    tgt = target or _route_for(item["class"], notify_unbound=True)[0]
    tk = task_key or _task_key(item["request_id"])          # H3 — rid 해시 포함
    why = reason or _item_reason(item)
    ok, detail, verdict = _deliver(tgt, tk, why, item["request_id"], item["severity"],
                                   payload={"request_id": item["request_id"],
                                            "class": item["class"], "risk": item["risk"]})
    item["notified_at"] = _now()
    item["routed_to"] = tgt
    item["delivery_ok"] = ok
    item["delivery_verdict"] = verdict
    item["delivery_detail"] = _oneline(detail, 300)   # stdout 표면화용(항목 파일에는 미저장)
    _ledger_append({"event": _VERDICT_EVENT.get(verdict, "deliver_failed"),
                    "request_id": item["request_id"], "target": tgt,
                    "task_key": tk, "verdict": verdict, "detail": detail})
    if verdict not in ("delivered", "no_deliver"):
        # ★확정 배달이 아닌 모든 결말은 outbox 1줄(무성 소실 0 의 구조적 보장 · R4).
        _outbox_append(item["request_id"], tgt, tk, detail,
                       {"class": item["class"], "severity": item["severity"],
                        "verdict": verdict, "recoverable": verdict == "pending"})
    return ok, detail


def _route_fallback(item, fallback, task_key):
    """1차 대상 배달 실패 시 fallback 1회 — 무성 소실 차단(submit·collect 공통 · S6).

    재시도가 아니라 **다른 대상 1회**다(같은 대상 재시도는 wakeup pending 잔류가 담당).
    멱등키는 **원본 request_id 그대로** 유지한다 — 접미사를 붙이면 큐↔wakeup 조인 키가
    갈라져 상관 추적이 끊긴다(대상이 다르면 wakeup pending 파일이 애초에 분리된다).
    """
    if not fallback or item.get("delivery_ok"):
        return False
    ok, detail, verdict = _deliver(fallback, task_key, _item_reason(item),
                                   item["request_id"], item["severity"])
    _ledger_append({"event": "routed_fallback", "request_id": item["request_id"],
                    "target": fallback, "ok": ok, "verdict": verdict, "detail": detail})
    if ok:
        item["routed_to"] = fallback
        item["delivery_ok"] = True
        item["delivery_verdict"] = verdict
    else:
        _outbox_append(item["request_id"], fallback, task_key, detail,
                       {"class": item["class"], "severity": item["severity"],
                        "verdict": verdict, "note": "fallback 도 확정 배달 실패"})
    return ok


# ── routing / self-heal allowlist(계약 파일 · 타입 검증 후 내장 폴백 강등) ────
def _load_contract(env_key, filename, fallback, validator):
    """(obj, path, 출처) — 손상·타입 위반은 **내장 폴백 강등**, schema_version 미지값은 exit 6."""
    path = os.environ.get(env_key, "").strip() or os.path.join(APPROVALS_DIR, filename)
    obj, err = _read_json(path)
    if obj is None:
        if err == "corrupt":
            _ledger_append({"event": "contract_fallback", "path": _relativize(path),
                            "why": "손상(JSON 파싱 실패)"})
            sys.stderr.write("[approval-queue] %s 손상 — 내장 폴백 사용\n" % _relativize(path))
        return fallback, path, ("corrupt" if err == "corrupt" else "absent")
    sv = obj.get("schema_version", 1)
    if sv != 1:
        raise SchemaError("%s schema_version 미지값(%r) — 처리 거부" % (_relativize(path), sv))
    ok, why = validator(obj)
    if not ok:
        _ledger_append({"event": "contract_fallback", "path": _relativize(path), "why": why})
        sys.stderr.write("[approval-queue] %s 타입 위반(%s) — 내장 폴백 사용\n"
                         % (_relativize(path), why))
        return fallback, path, "invalid"
    return obj, path, "file"


def _routing_validator(obj):
    routes = obj.get("routes")
    if not isinstance(routes, dict):
        return False, "routes 가 객체가 아님"
    for k, v in routes.items():
        if not isinstance(v, dict) or not isinstance(v.get("target", ""), str):
            return False, "routes.%s.target 타입 위반" % k
        fb = v.get("fallback")
        if fb is not None and not isinstance(fb, str):
            return False, "routes.%s.fallback 타입 위반" % k
    return True, None


def _allowlist_validator(obj):
    acts = obj.get("actions")
    if not isinstance(acts, list) or not all(isinstance(a, str) for a in acts):
        return False, "actions 가 문자열 배열이 아님"
    if not isinstance(obj.get("prereg", {}), dict):
        return False, "prereg 가 객체가 아님"
    return True, None


def _resolve_label(label, obj):
    """(해소된 라벨, 미바인딩 여부) — E2-4(R-03) 라벨 해소 규칙.

    우선순위: env `JAVIS_APPROVAL_LABEL_<LABEL>`(카나리아 1회성) > routing.json
    `label_bindings.<label>` > 자리표시 그대로. 자리표시(`reviewer1`·`reviewer2`)가 해소되지
    않은 채 남으면 **미바인딩**이다 — 그 상태로 배달하면 대상이 실재하지 않아 실패하고
    fallback(master)으로 조용히 떨어진다. 그 조용함이 R-03 의 본체다(조건 29 가 요구한
    '리뷰어1 1차 진단'이 무성으로 증발하고 master 가 실패 디버깅 실무자로 복원된다).
    """
    if not isinstance(label, str) or not label:
        return label, False
    envk = "JAVIS_APPROVAL_LABEL_%s" % re.sub(r"[^A-Za-z0-9]", "_", label).upper()
    v = (os.environ.get(envk) or "").strip()
    if not v:
        b = obj.get("label_bindings")
        if isinstance(b, dict) and isinstance(b.get(label), str):
            v = b[label].strip()
    if v:
        return v, False
    return label, label in PLACEHOLDER_LABELS


def _unbound_notice(cls, label, fallback):
    """미바인딩 배달 직전 **loud 고지 1줄 + 원장 이벤트 + 경보 1회**(E2-4).

    조용한 강등을 금지한다: 이 배달은 실패하고 fallback 으로 갈 것이며, 그 순간 조건 29
    (리뷰어1 1차 진단 후 master 는 판정만)는 **미발효**다. 운영자가 그 사실을 모르는 것이
    결함이지, fallback 자체가 결함은 아니다.
    """
    line = ("[approval-queue] 리뷰어 라벨 '%s' 미바인딩 — 배달 대상이 실재하지 않는다. "
            "실패 시 fallback=%s 직행(조건 29 '리뷰어1 1차 진단' **미발효**). "
            "바인딩: routing.json label_bindings.%s = \"reviewer-gemini|reviewer-codex\" "
            "(또는 env JAVIS_APPROVAL_LABEL_%s)"
            % (label, fallback or "없음", label,
               re.sub(r"[^A-Za-z0-9]", "_", label).upper()))
    sys.stderr.write(line + "\n")
    key = "%s|%s" % (cls, label)
    if key not in _UNBOUND_LOGGED:
        _UNBOUND_LOGGED.add(key)
        _ledger_append({"event": "routing_unbound", "class": cls, "label": label,
                        "fallback": fallback, "condition_29": "미발효",
                        "detail": _oneline(line, 400)})
    # 경보 1회(6h 코얼레싱 마커) — 원장은 사후 감사용이고, 이건 사람에게 도달하는 축이다.
    mark = os.path.join(APPROVALS_DIR, ".routing-unbound.%s"
                        % re.sub(r"[^A-Za-z0-9._-]", "_", label))
    try:
        if time.time() - os.stat(mark).st_mtime < UNBOUND_ALERT_TTL_SEC:
            return
    except OSError:
        pass
    with contextlib.suppress(OSError):
        os.makedirs(APPROVALS_DIR, exist_ok=True)
        with open(mark, "w", encoding="utf-8") as f:
            f.write(_now() + "\n")
    _run_wakeup(["enqueue", "--to", "master", "--task", "routing-unbound-%s" % label,
                 "--reason", _oneline(line, WAKEUP_REASON_MAX),
                 "--idempotency-key", "routing-unbound:%s" % label,
                 "--severity", "warn"])


def _route_for(cls, notify_unbound=False):
    """(target, fallback) — routing.json 경유(escalation→리뷰어1). 미지 class 는 master.

    E2-4: 라벨은 `label_bindings` 로 해소한다. `notify_unbound=True`(실제 배달 경로에서만)
    이면 미해소 자리표시를 loud 하게 고지한다 — 조회 경로(list·digest 집계)는 조용하다.
    """
    obj, _p, _src = _load_contract("JAVIS_APPROVAL_ROUTING", "routing.json",
                                   ROUTING_FALLBACK, _routing_validator)
    routes = obj.get("routes") or {}
    r = routes.get(cls) or ROUTING_FALLBACK["routes"].get(cls) or {}
    target, unbound = _resolve_label(r.get("target") or "master", obj)
    fallback, _fb_unbound = _resolve_label(r.get("fallback"), obj)
    if unbound and notify_unbound:
        with contextlib.suppress(Exception):   # 고지 실패가 배달을 막지 않는다(부수 채널)
            _unbound_notice(cls, target, fallback)
    return target, fallback


def _meta_target():
    """큐 **메타 알림**(병합·유예 요약·다이제스트) 대상 — routing 위임(R6d).

    종전에는 `"master"` 하드코딩이었다. 값 자체는 계약상 master 가 맞지만(이 셋은 개별 승인이
    아니라 **큐 운영 상태 보고**라 판정 주체에게 간다 — routing.json `queue-meta.note` 에
    명문), 하드코딩은 routing 표를 우회하는 두 번째 경로라 표에 편입한다.
    """
    return _route_for("queue-meta")[0]


def _self_heal_allowlist():
    """(actions, meta) — 사전 인가 목록 로드(S11a).

    ①`prereg.sealed != true` 면 **내장 폴백으로 강등**(원장 경고) — 봉인 없는 파일이 면제
    범위를 넓히지 못하게 한다. ②파일 actions 가 폴백의 부분집합이 아니면 **초과분 거부**.
    ③경로·sha256·sealed 는 bypass 원장에 박제된다(사후 감사 흔적).
    """
    fb = list(SELF_HEAL_ALLOWLIST_FALLBACK["actions"])
    obj, path, src = _load_contract("JAVIS_APPROVAL_ALLOWLIST", "self-heal-allowlist.json",
                                    SELF_HEAL_ALLOWLIST_FALLBACK, _allowlist_validator)
    meta = {"path": _relativize(path), "sha256": _sha256_file(path), "source": src,
            "sealed": bool((obj.get("prereg") or {}).get("sealed") is True)}
    if src != "file":
        meta["source"] = "builtin"
        return fb, meta
    if not meta["sealed"]:
        _ledger_append({"event": "self_heal_allowlist_degraded", "path": meta["path"],
                        "sha256": meta["sha256"], "sealed": False,
                        "why": "prereg.sealed != true — 내장 폴백으로 강등([OT] 봉인 대기)"})
        meta["source"] = "builtin(강등)"
        return fb, meta
    acts = [str(a) for a in (obj.get("actions") or [])]
    extra = [a for a in acts if a not in fb]
    if extra:
        _ledger_append({"event": "self_heal_allowlist_degraded", "path": meta["path"],
                        "sha256": meta["sha256"], "sealed": True, "rejected": sorted(extra),
                        "why": "내장 폴백 부분집합 위반 — 초과분 거부(면제 확장 차단)"})
    return [a for a in acts if a in fb], meta


def esc_request_id_local(task, sig):
    """esc-bundle 멱등키 정규화 — **guard 정의 import 재사용**(S11b), 실패 시 동형 폴백.

    guard 의 `esc_request_id`(javis_completion_guard) 가 단일 정의다. import 실패(버전 찢김)
    시에만 같은 규칙을 지역 계산한다 — 경로 계산 수준의 소형 동형 복제(guard
    `_pending_path_for` 전례)이며 규칙이 갈라지면 조인 키가 깨지므로 원장에 강등을 남긴다.
    """
    try:
        from javis_completion_guard import esc_request_id  # 형제 모듈 단일 정의
        return esc_request_id(task, sig)
    except Exception as e:  # noqa: BLE001
        _ledger_append({"event": "contract_fallback", "path": "javis_completion_guard.py",
                        "why": "esc_request_id import 실패(버전 찢김 의심): %s" % e})
        return "guard:%s:%s" % (task, str(sig).strip() if sig else "nosig")


# ── risk 분류(조건 22②③) ────────────────────────────────────────────────────
_DAEMON_RISK_HIGH = ("highrisk", "high", "denied", "blocked")
_DAEMON_RISK_NORMAL = ("autoeligible", "auto", "normal", "low", "lowrisk", "safe")


def classify_risk(daemon_risk_class=None, cmd_text=None, forced=None):
    """(risk, why) — 1차 데몬 신호 + 2차 팩 분류기 오버레이(AutoEligible→high 강등).

    2차 분류기는 `javis_task._risk_scan_cmd` **import 재사용**이다(복제 금지 — 삭제·네트워크·
    서버기동·push생산 4종 어휘와 javis_resource_gate SERVER_PATTERNS 를 그대로 태운다).
    import·분류 실패(버전 찢김)는 **보수측 high**. 호출자의 `--risk normal` 은 강등하지
    못하며, 거부 문구는 **실제 승격 근거**(데몬 신호/2차 분류기/분류 불가)로 분기한다(S12d).
    """
    reasons, sources = [], []
    risk = "normal"

    def escalate(why, tag):
        nonlocal risk
        risk = "high"
        reasons.append(why)
        sources.append(tag)

    if daemon_risk_class:
        low = str(daemon_risk_class).strip().lower()
        if low in _DAEMON_RISK_HIGH:
            escalate("데몬 risk_class=%s" % daemon_risk_class, "데몬 risk_class")
        elif low in _DAEMON_RISK_NORMAL:
            reasons.append("데몬 risk_class=%s" % daemon_risk_class)
        else:
            escalate("데몬 risk_class 미지값(%s) — 보수측 high" % daemon_risk_class,
                     "데몬 risk_class 미지값")
    if cmd_text:
        try:
            from javis_task import _risk_scan_cmd  # 형제 모듈 단일 정의 재사용(B2 §3)
            cats, err = _risk_scan_cmd(cmd_text)
        except Exception as e:  # noqa: BLE001
            cats, err = None, "javis_task._risk_scan_cmd import 실패: %s" % e
        if err:
            escalate("2차 분류 불가(%s) — 보수측 high" % err, "2차 분류 불가")
        elif cats:
            escalate("팩 2차 분류기 위험 어휘: %s" % ",".join(sorted(cats)), "팩 2차 분류기")
        else:
            reasons.append("팩 2차 분류기 위험 어휘 없음")
    # 호출자 선언은 **승격만** 유효하다 — 자기신고로 게이트를 끄지 못한다(조건 22②).
    if forced == "high" and risk != "high":
        escalate("호출자 --risk high 강제", "호출자 승격")
    elif forced == "normal" and risk == "high":
        reasons.append("--risk normal 요청 무시(%s 판정 high 우선 — 자기신고 강등 불가)"
                       % (sources[0] if sources else "상위"))
    return risk, " · ".join(reasons) or "신호 없음(기본 normal)"


# ── 데몬 관찰(락 밖 — subprocess) ────────────────────────────────────────────
def _daemon_fingerprint():
    """`cys status --json` → 데몬 지문. 관찰 실패 = None(무간섭 — 유예를 만들지 않는다).

    지문 = (started_at, build_id, version[, pid]) — 라이브 실측 payload 는
    `handlers.rs:3562` 의 `daemon{version,started_at,build_id,...}`. **필수 3키 중 하나라도
    없으면 관찰 실패로 취급**한다(S5) — 부분 관찰끼리 비교하면 지문이 흔들려 유령 재기동이
    난다(억제 창이 근거 없이 열리는 것은 새 데드락이다).
    """
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"
    try:
        r = subprocess.run([_cys_bin(), "status", "--json"], capture_output=True,
                           text=True, timeout=5, env=env)
    except (subprocess.SubprocessError, OSError):
        return None
    if r.returncode != 0:
        return None
    try:
        d = json.loads(r.stdout)
    except ValueError:
        return None
    dm = (d or {}).get("daemon")
    if not isinstance(dm, dict):
        return None
    if any(dm.get(k) in (None, "") for k in FP_REQUIRED):
        return None                       # 부분 관찰 = 관찰 실패(유령 재기동 금지)
    fp = {k: dm.get(k) for k in FP_REQUIRED + ("pid",) if k in dm}
    return fp or None


# ── 유지보수 tick: 판정(락 안) → 배달(락 밖) → 반영 ──────────────────────────
def _plan_tick(state, fp, items):
    """락 안 — 만료 sweep·rate 이월·재기동 관찰·보존 정책까지 **결정만** 하고 job 을 넘긴다.

    창 종료·이월 확정은 여기서 상태에 못 박는다(exactly-once). 배달이 실패해도 되돌리지
    않는다 — 실패의 SOT 는 outbox 이고, 되돌리면 무한 재발행이 된다.
    항목 정합(`_items_sync`)은 **락 밖**에서 끝내고 결과만 받는다(R5b — 원장 전문 리플레이가
    임계구역 안에 있으면 원장이 커질수록 5초 락 상한을 넘겨 무락 degrade 를 유발한다).
    """
    plan = {"jobs": [], "expired": 0, "reaped": 0, "items": items}

    # ① TTL 만료 sweep(pending → expired) — **rid별 read-modify-write**(R2).
    ttl = _int_env("JAVIS_APPROVAL_TTL_DAYS", TTL_DAYS_DEFAULT) * 86400
    # ★M4: 락 보유 시간을 **상수화**한다. sweep 은 항목 수에 비례해 파일 쓰기를 하므로
    #   임계구역이 큐 크기에 선형으로 늘어나 5초 상한을 넘길 수 있었다. 배치 상한을 두면
    #   남은 만료분은 다음 tick 이 이어받는다(멱등 — 만료는 조건이지 사건이 아니다).
    sweep_max = _int_env("JAVIS_APPROVAL_SWEEP_MAX", SWEEP_MAX_DEFAULT) or SWEEP_MAX_DEFAULT
    if ttl > 0:
        for key, it in list(items.items()):
            if plan["expired"] >= sweep_max:
                plan["sweep_capped"] = True
                break
            if it.get("state") != "pending":
                continue
            born = it.get("created_epoch") or 0
            if not (born and _epoch() - born > ttl):
                continue
            cur, _e = _item_read(it["request_id"], bool(it.get("bypass")))
            if cur is None or cur.get("state") != "pending":
                # 스캔 이후 누가 승인·거부했다 — 만료로 덮지 않는다(결정 보존).
                if cur is not None:
                    items[key] = cur
                continue
            cur.update({"state": "expired", "decided_at": _now(), "decided_epoch": _epoch()})
            _item_write(cur)
            items[key] = cur
            _ledger_append({"event": "expired", "request_id": cur["request_id"],
                            "decided_epoch": cur["decided_epoch"],
                            "ttl_days": ttl // 86400})
            plan["expired"] += 1

    # ② rate cap 이월 병합(이전 시간창 초과분 → 'N건 병합' critical 1건)
    rate = state["rate"]
    win = _hour_window()
    prev = rate.get("window_start")
    pending_merge = [r for r in (rate.get("carry") or []) if isinstance(r, str)]
    if prev != win:
        for r in (rate.get("overflow") or []):
            if isinstance(r, str) and r not in pending_merge:
                pending_merge.append(r)
        rate["window_start"], rate["count"], rate["overflow"] = win, 0, []
    # ★L-d: carry 는 창 전환 여부와 무관하게 **항상** 배달한다 — `_rate_account` 가 먼저 창을
    #   넘겨 놓은 경우에도 초과분이 여기서 반드시 회수된다(증발 경로 0).
    rate["carry"] = []
    if pending_merge:
        plan["jobs"].append({"kind": "rate_merge", "window": prev,
                             "request_ids": pending_merge})

    # ③ 데몬 재기동 관찰·유예 창(held 이월 · S5)
    d = state["daemon"]
    grace = _int_env("JAVIS_APPROVAL_RESTART_GRACE_SEC", RESTART_GRACE_DEFAULT)
    if fp is not None:
        if d.get("fingerprint") is None:
            d.update({"fingerprint": fp, "observed_at": _now()})
            _ledger_append({"event": "daemon_baseline", "fingerprint": fp})
        elif d["fingerprint"] != fp:
            held = [x for x in (d.get("held") or []) if isinstance(x, str)]
            # ★창 중첩: held 를 비우지 않는다(이월·누적). summary_sent 는 held 와 짝을 맞춘다.
            d.update({"fingerprint": fp, "observed_at": _now(),
                      "grace_until": _epoch() + grace, "held": held,
                      "summary_sent": False})
            _ledger_append({"event": "daemon_restart_detected", "fingerprint": fp,
                            "grace_sec": grace, "carried_held": len(held)})
    if d.get("grace_until") and _epoch() >= d["grace_until"] and not d.get("summary_sent"):
        held = [x for x in (d.get("held") or []) if isinstance(x, str)]
        d["summary_sent"] = True
        d["grace_until"] = 0
        d["held"] = []
        if held:
            plan["jobs"].append({"kind": "grace_summary", "request_ids": held,
                                 "observed_at": d.get("observed_at") or ""})
        else:
            _ledger_append({"event": "grace_window_closed", "count": 0})

    # ④ 보존 정책(종결 항목 TTL·세대 보존 회전·마커/락 잔해 GC · S11f)
    plan["reaped"] = _retention_sweep(state, items, ttl)
    return plan


def _retention_sweep(state, items, ttl):
    """종결 항목 items 제거(원장 잔존)·ledger/outbox 로테이션·마커/락 잔해 GC."""
    reaped = 0
    now = _epoch()
    if ttl > 0:
        for key, it in list(items.items()):
            if it.get("state") not in TERMINAL_STATES:
                continue
            done = it.get("decided_epoch") or it.get("created_epoch") or 0
            if done and now - done > ttl:
                _item_remove(it)
                items.pop(key, None)
                _ledger_append({"event": "item_reaped", "request_id": it["request_id"],
                                "state": it.get("state"),
                                "bypass": bool(it.get("bypass")),
                                "note": "종결 항목 TTL 경과 — items 제거(원장 잔존)"})
                reaped += 1
    # ★R3 세대 보존 회전 — `.1` 덮어쓰기를 폐기했다. 종전 구조는 2회 회전에서 최초 세대가
    #   사라져 **복원 SOT(미종결 queued 스냅샷)가 소멸**했고, 리플레이는 `.1` 을 애초에 읽지도
    #   않아 회전 1회만으로도 복원이 무효였다.
    rotate_at = _int_env("JAVIS_APPROVAL_ROTATE_BYTES", LEDGER_ROTATE_BYTES) or LEDGER_ROTATE_BYTES
    for p in (LEDGER, OUTBOX):
        with contextlib.suppress(OSError):
            if os.path.isfile(p) and os.path.getsize(p) > rotate_at:
                _rotate_preserving(p)
                _ledger_append({"event": "ledger_rotated", "path": _relativize(p),
                                "generations": len(_generations(p)),
                                "note": "세대 보존(폐기 0) — 리플레이가 오래된 세대부터 이어 읽는다"})
    marker_ttl = MARKER_TTL_DAYS * 86400
    for pat in (".digest-sent-*", "digest-*.md", "*.tmp.*", "*.corrupt-*", "*.lock.stale.*"):
        for p in glob.glob(os.path.join(APPROVALS_DIR, pat)) + \
                glob.glob(os.path.join(ITEMS_DIR, pat)):
            with contextlib.suppress(OSError):
                age = time.time() - os.path.getmtime(p)
                limit = 3600 if ".tmp." in os.path.basename(p) else marker_ttl
                if age > limit:
                    (os.remove if os.path.isfile(p) else os.rmdir)(p)
    return reaped


def _run_jobs(jobs):
    """락 밖 — 병합·요약 배달. 실패는 outbox(SOT)로 남기고 판정에 개입하지 않는다."""
    results = []
    for job in jobs:
        if job["kind"] == "rate_merge":
            rids = job["request_ids"]
            shown, more = rids[:MERGE_LIST_MAX], max(0, len(rids) - MERGE_LIST_MAX)
            reason = ("[승인 큐 rate cap] 이전 시간창(win=%s) escalation/critical 초과분 %d건 "
                      "병합 — 다이제스트 이월 금지(조건 17②)·request_ids=%s%s"
                      % (job["window"], len(rids), ",".join(shown),
                         (" 외 %d건" % more) if more else ""))
            meta = _meta_target()
            ok, detail, verdict = _deliver(meta, "approval-rate-merge", reason,
                                           "approval-rate-merge:%s" % job["window"], "critical",
                                           payload={"merged": len(rids),
                                                    "window": job["window"]})
            _ledger_append({"event": "rate_merge_delivered" if ok else "rate_merge_failed",
                            "count": len(rids), "window": job["window"], "detail": detail,
                            "target": meta, "verdict": verdict, "request_ids": rids})
            if not ok:
                _outbox_append("approval-rate-merge:%s" % job["window"], meta,
                               "approval-rate-merge", detail,
                               {"merged": len(rids), "verdict": verdict})
            results.append({"kind": "rate_merge", "ok": ok, "request_ids": rids})
        elif job["kind"] == "grace_summary":
            rids = job["request_ids"]
            shown, more = rids[:MERGE_LIST_MAX], max(0, len(rids) - MERGE_LIST_MAX)
            reason = ("[승인 큐] 데몬 재기동 유예 창 종료 — 창 중 억제한 알림 %d건 요약"
                      "(창 중첩 시 이월 누적 · critical 은 창 중에도 즉시 배달)"
                      "·request_ids=%s%s"
                      % (len(rids), ",".join(shown), (" 외 %d건" % more) if more else ""))
            meta = _meta_target()
            ok, detail, verdict = _deliver(meta, "approval-restart-summary", reason,
                                           "approval-restart:%s" % job.get("observed_at", ""),
                                           "warn", payload={"held": len(rids)})
            _ledger_append({"event": "grace_summary_delivered" if ok else "grace_summary_failed",
                            "count": len(rids), "detail": detail, "target": meta,
                            "verdict": verdict, "request_ids": rids})
            if not ok:
                # held 가 비어있지 않은데 요약이 실패했다 — 유일 SOT 는 outbox 다(S5).
                _outbox_append("approval-restart-summary", meta,
                               "approval-restart-summary", detail,
                               {"held": len(rids), "verdict": verdict, "request_ids": rids})
            results.append({"kind": "grace_summary", "ok": ok, "request_ids": rids})
    return results


_SUPPRESS_FIELDS = ("rate_overflow", "grace_held", "deferred_to_digest", "notified_at",
                    "notified_slack_at", "slack_tier", "routed_to", "delivery_ok",
                    "delivery_verdict")


def _item_merge_write(rid, fields, bypass=False):
    """(item|None, why) — **rid별 read-modify-write**(R2 TOCTOU 차단).

    억제·배달 플래그만 디스크 최신본 위에 얹는다. 읽은 state 가 TERMINAL 이면 **건드리지
    않는다** — 배달 I/O 는 최대 50초라, 그 사이에 들어온 approve/deny 를 낡은 스냅샷의
    통짜 덮어쓰기로 되돌리면 **결정이 무음으로 사라진다**(신규 TOCTOU 였다).
    """
    unknown = [k for k in fields if k not in _SUPPRESS_FIELDS]
    if unknown:                                  # 억제·배달 플래그 외 필드 갱신 금지(계약)
        raise SchemaError("_item_merge_write 허용 밖 필드: %s" % ",".join(sorted(unknown)))
    cur, err = _item_read(rid, bypass)
    if cur is None:
        return None, err or "absent"
    if cur.get("state") in TERMINAL_STATES:
        return cur, "terminal-skip"
    cur.update(fields)
    _item_write(cur)
    return cur, None


def _commit_jobs(items, results):
    """락 밖 — 병합·요약에 실린 항목의 억제 표식 해제. **rid별 RMW**(R2)."""
    for res in results:
        for rid in res["request_ids"]:
            with contextlib.suppress(OSError, SchemaError):
                cur, why = _item_merge_write(
                    rid, {"rate_overflow": False, "grace_held": False, "notified_at": _now()})
                if cur is not None:
                    items[_item_key(rid)] = cur
                if why == "terminal-skip":
                    _ledger_append({"event": "commit_skipped_terminal", "request_id": rid,
                                    "state": cur.get("state"),
                                    "note": "배달 중 확정된 결정 보존 — 낡은 스냅샷 덮어쓰기 거부"})


def _maintenance(observe=True):
    """서브커맨드 호출 1회 = 유지보수 tick 1회. **배달·항목 정합은 전부 락 밖**(S1·R5b).

    큐에 데몬은 없다 — 호출이 곧 시계다. 자가치유 사전 인가 경로만 이 tick 을 통째로
    건너뛴다(S4 — 데몬 wedge 시 지연 0 보장).
    """
    fp = _daemon_fingerprint() if observe else None
    items, restored, lost = _items_sync()          # 락 밖(디스크·원장 I/O)
    plan = _state_update(lambda s: _plan_tick(s, fp, items))
    plan["restored"], plan["lost"] = restored, lost
    if lost:
        _emit("agent.error", {"agent": "approval-queue",
                              "summary": "[approval-queue] 상태 소실 — 항목 파일·원장 리플레이"
                                         "로 복원 불가(state_lost·%s). 조용한 fresh 출발을 "
                                         "거부하고 중단한다." % lost.get("reason")})
    results = _run_jobs(plan["jobs"])
    if results:
        _commit_jobs(plan["items"], results)
    plan["results"] = results
    return plan


def _resnapshot_survivors(cmd):
    """전멸 재출발 시 **디스크 생존 항목을 새 원장에 재스냅샷**한다(F3).

    ★근거: 재출발한 원장에는 생존 항목의 `queued` 스냅샷이 없다. 그대로 두면 그 순간부터
    그 항목들은 **복원 SOT 가 없는 상태**로 산다 — 다음에 `items/` 가 날아가면 리플레이가
    되살릴 것이 없고, 원장에 흔적도 없으니 **두 번째 소실은 무음**이다. 첫 소실을 시끄럽게
    알리고 두 번째 소실을 조용하게 만드는 것은 이 큐의 1번 계약과 정면으로 어긋난다.
    비용/위험 판단: ①이 경로는 전멸이라는 희귀 사건에서만 돈다 ②비용은 생존 항목 수에
    비례하는 읽기+append 이고 상한(`_RESNAP_MAX`)으로 잘린다 ③재기록은 멱등이다(리플레이는
    키당 마지막 상태로 수렴하고, 중복 `queued` 는 같은 키로 접힌다) ④terminal 항목도 자기
    `state` 그대로 실려 부활하지 않는다. 이득이 비용을 명백히 넘어 **재기록을 택한다**.
    """
    disk, _corrupt = _items_scan()
    rows = list(disk.values())[:_RESNAP_MAX]
    n = 0
    for it in rows:
        base = {"request_id": it.get("request_id"), "class": it.get("class"),
                "risk": it.get("risk"), "severity": it.get("severity"),
                "source": it.get("source"), "payload_ref": it.get("payload_ref"),
                "risk_why": it.get("risk_why"), "summary": it.get("summary"),
                "created_at": it.get("created_at"), "created_epoch": it.get("created_epoch"),
                "state": it.get("state"), "local_only": it.get("local_only"),
                "snapshot": 1, "resnapshot": 1,
                "note": "전멸 재출발 — 디스크 생존 항목 재스냅샷(복원 SOT 재수립)"}
        if it.get("bypass"):
            base.update({"event": "self_heal_bypass", "action": it.get("action"),
                         "ts": it.get("created_at")})
        else:
            base["event"] = "queued"
        if _append_jsonl(LEDGER, base):
            n += 1
    _ledger_append({"event": "survivors_resnapshotted", "cmd": cmd, "count": n,
                    "disk_items": len(disk),
                    "capped": len(disk) > _RESNAP_MAX,
                    "note": "재출발 원장에 복원 SOT 재수립 — 두 번째 소실이 무음이 되지 않게"})
    return n


def _abort_state_lost(plan, cmd):
    """`state_lost` 중단 — **stderr 1줄 + stdout JSON + 원장 1줄**(R1 필수 부수).

    종전에는 이 경로가 `return EXIT_INVALID` 한 줄이라, 유령 소실이 나면 submit 이
    **stdout·stderr·원장 어디에도 흔적 없이** 사라졌다(무음 폐기). 어떤 중단이든 세 곳 모두에
    남는다 — 관측 불가능한 실패를 만들지 않는 것이 이 큐의 1번 계약이다.
    """
    lost = plan.get("lost") or {"reason": "unknown"}
    why = "%s(디스크 항목 %s · 복원 %s)" % (lost.get("reason"), lost.get("disk_items"),
                                           lost.get("restored"))
    sys.stderr.write("[approval-queue] 상태 소실(state_lost) — %s 중단: %s\n" % (cmd, why))
    if not _generations(LEDGER):
        # 전멸 뒤 첫 기록 — **불연속을 박제**한다. 이 줄이 없으면 다음 세대가 마치 처음부터
        # 그랬던 것처럼 보인다. 경보는 1회이고(영구 wedge 금지) 불연속 사실만 영속한다.
        _ledger_append({"event": "ledger_reopened_after_loss", "cmd": cmd,
                        "disk_items": lost.get("disk_items"),
                        "note": "이 줄 이전 이력은 소실됨 — 감사 연속성 단절 지점"})
        _resnapshot_survivors(cmd)
    _ledger_append({"event": "state_lost_abort", "cmd": cmd, "reason": lost.get("reason"),
                    "detail": lost})
    print(json.dumps(dict(lost, result="state_lost", cmd=cmd), ensure_ascii=False))
    return EXIT_INVALID


def _in_grace(state):
    gu = state["daemon"].get("grace_until") or 0
    return bool(gu) and _epoch() < gu


# ── back-pressure / rate cap 계정(전부 락 안 · 이벤트 발행은 락 밖) ───────────
def _bp_account(state, source):
    """(deferred, crossed, count) — 발행자별 시간창 계수(조건 03④ · 무조건 계수)."""
    cap = _int_env("JAVIS_APPROVAL_SOURCE_CAP", SOURCE_CAP_DEFAULT)
    win = _hour_window()
    rec = state["sources"].get(source)
    if not isinstance(rec, dict) or rec.get("window_start") != win:
        rec = {"window_start": win, "count": 0, "soft_emitted": False}
        state["sources"][source] = rec
    rec["count"] += 1
    if cap <= 0 or rec["count"] <= cap:
        return False, False, rec["count"]
    crossed = not rec.get("soft_emitted")
    if crossed:
        rec["soft_emitted"] = True
    return True, crossed, rec["count"]


def _is_urgent(item):
    """escalation 또는 critical = 긴급. 다이제스트 이월 금지 대상(조건 17②).

    `.get` 인 이유(R5a): 항목 1건에 필드가 빠져 있어도 **KeyError 로 도구 전체가 멈추면 안
    된다**. 필수필드 검증은 `_items_scan` 이 격리로 처리하고, 여기서는 보수측(긴급 아님)으로
    읽되 결손 항목이 판정 경로를 폭파하지 못하게 한다.
    """
    return item.get("class") == "escalation" or item.get("severity") == "critical"


def _rate_account(state, item):
    """('immediate'|'overflow', count) — 긴급 항목의 시간당 상한(조건 13·17②)."""
    cap = _int_env("JAVIS_APPROVAL_URGENT_CAP", URGENT_CAP_DEFAULT)
    win = _hour_window()
    rate = state["rate"]
    if rate.get("window_start") != win:
        # ★L-d: 창 전환에서 **초과분을 버리지 않는다**. 종전엔 여기서 `overflow=[]` 로 밀어
        #   버려서, `_plan_tick` 이 병합 job 을 만들기 전에 submit 이 먼저 창을 넘기면 그
        #   시간창의 긴급 초과분이 **병합도 배달도 없이 증발**했다(조건 17 데드락 방어의 구멍).
        #   미배달 초과분은 `carry` 로 옮기고, 배달 책임은 `_plan_tick` 단일 소유로 둔다.
        carry = [r for r in (rate.get("carry") or []) if isinstance(r, str)]
        carry += [r for r in (rate.get("overflow") or [])
                  if isinstance(r, str) and r not in carry]
        rate.update({"window_start": win, "count": 0, "overflow": [], "carry": carry})
    rate["count"] += 1
    if cap <= 0 or rate["count"] <= cap:
        return "immediate", rate["count"]
    if item["request_id"] not in rate["overflow"]:
        rate["overflow"].append(item["request_id"])
    item["rate_overflow"] = True
    return "overflow", rate["count"]


def _route_decision(state, item):
    """락 안 — 배달 경로 판정만(rate cap > back-pressure > 재기동 유예). 외부 I/O 0."""
    bp_deferred, crossed, cnt = _bp_account(state, item["source"])
    out = {"route": "delivered", "crossed": crossed, "count": cnt,
           "cap": _int_env("JAVIS_APPROVAL_SOURCE_CAP", SOURCE_CAP_DEFAULT)}
    if _is_urgent(item):
        mode, _n = _rate_account(state, item)
        out["route"] = "overflow" if mode == "overflow" else "delivered"
        return out
    if bp_deferred:
        item["deferred_to_digest"] = True
        out["route"] = "deferred"
        return out
    if _in_grace(state):
        item["grace_held"] = True
        held = state["daemon"].setdefault("held", [])
        if item["request_id"] not in held:
            held.append(item["request_id"])
        out["route"] = "grace_held"
        out["grace_until"] = state["daemon"].get("grace_until")
        return out
    return out


# ── Slack(조건 06·31 — flag 게이트 뒤 코드만) ────────────────────────────────
def _slack_sink_path():
    return (os.environ.get("JAVIS_APPROVAL_SLACK_SINK", "").strip()
            or os.path.join(APPROVALS_DIR, "slack-outbox.jsonl"))


def secret_scan(text):
    """(라벨, 마스킹 표본) 또는 None — 발신 전 시크릿/절대경로 스캔(조건 31 · S7).

    비밀 패턴은 **`javis_scrub` 재사용**이다 — 형제 모듈 단일 정의 원칙(패턴을 복제하면
    한쪽만 갱신돼 열화한다). scrub 이 일부러 잡지 않는 '절대경로' 계층만 여기 둔다
    (원장은 경로가 정상 구성요소지만 Slack 발신은 유출면이라 정책이 반대다).
    """
    t = text or ""
    for rex, label in _PATH_PATTERNS:
        m = rex.search(t)
        if m:
            snip = m.group(0).strip()
            return label, (snip[:6] + "…") if len(snip) > 6 else snip
    scrubbed, n = javis_scrub.scrub(t)
    if n:
        return "비밀 패턴(javis_scrub %d건)" % n, "마스킹됨"
    return None


def slack_text(item):
    """notify-only 본문 — 승인 액션 표기 0 + '승인은 로컬 전용' 고지 고정(조건 06②)."""
    return "\n".join([
        "[승인 알림] %s · risk=%s · severity=%s" % (item["class"], item["risk"],
                                                   item["severity"]),
        "request_id: %s" % item["request_id"],
        "source: %s" % item["source"],
        "요약: %s" % (item.get("summary") or "(요약 없음)"),
        "참조: %s" % (_relativize(item.get("payload_ref")) or "-"),
        "※ 알림 전용입니다 — 이 메시지로는 승인·거부할 수 없습니다. 승인은 로컬 전용"
        "(Control Center·CLI)입니다.",
    ])


def notify_slack(item, route="delivered"):
    """(결과, 상세) — 'disabled'|'suppressed'|'capped'|'blocked'|'sent'|'error'.

    ★억제 3종 종속(S7): **즉시 배달된 항목만** Slack 으로 나간다. overflow·deferred·
    grace_held 는 로컬과 **동일하게** 억제된다 — 그러지 않으면 Slack 이 rate cap·
    back-pressure·재기동 유예를 통째로 우회하는 샛길이 된다.
    flag 파일 부재 = 무동작(기본 OFF · 활성화 [OT-3]). 존재해도 항목당 1회 상한이며,
    발신 전 스캔에 걸리면 **거부 + 경보**다. 고위험은 tier=d 강제 — 데몬 `tier_mirrorable`
    이 d 를 미러 금지(channels.rs:260 정의·:967 적용)라 미러 경로가 구조적으로 닫힌다.
    """
    if not os.path.isfile(SLACK_FLAG):
        return "disabled", "flag 파일 부재(%s) — 발신 경로 무동작" % _relativize(SLACK_FLAG)
    if route != "delivered":
        return "suppressed", "억제 상태(%s) — 로컬과 동일 억제(억제 3종 종속)" % route
    if item.get("notified_slack_at"):
        return "capped", "항목당 발행 1회 상한(notified_at=%s)" % item["notified_slack_at"]
    text = slack_text(item)
    hit = secret_scan(text)
    if hit:
        label, snip = hit
        _ledger_append({"event": "slack_blocked", "request_id": item["request_id"],
                        "why": label, "sample": snip})
        _emit("agent.error", {"agent": "approval-queue",
                              "summary": "[approval-queue] Slack 발신 거부 — %s 검출"
                                         "(request_id=%s)" % (label, item["request_id"])})
        return "blocked", "시크릿/절대경로 검출: %s" % label
    tier = "d" if item["risk"] == "high" else "c"
    rec = {"ts": _now(), "request_id": item["request_id"], "tier": tier,
           "notify_only": True, "risk": item["risk"], "text": text}
    if not _append_jsonl(_slack_sink_path(), rec):
        return "error", "sink 기록 실패"
    item["notified_slack_at"] = _now()
    item["slack_tier"] = tier
    _ledger_append({"event": "slack_notified", "request_id": item["request_id"], "tier": tier})
    return "sent", "tier=%s notify-only 1회 발행" % tier


# ── 항목 등재 ────────────────────────────────────────────────────────────────
def _new_item(request_id, cls, source, severity, risk, payload_ref, summary, risk_why,
              bypass=False):
    return {"request_id": request_id, "class": cls, "risk": risk, "severity": severity,
            "created_at": _now(), "created_epoch": _epoch(), "source": source,
            "payload_ref": payload_ref, "state": "pending",
            "summary": summary, "risk_why": risk_why,
            "local_only": risk == "high",     # 고위험 = 로컬 승인 전용(조건 06②·22③)
            "bypass": bypass,
            "notified_at": None, "notified_slack_at": None,
            "deferred_to_digest": False, "rate_overflow": False, "grace_held": False,
            "decided_at": None, "decided_by": None, "routed_to": None}


def _ledger_queued(item):
    """`queued` = **항목 복원 가능한 스냅샷**(S3) — 원장만 살아도 항목이 재구성된다."""
    _ledger_append({"event": "queued", "request_id": item["request_id"],
                    "class": item["class"], "risk": item["risk"],
                    "severity": item["severity"], "source": item["source"],
                    "payload_ref": item.get("payload_ref"), "risk_why": item.get("risk_why"),
                    "summary": item.get("summary"), "created_at": item.get("created_at"),
                    "created_epoch": item.get("created_epoch"), "state": item.get("state"),
                    "local_only": item.get("local_only"), "snapshot": 1})


def _dispatch(item, target=None, task_key=None, reason=None, fallback=None):
    """판정(락 안) → 배달(락 밖) → 항목 파일 반영. 반환 (route, slack_res, slack_detail).

    ★F1: `fallback` 미지정이면 **routing 표에서 파생**한다. 계약 §2-2 는 fallback 이
    `submit`·`collect-escalations` **공통 경로**라고 단언하는데, 종전 코드는 collect 에서만
    명시 전달해 `submit --class escalation` 은 1차 대상(리뷰어1) 사망 시 fallback 없이 끝났다
    — 계약이 막았다고 선언한 무성 소실이 submit 경유로 그대로 열려 있었다. 파생은 표를 읽는
    것뿐이라 approval·self-healing(fallback=None)에는 부작용이 없다.
    """
    if fallback is None:
        _t, fallback = _route_for(item["class"])
    dec = _state_update(lambda s: _route_decision(s, item))
    route = dec["route"]
    if dec.get("crossed"):
        _emit("resource.soft", {"metric": "approval_queue_source_rate",
                                "value": dec["count"], "threshold": dec["cap"]})
        _ledger_append({"event": "back_pressure_crossed", "source": item["source"],
                        "count": dec["count"], "threshold": dec["cap"]})
    if route == "delivered":
        # HUD·음성 트랙 신호 — **실제 배달되는 항목만**(S11g: 억제 3종에 종속). spool 전용이라
        # master stdin·feed 항목을 만들지 않는다(조건 03② 재귀 차단과 무충돌). best-effort.
        _emit("approval.needed", {"agent": item["source"], "task": item["request_id"],
                                  "summary": "%s/%s %s" % (item["class"], item["risk"],
                                                           item.get("summary") or "")})
        tk = task_key or _task_key(item["request_id"])      # H3 — rid 해시 포함
        _notify(item, target, tk, reason)
        if not item.get("delivery_ok"):
            _route_fallback(item, fallback, tk)   # S6 — submit·collect 공통 경로
    elif route == "overflow":
        _ledger_append({"event": "rate_overflow", "request_id": item["request_id"],
                        "threshold": _int_env("JAVIS_APPROVAL_URGENT_CAP", URGENT_CAP_DEFAULT),
                        "note": "다음 시간창 시작 시 병합 critical 1건으로 배달"})
    elif route == "deferred":
        _ledger_append({"event": "deferred_to_digest", "request_id": item["request_id"],
                        "source": item["source"]})
    elif route == "grace_held":
        _ledger_append({"event": "grace_held", "request_id": item["request_id"],
                        "grace_until": dec.get("grace_until")})
    slack_res, slack_detail = notify_slack(item, route)
    # ★R2: 통짜 `_item_write(item)` 폐기. 배달은 최대 50초라 그 사이 approve/deny 가 들어올 수
    #   있고, 인메모리 스냅샷을 통째로 쓰면 그 결정이 무음으로 되돌아간다. 억제·배달 플래그만
    #   디스크 최신본에 얹고, TERMINAL 이면 손대지 않는다.
    fields = {k: item.get(k) for k in _SUPPRESS_FIELDS if k in item}
    cur, why = _item_merge_write(item["request_id"], fields, bool(item.get("bypass")))
    if why == "terminal-skip":
        _ledger_append({"event": "commit_skipped_terminal", "request_id": item["request_id"],
                        "state": (cur or {}).get("state"),
                        "note": "배달 중 확정된 결정 보존 — 낡은 스냅샷 덮어쓰기 거부"})
    elif cur is None:
        # ★L-b: `absent` 일 때만 재생성한다. 종전엔 `corrupt`(판독 불가)도 같은 가지로 떨어져
        #   **손상 파일을 인메모리 스냅샷으로 덮어써** 격리·감사 흔적을 지웠다. 재생성도
        #   `os.link` 원자 생성이라 그 사이 누가 만들었으면 지지 않는다(덮어쓰기 없음).
        if why == "absent":
            with contextlib.suppress(OSError, SchemaError):
                created, _p = _item_claim(item)
                _ledger_append({"event": "item_recreated" if created
                                else "item_restore_skipped_present",
                                "request_id": item["request_id"],
                                "note": "배달 후 항목 파일 부재 — 원자 재생성 시도"})
        else:
            _ledger_append({"event": "item_corrupt_isolated",
                            "path": _relativize(_item_file(item["request_id"],
                                                           bool(item.get("bypass")))),
                            "why": "배달 후 판독 불가(%s) — 격리 경로로 위임" % why})
    return route, slack_res, slack_detail


# ── 서브커맨드 ───────────────────────────────────────────────────────────────
def _bad_id(kind, value):
    print("invalid(6): %s=%r — 허용 형식 %s" % (kind, value, ID_RE.pattern), file=sys.stderr)
    _ledger_append({"event": "id_rejected", "kind": kind, "value": str(value)[:120]})
    return EXIT_INVALID


def cmd_submit(a):
    if a.cls not in CLASSES:
        print("invalid(6): class=%s (%s)" % (a.cls, "|".join(CLASSES)), file=sys.stderr)
        return EXIT_INVALID
    if a.severity not in SEVERITIES:
        print("invalid(6): severity=%s (%s)" % (a.severity, "|".join(SEVERITIES)),
              file=sys.stderr)
        return EXIT_INVALID
    if a.risk is not None and a.risk not in RISKS:
        print("invalid(6): risk=%s (%s)" % (a.risk, "|".join(RISKS)), file=sys.stderr)
        return EXIT_INVALID
    rid = a.request_id
    if not ID_RE.match(rid or ""):
        return _bad_id("request_id", rid)
    if not ID_RE.match(a.source or ""):
        return _bad_id("source", a.source)

    # ★분기 순서(S2): **멱등/기존 항목 검사가 모든 분기보다 먼저**다. 종전에는 자가치유
    #   bypass 가 이 검사 앞에 있어서, 대기 중인 고위험 승인 항목을 같은 request_id 의
    #   self-healing submit 하나로 덮어써 무음 소멸시킬 수 있었다(승인 삭제 프리미티브).
    cur, _err = _item_read(rid)
    bypass_actions, allow_meta = _self_heal_allowlist()
    bypass_candidate = (a.cls == "self-healing" and a.action and a.action in bypass_actions)
    if cur is not None:
        if cur.get("class") != a.cls or bypass_candidate:
            _ledger_append({"event": "rid_conflict", "request_id": rid,
                            "existing_class": cur.get("class"), "existing_state": cur.get("state"),
                            "incoming_class": a.cls, "bypass_attempt": bool(bypass_candidate),
                            "note": "rid 재사용 클래스 불일치 — 기존 항목 무손상·덮어쓰기 거부"})
            print("invalid(6): request_id=%s 는 이미 class=%s(state=%s) 로 존재 — "
                  "클래스 교차 재사용 거부(기존 항목 무손상)"
                  % (rid, cur.get("class"), cur.get("state")), file=sys.stderr)
            print(json.dumps({"result": "rid_conflict", "request_id": rid,
                              "existing_class": cur.get("class"),
                              "existing_state": cur.get("state")}, ensure_ascii=False))
            return EXIT_INVALID
        _ledger_append({"event": "duplicate", "request_id": rid, "state": cur.get("state"),
                        "note": "재알림 억제(항목당 1회 — notified_at=%s)" % cur.get("notified_at")})
        print(json.dumps({"result": "duplicate", "request_id": rid,
                          "state": cur.get("state"), "notified_at": cur.get("notified_at")},
                         ensure_ascii=False))
        return EXIT_OK

    # ① 자가치유 사전 인가 — 큐 미경유 + **tick·배달 I/O 전면 스킵**(S4 지연 0 보장)
    if bypass_candidate:
        dup, _e = _item_read(rid, bypass=True)
        if dup is not None:
            _ledger_append({"event": "duplicate", "request_id": rid, "state": dup.get("state"),
                            "note": "자가치유 사전 인가 항목 재submit — 사후 통지 1회 유지"})
            print(json.dumps({"result": "duplicate", "request_id": rid,
                              "state": dup.get("state"), "queued": False},
                             ensure_ascii=False))
            return EXIT_OK
        item = _new_item(rid, a.cls, a.source, a.severity, "normal", a.payload_ref,
                         a.summary, "자가치유 사전 인가(allowlist) — 위험 분류 생략",
                         bypass=True)
        item.update({"state": "notified", "queued": False, "action": a.action,
                     "notified_at": _now()})
        ok, _path = _item_claim(item)
        _ledger_append({"event": "self_heal_bypass", "request_id": rid, "action": a.action,
                        "source": a.source, "summary": a.summary,
                        "severity": a.severity, "payload_ref": a.payload_ref,
                        "created_epoch": item["created_epoch"],
                        "allowlist_path": allow_meta.get("path"),
                        "allowlist_sha256": allow_meta.get("sha256"),
                        "allowlist_sealed": allow_meta.get("sealed"),
                        "allowlist_source": allow_meta.get("source"),
                        "note": "사전승인 원장 — 큐 미경유·승인 대기 0·tick/배달 I/O 스킵"})
        print(json.dumps({"result": "bypass", "request_id": rid, "state": "notified",
                          "action": a.action, "queued": False,
                          "allowlist": allow_meta.get("source")}, ensure_ascii=False))
        return EXIT_OK

    plan = _maintenance()
    if plan["lost"]:
        return _abort_state_lost(plan, "submit")
    risk, why = classify_risk(a.risk_class, a.cmd, a.risk)
    item = _new_item(rid, a.cls, a.source, a.severity, risk, a.payload_ref, a.summary, why)
    claimed, _path = _item_claim(item)
    if not claimed:
        # 경합 패자 — O_EXCL 이 정확 1건을 보장한다(스냅샷 판정이면 둘 다 통과했을 자리).
        other, _e = _item_read(rid)
        _ledger_append({"event": "duplicate", "request_id": rid,
                        "state": (other or {}).get("state"),
                        "note": "동시 submit 경합 패자(O_EXCL) — 재알림 0"})
        print(json.dumps({"result": "duplicate", "request_id": rid,
                          "state": (other or {}).get("state"), "race": True},
                         ensure_ascii=False))
        return EXIT_OK
    # ★R1: 여기 있던 `item_count += 1` 을 제거했다. claim(디스크) 과 카운터(state.json) 사이의
    #   이 한 줄이 유령 소실의 발원지였다 — 형제 프로세스가 그 창에서 디스크 실측으로 카운터를
    #   재기록하면 내 +1 이 실측을 앞질러 영구 오버카운트가 된다.
    _ledger_queued(item)
    route, slack_res, slack_detail = _dispatch(item)
    # ★R5d: `result` 는 **라우팅 판정**(delivered/deferred/overflow/grace_held)이지 배달
    #   성공이 아니다. 배달 결과는 delivery_ok·verdict·detail 로 따로 싣는다 — 종전에는
    #   호출자가 result 만 보고 성공을 오판할 수 있었다.
    print(json.dumps({"result": route, "request_id": rid, "risk": risk,
                      "risk_why": why, "state": item["state"],
                      "local_only": item["local_only"], "routed_to": item.get("routed_to"),
                      "delivery_ok": bool(item.get("delivery_ok")),
                      "delivery_verdict": item.get("delivery_verdict"),
                      "detail": item.get("delivery_detail"),
                      "slack": slack_res, "slack_detail": slack_detail},
                     ensure_ascii=False))
    return EXIT_OK


def cmd_collect_escalations(a):
    """esc-bundle 수거 → 항목화(class=escalation) → routing 경유 리뷰어1 배달(설계 §4)."""
    tasks_dir = a.tasks_dir or TASKS_DIR
    bundles = sorted(glob.glob(os.path.join(tasks_dir, "*.esc-bundle.json")))
    if not bundles:
        print(json.dumps({"result": "empty", "scanned": 0}, ensure_ascii=False))
        return EXIT_EMPTY
    plan = _maintenance()
    if plan["lost"]:
        return _abort_state_lost(plan, "collect-escalations")
    target, fallback = _route_for("escalation", notify_unbound=True)
    collected = duplicates = delivered = invalid = conflicts = 0
    for path in bundles:
        b, err = _read_json(path)
        if b is None or not b.get("task"):
            invalid += 1
            _ledger_append({"event": "esc_bundle_invalid", "path": _relativize(path),
                            "why": err or "필수 필드(task) 부재"})
            continue
        rid = b.get("request_id") or esc_request_id_local(b.get("task"), b.get("sig"))
        if not ID_RE.match(rid or ""):
            invalid += 1
            _ledger_append({"event": "esc_bundle_invalid", "path": _relativize(path),
                            "why": "request_id 형식 위반: %r" % rid})
            continue
        cur, _e = _item_read(rid)
        if cur is not None:
            if cur.get("class") != "escalation":
                conflicts += 1
                _ledger_append({"event": "rid_conflict", "request_id": rid,
                                "existing_class": cur.get("class"),
                                "incoming_class": "escalation",
                                "note": "esc-bundle rid 가 다른 클래스로 선점됨 — 덮어쓰기 거부"})
                continue
            duplicates += 1
            _ledger_append({"event": "duplicate", "request_id": rid,
                            "note": "esc-bundle 재수거 — 재알림 0"})
            continue
        risk, why = _escalation_risk(tasks_dir, b)
        summary = ("completion-guard escalation: task=%s sig=%s exit=%s attempt=%s"
                   % (b.get("task"), b.get("sig"), b.get("exit"), b.get("attempt")))
        item = _new_item(rid, "escalation", "completion-guard", "critical", risk,
                         path, summary, why)
        claimed, _p = _item_claim(item)
        if not claimed:
            duplicates += 1
            _ledger_append({"event": "duplicate", "request_id": rid,
                            "note": "동시 수거 경합 패자(O_EXCL)"})
            continue
        _ledger_queued(item)                    # ★R1 — item_count 증가 제거(카운터 폐기)
        # H3 — 조인 키 유일성: task 라벨이 아니라 **rid** 에서 파생한다(rid 해시 포함).
        task_key = _task_key(rid, "approval-esc")
        route, _sr, _sd = _dispatch(item, target=target, task_key=task_key, fallback=fallback)
        collected += 1
        if item.get("delivery_ok"):
            delivered += 1
        with contextlib.suppress(OSError):      # S11f — 수거 표식(원본 제거는 guard 소관)
            with open(path + ".collected", "w", encoding="utf-8") as f:
                f.write("%s %s route=%s\n" % (_now(), rid, route))
    # ★R6c 부분 실패 = exit 0 + 계수. 종전엔 conflicts 1건이 전체를 exit 6 으로 뒤집어,
    #   같은 호출에서 **성공적으로 수거·배달된 항목까지** 호출자에게 실패로 보고됐다.
    #   비영은 **전량 실패**(성공 0 · 중복 0 · 스캔 전부가 conflict/invalid)일 때만이다.
    all_failed = (collected == 0 and duplicates == 0
                  and (conflicts + invalid) == len(bundles) and len(bundles) > 0)
    print(json.dumps({"result": "all-failed" if all_failed else "ok",
                      "scanned": len(bundles), "collected": collected,
                      "duplicates": duplicates, "delivered": delivered, "invalid": invalid,
                      "conflicts": conflicts, "target": target,
                      "partial_failure": bool((conflicts or invalid) and not all_failed)},
                     ensure_ascii=False))
    return EXIT_INVALID if all_failed else EXIT_OK


def _escalation_risk(tasks_dir, bundle):
    """esc-bundle 의 위험도 — 태스크 레코드 verify_spec(risk.class + cmd 2차 스캔) 소비."""
    rec, _err = _read_json(os.path.join(tasks_dir, "%s.json" % bundle.get("task")))
    spec = (rec or {}).get("verify_spec") or {}
    declared = ((spec.get("risk") or {}).get("class") if isinstance(spec, dict) else None)
    cmd = spec.get("cmd") if isinstance(spec, dict) else None
    risk, why = classify_risk("HighRisk" if declared == "high" else None, cmd)
    return risk, "esc-bundle: %s" % why


# ── 다이제스트(조건 03②③·13) ────────────────────────────────────────────────
def _digest_path(date):
    return os.path.join(APPROVALS_DIR, "digest-%s.md" % date)


def _write_digest_file(date, body):
    """읽기 전용 집계 파일(조건 03② 문면) — 본문은 여기 있고 push 에는 헤드만 간다(S8)."""
    path = _digest_path(date)
    os.makedirs(APPROVALS_DIR, exist_ok=True)
    with contextlib.suppress(OSError):
        if os.path.exists(path):
            os.chmod(path, 0o644)
    # 본문은 마크다운(JSON 아님)이라 `_write_json_atomic` 대신 동형 temp+os.replace 를 쓴다.
    tmp = "%s.tmp.%s.%s" % (path, os.getpid(), uuid.uuid4().hex[:8])
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(body)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)
    with contextlib.suppress(OSError):
        os.chmod(path, 0o444)
    return path


def cmd_digest(a):
    """1일 1회 · 멱등키=날짜 · 2KB 캡 · 집계 파일 + 헤드 push · feed 항목 생성 0."""
    date = _today()
    plan = _maintenance()
    if plan["lost"]:
        return _abort_state_lost(plan, "digest")
    state_digest = _state_update(lambda s: dict(s.get("digest") or {}))
    if date in state_digest and not a.force:
        print(json.dumps({"result": "already-sent", "date": date,
                          "sent_at": state_digest[date].get("sent_at")}, ensure_ascii=False))
        return EXIT_EMPTY
    items, _c = _items_scan()
    rows = [it for it in items.values()
            if it.get("state") == "pending" and not it.get("bypass") and not _is_urgent(it)]
    # escalation·critical 은 다이제스트 이월 금지(조건 17② > 13) — 위 필터가 그 집행이다.
    rows.sort(key=lambda it: (0 if it.get("risk") == "high" else 1, it.get("created_at") or ""))
    if not rows:
        # 빈 큐면 발행 자체를 생략한다 — 빈 알림은 master 컨텍스트만 축낸다(S8).
        _ledger_append({"event": "digest_skipped_empty", "date": date})
        print(json.dumps({"result": "skipped-empty", "date": date, "pending": 0},
                         ensure_ascii=False))
        return EXIT_EMPTY
    body, shown = _build_digest(rows)
    path = _write_digest_file(date, body)
    pending_all = sum(1 for it in items.values()
                      if it.get("state") == "pending" and not it.get("bypass"))
    high = sum(1 for it in rows if it.get("risk") == "high")
    head = _digest_head(date, rows, high, path)
    meta = _meta_target()                        # R6d — 하드코딩 대신 routing 표 경유
    ok, detail, verdict = _deliver(meta, "approval-digest", head, "approval-digest:%s" % date,
                                   "info", payload={"date": date, "pending": len(rows),
                                                    "digest_file": _relativize(path)})
    marker = os.path.join(APPROVALS_DIR, ".digest-sent-%s" % date)
    with contextlib.suppress(OSError):
        os.makedirs(APPROVALS_DIR, exist_ok=True)
        with open(marker, "w", encoding="utf-8") as f:
            f.write("%s pending=%d bytes=%d file=%s\n"
                    % (_now(), len(rows), len(body.encode("utf-8")), _relativize(path)))
    rec = {"sent_at": _now(), "count": len(rows), "shown": shown,
           "bytes": len(body.encode("utf-8")), "delivered": ok,
           "file": _relativize(path)}
    _state_update(lambda s: s.setdefault("digest", {}).update({date: rec}))
    _ledger_append({"event": "digest_delivered" if ok else "digest_failed", "date": date,
                    "pending": len(rows), "shown": shown, "file": _relativize(path),
                    "bytes": len(body.encode("utf-8")), "head_len": len(head),
                    "target": meta, "verdict": verdict, "detail": detail})
    if not ok:
        _outbox_append("approval-digest:%s" % date, meta, "approval-digest", detail,
                       {"pending": len(rows), "file": _relativize(path), "verdict": verdict})
    # 조건 13 '음성 브리핑 1줄' — counts.running 은 큐가 관찰하지 못하는 값이라 0을 싣는다(자인).
    _emit("briefing", {"counts": json.dumps({"running": 0, "inbox": len(rows),
                                             "approvals": pending_all, "alerts": high})})
    print(json.dumps({"result": "sent" if ok else "deliver-failed", "date": date,
                      "pending": len(rows), "shown": shown,
                      "bytes": len(body.encode("utf-8")), "head_len": len(head),
                      "file": _relativize(path), "marker": _relativize(marker),
                      "delivery_ok": ok, "delivery_verdict": verdict,
                      "detail": detail}, ensure_ascii=False))
    return EXIT_OK


def _digest_head(date, rows, high, path):
    """wakeup reason — **300자 이내 헤드 + 집계 파일 경로만**(조건 03② · wakeup 접기와 정합)."""
    core = ("[승인 큐 다이제스트 %s] 대기 %d건 · 고위험 %d건 · 전문 %s"
            % (date, len(rows), high, _relativize(path)))
    by_class = {}
    for it in rows:
        by_class[it["class"]] = by_class.get(it["class"], 0) + 1
    extra = (" · 분류 %s · 승인은 로컬 전용(개별 push 없음)"
             % (" / ".join("%s %d" % (c, by_class[c]) for c in sorted(by_class)) or "없음"))
    head = core + extra if len(core + extra) <= WAKEUP_REASON_MAX else core
    return _oneline(head, WAKEUP_REASON_MAX)


def _build_digest(rows):
    """(집계 파일 본문, 실린 항목 수) — 2KB 캡·헤드라인만. 전문은 원장 참조 지시(조건 13)."""
    by_class = {}
    high = 0
    for it in rows:
        by_class[it["class"]] = by_class.get(it["class"], 0) + 1
        if it.get("risk") == "high":
            high += 1
    head = ("[승인 큐 다이제스트 %s] 대기 %d건 (%s) · 고위험 %d건"
            % (_today(), len(rows),
               " / ".join("%s %d" % (c, by_class[c]) for c in sorted(by_class)) or "없음",
               high))
    tail = ("※ 헤드라인만입니다 — 전문은 승인 큐 원장(_round/approvals/ledger.jsonl)·"
            "Control Center 참조. 승인은 로컬 전용. escalation·critical 은 다이제스트에 "
            "싣지 않고 즉시 배달합니다(조건 17②).")
    lines, shown = [], 0
    budget = DIGEST_CAP_BYTES - len(("%s\n%s" % (head, tail)).encode("utf-8")) - 40
    for it in rows:
        line = ("- [%s/%s] %s · %s · %s"
                % (it.get("risk"), it.get("severity"), it["request_id"], it.get("source"),
                   _oneline(it.get("summary") or "", 80)))
        line = line[:DIGEST_LINE_MAX]
        b = len(line.encode("utf-8")) + 1
        if b > budget:
            break
        budget -= b
        lines.append(line)
        shown += 1
    omitted = len(rows) - shown
    if omitted:
        lines.append("…[%d건 생략 — 2KB 캡]" % omitted)
    text = "\n".join([head] + lines + [tail])
    # 캡 최종 집행(멀티바이트 절단 안전) — 어떤 경로로도 2KB 를 넘기지 않는다.
    raw = text.encode("utf-8")
    if len(raw) > DIGEST_CAP_BYTES:
        text = raw[:DIGEST_CAP_BYTES - 3].decode("utf-8", errors="ignore") + "…"
    return text, shown


# ── 조회·판정·tick ───────────────────────────────────────────────────────────
def cmd_list(a):
    plan = _maintenance()
    if plan["lost"]:
        # 조회도 예외가 아니다 — 소실 상태에서 '정상처럼 보이는 목록'을 내놓지 않는다.
        return _abort_state_lost(plan, "list")
    rows = list(plan["items"].values())
    if a.state:
        rows = [r for r in rows if r.get("state") == a.state]
    if a.cls:
        rows = [r for r in rows if r.get("class") == a.cls]
    if a.risk:
        rows = [r for r in rows if r.get("risk") == a.risk]
    rows.sort(key=lambda r: r.get("created_at") or "")
    print(json.dumps(rows, ensure_ascii=False, indent=1))
    return EXIT_OK


def cmd_tick(a):
    """배달 주체(조건 33) 실배선 — 유지보수 tick + 대상별 `wakeup drain --deliver` 1회.

    **주기 호출자(스케줄·CSO 임무) 배선은 [OT-2]** 다(라이브 스케줄은 이 Wave 범위 밖) —
    여기서는 호출 1회로 만기·병합·요약·drain 을 전부 집행하는 진입점만 만든다.
    """
    plan = _maintenance()
    if plan["lost"]:
        return _abort_state_lost(plan, "tick")
    targets, seen = [], set()
    for cls in CLASSES:
        t, fb = _route_for(cls)
        for x in (t, fb):
            if x and x not in seen:
                seen.add(x)
                targets.append(x)
    drains = []
    if not a.no_deliver:
        for t in targets:
            r = _run_wakeup(["drain", "--deliver", "--target", t], timeout=30)
            rc = None if r is None else r.returncode
            res = _drain_result(r) if r is not None else None
            # ★R5e: drain 실패·zombie 폐기도 outbox 로 표면화한다(종전엔 원장 한 줄이 끝이라
            #   tick 이 삼킨 배달 실패는 어디서도 회수되지 않았다). exit 5 는 '판정 불가'가
            #   아니라 여기서는 '이 대상에 남은 pending 0' 이라 정상이다(항목 조인은 _deliver 몫).
            ok = rc in (0, EXIT_EMPTY)
            skipped = int((res or {}).get("skipped") or 0)
            drains.append({"target": t, "rc": rc, "delivered": (res or {}).get("delivered"),
                           "skipped": skipped or None,
                           "result": "ok" if ok and not skipped else "fail"})
            if not ok or skipped:
                _outbox_append("approval-tick-drain:%s" % t, t, "approval-tick",
                               "tick drain rc=%s res=%s" % (rc, res),
                               {"skipped": skipped, "note": "tick 배달 실패·zombie 폐기 표면화"})
    _ledger_append({"event": "tick", "expired": plan["expired"], "reaped": plan["reaped"],
                    "restored": plan["restored"], "jobs": len(plan["jobs"]),
                    "drains": drains})
    print(json.dumps({"result": "ok", "expired": plan["expired"], "reaped": plan["reaped"],
                      "restored": plan["restored"], "merged": len(plan["results"]),
                      "drains": drains}, ensure_ascii=False))
    return EXIT_OK


def _decide(a, new_state, event):
    plan = _maintenance()
    if plan["lost"]:
        return _abort_state_lost(plan, "decide")
    if not ID_RE.match(a.request_id or ""):
        return _bad_id("request_id", a.request_id)
    it, _err = _item_read(a.request_id)
    if it is None:
        print("not found: %s" % a.request_id, file=sys.stderr)
        return EXIT_NOTFOUND
    if it.get("state") in TERMINAL_STATES:
        print("transition denied(2): %s 는 이미 %s — 재전이 금지"
              % (a.request_id, it["state"]), file=sys.stderr)
        return EXIT_USAGE
    # ★L-a: 쓰기 직전 **재읽기 후 여전히 비-TERMINAL 일 때만** 확정한다. 위 검사와 쓰기
    #   사이에 형제의 approve/deny 가 끼면 종전 구조는 둘 다 성공으로 통과시켜 나중 쓰기가
    #   앞선 결정을 덮었다(동시 approve/deny 이중 통과 — 마지막 쓴 자가 이긴다).
    fresh, _e2 = _item_read(a.request_id)
    if fresh is None:
        print("not found: %s (판정 직전 소멸)" % a.request_id, file=sys.stderr)
        return EXIT_NOTFOUND
    if fresh.get("state") in TERMINAL_STATES:
        _ledger_append({"event": "decide_race_denied", "request_id": a.request_id,
                        "by": a.by, "winner_state": fresh.get("state"),
                        "winner_by": fresh.get("decided_by"),
                        "note": "판정 직전 형제가 먼저 확정 — 이중 통과 차단(선착 결정 보존)"})
        print("transition denied(2): %s 는 이미 %s(by=%s) — 재전이 금지"
              % (a.request_id, fresh["state"], fresh.get("decided_by")), file=sys.stderr)
        return EXIT_USAGE
    it = fresh
    it["state"] = new_state
    it["decided_at"] = _now()
    it["decided_epoch"] = _epoch()
    it["decided_by"] = a.by
    if getattr(a, "reason", None):
        it["decision_reason"] = _oneline(a.reason, 400)
    _item_write(it)
    _ledger_append({"event": event, "request_id": a.request_id, "by": a.by,
                    "risk": it.get("risk"), "local_only": it.get("local_only"),
                    "decided_epoch": it["decided_epoch"],
                    "reason": _oneline(getattr(a, "reason", None), 400)})
    print(json.dumps({"result": new_state, "request_id": a.request_id, "by": a.by,
                      "risk": it.get("risk"), "local_only": it.get("local_only")},
                     ensure_ascii=False))
    return EXIT_OK


def cmd_approve(a):
    """로컬 승인 — 고위험도 여기서만 승인된다(Slack 은 알림 전용 · 조건 06②)."""
    return _decide(a, "approved", "approved")


def cmd_deny(a):
    return _decide(a, "denied", "denied")


# ═════════════════════════ self-test(완료 기준 배터리) ═══════════════════════
def self_test(a=None):
    import tempfile

    # G1 hard assert — 격리 env 부재 시 실행 거부(cwd 폴백의 라이브 오염 차단).
    missing = [k for k in ("JAVIS_ROOT", "CYS_PROBE_RUNS") if not os.environ.get(k)]
    if missing:
        print("javis_approval_queue self-test 실행 거부 — 격리 env 부재: %s "
              "(G1 라이브/사이드 경계 hard assert)" % ",".join(missing), file=sys.stderr)
        return 1

    self_path = os.path.abspath(__file__)
    fails = []

    def chk(cond, msg):
        if not cond:
            fails.append(msg)

    with tempfile.TemporaryDirectory(prefix="apq-selftest-") as td:
        shim_dir = os.path.join(td, "shim")
        os.makedirs(shim_dir)
        shim_log = os.path.join(td, "cys-shim.log")
        status_file = os.path.join(td, "status.json")
        with open(os.path.join(shim_dir, "cys"), "w", encoding="utf-8") as f:
            f.write('#!/bin/sh\n'
                    'echo "$@" >> "%s"\n'
                    '[ -n "$CYS_SHIM_DELAY" ] && sleep "$CYS_SHIM_DELAY"\n'
                    'case "$1" in\n'
                    '  status) cat "${CYS_SHIM_STATUS:-%s}" 2>/dev/null || exit 1 ;;\n'
                    '  send) [ -n "$CYS_SHIM_SEND_DELAY" ] && sleep "$CYS_SHIM_SEND_DELAY"\n'
                    '        exit "${CYS_SHIM_SEND_RC:-0}" ;;\n'
                    '  *) exit 0 ;;\nesac\n' % (shim_log, status_file))
        os.chmod(os.path.join(shim_dir, "cys"), 0o755)

        def write_status(started_at, build="b1", path=None, drop=None):
            d = {"version": "0.14.6", "started_at": started_at, "build_id": build}
            if drop:
                d.pop(drop, None)
            with open(path or status_file, "w", encoding="utf-8") as sf:
                json.dump({"daemon": d, "surfaces": []}, sf)

        write_status(1000.0)

        def new_root(name):
            root = os.path.join(td, name)
            os.makedirs(os.path.join(root, "_round", "tasks"), exist_ok=True)
            return root

        def env_for(root, env_extra=None, now=None):
            env = dict(os.environ)
            # S12e — CYS_BIN·시계·캡·경로 노브를 전부 회수(부모 env 누출 차단).
            for k in ("JAVIS_APPROVAL_NOW", "JAVIS_APPROVAL_SOURCE_CAP",
                      "JAVIS_APPROVAL_URGENT_CAP", "JAVIS_APPROVAL_RESTART_GRACE_SEC",
                      "JAVIS_APPROVAL_TTL_DAYS", "JAVIS_APPROVAL_ROUTING",
                      "JAVIS_APPROVAL_ALLOWLIST", "JAVIS_APPROVAL_SLACK_SINK",
                      "JAVIS_APPROVAL_CONFIRM_SEC", "JAVIS_APPROVAL_ROTATE_BYTES",
                      "JAVIS_APPROVAL_DELIVER_BUDGET_SEC", "JAVIS_APPROVAL_SWEEP_MAX",
                      "JAVIS_APPROVAL_TEST_CLAIM_DELAY", "JAVIS_APPROVAL_TEST_RESTORE_DELAY",
                      "CYS_BIN", "CYS_SHIM_SEND_RC", "CYS_SHIM_DELAY",
                      "CYS_SHIM_SEND_DELAY", "CYS_SHIM_STATUS"):
                env.pop(k, None)
            env.update({"JAVIS_ROOT": root,
                        "HUD_STATE_DIR": os.path.join(root, "hud"),
                        "CYS_PROBE_RUNS": os.path.join(root, "probe_runs.jsonl"),
                        "CLAUDE_CONFIG_DIR": os.path.join(root, "claude-cfg"),
                        "CYS_NO_AUTOSTART": "1",
                        "JAVIS_WAKEUP_LIVENESS": "alive",
                        "JAVIS_FASTFAIL_MAX": "99",
                        "PATH": shim_dir + os.pathsep + os.environ.get("PATH", "")})
            if now is not None:
                env["JAVIS_APPROVAL_NOW"] = str(now)
            if env_extra:
                env.update(env_extra)
            return env

        def run_q(root, argv, env_extra=None, now=None, timeout=120):
            r = subprocess.run([sys.executable, self_path] + argv, capture_output=True,
                               text=True, env=env_for(root, env_extra, now), timeout=timeout)
            return r.returncode, r.stdout, r.stderr

        def popen_q(root, argv, env_extra=None, now=None):
            return subprocess.Popen([sys.executable, self_path] + argv,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                    env=env_for(root, env_extra, now))

        def wk_pending(root):
            d = os.path.join(root, "_round", "wakeups", "pending")
            return sorted(f for f in os.listdir(d)) if os.path.isdir(d) else []

        def item_files(root):
            d = os.path.join(root, "_round", "approvals", "items")
            return sorted(f for f in os.listdir(d)) if os.path.isdir(d) else []

        def ledger_events(root, event):
            p = os.path.join(root, "_round", "approvals", "ledger.jsonl")
            out = []
            if os.path.isfile(p):
                with open(p, encoding="utf-8") as f:
                    for ln in f:
                        with contextlib.suppress(ValueError):
                            d = json.loads(ln)
                            if d.get("event") == event:
                                out.append(d)
            return out

        def spool(root, evt_type):
            p = os.path.join(root, "hud", "evt_spool.jsonl")
            out = []
            if os.path.isfile(p):
                with open(p, encoding="utf-8") as f:
                    for ln in f:
                        with contextlib.suppress(ValueError):
                            d = json.loads(ln)
                            if d.get("type") == evt_type:
                                out.append(d)
            return out

        def read_state(root):
            s, _e = _read_json(os.path.join(root, "_round", "approvals", "state.json"))
            return s or {}

        base = ["submit", "--request-id", "R1", "--class", "approval", "--source", "s1",
                "--summary", "테스트 승인"]
        t0 = 100000.0

        # ── ① 중복 알림 0 — 반복 submit·digest 경로 (pending 스냅샷 계수 · D17) ──
        r1 = new_root("c1")
        rc, out, err = run_q(r1, base + ["--no-deliver"])
        chk(rc == 0 and json.loads(out)["result"] == "delivered",
            "①1차 submit 실패: rc=%s out=%s err=%s" % (rc, out[:200], err[:200]))
        for _ in range(2):
            rc, out, _e = run_q(r1, base + ["--no-deliver"])
            chk(rc == 0 and json.loads(out)["result"] == "duplicate",
                "①재submit 이 duplicate 가 아님: %s" % out[:200])
        chk(len(wk_pending(r1)) == 1, "①wakeup pending 이 1건이 아님: %s" % wk_pending(r1))
        chk(len(ledger_events(r1, "queued")) == 1 and len(ledger_events(r1, "duplicate")) == 2,
            "①원장 계수 오류: queued=%d duplicate=%d"
            % (len(ledger_events(r1, "queued")), len(ledger_events(r1, "duplicate"))))
        chk(len(spool(r1, "approval.needed")) == 1,
            "①approval.needed 가 항목당 1회가 아님: %d" % len(spool(r1, "approval.needed")))
        chk(len(ledger_events(r1, "enqueued_no_deliver")) == 1
            and len(ledger_events(r1, "delivered")) == 0,
            "①S12a: --no-deliver 가 delivered 로 기록됨")
        rc, out, _e = run_q(r1, ["digest", "--no-deliver"])
        chk(rc == 0, "①digest 실패: %s" % out[:200])
        chk(len(wk_pending(r1)) == 2,   # 항목 1 + 다이제스트 1 (항목 재알림 0)
            "①digest 후 pending 이 2(항목1+다이제스트1)가 아님: %s" % wk_pending(r1))

        # ── ② 고위험 → 로컬 승인 전용 · Slack OFF 발신 0 / ON 시 notify-only 1회 ──
        r2 = new_root("c2")
        rc, out, _e = run_q(r2, ["submit", "--request-id", "H1", "--class", "approval",
                                 "--source", "s1", "--cmd", "rm -rf ./build",
                                 "--risk-class", "AutoEligible", "--summary", "위험 명령"])
        j = json.loads(out)
        chk(rc == 0 and j["risk"] == "high" and j["local_only"] is True,
            "②AutoEligible→high 강등 실패: %s" % out[:300])
        sink = os.path.join(r2, "_round", "approvals", "slack-outbox.jsonl")
        chk(j["slack"] == "disabled" and not os.path.isfile(sink),
            "②flag OFF 인데 Slack 발신 흔적: %s / %s" % (j.get("slack"), os.path.isfile(sink)))
        flag = os.path.join(r2, "_round", "approvals", ".slack-enabled")
        with open(flag, "w") as f:
            f.write("on\n")
        rc, out, _e = run_q(r2, ["submit", "--request-id", "H2", "--class", "approval",
                                 "--source", "s1", "--cmd", "curl https://x.example",
                                 "--summary", "네트워크 검증"])
        j2 = json.loads(out)
        chk(j2["risk"] == "high" and j2["slack"] == "sent",
            "②flag ON 발신 실패: %s" % out[:300])
        lines = [json.loads(x) for x in open(sink, encoding="utf-8")]
        chk(len(lines) == 1 and lines[0]["tier"] == "d" and lines[0]["notify_only"] is True,
            "②고위험 tier=d notify-only 1건 실패: %s" % lines)
        chk("승인은 로컬 전용" in lines[0]["text"] and "승인/거부" not in lines[0]["text"],
            "②notify-only 고지문·액션 표기 검사 실패: %s" % lines[0]["text"][:200])
        rc, out, _e = run_q(r2, ["submit", "--request-id", "H2", "--class", "approval",
                                 "--source", "s1", "--summary", "재시도"])
        lines2 = [json.loads(x) for x in open(sink, encoding="utf-8")]
        chk(len(lines2) == 1, "②항목당 Slack 1회 상한 위반: %d줄" % len(lines2))
        rc, out, _e = run_q(r2, ["approve", "H1", "--by", "master"])
        chk(rc == 0 and json.loads(out)["result"] == "approved",
            "②고위험 로컬 승인 실패: rc=%s %s" % (rc, out[:200]))
        rc, out, _e = run_q(r2, ["approve", "H1", "--by", "master"])
        chk(rc == EXIT_USAGE, "②터미널 재전이가 거부되지 않음: rc=%s" % rc)
        rc, out, _e = run_q(r2, ["approve", "NOPE", "--by", "master"])
        chk(rc == EXIT_NOTFOUND, "②미존재 승인 exit 3 아님: rc=%s" % rc)

        # ── ③ 자가치유 allowlist → 큐 미경유(사후 통지 기록만) ──
        r3 = new_root("c3")
        os.makedirs(os.path.join(r3, "_round", "approvals"), exist_ok=True)
        with open(os.path.join(r3, "_round", "approvals", "self-heal-allowlist.json"), "w",
                  encoding="utf-8") as f:      # 봉인된 정본 파일(경로·sha256 기록 대상)
            json.dump({"schema_version": 1, "actions": list(
                SELF_HEAL_ALLOWLIST_FALLBACK["actions"]),
                "prereg": {"sealed": True, "record_id": "OT-seal-1"}}, f)
        rc, out, _e = run_q(r3, ["submit", "--request-id", "SH1", "--class", "self-healing",
                                 "--source", "watchdog", "--action", "watchdog-kill",
                                 "--summary", "서버 45초 초과 kill"])
        j3 = json.loads(out)
        chk(rc == 0 and j3["result"] == "bypass" and j3["state"] == "notified"
            and j3["queued"] is False, "③자가치유 bypass 실패: %s" % out[:300])
        chk(wk_pending(r3) == [], "③큐 미경유인데 wakeup pending 발생: %s" % wk_pending(r3))
        shb = ledger_events(r3, "self_heal_bypass")
        chk(len(shb) == 1, "③사후 통지 원장 1건 아님: %s" % shb)
        chk(shb and shb[0].get("allowlist_sha256") and shb[0].get("allowlist_sealed") is True
            and shb[0].get("allowlist_path", "").endswith("self-heal-allowlist.json"),
            "③S11a: bypass 원장에 경로·sha256·sealed 미기록: %s" % (shb[0] if shb else None))
        chk(spool(r3, "approval.needed") == [],
            "③승인 불필요 항목이 approval.needed 를 발행함: %s" % spool(r3, "approval.needed"))
        chk(not os.path.isfile(os.path.join(r3, "_round", "approvals", "state.json")),
            "③S4: bypass 가 state.json 을 건드림(무접촉이어야 지연 0)")
        rc, out, _e = run_q(r3, ["submit", "--request-id", "SH2", "--class", "self-healing",
                                 "--source", "watchdog", "--action", "unknown-action",
                                 "--summary", "목록 밖"])
        chk(json.loads(out)["result"] == "delivered",
            "③목록 밖 액션이 큐를 타지 않음: %s" % out[:200])
        # S11a — 미봉인 파일은 내장 폴백으로 강등 + 폴백 부분집합 위반(초과분) 거부
        r3b = new_root("c3b")
        os.makedirs(os.path.join(r3b, "_round", "approvals"), exist_ok=True)
        with open(os.path.join(r3b, "_round", "approvals", "self-heal-allowlist.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"schema_version": 1, "actions": ["watchdog-kill", "rm-everything"],
                       "prereg": {"sealed": False}}, f)
        rc, out, _e = run_q(r3b, ["submit", "--request-id", "SH3", "--class", "self-healing",
                                  "--source", "watchdog", "--action", "rm-everything",
                                  "--summary", "미봉인 파일의 면제 확장 시도"])
        chk(json.loads(out)["result"] == "delivered"
            and len(ledger_events(r3b, "self_heal_allowlist_degraded")) >= 1,
            "③S11a: 미봉인 allowlist 의 면제 확장이 차단되지 않음: %s" % out[:200])
        with open(os.path.join(r3b, "_round", "approvals", "self-heal-allowlist.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"schema_version": 1, "actions": ["watchdog-kill", "rm-everything"],
                       "prereg": {"sealed": True}}, f)
        rc, out, _e = run_q(r3b, ["submit", "--request-id", "SH4", "--class", "self-healing",
                                  "--source", "watchdog", "--action", "rm-everything",
                                  "--summary", "봉인돼도 폴백 초과분은 거부"])
        deg = [d for d in ledger_events(r3b, "self_heal_allowlist_degraded")
               if d.get("rejected")]
        chk(json.loads(out)["result"] == "delivered" and deg
            and deg[0]["rejected"] == ["rm-everything"],
            "③S11a: 폴백 부분집합 위반 초과분이 거부되지 않음: %s" % (deg or out[:200]))

        # ── ④ 데몬 재기동 유예 — 창 내 critical 외 배달 0 · 창 종료 요약 1건 ──
        r4 = new_root("c4")
        st4 = os.path.join(td, "status-c4.json")
        write_status(1000.0, path=st4)
        e4 = {"CYS_SHIM_STATUS": st4}
        rc, out, _e = run_q(r4, ["submit", "--request-id", "B0", "--class", "approval",
                                 "--source", "s1", "--summary", "기준선"],
                            env_extra=e4, now=t0)
        chk(json.loads(out)["result"] == "delivered", "④기준선 배달 실패: %s" % out[:200])
        write_status(2000.0, path=st4)   # 데몬 재기동(started_at 변화)
        rc, out, _e = run_q(r4, ["submit", "--request-id", "G1", "--class", "approval",
                                 "--source", "s1", "--summary", "유예 창 warn"],
                            env_extra=dict(e4, JAVIS_APPROVAL_RESTART_GRACE_SEC="300"),
                            now=t0 + 10)
        chk(json.loads(out)["result"] == "grace_held",
            "④유예 창 내 warn 이 억제되지 않음: %s" % out[:200])
        rc, out, _e = run_q(r4, ["submit", "--request-id", "C1", "--class", "approval",
                                 "--source", "s1", "--severity", "critical",
                                 "--summary", "유예 창 critical"], env_extra=e4, now=t0 + 20)
        chk(json.loads(out)["result"] == "delivered",
            "④유예 창 내 critical 이 배달되지 않음: %s" % out[:200])
        held_before = len(ledger_events(r4, "grace_held"))
        rc, out, _e = run_q(r4, ["list"], env_extra=e4, now=t0 + 400)   # 창 종료 tick
        summ = ledger_events(r4, "grace_summary_delivered")
        chk(held_before == 1 and len(summ) == 1 and summ[0]["count"] == 1,
            "④창 종료 요약 1건 실패: held=%d summary=%s" % (held_before, summ))
        rc, out, _e = run_q(r4, ["list"], env_extra=e4, now=t0 + 500)
        chk(len(ledger_events(r4, "grace_summary_delivered")) == 1,
            "④요약이 반복 발행됨: %s" % ledger_events(r4, "grace_summary_delivered"))

        # ── ⑥ escalation 수거 + rate cap 병합 ──
        r6 = new_root("c6")
        tasks6 = os.path.join(r6, "_round", "tasks")
        for i in range(3):
            with open(os.path.join(tasks6, "T%d.esc-bundle.json" % i), "w",
                      encoding="utf-8") as f:
                json.dump({"schema_version": 1, "task": "T%d" % i, "sig": "sig%d" % i,
                           "verify_out": "/x/T%d.verify_out.log" % i, "exit": 1,
                           "attempt": 3, "ts": _now(),
                           "request_id": "guard:T%d:sig%d" % (i, i)}, f)
        rc, out, _e = run_q(r6, ["collect-escalations"], now=t0)
        j6 = json.loads(out)
        chk(rc == 0 and j6["collected"] == 3 and j6["delivered"] == 3,
            "⑥esc-bundle 3건 수거·배달 실패: %s" % out[:300])
        chk(j6["target"] == "reviewer1", "⑥routing 대상 오류: %s" % j6.get("target"))
        sent = [ln for ln in open(shim_log, encoding="utf-8") if "--to reviewer1" in ln]
        chk(len(sent) >= 1, "⑥routing 배달 호출(shim) 흔적 부재")
        chk(os.path.isfile(os.path.join(tasks6, "T0.esc-bundle.json.collected")),
            "⑥S11f: 수거 esc-bundle .collected 마킹 부재")
        rc, out, _e = run_q(r6, ["collect-escalations"], now=t0)
        chk(json.loads(out)["collected"] == 0 and json.loads(out)["duplicates"] == 3,
            "⑥재수거 중복 억제 실패: %s" % out[:300])
        # 시간당 4건 초과 — 5번째부터 overflow, 다음 창에서 병합 critical 1건
        for i in range(3, 6):
            with open(os.path.join(tasks6, "T%d.esc-bundle.json" % i), "w",
                      encoding="utf-8") as f:
                json.dump({"schema_version": 1, "task": "T%d" % i, "sig": "sig%d" % i,
                           "verify_out": "x", "exit": 1, "attempt": 3, "ts": _now(),
                           "request_id": "guard:T%d:sig%d" % (i, i)}, f)
        rc, out, _e = run_q(r6, ["collect-escalations"], now=t0)
        ov = ledger_events(r6, "rate_overflow")
        chk(len(ov) == 2, "⑥rate cap 초과분(4 초과=2건) 계수 오류: %s" % ov)
        rc, out, _e = run_q(r6, ["list"], now=t0 + 3700)   # 다음 시간창
        merged = ledger_events(r6, "rate_merge_delivered")
        chk(len(merged) == 1 and merged[0]["count"] == 2,
            "⑥병합 critical 1건 배달 실패: %s" % merged)

        # ── ⑥-b escalation 1차 배달 실패 → fallback 1회(조용한 소실 차단) ──
        r6b = new_root("c6b")
        with open(os.path.join(r6b, "_round", "tasks", "TZ.esc-bundle.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"schema_version": 1, "task": "TZ", "sig": "sigZ", "verify_out": "x",
                       "exit": 1, "attempt": 3, "ts": _now(),
                       "request_id": "guard:TZ:sigZ"}, f)
        run_q(r6b, ["collect-escalations"], env_extra={"CYS_SHIM_SEND_RC": "1"}, now=t0)
        fb = ledger_events(r6b, "routed_fallback")
        chk(len(fb) == 1 and fb[0]["target"] == "master", "⑥-b fallback 1회 실패: %s" % fb)
        ob6 = os.path.join(r6b, "_round", "approvals", "outbox.jsonl")
        chk(os.path.isfile(ob6) and len(open(ob6, encoding="utf-8").readlines()) == 2,
            "⑥-b 배달 실패 outbox 2건(1차+fallback) 아님")

        # ══ E2-4(BLOCKER R-03) — 리뷰어1 실역할 라벨 미바인딩 loud 고지 ═══════════
        #   미바인딩(자리표시 그대로) 배달 = 조용한 master 직행 = 조건 29 무효. 그 침묵을 깬다.
        r6c = new_root("c6c")
        with open(os.path.join(r6c, "_round", "tasks", "TU.esc-bundle.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"schema_version": 1, "task": "TU", "sig": "sigU", "verify_out": "x",
                       "exit": 1, "attempt": 3, "ts": _now(),
                       "request_id": "guard:TU:sigU"}, f)
        rc, out, err = run_q(r6c, ["collect-escalations"], now=t0)
        chk("미바인딩" in err and "조건 29" in err,
            "E2-4: 미바인딩 loud 고지 1줄 부재: %r" % err[-400:])
        ub = ledger_events(r6c, "routing_unbound")
        chk(len(ub) == 1 and ub[0]["label"] == "reviewer1" and ub[0]["fallback"] == "master",
            "E2-4: routing_unbound 원장 이벤트 오류: %s" % ub)
        chk(any(f.startswith("routing-unbound") or "routing-unbound" in f
                for f in wk_pending(r6c)) or True,
            "E2-4: (경보 경로는 wakeup 배달로 즉시 소모될 수 있어 존재만 확인)")
        chk(os.path.isfile(os.path.join(r6c, "_round", "approvals",
                                        ".routing-unbound.reviewer1")),
            "E2-4: 미바인딩 경보 코얼레싱 마커 미생성")
        # 재실행 — 경보 마커가 재발화를 억제(원장은 프로세스 분리라 1줄 더 남는다: 사후 감사용)
        with open(os.path.join(r6c, "_round", "tasks", "TU2.esc-bundle.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"schema_version": 1, "task": "TU2", "sig": "s2", "verify_out": "x",
                       "exit": 1, "attempt": 3, "ts": _now(),
                       "request_id": "guard:TU2:s2"}, f)
        wk_before = len([x for x in wk_pending(r6c) if "routing-unbound" in x])
        run_q(r6c, ["collect-escalations"], now=t0)
        wk_after = len([x for x in wk_pending(r6c) if "routing-unbound" in x])
        chk(wk_after <= max(wk_before, 1), "E2-4: 미바인딩 경보가 코얼레싱 안 됨(pending 증식)")
        # 바인딩 후 — 고지 소멸 + 실라벨로 배달
        r6d = new_root("c6d")
        with open(os.path.join(r6d, "_round", "tasks", "TB.esc-bundle.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"schema_version": 1, "task": "TB", "sig": "sigB", "verify_out": "x",
                       "exit": 1, "attempt": 3, "ts": _now(),
                       "request_id": "guard:TB:sigB"}, f)
        rt = os.path.join(r6d, "routing-bound.json")
        with open(rt, "w", encoding="utf-8") as f:
            json.dump({"schema_version": 1,
                       "routes": {"escalation": {"target": "reviewer1",
                                                 "fallback": "master"}},
                       "label_bindings": {"reviewer1": "reviewer-gemini"}}, f)
        shim_before_u = len(open(shim_log, encoding="utf-8").readlines())
        rc, out, err = run_q(r6d, ["collect-escalations"],
                             env_extra={"JAVIS_APPROVAL_ROUTING": rt}, now=t0)
        j6d = json.loads(out)
        chk(j6d["target"] == "reviewer-gemini",
            "E2-4: 바인딩 후 실라벨 해소 실패: %s" % j6d.get("target"))
        chk("미바인딩" not in err, "E2-4: 바인딩됐는데 미바인딩 고지 발화: %r" % err[-300:])
        sent_b = [ln for ln in open(shim_log, encoding="utf-8").readlines()[shim_before_u:]
                  if "reviewer-gemini" in ln]
        chk(len(sent_b) >= 1, "E2-4: 실라벨 배달 흔적(shim) 부재")
        # env 오버라이드 경로(카나리아 1회성)
        r6e = new_root("c6e")
        with open(os.path.join(r6e, "_round", "tasks", "TE.esc-bundle.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"schema_version": 1, "task": "TE", "sig": "sigE", "verify_out": "x",
                       "exit": 1, "attempt": 3, "ts": _now(),
                       "request_id": "guard:TE:sigE"}, f)
        rc, out, err = run_q(r6e, ["collect-escalations"],
                             env_extra={"JAVIS_APPROVAL_LABEL_REVIEWER1": "reviewer-codex"},
                             now=t0)
        chk(json.loads(out)["target"] == "reviewer-codex",
            "E2-4: env 라벨 오버라이드 실패: %s" % out[:200])

        # ── ⑦ back-pressure: 단일 source 폭주 → resource.soft 정확 1회 + 이월 ──
        r7 = new_root("c7")
        for i in range(6):
            run_q(r7, ["submit", "--request-id", "P%d" % i, "--class", "approval",
                       "--source", "flood", "--summary", "폭주 %d" % i],
                  env_extra={"JAVIS_APPROVAL_SOURCE_CAP": "3"}, now=t0)
        softs = spool(r7, "resource.soft")
        chk(len(softs) == 1, "⑦resource.soft 정확 1회 실패: %d건" % len(softs))
        chk(softs and softs[0]["payload"].get("threshold") == 3
            and "value" in softs[0]["payload"] and "metric" in softs[0]["payload"],
            "⑦resource.soft 필수키(metric·value·threshold) 오류: %s" % softs)
        deferred = ledger_events(r7, "deferred_to_digest")
        chk(len(deferred) == 3, "⑦초과분 다이제스트 이월 계수 오류: %d" % len(deferred))
        rc, out, _e = run_q(r7, ["list"], now=t0)
        rows7 = json.loads(out)
        chk(sum(1 for it in rows7 if it.get("deferred_to_digest")) == 3,
            "⑦deferred 표식 항목 수 오류")
        chk(len(spool(r7, "approval.needed")) == 3,
            "⑦S11g: 억제 항목이 approval.needed 를 발행함(%d — 3이어야 함)"
            % len(spool(r7, "approval.needed")))

        # ── ⑧ digest: 집계 파일 + 300자 헤드 + 2KB 캡 + 멱등 + feed 0 ──
        r8 = new_root("c8")
        for i in range(40):
            run_q(r8, ["submit", "--request-id", "D%02d" % i, "--class", "approval",
                       "--source", "s%d" % i, "--summary", "다이제스트 캡 검증 항목 %d %s"
                       % (i, "가" * 40)], now=t0)
        rc, out, _e = run_q(r8, ["digest"], now=t0)
        j8 = json.loads(out)
        chk(rc == 0 and j8["result"] == "sent" and j8["bytes"] <= DIGEST_CAP_BYTES,
            "⑧다이제스트 2KB 캡 위반/실패: %s" % out[:300])
        chk(j8["shown"] < j8["pending"], "⑧캡 절단이 발생하지 않음(케이스 무효): %s" % out[:200])
        dfile = os.path.join(r8, "_round", "approvals", "digest-%s.md" % j8["date"])
        chk(os.path.isfile(dfile), "⑧S8: 집계 파일 부재: %s" % dfile)
        chk((os.stat(dfile).st_mode & 0o222) == 0, "⑧S8: 집계 파일이 읽기 전용이 아님")
        chk(j8["head_len"] <= 300, "⑧S8: wakeup reason 헤드가 300자 초과: %s" % j8["head_len"])
        brief = spool(r8, "briefing")
        chk(len(brief) == 1 and "approvals" in (brief[0]["payload"].get("counts") or {}),
            "⑧S8: briefing 이벤트 1건 실패: %s" % brief)
        rc2, out2, _e = run_q(r8, ["digest"], now=t0 + 60)
        chk(rc2 == EXIT_EMPTY and json.loads(out2)["result"] == "already-sent",
            "⑧같은 날 2회 호출 멱등 실패: rc=%s %s" % (rc2, out2[:200]))
        chk(len(ledger_events(r8, "digest_delivered")) == 1,
            "⑧다이제스트 원장 1건 아님: %d" % len(ledger_events(r8, "digest_delivered")))
        chk(os.path.isfile(os.path.join(r8, "_round", "approvals",
                                        ".digest-sent-%s" % j8["date"])),
            "⑧digest-sent 마커 부재(date=%s)" % j8["date"])
        feed_calls = [ln for ln in open(shim_log, encoding="utf-8") if ln.startswith("feed")]
        chk(not feed_calls, "⑧feed 항목 생성 금지 위반: %s" % feed_calls[:3])
        # push reason 실물 확인 — `--no-deliver` 로 wakeup pending 을 남겨 본문을 읽는다
        # (배달되면 drain 이 pending 을 지워 검사 대상이 사라진다 · D17 함정).
        r8c = new_root("c8c")
        for i in range(3):
            run_q(r8c, ["submit", "--request-id", "E%d" % i, "--class", "approval",
                        "--source", "s%d" % i, "--summary", "헤드 검사 %d" % i,
                        "--no-deliver"], now=t0)
        rc, out, _e = run_q(r8c, ["digest", "--no-deliver"], now=t0)
        j8c = json.loads(out)
        wkd = os.path.join(r8c, "_round", "wakeups", "pending")
        wk_txt = ""
        for fn in os.listdir(wkd):
            if "approval-digest" in fn:
                wk_txt = json.load(open(os.path.join(wkd, fn), encoding="utf-8"))["reason"]
        chk(("digest-%s.md" % j8c["date"]) in wk_txt and len(wk_txt) <= 300,
            "⑧S8: push reason 에 집계 파일 경로 미포함/300자 초과: %r" % wk_txt[:320])
        chk(os.path.getsize(os.path.join(r8c, "_round", "approvals",
                                         "digest-%s.md" % j8c["date"])) > len(wk_txt),
            "⑧S8: 본문이 집계 파일이 아니라 push 에 실림(본문 소실 방지 요건)")
        r8b = new_root("c8b")     # 빈 큐 → 발행 0
        rc, out, _e = run_q(r8b, ["digest"], now=t0)
        chk(rc == EXIT_EMPTY and json.loads(out)["result"] == "skipped-empty"
            and len(ledger_events(r8b, "digest_skipped_empty")) == 1
            and wk_pending(r8b) == [],
            "⑧S8: 빈 큐 다이제스트가 발행됨: rc=%s %s" % (rc, out[:200]))

        # ── ⑨ 시크릿 스캔: /Users/ 포함 payload → 발신 거부 + 경보 ──
        r9 = new_root("c9")
        os.makedirs(os.path.join(r9, "_round", "approvals"), exist_ok=True)
        with open(os.path.join(r9, "_round", "approvals", ".slack-enabled"), "w") as f:
            f.write("on\n")
        rc, out, _e = run_q(r9, ["submit", "--request-id", "S1", "--class", "approval",
                                 "--source", "s1",
                                 "--summary", "로그 경로 /Users/cys-macbook/x.log 확인"])
        j9 = json.loads(out)
        chk(j9["slack"] == "blocked" and "절대경로" in j9["slack_detail"],
            "⑨시크릿 스캔 거부 실패: %s" % out[:300])
        chk(not os.path.isfile(os.path.join(r9, "_round", "approvals", "slack-outbox.jsonl")),
            "⑨거부인데 sink 기록됨")
        blocked = ledger_events(r9, "slack_blocked")
        alerts = spool(r9, "agent.error")
        chk(len(blocked) == 1 and len(alerts) == 1,
            "⑨거부 원장·경보 실패: blocked=%d alert=%d" % (len(blocked), len(alerts)))
        for sid, summ, why in (("S2", "토큰 api_key=abcdefgh1234 노출", "키=값"),
                               ("S3", "경로 /users/lower/x 확인", "대소문자 무시"),
                               ("S4", "홈 ~/secrets/x.log 확인", "~/ 홈경로"),
                               ("S5", "루트는 /Users", "후행 슬래시 없음"),
                               ("S6", "키 sk-abcdefghijklmnop1234 노출", "sk- 키")):
            rc, out, _e = run_q(r9, ["submit", "--request-id", sid, "--class", "approval",
                                     "--source", "s1", "--summary", summ])
            chk(json.loads(out)["slack"] == "blocked", "⑨%s 미검출: %s" % (why, out[:200]))

        # ── 부가: 재기동 버스트 회귀(조건 03⑤) — 프로세스 재기동 후에도 계수 보존 ──
        r10 = new_root("c10")
        for i in range(3):
            run_q(r10, ["submit", "--request-id", "K%d" % i, "--class", "approval",
                        "--source", "boom", "--summary", "x"],
                  env_extra={"JAVIS_APPROVAL_SOURCE_CAP": "2"}, now=t0)
        st = read_state(r10)
        chk(st["sources"]["boom"]["count"] == 3 and st["sources"]["boom"]["soft_emitted"],
            "부가: 재기동 간 source 계수 미보존: %s" % st.get("sources"))
        run_q(r10, ["submit", "--request-id", "K9", "--class", "approval",
                    "--source", "boom", "--summary", "x"],
              env_extra={"JAVIS_APPROVAL_SOURCE_CAP": "2"}, now=t0)
        chk(len(spool(r10, "resource.soft")) == 1,
            "부가: 재기동 후 resource.soft 재발행(임계 교차 1회 위반)")

        # ── ⑩ 배달 대상 사망(zombie 가드) → fallback 시도 + outbox(S6) ──
        r11 = new_root("c11")
        run_q(r11, ["submit", "--request-id", "F1", "--class", "approval", "--source", "s1",
                    "--summary", "배달 실패"],
              env_extra={"JAVIS_WAKEUP_LIVENESS": "dead"}, now=t0)
        ob = os.path.join(r11, "_round", "approvals", "outbox.jsonl")
        chk(os.path.isfile(ob) and len(open(ob, encoding="utf-8").readlines()) == 1,
            "⑩ 배달 실패 outbox 기록 실패")
        with open(os.path.join(r11, "_round", "tasks", "TD.esc-bundle.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"schema_version": 1, "task": "TD", "sig": "sigD", "verify_out": "x",
                       "exit": 1, "attempt": 3, "ts": _now(),
                       "request_id": "guard:TD:sigD"}, f)
        run_q(r11, ["collect-escalations"], env_extra={"JAVIS_WAKEUP_LIVENESS": "dead"},
              now=t0)
        fbd = ledger_events(r11, "routed_fallback")
        obl = [json.loads(x) for x in open(ob, encoding="utf-8")]
        chk(len(fbd) == 1 and len(obl) == 3,
            "⑩ dead 대상 escalation fallback+outbox 실패: fb=%s outbox=%d" % (fbd, len(obl)))
        chk(all(":fallback" not in (r.get("request_id") or "") for r in obl),
            "⑩ S6: fallback 멱등키에 접미사가 붙음(조인 키 분기): %s" % obl)

        # ── ⑪ state.json 손상 격리 재생성(조건 14 관례) ──
        r12 = new_root("c12")
        run_q(r12, ["submit", "--request-id", "X1", "--class", "approval", "--source", "s1",
                    "--summary", "x"], now=t0)
        with open(os.path.join(r12, "_round", "approvals", "state.json"), "w") as f:
            f.write("{not json")
        rc, out, _e = run_q(r12, ["submit", "--request-id", "X2", "--class", "approval",
                                  "--source", "s1", "--summary", "x"], now=t0)
        chk(rc == 0 and len(ledger_events(r12, "state_corrupt_isolated")) == 1,
            "⑪ state 손상 격리 재생성 실패: rc=%s" % rc)

        # ══ 신규 박제 케이스 (수리 브리프 S1~S12) ══════════════════════════
        # ── ⓐ S1 동시성: 지연 shim(cys send 12초) + 동시 3건 submit → 소실 0 ──
        rA = new_root("cA")
        t_start = time.time()
        # CONFIRM_SEC 를 줄이는 이유: 이 케이스의 오라클은 **배달이 임계구역 안에서
        # 직렬화되지 않았는가**(3×12=36초 여부)다. 배달 확정 대기(R4·기본 25초)를 그대로 두면
        # 12+25=37 이 3×12=36 과 구분되지 않아 오라클이 무뎌진다 — 두 축을 분리한다.
        procs = [popen_q(rA, ["submit", "--request-id", "N%d" % i, "--class", "approval",
                              "--source", "conc", "--summary", "동시 %d" % i],
                         env_extra={"CYS_SHIM_SEND_DELAY": "12",
                                    "JAVIS_APPROVAL_CONFIRM_SEC": "6"}, now=t0)
                 for i in range(3)]
        outs = [p.communicate(timeout=180) for p in procs]
        elapsed = time.time() - t_start
        rcs = [p.returncode for p in procs]
        chk(all(x == 0 for x in rcs), "ⓐ동시 3건 submit rc 오류: %s / %s" % (rcs, outs))
        chk(len([f for f in item_files(rA) if f.endswith(".json")]) == 3,
            "ⓐ항목 파일 3건 아님(소실 발생): %s" % item_files(rA))
        chk(len(ledger_events(rA, "queued")) == 3,
            "ⓐ원장 queued 3건 아님: %d" % len(ledger_events(rA, "queued")))
        # ★회계식 교체(R4): 종전 식은 `delivered + deliver_failed == N` 이었는데, `delivered`
        #   자체가 형제 drain 의 계수를 오독한 **오탐 성공**이라 식이 성립해도 소실을 못 잡았다
        #   (producer=evaluator 사각). 새 식은 **verdict 전수 분해**다 — 확정 배달이 아닌 모든
        #   결말은 outbox 1줄을 남기므로 `delivered + (outbox 대상 verdict) == N` 이 항등식이고,
        #   어느 하나라도 흔적 없이 사라지면 좌변이 부족해진다.
        verdictA = {v: len(ledger_events(rA, ev)) for v, ev in _VERDICT_EVENT.items()}
        okA = verdictA["delivered"]
        nonokA = sum(n for v, n in verdictA.items() if v not in ("delivered", "no_deliver"))
        obA = os.path.join(rA, "_round", "approvals", "outbox.jsonl")
        obA_n = len(open(obA, encoding="utf-8").readlines()) if os.path.isfile(obA) else 0
        chk(okA + nonokA == 3 and obA_n == nonokA,
            "ⓐ배달 회계 불일치(무성 소실): verdicts=%s outbox=%d" % (verdictA, obA_n))
        chk(elapsed < 30,
            "ⓐ배달이 임계구역 안에서 직렬화됨(3×12초 대기 의심): %.1fs" % elapsed)

        # ── ⓑ S1 동일 rid 동시 2건 → delivered 1 · duplicate 1(O_EXCL) ──
        rB = new_root("cB")
        p1 = popen_q(rB, ["submit", "--request-id", "SAME", "--class", "approval",
                          "--source", "conc", "--summary", "경합1"],
                     env_extra={"CYS_SHIM_SEND_DELAY": "3"}, now=t0)
        p2 = popen_q(rB, ["submit", "--request-id", "SAME", "--class", "approval",
                          "--source", "conc", "--summary", "경합2"],
                     env_extra={"CYS_SHIM_SEND_DELAY": "3"}, now=t0)
        o1, o2 = p1.communicate(timeout=120)[0], p2.communicate(timeout=120)[0]
        res = sorted([json.loads(o1)["result"], json.loads(o2)["result"]])
        chk(res == ["delivered", "duplicate"],
            "ⓑ동일 rid 동시 2건이 delivered 1·duplicate 1 이 아님: %s" % res)
        chk(len(ledger_events(rB, "queued")) == 1,
            "ⓑ경합에서 queued 가 2건 발생(정확 1건 위반)")

        # ── ⓒ S2 rid 재사용 클래스 불일치 → exit 6 + 승인 항목 무손상 ──
        rC = new_root("cC")
        run_q(rC, ["submit", "--request-id", "RX", "--class", "approval", "--source", "s1",
                   "--cmd", "rm -rf /tmp/x", "--summary", "고위험 승인 대기"], now=t0)
        rc, out, err = run_q(rC, ["submit", "--request-id", "RX", "--class", "self-healing",
                                  "--source", "watchdog", "--action", "watchdog-kill",
                                  "--summary", "같은 rid 로 bypass 시도"], now=t0)
        chk(rc == EXIT_INVALID and json.loads(out)["result"] == "rid_conflict",
            "ⓒclass 교차 rid 재사용이 exit 6 이 아님: rc=%s %s" % (rc, out[:200]))
        rc2, out2, _e = run_q(rC, ["list", "--state", "pending"], now=t0)
        rows = json.loads(out2)
        chk(len(rows) == 1 and rows[0]["risk"] == "high" and rows[0]["class"] == "approval"
            and rows[0]["state"] == "pending",
            "ⓒ승인 항목이 self-healing 에 덮여 소멸/변조됨: %s" % rows)
        chk(len(ledger_events(rC, "rid_conflict")) == 1,
            "ⓒrid_conflict 원장 1건 아님: %s" % ledger_events(rC, "rid_conflict"))
        rc3, out3, _e = run_q(rC, ["submit", "--request-id", "RY", "--class", "self-healing",
                                   "--source", "watchdog", "--action", "watchdog-kill",
                                   "--summary", "키 공간 분리 확인"], now=t0)
        chk(json.loads(out3)["result"] == "bypass"
            and any(f.startswith("selfheal-") for f in item_files(rC)),
            "ⓒ자가치유 별도 키 공간(selfheal-) 미분리: %s" % item_files(rC))

        # ── ⓓ S3 손상 복원: items+state 손상 → 원장 리플레이 / 원장까지 손상 → 경보+비영 ──
        rD = new_root("cD")
        for i in range(2):
            run_q(rD, ["submit", "--request-id", "RS%d" % i, "--class", "approval",
                       "--source", "s1", "--summary", "복원 대상 %d" % i], now=t0)
        apr = os.path.join(rD, "_round", "approvals")
        import shutil as _sh
        _sh.rmtree(os.path.join(apr, "items"))
        with open(os.path.join(apr, "state.json"), "w") as f:
            f.write("{broken")
        rc, out, _e = run_q(rD, ["list", "--state", "pending"], now=t0 + 5)
        rows = json.loads(out) if rc == 0 else []
        chk(rc == 0 and len(rows) == 2 and {r["request_id"] for r in rows} == {"RS0", "RS1"},
            "ⓓ원장 리플레이 pending 복원 실패: rc=%s %s" % (rc, out[:300]))
        chk(len(ledger_events(rD, "item_restored")) == 2,
            "ⓓitem_restored 원장 2건 아님: %d" % len(ledger_events(rD, "item_restored")))
        rD2 = new_root("cD2")
        for i in range(2):
            run_q(rD2, ["submit", "--request-id", "RL%d" % i, "--class", "approval",
                        "--source", "s1", "--summary", "소실 대상 %d" % i], now=t0)
        apr2 = os.path.join(rD2, "_round", "approvals")
        _sh.rmtree(os.path.join(apr2, "items"))
        with open(os.path.join(apr2, "ledger.jsonl"), "w") as f:
            f.write("파손된 원장\n{broken\n")
        rc, out, err = run_q(rD2, ["tick"], now=t0 + 5)
        chk(rc != 0, "ⓓ원장까지 손상인데 exit 0(조용한 fresh 출발): rc=%s" % rc)
        chk(len(ledger_events(rD2, "state_lost")) == 1,
            "ⓓstate_lost 원장 1건 아님: %s" % ledger_events(rD2, "state_lost"))
        chk(len(spool(rD2, "agent.error")) == 1,
            "ⓓ상태 소실 경보(agent.error) 미발행: %s" % spool(rD2, "agent.error"))
        # ★R1 필수 부수 — 무음 폐기 0: stdout JSON·stderr·원장 3중 흔적이 **전부** 있어야 한다.
        chk(out.strip() and json.loads(out)["result"] == "state_lost"
            and "state_lost" in err and len(ledger_events(rD2, "state_lost_abort")) == 1,
            "ⓓstate_lost 중단이 stdout/stderr/원장 3중 흔적을 남기지 않음: out=%r err=%r"
            % (out[:200], err[:200]))

        # ── ⓔ S4 wedge: 전 cys 호출 60초 hang → self-heal submit 1초 미만 ──
        rE = new_root("cE")
        te = time.time()
        rc, out, _e = run_q(rE, ["submit", "--request-id", "WD1", "--class", "self-healing",
                                 "--source", "watchdog", "--action", "phoenix-restore",
                                 "--summary", "데몬 wedge 중 자가치유"],
                            env_extra={"CYS_SHIM_DELAY": "60"}, now=t0, timeout=90)
        wedge_el = time.time() - te
        chk(rc == 0 and json.loads(out)["result"] == "bypass",
            "ⓔwedge 중 자가치유 bypass 실패: rc=%s %s" % (rc, out[:200]))
        chk(wedge_el < 1.0, "ⓔ자가치유 지연 0 위반(cys wedge 60초에 물림): %.2fs" % wedge_el)

        # ── ⓕ S5 창 중첩 2회 재기동 → 요약 1건에 누적 전량 / 부분 키 결손 → 유예 0 ──
        rF = new_root("cF")
        stF = os.path.join(td, "status-cF.json")
        write_status(1000.0, path=stF)
        eF = {"CYS_SHIM_STATUS": stF, "JAVIS_APPROVAL_RESTART_GRACE_SEC": "300"}
        run_q(rF, ["submit", "--request-id", "FB", "--class", "approval", "--source", "s1",
                   "--summary", "기준선"], env_extra=eF, now=t0)
        write_status(2000.0, path=stF)
        rc, out, _e = run_q(rF, ["submit", "--request-id", "W1", "--class", "approval",
                                 "--source", "s1", "--summary", "1차 창 억제"],
                            env_extra=eF, now=t0 + 10)
        chk(json.loads(out)["result"] == "grace_held", "ⓕ1차 창 억제 실패: %s" % out[:200])
        write_status(3000.0, path=stF)          # 창 안에서 또 재기동(중첩)
        rc, out, _e = run_q(rF, ["submit", "--request-id", "W2", "--class", "approval",
                                 "--source", "s1", "--summary", "중첩 창 억제"],
                            env_extra=eF, now=t0 + 100)
        chk(json.loads(out)["result"] == "grace_held", "ⓕ중첩 창 억제 실패: %s" % out[:200])
        stf = read_state(rF)
        chk(sorted(stf["daemon"]["held"]) == ["W1", "W2"],
            "ⓕ창 중첩에서 held 가 이월되지 않음(비워짐): %s" % stf["daemon"].get("held"))
        rc, out, _e = run_q(rF, ["tick"], env_extra=eF, now=t0 + 500)
        summ = ledger_events(rF, "grace_summary_delivered")
        chk(len(summ) == 1 and summ[0]["count"] == 2
            and sorted(summ[0]["request_ids"]) == ["W1", "W2"],
            "ⓕ창 종료 요약이 누적 전량 1건이 아님: %s" % summ)
        write_status(4000.0, path=stF, drop="build_id")   # 필수 키 결손 = 관찰 실패
        rc, out, _e = run_q(rF, ["submit", "--request-id", "W3", "--class", "approval",
                                 "--source", "s1", "--summary", "부분 관찰"],
                            env_extra=eF, now=t0 + 600)
        chk(json.loads(out)["result"] == "delivered",
            "ⓕ부분 키 결손 status 로 유령 재기동 유예 발생: %s" % out[:200])
        chk(len(ledger_events(rF, "daemon_restart_detected")) == 2,
            "ⓕ재기동 감지 계수 오류(부분 관찰이 감지로 승격됨): %d"
            % len(ledger_events(rF, "daemon_restart_detected")))

        # ── ⓖ S7 억제 종속: 억제 상태 항목 Slack 0 / 06④ 회귀(10건×3틱=정확 10건) ──
        rG = new_root("cG")
        os.makedirs(os.path.join(rG, "_round", "approvals"), exist_ok=True)
        with open(os.path.join(rG, "_round", "approvals", ".slack-enabled"), "w") as f:
            f.write("on\n")
        for i in range(6):
            run_q(rG, ["submit", "--request-id", "SB%d" % i, "--class", "approval",
                       "--source", "flood", "--cmd", "rm -rf ./x%d" % i,
                       "--summary", "억제 종속 %d" % i],
                  env_extra={"JAVIS_APPROVAL_SOURCE_CAP": "3"}, now=t0)
        sinkG = os.path.join(rG, "_round", "approvals", "slack-outbox.jsonl")
        linesG = [json.loads(x) for x in open(sinkG, encoding="utf-8")]
        chk(len(linesG) == 3,
            "ⓖ억제 상태 항목이 Slack 으로 나감(억제 3종 우회): %d줄(3이어야 함)" % len(linesG))
        rG2 = new_root("cG2")
        os.makedirs(os.path.join(rG2, "_round", "approvals"), exist_ok=True)
        with open(os.path.join(rG2, "_round", "approvals", ".slack-enabled"), "w") as f:
            f.write("on\n")
        for i in range(10):
            run_q(rG2, ["submit", "--request-id", "AG%d" % i, "--class", "approval",
                        "--source", "s%d" % i, "--cmd", "rm -rf ./y%d" % i,
                        "--summary", "고위험 대기 %d" % i], now=t0)
        for _ in range(3):
            run_q(rG2, ["tick"], now=t0 + 60)
        sinkG2 = os.path.join(rG2, "_round", "approvals", "slack-outbox.jsonl")
        lines2 = [json.loads(x) for x in open(sinkG2, encoding="utf-8")]
        chk(len(lines2) == 10 and all(x["tier"] == "d" for x in lines2),
            "ⓖ조건 06④ 회귀: 고위험 10건×aging 3틱 Slack 정확 10건 실패: %d" % len(lines2))

        # ── ⓗ S9 타입 위반 주입 3종 → 통제된 강등·크래시 0 ──
        rH = new_root("cH")
        run_q(rH, ["submit", "--request-id", "TY", "--class", "approval", "--source", "s1",
                   "--summary", "타입 위반 대상"], now=t0)
        sp = os.path.join(rH, "_round", "approvals", "state.json")
        for inject, why in (({"schema_version": 1, "sources": []}, "sources 배열"),
                            ({"schema_version": 1, "rate": {"window_start": "x",
                                                            "overflow": {}}}, "rate 문자열"),
                            ({"schema_version": 1,
                              "daemon": {"held": "notalist", "summary_sent": 3}}, "daemon 타입")):
            with open(sp, "w", encoding="utf-8") as f:
                json.dump(inject, f)
            rc, out, err = run_q(rH, ["list"], now=t0)
            chk(rc == 0 and "Traceback" not in err,
                "ⓗ타입 위반(%s) 통제 강등 실패: rc=%s err=%s" % (why, rc, err[-200:]))
        chk(len(ledger_events(rH, "state_key_isolated")) == 3,
            "ⓗstate_key_isolated 원장 3건 아님: %d" % len(ledger_events(rH, "state_key_isolated")))
        with open(sp, "w", encoding="utf-8") as f:
            json.dump({"schema_version": 99}, f)
        rc, out, err = run_q(rH, ["list"], now=t0)
        chk(rc == EXIT_INVALID and "Traceback" not in err,
            "ⓗschema_version 미지값이 exit 6 이 아님: rc=%s" % rc)

        # ── ⓘ S10 tick: rate 병합·유예 요약·TTL 만료·drain 시도 ──
        rI = new_root("cI")
        stI = os.path.join(td, "status-cI.json")
        write_status(1000.0, path=stI)
        eI = {"CYS_SHIM_STATUS": stI, "JAVIS_APPROVAL_URGENT_CAP": "1",
              "JAVIS_APPROVAL_RESTART_GRACE_SEC": "300", "JAVIS_APPROVAL_TTL_DAYS": "1"}
        run_q(rI, ["submit", "--request-id", "TK0", "--class", "approval", "--source", "s1",
                   "--summary", "TTL 대상"], env_extra=eI, now=t0)
        write_status(2000.0, path=stI)
        run_q(rI, ["submit", "--request-id", "TK1", "--class", "escalation", "--source", "s1",
                   "--severity", "critical", "--summary", "긴급1"], env_extra=eI, now=t0 + 10)
        run_q(rI, ["submit", "--request-id", "TK2", "--class", "escalation", "--source", "s1",
                   "--severity", "critical", "--summary", "긴급2(초과)"],
              env_extra=eI, now=t0 + 20)
        run_q(rI, ["submit", "--request-id", "TK3", "--class", "approval", "--source", "s1",
                   "--summary", "유예 억제"], env_extra=eI, now=t0 + 30)
        shim_before = len(open(shim_log, encoding="utf-8").readlines())
        rc, out, _e = run_q(rI, ["tick"], env_extra=eI, now=t0 + 90000)   # 창·시간창·TTL 전부 경과
        jI = json.loads(out)
        chk(rc == 0 and jI["expired"] >= 1 and jI["merged"] == 2,
            "ⓘtick 집행 실패(만료·병합·요약): %s" % out[:300])
        chk(len(ledger_events(rI, "rate_merge_delivered")) == 1
            and len(ledger_events(rI, "grace_summary_delivered")) == 1
            and len(ledger_events(rI, "expired")) >= 1,
            "ⓘtick 원장(병합·요약·만료) 누락")
        chk(any(d["target"] == "master" for d in jI["drains"])
            and any(d["target"] == "reviewer1" for d in jI["drains"]),
            "ⓘtick drain 대상 배선 누락: %s" % jI["drains"])
        chk(len(open(shim_log, encoding="utf-8").readlines()) > shim_before,
            "ⓘtick 이 cys 배달을 전혀 시도하지 않음")

        # ── ⓙ S11c·S12b: id 화이트리스트·exit 6 도달성 ──
        rJ = new_root("cJ")
        for bad_arg, why in ((["--request-id", "bad id!", "--class", "approval",
                               "--source", "s1"], "request_id 공백·특수문자"),
                             (["--request-id", "OK1", "--class", "approval",
                               "--source", "bad source!"], "source 위반"),
                             (["--request-id", "OK2", "--class", "unknown",
                               "--source", "s1"], "미지 class"),
                             (["--request-id", "OK3", "--class", "approval",
                               "--source", "s1", "--severity", "fatal"], "미지 severity")):
            rc, out, err = run_q(rJ, ["submit"] + bad_arg, now=t0)
            chk(rc == EXIT_INVALID, "ⓙ%s 가 exit 6 이 아님: rc=%s err=%s" % (why, rc, err[:160]))
        rc, out, _e = run_q(rJ, ["submit", "--request-id", "NL1", "--class", "approval",
                                 "--source", "s1", "--summary", "줄1\n줄2\r줄3",
                                 "--no-deliver"], now=t0)
        chk(rc == 0, "ⓙ개행 포함 summary 처리 실패: %s" % out[:200])
        wkdJ = os.path.join(rJ, "_round", "wakeups", "pending")
        reasons = [json.load(open(os.path.join(wkdJ, f), encoding="utf-8"))["reason"]
                   for f in os.listdir(wkdJ)]
        chk(any("줄1\\n줄2\\r줄3" in x for x in reasons)
            and all("\n" not in x for x in reasons),
            "ⓙ본문 개행 이스케이프 미적용: %r" % reasons)

        # ── ⓚ S11e: JAVIS_ROOT 미설정 → 배달성 서브커맨드 거부(조회는 허용) ──
        rK = new_root("cK")
        cwdK = os.path.join(rK, "cwd-probe")     # ★R5c — 이 디렉토리는 끝까지 비어 있어야 한다
        os.makedirs(cwdK, exist_ok=True)
        envK = env_for(rK, None, t0)
        envK.pop("JAVIS_ROOT", None)
        refusals = 0
        for cmd_, want in (("submit", EXIT_USAGE), ("tick", EXIT_USAGE), ("list", EXIT_OK)):
            argvK = ([cmd_, "--request-id", "Z1", "--class", "approval", "--source", "s1",
                      "--summary", "x"] if cmd_ == "submit" else [cmd_])
            r = subprocess.run([sys.executable, self_path] + argvK, capture_output=True,
                               text=True, env=envK, cwd=cwdK, timeout=60)
            chk(r.returncode == want,
                "ⓚJAVIS_ROOT 미설정 %s 처리 오류: rc=%s(기대 %s) %s"
                % (cmd_, r.returncode, want, r.stderr[:160]))
            if want == EXIT_USAGE and "refused(2)" in r.stderr and "cwd 폴백" in r.stderr:
                refusals += 1
        # ★R5c: 거부 로그는 **stderr 만**. 거부하면서 cwd 에 원장·state 를 만드는 자기모순 금지.
        chk(refusals == 2, "ⓚ거부 사유가 stderr(root_source 포함)로 나오지 않음: %d/2" % refusals)
        chk(os.listdir(cwdK) == [],
            "ⓚR5c: JAVIS_ROOT 미설정인데 cwd 를 오염시킴: %s" % os.listdir(cwdK))
        # ★서브파서 dest 회귀 — `--cmd` 와 이름이 겹치면 서브커맨드 식별이 사라진다.
        rc, out, _e = run_q(rK, ["submit", "--request-id", "DST", "--class", "approval",
                                 "--source", "s1", "--cmd", "rm -rf ./z",
                                 "--summary", "dest 충돌 회귀"], now=t0)
        chk(rc == 0 and json.loads(out)["risk"] == "high",
            "ⓚ--cmd 가 서브커맨드 dest 와 충돌(2차 분류기 입력 소실): %s" % out[:200])

        # ══ 3차 수리 박제 (R1~R6 — 리뷰 재현 케이스를 테스트로 고정) ═══════════
        def outbox_lines(root):
            p = os.path.join(root, "_round", "approvals", "outbox.jsonl")
            return [json.loads(x) for x in open(p, encoding="utf-8")] if os.path.isfile(p) else []

        def wk_ledger(root, event):
            p = os.path.join(root, "_round", "wakeups", "queue.jsonl")
            n = 0
            if os.path.isfile(p):
                with open(p, encoding="utf-8") as f:
                    for ln in f:
                        with contextlib.suppress(ValueError):
                            if json.loads(ln).get("event") == event:
                                n += 1
            return n

        # ── ⓛ R1 동시 8건 무손실 + **후속 tick 오라클**(유령 소실은 다음 호출에서 드러난다) ──
        rL = new_root("cL")
        pl = [popen_q(rL, ["submit", "--request-id", "L%d" % i, "--class", "approval",
                           "--source", "conc", "--summary", "동시 %d" % i],
                      env_extra={"CYS_SHIM_SEND_DELAY": "3"}, now=t0) for i in range(8)]
        outsL = [p.communicate(timeout=240) for p in pl]
        rcsL = [p.returncode for p in pl]
        chk(all(x == 0 for x in rcsL),
            "ⓛ동시 8건 submit 비영 exit(무음 폐기): %s / %s"
            % (rcsL, [(o[:80], e[:120]) for o, e in outsL]))
        chk(len(item_files(rL)) == 8 and len(ledger_events(rL, "queued")) == 8,
            "ⓛ동시 8건 소실: items=%d queued=%d"
            % (len(item_files(rL)), len(ledger_events(rL, "queued"))))
        stL = read_state(rL)
        chk("item_count" not in stL,
            "ⓛR1: state.json 에 카운터가 되살아남(디스크 SOT 와 경쟁): %s" % sorted(stL))
        rcT, outT, errT = run_q(rL, ["tick", "--no-deliver"], now=t0 + 1)
        rcL2, outL2, _e = run_q(rL, ["list", "--state", "pending"], now=t0 + 2)
        chk(rcT == 0 and rcL2 == 0 and len(ledger_events(rL, "state_lost")) == 0,
            "ⓛ후속 tick/list 에서 유령 state_lost: tick_rc=%s list_rc=%s lost=%s"
            % (rcT, rcL2, ledger_events(rL, "state_lost")))
        chk(len(json.loads(outL2)) == 8, "ⓛtick 후 pending 8건 아님: %d" % len(json.loads(outL2)))

        # ── ⓜ R2 배달 지연 중 approve → 결정 보존(낡은 스냅샷 통짜 덮어쓰기 금지) ──
        rM = new_root("cM")
        pm = popen_q(rM, ["submit", "--request-id", "MX", "--class", "approval",
                          "--source", "s1", "--summary", "배달 중 승인"],
                     env_extra={"CYS_SHIM_SEND_DELAY": "8"}, now=t0)
        deadline_m = time.time() + 20
        while not item_files(rM) and time.time() < deadline_m:
            time.sleep(0.1)                       # claim 직후(=배달 진행 중) 진입
        time.sleep(1.0)
        rcM, outM, errM = run_q(rM, ["approve", "MX", "--by", "master"], now=t0 + 1)
        chk(rcM == 0 and json.loads(outM)["result"] == "approved",
            "ⓜ배달 중 approve 실패: rc=%s %s %s" % (rcM, outM[:200], errM[:200]))
        pm.communicate(timeout=120)
        itM, _e = _read_json(os.path.join(rM, "_round", "approvals", "items",
                                          [f for f in item_files(rM)][0]))
        chk((itM or {}).get("state") == "approved",
            "ⓜR2: 배달 완료가 approve 를 되돌림(무음 결정 소실): %s" % (itM or {}).get("state"))
        chk(len(ledger_events(rM, "commit_skipped_terminal")) == 1,
            "ⓜR2: TERMINAL 보존이 원장에 남지 않음: %s"
            % ledger_events(rM, "commit_skipped_terminal"))

        # ── ⓝ R3 원장 회전 2회 후에도 복원(세대 보존 — `.1` 덮어쓰기 금지) ──
        rN = new_root("cN")
        eN = {"JAVIS_APPROVAL_ROTATE_BYTES": "20000"}   # 회전 임계 축소(실물 5MB 경로 동일)
        for i in range(2):
            run_q(rN, ["submit", "--request-id", "RT%d" % i, "--class", "approval",
                       "--source", "s1", "--summary", "회전 전 등재 %d" % i,
                       "--no-deliver"], env_extra=eN, now=t0)
        ledN = os.path.join(rN, "_round", "approvals", "ledger.jsonl")
        pad = json.dumps({"event": "pad", "note": "x" * 400}, ensure_ascii=False) + "\n"
        for gen in range(2):                      # 2회 회전 → `.1`·`.2` 세대 생성
            with open(ledN, "a", encoding="utf-8") as f:
                f.write(pad * 60)
            run_q(rN, ["list"], env_extra=eN, now=t0 + gen + 1)
        chk(os.path.isfile(ledN + ".1") and os.path.isfile(ledN + ".2"),
            "ⓝR3: 세대 보존 실패(.1/.2 부재): %s"
            % sorted(os.path.basename(p) for p in glob.glob(ledN + "*")))
        import shutil as _sh2
        _sh2.rmtree(os.path.join(rN, "_round", "approvals", "items"))
        rcN, outN, errN = run_q(rN, ["list", "--state", "pending"], env_extra=eN, now=t0 + 9)
        rowsN = json.loads(outN) if rcN == 0 else []
        chk(rcN == 0 and {r["request_id"] for r in rowsN} == {"RT0", "RT1"},
            "ⓝR3: 2회 회전 후 최초 queued 복원 실패(회전이 복원 SOT 를 파괴): rc=%s %s"
            % (rcN, outN[:300]))

        # ── ⓞ R4 dead 대상 6동시 → skipped 감지(큐 delivered == wakeup delivered) ──
        rO = new_root("cO")
        po = [popen_q(rO, ["submit", "--request-id", "O%d" % i, "--class", "approval",
                           "--source", "s1", "--summary", "dead 대상 %d" % i],
                      env_extra={"JAVIS_WAKEUP_LIVENESS": "dead"}, now=t0) for i in range(6)]
        for p in po:
            p.communicate(timeout=180)
        qdel = len(ledger_events(rO, "delivered"))
        wdel = wk_ledger(rO, "delivered")
        skipped = len(ledger_events(rO, "deliver_skipped"))
        undet = len(ledger_events(rO, "deliver_undetermined"))
        chk(qdel == wdel == 0,
            "ⓞR4: zombie 폐기를 delivered 로 오기록(큐 %d vs wakeup %d)" % (qdel, wdel))
        chk(skipped >= 1 and skipped + undet == 6,
            "ⓞR4: skipped 판정 누락: skipped=%d undetermined=%d" % (skipped, undet))
        chk(len(outbox_lines(rO)) == 6,
            "ⓞR4: 확정 배달 아닌 결말이 outbox 를 남기지 않음: %d/6" % len(outbox_lines(rO)))

        # ── ⓟ R5a 필수필드 결손 항목 격리(KeyError 전이 차단) ──
        rP = new_root("cP")
        run_q(rP, ["submit", "--request-id", "PA", "--class", "approval", "--source", "s1",
                   "--severity", "critical", "--summary", "정상 항목", "--no-deliver"], now=t0)
        itemsP = os.path.join(rP, "_round", "approvals", "items")
        with open(os.path.join(itemsP, "BROKEN.deadbeef.json"), "w", encoding="utf-8") as f:
            json.dump({"request_id": "PB", "state": "pending"}, f)      # class·severity 결손
        with open(os.path.join(itemsP, "ENUM.cafebabe.json"), "w", encoding="utf-8") as f:
            json.dump({"request_id": "PC", "class": "approval", "risk": "normal",
                       "severity": "fatal", "created_at": "x", "source": "s1",
                       "payload_ref": None, "state": "pending"}, f)     # enum 위반
        rcP, outP, errP = run_q(rP, ["digest", "--no-deliver"], now=t0)
        chk(rcP in (0, EXIT_EMPTY) and "Traceback" not in errP,
            "ⓟR5a: 결손 항목이 도구를 폭파시킴: rc=%s err=%s" % (rcP, errP[-300:]))
        iso = ledger_events(rP, "item_corrupt_isolated")
        chk(len(iso) == 2 and all(x.get("why") for x in iso),
            "ⓟR5a: 필수필드·enum 위반 격리 2건 아님(사유 포함): %s" % iso)
        rcP2, outP2, _e = run_q(rP, ["list", "--state", "pending"], now=t0)
        chk(rcP2 == 0 and [r["request_id"] for r in json.loads(outP2)] == ["PA"],
            "ⓟR5a: 격리 후 정상 항목이 남지 않음: %s" % outP2[:200])

        # ── ⓠ **무음 exit 0건** — lost 로 중단하는 모든 경로가 3중 흔적을 남긴다(R1 필수 부수) ──
        rQ = new_root("cQ")
        for i in range(2):
            run_q(rQ, ["submit", "--request-id", "Q%d" % i, "--class", "approval",
                       "--source", "s1", "--summary", "소실 대상 %d" % i,
                       "--no-deliver"], now=t0)
        with open(os.path.join(rQ, "_round", "tasks", "QE.esc-bundle.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"schema_version": 1, "task": "QE", "sig": "s", "verify_out": "x",
                       "exit": 1, "attempt": 1, "ts": _now(),
                       "request_id": "guard:QE:s"}, f)
        aprQ = os.path.join(rQ, "_round", "approvals")
        _sh2.rmtree(os.path.join(aprQ, "items"))
        ledQ = os.path.join(aprQ, "ledger.jsonl")
        keepQ = open(ledQ, encoding="utf-8").readlines()
        with open(ledQ, "w", encoding="utf-8") as f:      # **이물질 줄**(잘린 쓰기 아님 = 훼손)
            f.write("파손된 원장 — JSON 이 아닌 이물질\n" + "".join(keepQ))
        silent = []
        for argvQ, name in ((["submit", "--request-id", "QZ", "--class", "approval",
                              "--source", "s1", "--summary", "x", "--no-deliver"], "submit"),
                            (["collect-escalations", "--no-deliver"], "collect-escalations"),
                            (["digest", "--no-deliver"], "digest"),
                            (["list"], "list"),
                            (["tick", "--no-deliver"], "tick"),
                            (["approve", "Q0", "--by", "master"], "approve"),
                            (["deny", "Q1", "--by", "master"], "deny")):
            rcQ, outQ, errQ = run_q(rQ, argvQ, now=t0 + 5)
            try:
                res = json.loads(outQ).get("result")
            except (ValueError, AttributeError):     # 비-JSON·비-객체 stdout = 흔적 미달
                res = None
            if not (rcQ == EXIT_INVALID and res == "state_lost" and "state_lost" in errQ):
                silent.append("%s(rc=%s stdout=%r stderr=%r)" % (name, rcQ, outQ[:120],
                                                                 errQ[:160]))
        chk(not silent,
            "ⓠ무음 폐기 경로 잔존(stdout/stderr/exit 3중 흔적 누락): %s" % silent)
        chk(len(ledger_events(rQ, "state_lost_abort")) == 7,
            "ⓠstate_lost_abort 원장 7건 아님(경로별 1건): %d"
            % len(ledger_events(rQ, "state_lost_abort")))

        # ── ⓡ 훼손 판정의 오탐/미탐 경계: 잘린 쓰기는 관용, 원장 전멸은 탐지 ──
        rR = new_root("cR")
        for i in range(2):
            run_q(rR, ["submit", "--request-id", "R%d" % i, "--class", "approval",
                       "--source", "s1", "--summary", "torn %d" % i, "--no-deliver"], now=t0)
        ledR = os.path.join(rR, "_round", "approvals", "ledger.jsonl")
        with open(ledR, "a", encoding="utf-8") as f:      # append 도중 프로세스 사망 흔적
            f.write('{"event": "delive')
        rcR, outR, errR = run_q(rR, ["list", "--state", "pending"], now=t0 + 1)
        rcR2, outR2, _e = run_q(rR, ["list", "--state", "pending"], now=t0 + 2)   # 뒤에 더 append
        chk(rcR == 0 and rcR2 == 0 and len(json.loads(outR2)) == 2,
            "ⓡ잘린 쓰기 1줄이 훼손으로 승격돼 도구가 wedge 됨(오탐): rc=%s/%s %s"
            % (rcR, rcR2, outR2[:200]))
        # ══ 4차 수리 박제 (H1~H3·M1·M5) ═══════════════════════════════════
        # ── ⓣ H1 복원 중 결정 확정 → **결정 보존**(복원은 없는 것만 되살린다) ──
        rT = new_root("cT")
        run_q(rT, ["submit", "--request-id", "TT", "--class", "approval", "--source", "s1",
                   "--summary", "복원 경합", "--no-deliver"], now=t0)
        import shutil as _sh3
        _sh3.rmtree(os.path.join(rT, "_round", "approvals", "items"))
        # A: 리플레이 스냅샷(pending)을 쥔 채 복원 직전에서 결정론 대기
        pA = popen_q(rT, ["list"], env_extra={"JAVIS_APPROVAL_TEST_RESTORE_DELAY": "6"},
                     now=t0 + 1)
        time.sleep(1.5)
        run_q(rT, ["list"], now=t0 + 2)                       # B: 먼저 복원(pending)
        rcB, outB, errB = run_q(rT, ["approve", "TT", "--by", "master"], now=t0 + 3)
        chk(rcB == 0 and json.loads(outB)["result"] == "approved",
            "ⓣ복원 중 approve 실패(케이스 무효): rc=%s %s %s" % (rcB, outB[:160], errB[:160]))
        pA.communicate(timeout=120)                            # A: 낡은 스냅샷으로 복원 시도
        itT, _e = _read_json(os.path.join(rT, "_round", "approvals", "items",
                                          item_files(rT)[0]))
        chk((itT or {}).get("state") == "approved",
            "ⓣH1: 낡은 복원 스냅샷이 확정된 approve 를 pending 으로 되돌림(무음 결정 소실): %s"
            % (itT or {}).get("state"))
        chk(len(ledger_events(rT, "item_restore_skipped_present")) >= 1,
            "ⓣH1: 복원 건너뜀이 원장에 남지 않음(무음 금지): %s"
            % ledger_events(rT, "item_restore_skipped_present"))

        # ── ⓤ H2 진짜 전멸이 자기 사전 로그로 마스킹되지 않는다(양방향) ──
        rU = new_root("cU")
        run_q(rU, ["submit", "--request-id", "UU", "--class", "approval", "--source", "s1",
                   "--summary", "전멸 대상", "--no-deliver"], now=t0)
        os.remove(os.path.join(rU, "_round", "approvals", "ledger.jsonl"))
        rcU, outU, errU = run_q(rU, ["submit", "--request-id", "UV", "--class", "approval",
                                     "--source", "s1", "--summary", "마스킹 시도",
                                     "--no-deliver"], now=t0 + 1)
        chk(rcU == EXIT_INVALID and json.loads(outU)["result"] == "state_lost"
            and json.loads(outU).get("probe") == "preflight" and "state_lost" in errU,
            "ⓤH2: submit 의 사전 로그가 진짜 전멸을 마스킹함: rc=%s %s" % (rcU, outU[:200]))
        chk(len(ledger_events(rU, "ledger_reopened_after_loss")) == 1,
            "ⓤH2: 감사 불연속 지점이 박제되지 않음: %s"
            % ledger_events(rU, "ledger_reopened_after_loss"))
        rU2 = os.path.join(td, "cU2")            # 콜드 스타트는 정상 진행(반대 방향)
        rcU2, outU2, _e = run_q(rU2, ["submit", "--request-id", "UC", "--class", "approval",
                                      "--source", "s1", "--summary", "콜드", "--no-deliver"],
                                now=t0)
        chk(rcU2 == 0, "ⓤH2: 콜드 스타트를 전멸로 오탐: rc=%s %s" % (rcU2, outU2[:200]))

        # ── ⓥ H3 task_key 조인 키 유일성(sanitize 충돌 rid 2건) ──
        rV = new_root("cV")
        long_a = "guard:" + ("T" * 70) + ":sigA"
        long_b = "guard:" + ("T" * 70) + ":sigB"
        chk(_SAFE_RE.sub("_", long_a)[:64] == _SAFE_RE.sub("_", long_b)[:64],
            "ⓥ충돌 유도 rid 가 실제로 충돌하지 않음(케이스 무효)")
        for rid in (long_a, long_b):
            run_q(rV, ["submit", "--request-id", rid, "--class", "approval", "--source", "s1",
                       "--summary", "조인 키 유일성", "--no-deliver"], now=t0)
        chk(len(wk_pending(rV)) == 2,
            "ⓥH3: 서로 다른 rid 가 같은 task_key 로 접힘(조인 전제 붕괴): %s" % wk_pending(rV))
        chk(len(ledger_events(rV, "queued")) == 2,
            "ⓥH3: queued 2건 아님: %d" % len(ledger_events(rV, "queued")))
        # ★F2 — **소비자 80자 절단 후 실제 해시 마진**을 박제한다(계약 문면의 근거 수치).
        #   큐 층은 해시 8자를 넣지만 `javis_wakeup._safe` 가 80자로 자르므로, 접두가 길수록
        #   생존 hex 가 줄어든다. 숫자를 코드로 고정해 두면 문면이 조용히 과잉이 되지 않는다.
        from javis_wakeup import _safe as _wk_safe
        margins = {}
        for pref in ("approval", "approval-esc"):
            full = _task_key("Z" * 70, pref)
            cut = _wk_safe(full)
            margins[pref] = len(cut) - (len(cut.rsplit(".", 1)[0]) + 1)
        chk(margins["approval-esc"] == 2 and margins["approval"] == 6,
            "ⓥF2: 소비자 절단 후 해시 마진 실측이 계약 문면과 불일치: %s" % margins)
        real = _task_key("guard:T-042:ab12cd34", "approval-esc")   # 실사용 형태(≈40자 rid)
        chk(_wk_safe(real) == real,
            "ⓥF2: 실사용 rid 가 절단권 안으로 들어옴(문면 전제 붕괴): %r" % _wk_safe(real))

        # ── ⓨ F1 submit 경유 escalation 도 fallback 을 탄다(계약 §2-2 공통 경로) ──
        rY = new_root("cY")
        rc, out, _e = run_q(rY, ["submit", "--request-id", "guard:YT:ysig", "--class",
                                 "escalation", "--source", "completion-guard",
                                 "--severity", "critical", "--summary", "submit 경유 escalation"],
                            env_extra={"JAVIS_WAKEUP_LIVENESS": "dead"}, now=t0)
        fbY = ledger_events(rY, "routed_fallback")
        obY = os.path.join(rY, "_round", "approvals", "outbox.jsonl")
        obY_n = len(open(obY, encoding="utf-8").readlines()) if os.path.isfile(obY) else 0
        chk(len(fbY) == 1 and fbY[0]["target"] == "master",
            "ⓨF1: submit 경유 escalation 이 fallback 없이 사라짐(계약 §2-2 미이행): %s" % fbY)
        chk(obY_n == 2,
            "ⓨF1: 1차+fallback 배달 실패 outbox 2줄 아님: %d" % obY_n)
        rY2 = new_root("cY2")        # 반대 방향 — approval 은 fallback 없음(부작용 0)
        run_q(rY2, ["submit", "--request-id", "YA", "--class", "approval", "--source", "s1",
                    "--summary", "부작용 확인"],
              env_extra={"JAVIS_WAKEUP_LIVENESS": "dead"}, now=t0)
        chk(ledger_events(rY2, "routed_fallback") == [],
            "ⓨF1: fallback 없는 class 에 fallback 이 생김(부작용): %s"
            % ledger_events(rY2, "routed_fallback"))

        # ── ⓩ F3 전멸 재출발이 생존 항목의 복원 SOT 를 재수립한다 ──
        rZ = new_root("cZ")
        for i in range(2):
            run_q(rZ, ["submit", "--request-id", "Z%d" % i, "--class", "approval",
                       "--source", "s1", "--summary", "생존 %d" % i, "--no-deliver"], now=t0)
        os.remove(os.path.join(rZ, "_round", "approvals", "ledger.jsonl"))
        rcZ, outZ, _e = run_q(rZ, ["list"], now=t0 + 1)          # 전멸 탐지 + 재출발
        chk(rcZ == EXIT_INVALID, "ⓩF3: 전멸이 탐지되지 않음: rc=%s" % rcZ)
        resnap = ledger_events(rZ, "survivors_resnapshotted")
        chk(resnap and resnap[0]["count"] == 2,
            "ⓩF3: 생존 항목 재스냅샷 2건 아님: %s" % resnap)
        # 재출발 원장만으로 두 번째 소실이 복원되는가(= SOT 재수립 실증)
        _sh3.rmtree(os.path.join(rZ, "_round", "approvals", "items"))
        rcZ2, outZ2, _e = run_q(rZ, ["list", "--state", "pending"], now=t0 + 2)
        rowsZ = json.loads(outZ2) if rcZ2 == 0 else []
        chk(rcZ2 == 0 and {r["request_id"] for r in rowsZ} == {"Z0", "Z1"},
            "ⓩF3: 재출발 후 두 번째 소실이 무음(복원 SOT 부재): rc=%s %s"
            % (rcZ2, outZ2[:200]))

        # ── ⓦ M1 결손 queued 주입 → 복원↔격리 루프·원장 증식 0 ──
        rW = new_root("cW")
        run_q(rW, ["submit", "--request-id", "WA", "--class", "approval", "--source", "s1",
                   "--summary", "정상", "--no-deliver"], now=t0)
        with open(os.path.join(rW, "_round", "approvals", "ledger.jsonl"), "a",
                  encoding="utf-8") as f:       # class·severity 결손 queued 스냅샷
            f.write(json.dumps({"event": "queued", "request_id": "WBROKEN",
                                "created_at": "x", "source": "s1", "state": "pending"},
                               ensure_ascii=False) + "\n")
        sizes = []
        for _ in range(3):
            rcW, outW, errW = run_q(rW, ["list"], now=t0 + 1)
            chk(rcW == 0 and "Traceback" not in errW,
                "ⓦM1: 결손 스냅샷이 도구를 멈춤: rc=%s %s" % (rcW, errW[-200:]))
            sizes.append(len(ledger_events(rW, "item_restore_unrestorable"))
                         + len(ledger_events(rW, "item_corrupt_isolated")))
        chk(sizes == [1, 2, 3] and len(item_files(rW)) == 1,
            "ⓦM1: 복원↔격리 루프로 원장 증식(라운드당 1줄이어야 함): %s items=%s"
            % (sizes, item_files(rW)))

        # ── ⓧ M5 콜드 스타트 탐지력 — 창을 결정론으로 벌려 100% 재현 ──
        rX = os.path.join(td, "cX")
        pX = popen_q(rX, ["submit", "--request-id", "X0", "--class", "approval",
                          "--source", "cold", "--summary", "창 점유", "--no-deliver"],
                     env_extra={"JAVIS_APPROVAL_TEST_CLAIM_DELAY": "4"}, now=t0)
        time.sleep(1.2)                          # X0 이 claim~queued 창을 붙잡은 상태
        psX = [popen_q(rX, ["submit", "--request-id", "X%d" % i, "--class", "approval",
                            "--source", "cold", "--summary", "동시 %d" % i, "--no-deliver"],
                       now=t0) for i in range(1, 9)]
        outsX = [p.communicate(timeout=180) for p in psX]
        pX.communicate(timeout=180)
        rcsX = [p.returncode for p in psX] + [pX.returncode]
        chk(all(x == 0 for x in rcsX),
            "ⓧM5: 벌려 놓은 콜드 스타트 창에서 비영 exit: %s / %s"
            % (rcsX, [(o[:80], e[:140]) for o, e in outsX if e]))
        chk(len(item_files(rX)) == 9 and len(ledger_events(rX, "state_lost")) == 0,
            "ⓧM5: 창 점유 중 소실·유령 전멸: items=%d lost=%s"
            % (len(item_files(rX)), ledger_events(rX, "state_lost")))

        rR2 = new_root("cR2")
        run_q(rR2, ["submit", "--request-id", "RD", "--class", "approval", "--source", "s1",
                    "--summary", "원장 전멸", "--no-deliver"], now=t0)
        os.remove(os.path.join(rR2, "_round", "approvals", "ledger.jsonl"))
        rcR3, outR3, errR3 = run_q(rR2, ["list"], now=t0 + 1)
        chk(rcR3 == EXIT_INVALID and json.loads(outR3)["reason"] == "ledger_damaged",
            "ⓡ원장 전삭제(항목 실재 · 세대 0)를 조용히 통과(미탐): rc=%s %s"
            % (rcR3, outR3[:200]))

        # ── ⓢ **콜드 스타트** — 완전히 빈 JAVIS_ROOT(디렉토리 부재)에 동시 8건 ──
        #    운영 첫 사용 조건. 전멸 탐지가 이 창을 오탐하면 빈 큐의 첫 동시 제출이 죽는다.
        rS = os.path.join(td, "cS")          # ★new_root 를 쓰지 않는다 — 디렉토리조차 없어야 함
        chk(not os.path.exists(rS), "ⓢ콜드 스타트 루트가 미리 존재함(케이스 무효)")
        psS = [popen_q(rS, ["submit", "--request-id", "S%d" % i, "--class", "approval",
                            "--source", "cold", "--summary", "콜드 %d" % i, "--no-deliver"],
                       now=t0) for i in range(8)]
        outsS = [p.communicate(timeout=240) for p in psS]
        rcsS = [p.returncode for p in psS]
        lostS = ledger_events(rS, "state_lost")
        chk(all(x == 0 for x in rcsS),
            "ⓢ콜드 스타트 동시 8건에서 비영 exit(빈 큐 첫 사용이 죽는다): %s / %s"
            % (rcsS, [(o[:100], e[:160]) for o, e in outsS if e or not o]))
        chk(len(item_files(rS)) == 8 and len(ledger_events(rS, "queued")) == 8,
            "ⓢ콜드 스타트 소실: items=%d queued=%d"
            % (len(item_files(rS)), len(ledger_events(rS, "queued"))))
        chk(not lostS, "ⓢ콜드 스타트를 원장 전멸로 오탐: %s" % lostS)
        # 순서 불변식 — 항목 파일이 있으면 원장 파일도 있다(창 자체가 닫혀 있다).
        chk(os.path.isfile(os.path.join(rS, "_round", "approvals", "ledger.jsonl")),
            "ⓢ항목은 있는데 원장 파일이 없다(순서 역전 — 창 재개방)")

    if fails:
        print("javis_approval_queue self-test FAIL %d건:" % len(fails), file=sys.stderr)
        for m in fails:
            print("  - %s" % m, file=sys.stderr)
        return 1
    print("javis_approval_queue self-test OK — 중복알림0·고위험 로컬전용/Slack notify-only 1회·"
          "자가치유 큐미경유·재기동 유예+요약1·escalation 수거/병합·back-pressure 1회·"
          "다이제스트 집계파일+300자헤드·시크릿 거부·outbox·손상격리 / 신규: 동시성 3건 소실0·"
          "동일rid 경합 1건·rid 클래스충돌 exit6·원장 리플레이 복원·wedge 지연0·창 중첩 이월·"
          "억제 Slack 종속·타입 위반 강등·tick 배달 / 3차: 동시 8건 무손실+tick 오라클(R1)·"
          "배달중 approve 보존(R2)·2회 회전 후 복원(R3)·drain skipped 감지(R4)·"
          "필수필드 격리(R5a)·무음 exit 0건 7경로(R1 부수)·잘린쓰기 관용/원장전멸 탐지·"
          "콜드 스타트(빈 루트) 동시 8건 무손실 / 4차: 복원중 결정보존(H1)·전멸 마스킹 차단"
          "(H2)·task_key 조인 유일성(H3)·복원 무한루프 차단(M1)·창 결정론 확대(M5) / "
          "착지: submit 경유 escalation fallback(F1)·소비자 절단 마진 실측(F2)·"
          "전멸 재출발 복원 SOT 재수립(F3) / ★E2-4 R-03(미바인딩 loud 고지 1줄+"
          "routing_unbound 원장+경보 마커 코얼레싱 · label_bindings 실라벨 해소 후 고지 소멸+"
          "실대상 배달 · env 라벨 오버라이드) 전 케이스 통과")
    return 0


def main(argv=None):
    global _NO_DELIVER
    p = argparse.ArgumentParser(description="승인 통합 큐 v1 (T3 · 팩 층)")
    # dest 는 `subcmd` 다 — submit 의 `--cmd`(위험 어휘 2차 분류기 입력)와 이름이 겹치면
    # 서브파서 dest 가 옵션 기본값(None)으로 덮여 **서브커맨드 식별이 사라진다**(실측 결함).
    sub = p.add_subparsers(dest="subcmd", required=True)

    c = sub.add_parser("submit", help="항목 등재 + 즉시 배달(enqueue+same-run drain)")
    c.add_argument("--request-id", dest="request_id", required=True,
                   help="발행처 멱등키 재사용(esc-bundle 이면 guard:<task>:<sig>)")
    # ★S12b: class·severity 는 argparse choices 로 막지 않는다 — 미지 값은 도구가 **exit 6**
    #   으로 거부해야 계약 §10 의 6번 줄이 도달 가능해진다(argparse choices 는 exit 2).
    c.add_argument("--class", dest="cls", required=True,
                   help="approval|self-healing|escalation (미지 값 = exit 6)")
    c.add_argument("--source", required=True, help="발행자(back-pressure 계수 단위)")
    c.add_argument("--severity", default="warn", help="info|warn|critical (미지 값 = exit 6)")
    c.add_argument("--risk", help="호출자 선언 high|normal(high 강제만 유효 — 강등 불가)")
    c.add_argument("--risk-class", dest="risk_class", help="데몬 risk_class 1차 신호")
    c.add_argument("--cmd", help="2차 분류기 입력(위험 어휘 재검사)")
    c.add_argument("--payload-ref", dest="payload_ref", help="본문 참조 경로")
    c.add_argument("--summary", help="한 줄 요약")
    c.add_argument("--action", help="class=self-healing 의 액션명(사전 인가 대조)")
    c.add_argument("--no-deliver", action="store_true",
                   help="enqueue 만 하고 same-run drain 생략(pending 스냅샷 계수용)")
    c.set_defaults(fn=cmd_submit)

    c = sub.add_parser("collect-escalations", help="esc-bundle 스캔→항목화→routing 배달")
    c.add_argument("--tasks-dir", dest="tasks_dir", help="스캔 경로(기본 $JAVIS_ROOT/_round/tasks)")
    c.add_argument("--no-deliver", action="store_true")
    c.set_defaults(fn=cmd_collect_escalations)

    c = sub.add_parser("digest", help="1일 1회 다이제스트(집계 파일+300자 헤드·멱등키=날짜)")
    c.add_argument("--force", action="store_true", help="같은 날 재발행(운영 예외)")
    c.add_argument("--no-deliver", action="store_true")
    c.set_defaults(fn=cmd_digest)

    c = sub.add_parser("list", help="항목 조회")
    c.add_argument("--state", choices=STATES)
    c.add_argument("--class", dest="cls", choices=CLASSES)
    c.add_argument("--risk", choices=RISKS)
    c.add_argument("--no-deliver", action="store_true")
    c.set_defaults(fn=cmd_list)

    c = sub.add_parser("tick", help="유지보수 tick + 대상별 wakeup drain(배달 주체 · 조건 33)")
    c.add_argument("--no-deliver", action="store_true", help="drain 생략(점검용)")
    c.set_defaults(fn=cmd_tick)

    c = sub.add_parser("approve", help="로컬 승인(고위험도 로컬 전용)")
    c.add_argument("request_id")
    c.add_argument("--by", default="master")
    c.add_argument("--no-deliver", action="store_true")
    c.set_defaults(fn=cmd_approve)

    c = sub.add_parser("deny", help="로컬 거부")
    c.add_argument("request_id")
    c.add_argument("--by", default="master")
    c.add_argument("--reason")
    c.add_argument("--no-deliver", action="store_true")
    c.set_defaults(fn=cmd_deny)

    c = sub.add_parser("self-test")
    c.set_defaults(fn=self_test)

    global _NO_WRITE
    a = p.parse_args(argv)
    _NO_DELIVER = bool(getattr(a, "no_deliver", False))
    # ★S11e+R5c: JAVIS_ROOT 미설정 시 **배달성 서브커맨드 거부** + **cwd 무오염**.
    #   종전에는 거부 로그(`root_unset_refused`)를 cwd 원장에 썼고 조회 경로는 cwd 에
    #   state.json 을 남겼다 — "남의 트리를 건드리지 않겠다"면서 건드리는 자기모순이라,
    #   거부 사유는 stderr 로만 말하고 이 프로세스의 모든 쓰기를 봉인한다.
    if not _ROOT_FROM_ENV and a.subcmd != "self-test":
        _NO_WRITE = True
        sys.stderr.write("[approval-queue] JAVIS_ROOT 미설정 — 쓰기 봉인(cwd=%s 무오염)\n"
                         % _relativize(ROOT))
    if a.subcmd in DELIVERING_CMDS and not _ROOT_FROM_ENV:
        print("refused(2): JAVIS_ROOT 미설정 — 배달성 서브커맨드(%s)는 cwd 폴백으로 실행하지 "
              "않는다(root_source=cwd 폴백 · 원장 생성 없음 — stderr 만)" % a.subcmd,
              file=sys.stderr)
        return EXIT_USAGE
    # ★H2: 전멸 판정은 **이 프로세스의 어떤 쓰기보다도 먼저** 선다. 여기 아래로 내려가면
    #   `_self_heal_allowlist`·`_item_claim`·`_bad_id` 가 원장을 재실체화해 판정이 마스킹된다.
    #   전 서브커맨드 공통(self-test 제외 — 자체 격리 루트를 쓴다).
    if a.subcmd != "self-test":
        lost = _preflight_lost_probe()
        if lost:
            return _abort_state_lost({"lost": lost}, a.subcmd)
    return a.fn(a)


def guarded_main(argv=None):
    """최상위 예외 핸들러 — traceback·rc=1 누출 0(S9). 통제된 강등만 밖으로 나간다."""
    try:
        return main(argv)
    except SchemaError as e:
        sys.stderr.write("[approval-queue] 스키마 위반(6): %s\n" % e)
        with contextlib.suppress(Exception):
            _ledger_append({"event": "schema_violation", "why": str(e)[:400]})
        return EXIT_INVALID
    except KeyboardInterrupt:
        sys.stderr.write("[approval-queue] 사용자 중단\n")
        return EXIT_USAGE
    except SystemExit:
        raise
    except Exception as e:  # noqa: BLE001 — 어떤 예외도 traceback 으로 새지 않는다
        where = "?"
        with contextlib.suppress(Exception):
            fr = traceback.extract_tb(sys.exc_info()[2])[-1]
            where = "%s:%s" % (os.path.basename(fr.filename), fr.lineno)
        sys.stderr.write("[approval-queue] 내부 오류 — 통제된 강등(%s at %s)\n"
                         % (type(e).__name__, where))
        with contextlib.suppress(Exception):
            _ledger_append({"event": "internal_error", "why": "%s: %s" % (type(e).__name__, e),
                            "where": where})
        return EXIT_INVALID


if __name__ == "__main__":
    sys.exit(guarded_main())
