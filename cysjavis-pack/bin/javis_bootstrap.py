#!/usr/bin/env python3
"""javis_bootstrap.py — master 부트 시퀀스의 결정론 격상 (BOOTSTRAP_HARDENING WP-1).

"너는 마스터다" 이후의 기계적 절차 전부를 단일 exit-code 체인으로 수행한다.
LLM(master)의 역할은 이 스크립트 실행·출력 인용·이후 지휘뿐이다 — 산문 단계 수행 금지.

단계 체인 (실패 시 즉시 중단·단계명+원인을 stderr와 boot-last.json에 기록):
  ① preflight --fix (**비치명** — FAIL은 경고로 강등하고 계속. 팀 부팅의 진짜 게이트는 ⑤)
  ② cys ping                     ③ cys claim-role master
  ④ cys boot (결손>0에서만 — 결손 0=구성 충족이면 호출 생략·스폰 없음)
  ⑤ orchestra check (bounded retry **24회×5s ≈ 120s 상한** — 노드 스폰은 비동기·check는 무대기
  스냅샷이므로 레이스 봉쇄. 값의 진실원천은 CHECK_RETRIES·CHECK_INTERVAL_S 상수이고 env
  CYS_BOOT_CHECK_RETRIES·CYS_BOOT_CHECK_INTERVAL_S로만 덮인다 — 테스트 하네스 전용)
                                                          ⑥ 완료 마커 write
  ⑦ cys-dept promote-if-pending --request-only (비대기 — 부트와 승격 동의의 분리)
  ⑧ 기계 요약 JSON 출력 (master는 이것을 인용해 보고한다)

완료 마커 ~/.cys/.master-bootstrapped 는 base 데몬 전용 단일-writer 마커다:
  - writer = 이 스크립트의 성공 경로(⑤ exit 0 후 ⑥) 유일. 삭제 주체 없음(버전 필드로 stale 판정).
  - ★소켓 격리: CYS_SOCKET이 base가 아니면(부서 pane 부트) write하지 않는다 — 부서장 부트가
    base 마커를 오염시키면 CEO 승격 게이트(cys-dept)가 오개방된다.

exit(`run` 체인 — 코드 상수 EXIT_* 와 대조 유지 · 진실원천은 상수):
      0=부트 완료(또는 부서장 단독 각성=CEO 티켓 부재) / 3=ping / 4=boot
      6=check 최종 실패(CHECK_RETRIES 소진) / 7=claim 정당거부(이 surface는 master 아님 — 지휘 중단·인계)
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

부서 교리 게이트 (증분2 — D1 옵션 1'):
  ⓐ CEO 티켓 권한 게이트(P7): 부서 레인(CYS_SOCKET=부서 소켓)의 팀 기동은 CEO 발급 티켓 필수.
     티켓 부재/만료 → 실패가 아니라 '부서장 단독 각성'으로 강등(팀 기동만 생략·역할 등록/프리플라이트는
     정상·exit 0). 발급은 base 레인에서 `issue-ticket --dept <name>` 로만.
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


def _run(cmd, timeout=120):
    """서브프로세스 실행 — (exit, stdout+stderr 병합 텍스트). shell 미사용(경로 quoting 안전)."""
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           encoding="utf-8", errors="replace")
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


def _socket_is_base(sock):
    """순수 판정: 소켓 경로 문자열 → base 여부(§4.1 소켓 격리). CYS_SOCKET 미설정('')=base.
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


def _is_base_socket():
    """CYS_SOCKET env 래퍼(호출부 하위호환)."""
    return _socket_is_base(os.environ.get("CYS_SOCKET", ""))


def _sanitize_sock_key(sock):
    """소켓 전체 경로 → 파일명 안전 락 키(레인마다 유일). 부서 소켓은 basename(cys.sock)이 동일해
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
    ("RESOURCE_GATE_ABSENT", "④′resource-gate-absent"),
    ("RESOURCE_GATE", "④′resource-gate"),
    ("RESOURCE_GATE_NOTIFY", "④′resource-gate-notify"),
    ("RESOURCE_GATE_SKIP", "④′resource-gate-skip"),
    # ★W2: 팩↔바이너리 스큐 폴백은 ④boot **판정 기록 이전**에 남는다(선언 순서=실행 순서 계약).
    ("BOOT_SKEW", "④boot-skew"),
    ("BOOT", "④boot"),
    ("BOOT_BUSY", "④boot-busy"),
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


class _Log:
    """단계 결과를 (레인) boot-last 에 누적(진단 가시성 — 각 retry 시도 포함).

    ★A19 런 정체성: 레코드는 이제 **누가/언제/어디서** 돈 런인지 스스로 말한다.
      run_id(started+pid)·pid·surface·role 이 없던 종전에는 (ⓐ)한 레인의 두 pane 중 누구의 런인지,
      (ⓑ)진행 중인지 크래시했는지 구분이 불가능했다(둘 다 'result 키 없음'으로 보였다).
      `result:{"ok":null,"state":"running"}` 선기록 + finish() 의 try/finally 종결 기록이 그 둘을
      기계로 분리한다.
    ★role 은 **env 파생**(CYS_ROLE — 데몬이 pane에 주입)이다: 여기서 `cys surface-role` 왕복을
      추가하면 부트 시작 전에 데몬 왕복이 하나 더 늘고, ③ 이전에는 아직 claim 도 안 된 상태라
      권위값도 아니다. ③ 성공 후 `role_claimed` 를 별도로 남긴다(관측 파생 — 보고=실측).
    """

    def __init__(self):
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
                     "socket": os.environ.get("CYS_SOCKET", ""), "base_socket": _is_base_socket(),
                     # ★선기록 — 이 시점 이후 어떤 경로로 죽어도 '진행 중'이 남는다.
                     "result": {"ok": None, "state": "running", "run_id": self.run_id,
                                "surface": self.surface}}
        self._last_step_order = -1
        _atomic_write_json(self.path, self.data)

    def _attributed(self, res):
        """result 딕트에 런 귀속을 못박는다 — §0 소비 술어('자기 surface의 최신 완주 런')의 전제."""
        res.setdefault("run_id", self.run_id)
        res.setdefault("surface", self.surface)
        res.setdefault("pid", self.pid)
        return res

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
        _atomic_write_json(self.path, self.data)

    def result(self, **kw):
        """단계 성공/강등 경로의 result 기록(귀속 자동 첨부)."""
        self.data["result"] = self._attributed(dict(kw))
        _atomic_write_json(self.path, self.data)

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
        _atomic_write_json(self.path, self.data)

    def fail(self, name, code, detail, exit_code, ok=False, state="failed"):
        """실패·거부 경로의 공통 종결(단계 기록 → result → stderr → loud 알림).

        ★ok/state 파라미터(CS-2⑩ · 비평2 C-3): 정당거부(exit 7)·세션 컨텍스트 오류(exit 10)는
          `ok=None, state='declined'|'session_error'` 로 남긴다 — 공유 boot-last 에 `ok:false` 를
          덮으면 같은 레인의 건강한 master 가 남긴 ok:true 를 남의 pane 이 지워, §0 의 '직접 실행'
          분기를 무한 churn 시킨다(부트 폭풍). 인프라 실패(ping·boot·check·lane-pack)는 그대로
          ok=False 다 — 그건 실제로 이 레인의 부트가 깨진 사실이다.
        """
        self.step(name, code, detail)
        self.result(ok=ok, state=state, failed_step=name, exit=exit_code)
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
        _atomic_write_json(self.path, self.data)
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


def _singleflight_key(sock):
    """순수 판정: 소켓 → 싱글플라이트 락 키(R1-LOW-4). base 레인은 env 미설정·base 경로 명시
    어느 쪽이든 단일 'base' 키로 정규화한다 — 같은 base 데몬에 서로 다른 락을 주던 선재결함 교정.
    비-base(부서·커스텀)는 전체 경로 새니타이즈로 레인마다 유일."""
    return "base" if _socket_is_base(sock) else _sanitize_sock_key(sock)


def lane_key(sock=None):
    """이 부트가 속한 **레인 키** — 'base' 또는 소켓 경로 새니타이즈 값(레인마다 유일).
    락 키와 동일 규약을 쓴다(`_singleflight_key`) — 락은 레인별인데 상태 파일은 공유였던
    비대칭(G15·R3)을 없애려면 두 네임스페이스가 **같은 키 함수**를 써야 한다."""
    return _singleflight_key(os.environ.get("CYS_SOCKET", "") if sock is None else sock)


# 레인 스코프 상태의 **경로 규약 단일 소유자**(G15 · P3-A-DEPT-LANE · CS-7②).
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
_LANE_STATE_KINDS = {
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
_ALWAYS_LANE_SUFFIXED = ("skip", "lock", "delivery", "delivery_epoch")


def lane_state_path(kind, sock=None):
    """레인 스코프 상태 파일 경로.
    kind ∈ marker|boot_last|skip|lock|mission|delivery|delivery_epoch.
    base 레인: 역사적 경로(마커=`~/.cys/.master-bootstrapped` · `boot-last.json`).
    비-base 레인: `-<lane>` 접미(`.master-bootstrapped-<lane>` · `boot-last-<lane>.json`).
    ※ skip·lock·delivery* 는 **항상** 레인별이다 — 규약을 이 함수 하나로 모은다(사본 금지)."""
    try:
        base_dir, stem, ext = _LANE_STATE_KINDS[kind]
    except KeyError:
        raise ValueError("미지 레인 상태 종류: %r" % kind)
    if base_dir == _STATE:                       # 지연 해소(위 주석) — 호출 시점의 env 를 본다
        base_dir = state_dir()
    key = lane_key(sock)
    if kind in _ALWAYS_LANE_SUFFIXED:
        return os.path.join(base_dir, "%s-%s%s" % (stem, key, ext))   # 항상 레인별(구 동작 보존)
    if key == "base":
        return os.path.join(base_dir, stem + ext)
    return os.path.join(base_dir, "%s-%s%s" % (stem, key, ext))


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
#   launch-agent(role 은 surface.create 가 등록 — GUI ▶부서장 start_dept_master 와 동일 명령),
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
    """부서 대상 명령의 env — ★G34: 소켓과 팩은 항상 쌍으로 간다(start_dept_master 와 동일 계약).
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
        if code != 0 or not re.fullmatch(r"[a-z0-9][a-z0-9-]*", name or ""):
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

    # D2: 부서장 기동 — GUI ▶부서장(start_dept_master)과 동일 명령. 이미 살아 있으면 생략(멱등).
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


def _shared_verdict_deficit(status):
    """★A1 클래스 소멸: 결손 판정이 **⑤check 와 문자 그대로 같은 함수**를 소비한다.

    `javis_orchestra.check_verdicts(status)` 는 cmd_check 가 화면 출력에 쓰는 그 판정 코어다
    (공유 술어 `javis_boot_node.node_liveness`·`slot_satisfied` 소비). 결손 판정이 이 함수를
    소비하면 '결손 0인데 check 실패'(=④ 생략 → ⑤ 실패 → exit 6 → 재선언 동일의 라이브락)가
    **구조적으로 불가능**해진다 — 종전에는 이름공간(W0에서 정합)과 생존 신호(여기)가 갈려 있었다.

    ★역방향 회귀 차단: node_liveness 는 `absent` 만 미충족으로 본다. agent_alive 단독·좌석 점유·
      quiet_but_alive·**좌석 판정불가(unknown)** 는 전부 충족측이라, 건강한 quiet 노드를 결손>0
      으로 오판해 자원 hard-block 을 되살리는 경로가 없다(W0 handoff 가 명시한 그 경계 유지).
    반환 (결손 bool, 사유) | (None, 실패사유)."""
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
        return True, ("공유 판정 결손(의무 %s / 부재 %s) — 결손 존재 [신호=check_verdicts 동일]"
                      % (", ".join(verdicts), ", ".join(missing)))
    note = ("" if not presumed
            else " · 생존추정(각성 미확인) %s — 재각성 권장이나 결손 아님" % ", ".join(presumed))
    return False, ("공유 판정 충족(의무 %s 전원) — 결손 0(재선언)%s [신호=check_verdicts 동일]"
                   % (", ".join(verdicts), note))


def _team_has_deficit():
    """팀 결손 여부 산출 → (결손 bool, 사유). 신호 원천 실패 → 보수적으로 결손 가정(게이트 진행).

    ★W2 술어 단일화(A1 클래스·CS-1②): 1차 경로는 `javis_orchestra.check_verdicts` — ⑤check 가
      쓰는 **같은 함수**다. 2차는 W0 의 로스터 판정(cys list `role=`+`!exited` — 이름공간만 정합),
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
      Fatal 실패 = mandatory:true 인 role 의 outcome ∈ {failed, missing}.
      busy(다른 boot 진행 중)·already_alive·launched 는 실패가 아니다(G11 — busy 를 성공으로
      오인하지도, 실패로 오인하지도 않는다).
    ★--json 소비 불가(구 바이너리·파싱 실패)면 **종전 계약으로 보수 폴백**: 비0 = Fatal.
      새 계약을 못 읽는 상태에서 Degrade 로 접으면 진짜 실패를 은닉한다(fail-open 금지).
    ★(W4) 단 **exit 75(busy)** 는 그 보수 폴백에서 제외한다 — busy 는 실패가 아니라 '무스폰'이고,
      75 는 신 계약 전용 값이라(구 바이너리는 0/1/2만) 오해석 위험이 없다. 이걸 Fatal 로 접으면
      훅↔GUI 중첩 부트마다 exit 4 위경보가 난다(P3-B16 부류의 반복성 오경보)."""
    v = _parse_boot_json(out)
    if v is None or not isinstance(v.get("roles"), list):
        if code == CYS_BOOT_EXIT_BUSY:
            return None
        return (None if code == 0
                else "cys boot 실패(exit %s) — --json 계약 소비 불가로 종전 계약(비0=Fatal) 적용:\n%s"
                     % (code, out))
    bad = [r for r in v["roles"]
           if r.get("mandatory") and r.get("outcome") in ("failed", "missing")]
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


def _fatal_detail(bad, out):
    """Fatal 사유 1줄 조립 — install_hint 는 **그대로** 인용한다(플랫폼 분기는 생산자 몫·B15)."""
    detail = ", ".join("%s=%s%s" % (r.get("role"), r.get("outcome"),
                                    (" [" + r["install_hint"] + "]") if r.get("install_hint") else "")
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
    if not dept or not re.fullmatch(r"[a-z0-9][a-z0-9-]*", dept):
        sys.stderr.write("[issue-ticket] --dept <name>(kebab-case a-z0-9-) 필수: %r\n" % dept)
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
    log = _Log()
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
    _progress("② 데몬 생존 확인…")
    code, out = _run(["cys", "ping"], timeout=15)
    log.step(STEP.PING, code, out)
    if code != 0:
        return log.fail(STEP.PING, code, out, EXIT_PING)

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
    # env 가 없으면(§0 폴백의 포그라운드 직접 실행·구 훅) 종전대로 여기서 직접 claim 한다 —
    # 그 경로는 조상 체인이 온전하므로 정상 동작한다(하위호환·스큐 안전).
    # ★판정 결박(무바인딩 env 금지): 정수처럼 보이는 env 하나로 claim 을 건너뛰면, 사용자 셸·
    #   래퍼에 남은 값이 **치지도 않은 claim 을 '실측'으로** boot-last 에 적게 된다(CS-3 보고=실측
    #   위반). 그래서 판정은 ⓐ같은 surface 귀속(CYS_CLAIM_SID) ⓑ신선도(CYS_CLAIM_AT, 300s)까지
    #   갖췄을 때만 소비한다. 하나라도 어긋나면 **무시하고 직접 claim** 한다(구 훅·직접 실행과
    #   동일 경로 — 하위호환이 곧 안전한 기본값이다).
    _progress("③ master 역할 등록…")
    _pre_rc = os.environ.get("CYS_CLAIM_RC", "").strip()
    _pre_sid = re.sub(r"[^0-9]", "", os.environ.get("CYS_CLAIM_SID", ""))
    _my_sid = re.sub(r"[^0-9]", "", my_surface_id())
    try:
        _pre_age = time.time() - float(os.environ.get("CYS_CLAIM_AT") or 0)
    except (TypeError, ValueError):
        _pre_age = float("inf")
    _pre_bound = (_pre_rc.lstrip("-").isdigit() and _pre_sid and _pre_sid == _my_sid
                  and 0 <= _pre_age < float(os.environ.get("CYS_CLAIM_MAX_AGE_S") or 300))
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
                    "처방: pane 안에서(훅 발화 경로면 훅이 선행 claim 한다) 재선언하라."),
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
        if not ok:
            note = ("CEO 티켓 부재 — 부서장 단독 각성(팀 기동은 CEO 티켓 발급 후). "
                    "발급: base master에서 `javis_bootstrap.py issue-ticket --dept %s`. 사유: %s"
                    % (dept, why))
            # R1-LOW-3 검증 비대칭 경고: 발급(issue-ticket)은 정규식을 강제하나 소켓 쪽 부서명은
            # 자유 형식이라, 불일치 부서는 티켓을 영영 못 받는 비대칭이 침묵으로 남는다 — 명시.
            if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", dept):
                note += (" ★주의: 부서명 %r 은 발급 정규식([a-z0-9][a-z0-9-]*) 불일치 — "
                         "이 부서명은 티켓 발급 불가 형식이다(부서 재생성 필요)." % dept)
            _progress(note)
            log.step(STEP.CEO_TICKET, 0, note)
            summary = {"ok": True, "marker": "부서장 단독 각성(CEO 티켓 부재)",
                       "solo_awakening": True, "dept": dept,
                       "steps": [(s["step"], s["exit"]) for s in log.data["steps"]],
                       "lane": log.lane, "boot_last": log.path}
            # ★A7 채널 보존: solo_awakening 은 **성공** 경로이므로 stdout 최종 JSON 을 유지한다
            #   (session-start 산문 계약 "완료 선언은 최종 JSON 인용 시에만"의 소비 대상).
            log.result(ok=True, state="solo_awakening", solo_awakening=True, reason=why,
                       exit=EXIT_OK)
            print(json.dumps(summary, ensure_ascii=False))
            return EXIT_OK
        log.step(STEP.CEO_TICKET, 0, "CEO 티켓 유효 — 부서 팀 기동 진행. %s" % why)

    # ── 증분2 ⓑ 결손 기준 자원 사전 게이트 — 팀 기동(④) 직전 ──
    # 결손을 cys list 라이브 노드의 **로스터 판정**으로 산출 — 의무 역할 목록은 ⑤check와 동일 소스
    # (javis_orchestra.effective_required_roles)를 소비한다(W0 P0 지혈 · G26/A1). R1-MED-1의 '총수
    # 비교 폐기'는 그대로 유지되고, 그 위에서 역할 이름공간까지 check와 일치시킨 것이다.
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
        if boot_busy:
            log.step(STEP.BOOT_BUSY, code,
                     "다른 boot 가 락 보유 — 이 런은 무스폰 skip(exit %s). 티켓 미소비·Degrade 아님. "
                     "팀 기동 확인은 ⑤check 가 담당한다." % code)
            _progress("④ 다른 boot 진행 중(무스폰 skip) — 티켓 보존·⑤ 생존 확인으로 진행")
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
    code, out = 1, "orchestra 부재"
    daemon_gone = False
    for attempt in range(1, CHECK_RETRIES + 1):
        code, out = _run([py, orchestra, "check"], timeout=check_timeout)
        log.step(STEP.CHECK, code, out, suffix="#%d" % attempt)
        if code == 0:
            break
        # ★G32/H-EXIT-7 — check exit 2 는 '노드 미기동'이 **아니다**: 데몬 소실·cys 부재·status 파손
        #   (판정 불가)이다. 두 갈래를 뭉개면 처방이 뒤집힌다(2는 `cys ping`·데몬 기동, 1은 `cys boot`).
        #   판정 불가에서는 재시도가 의미 없으므로 즉시 이탈해 정확한 처방으로 실패한다(A12 영구 분류).
        if code == 2:
            daemon_gone = True
            break
        if code == 127:
            daemon_gone = True    # orchestra 스크립트/인터프리터 부재 — 영구, 처방이 다르다
            break
        if attempt < CHECK_RETRIES:
            if attempt % hb_every == 0:
                # 침묵 창 상쇄(B9 방향 ③) — 진행 하트비트는 stderr(verdict 채널 무오염)
                sys.stderr.write("[bootstrap] ⑤check 재시도 %d/%d 진행 중(노드 기동은 비동기)\n"
                                 % (attempt, CHECK_RETRIES))
                sys.stderr.flush()
            time.sleep(CHECK_INTERVAL_S)
    if code != 0:
        if daemon_gone:
            return log.fail(STEP.CHECK_UNJUDGEABLE, code,
                            "check 가 **판정 불가**를 반환했다(exit %s) — '노드 미기동'이 아니다. "
                            "exit 2=cysd 데몬 소실·cys 미설치·status 파손 / exit 127=orchestra "
                            "스크립트·인터프리터 부재. 처방: `cys ping` 으로 데몬을, CYS_PACK_DIR·"
                            "python 해소로 팩 배선을 확인하라(`cys boot` 는 이 상황의 처방이 아니다).\n%s"
                            % (code, out), EXIT_CHECK)
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
        log.step(STEP.PROMOTE_REQUEST, 0, "부서 컨텍스트 — 생략")

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
        # ★결손 판정 ↔ check verdict 공유(H-PRED-1): 같은 status fixture 에서 판정이 갈리지 않는다.
        _healthy = {"surfaces": [
            {"role": "cso", "exited": False, "awakened_at": 1.0},
            {"role": "worker-2", "exited": False, "status": {"age_secs": 3, "state": "working"}},
            {"role": "reviewer-gemini", "exited": False, "agent_alive": True},
            {"role": "reviewer-codex", "exited": False, "agent_alive": True}]}
        _has, _why = _shared_verdict_deficit(_healthy)
        assert _has is False, "건강한 팀(생존추정 포함)을 결손>0 으로 오판: %s" % _why
        _grok_only = {"surfaces": [
            {"role": "reviewer-grok", "exited": False, "agent_alive": True},
            {"role": "cso-1", "exited": False, "agent_alive": True}]}
        _has2, _why2 = _shared_verdict_deficit(_grok_only)
        assert _has2 is True, "grok·cso-1 좌석이 의무 슬롯을 채운 것으로 계상(G26 재발): %s" % _why2

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

        # ── t10: TCC 탐침 대상이 실자원 파생인가(P3-A-TCC · H-PRED-10) ──
        _t = _tcc_probe_targets()
        if sys.platform == "darwin":
            _paths = [p for p, _ in _t]
            assert _paths, "darwin 에서 TCC 탐침 대상이 비었다(탐침 소멸)"
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
    print("javis_bootstrap self-test OK (W6: 폴백 전제 판정 8종(부서 자동 생성 게이트) · "
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
