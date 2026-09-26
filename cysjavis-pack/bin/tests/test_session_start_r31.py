#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_session_start_r31.py — SessionStart 훅의 사용량 등록 블록 계약 (0.14.42 R3-1(a)).

무엇을 막는가:
  session-start.sh 는 훅 입력 한 줄에서 `<source>|<transcript_path>` 를 뽑아(62·70·71행) 두 갈래로 부른다.
  · clear → `cys usage-register --transcript <경로> --source clear`(10s 상한 · CYS_NO_AUTOSTART=1 ·
    rc 2 일 때만 플래그 없이 1회 재호출). 데몬은 이 신호로 resume 핀을 새 세션으로 바꾼다(R3-1(b)).
  · 그 밖 source → 종전 호출 `cys usage-register --transcript <경로>` 그대로(T5 좌석↔세션 1:1 등록).
  파싱 줄이 조용히 망가지면(예: `TP=${SS#*|}` → `TP=${SS%%|*}`) 두 갈래가 전 좌석에서 함께 꺼진다 — 데몬은
  `not absolute` 로 거부하고 출력은 /dev/null 로 사라져 아무도 모른다. 종전 커밋 핀(cys.rs
  r3_1_usage_register_source_flag_and_hook_fallback_contract)은 호출 문자열의 존재·순서만 보므로 이 변이에 초록이다.
  이 검체는 **훅을 실제로 돌려** 스텁 cys 가 받은 인자 전체를 단언한다.

밀폐: 임시 팩(지침 4종 + 레포 hooks/ 사본) · 격리 HOME·CYS_STATE_DIR · 스텁 cys(호출 기록) · 라이브 데몬 무접촉.
셸: sh 와, 있으면 dash·bash 각각(행 이름에 셸 표기). 데몬 무응답 행(10s 상한)은 첫 셸에서만 잰다(벽시계 비용).
Windows: CI 에 등재하지 않는다(mac·ubuntu 3완전 레인 · test_hook_r34 와 같은 5곳).
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 SESSION-START-R31-OK.
"""
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
SELF = os.path.dirname(os.path.abspath(__file__))
HOOKS = os.path.join(os.path.dirname(os.path.dirname(SELF)), "hooks")
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


# 스텁 cys — usage-register 만 기록한다(인자 전체 + 그 순간의 CYS_NO_AUTOSTART). 모드별 종료 코드:
#   ok     : 0
#   old    : --source 를 모르는 옛 CLI(clap 사용 오류 rc 2) · 플래그 없는 호출은 0
#   fail1  : 요청 실패(데몬 오류·거부·부재) rc 1
#   refused: autostart 거절 rc 2(플래그와 무관)
#   hang   : --source 호출은 60s 무응답(데몬 행) · 플래그 없는 호출은 0
# 그 밖 하위 명령(surface-role·reclaim-role·--version·claim-role …)은 훅 뒤 단계가 돌 수 있게만 답한다.
STUB = r'''#!/bin/sh
if [ "$1" = usage-register ]; then
  printf '%s|NA=%s\n' "$*" "${CYS_NO_AUTOSTART:-}" >> "$REGLOG"
  case "$STUB_MODE" in
    ok) exit 0 ;;
    old) case " $* " in *" --source "*) echo "error: unexpected argument '--source' found" >&2; exit 2 ;; esac; exit 0 ;;
    fail1) exit 1 ;;
    refused) exit 2 ;;
    hang) case " $* " in *" --source "*) exec /bin/sleep 60 ;; esac; exit 0 ;;
  esac
  exit 0
fi
case "$1" in
  surface-role) exit 0 ;;
  reclaim-role) printf 'role=\nreason=no_candidate\nenv_role=unknown\ndetail=\n'; exit 0 ;;
  --version) echo "cys 0.0.0-stub"; exit 0 ;;
esac
exit 0
'''

P = "/Users/x/.claude/projects/-p/22222222-2222-4222-8222-222222222222.jsonl"
P_PIPE = "/Users/x/a|b/.claude/projects/-p/33333333-3333-4333-8333-333333333333.jsonl"


def lab(root, name):
    d = os.path.join(root, name)
    pack = os.path.join(d, "pack")
    os.makedirs(os.path.join(pack, "directives"))
    for r in ("MASTER", "WORKER", "CSO", "REVIEWER"):
        with io.open(os.path.join(pack, "directives", r + "_DIRECTIVE.md"), "w", encoding="utf-8") as f:
            f.write("D-" + r + "\n")
    # 훅은 팩 사본에서 돈다 — 레인 가드(훅 쪽 팩 == CYS_PACK_DIR)가 위임 없이 통과하는 정상 배치와 같다.
    shutil.copytree(HOOKS, os.path.join(pack, "hooks"))
    binp = os.path.join(d, "bin")
    os.makedirs(binp)
    with io.open(os.path.join(binp, "cys"), "w", encoding="utf-8", newline="\n") as f:
        f.write(STUB)
    os.chmod(os.path.join(binp, "cys"), 0o755)
    for sub in ("home", "state"):
        os.makedirs(os.path.join(d, sub))
    env = dict(os.environ)
    for k in list(env):
        if k.startswith(("CYS_", "AITERM_", "CLAUDE")):
            env.pop(k, None)
    env.update({"CYS_PACK_DIR": pack, "CYS_SURFACE_ID": "3", "HOME": os.path.join(d, "home"),
                "CYS_STATE_DIR": os.path.join(d, "state"), "REGLOG": os.path.join(d, "reg.log"),
                "PATH": binp + os.pathsep + env.get("PATH", "")})
    return d, os.path.join(pack, "hooks", "session-start.sh"), env


def run(root, tag, sh, payload, mode="ok", crlf=False):
    d, hook, env = lab(root, tag)
    env["STUB_MODE"] = mode
    if crlf:
        # ★(WIN-1) 네이티브 Windows python 의 파이프 CRLF 모사 — 출력 줄 끝마다 \r 을 붙이는 래퍼를 CYS_PY 로(프리루드가 존중한다).
        w = os.path.join(d, "bin", "pywin")
        with io.open(w, "w", encoding="utf-8", newline="\n") as f:
            f.write('#!/bin/sh\n"%s" "$@" | sed "s/$/$(printf \'\\r\')/"\n' % sys.executable)
        os.chmod(w, 0o755)
        env["CYS_PY"] = w
    t0 = time.monotonic()
    r = subprocess.run([sh, hook], input=payload, capture_output=True, text=True, env=env, timeout=90)
    dt = time.monotonic() - t0
    try:
        # newline="" — CR 이 인자에 섞였으면 그대로 보이게(splitlines 는 \r 을 줄 경계로 먹는다).
        with io.open(os.path.join(d, "reg.log"), encoding="utf-8", newline="") as f:
            calls = [c for c in f.read().split("\n") if c]
    except OSError:
        calls = []
    return r.returncode, calls, dt


def line(obj):
    return json.dumps(obj) + "\n"


def CL(p=P):
    return "usage-register --transcript %s --source clear|NA=1" % p


def PL1(p=P):
    return "usage-register --transcript %s|NA=1" % p


def LEG(p=P):
    return "usage-register --transcript %s|NA=" % p


shells = [("sh", "sh")]
for alt in ("dash", "bash"):
    if shutil.which(alt):
        shells.append((alt, shutil.which(alt)))

root = tempfile.mkdtemp(prefix="ss-r31-")
try:
    for i, (sname, sh) in enumerate(shells):
        t = "[%s] " % sname
        k = [0]

        def R(payload, mode="ok", crlf=False):
            k[0] += 1
            return run(root, "%s-%d" % (sname, k[0]), sh, payload, mode, crlf)

        rc, calls, _ = R(line({"source": "clear", "transcript_path": P}))
        check(t + "SS-1 clear → `--source clear` 정확히 1회 · 경로 전문 · CYS_NO_AUTOSTART=1",
              rc == 0 and calls == [CL()], repr(calls))
        for src in ("startup", "resume", "compact", None, "weird|x"):
            o = {"transcript_path": P}
            if src is not None:
                o["source"] = src
            rc, calls, _ = R(line(o))
            check(t + "SS-2 source=%s → 종전 호출 그대로(플래그·autostart 차단 없음 · 경로 전문)" % src,
                  rc == 0 and calls == [LEG()], repr(calls))
        rc, calls, _ = R(line({"source": "clear", "transcript_path": P}), "old")
        check(t + "SS-3 옛 cys(--source 거부 rc 2) → 플래그 없이 정확히 1회 재호출(둘 다 NO_AUTOSTART)",
              rc == 0 and calls == [CL(), PL1()], repr(calls))
        rc, calls, _ = R(line({"source": "clear", "transcript_path": P}), "fail1")
        check(t + "SS-4 요청 실패(rc 1) → 재호출 없음(정확히 1회)", rc == 0 and calls == [CL()], repr(calls))
        rc, calls, _ = R(line({"source": "clear", "transcript_path": P}), "refused")
        check(t + "SS-5 autostart 거절(rc 2) → 즉시 재호출 1회", rc == 0 and calls == [CL(), PL1()], repr(calls))
        if i == 0:
            rc, calls, dt = R(line({"source": "clear", "transcript_path": P}), "hang")
            check(t + "SS-6 데몬 무응답 → 10s 상한 · 재호출 없음 · 훅 exit 0",
                  rc == 0 and calls == [CL()] and dt < 25, "calls=%r dt=%.1fs" % (calls, dt))
        for src in ("clear", "startup"):
            rc, calls, _ = R(line({"source": src, "transcript_path": None}))
            check(t + "SS-7 %s + transcript_path null → 호출 0(종전은 문자열 'None' 으로 불렀다)" % src,
                  rc == 0 and calls == [], repr(calls))
        rc, calls, _ = R(line({"source": "clear"}))
        check(t + "SS-7b transcript_path 부재 → 호출 0", rc == 0 and calls == [], repr(calls))
        rc, calls, _ = R("not json\n")
        check(t + "SS-8 깨진 입력 → 호출 0 · exit 0", rc == 0 and calls == [], repr(calls))
        rc, calls, _ = R(line({"source": "clear", "transcript_path": P_PIPE}))
        check(t + "SS-9 경로에 `|` 가 있어도 첫 `|` 뒤 전부가 경로(clear)", rc == 0 and calls == [CL(P_PIPE)], repr(calls))
        rc, calls, _ = R(line({"source": "startup", "transcript_path": P_PIPE}))
        check(t + "SS-9b 경로에 `|` 가 있어도 경로 보존(종전 호출)", rc == 0 and calls == [LEG(P_PIPE)], repr(calls))
        # ★(WIN-1) 네이티브 Windows python 의 파이프 CRLF — 경로 꼬리에 CR 이 붙지 않고 null 경로는 여전히 호출 0.
        #   RED(HEAD 1b614e47): `--transcript <경로>\r --source clear` · null 이면 `--transcript \r --source clear`.
        rc, calls, _ = R(line({"source": "clear", "transcript_path": P}), crlf=True)
        check(t + "SS-10 CRLF python + clear → 경로에 CR 없음(정확히 1회)", rc == 0 and calls == [CL()], repr(calls))
        rc, calls, _ = R(line({"source": "clear", "transcript_path": None}), crlf=True)
        check(t + "SS-10b CRLF python + null 경로 → 호출 0", rc == 0 and calls == [], repr(calls))
        rc, calls, _ = R(line({"source": "startup", "transcript_path": P}), crlf=True)
        check(t + "SS-10c CRLF python + startup → 종전 호출 · CR 없음", rc == 0 and calls == [LEG()], repr(calls))
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("SESSION-START-R31-OK")
sys.exit(0)
