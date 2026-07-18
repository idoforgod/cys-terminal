#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_report_gate.py — 하트비트 델타게이트 (무의미 wake 제거 · DESIGN §C1 구현)

5분 시계 잡의 역할을 "발화하라"→"발화 자격을 판정하라"로 전환한다. cysd가
`action:"command"`로 이 스크립트를 5분마다 호출하면:

  ① javis_report.py --json 수집(기존 결정론 자산 재사용)
  ② 정규화(노이즈 필드 블랙리스트 제거) → 직전 스냅샷과 diff
  ③ 분류(우선순위): WARN > DELTA > QUIET > NOCHG
  ④ 라우팅: WARN=master wake(스코프드)+EVT / DELTA=task_progress EVT(LLM 0) /
            NOCHG·QUIET=대장 기록만
  ⑤ 매 판정을 대장(ledger.jsonl)에 append — 기록 두절 자체가 데드맨 경보

설계 원리: 판단 0의 전달은 LLM 비경유. 모든 실패 방향은 fail-open(시끄러움=현행 복귀).

CLI:
  run            기본 실행(판정+배달+대장)
  run --shadow   판정·대장 기록만, 배달 0 (P1 shadow 검증용)
  status         대장 tail 사람 출력

종료 코드: 항상 0 (fail-open — schedule.error만으로는 아무도 안 깨기 때문에 죽지 않는다).
의존성: 파이썬 표준 라이브러리만. 외부 명령(javis_report/event/wakeup·cys)은 Runner로 주입 가능.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time

# javis_report.py IDLE_ALERT_SECS와 동일(절대지침 B3: idle 5분+). 자기보고가 아닌 데몬 실측
# idle_secs로만 판정한다(memory: stale self-report 함정). 여기 재정의(수집 실패 시에도 상수 필요).
IDLE_ALERT_SECS = 300
CYCLE_MINUTES_DEFAULT = 5      # schedule every_minutes=5
STALL_CYCLES_DEFAULT = 6       # 6주기=30분 무진행 → stall 승격(DESIGN 미결 기본값)
QUIET_CYCLES_DEFAULT = 12      # 12주기=60분 QUIET → 세션 주차 후보(P2·CSO 집행)
GAP_CYCLES = 3                 # 직전 대장과 간격 >3주기 = GAP(슬립·재부팅 복귀 위양성 강등)
SCHEMA_VERSION = 1
LEDGER_MAX_BYTES = 5 * 1024 * 1024   # 대장 5MB 도달 시 ledger.jsonl.1로 1세대 로테이션

# 정규화 블랙리스트 — 타임스탬프·수집시각·순서 비결정 항목만 제거한다. 화이트리스트 금지
# (신호 유실 단일 실패점). 미지의 새 필드는 자동으로 diff 대상 = 변화로 감지된다(fail-noisy).
# idle_secs/age_secs는 idle 노드에서 매 주기 증가하는 시간파생 노이즈라 diff에서 제외한다
# — WARN 추출은 정규화 '전' 원문에서 하므로 idle 감지 능력은 손실되지 않는다.
BLACKLIST_KEYS = frozenset({
    "idle_secs", "age_secs", "ts", "timestamp", "collected_at", "generated_at",
    "now", "uptime_secs", "last_seen", "seen_at", "mtime", "updated_at",
})

VERDICT_WARN, VERDICT_DELTA, VERDICT_QUIET, VERDICT_NOCHG = "WARN", "DELTA", "QUIET", "NOCHG"


# ─────────────────────────── 상태 경로·원장 ───────────────────────────

def default_state_dir():
    d = os.environ.get("CYS_REPORT_GATE_DIR")
    if d:
        return d
    return os.path.join(os.path.expanduser("~"), ".cys", "state", "report_gate")


def default_pack_bin():
    # ${CYS_PACK_DIR:-$HOME/.cys/pack}/bin 파이썬 등가. ★launchd 최소 env 전제: fire_command는
    # 데몬 env를 그대로 상속하고, launchd 기동 데몬엔 CYS_PACK_DIR이 없을 수 있다. 이때 __file__
    # 형제 디렉터리(javis_report.py 동거 확인)를 우선 쓴다 — 이 스크립트가 pack/bin에 있으므로 가장
    # 신뢰성 높은 해석이다(worktree·테스트에서도 정확). 그마저 아니면 $HOME/.cys/pack/bin 폴백.
    d = os.environ.get("CYS_PACK_DIR") or os.environ.get("JAVIS_PACK_DIR")
    if d:
        return os.path.join(d, "bin")
    here = os.path.dirname(os.path.abspath(__file__))
    if os.path.isfile(os.path.join(here, "javis_report.py")):
        return here
    return os.path.join(os.path.expanduser("~"), ".cys", "pack", "bin")


def resolve_pack_dir():
    """게이트가 소속을 판단하는 pack_dir(=default_pack_bin의 부모). 데몬 pack 해석과 동일 규칙."""
    d = os.environ.get("CYS_PACK_DIR") or os.environ.get("JAVIS_PACK_DIR")
    if d:
        return d
    return os.path.dirname(default_pack_bin())


# ── 외부 데몬 가드(핫픽스): socket-pack 정합 검사 ──────────────────────────────
# 실측 결함: 부서 데몬(env 오염 — CYS_PACK_DIR=본사 팩 + CYS_SOCKET=dept 소켓)이 본사
# schedule.json을 로드해 command 잡을 중복 실행한다(action:command는 push와 달리 if_absent
# 게이트가 없어 모든 로더에서 실행됨). 부서 데몬 자체 수정은 ACL 금지 → 게이트 자기방어로 해결.
# 정합 규칙:
#   - 본사 팩(realpath == $HOME/.cys/pack): CYS_SOCKET unset 또는 기본 소켓이어야 정합.
#     set인데 기본 소켓이 아니면 = 외부 데몬 컨텍스트 → SKIP.
#   - 부서 팩(basename == pack-dept-<X>): CYS_SOCKET 경로에 cys-dept-<X> 포함 요구(미래 부서 게이트 호환).
#   - 그 외(worktree·테스트 등): 판단 보류 → 정상 진행.
# 가드 자체 오류는 fail-open(정상 진행) — 가드가 본사 실행을 죽이면 안 된다.
DEFAULT_SOCKET = os.path.join("~", ".local", "state", "cys", "cys.sock")


def foreign_daemon_verdict():
    """정합이면 None, 외부 데몬 컨텍스트면 (verdict, reason)."""
    try:
        sock = os.environ.get("CYS_SOCKET")
        pack = os.path.realpath(resolve_pack_dir())
        base = os.path.basename(pack)
        m = re.match(r"pack-dept-(.+)$", base)
        if m:
            token = "cys-dept-%s" % m.group(1)
            if sock and token in sock:
                return None                      # 부서 데몬 정합 → 정상
            return ("SKIPPED_FOREIGN_DAEMON",
                    "dept pack(%s)엔 CYS_SOCKET에 '%s' 필요 — 실제=%s" % (base, token, sock))
        hq = os.path.realpath(os.path.expanduser(os.path.join("~", ".cys", "pack")))
        if pack == hq:
            default_sock = os.path.realpath(os.path.expanduser(DEFAULT_SOCKET))
            if sock and os.path.realpath(os.path.expanduser(sock)) != default_sock:
                return ("SKIPPED_FOREIGN_DAEMON",
                        "본사 팩인데 CYS_SOCKET=%s (기본 소켓 아님) = 외부 데몬 컨텍스트" % sock)
            return None                          # 본사 정합(unset 또는 기본 소켓)
        return None                              # 그 외(worktree·테스트) → 판단 보류·정상 진행
    except Exception:                            # noqa: BLE001 — 가드 오류=fail-open(정상 진행)
        return None


def resolve_cys_bin():
    """`cys` 바이너리 절대 해석 — ★launchd 최소 env는 PATH에 /usr/local/bin이 없을 수 있다.
    CYS_BIN(env) → PATH의 which('cys') → 흔한 절대경로 후보 첫 존재 → 최후 'cys'(PATH 의존)."""
    env = os.environ.get("CYS_BIN")
    if env:
        return env
    import shutil
    w = shutil.which("cys")
    if w:
        return w
    for cand in ("/usr/local/bin/cys", "/opt/homebrew/bin/cys",
                 os.path.expanduser("~/.local/bin/cys")):
        if os.path.isfile(cand):
            return cand
    return "cys"


# ── javis_wakeup.py의 _FileLock 패턴 복제(임포트 대신 복제 + 출처 주석) ──
#   출처: cysjavis-pack/bin/javis_wakeup.py class _FileLock (mkdir 원자성·stale 30초 회수).
#   다중 cysd 데몬·장기 실행 겹침이 stall/quiet 카운터를 이중 증가시키는 경로를 차단한다.
class _FileLock:
    """mkdir 원자성 기반 락. stale(270초+)은 rename으로 원자적 회수.

    ★stale_sec=270(5분 주기 직하): 최악의 직렬 실행(report 수집 + N개 emit + drain)이 30초를
    넘길 수 있다 — stale 30초면 아직 살아 실행 중인 게이트의 락을 다른 인스턴스가 탈취해 카운터를
    이중 증가시킨다(S2-3 재유입). 주기(300초) 직하로 잡아 정상 실행은 절대 stale 판정되지 않게 하되,
    진짜 죽은 락(주기 초과 잔존)은 다음 주기에 회수되게 한다."""

    def __init__(self, path, timeout=2.0, stale_sec=270.0):
        self.path, self.timeout, self.stale_sec = path, timeout, stale_sec

    def __enter__(self):
        deadline = time.time() + self.timeout
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        while True:
            try:
                os.mkdir(self.path)
                return self
            except FileExistsError:
                try:
                    if time.time() - os.stat(self.path).st_mtime > self.stale_sec:
                        os.rename(self.path, "%s.stale.%d" % (self.path, time.time_ns()))
                        continue
                except OSError:
                    pass
                if time.time() > deadline:
                    raise TimeoutError("lock timeout: %s" % self.path)
                time.sleep(0.02)

    def __exit__(self, *exc):
        try:
            os.rmdir(self.path)
        except OSError:
            pass


def _now_iso():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


def _load_json(path, default):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def _write_json_atomic(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = "%s.tmp.%d" % (path, os.getpid())
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def ledger_append(state_dir, entry):
    """O_APPEND 단일 write 원자 append(동시 방출 안전). schema_version 자동 부착.
    크기 임계(5MB) 도달 시 ledger.jsonl.1로 1세대 로테이션(무한 성장 차단)."""
    entry.setdefault("schema_version", SCHEMA_VERSION)
    path = os.path.join(state_dir, "ledger.jsonl")
    os.makedirs(state_dir, exist_ok=True)
    try:
        if os.path.getsize(path) >= LEDGER_MAX_BYTES:
            os.replace(path, path + ".1")   # 원자적 로테이션(기존 .1 덮어씀 = 1세대 보관)
    except OSError:
        pass                                # 부재·경합은 무해 — 이번 append로 새 파일 생성
    line = (json.dumps(entry, ensure_ascii=False) + "\n").encode("utf-8")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        os.write(fd, line)
    finally:
        os.close(fd)


def last_ledger(state_dir):
    """마지막 대장 항목 — seek 기반 tail 읽기(전체 readlines 금지·대용량 내성). 파일 끝에서
    최대 64KB만 읽어 마지막 파싱 가능한 줄을 반환한다(한 항목은 <1KB라 충분)."""
    path = os.path.join(state_dir, "ledger.jsonl")
    try:
        with open(path, "rb") as f:
            f.seek(0, os.SEEK_END)
            size = f.tell()
            f.seek(max(0, size - 65536))
            tail = f.read()
    except OSError:
        return None
    for line in reversed(tail.split(b"\n")):
        line = line.strip()
        if line:
            try:
                return json.loads(line.decode("utf-8", "replace"))
            except ValueError:
                continue
    return None


# ─────────────────────────── 정규화·diff ───────────────────────────

def normalize(obj):
    """블랙리스트 키 재귀 제거 + 리스트 결정론 정렬(순서 비결정 항목 안정화)."""
    if isinstance(obj, dict):
        return {k: normalize(v) for k, v in obj.items() if k not in BLACKLIST_KEYS}
    if isinstance(obj, list):
        items = [normalize(v) for v in obj]
        try:
            return sorted(items, key=lambda x: json.dumps(x, ensure_ascii=False, sort_keys=True))
        except TypeError:
            return items
    return obj


def diff_top_fields(old_snap, new_snap):
    """정규화 스냅샷의 최상위 변화 필드명 목록(결정론 정렬)."""
    if not isinstance(old_snap, dict) or not isinstance(new_snap, dict):
        return ["<snapshot>"] if old_snap != new_snap else []
    changed = []
    for k in sorted(set(old_snap) | set(new_snap)):
        if old_snap.get(k) != new_snap.get(k):
            changed.append(k)
    return changed


def node_changes(old_snap, new_snap):
    """(node_label, new_node_dict) 목록 — 진행이 바뀐 노드(task_progress payload 원천)."""
    old_nodes = {n.get("node"): n for n in (old_snap.get("nodes") or [])}
    new_nodes = {n.get("node"): n for n in (new_snap.get("nodes") or [])}
    out = []
    for name in sorted(set(old_nodes) | set(new_nodes)):
        if old_nodes.get(name) != new_nodes.get(name):
            out.append((name, new_nodes.get(name)))
    return out


# ─────────────────────────── report 해석 ───────────────────────────

def node_is_idle(report, node_label):
    """담당 노드가 데몬 실측 idle인가. True/False/None(정보 없음)."""
    for ln in report.get("live_nodes") or []:
        if (ln.get("role") or "").lower() == node_label:
            idle_secs = ln.get("idle_secs")
            if isinstance(idle_secs, int):
                return idle_secs >= IDLE_ALERT_SECS
            return None
    return None


def in_progress_tasks(report):
    return [n for n in (report.get("nodes") or [])
            if n.get("total", 0) > 0 and n.get("done", 0) < n.get("total", 0)]


def all_nodes_idle(report):
    """전 활성 노드가 idle인가. 정보 없음(활성 노드 0·status 미수집)이면 False(QUIET 단정 불가=보수적)."""
    alive = [n for n in (report.get("live_nodes") or []) if n.get("agent_alive")]
    if not alive:
        return False
    for n in alive:
        idle_secs = n.get("idle_secs")
        if not (isinstance(idle_secs, int) and idle_secs >= IDLE_ALERT_SECS):
            return False
    return True


# ── EVT payload 매핑 표(계약 SOT: _round/EVENT_CONTRACT.md · javis_event.SCHEMA) ──
#   idle    → agent.silent {agent, silent_minutes, level=critical}
#   feed    → approval.needed {agent, task, summary}
#   stall   → agent.silent {agent, silent_minutes, level=critical}
#   context → EVT 매핑 없음(계약에 ctx 타입 부재) → wake 전용
#   collect → EVT 매핑 없음 → wake 전용
#   DELTA   → task_progress {task, stage, [pct]}
#   날짜변경 → briefing {counts:{running,inbox,approvals,alerts}}
def extract_warnings(report):
    """원문 report에서 WARN 트리거 목록 추출. 각 항목: trigger/reason/wake_body/(evt_type,evt_fields)/idem."""
    warns = []
    proc = ("read-screen 확인·재지시.")
    tail = " 기상절차: cys status --json 1콜 점검 병행."

    # idle WARN은 pending-todo(진행 중 할 일이 남은) 노드 + todo 미상 노드에만 발화한다.
    # ★master 승인 2026-07-18(동작 현행 유지 — 리뷰어 노이즈 재유입 방지):
    #   - C4 간접 커버 주장은 **pending-todo 노드의 승인 프롬프트 대기에 한정**한다. done 노드
    #     (진행 완료·total==0 포함)는 억제되고, 무배정 노드(todo 없음)는 noisy-default로 발화하나
    #     프롬프트↔idle 매핑이 보장되지 않는다 → **done/무배정 노드의 프롬프트는 미커버**(직접
    #     감지기는 후속 과제). done 노드 idle 억제가 QUIET 도달성·quiet-park 신호를 살린다.
    #   - ※리뷰어 상시 idle 노이즈는 현행 유지(완화 금지·master 지시), 임계 튜닝은 shadow(P1) 소관.
    #   - ⚠P2-info(취약 결합): nodes[].node(=node_label, role 소문자)와 idle_nodes[].role을
    #     문자열 라벨공간으로 조인한다 — role 명명 규칙이 바뀌면 이 조인이 조용히 깨진다.
    pending = {n.get("node") for n in (report.get("nodes") or [])
               if n.get("total", 0) > 0 and n.get("done", 0) < n.get("total", 0)}
    todo_labels = {n.get("node") for n in (report.get("nodes") or [])}
    idle = [n for n in (report.get("idle_nodes") or [])
            if (n.get("role") or "").lower() in pending
            or (n.get("role") or "").lower() not in todo_labels]
    # ★master 승인 2026-07-18: idle 사유별로 task/idempotency-key를 노드 단위로 분리한다
    #   (예: gate-idle-reviewer-codex). 지속되는 동일 노드 idle이 항상 같은 pending에 병합·억제되어
    #   큐 병합이 최대화된다 — 다른 노드 idle 변동에 키가 흔들리지 않는다(집계 단일 키의 약점 제거).
    for n in idle:
        role = n.get("role", "unknown")
        mins = (n.get("idle_secs") or 0) // 60
        warns.append({
            "trigger": "idle",
            "task": "gate-idle-%s" % role,
            "reason": "idle_5min:%s" % role,
            "wake_body": "[gate] idle: %s idle 5분+ — %s%s" % (role, proc, tail),
            "evt_type": "agent.silent",
            "evt_fields": {"agent": role, "silent_minutes": int(mins), "level": "critical"},
            "idem": "gate-idle-%s" % role,
        })
    high = [n for n in (report.get("live_nodes") or [])
            if isinstance(n.get("context_pct"), int) and n["context_pct"] >= 60]
    if high:
        roles = ",".join("%s(%d%%)" % (n.get("role", "?"), n["context_pct"]) for n in high)
        warns.append({
            "trigger": "context",
            "task": "gate-context",
            "reason": "ctx_60:%s" % roles,
            "wake_body": "[gate] context: %s 컨텍스트 60%%+ — cycle-agent 집행 검토.%s" % (roles, tail),
            "evt_type": None, "evt_fields": None,
            "idem": "gate-context-%s" % ",".join(n.get("role", "?") for n in high),
        })
    feed = report.get("feed_pending")
    if isinstance(feed, int) and feed > 0:
        warns.append({
            "trigger": "feed",
            "task": "gate-feed",
            "reason": "feed_pending:%d" % feed,
            "wake_body": "[gate] feed: 승인 대기 %d건 — 즉결 필요.%s" % (feed, tail),
            "evt_type": "approval.needed",
            "evt_fields": {"agent": "master", "task": "feed-approval",
                           "summary": "%d건 대기" % feed},
            "idem": "gate-feed",
        })
    return warns


def build_stall_warnings(counters, report, cycle_minutes, stall_cycles, now_iso):
    """태스크(노드) 단위 stall 카운터 갱신 + 승격 대상 WARN 생성.

    전역 diff 카운터 금지 — 다른 태스크의 변화가 특정 태스크의 정체를 은폐한다(적대 검증 A6).
    승격 조건: 진행 시그니처 무변화 ≥stall_cycles **AND 담당 노드 idle**(데몬 실측). 노드 busy면
    정상 장기 라운드(리뷰 40분+ 등)이므로 카운터만 증가·승격 보류(오탐 억제 S1-2).
    """
    prev = counters.get("nodes", {}) or {}
    new_nodes = {}
    stalls = []
    tail = " 기상절차: cys status --json 1콜 점검 병행."
    for n in report.get("nodes") or []:
        label = n.get("node")
        sig = "%s/%s" % (n.get("done"), n.get("total"))
        pc = prev.get(label)
        if pc and pc.get("sig") == sig:
            count = pc.get("count", 0) + 1
            last_change = pc.get("last_change_ts", now_iso)
        else:
            count = 0                 # 진행 변화 시에만 리셋(해당 태스크 기준)
            last_change = now_iso
        new_nodes[label] = {"sig": sig, "count": count, "last_change_ts": last_change}

        in_progress = n.get("total", 0) > 0 and n.get("done", 0) < n.get("total", 0)
        if in_progress and count >= stall_cycles:
            idle = node_is_idle(report, label)
            if idle is False:
                continue              # 노드 busy → 승격 보류(카운터는 이미 증가)
            # idle True 또는 None(미지=보수적 시끄러운 쪽으로 승격) → stall WARN
            mins = count * cycle_minutes
            stalls.append({
                "trigger": "stall",
                "task": "gate-stall-%s" % label,
                "reason": "stall:%s(%d주기·%s)" % (label, count, "idle" if idle else "미지"),
                "wake_body": "[gate] stall: %s %d주기(%d분) 무진행·노드 %s — 워커 점검·재지시.%s"
                             % (label, count, mins, "idle" if idle else "생존미상", tail),
                "evt_type": "agent.silent",
                "evt_fields": {"agent": label, "silent_minutes": int(mins), "level": "critical"},
                "idem": "gate-stall-%s" % label,
            })
    counters["nodes"] = new_nodes
    return stalls


# ─────────────────────────── 외부 명령 Runner(주입 가능) ───────────────────────────

class Runner:
    """실 subprocess 러너 — 4개 외부 명령을 감싼다. 테스트는 동일 메서드의 대역을 주입한다.

    wakeup 큐 루트(JAVIS_ROOT)는 state_dir로 고정한다 — launchd cwd=/ 오염 사고 계열 방어
    (memory). enqueue/drain이 같은 루트를 공유하므로 self-consistent(배달은 drain이 cys send로 수행).
    """

    def __init__(self, pack_bin, state_dir, timeout=30):
        self.pack_bin = pack_bin
        self.timeout = timeout
        self.cys_bin = resolve_cys_bin()   # launchd 최소 env(PATH에 cys 부재)에서도 절대 해석
        # CYS_SOCKET 부재 = 기본 소켓(본사 데몬 기준 정상). 별도 설정 없이 그대로 상속.
        self.wk_env = dict(os.environ, JAVIS_ROOT=state_dir)

    def collect_report(self):
        """(ok, report_dict|None, err|None) — javis_report.py --json subprocess."""
        script = os.path.join(self.pack_bin, "javis_report.py")
        try:
            r = subprocess.run([sys.executable, script, "--json"],
                               capture_output=True, text=True, timeout=self.timeout)
        except (subprocess.SubprocessError, OSError) as e:
            return False, None, "수집 실행 실패: %s" % e
        if r.returncode != 0:
            return False, None, "javis_report exit=%d: %s" % (r.returncode, (r.stderr or "")[-160:])
        try:
            return True, json.loads(r.stdout), None
        except ValueError as e:
            return False, None, "JSON 파싱 실패: %s" % e

    def emit(self, evt_type, fields, surface="auto"):
        """(exit_code, stdout, stderr). exit 6=deny-by-default 거부(필수 키 부재 등)."""
        script = os.path.join(self.pack_bin, "javis_event.py")
        argv = [sys.executable, script, "emit", evt_type]
        for k, v in fields.items():
            val = v if isinstance(v, str) else json.dumps(v, ensure_ascii=False)
            argv += ["--field", "%s=%s" % (k, val)]
        argv += ["--spool", "--surface", surface]
        try:
            r = subprocess.run(argv, capture_output=True, text=True, timeout=self.timeout)
            return r.returncode, r.stdout, r.stderr
        except (subprocess.SubprocessError, OSError) as e:
            return 1, "", str(e)

    def enqueue(self, to, task, reason, idem, payload=None):
        script = os.path.join(self.pack_bin, "javis_wakeup.py")
        argv = [sys.executable, script, "enqueue", "--to", to, "--task", task, "--reason", reason]
        if idem:
            argv += ["--idempotency-key", idem]
        if payload:
            argv += ["--payload", json.dumps(payload, ensure_ascii=False)]
        try:
            r = subprocess.run(argv, capture_output=True, text=True,
                               timeout=self.timeout, env=self.wk_env)
            return r.returncode
        except (subprocess.SubprocessError, OSError):
            return 1

    def drain(self, target):
        """enqueue만으로는 배달 안 됨 — 같은 실행에서 drain --deliver까지 수행(치명 미완결 차단).
        (exit_code, delivered_count)."""
        script = os.path.join(self.pack_bin, "javis_wakeup.py")
        argv = [sys.executable, script, "drain", "--deliver"]
        if target:
            argv += ["--target", target]
        try:
            r = subprocess.run(argv, capture_output=True, text=True,
                               timeout=self.timeout, env=self.wk_env)
        except (subprocess.SubprocessError, OSError):
            return 1, 0
        delivered = 0
        for line in (r.stdout or "").splitlines():
            line = line.strip()
            if line.startswith("{"):
                try:
                    delivered = json.loads(line).get("delivered", 0)
                except ValueError:
                    pass
        return r.returncode, delivered

    def send_queued(self, to, body):
        """미등록 role은 즉시 exit 1 거부(큐 적재 없음 — P0 확정). 실패는 호출부가 조용히 종결."""
        try:
            r = subprocess.run([self.cys_bin, "send", "--queued", "--to", to, body],
                               capture_output=True, text=True, timeout=self.timeout)
            return r.returncode
        except (subprocess.SubprocessError, OSError):
            return 1


# ─────────────────────────── 게이트 코어 ───────────────────────────

class Gate:
    def __init__(self, state_dir, runner, cycle_minutes=CYCLE_MINUTES_DEFAULT,
                 stall_cycles=STALL_CYCLES_DEFAULT, quiet_cycles=QUIET_CYCLES_DEFAULT,
                 now_epoch_fn=time.time, now_iso_fn=_now_iso):
        self.state_dir = state_dir
        self.runner = runner
        self.cycle_minutes = cycle_minutes
        self.stall_cycles = stall_cycles
        self.quiet_cycles = quiet_cycles
        self.now_epoch_fn = now_epoch_fn
        self.now_iso_fn = now_iso_fn
        self.snap_path = os.path.join(state_dir, "last_snapshot.json")
        self.counters_path = os.path.join(state_dir, "counters.json")

    # ── 최종 stdout 1줄: schedule.command_done 텔레메트리에 실린다(데드맨 1차 강화·P0 확정) ──
    def _summary(self, verdict, delivered, reasons):
        print("verdict=%s delivered=%s reasons=%s"
              % (verdict, delivered, ",".join(reasons) if reasons else "-"))

    # ── 스냅샷은 래퍼로 저장(schema_version·S2-4) — 상수 키가 diff 본문에 섞여 오탐 DELTA를
    #    내지 않도록 {"schema_version":1,"data":<정규화 스냅샷>}로 감싼다. 로드는 구 포맷 하위호환. ──
    def _load_snapshot(self):
        raw = _load_json(self.snap_path, None)
        if isinstance(raw, dict) and "schema_version" in raw and "data" in raw:
            return raw["data"]
        return raw

    def _write_snapshot(self, snap):
        _write_json_atomic(self.snap_path, {"schema_version": SCHEMA_VERSION, "data": snap})

    def _write_counters(self, counters):
        counters["schema_version"] = SCHEMA_VERSION     # S2-4: 상태 파일 스키마 버전(추가-전용 마이그레이션)
        _write_json_atomic(self.counters_path, counters)

    def run(self, shadow=False):
        # ★외부 데몬 가드(핫픽스·락 획득 전): socket-pack 부정합(부서 데몬이 본사 팩 로드)이면
        #   대장에 SKIPPED_FOREIGN_DAEMON 1줄만 기록하고 즉시 exit 0 — 카운터·배달·stall 무접촉.
        foreign = foreign_daemon_verdict()
        if foreign is not None:
            verdict, reason = foreign
            try:
                ledger_append(self.state_dir, {"ts": self.now_iso_fn(),
                                               "ts_epoch": self.now_epoch_fn(),
                                               "verdict": verdict, "reasons": [reason],
                                               "delta_fields": [], "delivered": "none"})
            except OSError:
                pass
            self._summary(verdict, "none", [reason])
            return 0
        # ★최상위 fail-open(P1): 락 획득·state_dir 접근의 OSError(PermissionError/ENOSPC/EROFS
        #   포함)가 exit 1로 죽는 경로를 봉쇄한다 — 대장 기록이 불가한 상황이라도 최소한 master에
        #   직송을 시도하고 exit 0으로 종료한다(schedule.error만으론 아무도 안 깨기 때문).
        try:
            os.makedirs(self.state_dir, exist_ok=True)
            lock_path = os.path.join(self.state_dir, "lock")
            try:
                with _FileLock(lock_path):
                    return self._run_locked(shadow)
            except TimeoutError:
                # 단일 비행 위반(다중 데몬·겹침) — 카운터 이중 증가 차단, 기록 후 조용히 종료.
                ledger_append(self.state_dir, {"ts": self.now_iso_fn(),
                                               "ts_epoch": self.now_epoch_fn(),
                                               "verdict": "SKIPPED_CONCURRENT",
                                               "reasons": ["lock_held"], "delta_fields": [],
                                               "delivered": "none"})
                self._summary("SKIPPED_CONCURRENT", "none", ["lock_held"])
                return 0
        except Exception as e:                          # noqa: BLE001 — state/락 접근 실패 최상위 fail-open
            return self._fail_open_no_state(e)

    def _fail_open_no_state(self, exc):
        """state_dir/락 접근 불가(대장·카운터 기록 불능) → master 직송만 시도 + exit 0.
        대장에 남길 수 없으므로 직송 본문에 사유를 명시한다(reviewer1 P1)."""
        body = ("[gate] state 기록 불가(%s) — 대장/카운터 기록 불능 fail-open. "
                "게이트 state_dir 권한·용량(Permission/ENOSPC/EROFS) 점검 필요." % exc)
        try:
            self.runner.send_queued("master", body)
        except Exception:                               # noqa: BLE001 — 직송 실패도 삼키고 exit 0(차기 재판정 복원)
            pass
        self._summary("FAILOPEN", "send", ["state_unwritable:%s" % (str(exc)[:60])])
        return 0

    def _run_locked(self, shadow):
        counters = _load_json(self.counters_path, {})
        report = None
        try:
            report = self._judge_and_route(shadow, counters)
            counters["failopen_streak"] = 0
            self._write_counters(counters)
            return 0
        except Exception as e:                                   # noqa: BLE001 (최상위 fail-open)
            return self._fail_open(e, report, counters)

    def _fail_open(self, exc, report, counters):
        """게이트 내부 오류 → 원문 보고 직송 + FAILOPEN 기록 + exit 0(안전 방향=시끄러움)."""
        streak = counters.get("failopen_streak", 0) + 1
        counters["failopen_streak"] = streak
        try:
            self._write_counters(counters)
        except OSError:
            pass
        detail = json.dumps(report, ensure_ascii=False) if report else "수집 전 오류"
        body = ("[gate] 게이트 내부 오류 fail-open — 원문 보고 직송:\n%s\n예외: %s"
                % (detail, exc))
        if streak >= 3:
            body += "\n[gate] 게이트 자체 수리 필요(연속 실패 %d회)" % streak
        rc = self.runner.send_queued("master", body)
        delivered = "send" if rc == 0 else "send_failed"
        try:
            ledger_append(self.state_dir, {"ts": self.now_iso_fn(),
                                           "ts_epoch": self.now_epoch_fn(),
                                           "verdict": "FAILOPEN",
                                           "reasons": [str(exc)[:200]], "delta_fields": [],
                                           "delivered": delivered, "failopen_streak": streak})
        except OSError:
            pass
        self._summary("FAILOPEN", delivered, [str(exc)[:80]])
        return 0

    def _judge_and_route(self, shadow, counters):
        now_epoch = self.now_epoch_fn()
        now_iso = self.now_iso_fn()

        ok, report, err = self.runner.collect_report()
        reasons = []

        # 수집 실패 = 그 자체가 WARN(경로). 스냅샷/카운터는 건드리지 않고 wake만.
        if not ok:
            warns = [{
                "trigger": "collect", "task": "gate-collect", "reason": "collect_failed:%s" % err,
                "wake_body": "[gate] collect: javis_report 수집 실패(%s) — 게이트/데몬 점검. "
                             "기상절차: cys status --json 1콜 점검 병행." % err,
                "evt_type": None, "evt_fields": None, "idem": "gate-collect",
            }]
            delivered = self._route_warn(warns, shadow, reasons)
            ledger_append(self.state_dir, {"ts": now_iso, "ts_epoch": now_epoch,
                                           "verdict": VERDICT_WARN, "reasons": [w["reason"] for w in warns],
                                           "delta_fields": [], "delivered": delivered,
                                           "consecutive_nochg": counters.get("consecutive_nochg", 0),
                                           "consecutive_quiet": counters.get("consecutive_quiet", 0),
                                           "shadow": shadow})
            self._summary(VERDICT_WARN, delivered, [w["reason"] for w in warns])
            return report

        new_snap = normalize(report)
        old_snap = self._load_snapshot()

        # ── BASELINE: 스냅샷 부재(최초 실행·재설치) → 기록만, 배달 없음(DELTA 폭주 차단) ──
        if old_snap is None:
            self._write_snapshot(new_snap)
            counters.setdefault("consecutive_nochg", 0)
            counters.setdefault("consecutive_quiet", 0)
            ledger_append(self.state_dir, {"ts": now_iso, "ts_epoch": now_epoch,
                                           "verdict": "BASELINE", "reasons": [], "delta_fields": [],
                                           "delivered": "none", "shadow": shadow})
            self._summary("BASELINE", "none", [])
            return report

        # ── GAP: 직전 대장과 간격 >3주기 → re-baseline + 기록, wake 금지(슬립·재부팅 위양성) ──
        last = last_ledger(self.state_dir)
        if last is not None:
            last_epoch = last.get("ts_epoch")
            if last_epoch is None:
                last_epoch = _parse_iso_epoch(last.get("ts"))
            if isinstance(last_epoch, (int, float)) and \
                    (now_epoch - last_epoch) > GAP_CYCLES * self.cycle_minutes * 60:
                self._write_snapshot(new_snap)
                counters["nodes"] = {}                          # 연속성 상실 → stall 카운터 리셋
                counters["consecutive_nochg"] = 0
                counters["consecutive_quiet"] = 0
                self._write_counters(counters)
                ledger_append(self.state_dir, {"ts": now_iso, "ts_epoch": now_epoch,
                                               "verdict": "GAP", "reasons": ["interval>3cycles"],
                                               "delta_fields": [], "delivered": "none",
                                               "shadow": shadow})
                self._summary("GAP", "none", ["interval>3cycles"])
                return report

        if not report.get("status_available"):
            reasons.append("status_unavailable")           # 관측용(daemon 일시 부재)·wake 안 함

        # ── 분류 ──
        warns = extract_warnings(report)
        warns += build_stall_warnings(counters, report, self.cycle_minutes,
                                      self.stall_cycles, now_iso)
        delta_fields = diff_top_fields(old_snap, new_snap)

        if warns:
            verdict = VERDICT_WARN
        elif delta_fields:
            verdict = VERDICT_DELTA
        elif (not in_progress_tasks(report)) and all_nodes_idle(report):
            # ★master 승인 2026-07-18: DESIGN QUIET 조건의 "미해결 게이트 0"은 report 스키마에
            #   소스 필드가 없어 생략한다 — 보수적 방향(주차 덜 발동)이라 안전하다(승인됨). 필요 시
            #   report에 미해결 게이트 카운트를 추가하고 여기 AND 조건으로 편입한다.
            verdict = VERDICT_QUIET
        else:
            verdict = VERDICT_NOCHG

        # ── 연속 카운터 갱신 ──
        if verdict in (VERDICT_WARN, VERDICT_DELTA):
            counters["consecutive_nochg"] = 0
            counters["consecutive_quiet"] = 0
        elif verdict == VERDICT_NOCHG:
            counters["consecutive_nochg"] = counters.get("consecutive_nochg", 0) + 1
            counters["consecutive_quiet"] = 0
        else:  # QUIET
            counters["consecutive_quiet"] = counters.get("consecutive_quiet", 0) + 1
            counters["consecutive_nochg"] = 0

        # ── 라우팅 ──
        delivered = "none"
        if verdict == VERDICT_WARN:
            delivered = self._route_warn(warns, shadow, reasons)
        elif verdict == VERDICT_DELTA:
            delivered = self._route_delta(old_snap, new_snap, shadow, reasons)
        # QUIET/NOCHG → 대장 기록만

        # QUIET 연속 임계 도달 → 세션 주차 후보(P2 반자율·CSO 집행): enqueue만, 배달은 CSO 소관
        if verdict == VERDICT_QUIET and not shadow and \
                counters["consecutive_quiet"] >= self.quiet_cycles:
            self.runner.enqueue("cso", "master-park",
                                "QUIET %d주기(%d분) 지속 — 세션 주차 후보(cycle-agent 집행 검토)"
                                % (counters["consecutive_quiet"],
                                   counters["consecutive_quiet"] * self.cycle_minutes),
                                "gate-park")
            reasons.append("park_candidate")

        # 날짜 변경 → 일 1회 briefing 백스톱(빌트인 fleet-digest 수정 불가 F3 → 게이트가 소유)
        if not shadow and last is not None:
            self._maybe_briefing(last, now_epoch, report, warns, reasons)

        self._write_snapshot(new_snap)
        ledger_append(self.state_dir, {"ts": now_iso, "ts_epoch": now_epoch, "verdict": verdict,
                                       "reasons": reasons + [w["reason"] for w in warns],
                                       "delta_fields": delta_fields, "delivered": delivered,
                                       "consecutive_nochg": counters["consecutive_nochg"],
                                       "consecutive_quiet": counters["consecutive_quiet"],
                                       "shadow": shadow})
        self._summary(verdict, delivered, reasons + [w["reason"] for w in warns])
        return report

    def _route_warn(self, warns, shadow, reasons):
        """WARN → enqueue master wake 직후 같은 실행에서 drain --deliver(치명 미완결 차단)+병행 EVT."""
        if shadow:
            return "none"
        wake_any = False
        for w in warns:
            # task_key는 노드/사유 단위(w["task"]) — 지속 조건이 같은 pending에 병합·억제되어
            # 큐 병합 최대화(master 승인 2026-07-18). 누락 시 트리거 단위로 폴백.
            self.runner.enqueue("master", w.get("task", "gate-" + w["trigger"]),
                                w["wake_body"], w["idem"])
            wake_any = True
            if w.get("evt_type"):
                erc, _, _ = self.runner.emit(w["evt_type"], w["evt_fields"])
                if erc != 0:
                    reasons.append("evt_reject:%s(%d)" % (w["evt_type"], erc))
        if wake_any:
            _, delivered = self.runner.drain("master")
            # 배달 실패(zombie 등)여도 wake 시도 완결 — 조건 지속 시 다음 주기 재-enqueue(자가 복원).
            return "wake" if delivered > 0 else "wake_pending"
        return "none"

    def _route_delta(self, old_snap, new_snap, shadow, reasons):
        """DELTA → task_progress EVT(LLM 0). emit 거부는 대장 기록만(WARN급 아님 → 폴백 wake 안 함)."""
        if shadow:
            return "none"
        emitted = False
        changes = node_changes(old_snap, new_snap)
        if changes:
            for label, node in changes:
                if node is None:
                    continue
                fields = {"task": label or "unknown", "stage": "progress"}
                if isinstance(node.get("pct"), int):
                    fields["pct"] = node["pct"]
                erc, _, _ = self.runner.emit("task_progress", fields)
                if erc == 0:
                    emitted = True
                else:
                    reasons.append("evt_reject:task_progress(%d)" % erc)
        else:
            erc, _, _ = self.runner.emit("task_progress", {"task": "fleet", "stage": "update"})
            if erc == 0:
                emitted = True
            else:
                reasons.append("evt_reject:task_progress(%d)" % erc)
        return "evt" if emitted else "none"

    def _maybe_briefing(self, last, now_epoch, report, warns, reasons):
        last_epoch = last.get("ts_epoch") or _parse_iso_epoch(last.get("ts"))
        if not isinstance(last_epoch, (int, float)):
            return
        if _epoch_date(last_epoch) == _epoch_date(now_epoch):
            return
        alive = sum(1 for n in (report.get("live_nodes") or []) if n.get("agent_alive"))
        counts = {"running": alive, "inbox": len(in_progress_tasks(report)),
                  "approvals": report.get("feed_pending") or 0, "alerts": len(warns)}
        erc, _, _ = self.runner.emit("briefing", {"counts": counts})
        reasons.append("briefing" if erc == 0 else "briefing_reject(%d)" % erc)


def _parse_iso_epoch(s):
    if not s:
        return None
    for fmt in ("%Y-%m-%dT%H:%M:%S%z", "%Y-%m-%dT%H:%M:%S"):
        try:
            return time.mktime(time.strptime(s, fmt))
        except (ValueError, OverflowError):
            continue
    return None


def _epoch_date(epoch):
    return time.strftime("%Y-%m-%d", time.localtime(epoch))


# ─────────────────────────── status ───────────────────────────

def cmd_status(state_dir, n=20):
    path = os.path.join(state_dir, "ledger.jsonl")
    try:
        with open(path, encoding="utf-8") as f:
            lines = [l.strip() for l in f if l.strip()]
    except OSError:
        print("대장 없음: %s" % path)
        return 0
    print("게이트 대장 tail (%s):" % path)
    for line in lines[-n:]:
        try:
            e = json.loads(line)
        except ValueError:
            continue
        print("  %s  %-16s delivered=%-12s reasons=%s"
              % (e.get("ts", "?"), e.get("verdict", "?"), e.get("delivered", "?"),
                 ",".join(e.get("reasons", [])) or "-"))
    return 0


def main(argv=None):
    p = argparse.ArgumentParser(description="하트비트 델타게이트 (무의미 wake 제거·DESIGN §C1)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("run", help="판정+배달+대장(기본)")
    c.add_argument("--shadow", action="store_true", help="판정·대장 기록만·배달 0(P1 검증)")
    c.add_argument("--state-dir", default=None)

    c = sub.add_parser("status", help="대장 tail 사람 출력")
    c.add_argument("--state-dir", default=None)
    c.add_argument("-n", type=int, default=20)

    a = p.parse_args(argv)
    state_dir = getattr(a, "state_dir", None) or default_state_dir()

    if a.cmd == "status":
        return cmd_status(state_dir, a.n)
    runner = Runner(default_pack_bin(), state_dir)
    return Gate(state_dir, runner).run(shadow=a.shadow)


if __name__ == "__main__":
    sys.exit(main())
