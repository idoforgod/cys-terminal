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

    # ─── [C2 절 경계] ───

    # ─── [C3 절 경계] ───

    # ─── [C4 절 경계] ───

finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("HOOK-R34-OK")
sys.exit(0)
