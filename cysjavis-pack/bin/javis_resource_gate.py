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
               `ps -axo pid,pcpu,command` %CPU 합 / 100 / ncpu(분율). soft 0.5 · hard 1.0.
               차단은 '우리가 실제로 회수할 수 있는 자원'에만 건다. 계측 한계는 `_fleet_owner` 주석.
               ★소유권은 **argv0(실행 주체)** 로 판정한다 — 명령줄 아무 데나 든 이름이 아니다.
               ★자기 제외는 **PID**(os.getpid())로 한다 — 명령줄 부분문자열이 아니다.
               ★hard 는 **연속 900초**를 넘겨 유지되지 않는다(봉인표 ③ — 다른 부서의 부하가 전멸
                 부서의 복구를 무기한 막지 않게). 넘으면 그 포화가 끝날 때까지 soft(권고)다.
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
  /--fleet-cpu-override/--fleet-cpu-hold-override/--boot-elapsed-override
  (신설 실수 인자는 전부 **유한성 검사**를 거친다 — nan/inf 는 EX_USAGE 64. nan 은 모든 비교를
   거짓으로 만들어 조용한 allow 를 내고, json.dumps 는 비표준 NaN/Infinity 를 계약 채널로 흘린다)
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
import hashlib
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

# ── ★R2(리뷰 blocking): '실행 형상' 상수의 **정본은 preflight 하나**다 ──
# R1 노트는 "preflight 의 상수를 그대로 가져왔다"고 적었지만 코드는 값을 **복사**해 다시 정의했다
# (리뷰 지적 그대로다 — 정본을 고쳐도 이 축에 반영되지 않는다). 이제 실제로 import 한다.
# ★가져오는 것은 **상수(사실)** 이고, 판정기(`_is_claude_command`)는 공유하지 않는다.
#   근거(실측 2026-09-08): preflight 의 strict 판정기는 argv0 basename 을 **소문자로 접어**
#   `_CLAUDE_EXE_NAMES` 와 비교한다(javis_preflight.py:6798) → `/Applications/Claude.app/Contents/
#   MacOS/Claude` 와 `Claude Helper (Renderer)` 계열 10+행이 전부 claude 로 인정된다. 그 판정기를
#   그대로 쓰면 macOS GUI 앱 전체가 **함대 CPU 합**에 실려 B2형 오탐(조직 기동 거부)이 재발한다.
#   preflight 쪽에서는 그 관대함이 옳다(그 축은 '신뢰 시드를 미룰 claude 후보'를 **놓치지 않는** 것이
#   목적이라 과대계상이 안전 방향이다). 두 축의 안전 방향이 반대라 판정 규칙은 갈라지고, 갈라지는
#   지점을 여기 명시한다 — 상수(무엇이 우리 이름·마커·확장자인가)는 하나, 규칙(무엇을 소유로
#   인정하는가)은 축마다. self-test (d1) 이 상수 동일성을, (d2) 가 규칙 분기를 각각 핀한다.
# ★D1 경로 가드(형태 규약 = javis_radio.py:64-66 · javis_wakeup.py:47-57 과 동일):
#   팩 `bin/` 스크립트를 **다른 cwd 에서** 부르면 형제 모듈이 sys.path 에 없다. append + 중복 검사
#   (insert(0) 금지 — 목적은 발견이지 stdlib precedence 강등이 아니다). bin/tests/test_import_guard.py 가 검증한다.
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

try:
    import javis_preflight as _PF            # 같은 팩 bin/ — 순환 import 없음(preflight 는 이 모듈을 안 쓴다)
except Exception:                            # noqa: BLE001 — import 실패가 게이트를 죽이면 안 된다
    _PF = None                               # (죽으면 exit 1 → formation 이 'proceed' 로 읽어 축이 통째로 사문)


def _pf_const(name, fallback):
    """preflight 상수 우선 · 부재 시 리터럴 폴백. 폴백이 정본과 어긋나면 self-test (d1) 이 잡는다."""
    v = getattr(_PF, name, None) if _PF is not None else None
    return fallback if v is None else v


_CLAUDE_EXE_NAMES = _pf_const("_CLAUDE_EXE_NAMES", ("claude", "claude.exe", "claude.cmd"))
_CLAUDE_INSTALL_MARKER = _pf_const("_CLAUDE_INSTALL_MARKER", "/claude/versions/")
_CLAUDE_NPM_MARKER = _pf_const("_CLAUDE_NPM_MARKER", "/claude-code/")
_JS_SUFFIXES = _pf_const("_JS_SUFFIXES", (".js", ".mjs", ".cjs"))
_PF_JS_RUNTIME_NAMES = _pf_const("_JS_RUNTIME_NAMES", ("node", "node.exe", "bun", "bun.exe"))

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
# ★R1(리뷰 반영 2026-09-08) — **소유권 판정을 명령줄 부분문자열에서 실행 주체로 바꿨다.**
#   종전 판본은 `NODE_PATTERNS`(명령줄 전체 정규식 검색)를 그대로 빌려 썼고, 그 성질을 "기존
#   nodes 축과 같으니 여기서만 고치지 않는다"는 이유로 핀까지 박아 뒀다. 리뷰 2명이 독립적으로
#   **그 상속이 신설 hard 축의 정확성을 보증하지 않는다**고 지적했고, 그 지적이 옳다:
#     · `python3 /tmp/agy/report.py`(우리와 무관한 다중 스레드 작업) → `\bagy\b` 매칭 → ratio 1.0 → hard.
#       그러면 이 WP 가 고치려던 사고(B2 = **우리 소관 아닌 부하로 조직 기동 거부**)가 그대로 재발한다.
#     · `grep claude` 도 같다(인자에만 든 이름).
#     · 반대로 `/opt/codex --output /tmp/javis_resource_gate.log` 는 자기 제외의 부분문자열 검사에
#       걸려 **진짜 함대 프로세스가 0으로 누락**됐고, 네이티브 claude 의 버전 경로 실행
#       (`~/.local/share/claude/versions/2.1.261`)은 `claude(\s|$)` 가 놓쳤다.
#   그래서 이 축은 **자기 축 전용 소유권 규칙**을 갖는다(`_fleet_owner`): 명령줄의 아무 토큰이 아니라
#   **실행 주체**(argv0, 인터프리터/런처면 그것이 실행하는 스크립트)의 basename·경로 세그먼트만 본다.
#   자기 제외도 명령줄이 아니라 **PID**(`os.getpid()`)로 한다.
#   ★nodes 축은 **무수정**이다(정본: 기존 오류·계수 경로 현행 유지) — 두 축이 다른 모집단을 세는 것은
#     의도된 분기이고, 그 사실은 노트(§패턴 정밀도)와 아래 상수 주석에 남긴다. 계수 축(nodes)의
#     과대계상은 종전부터 `hard-overcount` 밸브가 받아 주지만, CPU 축에는 그 밸브가 없어서
#     오탐 하나가 곧바로 조직 기동 거부다 — 정밀도 요구가 애초에 다르다.
#   ★틀리는 방향: 이 정밀화는 **과소계상**(모르는 실행 형상을 함대로 안 셈) 쪽으로 틀릴 수 있다.
#     그 방향의 귀결은 '차단하지 않음' = 0.14.31 이전과 동일한 상태다. 반대로 과대계상의 귀결은
#     조직 기동 거부(B2 재발)라 되돌리기가 훨씬 비싸다.
# ★하위호환 보존용(이 축은 **더 이상 이 패턴으로 판정하지 않는다** — `_fleet_owner` 가 판정한다).
#   외부 임포터가 있을 수 있어 상수는 남기되, 여기 이름을 고쳐도 축의 판정은 바뀌지 않는다.
FLEET_CPU_PATTERNS = NODE_PATTERNS + [r"(^|/)cysd(\s|$)", r"\bserena\b"]
# ── 실행 주체 → 함대 소유자(진단 라벨). 판정에 쓰는 것은 '있다/없다' 하나다. ──
# ★규칙의 출처: `javis_preflight._is_claude_command(strict=True)` 와 그 상수(`_CLAUDE_EXE_NAMES` ·
#   `_CLAUDE_INSTALL_MARKER="/claude/versions/"` · `_CLAUDE_NPM_MARKER="/claude-code/"` ·
#   `_JS_RUNTIME_NAMES`). 저장소에 이미 실측으로 다듬어진 '실행 형상' 판정기가 있는데 새로 지어내면
#   두 곳이 다른 사실을 말하게 된다 — 같은 사실 위에 둔다.
# ★대소문자를 **구분**한다: 우리 CLI 이름은 전부 소문자다. 소문자로 접으면 macOS **GUI 앱 번들**
#   (`/Applications/Claude.app/Contents/MacOS/Claude` · Helper 프로세스 다수)이 전부 함대로 잡힌다 —
#   실측 2026-09-08 02:1x 이 기계에서 `Claude`/`Claude Helper` 행 10+개(최대 4.2%). 그것은 우리가
#   좌석 스폰을 위해 회수할 수 있는 프로세스가 아니다(B2 형 오탐). 번들 경로도 따로 배제한다.
FLEET_EXE_NAMES = {"claude": "claude", "codex": "codex", "agy": "agy",
                   "gemini": "gemini", "cysd": "cysd", "serena": "serena"}
# 제거 후 비교(판정기는 플랫폼과 무관하게 순수하게 둔다). ★R2: 확장자 목록의 정본은
# preflight `_CLAUDE_EXE_NAMES` = ("claude","claude.exe","claude.cmd") 다 — 거기서 파생한다.
FLEET_EXE_SUFFIXES = tuple(sorted({os.path.splitext(n)[1].lower()
                                   for n in _CLAUDE_EXE_NAMES if os.path.splitext(n)[1]})) \
    or (".exe", ".cmd")
_APP_BUNDLE_MARKER = ".app/contents/"          # macOS GUI 앱 번들 — CLI 함대가 아니다(실측 반례)
# argv0 자체가 우리 설치 레이아웃인 형상(basename 이 버전 문자열이라 이름이 안 남는다)
#   ★실측(2026-09-08 02:28 이 기계)으로 넣은 것: codex 는 wrapper 아래에 vendor 네이티브를 여러 개
#     띄우고 그 basename 이 `codex` 가 아닌 것도 있다 —
#     `…/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex-code-mode-host`.
#     그래서 `/@openai/codex/` 패키지 경로를 argv0 마커로 둔다(그 아래에 남의 프로그램이 살 일은 없다).
# ★R2: 첫 마커는 preflight `_CLAUDE_INSTALL_MARKER` 파생(값 복사 금지 — 정본 하나).
FLEET_ARGV0_MARKERS = ((_CLAUDE_INSTALL_MARKER, "claude"),  # ~/.local/share/claude/versions/<ver>
                       ("/.codex/bin/", "codex"),         # codex vendor native(설치 레이아웃 A)
                       ("/@openai/codex/", "codex"))      # codex vendor native(npm 레이아웃 · 실측)
# JS 번들 형상 — **`.js` 번들 경로일 때만** 소유권을 인정한다(preflight strict 와 동형):
#   `tail -f /x/claude-code/debug.log` 처럼 인자에 패키지 세그먼트가 실린 비-에이전트를 배제한다.
# ★R2: claude 마커는 preflight `_CLAUDE_NPM_MARKER` 파생 · `_JS_SUFFIXES` 는 위에서 import 했다.
FLEET_JS_BUNDLE_MARKERS = ((_CLAUDE_NPM_MARKER, "claude"), ("/@openai/codex/", "codex"),
                           ("/gemini-cli/", "gemini"))
# ★R2: 런타임 이름도 preflight `_JS_RUNTIME_NAMES`(node/node.exe/bun/bun.exe) 파생 + `deno`.
#   deno 를 **더하는** 이유: preflight 의 그 목록은 'claude 번들을 실행할 수 있는 argv0' 이라는
#   좁은 목적의 것이고, 이 축은 우리 CLI 전체(codex·gemini 포함)의 런타임 형상을 본다. 더하는
#   방향의 귀결은 언랩을 한 번 더 시도하는 것뿐이고, 언랩 자체가 R2 에서 '첫 비옵션 토큰 하나'
#   로 좁혀졌다(아래 `_fleet_owner`) — 오탐이 늘지 않는다.
FLEET_JS_RUNTIMES = frozenset([os.path.splitext(n)[0].lower() for n in _PF_JS_RUNTIME_NAMES]) \
    | frozenset(("deno",))
FLEET_PY_RUNTIMES = frozenset(("python", "py", "pypy"))
# 온디맨드 런처 — 이 자리의 positional 은 **정의상 실행할 프로그램 이름**이다.
#   실형상(javis_preflight.SERENA_STDIO_ARGS): `uvx --python 3.13 --from serena-agent==1.5.3 serena start-mcp-server …`
FLEET_RUNNERS = frozenset(("uv", "uvx", "npx", "bunx", "pipx", "npm", "pnpm", "yarn"))
FLEET_RUNNER_PROGRAMS = {
    "claude": "claude", "@anthropic-ai/claude-code": "claude",
    "codex": "codex", "@openai/codex": "codex",
    "gemini": "gemini", "gemini-cli": "gemini", "@google/gemini-cli": "gemini",
    "serena": "serena", "serena-agent": "serena",
}
# ★R2(리뷰 minor · B2 재발 계열): 하위 명령은 **런처마다 다르다**. 종전엔 `run` 을 무조건
#   건너뛰어 `npm run codex`(= package.json 스크립트 이름이 우리 CLI 와 같은 저장소)와
#   `yarn codex` 가 codex 로 계상됐다 — 실행된 것은 우리 CLI 가 아니라 그 저장소의 스크립트다.
#   값 = (허용 하위 명령 집합, 하위 명령 **필수** 여부).
#   npm/pnpm/yarn 의 맨 positional 은 정의상 **스크립트 이름**이라 `exec`/`dlx` 를 거친 형상만
#   프로그램 실행으로 인정한다(틀리는 방향 = 미계상 = 차단 안 함).
#   ★codex 위임 검체 D4: `uv`/`pipx` 도 **하위 명령 필수**다 — `uv codex`·`pipx codex` 는 실행
#     형상이 아닌데(그 런처에 그런 호출 규약이 없다) 종전 표는 codex 로 셌다(과대계상).
# ★R3(판정자 핀 2 · codex): 하위 명령은 **집합이 아니라 체인**이다. 종전의 `seen_sub` 는 허용
#   하위명령을 **반복해서** 소비해 `npm exec exec codex` · `uv run run codex` 의 두 번째 토큰
#   (=실행되는 프로그램/스크립트 이름)을 건너뛰고 그 뒤의 codex 를 실행 주체로 승격시켰다.
#   체인표는 그 반복을 원천적으로 막는다 — `("exec","exec")` 는 어떤 체인의 접두도 아니다.
#   ★`uv tool run` 은 `uvx` 의 정본 형태이고(uvx = uv tool run 별칭), `uv tool uvx` 는 이 기계의
#     실측 형상이다. 둘 다 체인으로 명시한다 — 전역 '1회 소비' 규칙은 전자를 놓친다(codex).
#   ★`pnpm dlx exec codex` 의 `exec` 는 프로그램/패키지 자리다 → 보수적 결과는 None(codex).
FLEET_RUNNER_CHAINS = {
    "uv":   frozenset((("run",), ("tool", "run"), ("tool", "uvx"), ("x",))),
    "uvx":  frozenset(),
    "npx":  frozenset(),
    "bunx": frozenset(),
    "pipx": frozenset((("run",),)),
    "npm":  frozenset((("exec",),)),
    "pnpm": frozenset((("exec",), ("dlx",))),
    "yarn": frozenset((("exec",), ("dlx",))),
}
# ★R3(codex): 옵션 표는 **런처별**이다. 약어를 런처 전체에 합집합으로 적용하면 안 된다 —
#   `-p` 는 npx 에서 `--package`(값), npm 에서 `--parseable`(불리언), uv 에서 `--python`(값)이다.
#   합집합으로 두면 그중 한 해석이 다른 런처에서 값을 먹거나 안 먹어 실행 주체가 어긋난다.
#   `-c`/`--call` 은 **셸 명령 문자열 모드**라 값 소비가 아니라 **언랩 종료**다(abort).
#   표에 없는 이름은 `unknown` → 언랩 포기(미계상 = 차단 안 함). 리콜의 대가는 정직 고지 대상이다.
_RUNNER_OPTS_EMPTY = {"value": frozenset(), "bool": frozenset(), "abort": frozenset()}
_UV_OPTS = {
    "value": frozenset(("-p", "--python", "--from", "--with", "--with-requirements", "-c",
                        "--constraints", "--index", "--index-url", "--extra-index-url",
                        "--directory", "--project", "--cache-dir", "--refresh-package",
                        "--config-file", "--color",
                        # ★R2(수렴): 기준선 복원분 — 전부 값을 먹는다.
                        "--with-editable", "--python-preference", "--index-strategy",
                        "--keyring-provider", "--exclude-newer", "--resolution",
                        "--prerelease", "--link-mode", "--default-index", "--find-links",
                        "--upgrade-package", "--allow-insecure-host", "--no-binary-package",
                        "--no-build-package", "--extra", "--group", "--no-group",
                        "--only-group", "--package")),
    "bool": frozenset(("-q", "--quiet", "-v", "--verbose", "-n", "--no-cache", "--refresh",
                       "--native-tls", "--offline", "--isolated", "--system", "--preview",
                       "--no-project", "--frozen", "--locked", "--no-sync", "--no-config",
                       "-h", "--help", "-V", "--version",
                       # ★R2(수렴 · 실측 `uv run --active serena` · `uv run --no-progress serena`).
                       "--active", "--no-active", "--no-progress", "--dev", "--no-dev",
                       "--all-extras", "--no-extra", "--all-groups", "--no-default-groups",
                       "--all-packages", "--no-editable", "--exact", "--inexact",
                       "--managed-python", "--no-managed-python", "--no-binary", "--no-build",
                       "--no-sources", "--reinstall", "-U", "--upgrade",
                       "--compile-bytecode", "--no-compile-bytecode")),
    # ★`--script` 는 **프로그램 자리를 옮긴다**: `uv run --script /tmp/x.py serena` 의 실행 대상은
    #   x.py 이고 serena 는 그 인자다. 값 옵션으로 두면(=값 하나 건너뛰기) 그 뒤 토큰이 실행 주체로
    #   승격돼 기준선의 오탐(`--script=/tmp/x.py serena` → serena)이 되살아난다. 그래서 abort 다.
    "abort": frozenset(("--script", "--module")),
}
FLEET_RUNNER_OPTS = {
    "uv": _UV_OPTS, "uvx": _UV_OPTS,
    "npx": {
        "value": frozenset(("-p", "--package", "--userconfig", "--cache", "--registry",
                            "--node-options", "--prefix", "-C", "--loglevel", "--workspace",
                            "--node-version")),
        "bool": frozenset(("-y", "--yes", "--no", "--no-install", "-q", "--quiet", "--silent",
                           "--prefer-online", "--prefer-offline", "--offline",
                           "--ignore-existing", "-h", "--help",
                           # ★R2(수렴 · 실측 `npx --no-audit codex`).
                           "--no-audit", "--no-fund", "--no-progress", "--no-update-notifier",
                           "-g", "--global", "-d", "--verbose")),
        "abort": frozenset(("-c", "--call")),      # 셸 명령 문자열 — 뒤는 그 셸의 몫이다
    },
    "npm": {
        # ★R2(수렴 · 리뷰 major 표 1행): `npm exec --package @openai/codex codex` 는 실형상인데
        #   `--package` 가 표에 없어 기준선이 세던 codex 를 통째로 잃었다.
        "value": frozenset(("-w", "--workspace", "-C", "--prefix", "--registry", "--userconfig",
                            "--cache", "--node-options", "--package", "--loglevel",
                            "--node-version", "--globalconfig")),
        "bool": frozenset(("-p", "--parseable", "-y", "--yes", "-g", "--global", "-s", "--silent",
                           "--workspaces", "--no-workspaces", "-q", "--quiet", "--offline",
                           "--prefer-online", "--prefer-offline", "--ignore-scripts",
                           "-h", "--help", "--no-audit", "--no-fund", "--no-progress",
                           "--if-present", "--include-workspace-root", "-d", "--verbose")),
        "abort": frozenset(("-c", "--call")),
    },
    "pnpm": {
        "value": frozenset(("-C", "--dir", "-F", "--filter", "--package", "--registry",
                            "--store-dir", "--reporter", "--loglevel", "--config-dir")),
        "bool": frozenset(("-r", "--recursive", "-w", "--workspace-root", "-y", "--yes",
                           "-s", "--silent", "--offline", "--prefer-offline", "-h", "--help",
                           "--no-progress", "--ignore-scripts", "--stream", "--aggregate-output")),
        "abort": frozenset(("-c", "--shell-mode")),
    },
    "yarn": {
        "value": frozenset(("--cwd", "--registry", "--cache-folder", "--modules-folder",
                            "-p", "--package")),
        "bool": frozenset(("-s", "--silent", "--no-lockfile", "--offline", "--prefer-offline",
                           "-y", "--yes", "-h", "--help",
                           # ★R2(수렴 · 실측 `yarn dlx --quiet codex`).
                           "-q", "--quiet", "--verbose", "--no-progress", "--non-interactive",
                           "--frozen-lockfile", "--ignore-engines", "--ignore-scripts")),
        "abort": frozenset(),
    },
    "pipx": {
        "value": frozenset(("--python", "--spec", "--index-url", "--pip-args", "--suffix")),
        "bool": frozenset(("-q", "--quiet", "--verbose", "-y", "--yes", "--force", "--no-cache",
                           "--include-deps", "-h", "--help")),
        "abort": frozenset(),
    },
    "bunx": {
        "value": frozenset(("--cwd", "--config")),
        "bool": frozenset(("-b", "--bun", "--silent", "--no-install", "-y", "--yes",
                           "-h", "--help", "-v", "--version", "--revision", "--no-summary")),
        "abort": frozenset(),
    },
}
# 하위호환 상수(외부 임포터 보존) — 판정은 위 두 표가 한다. 모듈 안에서는 아무도 안 쓴다.
# ★R2(수렴 · 리뷰 minor "하위호환 상수가 하위호환이 아니다"): `FLEET_RUNNER_VALUE_OPTS` 는 종전
#   **긴 값 옵션 집합**이었다. 런처별 표의 `value` 를 그대로 합집합하면 `-p`·`-C`·`-w` 같은 짧은
#   옵션까지 섞여 뜻이 조용히 바뀐다(같은 이름이 다른 것을 가리킨다). 긴 옵션만 남겨 뜻을 보존한다 —
#   짧은 옵션은 런처마다 해석이 갈리므로(`-p` = npx 값 · npm 불리언) 애초에 합집합이 성립하지 않는다.
FLEET_RUNNER_RULES = {k: (frozenset(c[0] for c in v if c), bool(v))
                      for k, v in FLEET_RUNNER_CHAINS.items()}
FLEET_RUNNER_VALUE_OPTS = frozenset(o for opt in FLEET_RUNNER_OPTS.values()
                                    for o in opt["value"] if o.startswith("--"))
FLEET_RUNNER_SUBCMDS = frozenset().union(*[frozenset(t for c in v for t in c)
                                           for v in FLEET_RUNNER_CHAINS.values()])
FLEET_RUNNER_SCAN_MAX = 40       # 인자 스캔 상한(비용 상한 — 12 는 `node --flag`×12 형상에서 짧았다)
# 런타임이 **코드 문자열**을 받는 모드. 이때는 뒤 토큰이 실행 대상이 아니므로 아예 언랩하지 않는다
# (`python3 -c 'print(1)' /tmp/serena` 가 serena 로 오인되던 길 · codex 위임 검체 4).
FLEET_CODE_MODE_FLAGS = frozenset(("-c", "-e", "--eval", "-p", "--print", "-m"))
# ★R2(리뷰 major): 값이 **붙은** 짧은 옵션(`-cprint(1)`)·클러스터(`-uc`)도 코드 모드다. 종전엔
#   그 토큰이 '그냥 옵션' 으로 건너뛰어져 뒤의 **데이터 인자**가 실행 주체로 승격됐다
#   (`python3 -cprint(1) /tmp/serena` → serena). 문자 집합은 런타임별로 가른다:
#   python 은 `-c`(명령)·`-m`(모듈), node/bun/deno 는 `-e`(eval)·`-p`(print).
_PY_CODE_MODE_LETTERS = "cm"
_JS_CODE_MODE_LETTERS = "ep"
_LONG_CODE_MODE_FLAGS = frozenset(("--eval", "--print", "--command", "--module"))
# ★R2(codex C1): **값을 먹는** 런타임 옵션. 값이 실행 대상보다 먼저 오는 실형상
#   (`node --require preload.js /x/@openai/codex/bin/codex.js` · `python3 -W ignore /x/bin/serena` ·
#   `python3 -X dev …`)에서 그 값을 실행 대상으로 오인하면 **진짜 함대를 놓친다**(미계상).
#   모르는 옵션의 값은 여전히 못 가리므로 그때는 미계상으로 접는다(정밀도 우선 원칙).
_PY_VALUE_SHORT = "WX"          # `-W ignore` · `-X dev` (붙여 쓰면 값이 클러스터 안에 있다)
_JS_VALUE_SHORT = "rC"          # `-r preload.js` · `-C condition`
_PY_VALUE_LONG = frozenset(("--check-hash-based-pycs",))
# ★R2(수렴 · 리뷰 major "기준선이 세던 형상이 사라졌다"): 표를 늘리는 것이 수리다. 여기 있는 이름은
#   전부 **값을 먹는** node/bun 옵션이라, 등재는 그 값을 실행 대상으로 오인하지 않게 만들면서
#   (`node --diagnostic-dir /tmp/codex /tmp/report.js` → 계속 None) 그 **뒤의 진짜 실행 대상**을
#   되찾는다(`node --diagnostic-dir /tmp/x /x/bin/codex` → codex). 두 방향 모두 검체로 박았다.
_JS_VALUE_LONG = frozenset((
    "--require", "--import", "--loader", "--experimental-loader",
    "--conditions", "--max-old-space-size", "--inspect-port",
    "--unhandled-rejections", "--stack-size", "--dns-result-order",
    "--max-http-header-size", "--max-semi-space-size", "--v8-pool-size",
    "--diagnostic-dir", "--cpu-prof-dir", "--cpu-prof-name", "--cpu-prof-interval",
    "--heap-prof-dir", "--heap-prof-name", "--heap-prof-interval",
    "--heapsnapshot-signal", "--heapsnapshot-near-heap-limit",
    "--report-directory", "--report-filename", "--report-signal",
    "--test-reporter", "--test-reporter-destination", "--test-name-pattern",
    "--test-shard", "--test-concurrency", "--test-timeout",
    "--redirect-warnings", "--disable-warning", "--disable-proto",
    "--env-file", "--env-file-if-exists", "--title", "--icu-data-dir",
    "--tls-cipher-list", "--tls-keylog", "--openssl-config",
    "--trace-event-categories", "--trace-event-file-pattern",
    "--secure-heap", "--secure-heap-min", "--use-largepages",
    "--snapshot-blob", "--build-snapshot-config", "--watch-path",
    "--allow-fs-read", "--allow-fs-write", "--experimental-policy", "--policy-integrity",
    "--experimental-default-type", "--input-type", "--experimental-test-isolation",
    "--tsconfig-override", "--experimental-config-file", "--inspect-publish-uid"))
# ★`=` 형이라고 무조건 자족적이지 않은 이름들 — **실행 모드 자체**를 바꾼다. `node --run=build`
#   뒤의 토큰은 Node 의 스크립트가 아니라 그 태스크의 인자다. 이 목록 밖의 `=` 형은 값 소비가
#   이미 끝난 형태이므로 이름을 몰라도 자족적 불리언으로 통과시킨다(기준선 규칙 복원 · 리콜).
_LONG_MODE_CHANGE_FLAGS = frozenset(("--run", "--eval", "--print", "--command", "--module"))
# ★R3(판정자 핀 2 · codex major B2 재발): **해석하지 못한 옵션 뒤의 토큰은 소유권의 양성 근거가
#   아니다.** 종전엔 미열거 긴 옵션을 '불리언' 으로 가정하고 건너뛰어, 그 옵션의 **값**이 첫
#   비옵션 경로 토큰이 되어 실행 주체로 승격됐다 —
#     `node --diagnostic-dir /tmp/codex /tmp/report.js` → codex(실제 실행 대상은 report.js) ·
#     `node --cpu-prof-dir …` · `node --test-reporter-destination /tmp/serena …`.
#   그 한 행의 CPU 가 그대로 함대 합에 실려 기동 거부(hard)를 만든다.
#   규칙: 옵션은 **값 소비·불리언·코드모드** 셋 중 하나로 *확정된 것만* 통과시키고, 그 밖은
#   그 지점에서 언랩을 포기한다(`None` = 미계상 = 차단 안 함 = 0.14.31 이전과 같은 상태).
#   ★대가(정직 고지 · codex 지적): `node --enable-source-maps /x/bin/codex` 처럼 표에 없는 진짜
#     불리언 옵션 뒤의 우리 CLI 는 미계상된다. 표를 늘리는 것이 수리이고, 늘릴 때는 **정상 형상과
#     '값이 codex 인 비함대 형상' 을 함께** 검체에 넣는다. 완전한 리콜은 포기한 것이다.
#   ★`=` 형이라고 무조건 안전하지 않다(codex): `node --run=build /tmp/codex` 는 값 소비가 끝났어도
#     **실행 모드**가 바뀌어 뒤 토큰이 Node 의 스크립트가 아니다. 그래서 `=` 형도 이름이 표에
#     있어야 통과한다(`--run` 은 어느 표에도 없다 → 포기).
_JS_BOOL_LONG = frozenset((
    "--enable-source-maps", "--experimental-vm-modules", "--experimental-modules",
    "--experimental-json-modules", "--experimental-specifier-resolution",
    "--no-warnings", "--trace-warnings", "--trace-uncaught", "--trace-exit",
    "--throw-deprecation", "--trace-deprecation", "--no-deprecation", "--pending-deprecation",
    "--preserve-symlinks", "--preserve-symlinks-main", "--abort-on-uncaught-exception",
    "--zero-fill-buffers", "--frozen-intrinsics", "--force-node-api-uncaught-exceptions-policy",
    "--check", "--interactive", "--version", "--help", "--expose-gc", "--no-experimental-fetch",
    # ★R2(수렴 · 리뷰 major): 기준선이 세던 **평범한 불리언** 형상의 복원. 값을 안 먹으므로
    #   등재는 뒤 토큰을 실행 대상으로 되살릴 뿐 새 오탐을 만들지 않는다.
    "--inspect", "--inspect-brk", "--inspect-wait", "--inspect-brk-node",
    "--experimental-strip-types", "--no-experimental-strip-types",
    "--experimental-transform-types", "--experimental-detect-module",
    "--no-experimental-detect-module", "--experimental-require-module",
    "--no-experimental-require-module", "--experimental-import-meta-resolve",
    "--experimental-network-imports", "--experimental-wasm-modules",
    "--experimental-permission", "--experimental-sqlite", "--experimental-websocket",
    "--experimental-test-coverage", "--experimental-global-webcrypto",
    "--experimental-abortcontroller", "--experimental-top-level-await",
    "--experimental-shadow-realm", "--experimental-webstorage", "--experimental-addon-modules",
    "--no-addons", "--no-global-search-paths", "--no-force-async-hooks-checks",
    "--no-node-snapshot", "--node-memory-debug", "--jitless", "--napi-modules",
    "--report-uncaught-exception", "--report-on-fatalerror", "--report-on-signal",
    "--report-compact", "--track-heap-objects", "--trace-sync-io", "--trace-tls",
    "--trace-sigint", "--trace-atomics-wait",
    "--use-openssl-ca", "--use-bundled-ca", "--openssl-legacy-provider",
    "--enable-fips", "--force-fips",
    "--insecure-http-parser", "--disallow-code-generation-from-strings",
    "--force-context-aware", "--build-snapshot", "--prof", "--perf-basic-prof",
    "--perf-prof", "--cpu-prof", "--heap-prof",
    "--tls-min-v1.0", "--tls-min-v1.1", "--tls-min-v1.2", "--tls-min-v1.3",
    "--tls-max-v1.2", "--tls-max-v1.3",
    "--enable-network-family-autoselection", "--no-network-family-autoselection",
    "--allow-child-process", "--allow-worker", "--allow-addons", "--allow-wasi",
    "--watch", "--watch-preserve-output", "--test", "--test-only", "--test-force-exit",
    "--test-update-snapshots",
    # bun 고유(실측 형상 `bun --smol /x/bin/codex`) — 전부 값을 안 먹는다.
    "--smol", "--hot", "--bun", "--no-install", "--silent", "--minify",
    "--no-clear-screen", "--no-summary"))
_PY_BOOL_LONG = frozenset(("--help", "--version", "--help-env", "--help-xoptions",
                           "--help-all"))
# 짧은 **불리언** 문자. 값 문자(`_*_VALUE_SHORT`)·코드 문자(`_*_CODE_MODE_LETTERS`)와 셋이
# 서로소여야 한다 — 어느 표에도 없는 문자는 '미지' 이고, 미지 클러스터는 포기한다.
#   python3: `-b -B -d -E -h -i -I -O -P -q -R -s -S -u -v -V -x`(값 없음 · 공식 목록)
#   node/bun/deno: `-c(--check) -h -i -v`(값 없음). `-e`/`-p` 는 코드, `-r`/`-C` 는 값이다.
_PY_BOOL_SHORT = "bBdEhiIOPqRsSuvVx"
_JS_BOOL_SHORT = "chiv"
_PY_VERSIONED_RE = re.compile(r"^python\d+(\.\d+)?$")
# ★쉘(`sh`/`bash`/`zsh`)·`env` 는 **언랩하지 않는다**: 데몬은 로그인셸 우산으로 좌석을 띄우지만
#   그 셸이 exec/spawn 한 실제 좌석은 **자기 ps 행**을 따로 갖는다(셸을 세면 이중계상이고, 셸의
#   명령줄 문자열에 든 'claude' 는 소유권 근거가 아니다 — 실측: `sh -c S=/tmp/claude-501/…` 행).
# ★여기에 NODE_EXCLUDE_PATTERNS 를 걸지 않는다: 그 제외(codex vendor native)는 **계수 인플레이션**
#   (codex 1노드 = wrapper + native 2프로세스)의 교정이다. CPU **합**에서는 vendor native 가 codex 가
#   실제로 태운 CPU 의 본체라, 빼면 함대 소비를 체계적으로 과소평가한다(축이 목적을 못 재는 방향).
FLEET_CPU_SOFT_DEFAULT = 0.5     # 함대가 전 코어의 50% 상당을 평균 점유 — 경고
FLEET_CPU_HARD_DEFAULT = 1.0     # 전 코어 100% 상당 — 착수 거부
FLEET_CPU_TOP_N = 3              # hard 시 남기는 상위 기여자 수(회수 대상 지목 — 봉인표 ③)
# ★R1(봉인표 ③ 직접 방어) — **hard 보류 상한**. 이 축의 hard 는 연속 15분을 넘겨 유지되지 않는다.
#   왜 필요한가(리뷰 blocking): 이 축은 **호스트 전체 함대 CPU** 를 재는데, 그 판정을 각 부서의
#   복구 허가에 그대로 적용한다. 그래서 '부서 A 전멸 · 부서 B 가 계속 바쁨' 형상에서 A 의 복구가
#   **무기한** 보류될 수 있다(A 의 죽은 좌석이 CPU 0 인 것은 B 의 기여를 지우지 않는다). Linux 의
#   `ps` %CPU 는 수명 평균이라 그 보류가 더 끈끈하다.
#   규칙: 이 축이 **연속으로** hard 인 시간이 이 상한을 넘으면 그 뒤로는 hard 를 soft 로 내린다
#   (`hold_expired`). 축이 hard 가 아닌 판정이 한 번이라도 나오면 래치는 지워지고 시계는 0 이다.
#   ★만료 뒤 래치를 **재무장하지 않는다**: 재무장하면 만료 창을 어느 호출자(완료 검증 게이트 등)가
#     먼저 소비해 버리고, 정작 복구가 필요한 편성 호출은 다시 hard 를 만난다 — 유계가 특정 호출자에게
#     보장되지 않는다. 만료는 '포화가 끝날 때까지 이 축은 권고(soft)' 라는 상태이고, 그것이 모든
#     호출자에게 동일하게 보인다.
#   ★계약 문면 정정(R2 리뷰 minor — 종전 문면 "포화 1회당 최대 15분 차단"은 장치보다 강했다):
#     이 장치가 실제로 보장하는 것은 **"이 축의 hard 는 첫 hard 관측 시각으로부터 벽시계
#     `FLEET_CPU_HARD_MAX_HOLD_SECS` 안에서만 유지된다"** 하나다. 게이트 호출 사이의 공백도 그
#     벽시계에 들어가므로, 관측 공백이 상한보다 크면 **그 포화는 처음부터 만료 상태로 관측된다**
#     (= 한 번도 차단하지 않는다 · 축의 사문화). 그 사실을 사유 `held_stale`/`expired_stale` 로
#     표기해 침묵시키지 않는다.
#     그 방향(덜 막음)을 **의도적으로 택했다**: 공백 뒤 재무장을 허용하면 첫 관측 기준의 절대
#     벽시계 상한이 사라져(재무장 시각부터 다시 900초) 봉인표 ③(전멸 부서의 복구 보류)의 유계가
#     약해진다 — codex R2 B1 의 반례 시각열이 그것이다. 유계 보존 > 축 실효.
FLEET_CPU_HARD_MAX_HOLD_SECS = 900.0
FLEET_HOLD_RECORD_V = 1           # 래치 레코드 스키마 판(구 형식 = bare float 도 읽는다)
FLEET_CPU_HOLD_BASENAME = "fleet-cpu-hard-since"
FLEET_HOLD_FUTURE_SLACK_S = 2.0   # 이만큼까지의 '미래' 저장값은 시계 역행이 아니라 반올림으로 본다
# 래치 저장 루트는 **팩 관례**(`CYS_STATE_DIR` ‖ `~/.cys/state`)다 — 데몬 상태 디렉터리
# (`~/.local/state/cys`)에는 쓰지 않는다(그쪽은 바이너리 소유). 테스트는 `CYS_STATE_DIR` 로 격리한다.
CYS_DIR_DEFAULT = "~/.cys"
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
def _is_windows_host():
    """이 셸이 **Windows 호스트** 위인가 — 인터프리터의 `os.name` 하나로 판정하지 않는다.

    ★R1(리뷰 minor): 종전 판정은 `os.name != "nt"` 였다. 그러면 Git Bash 에서 MSYS/Cygwin 파이썬
      (`os.name == "posix"`)이 CYS_PY 로 잡히고 PATH 앞에 MSYS `ps.exe`(=`-axo` 미지원)가 있는
      흔한 조합에서, 매 호출이 `measure_errors: fleet_cpu(ps)` → 최소 soft 가 된다. 그 결과
      `javis_completion_guard._soft_kind` 가 proceed_unmeasured → skip_soft 로 바뀌어 **완료 검증이
      영구 skip** 된다. 정본의 Windows 예외는 '이 플랫폼엔 이 축이 없다'는 뜻이지 인터프리터 종류가
      아니다.

    ★R2(리뷰 major · R1 반박 기각): R1 노트는 "`MSYSTEM` 하나로 판정하지 않는다"고 적었지만 코드는
      **그것 하나로** True 를 냈다. 그러면 POSIX 기계에 `MSYSTEM=MINGW64` 가 (상속·오설정으로) 떠
      있기만 해도 `ps` 조회 실패가 `measure_errors` 를 못 타고 **조용한 allow(exit 0)** 가 된다 —
      P-ORCH-1 위반이고, 이 축이 통째로 사라진 것을 아무도 모른다. `MSYSTEM` 은 상속되는 문자열일
      뿐 플랫폼 증명이 아니므로 **혼자서는 판정하지 못하게** 한다: Windows 커널 위임을 말해 주는
      독립 신호(`WINDIR`/`SYSTEMROOT`/`OS=Windows_NT` — Git Bash 는 Windows 환경을 상속하므로 전부
      있고, 순정 POSIX 에는 없다)가 **함께** 있어야 Windows 호스트로 접는다.
      틀리는 방향: 이 조임은 'Windows 인데 POSIX 로 봄'(=측정 실패가 loud soft) 쪽으로 틀릴 수 있고
      그 귀결은 완료 검증 skip 이다 — 반대(조용한 allow)보다 낫다."""
    if os.name == "nt":
        return True
    if sys.platform in ("msys", "cygwin"):
        return True
    if not os.environ.get("MSYSTEM"):
        return False
    return any(os.environ.get(k) for k in ("WINDIR", "SYSTEMROOT")) \
        or os.environ.get("OS", "").lower() == "windows_nt"


# ps 가 **플래그를 거부**한 형상(구현이 `-axo` 를 모른다)의 관용 문구. 이것은 '측정 실패'가 아니라
# '이 ps 로는 이 축을 못 잰다'는 플랫폼 능력 사실이라, 어느 플랫폼에서든 `unsupported` 로 접는다.
_PS_FLAG_REJECT_RE = re.compile(
    r"(unknown|invalid|illegal|unrecognized)\s+(option|argument|flag)|\bbad\s+option\b", re.I)
# `usage:` 하나만으로는 접지 않는다(codex R1-2): 진짜 호출 결함도 usage 를 낸다. Windows 호스트에서만
# 보조 신호로 인정한다 — 그 플랫폼에서는 어차피 이 축이 없는 것과 같고(exit 계약 불변) 다른
# 플랫폼에서는 '측정 실패' 로 남아 최소 soft 로 신호된다.
_PS_USAGE_RE = re.compile(r"^\s*usage:", re.I | re.M)


def _ps_cpu_lines():
    """`ps -axo pid,pcpu,command` 의 **헤더 제외** 줄들 → `(lines|None, reason|None)`.

    reason ∈ None(성공) · "absent"(`ps` 실행파일 자체가 없다) · "unsupported"(있는데 이 플래그를
    모른다 — MSYS/BusyBox) · "failed"(있고 플래그도 아는데 조회가 실패).
    ★이 셋을 가르는 것이 이 함수의 존재 이유다(codex R1 #4 · R1 리뷰 minor): 정본의 Windows 예외는
      **축이 없는 플랫폼**에 주어진 것이지 '조회 실패'에 주어진 게 아니다. 하나로 뭉개면 POSIX 에서
      ps 조회가 깨졌는데도 축이 조용히 사라져 exit 0(조용한 allow)이 난다 — P-ORCH-1 위반. 반대로
      셋을 다 '측정 실패'로 보면 MSYS 설치가 상시 soft 가 되어 exit 계약이 바뀐다.
    ★왜 `_ps_lines` 를 재활용하지 않는가: 그쪽은 `pid,command` 계약이고 `_count_matching` ·
      `_server_procs` · `_ledger_servers` 가 그 열 형상을 전제한다. 열을 바꾸면 그 셋이 전부 pcpu 를
      명령줄의 일부로 읽는다 — 축 하나를 얻자고 기존 세 축의 계수를 흔드는 거래는 나쁘다.
    ★열이 `pid,pcpu,command` 인 이유(정본 §4 WP-7 의 `pcpu,command` 에서 **의도적 이탈** · 노트 기록):
      자기 제외를 명령줄 부분문자열이 아니라 **PID** 로 하려면 pid 열이 있어야 한다. 종전 판본의
      부분문자열 제외는 출력 파일명에 이 모듈 이름이 들어간 진짜 함대 프로세스를 통째로 뺐다
      (`/opt/codex --output /tmp/javis_resource_gate.log` → 정상 측정값 0 · 리뷰 blocking).
    ★`_ps_lines` 와 달리 **rc 를 본다**: `-axo` 를 모르는 구현은 예외를 던지지 않고 rc≠0 + 빈 stdout 을
      내는데, 그것을 `[]` 로 접으면 '함대 CPU 0%' 라는 **거짓 측정**이 된다."""
    try:
        # ★R2(리뷰 major): `errors="replace"` 가 없으면 디코딩 불가 바이트 하나가
        #   `UnicodeDecodeError`(=ValueError)를 올리고, 그것이 아래 except 두 개에 안 잡혀
        #   **최상위 경계까지 올라가 exit 70** 이 된다 — Windows 분류(unavailable)에 닿지도 못하고
        #   stdout 이 아예 없어서 소비자는 '내부 오류'로 읽는다(formation 은 재시도 후 자원 판정
        #   없이 진행). ps 출력의 pid·pcpu 열은 ASCII 라 대체문자가 파싱을 바꾸지 않는다.
        p = subprocess.run(["ps", "-axo", "pid,pcpu,command"], capture_output=True,
                           text=True, errors="replace", timeout=10)
    except FileNotFoundError:
        return None, "absent"            # Windows 기본 — 이 플랫폼엔 이 축이 없다
    except (subprocess.SubprocessError, OSError, ValueError):
        # ValueError = 디코딩 실패 계열(`errors="replace"` 로 막았지만 대역·구현 차이의 잔여 경로).
        return None, "failed"            # 타임아웃·권한 등 — 측정 실패다
    if p.returncode != 0:
        diag = (p.stderr or "") + "\n" + (p.stdout or "")
        if _PS_FLAG_REJECT_RE.search(diag) or (_is_windows_host() and _PS_USAGE_RE.search(diag)):
            return None, "unsupported"   # 이 ps 는 `-axo` 를 모른다 = 축 부재와 같은 사실
        return None, "failed"
    lines = (p.stdout or "").splitlines()[1:]
    if not lines:
        return None, "failed"            # 헤더뿐 = 아무 프로세스도 못 봤다(형상 이상)
    return lines, None


def _fleet_exe_name(token):
    """ps 토큰 → `(슬래시 정규화 경로, basename(.exe/.cmd 제거))`. **대소문자 보존**. 순수."""
    raw = (token or "").strip("\"'").replace("\\", "/")
    base = raw.rsplit("/", 1)[-1]
    for suf in FLEET_EXE_SUFFIXES:
        if base.lower().endswith(suf):
            base = base[: -len(suf)]
            break
    return raw, base


def _fleet_code_mode(tok, js):
    """이 토큰이 런타임의 **코드 문자열/모듈 모드** 스위치인가 — 순수.

    참이면 뒤 토큰은 실행 파일이 아니므로 언랩을 중단한다. 붙은 값(`-cprint(1)`)과
    클러스터(`-uc`)까지 본다(R2 리뷰 major — 종전엔 그 둘이 '평범한 옵션' 으로 건너뛰어져
    **데이터 인자**가 실행 주체로 승격됐다)."""
    if tok in FLEET_CODE_MODE_FLAGS:
        return True
    if tok.startswith("--"):
        return tok.split("=", 1)[0] in _LONG_CODE_MODE_FLAGS
    if tok.startswith("-") and len(tok) > 1:
        # ★codex 위임 검체 D3: 클러스터를 **왼쪽부터** 훑되 값을 먹는 문자에서 **멈춘다** —
        #   그 뒤는 옵션 문자가 아니라 그 옵션의 **값**이다. 통째로 `any()` 하면
        #   `-Wmodule` 의 m · `-rpreload.js` 의 p · `-Cdevelopment` 의 e 가 코드모드로 읽혀
        #   **진짜 함대를 놓친다**(미계상).
        code = _JS_CODE_MODE_LETTERS if js else _PY_CODE_MODE_LETTERS
        val = _JS_VALUE_SHORT if js else _PY_VALUE_SHORT
        for ch in tok[1:]:
            if ch in code:
                return True
            if ch in val:
                return False                  # 여기부터는 값이다
    return False


def _fleet_opt_class(tok, js):
    """런타임 옵션 토큰 → `"value"`(다음 토큰을 값으로 먹는다) · `"bool"`(값을 안 먹는다) ·
    `"unknown"`(해석 못 함 — 언랩을 포기해야 한다). 순수(R3 · 판정자 핀 2).

    코드모드(`-c`/`-m`/`-e`/`-p`/`--eval`…)는 호출 전에 `_fleet_code_mode` 가 이미 잘라낸다.
    ★`unknown` 을 `bool` 로 뭉개면 그 옵션의 **값**이 실행 주체로 승격된다(B2 재발). 반대로
      `value` 로 뭉개면 진짜 실행 대상을 건너뛴다. 둘 다 틀리므로 **모른다고 말하고 포기**한다."""
    if tok.startswith("--"):
        name = tok.split("=", 1)[0]
        known_value = name in (_JS_VALUE_LONG if js else _PY_VALUE_LONG)
        known_bool = name in (_JS_BOOL_LONG if js else _PY_BOOL_LONG)
        if "=" in tok:
            # ★R2(수렴 · 리뷰 minor "= 형 거부는 순손실"): `=` 형은 값 소비가 **이미 끝난** 형태다 —
            #   다음 토큰은 그 옵션의 값일 수 없다. 그러니 이름을 몰라도 자족적 불리언이다.
            #   R3 가 이것까지 포기한 근거는 `--run=build`(실행 모드 변경) 하나였는데, 그 소수는
            #   이름으로 거부하는 편이 옳다 — 전체를 포기하면 기준선이 옳게 세던 형상 다수가
            #   미계상(=hard 가 안 걸림 = 축이 조용히 얇아짐)으로 바뀐다.
            return "unknown" if name in _LONG_MODE_CHANGE_FLAGS else "bool"
        if not (known_value or known_bool):
            return "unknown"                  # 표에 없는 이름 — 값 소비 여부를 모른다
        return "value" if known_value else "bool"
    letters = _JS_VALUE_SHORT if js else _PY_VALUE_SHORT
    code = _JS_CODE_MODE_LETTERS if js else _PY_CODE_MODE_LETTERS
    boolean = _JS_BOOL_SHORT if js else _PY_BOOL_SHORT
    body = tok[1:]
    if not body:
        return "unknown"                      # 맨 `-` 는 호출 전에 stdin 스크립트로 잘린다
    for i, ch in enumerate(body):
        if ch in code:
            return "bool"                     # 코드모드는 위에서 이미 잘렸다(도달 불가 방어)
        if ch in letters:
            # 값 문자에서 멈춘다 — 그 뒤는 옵션 문자가 아니라 **값**이다(`-Wignore`).
            return "value" if i == len(body) - 1 else "bool"
        if ch not in boolean:
            return "unknown"                  # 미지 문자 — 이 클러스터가 값을 먹는지 알 수 없다
    return "bool"


def _fleet_runner_opt_class(tok, opts):
    """런처 옵션 토큰 → `"value"` · `"bool"` · `"abort"`(셸/명령 문자열 모드 — 언랩 종료) ·
    `"unknown"`(표에 없음 — 언랩 포기). 순수(R3 · codex "약어는 런처별이다").
    긴 옵션의 `=` 형은 이름이 표에 있을 때만 자족적인 불리언으로 본다."""
    name = tok.split("=", 1)[0] if tok.startswith("--") else tok
    if name in opts["abort"]:
        return "abort"
    if name in opts["value"]:
        return "bool" if (tok.startswith("--") and "=" in tok) else "value"
    if name in opts["bool"]:
        return "bool"
    if tok.startswith("--") and "=" in tok:
        # ★R2(수렴 · 리뷰 minor): 런처에서도 `=` 형은 값 소비가 끝난 자족적 형태다
        #   (`npx --loglevel=silly codex` · `pnpm dlx --reporter=silent codex`). 실행 자리를
        #   옮기는 이름(`uv --script=`)은 위 `abort` 표가 먼저 잡는다.
        return "bool"
    return "unknown"


def _fleet_chain_is_prefix(chains, cand):
    """`cand`(토큰 튜플)가 어느 체인의 **접두**인가 — 순수."""
    return any(c[:len(cand)] == cand for c in chains)


def _fleet_runner_key(token):
    """런처 positional → 프로그램 이름(버전 지정 분리). 순수.
    `@openai/codex@1.2.3` → `@openai/codex` · `serena-agent==1.5.3` → `serena-agent` ·
    `/usr/bin/serena` → `serena` (codex 위임 검체 8)."""
    t = (token or "").strip("\"'")
    t = t.split("==", 1)[0]
    at = t.rfind("@")
    if at > 0:
        t = t[:at]
    return t if t.startswith("@") else _fleet_exe_name(t)[1]


def _fleet_owner(cmd):
    """명령줄 → 함대 소유자 이름 | None. **argv0 앵커** 판정(순수 · 스폰 0).

    왜 이렇게 좁은가(리뷰 blocking 2 "무관한 작업을 함대 소유로 오인" 의 수리): 종전 판본은 명령줄
    **전체**를 정규식으로 훑어 `python3 /tmp/agy/report.py`(디렉터리 이름) · `grep claude`(인자)까지
    함대로 셌다. 그 오탐 하나가 곧바로 조직 기동 거부(B2 재발)다. 그래서 소유권 근거를 **실행 주체**
    하나로 좁힌다:
      ① argv0 의 basename 이 우리 CLI 이름(대소문자 구분) → 그 소유자.
      ② argv0 경로가 우리 설치 레이아웃(`/claude/versions/` · `/.codex/bin/`) → 그 소유자.
      ③ argv0 가 그 자체로 우리 npm 번들(`…/claude-code/cli.js` shebang exec) → 그 소유자.
      ④ argv0 가 JS 런타임(node/bun/deno) → 인자 중 **패키지 경로이면서 `.js` 계열**인 토큰만 인정
         (preflight strict 와 동형 — `tail -f /x/claude-code/debug.log` 는 형상이 아니다).
      ⑤ argv0 가 파이썬 런타임 → 인자 중 **경로 토큰**의 basename 이 우리 이름일 때만
         (`python3 -c codex` 처럼 경로가 아닌 코드 문자열은 실행 대상이 아니다).
      ⑥ argv0 가 온디맨드 런처(uvx/npx/uv…) → 인자 중 첫 **프로그램 이름**이 우리 것일 때만
         (`uvx --python 3.13 --from serena-agent==1.5.3 serena start-mcp-server …` → serena).
      ⑦ 그 밖의 argv0(`grep`·`tail`·`sh`·`zsh`·`env`…)이면 **인자는 보지 않는다**.
    ★남는 오차(정직 고지): (a) 이름이 같은 **남의** codex/claude 도 우리 것으로 센다 — '도구 종류'
      판별이지 '우리 편성이 띄웠다'의 증명이 아니다(그 증명에는 pgid·env 결합이 필요하고, 그것은
      이 축의 범위 밖이다). (b) 우리가 모르는 실행 형상은 **미계상**된다 → 차단 안 함 = 0.14.31
      이전과 같은 상태. 두 방향 중 (b) 쪽이 되돌리기 싸다."""
    toks = (cmd or "").split()
    if not toks:
        return None
    norm0, base0 = _fleet_exe_name(toks[0])
    low0 = norm0.lower()
    # ★순서가 중요하다(실측 2026-09-08): **정확한 이름 매칭이 앱 번들 배제보다 앞선다.**
    #   우리 데몬은 이 기계에서 `/Applications/cys.app/Contents/MacOS/cysd`(5.7~11.0%)로 뜬다 —
    #   번들 배제를 먼저 걸면 **우리 데몬 본체를 통째로 놓친다**(과소계상). 반대로
    #   `/Applications/Claude.app/Contents/MacOS/Claude` 는 basename 이 대문자 `Claude` 라
    #   정확 매칭에 안 걸리고 번들 배제로 떨어진다 — 두 사실이 이 순서에서만 동시에 성립한다.
    owner = FLEET_EXE_NAMES.get(base0)
    if owner:
        return owner
    if _APP_BUNDLE_MARKER in low0:
        return None                                   # macOS GUI 앱 번들(Claude.app 등)
    for marker, name in FLEET_ARGV0_MARKERS:
        if marker in low0:
            return name
    for marker, name in FLEET_JS_BUNDLE_MARKERS:
        if marker in low0 and low0.endswith(_JS_SUFFIXES):
            return name
    rest = toks[1:1 + FLEET_RUNNER_SCAN_MAX]
    if base0 in FLEET_JS_RUNTIMES or base0 in FLEET_PY_RUNTIMES \
            or _PY_VERSIONED_RE.match(base0):
        js = base0 in FLEET_JS_RUNTIMES
        # ★**첫 경로 토큰 하나만** 본다(codex 위임 검체 4): 계속 훑으면
        #   `node /tmp/report.js /tmp/codex` 처럼 **데이터 인자**가 실행 주체로 승격된다 —
        #   그것이 바로 이 라운드가 없앤 B2 오탐이다. 대가는 `node --require x.js /x/codex` 형상의
        #   미계상(=차단 안 함=종전 상태)이고, 그 방향을 택한다.
        skip_next = False
        opts_done = False
        for t in rest:
            if skip_next:
                skip_next = False
                continue                              # 앞 옵션이 먹은 **값** — 실행 대상이 아니다
            if not opts_done:
                # ★R2(수렴 · 리뷰 major 표 1·2행): `--` 는 '모르는 옵션' 이 아니라 **옵션 해석을
                #   끝내는 상태 전이**다(POSIX). 그 뒤의 첫 토큰은 정의상 실행 대상이므로,
                #   `node -- /opt/bin/codex exec` · `python3 -- /opt/bin/serena` 는 기준선처럼
                #   다시 소유자를 낸다. 런처 갈래(:아래)는 이미 같은 규칙을 갖고 있었다.
                if t == "--":
                    opts_done = True
                    continue
                # ★코드 문자열 모드가 **경로 토큰보다 먼저** 나오면 언랩하지 않는다: `-c`/`-e`/`-m` 뒤의
                #   토큰은 실행 파일이 아니다(`python3 -c print(1) /tmp/serena`). 순서를 봐야 한다 —
                #   실측 `node /…/bin/codex exec -m gpt-6-astra …` 처럼 **대상 프로그램의 인자**에
                #   같은 글자가 오는 형상이 흔하다(그때는 이미 실행 주체를 정한 뒤다).
                if _fleet_code_mode(t, js):
                    return None
                if t == "-":
                    return None                       # stdin 스크립트 — 뒤는 그 스크립트의 인자다
                if t.startswith("-"):
                    cls = _fleet_opt_class(t, js)
                    if cls == "unknown":
                        # ★R3(판정자 핀 2): 해석 못 한 옵션 **뒤의 토큰을 소유권 근거로 쓰지 않는다**.
                        #   값 소비 여부를 모르면 다음 토큰이 '그 옵션의 값' 인지 '실행 대상' 인지도
                        #   모른다 — 둘 중 하나로 찍으면 반드시 한쪽이 오탐이다.
                        return None
                    skip_next = (cls == "value")
                    continue                          # 런타임 옵션 — 실행 대상이 아니다
            # ★R2(리뷰 major): 여기서 멈춘다 — 첫 **비옵션** 토큰이 실행 대상(스크립트·모듈)이고,
            #   그 뒤는 전부 그 프로그램의 **데이터 인자**다. 종전 판본은 '슬래시 있는 첫 토큰' 을
            #   찾느라 상대경로 스크립트를 건너뛰어 `python3 report.py /tmp/serena` ·
            #   `node report.js /tmp/codex` 의 데이터 인자를 실행 주체로 승격시켰다(B2 재발).
            if "/" not in t and "\\" not in t:
                return None                           # 상대 스크립트 — 경로 근거 없음 · 미계상
            norm, base = _fleet_exe_name(t)
            low = norm.lower()
            owner = FLEET_EXE_NAMES.get(base)
            if owner and _APP_BUNDLE_MARKER not in low:
                return owner                          # 실측: `node /Users/user/.local/bin/codex …`
            if js:
                for marker, name in FLEET_JS_BUNDLE_MARKERS:
                    if marker in low and low.endswith(_JS_SUFFIXES):
                        return name                   # `node …/claude-code/cli.js`
            return None                               # 첫 경로 토큰이 아니면 실행 주체가 아니다
        return None
    if base0 in FLEET_RUNNERS:
        # ★런처는 **첫 프로그램 위치 토큰 하나만** 본다(codex 위임 검체 5):
        #   `npm --prefix codex test` · `npm uninstall codex` · `npx prettier codex` 가 전부
        #   codex 로 오인되던 길을 닫는다. 긴 옵션(`--x`)은 값을 가질 수 있으니 다음 토큰까지
        #   건너뛰고(`--python 3.13` · `--from serena-agent==1.5.3`), 짧은 옵션(`-y`)은 값이 없다고
        #   본다(`npx -y @google/gemini-cli`). 하위 명령(`uv run …`)은 1회 건너뛴다.
        #   ★중첩 런처도 건너뛴다 — 실측 형상 `/…/bin/uv tool uvx --python 3.13 --from
        #     serena-agent==1.5.3 serena start-mcp-server`(uv → tool → uvx → serena).
        #   ★R2: 허용 하위 명령은 **런처별**이고, npm/pnpm/yarn 은 그것이 **필수**다
        #     (`npm run codex`·`yarn codex` 의 positional 은 프로그램이 아니라 스크립트 이름).
        chains = FLEET_RUNNER_CHAINS.get(base0, frozenset())
        opts = FLEET_RUNNER_OPTS.get(base0, _RUNNER_OPTS_EMPTY)
        chain = ()
        opts_done = False
        skip_next = False
        for t in rest:
            if skip_next:
                skip_next = False
                continue
            if not opts_done:
                if t == "--":
                    # ★R3(codex): `--` 는 '건너뛰기' 가 아니라 **옵션 해석을 끝내는 상태 전이**다.
                    #   그 뒤의 `-x` 는 옵션이 아니라 프로그램 이름/인자다(codex D6 형상 보존).
                    opts_done = True
                    continue
                if t.startswith("-") and t != "-":
                    cls = _fleet_runner_opt_class(t, opts)
                    if cls in ("unknown", "abort"):
                        return None                   # 모르는 옵션·셸 문자열 모드 — 언랩 포기
                    skip_next = (cls == "value")
                    continue
            low = t.lower()
            if _fleet_chain_is_prefix(chains, chain + (low,)):
                chain += (low,)
                continue                              # 하위 명령 체인이 이어진다
            # ★codex D5: 필수 하위 명령 검사가 **중첩 런처 전환보다 먼저**다 — 아니면
            #   `yarn npx codex` 처럼 스크립트 이름 자리의 토큰이 런처로 승격한다.
            if chains and chain not in chains:
                return None                           # 스크립트 이름 자리 — 실행 주체가 아니다
            nested = _fleet_exe_name(t)[1].lower()
            if nested in FLEET_RUNNERS:               # 중첩 런처 — 규칙도 그쪽으로 갈아탄다
                chains = FLEET_RUNNER_CHAINS.get(nested, frozenset())
                opts = FLEET_RUNNER_OPTS.get(nested, _RUNNER_OPTS_EMPTY)
                chain, opts_done = (), False
                continue
            return FLEET_RUNNER_PROGRAMS.get(_fleet_runner_key(t))
        return None
    return None


def _fleet_rows(lines, self_pid=None):
    """ps 줄들 → `(rows|None, reason|None)`. row = `{"pid","pcpu","owner","exe"}`(함대 행만).

    ★'매칭 0건'과 '측정 불능'을 섞지 않는다(codex R1 #6):
      - pcpu 를 수치로 읽은 줄이 하나라도 있고 매칭이 0건 = **함대가 안 떠 있다**는 정직한 사실 → [].
      - 한 줄도 `pid pcpu command` 로 못 읽음(`shape`) = 열 형상이 계약과 다르다 → None.
        ★열이 셋이라 오형상은 여기서 대부분 잡힌다(pid 는 정수, pcpu 는 실수여야 한다) — 종전
        2열 판본은 ps 가 조용히 `pid,command` 를 냈을 때 pid 를 CPU 로 읽어 합을 폭증시켰다.
      - **함대 행**의 pcpu 가 안 읽히거나(`row_unparsed`) 유한하지 않거나 음수(`row_nonfinite`)
        = 함대의 일부를 못 읽었다 → None. 그 줄만 조용히 건너뛰면 합이 과소계상된다.
    ★자기 제외는 **PID** 다(`os.getpid()`): 명령줄 부분문자열 제외는 출력 파일명에 이 모듈 이름을
      넣은 진짜 함대 프로세스를 통째로 뺐다(리뷰 blocking 1)."""
    if self_pid is None:
        self_pid = os.getpid()
    rows = []
    parsed = 0
    for line in lines or []:
        s = line.strip()
        if not s:
            continue
        parts = s.split(None, 2)
        if len(parts) < 3:
            continue
        pid_s, head, cmd = parts
        try:
            pid = int(pid_s)
        except ValueError:
            # 헤더 잔재·2열 출력이면 그냥 넘긴다. 그러나 **함대 행**이면 조용히 버리지 않는다 —
            # 그 줄만 빠지면 합이 과소계상되고 그것이 '측정 성공'으로 나간다(codex 위임 검체 9).
            if _fleet_owner(cmd) is not None:
                return None, "row_unparsed"
            continue
        if pid <= 0:
            if _fleet_owner(cmd) is not None:
                return None, "row_unparsed"
            continue                     # 유효 PID 가 아니다(열 순서가 뒤바뀐 형상 방어)
        owner = _fleet_owner(cmd)
        mine = owner is not None and pid != self_pid
        try:
            pcpu = float(head)
        except ValueError:
            if mine:
                return None, "row_unparsed"
            continue
        if not math.isfinite(pcpu) or pcpu < 0:
            if mine:
                return None, "row_nonfinite"
            continue
        parsed += 1
        if mine:
            rows.append({"pid": pid, "pcpu": pcpu, "owner": owner,
                         "exe": _fleet_exe_name((cmd.split() or [""])[0])[1][:64]})
    if parsed == 0:
        return None, "shape"
    return rows, None


def _fleet_cpu_percent(lines, self_pid=None):
    """함대 행들의 %CPU 합 → `(total|None, reason|None)`.

    ★`float("nan")` 은 예외 없이 성공하고 이후 모든 비교가 False 라, NaN 이 섞이면 합이 NaN 이 되어
      `NaN >= hard` 가 거짓 → **조용한 allow** 가 된다. `_fleet_rows` 의 유한성 검사가 그 길을 막는다."""
    rows, why = _fleet_rows(lines, self_pid)
    if rows is None:
        return None, why
    try:
        total = sum(r["pcpu"] for r in rows)
    except OverflowError:                # 보수적 합산 경로가 예외를 올릴 수도 있다(fsum 계열)
        return None, "sum_overflow"
    if not math.isfinite(total):
        # 각 항이 유한해도 **합**은 넘칠 수 있다(1e308+1e308). inf 는 `inf >= hard` 로 hard 를 내지만
        # JSON 으로 나가면 `Infinity` — 표준 JSON 이 아니라 엄격한 소비자에서 깨진다. 판정을
        # 사고로 내지 않고 '못 쟀다' 로 정직하게 접는다(codex R2 · 위임 검체 4).
        return None, "sum_overflow"
    return total, None


def _fleet_cpu_top(lines, top_n=FLEET_CPU_TOP_N, self_pid=None):
    """상위 기여 프로세스 `[{pid, exe, owner, pcpu}]` — hard 판정의 **회수 대상 지목**(봉인표 ③).

    ★명령줄(인자)을 담지 않는다(리뷰 major): 종전 판본은 명령줄 앞 120자를 마스킹 없이 실었고,
      그 값이 `measured` 를 타고 `boot-last.json`·편성 상태파일로 **영속**됐다
      (`… --api-key secret123` 같은 인자가 정상 판정에서도 디스크에 남는다). 회수에 필요한 것은
      PID·실행 주체·CPU 셋이고, 그 셋에는 인자가 필요 없다.
    실패는 조용한 빈 목록이다: 이것은 판정 입력이 아니라 사람이 읽을 증거이므로, 여기서 나는 오류가
    판정을 흔들면 안 된다(판정 입력은 `_fleet_cpu_percent` 하나).
    ★그 격리를 **말로만 두지 않는다**(codex R2 · 위임 검체 5): 종전 판본은 이 약속을 주석에만 두고
      예외를 그대로 흘려, 진단 보조 하나가 정상 측정을 exit 70(EX_SOFTWARE)으로 만들 수 있었다."""
    try:
        rows, _why = _fleet_rows(lines, self_pid)
        rows = list(rows or [])
        rows.sort(key=lambda r: r["pcpu"], reverse=True)
        return rows[:top_n]
    except Exception:                    # noqa: BLE001 — 진단이 판정을 죽이지 않는다는 계약
        return []


# ── ★R1(봉인표 ③): fleet_cpu hard 의 **연속 보류 상한** ──
def _pack_state_dir():
    """팩 관례 상태 루트 — `CYS_STATE_DIR` ‖ `~/.cys/state`(javis_bootstrap._state_root 와 동형).
    데몬 상태 디렉터리(`~/.local/state/cys`)에는 쓰지 않는다 — 그쪽은 바이너리 소유다."""
    return os.environ.get("CYS_STATE_DIR") or os.path.join(
        os.path.expanduser(CYS_DIR_DEFAULT), "state")


def _lane_socket():
    """이 호출이 **판정 대상으로 삼는 레인**의 소켓 경로 — `CYS_GATE_LANE_SOCKET` > `CYS_SOCKET`.

    ★R2(리뷰 major · codex A1): 편성(`javis_formation._resource_verdict`)은 `--socket <s>` 로 받은
      레인을 게이트에 전혀 알리지 않았고, 실제 호출자가 base 데몬의 심박 셸 루프라 **부서 레인의
      편성이 항상 base 의 boot-epoch·래치를 봤다**(부서 재시작 직후 = ③ 복구 구간에 유예 0).
    ★그런데 그 값을 `CYS_SOCKET` 이라는 이름으로 넘기면 안 된다: 그 변수는 `cys` CLI 자체가 읽는
      값이라, 게이트 안의 원장 조회(`_ledger_servers` 의 `cys ps`)가 부서 데몬을 보게 되고 **base
      원장이 통째로 집계에서 빠진다**(servers 3 → 0 이 가능 · codex R2 A1). 그래서 레인 판정
      **전용** 변수를 따로 나른다 — 구 게이트는 이 변수를 모르므로 그냥 무시하고 종전대로 돈다
      (플래그와 달리 EX_USAGE 스큐가 없다)."""
    return os.environ.get("CYS_GATE_LANE_SOCKET") or os.environ.get("CYS_SOCKET")


def _fleet_hold_thr_key(thr):
    """hard 임계 → 파일명 조각. **파싱된 float 의 정규 표현**을 쓴다(codex R2 B2):
    `1`·`1.0`·`1e0` 은 같은 래치여야 하고 `-0.0` 은 `0.0` 과 같아야 한다."""
    if thr is None:
        return "any"
    try:
        v = float(thr)
    except (TypeError, ValueError):
        return "any"
    if not math.isfinite(v):
        return "any"
    if v == 0:
        v = 0.0                                     # ±0 통일
    return re.sub(r"[^A-Za-z0-9._-]", "_", repr(v))


def _fleet_hold_path(thr=None):
    """레인·임계별 래치 경로.

    ★R2 두 가지를 고쳤다.
      ① 레인 키의 **해시가 잘려 사라지던 길**(리뷰 minor): 종전은 `name + "-" + sha8` 을 만든 뒤
         전체를 80자로 잘라, 디렉터리 이름이 긴 두 레인(`/a/<x×90>/cys.sock`·`/b/<x×90>/cys.sock`)이
         같은 경로를 냈다. 이제 **이름 쪽을 먼저 40자로 줄이고** 해시를 뒤에 붙인다(해시 불멸).
         해시 입력도 dirname 이 아니라 **소켓 전체 경로**다 — 한 디렉터리에 소켓이 둘이면
         dirname 해시는 여전히 충돌한다(codex R2 B2).
      ② 파일명에 **hard 임계**를 넣는다(리뷰 major · codex R2 B2 동의): 종전엔 임계가 다른 호출
         (`--fleet-cpu-hard 2`)이 '이 임계로는 below' 를 관측하고 **남의 래치를 지웠다** — 기본
         임계 호출자의 연속 시계가 매번 0으로 돌아가 상한이 영영 안 차는 길이었다. 래치가 뜻하는
         것은 '이 임계에서의 연속 포화' 이므로 임계마다 자리를 가진다."""
    sock = _lane_socket()
    key = "default"
    if sock:
        full = os.path.abspath(os.path.expanduser(sock))
        name = re.sub(r"[^A-Za-z0-9._-]", "_",
                      os.path.basename(os.path.dirname(full)) or "lane")[:40]
        key = "%s-%s" % (name, hashlib.sha256(full.encode("utf-8", "replace")).hexdigest()[:8])
    return os.path.join(_pack_state_dir(), "resource-gate",
                        "%s-%s-h%s" % (FLEET_CPU_HOLD_BASENAME, key, _fleet_hold_thr_key(thr)))


def _fleet_hold_legacy_path():
    """R1 판(임계 접미가 없던) 래치 이름 — **더 읽지 않는다**. `below` 때 잔재만 치운다.
    ★이름을 재현해서 지우는 이유: 새 이름으로 옮겨 가면 구 파일이 영원히 남는다. 그렇다고 레인
      접두로 싹 지우면 **다른 임계의 래치까지** 지워져 R2 가 고친 결함이 되살아난다(임계 X 에서의
      below 는 임계 Y<X 에서의 below 가 아니다)."""
    sock = _lane_socket()
    key = "default"
    if sock:
        full = os.path.dirname(os.path.abspath(os.path.expanduser(sock)))
        key = "%s-%s" % (os.path.basename(full) or "lane",
                         hashlib.sha256(full.encode("utf-8", "replace")).hexdigest()[:8])
    key = re.sub(r"[^A-Za-z0-9._-]", "_", key)[:80]
    return os.path.join(_pack_state_dir(), "resource-gate",
                        "%s-%s" % (FLEET_CPU_HOLD_BASENAME, key))


def _fleet_expired_mark_path(path):
    """래치 레코드 경로 → **만료 표식** 경로. 순수(부작용 0).

    ★왜 별도 파일인가(R3 · codex blocking 의 수리): 만료는 `below` 전까지 **단조**인 한 비트인데,
      그것을 가변 JSON 레코드 안에 두면 '읽기→판정→교체' 의 잃어버린 갱신이 그 비트를 되돌린다.
      실측 인터리브: A(now=1899)의 병합 재읽기와 `os.replace` **사이**에 B(now=1900)가 `expired=True`
      를 저장하고 그 완화를 **호출자에게 이미 공개**하면, A 의 낡은 `expired=False` 가 그것을 덮었다.
      그 뒤 시계 역행이 오면 `below` 관측 없이 재무장해 899초가 다시 막혔다(판정자 핀 1b/1c/1d).
    ★왜 잠금이 아닌가: ⓐ 이 게이트는 부트 체인 경로다 — 잠금 획득 실패·스테일 락 회수(시계 역행과
      결합하면 살아 있는 락을 뺏거나 죽은 락을 영원히 남긴다)라는 **새 실패 모드**를 들이는 값이
      이 이득보다 비싸다. ⓑ Windows 에 `flock` 이 없다. ⓒ 무엇보다 잠금은 이 문제를 못 푼다:
      직렬화해도 '타임아웃으로 만료를 공개한 호출' 의 공개가 파일에 남지 않는다(codex 반례).
      **생성만 가능한 표식**은 잃어버린 갱신이 원리적으로 불가능하다 — 덮어쓸 내용이 없다.
    ★단조성의 경계: 표식은 `below`(축이 임계 미만으로 관측됨)에서만 지워진다. 그 삭제와 동시에
      진행 중이던 hard 호출이 표식을 되만들 수 있다 — 귀결은 '한 번 더 soft(권고)' 이고, 방향은
      **덜 막음**이라 §3-3 이 허용하는 쪽이다(그리고 다음 `below` 가 다시 지운다)."""
    return path + ".expired"


def _fleet_expired_mark(path):
    """만료 표식 존재 여부 → bool. 판독 실패는 **False 가 아니라** 호출자가 따로 다룬다(여기선 존재만).
    ★`os.path.exists` 는 권한 오류에서도 False 다 — 그 경우의 귀결은 '만료를 못 봄 = 더 막음' 이라
      래치 본체의 `unreadable → unbounded_io` 정책이 상위에서 그 통을 덮는다."""
    try:
        return os.path.exists(_fleet_expired_mark_path(path))
    except OSError:
        return False


def _fleet_expired_mark_set(path):
    """만료 표식 **생성**(멱등) → 성공 여부. 실패는 예외가 아니라 False.
    이미 있으면 True — `O_EXCL` 의 EEXIST 는 '남이 먼저 만들었다' 이고 그것도 성공이다."""
    mp = _fleet_expired_mark_path(path)
    try:
        d = os.path.dirname(mp)
        if d:
            os.makedirs(d, exist_ok=True)
        fd = os.open(mp, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        try:
            os.write(fd, b"1\n")
        finally:
            os.close(fd)
        return True
    except FileExistsError:
        return True
    except OSError:
        return False


def _fleet_expired_reason(path, stale):
    """만료 사유 문자열 — **표식이 실제로 서 있는가**를 사유에 적는다. 순수하지 않음(표식 1회 조회).

    ★R2(수렴 · 리뷰 minor "표식 생성 실패가 조용하다"): `_fleet_hold_write` 는
      `_fleet_expired_mark_set` 의 반환값을 버린다 — 상태 디렉터리가 쓰기 불가(ENOSPC/EACCES)면
      표식이 안 서고, 만료는 **가변 레코드의 비트 하나**로만 남는다. 그 상태는 이 라운드가 고친
      잃어버린 갱신 경합이 그대로 되살아난 것인데, 종전엔 아무 표시가 없었다. 대칭적으로 `below`
      의 삭제 실패는 `clear_failed` 로 남는다 — 생성 실패만 침묵할 이유가 없다.
      방향: 사유만 바뀐다(만료 여부·exit 불변). `_stale` 접미는 유지한다 — 사람 출력의
      `endswith("_stale")` 판정이 그것을 읽는다."""
    base = "expired" if _fleet_expired_mark(path) else "expired_unmarked"
    return (base + "_stale") if stale else base


def _fleet_expired_mark_clear(path):
    """만료 표식 삭제 → 성공 여부(부재도 성공). `below` 경로 전용."""
    try:
        os.remove(_fleet_expired_mark_path(path))
        return True
    except FileNotFoundError:
        return True
    except OSError:
        return False


def _fleet_hold_read(path):
    """래치 → `(rec|None, reason)`. rec = `{"since","last","expired"}` (전부 유한 float / bool).

    ★`expired` 는 **레코드의 비트 ∨ 만료 표식의 존재**다(R3). 표식이 있으면 레코드가 무엇을 말하든
      만료다 — 그것이 이 단조 비트를 경합에서 지키는 유일한 장치다.

    reason ∈ None(정상) · "missing"(정상 부재) · "unreadable"(있는데 못 읽음) · "corrupt"(내용 파손).
    ★셋을 가르는 이유(codex R2 B3): '정상 부재' 는 **지금 무장**(상한이 지금부터 다시 유계)이지만,
      '읽기 불능' 은 유계를 증명할 수 없는 상태라 쓰기 실패와 같은 통(만료)에 넣어야 한다. 종전엔
      둘을 뭉개 권한 오류가 매 호출 재무장이 되어 상한이 영원히 안 찼다.
    ★구 형식(bare float)도 읽는다 — R1 래치와의 호환."""
    try:
        with open(path, "r", encoding="utf-8") as f:
            raw = (f.read() or "").strip()
    except FileNotFoundError:
        return None, "missing"
    except OSError:
        return None, "unreadable"
    except ValueError:
        # ★codex 위임 검체 D9: 디코딩 불가 바이트(UnicodeDecodeError = ValueError)가 이 함수를
        #   빠져나가면 최상위 경계가 그것을 exit 70('내부 오류')으로 접는다 — 래치 파일 한 줄이
        #   게이트 전체를 죽이는 길이다. 그것은 '내용 파손' 이다.
        return None, "corrupt"
    if not raw:
        return None, "corrupt"
    obj = None
    try:
        obj = json.loads(raw)
    except ValueError:
        obj = None
    if isinstance(obj, dict):
        since, last = obj.get("since"), obj.get("last", obj.get("since"))
        if not (isinstance(since, (int, float)) and isinstance(last, (int, float))) \
                or isinstance(since, bool) or isinstance(last, bool):
            return None, "corrupt"
        try:
            # ★codex 위임 검체 D10: JSON 파싱은 성공해도 `float(거대 정수)` 는 OverflowError 다
            #   (유한성 검사에 닿기 전에 예외로 탈출 → exit 70).
            since, last = float(since), float(last)
        except (OverflowError, ValueError, TypeError):
            return None, "corrupt"
        if not (math.isfinite(since) and math.isfinite(last)):
            return None, "corrupt"
        return {"since": since, "last": last,
                "expired": (obj.get("expired") is True) or _fleet_expired_mark(path)}, None
    try:
        v = float(raw)                       # 구 형식(R1 · bare float)
    except ValueError:
        return None, "corrupt"
    if not math.isfinite(v):
        return None, "corrupt"
    return {"since": v, "last": v, "expired": _fleet_expired_mark(path)}, None


def _fleet_hold_write(path, value, merge=False):
    """래치 원자 기록 → 성공 여부. 실패는 예외가 아니라 False(판정을 죽이지 않는다).
    value 는 레코드 dict 또는 (구 호출 호환) 저장 시각 float.

    ★merge=True 면 **교체 직전에 현재 레코드를 다시 읽어 단조 병합**한다(codex 위임 검체 D2):
      `since` 는 더 이른 값, `last` 는 더 늦은 값, `expired` 는 **논리합**. 두 게이트 프로세스의
      읽기→판정→쓰기가 겹칠 때 늦은 쓰기가 남의 `expired=True` 를 되돌리면 이미 공개된 완화가
      취소돼 차단이 되살아난다(봉인표 ③ 역행).
    ★정직 표기: 이것은 **완화**이지 직렬화가 아니다 — 재읽기와 replace 사이에도 창은 남는다.
      그 창에서 잃어버릴 수 있는 것은 이제 `since`/`last` 뿐이고(손해 = '한 호출이 다시 hard 를
      본다' · 다음 호출이 `now-since` 로 스스로 회복), **`expired` 는 이 레코드가 아니라 생성 전용
      표식이 지킨다**(R3 · `_fleet_expired_mark_path`). 잠금을 들이지 않은 이유는 그 함수 주석에
      있다 — 요지는 ⓐ부트 체인에 새 실패 모드를 늘리지 않는다 ⓑWindows 에 flock 이 없다
      ⓒ직렬화해도 '타임아웃으로 만료를 공개한 호출' 의 공개는 파일에 남지 않는다(codex 반례).
    ★쓰기가 `expired=True` 를 담고 있으면 **표식을 먼저 세운다**: 레코드만 True 인 상태로 남으면
      다음 경합이 그것을 되돌릴 수 있다(그 경로가 바로 이 라운드의 결함이었다)."""
    if not isinstance(value, dict):
        value = {"since": float(value), "last": float(value), "expired": False}
    rec = {"v": FLEET_HOLD_RECORD_V, "since": float(value["since"]),
           "last": float(value.get("last", value["since"])),
           "expired": bool(value.get("expired"))}
    if merge:
        cur, _why = _fleet_hold_read(path)
        if cur is not None:
            rec["since"] = min(rec["since"], cur["since"])
            rec["last"] = max(rec["last"], cur["last"])
            rec["expired"] = bool(rec["expired"] or cur["expired"])
    if rec["expired"]:
        # 표식이 진실의 보관소다 — 레코드의 비트는 그 사본(구 판본 호환·진단 가독성)일 뿐이다.
        _fleet_expired_mark_set(path)
    tmp = None
    try:
        d = os.path.dirname(path)
        if d:
            os.makedirs(d, exist_ok=True)
        # ★임시 이름은 **호출마다** 다르다(codex 위임 검체 R2 · 스레드 경합): pid 만 쓰면 같은
        #   프로세스의 두 호출이 같은 tmp 를 쓰다가 한쪽 `os.replace` 가 ENOENT 로 실패하고,
        #   그 실패가 `unbounded_io`(=만료 취급)로 번져 보류가 근거 없이 풀린다.
        tmp = "%s.%d.%s.tmp" % (path, os.getpid(), os.urandom(4).hex())
        with open(tmp, "w", encoding="utf-8") as f:
            # ★repr 정밀도로 쓴다(codex 위임 검체 13): `%.3f` 는 올림 때문에 저장값이 `now` 보다
            #   커질 수 있고, 그러면 다음 호출이 그것을 '미래' 로 읽어 시계를 0으로 되돌린다.
            f.write(json.dumps(rec) + "\n")
        os.replace(tmp, path)            # Windows 에서도 원자 교체
        return True
    except (OSError, ValueError, TypeError):
        if tmp:
            try:
                os.remove(tmp)
            except OSError:
                pass
        return False


def _fleet_hard_hold(state, override=None, now=None, thr=None):
    """이 축이 **첫 hard 관측 이후** 머문 초 → `(hold|None, reason, expired)`.

    state ∈ "hard"(임계 이상) · "below"(쟀는데 임계 미만) · "unmeasured"(값이 없다 — 축 부재·측정 실패).
    reason ∈ "override" · "armed" · "held" · "held_stale" · "expired" · "expired_stale" ·
             "expired_unmarked" · "expired_unmarked_stale" · "cleared" · "clear_failed" ·
             "unmeasured" · "unbounded_io".
    ★`expired_unmarked*` = 만료는 났는데 **표식 파일을 못 세웠다**(상태 디렉터리 쓰기 불가).
      만료 자체는 그대로 공개하지만(방향 불변), 그 만료는 경합에 취약하다는 사실을 사유로 남긴다.
    계약(봉인표 ③): `expired=True` 면 소비자(evaluate)가 hard 를 soft 로 내린다.
      · below            → 래치 삭제(연속만 센다) · expired False
      · unmeasured       → 래치 **무접촉**(codex R1-2): 값을 못 잰 호출이 남의 연속 hard 시계를 0으로
                          되돌리면, 간헐적 ps 실패만으로 상한이 영원히 안 찬다(유계가 사라진다)
      · 래치 부재/파손   → 지금으로 무장(상한이 지금부터 다시 유계) · 쓰기 실패면 **unbounded_io +
                          expired**(유계를 증명할 수 없는 상태에서 무기한 차단을 열지 않는다)
      · 래치 **읽기 불능** → 같은 이유로 unbounded_io + expired (R2 · codex B3)
      · 미래 값(시계 역행) → 다시 무장(같은 취급)
      · hold < 상한      → 유지
      · hold >= 상한     → expired True. **재무장하지 않는다** — 만료는 '이 포화가 끝날 때까지 권고'
                          라는 상태이고, 재무장하면 만료 창을 다른 호출자가 소비해 정작 복구가
                          필요한 편성 호출이 다시 hard 를 만난다(유계가 호출자별로 안 보장된다).
      · ★만료는 **저장된 상태**다(R2 리뷰 major · codex B1 ①): 종전은 매 호출 `now - since` 로
        재계산해서, 시계가 2초만 뒤로 가도 만료가 풀리고 차단이 되살아났다(저장값 1000·now=1901 →
        만료 / now=1899 → 다시 차단). 이제 한 번 만료하면 `below` 관측 전까지 만료다.
      · ★그리고 그 저장은 **가변 레코드가 아니라 생성 전용 표식**이다(R3 · 판정자 핀 1b/1c/1d ·
        codex blocking): 레코드 안의 비트는 '읽기→판정→교체' 의 잃어버린 갱신으로 되돌려졌다 —
        A 의 낡은 `expired=False` 가 B 가 **이미 호출자에게 공개한** 만료를 덮었고, 이어진 시계
        역행이 `below` 없이 재무장을 불러 899초를 다시 막았다. 생성 전용 표식은 덮어쓸 내용이
        없어 그 경합이 원리적으로 불가능하다. 표식은 `below` 에서만 지워진다.
      · ★관측 공백(`now - last > 상한`)은 사유에 `_stale` 로 남긴다 — 그 구간에 실제로 막힌 호출은
        없으므로 `hold` 수치를 '차단한 시간' 으로 읽으면 안 된다(R2 리뷰 minor · 정직 표기).
    ★부작용 경계: `override` 가 주어지면 파일을 **읽지도 쓰지도 않는다**(self-test·검체 밀폐)."""
    if override is not None:
        return override, "override", bool(state == "hard"
                                          and override >= FLEET_CPU_HARD_MAX_HOLD_SECS)
    if state == "unmeasured":
        return None, "unmeasured", False
    now = time.time() if now is None else now
    path = _fleet_hold_path(thr)
    if state != "hard":
        ok = True
        try:
            os.remove(path)
        except FileNotFoundError:
            pass
        except OSError:
            # 지우지 못했으면 그렇게 적는다 — 다음 hard 가 남은 래치 때문에 **더 일찍** 만료된다
            # (방향은 ③ 안전이지만 '연속 보류' 의 의미가 달라지므로 침묵하지 않는다).
            ok = False
        # ★만료 표식도 여기서만 지워진다(R3). 못 지우면 그 축은 다음 포화에서 처음부터 권고(soft)
        #   로 관측된다 — 방향은 **덜 막음**이라 §3-3 이 허용하는 쪽이고, 침묵하지 않고 사유에 적는다.
        if not _fleet_expired_mark_clear(path):
            ok = False
        try:
            os.remove(_fleet_hold_legacy_path())     # R1 잔재 청소(best-effort · 판정 무관)
        except OSError:
            pass
        return (None, "cleared", False) if ok else (None, "clear_failed", False)
    rec, why = _fleet_hold_read(path)
    if why == "unreadable":
        return None, "unbounded_io", True
    # ★저장된 만료는 **시계 역행보다 먼저** 판정한다(codex 위임 검체 D1): 순서를 뒤집으면 큰
    #   역행(저장 since 가 now+2s 보다 미래)이 `below` 없이 재무장을 일으켜 **이미 공개된 만료가
    #   취소되고 차단이 되살아난다** — 만료를 저장한 목적 그 자체가 무너진다.
    if rec is not None and rec["expired"]:
        hold = max(0.0, now - rec["since"])
        stale = (now - rec["last"]) > FLEET_CPU_HARD_MAX_HOLD_SECS
        _fleet_hold_write(path, {"since": rec["since"], "last": now, "expired": True}, merge=True)
        return hold, _fleet_expired_reason(path, stale), True
    if rec is None or rec["since"] > now + FLEET_HOLD_FUTURE_SLACK_S:
        # 부재·파손·미래 저장값(시계 역행) → 지금 무장. 쓰기 실패는 유계 증명 불능이다.
        # ★R3: **표식이 살아 있으면 무장이 아니라 만료다.** 레코드를 잃었거나(부재·파손) 시계가
        #   뒤로 갔더라도 `below` 는 관측되지 않았고, 그 사이 만료는 이미 호출자에게 공개됐다.
        #   여기서 재무장하면 그 공개가 취소되고 상한만큼이 다시 막힌다(판정자 핀 1c).
        if _fleet_expired_mark(path):
            _fleet_hold_write(path, {"since": now, "last": now, "expired": True}, merge=True)
            return 0.0, _fleet_expired_reason(path, False), True
        ok = _fleet_hold_write(path, {"since": now, "last": now, "expired": False})
        # ★R2(수렴 · 리뷰 minor "되읽기가 held 갈래에만 있다"): 무장 갈래도 **쓰기 뒤에** 표식을
        #   되읽는다. 종전엔 쓰기 **전** 한 번만 봐서, 우리가 재무장하는 사이 다른 호출이 공개한
        #   만료가 이 호출에만 안 보였다 — '만료는 모든 호출자에게 동일하게 보인다' 는 이 수리의
        #   계약이 이 갈래에서만 깨져 있었다(파일=True · 반환=False). 표식은 단조(생성 전용)라
        #   이 되읽기는 남의 갱신을 잃어버릴 수 없다.
        if ok and _fleet_expired_mark(path):
            return 0.0, _fleet_expired_reason(path, False), True
        return (0.0 if ok else None), ("armed" if ok else "unbounded_io"), (not ok)
    # 허용오차 안의 '미래' 저장값은 음수 경과를 만든다 — 0 으로 죈다(음수 보류초는 뜻이 없다).
    hold = max(0.0, now - rec["since"])
    stale = (now - rec["last"]) > FLEET_CPU_HARD_MAX_HOLD_SECS
    expired = hold >= FLEET_CPU_HARD_MAX_HOLD_SECS
    ok = _fleet_hold_write(path, {"since": rec["since"], "last": now, "expired": expired},
                           merge=True)
    if not ok:
        return hold, "unbounded_io", True
    # ★R3(codex "파일=True·반환=False 가 가능하다"): 쓰기 뒤 **표식을 되읽어** 반환값을 맞춘다.
    #   표식은 단조(생성 전용)라 이 되읽기는 뜻이 있다 — 가변 레코드의 되읽기와 달리 남의 갱신을
    #   잃어버릴 수 없다. 우리가 `held` 를 계산하는 사이 다른 호출이 만료를 공개했다면 그 완화가
    #   이 호출에도 보여야 한다(만료는 모든 호출자에게 동일하게 보인다는 계약).
    if not expired and _fleet_expired_mark(path):
        expired, stale = True, (now - rec["last"]) > FLEET_CPU_HARD_MAX_HOLD_SECS
    if expired:
        return hold, _fleet_expired_reason(path, stale), True
    return hold, ("held_stale" if stale else "held"), False


def _boot_epoch_path():
    """이 레인 데몬의 `boot-epoch` 경로 — `dirname(레인 소켓)` 우선(부서 레인은 부서 것을 본다).
    레인 소켓은 `_lane_socket()`(= `CYS_GATE_LANE_SOCKET` > `CYS_SOCKET`) — R2 리뷰 major 참조."""
    sock = _lane_socket()
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
    #   ⓐ `ps` **부재**(Windows 기본)·**플래그 미지원**(MSYS/BusyBox `-axo`)은 측정 실패가 아니라
    #      '이 플랫폼엔 이 축이 없다'는 사실이다 → `measure_errors` 에 넣지 않는다(정본 §4 WP-7 ·
    #      Windows exit 계약 불변). `checks` 에는 level="unavailable" 로 남겨 침묵하지 않는다.
    #   ⓑ ps 도 있고 플래그도 아는데 조회·형상이 깨진 것은 **측정 실패**다 → `measure_errors`
    #      (최소 soft 격상). 그러지 않으면 POSIX 에서 CPU 조회만 깨졌을 때 exit 0 = 조용한 allow 가
    #      열린다(codex R1 #4).
    #   ⓒ Windows 호스트(`_is_windows_host` — MSYS/Cygwin 파이썬·Git Bash 포함)에서는 ⓑ 도 ⓐ 로
    #      접는다: 그 플랫폼에서 상시 soft 가 되면 `completion_guard._soft_kind` 가 skip_soft 로
    #      바뀌어 완료 검증이 영구 skip 된다(exit 계약 변경).
    fleet_cpu_top = []
    fleet_lines = None       # 판정에 쓴 **그 ps 스냅샷**(진단도 같은 스냅샷에서 뽑는다)
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
                fleet_lines = cpu_lines
    if fleet_reason not in ("ok", "override", "absent", "unsupported") and not _is_windows_host():
        errors.append("fleet_cpu(ps)")

    # ★R1(봉인표 ③): 이 축이 **연속으로** hard 인 시간을 재고 상한(15분)을 넘으면 hard 를 내린다.
    #   여기서 재는 이유: `evaluate` 는 순수 함수라 파일 I/O 를 두지 않는다(테스트가 손으로 만든
    #   measured 로 그대로 부른다). 임계 비교는 evaluate 와 **같은 규칙**(value >= hard)을 쓴다.
    _fleet_hard_thr = getattr(a, "fleet_cpu_hard", FLEET_CPU_HARD_DEFAULT)
    if fleet_cpu_ratio is None or _fleet_hard_thr is None:
        _fleet_state = "unmeasured"
    elif fleet_cpu_ratio >= _fleet_hard_thr:
        _fleet_state = "hard"
    else:
        _fleet_state = "below"
    _would_hard = _fleet_state == "hard"
    _hold_ovr = getattr(a, "fleet_cpu_hold_override", None)
    if _hold_ovr is None and _fleet_ovr is not None:
        # 값 자체가 주입된 호출(self-test·검체)은 래치 파일을 **읽지도 쓰지도 않는다**(밀폐).
        fleet_hold, fleet_hold_reason, fleet_hold_expired = None, "axis_override", False
    else:
        # ★R2: 래치는 **이 호출의 hard 임계별**이다 — 다른 임계 호출이 남의 연속 시계를 못 지운다.
        fleet_hold, fleet_hold_reason, fleet_hold_expired = _fleet_hard_hold(
            _fleet_state, _hold_ovr, thr=_fleet_hard_thr)

    # ★R1(리뷰 major — 비밀값 전파): 진단(top)은 **hard 일 때만** 모은다. 종전엔 allow 에서도 모아
    #   `measured` 에 실렸고, 그것이 `boot-last.json`(bootstrap log.step)·편성 상태파일
    #   (`_gate_compact` 의 measured 보존)로 영속됐다. 회수 대상 지목이 필요한 것은 hard 뿐이다.
    fleet_cpu_procs = None
    if _would_hard and fleet_lines is not None:
        # ★진단 수집은 판정 **바깥**이다: 여기서 나는 어떤 예외도 측정을 무효로 만들지 않는다
        #   (최상위 경계가 그것을 exit 70='측정 실패'로 접기 때문 · codex R2 검체 5). 함수 안에도
        #   같은 격리가 있지만, 격리를 **호출 지점에서 선언**해 두어야 그 함수를 갈아끼워도 계약이 남는다.
        # ★수집 기준은 **원시값이 hard 임계 이상인가** 다 — 최종 verdict 가 아니다(codex R1-2 Q4).
        #   부트 유예·보류 상한으로 soft 가 된 순간에도 '왜 완화했나'를 설명할 기여자가 남아야 한다.
        try:
            fleet_cpu_top = _fleet_cpu_top(fleet_lines)
            _rows_all, _ = _fleet_rows(fleet_lines)
            fleet_cpu_procs = len(_rows_all) if _rows_all is not None else None
        except Exception:            # noqa: BLE001 — 진단이 판정을 죽이지 않는다는 계약
            fleet_cpu_top = []

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
            "fleet_cpu_procs": fleet_cpu_procs,      # 상위 N 이 전체 합을 설명하지 못할 수 있다
            "fleet_cpu_hold": (round(fleet_hold, 1) if fleet_hold is not None else None),
            "fleet_cpu_hold_reason": fleet_hold_reason,
            "fleet_cpu_hold_expired": fleet_hold_expired,
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

    hold_expired = bool(m.get("fleet_cpu_hold_expired"))

    def add(metric, value, soft, hard):
        """★WP-7 N 로 두 갈래가 늘었고, R1 에서 셋째가 늘었다.
          ① `hard is None` = **soft 전용 축**(hard 판정 자체가 없다). `load_ratio` 가 이쪽으로 간다 —
             호스트 부하에는 우리가 회수할 수 없는 성분이 섞이므로 그것으로 착수를 거부하지 않는다.
          ② 부트 유예: CPU 축(CPU_GRACE_AXES)의 hard 는 부트 후 300초 창에서 soft 로 내려간다.
             내려간 사실은 `boot_grace: true` 로 그 check 에 **남긴다**(조용한 완화 금지 — 사후에
             '왜 hard 가 아니었나' 를 판독할 수 있어야 한다).
          ③ ★보류 상한(봉인표 ③): `fleet_cpu_ratio` 가 **연속 15분** hard 였으면 그 뒤로는 soft 다
             (`hold_expired: true`). 이 축은 호스트 전체 함대 CPU 를 재면서 각 부서의 복구 허가에
             쓰이므로, 상한이 없으면 다른 부서의 부하가 전멸 부서의 복구를 무기한 막는다.
             ★이 함수는 순수하다 — 만료 판정 자체는 `measure()` 가 재서 `m` 으로 실어 온다."""
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
        if c["level"] == "hard" and hold_expired and metric == "fleet_cpu_ratio":
            c["level"] = "soft"
            c["hold_expired"] = True
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
    if getattr(a, "load_hard_ratio", None) is not None:
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
        if any(c.get("hold_expired") for c in checks):
            _stale = str(m.get("fleet_cpu_hold_reason") or "").endswith("_stale")
            print("fleet_cpu_hold_expired: 첫 hard 관측 이후 %ss(>=%ds) — 상한을 넘겨 soft 로 "
                  "내렸다(봉인표 ③: 다른 부서의 부하가 전멸 부서의 복구를 무기한 막지 않게). "
                  "포화가 끝나면 래치가 지워지고 이 축은 다시 hard 를 낼 수 있다.%s"
                  % (m.get("fleet_cpu_hold"), int(FLEET_CPU_HARD_MAX_HOLD_SECS),
                     (" ★그 구간에 **관측 공백**이 있었다(사유 %s) — 이 수치는 '차단한 시간'이 "
                      "아니라 첫 관측 이후의 벽시계다."
                      % m.get("fleet_cpu_hold_reason")) if _stale else ""))
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
                    # ★인자는 싣지 않는다(비밀값 전파 차단) — 회수에 필요한 것은 PID·실행 주체·CPU 다.
                    print("  ↳ fleet_cpu 기여 %.1f%% pid=%s %s(%s)"
                          % (row.get("pcpu", 0.0), row.get("pid"), row.get("exe", "?"),
                             row.get("owner", "?")))
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


def _finite_float(text):
    """argparse 실수 타입 — **유한한 수만** 받는다(nan·inf 거부 → EX_USAGE 64).

    ★R1(리뷰 blocking): `type=float` 는 `nan`·`inf` 를 정상 입력으로 받는다. NaN 은 이후 모든 비교가
      거짓이라 `--fleet-cpu-override nan` 이 **조용한 allow**(exit 0)를 만들고, `json.dumps` 는
      비표준 토큰 `NaN`/`Infinity` 를 계약 채널(stdout)로 흘려 엄격한 소비자를 깨뜨린다. 임계에
      NaN 을 주면 실제 ratio 가 1.0 이어도 `1.0 >= nan` 이 거짓이라 hard 가 사라진다.
    ★임계에 `inf`(축 무력화 관용)도 받지 않는다: 이 도구에는 축을 끄는 관용이 원래 없고
      (있었다면 `--fleet-cpu-hard inf` 를 쓰는 호출자가 있어야 하는데 저장소에 0건), 축을 끄는
      길을 '비표준 JSON 을 흘리는 입력'으로 열어 주는 것은 나쁜 거래다. 축을 사실상 끄려면
      유한한 큰 수(예: 1e9)를 준다 — 그 값은 JSON 으로 안전히 나간다."""
    try:
        v = float(text)
    except (TypeError, ValueError):
        raise argparse.ArgumentTypeError("실수가 아니다: %r" % (text,))
    if not math.isfinite(v):
        raise argparse.ArgumentTypeError(
            "유한한 실수가 아니다(nan·inf 금지 — 조용한 allow·비표준 JSON 차단): %r" % (text,))
    return v


def _nonneg_float(text):
    """유한 + **음수 아님**. 분율(비율) 인자 전용 — 음수 분율은 뜻이 없고, 임계에 음수가 들어가면
    모든 값이 hard 가 된다(반대 방향의 조용한 사고). 유한성 검사만으로는 유효한 설정이 되지 않는다."""
    v = _finite_float(text)
    if v < 0:
        raise argparse.ArgumentTypeError("음수 분율은 받지 않는다: %r" % (text,))
    return v


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
    c.add_argument("--context", type=_nonneg_float, default=None, help="자기보고 컨텍스트 %%")
    c.add_argument("--json", action="store_true")
    c.add_argument("--servers-soft", type=int, default=2)
    c.add_argument("--servers-hard", type=int, default=3)
    c.add_argument("--nodes-soft", type=int, default=12)
    c.add_argument("--nodes-hard", type=int, default=NODES_HARD_DEFAULT)
    # ★R2(리뷰 minor · R1-1 B4 문면 이행): **분율·백분율 인자는 음수도 거부**한다. 종전엔 이 축만
    #   `_finite_float` 라 `--load-soft-ratio -1` 이 통과했고, 그러면 **모든** load_ratio 가 soft 라
    #   게이트가 상시 exit 1 이 된다(반대 방향의 조용한 사고 — 임계에 음수가 들어가면 모든 값이
    #   트립이다). 같은 부류인 context/rate 백분율도 함께 조인다.
    c.add_argument("--load-soft-ratio", type=_nonneg_float, default=1.0)
    # ★WP-7 N: load 축은 soft 전용이 됐다 — 이 플래그는 **무동작**이다. 삭제하지 않는 이유는
    #   구 호출자가 넘기면 argparse 가 EX_USAGE(64)를 내고 소비부가 그것을 '측정 실패'로 loud 처리하기
    #   때문이다(회귀 방향이 더 나쁘다). 기본값과 다르게 주면 stderr 로 무동작임을 고지한다.
    #   ★R1(리뷰 minor): 판별을 '기본값과 다른가' 가 아니라 **명시 여부**로 한다 — 구 기본값을
    #     그대로 명시한 호출자(`--load-hard-ratio 2.0`)도 축이 사라진 사실을 통보받아야 한다.
    c.add_argument("--load-hard-ratio", type=_nonneg_float, default=None,
                   help="[무동작·0.14.31~] load_ratio 는 soft 전용. 함대 CPU 차단은 --fleet-cpu-hard "
                        "(종전 기본값 %s — 지금은 주든 안 주든 판정이 같다)" % LOAD_HARD_RATIO_DEFAULT)
    c.add_argument("--fleet-cpu-soft", dest="fleet_cpu_soft", type=_nonneg_float,
                   default=FLEET_CPU_SOFT_DEFAULT,
                   help="함대 %%CPU 합/100/ncpu 분율 soft 임계(기본 0.5 = 전 코어의 50%% 상당)")
    c.add_argument("--fleet-cpu-hard", dest="fleet_cpu_hard", type=_nonneg_float,
                   default=FLEET_CPU_HARD_DEFAULT,
                   help="같은 분율의 hard 임계(기본 1.0 = 전 코어 100%% 상당)")
    c.add_argument("--fleet-cpu-override", dest="fleet_cpu_override", type=_nonneg_float,
                   default=None,
                   help="테스트 주입 — fleet_cpu_ratio(분율)를 직접 주입하고 ps 조회를 생략")
    c.add_argument("--fleet-cpu-hold-override", dest="fleet_cpu_hold_override",
                   type=_finite_float, default=None,
                   help="테스트 주입 — 이 축이 연속 hard 로 머문 초(보류 상한 %ds 판정용 · "
                        "래치 파일 무접촉)" % int(FLEET_CPU_HARD_MAX_HOLD_SECS))
    c.add_argument("--boot-elapsed-override", dest="boot_elapsed_override", type=_finite_float,
                   default=None,
                   help="테스트 주입 — 데몬 부트 후 경과초(부트 유예 창 판정용 · boot-epoch mtime 대체)")
    c.add_argument("--context-soft", type=_nonneg_float, default=50.0)
    c.add_argument("--context-hard", type=_nonneg_float, default=60.0)
    c.add_argument("--servers-override", type=int, default=None, help="테스트 주입")
    c.add_argument("--nodes-override", type=int, default=None, help="테스트 주입")
    c.add_argument("--load-override", type=_nonneg_float, default=None,
                   help="테스트 주입 — 1분 부하는 음수일 수 없다")
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
    c.add_argument("--rate-soft", type=_nonneg_float, default=80.0, help="rate 5h used_pct soft 임계")
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
    """A13 타입드 exit 회귀 배터리 — 측정 없이 결정론(부작용 0).

    ★부작용 0 을 **강제**한다(R1 · codex 지적): 0.14.31 에서 이 모듈은 hard 보류 래치를
      `CYS_STATE_DIR ‖ ~/.cys/state` 아래에 쓴다. self-test 가 그 경로를 건드리면 '부작용 0' 이
      말뿐인 약속이 된다 — 실행 동안 `CYS_STATE_DIR` 을 임시 디렉터리로 고정하고 원복한다."""
    fails = []
    import shutil as _sh0
    import tempfile as _tf0
    _st_tmp = _tf0.mkdtemp(prefix="gate-selftest-")
    _st_saved = os.environ.get("CYS_STATE_DIR")
    os.environ["CYS_STATE_DIR"] = _st_tmp
    try:
        return _self_test_body(fails)
    finally:
        if _st_saved is None:
            os.environ.pop("CYS_STATE_DIR", None)
        else:
            os.environ["CYS_STATE_DIR"] = _st_saved
        _sh0.rmtree(_st_tmp, ignore_errors=True)


def _self_test_body(fails):

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
    #      ★R2(리뷰 minor · codex): 종전엔 `--load-override 59` 를 **그대로** 넣었는데 이 축의 판정
    #      입력은 load1 이 아니라 `load1/ncpu` 다 — 64코어 러너에서는 59/64=0.92 로 soft 에도 못 미쳐
    #      이 핀이 **기계에 따라 거짓 실패**했다. 그날의 사실은 "16코어에서 load1=59"(비율 3.6875)이므로
    #      비율을 보존해 재생한다(같은 판정을 어느 코어 수에서도 낸다).
    _replay_ratio = 59.0 / 16.0
    rc, doc = _cj(["check", "--json", "--servers-override", "0", "--nodes-override", "0",
                   "--load-override", repr(_replay_ratio * ncpu_local)] + ro, "0707-replay")
    chk(rc == EXIT_SOFT and _axis(doc, "load_ratio").get("level") == "soft",
        "07:07 스냅샷(load1/ncpu=3.6875) 재생이 soft 아님: rc=%r axis=%r"
        % (rc, _axis(doc, "load_ratio")))
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
    # ★R2(리뷰 minor): 기대값 분기도 프로덕션과 **같은 판정기**를 쓴다(`os.name` 하나가 아니다) —
    #   MSYS 파이썬(os.name=="posix")에서 이 핀이 거짓 실패하던 길.
    if _is_windows_host():
        chk(rc == EXIT_ALLOW, "Windows 에서 ps 조회 실패가 exit 를 바꿨다: rc=%r" % rc)
    else:
        chk(rc == EXIT_SOFT and "fleet_cpu(ps)" in ((doc.get("measured") or {})
                                                    .get("measure_errors") or []),
            "POSIX 에서 ps 조회 실패가 조용한 allow 로 접혔다: rc=%r m=%r"
            % (rc, (doc.get("measured") or {}).get("measure_errors")))
    # (g) `_fleet_cpu_percent` 순수 핀 — 합산·무매칭 0.0·형상 불일치·함대 행 파손·NaN
    #     ★열 계약이 `pid pcpu command` 로 바뀌었다(자기 제외를 PID 로 하기 위해 · R1 blocking 1).
    L = ["  101  10.5 /usr/local/bin/cysd --socket /x",
         "  102   4.5 /Users/user/.local/bin/claude --foo",
         "  103   1.0 /usr/bin/mediaanalysisd",
         "  104   2.0 /opt/serena --transport stdio",
         "  105   9.9 python3 /w/bin/javis_resource_gate.py check"]
    tot, why = _fleet_cpu_percent(L, self_pid=999999)
    chk(abs((tot or 0) - 17.0) < 1e-9 and why is None,
        "함대 %%CPU 합이 17.0(cysd10.5+claude4.5+serena2.0) 아님: %r/%r" % (tot, why))
    tot, why = _fleet_cpu_percent(["  1   1.0 /usr/bin/mediaanalysisd", "  2   0.0 /sbin/init"])
    chk(tot == 0.0 and why is None,
        "매칭 0건(깨끗한 기계)이 측정 불능으로 오분류: %r/%r" % (tot, why))
    tot, why = _fleet_cpu_percent(["root /sbin/init", "daemon /usr/sbin/cupsd"])
    chk(tot is None and why == "shape",
        "열 형상이 계약과 다른데 '0%%'로 위장됨: %r/%r" % (tot, why))
    tot, why = _fleet_cpu_percent(["  10.5 /usr/local/bin/cysd", "  4.5 /usr/bin/claude"])
    chk(tot is None and why == "shape",
        "구 2열 출력(pcpu,command)이 3열 계약으로 통과해 pid 를 CPU 로 읽었다: %r/%r" % (tot, why))
    tot, why = _fleet_cpu_percent(["  1   0.0 /sbin/init", "  2 BROKEN /usr/bin/codex"])
    chk(tot is None and why == "row_unparsed",
        "**함대 행** 파싱 실패가 조용히 건너뛰어 과소계상됨: %r/%r" % (tot, why))
    tot, why = _fleet_cpu_percent(["  1   0.0 /sbin/init", "  2 nan /usr/local/bin/cysd"])
    chk(tot is None and why == "row_nonfinite",
        "NaN 이 합에 섞여 모든 비교를 거짓으로 만들었다(조용한 allow): %r/%r" % (tot, why))
    # ★codex 위임 검체 9 — pid 열이 깨진 **함대 행**도 조용히 버리지 않는다(과소계상이 '성공'으로 나감)
    tot, why = _fleet_cpu_percent(["  1   2.0 tail -f x", "bad 90 /usr/bin/codex"], self_pid=999999)
    chk(tot is None and why == "row_unparsed",
        "pid 가 깨진 함대 행이 0 으로 접혔다: %r/%r" % (tot, why))
    # (g2) ★R1 blocking 1 — 자기 제외는 **PID** 다. 출력 파일명에 이 모듈 이름이 든 진짜 함대
    #      프로세스가 종전엔 통째로 빠졌다(정상 측정값 0 · 조용한 과소계상).
    tot, why = _fleet_cpu_percent(
        ["  1   0.0 /sbin/init",
         " 42  25.0 /opt/codex --output /tmp/javis_resource_gate.log"], self_pid=999999)
    chk((tot, why) == (25.0, None),
        "자기 제외가 여전히 명령줄 부분문자열이다(진짜 codex 를 뺐다): %r/%r" % (tot, why))
    tot, why = _fleet_cpu_percent(["  1   0.0 /sbin/init", " 42  25.0 /opt/codex"], self_pid=42)
    chk((tot, why) == (0.0, None), "PID 자기 제외가 동작하지 않는다: %r/%r" % (tot, why))
    # (g3) ★R1 blocking 2 — 소유권은 argv0(실행 주체)다. 아래는 **전부 비매칭**이어야 한다.
    for cmd_, tag in ((" 7  99.0 python3 /tmp/agy/report.py", "디렉터리 이름 agy"),
                      (" 7  99.0 grep claude", "인자에만 든 claude"),
                      (" 7  99.0 tail -f /x/claude-code/debug.log", "패키지 세그먼트가 인자(.js 아님)"),
                      (" 7  99.0 sh -c S=/tmp/claude-501/x; echo", "셸 명령 문자열 속 claude"),
                      (" 7  99.0 python3 -c codex", "코드 문자열 codex"),
                      (" 7  99.0 /Applications/Claude.app/Contents/MacOS/Claude", "macOS GUI 앱 번들"),
                      (" 7  99.0 /opt/codex-report --x", "codex- 접두 남의 프로그램"),
                      (" 7  99.0 python3 /home/u/.codex/report.py", "인자 속 .codex 경로"),
                      # ★codex 위임 검체(R1 R2)가 찾아낸 오탐 — 데이터 인자·런처 옵션값·하위 명령
                      (" 7  99.0 node /tmp/report.js /tmp/codex", "데이터 인자를 실행 주체로 승격"),
                      (" 7  99.0 python3 -c print(1) /tmp/serena", "코드 문자열 모드 뒤 경로"),
                      (" 7  99.0 npm --prefix codex test", "긴 옵션의 값이 프로그램으로 승격"),
                      (" 7  99.0 npm uninstall codex", "런처 하위 명령의 인자"),
                      (" 7  99.0 npx prettier codex", "런처가 실행하는 것은 첫 프로그램뿐"),
                      (" 7  99.0 python3 -m codex_like_thing", "-m 모듈 이름")):
        got = _fleet_cpu_percent(["  1   0.0 /sbin/init", cmd_], self_pid=999999)
        chk(got == (0.0, None), "B2 오탐 재발(%s): %r → %r" % (tag, cmd_, got))
    # (g4) ★진짜 함대 형상은 **전부 매칭**이어야 한다(과소계상 반대 방향의 음성 대조).
    for cmd_, tag in ((" 8  10.0 /Users/user/.local/share/claude/versions/2.1.261 -p", "버전 경로 직접 exec"),
                      (" 8  10.0 node /usr/lib/node_modules/@anthropic-ai/claude-code/cli.js",
                       "npm 번들 node 형상"),
                      (" 8  10.0 /Users/user/.codex/bin/codex-darwin-arm64 --child", "codex vendor native"),
                      (" 8  10.0 uvx --python 3.13 --from serena-agent==1.5.3 serena start-mcp-server",
                       "uvx 온디맨드 serena"),
                      (" 8  10.0 /usr/local/bin/cysd --socket /x", "데몬"),
                      # ★실측 형상(2026-09-08 이 기계) — 셋 다 R1 초안에서 **놓쳤던** 것들이다.
                      (" 8  10.0 /Applications/cys.app/Contents/MacOS/cysd",
                       "앱 번들 안의 우리 데몬(정확 이름이 번들 배제보다 우선)"),
                      (" 8  10.0 node /Users/user/.local/bin/codex "
                       "--dangerously-bypass-approvals-and-sandbox resume --last",
                       "node 래퍼가 실행하는 codex(비 .js 경로)"),
                      (" 8  10.0 /Users/user/.local/lib/node_modules/@openai/codex/node_modules/"
                       "@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex-code-mode-host",
                       "codex vendor native(basename 이 codex 가 아니다)"),
                      (" 8  10.0 claude --dangerously-skip-permissions", "맨 claude"),
                      (" 8  10.0 npx -y @google/gemini-cli", "짧은 옵션은 값이 없다"),
                      (" 8  10.0 npx @openai/codex@1.2.3", "버전 지정 런처 스펙"),
                      (" 8  10.0 uv run serena start-mcp-server", "런처 하위 명령 1회 건너뛰기"),
                      (" 8  10.0 uvx --from serena-agent==1.5.3 serena", "긴 옵션 값 건너뛰기"),
                      # ★실측 2차(2026-09-08) — R1 1차 조임이 **놓쳤던** 두 형상
                      (" 8  10.0 node /Users/user/.local/bin/codex exec -m gpt-6-astra -s read-only",
                       "대상 프로그램의 인자에 -m 이 있는 형상(코드모드 판정은 순서를 본다)"),
                      (" 8  10.0 /Users/user/.local/bin/uv tool uvx --python 3.13 "
                       "--from serena-agent==1.5.3 serena start-mcp-server",
                       "중첩 런처(uv → tool → uvx → serena)")):
        got = _fleet_cpu_percent(["  1   0.0 /sbin/init", cmd_], self_pid=999999)
        chk(got == (10.0, None), "진짜 함대 형상을 놓쳤다(%s): %r → %r" % (tag, cmd_, got))
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
    # (h2) ★R1 minor — 프로덕션 **기본 경로**(CYS_SOCKET 미설정)를 핀한다. 상수를 바꾸면 유예가
    #      조용히 `epoch_missing` 으로 사라진다(방향은 안전하나 봉인표 ③ 완화 장치가 없어진다).
    #      ★R2(리뷰 minor): 종전 핀은 `HOME` 을 덮어써 POSIX 문자열을 기대했다 — Windows 의
    #      `expanduser` 는 `USERPROFILE` 을 보므로 그 기계에서 거짓 실패한다. 핀의 대상은
    #      **상수의 모양**(`~/.local/state/cys` + basename)이므로 홈 확장은 양쪽 다 같은 함수로 하고
    #      세그먼트만 핀한다.
    saved_sock = os.environ.pop("CYS_SOCKET", None)
    saved_lane = os.environ.pop("CYS_GATE_LANE_SOCKET", None)
    try:
        want = os.path.join(os.path.expanduser("~"), ".local", "state", "cys",
                            BOOT_EPOCH_BASENAME)
        chk(_boot_epoch_path() == want,
            "boot-epoch 기본 경로(DEFAULT_STATE_DIR)가 바뀌었다: %r(기대 %r)"
            % (_boot_epoch_path(), want))
    finally:
        if saved_sock is not None:
            os.environ["CYS_SOCKET"] = saved_sock
        if saved_lane is not None:
            os.environ["CYS_GATE_LANE_SOCKET"] = saved_lane
    # (i) ★계측 타당성 음성 대조: 신설 축이 **없던** 코드에서는 이 판정이 성립할 수 없다.
    #     load 축 hard 키가 None 이라는 사실 자체가 'soft 전용 강등'의 유일한 기계 증거다.
    chk(_axis(_cj(q + det_boot + ["--fleet-cpu-override", "0.0"], "shape")[1],
              "load_ratio").get("hard") is None,
        "load_ratio 에 hard 임계가 남아 있다(강등 미적용)")

    # ⑬ ★codex 위임 검체(R2)에서 살아남은 반례 + R1 리뷰 반영 반례.
    # (a) 합계 오버플로: 각 항이 유한해도 합이 inf 면 측정 실패다(JSON 에 Infinity 를 흘리지 않는다)
    tot, why = _fleet_cpu_percent([" 1 1e308 /opt/codex", " 2 1e308 /opt/claude"], self_pid=999999)
    chk(tot is None and why == "sum_overflow",
        "유한한 항들의 합이 inf 인데 측정 성공으로 나갔다(비표준 JSON): %r/%r" % (tot, why))
    # (b) 진단 함수는 판정을 죽이지 않는다 — `_fleet_cpu_top` 이 터져도 exit 70 이 되면 안 된다
    #     (top 수집은 이제 **hard 일 때만** 이라 hard 값을 내는 대역을 쓴다)
    saved_top = _g["_fleet_cpu_top"]
    saved_cpu2 = _g["_ps_cpu_lines"]
    try:
        _g["_ps_cpu_lines"] = lambda: ([" 4242 99999.0 /usr/local/bin/cysd"], None)
        _g["_fleet_cpu_top"] = lambda *_a, **_k: (_ for _ in ()).throw(RuntimeError("boom"))
        rc, doc = _cj(["check", "--json", "--servers-override", "0", "--nodes-override", "0",
                       "--load-override", "0.0", "--fleet-cpu-hold-override", "0"]
                      + roster_only + det_boot, "top-raise")
    finally:
        _g["_fleet_cpu_top"] = saved_top
        _g["_ps_cpu_lines"] = saved_cpu2
    chk(rc != EXIT_INTERNAL and (doc.get("measured") or {}).get("fleet_cpu_ratio") is not None,
        "진단용 top 의 예외가 정상 측정을 exit 70 으로 만들었다: rc=%r" % rc)
    # (c) ★R1 major — 진단에 **명령줄 인자를 싣지 않는다**(비밀값이 measured 로 영속되던 길).
    top = _fleet_cpu_top([" 9 50.0 /opt/codex --api-key secret123 --output /x"], self_pid=999999)
    chk(top and top[0].get("pid") == 9 and top[0].get("exe") == "codex"
        and top[0].get("owner") == "codex" and "cmd" not in top[0]
        and not any("secret123" in str(v) for v in top[0].values()),
        "진단 행에 원시 명령줄(비밀값)이 남아 있다: %r" % (top,))
    # (d) ★R1 major — allow 판정에서는 top 자체를 모으지 않는다(정상 부트마다 argv 가 남지 않게)
    _, doc = _cj(q + det_boot + ["--fleet-cpu-override", "0.1"], "top-not-on-allow")
    chk((doc.get("measured") or {}).get("fleet_cpu_top") == [],
        "allow 인데 fleet_cpu_top 이 채워져 boot-last.json 으로 영속된다: %r"
        % ((doc.get("measured") or {}).get("fleet_cpu_top"),))
    # (e) 밀폐 확인: fleet override 를 주면 CPU 조회 스폰이 0 이고 **래치 파일도 안 건드린다**
    _spawn2 = []
    saved_cpu3 = _g["_ps_cpu_lines"]
    saved_hold = _g["_fleet_hard_hold"]
    _hold_calls = []
    try:
        _g["_ps_cpu_lines"] = lambda: _spawn2.append(1) or (None, "absent")
        _g["_fleet_hard_hold"] = lambda *_a, **_k: _hold_calls.append(1) or (None, "x", False)
        _cj(base_argv + ["--json"], "override-no-spawn")
    finally:
        _g["_ps_cpu_lines"] = saved_cpu3
        _g["_fleet_hard_hold"] = saved_hold
    chk(not _spawn2, "--fleet-cpu-override 를 줬는데 ps 조회가 일어났다(밀폐 파괴): %r" % _spawn2)
    chk(not _hold_calls, "--fleet-cpu-override 를 줬는데 래치 경로를 건드렸다(밀폐 파괴)")
    # (f) ★R1 blocking 3(봉인표 ③) — hard 보류 상한. 연속 hard 가 상한을 넘으면 soft 로 내려간다.
    for hold, want_rc, want_exp in ((0.0, EXIT_HARD, False),
                                    (FLEET_CPU_HARD_MAX_HOLD_SECS - 1, EXIT_HARD, False),
                                    (FLEET_CPU_HARD_MAX_HOLD_SECS, EXIT_SOFT, True),
                                    (FLEET_CPU_HARD_MAX_HOLD_SECS * 4, EXIT_SOFT, True)):
        rc, doc = _cj(q + det_boot + ["--fleet-cpu-override", "1.5",
                                      "--fleet-cpu-hold-override", str(hold)], "hold=%s" % hold)
        ax = _axis(doc, "fleet_cpu_ratio")
        chk(rc == want_rc and bool(ax.get("hold_expired")) is want_exp,
            "보류 상한 판정 이탈(hold=%s): rc=%r(기대 %r) axis=%r" % (hold, rc, want_rc, ax))
    # (f2) 상한 만료가 **다른 축의 hard 는 건드리지 않는다**(강등은 이 축 하나다)
    rc, doc = _cj(["check", "--json", "--servers-override", "99", "--nodes-override", "0",
                   "--load-override", "0.0", "--fleet-cpu-override", "1.5",
                   "--fleet-cpu-hold-override", str(FLEET_CPU_HARD_MAX_HOLD_SECS)]
                  + roster_only + det_boot, "hold-other-axis")
    chk(rc == EXIT_HARD and _axis(doc, "servers").get("level") == "hard",
        "보류 상한 만료가 CPU 아닌 축(servers)의 hard 까지 내렸다: rc=%r" % rc)
    # (f3) 만료 사실은 **warnings 가 아니라** check 에 남는다(completion_guard 완전일치 계약 보존)
    rc, doc = _cj(q + det_boot + ["--fleet-cpu-override", "1.5", "--fleet-cpu-hold-override",
                                  str(FLEET_CPU_HARD_MAX_HOLD_SECS)], "hold-warnings")
    chk(doc.get("warnings") == ["context_unmeasured"],
        "보류 상한 만료가 warnings 를 늘렸다(_soft_kind 분기 파손): %r" % doc.get("warnings"))
    # (f4) `_fleet_hard_hold` 순수/파일 핀 — 무장→유지→만료(재무장 없음)·below 는 삭제·
    #      unmeasured 는 **무접촉**(간헐 실패가 연속 시계를 0으로 되돌리지 않는다)
    _hd = _tf.mkdtemp()
    saved_state = os.environ.get("CYS_STATE_DIR")
    saved_sock2 = os.environ.pop("CYS_SOCKET", None)
    try:
        os.environ["CYS_STATE_DIR"] = _hd
        t0 = time.time()
        h, why, exp = _fleet_hard_hold("hard", now=t0)
        chk((h, why, exp) == (0.0, "armed", False), "첫 hard 가 무장이 아님: %r/%r/%r" % (h, why, exp))
        chk(os.path.exists(_fleet_hold_path()), "래치 파일이 안 생겼다: %s" % _fleet_hold_path())
        h, why, exp = _fleet_hard_hold("hard", now=t0 + 100)
        chk(abs(h - 100) < 2 and why == "held" and not exp, "연속 hard 100s 가 유지가 아님: %r" % (why,))
        h, why, exp = _fleet_hard_hold("unmeasured", now=t0 + 200)
        chk((h, why, exp) == (None, "unmeasured", False), "unmeasured 가 무접촉이 아님: %r" % (why,))
        h, why, exp = _fleet_hard_hold("hard", now=t0 + 300)
        chk(abs(h - 300) < 2 and why == "held",
            "측정 실패 한 번이 연속 시계를 되돌렸다(유계 소실): %r/%r" % (h, why))
        h, why, exp = _fleet_hard_hold("hard", now=t0 + FLEET_CPU_HARD_MAX_HOLD_SECS + 1)
        chk(exp and why == "expired", "상한 초과가 만료로 안 잡힘: %r/%r" % (h, why))
        h2, why2, exp2 = _fleet_hard_hold("hard", now=t0 + FLEET_CPU_HARD_MAX_HOLD_SECS + 2)
        chk(exp2 and why2 == "expired",
            "만료 뒤 래치가 재무장돼 다음 호출이 다시 hard 를 만난다(창 소모 반례): %r/%r" % (h2, why2))
        _fleet_hard_hold("below", now=t0 + FLEET_CPU_HARD_MAX_HOLD_SECS + 3)
        chk(not os.path.exists(_fleet_hold_path()), "포화가 끝났는데 래치가 안 지워졌다")
        h, why, exp = _fleet_hard_hold("hard", now=t0 + FLEET_CPU_HARD_MAX_HOLD_SECS + 4)
        chk((h, why, exp) == (0.0, "armed", False), "포화 재개 시 시계가 0에서 다시 시작하지 않는다")
        # ★R3(판정자 핀 1b/1c/1d · codex blocking) — 만료는 **생성 전용 표식**이 지킨다.
        #   ① `below` 는 레코드와 표식을 **둘 다** 지운다(위 armed 가 그 간접 증거지만, 표식을
        #      직접 보지 않으면 '레코드만 지웠는데 표식이 없어서 통과' 와 구별되지 않는다).
        _mk = _fleet_expired_mark_path(_fleet_hold_path())
        _fleet_hard_hold("hard", now=t0 + 2 * FLEET_CPU_HARD_MAX_HOLD_SECS + 10)
        chk(os.path.exists(_mk), "만료를 냈는데 표식이 안 생겼다: %s" % _mk)
        _fleet_hard_hold("below", now=t0 + 2 * FLEET_CPU_HARD_MAX_HOLD_SECS + 11)
        chk(not os.path.exists(_mk), "below 가 만료 표식을 안 지웠다(축이 영구 권고로 죽는다)")
        #   ② **경합 + 시계 역행**: A 의 병합 재읽기와 `os.replace` 사이에 B 가 만료를 공개하면,
        #      A 의 낡은 `expired=False` 가 그것을 덮으면 안 된다. 덮으면 이어진 역행이 `below`
        #      관측 없이 재무장해 상한만큼을 다시 막는다(그 회귀를 여기서 잡는다).
        _fleet_hard_hold("below", now=t0 + 3000)
        _fleet_hold_write(_fleet_hold_path(), {"since": 1000.0, "last": 1000.0, "expired": False})
        _race = {"fired": False, "b": None}
        _rp = os.replace

        def _race_replace(_a, _b):
            if not _race["fired"] and str(_b) == _fleet_hold_path():
                _race["fired"] = True
                _race["b"] = _fleet_hard_hold("hard", now=1900.0)
            return _rp(_a, _b)

        try:
            os.replace = _race_replace
            _a_ret = _fleet_hard_hold("hard", now=1899.0)
        finally:
            os.replace = _rp
        chk(bool(_race["b"]) and _race["b"][2] is True,
            "경합 검체가 만료를 공개하지 못했다(계측 타당성): %r" % (_race["b"],))
        _rec_r, _why_r = _fleet_hold_read(_fleet_hold_path())
        chk(bool(_rec_r) and _rec_r["expired"] is True,
            "공개된 만료가 낡은 쓰기에 덮였다: %r(why=%r · A=%r)" % (_rec_r, _why_r, _a_ret))
        chk(_a_ret[2] is True,
            "파일은 만료인데 같은 호출의 반환이 아니다(codex '파일=True·반환=False'): %r" % (_a_ret,))
        _c_ret = _fleet_hard_hold("hard", now=997.0)          # 시계 역행
        chk(_c_ret[1] != "armed" and _c_ret[2] is True,
            "시계 역행이 below 없이 재무장해 만료를 취소했다: %r" % (_c_ret,))
        chk(_fleet_hard_hold("hard", now=1896.0)[2] is True,
            "공개된 완화 뒤 상한 미만 구간이 다시 차단됐다")
        _fleet_hard_hold("below", now=t0 + 3100)              # 뒷정리(이후 검체 오염 금지)
        # ★codex 위임 검체(R2) — 임시 파일 이름이 호출마다 달라야 한다(같은 프로세스 두 호출이
        #   서로의 tmp 를 지우면 한쪽이 unbounded_io=만료로 번진다).
        _tmps = []
        _saved_replace = os.replace
        try:
            os.replace = lambda a, b: (_tmps.append(a), _saved_replace(a, b))[1]
            _fleet_hold_write(_fleet_hold_path(), 1.0)
            _fleet_hold_write(_fleet_hold_path(), 2.0)
        finally:
            os.replace = _saved_replace
        chk(len(_tmps) == 2 and _tmps[0] != _tmps[1],
            "래치 임시 이름이 호출마다 같다(같은 프로세스 경합에서 만료로 번진다): %r" % (_tmps,))
        # ★허용오차 안의 미래 저장값이 **음수 보류초**를 만들지 않는다
        with open(_fleet_hold_path(), "w", encoding="utf-8") as f:
            f.write("%r\n" % (t0 + 1.0))
        h, why, exp = _fleet_hard_hold("hard", now=t0)
        chk(h == 0.0 and why == "held" and not exp,
            "허용오차 안의 미래 저장값이 음수 보류초를 냈다: %r/%r" % (h, why))
        # 파손 래치(비수치·NaN)는 '지금 무장' 으로 접는다 — 만료 비교가 영원히 거짓이 되지 않게
        with open(_fleet_hold_path(), "w", encoding="utf-8") as f:
            f.write("nan\n")
        h, why, exp = _fleet_hard_hold("hard", now=t0 + 500)
        chk((h, why, exp) == (0.0, "armed", False), "NaN 래치가 만료 비교를 영원히 막았다: %r" % (why,))
    finally:
        if saved_state is None:
            os.environ.pop("CYS_STATE_DIR", None)
        else:
            os.environ["CYS_STATE_DIR"] = saved_state
        if saved_sock2 is not None:
            os.environ["CYS_SOCKET"] = saved_sock2
        _sh.rmtree(_hd, ignore_errors=True)
    # (g5) ★R1 blocking 4 — 신설 실수 인자의 nan/inf 는 EX_USAGE(64)다(조용한 allow·비표준 JSON 차단)
    for bad in (["--fleet-cpu-override", "nan"], ["--fleet-cpu-soft", "nan"],
                ["--fleet-cpu-hard", "nan"], ["--fleet-cpu-hard", "inf"],
                ["--fleet-cpu-override", "-1"], ["--boot-elapsed-override", "nan"],
                ["--context", "nan"], ["--load-override", "inf"]):
        with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()):
            rc = main(q + det_boot + bad)
        chk(rc == EXIT_USAGE, "%r 가 EX_USAGE(64) 로 거부되지 않았다: rc=%r" % (bad, rc))
    # (g6) 음성 대조: 같은 자리에 **유한한** 값이면 정상 판정이다(검증이 정상 입력을 막지 않는다)
    rc, _doc = _cj(q + det_boot + ["--fleet-cpu-override", "0.0"], "finite-ok")
    chk(rc == EXIT_ALLOW, "유한한 override 가 거부됐다: rc=%r" % rc)
    # (h3) ★R1 minor — Windows 판별이 인터프리터 os.name 하나가 아니다(MSYS python + MSYS ps 조합)
    #      ★R2: Git Bash 형상은 `MSYSTEM` **하나가 아니라** Windows 환경 상속(WINDIR 등)과 **함께**
    #      온다 — 그 조합을 재현한다. `MSYSTEM` 단독이 Windows 로 접히면 안 된다는 반대 방향은
    #      아래 (r3) 이 따로 핀한다(POSIX 측정 실패의 조용한 allow 차단).
    saved_msys = os.environ.get("MSYSTEM")
    saved_windir = os.environ.get("WINDIR")
    saved_cpu4 = _g2["_ps_cpu_lines"]
    try:
        os.environ["MSYSTEM"] = "MINGW64"
        os.environ["WINDIR"] = "C:\\Windows"
        _g2["_ps_cpu_lines"] = lambda: (None, "failed")
        rc, doc = _cj(q + det_boot, "msys-ps-failed")
    finally:
        _g2["_ps_cpu_lines"] = saved_cpu4
        for _k, _v in (("MSYSTEM", saved_msys), ("WINDIR", saved_windir)):
            if _v is None:
                os.environ.pop(_k, None)
            else:
                os.environ[_k] = _v
    chk(rc == EXIT_ALLOW and not [e for e in ((doc.get("measured") or {})
                                              .get("measure_errors") or []) if "fleet" in e],
        "Git Bash(MSYS) 에서 ps 실패가 상시 soft 로 굳어 완료 검증이 영구 skip 된다: rc=%r m=%r"
        % (rc, (doc.get("measured") or {}).get("measure_errors")))
    # (h4) 음성 대조: MSYS 가 아니면 같은 실패가 여전히 측정 실패(최소 soft)다
    saved_cpu5 = _g2["_ps_cpu_lines"]
    try:
        _g2["_ps_cpu_lines"] = lambda: (None, "failed")
        rc2, doc2 = _cj(q + det_boot, "posix-ps-failed")
    finally:
        _g2["_ps_cpu_lines"] = saved_cpu5
    if not _is_windows_host():
        chk(rc2 == EXIT_SOFT, "POSIX 에서 ps 조회 실패가 조용한 allow 로 접혔다: rc=%r" % rc2)
    # (h5) `unsupported`(플래그 미지원)는 **어느 플랫폼에서도** 축 부재와 같다(exit 계약 불변)
    saved_cpu6 = _g2["_ps_cpu_lines"]
    try:
        _g2["_ps_cpu_lines"] = lambda: (None, "unsupported")
        rc3, doc3 = _cj(q + det_boot, "ps-unsupported")
    finally:
        _g2["_ps_cpu_lines"] = saved_cpu6
    chk(rc3 == EXIT_ALLOW and _axis(doc3, "fleet_cpu_ratio").get("level") == "unavailable",
        "`-axo` 미지원 ps 가 측정 실패로 분류돼 상시 soft 가 된다: rc=%r" % rc3)
    # (i2) ★R1 minor — `--load-hard-ratio` 무동작 고지는 **명시 여부**로 낸다(구 기본값 명시 포함)
    for given, want in (("2.0", True), ("3.0", True)):
        buf = io.StringIO()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(buf):
            main(q + det_boot + ["--fleet-cpu-override", "0.0", "--load-hard-ratio", given])
        chk(("무동작" in buf.getvalue()) is want,
            "--load-hard-ratio %s 고지 이탈: %r" % (given, buf.getvalue()[:120]))
    buf = io.StringIO()
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(buf):
        main(q + det_boot + ["--fleet-cpu-override", "0.0"])
    chk("무동작" not in buf.getvalue(), "플래그를 안 줬는데 무동작 고지가 나갔다")

    # ══ ★R2 리뷰 반영 핀 ══════════════════════════════════════════════════════
    # (r1) blocking — '실행 형상' 상수의 정본은 preflight 하나다. 값 복사를 금지한다.
    chk(_PF is not None, "javis_preflight 를 import 하지 못했다 — 상수 정본이 폴백으로 굳었다")
    if _PF is not None:
        for _name, _got in (("_CLAUDE_EXE_NAMES", _CLAUDE_EXE_NAMES),
                            ("_CLAUDE_INSTALL_MARKER", _CLAUDE_INSTALL_MARKER),
                            ("_CLAUDE_NPM_MARKER", _CLAUDE_NPM_MARKER),
                            ("_JS_SUFFIXES", _JS_SUFFIXES),
                            ("_JS_RUNTIME_NAMES", _PF_JS_RUNTIME_NAMES)):
            chk(getattr(_PF, _name) == _got,
                "preflight 정본과 게이트 상수가 갈라졌다(%s): %r ≠ %r"
                % (_name, getattr(_PF, _name), _got))
        chk(FLEET_ARGV0_MARKERS[0][0] == _PF._CLAUDE_INSTALL_MARKER
            and FLEET_JS_BUNDLE_MARKERS[0][0] == _PF._CLAUDE_NPM_MARKER
            and set(FLEET_EXE_SUFFIXES) == {".exe", ".cmd"}
            and FLEET_JS_RUNTIMES >= {"node", "bun"},
            "게이트 표가 preflight 상수에서 파생되지 않았다(값 복사 잔존)")
        # (r2) ★판정 **규칙**은 일부러 다르다 — 그 차이를 목록으로 못박는다(codex R2 D1 최소 보완).
        #      preflight strict 는 argv0 basename 을 **소문자로 접어** 비교하므로 macOS GUI 앱
        #      번들(`/Applications/Claude.app/Contents/MacOS/Claude`)을 claude 로 인정한다 — 그 축은
        #      '놓치지 않는 것'이 안전 방향이라 옳지만, CPU **합**에 그것이 실리면 조직 기동 거부
        #      (B2 재발)다. 반대로 이 축은 실행 대상을 첫 비옵션 토큰 하나로 좁혀 데이터 인자를
        #      승격시키지 않는다. 아래 표가 바뀌면(= preflight 파서가 바뀌면) 이 핀이 먼저 운다.
        import shlex as _shlex
        _diff_want = {
            # cmd: (gate 가 claude 로 보나, preflight strict 가 claude 로 보나)
            "/Applications/Claude.app/Contents/MacOS/Claude": (False, True),
            "node /usr/local/bin/claude": (True, False),
            "node /tmp/report.js /x/claude-code/cli.js": (False, True),
            "claude --dangerously-skip-permissions": (True, True),
            "node /usr/lib/node_modules/@anthropic-ai/claude-code/cli.js": (True, True),
            "tail -f /x/claude-code/debug.log": (False, False),
        }
        for _cmd, _want in _diff_want.items():
            try:
                _toks = _shlex.split(_cmd)
            except ValueError:
                _toks = _cmd.split()
            _got = (_fleet_owner(_cmd) == "claude", bool(_PF._is_claude_command(_toks, strict=True)))
            chk(_got == _want,
                "게이트↔preflight 의도된 차이표 이탈(%s): %r ≠ 기대 %r" % (_cmd, _got, _want))
    # (r3) MSYSTEM **단독**은 Windows 증명이 아니다(R1 반박 기각 · 조용한 allow 차단)
    _wenv = {k: os.environ.get(k) for k in ("MSYSTEM", "WINDIR", "SYSTEMROOT", "OS")}
    try:
        for k in ("WINDIR", "SYSTEMROOT", "OS"):
            os.environ.pop(k, None)
        os.environ["MSYSTEM"] = "MINGW64"
        if os.name != "nt" and sys.platform not in ("msys", "cygwin"):
            chk(_is_windows_host() is False,
                "MSYSTEM 하나로 Windows 판정 — POSIX 측정 실패가 조용한 allow 가 된다")
            os.environ["WINDIR"] = "C:\\Windows"
            chk(_is_windows_host() is True, "MSYSTEM+WINDIR 조합이 Windows 로 안 잡힌다")
    finally:
        for k, v in _wenv.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
    # (r4) ps 디코딩 실패(UnicodeDecodeError)가 exit 70 으로 새지 않는다 — Windows 분류에 닿는다
    _saved_run = subprocess.run

    def _decode_boom(*_a, **_k):
        raise UnicodeDecodeError("utf-8", b"\xff", 0, 1, "invalid start byte")
    try:
        subprocess.run = _decode_boom
        _lines, _why = _ps_cpu_lines()
    finally:
        subprocess.run = _saved_run
    chk((_lines, _why) == (None, "failed"),
        "ps 디코딩 실패가 '측정 실패' 로 분류되지 않는다(최상위 경계로 새어 exit 70): %r/%r"
        % (_lines, _why))
    # (r5) 소유권 — R2 가 닫은 오탐과, 새로 잡게 된 진짜 형상
    for _cmd, _want, _tag in (
            ("python3 report.py /tmp/serena", None, "상대 스크립트 뒤 데이터 인자"),
            ("node report.js /tmp/codex", None, "상대 스크립트 뒤 데이터 인자(JS)"),
            ("python3 -cprint(1) /tmp/serena", None, "값이 붙은 코드모드 클러스터"),
            ("python3 - /tmp/serena", None, "stdin 스크립트"),
            ("npm run codex", None, "npm 스크립트 이름"),
            ("yarn codex", None, "yarn 스크립트 이름"),
            ("pnpm run gemini", None, "pnpm 스크립트 이름"),
            ("npm exec codex", "codex", "npm exec 는 프로그램 실행이다"),
            ("node --require preload.js /opt/node_modules/@openai/codex/bin/codex.js",
             "codex", "값을 먹는 긴 옵션 뒤의 진짜 대상"),
            ("python3 -W ignore /opt/venv/bin/serena start-mcp-server",
             "serena", "값을 먹는 짧은 옵션 뒤의 진짜 대상"),
            ("python3 -Wignore /opt/venv/bin/serena", "serena", "붙은 값은 다음 토큰을 안 먹는다"),
            ("node --max-old-space-size=4096 /x/bin/codex", "codex", "= 형 긴 옵션"),
            # ★codex 위임 검체(R2) 가 찾아낸 것들 — D3~D7
            ("python3 -Wmodule /tmp/serena", "serena", "값 안의 m 을 모듈 모드로 오독(D3)"),
            ("node -rpreload.js /tmp/codex", "codex", "값 안의 p 를 print 모드로 오독(D3)"),
            ("node -Cdevelopment /tmp/codex", "codex", "값 안의 e 를 eval 모드로 오독(D3)"),
            ("uv codex", None, "uv 는 하위 명령 없이 프로그램을 실행하지 않는다(D4)"),
            ("pipx codex", None, "pipx 도 동일(D4)"),
            ("yarn npx codex", None, "스크립트 이름 자리의 토큰이 런처로 승격(D5)"),
            ("uvx npm run codex", None, "중첩 런처가 규칙을 갈아탄 뒤의 스크립트 이름(D5 음성)"),
            ("npm exec -- codex", "codex", "옵션 끝 표시가 실행 대상을 삼킴(D6)"),
            ("npx --yes codex", "codex", "불리언 긴 옵션이 대상을 값으로 먹음(D7)"),
            ("pipx run serena", "serena", "pipx run 은 정상 형상(D4 음성 대조)"),
            # ★R3(판정자 핀 2 · codex major) — 해석 못 한 옵션 뒤 토큰의 실행 주체 승격(B2 재발).
            #   미열거 **긴** 옵션의 값이 첫 경로 토큰이 되어 소유자로 승격되던 길:
            ("node --diagnostic-dir /tmp/codex /tmp/report.js", None, "미열거 긴 옵션의 값(R3)"),
            ("node --cpu-prof-dir /tmp/codex /tmp/report.js", None, "미열거 긴 옵션의 값(R3)"),
            ("node --test-reporter-destination /tmp/serena /tmp/x.js", None,
             "미열거 긴 옵션의 값(R3)"),
            ("node --run=build /tmp/codex", None, "= 형이어도 실행 모드를 모르면 포기(R3·codex)"),
            ("python3 -Zq /tmp/serena", None, "미지 짧은 문자가 든 클러스터(R3)"),
            #   런처: 짧은 값 옵션의 값 · 셸 문자열 모드 · 허용 하위명령의 **반복** 소비:
            ("npx -p @openai/codex node /tmp/report.js", None, "런처 짧은 값 옵션 -p 의 값(R3)"),
            ("npx -c codex", None, "npx -c 는 셸 문자열 모드다(R3·codex)"),
            ("npm exec exec codex", None, "허용 하위명령 반복 소비(R3)"),
            ("uv run run codex", None, "허용 하위명령 반복 소비(R3)"),
            ("pnpm dlx exec codex", None, "dlx 뒤의 exec 는 패키지 자리다(R3·codex)"),
            ("npx --unknown-flag codex", None, "표에 없는 런처 옵션(R3)"),
            #   음성 대조 — 미계상 방향으로 **과교정**하면 축이 죽는다(전부 계속 양성이어야 한다):
            ("node --enable-source-maps /x/bin/codex", "codex", "표에 있는 불리언 긴 옵션(R3)"),
            ("python3 -u /opt/venv/bin/serena", "serena", "표에 있는 불리언 짧은 옵션(R3)"),
            ("uv tool run serena start-mcp-server", "serena", "uvx = uv tool run 별칭(R3·codex)"),
            ("npm exec -w pkg codex", "codex", "npm -w 는 값 소비다(R3)"),
            ("npm exec -p codex", "codex",
             "npm 의 -p 는 parseable **불리언** — npx 의 -p(package·값)와 다르다(R3·codex)")):
        chk(_fleet_owner(_cmd) == _want,
            "소유권 R2 규칙 이탈(%s): %r → %r(기대 %r)" % (_tag, _cmd, _fleet_owner(_cmd), _want))
    # (r6) 래치 — 레인 전용 변수 · 임계 격리 · 만료 영속 · 읽기 불능 · 관측 공백 표기 · 해시 불멸
    _hd2 = _tf.mkdtemp()
    _st2 = os.environ.get("CYS_STATE_DIR")
    _sk2 = os.environ.pop("CYS_SOCKET", None)
    _ln2 = os.environ.pop("CYS_GATE_LANE_SOCKET", None)
    try:
        os.environ["CYS_STATE_DIR"] = _hd2
        t1 = 1_000_000.0
        chk(_fleet_hard_hold("hard", now=t1, thr=1.0)[1] == "armed", "임계별 무장 실패")
        chk(_fleet_hard_hold("below", now=t1 + 10, thr=2.0)[1] == "cleared",
            "다른 임계의 below 가 cleared 로 안 나옴")
        h, why, exp = _fleet_hard_hold("hard", now=t1 + 20, thr=1.0)
        chk(abs(h - 20) < 1e-6 and why == "held",
            "다른 임계 호출이 기본 임계 래치를 지웠다(상한이 영영 안 찬다): %r/%r" % (h, why))
        h, why, exp = _fleet_hard_hold("hard", now=t1 + FLEET_CPU_HARD_MAX_HOLD_SECS + 1, thr=1.0)
        chk(exp and why == "expired", "상한 초과가 만료로 안 잡힘: %r" % (why,))
        h2, why2, exp2 = _fleet_hard_hold("hard", now=t1 + FLEET_CPU_HARD_MAX_HOLD_SECS - 1,
                                          thr=1.0)
        chk(exp2 and why2 == "expired",
            "시계를 되돌리자 만료가 풀려 차단이 되살아났다(만료가 상태가 아니다): %r/%r" % (h2, why2))
        _fleet_hard_hold("below", now=t1 + 2000, thr=1.0)
        _fleet_hold_write(_fleet_hold_path(1.0), {"since": t1, "last": t1, "expired": False})
        h3, why3, exp3 = _fleet_hard_hold("hard", now=t1 + 7200, thr=1.0)
        chk(exp3 and why3 == "expired_stale",
            "관측 공백이 있는 만료가 '연속 포화' 로 표기됐다(수치 오독 유발): %r/%r" % (h3, why3))
        # 읽기 불능 = 유계 증명 불능 → 만료(무기한 차단을 열지 않는다).
        # ★대역 없이 재현한다: 래치 자리에 **디렉터리**를 두면 `open()` 이 OSError 를 낸다
        #   (IsADirectory/Permission — 어느 쪽이든 FileNotFoundError 가 아니다). 모듈 전역·builtins
        #   을 건드리지 않는 방식이라 import 가드(정적 증명 가능 형태)와도 어긋나지 않는다.
        _dirlatch = _fleet_hold_path(1.0)
        try:
            os.remove(_dirlatch)
        except OSError:
            pass
        os.makedirs(_dirlatch, exist_ok=True)
        h4, why4, exp4 = _fleet_hard_hold("hard", now=t1 + 7300, thr=1.0)
        _sh.rmtree(_dirlatch, ignore_errors=True)
        chk((h4, why4, exp4) == (None, "unbounded_io", True),
            "래치 읽기 불능이 '지금 무장' 으로 접혀 상한이 매번 초기화된다: %r/%r/%r"
            % (h4, why4, exp4))
        # ★codex 위임 검체 D1 — 저장된 만료는 **큰 시계 역행**에도 취소되지 않는다
        _fleet_hard_hold("below", now=t1 + 8000, thr=1.0)
        chk(_fleet_hard_hold("hard", now=1000.0, thr=1.0)[1] == "armed", "D1 준비 무장 실패")
        chk(_fleet_hard_hold("hard", now=1900.0, thr=1.0)[2] is True, "D1 준비 만료 실패")
        _h, _w, _e = _fleet_hard_hold("hard", now=997.0, thr=1.0)
        chk(_e is True and _w == "expired",
            "큰 시계 역행이 만료를 취소하고 차단을 되살렸다(만료 판정이 역행 검사 뒤에 있다): %r/%r"
            % (_w, _e))
        # ★codex 위임 검체 D2 — 늦은 쓰기가 남의 만료를 되돌리지 않는다(단조 병합)
        _fleet_hold_write(_fleet_hold_path(1.0), {"since": 1000.0, "last": 1900.0,
                                                  "expired": True})
        _fleet_hold_write(_fleet_hold_path(1.0), {"since": 1000.0, "last": 1800.0,
                                                  "expired": False}, merge=True)
        _rec, _ = _fleet_hold_read(_fleet_hold_path(1.0))
        chk(_rec and _rec["expired"] is True and _rec["last"] == 1900.0,
            "병합 쓰기가 이미 공개된 만료를 되돌렸다(동시 호출에서 차단이 되살아난다): %r" % (_rec,))
        # ★codex 위임 검체 D9·D10 — 파손 래치가 예외로 게이트를 죽이지 않는다
        _cp = _fleet_hold_path(1.0)
        with open(_cp, "wb") as _f:
            _f.write(b"\xff\xfe not utf-8")
        chk(_fleet_hold_read(_cp) == (None, "corrupt"),
            "디코딩 불가 래치가 예외로 새어 exit 70 이 된다: %r" % (_fleet_hold_read(_cp),))
        with open(_cp, "w", encoding="utf-8") as _f:
            _f.write(json.dumps({"since": int("9" * 400), "last": 1}))
        chk(_fleet_hold_read(_cp) == (None, "corrupt"),
            "유한 float 로 못 바꾸는 저장 시각이 OverflowError 로 새어 exit 70 이 된다: %r"
            % (_fleet_hold_read(_cp),))
        os.remove(_cp)
        # 레인 키 — 긴 이름에서도 해시가 살아 있고, 같은 디렉터리의 두 소켓이 갈린다
        os.environ["CYS_GATE_LANE_SOCKET"] = "/a/" + ("x" * 90) + "/cys.sock"
        _p1 = _fleet_hold_path(1.0)
        os.environ["CYS_GATE_LANE_SOCKET"] = "/b/" + ("x" * 90) + "/cys.sock"
        _p2 = _fleet_hold_path(1.0)
        chk(_p1 != _p2, "긴 레인 이름에서 충돌 방지 해시가 잘려 두 레인이 래치를 공유한다: %r" % _p1)
        os.environ["CYS_GATE_LANE_SOCKET"] = "/d/lane/a.sock"
        _p3 = _fleet_hold_path(1.0)
        os.environ["CYS_GATE_LANE_SOCKET"] = "/d/lane/b.sock"
        chk(_p3 != _fleet_hold_path(1.0),
            "같은 디렉터리의 두 소켓이 같은 래치를 쓴다(레인 해시가 dirname 기준)")
        # 임계 키 정규화 — 1 · 1.0 · 1e0 은 같은 래치, ±0 도 하나
        chk(len({_fleet_hold_path(1), _fleet_hold_path(1.0), _fleet_hold_path(float("1e0"))}) == 1
            and _fleet_hold_path(-0.0) == _fleet_hold_path(0.0),
            "임계 키가 문자열 표기에 따라 갈린다(같은 임계가 두 래치를 쓴다)")
        # 레인 전용 변수가 CYS_SOCKET 보다 우선한다(부수 호출 오염 없이 레인만 바꾼다)
        os.environ["CYS_SOCKET"] = "/main/cys.sock"
        os.environ["CYS_GATE_LANE_SOCKET"] = "/dept/cys.sock"
        chk(_boot_epoch_path() == os.path.join("/dept", BOOT_EPOCH_BASENAME),
            "레인 전용 변수가 boot-epoch 레인을 못 바꾼다: %r" % _boot_epoch_path())
        os.environ.pop("CYS_GATE_LANE_SOCKET")
        chk(_boot_epoch_path() == os.path.join("/main", BOOT_EPOCH_BASENAME),
            "레인 전용 변수 부재 시 CYS_SOCKET 폴백이 깨졌다: %r" % _boot_epoch_path())
    finally:
        if _st2 is None:
            os.environ.pop("CYS_STATE_DIR", None)
        else:
            os.environ["CYS_STATE_DIR"] = _st2
        for _k, _v in (("CYS_SOCKET", _sk2), ("CYS_GATE_LANE_SOCKET", _ln2)):
            if _v is None:
                os.environ.pop(_k, None)
            else:
                os.environ[_k] = _v
        _sh.rmtree(_hd2, ignore_errors=True)
    # (r7) 분율·백분율 인자는 **음수도** 거부한다(R1-1 B4 문면 이행 · 반대 방향의 조용한 사고 차단)
    for bad in (["--load-soft-ratio", "-1"], ["--context-hard", "-1"],
                ["--context-soft", "-1"], ["--rate-soft", "-5"], ["--load-override", "-2"],
                ["--load-hard-ratio", "-1"], ["--context", "-1"]):
        with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()):
            rc = main(q + det_boot + ["--fleet-cpu-override", "0.0"] + bad)
        chk(rc == EXIT_USAGE, "%r 가 EX_USAGE(64) 로 거부되지 않았다: rc=%r" % (bad, rc))

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
          " + WP-7 codex 위임 반례 4종(합계 오버플로·진단 예외 격리·override 밀폐 2)"
          " + WP-7 함대CPU/부트유예 23종(임계 3점·유예 4점+비CPU축 음성대조·load soft전용·"
          "07:07 재생·ps부재 unavailable 4핀·ps실패 soft·순수 합산 6종·boot-epoch 3종+기본경로 1)"
          " + ★R1 리뷰 반영 33종(argv0 소유권: B2 오탐 14 반례 + 진짜 형상 15 음성대조(실측 5 포함) ·"
          " PID 자기제외 2 · 보류 상한 4점+타축 불변+warnings 불변+래치 순수 10 ·"
          " nan/inf EX_USAGE 8+유한 정상 1 · MSYS/unsupported 3 · 무동작 고지 3 ·"
          " 진단 비밀값 0/allow 무수집 2)"
          " + ★R2 리뷰 반영 47종(preflight 상수 정본화 6 + 판정기 의도된 차이표 6 ·"
          " MSYSTEM 단독 부정 2 · ps 디코딩 실패 1 · 소유권 오탐/정례 12 ·"
          " 래치(임계 격리 3·만료 영속 2·읽기불능 1·관측공백 표기 1·레인 해시 3·임계 키 1·레인 변수 2) ·"
          " 음수 분율 EX_USAGE 7 · codex 위임 검체 D1/D2/D3~D7/D9/D10 16)"
          " + A3 부서 로스터 8종(좌석 합산 22·floor 유지·응답 실패 soft·실데이터 9좌석 soft/hard 판별+"
          "음성 대조·--nodes-hard 우선·잘못된 주입 64·override 단락·라이브 경로 대역)")
    return 0


if __name__ == "__main__":
    if "--self-test" in sys.argv[1:]:
        sys.exit(self_test())
    sys.exit(main())
