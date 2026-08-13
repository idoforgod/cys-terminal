#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_cycle_autopilot.py — 컨텍스트 사이클 개시·집행·사후검증 결정론 상태기계 (컴포넌트 1).

설계 정본: 프로젝트 _round/fullauto-cycle/DESIGN_full-auto-cycle.md · 운영 가이드: docs/GUIDE-fullauto-cycle-KR.md (v2 §2 컴포넌트 1).
선례 스타일: ~/.cys/pack/bin/javis_reap_exited.py (결정론 집행), javis_wakeup.py (멱등 큐).

이 스크립트가 하는 일 (happy path LLM 판단 0회):
  tick      — 판단만·짧게. 게이트 6종 전부 통과하면 집행자를 setsid detach 로 스폰하고 즉시 종료.
              in-flight 사이클이 있으면 감독(죽은 집행자 인계 포함), phase=cleared 면 사후검증.
  execute   — 집행자(detach 자식). 선통보 → cys cycle-agent 를 **자식으로** 실행하며 3초 간격
              kill-switch 폴링(감지 즉시 자식 SIGTERM) → settle → 사후검증 → 종결.
  audit     — S0 shadow 오탐 oracle 집계(설계 §5). would_fire ↔ 직후 관측 레코드 대조.
  reset     — lockout 해제(운영자 수동·사유 원장 기록).
  bootstrap-verifier — 검증자 전용 pane 신설 + 워처 기동(게이트6의 전제 생성).
  status    — 부수효과 0. 현재 모드·lease·게이트 판정을 JSON 으로.
  self-test — 데몬 없이 픽스처로 핵심 로직 검증(게이트 판정표·phase 전이·동시성 append).

★안전 불변식 (코드 강제):
  - 자동 clear 의 유일 경로는 `cys cycle-agent --verifier` 다. 이 스크립트는 clear 를 직접 타이핑하지 않는다.
  - **`--force-no-verify` 는 어떤 경로로도 argv 에 실리지 않는다** (아래 build_cycle_agent_argv 참조·self-test 가 박제).
  - kill-switch 는 4중: ①데몬 pause(스케줄 동결) ②cys gate-check ③PAUSED 파일 절대경로 2중
    ④in-flight 3초 폴링 + 자식 SIGTERM.
  - 게이트 실패·측정 모호·원장 미완결 = 무집행(fail-closed). 오탐보다 미집행이 안전측.
  - 기본 모드는 shadow: would_fire 를 원장에 기록만 하고 아무것도 발화하지 않는다.

exit: 0=정상 / 1=오류 / 2=게이트 미통과(정상 skip) / 4=kill-switch / 5=원장 미완결 fail-closed
※ cys schedule 의 action:"command" 는 exit!=0 을 schedule.error 로 승격한다. 잡 등록 시
   command 문자열 끝에 `; exit 0` 을 붙여 정상 skip(2)이 매분 에러로 보이지 않게 한다(IMPL_NOTES_A §5).
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
#   ★이 형태를 고른 결정적 이유: 사용처 4곳 중 2곳이 아래 CONTRACT BLOCK v1 **안**에 있고
#     그 블록은 javis_cycle_verifier.py 와 **바이트 동일**해야 한다(self-test contract-parity).
#     사용처를 건드리는 어떤 수리도 즉시 parity 를 깨뜨린다 — import 층에서만 접는
#     이 shim 만이 두 계약을 동시에 만족한다(javis_cycle_verifier.py 와 동일 형태).
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


# ── ★T-0147-2 층1 I3 — escalation 발행 경로를 javis_wakeup 큐로 수렴 ─────────────────
#
# 설계 정본: `_round/T-0147-2-DESIGN-wakeup-demotion.md` §2 층1(I3) · §8(R2-C2 수용).
# 종전 `escalate` 는 `cys send` + `send-key Return` 으로 master stdin 을 **직접** 두드리는
# 두 번째 발행자였다. 그 경로는 큐의 코얼레싱·멱등·zombie 가드·digest 를 전부 우회하므로
# 게이트 push 와 무관하게 홍수를 재생산한다. 설계의 불변식은 "trigger-namespace별 단일
# 발행자" — pack 측 wakeup 은 **레인당 javis_wakeup 큐 하나**로만 나간다. 여기서 수렴시킨다.
#
# ⚠이 재정의가 CONTRACT BLOCK v1 **밖**에 있는 것은 의도다(회피가 아니라 범위 준수):
#   그 블록은 `javis_cycle_verifier.py` 와 **바이트 동일**해야 하고 양쪽 self-test 의
#   contract-parity 검사가 그것을 박제한다. 설계 층1 표가 지목한 주입자는 autopilot 의 I3
#   하나뿐이라, 블록 안을 고치면 범위 밖 파일(verifier)까지 같은 변경에 끌려들어간다.
#   그래서 블록은 원형 그대로 두고 **모듈 수준에서 심볼만 덮어쓴다** — autopilot 안의 모든
#   escalate 호출부(:879·:1244·:1413)는 아래 정의를 본다. verifier 도 같은 수렴을 받게 될
#   때는 블록 자체를 옮기는 것이 옳고, 그때 두 파일을 함께 갱신하면 parity 가 유지된다.
#   (`push_line` 은 건드리지 않는다 — 선통보 주입 :1169 가 그대로 쓴다.)

WAKEUP_SCRIPT_BASENAME = "javis_wakeup.py"
ESCALATION_TASK_DEFAULT = "autopilot-escalation"


def wakeup_script():
    """javis_wakeup.py 경로 — 형제 파일 우선(레포·배포 팩 양쪽에서 성립), 없으면 팩 bin."""
    sib = os.path.join(os.path.dirname(os.path.abspath(__file__)), WAKEUP_SCRIPT_BASENAME)
    return sib if os.path.exists(sib) else os.path.join(pack_dir(), "bin", WAKEUP_SCRIPT_BASENAME)


def wakeup_env():
    """큐 루트를 **명시 고정**해서 javis_wakeup 을 스폰한다(cwd 의존 제거).

    javis_wakeup 의 큐 루트는 `JAVIS_ROOT or os.getcwd()` 다. autopilot 은 schedule/launchd 가
    스폰하므로 cwd 가 `/` 인 채로 도는 일이 실제로 있었고(cwd=/ 오염 사고 계열), 그러면 큐가
    엉뚱한 곳에 생겨 escalation 이 **조용히** 사라진다. env 를 넘기는 기법 자체는
    `javis_report_gate.Runner.wk_env` 와 같은 관례다.

    값의 우선순위: 이미 배선된 `JAVIS_ROOT`(레인 배선이 이긴다) → `PROJECT`(이 스크립트가
    cycle 원장을 쓰는 그 루트 · 상단 `CYCLE_LOG` 와 동일 관례).
    ⚠레인 정합 주의: 게이트는 자기 레인 state dir(`CYS_REPORT_GATE_DIR`)를 큐 루트로 쓴다.
      루트가 갈리면 master 는 '게이트 digest 1건 + autopilot digest 1건'을 받는다(종전 N건
      대비 급감이지만 완전한 1건은 아니다). 설계 불변식은 **trigger-namespace 비중첩**
      (`gate-*` vs `autopilot-*`)이라 정합성 자체는 성립한다. 한 큐로 완전히 합치려면 레인
      배선에서 `JAVIS_ROOT` 를 게이트 state dir 로 맞춰 주면 이 함수가 그대로 따른다.
    """
    return dict(os.environ, JAVIS_ROOT=(os.environ.get("JAVIS_ROOT") or PROJECT))


def run_wakeup(cmd, timeout=RUN_TIMEOUT, stdin_text=None):
    """`run` 과 동형이되 큐 루트(JAVIS_ROOT)를 고정해 스폰하는 러너.

    `run` 의 시그니처에는 env 자리가 없고(계약 블록 = 변경 금지), 프로세스 env 를 일시
    변조하면 kill-switch 폴링 스레드의 스폰과 경쟁한다. 그래서 **전용 러너**를 따로 둔다 —
    주입 가능성(runner=…)은 그대로다.
    """
    try:
        p = subprocess.run(cmd, capture_output=True, text=True,
                           timeout=timeout, input=stdin_text, env=wakeup_env())
        return p.returncode, p.stdout or "", p.stderr or ""
    except Exception as e:  # noqa: BLE001 — 러너는 절대 예외를 올리지 않는다
        return 127, "", "runner error: %s" % e


def escalate(text, runner=run_wakeup, task_key=None):
    """master 로 1줄 escalation — **javis_wakeup 큐 경유**(설계 층1 I3).

    반환 계약 `(ok, why)` 와 "실패해도 예외를 올리지 않는다"는 종전 그대로다(원장 = 진실원,
    escalation 은 그 원장의 부수 통지다).

    `task_key` = escalation 의 **종류** 식별자. 큐의 병합 단위가 (target, task_key) 라 이 값이
    없으면 성격이 다른 사건들이 한 덩어리로 접힌다. 호출부가 자기 사건을 명시한다(미지정 시
    `autopilot-escalation`).

    멱등키 = `task_key` + 본문 sha1 접미. task_key 만 쓰면 **다른 사건**(다른 cycle_id·다른
    사유)까지 통째로 억제되고, 본문 전체를 키로 쓰면 같은 사건의 반복도 매번 새 키가 되어
    억제가 무동작한다. 본문 해시가 그 사이의 유일한 안정점이다(같은 문구=같은 사건).

    ok 판정:
      enqueue 실패              → (False, "enqueue rc=..")   큐에 들어가지도 못했다
      enqueue 성공 · drain 실패 → (True,  "queued(배달 보류)") 큐에 남아 다음 drain 이 배달한다
                                  (수신자 파킹은 escalation **자체**의 실패가 아니다)
    """
    if no_send():
        return False, "no_send"
    key = task_key or ESCALATION_TASK_DEFAULT
    idem = "%s-%s" % (key, hashlib.sha1(text.encode("utf-8")).hexdigest()[:12])
    script = wakeup_script()
    rc, _o, err = runner([sys.executable, script, "enqueue", "--to", "master",
                          "--task", key, "--reason", text,
                          "--severity", "critical", "--idempotency-key", idem])
    if rc != 0:
        return False, "enqueue rc=%d %s" % (rc, (err or "").strip()[:120])
    rc2, _o2, err2 = runner([sys.executable, script, "drain", "--deliver", "--target", "master"])
    if rc2 == 5:
        # javis_wakeup EXIT_EMPTY — 그 사이 다른 drain(게이트 주기)이 가져갔거나 멱등 억제로
        # 새 pending 이 없었다. 둘 다 "이미 전달됐다"는 뜻이라 성공이다.
        return True, ""
    if rc2 != 0:
        return True, "queued(배달 보류) drain rc=%d %s" % (rc2, (err2 or "").strip()[:120])
    return True, ""


# ── 모드·역할 노브 ────────────────────────────────────────────────────
MODE_SHADOW, MODE_LIVE = "shadow", "live"
IDLE_MIN = {"master": 180.0}      # 그 외 역할 = 60s
IDLE_MIN_DEFAULT = 60.0
OWNER_ACTIVE_WINDOW = 600.0       # $PACK/round/OWNER_ACTIVE mtime 10분
COOLDOWN_SECS = 1200.0            # cleared_verified 후 20분
SETTLE_SECS = 75.0                # 자식 종료 후 안정화 대기
# [v2.1 ④] in-flight kill-switch 폴링 — 설계 v2 의 2~5s 를 **1s 로 격상**한다.
# handshake~clear 구간만 격상해도 되지만, 외곽에서 stage 경계를 결정론으로 관측할 방법이 없어
# (자식 stderr 문구 파싱 = 화면 오라클) 자식 수명 **전 구간**을 1s 로 돌린다(엄격측).
# ★잔여 창(정직 표기): 검증자 allow → cycle-agent 의 clear 타이핑 사이 수 초는 회수 불가다.
#   1s 폴링은 그 창을 줄일 뿐 없애지 못한다 — 원장 detail.residual_window 로 매 사이클 명기한다.
KILL_POLL_SECS = 1.0
RESIDUAL_WINDOW_NOTE = ("allow→clear 구간 수초는 kill-switch 회수 불가(수용된 안전 한계 · "
                        "설계 v2.1 C1)")
RESET_COOLDOWN_SECS = 180.0       # 운영자 reset 후 재발화 최소 간격(무한 재시도 연타 방지)
LEASE_TTL = 900.0                 # lease 갱신 없음 + pid 사망 = 인계 대상
OBSERVE_WINDOW = 180.0            # audit 오탐 oracle 의 사후 관측 창
SWEEP_STATE = os.path.join(STATE_DIR, "sweep.json")   # [v2.1 ⑤] 틱 스윕 커서
SWEEP_MAX_EMIT = 20               # 1틱당 보충 기록 상한(폭주 차단)

PRENOTICE_TEXT = ("[CYCLE-PRE] 사이클 예정. **[CYCLE] 지시가 도착하면 그때** "
                  "대상 파일을 내용이 최신이어도 물리적으로 재기록하라")
RESUME_BASE = ("[RESUME] 컨텍스트 순환 완료. _round/SESSION_STATE.md와 자기 TODO를 읽고 "
               "직전 작업을 이어가라.")


def ctx_threshold(role, packdir=None):
    """역할 노브 context_clear_pct — $PACK/overrides/<base>.json (Rust overrides.rs:51-59 실측 대응).

    base 매핑: master / worker* / cso* / reviewer* / 그 외=worker. 범위 40-80 밖·손상은 60 폴백.
    """
    env = os.environ.get("CYS_AUTOPILOT_CTX_PCT", "").strip()
    if env.isdigit():
        return int(env)
    base = "worker"
    if role == "master":
        base = "master"
    elif role.startswith("worker"):
        base = "worker"
    elif role.startswith("cso"):
        base = "cso"
    elif role.startswith("reviewer"):
        base = "reviewer"
    p = os.path.join(packdir or pack_dir(), "overrides", "%s.json" % base)
    try:
        with open(p, "r", encoding="utf-8") as f:
            v = json.load(f).get("params", {}).get("context_clear_pct")
        if isinstance(v, int) and 40 <= v <= 80:
            return v
    except (OSError, ValueError, AttributeError):
        pass
    return 60


def mode():
    m = os.environ.get("CYS_AUTOPILOT_MODE", "").strip().lower() or MODE_SHADOW
    return m if m in (MODE_SHADOW, MODE_LIVE) else MODE_SHADOW


def roles():
    raw = os.environ.get("CYS_AUTOPILOT_ROLES", "").strip() or "worker"
    return [r.strip() for r in raw.split(",") if r.strip()]


def idle_min(role):
    return IDLE_MIN.get(role, IDLE_MIN_DEFAULT)


# ── 저장/복구 파일 세트 — [R2-A] **대상 surface 기준** 파생 ───────────
#
# ★codex R2 BLOCK 의 정확한 처방: role 문자열 + 실행 프로세스의 전역 PROJECT 로 정하면
#   master 가 실제로 쓰는 정본과 갈린다. 실측(2026-07-29): master surface:198 의 cwd 는
#   홈($HOME)이라 정본이 $HOME/_round/SESSION_STATE.md 인데,
#   스케줄 잡은 CYS_PROJECT_ROOT=<프로젝트 절대경로> 를 주입한다.
#   그대로면 stage2 가 영영 통과하지 못한다(fail-closed 이라 손실은 없지만 S2 불능).
def role_todo_file(role, packdir=None):
    """$PACK/round/<ROLE>_TODO.md — 그 역할 **단독 소유** 파일."""
    pd = packdir or pack_dir()
    return os.path.join(pd, "round", "%s_TODO.md" % role.upper().replace("-", "_"))


def node_round_dir(cwd, home=None, cys_root=None):
    """대상 노드의 `_round` 정본 디렉터리(순수) — `hooks/save-state.sh:24-32` **동형**.

    ① cwd 상향탐색: `_round` 를 가진 첫 디렉터리 (`/` 는 검사하지 않는다 — 원본과 동일)
    ② 폴백: `$CYS_ROOT`(미설정 시 `$HOME`)`/_round/ACTIVE_PROJECT` 첫 줄이 가리키는 프로젝트
    ③ 없으면 None
    반환 (round_dir|None, how) — how ∈ {"cwd-ascend","active-project","none"}
    """
    if cwd and os.path.isabs(cwd):
        d, prev = cwd, None
        while d and d != "/" and d != prev:
            if os.path.isdir(os.path.join(d, "_round")):
                return os.path.join(d, "_round"), "cwd-ascend"
            prev, d = d, os.path.dirname(d)
    root = cys_root or os.environ.get("CYS_ROOT", "").strip() or (
        home or os.path.expanduser("~"))
    try:
        with open(os.path.join(root, "_round", "ACTIVE_PROJECT"), "r", encoding="utf-8") as f:
            ap = f.readline().strip()
        if ap and os.path.isdir(os.path.join(ap, "_round")):
            return os.path.join(ap, "_round"), "active-project"
    except OSError:
        pass
    return None, "none"


def parse_cys_list(text):
    """`cys list` 텍스트 파싱(순수) — status --json 에 cwd 가 없을 때의 폴백.

    실측 형식(탭 구분): `surface:198\\trole=master\\tpid=92636\\texited=false\\t<title>\\t<cwd>`
    """
    out = {}
    for line in (text or "").splitlines():
        parts = [p for p in line.rstrip("\n").split("\t")]
        if len(parts) < 6 or not parts[0].startswith("surface:"):
            continue
        try:
            sid = int(parts[0].split(":", 1)[1])
        except ValueError:
            continue
        cwd = parts[-1].strip()
        role = parts[1][5:].strip() if parts[1].startswith("role=") else None
        out[sid] = {"role": role, "cwd": cwd}
    return out


def surface_cwd(row, runner=run):
    """대상 surface 의 실제 작업 디렉터리 — (cwd|None, source).

    우선순위: status --json 의 `live_cwd`(프로세스 실시간) → `cwd`(생성 시점) →
    `cys list` 마지막 필드 파싱(스키마 부재 대비 폴백).
    """
    for key in ("live_cwd", "cwd"):
        v = (row or {}).get(key)
        if isinstance(v, str) and v.strip():
            return v.strip(), "status.%s" % key
    sid = (row or {}).get("surface_id")
    if sid is None:
        return None, "none"
    rc, out, _e = runner([CYS, "list"])
    if rc == 0:
        got = parse_cys_list(out).get(sid)
        if got and got.get("cwd"):
            return got["cwd"], "cys-list"
    return None, "none"


def resolve_save_files(role, row, packdir=None, runner=run):
    """[R2-A] 대상 surface 기준 save-file **절대 목록** 파생 — 단일 출처.

    이 함수 결과를 lease 에 **1회** 저장하고, baseline·cycle-agent argv·검증자 매핑·
    사후검증 ⓓ 가 전부 그 lease 목록만 소비한다(재계산 금지 = 갈림 원천 제거).
    반환: {files[], round_dir, how, cwd, cwd_source, fallback(bool)}
    """
    todo = role_todo_file(role, packdir)
    cwd, cwd_src = surface_cwd(row, runner)
    rd, how = node_round_dir(cwd)
    fallback = rd is None
    if fallback:
        rd = os.path.join(PROJECT, "_round")   # 해석 실패 시에만 — 원장에 fallback 기록
        how = "project-fallback"
    files = ([os.path.join(rd, "SESSION_STATE.md"), todo] if role == "master" else [todo])
    return {"files": files, "round_dir": rd, "how": how, "cwd": cwd,
            "cwd_source": cwd_src, "fallback": fallback}


def resume_text(cycle_id):
    return "%s (nonce=%s)" % (RESUME_BASE, nonce_for(cycle_id))


def build_cycle_agent_argv(role, cycle_id, files):
    """cys cycle-agent argv(순수) — self-test 가 이 함수 결과를 박제한다.

    [R2-A] `files` 는 **반드시 lease 에 저장된 목록**을 그대로 넘긴다. 여기서 다시 파생하면
      baseline·argv·검증자·사후검증이 각자 계산해 갈릴 수 있다(단일 출처 원칙).

    ★`--force-no-verify` 는 **절대 금지**다. 저장 검증 없이 clear 하는 유일한 탈출구이고,
      전자동 개시자가 그것을 쥐는 순간 '미저장 clear'가 상시 경로가 된다. 어떤 인자·환경변수·
      실패 경로로도 이 리스트에 추가하지 말 것(self-test: force_no_verify_never_in_argv).
    """
    if not files:
        raise ValueError("save-file 목록이 비었다 — lease 저장 목록을 넘겨라(단일 출처)")
    return [CYS, "cycle-agent",
            "--role", role,
            "--verifier", VERIFIER_ROLE,
            "--timeout", str(CYCLE_AGENT_TIMEOUT),
            "--resume-text", resume_text(cycle_id)] + \
        [x for f in files for x in ("--save-file", f)]


# ── 측정(신선도 = 무효화 이벤트 부재) ─────────────────────────────────
def measure(row, role, now_ts, packdir=None):
    """statusline 측정 판정(순수). 반환 dict 의 ok 가 False 면 판정 투입 금지.

    설계 §2: source=="statusline" 만 채택(transcript/rollout/agy-rpc 금지 — D2 200k 전과).
    신선도는 wall-clock TTL 이 아니라 **무효화 부재**다:
      updated_at 이후 session_file 이 더 써졌으면(= assistant 활동) 그 보고는 이미 낡았다.
      유휴 노드의 오래된 statusline 은 유효하다(유휴=컨텍스트 불변).
    """
    u = (row or {}).get("usage") or {}
    out = {"ok": False, "reason": "", "source": u.get("source"),
           "ctx_pct": u.get("ctx_pct"), "ctx_tokens": u.get("ctx_tokens"),
           "session_file": u.get("session_file"), "updated_at": u.get("updated_at"),
           "threshold": ctx_threshold(role, packdir)}
    if not u:
        out["reason"] = "usage 부재"
        return out
    if u.get("source") != "statusline":
        out["reason"] = "source=%r (statusline 아님 — 판정 금지)" % u.get("source")
        return out
    sf, upd = u.get("session_file"), u.get("updated_at")
    if not sf or not isinstance(upd, (int, float)):
        out["reason"] = "session_file/updated_at 부재"
        return out
    mt = mtime_of(sf)
    if mt is None:
        out["reason"] = "session_file 접근 불가: %s" % sf
        return out
    out["session_mtime"] = mt
    if mt > upd + STAGE2_WINDOW:
        out["reason"] = ("무효화 — 보고(%.0f) 이후 세션파일이 %.0fs 더 써짐(assistant 활동)"
                         % (upd, mt - upd))
        return out
    pct = u.get("ctx_pct")
    if not isinstance(pct, (int, float)):
        out["reason"] = "ctx_pct 부재"
        return out
    out["ok"] = True
    out["over_threshold"] = pct >= out["threshold"]
    return out


# ── 게이트 6종 (순수 판정표) ──────────────────────────────────────────
def evaluate_gates(role, ctx, now_ts):
    """개시 안전 게이트 ALL-pass 판정(순수·부수효과 0).

    ctx 키:
      killed(bool), kill_reason(str), row(dict|None), measure(dict),
      owner_active_mtime(float|None), heartbeat_mtime(float|None),
      cycle_agent_procs(int), ledger(dict: last_terminal_phase/last_terminal_ts/
                                        cycles/incomplete/corrupt), lease_free(bool)
    반환: {"pass":bool, "exit":int, "gates":[{id,name,ok,detail}], "reason":str}
    """
    g = []

    def add(gid, name, ok, detail):
        g.append({"id": gid, "name": name, "ok": bool(ok), "detail": detail})

    # 1. kill-switch (2중 파일 + gate-check)
    add(1, "kill-switch", not ctx.get("killed"),
        ctx.get("kill_reason") or "clear")
    if ctx.get("killed"):
        return {"pass": False, "exit": EXIT_KILL, "gates": g,
                "reason": "kill-switch: %s" % (ctx.get("kill_reason") or "")}

    # 4-a. 원장 무결(짝짓기 fail-closed 의 전제) — 손상·미완결은 EXIT_LEDGER
    led = ctx.get("ledger") or {}
    if led.get("corrupt"):
        add(0, "원장 무결", False, "원장 손상(파싱 불가 라인 존재)")
        return {"pass": False, "exit": EXIT_LEDGER, "gates": g, "reason": "원장 손상"}
    if led.get("incomplete"):
        add(0, "원장 무결", False,
            "직전 사이클 미완결(terminal phase 부재): cycle_id=%s" % led.get("incomplete_cycle"))
        return {"pass": False, "exit": EXIT_LEDGER, "gates": g,
                "reason": "직전 사이클 미완결 — 발화 금지(fail-closed)"}

    # 2. 대상 유휴
    row = ctx.get("row")
    if not row:
        add(2, "대상 유휴", False, "role=%s surface 부재" % role)
    else:
        idle = row.get("idle_secs")
        qd = row.get("queue_depth")
        st = (row.get("status") or {}).get("state") if isinstance(row.get("status"), dict) else None
        need = idle_min(role)
        ok = (isinstance(idle, (int, float)) and idle >= need
              and qd == 0 and st != "working")
        add(2, "대상 유휴", ok,
            "idle=%s(>=%s) queue_depth=%s self_report=%s" % (idle, need, qd, st))

    # 2-b. 측정(신선도·임계)
    m = ctx.get("measure") or {}
    add(3, "측정 유효·임계 초과", bool(m.get("ok") and m.get("over_threshold")),
        "ok=%s reason=%s ctx_pct=%s threshold=%s"
        % (m.get("ok"), m.get("reason"), m.get("ctx_pct"), m.get("threshold")))

    # 3. 오너 존재 신호
    oam = ctx.get("owner_active_mtime")
    owner_idle = oam is None or (now_ts - oam) > OWNER_ACTIVE_WINDOW
    add(4, "오너 부재(OWNER_ACTIVE 10분)", owner_idle,
        "mtime=%s age=%s" % (oam, None if oam is None else round(now_ts - oam, 1)))

    # 4-b. 쿨다운 + 짝짓기
    if led.get("cycles", 0) == 0:
        add(5, "쿨다운·짝짓기", True, "원장 0건 — 부트스트랩 통과")
    else:
        last_phase = led.get("last_terminal_phase")
        last_ts = led.get("last_terminal_ts") or 0.0
        reset_ts = led.get("last_reset_ts") or 0.0
        if last_phase != SUCCESS_PHASE and reset_ts > last_ts:
            elapsed = now_ts - reset_ts
            add(5, "쿨다운·짝짓기", elapsed >= RESET_COOLDOWN_SECS,
                "직전=%s 이나 운영자 reset(%.0fs 전)이 종결보다 최신 — reset 쿨다운 %.0fs 적용"
                % (last_phase, elapsed, RESET_COOLDOWN_SECS))
        elif last_phase != SUCCESS_PHASE:
            add(5, "쿨다운·짝짓기", False,
                "직전 사이클 종결=%s (%s 아님 → 발화 금지)" % (last_phase, SUCCESS_PHASE))
        else:
            elapsed = now_ts - last_ts
            add(5, "쿨다운·짝짓기", elapsed >= COOLDOWN_SECS,
                "verified 후 %.0fs (필요 %.0fs)" % (elapsed, COOLDOWN_SECS))

    # 5. single-flight
    procs = ctx.get("cycle_agent_procs", 0)
    add(6, "single-flight", ctx.get("lease_free", False) and procs == 0,
        "lease_free=%s pgrep(cys cycle-agent)=%d" % (ctx.get("lease_free"), procs))

    # 6. 검증자 워처 생존
    hb = ctx.get("heartbeat_mtime")
    hb_ok = hb is not None and (now_ts - hb) <= HEARTBEAT_MAX_AGE
    add(7, "검증자 heartbeat", hb_ok,
        "mtime=%s age=%s (<= %.0fs)" % (hb, None if hb is None else round(now_ts - hb, 1),
                                        HEARTBEAT_MAX_AGE))

    failed = [x for x in g if not x["ok"]]
    if failed:
        return {"pass": False, "exit": EXIT_GATE, "gates": g,
                "reason": "; ".join("%s: %s" % (x["name"], x["detail"]) for x in failed)}
    return {"pass": True, "exit": EXIT_OK, "gates": g, "reason": "all gates pass"}


# ── phase 전이 ────────────────────────────────────────────────────────
def phase_transition_ok(cur, nxt):
    """상태기계 전이 합법성(순수). cur=None 은 armed 로만 진입 가능."""
    if cur is None:
        return nxt == "armed"
    if cur not in PHASE_NEXT:
        return False
    return nxt in PHASE_NEXT[cur]


# ── state.json (lease) ────────────────────────────────────────────────
def _ensure_state_dir():
    os.makedirs(STATE_DIR, exist_ok=True)


class _StateLock(object):
    """state.json 읽기-수정-쓰기 직렬화 — lease 획득의 상호배제(O_EXCL 대체·인계 가능)."""

    def __init__(self):
        self.fd = None

    def __enter__(self):
        _ensure_state_dir()
        self.fd = os.open(STATE_JSON + ".lock", os.O_RDWR | os.O_CREAT, 0o644)
        fcntl.flock(self.fd, fcntl.LOCK_EX)
        return self

    def __exit__(self, *a):
        try:
            fcntl.flock(self.fd, fcntl.LOCK_UN)
        finally:
            os.close(self.fd)
        return False


def load_state():
    try:
        with open(STATE_JSON, "r", encoding="utf-8") as f:
            d = json.load(f)
        return d if isinstance(d, dict) else {"version": 1}
    except (OSError, ValueError):
        return {"version": 1}


def save_state(st):
    _ensure_state_dir()
    tmp = STATE_JSON + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(st, f, ensure_ascii=False, indent=2, sort_keys=True)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, STATE_JSON)


def pid_alive(pid):
    if not isinstance(pid, int) or pid <= 0:
        return False
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError:
        return False


def lease_state(st, now_ts):
    """(lease|None, status) — status ∈ {none, live, dead, stale, terminal}."""
    lease = st.get("lease")
    if not isinstance(lease, dict):
        return None, "none"
    if lease.get("phase") in TERMINAL_PHASES:
        return lease, "terminal"
    if not pid_alive(lease.get("pid")):
        return lease, "dead"
    if now_ts - (lease.get("updated_at") or 0) > LEASE_TTL:
        return lease, "stale"
    return lease, "live"


def count_cycle_agent(runner=run):
    """pgrep -f 'cys cycle-agent' 건수. 판정 불능이면 보수적으로 1(=발화 금지)."""
    rc, out, _e = runner(["pgrep", "-f", "cys cycle-agent"])
    if rc == 1:
        return 0
    if rc != 0:
        return 1
    return len([x for x in out.split() if x.strip().isdigit()])


# ── 원장 뷰 (짝짓기·쿨다운 판정 입력) ─────────────────────────────────
def ledger_view(records, role, bad_lines):
    """role 기준 원장 요약(순수).

    - cycles: 그 role 의 서로 다른 cycle_id 수(would_fire·observe·reset 은 사이클로 세지 않는다)
    - last_terminal_phase/ts: 가장 최근 cycle_id 의 종결 phase
    - incomplete: 가장 최근 cycle_id 에 종결 phase 가 없다 = 미완결(fail-closed)
    - corrupt: 파싱 불가 라인 존재 또는 원장 읽기 불가
    """
    view = {"cycles": 0, "last_terminal_phase": None, "last_terminal_ts": None,
            "last_reset_ts": None,
            "incomplete": False, "incomplete_cycle": None,
            "corrupt": bool(bad_lines) and bad_lines != 0}
    if bad_lines and bad_lines != 0:
        view["corrupt"] = True
        return view
    by_cycle = {}
    for r in records:
        if r.get("role") != role:
            continue
        cid = r.get("cycle_id")
        ph = r.get("phase")
        if ph == "reset":
            ts = r.get("ts") or 0
            if ts > (view.get("last_reset_ts") or 0):
                view["last_reset_ts"] = ts
            continue
        if ph in ("would_fire", "observe", "verify", "bootstrap"):
            continue
        if not isinstance(cid, int):
            continue
        by_cycle.setdefault(cid, []).append(r)
    if not by_cycle:
        return view
    view["cycles"] = len(by_cycle)
    last_cid = max(by_cycle)
    terminal = [r for r in by_cycle[last_cid] if r.get("phase") in TERMINAL_PHASES]
    if not terminal:
        view["incomplete"] = True
        view["incomplete_cycle"] = last_cid
        return view
    terminal.sort(key=lambda r: r.get("ts") or 0)
    view["last_terminal_phase"] = terminal[-1].get("phase")
    view["last_terminal_ts"] = terminal[-1].get("ts")
    return view


# ── tick ──────────────────────────────────────────────────────────────
def collect_ctx(role, now_ts, status, killed, kill_reason, records, bad, lease_free,
                runner=run):
    row = surface_row(status, role)
    return {
        "killed": killed, "kill_reason": kill_reason,
        "row": row,
        "measure": measure(row, role, now_ts),
        "owner_active_mtime": mtime_of(os.path.join(pack_dir(), "round", "OWNER_ACTIVE")),
        "heartbeat_mtime": mtime_of(HEARTBEAT),
        "cycle_agent_procs": count_cycle_agent(runner),
        "ledger": ledger_view(records, role, bad),
        "lease_free": lease_free,
    }


def cmd_tick(args):
    """[v2.1 ③] **정상 skip·게이트 미통과는 전부 exit 0** 이다.

    schedule action:"command" 는 exit≠0 을 매 발화마다 schedule.error 로 표면화한다
    (schedule.rs:938-953 실측 · W-B 확인). 게이트 skip 은 정상 동작이므로 에러가 아니다 —
    사유는 CYCLE_LOG 와 stdout JSON 의 `gate_exit` 필드에 남긴다. 비0 은 진짜 내부 오류만.
    """
    now_ts = time.time()
    killed, kreason = kill_switch()
    if killed:
        log_append({"ts": now_ts, "cycle_id": None, "phase": "skip", "role": None,
                    "surface": None, "detail": {"gate": "kill-switch", "reason": kreason}})
        print(json.dumps({"result": "kill-switch", "reason": kreason, "gate_exit": EXIT_KILL},
                         ensure_ascii=False))
        return EXIT_OK

    with _StateLock():
        st = load_state()
        lease, lstatus = lease_state(st, now_ts)

    # ── in-flight 감독 / 죽은 집행자 인계 ──
    if lease and lstatus in ("live", "dead", "stale"):
        role = lease.get("role")
        cid = lease.get("cycle_id")
        if lstatus == "live":
            print(json.dumps({"result": "in-flight", "cycle_id": cid, "role": role,
                              "phase": lease.get("phase"), "pid": lease.get("pid")},
                             ensure_ascii=False))
            return EXIT_OK
        # 집행자 사망 — phase 로 인계 판정 [v2.1 ④]
        ph = lease.get("phase")
        if ph in ("executor_exited", "fired"):
            # fired 상태에서 집행자가 죽었어도 clear 가 났는지는 사후검증만이 안다 → 인계.
            verdict = post_verify(role, cid, lease, takeover=True)
            print(json.dumps({"result": "takeover-postverify", "cycle_id": cid,
                              "verdict": verdict["verdict"]}, ensure_ascii=False))
            return EXIT_OK
        _finalize(cid, role, lease.get("surface"), "failed",
                  {"takeover": True, "dead_phase": ph, "lease_status": lstatus})
        escalate("[CYCLE-AUTOPILOT] cycle-%s %s 집행자 사망(phase=%s) — failed 종결. "
                 "재개는 reset --role %s 후." % (cid, role, ph, role),
                 task_key="autopilot-executor-death")   # ★I3: 사건 종류별 병합 단위
        print(json.dumps({"result": "takeover-failed", "cycle_id": cid, "phase": ph},
                         ensure_ascii=False))
        return EXIT_OK

    sweep = sweep_ledger()   # [v2.1 ⑤] 훅 미경유 생산 경로 보충 기록

    status = fetch_status()
    if status is None:
        log_append({"ts": now_ts, "cycle_id": None, "phase": "skip", "role": None,
                    "surface": None, "detail": {"gate": "status", "reason": "cys status --json 조회 불가"}})
        print(json.dumps({"result": "skip", "reason": "status 조회 불가", "sweep": sweep,
                          "gate_exit": EXIT_GATE}, ensure_ascii=False))
        return EXIT_OK

    records, bad = read_ledger()
    _emit_observations(records, status, now_ts)

    report = []
    for role in roles():
        ctx = collect_ctx(role, now_ts, status, False, "", records, bad, True)
        verdict = evaluate_gates(role, ctx, now_ts)
        report.append({"role": role, "pass": verdict["pass"], "reason": verdict["reason"]})
        if not verdict["pass"]:
            if verdict["exit"] in (EXIT_KILL, EXIT_LEDGER):
                log_append({"ts": now_ts, "cycle_id": None, "phase": "skip", "role": role,
                            "surface": (ctx.get("row") or {}).get("surface_ref"),
                            "detail": {"gate_exit": verdict["exit"], "reason": verdict["reason"]}})
                print(json.dumps({"result": "blocked", "role": role, "sweep": sweep,
                                  "reason": verdict["reason"], "gate_exit": verdict["exit"]},
                                 ensure_ascii=False))
                return EXIT_OK
            continue

        cid = new_cycle_id()
        surface = (ctx.get("row") or {}).get("surface_ref")
        m = ctx["measure"]
        # [R2-A] save-file 목록은 여기서 **대상 surface 기준으로 1회** 파생한다.
        #   이후 baseline·argv·검증자 매핑·사후검증 ⓓ 는 전부 lease 의 이 목록만 소비한다.
        sfr = resolve_save_files(role, ctx.get("row"))
        detail = {"mode": mode(), "ctx_pct": m.get("ctx_pct"), "ctx_tokens": m.get("ctx_tokens"),
                  "threshold": m.get("threshold"), "session_file": m.get("session_file"),
                  "updated_at": m.get("updated_at"), "idle_secs": (ctx["row"] or {}).get("idle_secs"),
                  "queue_depth": (ctx["row"] or {}).get("queue_depth"),
                  "save_files": sfr["files"], "save_files_origin": {
                      "cwd": sfr["cwd"], "cwd_source": sfr["cwd_source"],
                      "round_dir": sfr["round_dir"], "how": sfr["how"],
                      "fallback": sfr["fallback"]},
                  "gates": [{"n": x["id"], "ok": x["ok"]} for x in verdict["gates"]]}
        if sfr["fallback"]:
            # 해석 실패 = 전역 PROJECT 폴백. 조용히 넘어가면 R2 BLOCK 이 재발하므로 원장에 못 박는다.
            log_append({"ts": now_ts, "cycle_id": cid, "phase": "skip", "role": role,
                        "surface": surface,
                        "detail": {"warn": "save-file 경로 해석 실패 — PROJECT 폴백 사용",
                                   "save_files_origin": detail["save_files_origin"]}})
        if mode() == MODE_SHADOW:
            # [R2-A] shadow 증거: 실제로 넘어갈 argv 를 그대로 원장에 남긴다(음성대조용).
            detail["cycle_agent_argv"] = build_cycle_agent_argv(role, cid, sfr["files"])
            log_append({"ts": now_ts, "cycle_id": cid, "phase": "would_fire", "role": role,
                        "surface": surface, "detail": detail})
            print(json.dumps({"result": "would_fire(shadow)", "role": role, "cycle_id": cid,
                              "save_files": sfr["files"], "sweep": sweep}, ensure_ascii=False))
            return EXIT_OK

        # live — lease 획득 후 집행자 detach 스폰
        with _StateLock():
            st = load_state()
            lease2, lstatus2 = lease_state(st, now_ts)
            if lease2 and lstatus2 == "live":
                print(json.dumps({"result": "race-lost", "role": role,
                                  "gate_exit": EXIT_GATE}, ensure_ascii=False))
                return EXIT_OK
            st["lease"] = {"cycle_id": cid, "role": role, "surface": surface,
                           "phase": "armed", "pid": None,
                           "started_at": now_ts, "updated_at": now_ts,
                           "pre": {"session_file": m.get("session_file"),
                                   "ctx_tokens": m.get("ctx_tokens"),
                                   "ctx_pct": m.get("ctx_pct"),
                                   "updated_at": m.get("updated_at")},
                           "save_files": sfr["files"],
                           "save_files_origin": detail["save_files_origin"]}
            save_state(st)
        log_append({"ts": now_ts, "cycle_id": cid, "phase": "armed", "role": role,
                    "surface": surface, "detail": detail})
        pid = _spawn_executor(cid, role)
        with _StateLock():
            st = load_state()
            if isinstance(st.get("lease"), dict) and st["lease"].get("cycle_id") == cid:
                st["lease"]["pid"] = pid
                st["lease"]["updated_at"] = time.time()
                save_state(st)
        print(json.dumps({"result": "fired", "role": role, "cycle_id": cid, "executor_pid": pid,
                          "sweep": sweep}, ensure_ascii=False))
        return EXIT_OK

    print(json.dumps({"result": "skip", "roles": report, "sweep": sweep,
                      "gate_exit": EXIT_GATE}, ensure_ascii=False))
    return EXIT_OK


def _emit_observations(records, status, now_ts):
    """audit 오탐 oracle 용 사후 관측 — 최근 would_fire 에 대해 관측 레코드 1건을 남긴다.

    would_fire 시점 직후 창(OBSERVE_WINDOW) 안에서 대상이 busy 로 전환됐는지를 사후에
    결정론으로 판정할 수 있게 하는 유일한 근거다(설계 §5 오탐 oracle (b)).
    """
    seen = set()
    for r in records:
        if r.get("phase") == "observe" and isinstance(r.get("cycle_id"), int):
            seen.add(r["cycle_id"])
    for r in records:
        if r.get("phase") != "would_fire":
            continue
        cid, ts = r.get("cycle_id"), r.get("ts") or 0
        if not isinstance(cid, int) or cid in seen:
            continue
        if not (0 < now_ts - ts <= OBSERVE_WINDOW):
            continue
        role = r.get("role")
        row = surface_row(status, role) or {}
        killed_now = any(os.path.exists(p) for p in paused_paths())
        log_append({"ts": now_ts, "cycle_id": cid, "phase": "observe", "role": role,
                    "surface": row.get("surface_ref"),
                    "detail": {"since_would_fire": round(now_ts - ts, 1),
                               "idle_secs": row.get("idle_secs"),
                               "queue_depth": row.get("queue_depth"),
                               "usage_source": ((row.get("usage") or {}).get("source")),
                               "session_file": ((row.get("usage") or {}).get("session_file")),
                               "usage_updated_at": ((row.get("usage") or {}).get("updated_at")),
                               "paused_file": killed_now}})
        seen.add(cid)


# ── [v2.1 ⑤] STATE_LEDGER 틱 스윕 ────────────────────────────────────
def sweep_scan(handoff_dir, tasks_dir):
    """현재 상태 관측(순수 I/O) — 훅을 경유하지 않은 생산 경로를 독립 관측한다.

    handoffs: 파일 목록(신규 출현만 본다)  /  tasks: id → status·owner (변화만 본다)
    """
    cur = {"handoffs": {}, "tasks": {}}
    try:
        for name in sorted(os.listdir(handoff_dir)):
            if not name.endswith(".md"):
                continue
            p = os.path.join(handoff_dir, name)
            cur["handoffs"][p] = mtime_of(p)
    except OSError:
        pass
    try:
        for name in sorted(os.listdir(tasks_dir)):
            if not name.endswith(".json"):
                continue
            d = read_json_file(os.path.join(tasks_dir, name)) or {}
            tid = d.get("id") or name[:-5]
            cur["tasks"][tid] = {"status": d.get("status"), "owner": d.get("owner"),
                                 "updated_at": d.get("updated_at")}
    except OSError:
        pass
    return cur


def sweep_plan(prev, cur, max_emit=SWEEP_MAX_EMIT):
    """(emits, seeded, truncated) — 순수.

    ★첫 스윕은 **seed** 다: 기록 0건으로 커서만 세운다. 그러지 않으면 과거 handoff 수십 건과
      완료 task 수십 건이 한꺼번에 원장에 쏟아져 SessionStart 주입 요약을 오염시킨다.
    커서는 emit 상한과 무관하게 항상 전진한다(상한 초과분은 truncated 로 계상 — 무한 재발화 금지).
    """
    if not prev or not isinstance(prev, dict) or not prev.get("seeded"):
        return [], True, 0
    emits = []
    old_h = prev.get("handoffs") or {}
    for p in sorted(cur.get("handoffs") or {}):
        if p in old_h:
            continue
        name = os.path.basename(p)
        emits.append({"type": "handoff", "fields": {"name": name[:-3] if name.endswith(".md")
                                                    else name, "path": p}})
    old_t = prev.get("tasks") or {}
    for tid in sorted(cur.get("tasks") or {}):
        new = (cur["tasks"][tid] or {}).get("status")
        old = (old_t.get(tid) or {}).get("status") if tid in old_t else None
        if tid in old_t and old == new:
            continue
        if new == "done":
            emits.append({"type": "task_done",
                          "fields": {"id": tid,
                                     "owner": (cur["tasks"][tid] or {}).get("owner") or "?"}})
        elif tid in old_t:
            emits.append({"type": "note",
                          "fields": {"text": "task %s status %s→%s" % (tid, old, new)}})
    truncated = max(0, len(emits) - max_emit)
    return emits[:max_emit], False, truncated


def sweep_ledger():
    """틱 스윕 집행 — 관측 → 계획 → STATE_LEDGER append → 커서 저장. best-effort."""
    handoff_dir = os.path.join(PROJECT, "_round", "handoffs")
    tasks_dir = os.path.join(PROJECT, "_round", "tasks")
    cur = sweep_scan(handoff_dir, tasks_dir)
    prev = read_json_file(SWEEP_STATE)
    emits, seeded, truncated = sweep_plan(prev, cur)
    wrote, failed = 0, 0
    for e in emits:
        ok, _why = ledger_append(e["type"], "cycle-autopilot-sweep", e["fields"])
        if ok:
            wrote += 1
        else:
            failed += 1
    cur["seeded"] = True
    cur["last_sweep"] = time.time()
    try:
        _ensure_state_dir()
        tmp = SWEEP_STATE + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(cur, f, ensure_ascii=False, sort_keys=True)
        os.replace(tmp, SWEEP_STATE)
    except OSError:
        pass
    return {"seeded": seeded, "emitted": wrote, "failed": failed, "truncated": truncated}


def _spawn_executor(cycle_id, role):
    """execute 를 setsid detach 로 스폰(틱은 즉시 종료 — 600s 예산 비점유)."""
    _ensure_state_dir()
    logdir = os.path.join(STATE_DIR, "exec_log")
    os.makedirs(logdir, exist_ok=True)
    logf = open(os.path.join(logdir, "execute-%d.log" % cycle_id), "ab")
    p = subprocess.Popen(
        [sys.executable, os.path.abspath(__file__), "execute",
         "--cycle-id", str(cycle_id), "--role", role],
        stdin=subprocess.DEVNULL, stdout=logf, stderr=subprocess.STDOUT,
        start_new_session=True, close_fds=True, cwd=PROJECT,
        env=dict(os.environ))
    return p.pid


# ── execute (집행자) ──────────────────────────────────────────────────
def _set_phase(cycle_id, role, surface, phase, detail):
    with _StateLock():
        st = load_state()
        lease = st.get("lease")
        if isinstance(lease, dict) and lease.get("cycle_id") == cycle_id:
            cur = lease.get("phase")
            if not phase_transition_ok(cur, phase):
                detail = dict(detail or {})
                detail["illegal_transition_from"] = cur
            lease["phase"] = phase
            lease["updated_at"] = time.time()
            st["lease"] = lease
            save_state(st)
    log_append({"ts": time.time(), "cycle_id": cycle_id, "phase": phase, "role": role,
                "surface": surface, "detail": detail})


def _finalize(cycle_id, role, surface, phase, detail):
    _set_phase(cycle_id, role, surface, phase, detail)
    with _StateLock():
        st = load_state()
        lease = st.get("lease")
        if isinstance(lease, dict) and lease.get("cycle_id") == cycle_id:
            st.setdefault("history", [])
            st["history"] = (st["history"] + [{"cycle_id": cycle_id, "role": role,
                                               "phase": phase, "ts": time.time()}])[-20:]
            st.pop("lease", None)
            save_state(st)


def cmd_execute(args):
    cid, role = args.cycle_id, args.role
    with _StateLock():
        st = load_state()
        lease = st.get("lease")
    if not isinstance(lease, dict) or lease.get("cycle_id") != cid:
        print("lease 불일치 — 집행 중단 (cycle_id=%s)" % cid, file=sys.stderr)
        return EXIT_ERR
    if not role:
        role = lease.get("role")
    surface = lease.get("surface")
    if mode() != MODE_LIVE:
        _finalize(cid, role, surface, "failed", {"reason": "execute 는 live 모드에서만 — 중단"})
        return EXIT_GATE

    killed, kreason = kill_switch()
    if killed:
        _finalize(cid, role, surface, "failed", {"reason": "집행 직전 kill-switch: %s" % kreason})
        return EXIT_KILL

    # 1) 선통보 (설계 v2 문안 고정 — 사전 저장 유발 금지)
    ok, why = push_line(role, PRENOTICE_TEXT)
    if not ok:
        _finalize(cid, role, surface, "failed", {"reason": "선통보 주입 실패: %s" % why})
        return EXIT_ERR
    _set_phase(cid, role, surface, "prenotified", {"prenotice": PRENOTICE_TEXT})

    # 2) [v2.1 ①] baseline 원장 기록 — **cycle-agent 실행 직전**. 검증자의 유일한 진실 기준.
    # [R2-A] 파일 목록은 lease 에 저장된 것만 쓴다. 여기서 재파생하면 tick 이 본 대상과
    #        execute 가 보는 대상이 갈릴 수 있다(단일 출처). 부재면 fail-closed.
    files = lease.get("save_files")
    if not files:
        _finalize(cid, role, surface, "failed",
                  {"reason": "lease 에 save_files 목록 없음 — 단일 출처 위반, 집행 중단"})
        return EXIT_ERR
    start_ts = time.time()
    bl = write_baseline(cid, role, surface, files, start_ts)
    with _StateLock():
        st = load_state()
        if isinstance(st.get("lease"), dict) and st["lease"].get("cycle_id") == cid:
            st["lease"]["baseline_path"] = bl["path"]
            st["lease"]["fired_at"] = start_ts
            save_state(st)

    # 3) cycle-agent 를 자식으로 — 1s 간격 kill-switch 폴링, 감지 즉시 SIGTERM
    argv = build_cycle_agent_argv(role, cid, files)
    _set_phase(cid, role, surface, "fired",
               {"argv": argv, "baseline": bl["path"], "file_set": bl["file_set"],
                "save_files_origin": lease.get("save_files_origin"),
                "started_at": start_ts, "poll_secs": KILL_POLL_SECS,
                "residual_window": RESIDUAL_WINDOW_NOTE})
    ledger_append("cycle", "cycle-autopilot",
                  {"phase": "fired", "id": nonce_for(cid), "role": role})
    child = subprocess.Popen(argv, stdin=subprocess.DEVNULL,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    # 자식 출력은 별도 스레드로 계속 빨아낸다 — 1s 폴링 루프가 PIPE 를 안 읽어 자식이
    # 블로킹되면 kill-switch 감시 자체가 무의미해진다.
    sink = {"buf": []}

    def _drain():
        try:
            for line in child.stdout:
                sink["buf"].append(line)
                if len(sink["buf"]) > 400:
                    del sink["buf"][:200]
        except Exception:                  # noqa: BLE001
            pass
    dr = threading.Thread(target=_drain, daemon=True)
    dr.start()

    aborted = None
    while True:
        rc = child.poll()
        if rc is not None:
            break
        k, kr = kill_switch()
        if k:
            aborted = kr
            try:
                child.terminate()          # SIGTERM — stage3 allow 이전 kill = clear 불가능
                child.wait(timeout=10)
            except Exception:              # noqa: BLE001
                try:
                    child.kill()
                except Exception:          # noqa: BLE001
                    pass
            break
        time.sleep(KILL_POLL_SECS)
    dr.join(timeout=5)
    tail = ("".join(sink["buf"]))[-1200:]
    rc = child.poll()

    if aborted is not None:
        _finalize(cid, role, surface, "failed",
                  {"reason": "in-flight kill-switch: %s" % aborted, "child_rc": rc,
                   "tail": tail[-400:], "residual_window": RESIDUAL_WINDOW_NOTE})
        escalate("[CYCLE-AUTOPILOT] cycle-%d %s in-flight kill-switch 로 중단(%s)."
                 % (cid, role, aborted),
                 task_key="autopilot-killswitch")       # ★I3: 사건 종류별 병합 단위
        return EXIT_KILL

    # 4) [v2.1 ④] 자식 종료 = executor_exited (clear 성공을 뜻하지 않는다). exit code 는 기록만.
    _set_phase(cid, role, surface, "executor_exited",
               {"child_rc": rc, "tail": tail[-800:], "started_at": start_ts})
    time.sleep(SETTLE_SECS)
    with _StateLock():
        st = load_state()
        lease = st.get("lease") or {}
    v = post_verify(role, cid, lease)
    return EXIT_OK if v["verdict"] == SUCCESS_PHASE else EXIT_ERR


def write_baseline(cycle_id, role, surface, files, started_at):
    """[v2.1 ①] 사이클 baseline 레코드 — cycle-agent 실행 **직전** 상태를 박제한다.

    검증자는 이 레코드 없이는 어떤 요청도 allow 하지 않는다. body 의 해시는 cycle-agent 가
    저장 검증 통과 **후** 계산한 값이라, 그것만으로는 '무엇에 대비해 바뀌었나'를 말할 수 없다
    (codex R1 BLOCK — 저장 전/후 대비 없이 '재기록됨'을 주장할 수 없다).
    """
    os.makedirs(BASELINE_DIR, exist_ok=True)
    file_set = sorted(set(files))
    rec = {"cycle_id": int(cycle_id), "role": role, "surface": surface,
           "nonce": nonce_for(cycle_id), "started_at": float(started_at),
           "file_set": file_set,
           "files": dict((f, file_state(f)) for f in file_set)}
    p = baseline_path(cycle_id)
    tmp = p + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(rec, f, ensure_ascii=False, sort_keys=True, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, p)
    _prune_baselines()
    rec["path"] = p
    return rec


def _prune_baselines(keep=50):
    try:
        names = sorted(n for n in os.listdir(BASELINE_DIR)
                       if n.startswith("cycle-") and n.endswith(".json"))
    except OSError:
        return
    for n in names[:-keep] if len(names) > keep else []:
        try:
            os.remove(os.path.join(BASELINE_DIR, n))
        except OSError:
            pass


# ── 사후검증 ──────────────────────────────────────────────────────────
def postverify_decide(pre, row, nonce_hits, save_mtimes, start_ts):
    """[v2.1 ④] 사후검증 3분기(순수) — 실효 확인 / clear 미실행 확인 / 불명.

    선결 조건 = **측정 유효성**: source=="statusline" AND updated_at > 개시시각.
      그게 아니면 세션 교체 여부 자체를 말할 수 없다 → indeterminate. (성공으로도 실패로도
      세지 않는다. 자식 exit code 를 성공 신호로 쓰던 종전 오류의 정확한 처방이다.)
    실효 확인(cleared_verified) = ⓐ 신규 session_file + ctx_tokens **상대 급락**
      (절대 하한 금지 — 복원 재주입 baseline 이 노드마다 다르다)
      AND ⓑ **신규** 세션 jsonl 에 nonce 도착 AND ⓓ 복구 읽기 파일 **전건** 재기록.
    [R2-B] ⓑ 는 session_file 이 교체됐을 때만 참이 될 수 있다. 세션이 그대로면 nonce_hits 가
      몇이든 False 다 — 구 세션에 남은 문양은 도착 증거가 아니다(실측 오염 cycle 1785338591).
    clear 미실행 확인(failed_preclear) = 측정은 유효한데 session_file 이 **그대로**.
    ⓓ 는 [v2.1 ①] baseline ALL-match 계약과 정합하도록 ALL 이다 — 검증자가 전 파일 재기록을
      이미 강제하므로 ALL 이 lockout 을 만들지 않는다(v2 의 ANY 편차는 철회).
    """
    u = (row or {}).get("usage") or {}
    res = {"measurement_valid": False, "a_effective": False, "b_nonce": False,
           "d_recovery": False, "queue_depth": (row or {}).get("queue_depth"),
           "stale_recovery": [], "detail": {}}
    new_sf, new_upd = u.get("session_file"), u.get("updated_at")
    new_tok = u.get("ctx_tokens")
    old_sf, old_tok = (pre or {}).get("session_file"), (pre or {}).get("ctx_tokens")
    res["detail"] = {"old_session_file": old_sf, "new_session_file": new_sf,
                     "old_ctx_tokens": old_tok, "new_ctx_tokens": new_tok,
                     "usage_source": u.get("source"), "usage_updated_at": new_upd}
    res["measurement_valid"] = bool(
        u.get("source") == "statusline" and isinstance(new_upd, (int, float))
        and new_upd > start_ts)
    session_changed = bool(new_sf and old_sf and new_sf != old_sf)
    res["session_changed"] = session_changed
    res["a_effective"] = bool(
        res["measurement_valid"] and session_changed
        and isinstance(new_tok, (int, float)) and isinstance(old_tok, (int, float))
        and new_tok < old_tok)
    # [R2-B] 세션 파일이 그대로면 그 파일의 nonce 문양은 stage5 **도착** 증거가 아니다
    #   (구 세션의 argv·로그 열람 흔적). 신규 세션에서만 유효 — 오염 차단을 여기서도 강제한다
    #   (post_verify 가 이미 0 을 넘기지만, 순수 함수 단독 호출자도 같은 계약을 받도록 이중 방어).
    res["b_nonce"] = bool(session_changed and nonce_hits and nonce_hits > 0)
    stale = [f for f, mt in (save_mtimes or {}).items()
             if mt is None or mt <= start_ts]
    res["stale_recovery"] = sorted(stale)
    res["d_recovery"] = bool(save_mtimes) and not stale

    if not res["measurement_valid"]:
        res["verdict"] = "indeterminate"
        res["reason"] = "측정 무효(source=%s, updated_at 이 개시 이후 아님) — 판정 불능" % u.get("source")
    elif res["a_effective"] and res["b_nonce"] and res["d_recovery"]:
        res["verdict"] = SUCCESS_PHASE
        res["reason"] = "실효 확인(ⓐⓑⓓ 전건)"
    elif new_sf and old_sf and new_sf == old_sf:
        res["verdict"] = "failed_preclear"
        res["reason"] = "측정 유효한데 session_file 불변 — clear 미실행 확인"
    else:
        miss = [k for k, ok in (("ⓐ실효", res["a_effective"]), ("ⓑnonce", res["b_nonce"]),
                                ("ⓓ복구파일", res["d_recovery"])) if not ok]
        res["verdict"] = "indeterminate"
        res["reason"] = "증거 불충분(%s) — 성공·실패 어느 쪽으로도 확정 불가" % ",".join(miss)
    return res


def _count_nonce(session_file, cycle_id):
    if not session_file:
        return 0
    pat = "(nonce=%s)" % nonce_for(cycle_id)  # 정확 문양 — lease/baseline 경로의 cycle-<id> 오염 차단(S1 실측)
    try:
        n = 0
        with open(session_file, "r", encoding="utf-8", errors="replace") as f:
            for line in f:
                n += line.count(pat)
        return n
    except OSError:
        return 0


def post_verify(role, cycle_id, lease, takeover=False):
    start_ts = (lease or {}).get("fired_at") or (lease or {}).get("started_at") or 0.0
    pre = (lease or {}).get("pre") or {}
    files = (lease or {}).get("save_files") or []   # [R2-A] 단일 출처 — 재파생 금지
    status = fetch_status()
    row = surface_row(status, role)
    new_sf = ((row or {}).get("usage") or {}).get("session_file")
    old_sf = pre.get("session_file")
    # [R2-B] nonce 는 **신규 세션 파일에서만** 센다. 세션이 그대로면 그 파일에 찍힌 문양은
    #   stage5 도착 증거가 아니라 구 세션의 로그 열람 흔적(argv·도구 출력)일 뿐이다
    #   — 실패 cycle 1785338591 에서 session_file 불변인데 hits=3 이 나온 실측 오염.
    hits_raw = _count_nonce(new_sf, cycle_id)
    session_changed = bool(new_sf and old_sf and new_sf != old_sf)
    hits = hits_raw if session_changed else 0
    mtimes = {f: mtime_of(f) for f in files}
    v = postverify_decide(pre, row, hits, mtimes, start_ts)
    v["takeover"] = takeover
    v["nonce_hits"] = hits
    v["nonce_hits_raw"] = hits_raw            # 오염 관측용(판정 미투입)
    v["nonce_counted_from"] = new_sf if session_changed else None
    if not files:
        v["save_files_missing"] = True
    v["save_mtimes"] = {os.path.basename(f): mtimes[f] for f in mtimes}

    # ⓒ /clear 큐 걸림 — 재집행 금지, 복원 포인터만 멱등 선적재
    if not v["a_effective"] and (v.get("queue_depth") or 0) > 0:
        rc, _o, e = run(["python3", os.path.join(pack_dir(), "bin", "javis_wakeup.py"), "enqueue",
                         "--to", role, "--task", "cycle-resume",
                         "--reason", "cycle-%d clear 큐 걸림 — 복원 포인터 선적재" % cycle_id,
                         "--idempotency-key", nonce_for(cycle_id)])
        v["wakeup_enqueued"] = (rc == 0)
        v["wakeup_err"] = e.strip()[:160] if rc != 0 else ""

    surface = (row or {}).get("surface_ref") or (lease or {}).get("surface")
    _finalize(cycle_id, role, surface, v["verdict"], v)
    ledger_append("cycle", "cycle-autopilot",
                  {"phase": v["verdict"], "id": nonce_for(cycle_id), "role": role})
    if v["verdict"] != SUCCESS_PHASE:
        reasons = [v.get("reason") or v["verdict"]]
        if v["stale_recovery"]:
            reasons.append("stale=%s" % ",".join(os.path.basename(x) for x in v["stale_recovery"]))
        escalate("[CYCLE-AUTOPILOT] cycle-%d %s 사후검증 %s(%s). 해제: "
                 "javis_cycle_autopilot.py reset --role %s --reason '<사유>'"
                 % (cycle_id, role, v["verdict"], " ".join(reasons), role),
                 task_key="autopilot-postverify")       # ★I3: 사건 종류별 병합 단위
    return v


# ── audit (S0 오탐 oracle) ────────────────────────────────────────────
def audit_records(records):
    """would_fire ↔ observe 대조로 오탐 판정(순수·설계 §5).

    오탐 성립 조건(하나라도):
      (a) 측정 무효화 위반 — would_fire 시점의 session_file mtime 이 보고 이후로 밀려 있었다
          (틱이 기록한 measure 무효 사유가 남아 있으면 그것으로 판정)
      (b) 직후 OBSERVE_WINDOW 안에서 대상 busy 전환(idle 리셋 = idle_secs 감소, 또는 queue>0)
      (c) 발화 시점에 PAUSED 파일 존재
    """
    obs = {}
    for r in records:
        if r.get("phase") == "observe" and isinstance(r.get("cycle_id"), int):
            obs.setdefault(r["cycle_id"], []).append(r)
    out = {"would_fire": 0, "observed": 0, "false_positive": 0, "unpaired": 0, "items": []}
    for r in records:
        if r.get("phase") != "would_fire":
            continue
        out["would_fire"] += 1
        cid = r.get("cycle_id")
        d = r.get("detail") or {}
        reasons = []
        if d.get("paused_file"):
            reasons.append("c:PAUSED 존재 중 발화")
        pairs = obs.get(cid) or []
        if not pairs:
            out["unpaired"] += 1
        else:
            out["observed"] += 1
            pairs.sort(key=lambda x: x.get("ts") or 0)
            o = (pairs[0].get("detail") or {})
            if o.get("paused_file"):
                reasons.append("c:직후 PAUSED 존재")
            oi, od = d.get("idle_secs"), o.get("idle_secs")
            if isinstance(oi, (int, float)) and isinstance(od, (int, float)) and od < oi:
                reasons.append("b:직후 idle 리셋(%s→%s)" % (oi, od))
            if isinstance(o.get("queue_depth"), int) and o["queue_depth"] > 0:
                reasons.append("b:직후 queue_depth=%d" % o["queue_depth"])
            if o.get("usage_source") == "statusline" and o.get("session_file") \
                    and d.get("session_file") and o["session_file"] != d["session_file"]:
                reasons.append("a:측정 무효화(직후 session_file 교체)")
        if reasons:
            out["false_positive"] += 1
        out["items"].append({"cycle_id": cid, "role": r.get("role"),
                             "ts": r.get("ts"), "false_positive": bool(reasons),
                             "reasons": reasons})
    return out


def cmd_audit(args):
    records, bad = read_ledger()
    if bad:
        print(json.dumps({"error": "원장 손상 라인 %s건" % bad}, ensure_ascii=False))
        return EXIT_LEDGER
    res = audit_records(records)
    res["mode"] = mode()
    print(json.dumps(res, ensure_ascii=False, indent=2))
    return EXIT_OK


# ── reset / status / bootstrap-verifier ───────────────────────────────
def cmd_reset(args):
    now_ts = time.time()
    with _StateLock():
        st = load_state()
        lease = st.get("lease")
        cleared = None
        if isinstance(lease, dict) and (args.role is None or lease.get("role") == args.role):
            cleared = lease
            st.pop("lease", None)
            save_state(st)
    log_append({"ts": now_ts, "cycle_id": (cleared or {}).get("cycle_id"), "phase": "reset",
                "role": args.role, "surface": (cleared or {}).get("surface"),
                "detail": {"reason": args.reason, "cleared_lease": bool(cleared),
                           "cleared_phase": (cleared or {}).get("phase"),
                           "by": os.environ.get("USER") or "?"}})
    print(json.dumps({"result": "reset", "role": args.role, "cleared_lease": bool(cleared),
                      "reason": args.reason}, ensure_ascii=False))
    return EXIT_OK


def cmd_status(args):
    now_ts = time.time()
    killed, kreason = kill_switch()
    status = fetch_status()
    records, bad = read_ledger()
    with _StateLock():
        st = load_state()
    lease, lstatus = lease_state(st, now_ts)
    out = {"mode": mode(), "roles": roles(), "no_send": no_send(),
           "kill_switch": {"killed": killed, "reason": kreason},
           "paused_paths": {p: os.path.exists(p) for p in paused_paths()},
           "lease": lease, "lease_status": lstatus,
           "heartbeat_age": (None if mtime_of(HEARTBEAT) is None
                             else round(now_ts - mtime_of(HEARTBEAT), 1)),
           "ledger": {"records": len(records), "bad_lines": bad, "path": CYCLE_LOG},
           "gates": {}}
    for role in roles():
        ctx = collect_ctx(role, now_ts, status or {}, killed, kreason, records, bad,
                          lease is None or lstatus in ("none", "terminal"))
        v = evaluate_gates(role, ctx, now_ts)
        sfr = resolve_save_files(role, ctx.get("row"))   # [R2-A] 대상 surface 기준 dry 파생
        out["gates"][role] = {"pass": v["pass"], "exit": v["exit"], "reason": v["reason"],
                              "detail": v["gates"],
                              "measure": ctx["measure"],
                              "save_files": sfr["files"],
                              "save_files_origin": {k: sfr[k] for k in
                                                    ("cwd", "cwd_source", "round_dir",
                                                     "how", "fallback")},
                              "cycle_agent_argv": build_cycle_agent_argv(role, 0, sfr["files"])}
    print(json.dumps(out, ensure_ascii=False, indent=2, default=str))
    return EXIT_OK


def cmd_bootstrap_verifier(args):
    """검증자 전용 pane 신설 + 워처 포그라운드 기동 (게이트6 전제).

    ★pane 안에서 포그라운드로 돌려야 한다 — detach/데몬 spawn 은 feed.reply 가
      self_approval_denied 로 fail-closed 된다(cysd state.rs:1046-1074 실측: 외부 프로세스인데
      어떤 surface 에도 귀속되지 않으면 자기승인으로 간주).
    """
    watcher = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_cycle_verifier.py")
    if not os.path.exists(watcher):
        print("검증자 스크립트 부재: %s" % watcher, file=sys.stderr)
        return EXIT_ERR
    cmdline = "python3 %s watch" % watcher
    plan = {"new_surface": [CYS, "new-surface", "--role", VERIFIER_ROLE,
                            "--title", "cycle-verifier", "--cwd", PROJECT],
            "send": cmdline}
    if args.dry_run:
        print(json.dumps({"result": "dry-run", "plan": plan}, ensure_ascii=False, indent=2))
        return EXIT_OK
    if no_send():
        print(json.dumps({"result": "blocked", "reason": "CYS_AUTOPILOT_NO_SEND=1"},
                         ensure_ascii=False))
        return EXIT_GATE
    rc, out, err = run(plan["new_surface"])
    if rc != 0:
        print("new-surface 실패: %s" % err.strip()[:200], file=sys.stderr)
        return EXIT_ERR
    ref = (out or "").strip().split()[-1] if out.strip() else ""
    if not ref.startswith("surface:"):
        print("surface ref 파싱 실패: %r" % out, file=sys.stderr)
        return EXIT_ERR
    rc1, _o, e1 = run([CYS, "send", "--surface", ref, cmdline])
    rc2, _o2, e2 = run([CYS, "send-key", "--surface", ref, "Return"])
    ok = rc1 == 0 and rc2 == 0
    log_append({"ts": time.time(), "cycle_id": None, "phase": "bootstrap", "role": VERIFIER_ROLE,
                "surface": ref, "detail": {"ok": ok, "cmd": cmdline,
                                           "err": (e1 + e2).strip()[:200]}})
    print(json.dumps({"result": "bootstrap", "surface": ref, "ok": ok}, ensure_ascii=False))
    return EXIT_OK if ok else EXIT_ERR


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


def _raises(fn):
    """호출이 예외를 올리면 True (계약 위반이 조용히 통과하지 않는지 확인용)."""
    try:
        fn()
    except Exception:  # noqa: BLE001
        return True
    return False


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
    print("== javis_cycle_autopilot.py self-test ==")

    # 1) 동시성 append
    print("[1] 원장 동시 append (O_APPEND + flock + 단일 write)")
    tmpd = tempfile.mkdtemp(prefix="cycautotest-")
    logp = os.path.join(tmpd, "cycle_autopilot_log.jsonl")
    kids, NPROC, NLINE = [], 8, 60
    for i in range(NPROC):
        pid = os.fork()
        if pid == 0:
            try:
                for j in range(NLINE):
                    log_append({"ts": time.time(), "cycle_id": i * 1000 + j, "phase": "would_fire",
                                "role": "w%d" % i, "surface": "surface:%d" % i,
                                "detail": {"x": "y" * 200}}, path=logp)
            finally:
                os._exit(0)
        kids.append(pid)
    for pid in kids:
        os.waitpid(pid, 0)
    lines = [l for l in open(logp, encoding="utf-8").read().splitlines() if l.strip()]
    parsed, badk = 0, 0
    for l in lines:
        try:
            r = json.loads(l)
        except ValueError:
            continue
        parsed += 1
        if set(r.keys()) != set(LOG_KEYS):
            badk += 1
    t.check("append 라인수 == %d" % (NPROC * NLINE), len(lines) == NPROC * NLINE,
            "실제 %d" % len(lines))
    t.check("전 라인 JSON 파싱", parsed == len(lines), "%d/%d" % (parsed, len(lines)))
    t.check("전 라인 키 == LOG_KEYS", badk == 0, "위반 %d" % badk)

    # 2) 로그 절단
    print("[2] LOG_MAX_BYTES 절단")
    big = os.path.join(tmpd, "big.jsonl")
    log_append({"ts": 1.0, "cycle_id": 1, "phase": "armed", "role": "worker",
                "surface": "surface:1", "detail": {"blob": "z" * 20000}}, path=big)
    raw = open(big, "rb").read()
    t.check("4KB 이하 단일 라인", len(raw) <= LOG_MAX_BYTES and raw.count(b"\n") == 1,
            "len=%d" % len(raw))

    # 3) phase 전이 [v2.1 ④]
    print("[3] phase 전이표 (v2.1 의미론)")
    t.check("None→armed 합법", phase_transition_ok(None, "armed"))
    t.check("armed→prenotified 합법", phase_transition_ok("armed", "prenotified"))
    t.check("prenotified→fired 합법", phase_transition_ok("prenotified", "fired"))
    t.check("fired→executor_exited 합법", phase_transition_ok("fired", "executor_exited"))
    t.check("executor_exited→cleared_verified 합법",
            phase_transition_ok("executor_exited", "cleared_verified"))
    t.check("executor_exited→failed_preclear 합법",
            phase_transition_ok("executor_exited", "failed_preclear"))
    t.check("executor_exited→indeterminate 합법",
            phase_transition_ok("executor_exited", "indeterminate"))
    t.check("모든 비종결 phase→failed 합법",
            all(phase_transition_ok(p, "failed") for p in
                ("armed", "prenotified", "fired", "executor_exited")))
    t.check("fired→cleared_verified 불법(사후검증 건너뛰기)",
            not phase_transition_ok("fired", "cleared_verified"))
    t.check("'cleared' 는 더 이상 phase 가 아니다",
            "cleared" not in PHASES and not phase_transition_ok("fired", "cleared"))
    t.check("cleared_verified→armed 불법(종결 재개)",
            not phase_transition_ok("cleared_verified", "armed"))
    t.check("None→fired 불법", not phase_transition_ok(None, "fired"))
    t.check("SUCCESS_PHASE 만 성공 종결",
            SUCCESS_PHASE == "cleared_verified" and SUCCESS_PHASE in TERMINAL_PHASES
            and set(TERMINAL_PHASES) == {"cleared_verified", "failed_preclear",
                                         "indeterminate", "failed"})

    # 4) 측정 — 무효화 부재 규칙
    print("[4] 측정 신선도 = 무효화 부재")
    sess = os.path.join(tmpd, "sess.jsonl")
    open(sess, "w").write("x")
    now_ts = time.time()
    os.utime(sess, (now_ts - 1000, now_ts - 1000))

    def row_of(source, pct, upd, sf=sess, tok=700000):
        return {"role": "worker", "idle_secs": 300, "queue_depth": 0, "status": None,
                "surface_ref": "surface:9",
                "usage": {"source": source, "ctx_pct": pct, "ctx_tokens": tok,
                          "session_file": sf, "updated_at": upd}}
    m = measure(row_of("statusline", 70, now_ts - 900), "worker", now_ts)
    t.check("유휴 노드의 오래된 statusline 은 유효", m["ok"], m.get("reason"))
    t.check("임계 초과 판정", m.get("over_threshold") is True)
    m2 = measure(row_of("transcript", 70, now_ts - 10), "worker", now_ts)
    t.check("source!=statusline → 판정 금지", not m2["ok"], m2.get("reason"))
    m3 = measure(row_of("statusline", 70, now_ts - 5000), "worker", now_ts)
    t.check("보고 이후 세션파일 활동 → 무효화", not m3["ok"], m3.get("reason"))
    m4 = measure(row_of("statusline", 30, now_ts - 900), "worker", now_ts)
    t.check("임계 미만 → over_threshold False", m4["ok"] and not m4["over_threshold"])
    m5 = measure(row_of("statusline", 70, now_ts - 900, sf="/nonexistent/x.jsonl"),
                 "worker", now_ts)
    t.check("session_file 접근 불가 → 판정 금지", not m5["ok"], m5.get("reason"))

    # 5) 게이트 판정표
    print("[5] 개시 안전 게이트 판정표")
    base_row = {"role": "worker", "idle_secs": 300, "queue_depth": 0, "status": None,
                "surface_ref": "surface:9",
                "usage": {"source": "statusline", "ctx_pct": 70, "ctx_tokens": 700000,
                          "session_file": sess, "updated_at": now_ts - 900}}

    def ctx_of(**kw):
        c = {"killed": False, "kill_reason": "", "row": dict(base_row),
             "owner_active_mtime": None, "heartbeat_mtime": now_ts - 10,
             "cycle_agent_procs": 0,
             "ledger": {"cycles": 0, "last_terminal_phase": None, "last_terminal_ts": None,
                        "incomplete": False, "corrupt": False},
             "lease_free": True}
        c["measure"] = measure(c["row"], "worker", now_ts)
        for k, v in kw.items():
            if k == "row_patch":
                c["row"].update(v)
                c["measure"] = measure(c["row"], "worker", now_ts)
            else:
                c[k] = v
        return c

    v = evaluate_gates("worker", ctx_of(), now_ts)
    t.check("기준선(부트스트랩): 전 게이트 통과", v["pass"], v["reason"])
    v = evaluate_gates("worker", ctx_of(killed=True, kill_reason="PAUSED"), now_ts)
    t.check("kill-switch → exit 4", (not v["pass"]) and v["exit"] == EXIT_KILL)
    v = evaluate_gates("worker", ctx_of(row_patch={"idle_secs": 30}), now_ts)
    t.check("worker idle 30s(<60) → skip", not v["pass"])
    v = evaluate_gates("master", ctx_of(row_patch={"idle_secs": 100, "role": "master"}), now_ts)
    t.check("master idle 100s(<180) → skip", not v["pass"])
    v = evaluate_gates("worker", ctx_of(row_patch={"idle_secs": 300, "queue_depth": 3}), now_ts)
    t.check("queue_depth>0 → skip", not v["pass"])
    v = evaluate_gates("worker", ctx_of(row_patch={"status": {"state": "working"}}), now_ts)
    t.check("자기보고 working → 보수적 skip", not v["pass"])
    v = evaluate_gates("worker", ctx_of(owner_active_mtime=now_ts - 60), now_ts)
    t.check("OWNER_ACTIVE 1분 전 → skip", not v["pass"])
    v = evaluate_gates("worker", ctx_of(owner_active_mtime=now_ts - 1200), now_ts)
    t.check("OWNER_ACTIVE 20분 전 → 통과", v["pass"], v["reason"])
    v = evaluate_gates("worker", ctx_of(heartbeat_mtime=now_ts - 200), now_ts)
    t.check("verifier heartbeat 200s → skip", not v["pass"])
    v = evaluate_gates("worker", ctx_of(heartbeat_mtime=None), now_ts)
    t.check("verifier heartbeat 부재 → skip", not v["pass"])
    v = evaluate_gates("worker", ctx_of(cycle_agent_procs=1), now_ts)
    t.check("pgrep cycle-agent 1건 → skip", not v["pass"])
    v = evaluate_gates("worker", ctx_of(lease_free=False), now_ts)
    t.check("lease 점유 → skip", not v["pass"])
    v = evaluate_gates("worker", ctx_of(ledger={"cycles": 2, "last_terminal_phase": SUCCESS_PHASE,
                                                "last_terminal_ts": now_ts - 100,
                                                "incomplete": False, "corrupt": False}), now_ts)
    t.check("쿨다운 100s(<1200) → skip", not v["pass"])
    v = evaluate_gates("worker", ctx_of(ledger={"cycles": 2, "last_terminal_phase": SUCCESS_PHASE,
                                                "last_terminal_ts": now_ts - 1500,
                                                "incomplete": False, "corrupt": False}), now_ts)
    t.check("쿨다운 1500s → 통과", v["pass"], v["reason"])
    for bad_phase in ("failed", "failed_preclear", "indeterminate"):
        v = evaluate_gates("worker", ctx_of(ledger={"cycles": 2,
                                                    "last_terminal_phase": bad_phase,
                                                    "last_terminal_ts": now_ts - 9999,
                                                    "incomplete": False, "corrupt": False}), now_ts)
        t.check("직전 %s → 발화 금지(짝짓기)" % bad_phase, not v["pass"])

    # 5-b) [R2-C] reset 해제 분기 직접 회귀 4검체
    print("[5-b] reset 짝짓기 해제 분기 (RESET_COOLDOWN_SECS=%.0f)" % RESET_COOLDOWN_SECS)
    FAIL_TS = now_ts - 9999

    def led_reset(reset_ts, last_ts=FAIL_TS, phase="failed_preclear"):
        d = {"cycles": 2, "last_terminal_phase": phase, "last_terminal_ts": last_ts,
             "incomplete": False, "corrupt": False}
        if reset_ts is not None:
            d["last_reset_ts"] = reset_ts
        return d
    v = evaluate_gates("worker", ctx_of(ledger=led_reset(None)), now_ts)
    t.check("① reset 전(실패 종결만) → 발화 금지", not v["pass"], v["reason"])
    v = evaluate_gates("worker", ctx_of(ledger=led_reset(now_ts - 179)), now_ts)
    t.check("② reset + 179s → 아직 금지(쿨다운 미충족)", not v["pass"], v["reason"])
    v = evaluate_gates("worker", ctx_of(ledger=led_reset(now_ts - 180)), now_ts)
    t.check("③ reset + 180s → 통과", v["pass"], v["reason"])
    v = evaluate_gates("worker",
                       ctx_of(ledger=led_reset(now_ts - 600, last_ts=now_ts - 300)), now_ts)
    t.check("④ reset 후 다음 실패 terminal 기록 → 재잠금(자동 재시도 루프 없음)",
            not v["pass"], v["reason"])
    v = evaluate_gates("worker",
                       ctx_of(ledger=led_reset(FAIL_TS - 1, last_ts=FAIL_TS)), now_ts)
    t.check("  reset 이 종결보다 **과거**면 해제 아님", not v["pass"], v["reason"])
    v = evaluate_gates("worker",
                       ctx_of(ledger=led_reset(now_ts - 10, last_ts=now_ts - 1500,
                                               phase=SUCCESS_PHASE)), now_ts)
    t.check("  성공 종결에는 reset 분기가 개입하지 않음(정상 쿨다운 경로)", v["pass"], v["reason"])
    lvr = ledger_view([{"ts": 100, "cycle_id": 1, "phase": "failed_preclear", "role": "worker"},
                       {"ts": 300, "cycle_id": None, "phase": "reset", "role": "worker"}],
                      "worker", 0)
    t.check("  원장의 reset 레코드가 last_reset_ts 로 집계(사이클로는 미계상)",
            lvr["last_reset_ts"] == 300 and lvr["cycles"] == 1 and not lvr["incomplete"])
    v = evaluate_gates("worker", ctx_of(ledger={"cycles": 1, "incomplete": True,
                                                "incomplete_cycle": 7, "corrupt": False}), now_ts)
    t.check("원장 미완결 → exit 5", (not v["pass"]) and v["exit"] == EXIT_LEDGER)
    v = evaluate_gates("worker", ctx_of(ledger={"cycles": 1, "corrupt": True}), now_ts)
    t.check("원장 손상 → exit 5", (not v["pass"]) and v["exit"] == EXIT_LEDGER)
    v = evaluate_gates("worker", ctx_of(row=None), now_ts)
    t.check("대상 surface 부재 → skip", not v["pass"])
    v = evaluate_gates("worker", ctx_of(row_patch={"usage": {"source": "rollout:heuristic",
                                                             "ctx_pct": 90, "ctx_tokens": 1,
                                                             "session_file": sess,
                                                             "updated_at": now_ts}}), now_ts)
    t.check("비-statusline 측정 → skip", not v["pass"])

    # 6) 원장 뷰
    print("[6] 원장 뷰(짝짓기 입력)")
    recs = [{"ts": 100, "cycle_id": 1, "phase": "armed", "role": "worker"},
            {"ts": 200, "cycle_id": 1, "phase": SUCCESS_PHASE, "role": "worker"},
            {"ts": 300, "cycle_id": 2, "phase": "armed", "role": "worker"}]
    lv = ledger_view(recs, "worker", 0)
    t.check("최근 사이클 미완결 감지", lv["incomplete"] and lv["incomplete_cycle"] == 2)
    lv2 = ledger_view(recs[:2], "worker", 0)
    t.check("완결 사이클 → last_terminal=cleared_verified",
            lv2["last_terminal_phase"] == SUCCESS_PHASE and lv2["last_terminal_ts"] == 200)
    lv2b = ledger_view([{"ts": 10, "cycle_id": 3, "phase": "fired", "role": "worker"},
                        {"ts": 20, "cycle_id": 3, "phase": "executor_exited", "role": "worker"}],
                       "worker", 0)
    t.check("executor_exited 는 종결이 아니다 → incomplete", lv2b["incomplete"])
    lv3 = ledger_view([{"ts": 1, "cycle_id": 9, "phase": "would_fire", "role": "worker"}],
                      "worker", 0)
    t.check("would_fire 는 사이클로 세지 않음", lv3["cycles"] == 0 and not lv3["incomplete"])
    lv4 = ledger_view(recs, "master", 0)
    t.check("타 역할 레코드는 무관", lv4["cycles"] == 0)
    lv5 = ledger_view([], "worker", 3)
    t.check("bad_lines>0 → corrupt", lv5["corrupt"])

    # 7) argv·저장세트 계약 — [R2-A] 대상 surface 기준 파생
    print("[7] save-file 대상 surface 파생 + argv 계약")

    def norow(role, cwd, sid=9):
        return {"role": role, "surface_id": sid, "live_cwd": cwd, "cwd": cwd,
                "surface_ref": "surface:%d" % sid}

    # 7-a) node_round_dir — save-state.sh 동형 상향탐색 (실디렉터리 대조)
    HOME = os.path.expanduser("~")
    rd, how = node_round_dir(HOME)
    t.check("cwd=$HOME → 홈 _round 정본 (save-state.sh 동형)",
            rd == os.path.join(HOME, "_round") and how == "cwd-ascend", "%s/%s" % (rd, how))
    rd2, how2 = node_round_dir(os.path.join(HOME, "Desktop", "CYSjavis"))
    t.check("cwd=프로젝트 → 프로젝트 _round",
            rd2 == os.path.join(HOME, "Desktop", "CYSjavis", "_round") and how2 == "cwd-ascend",
            str(rd2))
    deep = os.path.join(tmpd, "a", "b", "c")
    os.makedirs(os.path.join(tmpd, "a", "_round"))
    os.makedirs(deep)
    rd3, _h3 = node_round_dir(deep)
    t.check("깊은 하위에서 상향탐색으로 조상 _round 발견",
            rd3 == os.path.join(tmpd, "a", "_round"), str(rd3))
    rd4, how4 = node_round_dir("relative/not/abs")
    t.check("상대경로는 상향탐색 안 함(무한루프 방지)", how4 != "cwd-ascend")
    rd5, how5 = node_round_dir("/nonexistent-xyz", cys_root=tmpd)
    t.check("_round 도 ACTIVE_PROJECT 도 없으면 None", rd5 is None and how5 == "none")
    ap_root = os.path.join(tmpd, "aproot")
    os.makedirs(os.path.join(ap_root, "_round"))
    open(os.path.join(ap_root, "_round", "ACTIVE_PROJECT"), "w").write(
        os.path.join(tmpd, "a") + "\n")
    rd6, how6 = node_round_dir("/nonexistent-xyz", cys_root=ap_root)
    t.check("ACTIVE_PROJECT 폴백 동작",
            rd6 == os.path.join(tmpd, "a", "_round") and how6 == "active-project", str(rd6))

    # 7-b) ★R2 BLOCK 회귀 핀 — master cwd=$HOME 이면 홈 정본이 나와야 한다
    sfr_m = resolve_save_files("master", norow("master", HOME, 198), packdir="/PK")
    t.check("★[R2-A] master(cwd=$HOME) save-file = 홈 _round 정본",
            sfr_m["files"] == [os.path.join(HOME, "_round", "SESSION_STATE.md"),
                               "/PK/round/MASTER_TODO.md"], str(sfr_m["files"]))
    t.check("★[R2-A] 전역 PROJECT 에 결박되지 않음",
            os.path.join(PROJECT, "_round", "SESSION_STATE.md") not in sfr_m["files"]
            or PROJECT == HOME, str(sfr_m["files"]))
    t.check("파생 출처가 기록됨(cwd·round_dir·how·fallback)",
            sfr_m["cwd"] == HOME and sfr_m["how"] == "cwd-ascend"
            and sfr_m["fallback"] is False and sfr_m["cwd_source"] == "status.live_cwd")
    sfr_mp = resolve_save_files("master", norow("master", os.path.join(HOME, "Desktop", "CYSjavis"),
                                                198), packdir="/PK")
    t.check("master(cwd=프로젝트) 는 프로젝트 정본",
            sfr_mp["files"][0] == os.path.join(HOME, "Desktop", "CYSjavis", "_round",
                                               "SESSION_STATE.md"), str(sfr_mp["files"]))
    sfr_w = resolve_save_files("worker", norow("worker", os.path.join(HOME, "Desktop", "CYSjavis")),
                               packdir="/PK")
    t.check("worker save-file 단독 WORKER_TODO.md(round_dir 무관)",
            sfr_w["files"] == ["/PK/round/WORKER_TODO.md"], str(sfr_w["files"]))
    t.check("reviewer-codex → REVIEWER_CODEX_TODO.md",
            resolve_save_files("reviewer-codex", norow("reviewer-codex", HOME),
                               packdir="/PK")["files"] == ["/PK/round/REVIEWER_CODEX_TODO.md"])

    # 7-c) cwd 해석 실패 → PROJECT 폴백 + fallback 플래그
    sfr_fb = resolve_save_files("master", {"role": "master", "surface_id": None},
                                packdir="/PK", runner=lambda *a, **k: (1, "", "no daemon"))
    t.check("cwd 해석 실패 → PROJECT 폴백 + fallback=True",
            sfr_fb["fallback"] is True
            and sfr_fb["files"][0] == os.path.join(PROJECT, "_round", "SESSION_STATE.md"),
            str(sfr_fb))

    # 7-d) cys list 폴백 파싱
    listing = ("surface:198\trole=master\tpid=92636\texited=false\tmaster-claude · testhost\t"
               "/Users/x/h\n"
               "surface:200\trole=worker\tpid=6352\texited=false\tworker · CYSjavis\t"
               "/Users/x/proj\n"
               "쓰레기 줄\n")
    parsed = parse_cys_list(listing)
    t.check("cys list 파싱 — surface_id·role·cwd",
            parsed[198]["cwd"] == "/Users/x/h" and parsed[198]["role"] == "master"
            and parsed[200]["cwd"] == "/Users/x/proj", str(parsed))
    t.check("cys list 형식 위반 줄 무시", len(parsed) == 2)
    cwd_fb, src_fb = surface_cwd({"surface_id": 198},
                                 runner=lambda *a, **k: (0, listing, ""))
    t.check("status 에 cwd 없으면 cys list 폴백",
            cwd_fb == "/Users/x/h" and src_fb == "cys-list")

    # 7-e) argv 는 넘겨받은 목록만 소비 (단일 출처)
    argv_w = build_cycle_agent_argv("worker", 1234567, sfr_w["files"])
    argv_m = build_cycle_agent_argv("master", 1234567, sfr_m["files"])
    t.check("force_no_verify_never_in_argv(worker)", "--force-no-verify" not in argv_w)
    t.check("force_no_verify_never_in_argv(master)", "--force-no-verify" not in argv_m)
    t.check("--verifier cycle-verifier 고정",
            argv_w[argv_w.index("--verifier") + 1] == VERIFIER_ROLE)
    t.check("resume-text 에 nonce 포함",
            ("nonce=%s" % nonce_for(1234567)) in argv_w[argv_w.index("--resume-text") + 1])
    t.check("argv 에 lease 목록 전량 동봉", all(f in argv_m for f in sfr_m["files"]))
    t.check("★argv 가 자체 파생하지 않음(빈 목록이면 거부)",
            _raises(lambda: build_cycle_agent_argv("master", 1, [])))
    import inspect as _insp
    t.check("build_cycle_agent_argv 안에서 재파생 호출 없음",
            "resolve_save_files" not in _insp.getsource(build_cycle_agent_argv))
    t.check("execute 가 lease 목록만 소비(재파생 금지)",
            'lease.get("save_files")' in _insp.getsource(cmd_execute)
            and "resolve_save_files" not in _insp.getsource(cmd_execute))
    t.check("post_verify 가 lease 목록만 소비",
            "resolve_save_files" not in _insp.getsource(post_verify))

    # 8) 사후검증 3분기 [v2.1 ④]
    print("[8] 사후검증 판정표 (cleared_verified / failed_preclear / indeterminate)")
    pre = {"session_file": "/a/old.jsonl", "ctx_tokens": 700000, "ctx_pct": 70}
    st0 = 1000.0

    def prow(sf, tok, upd, q=0, src="statusline"):
        return {"queue_depth": q, "usage": {"session_file": sf, "ctx_tokens": tok,
                                            "updated_at": upd, "source": src}}
    r = postverify_decide(pre, prow("/a/new.jsonl", 20000, 1200.0), 3,
                          {"/f/a": 1100.0}, st0)
    t.check("정상 사이클 → cleared_verified", r["verdict"] == SUCCESS_PHASE, str(r))
    r = postverify_decide(pre, prow("/a/old.jsonl", 20000, 1200.0), 3,
                          {"/f/a": 1100.0}, st0)
    t.check("session_file 불변(측정 유효) → failed_preclear",
            r["verdict"] == "failed_preclear", str(r))
    r = postverify_decide(pre, prow("/a/new.jsonl", 20000, 1200.0, src="transcript"), 3,
                          {"/f/a": 1100.0}, st0)
    t.check("source=transcript → indeterminate(측정 무효)",
            r["verdict"] == "indeterminate" and not r["measurement_valid"])
    r = postverify_decide(pre, prow("/a/new.jsonl", 20000, 900.0), 3,
                          {"/f/a": 1100.0}, st0)
    t.check("statusline 이 개시 이전 → indeterminate", r["verdict"] == "indeterminate")
    r = postverify_decide(pre, prow("/a/new.jsonl", 900000, 1200.0), 3,
                          {"/f/a": 1100.0}, st0)
    t.check("신규 세션인데 ctx 급락 없음 → indeterminate",
            r["verdict"] == "indeterminate" and not r["a_effective"])
    r = postverify_decide(pre, prow("/a/new.jsonl", 20000, 1200.0), 0,
                          {"/f/a": 1100.0}, st0)
    t.check("nonce 미도착 → indeterminate", r["verdict"] == "indeterminate" and not r["b_nonce"])
    r = postverify_decide(pre, prow("/a/new.jsonl", 20000, 1200.0), 3,
                          {"/f/a": 900.0}, st0)
    t.check("복구파일 mtime 과거 → indeterminate + stale 보고",
            r["verdict"] == "indeterminate" and r["stale_recovery"] == ["/f/a"])
    r = postverify_decide(pre, prow("/a/new.jsonl", 20000, 1200.0), 3,
                          {"/f/a": 1100.0, "/f/b": 900.0}, st0)
    t.check("일부만 신선 → ⓓ ALL 실패(v2.1 계약)",
            not r["d_recovery"] and r["stale_recovery"] == ["/f/b"])
    r = postverify_decide(pre, prow("/a/new.jsonl", 20000, 1200.0), 3,
                          {"/f/a": 1100.0, "/f/b": 1100.0}, st0)
    t.check("전건 신선 → ⓓ 통과", r["d_recovery"] and r["verdict"] == SUCCESS_PHASE)
    r = postverify_decide(pre, prow("/a/old.jsonl", 700000, 1200.0, q=2), 0,
                          {"/f/a": 900.0}, st0)
    t.check("큐 걸림 시나리오 → failed_preclear + queue_depth 보존",
            r["verdict"] == "failed_preclear" and r["queue_depth"] == 2)
    t.check("어떤 분기도 자식 exit code 를 입력으로 쓰지 않는다",
            "child_rc" not in json.dumps(r, default=str))

    # 8-a2) [R2-B] nonce 오라클 오염 차단 — 실패 cycle 1785338591 재현형
    print("[8-a2] nonce 오라클 (신규 세션에서만 유효)")
    r = postverify_decide(pre, prow("/a/old.jsonl", 431710, 1200.0), 3,
                          {"/f/a": 1100.0}, st0)
    t.check("★[R2-B] 구세션 hits=3 인데 session_file 동일 → b_nonce False",
            r["b_nonce"] is False and r["session_changed"] is False, str(r))
    t.check("  그 경우 판정은 failed_preclear(성공으로 새지 않음)",
            r["verdict"] == "failed_preclear")
    r = postverify_decide(pre, prow("/a/new.jsonl", 20000, 1200.0), 3,
                          {"/f/a": 1100.0}, st0)
    t.check("신규 세션 + hits>0 → b_nonce True", r["b_nonce"] is True and r["session_changed"])
    r = postverify_decide({"session_file": None, "ctx_tokens": 700000},
                          prow("/a/new.jsonl", 20000, 1200.0), 3, {"/f/a": 1100.0}, st0)
    t.check("pre.session_file 부재 → 교체 판정 불가 → b_nonce False(보수)",
            r["b_nonce"] is False)
    src_pv = _insp.getsource(post_verify)
    t.check("post_verify 도 신규 세션에서만 카운트(원시값은 관측용 보존)",
            "session_changed" in src_pv and "nonce_hits_raw" in src_pv)
    # 정확 문양 계약(구 cycle-<id> 부분일치 오염 차단)
    nf = os.path.join(tmpd, "sess_nonce.jsonl")
    open(nf, "w", encoding="utf-8").write(
        "라인1 baseline path /baselines/cycle-777.json\n"
        "라인2 [RESUME] ... (nonce=cycle-777)\n")
    t.check("정확 문양 '(nonce=cycle-N)' 만 카운트", _count_nonce(nf, 777) == 1,
            str(_count_nonce(nf, 777)))
    t.check("세션 파일 부재 → 0", _count_nonce(None, 777) == 0)

    # 8-b) baseline 기록 계약 [v2.1 ①]
    print("[8-b] baseline 레코드 계약")
    bdir = os.path.join(tmpd, "bl")
    f1 = os.path.join(tmpd, "SS.md")
    open(f1, "w").write("v1")
    globals()["BASELINE_DIR"] = bdir
    rec = write_baseline(4242, "worker", "surface:9", [f1, "/nope/absent.md"], 5000.0)
    t.check("baseline 파일 생성", os.path.exists(rec["path"]))
    saved = read_json_file(rec["path"])
    t.check("file_set 정렬·중복제거", saved["file_set"] == sorted({f1, "/nope/absent.md"}))
    t.check("존재 파일 해시 기록", saved["files"][f1]["sha256"] == sha256_file(f1))
    t.check("미존재 파일은 exists=False·sha256=None",
            saved["files"]["/nope/absent.md"]["exists"] is False
            and saved["files"]["/nope/absent.md"]["sha256"] is None)
    t.check("started_at·nonce·role 각인",
            saved["started_at"] == 5000.0 and saved["nonce"] == "cycle-4242"
            and saved["role"] == "worker")
    t.check("baseline_path 규칙 일치", rec["path"] == baseline_path(4242))
    globals()["BASELINE_DIR"] = os.path.join(STATE_DIR, "baselines")

    # 8-c) STATE_LEDGER 틱 스윕 [v2.1 ⑤]
    print("[8-c] STATE_LEDGER 틱 스윕 계획")
    hd, td = os.path.join(tmpd, "handoffs"), os.path.join(tmpd, "tasks")
    os.makedirs(hd)
    os.makedirs(td)
    open(os.path.join(hd, "old-stage.md"), "w").write("x")
    json.dump({"id": "T-1", "status": "doing", "owner": "worker"},
              open(os.path.join(td, "T-1.json"), "w"))
    cur1 = sweep_scan(hd, td)
    emits, seeded, trunc = sweep_plan(None, cur1)
    t.check("첫 스윕은 seed — 기록 0건", seeded and emits == [] and trunc == 0)
    cur1["seeded"] = True
    open(os.path.join(hd, "new-stage.md"), "w").write("y")
    json.dump({"id": "T-1", "status": "done", "owner": "worker"},
              open(os.path.join(td, "T-1.json"), "w"))
    json.dump({"id": "T-2", "status": "todo", "owner": None},
              open(os.path.join(td, "T-2.json"), "w"))
    cur2 = sweep_scan(hd, td)
    emits, seeded, trunc = sweep_plan(cur1, cur2)
    kinds = sorted((e["type"], e["fields"].get("name") or e["fields"].get("id") or "")
                   for e in emits)
    t.check("신규 handoff 1건만 emit",
            ("handoff", "new-stage") in kinds and ("handoff", "old-stage") not in kinds, str(kinds))
    t.check("status→done 은 task_done", ("task_done", "T-1") in kinds, str(kinds))
    t.check("신규 미완료 task 는 emit 안 함(seed 이후 신규는 note 아님)",
            all(not (e["type"] == "task_done" and e["fields"].get("id") == "T-2")
                for e in emits))
    emits3, _s3, _t3 = sweep_plan(cur2, cur2)
    t.check("변화 없으면 0건(무한 재발화 금지)", emits3 == [])
    many = {"seeded": True, "handoffs": {}, "tasks": {}}
    cur3 = {"handoffs": dict((("/h/%03d.md" % i), 1.0) for i in range(40)), "tasks": {}}
    e4, _s4, tr4 = sweep_plan(many, cur3, max_emit=20)
    t.check("emit 상한 20 + truncated 계상", len(e4) == 20 and tr4 == 20)

    # 9) audit 오탐 oracle
    print("[9] audit 오탐 oracle")
    ar = audit_records([
        {"ts": 10, "cycle_id": 1, "phase": "would_fire", "role": "worker",
         "detail": {"idle_secs": 300, "session_file": "/s/a"}},
        {"ts": 60, "cycle_id": 1, "phase": "observe", "role": "worker",
         "detail": {"idle_secs": 400, "queue_depth": 0, "usage_source": "statusline",
                    "session_file": "/s/a"}},
        {"ts": 10, "cycle_id": 2, "phase": "would_fire", "role": "worker",
         "detail": {"idle_secs": 300, "session_file": "/s/a"}},
        {"ts": 60, "cycle_id": 2, "phase": "observe", "role": "worker",
         "detail": {"idle_secs": 5, "queue_depth": 0}},
        {"ts": 10, "cycle_id": 3, "phase": "would_fire", "role": "worker",
         "detail": {"idle_secs": 300, "paused_file": True}},
        {"ts": 60, "cycle_id": 3, "phase": "observe", "role": "worker", "detail": {}},
        {"ts": 10, "cycle_id": 4, "phase": "would_fire", "role": "worker",
         "detail": {"idle_secs": 300, "session_file": "/s/a"}},
        {"ts": 60, "cycle_id": 4, "phase": "observe", "role": "worker",
         "detail": {"idle_secs": 400, "queue_depth": 2}},
    ])
    t.check("would_fire 4건 집계", ar["would_fire"] == 4, str(ar))
    t.check("오탐 3건(idle 리셋·PAUSED·queue)", ar["false_positive"] == 3, str(ar))
    t.check("정상 1건은 오탐 아님",
            [i for i in ar["items"] if i["cycle_id"] == 1][0]["false_positive"] is False)
    ar2 = audit_records([{"ts": 10, "cycle_id": 5, "phase": "would_fire", "role": "w",
                          "detail": {}}])
    t.check("관측 미짝 → unpaired 계상", ar2["unpaired"] == 1)

    # 10) 계약 블록 동일성 (이음매 드리프트)
    print("[10] 두 스크립트 계약 블록 동일성")
    mine = extract_contract_block(os.path.abspath(__file__))
    sib = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_cycle_verifier.py")
    t.check("자기 계약 블록 추출", bool(mine))
    if os.path.exists(sib):
        other = extract_contract_block(sib)
        t.check("javis_cycle_verifier.py 와 바이트 동일", mine == other,
                "" if mine == other else "블록 불일치 — 한쪽만 수정됨")
    else:
        t.check("sibling 부재(SKIP 처리)", True, "(verifier 미배치)")

    # 11) ctx_threshold 노브
    print("[11] context_clear_pct 노브")
    pk = os.path.join(tmpd, "pack")
    os.makedirs(os.path.join(pk, "overrides"))
    open(os.path.join(pk, "overrides", "worker.json"), "w").write(
        '{"params":{"context_clear_pct":75}}')
    t.check("overrides 반영", ctx_threshold("worker-2", packdir=pk) == 75)
    t.check("파일 없는 역할 → 60 폴백", ctx_threshold("master", packdir=pk) == 60)
    open(os.path.join(pk, "overrides", "cso.json"), "w").write(
        '{"params":{"context_clear_pct":999}}')
    t.check("범위 밖 → 60 폴백", ctx_threshold("cso", packdir=pk) == 60)

    # 12) 틱 exit 계약 [v2.1 ③] — 정적 회귀 핀
    print("[12] 틱 exit 계약 (게이트 skip 은 exit 0)")
    import inspect
    tick_src = inspect.getsource(cmd_tick)
    leaks = [tok for tok in ("return EXIT_GATE", "return EXIT_KILL", "return EXIT_LEDGER",
                             'return verdict["exit"]') if tok in tick_src]
    t.check("cmd_tick 이 비0 게이트 코드를 반환하지 않음", not leaks, str(leaks))
    t.check("cmd_tick 이 gate_exit 필드로 사유를 노출", "gate_exit" in tick_src)
    t.check("cmd_tick 이 스윕을 호출", "sweep_ledger()" in tick_src)
    exec_src = inspect.getsource(cmd_execute)
    t.check("execute 가 baseline 을 cycle-agent **이전**에 기록",
            exec_src.index("write_baseline(") < exec_src.index("subprocess.Popen(argv"))
    t.check("execute 가 executor_exited 로 종료 phase 기록",
            '"executor_exited"' in exec_src and '"cleared"' not in exec_src)
    t.check("in-flight 폴링 1s", KILL_POLL_SECS == 1.0)

    print("\n결과: PASS %d / FAIL %d" % (t.ok, len(t.fail)))
    if t.fail:
        for n in t.fail:
            print("  - %s" % n)
        return EXIT_ERR
    return EXIT_OK


# ── main ──────────────────────────────────────────────────────────────
def main(argv=None):
    ap = argparse.ArgumentParser(description="컨텍스트 사이클 전자동 상태기계 (컴포넌트 1)")
    sub = ap.add_subparsers(dest="cmd")
    sub.add_parser("tick", help="개시 판정 1틱(짧게·집행은 detach)")
    ex = sub.add_parser("execute", help="집행자(내부용 — tick 이 detach 스폰)")
    ex.add_argument("--cycle-id", type=int, required=True)
    ex.add_argument("--role")
    sub.add_parser("audit", help="S0 shadow 오탐 oracle 집계")
    rs = sub.add_parser("reset", help="lockout 해제(사유 원장 기록)")
    rs.add_argument("--role", required=True)
    rs.add_argument("--reason", required=True)
    bv = sub.add_parser("bootstrap-verifier", help="검증자 pane 신설 + 워처 기동")
    bv.add_argument("--dry-run", action="store_true")
    sub.add_parser("status", help="현재 모드·lease·게이트 판정(부수효과 0)")
    sub.add_parser("self-test", help="데몬 없이 픽스처 검증")
    args = ap.parse_args(argv)
    table = {"tick": cmd_tick, "execute": cmd_execute, "audit": cmd_audit, "reset": cmd_reset,
             "bootstrap-verifier": cmd_bootstrap_verifier, "status": cmd_status,
             "self-test": cmd_self_test}
    if args.cmd not in table:
        ap.print_help()
        return EXIT_ERR
    try:
        return table[args.cmd](args)
    except Exception as e:  # noqa: BLE001 — 최상위는 항상 코드로 답한다
        print("error: %s" % e, file=sys.stderr)
        return EXIT_ERR


if __name__ == "__main__":
    sys.exit(main())
