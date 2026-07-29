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
    """★각성 판정 — agent_alive OR fresh set-status 만(프로세스 제외).
    빈 CLI(디렉티브 미수신)를 각성으로 오인증하지 않는다(codex R1 결함1·5)."""
    s = status_surface(status, role)
    if s is None:
        return False, "surface 없음"
    if s.get("agent_alive"):
        return True, "agent_alive"
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
    awake_ready(각성) OR quiet_but_alive(각성이력+프로세스). 둘 다 아니면 '죽음/미각성'."""
    return awake_ready(status, role)[0] or quiet_but_alive(status, role)


def post_inject_ack(status, role, elapsed):
    """주입 *이후* 발신된 fresh set-status(=각성 ack)면 True.
    ★age < 주입후 경과시간(엄격·+마진 없음 — codex R2 결함3): 주입 *전* 보고는 age=age0+elapsed>elapsed
    이라 수학적으로 통과 불가. 오직 t_inject 뒤 발신만 통과한다."""
    s = status_surface(status, role)
    if s is None:
        return False
    st = s.get("status") or {}
    age = st.get("age_secs")
    return isinstance(age, (int, float)) and bool(st.get("state")) and age < elapsed


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
    rc, _, _ = _kill(pid)
    time.sleep(1.5)
    if role_surface_row(role) is not None and _pid_for_surface_ref(ref) == pid:
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

    if fails:
        print("self-test FAIL:")
        for f in fails:
            print("  ✗ " + f)
        return 1
    print("self-test OK — %d 케이스 통과(상태계약 분리·basename매칭·확장자정규화·회수판정4분기·claim-role·엄격ack·F4·무구독폴백슬롯)"
          % (7 + 10 + 6 + 4 + 4 + 4 + 4))
    return 0


# ─────────────────── 메인 ───────────────────
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--role")
    ap.add_argument("--agent")
    ap.add_argument("--cwd", default=None)
    ap.add_argument("--idle", type=float, default=4.0, help="주입 전 요구 idle_secs 안착치")
    ap.add_argument("--timeout", type=float, default=90.0, help="전체 타임아웃(초)")
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
        rc, _, _ = run(cmd, timeout=80)
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
    t_inject = time.time()
    ok, why3 = inject(a.role, awaken_message(a.role))
    emit("inject", why3)
    if not ok:
        return done("inject_failed", why3, surface, code=1)

    # 5) VERIFY — t_inject 이후의 fresh set-status ack 만 성공(프로세스·agent_alive 단독 불인정)
    while remaining() > 0:
        st = cys_status()
        if st and post_inject_ack(st, a.role, time.time() - t_inject):
            emit("verify", "각성 확정 — 주입후 fresh set-status ack")
            return done("awake", "post_inject_ack", surface)
        time.sleep(3)

    emit("verify", "타임아웃 — 주입은 됐으나 set-status ack 미확인(read-screen 점검 권장)")
    return done("injected_unverified", "no_ack", surface, code=1)


if __name__ == "__main__":
    sys.exit(main())
