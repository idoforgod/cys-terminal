#!/usr/bin/env python3
"""javis_resource_gate.py — P0-3 자원 사전 게이트 (getInvocationBlock의 정액제 번안)

계약(출처: _research/Paperclip_박사급_연구보고서.md §4 P0-3 · §2-7):
- Paperclip의 진짜 런어웨이 차단 = "새 run 시작 전 라이브 재계산해 초과면 착수 거부"(사전 게이트).
- 정액제(Claude Max)엔 달러 예산이 무력하므로 metric을 자원으로 치환:
    servers  = 로컬 dev/서버 프로세스 수         (자원 거버넌스 '서버 누적' 사고 이력)
    nodes    = claude/agy/codex 노드 프로세스 수
    load     = 1분 load average / CPU 코어 수 비율
    context  = 자기보고 컨텍스트 %               (60% /clear 규칙)
- soft/hard 2단(Paperclip warnPercent 사상): soft=경고 후 진행 허용, hard=착수 거부.
- 판정은 결정론: exit code 0=allow · 1=soft warn · 2=hard block. (LLM 자연어 판단 제거)
- "저장값 재신뢰 금지, 매번 재계산" — 게이트는 항상 라이브 측정.

기본 임계(우리 자원 거버넌스 실사고 기준):
  servers  soft 2  / hard 3     (watchdog '3개+' 규칙과 정합 — 사후 kill 전에 사전 차단)
  nodes    soft 12 / hard 18(+동적: max(18, 12 + 활성 부서수*5) — 부서 소켓 존재 기준,
           2026-07-06 CSO 위임 오탐 수정. --nodes-hard 명시 지정 시 그 값 그대로 우선)
  load     soft 1.0×ncpu / hard 2.0×ncpu
  context  soft 50 / hard 60    (60% 도달 전 저장 후 /clear 규칙)

테스트/자동화 주입: --servers-override/--nodes-override/--load-override (라이브 측정 대체).
사용 예: python3 javis_resource_gate.py check --context 42 --json
exit codes: 0 allow · 1 soft · 2 hard · (3+ 내부 오류)
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time

EXIT_ALLOW, EXIT_SOFT, EXIT_HARD = 0, 1, 2

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
    # ★browserd(`bun run …/browserd/server.ts`)는 엔진 파일락이 싱글톤을 강제하므로 '누적되는 dev 서버'가
    # 아니다 — 계수되면 cast pane 1개가 전 노드 착수를 막고, enforce --kill의 pid-tree kill이 사람이 보고
    # 있는 Chromium까지 죽인다(_server_procs가 이 목록을 공유하므로 계수·집행 양쪽이 함께 면제된다).
    r"browserd/server\.ts",
]
NODE_PATTERNS = [r"claude(\s|$)", r"\bagy\b", r"\bcodex\b", r"\bgemini\b"]
# ★2026-07-11 CSO(CEO B승인): codex 노드 1개 = node wrapper + darwin-arm64 vendor native 2프로세스가
# 둘 다 \bcodex\b 매칭 → 이중계수. vendor native를 제외해 codex는 wrapper 1개만 계수(계수 인플레이션 차단).
NODE_EXCLUDE_PATTERNS = [r"codex-darwin-arm64"]

# ★2026-07-06 CSO 위임(master 승인): nodes hard_block 오탐 수정 — A(정적상향)+B(동적 부서가산).
# 부서 1개 상시 기동만으로도 정적 임계(구 12)를 넘어 오탐하던 문제. 부서 소켓 존재=활성 부서로
# 세어 그만큼 임계를 완화한다(전면 동적화(C안)는 보류·백로그 — 이번은 A+B만 채택).
NODES_HARD_DEFAULT = 18   # STEP A 정적 floor(구 12) — depts 0~1일 때도 이 완화는 유지
NODES_HARD_BASE = 12      # STEP B 동적 가산 base — depts>=2부터 동적값이 floor를 추월
NODES_HARD_PER_DEPT = 5
DEPT_SOCKET_GLOB = "~/.local/state/cys-dept-*/cys.sock"


def _active_dept_count():
    """부서 소켓(cys-dept-*/cys.sock) 존재 개수 = 활성 부서 수(소켓 파일 존재=기동중)."""
    import glob
    return len(glob.glob(os.path.expanduser(DEPT_SOCKET_GLOB)))


# ── ★W6 곱셈 자원 예산(설계 DD-3/W6·오너 2026-07-24) ──
# 부서 수 × 편성 크기 곱셈으로 조직 전체의 투영 에이전트 수를 예산과 대조한다. 역할 중립(CEO·부서장·
# base 동일 물리법칙) · **hard-fail 아님** — 초과 시 편성은 pending-resource 로 '대기·자동 재시도'
# (기존 hard/soft 의미론 보존: 게이트는 master/편성 부트 플로우가 호출하므로 조직 기동을 영영 막지 않는다).
FORMATION_BUDGET_ENV = "CYS_FORMATION_BUDGET"
# 기본 20 근거: complete 편성 1레인 = master+4종 의무 노드(CSO·worker·agy·codex) = 5노드.
#   20 = base + 3부서까지 동시 full 편성(4레인×5) 여유 — 기존 nodes_hard 계열(정적 floor 18·부서당 +5)과
#   같은 자릿수로 정렬(자원 거버넌스 실사고 상한). env CYS_FORMATION_BUDGET 로 하드웨어별 오버라이드.
FORMATION_BUDGET_DEFAULT = 20


def _formation_budget_value(explicit=None):
    """예산 해석 우선순위: 명시 인자 > CYS_FORMATION_BUDGET env > 기본 20. 파싱 실패는 기본값(보수)."""
    if explicit is not None:
        return explicit
    raw = os.environ.get(FORMATION_BUDGET_ENV)
    if raw is None:
        return FORMATION_BUDGET_DEFAULT
    try:
        return int(raw)
    except (TypeError, ValueError):
        return FORMATION_BUDGET_DEFAULT


class BudgetVerdict:
    """곱셈 편성 예산 판정 결과. blocked=True → pending-resource(대기·자동 재시도·hard-fail 아님)."""

    def __init__(self, blocked, projected, budget, live_agents, formation_size, dept_count):
        self.blocked = blocked
        self.projected = projected
        self.budget = budget
        self.live_agents = live_agents
        self.formation_size = formation_size
        self.dept_count = dept_count
        self.state = "pending-resource" if blocked else "ok"

    def __repr__(self):
        rel = ">" if self.blocked else "<="
        return "%s(projected=%d%sbudget=%d · live=%d + depts=%d×size=%d)" % (
            self.state, self.projected, rel, self.budget,
            self.live_agents, self.dept_count, self.formation_size)

    __str__ = __repr__


def formation_budget_check(live_agents, formation_size, dept_count=1, budget=None):
    """곱셈 편성 예산: projected = live_agents + max(1,dept_count)×formation_size 가 budget 초과면 차단.
    역할 중립 · hard-fail 아님(초과=pending-resource 대기). 반환: BudgetVerdict(.blocked·.state)."""
    budget = _formation_budget_value(budget)
    dept_count = max(1, int(dept_count or 1))
    projected = int(live_agents or 0) + dept_count * int(formation_size or 0)
    return BudgetVerdict(projected > budget, projected, budget,
                         int(live_agents or 0), int(formation_size or 0), dept_count)


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


def measure(a):
    # 측정 실패는 0으로 조용히 넘기지 않고 measure_errors로 신호(P-ORCH-1) — 소비자(evaluate)가
    # 최소 soft로 격상해 '측정 실패=조용한 allow'를 차단한다.
    errors = []
    need_ps = a.servers_override is None or a.nodes_override is None
    lines = _ps_lines() if need_ps else None
    ps_failed = need_ps and lines is None

    if a.servers_override is not None:
        servers = a.servers_override
    elif ps_failed:
        errors.append("servers(ps)")
        servers = None
    else:
        servers = _count_matching(lines, SERVER_PATTERNS, SERVER_EXCLUDE_PATTERNS)

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

    # STEP B: --nodes-hard가 argparse 기본값(NODES_HARD_DEFAULT)에서 명시적으로 바뀌지 않았으면
    # 동적 계산 적용, 바뀌었으면(테스트 주입 등) 그 값 그대로 우선 — 동적계산 생략.
    active_depts = _active_dept_count()
    if a.nodes_hard != NODES_HARD_DEFAULT:
        nodes_hard_effective = a.nodes_hard
    else:
        nodes_hard_effective = max(NODES_HARD_DEFAULT,
                                    NODES_HARD_BASE + active_depts * NODES_HARD_PER_DEPT)

    return {"servers": servers, "nodes": nodes,
            "load1": round(load1, 2) if load1 is not None else None,
            "ncpu": ncpu,
            "load_ratio": round(load1 / ncpu, 3) if load1 is not None else None,
            "context_pct": a.context, "measure_errors": errors,
            "active_depts": active_depts, "nodes_hard_effective": nodes_hard_effective}


# ── ★opt-in rate 축(soft-only) — 정액제(Claude Max) 5h rate 사용률 사전 경고 ──
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


def evaluate(m, a):
    checks = []

    def add(metric, value, soft, hard):
        if value is None:
            return
        level = "hard" if value >= hard else ("soft" if value >= soft else "ok")
        checks.append({"metric": metric, "value": value, "soft": soft,
                       "hard": hard, "level": level})

    add("servers", m["servers"], a.servers_soft, a.servers_hard)
    add("nodes", m["nodes"], a.nodes_soft, m["nodes_hard_effective"])
    add("load_ratio", m["load_ratio"], a.load_soft_ratio, a.load_hard_ratio)
    add("context_pct", m["context_pct"], a.context_soft, a.context_hard)

    # ★opt-in rate 축(soft-only) — --rate-check 또는 CYS_GATE_RATE=1일 때만. hard 없음:
    # 게이트는 master 부트 플로우가 호출하므로 rate로 조직 기동을 막지 않는다(soft만 반영).
    if _rate_enabled(a):
        checks.extend(_rate_checks(a))

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
    # ★W6 곱셈 편성 예산 축(opt-in — --formation-size N>0 일 때만). 투영 = 측정 노드 + 부서수×편성크기.
    #   초과=hard(→ 편성 소비자에서 pending-resource 로 대기). formation-size 미지정 시 완전 무영향.
    if getattr(a, "formation_size", 0):
        live = m["nodes"] if m["nodes"] is not None else 0
        depts = a.dept_count if getattr(a, "dept_count", None) else max(1, m["active_depts"])
        bv = formation_budget_check(live_agents=live, formation_size=a.formation_size,
                                    dept_count=depts, budget=a.formation_budget)
        checks.append({"metric": "formation_budget", "value": bv.projected,
                       "soft": bv.budget, "hard": bv.budget + 1,
                       "level": "hard" if bv.blocked else "ok"})
        if bv.blocked:
            worst = "hard"
    verdict = {"ok": "allow", "soft": "soft_warn", "hard": "hard_block"}[worst]
    trips = [c for c in checks if c["level"] != "ok"]
    warnings = []
    if m["measure_errors"]:
        warnings.append("measure_error:" + ",".join(m["measure_errors"]))
    if m["context_pct"] is None:
        warnings.append("context_unmeasured")
    result = {"verdict": verdict, "measured": m, "trips": trips,
              "checks": checks, "warnings": warnings}
    if a.json:
        print(json.dumps(result, ensure_ascii=False, indent=1))
    else:
        print(f"verdict: {verdict}")
        for w in warnings:
            print(f"  ⚠ {w}")
        for c in checks:
            mark = {"ok": "·", "soft": "⚠", "hard": "✗"}[c["level"]]
            print(f"  {mark} {c['metric']}={c['value']} (soft {c['soft']} / hard {c['hard']})")
        if m["measure_errors"]:
            print("measure_error: 자원 측정 실패(ps/load) — 조용한 allow 금지, 최소 soft로 격상. "
                  "측정 환경 확인 후 재시도.")
        if m["context_pct"] is None:
            print("context_unmeasured: --context 미제공 — 컨텍스트 60%/clear 규칙을 검사하지 못함. "
                  "check 시 --context <pct> 전달 권장.")
        if worst == "hard":
            print("hard_block: 착수 거부 — 자원 정리(서버 kill·/clear·노드 회수) 후 재시도하거나 "
                  "master 승인으로 임계 상향. (사후 watchdog와 별개의 사전 게이트)")
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
def _server_procs(lines=None):
    """SERVER_PATTERNS 매칭 (pid, cmd) 목록 — _count_matching과 동일 분류(제외 패턴 포함)."""
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
    return out


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


def main(argv=None):
    p = argparse.ArgumentParser(description="자원 사전 게이트 — 착수 전 차단 (P0-3)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("check")
    c.add_argument("--context", type=float, default=None, help="자기보고 컨텍스트 %%")
    c.add_argument("--json", action="store_true")
    c.add_argument("--servers-soft", type=int, default=2)
    c.add_argument("--servers-hard", type=int, default=3)
    c.add_argument("--nodes-soft", type=int, default=12)
    c.add_argument("--nodes-hard", type=int, default=NODES_HARD_DEFAULT)
    c.add_argument("--load-soft-ratio", type=float, default=1.0)
    c.add_argument("--load-hard-ratio", type=float, default=2.0)
    c.add_argument("--context-soft", type=float, default=50.0)
    c.add_argument("--context-hard", type=float, default=60.0)
    c.add_argument("--servers-override", type=int, default=None, help="테스트 주입")
    c.add_argument("--nodes-override", type=int, default=None, help="테스트 주입")
    c.add_argument("--load-override", type=float, default=None, help="테스트 주입")
    c.add_argument("--rate-check", action="store_true",
                   help="opt-in: 5h rate 사용률 soft 경고 축 추가(env CYS_GATE_RATE=1과 동등)")
    c.add_argument("--rate-soft", type=float, default=80.0, help="rate 5h used_pct soft 임계")
    c.add_argument("--rate-override", default=None,
                   help="테스트 주입 — usage-accounts JSON(accounts 배열) 직접 주입")
    # ★W6 곱셈 편성 예산(opt-in): --formation-size>0 일 때만 예산 축 발화. 초과=hard(pending-resource).
    c.add_argument("--formation-size", dest="formation_size", type=int, default=0,
                   help="편성 로스터 크기(W6 곱셈 예산 발화 — 0=축 비활성)")
    c.add_argument("--dept-count", dest="dept_count", type=int, default=None,
                   help="부서 수(미지정 시 활성 소켓 수 측정값 사용)")
    c.add_argument("--formation-budget", dest="formation_budget", type=int, default=None,
                   help="편성 예산 한도(미지정 시 CYS_FORMATION_BUDGET env 또는 기본 20)")
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

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
