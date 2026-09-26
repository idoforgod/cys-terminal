#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_hook_r34.py — 프롬프트·도구 훅 지연 계약 (0.14.42 R3-4).

절(section)은 커밋 단위다 — 각 절은 자기 커밋의 코드만 판정하고, 그 커밋을 되돌리면 절도 함께 내려간다
(`# ─── [Cn 절 경계] ───` 앵커 사이 · 되돌림 순서 제약은 C2 → C1 하나뿐: C2 가 C1 이 만든 갈래를 고친다).
무엇을 막는지는 각 절 머리 주석이 적는다.
밀폐: 임시 디렉터리 · 스텁 cys · 호출 기록 래퍼(sleep/ls/tail/sed/head/cat) · 라이브 데몬 무접촉.
시계: 판정 행은 호출 **순서·횟수**(구조)로 잰다. 벽시계는 시한 행에만 쓰고, 그 lab 들은 래퍼를 끼우지 않으며
  (exec 1회 추가가 부하에서 틱을 늘린다) 느린 자식을 시한의 4배로 두어 여유를 넓게 잡는다.
Windows: 실 Git Bash(cygpath 실재)에서는 맥·리눅스 갈래 행을 건너뛰고 WIN·(msys) 행만 돈다. 이 검체는 현재
  mac·ubuntu 레인에만 등재돼 있다 — (msys) 행은 가짜 cygpath 모사이지 플랫폼 증명이 아니다.
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 HOOK-R34-OK.
"""
import hashlib
import io
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

SELF = os.path.dirname(os.path.abspath(__file__))
HOOKS = os.path.join(os.path.dirname(os.path.dirname(SELF)), "hooks")
LAUNCHER = os.path.join(HOOKS, "role-bootstrap.sh")
CYSHOOK = os.path.join(HOOKS, "cys-hook.sh")
REAL_MSYS = shutil.which("cygpath") is not None
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def w(path, body, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with io.open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, mode)


def rd(path):
    try:
        with io.open(path, encoding="utf-8", newline="") as f:
            return f.read()
    except OSError:
        return ""


STUB_BODY = '#!/bin/sh\necho BODY-RAN >> "$MARK"\n[ -n "${1:-}" ] && rm -f "$1"\nexit 0\n'

# 가짜 cys — --help 는 STUB_HELP_INPUT=1 일 때만 --input 을 광고 · --input 은 STUB_PRE(사전 동작) 후 STUB_RC.
#   STUB_PRE: emptyrc = 빈 RC 를 먼저 만들고 0.4s(런처 자식의 '열었으나 아직 안 씀' 창을 늘린 모형)
#             slow    = STUB_SLOW 초 대기
#             diequiet = 부모(런처의 자식 서브셸)를 죽인다 — rc 를 못 남긴 자식
#             dieempty = 빈 RC 를 만들고 부모를 죽인다 — 빈 rc 를 남긴 자식
#   usage-event-stdin: STUB_USAGE=fail 이면 stdin 을 **읽지 않고** rc 1(기동 중 사망 모형)
STUB_CYS = r'''#!/bin/sh
printf '%s\n' "cys $*" >> "$CYSLOG"
if [ "$1" = hook ] && [ "$3" = --help ]; then
  if [ "${STUB_HELP_INPUT:-1}" = 1 ]; then printf 'Options:\n      --input <FILE>\n'; else printf 'Options:\n  -h\n'; fi
  exit 0
fi
if [ "$1" = hook ] && [ "$3" = --input ]; then
  if [ "${STUB_HELP_INPUT:-1}" != 1 ]; then echo "error: unexpected argument '--input' found" >&2; exit 2; fi
  case "${STUB_PRE:-}" in
    emptyrc) : > "$4.rc"; /bin/sleep 0.4 ;;
    slow) /bin/sleep "${STUB_SLOW:-4}" ;;
    diequiet) kill -9 "$PPID" ;;
    dieempty) : > "$4.rc"; kill -9 "$PPID" ;;
  esac
  exit "${STUB_RC:-3}"
fi
if [ "$1" = usage-event-stdin ]; then
  if [ "${STUB_USAGE:-}" = fail ]; then echo USAGE-FAIL >> "$CYSLOG"; exit 1; fi
  /bin/cat > "$CYSLOG.stdin"; echo USAGE-EVT >> "$CYSLOG"; exit 0
fi
exit 0
'''

# 가짜 cygpath(Windows 모사) — -u/-w 는 경로를 그대로 돌려준다. 런처·cys-hook 의 플랫폼 판별 술어(`command -v cygpath`)만 켠다.
FAKE_CYGPATH = '#!/bin/sh\nwhile [ "$#" -gt 1 ]; do shift; done\nprintf \'%s\\n\' "$1"\n'


def _posix(p):
    """Windows(Git Bash) 에서 래퍼가 exec 할 실물 경로를 POSIX 표기로 — 공백·역슬래시가 셸에서 깨지지 않게."""
    if REAL_MSYS:
        try:
            out = subprocess.run(["cygpath", "-u", p], capture_output=True, text=True, timeout=10).stdout.strip()
            return out or p
        except (OSError, subprocess.SubprocessError):
            return p
    return p


# 호출 기록형 래퍼(실물로 exec) — 인자만 남긴다. 실물 경로는 셸 인용(공백·역슬래시 안전).
def logging_wrapper(name):
    real = _posix(shutil.which(name) or "/bin/" + name)
    return '#!/bin/sh\nprintf "%%s\\n" "%s $*" >> "$TOOLLOG"\nexec %s "$@"\n' % (name, shlex.quote(real))


def lab(root, name, wrap=("sleep",), msys=False):
    d = os.path.join(root, name)
    hooks = os.path.join(d, "hooks")
    binp = os.path.join(d, "bin")
    state = os.path.join(d, "state")
    os.makedirs(hooks)
    os.makedirs(binp)
    shutil.copy(LAUNCHER, os.path.join(hooks, "role-bootstrap.sh"))
    shutil.copy(CYSHOOK, os.path.join(hooks, "cys-hook.sh"))
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), STUB_CYS, 0o755)
    for t in wrap:
        w(os.path.join(binp, t), logging_wrapper(t), 0o755)
    if msys and not REAL_MSYS:
        w(os.path.join(binp, "cygpath"), FAKE_CYGPATH, 0o755)
    env = dict(os.environ)
    for k in ("AITERM_SURFACE_ID", "CYS_MISSION", "CYS_SOCKET", "CYS_LOCAL_DIR", "CYS_HOOK_INPUT_DEADLINE_S"):
        env.pop(k, None)
    env.update({"CYS_SURFACE_ID": "7", "CYS_STATE_DIR": state, "MARK": os.path.join(d, "mark"),
                "CYS_PACK_DIR": d, "CYSLOG": os.path.join(d, "cys.log"), "TOOLLOG": os.path.join(d, "tool.log"),
                "HOME": os.path.join(d, "home"), "PATH": binp + os.pathsep + env.get("PATH", "")})
    return d, hooks, binp, state, env


def run(hooks, env, script="role-bootstrap.sh", payload='{"prompt":"hello"}', **extra):
    e = dict(env)
    e.update({k: str(v) for k, v in extra.items()})
    t0 = time.monotonic()
    r = subprocess.run(["sh", os.path.join(hooks, script)], input=payload, capture_output=True, text=True,
                       timeout=90, env=e)
    return r, time.monotonic() - t0


def sleeps(d):
    return [l.split(" ", 1)[1] for l in rd(os.path.join(d, "tool.log")).splitlines() if l.startswith("sleep ")]


def body_count(d):
    return rd(os.path.join(d, "mark")).count("BODY-RAN")


root = tempfile.mkdtemp()
try:
    # ─── [C0 실험실 타당성] ───
    # 실험실이 런처를 스텁 본체까지 끝까지 돌리는가(구 CLI 모사 → 프로브 음성 → 본체 1회) — 아래 절들의 전제.
    d, hooks, binp, state, env = lab(root, "h0", wrap=())
    r, el = run(hooks, env, STUB_HELP_INPUT=0)
    check("HARNESS-0 실험실 런처 → 본체 1회 · exit 0(계측 타당성)", body_count(d) == 1 and r.returncode == 0,
          repr((body_count(d), r.returncode, r.stderr[-200:])))

    # ─── [C1 절 경계] ───
    # ═════════ [C1] ⑥ 완료 감지 — 무엇을 막는가 ═════════
    #   ①100ms 계단 — 런처가 자식 완료를 0.1s 단위로만 보아 즉시 끝나는 자식에도 100ms+ 를 쓴다(S36 400/400).
    #   ②완료 오판 — RCF 의 '존재'를 완료로 읽으면 빈 값(자식이 연 순간)이나 남의 값(pid 재사용 · 늦은 기록자)을 읽는다.
    #     완료 신호는 자식 종료다. 시한(T2-2)은 늦은 기록자가 있어도 선다(무상한 wait 금지).
    #   ③Windows 갈래는 종전 ⑥ 바이트 그대로다(WIN-0 핀 · 가짜 cygpath 모사 WIN-1·2).
    def run_pre(hooks, env, pre_sh, **extra):
        """`sh -c '<pre_sh>; exec sh 런처'` — exec 는 pid 를 유지하므로 pre_sh 의 `$$` = 런처의 `$$`(입력·RCF 이름의 pid)."""
        e = dict(env)
        e.update({k: str(v) for k, v in extra.items()})
        t0 = time.monotonic()
        r = subprocess.run(["sh", "-c", pre_sh + '\nexec sh "$0"', os.path.join(hooks, "role-bootstrap.sh")],
                           input='{"prompt":"hello"}', capture_output=True, text=True, timeout=90, env=e)
        return r, time.monotonic() - t0

    NOTE = "백그라운드로 계속"
    SCHED = ["0.01", "0.02", "0.03", "0.04"]       # 잘게 보는 틱(누계 0.10s = 종전 한 칸) — 그 뒤는 종전 0.1s

    def is_sched_prefix(seq):
        exp = SCHED + ["0.1"] * max(0, len(seq) - len(SCHED))
        return seq == exp[:len(seq)]

    # 늦은 기록자 — pid 가 재사용된 이전 런처(같은 좌석)의 T2-2 자식이 **이번 런처의 선제거 뒤에** 같은 이름의 RCF 에 6 을 쓴다.
    #   이번 입력 파일이 생긴 뒤(= ⑤ 통과 · ⑥ 선제거 직전) delay 초 뒤에 쓴다 · 대기 상한 3s(입력이 끝내 안 보이면 포기).
    LATE_WRITER = ('( n=0; while [ ! -f "$CYS_STATE_DIR/hook-input-7-$$.json" ] && [ "$n" -lt 300 ]; do /bin/sleep 0.01; '
                   'n=$((n+1)); done; /bin/sleep %s; printf 6 > "$CYS_STATE_DIR/hook-input-7-$$.json.rc" ) '
                   '>/dev/null 2>&1 </dev/null &')

    # 6371f8bd ⑥ if-블록(36행 · `if command -v cys` ~ `fi`) sha256 — 2026-09-25 측정(기준 커밋 고정 · 시각 무관 불변값).
    WIN_BLOCK_SHA = "76be13103f9067909e4ddfa64f9cc8de70eceac9858d51c9c1b9ddd100616727"

    # ───────── WIN-0: Windows 갈래 = 종전 ⑥ 바이트 동일(정적 핀) ─────────
    src = rd(LAUNCHER)
    a_tag, b_tag = "# ── WIN-BEGIN(종전 ⑥ · 무변경) ──\n", "# ── WIN-END ──"
    blk = src[src.index(a_tag) + len(a_tag):src.index(b_tag)] if (a_tag in src and b_tag in src) else ""
    check("WIN-0 Windows 갈래(cygpath 실재)는 6371f8bd ⑥ 과 바이트 동일",
          hashlib.sha256(blk.encode("utf-8")).hexdigest() == WIN_BLOCK_SHA and src.count(a_tag) == 1,
          "len=%d" % len(blk))

    if not REAL_MSYS:
        # ───────── LAT-1: 즉시 끝나는 자식에서 0.1s 계단을 밟지 않는다(구조 판정 · 시계 무관) ─────────
        d, hooks, binp, state, env = lab(root, "lat1")
        r, el = run(hooks, env, STUB_RC=3)
        seq = sleeps(d)
        check("LAT-1a 첫 완료 확인 대기가 0.01s 다(종전: 무조건 0.1s)", bool(seq) and seq[0] == "0.01", repr(seq[:4]))
        check("LAT-1b 대기 순서가 0.01→0.02→0.03→0.04→0.1… 의 앞부분이다", is_sched_prefix(seq), repr(seq[:8]))
        check("LAT-1c rc3 → 본체 미실행 · exit 0 · stdout 0", r.returncode == 0 and body_count(d) == 0 and r.stdout == "",
              repr((r.returncode, body_count(d), r.stdout[:80])))
        ws = sorted(run(hooks, env, STUB_RC=3)[1] for _ in range(5))
        # 정보 행(판정 아님) — 벽시계는 CI 부하에 흔들린다. 판정은 위 구조 행이 진다.
        print("INFO LAT-2 벽시계 중앙값(5회) %.1fms — 종전 구조 하한 ≥ 100ms" % (ws[2] * 1000))

        # ───────── LAT-3: 느린 자식 — 잘게 보는 틱은 최대 4회(동시 방송 스폰 상한 · 구조 판정) ─────────
        d, hooks, binp, state, env = lab(root, "lat3")
        r, el = run(hooks, env, STUB_PRE="slow", STUB_SLOW="0.6", STUB_RC=3)
        seq = sleeps(d)
        fine = [x for x in seq if x != "0.1"]
        check("LAT-3a 느린 자식(0.6s) — 대기 순서가 계획표의 앞부분 · 잘게 보는 틱 ≤ 4",
              is_sched_prefix(seq) and len(fine) <= 4, repr(seq[:10]))
        check("LAT-3b 0.1s 로 넘어갔다면 그 앞은 정확히 0.01·0.02·0.03·0.04(총 4회 · 누계 0.10s)",
              "0.1" not in seq or seq[:4] == SCHED, repr(seq[:6]))
        check("LAT-3c rc3 → 본체 0 · exit 0 · stdout 0", r.returncode == 0 and body_count(d) == 0 and r.stdout == "",
              repr((r.returncode, body_count(d), r.stdout[:80])))

        # ───────── RCF-1: 빈 RC 창에서 본체를 돌리지 않는다(rc6 이중 처리 봉인) ─────────
        d, hooks, binp, state, env = lab(root, "rcf1")
        r, el = run(hooks, env, STUB_PRE="emptyrc", STUB_RC=6)
        check("RCF-1 빈 RC 창에서도 rc6 이면 본체 0회(이중 처리 없음)", body_count(d) == 0 and r.returncode == 0,
              repr((body_count(d), r.returncode, r.stdout[:120])))
        check("RCF-1b 입력 파일·RC 파일 회수", not [n for n in os.listdir(state) if n.startswith("hook-input-")],
              str(os.listdir(state)))

        # ───────── RCF-2: 같은 좌석·같은 pid 의 **낡은 RCF**(이미 끝난 T2-2 잔재)를 읽지 않는다 ─────────
        d, hooks, binp, state, env = lab(root, "rcf2", wrap=())
        os.makedirs(state, exist_ok=True)
        r, el = run_pre(hooks, env, 'printf 6 > "$CYS_STATE_DIR/hook-input-7-$$.json.rc"', STUB_RC=0)
        check("RCF-2 낡은 RCF(6)를 무시하고 이번 자식의 rc0 으로 본체 정확히 1회", body_count(d) == 1 and r.returncode == 0,
              repr((body_count(d), r.returncode, r.stdout[:100])))

        # ───────── RCF-3: 낡은 RCF + 느린 자식 → 시한이 선다 ─────────
        d, hooks, binp, state, env = lab(root, "rcf3", wrap=())
        os.makedirs(state, exist_ok=True)
        r, el = run_pre(hooks, env, 'printf 6 > "$CYS_STATE_DIR/hook-input-7-$$.json.rc"',
                        STUB_RC=6, STUB_PRE="slow", STUB_SLOW=4, CYS_HOOK_INPUT_DEADLINE_S=1)
        check("RCF-3 낡은 RCF + 느린 자식(4s) → 시한(1s)에 지연 고지 · 본체 0 · 무기한 대기 없음",
              NOTE in r.stdout and el < 3.0 and body_count(d) == 0 and r.returncode == 0,
              "%.2fs %r" % (el, r.stdout[:100]))

        # ───────── RCF-4: **늦은 기록자**(선제거 뒤에 남의 6 이 도착) ─────────
        #   a) 이번 자식이 느리다 → 시한에 T2-2 로 닫힌다(무상한 wait 로 막히지 않는다 · 남의 6 을 소비하지 않는다).
        d, hooks, binp, state, env = lab(root, "rcf4a", wrap=())
        r, el = run_pre(hooks, env, LATE_WRITER % "0.2", STUB_RC=6, STUB_PRE="slow", STUB_SLOW=4,
                        CYS_HOOK_INPUT_DEADLINE_S=1)
        check("RCF-4a 늦은 기록자 + 느린 자식(4s) → 시한(1s)에 지연 고지 · 본체 0 · 남의 값 무소비",
              NOTE in r.stdout and el < 3.0 and body_count(d) == 0 and r.returncode == 0,
              "%.2fs %r" % (el, r.stdout[:100]))
        #   b) 이번 자식이 시한 안에 rc0 으로 끝난다 → 이번 값(0)으로 본체 1회(남의 6 으로 프롬프트를 버리지 않는다).
        d, hooks, binp, state, env = lab(root, "rcf4b", wrap=())
        r, el = run_pre(hooks, env, LATE_WRITER % "0.2", STUB_RC=0, STUB_PRE="slow", STUB_SLOW="1.2")
        check("RCF-4b 늦은 기록자 + 이번 자식 rc0(1.2s) → 본체 정확히 1회 · 고지 없음",
              body_count(d) == 1 and NOTE not in r.stdout and r.returncode == 0,
              "%.2fs body=%d %r" % (el, body_count(d), r.stdout[:100]))

        # ───────── RCF-5·6: rc 를 못 남긴 자식 — 종전 거동 보존(파리티) ─────────
        d, hooks, binp, state, env = lab(root, "rcf5", wrap=())
        r, el = run(hooks, env, STUB_PRE="diequiet", STUB_RC=6)
        check("RCF-5 rc 없이 죽은 자식 → 종전처럼 T2-2 고지 · 본체 0 · exit 0",
              NOTE in r.stdout and body_count(d) == 0 and r.returncode == 0,
              "%.2fs body=%d %r" % (el, body_count(d), r.stdout[:100]))
        # RCF-5b: rc 없이 죽은 자식 + 같은 이름의 낡은 RCF(6) — 선제거가 없으면 남의 6 을 이번 결과로 읽는다(프롬프트 무처리).
        d, hooks, binp, state, env = lab(root, "rcf5b", wrap=())
        os.makedirs(state, exist_ok=True)
        r, el = run_pre(hooks, env, 'printf 6 > "$CYS_STATE_DIR/hook-input-7-$$.json.rc"', STUB_PRE="diequiet", STUB_RC=6)
        check("RCF-5b rc 없이 죽은 자식 + 낡은 RCF(6) → 남의 값 무소비 · 종전 '결과 없음' 갈래(T2-2 고지) · 본체 0",
              NOTE in r.stdout and body_count(d) == 0 and r.returncode == 0,
              "%.2fs body=%d %r" % (el, body_count(d), r.stdout[:100]))
        d, hooks, binp, state, env = lab(root, "rcf6", wrap=())
        r, el = run(hooks, env, STUB_PRE="dieempty", STUB_RC=6)
        check("RCF-6 빈 rc 를 남기고 죽은 자식 → 종전처럼 본체 1회 · exit 0",
              body_count(d) == 1 and NOTE not in r.stdout and r.returncode == 0,
              "%.2fs body=%d %r" % (el, body_count(d), r.stdout[:100]))

        # ───────── NZ: **끝난 자식을 스스로 회수하지 않는 셸**(리뷰 F4) ─────────
        #   완료 신호가 `kill -0` 실패라, 셸이 끝난 백그라운드 자식을 회수하지 않으면 좀비에 `kill -0` 이 계속 성공한다
        #   → 매 프롬프트가 시한까지 기다린 뒤 T2-2 로 빠져 rc0(proceed) 본문을 건너뛴다(종전 RCF 존재 판정은 면역).
        #   후보: sleep 을 포크하지 않고(NOFORK) SIGCHLD 로도 회수하지 않는 셸(예: FEATURE_SH_NOFORK busybox ash) —
        #   이 머신에 없어 **모형**으로 잰다: bash + BASH_ENV 로 `kill -0` 을 '회수 전까지 성공(좀비)' 으로, 명시 회수
        #   내장(`jobs`·`wait`)을 '회수' 로 바꾼다. 런처는 RCF 에 값이 찬 뒤 비차단 회수(`jobs`)를 불러야 이 모형을
        #   지난다. `wait` 는 쓰지 않는다 — 늦은 기록자(RCF-4)가 값을 채운 채 이번 자식이 살아 있으면 무상한 대기다.
        bash_bin = shutil.which("bash")
        if bash_bin:
            def run_nz(name, **extra):
                d, hooks, binp, state, env = lab(root, name, wrap=())
                zd = os.path.join(d, "zombie")
                os.makedirs(zd)
                model = os.path.join(d, "no-reap-model.sh")
                w(model, 'kill() {\n'
                         '  if [ "$1" = -0 ]; then\n'
                         '    command kill -0 "$2" 2>/dev/null && return 0\n'
                         '    [ -e "$NZ_DIR/reaped" ] && return 1\n'
                         '    return 0\n'
                         '  fi\n'
                         '  command kill "$@"\n'
                         '}\n'
                         'jobs() { command jobs "$@"; : > "$NZ_DIR/reaped"; }\n'
                         'wait() { command wait "$@"; _nz_r=$?; : > "$NZ_DIR/reaped"; return $_nz_r; }\n')
                e = dict(env, BASH_ENV=model, NZ_DIR=zd)
                e.update({k: str(v) for k, v in extra.items()})
                t0 = time.monotonic()
                r = subprocess.run([bash_bin, os.path.join(hooks, "role-bootstrap.sh")], input='{"prompt":"hello"}',
                                   capture_output=True, text=True, timeout=90, env=e)
                return d, r, time.monotonic() - t0
            d, r, el = run_nz("nz1", STUB_RC=0, CYS_HOOK_INPUT_DEADLINE_S=3)
            check("NZ-1 비회수 셸 모형 · 즉시 끝난 자식 rc0 → 본문 정확히 1회 · 지연 고지 없음 · 시한(3s) 전 종료",
                  body_count(d) == 1 and NOTE not in r.stdout and r.returncode == 0 and el < 2.0,
                  "%.2fs body=%d %r" % (el, body_count(d), r.stdout[:100]))
            d, r, el = run_nz("nz2", STUB_RC=6, CYS_HOOK_INPUT_DEADLINE_S=3)
            check("NZ-2 비회수 셸 모형 · rc6 → 본문 0 · 고지 없음 · 시한 전 종료",
                  body_count(d) == 0 and NOTE not in r.stdout and r.returncode == 0 and el < 2.0,
                  "%.2fs body=%d %r" % (el, body_count(d), r.stdout[:100]))
            d, r, el = run_nz("nz3", STUB_RC=6, STUB_PRE="slow", STUB_SLOW=4, CYS_HOOK_INPUT_DEADLINE_S=1)
            check("NZ-3 비회수 셸 모형 · 느린 자식(4s) → 시한(1s)에 지연 고지 · 본문 0(회수 호출이 시한을 늘리지 않는다)",
                  NOTE in r.stdout and body_count(d) == 0 and el < 3.0 and r.returncode == 0,
                  "%.2fs %r" % (el, r.stdout[:100]))
        else:
            print("SKIP NZ 비회수 셸 모형 — bash 부재(검사 수에 넣지 않음)")

        # ───────── DL-1: 데드라인 초과 = 비동기 계속(T2-2) — 1/100초 회계에서도 시한 유지 ─────────
        d, hooks, binp, state, env = lab(root, "dl1", wrap=())
        r, el = run(hooks, env, STUB_PRE="slow", STUB_SLOW=4, STUB_RC=6, CYS_HOOK_INPUT_DEADLINE_S=1)
        check("DL-1a 시한 1s 초과 → 지연 고지 1줄 · 본체 0회 · exit 0",
              r.returncode == 0 and body_count(d) == 0 and r.stdout.count(NOTE) == 1, repr((r.stdout[:160], body_count(d))))
        check("DL-1b 시한을 지킨다(0.9s ≤ 경과 < 3.0s · 자식 4s)", 0.9 <= el < 3.0, "%.2fs" % el)

    # ───────── WIN-1·2: Windows 모사(가짜 cygpath) — 종전 ⑥ 거동 그대로 ─────────
    d, hooks, binp, state, env = lab(root, "win1", msys=True)
    r, el = run(hooks, env, STUB_RC=3)
    seq = sleeps(d)
    check("WIN-1 (msys) 종전 0.1s 계단 그대로(첫 대기 0.1s · 0.01s 0회) · rc3 → 본체 0",
          bool(seq) and seq[0] == "0.1" and "0.01" not in seq and body_count(d) == 0 and r.returncode == 0,
          repr((seq[:4], body_count(d))))
    d, hooks, binp, state, env = lab(root, "win2", wrap=(), msys=True)
    r, el = run(hooks, env, STUB_PRE="slow", STUB_SLOW=4, STUB_RC=6, CYS_HOOK_INPUT_DEADLINE_S=1)
    check("WIN-2 (msys) 시한 초과 → 종전처럼 지연 고지 · 본체 0",
          NOTE in r.stdout and body_count(d) == 0 and el < 3.0 and r.returncode == 0, "%.2fs %r" % (el, r.stdout[:80]))

    # ─── [C2 절 경계] ───
    # ═════════ [C2] 능력 프로브 양성 캐시 — 양성만 · 경로+mtime 키 · 낡은 양성은 rc2 → 본체(fail-safe) · Windows 무캐시 ═════════
    past = time.time() - 3600
    if not REAL_MSYS:
        d, hooks, binp, state, env = lab(root, "pc1", wrap=())
        os.utime(os.path.join(binp, "cys"), (past, past))
        run(hooks, env, STUB_RC=3)
        run(hooks, env, STUB_RC=3)
        log = rd(os.path.join(d, "cys.log"))
        check("PC-1 같은 cys 로 두 번 → 프로브 1회 · 위임 2회", log.count("--help") == 1 and log.count("--input") == 2,
              repr(log))
        # mtime 무효화 — **제자리 교체**(개발 빌드·cp: 바이너리 mtime 이 캐시보다 새로워짐) 모사.
        #   ★설치기·압축 해제는 빌드 시각 mtime 을 보존하는 일이 흔해 이 무효화가 서지 않는다(새 바이너리가 캐시보다
        #   '오래된' 것이 된다). 그 경우의 안전은 PC-4(낡은 양성 → rc2 → 본체)가 진다 — 이 행은 그 반대 조건만 잰다.
        fut = time.time() + 5
        os.utime(os.path.join(binp, "cys"), (fut, fut))
        run(hooks, env, STUB_RC=3)
        check("PC-2 바이너리 mtime 이 캐시보다 새로우면 재프로브(제자리 교체 모사)",
              rd(os.path.join(d, "cys.log")).count("--help") == 2, repr(rd(os.path.join(d, "cys.log"))))
        # 경로 키: 다른 경로의 cys(더 오래된 mtime)는 캐시를 쓰지 않는다
        bin2 = os.path.join(d, "bin2")
        w(os.path.join(bin2, "cys"), STUB_CYS, 0o755)
        os.utime(os.path.join(bin2, "cys"), (past, past))
        env2 = dict(env, PATH=bin2 + os.pathsep + env["PATH"])
        open(os.path.join(d, "cys.log"), "w").close()
        run(hooks, env2, STUB_RC=3)
        check("PC-3 cys 경로가 바뀌면 재프로브(경로 키)", rd(os.path.join(d, "cys.log")).count("--help") == 1,
              repr(rd(os.path.join(d, "cys.log"))))
        # 낡은 양성(fail-safe) = **앱 업데이트가 mtime 을 보존한 경우 그대로**: 캐시 적중 상태에서 같은 경로의 CLI 가
        #   `--input` 을 모르는 것으로 바뀌었는데 mtime 은 캐시보다 오래됨 → 위임 시도 rc2 → 본체 정확히 1회.
        d, hooks, binp, state, env = lab(root, "pc4", wrap=())
        os.utime(os.path.join(binp, "cys"), (past, past))
        run(hooks, env, STUB_RC=3)          # 양성 캐시 기록
        open(os.path.join(d, "mark"), "w").close()
        open(os.path.join(d, "cys.log"), "w").close()
        r, el = run(hooks, env, STUB_HELP_INPUT=0)
        log = rd(os.path.join(d, "cys.log"))
        check("PC-4 낡은 양성 캐시(mtime 보존 업데이트 모사) → 프로브 생략 · 위임 시도 rc2 → 본체 정확히 1회 · exit 0",
              log.count("--help") == 0 and log.count("--input") == 1 and body_count(d) == 1 and r.returncode == 0,
              repr((log, body_count(d), r.returncode)))
        # 음성은 적지 않는다: 구 CLI 는 매번 프로브 · 위임 0
        d, hooks, binp, state, env = lab(root, "pc5", wrap=())
        os.utime(os.path.join(binp, "cys"), (past, past))   # 캐시가 '더 새것' 이 되는 조건 — 음성을 적었다면 여기서 적중한다
        run(hooks, env, STUB_HELP_INPUT=0)
        run(hooks, env, STUB_HELP_INPUT=0)
        log = rd(os.path.join(d, "cys.log"))
        check("PC-5 구 CLI 는 캐시 없이 매번 프로브 · --input 위임 0 · 본체 2회",
              log.count("--help") == 2 and "--input" not in log and body_count(d) == 2, repr(log))
    # Windows 모사: 캐시를 쓰지도 읽지도 않는다(종전 ⑥ 그대로)
    d, hooks, binp, state, env = lab(root, "pcw", wrap=(), msys=True)
    os.utime(os.path.join(binp, "cys"), (past, past))
    run(hooks, env, STUB_RC=3)
    run(hooks, env, STUB_RC=3)
    log = rd(os.path.join(d, "cys.log"))
    check("PC-W (msys) 매번 프로브 · 캐시 파일 무생성", log.count("--help") == 2
          and not os.path.exists(os.path.join(state, "hook-probe-input.ok")), repr(log))


    # ─── [C3 절 경계] ───
    # ═════════ [C3] GC 게이트 — 20개 이하에서 ls/tail 을 띄우지 않는다(동치) ═════════
    #   잔재 픽스처의 pid 는 pid_max(리눅스 4194304 · 맥 99998) 밖 값 — 살아 있는 pid 를 건너뛰는 GC 와 결정론으로 맞물린다.
    for tag, msys in (("", False), (" (msys 모사 · 게이트는 Windows 갈래에도 적용)", True)):
        d, hooks, binp, state, env = lab(root, "gc2" + ("w" if msys else ""), wrap=("ls", "tail"), msys=msys)
        os.makedirs(state, exist_ok=True)
        for i in range(5):
            w(os.path.join(state, "hook-input-9%02d-49999%02d.json" % (i, i)), "{}")
        run(hooks, env, STUB_RC=3)
        tl = rd(os.path.join(d, "tool.log"))
        check("GC-2 20개 이하 → ls/tail 미기동(동치 · 외부 명령 0)" + tag, "ls " not in tl and "tail " not in tl,
              repr(tl[:200]))
        for i in range(40):
            w(os.path.join(state, "hook-input-8%02d-49998%02d.json" % (i, i)), "{}")
        run(hooks, env, STUB_RC=3)
        left = [n for n in os.listdir(state) if n.startswith("hook-input-") and n.endswith(".json")]
        check("GC-3 20개 초과 → 종전 GC 가 돈다(≤20)" + tag, len(left) <= 20 and "ls " in rd(os.path.join(d, "tool.log")),
              str(len(left)))


    # ─── [C4 절 경계] ───
    # ═════════ [C4] 도구 훅(cys-hook.sh) 빠른 길 — 오버레이 없으면 stdin 을 cys 로 직행 · 실패 시 cat 이 소진 ·
    #   오버레이 있으면 종전 · 항상 exit 0 · stdout 0 ═════════
    def run_stream(hooks, env, data, **extra):
        """입력을 직접 흘려 쓰기 쪽 EPIPE 를 관측한다(subprocess.run 의 communicate 는 EPIPE 를 삼켜 못 본다)."""
        e = dict(env)
        e.update({k: str(v) for k, v in extra.items()})
        p = subprocess.Popen(["sh", os.path.join(hooks, "cys-hook.sh")], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=e)
        epipe = False
        try:
            p.stdin.write(data)
            p.stdin.close()
        except BrokenPipeError:
            epipe = True
            try:
                p.stdin.close()
            except OSError:
                pass
        out = p.stdout.read()
        return p.wait(timeout=90), out, epipe

    ev = json.dumps({"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_input": {"command": "true"},
                     "tool_response": {"exit_code": 0}})
    big = (ev + "\n") * 20000          # 약 2.4MB — 파이프 버퍼(64KB)를 크게 넘는다
    if not REAL_MSYS:
        d, hooks, binp, state, env = lab(root, "ch1", wrap=("sed", "head", "cat"))
        r, el = run(hooks, env, script="cys-hook.sh", payload=ev)
        tl = rd(os.path.join(d, "tool.log"))
        check("CH-0 exit 0 · stdout 0", r.returncode == 0 and r.stdout == "", repr((r.returncode, r.stdout[:80])))
        check("CH-1 사용량 이벤트 정확히 1건 · 본문 바이트 동일",
              rd(os.path.join(d, "cys.log")).count("USAGE-EVT") == 1 and rd(os.path.join(d, "cys.log.stdin")) == ev,
              repr(rd(os.path.join(d, "cys.log.stdin"))[:80]))
        check("CH-2 오버레이 없음 → sed/head/cat 미기동", not any(x in tl for x in ("sed ", "head ", "cat ")), repr(tl))
        # cys 가 입력을 읽지 않고 실패(기동 중 사망 · 구 바이너리 인자 오류) → `|| cat` 이 남은 입력을 소진한다
        d, hooks, binp, state, env = lab(root, "ch6", wrap=())
        rc, out, epipe = run_stream(hooks, env, big.encode("utf-8"), STUB_USAGE="fail")
        check("CH-6 cys 가 읽지 않고 실패 → 남은 입력 소진(쓰기 쪽 EPIPE 없음) · exit 0 · stdout 0",
              not epipe and rc == 0 and out == b"" and "USAGE-FAIL" in rd(os.path.join(d, "cys.log")),
              repr((epipe, rc, out[:60])))
    # 오버레이 있음 → 종전 경로 · 오버레이가 입력을 받는다
    d, hooks, binp, state, env = lab(root, "ch3")
    ov = os.path.join(d, "home", ".cys", "local", "hooks", "PostToolUse.d")
    w(os.path.join(ov, "a.sh"), '#!/bin/sh\ncat > "$MARK.ov"\n', 0o755)
    r, el = run(hooks, env, script="cys-hook.sh", payload=ev)
    check("CH-3 오버레이 있으면 종전 경로 — 오버레이가 입력 전문을 받는다 · 사용량 1건",
          rd(os.path.join(d, "mark.ov")) == ev and rd(os.path.join(d, "cys.log")).count("USAGE-EVT") == 1
          and r.returncode == 0 and r.stdout == "", repr((rd(os.path.join(d, "mark.ov"))[:60], r.returncode)))
    # 다른 이벤트의 오버레이만 있어도 종전 경로(상위집합) — PostToolUse 입력은 Stop.d 로 가지 않는다
    d, hooks, binp, state, env = lab(root, "ch4")
    ov = os.path.join(d, "home", ".cys", "local", "hooks", "Stop.d")
    w(os.path.join(ov, "a.sh"), '#!/bin/sh\ncat > "$MARK.ov"\n', 0o755)
    r, el = run(hooks, env, script="cys-hook.sh", payload=ev)
    check("CH-4 타 이벤트 오버레이는 실행되지 않는다(이벤트 격리 불변)",
          not os.path.exists(os.path.join(d, "mark.ov")) and rd(os.path.join(d, "cys.log")).count("USAGE-EVT") == 1,
          repr(os.listdir(d)))
    # cys 부재 → exit 0 · stdin 소진
    d, hooks, binp, state, env = lab(root, "ch5", wrap=())
    os.remove(os.path.join(binp, "cys"))
    env5 = dict(env, PATH=binp + os.pathsep + "/usr/bin:/bin")
    rc, out, epipe = run_stream(hooks, env5, big.encode("utf-8"))
    check("CH-5 cys 부재 → exit 0 · stdout 0 · 입력 소진(EPIPE 없음)", rc == 0 and out == b"" and not epipe,
          repr((rc, out[:60], epipe)))
    # Windows 모사 → 종전 경로(sed 추출 유지)
    d, hooks, binp, state, env = lab(root, "chw", wrap=("sed",), msys=True)
    r, el = run(hooks, env, script="cys-hook.sh", payload=ev)
    check("CH-W (msys) 종전 경로(sed 추출 유지) · 사용량 1건 · exit 0 · stdout 0",
          "sed " in rd(os.path.join(d, "tool.log")) and rd(os.path.join(d, "cys.log")).count("USAGE-EVT") == 1
          and r.returncode == 0 and r.stdout == "", repr(rd(os.path.join(d, "tool.log"))[:120]))


finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("HOOK-R34-OK")
sys.exit(0)
