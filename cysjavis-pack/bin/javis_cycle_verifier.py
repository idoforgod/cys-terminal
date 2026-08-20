#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_cycle_verifier.py — 결정론 검증자 (컴포넌트 2 · 전용 리스너 pane 상주).

설계 정본: 프로젝트 _round/fullauto-cycle/DESIGN_full-auto-cycle.md · 운영 가이드: docs/GUIDE-fullauto-cycle-KR.md (v2 §2 컴포넌트 2).

무엇을 하나:
  watch      — 포그라운드 워처. feed 의 pending `cycle-verify` 요청을 결정론으로 심사해
               `cys feed reply <id> allow|deny` 로 판정하고, 30초마다 heartbeat 파일을 touch 한다.
  adjudicate — 단발 심사(테스트·운영 점검용). 기본은 **판정만 출력**하고 회신하지 않는다(--apply 필요).
  self-test  — 데몬 없이 픽스처로 본문 파싱·3분기 판정·동시성 append 를 검증한다.

★배치 규칙 (cysd state.rs:1046-1074 `is_self_approval` 실측):
  - 반드시 `cys new-surface` 로 만든 **pane 안에서 포그라운드**로 돌린다. pane 자식은 조상 추적으로
    surface 에 귀속되어 feed.reply 가 수리된다.
  - **detach·setsid·데몬 spawn 금지** — 외부 프로세스인데 어떤 surface 에도 귀속되지 않으면
    cysd 가 자기승인 탈출로 보고 fail-closed 거부한다(self_approval_denied).
  - **맨 셸 pane 금지** — cycle-agent 가 주입하는 안내문에 backtick 이 들어 있어 셸이 command
    substitution 으로 '고무도장' 승인을 찍을 수 있다. 이 워처는 stdin 을 별도 스레드로 읽어
    **그냥 버린다**(주입 텍스트 무해 흡수 — 절대 실행·파싱하지 않는다).
  - **LLM pane 금지** — happy path 에 LLM 판단 0회가 계약이다(설계 C2).
  - ★승인 토큰 자동 첨부 코드는 어떤 형태로도 작성하지 않는다(설계 C1 · self-test 가 소스를 검사).

심사 규칙 [v2.1 ① — baseline 계약. R1 codex BLOCK 수용]:
  body 1줄 = `<절대경로> (sha256: <hex>)` 또는 `<절대경로> (미생성 — 생성 자체가 증거)`
             (cys.rs `handshake_file_line` 실측 형식). **body 의 해시는 참고일 뿐이다** —
             그것은 cycle-agent 가 저장 검증 통과 *후* 계산한 값이라 '저장 전 대비 변화'를
             증명하지 못한다.
  진실 기준 = autopilot 이 cycle-agent 실행 **직전** 기록한 baseline 레코드
             (`STATE_DIR/baselines/cycle-<id>.json` — 파일별 {exists, sha256, mtime} + started_at).
  매핑     = in-flight lease(phase=fired) + cycle_id + **파일 집합 정확 일치** + 시간 순서 + role.
  allow    : 매핑 성립 AND **모든 파일**이 [자체 재계산 sha256 ≠ baseline sha256]
             AND [mtime > started_at]
  deny(stale)     : 매핑은 됐는데 미재기록 파일이 1건이라도 있음
  deny+escalation : baseline 부재 · 파일 집합 불일치 · 매핑 불일치 · 소실/판독 불가 · 본문 파싱 불능
  allow 발신 직전 재확인: kill-switch(2중 파일 + gate-check) · 대상 유휴(queue 0·idle 상승)
  reply 후 resolved 재조회 self-verify (미착지 = hard fail · 원장 기록)

exit: 0=정상 / 1=오류 / 2=게이트 미통과(정상 skip) / 4=kill-switch / 5=원장 미완결 fail-closed
"""
import argparse
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import threading
import time

# ★Windows 즉사 차단(P0): top-level `import fcntl` 은 Windows 에 그 모듈이 없어
#   ModuleNotFoundError 로 이 스크립트 **전체**를 불능화한다 — javis_org.py:9-22 가 같은
#   사고를 먼저 겪고 msvcrt 폴백으로 지혈한 선례가 있다.
#   여기서는 **이름 `fcntl` 을 그대로 유지하는 shim** 을 바인딩한다. 아래 사용처의
#   `fcntl.flock(fd, fcntl.LOCK_EX)` 호출을 한 글자도 바꾸지 않으므로
#     · posix: 진짜 fcntl 모듈이 그대로 바인딩되어 동작이 **바이트 단위로 보존**된다.
#     · Windows: msvcrt 바이트락으로만 접힌다.
#   ★이 형태를 고른 결정적 이유: 유일한 사용처(`_append_ledger`)가 아래 CONTRACT BLOCK v1
#     **안**에 있고 그 블록은 javis_cycle_autopilot.py 와 **바이트 동일**해야 한다
#     (self-test contract-parity). 사용처를 건드리는 어떤 수리도 즉시 parity 를 깨뜨린다 —
#     import 층에서만 접는 이 shim 만이 두 계약을 동시에 만족한다.
try:
    import fcntl
except ImportError:  # Windows
    import msvcrt as _msvcrt

    class _FcntlShim(object):
        LOCK_EX = 2      # 값은 posix fcntl 과 동일 — 호출부가 이 상수만 쓰므로 자기정합
        LOCK_UN = 8

        @staticmethod
        def flock(fd, op):
            # msvcrt.locking 은 '현재 위치의 1바이트' 영역락이라 잠글 때와 풀 때의 위치가
            # 같아야 짝이 맞는다 → 위치를 0으로 고정해 걸고 원복한다(중간 lseek 와 무관).
            pos = os.lseek(fd, 0, os.SEEK_CUR)
            try:
                os.lseek(fd, 0, os.SEEK_SET)
                try:
                    _msvcrt.locking(
                        fd, _msvcrt.LK_LOCK if op == _FcntlShim.LOCK_EX else _msvcrt.LK_UNLCK, 1)
                except OSError:
                    pass          # best-effort — 락 실패가 기록 자체를 막지는 않는다
            finally:
                os.lseek(fd, pos, os.SEEK_SET)

    fcntl = _FcntlShim()

# ═══════════════════════ CONTRACT BLOCK v1 START ═══════════════════════
# ★이 블록은 javis_cycle_autopilot.py / javis_cycle_verifier.py 에 **바이트 동일**하게 존재한다.
# 한쪽만 고치면 양쪽 self-test 의 contract-parity 검사가 즉시 실패한다(이음매 드리프트 차단).
# 고정 계약(발주 브리프 · 설계 v2 §2) — 임의 변경 금지.

def _resolve_project_root():
    """프로젝트 루트 결정론 해석 — 배포 이식성(개인 경로 하드코딩 금지).

    우선순위: env CYS_PROJECT_ROOT → cwd 상향탐색(_round/ 보유 디렉토리) →
    $HOME/_round/ACTIVE_PROJECT 포인터(save-state.sh 동형 관행) → $HOME 폴백.
    스케줄 스폰(cwd 비보장) 운영은 잡 command 에 CYS_PROJECT_ROOT 명시를 권장한다.
    """
    env = os.environ.get("CYS_PROJECT_ROOT", "").strip()
    if env and os.path.isdir(os.path.join(env, "_round")):
        return env
    d = os.getcwd()
    prev = None
    while d and d != prev:
        if os.path.isdir(os.path.join(d, "_round")):
            return d
        prev, d = d, os.path.dirname(d)
    home = os.path.expanduser("~")
    try:
        with open(os.path.join(home, "_round", "ACTIVE_PROJECT"), "r", encoding="utf-8") as f:
            cand = f.readline().strip()
        if cand and os.path.isdir(os.path.join(cand, "_round")):
            return cand
    except OSError:
        pass
    return home


PROJECT = _resolve_project_root()
CYCLE_LOG = os.path.join(PROJECT, "_round", "cycle_autopilot_log.jsonl")
STATE_DIR = os.path.join(os.path.expanduser("~"), ".local", "state", "cys", "cycle_autopilot")
STATE_JSON = os.path.join(STATE_DIR, "state.json")
HEARTBEAT = os.path.join(STATE_DIR, "verifier.heartbeat")
BASELINE_DIR = os.path.join(STATE_DIR, "baselines")   # [v2.1] 사이클별 baseline 원장

EXIT_OK = 0       # 정상
EXIT_ERR = 1      # 오류
EXIT_GATE = 2     # 게이트 미통과(정상 skip) — ※tick 은 이 코드를 밖으로 내보내지 않는다(§tick 계약)
EXIT_KILL = 4     # kill-switch
EXIT_LEDGER = 5   # 원장 미완결 fail-closed

# [v2.1 ④] phase 의미론 교정: 자식 종료는 clear 성공을 뜻하지 않는다.
#   fired → executor_exited(자식 종료·exit 기록) → 사후검증이 셋 중 하나로 종결:
#     cleared_verified  실효 확인(신규 세션·상대 급락·nonce·복구파일)
#     failed_preclear   clear 미실행이 **확인**됨(측정 유효한데 세션 그대로)
#     indeterminate     판정 불능(측정 무효·조회 실패) — 성공으로도 실패로도 세지 않는다
#   failed 는 집행 자체가 깨진 경우(kill-switch 중단·집행자 사망·선통보 실패).
PHASES = ("armed", "prenotified", "fired", "executor_exited",
          "cleared_verified", "failed_preclear", "indeterminate", "failed")
TERMINAL_PHASES = ("cleared_verified", "failed_preclear", "indeterminate", "failed")
SUCCESS_PHASE = "cleared_verified"
PHASE_NEXT = {
    "armed": ("prenotified", "failed"),
    "prenotified": ("fired", "failed"),
    "fired": ("executor_exited", "failed"),
    "executor_exited": ("cleared_verified", "failed_preclear", "indeterminate", "failed"),
    "cleared_verified": (),
    "failed_preclear": (),
    "indeterminate": (),
    "failed": (),
}
LOG_KEYS = ("ts", "cycle_id", "phase", "role", "surface", "detail")
LOG_MAX_BYTES = 4096

HEARTBEAT_MAX_AGE = 90.0      # 게이트6 — 검증자 워처 생존 판정 창
HEARTBEAT_TOUCH_SECS = 30.0   # 워처 touch 주기
STAGE2_WINDOW = 120.0         # cys cycle-agent --timeout 기본값 = 검증자 신선도 기준선 폭
CYCLE_AGENT_TIMEOUT = 120     # --timeout (예산표: 120*2 + settle 75 + 검증 <= 510s)
VERIFIER_ROLE = "cycle-verifier"
CYS = "cys"
RUN_TIMEOUT = 25.0
PAUSED_BASENAME = "AUTOPILOT_PAUSED"


def pack_dir():
    """$CYS_PACK_DIR 우선, 없으면 ~/.cys/pack (팩 스크립트 관행과 동일)."""
    return os.environ.get("CYS_PACK_DIR", "").strip() or os.path.join(
        os.path.expanduser("~"), ".cys", "pack")


def paused_paths():
    """kill-switch 파일 경로 2중(절대경로 핀). 하나라도 존재하면 무집행."""
    return [os.path.join(pack_dir(), PAUSED_BASENAME),
            os.path.join(PROJECT, "_round", PAUSED_BASENAME)]


def state_socket_dir():
    """cys 소켓 부모 디렉터리 = 데몬 상태 디렉터리(feed.jsonl 이 사는 곳)."""
    sock = (os.environ.get("AITERM_SOCKET", "").strip()
            or os.environ.get("CYS_SOCKET", "").strip())
    if sock:
        return os.path.dirname(os.path.abspath(sock))
    return os.path.join(os.path.expanduser("~"), ".local", "state", "cys")


def nonce_for(cycle_id):
    """사이클 nonce — --resume-text 에 실려 신규 세션 jsonl 에서 grep 된다."""
    return "cycle-%d" % int(cycle_id)


def new_cycle_id():
    return int(time.time())


def run(cmd, timeout=RUN_TIMEOUT, stdin_text=None):
    """subprocess 러너 — (rc, stdout, stderr). 예외도 rc!=0 로 정규화(fail-soft)."""
    try:
        p = subprocess.run(cmd, capture_output=True, text=True,
                           timeout=timeout, input=stdin_text)
        return p.returncode, p.stdout or "", p.stderr or ""
    except Exception as e:  # noqa: BLE001 — 러너는 절대 예외를 올리지 않는다
        return 127, "", "runner error: %s" % e


def no_send():
    """CYS_AUTOPILOT_NO_SEND=1 이면 모든 아웃바운드 주입을 봉인(테스트·개발 안전핀)."""
    return os.environ.get("CYS_AUTOPILOT_NO_SEND", "").strip() in ("1", "true", "yes")


def kill_switch(runner=run):
    """(killed:bool, reason:str). 판정 불능(데몬 무응답)도 killed=True — fail-closed."""
    for p in paused_paths():
        if os.path.exists(p):
            return True, "PAUSED 파일 존재: %s" % p
    rc, _out, err = runner([CYS, "gate-check"])
    if rc == 4:
        return True, "cys gate-check exit 4 (daemon paused)"
    if rc != 0:
        return True, "cys gate-check exit %d (판정 불능 → fail-closed): %s" % (rc, err.strip()[:120])
    return False, ""


def fetch_status(runner=run):
    """cys status --json → dict. 조회 불가 시 None(= 무집행 신호)."""
    rc, out, _err = runner([CYS, "status", "--json"])
    if rc != 0:
        return None
    try:
        d = json.loads(out)
    except ValueError:
        return None
    return d if isinstance(d, dict) else None


def surface_row(status, role):
    """role 로 살아있는 surface row 1건(동명 다수면 surface_id 최소 — 결정론)."""
    rows = []
    for s in (status or {}).get("surfaces") or []:
        if not isinstance(s, dict):
            continue
        if s.get("role") != role:
            continue
        if s.get("exited") is True:
            continue
        rows.append(s)
    if not rows:
        return None
    rows.sort(key=lambda r: r.get("surface_id") or 0)
    return rows[0]


def sha256_file(path):
    """파일 sha256 hex. 없거나 못 읽으면 None (cycle-agent handshake 본문과 동일 알고리즘)."""
    try:
        h = hashlib.sha256()
        with open(path, "rb") as f:
            while True:
                b = f.read(1 << 16)
                if not b:
                    break
                h.update(b)
        return h.hexdigest()
    except OSError:
        return None


def mtime_of(path):
    try:
        return os.stat(path).st_mtime
    except OSError:
        return None


def file_state(path):
    """[v2.1 ①] baseline 기록·검증자 심사 공용 파일 관측 — {exists, sha256, mtime}."""
    try:
        st = os.stat(path)
    except OSError:
        return {"exists": False, "sha256": None, "mtime": None}
    return {"exists": True, "sha256": sha256_file(path), "mtime": st.st_mtime}


def baseline_path(cycle_id):
    """[v2.1 ①] 사이클 baseline 레코드 경로. 검증자가 같은 규칙으로 찾아 읽는다."""
    return os.path.join(BASELINE_DIR, "cycle-%d.json" % int(cycle_id))


def read_json_file(path):
    """JSON 객체 1건 읽기. 부재·손상은 None(호출자가 fail-closed 판정)."""
    try:
        with open(path, "r", encoding="utf-8") as f:
            d = json.load(f)
    except (OSError, ValueError):
        return None
    return d if isinstance(d, dict) else None


def ledger_append(rtype, actor, fields, timeout=15):
    """[v2.1 ⑤] STATE_LEDGER(javis_state_ledger.py) 보조 기록. 실패해도 예외 없음.

    사이클의 진실원은 CYCLE_LOG 다. STATE_LEDGER 는 SessionStart 주입면(요약 표면)이라
    실패가 사이클 판정에 영향을 주면 안 된다 — 그래서 best-effort 다.
    """
    here = os.path.dirname(os.path.abspath(__file__))
    for cand in (os.path.join(here, "javis_state_ledger.py"),
                 os.path.join(pack_dir(), "bin", "javis_state_ledger.py")):
        if os.path.exists(cand):
            cmd = [sys.executable, cand, "append", "--type", rtype, "--actor", actor]
            for k in sorted((fields or {}).keys()):
                cmd += ["--field", "%s=%s" % (k, fields[k])]
            rc, _o, e = run(cmd, timeout=timeout)
            return rc == 0, ("" if rc == 0 else "rc=%d %s" % (rc, e.strip()[:120]))
    return False, "javis_state_ledger.py 부재"


def log_append(record, path=None):
    """CYCLE_LOG 1줄 append — O_APPEND + flock + **단일 os.write**(인터리빙 0).

    레코드 키는 LOG_KEYS 고정. LOG_MAX_BYTES 초과 시 detail 만 축약해 라인 무결을 지킨다.
    """
    path = path or CYCLE_LOG
    rec = {}
    for k in LOG_KEYS:
        rec[k] = record.get(k)
    if rec.get("ts") is None:
        rec["ts"] = time.time()
    line = json.dumps(rec, ensure_ascii=False, sort_keys=True)
    data = (line + "\n").encode("utf-8")
    if len(data) > LOG_MAX_BYTES:
        keep = json.dumps(rec.get("detail"), ensure_ascii=False, sort_keys=True)
        rec["detail"] = {"truncated": True, "head": keep[:800]}
        data = (json.dumps(rec, ensure_ascii=False, sort_keys=True) + "\n").encode("utf-8")
        if len(data) > LOG_MAX_BYTES:
            rec["detail"] = {"truncated": True}
            data = (json.dumps(rec, ensure_ascii=False, sort_keys=True) + "\n").encode("utf-8")
    d = os.path.dirname(path)
    if d and not os.path.isdir(d):
        os.makedirs(d, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        os.write(fd, data)
    finally:
        try:
            fcntl.flock(fd, fcntl.LOCK_UN)
        finally:
            os.close(fd)


def read_ledger(path=None):
    """(records, bad_lines). bad_lines>0 = 원장 손상 → 호출자가 fail-closed 판정."""
    path = path or CYCLE_LOG
    records, bad = [], 0
    try:
        with open(path, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    r = json.loads(line)
                except ValueError:
                    bad += 1
                    continue
                if isinstance(r, dict):
                    records.append(r)
                else:
                    bad += 1
    except FileNotFoundError:
        return [], 0
    except OSError:
        return [], -1  # 읽기 불가 = 판정 불능
    return records, bad


def push_line(target_role, text, runner=run):
    """cys send + send-key Return 1세트. no_send() 이면 봉인(주입 0)."""
    if no_send():
        return False, "no_send"
    rc, _o, e = runner([CYS, "send", "--to", target_role, text])
    if rc != 0:
        return False, "send rc=%d %s" % (rc, e.strip()[:120])
    rc2, _o2, e2 = runner([CYS, "send-key", "--to", target_role, "Return"])
    if rc2 != 0:
        return False, "send-key rc=%d %s" % (rc2, e2.strip()[:120])
    return True, ""


def escalate(text, runner=run):
    """master 로 1줄 escalation push. 실패해도 원장 기록이 진실원이라 예외를 올리지 않는다."""
    ok, why = push_line("master", text, runner=runner)
    return ok, why
# ═══════════════════════ CONTRACT BLOCK v1 END ═══════════════════════


# ── (블록 밖 · I3 선례) R1 — Windows 상태 dir 정합 override ──────────────────
#
# ⚠이 재정의가 CONTRACT BLOCK v1 **밖**에 있는 것은 의도다(회피가 아니라 범위 준수):
#   블록은 javis_cycle_autopilot.py 와 **바이트 동일**해야 하고 self-test [7]의
#   contract-parity 검사가 그것을 박제한다. 그래서 블록은 원형 그대로 두고 **모듈
#   수준에서 심볼만 덮어쓴다** — 선례 = autopilot 의 I3 escalate 재정의
#   (javis_cycle_autopilot.py:386-403). 이 파일 안의 소비자는 feed_jsonl_path 하나뿐이며
#   아래 재정의를 본다. 블록을 옮길 때(양쪽 동시 갱신)는 이 override 를 블록으로 흡수하라.
#
# 왜 필요한가(실측): 블록 원형 state_socket_dir 는 "소켓 dirname = 상태 dir"(unix 전제)를
#   가정한다. Windows 의 소켓은 named pipe(`\\.\pipe\cys`)라 파일시스템 부모가 없다 —
#   dirname 은 `\\.\pipe` 로 접히고, env 부재 폴백 ~/.local/state/cys 도 데몬 정본
#   %LOCALAPPDATA%\cys(src/bin/cysd/state.rs::pipe_slug/state_dir)와 불일치한다.
#   → feed.jsonl 을 영영 못 읽어 전건 V_DENY_AMBIGUOUS(Windows 검증자 무력화).

# 번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
# append 인 이유: 발견이 목적이지 기존 항목의 precedence 강등이 아니다(선례: javis_memory.py:41-43).
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

_SNAPSHOT_MOD = None            # None=미시도 · False=로드 실패(LOUD 1회 박제) · 모듈=성공


def _snapshot_mod():
    """형제 파일 javis_state_snapshot 로드·캐시 — Windows 파이프→상태 dir 매핑 규칙의
    단일 소스 재사용(중복 구현 금지 · javis_phoenix.py _snap_mod 동형). 실패 시 None
    (LOUD stderr 1줄 · 실패도 캐시 — watch 2초 폴링의 스팸 방지)."""
    global _SNAPSHOT_MOD
    if _SNAPSHOT_MOD is None:
        try:
            import javis_state_snapshot as _s
            _SNAPSHOT_MOD = _s
        except Exception as e:  # noqa: BLE001 — 아래 최후 폴백이 있어 fail-soft
            _SNAPSHOT_MOD = False
            print("⚠ [cycle-verifier] javis_state_snapshot 로드 실패 — Windows 상태 dir "
                  "매핑을 최후 폴백(LOCALAPPDATA 직접 조립·부서 슬러그 미지원)으로 접는다: %s" % e,
                  file=sys.stderr)
    return _SNAPSHOT_MOD or None


_state_socket_dir_contract = state_socket_dir   # 블록 원형 보존(POSIX 동작 바이트 불변)


def state_socket_dir():
    """[R1] Windows: 소켓 dirname ≠ 상태 dir(named pipe 엔 파일시스템 부모가 없다) —
    데몬 정본(%LOCALAPPDATA%\\cys · state.rs pipe_slug/state_dir 규칙)을 따른다.
    매핑 단일 소스 = javis_state_snapshot._win_state_dir_for_socket 재사용(복제 금지)."""
    if os.name != "nt":
        return _state_socket_dir_contract()
    sock = (os.environ.get("AITERM_SOCKET", "").strip()
            or os.environ.get("CYS_SOCKET", "").strip())  # 키 순서 = 블록 원형 그대로(변경 금지)
    snap = _snapshot_mod()
    if snap is not None:
        return snap._win_state_dir_for_socket(sock or "\\\\.\\pipe\\cys")
    return _fallback_state_dir(sock or "\\\\.\\pipe\\cys")


def _fallback_state_dir(sock, localappdata=None):
    """최후 폴백(스냅샷 모듈 소실) — state.rs **기본 데몬** 규칙만 직접 접는다.

    [codex·gemini R1 교차 지적 수용 2026-08-20] 부서 파이프(cys-dept-*)까지 base/cys 로
    접으면 부서 격리가 붕괴한다(남의 feed 를 읽고 판정). 슬러그 규칙 복제는 사본 드리프트라
    금지(정본 = javis_state_snapshot 한 곳)이므로, 부서 파이프는 **실존 불가 센티널 경로**를
    돌려 fail-closed 한다 — 이후 feed 조회가 비어 V_DENY_AMBIGUOUS 로 접힌다(검증자 철학
    '모호=deny' 정합). localappdata 인자는 self-test 주입용(프로덕션은 env 경로).
    """
    base = localappdata or os.environ.get("LOCALAPPDATA") or os.path.join(
        os.path.expanduser("~"), "AppData", "Local")
    if "cys-dept-" in sock:
        print("⚠ [cycle-verifier] 부서 파이프 상태 dir 매핑 불가(스냅샷 모듈 소실) — "
              "fail-closed", file=sys.stderr)
        return os.path.join(base, "cys", ".dept-mapping-unavailable")
    return os.path.join(base, "cys")


# ── 검증자 고유 상수 ──────────────────────────────────────────────────
FEED_KIND = "cycle-verify"
POLL_SECS = 2.0                 # pending 폴링 주기
ALLOW_RECHECK_MAX = 108.0       # allow 직전 유휴 대기 상한(stage3 timeout 120s 내·S1 실측 튜닝: 90s는 워커 턴 종료 리듬보다 짧았음)
RECHECK_IDLE_MIN = 5.0          # 대상 유휴 재확인 하한(S1 실측 튜닝)
FEED_LOOKUP_RETRY = 4           # feed.jsonl 반영 지연 흡수
SELF_VERIFY_RETRY = 5

V_ALLOW = "allow"
V_DENY_STALE = "deny_stale"
V_DENY_AMBIGUOUS = "deny_ambiguous"

# cys.rs handshake_file_line 실측 형식
_RE_HASHED = re.compile(r"^(?P<path>.+?) \(sha256: (?P<hash>[0-9a-f]{64})\)$")
_RE_ABSENT = re.compile(r"^(?P<path>.+?) \(미생성 .*\)$")
_RE_TITLE_ROLE = re.compile(r"^\[CYCLE-VERIFY\]\s+(?P<role>\S+)\s")


def feed_jsonl_path():
    return os.path.join(state_socket_dir(), "feed.jsonl")


# ── 본문 파싱·심사 (순수) ─────────────────────────────────────────────
def parse_body(body):
    """handshake 본문 → [(path, expected_hash|None)] + 파싱 실패 줄 목록.

    반환 (entries, unparsed). unparsed 가 비어있지 않으면 호출자는 모호(deny)로 판정한다 —
    본문 형식이 계약과 다르면 '전량 대조'를 수행했다고 말할 수 없기 때문이다.
    """
    entries, unparsed = [], []
    for raw in (body or "").splitlines():
        line = raw.strip()
        if not line:
            continue
        m = _RE_HASHED.match(line)
        if m:
            entries.append((m.group("path"), m.group("hash")))
            continue
        m = _RE_ABSENT.match(line)
        if m:
            entries.append((m.group("path"), None))
            continue
        unparsed.append(line[:200])
    return entries, unparsed


def role_from_title(title):
    m = _RE_TITLE_ROLE.match((title or "").strip())
    return m.group("role") if m else None


def default_statfn(path):
    """심사 시점의 **자체 재계산** 관측. body 의 해시는 참고일 뿐 이것이 사실이다."""
    return file_state(path)


def match_baseline(request_files, created_at, lease, baseline_rec, role_from_req=None):
    """[v2.1 ①] 요청 ↔ in-flight 사이클 baseline 매핑(순수).

    매핑 성립 조건(전건):
      ① baseline 레코드 존재
      ② lease 가 존재하고 phase 가 `fired`(handshake 는 fired 구간에서만 발생한다)
      ③ lease.cycle_id == baseline.cycle_id
      ④ baseline.file_set == 요청 body 의 파일 집합 (정확 일치 — 부분집합 불허)
      ⑤ baseline.started_at <= 요청 created_at (요청이 baseline 이후에 생겼다)
      ⑥ role 일치(요청 title 의 role == baseline.role) — 확인 가능할 때만
    하나라도 어긋나면 (False, 사유). 이 함수가 통과시키지 않은 요청은 절대 allow 되지 않는다.
    """
    if not baseline_rec:
        return False, "baseline 레코드 부재 — 전자동 사이클이 발행한 요청이 아니다"
    if not isinstance(lease, dict) or not lease:
        return False, "in-flight 사이클 없음(state.json lease 부재)"
    if lease.get("phase") != "fired":
        return False, "lease phase=%s (fired 아님 — handshake 구간이 아니다)" % lease.get("phase")
    if lease.get("cycle_id") != baseline_rec.get("cycle_id"):
        return False, "cycle_id 불일치(lease=%s baseline=%s)" % (
            lease.get("cycle_id"), baseline_rec.get("cycle_id"))
    want = sorted(set(baseline_rec.get("file_set") or []))
    got = sorted(set(request_files or []))
    if want != got:
        return False, "파일 집합 불일치(baseline %d건 vs 요청 %d건)" % (len(want), len(got))
    started = baseline_rec.get("started_at")
    if not isinstance(started, (int, float)) or float(created_at) < started:
        return False, "시간 역전(baseline started_at=%s > request created_at=%s)" % (
            started, created_at)
    if role_from_req and baseline_rec.get("role") and role_from_req != baseline_rec["role"]:
        return False, "role 불일치(요청=%s baseline=%s)" % (role_from_req, baseline_rec["role"])
    return True, "cycle-%s 매핑" % baseline_rec.get("cycle_id")


def adjudicate_body(entries, unparsed, created_at, statfn=default_statfn,
                    lease=None, baseline_rec=None, role_from_req=None):
    """[v2.1 ①] 3분기 판정(순수·부수효과 0) — **baseline 대비 전 파일 재기록**을 요구한다.

    종전(v2)은 body 해시와 디스크 해시의 일치 + mtime 창만 봤다. 그것으로는 "저장 전 대비
    바뀌었다"를 증명할 수 없다(codex R1 BLOCK). v2.1 계약:
      모든 파일에 대해  [현재 자체 재계산 sha256 ≠ baseline sha256]  AND  [현재 mtime > started_at]
    body 의 해시는 대조 기록으로만 남긴다(body_matches) — 판정 입력이 아니다.
    미생성이었던 파일은 baseline sha256=None 이므로 '생성' 자체가 해시 변화로 잡힌다.
    """
    files, ambiguous, stale = [], [], []
    if unparsed:
        ambiguous.append("본문 파싱 불능 %d줄: %s" % (len(unparsed), unparsed[0]))
    if not entries:
        ambiguous.append("본문 파일 0건 — 전량 대조 불가")

    req_paths = [p for p, _h in entries]
    mapped, map_reason = match_baseline(req_paths, created_at, lease, baseline_rec, role_from_req)
    started = (baseline_rec or {}).get("started_at")
    if not mapped:
        ambiguous.append(map_reason)

    base_files = (baseline_rec or {}).get("files") or {}
    for path, body_hash in entries:
        st = statfn(path) or {}
        base = base_files.get(path) or {}
        row = {"path": path, "body_hash": body_hash, "exists": bool(st.get("exists")),
               "mtime": st.get("mtime"), "sha256": st.get("sha256"),
               "baseline_sha256": base.get("sha256"), "baseline_mtime": base.get("mtime"),
               "body_matches": (body_hash is not None and st.get("sha256") == body_hash),
               "state": ""}
        if not mapped:
            row["state"] = "unmapped"
        elif not st.get("exists") or st.get("sha256") is None:
            row["state"] = "missing-or-unreadable"
            ambiguous.append("등록 파일 소실·판독 불가: %s" % path)
        elif st["sha256"] == base.get("sha256"):
            row["state"] = "unchanged"
            stale.append(path)
        elif not isinstance(st.get("mtime"), (int, float)) or st["mtime"] <= started:
            row["state"] = "mtime-not-after-start"
            stale.append(path)
        else:
            row["state"] = "rewritten"
        files.append(row)

    if ambiguous:
        verdict, reason = V_DENY_AMBIGUOUS, "; ".join(ambiguous[:3])
    elif stale:
        verdict, reason = V_DENY_STALE, "baseline 대비 미재기록 %d건: %s" % (
            len(stale), ", ".join(os.path.basename(p) for p in stale[:3]))
    else:
        verdict, reason = V_ALLOW, "전 파일(%d) baseline 대비 재기록 확인 (%s)" % (
            len(files), map_reason)
    return {"verdict": verdict, "reason": reason, "mapped": mapped,
            "map_reason": map_reason, "started_at": started,
            "evidence": len([f for f in files if f["state"] == "rewritten"]),
            "files": files}


def recheck_ready(row):
    """allow 발신 직전 대상 유휴 재확인(순수). 고속 allow 의 busy-clear 재도입 봉합."""
    if not row:
        return False, "대상 surface 부재"
    if row.get("exited") is True:
        return False, "대상 exited"
    qd = row.get("queue_depth")
    if qd != 0:
        return False, "queue_depth=%s" % qd
    st = row.get("status")
    if isinstance(st, dict) and st.get("state") == "working":
        return False, "자기보고 working"
    idle = row.get("idle_secs")
    if not isinstance(idle, (int, float)) or idle < RECHECK_IDLE_MIN:
        return False, "idle_secs=%s (<%.0f)" % (idle, RECHECK_IDLE_MIN)
    return True, "idle=%s queue=0" % idle


# ── feed 조회 (실측 CLI·JSONL 계약) ───────────────────────────────────
def parse_feed_list(text):
    """`cys feed list` 텍스트 파싱 — 실측 형식: id\\t[status]\\tkind\\ttitle\\tdecision=x.

    ※ cys feed 에는 --json 도 show 서브커맨드도 **없다**(실측). body 는 이 출력에 실리지 않아
       feed.jsonl 에서 회수한다(IMPL_NOTES_A §2).
    """
    items = []
    for line in (text or "").splitlines():
        line = line.rstrip("\n")
        if not line.strip() or line.strip() == "(feed empty)":
            continue
        parts = line.split("\t")
        if len(parts) < 4:
            continue
        status = parts[1].strip()
        if status.startswith("[") and status.endswith("]"):
            status = status[1:-1]
        items.append({"request_id": parts[0].strip(), "status": status,
                      "kind": parts[2].strip(), "title": parts[3].strip(),
                      "decision": (parts[4].split("=", 1)[1].strip()
                                   if len(parts) > 4 and "=" in parts[4] else None)})
    return items


def list_feed(status=None, runner=run):
    cmd = [CYS, "feed", "list"]
    if status:
        cmd += ["--status", status]
    rc, out, err = runner(cmd)
    if rc != 0:
        return None, err.strip()[:200]
    return parse_feed_list(out), ""


def read_feed_records(path=None):
    """feed.jsonl → {request_id: 마지막 레코드}. 데몬은 push·resolve 마다 append 한다(last-wins)."""
    path = path or feed_jsonl_path()
    out = {}
    try:
        with open(path, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    r = json.loads(line)
                except ValueError:
                    continue
                rid = r.get("request_id") if isinstance(r, dict) else None
                if rid:
                    out[rid] = r
    except OSError:
        return {}
    return out


def lookup_request(request_id, retries=FEED_LOOKUP_RETRY, sleep=0.5):
    """feed.jsonl 반영 지연(push→append)을 흡수하며 레코드 회수."""
    for i in range(max(1, retries)):
        rec = read_feed_records().get(request_id)
        if rec is not None:
            return rec
        if i + 1 < retries:
            time.sleep(sleep)
    return None


# ── 회신·자기검증 ─────────────────────────────────────────────────────
def send_reply(request_id, decision, reason, runner=run):
    """cys feed reply <id> <allow|deny> --reason <...>.

    ★승인 토큰류를 여기에 첨부하지 않는다 — 자기승인 차단은 cysd 가 발행자 pid/pgid/surface 로
      판정한다. 이 워처는 pane 자식이므로 정상 귀속되어 그대로 수리된다.
    """
    if no_send():
        return False, "no_send"
    cmd = [CYS, "feed", "reply", request_id, decision]
    if reason:
        cmd += ["--reason", reason[:300]]
    rc, _o, e = runner(cmd)
    return rc == 0, ("" if rc == 0 else "rc=%d %s" % (rc, e.strip()[:200]))


def self_verify_resolved(request_id, decision, runner=run, retries=SELF_VERIFY_RETRY, sleep=0.6):
    """reply 후 resolved 재조회 — 미착지 = hard fail."""
    for i in range(max(1, retries)):
        items, _err = list_feed(None, runner=runner)
        for it in items or []:
            if it["request_id"] == request_id:
                if it["status"] == "resolved" and (it.get("decision") or "") == decision:
                    return True, "resolved"
                if it["status"] == "resolved":
                    return False, "resolved 이지만 decision=%s (기대 %s)" % (it.get("decision"), decision)
        if i + 1 < retries:
            time.sleep(sleep)
    return False, "재조회 미착지(pending 잔류 또는 조회 실패)"


# ── 단발 심사 파이프라인 ──────────────────────────────────────────────
def adjudicate_request(request_id, apply_reply, runner=run, wait_idle=True):
    """1건 심사 전 과정. 반환 dict(원장 기록·표준출력용)."""
    started = time.time()
    rec = lookup_request(request_id)
    if rec is None:
        res = {"request_id": request_id, "verdict": V_DENY_AMBIGUOUS,
               "reason": "feed 레코드 회수 불가(%s)" % feed_jsonl_path(), "files": []}
        _finish(res, apply_reply, runner, escalate_it=True)
        return res
    if rec.get("kind") != FEED_KIND:
        return {"request_id": request_id, "verdict": "skip",
                "reason": "kind=%s (대상 아님)" % rec.get("kind")}

    role = role_from_title(rec.get("title"))
    entries, unparsed = parse_body(rec.get("body"))
    # [v2.1 ①] in-flight lease + 사이클 baseline 을 결합해 심사한다.
    lease = (read_json_file(STATE_JSON) or {}).get("lease")
    bl = None
    if isinstance(lease, dict) and lease.get("cycle_id") is not None:
        bl = read_json_file(lease.get("baseline_path")
                            or baseline_path(lease["cycle_id"]))
    res = adjudicate_body(entries, unparsed, rec.get("created_at") or started,
                          lease=lease, baseline_rec=bl, role_from_req=role)
    res.update({"request_id": request_id, "role": role,
                "surface_id": rec.get("surface_id"),
                "created_at": rec.get("created_at"), "title": rec.get("title"),
                "cycle_id": (lease or {}).get("cycle_id")})

    if res["verdict"] == V_ALLOW:
        ok, why = _allow_recheck(rec.get("surface_id"), runner=runner, wait_idle=wait_idle)
        res["recheck"] = why
        if not ok:
            res["verdict"] = V_DENY_STALE if "유휴" in why or "idle" in why else V_DENY_AMBIGUOUS
            res["reason"] = "allow 직전 재확인 실패: %s" % why
    _finish(res, apply_reply, runner,
            escalate_it=(res["verdict"] == V_DENY_AMBIGUOUS))
    return res


def _allow_recheck(surface_id, runner=run, wait_idle=True):
    """kill-switch + 대상 유휴 재확인. 유휴 미달이면 ALLOW_RECHECK_MAX 까지 대기."""
    killed, why = kill_switch(runner=runner)
    if killed:
        return False, "kill-switch: %s" % why
    if surface_id is None:
        return False, "대상 surface_id 부재"
    deadline = time.time() + (ALLOW_RECHECK_MAX if wait_idle else 0)
    last = "미측정"
    while True:
        status = fetch_status(runner=runner)
        row = None
        for s in (status or {}).get("surfaces") or []:
            if isinstance(s, dict) and s.get("surface_id") == surface_id:
                row = s
                break
        ok, last = recheck_ready(row)
        if ok:
            killed2, why2 = kill_switch(runner=runner)
            if killed2:
                return False, "kill-switch(재확인): %s" % why2
            return True, last
        if time.time() >= deadline:
            return False, "대상 유휴 미달(%s) — %.0fs 대기 후 포기" % (last, ALLOW_RECHECK_MAX)
        time.sleep(POLL_SECS)


def _finish(res, apply_reply, runner, escalate_it=False):
    decision = "allow" if res["verdict"] == V_ALLOW else "deny"
    res["decision"] = decision
    if apply_reply:
        ok, why = send_reply(res["request_id"], decision, res.get("reason", ""), runner=runner)
        res["replied"] = ok
        res["reply_err"] = why
        if ok:
            sv_ok, sv_why = self_verify_resolved(res["request_id"], decision, runner=runner)
            res["self_verify"] = sv_ok
            res["self_verify_detail"] = sv_why
            if not sv_ok:
                escalate_it = True
    else:
        res["replied"] = False
        res["reply_err"] = "dry-run(--apply 미지정)"
    log_append({"ts": time.time(), "cycle_id": None, "phase": "verify",
                "role": res.get("role"),
                "surface": (None if res.get("surface_id") is None
                            else "surface:%s" % res["surface_id"]),
                "detail": {k: res.get(k) for k in
                           ("request_id", "cycle_id", "verdict", "decision", "reason",
                            "mapped", "map_reason", "evidence", "recheck", "replied",
                            "reply_err", "self_verify", "self_verify_detail", "files")}})
    if escalate_it:
        escalate("[CYCLE-VERIFY] %s 판정 %s — %s"
                 % (res["request_id"], res["verdict"], (res.get("reason") or "")[:160]),
                 runner=runner)


# ── watch ─────────────────────────────────────────────────────────────
_STOP = threading.Event()


def _stdin_sink():
    """주입 텍스트를 읽어 **버린다**. 절대 실행·평가하지 않는다(고무도장 함정 봉인)."""
    try:
        for _line in sys.stdin:
            if _STOP.is_set():
                return
    except Exception:  # noqa: BLE001
        return


def _touch_heartbeat():
    os.makedirs(STATE_DIR, exist_ok=True)
    with open(HEARTBEAT, "w", encoding="utf-8") as f:
        f.write("%d\n" % int(time.time()))


def cmd_watch(args):
    def _sig(_s, _f):
        _STOP.set()
    signal.signal(signal.SIGTERM, _sig)
    signal.signal(signal.SIGINT, _sig)
    threading.Thread(target=_stdin_sink, daemon=True).start()

    _touch_heartbeat()
    last_hb = time.time()
    handled = set()
    print("[cycle-verifier] watch 시작 — heartbeat=%s poll=%.1fs" % (HEARTBEAT, POLL_SECS),
          flush=True)
    log_append({"ts": time.time(), "cycle_id": None, "phase": "verify", "role": VERIFIER_ROLE,
                "surface": os.environ.get("AITERM_SURFACE_ID") or os.environ.get("CYS_SURFACE_ID"),
                "detail": {"event": "watch_start", "pid": os.getpid()}})
    while not _STOP.is_set():
        now = time.time()
        if now - last_hb >= HEARTBEAT_TOUCH_SECS:
            _touch_heartbeat()
            last_hb = now
        items, err = list_feed("pending")
        if items is None:
            print("[cycle-verifier] feed list 실패: %s" % err, flush=True)
        else:
            for it in items:
                if it["kind"] != FEED_KIND or it["request_id"] in handled:
                    continue
                handled.add(it["request_id"])
                print("[cycle-verifier] 심사 %s %s" % (it["request_id"], it["title"]), flush=True)
                res = adjudicate_request(it["request_id"], apply_reply=True)
                print("[cycle-verifier]  → %s (%s)" % (res.get("decision"), res.get("reason")),
                      flush=True)
        _STOP.wait(POLL_SECS)
    _touch_heartbeat()
    log_append({"ts": time.time(), "cycle_id": None, "phase": "verify", "role": VERIFIER_ROLE,
                "surface": None, "detail": {"event": "watch_stop"}})
    print("[cycle-verifier] watch 종료", flush=True)
    return EXIT_OK


def cmd_adjudicate(args):
    res = adjudicate_request(args.request_id, apply_reply=bool(args.apply),
                             wait_idle=not args.no_wait)
    print(json.dumps(res, ensure_ascii=False, indent=2, default=str))
    if res["verdict"] == V_ALLOW:
        return EXIT_OK
    return EXIT_GATE


# ── self-test ─────────────────────────────────────────────────────────
_MARK = "CONTRACT" + " BLOCK" + " v1 "


def extract_contract_block(path):
    """두 스크립트의 공유 계약 블록 텍스트를 추출(이음매 드리프트 검사용)."""
    start, end = _MARK + "START", _MARK + "END"
    lines, on, buf = open(path, "r", encoding="utf-8").read().splitlines(), False, []
    for ln in lines:
        if not on and start in ln:
            on = True
            continue
        if on and end in ln:
            return "\n".join(buf)
        if on:
            buf.append(ln)
    return None


class _T(object):
    def __init__(self):
        self.ok = 0
        self.fail = []

    def check(self, name, cond, extra=""):
        if cond:
            self.ok += 1
            print("  PASS  %s" % name)
        else:
            self.fail.append(name)
            print("  FAIL  %s %s" % (name, extra))


def cmd_self_test(args):
    import tempfile
    t = _T()
    print("== javis_cycle_verifier.py self-test ==")
    tmpd = tempfile.mkdtemp(prefix="cycverftest-")

    # 1) 본문 파싱 (cys.rs handshake_file_line 실측 형식)
    print("[1] handshake 본문 파싱")
    h = "a" * 64
    body = ("/Users/x/_round/SESSION_STATE.md (sha256: %s)\n"
            "/Users/x/.cys/pack/round/MASTER_TODO.md (미생성 — 생성 자체가 증거)\n" % h)
    ents, unp = parse_body(body)
    t.check("2줄 파싱", len(ents) == 2 and not unp, str(ents))
    t.check("해시줄 → (path, hash)", ents[0] == ("/Users/x/_round/SESSION_STATE.md", h))
    t.check("미생성줄 → (path, None)",
            ents[1] == ("/Users/x/.cys/pack/round/MASTER_TODO.md", None))
    ents2, unp2 = parse_body("garbage line without form\n")
    t.check("형식 위반 → unparsed", not ents2 and len(unp2) == 1)
    ents3, unp3 = parse_body("")
    t.check("빈 본문 → 0건", not ents3 and not unp3)
    t.check("공백줄 무시", parse_body("\n\n%s\n" % ("/p (sha256: %s)" % h))[0] ==
            [("/p", h)])
    t.check("title 에서 role 추출",
            role_from_title("[CYCLE-VERIFY] reviewer-codex 저장 검증 요청") == "reviewer-codex")
    t.check("title 형식 위반 → None", role_from_title("무관한 제목") is None)

    # 2) [v2.1 ①] baseline 매핑 + 3분기 판정
    print("[2] baseline 매핑·adjudicate 3분기 판정표")
    started = 10000.0
    created = started + 30.0
    old_h, new_h = "a" * 64, "d" * 64
    LEASE = {"cycle_id": 77, "role": "worker", "phase": "fired"}
    BASE = {"cycle_id": 77, "role": "worker", "started_at": started,
            "file_set": ["/p"], "files": {"/p": {"exists": True, "sha256": old_h,
                                                 "mtime": started - 100}}}

    def stat(exists=True, mtime=None, sha=None):
        return lambda p: {"exists": exists, "sha256": sha, "mtime": mtime}

    # 매핑 술어 단독
    ok, why = match_baseline(["/p"], created, LEASE, BASE, "worker")
    t.check("정상 매핑 성립", ok, why)
    ok, why = match_baseline(["/p"], created, LEASE, None, "worker")
    t.check("baseline 부재 → 매핑 실패", not ok, why)
    ok, why = match_baseline(["/p"], created, None, BASE, "worker")
    t.check("lease 부재(in-flight 아님) → 매핑 실패", not ok, why)
    ok, why = match_baseline(["/p"], created, dict(LEASE, phase="armed"), BASE, "worker")
    t.check("phase!=fired → 매핑 실패", not ok, why)
    ok, why = match_baseline(["/p"], created, dict(LEASE, cycle_id=78), BASE, "worker")
    t.check("cycle_id 불일치 → 매핑 실패", not ok, why)
    ok, why = match_baseline(["/p", "/q"], created, LEASE, BASE, "worker")
    t.check("파일 집합 불일치(초과) → 매핑 실패", not ok, why)
    ok, why = match_baseline([], created, LEASE, BASE, "worker")
    t.check("파일 집합 불일치(부족) → 매핑 실패", not ok, why)
    ok, why = match_baseline(["/p"], started - 10, LEASE, BASE, "worker")
    t.check("시간 역전(요청이 baseline 이전) → 매핑 실패", not ok, why)
    ok, why = match_baseline(["/p"], created, LEASE, BASE, "master")
    t.check("role 불일치 → 매핑 실패", not ok, why)

    # 3분기
    r = adjudicate_body([("/p", new_h)], [], created, stat(True, started + 10, new_h),
                        lease=LEASE, baseline_rec=BASE, role_from_req="worker")
    t.check("baseline 대비 해시 변화 + mtime 이후 → allow", r["verdict"] == V_ALLOW, str(r))
    r = adjudicate_body([("/p", old_h)], [], created, stat(True, started + 10, old_h),
                        lease=LEASE, baseline_rec=BASE, role_from_req="worker")
    t.check("해시가 baseline 과 동일(미재기록) → deny_stale",
            r["verdict"] == V_DENY_STALE, str(r))
    r = adjudicate_body([("/p", new_h)], [], created, stat(True, started - 5, new_h),
                        lease=LEASE, baseline_rec=BASE, role_from_req="worker")
    t.check("해시는 변했는데 mtime 이 개시 이전 → deny_stale", r["verdict"] == V_DENY_STALE)
    r = adjudicate_body([("/p", new_h)], [], created, stat(False, None, None),
                        lease=LEASE, baseline_rec=BASE, role_from_req="worker")
    t.check("등록 파일 소실 → deny_ambiguous", r["verdict"] == V_DENY_AMBIGUOUS)
    r = adjudicate_body([("/p", new_h)], [], created, stat(True, started + 10, None),
                        lease=LEASE, baseline_rec=BASE, role_from_req="worker")
    t.check("판독 불가 → deny_ambiguous", r["verdict"] == V_DENY_AMBIGUOUS)
    r = adjudicate_body([], [], created, stat(), lease=LEASE, baseline_rec=BASE)
    t.check("본문 0건 → deny_ambiguous", r["verdict"] == V_DENY_AMBIGUOUS)
    r = adjudicate_body([("/p", new_h)], ["쓰레기줄"], created, stat(True, started + 10, new_h),
                        lease=LEASE, baseline_rec=BASE, role_from_req="worker")
    t.check("파싱 불능 줄 존재 → deny_ambiguous(증거 있어도)", r["verdict"] == V_DENY_AMBIGUOUS)
    r = adjudicate_body([("/p", new_h)], [], created, stat(True, started + 10, new_h),
                        lease=LEASE, baseline_rec=None, role_from_req="worker")
    t.check("baseline 부재 → deny_ambiguous(증거 무관)", r["verdict"] == V_DENY_AMBIGUOUS)

    # body 해시는 판정 입력이 아니다
    r = adjudicate_body([("/p", "f" * 64)], [], created, stat(True, started + 10, new_h),
                        lease=LEASE, baseline_rec=BASE, role_from_req="worker")
    t.check("body 해시가 디스크와 달라도 baseline 대비 재기록이면 allow",
            r["verdict"] == V_ALLOW and r["files"][0]["body_matches"] is False, str(r))

    # 미생성 → 생성
    BASE_ABS = {"cycle_id": 77, "role": "worker", "started_at": started,
                "file_set": ["/n"], "files": {"/n": {"exists": False, "sha256": None,
                                                     "mtime": None}}}
    r = adjudicate_body([("/n", None)], [], created, stat(True, started + 10, new_h),
                        lease=LEASE, baseline_rec=BASE_ABS, role_from_req="worker")
    t.check("baseline 미생성 → 생성됨 = 해시 변화 → allow", r["verdict"] == V_ALLOW)
    r = adjudicate_body([("/n", None)], [], created, stat(False, None, None),
                        lease=LEASE, baseline_rec=BASE_ABS, role_from_req="worker")
    t.check("baseline 미생성 → 여전히 부재 → deny_ambiguous", r["verdict"] == V_DENY_AMBIGUOUS)

    # ALL-match (v2.1: ANY 아님)
    BASE2 = {"cycle_id": 77, "role": "master", "started_at": started,
             "file_set": ["/a", "/b"],
             "files": {"/a": {"exists": True, "sha256": old_h, "mtime": started - 1},
                       "/b": {"exists": True, "sha256": "c" * 64, "mtime": started - 1}}}

    def multi(p):
        return {"/a": {"exists": True, "sha256": new_h, "mtime": started + 5},
                "/b": {"exists": True, "sha256": "c" * 64, "mtime": started + 5}}[p]
    r = adjudicate_body([("/a", new_h), ("/b", "c" * 64)], [], created, multi,
                        lease=dict(LEASE, role="master"), baseline_rec=BASE2,
                        role_from_req="master")
    t.check("한 건만 재기록 → deny_stale (ALL-match 계약)",
            r["verdict"] == V_DENY_STALE and r["evidence"] == 1, str(r))

    # 3) 실파일 통합 심사
    print("[3] 실파일 통합 심사(baseline 결합)")
    fp = os.path.join(tmpd, "SESSION_STATE.md")
    open(fp, "w", encoding="utf-8").write("before")
    base_state = file_state(fp)
    now_ts = time.time()
    RB = {"cycle_id": 77, "role": "worker", "started_at": now_ts - 1,
          "file_set": [fp], "files": {fp: base_state}}
    r = adjudicate_body([(fp, base_state["sha256"])], [], now_ts, default_statfn,
                        lease=LEASE, baseline_rec=RB, role_from_req="worker")
    t.check("저장 전 상태 그대로 → deny_stale", r["verdict"] == V_DENY_STALE, str(r))
    open(fp, "w", encoding="utf-8").write("after — 물리 재기록")
    os.utime(fp, (now_ts + 5, now_ts + 5))
    after = file_state(fp)
    r = adjudicate_body([(fp, after["sha256"])], [], now_ts, default_statfn,
                        lease=LEASE, baseline_rec=RB, role_from_req="worker")
    t.check("물리 재기록 후 → allow", r["verdict"] == V_ALLOW, str(r))
    t.check("자체 재계산 해시가 기록됨", r["files"][0]["sha256"] == after["sha256"])

    # 4) allow 직전 재확인 술어
    # [R2-C] 라벨을 S1 실측 튜닝 상수(RECHECK_IDLE_MIN=5s · ALLOW_RECHECK_MAX=108s)로 정정.
    #   종전 라벨은 "idle 3(<10)" 이라 상수 변경 후에도 옛 계약을 설명하고 있었다.
    print("[4] allow 직전 대상 유휴 재확인 (하한 %.0fs · 대기 상한 %.0fs)"
          % (RECHECK_IDLE_MIN, ALLOW_RECHECK_MAX))
    ok, why = recheck_ready({"idle_secs": 30, "queue_depth": 0, "status": None})
    t.check("idle 30 queue 0 → ready", ok, why)
    ok, why = recheck_ready({"idle_secs": 6, "queue_depth": 0})
    t.check("idle 6(>=%.0f) → ready (S1 성공 사이클 실측치)" % RECHECK_IDLE_MIN, ok, why)
    ok, why = recheck_ready({"idle_secs": 3, "queue_depth": 0})
    t.check("idle 3(<%.0f) → 대기" % RECHECK_IDLE_MIN, not ok, why)
    t.check("상수 계약 핀 — RECHECK_IDLE_MIN=5 · ALLOW_RECHECK_MAX=108",
            RECHECK_IDLE_MIN == 5.0 and ALLOW_RECHECK_MAX == 108.0)
    ok, why = recheck_ready({"idle_secs": 30, "queue_depth": 2})
    t.check("queue_depth>0 → 대기", not ok, why)
    ok, why = recheck_ready({"idle_secs": 30, "queue_depth": 0,
                             "status": {"state": "working"}})
    t.check("자기보고 working → 대기", not ok, why)
    ok, why = recheck_ready({"idle_secs": 30, "queue_depth": 0, "exited": True})
    t.check("exited → 거부", not ok, why)
    ok, why = recheck_ready(None)
    t.check("row 부재 → 거부", not ok, why)

    # 5) feed 계약 파싱
    print("[5] cys feed list / feed.jsonl 계약")
    txt = ("req-1\t[pending]\tcycle-verify\t[CYCLE-VERIFY] worker 저장 검증 요청\tdecision=-\n"
           "req-2\t[resolved]\tpermission\t뭔가\tdecision=allow\n"
           "(feed empty)\n")
    items = parse_feed_list(txt)
    t.check("2건 파싱", len(items) == 2, str(items))
    t.check("status 대괄호 제거", items[0]["status"] == "pending")
    t.check("kind·title 보존",
            items[0]["kind"] == "cycle-verify"
            and items[0]["title"].startswith("[CYCLE-VERIFY] worker"))
    t.check("decision 파싱", items[1]["decision"] == "allow")
    t.check("(feed empty) 무시", all(i["request_id"] != "(feed empty)" for i in items))
    fj = os.path.join(tmpd, "feed.jsonl")
    with open(fj, "w", encoding="utf-8") as f:
        f.write(json.dumps({"request_id": "r1", "status": "pending", "body": "A"}) + "\n")
        f.write("깨진 줄\n")
        f.write(json.dumps({"request_id": "r1", "status": "resolved", "body": "B"}) + "\n")
    recs = read_feed_records(fj)
    t.check("feed.jsonl last-wins", recs["r1"]["status"] == "resolved" and recs["r1"]["body"] == "B")
    t.check("손상 줄 무시(부분 회수)", len(recs) == 1)

    # 6) 동시성 append (계약 공유)
    print("[6] 원장 동시 append")
    logp = os.path.join(tmpd, "cycle_autopilot_log.jsonl")
    # ★Windows(R3): os.fork 는 POSIX 전용 → 이 케이스만 [SKIP](PASS 아님·나머지 배터리와
    #   패리티 [7] 은 전부 실행). 동시성 계약 자체는 POSIX drill(mac CI)이 확증한다 —
    #   javis_state_snapshot T2 선례 동형(블랜킷 skip 금지). hasattr 겹은 fork **부재**가
    #   판정 실체이기 때문(winsim 재현 포함 — nt 에서는 hasattr 이 항상 False 라 동치).
    if os.name == "nt" or not hasattr(os, "fork"):
        print("  [SKIP] 동시 append 는 os.fork(POSIX) 필요 — Windows 미지원(mac CI 가 확증). "
              "나머지 케이스는 실행.")
    else:
        kids, NPROC, NLINE = [], 6, 50
        for i in range(NPROC):
            pid = os.fork()
            if pid == 0:
                try:
                    for j in range(NLINE):
                        log_append({"ts": time.time(), "cycle_id": None, "phase": "verify",
                                    "role": "v%d" % i, "surface": None,
                                    "detail": {"pad": "q" * 150, "j": j}}, path=logp)
                finally:
                    os._exit(0)
            kids.append(pid)
        for pid in kids:
            os.waitpid(pid, 0)
        lines = [l for l in open(logp, encoding="utf-8").read().splitlines() if l.strip()]
        good = 0
        for l in lines:
            try:
                r = json.loads(l)
                if set(r.keys()) == set(LOG_KEYS):
                    good += 1
            except ValueError:
                pass
        t.check("라인수 == %d" % (NPROC * NLINE), len(lines) == NPROC * NLINE,
                "실제 %d" % len(lines))
        t.check("전 라인 무결(JSON+키)", good == len(lines), "%d/%d" % (good, len(lines)))

    # 7) 계약 블록 동일성
    print("[7] 두 스크립트 계약 블록 동일성")
    mine = extract_contract_block(os.path.abspath(__file__))
    sib = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_cycle_autopilot.py")
    t.check("자기 계약 블록 추출", bool(mine))
    if os.path.exists(sib):
        other = extract_contract_block(sib)
        t.check("javis_cycle_autopilot.py 와 바이트 동일", mine == other,
                "" if mine == other else "블록 불일치 — 한쪽만 수정됨")
    else:
        t.check("sibling 부재(SKIP 처리)", True, "(autopilot 미배치)")

    # 8) 금지 계약 — 승인 토큰 코드·detach·force-no-verify 부재
    # ★needle 은 전부 런타임 조립이다. 리터럴로 쓰면 이 검사 코드 자신이 위반으로 잡힌다.
    print("[8] 금지 계약 검사")
    src = open(os.path.abspath(__file__), "r", encoding="utf-8").read()
    t.check("승인 토큰 식별자 0회", src.count("operator" + "_token") == 0
            and src.count("operator" + ".token") == 0)
    t.check("force-no-verify 플래그 0회", src.count("--force-no" + "-verify") == 0)
    t.check("detach 수단 0회(검증자 pane 포그라운드 강제)",
            src.count("start_new" + "_session") == 0 and src.count("os." + "setsid") == 0
            and src.count("preexec" + "_fn") == 0)
    t.check("clear 명령 직접 타이핑 0회", src.count("--clear" + "-cmd") == 0)

    # 9) [R1] 블록 밖 Windows 상태 dir override 정합 — mac 에서 실행 가능한 회귀 핀
    print("[9] R1 — Windows 상태 dir override(블록 밖·I3 선례) 정합")
    t.check("블록 원형 심볼 보존(_state_socket_dir_contract)",
            callable(_state_socket_dir_contract)
            and _state_socket_dir_contract is not state_socket_dir)
    saved_env = {k: os.environ.get(k) for k in ("AITERM_SOCKET", "CYS_SOCKET")}
    try:
        os.environ.pop("AITERM_SOCKET", None)
        os.environ["CYS_SOCKET"] = os.path.join(tmpd, "cys.sock")
        if os.name != "nt":
            t.check("POSIX 관통 — override == 블록 원형(소켓 env 주입)",
                    state_socket_dir() == _state_socket_dir_contract() == tmpd,
                    "%s vs %s" % (state_socket_dir(), tmpd))
            t.check("feed_jsonl_path 가 override 를 경유(소켓 env 주입)",
                    feed_jsonl_path() == os.path.join(tmpd, "feed.jsonl"))
            os.environ.pop("CYS_SOCKET", None)
            t.check("POSIX 관통 — env 부재 폴백도 블록 원형과 동일",
                    state_socket_dir() == _state_socket_dir_contract())
    finally:
        for k, v in saved_env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
    snap = _snapshot_mod()
    t.check("매핑 단일 소스(javis_state_snapshot) 로드", snap is not None)
    if snap is not None:
        lad = os.path.join(tmpd, "LocalAppData")
        pipe_default = "\\\\.\\pipe\\cys"
        pipe_dept = "\\\\.\\pipe\\cys-dept-x"
        t.check("기본 파이프 → LOCALAPPDATA/cys (주입식 매핑)",
                snap._win_state_dir_for_socket(pipe_default, localappdata=lad)
                == os.path.realpath(os.path.join(lad, "cys")))
        t.check("부서 파이프 → cys/<슬러그> 격리 dir (주입식 매핑)",
                snap._win_state_dir_for_socket(pipe_dept, localappdata=lad)
                == os.path.realpath(os.path.join(lad, "cys", "cys-dept-x")))
        t.check("슬러그 규칙 = state.rs pipe_slug 동형",
                snap._win_pipe_slug(pipe_default) == "cys"
                and snap._win_pipe_slug(pipe_dept) == "cys-dept-x")

    # [codex·gemini R1 수용] 최후 폴백(스냅샷 모듈 소실 시뮬) — 함수 분리(_fallback_state_dir)
    # +인자 주입으로 폴백 분기를 직접 검증한다(모듈 로드 성공 여부와 무관).
    lad_fb = os.path.join(tmpd, "LAD-fallback")
    t.check("최후 폴백: 기본 파이프 → base/cys (state.rs 기본 규칙 직조립 유지)",
            _fallback_state_dir("\\\\.\\pipe\\cys", localappdata=lad_fb)
            == os.path.join(lad_fb, "cys"))
    _fb_dept = _fallback_state_dir("\\\\.\\pipe\\cys-dept-x", localappdata=lad_fb)
    t.check("최후 폴백: 부서 파이프 → 센티널 경로 fail-closed (base/cys 접힘 금지)",
            ".dept-mapping-unavailable" in _fb_dept
            and _fb_dept != os.path.join(lad_fb, "cys"))

    print("\n결과: PASS %d / FAIL %d" % (t.ok, len(t.fail)))
    if t.fail:
        for n in t.fail:
            print("  - %s" % n)
        return EXIT_ERR
    return EXIT_OK


# ── main ──────────────────────────────────────────────────────────────
def main(argv=None):
    ap = argparse.ArgumentParser(description="결정론 사이클 검증자 (컴포넌트 2)")
    sub = ap.add_subparsers(dest="cmd")
    sub.add_parser("watch", help="포그라운드 워처 상주(pane 안에서만)")
    aj = sub.add_parser("adjudicate", help="단발 심사(기본 dry-run)")
    aj.add_argument("--request-id", required=True)
    aj.add_argument("--apply", action="store_true", help="실제 cys feed reply 발신")
    aj.add_argument("--no-wait", action="store_true", help="유휴 대기 없이 즉시 판정")
    sub.add_parser("self-test", help="데몬 없이 픽스처 검증")
    args = ap.parse_args(argv)
    table = {"watch": cmd_watch, "adjudicate": cmd_adjudicate, "self-test": cmd_self_test}
    if args.cmd not in table:
        ap.print_help()
        return EXIT_ERR
    try:
        return table[args.cmd](args)
    except KeyboardInterrupt:
        return EXIT_OK
    except Exception as e:  # noqa: BLE001 — 최상위는 항상 코드로 답한다
        print("error: %s" % e, file=sys.stderr)
        return EXIT_ERR


if __name__ == "__main__":
    sys.exit(main())
