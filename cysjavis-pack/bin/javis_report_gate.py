#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_report_gate.py — 하트비트 델타게이트 (무의미 wake 제거 · DESIGN §C1 구현)

5분 시계 잡의 역할을 "발화하라"→"발화 자격을 판정하라"로 전환한다. cysd가
`action:"command"`로 이 스크립트를 5분마다 호출하면:

  ① javis_report.py --json 수집(기존 결정론 자산 재사용)
  ② 정규화(노이즈 필드 블랙리스트 제거) → 직전 스냅샷과 diff
  ③ 분류(우선순위): WARN > DELTA > QUIET > NOCHG
  ④ 라우팅: **채널 정책표**가 정한다(층2) — ledger / evt / badge / push
  ⑤ 매 판정을 대장(ledger.jsonl)에 append — 기록 두절 자체가 데드맨 경보

═══ W5 (T-0147-2 wakeup 홍수 해소 · 설계 FINAL v4.1) ═══════════════════════════
종전 게이트는 모든 WARN을 master stdin push로 보냈다. 실측 결과 그 push가 master를
잠식했고(§0-2 병합 무동작·§0-3 라벨공간 교차조인 실패로 인한 오발화), 자기강화 루프
(stdin 주입→idle 리셋→QUIET 불가)까지 만들었다. W5는 네 층으로 그것을 끊는다.

  층1  master-stdin 주입자 전수 인벤토리(I1~I7) — 이 파일은 I1·I2 소유자다.
  층2  채널 라우팅 정책(데이터가 정한다): warn 객체에 `severity`·`channels`를 부착하고
       `_route_warn`은 **필드만 집행**한다(if 체인 금지). push는 예외적 승격이다.
         · gate-idle-*  = ledger+evt+badge (push 금지)
         · gate-stall-* = ledger+evt+badge, **fail-closed 확증**(§2-C 2차 증거 3종)에서만 push
         · gate-context/gate-feed = push 금지
         · 시스템 데드락(P3)·노드 사망 = push(엣지+쿨다운·디바운스)
  층3  발화 정밀화: B4′ alias resolver(라벨→live role 접두 해소) · A3′ 엣지+쿨다운 +
       전달 상태머신(warn=at-most-once / critical=at-least-once, queue.delivered 영수증) ·
       A6′ seen-store(1키 1파일·O_EXCL·TTL·GC) · B7 레인 분리 + foreign-daemon 가드.
  층4  판정 근거 단일화: `javis_report`의 role별 권위 레코드(`role_measurements`)만 소비하고,
       판정 소스·샘플 시각·실측값을 wake 본문·원장·`last_measurement.json`에 스탬프한다.

설계 원리(불변): 판단 0의 전달은 LLM 비경유. **다만 "시끄러움=안전"은 push 채널에서만
철회된다** — 정보는 ledger·EVT·badge로 전부 남고(대안 채널 전부 가동 중 · 설계 §0-8),
사라지는 것은 중복 stdin 주입뿐이다. 침묵은 금지다(측정 불가는 badge로 노출한다).

CLI:
  run            기본 실행(판정+배달+대장)
  run --shadow   판정·대장 기록만, 배달 0 (P1 shadow 검증용)
  status         대장 tail 사람 출력

종료 코드: 항상 0 (fail-open — schedule.error만으로는 아무도 안 깨기 때문에 죽지 않는다).
의존성: 파이썬 표준 라이브러리 + 형제 모듈(javis_lock·javis_boot_node — 부재 시 loud 폴백).
외부 명령(javis_report/event/wakeup·cys)은 Runner로 주입 가능.
"""

import argparse
import json
import os
import re
import socket
import subprocess
import sys
import time

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
#   ._pth 는 표준 경로 계산을 우회해 **스크립트 폴더를 sys.path 에 넣지 않는다**.
#   선례(append 형태): javis_wakeup.py:43-45 · javis_report.py:33-34.
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

# ── 형제 모듈 소비(재발명 금지) — 부재는 **loud 폴백**(조용한 접힘 금지 · W1a 관례) ──
#   javis_lock  : 원자 쓰기 유틸(A6′ seen-store가 소비) — 고정 .tmp 교차파손 방어가 이미 들어있다.
#   javis_boot_node : W2가 export한 공유 술어(role_family) — B4′ alias resolver가 소비한다.
#     ★재발명 금지가 계약이다. 가족 판정을 여기서 다시 쓰면 B10·G26 계열(언어·소비처마다
#       가족 판정이 갈리는 결함)이 그대로 재발한다.
_LOCK_IMPORT_ERR = None
_BOOTNODE_IMPORT_ERR = None
try:
    import javis_lock as _lock
except Exception as _e:                       # noqa: BLE001 — 게이트는 죽지 않는다(사유는 대장에)
    _lock, _LOCK_IMPORT_ERR = None, str(_e)[:120]
try:
    import javis_boot_node as _bn
except Exception as _e:                       # noqa: BLE001
    _bn, _BOOTNODE_IMPORT_ERR = None, str(_e)[:120]

# javis_report.py IDLE_ALERT_SECS와 동일(절대지침 B3: idle 5분+). 자기보고가 아닌 데몬 실측
# idle_secs로만 판정한다(memory: stale self-report 함정). 여기 재정의(수집 실패 시에도 상수 필요).
IDLE_ALERT_SECS = 300
CYCLE_MINUTES_DEFAULT = 5      # schedule every_minutes=5
STALL_CYCLES_DEFAULT = 6       # 6주기=30분 무진행 → stall 승격(DESIGN 미결 기본값)
QUIET_CYCLES_DEFAULT = 12      # 12주기=60분 QUIET → 세션 주차 후보(P2·CSO 집행)
GAP_CYCLES = 3                 # 직전 대장과 간격 >3주기 = GAP(슬립·재부팅 복귀 위양성 강등)
SCHEMA_VERSION = 1
STALL_COOLDOWN_SECS = 3600     # stall 재발화 쿨다운(1h·12주기) — 2026-07-26 무한 발화 결함 수정
LEDGER_MAX_BYTES = 5 * 1024 * 1024   # 대장 5MB 도달 시 ledger.jsonl.1로 1세대 로테이션

# ── W5 상수 ──────────────────────────────────────────────────────────────────
# A3′ 엣지+쿨다운(설계 §2 층3 · 출처 idle-standby-v5 §3.2). 진동 노드의 발화 상한이다.
EDGE_COOLDOWN_SECS = 7200            # idle 엣지 재발화 상한 = 1회/2h/role
DEADLOCK_COOLDOWN_SECS = 7200        # P3 시스템 데드락 push 쿨다운(설계 층2 표: 엣지+쿨다운 2h)
DEATH_DEBOUNCE_SECS = 300            # 노드 사망 push 디바운스(설계 층2 표: 5분)
DEADLOCK_IDLE_SECS = 1800            # P3: 미배정 티켓 미checkout·set-status age 임계(30분)
SETSTATUS_STALE_SECS = 900           # §2-C 2차 증거 ②: set-status age > 15분
SEEN_TTL_SECS = 1800                 # A6′ seen-store TTL(=critical 재enqueue 상한 = TTL당 1)
BADGE_SCHEMA_VERSION = 1             # badges.json — 데몬 alerts.rs `node_liveness` 가 소비
EVENT_POLL_TIMEOUT = 1.5             # queue.delivered/master.deadman 회수 상한(초)

# 정규화 블랙리스트 — 타임스탬프·수집시각·순서 비결정 항목만 제거한다. 화이트리스트 금지
# (신호 유실 단일 실패점). 미지의 새 필드는 자동으로 diff 대상 = 변화로 감지된다(fail-noisy).
# idle_secs/age_secs는 idle 노드에서 매 주기 증가하는 시간파생 노이즈라 diff에서 제외한다
# — WARN 추출은 정규화 '전' 원문에서 하므로 idle 감지 능력은 손실되지 않는다.
# ★W5 층4: `sampled_at`·`status_age_secs`·`usage_ctx_tokens` 도 시간파생이라 **여기 추가**한다.
#   판정은 정규화 '전' 원문(권위 레코드)에서 하고, 실측값의 영속 수용처는 `last_measurement.json`
#   이다 — BLACKLIST 를 푸는 것(=diff 대상화)은 매 주기 DELTA 폭주를 부르므로 금지다.
BLACKLIST_KEYS = frozenset({
    "idle_secs", "age_secs", "ts", "timestamp", "collected_at", "generated_at",
    "now", "uptime_secs", "last_seen", "seen_at", "mtime", "updated_at",
    "sampled_at", "status_age_secs", "usage_ctx_tokens",
})

VERDICT_WARN, VERDICT_DELTA, VERDICT_QUIET, VERDICT_NOCHG = "WARN", "DELTA", "QUIET", "NOCHG"

# ── 층2 채널 라우팅 정책표(데이터가 정한다 · if 체인 금지) ─────────────────────
SEV_INFO, SEV_WARN, SEV_CRIT = "info", "warn", "critical"
CH_LEDGER, CH_EVT, CH_BADGE, CH_PUSH = "ledger", "evt", "badge", "push"

#   trigger → (severity, channels). `_route_warn` 은 이 필드만 집행한다.
#   push 는 **예외적 승격**이고, 승격 자격은 아래 세 갈래뿐이다:
#     · stall_confirmed  §2-C fail-closed 확증(2차 증거 3종 전부 성립)
#     · deadlock         §2 층2 P3 술어(last_output 완전 배제)
#     · death            데몬 deadman 이벤트 소비(게이트 자체 중복 채널 제거)
CHANNEL_POLICY = {
    "idle":            (SEV_WARN, (CH_LEDGER, CH_EVT, CH_BADGE)),
    "stall":           (SEV_WARN, (CH_LEDGER, CH_EVT, CH_BADGE)),
    "stall_confirmed": (SEV_CRIT, (CH_LEDGER, CH_EVT, CH_BADGE, CH_PUSH)),
    "context":         (SEV_INFO, (CH_LEDGER, CH_EVT)),
    "feed":            (SEV_INFO, (CH_LEDGER, CH_EVT, CH_BADGE)),  # EVT 복원(master 검수): approval.needed는 HUD·음성 구독 토대(EVENT_CONTRACT) — 설계 표의 취지는 push 금지이지 EVT 제거가 아니다
    "collect":         (SEV_WARN, (CH_LEDGER, CH_BADGE)),
    "label_unjoined":  (SEV_WARN, (CH_LEDGER, CH_BADGE)),
    "measure":         (SEV_WARN, (CH_LEDGER, CH_BADGE)),
    "deadlock":        (SEV_CRIT, (CH_LEDGER, CH_EVT, CH_BADGE, CH_PUSH)),
    "death":           (SEV_CRIT, (CH_LEDGER, CH_EVT, CH_BADGE, CH_PUSH)),
}

# 수신 계층(설계 §2 층2): push 1차 수신자는 CSO다. "critical만 master 직송"은 **허가**이지
# 의무가 아니다 — 어느 trigger 가 master 로 직송되는지는 아래 표(데이터)가 정한다.
#   · deadlock         → CSO 1줄 push(층2 표 명문)
#   · stall_confirmed  → CSO(2026-08-01 master 재정 ①)
#   · death            → CSO(2026-08-01 master 재정 ①)
#   ★2026-08-01 재정 ① — stall_confirmed·death 의 종전 `master` 직송을 **cso** 로 되돌린다.
#     근거는 규범 "시스템·자원 사안의 1차 수신자는 CSO"(CLAUDE.md §4)의 기계 반영이다: 노드
#     사망·확증 stall 은 전형적인 시스템·자원 사안이고, 판단 주체가 master 라는 것과 **경보를
#     누가 먼저 받는가**는 다른 문제다(보고 사슬은 CSO→master 로 이어지므로 끊기지 않는다).
#     부수 효과로 death:master 의 자기참조(죽은 master 에게 자기 부고 배달 → 영구 재발화)가
#     표 층위에서 사라진다 — 다만 그것은 결과이지 이 표의 목적이 아니므로, 자기참조 차단은
#     `_push_target(avoid=...)` 에 **독립 장치**로 남긴다(death:cso 를 막는 것이 그쪽 몫이다).
#   CSO 부재 시에는 전부 master 폴백(M3: "CSO(부재 시 master)").
PUSH_TARGET = {"deadlock": "cso", "stall_confirmed": "cso", "death": "cso"}

# 전달 티어(설계 §2 층3 A3′ 상태머신):
#   warn-tier     = enqueue 성공 시 disarm (at-most-once · 유실은 badge 가 보완)
#   critical-tier = queue.delivered(진짜 Inject 영수증) 수신 후에만 disarm (at-least-once)
TIER_AT_MOST_ONCE, TIER_AT_LEAST_ONCE = "at-most-once", "at-least-once"


def tier_for(severity):
    return TIER_AT_LEAST_ONCE if severity == SEV_CRIT else TIER_AT_MOST_ONCE


# ─────────────────────────── 상태 경로·원장 ───────────────────────────

def default_state_dir():
    d = os.environ.get("CYS_REPORT_GATE_DIR")
    if d:
        return d
    return os.path.join(os.path.expanduser("~"), ".cys", "state", "report_gate")


def lane_id(state_dir):
    """레인 식별자 = state_dir basename(예: `report_gate`, `report_gate-dept-2`).
    데몬 alerts.rs 가 배지 key 접두로 쓰므로 레인 간 배지 충돌이 생기지 않는다(B7)."""
    return os.path.basename(os.path.normpath(state_dir)) or "report_gate"


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


# ── B7: 외부 데몬 가드 — socket-pack 정합 검사 ────────────────────────────────
# 출처: 커밋 4785553(release/0.12.91) `foreign_daemon_verdict` — 이 계보가 현행 라인에
# 미편입돼 **구현이 소실**돼 있었다(설계 §2 층3 "가드 복원"). 원문 의미를 보존해 재편입한다.
#
# 실측 결함: 부서 데몬(env 오염 — CYS_PACK_DIR=본사 팩 + CYS_SOCKET=dept 소켓)이 본사
# schedule.json을 로드해 command 잡을 중복 실행한다(action:command는 push와 달리 if_absent
# 게이트가 없어 모든 로더에서 실행됨). 부서 데몬 자체 수정은 ACL 금지 → 게이트 자기방어로 해결.
# 정합 규칙:
#   - 본사 팩(realpath == $HOME/.cys/pack): CYS_SOCKET unset 또는 기본 소켓이어야 정합.
#     set인데 기본 소켓이 아니면 = 외부 데몬 컨텍스트 → SKIP.
#   - 부서 팩(basename == pack-dept-<X>): CYS_SOCKET 경로에 cys-dept-<X> 포함 요구.
#   - 그 외(worktree·테스트 등): 판단 보류 → 정상 진행.
# 가드 자체 오류는 fail-open(정상 진행) — 가드가 본사 실행을 죽이면 안 된다.
#
# ★불변식(설계 §2 층3 "발행자 불변식"): 레인(=trigger-namespace)당 발행자는 하나다. 이 가드가
#   레인 간 교차를 차단하고, `CYS_REPORT_GATE_DIR` 레인별 배선이 상태를 분리하며, pack측
#   wakeup push는 레인당 javis_wakeup 큐 하나만 경유한다(I3 수렴으로 성립).
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


def resolve_socket_path():
    """데몬 unix 소켓 경로 — `cys::socket_path()`(src/lib.rs:54) 파이썬 등가.
    Windows(named pipe)는 AF_UNIX 로 열 수 없으므로 None(=이벤트 회수 불가·loud 강등)."""
    p = os.environ.get("CYS_SOCKET")
    if p:
        return os.path.expanduser(p)
    if os.name == "nt":
        return None
    return os.path.expanduser(DEFAULT_SOCKET)


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
    """원자 교체 — javis_lock.atomic_write_json 소비(재발명 금지 · 고정 .tmp 교차파손 방어).
    형제 모듈 부재 시에만 동형 폴백(사유는 대장 `lock_module` 필드로 노출된다)."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if _lock is not None:
        _lock.atomic_write_json(path, obj, indent=1, ensure_ascii=False)
        return
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


# ─────────────────────── A6′ seen-store (설계 §2 층3) ───────────────────────
# 위치=레인별 `<state_dir>/seen/`, 레코드=`{key, severity, first_ts, last_ts, state, wakeup_id}`
# 1키 1파일. 원자성=`O_CREAT|O_EXCL`(선점) → 내용은 javis_lock 원자 유틸로 채운다.
# TTL=epoch 초(시계 역행 시 TTL 재시작 — 안전 방향). GC=매 run 만료분 삭제.
# **severity 상승은 TTL 우회**: key 자체에 severity 를 포함하므로 상위 severity 는 별도 키가
# 되어 하위 seen 에 억제되지 않는다(gemini ISSUE-4 수용의 구조적 구현).
SEEN_STATE_CLAIMED, SEEN_STATE_INFLIGHT, SEEN_STATE_DELIVERED = "claimed", "inflight", "delivered"


def _safe_key(key):
    return re.sub(r"[^A-Za-z0-9._-]", "_", key)[:120]


def seen_dir(state_dir):
    return os.path.join(state_dir, "seen")


def seen_path(state_dir, key):
    return os.path.join(seen_dir(state_dir), _safe_key(key) + ".json")


def seen_key(trigger, subject, severity):
    """seen-store 키 — severity 포함이 계약이다(상위 승격이 하위 TTL을 우회하도록)."""
    return "%s:%s:%s" % (trigger, subject or "-", severity)


def seen_claim(state_dir, key, severity, now, ttl=SEEN_TTL_SECS):
    """(claimed, record) — 이번 실행이 이 키의 유일 발행자인가.

    claimed=True  : 선점 성공(=발화 자격). 레코드는 `claimed` 상태로 기록된다.
    claimed=False : 유효한 기존 레코드가 억제한다. record 로 상태·wakeup_id 를 돌려준다.

    복구 규칙(설계 C1·C3 oracle: 총 delivery 1..2 · duplicate 0..1):
      · `claimed`  = seen 선점 후 enqueue 전에 죽은 흔적. **다음 주기 즉시 재시도**(배달 0회였으므로
                     중복이 아니라 미달이다). → 총 1회로 수렴.
      · `inflight` = enqueue 성공·Inject 영수증 미수신. TTL 만료까지 억제 → 만료 후 1회 재enqueue.
                     (중복 상한 = TTL당 1)
      · `delivered`= 완결. TTL 만료까지 억제.
    """
    os.makedirs(seen_dir(state_dir), exist_ok=True)
    path = seen_path(state_dir, key)
    rec = _load_json(path, None)
    if isinstance(rec, dict):
        first = rec.get("first_ts")
        expired = not isinstance(first, (int, float)) or (now - first) >= ttl
        clock_back = isinstance(first, (int, float)) and now < first
        if clock_back:
            # 시계 역행: TTL 재시작(안전 방향 = 이번 주기는 억제). 침묵이 아니라 지연이다.
            rec["first_ts"] = now
            rec["last_ts"] = now
            _seen_write(path, rec)
            return False, rec
        if not expired and rec.get("state") != SEEN_STATE_CLAIMED:
            return False, rec
        try:
            os.unlink(path)                      # 만료 또는 crash 흔적(claimed) → 재선점 허용
        except OSError:
            pass
    try:
        fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644)
    except FileExistsError:
        return False, _load_json(path, {}) or {}   # 경합 패배 = 다른 발행자가 소유
    except OSError:
        return False, {}                            # 기록 불능 = 발화 보류(보수적)
    os.close(fd)
    rec = {"key": key, "severity": severity, "first_ts": now, "last_ts": now,
           "state": SEEN_STATE_CLAIMED, "wakeup_id": None}
    _seen_write(path, rec)
    return True, rec


def _seen_write(path, rec):
    try:
        _write_json_atomic(path, rec)
    except OSError:
        pass


def seen_mark(state_dir, key, now, **fields):
    path = seen_path(state_dir, key)
    rec = _load_json(path, None)
    if not isinstance(rec, dict):
        rec = {"key": key, "first_ts": now}
    rec.update(fields)
    rec["last_ts"] = now
    _seen_write(path, rec)
    return rec


def seen_iter(state_dir):
    d = seen_dir(state_dir)
    out = []
    try:
        names = sorted(os.listdir(d))
    except OSError:
        return out
    for name in names:
        if not name.endswith(".json"):
            continue
        rec = _load_json(os.path.join(d, name), None)
        if isinstance(rec, dict) and rec.get("key"):
            out.append(rec)
    return out


def seen_gc(state_dir, now, ttl=SEEN_TTL_SECS):
    """매 run 만료분 삭제 — 무한 성장 차단. 반환=삭제 건수."""
    removed = 0
    for rec in seen_iter(state_dir):
        first = rec.get("first_ts")
        if not isinstance(first, (int, float)) or (now - first) >= ttl:
            try:
                os.unlink(seen_path(state_dir, rec["key"]))
                removed += 1
            except OSError:
                pass
    return removed


# ─────────────── A3′ 엣지 상태(counters) — 순수 헬퍼 ───────────────
def edge_state(counters, bucket, key):
    return (counters.get(bucket) or {}).get(key) or {"armed": True, "last_fired": 0}


def edge_allows(counters, bucket, key, now, cooldown):
    """엣지 자격 AND 쿨다운 경과. 두 축은 **서로를 대체하지 않는다**:
      · armed  = 이 에피소드에서 아직 발화하지 않았다(조건 해소 시 호출부가 재무장한다)
      · 쿨다운 = 진동(해소↔재발) 노드의 발화 상한 — 재무장돼도 창 안이면 막는다
    disarmed 상태에서도 쿨다운이 **양수**이면 그 경과만으로 재발화를 허용한다(장기 지속 조건의
    주기적 재통보). 쿨다운 0인 트리거는 재무장 없이는 절대 재발화하지 않는다(엣지 순수형)."""
    st = edge_state(counters, bucket, key)
    elapsed = now - (st.get("last_fired") or 0)
    if st.get("armed", True):
        return elapsed >= cooldown
    return cooldown > 0 and elapsed >= cooldown


def edge_fire(counters, bucket, key, now):
    counters.setdefault(bucket, {})[key] = {"armed": False, "last_fired": now}


def edge_rearm(counters, bucket, key):
    """조건 해소 → 재무장. last_fired 는 **보존**한다(쿨다운이 진동 상한을 이룬다)."""
    st = counters.setdefault(bucket, {}).get(key)
    if st is None:
        return
    st["armed"] = True


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


# ─────────────── 층4: 권위 측정 레코드 (설계 §2 층4) ───────────────
# `javis_report`가 role별 `{role, idle_secs, sampled_at, source}` 단일 권위 레코드를 방출하고
# 게이트는 **그것만** 소비한다. 구버전 보고기(필드 부재)는 live_nodes 로 파생해 동형 레코드를
# 만든다 — ADR-2 스큐 안전(양측은 서로를 전제하지 않는다).

def measurements(report):
    """role → 권위 레코드 dict. 없으면 live_nodes 에서 파생(source 에 파생 사실을 남긴다)."""
    out = {}
    rms = report.get("role_measurements")
    if isinstance(rms, list) and rms:
        for m in rms:
            role = (m.get("role") or "").lower()
            if role:
                out[role] = dict(m, role=role)
        return out
    sampled = report.get("sampled_at")
    for n in report.get("live_nodes") or []:
        role = (n.get("role") or "").lower()
        if not role:
            continue
        idle = n.get("idle_secs")
        out[role] = {
            "role": role,
            "idle_secs": idle if isinstance(idle, int) else None,
            "sampled_at": sampled,
            "source": "derived.live_nodes" if isinstance(idle, int) else "unavailable",
            "agent_alive": n.get("agent_alive"),
            "status_age_secs": n.get("status_age_secs"),
            "usage_ctx_tokens": n.get("usage_ctx_tokens"),
        }
    return out


def live_role_names(report):
    """관측된 live role 집합(소문자). 라벨 조인의 유일한 모집단이다.

    ★`idle_nodes` 도 모집단에 넣는다: 산출기가 `live_nodes` 를 싣지 않은 보고(구버전·부분 수집)
      에서 모집단이 공집합이 되면 **전 라벨이 미조인**으로 접혀 idle·stall 감지가 통째로
      사라진다 — 침묵 방향의 붕괴라 가장 위험하다. idle_nodes ⊂ live_nodes 이므로 합집합은
      항상 안전하다."""
    names = set(measurements(report))
    for bucket in ("live_nodes", "idle_nodes"):
        for n in report.get(bucket) or []:
            r = (n.get("role") or "").lower()
            if r:
                names.add(r)
    return names


def role_family(role):
    """W2 공유 술어 소비(javis_boot_node.role_family). 소비 불가 시에만 동형 폴백.
    ★재발명 금지 — 가족 판정이 소비처마다 갈리면 B10·G26 계열이 재발한다."""
    if _bn is not None:
        try:
            return _bn.role_family(role)
        except Exception:                     # noqa: BLE001
            pass
    r = (role or "").strip()
    if not r:
        return None
    if r == "master":
        return "master"
    for fam in ("worker", "cso", "reviewer"):
        if r.startswith(fam):
            return fam
    return None


def resolve_label_roles(label, live_roles):
    """B4′ alias resolver — 파일 라벨 → live role 조인. `(roles, how)`.

    how ∈ `exact` | `family` | `none`.
      ① 정확일치(대소문자 무시)
      ② **접두 해소**: 라벨이 가족명(`reviewer`)이면 그 가족의 live role **전원**
         (`reviewer-gemini`·`reviewer-codex` …)으로 해소한다. 설계 §2 층3: "하나라도
         진행 중이면 stall 아님" — 그래서 전원을 돌려주고 판정은 호출부가 AND 로 한다.
      ③ 해소 불가 → `none`. 호출부는 **미발화 + ledger `label_unjoined` + badge '스키마 결함'**
         이다. 조용히 억제하면 D3(조인이 왜 안 붙는지 감춤)가 그대로 재발한다.

    ※ 이 조인은 설계 §0-3이 확정한 오발화의 근원이다(파일 라벨 `reviewer` vs live role
      `reviewer-gemini/codex` → None → 보수적 발화). 접두 해소가 그 근원을 없앤다.
    """
    lab = (label or "").strip().lower()
    if not lab:
        return [], "none"
    live = sorted(r for r in (live_roles or ()) if r)
    if lab in live:
        return [lab], "exact"
    fam = role_family(lab)
    #   `master` 는 정확일치 가족이라 접두 해소 대상이 아니다(pack.rs 규약과 동형).
    if fam is not None and fam == lab and lab != "master":
        kin = [r for r in live if role_family(r) == fam]
        if kin:
            return kin, "family"
    return [], "none"


# ─────────────────────────── report 해석 ───────────────────────────

def node_is_idle(report, node_label):
    """담당 노드가 데몬 실측 idle인가. True/False/None(정보 없음).

    ★W5 층3·층4: 권위 레코드(`measurements`)를 소비하고, 라벨은 B4′ resolver 로 해소한다.
      가족 해소된 경우 **전원 idle 일 때만** True(하나라도 진행 중이면 stall 아님)."""
    ms = measurements(report)
    roles, how = resolve_label_roles(node_label, set(ms))
    if how == "none":
        return None
    vals = [ms[r].get("idle_secs") for r in roles if r in ms]
    if not vals or any(not isinstance(v, int) for v in vals):
        return None
    return all(v >= IDLE_ALERT_SECS for v in vals)


def in_progress_tasks(report):
    return [n for n in (report.get("nodes") or [])
            if n.get("total", 0) > 0 and n.get("done", 0) < n.get("total", 0)]


def pending_outside_nodes(report):
    """`nodes[]` **밖**에 남은 미완 작업 **전건**(javis_report의 동명 필드 · 교정 2-b).

    구버전 보고기(이 필드를 싣지 않는 팩)에서는 빈 목록 → 종전 동작과 동일하다(ADR-2 스큐 안전:
    양측은 서로를 전제하지 않는다). 새 필드가 도착하면 그때부터 불변식이 발효한다.

    ⚠park 판정에 쓰는 것은 이 전건이 아니라 `unresolved_pending_nodes`다(W15 교정 3).
    이 함수는 보고·진단용 전건 접근자로 남는다.
    """
    return list(report.get("pending_outside_nodes") or [])


# ★W15 교정 3 — park 차단에서 **면제되는** 갈래(javis_report.PENDING_KIND_STALE_GHOST와 같은 값).
# 값이 갈리면 게이트가 유령을 unresolved로 보아 종전 결함(park 영구 차단)이 그대로 남거나,
# 반대로 unresolved를 유령으로 보아 false QUIET이 난다 — 후자가 훨씬 위험하므로 **모르는
# kind는 전부 unresolved로 취급**한다(아래 비교가 `!=` 인 이유).
PENDING_KIND_STALE_GHOST = "stale_ghost"


def unresolved_pending_nodes(report):
    """집계 밖 미완 중 **주인 불명**인 것만(= park를 막는 것만 · W15 교정 3).

    종전에는 `pending_outside_nodes` 전체가 park를 막았고, 그래서 07-26 형태의 유령이
    존재하는 동안 `quiet_branch_holds`가 **영원히** False였다 — 유령을 성공적으로 배제한
    바로 그 사실이 세션 주차를 영구 차단한 것이다. 유령은 정의상 오래된 파일이라 시간이
    풀어주지도 않는다.

    분리 기준은 **누가 그렇게 말했는가**다(판정은 산출기 `javis_report.pending_kind`가 내리고
    여기서는 그 라벨만 읽는다 — 두 번째 기준을 만들지 않는다).
      · `unresolved`  주인 불명(`unclaimed`·`orphan-scope`·`shadowed`·휴리스틱 `retired`)
                      **+ 휴리스틱 `stale`**(담당 role이 살아 있다 · W18 교정 1)
                      → 우리가 마지막 관측자다. **park 차단 유지.**
      · `stale_ghost` 우리 추론이고 담당 role도 없다(휴리스틱 `orphan`) → park를 막지 않는다.
                      이미 집계에서 배제할 만큼 확신한 판정을 park만 영원히 막게 두는 것은
                      일관성이 없다(`foreign-scope` 면제와 같은 논리다).

    ★W18 교정 1 — 종전에는 휴리스틱 `stale`도 면제였다. `orphan`과 `stale`을 가르는 것은
    **담당 role의 생사**뿐인데(`javis_report.classify_files`), `stale`은 담당 role이 **살아
    있는** 파일이다. 살아있는 담당자의 미완을 우리 추론(mtime)만으로 침묵시키면 ADR-3·A3
    교훈에 어긋난다. 07-26 유령 4파일은 전부 종결 레인(role 부재)=`orphan`이라 W15가 풀려던
    livelock 해소는 그대로 유지된다.

    ★스큐(ADR-2): 구버전 보고기는 `kind`를 싣지 않는다 → 그 항목은 `unresolved`가 되어
    **종전과 똑같이** park를 막는다. 새 필드가 도착해야 면제가 발효한다 = 보수적 방향이다.
    """
    return [r for r in pending_outside_nodes(report)
            if r.get("kind") != PENDING_KIND_STALE_GHOST]


def quiet_branch_holds(report):
    """★구조적 불변식 — QUIET(=세션 주차 후보) 분기의 **전체** 성립 조건.

    조건 3개의 AND다:
      ① `nodes[]`에 미완 작업이 없다
      ② **`nodes[]` 밖에도** 주인 불명(`unresolved`) 미완 작업이 없다   ← 신설(W15에서 축소)
         (면제 = **주인이 처분을 명시한 것**(`retired`·`foreign-scope` · 산출기가 목록에서
          아예 뺀다) + **담당 role이 사라진 종결 레인 유령**(`stale_ghost` = 휴리스틱 `orphan`
          · W15 교정 3에서 park 차단만 면제 · W18 교정 1에서 `stale` 제외). 주인 불명인
          `orphan-scope`·`unclaimed`·`shadowed`·휴리스틱 `retired`와, 담당 role이 살아 있는
          휴리스틱 `stale`은 면제가 아니다.
          판정은 산출기 `javis_report.pending_kind`가 내리고 여기서는 라벨만 읽는다.)
      ③ 전 활성 노드가 idle이다

    ★W15 교정 3 — ②를 전건에서 `unresolved`로 좁힌 이유. 종전 ②는 유령이 존재하는 동안
    park를 **영구히** 막았다. 유령을 배제한 바로 그 사실이 세션 주차를 영영 못 하게 만드는
    것은 일관성이 없고, 운영자가 유령을 처분할 경로도 없었다(그 경로는
    `javis_todo_stamp.py --promote-retire`로 함께 열었다 — 정책을 지키려면 도구가 그 정책을
    집행할 수 있어야 한다). 유령은 **보고에는 계속 노출된다**: park 조건에서만 뺐다.

    ②가 없던 시절, 버킷 분류가 한 번만 틀려도(orphan-scope 조용한 배제·완료 파일이 미완
    파일을 shadow) 미완 작업이 `nodes[]`에서 사라지고 ①이 공집합으로 성립해 게이트가
    **false QUIET → 세션 주차**로 갔다. 두 결함은 분류 단계가 달랐을 뿐 같은 사슬을 탔다.
    ②는 그 사슬의 마지막 고리를 끊는다 — 앞으로 분류가 또 틀려도 park 오발동으로 번지지 않는다.

    판정을 술어 하나로 뽑아 둔 이유: 분기가 인라인 AND로 흩어져 있으면 다음 소비자가
    ②를 빠뜨린 채 ①만 복제한다. 이 함수가 QUIET 판정의 단일 진입점이다.
    """
    return (not in_progress_tasks(report)) \
        and (not unresolved_pending_nodes(report)) \
        and all_nodes_idle(report)


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


def apply_policy(w):
    """층2 집행의 전제 — warn 객체에 `severity`·`channels`를 부착한다(라우터는 필드만 본다).
    정책표에 없는 trigger 는 **가장 조용한 기본값**(ledger+badge)으로 접는다: 미지의 발화가
    push 로 새는 경로를 원천 차단하되(deny-by-default), badge 로 존재는 드러낸다(침묵 금지)."""
    sev, chans = CHANNEL_POLICY.get(w.get("trigger"), (SEV_WARN, (CH_LEDGER, CH_BADGE)))
    w.setdefault("severity", sev)
    w.setdefault("channels", tuple(chans))
    return w


# ── EVT payload 매핑 표(계약 SOT: _round/EVENT_CONTRACT.md · javis_event.SCHEMA) ──
#   idle    → agent.silent {agent, silent_minutes, level=critical}
#   feed    → approval.needed {agent, task, summary}
#   stall   → agent.silent {agent, silent_minutes, level=critical}
#   context → EVT 매핑 없음(계약에 ctx 타입 부재) → 대장 전용
#   collect → EVT 매핑 없음 → 대장+badge
#   DELTA   → task_progress {task, stage, [pct]}
#   날짜변경 → briefing {counts:{running,inbox,approvals,alerts}}
def extract_warnings(report, counters=None, now=0, edge_cooldown=EDGE_COOLDOWN_SECS):
    """원문 report에서 WARN 트리거 목록 추출. 각 항목: trigger/reason/wake_body/(evt_type,evt_fields)/idem.

    ★W5: `counters`·`now`·`edge_cooldown` 를 **실제로 소비한다**(A3′ — 상류 v0.13.x의 idle_edge
    갈래 복원). counters 는 **읽기 전용**이다(탐지=순수 / 확정=배달 성공 후 disarm — 라우터 소유).
    counters 가 None 이면 엣지 판정을 건너뛰고 종전(레벨)처럼 동작한다(하위호환).
    """
    warns = []
    proc = ("read-screen 확인·재지시.")
    tail = " 기상절차: cys status --json 1콜 점검 병행."
    ms = measurements(report)

    # idle WARN은 pending-todo(진행 중 할 일이 남은) 노드 + todo 미상 노드에만 발화한다.
    # ★master 승인 2026-07-18(동작 현행 유지 — 리뷰어 노이즈 재유입 방지):
    #   - C4 간접 커버 주장은 **pending-todo 노드의 승인 프롬프트 대기에 한정**한다. done 노드
    #     (진행 완료·total==0 포함)는 억제되고, 무배정 노드(todo 없음)는 noisy-default로 발화하나
    #     프롬프트↔idle 매핑이 보장되지 않는다 → **done/무배정 노드의 프롬프트는 미커버**(직접
    #     감지기는 후속 과제). done 노드 idle 억제가 QUIET 도달성·quiet-park 신호를 살린다.
    #   - ★W5 층3: 라벨공간 조인은 B4′ resolver 가 담당한다(문자열 직접 비교 폐기 — 설계 §0-3).
    pending = {n.get("node") for n in (report.get("nodes") or [])
               if n.get("total", 0) > 0 and n.get("done", 0) < n.get("total", 0)}
    todo_labels = {n.get("node") for n in (report.get("nodes") or [])}
    live = set(ms) or live_role_names(report)
    pending_roles, unassigned_roles = set(), set()
    for lab in todo_labels:
        roles, how = resolve_label_roles(lab, live)
        if how != "none" and lab in pending:
            pending_roles.update(roles)
    for r in live:
        # 무배정 = 어떤 todo 라벨로도 해소되지 않는 live role.
        claimed = any(r in resolve_label_roles(lab, live)[0] for lab in todo_labels if lab)
        if not claimed:
            unassigned_roles.add(r)

    for n in report.get("idle_nodes") or []:
        role = (n.get("role") or "").lower()
        if not role:
            continue
        rec = ms.get(role) or {}
        mins = (rec.get("idle_secs") if isinstance(rec.get("idle_secs"), int)
                else (n.get("idle_secs") or 0)) // 60
        stamp = {"measure_source": rec.get("source") or "derived.idle_nodes",
                 "sampled_at": rec.get("sampled_at"),
                 "measured_idle_secs": rec.get("idle_secs", n.get("idle_secs"))}
        if role in pending_roles:
            #   active(배정된 미완 작업 보유) = 레벨 유지. 해소 주체가 명확한 클래스이고,
            #   push 는 이미 강등돼 있어(층2) 레벨이 stdin 을 잠식하지 않는다.
            warns.append(apply_policy({
                "trigger": "idle",
                "task": "gate-idle-%s" % role,
                "reason": "idle_5min:%s" % role,
                "wake_body": "[gate] idle: %s idle 5분+ — %s%s" % (role, proc, tail),
                "evt_type": "agent.silent",
                "evt_fields": {"agent": role, "silent_minutes": int(mins), "level": "critical",
                               "measure_source": stamp["measure_source"]},
                "idem": "gate-idle-%s" % role,
                "stamp": stamp,
            }))
        elif role in unassigned_roles:
            #   무배정 idle = **엣지 1회 + 쿨다운**(A3′ · idle-standby-v5 D1/D7). 종전의 레벨
            #   트리거가 매 주기 재발화해 §0-5의 자기강화 루프를 먹였다.
            if counters is not None and not edge_allows(counters, "idle_edge", role,
                                                        now, edge_cooldown):
                continue
            warns.append(apply_policy({
                "trigger": "idle",
                "task": "gate-idle-%s" % role,
                "reason": "idle_edge:%s" % role,
                "wake_body": "[gate] idle-신규: %s 무배정 idle 진입(5분+) — 1회 통보"
                             "(standby 억제 개시). 점검 후 임무 배정 또는 standby 승인.%s"
                             % (role, tail),
                "evt_type": "agent.silent",
                "evt_fields": {"agent": role, "silent_minutes": int(mins), "level": "critical",
                               "measure_source": stamp["measure_source"]},
                "idem": "gate-idle-%s" % role,
                "edge_role": role,
                "stamp": stamp,
            }))

    high = [n for n in (report.get("live_nodes") or [])
            if isinstance(n.get("context_pct"), int) and n["context_pct"] >= 60]
    if high:
        roles = ",".join("%s(%d%%)" % (n.get("role", "?"), n["context_pct"]) for n in high)
        warns.append(apply_policy({
            "trigger": "context",
            "task": "gate-context",
            "reason": "ctx_60:%s" % roles,
            "wake_body": "[gate] context: %s 컨텍스트 60%%+ — cycle-agent 집행 검토.%s" % (roles, tail),
            "evt_type": None, "evt_fields": None,
            "idem": "gate-context-%s" % ",".join(n.get("role", "?") for n in high),
        }))
    feed = report.get("feed_pending")
    if isinstance(feed, int) and feed > 0:
        warns.append(apply_policy({
            "trigger": "feed",
            "task": "gate-feed",
            "reason": "feed_pending:%d" % feed,
            "wake_body": "[gate] feed: 승인 대기 %d건 — 즉결 필요.%s" % (feed, tail),
            "evt_type": "approval.needed",
            "evt_fields": {"agent": "master", "task": "feed-approval",
                           "summary": "%d건 대기" % feed},
            "idem": "gate-feed",
        }))
    return warns


def build_label_join_warnings(report):
    """B4′ — 라벨→live role 조인 실패의 **노출**(설계 §2 층3 gemini ISSUE-3 수용).

    두 갈래를 모두 잡는다:
      ① 게이트 자신의 조인 실패: `nodes[]` 라벨이 live role 로 해소되지 않는다.
      ② 산출기(javis_report)가 이미 진단한 실패: 레코드의 `owner_unresolved`
         (선언 owner 가 role 라벨공간에 실재하지 않음 · javis_report.py:481).
    둘 다 **미발화 + ledger `label_unjoined` + badge '스키마 결함'** 이다. 조용한 억제 금지.
    ★status 미수집(live 모집단 공집합)에서는 판정하지 않는다 — 그때의 '미조인'은 스키마 결함이
      아니라 관측 부재이고, 매 데몬 부재 주기마다 거짓 결함 배지를 세우게 된다.
    """
    live = live_role_names(report)
    out = []
    if live:
        for n in report.get("nodes") or []:
            lab = n.get("node")
            if not lab:
                continue
            _roles, how = resolve_label_roles(lab, live)
            if how == "none":
                out.append(apply_policy({
                    "trigger": "label_unjoined",
                    "task": "gate-label-%s" % lab,
                    "reason": "label_unjoined:%s" % lab,
                    "wake_body": "[gate] 스키마 결함: todo 라벨 '%s'가 live role 어디에도 조인되지 "
                                 "않는다(접두 해소 실패). 발화는 억제하고 결함만 노출한다." % lab,
                    "evt_type": None, "evt_fields": None,
                    "idem": "gate-label-%s" % lab,
                    "badge_detail": {"label": lab, "live_roles": sorted(live),
                                     "path": n.get("path")},
                }))
    for n in report.get("nodes") or []:
        owner = n.get("owner_unresolved")
        if owner:
            out.append(apply_policy({
                "trigger": "label_unjoined",
                "task": "gate-owner-%s" % owner,
                "reason": "owner_unresolved:%s" % owner,
                "wake_body": "[gate] 스키마 결함: 선언 owner '%s'가 role 라벨공간에 실재하지 "
                             "않는다(javis_report 진단 소비)." % owner,
                "evt_type": None, "evt_fields": None,
                "idem": "gate-owner-%s" % owner,
                "badge_detail": {"owner": owner, "path": n.get("path")},
            }))
    return out


def build_stall_warnings(counters, report, cycle_minutes, stall_cycles, now_iso, now_epoch=0):
    """태스크(노드) 단위 stall 카운터 갱신 + 승격 대상 WARN 생성.

    전역 diff 카운터 금지 — 다른 태스크의 변화가 특정 태스크의 정체를 은폐한다(적대 검증 A6).
    승격 조건: 진행 시그니처 무변화 ≥stall_cycles **AND 담당 노드 idle**(데몬 실측). 노드 busy면
    정상 장기 라운드(리뷰 40분+ 등)이므로 카운터만 증가·승격 보류(오탐 억제 S1-2).

    ★2026-07-26 수정(유령 stall 무한 발화): 승격 후 재발화에 쿨다운(STALL_COOLDOWN_SECS).

    ★W5 §2-C **fail-closed 확증**: last_output 유래 idle 은 **push 확증 입력에서 제거**된다.
      stall WARN 자체는 종전대로 나가되(ledger+evt+badge), **critical push 승격**은 last_output
      과 독립인 2차 증거 3종이 **전부** 성립할 때만이다:
        ① todo 시그니처 무변화 ≥ 6주기 (파일시스템 증거 — 이 카운터)
        ② `cys set-status` age > 15분 (자기보고 증거)
        ③ usage 토큰 델타 0 (데몬 usage 관측)
      셋 중 하나라도 **미측정**이면 push 금지 + badge '측정 불가'(침묵 금지).
    """
    prev = counters.get("nodes", {}) or {}
    new_nodes = {}
    stalls = []
    tail = " 기상절차: cys status --json 1콜 점검 병행."
    ms = measurements(report)
    live = set(ms)
    for n in report.get("nodes") or []:
        label = n.get("node")
        sig = "%s/%s" % (n.get("done"), n.get("total"))
        pc = prev.get(label)
        if pc and pc.get("sig") == sig:
            count = pc.get("count", 0) + 1
            last_change = pc.get("last_change_ts", now_iso)
            last_stall = pc.get("last_stall_fired", 0)
        else:
            count = 0                 # 진행 변화 시에만 리셋(해당 태스크 기준)
            last_change = now_iso
            last_stall = 0            # 진행 재개 = 쿨다운도 리셋(다음 정체는 즉시 알린다)
        new_nodes[label] = {"sig": sig, "count": count, "last_change_ts": last_change,
                            "last_stall_fired": last_stall}

        in_progress = n.get("total", 0) > 0 and n.get("done", 0) < n.get("total", 0)
        if in_progress and count >= stall_cycles:
            if now_epoch and (now_epoch - last_stall) < STALL_COOLDOWN_SECS:
                continue              # 쿨다운 창 — 카운터는 계속 증가(다음 발화에 실제 경과 반영)
            roles, how = resolve_label_roles(label, live)
            if how == "none":
                continue              # 조인 불가 → build_label_join_warnings 가 결함으로 노출
            idle = node_is_idle(report, label)
            if idle is False:
                continue              # 노드 busy → 승격 보류(카운터는 이미 증가)
            # idle True 또는 None(미지=보수적 시끄러운 쪽으로 승격) → stall WARN
            new_nodes[label]["last_stall_fired"] = now_epoch or last_stall
            mins = count * cycle_minutes
            measured = [ms[r].get("idle_secs") for r in roles if r in ms]
            measured_idle = max([v for v in measured if isinstance(v, int)] or [0])
            confirm, evidence = stall_confirmation(counters, roles, ms, count, stall_cycles)
            trigger = "stall_confirmed" if confirm is True else "stall"
            stalls.append(apply_policy({
                "trigger": trigger,
                "task": "gate-stall-%s" % label,
                "reason": "stall:%s(%d주기·%s·confirm=%s)"
                          % (label, count, "idle" if idle else "미지", confirm),
                "wake_body": "[gate] ⚠ stall: %s %d주기(%d분) 무진행·노드 %s — 워커 점검·재지시. "
                             "근거[%s]%s"
                             % (label, count, mins, "idle" if idle else "생존미상",
                                evidence.get("summary", "-"), tail),
                "evt_type": "agent.silent",
                # ★층4 silent_minutes 필드 분리: `silent_minutes` 는 **실측**(권위 레코드) 값이고,
                #   주기 환산값은 `stall_minutes` 로 따로 싣는다. 두 값을 한 필드에 섞으면
                #   "무엇을 재서 그렇게 판정했는가"가 소실된다(설계 §2 층4).
                "evt_fields": {"agent": label,
                               "silent_minutes": int(measured_idle // 60),
                               "stall_minutes": int(mins), "stall_cycles": int(count),
                               "level": "critical",
                               "measure_source": (ms.get(roles[0], {}) or {}).get("source")
                               if roles else "unavailable"},
                "idem": "gate-stall-%s" % label,
                # push 승격 시의 재발화 상한 = WARN 자체의 쿨다운과 같은 창(1h).
                "cooldown": STALL_COOLDOWN_SECS,
                "stamp": {"measure_source": (ms.get(roles[0], {}) or {}).get("source")
                          if roles else "unavailable",
                          "sampled_at": (ms.get(roles[0], {}) or {}).get("sampled_at")
                          if roles else None,
                          "measured_idle_secs": measured_idle,
                          "silent_minutes_derived": int(mins)},
                "confirm": confirm,
                "evidence": evidence,
                "badge_detail": {"label": label, "roles": roles, "evidence": evidence},
            }))
    counters["nodes"] = new_nodes
    return stalls


def build_measure_warnings(stall_warns):
    """§2-C '측정 불가'의 노출 — stall 확증 입력이 하나라도 미측정이면 **배지로 드러낸다**.

    ★`build_stall_warnings` 밖에 두는 이유: 그 함수의 반환 계약은 "stall 경고 목록"이고,
      다른 성질의 항목을 섞으면 기존 소비자(`w["evt_fields"]["agent"]` 를 훑는 회귀 테스트 등)가
      조용히 깨진다. 판정 결과(`confirm`·`evidence`)는 stall warn 이 이미 싣고 있으므로 여기서
      파생만 한다 — 두 번째 판정 기준을 만들지 않는다.
    """
    out = []
    for w in stall_warns:
        if w.get("confirm") is not None:
            continue
        ev = w.get("evidence") or {}
        label = (w.get("task") or "gate-stall-?")[len("gate-stall-"):]
        out.append(apply_policy({
            "trigger": "measure",
            "task": "gate-measure-%s" % label,
            "reason": "stall_unmeasurable:%s(%s)" % (label, ev.get("missing")),
            "wake_body": "[gate] 측정 불가: %s stall 확증 2차 증거 미측정(%s) — "
                         "push 금지·배지 노출." % (label, ev.get("missing")),
            "evt_type": None, "evt_fields": None,
            "idem": "gate-measure-%s" % label,
            "badge_detail": {"label": label, "missing": ev.get("missing")},
        }))
    return out


def stall_confirmation(counters, roles, ms, count, stall_cycles):
    """§2-C fail-closed 확증. 반환 `(verdict, evidence)`.

    verdict: True=확증(3종 전부 성립) / False=반증(하나라도 불성립) / None=**미측정**(push 금지).
    last_output 유래 idle 은 입력에 없다(설계 §2-C 핵심 변경 — codex C4 수용).
    """
    ev = {"sig_unchanged_cycles": int(count), "roles": list(roles)}
    missing = []
    #   ① 파일시스템 증거 — todo 시그니처 무변화 ≥ stall_cycles
    ev["e1_sig"] = count >= stall_cycles
    #   ② 자기보고 증거 — set-status age > 15분
    ages = [(ms.get(r) or {}).get("status_age_secs") for r in roles]
    if not ages or any(not isinstance(a, (int, float)) for a in ages):
        missing.append("set_status_age")
        ev["e2_setstatus"] = None
    else:
        ev["e2_setstatus"] = all(a > SETSTATUS_STALE_SECS for a in ages)
        ev["status_age_secs"] = [int(a) for a in ages]
    #   ③ 데몬 usage 관측 — 토큰 델타 0(주기 간 차분)
    prev_usage = (counters.get("usage") or {})
    deltas = []
    for r in roles:
        cur = (ms.get(r) or {}).get("usage_ctx_tokens")
        old = (prev_usage.get(r) or {}).get("tokens")
        if not isinstance(cur, int) or not isinstance(old, int):
            deltas.append(None)
        else:
            deltas.append(cur - old)
    if any(d is None for d in deltas):
        missing.append("usage_tokens")
        ev["e3_usage"] = None
    else:
        ev["e3_usage"] = all(d == 0 for d in deltas)
        ev["usage_delta"] = deltas
    ev["missing"] = ",".join(missing) if missing else ""
    if missing:
        ev["summary"] = "미측정:%s" % ev["missing"]
        return None, ev
    verdict = bool(ev["e1_sig"] and ev["e2_setstatus"] and ev["e3_usage"])
    ev["summary"] = "sig=%s·setstatus=%s·usage=%s" % (ev["e1_sig"], ev["e2_setstatus"],
                                                      ev["e3_usage"])
    return verdict, ev


def track_usage(counters, report):
    """§2-C 증거 ③의 차분 기반 — role별 누적 토큰을 counters 에 이월한다(다음 주기 델타용)."""
    ms = measurements(report)
    cur = {}
    for role, rec in ms.items():
        tok = rec.get("usage_ctx_tokens")
        if isinstance(tok, int):
            cur[role] = {"tokens": tok}
    counters["usage"] = cur


def build_deadlock_warning(report, tasks, now_epoch):
    """P3 시스템 데드락 술어(설계 §2 층2 · R2-C4 수용 — **last_output 완전 배제**).

    성립 조건(AND):
      ① 미배정(owner 없음) pending 티켓이 **존재**한다            [javis_task 원장 증거]
      ② **30분간 어떤 노드도 checkout 하지 않았다**(전 티켓 updated_at 최신값 age>30분)
      ③ 전 노드 **set-status age > 30분**                          [자기보고 증거]
    반환 `(warn|None, reason)` — reason 은 미성립·미측정 사유(대장에 남는다).

    ★idle(=last_output 파생)은 어떤 항에도 들어가지 않는다. 설계 §0-4가 확정했듯 그 값은
      "픽셀이 안 바뀐 시간"이지 "에이전트가 멎은 시간"이 아니다.
    """
    if tasks is None:
        return None, "deadlock_unmeasured:tasks"
    open_statuses = ("backlog", "todo", "in_progress", "in_review", "blocked")
    rows = [t for t in tasks if isinstance(t, dict)]
    unassigned = [t for t in rows
                  if t.get("status") in open_statuses and not t.get("owner")]
    if not unassigned:
        return None, "deadlock_no:no_unassigned_ticket"
    stamps = []
    for t in rows:
        e = _parse_iso_epoch(t.get("updated_at")) or _parse_iso_epoch(t.get("created_at"))
        if isinstance(e, (int, float)):
            stamps.append(e)
    if not stamps:
        return None, "deadlock_unmeasured:ticket_ts"
    since_checkout = now_epoch - max(stamps)
    if since_checkout < DEADLOCK_IDLE_SECS:
        return None, "deadlock_no:recent_ticket_activity(%ds)" % int(since_checkout)
    ms = measurements(report)
    if not ms:
        return None, "deadlock_unmeasured:no_live_roles"
    ages = [(rec or {}).get("status_age_secs") for rec in ms.values()]
    if any(not isinstance(a, (int, float)) for a in ages):
        return None, "deadlock_unmeasured:set_status_age"
    if not all(a > DEADLOCK_IDLE_SECS for a in ages):
        return None, "deadlock_no:some_node_reported_recently"
    detail = {"unassigned_tickets": [t.get("id") for t in unassigned][:20],
              "since_last_ticket_activity_secs": int(since_checkout),
              "status_age_secs": sorted(int(a) for a in ages)}
    return apply_policy({
        "trigger": "deadlock",
        "task": "gate-deadlock",
        "reason": "deadlock:미배정 %d건·티켓 무활동 %d분·전 노드 자기보고 %d분+"
                  % (len(unassigned), int(since_checkout // 60), DEADLOCK_IDLE_SECS // 60),
        "wake_body": "[gate] ⚠ 시스템 데드락 의심: 미배정 pending 티켓 %d건(%s)이 %d분간 "
                     "아무 노드에도 checkout 되지 않았고, 전 노드 set-status 가 %d분+ 정지. "
                     "배정·재기동 판단 필요."
                     % (len(unassigned), ",".join(detail["unassigned_tickets"][:5]),
                        int(since_checkout // 60), DEADLOCK_IDLE_SECS // 60),
        "evt_type": "agent.silent",
        "evt_fields": {"agent": "fleet", "silent_minutes": int(since_checkout // 60),
                       "level": "critical"},
        "idem": "gate-deadlock",
        "badge_detail": detail,
        "cooldown": DEADLOCK_COOLDOWN_SECS,
    }), "deadlock_yes"


def build_death_warnings(events):
    """노드 사망 — **데몬 deadman 이벤트 소비**(설계 §2 층2: 중복 채널 제거).

    게이트가 스냅샷 diff 로 사망을 독자 판정하던 갈래(idle-standby-v5 D6)는 W5에서 **폐기**한다.
    같은 사실에 두 개의 판정자를 두면 두 배로 울리고, 두 판정이 갈리면 어느 쪽도 못 믿는다.
    데몬 `master.deadman` 이 유일 판정자이고 게이트는 그것을 소비해 채널(push/badge)로 옮긴다.
    """
    out = []
    for ev in events or []:
        if ev.get("name") != "master.deadman":
            continue
        payload = ev.get("payload") or {}
        role = (payload.get("role") or "master")
        out.append(apply_policy({
            "trigger": "death",
            "task": "gate-death-%s" % role,
            "reason": "death:%s" % role,
            "wake_body": "[gate] ⚠ death: %s 노드 사망(데몬 deadman 확증) — 재기동·복구 판단 필요. "
                         "기상절차: cys status --json 1콜." % role,
            "evt_type": "agent.silent",
            "evt_fields": {"agent": role, "silent_minutes": 0, "level": "critical"},
            "idem": "gate-death-%s" % role,
            "badge_detail": {"role": role, "seq": ev.get("seq"), "payload": payload},
            "cooldown": DEATH_DEBOUNCE_SECS,
            #   ★자기참조 차단 입력 — "누가 죽었는가"를 라우터까지 옮긴다. live_roles 로는 판정할
            #     수 없다(빈 셸이 role 을 쥐면 로스터상 '생존'으로 보인다 — 실측 surface:241).
            #     사망 당사자의 유일한 권위 판정은 deadman 이벤트의 role 이다.
            "avoid_role": role,
        }))
    return out


# ─────────────────────────── 외부 명령 Runner(주입 가능) ───────────────────────────

class Runner:
    """실 subprocess 러너 — 외부 명령을 감싼다. 테스트는 동일 메서드의 대역을 주입한다.

    wakeup 큐 루트(JAVIS_ROOT)는 state_dir로 고정한다 — launchd cwd=/ 오염 사고 계열 방어
    (memory). enqueue/drain이 같은 루트를 공유하므로 self-consistent(배달은 drain이 cys send로 수행).
    """

    def __init__(self, pack_bin, state_dir, timeout=30):
        self.pack_bin = pack_bin
        self.timeout = timeout
        self.cys_bin = resolve_cys_bin()   # launchd 최소 env(PATH에 cys 부재)에서도 절대 해석
        # CYS_SOCKET 부재 = 기본 소켓(본사 데몬 기준 정상). 별도 설정 없이 그대로 상속.
        self.wk_env = dict(os.environ, JAVIS_ROOT=state_dir)
        # 티켓 원장 루트는 wakeup 큐 루트와 **다르다**(티켓은 워크스페이스 자산이다).
        # schedule.json 의 learn 잡 관례와 동형: `JAVIS_ROOT="${JAVIS_ROOT:-$HOME/.cys/state}"`.
        self.task_root = (os.environ.get("JAVIS_TASK_ROOT") or os.environ.get("CYS_TASK_ROOT")
                          or os.environ.get("JAVIS_ROOT")
                          or os.path.join(os.path.expanduser("~"), ".cys", "state"))

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

    def enqueue(self, to, task, reason, idem, payload=None, severity=None):
        """(rc, wakeup_id|None). ★W5: severity 를 큐에 실어 digest 가 critical 필드를 보존한다.
        wakeup_id 는 A3′ 전달 상태머신이 `queue.delivered` 영수증과 대조하는 열쇠다."""
        script = os.path.join(self.pack_bin, "javis_wakeup.py")
        argv = [sys.executable, script, "enqueue", "--to", to, "--task", task, "--reason", reason]
        if idem:
            argv += ["--idempotency-key", idem]
        if payload:
            argv += ["--payload", json.dumps(payload, ensure_ascii=False)]
        if severity:
            argv += ["--severity", severity]
        try:
            r = subprocess.run(argv, capture_output=True, text=True,
                               timeout=self.timeout, env=self.wk_env)
        except (subprocess.SubprocessError, OSError):
            return 1, None
        wid = None
        for line in (r.stdout or "").splitlines():
            line = line.strip()
            if line.startswith("{"):
                try:
                    wid = json.loads(line).get("id") or wid
                except ValueError:
                    pass
        return r.returncode, wid

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

    def poll_events(self, after_seq, names, timeout=EVENT_POLL_TIMEOUT):
        """(ok, events, latest_seq) — 데몬 이벤트 링버퍼 1회 회수(구독 아님·즉시 종료).

        `events.stream` 은 스트리밍 RPC지만 **구독 즉시 replay 를 밀어준다**(handlers.rs:2222
        `Reply::EventStream{ack, after_seq, ...}`). 게이트는 5분 주기 잡이라 상주 구독이 불가하므로
        연결→요청→replay 수신→종료의 1회 폴링으로 소비한다(읽기 전용·데몬 상태 무변경).

        ok=False = 회수 불가(소켓 부재·Windows named pipe·데몬 미가동). 호출부는 이때
        **critical 티어를 at-most-once 로 loud 강등**한다 — ack 를 영영 못 받는 환경에서
        at-least-once 를 고수하면 TTL 마다 영구 재발화가 된다(침묵도 폭주도 아닌 제3의 길).
        """
        path = resolve_socket_path()
        if not path:
            return False, [], after_seq
        req = json.dumps({"id": 1, "method": "events.stream",
                          "params": {"after_seq": after_seq, "names": list(names),
                                     "categories": []}}) + "\n"
        events, latest, ack_latest = [], after_seq, None
        sock = None
        try:
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            sock.settimeout(timeout)
            sock.connect(path)
            sock.sendall(req.encode("utf-8"))
            buf = b""
            deadline = time.time() + timeout
            while time.time() < deadline:
                try:
                    chunk = sock.recv(65536)
                except socket.timeout:
                    break
                except OSError:
                    break
                if not chunk:
                    break
                buf += chunk
                while b"\n" in buf:
                    raw, buf = buf.split(b"\n", 1)
                    raw = raw.strip()
                    if not raw:
                        continue
                    try:
                        v = json.loads(raw.decode("utf-8", "replace"))
                    except ValueError:
                        continue
                    kind = v.get("type")
                    if kind == "event":
                        seq = v.get("seq")
                        if isinstance(seq, int):
                            latest = max(latest, seq)
                        events.append(v)
                    elif kind == "ack":
                        ls = v.get("latest_seq")
                        if isinstance(ls, int):
                            ack_latest = ls
                            if ls <= after_seq:
                                #   밀린 이벤트가 없다 = replay 도 없다 → 즉시 종료(대기 낭비 0).
                                raise _PollDone()
                    elif kind == "heartbeat":
                        #   keepalive 도달 = replay 는 이미 끝났다(순차 전송) → 종료.
                        raise _PollDone()
        except _PollDone:
            pass
        except (OSError, ValueError):
            return False, events, after_seq
        finally:
            if sock is not None:
                try:
                    sock.close()
                except OSError:
                    pass
        if ack_latest is not None:
            latest = max(latest, ack_latest)
        return True, events, latest

    def task_snapshot(self):
        """(ok, rows|None, err) — javis_task.py list(전건 JSON). P3 데드락 술어의 원장 증거."""
        script = os.path.join(self.pack_bin, "javis_task.py")
        if not os.path.isfile(script):
            return False, None, "javis_task.py 부재"
        env = dict(os.environ, JAVIS_ROOT=self.task_root)
        try:
            r = subprocess.run([sys.executable, script, "list"],
                               capture_output=True, text=True, timeout=self.timeout, env=env)
        except (subprocess.SubprocessError, OSError) as e:
            return False, None, "task list 실행 실패: %s" % e
        if r.returncode != 0:
            return False, None, "javis_task exit=%d" % r.returncode
        try:
            rows = json.loads(r.stdout)
        except ValueError as e:
            return False, None, "task JSON 파싱 실패: %s" % e
        return True, (rows if isinstance(rows, list) else []), None

    def send_queued(self, to, body):
        """레거시 직송 경로 — ★W5 I2 수렴으로 **게이트는 더 이상 호출하지 않는다**.
        (제거하지 않는 이유: 외부 대역·테스트가 이 표면을 참조하고, 표면 제거는 계약 파괴다.)"""
        try:
            r = subprocess.run([self.cys_bin, "send", "--queued", "--to", to, body],
                               capture_output=True, text=True, timeout=self.timeout)
            return r.returncode
        except (subprocess.SubprocessError, OSError):
            return 1


class _PollDone(Exception):
    """poll_events 내부 조기 종료 신호(정상 경로)."""


# ─────────────────────────── 게이트 코어 ───────────────────────────

class Gate:
    def __init__(self, state_dir, runner, cycle_minutes=CYCLE_MINUTES_DEFAULT,
                 stall_cycles=STALL_CYCLES_DEFAULT, quiet_cycles=QUIET_CYCLES_DEFAULT,
                 now_epoch_fn=time.time, now_iso_fn=_now_iso,
                 edge_cooldown=EDGE_COOLDOWN_SECS, seen_ttl=SEEN_TTL_SECS):
        self.state_dir = state_dir
        self.runner = runner
        self.cycle_minutes = cycle_minutes
        self.stall_cycles = stall_cycles
        self.quiet_cycles = quiet_cycles
        self.now_epoch_fn = now_epoch_fn
        self.now_iso_fn = now_iso_fn
        self.edge_cooldown = edge_cooldown
        self.seen_ttl = seen_ttl
        self.snap_path = os.path.join(state_dir, "last_snapshot.json")
        self.counters_path = os.path.join(state_dir, "counters.json")
        self.badges_path = os.path.join(state_dir, "badges.json")
        self.measure_path = os.path.join(state_dir, "last_measurement.json")
        self.cursor_path = os.path.join(state_dir, "event_cursor.json")
        self._badges = []
        self._push_log = []          # 이번 실행에서 실제로 큐에 넣은 push(대장·검증용)
        self._events = []            # 이번 실행에서 회수한 데몬 이벤트(1회 폴링 결과)
        self._ack_ok = False         # 영수증 회수 가능 여부(critical 티어 판정 입력)

    # ── 최종 stdout 1줄: schedule.command_done 텔레메트리에 실린다(데드맨 1차 강화·P0 확정) ──
    #    ★W5 N6b: `gate_signal=<name>` 토큰은 **state 를 못 쓰는 상황의 유일한 출구**다.
    #      데몬(schedule.rs)이 이 토큰을 읽어 `gate.state_unwritable` 이벤트를 발행하고
    #      CC 배지로 올린다 — 같은 state 의 ledger 를 기대할 수 없으므로 state **외부** oracle 이다.
    def _summary(self, verdict, delivered, reasons, signals=()):
        line = ("verdict=%s delivered=%s reasons=%s"
                % (verdict, delivered, ",".join(reasons) if reasons else "-"))
        for s in signals:
            line += " gate_signal=%s" % s
        print(line)

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

    # ── A8 배지 — 데몬 alerts.rs `node_liveness` 가 소비해 CC 배지로 자동 노출된다(GUI 작업 0) ──
    def _badge(self, key, severity, message, detail=None):
        self._badges.append({"key": key, "severity": severity, "message": message,
                             "detail": detail or {}})

    def _flush_badges(self):
        """매 run 마다 **반드시** 기록한다 — 갱신 정지 자체가 데몬측 데드맨 신호가 되기 때문이다.
        경고가 없으면 `quiet` 배지 1건(정상 대기의 적극적 증거)."""
        badges = self._badges or [{"key": "quiet", "severity": SEV_INFO,
                                   "message": "정상 대기(경고 없음)", "detail": {}}]
        try:
            _write_json_atomic(self.badges_path, {
                "schema_version": BADGE_SCHEMA_VERSION,
                "updated_at": self.now_epoch_fn(),
                "lane": lane_id(self.state_dir),
                "badges": badges[:32],
            })
        except OSError:
            pass

    def _write_measurement(self, report):
        """층4 — 판정에 쓴 **실측값**의 영속 수용처. BLACKLIST(diff 제외)와 역할이 다르다:
        diff 에서 빼는 것은 오탐 DELTA 방지이고, 여기 남기는 것은 사후 추적성 확보다."""
        ms = measurements(report)
        try:
            _write_json_atomic(self.measure_path, {
                "schema_version": SCHEMA_VERSION,
                "written_at": self.now_epoch_fn(),
                "sampled_at": report.get("sampled_at"),
                "measure_source": report.get("measure_source"),
                "records": [ms[k] for k in sorted(ms)],
            })
        except OSError:
            pass

    def _load_cursor(self):
        c = _load_json(self.cursor_path, None)
        if isinstance(c, dict) and isinstance(c.get("seq"), int):
            return c["seq"]
        return 0

    def _save_cursor(self, seq):
        try:
            _write_json_atomic(self.cursor_path, {"seq": int(seq)})
        except OSError:
            pass

    def run(self, shadow=False):
        # ★B7 외부 데몬 가드(락 획득 전): socket-pack 부정합(부서 데몬이 본사 팩 로드)이면
        #   대장에 SKIPPED_FOREIGN_DAEMON 1줄만 기록하고 즉시 exit 0 — 카운터·배달·stall 무접촉.
        foreign = foreign_daemon_verdict()
        if foreign is not None:
            verdict, reason = foreign
            try:
                ledger_append(self.state_dir, {"ts": self.now_iso_fn(),
                                               "ts_epoch": self.now_epoch_fn(),
                                               "verdict": verdict, "reasons": [reason],
                                               "delta_fields": [], "delivered": "none",
                                               "lane": lane_id(self.state_dir)})
            except OSError:
                pass
            self._summary(verdict, "none", [reason])
            return 0
        # ★최상위 fail-open(P1): 락 획득·state_dir 접근의 OSError(PermissionError/ENOSPC/EROFS
        #   포함)가 exit 1로 죽는 경로를 봉쇄한다.
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
                                               "delivered": "none",
                                               "lane": lane_id(self.state_dir)})
                self._summary("SKIPPED_CONCURRENT", "none", ["lock_held"])
                return 0
        except Exception as e:                          # noqa: BLE001 — state/락 접근 실패 최상위 fail-open
            return self._fail_open_no_state(e)

    def _fail_open_no_state(self, exc):
        """state_dir/락 접근 불가(대장·카운터·배지 전부 기록 불능).

        ★W5 I2: 종전의 **master 직송을 제거**한다(설계 층1 I2 — 라우터 경유로 수렴). 대신
        stdout 요약에 `gate_signal=state_unwritable` 토큰을 실어 데몬이 `gate.state_unwritable`
        이벤트로 승격하고 CC 배지로 노출한다. 같은 state 의 ledger 를 기대할 수 없으므로 이것이
        **state 외부 독립 oracle** 이다(설계 §1-B N6b · R3-GATE 수용).
        """
        self._summary("FAILOPEN", "none", ["state_unwritable:%s" % (str(exc)[:60])],
                      signals=("state_unwritable",))
        return 0

    def _run_locked(self, shadow):
        counters = _load_json(self.counters_path, {})
        now_epoch = self.now_epoch_fn()
        report = None
        try:
            #   ★이벤트 폴링은 **실행당 정확히 1회**다. 커서가 하나뿐이라 이름별로 나눠 폴링하면
            #     먼저 도는 쪽이 커서를 전역 최신으로 밀어 다른 이름의 이벤트를 통째로 삼킨다
            #     (구현 중 실측한 함정). 한 번에 받아 소비처가 이름으로 갈라 쓴다.
            self._poll_once()
            #   A3′ crash 복구 + A6′ GC 는 판정보다 **먼저** 돈다: 지난 실행의 inflight 가
            #   이번 주기의 억제/재시도 판정 입력이기 때문이다.
            self._reconcile_inflight(counters, now_epoch)
            seen_gc(self.state_dir, now_epoch, self.seen_ttl)
            report = self._judge_and_route(shadow, counters)
            counters["failopen_streak"] = 0
            self._write_counters(counters)
            return 0
        except Exception as e:                                   # noqa: BLE001 (최상위 fail-open)
            return self._fail_open(e, report, counters)

    def _fail_open(self, exc, report, counters):
        """게이트 내부 오류 → **직송 없이** 대장+배지 기록 후 exit 0(설계 층1 I2 수렴).

        종전에는 원문 보고를 master 로 직송했다. 그 직송이 라우터를 우회하는 두 번째 발행자였고
        (§0 인벤토리 I2), 게이트가 고장난 순간 가장 시끄럽게 master 를 잠식했다. 정보는
        대장·배지로 전부 남으므로 손실이 아니라 채널 이동이다.
        """
        streak = counters.get("failopen_streak", 0) + 1
        counters["failopen_streak"] = streak
        try:
            self._write_counters(counters)
        except OSError:
            pass
        msg = "게이트 내부 오류 fail-open: %s" % exc
        if streak >= 3:
            msg += " (게이트 자체 수리 필요 — 연속 실패 %d회)" % streak
        self._badge("gate-internal-error", SEV_WARN, msg,
                    {"streak": streak, "has_report": bool(report)})
        self._flush_badges()
        try:
            ledger_append(self.state_dir, {"ts": self.now_iso_fn(),
                                           "ts_epoch": self.now_epoch_fn(),
                                           "verdict": "FAILOPEN",
                                           "reasons": [str(exc)[:200]], "delta_fields": [],
                                           "delivered": "none", "failopen_streak": streak,
                                           "lane": lane_id(self.state_dir)})
        except OSError:
            pass
        self._summary("FAILOPEN", "none", [str(exc)[:80]])
        return 0

    # ── A3′ 전달 상태머신: 1회 폴링 + crash 복구 + 영수증 대조 ─────────────────
    def _poll_once(self):
        """데몬 이벤트 1회 회수(`queue.delivered` 영수증 + `master.deadman` 사망 확증).
        러너가 이 표면을 갖지 않으면(구형 대역) 조용히 비활성 — 판정은 그만큼 보수적이 된다."""
        self._events, self._ack_ok = [], False
        fn = getattr(self.runner, "poll_events", None)
        if fn is None:
            return
        try:
            ok, events, latest = fn(self._load_cursor(),
                                    ["queue.delivered", "master.deadman"])
        except Exception:                       # noqa: BLE001 — 관측 실패가 판정을 죽이지 않는다
            return
        self._ack_ok = bool(ok)
        if ok:
            self._events = list(events or [])
            self._save_cursor(latest)

    def _tasks(self):
        fn = getattr(self.runner, "task_snapshot", None)
        if fn is None:
            return False, None, "runner_no_task_snapshot"
        try:
            return fn()
        except Exception as e:                  # noqa: BLE001
            return False, None, str(e)[:60]

    def _reconcile_inflight(self, counters, now_epoch):
        """지난 실행의 `inflight` 를 `queue.delivered` 영수증과 대조해 확정한다.

        영수증이 도착했으면 seen 을 `delivered` 로 굳히고 엣지를 disarm 한다(그때서야
        at-least-once 가 완결된다). 영수증이 없으면 아무것도 하지 않는다 — TTL 만료 시
        `seen_claim` 이 재선점을 허용해 **TTL당 1회**의 재enqueue 가 일어난다(C1·C3 oracle).
        """
        pend = [r for r in seen_iter(self.state_dir)
                if r.get("state") == SEEN_STATE_INFLIGHT and r.get("wakeup_id")]
        if not pend or not self._ack_ok:
            return
        acked = set()
        for ev in self._events:
            if ev.get("name") != "queue.delivered":
                continue
            payload = ev.get("payload") or {}
            for i in payload.get("entry_ids") or []:
                acked.add(i)
        for rec in pend:
            if rec.get("wakeup_id") in acked:
                seen_mark(self.state_dir, rec["key"], now_epoch,
                          state=SEEN_STATE_DELIVERED)
                edge_fire(counters, "push_edge", rec["key"], now_epoch)

    def _judge_and_route(self, shadow, counters):
        now_epoch = self.now_epoch_fn()
        now_iso = self.now_iso_fn()
        self._badges = []
        self._push_log = []

        ok, report, err = self.runner.collect_report()
        reasons = []
        if _LOCK_IMPORT_ERR:
            reasons.append("lock_module_missing")
        if _BOOTNODE_IMPORT_ERR:
            reasons.append("bootnode_module_missing")

        # 수집 실패 = **대장+배지**(설계 §1-B N6a: push 0, 정상 state ledger `collect_fail`).
        # 종전에는 여기서 WARN push 가 나갔다 — 데몬이 잠깐 없을 때마다 master 를 두드리던 경로다.
        if not ok:
            warns = [apply_policy({
                "trigger": "collect", "task": "gate-collect", "reason": "collect_fail:%s" % err,
                "wake_body": "[gate] collect: javis_report 수집 실패(%s) — 게이트/데몬 점검." % err,
                "evt_type": None, "evt_fields": None, "idem": "gate-collect",
                "badge_detail": {"error": str(err)[:200]},
            })]
            delivered = self._route_warn(warns, shadow, reasons, counters, now_epoch, set())
            self._flush_badges()
            ledger_append(self.state_dir, {"ts": now_iso, "ts_epoch": now_epoch,
                                           "verdict": VERDICT_WARN, "reasons": [w["reason"] for w in warns],
                                           "delta_fields": [], "delivered": delivered,
                                           "consecutive_nochg": counters.get("consecutive_nochg", 0),
                                           "consecutive_quiet": counters.get("consecutive_quiet", 0),
                                           "lane": lane_id(self.state_dir), "shadow": shadow})
            self._summary(VERDICT_WARN, delivered, [w["reason"] for w in warns])
            return report

        self._write_measurement(report)
        new_snap = normalize(report)
        old_snap = self._load_snapshot()

        # ── BASELINE: 스냅샷 부재(최초 실행·재설치) → 기록만, 배달 없음(DELTA 폭주 차단) ──
        if old_snap is None:
            self._write_snapshot(new_snap)
            counters.setdefault("consecutive_nochg", 0)
            counters.setdefault("consecutive_quiet", 0)
            init_idle_edge(counters, report, now_epoch)
            self._flush_badges()
            ledger_append(self.state_dir, {"ts": now_iso, "ts_epoch": now_epoch,
                                           "verdict": "BASELINE", "reasons": reasons, "delta_fields": [],
                                           "delivered": "none", "lane": lane_id(self.state_dir),
                                           "shadow": shadow})
            self._summary("BASELINE", "none", reasons)
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
                init_idle_edge(counters, report, now_epoch)
                self._write_counters(counters)
                self._flush_badges()
                ledger_append(self.state_dir, {"ts": now_iso, "ts_epoch": now_epoch,
                                               "verdict": "GAP", "reasons": ["interval>3cycles"],
                                               "delta_fields": [], "delivered": "none",
                                               "lane": lane_id(self.state_dir), "shadow": shadow})
                self._summary("GAP", "none", ["interval>3cycles"])
                return report

        if not report.get("status_available"):
            reasons.append("status_unavailable")           # 관측용(daemon 일시 부재)·wake 안 함

        # ── 분류 ──
        warns = extract_warnings(report, counters, now_epoch, self.edge_cooldown)
        warns += build_label_join_warnings(report)
        stalls = build_stall_warnings(counters, report, self.cycle_minutes,
                                      self.stall_cycles, now_iso, now_epoch)
        warns += stalls
        warns += build_measure_warnings(stalls)
        #   ★§2-C 증거 ③의 차분 기반 이월은 **stall 판정 뒤**에 한다. 앞서 갱신하면 직전 값이
        #     현재 값으로 덮여 델타가 항상 0 이 되고, fail-closed 가 fail-open 으로 뒤집힌다.
        track_usage(counters, report)
        #   노드 사망 = 데몬 deadman 이벤트 소비(중복 채널 제거 · 설계 층2)
        if self._ack_ok:
            warns += build_death_warnings(self._events)
        else:
            reasons.append("deadman_poll_unavailable")
        #   P3 시스템 데드락(last_output 완전 배제)
        t_ok, tasks, t_err = self._tasks()
        dl, dl_reason = build_deadlock_warning(report, tasks if t_ok else None, now_epoch)
        reasons.append(dl_reason if t_ok else "deadlock_unmeasured:%s" % (t_err or "task")[:40])
        if dl is not None:
            warns.append(dl)

        #   조건 해소 → push 엣지 재무장. 이번 주기에 나타나지 않은 push 키는 에피소드가 끝난
        #   것이므로 무장을 되돌린다(엣지의 정의). 되돌리지 않으면 쿨다운 0인 트리거가 영구
        #   침묵한다 — 엣지화의 가장 흔한 자해다.
        present_push = {seen_key(w["trigger"], w.get("idem") or w.get("task"),
                                 w.get("severity", SEV_WARN))
                        for w in warns if CH_PUSH in (w.get("channels") or ())}
        for k in list((counters.get("push_edge") or {})):
            if k not in present_push:
                edge_rearm(counters, "push_edge", k)

        delta_fields = diff_top_fields(old_snap, new_snap)

        if warns:
            verdict = VERDICT_WARN
        elif delta_fields:
            verdict = VERDICT_DELTA
        elif quiet_branch_holds(report):
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

        # ── 라우팅(층2 정책표 집행) ──
        live = live_role_names(report)
        delivered = "none"
        if verdict == VERDICT_WARN:
            delivered = self._route_warn(warns, shadow, reasons, counters, now_epoch, live)
        elif verdict == VERDICT_DELTA:
            delivered = self._route_delta(old_snap, new_snap, shadow, reasons)
        # QUIET/NOCHG → 대장 기록만

        #   idle 엣지 재무장: idle 목록에서 빠진(활동 재개한) role 은 다시 무장한다.
        #   last_fired 는 보존되므로 쿨다운이 진동 상한을 이룬다(idle-standby-v5 §3.2).
        idle_now = {(n.get("role") or "").lower() for n in (report.get("idle_nodes") or [])}
        for role in list((counters.get("idle_edge") or {})):
            if role not in idle_now:
                edge_rearm(counters, "idle_edge", role)

        # QUIET 연속 임계 도달 → 세션 주차 후보(P2 반자율·CSO 집행): enqueue만, 배달은 CSO 소관
        if verdict == VERDICT_QUIET and not shadow and \
                counters["consecutive_quiet"] >= self.quiet_cycles and \
                not counters.get("park_notified"):
            self._enqueue("cso", "master-park",
                          "QUIET %d주기(%d분) 지속 — 세션 주차 후보(cycle-agent 집행 검토)"
                          % (counters["consecutive_quiet"],
                             counters["consecutive_quiet"] * self.cycle_minutes),
                          "gate-park", SEV_INFO)
            counters["park_notified"] = True
            reasons.append("park_candidate")
        if in_progress_tasks(report):
            #   재무장은 verdict 가 아니라 **결정론 활동 신호**에 건다(idle-standby-v5 D5·v2.2).
            counters["park_notified"] = False

        # 날짜 변경 → 일 1회 briefing 백스톱(빌트인 fleet-digest 수정 불가 F3 → 게이트가 소유)
        if not shadow and last is not None:
            self._maybe_briefing(last, now_epoch, report, warns, reasons)

        self._write_snapshot(new_snap)
        self._flush_badges()
        ledger_append(self.state_dir, {"ts": now_iso, "ts_epoch": now_epoch, "verdict": verdict,
                                       "reasons": reasons + [w["reason"] for w in warns],
                                       "delta_fields": delta_fields, "delivered": delivered,
                                       "consecutive_nochg": counters["consecutive_nochg"],
                                       "consecutive_quiet": counters["consecutive_quiet"],
                                       "lane": lane_id(self.state_dir),
                                       "pushes": self._push_log,
                                       "measure_source": report.get("measure_source"),
                                       "sampled_at": report.get("sampled_at"),
                                       "shadow": shadow})
        self._summary(verdict, delivered, reasons + [w["reason"] for w in warns])
        return report

    def _route_warn(self, warns, shadow, reasons, counters, now_epoch, live_roles):
        """층2 집행기 — **warn 객체의 `channels`/`severity` 필드만 본다**(if 체인 금지).

        push 채널을 가진 warn 만 A3′ 상태머신(엣지→seen-claim→enqueue→영수증)을 탄다.
        나머지는 ledger(대장 항목의 reasons)·EVT·badge 로 끝난다 — 정보는 남고 stdin 만 비운다.
        """
        pushed_targets = set()
        push_ok = 0
        for w in warns:
            chans = w.get("channels") or ()
            #   ★badge 는 **shadow 에서도** 기록한다: 배지는 배달이 아니라 상태 파일이다
            #     (대장과 같은 층). shadow 의 계약은 "배달 0"이지 "관측 0"이 아니다.
            if CH_BADGE in chans:
                self._badge(w.get("idem") or w.get("task") or w["trigger"],
                            w.get("severity", SEV_WARN),
                            w.get("wake_body") or w.get("reason") or "",
                            dict(w.get("badge_detail") or {}, trigger=w["trigger"],
                                 stamp=w.get("stamp") or {}))
            if shadow:
                continue
            #   A3′ 확정 — 탐지(extract_warnings·순수)와 확정(여기)의 분리. **기록 채널의
            #   성공이 곧 확정**이다: push 가 강등된 뒤로 idle 엣지의 '배달'은 ledger·EVT·badge
            #   이므로, 종전의 "enqueue rc==0에서 disarm"을 그 자리로 옮긴다. shadow 는
            #   배달 0 이므로 disarm 하지 않는다(다음 실전 주기가 정상 발화).
            if w.get("edge_role"):
                edge_fire(counters, "idle_edge", w["edge_role"], now_epoch)
            if CH_EVT in chans and w.get("evt_type"):
                erc, _, _ = self.runner.emit(w["evt_type"], w["evt_fields"])
                if erc != 0:
                    reasons.append("evt_reject:%s(%d)" % (w["evt_type"], erc))
            if CH_PUSH not in chans:
                continue
            target = self._push(w, counters, now_epoch, reasons, live_roles)
            if target:
                pushed_targets.add(target)
                push_ok += 1
        delivered = "none"
        for target in sorted(pushed_targets):
            #   같은 실행에서 drain 까지 수행(치명 미완결 차단). digest(I6)가 target 별 N→1 로 접는다.
            _rc, n = self.runner.drain(target)
            delivered = "wake" if n > 0 else "wake_pending"
        if push_ok and delivered == "none":
            delivered = "wake_pending"
        return delivered

    def _push(self, w, counters, now_epoch, reasons, live_roles):
        """push 승격의 유일 경로. 반환=배달 target(성공) 또는 None(억제·실패)."""
        trigger = w["trigger"]
        severity = w.get("severity", SEV_WARN)
        key = seen_key(trigger, w.get("idem") or w.get("task"), severity)
        cooldown = w.get("cooldown", 0)
        if not edge_allows(counters, "push_edge", key, now_epoch, cooldown):
            reasons.append("push_suppressed_edge:%s" % trigger)
            return None
        claimed, rec = seen_claim(self.state_dir, key, severity, now_epoch, self.seen_ttl)
        if not claimed:
            reasons.append("push_suppressed_seen:%s(%s)" % (trigger, rec.get("state")))
            return None
        target, fallback = self._push_target(trigger, live_roles, w.get("avoid_role"))
        if target is None:
            #   수신자 = 사망 당사자인데 대체 수신자도 없다 → push 만 억제한다(ledger·EVT·badge 는
            #   이미 이 warn 에 대해 기록됐다 = 침묵이 아니라 채널 강등). seen 을 되돌려 좌석이
            #   복구되면 자연 재시도(enqueue 실패 경로와 동형 — 레벨 트리거의 자가치유 보존).
            try:
                os.unlink(seen_path(self.state_dir, key))
            except OSError:
                pass
            reasons.append("push_suppressed_self_target:%s" % fallback)
            return None
        #   ★층4 — 나가는 본문에 **판정 소스·샘플 시각·실측값**을 스탬프한다. 수신자가 "무엇을
        #     재서 그렇게 판정했는가"를 되물으러 오지 않아도 되게 하는 것이 목적이고, 사후에
        #     오발화를 역추적할 때 유일한 단서이기도 하다(설계 §2 층4).
        body = w["wake_body"] + stamp_suffix(w.get("stamp"))
        rc, wid = self._enqueue(target, w.get("task", "gate-" + trigger),
                                body, w["idem"], severity)
        if rc != 0:
            #   enqueue 실패 → seen 을 되돌려 다음 주기 자연 재시도(레벨 트리거의 자가치유 보존).
            try:
                os.unlink(seen_path(self.state_dir, key))
            except OSError:
                pass
            reasons.append("push_enqueue_failed:%s" % trigger)
            return None
        tier = tier_for(severity)
        #   영수증 회수가 **이번 실행에서 실제로 성립했는가**로 판정한다(경로 존재 여부가 아니라).
        #   데몬 미가동·Windows named pipe·구형 대역 전부 여기서 False 로 수렴한다.
        ack_capable = bool(self._ack_ok)
        if tier == TIER_AT_LEAST_ONCE and ack_capable:
            #   critical = queue.delivered 영수증 수신 후에만 disarm(§8 R2-C3).
            seen_mark(self.state_dir, key, now_epoch, state=SEEN_STATE_INFLIGHT, wakeup_id=wid)
        else:
            if tier == TIER_AT_LEAST_ONCE:
                #   ★loud 강등: 영수증 회수가 구조적으로 불가한 환경(named pipe·소켓 부재)에서
                #     at-least-once 를 고수하면 TTL 마다 영구 재발화가 된다. 배지로 드러내고
                #     at-most-once 로 내린다(침묵도 폭주도 아닌 제3의 길).
                reasons.append("ack_unavailable_downgrade:%s" % trigger)
                self._badge("ack-unavailable", SEV_WARN,
                            "queue.delivered 영수증 회수 불가 — critical 전달을 at-most-once 로 강등",
                            {"trigger": trigger})
            seen_mark(self.state_dir, key, now_epoch, state=SEEN_STATE_DELIVERED, wakeup_id=wid)
            edge_fire(counters, "push_edge", key, now_epoch)
        self._push_log.append({"trigger": trigger, "severity": severity, "target": target,
                               "wakeup_id": wid, "tier": tier, "fallback": fallback,
                               "key": key, "stamp": w.get("stamp") or {}})
        if fallback:
            reasons.append("push_target_fallback:%s" % fallback)
        return target

    def _enqueue(self, target, task, body, idem, severity):
        """(rc, wakeup_id) — 신형 `severity` 계약을 쓰되 구형 대역(인자 미보유)도 수용한다.
        표면 스큐로 게이트가 죽는 경로를 만들지 않는다(ADR-2)."""
        try:
            res = self.runner.enqueue(target, task, body, idem, severity=severity)
        except TypeError:
            res = self.runner.enqueue(target, task, body, idem)
        return _rc_and_id(res)

    def _push_target(self, trigger, live_roles, avoid=None):
        """수신 계층 — 1차 CSO, 표가 지정한 trigger 만 master 직송, CSO 부재 시 master 폴백.

        ★자기참조 차단(2026-08-01 실사고 수리): 사망 당사자에게 그 사망 경보를 보내지 않는다.
          종전 PUSH_TARGET["death"]="master" 는 **누가 죽었는지와 무관하게** master 직송이라,
          death:master 가 죽은 master 자신에게 배달됐다. 좌석이 비어 배달이 성립하지 않으니
          queue.delivered 영수증도 영영 없고, critical 은 at-least-once 라 seen 이 inflight 로
          남아 TTL(30분)마다 재발화 → 죽은 좌석 큐에 자기 부고가 무한 적재된다.
          라이브 실측(surface:241): 11.5h 동안 death push 23회 · queue depth 36 · 재발화 간격
          중앙값 정확히 1800s(=SEEN_TTL_SECS).
          재정 ①(표 → cso)로 death:master 갈래는 표 층위에서 사라졌지만, 이 장치는 **남긴다**:
          표가 cso 를 가리키는 한 death:cso 는 같은 병을 앓고, 표는 언제든 다시 바뀐다.
          `avoid` 는 오직 deadman 이 확증한 사망 당사자만 담는다(stall·deadlock 은 좌석이 살아
          있어 자기수신이 곧 처방이므로 대상이 아니다 — 넘기지 않는 것이 계약이다).
        """
        want = PUSH_TARGET.get(trigger, "cso")
        has_cso = any(role_family(r) == "cso" for r in (live_roles or ()))
        want_fam = role_family(want)
        if avoid and want_fam is not None and want_fam == role_family(avoid):
            #   수신자 자신이 사망 당사자 → 살아있는 대체 수신자(CSO)로 이관.
            if want != "cso" and has_cso:
                return "cso", "self_target_avoided:%s" % avoid
            #   대체 수신자 없음 → push 억제(호출자가 배지·EVT 로 강등). 죽은 대상 재송신 금지.
            #   ※ master 로는 폴백하지 않는다: 재정 ① 이후 master 는 이 표의 수신처가 아니고,
            #     `cso_absent` 폴백은 '부재'용이지 '사망 당사자 회피'용이 아니다(경보는 ledger·
            #     EVT·badge 로 남아 master 능동 모니터링이 회수한다).
            return None, "self_target_no_alt:%s" % avoid
        if want == "cso" and not has_cso:
            #   ★A4 잔존결함 수리(2026-08-01 적대검증): 이 폴백도 `avoid` 를 봐야 한다.
            #     위 자기참조 분기는 want 가족 == avoid 가족일 때만 걸린다. want='cso' 인데
            #     죽은 것이 master 면 가족이 달라 그 분기를 통과하고, 여기서 CSO 부재 폴백이
            #     **죽은 master 에게 자기 부고를 배달**한다 — 위 docstring 이 닫았다고 적은
            #     바로 그 병리(배달 불성립 → queue.delivered 영수증 영영 없음 → critical
            #     at-least-once 라 seen 이 inflight 로 남아 SEEN_TTL_SECS(1800s)마다 재발화)가
            #     폴백 경로로 되살아난다. 실측 재현: _push_target('death', ['worker'], 'master')
            #     → ('master','cso_absent') · _push_target('death', [], 'master') → 동일.
            #     처방: 폴백 수신자(master)가 사망 당사자면 push 억제(None) — 호출부의 기존
            #     자기참조 억제 분기(seen unlink + reasons `push_suppressed_self_target`)가
            #     그대로 받아 레벨 트리거 자가치유를 보존하고, 경보 자체는 ledger·EVT·badge 로 산다.
            if avoid and role_family(avoid) == "master":
                return None, "self_target_no_alt:%s" % avoid
            return "master", "cso_absent"
        return want, ""

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


def stamp_suffix(stamp):
    """층4 판정 근거 스탬프 문자열. 스탬프가 없으면 빈 문자열(본문 무변경 — 무회귀)."""
    if not stamp:
        return ""
    parts = []
    if stamp.get("measure_source"):
        parts.append("source=%s" % stamp["measure_source"])
    if stamp.get("sampled_at") is not None:
        parts.append("sampled=%s" % _epoch_stamp(stamp["sampled_at"]))
    if stamp.get("measured_idle_secs") is not None:
        parts.append("idle=%ss" % stamp["measured_idle_secs"])
    if stamp.get("silent_minutes_derived") is not None:
        parts.append("주기환산=%s분" % stamp["silent_minutes_derived"])
    return (" [측정 %s]" % " ".join(parts)) if parts else ""


def _epoch_stamp(v):
    try:
        return time.strftime("%H:%M:%S", time.localtime(float(v)))
    except (TypeError, ValueError, OSError, OverflowError):
        return str(v)


def init_idle_edge(counters, report, now_epoch):
    """BASELINE·GAP·카운터 파손 복원의 **공통** 초기화(idle-standby-v5 §3.2 v2.2 O-1).

    현재 idle 인 role 을 **disarmed** 로 시드한다 — 업그레이드·재부팅 직후 전 노드가 동시에
    엣지 발화하는 파도를 막는다. `park_notified` 도 함께 내린다.
    """
    edges = counters.setdefault("idle_edge", {})
    for n in report.get("idle_nodes") or []:
        role = (n.get("role") or "").lower()
        if role:
            edges[role] = {"armed": False, "last_fired": now_epoch}
    counters["park_notified"] = False


def _rc_and_id(res):
    """Runner.enqueue 반환 정규화 — 신형 `(rc, wakeup_id)` / 구형 `rc` 대역 양쪽 수용.
    (테스트 대역·외부 구현이 구형 계약을 쓰고 있어 표면을 깨지 않는다 — ADR-2 스큐 안전.)"""
    if isinstance(res, tuple):
        return (res[0], res[1] if len(res) > 1 else None)
    return res, None


def _parse_iso_epoch(s):
    if not s:
        return None
    if isinstance(s, (int, float)):
        return float(s)
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
