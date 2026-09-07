#!/usr/bin/env python3
"""javis_resource_gate.py — P0-3 자원 사전 게이트 (getInvocationBlock의 정액제 번안)

계약(출처: _research/Paperclip_박사급_연구보고서.md §4 P0-3 · §2-7):
- Paperclip의 진짜 런어웨이 차단 = "새 run 시작 전 라이브 재계산해 초과면 착수 거부"(사전 게이트).
- 승인 패턴은 **구독제(정액) 전용·종량제 과금 금지**다. 정액 구독엔 달러 예산이라는 브레이크가
  아예 없으므로(쓴 만큼 청구되는 축이 없다) metric을 달러가 아닌 자원으로 치환한다:
    servers  = 로컬 dev/서버 **논리** 개수(`cys ps` 원장 항목 · A3-b) — 원장 조회 실패 시에만
               ps 패턴 체인 루트 계수로 폴백(자원 거버넌스 '서버 누적' 사고 이력)
    nodes    = claude/agy/codex 노드 프로세스 수
    load     = 1분 load average / CPU 코어 수 비율   (★0.14.31~ **soft 전용** — 호스트 전체 부하에는
               우리가 회수할 수 없는 성분(예: macOS mediaanalysisd)이 섞이므로 착수 거부 근거가 아니다)
    fleet_cpu= ★0.14.31 신설 — **우리 프로세스들**(claude/agy/codex/gemini + cysd + serena)의
               `ps -axo pcpu,command` %CPU 합 / 100 / ncpu(분율). soft 0.5 · hard 1.0.
               차단은 '우리가 실제로 회수할 수 있는 자원'에만 건다. 계측 한계는 FLEET_CPU_PATTERNS 주석.
    context  = 자기보고 컨텍스트 %               (60% /clear 규칙)
- soft/hard 2단(Paperclip warnPercent 사상): soft=경고 후 진행 허용, hard=착수 거부.
- 판정은 결정론: exit code 0=allow · 1=soft warn · 2=hard block. (LLM 자연어 판단 제거)
- "저장값 재신뢰 금지, 매번 재계산" — 게이트는 항상 라이브 측정.

기본 임계(우리 자원 거버넌스 실사고 기준):
  servers  soft 2  / hard 3     (watchdog '3개+' 규칙과 정합 — 사후 kill 전에 사전 차단)
  nodes    soft 12 / hard 18(+동적: max(18, 12 + Σ활성 부서 좌석) — 활성 = 부서 데몬이
           `cys status --json --socket <sock>` 에 응답한 부서 · 좌석 = 그 응답의 비-exited surfaces 수 ·
           응답 실패 = measure_errors `dept(<이름>)` 로 soft 격상(계상 제외). 2026-07-06 CSO 위임
           오탐 수정(부서당 +5)을 2026-09-03 A3(SURVEY A4·B6-2 · PREP #8)가 좌석 합산으로 치환.
           --nodes-hard 명시 지정 시 그 값 그대로 우선)
  load     soft 1.0×ncpu / hard **없음**(soft 전용 · `--load-hard-ratio` 는 0.14.31 부터 무동작)
  fleet_cpu soft 0.5 / hard 1.0 (분율 · Windows 등 `ps` 부재 플랫폼은 checks 에 unavailable 라벨만
           남기고 measure_errors 에 넣지 않는다 = exit 계약 불변. ps 는 있는데 조회가 깨진 것은
           측정 실패라 measure_errors 로 간다)
  context  soft 50 / hard 60    (60% 도달 전 저장 후 /clear 규칙)
  ★부트 유예: 데몬 부트(`<state>/boot-epoch` mtime) 후 300초 안에는 **CPU 축(fleet_cpu·load)의 hard 만**
    soft 로 내린다(다른 축 hard 는 유지). 근거를 못 읽으면 유예 없음(=종전 판정).

★T9(P3-1·R3-P03-1) 곱셈 편성 예산 축(W6): check --formation-size <n> + env CYS_FORMATION_BUDGET.
  발화 조건은 **둘 다**다 — formation_size is not None ∧ CYS_FORMATION_BUDGET 정수 파싱 성공.
  어느 한쪽 부재(또는 env 비정수)=완전 무동작(inert) — 기존 호출자 3곳(bootstrap ④′·
  completion_guard·formation)의 회귀 0. 발화 시 투영 = 측정 nodes + 활성 부서수×formation_size 를
  예산과 대조, **초과(>)면 hard(exit 2)**. nodes 미측정(ps 실패)이면 이 축은 무발화하고
  measure_errors 경로(최소 soft 격상)가 신호한다 — 예외로 터뜨리지 않는다(70 방지).
  종전에는 이 플래그가 없어서 formation 의 호출이 매번 exit 64(사용오류)로 접혔다(편성
  자원게이트 완전 사문 — R3-P03-1 실측). 이 리비전이 그 **플래그 스큐**의 수리다.
  ★수리됨 ≠ 발화함(R2 적대검증 · 2026-08-26 정본화): 위 수리로 일반 축(servers/nodes/load/
  context)은 정상 판정하게 됐지만, **곱셈 예산 축 자체는 프로덕션에서 무발화**다 —
  `CYS_FORMATION_BUDGET` 을 세우는 생산자가 저장소에 하나도 없다(실측: 정의처·self-test·
  formation 호출부뿐). 즉 이 축은 **운영자 opt-in**이며(env 를 세우기 전까지 항상 proceed),
  그것이 의도된 현 상태다. 따라서 상비편성 폭주 방어의 **실제 담당자는 `javis_formation.py`
  의 `_attempts_carry` 시도 원장 하나**이고, 곱셈 투영은 그 위에 얹는 선택 층이다.
  켜는 법: `CYS_FORMATION_BUDGET=<정수> ... check --formation-size <n>`.

테스트/자동화 주입: --servers-override/--nodes-override/--load-override/--dept-roster-override
  (라이브 측정 대체 · 마지막은 부서 로스터 JSON {active,seats,errors,depts} — 잘못된 JSON=EX_USAGE 64).
사용 예: python3 javis_resource_gate.py check --context 42 --json
exit codes(A13 타입드 — 코드 상수와 기계 대조):
  0  EXIT_ALLOW     허용
  1  EXIT_SOFT      soft_warn(경고 후 진행)
  2  EXIT_HARD      hard_block(착수 거부) — **자원 판정**에만 쓰인다
  64 EXIT_USAGE     EX_USAGE — 미지 서브커맨드·인자 오류(측정 자체가 일어나지 않음).
                    종전 argparse 기본 exit 2 가 EXIT_HARD 와 충돌해, 오타 하나가 '팀 기동 거부'로
                    오독됐다(재감사 A13 치환 결함). 소비부는 이 코드를 '측정 실패'로 loud 처리한다.
  70 EXIT_INTERNAL  EX_SOFTWARE — 게이트 내부 예외(측정 실패 ≠ soft_warn. 종전 exit 1 오분류).
자체검증: python3 javis_resource_gate.py --self-test
"""
import argparse
import errno
import glob
import json
import math
import os
import re
import socket
import subprocess
import sys
import time

EXIT_ALLOW, EXIT_SOFT, EXIT_HARD = 0, 1, 2
# ★A13(T-0147-7 W2 · 하드 제약 8) — argparse 의 exit 2 ↔ EXIT_HARD=2 **의미 공간 충돌** 해소.
#   종전에는 `check --unknown-flag` 같은 사용오류가 argparse 기본 동작으로 exit 2 를 냈고,
#   소비부(javis_bootstrap ④′)는 그 2를 '자원 hard_block'으로 읽어 **아무 측정도 없이 팀 기동을
#   거부**했다(판정과 사용오류의 융합 — RC2). 사용오류는 sysexits.h 의 EX_USAGE(64)로 분리한다.
#   ★내부 예외(게이트가 재다가 터진 경우)도 exit 1='soft' 로 오분류됐다 — EXIT_INTERNAL 로 분리한다.
EXIT_USAGE = 64          # EX_USAGE — 미지 서브커맨드·인자 오류(측정 자체가 일어나지 않음)
EXIT_INTERNAL = 70       # EX_SOFTWARE — 게이트 내부 예외(측정 실패 ≠ soft_warn)

SERVER_PATTERNS = [
    r"bun .*server", r"node .*server", r"vite(\s|$)", r"next dev", r"uvicorn",
    # ★G12 실측 교정(2026-07-04): macOS 프레임워크 파이썬은 ps에
    #   ".../Python.app/Contents/MacOS/Python -m http.server"로 표시 — 'python3? ' 접두는
    #   실서버를 영영 못 잡는다(분류 갭 실측). 경로·대소문자 내성으로 확장.
    r"(?i)python[^ ]* -m http\.server", r"(?i)python[^ ]* .*server\.py",
    r"webpack.*serve",
]
# 서버가 아닌 상주 인프라(오탐 제외): 언어 서버(LSP)·MCP 서버 등은 자원 거버넌스의
# 'dev 서버 누적' 대상이 아니다 (실측: pyright langserver.index.js가 node .*server에 걸림).
SERVER_EXCLUDE_PATTERNS = [
    r"langserver", r"language[-_ ]?server", r"\blsp\b", r"mcp[-_ ]?server",
    r"tsserver", r"copilot",
]
NODE_PATTERNS = [r"claude(\s|$)", r"\bagy\b", r"\bcodex\b", r"\bgemini\b"]
# ★2026-07-11 CSO(CEO B승인): codex 노드 1개 = node wrapper + darwin-arm64 vendor native 2프로세스가
# 둘 다 \bcodex\b 매칭 → 이중계수. vendor native를 제외해 codex는 wrapper 1개만 계수(계수 인플레이션 차단).
NODE_EXCLUDE_PATTERNS = [r"codex-darwin-arm64"]

# ★WP-7 N(0.14.31 · 버그리포트 B2 "편성 stuck") — **함대 소유 CPU 축**.
#   실측 2026-09-06 07:07: macOS `mediaanalysisd` 가 79% 를 태워 load1=59 → `load_ratio` hard →
#   편성이 hard_block 으로 12분 잠겼고, 07:19 재측정에서 저절로 soft 로 풀렸다. 즉 게이트가
#   **우리 소관이 아닌 호스트 부하**로 조직 기동을 거부했다. 그래서 사실을 둘로 가른다:
#     load_ratio      = 호스트 전체 1분 부하/코어 — 우리가 못 고치는 성분이 섞인다 → **soft 전용**
#     fleet_cpu_ratio = 우리 프로세스들의 %CPU 합 / 100 / ncpu(**분율**) → soft 0.5 · hard 1.0
#   "막는 쪽으로만 틀린다"는 여기서 **차단을 우리가 실제로 회수할 수 있는 자원에만 건다**는 뜻이다.
#
#   ★계측 한계(라벨을 실제보다 강하게 쓰지 않는다 · codex R1 #2 정정): `ps` 의 %CPU 는 어느
#     플랫폼에서도 **순간 점유율이 아니고, 두 플랫폼의 의미가 서로 다르다**.
#       Linux(procps `ps.1`)  : 프로세스 **수명 전체** 평균(누적 CPU시간 ÷ 경과시간) — 과거가 계속 남는다.
#       macOS(adv_cmds `ps.1`): 커널 `cpu_usage` 기반 **최근 약 1분의 감쇠 평균** — 과거가 빠르게 잊힌다.
#     그래서 이 값은 '지금 함대가 먹는 CPU'의 실측이 아니라 **패턴에 걸린 프로세스들의 OS별 평균
#     CPU 추정치**다. 방금 시작된 폭주는 (특히 Linux 에서) 늦게 드러나고, Linux 에서는 한때 바빴다가
#     지금 노는 장수 프로세스가 계속 높게 남는다.
#     그 성질 때문에 (a) hard 를 1.0(= 전 코어 100% 상당)이라는 높은 자리에 두고 (b) 부트 직후
#     창(BOOT_GRACE_SECS)에는 hard 를 soft 로 내린다.
#   ★잔여 노출(정직 고지 · 봉인표 ③): Linux 의 수명 평균은 **끈끈하다** — 이 축이 hard 인 동안
#     편성(javis_formation ensure ④)은 pending-resource 로 접히고, 그 상태를 스스로 푸는 장치는
#     이 축에 없다(부하가 내려가거나 그 프로세스가 죽어야 풀린다). 다만 **죽은 좌석은 CPU 를 먹지
#     않는다** — 좌석 전멸(치명 ③) 형상에서 그 부서의 몫은 0 이므로, 이 축이 그 복구를 막으려면
#     '다른 살아 있는 함대 프로세스가 전 코어를 계속 태우고 있다'는 사실이 참이어야 한다. 그때
#     스폰을 더 하지 않는 것이 이 축의 목적이다. 회수 대상을 사람이 곧장 짚도록 hard 시 상위
#     기여자를 `fleet_cpu_top` 으로 남긴다(원인 미상의 영구 보류를 만들지 않는다).
FLEET_CPU_PATTERNS = NODE_PATTERNS + [r"(^|/)cysd(\s|$)", r"\bserena\b"]
# ★상속되는 계측 한계(codex R2 · 위임 검체 1~3 — 숨기지 않는다): 이 패턴들은 기존 `nodes` 축의
#   `NODE_PATTERNS` 를 그대로 쓰고 매칭도 `_count_matching` 과 같은 '명령줄 전체 검색'이다. 그래서
#   `python3 /tmp/agy/report.py` · `grep claude` 처럼 **에이전트가 아닌 프로세스**도 걸린다(과대계상 =
#   차단 방향). 자기 제외(`"javis_resource_gate" in cmd`)도 같은 이유로 출력 파일명에 이 이름을 넣은
#   진짜 함대 프로세스를 뺄 수 있다(과소계상). 둘 다 **기존 nodes 축과 동일한 성질**이라 여기서만
#   고치면 두 축이 다른 모집단을 세게 된다 — 패턴 정밀화는 두 축을 함께 다루는 별 백로그다.
# ★여기에 NODE_EXCLUDE_PATTERNS 를 걸지 않는다: 그 제외(codex vendor native)는 **계수 인플레이션**
#   (codex 1노드 = wrapper + native 2프로세스)의 교정이다. CPU **합**에서는 vendor native 가 codex 가
#   실제로 태운 CPU 의 본체라, 빼면 함대 소비를 체계적으로 과소평가한다(축이 목적을 못 재는 방향).
FLEET_CPU_SOFT_DEFAULT = 0.5     # 함대가 전 코어의 50% 상당을 평균 점유 — 경고
FLEET_CPU_HARD_DEFAULT = 1.0     # 전 코어 100% 상당 — 착수 거부
FLEET_CPU_TOP_N = 3              # hard 시 남기는 상위 기여자 수(회수 대상 지목 — 봉인표 ③)
# `--load-hard-ratio` 의 종전 기본값. 이제 load 축은 soft 전용이라 이 플래그는 **무동작**이다 —
# 계약 호환(구 호출자의 EX_USAGE 64 회귀 방지)을 위해 받기만 하고, 기본값과 다르면 stderr 로 고지한다.
LOAD_HARD_RATIO_DEFAULT = 2.0
# ★부트 유예 — **CPU 축에만** 적용한다(정본 §4 WP-7 · codex N). servers/nodes/context 의 hard 는
#   부트 창에서도 그대로다: 그것들은 '지금 몇 개인가'라 부트라고 해서 덜 참인 사실이 아니다.
#   반대로 CPU 는 부트 순간에만 치솟는 성분(좌석 스폰·인덱싱)이 크다.
BOOT_GRACE_SECS = 300
CPU_GRACE_AXES = ("fleet_cpu_ratio", "load_ratio")
# 데몬이 부트마다 새로 내리는 파일(boot_supervisor::bump_boot_epoch) — **내용은 nonce 라 시각이
# 아니고**, 부트 시각의 증거는 그 파일의 mtime 하나뿐이다.
BOOT_EPOCH_BASENAME = "boot-epoch"
DEFAULT_STATE_DIR = "~/.local/state/cys"

# ★2026-07-06 CSO 위임(master 승인): nodes hard_block 오탐 수정 — A(정적상향)+B(동적 부서가산).
# 부서 1개 상시 기동만으로도 정적 임계(구 12)를 넘어 오탐하던 문제. 부서 소켓 존재=활성 부서로
# 세어 그만큼 임계를 완화한다(전면 동적화(C안)는 보류·백로그 — 이번은 A+B만 채택).
# ★2026-09-03 A3(SURVEY A4·B6-2 · PREP #8 · dept-1 CSO 22:05 "자원 게이트 hard_block 진입 — 계수 결함"):
#   STEP B 의 '부서당 +5' 는 실제 로스터를 과소평가했다 — dept-1 실측 좌석 9(본부 5 + 9 = nodes 14~17 ·
#   hard 18 턱밑)라 2부서부터 hard 오탐 경로(SURVEY 좌석 스윕: 2부서 23 > 22). 규칙을
#   max(18, 12 + Σ좌석)으로 치환한다. 활성 부서 = 소켓 파일 존재가 아니라 **부서 데몬이
#   `cys status --json --socket <sock>` 에 응답한 부서**(기존 표면 재사용 · 신규 RPC/플래그 0),
#   좌석 = 그 응답의 비-exited surfaces 수(agent_alive=false 라도 role 을 쥔 좌석은 점유 — PREP #19).
#   응답 실패(비0·timeout·JSON 파싱·cys 부재·stale 소켓)는 measure_errors `dept(<이름>)` 로 합류해
#   최소 soft 격상(조용한 계상 제외 금지 · P-ORCH-1) · 활성/좌석 미계상. Windows: named pipe 는
#   파일이 아니라 glob 무매치 → depts 0(종전 동일) · 호출은 list argv(shell 0).
NODES_HARD_DEFAULT = 18   # STEP A 정적 floor(구 12) — depts 0~1일 때도 이 완화는 유지
NODES_HARD_BASE = 12      # STEP B 동적 base — 12 + Σ좌석 이 floor 18 을 넘으면 그 값이 hard
DEPT_SOCKET_GLOB = "~/.local/state/cys-dept-*/cys.sock"
DEPT_STATUS_TIMEOUT = 5   # 부서 데몬 응답 대기(초) — 초과 = 그 부서 dept(<이름>) 오류(계상 제외)
LEDGER_TIMEOUT = 5        # ★A3-b: `cys ps` 원장 조회 대기(초) — 초과 = 원장 신뢰 불가(패턴 폴백)


def _active_dept_count():
    """호환 래퍼 — 부서 소켓(cys-dept-*/cys.sock) 파일 존재 개수(구 '활성 부서' 정의).
    ★A3 이후 measure() 는 이 값을 쓰지 않는다(활성 = 데몬 응답 · _dept_roster). 외부 호출자 보존용."""
    return len(glob.glob(os.path.expanduser(DEPT_SOCKET_GLOB)))


SOCKET_PROBE_TIMEOUT = 0.3   # ★A3-c: 리스너 유무만 보는 연결 프로브(로컬 unix 소켓 · 밀리초 단위)
# 확실한 죽음으로 판정하는 errno 집합 — 그 밖은 '판정 불가'라 종전 경로(cys status 왕복)로 간다.
_ERRNO_DEAD = (errno.ECONNREFUSED, errno.ENOENT, errno.ENOTSOCK)


def _socket_listening(path):
    """unix 소켓에 **리스너가 있는가**(스폰 0 · 밀리초). 판정 불가는 True(종전 경로로 진행).

    ★왜 True 로 접는가: 이 프로브의 목적은 '확실히 죽은 소켓에서 5초를 태우지 않는 것' 하나다.
      애매한 경우(Windows named pipe·AF_UNIX 미지원·권한 오류 등)까지 여기서 죽이면 프로브가
      판정기가 되어 버린다 — 판정은 여전히 `cys status` 왕복이 한다(측정 불능은 통과가 아니라
      **종전 경로로 진행**이다)."""
    if os.name == "nt":
        return True                      # named pipe — AF_UNIX 프로브 대상 아님(분기 보존)
    af_unix = getattr(socket, "AF_UNIX", None)
    if af_unix is None:
        return True
    s = None
    try:
        s = socket.socket(af_unix, socket.SOCK_STREAM)
        s.settimeout(SOCKET_PROBE_TIMEOUT)
        s.connect(path)
        return True
    except OSError as e:
        # ★'확실한 죽음' 3종만 False 다(실측 2026-09-03): ECONNREFUSED = 소켓은 있으나 리스너
        #   없음(데몬 비정상 종료 잔재의 전형) · ENOENT = 경로 소멸(glob 과 프로브 사이 레이스) ·
        #   ENOTSOCK = 소켓이 아닌 일반 파일이 그 자리에 있다(errno 38 — 잔재·오생성). 그 밖의
        #   OSError(권한·타임아웃·미지원)는 **판정 불가**이므로 True 로 접어 종전 경로로 보낸다.
        if e.errno in (_ERRNO_DEAD):
            return False
        return True
    finally:
        if s is not None:
            try:
                s.close()
            except OSError:
                pass


def _dept_roster(override=None):
    """부서 로스터 — {"active": 응답 부서 수, "seats": Σ비-exited 좌석, "errors": ["dept(<이름>)", …],
    "depts": [{"name", "seats"}, …]}. 소켓 glob 마다 `cys status --json --socket <sock>` 를 묻는다.
    override(--dept-roster-override 로 파싱된 dict)가 있으면 라이브 조회를 전부 생략한다(테스트 주입 —
    self-test 가 이 머신의 라이브 소켓에 오염되지 않게). 실패한 부서는 errors 에만 남고 활성/좌석에
    들어가지 않는다(조용한 0 좌석 금지)."""
    if override is not None:
        return {"active": int(override.get("active", 0) or 0),
                "seats": int(override.get("seats", 0) or 0),
                "errors": list(override.get("errors") or []),
                "depts": list(override.get("depts") or [])}
    roster = {"active": 0, "seats": 0, "errors": [], "depts": []}
    for sock in sorted(glob.glob(os.path.expanduser(DEPT_SOCKET_GLOB))):
        name = os.path.basename(os.path.dirname(sock))
        if name.startswith("cys-dept-"):
            name = name[len("cys-dept-"):]
        # ★A3-c(2026-09-03 23:1x 실측): 죽은 데몬이 남긴 **stale 소켓 파일** 하나당 이 루프가
        #   DEPT_STATUS_TIMEOUT(5s)을 통째로 태운다(실측 5.11s). 이 게이트는 부트 ④′와 formation
        #   심박(10분)이 부르는 경로라 그 지연이 그대로 부트에 얹힌다. 리스너 유무는 connect
        #   프로브로 **밀리초 안에** 판정되므로 스폰 전에 먼저 묻는다 — 판정(오류 계상·soft 격상)은
        #   종전과 동일하고 **시간만** 줄인다. 정상 teardown 은 상태 디렉터리를 cys-trash 로
        #   격리해 glob 이 무매치이므로(cys-dept dept_tombstone) 이 형상은 비정상 종료 잔재다.
        #   Windows: 부서 소켓은 named pipe 라 AF_UNIX 프로브 대상이 아니다 → 프로브를 건너뛰고
        #   종전 경로(cys status 왕복)로 간다(분기 보존).
        if not _socket_listening(sock):
            roster["errors"].append("dept(%s)" % name)
            continue
        try:
            p = subprocess.run(["cys", "status", "--json", "--socket", sock],
                               capture_output=True, encoding="utf-8", errors="replace",
                               timeout=DEPT_STATUS_TIMEOUT)
            if p.returncode != 0:
                raise ValueError("rc=%d" % p.returncode)
            doc = json.loads(p.stdout)
            surfaces = doc.get("surfaces") if isinstance(doc, dict) else None
            if not isinstance(surfaces, list):
                raise ValueError("surfaces 부재")
        except (subprocess.SubprocessError, OSError, ValueError):
            # TimeoutExpired ⊂ SubprocessError · FileNotFoundError(cys 부재) ⊂ OSError ·
            # JSONDecodeError ⊂ ValueError — 어느 실패든 '조용한 0 좌석' 이 아니라 오류로 신호한다.
            roster["errors"].append("dept(%s)" % name)
            continue
        seats = sum(1 for s in surfaces if isinstance(s, dict) and not s.get("exited"))
        roster["active"] += 1
        roster["seats"] += seats
        roster["depts"].append({"name": name, "seats": seats})
    return roster


def _ledger_override_arg(raw):
    """--servers-ledger-override 의 argparse type — 형식 위반은 인자 오류(EX_USAGE 64)다
    (_roster_override_arg 와 동일 원칙: 조용한 라이브 폴백·내부 예외 융합 금지)."""
    try:
        doc = json.loads(raw)
    except ValueError as e:
        raise argparse.ArgumentTypeError("servers-ledger-override JSON 파싱 실패: %s" % e)
    if not isinstance(doc, dict):
        raise argparse.ArgumentTypeError(
            "servers-ledger-override 는 JSON 객체({lane,depts})여야 한다")
    return doc


def _roster_override_arg(raw):
    """--dept-roster-override 의 argparse type — 잘못된 JSON·비객체는 인자 오류(EX_USAGE 64)다.
    조용히 None 으로 접어 라이브 조회로 폴백하면 주입 의도(결정론)가 깨지고, 내부 예외(70)로
    흘리면 '측정이 일어나지 않은 사용오류' 와 융합된다(A13 분리 원칙)."""
    try:
        doc = json.loads(raw)
    except ValueError as e:
        raise argparse.ArgumentTypeError("dept-roster-override JSON 파싱 실패: %s" % e)
    if not isinstance(doc, dict):
        raise argparse.ArgumentTypeError(
            "dept-roster-override 는 JSON 객체({active,seats,errors,depts})여야 한다")
    return doc


def _ps_lines():
    # 측정 실패는 None으로 신호(빈 리스트로 위장하면 '0=건강'으로 조용히 통과 — P-ORCH-1).
    try:
        out = subprocess.run(["ps", "-axo", "pid,command"], capture_output=True,
                             text=True, timeout=10).stdout
        return out.splitlines()[1:]
    except (subprocess.SubprocessError, OSError):
        return None


def _count_matching(lines, patterns, exclude_patterns=()):
    regs = [re.compile(p) for p in patterns]
    excl = [re.compile(p, re.IGNORECASE) for p in exclude_patterns]
    n = 0
    for line in lines:
        cmd = line.strip().split(None, 1)[-1] if line.strip() else ""
        if "javis_resource_gate" in cmd:
            continue
        if any(r.search(cmd) for r in regs) and not any(r.search(cmd) for r in excl):
            n += 1
    return n


# ── ★WP-7 N: 함대 CPU 측정(별 ps 스폰 · 기존 `_ps_lines` 오류 경로 무접촉) ──
def _ps_cpu_lines():
    """`ps -axo pcpu,command` 의 **헤더 제외** 줄들 → `(lines|None, reason|None)`.

    reason ∈ None(성공) · "absent"(`ps` 실행파일 자체가 없다) · "failed"(있는데 조회가 실패).
    ★이 둘을 가르는 것이 이 함수의 존재 이유다(codex R1 #4): 정본의 Windows 예외는 **`ps` 부재**에
      주어진 것이지 '조회 실패'에 주어진 게 아니다. 하나로 뭉개면 POSIX 에서 ps 조회가 깨졌는데도
      축이 조용히 사라져 exit 0(조용한 allow)이 난다 — P-ORCH-1 위반.
    ★왜 `_ps_lines` 를 재활용하지 않는가: 그쪽은 `pid,command` 계약이고 `_count_matching` ·
      `_server_procs` · `_ledger_servers` 가 그 열 형상을 전제한다. 열을 바꾸면 그 셋이 전부 pcpu 를
      명령줄의 일부로 읽는다 — 축 하나를 얻자고 기존 세 축의 계수를 흔드는 거래는 나쁘다.
      정본(§4 WP-7)도 `ps -axo pcpu,command` 라고 못박았다.
    ★`_ps_lines` 와 달리 **rc 를 본다**: `-axo` 를 모르는 구현(MSYS/BusyBox ps)은 예외를 던지지 않고
      rc≠0 + 빈 stdout 을 내는데, 그것을 `[]` 로 접으면 '함대 CPU 0%' 라는 **거짓 측정**이 된다.
      (기존 `_ps_lines` 의 같은 성질은 이 WP 의 범위가 아니다 — 정본: 기존 오류 경로 현행 유지.)"""
    try:
        p = subprocess.run(["ps", "-axo", "pcpu,command"], capture_output=True,
                           text=True, timeout=10)
    except FileNotFoundError:
        return None, "absent"            # Windows 기본 — 이 플랫폼엔 이 축이 없다
    except (subprocess.SubprocessError, OSError):
        return None, "failed"            # 타임아웃·권한 등 — 측정 실패다
    if p.returncode != 0:
        return None, "failed"            # 플래그 미지원 구현 — '0%' 로 위장하지 않는다
    lines = (p.stdout or "").splitlines()[1:]
    if not lines:
        return None, "failed"            # 헤더뿐 = 아무 프로세스도 못 봤다(형상 이상)
    return lines, None


def _fleet_cpu_percent(lines):
    """FLEET_CPU_PATTERNS 에 걸리는 줄들의 %CPU 합 → `(total|None, reason|None)`.

    ★'매칭 0건'과 '측정 불능'을 섞지 않는다(codex R1 #6):
      - 첫 열을 수치로 읽은 줄이 하나라도 있고 매칭이 0건 = **함대가 안 떠 있다**는 정직한 사실 → 0.0.
      - 첫 열을 한 줄도 수치로 못 읽음(`shape`) = 열 형상이 `pcpu,command` 가 아니다 → None.
        ★이 검사는 '수치가 아님'만 잡는다: ps 가 조용히 `pid,command` 를 냈다면 pid 가 수치라
        통과하고 합이 터무니없이 커진다 → hard. 즉 그 오형상의 귀결은 **차단 방향**이지 조용한
        allow 가 아니다(우리가 감수하는 오차 방향).
      - **매칭된 줄**의 pcpu 가 안 읽히거나(`row_unparsed`) 유한하지 않거나 음수(`row_nonfinite`)
        = 함대의 일부를 못 읽었다 → None. 그 줄만 조용히 건너뛰면 합이 과소계상된다.
    ★`float("nan")` 은 예외 없이 성공하고 이후 모든 비교가 False 라, NaN 이 섞이면 합이 NaN 이 되어
      `NaN >= hard` 가 거짓 → **조용한 allow** 가 된다. 유한성 검사가 그 길을 막는다."""
    regs = [re.compile(pat) for pat in FLEET_CPU_PATTERNS]
    total = 0.0
    parsed = 0
    for line in lines or []:
        s = line.strip()
        if not s:
            continue
        parts = s.split(None, 1)
        if len(parts) < 2:
            continue
        head, cmd = parts[0], parts[1]
        if "javis_resource_gate" in cmd:
            continue                     # 자기 자신은 세지 않는다(_count_matching 과 동형)
        hit = any(r.search(cmd) for r in regs)
        try:
            pcpu = float(head)
        except ValueError:
            if hit:
                return None, "row_unparsed"
            continue
        if not math.isfinite(pcpu) or pcpu < 0:
            if hit:
                return None, "row_nonfinite"
            continue
        parsed += 1
        if hit:
            total += pcpu
    if parsed == 0:
        return None, "shape"
    if not math.isfinite(total):
        # 각 항이 유한해도 **합**은 넘칠 수 있다(1e308+1e308). inf 는 `inf >= hard` 로 hard 를 내지만
        # JSON 으로 나가면 `Infinity` — 표준 JSON 이 아니라 엄격한 소비자에서 깨진다. 판정을
        # 사고로 내지 않고 '못 쟀다' 로 정직하게 접는다(codex R2 · 위임 검체 4).
        return None, "sum_overflow"
    return total, None


def _fleet_cpu_top(lines, top_n=FLEET_CPU_TOP_N):
    """상위 기여 프로세스 [{pcpu, cmd}] — hard 판정의 **회수 대상 지목**(봉인표 ③).

    실패는 조용한 빈 목록이다: 이것은 판정 입력이 아니라 사람이 읽을 증거이므로, 여기서 나는 오류가
    판정을 흔들면 안 된다(판정 입력은 `_fleet_cpu_percent` 하나).
    ★그 격리를 **말로만 두지 않는다**(codex R2 · 위임 검체 5): 종전 판본은 이 약속을 주석에만 두고
      예외를 그대로 흘려, 진단 보조 하나가 정상 측정을 exit 70(EX_SOFTWARE)으로 만들 수 있었다."""
    try:
        regs = [re.compile(pat) for pat in FLEET_CPU_PATTERNS]
        rows = []
        for line in lines or []:
            s = line.strip()
            parts = s.split(None, 1) if s else []
            if len(parts) < 2 or "javis_resource_gate" in parts[1]:
                continue
            try:
                pcpu = float(parts[0])
            except ValueError:
                continue
            if not math.isfinite(pcpu) or pcpu < 0:
                continue
            if any(r.search(parts[1]) for r in regs):
                rows.append({"pcpu": pcpu, "cmd": parts[1][:120]})
        rows.sort(key=lambda r: r["pcpu"], reverse=True)
        return rows[:top_n]
    except Exception:                    # noqa: BLE001 — 진단이 판정을 죽이지 않는다는 계약
        return []


def _boot_epoch_path():
    """이 레인 데몬의 `boot-epoch` 경로 — `dirname($CYS_SOCKET)` 우선(부서 레인은 부서 것을 본다)."""
    sock = os.environ.get("CYS_SOCKET")
    if sock:
        return os.path.join(os.path.dirname(os.path.abspath(os.path.expanduser(sock))),
                            BOOT_EPOCH_BASENAME)
    return os.path.join(os.path.expanduser(DEFAULT_STATE_DIR), BOOT_EPOCH_BASENAME)


def _boot_elapsed(override=None):
    """데몬 부트 후 경과초 → `(elapsed|None, reason)`. None = **유예 없음**(종전 판정 그대로).

    reason ∈ "override" · "ok" · "epoch_missing" · "epoch_unreadable" · "clock_backwards".
    ★근거는 `boot-epoch` 파일의 **mtime** 하나다. 파일 내용은 nonce(boot_supervisor::epoch_nonce —
      시각이 아니라 해시)라 파싱해도 시각이 안 나온다. 그래서 이것은 '부트 시각' 이 아니라
      **감독자가 그 파일을 마지막으로 내린 시각**이다. 어긋나는 형상과 그 방향(codex R1 #3):
        · 감독자 off(`CYS_BOOT_GATES=0`)로 실제 재부트 → 파일이 낡음/부재 → **유예 부족(차단 과다)**
        · 부서 레인이 부서 소켓을 가리킴 → 그 부서의 부트를 본다(이 레인의 판정으로는 옳다).
          단 **다른 부서만 재시작**했는데 이 레인이 그 소켓을 가리키면 유예 과다(차단 못함)가 된다.
        · 복사·동기화가 mtime 을 현재로 갱신 → **유예 과다** · 옛 mtime 보존 → **유예 부족**
        · 시계 전진 → 조기 만료(유예 부족) · 시계 역행(mtime 이 미래) → None(유예 부족)
      완화(유예)의 근거가 흔들릴 때는 **완화하지 않는 쪽**으로 접는다 — 근거 없는 완화는 게이트를
      조용히 약하게 만들지만, 근거 없는 비완화는 종전 판정일 뿐이다. reason 은 `measured` 에 남겨
      '왜 유예가 없었나'를 사후에 읽게 한다(조용한 실패 금지)."""
    if override is not None:
        return override, "override"
    path = _boot_epoch_path()
    try:
        mtime = os.path.getmtime(path)
    except FileNotFoundError:
        return None, "epoch_missing"
    except OSError:
        return None, "epoch_unreadable"
    elapsed = time.time() - mtime
    if elapsed < 0:
        return None, "clock_backwards"   # 미래 mtime. '갓 부팅' 으로 읽지 않는다
    return elapsed, "ok"


def measure(a):
    # 측정 실패는 0으로 조용히 넘기지 않고 measure_errors로 신호(P-ORCH-1) — 소비자(evaluate)가
    # 최소 soft로 격상해 '측정 실패=조용한 allow'를 차단한다.
    errors = []
    need_ps = a.servers_override is None or a.nodes_override is None
    lines = _ps_lines() if need_ps else None
    ps_failed = need_ps and lines is None

    # ★A3-b(dept-1 22:05 실측): servers 의 정본은 **프로세스 원장**(`cys ps`)이다 — 논리 서버 1개가
    #   래퍼 체인(cys run → npm exec vite → node vite) 때문에 ps 패턴에서 3으로 세어져 hard(3)에
    #   걸렸다. 원장은 `cys run` 1회당 항목 1개라 체인과 무관하다. 원장 조회가 실패할 때만 패턴으로
    #   폴백하되, 그때도 **체인 루트만** 세고(_server_procs collapse) 그 사실을 measure_errors 로
    #   신호한다(조용한 과대계수 금지). 임계(soft 2/hard 3)는 그대로다 — 근본은 계수였다.
    if a.servers_override is not None:
        servers = a.servers_override
    else:
        led, led_errors = _ledger_servers(getattr(a, "servers_ledger_override", None))
        errors.extend(led_errors)
        if led is not None:
            servers = led
        elif ps_failed:
            errors.append("servers(ps)")
            servers = None
        else:
            roots = _server_procs(lines)          # 패턴 폴백(체인 루트 접기)
            servers = len(roots)

    if a.nodes_override is not None:
        nodes = a.nodes_override
    elif ps_failed:
        errors.append("nodes(ps)")
        nodes = None
    else:
        nodes = _count_matching(lines, NODE_PATTERNS, NODE_EXCLUDE_PATTERNS)

    if a.load_override is not None:
        load1 = a.load_override
    else:
        try:
            load1 = os.getloadavg()[0]
        except (OSError, AttributeError):
            errors.append("load(getloadavg)")
            load1 = None
    ncpu = os.cpu_count() or 1

    # ★WP-7 N: 함대 CPU 축.
    #   ⓐ `ps` **부재**(Windows 기본)는 측정 실패가 아니라 '이 플랫폼엔 이 축이 없다'는 사실이다 →
    #      `measure_errors` 에 넣지 않는다(정본 §4 WP-7 · Windows exit 계약 불변). `checks` 에는
    #      level="unavailable" 로 남겨 침묵하지 않는다.
    #   ⓑ ps 는 있는데 조회·형상이 깨진 것은 **측정 실패**다 → `measure_errors`(최소 soft 격상).
    #      그러지 않으면 POSIX 에서 CPU 조회만 깨졌을 때 exit 0 = 조용한 allow 가 열린다(codex R1 #4).
    #      단 Windows 에서는 '있으나 -axo 를 모르는 ps'(MSYS)가 흔해 실질적으로 축 부재와 같으므로
    #      ⓐ 로 접는다 — 그러지 않으면 전 Windows 설치가 상시 soft 가 되어 exit 계약이 바뀐다.
    fleet_cpu_top = []
    _fleet_ovr = getattr(a, "fleet_cpu_override", None)   # 신설 플래그 — 구 네임스페이스 안전
    if _fleet_ovr is not None:
        fleet_cpu_ratio, fleet_reason = _fleet_ovr, "override"
    else:
        cpu_lines, ps_err = _ps_cpu_lines()
        if ps_err is not None:
            fleet_cpu_ratio, fleet_reason = None, ps_err
        else:
            fleet_pct, sum_err = _fleet_cpu_percent(cpu_lines)
            if sum_err is not None:
                fleet_cpu_ratio, fleet_reason = None, sum_err
            else:
                # ★판정은 **원시값**으로 한다(codex R1 #10): 판정 전에 소수 셋째 자리로 반올림하면
                #   0.9996 이 1.0 이 되어 명시 임계보다 낮은 자리에서 hard 가 난다. 반올림은 사람이
                #   읽는 출력에서만 한다.
                fleet_cpu_ratio, fleet_reason = fleet_pct / 100.0 / ncpu, "ok"
                # ★진단 수집은 판정 **바깥**이다: 여기서 나는 어떤 예외도 측정을 무효로 만들지
                #   않는다(최상위 경계가 그것을 exit 70='측정 실패'로 접기 때문 · codex R2 검체 5).
                #   함수 안에도 같은 격리가 있지만, 격리를 **호출 지점에서 선언**해 두어야 그 함수를
                #   갈아끼우거나 다시 써도 계약이 남는다.
                try:
                    fleet_cpu_top = _fleet_cpu_top(cpu_lines)
                except Exception:        # noqa: BLE001 — 진단이 판정을 죽이지 않는다는 계약
                    fleet_cpu_top = []
    if fleet_reason not in ("ok", "override", "absent") and os.name != "nt":
        errors.append("fleet_cpu(ps)")

    # ★부트 유예 — CPU 축 hard→soft 창(300s). 근거 없음(None)은 유예 없음이다(_boot_elapsed).
    boot_elapsed, boot_reason = _boot_elapsed(getattr(a, "boot_elapsed_override", None))
    # 음수 경과초는 시각이 아니다(주입 오류·시계 이상) — 유예를 주지 않는다.
    boot_grace = boot_elapsed is not None and 0 <= boot_elapsed < BOOT_GRACE_SECS

    # STEP B(★A3 치환): 활성 부서·좌석은 부서 데몬 응답(_dept_roster)에서 — 소켓 파일 수
    # (_active_dept_count)가 아니다. 응답 실패는 measure_errors 로 합류(→ evaluate 가 최소 soft 격상 ·
    # 조용한 allow 금지). --nodes-hard가 argparse 기본값(NODES_HARD_DEFAULT)에서 명시적으로 바뀌지
    # 않았으면 동적 계산 max(18, 12 + Σ좌석) 적용, 바뀌었으면(테스트 주입 등) 그 값 그대로 우선 —
    # 동적계산 생략(종전 규약 유지).
    roster = _dept_roster(getattr(a, "dept_roster_override", None))
    active_depts = roster["active"]
    errors.extend(roster["errors"])
    if a.nodes_hard != NODES_HARD_DEFAULT:
        nodes_hard_effective = a.nodes_hard
    else:
        nodes_hard_effective = max(NODES_HARD_DEFAULT, NODES_HARD_BASE + roster["seats"])

    return {"servers": servers, "nodes": nodes,
            "load1": round(load1, 2) if load1 is not None else None,
            "ncpu": ncpu,
            "load_ratio": round(load1 / ncpu, 3) if load1 is not None else None,
            "fleet_cpu_ratio": fleet_cpu_ratio,
            "fleet_cpu_reason": fleet_reason,
            "fleet_cpu_top": fleet_cpu_top,
            "boot_elapsed": (round(boot_elapsed, 1) if boot_elapsed is not None else None),
            "boot_grace": boot_grace, "boot_grace_reason": boot_reason,
            "context_pct": a.context, "measure_errors": errors,
            "active_depts": active_depts, "dept_seats": roster["seats"], "depts": roster["depts"],
            "nodes_hard_effective": nodes_hard_effective}


# ── ★opt-in rate 축(soft-only) — 구독제(정액) 5h rate 사용률 사전 경고 ──
def _rate_enabled(a):
    """rate 축은 opt-in — --rate-check 플래그 또는 env CYS_GATE_RATE=1일 때만 발화."""
    return bool(getattr(a, "rate_check", False)) or os.environ.get("CYS_GATE_RATE") == "1"


def _rate_accounts(a):
    """rate 원천 — --rate-override(테스트 주입) 우선, 없으면 `cys usage-accounts --json`.
    cys 부재·타임아웃·파싱 실패는 None(축 자체 스킵) — best-effort, 조직 기동 무차단."""
    if getattr(a, "rate_override", None) is not None:
        try:
            data = json.loads(a.rate_override)
        except ValueError:
            return None
    else:
        try:
            out = subprocess.run(["cys", "usage-accounts", "--json"],
                                 capture_output=True, text=True, timeout=3).stdout
            data = json.loads(out)
        except (subprocess.SubprocessError, OSError, ValueError):
            return None
    if isinstance(data, dict):      # {"accounts":[...]} 또는 바로 [...] 둘 다 수용
        data = data.get("accounts")
    return data if isinstance(data, list) else None


def _rate_checks(a):
    """rate 5h 사용률 soft 경고 축(soft-only). 발화 조건: rate label=="5h"·신선(stale_secs<600)·
    used_pct>=rate_soft. hard 없음 — 게이트는 master 부트 플로우가 호출하므로 rate로 조직 기동을
    막지 않는다. 반환: soft check dict 리스트(빈 리스트=무발화). (테스트 주입=--rate-override)"""
    accounts = _rate_accounts(a)
    if not accounts:
        return []
    out = []
    for acct in accounts:
        if not isinstance(acct, dict):
            continue
        label = acct.get("label", "?")
        for entry in acct.get("rate", []) or []:
            if not isinstance(entry, dict) or entry.get("label") != "5h":
                continue
            stale = entry.get("stale_secs")
            if stale is None:            # rate 엔트리에 없으면 계정 레벨로 폴백
                stale = acct.get("stale_secs")
            if stale is None or stale >= 600:   # null·비신선(오래된 측정)은 스킵
                continue
            used = entry.get("used_pct")
            if used is None or used < a.rate_soft:
                continue
            out.append({"metric": "rate_5h(%s)" % label, "value": used,
                        "soft": a.rate_soft, "hard": None, "level": "soft"})
    return out


# ── ★T9(P3-1·R3-P03-1) 곱셈 편성 예산 축(W6) ──
def _formation_budget_check(m, a):
    """check --formation-size + env CYS_FORMATION_BUDGET 의 곱셈 편성 예산 축.

    발화 조건(계약 문면 그대로): formation_size is not None **이면서** CYS_FORMATION_BUDGET 이
    정수로 파싱될 때만 — 어느 한쪽 부재=완전 무동작(None 반환·기존 호출자 회귀 0).
    투영 = 측정 nodes + 활성 부서수 × formation_size. 초과(projected > budget)=hard.
    nodes=None(ps 실패)은 None 반환 — 예외 금지(터지면 70). measure_errors('nodes(ps)')가
    이미 최소 soft 격상을 담당하므로 '측정 불능=조용한 allow'는 아니다(P-ORCH-1).
    반환: (check_dict_or_None, warning_str_or_None) — 비정수 env 는 발화 조건 미충족이라
    verdict 무접촉이되 warning 으로만 가청화한다(침묵 금지·판정 오염 0)."""
    size = getattr(a, "formation_size", None)
    if size is None:
        return None, None
    raw = os.environ.get("CYS_FORMATION_BUDGET")
    if raw is None:
        return None, None
    try:
        budget = int(raw.strip())
    except (ValueError, AttributeError):
        return None, "formation_budget_env_invalid:%r" % raw
    nodes = m.get("nodes")
    if nodes is None:
        return None, None  # ps 실패 — measure_errors 경로 소관(예외 금지 — 70 방지)
    projected = nodes + m.get("active_depts", 0) * size
    level = "hard" if projected > budget else "ok"
    return {"metric": "formation_budget", "value": projected, "soft": budget,
            "hard": budget, "level": level}, None


def evaluate(m, a):
    checks = []
    grace = bool(m.get("boot_grace"))

    def add(metric, value, soft, hard):
        """★WP-7 N 로 두 갈래가 늘었다.
          ① `hard is None` = **soft 전용 축**(hard 판정 자체가 없다). `load_ratio` 가 이쪽으로 간다 —
             호스트 부하에는 우리가 회수할 수 없는 성분이 섞이므로 그것으로 착수를 거부하지 않는다.
          ② 부트 유예: CPU 축(CPU_GRACE_AXES)의 hard 는 부트 후 300초 창에서 soft 로 내려간다.
             내려간 사실은 `boot_grace: true` 로 그 check 에 **남긴다**(조용한 완화 금지 — 사후에
             '왜 hard 가 아니었나' 를 판독할 수 있어야 한다)."""
        if value is None:
            return
        if hard is None:
            level = "soft" if value >= soft else "ok"
        else:
            level = "hard" if value >= hard else ("soft" if value >= soft else "ok")
        c = {"metric": metric, "value": value, "soft": soft, "hard": hard, "level": level}
        if level == "hard" and grace and metric in CPU_GRACE_AXES:
            c["level"] = "soft"
            c["boot_grace"] = True
        checks.append(c)

    add("servers", m["servers"], a.servers_soft, a.servers_hard)
    add("nodes", m["nodes"], a.nodes_soft, m["nodes_hard_effective"])
    # ★WP-7 N: 함대 CPU 축(hard 있음) · 호스트 load 축(soft 전용 — hard=None).
    if m.get("fleet_cpu_ratio") is None:
        # 축 미적용/측정 불능 — 값이 없으므로 트립이 아니다. 사유는 라벨에 붙여 침묵을 막는다
        # (측정 실패 쪽의 soft 격상은 measure_errors 가 이미 담당한다 — measure() ⓑ).
        checks.append({"metric": "fleet_cpu_ratio", "value": None,
                       "soft": getattr(a, "fleet_cpu_soft", FLEET_CPU_SOFT_DEFAULT),
                       "hard": getattr(a, "fleet_cpu_hard", FLEET_CPU_HARD_DEFAULT),
                       "level": "unavailable", "reason": m.get("fleet_cpu_reason")})
    else:
        add("fleet_cpu_ratio", m["fleet_cpu_ratio"],
            getattr(a, "fleet_cpu_soft", FLEET_CPU_SOFT_DEFAULT),
            getattr(a, "fleet_cpu_hard", FLEET_CPU_HARD_DEFAULT))
    add("load_ratio", m["load_ratio"], a.load_soft_ratio, None)
    add("context_pct", m["context_pct"], a.context_soft, a.context_hard)

    # ★opt-in rate 축(soft-only) — --rate-check 또는 CYS_GATE_RATE=1일 때만. hard 없음:
    # 게이트는 master 부트 플로우가 호출하므로 rate로 조직 기동을 막지 않는다(soft만 반영).
    if _rate_enabled(a):
        checks.extend(_rate_checks(a))

    # ★T9(W6) 곱셈 편성 예산 축 — 발화 조건(플래그∧env 정수) 미충족이면 완전 무동작(회귀 0).
    fb, _fb_warn = _formation_budget_check(m, a)
    if fb is not None:
        checks.append(fb)

    # 측정 실패는 최소 soft로 격상(조용한 allow 금지 · P-ORCH-1) — 실제 hard 트립이 있으면 hard가 우선.
    worst = "soft" if m.get("measure_errors") else "ok"
    for c in checks:
        if c["level"] == "hard":
            worst = "hard"
            break
        if c["level"] == "soft":
            worst = "soft"
    return worst, checks


def cmd_check(a):
    m = measure(a)
    worst, checks = evaluate(m, a)
    # ★T1/B1(Phase 1 · DESIGN-DECISIONS §2-5 · 조건 10): --require-context 지정 시 context
    #   미제공(자기보고 부재)을 soft(exit 1)로 격상 — '미측정=조용한 allow' 상속을 소비부
    #   (javis_completion_guard 등 verify 실행 경로)가 결정론으로 감지하게 한다.
    #   기본 동작 불변(플래그 없으면 종전과 동일 — 기존 부트 플로우 회귀 0). 실제 자원
    #   soft/hard 트립이 있으면 그것이 그대로 우선한다(여기서는 ok→soft 승격만).
    if getattr(a, "require_context", False) and m["context_pct"] is None and worst == "ok":
        worst = "soft"
    verdict = {"ok": "allow", "soft": "soft_warn", "hard": "hard_block"}[worst]
    # ★WP-7 N: trips = **실제로 트립한 축**(soft·hard)만. 종전 `!= "ok"` 는 신설 level
    #   "unavailable"(측정 불능 라벨)까지 트립으로 실어, 소비부(bootstrap 의 nodes-only 예외 판정 ·
    #   completion_guard `_pick_trip`)에 '아무 값도 없는 트립'을 흘렸을 것이다.
    trips = [c for c in checks if c["level"] in ("soft", "hard")]
    warnings = []
    if m["measure_errors"]:
        warnings.append("measure_error:" + ",".join(m["measure_errors"]))
    if m["context_pct"] is None:
        warnings.append("context_unmeasured")
    # ★WP-7 N: `warnings` 에는 **아무것도 더 넣지 않는다**(codex R1 #5). 이 배열은 표기가 아니라
    #   판정 입력이다 — `javis_completion_guard._soft_kind` 는 `warnings == ["context_unmeasured"]`
    #   **완전일치**로 proceed_unmeasured/skip_soft 를 가른다. 축 부재·부트 유예·무동작 플래그를
    #   여기 얹으면 exit 은 그대로여도 소비자의 분기가 바뀐다(Windows 에서 상시 skip_soft).
    #   그래서 이 세 사실은 각각 제 자리에 둔다:
    #     · 축 부재/사유  → `checks[].level=="unavailable"` + `measured.fleet_cpu_reason`
    #     · 부트 유예     → `checks[].boot_grace` + `measured.boot_grace/boot_elapsed/boot_grace_reason`
    #     · 무동작 플래그 → **stderr**(진단 채널 — 계약 채널 stdout 은 오염시키지 않는다)
    if getattr(a, "load_hard_ratio", LOAD_HARD_RATIO_DEFAULT) != LOAD_HARD_RATIO_DEFAULT:
        sys.stderr.write("[resource-gate] --load-hard-ratio=%s 는 무동작이다 — load_ratio 는 "
                         "0.14.31 부터 soft 전용(호스트 부하로 착수를 거부하지 않는다). "
                         "함대 CPU 차단은 --fleet-cpu-hard 소관.\n" % a.load_hard_ratio)
    # ★T9: 비정수 CYS_FORMATION_BUDGET 는 판정 무접촉(발화 조건 미충족)이되 침묵하지 않는다.
    _fb, fb_warn = _formation_budget_check(m, a)
    if fb_warn:
        warnings.append(fb_warn)
    result = {"verdict": verdict, "measured": m, "trips": trips,
              "checks": checks, "warnings": warnings}
    if a.json:
        print(json.dumps(result, ensure_ascii=False, indent=1))
    else:
        print(f"verdict: {verdict}")
        for w in warnings:
            print(f"  ⚠ {w}")
        for c in checks:
            # ★"unavailable" 을 표에 넣지 않으면 KeyError → 최상위 경계가 exit 70(측정 실패)으로
            #   접는다 — 라벨 하나 때문에 게이트 전체가 '내부 오류'가 되는 길을 막는다.
            mark = {"ok": "·", "soft": "⚠", "hard": "✗", "unavailable": "?"}[c["level"]]
            hard_txt = "—(soft 전용)" if c["hard"] is None else c["hard"]
            note = " [부트 유예 hard→soft]" if c.get("boot_grace") else ""
            if c["level"] == "unavailable":
                print(f"  {mark} {c['metric']}=축 미적용/측정 불능"
                      f"(사유 {c.get('reason')} · ps 부재면 이 플랫폼에 이 축이 없다)")
            else:
                # 판정은 원시값으로 했고(codex R1 #10) 여기서만 읽기 좋게 줄인다.
                shown = f"{c['value']:.3f}" if isinstance(c["value"], float) else c["value"]
                print(f"  {mark} {c['metric']}={shown} "
                      f"(soft {c['soft']} / hard {hard_txt}){note}")
        # ★A3: 부서 좌석 1줄 — nodes hard 가 어디서 왔는지(어느 부서·몇 좌석) 사람이 읽게.
        dept_list = ", ".join("%s=%s" % (d.get("name"), d.get("seats")) for d in m.get("depts") or [])
        how = ("--nodes-hard 명시" if a.nodes_hard != NODES_HARD_DEFAULT
               else "max(%d, %d+Σ좌석)" % (NODES_HARD_DEFAULT, NODES_HARD_BASE))
        print(f"  depts: active={m['active_depts']} seats={m['dept_seats']}"
              f" [{dept_list or '-'}] → nodes hard {m['nodes_hard_effective']} ({how})")
        if m["measure_errors"]:
            print("measure_error: 자원 측정 실패(ps/load/부서 데몬 dept(<이름>)) — 조용한 allow 금지, "
                  "최소 soft로 격상. 측정 환경 확인 후 재시도(dept(…)=그 부서 데몬 무응답·stale 소켓).")
        if m["context_pct"] is None:
            print("context_unmeasured: --context 미제공 — 컨텍스트 60%/clear 규칙을 검사하지 못함. "
                  "check 시 --context <pct> 전달 권장.")
        # ★WP-7 N: 부트 유예가 실제로 판정을 바꿨으면 그 사실을 사람에게도 남긴다(조용한 완화 금지).
        if m.get("boot_grace") and any(c.get("boot_grace") for c in checks):
            print("boot_grace: 데몬 부트 후 %ss(<%ds) — CPU 축 hard 를 soft 로 내렸다. "
                  "유예 밖이면 같은 값이 hard_block 이다."
                  % (m.get("boot_elapsed"), BOOT_GRACE_SECS))
        if worst == "hard":
            print("hard_block: 착수 거부 — 자원 정리(서버 kill·/clear·노드 회수) 후 재시도하거나 "
                  "master 승인으로 임계 상향. (사후 watchdog와 별개의 사전 게이트)")
            # ★봉인표 ③: 함대 CPU 로 막힌 경우 **회수 대상**을 지목한다. 원인 미상의 영구 보류는
            #   자가치유(편성이 좌석 전멸 부서의 유일 복구 경로)를 사람 없이 못 풀게 만든다.
            if any(c["metric"] == "fleet_cpu_ratio" and c["level"] == "hard" for c in checks):
                top = m.get("fleet_cpu_top") or []
                for row in top:
                    print("  ↳ fleet_cpu 기여 %.1f%% %s" % (row.get("pcpu", 0.0), row.get("cmd", "")))
                print("  ↳ fleet_cpu 는 우리 프로세스만 센다 — %s 회수하면 풀린다"
                      "(죽은 좌석은 CPU 를 먹지 않으므로 이 축이 좌석 복구 자체를 막지는 않는다)."
                      % ("위 프로세스를" if top else "`ps -axo pcpu,command` 상위 함대 프로세스를"))
        elif worst == "soft":
            print("soft_warn: 진행 허용하되 경고 push 권장.")
    return {"ok": EXIT_ALLOW, "soft": EXIT_SOFT, "hard": EXIT_HARD}[worst]


def cmd_classify(a):
    """stdin의 ps 형식 줄들을 패턴으로 분류(테스트·디버그용 결정론 경로)."""
    lines = sys.stdin.read().splitlines()
    result = {
        "servers": _count_matching(lines, SERVER_PATTERNS, SERVER_EXCLUDE_PATTERNS),
        "nodes": _count_matching(lines, NODE_PATTERNS, NODE_EXCLUDE_PATTERNS),
    }
    print(json.dumps(result, ensure_ascii=False))
    return EXIT_ALLOW


# ── ★G12(cokacdir 성찰 2026-07-04): hard_block '판정'과 분리돼 있던 '집행' ──
def _server_procs(lines=None, collapse=True):
    """SERVER_PATTERNS 매칭 (pid, cmd) 목록 — _count_matching과 동일 분류(제외 패턴 포함).

    ★A3-b(2026-09-03 dept-1 실측): collapse=True 면 **체인 루트만** 남긴다 — 매칭된 프로세스의
      조상이 이미 매칭돼 있으면 그것은 같은 논리 서버의 자식이다(`cys run -- npm exec vite` →
      `npm exec vite` → `node …/vite` 3프로세스 = 서버 1개). kill 대상 집합은 바뀌지 않는다
      (호출부가 roots ∪ _descendants(roots) 를 죽인다) — 바뀌는 것은 **계수**뿐이다."""
    lines = lines if lines is not None else (_ps_lines() or [])
    regs = [re.compile(p) for p in SERVER_PATTERNS]
    excl = [re.compile(p, re.IGNORECASE) for p in SERVER_EXCLUDE_PATTERNS]
    out = []
    for line in lines:
        parts = line.strip().split(None, 1)
        if len(parts) != 2 or not parts[0].isdigit():
            continue
        pid, cmd = int(parts[0]), parts[1]
        if "javis_resource_gate" in cmd:
            continue
        if any(r.search(cmd) for r in regs) and not any(r.search(cmd) for r in excl):
            out.append((pid, cmd))
    return _collapse_to_roots(out) if collapse else out


def _ppid_map():
    """pid → ppid. 조회 실패는 None(체인 접기 불가 — 호출부가 measure_errors 로 신호한다)."""
    try:
        out = subprocess.run(["ps", "-Ao", "pid=,ppid="], capture_output=True,
                             text=True, timeout=10).stdout
    except (subprocess.SubprocessError, OSError):
        return None
    m = {}
    for line in out.splitlines():
        f = line.split()
        if len(f) == 2 and f[0].isdigit() and f[1].isdigit():
            m[int(f[0])] = int(f[1])
    return m


def _collapse_to_roots(procs, ppid=None):
    """매칭 프로세스 목록 → **체인 루트만**(조상이 이미 매칭이면 제외). ppid 조회 실패 시 원본 그대로.

    ★왜 계수를 접는가(A3-b · dept-1 22:05 실측 근거 impl/live-evidence/dept1-queue-starvation-2205.txt:56):
      논리 서버 1개가 래퍼 체인 때문에 3으로 세어져 servers hard(3)에 걸렸다 — '서버 누적'을 막는
      임계가 **하나도 안 띄운 상태에서** 착수를 거부한 것이다. 임계는 그대로 두고(근본은 계수)
      같은 트리에 속한 자식을 접는다."""
    if not procs:
        return procs
    pm = _ppid_map() if ppid is None else ppid
    if not pm:
        return procs                       # 체인 판정 불가 — 종전 계수(보수적 과대) 유지
    matched = {p for p, _c in procs}
    roots = []
    for pid, cmd in procs:
        cur, depth, has_matched_ancestor = pm.get(pid), 0, False
        while cur and cur > 1 and depth < 64:      # depth 상한 = 순환 방어
            if cur in matched:
                has_matched_ancestor = True
                break
            cur, depth = pm.get(cur), depth + 1
        if not has_matched_ancestor:
            roots.append((pid, cmd))
    return roots


def _ledger_servers(override=None, socket_path=None):
    """`cys ps` **프로세스 원장** 기준 논리 서버 수 → (개수 or None, 오류 목록).

    ★A3-b: 원장은 `cys run -- <명령>` 1회당 항목 1개다(래퍼 체인이 몇 프로세스든). 그래서 계수의
      정본은 ps 패턴이 아니라 원장이다. 단 원장은 서버 전용이 아니므로(dept-1 실측: `cys events
      --category … --reconnect` 가 등재돼 있다) **SERVER_PATTERNS 매칭 항목만** 센다.
    범위: 현재 레인(`cys ps`) + 부서 소켓(`cys ps --socket <sock>`) 합집합 · pid 중복 제거.
    실패: 현재 레인 조회 실패 → (None, ["servers(ledger)"]) 로 호출부가 **패턴 폴백**하게 한다.
      부서 조회 실패는 그 부서만 제외하고 `servers-ledger(<이름>)` 오류로 남긴다(전면 폴백 아님).
    출력 형식(실측): `pid=<p>\\tpgid=<g>\\tscoped=<b>\\tsurface=<id>\\t<cmd>` 또는 `(ledger empty)`.
    override(--servers-ledger-override)는 {"lane": "<텍스트>", "depts": {"<sock>": "<텍스트>"}}."""
    regs = [re.compile(p) for p in SERVER_PATTERNS]
    excl = [re.compile(p, re.IGNORECASE) for p in SERVER_EXCLUDE_PATTERNS]
    errors, seen = [], {}

    def _consume(text):
        for line in (text or "").splitlines():
            if not line.startswith("pid="):
                continue                    # "(ledger empty)" · 잡음 행
            fields = line.split("\t")
            try:
                pid = int(fields[0][len("pid="):])
            except (ValueError, IndexError):
                continue
            cmd = fields[-1] if len(fields) >= 5 else ""
            if any(r.search(cmd) for r in regs) and not any(r.search(cmd) for r in excl):
                seen[pid] = cmd

    def _run_ps(argv):
        p = subprocess.run(argv, capture_output=True, encoding="utf-8",
                           errors="replace", timeout=LEDGER_TIMEOUT)
        if p.returncode != 0:
            raise ValueError("rc=%d" % p.returncode)
        return p.stdout

    if override is not None:
        _consume(override.get("lane") or "")
        for _sock, text in (override.get("depts") or {}).items():
            _consume(text)
        return len(seen), errors

    try:
        _consume(_run_ps(["cys", "ps"]))
    except (subprocess.SubprocessError, OSError, ValueError):
        return None, ["servers(ledger)"]     # 현재 레인 실패 = 원장 신뢰 불가 → 패턴 폴백
    lane_sock = os.environ.get("CYS_SOCKET")
    for sock in sorted(glob.glob(os.path.expanduser(DEPT_SOCKET_GLOB))):
        if lane_sock and os.path.abspath(sock) == os.path.abspath(lane_sock):
            continue                         # 현재 레인과 같은 소켓 — 이미 셌다
        name = os.path.basename(os.path.dirname(sock))
        if name.startswith("cys-dept-"):
            name = name[len("cys-dept-"):]
        if not _socket_listening(sock):      # A3-c 프로브 재사용(stale 소켓에서 대기 0)
            errors.append("servers-ledger(%s)" % name)
            continue
        try:
            _consume(_run_ps(["cys", "ps", "--socket", sock]))
        except (subprocess.SubprocessError, OSError, ValueError):
            errors.append("servers-ledger(%s)" % name)
    return len(seen), errors


def _descendants(roots):
    """pid/ppid 체인 전(全) 자손 — phoenix_harness._descendants 동형(문자열 매칭 아님·collateral 0)."""
    try:
        out = subprocess.run(["ps", "-Ao", "pid=,ppid="], capture_output=True,
                             text=True, timeout=10).stdout
    except (subprocess.SubprocessError, OSError):
        return set()
    kids = {}
    for line in out.splitlines():
        p = line.split()
        if len(p) == 2 and p[0].isdigit() and p[1].isdigit():
            kids.setdefault(int(p[1]), []).append(int(p[0]))
    seen, stack = set(), list(roots)
    while stack:
        for c in kids.get(stack.pop(), []):
            if c not in seen:
                seen.add(c)
                stack.append(c)
    return seen


def _proc_age_sec(pid):
    """ps etime([[dd-]hh:]mm:ss) → 초. 조회 불가 시 None."""
    try:
        et = subprocess.run(["ps", "-o", "etime=", "-p", str(pid)],
                            capture_output=True, text=True, timeout=10).stdout.strip()
        if not et:
            return None
        days, rest = (et.split("-", 1) + [""])[:2] if "-" in et else ("0", et)
        parts = [int(x) for x in rest.split(":")]
        while len(parts) < 3:
            parts.insert(0, 0)
        h, m, s = parts
        return int(days) * 86400 + h * 3600 + m * 60 + s
    except (subprocess.SubprocessError, OSError, ValueError):
        return None


def cmd_enforce(a):
    """dev 서버 초과분 정리 집행 — hard 임계 도달 시 매칭 서버 pid-tree kill.
    기본 dry-run(파괴 행위 deny-by-default) · --kill 명시 시만 실행 · 원장 기록.
    --min-age N: 기동 N초 미만 서버는 보호(watchdog '45초+' 규칙 — 방금 띄운 의도 서버 오살 방지).
    --notify R: 실제 kill 발생 시에만 역할 R에 1줄 push(무사건 무push — 스케줄 스팸 0).
    (사후 watchdog·사전 check와 별개의 '집행' 경로 — 판정과 집행의 분리 해소.)"""
    import signal as _signal
    if a.pids:  # 테스트 결정론 주입(servers-override 관례) — 임계 게이트 우회
        roots = [(p, "(injected)") for p in a.pids]
    else:
        roots = _server_procs()
        if len(roots) < a.servers_hard:
            print(json.dumps({"verdict": "no_enforce", "servers": len(roots),
                              "hard": a.servers_hard}, ensure_ascii=False))
            return EXIT_ALLOW
        if a.min_age:
            aged = []
            for p, c in roots:
                age = _proc_age_sec(p)
                if age is None or age >= a.min_age:  # 나이 미상=보호 아님(watchdog 의도 우선)
                    aged.append((p, c))
            if not aged:
                print(json.dumps({"verdict": "no_enforce", "servers": len(roots),
                                  "why": "전건 min-age(%ss) 미만 — 신생 보호" % a.min_age},
                                 ensure_ascii=False))
                return EXIT_ALLOW
            roots = aged
    root_pids = [p for p, _ in roots]
    victims = sorted(set(root_pids) | _descendants(root_pids))  # 죽이기 전에 트리 수집
    killed = 0
    if a.kill:
        # Windows 패리티: SIGKILL 부재(getattr 폴백) · os.kill(pid,0) 프로브는 Windows에서
        # TerminateProcess라 금지 — 생존 확인은 ps로만(부재 시 kill 시도 완료를 종료로 간주).
        sigkill = getattr(_signal, "SIGKILL", _signal.SIGTERM)
        for v in victims:
            try:
                os.kill(v, _signal.SIGTERM)
            except OSError:
                pass
        time.sleep(1)
        for v in victims:
            try:
                st = subprocess.run(["ps", "-o", "pid=", "-p", str(v)],
                                    capture_output=True, text=True, timeout=10).stdout.strip()
            except (subprocess.SubprocessError, OSError):
                st = ""
            if st:
                try:
                    os.kill(v, sigkill)
                except OSError:
                    pass
        time.sleep(0.3)
        for v in victims:  # 좀비 인지 집계 — kill(v,0) 프로브는 좀비에 성공해 잔존으로 오판(G5 동형)
            try:
                st = subprocess.run(["ps", "-o", "state=", "-p", str(v)],
                                    capture_output=True, text=True, timeout=10).stdout.strip()
            except (subprocess.SubprocessError, OSError):
                st = ""
            if not st or st.startswith("Z"):
                killed += 1
    ledger = os.path.join(os.environ.get("JAVIS_ROOT") or os.getcwd(),
                          "_round", "resource_enforce.jsonl")
    try:
        os.makedirs(os.path.dirname(ledger), exist_ok=True)
        with open(ledger, "a", encoding="utf-8") as f:
            f.write(json.dumps({"ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                                "mode": "kill" if a.kill else "dry_run",
                                "roots": [{"pid": p, "cmd": c[:120]} for p, c in roots],
                                "victims": victims, "killed": killed},
                               ensure_ascii=False) + "\n")
    except OSError:
        pass
    if a.kill and killed and getattr(a, "notify", None):
        try:  # 실사건에만 push — 무사건 스케줄 주기는 침묵(스팸 0)
            subprocess.run(["cys", "send", "--queued", "--to", a.notify,
                            "[watchdog] 자원 집행 — dev 서버 pid-tree %d개 kill (roots %s). "
                            "원장: _round/resource_enforce.jsonl" % (killed, root_pids)],
                           timeout=15)
        except (subprocess.SubprocessError, OSError, FileNotFoundError):
            pass
    print(json.dumps({"verdict": "enforced" if a.kill else "dry_run",
                      "roots": root_pids, "victims": victims, "killed": killed},
                     ensure_ascii=False))
    return EXIT_ALLOW


class _UsageExit(Exception):
    """argparse 의 SystemExit(2) 를 가로채 EX_USAGE(64) 로 remap 하기 위한 내부 신호."""


class _GateArgumentParser(argparse.ArgumentParser):
    """★A13: argparse 의 사용오류 종료를 exit 2(=EXIT_HARD) 로 흘려보내지 않는다.

    argparse 는 error()/exit(2) 로 SystemExit(2) 를 던진다 — 그 2가 이 도구의 '자원 hard_block'
    코드와 같아서, 소비부가 **오타 하나를 팀 기동 거부로 오독**했다. usage 메시지는 그대로
    stderr 에 내되(진단 보존), 종료 코드만 EX_USAGE(64)로 분리한다."""

    def error(self, message):
        self.print_usage(sys.stderr)
        sys.stderr.write("%s: error: %s\n" % (self.prog, message))
        raise _UsageExit(message)

    def exit(self, status=0, message=None):
        if message:
            sys.stderr.write(message)
        if status == 0:
            raise SystemExit(0)          # --help 등 정상 종료는 보존
        raise _UsageExit("argparse exit %s" % status)


def main(argv=None):
    p = _GateArgumentParser(description="자원 사전 게이트 — 착수 전 차단 (P0-3)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("check")
    c.add_argument("--context", type=float, default=None, help="자기보고 컨텍스트 %%")
    c.add_argument("--json", action="store_true")
    c.add_argument("--servers-soft", type=int, default=2)
    c.add_argument("--servers-hard", type=int, default=3)
    c.add_argument("--nodes-soft", type=int, default=12)
    c.add_argument("--nodes-hard", type=int, default=NODES_HARD_DEFAULT)
    c.add_argument("--load-soft-ratio", type=float, default=1.0)
    # ★WP-7 N: load 축은 soft 전용이 됐다 — 이 플래그는 **무동작**이다. 삭제하지 않는 이유는
    #   구 호출자가 넘기면 argparse 가 EX_USAGE(64)를 내고 소비부가 그것을 '측정 실패'로 loud 처리하기
    #   때문이다(회귀 방향이 더 나쁘다). 기본값과 다르게 주면 stderr 로 무동작임을 고지한다.
    c.add_argument("--load-hard-ratio", type=float, default=LOAD_HARD_RATIO_DEFAULT,
                   help="[무동작·0.14.31~] load_ratio 는 soft 전용. 함대 CPU 차단은 --fleet-cpu-hard")
    c.add_argument("--fleet-cpu-soft", dest="fleet_cpu_soft", type=float,
                   default=FLEET_CPU_SOFT_DEFAULT,
                   help="함대 %%CPU 합/100/ncpu 분율 soft 임계(기본 0.5 = 전 코어의 50%% 상당)")
    c.add_argument("--fleet-cpu-hard", dest="fleet_cpu_hard", type=float,
                   default=FLEET_CPU_HARD_DEFAULT,
                   help="같은 분율의 hard 임계(기본 1.0 = 전 코어 100%% 상당)")
    c.add_argument("--fleet-cpu-override", dest="fleet_cpu_override", type=float, default=None,
                   help="테스트 주입 — fleet_cpu_ratio(분율)를 직접 주입하고 ps 조회를 생략")
    c.add_argument("--boot-elapsed-override", dest="boot_elapsed_override", type=float,
                   default=None,
                   help="테스트 주입 — 데몬 부트 후 경과초(부트 유예 창 판정용 · boot-epoch mtime 대체)")
    c.add_argument("--context-soft", type=float, default=50.0)
    c.add_argument("--context-hard", type=float, default=60.0)
    c.add_argument("--servers-override", type=int, default=None, help="테스트 주입")
    c.add_argument("--nodes-override", type=int, default=None, help="테스트 주입")
    c.add_argument("--load-override", type=float, default=None, help="테스트 주입")
    c.add_argument("--servers-ledger-override", dest="servers_ledger_override",
                   type=_ledger_override_arg, default=None,
                   help="★A3-b 테스트 주입 — 원장 텍스트 JSON {\"lane\":\"<cys ps 출력>\","
                        "\"depts\":{\"<sock>\":\"<출력>\"}}. 지정 시 라이브 `cys ps` 조회를 "
                        "전부 생략한다(결정론). 잘못된 JSON=EX_USAGE 64")
    c.add_argument("--dept-roster-override", dest="dept_roster_override", default=None,
                   type=_roster_override_arg,
                   help="테스트 주입 — 부서 로스터 JSON {active,seats,errors,depts}(라이브 "
                        "`cys status --json --socket` 조회 전부 생략 · 잘못된 JSON=EX_USAGE 64)")
    c.add_argument("--rate-check", action="store_true",
                   help="opt-in: 5h rate 사용률 soft 경고 축 추가(env CYS_GATE_RATE=1과 동등)")
    c.add_argument("--rate-soft", type=float, default=80.0, help="rate 5h used_pct soft 임계")
    c.add_argument("--rate-override", default=None,
                   help="테스트 주입 — usage-accounts JSON(accounts 배열) 직접 주입")
    c.add_argument("--require-context", dest="require_context", action="store_true",
                   help="Phase 1 §2-5: context 미제공 시 context_unmeasured 를 soft(exit 1)로 "
                        "격상 — verify 실행 경로(completion-guard) 전용. 기본 동작 불변")
    c.add_argument("--formation-size", dest="formation_size", type=int, default=None,
                   help="★T9(W6): 이 레인 편성 크기 — env CYS_FORMATION_BUDGET(정수)과 둘 다 "
                        "있을 때만 곱셈 예산 축 발화(투영=nodes+부서수×크기 · 초과=hard). "
                        "어느 한쪽 부재=완전 무동작(기존 호출자 회귀 0)")
    c.set_defaults(fn=cmd_check)

    c = sub.add_parser("classify")
    c.set_defaults(fn=cmd_classify)

    c = sub.add_parser("enforce")
    c.add_argument("--servers-hard", type=int, default=3)
    c.add_argument("--kill", action="store_true",
                   help="실제 kill 집행 — 미지정 시 dry-run(대상 목록만)")
    c.add_argument("--min-age", dest="min_age", type=int, default=0,
                   help="기동 N초 미만 서버 보호(watchdog 45초 규칙)")
    c.add_argument("--notify", default=None,
                   help="실제 kill 발생 시에만 이 역할로 1줄 push(무사건 무push)")
    c.add_argument("--pids", type=int, nargs="*", default=None, help="테스트 주입(임계 우회)")
    c.set_defaults(fn=cmd_enforce)

    try:
        a = p.parse_args(argv)
    except _UsageExit:
        return EXIT_USAGE
    # ★내부 예외를 exit 1('soft_warn')로 흘리지 않는다 — '측정 실패'와 '자원 경고'는 다른 사실이다.
    #   traceback 은 stderr 로 남기고(진단 보존) 계약 채널(stdout)은 오염시키지 않는다.
    try:
        return a.fn(a)
    except SystemExit:
        raise
    except Exception as e:                      # noqa: BLE001 — 최상위 경계에서 타입 분리가 목적
        import traceback
        traceback.print_exc()
        sys.stderr.write("[resource-gate] 내부 예외로 측정 실패(exit %d=EX_SOFTWARE): %s\n"
                         % (EXIT_INTERNAL, e))
        return EXIT_INTERNAL


def self_test():
    """A13 타입드 exit 회귀 배터리 — 측정 없이 결정론(부작용 0)."""
    fails = []

    def chk(cond, msg):
        if not cond:
            fails.append(msg)

    import io
    import contextlib
    # ★A3: 모든 check 호출에 고정 로스터를 주입 — 이 머신의 라이브 부서 소켓(dept-1 등)이 판정에
    #   스며들면 self-test 가 비결정론이 된다(nodes hard 가 좌석 수에 따라 18·21·30… 으로 움직임).
    # ★WP-7 N 동형 밀폐: 신설 CPU 축·부트 유예도 라이브(이 기계의 ps·boot-epoch mtime)를 읽으면
    #   self-test 가 비결정론이 된다 — 바쁜 기계에서 fleet_cpu 가 hard 로 튀면 기존 핀이 전부 흔들린다.
    #   `--fleet-cpu-override 0.0`(축은 살아 있되 값 고정) + `--boot-elapsed-override 99999`(유예 밖).
    det_boot = ["--boot-elapsed-override", "99999"]           # 유예 밖 고정
    det = ["--fleet-cpu-override", "0.0"] + det_boot          # + 함대 CPU 값 고정
    ro = ["--dept-roster-override", '{"active":0,"seats":0,"errors":[],"depts":[]}'] + det
    # ① 미지 서브커맨드 → EX_USAGE(64), EXIT_HARD(2) 와 분리
    with contextlib.redirect_stderr(io.StringIO()):
        rc = main(["definitely-not-a-subcommand"])
    chk(rc == EXIT_USAGE, "미지 서브커맨드가 EX_USAGE(64) 아님: rc=%r" % rc)
    chk(rc != EXIT_HARD, "사용오류가 hard_block(2)로 오독됨 — argparse↔EXIT_HARD 충돌 잔존")
    # ② 미지 플래그도 동일
    with contextlib.redirect_stderr(io.StringIO()):
        rc = main(["check", "--no-such-flag"] + ro)
    chk(rc == EXIT_USAGE, "미지 플래그가 EX_USAGE(64) 아님: rc=%r" % rc)
    # ③ 인자 없음(subparser required) → EX_USAGE
    with contextlib.redirect_stderr(io.StringIO()):
        rc = main([])
    chk(rc == EXIT_USAGE, "서브커맨드 부재가 EX_USAGE(64) 아님: rc=%r" % rc)
    # ④ 정상 판정 경로는 무회귀(override 주입으로 측정 대체 — 라이브 ps 무의존)
    with contextlib.redirect_stdout(io.StringIO()):
        rc = main(["check", "--servers-override", "0", "--nodes-override", "0",
                   "--load-override", "0.0"] + ro)
    chk(rc == EXIT_ALLOW, "정상 allow 경로 회귀: rc=%r" % rc)
    with contextlib.redirect_stdout(io.StringIO()):
        rc = main(["check", "--servers-override", "99", "--nodes-override", "0",
                   "--load-override", "0.0"] + ro)
    chk(rc == EXIT_HARD, "servers hard 경로 회귀: rc=%r" % rc)
    # ⑤ 내부 예외 → EX_SOFTWARE(70), 'soft'(1) 오분류 아님
    import types
    ns = types.SimpleNamespace(fn=lambda _a: (_ for _ in ()).throw(RuntimeError("boom")))
    saved = _GateArgumentParser.parse_args
    try:
        _GateArgumentParser.parse_args = lambda self, argv=None: ns
        with contextlib.redirect_stderr(io.StringIO()):
            rc = main(["check"] + ro)
    finally:
        _GateArgumentParser.parse_args = saved
    chk(rc == EXIT_INTERNAL, "내부 예외가 EX_SOFTWARE(70) 아님: rc=%r" % rc)
    chk(rc != EXIT_SOFT, "내부 예외가 soft_warn(1)로 오분류 — 측정 실패↔자원 경고 융합 잔존")
    # ⑥ 계약 채널: --json 의 stdout 은 순수 JSON(진단 혼입 0)
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(io.StringIO()):
        main(["check", "--json", "--servers-override", "0", "--nodes-override", "0",
              "--load-override", "0.0"] + ro)
    try:
        json.loads(buf.getvalue().strip())
    except ValueError as e:
        fails.append("--json stdout 이 순수 JSON 아님: %s" % e)
    # ⑦ ★B1(§2-5): --require-context + context 미제공 → soft(exit 1)·trips 는 비어 있음
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = main(["check", "--json", "--require-context", "--servers-override", "0",
                   "--nodes-override", "0", "--load-override", "0.0"] + ro)
    chk(rc == EXIT_SOFT, "--require-context 미제공이 soft(1) 아님: rc=%r" % rc)
    try:
        doc = json.loads(buf.getvalue().strip())
        chk(doc.get("trips") == [], "--require-context 승격이 trips 를 오염: %r" % doc.get("trips"))
        chk("context_unmeasured" in (doc.get("warnings") or []),
            "--require-context 미제공에 context_unmeasured 경고 부재")
    except ValueError as e:
        fails.append("--require-context --json 파싱 실패: %s" % e)
    # ⑧ --require-context + context 제공 → 종전 판정 그대로(42=allow · 61=hard)
    with contextlib.redirect_stdout(io.StringIO()):
        rc = main(["check", "--require-context", "--context", "42", "--servers-override", "0",
                   "--nodes-override", "0", "--load-override", "0.0"] + ro)
    chk(rc == EXIT_ALLOW, "--require-context+context 42 가 allow 아님: rc=%r" % rc)
    with contextlib.redirect_stdout(io.StringIO()):
        rc = main(["check", "--require-context", "--context", "61", "--servers-override", "0",
                   "--nodes-override", "0", "--load-override", "0.0"] + ro)
    chk(rc == EXIT_HARD, "--require-context+context 61 이 hard(2) 아님: rc=%r" % rc)
    # ⑨ 플래그 없는 기존 호출 = 기본 동작 불변(context 미제공 = allow · 회귀 0)
    with contextlib.redirect_stdout(io.StringIO()):
        rc = main(["check", "--servers-override", "0", "--nodes-override", "0",
                   "--load-override", "0.0"] + ro)
    chk(rc == EXIT_ALLOW, "플래그 없는 context 미제공이 allow 아님(기본 동작 회귀): rc=%r" % rc)

    # ⑩ ★T9(P3-1·R3-P03-1) 곱셈 편성 예산 축 4형상 — 발화는 (--formation-size ∧ env 정수) 둘 다일 때만.
    #    결정론 확보: --formation-size 0 이면 투영 = nodes_override + depts×0 = nodes_override 라
    #    부서 수와 무관하게 판정이 고정된다(밀폐) — A3 이후 ro 주입(active 0)으로 이중 밀폐.
    base_argv = ["check", "--servers-override", "0", "--nodes-override", "0",
                 "--load-override", "0.0"] + ro
    saved_budget = os.environ.pop("CYS_FORMATION_BUDGET", None)
    try:
        # (a) 둘 다 + 예산 내(투영 0 ≤ 0) → allow
        os.environ["CYS_FORMATION_BUDGET"] = "0"
        with contextlib.redirect_stdout(io.StringIO()):
            rc = main(base_argv + ["--formation-size", "0"])
        chk(rc == EXIT_ALLOW, "예산 내(0≤0)가 allow 아님: rc=%r" % rc)
        # (b) 둘 다 + 예산 초과(투영 0 > -1) → hard(2) + trips 에 formation_budget
        os.environ["CYS_FORMATION_BUDGET"] = "-1"
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = main(base_argv + ["--json", "--formation-size", "0"])
        chk(rc == EXIT_HARD, "예산 초과(0>-1)가 hard(2) 아님: rc=%r" % rc)
        try:
            doc = json.loads(buf.getvalue().strip())
            chk(any(t.get("metric") == "formation_budget" for t in doc.get("trips") or []),
                "예산 초과 trips 에 formation_budget 부재: %r" % doc.get("trips"))
        except ValueError as e:
            fails.append("예산 축 --json 파싱 실패: %s" % e)
        # (c) 플래그만(env 부재) → 완전 무동작 = allow
        os.environ.pop("CYS_FORMATION_BUDGET", None)
        with contextlib.redirect_stdout(io.StringIO()):
            rc = main(base_argv + ["--formation-size", "0"])
        chk(rc == EXIT_ALLOW, "env 부재인데 예산 축이 발화(무동작 계약 위반): rc=%r" % rc)
        # (d) env 만(플래그 부재) → 완전 무동작 = allow (기존 호출자 3곳 회귀 0)
        os.environ["CYS_FORMATION_BUDGET"] = "-1"
        with contextlib.redirect_stdout(io.StringIO()):
            rc = main(base_argv)
        chk(rc == EXIT_ALLOW, "플래그 부재인데 예산 축이 발화(기존 호출자 회귀): rc=%r" % rc)
        # (e) env 비정수 + 플래그 → 판정 무접촉(allow) + warning 가청화
        os.environ["CYS_FORMATION_BUDGET"] = "abc"
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = main(base_argv + ["--json", "--formation-size", "0"])
        chk(rc == EXIT_ALLOW, "비정수 env 가 판정을 오염: rc=%r" % rc)
        try:
            doc = json.loads(buf.getvalue().strip())
            chk(any(w.startswith("formation_budget_env_invalid") for w in doc.get("warnings") or []),
                "비정수 env 가 침묵(warning 부재): %r" % doc.get("warnings"))
        except ValueError as e:
            fails.append("예산 축(비정수 env) --json 파싱 실패: %s" % e)
        # (f) nodes 미측정(ps 실패 형상) → 예외 금지(70 방지·None 무발화) — 순수 함수 직접 핀
        os.environ["CYS_FORMATION_BUDGET"] = "1"
        ns2 = types.SimpleNamespace(formation_size=5)
        fb, fw = _formation_budget_check({"nodes": None, "active_depts": 3}, ns2)
        chk(fb is None and fw is None,
            "nodes=None(ps 실패)에서 예산 축이 무발화가 아님(70 위험): %r/%r" % (fb, fw))
    finally:
        if saved_budget is None:
            os.environ.pop("CYS_FORMATION_BUDGET", None)
        else:
            os.environ["CYS_FORMATION_BUDGET"] = saved_budget

    # ⑪ ★A3(SURVEY A4·B6-2 · PREP #8) 부서 로스터 축 — 활성=데몬 응답 · hard=max(18, 12+Σ좌석) · 실패=soft.
    def _check_json(argv):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(io.StringIO()):
            rc = main(argv)
        try:
            return rc, json.loads(buf.getvalue().strip())
        except ValueError as e:
            fails.append("A3 --json 파싱 실패(%r): %s" % (argv[-1], e))
            return rc, {}

    quiet = ["check", "--json", "--servers-override", "0", "--load-override", "0.0"] + det
    # (a) 좌석 합 10(4+6 · 2부서) → hard 22 = 12+10 · measured 에 active_depts/dept_seats/depts 노출
    r10 = ('{"active":2,"seats":10,"errors":[],'
           '"depts":[{"name":"a","seats":4},{"name":"b","seats":6}]}')
    rc, doc = _check_json(quiet + ["--nodes-override", "0", "--dept-roster-override", r10])
    mm = doc.get("measured") or {}
    chk(rc == EXIT_ALLOW and mm.get("nodes_hard_effective") == 22,
        "좌석 10 로스터의 nodes hard 가 22(=12+10) 아님: rc=%r m=%r" % (rc, mm))
    chk(mm.get("active_depts") == 2 and mm.get("dept_seats") == 10
        and len(mm.get("depts") or []) == 2,
        "measured 에 active_depts/dept_seats/depts 가 로스터대로 없음: %r" % mm)
    # (b) 좌석 3(1부서) → 12+3=15 < floor 18 → 18 유지(floor 규약 불변 · 구 '+5/부서' 잔존이면 18 그대로라
    #     이 핀만으로는 못 가르지만, (d) 의 9좌석 케이스가 가른다)
    rc, doc = _check_json(quiet + ["--nodes-override", "0", "--dept-roster-override",
                                   '{"active":1,"seats":3,"errors":[],"depts":[{"name":"a","seats":3}]}'])
    chk((doc.get("measured") or {}).get("nodes_hard_effective") == NODES_HARD_DEFAULT,
        "좌석 3 로스터가 floor 18 을 깎음: %r" % (doc.get("measured") or {}).get("nodes_hard_effective"))
    # (c) 응답 실패 부서 → measure_errors 에 dept(x) · verdict soft(exit 1) · trips 는 비어 있음(트립 아님)
    rerr = '{"active":0,"seats":0,"errors":["dept(x)"],"depts":[]}'
    rc, doc = _check_json(quiet + ["--nodes-override", "0", "--dept-roster-override", rerr])
    chk(rc == EXIT_SOFT and doc.get("verdict") == "soft_warn",
        "부서 응답 실패가 soft(1) 아님(조용한 allow 회귀): rc=%r verdict=%r" % (rc, doc.get("verdict")))
    chk("dept(x)" in ((doc.get("measured") or {}).get("measure_errors") or []),
        "measure_errors 에 dept(x) 부재: %r" % (doc.get("measured") or {}).get("measure_errors"))
    chk(doc.get("trips") == [] and "measure_error:dept(x)" in (doc.get("warnings") or []),
        "부서 응답 실패가 trips 를 오염하거나 warnings 에 없음: trips=%r warnings=%r"
        % (doc.get("trips"), doc.get("warnings")))
    # (d) 실데이터 형상(SURVEY A4 · dept-1 CSO 22:05 · evidence G3-dept1-status-raw.json 좌석 9):
    #     1부서 좌석 9 + nodes 15 → hard 21 = max(18, 12+9) · 15 ≥ soft 12 → soft(exit 1) · 15 < 21.
    #     판별 지점: nodes 20 은 구 규칙(1부서 → 18)에서 hard(20≥18) / 신 규칙 soft(20<21).
    #     음성 대조: 로스터 0(ro) + nodes 20 → floor 18 → hard(2) — 좌석이 판정을 바꿈을 실증.
    r9 = '{"active":1,"seats":9,"errors":[],"depts":[{"name":"dept-1","seats":9}]}'
    rc, doc = _check_json(quiet + ["--nodes-override", "15", "--dept-roster-override", r9])
    node_c = next((c for c in doc.get("checks") or [] if c.get("metric") == "nodes"), {})
    chk(rc == EXIT_SOFT and node_c.get("hard") == 21 and node_c.get("level") == "soft",
        "1부서 9좌석+nodes 15 가 soft/hard 21 아님: rc=%r nodes=%r" % (rc, node_c))
    rc, _doc = _check_json(quiet + ["--nodes-override", "20", "--dept-roster-override", r9])
    chk(rc == EXIT_SOFT, "1부서 9좌석+nodes 20 이 soft(1) 아님(구 규칙 hard 18 잔존?): rc=%r" % rc)
    rc, _doc = _check_json(quiet + ["--nodes-override", "20"] + ro)
    chk(rc == EXIT_HARD, "로스터 0 + nodes 20 이 hard(2) 아님(floor 18 회귀 · 음성 대조): rc=%r" % rc)
    rc, _doc = _check_json(quiet + ["--nodes-override", "21", "--dept-roster-override", r9])
    chk(rc == EXIT_HARD, "1부서 9좌석+nodes 21 이 hard(2) 아님(21≥21): rc=%r" % rc)
    # (e) --nodes-hard 명시는 로스터보다 우선(종전 규약 유지)
    rc, doc = _check_json(quiet + ["--nodes-override", "0", "--nodes-hard", "7",
                                   "--dept-roster-override", r10])
    chk((doc.get("measured") or {}).get("nodes_hard_effective") == 7,
        "--nodes-hard 명시가 로스터 동적값에 밀림: %r"
        % (doc.get("measured") or {}).get("nodes_hard_effective"))
    # (f) 잘못된 주입 JSON → EX_USAGE(64) — 조용한 라이브 폴백도, 내부 예외(70)도 아니다
    with contextlib.redirect_stderr(io.StringIO()):
        rc = main(["check", "--dept-roster-override", "{not json"])
    chk(rc == EXIT_USAGE, "잘못된 --dept-roster-override 가 EX_USAGE(64) 아님: rc=%r" % rc)
    with contextlib.redirect_stderr(io.StringIO()):
        rc = main(["check", "--dept-roster-override", "[1,2]"])
    chk(rc == EXIT_USAGE, "비객체 --dept-roster-override 가 EX_USAGE(64) 아님: rc=%r" % rc)
    # (g) 순수 함수: override 정규화(부분 dict) · 라이브 조회 0회(glob 에 소켓이 있어도 subprocess 무호출)
    def _no_live(*_x, **_k):
        raise AssertionError("override 단락 실패 — 라이브 cys 호출 발생")
    saved_run, saved_glob = subprocess.run, glob.glob
    try:
        subprocess.run = _no_live
        glob.glob = lambda *_x, **_k: ["/nonexistent/cys-dept-z/cys.sock"]
        r = _dept_roster({"seats": 10})
    finally:
        subprocess.run, glob.glob = saved_run, saved_glob
    chk(r == {"active": 0, "seats": 10, "errors": [], "depts": []}, "override 정규화 실패: %r" % r)
    # (h) 라이브 경로 시뮬(subprocess 대역): a=응답(좌석 2 + exited 1) · b=rc 1 → active 1 · seats 2 ·
    #     errors [dept(b)] · argv 는 기존 표면 `cys status --json --socket <sock>` 의 list 형(shell 0).
    def _fake_run(argv, **kw):
        chk(argv[:4] == ["cys", "status", "--json", "--socket"] and kw.get("shell") is not True
            and kw.get("timeout") == DEPT_STATUS_TIMEOUT,
            "부서 조회 argv 형상 이탈(list argv · shell 0 · timeout 계약): %r %r" % (argv, kw))
        if argv[4].endswith("cys-dept-a/cys.sock"):
            return subprocess.CompletedProcess(argv, 0, stdout=json.dumps(
                {"surfaces": [{"exited": False}, {"exited": False}, {"exited": True}]}), stderr="")
        return subprocess.CompletedProcess(argv, 1, stdout="", stderr="connect: refused")
    # ★A3-c 축 분리: 리스너 프로브는 별도 축이다(전용 핀 = tests/test_resource_gate.py
    #   TestSocketProbe · 실소켓 픽스처). 여기서 재는 것은 '데몬 응답 → 좌석 계상' 이므로
    #   가짜 경로에서 프로브가 먼저 죽지 않게 통과로 고정한다(이 시뮬의 대상이 아니다).
    _g = globals()
    saved_run, saved_glob, saved_probe = subprocess.run, glob.glob, _g["_socket_listening"]
    try:
        subprocess.run = _fake_run
        _g["_socket_listening"] = lambda _p: True
        glob.glob = lambda *_x, **_k: ["/h/.local/state/cys-dept-b/cys.sock",
                                       "/h/.local/state/cys-dept-a/cys.sock"]
        r = _dept_roster()
    finally:
        subprocess.run, glob.glob = saved_run, saved_glob
        _g["_socket_listening"] = saved_probe
    chk(r == {"active": 1, "seats": 2, "errors": ["dept(b)"], "depts": [{"name": "a", "seats": 2}]},
        "라이브 경로 시뮬 로스터 불일치: %r" % r)
    # ★A3-c: 프로브가 죽었다고 판정하면 **스폰 0** 으로 그 부서를 오류 계상한다(5s 절감의 본체).
    _spawned = []
    saved_run2, saved_glob2, saved_probe2 = subprocess.run, glob.glob, _g["_socket_listening"]
    try:
        subprocess.run = lambda *a2, **k2: _spawned.append(a2) or subprocess.CompletedProcess(
            a2[0] if a2 else [], 0, stdout="{}", stderr="")
        _g["_socket_listening"] = lambda _p: False
        glob.glob = lambda *_x, **_k: ["/h/.local/state/cys-dept-dead/cys.sock"]
        r2 = _dept_roster()
    finally:
        subprocess.run, glob.glob = saved_run2, saved_glob2
        _g["_socket_listening"] = saved_probe2
    chk(r2 == {"active": 0, "seats": 0, "errors": ["dept(dead)"], "depts": []},
        "프로브 죽음 판정이 오류 계상으로 이어지지 않음: %r" % r2)
    chk(not _spawned, "프로브가 죽음으로 판정했는데 cys status 를 스폰했다(지연 절감 무효)")

    # ⑫ ★WP-7 N(0.14.31) 함대 CPU 축 · load soft 전용 · 부트 유예 — 순수 함수 핀 + 종단 판정.
    ncpu_local = os.cpu_count() or 1
    roster_only = ["--dept-roster-override", '{"active":0,"seats":0,"errors":[],"depts":[]}']
    q = ["check", "--json", "--servers-override", "0", "--nodes-override", "0",
         "--load-override", "0.0"] + roster_only

    def _cj(argv, label):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(io.StringIO()):
            rc = main(argv)
        try:
            return rc, json.loads(buf.getvalue().strip())
        except ValueError as e:
            fails.append("WP-7 --json 파싱 실패(%s): %s" % (label, e))
            return rc, {}

    def _axis(doc, metric):
        return next((c for c in (doc.get("checks") or []) if c.get("metric") == metric), {})

    # (a) 임계 3점: 0.4 ok · 0.5 soft · 1.0 hard (유예 밖)
    for val, want_rc, want_lvl in ((0.4, EXIT_ALLOW, "ok"), (0.5, EXIT_SOFT, "soft"),
                                   (1.0, EXIT_HARD, "hard")):
        rc, doc = _cj(q + det_boot + ["--fleet-cpu-override", str(val)], "fleet=%s" % val)
        chk(rc == want_rc and _axis(doc, "fleet_cpu_ratio").get("level") == want_lvl,
            "fleet_cpu_ratio=%s 가 %s/%s 아님: rc=%r axis=%r"
            % (val, want_rc, want_lvl, rc, _axis(doc, "fleet_cpu_ratio")))
    # (b) 부트 유예 — CPU 축 hard→soft. 경계는 `< 300`(300 은 유예 밖)
    for elapsed, want_rc, want_grace in ((10, EXIT_SOFT, True), (299, EXIT_SOFT, True),
                                         (300, EXIT_HARD, False), (99999, EXIT_HARD, False)):
        rc, doc = _cj(q + roster_only + ["--fleet-cpu-override", "1.5",
                                         "--boot-elapsed-override", str(elapsed)],
                      "grace=%s" % elapsed)
        ax = _axis(doc, "fleet_cpu_ratio")
        chk(rc == want_rc and bool(ax.get("boot_grace")) is want_grace,
            "부트 경과 %ss 유예 판정 이탈: rc=%r(기대 %r) axis=%r" % (elapsed, rc, want_rc, ax))
        chk((doc.get("measured") or {}).get("boot_grace") is (elapsed < BOOT_GRACE_SECS),
            "measured.boot_grace 가 경과초와 불일치(%ss): %r"
            % (elapsed, (doc.get("measured") or {}).get("boot_grace")))
    # (c) ★음성 대조: 유예는 **CPU 축에만**. 같은 유예 창에서 servers hard 는 그대로 hard 다.
    rc, doc = _cj(["check", "--json", "--servers-override", "99", "--nodes-override", "0",
                   "--load-override", "0.0", "--fleet-cpu-override", "0.0",
                   "--boot-elapsed-override", "10"] + roster_only, "grace-non-cpu")
    chk(rc == EXIT_HARD and _axis(doc, "servers").get("level") == "hard"
        and not _axis(doc, "servers").get("boot_grace"),
        "부트 유예가 CPU 아닌 축(servers)까지 완화했다: rc=%r axis=%r" % (rc, _axis(doc, "servers")))
    # (d) load_ratio 는 soft 전용 — 종전이면 hard 였을 값이 soft 다(hard 키는 None)
    rc, doc = _cj(["check", "--json", "--servers-override", "0", "--nodes-override", "0",
                   "--load-override", str(3.0 * ncpu_local)] + ro, "load-soft-only")
    la = _axis(doc, "load_ratio")
    chk(rc == EXIT_SOFT and la.get("level") == "soft" and la.get("hard") is None,
        "load_ratio 3.0×ncpu 가 soft 전용이 아님(구 hard 2.0 잔존?): rc=%r axis=%r" % (rc, la))
    # (d') ★07:07 스냅샷 재생(정본 §4 WP-7 수용 기준) — 그날 hard_block 을 낸 입력은 load1=59 하나다.
    #      그때의 함대 CPU 는 **기록이 없다** — 그래서 0.0 으로 고정해 '호스트 부하 축 단독'의 판정만
    #      재생한다(현재 기계 값을 넣으면 그것은 재생이 아니라 합성이다 · codex R1 #11).
    rc, doc = _cj(["check", "--json", "--servers-override", "0", "--nodes-override", "0",
                   "--load-override", "59"] + ro, "0707-replay")
    chk(rc == EXIT_SOFT and _axis(doc, "load_ratio").get("level") == "soft",
        "07:07 스냅샷(load1=59) 재생이 soft 아님: rc=%r axis=%r" % (rc, _axis(doc, "load_ratio")))
    chk(not [c for c in (doc.get("checks") or []) if c.get("level") == "hard"],
        "07:07 재생에 hard 축이 남아 있다: %r" % (doc.get("trips"),))
    # (e) ★Windows 형상(ps 부재) — checks 에 unavailable 라벨 · measure_errors 무접촉 · exit 계약 불변
    _g2 = globals()
    saved_cpu = _g2["_ps_cpu_lines"]
    try:
        _g2["_ps_cpu_lines"] = lambda: (None, "absent")
        rc, doc = _cj(q + det_boot, "ps-absent")
    finally:
        _g2["_ps_cpu_lines"] = saved_cpu
    ax = _axis(doc, "fleet_cpu_ratio")
    mm2 = doc.get("measured") or {}
    chk(rc == EXIT_ALLOW and ax.get("level") == "unavailable" and ax.get("value") is None,
        "ps 부재에서 fleet 축이 unavailable/exit 불변이 아님: rc=%r axis=%r" % (rc, ax))
    chk(not [e for e in (mm2.get("measure_errors") or []) if "fleet" in e],
        "ps **부재**가 measure_errors 로 새어 Windows exit 계약을 바꿨다: %r"
        % mm2.get("measure_errors"))
    chk(doc.get("trips") == [], "unavailable 축이 trips 를 오염: %r" % doc.get("trips"))
    chk(doc.get("warnings") == ["context_unmeasured"],
        "신설 축이 warnings 를 늘렸다(completion_guard `_soft_kind` 완전일치 계약 파손): %r"
        % doc.get("warnings"))
    # (f) ★음성 대조: ps 는 있는데 **조회가 깨진** 것은 측정 실패다 — 최소 soft(조용한 allow 금지).
    try:
        _g2["_ps_cpu_lines"] = lambda: (None, "failed")
        rc, doc = _cj(q + det_boot, "ps-failed")
    finally:
        _g2["_ps_cpu_lines"] = saved_cpu
    if os.name == "nt":
        chk(rc == EXIT_ALLOW, "Windows 에서 ps 조회 실패가 exit 를 바꿨다: rc=%r" % rc)
    else:
        chk(rc == EXIT_SOFT and "fleet_cpu(ps)" in ((doc.get("measured") or {})
                                                    .get("measure_errors") or []),
            "POSIX 에서 ps 조회 실패가 조용한 allow 로 접혔다: rc=%r m=%r"
            % (rc, (doc.get("measured") or {}).get("measure_errors")))
    # (g) `_fleet_cpu_percent` 순수 핀 — 합산·무매칭 0.0·형상 불일치·함대 행 파손·NaN
    L = [" 10.5 /usr/local/bin/cysd --socket /x",
         "  4.5 /Users/u/.local/bin/claude --foo",
         "  1.0 /usr/bin/mediaanalysisd",
         "  2.0 /opt/serena-mcp-server",
         "  9.9 python3 /w/bin/javis_resource_gate.py check"]
    tot, why = _fleet_cpu_percent(L)
    chk(abs((tot or 0) - 17.0) < 1e-9 and why is None,
        "함대 %%CPU 합이 17.0(cysd10.5+claude4.5+serena2.0) 아님: %r/%r" % (tot, why))
    tot, why = _fleet_cpu_percent(["  1.0 /usr/bin/mediaanalysisd", "  0.0 /sbin/init"])
    chk(tot == 0.0 and why is None,
        "매칭 0건(깨끗한 기계)이 측정 불능으로 오분류: %r/%r" % (tot, why))
    tot, why = _fleet_cpu_percent(["root /sbin/init", "daemon /usr/sbin/cupsd"])
    chk(tot is None and why == "shape",
        "첫 열이 수치가 아닌 형상이 '0%%'로 위장됨: %r/%r" % (tot, why))
    tot, why = _fleet_cpu_percent(["  0.0 /sbin/init", "BROKEN /usr/bin/codex"])
    chk(tot is None and why == "row_unparsed",
        "**함대 행** 파싱 실패가 조용히 건너뛰어 과소계상됨: %r/%r" % (tot, why))
    tot, why = _fleet_cpu_percent(["  0.0 /sbin/init", "nan /usr/local/bin/cysd"])
    chk(tot is None and why == "row_nonfinite",
        "NaN 이 합에 섞여 모든 비교를 거짓으로 만들었다(조용한 allow): %r/%r" % (tot, why))
    # (h) `_boot_elapsed` 순수 핀 — 부재·미래 mtime·정상. 근거 없음은 **유예 없음**이다.
    import shutil as _sh
    import tempfile as _tf
    _bd = _tf.mkdtemp()
    saved_sock = os.environ.get("CYS_SOCKET")
    try:
        os.environ["CYS_SOCKET"] = os.path.join(_bd, "cys.sock")
        el, why = _boot_elapsed()
        chk(el is None and why == "epoch_missing",
            "boot-epoch 부재가 '유예 없음(None)'이 아님: %r/%r" % (el, why))
        _bp = os.path.join(_bd, BOOT_EPOCH_BASENAME)
        with open(_bp, "w", encoding="utf-8") as f:
            f.write("0\n")
        os.utime(_bp, (time.time() + 3600, time.time() + 3600))
        el, why = _boot_elapsed()
        chk(el is None and why == "clock_backwards",
            "미래 mtime(시계 역행)이 '갓 부팅'으로 읽혀 유예가 열렸다: %r/%r" % (el, why))
        os.utime(_bp, (time.time() - 42, time.time() - 42))
        el, why = _boot_elapsed()
        chk(el is not None and 41 <= el <= 60 and why == "ok",
            "정상 mtime 에서 경과초가 안 나옴: %r/%r" % (el, why))
    finally:
        if saved_sock is None:
            os.environ.pop("CYS_SOCKET", None)
        else:
            os.environ["CYS_SOCKET"] = saved_sock
        _sh.rmtree(_bd, ignore_errors=True)
    # (i) ★계측 타당성 음성 대조: 신설 축이 **없던** 코드에서는 이 판정이 성립할 수 없다.
    #     load 축 hard 키가 None 이라는 사실 자체가 'soft 전용 강등'의 유일한 기계 증거다.
    chk(_axis(_cj(q + det_boot + ["--fleet-cpu-override", "0.0"], "shape")[1],
              "load_ratio").get("hard") is None,
        "load_ratio 에 hard 임계가 남아 있다(강등 미적용)")

    # ⑬ ★codex 위임 검체(R2)에서 살아남은 반례 — 두 개는 실결함이었고 셋은 '상속된 성질' 이다.
    #    상속된 것도 **핀으로 박아** 다음 사람이 모르고 바꾸지 못하게 한다(사실의 기록).
    # (a) 합계 오버플로: 각 항이 유한해도 합이 inf 면 측정 실패다(JSON 에 Infinity 를 흘리지 않는다)
    tot, why = _fleet_cpu_percent(["1e308 /opt/codex", "1e308 /opt/claude"])
    chk(tot is None and why == "sum_overflow",
        "유한한 항들의 합이 inf 인데 측정 성공으로 나갔다(비표준 JSON): %r/%r" % (tot, why))
    # (b) 진단 함수는 판정을 죽이지 않는다 — `_fleet_cpu_top` 이 터져도 exit 70 이 되면 안 된다
    saved_top = _g["_fleet_cpu_top"]
    saved_cpu2 = _g["_ps_cpu_lines"]
    try:
        _g["_ps_cpu_lines"] = lambda: (["50.0 /usr/local/bin/cysd"], None)
        _g["_fleet_cpu_top"] = lambda *_a, **_k: (_ for _ in ()).throw(RuntimeError("boom"))
        rc, doc = _cj(["check", "--json", "--servers-override", "0", "--nodes-override", "0",
                       "--load-override", "0.0"] + roster_only + det_boot, "top-raise")
    finally:
        _g["_fleet_cpu_top"] = saved_top
        _g["_ps_cpu_lines"] = saved_cpu2
    chk(rc != EXIT_INTERNAL and (doc.get("measured") or {}).get("fleet_cpu_ratio") is not None,
        "진단용 top 의 예외가 정상 측정을 exit 70 으로 만들었다: rc=%r" % rc)
    # (c) 상속 핀 — 명령줄 전체 검색이라 '에이전트가 아닌 프로세스'도 걸린다(과대계상=차단 방향).
    #     기존 `nodes` 축과 **같은 성질**이다: 고치려면 두 축을 함께 고쳐야 한다(별 백로그).
    chk(_fleet_cpu_percent(["25 /usr/bin/python3 /tmp/agy/report.py"]) == (25.0, None),
        "상속 핀 이탈: NODE_PATTERNS 의 명령줄 전체 검색 성질이 바뀌었다(nodes 축과 분기?)")
    chk(_fleet_cpu_percent(["25 grep claude"]) == (25.0, None),
        "상속 핀 이탈: 인자에만 든 `claude` 매칭 성질이 바뀌었다(nodes 축과 분기?)")
    # (d) 상속 핀 — 자기 제외도 `_count_matching` 과 같은 부분문자열 검사다(과소계상 방향).
    chk(_fleet_cpu_percent(["0 /sbin/init",
                            "25 /opt/codex --output /tmp/javis_resource_gate.log"]) == (0.0, None),
        "상속 핀 이탈: 자기 제외의 부분문자열 성질이 바뀌었다(nodes 축과 분기?)")
    # (e) 밀폐 확인: fleet override 를 주면 CPU 조회 스폰이 0 이다
    _spawn2 = []
    saved_cpu3 = _g["_ps_cpu_lines"]
    try:
        _g["_ps_cpu_lines"] = lambda: _spawn2.append(1) or (None, "absent")
        _cj(base_argv + ["--json"], "override-no-spawn")
    finally:
        _g["_ps_cpu_lines"] = saved_cpu3
    chk(not _spawn2, "--fleet-cpu-override 를 줬는데 ps 조회가 일어났다(밀폐 파괴): %r" % _spawn2)

    if fails:
        print("javis_resource_gate self-test FAIL:")
        for f in fails:
            print("  ✗ " + f)
        return 1
    print("javis_resource_gate self-test OK — A13 타입드 exit 9종"
          "(EX_USAGE 3·정상 2·EX_SOFTWARE 2·계약 채널 1·충돌 분리 1)"
          " + B1 --require-context 5종(미제공 soft·trips 비오염·context 제공 allow/hard·"
          "무플래그 기본 동작 불변)"
          " + T9 편성 예산 축 6종(예산 내 allow·초과 hard·env/플래그 단독 무동작·"
          "비정수 env 가청화·nodes 미측정 무예외)"
          " + WP-7 codex 위임 반례 6종(합계 오버플로·진단 예외 격리·상속 3핀·override 밀폐)"
          " + WP-7 함대CPU/부트유예 21종(임계 3점·유예 4점+비CPU축 음성대조·load soft전용·"
          "07:07 재생·ps부재 unavailable 4핀·ps실패 soft·순수 합산 5종·boot-epoch 3종)"
          " + A3 부서 로스터 8종(좌석 합산 22·floor 유지·응답 실패 soft·실데이터 9좌석 soft/hard 판별+"
          "음성 대조·--nodes-hard 우선·잘못된 주입 64·override 단락·라이브 경로 대역)")
    return 0


if __name__ == "__main__":
    if "--self-test" in sys.argv[1:]:
        sys.exit(self_test())
    sys.exit(main())
