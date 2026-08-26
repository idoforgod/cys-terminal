#!/usr/bin/env python3
"""javis_bootstrap.py — master 부트 시퀀스의 결정론 격상 (BOOTSTRAP_HARDENING WP-1).

"너는 마스터다" 이후의 기계적 절차 전부를 단일 exit-code 체인으로 수행한다.
LLM(master)의 역할은 이 스크립트 실행·출력 인용·이후 지휘뿐이다 — 산문 단계 수행 금지.

단계 체인 (실패 시 즉시 중단·단계명+원인을 stderr와 boot-last.json에 기록):
  ① preflight --fix (**비치명** — FAIL은 경고로 강등하고 계속. 팀 부팅의 진짜 게이트는 ⑤)
  ② cys ping (**유계 재시도** — 벽시계 총예산 창 안에서 간격 재시도. 데몬 콜드스타트·
  Defender 첫 스캔 내성 · 창 소진 시 종전대로 EXIT_PING + '몇 회·총 몇 초' 진단. 값의
  진실원천은 javis_budget leaf CYS_PING_RETRY_TOTAL_S·CYS_PING_RETRY_INTERVAL_S — W-A3)
                                 ③ cys claim-role master
  ④ cys boot (결손>0에서만 — 결손 0=구성 충족이면 호출 생략·스폰 없음)
  ⑤ orchestra check (bounded retry **24회×5s ≈ 120s 상한** — 노드 스폰은 비동기·check는 무대기
  스냅샷이므로 레이스 봉쇄. 값의 진실원천은 CHECK_RETRIES·CHECK_INTERVAL_S 상수이고 env
  CYS_BOOT_CHECK_RETRIES·CYS_BOOT_CHECK_INTERVAL_S로만 덮인다 — 테스트 하네스 전용.
  ★exit 2 는 `cys ping` 1회 재확인으로 '데몬 소실'(즉시 이탈)과 '일시 실패/팩 결손 가능성'
  (창 내 계속·별도 상한)을 실측 분리한다 — W-A3)
                                                          ⑥ 완료 마커 write
  ⑦ cys-dept promote-if-pending --request-only (비대기 — 부트와 승격 동의의 분리.
  ★T10: 부서 레인도 **신호만** 발사한다 — base 팩 cys-dept 를 CYS_NO_AUTOSTART=1 +
  CYS_SOCKET·CYS_PACK_DIR·CYS_ACCOUNT_DIR 스크럽 env 로 호출. 집행은 base 스케줄 틱)
  ⑧ 기계 요약 JSON 출력 (master는 이것을 인용해 보고한다)

완료 마커 ~/.cys/.master-bootstrapped 는 base 데몬 전용 단일-writer 마커다:
  - writer = 이 스크립트의 성공 경로(⑤ exit 0 후 ⑥) 유일. 삭제 주체 없음(버전 필드로 stale 판정).
  - ★소켓 격리: CYS_SOCKET이 base가 아니면(부서 pane 부트) write하지 않는다 — 부서장 부트가
    base 마커를 오염시키면 CEO 승격 게이트(cys-dept)가 오개방된다.

exit(`run` 체인 — 코드 상수 EXIT_* 와 대조 유지 · 진실원천은 상수):
      0=부트 완료(또는 부서장 단독 각성=CEO 티켓 부재) / 3=ping / 4=boot
      6=check 최종 실패(CHECK_RETRIES 소진 **또는** 판정 불가 확정 — 데몬 소실(ping 재확인
         실패)·인터프리터 소실·exit 2 반복+데몬 생존(팩 결손 가능성) · 값은 하나, 진단이 갈린다)
      7=claim 정당거부(이 surface는 master 아님 — 지휘 중단·인계)
      8=레인↔팩 정합 실패 또는 불량 레인(빈 부서명) — 교차 오염 차단·팀 기동 전 중단
      9=자원 hard_block(결손 기준 자원 사전 게이트 — 팀 기동 전 착수 거부·CEO escalation)
      10=세션 컨텍스트 오류(claim 왕복이 정당거부가 **아닌** 사유로 실패 — CYS_SURFACE_ID 부재·
         **발신 pane 미식별**(세션 분리·재부모화로 조상 체인 단절 = CLI rc 6)·데몬 미응답·
         바이너리 부재 등. 7과 분리해야 '남의 master' 오보를 안 만든다 — A20).
         ★여기에는 '부서 자동 생성 전제 미확인'(③-d dept-guard)도 포함된다 — 폴백의 전제인
         살아있는 master 보유자가 실측되지 않으면 부서를 만들지 않고 이 코드로 종결한다.
      11=skipped_inflight(단일 실행 락 패자 — 이미 다른 런이 부트 중. **실패 아님**)
  다른 서브커맨드: 5=assert-ready 게이트 실패(하위 게이트 전용) / 2=`issue-ticket` 사용오류
      (--dept 형식 위반 또는 base 레인 아님) / 64=EX_USAGE(미지 서브커맨드 — A14).
  ★2는 preflight가 아니다 — ①preflight는 비치명(FAIL이어도 경고 강등·계속)이라 전용 exit가
    없다. 구 헤더의 '2=preflight'는 낡은 계약이었다(G31).
안전밸브: CYS_BOOT_GATE=warn(assert-ready 실패를 경고로 강등)|off(게이트 무력).
        CYS_BOOT_LANE_LEGACY=1(U-24 이관 롤백 — 레인 경로 규약을 `javis_lane` 대신 이 파일의
        레거시 인라인 정의로 되돌린다. 기본값은 정본 소비이며 두 경로의 동치는 검체 H-LANE-1 이
        전수 대조한다. **판정 축이 아니므로 `CYS_BOOT_GATES` 마스터에 접지 않는다** — 게이트를
        끄는 사람이 의도하지 않은 경로 변경을 함께 겪으면 안 된다).

★종료 채널 계약(T-0147-7 W1b · A7 · 재감사 §3 CS-2① — 비평2 C-2 반영):
  · completed / solo_awakening → **stdout 최종 JSON**(배포된 산문 계약 "완료 선언은 이 스크립트의
    최종 JSON을 인용할 때만" — session-start.sh — 을 그대로 보존한다. 이 채널은 성공 전용이다).
  · skipped_inflight / failed → **stderr 1줄 verdict JSON + 구분 exit**. stdout(계약 채널)은
    건드리지 않는다 — 디렉티브 미개정 기계의 master가 skip JSON을 '완료'로 인용하는 회귀 차단.
  · skip은 **즉시 반환**한다(수렴 대기 금지 — 금지 방향 ⑨: 정상 시나리오가 이미 동시 2호출이라
    대기는 LLM Bash 호출을 냉부팅 예산만큼 블록해 도구 타임아웃·재시도 홍수를 만든다).
  · skip 기록은 boot-last.json 본체를 **덮지 않는다**(단일-writer 불변식 보존) — 레인별
    `boot-skip-<lane>.json` 별도 파일에 남긴다.

★런 정체성(A19 · CS-7①): boot-last.json 은 run_id(started+pid)·pid·surface·role 을 귀속하고,
  cmd_run 전체가 try/finally로 `ended`·`exit`(예외 시 `exc`)를 **항상** 기록한다. 시작 시점에
  `result:{"ok":null,"state":"running"}` 을 선기록하므로 '진행 중 / 중단 / 크래시 / 완주'가
  기계 구분된다(종전엔 진행 중과 크래시가 똑같이 'result 없음'이었다).
★boot-last 오염 차단(CS-2⑩ · 비평2 C-3): 정당거부(exit 7)·세션 컨텍스트 오류(exit 10)는 공유
  boot-last 에 `ok:false` 를 쓰지 않는다(`ok:null` + state=declined|session_error). 같은 레인
  두 번째 pane의 정당한 거부가 건강한 master의 ok:true 를 덮어 §0 소비 술어를 churn 시키는 것을
  막는다. §0 이 읽어야 하는 신호는 '**자기 surface**의 최신 완주 런'이다 — 그래서 귀속이 필수다.
★세션 오류 재시도 래치(P0-3): boot-last **최상위** `retry` 맵({sid:{count,at}})은 per-surface
  session_error 재시도 원장이다 — 새 런이 _Log 선기록 **이전**에 스냅샷해 carry-forward(타
  surface 완주 런이 본체를 덮어도 불소실), session_error 기록 시 자기 sid count+1, 자기 sid
  정상 완주(ok:true) 시 항목 제거, 24h 경과 회수. `result.retry_eligible=(기록 후 count<=1)`
  이 §0-A session_error 행(자기 surface 1회 재실행)의 **유일한** 재실행 근거다(오너 결정 ⑬Y:
  최초 실패=count 1=true · 같은 surface 연속 2회째=false). 상수부 래치 계약 주석이 정본이다.

부서 교리 게이트 (증분2 — D1 옵션 1'):
  ⓐ CEO 티켓 권한 게이트(P7): 부서 레인(CYS_SOCKET=부서 소켓)의 팀 기동은 CEO 발급 티켓 필수.
     티켓 부재/만료 → 실패가 아니라 '부서장 단독 각성'으로 강등(팀 기동만 생략·역할 등록/프리플라이트는
     정상·exit 0). 발급은 base 레인에서 `issue-ticket --dept <name>` 로만.
     ★2026-08-22 결함 #2 봉합: 티켓 부재를 감지하면 **스크립트가 결정론으로** base CEO 에
       `cys send --queued --to master` 로 발급을 요청하고(`env -u CYS_SOCKET` 동형 · 요청 마커
       + DEPT_TICKET_REQUEST_TTL_S 로 스팸 억제), 짧은 유계 대기(DEPT_TICKET_WAIT_BUDGET_S ·
       DEPT_TICKET_WAIT_INTERVAL_S 간격) 뒤 도착하면 그대로 팀 기동으로 이어간다. 종전엔
       안내문만 출력해 **오너가 추가 명령을 쳐야** 요청이 나갔다(실측 06:20→06:26→06:29→06:30).
       요청·대기 실패는 전부 fail-open(단독 각성 exit 0) — base 레인 영향 0.
  ⓑ 결손 기준 자원 게이트: 팀 기동 직전 결손을 cys list 라이브 노드의 **로스터 판정**으로 산출 —
     의무 역할 목록은 `javis_orchestra.effective_required_roles()`(=⑤check가 검증하는 그 목록,
     감지 폴백 적용)을 그대로 소비하고 좌석 이름은 정확일치(worker만 worker-N 접두 수용 — check와
     동일 규약)로 대조한다. **역할 이름공간을 ⑤check와 하나로 묶는 것이 이 판정의 전부다**(W0 P0
     지혈 · 재감사 G26 + A1 '결손 0 오판' 절반): 구 가족 접두 계수는 reviewer-grok(선택)·
     reviewer-claude-*(대체)·cso-N(변형) 좌석을 의무 슬롯 충족으로 계상해 결손 0 → ④ 생략 →
     ⑤check 실패 → exit 6 → 재선언 동일의 라이브락을 먹였다. orchestra 소비 불가(부서 팩 결손·
     팩 스큐)면 구 가족 접두 계수로 graceful 폴백(사유를 판정 사유에 병기). ★생존 **술어**는
     여전히 cys list의 role= + !exited이며 check 술어(agent_alive/ack)보다 관대하다 — 술어 격상은
     W2 소속(여기서 격상하면 건강한 quiet 노드를 결손>0으로 오판하는 역방향 회귀).
     결손 0(재선언)이면 게이트와 ④ cys boot 호출 자체를
     생략(스폰 없음·오탐 hard-block 방지 — "결손 0=스폰 없음"의 결정론화). 결손>0이면 자원 사전
     게이트 발동(hard=exit 9, 단 nodes 과계수 결함은 cys list 라이브 교차확인으로 무효화·1회 경고 후
     진행 / soft=매번 경고 후 진행 — 결손 0이면 게이트 자체를 생략하므로 soft 경고는 실팀기동
     시에만 발생, 소음 아니라 신호). 이 게이트는 base 포함 전 레인에 적용된다.

신뢰 모델: 티켓·마커·구성 게이트는 LLM 드리프트 차단용 결정론 가드이지 보안 경계가 아니다
(동일 $HOME 신뢰 도메인 — 파일 권한으로 악의 행위자를 막는 설계가 아님).
"""
import json
import os
import re
import subprocess
import sys
import tempfile
import time

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
#   ._pth 는 표준 경로 계산을 우회해 **스크립트 폴더를 sys.path 에 넣지 않는다**
#   (2026-07-29 Windows 0.14.4 실측: `ModuleNotFoundError: No module named 'javis_scrub'`).
#   unix/mac 은 스크립트 폴더가 이미 sys.path[0] 이라 이 블록은 무동작(멱등).
#   ★append 인 이유: **발견이 목적이지 기존 항목의 precedence 를 강등하지 않는다**(bin/ 을 stdlib
#   앞에 놓지 않아 미래의 이름충돌 shadowing 을 원천 차단). 선례: javis_orchestra.py:72-74.
#   계약(tests/test_import_guard.py): 가드를 건 뒤 형제 import 전까지 sys.path 를 건드리지 않는다.
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

# ★공용 크로스플랫폼 락(T-0147-7 W1a · A8py): 싱글플라이트를 javis_lock 단일 소유로 옮긴다.
#   종전 인라인 구현은 fcntl 단독이라 Windows에서 직렬화가 전무했고(항상 '획득'으로 접힘),
#   락 파일에 보유자 신원이 없어 스테일 회수도 불가능했다(R1). import 실패는 부트를 죽이지
#   않는다 — None 이면 아래 폴백이 구 동작(직렬화 없이 진행)으로 강등한다.
try:
    import javis_lock as _lock
except Exception:  # 팩 스큐·부서 팩 결손 — 지혈이 새 크래시 지점이 되면 안 된다
    _lock = None

# ★U-24 이관 1단: **레인 경로 규약의 소유자를 `javis_lane` 로 옮긴다.** 이 파일은 재수출만 한다.
#   왜 먼저인가: ③claim·싱글플라이트 집행이 데몬 감독자로 이관되며 이 파일 본문이 얇아지는데,
#   그 리팩터에 **경로 규약이 딸려 흔들리면** 소비자 6곳(javis_mission·훅 lane-path·디렉티브
#   2벌·preflight CONTENT_PINS·delivery.rs 파리티)이 조용히 갈린다. 축을 먼저 분리한다.
#
# ★롤백 스위치(env 1지점 · 기본값=신 경로): `CYS_BOOT_LANE_LEGACY=1` 이면 이 파일의 **레거시
#   인라인 정의**로 즉시 되돌아간다(이관은 1릴리스 병존). 두 경로의 동치는 검체 `H-LANE-1` 이
#   매트릭스로 대조하므로 스위치를 넘겨도 경로가 갈리지 않는다.
#   ★왜 `CYS_BOOT_GATES` 마스터에 접지 않는가: 저 마스터는 **판정 축**(관문 보류·주입 가드·
#     신뢰 등급 — `src/lib.rs gate_axes_from`)의 합류점이고, 이것은 판정이 아니라 **같은 값을
#     내는 구현의 출처**다. 사고 순간에 게이트를 끄면 레인 경로까지 함께 바뀌는 결합은
#     '노브 조합 불가' 원칙의 반대편 함정이다 — 끄는 사람이 의도하지 않은 축이 따라 움직인다.
# ★import 실패는 부트를 죽이지 않는다 — 배포된 구 팩·부서 팩에 이 모듈이 아직 없을 수 있다
#   (`build.rs` 는 **git 추적 파일만** 임베드한다 · 검체 `H-PACK-TRACK-1` 이 그 조건을 감시).
_LANE_SOURCE = "javis_lane"
try:
    if os.environ.get("CYS_BOOT_LANE_LEGACY") == "1":
        raise ImportError("CYS_BOOT_LANE_LEGACY=1 — 이관 롤백(레거시 인라인 경로 강제)")
    import javis_lane as _lane_mod
except Exception as _lane_e:      # 팩 스큐·부서 팩 결손·명시 롤백
    _lane_mod = None
    _LANE_SOURCE = "legacy-inline(%s: %s)" % (type(_lane_e).__name__, _lane_e)

# ★R3(D-IMPL-3): Windows 파이프 환경(cp949/cp1252)에서 한글 출력 UnicodeEncodeError 크래시 방어 —
# PYTHONUTF8 export는 cys-dept 경로에만 있어 이 스크립트의 직접 실행을 보호하지 못한다.
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

HOME = os.path.expanduser("~")
CYS_DIR = os.path.join(HOME, ".cys")
# ★A11(W3): 팩 경로 env 키 목록·순서의 단일 상수 — src/pack.rs `PACK_DIR_ENV_KEYS`·javis_preflight·
#   javis_report·javis_orchestra·javis_todo_stamp 와 **같은 목록·같은 순서**(기계 대조:
#   bin/tests/test_todo_shared_constants.py). 종전 이 파일은 `CYS_PACK_DIR` **1키**만 봤다 —
#   레거시 env(JAVIS_PACK_DIR·AITERM_*)만 설정된 기계에서 부트가 홈 기본 팩을, orchestra·preflight 는
#   레거시 팩을 봐서 **부트가 검사·기동하는 팩이 갈렸다**(A11 실분열 4/3/2/1 의 '1').
PACK_DIR_ENV_KEYS = ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR")


def _pack_from_env():
    for _k in PACK_DIR_ENV_KEYS:
        _v = os.environ.get(_k, "")
        if _v:
            return _v
    return os.path.join(CYS_DIR, "pack")


PACK = _pack_from_env()
# ★base 마커(CEO 승격 게이트의 SOT) — 경로 불변. writer 는 base 레인 성공 경로 유일이며,
#   cys-dept 의 `ceo_promote` 가 이 파일의 **존재**로 승격 게이트를 연다. 레인별 마커를 여기에
#   쓰면 부서장 부트가 CEO 게이트를 오개방한다(P3-A-DEPT-LANE 금지 방향 ① — 절대 금지).
MARKER = os.path.join(CYS_DIR, ".master-bootstrapped")


def state_dir():
    """상태 파일 루트 — `CYS_STATE_DIR` 우선, 없으면 역사적 기본값 `~/.cys/state`.

    ★경로 계약 통일(2026-08-01 T1 봉합): 종전 이 모듈만 env 를 **무시**하고 HOME 만 봤다.
      같은 계약을 이미 env-우선으로 쓰는 소비자들: `hooks/_lib.sh:235`(CYS_STATE_DIR 기본값
      주입·export) · `javis_formation.py:92` · `javis_state_ledger.py:131` ·
      `javis_memory_inject.py:65` · `hooks/fullauto/50-state-ledger.sh:125`.
      한쪽만 env 를 안 보면 **격리해서 돌린 것이 실 HOME 을 읽는다** — 실제로
      `javis_mission.py --self-test ⑧`(CYS_STATE_DIR=tmp 로 격리)이 사용자의 진짜 임무 대장을
      읽어, **임무를 정상 수신한 세션에서 preflight C77 이 FAIL** 하는 밀폐 붕괴가 났다.
    ★프로덕션 무변경: 훅 경로는 `_lib.sh` 가 CYS_STATE_DIR 을 정확히 `$HOME/.cys/state` 로
      채워 export 하므로 값이 동일하다. 모델의 §0 폴백(직접 실행)은 env 미설정 → 같은 기본값.
    ★불변식 유지: `MARKER`(CEO 승격 게이트 SOT)와 `CYS_DIR` 은 여기 걸리지 않는다 —
      마커 경로 불변은 금지 방향 ①이다.
    """
    return os.environ.get("CYS_STATE_DIR") or os.path.join(CYS_DIR, "state")


STATE_DIR = state_dir()
# ★base 레인 boot-last(§0 산문·GUI·테스트가 읽는 역사적 경로) — 비-base 레인은 `lane_state_path`
#   가 `boot-last-<lane>.json` 으로 분리한다(G15).
BOOT_LAST = os.path.join(STATE_DIR, "boot-last.json")
# ⑤ bounded retry — 무한 대기 금지(자원 거버넌스). env 오버라이드는 테스트 하네스 전용.
# ★예산 확대(2026-07-15 적대검증 adv#4): 냉시작 claude는 모델 로드+MCP init로 30초 내
# agent_alive/set-status ack가 안 나 check가 조기 실패(팀은 아직 뜨는 중)했다. 노드 기동은 비동기라
# 넉넉히 기다린다 — 24×5s ≈ 120초 상한(무한 아님·자원 거버넌스 유지).

# ★시간 예산 단일 소스(W2 B9·B17·P3-A-120S) — 상수 곱 산술과 하드코딩 timeout 을 폐기한다.
#   javis_budget 가 leaf(냉시작 실측 하한·감액 금지) 와 파생(외부 상한=하위 최악치 합+마진)의 SOT다.
#   ★소비 불가(부서 팩 결손·팩 스큐) 시엔 **종전 하드코딩 값을 명시 폴백**으로 쓴다 — 예산 모듈
#   부재가 새 크래시 지점이 되면 안 되고, 폴백은 조용하지 않다(사유가 step detail 에 남는다).
try:
    import javis_budget as _budget_mod
except Exception:
    _budget_mod = None


def _budget_leaf(name, fallback):
    if _budget_mod is None:
        return fallback
    try:
        return _budget_mod.leaf(name)
    except Exception:
        return fallback


def _budget_derived(fn_name, fallback):
    if _budget_mod is None:
        return fallback
    try:
        return int(round(float(getattr(_budget_mod, fn_name)())))
    except Exception:
        return fallback


CHECK_RETRIES = max(1, int(os.environ.get("CYS_BOOT_CHECK_RETRIES",
                                          str(_budget_leaf("CHECK_RETRIES", 24)))))
CHECK_INTERVAL_S = float(os.environ.get("CYS_BOOT_CHECK_INTERVAL_S",
                                        str(_budget_leaf("CHECK_INTERVAL_S", 5))))

# ── ② ping 유계 재시도 예산(W-A3 — 소비자 · leaf 는 W-A4 가 javis_budget 에 선등재) ──
# ★왜 재시도인가: 단발 ping(15s 1회)은 데몬 콜드스타트(자동기동+소켓 바인드)·Windows Defender
#   첫 스캔(바이너리 첫 실행 지연) 창에서 첫 응답만 늦어도 **선언 전체를 EXIT_PING 으로 폐기**
#   했다 — 몇 초 뒤 살아날 데몬인데 체인이 통째로 접혔다.
# ★유계의 형태(치명 앵커 ③ 자가치유): 이 창 동안 싱글플라이트 락을 쥔다 — 무계 재시도('뜰 때까지')
#   는 이후 모든 재선언을 exit 11 로 접는 자가치유 봉쇄라 기각. **벽시계 총예산**(B17 카운트 회계
#   금지 — javis_budget 교리)으로 창을 닫고, 소진 시 종전대로 EXIT_PING 이다.
# ★값의 진실원천: javis_budget leaf(CYS_PING_RETRY_TOTAL_S=45·CYS_PING_RETRY_INTERVAL_S=3·
#   CYS_PING_TIMEOUT_S=15)가 SOT. fallback 인자는 팩 스큐(구 javis_budget — 키 부재) 대비이며
#   **leaf 하한과 같은 값**으로 박아 SOT/폴백 거동 차를 0 으로 한다(예산 모듈은 타 티켓 소유·
#   수정 금지 계약 — 키가 그쪽에 있으면 자동으로 그쪽이 이긴다).
# env 오버라이드(CYS_BOOT_PING_*)는 CHECK_* 와 동일 규약 — 테스트 하네스 전용(budget 의
# CYS_BUDGET_* 는 하한 clamp 라 축소 불가·하네스가 창을 줄일 유일 경로가 이것이다).
# ★U-24 개명: `PING_TIMEOUT_S` → `DAEMON_PROBE_TIMEOUT_S`. ②만 쓰던 이름이 아니다 —
#   ⑤check 의 **exit 2 재확인 경로**(`daemon_gone` / '팩 결손 가능성' 실측 분리)가 같은 값을
#   소비한다. ②의 재시도 루프가 감독자로 이관되어도 ⑤의 재확인은 이 파일에 남으므로, 이름이
#   '②ping 전용'으로 읽히면 이관 시 **함께 지워지는** 사고가 난다(그 순간 ⑤는 무상한 ping 을
#   돌리거나 임의 하드코딩으로 갈린다 = 치명 앵커 ① 폭주/③ 자가치유 봉쇄).
#   ∴ 이름을 소비 지점 전체를 덮는 '데몬 탐침 상한'으로 승격한다. **값·leaf 키는 무변경**
#   (`CYS_PING_TIMEOUT_S` 는 javis_budget 소유 · 이 티켓의 수정 대상이 아니다).
DAEMON_PROBE_TIMEOUT_S = float(_budget_leaf("CYS_PING_TIMEOUT_S", 15))
# 하위호환 별칭(구 이름 소비자 대비 · 값 동일). 신규 코드는 `DAEMON_PROBE_TIMEOUT_S` 를 쓴다.
PING_TIMEOUT_S = DAEMON_PROBE_TIMEOUT_S
PING_RETRY_TOTAL_S = max(0.0, float(os.environ.get(
    "CYS_BOOT_PING_RETRY_TOTAL_S", str(_budget_leaf("CYS_PING_RETRY_TOTAL_S", 45)))))
# 간격 하한 0.05: 벽시계 창 + 0 간격은 fail-fast 데몬 부재에서 서브프로세스 스폰 폭주가 된다
# (창이 시간으로만 닫히므로 횟수는 간격이 유일하게 유계화한다 — 앵커 ① 폭주 방지).
PING_RETRY_INTERVAL_S = max(0.05, float(os.environ.get(
    "CYS_BOOT_PING_RETRY_INTERVAL_S", str(_budget_leaf("CYS_PING_RETRY_INTERVAL_S", 3)))))

# ── ③ 선행 claim 결박 신선도의 시간 기준(P0-1 — CLM-2 라이브락 절단) ──
# ★런 시작 시각 1회 캡처: 결박 나이(_pre_age)의 기준점이다. 종전엔 **소비 시각**(time.time())
#   기준이라 ①preflight(상한 300s)·②ping(상한 ~45s)의 in-run 소요가 신선도 창(기본 300s)과
#   동일 자릿수로 경합했다 — 훅이 claim 직후·spawn 이전에 찍은 스탬프(role-bootstrap.sh:674)가
#   ③ 소비 시점엔 이미 만료 → 미결박 폴백 → 재부모화된 이 프로세스의 직접 claim → 신원
#   미해석(rc6) → session_error(CLM-2 라이브락). 런 시작 기준이면 in-run 소요와 무관하므로
#   preflight 예산이 늘어도 재발하지 않는다(창 상수와 preflight 상수의 결합 자체를 절단).
# ★의미 재정의(P0-1 고지): 이로써 `CYS_CLAIM_MAX_AGE_S`(기본 300)의 의미는
#   '소비 시각 기준 최대 나이' → '**런 시작 시점 기준** 최대 나이'로 바뀐다. 소비처는 ③의
#   결박 술어 1곳뿐이라 파급은 그 함수 안으로 닫힌다. 단 MAX<=0 은 종전 관용('0=항상 미결박'
#   =소비 차단 idiom) 그대로다 — 유계 음수 허용이 이 관용을 시계 후퇴 창에서 조용히 깨지
#   않도록 ③ 술어에 MAX>0 가드를 둔다.
# ★벽시계(time.time())인 이유: 비교 상대 CYS_CLAIM_AT 이 훅의 `date +%s`(epoch) 스탬프 —
#   교차 프로세스라 monotonic 비교가 성립하지 않는다. 같은 기계의 같은 벽시계이므로 스큐
#   표면은 스탬프↔기동 사이의 NTP 후퇴뿐이고, 그것은 아래 유계 음수 허용이 흡수한다.
# ★모듈 로드 시각 캡처가 안전한 이유(구조 실측 R3-P01-1): 모듈 최상위는 순수(sys.path
#   append·stdout reconfigure·env 읽기·budget import)하고, 부트 run 외 경로(issue-ticket·
#   lane-path·status·assert-ready·--self-test·javis_mission 의 import 경유)는 ③ 신선도 검사에
#   도달하지 않는다 — 이 캡처는 어느 서브커맨드에도 부작용 0. 훅 spawn 은 스탬프 직후이므로
#   _RUN_T0 ≈ CYS_CLAIM_AT + ms 단위(Windows Defender 콜드스타트 최악에도 수십 초)다.
_RUN_T0 = time.time()
# ★유계 음수 허용(시계 후퇴 흡수): NTP 후퇴가 훅 스탬프와 프로세스 기동 사이에 끼면
#   _RUN_T0 - CYS_CLAIM_AT < 0 이 된다. 이를 미결박으로 접으면 **정확히 rc6 을 재생산**하므로
#   -120s 까지는 결박을 인정한다(초과는 미결박 — 무한 음수 허용 아님). 신뢰 모델(헤더 §신뢰
#   모델: LLM 드리프트 차단 가드이지 보안 경계가 아니다)상 유계 음수는 위조 표면을 넓히지
#   않는다 — 위조는 어차피 sid 동일성 결박(4개 env 동시 export)을 넘어야 하고, 그 능력자는
#   pane 안에서 직접 claim 이 가능하다(위협 등급 불변).
_CLAIM_SKEW_TOL_S = 120.0

# ── ③′ 세션 오류 재시도 래치(P0-3 — '재시도 주체 0' 봉합의 기계 유계) ──
# ★boot-last **최상위** "retry" 맵({sid: {count, at}}): per-surface session_error(exit 10)
#   재시도 원장이다. §0-A 의 session_error 행(자기 surface 1회 재실행)은 이 도구 파생값
#   `result.retry_eligible` 만을 근거로 발동한다 — '1회 한정'을 LLM 재량·기억이 아니라
#   도구 출력으로 강제한다(결정론 환원 · R3-P03-1 '지워질 수 없는 카운터').
# ★carry-forward 가 핵심이다: 슬롯은 레인당 1개라, '직전 런 대조' 방식의 래치는 타 pane 의
#   완주 런(정당거부 exit 7 포함 — ok:null 이지만 본체는 덮는다)이 끼어들면 소진 증거를 잃고
#   재무장한다(R3-P03-1 음성 독해 — 기계 유계 붕괴). 그래서 cmd_run 이 _Log 선기록으로 슬롯을
#   덮기 **이전에** 기존 맵을 스냅샷해 새 레코드로 이월한다(_load_retry_carry) — 래치가 외래
#   쓰기에 살아남으면서 정본 1곳·단일 writer 불변식은 유지된다(R3-P03-2 ⓐ 채택 · ⓑ별도
#   원장 파일은 정본 이원화 기각·ⓒ prev 1단 보존은 개입 2회에 깨져 기각).
# ★유계 2종: surface 당 1항목 + TTL(24h) 경과 회수. boot-last 파손·판독 불가 시 맵 소실 →
#   재시도 최대 1회 추가(유계 1회 · 수용 — R3-P03-1 잔여 위험). carry-forward 는
#   read-modify-write 지만 레인 싱글플라이트 락 아래라 경합이 없다.
# ★오너 결정 ⑬Y(2026-08-25): 최초 부트(자기 이력 0)의 첫 session_error 는 count=1 →
#   retry_eligible=true(in-pane 1회 자동 재실행 허용 — S1/S3 본 사건 계급의 자가치유 복구).
#   같은 surface 연속 2회째(count=2)부터 false=소진(오너 보고 후 정지).
# ★리셋 조건: 자기 sid 의 정상 완주(ok:true)만 항목을 제거한다 — 직전 런 대조가 아니라
#   완주 관측이 리셋이므로 교차 실행이 래치를 지우지 못한다.
RETRY_LATCH_TTL_S = 24 * 3600.0
RETRY_LATCH_MAX = 1          # 기록 후 count <= MAX 일 때만 retry_eligible=true

# ── exit 코드 단일 소스(A7·A14·A20 — 헤더 exit 표의 진실원천) ──
# ★타입드 종료: '성공'·'정당거부'·'세션 컨텍스트 오류'·'정상 skip'·'사용오류'가 각자 코드를 갖는다.
#   구 계약은 정당거부와 인프라 실패를 7 하나로, 정상 skip과 완주를 0 하나로 뭉갰다(RC2).
EXIT_OK = 0
EXIT_PING = 3
EXIT_BOOT = 4
EXIT_ASSERT_READY = 5
EXIT_CHECK = 6
EXIT_CLAIM_DENIED = 7        # 정당거부 — 살아있는 master 가 이미 있다(이 surface 는 master 아님)
EXIT_LANE_PACK = 8
EXIT_RESOURCE_HARD = 9
EXIT_SESSION_CONTEXT = 10    # claim 왕복이 '정당거부가 아닌' 사유로 실패(A20)
EXIT_SKIPPED_INFLIGHT = 11   # 싱글플라이트 패자 — 실패 아님(A7)
EXIT_USAGE = 64              # EX_USAGE(sysexits.h) — 미지 서브커맨드·사용오류(A14)

# ★(W4) `cys boot` 의 **busy 전용 종료코드**(EX_TEMPFAIL) — Rust 정본 `cys.rs::EXIT_BOOT_BUSY`.
#   구계약에서 busy 는 0(성공)이었고, 그래서 ④가 **무스폰인데 CEO 티켓을 소각**했다(G11).
#   신계약: 0=Fatal 없음 / 1=Fatal / 75=busy(무스폰 skip). 이 값은 소비 분기의 근거이고,
#   `H-EXIT-2` 가 Rust 상수와 기계 대조한다. 구 바이너리는 75 를 내지 않으므로(0/1/2만)
#   이 분기는 신 바이너리에서만 발동한다 — 스큐 안전.
CYS_BOOT_EXIT_BUSY = 75

# ★(U-11) `cys launch-agent` 의 **관문 보류 전용 종료코드**(sysexits EX_CONFIG) — Rust 정본
#   `cys::EXIT_GATE_PENDING`. 의미: "pane 은 만들어졌고 에이전트 프로세스는 살아 있으나 첫기동
#   관문(테마 → 로그인방식 → OAuth → 폴더신뢰 → 면책 → 새기능안내)에 갇혀 입력을 받지 못한다.
#   좌석은 **닫지 않았다**."
#   0 도 1 도 아닌 값을 쓰는 이유: 0 이면 소비부가 '노드를 세웠다'로 읽어 지침·티켓을 태우고
#   (그 주입의 Return 이 실측상 면책 창의 `No, exit` 을 누른다 = 노드 사망), 1 이면 '깨졌다'로
#   읽어 **살아 있는 좌석을 회수·파괴**하려 든다. `H-EXIT-11` 이 3자 파리티를 기계 대조한다.
#   구 바이너리는 78 을 내지 않으므로(0/1 만) 이 분기는 신 바이너리에서만 발동한다 — 스큐 안전.
CYS_LAUNCH_EXIT_GATE_PENDING = 78

# ★(M3-짝 · 2026-08-24) `cys boot` 도 **같은 사실**(관문 보류)에 대해 같은 값을 낸다.
#   Rust 쪽 `boot_exit_code` 가 summary 의 `gate_pending` 버킷이 비지 않았을 때 이 값을 내고,
#   여기가 그 유일한 파이썬 소비 상수다. **별도 숫자를 만들지 않는 것이 이 두 줄의 전부다** —
#   같은 사실에 값이 둘이면 한쪽만 고쳐지는 날이 오고, 그날의 증상은 "관문 보류인데 부트가
#   성공으로 읽힌다"(=거짓 성공)다. 별칭이므로 드리프트가 구조적으로 불가능하다.
CYS_BOOT_EXIT_GATE_PENDING = CYS_LAUNCH_EXIT_GATE_PENDING

# ★`cys boot --json` 의 role 별 `outcome` 중 **Fatal 로 판정하는 값 집합**(단일 등재소).
#   여기 없는 outcome 은 Fatal 이 아니다 — 그래서 새 outcome 이 생기면 **이 한 줄**을 고치게
#   된다(종전엔 집합이 `_boot_fatal_verdict` 본문 안 튜플 리터럴이라, 새 outcome 이 어디에도
#   안 걸리는 것이 diff 에 드러나지 않았다 · 실제 사고 형태 = `gate_pending` 무처리).
#   · failed  = 기동 시도했고 실패했다      · missing = 그 에이전트가 이 기계에 없다
# ★★`gate_pending` 은 **일부러 여기 없다.** U-11 이 명시적으로 결박한 계약이다(집행자 = 검체
#   `H-EXIT-11` ⑥): 관문 보류는 pane 도 에이전트 프로세스도 **살아 있는** 상태라, Fatal 로
#   접으면 소비부가 '기동이 깨졌다'로 읽어 **살아 있는 좌석을 회수·파괴**하려 든다
#   (치명위험 ④ — 오살이 오탐보다 훨씬 비싸다). 그렇다고 Degrade 로 흘려보내면 종전처럼
#   "비0 이지만 Fatal 역할은 전원 확보" 라는 **거짓 문장**이 기록된다(관문에 갇힌 의무 역할이
#   있는데 전원 확보라고 적는 것).
#   ∴ 제3 분기다 — `_boot_gate_pending_verdict` 가 잡고, 처방은 회수·재기동이 아니라
#     '사람이 그 pane 에서 관문 1회 통과' 이며, 최종 게이트는 종전대로 ⑤check 다
#     (⑤ 의 결손 산출은 U-10 이 gate_pending 좌석을 이미 '못 쓰는 좌석'으로 센다 — 4자 파리티).
BOOT_FATAL_OUTCOMES = ("failed", "missing")

# 제3 상태의 outcome 값(정본 = Rust `cys::GATE_PENDING_KEY`).
BOOT_GATE_PENDING_OUTCOME = "gate_pending"

# 관문 보류 처방 문안(단일 출처) — launch-agent 소비부(U-11)와 **같은 사실을 같은 말로** 낸다.
# ★면책 창 경고를 반드시 동봉한다: 실측상 기본 포커스가 `No, exit` 이라 그대로 Return 하면
#   좌석이 죽는다(2026-07-29 실사고 형태 · e2e 오라클 H-FAKE-2 가 rc 1 로 박제한 그 축).
_GATE_PENDING_PRESCRIPTION = (
    "→ 그 pane 에서 첫기동 관문(테마 → 로그인방식 → OAuth → 폴더신뢰 → 면책 → 새기능안내)을 "
    "1회 통과시켜라. ★면책 창의 기본 포커스는 `No, exit` 이므로 그대로 Return 하면 노드가 "
    "종료된다(아래 방향키 1회 뒤 Return). 좌석과 프로세스는 살아 있으므로 **회수·재기동·kill 은 "
    "하지 마라** — 재부트가 스폰 없이 그 좌석을 채택한다."
)

# claim 출력이 **정당거부**임을 확정하는 마커(데몬 문구 — hooks/session-start.sh 의 self-demote
# 대조 지점과 동일 어휘. 종전 주석은 `session-start.sh:101` 을 가리켰으나 실제 대조는 그 아래
# `$CLAIM_OUT` grep 이다 — 낡은 라인 참조를 지운다).
# 이 마커가 없는 rc≠0 은 거부가 아니라 세션 컨텍스트 오류다(A20: 판정·escalation 층위 뭉개기 해소).
# ★2026-08-16 이후 데몬은 신원 실패를 claim_caller_unresolved·claim_not_owner 로 낸다 — 두 코드
#   모두 위 마커 부분문자열을 **포함하지 않으므로** 이 상수는 무변경으로 정확하다(구 데몬 호환).
_CLAIM_DENIED_MARKERS = ("claim_denied", "privileged role held")

# ── 증분2: 부서 교리 게이트 상태 ──
# CEO 티켓 저장소(base 레인이 발급·부서 레인이 소비) + 24h TTL + 1회성(소비 시 .used rename).
TICKET_DIR = os.path.join(STATE_DIR, "dept-boot-tickets")
TICKET_TTL_SECS = float(os.environ.get("CYS_DEPT_TICKET_TTL_SECS", str(24 * 3600)))

# ── 부서명 규약 — **단일 출처**(2026-08-22 적대검증 중대③ 비대칭 봉합) ──────────────
# ★결함: 생성기(`cys-dept:185 dept_name_ok` = `^[A-Za-z0-9][A-Za-z0-9_-]{0,39}$`)는 대문자·`_`
#   를 허용하는데 발급기(issue-ticket)는 `[a-z0-9][a-z0-9-]*` 만 받았다. 오너가 `Sales`·`dept_1`
#   로 부서를 만들면 **티켓을 영영 못 받아** 결함 #1(팀 미기동)이 그대로 남는다.
# ★master 결정: **발급기를 생성기에 맞춰 넓힌다**. 부서가 이미 그 이름으로 존재하는데 티켓을
#   거부해 봐야 아무도 이롭지 않다. 경로 안전성은 문자 집합이 `[A-Za-z0-9_-]` 뿐이라 유지된다
#   (경로 구분자·`.`·공백 불가 → 디렉터리 탈출 없음). 길이 상한 40자도 생성기와 동일하다.
# ★생성기가 이 규약의 정의처다 — 여기를 고칠 때는 `cys-dept::dept_name_ok` 와 함께 고친다
#   (self-test 가 두 소스의 정규식 문자열을 실제로 대조해 드리프트를 잡는다).
DEPT_NAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,39}")


def dept_name_ok(name):
    """부서명 규약 판정 — `cys-dept::dept_name_ok` 와 **같은 집합**(단일 출처·사본 금지)."""
    return bool(DEPT_NAME_RE.fullmatch(name or ""))

# ── 부서 레인 CEO 티켓 **자동 요청**(2026-08-22 현장 결함 #2) ────────────────────
# ★결함: ③″ 티켓 게이트가 티켓 부재를 감지하고도 **안내문만 출력하고 끝났다**. 그래서 부서장은
#   팀 없이 대기했고, 오너가 추가 명령을 쳐야만 CEO 에게 티켓을 요청했다 —
#   실측 2026-08-22: 06:20 단독각성 → 06:26 **오너 추가입력** → 06:29 요청 → 06:30 발급 → 팀 기동.
#   오너 절대규칙(아래 _dept_fallback 상단 주석)은 "선언자는 새 부서장이 되며 팀이 **기동돼야
#   한다**"이므로, 요청은 오너 타이핑이 아니라 **스크립트가 결정론으로** 발사한다.
# ★계약(fail-open 불변): 요청·대기의 어떤 실패도 부트를 실패시키지 않는다. 미도착이면 종전과
#   똑같이 단독 각성(exit 0) — 이 경로는 부서 레인 전용이라 base 레인 영향은 0 이다.
DEPT_TICKET_REQ_DIR = os.path.join(STATE_DIR, "dept-ticket-requests")
# 요청 억제 TTL(초) — 재부팅 반복 시 CEO 큐 스팸 차단. ★TTL 경과 후에는 **다시 요청한다**
#   (영구 침묵 금지: 한 번 실패한 부서가 영영 팀을 못 받는 상태가 더 나쁘다).
DEPT_TICKET_REQUEST_TTL_S = float(os.environ.get("CYS_DEPT_TICKET_REQUEST_TTL_S", "600"))
# 요청 후 티켓 도착을 기다리는 **유계** 예산(초)·폴링 간격(초). 부트 총시간에 얹히는 값이라
# 짧게 잡는다 — 못 받아도 단독 각성으로 진행하므로 길게 기다릴 이유가 없다.
DEPT_TICKET_WAIT_BUDGET_S = float(os.environ.get("CYS_DEPT_TICKET_WAIT_S", "90"))
DEPT_TICKET_WAIT_INTERVAL_S = float(os.environ.get("CYS_DEPT_TICKET_WAIT_INTERVAL_S", "3"))
# 요청 push 1회의 상한(초) — 데몬 부재 시 부트가 매달리지 않게.
DEPT_TICKET_PUSH_TIMEOUT_S = 15

def _atomic_write_json(path, obj):
    """CRLF 함정 회피(newline='\\n')·원자 교체 — Windows 재직렬화 원복 교훈.
    ★W1a: 구현을 javis_lock.atomic_write_json 으로 단일화한다(mkstemp 헬퍼 공용화·CS-5②).
      바이트 계약(indent=1·ensure_ascii=False·말미 개행)은 동일하다. import 실패 시에만
      아래 인라인 사본으로 강등한다(팩 스큐 내성)."""
    if _lock is not None:
        return _lock.atomic_write_json(path, obj, indent=1, ensure_ascii=False,
                                       trailing_newline=True)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=".tmp-boot-")
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as f:
            json.dump(obj, f, ensure_ascii=False, indent=1)
            f.write("\n")
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            try:
                os.unlink(tmp)
            except OSError:
                pass


def _read_json(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return None


def _tcc_probe_targets():
    """macOS TCC(파일·폴더 권한) 탐침 대상 — **이미 진입한 실자원**에서 파생한다.

    ★P3-A-TCC(재감사 · RC3 "계측기가 대상을 못 잰다"): 구 탐침은 `~/Desktop` **하드코딩**이었다.
      부트가 실제로 읽고 쓰는 자원은 (a) 이 세션의 작업 디렉터리(pane cwd — claude 가 파일을
      읽는 곳) 와 (b) 팩 디렉터리(디렉티브·상태·헬퍼) 인데, 탐침은 그 둘 중 아무것도 찌르지
      않았다. 그래서 ①Desktop 을 안 쓰는 기계에선 **거짓 경고**가 되고 ②Documents·프로젝트
      폴더만 막힌 실제 사고에서는 **침묵**했다(GUI perm-warning 은 Desktop+Documents 를 보므로
      부트 탐침이 GUI 보다도 좁았다). 탐침 대상을 실자원 파생으로 바꾼다 — 계측기는 자기가
      실제로 쓰는 자원을 재야 한다.
    ★HOME 은 탐침하지 않는다: 홈 루트는 TCC 대상이 아니고(하위 특수 폴더가 대상), 여기서
      필요한 사실은 '내가 지금 쓰는 폴더를 읽을 수 있나' 뿐이다.
    반환: [(경로, 라벨)] — darwin 아니면 빈 목록(무동작). 중복·부재 경로는 제거한다.
    """
    if sys.platform != "darwin":
        return []
    try:
        cwd = os.getcwd()
    except OSError:                      # cwd 가 삭제된 세션 — 탐침 대상에서 제외(크래시 금지)
        cwd = None
    out, seen = [], set()
    for path, label in ((cwd, "작업 디렉터리"), (PACK, "팩")):
        if not path:
            continue
        real = os.path.realpath(path)
        if real in seen or not os.path.isdir(real):
            continue
        seen.add(real)
        out.append((real, label))
    return out


def _progress(msg):
    """★R12: 단계 시작 신호(stderr 1줄) — 진행 중 무출력이면 최악 수 분의 침묵 창이 생겨
    관찰자(초보·master)가 '멈춤'으로 오인한다(실사고 증상②의 형태적 재생산 방지).
    기계 계약(exit code·⑧ JSON)과 별개의 인간 관찰자용 인터페이스."""
    sys.stderr.write("[bootstrap] %s\n" % msg)
    try:
        sys.stderr.flush()
    except Exception:
        pass


def _run(cmd, timeout=120, env=None):
    """서브프로세스 실행 — (exit, stdout+stderr 병합 텍스트). shell 미사용(경로 quoting 안전).
    env=None 이면 상속(종전 계약 그대로) — 명시 dict 는 ⑦ 부서 레인 신호처럼 스크럽이 계약인
    호출부 전용(additive · 기존 호출자 무영향)."""
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           encoding="utf-8", errors="replace", env=env)
        return r.returncode, (r.stdout or "") + (r.stderr or "")
    except FileNotFoundError:
        return 127, "명령 없음: %s" % cmd[0]
    except subprocess.TimeoutExpired:
        return 124, "timeout(%ss): %s" % (timeout, " ".join(cmd))


def _run_split(cmd, timeout=120):
    """서브프로세스 실행 — (exit, stdout, stderr)를 **분리** 반환. `_run`(병합)의 additive 형제.

    ★왜 분리판이 필요한가(A13): 기계 계약(JSON)이 stdout 이고 진단이 stderr 인 도구를 병합 텍스트로
      파싱하면, 진단 한 줄이 계약을 파괴한다 — '판정 실패'가 '판정 결과'로 위장되는 경로다.
      `_run` 은 종전 소비처(산문 진단 목적)가 많아 계약을 바꾸지 않고 그대로 둔다(무접촉)."""
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           encoding="utf-8", errors="replace")
        return r.returncode, (r.stdout or ""), (r.stderr or "")
    except FileNotFoundError:
        return 127, "", "명령 없음: %s" % cmd[0]
    except subprocess.TimeoutExpired:
        return 124, "", "timeout(%ss): %s" % (timeout, " ".join(cmd))


def _legacy_socket_is_base(sock):
    """★U-24 **레거시 폴백 전용**(정본은 `javis_lane.socket_is_base`) — 이 파일에 `javis_lane`
    이 없는 팩(구 배포·부서 팩 스큐)이나 `CYS_BOOT_LANE_LEGACY=1` 롤백에서만 소비된다.
    정본과의 동치는 검체 `H-LANE-1` 이 매트릭스로 대조한다(사본 드리프트 차단).

    순수 판정: 소켓 경로 문자열 → base 여부(§4.1 소켓 격리). CYS_SOCKET 미설정('')=base.
    ★보수성(아키텍트 성찰): base = (미설정) 또는 (basename이 cys/cys.sock **AND** cys-dept- 성분
    없음). 커스텀 소켓(/tmp/whatever.sock)은 구코드처럼 **비-base·비-dept**다 — base 마커 무접촉·
    issue-ticket 불허·티켓 게이트 비적용(구동작 보존). "cys-dept- 성분 없으면 전부 base"는 미지
    소켓에 base 특권(마커 write·티켓 발급)을 주던 과관용이었다.
    ★경로 기반 dept 판정(basename 아님): 부서 소켓 ~/.local/state/cys-dept-<name>/cys.sock 은
    basename이 본부와 동일한 'cys.sock'이라 basename 단독 판정이 부서를 base로 오판했다
    (마커 오염·ceo_promote 오개방) — cys-dept- 성분이 있으면 무조건 비-base.
    Windows named pipe(백슬래시.백슬래시 pipe 형식)는 성분 분해가 부적합하므로 기존 basename 동작을 보존한다."""
    sock = (sock or "").strip()
    if not sock:
        return True
    norm = sock.replace("\\", "/")
    if sock.startswith("\\\\") or norm.lower().startswith("//./pipe/"):  # win named pipe — 기존 동작 보존
        return os.path.basename(norm) in ("cys", "cys.sock")
    for part in norm.split("/"):
        if part.startswith("cys-dept-"):
            return False
    return os.path.basename(norm) in ("cys", "cys.sock")


# 재수출(정본=`javis_lane`) — 이름은 불변이고 소비자(자기 `--self-test`·검체·훅)는 무개정이다.
_socket_is_base = _lane_mod.socket_is_base if _lane_mod is not None else _legacy_socket_is_base


def _is_base_socket():
    """CYS_SOCKET env 래퍼(호출부 하위호환)."""
    return _socket_is_base(os.environ.get("CYS_SOCKET", ""))


def _legacy_sanitize_sock_key(sock):
    """★U-24 **레거시 폴백 전용**(정본은 `javis_lane.sanitize_sock_key`).
    소켓 전체 경로 → 파일명 안전 락 키(레인마다 유일). 부서 소켓은 basename(cys.sock)이 동일해
    basename 키를 쓰면 모든 레인이 같은 락 파일을 공유했다 — 전체 경로 새니타이즈로 레인 유일화.
    경로 구분자(os.sep·'/'·'\\')·':'를 '_'로 치환. 파일명 길이 상한(255) 여유 — 과길면 앞부분+경로
    해시로 유일성 보존(절단만 하면 서로 다른 긴 경로가 같은 키로 충돌)."""
    raw = (sock or "").strip() or "base"
    for ch in (os.sep, "/", "\\", ":"):
        raw = raw.replace(ch, "_")
    raw = raw.strip("_") or "base"
    if len(raw) > 160:
        import hashlib
        raw = raw[:120] + "-" + hashlib.sha1(raw.encode("utf-8")).hexdigest()[:16]
    return raw


_sanitize_sock_key = (_lane_mod.sanitize_sock_key if _lane_mod is not None
                      else _legacy_sanitize_sock_key)


def _socket_dept(sock=None):
    """순수 판정: 소켓 경로 → 부서명(cys-dept-<name> 성분) 또는 None(비-부서). 기본값은 CYS_SOCKET
    env. ※ dept None ≠ base: 커스텀 소켓(/tmp/whatever.sock)은 비-base·비-dept(dept None)다 —
    base 판정은 _socket_is_base가 별도로 한다. 빈 suffix(cys-dept-/)는 _socket_malformed_dept가
    불량 레인으로 걸러 cmd_run이 명시 실패한다."""
    sock = os.environ.get("CYS_SOCKET", "") if sock is None else sock
    sock = (sock or "").strip()
    if not sock:
        return None
    for part in sock.replace("\\", "/").split("/"):
        if part.startswith("cys-dept-"):
            return part[len("cys-dept-"):] or None
    return None


def _socket_malformed_dept(sock=None):
    """순수 판정: 빈 부서명 레인(경로 성분이 정확히 'cys-dept-' — suffix 없음) 감지(R1-LOW-2).
    이 소켓은 _socket_is_base=False인데 _socket_dept=None이라 어느 레인 계약에도 못 들어가는
    불량 레인이다 — cmd_run이 레인↔팩 가드 계열(exit 8)로 시끄럽게 명시 실패한다."""
    sock = os.environ.get("CYS_SOCKET", "") if sock is None else sock
    sock = (sock or "").strip()
    if not sock:
        return False
    return any(part == "cys-dept-" for part in sock.replace("\\", "/").split("/"))


def _pack_dept(pack=None):
    """순수 판정: 팩 경로 마지막 성분 pack-dept-<name> → name, 아니면 None(메인 팩). 기본값은 PACK."""
    pack = PACK if pack is None else pack
    base = os.path.basename((pack or "").replace("\\", "/").rstrip("/"))
    if base.startswith("pack-dept-"):
        return base[len("pack-dept-"):] or None
    return None


def _lane_pack_mismatch(sock=None, pack=None):
    """레인(소켓 부서)↔팩(부서 팩) 정합 판정. 정합이면 None, 불일치면 (sock_dept, pack_dept).
    교차 오염(UT-14): dept-X 레인이 메인/다른 부서 팩을 쓰거나 base 레인이 부서 팩을 쓰면 위험."""
    sd = _socket_dept(sock)
    pd = _pack_dept(pack)
    return None if sd == pd else (sd, pd)


def _notify_loud(title, body):
    """실패를 시끄럽게 알림 — feed push(승인 채널) 우선, 실패 시 cys send --queued --to master 폴백.
    둘 다 best-effort·짧은 timeout(데몬 부재 시 행 금지·graceful). 성공 채널명 또는 'none' 반환(흔적)."""
    for name, cmd in (
        ("feed", ["cys", "feed", "push", "--kind", "bootstrap-fail", "--title", title, "--body", body]),
        ("send", ["cys", "send", "--queued", "--to", "master", "[부트 중단] %s — %s" % (title, body)]),
    ):
        try:
            r = subprocess.run(cmd, capture_output=True, timeout=10)
            if r.returncode == 0:
                return name
        except Exception:
            continue
    return "none(데몬 부재 등 — 비제로 exit·boot-last.json이 최종 증거)"


def _pack_version():
    v = None
    for cand in (os.path.join(CYS_DIR, ".pack-version"), os.path.join(PACK, ".pack-version")):
        try:
            with open(cand, encoding="utf-8") as f:
                v = f.read().strip() or None
            if v:
                break
        except OSError:
            continue
    return v or "unknown"


def _binary_version():
    code, out = _run(["cys", "--version"], timeout=10)
    return out.strip().splitlines()[0] if code == 0 and out.strip() else "unknown"


def my_surface_id():
    """이 런이 속한 surface 참조(귀속 키). 구 env 이름도 수용(프리루드 A2 게이트와 동일 규약).

    ★surface 판독 규약의 **단일 소유자**(2026-08-02 R6 · 사본 금지). 훅 게이트
      (`hooks/_lib.sh::in_cys_pane`)와 형제 모듈(`javis_task._env_surface`)이 신·구 env 를
      **함께** 보는데 `javis_mission._surface()` 만 `CYS_SURFACE_ID` 하나만 봐서, 구 env 로만
      선 pane 에서는 훅 게이트는 통과하고 임무 게이트의 **surface 결박(층1 의 전제)** 은 빈
      문자열로 무너지는 비대칭이 있었다. 판독을 여기 한 곳으로 모아 그 갈림을 없앤다.
      (공개 이름 = 다른 팩 모듈이 이 규약을 재구현하지 말라는 신호다.)
    """
    return (os.environ.get("CYS_SURFACE_ID", "")
            or os.environ.get("AITERM_SURFACE_ID", "") or "")


def _my_surface_id():
    """구 내부 이름 — 호출부 보존용 얇은 별칭(규약 본체는 `my_surface_id`)."""
    return my_surface_id()


# ── 단계 정체성 레지스트리 (P3-A-STEP-NAME · W3) ─────────────────────────────
# ★결함 2건(재검증이 성립 확정한 절반):
#   ⓐ **동명이의 재사용** — `③′lane-pack` 이 '불량 레인(빈 부서명)'과 '레인↔팩 불일치' **두 개의
#      다른 단계**에 쓰였고, `④′resource-gate` 가 '게이트 실행'과 '게이트 생략'에 쓰였다. 진단에서
#      같은 이름이 다른 사건을 뜻하면 원인 추적이 갈린다.
#   ⓑ **서수 ↔ 실행순서 불일치** — 레인 가드는 ①preflight 보다 **먼저** 도는데 이름은 `③′` 였고,
#      티켓 소비는 ④boot **뒤**인데 이름은 `③″` 였다. 서수는 계약처럼 읽히므로(로그·산문·티켓)
#      순서를 거짓말하는 이름은 그 자체가 오정보다.
# ★해법: 라벨을 **선언 순서 = 실행 순서**인 단일 레지스트리로 승격하고(아래 튜플), 호출부는
#   문자열 리터럴 대신 `STEP.*` 상수만 쓴다. `_Log.step` 이 미등록 라벨과 순서 역행을 런타임에
#   기록하고(측정 실패를 침묵시키지 않는다), `--self-test` 가 ①유일성 ②리터럴 0(전 호출부가
#   레지스트리 경유) ③기록 순서 단조를 단언한다(H-LIFE-2).
_STEP_DEFS = (
    # (상수명, 기록 라벨) — 이 순서가 실행 순서 계약이다.
    ("LANE_MALFORMED", "⓪lane-malformed"),
    ("LANE_MALFORMED_NOTIFY", "⓪lane-malformed-notify"),
    ("LANE_PACK", "⓪lane-pack"),
    ("LANE_PACK_NOTIFY", "⓪lane-pack-notify"),
    ("PREFLIGHT", "①preflight"),
    ("PING", "②ping"),
    ("CLAIM_ROLE", "③claim-role"),
    ("CLAIM_ROLE_CONTEXT", "③claim-role-context"),
    # ★위계 폴백(2026-08 현장 결함 3호 · 오너 결정 D1ⓐ/D2/D3): base 레인에서 살아있는 master 가
    #   있어 ③이 정당거부될 때, 선언(=오너 타이핑·MO 게이트 통과)을 '부서 창설 의도'로 해석해
    #   부서 자동 생성 → 티켓 발급 → 부서장·팀 기동으로 이어주는 단계들. ③ 거부 직후에만 돈다.
    ("DEPT_FB", "③-d dept-fallback"),
    # ★전제 실측 가드(2026-08-16): 폴백 **미진입** 판정의 전용 단계. 종전엔 이 결과를
    #   CLAIM_ROLE_CONTEXT(order 7)로 적었는데, 그건 DEPT_FB(order 8) 뒤에 오는 역행이라
    #   boot-last 가 매번 order_violation 을 남겼다(계측기가 스스로 '깨졌다'고 기록).
    ("DEPT_FB_GUARD", "③-d dept-guard"),
    ("DEPT_FB_ALLOC", "③-d dept-alloc"),
    ("DEPT_FB_TICKET", "③-d dept-ticket"),
    ("DEPT_FB_MASTER", "③-d dept-master"),
    ("DEPT_FB_TEAM", "③-d dept-team"),
    ("DEPT_FB_CHECK", "③-d dept-check"),
    ("DEPT_FB_NOTIFY", "③-d dept-notify"),
    ("CEO_TICKET", "③″ceo-ticket"),
    # ★2026-08-22 결함 #2: 티켓 부재를 감지만 하고 끝내지 않는다 — base CEO 에 발급을 요청하고
    #   유계 대기한 뒤, 그래도 없으면 단독 각성으로 강등한다. 선언 순서 = 실행 순서 계약대로
    #   감지(CEO_TICKET) → 요청 → 대기 → 단독각성 고지 순으로 선언한다.
    ("CEO_TICKET_REQUEST", "③″ceo-ticket-request"),
    ("CEO_TICKET_WAIT", "③″ceo-ticket-wait"),
    ("CEO_TICKET_SOLO", "③″ceo-ticket-solo"),
    ("RESOURCE_GATE_ABSENT", "④′resource-gate-absent"),
    ("RESOURCE_GATE", "④′resource-gate"),
    ("RESOURCE_GATE_NOTIFY", "④′resource-gate-notify"),
    ("RESOURCE_GATE_SKIP", "④′resource-gate-skip"),
    # ★W2: 팩↔바이너리 스큐 폴백은 ④boot **판정 기록 이전**에 남는다(선언 순서=실행 순서 계약).
    ("BOOT_SKEW", "④boot-skew"),
    ("BOOT", "④boot"),
    ("BOOT_BUSY", "④boot-busy"),
    # ★(M3-짝) 제3 상태 — 성공도 실패도 busy 도 아닌 '관문 보류'. busy 판정 **뒤**, Degrade
    #   **앞**에 선언한다(선언 순서 = 실행 순서 계약). 종전엔 이 사실이 Degrade 로 접혀
    #   "Fatal 역할은 전원 확보" 라는 거짓 문장으로 기록됐다.
    ("BOOT_GATE_PENDING", "④boot-gate-pending"),
    ("BOOT_DEGRADE", "④boot-degrade"),
    ("BOOT_TICKET_CONSUME", "④boot-ticket-consume"),
    ("BOOT_REVIEWERS", "④b-boot-reviewers"),
    ("BOOT_REVIEWERS_PERMANENT", "④b-permanent"),
    ("BOOT_SKIP", "④boot-skip"),
    ("CHECK", "⑤check"),
    ("CHECK_UNJUDGEABLE", "⑤check-unjudgeable"),
    ("MARKER", "⑥marker"),
    ("PROMOTE_REQUEST", "⑦promote-request"),
)


class _StepIds:
    """단계 라벨 상수 네임스페이스(enum 승격) — `STEP.BOOT` 형태로만 참조한다."""

    def __init__(self, defs):
        for name, label in defs:
            setattr(self, name, label)


STEP = _StepIds(_STEP_DEFS)
STEP_ORDER = tuple(label for _, label in _STEP_DEFS)
STEP_INDEX = {label: i for i, label in enumerate(STEP_ORDER)}


def _load_retry_carry(path):
    """boot-last 최상위 "retry" 맵의 carry-forward 스냅샷(P0-3 · 파손·부재 안전).

    ★호출 시점 계약: cmd_run 이 **_Log 생성 이전**에 부른다 — _Log.__init__ 은 새 data 딕트의
      선기록(_persist)으로 슬롯을 즉시 덮으므로, 그 뒤에 읽으면 이월할 원본이 이미 없다
      (R3-P03-1 실측: __init__ 은 self.path 를 읽지 않는다 — 이 함수가 유일한 선독 지점이다).
    ★파손·부재는 빈 맵으로 접는다(래치 소실 = 재시도 최대 1회 추가 — 유계·수용). 항목 검증은
      보수적으로: count 양의 정수·at 실수·TTL(24h) 창 안(미래 스탬프 = 시계 후퇴 잔재도 회수)
      만 이월한다 — 위조·오염된 항목이 소진(false)을 조작하는 방향보다 '한 번 더 재시도'가
      싸다(신뢰 모델: 드리프트 가드이지 보안 경계가 아니다).
    """
    prev = _read_json(path)
    out = {}
    if not isinstance(prev, dict):
        return out
    raw = prev.get("retry")
    if not isinstance(raw, dict):
        return out
    now = time.time()
    for sid, ent in raw.items():
        if not isinstance(ent, dict):
            continue
        try:
            count = int(ent.get("count", 0) or 0)
            at = float(ent.get("at", 0) or 0)
        except (TypeError, ValueError):
            continue
        if count <= 0:
            continue
        if not (0 <= now - at < RETRY_LATCH_TTL_S):   # 만료(24h+)·미래 스탬프 회수(맵 유계)
            continue
        out[str(sid)] = {"count": count, "at": at}
    return out


class _Log:
    """단계 결과를 (레인) boot-last 에 누적(진단 가시성 — 각 retry 시도 포함).

    ★디스크 반영은 전부 `_persist()`(best-effort — W-A3 ③) 를 경유한다: 계측 쓰기 실패
      (Windows 공유 위반 등)가 부트를 죽이지 않되, 실패 사실은 stderr + log_write_failures
      필드로 남는다(보고=실측 비약화). 새 쓰기 지점을 추가하면 반드시 `_persist` 를 쓰라 —
      `_atomic_write_json(self.path, …)` 직호출은 finally 재예외 크래시 경로의 부활이다.

    ★A19 런 정체성: 레코드는 이제 **누가/언제/어디서** 돈 런인지 스스로 말한다.
      run_id(started+pid)·pid·surface·role 이 없던 종전에는 (ⓐ)한 레인의 두 pane 중 누구의 런인지,
      (ⓑ)진행 중인지 크래시했는지 구분이 불가능했다(둘 다 'result 키 없음'으로 보였다).
      `result:{"ok":null,"state":"running"}` 선기록 + finish() 의 try/finally 종결 기록이 그 둘을
      기계로 분리한다.
    ★role 은 **env 파생**(CYS_ROLE — 데몬이 pane에 주입)이다: 여기서 `cys surface-role` 왕복을
      추가하면 부트 시작 전에 데몬 왕복이 하나 더 늘고, ③ 이전에는 아직 claim 도 안 된 상태라
      권위값도 아니다. ③ 성공 후 `role_claimed` 를 별도로 남긴다(관측 파생 — 보고=실측).
    """

    def __init__(self, retry_carry=None):
        started = time.strftime("%Y-%m-%dT%H:%M:%S")
        self.pid = os.getpid()
        self.run_id = "%s-%d" % (started, self.pid)
        self.surface = _my_surface_id()
        # ★G15: 기록 대상은 **레인 boot-last** 다(base 레인은 역사적 경로 그대로).
        #   전 레인 공유 단일 파일이던 종전에는 base·부서 동시 부트가 서로의 진단을 덮었다.
        self.path = lane_state_path("boot_last")
        self.lane = lane_key()
        self.data = {"started": started, "run_id": self.run_id, "pid": self.pid,
                     "surface": self.surface, "role": os.environ.get("CYS_ROLE", ""),
                     "lane": self.lane, "boot_last_path": self.path,
                     "steps": [],
                     # ★P0-3 재시도 래치 carry-forward: 선기록이 슬롯을 덮어도 per-surface
                     #   재시도 원장은 이월된다(스냅샷은 호출자 cmd_run 이 _Log 생성 **이전**에
                     #   뜬다 — _load_retry_carry 계약 주석). 이 맵이 없으면 타 pane 완주 런이
                     #   본체를 덮을 때마다 소진 래치가 재무장한다(R3-P03-1 음성 독해).
                     "retry": dict(retry_carry or {}),
                     "socket": os.environ.get("CYS_SOCKET", ""), "base_socket": _is_base_socket(),
                     # ★선기록 — 이 시점 이후 어떤 경로로 죽어도 '진행 중'이 남는다.
                     "result": {"ok": None, "state": "running", "run_id": self.run_id,
                                "surface": self.surface}}
        self._last_step_order = -1
        self._persist()

    def _attributed(self, res):
        """result 딕트에 런 귀속을 못박는다 — §0 소비 술어('자기 surface의 최신 완주 런')의 전제."""
        res.setdefault("run_id", self.run_id)
        res.setdefault("surface", self.surface)
        res.setdefault("pid", self.pid)
        return res

    def _persist(self):
        """boot-last 디스크 반영 — **best-effort 예외 흡수**(W-A3 ③ · _Log 쓰기 유일 경로).

        ★왜 흡수하는가: 진단 계측(_Log)의 쓰기 실패가 부트 본체를 죽이면 안 된다. 실재 경로 —
          Windows 공유 위반: 뷰어·백업·인덱서가 boot-last.json 을 연 동안 os.replace 가
          PermissionError 를 던진다. 종전엔 그 예외가 ⓐ step()/result() 를 타고 체인을 즉사시켰고
          ⓑ cmd_run 의 finally→finish() 가 **같은 파일에 다시 쓰다 같은 예외를 재발생**시켜,
          체인이 EXIT_OK 로 완주한 경우조차 uncaught 크래시(exit 1)로 뒤집었다(finally 발 예외가
          정상 return 을 삼킨다).
        ★조용히 삼키지 않는다(보고=실측 계약 유지): 실패마다 ⓐstderr 1줄 ⓑself.data 에
          log_write_failures(누적 횟수·마지막 원인)를 남긴다 — data 는 누적 구조라 **다음 성공
          write 가 실패 사실까지 파일에 박제**한다. 전 write 가 실패하면 stderr 줄들이 최종
          증거다(쓰기 실패의 기록 자체를 잃지 않는다).
        ★대안 기각 — 쓰기 재시도 루프: 공유 위반은 수 초 지속될 수 있고 부트 경로의 동기 재시도는
          싱글플라이트 락 보유 연장(치명 앵커 ③)이다. 어차피 다음 단계의 write 가 곧 같은 내용을
          다시 시도한다(유실 없음) — 여기서 기다릴 이유가 없다.
        """
        try:
            _atomic_write_json(self.path, self.data)
            return True
        except Exception as e:
            info = self.data.setdefault("log_write_failures", {"count": 0, "last": ""})
            info["count"] = int(info.get("count", 0) or 0) + 1
            info["last"] = ("%s: %s" % (type(e).__name__, e))[:300]
            try:
                sys.stderr.write("[bootstrap] ⚠ boot-last 기록 실패 %d회(best-effort 계속·부트 비중단): %s\n"
                                 % (info["count"], info["last"]))
                sys.stderr.flush()
            except Exception:
                pass  # stderr 자체 불능 — 계측의 계측은 여기서 끝낸다(부트 본체가 우선)
            return False

    def write_failures(self):
        """이번 런에서 boot-last **쓰기가 실패한 횟수**(0 = 지금까지 전부 디스크에 반영됐다).

        `_persist` 가 best-effort 로 흡수한 실패의 유일한 기계 관측점이다. 이 값이 0 이 아니면
        디스크의 boot-last 는 이 런의 사실을 담고 있지 **않을 수 있다**.
        """
        info = self.data.get("log_write_failures") or {}
        try:
            return int(info.get("count", 0) or 0)
        except (TypeError, ValueError):
            return 0

    def _seal_session_error(self, exit_code):
        """★R2-3(2026-08-26 적대검증) — session_error 종결을 **지속성 관측과 교차**하고
        디스크가 죽어도 남는 stdout 채널을 만든다.

        ## 무엇이 틀렸었는가
        P0-3 래치와 §0-A 소비면은 boot-last 쓰기 **하나**에 단일 의존하는데 그 쓰기 실패는
        `_persist` 가 침묵 흡수한다(부트 본체를 죽이지 않기 위한 정당한 설계). 그래서 정상
        완주(`ok:true`)가 래치를 비운 뒤 boot-last 쓰기가 영구 실패하면, session_error 런은
        디스크에 **전혀 반영되지 않고** 모델이 읽는 유일 채널에는 직전의 `ok:true` 가 남는다.
        실측(모듈 직접 구동): state 디렉터리를 0o500 으로 잠근 뒤 fail(…session_error) →
        디스크 result = `{ok:true,state:completed}`(무변화) · 인메모리 = session_error ·
        `log_write_failures.count=5`. 귀결 둘 — ⓐ master 가 팀 0노드에서 '기동 완료'를 보고
        하고(P0-4 정직 예보가 막으려던 허위 낙관의 디스크면) ⓑ 매 런이 자기 이력 0 으로
        재판정돼 `retry_eligible` 이 항상 true(기계 유계 무효화).
        결정적으로 **쓰기 실패의 증거(log_write_failures)가 쓰기에 실패한 바로 그 파일 안에만**
        저장된다 — 저장소 교리('측정 불능은 어떤 게이트에서도 통과가 아니다')와 반대 방향이다.

        ## 수리
        ① 지속성 교차 — 이번 런의 `_persist` 가 1회라도 실패했으면 `retry_eligible=false` 로
           접고 `retry_eligible_unknown=true`·`persist_failed=N` 을 함께 남긴다(측정 불능은
           통과가 아니다 = 재실행 허가도 아니다).
        ② stdout 미러 — 성공 경로의 A7 최종 JSON 과 **동형**으로, 실패 경로에도 모델이 인용할
           수 있는 1줄 기계 JSON 을 남긴다(디스크가 죽어도 이 채널은 산다).
        ③ 런 정체성 동봉 — run_id·surface 를 실어 §0-A 가 남의 런·묵은 런을 자기 결과로 읽지
           못하게 한다(판독 규약의 stale 판정 재료).
        """
        res = self.data.get("result")
        if not isinstance(res, dict):
            return
        wf = self.write_failures()
        if wf:
            res["retry_eligible"] = False
            res["retry_eligible_unknown"] = True
            res["persist_failed"] = wf
            self._persist()          # 되면 좋고, 안 되면 아래 stdout 이 최종 증거다
        line = {"channel": "boot-last-mirror", "state": res.get("state"),
                "exit": exit_code, "run_id": self.run_id, "surface": self.surface,
                "lane": self.lane, "boot_last": self.path,
                "retry_eligible": res.get("retry_eligible"),
                "retry_eligible_unknown": bool(res.get("retry_eligible_unknown")),
                "log_write_failures": wf}
        try:
            sys.stdout.write(json.dumps(line, ensure_ascii=False) + "\n")
            sys.stdout.flush()
        except Exception:
            pass                     # stdout 자체 불능 — 계측의 계측은 여기서 끝낸다

    def step(self, name, code, detail="", suffix=""):
        """단계 기록. `name` 은 **STEP.* 상수**여야 한다(P3-A-STEP-NAME).

        `suffix` 는 같은 단계의 반복 시도 표기(⑤check 의 `#3`) — 정체성은 base 라벨이 갖고
        순서·유일성 판정도 base 라벨로 한다(구 `"⑤check#%d" % attempt` 문자열 조립이 단계 정체성을
        매 시도마다 새로 만들던 것을 없앤다).
        ★측정 실패를 침묵시키지 않는다: 미등록 라벨·순서 역행을 레코드에 상태로 남기고 stderr 로도
          알린다(부트는 계속 — 진단 계측이 부트를 죽이면 안 된다).
        """
        rec = {"step": name + suffix, "exit": code, "detail": detail.strip()[-2000:]}
        idx = STEP_INDEX.get(name)
        if idx is None:
            rec["step_unregistered"] = True
            sys.stderr.write("[bootstrap] ⚠ 미등록 단계 라벨: %r(레지스트리 갱신 필요)\n" % name)
        else:
            rec["order"] = idx
            if idx < self._last_step_order:
                rec["order_violation"] = self._last_step_order
                sys.stderr.write("[bootstrap] ⚠ 단계 순서 역행: %s(order %d < %d)\n"
                                 % (name, idx, self._last_step_order))
            self._last_step_order = max(self._last_step_order, idx)
        self.data["steps"].append(rec)
        self._persist()

    def result(self, **kw):
        """단계 성공/강등 경로의 result 기록(귀속 자동 첨부).

        ★P0-3 래치 리셋: 자기 sid 의 **정상 완주(ok:true)** 만 재시도 항목을 제거한다 —
          declined(exit 7)·session_error(exit 10)·dept_fallback 등 ok:null 경로는 리셋이
          아니다(수리 관측 없이 래치를 풀면 소진 상한이 무의미해진다). 리셋 조건을 '직전 런
          대조'가 아니라 완주 관측으로 두므로 교차 실행이 래치를 지우지 못한다.
        """
        if kw.get("ok") is True:
            (self.data.get("retry") or {}).pop(self.surface, None)
        self.data["result"] = self._attributed(dict(kw))
        self._persist()

    def finish(self, exit_code, exc=None):
        """★A19 종결 기록 — cmd_run 의 finally 가 유일 호출자. 어떤 경로로 끝나도 도달한다.

        체인이 result 를 남기지 못한 채 끝났다면(=크래시·중단) 그 사실을 상태로 박는다.
        종전에는 SIGKILL·예외·중간 return 이 전부 '기록 없음'으로 수렴해 '진행 중'과 구분되지
        않았다(A19 재검증: "필드 추가만으론 '중단 vs 크래시' 융합 잔존 — try/finally 필수").
        """
        self.data["ended"] = time.strftime("%Y-%m-%dT%H:%M:%S")
        self.data["exit"] = exit_code
        if exc:
            self.data["exc"] = str(exc)[:500]
        res = self.data.get("result")
        if not isinstance(res, dict) or res.get("state") == "running":
            self.data["result"] = self._attributed(
                {"ok": None, "state": "crashed" if exc else "aborted",
                 "exit": exit_code, "exc": (str(exc)[:500] if exc else None)})
        else:
            res.setdefault("exit", exit_code)
            self._attributed(res)
        # ★W-A3 ③: 종결 기록도 best-effort — finally 발 재예외가 체인의 정상 exit 를 삼키는
        #   경로(위 docstring ⓑ)의 봉인 지점이 바로 여기다.
        self._persist()

    def fail(self, name, code, detail, exit_code, ok=False, state="failed"):
        """실패·거부 경로의 공통 종결(단계 기록 → result → stderr → loud 알림).

        ★ok/state 파라미터(CS-2⑩ · 비평2 C-3): 정당거부(exit 7)·세션 컨텍스트 오류(exit 10)는
          `ok=None, state='declined'|'session_error'` 로 남긴다 — 공유 boot-last 에 `ok:false` 를
          덮으면 같은 레인의 건강한 master 가 남긴 ok:true 를 남의 pane 이 지워, §0 의 '직접 실행'
          분기를 무한 churn 시킨다(부트 폭풍). 인프라 실패(ping·boot·check·lane-pack)는 그대로
          ok=False 다 — 그건 실제로 이 레인의 부트가 깨진 사실이다.
        ★P0-3 재시도 래치 기록: state='session_error' 일 때만 자기 sid 의 카운터를 +1 하고
          result 에 `retry_eligible=(기록 후 count<=RETRY_LATCH_MAX)` 를 파생한다 — §0-A 의
          session_error 행이 소비하는 **유일한** 재실행 근거다(오너 결정 ⑬Y: 최초 실패
          count=1=true · 같은 surface 연속 2회째부터 false=소진). session_error 기록 지점은
          이 함수의 두 호출부(③ 선행/직접 claim 비거부 실패 · ③-d 폴백 전제 미확인)뿐이라
          여기 한 곳이 래치 갱신의 단일 소유자다.
        """
        extra = {}
        if state == "session_error":
            latch = self.data.setdefault("retry", {})
            ent = latch.get(self.surface)
            count = (int(ent.get("count", 0) or 0) if isinstance(ent, dict) else 0) + 1
            latch[self.surface] = {"count": count, "at": time.time()}
            extra["retry_eligible"] = (count <= RETRY_LATCH_MAX)
        self.step(name, code, detail)
        self.result(ok=ok, state=state, failed_step=name, exit=exit_code, **extra)
        if state == "session_error":
            self._seal_session_error(exit_code)
        sys.stderr.write("[bootstrap] 단계 실패: %s (exit %d)\n%s\n" % (name, code, detail.strip()))
        # ★실패 가시화(2026-07-15 적대검증 adv#5): 훅이 배경 실행이라 stderr가 화면에 안 보인다.
        # 훅 NOTE는 "팀이 뜬다"고 알렸는데 부트가 조용히 실패하면 사용자는 원인을 모른다 — 알림으로 승격.
        #
        # ★W1a A15+R2 — notifier 단일화 + ②ping 카브아웃 제거:
        #   ① 종전엔 여기서 `cys feed push` 를 직접 호출해 `_notify_loud`(feed→send 폴백)와 **이중
        #      구현**이었다(R2). 채널 정책이 두 곳에 흩어지면 한쪽만 고쳐지는 드리프트가 난다 →
        #      알림 채널의 단일 소유자는 `_notify_loud` 하나다.
        #   ② 종전 `if name != "②ping"` 카브아웃은 **채널 가용성(런타임)을 단계 정체성(정적)으로
        #      대체**한 오류였다(A15). ②ping 실패가 곧 '알림 불가'는 아니다 — ping은 짧은 타임아웃·
        #      경합·부분 장애로도 실패하고, 데몬이 정말 죽었어도 send --queued 폴백이 큐에 남길 수
        #      있다. 가용성은 **시도해 보고** 판정한다(_notify_loud는 best-effort·짧은 timeout이라
        #      데몬 부재에서도 행 걸지 않는다).
        #   ③ 알림 결과 채널명을 boot-last에 남긴다 — '알렸다'는 주장이 아니라 실측 파생 기록
        #      (CS-3 보고=실측). 'none(...)' 이면 비제로 exit·boot-last가 최종 증거다.
        # ★키는 STEP 상수다(리터럴 금지) — 라벨이 바뀌면 힌트가 조용히 안 붙는 드리프트를 차단한다.
        hint = {STEP.CLAIM_ROLE: "다른 pane이 이미 master입니다(조직당 master 1명). 새 부서장을 세우려면 "
                                 "GUI ＋부서(부서 워크스페이스 추가)를 쓰거나, base 레인(unix)에서 오너가 "
                                 "직접 타이핑한 선언(훅 발화)으로 재선언하세요 — 그 경로만 부서 자동 생성으로 "
                                 "이어집니다.",
                STEP.DEPT_FB_GUARD: "부서 자동 생성의 전제(살아있는 master)가 확인되지 않아 만들지 "
                                    "않았습니다 — 역할 등록이 '신원 미확정'으로 거부됐을 가능성이 "
                                    "큽니다(세션 배선). `cys list` 의 role 열을 확인하고, pane 안에서 "
                                    "재선언하세요. 새 부서가 목적이면 GUI ＋부서·`cys-dept allocate` 를 쓰세요.",
                STEP.DEPT_FB_ALLOC: "부서 자동 생성 실패 — 부서 상한(CYS_DEPT_CAP 기본 8)·~/.cys/depts.json 을 확인하세요.",
                STEP.DEPT_FB_MASTER: "부서는 생성됐지만 부서장 기동 실패 — 그 부서 pane에서 claude 실행 후 '너는 마스터다' 선언(훅 자동 부트) 또는 부서 pane 안에서 cys launch-agent --role master --agent claude 로 재시도하세요.",
                STEP.BOOT: "팀(CSO·워커·리뷰어) 기동 실패 — claude CLI 설치를 확인하세요.",
                STEP.CHECK: "팀 노드가 제 시간에 안 떴습니다 — cys list로 확인하고 필요시 재선언하세요.",
                STEP.PING: "cysd 데몬에 응답이 없습니다 — cys list로 데몬 상태를 확인하세요(자동 기동 대기 중일 수 있음).",
                STEP.CLAIM_ROLE_CONTEXT: "역할 등록 왕복이 세션 컨텍스트 오류로 실패했습니다 — "
                                       "이 세션이 cys pane 안인지(CYS_SURFACE_ID)와 데몬 응답을 확인하세요"
                                       "(‘남이 master’라는 뜻이 아닙니다).",
                }.get(name, "부트스트랩이 %s 단계에서 실패했습니다 — cys list·boot-last.json 확인." % name)
        channel = _notify_loud("부트스트랩 미완(%s)" % name, hint)
        self.data["result"]["notify"] = {"attempted": True, "channel": channel}
        self._persist()
        return exit_code


def _observe_surface_role():
    """이 surface 의 데몬 권위 역할 관측 → (role|None, 사유). None = 판정 불가.

    ★G17 전용(싱글플라이트 패자 경로에서만 호출): 부트를 건너뛴 pane 이 **자기 신원을 확인하지
      않고** 조용히 exit 0 하면, 그 pane 의 LLM 은 "부트 완료된 master"를 자칭하며 지휘를
      계속한다(자칭 master 잔존 — javis_bootstrap.py:516 재검증). 그래서 skip verdict 에는
      '나는 master 가 아니다'를 명시할 근거를 실측으로 붙인다.
    ★rc0+빈출력 삼킴(A5)은 여기서 완전 해소되지 않는다(cys.rs 3상화는 W2) — 다만 CYS_SURFACE_ID
      가 있는데 빈값이면 '미claim(비-master)'로 읽는 것이 보수적이고 이 목적에 정확하다.
    """
    if not _my_surface_id():
        return None, "CYS_SURFACE_ID 부재 — surface 귀속 불가"
    code, out = _run(["cys", "surface-role"], timeout=5)
    if code != 0:
        return None, "surface-role 판정 불가(rc=%s)" % code
    role = (out or "").strip().splitlines()
    role = role[0].strip() if role else ""
    return role, ("role=%s" % role if role else "미claim(빈 좌석)")


def _skip_record_path():
    """싱글플라이트 skip 기록 경로(레인별) — boot-last 본체를 덮지 않는다(단일-writer 보존)."""
    return lane_state_path("skip")


def _emit_skip_verdict():
    """A7·G17: 싱글플라이트 패자의 타입드 종료 — stderr 1줄 verdict JSON + exit 11.

    ★즉시 반환한다(수렴 대기 금지 — 금지 방향 ⑨). 정상 시나리오가 이미 동시 2호출
      (role-bootstrap 백그라운드 + session-start 산문 지시)이므로, 여기서 승자의 완주를 기다리면
      LLM 의 Bash 호출이 냉부팅 최악 예산(≈1555s)만큼 블록되고 도구 타임아웃→재시도 홍수가 된다.
    ★stdout 무접촉: '완료 선언은 최종 JSON 인용 시에만'이라는 배포된 산문 계약을 지키려면 skip 은
      절대 stdout 에 JSON 을 내면 안 된다(구 코드는 그냥 침묵했고, 침묵은 자칭 master 를 낳았다).
    """
    role, why = _observe_surface_role()
    is_master = (role == "master") if role is not None else None
    if is_master is True:
        self_check = ("이 surface 는 master 좌석이다 — 그러나 부트는 **다른 런**이 진행 중이므로 "
                      "재실행하지 말고 cys list·boot-last.json 으로 그 런의 결과를 확인하라.")
    elif is_master is False:
        self_check = ("이 surface 는 **비-master**(%s) 다 — 마스터를 자칭하거나 팀을 지휘하지 마라. "
                      "부트는 다른 pane 이 소유한다." % why)
    else:
        self_check = ("이 surface 의 역할을 판정할 수 없다(%s) — master 를 자칭하지 마라"
                      "(판정 불가는 '나는 master' 가 아니다)." % why)
    verdict = {"verdict": "skipped_inflight", "ok": None, "exit": EXIT_SKIPPED_INFLIGHT,
               "reason": "다른 부트스트랩 런이 진행 중(단일 실행 락 비획득) — 즉시 반환",
               "run_id": "%s-%d" % (time.strftime("%Y-%m-%dT%H:%M:%S"), os.getpid()),
               "pid": os.getpid(), "surface": _my_surface_id(),
               "surface_role": role, "is_master": is_master, "self_check": self_check,
               "lock": _singleflight_path(), "waited": False,
               "boot_last_untouched": True, "record": _skip_record_path()}
    _progress("부트스트랩 이미 진행 중(단일 실행 락) — 중복 실행 skip(exit %d). 진행은 cys list로 확인."
              % EXIT_SKIPPED_INFLIGHT)
    sys.stderr.write(json.dumps(verdict, ensure_ascii=False) + "\n")
    try:
        _atomic_write_json(_skip_record_path(), verdict)
    except OSError:
        pass          # 기록 실패가 skip 을 실패로 바꾸면 안 된다(verdict 는 이미 stderr 에 있다)
    return EXIT_SKIPPED_INFLIGHT


def _legacy_singleflight_key(sock):
    """★U-24 **레거시 폴백 전용**(정본은 `javis_lane.singleflight_key`).
    순수 판정: 소켓 → 싱글플라이트 락 키(R1-LOW-4). base 레인은 env 미설정·base 경로 명시
    어느 쪽이든 단일 'base' 키로 정규화한다 — 같은 base 데몬에 서로 다른 락을 주던 선재결함 교정.
    비-base(부서·커스텀)는 전체 경로 새니타이즈로 레인마다 유일.
    ★레거시 체인은 **레거시 위에서 닫힌다**(재수출된 이름을 부르지 않는다): 정본이 있는 트리에서
      폴백이 정본 조각을 빌려 쓰면 검체 `H-LANE-1` 의 파리티가 '같은 코드끼리 비교'가 되어
      동치 증명이 공허해진다. 소켓 판정·키 새니타이즈까지 레거시 것을 명시 호출한다."""
    return ("base" if _legacy_socket_is_base(sock)
            else _legacy_sanitize_sock_key(sock))


def _legacy_lane_key(sock=None):
    """★U-24 **레거시 폴백 전용**(정본은 `javis_lane.lane_key`).
    이 부트가 속한 **레인 키** — 'base' 또는 소켓 경로 새니타이즈 값(레인마다 유일).
    락 키와 동일 규약을 쓴다(`_singleflight_key`) — 락은 레인별인데 상태 파일은 공유였던
    비대칭(G15·R3)을 없애려면 두 네임스페이스가 **같은 키 함수**를 써야 한다."""
    return _legacy_singleflight_key(
        os.environ.get("CYS_SOCKET", "") if sock is None else sock)


_singleflight_key = (_lane_mod.singleflight_key if _lane_mod is not None
                     else _legacy_singleflight_key)
lane_key = _lane_mod.lane_key if _lane_mod is not None else _legacy_lane_key


# 레인 스코프 상태의 **경로 규약 단일 소유자**(G15 · P3-A-DEPT-LANE · CS-7②).
# ★U-24: 소유자가 `javis_lane` 로 이사했다. 아래 표·함수는 **레거시 폴백 전용 사본**이며
#   실사용 바인딩은 이 절 끝의 재수출이 결정한다(정본 부재·명시 롤백에서만 아래가 산다).
#
# ★결함: 락은 레인별(bootstrap-<lane>.lock)인데 상태는 **전 레인 공유 단일 파일**이었다 —
#   base 와 부서가 동시에 부트하면 서로의 boot-last.json 을 덮어 진단 SOT 가 소실됐고(G15),
#   부서 레인은 마커가 아예 없어 재선언마다 300s preflight 를 통째로 다시 돌았다
#   (P3-A-DEPT-LANE: fast path 부재).
# ★불변식 2개(금지 방향 ①):
#   ⓐ **base 마커 경로는 절대 레인화하지 않는다** — cys-dept 의 CEO 승격 게이트가 그 파일의
#      존재를 읽는다. 부서 마커를 base 경로에 쓰면 게이트가 오개방된다.
#   ⓑ base 레인의 경로는 **역사적 경로 그대로**다(§0 산문·GUI·테스트 호환·회귀 0). 접미는
#      비-base 레인에만 붙는다.
# ★같은 레인의 다중 pane 오염은 이 분리로 해결되지 않는다 — 그쪽은 run 귀속(CS-2⑩·W1b)이 담당한다.
# ★base_dir 자리의 `_STATE` 는 **지연 해소 표식**이다(리터럴 경로 아님) — import 시점에 얼린
#   문자열을 넣으면 `CYS_STATE_DIR` 을 나중에 바꾼 격리 실행(self-test·테스트 하네스)에서
#   경로가 실 HOME 으로 새어 나간다(2026-08-01 T1 밀폐 붕괴의 기제).
_STATE = "\0state_dir"
_LEGACY_LANE_STATE_KINDS = {
    "marker": (CYS_DIR, ".master-bootstrapped", ""),
    "boot_last": (_STATE, "boot-last", ".json"),
    "skip": (_STATE, "boot-skip", ".json"),
    "lock": (_STATE, "bootstrap", ".lock"),
    # ★T1(2026-08-01 윈도우 실사고): 임무 대장 — '이 세션에 오너가 임무를 지정했는가'의 결정론
    #   상태. 소유자는 `javis_mission.py`(판정)이고 **경로 규약만** 여기서 발급한다(사본 금지).
    #   레인별인 이유: 부서 레인의 오너 임무가 base master 의 자율 착수 권한이 되면 안 된다.
    "mission": (_STATE, "mission", ".json"),
    # ★R1(2026-08-01 배달 원장): **데몬(cysd)이 쓰고 훅이 읽는** out-of-band 채널.
    #   - delivery       : pane stdin 주입 직전의 append-only 원장(JSONL) — '이 문장은 기계가
    #                      밀어 넣은 것'의 증거. 문자열 라벨(발신자가 고를 수 있는 값)에
    #                      의존하던 기계/오너 판별을 대체한다.
    #   - delivery_epoch : 데몬 인스턴스 표식 — 임무의 **세션 결박**(과거 임무 무기한 유효 차단).
    #   ★둘 다 **항상 레인 접미**다(base 도 `delivery-base.jsonl`). 역사적 무접미 경로가 없어
    #     base 예외를 둘 이유가 없고, 접미가 항상 있으면 파일명만으로 레인이 결정론이다.
    #   ★생산자는 Rust `src/bin/cysd/delivery.rs`(ledger_path/epoch_path) — 경로 규약이 갈리면
    #     원장이 **조용히** 무력화되므로 양쪽에 교차 테스트를 둔다.
    "delivery": (_STATE, "delivery", ".jsonl"),
    "delivery_epoch": (_STATE, "delivery", ".epoch.json"),
}

# 항상 레인 접미가 붙는 종류(base 예외 없음) — 위 주석의 규약을 코드로 고정한다.
_LEGACY_ALWAYS_LANE_SUFFIXED = ("skip", "lock", "delivery", "delivery_epoch")


def _legacy_lane_state_path(kind, sock=None):
    """★U-24 **레거시 폴백 전용**(정본은 `javis_lane.lane_state_path`).
    레인 스코프 상태 파일 경로.
    kind ∈ marker|boot_last|skip|lock|mission|delivery|delivery_epoch.
    base 레인: 역사적 경로(마커=`~/.cys/.master-bootstrapped` · `boot-last.json`).
    비-base 레인: `-<lane>` 접미(`.master-bootstrapped-<lane>` · `boot-last-<lane>.json`).
    ※ skip·lock·delivery* 는 **항상** 레인별이다 — 규약을 이 함수 하나로 모은다(사본 금지)."""
    try:
        base_dir, stem, ext = _LEGACY_LANE_STATE_KINDS[kind]
    except KeyError:
        raise ValueError("미지 레인 상태 종류: %r" % kind)
    if base_dir == _STATE:                       # 지연 해소(위 주석) — 호출 시점의 env 를 본다
        base_dir = state_dir()
    key = _legacy_lane_key(sock)
    if kind in _LEGACY_ALWAYS_LANE_SUFFIXED:
        return os.path.join(base_dir, "%s-%s%s" % (stem, key, ext))   # 항상 레인별(구 동작 보존)
    if key == "base":
        return os.path.join(base_dir, stem + ext)
    return os.path.join(base_dir, "%s-%s%s" % (stem, key, ext))


# ── 재수출(U-24) — 이 세 이름이 **공개 계약**이다. 소비자는 무개정으로 계속 소비한다. ──
#   `javis_mission.py`(import) · 훅 `lane-path`(cmd_lane_path) · 검체 `H-LIFE-1`(B.lane_state_path)
#   · 이 파일 `--self-test`. 정본이 있으면 정본 객체 **그 자체**를 바인딩한다(사본 아님).
_LANE_STATE_KINDS = (_lane_mod.LANE_STATE_KINDS if _lane_mod is not None
                     else _LEGACY_LANE_STATE_KINDS)
_ALWAYS_LANE_SUFFIXED = (_lane_mod.ALWAYS_LANE_SUFFIXED if _lane_mod is not None
                         else _LEGACY_ALWAYS_LANE_SUFFIXED)
lane_state_path = (_lane_mod.lane_state_path if _lane_mod is not None
                   else _legacy_lane_state_path)


def _singleflight_path():
    """싱글플라이트 락 파일 경로(레인별) — 경로 규약은 lane_state_path 단일 소유."""
    return lane_state_path("lock")


def _acquire_singleflight():
    """부트스트랩 전체 단일 실행 락(2026-07-15 적대검증·아키텍트: preflight 300s는 boot 락으로
    직렬화되지 않아 중복 fire가 settings.json read-modify-write를 경쟁하고 300s 프리플라이트를 중복
    실행했다). 소켓별 비차단 — 이미 진행 중이면 None 반환(호출부가 no-op 종료).

    ★W1a A8py: 구현을 `javis_lock.FileLock` 로 교체했다. 종전 인라인은 `if os.name == "posix"`
      가드로 fcntl만 걸어 **Windows에서 직렬화가 전무**했다(락을 못 잡는 게 아니라 '항상 획득'으로
      접혀 중복 부트가 결정론적으로 무장 — A6와 같은 웨이브가 필수인 이유). 이제 백엔드가
      posix=flock / windows=msvcrt.locking / 폴백=pidfile(+스테일 pid 회수 — R1의 최대 ~330s
      부트 거부 창 해소)로 가용성 기반 분기한다.
    ★타입드 3상 소비: acquired→진행 / busy→None(no-op) / unavailable→진행(보수적 허용).
      'busy'와 'unavailable'을 융합하면 락 인프라 고장이 조용한 부트 거부가 된다(구 코드는
      `except OSError: return True` 로 unavailable만 분리했고 windows busy는 아예 없었다).
    ★fd 보유: FileLock 인스턴스를 모듈 전역에 붙여 GC로 fd가 닫혀 락이 조용히 풀리는 것을 막는다."""
    lock_path = _singleflight_path()
    if _lock is None:
        # 팩 스큐(javis_lock 부재) — 구 동작으로 강등: 직렬화 없이 진행. 조용히 접지 않게 흔적 1줄.
        _progress("경고: javis_lock 미적재 — 싱글플라이트 직렬화 없이 진행(팩 배포 확인 필요)")
        return True
    lk = _lock.FileLock(lock_path, owner="javis_bootstrap", blocking=False)
    st = lk.acquire()
    if st == _lock.BUSY:
        return None  # 다른 부트스트랩 진행 중 — no-op
    if st == _lock.UNAVAILABLE:
        _progress("경고: 단일 실행 락 사용 불가(%s) — 직렬화 없이 진행" % lk.detail)
        return True
    _acquire_singleflight._lk = lk  # 프로세스 수명동안 보유(GC 해제 차단)
    if lk.reclaimed_stale:
        _progress("스테일 부트 락 회수(사망 보유자) — 신규 부트 진행")
    return True


# ── 증분2 ⓐ: CEO 티켓 권한 게이트(P7) ──
# 이 게이트는 LLM 드리프트 차단용 결정론 가드이지 보안 경계가 아니다(동일 $HOME 신뢰 도메인).
def _ticket_path(dept):
    return os.path.join(TICKET_DIR, "%s.ticket" % dept)


def _parse_ticket_json(text, dept, now):
    """순수 판정: 티켓 파일 텍스트 → (유효 bool, 사유). now(epoch) 주입으로 TTL 결정론 검증.
    계약: JSON 객체 · dept 일치 · issued_at(epoch 숫자) 존재 · 0<=경과<=TTL. 위반=강등(단독 각성)."""
    try:
        d = json.loads(text)
    except (ValueError, TypeError):
        return False, "티켓 JSON 파싱 실패"
    if not isinstance(d, dict):
        return False, "티켓 루트가 객체 아님"
    if d.get("dept") != dept:
        return False, "티켓 dept 불일치(%r≠%r)" % (d.get("dept"), dept)
    ts = d.get("issued_at")
    if not isinstance(ts, (int, float)) or isinstance(ts, bool):
        return False, "issued_at 없음/형식오류"
    age = now - ts
    if age < 0:
        return False, "issued_at 미래(시계 이상 %ds)" % int(-age)
    if age > TICKET_TTL_SECS:
        return False, "티켓 만료(%dh 경과 > TTL %dh)" % (age / 3600, TICKET_TTL_SECS / 3600)
    return True, "유효(발급 %dm 전 · issuer=%s)" % (age / 60, d.get("issuer", "?"))


def _peek_dept_ticket(dept):
    """부서 티켓 유효성 조회(소비하지 않음). → (유효 bool, 사유, path)."""
    path = _ticket_path(dept)
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return False, "티켓 파일 부재(%s)" % path, path
    ok, why = _parse_ticket_json(text, dept, time.time())
    return ok, why, path


def _consume_dept_ticket(path):
    """티켓을 .used 로 rename(1회성 소비). 실패해도 부트는 계속(best-effort) — 흔적만 반환."""
    try:
        os.replace(path, path + ".used")
        return "소비(.used)"
    except OSError as e:
        return "소비 실패(%s — 이미 rename됐거나 권한): 계속" % e


# ── 증분3 ⓐ: 티켓 부재 → base CEO 에 **결정론 발급 요청**(2026-08-22 현장 결함 #2) ──
def _dept_ticket_request_path(dept):
    return os.path.join(DEPT_TICKET_REQ_DIR, "%s.request" % dept)


def _dept_ticket_request_message(dept):
    """CEO 에게 보낼 요청 문안 — **단일 출처**(테스트가 이 함수를 대조한다).
    수신자가 그대로 복사해 실행할 수 있게 발급 명령을 문안에 넣는다."""
    return ("[부서장→CEO 요청] %s 부서장입니다. 팀 기동용 CEO 티켓 발급을 요청합니다"
            "(javis_bootstrap.py issue-ticket --dept %s)." % (dept, dept))


def _dept_ticket_request_cmd(dept):
    """요청 push 명령 — **단일 출처**. `--queued` 는 대상이 조용해질 때 데몬이 주입하고 Return 도
    데몬이 넣는다(send-key 불필요). 역할 주소 master = base 레인의 CEO 다."""
    return ["cys", "send", "--queued", "--to", "master", _dept_ticket_request_message(dept)]


def _base_lane_env():
    """base 레인 대상 명령의 env — 부서 소켓 상속 제거(`env -u CYS_SOCKET` 과 동형).

    ★surface 변수도 함께 벗긴다: 부서 데몬의 surface id 공간과 base 데몬의 id 공간은 다르므로,
      그대로 물려주면 base 쪽에서 **동번호 남의 pane** 으로 오배선될 여지가 생긴다
      (`_dept_lane_env` 가 반대 방향에서 같은 이유로 pop 하는 것과 대칭이다).
    """
    e = dict(os.environ)
    e.pop("CYS_SOCKET", None)
    e.pop("CYS_SURFACE_ID", None)
    e.pop("CYS_SURFACE_REF", None)
    return e


def _base_ceo_alive():
    """base 레인에 살아있는 CEO(master 좌석 · agent 실재)가 있는가 — `cys status --json` 실측.

    ★술어를 새로 쓰지 않고 `_dept_master_alive` 를 **재사용**한다: '살아있는 master' 판정이
      레인마다 갈리면 두 판정이 따로 낡는다(P0 교정에서 이미 한 번 태운 값이다).
    ★판독 불가(비0·JSON 아님·빈 로스터)는 **없음**으로 접는다 — 이 경로에서 '없음'은 종전
      동작(요청 없이 단독 각성)이므로 안전 방향이다.
    """
    return _dept_master_alive(_base_lane_env())


def _dept_ticket_request_suppressed(dept, now=None):
    """(억제 bool, 사유) — 같은 부서의 최근 요청이 TTL 안이면 재요청하지 않는다(스팸 차단).

    마커 판독 실패(부재·손상)는 **억제하지 않는다**(요청이 나가는 방향 = 팀이 서는 방향).
    """
    now = time.time() if now is None else now
    rec = _read_json(_dept_ticket_request_path(dept))
    if not isinstance(rec, dict):
        return False, "요청 마커 없음(첫 요청)"
    ts = rec.get("requested_at")
    if not isinstance(ts, (int, float)) or isinstance(ts, bool):
        return False, "요청 마커 형식 오류 — 억제하지 않는다"
    age = now - ts
    if age < 0:
        return False, "요청 마커 시각이 미래(시계 이상 %ds) — 억제하지 않는다" % int(-age)
    if age > DEPT_TICKET_REQUEST_TTL_S:
        return False, ("직전 요청 %dm 전 > 억제 TTL %dm — **다시 요청한다**(영구 침묵 금지)"
                       % (age / 60, DEPT_TICKET_REQUEST_TTL_S / 60))
    return True, ("직전 요청 %ds 전(억제 TTL %ds 이내) — 중복 push 생략(CEO 큐 스팸 차단)"
                  % (age, DEPT_TICKET_REQUEST_TTL_S))


def _mark_dept_ticket_requested(dept, now=None):
    """요청 발사 흔적 기록(멱등 억제의 근거). 쓰기 실패는 부트를 죽이지 않는다 — 최악은 재요청."""
    now = time.time() if now is None else now
    try:
        _atomic_write_json(_dept_ticket_request_path(dept),
                           {"dept": dept, "requested_at": now,
                            "requested_at_iso": time.strftime("%Y-%m-%dT%H:%M:%S",
                                                              time.localtime(now)),
                            "requester": my_surface_id() or "dept-master",
                            "ttl_secs": DEPT_TICKET_REQUEST_TTL_S})
        return True, "요청 마커 기록"
    except Exception as e:
        return False, "요청 마커 기록 실패(%s: %s) — 다음 부트가 재요청할 수 있다" % (
            type(e).__name__, e)


def _request_dept_ticket(dept):
    """티켓 부재 → base CEO 에게 발급 요청 push. → (요청됨 bool, 사유).

    ★fail-open 절대: CEO 노드 부재·데몬 다운·push 실패 전부 (False, 사유) 로 **조용히가 아니라
      시끄럽게** 돌아온다. 호출자는 그래도 부트를 계속한다(단독 각성 exit 0).
    ★멱등: 같은 부트에서 이 함수는 1회만 불린다(호출 지점이 하나다). 재부팅 반복은 요청 마커
      + TTL 이 억제하고, TTL 경과 후에는 다시 요청한다.
    """
    # ★이름 유효성 **선검사**(중대③ · master 지시): 발급기가 거부할 이름이면 요청 자체를 보내지
    #   않는다. 종전 배치는 요청이 먼저 나가고 정규식 경고는 뒤에 찍혀, "실행하면 exit 2 가 나는
    #   명령"이 억제 TTL 주기로 CEO 큐에 쌓였다(수신자가 할 수 있는 일이 없는 요청 = 소음).
    if not dept_name_ok(dept):
        return False, ("부서명 %r 이 발급 규약(%s) 불일치 — 실행해도 exit 2 가 나는 명령이라 "
                       "CEO 큐에 넣지 않는다. 부서를 정규 이름으로 재생성해야 해소된다"
                       % (dept, DEPT_NAME_RE.pattern))
    suppressed, swhy = _dept_ticket_request_suppressed(dept)
    if suppressed:
        return False, swhy
    if not _base_ceo_alive():
        # ★수신자 확인 선행(계약: "CEO 노드가 없으면 fail-open"). 없는 수신자에게 던지고
        #   유계 대기까지 도는 것은 부트 시간만 태운다 — 종전 동작(단독 각성)으로 그대로 간다.
        return False, ("base 레인에 살아있는 CEO(master 좌석·agent 실재)를 확인하지 못했다 — "
                       "요청 push 를 보류하고 단독 각성으로 진행한다(fail-open)")
    code, out, err = _run_env(_dept_ticket_request_cmd(dept), _base_lane_env(),
                              timeout=DEPT_TICKET_PUSH_TIMEOUT_S)
    if code != 0:
        return False, ("요청 push 실패(exit %s) — 부트는 계속한다(fail-open): %s"
                       % (code, (err or out or "").strip()[:200]))
    _marked, mwhy = _mark_dept_ticket_requested(dept)
    return True, ("base 레인 master(CEO)에 티켓 발급 요청 push(--queued 배달 — Return 은 데몬이 "
                  "넣는다). %s" % mwhy)


def _await_dept_ticket(dept, budget_s=None, interval_s=None, sleeper=None, clock=None):
    """요청 후 티켓 도착을 **유계** 폴링. → (유효 bool, 사유, path).

    ★유계인 이유: 무기한 대기는 싱글플라이트 락 보유 연장(치명 앵커 ③)이고, 미도착이어도
      단독 각성으로 진행하는 것이 계약이다. 첫 조회는 **자지 않고** 즉시 한다(이미 도착했을 수 있다).
    ★sleeper/clock 주입: 테스트가 실시간을 소모하지 않고 예산 소진을 결정론으로 검증한다.
    """
    budget = DEPT_TICKET_WAIT_BUDGET_S if budget_s is None else budget_s
    interval = DEPT_TICKET_WAIT_INTERVAL_S if interval_s is None else interval_s
    sleeper = time.sleep if sleeper is None else sleeper
    clock = time.time if clock is None else clock
    deadline = clock() + max(0.0, budget)
    polls = 0
    while True:
        polls += 1
        ok, why, path = _peek_dept_ticket(dept)
        if ok:
            return True, "티켓 도착(폴링 %d회) — %s" % (polls, why), path
        remain = deadline - clock()
        if remain <= 0:
            return False, ("유계 대기 %ds(간격 %ds · 폴링 %d회) 안에 티켓이 오지 않았다 — %s"
                           % (budget, interval, polls, why)), path
        sleeper(min(interval, remain))


# ---------------- 위계 폴백: 2번째 마스터 선언 → 부서 자동 생성 (현장 결함 3호) ----------------
#
# 절대규칙(오너): 1번째 선언=master, 2번째 선언(다른 워크스페이스)=첫 master 는 CEO 로 승격되고
# 선언자는 새 '부서장'이 되며 팀(cso·worker·리뷰어들)이 기동돼야 한다 — N번째도 동일(부서 증식).
# 종전 현실: 같은 데몬에서의 2번째 선언은 ③정당거부(exit 7) 데드엔드였고, 위계 기계(cys-dept 의
# CEO 승격·부서 데몬·티켓)는 GUI ＋부서 버튼으로만 발동했다 — 선언 경로와 미배선.
#
# 오너 결정(2026-08-12): D1ⓐ 선언=부서 창설 동의(즉시 진행·멱등 동반) · D2 "선언 pane 이 그대로
# 부서장이 되는" 것은 PTY 소속 구조상 불가 — 새 부서에 부서장을 기동하고 선언 pane 에는 결과
# 컨텍스트를 주입한다 · D3 티켓 자동 발급(오너 타이핑 선언=발급 동의 — base 레인의 "선언=팀 기동
# 승인" 기존 재정의(role-bootstrap.sh)를 부서 레인으로 일관 확장).
#
# ★폭주 봉인(ABSOLUTE ANCHOR ①): 이 폴백에 도달하려면 이미 ⓐ훅 allowlist(master|미claim pane 만
#   발화) ⓑmachine-origin 게이트(오너 타이핑 판정 — 기계 배달 선언은 스폰 자체가 없다) ⓒ레인
#   싱글플라이트 락을 전부 통과한 상태다. 여기에 ⓓsurface별 멱등 맵(재선언=기존 부서 재사용)
#   ⓔcys-dept allocate 의 live-count 상한(CYS_DEPT_CAP 기본 8) ⓕunix 한정(Windows 는 안내만 —
#   설치파일 신중 앵커)을 얹는다. 에이전트끼리 선언 문구를 주고받아 부서가 증식하는 경로는
#   ⓑ가 원천 차단한다(배달 원장 해시·라벨 기반 — 원장 밖 동일 UID 위조는 기존 잔여위험과 동일).
#
# ★caller 게이트 우회 금지(설계 제약 실측): 데몬 claim_role 은 "발신 pane==대상 surface" 를
#   커널 peer 로 강제한다 — pane 밖 프로세스(이 폴백)는 어떤 surface 의 역할도 claim 할 수 없다.
#   그래서 폴백은 **claim-free 프리미티브만** 쓴다: cys-dept allocate(부서 데몬+CEO 승격),
#   launch-agent(role 은 surface.create 가 등록 — start_dept_master 와 동일 명령 · 구 GUI
#   ▶부서장 버튼 경로는 2026-08-20 P2로 제거, Rust 커맨드 start_dept_master 는 존치),
#   cys boot(팀 스폰 — BOOT_PLAN 에 master 없음). 티켓은 **소비하지 않고 남긴다**: 부서장의
#   in-pane 부트 체인(session-start 부트 브리지)이 유효 티켓으로 결손을 자가치유하는 인가가 된다
#   (P7 게이트 의미 보존 — G11 "티켓 소비 ⟺ 실스폰"은 그 체인이 지킨다. TTL 24h 자연 만료).

_DEPT_FB_MAP = os.path.join(STATE_DIR, "dept-fallback-map.json")


def _run_env(cmd, env, timeout=120):
    """_run 의 env 지정판 — (exit, stdout, stderr) **분리** 반환(cys-dept allocate 의
    'stdout 마지막 줄=부서명' 계약을 병합 텍스트에서 긁는 오파싱 방지)."""
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           encoding="utf-8", errors="replace", env=env)
        return r.returncode, (r.stdout or ""), (r.stderr or "")
    except FileNotFoundError:
        return 127, "", "명령 없음: %s" % cmd[0]
    except subprocess.TimeoutExpired:
        return 124, "", "timeout(%ss): %s" % (timeout, " ".join(str(c) for c in cmd))


def _dept_fb_load_map():
    try:
        with open(_DEPT_FB_MAP, encoding="utf-8") as f:
            m = json.load(f)
        return m if isinstance(m, dict) else {}
    except (OSError, ValueError):
        return {}


def _dept_lane_env(sock, pack):
    """부서 대상 명령의 env — ★G34: 소켓과 팩은 항상 쌍으로 간다(start_dept_master 와 동일 계약
    — 구 GUI ▶부서장 버튼은 2026-08-20 P2로 제거, Rust 커맨드는 존치).
    ★CYS_SURFACE_ID/REF 제거: base pane 의 표적이 부서 데몬의 동번호 surface 로 오배선되는
    교차 오염 차단(다른 데몬 = 다른 id 공간)."""
    e = dict(os.environ)
    e["CYS_SOCKET"] = sock
    e["CYS_PACK_DIR"] = pack
    e.pop("CYS_SURFACE_ID", None)
    e.pop("CYS_SURFACE_REF", None)
    # ★선행 claim 판정 위생(2026-08-16): 이 판정은 **base pane 의 것**이다 — 부서 레인 명령에
    #   딸려 들어가면 그 레인의 부트가 남의 판정을 자기 것으로 소비할 여지가 생긴다(현재 하류
    #   소비자는 없지만, env 누수는 소비자가 생기는 순간 결함이 된다 — 근원에서 끊는다).
    for _k in ("CYS_CLAIM_RC", "CYS_CLAIM_OUT", "CYS_CLAIM_SID", "CYS_CLAIM_AT"):
        e.pop(_k, None)
    return e


def _dept_master_alive(dept_env):
    """부서에 살아있는 **부서장**(agent 실재)이 있는가 — `cys status --json` 실측.

    ★P0 교정(2026-08-12 R2 확정): 종전 판정(role=master ∧ exited=false)은 allocate 가 만드는
    role=master **빈 셸**(agent 없는 로그인 셸 — cys-dept 제품 기본 정책)을 살아있는 부서장으로
    오판해 launch-agent 가 **항상** 생략됐다(신규 부서 = 부서장 영구 미기동). 판정을 agent 실재로
    올린다: agent_alive==true(관측 등록·생존) 또는 seat=="occupied"(관측 미등록이나 커널이 자손
    점유를 본 좌석 — 이중 기동 회피)만 '있음'이다. 빈 셸(agent 없음·seat empty/unknown)은
    '없음' — launch-agent 의 takeover_empty_seat 가 데몬 재판정(seat_claimable)으로 그 좌석을
    정당 승계한다(강제 아님·요청)."""
    code, out, err = _run_env(["cys", "status", "--json"], dept_env, timeout=15)
    if code != 0:
        return False
    try:
        st = json.loads(out)
    except ValueError:
        return False
    for s in (st.get("surfaces") or []) if isinstance(st, dict) else []:
        if s.get("role") == "master" and not s.get("exited"):
            if s.get("agent_alive") is True or s.get("seat") == "occupied":
                return True
    return False


def _live_master_from_status(status, exclude_sid):
    """`cys status --json` 스냅샷에 **살아있는 master 보유자**가 있는가 — 순수 판정.

    반환: (판정, 사유)  판정 = True(있음) / False(없음) / None(판독 불가 — 알 수 없음)

    ★순수 함수인 이유: 이 판정은 부서 자동 생성이라는 **비가역 스폰의 유일한 전제**다.
      I/O 와 붙여 두면 `--self-test`(밀폐 계약)로 핀할 수 없어, 판정이 조용히 낡는다.

    ★왜 별도 술어인가(_dept_master_alive 와 다른 질문): _dept_master_alive 는 "부서에 **에이전트가
      붙은** 부서장이 있는가"(agent_alive/seat)를 묻는다 — launch-agent 를 생략할지 결정하는
      기준이다. 여기서 필요한 것은 데몬 claim_role 이 **거부 판정에 쓴 그 기준**(roles["master"]
      보유자가 exited 가 아닌 다른 surface 인가 — handlers.rs `holder_live`)이다. 기준을 섞으면
      가드가 데몬과 다른 사실을 보고 서로 어긋난다(빈 셸 master 를 '없음'으로 판정 → 정당한 부서
      창설을 막는 반대 방향 결함).
    """
    if not isinstance(status, dict):
        return None, "cys status --json 판독 불가(데몬 미응답·파싱 실패)"
    surfaces = status.get("surfaces")
    if not isinstance(surfaces, list):
        return None, "cys status --json 스키마 불일치(surfaces 배열 없음)"
    for s in surfaces:
        if not isinstance(s, dict):
            continue
        if s.get("role") != "master" or s.get("exited"):
            continue
        holder = s.get("surface_id")
        if str(holder) == str(exclude_sid):
            continue  # 자기 자신은 '남의 보유'가 아니다(멱등 재claim 경로)
        return True, "살아있는 master 보유자 surface=%s" % holder
    return False, "roles 에 살아있는 master 보유자가 없다"


def _base_live_master(exclude_sid):
    """위 판정의 I/O 래퍼 — status 입구는 **기존 단일 SOT**(_cys_status_json)를 재사용한다.
    세 번째 status 리더를 만들지 않는다(예산·채널 분리 규약이 그 한 곳에 산다)."""
    return _live_master_from_status(_cys_status_json(), exclude_sid)


def _dept_fallback(log, claim_out):
    """③정당거부 → 부서 자동 생성 폴백. 반환: exit 코드(처리함) 또는 None(비적용 — 종전 exit 7 경로로).

    비적용 조건(전부 fail-closed·조용한 강등 없이 step 기록): 킬스위치 / Windows /
    비-base 레인 / cys-dept 부재. 이 함수는 cmd_run 체인 안에서만 불린다(싱글플라이트 락 보유).

    ★킬스위치: `CYS_DEPT_FALLBACK=0|off` — 현장 롤백 채널(구계약 exit 7 즉시 복원). 새 자동
    스폰 경로에는 반드시 무배포 롤백 수단을 함께 싣는다(B안 필터 킬스위치와 동일 원칙)."""
    if os.environ.get("CYS_DEPT_FALLBACK", "").lower() in ("0", "off"):
        return None  # 명시 비활성 — 구계약(정당거부 exit 7) 그대로
    if os.name == "nt":
        return None  # Windows 는 자동 스폰 금지(설치파일 신중 앵커) — 강화된 안내만
    # ★폭주 봉인 ⓑ 실강제(2026-08-12 R2 확정): 상단 계약 주석은 "이 폴백 도달 = machine-origin
    #   게이트 통과 후"를 전제하지만, 그 전제는 **훅 발화 경로에서만** 참이었다 — CLAUDE.md §0
    #   공식 폴백(에이전트가 javis_bootstrap.py 를 직접 실행)은 게이트 없이 여기 도달해, 기계
    #   배달 선언의 최악 결과가 'exit 7 데드엔드'에서 '부서+팀 자동 스폰'으로 격상됐다. 훅이
    #   human 판정 직후 스폰에만 싣는 마커(CYS_DECL_ORIGIN=hook-human)를 요구한다. 마커 없는
    #   직접 실행은 구계약(정당거부 안내)으로 — 오너 타이핑이었다면 훅 발화 재선언이 정공법이다.
    if os.environ.get("CYS_DECL_ORIGIN") != "hook-human":
        log.step(STEP.DEPT_FB, 1, "선언 유래 미보증(CYS_DECL_ORIGIN≠hook-human — 직접 실행 경로) "
                                  "— 폴백 비적용(폭주 봉인 ⓑ). 부서 창설은 오너 타이핑 선언(훅 발화)"
                                  "·GUI ＋부서·cys-dept allocate 로만.")
        return None
    if not _is_base_socket():
        return None  # 부서 레인의 master 충돌은 부서 내부 문제 — 부서 안에 부서를 만들지 않는다
    cys_dept = os.path.join(PACK, "bin", "cys-dept")
    if not os.path.isfile(cys_dept):
        log.step(STEP.DEPT_FB, 1, "cys-dept 부재(%s) — 폴백 비적용" % cys_dept)
        return None

    # ★sid 는 my_surface_id() 규약 경유(2026-08-12 R2 확정): CYS_SURFACE_ID 직접 판독은 구
    #   env(AITERM_SURFACE_ID)로만 선 pane 들을 전부 'unknown' 단일 멱등 키로 수렴시켜, 서로
    #   다른 pane 의 선언이 한 부서로 합쳐지고 결과 컨텍스트 주입(cys send --surface)이 생략됐다.
    sid = re.sub(r"[^0-9]", "", my_surface_id()) or "unknown"

    # ★L3 실측 가드(2026-08-16 현장 결함 — "없는 master 를 있다고 믿고 부서를 만든다") ────────
    # 이 폴백의 **유일한 전제**는 "살아있는 master 가 이미 있다"이다. 종전엔 그 전제를 데몬의
    # 거부 코드 하나로 추론했는데, 그 코드(claim_denied)가 신원 해석 실패까지 뭉쳐 담고 있어서
    # **보유자가 0명인 기계에서도** 폴백이 돌았다(실측: role=- 인 채 dept-N 증식). 코드 분리(L1)로
    # 그 오역 경로는 닫혔지만, 이 가드는 그것과 **독립적으로** 전제를 직접 잰다 —
    #   · 구 데몬(코드 미분리)과 신 팩이 만나는 스큐 조합에서도 부서 증식을 막고,
    #   · 앞으로 rc 7 로 접히는 **다른 경로**가 생겨도 전제 없는 스폰을 막는다.
    # 측정 실패(None)는 **폴백 비적용**이다 — 이 레포의 스폰 게이트 규율(무단 스폰이 판정 보류보다
    # 나쁘다 · role-bootstrap.sh 기계유래 게이트와 같은 방향)을 그대로 따른다.
    have, why = _base_live_master(sid)
    if have is not True:
        detail = ("부서 자동 생성 **미진입** — 폴백의 전제(살아있는 master 보유자 존재)가 실측으로 "
                  "확인되지 않았다: %s. 이 거부는 '조직 사실'이 아니라 **세션 배선 사실**일 가능성이 "
                  "높다(발신 pane 미해석 — 세션 분리·재부모화·pane 밖 실행). 진단: `cys list` 의 "
                  "role 열이 비어 있는데 claim 이 거부됐다면 신원 배선 문제다. "
                  "claim 출력:\n%s" % (why, (claim_out or "")[-800:]))
        # 단일 단계로 종결한다 — DEPT_FB(order 8) 기록 후 CLAIM_ROLE_CONTEXT(order 7)로 실패시키면
        # 단계 순서가 역행해 boot-last 에 order_violation 이 매번 남는다(계측기 자기파손).
        return log.fail(STEP.DEPT_FB_GUARD, 1, detail, EXIT_SESSION_CONTEXT,
                        ok=None, state="session_error")

    _progress("③-d 위계 폴백: 살아있는 master 존재(실측 확인) — 선언을 '부서 창설'로 해석(D1ⓐ)…")
    log.step(STEP.DEPT_FB, 0,
             "정당거부 → 부서 자동 생성 진입(선언 surface=%s · 전제 실측: %s)" % (sid, why))

    # base 셸 env: cys-dept 는 base 레지스트리 대상 — CYS_SOCKET 제거(ceo_reinject_master 동형).
    # PATH 에 이 인터프리터 디렉토리를 선두 주입 — cys-dept 내부 python3(레지스트리 flock RMW)가
    # 번들 런타임으로 해소되게 한다(GUI 의 inject_runtime_path 와 동일 취지).
    base_env = dict(os.environ)
    base_env.pop("CYS_SOCKET", None)
    for _k in ("CYS_CLAIM_RC", "CYS_CLAIM_OUT", "CYS_CLAIM_SID", "CYS_CLAIM_AT"):
        base_env.pop(_k, None)   # 선행 claim 판정 위생(_dept_lane_env 와 같은 이유)
    base_env["PATH"] = os.path.dirname(sys.executable or "python3") + os.pathsep + base_env.get("PATH", "")

    # ⓓ멱등: 같은 surface 의 재선언은 새 부서를 만들지 않고 기존 부서를 재사용한다(살아 있을 때).
    fb_map = _dept_fb_load_map()
    name = None
    prev = fb_map.get(sid)
    if isinstance(prev, dict) and prev.get("dept"):
        code, out, err = _run_env(["bash", cys_dept, "sock", prev["dept"]], base_env, timeout=30)
        if code == 0 and out.strip():
            probe_env = _dept_lane_env(out.strip().splitlines()[-1], prev.get("pack", ""))
            pc, _o, _e = _run_env(["cys", "ping"], probe_env, timeout=10)
            if pc == 0:
                name = prev["dept"]
                log.step(STEP.DEPT_FB_ALLOC, 0, "멱등 재사용: 기존 부서 %s 생존 — 신규 생성 생략" % name)

    if name is None:
        code, out, err = _run_env(["bash", cys_dept, "allocate"], base_env,
                                  timeout=_budget_leaf("CYS_DEPT_FB_ALLOC_S", 240))
        lines = [ln.strip() for ln in out.splitlines() if ln.strip()]
        name = lines[-1] if lines else ""
        log.step(STEP.DEPT_FB_ALLOC, code, "allocate → %r\n%s%s" % (name, out[-1500:], err[-1500:]))
        if code != 0 or not dept_name_ok(name):
            # ★ok=None(CS-2⑩ 동형 — 2026-08-12 R2 확정): 폴백 실패는 'base 부트가 깨졌다'가
            #   아니다(base master 는 건강해서 정당거부가 난 것). 기본값(ok=False·state=failed)로
            #   공유 boot-last 를 덮으면 다음 세션의 §0 이 '최신 완주 런 실패'로 읽고 재부트를
            #   churn 한다. 성공 경로의 ok=None 과 대칭으로 실패도 귀속 상태만 남긴다.
            return log.fail(STEP.DEPT_FB_ALLOC, code or 1,
                            "부서 생성 실패(allocate exit %s · name=%r). 상한(CYS_DEPT_CAP 기본 8) 도달"
                            " 여부·레지스트리(~/.cys/depts.json)를 확인하라.\n%s%s" % (code, name, out[-800:], err[-800:]),
                            EXIT_BOOT, ok=None, state="dept_fallback_failed")

    # 부서 (소켓, 팩) 쌍 — cys-dept 가 SOT(`<name> -- <cmd>` env 주입 경로)다. 중복 유도 금지.
    code, out, err = _run_env(
        ["bash", cys_dept, name, "--", "sh", "-c", 'printf "%s\\n%s\\n" "$CYS_SOCKET" "$CYS_PACK_DIR"'],
        base_env, timeout=30)
    pair = [ln for ln in out.splitlines() if ln.strip()]
    if code != 0 or len(pair) < 2:
        return log.fail(STEP.DEPT_FB_ALLOC, code or 1,
                        "부서 소켓/팩 쌍 유도 실패(%s)\n%s%s" % (name, out[-500:], err[-500:]),
                        EXIT_BOOT, ok=None, state="dept_fallback_failed")
    sock, pack = pair[0].strip(), pair[1].strip()
    dept_env = _dept_lane_env(sock, pack)

    # 멱등 맵 영속(부분 실패 후 재선언도 같은 부서로 수렴하게 — 생성 직후 기록).
    fb_map[sid] = {"dept": name, "sock": sock, "pack": pack, "at": time.time(),
                   "at_iso": time.strftime("%Y-%m-%dT%H:%M:%S")}
    _atomic_write_json(_DEPT_FB_MAP, fb_map)

    # D3: CEO 티켓 자동 발급(오너 타이핑 선언=발급 동의 — cmd_issue_ticket 와 동일 스키마·경로).
    tpath = _ticket_path(name)
    if not _peek_dept_ticket(name)[0]:
        now = time.time()
        _atomic_write_json(tpath, {
            "dept": name, "issued_at": now,
            "issued_at_iso": time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(now)),
            "issuer": "dept-fallback(surface %s)" % sid})
        log.step(STEP.DEPT_FB_TICKET, 0, "티켓 발급(D3): %s" % tpath)
    else:
        log.step(STEP.DEPT_FB_TICKET, 0, "유효 티켓 기존재 — 재발급 생략: %s" % tpath)

    # 부서 팩 준비 대기 — 부서 데몬 부팅 자동설치가 pack-dept-<name> 을 채운다(G34: 팩 없이
    # launch-agent 를 쏘면 본부 팩 교차 서빙·레인 가드 exit 8 계열 결함 재현).
    dept_boot_py = os.path.join(pack, "bin", "javis_bootstrap.py")
    deadline = time.time() + _budget_leaf("CYS_DEPT_FB_PACK_S", 90)
    while time.time() < deadline and not os.path.isfile(dept_boot_py):
        time.sleep(1)
    if not os.path.isfile(dept_boot_py):
        return log.fail(STEP.DEPT_FB_MASTER, 1,
                        "부서 팩 미준비(%s — 데몬 자동설치 대기 초과). 부서 데몬 기동 상태를 확인하라."
                        % dept_boot_py, EXIT_BOOT, ok=None, state="dept_fallback_failed")

    # D2: 부서장 기동 — start_dept_master(구 GUI ▶부서장 버튼 경로 — 버튼은 2026-08-20 P2로
    #     제거·Rust 커맨드는 존치)와 동일 명령. 이미 살아 있으면 생략(멱등).
    sref = None
    if _dept_master_alive(dept_env):
        log.step(STEP.DEPT_FB_MASTER, 0, "부서장 생존 — 기동 생략(멱등)")
    else:
        code, out, err = _run_env(["cys", "launch-agent", "--role", "master", "--agent", "claude"],
                                  dept_env, timeout=_budget_leaf("CYS_DEPT_FB_MASTER_S", 420))
        m = re.search(r"surface:\d+", out + err)
        sref = m.group(0) if m else None
        log.step(STEP.DEPT_FB_MASTER, code, "launch-agent master → %s\n%s%s"
                 % (sref, out[-1200:], err[-1200:]))
        # ★(U-11) 보류는 '기동 실패' 가 아니다 — pane 도 프로세스도 살아 있고, 사람이 관문을
        #   1회 통과시키면 그 좌석을 그대로 쓴다. 처방이 갈리므로 문안도 갈라야 한다(종전 문안은
        #   "claude CLI 설치를 확인하라" 였다 — 설치는 멀쩡한데 엉뚱한 곳을 보게 만든다).
        #   판정은 여전히 실패다: 부서장이 아직 지휘할 수 없기 때문이다(거짓 성공 금지).
        if code == CYS_LAUNCH_EXIT_GATE_PENDING:
            return log.fail(STEP.DEPT_FB_MASTER, code,
                            "부서장 pane 은 떴고 프로세스도 살아 있으나 **첫기동 관문**에 갇혀 있다"
                            "(%s · 좌석은 닫지 않았다). 그 pane 에서 관문을 1회 통과시킨 뒤 재시도하라 —"
                            " ★면책 창의 기본 포커스는 `No, exit` 이므로 그대로 Return 하면 노드가"
                            " 종료된다(아래 방향키 1회 뒤 Return).\n%s%s"
                            % (name, out[-800:], err[-800:]),
                            EXIT_BOOT, ok=None, state="dept_fallback_gate_pending")
        if code != 0:
            return log.fail(STEP.DEPT_FB_MASTER, code,
                            "부서장 기동 실패(%s). claude CLI 설치·부서 데몬 상태를 확인하라.\n%s%s"
                            % (name, out[-800:], err[-800:]),
                            EXIT_BOOT, ok=None, state="dept_fallback_failed")

    # 팀 결정론 스폰 — cys boot(BOOT_PLAN: cso·worker 의무 + 리뷰어들. master 는 스폰 대상 아님).
    # busy(75)=다른 boot 가 진행 중 — 실패 아님(그 boot 가 팀을 세운다).
    code, out, err = _run_env(["cys", "boot", "--json"], dept_env,
                              timeout=_budget_leaf("CYS_DEPT_FB_TEAM_S", 600))
    boot_exit = code
    log.step(STEP.DEPT_FB_TEAM, 0 if code in (0, 75) else code,
             "cys boot exit=%s%s\n%s%s" % (code, " (busy — 진행 중 boot 존중)" if code == 75 else "",
                                           out[-1500:], err[-1500:]))
    # ④-b 동형: 리뷰어 슬롯 재확인(Degrade 정책 — 비0 이어도 계속).
    orch = os.path.join(pack, "bin", "javis_orchestra.py")
    if os.path.isfile(orch):
        code, out, err = _run_env([sys.executable, orch, "boot-reviewers"], dept_env,
                                  timeout=_budget_leaf("CYS_DEPT_FB_REV_S", 300))
        log.step(STEP.DEPT_FB_TEAM, 0 if code == 0 else code,
                 "boot-reviewers exit=%s(Degrade 계속)\n%s%s" % (code, out[-800:], err[-800:]))
    # ⑤ 동형 1패스: 생존 판정은 보고용(바운디드 — 재시도 루프는 부서장 in-pane 체인 소관).
    check_exit = None
    if os.path.isfile(orch):
        code, out, err = _run_env([sys.executable, orch, "check"], dept_env,
                                  timeout=_budget_leaf("CYS_DEPT_FB_CHECK_S", 180))
        check_exit = code
        log.step(STEP.DEPT_FB_CHECK, 0 if code == 0 else code,
                 "orchestra check exit=%s(보고용 1패스)\n%s%s" % (code, out[-800:], err[-800:]))

    # D2: 선언 pane 컨텍스트 주입(queued — 조용 시점 배달) + 승인 Feed loud 알림.
    note = ("[부서 자동 생성 — 위계 폴백] 이 조직(base)에는 살아있는 master(CEO)가 있어, 절대규칙에 따라 "
            "새 부서 %(d)s 를 생성하고 부서장(claude)을 기동했습니다%(m)s. 팀 스폰 exit=%(b)s · "
            "생존 판정 exit=%(c)s(0=전원 생존·부서장 체인이 결손을 자가치유). 첫 부서 생성 시 기존 "
            "master 는 CEO 규약으로 자동 승격됩니다(cys-dept). 이 pane 은 역할 없는 일반 세션으로 "
            "유지됩니다 — 부서장 대화는 부서 워크스페이스 pane(GUI 탭이 없으면 ＋부서 버튼의 부서 "
            "선택에서 %(d)s 를 여세요) 또는 `CYS_SOCKET=%(s)s cys send --to master` 를 쓰세요."
            % {"d": name, "m": (" (%s)" % sref if sref else ""), "b": boot_exit,
               "c": check_exit, "s": sock})
    channel = _notify_loud("부서 자동 생성: %s (마스터 선언 폴백)" % name, note)
    sent = "미시도(선언 surface 미상)"
    if sid != "unknown":
        code, _o, _e = _run_env(["cys", "send", "--queued", "--surface", sid, note],
                                dict(os.environ), timeout=20)
        sent = "queued 주입 exit=%s" % code
    log.step(STEP.DEPT_FB_NOTIFY, 0, "feed=%s · 선언 pane=%s" % (channel, sent))

    summary = {"ok": True, "state": "dept_fallback", "dept": name, "dept_socket": sock,
               "dept_pack": pack, "master": sref or "reused", "boot_exit": boot_exit,
               "check_exit": check_exit, "ticket": tpath,
               "steps": [(s["step"], s["exit"]) for s in log.data["steps"]],
               "lane": log.lane, "boot_last": log.path}
    # ★ok=None(CS-2⑩ 동형): 이 런은 base 팀을 부트한 것도, base 부트가 깨진 것도 아니다 —
    #   공유 boot-last 의 건강 기록을 어느 방향으로도 덮지 않는다. 사실은 state 가 말한다.
    log.result(ok=None, state="dept_fallback", dept=name, exit=EXIT_OK)
    print(json.dumps(summary, ensure_ascii=False))
    return EXIT_OK


# ── 증분2 ⓑ: 결손 기준 자원 사전 게이트 ──
# 의무 구성(cso 1·worker 1·리뷰어계열 2 — grok 선택 제외). ★총수 비교가 아니라 역할별 구성 판정:
# 총수 4 비교는 reviewer 4개 생존+cso/worker 사망을 결손 0으로 오판했다(R1-MED-1).
# ★★이 가족 접두 계수는 이제 **폴백 전용**이다 — 1차 경로는 아래 `_team_roster_deficit`
# (javis_orchestra.effective_required_roles 소비). 근거·경계는 그 함수 주석 참조(T-0147-7 W0 지혈).
_REQUIRED_COMPOSITION = (("cso", 1), ("worker", 1), ("reviewer", 2))


def _team_composition_deficit(counts):
    """순수 판정(**폴백 전용** — orchestra 소비 불가 시): 역할별 가족 카운트 dict → (결손 bool, 사유).
    cso≥1 ∧ worker≥1 ∧ reviewer계열≥2 **전부 충족** 시에만 결손 0(R1-MED-1 — 구 총수 4 비교 폐기).
    ★이 판정의 알려진 한계가 G26이다: 'reviewer계열 2'는 이름공간을 뭉개서 reviewer-grok(선택)·
    reviewer-claude-*(대체) 좌석이 의무 슬롯을 대신 채운 것으로 계상하고, cso-N 변형이 정확일치
    'cso'를 요구하는 ⑤check를 만족시킨 것으로 계상한다. 그래서 1차 경로에서 밀어냈다."""
    missing = ["%s %d<%d" % (role, counts.get(role, 0), need)
               for role, need in _REQUIRED_COMPOSITION if counts.get(role, 0) < need]
    if missing:
        return True, "구성 결손(%s) — 결손 존재" % ", ".join(missing)
    return False, "구성 충족(cso=%d worker=%d reviewer=%d) — 결손 0(재선언·전 구성 생존)" % (
        counts.get("cso", 0), counts.get("worker", 0), counts.get("reviewer", 0))


def _required_roles_from_orchestra():
    """⑤check가 실제로 요구하는 **유효 의무 역할 목록**을 orchestra 단일 소스에서 가져온다.
    = `javis_orchestra.effective_required_roles()` (cso·worker + 감지 폴백 적용 리뷰어 로스터).
    ★형제 모듈 직접 import — cross-import 선례는 javis_orchestra.py:214(`import javis_boot_node`).
      서브프로세스를 쓰지 않는 이유는 종전과 같다: ④-b(boot-reviewers)→⑤(check) orchestra 호출
      순서·검증 계약에 deficit용 별도 호출이 끼면 안 된다.
    ★graceful 폴백: 부서 팩 결손·팩 스큐로 import/호출이 실패하면 (None, 사유)를 돌려 호출부가
      구 가족 접두 계수로 되돌아간다 — 지혈이 새 crash 지점이 되면 안 된다.
    반환: (roles list, None) | (None, 실패사유)."""
    try:
        import javis_orchestra as _orch
        roles = list(_orch.effective_required_roles())
    except Exception as e:
        return None, "orchestra 소비 불가(%s: %s)" % (type(e).__name__, e)
    if not roles or not all(isinstance(r, str) and r for r in roles):
        return None, "orchestra effective_required_roles 반환 이상(%r)" % (roles,)
    return roles, None


def _role_satisfied(role, live):
    """순수 판정: required role 하나가 라이브 role 집합으로 충족되는가.
    ★수용 규약은 `javis_orchestra.cmd_check`와 **동일**하게 유지한다(orchestra.py:239-241):
      worker만 접두 수용(worker/worker-N — 데몬이 둘째 워커부터 worker-N으로 dedup), 그 밖은
      정확일치. cso-N을 'cso'로 받거나 reviewer-*를 뭉개는 관용은 check에 없으므로 여기에도 없다."""
    if role in live:
        return True
    if role == "worker":
        return any(r == "worker" or r.startswith("worker-") for r in live)
    return False


def _team_roster_deficit(required, live):
    """순수 판정: (⑤check의 유효 의무 역할 목록, 라이브 role 집합) → (결손 bool, 사유).

    ★W0 P0 지혈(T-0147-7 재감사 G26 + A1 '결손 0 오판' 절반). 고치는 것은 **역할 이름공간 정합**
    하나다: 결손 판정이 세는 좌석 집합을 ⑤check가 요구하는 좌석 집합과 같게 만든다. 종전 가족
    접두 계수는 reviewer-grok(선택 좌석)·reviewer-claude-*(대체 좌석)·cso-N(변형 좌석)을 의무
    슬롯 충족으로 계상해 **결손 0 → ④ boot 생략 → ⑤check 실패 → exit 6 → 재선언 동일**의
    라이브락을 먹였다.
    ★고치지 않는 것(경계 명시 — 이걸 여기서 건드리면 반대 방향 회귀다): 생존 **술어**는 여전히
      cys list의 `role=` + `!exited`다. 이것은 check의 술어(agent_alive ∨ 신선 set-status ∨
      quiet_but_alive)보다 **관대**하므로, 판정 방향은 '결손>0 ⟹ check도 부재'로만 흐른다
      (좌석이 아예 없을 때만 결손). 건강한 quiet 노드를 결손>0으로 오판해 재선언 오탐
      hard-block을 되살리는 역방향 회귀가 구조적으로 불가능하다. 술어 격상(agent 죽은 좌석의
      결손 인식)은 W2 소속 — 데몬 SOT·reclaim 체인과 원자로 착지해야 한다."""
    missing = [r for r in required if not _role_satisfied(r, live)]
    if missing:
        return True, "로스터 결손(의무 %s / 부재 %s / 라이브 %s) — 결손 존재" % (
            ", ".join(required), ", ".join(missing), ", ".join(sorted(live)) or "없음")
    return False, "로스터 충족(의무 %s 전원 좌석 생존) — 결손 0(재선언)" % ", ".join(required)


def _cys_status_json():
    """`cys status --json` 파싱 결과(실패=None). 결손 판정이 check 와 **같은 신호**를 보게 하는 입구.
    ★cys list 텍스트에는 agent_alive/seat/awakened_at 필드가 아예 없다(cys.rs 포맷) — 그래서 W0
      단계의 결손 판정은 check 와 다른 신호를 쓸 수밖에 없었다. W2 는 그 원천을 status 로 올린다."""
    code, out = _run(["cys", "status", "--json"], timeout=_budget_leaf("CYS_STATUS_TIMEOUT_S", 12))
    if code != 0:
        return None
    try:
        return json.loads(out.strip())
    except (ValueError, TypeError):
        return None


def _shared_verdict_deficit(status, requery=None, tick_s=None, detect=None, agents=None):
    """★부트 ④ 결손 산출 — `javis_orchestra._shared_verdict_deficit`(정본) **위임 소비**(W-B3 배선).

    정본(orchestra 판)은 check_verdicts(⑤check 판정 코어) 소비에 더해 **unknown 등급을
    시한부 해소 후 잔존 시 결손**으로 계상한다(`javis_boot_node.resolve_unknown_for_spawn`
    — `cys boot` 스폰 경로가 이미 쓰는 규약: 워치독 1주기 대기 → 재조회 1회·전 역할 공유 1회).
    죽었는데 프로브만 실패한 좌석이 '충족'으로 접혀 ④ boot 가 영영 생략되는 잔여 B3 를
    닫는다. 중복 스폰은 boot 락 + `cys boot` 자체의 Unknown 시한부 해소 +
    seat_death_confirmed 죽음확정 게이트가 방어한다.

    ★역방향 회귀 차단(경계 갱신 — W-B3): agent_alive 단독·좌석 점유·quiet_but_alive 는 종전대로
      충족측이고, **unknown 만** 시한부 해소(생존 확인 시 충족 복귀) 후 잔존할 때 결손이다 —
      건강한 quiet 노드를 결손>0 으로 오판하는 경로는 여전히 없다(W0 handoff 경계 유지).
    ★⑤check 의 satisfied 는 불변이다(unknown=충족측 fail-open 유지 — 결손 산출만 갈린다).
      같은 함수에 넣으면 데몬 콜드스타트 창에서 ⑤check 실패 → exit 6 라이브락(감사 확정).
      불변 핀: tests/test_seat_latch_negation.py BootDeficitUnknown · H-PRED-1 신계약 절.
    ★스큐 폴백: orchestra import 실패·구 팩(함수 부재)·위임 호출 예외 시 종전 로컬 산출
      (`_shared_verdict_deficit_fallback` — unknown=충족측 구 계약)로 되돌아간다. 신팩
      bootstrap + 구팩 orchestra 혼재에서 부트가 죽으면 안 된다. 폴백 발동은 stderr 1줄
      고지(조용한 강등 금지 — 어느 계약으로 판정했는지가 진단의 절반이다).
    requery/tick_s 는 밀폐 테스트 주입(기본: cys status 재조회·워치독 1주기) — 정본에 그대로
    전달된다. 반환 계약 (결손 bool, 사유) | (None, 실패사유)는 정본·폴백 동일(drop-in —
    양판 반환문 직접 대조로 확인 2026-08-21).
    ★detect/agents(2026-08-22 적대검증 중대④): 리뷰어 로스터 밀폐 주입을 정본까지 전달한다.
      막혀 있던 탓에 **깨끗한 기계(agy·codex 미설치 = 신규 사용자 대다수)에서 이 파일의
      `--self-test` 가 항상 exit 1** 이었다. 미주입=None 이면 실감지 — 프로덕션 거동 불변.
      ★구 팩 스큐 방어: 정본이 이 키워드를 모르는 구버전이면 `TypeError` 가 난다. 그때는
      **주입 없이 1회 재시도**한 뒤 그래도 실패해야 폴백으로 내려간다(신팩 bootstrap + 구팩
      orchestra 혼재에서 부트가 죽으면 안 된다는 기존 계약 유지)."""
    try:
        import javis_orchestra as _orch
        _fn = getattr(_orch, "_shared_verdict_deficit", None)
        if _fn is not None:
            try:
                return _fn(status, requery=requery, tick_s=tick_s,
                           detect=detect, agents=agents)
            except TypeError:
                if detect is None and agents is None:
                    raise
                return _fn(status, requery=requery, tick_s=tick_s)
        skew_why = "구 팩 스큐(javis_orchestra 에 _shared_verdict_deficit 부재)"
    except Exception as e:
        skew_why = "orchestra 위임 불가(%s: %s)" % (type(e).__name__, e)
    print("[bootstrap] 결손 산출 폴백 발동: %s — 로컬 구 계약(unknown=충족측) 시도" % skew_why,
          file=sys.stderr)
    return _shared_verdict_deficit_fallback(status)


def _shared_verdict_deficit_fallback(status):
    """★폴백 전용(정본 아님) — 구 팩 스큐에서만 호출되는 종전 W2 로컬 산출.

    orchestra 에 부트 경로 산출기(`_shared_verdict_deficit`)가 없는 구 팩과 혼재할 때만
    위 위임 래퍼가 여기로 강등한다(동명 쌍둥이 드리프트 함정 제거 — W-B3). 구 계약
    그대로: check_verdicts 소비 · **unknown=충족측**(시한부 해소 없음 — 잔여 B3 는 이
    폴백에선 열려 있고, 그래서 폴백 발동이 stderr 로 고지된다). 정본 경로가 살아 있으면
    절대 호출되지 않는다. 반환 (결손 bool, 사유) | (None, 실패사유) — 래퍼와 동일 계약."""
    try:
        import javis_orchestra as _orch
        verdicts, _roster = _orch.check_verdicts(status)
    except Exception as e:
        return None, "check_verdicts 소비 불가(%s: %s)" % (type(e).__name__, e)
    if not verdicts:
        return None, "check_verdicts 빈 판정(로스터 산출 실패)"
    missing = [r for r, v in verdicts.items() if not v.get("satisfied")]
    presumed = [r for r, v in verdicts.items()
                if v.get("satisfied") and v.get("grade") == "alive_presumed"]
    if missing:
        return True, ("공유 판정 결손(의무 %s / 부재 %s) — 결손 존재 [신호=check_verdicts 동일·폴백]"
                      % (", ".join(verdicts), ", ".join(missing)))
    note = ("" if not presumed
            else " · 생존추정(각성 미확인) %s — 재각성 권장이나 결손 아님" % ", ".join(presumed))
    return False, ("공유 판정 충족(의무 %s 전원) — 결손 0(재선언)%s [신호=check_verdicts 동일·폴백]"
                   % (", ".join(verdicts), note))


def _team_has_deficit():
    """팀 결손 여부 산출 → (결손 bool, 사유). 신호 원천 실패 → 보수적으로 결손 가정(게이트 진행).

    ★W2 술어 단일화(A1 클래스·CS-1②) + W-B3 위임: 1차 경로는 orchestra 정본
      `_shared_verdict_deficit` — ⑤check 판정 코어(check_verdicts)를 소비하고 unknown 은
      시한부 해소 후 잔존 시 결손이다(⑤ satisfied 는 불변 — 위 래퍼 docstring).
      2차는 W0 의 로스터 판정(cys list `role=`+`!exited` — 이름공간만 정합),
      3차는 구 가족 접두 계수(G26 한계 포함). 강등할 때마다 **사유를 사유 문자열에 남긴다**
      (조용한 접힘 금지 — 어떤 신호로 판정했는지가 진단의 절반이다).
    ★orchestra check **서브프로세스**는 여전히 쓰지 않는다: ④-b→⑤ 호출 순서·검증 계약에
      deficit 용 별도 호출이 끼면 안 된다. 소비는 in-process import 다(W0 선례와 동일)."""
    status = _cys_status_json()
    if status is not None:
        has, why = _shared_verdict_deficit(status)
        if has is not None:
            return has, why
        why_shared = why
    else:
        why_shared = "cys status --json 실패/파싱 불가"
    roles = _live_role_names()
    if roles is None:
        return True, "cys status·cys list 모두 실패(%s) — 결손 가정(게이트 진행·보수)" % why_shared
    required, why_no_orch = _required_roles_from_orchestra()
    if required is not None:
        has, why = _team_roster_deficit(required, set(roles))
        return has, "%s [강등: %s — W0 로스터 판정 사용]" % (why, why_shared)
    # graceful 폴백 — 구 가족 접두 계수(G26 한계 포함). 조용히 접히지 않게 사유를 사유 문자열에 남긴다.
    has, why = _team_composition_deficit(_family_counts(roles))
    return has, "%s [폴백: %s / %s — 구 가족 접두 계수 사용]" % (why, why_shared, why_no_orch)


def _live_role_names():
    """cys list 로 라이브(미exited) 노드의 role 문자열 목록 산출(빈 role 제외).
    파싱 불가/데몬 부재 → None(호출부 보수 판정)."""
    code, out = _run(["cys", "list"], timeout=15)
    if code != 0:
        return None
    roles = []
    for line in out.splitlines():
        f = line.rstrip("\n").split("\t")
        if len(f) < 4:
            continue
        role = f[1][5:] if f[1].startswith("role=") else ""
        if f[3].strip().endswith("true"):  # exited surface 무시
            continue
        if role:
            roles.append(role)
    return roles


def _family_counts(roles):
    """순수 함수(**폴백 전용**): 라이브 role 목록 → 가족 접두 카운트 dict.
    변형 역할(cso-1·worker-2·reviewer-*)을 접두로 귀속한다 — 이 관용이 G26의 원인이라
    1차 경로에서는 쓰지 않는다."""
    counts = {"cso": 0, "worker": 0, "reviewer": 0}
    for role in roles:
        if role == "cso" or role.startswith("cso-"):
            counts["cso"] += 1
        elif role == "worker" or role.startswith("worker-"):
            counts["worker"] += 1
        elif role.startswith("reviewer"):
            counts["reviewer"] += 1
    return counts


def _live_role_counts():
    """구 판정 호환 래퍼(폴백·self-test 경로) — cys list 라이브 노드의 가족 접두 카운트.
    파싱 불가/데몬 부재 → None."""
    roles = _live_role_names()
    return None if roles is None else _family_counts(roles)


def _live_node_count():
    """cys list 로 라이브(미exited) 노드 role surface 수 산출(ps 과계수 결함 교차확인용).
    파싱 불가/데몬 부재 → None(호출부는 교차확인 불가 시 genuine hard-block로 보수 판정)."""
    code, out = _run(["cys", "list"], timeout=15)
    if code != 0:
        return None
    n = 0
    for line in out.splitlines():
        f = line.rstrip("\n").split("\t")
        if len(f) < 4:
            continue
        role = f[1][5:] if f[1].startswith("role=") else ""
        if f[3].strip().endswith("true"):  # exited surface 무시
            continue
        if role in ("cso", "worker") or role.startswith("worker-") or role.startswith("reviewer"):
            n += 1
    return n


def _resource_gate_decision(gate_exit, gate_json, live_node_count):
    """순수 판정: 자원 게이트 exit·json·라이브 노드 수 → (verdict, 사유).
    verdict ∈ allow|soft|hard-overcount|hard-block|usage-error|unknown-exit.
      exit 0=allow · 1=soft · 2=hard(단, nodes-only hard이고 라이브 노드<유효임계면 과계수→overcount로 무효화).

    ★A13(W2 · 하드 제약 8) — 미지 exit 의 fail-open 제거:
      종전 반환은 **모든** 미지 exit 를 'allow(보수적 진행)'로 접었다. 그 결과 게이트가
      **사용오류로 아무 측정도 못 한 상태**(argparse 가 exit 2 를 내던 구 코드에서는 EXIT_HARD 와
      충돌해 hard 로 오독되기도 했다)를 '자원 여유 있음'과 동일하게 처리했다 — 판정과 판정불가의
      융합(RC2)이다. 이제:
        64(EX_USAGE) = 사용오류    → **명시 fail + loud**(조용한 allow 금지). 부트는 계속하되
                                    '측정 실패'를 시끄럽게 남긴다(게이트가 없는 것과 같음을 고지).
        그 밖 미지 exit           = 'unknown-exit' 라벨로 진행 + loud(과거의 조용한 allow 와 구분).
      ★부트 차단(hard-block)으로 승격하지 않는 이유: 게이트 자체가 못 재는 상태에서 팀 기동을
      거부하면 자원 무관 사유로 조직이 영구 정지한다(fail-closed 의 오적용). 방향은 'loud 진행'이다."""
    if gate_exit == 0:
        return "allow", "자원 게이트 allow"
    if gate_exit == 1:
        return "soft", "자원 게이트 soft_warn"
    if gate_exit == 64:
        return "usage-error", (
            "자원 게이트 **사용오류**(exit 64=EX_USAGE) — 측정이 아예 일어나지 않았다. "
            "게이트 없음과 동등하므로 자원 판정 없이 진행한다(조용한 allow 아님). "
            "원인: 미지 서브커맨드·인자 오류·게이트 버전 스큐. 호출 인자를 점검하라.")
    if gate_exit == 2:
        trips = (gate_json or {}).get("trips") or []
        hard = [t for t in trips if t.get("level") == "hard"]
        nodes_hard = [t for t in hard if t.get("metric") == "nodes"]
        other_hard = [t for t in hard if t.get("metric") != "nodes"]
        if nodes_hard and not other_hard and isinstance(live_node_count, (int, float)):
            eff = ((gate_json or {}).get("measured") or {}).get("nodes_hard_effective")
            if isinstance(eff, (int, float)) and live_node_count < eff:
                return "hard-overcount", (
                    "nodes hard(ps=%s)이나 라이브 노드 %d < 유효임계 %s — ps 과계수 결함으로 판단, "
                    "1회 경고 후 진행" % (nodes_hard[0].get("value"), live_node_count, eff))
        detail = ", ".join("%s=%s" % (t.get("metric"), t.get("value")) for t in hard) or "미상"
        return "hard-block", "자원 hard_block(트립: %s) — 팀 기동 거부" % detail
    return "unknown-exit", (
        "자원 게이트 미지 exit %s(내부오류) — **측정 신뢰 불가**. 자원 판정 없이 진행하되 "
        "조용히 allow 로 접지 않는다(A13: 판정불가와 allow 의 융합 제거). 게이트 로그를 확인하라."
        % gate_exit)


def _is_unknown_arg_error(out):
    """순수 판정: 출력이 **'미지 인자' 사용오류**인가(구 바이너리 스큐 신호).

    clap(rust CLI)·getopt 계열의 관용 문구를 본다. ★보수적으로 좁게 본다 — 넓히면 진짜 부트 실패를
    '스큐'로 오독해 재시도 루프를 만든다(폭주 방향). 여기서 찾는 것은 **인자 파싱 실패**뿐이다."""
    low = (out or "").lower()
    return any(m in low for m in (
        "unexpected argument", "unrecognized option", "unknown option",
        "found argument", "invalid option", "unexpected value",
    ))


def _parse_boot_json(out):
    """`cys boot --json` 출력에서 JSON 오브젝트를 뽑는다(진행 산문과 섞여 나올 수 있다).
    ★stdout 마지막 '{'…'}' 블록만 취한다 — 산문 로그와 기계 계약의 공존 규약(A7 채널 분리 정신).
    반환 dict | None(구 바이너리·파싱 불가)."""
    if not out:
        return None
    s = out.strip()
    start = s.rfind("\n{")
    cand = s[start + 1:] if start >= 0 else (s if s.startswith("{") else None)
    if not cand:
        return None
    try:
        v = json.loads(cand)
    except (ValueError, TypeError):
        return None
    return v if isinstance(v, dict) else None


def _boot_fatal_verdict(code, out):
    """★B1 정책 열 소비 — `cys boot` 결과가 **Fatal 실패**인가. Fatal 이면 사유 문자열, 아니면 None.

    판정 재료는 `cys boot --json` 의 role 별 `{outcome, mandatory}`:
      Fatal 실패 = mandatory:true 인 role 의 outcome ∈ `BOOT_FATAL_OUTCOMES`(= failed · missing).
      busy(다른 boot 진행 중)·already_alive·launched 는 실패가 아니다(G11 — busy 를 성공으로
      오인하지도, 실패로 오인하지도 않는다).
      **gate_pending 도 Fatal 이 아니다**(U-11 · 위 `BOOT_FATAL_OUTCOMES` 주석) — 제3 분기
      `_boot_gate_pending_verdict` 가 잡는다. 그쪽이 더 **구체적인** 판정이라 여기서 빠지는 것은
      감시의 축소가 아니라 이관이다.
    ★--json 소비 불가(구 바이너리·파싱 실패)면 **종전 계약으로 보수 폴백**: 비0 = Fatal.
      새 계약을 못 읽는 상태에서 Degrade 로 접으면 진짜 실패를 은닉한다(fail-open 금지).
    ★(W4) 단 **exit 75(busy)** 는 그 보수 폴백에서 제외한다 — busy 는 실패가 아니라 '무스폰'이고,
      75 는 신 계약 전용 값이라(구 바이너리는 0/1/2만) 오해석 위험이 없다. 이걸 Fatal 로 접으면
      훅↔GUI 중첩 부트마다 exit 4 위경보가 난다(P3-B16 부류의 반복성 오경보).
    ★(M3-짝) **exit 78(관문 보류)** 도 같은 이유로 제외한다 — 다만 근거가 다르다. 75 는 '실패가
      아니라서' 빼고, 78 은 '**다른 분기가 반드시 잡아서**' 뺀다(`_boot_gate_pending_verdict` 의
      축 ⓐ 가 exit 78 이므로 파싱이 깨져도 놓치지 않는다). 여기서 Fatal 로 접으면 소비부가 살아
      있는 좌석에 회수·파괴 처방을 낸다(치명위험 ④). 78 역시 신 계약 전용 값이라(구 바이너리는
      0/1/2만) 파싱 불가 상태에서도 의미가 모호하지 않다."""
    v = _parse_boot_json(out)
    if v is None or not isinstance(v.get("roles"), list):
        if code in (CYS_BOOT_EXIT_BUSY, CYS_BOOT_EXIT_GATE_PENDING):
            return None
        return (None if code == 0
                else "cys boot 실패(exit %s) — --json 계약 소비 불가로 종전 계약(비0=Fatal) 적용:\n%s"
                     % (code, out))
    bad = [r for r in v["roles"]
           if r.get("mandatory") and r.get("outcome") in BOOT_FATAL_OUTCOMES]
    if not bad:
        return None
    return _fatal_detail(bad, out)


def _boot_was_busy(code, out):
    """④ `cys boot` 가 **무스폰 skip**(다른 boot 가 락 보유)이었나 — 티켓 소각 차단의 근거(G11).

    판정은 두 축의 **OR** 이다: ⓐ exit == 75(신 계약 전용 값) ⓑ --json summary.lock == "busy".
    둘 중 하나만 봐도 되게 만들지 않는 이유: exit 만 보면 --json 파싱이 성공했는데 종료코드가
    스큐된 조합(팩 신 / 바이너리 구)을 못 걸고, JSON 만 보면 파싱 실패 시 busy 를 놓친다.
    ★'busy' 는 실패가 아니다 — 다른 런이 팀을 세우는 중이고, 최종 게이트는 ⑤check 다."""
    if code == CYS_BOOT_EXIT_BUSY:
        return True
    v = _parse_boot_json(out) or {}
    summary = v.get("summary") if isinstance(v.get("summary"), dict) else {}
    return summary.get("lock") == "busy"


def _boot_gate_pending_verdict(code, out):
    """★(M3-짝) `cys boot` 결과가 **관문 보류**인가 — 보류면 사유 문자열, 아니면 None.

    ★왜 전용 분기인가(종전 결함): `gate_pending` 은 Fatal 집합에도 busy 판정에도 없어서
      **어디에도 걸리지 않았다.** 그 결과 `elif code != 0` 의 Degrade 가지로 흘러
      "비0 이지만 **Fatal 역할은 전원 확보**" 라는 문장이 기록됐다 — 의무 역할이 관문에 갇혀
      팀이 서지 않았는데 전원 확보라고 적는 것이라, 로그를 읽는 사람과 기계 모두를 속인다.
      Fatal 로 승격하는 수리는 반대편 벽에 부딪힌다(U-11: 살아 있는 좌석 회수·파괴 = 치명위험 ④).
      ∴ 성공도 실패도 busy 도 아닌 **제3 상태**로 이름 붙여 잡는다.

    판정은 두 축의 **OR** 이다(`_boot_was_busy` 와 같은 형태·같은 이유):
      ⓐ exit == 78(`CYS_BOOT_EXIT_GATE_PENDING` · 신 계약 전용 값)
      ⓑ --json 의 mandatory role 중 outcome == "gate_pending"
    한쪽만 보면 스큐를 놓친다 — exit 만 보면 종료코드가 구 바이너리로 스큐된 조합을 놓치고,
    JSON 만 보면 파싱 실패에서 보류를 통째로 놓친다.

    ★귀결은 **중단이 아니라 큰 소리 + 계속**이다. 좌석이 살아 있으므로 여기서 exit 4 를 내면
      U-11 이 막으려는 회수·파괴 처방이 나간다. 최종 게이트는 종전대로 ⑤check 이고, ⑤ 의 결손
      산출은 U-10 이 gate_pending 좌석을 이미 '못 쓰는 좌석' 으로 센다(4자 파리티) — 즉 보류
      상태에서 부트가 조용히 성공으로 끝나지 않는다."""
    v = _parse_boot_json(out) or {}
    gated = [r for r in (v.get("roles") or [])
             if isinstance(r, dict) and r.get("mandatory")
             and r.get("outcome") == BOOT_GATE_PENDING_OUTCOME]
    if not gated and code != CYS_BOOT_EXIT_GATE_PENDING:
        return None
    who = ", ".join("%s=%s%s" % (r.get("role"), r.get("outcome"),
                                 (" [" + r["reason"] + "]") if r.get("reason") else "")
                    for r in gated) or "(--json 소비 불가 — exit %s 로 판정)" % code
    return ("의무 역할 첫기동 관문 보류(exit %s): %s\n"
            "  ★좌석과 에이전트 프로세스는 **살아 있다** — 실패가 아니므로 회수·파괴하지 않는다.\n"
            "  %s\n%s" % (code, who, _GATE_PENDING_PRESCRIPTION, out))


def _fatal_detail(bad, out):
    """Fatal 사유 1줄 조립 — install_hint 는 **그대로** 인용한다(플랫폼 분기는 생산자 몫·B15).

    ★(M3-짝) 생산자가 붙여 준 처방 필드를 넓게 인용한다 — 종전엔 `install_hint` 만 봤는데,
      `cys.rs` 는 outcome 에 따라 `hint`·`reason` 도 싣는다. 그 문장들이 소비부에서 버려지면
      사람이 로그만 보고는 무엇을 해야 할지 알 수 없다(플랫폼·상황 분기는 생산자 몫·B15)."""
    # 생산자(cys.rs)가 붙여 준 처방을 **그대로** 인용한다 — 우선순위는 구체적인 것부터:
    #   install_hint(설치 처방) → hint(관문 보류 처방 · gate_pending 이 싣는다) → reason(사유).
    def _hint(r):
        return r.get("install_hint") or r.get("hint") or r.get("reason")
    detail = ", ".join("%s=%s%s" % (r.get("role"), r.get("outcome"),
                                    (" [" + _hint(r) + "]") if _hint(r) else "")
                       for r in bad)
    return "의무(Fatal) 역할 기동 실패: %s\n%s" % (detail, out)


def _run_resource_gate(py, log):
    """결손>0 확정 후의 자원 사전 게이트(호출부가 결손 0이면 이 함수를 호출하지 않는다).
    반환: None=진행 / 9=hard-block(팀 기동 0·CEO escalation)."""
    gate = os.path.join(PACK, "bin", "javis_resource_gate.py")
    if not os.path.isfile(gate):
        log.step(STEP.RESOURCE_GATE_ABSENT, 0, "결손>0이나 resource_gate 부재 — 게이트 생략(계속)")
        return None
    # ★A13 잠복 경로 차단(착수 전 재검증 산물): 종전 호출은 `_run`(stdout+stderr **병합**)의
    #   병합 텍스트를 json.loads 에 넣었다. 게이트가 stderr 를 한 줄이라도 흘리는 날(파이썬 경고·
    #   미래 진단 로그) 파싱이 깨져 `gate_json=None` 이 되고, exit 2 + json None 은
    #   nodes 과계수 무효화(hard-overcount)를 성립 불가로 만들어 **건강한 기계를 hard-block(exit 9)**
    #   시킨다. 실측으로 재현 가능한 인접 결함이므로(원 메커니즘 판정은 기각) 채널을 분리한다:
    #   **계약 채널은 stdout 뿐**이고 stderr 는 진단으로만 남긴다.
    code, gout, gerr = _run_split([py, gate, "check", "--json"],
                                  timeout=_budget_leaf("RPC_SLACK_S", 10) * 3)
    try:
        gate_json = json.loads((gout or "").strip())
    except (ValueError, TypeError):
        gate_json = None
    live = _live_node_count() if code == 2 else None
    verdict, why = _resource_gate_decision(code, gate_json, live)
    out = (gout or "") + (("\n[stderr] " + gerr.strip()) if (gerr or "").strip() else "")
    log.step(STEP.RESOURCE_GATE, code, "결손>0 · verdict=%s · %s\n%s" % (verdict, why, out))
    if verdict in ("usage-error", "unknown-exit"):
        # ★조용한 allow 금지 — 측정 실패는 시끄럽게(loud) 남기고 진행한다(fail-open 제거).
        _progress("⚠ 자원 게이트 측정 실패(%s) — 자원 판정 없이 진행: %s" % (verdict, why))
        _notify_loud("자원 게이트 측정 실패(%s)" % verdict, why)
        return None
    if verdict == "hard-block":
        _progress("✗ 자원 hard_block — 팀 기동 0·CEO escalation: " + why)
        notified = _notify_loud("자원 hard_block(부트 중단)",
                                "%s. 자원 정리(서버 kill·/clear·노드 회수) 후 재선언하라." % why)
        log.step(STEP.RESOURCE_GATE_NOTIFY, 0, "알림 채널: %s" % notified)
        log.result(ok=False, state="failed", failed_step="resource-gate",
                   exit=EXIT_RESOURCE_HARD)
        return EXIT_RESOURCE_HARD
    if verdict == "hard-overcount":
        _progress("⚠ 자원 nodes hard(과계수 결함으로 판단) — cys list 교차확인 후 1회 경고·진행: " + why)
        _notify_loud("자원 게이트 nodes 과계수 경고", why)
    elif verdict == "soft":
        # 매번 경고 후 진행(디바운스 없음) — 결손 0이면 게이트 자체를 생략하므로
        # 이 경고는 실팀기동 시에만 발생한다: 소음 아니라 신호(설계 v0.3 판정).
        _progress("⚠ 자원 soft_warn — 경고 push 후 진행: " + why)
        _notify_loud("자원 soft_warn", why)
    return None


# ── 부서명 규약 동작 대조 (2026-08-22 적대검증 2회차 중대6) ──────────────────────
# 두 구현이 갈려 있는 입력의 허용 목록 — **지금은 비어 있다(= 완전 일치)**.
# 이력: 종전 `cys-dept::dept_name_ok` 는 `grep -Eq` 라 **줄 단위**로 판정해 `$'abc\nrm -rf /'`
# 같은 개행 포함 이름을 통과시켰다(python `fullmatch` 는 거부). 2026-08-22 그 파일이 bash
# `[[ =~ ]]` 로 교체되면서 해소됐고, 아래 대조로 16종 전건 일치를 실측 확인했다.
# ★목록에 항목을 추가하는 것은 "불일치를 알고 남긴다"는 선언이다 — 반드시 사유·소유자를 함께 적어라.
_DEPT_NAME_KNOWN_DIVERGENCE = ()


def _extract_dept_name_ok_def(src):
    """`cys-dept` 원문에서 `dept_name_ok` **함수 정의 전체**를 잘라낸다. → str | None.

    ★다중 줄 정의를 견딘다(2026-08-22 master 지시): 종전은 "한 줄이고 `}` 로 끝난다"만
      인정해서, 그 파일이 여러 줄 정의로 바뀌는 순간 추출이 실패했다. 두 형태를 받는다 —
        ⓐ 한 줄:   `dept_name_ok(){ …; }`
        ⓑ 여러 줄: `dept_name_ok(){` … 단독 `}` 줄까지
    ★그래도 이 추출은 **언젠가 깨진다**(셸 문법을 전부 파싱하지는 않는다). 그래서 견고화보다
      중요한 것이 호출자의 **loud 실패**다 — 추출 실패를 조용히 넘기면 대조가 사라진다.
    """
    lines = src.splitlines(True)
    for idx, ln in enumerate(lines):
        if not ln.lstrip().startswith("dept_name_ok()"):
            continue
        if ln.rstrip().endswith("}"):
            return ln                                   # ⓐ 한 줄 정의
        buf = [ln]
        for ln2 in lines[idx + 1:]:                     # ⓑ 다중 줄 — 단독 `}` 줄까지
            buf.append(ln2)
            if ln2.strip() == "}":
                return "".join(buf)
        return None                                     # 닫히지 않았다 = 추출 실패
    return None


def _cys_dept_name_ok_batch(names):
    """`cys-dept` 의 `dept_name_ok` 를 **실제로 실행**해 판정 목록을 받는다.

    → `(판정목록, None)` 성공 / `(None, ("absent"|"drift", 사유))` 실패.

    ★사본을 만들지 않는다: 함수 정의를 파일에서 그대로 뽑아 실행하므로, 대조 대상은 **그 파일의
      실제 코드**다(정규식을 여기 옮겨 적으면 그 순간 대조가 무의미해진다).
    ★스크립트 전체를 source 하지 않는 이유: 최상위 인자 파싱·부작용이 돌 수 있다. 필요한 것은
      순수 판정 함수 하나뿐이다.

    ## ★실패를 두 갈래로 가르는 이유 (2026-08-22 master 결정 — 무성 강등 제거)
    종전은 모든 실패를 `None` 하나로 뭉쳐 호출자가 전부 NOTE 로 강등했다. 그러면 `cys-dept` 가
    다중 줄 정의로 바뀌는 순간 **부서명 규약 대조가 소리 없이 사라지고 self-test 는 초록**이다 —
    "대조가 죽었는데 초록이 뜬다"는 우리가 3라운드 내내 제거해 온 결함 부류 그 자체다.
      · `absent` = **정당한 부재**(파일 자체가 없다 · bash 가 없다). 팩 밖에서 돌리는 경우가
        있으므로 NOTE 로 강등한다. 대조할 대상이 애초에 없는 것은 드리프트가 아니다.
      · `drift`  = **파일은 있는데 못 읽었다/못 뽑았다/못 돌렸다**. 이건 부재가 아니라 신호다 —
        호출자가 **hard fail** 시킨다.
    """
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cys-dept")
    if not os.path.isfile(path):
        return None, ("absent", "cys-dept 파일이 없다(%s) — 팩 밖 실행으로 본다" % path)
    import shutil as _sh
    bash = _sh.which("bash")
    if not bash:
        # bash 부재는 드리프트가 아니라 **환경 능력 부재**다(해석기가 없으면 무엇도 못 돌린다).
        return None, ("absent", "bash 실행기가 PATH 에 없다 — 셸 구현을 실행할 수단이 없다")
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            src = f.read()
    except OSError as e:
        return None, ("drift", "cys-dept 를 읽지 못했다(%s: %s)" % (type(e).__name__, e))
    fndef = _extract_dept_name_ok_def(src)
    if not fndef:
        return None, ("drift",
                      "cys-dept 에서 `dept_name_ok` 정의를 뽑지 못했다 — 함수가 사라졌거나 "
                      "추출기가 모르는 형태로 바뀌었다(한 줄 정의·단독 `}` 로 닫는 다중 줄 정의만 "
                      "인식한다). `_extract_dept_name_ok_def` 를 그 형태에 맞춰 고쳐라")
    script = fndef + '\nfor n in "$@"; do dept_name_ok "$n" && echo Y || echo N; done\n'
    try:
        r = subprocess.run([bash, "-c", script, "_"] + list(names),
                           capture_output=True, text=True, timeout=30)
    except Exception as e:
        return None, ("drift", "추출한 dept_name_ok 실행에 실패했다(%s: %s)"
                      % (type(e).__name__, e))
    out = (r.stdout or "").split()
    if len(out) != len(names):
        return None, ("drift",
                      "판정 개수가 입력과 다르다(입력 %d · 출력 %d) — 추출한 정의가 문법 오류이거나 "
                      "함수가 여분의 출력을 낸다. stderr=%r"
                      % (len(names), len(out), (r.stderr or "")[:200]))
    return [tok == "Y" for tok in out], None


# 부서명 대조 코퍼스 — (이름, 수용되어야 하는가, 축 설명).
# ★두 축을 **동시에** 본다(둘 중 하나만으로는 무증거다):
#     ⓐ 비대칭 0 — 두 구현의 판정이 전건 일치하는가(생성기는 만들고 발급기는 거부하는 부서 0)
#     ⓑ 절대 기대값 — 그 일치가 **옳은 값**인가. 비대칭 0 만 보면 두 구현이 **함께 틀린** 경우
#        (예: 양쪽 다 `a;b` 를 수용)가 만점을 받는다. 이름은 경로·소켓 이름에 들어가므로
#        "둘이 사이좋게 위험한" 상태를 green 으로 넘기면 안 된다.
_DEPT_NAME_CORPUS = (
    # ── 수용되어야 하는 정상 이름 ──
    ("Sales", True, "영문 대문자 시작"),
    ("dept-1", True, "kebab + 숫자"),
    ("dept_1", True, "underscore + 숫자"),
    ("dept-3", True, "kebab"),
    ("Sales_Team", True, "대문자 + underscore 혼합"),
    ("a", True, "1자 하한"),
    ("A9_x-y", True, "허용 문자 전종 혼합"),
    ("a" * 40, True, "40자 상한 경계(수용)"),
    # ── 거부되어야 하는 이름 ──
    ("", False, "빈 문자열"),
    ("a" * 41, False, "41자 — 상한 초과(소켓 104B 여유)"),
    ("-abc", False, "하이픈 시작"),
    ("-lead", False, "하이픈 시작"),
    ("_x", False, "underscore 시작"),
    ("a/b", False, "경로 구분자 — 디렉터리 탈출"),
    ("a.b", False, "점 — 경로 성분"),
    ("a b", False, "공백"),
    ("a;b", False, "세미콜론 — 셸 명령 구분자"),
    ("부서", False, "비ASCII(한글) — 소켓·경로 결정론 밖"),
    # ── ★개행 축(중대6 재현 입력 그대로) — 리터럴 대조로는 원리적으로 못 잡던 검체 ──
    #   구 `grep -Eq` 는 **줄 단위**라 여러 줄 중 **한 줄만** 맞아도 성립했다.
    ("abc\nrm -rf /", False, "★첫 줄이 정상 — 구 grep 통과(명령 주입)"),
    ("\nabc", False, "★둘째 줄이 정상 — 구 grep 통과"),
    ("abc\n", False, "★후행 개행 — 구 grep 통과($ 가 줄 끝을 앵커)"),
    ("x\nAAAA", False, "★두 줄 다 정상 — 구 grep 통과"),
    ("a\n\nb", False, "★빈 줄 낀 다줄"),
)


def _selftest_dept_name_parity():
    """부서명 판정의 **동작** 대조 — 리터럴 대조로는 못 잡는 의미론 차이를 잡는다."""
    corpus = [n for n, _want, _why in _DEPT_NAME_CORPUS]
    shell, err = _cys_dept_name_ok_batch(corpus)
    if err is not None:
        kind, detail = err
        # ★drift = hard fail(2026-08-22 master 결정). 파일이 **있는데** 대조를 못 돌렸다면
        #   그것은 부재가 아니라 신호다 — 조용히 넘기면 개행 결함이 재발해도 아무도 모른다.
        assert kind == "absent", (
            "★부서명 규약 **동작 대조가 무력화됐다** — %s\n"
            "  이건 '대조 대상 부재'가 아니라 **드리프트 신호**다(cys-dept 파일은 존재한다).\n"
            "  이 상태로 초록을 내면 `dept_name_ok` 가 다시 줄 단위 판정으로 돌아가도 "
            "self-test 가 잡지 못한다(중대6 재발 · 개행 이름이 경로·소켓에 들어간다).\n"
            "  조치: 추출기(`_extract_dept_name_ok_def`)를 현재 정의 형태에 맞추거나, "
            "cys-dept 소유 워커에게 정의 형태 변경을 확인하라." % detail)
        # 정당한 부재만 여기 온다 — 대조할 대상이 애초에 없다.
        print("javis_bootstrap self-test NOTE: 부서명 동작 대조를 건너뛴다(%s) — "
              "리터럴 대조로 대체하지 않는다" % detail, file=sys.stderr)
        return
    known = set(_DEPT_NAME_KNOWN_DIVERGENCE)
    asym = []
    for (name, want, why), sh in zip(_DEPT_NAME_CORPUS, shell):
        py = dept_name_ok(name)
        # ⓑ 절대 기대값 — 두 구현이 **함께 틀리는** 것을 비대칭 0 이 가려 주지 못하게 한다.
        assert py == want, (
            "발급기 dept_name_ok(%r) = %s, 기대 %s (%s). 이름은 경로·소켓 이름에 들어간다 — "
            "관대한 쪽이 위험하다." % (name, py, want, why))
        assert sh == want, (
            "생성기 cys-dept::dept_name_ok(%r) = %s, 기대 %s (%s). ★`cys-dept` 는 다른 워커 "
            "소유다 — 이 단언이 깨지면 고치지 말고 보고하라." % (name, sh, want, why))
        # ⓐ 비대칭 0
        if sh != py:
            asym.append((name, sh, py))
            assert name in known, (
                "부서명 판정 불일치(신규): %r → cys-dept=%s / javis_bootstrap=%s. "
                "이름은 경로에 들어간다(<dept>.ticket·cys-dept-<name>) — 관대한 쪽이 위험하다. "
                "두 구현을 같은 의미론으로 맞춰라(리터럴이 아니라 동작이 계약이다)."
                % (name, sh, py))
        elif name in known:
            print("javis_bootstrap self-test NOTE: 부서명 %r 의 알려진 불일치가 해소됐다 — "
                  "`_DEPT_NAME_KNOWN_DIVERGENCE` 에서 지워라(cys-dept 수리 반영)"
                  % name, file=sys.stderr)
    # ★비대칭 0 을 **명시적으로** 단언한다(허용 목록이 비어 있으므로 전건 일치가 계약이다).
    assert not asym, "부서명 판정 비대칭이 남아 있다: %r" % (asym,)
    # 내 쪽 보증: 개행이 든 이름은 **무조건 거부**한다(경로 안전 — 이건 갈릴 수 없다)
    for name in ("abc\nrm -rf /", "\nabc", "abc\n", "x\nAAAA", "a\n\nb"):
        assert not dept_name_ok(name), "개행이 든 부서명을 수용한다(경로 안전 붕괴): %r" % name


def cmd_issue_ticket(argv):
    """CEO 티켓 발급 — base 레인 전용. 사용: issue-ticket --dept <name>.
    exit: 0=발급(경로 stdout) / 2=base 레인 아님 또는 --dept 형식 위반.
    이 게이트는 LLM 드리프트 차단용 결정론 가드이지 보안 경계가 아니다(동일 $HOME 신뢰 도메인)."""
    dept = None
    for i, a in enumerate(argv):
        if a == "--dept" and i + 1 < len(argv):
            dept = argv[i + 1]
        elif a.startswith("--dept="):
            dept = a.split("=", 1)[1]
    if not dept or not dept_name_ok(dept):
        # ★중대③ 봉합: 생성기(cys-dept::dept_name_ok)와 **같은 집합**으로 넓혔다. 종전
        #   `[a-z0-9][a-z0-9-]*` 은 `Sales`·`dept_1` 로 만든 부서의 티켓을 영영 거부했다.
        sys.stderr.write("[issue-ticket] --dept <name>(%s · cys-dept 생성 규약과 동일) 필수: %r\n"
                         % (DEPT_NAME_RE.pattern, dept))
        return 2
    if not _is_base_socket():
        sys.stderr.write("[issue-ticket] base 레인에서만 티켓 발급 허용 — 현재 소켓은 부서 레인(%s). "
                         "본부(base) master에서 발급하라.\n" % os.environ.get("CYS_SOCKET", ""))
        return 2
    now = time.time()
    ticket = {"dept": dept, "issued_at": now,
              "issued_at_iso": time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(now)),
              "issuer": os.environ.get("CYS_SURFACE_ID", "") or "base-master"}
    path = _ticket_path(dept)
    _atomic_write_json(path, ticket)
    print(json.dumps({"ok": True, "dept": dept, "ticket": path,
                      "ttl_hours": round(TICKET_TTL_SECS / 3600, 1)}, ensure_ascii=False))
    return 0


def cmd_run():
    """★A7 단일 종료 불변식의 소유자 — 여기 밖으로 나가는 종료 경로는 없다.

    ① 싱글플라이트 패자 → `_emit_skip_verdict()`(stderr verdict + exit 11 · boot-last 무접촉)
    ② 그 밖 전부 → `_Log()` 생성 후 `_cmd_run_chain()`, **try/finally 로 종결 기록**(A19).
       finally 는 정상 return·중간 return·예외·SystemExit 전부에서 돈다 — '진행 중'으로 영원히
       남는 레코드가 사라진다.
    ★예외는 삼키지 않는다(re-raise): 트레이스백은 진단의 최종 증거다. 다만 기록되는 exit 는
      파이썬이 실제로 낼 값(uncaught=1)이어야 하므로 그렇게 남긴다 — 기록≠실측 금지.
    """
    if _acquire_singleflight() is None:
        return _emit_skip_verdict()
    # ★P0-3 래치 스냅샷은 반드시 _Log 생성 **이전**이다 — __init__ 의 선기록(_persist)이 슬롯을
    #   즉시 덮으므로 순서가 뒤집히면 carry-forward 원본이 사라진다. 싱글플라이트 획득 **후**라
    #   read-modify-write 경합도 없다(패자는 boot-last 무접촉 exit 11).
    log = _Log(_load_retry_carry(lane_state_path("boot_last")))
    exit_code = None
    try:
        exit_code = _cmd_run_chain(log)
        return exit_code
    except SystemExit as e:
        exit_code = e.code if isinstance(e.code, int) else 1
        raise
    except BaseException as e:
        exit_code = 1                        # uncaught → 파이썬 프로세스 실제 exit
        log.finish(exit_code, exc="%s: %s" % (type(e).__name__, e))
        raise
    finally:
        if "ended" not in log.data:          # 예외 경로에서 이미 기록했으면 중복 기록 금지
            log.finish(exit_code if exit_code is not None else 1)


def _cmd_run_chain(log):
    """부트 단계 체인(①~⑧) — 종료 기록은 호출자(cmd_run)의 try/finally 가 소유한다."""
    py = sys.executable or "python3"

    # ★불량 레인 가드(R1-LOW-2): 빈 부서명(cys-dept-/ — suffix 없음) 소켓은 base도 부서도 아닌
    # 불량 레인 — 어느 레인 계약(마커·티켓·팩 페어링)에도 못 들어가므로 시끄럽게 명시 실패한다
    # (레인↔팩 가드 계열·exit 8).
    if _socket_malformed_dept():
        detail = ("불량 레인(빈 부서명): CYS_SOCKET=%s 의 'cys-dept-' 성분에 부서명이 없다 — "
                  "base도 부서도 아닌 소켓으로는 부트 불가. 부서를 정규 이름(cys-dept-<name>)으로 "
                  "재생성한 뒤 재선언하라." % os.environ.get("CYS_SOCKET", ""))
        log.step(STEP.LANE_MALFORMED, 1, detail)
        _progress("⚠ " + detail)
        notified = _notify_loud("불량 레인(빈 부서명 — 부트 중단)", detail)
        log.step(STEP.LANE_MALFORMED_NOTIFY, 0, "알림 채널: %s" % notified)
        log.result(ok=False, state="failed", failed_step="lane-pack", exit=EXIT_LANE_PACK)
        return EXIT_LANE_PACK

    # ★레인↔팩 정합 가드(증분1 · UT-14 교차 오염 차단): 부서 소켓 레인은 그 부서 팩(pack-dept-X)을,
    # base 레인은 메인 팩을 써야 한다. 불일치면 잘못된 데몬/팩 조합이 마커·승격·디렉티브를 오염시키므로
    # 팀 기동(④) 전에 시끄럽게 실패한다(조용한 진행이 최악 — adv#5 실패 가시화 계열).
    mismatch = _lane_pack_mismatch()
    if mismatch is not None:
        sd, pd = mismatch
        detail = ("레인↔팩 불일치(교차 오염·UT-14): 소켓 부서=%s · 팩 부서=%s. CYS_SOCKET과 "
                  "CYS_PACK_DIR이 같은 부서를 가리켜야 한다(base↔메인팩 / dept-X↔pack-dept-X). "
                  "팀 기동 중단." % (sd or "base", pd or "메인"))
        log.step(STEP.LANE_PACK, 1, detail)
        _progress("⚠ " + detail)
        notified = _notify_loud("레인↔팩 불일치(부트 중단)", detail)
        log.step(STEP.LANE_PACK_NOTIFY, 0, "알림 채널: %s" % notified)
        log.result(ok=False, state="failed", failed_step="lane-pack", exit=EXIT_LANE_PACK)
        return EXIT_LANE_PACK

    # ★TCC 보조 경고(2026-07-15): macOS 폴더 권한 리셋(서명 변경 업그레이드) 시 pane 자식이
    # EPERM으로 죽는 실사고 — 부트가 살아있는 세션에서라도 조기 경고(주 안내는 GUI perm-warning).
    for _probe, _label in _tcc_probe_targets():
        try:
            os.listdir(_probe)
        except PermissionError:
            _progress("⚠ macOS 폴더 접근 거부(%s: %s) — 시스템 설정→개인정보 보호 및 보안→"
                      "파일 및 폴더에서 cys 허용 후 앱 재시작(미허용 시 pane의 claude가 EPERM으로 꺼짐)"
                      % (_label, _probe))
            break
        except OSError:
            pass

    # ① preflight --fix — ★비치명화(2026-07-15 적대검증 adv#1 CRITICAL): 종전엔 preflight가
    # 완전-green(exit 0)이 아니면 여기서 abort해 ④ 팀 부팅이 영영 안 됐다. preflight는 60+ 체크
    # 표면이라 자동수리 불가 FAIL 하나(구 hook·수동 디렉티브 핀·git 부재)만 있어도 팀 0개 — "5노드
    # 100%" 요구와 정면 충돌(이 기계도 잔여 FAIL 존재). 팀 부팅의 진짜 게이트는 ⑤ check다. 따라서
    # preflight FAIL은 경고로 강등하고 ④로 계속한다. 부팅-치명 전제(데몬·claude)는 ②ping·cys boot가
    # 각자 검증하므로 preflight와 분리해도 안전. 마커가 현재 pack_version이면 300s preflight 자체를
    # 생략(재선언 fast path — pile-up·재실행 비용 제거).
    preflight = os.path.join(PACK, "bin", "javis_preflight.py")
    # ★P3-A-DEPT-LANE(W3): fast path 는 **레인 마커**를 읽는다. 종전엔 `_is_base_socket()` 조건이
    #   붙어 부서 레인엔 fast path 가 아예 없었고, 부서장 재선언마다 300s preflight 를 통째로 다시
    #   돌았다(레인별 마커가 없었으니 판정 근거 자체가 없었다). base 레인 경로·의미는 불변이고,
    #   CEO 승격 게이트가 읽는 base 마커는 여전히 base 레인만 쓴다(금지 방향 ①).
    _marker = _read_json(lane_state_path("marker")) or {}
    _marker_fresh = (_marker.get("pack_version") == _pack_version()
                     and _marker.get("pack_version") not in (None, "unknown"))
    if _marker_fresh:
        log.step(STEP.PREFLIGHT, 0,
                 "레인 마커(%s)가 현재 pack_version — preflight 생략(fast path)"
                 % lane_state_path("marker"))
    elif os.path.isfile(preflight):
        _progress("① preflight --fix 실행 중(최대 300s · 비치명 — FAIL이어도 팀 부팅 계속)…")
        code, out = _run([py, preflight, "--fix"], timeout=300)
        log.step(STEP.PREFLIGHT, code, out)
        if code != 0:
            _progress("⚠ preflight 잔여 FAIL(비치명) — 팀 부팅 계속·진짜 게이트는 ⑤ check. 상세 boot-last.json")
    else:
        log.step(STEP.PREFLIGHT, 0, "preflight 부재 — 생략(팩 불완전 가능·계속)")

    # ② 데몬 생존 — 이후 ③의 비정상 exit를 '거부'로 해석하는 전제(데몬 생존 보증)
    # ★유계 재시도(W-A3 — 상수 블록 PING_* 의 설계 근거 주석 참조): 종전 단발 15s ping 은 데몬
    #   콜드스타트·Defender 첫 스캔 창의 첫 실패 하나로 선언 전체를 폐기했다. 벽시계 데드라인
    #   (PING_RETRY_TOTAL_S) 안에서 간격(PING_RETRY_INTERVAL_S) 재시도한다.
    #   ⓐ 진입 게이트 = 데드라인(잔여 창 < 간격이면 재진입 금지) · **진입한 시도는 자기 상한
    #     (DAEMON_PROBE_TIMEOUT_S)을 다 쓴다** — javis_budget.ping_retry_worst_s() 가 계상하는
    #     'TOTAL + 마지막 시도 granularity' 유계화 패턴과 1:1(계상=실최악 · 역전 0).
    #   ⓑ 하트비트는 벽시계 스로틀(HEARTBEAT_INTERVAL_S) — 시도당 발화는 fail-fast(즉시 거절)
    #     에서 최대 ~15줄 소음이 된다(간격 3s × 창 45s). 침묵 창 상쇄와 폭주 방지의 균형.
    #   ⓒ ping 은 관측 전용(스폰 0·큐/feed 발화 0) — 재시도가 무엇도 반복 유발하지 않는다(앵커 ①).
    _progress("② 데몬 생존 확인(유계 재시도 — 총예산 %.0fs 창·간격 %.1fs)…"
              % (PING_RETRY_TOTAL_S, PING_RETRY_INTERVAL_S))
    _ping_t0 = time.monotonic()
    _ping_deadline = _ping_t0 + PING_RETRY_TOTAL_S
    _ping_hb_at = _ping_t0 + _budget_leaf("HEARTBEAT_INTERVAL_S", 20)
    ping_attempts = 0
    while True:
        ping_attempts += 1
        code, out = _run(["cys", "ping"], timeout=DAEMON_PROBE_TIMEOUT_S)
        # 첫 시도는 종전과 동일하게 무suffix(happy path 의 boot-last 형태 불변) — 재시도만 #N.
        log.step(STEP.PING, code, out,
                 suffix="" if ping_attempts == 1 else "#%d" % ping_attempts)
        if code == 0:
            break
        now = time.monotonic()
        if now + PING_RETRY_INTERVAL_S >= _ping_deadline:
            break                      # 잔여 창 < 간격 — 유계: 더 기다리지 않는다(EXIT_PING 로)
        if now >= _ping_hb_at:
            _progress("② ping 무응답(rc=%s) — 재시도 중 %d회째·%.0fs 경과(창 %.0fs · "
                      "데몬 콜드스타트/Defender 첫 스캔 내성 대기)"
                      % (code, ping_attempts, now - _ping_t0, PING_RETRY_TOTAL_S))
            _ping_hb_at = now + _budget_leaf("HEARTBEAT_INTERVAL_S", 20)
        time.sleep(PING_RETRY_INTERVAL_S)
    if code != 0:
        _ping_waited = time.monotonic() - _ping_t0
        # ★진단 계약(W-A3): '몇 회 시도·총 몇 초 대기'를 명시한다 — 재시도까지 소진한 무응답은
        #   일시 지연이 아니라 데몬 부재·기동 실패 쪽이므로 관찰자에게 그 판별 근거를 준다.
        return log.fail(STEP.PING, code,
                        "cys ping 유계 재시도 소진 — %d회 시도·총 %.1fs 대기(창 상한 %.0fs)에도 "
                        "데몬 무응답(마지막 rc=%s).\n%s"
                        % (ping_attempts, _ping_waited, PING_RETRY_TOTAL_S, code, out), EXIT_PING)

    # ③ claim-role master — 거부=exit 7(유령 master 차단: 이 surface는 master가 아니다)
    # ★SEAT(2026-07-17 실사고): 보유자가 '빈 좌석'(role 만 쥔 agent 없는 셸 — cys-dept 가 부서 생성 시
    #   띄우는 그 셸)이면 종전엔 여기서 영구 거부돼 부트가 데드엔드에 빠졌다(부서장이 영영 못 뜸).
    #   --takeover-empty-seat 는 **요청**일 뿐이다: 데몬이 커널 사실(자손 프로세스 0·agent 메타 없음·
    #   최근 입력 없음)로 재판정해 정말 빈 좌석일 때만 승계를 허용하고, agent 가 붙은 정당한 master 는
    #   종전대로 거부한다(유령 master 차단 규칙 불변 — 살아있는 master 가 있으면 여전히 exit 7).
    # ★A20 타입드 판정(W1b 소비층): rc≠0 을 전부 '정당거부(exit 7)'로 접던 것을 둘로 쪼갠다.
    #   ⓐ 출력에 데몬의 거부 마커(claim_denied / privileged role held)가 있으면 **정당거부=exit 7**
    #     — "살아있는 master 가 있다"는 판정이 성립한 경우다(지휘 중단·인계가 정답).
    #   ⓑ 마커가 없는 rc≠0 은 **세션 컨텍스트 오류=exit 10**: CYS_SURFACE_ID 부재(cys.rs 가 대상
    #     surface 를 못 정함)·데몬 미응답·바이너리 부재·사용오류 등. 이걸 7로 보고하면 사용자에게
    #     "다른 pane 이 이미 master 다"라는 **거짓 판정**을 주고, 그 처방(기존 master 탭으로 가라)은
    #     실제 원인(세션 배선)을 영영 못 찾게 만든다(A20: 판정·escalation 층위의 뭉개기).
    # ★L2 선행 claim 소비(2026-08-16 현장 결함 근본수리) ──────────────────────────────────
    # 데몬 claim_role 은 발신 pane 을 **커널 peer pid 의 조상 체인**으로 확정한다(위조 방지 —
    # 클라이언트 자기신고 CYS_SURFACE_ID 는 신뢰하지 않는다). 그런데 훅은 이 스크립트를 백그라운드로
    # 발화하고 곧 종료하므로, 이 프로세스는 **재부모화(ppid→1)** 되어 조상 체인이 끊긴다 → 여기서
    # claim 을 치면 언제나 '발신 pane 미해석'으로 거부된다(실측 e2e: 같은 surface 에서 동기 실행은
    # 성공·분리 실행은 거부). 그래서 훅 발화 경로에서는 **조상 체인이 온전한 훅 프로세스**가 spawn
    # 이전에 claim 을 끝내고, 이 스크립트는 그 판정을 소비한다(claim 을 두 번 치지 않는다 — 중복
    # claim 은 좌석 프로브·감사 이벤트를 두 번 태운다).
    # 판정 전달은 env 다: CYS_CLAIM_RC(필수·정수) / CYS_CLAIM_OUT(진단 문안·선택).
    # ★감독자 주입 제3 경로(P2 · R3-P2-7 ⑥): 이 env 4종의 생산자는 훅만이 아니다 — 데몬 부트
    #   감독자(boot_supervisor.rs run_ensure_team)도 스풀 인텐트 디스패치 직전 자기 roles
    #   레지스트리 **재실측**이 참일 때만 rc=0·신선한 CYS_CLAIM_AT(훅 스탬프 이월 금지)·
    #   CYS_CLAIM_OUT="[supervisor] registry re-verified" 를 주입한다(불일치=무스폰 Retire).
    #   소비 결박(_pre_bound)은 생산자 공통이고 이 스크립트는 생산자를 구분하지 않는다 —
    #   provenance 는 CYS_CLAIM_OUT 문안이 실어 나른다. 잔여 위조면은 THREAT-MODEL §4-10 등재.
    # env 가 없으면(§0 폴백의 포그라운드 직접 실행·구 훅) 종전대로 여기서 직접 claim 한다 —
    # 그 경로는 조상 체인이 온전하므로 정상 동작한다(하위호환·스큐 안전).
    # ★판정 결박(무바인딩 env 금지): 정수처럼 보이는 env 하나로 claim 을 건너뛰면, 사용자 셸·
    #   래퍼에 남은 값이 **치지도 않은 claim 을 '실측'으로** boot-last 에 적게 된다(CS-3 보고=실측
    #   위반). 그래서 판정은 ⓐ같은 surface 귀속(CYS_CLAIM_SID) ⓑ신선도(CYS_CLAIM_AT — **런 시작
    #   시각 `_RUN_T0` 기준** CYS_CLAIM_MAX_AGE_S=300s · P0-1 재정의)까지 갖췄을 때만 소비한다.
    #   하나라도 어긋나면 **무시하고 직접 claim** 한다(구 훅·직접 실행과 동일 경로 — 하위호환이
    #   곧 안전한 기본값이다). 신선도가 소비 시각이 아니라 런 시작 기준인 이유·의미 재정의
    #   고지는 상수부 `_RUN_T0` 계약 주석이 정본이다(CLM-2: ①preflight·②ping 의 in-run 소요가
    #   신선한 claim 을 소비 시점 만료로 접던 결합의 절단).
    _progress("③ master 역할 등록…")
    _pre_rc = os.environ.get("CYS_CLAIM_RC", "").strip()
    _pre_sid = re.sub(r"[^0-9]", "", os.environ.get("CYS_CLAIM_SID", ""))
    _my_sid = re.sub(r"[^0-9]", "", my_surface_id())
    try:
        # ★런 시작 기준 나이(P0-1): _RUN_T0 는 모듈 로드(≈훅 spawn 직후)에 1회 캡처된 벽시계 —
        #   ①②가 여기 도달 전에 아무리 오래 걸려도 이 값은 변하지 않는다. `date +%s` 실패 시
        #   훅은 0 을 싣는데(role-bootstrap.sh:674) 그 경우 나이 ≈ 현재 epoch(수십억 초)로
        #   여전히 창 밖 기각 — 종전 안전 거동 불변.
        _pre_age = _RUN_T0 - float(os.environ.get("CYS_CLAIM_AT") or 0)
    except (TypeError, ValueError):
        _pre_age = float("inf")
    if _pre_rc and _pre_age < 0:
        # 음수 = 훅 스탬프가 _RUN_T0 보다 미래(스탬프↔기동 사이 NTP 후퇴). 유계 허용창
        # (-_CLAIM_SKEW_TOL_S) 안이면 결박은 유지하되, 시계가 움직인 사실 자체는 진단으로 남긴다.
        sys.stderr.write("[bootstrap] 선행 claim 스탬프가 런 시작보다 미래(%.0fs) — 시계 후퇴 "
                         "감지(허용창 %.0fs 안이면 결박 유지)\n" % (-_pre_age, _CLAIM_SKEW_TOL_S))
    _pre_max = float(os.environ.get("CYS_CLAIM_MAX_AGE_S") or 300)
    # ★MAX<=0 가드(수동 튜닝 관용 보존): 구 술어(0<=age<MAX)에서 MAX=0 은 '항상 미결박'
    #   (소비 차단 idiom)이었다. 유계 음수 허용(-120s<=age<0)만으로는 시계 후퇴 창에서
    #   MAX=0 이 결박을 허용하게 되므로, MAX<=0 이면 나이와 무관하게 무조건 미결박으로 접는다.
    _pre_bound = (_pre_max > 0
                  and _pre_rc.lstrip("-").isdigit() and _pre_sid and _pre_sid == _my_sid
                  and -_CLAIM_SKEW_TOL_S <= _pre_age < _pre_max)
    if _pre_rc and not _pre_bound:
        sys.stderr.write("[bootstrap] 선행 claim 판정 미결박 — 무시하고 직접 claim 한다"
                         "(rc=%r sid=%r≠%r age=%.0fs)\n" % (_pre_rc, _pre_sid, _my_sid, _pre_age))
    if _pre_bound:
        code = int(_pre_rc)
        out = os.environ.get("CYS_CLAIM_OUT", "") or "(선행 claim 출력 없음)"
        out = ("[선행 claim 소비] 훅(role-bootstrap.sh)이 **조상 체인이 온전한 시점에** "
               "claim-role 을 수행했고 이 런은 그 판정(rc=%d · surface=%s · %.0fs 전)을 "
               "소비한다.\n%s" % (code, _my_sid, _pre_age, out))
        log.step(STEP.CLAIM_ROLE, code, out)
    else:
        code, out = _run(["cys", "claim-role", "master", "--takeover-empty-seat"],
                         timeout=_budget_leaf("CYS_CLAIM_TIMEOUT_S", 15))
        log.step(STEP.CLAIM_ROLE, code, out)
    if code != 0:
        low = (out or "").lower()
        # ★A20 타입드 exit 소비(W2 · H-EXIT-3): CLI 가 이제 판정 타입을 **exit 로** 낸다 —
        #   0=성공 / 7=정당거부 / 3=미도달 / 2=식별불가. 문자열 grep 은 **구 바이너리 하위호환
        #   폴백**으로만 남긴다(신 바이너리에서는 exit 가 1차 근거다). 종전엔 grep 이 유일 근거라
        #   데몬 메시지 문안이 바뀌면 정당거부가 조용히 '세션 오류'로 오분류됐다(문자열 계약 드리프트).
        if code == EXIT_CLAIM_DENIED or (code == 1 and any(m in low for m in _CLAIM_DENIED_MARKERS)):
            # ★위계 폴백(현장 결함 3호 · D1ⓐ): 정당거부는 '유령 master 차단'이지 '조직 확장
            #   금지'가 아니다. base 레인 + unix 면 선언을 부서 창설로 이어준다(_dept_fallback
            #   상단 계약 주석 참조). 비적용(None)이면 종전 exit 7 경로 그대로(안내만 보강).
            fb = _dept_fallback(log, out)
            if fb is not None:
                return fb
            msg = ("이 surface는 master가 아님(claim 거부). 살아있는 master가 레지스트리에 존재한다 — "
                   "선언을 중단하고 기존 master에 인계하라. 새 부서장을 세우려는 의도였다면: GUI "
                   "＋부서(부서 워크스페이스 추가)를 쓰거나, base 레인 unix 에서 **오너가 직접 타이핑한** "
                   "선언(훅 발화 경로)으로 재선언하라 — 그 경로만 부서 자동 생성으로 이어진다"
                   "(직접 실행·기계 배달 선언은 폭주 봉인 ⓑ로 비적용).\n%s" % out)
            # ★ok=None(CS-2⑩): 정당거부는 '이 레인의 부트가 깨졌다'가 아니다 — 공유 boot-last 의
            #   ok:true(건강한 master 의 완주 기록)를 ok:false 로 덮으면 §0 이 churn 한다.
            return log.fail(STEP.CLAIM_ROLE, code, msg, EXIT_CLAIM_DENIED,
                            ok=None, state="declined")
        # ★A20 타입드 exit 의 정확한 처방 분기(6=신원 미확정 / 3=미도달 / 2=식별불가 / 그 밖=구 계약).
        #   ※ 이 숫자들은 **cys CLI 의 rc 이름공간**이다(위 EXIT_* 는 이 스크립트의 이름공간 —
        #     값이 겹쳐 보여도 다른 계약이다. rc 6 ≠ EXIT_CHECK).
        kind = {6: ("발신 신원 미확정 — 데몬은 응답했으나 이 프로세스를 발신 pane 에 붙이지 못했다"
                    "(세션 분리·재부모화로 조상 체인 단절 · pane 밖 실행 · 타 surface 지정). "
                    "**'다른 pane 이 master' 라는 뜻이 아니다** — 부서 자동 생성으로 이어지지 않는다. "
                    "처방: pane 안에서(훅 발화 경로면 훅이 선행 claim 한다) 재선언하라. "
                    "★(P1 seat 토큰) 같은 rc 6 가족에 데몬 사유 2종이 더 있다(출력의 reason 확인): "
                    "token_mismatch = 실려 온 seat 토큰이 대상 surface 토큰과 다르고 동세대"
                    "(env 오염·타 surface 토큰 복사 의심 — 그 pane 의 env 그대로 pane 안에서 재claim), "
                    "token_chain_conflict = 토큰은 유효하나 발신 조상 체인이 다른 pane 으로 해석"
                    "(타 pane 토큰 절취·env 복사 의심 — 자기 pane 에서 재선언). 둘 다 "
                    "**'살아있는 master 가 있다'가 아니며** 처방·행동 계약(부서 자동 생성 금지·"
                    "session_error 기록)은 종전과 동일하다."),
                3: ("미도달 — 요청이 데몬에 닿지 못했다(소켓 부재·데몬 다운·왕복 실패). "
                    "처방: `cys ping` 으로 데몬을 확인하라."),
                2: ("식별 불가 — surface 해석 실패·인자 오류(요청을 만들 수조차 없었다). "
                    "처방: 이 세션이 cys pane 안인지(CYS_SURFACE_ID) 확인하라.")}.get(
            code, "구 계약(exit %s) — 거부 마커가 출력에 없다" % code)
        msg = ("역할 등록 왕복이 **거부가 아닌** 사유로 실패했다(세션 컨텍스트 오류): %s "
               "'다른 pane이 master' 라는 뜻이 **아니다** — 세션 배선을 확인하라.\n%s" % (kind, out))
        return log.fail(STEP.CLAIM_ROLE_CONTEXT, code, msg, EXIT_SESSION_CONTEXT,
                        ok=None, state="session_error")
    log.data["role_claimed"] = "master"   # 관측 파생 귀속(보고=실측)

    # ── 증분2 ⓐ CEO 티켓 권한 게이트(P7) — 부서 레인 팀 기동 전. 티켓 부재/만료=단독 각성 강등(exit 0) ──
    dept = _socket_dept()
    ticket_path = None
    if dept is not None:
        _progress("③″ CEO 티켓 권한 게이트(부서 레인=%s)…" % dept)
        ok, why, ticket_path = _peek_dept_ticket(dept)
        requested, req_why = False, None
        log.step(STEP.CEO_TICKET, 0,
                 ("CEO 티켓 유효 — 부서 팀 기동 진행. %s" % why) if ok else
                 ("CEO 티켓 부재/만료 — base CEO 에 발급을 **요청**하고 유계 대기한다. 사유: %s"
                  % why))
        if not ok:
            # ★2026-08-22 결함 #2 봉합: 종전엔 여기서 안내문만 출력하고 끝나 부서장이 팀 없이
            #   대기했고, **오너가 추가 명령을 쳐야** CEO 에게 요청이 갔다(실측 06:20→06:26→06:29).
            #   오너 절대규칙은 "선언자는 새 부서장이 되며 팀이 기동돼야 한다"이므로 요청은
            #   스크립트가 결정론으로 발사한다. 실패는 전부 fail-open(부트 무중단).
            requested, req_why = _request_dept_ticket(dept)
            log.step(STEP.CEO_TICKET_REQUEST, 0 if requested else 1, req_why)
            _progress(("③″ CEO 티켓 발급 요청 push 발사 — %s" if requested
                       else "⚠ ③″ CEO 티켓 발급 요청 미발사 — %s") % req_why)
            if requested:
                _progress("③″ CEO 티켓 도착 대기(최대 %ds · %ds 간격 폴링)…"
                          % (DEPT_TICKET_WAIT_BUDGET_S, DEPT_TICKET_WAIT_INTERVAL_S))
                ok, wait_why, ticket_path = _await_dept_ticket(dept)
                log.step(STEP.CEO_TICKET_WAIT, 0 if ok else 1, wait_why)
                _progress(("③″ CEO 티켓 수령 — 팀 기동으로 진행. %s" if ok
                           else "③″ CEO 티켓 미도착 — 단독 각성 유지. %s") % wait_why)
                why = wait_why
        if not ok:
            # ── #4-a: 단독 각성 보고는 **원인·현재 상태·다음 단계**를 한 문장 안에 다 말한다 ──
            #   종전 문안은 "발급하라"는 명령형뿐이라, 오너가 ⓐ요청이 이미 나갔는지 ⓑ기다리면
            #   되는지 ⓒ자기가 뭘 해야 하는지를 알 수 없었다.
            if requested:
                note = ("CEO 티켓 부재 — CEO에 티켓 발급을 **요청했습니다**(대기 중). "
                        "도착하면 팀이 자동 기동되고, 미도착이면 부서장 단독 각성을 유지합니다. "
                        "지금은 단독 각성 상태입니다(팀 미기동). 요청: %s / 대기 결과: %s"
                        % (req_why, why))
            else:
                note = ("CEO 티켓 부재 — 발급 요청을 **보내지 못했습니다**(%s). "
                        "부서장 단독 각성으로 계속합니다(팀 미기동). 수동 발급: base master 에서 "
                        "`javis_bootstrap.py issue-ticket --dept %s` — 발급되면 다음 부트에서 "
                        "팀이 기동됩니다. 사유: %s" % (req_why, dept, why))
            # R1-LOW-3 검증 비대칭 경고: 발급(issue-ticket)은 정규식을 강제하나 소켓 쪽 부서명은
            # 자유 형식이라, 불일치 부서는 티켓을 영영 못 받는 비대칭이 침묵으로 남는다 — 명시.
            if not dept_name_ok(dept):
                note += (" ★주의: 부서명 %r 은 발급 규약(%s) 불일치 — 이 부서명으로는 티켓을 "
                         "발급할 수 없다(부서 재생성 필요). 그래서 CEO 요청도 보내지 않았다."
                         % (dept, DEPT_NAME_RE.pattern))
            _progress(note)
            log.step(STEP.CEO_TICKET_SOLO, 0, note)
            summary = {"ok": True, "marker": "부서장 단독 각성(CEO 티켓 부재)",
                       "solo_awakening": True, "dept": dept,
                       # ★요청 축을 기계 필드로도 노출한다 — 산문만 있으면 소비자(GUI·상위 보고)가
                       #   "요청이 나갔는가"를 문자열 파싱으로 알아내야 한다.
                       "ticket_requested": requested, "ticket_request_detail": req_why,
                       "steps": [(s["step"], s["exit"]) for s in log.data["steps"]],
                       "lane": log.lane, "boot_last": log.path}
            # ★A7 채널 보존: solo_awakening 은 **성공** 경로이므로 stdout 최종 JSON 을 유지한다
            #   (session-start 산문 계약 "완료 선언은 최종 JSON 인용 시에만"의 소비 대상).
            log.result(ok=True, state="solo_awakening", solo_awakening=True, reason=why,
                       ticket_requested=requested, exit=EXIT_OK)
            print(json.dumps(summary, ensure_ascii=False))
            return EXIT_OK

    # ── 증분2 ⓑ 결손 기준 자원 사전 게이트 — 팀 기동(④) 직전 ──
    # 결손 산출 1차 경로는 orchestra 정본 _shared_verdict_deficit 위임(⑤check 판정 코어 소비
    # + unknown 시한부 해소 — W-B3)이고, cys list 로스터 판정(W0 · effective_required_roles
    # 소비)은 강등 폴백이다(_team_has_deficit docstring). R1-MED-1의 '총수 비교 폐기'는 그대로.
    # 결손 0(재선언·의무 좌석 전원 생존) → 게이트와 ④ cys boot 호출 자체를 생략("결손 0=스폰 없음").
    orchestra = os.path.join(PACK, "bin", "javis_orchestra.py")
    has_deficit, deficit_why = _team_has_deficit()
    if has_deficit:
        gate_rc = _run_resource_gate(py, log)
        if gate_rc is not None:
            return gate_rc  # EXIT_RESOURCE_HARD(9) = 자원 hard_block(팀 기동 0·escalation)

        # ④ 4종 의무 노드 기동 — 결손>0에서만 호출(결손 0=스폰 경로 미진입)
        boot_budget = _budget_derived("cys_boot_outer_s", 300)
        _progress("④ 4종 의무 노드 기동 중(최대 %ds — 예산 파생)…" % boot_budget)
        code, out = _run(["cys", "boot", "--json"], timeout=boot_budget)
        # ★★팩↔바이너리 스큐 방어(온보딩 치명 위험 차단): 구 `cys` 바이너리는 `--json` 을 모르므로
        #   clap 이 **사용오류(exit 2)** 로 즉사한다 — 그대로 두면 아래 `_boot_fatal_verdict` 가
        #   '비0=Fatal' 보수 폴백을 적용해 **모든 부트가 exit 4 로 실패**한다(팩만 먼저 갱신된
        #   기계 = 온보딩 전멸·팀 0). 미지 인자 신호를 확인하면 bare `cys boot`(구계약)로 1회
        #   재시도한다. 사용오류는 **인자 파싱 단계**라 스폰이 아직 없었으므로 이중 스폰 위험 0이다.
        if code != 0 and _is_unknown_arg_error(out):
            log.step(STEP.BOOT_SKEW, code,
                     "`cys boot --json` 미지원(구 바이너리) — bare `cys boot` 로 1회 폴백. "
                     "팩↔바이너리 버전 스큐를 해소하라(`cys --version` 확인).")
            _progress("⚠ ④ `--json` 미지원 바이너리 — 구계약(bare cys boot)으로 폴백")
            code, out = _run(["cys", "boot"], timeout=boot_budget)
        log.step(STEP.BOOT, code, out)
        # ★B1 PLAN 정책 열 소비 — exit 4(부트 실패)는 **Fatal 실패에서만** 낸다.
        #   종전엔 `cys boot` 의 어떤 비0 도 exit 4 로 승격돼, 리뷰어 1종 고장(Degrade)이 팀 전체
        #   기동 실패로 번지는 영구 데드엔드였다(B1). 판정은 --json 의 role 별 outcome·mandatory 를
        #   읽어 내리고, --json 소비 불가(구 바이너리)면 종전 계약(비0=Fatal)으로 보수 폴백한다.
        fatal_why = _boot_fatal_verdict(code, out)
        if fatal_why is not None:
            # 티켓은 아직 미소비 — boot 실패(exit 4)면 보존돼 재시도 가능(R2-LOW-C)
            return log.fail(STEP.BOOT, code, fatal_why, EXIT_BOOT)
        # ★(W4 · G11) busy = **무스폰**. 성공(0)도 실패도 아니므로 ⓐ티켓을 태우지 않고
        #   ⓑDegrade 경고도 내지 않는다(정상적인 훅↔GUI 중첩 부트에서 매번 뜨는 위경보 차단).
        #   팀은 락을 쥔 그 런이 세우고, 이 런의 최종 게이트는 ⑤check 다(재시도 창 내장).
        boot_busy = _boot_was_busy(code, out)
        gate_why = _boot_gate_pending_verdict(code, out)
        if boot_busy:
            log.step(STEP.BOOT_BUSY, code,
                     "다른 boot 가 락 보유 — 이 런은 무스폰 skip(exit %s). 티켓 미소비·Degrade 아님. "
                     "팀 기동 확인은 ⑤check 가 담당한다." % code)
            _progress("④ 다른 boot 진행 중(무스폰 skip) — 티켓 보존·⑤ 생존 확인으로 진행")
        # ★(M3-짝) 제3 상태: 관문 보류. Fatal 로 접으면 살아 있는 좌석에 회수·파괴 처방이
        #   나가고(U-11 치명위험 ④), Degrade 로 접으면 "Fatal 역할은 전원 확보" 라는 거짓
        #   문장이 남는다. 큰 소리로 이름 붙여 기록하고 ⑤check 로 넘긴다 — ⑤ 의 결손 산출은
        #   gate_pending 좌석을 '못 쓰는 좌석' 으로 세므로(U-10 4자 파리티) 조용한 성공은 없다.
        elif gate_why is not None:
            log.step(STEP.BOOT_GATE_PENDING, code, gate_why)
            _progress("⚠ ④ 의무 노드가 첫기동 관문 보류 — 좌석은 살아 있다(회수·파괴 없음) · "
                      "사람이 관문 1회 통과 후 재부트 · ⑤ 생존 확인으로 계속")
        elif code != 0:
            log.step(STEP.BOOT_DEGRADE, code,
                     "비0 이지만 Fatal 역할은 전원 확보 — 경고 강등 후 ④-b·⑤ 계속(B1 정책 열)")
            _progress("⚠ ④ 일부 선택/리뷰어 노드 미기동(Degrade) — 팀 기동 계속")

        # 부서 레인 CEO 티켓 소비 — ④ boot **성공 직후**(실스폰 발생)에만 1회성 소비(R2-LOW-C):
        # "1회성 티켓 ⟺ 실스폰" 불변식. 착수 전 소각은 boot 실패 시 재시도 티켓까지 태웠다.
        # ★busy(무스폰)에서는 소비하지 않는다 — 티켓 1장은 스폰 1회에 대응한다(G11).
        if ticket_path is not None and not boot_busy:
            log.step(STEP.BOOT_TICKET_CONSUME, 0, _consume_dept_ticket(ticket_path))

        # ④-b 리뷰어 감지·무구독 폴백(R1·D-IMPL-1 — 산문 §0 ④-b의 코드 전사): cys boot는 미설치
        # CLI를 건너뛰므로 agy/codex 부재 기계(초보 전원)에서 대체 리뷰어(reviewer-claude-*)를 기동할
        # 주체가 없으면 ⑤ check가 영영 실패한다. 실패=기록만(best-effort) — 최종 게이트는 ⑤ check.
        rev_budget = _budget_derived("boot_reviewers_outer_s", 320)
        _progress("④-b 리뷰어 감지·폴백 기동 중(최대 %ds — 예산 파생: 슬롯 %d × 폴백 %d회)…"
                  % (rev_budget, _budget_leaf("REVIEWER_SLOT_COUNT", 2),
                     _budget_leaf("REVIEWER_FALLBACK_ATTEMPTS", 2)))
        code, out = _run([py, orchestra, "boot-reviewers"], timeout=rev_budget)
        log.step(STEP.BOOT_REVIEWERS, code, out)
        # ★B1: 리뷰어는 Degrade — 비0 이어도 ⑤ 로 계속한다(최종 게이트는 ⑤ check). 단 A12 분류로
        #   '영구 실패(2/127)'만은 처방을 정확히 남긴다(재시도 안내 금지 — 재시도해도 같은 결과).
        if code in (2, 127):
            log.step(STEP.BOOT_REVIEWERS_PERMANENT, code,
                     "boot-reviewers 영구 실패(exit %s — 데몬 다운 또는 스크립트/인터프리터 부재): "
                     "재시도는 무의미하다. `cys ping`·CYS_PACK_DIR·python 해소를 점검하라." % code)
            _progress("⚠ ④-b 영구 실패(exit %s) — 리뷰어는 Degrade 정책이라 ⑤ 로 계속(처방은 로그)" % code)
    else:
        log.step(STEP.RESOURCE_GATE_SKIP, 0,
                 "결손 0(%s) — 자원 게이트 생략(재선언 오탐 hard-block 방지)" % deficit_why)
        # 결손 0 재선언은 스폰이 없으므로 ④ cys boot·④-b 폴백을 호출하지 않고 티켓도 태우지
        # 않는다(향후 실 기동에 재사용). ⑤ check는 유지(생존 재확인).
        log.step(STEP.BOOT_SKIP, 0, "결손 0(구성 충족) — cys boot·④-b 생략(스폰 없음·티켓 미소비)")
        _progress("④ 결손 0(전 구성 생존) — 팀 기동 생략, ⑤ 생존 재확인으로 진행")

    # ⑤ orchestra check — bounded retry(노드 ready는 비동기·check는 스냅샷)
    _progress("⑤ 노드 생존 결정론 확인(check · 최대 %d회×%.0fs 재시도 = %ds 창)…"
              % (CHECK_RETRIES, CHECK_INTERVAL_S, _budget_derived("check_window_s", 120)))
    check_timeout = _budget_leaf("CHECK_SUBPROC_TIMEOUT_S", 60)
    hb_every = max(1, int(_budget_leaf("HEARTBEAT_INTERVAL_S", 20) // max(1, CHECK_INTERVAL_S)))
    # exit 2 '계속' 분기 전용 상한(W-A3 — 키는 아직 javis_budget 에 없어 fallback 실효·키가
    # 생기면 자동으로 그쪽이 SOT) — 상한의 존재 이유는 아래 분기 주석 ⓑ.
    unjudgeable_cap = max(1, int(_budget_leaf("CHECK_UNJUDGEABLE_RETRIES", 3)))
    code, out = 1, "orchestra 부재"
    unjudgeable = None      # 'daemon_gone' | 'interpreter_gone' | None — 즉시 이탈 확정 사유
    unjudgeable_seen = 0    # exit 2 를 '계속'으로 접은 횟수(ping 생존 재확인 성공 시에만 증가)
    for attempt in range(1, CHECK_RETRIES + 1):
        code, out = _run([py, orchestra, "check"], timeout=check_timeout)
        log.step(STEP.CHECK, code, out, suffix="#%d" % attempt)
        if code == 0:
            break
        # ★G32/H-EXIT-7 + W-A3 정밀 분기 — check exit 2 는 '노드 미기동'이 아니라 **판정 불가**다.
        #   그런데 exit 2 하나에 성질이 다른 사건들이 겹친다:
        #     ⓐ cysd 데몬 소실·cys 미설치 (orchestra 가 status 수집 실패 → return 2) — 영구.
        #     ⓑ 일시적 status 수집 실패(데몬 바쁨·RPC 순간 실패) **또는 팩/스크립트 문제** —
        #       `_run([py, orchestra, …])` 는 cmd[0] 이 sys.executable 이라 **orchestra 스크립트
        #       부재에도 python 이 rc 2** 를 낸다(127 이 아니다).
        #   종전 '무조건 즉시 이탈'은 ⓑ 전부를 '데몬 소실'로 오진해 처방(`cys ping`·데몬 기동)을
        #   뒤집었다. 원래 제안 'break 만 제거'는 **기각** — 진짜 데몬 소실(ⓐ)에서 락을 쥔 채
        #   수 분 헛돌면 그동안의 모든 재선언이 exit 11 로 접힌다(치명 앵커 ③ 자가치유 봉쇄).
        #   ★채택 설계: `cys ping` **1회 재확인**으로 ⓐ/ⓑ 를 실측 분리한다(관측 전용·스폰 0 —
        #     check 도 ping 도 아무것도 기동하지 않으므로 재시도가 스폰을 반복 유발하지 않는다·앵커 ①).
        #     - ping 실패 → ⓐ 데몬 소실 확정 → 종전대로 즉시 이탈(정확한 처방으로 실패).
        #     - ping 성공 → ⓑ 데몬 생존 → 재시도 창 안에서 계속하되 이 분기 소모 횟수에 **별도
        #       상한**(unjudgeable_cap)을 둔다 — 상한 없이는 영구 팩 결손이 CHECK_RETRIES 전량
        #       (24회 × 회당 재확인 ping 최대 15s)을 태우며 헛돈다(같은 앵커 ③의 반대쪽 함정).
        if code == 2:
            ping_rc, ping_out = _run(["cys", "ping"], timeout=DAEMON_PROBE_TIMEOUT_S)
            if ping_rc != 0:
                unjudgeable = "daemon_gone"
                log.step(STEP.CHECK, ping_rc,
                         "exit 2 → `cys ping` 재확인 실패(rc=%s) — 데몬 소실 확정·즉시 이탈.\n%s"
                         % (ping_rc, ping_out), suffix="#%d-ping" % attempt)
                break
            unjudgeable_seen += 1
            log.step(STEP.CHECK, 0,
                     "exit 2 → `cys ping` 재확인 생존(rc 0) — 일시 status 수집 실패 또는 "
                     "팩/스크립트 문제로 보고 재시도 계속(%d/%d)"
                     % (unjudgeable_seen, unjudgeable_cap), suffix="#%d-ping" % attempt)
            if unjudgeable_seen >= unjudgeable_cap:
                break       # exit 2 반복 + 데몬 생존 — 루프 밖 code==2 분기가 '팩 결손 가능성' 진단
        elif code == 127:
            # ★사실상 도달 불가 방어선(W-A3 사문 정정): `_run` 이 127 을 내는 유일 경로는
            #   cmd[0]=sys.executable 의 FileNotFoundError — 즉 **파이썬 인터프리터 자체 소실**
            #   이라는 극단뿐이다. 'orchestra 스크립트 부재=127' 이라던 구 문안은 오진이었다
            #   (스크립트 부재는 python rc 2 — 위 exit 2 분기가 실소유자다). 인터프리터 소실은
            #   영구 조건이므로 즉시 이탈은 유지한다.
            unjudgeable = "interpreter_gone"
            break
        if attempt < CHECK_RETRIES:
            if attempt % hb_every == 0:
                # 침묵 창 상쇄(B9 방향 ③) — 진행 하트비트는 stderr(verdict 채널 무오염)
                sys.stderr.write("[bootstrap] ⑤check 재시도 %d/%d 진행 중(노드 기동은 비동기)\n"
                                 % (attempt, CHECK_RETRIES))
                sys.stderr.flush()
            time.sleep(CHECK_INTERVAL_S)
    if code != 0:
        if unjudgeable == "daemon_gone":
            return log.fail(STEP.CHECK_UNJUDGEABLE, code,
                            "check 가 **판정 불가**(exit %s)를 반환했고 `cys ping` 재확인도 "
                            "실패했다 — cysd 데몬 소실·cys 미설치가 확정적이다('노드 미기동'이 "
                            "아니다). 처방: `cys ping` 으로 데몬을 확인·기동하라(`cys boot` 는 "
                            "이 상황의 처방이 아니다).\n%s" % (code, out), EXIT_CHECK)
        if unjudgeable == "interpreter_gone":
            return log.fail(STEP.CHECK_UNJUDGEABLE, code,
                            "check 호출 자체가 불가(exit 127 — 파이썬 인터프리터 %r 소실). "
                            "재시도는 무의미하다 — python 설치·PATH 해소를 복구하라.\n%s"
                            % (py, out), EXIT_CHECK)
        if code == 2:
            # ping 생존인데 exit 2 잔존(별도 상한 도달 또는 창 소진) — 데몬 소실이 아니라
            # **팩 결손 가능성**이다. 근거는 실측으로 붙인다(os.path.isfile — 스크립트 실재
            # 여부가 처방을 가른다 · 결정론 환원: 추론이 아니라 파일계 사실).
            orch_exists = os.path.isfile(orchestra)
            return log.fail(STEP.CHECK_UNJUDGEABLE, code,
                            "check 가 exit 2 를 반복하는데 `cys ping` 은 생존(rc 0)이다 — 데몬 "
                            "소실이 아니라 **팩 결손 가능성**이다. orchestra 스크립트 실재: "
                            "%s(%s). 처방: %s\n%s"
                            % ("있음" if orch_exists else "없음", orchestra,
                               ("스크립트는 실재 — orchestra 내부 status 수집 실패·팩↔바이너리 "
                                "스큐를 점검하라(`cys status --json`·CYS_PACK_DIR)." if orch_exists
                                else "팩 결손 확정적 — CYS_PACK_DIR 배선·팩 재설치로 복구하라"
                                     "(`cys boot`·데몬 재기동은 이 상황의 처방이 아니다)."),
                               out), EXIT_CHECK)
        return log.fail(STEP.CHECK, code,
                        "%d회 재시도 후에도 의무 노드 미기동:\n%s" % (CHECK_RETRIES, out), EXIT_CHECK)

    # ⑥ 완료 마커 — ⑤ exit 0에서만 도달.
    # ★G15/P3-A-DEPT-LANE: **레인 마커**를 쓴다(base 레인은 역사적 경로 = base 마커 그 자체).
    #   부서 레인은 `.master-bootstrapped-<lane>` 에 쓰고 **base 마커는 절대 건드리지 않는다** —
    #   base 마커의 존재가 cys-dept CEO 승격 게이트를 여는 SOT 이므로, 부서장 부트가 그것을 쓰면
    #   게이트가 오개방된다(금지 방향 ① · 이 불변식은 레인화의 전제 조건이다).
    _marker_payload = {
        "pack_version": _pack_version(), "binary_version": _binary_version(),
        "ts": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "surface_ref": os.environ.get("CYS_SURFACE_ID", ""),
        "lane": lane_key(),
        "socket": os.environ.get("CYS_SOCKET", ""), "orchestra_check": "exit 0"}
    _marker_path = lane_state_path("marker")
    _atomic_write_json(_marker_path, _marker_payload)
    if _is_base_socket():
        log.step(STEP.MARKER, 0, _marker_path)
        marker_note = "base 마커 기록"
    else:
        log.step(STEP.MARKER, 0,
                 "레인 마커 기록: %s (base 마커 %s 무접촉 — CEO 게이트 불가침)"
                 % (_marker_path, MARKER))
        marker_note = "부서 레인 마커 기록 — base 마커 무접촉"

    # ⑦ 승격 pending 해소 요청(비대기) — 동의·실제 승격은 부트 밖(배지/feed·차기 lifecycle)
    if _is_base_socket():
        dept = os.path.join(PACK, "bin", "cys-dept")
        if os.path.isfile(dept):
            code, out = _run(["bash", dept, "promote-if-pending", "--request-only"], timeout=30)
            log.step(STEP.PROMOTE_REQUEST, code, out)  # best-effort — 실패해도 부트는 성공
        else:
            log.step(STEP.PROMOTE_REQUEST, 0, "cys-dept 부재 — 생략")
    else:
        # ★T10(P3-2·R3-P03-2): 부서 레인 ⑦ '무조건 생략'을 **신호 발사**로 교체 — 부트와 승격
        #   동의의 분리는 유지한 채(--request-only 는 무변조·알림뿐), 실제 승격(대기형 집행)은
        #   base 데몬의 스케줄 틱(role-less)이 담당한다(신호/집행 분리 — cys-dept 승격 조건 3중·
        #   부트마커 게이트·단일소유 가드·A11 dedupe 전부 무접촉). 게이트 파일 3종(CEO_PENDING·
        #   BOOT_MARKER·REG)은 $HOME 절대경로라 부서 레인 실행도 base 상태를 그대로 읽고,
        #   --request-only 는 단일소유 가드 면제(cys-dept 실측)라 부서 master 세션 호출이 통과한다.
        # ★env 계약(R3-P03-2 치명 결함 봉합): 이 지점의 PACK 은 **부서 팩**이므로 대상 스크립트는
        #   base 팩($HOME/.cys/pack — cys-dept:PACK_DEFAULT 동일 규약)에서 명시 해석하고,
        #   ⓐ CYS_NO_AUTOSTART=1 — base 데몬이 죽어 있는 창에서 cys 클라이언트 autostart 가
        #     부서 env(CYS_PACK_DIR=pack-dept-N)를 상속한 cysd 를 base 소켓에 띄우면 격리·스케줄·
        #     ACL 교차 오염(ensure_daemon_lane_pack 은 base 소켓 방향 무가드). base 사망 중 신호
        #     유실은 설계상 흡수 — 집행자(base 스케줄 틱)가 base 부활 시 자가 치유한다.
        #   ⓑ CYS_SOCKET·CYS_PACK_DIR·CYS_ACCOUNT_DIR 스크럽 — 자식 체인 전체가 base 컨텍스트로만
        #     붙게 한다(이중 방어 — cys-dept 내부 env -u CYS_SOCKET 과 별개 층).
        #   best-effort·fail-open(비0 이어도 부트 성공 무관 — base 분기와 동일 규약).
        base_dept = os.path.join(os.path.expanduser("~"), ".cys", "pack", "bin", "cys-dept")
        if os.path.isfile(base_dept):
            _sig_env = dict(os.environ)
            for _k in ("CYS_SOCKET", "CYS_PACK_DIR", "CYS_ACCOUNT_DIR"):
                _sig_env.pop(_k, None)
            _sig_env["CYS_NO_AUTOSTART"] = "1"
            code, out = _run(["bash", base_dept, "promote-if-pending", "--request-only"],
                             timeout=30, env=_sig_env)
            log.step(STEP.PROMOTE_REQUEST, code,
                     "부서 레인 신호 발사(request-only · env 스크럽: -CYS_SOCKET -CYS_PACK_DIR "
                     "-CYS_ACCOUNT_DIR +CYS_NO_AUTOSTART=1) — %s" % out)
        else:
            log.step(STEP.PROMOTE_REQUEST, 0,
                     "base 팩 cys-dept 부재 — 신호 생략(집행 틱이 base 부활 시 자가 치유)")

    # ⑧ 기계 요약 — master는 이 JSON을 인용해 '기동 완료'를 보고한다(다른 근거 인용 금지)
    summary = {"ok": True, "marker": marker_note,
               "steps": [(s["step"], s["exit"]) for s in log.data["steps"]],
               "lane": log.lane, "boot_last": log.path}
    # ★A7 채널 보존: 완주는 stdout 최종 JSON(구 산문 계약의 유일한 인용 근거)이다.
    log.result(ok=True, state="completed", exit=EXIT_OK)
    print(json.dumps(summary, ensure_ascii=False))
    return EXIT_OK


def cmd_status():
    """레인 스코프 진단 덤프(G15) — **이 레인의** 마커·boot-last 와 경로를 함께 낸다.
    base 마커(CEO 게이트 SOT)는 항상 별도 필드로 보여 부서 레인에서도 게이트 상태를 판독한다."""
    lane = lane_key()
    marker_path = lane_state_path("marker")
    bl_path = lane_state_path("boot_last")
    print(json.dumps({"lane": lane,
                      "marker": _read_json(marker_path), "marker_path": marker_path,
                      "base_marker": _read_json(MARKER), "base_marker_path": MARKER,
                      "boot_last": _read_json(bl_path), "boot_last_path": bl_path,
                      "skip_record": _read_json(lane_state_path("skip")),
                      "base_socket": _is_base_socket()}, ensure_ascii=False, indent=1))
    return 0


def cmd_assert_ready():
    """하위 게이트 전용: 마커 부재/pack_version 불일치 → exit 5.
    stale 정책(설계 §4.1): assert-ready는 버전 대조(ceo_promote 게이트는 존재만 — cys-dept 측)."""
    gate = os.environ.get("CYS_BOOT_GATE", "").strip().lower()
    if gate == "off":
        return 0
    # ★G15: 게이트가 읽는 마커는 **이 레인의** 마커다(base 레인은 역사적 경로 그대로) —
    #   부서 레인이 base 마커를 읽으면 "본부가 떴으니 나도 준비됨"이라는 거짓 통과가 된다.
    m = _read_json(lane_state_path("marker"))
    ok = bool(m) and m.get("pack_version") == _pack_version()
    if ok:
        return 0
    why = "마커 부재" if not m else "pack_version 불일치(%s≠%s) — 재부트 필요" % (
        m.get("pack_version"), _pack_version())
    sys.stderr.write("[bootstrap assert-ready] %s\n" % why)
    return 0 if gate == "warn" else 5


def cmd_self_test():
    """레인 격리 3종 순수 판정 자체검증(orchestra 관례 — assert 배터리 → OK/FAIL).
    결정론·밀폐: env·데몬·파일 무접촉(순수 함수만 호출).
    ★단 하나의 예외(t7 ⓗ): 형제 모듈 `javis_orchestra` import 1회 + `_required_roles_from_orchestra()`
      1회 — 배선이 끊기면 결손 판정이 구 가족 접두 계수로 **조용히** 강등되므로 그 배선만은 실측한다.
      로스터 자체는 detect/agents 주입으로 고정해 PATH·agents.json 내용에 의존하지 않는다."""
    try:
        # ── t1: base/dept 판정 매트릭스(unix base·unix dept·win pipe) ──
        assert _socket_is_base("") is True, "unset=base"
        assert _socket_is_base("/Users/x/.local/state/cys/cys.sock") is True, "unix base"
        assert _socket_is_base("/Users/x/.local/state/cys-dept-dept-1/cys.sock") is False, \
            "unix dept 소켓이 base로 오판(원 버그 — basename cys.sock 동일)"
        assert _socket_is_base("/Users/x/.local/state/cys-dept-ceo/cys.sock") is False, "unix dept(ceo)"
        assert _socket_is_base("\\\\.\\pipe\\cys") is True, "win base pipe(basename 보존)"
        assert _socket_is_base("\\\\.\\pipe\\cys-dept-foo") is False, "win dept pipe"
        # ★보수성 복원(아키텍트 성찰): 커스텀 소켓은 비-base·비-dept — base 특권(마커·티켓 발급) 없음
        assert _socket_is_base("/tmp/whatever.sock") is False, \
            "커스텀 소켓이 base로 오판(과관용 — 마커 write·티켓 발급 특권 오부여)"
        assert _socket_dept("/tmp/whatever.sock") is None, "커스텀 소켓은 비-dept(구동작 보존)"
        assert _socket_is_base("/Users/x/.local/state/cys/cys") is True, "basename cys(무확장)도 base"
        # ★불량 레인(R1-LOW-2): 빈 부서명(cys-dept-/)은 비-base·dept None — malformed로 명시 검출
        assert _socket_is_base("/s/cys-dept-/cys.sock") is False, "빈 부서명이 base로 오판"
        assert _socket_dept("/s/cys-dept-/cys.sock") is None, "빈 부서명 dept가 None 아님"
        assert _socket_malformed_dept("/s/cys-dept-/cys.sock") is True, "빈 부서명 malformed 미검출"
        assert _socket_malformed_dept("/s/cys-dept-dept-1/cys.sock") is False, "정규 부서가 malformed 오판"
        assert _socket_malformed_dept("") is False, "미설정이 malformed 오판"
        assert _socket_malformed_dept("/tmp/whatever.sock") is False, "커스텀 소켓이 malformed 오판"
        assert _socket_malformed_dept("\\\\.\\pipe\\cys-dept-") is True, "win 빈 부서명 pipe 미검출"

        # ── t2: 락 키 유일성(부서 basename 동일 → 전체 경로 유일화) ──
        k1 = _sanitize_sock_key("/Users/x/.local/state/cys-dept-dept-1/cys.sock")
        k2 = _sanitize_sock_key("/Users/x/.local/state/cys-dept-dept-2/cys.sock")
        kb = _sanitize_sock_key("")
        assert k1 != k2, "동일 basename 두 부서 소켓이 같은 락 키(원 버그)"
        assert kb == _sanitize_sock_key("base") == "base", "미설정=base 키"
        assert k1 != kb and k2 != kb, "부서 키가 base 키와 충돌"
        for k in (k1, k2, kb):
            assert k and "/" not in k and os.sep not in k and ":" not in k and "\\" not in k, \
                "락 키에 경로 구분자/공백 잔존: %r" % k
        klong = _sanitize_sock_key("/" + "a" * 400 + "/cys-dept-z/cys.sock")
        assert len(klong) <= 180, "과길이 소켓 키 미절단: %d" % len(klong)
        assert klong == _sanitize_sock_key("/" + "a" * 400 + "/cys-dept-z/cys.sock"), "새니타이즈 비결정론"
        assert klong != _sanitize_sock_key("/" + "b" * 400 + "/cys-dept-z/cys.sock"), "과길이 경로 해시 충돌"
        # ★락 키 base 정규화(R1-LOW-4): env 미설정과 base 경로 명시가 같은 base 데몬에 다른 락을
        # 주던 선재결함 — 싱글플라이트 키는 base면 단일 'base'로 수렴해야 한다.
        assert _singleflight_key("") == "base", "미설정 락 키≠base"
        assert _singleflight_key("/Users/x/.local/state/cys/cys.sock") == "base", \
            "base 경로 명시가 'base' 키로 정규화되지 않음(같은 데몬에 다른 락)"
        assert _singleflight_key("/s/cys-dept-dept-1/cys.sock") != "base", "부서 락 키가 base로 수렴"
        assert _singleflight_key("/tmp/whatever.sock") != "base", "커스텀 소켓 락 키가 base로 수렴"
        assert _singleflight_key("/s/cys-dept-dept-1/cys.sock") != \
            _singleflight_key("/s/cys-dept-dept-2/cys.sock"), "부서 간 락 키 충돌"

        # ── t3: 레인↔팩 정합(부서명 추출 + 불일치 판정) ──
        assert _socket_dept("") is None, "base 소켓 dept=None"
        assert _socket_dept("/s/cys-dept-dept-1/cys.sock") == "dept-1", "부서명 추출"
        assert _socket_dept("/s/cys/cys.sock") is None, "본부 소켓 dept=None"
        assert _pack_dept("/h/.cys/pack") is None, "메인 팩 dept=None"
        assert _pack_dept("/h/.cys/pack-dept-dept-1") == "dept-1", "부서 팩명 추출"
        assert _pack_dept("/h/.cys/pack-dept-dept-1/") == "dept-1", "trailing slash 관용"
        assert _lane_pack_mismatch("", "/h/.cys/pack") is None, "base+메인팩=정합"
        assert _lane_pack_mismatch("/s/cys-dept-dept-1/cys.sock", "/h/.cys/pack-dept-dept-1") is None, \
            "dept-X+pack-dept-X=정합"
        assert _lane_pack_mismatch("/s/cys-dept-dept-1/cys.sock", "/h/.cys/pack") == ("dept-1", None), \
            "dept 소켓+메인 팩=불일치(UT-14)"
        assert _lane_pack_mismatch("", "/h/.cys/pack-dept-dept-2") == (None, "dept-2"), \
            "base 소켓+부서 팩=불일치"
        assert _lane_pack_mismatch("/s/cys-dept-dept-1/cys.sock", "/h/.cys/pack-dept-dept-2") \
            == ("dept-1", "dept-2"), "교차 부서=불일치"

        # ── t4: CEO 티켓 파싱·TTL(증분2 ⓐ 순수 로직) ──
        now = 1_000_000.0
        good = json.dumps({"dept": "dept-1", "issued_at": now - 60, "issuer": "base-master"})
        ok, _ = _parse_ticket_json(good, "dept-1", now)
        assert ok, "유효 티켓(60s 전)이 거부됨"
        expired = json.dumps({"dept": "dept-1", "issued_at": now - TICKET_TTL_SECS - 1})
        ok, why = _parse_ticket_json(expired, "dept-1", now)
        assert not ok and "만료" in why, "TTL 초과 티켓이 유효로 통과: %s" % why
        # TTL 경계(정확히 TTL 경과)는 유효(<=)
        edge = json.dumps({"dept": "dept-1", "issued_at": now - TICKET_TTL_SECS})
        assert _parse_ticket_json(edge, "dept-1", now)[0], "TTL 경계(정확히 24h)가 만료 처리됨"
        wrong_dept = json.dumps({"dept": "dept-2", "issued_at": now})
        assert not _parse_ticket_json(wrong_dept, "dept-1", now)[0], "dept 불일치 티켓이 통과"
        future = json.dumps({"dept": "dept-1", "issued_at": now + 100})
        assert not _parse_ticket_json(future, "dept-1", now)[0], "미래 issued_at(시계 이상)이 통과"
        assert not _parse_ticket_json("{not json", "dept-1", now)[0], "손상 JSON이 유효로 통과"
        assert not _parse_ticket_json(json.dumps({"dept": "dept-1"}), "dept-1", now)[0], \
            "issued_at 부재 티켓이 통과"
        assert not _parse_ticket_json(json.dumps(
            {"dept": "dept-1", "issued_at": True}), "dept-1", now)[0], "bool issued_at이 숫자로 통과"

        # ── t5: 자원 게이트 결정 순수 로직(증분2 ⓑ) ──
        assert _resource_gate_decision(0, None, None)[0] == "allow", "exit 0=allow"
        assert _resource_gate_decision(1, None, None)[0] == "soft", "exit 1=soft"
        srv_hard = {"trips": [{"metric": "servers", "level": "hard", "value": 5}]}
        assert _resource_gate_decision(2, srv_hard, 0)[0] == "hard-block", "servers hard=block"
        # nodes-only hard + 라이브 노드<유효임계 → 과계수 무효화
        nodes_hard = {"trips": [{"metric": "nodes", "level": "hard", "value": 22}],
                      "measured": {"nodes_hard_effective": 18}}
        assert _resource_gate_decision(2, nodes_hard, 5)[0] == "hard-overcount", \
            "nodes 과계수(라이브 5<18) 미무효화"
        # nodes hard인데 라이브 노드가 임계 이상 → genuine block(과계수 아님)
        assert _resource_gate_decision(2, nodes_hard, 20)[0] == "hard-block", \
            "라이브 노드 20>=18인데 과계수로 오무효화"
        # 라이브 노드 측정 불가(None) → 교차확인 불가 → genuine block(보수)
        assert _resource_gate_decision(2, nodes_hard, None)[0] == "hard-block", \
            "라이브 측정 불가 시 보수적 block 아님"
        # nodes+servers 복합 hard → nodes 과계수 무효화 불가(servers는 실자원) → block
        mixed = {"trips": [{"metric": "nodes", "level": "hard", "value": 22},
                           {"metric": "servers", "level": "hard", "value": 5}],
                 "measured": {"nodes_hard_effective": 18}}
        assert _resource_gate_decision(2, mixed, 5)[0] == "hard-block", "복합 hard가 과계수로 오무효화"
        # ★A13(W2): 미지 exit 의 fail-open 제거 — 'allow' 로 접히지 않고 타입으로 분리된다.
        assert _resource_gate_decision(3, None, None)[0] == "unknown-exit", \
            "미지 exit 이 여전히 allow 로 접힘(fail-open 잔존 — 판정불가↔allow 융합)"
        assert _resource_gate_decision(64, None, None)[0] == "usage-error", \
            "EX_USAGE(64)가 사용오류로 분리되지 않음(argparse 충돌·조용한 allow 재발)"
        assert "조용한 allow 아님" in _resource_gate_decision(64, None, None)[1], \
            "EX_USAGE 사유에 loud 계약 문구 누락"
        # 정상 판정 3종은 종전 계약 그대로(무회귀)
        assert _resource_gate_decision(0, None, None)[0] == "allow", "exit 0=allow 회귀"
        assert _resource_gate_decision(1, None, None)[0] == "soft", "exit 1=soft 회귀"
        # ★_run_split 채널 분리(A13 잠복 경로): stderr 오염이 stdout 계약을 파괴하지 않는다.
        _rc, _so, _se = _run_split([sys.executable, "-c",
                                    "import sys;sys.stderr.write('diag\\n');print('{\"verdict\":\"allow\"}')"])
        assert _rc == 0 and _se.strip() == "diag", "_run_split 이 stderr 를 분리하지 못함"
        assert json.loads(_so.strip())["verdict"] == "allow", \
            "stderr 오염이 stdout JSON 계약을 파괴(A13 잠복 경로 잔존)"
        # ★B1 PLAN 정책 소비: --json 계약에서 Fatal 실패만 exit 4 로 승격된다.
        _degraded = json.dumps({"roles": [
            {"role": "cso", "agent": "claude", "outcome": "launched", "mandatory": True},
            {"role": "worker", "agent": "claude", "outcome": "already_alive", "mandatory": True},
            {"role": "reviewer-grok", "agent": "grok", "outcome": "missing", "mandatory": False}]})
        assert _boot_fatal_verdict(1, "진행 산문\n" + _degraded) is None, \
            "Degrade 역할(선택 리뷰어) 실패가 Fatal 로 승격됨(B1 데드엔드 재발)"
        _fatal = json.dumps({"roles": [
            {"role": "cso", "agent": "claude", "outcome": "failed", "mandatory": True,
             "install_hint": "claude 설치"}]})
        assert _boot_fatal_verdict(1, _fatal) is not None, "Fatal 역할 실패가 exit 4 로 승격되지 않음"
        assert "claude 설치" in _boot_fatal_verdict(1, _fatal), "Fatal 사유에 install_hint 미동봉"
        _busy = json.dumps({"roles": [
            {"role": "cso", "agent": "claude", "outcome": "busy", "mandatory": True}],
            "summary": {"lock": "busy"}})
        assert _boot_fatal_verdict(0, _busy) is None, "busy 가 Fatal 실패로 오분류(G11)"
        # --json 미소비(구 바이너리) → 종전 계약으로 보수 폴백(비0=Fatal · fail-open 금지)
        assert _boot_fatal_verdict(1, "구 바이너리 산문 출력") is not None, \
            "--json 소비 불가에서 비0 이 Degrade 로 접힘(진짜 실패 은닉)"
        assert _boot_fatal_verdict(0, "구 바이너리 산문 출력") is None, "exit 0 이 Fatal 로 오판"
        # ★(W4) bare exit 의미 전환 소비 — busy(75)·Fatal(1)·Degrade-only(0) 3분기
        assert CYS_BOOT_EXIT_BUSY == 75, "busy exit 상수 변경(Rust EXIT_BOOT_BUSY 와 파리티 깨짐)"
        assert _boot_was_busy(CYS_BOOT_EXIT_BUSY, _busy) is True, "exit 75 를 busy 로 못 읽음"
        assert _boot_was_busy(0, _busy) is True, "summary.lock=busy 를 busy 로 못 읽음(스큐 축)"
        assert _boot_was_busy(CYS_BOOT_EXIT_BUSY, "파싱 불가 산문") is True, \
            "JSON 파싱 실패 + exit 75 를 busy 로 못 읽음"
        assert _boot_was_busy(0, _degraded) is False, "Degrade-only 를 busy 로 오판(티켓 미소비 회귀)"
        assert _boot_was_busy(1, _fatal) is False, "Fatal 을 busy 로 오판(실패 은닉)"
        # busy 는 Fatal 이 아니다 — 파싱 실패 폴백에서도 exit 75 만은 예외로 통과한다
        assert _boot_fatal_verdict(CYS_BOOT_EXIT_BUSY, "파싱 불가 산문") is None, \
            "exit 75(busy)가 보수 폴백에서 Fatal 로 접힘(중첩 부트 위경보)"
        assert _boot_fatal_verdict(2, "파싱 불가 산문") is not None, \
            "busy 예외가 다른 비0 까지 열어줌(fail-open)"
        # ★(M3-짝 2026-08-24) 관문 보류(gate_pending · exit 78) = **제3 상태**.
        #   성공도 실패(Fatal)도 busy 도 아니다 — 종전엔 어디에도 안 걸려 Degrade 로 흘렀고,
        #   그 가지의 문안이 "Fatal 역할은 전원 확보"(거짓)였다.
        assert CYS_BOOT_EXIT_GATE_PENDING == 78, \
            "관문 보류 exit 상수 이탈(Rust boot_exit_code 78 과 파리티 깨짐)"
        assert CYS_BOOT_EXIT_GATE_PENDING == CYS_LAUNCH_EXIT_GATE_PENDING, \
            "같은 사실(관문 보류)에 값이 둘 — boot/launch-agent 파리티 붕괴"
        assert "gate_pending" not in BOOT_FATAL_OUTCOMES, \
            "gate_pending 이 Fatal 집합에 들어갔다 — 살아 있는 좌석에 회수·파괴 처방(U-11 치명위험 ④)"
        _gp = json.dumps({"roles": [
            {"role": "cso", "agent": "claude", "outcome": "gate_pending", "mandatory": True,
             "reason": "면책 창 상주"}],
            "summary": {"gate_pending": 1}})
        # ⓐ Fatal 도 busy 도 아니다(U-11 계약 · 검체 H-EXIT-11 ⑥ 과 같은 축).
        assert _boot_fatal_verdict(1, _gp) is None, "관문 보류가 Fatal 로 오분류(좌석 회수 처방)"
        assert _boot_was_busy(1, _gp) is False, "관문 보류가 busy 로 오분류(티켓 회계 오염)"
        # ⓑ 그러나 **반드시 잡힌다** — 이것이 M3-짝 의 수리 지점이다.
        _gp_why = _boot_gate_pending_verdict(1, _gp)
        assert _gp_why is not None, "관문 보류가 어디에도 걸리지 않는다(Degrade 로 흘러 거짓 문장)"
        assert "관문" in _gp_why and "No, exit" in _gp_why and "회수" in _gp_why, \
            "보류 사유에 관문 통과 처방·면책 창 경고·비파괴 지시가 없다"
        assert "면책 창 상주" in _gp_why, "생산자 reason 이 소비부에서 버려진다(처방 소실)"
        # ⓒ 두 축 OR — exit 만 있어도(JSON 파싱 불가) 놓치지 않는다.
        assert _boot_gate_pending_verdict(CYS_BOOT_EXIT_GATE_PENDING, "파싱 불가 산문") is not None, \
            "exit 78 단독(파싱 불가)에서 보류를 놓친다"
        assert _boot_fatal_verdict(CYS_BOOT_EXIT_GATE_PENDING, "파싱 불가 산문") is None, \
            "exit 78 이 보수 폴백에서 Fatal 로 접힌다 — 살아 있는 좌석 회수 처방(U-11 위반)"
        assert _boot_was_busy(CYS_BOOT_EXIT_GATE_PENDING, "파싱 불가 산문") is False, \
            "exit 78 을 busy(무스폰)로 오판 — 티켓이 무스폰 소각되거나 보류가 은닉된다"
        # ⓓ 과잉 발화 금지 — 선택 역할 보류·정상 성공·진짜 실패는 이 분기가 아니다.
        _gp_opt = json.dumps({"roles": [
            {"role": "reviewer-codex", "agent": "codex", "outcome": "gate_pending",
             "mandatory": False}]})
        assert _boot_gate_pending_verdict(1, _gp_opt) is None, \
            "선택 역할 보류까지 제3 상태로 승격(Degrade 가 옳다)"
        assert _boot_gate_pending_verdict(0, _degraded) is None, "정상 Degrade 를 보류로 오판"
        assert _boot_gate_pending_verdict(1, _fatal) is None, "진짜 실패를 보류로 오판(실패 은닉)"
        assert _boot_gate_pending_verdict(CYS_BOOT_EXIT_BUSY, _busy) is None, \
            "busy 를 보류로 오판(무스폰 회계 오염)"
        # ⓔ 선언 순서 = 실행 순서: 보류 단계는 busy 뒤·Degrade 앞이다.
        assert (STEP_INDEX[STEP.BOOT_BUSY] < STEP_INDEX[STEP.BOOT_GATE_PENDING]
                < STEP_INDEX[STEP.BOOT_DEGRADE]), "④ 보류 단계 선언 순서 이탈"
        # ★결손 판정 ↔ check verdict 공유(H-PRED-1): 같은 status fixture 에서 판정이 갈리지 않는다.
        _healthy = {"surfaces": [
            {"role": "cso", "exited": False, "awakened_at": 1.0},
            {"role": "worker-2", "exited": False, "status": {"age_secs": 3, "state": "working"}},
            {"role": "reviewer-gemini", "exited": False, "agent_alive": True},
            {"role": "reviewer-codex", "exited": False, "agent_alive": True}]}
        # ★밀폐 주입(2026-08-22 적대검증 중대④): 로스터를 **감지 결과가 아니라 테스트 상수**로
        #   고정한다. 종전엔 실감지를 타서 agy·codex 미설치 기계(신규 사용자 대다수)에서
        #   reviewer-claude-* 대체 좌석이 required 에 들어오고 합성 status 에는 그 좌석이 없어
        #   **항상 결손>0** → 이 단언이 FAIL → 뒤따르는 t9b(티켓 자동 요청 커버리지)가 통째로
        #   실행되지 않았다. `javis_orchestra.check_verdicts`/`_shared_verdict_deficit` 이
        #   이미 갖고 있는 주입 규약을 그대로 쓴다(미주입=실감지 — 프로덕션 거동 불변).
        _yes = lambda ag, agents=None: (True, "테스트 주입")   # noqa: E731
        _synth = {"gemini": {"cmd": "/x/agy"}, "codex": {"cmd": "/x/codex"},
                  "claude": {"cmd": "claude"}}
        _has, _why = _shared_verdict_deficit(_healthy, detect=_yes, agents=_synth)
        assert _has is False, "건강한 팀(생존추정 포함)을 결손>0 으로 오판: %s" % _why
        _grok_only = {"surfaces": [
            {"role": "reviewer-grok", "exited": False, "agent_alive": True},
            {"role": "cso-1", "exited": False, "agent_alive": True}]}
        _has2, _why2 = _shared_verdict_deficit(_grok_only, detect=_yes, agents=_synth)
        assert _has2 is True, "grok·cso-1 좌석이 의무 슬롯을 채운 것으로 계상(G26 재발): %s" % _why2
        # ★중대④ 회귀 핀: 주입 경로가 정본까지 **실제로 뚫려 있는가**. 막히면 깨끗한 기계에서
        #   이 self-test 가 통째로 red 가 되고(위 단언이 먼저 죽는다) 뒤 커버리지가 안 돈다.
        import inspect as _insp
        try:
            import javis_orchestra as _orch_pin
            _sig = _insp.signature(_orch_pin._shared_verdict_deficit).parameters
            assert "detect" in _sig and "agents" in _sig, \
                "orchestra 정본이 detect/agents 주입을 받지 않는다 — 형제 소비자 밀폐 붕괴(중대④ 재발)"
        except ImportError:
            pass                                  # 구 팩 스큐 — 위임 래퍼가 폴백으로 흡수한다
        assert "detect" in _insp.signature(_shared_verdict_deficit).parameters, \
            "위임 래퍼가 주입을 전달하지 않는다(중대④ 재발)"

        # ── t6: 결손 구성 판정(R1-MED-1 순수 로직 — 총수 비교 폐기) ──
        full = {"cso": 1, "worker": 1, "reviewer": 2}
        assert _team_composition_deficit(full)[0] is False, "완전 구성이 결손으로 오판"
        assert _team_composition_deficit({"cso": 2, "worker": 3, "reviewer": 5})[0] is False, \
            "초과 구성이 결손으로 오판"
        # ★네거티브(원 결함): reviewer만 4 — 구 총수 4 비교는 결손 0으로 오판했다
        half, why = _team_composition_deficit({"cso": 0, "worker": 0, "reviewer": 4})
        assert half is True, "반쪽 팀(reviewer만 4)이 결손 0으로 오판(총수 비교 잔재)"
        assert "cso" in why and "worker" in why, "결손 사유에 결손 역할 미명시: %s" % why
        assert _team_composition_deficit({"cso": 1, "worker": 1, "reviewer": 1})[0] is True, \
            "reviewer 1(<2)이 결손 0으로 오판"
        assert _team_composition_deficit({})[0] is True, "빈 카운트가 결손 0으로 오판"

        # ── t7: 로스터 결손 판정(W0 P0 지혈 — G26 + A1 '결손 0 오판' 절반) ──
        # ★신구 계수 판정을 같은 검체로 **차분**한다: 구 판정(_family_counts+_team_composition_deficit)
        #   =가족 접두 계수, 신 판정(_team_roster_deficit)=⑤check와 동일한 역할 이름공간.
        #   차분이 갈리는 검체가 바로 G26이 먹였던 라이브락 입력이다.
        def _old(roles):
            return _team_composition_deficit(_family_counts(roles))[0]

        def _new(roles, required=("cso", "worker", "reviewer-gemini", "reviewer-codex")):
            return _team_roster_deficit(list(required), set(roles))[0]

        # ⓐ reviewer-grok 좌석이 의무 리뷰어 슬롯을 대신 채운 검체 — 구=결손 0(오판), 신=결손 존재
        grok_seat = ["cso", "worker", "reviewer-gemini", "reviewer-grok"]
        assert _old(grok_seat) is False, "검체 전제 붕괴: 구 판정이 grok 좌석을 이미 결손으로 봄"
        assert _new(grok_seat) is True, \
            "reviewer-grok(선택 좌석)이 의무 reviewer-codex 슬롯을 채운 것으로 계상(G26 미수리)"
        assert "reviewer-codex" in _team_roster_deficit(
            ["cso", "worker", "reviewer-gemini", "reviewer-codex"], set(grok_seat))[1], \
            "결손 사유에 부재 역할(reviewer-codex) 미명시"
        # ⓑ cso-1 변형 좌석 — 구=결손 0(오판: 접두 귀속), 신=결손 존재(check는 정확일치 'cso' 요구)
        cso_variant = ["cso-1", "worker", "reviewer-gemini", "reviewer-codex"]
        assert _old(cso_variant) is False, "검체 전제 붕괴: 구 판정이 cso-1을 이미 결손으로 봄"
        assert _new(cso_variant) is True, \
            "cso-1 변형 좌석이 정확일치 'cso' 의무를 충족한 것으로 계상(G26 미수리)"
        # ⓒ 정상 5노드(의무 4 + 선택 grok) — 구·신 **모두** 결손 0(역방향 회귀 금지 핀)
        healthy5 = ["cso", "worker", "reviewer-gemini", "reviewer-codex", "reviewer-grok"]
        assert _old(healthy5) is False and _new(healthy5) is False, \
            "정상 5노드가 결손>0으로 오판(재선언 오탐 hard-block 부활 — 역방향 회귀)"
        # ⓓ 결손 1(reviewer-codex 부재) — 구·신 모두 결손 존재(합치 검체)
        one_missing = ["cso", "worker", "reviewer-gemini"]
        assert _old(one_missing) is True and _new(one_missing) is True, \
            "결손 1 검체에서 신구 판정 불일치"
        # ⓔ worker-N 접두 수용 — cmd_check(orchestra.py:239-241)와 동일 규약 유지(역방향 회귀 금지)
        worker_n = ["cso", "worker-2", "reviewer-gemini", "reviewer-codex"]
        assert _new(worker_n) is False, "worker-2가 'worker' 의무를 못 채움(check 규약 이탈·역방향 회귀)"
        # ⓕ 대체 로스터(agy/codex 미감지 → reviewer-claude-1/2)도 좌석이 맞으면 결손 0
        subst = ("cso", "worker", "reviewer-claude-1", "reviewer-claude-2")
        assert _new(["cso", "worker", "reviewer-claude-1", "reviewer-claude-2"], subst) is False, \
            "대체 리뷰어 좌석이 대체 로스터 의무를 못 채움"
        assert _new(["cso", "worker", "reviewer-gemini", "reviewer-codex"], subst) is True, \
            "대체 로스터 요구를 네이티브 좌석이 대신 채운 것으로 계상(이름공간 뭉갬 잔재)"
        # ⓖ 빈 라이브·부분 좌석 — fail-safe 방향(결손 존재)
        assert _new([]) is True, "라이브 0이 결손 0으로 오판"
        assert _new(["cso"]) is True, "cso 단독이 결손 0으로 오판"
        # ⓗ orchestra 소비 배선 — 형제 import가 실제로 해소되는가(해소 실패=구 판정으로 조용히 강등).
        #    ★밀폐: 로스터는 detect/agents **주입**으로 고정한다(PATH·agents.json 무접촉).
        #      전수 런타임 import 증명은 tests/test_import_guard.py ③이 담당한다.
        try:
            import javis_orchestra as _orch_st
        except Exception:                                             # pragma: no cover
            _orch_st = None       # 팩 스큐·부서 팩 결손 — 폴백 계약은 아래 반환 계약 assert가 지킨다
        if _orch_st is not None:
            nat = _orch_st.effective_required_roles(
                detect=lambda a, g=None: (True, "stub-installed"), agents={})
            sub = _orch_st.effective_required_roles(
                detect=lambda a, g=None: (False, "stub-missing"), agents={})
            assert nat == ["cso", "worker", "reviewer-gemini", "reviewer-codex"], \
                "네이티브 로스터 계약 이탈: %r" % (nat,)
            assert sub == ["cso", "worker", "reviewer-claude-1", "reviewer-claude-2"], \
                "대체 로스터 계약 이탈: %r" % (sub,)
            # 신 판정이 두 로스터를 각각 그대로 소비하는지(이름공간 결박 확인)
            assert _team_roster_deficit(nat, set(healthy5))[0] is False, "네이티브 로스터 결박 실패"
            assert _team_roster_deficit(sub, set(healthy5))[0] is True, \
                "대체 로스터 요구인데 네이티브 좌석으로 결손 0(이름공간 미결박)"
        # 반환 계약(roles XOR 사유) — 반환 이상은 폴백으로 강등하고 crash하지 않는다
        req, why_no = _required_roles_from_orchestra()
        assert (req is None) != (why_no is None), \
            "_required_roles_from_orchestra 반환 계약 위반(roles/사유 동시 유효 또는 동시 부재)"
        if req is not None:
            assert "cso" in req and "worker" in req and len(req) >= 4, \
                "orchestra 의무 역할 목록이 cso·worker+리뷰어 2를 포함하지 않음: %r" % (req,)
        # ⓘ _role_satisfied 순수 규약 — worker만 접두, 그 밖은 정확일치
        assert _role_satisfied("worker", {"worker-3"}) is True, "worker 접두 수용 소실"
        assert _role_satisfied("cso", {"cso-1"}) is False, "cso 접두 관용 잔재(check 규약 이탈)"
        assert _role_satisfied("reviewer-codex", {"reviewer-codex-2"}) is False, \
            "리뷰어 접두 관용 잔재(check 규약 이탈)"
        assert _role_satisfied("reviewer-gemini", {"reviewer-gemini"}) is True, "정확일치 실패"

        # ── t8: 레인 스코프 상태 경로(G15 · P3-A-DEPT-LANE) ──
        _base_sock = "/Users/x/.local/state/cys/cys.sock"
        _dept_sock = "/Users/x/.local/state/cys-dept-dept-1/cys.sock"
        assert lane_key(_base_sock) == "base", "base 레인 키 이탈"
        assert lane_key("") == "base", "env 미설정=base 레인"
        assert lane_key(_dept_sock) != "base", "부서 레인 키가 base로 수렴"
        assert lane_key("/Users/x/.local/state/cys-dept-dept-2/cys.sock") != lane_key(_dept_sock), \
            "부서 간 레인 키 충돌(상태 파일 공유 재발)"
        # ⓐ base 레인은 **역사적 경로**를 그대로 쓴다(§0 산문·GUI·테스트 호환)
        assert lane_state_path("marker", _base_sock) == MARKER, "base 마커 경로가 변경됨(회귀)"
        assert lane_state_path("boot_last", _base_sock) == BOOT_LAST, "base boot-last 경로 변경(회귀)"
        # ⓑ 부서 레인은 분리된다 — 그리고 **base 마커를 절대 가리키지 않는다**(CEO 게이트 불가침)
        _dm = lane_state_path("marker", _dept_sock)
        _dbl = lane_state_path("boot_last", _dept_sock)
        assert _dm != MARKER, "부서 레인이 base 마커를 가리킨다(CEO 승격 게이트 오개방 — 금지 방향 ①)"
        assert _dbl != BOOT_LAST, "부서 레인이 base boot-last 를 공유한다(G15 미수리)"
        assert os.path.basename(_dm).startswith(".master-bootstrapped-"), "부서 마커 명명 규약 이탈"
        assert os.path.basename(_dbl).startswith("boot-last-"), "부서 boot-last 명명 규약 이탈"
        assert lane_state_path("marker", "/Users/x/.local/state/cys-dept-dept-2/cys.sock") != _dm, \
            "부서 간 마커 충돌(레인 오염)"
        # ⓒ skip·lock 은 base 에서도 레인 접미(구 동작 보존)
        assert lane_state_path("lock", _base_sock).endswith("bootstrap-base.lock"), "base 락 경로 변경"
        assert lane_state_path("skip", _base_sock).endswith("boot-skip-base.json"), "base skip 경로 변경"
        # ⓓ ★R1 배달 원장 — Rust 생산자(src/bin/cysd/delivery.rs)와 **파일명이 정확히** 같아야
        #    한다. 갈리면 훅이 빈 원장을 읽어 기계 push 를 오너 임무로 기록한다(사고 재현).
        assert lane_state_path("delivery", _base_sock).endswith("delivery-base.jsonl"), \
            "배달 원장 base 파일명 규약 이탈(cysd delivery::ledger_path 와 불일치)"
        assert lane_state_path("delivery_epoch", _base_sock).endswith("delivery-base.epoch.json"), \
            "데몬 표식 base 파일명 규약 이탈(cysd delivery::epoch_path 와 불일치)"
        assert lane_state_path("delivery", _dept_sock) != lane_state_path("delivery", _base_sock), \
            "부서 레인 배달 원장이 base 와 섞인다(교차 레인 오판)"
        assert os.path.dirname(lane_state_path("delivery", _base_sock)) == state_dir(), \
            "배달 원장이 팩 계약 상태 디렉터리(CYS_STATE_DIR‖~/.cys/state) 밖에 있다"
        try:
            lane_state_path("nope")
            raise AssertionError("미지 상태 종류가 조용히 통과(오타가 새 파일을 만든다)")
        except ValueError:
            pass

        # ── t8b: 폴백 전제 판정(부서 자동 생성의 유일한 전제 — 2026-08-16) ──
        # 이 판정이 틀리면 **없는 master 로 부서가 생기거나**(거짓 양성) **정당한 부서 창설이
        # 막힌다**(거짓 음성). 비가역 스폰의 게이트라 순수 함수로 분리하고 여기서 박제한다.
        _S = lambda *rows: {"surfaces": list(rows)}                       # noqa: E731
        _m = lambda sid, role="master", ex=False: {"surface_id": sid, "role": role, "exited": ex}  # noqa: E731
        for _st, _sid, _want, _why in (
            (_S(), "7", False, "빈 로스터"),
            (_S(_m(7)), "7", False, "자기 자신만 보유(멱등 재claim) = '남의 보유' 아님"),
            (_S(_m(9)), "7", True, "타 surface 가 살아있는 master"),
            (_S(_m(9, ex=True)), "7", False, "exited master 는 보유자가 아니다"),
            (_S(_m(9, role="worker-1")), "7", False, "master 아닌 역할은 무관"),
            (_S(_m(7), _m(9)), "7", True, "자기+타 surface 혼재 → 타 surface 가 보유"),
            (None, "7", None, "status 판독 불가 = 알 수 없음(없음으로 접지 않는다)"),
            ({"surfaces": "nope"}, "7", None, "스키마 불일치 = 알 수 없음"),
        ):
            _got, _reason = _live_master_from_status(_st, _sid)
            assert _got is _want, "폴백 전제 판정 오류(%s): got=%r want=%r · %s" % (
                _why, _got, _want, _reason)

        # ── t9: 단계 정체성 레지스트리(P3-A-STEP-NAME · H-LIFE-2) ──
        assert len(set(STEP_ORDER)) == len(STEP_ORDER), \
            "단계 라벨 중복(동명이의 재사용 재발): %r" % (
                [l for l in STEP_ORDER if STEP_ORDER.count(l) > 1],)
        _names = [n for n, _ in _STEP_DEFS]
        assert len(set(_names)) == len(_names), "단계 상수명 중복"
        # ⓐ **호출부 리터럴 0** — 전 기록 지점이 레지스트리를 경유한다(드리프트 원천 차단)
        _src = open(os.path.abspath(__file__), encoding="utf-8").read()
        _lit = re.findall(r'log\.(?:step|fail)\(\s*"', _src)
        assert not _lit, "단계 기록에 문자열 리터럴 잔존 %d건(레지스트리 미경유)" % len(_lit)
        # ⓑ 서수 ↔ 실행순서: 레인 가드는 ①preflight **앞**, 티켓 소비는 ④boot **뒤**
        assert STEP_INDEX[STEP.LANE_MALFORMED] < STEP_INDEX[STEP.PREFLIGHT], \
            "레인 가드가 preflight 뒤로 선언됨(서수-실행순서 불일치 재발)"
        assert STEP_INDEX[STEP.LANE_PACK] < STEP_INDEX[STEP.PREFLIGHT], "레인↔팩 가드 순서 이탈"
        assert STEP_INDEX[STEP.BOOT] < STEP_INDEX[STEP.BOOT_TICKET_CONSUME] \
            < STEP_INDEX[STEP.BOOT_REVIEWERS], "티켓 소비가 ④boot~④-b 사이가 아니다"
        assert STEP_INDEX[STEP.RESOURCE_GATE] < STEP_INDEX[STEP.BOOT], "자원 게이트가 ④boot 뒤"
        assert STEP_INDEX[STEP.CHECK] < STEP_INDEX[STEP.MARKER] < STEP_INDEX[STEP.PROMOTE_REQUEST], \
            "⑤check→⑥marker→⑦promote 순서 이탈"
        # ⓑ′ 폴백 전제 가드는 ③-d 진입(DEPT_FB) **뒤**로 선언한다 — 앞으로 선언하면 기록이
        #    역행해 boot-last 가 매 발동마다 order_violation 을 남긴다(2026-08-16 자기파손 수리).
        assert STEP_INDEX[STEP.DEPT_FB] < STEP_INDEX[STEP.DEPT_FB_GUARD] \
            < STEP_INDEX[STEP.DEPT_FB_ALLOC], "③-d 전제 가드 서수가 진입~allocate 사이가 아니다"
        # ⓒ 동명이의였던 3쌍이 이제 서로 다른 라벨이다
        for _a, _b in ((STEP.LANE_MALFORMED, STEP.LANE_PACK),
                       (STEP.RESOURCE_GATE, STEP.RESOURCE_GATE_SKIP),
                       (STEP.RESOURCE_GATE, STEP.RESOURCE_GATE_ABSENT)):
            assert _a != _b, "동명이의 잔존: %r" % (_a,)
        # ⓓ ★티켓 자동 요청(2026-08-22 결함 #2): 감지 → 요청 → 대기 → 단독각성 고지 순서.
        #    역순으로 선언하면 boot-last 가 매 부트마다 order_violation 을 남긴다(계측 자기파손).
        assert STEP_INDEX[STEP.CEO_TICKET] < STEP_INDEX[STEP.CEO_TICKET_REQUEST] \
            < STEP_INDEX[STEP.CEO_TICKET_WAIT] < STEP_INDEX[STEP.CEO_TICKET_SOLO] \
            < STEP_INDEX[STEP.BOOT], "③″ 티켓 요청·대기 서수가 감지~④boot 사이가 아니다"

        # ── t9b: 부서 CEO 티켓 자동 요청(2026-08-22 결함 #2 · fail-open 불변) ──
        _tq_dept = "selftest-dept"
        # ⓐ 요청 명령 조립 — `--queued`(데몬이 Return 주입) · 역할 주소 master · 발급 명령 동봉
        _tq_cmd = _dept_ticket_request_cmd(_tq_dept)
        assert _tq_cmd[:5] == ["cys", "send", "--queued", "--to", "master"], \
            "요청 push 명령 계약 이탈(--queued/--to master): %r" % (_tq_cmd,)
        assert "issue-ticket --dept %s" % _tq_dept in _tq_cmd[5], \
            "요청 문안에 발급 명령이 없다(수신자가 무엇을 해야 하는지 모른다): %r" % _tq_cmd[5]
        # ⓐ′ ★부서명 규약 비대칭(2026-08-22 적대검증 중대③) — 생성기와 **같은 집합**인가.
        #     리뷰어 재현 입력 그대로: 오너가 `Sales`·`dept_1` 로 만든 부서는 종전 발급 정규식
        #     (`[a-z0-9][a-z0-9-]*`)이 거부해 **티켓을 영영 못 받았다**(결함 #1 잔존).
        for _n in ("Sales", "dept_1", "dept-3", "a", "A9_x-y"):
            assert dept_name_ok(_n), "생성기가 만드는 이름 %r 을 발급기가 거부한다(중대③ 재발)" % _n
        for _n in ("", "-lead", "_x", "a/b", "a.b", "a b", "a" * 41):
            assert not dept_name_ok(_n), "경로·형식 위험 이름 %r 을 발급기가 수용한다" % _n
        # ★생성기(cys-dept)와의 **동작 대조**(2026-08-22 적대검증 2회차 중대6).
        #   종전은 정규식 **문자열 리터럴 대조**였다 — 그건 원리적으로 의미론 차이를 못 잡는다.
        #   실제로 두 구현은 문자 집합이 같은데도 **개행 축에서 갈렸다**: shell 쪽 `grep -Eq` 는
        #   **줄 단위**라 `$'abc\nrm -rf /'` 처럼 **어느 한 줄만** 맞으면 성립하고, python 쪽
        #   `fullmatch` 는 문자열 전체를 요구한다. 이름은 경로에 들어간다(`<dept>.ticket`·
        #   `cys-dept-$name`)므로 관대한 쪽이 위험하다.
        #   → 그래서 `cys-dept` 의 `dept_name_ok` 를 **실제로 실행**해 같은 코퍼스로 대조한다.
        _selftest_dept_name_parity()
        # ⓐ″ ★실패할 명령을 CEO 큐에 넣지 않는다(중대③ 후반) — 불량 이름은 요청 **미발사**
        _bad_req, _bad_why = _request_dept_ticket("Bad/Name")
        assert _bad_req is False and "발급 규약" in _bad_why, \
            "발급기가 거부할 이름인데 CEO 큐에 요청을 넣었다(600s 주기 소음): %s" % _bad_why
        # ⓑ base 레인 env — 부서 소켓 상속 제거(`env -u CYS_SOCKET` 동형)
        _tq_env_backup = {k: os.environ.get(k) for k in
                          ("CYS_SOCKET", "CYS_SURFACE_ID", "CYS_SURFACE_REF")}
        try:
            os.environ["CYS_SOCKET"] = "/x/cys-dept-%s/cys.sock" % _tq_dept
            os.environ["CYS_SURFACE_ID"] = "77"
            _tq_env = _base_lane_env()
            assert "CYS_SOCKET" not in _tq_env, "요청 push 가 부서 소켓을 물고 나간다(base 미도달)"
            assert "CYS_SURFACE_ID" not in _tq_env, "부서 surface id 가 base 레인으로 새어 나간다"
        finally:
            for _k, _v in _tq_env_backup.items():
                if _v is None:
                    os.environ.pop(_k, None)
                else:
                    os.environ[_k] = _v
        # ⓒ 멱등 억제 — TTL 안 = 억제 / TTL 밖 = **재요청**(영구 침묵 금지) / 마커 없음 = 요청
        #    ★밀폐: 요청 마커 디렉터리를 임시 경로로 갈아 끼운다(사용자 실 상태 무접촉).
        _tq_now = 1_700_000_000.0
        _tq_dir_backup = DEPT_TICKET_REQ_DIR
        _tq_tmp = tempfile.mkdtemp(prefix="boot-tq-")
        try:
            globals()["DEPT_TICKET_REQ_DIR"] = _tq_tmp
            _tq_path = _dept_ticket_request_path(_tq_dept)
            assert _tq_path.startswith(_tq_tmp), "밀폐 붕괴: 요청 마커 경로가 임시 밖이다"
            assert _dept_ticket_request_suppressed(_tq_dept, now=_tq_now)[0] is False, \
                "마커가 없는데 요청이 억제됐다(첫 요청이 영영 안 나간다)"
            _atomic_write_json(_tq_path, {"dept": _tq_dept, "requested_at": _tq_now})
            assert _dept_ticket_request_suppressed(_tq_dept, now=_tq_now + 1)[0] is True, \
                "TTL 안 재요청이 억제되지 않는다(CEO 큐 스팸)"
            assert _dept_ticket_request_suppressed(
                _tq_dept, now=_tq_now + DEPT_TICKET_REQUEST_TTL_S + 1)[0] is False, \
                "TTL 경과 후에도 억제된다(영구 침묵 — 부서가 팀을 영영 못 받는다)"
            _atomic_write_json(_tq_path, {"dept": _tq_dept, "requested_at": "망가진 값"})
            assert _dept_ticket_request_suppressed(_tq_dept, now=_tq_now)[0] is False, \
                "손상 마커가 요청을 억제한다(값을 망가뜨리면 침묵시킬 수 있다)"
        finally:
            globals()["DEPT_TICKET_REQ_DIR"] = _tq_dir_backup
            import shutil as _shutil
            _shutil.rmtree(_tq_tmp, ignore_errors=True)
        # ⓓ 유계 대기 — 티켓 없음이면 예산 안에서 끝난다(무기한 대기 = 락 보유 연장·치명)
        _tq_slept = []
        _tq_clock = [0.0]

        def _tq_sleep(sec):
            _tq_slept.append(sec)
            _tq_clock[0] += sec

        _tq_ok, _tq_why, _ = _await_dept_ticket(
            _tq_dept, budget_s=9, interval_s=3,
            sleeper=_tq_sleep, clock=lambda: _tq_clock[0])
        assert _tq_ok is False, "없는 티켓을 도착으로 판정했다: %s" % _tq_why
        assert sum(_tq_slept) <= 9 + 1e-6, "유계 대기가 예산을 넘겼다(합 %s)" % sum(_tq_slept)
        assert len(_tq_slept) >= 1, "폴링 없이 즉시 포기했다(요청 직후 도착을 못 받는다)"
        # 예산 0 이어도 **첫 조회는 한다**(이미 도착한 티켓을 놓치지 않는다)
        _tq_slept2 = []
        _await_dept_ticket(_tq_dept, budget_s=0, interval_s=3,
                           sleeper=lambda s: _tq_slept2.append(s), clock=lambda: 0.0)
        assert not _tq_slept2, "예산 0 인데 잠들었다(부트 시간 낭비)"

        # ── t10: TCC 탐침 대상이 실자원 파생인가(P3-A-TCC · H-PRED-10) ──
        _t = _tcc_probe_targets()
        if sys.platform == "darwin":
            _paths = [p for p, _ in _t]
            assert _paths, "darwin 에서 TCC 탐침 대상이 비었다(탐침 소멸)"
            # ★부재 팩 예외(2026-08-22): 이 단언은 "부재 경로는 탐침 대상에서 제외한다"는
            #   `_tcc_probe_targets` 계약(및 바로 아래 isdir 단언)과 **정면으로 모순**이었다 —
            #   팩이 아직 없는 격리 HOME(빈 HOME self-test·신규 설치 직전)에서 무조건 FAIL 했다.
            #   실재할 때만 대상 포함을 요구한다(계약 위반 탐지력은 그대로).
            if os.path.isdir(PACK):
                assert os.path.realpath(PACK) in _paths, "팩(실자원)이 탐침 대상에 없다"
            assert all(os.path.isdir(p) for p in _paths), "부재 경로가 탐침 대상에 남았다"
            # 하드코딩된 Desktop 이 아니다 — Desktop 이 cwd·PACK 일 때만 우연히 포함될 수 있다
            _desk = os.path.realpath(os.path.join(HOME, "Desktop"))
            assert _desk not in _paths or _desk in (os.path.realpath(os.getcwd()),
                                                   os.path.realpath(PACK)), \
                "Desktop 하드코딩 잔존(P3-A-TCC 미수리)"
            assert all(isinstance(l, str) and l for _p, l in _t), "탐침 라벨 결손"
        else:
            assert _t == [], "비-darwin 에서 TCC 탐침이 동작한다(무동작 계약 위반)"
    except AssertionError as e:
        print("javis_bootstrap self-test FAIL: %s" % e, file=sys.stderr)
        return 1
    print("javis_bootstrap self-test OK (★결함#2: CEO 티켓 자동 요청(명령 조립·base env 소켓 "
          "격리·멱등 TTL 4종·유계 대기 2종·단계 서수) · "
          "★중대③ 부서명 규약 단일화(수용 5·거부 7·불량명 요청 미발사) + ★중대⑥ cys-dept "
          "**동작 대조** 23종(수용 8·거부 10·★개행 축 5종 — 리터럴 대조 폐기 · "
          "비대칭 0 + 절대 기대값 양축 · ★대조 무력화=hard fail(파일 부재만 NOTE 강등 — "
          "무성 강등 제거)) · "
          "★중대④ 로스터 밀폐 주입 관통(정본·래퍼 시그니처 핀 — 깨끗한 HOME green) · "
          "W6: 폴백 전제 판정 8종(부서 자동 생성 게이트) · "
          "W4: TCC 탐침 실자원 파생 · W3: 레인 상태 경로 12종 + 단계 레지스트리 9종 · "
          "레인 격리 3종 + 부서 교리 게이트 2종 + 결손 구성 판정 + "
          "로스터 결손 신구 차분 9종 + W2(A13 타입드 게이트 exit·_run_split 채널분리·B1 PLAN 정책 "
          "소비 6종·공유 판정 결손 2종) — base/dept 판정·불량 레인·락 키·레인↔팩·CEO 티켓 TTL·"
          "자원 게이트 결정·구성 결손·grok 좌석·cso-1 좌석·정상 5노드·결손 1·worker-N·대체 로스터)")
    return 0


def cmd_lane_path(argv):
    """`lane-path [kind]` — 이 레인의 상태 파일 경로를 stdout 한 줄로(훅·산문 안내의 단일 출처).

    ★왜 서브커맨드인가(G15 소비처 통일): 훅 note·§0 산문이 `~/.cys/state/boot-last.json` 을
      **하드코딩**하면 부서 레인에서 거짓 경로를 안내한다(그 레인의 진단은 다른 파일에 있다).
      경로 규약의 소유자는 `lane_state_path` 하나이고, 소비자는 이 명령으로 물어본다.
    인자 없음 = boot_last. 미지 종류는 EX_USAGE(64) 거부(오타가 새 파일을 만들지 않는다)."""
    kind = (argv[0] if argv else "boot_last").replace("-", "_")
    if kind == "all":
        out = {k: lane_state_path(k) for k in sorted(_LANE_STATE_KINDS)}
        out.update({"lane": lane_key(), "base_marker": MARKER})   # dict | 는 3.9+ 전용 — 회피
        # ★U-24 관측(침묵 폴백 차단): 이 레인이 **정본 `javis_lane`** 을 쓰는가, 아니면
        #   레거시 인라인으로 접혔는가. 폴백은 값이 같아서 **아무 증상이 없다** — 그래서
        #   설치본에 팩 파일이 빠졌는지를 사람이 알 방법이 이 한 줄뿐이다(경로는 불변).
        out["lane_source"] = _LANE_SOURCE
        print(json.dumps(out, ensure_ascii=False))
        return 0
    try:
        print(lane_state_path(kind))
    except ValueError:
        sys.stderr.write("[bootstrap] 미지 레인 상태 종류: %r "
                         "(marker|boot_last|skip|lock|mission|all)\n" % kind)
        return EXIT_USAGE
    return 0


_USAGE = """usage: javis_bootstrap.py [run|status|assert-ready|lane-path <kind>|issue-ticket --dept <name>] [--self-test]
  (인자 없음 = run)
exit: 0=완료/단독각성 · 3=ping · 4=boot · 5=assert-ready 게이트 · 6=check ·
      7=claim 정당거부 · 8=레인↔팩 · 9=자원 hard_block · 10=세션 컨텍스트 오류 ·
      11=skipped_inflight(정상 skip) · 64=EX_USAGE(사용오류)
"""


def detach_session(argv=None, emit=None):
    """★A18 조건부 내재화(T-0147-7 W2 · W1b 실측 확정) — 세션 분리를 **python 안에서** 수행한다.

    ## 왜(실측 근거)
    W1b 의 `probe_pgid.py` 실측이 확정한 사실: 훅 발화의 **nohup 분기는 pgid 를 분리하지 않아**
    하네스의 group-kill(음수 pid kill)에 부트가 **함께 죽는다**(대조군은 생존 = 계측 타당성 확인).
    원 감사의 'cysd 그룹정리 동사' 메커니즘은 반박됐지만, 이 잔존 위협은 실측으로 성립했다 →
    A18 은 P3→P2 로 복귀했고 처방이 '조건부 내재화'로 확정됐다.

    ## 3중 가드(이 함수의 존재 이유 전부)
    ① **명시 opt-in**: `--detach-session` 인자가 있을 때만 동작한다. 이 인자는 **훅 발화부만**
       넘긴다 — MASTER_DIRECTIVE §0 폴백의 **포그라운드 직접 실행 경로는 미적용**이다.
       (그 경로에 setsid 를 걸면 호출자가 Ctrl-C 로 포기한 뒤에도 스폰이 계속되는 **고아 신설**이
        된다 = MEMORY 'nohup 고아화' 와 같은 부류의 반대 방향 결함. job control 을 보존한다.)
    ② **세션 리더 검사 후 no-op**: 훅의 1순위 분기는 이미 `setsid "$CYS_PY" "$BOOT"` 로 감싼다 —
       그 경우 이 프로세스가 이미 세션 리더라 `os.setsid()` 는 **EPERM(PermissionError)** 을 던지고,
       가드가 없으면 훅 경로 전체가 크래시한다(비평2 B-6 이 선제 내재화 처방을 철회시킨 이유).
       `os.getsid(0) == os.getpid()` 면 조용히 no-op 한다(이미 목적 달성 상태).
    ③ **플랫폼·실패 내성**: `os.setsid` 부재(Windows)·기타 OSError 는 no-op 로 강등한다.
       세션 분리는 **강건성 보강**이지 부트의 전제조건이 아니다 — 실패가 부트를 죽이면 안 된다.

    ## ★U-24 이관 경계 — 이 플래그를 지우려면 **같은 커밋에서** 아래 두 주석을 함께 고쳐라
    `--detach-session` 은 이 파일 안에서만 사는 인자가 아니다. 데몬의 **창작자 ACL 원장**
    (`src/bin/cysd/state.rs` `Daemon::create_caller` · 판정은 `src/bin/cysd/handlers.rs`
    `creator_matches`/`ACL_ROLE_CREATOR`)이 존재 근거로 **이 문자열을 인용**한다: "훅이
    `setsid python3 javis_bootstrap.py --detach-session` 으로 부트를 백그라운드 발화하면 그
    프로세스가 launchd 로 재부모화돼 `external` 등급이 되고, 부트가 **자기가 방금 만든 워커
    좌석**에 지침을 넣는 것까지 ACL 에 걸린다"(2026-08-22 부트 실사고 결함8).
    ∴ 여기서 인자를 지우면 저쪽 주석은 **유령 인용**이 되어, 다음 사람이 "근거가 사라졌으니
    원장도 지워도 되겠다"고 읽는다 — 그 순간 워커 좌석 주입이 다시 `acl denied: external →
    worker` 로 막히고 **전 pane 이 글자 0 으로 죽는다**(치명 앵커 ④).
    ★이 결박은 검체 `H-DOC-10` 이 기계 대조한다(플래그와 인용의 동시 존재 ∨ 동시 부재).
    ★U-24 에서 **제거하지 않은 이유**: 제거는 이 파일 밖 3곳(훅 발화부·state.rs·handlers.rs)의
      동시 개정을 요구하는데 그 파일들은 이 작업 단위의 반경 밖이다. 반쪽 제거가 곧 위 사고다.

    반환: (분리됨 bool, 사유) — 사유는 stderr 진단에만 쓴다(stdout 계약 무오염).
    """
    argv = sys.argv if argv is None else argv
    log = emit if emit is not None else (lambda m: sys.stderr.write("[bootstrap] %s\n" % m))
    if "--detach-session" not in argv:
        return False, "미요청(포그라운드·직접 실행 경로는 미적용 — job control 보존)"
    if not hasattr(os, "setsid") or not hasattr(os, "getsid"):
        log("세션 분리 생략: os.setsid 미지원 플랫폼")
        return False, "플랫폼 미지원"
    try:
        if os.getsid(0) == os.getpid():
            # 이미 세션 리더 = 훅의 setsid(1) 분기가 이미 분리했다. 여기서 os.setsid()를 부르면
            # EPERM 으로 훅 경로가 크래시한다 — no-op 이 정답이다.
            return False, "이미 세션 리더(setsid 래핑 분기) — no-op"
        os.setsid()
        return True, "세션 분리 완료(group-kill 내성 확보 — nohup·bare & 분기)"
    except (OSError, AttributeError) as e:
        log("세션 분리 실패(무시·부트 계속): %s: %s" % (type(e).__name__, e))
        return False, "실패(%s)" % type(e).__name__


def main(argv):
    # preflight/CI 호환: `--self-test`는 subcommand 없이도 동작(가로채기).
    if "--self-test" in argv:
        return cmd_self_test()
    # ★A18: 발화부에서 넘긴 명시 요청이 있을 때만 세션을 분리한다(위 3중 가드 참조).
    #   subcommand 판정 **전에** 수행한다 — 분리는 프로세스 정체성의 문제이고, 어떤 서브커맨드든
    #   하네스 group-kill 에 함께 죽는 것은 같은 결함이다.
    if "--detach-session" in argv:
        detached, why = detach_session(argv)
        sys.stderr.write("[bootstrap] --detach-session: %s (%s)\n"
                         % ("분리" if detached else "무동작", why))
        argv = [a for a in argv if a != "--detach-session"]
    cmd = argv[1] if len(argv) > 1 else "run"
    if cmd == "issue-ticket":
        return cmd_issue_ticket(argv[2:])
    if cmd == "lane-path":
        return cmd_lane_path(argv[2:])
    table = {"run": cmd_run, "status": cmd_status, "assert-ready": cmd_assert_ready}
    fn = table.get(cmd)
    if fn is None:
        # ★A14: 구 `.get(cmd, cmd_run)` 는 **미지 입력의 기본값을 최대 부작용**(전면 부트)으로 뒀다 —
        #   오타 한 글자('runn'·'--status')가 좌석 탈취·CEO 티켓 소비 같은 비가역 부작용을 일으켰다.
        #   미지 서브커맨드는 거부한다(fail-closed) — EX_USAGE(64)는 게이트 exit 공간(2·3·…·11)과
        #   겹치지 않아 소비처가 '사용오류'를 판정 결과로 오독하지 않는다(RC2 타입드 종료).
        sys.stderr.write("[bootstrap] 미지 서브커맨드: %r — 실행하지 않았다(부작용 0).\n%s"
                         % (cmd, _USAGE))
        return EXIT_USAGE
    return fn()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
