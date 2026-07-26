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

라이브 게이트(2026-07-05 박사님 지시 — 상시 라이브·기본 허용·명시적 opt-out):
  저널·산출은 항상 소켓의 상태 디렉터리 하위 'phoenix/'에만 쓴다(라이브 상태 파일 자체는 무접촉).
  기본값 = 라이브 허용(불사조 가동 시작·상시 라이브 고정). opt-out `PHOENIX_FORBID_LIVE=1` 일 때만
  라이브 상태 디렉터리 대상 실행을 거부한다(격리 하네스·순수 개발 재현용). 레거시 `PHOENIX_ALLOW_LIVE`
  는 더 이상 필요 없다(설정돼 있어도 무해하게 무시). 개발·검증은 여전히 격리 하네스 소켓(--socket)을 쓴다.

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
import atexit
import glob
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time

HOME = os.path.expanduser("~")

# 플랫폼 분기(Windows 패리티) — 상태 디렉터리·소켓 규약이 Rust(src/lib.rs·state.rs)와 정합해야 한다.
IS_WINDOWS = os.name == "nt"


def _live_state_dir():
    """라이브 상태 디렉터리 — unix: ~/.local/state/cys · Windows: %LOCALAPPDATA%\\cys
    (Rust cys::socket_path / state.rs state_dir 의 기본 데몬 규약과 정합)."""
    if IS_WINDOWS:
        base = os.environ.get("LOCALAPPDATA") or os.path.join(HOME, "AppData", "Local")
        return os.path.realpath(os.path.join(base, "cys"))
    return os.path.realpath(os.path.join(HOME, ".local", "state", "cys"))


LIVE_STATE = _live_state_dir()

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

# ★W1/C1 exit code 계약: run_restore 최종 판정 → 프로세스 exit code(P0-1 신호 역전 수리).
#   과거: cmd_restore 는 FAILED 여도 dict 만 return → 프로세스는 항상 exit 0 → cysd 가 완전실패와 성공을
#   구분 불가(exit 1 은 오히려 미포착 예외 crash 때만 나와 신호가 뒤집혀 있었다). 이제 판정을 결정론 exit 로 방출한다.
#   cysd 재시도 정책과 정합: 5(BREAKER)·6(손상/identity)=재시도 금지, 1·3=1회 지연 재시도, 0=성공(재시도 없음).
PHOENIX_EXIT_OK = 0        # VERIFIED · VERIFIED_FRESH · NOOP · LEASE_HELD(멱등 skip — 다른 restore가 담당)
PHOENIX_EXIT_FAILED = 1    # FAILED · INCOMPLETE(재시도 소진 후 미부활) — 재시도 가치 있음
PHOENIX_EXIT_DEGRADED = 3  # UNVERIFIED · DEGRADED(부활은 됐으나 세션 미검증) — 재시도 가치 있음
PHOENIX_EXIT_BREAKER = 5   # BREAKER_OPEN(크래시루프 정지) — 재시도 금지(폭주 방지·사람 승인)
PHOENIX_EXIT_CORRUPT = 6   # 손상 감지 / cys identity 불일치 — 재시도 금지(사람 개입)


def restore_exit_code(result):
    """run_restore 결과 dict → 프로세스 exit code(C1 계약). 최악 조건 우선(worst-wins):
    실 미부활(INCOMPLETE)이 세션 미검증(UNVERIFIED)보다, 손상/차단기가 그보다 상위다.
    result 가 dict 가 아니면(비정상) 보수적으로 FAILED(1) — 성공 오판 방지."""
    if not isinstance(result, dict):
        return PHOENIX_EXIT_FAILED
    verdict = result.get("phoenix_restore")
    completeness = result.get("completeness")
    if result.get("corruption"):
        return PHOENIX_EXIT_CORRUPT
    if verdict == "BREAKER_OPEN":
        return PHOENIX_EXIT_BREAKER
    # INCOMPLETE(readiness 기반 실 미부활)는 verdict 가 무엇이든 최우선 실패 신호 — 침묵 성공 금지.
    if verdict == "FAILED" or completeness == "INCOMPLETE":
        return PHOENIX_EXIT_FAILED
    if verdict in ("UNVERIFIED", "DEGRADED"):
        return PHOENIX_EXIT_DEGRADED
    # VERIFIED · VERIFIED_FRESH · NOOP · LEASE_HELD → 성공/무해(0)
    return PHOENIX_EXIT_OK

# ★Phase 11: 독약 세션(unresumable) fresh-spawn fallback — DRILL_LIVE_4 §15 수리.
#   완결성(Phase10)은 resume(세션핀) 기반 spawn 을 반복하는데, 세션이 독약(resume 불가·손상)이면 매 재시도가
#   동일하게 실패한다(DRILL_LIVE_4: claude --resume 워커만 부활 실패). 근본 = §3 원칙5 "N회 resume 실패→
#   무 resume(fresh) 기동 + 원장 재주입" 미구현. 수리: resume 재시도 소진 후에도 미부활이면, 해당 역할을
#   fresh(무 resume) 재기동으로 '강등'해 roster 100% 부활을 보장한다(독약 세션이 무한 재시도로 roster 를
#   막지 않게). fresh 전환은 저널·결과에 정직 명시(resumed→fresh — 세션 보존 실패를 숨기지 않는다).
#   resume 성공은 그대로 우선(fresh 는 최후수단). PHOENIX_POISON_FRESH_FALLBACK=0 이면 강등을 끈다(A/B 재현용).
POISON_FRESH_FALLBACK = os.environ.get("PHOENIX_POISON_FRESH_FALLBACK", "1") != "0"

CYS = None  # lazy resolve

# ★phoenix protocol version — Rust cys::pack::PHOENIX_PROTOCOL_VERSION 과 동기(identity 3중 대조·B1 self-test 표기).
PHOENIX_PROTOCOL_VERSION = "1"


# ------------------------------------------------------------------ 기반 유틸

def _which(name):
    import shutil
    return shutil.which(name)


# ★W1/B3·§5-1: 표준 설치 경로 폴백 후보(GUI/데몬 최소 PATH 에 cys 가 없을 때). 라이브 실증(2026-07-06):
#   GUI 기동 데몬 PATH=/usr/bin:/bin:/usr/sbin:/sbin 뿐 → /opt/homebrew/bin/cys 미탐색 → 리터럴 'cys'
#   FileNotFoundError → auto-restore exit 1 침묵사. 폴백은 identity-check 통과 시에만 채택한다.
_CYS_STD_PATHS = ["/opt/homebrew/bin/cys", "/usr/local/bin/cys"]


def _extract_version(text):
    """`cys --version`('cys 0.12.20') / daemon.version('0.12.20') 에서 x.y.z 를 추출(진단 로깅용)."""
    m = re.search(r"\d+\.\d+\.\d+", text or "")
    return m.group(0) if m else ""


# ★W1 identity 3중 대조 필드(§5-1② · codex R3 blocking): 버전 문자열 단일 대조는 shadowing 구멍
#   (같은 version 문자열의 다른 빌드·다른 embedded pack·다른 protocol 이 통과). 3필드 전건 일치를 요구한다.
_IDENTITY_FIELDS = ("build_id", "embedded_pack_hash", "protocol_version")

# ★W1(gate2): 해석된 cys 의 identity 검증 상태(결과 JSON `cys_identity` 로 노출).
#   "verified"=3중 대조 match · "degraded-unverified"=inconclusive 채택(데몬 미도달 등 — 검증 통과 아님·정직 분리).
#   mismatch 는 채택 자체가 없다(exit 6). None=미해석(직접 함수 호출 등).
_CYS_IDENTITY = None

# 재시도 파라미터(gate2 fix 2a·gemini): 데몬 소켓 기동 직후 status 미도달(inconclusive) 레이스 해소.
_IDENTITY_RETRY_TRIES = int(os.environ.get("PHOENIX_IDENTITY_RETRY_TRIES", "2"))
_IDENTITY_RETRY_SLEEP = float(os.environ.get("PHOENIX_IDENTITY_RETRY_SLEEP", "1.0"))


def _cys_self_identity(candidate):
    """후보 cys 자신의 3필드 self-report(`cys phoenix-identity` — 데몬 불요·컴파일타임 상수). 실패=None."""
    try:
        r = subprocess.run([candidate, "phoenix-identity"], capture_output=True, text=True, timeout=10)
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None
    try:
        return json.loads(r.stdout or "{}")
    except Exception:
        return None


def _daemon_identity(candidate, socket):
    """대상 데몬의 3필드(`cys status --json`.daemon). 서브프로세스 실패/미도달/파싱실패=None."""
    cmd = [candidate]
    if socket:
        cmd += ["--socket", socket]
    cmd += ["status", "--json"]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=12)
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None
    try:
        return (json.loads(r.stdout or "{}").get("daemon") or {})
    except Exception:
        return None


def _cys_identity_check(candidate, socket):
    """★3중 대조(§5-1②): 후보 self-report(phoenix-identity) ↔ 대상 데몬 status.daemon 의
    build_id·embedded_pack_hash·protocol_version 전건 일치 검증. 반환 (status, field, detail):
      · ('match', None, detail)        전 3필드 일치(같은 빌드·팩·프로토콜 확증)
      · ('mismatch', <field>, detail)  특정 필드 불일치/부재(다른 빌드/팩/프로토콜 — shadowing 차단, 어느 필드인지 명시)
      · ('inconclusive', reason, '')   self-report 또는 데몬 status 미도달로 대조 불가."""
    self_id = _cys_self_identity(candidate)
    if not self_id:
        return ("inconclusive", "self-report(phoenix-identity) 실패 — 구버전 cys(커맨드 부재) 의심", "")
    dmn = _daemon_identity(candidate, socket)
    if not dmn:
        return ("inconclusive", "데몬 status 미도달(대조 불가)", "")
    for f in _IDENTITY_FIELDS:
        sv, dv = self_id.get(f), dmn.get(f)
        if not sv or not dv:
            return ("mismatch", f, "self=%r daemon=%r(필드 부재 — 구버전/legacy 의심)" % (sv, dv))
        if sv != dv:
            return ("mismatch", f, "self=%r daemon=%r" % (sv, dv))
    return ("match", None, "build_id=%s proto=%s" % (self_id.get("build_id"), self_id.get("protocol_version")))


def _identity_with_retry(candidate, socket):
    """★gate2 fix 2a(gemini): inconclusive 는 데몬 소켓 기동 직후 status 미도달 레이스일 수 있으므로,
    1s 간격 재시도 후 재대조한다(mismatch·match 는 즉시 확정 — 재시도 불요). 최종 (status, field, detail)."""
    status, field, detail = _cys_identity_check(candidate, socket)
    tries = 0
    while status == "inconclusive" and tries < _IDENTITY_RETRY_TRIES:
        tries += 1
        time.sleep(_IDENTITY_RETRY_SLEEP)
        status, field, detail = _cys_identity_check(candidate, socket)
    return status, field, detail


def _accept_with_identity(socket, candidate, source_label, allow_inconclusive):
    """★gate2 fix 1·2: 후보 cys 를 3중 identity 계약으로 채택/거부한다(PHOENIX_CYS·PATH·표준경로 공통 경로).
      · match        → 채택(_CYS_IDENTITY='verified')
      · mismatch     → 어디서든 즉사(exit 6·불일치 필드 명시) — 적극적 모순은 availability 명분으로 통과 불가
      · inconclusive → allow_inconclusive 면 채택하되 **'검증 통과'가 아니라 degraded availability exception**으로
                       분리 기록(_CYS_IDENTITY='degraded-unverified'·저널+stderr). 아니면 exit 6.
    ★fix 2c(provenance): 'Rust 가 주입했다'는 출처를 env 토큰으로 증명하는 방식은 채택하지 않는다 — env 토큰은
       위조 가능해 shadowing 을 못 막는다(같은 토큰을 심으면 우회). 정직한 해법은 검증 실패를 숨기지 않고
       degraded 로 분리 보고하는 것(availability 는 지키되 'verified' 라 거짓말하지 않는다)."""
    global _CYS_IDENTITY
    status, field, detail = _identity_with_retry(candidate, socket)
    if status == "match":
        _CYS_IDENTITY = "verified"
        sys.stderr.write("[phoenix] cys 채택(%s): %s — 3중 identity match(%s)\n" % (source_label, candidate, detail))
        sys.stderr.flush()
        return candidate
    if status == "mismatch":
        _resolve_die(socket, "cys(%s=%s) identity 3중 대조 불일치 — 불일치 필드=%s (%s). "
                     "다른 빌드/팩/프로토콜 스큐 차단·exit 6." % (source_label, candidate, field, detail), 6)
    # inconclusive
    if not allow_inconclusive:
        _resolve_die(socket, "cys(%s=%s) identity inconclusive(%s) — positive proof 요구 경로라 채택 거부·exit 6. "
                     "PHOENIX_CYS 로 이 데몬과 동일 빌드의 cys 를 명시하라." % (source_label, candidate, field), 6)
    _CYS_IDENTITY = "degraded-unverified"
    _resolve_journal_mark(socket, "degraded",
                          "cys(%s=%s) identity inconclusive(%s) — availability 위해 채택하되 '검증 통과' 아님"
                          "(degraded-unverified 분리 보고)." % (source_label, candidate, field))
    sys.stderr.write("[phoenix] ★degraded availability exception: cys(%s=%s) identity **unverified(degraded)** — "
                     "재시도 후에도 inconclusive(%s). 채택하되 3중 대조 미완(검증 통과 아님).\n"
                     % (source_label, candidate, field))
    sys.stderr.flush()
    return candidate


def _resolve_journal_start(socket):
    """★gemini major: _resolve_cys die 전에 저널 뼈대(ticket=resolve·타임스탬프·stage=resolve_cys)를 먼저
    생성한다 — die 로 무단 종료해도 저널에 시도 이력이 남게 하는 관측 사각 제거. best-effort."""
    try:
        j = load_journal(socket, "resolve")
        jevent(j, "*", "resolve_cys", "start", "cys 실행 경로 해석 시작")
        save_journal(socket, "resolve", j)
    except Exception:
        pass


def _resolve_journal_mark(socket, status, msg):
    """resolve 저널에 이벤트 1건 기록(degraded 채택 등 — die 아닌 경로). best-effort."""
    try:
        j = load_journal(socket, "resolve")
        jevent(j, "*", "resolve_cys", status, msg[:280])
        save_journal(socket, "resolve", j)
    except Exception:
        pass


def _resolve_die(socket, msg, code):
    """★gemini major: resolve 실패를 저널에 기록한 뒤 die — 침묵사(저널 무이력 종료) 차단."""
    try:
        j = load_journal(socket, "resolve")
        jevent(j, "*", "resolve_cys", "fail", msg[:280])
        j.setdefault("resolve", {})["failed_at"] = _now()
        save_journal(socket, "resolve", j)
    except Exception:
        pass
    die(msg, code=code)


def _resolve_cys(socket):
    """cys 실행 경로 해석(§5-1 W1). 우선순위: PHOENIX_CYS > which('cys') > 표준경로 폴백.
    ★리터럴 'cys' 최종 폴백은 제거됨 — PATH 미해석 침묵 통과가 FileNotFoundError→exit 1 침묵사 근원(2026-07-06).
    ★gate2: 모든 실행 경로(PHOENIX_CYS·_which PATH·표준경로)에 3중 identity 계약을 적용한다 — 어느 경로든
       mismatch=exit 6. inconclusive 정책만 경로별 차등:
      · PHOENIX_CYS(명시)·_which PATH 후보 → X_OK + identity. inconclusive 는 재시도 후 degraded-unverified 로
        채택(분리 보고) — 하네스·격리·비데몬 서브커맨드에서 데몬 미도달 inconclusive 가 정상 존재하므로.
      · 표준경로 폴백(PATH·PHOENIX_CYS 모두 미해석) → positive proof 요구: **match 만** 채택(inconclusive=exit 6).
      · PHOENIX_STRICT_CYS=1 / 하네스(--socket·PHOENIX_FORBID_LIVE) → 표준경로 폴백 자체 금지(미해석=exit 6).
    실패·채택·degraded 전부 저널·stderr(→cysd phoenix-restore.log)에 기록한다."""
    _resolve_journal_start(socket)
    explicit = os.environ.get("PHOENIX_CYS")
    if explicit:
        if not (os.path.isfile(explicit) and os.access(explicit, os.X_OK)):
            _resolve_die(socket, "PHOENIX_CYS(%s) 실행 불가(파일 부재/실행권한 없음) — 무검증 수용 금지·exit 6." % explicit, 6)
        return _accept_with_identity(socket, explicit, "PHOENIX_CYS", allow_inconclusive=True)
    found = _which("cys")
    if found:
        # ★gate2 fix 1(BLOCKING): PATH 후보도 identity gate 를 우회하지 않는다(과거엔 곧바로 return).
        return _accept_with_identity(socket, found, "PATH(which)", allow_inconclusive=True)
    # 여기부터 표준경로 폴백 — PATH·PHOENIX_CYS 모두 cys 미해석.
    strict = os.environ.get("PHOENIX_STRICT_CYS") == "1"
    harness = bool(socket) or os.environ.get("PHOENIX_FORBID_LIVE") == "1"
    if strict:
        _resolve_die(socket, "PHOENIX_STRICT_CYS=1 이나 cys 를 PHOENIX_CYS/PATH 로 해석하지 못했다 — 표준경로 폴백 "
                     "금지(exit 6). Rust auto-restore 의 PHOENIX_CYS 주입이 누락됐을 수 있다(B3).", 6)
    if harness:
        _resolve_die(socket, "하네스 모드(--socket/PHOENIX_FORBID_LIVE)에서 cys 미해석 — 표준경로 폴백 금지"
                     "(테스트 독립성·exit 6). 격리 실행은 PHOENIX_CYS 로 대상 cys 를 명시하라.", 6)
    for c in _CYS_STD_PATHS:
        if os.path.isfile(c) and os.access(c, os.X_OK):
            # 표준경로는 PATH-미해석 추측 후보 — positive proof(match) 요구(inconclusive 도 거부).
            return _accept_with_identity(socket, c, "표준경로", allow_inconclusive=False)
    _resolve_die(socket, "cys 실행 경로 미해석(PHOENIX_CYS·PATH·표준경로 %s 모두 실패) — 리터럴 폴백 제거됨"
                 "(침묵사 방지·exit 6)." % _CYS_STD_PATHS, 6)


def die(msg, code=2):
    sys.stderr.write("[phoenix][FATAL] %s\n" % msg)
    sys.exit(code)


def log(msg):
    sys.stdout.write("[phoenix] %s\n" % msg)
    sys.stdout.flush()


def _emit_evt(evt_type, **fields):
    """★C2/C3(W3): 구조화 이벤트를 HUD·음성 버스로 best-effort 방출(EVENT_CONTRACT v2·형제 javis_event.py).
    손상 escalation·설명불가 축소 거부 등 운영자 가시성이 필요한 사건 전용. 절대 restore 판정을 죽이지 않는다
    (버스 미도달·도구 부재는 조용히 무시하되, 사건 자체는 호출측이 log()/저널로도 남긴다=이중 기록)."""
    mod = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_event.py")
    if not os.path.exists(mod):
        return False
    cmd = [sys.executable, mod, "emit", evt_type]
    for k, v in fields.items():
        cmd += ["--field", "%s=%s" % (k, v)]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=8)
        return r.returncode == 0
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return False


_SNAP_MOD = None


def _snap_mod():
    """형제 파일 javis_state_snapshot 모듈 로드·캐시. Windows 파이프→state_dir 매핑 규칙의 단일 소스이며
    (Rust state.rs 정합) phoenix 는 여기서 재사용한다(중복 구현 금지). mac 경로는 이 함수를 호출하지 않는다."""
    global _SNAP_MOD
    if _SNAP_MOD is None:
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        import javis_state_snapshot as _s
        _SNAP_MOD = _s
    return _SNAP_MOD


def _win_pipe_slug(socket):
    """Windows named pipe 슬러그 — 규칙 단일 소스는 javis_state_snapshot(중복 구현 금지)."""
    return _snap_mod()._win_pipe_slug(socket)


def _win_state_dir_for_socket(socket):
    """Windows 소켓(named pipe)→상태 디렉터리 — 규칙 단일 소스는 javis_state_snapshot(중복 구현 금지)."""
    return _snap_mod()._win_state_dir_for_socket(socket)


def state_dir_for(socket):
    """소켓 경로에서 데몬 상태 디렉터리를 파생. unix=소켓 부모(하네스 격리 계약과 동일) ·
    Windows=named pipe 슬러그 매핑(Rust state_dir 규칙 — 파이프엔 파일시스템 부모가 없다)."""
    if socket:
        if IS_WINDOWS:
            return _win_state_dir_for_socket(socket)
        return os.path.realpath(os.path.dirname(socket))
    # 소켓 미지정 시 라이브 기본
    return LIVE_STATE


def phoenix_home(socket):
    """저널·산출 루트 = <상태 디렉터리>/phoenix/. 라이브 상태 파일 자체는 절대 건드리지 않는다(phoenix/ 하위만).
    ★2026-07-05 박사님 지시 — 상시 라이브(기본 허용). opt-out PHOENIX_FORBID_LIVE=1 일 때만 라이브 대상 거부."""
    sd = state_dir_for(socket)
    if sd == LIVE_STATE and os.environ.get("PHOENIX_FORBID_LIVE") == "1":
        die("라이브 상태 디렉터리(%s) 대상 실행이 opt-out(PHOENIX_FORBID_LIVE=1)으로 거부됐다. "
            "격리 하네스에서 개발하려면 --socket 으로 하네스 소켓을 주거나 PHOENIX_FORBID_LIVE 를 해제하라." % sd)
    home = os.path.join(sd, "phoenix")
    os.makedirs(home, exist_ok=True)
    return home


class _CapR:
    """subprocess 결과 대역(returncode/stdout/stderr) — _run_capture 반환형."""
    def __init__(self, returncode=124, stdout="", stderr=""):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def _run_capture(cmd, env, timeout):
    """★Windows hang 방지 캡처(CI run 28733378888 케이스③ 12분 hang 수리). 파이프 대신 임시파일로 stdout/stderr 를
    받고 '직접 자식(cys.exe) 종료'만 기다린다 — 데몬을 스폰하지 않는 명령에도 안전하고, 스폰하는 명령(cys list)에서
    치명적이다: `cys list` 는 detached cysd 를 스폰하는데 Rust spawn 이 bInheritHandles=TRUE 라, 파이썬 subprocess 가
    cys.exe 에 물려준 inheritable 파이프 write 핸들을 cysd 가 상속해 계속 연다. 그러면 subprocess.run(capture_output=
    True)+communicate() 는 EOF 를 영영 못 받아 hang 하고, timeout 후 정리 communicate() 는 timeout 이 없어 무한 대기한다
    (=CI 12분 스텝 타임아웃). 임시파일 리다이렉트는 파이프 EOF 문제를 없애고, p.wait() 는 데몬이 아니라 cys.exe 종료만
    기다린다(cys.exe 는 lazy-spawn 후 곧 종료). 크로스플랫폼(테스트는 mac 에서도 가능). 반환=_CapR."""
    import tempfile
    of = tempfile.TemporaryFile()
    ef = tempfile.TemporaryFile()
    r = _CapR()
    try:
        try:
            p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=of, stderr=ef, env=env)
        except (FileNotFoundError, OSError) as e:
            # ★codex major(Windows 대칭): cys.exe 미해석/실행불가 → 비구조화 crash 대신 구조화 실패(rc=127).
            r.returncode = 127
            r.stderr = "cys 실행 불가(%s: %s) cmd=%r" % (type(e).__name__, e, cmd)
            return r
        try:
            p.wait(timeout=timeout)
            r.returncode = p.returncode
        except subprocess.TimeoutExpired:
            for killer in (lambda: p.kill(),):
                try:
                    killer()
                except Exception:
                    pass
            try:
                p.wait(timeout=5)
            except Exception:
                pass
            r.returncode = 124
        of.seek(0); ef.seek(0)
        r.stdout = of.read().decode("utf-8", "replace")
        se = ef.read().decode("utf-8", "replace")
        r.stderr = se if se else ("TIMEOUT %ss" % timeout if r.returncode == 124 else "")
    finally:
        of.close(); ef.close()
    return r


def cys(*args, socket=None, timeout=25):
    cmd = [CYS]
    if socket:
        cmd += ["--socket", socket]
    cmd += [str(a) for a in args]
    env = dict(os.environ)
    env.pop("AITERM_SOCKET", None)
    # ★Windows: 임시파일 캡처(_run_capture)로 detached cysd 파이프 상속 hang 회피. mac 은 기존 경로 유지(무회귀).
    if IS_WINDOWS:
        return _run_capture(cmd, env, timeout)
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
        return r
    except subprocess.TimeoutExpired as e:
        class _R:
            returncode = 124
            stdout = (e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")) if e.stdout else ""
            stderr = "TIMEOUT %ss" % timeout
        return _R()
    except (FileNotFoundError, OSError) as e:
        # ★codex major: CYS 해석 후에도 파일이 사라지거나 실행 불가면 여기서 비구조화 exit 1 crash 가 났다.
        #   구조화 실패(rc=127)로 강등해 상위 판정(스폰 실패→INCOMPLETE 등)이 정직히 흐르게 한다.
        class _R:
            returncode = 127
            stdout = ""
            stderr = "cys 실행 불가(%s: %s) cmd=%r" % (type(e).__name__, e, cmd)
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


def _atomic_write_json(path, obj, keep_bak=False):
    """tmp+fsync+replace+dir fsync — javis_state_snapshot 과 동일한 원자성 규약.
    ★os.replace(os.rename 아님): 대상이 이미 존재해도 원자적 덮어쓰기. POSIX는 rename과 동일 동작(mac 무변경)이고
    Windows는 os.rename 이 대상 존재 시 FileExistsError 로 죽어(저널은 반복 갱신됨) 반드시 replace 여야 한다.
    ★C2(W3): keep_bak=True 면 새 내용으로 덮기 **직전**에 현재 유효본을 `<path>.bak` 으로 보존한다 —
    손상 폴백 체인의 '직전 유효본' 소스. 현재 파일이 손상(파싱 실패)이면 백업하지 않는다(손상 전파 차단)."""
    d = os.path.dirname(path)
    tmp = os.path.join(d, ".tmp-%d-%s" % (os.getpid(), os.path.basename(path)))
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    if keep_bak and os.path.exists(path):
        try:
            if _roster_file_status(path) == "valid":  # 유효본만 .bak 으로 승격(손상 백업 금지)
                shutil.copy2(path, path + ".bak")
        except Exception:
            pass
    os.replace(tmp, path)
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

def _roster_file_status(path):
    """★C2 sentinel(W4 게이트): 상태파일 3분류 — 부재='missing'(fresh install 정상)·파싱성공='valid'·
    파싱실패='corrupt'. corrupt 를 missing 과 **구분**해, 손상 desired 가 빈 상태(fresh)로 위장해 통과하는
    silent-empty 를 차단하는 근거를 제공한다(현행 load_* 는 corrupt 를 missing 과 동일 빈집합 처리 — 그 구멍)."""
    if not os.path.exists(path):
        return "missing"
    try:
        with open(path) as f:
            json.load(f)
        return "valid"
    except Exception:
        return "corrupt"


def desired_roster_path(socket):
    return os.path.join(phoenix_home(socket), "desired_roster.json")


def load_desired_roster(socket):
    p = desired_roster_path(socket)
    if os.path.exists(p):
        try:
            d = json.load(open(p))
            return d.get("roster", {}), set(d.get("tombstones", []))
        except Exception:
            # ★C2: 손상은 침묵 빈집합이 아니라 로그(sentinel 이 run_restore 진입에서 부활 차단). 전체 복원 체인=W3.
            log("★C2 경고: desired_roster 파싱 실패(손상) — %s (빈 상태 반환은 run_restore sentinel 이 차단)" % p)
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
    # ★E3 주의(gemini W6): 여기는 단일 '최신 세대' 선택(heal 소스)이라 GC 처럼 union 보호가 불가능하다.
    #   mtime max 를 쓰면 cp/touch 오염 세대가 '최신'으로 오선택돼 stale topology 로 heal 할 위험이 있어,
    #   명목(이름) 기준을 유지한다(best-effort 소스 — 데이터 손실 아님). P2-5 의 실질 수리(오삭제 방어)는
    #   compute_gc 의 명목∪실효 union 이 담당한다.
    for g in reversed(gens):  # 이름(명목) 최신 세대부터
        tp = os.path.join(gen_root, g, "topology.json")
        if os.path.exists(tp):
            try:
                t = json.load(open(tp))
                return {e["role"]: e for e in t.get("entries", []) if e.get("role")}
            except Exception:
                continue
    return {}


def _snapshot_tombstones(socket):
    """직전 세대 스냅샷 topology.json 의 tombstones 필드(옵션A: 데몬이 여기에 묘비를 영속) — desired 손상 시
    묘비 복원 소스. 라이브 대상일 때만 유효(격리 하네스는 자기 topology 만). 반환 set|None(스냅샷 부재)."""
    if state_dir_for(socket) != LIVE_STATE:
        return None
    gen_root = os.path.join(HOME, ".cys", "state-generations")
    try:
        gens = sorted(g for g in os.listdir(gen_root) if re.match(r"\d{8}T\d{6}Z", g))
    except Exception:
        return None
    for g in reversed(gens):  # 명목 최신 세대부터(_snapshot_roster_entries 와 동일 규약)
        tp = os.path.join(gen_root, g, "topology.json")
        if os.path.exists(tp):
            try:
                t = json.load(open(tp))
                return set(x for x in t.get("tombstones", []) if isinstance(x, str))
            except Exception:
                continue
    return None


# ---------------- C2(W3): 손상 격리 + 폴백 체인 ----------------
# retention-critical(desired/dept) 파일이 파싱 실패(손상)면 침묵 빈-empty 통과 금지(W4 sentinel). W3 는 그 위에
# 완전한 복원 체인을 얹는다: .corrupt-<ts> 로 격리(최근 3개만·inode DoS 차단) → .bak(직전 유효본) → 세대 스냅샷 →
# 전부 불가면 unrecoverable(exit 6). 폴백으로 복원되면 degraded(묘비 불확실 → 부활 보류+escalation, fail-safe).

def _isolate_corrupt(path):
    """손상 파일을 `<path>.corrupt-<ts>` 로 격리(원본 자리 비움) 후 최근 3개만 유지·초과 prune(inode DoS 차단).
    반환: 격리된 경로(str)|None(격리 실패)."""
    ts = time.strftime("%Y%m%dT%H%M%S", time.gmtime()) + "-%06d" % (int(time.time() * 1e6) % 1000000)
    dst = "%s.corrupt-%s" % (path, ts)
    isolated = None
    try:
        os.replace(path, dst)  # 손상 원본을 자리에서 치워 다음 로드가 폴백/빈 상태로 가게
        isolated = dst
    except Exception:
        isolated = None
    _prune_corrupt(path, keep=3)
    return isolated


def _prune_corrupt(path, keep=3):
    """`<path>.corrupt-*` 격리본을 최근 keep 개만 남기고 오래된 것 제거(이름=시각순 정렬 규약)."""
    d = os.path.dirname(path) or "."
    prefix = os.path.basename(path) + ".corrupt-"
    try:
        cands = sorted(f for f in os.listdir(d) if f.startswith(prefix))
    except OSError:
        return
    for f in cands[:-keep] if len(cands) > keep else []:
        try:
            os.remove(os.path.join(d, f))
        except OSError:
            pass


def _recovered_provenance(path):
    """복구본에 영속된 degraded provenance(`recovered_from`)를 읽는다 — 있으면 dict{source,ts}, 없으면 None.
    ★codex W3 BLOCKING: 이 필드가 존재하는 한(파일이 유효 JSON 이어도) 부활 보류가 유지된다 — degraded 가
    복구 1회로 휘발하지 않고 auto-retry 전반에 걸쳐 영속(fail-safe). 해제=검증된-건강 topology replace 또는 --rebase."""
    try:
        with open(path, encoding="utf-8") as f:
            d = json.load(f)
        rf = d.get("recovered_from")
        return rf if isinstance(rf, dict) else None
    except Exception:
        return None


def _write_recovered(path, obj, source):
    """복구본을 provenance(`recovered_from`)와 함께 원자적으로 쓴다 — degraded 영속의 단일 지점."""
    obj = dict(obj)
    obj["recovered_from"] = {"source": source, "ts": _now()}
    obj.setdefault("updated_at", _now())
    _atomic_write_json(path, obj, keep_bak=False)


def _recover_retention_file(socket, path, kind):
    """retention-critical 파일 상태를 판정하고, 손상이면 폴백 체인으로 복원한다(복원분을 path 에 되써 후속
    load_* 가 정상 읽게 한다). ★codex W3 BLOCKING: 복구본에 `recovered_from` provenance 를 영속해 degraded
    부활 보류가 1회로 휘발하지 않게 한다(다음 auto-retry 도 동일 hold). 반환 dict:
      · {'status':'valid'}              정상(provenance 없음)
      · {'status':'missing'}            부재(fresh install 정상 — hard-fail 아님)
      · {'status':'degraded','source':..,'isolated':..,'pending':bool}  손상→폴백 복원 또는 valid+provenance(보류 유지)
      · {'status':'unrecoverable','isolated':..}          손상→전 폴백 실패(exit 6)"""
    st = _roster_file_status(path)
    if st == "missing":
        return {"status": "missing"}
    if st == "valid":
        # ★영속된 degraded: 유효 JSON 이어도 provenance 가 있으면 아직 보류 상태(전 auto-retry 지속).
        prov = _recovered_provenance(path)
        if prov:
            return {"status": "degraded", "source": prov.get("source"), "pending": True, "isolated": None}
        return {"status": "valid"}
    # corrupt — 격리 후 폴백 체인
    isolated = _isolate_corrupt(path)
    bak = path + ".bak"
    if _roster_file_status(bak) == "valid":
        try:
            with open(bak, encoding="utf-8") as f:
                bak_obj = json.load(f)
            _write_recovered(path, bak_obj, "bak")
            return {"status": "degraded", "source": "bak", "isolated": isolated}
        except Exception:
            pass
    if kind == "desired_roster":
        snap_r = _snapshot_roster_entries(socket)
        snap_t = _snapshot_tombstones(socket)
        if snap_r or snap_t:
            try:
                _write_recovered(path, {"roster": snap_r, "tombstones": sorted(snap_t or [])}, "snapshot")
                return {"status": "degraded", "source": "snapshot", "isolated": isolated}
            except Exception:
                pass
    elif kind == "dept_roster":
        # 부서는 glob(cys-dept-*)∪registry 로 재발견 가능(discover_depts) — 빈 상태로 복원하면 observe 가 roster 를
        #   재구성한다. 단 묘비는 스냅샷 소스가 없어 소실 → degraded(부활 보류). '전부 불가' 아님(재발견 경로 존재).
        try:
            _write_recovered(path, {"roster": {}, "tombstones": []}, "discovery")
            return {"status": "degraded", "source": "discovery", "isolated": isolated}
        except Exception:
            pass
    return {"status": "unrecoverable", "isolated": isolated}


# ---------------- A-S3 tombstone intent 저널 (다운타임 폴백) ----------------
# 데몬이 topology 묘비 유일 작성자(옵션A). CLI 폐역은 데몬 RPC 경유가 정석이나, 데몬 다운타임엔 append-only
# intent 저널에 기록하고 observe 가 replace **이전** 멱등 적용 → 데몬 복귀 후 RPC 재동기 → **갱신 rev 담긴
# topology.json 디스크 영속을 phoenix 가 로드 확인한 후에만 절단**(gemini R3: RPC 응답 시점 절단 금지·TOCTOU).

def _intent_journal_path(socket):
    return os.path.join(phoenix_home(socket), "tombstone-intents.jsonl")


def _append_tombstone_intent(socket, role, remove):
    """다운타임 폴백 — intent 를 append-only jsonl 에 배타 락으로 기록(C2 정책: 원자 append·부분절단 내성).
    ★D2(W5): 락을 unix flock·Windows msvcrt 통합(_try_lock_nb, best-effort NB) — 과거 Windows 무락(P1-8)을
    제거. 락 실패해도 단문 append(원자성)·corrupt-line-skip 내성으로 진행(가용성). a+ 로 열어 Windows msvcrt
    byte0 락 영역이 프로세스 간 일치하게 한다(write 는 append 모드라 end 로 간다)."""
    p = _intent_journal_path(socket)
    line = json.dumps({"op": "remove" if remove else "add", "role": role, "ts": _now()}, ensure_ascii=False)
    try:
        with open(p, "a+", encoding="utf-8") as f:
            _try_lock_nb(f)  # best-effort 배타 락(unix flock·Windows msvcrt.locking) — 실패해도 append 진행
            f.write(line + "\n")
    except Exception:
        pass


def _read_tombstone_intents(socket):
    """intent jsonl 파싱 → [{op, role, ts}]. 손상 라인은 skip(부분절단·조작 내성)."""
    p = _intent_journal_path(socket)
    out = []
    if os.path.exists(p):
        try:
            with open(p, encoding="utf-8") as f:
                for ln in f:
                    ln = ln.strip()
                    if not ln:
                        continue
                    try:
                        out.append(json.loads(ln))
                    except Exception:
                        continue
        except Exception:
            pass
    return out


def _apply_intents_to_tombstones(intents, tombstones):
    """intent 를 기록 순서대로 tombstones 집합에 멱등 적용(add→추가·remove→제거)."""
    for it in intents:
        role = it.get("role")
        if not role:
            continue
        if it.get("op") == "remove":
            tombstones.discard(role)
        else:
            tombstones.add(role)
    return tombstones


def _truncate_tombstone_intents(socket):
    """intent 저널 절단(소화 완료). ★gemini R3: 호출측이 topology.json 디스크 영속(갱신 rev·묘비 반영) 확인 후에만."""
    p = _intent_journal_path(socket)
    try:
        if os.path.exists(p):
            os.remove(p)
    except Exception:
        pass


def _resync_intents_if_daemon_up(socket, intents):
    """★A-S3: 데몬 복귀 시 미소화 intent 를 RPC(cys tombstone) 로 재동기하고, topology.json 을 다시 로드해
    intent 효과가 **디스크에 영속**(add 역할이 topology 묘비에 존재·remove 역할이 부재)됐음을 확인한 후에만
    절단한다(gemini R3 TOCTOU: RPC 응답 직후 데몬이 topology 영속 전 크래시하면 묘비 소실 — 그 전에 절단 금지).
    반환: True(절단됨)/False(보류)."""
    if not intents:
        return False
    for it in intents:
        role = it.get("role")
        if not role:
            continue
        args = ["tombstone", role]
        if it.get("op") == "remove":
            args.append("--remove")
        r = cys(*args, socket=socket, timeout=12)
        if getattr(r, "returncode", 1) != 0:
            return False  # 데몬 미도달/RPC 실패 — 다음 observe 에서 재시도(절단 보류)
    # ★영속 확인: topology.json 재로드 → intent 효과가 디스크에 반영됐는지.
    topo2 = read_topology(socket)
    if "_error" in topo2 or "schema_version" not in topo2:
        return False  # 손상/legacy — 확인 불가, 절단 보류
    topo_tombs = set(t for t in topo2.get("tombstones", []) if isinstance(t, str))
    for it in intents:
        role = it.get("role")
        if not role:
            continue
        if it.get("op") == "remove" and role in topo_tombs:
            return False  # remove 미반영
        if it.get("op") != "remove" and role not in topo_tombs:
            return False  # add 미반영
    _truncate_tombstone_intents(socket)  # 디스크 영속 확인 완료 → 절단
    return True


def _ephemeral_verdict(role, entry=None):
    """★P2-1 legacy migration 결정표(codex W2 minor): 세 판정 반환.
    · 'ephemeral' = 명백한 일회성 → quarantine(부활 대상 제외·엔트리 제거). 근거: source 플래그
      (source∈{fresh,ephemeral}·데몬 기록) 또는 이름의 '-fresh-' **부분문자열**(worker-fresh-<epoch> 등).
    · 'ambiguous' = 비표준 변형('fresh' 포함하나 '-fresh-' 패턴 아님) → 부활 보류 + escalation(사람 판단).
    · 'normal' = metadata 無 비-fresh → 보존·정상 부활(오판 금지)."""
    e = entry or {}
    if e.get("source") in ("fresh", "ephemeral"):
        return "ephemeral"
    r = str(role or "")
    if "-fresh-" in r:
        return "ephemeral"
    if "fresh" in r.lower():
        return "ambiguous"
    return "normal"


def _is_ephemeral_role(role, entry=None):
    """★P2-1: 명백한 ephemeral(quarantine 대상)인가. ambiguous/normal 은 False(관측 병합 단계 제외 판정용)."""
    return _ephemeral_verdict(role, entry) == "ephemeral"


def _try_lock_nb(f):
    """★D2(W5): 비차단 배타 락 1회 시도(unix·Windows 통합). 반환 True(획득)·False(다른 보유)·None(락 기구
    미가용=fail-open). unix=fcntl.flock(LOCK_EX|NB) · Windows=msvcrt.locking(LK_NBLCK, byte0). 둘 다 프로세스
    사망 시 OS 가 자동 해제(stale lock 없음). Windows 는 seek(0)로 동일 바이트 영역을 잠가 상호배제를 보장한다."""
    if IS_WINDOWS:
        try:
            import msvcrt
        except Exception:
            return None
        try:
            f.seek(0)
            msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
            return True
        except OSError:
            return False          # 다른 프로세스가 byte0 보유 — 경합
        except Exception:
            return None           # 예기치 못한 실패 → fail-open
    try:
        import fcntl
    except Exception:
        return None
    try:
        fcntl.flock(f.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        return True
    except OSError:
        return False
    except Exception:
        return None


def _acquire_roster_lock(socket, tag="roster", tries=40, sleep=0.05):
    """★P1-7(W3): desired/dept read-modify-write 직렬화 lock. reconcile·roster·inherit·restore 가 동시에
    desired/dept 를 RMW 하면 lost update 가 난다 — `<phoenix_home>/<tag>.lock` 에 비차단 배타 락을 tries×sleep
    동안 재시도해 직렬화한다. ★D2(W5): unix flock·Windows msvcrt 통합(_try_lock_nb) — 과거 Windows fail-open
    (P1-8)을 제거. 끝내 못 잡거나 락 기구 미가용이면 fail-open(핸들 보유·경고) — 무한 블록/행 금지. 반환 handle|None."""
    try:
        p = os.path.join(phoenix_home(socket), "%s.lock" % tag)
        f = open(p, "a+")  # 무truncate·생성·byte0 락 대상(Windows msvcrt 영역 일치)
    except Exception:
        return None
    for _ in range(max(1, tries)):
        r = _try_lock_nb(f)
        if r is True:
            return f
        if r is None:
            return f  # 락 기구 미가용 → fail-open(가용성)
        time.sleep(sleep)  # r is False(다른 보유) → 재시도
    log("★P1-7: roster RMW lock(%s) 확보 실패(경합 지속) — fail-open 진행(가용성)." % tag)
    return f  # 못 잡아도 핸들 보유(fail-open·무한 블록 금지)


def observe_and_persist_roster(socket, rebase=False):
    """desired 로스터 관측·영속의 공개 진입점. ★P1-7(W3): RMW 를 roster.lock flock 으로 직렬화(lost update 차단)."""
    _lock = _acquire_roster_lock(socket, "roster")
    try:
        return _observe_and_persist_roster_locked(socket, rebase)
    finally:
        _release_lease(_lock)


def _observe_and_persist_roster_locked(socket, rebase=False):
    """현재 관측(topology + 세대 스냅샷)을 desired 로스터에 영속하고 (roster, tombstones) 반환.
    ★C3(W3): rebase=True 면 '설명-가능-축소 불변식'을 1회 우회해 현재 관측을 강제 수용한다(운영자 `phoenix
    roster --rebase` 전용 — 정당한 축소로 판단한 사람이 교착을 푸는 경로).
    ★W2 옵션A(A-S1): 데몬 topology.json = 묘비 유일 진실. desired 묘비 = 미러(조건부 replace). 데몬의
    claim/create 해제(topology 묘비 감소)가 자동으로 desired 에 흘러 제3겹(수동 tombstone --remove 불요)이 소멸한다.
    replace 게이트: 파싱 성공 + schema_version 마커 실존 + tombstones_rev ≥ 마지막으로 본 rev. 역행은 gemini R3
    rebase(epoch 변경/rev=0)로만 허용하고, 정당근거 없는 역행(부분절단·조작)은 replace 생략+desired 보존. legacy
    (마커 부재)는 add-merge 유지+경고+부활 정상 진행. 손상(파싱 실패)은 replace 생략(전체 폴백은 W3/governance)."""
    roster, tombstones = load_desired_roster(socket)
    # 이전에 본 rev/epoch(desired_roster.json) — A-S1 조건부 replace 판정 기준.
    prev = {}
    try:
        _dp = desired_roster_path(socket)
        if os.path.exists(_dp):
            prev = json.load(open(_dp))
    except Exception:
        prev = {}
    last_seen_rev = prev.get("tombstones_rev")
    last_seen_epoch = prev.get("daemon_epoch")
    cur_epoch = _ACTIVE_EPOCH if _ACTIVE_EPOCH is not None else get_boot_epoch(socket)
    # ★A-S2: 태그 = 가변 소켓 경로가 아니라 canonical state dir(실경로) — 하네스 임시 소켓의 태그 불일치 DoS 회피
    #   (gemini R2). desired 파일이 다른 state dir 에서 복사/이동돼 온 이물(태그 불일치)이면 write 를 거부해
    #   교차오염을 막는다(파일은 늘 state_dir_for(socket) 하위이므로 정상 경로는 항상 일치·edge 방어).
    cur_tag = os.path.realpath(state_dir_for(socket))
    _foreign = bool(prev.get("state_dir_tag")) and prev.get("state_dir_tag") != cur_tag
    if _foreign:
        log("★A-S2: desired_roster 의 state_dir_tag(%s)가 현재 canonical(%s)과 불일치 — 이물 파일, write 거부(교차오염 차단)."
            % (prev.get("state_dir_tag"), cur_tag))

    # ★A-S3: 데몬 복귀 시 미소화 intent 를 RPC 재동기·topology 디스크 영속 확인 후에만 절단(gemini R3 TOCTOU).
    #   재동기 전에 intent 를 읽어둔다 — 절단됐으면 이후 replace 는 topology(이미 반영)만 보면 되고,
    #   절단 보류면 아래에서 replace 결과에 intent 를 멱등 재적용(다운타임 CLI 폐역이 즉시 반영되게).
    _intents = _read_tombstone_intents(socket)
    _intents_truncated = _resync_intents_if_daemon_up(socket, _intents)

    topo = read_topology(socket)
    has_marker = "schema_version" in topo               # A-S1 마커(신 데몬)
    topo_rev = topo.get("tombstones_rev")
    topo_tombs = set(t for t in topo.get("tombstones", []) if isinstance(t, str))
    new_rev = last_seen_rev

    # ★codex W3 BLOCKING(2): degraded provenance 해제 판정 — 검증된-건강 topology replace(rev 마커 확정)가
    #   묘비 집합을 확정하는 시점에만 True. 그때 복구본의 recovered_from 을 제거(부활 보류 자동 해제).
    _healthy_replace = False
    # ★버그 수정: A-S1 rev-rollback rebase 는 함수 파라미터 rebase(운영자 --rebase)와 **별개 개념**이다 —
    #   지역명을 _rev_rebase 로 분리한다(과거 동명 재대입이 has_marker 경로에서 운영자 rebase 를 무력화했음).
    if "_error" in topo:
        # 손상 topology → replace 금지(desired 보존). 전체 폴백 체인(.bak·스냅샷·degraded)은 W3/governance(P0-3).
        log("★A-S1: topology 손상(%s) — 묘비 replace 생략, desired 보존." % topo.get("_error"))
    elif has_marker and topo_rev is not None:
        _rev_rebase = False
        if last_seen_rev is not None and topo_rev < last_seen_rev:
            # 역행 — gemini R3: epoch 변경 또는 rev=0(리셋/fresh install/.bak) 만 정당한 rebase.
            if topo_rev == 0 or (cur_epoch is not None and cur_epoch != last_seen_epoch):
                _rev_rebase = True
        if last_seen_rev is None or topo_rev >= last_seen_rev or _rev_rebase:
            # ★옵션A 조건부 replace: 데몬 topology 묘비를 desired 에 **그대로 대입**(add-merge 아님).
            #   데몬 해제가 자동 반영 → 제3겹 소멸. rebase 시엔 정당한 역행을 손상으로 오판하지 않는다.
            tombstones = set(topo_tombs)
            new_rev = topo_rev
            _healthy_replace = True  # 건강 데몬이 묘비 집합 확정 → degraded provenance 해제 근거
            if _rev_rebase:
                log("★A-S1 rebase: topology rev 정당 역행(%s<%s·epoch변경/rev0) — 강제 rebase 후 replace."
                    % (topo_rev, last_seen_rev))
        else:
            log("★A-S1: topology rev 역행(%s<%s·정당근거 없음) — replace 생략, desired 보존(부분절단/조작 의심)."
                % (topo_rev, last_seen_rev))
    else:
        # legacy topology(마커 부재)=손상 아님 → 조건부 replace 생략·add-merge 하위호환·경고·부활 정상 진행.
        for t in topo_tombs:
            tombstones.add(t)
        log("★A-S1: legacy topology(schema_version 부재) — 조건부 replace 생략, add-merge 유지·부활 정상 진행.")

    # ★A-S3: 미절단(=데몬이 아직 소화 못한) intent 를 replace 결과에 멱등 재적용 — 다운타임 CLI 폐역/해제가
    #   데몬 복귀·영속 확인 전에도 즉시 반영되게(절단됐으면 topology 에 이미 반영돼 재적용 무해).
    if _intents and not _intents_truncated:
        _apply_intents_to_tombstones(_intents, tombstones)

    # 우선순위: 기존 desired < 세대 스냅샷 < 현재 topology (최신 관측이 메타를 갱신). ★P2-1: ephemeral 제외(미래 유입 차단).
    for role, e in _snapshot_roster_entries(socket).items():
        if not _is_ephemeral_role(role, e):
            roster[role] = e
    for e in topo.get("entries", []):
        if e.get("role") and not _is_ephemeral_role(e["role"], e):
            roster[e["role"]] = e
    # ★Phase 7: 라이브 role 직접 병합 — claim-role 즉시 자동 등재(topology 영속 지연/침식 무관).
    for role, _surfs in live_role_surfaces(socket).items():
        if role and role != "-" and not _is_ephemeral_role(role):
            roster.setdefault(role, {"role": role})
    # ★P2-1 legacy 마이그레이션 결정표(codex W2): ephemeral=quarantine(엔트리 제거·일회성) · ambiguous=부활 보류+
    #   escalation(엔트리 보존·tombstones 로 target 제외해 사람 판단까지 부활 안 함·untomb 로 재편입 가능).
    for role in list(roster.keys()):
        v = _ephemeral_verdict(role, roster.get(role))
        if v == "ephemeral":
            roster.pop(role, None)
        elif v == "ambiguous":
            tombstones.add(role)  # 부활 보류(엔트리 보존·target 제외)
            log("★P2-1 escalation: 비표준 ephemeral 변형(%s) 판정 애매 — 부활 보류(사람 판단·untomb 로 재편입)." % role)
    # ★codex W2 BLOCKING: 묘비 역할의 **roster 엔트리는 보존**한다(pop 금지). 배제는 부활 target 산정 시점에
    #   tombstones 대조로 수행(run_restore·cmd_reconcile). 엔트리·메타(session/cwd/agent)가 남아 있어야 데몬이
    #   묘비를 해제(untomb)하면 즉시 부활 가능하다 — 과거 pop 은 untomb 를 무의미하게 만드는 함정(false-green).
    #   ※ 기존 desired 파일에 이미 pop 된 역할은 세대 스냅샷·live 병합이 자연 치유한다(신규 의미론은 전향 적용).
    # ★C3 설명-가능-축소 불변식(W3): desired 는 관측으로만 늘고 묘비(OwnerClose)·ephemeral 로만 준다. 직전
    #   영속본 대비 '설명되지 않는 축소'(사라진 역할이 tombstone 도 ephemeral 도 아님)는 손상/조작/버그 신호다 —
    #   영구 교착 대신 이 write 1회 거부(직전 상태 보존)+EVT/저널 escalation → 다음 관측 사이클이 재평가한다
    #   (gemini R2 TOCTOU: 데몬의 엔트리 제거↔묘비 등록 지연이 만든 오판은 1사이클 유예로 자연 해소). 정당한 축소는
    #   운영자 `phoenix roster --rebase`(rebase=True)로 강제 수용.
    _prev_roster = prev.get("roster") or {}
    _lost = set(_prev_roster.keys()) - set(roster.keys())
    _unexplained = sorted(r for r in _lost
                          if r not in tombstones
                          and _ephemeral_verdict(r, _prev_roster.get(r)) != "ephemeral")
    if _unexplained and not rebase and not _foreign:
        log("★C3 설명불가 축소 거부: 직전 대비 사라진 역할 %s 이 묘비·ephemeral 로 설명 안 됨 — 이 write 1회 "
            "거부(직전 상태 보존·다음 사이클 재평가). 정당하면 `phoenix roster --rebase`." % _unexplained)
        _emit_evt("agent.error", agent="phoenix",
                  summary="C3 설명불가 desired 축소 거부(%s) — write 보류, --rebase 로 수용" % _unexplained)
        # 직전 영속본을 그대로 반환(축소를 확정하지 않음) — 다운스트림(target 산정)이 좋은 이전 상태를 쓴다.
        return dict(_prev_roster), set(t for t in (prev.get("tombstones") or []) if isinstance(t, str))
    # ★A-S2: 이물(state_dir_tag 불일치) 파일이면 write 거부 — 교차오염 차단. 정상 경로는 태그를 기록해 영속.
    #   ★C2(W3): keep_bak=True 로 직전 유효본을 .bak 에 보존(손상 폴백 소스). 쓰기 실패는 침묵(except:pass) 금지 →
    #   log+EVT(P2-8: 영속 실패 무보고 차단).
    if not _foreign:
        _out = {"roster": roster, "tombstones": sorted(tombstones),
                "tombstones_rev": new_rev, "daemon_epoch": cur_epoch,
                "state_dir_tag": cur_tag, "updated_at": _now()}
        # ★codex W3 BLOCKING(1): degraded provenance 영속 — 복구본의 recovered_from 을 다음 write 로 이월해
        #   부활 보류가 auto-retry 전반에 지속되게 한다. 해제 조건: 검증된-건강 topology replace(_healthy_replace)
        #   또는 운영자 --rebase(rebase) — 그때만 provenance 를 떨궈 정상 복귀. 그 외에는 계속 보류(fail-safe).
        _prev_prov = prev.get("recovered_from")
        if isinstance(_prev_prov, dict) and not _healthy_replace and not rebase:
            _out["recovered_from"] = _prev_prov
        try:
            _atomic_write_json(desired_roster_path(socket), _out, keep_bak=True)
        except Exception as _e:
            log("★C3/P2-8: desired_roster 원자쓰기 실패(%s: %s) — 침묵 삼킴 금지, escalation." % (type(_e).__name__, _e))
            _emit_evt("agent.error", agent="phoenix",
                      summary="desired_roster 영속 실패(%s) — 상태 갱신 미반영" % type(_e).__name__)
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


def observe_and_persist_depts(socket, rebase=False):
    """발견된 부서를 dept_roster 에 단조 병합·영속(침식 면역). (roster, tombstones) 반환.
    ★phoenix 소유 dept_roster.json 에만 쓴다 — 실 depts.json 무접촉. tombstone 된 부서는 제외(의도적 폐역).
    ★P1-7(W3): dept RMW 도 dept.lock flock 으로 직렬화(lost update 차단·role 경로와 대칭).
    ★C2(W3): rebase=True 면 degraded provenance(recovered_from) 를 제거해 부활 보류를 해제(운영자 --rebase)."""
    _lock = _acquire_roster_lock(socket, "dept")
    try:
        return _observe_and_persist_depts_locked(socket, rebase)
    finally:
        _release_lease(_lock)


def _observe_and_persist_depts_locked(socket, rebase=False):
    roster, tombstones = load_dept_roster(socket)
    # ★codex W3 BLOCKING(1): dept degraded provenance 이월 — 손상 복구 후 부활 보류가 auto-retry 전반 지속.
    #   dept 는 topology 건강-replace 경로가 없으므로(glob 재발견) 해제=운영자 --rebase 만(fail-safe 기본).
    _prev_prov = None
    try:
        _dp = dept_roster_path(socket)
        if os.path.exists(_dp):
            _prev_prov = json.load(open(_dp)).get("recovered_from")
    except Exception:
        _prev_prov = None
    for dept, info in discover_depts().items():
        cur = roster.get(dept, {})
        cur.update(info)
        roster[dept] = cur
    # ★codex W2 BLOCKING(dept 동형): 묘비 부서의 roster 엔트리 보존(pop 금지) — untomb 즉시 부활. 배제는
    #   소비 시점(target 산정)에 tombstones 대조로 수행한다(role 경로와 대칭).
    _out = {"roster": roster, "tombstones": sorted(tombstones), "updated_at": _now()}
    if isinstance(_prev_prov, dict) and not rebase:
        _out["recovered_from"] = _prev_prov  # 보류 지속(해제=--rebase)
    try:
        _atomic_write_json(dept_roster_path(socket), _out, keep_bak=True)  # ★C2(W3): .bak 유지(손상 폴백 소스)
    except Exception as _e:
        log("★P2-8: dept_roster 원자쓰기 실패(%s: %s) — 침묵 삼킴 금지, escalation." % (type(_e).__name__, _e))
        _emit_evt("agent.error", agent="phoenix",
                  summary="dept_roster 영속 실패(%s)" % type(_e).__name__)
    return roster, tombstones


def _status_json(socket):
    """★C5(W3): `cys status --json` 파싱 결과(dict) — 구조화 liveness/readiness 단일 소스. 실패=None."""
    r = cys("status", "--json", socket=socket, timeout=12)
    if getattr(r, "returncode", 1) != 0:
        return None
    try:
        return json.loads(r.stdout or "{}")
    except Exception:
        return None


def _live_role_surfaces_from_list(socket):
    """폴백: `cys list` 화면 정규식 파싱(status --json 미도달 시). 반환 (dict, known:bool).
    ★codex W3 BLOCKING(3): 이 폴백이 P1-6(정규식 취약)을 되살리지 않도록 **구조 유효성**을 판정한다 — list 가
    rc≠0 이거나, stdout 이 비어있지 않은데 파싱 0건이거나, `surface:` 로 시작하는 행 중 4필드 정규식 미매칭이
    하나라도 있으면 known=False(구조 드리프트). known=False 는 '전원 사망' 추정 금지 신호로, 호출측이 부활을
    보류한다(대량 오스폰 방지 fail-safe). 구조 유효(빈 출력=surface 0, 또는 surface 행 전부 매칭)일 때만 known=True."""
    r = cys("list", socket=socket, timeout=12)
    txt = r.stdout or ""
    out = {}
    lines = [ln for ln in txt.splitlines() if ln.strip()]
    surface_lines = [ln for ln in lines if ln.strip().startswith("surface:")]
    matched = 0
    for line in lines:
        m = re.match(r"(surface:\d+)\s+role=(\S+)\s+pid=(\d+)\s+exited=(\S+)", line)
        if m:
            matched += 1
            ref, role, pid, exited = m.group(1), m.group(2), int(m.group(3)), m.group(4)
            out.setdefault(role, []).append({"surface": ref, "pid": pid, "exited": exited == "true"})
    # ★codex W3(3) 정밀: known=False 는 **구조 드리프트**(데몬이 살아 데이터를 주는데 형식이 바뀐 오파싱)로 한정한다
    #   — 즉 stdout 이 비어있지 않은데 파싱 0건, 또는 `surface:` 행 일부 미매칭. rc≠0·빈 출력(데몬 미도달/콜드부트)은
    #   '살아있는 surface 0'이 정당한 관측이므로 known=True(그 경우 부활은 정상 진행 = phoenix 본연). 형식 드리프트만
    #   전원 사망 오판 → 대량 스폰 위험이라 보류시킨다.
    known = True
    if lines and matched == 0:
        known = False                                   # 비어있지 않은데 0건 = 구조 드리프트
    if surface_lines and matched < len(surface_lines):
        known = False                                   # surface: 행 일부 미매칭 = 부분 드리프트
    return out, known


# ★codex W3(3): 직전 liveness 관측의 신뢰도(구조 유효성). live_role_surfaces 가 실제 소스를 파싱할 때 갱신되고,
#   테스트가 live_role_surfaces 를 통째로 몽키패치하면(=실 소스 우회) 직전값(기본 True=신뢰)을 유지한다.
_LAST_LIVENESS_KNOWN = True


def _live_surfaces_raw(socket):
    """실 liveness 파싱 — status --json(구조화) 우선·실패 시 list 폴백. _LAST_LIVENESS_KNOWN 을 갱신하고 dict 반환."""
    global _LAST_LIVENESS_KNOWN
    st = _status_json(socket)
    if st is None:
        out, known = _live_role_surfaces_from_list(socket)
        _LAST_LIVENESS_KNOWN = known
        return out
    out = {}
    for s in st.get("surfaces", []):
        # 미claim surface 는 status --json 에서 role=null — 구 `cys list` 규약(미claim="-")과 일치시켜 보존한다.
        #   role 무관 잔재(C6 exited 회수 등)를 놓치지 않기 위함(role=null skip 은 P1-6 회귀).
        role = s.get("role") or "-"
        out.setdefault(role, []).append({
            "surface": s.get("surface_ref"),
            "pid": None,  # OS pid 는 status --json 미노출(보고 전용 필드 — liveness 판정엔 exited 사용)
            "exited": bool(s.get("exited")),
            "agent_alive": s.get("agent_alive"),  # ★C5 구조화 readiness ack 신호(P1-10)
            # ★SEAT: 데몬이 커널 사실(자손 프로세스 유무)로 판정한 좌석 점유 — occupied|empty|unknown.
            #   **소비만 한다**(판정 재구현 금지 — phoenix 가 자기 규칙을 따로 두면 데몬과 이원화돼
            #   오늘의 결함[빈 좌석을 생존으로 오인]이 다른 얼굴로 재발한다). 구 데몬은 이 키가 없어
            #   None → _alive() 가 종전(exited 만) 규칙으로 degrade 한다(하위호환·악화 0).
            "seat": s.get("seat"),
        })
    _LAST_LIVENESS_KNOWN = True
    return out


def live_role_surfaces(socket):
    """현재 살아있는 surface들의 role→[{surface, pid, exited, agent_alive}] 실측(dict만).
    ★C5/P1-6(W3): liveness 근원을 화면 정규식(`cys list`)에서 **구조화 소스**(`cys status --json`.surfaces)로
    전환. 보고/관측·부활 target 산정 공통 진입점(몽키패치 지점)."""
    return _live_surfaces_raw(socket)


def _live_role_surfaces_checked(socket):
    """부활 target 산정용 liveness (dict, known:bool). live_role_surfaces 를 경유해(몽키패치 존중) dict 를 얻고,
    직전 파싱의 구조 신뢰도(_LAST_LIVENESS_KNOWN)를 함께 반환한다 — known=False 면 호출측이 부활 보류(fail-safe)."""
    d = live_role_surfaces(socket)
    return d, _LAST_LIVENESS_KNOWN


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
        except Exception as _e:
            # ★C2 2단계 보조상태(W3): 저널은 retention-critical 이 아니다(단계 진행 캐시 — 소실 시 재수행). 손상 시
            #   hard-fail 하지 않고 격리(.corrupt-<ts>·최근3)+경고 후 fresh 로 시작한다. 단 '침묵 삼킴'(try:pass)은
            #   금지 — 재개가 불가능해졌음을 log/EVT 로 가시화(gemini W1-gate3: resolve 저널 침묵 제거).
            isolated = _isolate_corrupt(p)
            log("★C2 보조상태: journal(%s) 손상 — 격리(%s) 후 fresh 시작(단계 재수행). 침묵 아님."
                % (ticket_id, isolated))
            _emit_evt("agent.error", agent="phoenix",
                      summary="phoenix journal(%s) 손상 — fresh 재시작(단계 진행 소실)" % ticket_id)
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
    """이번 restore 시도를 기록하고, T초 내 N회 이상이면 (open=True, 최근 시도 리스트) 반환.
    ★C2/P2-6(W3): 보조상태(breaker)는 retention-critical 이 아니므로 손상 시 hard-fail 하지 않고 격리+경고 후
    빈 카운트로 재시작한다 — 단 '침묵 리셋'은 금지(크래시루프 감지 무력화 위험)이므로 log+EVT 로 가시화한다."""
    p = breaker_file(socket)
    now = _now()
    attempts = []
    if os.path.exists(p):
        try:
            attempts = json.load(open(p)).get("attempts", [])
        except Exception:
            isolated = _isolate_corrupt(p)  # 손상 격리(.corrupt-<ts>·최근3 prune)
            log("★P2-6: breaker.json 손상 — 격리(%s) 후 빈 카운트 재시작(침묵 리셋 아님·경고). "
                "크래시루프 감지가 이번 창에서 리셋될 수 있음." % isolated)
            _emit_evt("agent.error", agent="phoenix",
                      summary="breaker.json 손상 격리+리셋 — 크래시루프 감지 일시 약화")
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
        # ★C4/P1-5: raw subprocess 가드 — TimeoutExpired→정직 강등(rc 124 상당), 인터프리터/도구 부재→명시 실패.
        #   무가드 시 스냅샷 도구가 15초 초과하면 traceback→이유 없는 exit 1(P1-5). 롤백 '제안'은 부가정보이므로
        #   실패해도 restore 판정을 죽이지 않고 note 로 정직히 남긴다.
        try:
            r = subprocess.run([sys.executable, snap, "list"], capture_output=True, text=True, timeout=15)
            prop["generations_raw"] = (r.stdout or r.stderr or "").strip()[:600]
            gens = re.findall(r"(\d{8}T\d{6}Z)", r.stdout or "")
            prop["generations"] = gens
            if gens:
                prop["suggested_rollback_to"] = gens[-1]  # 목록상 직전 세대(도구 정렬 규약 따름)
        except subprocess.TimeoutExpired:
            prop["error"] = "스냅샷 도구 list 15s TIMEOUT(rc 124 상당) — 롤백 제안 생략(정직 강등)"
        except (FileNotFoundError, OSError) as e:
            prop["error"] = "스냅샷 도구 실행 불가(%s: %s) — 롤백 제안 생략" % (type(e).__name__, e)
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
    # stub 명령 주입: (watch가 이길 시간을 주려 지연) → ready 마커 + 세션 표식 → 생존.
    # ※ printf %s / Python %s 충돌 회피 위해 문자열 결합으로 값 삽입(포맷 지정자 미사용).
    # ★Windows(S6): surface 셸이 PowerShell 이라 bash 문법(exec) 불가 → PowerShell 형(Start-Sleep·Write-Output)으로 분기.
    #   ready 마커 substring·SESSION= 추출은 양 셸 렌더가 동일하게 만족(마지막 Start-Sleep/exec 로 surface 생존 유지).
    if IS_WINDOWS:
        cmdline = ("Start-Sleep 1; Write-Output 'PHOENIX_STUB_READY role=" + role +
                   " SESSION=" + observed_sid + " SPAWNMODE=" + mode + " ENDMARK'; Start-Sleep 3600")
    else:
        cmdline = ("sleep 1.2; echo PHOENIX_STUB_READY role=" + role +
                   " SESSION=" + observed_sid + " SPAWNMODE=" + mode + " ENDMARK; exec sleep 3600")
    cys("send", "--surface", ref, cmdline, socket=socket, timeout=10)
    cys("send-key", "--surface", ref, "Return", socket=socket, timeout=10)
    return ref, "surrogate stub on %s (SESSION=%s mode=%s)" % (ref, observed_sid, mode)


# ------------------------------------------------------------------ 단계 실행기

def _surface_agent_alive(socket, surface):
    """★C5/P1-10(W3): 특정 surface 의 agent_alive(구조화 readiness ack) — status --json 에서 surface_ref 대조.
    반환 True/False/None(미도달·미관측). 배너 리터럴 대신 구조화 신호로 readiness 를 판정하는 근거."""
    st = _status_json(socket)
    if st is None:
        return None
    for s in st.get("surfaces", []):
        if s.get("surface_ref") == surface:
            return s.get("agent_alive")
    return None


def stage_ready(socket, role, surface, stub):
    """기동 완료(ready) 판정 — 실 응답 신호(ready_marker) 확인. ★Phase10: 대량 부활에서 스폰이 스태거되면
    watch(신규 출력)가 이미 emit된 marker를 놓쳐 ready 타임아웃 → 부분부활. 먼저 현재 화면(read-screen)에
    marker 존재를 확인해 '지금 응답 가능한가'를 판정하고, 없을 때만 watch(신규 출력)로 대기한다.
    ★C5/P1-10(W3): prod 는 배너 리터럴 이전에 **구조화 ack**(status --json agent_alive)를 우선 확인한다 —
    배너 문자열 파싱의 취약성(포맷 변경 시 오판)을 줄이는 근원 신호. 구조화 미확정 시 배너 폴백(가용성)."""
    if not stub:
        alive = _surface_agent_alive(socket, surface)
        if alive is True:
            return True, "structured ack: agent_alive=true (status --json)"
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


def _surface_agent_present(socket, surface):
    """★WP-11 각성핑 agent-gate: 대상 surface에 배정된 에이전트가 있는지 조회.
    True=agent 배정됨(재주입 대상)·False=agent=None 빈 셸(각성 핑 skip)·None=조회불가(보수적=진행).
    빈 zsh 셸(role=master·agent=None)에 reinject --check 핑을 쏘면 셸이 glob/pipe로 오해석해
    zsh 에러(감사 에러①)가 난다. 실 크래시 master(agent 있던)는 계속 복구하므로 agent=None만 skip한다."""
    try:
        r = cys("status", "--json", socket=socket, timeout=8)
        if r.returncode != 0:
            return None
        surfaces = (json.loads(r.stdout or "{}") or {}).get("surfaces", [])
    except Exception:
        return None
    for s in surfaces:
        if s.get("surface_ref") == surface or ("surface:%s" % s.get("surface_id")) == surface:
            return s.get("agent") is not None
    return None


def stage_reinject(socket, role, surface, stub):
    """디렉티브 재주입 — reinject --check 재사용(각성 핑 후 필요 시 주입).
    ★WP-11 agent-gate: agent=None 빈 셸엔 각성 핑을 쏘지 않는다(zsh 오해석 에러 차단)."""
    if _surface_agent_present(socket, surface) is False:
        return True, "reinject skip: agent 없음(빈 셸) — 각성 핑 미발사(WP-11 agent-gate)"
    r = cys("reinject", "--check", "--role", role, "--surface", surface, "--timeout", "6",
            socket=socket, timeout=12)
    return r.returncode == 0, "reinject rc=%s %s" % (r.returncode, (r.stdout or r.stderr or "").strip()[:120])


def stage_g2_ack(socket, role, surface, stub):
    """G2 핸드셰이크 ack — 부활 노드가 원장 대조 핑에 응답하는지(M7). 응답 없으면
    타임아웃 → unverified 격하 모드로 전진(무한 보류 금지). stub은 응답자가 없으므로
    best-effort 로 시도만 하고 결과를 저널에 남긴다.
    ★WP-11 agent-gate: agent=None 빈 셸엔 각성 핑을 쏘지 않는다(빈 셸은 ack 주체 없음)."""
    if _surface_agent_present(socket, surface) is False:
        return False, "g2 skip: agent 없음(빈 셸) — 각성 핑 미발사(WP-11 agent-gate)"
    r = cys("reinject", "--check", "--role", role, "--surface", surface, "--timeout", "4",
            socket=socket, timeout=10)
    acked = (r.returncode == 0) and ("각성" in (r.stdout or "") or "awake" in (r.stdout or "").lower())
    return acked, "g2 ack=%s (%s)" % (acked, (r.stdout or r.stderr or "").strip()[:120])


# ------------------------------------------------------------------ restore 상태머신

def _acquire_restore_lease(socket):
    """★W2 restore lease: 단일 restore-in-progress 파일락 — 콜드부트 auto-restore와 deploy
    오케스트레이션이 동시에 restore를 돌려 같은 역할을 이중 스폰하는 TOCTOU를 차단한다.
    반환 (ok, handle): ok=False 는 '다른 restore가 진행 중 → 중복 skip'. ok=True 면 진행하되
    handle(열린 파일객체)를 함수 끝까지 살려 락을 유지해야 한다(fail-open 시 handle=None).
    ★D2(W5): unix flock·Windows msvcrt 통합(_try_lock_nb) — 과거 Windows 전면 fail-open(P1-8: auto+수동
    restore 이중 스폰)을 제거. 락 기구 미가용만 fail-open."""
    try:
        lease_path = os.path.join(phoenix_home(socket), "restore.lease")
        f = open(lease_path, "a+")  # 무truncate·생성·byte0 락 대상(Windows msvcrt 영역 일치)
    except Exception:
        return True, None  # 락 파일 생성 실패 = 게이트 없이 진행(가용성 우선 fail-open)
    r = _try_lock_nb(f)
    if r is False:
        f.close()
        return False, None  # 다른 restore 보유 중 — 중복 인지 skip
    return True, f          # True(획득) 또는 None(락 기구 미가용=fail-open) → 핸들 보유하고 진행


def _release_lease(handle):
    """restore lease 핸들 해제(atexit 등록 대상). flock 은 close 로 자동 해제된다. best-effort(이중 해제 무해)."""
    try:
        handle.close()
    except Exception:
        pass


def c6_detect_stale_surfaces(socket):
    """★C6 S0: exited=true surface 잔재를 열거한다. 라이브(exited=false)는 어떤 경우에도 비대상."""
    stale = []
    for role, surfs in live_role_surfaces(socket).items():
        for s in surfs:
            if s.get("exited"):
                stale.append({"surface": s.get("surface"), "role": role, "pid": s.get("pid")})
    return stale


# ------------- CU-6B: 죽은-에이전트 페인 **관찰자**(표식·보고 전용) + agent_seen 원장 -------------
#
# ADR-5(설계 정본 §3): **회수기는 순수 관찰자이고, 스폰 권위는 phoenix 복구 경로 단독이다.** 이 절의
# 코드는 어떤 surface 도 닫지 않고(close-surface 호출 0) 띄우지 않는다(node-recover·restore 호출 0).
# 외부로 나가는 호출은 `cys status --json` 읽기 하나뿐이다. 사다리(기획서 §5-5)는 1단=표식 ·
# 2단=GUI 접힘(ui/src) · 3단=오너 close 이며, **3단(자동 close)은 본 CU 비범위**(오너 결재 사항).
#
# 왜 원장이 필요한가: `agent_alive=false` 하나만으로는 "죽었다"를 말할 수 없다. 데몬은 launch-agent 로
# 등록된 노드의 presence 만 답하므로, 아직 한 번도 뜬 적 없는 부팅 중 pane 도 false 로 보인다. 그래서
# **살아 있는 것을 본 이력**(agent_seen)을 파일 원장에 남기고, 이력이 있는 pane 만 '죽었다'고 판정한다.
# (null=미상은 어떤 경우에도 비대상 — 수동 셸·구데몬·claim-role 만 한 pane 이 여기 걸리면 오살이다.)

DEADAGENT_GRACE_DEFAULT = 300      # 부팅 창 보호(초). env CYS_DEADAGENT_GRACE_SECS 로 상향·하향.
REAP_LEDGER_NAME = "reap_ledger.json"


def reap_ledger_path(socket):
    """agent_seen 원장 경로 — phoenix_home(socket) 하위(라이브 상태 파일 무접촉 계약과 동일 규약)."""
    return os.path.join(phoenix_home(socket), REAP_LEDGER_NAME)


def deadagent_grace_secs():
    """부팅 창 보호 초. env CYS_DEADAGENT_GRACE_SECS 우선(정수·음수 아님), 부재·부적격이면 기본 300s.
    부적격 값을 조용히 삼키지 않는다(오설정이 탐지 술어를 몰래 바꾸는 것을 막는다)."""
    raw = os.environ.get("CYS_DEADAGENT_GRACE_SECS")
    if raw is None or str(raw).strip() == "":
        return DEADAGENT_GRACE_DEFAULT
    try:
        v = int(str(raw).strip())
    except (TypeError, ValueError):
        log("★CU-6B: CYS_DEADAGENT_GRACE_SECS=%r 는 정수가 아니다 — 기본 %ds 사용(무시 아님·고지)."
            % (raw, DEADAGENT_GRACE_DEFAULT))
        return DEADAGENT_GRACE_DEFAULT
    if v < 0:
        log("★CU-6B: CYS_DEADAGENT_GRACE_SECS=%s 는 음수 — 기본 %ds 사용(보호창 무력화 금지)."
            % (v, DEADAGENT_GRACE_DEFAULT))
        return DEADAGENT_GRACE_DEFAULT
    return v


def load_reap_ledger(path):
    """원장 로드 → {surface_ref: {first_seen, last_seen}}. 부재=빈 dict(fresh 정상·경고 없음).
    손상=기존 `_isolate_corrupt` 로 격리 후 빈 dict — **침묵 리셋 금지**(log+EVT 로 이력 소실을 고지한다).
    이력 소실의 방향은 안전하다: 이력이 없으면 판정이 '비대상'으로 보수화된다(오살 아님·탐지 지연)."""
    if not os.path.exists(path):
        return {}
    try:
        with open(path, encoding="utf-8") as f:
            obj = json.load(f)
        if not isinstance(obj, dict):
            raise ValueError("최상위가 object 가 아님")
        seen = obj.get("surfaces", {})
        if seen is None:
            seen = {}
        if not isinstance(seen, dict):
            raise ValueError("surfaces 가 object 가 아님")
    except Exception as _e:
        isolated = _isolate_corrupt(path)
        log("★CU-6B 원장 손상(%s: %s) — 격리(%s) 후 빈 원장으로 재시작. 침묵 아님: agent_seen 이력이 "
            "소실돼 당분간 탐지가 보수화(비대상)된다." % (type(_e).__name__, _e, isolated))
        _emit_evt("agent.error", agent="phoenix",
                  summary="reap_ledger 손상 — 격리 후 빈 원장 재시작(agent_seen 이력 소실·탐지 보수화)")
        return {}
    out = {}
    for ref, rec in seen.items():
        if isinstance(ref, str) and ref and isinstance(rec, dict):
            out[ref] = {"first_seen": rec.get("first_seen"), "last_seen": rec.get("last_seen")}
    return out


def save_reap_ledger(path, ledger):
    """원장 영속(원자쓰기 관용구 재사용). 봉투(surfaces/updated_at)는 additive 확장을 위한 것."""
    _atomic_write_json(path, {"surfaces": ledger, "updated_at": _now()})


def ledger_observe_alive(ledger, entries, now=None):
    """**순수 함수**(I/O 0·입력 불변 — 테스트 주입 지점): `status --json`.surfaces 중 `agent_alive is True`
    인 항목을 원장에 upsert 한 **새 dict** 를 반환한다. first_seen 은 최초 관측 시각 고정, last_seen 은 매
    관측 갱신. 살아있지 않은(false/null) 항목은 원장을 건드리지 않는다 — 그래야 '죽은 것'이 자기 이력을
    스스로 만들어내는 자기오염이 원천적으로 불가능하다."""
    now = _now() if now is None else int(now)
    out = {k: dict(v) for k, v in (ledger or {}).items() if isinstance(v, dict)}
    for s in entries or []:
        if not isinstance(s, dict):
            continue
        ref = s.get("surface_ref")
        if not isinstance(ref, str) or not ref:
            continue
        if s.get("agent_alive") is not True:       # 엄격 — truthy 가 아니라 True 동일성
            continue
        rec = out.get(ref)
        if rec is None:
            out[ref] = {"first_seen": now, "last_seen": now}
        else:
            rec["last_seen"] = now
            if rec.get("first_seen") is None:
                rec["first_seen"] = now
    return out


def detect_dead_agent_entries(entries, ledger, now=None, grace=None):
    """**순수 판정 함수**(I/O 0): 술어 5종을 **전부** 충족한 surface 만 반환한다(하나라도 불충족=비대상).
      ① `agent_alive is False` — 엄격. None(미상)은 절대 비대상: 수동 셸·구데몬·claim-role 전용 pane.
      ② `exited is False` — 엄격. 종료 잔재는 C6 stale 회수기 소관이라 이중 취급하지 않는다.
      ③ agent 등록됨(비어있지 않은 문자열) — launch-agent 로 띄운 노드만. 빈 셸·온보딩 pane 비대상.
      ④ `created_at + grace < now` — 부팅 창 보호(기본 300s). created_at 부재·비수치면 비대상(모르면 안 센다).
      ⑤ 원장에 agent_seen 이력 존재 — 한 번도 살아있는 걸 본 적 없으면 '죽은' 게 아니라 '아직'이다.
    반환 항목은 표식·보고 전용 관측 레코드다(행동 지시 아님)."""
    now = _now() if now is None else int(now)
    grace = deadagent_grace_secs() if grace is None else int(grace)
    ledger = ledger or {}
    out = []
    for s in entries or []:
        if not isinstance(s, dict):
            continue
        ref = s.get("surface_ref")
        if not isinstance(ref, str) or not ref:
            continue
        if s.get("agent_alive") is not False:                       # ①
            continue
        if s.get("exited") is not False:                            # ②
            continue
        agent = s.get("agent")
        if not isinstance(agent, str) or not agent.strip():         # ③
            continue
        created = s.get("created_at")
        if not isinstance(created, (int, float)) or isinstance(created, bool):  # ④(모르면 비대상)
            continue
        age = now - float(created)
        if age <= grace:
            continue
        seen = ledger.get(ref)                                      # ⑤
        if not isinstance(seen, dict) or seen.get("first_seen") is None:
            continue
        out.append({
            "surface": ref,
            "role": s.get("role"),
            "agent": agent,
            "created_at": created,
            "age_secs": int(age),
            "agent_seen_first": seen.get("first_seen"),
            "agent_seen_last": seen.get("last_seen"),
        })
    return out


def c6_detect_dead_agent_surfaces(socket, now=None):
    """★CU-6B 관찰자 진입점 — `cys status --json` 1콜로 ⓐ살아있는 노드의 agent_seen 원장 upsert,
    ⓑ죽은-에이전트 페인 탐지를 함께 수행하고 **탐지 결과만** 반환한다. 어떤 행동도 하지 않는다
    (close·reap·스폰·복구 호출 0 — ADR-5). status 불신(rc≠0·파싱 실패)=빈 목록(fail-safe: 소스를
    못 믿을 때 '전원 사망 추정'을 하지 않는다는 C5 원칙과 동일 방향)."""
    st = _status_json(socket)
    if not isinstance(st, dict):
        return []
    entries = st.get("surfaces")
    if not isinstance(entries, list):
        return []
    now = _now() if now is None else int(now)
    path = reap_ledger_path(socket)
    ledger = load_reap_ledger(path)
    updated = ledger_observe_alive(ledger, entries, now)
    if updated != ledger:
        try:
            save_reap_ledger(path, updated)
        except Exception as _e:
            # 원장 쓰기 실패가 탐지를 죽이지 않는다(다음 틱 재시도) — 단 침묵하지 않는다.
            log("★CU-6B: reap_ledger 쓰기 실패(%s: %s) — 이번 틱 이력은 휘발, 탐지는 진행."
                % (type(_e).__name__, _e))
    return detect_dead_agent_entries(entries, updated, now, deadagent_grace_secs())


def c6_reap_stale_surfaces(socket):
    """★C6 S0 실 회수(W2 — P0-6 cause 활용): exited=true 잔재를 **Reap 사유로만** 회수한다
    (`cys close-surface <ref> --reap` → CloseCause::Reap → 묘비 미생성·부활 대상 유지). 고정 OwnerClose 경로
    (--reap 없는 close)는 절대 쓰지 않는다(P0-6 오묘비 함정). 라이브(exited=false)는 비대상. 반환:
    {"detected":[...], "reaped":[refs], "reap_failed":[...]}. 회수 후에도 묘비 0이어야 함(호출측 게이트 검증)."""
    detected = c6_detect_stale_surfaces(socket)
    reaped, failed = [], []
    for item in detected:
        ref = item.get("surface")
        if not ref:
            continue
        r = cys("close-surface", ref, "--reap", socket=socket, timeout=12)
        if getattr(r, "returncode", 1) == 0:
            reaped.append(ref)
        else:
            failed.append({"surface": ref, "err": (getattr(r, "stderr", "") or getattr(r, "stdout", ""))[:120]})
    return {"detected": detected, "reaped": reaped, "reap_failed": failed}


def run_restore(socket, ticket="default", stub=False, no_breaker=False, roles=None,
                include_master=False, stub_sids=None, print_result=True):
    """부활 저널 상태머신 본체(재사용 가능한 함수). cmd_restore(CLI)와 cmd_deploy(restore 단계)가 이 하나를
    공유한다 — P2 재사용 제1원칙(신규 부활 엔진을 만들지 않는다·코드 복제 금지). print_result=False 면 결과
    dict 만 반환하고 stdout 에 출력하지 않는다(deploy 가 단일 JSON 레코드로 감싸 출력할 때 사용)."""
    global _ACTIVE_EPOCH
    # ★W2 restore lease: 동시 restore(콜드부트 auto vs deploy) 이중 스폰 차단. 먼저 획득해
    # breaker·spawn 전체를 직렬화한다. 다른 restore 진행 중이면 즉시 중복 skip(무해).
    _lease_ok, _lease_handle = _acquire_restore_lease(socket)
    if not _lease_ok:
        out = {"phoenix_restore": "LEASE_HELD",
               "note": "다른 restore가 진행 중 — 이중 스폰 방지 위해 이번 호출은 skip(멱등)."}
        log("★restore lease 보유 중(다른 restore 진행) — 중복 skip.")
        if print_result:
            print(json.dumps(out, ensure_ascii=False, indent=2))
        return out
    # ★P2-7/W1: lease 핸들을 atexit 로 확실히 해제 등록(예외·sys.exit 경로에서도 flock 이 남지 않게).
    #   flock 은 fd close 시 자동 해제되지만, 프로세스가 살아있는 채 다음 restore 를 부르는 경로(deploy 중첩)
    #   에서 GC 타이밍에 의존하지 않도록 명시 해제한다. handle=None(fail-open)이면 등록 불요.
    if _lease_handle is not None:
        atexit.register(lambda h=_lease_handle: _release_lease(h))
    # ★Phase 6: 이 부팅 세대(재시작마다 변경)를 취득 — 저널 완료 마킹의 유효성 기준.
    _ACTIVE_EPOCH = get_boot_epoch(socket)

    # ★C2 손상 대응 — 2단계 계층화(W4 sentinel + W3 폴백 체인). missing(부재)=fresh install 정상 진행.
    #   corrupt(파싱 실패)=격리(.corrupt-<ts>·최근3 prune) 후 폴백 체인(.bak → 세대 스냅샷 → dept 재발견):
    #     · 폴백 복원 성공 = degraded(묘비 불확실 → 부활 보류+escalation, fail-safe · exit 3)
    #     · 전 폴백 실패 = unrecoverable(부활 중단+escalation · exit 6)
    #   빈 상태(fresh) 위장 통과는 어느 경우도 없다(silent-empty 차단).
    #     · 폴백 복원(신규 손상)=recovered_from provenance 를 파일에 영속 → degraded(부활 보류·전 auto-retry 지속).
    #   ★codex W3 BLOCKING: degraded 최종 판정은 observe **후** 파일의 recovered_from 으로 한다(휘발 방지 — 아래).
    for _p, _kind in ((desired_roster_path(socket), "desired_roster"),
                      (dept_roster_path(socket), "dept_roster")):
        _rec = _recover_retention_file(socket, _p, _kind)
        if _rec["status"] == "unrecoverable":
            log("★C2 손상 감지: %s 파싱 실패·전 폴백(.bak/스냅샷) 불가 — 부활 중단(exit 6). 격리=%s"
                % (_kind, _rec.get("isolated")))
            _emit_evt("agent.error", agent="phoenix",
                      summary="C2 손상 복원 불가(%s) — 부활 중단, 사람 개입 필요" % _kind)
            out = {"phoenix_restore": "CORRUPT", "corruption": True, "corrupt_file": _kind,
                   "corrupt_path": _p, "isolated": _rec.get("isolated"),
                   "note": ("손상된 %s — .bak·세대 스냅샷 전 폴백 불가. 빈 상태(fresh)로 진행하지 않는다"
                            "(missing≠corrupt). 사람 개입(phoenix roster --rebase 또는 복원) 필요." % _kind)}
            if print_result:
                print(json.dumps(out, ensure_ascii=False, indent=2))
            return out
        if _rec["status"] == "degraded" and not _rec.get("pending"):
            # 이번 실행에서 새로 손상→복구(fresh) — escalation EVT 1회. pending(전 auto-retry 이월)은 재방출 안 함.
            log("★C2 폴백 복원: %s 손상 → %s 로 복원(degraded·provenance 영속). 묘비 불확실 → 부활 보류(fail-safe)."
                % (_kind, _rec.get("source")))
            _emit_evt("agent.error", agent="phoenix",
                      summary="C2 손상→폴백 복원(%s src=%s) — degraded, 부활 보류(영속)"
                              % (_kind, _rec.get("source")))

    j = load_journal(socket, ticket)
    # ★C6 S0 실 회수(W2 — P0-6 cause): exited=true 잔재를 Reap 사유로만 회수(cys close-surface --reap →
    #   CloseCause::Reap → 묘비 미생성). 라이브(exited=false)는 비대상. OwnerClose 경로는 절대 미사용(오묘비 함정).
    c6 = c6_reap_stale_surfaces(socket)
    c6_stale = c6.get("detected", [])
    if c6_stale:
        jevent(j, "*", "s0_c6", "reaped",
               "exited 잔재 %d개 탐지·%d개 Reap 회수(묘비 미생성): %s"
               % (len(c6_stale), len(c6.get("reaped", [])), c6.get("reaped", [])))
        log("★C6 S0: 죽은 surface 잔재 %d개 탐지 → %d개 Reap 회수(묘비 0·라이브 무접촉)."
            % (len(c6_stale), len(c6.get("reaped", []))))
        save_journal(socket, ticket, j)
    # ★CU-6B 관찰자(표식 전용): agent 가 죽은 라이브 페인을 **탐지만** 한다 — 회수·close·스폰 0(ADR-5).
    #   관찰이 부활을 죽이면 안 되므로 어떤 예외도 여기서 흡수한다(탐지 실패=빈 목록·보고에 그대로 노출).
    try:
        c6_dead = c6_detect_dead_agent_surfaces(socket)
    except Exception as _e:
        log("★CU-6B: 죽은-에이전트 탐지 중 예외(%s: %s) — 관찰 skip(부활 경로 무영향)."
            % (type(_e).__name__, _e))
        c6_dead = []
    if c6_dead:
        log("★CU-6B: agent 사망 페인 %d개 탐지(표식만 — 자동 close·회수 없음): %s"
            % (len(c6_dead), [d.get("surface") for d in c6_dead]))
    # ★Phase 4: 대상 판정 근거 = actual-state(topology)가 아니라 desired 로스터.
    # 관측을 조기·단조 영속해 topology 침식(부분 부활 후 미부활 역할 삭제)에 면역시킨다(§12).
    entries, _tombstones = observe_and_persist_roster(socket)
    # ★codex W3 BLOCKING(1): degraded 최종 판정 = observe **후** 영속된 recovered_from(진실은 파일 상태).
    #   observe 의 검증된-건강 topology replace 가 provenance 를 떨궜으면 해제(부활 진행), 남아있으면 hold —
    #   따라서 복구본이 유효 JSON 이 된 뒤 도는 cysd auto-retry 2차 실행도 동일하게 보류가 유지된다(휘발 방지).
    _c2_degraded = []
    for _p, _kind in ((desired_roster_path(socket), "desired_roster"),
                      (dept_roster_path(socket), "dept_roster")):
        _prov = _recovered_provenance(_p)
        if _prov:
            _c2_degraded.append({"file": _kind, "source": _prov.get("source"), "ts": _prov.get("ts")})
    live, _live_known = _live_role_surfaces_checked(socket)

    # 대상 = desired 로스터에 있으나 살아있지 않은(또는 exited) 역할
    # ★SEAT(2026-07-17 실사고 수리): '살아있다'를 exited 만으로 판정하면, role=master 를 쥔 채
    #   agent 가 없는 **빈 셸**이 生으로 잡혀 master 가 부활 대상에서 통째로 빠졌다(09:35 dept-2 실측:
    #   대상역할에 master 부재 → 아래 completeness 가 master 를 검증조차 않고 COMPLETE 선언 = 침묵 성공).
    #   좌석이 비었으면(seat=='empty') 그 역할은 살아있는 게 아니라 **부활 대상**이다.
    #   seat 키가 없는 구 데몬에선 None → 종전 규칙으로 degrade(하위호환).
    def _alive(role):
        for s in live.get(role, []):
            if s["exited"]:
                continue
            if s.get("seat") == "empty":
                continue  # 좌석은 있으나 아무도 앉아 있지 않다 — 부활시킨다(in-seat 연결은 cys restore 담당)
            return True
        return False

    # ★SEAT: 빈 좌석으로 판정돼 부활 대상이 된 역할(정직 보고용) — completeness 의 manual_seats 근거.
    def _empty_seat_roles():
        out = []
        for role, ss in live.items():
            if role == "-":
                continue
            if any((not s["exited"]) and s.get("seat") == "empty" for s in ss):
                out.append(role)
        return sorted(out)

    # ★codex W2 BLOCKING: 묘비 제외는 target 산정 **시점**에 tombstones 대조로 수행(roster 엔트리는 보존됨).
    #   entries 에 묘비 역할이 남아 있어도 부활 대상에서만 빠진다 — untomb 시 엔트리·메타가 있어 즉시 부활 가능.
    # ★codex W2 재판정: 명시 --roles 경로도 tombstone 필터를 반드시 통과시킨다(과거 `roles or [...]` 는 명시
    #   경로가 필터를 우회 — 의도삭제된 역할을 강제 부활시키는 구멍). '의도삭제>강제부활' 1급 원칙상 우회 스위치
    #   (--ignore-tombstones 류)는 추가하지 않는다 — 정당한 재편입은 untomb RPC(cys tombstone --remove)가 정도(正道)다.
    _requested = roles if roles is not None else [r for r in entries if not _alive(r)]
    _tomb_skipped = [r for r in _requested if r in _tombstones]
    target_roles = [r for r in _requested if not _alive(r) and r not in _tombstones]
    if _tomb_skipped:
        log("★묘비 필터: 요청 역할 중 폐역(tombstone) %s 은 부활 대상에서 제외(의도삭제>강제부활). untomb 로만 재편입." % _tomb_skipped)
        for _r in _tomb_skipped:
            jevent(j, _r, "target", "skip_tombstoned",
                   "명시 요청됐으나 폐역(tombstone) — 부활 제외(의도삭제>강제부활). 재편입=untomb RPC.")
        save_journal(socket, ticket, j)
    # ★C5 unknown-liveness 부활 보류(codex W3 BLOCKING(3)·fail-safe): status --json 실패 + list 폴백 구조
    #   드리프트로 liveness 를 신뢰할 수 없으면(_live_known=False) 전 역할을 '죽음'으로 오판해 대량 스폰하지 않고
    #   보류+escalation 한다(P1-6 재도입 차단). 복귀 = liveness 소스 회복(status --json 정상 또는 list 형식 복구).
    if not _live_known:
        log("★C5 unknown-liveness: status --json 실패 + list 폴백 구조 무효 — 전원 사망 추정 금지, 부활 보류(fail-safe).")
        _emit_evt("agent.error", agent="phoenix",
                  summary="liveness 소스 불신(status --json 실패+list 드리프트) — 부활 보류, 대량 스폰 차단")
        jevent(j, "*", "c5_unknown_liveness", "hold", "liveness 불신 — 부활 보류(대량 오스폰 방지)")
        save_journal(socket, ticket, j)
        out = {"phoenix_restore": "DEGRADED", "degraded_reason": "unknown_liveness",
               "note": ("liveness 소스(status --json/list) 불신 — 전원 사망 추정 금지, 부활 보류(fail-safe). "
                        "복귀: liveness 소스 회복.")}
        if print_result:
            print(json.dumps(out, ensure_ascii=False, indent=2))
        return out
    # ★C2 degraded 부활 보류(fail-safe): retention 파일에 recovered_from provenance 가 영속돼 있으면(손상 폴백
    #   복원으로 묘비 불확실) 전 대상 부활을 보류하고 DEGRADED(exit 3)로 종료한다. ★codex W3 BLOCKING(1): 이
    #   판정은 observe 후 파일 상태(recovered_from)로 하므로, 복구본이 유효 JSON 이 된 뒤 도는 auto-retry 2차
    #   실행도 동일하게 보류가 유지된다. 복귀 = 검증된-건강 topology replace(provenance 자동 제거) 또는 `--rebase`.
    if _c2_degraded:
        for _d in _c2_degraded:
            jevent(j, "*", "c2_degraded", "hold",
                   "손상 폴백 복원(%s src=%s·provenance 영속) — 묘비 불확실, 부활 보류(fail-safe)" % (_d["file"], _d["source"]))
        jevent(j, "*", "c2_degraded", "held_roles", "부활 보류 역할: %s" % target_roles)
        save_journal(socket, ticket, j)
        out = {"phoenix_restore": "DEGRADED", "degraded_reason": "c2_corrupt_fallback",
               "degraded": _c2_degraded, "held_roles": target_roles,
               "note": ("retention 파일 손상→폴백 복원(recovered_from 영속)으로 묘비 불확실 — 부활 보류(fail-safe). "
                        "복귀: 검증된-건강 topology replace 또는 `phoenix roster --rebase`.")}
        if print_result:
            print(json.dumps(out, ensure_ascii=False, indent=2))
        return out
    # ★M5/P1-3 회로차단기: '실제 부활 시도(죽은 역할 존재)'에만 기록한다. NOOP(대상 0)은 스폰 시도가
    #   아니므로 카운트하지 않고 오히려 창을 리셋한다 — NOOP 반복이 차단기를 OPEN 시켜 진짜 부활을 막는
    #   오검출(P1-3)을 차단. target 산출 이후로 이동해 "spawn 시도가 있을 때만 기록"(설계 축C4)을 만족.
    if not target_roles:
        log("부활 대상 죽은 역할 0 — restore 무작업(멱등).")
        if not no_breaker:
            breaker_reset(socket)
    elif not no_breaker:
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
    # dedup(P4): 이 티켓 저널에서 이미 verify까지 done 인 역할은 skip
    pending = [r for r in target_roles if not stage_done(j, r, "verify")]
    log("티켓=%s · 대상역할=%s · 이번 진행=%s (완료 skip=%s)" % (
        ticket, target_roles, pending, [r for r in target_roles if r not in pending]))

    # ── spawn 단계(공유): production=cys restore 1회 / surrogate=역할별 stub ──
    role_surface = {}
    forced_sids = {}
    if stub_sids:
        try:
            forced_sids = json.loads(stub_sids)
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
            res = spawn_production(socket, need, include_master=include_master)
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
        # ★P2-3: verify done 은 outcome 이 verified/fresh(성공 부활)일 때만 True. unverified(transient/fork)는
        #   done=False 로 남겨 다음 restore 사이클이 재검증한다 — 과거처럼 unconditional done=True 로 마킹하면
        #   같은 부트 세대 내내 UNVERIFIED 가 고착(재검증 영구 skip)됐다. dedup(pending)이 이 done 을 본다.
        verify_done = outcome in ("verified", "fresh")
        mark_stage(j, role, "verify", verify_done,
                   "M9: expected=%r observed=%r → %s (%s)" % (exp, obs, outcome, reason))
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
    # ★SEAT 정직 보고(침묵 성공 금지): 부활을 마쳤는데도 **여전히 빈 좌석**인 역할을 명시한다.
    #   종전엔 '살아있는 것으로 보이는' 역할은 target 에서 빠져 검증조차 안 됐고(09:35 실측), 그 결과
    #   role=master 를 쥔 빈 셸이 있는데도 COMPLETE 가 선언됐다 — 시스템이 자기 실패를 몰랐다.
    #   이제 좌석 사실을 다시 읽어, 비어 있는 역할이 남았으면 그 사실을 verdict 에 싣는다.
    #   (구 데몬은 seat 키가 없어 항상 빈 목록 → 종전 동작과 동일하게 degrade.)
    manual_seats = sorted({
        role for role, ss in live_end.items()
        if role != "-" and any((not s["exited"]) and s.get("seat") == "empty" for s in ss)
    })
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
    if manual_seats:
        honesty += (" ★빈 좌석 잔존=%s: 이 역할들은 surface(좌석)는 있으나 **에이전트가 앉아 있지 않다**"
                    "(자손 프로세스 0 — 커널 사실). 부활이 아니라 '사람이 그 pane 에서 직접 agent 를 "
                    "실행'해야 채워지는 상태이거나, agent 미상(claim-role 등록 pane)이라 무엇을 띄울지 "
                    "결정론으로 알 수 없는 상태다. 좌석 앞 큐 메시지는 배달 보류(보존)된다 — 침묵 성공 "
                    "아님." % manual_seats)
    if fresh_fallback_roles:
        honesty += (" ★fresh 강등 역할=%s: 원 세션이 독약(resume 불가)이라 무 resume 로 새 세션을 기동하고 "
                    "디렉티브/원장을 재주입했다(세션 보존 실패를 정직히 밝힘 — roster 는 부활 완료). "
                    "독약 세션이 무한 재시도로 roster 를 막지 않게 유한 강등했다(§15·DRILL_LIVE_4)." % fresh_fallback_roles)

    result = {
        "phoenix_restore": final,
        "completeness": completeness,          # ★Phase10: readiness 기반 전원 부활 판정
        "incomplete_roles": incomplete_roles,  # ★Phase10: 미부활 역할 정직 명시(침묵 성공 금지)
        "manual_seats": manual_seats,          # ★SEAT: 좌석은 있으나 에이전트 부재(사람 개입 필요) — 정직 명시
        "fresh_fallback_roles": fresh_fallback_roles,  # ★Phase11: 독약 세션→fresh 강등 역할 정직 명시
        "ready_roles": ready_roles,
        "ticket": ticket,
        "boot_epoch": _ACTIVE_EPOCH,      # ★Phase6: 이 부활이 판정 기준으로 쓴 세대
        "epoch_gate": EPOCH_GATE,
        "backend": "surrogate(stub)" if stub else "production(cys restore)",
        "target_roles": target_roles,
        "per_role_outcome": outcomes,
        "c6_stale_surfaces": c6_stale,     # ★C6 S0: 탐지된 죽은 surface 잔재
        # ★CU-6B(additive): agent 가 죽었지만 셸은 살아있는 페인의 **탐지 결과만**. 이 키를 보고
        #   어떤 자동 행동도 하지 않는다(사다리 1단=표식 / 2단=GUI 접힘 / 3단=오너 close).
        "dead_agent_surfaces": c6_dead,
        "c6_reaped": c6.get("reaped", []), # ★C6 S0(W2): Reap 사유로 실제 회수한 잔재(묘비 미생성·라이브 무접촉)
        "c6_reap_failed": c6.get("reap_failed", []),
        "cys_identity": _CYS_IDENTITY,     # ★gate2: 해석된 cys 의 3중 identity 검증 상태(verified|degraded-unverified|None)
        "journal": journal_path(socket, ticket),
        "honesty_note": honesty,
    }
    # ★M9 계약: 미검증 결과에는 정상완료 주장 문자열을 절대 넣지 않는다(위 dict에도 없음)
    if print_result:
        print(json.dumps(result, ensure_ascii=False, indent=2))
    return result


def cmd_restore(args):
    """CLI 래퍼 — argparse 값을 run_restore 로 전달(본체는 run_restore·deploy 와 공유)."""
    # ★W2 --auto(콜드부트): master 포함 강제. master가 묘비면 observe_and_persist_roster의
    #   tombstone 병합이 roster에서 배제하므로 include_master여도 부활 대상이 되지 않는다
    #   (1급 원칙: 의도삭제>강제부활). raw `cys restore --include-master`도 묘비를 skip(심층방어).
    include_master = args.include_master or getattr(args, "auto", False)
    result = run_restore(
        args.socket, ticket=args.ticket or "default", stub=args.stub,
        no_breaker=args.no_breaker, roles=args.roles,
        include_master=include_master, stub_sids=args.stub_sids,
        print_result=True,
    )
    # ★W1/C1: CLI 진입점에서만 exit 를 방출한다(deploy 의 내부 run_restore 호출은 반환값을 쓰고
    #   자체 _finish 로 exit — 이 경로는 print_result=False 라 여기 오지 않는다). 판정→결정론 exit.
    sys.exit(restore_exit_code(result))


# ------------------------------------------------------------------ B1 조정 패스

def cmd_reconcile(args):
    """재기동 시 위임 대장(topology) vs 실측(surface·WORKER_TODO) 대조 → 불일치 보고.
    부활 직후 첫 행동은 '작업 계속'이 아니라 '원장 대조'(§10.4)."""
    socket = args.socket
    # ★Phase 4: 대장 = actual topology 대신 desired 로스터(침식 면역·§12). 관측을 조기 영속.
    roster, tombstones = observe_and_persist_roster(socket)
    live = live_role_surfaces(socket)
    todo = _read_worker_todo()

    # ★codex W2 BLOCKING: 묘비 역할은 roster 엔트리가 보존되므로 여기서 tombstones 대조로 제외 —
    #   의도적 폐역을 'MISSING(부활 필요)'로 오판하지 않는다(엔트리는 untomb 시 부활용으로 보존).
    expected_roles = sorted(r for r in roster.keys() if r not in tombstones)
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
    name = args.role
    if is_dept:
        # 부서(dept)는 phoenix 소유 dept_roster.json 이 진실(데몬 topology 무관) — 기존 파일 경로 유지.
        roster, tombstones = load_dept_roster(socket)
        path = dept_roster_path(socket)
        if args.remove:
            tombstones.discard(name); action = "폐역 해제(재편입 가능)"
        else:
            # ★codex W2 BLOCKING(dept): 엔트리 보존(pop 금지) — 배제는 소비 시점 tombstones 대조. untomb 즉시 부활.
            tombstones.add(name); action = "폐역(보호집합에서 제외 — 엔트리 보존·untomb 시 부활)"
        _atomic_write_json(path, {"roster": roster, "tombstones": sorted(tombstones), "updated_at": _now()})
        out = {"tombstone": name, "kind": "dept", "action": action, "via": "dept_roster",
               "tombstones": sorted(tombstones),
               "remaining": sorted(r for r in roster.keys() if r not in tombstones)}
        print(json.dumps(out, ensure_ascii=False, indent=2))
        return out
    # ★A-S3: role 폐역은 desired 직접 쓰기 대신 **데몬 RPC 경유**(옵션A 단일 작성자=데몬). RPC 성공=topology 묘비.
    #   실패(데몬 다운타임)=intent 저널 폴백 → observe 가 replace 이전 멱등 적용·데몬 복귀 후 재동기+영속 확인 후 절단.
    rpc_args = ["tombstone", name] + (["--remove"] if args.remove else [])
    r = cys(*rpc_args, socket=socket, timeout=12)
    if getattr(r, "returncode", 1) == 0:
        action = ("폐역 해제(RPC·데몬 topology)" if args.remove else "폐역(RPC·데몬 topology 묘비)")
        out = {"tombstone": name, "kind": "role", "action": action, "via": "daemon-rpc",
               "rpc_out": (getattr(r, "stdout", "") or "").strip()[:200]}
    else:
        _append_tombstone_intent(socket, name, args.remove)
        action = ("폐역 해제(intent 저널·데몬 down)" if args.remove else "폐역(intent 저널·데몬 down)")
        out = {"tombstone": name, "kind": "role", "action": action, "via": "intent-journal",
               "note": "데몬 미도달 — intent 저널 기록. observe 가 replace 이전 멱등 적용·데몬 복귀 후 재동기·영속 확인 후 절단."}
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


def cmd_roster(args):
    """desired 로스터(대장) 현황 — actual topology와 분리된 선언 상태를 노출(§12).
    ★C3: --rebase 면 설명-가능-축소 불변식을 1회 우회해 현재 관측을 강제 수용한다(운영자 명시 재기반)."""
    socket = args.socket
    _rebase = getattr(args, "rebase", False)
    roster, tombstones = observe_and_persist_roster(socket, rebase=_rebase)
    live = live_role_surfaces(socket)
    alive = {r for r, ss in live.items() if r != "-" and any(not s["exited"] for s in ss)}
    topo_roles = sorted(e.get("role") for e in read_topology(socket).get("entries", []) if e.get("role"))
    # ★C2(W3): --rebase 는 dept degraded provenance 도 해제(운영자 명시 재기반).
    dept_roster, dept_tomb = observe_and_persist_depts(socket, rebase=_rebase)  # ★Phase7: 부서도 보호집합에 노출
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
    """워커 todo 미완(- [ ]) 집계(실측 요약). ★복수 워커 정합(2026-07-16): cys todo-path 가
    worker-2 등 역할별 고유 파일(WORKER*_TODO.md)을 만들므로 WORKER_TODO.md 단일 하드코딩을
    글롭 집계로 교체 — 다중 워커 환경에서 잘못된/일부 todo 만 읽는 결함 봉합."""
    rdir = os.path.join(os.environ.get("CYS_PACK_DIR", os.path.join(HOME, ".cys", "pack")),
                        "round")
    files = sorted(glob.glob(os.path.join(rdir, "WORKER*_TODO.md")))
    if not files:
        return {"path": os.path.join(rdir, "WORKER*_TODO.md"), "exists": False}
    open_items = done_items = 0
    per_file, last_section = [], None
    for cand in files:
        try:
            txt = open(cand, errors="replace").read()
        except OSError:
            continue
        o, d = txt.count("- [ ]"), txt.count("- [x]")
        open_items += o
        done_items += d
        secs = re.findall(r"^#\s*(.+)$", txt, re.M)
        if secs:
            last_section = secs[-1][:80]
        per_file.append({"path": cand, "open_items": o, "done_items": d})
    return {"path": files[0] if len(files) == 1 else rdir, "exists": True,
            "open_items": open_items, "done_items": done_items,
            "last_section": last_section, "files": per_file}


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
    out = supervisor_status(args.label, running_pid=args.pid)
    out["honesty"] = ("managed=재부팅/로그온 자동기동 토대 intact · orphan=프로세스는 살아있으나 관리 이탈(재부팅되면 안 뜸) · "
                      "unmanaged=등록 부재. 읽기 전용(mac launchctl list/print · win schtasks /Query) — 데몬을 재시작·변경하지 않는다.")
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


def cmd_launchd_ensure(args):
    out = supervisor_ensure(args.label, args.plist, running_pid=args.pid)
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return out


# ------------------------------------------------------------------ supervisor 추상화(플랫폼 감독자)
# S1(Windows 패리티): 재부팅/로그온 자동기동 토대의 '관리 상태'와 '재시작'을 플랫폼 중립으로 얇게 감싼다.
#   · macOS: 기존 launchd_* 함수를 그대로 위임(무변경 동작 보장 — launchctl 경로·fake launchctl drill 불변).
#   · Windows: schtasks(작업 스케줄러) 기반. supervisor_status=schtasks /Query, supervisor_ensure=`cys daemon install`
#     (기존 Rust 로직 재사용 — schtasks 직접 조립 금지), 재시작=identify→taskkill /T /F→파이프 해제 폴링→재기동 유발.
# ★Windows 데몬은 사망 시 자동 respawn 이 없다(schtasks ONLOGON — KeepAlive 대응 없음). 재기동 유발은
#   managed 면 `schtasks /Run`, 비관리/실패면 `cys list`(CLI lazy-spawn 보완). 진짜 KeepAlive 패리티는 후속 사이클.

def _schtasks(*args, timeout=10):
    try:
        return subprocess.run(["schtasks"] + [str(a) for a in args],
                              capture_output=True, text=True, timeout=timeout)
    except Exception as e:
        class _R:
            returncode = 127
            stdout = ""
            stderr = str(e)
        return _R()


def _schtasks_has_restart_on_failure(task):
    """schtasks /Query /XML 에 RestartOnFailure 존재 여부(=진짜 KeepAlive 켜짐 · Rust cys daemon install 이 심는 XML).
    ★null 바이트 제거로 UTF-16/UTF-8 출력 모두에서 ASCII 태그를 안정 검출(UTF-16LE 는 ASCII 사이에 0x00 이 낀다)."""
    try:
        r = subprocess.run(["schtasks", "/Query", "/TN", task, "/XML"], capture_output=True, timeout=8)
        raw = (r.stdout or b"").replace(b"\x00", b"")
        return b"RestartOnFailure" in raw
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        # ★C4: 타임아웃/도구부재는 '검출 실패' = 보수적으로 False(KeepAlive 미확인 → 경고 경로). 정직 강등.
        return False
    except Exception:
        return False


def _schtasks_status(task, running_pid=None):
    """schtasks 등록 상태 분류(읽기 전용) — launchd_status 의 Windows 대응.
      managed   : `schtasks /Query /TN <task>` 존재(등록됨) — 로그온 자동기동 토대 intact
      orphan    : 미등록 + 프로세스는 살아있음(running_pid) — 로그온 시 자동기동 안 됨(관리 이탈)
      unmanaged : 미등록 + 프로세스도 없음(등록 자체 부재).
    ★keepalive: 등록됐고 XML 에 RestartOnFailure(진짜 KeepAlive·사망 시 자동 재기동)가 있으면 True, 없으면 False
      (구버전 install=ONLOGON 만·재기동 없음). launchd_status(keepalive) 계약과 대칭."""
    r = _schtasks("/Query", "/TN", task, timeout=8)
    registered = (getattr(r, "returncode", 1) == 0)
    if registered:
        state = "managed"
    elif running_pid:
        state = "orphan"
    else:
        state = "unmanaged"
    keepalive = _schtasks_has_restart_on_failure(task) if registered else False
    return {"label": task, "loaded": registered, "keepalive": keepalive, "runatload": registered,
            "state": state, "supervisor": "schtasks",
            "evidence": "schtasks /Query /TN %s rc=%s restart_on_failure=%s" % (task, getattr(r, "returncode", None), keepalive)}


def _schtasks_ensure(task, running_pid=None):
    """미관리/고아면 `cys daemon install`(기존 Rust schtasks 등록 로직 재사용)로 managed 복원. 멱등(이미 managed=noop).
    ★schtasks XML 직접 조립 금지 — Rust 단일 소스(cys daemon install)만 사용."""
    before = _schtasks_status(task, running_pid=running_pid)
    if before["state"] == "managed":
        return {"ensured": True, "action": "noop(이미 등록)", "before": before, "after": before}
    r = cys("daemon", "install", socket=None, timeout=30)
    after = _schtasks_status(task, running_pid=running_pid)
    return {"ensured": after["state"] == "managed",
            "action": "cys daemon install rc=%s" % getattr(r, "returncode", None),
            "out": (getattr(r, "stdout", "") or getattr(r, "stderr", "") or "").strip()[:300],
            "before": before, "after": after}


def _win_identify_daemon_pid(socket):
    """cys identify 결과 최상위 daemon_pid(handlers.rs system.identify) 획득 — taskkill 대상 pid.
    ★데몬 생존 중에만 호출한다(kill 전). 획득 실패 시 None(호출측이 kill 생략하고 재기동만 시도)."""
    r = cys("identify", socket=socket, timeout=8)
    txt = getattr(r, "stdout", "") or ""
    i = txt.find("{")
    if i < 0:
        return None
    try:
        d = json.loads(txt[i:])
    except Exception:
        return None
    pid = d.get("daemon_pid")
    if isinstance(pid, int):
        return pid
    for k in ("result", "caller"):  # 버전차 방어(감쌈 가능성)
        v = d.get(k)
        if isinstance(v, dict) and isinstance(v.get("daemon_pid"), int):
            return v["daemon_pid"]
    return None


def _win_restart_daemon(socket, timeout):
    """Windows 재시작 프리미티브(launchd kill 대역): identify→taskkill /T /F→★파이프 해제 폴링→재기동 유발.
    공통 pong+boot-epoch delta 확증은 호출자(_deploy_restart)가 담당(플랫폼 무관). 반환 dict 는 res 에 병합된다.
    ★파이프 해제 폴링(socket_death): named pipe first_pipe_instance(true) 단일인스턴스 경합 회피 —
      새 cysd 가 bind 하기 전에 기존 데몬이 파이프를 놓을 때(ping 무응답)까지 기다린 뒤 재기동을 유발한다
      (즉시 respawn vs 파이프 해제 race). taskkill /F 는 프로세스를 강제 종료하고 OS 가 종료 시 파이프 인스턴스를 파괴한다."""
    sdst = _schtasks_status(SUPERVISOR_LABEL)
    res = {"label": SUPERVISOR_LABEL, "supervisor_before": sdst}
    pid = _win_identify_daemon_pid(socket)
    res["daemon_pid"] = pid
    if pid:
        try:
            kr = subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"],
                                capture_output=True, text=True, timeout=15)
            res["taskkill_rc"] = kr.returncode
            res["taskkill_out"] = ((kr.stdout or "") + (kr.stderr or "")).strip()[:200]
        except Exception as e:
            res["taskkill_rc"] = 127
            res["taskkill_out"] = str(e)[:200]
    else:
        res["taskkill_rc"] = None
        res["taskkill_out"] = "cys identify 로 daemon_pid 미획득 — kill 생략(재기동만 시도)"
    # ★파이프 해제(소켓 소멸) 폴링 — 재기동 유발 전 필수(단일인스턴스 경합 회피). ping 은 autostart 안 함(안전 프로브).
    death = False
    t0 = time.time()
    while time.time() - t0 < min(8, timeout):
        pr = cys("ping", socket=socket, timeout=4)
        if not (getattr(pr, "returncode", 1) == 0 and "pong" in (getattr(pr, "stdout", "") or "")):
            death = True
            break
        time.sleep(0.3)
    res["socket_death_observed"] = death
    # ★재기동 유발: managed=schtasks /Run(등록 태스크 기동) · 비관리/실패=cys list(형제 cysd lazy-spawn 보완).
    if sdst.get("state") == "managed":
        rr = _schtasks("/Run", "/TN", SUPERVISOR_LABEL, timeout=10)
        res["retrigger"] = "schtasks /Run /TN %s rc=%s" % (SUPERVISOR_LABEL, getattr(rr, "returncode", None))
        if getattr(rr, "returncode", 1) != 0:  # /Run 실패 시 CLI lazy-spawn 폴백
            lr = cys("list", socket=socket, timeout=15)
            res["retrigger"] += " → 폴백 cys list rc=%s" % getattr(lr, "returncode", None)
    else:
        lr = cys("list", socket=socket, timeout=15)   # list=lazy-spawn(형제 cysd 자동기동)
        res["retrigger"] = "cys list(lazy-spawn) rc=%s" % getattr(lr, "returncode", None)
    return res


def supervisor_status(label, running_pid=None):
    """플랫폼 감독자 관리 상태(재부팅/로그온 자동기동 토대). mac=launchd_status · win=_schtasks_status.
    반환 dict 의 state(managed/orphan/unmanaged) 계약은 양 플랫폼 동일(deploy 게이트가 이 필드만 본다)."""
    if IS_WINDOWS:
        return _schtasks_status(label, running_pid=running_pid)
    return launchd_status(label, running_pid=running_pid)


def supervisor_ensure(label, plist=None, running_pid=None):
    """미관리/고아면 managed 복원(멱등). mac=launchd_ensure(bootstrap) · win=_schtasks_ensure(cys daemon install)."""
    if IS_WINDOWS:
        return _schtasks_ensure(label, running_pid=running_pid)
    return launchd_ensure(label, plist, running_pid=running_pid)


# ------------------------------------------------------------------ deploy (Phase 3)
# 설계 §9.4-3 · §10.1 전환 절차 · §11.3 M1(단일 게이트·quiescent→스냅샷→적용→재시작→부활→drill 기록 일체형·
#   생략 불가) · §11.1 하한2①(독립 수동 경로 동봉) · §14~16(DRILL_LIVE_3~5 교훈).
# deploy 자체가 데몬 재시작 = 전멸·부활 이벤트다(§10.1). 그 재시작을 첫 실전 drill 로 기록하고, 자동 부활
#   실패 대비 독립 수동 runbook 을 세대 스냅샷에 동봉한다(집행 계층 비의존). 단계는 P4 저널 상태머신으로
#   기록·재개하며(restore 와 동일 프리미티브 재사용·코드 복제 금지), 판정은 M9 정직 enum + 결정론 exit code.

# launchd 계열(라이브 재시작 경로) — src/launchd.rs 단일 소스(LAUNCHD_LABEL·plist 경로)와 정합.
LAUNCHD_LABEL = "com.cysjavis.cysd"
# supervisor 레이블 — mac=launchd label · win=schtasks 태스크명 'cysd'(Rust cys.rs DaemonAction TASK 와 정합).
#   PHOENIX_SUPERVISOR_LABEL 오버라이드: 격리 스모크가 글로벌 'cysd' 태스크와 분리(예: 존재하지 않는 테스트 태스크명
#   → schtasks 미등록=unmanaged → 재기동 유발이 커스텀 파이프의 cys list lazy-spawn 경로를 타게 함·교차오염 0).
SUPERVISOR_LABEL = os.environ.get("PHOENIX_SUPERVISOR_LABEL") or ("cysd" if IS_WINDOWS else LAUNCHD_LABEL)


def _launchd_plist_path():
    """~/Library/LaunchAgents/com.cysjavis.cysd.plist — src/launchd.rs plist_path() 와 동일 규약.
    launchd 재등록(launchd_ensure)의 소스. 부재하면 재등록 불가(정직 실패)."""
    return os.path.join(HOME, "Library", "LaunchAgents", LAUNCHD_LABEL + ".plist")

# deploy 단계(P4 저널) — 순서 고정. quiescent→스냅샷→적용→재시작→부활→판정(M1 일체형·생략 불가).
DEPLOY_STAGES = ["preflight", "quiescent", "snapshot", "apply", "restart", "restore", "verdict"]


def deploy_journal_path(socket, ticket):
    return os.path.join(phoenix_home(socket), "deploy-journal-%s.json" % _slug(ticket))


def load_deploy_journal(socket, ticket):
    p = deploy_journal_path(socket, ticket)
    if os.path.exists(p):
        try:
            return json.load(open(p))
        except Exception:
            # ★C2 2단계 보조상태(W3): deploy 저널도 재개 캐시(비 retention) — 손상 시 격리+경고 후 fresh.
            isolated = _isolate_corrupt(p)
            log("★C2 보조상태: deploy journal(%s) 손상 — 격리(%s) 후 fresh 시작." % (ticket, isolated))
            _emit_evt("agent.error", agent="phoenix",
                      summary="phoenix deploy journal(%s) 손상 — fresh 재시작" % ticket)
    return {"ticket": ticket, "stages": {}, "events": [], "created": _now()}


def save_deploy_journal(socket, ticket, j):
    _atomic_write_json(deploy_journal_path(socket, ticket), j)


def _dmark(j, stage, status, evidence="", **extra):
    """deploy 저널 단계 마킹(P4). epoch(재개 세대 기준)·runbook 등 재개에 필요한 최소 필드를 extra 로 첨부."""
    ent = {"status": status, "ts": _now(), "evidence": str(evidence)[:800]}
    ent.update(extra)
    j["stages"][stage] = ent
    j["events"].append({"ts": _now(), "stage": stage, "status": status, "msg": str(evidence)[:300]})


def _render_deploy_runbook(roster):
    """★하한2① 독립 수동 복원 runbook(MANUAL_RESTORE.sh) — 집행 계층(phoenix/데몬/hook) 비의존 자기완결 평문.
    cys 바이너리 + launchctl 만으로 사람이 그대로 복사-실행. 현재 로스터를 반영해 launchd 재기동 토대 복원 +
    역할별 순차 launch-agent + reinject 를 나열(§11.1 하한2 독립성 · §10.4 첫 행동=원장 대조)."""
    lines = [
        "#!/bin/bash",
        "# MANUAL_RESTORE.sh — 불사조 deploy '독립 수동 복원' 경로 (M1 출하 조건 · §11.1 하한2 독립성)",
        "# 자동 부활(cys phoenix)이 불능일 때, 사람이 이 스냅샷 세대 안에서 직접 조직을 재건한다.",
        "# 의존: cys 바이너리 + launchctl 만 — phoenix/데몬/hook 등 집행 계층 로직에 의존하지 않는다(자기완결 평문).",
        "# ★참석(attended) 경로 — 사람이 한 줄씩 확인하며 실행(§11.1 하한1: 유인 복구는 어떤 상태에서도 잠기지 않는다).",
        "set -u",
        'HERE="$(cd "$(dirname "$0")" && pwd)"',
        'U="$(id -u)"',
        'LABEL="%s"' % LAUNCHD_LABEL,
        'PLIST="$HOME/Library/LaunchAgents/%s.plist"' % LAUNCHD_LABEL,
        'echo "== 불사조 수동 복원 (deploy runbook · 세대 $HERE) =="',
        "",
        "# 1) 데몬 launchd 관리·재부팅 자동기동 토대 복원(KeepAlive/RunAtLoad — §15 발견3 관리 무결):",
        'if [ -f "$PLIST" ]; then',
        '  launchctl bootstrap "gui/$U" "$PLIST" 2>/dev/null || echo "  (이미 로드됐거나 bootstrap 실패 — launchctl print gui/$U/$LABEL 로 확인)"',
        '  launchctl kickstart -k "gui/$U/$LABEL" 2>/dev/null || true',
        "else",
        '  echo "  !! plist 부재($PLIST) — cys daemon install 로 재생성 후 재시도"',
        "fi",
        'echo "데몬 소켓 응답 대기..."; for i in $(seq 1 40); do cys ping 2>/dev/null | grep -q pong && { echo "  pong OK"; break; }; sleep 0.5; done',
        "",
        "# 2) 역할별 노드 순차 재기동(동시 resume 폭주 방지 §10.4 — 한 줄씩 확인 후 실행):",
    ]
    for role in sorted(roster.keys()):
        agent = (roster.get(role) or {}).get("agent") or "claude"
        lines.append('cys launch-agent --role %s --agent %s   # 각성 확인 후: cys reinject --role %s'
                     % (role, agent, role))
    lines += [
        "",
        'echo "★기동 후 첫 행동 = 원장 대조(G2), 작업 재개 아님(§10.4). 각 노드가 SESSION_STATE/자기 TODO 정합 후 대기."',
        'echo "※ 이 폴더의 topology.json(세대 스냅샷 사본)에 역할·세션 상세가 있다 — 참조용."',
        "",
    ]
    return "\n".join(lines) + "\n"


def _render_deploy_runbook_ps1(roster):
    """★Windows 독립 수동 복원 runbook(MANUAL_RESTORE.ps1) — 집행 계층(phoenix/데몬/hook) 비의존 자기완결 평문 PowerShell.
    cys.exe + schtasks(작업 스케줄러)만으로 사람이 그대로 실행: 로그온 자동기동 토대 재등록 → cys list 로 데몬 기동
    → cys ping 대기 → 역할별 순차 launch-agent + reinject 나열(§11.1 하한2 독립성 · §10.4 첫 행동=원장 대조).
    ★Windows 데몬은 사망 시 자동 respawn 이 없다 — schtasks 등록은 '로그온 자동기동' 토대이고, CLI lazy-spawn(cys list)이 보완."""
    lines = [
        "# MANUAL_RESTORE.ps1 — 불사조 deploy '독립 수동 복원' 경로 (Windows · M1 출하 조건 · §11.1 하한2 독립성)",
        "# 자동 부활(cys phoenix)이 불능일 때, 사람이 이 스냅샷 세대 안에서 직접 조직을 재건한다.",
        "# 의존: cys.exe + schtasks 만 — phoenix/데몬/hook 등 집행 계층 로직에 의존하지 않는다(자기완결 평문).",
        "# ★참석(attended) 경로 — 사람이 한 줄씩 확인하며 실행(§11.1 하한1: 유인 복구는 어떤 상태에서도 잠기지 않는다).",
        "$ErrorActionPreference = 'Continue'",
        "$Here = Split-Path -Parent $MyInvocation.MyCommand.Path",
        "$Task = '%s'" % SUPERVISOR_LABEL,
        'Write-Host "== 불사조 수동 복원 (deploy runbook · Windows · 세대 $Here) =="',
        "",
        "# 1) 데몬 로그온 자동기동 토대(작업 스케줄러) 재등록(§15 발견3). 사망 시 자동 respawn 은 미지원 — CLI 자동기동이 보완:",
        "schtasks /Query /TN $Task 2>$null | Out-Null",
        "if ($LASTEXITCODE -ne 0) {",
        '  Write-Host "  작업 스케줄러 미등록 → cys daemon install"; cys daemon install',
        "} else {",
        '  Write-Host "  (이미 등록됨) schtasks /Run 으로 기동 시도"; schtasks /Run /TN $Task 2>$null | Out-Null',
        "}",
        "",
        "# 2) 데몬 기동 유발(cys list = 형제 cysd lazy-spawn) + named pipe 응답 대기:",
        "cys list 2>$null | Out-Null",
        'Write-Host "데몬 named pipe 응답 대기..."',
        "for ($i = 0; $i -lt 40; $i++) { if ((cys ping 2>$null) -match 'pong') { Write-Host '  pong OK'; break }; Start-Sleep -Milliseconds 500 }",
        "",
        "# 3) 역할별 노드 순차 재기동(동시 resume 폭주 방지 §10.4 — 한 줄씩 확인 후 실행):",
    ]
    for role in sorted(roster.keys()):
        agent = (roster.get(role) or {}).get("agent") or "claude"
        lines.append("cys launch-agent --role %s --agent %s   # 각성 확인 후: cys reinject --role %s"
                     % (role, agent, role))
    lines += [
        "",
        "Write-Host '★기동 후 첫 행동 = 원장 대조(G2), 작업 재개 아님(§10.4). 각 노드가 SESSION_STATE/자기 TODO 정합 후 대기.'",
        "Write-Host '※ 이 폴더의 topology.json(세대 스냅샷 사본)에 역할·세션 상세가 있다 — 참조용.'",
        "",
    ]
    return "\n".join(lines) + "\n"


def _deploy_snapshot(socket, roster):
    """세대 스냅샷 + ★하한2① 독립 수동 runbook 동봉. 라이브=~/.cys/state-generations + default_sources(전 L1 선언상태),
    격리(--socket 하네스)=<phoenix_home>/state-generations + 소켓 상태 디렉터리의 L1 파일(자동 격리·hermetic)."""
    import io
    import contextlib
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    try:
        import javis_state_snapshot as _snap
    except Exception as e:
        return {"ok": False, "error": "javis_state_snapshot import 실패: %s" % e}
    sd = state_dir_for(socket)
    if sd == LIVE_STATE:
        gen_root = os.path.join(HOME, ".cys", "state-generations")
        sources = None  # default_sources() — 전 L1 선언상태(topology/schedule/autopilot/부서/원장/기억)
    else:
        gen_root = os.path.join(phoenix_home(socket), "state-generations")
        cand = [os.path.join(sd, b) for b in ("topology.json", "schedule_state.json", "autopilot.json", "event.seq")]
        sources = [s for s in cand if os.path.isfile(s)]
    buf = io.StringIO()
    gen_name = None
    try:
        with contextlib.redirect_stdout(buf):
            gen_name = _snap.do_snapshot(sources=sources, gen_root=gen_root)
    except Exception as e:
        return {"ok": False, "error": "do_snapshot 실패: %s" % e, "gen_root": gen_root}
    # 세대 디렉터리 안에 독립 runbook 동봉(자동 부활 불능 시 사람이 이 폴더에서 재건 — 하한2 독립성)
    gen_dir = os.path.join(gen_root, gen_name) if gen_name else os.path.join(phoenix_home(socket), "deploy-manual-fallback")
    os.makedirs(gen_dir, exist_ok=True)
    if IS_WINDOWS:
        # Windows 독립 수동 복원 = PowerShell(.ps1). utf-8-sig(BOM)로 써서 Windows PowerShell 5.1도 한글을 utf-8로 읽게 한다.
        runbook = os.path.join(gen_dir, "MANUAL_RESTORE.ps1")
        with open(runbook, "w", encoding="utf-8-sig") as f:
            f.write(_render_deploy_runbook_ps1(roster))
        # ★.ps1 은 PowerShell 로 실행 — 실행권한 비트(chmod) 개념 없음(생략).
    else:
        runbook = os.path.join(gen_dir, "MANUAL_RESTORE.sh")
        with open(runbook, "w") as f:
            f.write(_render_deploy_runbook(roster))
        os.chmod(runbook, 0o755)
    return {"ok": bool(gen_name), "gen_root": gen_root, "gen": gen_name, "gen_dir": gen_dir,
            "runbook": runbook, "snapshot_log": buf.getvalue().strip()[:400]}


def _deploy_restart(socket, restart_hook, timeout):
    """재시작 단계 — launchd(라이브) 또는 hook(격리·injected). boot-epoch delta 로 '실제 재시작'을 확증한다
    (타이밍 무관·§15 발견2 realpath/launchctl 기반 kill). hard fail = timeout 내 pong 미복귀(부활 실패)."""
    epoch_before = get_boot_epoch(socket)
    res = {"epoch_before": epoch_before, "timeout": timeout}
    if restart_hook:
        res["path"] = "hook(격리·injected)"
        res["hook"] = restart_hook
        try:
            r = subprocess.run(restart_hook, shell=True, capture_output=True, text=True, timeout=max(timeout, 30))
            res["hook_rc"] = r.returncode
            res["hook_out"] = (r.stdout or r.stderr or "").strip()[-500:]
        except subprocess.TimeoutExpired:
            res["hook_rc"] = 124
            res["hook_out"] = "TIMEOUT"
        except (FileNotFoundError, OSError) as e:
            res["hook_rc"] = 127
            res["hook_out"] = "hook 실행 불가(%s: %s)" % (type(e).__name__, e)
    elif IS_WINDOWS:
        # Windows 라이브 재시작(schtasks/taskkill) — launchd kill 대역. 공통 pong+epoch delta 확증은 아래 그대로.
        res["path"] = "schtasks/taskkill(라이브)"
        res.update(_win_restart_daemon(socket, timeout))
    else:
        res["path"] = "launchd(라이브)"
        res["label"] = LAUNCHD_LABEL
        res["launchd_before"] = launchd_status(LAUNCHD_LABEL)
        # SIGKILL 은 launchctl service-target 기반(pkill 심링크 빗나감 회피 §15 발견2). KeepAlive 가 자동 respawn.
        kr = _launchctl("kill", "SIGKILL", "gui/%d/%s" % (os.getuid(), LAUNCHD_LABEL), timeout=10)
        res["launchctl_kill_rc"] = getattr(kr, "returncode", None)
        res["launchctl_kill_err"] = (getattr(kr, "stderr", "") or "").strip()[:200]
        # 소켓 소멸 관측(best-effort — KeepAlive 가 빠르게 respawn 하면 창을 못 볼 수 있다·hard 판정은 epoch/pong)
        death = False
        t0 = time.time()
        while time.time() - t0 < min(8, timeout):
            pr = cys("ping", socket=socket, timeout=4)
            if not (pr.returncode == 0 and "pong" in (pr.stdout or "")):
                death = True
                break
            time.sleep(0.3)
        res["socket_death_observed"] = death
    # 부활 폴링(pong) — timeout 내 미복귀면 hard fail
    revived = False
    t0 = time.time()
    while time.time() - t0 < timeout:
        pr = cys("ping", socket=socket, timeout=4)
        if pr.returncode == 0 and "pong" in (pr.stdout or ""):
            revived = True
            break
        time.sleep(0.4)
    res["revived"] = revived
    epoch_after = get_boot_epoch(socket)
    res["epoch_after"] = epoch_after
    res["boot_epoch_changed"] = (epoch_before is not None and epoch_after is not None and epoch_after != epoch_before)
    if not restart_hook:
        # 관리 무결(자동기동 토대 intact) 확인 — orphan(관리 이탈)이면 재부팅/로그온 자동기동 토대가 약화됨(§15 발견3).
        # ★Windows 는 launchd_status(os.getuid 사용) 대신 supervisor_status(schtasks) — win 에서 os.getuid 미존재 크래시 회피.
        res["launchd_after"] = supervisor_status(SUPERVISOR_LABEL)
        res["launchd_managed_intact"] = (res["launchd_after"].get("state") == "managed")
    return res


def cmd_deploy(args):
    """M1 단일 게이트 deploy — quiescent→스냅샷→적용→재시작→부활→판정 일체형(생략 불가).
    단계별 저널 상태머신(P4 재개 가능)·M9 정직 enum·결정론 exit code(COMPLETE=0·그 외 비0)."""
    socket = args.socket
    ticket = args.ticket or "default"
    stub = args.stub
    apply_cmd = args.apply_cmd
    restart_hook = args.restart_hook
    include_master = args.include_master
    timeout = args.restart_timeout
    ts = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())

    # ── --plan: 단계 계획만 출력(무실행) ──
    if args.plan:
        plan = {
            "deploy": "PLAN",
            "stages": DEPLOY_STAGES,
            "ticket": ticket,
            "backend": "surrogate(stub)" if stub else "production(cys restore)",
            "apply_cmd": apply_cmd or "(없음 · 재시작 전용 deploy)",
            "restart_path": "hook(injected·격리)" if restart_hook else "launchd(%s)" % LAUNCHD_LABEL,
            "include_master": include_master,
            "restart_timeout": timeout,
            "note": "계획만 출력(무실행). 실 배포는 --plan 없이 실행하라(exit 0).",
        }
        print(json.dumps(plan, ensure_ascii=False, indent=2))
        sys.exit(0)

    j = load_deploy_journal(socket, ticket)
    cur_epoch = get_boot_epoch(socket)
    if "pre_restart_epoch" not in j:
        j["pre_restart_epoch"] = cur_epoch

    def _ok(stage):
        return j["stages"].get(stage, {}).get("status") == "ok"

    def _same_gen(stage):
        return _ok(stage) and j["stages"][stage].get("epoch") == cur_epoch

    record = {"deploy": None, "ticket": ticket, "ts": ts,
              "backend": "surrogate(stub)" if stub else "production(cys restore)",
              "pre_restart_epoch": j.get("pre_restart_epoch")}

    def _finish(verdict, exit_code, extra=None):
        record["deploy"] = verdict
        record["exit_code"] = exit_code
        if extra:
            record.update(extra)
        record["stages"] = j["stages"]
        record["honesty_note"] = (
            "★deploy 값만 신뢰하라(자기채점 금지·무출력=성공 해석 금지). COMPLETE=roster 전원 부활+세션 검증. "
            "DEGRADED=전원 부활했으나 일부 세션 미검증(escalation). 그 외(FAILED/APPLY_FAILED/RESTART_FAILED/"
            "SNAPSHOT_FAILED/BREAKER_OPEN/NO_DAEMON/SUPERVISOR_UNMANAGED)=정상 완료 아님(사람/master 개입).")
        try:
            _atomic_write_json(os.path.join(phoenix_home(socket), "deploy-%s.json" % ts), record)
        except Exception:
            pass
        save_deploy_journal(socket, ticket, j)
        print(json.dumps(record, ensure_ascii=False, indent=2))
        sys.exit(exit_code)

    # ── 재개 판정: 이 deploy 에서 이미 재시작이 일어났는가(post-restart resume) ──
    # 데몬 세대가 pre_restart_epoch 와 달라졌다 = 재시작이 이미 발생 → 재시작·apply 를 재실행하지 않는다
    # (비멱등 apply 중복 방지·§요구B 재개 신중성). restart 단계 완료 마킹의 epoch_after == 현재 세대여도 동일.
    rst = j["stages"].get("restart", {})
    post_restart = (
        rst.get("status") == "ok" and cur_epoch is not None and rst.get("epoch_after") == cur_epoch
    ) or (
        _ok("snapshot") and j.get("pre_restart_epoch") is not None
        and cur_epoch is not None and cur_epoch != j.get("pre_restart_epoch")
    )

    if not post_restart:
        # ── preflight (항상 실행 — 읽기전용 게이트) ──
        if not args.no_breaker:
            opened, attempts = breaker_check_and_record(socket)
            if opened:
                _dmark(j, "preflight", "breaker_open", "M5 OPEN(%d회/%ds)" % (len(attempts), BREAKER_T), epoch=cur_epoch)
                _finish("BREAKER_OPEN", 5, {"rollback_proposal": rollback_proposal(socket),
                        "alert": "%ss 내 %d회 배포 시도(임계 %d) — 자동 배포 정지. 사람 승인 필요."
                                 % (BREAKER_T, len(attempts), BREAKER_N)})
        pong = cys("ping", socket=socket, timeout=8)
        if not (pong.returncode == 0 and "pong" in (pong.stdout or "")):
            _dmark(j, "preflight", "no_daemon", "데몬 pong 실패 — 배포 착수 거부", epoch=cur_epoch)
            _finish("NO_DAEMON", 6, {"alert": "대상 데몬이 응답하지 않는다(ping 실패). 데몬 기동 후 재시도."})
        # ★supervisor 관리 게이트(라이브 경로만·§15 발견3 고아 재발 방어): 데몬이 감독자(mac=launchd·win=schtasks)
        #   비관리(orphan/unmanaged)면 재시작 자동기동 토대가 약화된다. 미관리면 supervisor_ensure 로 재등록 시도
        #   (mac=launchd bootstrap · win=`cys daemon install` — 기존 로직 재사용·코드 복제 금지), 여전히 비관리면
        #   SUPERVISOR_UNMANAGED(exit 8 계약 유지)로 거부. win 도 이제 진짜 KeepAlive(RestartOnFailure) 지원 —
        #   로그온 자동기동 토대(schtasks)의 관리 무결은 동일하게 게이트하고, KeepAlive 부재는 경고만(구버전 install 호환).
        if not restart_hook:  # hook 경로(격리)는 supervisor 무관 — 라이브 경로에만 적용
            sdst = supervisor_status(SUPERVISOR_LABEL)
            if sdst.get("state") != "managed":
                plist = None if IS_WINDOWS else _launchd_plist_path()
                ens = supervisor_ensure(SUPERVISOR_LABEL, plist)  # ★기존 ensure 본체 재사용(플랫폼 디스패치)
                if not ens.get("ensured"):
                    _dmark(j, "preflight", "supervisor_unmanaged",
                           "supervisor 비관리(state=%s)·재등록 실패" % sdst.get("state"),
                           epoch=cur_epoch, supervisor_before=sdst, ensure=ens)
                    _finish("SUPERVISOR_UNMANAGED", 8, {"supervisor_status": sdst, "ensure_attempt": ens,
                            "alert": "대상 데몬이 감독자 관리 밖(state=%s)이라 재시작 자동기동 토대가 없다. "
                                     "재등록 실패 — mac: `launchctl bootstrap gui/$(id -u) %s` · win: `cys daemon install` "
                                     "후 재시도(§15 발견3 재부팅/로그온 자동기동 토대)."
                                     % (sdst.get("state"), plist)})
            elif sdst.get("keepalive") is not True:
                # ★managed 지만 KeepAlive(win=RestartOnFailure·mac=KeepAlive) 부재 — 구버전 install 호환. hard fail 금지·경고만.
                #   사망 시 감시자 자동 재기동이 없어 CLI lazy-spawn/사람 개입에 의존. `cys daemon install` 재실행 권장.
                _dmark(j, "preflight", "keepalive_warn",
                       "supervisor managed 이나 KeepAlive(자동 재기동) 부재(구버전 install 의심). 배포는 계속(경고) — "
                       "`cys daemon install` 재실행으로 RestartOnFailure 갱신 권장.",
                       epoch=cur_epoch, supervisor=sdst)
        # ★roster 조기·단조 영속 — restart 가 topology 를 소거해도 desired 로스터로 restore 가능(§12 침식 면역)
        roster, _tomb = observe_and_persist_roster(socket)
        restore_targets = sorted(r for r in roster if (include_master or r != "master"))
        _dmark(j, "preflight", "ok", "roster=%s · restore대상=%s" % (sorted(roster.keys()), restore_targets),
               epoch=cur_epoch, roster=sorted(roster.keys()), restore_targets=restore_targets)
        save_deploy_journal(socket, ticket, j)

        # ── quiescent (cys drain · best-effort · 성공 단정 금지) ──
        if not _same_gen("quiescent"):
            dr = cys("drain", socket=socket, timeout=30)
            _dmark(j, "quiescent", "ok",
                   "cys drain rc=%s · %s" % (getattr(dr, "returncode", None),
                                             (dr.stdout or dr.stderr or "").strip()[:200]),
                   epoch=cur_epoch, best_effort=True,
                   note="best-effort(노드 LLM 협조·자체 watchdog 12s 의존) — 저장 성공을 단정하지 않는다")
            save_deploy_journal(socket, ticket, j)

        # ── snapshot (+ 하한2① 독립 수동 runbook) ──
        if not _same_gen("snapshot"):
            snap = _deploy_snapshot(socket, roster)
            if not snap.get("ok"):
                _dmark(j, "snapshot", "fail", "스냅샷 실패: %s" % snap.get("error"),
                       epoch=cur_epoch, gen_root=snap.get("gen_root"))
                _finish("SNAPSHOT_FAILED", 7, {"snapshot": snap,
                        "alert": "세대 스냅샷 생성 실패 — 롤백 안전망 없이 재시작 진입 금지."})
            _dmark(j, "snapshot", "ok", "gen=%s runbook=%s" % (snap.get("gen"), snap.get("runbook")),
                   epoch=cur_epoch, gen=snap.get("gen"), gen_dir=snap.get("gen_dir"),
                   runbook=snap.get("runbook"), gen_root=snap.get("gen_root"))
            record["snapshot"] = snap
            save_deploy_journal(socket, ticket, j)

        # ── apply (선택) — 실패 시 재시작 진입 금지(부작용 확산 차단) ──
        if apply_cmd and not _same_gen("apply"):
            try:
                ar = subprocess.run(apply_cmd, shell=True, capture_output=True, text=True, timeout=600)
                arc, aout, aerr = ar.returncode, (ar.stdout or "")[-600:], (ar.stderr or "")[-400:]
            except subprocess.TimeoutExpired:
                arc, aout, aerr = 124, "", "TIMEOUT"
            except (FileNotFoundError, OSError) as e:
                arc, aout, aerr = 127, "", "apply 실행 불가(%s: %s)" % (type(e).__name__, e)
            if arc != 0:
                _dmark(j, "apply", "fail", "apply rc=%s" % arc, epoch=cur_epoch, cmd=apply_cmd)
                _finish("APPLY_FAILED", 2, {"apply": {"cmd": apply_cmd, "rc": arc, "out": aout, "err": aerr},
                        "alert": "apply 실패 — 재시작 진입 중단(부작용 확산 차단). 원인 교정 후 재실행."})
            _dmark(j, "apply", "ok", "apply rc=0", epoch=cur_epoch, cmd=apply_cmd)
            record["apply"] = {"cmd": apply_cmd, "rc": 0, "out": aout}
            save_deploy_journal(socket, ticket, j)
        elif not apply_cmd:
            _dmark(j, "apply", "skip", "재시작 전용 deploy(--apply-cmd 미지정)", epoch=cur_epoch)
            save_deploy_journal(socket, ticket, j)

        # ── restart (launchd / hook) — ★재시작 확증을 hard 조건으로 ──
        #   revived(pong 복귀)만으로는 부족하다: launchd 비관리 데몬은 launchctl kill 이 빗나가도(rc≠0) 계속
        #   살아있어 pong 이 즉시 True → '재시작 안 됐는데 성공' 조용한 오복원(§10.2). 그래서 boot-epoch delta로
        #   '실제로 새 세대가 떴는가'를 확증한다. epoch 불변/미관측이면 재시작 미확증 → RESTART_FAILED(정직).
        rr = _deploy_restart(socket, restart_hook, timeout)
        record["restart"] = rr
        restart_confirmed = bool(rr.get("revived")) and bool(rr.get("boot_epoch_changed"))
        if not restart_confirmed:
            if not rr.get("revived"):
                reason = "재시작 후 pong 미복귀(timeout %ss)" % timeout
            else:
                reason = ("재시작 미확증 — boot-epoch %s→%s 불변/미관측(launchctl kill 빗나감·launchd 비관리(고아) 의심)"
                          % (rr.get("epoch_before"), rr.get("epoch_after")))
            _dmark(j, "restart", "fail", reason, epoch=cur_epoch, epoch_after=rr.get("epoch_after"),
                   boot_epoch_changed=rr.get("boot_epoch_changed"))
            runbook = j["stages"].get("snapshot", {}).get("runbook")
            _finish("RESTART_FAILED", 4, {
                "runbook_path": runbook, "restart_reason": reason,
                "alert": "데몬 재시작 실패/미확증(%s). 독립 수동 복원 runbook 을 실행하라: %s (§11.1 하한2 독립 경로). "
                         "launchd 비관리(고아) 의심 시 `javis_phoenix.py --socket <s> launchd-ensure --label %s "
                         "--plist %s` 로 관리 복원 후 재시도. launchd 진단: launchctl print gui/$(id -u)/%s"
                         % (reason, runbook, LAUNCHD_LABEL, _launchd_plist_path(), LAUNCHD_LABEL)})
        _dmark(j, "restart", "ok",
               "재시작 확증(path=%s epoch %s→%s changed=%s)"
               % (rr.get("path"), rr.get("epoch_before"), rr.get("epoch_after"), rr.get("boot_epoch_changed")),
               epoch=cur_epoch, epoch_after=rr.get("epoch_after"), revived=True,
               boot_epoch_changed=rr.get("boot_epoch_changed"),
               launchd_managed_intact=rr.get("launchd_managed_intact"))
        save_deploy_journal(socket, ticket, j)
    else:
        record["restart"] = {"skipped": True, "epoch": cur_epoch,
                             "reason": "재개(post-restart) — 이 deploy 의 재시작이 이미 완료됨(세대 일치). "
                                       "비멱등 apply/재시작을 재실행하지 않는다."}

    # ── restore (run_restore 재사용 — 코드 복제 금지·P2). deploy 가 crash-loop 단위이므로 내부 차단기는 끈다. ──
    restore_res = run_restore(socket, ticket="deploy-" + ticket, stub=stub, no_breaker=True,
                              roles=None, include_master=include_master, stub_sids=None, print_result=False)
    # ★P2-7/W1: deploy 내부 restore 가 다른 restore(콜드부트 auto)의 lease 에 막혀 LEASE_HELD 를 받으면,
    #   과거엔 아래 verdict 분기가 else 로 떨어져 FAILED(허위)로 강등됐다. lease 는 짧게 잡히므로 backoff 재시도로
    #   회복을 노리고, 2회 재시도 후에도 여전히 held 면 LEASE_HELD 를 정직히 보고한다(FAILED 아님 — 다른 restore 가
    #   부활을 담당). 진짜 죽은(stale) lease 는 flock 이 프로세스 사망 시 자동 해제하므로 여기 오지 않는다.
    _lease_retry = 0
    while restore_res.get("phoenix_restore") == "LEASE_HELD" and _lease_retry < 2:
        _lease_retry += 1
        # ★gemini minor: 재시도 대기를 deploy 저널에 이벤트로 기록(대기 이력 관측 · 반복 run_restore 로 인한 이력 꼬임 방지).
        _dmark(j, "restore", "lease_retry",
               "다른 restore lease 보유 — %d/2회차 재시도 대기(backoff %.1fs)" % (_lease_retry, SPAWN_BACKOFF * _lease_retry),
               epoch=get_boot_epoch(socket), lease_retry=_lease_retry)
        save_deploy_journal(socket, ticket, j)
        time.sleep(SPAWN_BACKOFF * _lease_retry)
        restore_res = run_restore(socket, ticket="deploy-" + ticket, stub=stub, no_breaker=True,
                                  roles=None, include_master=include_master, stub_sids=None, print_result=False)
    record["restore"] = restore_res
    _dmark(j, "restore", "ok", "phoenix_restore=%s completeness=%s"
           % (restore_res.get("phoenix_restore"), restore_res.get("completeness")),
           epoch=get_boot_epoch(socket), phoenix_restore=restore_res.get("phoenix_restore"),
           completeness=restore_res.get("completeness"))
    save_deploy_journal(socket, ticket, j)

    # ── verdict (M9 정직 enum → 결정론 exit code) ──
    final = restore_res.get("phoenix_restore")
    completeness = restore_res.get("completeness")
    incomplete = restore_res.get("incomplete_roles") or []
    if final == "LEASE_HELD":
        # ★P2-7/W1: 재시도 후에도 다른 restore 가 lease 보유 — 이 deploy 의 실패가 아니라 부활 담당 이관.
        #   FAILED(재시도 소진 미부활)와 구분해 정직 보고. code 3=미확증(escalation 필요)이되 hard fail(1) 아님.
        verdict, code = "LEASE_HELD", 3
    elif completeness == "NOOP" and final == "NOOP":
        verdict, code = "COMPLETE", 0        # 재시작은 됐고 부활 대상 0(전원 이미 생존) — 배포 성공
    elif completeness == "COMPLETE" and final in ("VERIFIED", "VERIFIED_FRESH"):
        verdict, code = "COMPLETE", 0
    elif completeness == "COMPLETE":
        verdict, code = "DEGRADED", 3        # 전원 부활했으나 일부 세션 미검증(nodes up·session unverified)
    else:
        verdict, code = "FAILED", 1          # 일부 역할 미부활(INCOMPLETE) 등 — 정직한 실패
    if verdict == "COMPLETE" and not args.no_breaker:
        breaker_reset(socket)                # 성공 배포는 차단기 창 리셋
    _dmark(j, "verdict", "ok", "%s (exit %d)" % (verdict, code), epoch=get_boot_epoch(socket))
    unver = [r for r, o in (restore_res.get("per_role_outcome") or {}).items() if o == "unverified"]
    runbook = j["stages"].get("snapshot", {}).get("runbook")
    esc = None
    if verdict != "COMPLETE":
        esc = ("★%s — 정상 완료 아님. incomplete_roles=%s · 세션미검증=%s. 독립 수동 복원 runbook: %s (§11.1 하한2). "
               "재개는 같은 ticket 으로 재실행(완료 단계 skip)." % (verdict, incomplete, unver, runbook))
    _finish(verdict, code, {"completeness": completeness, "phoenix_restore": final,
            "incomplete_roles": incomplete, "fresh_fallback_roles": restore_res.get("fresh_fallback_roles"),
            "runbook_path": runbook, "escalation": esc})


# ------------------------------------------------------------------ main

def main():
    global CYS
    # ★B1 self-test(임베드 추출 직후 cysd 가 호출): 추출된 phoenix 가 실행가능한지만 확인한다 — 데몬·cys 해석·
    #   상태파일 무접촉. argparse(서브커맨드 required)·_resolve_cys 이전에 조기 종료해 순수 실행성만 검증.
    #   설계 §2 B1③ 수용조건. --pack-version 은 별칭(설계 표기 정합).
    if "--selftest" in sys.argv or "--pack-version" in sys.argv:
        sys.stdout.write("phoenix selftest ok (proto=%s)\n" % PHOENIX_PROTOCOL_VERSION)
        sys.stdout.flush()
        sys.exit(0)
    # ★Windows 패리티(S1~S5): 재부팅 자동기동 토대는 mac=launchd·win=schtasks 로 플랫폼 디스패치하고, 경로/소켓은
    # named pipe→state_dir 매핑으로 해소한다. 과거 os.name=="nt" hard-gate 는 제거됐다 — 전 서브커맨드 Windows 동작.
    ap = argparse.ArgumentParser(description="불사조 부활 저널 상태머신 MVP (M1 게이트)")
    ap.add_argument("--socket", help="대상 데몬 소켓(격리 하네스 소켓 권장 — 라이브 무접촉)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("restore"); p.add_argument("--ticket"); p.add_argument("--stub", action="store_true")
    p.add_argument("--stub-sids", help='role→observed session_id JSON(오복원 시뮬레이션용)')
    p.add_argument("--roles", nargs="*"); p.add_argument("--include-master", action="store_true")
    p.add_argument("--no-breaker", action="store_true")
    # ★W2 콜드부트: cysd가 소켓 바인드 직후 detached 스폰. master 포함 강제(부활할 상위가 없으므로)
    #   — 단 master가 묘비면 부활 금지(1급 원칙이 include-master보다 상위, tombstone 병합으로 자동 배제).
    p.add_argument("--auto", action="store_true", help="콜드부트 자동 복원(master 포함·묘비는 배제)")
    sub.add_parser("reconcile")
    sub.add_parser("status")
    rp = sub.add_parser("roster")  # Phase 4: desired 로스터 현황(침식 면역) + Phase7 부서
    rp.add_argument("--rebase", action="store_true",
                    help="★C3: 설명-가능-축소 불변식을 1회 우회해 현재 관측을 강제 수용(운영자 명시 재기반)")
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
    dp = sub.add_parser("deploy")  # ★Phase3: quiescent→스냅샷→적용→재시작→부활→판정 일체형(M1)
    dp.add_argument("--ticket")
    dp.add_argument("--stub", action="store_true")                 # 격리 하네스 surrogate 백엔드
    dp.add_argument("--plan", action="store_true")                 # 단계 계획만 출력(무실행)
    dp.add_argument("--apply-cmd")                                 # 적용 셸 명령(미지정=재시작 전용)
    dp.add_argument("--restart-hook")                              # 격리 재시작 프리미티브 주입(미지정=launchd)
    dp.add_argument("--restart-timeout", type=int, default=60)     # 재시작 후 부활(pong) 대기 상한
    dp.add_argument("--include-master", action="store_true")       # 기본 master 제외(실행자)
    dp.add_argument("--no-breaker", action="store_true")

    args = ap.parse_args()
    # ★W1/B3·§5-1: cys 실행 경로를 여기서(소켓 인지 후) 해석한다 — PHOENIX_CYS > which('cys') > 표준경로
    #   폴백(identity-check·STRICT/하네스 금지). 리터럴 'cys' 최종 폴백 제거로 FileNotFoundError 침묵사를 차단.
    #   미해석/불일치는 _resolve_cys 내부에서 die(exit 6)로 정직 정지한다(라이브 실증 2026-07-06 근원 수리).
    # ★W5(⑥ 수리·CI run 28778120380 실증): `deploy --plan` 은 계획만 출력하고 sys.exit(0) 하는 **무실행** 경로라
    #   cys 를 전혀 호출하지 않는다. 그런데 _resolve_cys 는 identity 프로브(subprocess `cys status/phoenix-identity`)를
    #   돌리는데, 데몬이 없는 환경(스모크 ⑥)에서 Windows 는 이 프로브가 12s×재시도로 지연돼 --plan 이 30s 타임아웃
    #   (exit 124)됐다. 무실행 서브커맨드는 CYS 해석을 lazy 로 건너뛴다(실 subprocess 필요 경로는 모두 CYS 해석 후에
    #   실행되므로 무영향). die 경로(cys 부재)도 --plan 에선 부적절(계획 출력에 cys 불요)이라 함께 해소된다.
    _no_exec_plan = args.cmd == "deploy" and getattr(args, "plan", False)
    CYS = None if _no_exec_plan else _resolve_cys(args.socket)
    {
        "restore": cmd_restore, "reconcile": cmd_reconcile, "status": cmd_status,
        "roster": cmd_roster, "inherit": cmd_inherit, "tombstone": cmd_tombstone,
        "gen-manual": cmd_gen_manual, "gen-protect": cmd_gen_protect, "deploy": cmd_deploy,
        "launchd-status": cmd_launchd_status, "launchd-ensure": cmd_launchd_ensure,
    }[args.cmd](args)


if __name__ == "__main__":
    main()
