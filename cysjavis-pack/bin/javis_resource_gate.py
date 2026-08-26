#!/usr/bin/env python3
"""javis_resource_gate.py — P0-3 자원 사전 게이트 (getInvocationBlock의 정액제 번안)

계약(출처: _research/Paperclip_박사급_연구보고서.md §4 P0-3 · §2-7):
- Paperclip의 진짜 런어웨이 차단 = "새 run 시작 전 라이브 재계산해 초과면 착수 거부"(사전 게이트).
- 승인 패턴은 **구독제(정액) 전용·종량제 과금 금지**다. 정액 구독엔 달러 예산이라는 브레이크가
  아예 없으므로(쓴 만큼 청구되는 축이 없다) metric을 달러가 아닌 자원으로 치환한다:
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

테스트/자동화 주입: --servers-override/--nodes-override/--load-override (라이브 측정 대체).
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
import json
import os
import re
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
    trips = [c for c in checks if c["level"] != "ok"]
    warnings = []
    if m["measure_errors"]:
        warnings.append("measure_error:" + ",".join(m["measure_errors"]))
    if m["context_pct"] is None:
        warnings.append("context_unmeasured")
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
    # ① 미지 서브커맨드 → EX_USAGE(64), EXIT_HARD(2) 와 분리
    with contextlib.redirect_stderr(io.StringIO()):
        rc = main(["definitely-not-a-subcommand"])
    chk(rc == EXIT_USAGE, "미지 서브커맨드가 EX_USAGE(64) 아님: rc=%r" % rc)
    chk(rc != EXIT_HARD, "사용오류가 hard_block(2)로 오독됨 — argparse↔EXIT_HARD 충돌 잔존")
    # ② 미지 플래그도 동일
    with contextlib.redirect_stderr(io.StringIO()):
        rc = main(["check", "--no-such-flag"])
    chk(rc == EXIT_USAGE, "미지 플래그가 EX_USAGE(64) 아님: rc=%r" % rc)
    # ③ 인자 없음(subparser required) → EX_USAGE
    with contextlib.redirect_stderr(io.StringIO()):
        rc = main([])
    chk(rc == EXIT_USAGE, "서브커맨드 부재가 EX_USAGE(64) 아님: rc=%r" % rc)
    # ④ 정상 판정 경로는 무회귀(override 주입으로 측정 대체 — 라이브 ps 무의존)
    with contextlib.redirect_stdout(io.StringIO()):
        rc = main(["check", "--servers-override", "0", "--nodes-override", "0",
                   "--load-override", "0.0"])
    chk(rc == EXIT_ALLOW, "정상 allow 경로 회귀: rc=%r" % rc)
    with contextlib.redirect_stdout(io.StringIO()):
        rc = main(["check", "--servers-override", "99", "--nodes-override", "0",
                   "--load-override", "0.0"])
    chk(rc == EXIT_HARD, "servers hard 경로 회귀: rc=%r" % rc)
    # ⑤ 내부 예외 → EX_SOFTWARE(70), 'soft'(1) 오분류 아님
    import types
    ns = types.SimpleNamespace(fn=lambda _a: (_ for _ in ()).throw(RuntimeError("boom")))
    saved = _GateArgumentParser.parse_args
    try:
        _GateArgumentParser.parse_args = lambda self, argv=None: ns
        with contextlib.redirect_stderr(io.StringIO()):
            rc = main(["check"])
    finally:
        _GateArgumentParser.parse_args = saved
    chk(rc == EXIT_INTERNAL, "내부 예외가 EX_SOFTWARE(70) 아님: rc=%r" % rc)
    chk(rc != EXIT_SOFT, "내부 예외가 soft_warn(1)로 오분류 — 측정 실패↔자원 경고 융합 잔존")
    # ⑥ 계약 채널: --json 의 stdout 은 순수 JSON(진단 혼입 0)
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(io.StringIO()):
        main(["check", "--json", "--servers-override", "0", "--nodes-override", "0",
              "--load-override", "0.0"])
    try:
        json.loads(buf.getvalue().strip())
    except ValueError as e:
        fails.append("--json stdout 이 순수 JSON 아님: %s" % e)
    # ⑦ ★B1(§2-5): --require-context + context 미제공 → soft(exit 1)·trips 는 비어 있음
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = main(["check", "--json", "--require-context", "--servers-override", "0",
                   "--nodes-override", "0", "--load-override", "0.0"])
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
                   "--nodes-override", "0", "--load-override", "0.0"])
    chk(rc == EXIT_ALLOW, "--require-context+context 42 가 allow 아님: rc=%r" % rc)
    with contextlib.redirect_stdout(io.StringIO()):
        rc = main(["check", "--require-context", "--context", "61", "--servers-override", "0",
                   "--nodes-override", "0", "--load-override", "0.0"])
    chk(rc == EXIT_HARD, "--require-context+context 61 이 hard(2) 아님: rc=%r" % rc)
    # ⑨ 플래그 없는 기존 호출 = 기본 동작 불변(context 미제공 = allow · 회귀 0)
    with contextlib.redirect_stdout(io.StringIO()):
        rc = main(["check", "--servers-override", "0", "--nodes-override", "0",
                   "--load-override", "0.0"])
    chk(rc == EXIT_ALLOW, "플래그 없는 context 미제공이 allow 아님(기본 동작 회귀): rc=%r" % rc)

    # ⑩ ★T9(P3-1·R3-P03-1) 곱셈 편성 예산 축 4형상 — 발화는 (--formation-size ∧ env 정수) 둘 다일 때만.
    #    결정론 확보: --formation-size 0 이면 투영 = nodes_override + depts×0 = nodes_override 라
    #    라이브 부서 소켓 수(_active_dept_count)와 무관하게 판정이 고정된다(밀폐).
    base_argv = ["check", "--servers-override", "0", "--nodes-override", "0",
                 "--load-override", "0.0"]
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
          "비정수 env 가청화·nodes 미측정 무예외)")
    return 0


if __name__ == "__main__":
    if "--self-test" in sys.argv[1:]:
        sys.exit(self_test())
    sys.exit(main())
