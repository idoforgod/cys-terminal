#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_completion_guard.py — Phase 1 Wave B1: Stop 훅 completion guard 본체.

설계 SOT: _work/phase1-impl-package/DESIGN-DECISIONS.md §2-1~§2-4 (성찰 1 반영 v2).
전제 실측: T0-BASELINE §1(6프로필·Stop 병렬)·§2(카운터 합산 cap+1)·§5-5(grill fail-open 전례)·
D2·D3·D5·D20·D27. 래퍼 = hooks/completion-guard.sh(14줄·무수정) — settings Stop 등록은 [OT-2].

동작 계약(요지):
  · 무장 스위치 = env `CYS_COMPLETION_GUARD=1` **단독**(조건 37 pane 카나리아). 미무장 = 즉시
    exit 0(파일 stat 0회 — 완전 무발동).
  · 태스크 바인딩 = 파일 계약: javis_task checkout 이 쓰는
    `$JAVIS_ROOT/_round/tasks/.guard-claim.<surface>` (task_id·pid·ts). 부재 = 즉시 exit 0
    (조건 23④ — 전수 스캔 금지·stat 1개). claim task_id 는 javis_task G10 allowlist 정규식
    재사용으로 재검증(F8 심층방어) — 불일치 = claim-corrupt 경로(경보 1회 + exit 0).
    ★이음매 자인(F3): 표준 플로우에서 checkout 은 master pane 에서 실행되므로 그대로는
    claim 이 master surface 에 기록돼 워커 pane 의 guard 가 무발동한다 — master 는
    `javis_task checkout --claim-surface <워커 sid>` 로 위임 대상 워커 surface 에 claim 을
    기록하거나, 워커가 자기 pane 에서 동일-owner 재checkout(멱등 재진입)해야 한다.
    ★E2-1(BLOCKER R-02) claim 생존 판정 재정의: 종전은 `pid 사망 = stale`이었는데 그 pid 는
    **checkout 을 실행한 master 셸**의 것이라, master `/clear`·재기동만으로 워커 guard 가
    영구 exit 0 이 됐다(무장했는데 아무것도 검증 안 함 = 정상과 구분 불가). 지금은
    `_claim_liveness()` 가 ①claim mtime 24h 초과 = stale ②태스크 레코드가 활성
    (in_progress|in_review) ∧ 신선 = **유효**(pid 사망이어도 유지 · 정보성 경보 1회)
    ③그 밖 = pid 생존 여부로 판정한다. 즉 pid 는 **보조 신호로 강등**됐고 1차 근거는
    "이 태스크가 아직 열려 있는가"라는 검증 가능한 사실이다. writer 짝은 javis_task 가
    위임 경로에서 pid 를 아예 기록하지 않는 것(`--claim-pid` 명시 시에만 기록).
  · ★E2-1(c) 침묵 탐지기 — 침묵형 결함의 일반 방어: 무장 상태에서 **아무것도 검증하지 않고
    종결**(claim 부재/손상/stale·disarm·SKIPPED_*)한 횟수를 `.guard-silence.<surface>.json`
    에 연속 계수하고, 임계(기본 5 · env CYS_GUARD_SILENCE_N)에서 경보 1회를 낸다. 실제
    verify 가 1회라도 돌면 스트릭을 끊는다(파일 삭제). 부트 짝은 preflight C72 WARN —
    "무장인데 침묵 지속"을 런타임·부트 2경로에서 표면화한다. **무장 ≠ 발동**이라는 사실
    자체를 관측 가능하게 만드는 장치이며, 이 파일 밖의 다른 침묵 경로에도 같은 형태로
    적용할 수 있다.
  · 판정 enum 10종: PASS | VERIFY_FAIL | SKIPPED_NO_SPEC | SKIPPED_BUDGET | SKIPPED_RESOURCE |
    SKIPPED_CONTEXT | SKIPPED_CYCLE | SKIPPED_GRACE | NO_BLOCK_PASS | INFRA_ERROR.
    **exit 2 는 오직 VERIFY_FAIL** — 그 외 모든 경로는 exit 0(무조건 exit 2 경로 부재를
    self-test 가 박제 · 조건 24④).
  · 양보 사다리 6단(자기판정 — Stop 병렬 실행이라 순서 의존 금지 D3):
      1 CYCLE_IN_PROGRESS.<surface> 마커(TTL 30분·만료 마커는 무시+경보 1회) → SKIPPED_CYCLE
      2 부트 grace 120s — role-bootstrap 완료 마커(부재=부트 미완=보수측) → SKIPPED_GRACE
      3 ctx% 선확인 — javis_cycle_autopilot.measure() import 재사용(D5 statusline 체인 단일·
        transcript 파싱 금지) + `cys status --json`(CYS_NO_AUTOSTART·timeout) + CYS_SURFACE_ID
        자기 row. ctx≥50% → SKIPPED_CONTEXT(통과+context_yield+run.failed escalation 1회).
        측정 실패 → **NO_BLOCK_PASS 모드**(verify 는 실행하되 블록 금지·기록만 · 조건 10)
        + 세션 코얼레싱 경보.
      4 javis_resource_gate check --require-context [--context 실측]: exit 1/2 →
        SKIPPED_RESOURCE+경보 1회(hard 는 deferred 마킹 · 조건 04①) · exit 64/70 →
        INFRA_ERROR(측정 실패=loud · D20). ctx 미측정 시 --context 미주입(soft 경로) —
        이때 exit 1 판별은 **`--json` warnings 판독**(F4): warnings 가 정확히
        `['context_unmeasured']` **단독**일 때만 NO_BLOCK_PASS 모드로 verify 진행,
        `measure_error:*` 포함(ps/load 실측 실패) 등 그 외 전부 skip_soft(SKIPPED_RESOURCE)
        — trips 공집합 판별은 자원 측정 실패를 조용히 통과시키는 오판 경로라 폐기.
      5 verifier tax(B2 §3 실구현): `_round/verify-budget/` 부재 = 무제한 통과(배포 전 호환).
        존재 시 per-surface 샤드(자기 샤드만 기록) 합산 = 총량 검사·고정창 1h(창 키=샤드
        window_start)·소진 → SKIPPED_BUDGET(deferred 마킹·카운터 동결) + **resource.hard
        플릿 단일 발행**(조건 04② — 워커별 발화 금지: 멱등 마커 `.hard-emitted-<창키>`
        O_EXCL 원자 생성 승자만 emit·창당 at-most-once(G7c — 승자 사망 시 그 창 무발화
        수용)·블로킹 대기 없음(조건 25①)).
        예산 한도 = 기본(60회/300초) < policy.json(플릿 배포값) < env
        JAVIS_VERIFY_BUDGET_RUNS/SECS(pane 실험 오버라이드 — 최우선).
      6 verify_spec 부재 → SKIPPED_NO_SPEC(워커 측 fail-closed 금지 · 조건 18②).
  · verify 실행 = setsid 프로세스 그룹 + 종료 시 killpg(손자 잔존 차단 · 조건 07③) ·
    잔존 감지 시 tamper-adjacent 이벤트 1회. 타임아웃(spec timeout_s·상한 30s)=INFRA_ERROR
    + timeout_violations 적재(B2 calibrated=false 강등 소비 · 조건 07②). 자체 데드라인
    SIGALRM 50s(U1 — 훅 기본 타임아웃에 의존하지 않음). ★데드라인 경로 전면 계약(F1):
    활성 verify Popen 그룹은 모듈 수준(_ACTIVE_PGIDS)에서 추적하고 `_Deadline` 발화·최상위
    예외 핸들러가 killpg 로 정리한다(고아 0). 데드라인 초과 = `_finish_infra` 경유
    INFRA_ERROR 로 상태 적재(infra_count++·timeout_violations++·guard.json 기록 — 회로차단기
    편입)·escalated_at 저장~_escalate 사이 발화 시 esc-bundle 을 핸들러가 보강 기록한다.
    각 attempt 진입 전 `(경과+timeout_s) > 데드라인−여유 5s` 검사로 잔여 attempt 를 조기
    INFRA 종결(n_of_m 누적이 데드라인을 초과할 수 없음).
    ★상한 사다리 정정(E1-2 · BLOCKER R-01 — 종전 문면 "60>50>30 3중 상한"은 이 배포 환경에서
    사실이 아니었다): coreutils `timeout` 이 없는 기계(macOS 기본 · 실측 rc=1)에서는 래퍼
    분기가 항상 폴백이었고 등록 도구도 settings 항목에 `timeout` 을 넣지 않아 **바깥 두 겹이
    동시에 부재**했다 — 실효는 SIGALRM 50s 1겹. 지금의 겹은 환경 무관하게 4겹이다:
      ① verify 개별 30s(VERIFY_TIMEOUT_MAX) ② 자체 데드라인 50s(POSIX=SIGALRM + **전 플랫폼
      워치독 스레드** `_arm_self_deadline` — Windows 는 워치독이 유일 겹) ③ 래퍼 외곽 60s
      (`cys_timeout_run`: timeout→gtimeout→CYS_PY 실행기라 coreutils 부재에도 존재)
      ④ settings 훅 항목 `"timeout": 60`(javis_guard_register 가 등록 시 기입).
    ★플랫폼 분기(E1-3 · BLOCKER R-09): `/bin/sh` 하드코딩 → `_shell_argv`(nt=COMSPEC),
    killpg → `_win_taskkill`(`taskkill /T /F`), setsid → `CREATE_NEW_PROCESS_GROUP`.
    다만 **Windows 노드는 기본 무장 거부**(`_windows_arming_refusal` · opt-in
    `CYS_GUARD_ALLOW_WINDOWS=1`) — 거부는 상태 파일 생성 전이라 INFRA 누적·영구 disarm 0.
  · 블록: 재주입 head 1KB+tail 3KB 캡·전문 `<id>.verify_out.log`·기계 접두 stderr(조건 19②).
    블록 카운터 N=3 에서 escalation 전환(조건 08 채택 — 01③/23⑤/29 의 N=5 supersede).
    N≥8 설정은 코드 거부(하네스 cap+1=9 천장 · D2).
  · escalation: 태스크당 1회(escalated_at) · javis_wakeup enqueue
    `--idempotency-key guard:<task>:<sig>` severity critical + **same-run drain --deliver 짝**
    (autopilot :442 전례 — 실패 무해·pending 잔류 시 후속 drain 편승) + `<id>.esc-bundle.json`
    (Wave C 큐 수거 대상 · request_id=wakeup 멱등키 재사용 조인).
    사후-escalation: 신규 sig 포함 전부 통과+기록 — 재블록은 verify PASS 로 카운터 리셋 후
    재임계 도달 시에만(조건 01① 문언).
  · 오버라이드 감지(F2 재설계 — 카운터 기반): guard.json `block_count ≥ 하네스 Stop 블록
    상한(env CLAUDE_CODE_STOP_HOOK_BLOCK_CAP 또는 8)` 상태에서 verify PASS 없이 새 Stop
    도착 = 오버라이드/비정상 스트릭 → tamper-adjacent 이벤트 + 기존 pending 갱신만(신규
    escalation 금지 · 조건 01④·29). 에피소드당 1회 코얼레싱(`_alert_once` 마커
    `override-<task>-<sig>`) — 반복 Stop 에서 이벤트·pending 갱신 재발화 금지. 발화 위치는
    CYCLE 마커·부트 grace 양보 판정 **이후**(인가 clear ≠ 오버라이드 — 마커 존재 시 감지
    억제). ※`stop_hook_active=false` 는 판별자가 아니라 보조 힌트로만 기록한다 — T0 실측상
    실제 오버라이드에서도 true 가 유지(역방향)되고, false 는 /clear·인터럽트와 구분할 수
    없다(자인).
  · guard.json 스키마(§2-4)·손상 = corrupt-<ts> 격리 rename + temp+os.replace 재생성 +
    카운터 0 + 경보 1회(조건 14·phoenix P2-6). 회로차단기: infra_count 연속 3 →
    `<id>.guard-disarmed` 마커+경보·이후 무발동. **재무장 = master 가 마커 파일을 직접
    삭제(명시 명령)** — 자동 재무장 없음.
  · 부수 채널(emit·wakeup)은 전부 best-effort 격리 — 실패는 판정에 불개입, 로컬
    guard.json/esc-bundle 이 SOT(조건 19① OBSERVABILITY 상속).
  · probe mode 계약(F6): spec.probe.args 는 **actprobe 서브커맨드 플래그 그대로**
    (예: ["--path","<p>"] — 위치 인자가 아니라 플래그형)·판정은 actprobe exit
    (0=PASS/2=FAIL/3=INDET→INFRA 수렴) 흡수. 영수증은 CYS_PROBE_RUNS 원장으로 격리(G1).
  · Stop hook stdin(JSON)은 소비하되 transcript_path 의 **내용은 절대 파싱하지 않는다**
    (D5 — "D2 200k 전과" 금지 명문).

서브커맨드(E2-2 — Stop 훅 경로와 분리·명시 argv 로만 진입):
  budget-init [--max-runs N] [--max-secs S] [--force]   verifier tax 활성화(디렉터리+policy.json)
  budget-status [--json]                                 tax 활성/비활성·한도·현재 창 소비 조회
  ※ 되돌리기 = `rm -rf $JAVIS_ROOT/_round/verify-budget` (부재 = 무제한 통과로 즉시 복귀).

exit: 0 = 통과/양보/인프라(fail-open 전부) · 2 = VERIFY_FAIL 블록만.
      --self-test: 0 = 전부 통과 · 1 = 실패.

env 노브(테스트 주입 포함):
  CYS_COMPLETION_GUARD=1        무장(카나리아 — pane 단위)
  CYS_GUARD_ESC_N               escalation 임계(기본 3 · ≥8 거부→3 강등+경고)
  CYS_GUARD_SELF_DEADLINE_SEC   자체 데드라인 초(기본 50 — F1 테스트 주입용·<1 무시)
  CLAUDE_CODE_STOP_HOOK_BLOCK_CAP  하네스 Stop 블록 상한(기본 8) — F2 오버라이드 판별 임계
  CYS_GUARD_GRACE_SEC           부트 grace 창(기본 120)
  CYS_GUARD_BOOT_MARKER         role-bootstrap 완료 마커 경로(기본 ~/.cys/.master-bootstrapped)
  CYS_GUARD_LIVENESS=alive|dead claim pid 생존 결정론 주입(javis_task LIVENESS 관례)
  CYS_GUARD_SILENCE_N           E2-1(c) 침묵 경보 임계 — 연속 무검증 종결 횟수(기본 5·<1 무시)
  CYS_GUARD_RESOURCE_ARGS       resource_gate 추가 인자(테스트 override 주입용 공백 구분)
  CYS_BIN                       cys 바이너리(기본 cys — 테스트는 PATH shim)
  JAVIS_VERIFY_BUDGET_RUNS      B2: verifier tax 창당 실행 상한(기본 60 — policy.json 보다 우선)
  JAVIS_VERIFY_BUDGET_SECS      B2: verifier tax 창당 벽시계 상한 초(기본 300 — 동일 우선순위)
"""
import contextlib
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import threading
import time
import uuid

# ★번들 파이썬 경로 가드(javis_task.py:64 관례) — 형제 모듈 import 보장.
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

ROOT = os.environ.get("JAVIS_ROOT") or os.getcwd()  # javis_task ROOT 관례와 동일
TASKS_DIR = os.path.join(ROOT, "_round", "tasks")
BUDGET_DIR = os.path.join(ROOT, "_round", "verify-budget")

VERDICTS = ("PASS", "VERIFY_FAIL", "SKIPPED_NO_SPEC", "SKIPPED_BUDGET", "SKIPPED_RESOURCE",
            "SKIPPED_CONTEXT", "SKIPPED_CYCLE", "SKIPPED_GRACE", "NO_BLOCK_PASS", "INFRA_ERROR")
EXIT_PASS, EXIT_BLOCK = 0, 2

CTX_YIELD_PCT = 50.0          # §2-2 3단 — ctx≥50% = context yield(resource_gate soft 와 동일선)
CYCLE_TTL_SEC = 30 * 60       # §2-2 1단 R1 — 만료 마커는 무시(고아화 영구 SKIP 차단)
GRACE_SEC_DEFAULT = 120.0     # §2-2 2단 · 조건 04④
CLAIM_STALE_SEC = 24 * 3600   # §2-1 — mtime 24h 초과 = stale(E2-1: pid 단독 판정은 폐기)
ACTIVE_STATUSES = ("in_progress", "in_review")  # E2-1(a) — claim 생존의 1차 근거(javis_task 어휘)
ALERT_TTL_SEC = 6 * 3600      # '경보 1회(세션 코얼레싱)' 마커 창
ESC_N_DEFAULT = 3             # §2-3 — 조건 08 채택(N=5 supersede)
ESC_N_CEILING = 8             # D2 — 하네스 cap+1=9 천장: N≥8 설정 코드 거부
VERIFY_TIMEOUT_DEFAULT = 10.0
VERIFY_TIMEOUT_MAX = 30.0     # 조건 23① — 래퍼/설정 timeout 과 3중 상한의 자체 몫
SELF_DEADLINE_SEC = 50        # U1 — 훅 기본 타임아웃 미의존 자체 데드라인
DEADLINE_MARGIN_SEC = 5.0     # F1 — attempt 예산 검사 여유(경과+timeout_s > 데드라인−여유 = 조기 INFRA)
WATCHDOG_MARGIN_SEC = 8       # E1-2/E1-3 — POSIX 워치독은 SIGALRM 뒤(50+8=58 < 래퍼 외곽 60)
STOP_BLOCK_CAP_DEFAULT = 8    # F2 — 하네스 Stop 블록 상한 기본(CLAUDE_CODE_STOP_HOOK_BLOCK_CAP)
HEAD_CAP, TAIL_CAP = 1024, 3072   # 조건 08 — 재주입 4KB 캡
VERIFY_LOG_MAX = 8 * 1024 * 1024  # 전문 로그 상한(비대 방어 — 절단 표기)
N_OF_M_CAP = 5                # n_of_m.m 실행 상한(자체 데드라인 내 유계)
BUDGET_WINDOW_SEC = 3600      # B2 §3 고정창(1h) 선(先)정합
BUDGET_MAX_RUNS_DEFAULT = 60  # B2 §3 초기값(총 60회/300초) — policy.json 으로 조정
BUDGET_MAX_SECS_DEFAULT = 300.0
INFRA_EXITS = (124, 125, 126, 127)  # 조건 14 — spawn/timeout 계열 rc 는 INFRA(블록 금지)

_SAFE_RE = re.compile(r"[^A-Za-z0-9._-]")  # javis_wakeup._safe 동형(pending 파일명 계산용)

# ── F1: 자체 데드라인 경로 전면 계약 — 모듈 수준 추적 3종 ────────────────────
_ACTIVE_PGIDS = set()   # 활성 verify Popen 그룹(pgid) — _Deadline·최상위 예외 핸들러가 killpg
_RUN_T0 = time.time()   # 데드라인 기산점(모듈 로드≈main 진입) — attempt 예산 검사 기준
_DEADLINE_CTX = {"state": None, "sid": None}  # 발화 시 INFRA 적재용 컨텍스트(state 로드 후 설정)


def _self_deadline():
    """자체 데드라인 초 — 기본 50(U1). CYS_GUARD_SELF_DEADLINE_SEC 는 테스트 주입용(<1 무시)."""
    raw = os.environ.get("CYS_GUARD_SELF_DEADLINE_SEC", "").strip()
    try:
        v = int(raw) if raw else SELF_DEADLINE_SEC
    except ValueError:
        return SELF_DEADLINE_SEC
    return v if v >= 1 else SELF_DEADLINE_SEC


def _windows_arming_refusal():
    """E1-3(BLOCKER R-09): Windows 노드 **기본 무장 거부** 사유 1줄(허용이면 None).

    오너가 특별 경고한 축이다("윈도우 설치파일에서 에러·코드 깨짐이 자주 발생한다"). 이 파일의
    POSIX 전용 호출은 전부 분기됐지만(`_shell_argv`·`_popen_group`·`_kill_active_groups`·
    `_arm_self_deadline`), **실기 Windows 회귀 하네스가 0**이다 — self-test 는 `os.mkfifo`
    의존 케이스를 갖고 있어 Windows 실행 자체가 불완전하다. 검증되지 않은 플랫폼에서 turn 을
    막는 게이트를 켜는 것은 "전 pane 에이전트 사망"(결함군 ④)의 가장 짧은 경로다.

    그래서 **기능 자체를 비활성**한다(무장 거부 + stderr 고지). 핵심은 거부 시점이다:
    상태 파일·claim·guard.json 을 만들기 **전**이라 `INFRA_ERROR` 가 한 번도 계상되지 않고,
    따라서 `infra_count≥3 → guard-disarmed` 영구 잠금도 발생하지 않는다(브리프 E1-3 요구).
    검증을 마친 뒤 켜려면 `CYS_GUARD_ALLOW_WINDOWS=1` 을 명시한다(분기 코드는 이미 산다).
    """
    if os.name != "nt" or os.environ.get("CYS_GUARD_ALLOW_WINDOWS") == "1":
        return None
    return ("Windows 노드는 기본 무장 거부(E1-3/R-09) — 플랫폼 분기는 구현됐으나 실기 회귀 "
            "하네스가 없어 미검증이다. 상태 파일 생성 전에 끊으므로 INFRA 누적·영구 disarm 은 "
            "일어나지 않는다. 검증 후 켜려면 CYS_GUARD_ALLOW_WINDOWS=1.")


def _arm_self_deadline(deadline):
    """자체 데드라인 무장 → **해제 콜러블** 반환(E1-2 R-01 · E1-3 R-09 공통 수리).

    2겹으로 건다:
      ① POSIX SIGALRM(deadline) — `_Deadline` 예외를 던져 **graceful** 종결(`_on_deadline`
         이 killpg + INFRA 적재 + esc-bundle 보강까지 한다). 종전 유일한 겹.
      ② 전 플랫폼 워치독 스레드(deadline + 여유) — 종전엔 없었다.
         · Windows 에는 SIGALRM 이 아예 없어 **자체 상한이 0겹**이었다(R-09: "남는 상한은
           하네스 기본 훅 타임아웃뿐이고 그때 verify 자식은 고아").
         · POSIX 에서도 의미가 있다: SIGALRM 핸들러는 인터프리터가 제어를 되찾아야 실행되지만,
           워치독 스레드는 주 스레드가 GIL 을 놓은 블로킹 C 호출에 갇혀 있어도 돈다.
      워치독은 최후 수단이라 graceful 경로를 best-effort 로 한 번 시도한 뒤 `os._exit(0)` 한다
      — **어떤 경우에도 블록(2)으로 승격하지 않는다**(조건 14·19 fail-open 계약).
    """
    fired = {"v": False}

    def _watchdog():
        fired["v"] = True
        print("[completion-guard] 워치독 데드라인 %ds 초과 — 프로세스 그룹 정리 후 강제 "
              "fail-open(exit 0)" % deadline, file=sys.stderr, flush=True)
        _kill_active_groups()
        with contextlib.suppress(Exception):
            _on_deadline("자체 데드라인 %ds 초과(워치독 %s)"
                         % (deadline, "primary" if os.name != "posix" else "backstop"))
        with contextlib.suppress(Exception):
            sys.stderr.flush()
        os._exit(EXIT_PASS)   # noqa: SLF001 — 스레드에서 프로세스를 끝내는 유일한 수단

    # POSIX 는 SIGALRM 이 먼저 잡으므로 워치독은 여유를 두고 뒤에 선다(백스톱).
    # Windows 는 워치독이 유일한 겹이라 여유 없이 deadline 에 선다.
    margin = WATCHDOG_MARGIN_SEC if os.name == "posix" else 0
    timer = threading.Timer(deadline + margin, _watchdog)
    timer.daemon = True
    timer.start()
    if os.name == "posix":
        signal.signal(signal.SIGALRM, lambda *_a: (_ for _ in ()).throw(_Deadline()))
        signal.alarm(deadline)

    def _disarm():
        with contextlib.suppress(Exception):
            timer.cancel()
        if os.name == "posix":
            with contextlib.suppress(Exception):
                signal.alarm(0)
    return _disarm


def _win_taskkill(pid):
    """E1-3(R-09): Windows 프로세스 **트리** 종료 — killpg 대응물(`taskkill /T /F`).

    POSIX 의 `os.killpg(pgid, SIGKILL)` 은 그룹 전체(손자 포함)를 죽인다. Windows 에는 그
    개념이 없어 종전 코드는 `_ACTIVE_PGIDS.clear()` 로 **set 만 비우고 아무도 죽이지 않았다**
    (고아 잔존). `taskkill /T` 가 자식 트리를 따라간다 — 도구 부재·이미 종료는 무해 통과.
    """
    with contextlib.suppress(Exception):
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)


def _kill_active_groups():
    """F1: 추적 중인 verify 프로세스 그룹 전부 종료(고아 0 계약).

    POSIX = killpg(SIGKILL) / Windows = taskkill /T /F(E1-3 · R-09 — 종전엔 set 만 비웠다).
    """
    for pid in list(_ACTIVE_PGIDS):
        if os.name == "posix":
            with contextlib.suppress(OSError):
                os.killpg(pid, signal.SIGKILL)
        else:
            _win_taskkill(pid)
        _ACTIVE_PGIDS.discard(pid)


def _stop_block_cap():
    """F2: 하네스 Stop 블록 상한 — env CLAUDE_CODE_STOP_HOOK_BLOCK_CAP(비수치·<1 은 기본 8)."""
    raw = os.environ.get("CLAUDE_CODE_STOP_HOOK_BLOCK_CAP", "").strip()
    try:
        v = int(raw) if raw else STOP_BLOCK_CAP_DEFAULT
    except ValueError:
        return STOP_BLOCK_CAP_DEFAULT
    return v if v >= 1 else STOP_BLOCK_CAP_DEFAULT


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


def _cys_bin():
    return os.environ.get("CYS_BIN", "cys")


def _surface_id():
    """cysd 주입 CYS_SURFACE_ID(구 AITERM_SURFACE_ID 수용 — _lib.sh A2 게이트 동일 규약)."""
    raw = (os.environ.get("CYS_SURFACE_ID", "")
           or os.environ.get("AITERM_SURFACE_ID", "")).strip()
    return re.sub(r"[^0-9A-Za-z_-]", "", raw)  # grill_gate._surface_id 동형


def _role():
    """emit agent 필드 — env 파생(CYS_ROLE·데몬 주입 — javis_bootstrap :502 관례)."""
    return os.environ.get("CYS_ROLE") or ("surface:%s" % (_surface_id() or "unknown"))


def _esc_n():
    """escalation 임계 — 기본 3 · N≥8 은 코드 거부(D2 천장)·비수치도 3."""
    raw = os.environ.get("CYS_GUARD_ESC_N", "").strip()
    try:
        n = int(raw) if raw else ESC_N_DEFAULT
    except ValueError:
        return ESC_N_DEFAULT
    if n >= ESC_N_CEILING or n < 1:
        print("[completion-guard] CYS_GUARD_ESC_N=%s 거부(1≤N<8 · 하네스 cap+1=9 천장) — 3 강등"
              % raw, file=sys.stderr)
        return ESC_N_DEFAULT
    return n


def _claim_path(sid):
    return os.path.join(TASKS_DIR, ".guard-claim.%s" % sid)


def _cycle_marker_path(sid):
    return os.path.join(TASKS_DIR, "CYCLE_IN_PROGRESS.%s" % sid)


def _boot_marker_path():
    v = os.environ.get("CYS_GUARD_BOOT_MARKER", "").strip()
    if v:
        return v
    return os.path.join(os.path.expanduser("~"), ".cys", ".master-bootstrapped")


def _guard_state_path(task_id):
    return os.path.join(TASKS_DIR, "%s.guard.json" % task_id)


def _disarm_path(task_id):
    return os.path.join(TASKS_DIR, "%s.guard-disarmed" % task_id)


def _verify_log_path(task_id):
    return os.path.join(TASKS_DIR, "%s.verify_out.log" % task_id)


def _bundle_path(task_id):
    return os.path.join(TASKS_DIR, "%s.esc-bundle.json" % task_id)


def _write_json_atomic(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = "%s.tmp.%s.%s" % (path, os.getpid(), uuid.uuid4().hex[:8])
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def _read_json(path):
    """(obj|None, err|None) — 부재 (None,'absent') · 손상 (None,'corrupt') · dict 아님도 corrupt."""
    try:
        with open(path, encoding="utf-8") as f:
            d = json.load(f)
    except FileNotFoundError:
        return None, "absent"
    except (OSError, ValueError):
        return None, "corrupt"
    return (d, None) if isinstance(d, dict) else (None, "corrupt")


RESERVED_ID_SUFFIXES = (".guard", ".esc-bundle")  # F5 — 사이드카 파생 유령 id(javis_task 정합)


def _claim_id_ok(task_id):
    """F8: claim task_id 심층방어 — javis_task G10 allowlist 정규식 **재사용**(중복 정의 금지)
    + 예약 접미 거부(F5 정합). import 실패(버전 찢김)는 동형 폴백으로 판정 지속."""
    try:
        from javis_task import _ID_RE as _g10_re  # 형제 모듈 — G10 단일 진실 재사용
    except Exception:  # noqa: BLE001 — 버전 찢김 시 동형 폴백(판정 자체는 유지)
        _g10_re = re.compile(r"^[A-Za-z0-9._-]{1,80}$")
    return bool(_g10_re.match(task_id)) and not task_id.endswith(RESERVED_ID_SUFFIXES)


def _pid_alive(pid):
    override = os.environ.get("CYS_GUARD_LIVENESS")
    if override == "alive":
        return True
    if override == "dead":
        return False
    try:
        os.kill(int(pid), 0)
        return True
    except (ProcessLookupError, ValueError, TypeError):
        return False
    except PermissionError:
        return True


# ── 부수 채널: emit·경보(전부 best-effort — 판정 불개입 · 조건 19①) ─────────
def _emit(evt_type, fields):
    """javis_event emit --spool — 실패 전부 무해 격리. fields = {key: str(value)}."""
    ev = os.path.join(_SELF_DIR, "javis_event.py")
    if not os.path.isfile(ev):
        return
    argv = [sys.executable, ev, "emit", evt_type, "--spool"]
    for k, v in fields.items():
        argv += ["--field", "%s=%s" % (k, v)]
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"
    with contextlib.suppress(Exception):
        subprocess.run(argv, capture_output=True, text=True, timeout=5, env=env)


def _alert_once(sid, kind, evt_type, fields):
    """세션 코얼레싱 경보 — `.guard-alert.<surface>.<kind>` 마커(TTL 6h) 내 재발화 억제."""
    mark = os.path.join(TASKS_DIR, ".guard-alert.%s.%s" % (sid or "nosid", kind))
    try:
        if time.time() - os.stat(mark).st_mtime < ALERT_TTL_SEC:
            return False
    except OSError:
        pass
    _emit(evt_type, fields)
    with contextlib.suppress(OSError):
        os.makedirs(TASKS_DIR, exist_ok=True)
        with open(mark, "w", encoding="utf-8") as f:
            f.write(_now() + "\n")
    return True


# ── E2-1(a): claim 생존 판정 — pid 는 보조 신호, 태스크 레코드 상태가 1차 ──────
def _claim_liveness(claim, claim_p, task_id):
    """(live: bool, why: str) — claim 이 아직 살아 있는가.

    ★BLOCKER R-02 교정. 종전 판정은 `pid 사망 = stale`이었고, 표준 플로우에서 그 pid 는
    **checkout 을 실행한 master 셸**의 것이었다. master 가 `/clear`·재기동하면 pid 가 죽고
    워커 guard 는 claim 을 stale 로 보고 **영구 exit 0** 한다 — 무장했는데 아무것도 검증하지
    않는 상태가 정상 동작과 구분되지 않는다(카나리아 1일차 통과·2일차 침묵).

    새 판정 순서(브리프 처방 (a)):
      1) claim mtime > 24h  → stale (종전 유지 — 절대 상한)
      2) 태스크 레코드가 **활성**(status ∈ in_progress|in_review) ∧ 레코드 신선(mtime ≤ 24h)
         → **claim 유효**. pid 생존 여부는 보조 신호로 강등한다(죽어 있으면 정보성 경보 1회).
      3) 그 밖 → pid 생존이면 유효, pid 부재/사망이면 stale.
    즉 "누구의 pid 인가"라는 답할 수 없는 질문 대신 "이 태스크가 아직 열려 있는가"라는
    **검증 가능한 사실**로 판정한다. 태스크가 done/cancelled 로 닫히면 checkout 측
    `_remove_guard_claims` 가 claim 을 소거하므로 이 경로는 고아 claim 만 남는다.
    """
    try:
        if time.time() - os.stat(claim_p).st_mtime > CLAIM_STALE_SEC:
            return False, "claim mtime 24h 초과"
    except OSError:
        return False, "claim stat 실패"
    pid = claim.get("pid")
    pid_live = _pid_alive(pid) if pid is not None else False
    task, terr = _read_json(os.path.join(TASKS_DIR, "%s.json" % task_id))
    status = (task or {}).get("status")
    fresh = False
    if task is not None:
        with contextlib.suppress(OSError):
            fresh = (time.time()
                     - os.stat(os.path.join(TASKS_DIR, "%s.json" % task_id)).st_mtime
                     ) <= CLAIM_STALE_SEC
    if status in ACTIVE_STATUSES and fresh:
        why = "태스크 활성(status=%s)" % status
        if not pid_live:
            why += " · pid(%r·%s) 사망/부재 — 보조 신호로 강등" % (
                pid, claim.get("pid_source") or "unknown")
        return True, why
    if pid_live:
        return True, "pid %s 생존(태스크 status=%s)" % (pid, status)
    if terr == "absent":
        return False, "pid %r 사망/부재 ∧ 태스크 레코드 부재" % pid
    return False, "pid %r 사망/부재 ∧ 태스크 비활성(status=%s·fresh=%s)" % (pid, status, fresh)


# ── E2-1(c): 침묵 탐지기 — "무장했는데 연속 N회 아무것도 검증하지 않음" ────────
#    침묵형 결함의 일반 처방이다. guard 뿐 아니라 어떤 게이트든 "무장 = 발동"이 아니고
#    "무장 ∧ 조건 충족 = 발동"인 이상, 조건이 조용히 영영 거짓이 되는 경로가 존재한다.
#    그 상태는 **정상 동작과 관측적으로 구분되지 않는다**(둘 다 exit 0·둘 다 무음).
#    여기서는 무검증 종결을 연속 계수하고 임계에서 1회 경보 + 상태 파일을 남긴다
#    (preflight C72 가 부트 시 그 파일을 WARN 으로 표면화한다 — 런타임·부트 2경로).
SILENCE_N_DEFAULT = 5     # 연속 무검증 종결 임계(카나리아 1세션 기준 보수측)


def _silence_n():
    raw = os.environ.get("CYS_GUARD_SILENCE_N", "").strip()
    try:
        n = int(raw) if raw else SILENCE_N_DEFAULT
    except ValueError:
        return SILENCE_N_DEFAULT
    return n if n >= 1 else SILENCE_N_DEFAULT


def _silence_path(sid):
    return os.path.join(TASKS_DIR, ".guard-silence.%s.json" % (sid or "nosid"))


def _silence_note(sid, reason, task_id=None):
    """무검증 종결 1회 계수 — 임계 도달 시 경보 1회(_alert_once 코얼레싱). best-effort."""
    path = _silence_path(sid)
    rec, _e = _read_json(path)
    if not isinstance(rec, dict) or rec.get("schema_version") != 1:
        rec = {"schema_version": 1, "streak": 0, "first_ts": _now(), "alerted_at": None}
    rec["streak"] = int(rec.get("streak", 0) or 0) + 1
    rec["last_ts"] = _now()
    rec["last_reason"] = reason
    rec["last_task"] = task_id
    rec["threshold"] = _silence_n()
    if rec["streak"] >= _silence_n():
        fired = _alert_once(
            sid, "guard-silent", "agent.error",
            {"agent": _role(),
             "summary": "[completion-guard][침묵 탐지] 무장(CYS_COMPLETION_GUARD=1)인데 "
                        "연속 %d회 아무것도 검증하지 않고 종결했다(최근 사유: %s · task=%s). "
                        "무장≠발동 — claim 소유/바인딩을 확인하라"
                        % (rec["streak"], reason, task_id or "-")})
        if fired:
            rec["alerted_at"] = _now()
    with contextlib.suppress(OSError):
        _write_json_atomic(path, rec)
    return rec["streak"]


def _silence_reset(sid):
    """실제 검증이 1회라도 돌면 침묵 스트릭을 끊는다(파일 삭제 = 무침묵 상태의 결정론 표현)."""
    with contextlib.suppress(OSError):
        os.remove(_silence_path(sid))


# ── guard.json(guard_state) — §2-4 스키마·손상 격리 재생성 ────────────────────
def _fresh_state(task_id):
    return {"schema_version": 1, "task_id": task_id,
            "block_count": 0, "infra_count": 0,
            "last_verdict": None, "last_sig": None, "escalated_at": None,
            "deferred": False, "context_yield": False, "timeout_violations": 0,
            "last_blocked_at": None, "disarm_reason": None}


def _load_guard_state(task_id):
    """(state, corrupted:bool) — 손상은 corrupt-<ts> rename 격리 + 신규 생성·카운터 0(조건 14)."""
    path = _guard_state_path(task_id)
    obj, err = _read_json(path)
    if err == "corrupt":
        with contextlib.suppress(OSError):
            os.rename(path, "%s.corrupt-%d" % (path, int(time.time())))
        fresh = _fresh_state(task_id)
        with contextlib.suppress(OSError):
            _write_json_atomic(path, fresh)
        return fresh, True
    if obj is None:
        return _fresh_state(task_id), False
    base = _fresh_state(task_id)
    base.update({k: obj[k] for k in obj if k in base})  # 전방 호환 — 미지 키는 버리고 스키마 핀
    return base, False


def _save_guard_state(state):
    with contextlib.suppress(OSError):
        _write_json_atomic(_guard_state_path(state["task_id"]), state)


# ── ctx% 선확인(§2-2 3단 — D5 statusline 체인 단일) ──────────────────────────
def _fetch_status_json():
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
    return d if isinstance(d, dict) else None


def _own_row(status, sid):
    """CYS_SURFACE_ID 로 자기 surface row 선택(role 아님 — R1-contract)."""
    for s in (status or {}).get("surfaces") or []:
        if not isinstance(s, dict) or s.get("exited") is True:
            continue
        rid = s.get("surface_id")
        if str(rid) == str(sid):
            return s
    return None


def _measure_ctx(sid):
    """(ctx_pct|None, reason) — measure() import 재사용(복제 금지 · §2-2). 실패 = (None, 사유)."""
    try:
        import javis_cycle_autopilot as _ap  # 형제 모듈 — 지역 import(부재=버전 찢김=측정 실패)
    except Exception as e:  # noqa: BLE001 — 측정 실패로 수렴(NO_BLOCK_PASS 모드)
        return None, "autopilot import 실패: %s" % e
    st = _fetch_status_json()
    if st is None:
        return None, "cys status --json 조회 실패(headless·타임아웃·데몬 부재)"
    row = _own_row(st, sid)
    if row is None:
        return None, "자기 surface row 부재(sid=%s)" % sid
    try:
        m = _ap.measure(row, _role(), time.time())
    except Exception as e:  # noqa: BLE001
        return None, "measure() 예외: %s" % e
    if not m.get("ok"):
        return None, "measure 무효: %s" % m.get("reason")
    pct = m.get("ctx_pct")
    if not isinstance(pct, (int, float)):
        return None, "ctx_pct 비수치"
    return float(pct), ""


# ── resource_gate(§2-2 4단 — 조건 10·15·35·D20) ─────────────────────────────
def _soft_kind(ctx_pct, warnings):
    """F4: gate exit 1 판별 정밀화 — trips 공집합이 아니라 `--json` warnings 판독.
    warnings 가 정확히 ['context_unmeasured'] **단독**(ctx 미주입 소프트 승격뿐)일 때만
    proceed_unmeasured. `measure_error:*` 포함(ps/load 실측 실패) 등 그 외 전부
    skip_soft(SKIPPED_RESOURCE) — 자원 측정 실패의 조용한 통과 차단(P-ORCH-1 정합)."""
    if ctx_pct is None and list(warnings or []) == ["context_unmeasured"]:
        return "proceed_unmeasured"
    return "skip_soft"


def _pick_trip(trips, level):
    """F7: gkind(level)와 일치하는 첫 트립 선택 — skip_hard 페이로드에 soft 트립이 실리는
    자기모순 이벤트 차단. 일치 항목 부재 시 None(제네릭 agent.error 경보로 강등)."""
    for t in trips:
        if isinstance(t, dict) and t.get("level") == level:
            return t
    return None


def _resource_gate(ctx_pct):
    """반환 (kind, detail, trip) · kind ∈ ok|skip_soft|skip_hard|proceed_unmeasured|infra."""
    gate = os.path.join(_SELF_DIR, "javis_resource_gate.py")
    if not os.path.isfile(gate):
        return "infra", "javis_resource_gate 부재(버전 찢김)", None
    argv = [sys.executable, gate, "check", "--require-context", "--json"]
    extra = os.environ.get("CYS_GUARD_RESOURCE_ARGS", "").split()
    argv += extra
    if ctx_pct is not None:
        argv += ["--context", str(ctx_pct)]
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=10, env=env)
    except (subprocess.SubprocessError, OSError) as e:
        return "infra", "resource_gate 실행 실패: %s" % e, None
    rc = r.returncode
    if rc in (64, 70):
        return "infra", "resource_gate 측정 실패 exit %d(EX_USAGE/EX_SOFTWARE — D20 loud)" % rc, None
    doc = {}
    with contextlib.suppress(ValueError, TypeError):
        doc = json.loads(r.stdout) or {}
    if not isinstance(doc, dict):
        doc = {}
    trips = doc.get("trips") or []
    warnings = doc.get("warnings") or []
    if rc == 0:
        return "ok", "", None
    if rc == 1:
        # F4: 판별은 warnings 판독(--json) — context_unmeasured 단독일 때만 no-block 진행.
        #     JSON 파싱 실패 시 warnings=[] → skip_soft(보수측).
        kind = _soft_kind(ctx_pct, warnings)
        if kind == "proceed_unmeasured":
            return "proceed_unmeasured", "context_unmeasured 단독 soft — no-block 진행", None
        return "skip_soft", "resource soft(exit 1 · warnings=%s)" % ",".join(
            str(w) for w in warnings), _pick_trip(trips, "soft")
    if rc == 2:
        return "skip_hard", "resource hard(exit 2)", _pick_trip(trips, "hard")
    return "infra", "resource_gate 미지 exit %d" % rc, None


# ── verifier tax 샤드(§2-2 5단 — B2 §3 실구현·부재=무제한) ───────────────────
def _budget_window():
    return int(time.time() // BUDGET_WINDOW_SEC) * BUDGET_WINDOW_SEC


def _budget_limits():
    """유효 예산 한도 (max_runs, max_secs) — 우선순위: 기본(60회/300초) < policy.json(플릿
    배포값) < env JAVIS_VERIFY_BUDGET_RUNS/SECS(pane 실험 오버라이드 — 최우선·카나리아용).
    ★조정 명문(B2 §3 — 근거 자인): 초기값 총 60회/300초·고정창 1h 는 **카나리아 실측 전
    보수 추정**이다(실측 근거 없음). OT-2 카나리아에서 창당 실 verify 소비를 실측한 뒤
    policy.json(플릿)·env(개별 pane) 로 조정하라 — 코드 상수 재조정은 측정 근거 병기 없이
    금지. 비수치 값은 해당 층만 무시(폴백 유지)."""
    # G7b: 한도 ≤0 은 해당 층 무시 폴백 — '비수치 무시'와 동형(0/음수 한도는 상시 소진 =
    # 전 함대 verify 정지 footgun — 오타·오배포 방어).
    max_runs, max_secs = BUDGET_MAX_RUNS_DEFAULT, BUDGET_MAX_SECS_DEFAULT
    pol, _e = _read_json(os.path.join(BUDGET_DIR, "policy.json"))
    if pol:
        with contextlib.suppress(TypeError, ValueError):
            v = int(pol.get("max_runs", max_runs))
            if v > 0:
                max_runs = v
        with contextlib.suppress(TypeError, ValueError):
            v = float(pol.get("max_secs", max_secs))
            if v > 0:
                max_secs = v
    raw = os.environ.get("JAVIS_VERIFY_BUDGET_RUNS", "").strip()
    if raw:
        with contextlib.suppress(ValueError):
            v = int(raw)
            if v > 0:
                max_runs = v
    raw = os.environ.get("JAVIS_VERIFY_BUDGET_SECS", "").strip()
    if raw:
        with contextlib.suppress(ValueError):
            v = float(raw)
            if v > 0:
                max_secs = v
    return max_runs, max_secs


def _budget_check():
    """(exhausted:bool, detail, stats:dict|None). 디렉터리 부재 = (False,'absent',None) —
    배포 전 호환. 판독 = 전 샤드 합산(자기 샤드만 기록·판독은 전체 — 총량 검사·R1)."""
    if not os.path.isdir(BUDGET_DIR):
        return False, "absent", None
    max_runs, max_secs = _budget_limits()
    win = _budget_window()
    runs, secs = 0, 0.0
    try:
        names = os.listdir(BUDGET_DIR)
    except OSError:
        return False, "unreadable", None
    for name in names:
        if not name.endswith(".json") or name == "policy.json":
            continue
        shard, _e = _read_json(os.path.join(BUDGET_DIR, name))
        if shard and shard.get("window_start") == win:
            with contextlib.suppress(TypeError, ValueError):
                runs += int(shard.get("runs", 0))
            with contextlib.suppress(TypeError, ValueError):
                secs += float(shard.get("secs", 0.0))
    stats = {"runs": runs, "secs": secs, "max_runs": max_runs, "max_secs": max_secs,
             "window": win}
    if runs >= max_runs or secs >= max_secs:
        return True, "runs=%d/%d secs=%.1f/%.1f(1h 고정창)" % (runs, max_runs, secs,
                                                               max_secs), stats
    return False, "runs=%d/%d secs=%.1f/%.1f" % (runs, max_runs, secs, max_secs), stats


def _budget_hard_emit_once(stats):
    """B2(조건 04②): 소진 = `resource.hard`(metric=verify_budget) **플릿 단일 발행** —
    워커별 발화 금지. 멱등 마커 `.hard-emitted-<창키>` 를 O_EXCL 원자 생성(락·대기 없음 ·
    조건 25①)한 마커 선점 승자만 emit — 반복 소진·타 워커 동시 도달은 마커 존재로 무발화,
    창이 바뀌면 새 마커. ★G7c 보증 수위 정정: 이 계약은 창당 **at-most-once** 다 — 마커
    생성 '후' emit '전' 승자 사망 시 그 창은 무발화(수용: 이벤트는 best-effort 부수 채널 ·
    조건 19① — 판정(SKIPPED_BUDGET·deferred)은 마커와 무관하게 각 워커에서 성립).
    value/threshold 는 소진 축(runs 우선) 기준. 지난 창 마커(24h+)는 best-effort 청소
    (비블로킹·실패 무해)."""
    if not stats:
        return False
    win = stats.get("window")
    marker = os.path.join(BUDGET_DIR, ".hard-emitted-%s" % win)
    try:
        fd = os.open(marker, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    except OSError:
        return False  # 이미 발행됨(창당 1회) 또는 디렉터리 소실 — 무발화
    with contextlib.suppress(OSError):
        os.write(fd, (_now() + "\n").encode("utf-8"))
    with contextlib.suppress(OSError):
        os.close(fd)
    if stats.get("runs", 0) >= stats.get("max_runs", 0):
        value, threshold = stats.get("runs", 0), stats.get("max_runs", 0)
    else:
        value, threshold = int(stats.get("secs", 0)), int(stats.get("max_secs", 0))
    _emit("resource.hard", {"metric": "verify_budget", "value": str(value),
                            "threshold": str(threshold)})
    with contextlib.suppress(OSError):
        for name in os.listdir(BUDGET_DIR):
            if name.startswith(".hard-emitted-") and name != os.path.basename(marker):
                p = os.path.join(BUDGET_DIR, name)
                with contextlib.suppress(OSError):
                    if time.time() - os.stat(p).st_mtime > 24 * 3600:
                        os.remove(p)
    return True


def _budget_record(sid, secs):
    """자기 샤드만 기록(lost-update 없음 — surface 당 Stop 은 직렬). 디렉터리 부재 시 무동작."""
    if not os.path.isdir(BUDGET_DIR):
        return
    path = os.path.join(BUDGET_DIR, "%s.json" % (sid or "nosid"))
    win = _budget_window()
    shard, _e = _read_json(path)
    if not shard or shard.get("window_start") != win:
        shard = {"schema_version": 1, "window_start": win, "runs": 0, "secs": 0.0}
    with contextlib.suppress(TypeError, ValueError):
        shard["runs"] = int(shard.get("runs", 0)) + 1
        shard["secs"] = float(shard.get("secs", 0.0)) + float(secs)
    with contextlib.suppress(OSError):
        _write_json_atomic(path, shard)


def budget_state():
    """(active: bool, detail: dict) — verifier tax 활성 상태의 단일 판독 진실.

    ★E2-2(BLOCKER R-07): tax 는 `_round/verify-budget/` **디렉터리 부재 = 무제한 통과**
    (기본 OFF)인데, 종전에는 ①그 디렉터리를 만드는 생성자가 팩 전체에 0건이었고 ②활성화
    절차가 어디에도 없었으며 ③OT-2 §2.8 은 "영원히 생기지 않는 파일"을 관찰하라고 지시했다
    ⇒ 운영자는 "압력 없음"이라는 **거짓 확신**을 얻는다. 이 함수 + `budget-init` 서브커맨드
    + preflight C72 의 상태 1줄이 그 침묵을 깬다(활성/비활성을 **말하게** 만든다).
    """
    active = os.path.isdir(BUDGET_DIR)
    max_runs, max_secs = _budget_limits()
    d = {"budget_dir": BUDGET_DIR, "active": active,
         "policy_present": os.path.isfile(os.path.join(BUDGET_DIR, "policy.json")),
         "max_runs": max_runs, "max_secs": max_secs,
         "env_override": {
             "JAVIS_VERIFY_BUDGET_RUNS": os.environ.get("JAVIS_VERIFY_BUDGET_RUNS") or None,
             "JAVIS_VERIFY_BUDGET_SECS": os.environ.get("JAVIS_VERIFY_BUDGET_SECS") or None}}
    if active:
        exhausted, detail, stats = _budget_check()
        d.update({"exhausted": exhausted, "usage": detail,
                  "window": (stats or {}).get("window"),
                  "shards": len([n for n in os.listdir(BUDGET_DIR)
                                 if n.endswith(".json") and n != "policy.json"])
                  if os.path.isdir(BUDGET_DIR) else 0})
    return active, d


def cmd_budget(argv):
    """`budget-init` / `budget-status` — verifier tax 활성화·되돌리기·상태 조회(E2-2).

    되돌리기는 **디렉터리 삭제 1개**다: `rm -rf $JAVIS_ROOT/_round/verify-budget`
    (부재 = 무제한 통과로 즉시 복귀 — 코드 변경·재배포 불필요). 활성/비활성 전환은 어느
    방향이든 guard 판정을 **차단 방향으로 바꾸지 않는다**(소진 = SKIPPED_BUDGET = exit 0).
    """
    sub = argv[0]
    args = argv[1:]
    as_json = "--json" in args
    if sub == "budget-status":
        active, d = budget_state()
        if as_json:
            print(json.dumps(d, ensure_ascii=False, indent=1))
        else:
            print("verifier tax: %s" % ("활성(ON)" if active else "비활성(OFF — 무제한 통과)"))
            print("  디렉터리: %s (%s)" % (BUDGET_DIR, "존재" if active else "부재"))
            print("  policy.json: %s" % ("있음" if d["policy_present"] else "없음(기본값 사용)"))
            print("  유효 한도: %d회 / %.0f초 (1h 고정창)" % (d["max_runs"], d["max_secs"]))
            if active:
                print("  현재 창 소비: %s · 샤드 %d개 · 소진=%s"
                      % (d.get("usage"), d.get("shards", 0), d.get("exhausted")))
            else:
                print("  ⚠ 비활성 상태에서는 `_round/verify-budget/*.json` 샤드가 "
                      "**영원히 생기지 않는다** — 샤드 관찰 지시는 활성화 이후에만 유효하다.")
        return 0
    # ── budget-init ──
    force = "--force" in args
    max_runs, max_secs = BUDGET_MAX_RUNS_DEFAULT, BUDGET_MAX_SECS_DEFAULT
    for i, tok in enumerate(args):
        if tok == "--max-runs" and i + 1 < len(args):
            with contextlib.suppress(ValueError):
                max_runs = int(args[i + 1])
        if tok == "--max-secs" and i + 1 < len(args):
            with contextlib.suppress(ValueError):
                max_secs = float(args[i + 1])
    if max_runs <= 0 or max_secs <= 0:
        print("budget-init 거부: 한도는 양수여야 한다(0/음수 = 상시 소진 footgun · G7b)",
              file=sys.stderr)
        return 2
    pol_p = os.path.join(BUDGET_DIR, "policy.json")
    if os.path.isfile(pol_p) and not force:
        cur, _e = _read_json(pol_p)
        print("이미 활성: %s (기존 policy=%r) — 덮어쓰려면 --force" % (BUDGET_DIR, cur))
        return 0
    try:
        os.makedirs(BUDGET_DIR, exist_ok=True)
        _write_json_atomic(pol_p, {
            "schema_version": 1, "max_runs": max_runs, "max_secs": max_secs,
            "window_sec": BUDGET_WINDOW_SEC, "created_at": _now(),
            "note": "verifier tax 정책(B2 §3). 초기값은 카나리아 실측 전 보수 추정이다 — "
                    "1시간 관찰 후 샤드 실측치로 조정하라. 되돌리기 = 이 디렉터리 삭제."})
    except OSError as e:
        print("budget-init 실패: %s" % e, file=sys.stderr)
        return 1
    print("verifier tax 활성화 완료")
    print("  1) 디렉터리: %s" % BUDGET_DIR)
    print("  2) policy.json: max_runs=%d · max_secs=%.0f · window_sec=%d"
          % (max_runs, max_secs, BUDGET_WINDOW_SEC))
    print("  3) 1시간 뒤 실측: python3 %s budget-status  (샤드 소비량 확인 후 한도 조정)"
          % os.path.basename(__file__))
    print("  되돌리기: rm -rf %s  (부재 = 무제한 통과 · 코드 변경 불필요)" % BUDGET_DIR)
    return 0


def _maybe_demote_calibrated(task_id, state, spec):
    """B2(조건 07②): timeout_violations ≥2 → verify_spec.calibrated=false 자동 강등 전이.
    집행은 javis_task `set-verify-spec <id> --demote-calibrated` 위임 — 레코드 갱신이
    wlock(5번째 mutator) 직렬화를 그대로 타고 멱등이다(guard 의 직접 갱신 금지 — 락 계약
    단일화). calibrated 가 이미 false/부재면 스폰 자체를 생략(반복 Stop 무의미 스폰 차단).
    best-effort: 실패는 판정 불개입(조건 19① OBSERVABILITY 상속).
    ★G5: 호출처는 **정상 Stop(_hook_main) 한정** — 데드라인 경로(_on_deadline)는 스폰을
    생략하고 카운트만 적재한다(다음 정상 Stop 이 여기서 소비 — 3중 상한 여유 보존)."""
    if int(state.get("timeout_violations", 0)) < 2:
        return
    if spec is None:
        task, _e = _read_json(os.path.join(TASKS_DIR, "%s.json" % task_id))
        spec = (task or {}).get("verify_spec") if isinstance(task, dict) else None
    if not isinstance(spec, dict) or spec.get("calibrated") is not True:
        return
    jt = os.path.join(_SELF_DIR, "javis_task.py")
    if not os.path.isfile(jt):
        return
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"
    with contextlib.suppress(Exception):
        subprocess.run([sys.executable, jt, "set-verify-spec", task_id,
                        "--demote-calibrated"],
                       capture_output=True, text=True, timeout=10, env=env)


def _grill_active():
    """G6(성찰 2): grill 마커 활성(collecting·비만료) 판정 — grill_gate 형제 모듈 재사용
    (C75 import 전례·복제 금지). 실패 전부 비활성 취급(best-effort — 판정 불개입)."""
    try:
        import grill_gate as _gg
        m = _gg._load_marker()
        return bool(m and m.get("status") == "collecting" and not _gg._expired(m))
    except Exception:  # noqa: BLE001 — 마커/모듈 어떤 이상도 경보 억제측(무해)
        return False


def _warn_armed_combo(sid):
    """G6(성찰 2 MEDIUM): 무장 조합 위험 경보 — 무장 첫 발동 시 1회(_alert_once 코얼레싱).
    조합: CLAUDE_CODE_STOP_HOOK_BLOCK_CAP 미설정(하네스 Stop 지갑=기본 cap 8+1=9) ∧ grill
    마커 활성 ∧ GRILL_STOP_SELF_CAP=0(self-cap off) — grill 블록 K 가 지갑을 선점하면
    9−K < N(escalation 임계 3) 이 되어 guard 가 escalation 도달 전 지갑 소진(무검증
    오버라이드 창 — T0 §2 실험 4 산술). 처방 = cap 상향+self-cap 활성 **원자 조합**(OT-2)."""
    if os.environ.get("CLAUDE_CODE_STOP_HOOK_BLOCK_CAP", "").strip():
        return
    if os.environ.get("GRILL_STOP_SELF_CAP", "0").strip() not in ("", "0"):
        return
    if not _grill_active():
        return
    _alert_once(sid, "armed-combo-risk", "agent.error",
                {"agent": _role(),
                 "summary": "[completion-guard] 무장 조합 위험 — 선점 산술 위험(9−K<N): "
                            "CLAUDE_CODE_STOP_HOOK_BLOCK_CAP 미설정 ∧ grill 마커 활성 ∧ "
                            "GRILL_STOP_SELF_CAP=0 — grill 블록이 하네스 Stop 지갑(9)을 "
                            "선점하면 guard escalation(N=3) 도달 전 소진. cap 상향+self-cap "
                            "활성 원자 조합 적용 요망(OT-2)"})


# ── verify 실행(setsid 그룹 + killpg — §2-2·조건 07③·23) ────────────────────
def _shell_argv(cmd):
    """E1-3(R-09): mode=command 의 셸 실행 argv — 플랫폼 분기.

    종전: `["/bin/sh", "-c", cmd]` 하드코딩. Windows 에는 `/bin/sh` 가 없어 **모든** command
    태스크가 `FileNotFoundError` → `INFRA_ERROR` 로 수렴하고, 3연속이면 태스크마다
    `guard-disarmed` + critical wakeup 이 터진다(결함군 ① 폭주와 교차). 이제 `COMSPEC`
    (기본 `cmd.exe`)로 분기한다 — 무장 자체가 기본 거부이므로 이 경로는 명시 opt-in 시에만
    돈다(아래 `_windows_arming_refusal`).
    """
    if os.name == "nt":
        return [os.environ.get("COMSPEC") or "cmd.exe", "/c", cmd]
    return ["/bin/sh", "-c", cmd]


def _popen_group(argv, cwd, timeout):
    """프로세스 그룹 실행 → (rc, out_text, timed_out, survivors). 종료 시 그룹 정리.

    POSIX = setsid 그룹 + killpg / Windows = CREATE_NEW_PROCESS_GROUP + taskkill /T(E1-3).
    """
    kw = {}
    if os.name == "posix":
        kw["start_new_session"] = True  # setsid — 그룹 리더로 스폰
    else:
        # E1-3: 새 프로세스 그룹으로 스폰 — 부모(훅)의 Ctrl 이벤트가 verify 로 전파되지 않고,
        #       종료는 taskkill /T 가 트리를 따라간다.
        kw["creationflags"] = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
    p = subprocess.Popen(argv, cwd=cwd or None, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, **kw)
    # F1: 활성 그룹 등록 — _Deadline·최상위 예외로 이 함수가 중도 이탈하면 등록이 남고,
    #     핸들러(_kill_active_groups)가 정리한다. finally 해제는 금지 — 예외 unwinding 중
    #     해제되면 핸들러가 죽일 대상을 잃는다(고아 재발).
    #     ★E1-3: 종전엔 posix 에서만 등록해 Windows 는 데드라인 경로의 정리 대상이 **아예
    #     없었다**(고아 확정). 이제 전 플랫폼 등록하고 종료 수단만 분기한다.
    _ACTIVE_PGIDS.add(p.pid)
    timed_out = False
    try:
        out, _ = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        if os.name == "posix":
            with contextlib.suppress(OSError):
                os.killpg(p.pid, signal.SIGKILL)
        else:
            _win_taskkill(p.pid)   # E1-3: 트리 종료(자식만 kill 하면 손자가 파이프를 붙잡는다)
            with contextlib.suppress(Exception):
                p.kill()
        with contextlib.suppress(Exception):
            out, _ = p.communicate(timeout=5)
        out = out or b""
    survivors = False
    if os.name == "posix":
        # 잔존 감지(손자 프로세스) → 정리. start_new_session 이라 pgid == p.pid.
        try:
            os.killpg(p.pid, 0)
            survivors = True
        except OSError:
            survivors = False
        if survivors:
            with contextlib.suppress(OSError):
                os.killpg(p.pid, signal.SIGTERM)
            time.sleep(0.2)
            with contextlib.suppress(OSError):
                os.killpg(p.pid, signal.SIGKILL)
    _ACTIVE_PGIDS.discard(p.pid)  # 정상 경로 정리 완료 — 핸들러 killpg 대상에서 해제
    text = (out or b"").decode("utf-8", errors="replace")
    return p.returncode, text, timed_out, survivors


def _classify_rc(rc, out, pass_rule, timed_out):
    """pass_rule 판정 → PASS | VERIFY_FAIL | INFRA_ERROR (조건 14 — infra 오판 금지)."""
    if timed_out:
        return "INFRA_ERROR", "타임아웃(INFRA — 블록 금지·위반 카운트 적재)"
    pr = pass_rule if isinstance(pass_rule, dict) else {}
    kind = pr.get("kind")
    if kind == "exit_map":
        if rc in (pr.get("pass_exits") or []):
            return "PASS", ""
        if rc in (pr.get("fail_exits") or []):
            return "VERIFY_FAIL", "exit %d ∈ fail_exits" % rc
        if rc in (pr.get("indet_exits") or []):
            return "INFRA_ERROR", "exit %d ∈ indet_exits(INDET→INFRA 수렴)" % rc
        if rc in INFRA_EXITS or rc < 0:
            return "INFRA_ERROR", "exit %d(spawn/timeout 계열 — 조건 14)" % rc
        return "VERIFY_FAIL", "exit %d ∉ pass_exits(불충족)" % rc
    if rc in INFRA_EXITS or rc < 0:
        return "INFRA_ERROR", "exit %d(spawn/timeout 계열 — 조건 14)" % rc
    if kind == "output_contains":
        needle = pr.get("needle") or ""
        if needle and needle in out:
            return "PASS", ""
        return "VERIFY_FAIL", "needle %r 부재(불충족)" % needle[:80]
    if kind == "json_check":
        expr = (pr.get("expr") or "").strip()
        try:
            doc = json.loads(out.strip() or "null")
        except ValueError:
            return "VERIFY_FAIL", "출력이 JSON 아님(json_check 불충족)"
        if "==" in expr:
            path, want_raw = expr.split("==", 1)
            try:
                want = json.loads(want_raw.strip())
            except ValueError:
                want = want_raw.strip()
            got = _json_path(doc, path.strip())
            if got == (want,):  # 존재+일치를 튜플로 구분(값 None 정당 비교)
                return "PASS", ""
            return "VERIFY_FAIL", "expr %s 불일치(got=%r)" % (expr[:80], got)
        got = _json_path(doc, expr)
        if got is not None and got != ():
            return "PASS", ""
        return "VERIFY_FAIL", "expr 경로 %r 부재(불충족)" % expr[:80]
    return "INFRA_ERROR", "pass_rule.kind 미지(%r) — spec 파싱 실패 취급" % (kind,)


def _json_path(doc, dotted):
    """점 표기 경로 → (value,) | () (부재). 판정용 존재/일치 프리미티브."""
    cur = doc
    for seg in [s for s in dotted.split(".") if s]:
        if isinstance(cur, dict) and seg in cur:
            cur = cur[seg]
        else:
            return ()
    return (cur,)


def _run_verify(spec, task_id, sid):
    """verify_spec 실행(mode dispatch + n_of_m) → dict(verdict·rc·out·detail·elapsed·
    timed_out_any·survivors_any). 판정 3치 수렴: PASS/VERIFY_FAIL/INFRA_ERROR."""
    mode = spec.get("mode")
    try:
        timeout_s = float(spec.get("timeout_s") or VERIFY_TIMEOUT_DEFAULT)
    except (TypeError, ValueError):
        timeout_s = VERIFY_TIMEOUT_DEFAULT
    timeout_s = min(max(timeout_s, 0.1), VERIFY_TIMEOUT_MAX)

    if mode == "waiver":
        # 유효성은 checkout 게이트(T1) 소관 — guard 시점은 검증 면제 = PASS(실행 0).
        return {"verdict": "PASS", "rc": 0, "out": "", "detail": "waiver 면제(실행 0)",
                "elapsed": 0.0, "timed_out_any": False, "survivors_any": False}

    if mode == "procedural":
        return _run_procedural(spec, timeout_s, sid)

    nm = spec.get("n_of_m") if isinstance(spec.get("n_of_m"), dict) else {}
    try:
        need_n = max(1, int(nm.get("n", 1)))
        total_m = max(need_n, min(int(nm.get("m", 1)), N_OF_M_CAP))
    except (TypeError, ValueError):
        need_n, total_m = 1, 1

    if mode == "probe":
        pb = spec.get("probe") if isinstance(spec.get("probe"), dict) else {}
        name = pb.get("name")
        if not name:
            return {"verdict": "INFRA_ERROR", "rc": None, "out": "",
                    "detail": "probe.name 부재(spec 파싱 실패 취급)", "elapsed": 0.0,
                    "timed_out_any": False, "survivors_any": False}
        argv = [sys.executable, os.path.join(_SELF_DIR, "javis_actprobe.py"), str(name)]
        argv += [str(x) for x in (pb.get("args") or [])]
        if task_id:
            argv += ["--task", task_id]
        runner = ("argv", argv, None)
    elif mode == "command":
        cmd = spec.get("cmd")
        if not isinstance(cmd, str) or not cmd.strip():
            return {"verdict": "INFRA_ERROR", "rc": None, "out": "",
                    "detail": "cmd 부재(spec 파싱 실패 취급)", "elapsed": 0.0,
                    "timed_out_any": False, "survivors_any": False}
        runner = ("sh", _shell_argv(cmd), spec.get("cwd"))   # E1-3(R-09): 플랫폼 분기
    else:
        return {"verdict": "INFRA_ERROR", "rc": None, "out": "",
                "detail": "mode 미지(%r) — spec 파싱 실패 취급" % (mode,), "elapsed": 0.0,
                "timed_out_any": False, "survivors_any": False}

    passes = 0
    last = {"verdict": "VERIFY_FAIL", "rc": None, "out": "", "detail": "무실행"}
    elapsed_total = 0.0
    timed_out_any = survivors_any = False
    deadline = _self_deadline()
    for attempt in range(total_m):
        # F1: attempt 예산 검사 — (경과+timeout_s) > 데드라인−여유(5s)면 잔여 attempt 를
        #     조기 INFRA 종결(n_of_m 누적이 자체 데드라인을 초과할 수 없음 · SIGALRM 은
        #     verify 밖 구간의 최후 방어로 남는다).
        elapsed_run = time.time() - _RUN_T0
        if elapsed_run + timeout_s > deadline - DEADLINE_MARGIN_SEC:
            last = {"verdict": "INFRA_ERROR", "rc": None, "out": last.get("out", ""),
                    "detail": "데드라인 예산 부족(경과 %.1fs + timeout %.1fs > %ds−%ds) — "
                              "잔여 attempt 조기 INFRA 종결(attempt %d/%d)"
                              % (elapsed_run, timeout_s, deadline,
                                 int(DEADLINE_MARGIN_SEC), attempt + 1, total_m)}
            break
        t0 = time.time()
        try:
            rc, out, timed_out, survivors = _popen_group(runner[1], runner[2], timeout_s)
        except OSError as e:
            return {"verdict": "INFRA_ERROR", "rc": None, "out": "",
                    "detail": "spawn 예외: %s" % e, "elapsed": elapsed_total,
                    "timed_out_any": timed_out_any, "survivors_any": survivors_any}
        dt = time.time() - t0
        elapsed_total += dt
        _budget_record(sid, dt)  # 재시도 비용도 tax 계상(조건 20③)
        timed_out_any = timed_out_any or timed_out
        survivors_any = survivors_any or survivors
        if mode == "probe":
            if timed_out:
                verdict, detail = "INFRA_ERROR", "probe 타임아웃"
            elif rc == 0:
                verdict, detail = "PASS", ""
            elif rc == 2:
                verdict, detail = "VERIFY_FAIL", "probe FAIL(exit 2)"
            else:
                verdict, detail = "INFRA_ERROR", "probe INDET/오류(exit %s)" % rc
        else:
            verdict, detail = _classify_rc(rc, out, spec.get("pass_rule"), timed_out)
        last = {"verdict": verdict, "rc": rc, "out": out, "detail": detail}
        if verdict == "INFRA_ERROR":
            break
        if verdict == "PASS":
            passes += 1
            if passes >= need_n:
                break
        remaining = total_m - attempt - 1
        if passes + remaining < need_n:
            break  # 도달 불가 — 조기 종결
    if last["verdict"] != "INFRA_ERROR":
        last["verdict"] = "PASS" if passes >= need_n else "VERIFY_FAIL"
        if last["verdict"] == "VERIFY_FAIL" and total_m > 1:
            last["detail"] = "n_of_m %d/%d 미달 · %s" % (passes, total_m, last["detail"])
    last.update({"elapsed": elapsed_total, "timed_out_any": timed_out_any,
                 "survivors_any": survivors_any})
    return last


def _run_procedural(spec, timeout_s, sid):
    """procedural mode — 실도구 재사용(§1-1 R1-contract): factcheck=evaluate(text) import ·
    check-criteria=javis_manifest CLI(합성 매니페스트). 입력 오류는 전부 INFRA(블록 금지)."""
    pc = spec.get("procedural") if isinstance(spec.get("procedural"), dict) else {}
    tool = pc.get("tool")
    base = {"rc": None, "out": "", "elapsed": 0.0, "timed_out_any": False,
            "survivors_any": False}
    if tool == "factcheck":
        path = pc.get("report_path")
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                text = f.read()
        except (OSError, TypeError) as e:
            return dict(base, verdict="INFRA_ERROR", detail="report 읽기 실패: %s" % e)
        try:
            import javis_factcheck_gate as _fc  # evaluate(text) 순수 함수 재사용(§1-1)
            code, detail = _fc.evaluate(text)
        except Exception as e:  # noqa: BLE001
            return dict(base, verdict="INFRA_ERROR", detail="factcheck evaluate 실패: %s" % e)
        if code == 0:
            return dict(base, verdict="PASS", rc=0, detail="factcheck 통과")
        if code == 2:
            return dict(base, verdict="VERIFY_FAIL", rc=2,
                        detail="factcheck 잔여 미통과: %s" % (detail,))
        return dict(base, verdict="INFRA_ERROR", rc=code,
                    detail="factcheck 불완결(exit %s — INFRA)" % code)
    if tool == "check-criteria":
        import tempfile
        manifest = {"name": "guard-procedural",
                    "phases": [{"id": str(pc.get("phase") or "verify"), "skill": "verify",
                                "success_criteria": {"statement": "guard procedural verify",
                                                     "checks": pc.get("criteria") or []}}]}
        fd, mpath = tempfile.mkstemp(prefix="guard-manifest-", suffix=".json")
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                json.dump(manifest, f, ensure_ascii=False)
            argv = [sys.executable, os.path.join(_SELF_DIR, "javis_manifest.py"),
                    "check-criteria", mpath, "--phase", str(pc.get("phase") or "verify"),
                    "--artifact", str(pc.get("artifact_path") or "")]
            t0 = time.time()
            rc, out, timed_out, survivors = _popen_group(argv, None, timeout_s)
            dt = time.time() - t0
            _budget_record(sid, dt)
            if timed_out:
                return dict(base, verdict="INFRA_ERROR", rc=rc, out=out, elapsed=dt,
                            timed_out_any=True, survivors_any=survivors,
                            detail="check-criteria 타임아웃")
            if rc == 0:
                return dict(base, verdict="PASS", rc=0, out=out, elapsed=dt,
                            survivors_any=survivors, detail="check-criteria 수렴")
            if rc == 1:
                return dict(base, verdict="VERIFY_FAIL", rc=1, out=out, elapsed=dt,
                            survivors_any=survivors, detail="check-criteria 기준 미달")
            return dict(base, verdict="INFRA_ERROR", rc=rc, out=out, elapsed=dt,
                        survivors_any=survivors,
                        detail="check-criteria 입력 오류(exit %s — INFRA)" % rc)
        finally:
            with contextlib.suppress(OSError):
                os.remove(mpath)
    return dict(base, verdict="INFRA_ERROR",
                detail="procedural.tool 미지(%r) — spec 파싱 실패 취급" % (tool,))


# ── failure_sig(§2-3 R1 — 타임스탬프·16진·절대경로 마스킹 정규화 해시) ────────
_SIG_TS_RE = re.compile(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}[0-9.+:Z-]*|\b\d{2}:\d{2}:\d{2}\b")
_SIG_HEX_RE = re.compile(r"0x[0-9a-fA-F]+|\b[0-9a-f]{8,}\b")
_SIG_PATH_RE = re.compile(r"(?<![\w])/[\w./-]{4,}")


def _failure_sig(cmd_repr, rc, out):
    head = (out or "")[:512]
    head = _SIG_TS_RE.sub("<TS>", head)
    head = _SIG_HEX_RE.sub("<HEX>", head)
    head = _SIG_PATH_RE.sub("<PATH>", head)
    raw = "%s\x00%s\x00%s" % (cmd_repr, rc, head)
    return hashlib.sha256(raw.encode("utf-8", errors="replace")).hexdigest()[:16]


def esc_request_id(task_id, sig):
    """escalation·esc-bundle 멱등키 정규화 — **단일 정의**(Wave C 승인 큐가 import 재사용).

    sig 부재(측정 실패·백필)는 `nosig` 로 접는다. 이 규칙이 갈라지면 guard 가 쏜 wakeup
    멱등키와 큐가 만든 항목 request_id 가 서로 다른 문자열이 되어 **큐↔wakeup 조인이
    끊긴다**(설계 §4 R1: request_id = 조인 키).
    """
    s = str(sig).strip() if sig else ""
    return "guard:%s:%s" % (task_id, s or "nosig")


def _spec_cmd_repr(spec):
    mode = spec.get("mode")
    if mode == "command":
        return str(spec.get("cmd") or "")
    if mode == "probe":
        pb = spec.get("probe") or {}
        return "probe:%s %s" % (pb.get("name"), " ".join(pb.get("args") or []))
    if mode == "procedural":
        pc = spec.get("procedural") or {}
        return "procedural:%s" % pc.get("tool")
    return str(mode)


def _cap_output(out):
    """head 1KB + tail 3KB 결정론 절단(조건 08) — 절단 시 바이트 수 표기."""
    b = (out or "").encode("utf-8", errors="replace")
    if len(b) <= HEAD_CAP + TAIL_CAP:
        return out or ""
    head = b[:HEAD_CAP].decode("utf-8", errors="replace")
    tail = b[-TAIL_CAP:].decode("utf-8", errors="replace")
    return "%s\n…[%d바이트 절단]…\n%s" % (head, len(b) - HEAD_CAP - TAIL_CAP, tail)


def _write_verify_log(task_id, spec, rc, out, verdict):
    path = _verify_log_path(task_id)
    body = out or ""
    if len(body.encode("utf-8", errors="replace")) > VERIFY_LOG_MAX:
        body = body[:VERIFY_LOG_MAX] + "\n…[전문 로그 상한 절단]…\n"
    with contextlib.suppress(OSError):
        os.makedirs(TASKS_DIR, exist_ok=True)
        tmp = "%s.tmp.%s" % (path, os.getpid())
        with open(tmp, "w", encoding="utf-8") as f:
            f.write("# completion-guard verify 전문 로그\n")
            f.write("# ts=%s task=%s verdict=%s exit=%s\n" % (_now(), task_id, verdict, rc))
            f.write("# cmd=%s\n\n" % _spec_cmd_repr(spec))
            f.write(body)
        os.replace(tmp, path)
    return path


# ── escalation(§2-3 — wakeup enqueue+same-run drain 짝·esc-bundle) ───────────
def _wakeup(argv_tail):
    wk = os.path.join(_SELF_DIR, "javis_wakeup.py")
    if not os.path.isfile(wk):
        return False
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"
    with contextlib.suppress(Exception):
        r = subprocess.run([sys.executable, wk] + argv_tail, capture_output=True,
                           text=True, timeout=5, env=env)
        return r.returncode in (0, 5)  # 5=EXIT_EMPTY(이미 배달·억제) — 성공 취급(autopilot 전례)
    return False


def _pending_path_for(task_id):
    """wakeup pending 파일 경로 계산(javis_wakeup._safe/_pending_path 동형 — 갱신 판정용)."""
    safe = lambda s: _SAFE_RE.sub("_", s)[:80]  # noqa: E731 — 소형 동형 복제(경로 계산만)
    return os.path.join(ROOT, "_round", "wakeups", "pending",
                        "%s__%s.json" % (safe("master"), safe("guard-esc-%s" % task_id)))


def _escalate(task_id, sig, rc, attempt, log_path):
    """태스크당 1회 — enqueue(critical·멱등키) + same-run drain 짝 + esc-bundle 기록."""
    idem = esc_request_id(task_id, sig)   # ★단일 정의(Wave C 큐 import 재사용 · nosig 규칙 통일)
    bundle = {"schema_version": 1, "task": task_id, "sig": sig,
              "verify_out": log_path, "exit": rc, "attempt": attempt,
              "ts": _now(), "request_id": idem}
    with contextlib.suppress(OSError):
        _write_json_atomic(_bundle_path(task_id), bundle)  # 로컬 SOT — 전달 실패와 무관(조건 19①)
    reason = ("completion-guard escalation: task=%s 동일 실패 %d회(sig=%s exit=%s) — "
              "verify 전문 %s · esc-bundle %s"
              % (task_id, attempt, sig, rc, log_path, _bundle_path(task_id)))
    _wakeup(["enqueue", "--to", "master", "--task", "guard-esc-%s" % task_id,
             "--reason", reason, "--idempotency-key", idem, "--severity", "critical"])
    _wakeup(["drain", "--deliver", "--target", "master"])
    _emit("run.failed", {"agent": _role(), "task": task_id,
                         "summary": "verify 실패 %d회 — escalation(sig=%s)" % (attempt, sig),
                         "reason": "verify_fail_escalation"})


def _refresh_pending_only(task_id, sig, note):
    """오버라이드 감지 시 — 기존 pending '갱신만'(신규 escalation 금지 · 조건 01④).
    pending 부재면 아무것도 만들지 않는다(코얼레싱이 신규 생성으로 오용되는 경로 차단)."""
    if not os.path.isfile(_pending_path_for(task_id)):
        return
    # ★단일 정의 파생(R6e): override 멱등키도 `esc_request_id` 에서 **파생**한다. 손으로 쓴
    #   `"guard:%s:%s:override"` 는 sig 부재 시 `guard:T::override` 가 되어 정규화(`nosig`)
    #   규칙과 갈라졌고, 그 순간 큐↔wakeup 조인 키가 두 갈래가 된다.
    _wakeup(["enqueue", "--to", "master", "--task", "guard-esc-%s" % task_id,
             "--reason", note,
             "--idempotency-key", esc_request_id(task_id, sig) + ":override",
             "--severity", "critical"])


# ── Stop hook 본체 ───────────────────────────────────────────────────────────
class _Deadline(Exception):
    pass


def _read_hook_stdin():
    """Stop hook stdin(JSON) 소비 — stop_hook_active 등 필드만 파싱. transcript_path 는
    값만 보존하고 **내용은 절대 열지 않는다**(D5)."""
    try:
        raw = sys.stdin.read()
    except Exception:  # noqa: BLE001
        return {}
    try:
        d = json.loads(raw or "{}")
    except ValueError:
        return {}
    return d if isinstance(d, dict) else {}


def _hook_main():
    # 1) 무장 스위치 — 미무장 = 파일 stat 0회 즉시 무발동(조건 37).
    if os.environ.get("CYS_COMPLETION_GUARD") != "1":
        return EXIT_PASS
    hook = _read_hook_stdin()
    stop_hook_active = bool(hook.get("stop_hook_active"))
    sid = _surface_id()
    if not sid:
        return EXIT_PASS  # cys pane 밖 — grill_gate surface 격리 fail-open 전례
    # 2) claim 바인딩 — stat 1개(조건 23④).
    claim_p = _claim_path(sid)
    if not os.path.isfile(claim_p):
        _silence_note(sid, "claim 부재")   # E2-1(c) 침묵 계수
        return EXIT_PASS
    claim, cerr = _read_json(claim_p)
    if cerr == "corrupt" or not claim or not claim.get("task_id"):
        _alert_once(sid, "claim-corrupt", "agent.error",
                    {"agent": _role(),
                     "summary": "[completion-guard] guard-claim 파싱 실패(%s) — INFRA 무발동" % claim_p})
        _silence_note(sid, "claim 손상")
        return EXIT_PASS
    task_id = str(claim["task_id"])
    # F8: claim task_id 심층방어 — javis_task G10 allowlist 재사용 검증(경로 조합 전).
    #     불일치 = claim-corrupt 경로(경보 1회 + exit 0) — 손상/주입 claim 이 사이드카
    #     경로(<id>.guard.json 등) 조합에 유입되는 것을 차단.
    if not _claim_id_ok(task_id):
        _alert_once(sid, "claim-corrupt", "agent.error",
                    {"agent": _role(),
                     "summary": "[completion-guard] guard-claim task_id allowlist 불일치(%r)"
                                " — claim-corrupt 무발동" % task_id[:80]})
        _silence_note(sid, "claim task_id 불일치")
        return EXIT_PASS
    # stale claim(§2-1 · E2-1(a) 재정의): mtime 24h 초과 = stale. pid 는 **보조 신호**이고
    # 1차 판정은 태스크 레코드 상태다(master 셸 pid 사망이 워커 무장을 끄던 R-02 이음매 차단).
    claim_live, claim_why = _claim_liveness(claim, claim_p, task_id)
    if not claim_live:
        _alert_once(sid, "claim-stale", "agent.error",
                    {"agent": _role(),
                     "summary": "[completion-guard] stale claim(%s · task=%s) — 무시 exit 0"
                                % (claim_why, task_id)})
        _silence_note(sid, "stale claim(%s)" % claim_why, task_id)
        return EXIT_PASS
    if "보조 신호로 강등" in claim_why:
        # 정보성 1회 — "pid 는 죽었지만 태스크가 살아 있어 계속 무장한다"를 표면화한다.
        _alert_once(sid, "claim-pid-demoted", "agent.error",
                    {"agent": _role(),
                     "summary": "[completion-guard] claim pid 강등 판정(%s · task=%s) — "
                                "태스크 활성이라 무장 유지(R-02 교정 경로)" % (claim_why, task_id)})
    # 3) 회로차단기 disarm — 이후 무발동(재무장=master 가 마커 삭제).
    if os.path.isfile(_disarm_path(task_id)):
        _silence_note(sid, "회로차단기 disarm(%s)" % task_id, task_id)
        return EXIT_PASS
    # 3b) G6: 무장 조합 위험 경보 — 무장 guard 가 실제 발동(claim 유효)한 시점에 1회
    #     (에피소드 코얼레싱 — _alert_once). best-effort·판정 불개입.
    _warn_armed_combo(sid)
    # 4) 태스크 레코드 — 부재/파싱 실패 = INFRA_ERROR 경로(§2-1).
    task, terr = _read_json(os.path.join(TASKS_DIR, "%s.json" % task_id))
    state, corrupted = _load_guard_state(task_id)
    _DEADLINE_CTX.update({"state": state, "sid": sid})  # F1: 데드라인 발화 시 INFRA 적재 컨텍스트
    if corrupted:
        _alert_once(sid, "state-corrupt-%s" % task_id, "run.failed",
                    {"agent": _role(), "task": task_id,
                     "summary": "guard.json 손상 — corrupt 격리+재생성(카운터 0)",
                     "reason": "guard_infra"})
        return _finish_infra(state, sid, "guard.json 손상(격리 재생성)")
    if terr is not None or task is None:
        _emit("run.failed", {"agent": _role(), "task": task_id,
                             "summary": "태스크 레코드 %s(%s.json) — INFRA fail-open" % (terr, task_id),
                             "reason": "guard_infra"})
        return _finish_infra(state, sid, "태스크 레코드 %s" % terr)
    # ── 양보 사다리(§2-2 — 자기판정·순서 무의존 원칙 하의 자체 평가 순서) ──
    # 1단 CYCLE 마커(TTL 30분).
    cyc = _cycle_marker_path(sid)
    if os.path.isfile(cyc):
        try:
            age = time.time() - os.stat(cyc).st_mtime
        except OSError:
            age = None
        if age is not None and age <= CYCLE_TTL_SEC:
            return _finish_skip(state, "SKIPPED_CYCLE")
        _alert_once(sid, "cycle-stale", "agent.error",
                    {"agent": _role(),
                     "summary": "[completion-guard] CYCLE 마커 만료(%s) — 무시하고 진행(TTL 30분)"
                                % cyc})
    # 2단 부트 grace(120s — 마커 부재=부트 미완=보수측).
    try:
        grace = float(os.environ.get("CYS_GUARD_GRACE_SEC", "") or GRACE_SEC_DEFAULT)
    except ValueError:
        grace = GRACE_SEC_DEFAULT
    bm = _boot_marker_path()
    try:
        boot_age = time.time() - os.stat(bm).st_mtime
    except OSError:
        boot_age = None
    if boot_age is None or boot_age < grace:
        return _finish_skip(state, "SKIPPED_GRACE")
    # 오버라이드 감지(F2 재설계 — 카운터 기반·CYCLE 마커/부트 grace 양보 판정 **이후**:
    #   인가 clear ≠ 오버라이드 — 마커 존재 시 위 1·2단 선반환으로 감지 자체가 억제된다).
    #   판별자: block_count ≥ 하네스 Stop 블록 상한(CLAUDE_CODE_STOP_HOOK_BLOCK_CAP 또는 8)
    #   상태에서 verify PASS 없이(PASS 는 block_count 를 0 으로 리셋) 새 Stop 도착 =
    #   하네스가 블록을 무시/강제 종결했거나 그에 준하는 비정상 스트릭.
    #   ※stop_hook_active 는 보조 힌트로만 기록 — T0 실측상 오버라이드에서도 true 유지
    #   (역방향)이고 false 는 /clear·인터럽트와 구분 불가(자인). 판별자로 쓰지 않는다.
    #   코얼레싱: 에피소드당 1회 — `_alert_once` 마커 `override-<task>-<sig>`(TTL 6h) 가
    #   이벤트·pending 갱신을 함께 게이트해 반복 Stop 재발화를 금지한다.
    if int(state.get("block_count", 0)) >= _stop_block_cap():
        o_sig = state.get("last_sig") or "nosig"
        fired = _alert_once(
            sid, "override-%s-%s" % (task_id, o_sig), "agent.error",
            {"agent": _role(),
             "summary": "[completion-guard][tamper-adjacent] task=%s — block_count %d ≥ 하네스 "
                        "블록 상한 %d 인데 PASS 없이 새 Stop 도착(오버라이드 의심 · "
                        "stop_hook_active=%s 는 보조 힌트) · pending 갱신만"
                        % (task_id, int(state.get("block_count", 0)), _stop_block_cap(),
                           stop_hook_active)})
        if fired:
            _refresh_pending_only(task_id, o_sig,
                                  "completion-guard 오버라이드 감지: task=%s 블록 미해소"
                                  "(block_count ≥ 하네스 상한) 채 새 Stop 도착" % task_id)
    # 3단 ctx% 선확인.
    ctx_pct, ctx_why = _measure_ctx(sid)
    no_block_mode = False
    if ctx_pct is None:
        no_block_mode = True
        _alert_once(sid, "ctx-measure-fail", "agent.error",
                    {"agent": _role(),
                     "summary": "[completion-guard] ctx 측정 실패(%s) — NO_BLOCK_PASS 모드"
                                % ctx_why})
    elif ctx_pct >= CTX_YIELD_PCT:
        if not state.get("context_yield"):
            _emit("run.failed", {"agent": _role(), "task": task_id,
                                 "summary": "ctx %.0f%% ≥ %.0f%% — 블록 양보(context yield)"
                                            % (ctx_pct, CTX_YIELD_PCT),
                                 "reason": "context_yield"})
        state["context_yield"] = True
        return _finish_skip(state, "SKIPPED_CONTEXT")
    # 4단 resource gate(--require-context 원자쌍 — 조건 35).
    gkind, gdetail, trip = _resource_gate(ctx_pct)
    if gkind == "infra":
        _emit("run.failed", {"agent": _role(), "task": task_id,
                             "summary": "resource_gate 측정 실패: %s" % gdetail,
                             "reason": "guard_infra"})
        return _finish_infra(state, sid, gdetail)
    if gkind in ("skip_soft", "skip_hard"):
        if trip and all(k in trip for k in ("metric", "value")):
            _alert_once(sid, "resource-%s" % gkind,
                        "resource.hard" if gkind == "skip_hard" else "resource.soft",
                        {"metric": str(trip.get("metric")), "value": str(trip.get("value")),
                         "threshold": str(trip.get("hard" if gkind == "skip_hard" else "soft"))})
        else:
            _alert_once(sid, "resource-%s" % gkind, "agent.error",
                        {"agent": _role(),
                         "summary": "[completion-guard] %s — SKIPPED_RESOURCE" % gdetail})
        if gkind == "skip_hard":
            state["deferred"] = True  # 조건 04① — verify-deferred 통과 마킹
        return _finish_skip(state, "SKIPPED_RESOURCE")
    # 5단 verifier tax(부재=무제한 — 배포 전 호환). B2(조건 04②): 소진 이벤트는 워커별
    # 발화가 아니라 resource.hard 플릿 단일 발행(멱등 마커 — 창당 정확 1회).
    exhausted, bdetail, bstats = _budget_check()
    if exhausted:
        state["deferred"] = True
        _budget_hard_emit_once(bstats)
        print("[completion-guard] verifier tax 소진(%s) — SKIPPED_BUDGET(verify-deferred 통과)"
              % bdetail, file=sys.stderr)
        return _finish_skip(state, "SKIPPED_BUDGET")  # 카운터 동결 — block/infra 무변경
    # 6단 verify_spec 부재.
    spec = task.get("verify_spec")
    if not isinstance(spec, dict) or not spec:
        return _finish_skip(state, "SKIPPED_NO_SPEC")
    # ── verify 실행 ── (E2-1(c): 여기까지 왔다 = 실제로 검증한다 = 침묵 스트릭 절단)
    _silence_reset(sid)
    res = _run_verify(spec, task_id, sid)
    if res.get("timed_out_any"):
        state["timeout_violations"] = int(state.get("timeout_violations", 0)) + 1
    # B2(조건 07②) + G5 짝: 강등 소비는 **무조건 호출** — 데드라인 경로(_on_deadline 은 스폰
    # 생략)가 적재한 위반분도 다음 정상 Stop 이 여기서 소비한다(이번 run 의 타임아웃 여부와
    # 무관). 함수 자체가 <2 또는 calibrated≠true 면 즉시 반환이라 평시 비용 0(스폰 없음).
    _maybe_demote_calibrated(task_id, state, spec)
    if res.get("survivors_any"):
        _emit("agent.error",
              {"agent": _role(),
               "summary": "[completion-guard][tamper-adjacent] task=%s verify 프로세스 그룹 잔존 "
                          "감지 — killpg 정리(조건 07③)" % task_id})
    verdict = res["verdict"]
    if verdict == "PASS":
        state.update({"block_count": 0, "infra_count": 0, "last_verdict": "PASS",
                      "last_sig": None, "escalated_at": None, "deferred": False})
        _save_guard_state(state)
        return EXIT_PASS
    if verdict == "INFRA_ERROR":
        _emit("run.failed", {"agent": _role(), "task": task_id,
                             "summary": "verify 인프라 오류: %s" % res.get("detail"),
                             "reason": "guard_infra"})
        return _finish_infra(state, sid, res.get("detail"))
    # VERIFY_FAIL — verify 실행 성공이므로 infra 연속성 단절(§2-4).
    state["infra_count"] = 0
    sig = _failure_sig(_spec_cmd_repr(spec), res.get("rc"), res.get("out"))
    log_path = _write_verify_log(task_id, spec, res.get("rc"), res.get("out"), verdict)
    if no_block_mode:
        # NO_BLOCK_PASS — verify 는 실행·기록했으나 블록 금지(조건 10 문언). 카운터 무증가.
        state.update({"last_verdict": "NO_BLOCK_PASS", "last_sig": sig})
        _save_guard_state(state)
        return EXIT_PASS
    state["block_count"] = int(state.get("block_count", 0)) + 1
    state["last_sig"] = sig
    state["last_verdict"] = "VERIFY_FAIL"
    attempt = state["block_count"]
    esc_n = _esc_n()
    if state.get("escalated_at"):
        # 사후-escalation(조건 01①) — 신규 sig 포함 전부 통과+기록.
        _save_guard_state(state)
        print("[completion-guard] task=%s verdict=VERIFY_FAIL(사후-escalation 통과) "
              "attempt=%d sig=%s — 재블록은 verify PASS 후 재임계 시에만" % (task_id, attempt, sig),
              file=sys.stderr)
        return EXIT_PASS
    if attempt >= esc_n:
        state["escalated_at"] = _now()
        _save_guard_state(state)
        _escalate(task_id, sig, res.get("rc"), attempt, log_path)
    else:
        _save_guard_state(state)
    state["last_blocked_at"] = _now()
    _save_guard_state(state)
    # 블록 — 기계 접두(조건 19②) + 4KB 캡 재주입 + 전문 경로.
    sys.stderr.write(
        "[completion-guard] task=%s verdict=VERIFY_FAIL attempt=%d/%d cmd=%s — "
        "이 블록은 검증 실패이며 인프라 오류가 아님\n"
        % (task_id, attempt, esc_n, _spec_cmd_repr(spec)[:200]))
    sys.stderr.write(_cap_output(res.get("out")) + "\n")
    sys.stderr.write("전문: %s\n" % log_path)
    sys.stderr.write("검증을 통과시킨 뒤 종료하라 — 검증 명령을 직접 실행해 고쳐라"
                     "(도구 실행 턴은 하네스 블록 카운터를 리셋한다 · D2).\n")
    return EXIT_BLOCK


def _finish_skip(state, verdict):
    """양보·건너뜀 종결. E2-1(c): **양보는 통과가 아니다** — SKIPPED_* 는 "검증하지 않았다"의
    다른 이름이라 침묵 스트릭에 계수한다(sid 는 state 적재 시 함께 박힌 _DEADLINE_CTX 재사용
    — 호출부 6곳 시그니처 churn 회피). NO_BLOCK_PASS 는 verify 를 **실행**했으므로 여기로
    오지 않는다(별도 경로)."""
    state["last_verdict"] = verdict
    _save_guard_state(state)
    with contextlib.suppress(Exception):
        _silence_note(_DEADLINE_CTX.get("sid"), verdict, state.get("task_id"))
    return EXIT_PASS


def _finish_infra(state, sid, detail):
    """INFRA_ERROR 종결 — infra_count 연속 증가·3회 도달 시 회로차단기 disarm(§2-4)."""
    state["infra_count"] = int(state.get("infra_count", 0)) + 1
    state["last_verdict"] = "INFRA_ERROR"
    if state["infra_count"] >= 3 and not os.path.isfile(_disarm_path(state["task_id"])):
        state["disarm_reason"] = "infra_count %d회 연속(마지막: %s)" % (state["infra_count"], detail)
        with contextlib.suppress(OSError):
            os.makedirs(TASKS_DIR, exist_ok=True)
            with open(_disarm_path(state["task_id"]), "w", encoding="utf-8") as f:
                f.write("%s %s\n재무장: master 가 이 마커 파일을 삭제(명시 명령)\n"
                        % (_now(), state["disarm_reason"]))
        _emit("agent.error",
              {"agent": _role(),
               "summary": "[completion-guard] task=%s 회로차단기 개방 — guard-disarmed(%s)"
                          % (state["task_id"], state["disarm_reason"])})
        _wakeup(["enqueue", "--to", "master", "--task", "guard-disarm-%s" % state["task_id"],
                 "--reason", "completion-guard 무장해제: %s" % state["disarm_reason"],
                 "--idempotency-key", "guard-disarm:%s" % state["task_id"],
                 "--severity", "critical"])
        _wakeup(["drain", "--deliver", "--target", "master"])
    _save_guard_state(state)
    return EXIT_PASS


def _on_deadline(detail):
    """F1: _Deadline 발화 종결 — ①추적 그룹 killpg(고아 0) ②INFRA_ERROR 상태 적재
    (`_finish_infra` 경유 — infra_count++·timeout_violations++·회로차단기 편입)
    ③escalated_at 저장~_escalate 사이 발화 시 esc-bundle 보강 기록(escalation 소실 창)."""
    _kill_active_groups()
    state, sid = _DEADLINE_CTX.get("state"), _DEADLINE_CTX.get("sid")
    if state is None:
        return EXIT_PASS  # state 로드 전(무장·claim 판정 구간) — 적재할 대상 없음
    state["timeout_violations"] = int(state.get("timeout_violations", 0)) + 1
    task_id = state.get("task_id") or ""
    # ★G5(b2-bugs·성찰 2): 데드라인 경로에서는 _maybe_demote_calibrated 스폰을 **생략**한다 —
    #   이 핸들러는 자체 데드라인 소진 직후의 잔여 시간 창에서 도는 정리 경로라, 최대 10s
    #   subprocess 스폰이 3중 상한(SIGALRM·훅 타임아웃·하네스) 여유를 잠식한다. 위반 카운트는
    #   위에서 적재됐으므로 다음 정상 Stop 의 무조건 소비 지점(_hook_main — G5 짝)이 처리한다.
    if state.get("escalated_at") and not os.path.isfile(_bundle_path(task_id)):
        sig = state.get("last_sig") or "nosig"
        bundle = {"schema_version": 1, "task": task_id, "sig": sig,
                  "verify_out": _verify_log_path(task_id), "exit": None,
                  "attempt": int(state.get("block_count", 0)), "ts": _now(),
                  "request_id": esc_request_id(task_id, sig),
                  "note": "deadline-handler backfill(F1 — escalated_at 저장~_escalate 소실 창)"}
        with contextlib.suppress(OSError):
            _write_json_atomic(_bundle_path(task_id), bundle)
    _emit("run.failed", {"agent": _role(), "task": task_id,
                         "summary": "completion-guard %s — INFRA fail-open" % detail,
                         "reason": "guard_infra"})
    return _finish_infra(state, sid, detail)


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if "--self-test" in argv:
        return self_test()
    # E2-2: 관리 서브커맨드 — **명시 argv 로만** 진입한다. Stop 훅 호출은 인자가 없으므로
    #       무장 스위치 선검사(파일 stat 0회 무발동 계약 · 조건 37)는 그대로 보존된다.
    if argv and argv[0] in ("budget-init", "budget-status"):
        return cmd_budget(argv)
    # 무장 스위치는 어떤 파일 접근보다 먼저(완전 무발동 계약).
    if os.environ.get("CYS_COMPLETION_GUARD") != "1":
        return EXIT_PASS
    # E1-3(BLOCKER R-09): Windows 기본 무장 거부 — **상태 파일을 만들기 전에** 끊는다.
    #   여기서 끊지 않으면 미지원 경로가 INFRA_ERROR 로 수렴해 태스크마다 guard-disarmed
    #   마커 + critical wakeup 이 쌓인다(영구 disarm + 폭주 교차). 거부는 파일 0개·경보 0건.
    _win_refuse = _windows_arming_refusal()
    if _win_refuse:
        print("[completion-guard] %s" % _win_refuse, file=sys.stderr)
        return EXIT_PASS
    deadline = _self_deadline()
    _disarm_deadline = _arm_self_deadline(deadline)
    try:
        return _hook_main()
    except _Deadline:
        print("[completion-guard] 자체 데드라인 %ds 초과 — killpg+INFRA 적재 후 "
              "fail-open(exit 0)" % deadline, file=sys.stderr)
        return _on_deadline("자체 데드라인 %ds 초과(SIGALRM)" % deadline)
    except Exception as e:  # noqa: BLE001 — 최상위 경계: 어떤 예외도 블록으로 승격 금지(조건 14)
        import traceback
        traceback.print_exc()
        _kill_active_groups()  # F1: 예외 이탈 시에도 verify 그룹 잔존 차단
        print("[completion-guard] 내부 예외 — INFRA fail-open(exit 0): %s" % e, file=sys.stderr)
        return EXIT_PASS
    finally:
        _disarm_deadline()


def _self_test_platform():
    """E1-2(R-01)·E1-3(R-09) 회귀 — 플랫폼 분기와 데드라인 겹을 **모킹**으로 검증.

    실기 Windows 가 없으므로 `os.name` 을 뒤집어 분기 함수를 직접 태운다(프로세스 스폰 0 ·
    실행 시간 ~0). 이 방식의 한계는 자인한다: **구문·분기 선택만** 증명하고 Win32 실동작
    (taskkill 실재·CREATE_NEW_PROCESS_GROUP 의미)은 증명하지 못한다 — 그래서 Windows 무장은
    기본 거부로 남는다(`_windows_arming_refusal`).
    """
    fails = []

    def chk(cond, msg):
        if not cond:
            fails.append("E1 platform: " + msg)

    real_name = os.name
    real_env = dict(os.environ)
    try:
        # ── ① 셸 argv 분기: posix=/bin/sh · nt=COMSPEC ──
        os.name = "posix"
        chk(_shell_argv("echo hi") == ["/bin/sh", "-c", "echo hi"],
            "posix shell argv 회귀: %s" % _shell_argv("echo hi"))
        os.name = "nt"
        os.environ["COMSPEC"] = r"C:\WINDOWS\system32\cmd.exe"
        chk(_shell_argv("echo hi") == [r"C:\WINDOWS\system32\cmd.exe", "/c", "echo hi"],
            "nt shell argv 미분기(= /bin/sh 하드코딩 잔존): %s" % _shell_argv("echo hi"))
        os.environ.pop("COMSPEC", None)
        chk(_shell_argv("x")[0] == "cmd.exe", "nt COMSPEC 부재 폴백 실패: %s" % _shell_argv("x"))

        # ── ② Windows 기본 무장 거부(+opt-in 해제) ──
        os.environ.pop("CYS_GUARD_ALLOW_WINDOWS", None)
        chk(_windows_arming_refusal() is not None, "nt 인데 무장이 거부되지 않음(R-09 재발)")
        chk("CYS_GUARD_ALLOW_WINDOWS" in (_windows_arming_refusal() or ""),
            "거부 문면에 해제 경로 부재")
        os.environ["CYS_GUARD_ALLOW_WINDOWS"] = "1"
        chk(_windows_arming_refusal() is None, "opt-in 인데 여전히 거부")
        os.name = "posix"
        os.environ.pop("CYS_GUARD_ALLOW_WINDOWS", None)
        chk(_windows_arming_refusal() is None, "posix 인데 무장이 거부됨(회귀 — 라이브 정지)")

        # ── ③ _kill_active_groups 가 nt 에서 **taskkill 을 실제로 부른다**(종전: set 만 비움) ──
        called = []
        real_run = subprocess.run
        try:
            os.name = "nt"
            subprocess.run = lambda *a, **k: called.append(a[0]) or None
            _ACTIVE_PGIDS.add(424242)
            _kill_active_groups()
            chk(len(called) == 1 and called[0][:1] == ["taskkill"],
                "nt kill 경로가 taskkill 을 부르지 않음: %s" % called)
            chk(424242 not in _ACTIVE_PGIDS, "nt kill 후 추적 set 미정리")
        finally:
            subprocess.run = real_run
            _ACTIVE_PGIDS.discard(424242)

        # ── ④ 워치독: 전 플랫폼 무장 + 해제 콜러블 · POSIX 는 SIGALRM 뒤(백스톱) ──
        os.name = "posix"
        os.environ["CYS_GUARD_SELF_DEADLINE_SEC"] = "45"
        disarm = _arm_self_deadline(_self_deadline())
        wd = [t for t in threading.enumerate() if isinstance(t, threading.Timer)]
        chk(len(wd) >= 1, "posix 워치독 스레드 미기동")
        chk(all(t.daemon for t in wd), "워치독이 daemon 이 아님(프로세스 종료를 붙잡는다)")
        chk(abs(getattr(wd[0], "interval", 0) - (45 + WATCHDOG_MARGIN_SEC)) < 0.01,
            "posix 워치독이 SIGALRM 보다 먼저 선다(interval=%s)" % getattr(wd[0], "interval", None))
        chk(signal.getitimer(signal.ITIMER_REAL)[0] > 0, "posix SIGALRM 미무장")
        disarm()
        chk(signal.getitimer(signal.ITIMER_REAL)[0] == 0, "disarm 후 SIGALRM 잔존")
        for t in wd:
            t.join(2)
        chk(not any(t.is_alive() for t in wd), "disarm 후 워치독 타이머 잔존")
        # Windows 경로: SIGALRM 없이도 워치독이 유일 겹으로 서고 여유 0(= deadline 정각)
        os.name = "nt"
        disarm2 = _arm_self_deadline(_self_deadline())
        wd2 = [t for t in threading.enumerate() if isinstance(t, threading.Timer) and t.is_alive()]
        chk(len(wd2) == 1 and abs(wd2[0].interval - 45) < 0.01,
            "nt 워치독 interval 오류(= 자체 상한 0겹 재발): %s"
            % [getattr(t, "interval", None) for t in wd2])
        disarm2()
        for t in wd2:
            t.join(2)
        chk(not any(t.is_alive() for t in wd2), "nt disarm 후 워치독 잔존")
    finally:
        os.name = real_name
        os.environ.clear()
        os.environ.update(real_env)
    return fails


# ═════════════════════════ self-test(§2-4 필수 5케이스 + 완료 기준 배터리) ═════
def self_test():
    import tempfile

    # ── F10(G1 hard assert): 격리 env 필수 — JAVIS_ROOT·CYS_PROBE_RUNS 미설정 시 실행 거부.
    #    자급 격리(tmpdir 밀폐)와 병행하는 이중 방어 — cwd/기본 원장 폴백의 라이브 오염 차단
    #    (T1 하네스 G1 규약 정렬).
    _missing_env = [k for k in ("JAVIS_ROOT", "CYS_PROBE_RUNS") if not os.environ.get(k)]
    if _missing_env:
        print("javis_completion_guard self-test 실행 거부 — 격리 env 부재: %s "
              "(G1 라이브/사이드 경계 hard assert · 격리 스크래치 경로로 설정 후 재실행)"
              % ",".join(_missing_env), file=sys.stderr)
        return 1

    # ── E1-3(R-09): 이 배터리는 **POSIX 전용**이다 — `#!/bin/sh` 셔임·`os.mkfifo`·killpg 픽스처
    #    의존. Windows 에서 조용히 크래시(AttributeError)하거나, 더 나쁘게는 "그린"으로 읽히지
    #    않도록 여기서 명시 중단한다. 플랫폼 분기 케이스(_self_test_platform)는 플랫폼 무관이라
    #    실행하고 결과를 보고한다. **Windows 회귀 하네스 부재가 Windows 기본 무장 거부의 근거**다.
    if os.name != "posix":
        pf = _self_test_platform()
        print("javis_completion_guard self-test: POSIX 전용 배터리 미실행(os.name=%s · 셸 셔임·"
              "FIFO·killpg 픽스처 의존). 플랫폼 분기 케이스만 실행 — %s. 이 플랫폼에 그린 러너는 "
              "없으며 그것이 CYS_GUARD_ALLOW_WINDOWS 기본 거부의 근거다."
              % (os.name, "OK" if not pf else "FAIL %d건: %s" % (len(pf), pf)), file=sys.stderr)
        return 1

    self_path = os.path.abspath(__file__)
    fails = []

    def chk(cond, msg):
        if not cond:
            fails.append(msg)

    with tempfile.TemporaryDirectory(prefix="guard-selftest-") as td:
        # ── 공용 셔임·픽스처 준비 ──
        shim_dir = os.path.join(td, "shim")
        os.makedirs(shim_dir)
        shim_log = os.path.join(td, "cys-shim.log")
        status_file = os.path.join(td, "status.json")
        hang_fifo = os.path.join(td, "status-hang.fifo")
        os.mkfifo(hang_fifo)  # F1b: writer 없는 FIFO — status 응답 무기 지연 시뮬
        with open(os.path.join(shim_dir, "cys"), "w", encoding="utf-8") as f:
            f.write('#!/bin/sh\necho "$@" >> "%s"\ncase "$1" in\n'
                    '  status) [ -n "$CYS_SHIM_STATUS_HANG" ] && exec cat "%s"; '
                    'cat "%s" 2>/dev/null || exit 1 ;;\n'
                    '  send) exit "${CYS_SHIM_SEND_RC:-1}" ;;\n'
                    '  *) exit 0 ;;\nesac\n' % (shim_log, hang_fifo, status_file))
        os.chmod(os.path.join(shim_dir, "cys"), 0o755)
        boot_marker = os.path.join(td, "boot.marker")
        with open(boot_marker, "w") as f:
            f.write("boot\n")
        os.utime(boot_marker, (time.time() - 3600, time.time() - 3600))  # grace 경과 상태

        sess_file = os.path.join(td, "sess.jsonl")
        with open(sess_file, "w") as f:
            f.write("{}\n")

        def write_status(ctx_pct, source="statusline", sid=7):
            now = time.time()
            os.utime(sess_file, (now - 10, now - 10))
            with open(status_file, "w", encoding="utf-8") as sf:
                json.dump({"surfaces": [{"surface_id": sid, "role": "worker",
                                         "usage": {"source": source, "ctx_pct": ctx_pct,
                                                   "ctx_tokens": 1000,
                                                   "session_file": sess_file,
                                                   "updated_at": now}}]}, sf)

        write_status(20.0)

        roots = {}

        def new_root(name):
            root = os.path.join(td, name)
            tasks = os.path.join(root, "_round", "tasks")
            os.makedirs(tasks, exist_ok=True)
            roots[name] = root
            return root, tasks

        def mk_task(tasks, tid, spec, status="in_progress"):
            rec = {"id": tid, "title": tid, "status": status,
                   "created_at": _now(), "updated_at": _now()}
            if spec is not None:
                rec["verify_spec"] = spec
            with open(os.path.join(tasks, "%s.json" % tid), "w", encoding="utf-8") as f:
                json.dump(rec, f, ensure_ascii=False)

        def mk_claim(tasks, sid, tid, pid=-1, pid_source="caller(자기 pane checkout)"):
            """E2-1: pid=-1 = 이 프로세스 pid(기본·종전 동형) · pid=None = 위임 경로 미기록."""
            rec = {"task_id": tid, "pid": os.getpid() if pid == -1 else pid,
                   "pid_source": pid_source, "ts": _now()}
            with open(os.path.join(tasks, ".guard-claim.%s" % sid), "w",
                      encoding="utf-8") as f:
                json.dump(rec, f)

        def run_guard(root, stdin_obj=None, env_extra=None, armed=True):
            env = dict(os.environ)
            for k in ("CYS_COMPLETION_GUARD", "CYS_SURFACE_ID", "AITERM_SURFACE_ID",
                      "CYS_GUARD_ESC_N", "CYS_GUARD_RESOURCE_ARGS", "CYS_SHIM_SEND_RC",
                      "CYS_GUARD_SELF_DEADLINE_SEC", "CLAUDE_CODE_STOP_HOOK_BLOCK_CAP",
                      "CYS_SHIM_STATUS_HANG", "JAVIS_VERIFY_BUDGET_RUNS",
                      "JAVIS_VERIFY_BUDGET_SECS", "GRILL_STOP_SELF_CAP"):
                env.pop(k, None)
            env.update({
                # G6 밀폐: grill 마커 기본 경로(~/Desktop/CYSjavis — 라이브)를 읽지 않게
                # root 내 부재 경로로 핀. G6 케이스만 env_extra 로 실마커 주입.
                "GRILL_MARKER": os.path.join(root, ".grill-absent.json"),
                "JAVIS_ROOT": root,
                "HUD_STATE_DIR": os.path.join(root, "hud"),
                "CYS_PROBE_RUNS": os.path.join(root, "probe_runs.jsonl"),
                "CYS_NO_AUTOSTART": "1",
                "CYS_GUARD_BOOT_MARKER": boot_marker,
                "CYS_GUARD_LIVENESS": "alive",
                "CYS_ROLE": "worker",
                "CYS_GUARD_RESOURCE_ARGS":
                    "--servers-override 0 --nodes-override 0 --load-override 0.0",
                "JAVIS_WAKEUP_LIVENESS": "alive",
                "JAVIS_FASTFAIL_MAX": "99",
                "PATH": shim_dir + os.pathsep + os.environ.get("PATH", ""),
            })
            if armed:
                env["CYS_COMPLETION_GUARD"] = "1"
            env["CYS_SURFACE_ID"] = "7"
            if env_extra:
                env.update(env_extra)
            stdin_text = json.dumps(stdin_obj or {"stop_hook_active": False,
                                                  "transcript_path": "/nonexistent.jsonl"})
            r = subprocess.run([sys.executable, self_path], input=stdin_text,
                               capture_output=True, text=True, env=env, timeout=120)
            return r.returncode, r.stdout, r.stderr

        def run_cli(root, args, env_extra=None):
            """E2-2: 관리 서브커맨드(budget-init/status) 실행 — **무장 env 없이** 돌린다.
            Stop 훅 경로(인자 0개)와 분리돼 있음을 실행 자체로 확인한다."""
            env = dict(os.environ)
            for k in ("CYS_COMPLETION_GUARD", "JAVIS_VERIFY_BUDGET_RUNS",
                      "JAVIS_VERIFY_BUDGET_SECS"):
                env.pop(k, None)
            env.update({"JAVIS_ROOT": root,
                        "HUD_STATE_DIR": os.path.join(root, "hud"),
                        "CYS_PROBE_RUNS": os.path.join(root, "probe_runs.jsonl"),
                        "CYS_NO_AUTOSTART": "1",
                        "PATH": shim_dir + os.pathsep + os.environ.get("PATH", "")})
            if env_extra:
                env.update(env_extra)
            r = subprocess.run([sys.executable, self_path] + list(args),
                               capture_output=True, text=True, env=env, timeout=120)
            return r.returncode, r.stdout, r.stderr

        def gstate(root, tid):
            p = os.path.join(root, "_round", "tasks", "%s.guard.json" % tid)
            with open(p, encoding="utf-8") as f:
                return json.load(f)

        def spool_lines(root):
            p = os.path.join(root, "hud", "evt_spool.jsonl")
            if not os.path.isfile(p):
                return []
            with open(p, encoding="utf-8") as f:
                return [json.loads(ln) for ln in f.read().splitlines() if ln.strip()]

        ok_spec = {"mode": "command", "cmd": "exit 0",
                   "pass_rule": {"kind": "exit_map", "pass_exits": [0]}, "timeout_s": 5}
        fail_spec = {"mode": "command", "cmd": "echo boom; exit 7",
                     "pass_rule": {"kind": "exit_map", "pass_exits": [0]}, "timeout_s": 5}

        # ── ④(§2-4) env 무장 OFF → 완전 무발동 ──
        root, tasks = new_root("r-armoff")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1")
        rc, _o, _e = run_guard(root, armed=False)
        chk(rc == 0, "무장 OFF 가 exit 0 아님: %s" % rc)
        chk(not os.path.isfile(os.path.join(tasks, "T1.guard.json")),
            "무장 OFF 인데 guard.json 생성(무발동 위반)")
        # claim 부재 → 즉시 exit 0(기준 ⑥ 후단)
        root2, tasks2 = new_root("r-noclaim")
        mk_task(tasks2, "T1", fail_spec)
        rc, _o, _e = run_guard(root2)
        chk(rc == 0, "claim 부재가 exit 0 아님: %s" % rc)
        chk(not os.path.isfile(os.path.join(tasks2, "T1.guard.json")),
            "claim 부재인데 guard.json 생성")

        # ── stale claim: pid 사망 ∧ **태스크 비활성** → exit 0 + 경보 1회 ──
        #    (E2-1: pid 사망 단독으로는 더 이상 stale 이 아니다 — 아래 R-02 배터리 참조)
        root, tasks = new_root("r-stale")
        mk_task(tasks, "T1", fail_spec, status="done")
        mk_claim(tasks, "7", "T1")
        rc, _o, _e = run_guard(root, env_extra={"CYS_GUARD_LIVENESS": "dead"})
        chk(rc == 0, "stale claim(pid dead ∧ 비활성)이 exit 0 아님: %s" % rc)
        ev = [x for x in spool_lines(root) if x["type"] == "agent.error"
              and "stale claim" in x["payload"].get("summary", "")]
        chk(len(ev) == 1, "stale claim 경보 1회 아님: %d" % len(ev))
        rc, _o, _e = run_guard(root, env_extra={"CYS_GUARD_LIVENESS": "dead"})
        ev = [x for x in spool_lines(root) if x["type"] == "agent.error"
              and "stale claim" in x["payload"].get("summary", "")]
        chk(len(ev) == 1, "stale claim 경보가 코얼레싱 안 됨: %d" % len(ev))

        # ══ E2-1(BLOCKER R-02) 배터리 — guard-claim 침묵형 무발동 ═══════════════
        # (a) master 셸 pid 사망 + 태스크 활성 → claim 유효 유지 → **실제 발동**(exit 2).
        #     종전 코드에서는 여기서 영구 exit 0(무장했는데 아무것도 안 함)이었다.
        root, tasks = new_root("r-r02-alive")
        mk_task(tasks, "T1", fail_spec)                 # status=in_progress
        mk_claim(tasks, "7", "T1")
        rc, _o, err = run_guard(root, env_extra={"CYS_GUARD_LIVENESS": "dead"})
        chk(rc == 2, "R-02(a): master pid 사망인데 태스크 활성 — 발동(exit 2) 실패: %s" % rc)
        chk(os.path.isfile(os.path.join(tasks, "T1.guard.json")),
            "R-02(a): 발동했는데 guard.json 미생성")
        ev = [x for x in spool_lines(root) if x["type"] == "agent.error"
              and "claim pid 강등 판정" in x["payload"].get("summary", "")]
        chk(len(ev) == 1, "R-02(a): pid 강등 정보성 경보 1회 아님: %d" % len(ev))
        # (a2) 위임 경로 pid 미기록(pid=None) + 태스크 활성 → 동일하게 발동
        root, tasks = new_root("r-r02-nopid")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1", pid=None, pid_source="delegated(미기록)")
        rc, _o, _e = run_guard(root, env_extra={"CYS_GUARD_LIVENESS": "dead"})
        chk(rc == 2, "R-02(a2): pid 미기록 claim + 활성 태스크가 발동 안 함: %s" % rc)
        # (c) 침묵 탐지기 — claim 부재로 연속 N회 무검증 종결 → 임계에서 경보 1회 + 상태파일
        root, tasks = new_root("r-silence")
        mk_task(tasks, "T1", fail_spec)                 # claim 은 만들지 않는다
        for i in range(4):
            rc, _o, _e = run_guard(root, env_extra={"CYS_GUARD_SILENCE_N": "3"})
            chk(rc == 0, "R-02(c): 침묵 경로가 exit 0 아님(i=%d): %s" % (i, rc))
        sil_p = os.path.join(tasks, ".guard-silence.7.json")
        chk(os.path.isfile(sil_p), "R-02(c): 침묵 상태 파일 미생성")
        with open(sil_p, encoding="utf-8") as f:
            sil = json.load(f)
        chk(sil.get("streak") == 4 and sil.get("alerted_at"),
            "R-02(c): 스트릭/경보 기록 오류: %r" % sil)
        ev = [x for x in spool_lines(root) if x["type"] == "agent.error"
              and "침묵 탐지" in x["payload"].get("summary", "")]
        chk(len(ev) == 1, "R-02(c): 침묵 경보 1회 아님(코얼레싱 포함): %d" % len(ev))
        # 임계 미달에서는 무경보(오탐 억제)
        root, tasks = new_root("r-silence-quiet")
        mk_task(tasks, "T1", fail_spec)
        run_guard(root, env_extra={"CYS_GUARD_SILENCE_N": "5"})
        ev = [x for x in spool_lines(root) if "침묵 탐지" in x["payload"].get("summary", "")]
        chk(ev == [], "R-02(c): 임계 미달인데 침묵 경보 발화: %d" % len(ev))
        # 실제 verify 가 돌면 스트릭 절단(파일 삭제)
        root, tasks = new_root("r-silence-reset")
        mk_task(tasks, "T1", ok_spec)
        rc, _o, _e = run_guard(root, env_extra={"CYS_GUARD_SILENCE_N": "2"})  # claim 부재 1회
        chk(os.path.isfile(os.path.join(tasks, ".guard-silence.7.json")),
            "R-02(c): 절단 전 상태 파일 부재")
        mk_claim(tasks, "7", "T1")
        rc, _o, _e = run_guard(root, env_extra={"CYS_GUARD_SILENCE_N": "2"})
        chk(rc == 0, "R-02(c): PASS spec 가 exit 0 아님: %s" % rc)
        chk(not os.path.isfile(os.path.join(tasks, ".guard-silence.7.json")),
            "R-02(c): 실제 verify 후에도 침묵 스트릭 잔존(절단 실패)")

        # ══ E2-2(BLOCKER R-07) — verifier tax 활성화 절차 실재 ═════════════════
        root, tasks = new_root("r-budget-init")
        bdir = os.path.join(root, "_round", "verify-budget")
        rc, out, _e = run_cli(root, ["budget-status", "--json"])
        chk(rc == 0 and json.loads(out).get("active") is False,
            "E2-2: 초기 상태가 비활성이 아님: rc=%s out=%s" % (rc, out[:200]))
        chk(not os.path.isdir(bdir), "E2-2: budget-status 가 디렉터리를 생성(읽기전용 위반)")
        rc, out, _e = run_cli(root, ["budget-init", "--max-runs", "7", "--max-secs", "11"])
        chk(rc == 0 and os.path.isfile(os.path.join(bdir, "policy.json")),
            "E2-2: budget-init 실패: rc=%s out=%s" % (rc, out[:200]))
        rc, out, _e = run_cli(root, ["budget-status", "--json"])
        st = json.loads(out)
        chk(st.get("active") is True and st.get("max_runs") == 7 and st.get("max_secs") == 11,
            "E2-2: 활성 상태 판독 오류: %r" % st)
        rc, out2, _e = run_cli(root, ["budget-init"])   # 멱등 — 덮어쓰지 않는다
        chk(rc == 0 and "이미 활성" in out2, "E2-2: budget-init 멱등 실패: %s" % out2[:200])
        st2 = json.loads(run_cli(root, ["budget-status", "--json"])[1])
        chk(st2.get("max_runs") == 7, "E2-2: 멱등 호출이 정책을 덮어씀: %r" % st2)
        rc, _o, _e = run_cli(root, ["budget-init", "--max-runs", "0", "--force"])
        chk(rc == 2, "E2-2: 0 한도(상시 소진 footgun) 미거부: %s" % rc)
        import shutil as _sh                      # 되돌리기 = 디렉터리 삭제 1개
        _sh.rmtree(bdir)
        chk(json.loads(run_cli(root, ["budget-status", "--json"])[1]).get("active") is False,
            "E2-2: 디렉터리 삭제 후에도 활성으로 판독(되돌리기 불성립)")

        # ── 사다리 1단: CYCLE 마커 → SKIPPED_CYCLE(verify 미실행) / 만료 마커 → 진행 ──
        root, tasks = new_root("r-cycle")
        probe_out = os.path.join(root, "ran.txt")
        spec_touch = {"mode": "command", "cmd": "touch %s; exit 0" % probe_out,
                      "pass_rule": {"kind": "exit_map", "pass_exits": [0]}, "timeout_s": 5}
        mk_task(tasks, "T1", spec_touch)
        mk_claim(tasks, "7", "T1")
        cyc = os.path.join(tasks, "CYCLE_IN_PROGRESS.7")
        with open(cyc, "w") as f:
            f.write("cycling\n")
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and not os.path.isfile(probe_out),
            "CYCLE 마커인데 verify 실행/블록: rc=%s ran=%s" % (rc, os.path.isfile(probe_out)))
        chk(gstate(root, "T1")["last_verdict"] == "SKIPPED_CYCLE", "SKIPPED_CYCLE 미기록")
        os.utime(cyc, (time.time() - CYCLE_TTL_SEC - 60,) * 2)  # TTL 만료 → 무시+진행
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and os.path.isfile(probe_out),
            "만료 CYCLE 마커가 무시되지 않음: rc=%s ran=%s" % (rc, os.path.isfile(probe_out)))
        chk(gstate(root, "T1")["last_verdict"] == "PASS", "만료 마커 후 PASS 미기록")

        # ── 사다리 2단: 부트 grace — 마커 부재/신선 → SKIPPED_GRACE ──
        root, tasks = new_root("r-grace")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1")
        rc, _o, _e = run_guard(root, env_extra={"CYS_GUARD_BOOT_MARKER":
                                                os.path.join(root, "absent.marker")})
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "SKIPPED_GRACE",
            "부트 마커 부재가 SKIPPED_GRACE 아님")
        fresh_marker = os.path.join(root, "fresh.marker")
        with open(fresh_marker, "w") as f:
            f.write("x")
        rc, _o, _e = run_guard(root, env_extra={"CYS_GUARD_BOOT_MARKER": fresh_marker})
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "SKIPPED_GRACE",
            "신선 부트 마커(grace 내)가 SKIPPED_GRACE 아님")

        # ── 기준 ④a: ctx 60% 픽스처 → 블록 아닌 통과+context_yield+escalation(emit 1회) ──
        root, tasks = new_root("r-ctx60")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1")
        write_status(60.0)
        rc, _o, _e = run_guard(root)
        st = gstate(root, "T1")
        chk(rc == 0 and st["last_verdict"] == "SKIPPED_CONTEXT" and st["context_yield"] is True,
            "ctx60 이 SKIPPED_CONTEXT+context_yield 아님: rc=%s st=%s" % (rc, st))
        ev = [x for x in spool_lines(root) if x["type"] == "run.failed"
              and x["payload"].get("reason") == "context_yield"]
        chk(len(ev) == 1, "context_yield escalation emit 1회 아님: %d" % len(ev))
        rc, _o, _e = run_guard(root)  # 재양보 — emit 재발화 금지(1회)
        ev = [x for x in spool_lines(root) if x["type"] == "run.failed"
              and x["payload"].get("reason") == "context_yield"]
        chk(len(ev) == 1, "context_yield emit 이 재발화됨: %d" % len(ev))

        # ── 기준 ④b: ctx 측정실패 픽스처 → NO_BLOCK_PASS(verify 실행 기록은 남음) ──
        root, tasks = new_root("r-ctxfail")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1")
        write_status(20.0, source="transcript")  # source≠statusline → 측정 실패
        rc, _o, _e = run_guard(root)
        st = gstate(root, "T1")
        chk(rc == 0 and st["last_verdict"] == "NO_BLOCK_PASS" and st["block_count"] == 0,
            "ctx 측정실패가 NO_BLOCK_PASS(카운터 무증가) 아님: rc=%s st=%s" % (rc, st))
        chk(os.path.isfile(os.path.join(tasks, "T1.verify_out.log")),
            "NO_BLOCK_PASS 인데 verify 실행 기록(전문 로그) 없음")
        write_status(20.0)  # 픽스처 원복

        # ── 사다리 4단: resource hard/soft → SKIPPED_RESOURCE · 64 → INFRA ──
        root, tasks = new_root("r-gate")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1")
        rc, _o, _e = run_guard(root, env_extra={
            "CYS_GUARD_RESOURCE_ARGS":
                "--servers-override 99 --nodes-override 0 --load-override 0.0"})
        st = gstate(root, "T1")
        chk(rc == 0 and st["last_verdict"] == "SKIPPED_RESOURCE" and st["deferred"] is True,
            "gate hard 가 SKIPPED_RESOURCE+deferred 아님: rc=%s st=%s" % (rc, st))
        rc, _o, _e = run_guard(root, env_extra={
            "CYS_GUARD_RESOURCE_ARGS":
                "--servers-override 2 --nodes-override 0 --load-override 0.0"})
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "SKIPPED_RESOURCE",
            "gate soft 가 SKIPPED_RESOURCE 아님")
        rc, _o, _e = run_guard(root, env_extra={
            "CYS_GUARD_RESOURCE_ARGS": "--no-such-flag"})  # EX_USAGE 64 → INFRA(D20)
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "INFRA_ERROR",
            "gate exit 64 가 INFRA_ERROR 아님: %s" % gstate(root, "T1"))

        # ── 사다리 5단: budget — 부재=무제한·소진=SKIPPED_BUDGET(deferred) ──
        root, tasks = new_root("r-budget")
        mk_task(tasks, "T1", ok_spec)
        mk_claim(tasks, "7", "T1")
        bdir = os.path.join(root, "_round", "verify-budget")
        os.makedirs(bdir)
        with open(os.path.join(bdir, "policy.json"), "w") as f:
            json.dump({"max_runs": 2, "max_secs": 300}, f)
        with open(os.path.join(bdir, "999.json"), "w") as f:
            json.dump({"schema_version": 1, "window_start": _budget_window(),
                       "runs": 2, "secs": 1.0}, f)
        rc, _o, _e = run_guard(root)
        st = gstate(root, "T1")
        chk(rc == 0 and st["last_verdict"] == "SKIPPED_BUDGET" and st["deferred"] is True,
            "budget 소진이 SKIPPED_BUDGET+deferred 아님: %s" % st)
        # ── B2(조건 04②): 소진 = resource.hard 플릿 단일 발행(멱등 마커) — 워커별 발화 금지 ──
        hard_evs = [x for x in spool_lines(root) if x["type"] == "resource.hard"
                    and x["payload"].get("metric") == "verify_budget"]
        chk(len(hard_evs) == 1, "B2 budget 소진 resource.hard 정확 1회 아님: %d" % len(hard_evs))
        b2_markers = [n for n in os.listdir(bdir) if n.startswith(".hard-emitted-")]
        chk(len(b2_markers) == 1, "B2 멱등 마커(.hard-emitted-<창키>) 부재/중복: %s" % b2_markers)
        # 샤드 기록: budget 있는 상태에서 verify 실행되면 자기 샤드 적재
        with open(os.path.join(bdir, "999.json"), "w") as f:
            json.dump({"schema_version": 1, "window_start": _budget_window(),
                       "runs": 0, "secs": 0.0}, f)
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "PASS",
            "budget 잔여 시 verify 미실행: %s" % gstate(root, "T1"))
        shard, _err = _read_json(os.path.join(bdir, "7.json"))
        chk(shard is not None and shard.get("runs", 0) >= 1,
            "verify 실행이 자기 샤드에 미적재: %s" % shard)
        # ── B2 기준 ①: 샤드 병행(999·888·자기 7) 합산 경계 실측 + 반복 소진 창당 1회 ──
        with open(os.path.join(bdir, "policy.json"), "w") as f:
            json.dump({"max_runs": 4, "max_secs": 300}, f)
        for name, n_runs in (("999.json", 1), ("888.json", 1)):
            with open(os.path.join(bdir, name), "w") as f:
                json.dump({"schema_version": 1, "window_start": _budget_window(),
                           "runs": n_runs, "secs": 0.5}, f)
        rc, _o, _e = run_guard(root)  # 합산 3(999=1+888=1+자기=1) < 4 → 통과·자기 샤드 → 2
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "PASS",
            "B2 합산 3/4 미만인데 통과 아님: %s" % gstate(root, "T1"))
        rc, _o, _e = run_guard(root)  # 합산 4 ≥ 4 → 반복 소진(같은 창)
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "SKIPPED_BUDGET",
            "B2 합산 4/4 경계 소진 실패: %s" % gstate(root, "T1"))
        hard_evs = [x for x in spool_lines(root) if x["type"] == "resource.hard"
                    and x["payload"].get("metric") == "verify_budget"]
        chk(len(hard_evs) == 1,
            "B2 반복 소진에 resource.hard 재발화(창당 1회 위반): %d" % len(hard_evs))

        # ── B2: 예산 env 노브 — JAVIS_VERIFY_BUDGET_RUNS 가 policy.json 보다 우선 ──
        root, tasks = new_root("r-budget-env")
        mk_task(tasks, "T1", ok_spec)
        mk_claim(tasks, "7", "T1")
        bdir2 = os.path.join(root, "_round", "verify-budget")
        os.makedirs(bdir2)
        with open(os.path.join(bdir2, "policy.json"), "w") as f:
            json.dump({"max_runs": 99, "max_secs": 300}, f)
        rc, _o, _e = run_guard(root)  # policy 99 여유 → 통과(자기 샤드 1 적재)
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "PASS",
            "B2 env 노브 사전 통과 실패: %s" % gstate(root, "T1"))
        rc, _o, _e = run_guard(root, env_extra={"JAVIS_VERIFY_BUDGET_RUNS": "1"})
        st = gstate(root, "T1")
        chk(rc == 0 and st["last_verdict"] == "SKIPPED_BUDGET",
            "B2 env JAVIS_VERIFY_BUDGET_RUNS=1 이 policy(99) 를 우선하지 않음: %s" % st)
        env_hard = [x for x in spool_lines(root) if x["type"] == "resource.hard"
                    and x["payload"].get("metric") == "verify_budget"]
        chk(len(env_hard) == 1 and str(env_hard[0]["payload"].get("threshold")) == "1",
            "B2 env 한도 resource.hard threshold 오기록(emit 파서가 수치 문자열을 int 로 "
            "정규화 — str 대조): %s" % [x["payload"] for x in env_hard])

        # ── B2 기준 ⑦(조건 07②): timeout 위반 2회 → calibrated=false 강등 전이 +
        #    이후 strict checkout 거동(spec 존재 통과 + 강등 경고 표면화) ──
        root, tasks = new_root("r-demote")
        mk_task(tasks, "T1", {"mode": "command", "cmd": "sleep 30",
                              "pass_rule": {"kind": "exit_map", "pass_exits": [0]},
                              "timeout_s": 0.5, "calibrated": True})
        mk_claim(tasks, "7", "T1")
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and gstate(root, "T1")["timeout_violations"] == 1,
            "B2 demote 1회차 위반 적재 실패: %s" % gstate(root, "T1"))
        rec1, _err = _read_json(os.path.join(tasks, "T1.json"))
        chk(rec1["verify_spec"]["calibrated"] is True,
            "B2 위반 1회에 조기 강등: %s" % rec1["verify_spec"])
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and gstate(root, "T1")["timeout_violations"] == 2,
            "B2 demote 2회차 위반 적재 실패: %s" % gstate(root, "T1"))
        rec2, _err = _read_json(os.path.join(tasks, "T1.json"))
        chk(rec2["verify_spec"]["calibrated"] is False,
            "B2 위반 2회 후 calibrated=false 전이 실패: %s" % rec2.get("verify_spec"))
        env_dc = dict(os.environ)
        for k in ("CYS_SURFACE_ID", "AITERM_SURFACE_ID"):
            env_dc.pop(k, None)
        env_dc.update({"JAVIS_ROOT": root, "CYS_NO_AUTOSTART": "1",
                       "JAVIS_VERIFY_GATE": "strict", "JAVIS_TASK_LIVENESS": "alive",
                       "PATH": shim_dir + os.pathsep + os.environ.get("PATH", "")})
        r_dc = subprocess.run([sys.executable, os.path.join(_SELF_DIR, "javis_task.py"),
                               "checkout", "T1", "--owner", "w-b2"],
                              capture_output=True, text=True, env=env_dc, timeout=60)
        chk(r_dc.returncode == 0 and "calibrated 강등" in r_dc.stderr,
            "B2 강등 후 strict checkout 거동(통과+경고) 실패: rc=%s err=%s"
            % (r_dc.returncode, r_dc.stderr[:300]))

        # ── G5(수리): 데드라인 경로 demote 스폰 생략 — 카운트만 적재·다음 정상 Stop 소비 ──
        root, tasks = new_root("r-g5-deadline")
        mk_task(tasks, "T1", {"mode": "command", "cmd": "exit 0",
                              "pass_rule": {"kind": "exit_map", "pass_exits": [0]},
                              "timeout_s": 5, "calibrated": True})
        mk_claim(tasks, "7", "T1")
        with open(os.path.join(tasks, "T1.guard.json"), "w", encoding="utf-8") as f:
            json.dump(dict(_fresh_state("T1"), timeout_violations=1), f)
        t0 = time.time()
        rc, _o, _e = run_guard(root, env_extra={"CYS_GUARD_SELF_DEADLINE_SEC": "2",
                                                "CYS_SHIM_STATUS_HANG": "1"})
        g5_dt = time.time() - t0
        st = gstate(root, "T1")
        chk(rc == 0 and st["last_verdict"] == "INFRA_ERROR"
            and st["timeout_violations"] == 2,
            "G5 데드라인 INFRA 적재(violations 1→2) 실패: rc=%s st=%s" % (rc, st))
        rec_g5, _err = _read_json(os.path.join(tasks, "T1.json"))
        chk(rec_g5["verify_spec"]["calibrated"] is True,
            "G5 데드라인 경로가 demote 스폰(생략 위반): %s" % rec_g5.get("verify_spec"))
        chk(g5_dt < 8, "G5 데드라인 경로 소요 시간 마진 붕괴(스폰 잠식 의심): %.1fs" % g5_dt)
        rc, _o, _e = run_guard(root)  # 다음 정상 Stop — 무조건 소비 지점이 잔여분 처리
        rec_g5, _err = _read_json(os.path.join(tasks, "T1.json"))
        chk(rc == 0 and rec_g5["verify_spec"]["calibrated"] is False
            and rec_g5["verify_spec"].get("demoted_at"),
            "G5 다음 정상 Stop 의 강등 소비 실패: %s" % rec_g5.get("verify_spec"))

        # ── G6(수리): 무장 조합 위험 경보 — cap 미설정 ∧ grill 활성 ∧ self-cap 0 = 1회 ──
        root, tasks = new_root("r-g6-combo")
        mk_task(tasks, "T1", ok_spec)
        mk_claim(tasks, "7", "T1")
        g6_marker = os.path.join(root, "grill_session.json")
        with open(g6_marker, "w", encoding="utf-8") as f:
            json.dump({"session_id": "g6", "floor": 20, "status": "collecting",
                       "started_at": time.time(), "ttl_secs": 3600, "evidence": []}, f)
        rc, _o, _e = run_guard(root, env_extra={"GRILL_MARKER": g6_marker})
        rc, _o, _e = run_guard(root, env_extra={"GRILL_MARKER": g6_marker})
        g6_evs = [x for x in spool_lines(root) if x["type"] == "agent.error"
                  and "선점 산술 위험" in x["payload"].get("summary", "")]
        chk(len(g6_evs) == 1,
            "G6 무장 조합 위험 경보 1회(반복 무재발화) 아님: %d" % len(g6_evs))
        # self-cap 활성(≥1)·cap 상향은 각각 조합 해제 → 무경보
        root, tasks = new_root("r-g6-off")
        mk_task(tasks, "T1", ok_spec)
        mk_claim(tasks, "7", "T1")
        g6_marker2 = os.path.join(root, "grill_session.json")
        with open(g6_marker2, "w", encoding="utf-8") as f:
            json.dump({"session_id": "g6b", "floor": 20, "status": "collecting",
                       "started_at": time.time(), "ttl_secs": 3600, "evidence": []}, f)
        rc, _o, _e = run_guard(root, env_extra={"GRILL_MARKER": g6_marker2,
                                                "GRILL_STOP_SELF_CAP": "2"})
        rc, _o, _e = run_guard(root, env_extra={"GRILL_MARKER": g6_marker2,
                                                "CLAUDE_CODE_STOP_HOOK_BLOCK_CAP": "12"})
        g6_evs = [x for x in spool_lines(root) if x["type"] == "agent.error"
                  and "선점 산술 위험" in x["payload"].get("summary", "")]
        chk(len(g6_evs) == 0,
            "G6 조합 해제(self-cap on/cap 상향)인데 경보 발화: %d" % len(g6_evs))

        # ── 사다리 6단: spec 부재 → SKIPPED_NO_SPEC(fail-open) ──
        root, tasks = new_root("r-nospec")
        mk_task(tasks, "T1", None)
        mk_claim(tasks, "7", "T1")
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "SKIPPED_NO_SPEC",
            "spec 부재가 SKIPPED_NO_SPEC 아님")

        # ── ①(§2-4·기준 ①): 동일 실패 100회 → pending 정확 1건+escalation 1회+bundle 1개 ──
        root, tasks = new_root("r-esc100")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1")
        block_rcs = []
        first_err = ""
        for i in range(100):
            stdin_obj = {"stop_hook_active": i > 0}  # 같은 블록 턴 스트릭 시뮬(오버라이드 미발생)
            rc, _o, _e = run_guard(root, stdin_obj=stdin_obj)
            if i == 0:
                first_err = _e
            block_rcs.append(rc)
        chk(block_rcs[:3] == [2, 2, 2], "선행 3회가 블록(exit 2) 아님: %s" % block_rcs[:5])
        # 블록 stderr 기계 접두(조건 19②) — 첫 블록의 원문으로 검증
        chk("[completion-guard] task=T1 verdict=VERIFY_FAIL attempt=1/3" in first_err
            and "인프라 오류가 아님" in first_err,
            "블록 stderr 기계 접두 누락: %s" % first_err[:200])
        chk(all(r == 0 for r in block_rcs[3:]),
            "사후-escalation 이 전부 통과(exit 0) 아님: %s" % sorted(set(block_rcs[3:])))
        st = gstate(root, "T1")
        chk(st["block_count"] == 100 and st["escalated_at"], "카운터/escalated_at 오기록: %s" % st)
        pend_dir = os.path.join(root, "_round", "wakeups", "pending")
        pend = [n for n in os.listdir(pend_dir) if n.endswith(".json")] \
            if os.path.isdir(pend_dir) else []
        chk(len(pend) == 1, "동일 실패 100회 후 pending ≠ 1건(D17 drain 전 계수): %s" % pend)
        with open(os.path.join(root, "_round", "wakeups", "queue.jsonl"),
                  encoding="utf-8") as f:
            ledger = [json.loads(ln) for ln in f.read().splitlines() if ln.strip()]
        queued = [x for x in ledger if x.get("event") == "queued"
                  and x.get("task_key") == "guard-esc-T1"]
        chk(len(queued) == 1, "escalation enqueue(queued) 1회 아님: %d" % len(queued))
        chk(os.path.isfile(os.path.join(tasks, "T1.esc-bundle.json")), "esc-bundle 부재")
        bundle, _err = _read_json(os.path.join(tasks, "T1.esc-bundle.json"))
        chk(bundle and bundle.get("request_id", "").startswith("guard:T1:"),
            "esc-bundle request_id(멱등키 조인) 오기록: %s" % bundle)
        esc_ev = [x for x in spool_lines(root) if x["type"] == "run.failed"
                  and x["payload"].get("reason") == "verify_fail_escalation"]
        chk(len(esc_ev) == 1, "escalation emit 1회 아님: %d" % len(esc_ev))
        with open(shim_log, encoding="utf-8") as f:
            sends = [ln for ln in f.read().splitlines() if ln.startswith("send")]
        chk(len(sends) >= 1, "same-run drain 배달 시도 흔적 없음")
        # F2 코얼레싱: 100회 스트릭 중 block_count 가 기본 cap 8 을 교차(9회째)한 뒤에도
        # override tamper 이벤트는 에피소드당 정확 1회(약 92회 반복 Stop 재발화 0).
        ov100 = [x for x in spool_lines(root) if x["type"] == "agent.error"
                 and "오버라이드 의심" in x["payload"].get("summary", "")]
        chk(len(ov100) == 1,
            "100회 스트릭 override 이벤트 1회 아님(에피소드 코얼레싱 실패): %d" % len(ov100))

        # ── PASS 리셋 → 재임계 재escalation 허용(조건 01①) ──
        marker = os.path.join(root, "fixnow")
        flaky = {"mode": "command",
                 "cmd": "test -f %s && exit 0 || exit 7" % marker,
                 "pass_rule": {"kind": "exit_map", "pass_exits": [0]}, "timeout_s": 5}
        mk_task(tasks, "T1", flaky)  # spec 교체(레코드 덮어쓰기)
        with open(marker, "w") as f:
            f.write("x")
        rc, _o, _e = run_guard(root, stdin_obj={"stop_hook_active": True})
        st = gstate(root, "T1")
        chk(rc == 0 and st["last_verdict"] == "PASS" and st["block_count"] == 0
            and st["escalated_at"] is None,
            "PASS 가 카운터·escalated_at 리셋 안 함: %s" % st)

        # ── PASS 리셋 후 재실패 = 재블록 허용(조건 01① — 사후-escalation 스트릭 종료) ──
        os.remove(marker)
        rc, _o, _e = run_guard(root, stdin_obj={"stop_hook_active": False})
        chk(rc == 2, "PASS 리셋 후 재실패가 재블록 아님: %s" % rc)

        # ── ⑤(F2 재설계·§2-4): 오버라이드 — 블록 3회→조건 조성(cap 4)→새 Stop 5회 →
        #    tamper 정확 1회·pending 무증식·신규 escalation 0 ──
        root, tasks = new_root("r-override")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1")
        cap_env = {"CLAUDE_CODE_STOP_HOOK_BLOCK_CAP": "4"}
        for i in range(3):  # 블록 3회(3회째 escalation 전환 포함)
            rc, _o, _e = run_guard(root, stdin_obj={"stop_hook_active": i > 0},
                                   env_extra=cap_env)
            chk(rc == 2, "override 준비 블록(%d회째)이 exit 2 아님: %s" % (i + 1, rc))
        # 조건 조성: 4회째(사후-escalation 통과)로 block_count=4 = cap 도달
        rc, _o, _e = run_guard(root, stdin_obj={"stop_hook_active": True}, env_extra=cap_env)
        chk(rc == 0, "사후-escalation 통과가 exit 0 아님: %s" % rc)
        ov_pend_dir = os.path.join(root, "_round", "wakeups", "pending")
        pend_before = len([n for n in os.listdir(ov_pend_dir) if n.endswith(".json")])
        for i in range(5):  # 새 Stop 5회 — stop_hook_active 혼합(보조 힌트 무관 증명)
            rc, _o, _e = run_guard(root, stdin_obj={"stop_hook_active": i % 2 == 0},
                                   env_extra=cap_env)
            chk(rc == 0, "override 구간 새 Stop(%d회째)이 exit 0 아님: %s" % (i + 1, rc))
        ov = [x for x in spool_lines(root) if x["type"] == "agent.error"
              and "오버라이드 의심" in x["payload"].get("summary", "")]
        chk(len(ov) == 1, "override tamper 이벤트 정확 1회 아님: %d" % len(ov))
        pend_after = len([n for n in os.listdir(ov_pend_dir) if n.endswith(".json")])
        chk(pend_after == pend_before, "override 반복 Stop 에 pending 증식: %d→%d"
            % (pend_before, pend_after))
        with open(os.path.join(root, "_round", "wakeups", "queue.jsonl"),
                  encoding="utf-8") as f:
            ov_ledger = [json.loads(ln) for ln in f.read().splitlines() if ln.strip()]
        ov_queued = [x for x in ov_ledger if x.get("event") == "queued"
                     and x.get("task_key") == "guard-esc-T1"]
        chk(len(ov_queued) == 1, "override 감지가 신규 escalation(queued) 생성: %d"
            % len(ov_queued))

        # ── F2 후단: CYCLE 마커(인가 clear) 존재 시 감지 0 — 발화 위치가 양보 판정 이후 ──
        root, tasks = new_root("r-override-cycle")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1")
        with open(os.path.join(tasks, "T1.guard.json"), "w", encoding="utf-8") as f:
            json.dump(dict(_fresh_state("T1"), block_count=10,
                           last_verdict="VERIFY_FAIL", last_sig="s1"), f)
        with open(os.path.join(tasks, "CYCLE_IN_PROGRESS.7"), "w") as f:
            f.write("cycling\n")
        rc, _o, _e = run_guard(root, env_extra=cap_env)
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "SKIPPED_CYCLE",
            "CYCLE 마커 우선 양보 실패: rc=%s %s" % (rc, gstate(root, "T1")))
        ov2 = [x for x in spool_lines(root) if x["type"] == "agent.error"
               and "오버라이드 의심" in x["payload"].get("summary", "")]
        chk(len(ov2) == 0, "CYCLE 마커(인가 clear) 존재인데 override 감지: %d" % len(ov2))

        # ── ②(§2-4·기준 ②): 인프라 오류 3종 주입 → 블록 0·경보 3·corrupt 격리 ──
        root, tasks = new_root("r-infra3")
        infra_alerts = 0
        # (a) 명령부재 rc127
        mk_task(tasks, "Ta", {"mode": "command", "cmd": "definitely-not-a-command-xyz",
                              "pass_rule": {"kind": "exit_map", "pass_exits": [0]},
                              "timeout_s": 5})
        mk_claim(tasks, "7", "Ta")
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and gstate(root, "Ta")["last_verdict"] == "INFRA_ERROR",
            "rc127 이 INFRA fail-open 아님: rc=%s %s" % (rc, gstate(root, "Ta")))
        # (b) 타임아웃
        mk_task(tasks, "Tb", {"mode": "command", "cmd": "sleep 30",
                              "pass_rule": {"kind": "exit_map", "pass_exits": [0]},
                              "timeout_s": 0.5})
        mk_claim(tasks, "7", "Tb")
        rc, _o, _e = run_guard(root)
        stb = gstate(root, "Tb")
        chk(rc == 0 and stb["last_verdict"] == "INFRA_ERROR" and stb["timeout_violations"] == 1,
            "타임아웃이 INFRA+violations 적재 아님: rc=%s %s" % (rc, stb))
        # (c) guard.json 손상
        mk_task(tasks, "Tc", ok_spec)
        mk_claim(tasks, "7", "Tc")
        with open(os.path.join(tasks, "Tc.guard.json"), "w") as f:
            f.write("{corrupted!!!")
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and gstate(root, "Tc")["last_verdict"] == "INFRA_ERROR",
            "손상 guard.json 이 INFRA fail-open 아님: rc=%s" % rc)
        corrupt = [n for n in os.listdir(tasks) if n.startswith("Tc.guard.json.corrupt-")]
        chk(len(corrupt) == 1, "corrupt 격리 파일 미생성: %s" % corrupt)
        infra_alerts = [x for x in spool_lines(root) if x["type"] == "run.failed"
                        and x["payload"].get("reason") == "guard_infra"]
        chk(len(infra_alerts) == 3, "인프라 경보 3회 아님: %d" % len(infra_alerts))

        # ── 기준 ③: 연속 인프라 3회 → guard-disarmed 마커+경보·이후 무발동 ──
        root, tasks = new_root("r-breaker")
        mk_task(tasks, "T1", {"mode": "command", "cmd": "definitely-not-a-command-xyz",
                              "pass_rule": {"kind": "exit_map", "pass_exits": [0]},
                              "timeout_s": 5})
        mk_claim(tasks, "7", "T1")
        for _i in range(3):
            rc, _o, _e = run_guard(root)
            chk(rc == 0, "인프라 경로가 exit 0 아님: %s" % rc)
        disarm = os.path.join(tasks, "T1.guard-disarmed")
        chk(os.path.isfile(disarm), "연속 3회 인프라 후 guard-disarmed 마커 부재")
        chk(gstate(root, "T1")["infra_count"] == 3 and gstate(root, "T1")["disarm_reason"],
            "breaker 상태 오기록: %s" % gstate(root, "T1"))
        dis_ev = [x for x in spool_lines(root) if x["type"] == "agent.error"
                  and "guard-disarmed" in x["payload"].get("summary", "")]
        chk(len(dis_ev) == 1, "disarm 경보 1회 아님: %d" % len(dis_ev))
        st_before = gstate(root, "T1")
        rc, _o, _e = run_guard(root)  # 이후 무발동
        chk(rc == 0 and gstate(root, "T1") == st_before,
            "disarm 후 재발동(상태 변이): %s" % gstate(root, "T1"))

        # ── 기준 ⑤: 대형 출력(1MB) → 재주입 4KB 캡·전문 로그 존재 ──
        root, tasks = new_root("r-big")
        big = {"mode": "command",
               "cmd": "head -c 1048576 /dev/zero | tr '\\0' 'A'; exit 7",
               "pass_rule": {"kind": "exit_map", "pass_exits": [0]}, "timeout_s": 15}
        mk_task(tasks, "T1", big)
        mk_claim(tasks, "7", "T1")
        rc, _o, err = run_guard(root)
        chk(rc == 2, "대형 출력 실패가 블록 아님: %s" % rc)
        chk(len(err.encode("utf-8")) < HEAD_CAP + TAIL_CAP + 1024,
            "재주입 stderr 가 4KB 캡 초과: %d바이트" % len(err.encode("utf-8")))
        chk("바이트 절단" in err, "절단 표기 부재")
        vlog = os.path.join(tasks, "T1.verify_out.log")
        chk(os.path.isfile(vlog) and os.path.getsize(vlog) >= 1048576,
            "전문 로그 부재/불완전: %s" % (os.path.getsize(vlog) if os.path.isfile(vlog) else None))

        # ── 기준 ⑨: 프로세스 그룹 잔존 — sleep 자식 스폰 → killpg 잔존 0 ──
        root, tasks = new_root("r-pg")
        childpid_f = os.path.join(root, "childpid")
        # 배경 자식은 stdout 파이프를 상속하면 communicate() EOF 를 붙잡는다 — 리다이렉트 필수
        pg = {"mode": "command",
              "cmd": "sleep 300 >/dev/null 2>&1 & echo $! > %s; exit 5" % childpid_f,
              "pass_rule": {"kind": "exit_map", "pass_exits": [0]}, "timeout_s": 5}
        mk_task(tasks, "T1", pg)
        mk_claim(tasks, "7", "T1")
        rc, _o, _e = run_guard(root)
        chk(rc == 2, "그룹 테스트 verify 실패가 블록 아님: %s" % rc)
        with open(childpid_f) as f:
            child = int(f.read().strip())
        dead = False
        for _i in range(20):
            try:
                os.kill(child, 0)
                time.sleep(0.1)
            except ProcessLookupError:
                dead = True
                break
        chk(dead, "killpg 후 sleep 자식(pid %d) 잔존" % child)
        tam2 = [x for x in spool_lines(root) if x["type"] == "agent.error"
                and "프로세스 그룹 잔존" in x["payload"].get("summary", "")]
        chk(len(tam2) == 1, "그룹 잔존 tamper-adjacent 이벤트 1회 아님: %d" % len(tam2))

        # ── F1a: 데드라인 예산 조기 INFRA — 장시간 spec+n_of_m 이 데드라인 초과 불가 ──
        root, tasks = new_root("r-dl-budget")
        childpids = os.path.join(root, "childpids")
        slow = {"mode": "command",
                "cmd": "sleep 60 >/dev/null 2>&1 & echo $! >> %s; sleep 1; exit 7" % childpids,
                "pass_rule": {"kind": "exit_map", "pass_exits": [0]}, "timeout_s": 3,
                "n_of_m": {"n": 1, "m": 5}}
        mk_task(tasks, "T1", slow)
        mk_claim(tasks, "7", "T1")
        t0 = time.time()
        rc, _o, _e = run_guard(root, env_extra={"CYS_GUARD_SELF_DEADLINE_SEC": "10"})
        dl_dt = time.time() - t0
        st = gstate(root, "T1")
        chk(rc == 0 and st["last_verdict"] == "INFRA_ERROR",
            "데드라인 예산 케이스가 INFRA fail-open 아님: rc=%s st=%s" % (rc, st))
        chk(dl_dt < 10, "n_of_m 누적이 데드라인(10s) 초과: %.1fs" % dl_dt)
        ev = [x for x in spool_lines(root) if x["type"] == "run.failed"
              and "데드라인 예산 부족" in x["payload"].get("summary", "")]
        chk(len(ev) == 1, "데드라인 예산 조기 INFRA emit 부재/중복: %d" % len(ev))
        time.sleep(0.3)
        with open(childpids, encoding="utf-8") as f:
            dl_pids = [int(x) for x in f.read().split() if x.strip().isdigit()]
        dl_alive = []
        for pid in dl_pids:
            try:
                os.kill(pid, 0)
                dl_alive.append(pid)
            except OSError:
                pass
        chk(dl_pids and dl_alive == [],
            "데드라인 예산 경로 고아 프로세스 잔존(ps 실측): pids=%s alive=%s"
            % (dl_pids, dl_alive))

        # ── F1b: SIGALRM _Deadline 발화(E2E) — status 무기 지연 → INFRA 적재·violations++ ──
        root, tasks = new_root("r-dl-alarm")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1")
        rc, _o, err = run_guard(root, env_extra={"CYS_GUARD_SELF_DEADLINE_SEC": "2",
                                                 "CYS_SHIM_STATUS_HANG": "1"})
        st = gstate(root, "T1")
        chk(rc == 0 and st["last_verdict"] == "INFRA_ERROR" and st["infra_count"] == 1
            and st["timeout_violations"] == 1,
            "_Deadline 발화가 INFRA 적재(infra++·violations++) 아님: rc=%s st=%s" % (rc, st))
        chk("자체 데드라인" in err, "데드라인 stderr 문면 부재: %s" % err[:200])
        ev = [x for x in spool_lines(root) if x["type"] == "run.failed"
              and x["payload"].get("reason") == "guard_infra"
              and "자체 데드라인" in x["payload"].get("summary", "")]
        chk(len(ev) == 1, "데드라인 INFRA emit 1회 아님: %d" % len(ev))

        # ── F1c(in-process): _on_deadline 단위 — 추적 그룹 killpg·esc-bundle 소실 창 보강 ──
        droot = os.path.join(td, "r-dl-unit")
        dtasks = os.path.join(droot, "_round", "tasks")
        os.makedirs(dtasks, exist_ok=True)
        g = globals()
        dl_saved = {"ROOT": g["ROOT"], "TASKS_DIR": g["TASKS_DIR"],
                    "hud": os.environ.get("HUD_STATE_DIR")}
        p_track = subprocess.Popen(["sleep", "60"], start_new_session=True,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            g["ROOT"], g["TASKS_DIR"] = droot, dtasks
            os.environ["HUD_STATE_DIR"] = os.path.join(droot, "hud")
            _ACTIVE_PGIDS.add(p_track.pid)
            st_u = _fresh_state("Tdl")
            st_u.update({"escalated_at": _now(), "last_sig": "sigX", "block_count": 3})
            _DEADLINE_CTX.update({"state": st_u, "sid": "7"})
            rc_u = _on_deadline("자체 데드라인 테스트(단위)")
            chk(rc_u == 0, "_on_deadline 이 exit 0 아님: %s" % rc_u)
            dl_dead = False
            for _i in range(25):
                if p_track.poll() is not None:
                    dl_dead = True
                    break
                time.sleep(0.1)
            chk(dl_dead, "_on_deadline killpg 후 추적 그룹(sleep pid %d) 잔존" % p_track.pid)
            chk(not _ACTIVE_PGIDS, "_on_deadline 후 _ACTIVE_PGIDS 미정리: %s" % _ACTIVE_PGIDS)
            gj, _e2 = _read_json(os.path.join(dtasks, "Tdl.guard.json"))
            chk(gj is not None and gj["last_verdict"] == "INFRA_ERROR"
                and gj["infra_count"] == 1 and gj["timeout_violations"] == 1,
                "_on_deadline INFRA 적재 오류: %s" % gj)
            bd, _e2 = _read_json(os.path.join(dtasks, "Tdl.esc-bundle.json"))
            chk(bd is not None and bd.get("request_id") == "guard:Tdl:sigX"
                and "backfill" in bd.get("note", ""),
                "escalated_at~_escalate 소실 창 esc-bundle 보강 부재/오기록: %s" % bd)
        finally:
            g["ROOT"], g["TASKS_DIR"] = dl_saved["ROOT"], dl_saved["TASKS_DIR"]
            if dl_saved["hud"] is None:
                os.environ.pop("HUD_STATE_DIR", None)
            else:
                os.environ["HUD_STATE_DIR"] = dl_saved["hud"]
            _DEADLINE_CTX.update({"state": None, "sid": None})
            with contextlib.suppress(Exception):
                p_track.kill()
            with contextlib.suppress(Exception):
                p_track.wait(timeout=5)

        # ── F4(unit): gate exit 1 판별 — warnings 판독(문서화된 4상) ──
        chk(_soft_kind(None, ["context_unmeasured"]) == "proceed_unmeasured",
            "F4: context_unmeasured 단독이 proceed_unmeasured 아님")
        chk(_soft_kind(None, ["measure_error:servers(ps),nodes(ps)", "context_unmeasured"])
            == "skip_soft", "F4: measure_error 포함이 skip_soft 아님")
        chk(_soft_kind(None, []) == "skip_soft", "F4: warnings 공집합이 skip_soft 아님")
        chk(_soft_kind(20.0, ["context_unmeasured"]) == "skip_soft",
            "F4: ctx 실측 상태의 context_unmeasured 가 proceed 오판")

        # ── F6: probe mode 3케이스(PASS/FAIL(블록)/INDET→INFRA) + CYS_PROBE_RUNS 격리 ──
        root, tasks = new_root("r-probe")
        art = os.path.join(root, "artifact.txt")
        with open(art, "w", encoding="utf-8") as f:
            f.write("payload\n")
        pack_default_runs = os.path.join(os.path.dirname(_SELF_DIR), "state",
                                         "probe_runs.jsonl")
        default_size0 = (os.path.getsize(pack_default_runs)
                         if os.path.isfile(pack_default_runs) else -1)
        mk_task(tasks, "Tp1", {"mode": "probe",
                               "probe": {"name": "artifact", "args": ["--path", art]},
                               "timeout_s": 10})
        mk_claim(tasks, "7", "Tp1")
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and gstate(root, "Tp1")["last_verdict"] == "PASS",
            "probe PASS(exit 0) 케이스 실패: rc=%s %s" % (rc, gstate(root, "Tp1")))
        mk_task(tasks, "Tp2", {"mode": "probe",
                               "probe": {"name": "artifact",
                                         "args": ["--path", os.path.join(root, "absent.bin")]},
                               "timeout_s": 10})
        mk_claim(tasks, "7", "Tp2")
        rc, _o, _e = run_guard(root)
        chk(rc == 2 and gstate(root, "Tp2")["last_verdict"] == "VERIFY_FAIL",
            "probe FAIL(exit 2→블록) 케이스 실패: rc=%s %s" % (rc, gstate(root, "Tp2")))
        mk_task(tasks, "Tp3", {"mode": "probe",
                               "probe": {"name": "artifact",
                                         "args": ["--path", art, "--since", "not-a-time"]},
                               "timeout_s": 10})
        mk_claim(tasks, "7", "Tp3")
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and gstate(root, "Tp3")["last_verdict"] == "INFRA_ERROR",
            "probe INDET(exit 3→INFRA·블록 금지) 케이스 실패: rc=%s %s"
            % (rc, gstate(root, "Tp3")))
        iso_runs = os.path.join(root, "probe_runs.jsonl")
        chk(os.path.isfile(iso_runs), "CYS_PROBE_RUNS 격리 원장 미생성(영수증 누락)")
        receipts = []
        if os.path.isfile(iso_runs):
            with open(iso_runs, encoding="utf-8") as f:
                receipts = [ln for ln in f.read().splitlines() if ln.strip()]
        chk(len(receipts) == 3, "격리 원장 영수증 3건 아님: %d" % len(receipts))
        default_size1 = (os.path.getsize(pack_default_runs)
                         if os.path.isfile(pack_default_runs) else -1)
        chk(default_size0 == default_size1,
            "probe 영수증이 팩 기본 원장으로 누출(CYS_PROBE_RUNS 격리 실패): %s→%s"
            % (default_size0, default_size1))

        # ── F3: guard-claim 이음매 E2E — checkout --claim-surface → guard 조인 ──
        root, tasks = new_root("r-claimjoin")
        jt = os.path.join(_SELF_DIR, "javis_task.py")
        env_jt = dict(os.environ)
        env_jt.pop("CYS_SURFACE_ID", None)
        env_jt.pop("AITERM_SURFACE_ID", None)
        env_jt.update({"JAVIS_ROOT": root, "CYS_NO_AUTOSTART": "1",
                       "PATH": shim_dir + os.pathsep + os.environ.get("PATH", "")})
        r1 = subprocess.run([sys.executable, jt, "create", "TF3", "--id", "TF3"],
                            capture_output=True, text=True, env=env_jt, timeout=60)
        chk(r1.returncode == 0, "F3 create 실패: %s" % r1.stderr[:200])
        r2 = subprocess.run([sys.executable, jt, "checkout", "TF3", "--owner", "master",
                             "--claim-surface", "7"],
                            capture_output=True, text=True, env=env_jt, timeout=60)
        chk(r2.returncode == 0, "F3 checkout --claim-surface 실패: rc=%s %s"
            % (r2.returncode, r2.stderr[:300]))
        f3_cl, _e3 = _read_json(os.path.join(tasks, ".guard-claim.7"))
        chk(f3_cl is not None and f3_cl.get("task_id") == "TF3",
            "F3 지정 surface claim 미기록/오기록: %s" % f3_cl)
        rc, _o, _e = run_guard(root)
        f3_gj = os.path.join(tasks, "TF3.guard.json")
        chk(rc == 0 and os.path.isfile(f3_gj),
            "F3 guard 조인 실패 — guard.json 미생성(무발동 이음매 잔존): rc=%s" % rc)
        if os.path.isfile(f3_gj):
            chk(gstate(root, "TF3")["last_verdict"] == "SKIPPED_NO_SPEC",
                "F3 guard 조인 판정 오기록: %s" % gstate(root, "TF3"))

        # ── ③(§2-4): 무조건 exit 2 경로 부재 — 역경 배터리 전부 exit 0 ──
        root, tasks = new_root("r-adverse")
        mk_task(tasks, "T1", ok_spec)
        mk_claim(tasks, "7", "T1")
        adverse = []
        r1, _o, _e = run_guard(root, stdin_obj=None)  # 정상 PASS
        adverse.append(("PASS", r1))
        # 빈 stdin
        env = dict(os.environ)
        env.update({"JAVIS_ROOT": root, "CYS_COMPLETION_GUARD": "1", "CYS_SURFACE_ID": "7",
                    "CYS_GUARD_BOOT_MARKER": boot_marker, "CYS_GUARD_LIVENESS": "alive",
                    "HUD_STATE_DIR": os.path.join(root, "hud"),
                    "CYS_PROBE_RUNS": os.path.join(root, "probe_runs.jsonl"),
                    "CYS_NO_AUTOSTART": "1", "CYS_ROLE": "worker",
                    "CYS_GUARD_RESOURCE_ARGS":
                        "--servers-override 0 --nodes-override 0 --load-override 0.0",
                    "PATH": shim_dir + os.pathsep + os.environ.get("PATH", "")})
        r2 = subprocess.run([sys.executable, self_path], input="not-json{{{",
                            capture_output=True, text=True, env=env, timeout=60).returncode
        adverse.append(("stdin 비JSON", r2))
        # 태스크 레코드 부재
        mk_claim(tasks, "7", "Tghost")
        r3, _o, _e = run_guard(root)
        adverse.append(("태스크 레코드 부재", r3))
        mk_claim(tasks, "7", "T1")
        # spec 미지 mode
        mk_task(tasks, "T1", {"mode": "vibes"})
        r4, _o, _e = run_guard(root)
        adverse.append(("spec 미지 mode", r4))
        mk_task(tasks, "T1", ok_spec)
        # claim 손상
        with open(os.path.join(tasks, ".guard-claim.7"), "w") as f:
            f.write("garbage")
        r5, _o, _e = run_guard(root)
        adverse.append(("claim 손상", r5))
        # F8: claim task_id allowlist 불일치 — traversal·예약 접미 = claim-corrupt 무발동
        with open(os.path.join(tasks, ".guard-claim.7"), "w", encoding="utf-8") as f:
            json.dump({"task_id": "../../tmp/evil", "pid": os.getpid(), "ts": _now()}, f)
        r6, _o, _e = run_guard(root)
        adverse.append(("F8 claim id traversal", r6))
        with open(os.path.join(tasks, ".guard-claim.7"), "w", encoding="utf-8") as f:
            json.dump({"task_id": "T1.guard", "pid": os.getpid(), "ts": _now()}, f)
        r7, _o, _e = run_guard(root)
        adverse.append(("F8 claim id 예약 접미", r7))
        chk(not os.path.isfile(os.path.join(tasks, "T1.guard.guard.json")),
            "F8: 예약 접미 claim id 가 사이드카 파생 파일을 생성")
        for name, r in adverse:
            chk(r == 0, "역경 케이스 %r 가 exit 0 아님(무조건 exit 2 경로?): %s" % (name, r))

        # ── N≥8 설정 거부(D2 천장) ──
        root, tasks = new_root("r-escn")
        mk_task(tasks, "T1", fail_spec)
        mk_claim(tasks, "7", "T1")
        for i in range(3):
            rc, _o, err = run_guard(root, stdin_obj={"stop_hook_active": i > 0},
                                    env_extra={"CYS_GUARD_ESC_N": "9"})
        st = gstate(root, "T1")
        chk(st["escalated_at"] is not None,
            "CYS_GUARD_ESC_N=9 거부(→3 강등) 실패 — 3회에 escalation 안 됨: %s" % st)

        # ── emit 필드셋 parity(§2-3) — guard 가 쓰는 payload 가 EVT 스키마 통과 ──
        import javis_event as _ev
        for typ, payload in (
                ("run.failed", {"agent": "worker", "task": "T1", "summary": "s",
                                "reason": "guard_infra"}),
                ("agent.error", {"agent": "worker", "summary": "s"}),
                ("resource.soft", {"metric": "servers", "value": "2", "threshold": "2"}),
                ("resource.hard", {"metric": "servers", "value": "99", "threshold": "3"})):
            ok, err_v = _ev.validate(typ, payload)
            chk(ok, "emit 필드셋 회귀 — %s 스키마 거부: %s" % (typ, err_v))

        # ── waiver mode = PASS(실행 0) · probe/procedural 경로 최소 확인 ──
        root, tasks = new_root("r-modes")
        mk_task(tasks, "T1", {"mode": "waiver", "waiver_ref": "WVR-x"})
        mk_claim(tasks, "7", "T1")
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "PASS",
            "waiver mode 가 PASS 아님: %s" % gstate(root, "T1"))
        report = os.path.join(root, "report.md")
        with open(report, "w", encoding="utf-8") as f:
            f.write("| 원문 | 판정 | 출처 |\n| --- | --- | --- |\n"
                    "| 주장 A | 통과 | https://example.com |\n")
        mk_task(tasks, "T1", {"mode": "procedural",
                              "procedural": {"tool": "factcheck", "report_path": report}})
        rc, _o, _e = run_guard(root)
        chk(rc == 0 and gstate(root, "T1")["last_verdict"] == "PASS",
            "procedural factcheck 통과 경로 실패: %s" % gstate(root, "T1"))

    fails.extend(_self_test_platform())   # E1-2/E1-3 — 플랫폼 분기·워치독(모킹 · 스폰 0)

    if fails:
        print("javis_completion_guard self-test FAIL %d건:" % len(fails), file=sys.stderr)
        for x in fails:
            print("  ✗ " + x, file=sys.stderr)
        return 1
    print("javis_completion_guard self-test OK — 무장OFF/claim부재·stale claim 경보1회·"
          "CYCLE 마커+TTL·부트 grace·ctx60 yield(emit 1회)·ctx 측정실패 NO_BLOCK_PASS·"
          "resource soft/hard/64→INFRA·budget 부재/소진/샤드 적재·"
          "B2 tax(resource.hard 단일발행+멱등 마커·반복 소진 창당 at-most-once·샤드 3벌 합산 "
          "경계 3/4→4/4·env 노브 policy 우선·threshold 기록)·"
          "B2 demote(timeout 2회→calibrated=false 전이·strict checkout 통과+강등 경고)·"
          "G5(데드라인 demote 스폰 생략·카운트 적재·다음 정상 Stop 소비·시간 마진)·"
          "G6(무장 조합 위험 경보 1회·조합 해제 무경보)·no-spec·"
          "동일실패 100회(pending 1·queued 1·bundle 1·emit 1·override 코얼레싱 1)·"
          "PASS 리셋 재블록·F2 오버라이드(카운터 기반 — 새 Stop 5회 tamper 1·pending 무증식·"
          "CYCLE 마커 감지 0)·인프라 3종(블록0·경보3·corrupt 격리)·breaker disarm·"
          "1MB 캡 재주입·killpg 잔존 0(tamper 1)·F1 데드라인(예산 조기 INFRA 고아 0·"
          "SIGALRM INFRA 적재·_on_deadline killpg+bundle 보강)·F4 warnings 판별 4상·"
          "F6 probe 3케이스+CYS_PROBE_RUNS 격리·F3 --claim-surface 조인·"
          "역경 배터리 exit2 부재(F8 claim id 방어 포함)·N≥8 거부·"
          "emit 필드셋 parity·waiver/procedural 경로·"
          "★E2-1 R-02(master pid 사망+태스크 활성→발동 exit2·pid 강등 경보 1회·pid 미기록 "
          "claim 발동·침묵 탐지기 임계 경보 1회+상태파일·임계 미달 무경보·verify 후 스트릭 절단)·"
          "★E2-2 R-07(budget-status 비활성 판독·읽기전용·budget-init 활성/멱등/0한도 거부/"
          "디렉터리 삭제 복귀)·"
          "★E1-2/E1-3 R-01·R-09(셸 argv 플랫폼 분기·COMSPEC 폴백·Windows 기본 무장 거부+opt-in·"
          "posix 무영향·nt taskkill 실호출+추적 정리·워치독 daemon 무장/해제·posix 백스톱 여유 "
          "8s·nt 워치독 유일 겹)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
