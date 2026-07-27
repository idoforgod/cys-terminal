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
                단 **agent 가 죽은 좌석**(status.agent_alive is False)은 입양 금지 —
                맨 셸에 각성 산문을 보내면 command not found 로 끝나 복구율 0이다(F1).
                already_up 도 불인정한다(죽기 직전 set-status 잔향을 생존으로 쓰지 않는다).
  2) LAUNCH/RECOVER/ADOPT  3분기(precheck_action):
                launch  = role surface 부재 → launch-agent 1회(허위 실패 텍스트 무시·재조회 F2)
                recover = 죽은 agent 좌석 → `cys node-recover`(같은 surface CLI 재기동+디렉티브
                          재주입). 승계(takeover_empty_seat)로는 못 고친다 — 데몬 seat_claimable 이
                          agent_meta 보유 좌석을 배제해 **조용한 무승계**로 끝난다(격리 실측).
                          살아나지 않으면 주입하지 않고 중단한다(맨 셸 산문 금지).
                adopt   = role 보유 + agent 생존/미상 → 종전대로 입양해 주입(재기동 안 함).
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
    for s in (status or {}).get("surfaces", []):
        if s.get("role") == role and not s.get("exited"):
            return s
    return None


# ★F1(2026-07-27 적대리뷰 REVISE-1) — 죽은 agent pane 복구 경로 실배선.
#   종전 PRE-CHECK 은 `role_surface_row(role)`(cys list) 만 봤다. cys list 는 agent 생존을
#   말하지 않으므로, **agent 가 죽고 셸만 남은 pane**(라이브 실측: 부서 8노드 전원
#   agent_alive=false)이 role 을 점유 중이면 launch 를 건너뛰고 '입양'(각성 산문 주입)으로 갔다.
#   맨 셸에 한국어 산문이 떨어지면 `command not found` 로 끝나 복구율이 0이었다.
#   편성 ensure(javis_formation._roster_from_status)는 이미 agent_alive is False 를 '역할 부재'로
#   보고 _boot_node 를 부르는데, 정작 boot_node 가 여기서 no-op 이 되어 복구 사슬이 끊겼다.
#
# ★복구 경로 정정(격리 데몬 실측 2026-07-27): 이 결함의 처방은 `cys launch-agent`(빈 좌석 승계)가
#   **아니다**. 데몬의 승계 게이트 `governance::seat_claimable` 은 `agent_meta.is_none()` 을 요구한다
#   ("죽은 에이전트의 좌석은 node-recover 영역이지 탈취 대상이 아니다" — governance.rs 문언).
#   그런데 죽은 agent 좌석은 **정의상 agent_meta 를 보유**한다(그래서 agent_alive 가 None 이 아니라
#   False 다) → takeover 는 구조적으로 성립 불가다. 격리 데몬 프로브 실측: dead seat 에
#   `surface.create{role, takeover_empty_seat:true}` 를 걸면 **오류 없이** 새 surface 만 생기고 role 은
#   죽은 좌석에 그대로 남는다(조용한 무승계). 그 위에 각성 산문을 얹으면 결함이 그대로 재발한다.
#   데몬이 지정한 경로는 `cys node-recover` 다 — 같은 surface 에서 CLI 를 재기동하고 디렉티브를
#   재주입하며, 전제조건이 정확히 'agent 메타 보유 + agent_alive != true'(= 우리의 dead seat)다.
def seat_agent_dead(status, role):
    """role 을 점유한 비종료 surface 가 **빈 좌석**(agent 사망·셸만 생존)인가.

    True  = `agent_alive is False` — 입양(각성 산문 주입) 금지. `cys launch-agent` 가
            `takeover_empty_seat` 로 좌석을 승계해 CLI 를 실제로 다시 띄워야 한다.
    False = ①산 agent(True) ②필드 부재(구 데몬 — agent_alive 를 아예 노출하지 않음)
            ③None(등록 agent 없는 수동 셸·부팅 중) ④해당 role surface 없음.
            ②③은 종전 입양 동작을 그대로 유지한다(스큐 안전 — 판정 실패를 근거로
            멀쩡한 pane 을 재기동해 오살하지 않는다. javis_formation 의 None 처리와 동일 규약)."""
    s = status_surface(status, role)
    if s is None:
        return False
    return s.get("agent_alive") is False


def precheck_already_up(status, role):
    """PRE-CHECK 의 already_up 판정 — awake_ready 이되 **빈 좌석이면 불인정**.

    ★왜 필요한가: awake_ready 의 2번째 근거는 'fresh set-status(age<=600s)'다. agent 가 방금
    죽은 좌석은 죽기 직전에 남긴 set-status 잔향이 아직 신선해서 already_up 으로 통과한다 →
    launch 분기(F1)에 닿지도 못하고 no-op 으로 끝난다. `agent_alive is False` 는 '그 agent 는
    죽었다'는 데몬의 결정론 사실이므로, 그 잔향을 생존 근거로 쓰지 않는다.
    (agent_alive is True 면 awake_ready 가 그 자체로 참이라 이 게이트는 무해하다.)"""
    if seat_agent_dead(status, role):
        return False, "agent_alive=false(빈 좌석) — set-status 잔향을 생존으로 불인정"
    return awake_ready(status, role)


def precheck_action(row_present, dead_seat):
    """PRE-CHECK 3분기(순수) — 무엇을 할 것인가.

      "launch"  role 보유 surface 자체가 없다 → `cys launch-agent`(신규 기동).
      "recover" role 은 있는데 그 좌석의 agent 가 죽었다 → `cys node-recover`(같은 surface 재기동
                +디렉티브 재주입). **입양 금지** — 맨 셸에 각성 산문을 보내면 command not found 다.
      "adopt"   role 보유 + agent 생존/미상 → 종전대로 입양해 각성 지침을 주입.

    ★재기동 중복 방지 보존: agent 가 살아있으면(dead_seat=False) 절대 재스폰·재기동하지 않는다."""
    if not row_present:
        return "launch"
    return "recover" if dead_seat else "adopt"


def _pid_for_surface_ref(surface_ref):
    """주어진 surface_ref 의 비종료 row 의 pid(없으면 None). status 를 그 surface 생애에 결박."""
    for r in cys_list_rows():
        if r["surface_ref"] == surface_ref and r["exited"] is False:
            return r["pid"]
    return None


# ─────────────────── 상태 계약 3분리 ───────────────────
def _comm_matches(comm, agent):
    """comm(ps -o comm= 결과·macOS 는 전체경로)의 basename 이 agent 고유 바이너리와 동등하면 True.
    ★미지/빈 agent 는 후보 없음→False(wildcard 차단·R1 결함2)·basename 동등(substring 오매칭 차단·R2 논쟁점)."""
    names = AGENT_COMM.get(agent or "", ())
    if not names:
        return False
    base = os.path.basename((comm or "").strip()).lower()
    return base in names


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


def important_unsupported(err):
    """`--important` 가 **플래그 자체 때문에** 거부됐는가(구 CLI/구 데몬·권한 미충족).
    참이면 플래그 없이 1회 재시도한다(각성 주입은 TTL 면제가 '있으면 좋은' 것이지 전제가 아니다).
    거부 사유가 --important 와 무관하면 False → 기존 재시도 루프가 그대로 처리."""
    e = (err or "").lower()
    if "important" not in e:
        return False
    return any(tok in e for tok in (
        "unexpected argument", "unrecognized", "unknown", "invalid",
        "found argument", "requires an authoritative sender", "authoritative",
    ))


# ★B2(리뷰 수렴): 데몬의 소프트캡 거부는 **실패가 아니라 종결**이다 — 거부와 동시에 데몬이
#   전문을 dead-letter 원장에 기록했으므로 재전송할 이유가 없다(SOFTCAP_DIRECTIVE 문언).
#   종전 inject 는 이를 일반 실패로 보고 4연타(attempts=4)로 재전송했고, 그 4건이 전부
#   거부→원장 기록되어 **혼잡을 키우면서 원장까지 오염**시켰다. 판정 함수는
#   javis_wakeup.is_softcap_rejection 과 동일 술어를 여기에 자체 정의한다(형제 import 를
#   부트 경로에 끌어들이지 않는다 — 부트는 의존을 최소로 유지한다).
SOFTCAP_ERR = "queue_softcap_exceeded"


def is_softcap_rejection(*streams):
    """cys send 출력(stdout/stderr)에 소프트캡 거부 코드가 있으면 True."""
    return any(SOFTCAP_ERR in (s or "") for s in streams)


def inject(role, msg, attempts=4):
    """★`cys send --queued` 단일경로(codex R2 결함4): 큐는 대상이 조용해질 때 메시지+자동 Return 을
    원자적으로 배달한다 → typing_guard 우회(F1)·Return 분리 실패로 인한 중복 입력 위험 제거.
    (idle-then-inject 로 이미 안착했으므로 큐는 즉시 배달된다.)

    ★C7/W6(DESIGN §2.3): 각성 디렉티브는 **TTL 로 사라지면 안 되는 지휘 메시지**다 →
    `--important`(TTL 면제)로 승격한다. 채널은 그대로 `--queued` 단일경로다(플래그만 승격).
    발신은 master pane 이라 authoritative 권한을 충족한다. 구 데몬/구 CLI 는 이 플래그를
    모르므로, 플래그 때문에 거부되면 플래그 없이 1회 폴백한다(하위호환)."""
    errq = ""
    outq = ""
    use_important = True
    for i in range(attempts):
        cmd = ["cys", "send", "--queued"] + (["--important"] if use_important else []) + \
              ["--to", role, msg]
        rcq, outq, errq = run(cmd, timeout=12)
        if rcq == 0:
            return True, "주입 큐 등록(자동 Return 배달·시도 %d%s)" % (
                i + 1, "" if use_important else "·--important 미지원 폴백")
        # ★B2 터미널 처리: 소프트캡 거부는 종결이다 — 재시도 루프를 **즉시 중단**한다.
        #   (--important 는 TTL 면제일 뿐 소프트캡 면제가 아니다 — 의도된 설계라 폴백도 무의미.)
        if is_softcap_rejection(errq, outq):
            why = "dead-lettered(softcap)"
            print("warn: %s 각성 주입이 소프트캡 거부됨 — 데몬이 전문을 dead-letter 원장에 "
                  "기록 완료. 재전송 금지(시도 %d 에서 중단). 대상 큐 적체를 먼저 해소하라."
                  % (role, i + 1), file=sys.stderr)
            return False, why
        if use_important and important_unsupported(errq):
            # 구 데몬/권한 미충족 — 플래그를 내리고 같은 시도 안에서 1회 재전송
            use_important = False
            rcq, outq, errq = run(["cys", "send", "--queued", "--to", role, msg], timeout=12)
            if rcq == 0:
                return True, "주입 큐 등록(자동 Return 배달·시도 %d·--important 미지원 폴백)" % (i + 1)
            if is_softcap_rejection(errq, outq):
                print("warn: %s 각성 주입이 소프트캡 거부됨(폴백 경로) — 재전송 금지."
                      % role, file=sys.stderr)
                return False, "dead-lettered(softcap)"
        time.sleep(2)
    return False, "주입 실패(%d회·큐 등록 실패: %s)" % (attempts, (errq or "").strip()[:80])


# ─────────────────── 죽은 agent 좌석 복구(F1) ───────────────────
def node_recover(role, emit, budget=60.0):
    """죽은 agent 좌석을 `cys node-recover`로 되살린다. 성공 = agent_alive 가 False 를 벗어남.

    node-recover 는 셸을 죽이지 않는다 — 같은 surface 에서 C-u 로 입력을 비우고 CLI 를 재기동한
    뒤 디렉티브를 재주입한다(cys.rs run_node_recover). 그래서 pane·스크롤백이 보존되고,
    `--reclaim`(kill) 같은 비가역 조치가 필요 없다.
    실패하면 **False 를 돌려 주입을 막는다** — 살아나지 않은 셸에 산문을 보내지 않는 것이 F1 의 핵심.

    ★budget 은 **전체 상한**이다(하위 subprocess timeout 포함). 호출측의 --timeout 예산 안에서
    돌아야 POLL/INJECT/VERIFY 가 굶지 않고, 상위 formation(_boot_node timeout=200)도 안 터진다."""
    t0 = time.time()
    rc, out, err = run(["cys", "node-recover", "--role", role], timeout=max(20.0, budget * 0.7))
    emit("recover", "node-recover rc=%d %s" % (rc, (err or out or "").strip().replace("\n", " ")[:160]))
    if rc != 0:
        return False
    while time.time() - t0 < budget:
        st = cys_status()
        if st is not None and not seat_agent_dead(st, role):
            emit("recover", "%s agent 재기동 확인(agent_alive 가 false 를 벗어남)" % role)
            return True
        time.sleep(2)
    emit("recover", "node-recover 후에도 agent_alive=false 지속 — 맨 셸 주입 금지(중단). "
                    "`javis_boot_node.py --reclaim --role %s` 로 좌석을 회수한 뒤 재기동하라" % role)
    return False


# ─────────────────── 회수(F5) ───────────────────
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

    # ★F1: 죽은 agent 좌석 판정 + PRE-CHECK 분기(입양 vs launch-agent 승계)
    dead = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": False,
                          "status": {"age_secs": 5, "state": "working"}}]}
    alive = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": True}]}
    shell = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": None}]}
    legacy = {"surfaces": [{"role": "cso", "exited": False}]}          # 구 데몬(필드 부재)
    exited_row = {"surfaces": [{"role": "cso", "exited": True, "agent_alive": False}]}
    chk(seat_agent_dead(dead, "cso") is True, "agent 사망 좌석 미탐지(입양으로 새어나가 복구 0)")
    chk(seat_agent_dead(alive, "cso") is False, "산 agent 를 빈 좌석으로 오판(재스폰 위험)")
    chk(seat_agent_dead(shell, "cso") is False, "agent_alive=None(빈 셸)을 사망으로 오판")
    chk(seat_agent_dead(legacy, "cso") is False, "agent_alive 필드 부재(구 데몬)를 사망으로 오판(스큐 안전 위반)")
    chk(seat_agent_dead(exited_row, "cso") is False, "종료 surface 를 현행 좌석으로 오인")
    chk(seat_agent_dead({}, "cso") is False, "빈 status 에 오발동")
    chk(seat_agent_dead(None, "cso") is False, "None status 에 오발동")
    chk(precheck_action(False, False) == "launch", "surface 부재인데 launch 안 함")
    chk(precheck_action(False, True) == "launch", "surface 부재+dead 신호에 launch 안 함")
    chk(precheck_action(True, True) == "recover", "죽은 agent 좌석인데 입양으로 감(F1 회귀)")
    chk(precheck_action(True, False) == "adopt", "산 agent 보유 role 을 재기동(중복 기동 회귀)")
    # ★승계(takeover)로 가면 안 된다: 데몬 seat_claimable 이 agent_meta 보유 좌석을 배제하므로
    #   launch-agent 는 dead seat 에서 조용히 무승계로 끝난다(격리 데몬 실측).
    chk(precheck_action(True, True) != "launch",
        "dead seat 을 launch-agent 승계로 처리(데몬이 배제하는 경로 — 조용한 무승계)")
    # already_up 게이트가 빈 좌석을 삼키면 launch 분기에 닿지도 못한다(F1 우회 경로 차단).
    chk(precheck_already_up(dead, "cso")[0] is False,
        "죽은 좌석의 set-status 잔향을 already_up 으로 인정(F1 이 no-op 으로 무력화)")
    chk(precheck_already_up(alive, "cso")[0] is True, "산 agent 를 already_up 미인정(불필요 재기동)")
    chk(precheck_already_up(fresh, "cso")[0] is True,
        "agent_alive 미상 + fresh set-status 를 already_up 미인정(종전 계약 파손)")
    chk(precheck_already_up(stale, "cso")[0] is False, "stale set-status 를 already_up 오인정")
    chk(precheck_already_up(legacy, "cso")[0] is False, "구 데몬 무신호를 already_up 오인정")

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

    # ★W6: --important 폴백 판정(구 CLI/구 데몬·권한) — 무관한 실패에는 폴백하지 않는다
    chk(important_unsupported("error: unexpected argument '--important' found") is True,
        "구 CLI unexpected argument 미탐지")
    chk(important_unsupported("unrecognized option --important") is True,
        "구 CLI unrecognized 미탐지")
    chk(important_unsupported("important requires an authoritative sender (master/cso lineage)") is True,
        "권한 미충족 폴백 미탐지")
    chk(important_unsupported("send failed: surface not found") is False,
        "무관 실패에 --important 폴백 오발동")
    chk(important_unsupported("") is False, "빈 오류에 폴백 오발동")

    # ★B2: 소프트캡 거부 = 종결 판정(재시도 중단). javis_wakeup 과 동일 술어여야 한다.
    chk(is_softcap_rejection("error: queue_softcap_exceeded — 재전송하지 마라") is True,
        "소프트캡 거부 미탐지(stderr)")
    chk(is_softcap_rejection("", "queue_softcap_exceeded (dead-letter 기록)") is True,
        "소프트캡 거부 미탐지(다중 스트림)")
    chk(is_softcap_rejection("send failed: surface not found") is False,
        "무관 실패를 소프트캡으로 오판(각성 주입이 조기 포기된다)")
    chk(is_softcap_rejection(None, None) is False, "빈 스트림에 오발동")

    # 4연타 회귀 차단: 소프트캡이 오면 send 는 **1회만** 나가고 상태는 dead-lettered(softcap).
    calls = []

    def _fake_run(args, timeout=15):
        calls.append(list(args))
        return 1, "", "error: %s — 재전송하지 마라" % SOFTCAP_ERR

    _orig_run = globals()["run"]
    globals()["run"] = _fake_run
    try:
        ok, why = inject("worker", "각성", attempts=4)
    finally:
        globals()["run"] = _orig_run
    chk(ok is False, "소프트캡 거부를 성공으로 보고")
    chk(why == "dead-lettered(softcap)", "소프트캡 종결 상태 문자열 불일치: %r" % why)
    chk(len(calls) == 1, "소프트캡인데 재전송했다(%d회) — 혼잡 증폭·원장 오염" % len(calls))

    if fails:
        print("self-test FAIL:")
        for f in fails:
            print("  ✗ " + f)
        return 1
    print("self-test OK — %d 케이스 통과(상태계약 분리·basename매칭·claim-role·엄격ack·F4·"
          "무구독폴백슬롯·important폴백·소프트캡종결·죽은agent좌석복구분기)"
          % (7 + 4 + 17 + 4 + 4 + 4 + 5 + 7))
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

    # 1) PRE-CHECK — 이미 각성?(awake_ready=프로세스 제외 · 빈 좌석이면 잔향 불인정 — F1)
    ready, why = precheck_already_up(status, a.role)
    if ready:
        row = role_surface_row(a.role)
        emit("precheck", "이미 각성 — %s (%s). 재기동 생략." % (row["surface_ref"] if row else "?", why))
        return done("already_up", why, row["surface_ref"] if row else None)

    # 2) LAUNCH / RECOVER / ADOPT — 3분기(F2 허위 실패보고 무시·재조회 · F1 죽은 좌석 복구)
    row = role_surface_row(a.role)
    dead_seat = seat_agent_dead(status, a.role)
    action = precheck_action(row is not None, dead_seat)

    if action == "launch":
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

    elif action == "recover":
        # ★F1: agent 만 죽고 셸은 살아있는 좌석. 입양(각성 산문 주입) 금지 — 맨 셸은 그 산문을
        #   command not found 로 흘린다(복구율 0). 데몬이 지정한 경로는 node-recover 다
        #   (승계 게이트는 agent_meta 보유 좌석을 구조적으로 배제한다 — 상단 주석 참조).
        emit("precheck", "%s 가 role 을 점유했으나 agent 사망(agent_alive=false) — 입양 금지, "
                         "node-recover 로 같은 surface 에서 CLI 재기동" % row["surface_ref"])
        # POLL-IDLE·INJECT·VERIFY 에 최소 15초는 남겨둔다(전체 --timeout 예산 준수).
        if not node_recover(a.role, emit, budget=max(20.0, remaining() - 15)):
            return done("recover_failed", "dead_agent_not_revived", row["surface_ref"], code=1)
        row = role_surface_row(a.role)
        if row is None:
            emit("fail", "node-recover 후 %s role surface 소실" % a.role)
            return done("no_surface", "recover_lost_surface", code=1)

    else:
        emit("precheck", "%s 가 이미 role 보유(미각성·agent 생존/미상) — 입양해 주입(재기동 안 함)"
             % row["surface_ref"])
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
