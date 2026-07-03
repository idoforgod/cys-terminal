#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
javis_phoenix.py — 불사조(무손실 복원) Phase 2: 부활 저널 상태머신 MVP (M1 단일 게이트)

설계 근거: _round/ZERO_LOSS_RESTORE_DESIGN.md §9.4-2 · §11(M1/M5/M9) · §10(운영 수칙)
원칙(P2 재사용 제1원칙): 신규 부활 엔진을 만들지 않는다 — 기존 프리미티브
  (cys restore · node-recover · watch · reinject · attest · javis_state_snapshot)를
  '단계별 저널 상태머신'으로 접착만 한다. 이 파일이 더하는 것은 저널·재개·정직한
  라벨(M9)·회로차단기(M5)·조정 패스(B1)라는 '얇은 접착층'뿐이다.

핵심 안전 성질(설계 §11):
  · M9 정직한 상태 enum: 부활 결과는 VERIFIED / UNVERIFIED / FAILED 로만 분리 출력한다.
    - resume된 세션의 실제 session_id 를 topology 기록과 대조해 일치할 때만 VERIFIED.
    - 미검증 복원은 "성공(success)" 문자열을 절대 출력하지 않는다. "무출력=성공" 해석 금지.
    - (§10.2 조용한 오복원: 엉뚱한 세션에 붙고 성공 로그를 남기는 것이 명백한 실패보다 위험.)
  · M5 크래시 루프 회로차단기: T분 내 N회 부활 시도 → 차단기 OPEN → 직전 세대 롤백 제안
    → 정지 + 알림. (차단기 자신의 사망=폭주 방향이므로 meta-drill로 시험한다.)
  · P4 단계별 저널: 생성→기동(ready)→resume→디렉티브 주입(reinject)→G2 ack→검증(verify)의
    각 단계를 저널에 기록하고, 중단 시 완료 단계는 skip하고 미완 단계부터 재개한다.
    dedup 키 = (role, ticket_id) — role당 복수 워커 정책과 충돌하지 않게 티켓 ID를 포함한다.

라이브 무접촉(이 티켓 범위): 저널·산출은 소켓의 상태 디렉터리 하위 'phoenix/'에 쓴다.
  소켓이 라이브(~/.local/state/cys)면 저널 쓰기를 거부한다(PHOENIX_ALLOW_LIVE=1 이 없는 한).
  → 개발·검증은 전부 격리 하네스 소켓에서만 이뤄진다.

spawn(생성) 백엔드 2종:
  · production(기본): `cys restore` — 실제 죽은 역할을 topology에서 일괄 재기동(실 프리미티브 재사용).
  · surrogate(--stub / 하네스): `cys new-surface` + 경량 stub 에이전트. 실 claude/agy/codex를
    스폰하지 않고(토큰0·자원 안전) 저널·M9·M5·B1 기계를 격리에서 증명한다. surrogate의
    session_id 대조로 M9의 VERIFIED/UNVERIFIED 두 라벨을 정직하게 재현한다.

서브커맨드:
  status     — 저널·신뢰 상태(GREEN/AMBER/RED 개념)·회로차단기 상태를 정직하게 출력(무변경)
  restore    — 부활 저널 상태머신 실행(재개 가능). --stub 이면 surrogate 백엔드.
  reconcile  — B1 조정 패스: topology 위임 대장 vs 실측(surface·WORKER_TODO) 대조·불일치 보고
  drill      — 하네스에서 완료 기준 drill(중단→재개·M9·M5) 자체 실행용 헬퍼(무손실 하네스가 호출)
  gen-manual — 세대 스냅샷에 '독립 수동 복원 스크립트'(데몬/hook 비의존 평문) 동봉(⑥·M1 출하조건)
  gen-protect— M4 역할기반 쓰기보호 스크립트 생성(기본 DRY-RUN — 라이브 파일에 적용하지 않음)
  deploy     — Phase 3로 연기(quiescent→스냅샷→적용→drill 내장). 지금은 안내만 출력.
"""

import argparse
import json
import os
import re
import signal
import subprocess
import sys
import time

HOME = os.path.expanduser("~")
LIVE_STATE = os.path.realpath(os.path.join(HOME, ".local", "state", "cys"))

# 부활 저널의 단계(P4) — 순서 고정
STAGES = ["spawn", "ready", "resume", "reinject", "g2_ack", "verify"]

# M5 회로차단기 기본 파라미터(환경변수로 격리 drill에서 조정 가능)
BREAKER_N = int(os.environ.get("PHOENIX_BREAKER_N", "3"))       # T분 내 N회
BREAKER_T = int(os.environ.get("PHOENIX_BREAKER_T", "300"))     # T초 창

# ★Phase 6: boot-epoch 세대 태그(DRILL_LIVE_2 수리). 데몬 기동마다 바뀌는 식별자(daemon.started_at)를
#   저널 완료 마킹에 붙이고, skip 판정은 '완료 마킹의 epoch == 현재 epoch'일 때만 유효로 본다.
#   재부팅을 넘긴(=이전 세대) 완료 마킹은 stale로 무효화 → 재spawn 대상(잘못-skip 방지).
#   PHOENIX_EPOCH_GATE=0 이면 게이트를 끈다(레거시 동작 — 하네스 A/B 재현 전용, 평시 사용 금지).
EPOCH_GATE = os.environ.get("PHOENIX_EPOCH_GATE", "1") != "0"
_ACTIVE_EPOCH = None  # cmd_restore 시작 시 cys status의 daemon.started_at로 취득

# ★Phase 10: 부활 완결성(retry-until-full) — DRILL_LIVE_3 부분실패(cso 3/4) 수리.
#   대량 동시 스폰 시 한 역할이 readiness 경합/타임아웃으로 미스폰돼도 재시도해 roster 전원 부활까지 COMPLETE.
#   부분 부활 = INCOMPLETE(잔여 역할 정직 명시·escalation). 스폰 후 settle·재시도 backoff 증가로 경합 완화(부활 폭풍 방지).
SPAWN_RETRIES = int(os.environ.get("PHOENIX_SPAWN_RETRIES", "3"))       # 미스폰 역할 재시도 횟수
SPAWN_SETTLE = float(os.environ.get("PHOENIX_SPAWN_SETTLE", "1.0"))     # 스폰 후 surface 등장 정착 대기
SPAWN_BACKOFF = float(os.environ.get("PHOENIX_SPAWN_BACKOFF", "1.5"))   # 재시도 간격 기준(회차마다 증가)

# ★Phase 11: 독약 세션(unresumable) fresh-spawn fallback — DRILL_LIVE_4 §15 수리.
#   완결성(Phase10)은 resume(세션핀) 기반 spawn 을 반복하는데, 세션이 독약(resume 불가·손상)이면 매 재시도가
#   동일하게 실패한다(DRILL_LIVE_4: claude --resume 워커만 부활 실패). 근본 = §3 원칙5 "N회 resume 실패→
#   무 resume(fresh) 기동 + 원장 재주입" 미구현. 수리: resume 재시도 소진 후에도 미부활이면, 해당 역할을
#   fresh(무 resume) 재기동으로 '강등'해 roster 100% 부활을 보장한다(독약 세션이 무한 재시도로 roster 를
#   막지 않게). fresh 전환은 저널·결과에 정직 명시(resumed→fresh — 세션 보존 실패를 숨기지 않는다).
#   resume 성공은 그대로 우선(fresh 는 최후수단). PHOENIX_POISON_FRESH_FALLBACK=0 이면 강등을 끈다(A/B 재현용).
POISON_FRESH_FALLBACK = os.environ.get("PHOENIX_POISON_FRESH_FALLBACK", "1") != "0"

CYS = None  # lazy resolve


# ------------------------------------------------------------------ 기반 유틸

def _which(name):
    import shutil
    return shutil.which(name)


def die(msg, code=2):
    sys.stderr.write("[phoenix][FATAL] %s\n" % msg)
    sys.exit(code)


def log(msg):
    sys.stdout.write("[phoenix] %s\n" % msg)
    sys.stdout.flush()


def state_dir_for(socket):
    """소켓 경로에서 데몬 상태 디렉터리(=소켓 부모)를 파생 — 하네스 격리 계약과 동일."""
    if socket:
        return os.path.realpath(os.path.dirname(socket))
    # 소켓 미지정 시 라이브 기본
    return LIVE_STATE


def phoenix_home(socket):
    """저널·산출 루트. 라이브 상태 디렉터리에는 쓰지 않는다(무접촉 가드)."""
    sd = state_dir_for(socket)
    if sd == LIVE_STATE and os.environ.get("PHOENIX_ALLOW_LIVE") != "1":
        die("라이브 상태 디렉터리(%s)에 저널 쓰기를 거부한다(이 티켓=라이브 무접촉). "
            "격리 하네스 소켓으로 --socket 을 주거나 PHOENIX_ALLOW_LIVE=1." % sd)
    home = os.path.join(sd, "phoenix")
    os.makedirs(home, exist_ok=True)
    return home


def cys(*args, socket=None, timeout=25):
    cmd = [CYS]
    if socket:
        cmd += ["--socket", socket]
    cmd += [str(a) for a in args]
    env = dict(os.environ)
    env.pop("AITERM_SOCKET", None)
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
        return r
    except subprocess.TimeoutExpired as e:
        class _R:
            returncode = 124
            stdout = (e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")) if e.stdout else ""
            stderr = "TIMEOUT %ss" % timeout
        return _R()


def get_boot_epoch(socket):
    """boot-epoch = 데몬 기동 세대 식별자. cys status --json 의 daemon.started_at 실측(재시작마다 변경).
    이 값이 저널 완료 마킹의 세대 유효성 기준이다(재부팅을 넘긴 마킹 = stale). 획득 실패 시 None
    → 호출측(stage_done)이 보수적으로 stale 취급(=재spawn, 잘못-skip 아님)."""
    r = cys("status", "--json", socket=socket, timeout=12)
    if getattr(r, "returncode", 1) != 0:
        return None
    try:
        st = json.loads(r.stdout or "{}")
    except Exception:
        return None
    sa = (st.get("daemon") or {}).get("started_at")
    if sa is None:
        return None
    return "sa:%s" % sa


def _atomic_write_json(path, obj):
    """tmp+fsync+rename+dir fsync — javis_state_snapshot 과 동일한 원자성 규약."""
    d = os.path.dirname(path)
    tmp = os.path.join(d, ".tmp-%d-%s" % (os.getpid(), os.path.basename(path)))
    with open(tmp, "w") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.rename(tmp, path)
    try:
        dfd = os.open(d, os.O_RDONLY)
        os.fsync(dfd)
        os.close(dfd)
    except Exception:
        pass


def read_topology(socket):
    """데몬 상태 디렉터리의 topology.json(위임 대장의 진실). 읽기 전용."""
    sd = state_dir_for(socket)
    p = os.path.join(sd, "topology.json")
    if not os.path.exists(p):
        return {"entries": [], "updated_at": 0, "_path": p, "_missing": True}
    try:
        t = json.load(open(p))
        t["_path"] = p
        return t
    except Exception as e:
        return {"entries": [], "updated_at": 0, "_path": p, "_error": str(e)}


# ---------------- desired-state 로스터 (Phase 4 · DRILL_LIVE_1 desired-state 침식 수리) ----------------
# 문제(§12): topology.json = persist_topology가 !exited(라이브)만 쓰는 actual-state라, 부분 부활 직후
#   미부활 역할이 선언(desired)에서 삭제된다 → phoenix가 "죽은 역할 0(NOOP)" 오판.
# 수리(§12 원칙2 tombstone): phoenix가 관측 시점에 desired 로스터를 조기·단조 영속(침식 전 전 역할 박제).
#   desired는 관측으로만 늘고, ★tombstone(의도적 폐역)으로만 준다 — transient 사망으로 줄지 않는다.
#   죽은 역할 판정 = desired − 현재 생존. topology 침식과 무관해진다.

def desired_roster_path(socket):
    return os.path.join(phoenix_home(socket), "desired_roster.json")


def load_desired_roster(socket):
    p = desired_roster_path(socket)
    if os.path.exists(p):
        try:
            d = json.load(open(p))
            return d.get("roster", {}), set(d.get("tombstones", []))
        except Exception:
            pass
    return {}, set()


def _snapshot_roster_entries(socket):
    """직전 세대 스냅샷(javis_state_snapshot)의 topology.json 로스터 — §12가 실증한 안전망.
    라이브 상태 디렉터리 대상일 때만 유효(격리 하네스는 자기 topology만 씀). best-effort."""
    if state_dir_for(socket) != LIVE_STATE:
        return {}
    snap = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_state_snapshot.py")
    gen_root = os.path.join(HOME, ".cys", "state-generations")
    try:
        gens = sorted(g for g in os.listdir(gen_root) if re.match(r"\d{8}T\d{6}Z", g))
    except Exception:
        return {}
    for g in reversed(gens):  # 최신 세대부터
        tp = os.path.join(gen_root, g, "topology.json")
        if os.path.exists(tp):
            try:
                t = json.load(open(tp))
                return {e["role"]: e for e in t.get("entries", []) if e.get("role")}
            except Exception:
                continue
    return {}


def observe_and_persist_roster(socket):
    """현재 관측(topology + 세대 스냅샷)을 desired 로스터에 단조 병합·영속하고 (roster, tombstones) 반환.
    ★침식 전에 호출되면 전 역할이 박제된다 — 이후 topology가 줄어도 desired는 보존된다."""
    roster, tombstones = load_desired_roster(socket)
    topo = read_topology(socket)
    # 우선순위: 기존 desired < 세대 스냅샷 < 현재 topology (최신 관측이 메타를 갱신)
    for role, e in _snapshot_roster_entries(socket).items():
        roster[role] = e
    for e in topo.get("entries", []):
        if e.get("role"):
            roster[e["role"]] = e
    # ★Phase 7: 라이브 role 직접 병합 — claim-role 즉시 자동 등재(topology 영속 지연/침식 무관).
    #   '태어날 때부터 보호': 역할이 살아있는 순간 보호집합에 편입된다. 이미 있으면 갱신 안 함(topology 엔트리 우선).
    for role, _surfs in live_role_surfaces(socket).items():
        if role and role != "-":
            roster.setdefault(role, {"role": role})
    # tombstone된 역할은 desired에서 제외(의도적 폐역)
    for t in tombstones:
        roster.pop(t, None)
    try:
        _atomic_write_json(desired_roster_path(socket),
                           {"roster": roster, "tombstones": sorted(tombstones), "updated_at": _now()})
    except Exception:
        pass
    return roster, tombstones


# ---------------- 부서 dept-roster (Phase 7 · 자동 보호 상속 — 부서판) ----------------
# 원리(§12 R3 선행조건): 부서(dept)도 노드 role 과 동일하게 '태어날 때부터 보호집합'에 자동 편입돼야 한다.
#   실측 갭: 실 depts.json 은 stale(dept-1 만 등록)인데 디스크엔 dept-1~5 존재 → registry 만 믿으면 누락.
#   수리: 부서를 glob(state_root/cys-dept-*) ∪ depts.json 으로 동적 발견(파일시스템 truth)해 phoenix 소유
#   dept_roster.json 에 단조 등재. ★실 depts.json 은 읽기 전용(무접촉) · 부서명 하드코딩 0(모든 사용자 동일).
#   격리: 하네스는 PHOENIX_DEPT_STATE_ROOT/PHOENIX_DEPTS_JSON env 로 합성 부서를 주입(라이브 무접촉).

def _dept_discovery_roots():
    state_root = os.environ.get("PHOENIX_DEPT_STATE_ROOT") or os.path.join(HOME, ".local", "state")
    depts_json = os.environ.get("PHOENIX_DEPTS_JSON") or os.path.join(HOME, ".cys", "depts.json")
    return state_root, depts_json


def discover_depts():
    """현재 존재하는 부서를 동적 발견 — glob(state_root/cys-dept-*) ∪ depts.json 레지스트리.
    registry stale 면역(파일시스템 truth). {deptname: {state_dir, socket?, pack_dir?}} 반환. 읽기 전용."""
    state_root, depts_json = _dept_discovery_roots()
    found = {}
    try:
        for name in os.listdir(state_root):
            if name.startswith("cys-dept-"):
                p = os.path.join(state_root, name)
                if os.path.isdir(p):
                    found[name[len("cys-dept-"):]] = {"state_dir": os.path.realpath(p)}
    except OSError:
        pass
    if os.path.isfile(depts_json):
        try:
            reg = json.load(open(depts_json))
            for dept, meta in (reg.get("depts") or {}).items():
                info = found.setdefault(dept, {})
                sock = (meta or {}).get("socket")
                if sock:
                    info["socket"] = sock
                    info.setdefault("state_dir", os.path.realpath(os.path.dirname(sock)))
                if (meta or {}).get("pack_dir"):
                    info["pack_dir"] = meta["pack_dir"]
        except Exception:
            pass
    return found


def dept_roster_path(socket):
    return os.path.join(phoenix_home(socket), "dept_roster.json")


def load_dept_roster(socket):
    p = dept_roster_path(socket)
    if os.path.exists(p):
        try:
            d = json.load(open(p))
            return d.get("roster", {}), set(d.get("tombstones", []))
        except Exception:
            pass
    return {}, set()


def observe_and_persist_depts(socket):
    """발견된 부서를 dept_roster 에 단조 병합·영속(침식 면역). (roster, tombstones) 반환.
    ★phoenix 소유 dept_roster.json 에만 쓴다 — 실 depts.json 무접촉. tombstone 된 부서는 제외(의도적 폐역)."""
    roster, tombstones = load_dept_roster(socket)
    for dept, info in discover_depts().items():
        cur = roster.get(dept, {})
        cur.update(info)
        roster[dept] = cur
    for t in tombstones:
        roster.pop(t, None)
    try:
        _atomic_write_json(dept_roster_path(socket),
                           {"roster": roster, "tombstones": sorted(tombstones), "updated_at": _now()})
    except Exception:
        pass
    return roster, tombstones


def live_role_surfaces(socket):
    """현재 살아있는 surface들의 role→(surface_ref, pid, exited) 실측."""
    r = cys("list", socket=socket, timeout=12)
    out = {}
    for line in (r.stdout or "").splitlines():
        m = re.match(r"(surface:\d+)\s+role=(\S+)\s+pid=(\d+)\s+exited=(\S+)", line)
        if m:
            ref, role, pid, exited = m.group(1), m.group(2), int(m.group(3)), m.group(4)
            out.setdefault(role, []).append({"surface": ref, "pid": pid, "exited": exited == "true"})
    return out


# ------------------------------------------------------------------ 저널

def journal_path(socket, ticket_id):
    return os.path.join(phoenix_home(socket), "journal-%s.json" % _slug(ticket_id))


def _slug(s):
    return re.sub(r"[^A-Za-z0-9_.-]", "_", str(s))[:64] or "default"


def load_journal(socket, ticket_id):
    p = journal_path(socket, ticket_id)
    if os.path.exists(p):
        try:
            return json.load(open(p))
        except Exception:
            pass
    return {"ticket_id": ticket_id, "roles": {}, "events": [], "created": _now()}


def save_journal(socket, ticket_id, j):
    _atomic_write_json(journal_path(socket, ticket_id), j)


def _now():
    return int(time.time())


def jevent(j, role, stage, status, msg=""):
    j["events"].append({"ts": _now(), "role": role, "stage": stage, "status": status, "msg": msg[:300]})


def stage_done(j, role, stage, epoch=None):
    """단계 완료 여부. ★Phase 6: EPOCH_GATE 가 켜져 있으면 '완료 마킹의 epoch == 현재 epoch'일 때만
    완료로 인정한다(재부팅을 넘긴 stale 마킹 무효화 — DRILL_LIVE_2 worker 잘못-skip 수리).
    현재 epoch 미상이거나 마킹에 epoch가 없거나 상이하면 보수적으로 미완료(=재spawn 대상)로 본다
    — fail 방향 = 재spawn(가용성)이지 잘못 skip 이 아니다(대상역할은 이미 죽은 역할로 선별됨)."""
    s = j["roles"].get(role, {}).get("stages", {}).get(stage, {})
    if not s.get("done"):
        return False
    if not EPOCH_GATE:
        return True  # 레거시(드릴 A/B 재현 전용) — 세대 무시
    cur = epoch if epoch is not None else _ACTIVE_EPOCH
    if cur is None:
        return False  # 현재 세대 미상 → 안전하게 stale 취급(재spawn)
    return s.get("epoch") == cur  # 마킹 epoch 부재(None)/상이 → stale → False


def mark_stage(j, role, stage, done, evidence="", epoch=None):
    rr = j["roles"].setdefault(role, {"stages": {}})
    ent = {"done": done, "ts": _now(), "evidence": str(evidence)[:400]}
    ep = epoch if epoch is not None else _ACTIVE_EPOCH  # ★Phase6: 완료 당시 세대 태그 첨부
    if ep is not None:
        ent["epoch"] = ep
    rr["stages"][stage] = ent


# ------------------------------------------------------------------ M5 회로차단기

def breaker_file(socket):
    return os.path.join(phoenix_home(socket), "breaker.json")


def breaker_check_and_record(socket):
    """이번 restore 시도를 기록하고, T초 내 N회 이상이면 (open=True, 최근 시도 리스트) 반환."""
    p = breaker_file(socket)
    now = _now()
    attempts = []
    if os.path.exists(p):
        try:
            attempts = json.load(open(p)).get("attempts", [])
        except Exception:
            attempts = []
    attempts = [t for t in attempts if now - t <= BREAKER_T]  # 창 밖 제거
    attempts.append(now)
    _atomic_write_json(p, {"attempts": attempts, "N": BREAKER_N, "T": BREAKER_T})
    return (len(attempts) >= BREAKER_N, attempts)


def breaker_reset(socket):
    p = breaker_file(socket)
    if os.path.exists(p):
        _atomic_write_json(p, {"attempts": [], "N": BREAKER_N, "T": BREAKER_T})


def rollback_proposal(socket):
    """직전 GREEN 세대로의 롤백 제안(제안만 — 실행 금지). javis_state_snapshot list 재사용."""
    snap = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_state_snapshot.py")
    prop = {"snapshot_tool": snap, "generations": [], "note": "실행하지 않는다 — 사람 승인 후 --at 롤백"}
    if os.path.exists(snap):
        r = subprocess.run([sys.executable, snap, "list"], capture_output=True, text=True, timeout=15)
        prop["generations_raw"] = (r.stdout or r.stderr or "").strip()[:600]
        gens = re.findall(r"(\d{8}T\d{6}Z)", r.stdout or "")
        prop["generations"] = gens
        if gens:
            prop["suggested_rollback_to"] = gens[-1]  # 목록상 직전 세대(도구 정렬 규약 따름)
    return prop


# ------------------------------------------------------------------ spawn 백엔드

def spawn_production(socket, pending_roles, include_master=False):
    """실 프리미티브 재사용: cys restore 로 죽은 역할 일괄 재기동(세션핀 resume 경로)."""
    args = ["restore"]
    if include_master:
        args.append("--include-master")
    r = cys(*args, socket=socket, timeout=90)
    return {"backend": "production(cys restore)", "rc": r.returncode,
            "out": (r.stdout or r.stderr or "").strip()[:800]}


def spawn_fresh_production(socket, role, agent):
    """★Phase11 독약세션 fresh-fallback(prod): 무 resume 로 새 세션 기동(cys launch-agent).
    cys restore 는 topology 의 session_id 를 resume 하므로 독약 세션이면 계속 실패한다 → 세션핀을 버리고
    launch-agent 로 fresh 기동한다. launch-agent 는 역할 디렉티브를 자동 주입한다(각성). 세션 보존은 포기하지만
    (원 세션이 독약이므로 불가피) 노드는 부활한다. 원장(SESSION_STATE/TODO) 재주입은 후행 reinject 단계가 담당."""
    r = cys("launch-agent", "--role", role, "--agent", agent or "claude", socket=socket, timeout=60)
    return {"rc": r.returncode, "out": (r.stdout or r.stderr or "").strip()[:400]}


def spawn_surrogate(socket, role, observed_sid, attempt=0, mode="resume"):
    """하네스 전용: 실 에이전트 없이 경량 stub surface 하나를 띄운다.
    stub은 ready 마커 + SESSION=<observed_sid> 를 출력하고 생존한다(watch·read-screen·M9 검증용).
    ★Phase10 fault 주입: PHOENIX_SPAWN_FAIL_ONCE=<role,...> 에 든 역할은 attempt0에서 스폰 실패를
    시뮬레이션한다(대량 스폰 경합 재현 — 완결성 재시도가 이를 회복하는지 실증하는 테스트 훅).
    ★Phase11 mode: 'resume'(세션핀 재개) vs 'fresh'(무 resume 재기동). PHOENIX_POISON_SESSION=<role,...> 에
    든 역할은 resume 모드에서 항상 실패(독약 세션 = 재개 불가 모델)하고, fresh 모드에서는 성공한다(무 resume
    launch-agent 로 즉시 복구되는 §15 실측 재현). fresh 는 원 세션핀이 아니라 새 세션으로 뜬다(정직: 세션 보존 아님)."""
    _fail_once = [x.strip() for x in os.environ.get("PHOENIX_SPAWN_FAIL_ONCE", "").split(",") if x.strip()]
    if role in _fail_once and attempt == 0:
        return None, "★주입된 스폰 실패(attempt0·완결성 재시도 테스트 — DRILL_LIVE_3 경합 재현)"
    _fail_always = [x.strip() for x in os.environ.get("PHOENIX_SPAWN_FAIL_ALWAYS", "").split(",") if x.strip()]
    if role in _fail_always:
        return None, "★주입된 영구 스폰 실패(재시도 소진→INCOMPLETE escalation 테스트)"
    # ★Phase11: 독약 세션 — resume 모드에서만 실패(재개 불가), fresh 모드는 새 세션으로 성공.
    _poison = [x.strip() for x in os.environ.get("PHOENIX_POISON_SESSION", "").split(",") if x.strip()]
    if role in _poison and mode == "resume":
        return None, "★독약 세션(resume 불가·attempt %d) — fresh 강등 필요(DRILL_LIVE_4 §15)" % attempt
    r = cys("new-surface", socket=socket, timeout=15)
    m = re.search(r"(surface:\d+)", r.stdout or "")
    ref = m.group(1) if m else None
    if not ref:
        return None, (r.stderr or r.stdout or "new-surface 실패")
    # stub 명령 주입: (watch가 이길 시간을 주려 1.2s 지연) → ready 마커 + 세션 표식 → 생존.
    # ※ printf %s / Python %s 충돌 회피 위해 문자열 결합으로 값 삽입(포맷 지정자 미사용).
    cmdline = ("sleep 1.2; echo PHOENIX_STUB_READY role=" + role +
               " SESSION=" + observed_sid + " SPAWNMODE=" + mode + " ENDMARK; exec sleep 3600")
    cys("send", "--surface", ref, cmdline, socket=socket, timeout=10)
    cys("send-key", "--surface", ref, "Return", socket=socket, timeout=10)
    return ref, "surrogate stub on %s (SESSION=%s mode=%s)" % (ref, observed_sid, mode)


# ------------------------------------------------------------------ 단계 실행기

def stage_ready(socket, role, surface, stub):
    """기동 완료(ready) 판정 — 실 응답 신호(ready_marker) 확인. ★Phase10: 대량 부활에서 스폰이 스태거되면
    watch(신규 출력)가 이미 emit된 marker를 놓쳐 ready 타임아웃 → 부분부활. 먼저 현재 화면(read-screen)에
    marker 존재를 확인해 '지금 응답 가능한가'를 판정하고, 없을 때만 watch(신규 출력)로 대기한다."""
    marker = "PHOENIX_STUB_READY" if stub else "bypass permissions on"
    r0 = cys("read-screen", "--surface", surface, socket=socket, timeout=10)
    if marker in (r0.stdout or ""):
        return True, "ready marker present on screen (read-screen)"
    r = cys("watch", "--surface", surface, "--until", marker, "--timeout", "12",
            socket=socket, timeout=16)
    return r.returncode == 0, "watch rc=%s until=%r" % (r.returncode, marker)


# ★Phase 5 ③: 세션 핀 grace 윈도우(골격). grace 값은 placeholder — 다음 라이브 drill에서
# master가 실 에이전트 재핀 타이밍을 실측해 캘리브레이션한다(usage 수집기가 transcript 발견 후
# agent_session_id를 topology에 재기록하기까지의 지연). 정직성 불변: grace 소진 후에도 미관측이면 unverified.
PHOENIX_SESSION_GRACE_TRIES = int(os.environ.get("PHOENIX_SESSION_GRACE_TRIES", "3"))  # placeholder
PHOENIX_SESSION_GRACE_SLEEP = float(os.environ.get("PHOENIX_SESSION_GRACE_SLEEP", "1.5"))  # placeholder


def _topology_session_for(socket, role):
    """prod 재핀 경로: topology.json에 usage 수집기가 재기록한 role의 session_id를 읽는다."""
    for e in read_topology(socket).get("entries", []):
        if e.get("role") == role:
            return e.get("session_id")
    return None


def stage_observe_session(socket, surface, stub, role=None):
    """resume된 세션의 실제 session_id 관측(grace 폴링). stub=스크린 SESSION= / prod=topology 재핀.
    grace 내 미관측(재핀 전)은 None으로 반환해 verify가 transient로 다룬다(정직: 불확실=unverified)."""
    last_txt = ""
    tries = 1 if stub else max(1, PHOENIX_SESSION_GRACE_TRIES)
    for attempt in range(tries):
        r = cys("read-screen", "--surface", surface, socket=socket, timeout=12)
        txt = r.stdout or ""
        last_txt = txt
        # stub: 렌더된 'PHOENIX_STUB_READY role=.. SESSION=<sid> ENDMARK' 라인에서만 추출(에코 꼬리 배제)
        ms = re.findall(r"PHOENIX_STUB_READY\s+role=\S+\s+SESSION=([A-Za-z0-9._-]+)\s+ENDMARK", txt)
        if ms:
            return ms[-1], txt.strip()[-200:]
        # prod: topology 재핀(usage 수집기) 우선 — grace 동안 재핀을 기다린다
        if not stub and role:
            sid = _topology_session_for(socket, role)
            if sid:
                return sid, "topology re-pin(grace attempt %d)" % (attempt + 1)
        # 스크린 폴백(prod 재핀 전 임시 신호)
        m = re.search(r"SESSION=([A-Za-z0-9._-]+)", txt)
        if m:
            return m.group(1), txt.strip()[-200:]
        if attempt < tries - 1:
            time.sleep(PHOENIX_SESSION_GRACE_SLEEP)
    return None, last_txt.strip()[-200:]  # grace 소진·미관측 → transient(verify가 unverified 처리)


def stage_reinject(socket, role, surface, stub):
    """디렉티브 재주입 — reinject --check 재사용(각성 핑 후 필요 시 주입)."""
    r = cys("reinject", "--check", "--role", role, "--surface", surface, "--timeout", "6",
            socket=socket, timeout=12)
    return r.returncode == 0, "reinject rc=%s %s" % (r.returncode, (r.stdout or r.stderr or "").strip()[:120])


def stage_g2_ack(socket, role, surface, stub):
    """G2 핸드셰이크 ack — 부활 노드가 원장 대조 핑에 응답하는지(M7). 응답 없으면
    타임아웃 → unverified 격하 모드로 전진(무한 보류 금지). stub은 응답자가 없으므로
    best-effort 로 시도만 하고 결과를 저널에 남긴다."""
    r = cys("reinject", "--check", "--role", role, "--surface", surface, "--timeout", "4",
            socket=socket, timeout=10)
    acked = (r.returncode == 0) and ("각성" in (r.stdout or "") or "awake" in (r.stdout or "").lower())
    return acked, "g2 ack=%s (%s)" % (acked, (r.stdout or r.stderr or "").strip()[:120])


# ------------------------------------------------------------------ restore 상태머신

def cmd_restore(args):
    global _ACTIVE_EPOCH
    socket = args.socket
    ticket = args.ticket or "default"
    stub = args.stub
    # ★Phase 6: 이 부팅 세대(재시작마다 변경)를 취득 — 저널 완료 마킹의 유효성 기준.
    _ACTIVE_EPOCH = get_boot_epoch(socket)
    # M5: 이번 시도 기록 + 차단기 판정
    if not args.no_breaker:
        opened, attempts = breaker_check_and_record(socket)
        if opened:
            log("★M5 회로차단기 OPEN — %ss 내 %d회 부활 시도(임계 %d). 자동 부활 정지." % (
                BREAKER_T, len(attempts), BREAKER_N))
            prop = rollback_proposal(socket)
            out = {"phoenix_restore": "BREAKER_OPEN", "attempts_in_window": len(attempts),
                   "threshold": BREAKER_N, "window_secs": BREAKER_T,
                   "rollback_proposal": prop,
                   "alert": "정지 후 사람 승인 필요 — 자동 롤백/재부활을 실행하지 않는다."}
            print(json.dumps(out, ensure_ascii=False, indent=2))
            return out

    j = load_journal(socket, ticket)
    # ★Phase 4: 대상 판정 근거 = actual-state(topology)가 아니라 desired 로스터.
    # 관측을 조기·단조 영속해 topology 침식(부분 부활 후 미부활 역할 삭제)에 면역시킨다(§12).
    entries, _tombstones = observe_and_persist_roster(socket)
    live = live_role_surfaces(socket)

    # 대상 = desired 로스터에 있으나 살아있지 않은(또는 exited) 역할
    def _alive(role):
        for s in live.get(role, []):
            if not s["exited"]:
                return True
        return False

    target_roles = args.roles or [r for r in entries if not _alive(r)]
    if not target_roles:
        log("부활 대상 죽은 역할 0 — restore 무작업(멱등).")
    # dedup(P4): 이 티켓 저널에서 이미 verify까지 done 인 역할은 skip
    pending = [r for r in target_roles if not stage_done(j, r, "verify")]
    log("티켓=%s · 대상역할=%s · 이번 진행=%s (완료 skip=%s)" % (
        ticket, target_roles, pending, [r for r in target_roles if r not in pending]))

    # ── spawn 단계(공유): production=cys restore 1회 / surrogate=역할별 stub ──
    role_surface = {}
    forced_sids = {}
    if args.stub_sids:
        try:
            forced_sids = json.loads(args.stub_sids)
        except Exception:
            forced_sids = {}
    # 이미 완료(재개)된 역할 먼저 매핑
    for role in pending:
        if stage_done(j, role, "spawn"):
            role_surface[role] = j["roles"][role].get("surface")
            jevent(j, role, "spawn", "skip", "이미 완료 — 재개")

    # ── ★Phase 10: 스폰 완결성(retry-until-full) — 미스폰 역할을 백오프로 재시도한다(DRILL_LIVE_3 cso 3/4 수리).
    #    prod: cys restore 는 idempotent(죽은 역할만 재스폰)이라 재호출로 미스폰 역할만 다시 시도된다.
    #    stub: 역할별 재시도. 스폰 후 settle·회차별 backoff 증가로 동시 경합(부활 폭풍)을 완화한다. ──
    need = [r for r in pending if not stage_done(j, r, "spawn") and r not in role_surface]
    attempt = 0
    while need and attempt <= SPAWN_RETRIES:
        if stub:
            still = []
            for role in need:
                exp = entries.get(role, {}).get("session_id", "")
                observed = forced_sids.get(role, exp)
                ref, msg = spawn_surrogate(socket, role, observed, attempt=attempt)
                if ref:
                    role_surface[role] = ref
                    j["roles"].setdefault(role, {"stages": {}})["surface"] = ref
                    j["roles"][role]["expected_sid"] = exp
                    mark_stage(j, role, "spawn", True, "%s (attempt %d)" % (msg, attempt))
                    jevent(j, role, "spawn", "ok", "%s (attempt %d)" % (msg, attempt))
                else:
                    still.append(role)
                    jevent(j, role, "spawn", "retry" if attempt < SPAWN_RETRIES else "fail",
                           "%s (attempt %d)" % (msg, attempt))
        else:
            res = spawn_production(socket, need, include_master=args.include_master)
            jevent(j, "*", "spawn", "ok" if res["rc"] == 0 else "fail",
                   "attempt %d · %s" % (attempt, json.dumps(res, ensure_ascii=False)))
            time.sleep(SPAWN_SETTLE)  # surface 등장 정착 대기(readiness 경합 완화)
            live2 = live_role_surfaces(socket)
            still = []
            for role in need:
                alive = [s for s in live2.get(role, []) if not s["exited"]]
                if alive:
                    ref = alive[0]["surface"]
                    role_surface[role] = ref
                    j["roles"].setdefault(role, {"stages": {}})["surface"] = ref
                    j["roles"][role]["expected_sid"] = entries.get(role, {}).get("session_id", "")
                    mark_stage(j, role, "spawn", True, "cys restore → %s (attempt %d)" % (ref, attempt))
                else:
                    still.append(role)
        if not still:
            need = []
            break
        attempt += 1
        need = still
        if attempt <= SPAWN_RETRIES:
            backoff = SPAWN_BACKOFF * attempt  # 회차마다 증가(경합 완화)
            log("★완결성 재시도: 미스폰 역할=%s → %d회차(backoff %.1fs)" % (still, attempt, backoff))
            time.sleep(backoff)

    # ── ★Phase 11: 독약 세션 fresh-spawn fallback(§15 · DRILL_LIVE_4 수리) ──
    #    resume(세션핀) 재시도가 소진됐는데도 미스폰인 역할 = 세션이 독약(resume 불가)일 개연. 무한 재시도로
    #    roster 를 막지 않고, 세션핀을 버리고 fresh(무 resume) 재기동으로 '강등'해 부활을 마무리한다.
    #    fresh 는 원 세션 보존이 아니라 새 세션 + 디렉티브/원장 재주입이다(정직: resumed→fresh 전환을 저널에 명시).
    #    fresh 는 최후수단 — resume 성공/재시도 회복은 이 지점에 오지 않는다.
    if need and POISON_FRESH_FALLBACK:
        fresh_still = []
        for role in need:
            exp = entries.get(role, {}).get("session_id", "")
            if stub:
                # fresh stub = 새 세션(원 poison sid 아님)으로 뜬다 — observed≠expected 로 정직 반영.
                fresh_sid = "FRESH-" + _slug(role)
                ref, msg = spawn_surrogate(socket, role, fresh_sid, attempt=attempt, mode="fresh")
            else:
                agent = entries.get(role, {}).get("agent", "claude")
                res = spawn_fresh_production(socket, role, agent)
                time.sleep(SPAWN_SETTLE)
                alive = [s for s in live_role_surfaces(socket).get(role, []) if not s["exited"]]
                ref = alive[0]["surface"] if alive else None
                msg = "cys launch-agent(fresh·rc=%s) → %s" % (res["rc"], ref or res["out"])
            if ref:
                role_surface[role] = ref
                rr = j["roles"].setdefault(role, {"stages": {}})
                rr["surface"] = ref
                rr["expected_sid"] = exp            # 원 세션핀(독약) 보존 기록 — verify 에서 '보존 실패'로 정직 대조
                rr["fresh_fallback"] = True          # ★정직: resumed→fresh 강등(세션 보존 포기·의도적 전환)
                mark_stage(j, role, "spawn", True, "★fresh 강등(독약 세션): " + msg)
                jevent(j, role, "spawn", "fresh_fallback",
                       "resume %d회 소진→fresh 강등(무 resume 재기동): %s" % (SPAWN_RETRIES, msg))
                log("★독약 세션 fresh 강등: role=%s → %s (resume 불가 → 무 resume 부활)" % (role, ref))
            else:
                fresh_still.append(role)
                jevent(j, role, "spawn", "fail", "fresh 강등도 실패: %s" % msg)
        need = fresh_still

    # 재시도(resume) + fresh 강등 모두 소진 후에도 미스폰인 역할 = 정직 마킹(완결성 판정에서 INCOMPLETE 로 escalation)
    for role in need:
        mark_stage(j, role, "spawn", False, "재시도 %d회 + fresh 강등 소진 후에도 surface 미발견" % SPAWN_RETRIES)
        jevent(j, role, "spawn", "fail", "재시도 %d회 + fresh 강등 소진 — 부활 실패(INCOMPLETE)" % SPAWN_RETRIES)
    save_journal(socket, ticket, j)

    # ── 역할별 하위 단계: ready → resume → reinject → g2_ack → verify ──
    for role in pending:
        surface = role_surface.get(role)
        if not surface:
            jevent(j, role, "ready", "fail", "surface 없음 — 하위 단계 skip")
            continue
        # ready
        if not stage_done(j, role, "ready"):
            ok, ev = stage_ready(socket, role, surface, stub)
            mark_stage(j, role, "ready", ok, ev); jevent(j, role, "ready", "ok" if ok else "fail", ev)
            save_journal(socket, ticket, j)
        # resume(observe session · ③ grace 폴링)
        if not stage_done(j, role, "resume"):
            sid, ev = stage_observe_session(socket, surface, stub, role=role)
            j["roles"][role]["observed_sid"] = sid
            mark_stage(j, role, "resume", sid is not None, "observed_sid=%s | %s" % (sid, ev))
            jevent(j, role, "resume", "ok" if sid else "fail", "observed_sid=%s" % sid)
            save_journal(socket, ticket, j)
        # reinject
        if not stage_done(j, role, "reinject"):
            ok, ev = stage_reinject(socket, role, surface, stub)
            mark_stage(j, role, "reinject", ok, ev); jevent(j, role, "reinject", "ok" if ok else "warn", ev)
            save_journal(socket, ticket, j)
        # g2_ack (best-effort; 실패해도 전진하되 verify에서 정직 라벨)
        if not stage_done(j, role, "g2_ack"):
            ok, ev = stage_g2_ack(socket, role, surface, stub)
            mark_stage(j, role, "g2_ack", ok, ev); jevent(j, role, "g2_ack", "ok" if ok else "degraded", ev)
            save_journal(socket, ticket, j)
        # verify (M9 핵심): observed_sid == expected_sid 이며 비어있지 않아야 VERIFIED
        exp = j["roles"][role].get("expected_sid", "")
        obs = j["roles"][role].get("observed_sid", None)
        fresh_fb = j["roles"][role].get("fresh_fallback", False)
        # ★Phase 11: fresh 강등(독약 세션)은 fork(오복원)가 아니라 '의도적 세션 폐기 후 재기동'이다.
        # 세션 보존 실패는 정직히 밝히되(verified 아님) 실패(unverified/failed)로 오분류하지 않는다 —
        # 별도 outcome 'fresh' 로 라벨링(원 세션 독약 → 무 resume 부활·디렉티브/원장 재주입).
        if fresh_fb:
            outcome = "fresh"
            reason = ("★독약 세션 fresh 강등(원 세션 %r unresumable → 무 resume 새 세션 %r·디렉티브/원장 재주입). "
                      "정직: 세션 보존 아님·의도적 전환(fork/오복원 아님·roster 부활 완료)" % (exp, obs))
        else:
            verified = bool(exp) and bool(obs) and (exp == obs)
            outcome = "verified" if verified else "unverified"
            # ★Phase 5 ③: transient(재핀 전·미관측)와 fork(진짜 오복원·상이 세션)를 구분해 라벨링.
            # 둘 다 unverified(정직성 불변)지만 사유를 남겨 라이브 grace 캘리브레이션·진단을 돕는다.
            if not verified:
                if not obs:
                    reason = "transient(세션 재핀 전 — grace 소진·미관측)"
                elif exp and obs != exp:
                    reason = "fork(관측 세션≠핀 — 진짜 오복원 의심)"
                else:
                    reason = "핀 부재(expected 미기록)"
            else:
                reason = "세션 일치"
        j["roles"][role]["outcome"] = outcome
        j["roles"][role]["verify_reason"] = reason
        mark_stage(j, role, "verify", True, "M9: expected=%r observed=%r → %s (%s)" % (exp, obs, outcome, reason))
        jevent(j, role, "verify", outcome, "expected=%r observed=%r [%s]" % (exp, obs, reason))
        save_journal(socket, ticket, j)

    # ── M9 정직한 최종 enum ──
    outcomes = {r: j["roles"].get(r, {}).get("outcome", "failed") for r in target_roles}
    fresh_fallback_roles = [r for r in target_roles if outcomes.get(r) == "fresh"]  # ★Phase11 정직 명시
    all_verified = target_roles and all(outcomes.get(r) == "verified" for r in target_roles)
    # ★Phase11: 전원이 verified 또는 fresh(독약→fresh 강등)면 roster 는 부활했다. 단 일부 세션은 보존 못 했으므로
    #   VERIFIED 로 뭉뚱그리지 않고 VERIFIED_FRESH 로 정직히 구분한다(세션 보존 실패를 숨기지 않는다).
    all_revived = target_roles and all(outcomes.get(r) in ("verified", "fresh") for r in target_roles)
    any_unver = any(outcomes.get(r) == "unverified" for r in target_roles)
    if not target_roles:
        final = "NOOP"
    elif all_verified:
        final = "VERIFIED"
        breaker_reset(socket)  # 성공 부활은 차단기 창 리셋
    elif all_revived:
        final = "VERIFIED_FRESH"    # 전원 부활했으나 일부는 독약 세션→fresh 강등(정직)
        breaker_reset(socket)       # roster 전원 생존 = 성공 부활 → 차단기 창 리셋
    elif any_unver:
        final = "UNVERIFIED"
    else:
        final = "FAILED"

    # ── ★Phase 10: readiness 기반 완결성 판정 (프로세스 존재 아닌 실 ready_marker + surface 생존) ──
    #    phoenix_restore(세션 검증 enum)와 직교하는 차원 — '전원 부활했는가'를 정직하게 답한다.
    live_end = live_role_surfaces(socket)
    alive_refs = {s["surface"] for ss in live_end.values() for s in ss if not s["exited"]}

    def _revived_complete(role):
        if not stage_done(j, role, "ready"):  # 실 ready_marker 관측(실응답)만 인정
            return False
        surf = j["roles"].get(role, {}).get("surface")
        return bool(surf) and surf in alive_refs

    incomplete_roles = [r for r in target_roles if not _revived_complete(r)]
    ready_roles = [r for r in target_roles if r not in incomplete_roles]
    if not target_roles:
        completeness = "NOOP"
    elif not incomplete_roles:
        completeness = "COMPLETE"
    else:
        completeness = "INCOMPLETE"

    honesty = ("★M9: phoenix_restore 값만 신뢰하라. UNVERIFIED/FAILED 는 정상 완료가 아니다 "
               "(자기채점 금지·무출력을 정상으로 해석 금지). 세션 대조가 일치할 때만 VERIFIED. "
               "★완결성: COMPLETE 는 roster 전원이 실 ready_marker 로 응답 가능함을 뜻한다.")
    if completeness == "INCOMPLETE":
        honesty += (" ★INCOMPLETE — 재시도 소진 후에도 미부활 역할=%s. 침묵 성공 금지: "
                    "이 역할들은 실제로 부활하지 않았다(escalation 필요·master/사람 개입)." % incomplete_roles)
    if fresh_fallback_roles:
        honesty += (" ★fresh 강등 역할=%s: 원 세션이 독약(resume 불가)이라 무 resume 로 새 세션을 기동하고 "
                    "디렉티브/원장을 재주입했다(세션 보존 실패를 정직히 밝힘 — roster 는 부활 완료). "
                    "독약 세션이 무한 재시도로 roster 를 막지 않게 유한 강등했다(§15·DRILL_LIVE_4)." % fresh_fallback_roles)

    result = {
        "phoenix_restore": final,
        "completeness": completeness,          # ★Phase10: readiness 기반 전원 부활 판정
        "incomplete_roles": incomplete_roles,  # ★Phase10: 미부활 역할 정직 명시(침묵 성공 금지)
        "fresh_fallback_roles": fresh_fallback_roles,  # ★Phase11: 독약 세션→fresh 강등 역할 정직 명시
        "ready_roles": ready_roles,
        "ticket": ticket,
        "boot_epoch": _ACTIVE_EPOCH,      # ★Phase6: 이 부활이 판정 기준으로 쓴 세대
        "epoch_gate": EPOCH_GATE,
        "backend": "surrogate(stub)" if stub else "production(cys restore)",
        "target_roles": target_roles,
        "per_role_outcome": outcomes,
        "journal": journal_path(socket, ticket),
        "honesty_note": honesty,
    }
    # ★M9 계약: 미검증 결과에는 정상완료 주장 문자열을 절대 넣지 않는다(위 dict에도 없음)
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return result


# ------------------------------------------------------------------ B1 조정 패스

def cmd_reconcile(args):
    """재기동 시 위임 대장(topology) vs 실측(surface·WORKER_TODO) 대조 → 불일치 보고.
    부활 직후 첫 행동은 '작업 계속'이 아니라 '원장 대조'(§10.4)."""
    socket = args.socket
    # ★Phase 4: 대장 = actual topology 대신 desired 로스터(침식 면역·§12). 관측을 조기 영속.
    roster, tombstones = observe_and_persist_roster(socket)
    live = live_role_surfaces(socket)
    todo = _read_worker_todo()

    expected_roles = sorted(roster.keys())
    alive_roles = [role for role, ss in live.items() if role != "-" and any(not s["exited"] for s in ss)]

    missing = [r for r in expected_roles if r not in alive_roles]           # 대장엔 있는데 죽음
    extra = [r for r in alive_roles if r not in expected_roles]             # 대장에 없는 생존
    # 세션 불일치: 살아있으나 session_id 대조 불가(재기동 후 미검증)
    sid_map = {r: roster[r].get("session_id") for r in expected_roles}

    report = {
        "reconcile": "B1",
        "desired_roster_path": desired_roster_path(socket),
        "tombstones(의도적 폐역)": sorted(tombstones),
        "expected_roles(desired 대장)": expected_roles,
        "alive_roles(실측)": alive_roles,
        "MISSING(대장O/실측X=부활필요)": missing,
        "EXTRA(대장X/실측O=미등록생존)": extra,
        "worker_todo_inflight": todo,
        "expected_session_ids": sid_map,
        "verdict": ("CONVERGED(대장=실측 일치)" if not missing and not extra
                    else "DIVERGED(불일치 — 위 MISSING/EXTRA 처리 필요)"),
        "next_action_note": "MISSING 있으면 phoenix restore, EXTRA 있으면 등록/정리 — 자동 진행 아님(사람/master 판단).",
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return report


def cmd_tombstone(args):
    """의도적 폐역(roster에서 영구 제외) — 상태를 '줄이는' 유일 경로(§12 원칙2). transient 사망과 명시
    폐역을 구분해, 폐역된 대상은 부활/보호 집합에서 빠진다. --dept 면 부서 dept_roster 에 적용(Phase7 대칭)."""
    socket = args.socket
    is_dept = getattr(args, "dept", False)
    if is_dept:
        roster, tombstones = load_dept_roster(socket)
        path = dept_roster_path(socket)
        kind = "dept"
    else:
        roster, tombstones = load_desired_roster(socket)
        path = desired_roster_path(socket)
        kind = "role"
    name = args.role
    if args.remove:
        tombstones.discard(name)
        action = "폐역 해제(재편입 가능)"
    else:
        tombstones.add(name)
        roster.pop(name, None)
        action = "폐역(보호집합에서 제외 — 부활 안 함)"
    _atomic_write_json(path, {"roster": roster, "tombstones": sorted(tombstones), "updated_at": _now()})
    out = {"tombstone": name, "kind": kind, "action": action, "tombstones": sorted(tombstones),
           "remaining": sorted(roster.keys())}
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


def cmd_roster(args):
    """desired 로스터(대장) 현황 — actual topology와 분리된 선언 상태를 노출(§12)."""
    socket = args.socket
    roster, tombstones = observe_and_persist_roster(socket)
    live = live_role_surfaces(socket)
    alive = {r for r, ss in live.items() if r != "-" and any(not s["exited"] for s in ss)}
    topo_roles = sorted(e.get("role") for e in read_topology(socket).get("entries", []) if e.get("role"))
    dept_roster, dept_tomb = observe_and_persist_depts(socket)  # ★Phase7: 부서도 보호집합에 노출
    out = {
        "desired_roster(선언·침식 면역)": sorted(roster.keys()),
        "tombstones(의도적 폐역)": sorted(tombstones),
        "actual_topology(라이브·침식됨)": topo_roles,
        "alive_now": sorted(alive),
        "dead_by_desired(부활 대상)": sorted(r for r in roster if r not in alive),
        "dept_roster(부서 보호집합·자동 상속)": sorted(dept_roster.keys()),
        "dept_tombstones": sorted(dept_tomb),
        "note": "desired−alive 로 죽은 역할을 판정한다 — topology(actual)가 침식돼도 NOOP 오판이 없다. "
                "부서는 glob∪registry 로 자동 발견돼 dept_roster 에 단조 등재된다(손 배선 0).",
    }
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


def cmd_inherit(args):
    """★Phase 7 자동 보호 상속 primitive: 현재 라이브 노드 role + 발견 부서를 보호집합(rosters)에 능동 포착한다.
    '태어날 때부터 보호' = 노드/부서 창조 시점 또는 주기 reconciler 가 이 명령을 호출하면 손 배선 없이 편입된다.
    단조(관측→박제)·크래시 잔존·명시 tombstone 만 제거. 실 depts.json 무접촉(읽기 전용).
    ※구현 계층 권고: cysd 무변경. 이 primitive 를 (a)launch-agent 후행 훅 또는 (b)`cys schedule` 주기 reconciler 로
      배선하면 창조시점 자동 상속이 완성된다(1회 배선·부서/노드당 손 배선 0)."""
    socket = args.socket
    node_roster, node_tomb = observe_and_persist_roster(socket)
    dept_roster, dept_tomb = observe_and_persist_depts(socket)
    live = live_role_surfaces(socket)
    alive = sorted(r for r, ss in live.items() if r != "-" and any(not s["exited"] for s in ss))
    out = {
        "inherit": "OK",
        "node_roster(보호집합)": sorted(node_roster.keys()),
        "node_tombstones": sorted(node_tomb),
        "alive_nodes_now": alive,
        "dept_roster(보호집합)": sorted(dept_roster.keys()),
        "dept_tombstones": sorted(dept_tomb),
        "note": "노드·부서가 발견 시점에 자동 편입(손 배선 0). 크래시는 roster 잔존=부활 대상. "
                "명시 close-surface/kill→tombstone 만 제거. 실 depts.json 읽기 전용(무접촉).",
    }
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


def _read_worker_todo():
    """WORKER_TODO 에서 미완(- [ ]) 항목 개수와 최근 섹션 제목을 추출(실측 요약)."""
    cand = os.path.join(os.environ.get("CYS_PACK_DIR", os.path.join(HOME, ".cys", "pack")),
                        "round", "WORKER_TODO.md")
    if not os.path.exists(cand):
        return {"path": cand, "exists": False}
    txt = open(cand, errors="replace").read()
    open_items = txt.count("- [ ]")
    done_items = txt.count("- [x]")
    secs = re.findall(r"^#\s*(.+)$", txt, re.M)
    return {"path": cand, "exists": True, "open_items": open_items, "done_items": done_items,
            "last_section": secs[-1][:80] if secs else None}


# ------------------------------------------------------------------ status

def _protection_grade():
    """★Phase 8: 정직한 보호등급(GREEN/AMBER/RED)을 javis_backup 에서 가져온다(앵커 불요·RED 기본).
    백업 도구가 없거나 실패해도 status 는 죽지 않는다 — 보호 미상은 정직하게 RED 로 보고."""
    try:
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        import javis_backup
        return javis_backup.protection_status()
    except Exception as e:
        return {"grade": "RED", "reasons": ["보호 상태 산출 불가(%s) — 정직 기본 RED" % type(e).__name__]}


def cmd_status(args):
    socket = args.socket
    home = phoenix_home(socket)
    journals = [f for f in os.listdir(home) if f.startswith("journal-")] if os.path.isdir(home) else []
    bp = breaker_file(socket)
    breaker = json.load(open(bp)) if os.path.exists(bp) else {"attempts": []}
    now = _now()
    recent = [t for t in breaker.get("attempts", []) if now - t <= BREAKER_T]
    st = {
        "phoenix_home": home,
        "boot_epoch": get_boot_epoch(socket),   # ★Phase6: 현재 데몬 세대(하네스가 동일 문자열 취득에 사용)
        "epoch_gate": EPOCH_GATE,
        "journals": journals,
        "breaker_recent_attempts": len(recent),
        "breaker_threshold": "%d회 / %ds" % (BREAKER_N, BREAKER_T),
        "breaker_state": "OPEN(정지)" if len(recent) >= BREAKER_N else "CLOSED(정상)",
        "protection": _protection_grade(),  # ★Phase8: 정직한 백업 보호등급(M2·§11.5)
        "honesty": "이 상태는 자기채점이 아니다 — 부활 라벨은 restore의 M9 verify(세션 대조)로만 VERIFIED. "
                   "protection 등급은 백업/암호화/오프사이트 앵커의 진실만 말한다(무방비=RED, 숨김 없음).",
    }
    print(json.dumps(st, ensure_ascii=False, indent=2))
    return st


# ------------------------------------------------------------------ ⑥ 독립 수동 복원 스크립트

MANUAL_RESTORE_TEMPLATE = r'''#!/bin/bash
# manual_restore.sh — 불사조 '독립 수동 복원' 경로 (M1 출하 조건 · 데몬/hook 비의존 자기완결 평문)
# 자동 부활(cys phoenix/restore)이 불능일 때, 사람이 이 세대 스냅샷 안에서 직접 조직을 재건한다.
# 의존: cys 바이너리 + 같은 폴더의 topology.json 사본. 그 외 어떤 데몬 상태·hook·팩 로직에도 의존하지 않는다.
# ★이 스크립트는 참석(attended) 경로다 — 사람이 읽고 한 줄씩 확인하며 실행한다(§11.1 하한1: 유인 복구는 잠기지 않는다).
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
TOPO="$HERE/topology.json"
echo "== 불사조 수동 복원 (세대: $HERE) =="
if [ ! -f "$TOPO" ]; then echo "!! topology.json 없음 — 복원 불가"; exit 1; fi
echo "재건 대상 역할:"; python3 -c "import json;[print(' -',e['role'],'/',e.get('agent'),'/ sid',e.get('session_id')) for e in json.load(open('$TOPO'))['entries']]"
echo ""
echo "아래 명령을 한 줄씩 확인 후 실행하라(순차 기동 — 동시 resume 폭주 방지 §10.4):"
python3 - "$TOPO" <<'PY'
import json,sys
t=json.load(open(sys.argv[1]))
for e in t.get('entries',[]):
    role=e['role']; agent=e.get('agent','claude')
    print("cys launch-agent --role %s --agent %s   # 기동 후 각성 확인, 필요시 cys reinject --role %s" % (role, agent, role))
PY
echo ""
echo "★기동 후 첫 행동 = 원장 대조(G2), 작업 재개 아님. 각 노드가 SESSION_STATE/자기 TODO를 읽고 정합 후 대기."
'''


def cmd_gen_manual(args):
    """세대 스냅샷 디렉터리(또는 하네스 지정 위치)에 manual_restore.sh + topology.json 사본 동봉."""
    socket = args.socket
    dest = args.dest or os.path.join(phoenix_home(socket), "generation-manual")
    os.makedirs(dest, exist_ok=True)
    topo = read_topology(socket)
    # topology 사본(수동 경로의 유일 의존물)
    _atomic_write_json(os.path.join(dest, "topology.json"),
                       {"entries": topo.get("entries", []), "updated_at": topo.get("updated_at", 0)})
    sp = os.path.join(dest, "manual_restore.sh")
    with open(sp, "w") as f:
        f.write(MANUAL_RESTORE_TEMPLATE)
    os.chmod(sp, 0o755)
    out = {"manual_restore_script": sp, "topology_copy": os.path.join(dest, "topology.json"),
           "self_contained": True,
           "note": "데몬/hook 비의존 — cys 바이너리 + 이 폴더의 topology.json 만 있으면 사람이 재건 가능(§11.1 하한2 독립성)."}
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


# ------------------------------------------------------------------ M4 쓰기 보호(생성만·적용 금지)

def protected_paths():
    """M4 보호 대상 목록 — ★전부 HOME/config 파생(개인 경로/계정 리터럴 하드코딩 0·모든 사용자 동일 적용).
    사용자 config soul 은 env(CYS_SOUL_PATH 우선, 없으면 CLAUDE_CONFIG_DIR/soul.md)에서 파생한다 — 없으면
    pack soul 만 보호(누구의 개인 config 경로도 소스에 박지 않는다). Phase9 하드코딩 감사 수리."""
    hp = "$HOME/.cys/pack"
    paths = [
        hp + "/agents.json",
        hp + "/bin/javis_phoenix.py",
        hp + "/bin/javis_state_snapshot.py",
        hp + "/bin/javis_backup.py",
        hp + "/directives",
        hp + "/soul.md",
    ]
    soul = os.environ.get("CYS_SOUL_PATH")
    if not soul:
        ccd = os.environ.get("CLAUDE_CONFIG_DIR")
        if ccd:
            soul = os.path.join(ccd, "soul.md")
    if soul and soul not in paths:
        paths.append(soul)  # 사용자 자신의 env 로 해소된 경로(런타임 사용자 데이터·소스 리터럴 아님)
    return paths


def cmd_gen_protect(args):
    """M4 역할기반 쓰기 보호 스크립트 생성. ★기본 DRY-RUN — 라이브 파일에 chflags 를 적용하지 않는다.
    실제 적용은 master 검증 + 소유자(owner) 승인 게이트(§10.3 자기수정 금지) 후 별도 --apply 로만."""
    socket = args.socket
    dest = args.dest or os.path.join(phoenix_home(socket), "phoenix_protect.sh")
    protected = protected_paths()
    body = "#!/bin/bash\n"
    body += "# phoenix_protect.sh — M4 부활 파일 쓰기보호 (워커/리뷰어 쓰기 차단)\n"
    body += "# ★기본 DRY-RUN. 실제 잠금은 반드시 master 검증 + 소유자(owner) 승인 후 './phoenix_protect.sh --apply'.\n"
    body += "# 해제(uchg 제거)는 GUI+sudo 물리 참석 경로에서만, 즉시 기록·RED(§11.1 하한2 참석성).\n"
    body += "set -u\nMODE=\"${1:-dry-run}\"\n"
    body += "FILES=(\n" + "\n".join('  "%s"' % p for p in protected) + "\n)\n"
    body += '''for f in "${FILES[@]}"; do
  ff=$(eval echo "$f")
  if [ "$MODE" = "--apply" ]; then
    echo "[apply] chflags uchg $ff"; chflags uchg "$ff" 2>/dev/null || echo "  (실패/부재: $ff)"
  else
    echo "[dry-run] would: chflags uchg $ff  (PreToolUse hook + uchg — 지금은 적용 안 함)"
  fi
done
echo "※ hook 사망=조용한 해제 방향이므로 hook 생존을 신뢰 원장의 감시 항목에 포함할 것(§11.2 meta-drill)."
'''
    with open(dest, "w") as f:
        f.write(body)
    os.chmod(dest, 0o755)
    out = {"protect_script": dest, "applied": False, "mode": "dry-run",
           "note": "라이브 파일에 잠금을 적용하지 않았다(이 티켓=적용 금지). master 검증 후 별도 --apply."}
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


# ------------------------------------------------------------------ ③ launchd 관리 무결성 (Phase 11)
# 재부팅 자동기동(KeepAlive·RunAtLoad)의 토대 = launchd 등록이 intact 여야 한다. 드릴/복원 절차가 데몬을
# unload(bootout) 하면 '복원까지' 보장해야 하는데, 지금은 관리 상태(등록됨 vs 고아)를 점검·assert 할
# primitive 가 없다. 이 절이 그 primitive 를 더한다:
#   · launchd_status(label): managed(로드+KeepAlive/RunAtLoad intact) / orphan(프로세스는 살아있으나 관리 밖·
#     재부팅 자동기동 안 됨) / unmanaged(로드 자체 없음) 를 분류. ★읽기 전용(launchctl list/print)만 —
#     라이브 데몬을 재시작·변경하지 않는다.
#   · launchd_ensure(label, plist): 미관리/고아면 bootstrap 으로 재등록해 관리 상태를 '복원까지 보장'.
# 격리·결정론: PHOENIX_LAUNCHCTL 로 fake launchctl 을 주입하면 실 launchctl 무접촉으로 드릴이 돈다.

def _launchctl_bin():
    return os.environ.get("PHOENIX_LAUNCHCTL") or "launchctl"


def _launchctl(*args, timeout=10):
    try:
        return subprocess.run([_launchctl_bin()] + [str(a) for a in args],
                              capture_output=True, text=True, timeout=timeout)
    except Exception as e:
        class _R:
            returncode = 127
            stdout = ""
            stderr = str(e)
        return _R()


def launchd_status(label, running_pid=None):
    """label 의 launchd 관리 상태를 분류한다(읽기 전용). running_pid 가 주어지면 '프로세스는 사는데 관리 밖'
    (orphan)을 구분한다. 반환: {label, loaded, keepalive, runatload, state, evidence}.
      state = managed   : launchctl list 에 존재(로드됨) — 재부팅 자동기동 가능(KeepAlive/RunAtLoad 확인)
              orphan    : 로드 안 됨 + 프로세스는 살아있음(running_pid) — 재부팅되면 안 뜸(관리 이탈)
              unmanaged : 로드 안 됨 + 프로세스도 없음(등록 자체 부재)."""
    r = _launchctl("list", label, timeout=8)
    loaded = (r.returncode == 0)
    out = (r.stdout or "")
    # print 로 KeepAlive/RunAtLoad intact 여부 확인(가능한 경우 — 재부팅 자동기동 토대 키). 부재 시 None.
    keepalive = runatload = None
    if loaded:
        pr = _launchctl("print", "gui/%d/%s" % (os.getuid(), label), timeout=8)
        blob = (pr.stdout or "") + out
        if "KeepAlive" in blob:
            keepalive = "KeepAlive" in blob      # 로드된 plist 에 키 존재 = intact(값 형식은 launchctl 버전차)
        if "RunAtLoad" in blob:
            runatload = "RunAtLoad" in blob
    if loaded:
        state = "managed"
    elif running_pid:
        state = "orphan"
    else:
        state = "unmanaged"
    return {"label": label, "loaded": loaded, "keepalive": keepalive, "runatload": runatload,
            "state": state, "evidence": ("launchctl list rc=%s" % r.returncode)}


def launchd_ensure(label, plist, running_pid=None):
    """관리 무결성 '복원까지 보장': 미관리/고아면 bootstrap(재등록)해 managed 로 되돌린다.
    ★이미 managed 면 무작업(멱등). plist 미존재면 재등록 불가 → 정직히 실패 사유 반환(침묵 성공 금지)."""
    before = launchd_status(label, running_pid=running_pid)
    if before["state"] == "managed":
        return {"ensured": True, "action": "noop(이미 managed)", "before": before, "after": before}
    if not plist or not os.path.exists(plist):
        return {"ensured": False, "action": "재등록 불가(plist 부재)", "plist": plist,
                "before": before, "after": before,
                "note": "복원 실패를 숨기지 않는다 — plist 경로 없이는 launchd 재등록 불가."}
    dom = "gui/%d" % os.getuid()
    r = _launchctl("bootstrap", dom, plist, timeout=12)
    after = launchd_status(label, running_pid=running_pid)
    return {"ensured": after["state"] == "managed", "action": "bootstrap %s %s (rc=%s)" % (dom, plist, r.returncode),
            "before": before, "after": after}


def cmd_launchd_status(args):
    out = launchd_status(args.label, running_pid=args.pid)
    out["honesty"] = ("managed=재부팅 자동기동 토대 intact · orphan=프로세스는 살아있으나 관리 이탈(재부팅되면 안 뜸) · "
                      "unmanaged=등록 부재. 읽기 전용(launchctl list/print) — 데몬을 재시작·변경하지 않는다.")
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


def cmd_launchd_ensure(args):
    out = launchd_ensure(args.label, args.plist, running_pid=args.pid)
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


# ------------------------------------------------------------------ deploy (연기)

def cmd_deploy(args):
    out = {"deploy": "DEFERRED_TO_PHASE3",
           "note": "deploy(quiescent→스냅샷→적용→drill 내장)은 §9.4-3 Phase 3. "
                   "지금은 cysd 최소 내구성 패치(큐 WAL+feed.jsonl fsync)와 함께 첫 실전 drill로 배포 예정."}
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


# ------------------------------------------------------------------ main

def main():
    global CYS
    # 플랫폼 가드(Phase12): 무손실 복원은 launchctl·cys launch-agent(unix) 의존 — 비-Mac/Unix(Windows)에서
    # 호출되면 크래시 대신 정직한 안내로 clean exit. 온디맨드라 자동 실행은 없다.
    if os.name == "nt":
        print("불사조 무손실 복원(phoenix)은 현재 macOS/Unix 전용입니다 — "
              "Windows 패리티는 예정되어 있습니다. "
              "(phoenix restoration is macOS/Unix-only for now; Windows parity planned.)")
        return
    CYS = _which("cys") or "cys"
    ap = argparse.ArgumentParser(description="불사조 부활 저널 상태머신 MVP (M1 게이트)")
    ap.add_argument("--socket", help="대상 데몬 소켓(격리 하네스 소켓 권장 — 라이브 무접촉)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("restore"); p.add_argument("--ticket"); p.add_argument("--stub", action="store_true")
    p.add_argument("--stub-sids", help='role→observed session_id JSON(오복원 시뮬레이션용)')
    p.add_argument("--roles", nargs="*"); p.add_argument("--include-master", action="store_true")
    p.add_argument("--no-breaker", action="store_true")
    sub.add_parser("reconcile")
    sub.add_parser("status")
    sub.add_parser("roster")  # Phase 4: desired 로스터 현황(침식 면역) + Phase7 부서
    sub.add_parser("inherit")  # ★Phase 7: 자동 보호 상속 — 노드+부서 능동 포착
    tb = sub.add_parser("tombstone")  # Phase 4/7: 의도적 폐역(roster 축소 유일 경로)
    tb.add_argument("role"); tb.add_argument("--remove", action="store_true")
    tb.add_argument("--dept", action="store_true")  # Phase7: 부서 dept_roster 대상
    gm = sub.add_parser("gen-manual"); gm.add_argument("--dest")
    gp = sub.add_parser("gen-protect"); gp.add_argument("--dest")
    ls = sub.add_parser("launchd-status")  # ★Phase11: launchd 관리 무결성 점검(managed/orphan/unmanaged)
    ls.add_argument("--label", required=True); ls.add_argument("--pid", type=int)
    le = sub.add_parser("launchd-ensure")  # ★Phase11: 미관리/고아 시 재등록(복원까지 보장)
    le.add_argument("--label", required=True); le.add_argument("--plist"); le.add_argument("--pid", type=int)
    sub.add_parser("deploy")

    args = ap.parse_args()
    {
        "restore": cmd_restore, "reconcile": cmd_reconcile, "status": cmd_status,
        "roster": cmd_roster, "inherit": cmd_inherit, "tombstone": cmd_tombstone,
        "gen-manual": cmd_gen_manual, "gen-protect": cmd_gen_protect, "deploy": cmd_deploy,
        "launchd-status": cmd_launchd_status, "launchd-ensure": cmd_launchd_ensure,
    }[args.cmd](args)


if __name__ == "__main__":
    main()
