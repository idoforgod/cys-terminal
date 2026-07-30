#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
javis_boot_node.py — 결정론 단일 노드 부트 헬퍼 (부트스트랩 시행착오 재발방지)

배경(2026-06-13 실측 부트스트랩 시행착오 · MASTER_DIRECTIVE §0 등재 · codex R1·R2 적대검증 반영):
  F1 typing_guard 콜드스타트 레이스 — `cys launch-agent`는 pane 생성 직후 *즉시* 디렉티브를
     주입한다. 그러나 갓 뜬 CLI 의 시작 애니메이션이 idle_secs=0 을 유지해 데몬 typing_guard 가
     "사람 입력 중"으로 오탐 → 주입 send 차단(CYS_TYPING_GUARD_SECS=0 무효 — 활동기반).
  F2 launch-agent 가 "failed … closed(role 점유 해제)"로 *허위* 실패보고하나 실제 surface 는
     생존·role 점유 → 재기동 시 claim_denied + litter surface.
  F3 orchestra check 의 agent_alive 가 주입실패(메타=None)·노드래퍼면 구조적 false-negative.
  F4 헌법 가드가 cys send 본문의 "*_DIRECTIVE.md"·"soul.md" 패턴을 헌법쓰기로 오탐·차단.
  F5 막힌 privileged-role surface 회수에 kill -9 필요(surface.close self-only).

★상태 계약 3분리(codex R1 핵심 권고) — 절대 섞지 마라:
  surface_occupied : role 을 가진 비종료 surface 존재(cys list).
  process_present  : 그 surface 자손에 *해당 CLI 고유 바이너리* 프로세스 생존(basename 동등 매칭).
  awake_ready      : 노드가 디렉티브를 읽고 각성 = agent_alive OR 'fresh set-status' 만(프로세스 X).
부트 성공의 유일한 계약 = '이번 주입 이후의 fresh set-status ack(age<주입후 경과)'.

★생존 술어 단일화(codex R2 결함2 — cmd_check 와 reclaim 이 같은 상태를 반대로 해석하던 버그):
  node_alive(status, role) = awake_ready OR quiet_but_alive. orchestra 의 READY 보강과 reclaim 의
  '죽음' 판정이 *같은 함수*를 공유한다 → 건강한 quiet 노드를 reclaim 이 죽이는 모순 차단.
  quiet_but_alive = '각성 이력(set-status state 존재) + 현재 surface_ref 에 결박된 pid 의 기대 agent
  프로세스 생존'. status 를 surface_ref 로 결박해 litter/exited row·과거이력 오인(codex R2 결함1·5) 차단.

★★공유 술어 3종 단일 export (T-0147-7 W2 · CS-1② · A1 클래스 소멸):
  이 모듈이 `node_liveness()`·`role_family()` 의 **유일 정의**이고, `javis_orchestra` 가
  `slot_satisfied()` 의 유일 정의다. 전 소비처(orchestra check · bootstrap 결손 · wakeup zombie
  가드 · reclaim · PRE-CHECK)가 **import 소비만** 한다 — 각자 판정을 재구현하면 판정 이원화로
  재발한다(cysd governance.rs:1343-1360 · handlers.rs:1266-1268 가 두 번 문서화한 그 클래스).

  node_liveness(status, role) → (grade, reason)
    "awake_confirmed"  각성 **확정**. 근거는 ①`awakened_at` 래치(데몬 SOT·영속·단방향) 또는
                       ②신선 set-status ack. 재주입·재스폰 금지.
    "alive_presumed"   생존 **추정**. agent_alive 단독 / 좌석 점유(자손 프로세스) / quiet_but_alive.
                       ★agent_alive 단독은 '각성'이 아니다(B6): 빈 CLI 도 프로세스는 산다.
                       그래도 재스폰·재주입은 금지다(있는 노드를 두 번 띄우면 A1 역방향 결함).
    "absent"           좌석 없음/exited/좌석 비었음(자손 0). 스폰 대상.
    "unknown"          **판정 불가**(좌석 프로브 실패). 3등급 중 하나가 아니라 '타입으로 구분되는
                       판정불가'다(CS-2 원칙: 판정과 판정불가는 절대 융합하지 않는다).
                       소비 규약 = Unknown 이원 규칙(아래 resolve_unknown_for_spawn 참조).

  ★래치 단방향 불변식(금지 방향 ⑦ · 비평2 B-1): 래치 **존재**=awake 확정. 래치 **부재**는
    NOT-awake 가 아니다 — 배포 이전에 각성한 노드·데몬 재시작 직후는 영원히 래치가 없다
    (legacy-presumed). 그러므로 부재는 기존 균형 술어(agent_alive OR fresh set-status —
    codex R1/R2 적대검증 산물)로 **강등**만 하고, 어떤 경로도 '래치 없음 ⟹ 재주입/재스폰'을
    유도하지 못한다. 이 불변식이 깨지면 A1 라이브락의 역방향(건강한 전 팀 재스폰)이 신설된다.

프로토콜(idle-then-inject):
  1) PRE-CHECK  awake_ready 면 already_up. 점유만 됐고 미각성이면 입양→주입.
  2) LAUNCH     없을 때만 launch-agent 1회. 실패 텍스트 무시·cys list 재조회(F2).
  3) POLL-IDLE  idle_secs>=IDLE 안착까지 폴링(F1). 폴링 중 awake_ready 잡히면 즉시 종료.
  4) INJECT     주입 직전 t_inject 기록. 자연어(확장자 없음·F4) 각성 지침을 `cys send --queued`
                단일경로로 주입(메시지+자동 Return 원자적·typing_guard 우회·중복 위험 제거 — codex R2 결함4).
  5) VERIFY     t_inject *이후*의 fresh set-status ack(age<경과·+마진 없음 — codex R2 결함3)만 성공.

사용:
  python3 javis_boot_node.py --role cso --agent claude [--cwd D] [--idle 4] [--timeout 90] [--json]
  python3 javis_boot_node.py --reclaim --role cso          # 막힌(죽은) 미각성 surface 결정론 회수
  python3 javis_boot_node.py --self-test                   # 순수함수 회귀 배터리
  종료코드 0=각성확정/회수성공/self-test통과 · 1=미확정(타임아웃) · 2=치명(데몬다운·인자오류).

이 헬퍼는 결정론 환원 원칙(MASTER_DIRECTIVE §12)의 산물 — 노드 기동을 LLM 시행착오가 아니라
스크립트가 처리한다.
"""
import argparse
import json
import os
import subprocess
import sys
import time

PACK_DIR = os.environ.get("CYS_PACK_DIR") or os.path.expanduser("~/.cys/pack")
STATUS_FRESH_SECS = 600   # set-status 신선도 임계('살아 일하는 중' 인정 폭)

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
#   ._pth 는 표준 경로 계산을 우회해 **스크립트 폴더를 sys.path 에 넣지 않는다**
#   (2026-07-29 Windows 0.14.4 실측: `ModuleNotFoundError`). unix/mac 은 스크립트 폴더가 이미
#   sys.path[0] 이라 이 블록은 무동작(멱등). append 인 이유는 형제 발견이 목적이고 stdlib
#   precedence 를 강등하지 않기 위함이다(선례: javis_orchestra.py·javis_report.py).
#   ★W2 에서 이 가드가 **필수**가 됐다 — javis_budget 형제 import 가 아래에 생겼고,
#   test_import_guard 가 '형제 import 는 유효 가드 뒤에만' 을 기계 강제한다.
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

# ★시간 예산 단일 소스(B9·B17·P3-A-120S) — 상수 곱 산술 금지. import 실패 시에만 leaf 하한 폴백.
try:
    import javis_budget as _budget
except Exception:                                     # 부서 팩 결손·팩 스큐 — 새 크래시 지점 금지
    _budget = None


def budget(name, fallback):
    """예산 leaf 해소 — javis_budget 가 유일 SOT. 소비 불가 시 명시 폴백(조용한 접힘 금지)."""
    if _budget is None:
        return fallback
    try:
        return _budget.leaf(name)
    except Exception:
        return fallback


# ── 공유 술어: liveness 등급 상수(전 소비처가 문자열 리터럴 대신 이 상수를 쓴다) ──
LIVENESS_AWAKE = "awake_confirmed"
LIVENESS_PRESUMED = "alive_presumed"
LIVENESS_ABSENT = "absent"
LIVENESS_UNKNOWN = "unknown"
# 좌석 캐시의 유일 writer = cysd watchdog 틱(governance.rs:13,:47 — 5초). Unknown 의 시한부
# 해소는 '1주기 대기'가 상한이다(그 이상 기다리는 것은 가용성 손해이고, 중복 스폰은 boot 락이 막는다).
SEAT_WATCHDOG_TICK_S = 5

# agent → 자식프로세스 comm 고유 바이너리 basename(생존탐침 전용 — 각성/READY 판정엔 안 씀).
# ★범용 "node" 폴백·빈문자 매칭 금지(codex R1 결함2)·substring 금지(codex R2 논쟁점: my-claude-helper
# 오매칭) → basename 동등 매칭. 실측 2026-06-13: claude=claude · codex=codex(node 래퍼 아래 손자)
# · gemini=agy(Antigravity CLI) · grok=grok.
AGENT_COMM = {
    "claude": ("claude",),
    "codex":  ("codex",),
    "gemini": ("agy", "gemini"),
    "grok":   ("grok",),
}
# role → 기대 agent(status 의 agent 메타가 None 일 때 명시 매핑 — wildcard 추정 금지).
ROLE_AGENT = {
    "cso": "claude", "worker": "claude", "master": "claude",
    "reviewer-gemini": "gemini", "reviewer-codex": "codex",
    "reviewer-grok": "grok", "reviewer": "claude",
    # ★무구독 폴백(오너 2026-06-14): agy/codex 미감지 시 Claude 대체 리뷰어 슬롯.
    "reviewer-claude-1": "claude", "reviewer-claude-2": "claude",
}
# role → (각성 지침에서 가리킬 디렉티브 자연어 명칭[확장자 없음 — F4], 기본 set-status state)
ROLE_DIRECTIVE = {
    "master":          ("MASTER(부서장) 절대지침", "working"),
    "cso":             ("CSO(최고 시스템 운영자) 절대지침", "working"),
    "worker":          ("WORKER(워커) 절대지침", "waiting"),
    "reviewer-gemini": ("REVIEWER(리뷰어) 절대지침", "waiting"),
    "reviewer-codex":  ("REVIEWER(리뷰어) 절대지침", "waiting"),
    "reviewer":        ("REVIEWER(리뷰어) 절대지침", "waiting"),
    "reviewer-claude-1": ("REVIEWER(리뷰어) 절대지침", "waiting"),
    "reviewer-claude-2": ("REVIEWER(리뷰어) 절대지침", "waiting"),
}


# ───────────────────────── cys 호출 ─────────────────────────
def run(args, timeout=15):
    try:
        r = subprocess.run(args, capture_output=True, timeout=timeout)
        return r.returncode, r.stdout.decode("utf-8", "replace"), r.stderr.decode("utf-8", "replace")
    except Exception as e:
        return 255, "", str(e)


def _kill(pid, force=False):
    """OS중립 프로세스 종료(RC-6) — unix=`kill [-9] <pid>`, Windows=`taskkill /PID <pid> /T [/F]`.
    Windows엔 kill.exe가 PATH에 없어(구: FileNotFoundError로 회수 경로 붕괴) taskkill로 분기한다."""
    if os.name == "nt":
        args = ["taskkill", "/PID", str(pid), "/T"] + (["/F"] if force else [])
    else:
        args = ["kill"] + (["-9"] if force else []) + [str(pid)]
    return run(args, timeout=5)


# ★G24(H-WIN-10): 1차 '그레이스풀' 단계가 이 플랫폼에서 **실제로 효과가 있나**.
#   Windows 의 `taskkill /PID <pid> /T`(무 `/F`)는 WM_CLOSE 를 보내는 것이고, 콘솔 프로세스
#   (claude·codex·agy 같은 CLI TUI)는 메시지 루프가 없어 **종료되지 않는다** — "This process
#   can only be terminated forcefully" 를 내며 구조적 no-op 이다. 그럼에도 회수 경로는 1차를
#   보내고 1.5s 를 기다리고 나서 강제 단계로 갔다 — 매 회수마다 무의미한 지연 + '그레이스풀을
#   시도했다'는 **거짓 기록**이 남았다.
#   처방(W2 handoff '인터프리터·후보 확대 금지 — loud no-op 이 정답'과 동형): 대체 시그널
#   (CTRL_BREAK 등)을 새로 **발명하지 않는다**. 무효를 **명시 로그**하고 강제 단계로 직행한다.
GRACEFUL_KILL_SUPPORTED = os.name != "nt"
GRACEFUL_KILL_NOOP_REASON = (
    "Windows: 1차 그레이스풀 단계 무효(taskkill /T 무 /F 는 WM_CLOSE — 콘솔 프로세스 미종료) "
    "→ 생략하고 강제 단계로 직행"
)


def cys_status():
    rc, out, _ = run(["cys", "status", "--json"], timeout=12)
    if rc != 0:
        return None
    try:
        return json.loads(out)
    except Exception:
        return None


def cys_list_rows():
    """cys list 의 모든 행을 {surface_ref, role, pid, exited} 로 파싱.
    ★key=value 컬럼을 위치가정 없이 전부 훑는다(codex R1 논쟁점: 컬럼순서 변동 견고화)."""
    rc, out, _ = run(["cys", "list"], timeout=12)
    rows = []
    if rc != 0:
        return rows
    for ln in out.splitlines():
        cols = ln.split("\t")
        if not cols or not cols[0].strip().startswith("surface:"):
            continue
        row = {"surface_ref": cols[0].strip(), "role": None, "pid": None, "exited": None}
        for c in cols[1:]:
            if "=" not in c:
                continue
            k, v = c.split("=", 1)
            k, v = k.strip(), v.strip()
            if k == "role":
                row["role"] = v
            elif k == "pid":
                row["pid"] = int(v) if v.isdigit() else None
            elif k == "exited":
                row["exited"] = (v == "true")
        rows.append(row)
    return rows


def role_surface_row(role):
    """role 을 가진 비종료 surface 1건 반환(없으면 None)."""
    for r in cys_list_rows():
        if r["role"] == role and r["exited"] is False:
            return r
    return None


def status_surface(status, role):
    for s in status.get("surfaces", []):
        if s.get("role") == role and not s.get("exited"):
            return s
    return None


def _pid_for_surface_ref(surface_ref):
    """주어진 surface_ref 의 비종료 row 의 pid(없으면 None). status 를 그 surface 생애에 결박."""
    for r in cys_list_rows():
        if r["surface_ref"] == surface_ref and r["exited"] is False:
            return r["pid"]
    return None


# ─────────────────── 상태 계약 3분리 ───────────────────
# Windows 실행 확장자 화이트리스트(Rust governance.rs WIN_EXEC_EXTS 와 동일 목록·파리티).
# ★임의 확장자 strip 금지 — 목록을 열면 무관 파일이 에이전트로 오판된다.
WIN_EXEC_EXTS = ("cmd", "bat", "exe", "ps1", "com")


def _norm_comm(comm):
    """comm → 비교용 정규명. ①경로 구분자 `/`·`\\` 양쪽으로 basename 추출(Windows 경로는
    posix os.path.basename 이 쪼개지 못한다) ②lower ③등록 실행 확장자 **1개만** 제거.
    ★AGENT_COMM 값 확장이 아니라 입력 정규화로 해결한다 — 원천 명단(정본 바이너리명) 유지."""
    s = (comm or "").strip()
    for sep in ("\\", "/"):
        s = s.rsplit(sep, 1)[-1]
    s = s.lower()
    i = s.rfind(".")
    # i>0: `.cmd` 같은 도트파일(본체가 빈 문자열)은 strip 대상이 아니다.
    if i > 0 and s[i + 1:] in WIN_EXEC_EXTS:
        s = s[:i]
    return s


def _comm_matches(comm, agent):
    """comm(ps -o comm= 결과·macOS 는 전체경로)의 basename 이 agent 고유 바이너리와 동등하면 True.
    ★미지/빈 agent 는 후보 없음→False(wildcard 차단·R1 결함2)·basename 동등(substring 오매칭 차단·R2 논쟁점).
    ★확장자 정규화(2026-07-29 현장 결함 2호 파리티): Windows comm 은 `claude.exe` 로 관측돼
    기본 설정에서도 `("claude",)` 와 불일치→False 였다(잠복 결함). 확장자를 벗겨 성립시킨다.
    단, 개명 래퍼명(`claude-2.cmd`→`claude-2`)은 여기서 여전히 False 다 — 이 함수는 *정본
    바이너리명* 동등 비교이고, 래퍼명 매칭은 cysd 쪽 cmdline_matches_agent 의 책임이다."""
    names = AGENT_COMM.get(agent or "", ())
    if not names:
        return False
    return _norm_comm(comm) in names


def process_present(pid, agent):
    """surface 루트 pid 자손에 agent 고유 프로세스가 살아있으면 True.
    ★recovery/생존추정 보조 전용 — 각성/READY 의 단독 근거로 쓰지 않는다(codex R1 결함1·5)."""
    if not pid or not AGENT_COMM.get(agent or ""):
        return False
    # RC-6: pgrep/ps는 unix 전용 — Windows엔 부재. 이 함수는 보조 생존추정이라(단독 근거 아님)
    # Windows에선 child-scan을 건너뛰고 False로 degrade한다(READY 판정은 화면 marker 등 타 근거 사용).
    if os.name != "posix":
        return False
    seen, frontier = set(), [pid]
    while frontier:
        p = frontier.pop()
        if p in seen:
            continue
        seen.add(p)
        rc, out, _ = run(["pgrep", "-P", str(p)], timeout=5)
        if rc != 0:
            continue
        for c in out.split():
            if not c.isdigit():
                continue
            cpid = int(c)
            _, comm, _ = run(["ps", "-o", "comm=", "-p", str(cpid)], timeout=5)
            if _comm_matches(comm, agent):
                return True
            frontier.append(cpid)
    return False


def awake_ready(status, role):
    """★각성 판정 — awakened_at 래치 OR agent_alive OR fresh set-status(프로세스 제외).
    빈 CLI(디렉티브 미수신)를 각성으로 오인증하지 않는다(codex R1 결함1·5).

    ★W2 B6 — **반전이 아니라 보강**이다(W2 게이트 명문): 래치를 **1순위 확정 근거로 추가**하고,
      기존 균형 술어(agent_alive OR fresh set-status — codex R1/R2 적대검증 산물)는 래치 부재
      기계를 위한 **폴백으로 보존**한다. 여기서 agent_alive 를 빼는 '반전'을 하면 래치 배포 이전
      기계의 건강한 전 팀이 already_up 을 잃고 재입양·재주입 대상이 된다(A1 역방향).
      '각성 확정 vs 생존추정'의 등급 구분은 `node_liveness` 가 담당한다 — 그쪽이 라벨링·결손
      판정의 SOT 이고, 이 함수는 boot_node 프로토콜의 '재기동 생략' 게이트다(층위가 다르다)."""
    s = status_surface(status, role)
    if s is None:
        return False, "surface 없음"
    latch = awakened_latch(status, role)
    if latch is not None:
        return True, "awakened_at 래치(%d — 각성 확정)" % int(latch)
    if s.get("agent_alive"):
        return True, "agent_alive(래치 부재=legacy-presumed 폴백)"
    st = s.get("status") or {}
    age = st.get("age_secs")
    if isinstance(age, (int, float)) and age <= STATUS_FRESH_SECS and st.get("state"):
        return True, "set-status(%s·age%ss)" % (st.get("state"), int(age))
    return False, "각성신호 없음"


def quiet_but_alive(status, role):
    """각성 이력(set-status state 존재) + 현재 surface_ref 에 결박된 pid 의 기대 agent 프로세스 생존.
    ★프로세스 단독 인증 아님: status.state 가 있어야(=각성 이력) 후보 → 빈 CLI(status 없음) 배제.
    ★status 를 그 surface_ref 의 현재 pid 에 결박해 litter/exited row·과거이력 오인 차단(codex R2 결함1·5).
    주입실패로 agent_alive 가 None 으로 굳은 idle 노드(set-status 노후화)가 살아있음을 인정하는 용도."""
    s = status_surface(status, role)
    if s is None:
        return False
    st = s.get("status") or {}
    if not st.get("state"):           # 각성 이력 없음 → 인증 안 함(빈 CLI 차단)
        return False
    ref = s.get("surface_ref")
    pid = _pid_for_surface_ref(ref) if ref else None
    if not pid:
        return False
    agent = s.get("agent") or ROLE_AGENT.get(role, "")
    return process_present(pid, agent)


def node_alive(status, role):
    """★생존 술어 단일화(codex R2 결함2): orchestra READY 보강과 reclaim '죽음' 판정이 공유.
    awake_ready(각성) OR quiet_but_alive(각성이력+프로세스). 둘 다 아니면 '죽음/미각성'.

    ★W2 이후 이 함수는 `node_liveness` 의 **얇은 래퍼**다(bool 소비처 하위호환) — 등급이 필요한
      소비처는 node_liveness 를 직접 쓴다. unknown 은 여기서 True(=보류측)로 접힌다:
      이 함수의 유일한 파괴 소비처가 reclaim 이고, 파괴 경로의 Unknown 은 무조건 hold 다."""
    grade, _ = node_liveness(status, role)
    return grade in (LIVENESS_AWAKE, LIVENESS_PRESUMED, LIVENESS_UNKNOWN)


# ─────────────── 공유 술어 ①: node_liveness (단일 정의 · 전 소비처 import) ───────────────
def awakened_latch(status, role):
    """데몬 SOT 의 `awakened_at` 래치 값(epoch초) — 없으면 None.

    ★단방향(금지 방향 ⑦): 값이 있으면 '주입 후 최소 1회 set-status 를 받았다'는 **확정**이고,
      없으면 **아무것도 말하지 않는다**(구 데몬·업그레이드 전 각성·재시작 직후 = legacy-presumed).
      그래서 이 함수의 None 은 어떤 소비처에서도 'NOT-awake' 로 해석되지 않는다."""
    s = status_surface(status, role)
    if s is None:
        return None
    v = s.get("awakened_at")
    return v if isinstance(v, (int, float)) and v > 0 else None


def seat_state(status, role):
    """좌석(커널 사실) — "occupied"|"empty"|"unknown", 또는 **None=좌석 차원 없음**.

    ★필드 부재(구 데몬·SEAT 도입 이전)와 "unknown"(현 데몬이 프로브 실패를 명시 보고)을
      **절대 융합하지 않는다**. 융합하면 구 데몬에서 전 노드가 영구 unknown → 파괴 경로의
      무조건 hold 규칙과 맞물려 **reclaim 이 영구 마비**된다(회수 불가 = 막힌 좌석 영구화).
      필드 부재는 '이 차원에 대해 아무 말도 없음'이므로 좌석 신호를 쓰지 않고 W2 이전 균형
      술어로 흐르게 한다 — awakened_at 래치의 단방향 규약과 동형(부재 ≠ 부정)."""
    s = status_surface(status, role)
    if s is None:
        return None
    v = s.get("seat")
    if v in ("occupied", "empty", LIVENESS_UNKNOWN):
        return v
    return None


def node_liveness(status, role):
    """★공유 술어 ① — (grade, reason). 등급 정의·불변식은 모듈 docstring 참조.

    판정 순서가 곧 신호의 권위 순서다:
      ① awakened_at 래치      → awake_confirmed  (데몬 SOT·영속·단방향)
      ② 신선 set-status ack   → awake_confirmed  (부트 성공의 계약)
      ③ agent_alive 단독      → alive_presumed   (★B6: 각성 아님 — 빈 CLI 도 프로세스는 산다)
      ④ 좌석 occupied         → alive_presumed   (커널 사실: 자손 프로세스 존재)
      ⑤ quiet_but_alive       → alive_presumed   (각성이력 + pid 결박 프로세스)
      ⑥ 좌석 unknown          → unknown          (판정 불가 — 이원 규칙 소비)
      ⑦ 그 밖                 → absent           (좌석 없음/exited/좌석 비었음/구 데몬 무신호)

    ★구 데몬(seat 필드 부재)은 ⑥을 건너뛰고 ⑦로 흐른다 — W2 이전 균형 술어와 동일 결론이다
      (부재 ≠ 판정불가. 융합하면 구 데몬에서 reclaim 이 영구 마비된다 — seat_state 주석 참조).
    """
    s = status_surface(status, role)
    if s is None:
        return LIVENESS_ABSENT, "좌석 없음(role 보유 비종료 surface 부재)"
    latch = awakened_latch(status, role)
    if latch is not None:
        return LIVENESS_AWAKE, "awakened_at 래치(%d)" % int(latch)
    st = s.get("status") or {}
    age = st.get("age_secs")
    if isinstance(age, (int, float)) and age <= STATUS_FRESH_SECS and st.get("state"):
        return LIVENESS_AWAKE, "set-status(%s·age%ss)" % (st.get("state"), int(age))
    if s.get("agent_alive"):
        # ★B6: 종전엔 이 신호가 곧 'awake' 였다(self-test 가 그 오답을 박제 중이었다).
        #   프로세스 생존은 '각성'의 증거가 아니다 — 강등해 정직하게 라벨링한다.
        return LIVENESS_PRESUMED, "agent_alive 단독(각성 미확인 — 생존추정·래치 부재=legacy)"
    seat = seat_state(status, role)
    if seat == "occupied":
        return LIVENESS_PRESUMED, "좌석 점유(자손 프로세스 존재·각성 미확인)"
    if quiet_but_alive(status, role):
        return LIVENESS_PRESUMED, "각성이력+pid결박 프로세스 생존(set-status 노후)"
    if seat == LIVENESS_UNKNOWN:
        return LIVENESS_UNKNOWN, "좌석 판정 불가(프로브 실패·구 데몬 필드 부재)"
    return LIVENESS_ABSENT, "좌석 비었음(seat=empty·각성신호 없음)"


def resolve_unknown_for_spawn(role, requery, probe=None, tick_s=None):
    """★Unknown 이원 규칙(비평2 B-2)의 **스폰 경로** 절반 — 시한부 해소.

    파괴 경로(kill·reclaim)의 Unknown 은 무조건 hold 이므로 여기 오지 않는다(fail-closed).
    스폰 경로(boot)의 Unknown 을 영구 hold 하면 GUI 콜드스타트(main.rs:2138 이 앱 시작 즉시
    spawn_orchestra_boot)에서 좌석 캐시가 아직 안 채워진 창에 술어가 구 `!exited` 로 퇴화해
    B3 를 보존한다 — 그래서 **가용성 우선**으로 해소한다:

      워치독 1주기(≤5s) 대기 → 재조회 1회 → 여전히 Unknown 이면 프로브 1회 →
      그래도 불명이면 **결손 취급(스폰)**. 중복 스폰은 boot 락이 방어한다(G11·G12).

    requery() → status(dict|None), probe() → bool|None(프로세스 관측). 둘 다 주입 가능(밀폐 테스트).
    반환: (grade, reason) — grade 는 절대 unknown 이 아니다(시한부 해소의 정의).
    """
    tick = tick_s if tick_s is not None else SEAT_WATCHDOG_TICK_S
    time.sleep(max(0.0, float(tick)))
    st2 = requery()
    if st2 is not None:
        grade, why = node_liveness(st2, role)
        if grade != LIVENESS_UNKNOWN:
            return grade, "워치독 1주기 후 해소 — %s" % why
    if probe is not None:
        try:
            seen = probe()
        except Exception:
            seen = None
        if seen is True:
            return LIVENESS_PRESUMED, "재조회 불명 → 프로브 1회에서 프로세스 관측(생존추정)"
        if seen is False:
            return LIVENESS_ABSENT, "재조회 불명 → 프로브 1회에서 프로세스 부재(결손 취급·스폰)"
    return LIVENESS_ABSENT, ("잔존 불명(워치독 1주기+프로브 1회) → 결손 취급·스폰"
                             " — 중복은 boot 락이 방어")


# ─────────────── 공유 술어 ②: role_family (pack.rs:1674-1684 접두 의미 미러) ───────────────
# ★Rust 정본: `src/pack.rs::role_directive_path` 의 접두 분기(master / worker* / cso* / reviewer*).
#   B10·G26·G2·A3 의 공통 근저는 "같은 역할 가족 판정이 언어마다 재발명됨"이다. 여기가 python 측
#   단일 정의이고, H-PRED-5 가 데몬이 발권 가능한 전 role × 소비처 전수를 기계 대조한다.
#   ★접두 관용의 **경계**: 가족 판정은 '어느 디렉티브를 받는가'(=지침 배선)용이다. 의무 슬롯
#   충족 판정에 쓰면 G26 이 재발한다(reviewer-grok 이 의무 리뷰어 슬롯을 채운 것으로 계상).
#   슬롯 충족은 `javis_orchestra.slot_satisfied` 가, 정확일치 요건은 `role_matches_requirement` 가 담당.
ROLE_FAMILIES = ("master", "worker", "cso", "reviewer")


def role_family(role):
    """role → 가족명("master"|"worker"|"cso"|"reviewer") 또는 None(가족 없음).
    ★master 는 **정확일치**다(pack.rs 와 동일) — 'master-2' 는 가족이 없다(발권되지 않는 이름).
    그 밖은 접두 일치(worker-2·cso-1·reviewer-gemini·reviewer-claude-1 …)."""
    r = (role or "").strip()
    if not r:
        return None
    if r == "master":
        return "master"
    for fam in ("worker", "cso", "reviewer"):
        if r.startswith(fam):
            return fam
    return None


def role_matches_requirement(required, candidate):
    """의무 역할 `required` 가 라이브 역할 `candidate` 로 충족되는가 — **정확일치 + worker 접두만**.

    ★orchestra.cmd_check 의 수용 규약과 byte-equivalent 여야 한다(쌍둥이 규약의 단일화):
      worker 는 데몬이 둘째부터 worker-N 으로 dedup 하므로 접두 수용이 **필수**이고, cso-N·
      reviewer-* 의 관용은 check 에 없으므로 여기에도 없다(G26 의 원인 = 이 관용).
    """
    if required == candidate:
        return True
    if required == "worker":
        return candidate == "worker" or candidate.startswith("worker-")
    return False


def requirement_satisfied(required, live_roles):
    """의무 역할 하나가 라이브 role 집합으로 충족되는가(role_matches_requirement 의 집합판)."""
    return any(role_matches_requirement(required, c) for c in live_roles)


def post_inject_ack(status, role, elapsed, t_inject=None):
    """주입 *이후* 발신된 fresh set-status(=각성 ack)면 True.
    ★age < 주입후 경과시간(엄격·+마진 없음 — codex R2 결함3): 주입 *전* 보고는 age=age0+elapsed>elapsed
    이라 수학적으로 통과 불가. 오직 t_inject 뒤 발신만 통과한다.

    ★W2 B6/B14 추가 근거: `awakened_at` 래치가 t_inject 이후 시각이면 그것도 ack 다(데몬이 첫
      set-status 를 받은 시각을 못박은 값 — age 계산보다 강한 증거다). t_inject 미전달 시엔
      래치 근거를 쓰지 않는다(구 호출부 하위호환 · 과거 래치의 오통과 차단)."""
    s = status_surface(status, role)
    if s is None:
        return False
    if t_inject is not None:
        latch = awakened_latch(status, role)
        if latch is not None and latch >= t_inject:
            return True
    st = s.get("status") or {}
    age = st.get("age_secs")
    return isinstance(age, (int, float)) and bool(st.get("state")) and age < elapsed


# ─────────────── B11: 주입 배달 3분기 (pending / dropped / delivered-무ack) ───────────────
DELIVERY_PENDING = "pending"
DELIVERY_DROPPED = "dropped"
DELIVERY_DELIVERED_NO_ACK = "delivered_no_ack"


def classify_delivery(queue_entries, surface_row, marker):
    """★순수 판정(B11) — 주입 verdict 타임아웃 시 '무엇이 실패했는가'를 타입으로 가른다.

    종전 보고는 "유실"이었지만 재검증이 확정한 실체는 '무기한 지연 + 미배달/배달-무ack **구분
    불가**'다. 구분이 없으면 처방이 갈린다:
      pending           = 큐에 아직 남아 있다 → **재전송 금지**(맹목 재전송이 wakeup 홍수의 원인).
      dropped           = 큐도 비었고 좌석도 죽었다 → 재기동 격상(재주입 무의미).
      delivered_no_ack  = 배달됐으나 ack 없음 → **멱등 가드 1회** 재주입 후 증거 동봉 에스컬레이션.

    queue_entries: `cys queue list --surface <ref>` 파싱 결과(preview 문자열 목록).
    surface_row  : cys_list_rows() 의 해당 행(None=좌석 소멸).
    marker       : 우리 주입문에서 뽑은 식별 조각(preview 는 80자 절단이므로 앞부분에서 취한다).
    """
    if any(marker and marker in (p or "") for p in queue_entries):
        return DELIVERY_PENDING, "큐에 주입문 잔존(미배달·대기) — 재전송 금지"
    if surface_row is None or surface_row.get("exited") is not False:
        return DELIVERY_DROPPED, "큐 비었고 좌석 소멸/종료 — 배달 유실(재기동 격상)"
    return DELIVERY_DELIVERED_NO_ACK, "큐 비었고 좌석 생존 — 배달됨·ack 부재(멱등 1회 재주입)"


def queue_previews(surface_ref, timeout=8):
    """`cys queue list --surface <ref>` 의 preview 열 목록(조회 실패=빈 목록·보수)."""
    rc, out, _ = run(["cys", "queue", "list", "--surface", surface_ref], timeout=timeout)
    if rc != 0:
        return []
    previews = []
    for ln in out.splitlines():
        cols = ln.split("\t")
        if len(cols) >= 4:
            previews.append(cols[3])
    return previews


# ─────────────────── 주입 ───────────────────
def awaken_message(role):
    name, state = ROLE_DIRECTIVE.get(role, ("해당 역할 절대지침", "working"))
    # ★claim-role 은 *풀 role* 그대로(reviewer-gemini/reviewer-codex) — generic 'reviewer' 금지(codex R1 결함3).
    return (
        "너는 이 cys 워크스페이스의 %s 노드다. 즉시 각성하라: "
        "① pack 디렉티브 폴더의 %s 문서와 soul 헌장을 읽고 정체를 확정 "
        "② cys claim-role %s 로 역할 확인(이미 보유 시 cys list 로 확인만) "
        "③ cys set-status --state %s --context 5 로 생존 신호 발신 "
        "④ 너의 TODO 파일 확인·복원 "
        "⑤ 각성 완료를 'cys send --to master' 후 'cys send-key --to master Return' 으로 master 에게 push 보고하라."
        % (role, name, role, state)
    )


def inject(role, msg, attempts=4):
    """★`cys send --queued` 단일경로(codex R2 결함4): 큐는 대상이 조용해질 때 메시지+자동 Return 을
    원자적으로 배달한다 → typing_guard 우회(F1)·Return 분리 실패로 인한 중복 입력 위험 제거.
    (idle-then-inject 로 이미 안착했으므로 큐는 즉시 배달된다.)"""
    for i in range(attempts):
        rcq, _, errq = run(["cys", "send", "--queued", "--to", role, msg], timeout=12)
        if rcq == 0:
            return True, "주입 큐 등록(자동 Return 배달·시도 %d)" % (i + 1)
        time.sleep(2)
    return False, "주입 실패(%d회·큐 등록 실패: %s)" % (attempts, (errq or "").strip()[:80])


# ─────────────────── 회수(F5) ───────────────────
def _reclaim_verdict(fresh_st, role, pid, cur_pid):
    """kill 직전 최종 허용 판정 — **순수함수**(cys 호출·부작용 0·self_test 결정론 대상).
    입력의 fresh_st·cur_pid 는 호출부가 *신선하게* 조회해 넘긴다(조회는 부작용이라 밖에 둔다).
    반환: "kill" | "hold-alive" | "hold-pid" | "hold-status".
    ★보류 우선 원칙: 조금이라도 불확실하면 살려둔다 — 죽은 노드를 못 죽이는 손해(재시도 가능)
    보다 산 노드를 죽이는 손해(작업 소실·비가역)가 훨씬 크다."""
    if fresh_st is None:
        return "hold-status"          # 상태 불명 → 판단 불가 → 보존
    if node_alive(fresh_st, role):
        return "hold-alive"           # 되살아났다 → 오살 직전 회피
    if not pid or cur_pid != pid:
        return "hold-pid"             # pid 가 갈렸다 → 엉뚱한 프로세스 종료 방지
    return "kill"



def reclaim(role, emit):
    """막힌(죽은) 미각성 surface 결정론 회수. ★node_alive(orchestra 와 동일 술어)가 True 면 절대 종료
    금지 — 건강한 quiet 노드 오살 차단(codex R2 결함2·4). 기대 agent 는 ROLE_AGENT 로 강제(인자 불신·R2 #5).
    종료 직전 surface_ref·pid 재확인(R2 #4)."""
    st = cys_status()
    if st is None:
        emit("reclaim", "cys status 실패 — 회수 보류")
        return 2
    if node_alive(st, role):
        ready, why = awake_ready(st, role)
        emit("reclaim", "%s 는 생존(%s) — 회수 대상 아님(중단)" % (role, why if ready else "quiet_but_alive"))
        return 1
    row = role_surface_row(role)
    if row is None:
        emit("reclaim", "%s role 보유 surface 없음 — 회수 불필요" % role)
        return 0
    exp_agent = ROLE_AGENT.get(role, "")   # ★인자 --agent 불신·role 기대 agent 강제
    ref, pid = row["surface_ref"], row["pid"]
    # 종료 직전 재확인: 같은 surface_ref 의 현재 pid 가 동일한가
    if _pid_for_surface_ref(ref) != pid or not pid:
        emit("reclaim", "%s pid 불일치/부재 — 회수 보류(잘못된 종료 방지)" % ref)
        return 1
    # ★오살 창 축소(2026-07-29 현장 결함 2호) — **완전 차단이 아니다**(codex R1 minor).
    # 재조회와 _kill 사이에도 창은 남는다(TOCTOU 는 원리상 소거 불가·OS 원자 연산 부재).
    # 여기서 하는 일은 그 창을 status 1회 왕복 길이로 *좁히는* 것뿐이다.
    # 결함 2호는 생존 매처가 확장자를 벗기지 않아 agent_alive 를 *영구* false 로 만들어
    # '오판→오살' 사슬을 성립시켰다 — 창이 좁든 넓든 판정이 상시 거짓이면 무의미했다.
    # 매처 수리(governance.rs 확장자 정규화)로 agent_alive 가 신뢰 가능해진 지금에야
    # 이 재조회가 실질 효과를 갖는다(수리 1이 선행 조건).
    # ★새 판정 규칙 도입 없음 — 동일한 node_alive 술어를 신선한 status 로 한 번 더 물을 뿐.
    verdict = _reclaim_verdict(cys_status(), role, pid, _pid_for_surface_ref(ref))
    if verdict != "kill":
        emit("reclaim", {
            "hold-status": "%s kill 직전 status 재조회 실패 — 회수 보류(불확실할 땐 보존)" % ref,
            "hold-alive": "%s 가 kill 직전 재조회에서 생존 — 회수 중단(오살 창 축소)" % role,
            "hold-pid": "%s kill 직전 pid 재확인 불일치 — 회수 보류(엉뚱한 pid 종료 방지)" % ref,
        }[verdict])
        return 1
    # ★G24: 1차 그레이스풀은 **효과가 있는 플랫폼에서만** 시도한다. 무효 플랫폼에서는 시도했다고
    #   기록하지 않고(보고=실측), 사유를 남긴 뒤 강제 단계로 직행한다(무의미한 1.5s 지연 제거).
    if GRACEFUL_KILL_SUPPORTED:
        _kill(pid)
        time.sleep(1.5)
        need_force = role_surface_row(role) is not None and _pid_for_surface_ref(ref) == pid
    else:
        emit("reclaim", GRACEFUL_KILL_NOOP_REASON)
        need_force = True
    if need_force:
        _kill(pid, force=True)
        time.sleep(1.5)
    if role_surface_row(role) is None:
        emit("reclaim", "%s(pid=%s·exp_agent=%s) 종료·role 해제 완료 — 헬퍼로 재기동 가능"
             % (ref, pid, exp_agent))
        return 0
    emit("reclaim", "%s 종료했으나 role 미해제 — 수동 점검 필요" % ref)
    return 1


# ─────────────────── self-test(순수함수 회귀) ───────────────────
def self_test():
    fails = []

    def chk(cond, msg):
        if not cond:
            fails.append(msg)

    # agent 매칭: basename 동등·빈/미지/오매칭 차단(R1 결함2·R2 논쟁점)
    chk(_comm_matches("/x/bin/codex", "codex") is True, "codex 매칭 실패")
    chk(_comm_matches("node", "codex") is False, "node→codex 오매칭")
    chk(_comm_matches("/x/bin/agy", "gemini") is True, "agy(gemini) 매칭 실패")
    chk(_comm_matches("my-claude-helper", "claude") is False, "substring 오매칭(my-claude-helper)")
    chk(_comm_matches("anything", "") is False, "빈 agent wildcard 오탐")
    chk(_comm_matches("claude", None) is False, "None agent wildcard 오탐")
    chk(process_present(123, "") is False, "빈 agent process_present 오탐")

    # ★확장자 정규화 파리티(2026-07-29 현장 결함 2호): Windows comm 은 `claude.exe` 로 관측된다.
    chk(_comm_matches("claude.exe", "claude") is True, "claude.exe 정규화 실패(기본설정 잠복결함)")
    chk(_comm_matches("C:\\Users\\x\\.local\\bin\\claude.exe", "claude") is True,
        "Windows 전체경로 basename 분해 실패")
    chk(_comm_matches("CLAUDE.EXE", "claude") is True, "comm 대소문자 무시 회귀")
    chk(_comm_matches("agy.exe", "gemini") is True, "agy.exe(gemini) 정규화 실패")
    chk(_comm_matches("codex.cmd", "codex") is True, "codex.cmd 정규화 실패")
    # ★개명 래퍼명은 여기서 여전히 False — 이 함수는 *정본 바이너리명* 동등 비교이고,
    #   래퍼명(`claude-2`) 매칭은 cysd 의 cmdline_matches_agent 책임이다(역할 경계 박제).
    chk(_comm_matches("claude-2.cmd", "claude") is False, "래퍼명이 정본명 비교를 통과(경계 붕괴)")
    # 등록 외 확장자는 이름 본체 — strip 금지(무관 파일 오판 차단)
    chk(_comm_matches("claude.something", "claude") is False, "미등록 확장자 strip 오탐")
    chk(_comm_matches("claude.backup", "claude") is False, "미등록 확장자 strip 오탐(.backup)")
    chk(_comm_matches("/usr/local/bin/.cmd", "claude") is False, "도트파일 strip 후 빈이름 오탐")
    chk(_comm_matches("my-claude-helper.exe", "claude") is False, "substring 오매칭(확장자 경유)")

    # ★_reclaim_verdict 4분기(codex R1 minor·missing3): kill 직전 최종 게이트를 순수함수로
    #   떼어내 cys 스텁 없이 결정론 검증한다. 픽스처는 _pid_for_surface_ref 호출 전에
    #   단락되도록 구성(agent_alive=True 는 awake_ready 즉시 True / status=None 은 상태 부재 즉시 False).
    v_alive = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": True}]}
    v_dead = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": False, "status": None}]}
    chk(_reclaim_verdict(None, "cso", 100, 100) == "hold-status", "status 실패인데 kill 허용")
    chk(_reclaim_verdict(v_alive, "cso", 100, 100) == "hold-alive", "생존 노드에 kill 허용(오살)")
    chk(_reclaim_verdict(v_dead, "cso", 100, 999) == "hold-pid", "pid 불일치인데 kill 허용")
    chk(_reclaim_verdict(v_dead, "cso", 0, 0) == "hold-pid", "pid 부재인데 kill 허용")
    chk(_reclaim_verdict(v_dead, "cso", 100, 100) == "kill", "죽은 노드 회수 불가(회수 마비)")
    # 우선순위: 생존 판정이 pid 불일치보다 앞선다(어느 쪽이든 보류라 결과 동일 — 계약 고정)
    chk(_reclaim_verdict(v_alive, "cso", 100, 999) == "hold-alive", "보류 사유 우선순위 계약 이탈")
    # ★W2 Unknown 이원 규칙의 파괴 경로 절반: 좌석 판정불가는 **무조건 hold**(fail-closed).
    v_unknown_seat = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": False,
                                    "status": None, "seat": "unknown"}]}
    chk(_reclaim_verdict(v_unknown_seat, "cso", 100, 100) == "hold-alive",
        "좌석 판정불가(unknown)인데 kill 허용 — 파괴 경로 fail-closed 위반(오살)")
    # 좌석이 명시적으로 empty 면 죽음 확정 — 회수 가능(가용성 보존)
    v_empty_seat = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": False,
                                  "status": None, "seat": "empty"}]}
    chk(_reclaim_verdict(v_empty_seat, "cso", 100, 100) == "kill",
        "좌석 empty(죽음 확정)인데 회수 거부 — 막힌 좌석 영구화")

    # awake_ready: 프로세스 제외·fresh/stale 구분(R1 결함1)
    only_proc = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": None, "status": None}]}
    chk(awake_ready(only_proc, "cso")[0] is False, "프로세스만으로 awake 오판(주입 skip 위험)")
    chk(awake_ready({"surfaces": [{"role": "cso", "exited": False, "agent_alive": True}]}, "cso")[0] is True,
        "agent_alive awake 미인정")
    fresh = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": None,
                           "status": {"age_secs": 10, "state": "working"}}]}
    chk(awake_ready(fresh, "cso")[0] is True, "fresh set-status awake 미인정")
    stale = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": None,
                           "status": {"age_secs": 9999, "state": "working"}}]}
    chk(awake_ready(stale, "cso")[0] is False, "stale set-status awake 오인정")

    # reviewer claim-role 풀네임(R1 결함3)·각성 메시지 .md 미포함(F4)
    chk("cys claim-role reviewer-codex" in awaken_message("reviewer-codex"), "reviewer-codex claim 풀네임 누락")
    chk("cys claim-role reviewer-gemini" in awaken_message("reviewer-gemini"), "reviewer-gemini claim 풀네임 누락")
    chk("claim-role reviewer " not in awaken_message("reviewer-codex"), "generic reviewer claim 잔존")
    chk(".md" not in awaken_message("cso"), "각성 메시지 .md 포함(헌법가드 오탐)")
    # ★무구독 폴백 슬롯(오너 2026-06-14): Claude 대체 리뷰어 역할이 claude 로 매핑·REVIEWER 각성
    chk(ROLE_AGENT.get("reviewer-claude-1") == "claude", "reviewer-claude-1 agent 매핑 누락")
    chk(ROLE_AGENT.get("reviewer-claude-2") == "claude", "reviewer-claude-2 agent 매핑 누락")
    chk("cys claim-role reviewer-claude-1" in awaken_message("reviewer-claude-1"), "reviewer-claude-1 claim 풀네임 누락")
    chk("REVIEWER" in awaken_message("reviewer-claude-2"), "reviewer-claude-2 REVIEWER 디렉티브 미지정")

    # post_inject_ack: 엄격 age<elapsed·+마진 없음(R2 결함3 — 경계 케이스 a/c)
    ackable = {"surfaces": [{"role": "cso", "exited": False, "status": {"age_secs": 3, "state": "working"}}]}
    chk(post_inject_ack(ackable, "cso", elapsed=20) is True, "주입후 fresh ack 미인정")
    chk(post_inject_ack(stale, "cso", elapsed=20) is False, "주입전 stale 을 ack 오인정")
    edge = {"surfaces": [{"role": "cso", "exited": False, "status": {"age_secs": 1, "state": "working"}}]}
    chk(post_inject_ack(edge, "cso", elapsed=0) is False, "age=1,elapsed=0 경계를 ack 오인정(+마진 잔존)")
    chk(post_inject_ack(edge, "cso", elapsed=1) is False, "age=1,elapsed=1(동일) ack 오인정(엄격 < 위반)")

    # ─────────── W2 보강(반전 아님): awakened_at 래치·liveness 3등급·role_family·배달 3분기 ───────────
    # ★기존 균형 술어 검체(위 awake_ready 블록)는 **폐기하지 않고 폴백 검체로 보존**한다 —
    #   래치 배포 이전 기계의 정당한 동작이며, 그것이 legacy-presumed 계약의 실체다(H-PRED-3).
    def surf(**kw):
        base = {"role": "cso", "exited": False}
        base.update(kw)
        return {"surfaces": [base]}

    latched = surf(agent_alive=None, status=None, awakened_at=1_700_000_000.0)
    chk(awakened_latch(latched, "cso") == 1_700_000_000.0, "래치 값 판독 실패")
    chk(awake_ready(latched, "cso")[0] is True, "래치 존재인데 각성 미확정")
    chk(node_liveness(latched, "cso")[0] == LIVENESS_AWAKE, "래치 존재인데 awake_confirmed 아님")
    # ★단방향: 래치 부재 + agent_alive → legacy-presumed(재스폰·재주입 금지 · NOT-awake 단정 금지)
    legacy = surf(agent_alive=True, status=None)
    chk(awakened_latch(legacy, "cso") is None, "래치 부재 판독 실패")
    chk(node_liveness(legacy, "cso")[0] == LIVENESS_PRESUMED,
        "agent_alive 단독이 awake 로 계상됨(B6 오답 박제 잔존)")
    chk(awake_ready(legacy, "cso")[0] is True,
        "래치 부재+agent_alive 를 NOT-awake 로 단정(금지 방향 ⑦ 위반 — 재주입·재스폰 유도)")
    chk(node_alive(legacy, "cso") is True, "legacy-presumed 를 죽음으로 판정(오살 위험)")
    # 데몬 재시작·업그레이드 fixture: 전 팀이 래치 없음 + agent_alive True → 전원 presumed(NOT-awake 0)
    team = {"surfaces": [{"role": r, "exited": False, "agent_alive": True}
                         for r in ("cso", "worker", "reviewer-gemini", "reviewer-codex")]}
    grades = [node_liveness(team, r)[0] for r in ("cso", "worker", "reviewer-gemini", "reviewer-codex")]
    chk(all(g == LIVENESS_PRESUMED for g in grades),
        "데몬 재시작 fixture 에서 NOT-awake(absent) 오판 발생: %s" % grades)
    # fresh set-status → awake_confirmed (래치 없어도 부트 계약은 성립)
    fresh_ack = surf(agent_alive=None, status={"age_secs": 5, "state": "working"})
    chk(node_liveness(fresh_ack, "cso")[0] == LIVENESS_AWAKE, "신선 ack 가 awake_confirmed 아님")
    # 좌석 신호: occupied=presumed · empty=absent · 필드부재/unknown=unknown(판정불가 타입 분리)
    chk(node_liveness(surf(agent_alive=None, status=None, seat="occupied"), "cso")[0]
        == LIVENESS_PRESUMED, "좌석 occupied 가 presumed 아님")
    chk(node_liveness(surf(agent_alive=None, status=None, seat="empty"), "cso")[0]
        == LIVENESS_ABSENT, "좌석 empty 가 absent 아님")
    chk(node_liveness(surf(agent_alive=None, status=None, seat="unknown"), "cso")[0]
        == LIVENESS_UNKNOWN, "좌석 unknown 이 판정으로 융합됨(CS-2 위반)")
    # ★구 데몬(seat 필드 부재) = 좌석 차원 무신호 → W2 이전 결론(absent)로 흐른다.
    #   여기서 unknown 으로 융합하면 파괴 경로 hold 규칙과 맞물려 reclaim 이 영구 마비된다.
    chk(seat_state(surf(agent_alive=None, status=None), "cso") is None,
        "구 데몬 seat 필드 부재가 'unknown' 으로 융합됨(회수 마비 유발)")
    chk(node_liveness(surf(agent_alive=None, status=None), "cso")[0] == LIVENESS_ABSENT,
        "구 데몬(seat 부재)이 판정불가로 융합 — W2 이전 균형 술어 결론과 이탈")
    chk(node_liveness({"surfaces": []}, "cso")[0] == LIVENESS_ABSENT, "좌석 없음이 absent 아님")
    # ★Unknown 이원 규칙: 파괴 경로는 hold(node_alive True) / 스폰 경로는 시한부 해소(절대 unknown 반환 0)
    unk = surf(agent_alive=None, status=None, seat="unknown")
    chk(node_alive(unk, "cso") is True, "파괴 경로에서 Unknown 이 hold 되지 않음(오살 위험)")
    g1, _ = resolve_unknown_for_spawn("cso", lambda: surf(agent_alive=True, status=None), tick_s=0)
    chk(g1 == LIVENESS_PRESUMED, "워치독 1주기 재조회 해소 실패")
    g2, _ = resolve_unknown_for_spawn("cso", lambda: unk, probe=lambda: False, tick_s=0)
    chk(g2 == LIVENESS_ABSENT, "프로브 부재 관측이 결손 취급으로 이어지지 않음")
    g3, _ = resolve_unknown_for_spawn("cso", lambda: unk, probe=lambda: True, tick_s=0)
    chk(g3 == LIVENESS_PRESUMED, "프로브 관측이 생존추정으로 이어지지 않음")
    g4, _ = resolve_unknown_for_spawn("cso", lambda: None, tick_s=0)
    chk(g4 == LIVENESS_ABSENT, "잔존 불명이 스폰(가용성 우선)으로 해소되지 않음")
    chk(g4 != LIVENESS_UNKNOWN, "스폰 경로가 unknown 을 반환(시한부 해소 계약 위반)")

    # role_family — pack.rs:1674-1684 접두 의미 미러(master 정확일치·그 밖 접두)
    chk(role_family("master") == "master", "master 가족 판정 실패")
    chk(role_family("master-2") is None, "master 접두 관용(발권 불가 이름)")
    chk(role_family("worker-2") == "worker", "worker-2 가족 판정 실패")
    chk(role_family("cso-1") == "cso", "cso-1 가족 판정 실패")
    chk(role_family("reviewer-grok") == "reviewer", "reviewer-grok 가족 판정 실패")
    chk(role_family("reviewer-claude-1") == "reviewer", "대체 리뷰어 가족 판정 실패")
    chk(role_family("verifier") is None, "미지 role 이 가족 배정됨")
    chk(role_family("") is None and role_family(None) is None, "빈 role 가족 배정됨")
    # 의무 슬롯 충족은 정확일치+worker 접두만(G26 관용 금지)
    chk(role_matches_requirement("worker", "worker-2") is True, "worker-N dedup 수용 실패")
    chk(role_matches_requirement("cso", "cso-1") is False, "cso-1 이 의무 cso 를 충족(G26 재발)")
    chk(role_matches_requirement("reviewer-gemini", "reviewer-grok") is False,
        "reviewer-grok 이 의무 리뷰어 슬롯을 충족(G26 재발)")
    chk(requirement_satisfied("worker", {"worker-3"}) is True, "집합판 worker 접두 수용 실패")
    chk(requirement_satisfied("reviewer-codex", {"reviewer-claude-2"}) is False,
        "대체 좌석이 네이티브 의무 슬롯을 무단 충족")

    # B11 배달 3분기 — 맹목 재전송 차단의 근거
    alive_row = {"surface_ref": "surface:7", "role": "cso", "pid": 9, "exited": False}
    chk(classify_delivery(["너는 이 cys 워크스페이스의 cso"], alive_row, "너는 이 cys")[0]
        == DELIVERY_PENDING, "큐 잔존이 pending 으로 분기되지 않음(맹목 재전송 위험)")
    chk(classify_delivery([], None, "너는 이 cys")[0] == DELIVERY_DROPPED,
        "좌석 소멸이 dropped 로 분기되지 않음")
    chk(classify_delivery([], {"surface_ref": "surface:7", "exited": True}, "너는")[0]
        == DELIVERY_DROPPED, "exited 좌석이 dropped 로 분기되지 않음")
    chk(classify_delivery([], alive_row, "너는 이 cys")[0] == DELIVERY_DELIVERED_NO_ACK,
        "배달·무ack 분기 실패")
    # 래치 기반 ack — t_inject 이후 래치만 인정(과거 래치 오통과 차단)
    lt = surf(status=None, awakened_at=1000.0)
    chk(post_inject_ack(lt, "cso", elapsed=5, t_inject=900.0) is True, "주입후 래치 ack 미인정")
    chk(post_inject_ack(lt, "cso", elapsed=5, t_inject=1100.0) is False, "주입전 래치를 ack 오인정")
    chk(post_inject_ack(lt, "cso", elapsed=5) is False, "t_inject 미전달인데 래치로 ack 오인정")

    # ─────────── W4 · G24: 1차 그레이스풀 단계 유효성(H-WIN-10) ───────────
    # 플랫폼 술어가 kill 인자 형상과 **일치**하는지 — Windows 에서 무 /F 는 콘솔 프로세스에
    # 무동작이므로 '지원 안 함'으로 선언돼야 하고(그러면 회수는 강제 단계로 직행한다),
    # unix 에서는 SIGTERM 이 실효라 '지원'으로 선언돼야 한다(구 동작 보존).
    chk(GRACEFUL_KILL_SUPPORTED == (os.name != "nt"), "그레이스풀 지원 술어가 플랫폼과 불일치")
    chk(bool(GRACEFUL_KILL_NOOP_REASON) and "강제" in GRACEFUL_KILL_NOOP_REASON,
        "무효 사유 문구가 강제 단계 직행을 명시하지 않는다(조용한 생략 금지)")
    _src_bn = open(os.path.abspath(__file__), encoding="utf-8").read()
    chk("if GRACEFUL_KILL_SUPPORTED:" in _src_bn,
        "회수 경로가 그레이스풀 지원 술어로 분기하지 않는다(G24 미수리)")

    if fails:
        print("self-test FAIL:")
        for f in fails:
            print("  ✗ " + f)
        return 1
    print("self-test OK — %d 케이스 통과(상태계약 분리·basename매칭·확장자정규화·회수판정4분기·"
          "claim-role·엄격ack·F4·무구독폴백슬롯 + W2: 래치 단방향·liveness 3등급+판정불가·"
          "Unknown 이원·role_family·배달 3분기 + W4: G24 그레이스풀 유효성 3)" % (7 + 10 + 6 + 4 + 4 + 4 + 4 + 41 + 3))
    return 0


# ─────────────────── 메인 ───────────────────
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--role")
    ap.add_argument("--agent")
    ap.add_argument("--cwd", default=None)
    # ★예산 단일 소스(B9) — 기본값 하드코딩 금지. javis_budget leaf 가 SOT(감액 clamp 포함).
    ap.add_argument("--idle", type=float, default=float(budget("BOOT_NODE_IDLE_SETTLE_S", 4)),
                    help="주입 전 요구 idle_secs 안착치")
    ap.add_argument("--timeout", type=float, default=float(budget("BOOT_NODE_TOTAL_S", 90)),
                    help="전체 타임아웃(초) — 하위 서브프로세스에 데드라인 전파됨")
    ap.add_argument("--reclaim", action="store_true", help="막힌(죽은) 미각성 surface 결정론 회수")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    if getattr(a, "self_test", False):
        return self_test()
    if not a.role:
        print("error: --role 필수(또는 --self-test)")
        return 2

    log = []
    def emit(stage, msg):
        log.append({"stage": stage, "msg": msg})
        if not a.json:
            print("[boot-node:%s] %s" % (stage, msg))

    def done(result, reason, surface=None, code=0):
        if a.json:
            print(json.dumps({"role": a.role, "result": result, "reason": reason,
                              "surface": surface, "log": log}, ensure_ascii=False))
        return code

    status = cys_status()
    if status is None:
        emit("fatal", "cys status 수집 실패 — 데몬 미가동? `cys ping` 확인")
        return done("fatal", "daemon_down", code=2)

    if a.reclaim:
        return reclaim(a.role, emit)

    if not a.agent:
        print("error: 기동에는 --agent 필수")
        return 2

    t0 = time.time()
    def remaining():
        return a.timeout - (time.time() - t0)

    # ★데드라인 전파(B9 CS-5④ · 내부 감액 0): leaf 상한과 남은 데드라인의 **작은 쪽**을 쓴다.
    #   종전에는 LAUNCH 서브프로세스가 80s 를 데드라인과 무관하게 태워 내부 최악치가 외부 상한
    #   (_boot_one_node 130s)을 넘었다(B9 역전 2/3). leaf 는 그대로 두고 유계화만 한다.
    def deadline_capped(leaf_secs, floor=2.0):
        return max(floor, min(float(leaf_secs), max(floor, remaining())))

    def heartbeat(stage, msg):
        """침묵 창 상쇄(B9 방향 ③) — 진행 하트비트는 stderr 전용(verdict 채널 무오염)."""
        sys.stderr.write("[boot-node:%s] %s (남은 예산 %.0fs)\n" % (stage, msg, max(0.0, remaining())))
        sys.stderr.flush()

    # 1) PRE-CHECK — 이미 각성?(awake_ready=프로세스 제외)
    ready, why = awake_ready(status, a.role)
    if ready:
        row = role_surface_row(a.role)
        emit("precheck", "이미 각성 — %s (%s). 재기동 생략." % (row["surface_ref"] if row else "?", why))
        return done("already_up", why, row["surface_ref"] if row else None)

    # 2) LAUNCH — role 보유 surface 가 없을 때만(F2: 허위 실패보고 무시·재조회)
    row = role_surface_row(a.role)
    if row is None:
        cmd = ["cys", "launch-agent", "--role", a.role, "--agent", a.agent]
        if a.cwd:
            cmd += ["--cwd", a.cwd]
        launch_cap = deadline_capped(budget("BOOT_NODE_LAUNCH_SUBPROC_S", 80))
        heartbeat("launch", "launch-agent 기동(상한 %.0fs)" % launch_cap)
        rc, _, _ = run(cmd, timeout=launch_cap)
        emit("launch", "launch-agent rc=%d (실패 텍스트 무시·cys list 재조회)" % rc)
        for _ in range(3):
            time.sleep(2)
            row = role_surface_row(a.role)
            if row is not None:
                break
        if row is None:
            emit("fail", "launch 후에도 %s surface 생성 안 됨" % a.role)
            return done("no_surface", "launch_failed", code=1)
    else:
        emit("precheck", "%s 가 이미 role 보유(미각성) — 입양해 주입(재기동 안 함)" % row["surface_ref"])
    surface = row["surface_ref"]

    # 3) POLL-IDLE — 시작 애니메이션이 가라앉을 때까지(F1 핵심). 폴링 중 각성되면 즉시 종료.
    settled = False
    while remaining() > 12:
        st = cys_status()
        if st:
            r2, why2 = awake_ready(st, a.role)
            if r2:
                emit("poll", "폴링 중 각성 감지 — %s" % why2)
                return done("awake", why2, surface)
            srow = status_surface(st, a.role)
            idle = srow.get("idle_secs") if srow else None
            if isinstance(idle, (int, float)) and idle >= a.idle:
                settled = True
                emit("poll", "%s idle=%ss 안착 — 주입" % (surface, int(idle)))
                break
        time.sleep(2)
    if not settled:
        emit("poll", "idle 안착 대기 타임아웃 직전 — 일단 주입 시도")

    # 4) INJECT — 주입 직전 시각 기록(post-injection ack 기준)·queued 단일경로
    msg = awaken_message(a.role)
    marker = msg[:40]            # queue preview(80자 절단)와 대조할 식별 조각
    t_inject = time.time()
    ok, why3 = inject(a.role, msg)
    emit("inject", why3)
    if not ok:
        return done("inject_failed", why3, surface, code=1)

    # 5) VERIFY — t_inject 이후의 fresh set-status ack 또는 awakened_at 래치만 성공
    #    (프로세스·agent_alive 단독 불인정 — 부트 성공의 계약은 ack 다)
    def poll_ack(deadline_at):
        while time.time() < deadline_at and remaining() > 0:
            st = cys_status()
            if st and post_inject_ack(st, a.role, time.time() - t_inject, t_inject=t_inject):
                return True
            time.sleep(3)
        return False

    if poll_ack(t0 + a.timeout):
        emit("verify", "각성 확정 — 주입후 fresh set-status ack/awakened_at 래치")
        return done("awake", "post_inject_ack", surface)

    # ── B11: 타임아웃을 '유실'로 뭉개지 않는다 — 배달 3분기로 처방을 가른다 ──
    #   ★맹목 재전송 금지: 종전 처방("재전송")은 pending(아직 큐에 있음)에도 재전송해
    #     wakeup 홍수를 재생산했다. 재주입은 **delivered_no_ack 에서만·멱등 가드 1회**다.
    previews = queue_previews(surface)
    row = None
    for r in cys_list_rows():
        if r["surface_ref"] == surface:
            row = r
            break
    kind, why4 = classify_delivery(previews, row, marker)
    emit("verify", "ack 미확인 → 배달 분기=%s (%s)" % (kind, why4))
    if kind == DELIVERY_PENDING:
        # 큐가 살아 있으므로 배달은 아직 일어날 수 있다 — 실패로 보고하되 재전송은 하지 않는다.
        return done("injected_unverified", "queue_pending", surface, code=1)
    if kind == DELIVERY_DROPPED:
        return done("injected_unverified", "delivery_dropped", surface, code=1)
    # delivered_no_ack — 멱등 가드 1회 재주입 후 짧은 재확인, 그 뒤 증거 동봉 실패 승격.
    #   멱등 가드: 이 런에서 단 한 번(재귀·루프 없음). 큐가 비었음을 위에서 확인했으므로 중복 배달 0.
    heartbeat("reinject", "배달됨·ack 부재 — 멱등 1회 재주입")
    ok2, why5 = inject(a.role, msg, attempts=1)
    emit("reinject", "%s(멱등 1회 — 추가 재전송 없음)" % why5)
    if ok2:
        grace = float(budget("BOOT_NODE_INJECT_TIMEOUT_S", 12))
        if poll_ack(time.time() + grace):
            emit("verify", "재주입 후 각성 확정")
            return done("awake", "post_reinject_ack", surface)
    emit("verify", "타임아웃 — 배달됨·ack 미확인(증거: 큐 %d건·좌석 %s·read-screen 점검 권장)"
         % (len(previews), surface))
    return done("injected_unverified", "delivered_no_ack", surface, code=1)


if __name__ == "__main__":
    sys.exit(main())
