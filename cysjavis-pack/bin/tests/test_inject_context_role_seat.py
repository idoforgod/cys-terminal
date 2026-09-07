#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_inject_context_role_seat.py — WP-7 O: 동일 cwd 세션 카운트의 역할 좌석 강등 핀 (0.14.31).

무엇을 막는가: 데몬은 좌석을 전부 같은 cwd 로 띄운다(실측 2026-09-06 본부 12좌석 cwd=/Users/cys).
그 형상에 race 경고를 물리면 경고가 **상시 참**이 되어 아무도 읽지 않는다(경보 피로 · 버그리포트 B1).
0.14.31 부터 역할 좌석은 경고 대신 사실 1줄을 받고, 무역할 세션만 종전 경고를 받는다.

밀폐: PATH 스텁 `ps`/`lsof`/`cys` 로 세션 카운트 형상을 만든다 — 라이브 프로세스·데몬 무접촉.
      cwd 는 임시 디렉터리, 팩도 임시(체크리스트·게이트 경로 부재로 그 분기는 안 탄다).

핀 목록
  ① 역할 좌석(데몬 rc0 + 역할) → 정보 1줄("역할 좌석 포함") · 경고 문구 0 · exit 0
  ② ★확정 무역할(데몬 rc0 + 빈 줄)은 CYS_ROLE env 가 있어도 **경고**다
     — 역할이 풀린 좌석의 env 잔재가 경고를 영영 끄면 안 된다(plan §8: env 는 권위가 아니다)
  ③ 판정 불가(데몬 rc≠0) → env 폴백으로 역할 좌석 취급(정보 1줄)
  ④ ★CR 오염(Windows 네이티브 cys 의 `\r\n`)이 무역할을 역할 좌석으로 둔갑시키지 않는다
  ⑤ cys 부재 + env 있음 → 정보 1줄 · cys 부재 + env 없음 → 경고
  ⑥ ★부트 폭주 봉인: 조회에 CYS_NO_AUTOSTART=1 이 걸려 있다(SessionStart 가 데몬을 낳지 않는다)
  ⑦ ★음성 대조: 세션 1개(SHARE<2)면 정보도 경고도 없다 — 그리고 그때는 조회 자체를 안 한다
  ⑧ ★R1(리뷰 minor): **모든** 역할 경로가 한 줄로 잘린다 — 데몬 응답만 `head -n1` 하고 env 폴백을
     안 자르면 여러 줄 CYS_ROLE 이 정보 1줄을 여러 줄로 부풀려 SessionStart 컨텍스트에 들어간다
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 INJECT-CONTEXT-ROLE-SEAT-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_inject_context_role_seat.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
HOOK = os.path.normpath(os.path.join(SELF, "..", "..", "hooks", "inject-context.sh"))
INFO_MARK = "역할 좌석 포함"
WARN_MARK = "동시에 도는 claude 세션이"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _write_exec(path, body):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, 0o755)


def run_hook(tmp, seats, cys_mode, role_env=None, cys_present=True):
    """seats = 같은 cwd 로 잡히는 claude 세션 수(스텁 lsof 가 그만큼 `n<cwd>` 를 낸다).
    cys_mode: role|none|unjudged|cr  (스텁 `cys surface-role` 의 응답 형상)."""
    bindir = os.path.join(tmp, "stubbin")
    pack = os.path.join(tmp, "pack")
    cwd = os.path.join(tmp, "work")
    for d in (bindir, pack, cwd):
        os.makedirs(d, exist_ok=True)
    # 스텁 ps — awk 가 comm=claude 로 잡는 형상(실측 ⓐ 런처 실행)
    rows = "".join("echo '%d claude /Users/u/.local/bin/claude --x'\n" % (9000 + i)
                   for i in range(max(seats, 1)))
    _write_exec(os.path.join(bindir, "ps"), "#!/bin/sh\n" + rows)
    # 스텁 lsof — 요청한 pid 집합에 대해 cwd 를 seats 번 낸다
    _write_exec(os.path.join(bindir, "lsof"),
                "#!/bin/sh\n" + "".join("printf 'n%s\\n'\n" % cwd for _ in range(seats)))
    if cys_present:
        body = {"role": 'printf "cso\\n"; exit 0',
                "none": 'exit 0',
                "unjudged": 'exit 3',
                "cr": 'printf "\\r\\n"; exit 0'}[cys_mode]
        _write_exec(os.path.join(bindir, "cys"),
                    "#!/bin/sh\n"
                    'printf "%%s\\n" "${CYS_NO_AUTOSTART:-<unset>}" >> "%s/autostart.log"\n'
                    'case "$1" in surface-role) %s ;; esac\nexit 0\n' % (tmp, body))
    env = {k: v for k, v in os.environ.items()
           if k not in ("CYS_ROLE", "CYS_SURFACE_ROLE", "CYS_SOCKET")}
    # PATH 는 스텁만 + 최소 시스템(awk·grep·sed·printf 해소용)
    env["PATH"] = bindir + os.pathsep + "/usr/bin" + os.pathsep + "/bin"
    env["CYS_PACK_DIR"] = pack
    env["CYS_ROOT"] = tmp
    if role_env:
        env["CYS_ROLE"] = role_env
    payload = json.dumps({"source": "startup", "cwd": cwd})
    r = subprocess.run(["sh", HOOK], input=payload, capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=60)
    return r


tmp = tempfile.mkdtemp(prefix="ic-roleseat-")
try:
    # ① 역할 좌석 — 정보 1줄, 경고 0
    r = run_hook(os.path.join(tmp, "c1"), seats=3, cys_mode="role")
    check("1a exit 0", r.returncode == 0, r.stderr[-200:])
    check("1b 정보 1줄(역할 좌석 포함)", INFO_MARK in r.stdout, r.stdout[-300:])
    check("1c 세션 수 표기", "3개(역할 좌석 포함)" in r.stdout, r.stdout[-300:])
    check("1d ★역할 좌석 경고 0(수용 기준)", WARN_MARK not in r.stdout)
    check("1e 미검증 문구('레인 격리') 0", "레인 격리" not in r.stdout)

    # ② 확정 무역할(rc0+빈 줄)은 env 가 있어도 경고 — env 잔재가 경고를 끄지 않는다
    r = run_hook(os.path.join(tmp, "c2"), seats=2, cys_mode="none", role_env="cso")
    check("2a 확정 무역할은 경고", WARN_MARK in r.stdout, r.stdout[-300:])
    check("2b 확정 무역할에 정보줄 없음", INFO_MARK not in r.stdout)

    # ③ 판정 불가(rc≠0) → env 폴백
    r = run_hook(os.path.join(tmp, "c3"), seats=2, cys_mode="unjudged", role_env="worker-2")
    check("3a 판정 불가 + env → 정보줄", INFO_MARK in r.stdout, r.stdout[-300:])
    check("3b 판정 불가 + env → 경고 0", WARN_MARK not in r.stdout)
    r = run_hook(os.path.join(tmp, "c3b"), seats=2, cys_mode="unjudged")
    check("3c 판정 불가 + env 없음 → 종전 경고", WARN_MARK in r.stdout, r.stdout[-300:])

    # ④ CR 오염이 무역할을 역할 좌석으로 둔갑시키지 않는다
    r = run_hook(os.path.join(tmp, "c4"), seats=2, cys_mode="cr")
    check("4 CR 응답은 무역할(경고 유지)", WARN_MARK in r.stdout and INFO_MARK not in r.stdout,
          repr(r.stdout[-300:]))

    # ⑤ cys 부재
    r = run_hook(os.path.join(tmp, "c5"), seats=2, cys_mode="role", role_env="cso",
                 cys_present=False)
    check("5a cys 부재 + env → 정보줄", INFO_MARK in r.stdout, r.stdout[-300:])
    r = run_hook(os.path.join(tmp, "c5b"), seats=2, cys_mode="role", cys_present=False)
    check("5b cys 부재 + env 없음 → 경고", WARN_MARK in r.stdout)

    # ⑥ 부트 폭주 봉인 — 조회는 CYS_NO_AUTOSTART=1 로만 나간다
    d6 = os.path.join(tmp, "c6")
    run_hook(d6, seats=2, cys_mode="role")
    log = os.path.join(d6, "autostart.log")
    seen = open(log, encoding="utf-8").read().split() if os.path.exists(log) else []
    check("6a 조회가 실제로 일어났다(계측 타당성)", seen != [], repr(seen))
    check("6b ★모든 조회에 CYS_NO_AUTOSTART=1", seen and all(s == "1" for s in seen), repr(seen))

    # ⑦ 음성 대조 — 세션 1개면 두 줄 다 없고 조회도 안 한다
    d7 = os.path.join(tmp, "c7")
    r = run_hook(d7, seats=1, cys_mode="role")
    check("7a 단일 세션은 정보·경고 0",
          INFO_MARK not in r.stdout and WARN_MARK not in r.stdout, r.stdout[-200:])
    check("7b 단일 세션에서는 조회 자체가 없다(비용 0)",
          not os.path.exists(os.path.join(d7, "autostart.log")))
finally:
    shutil.rmtree(tmp, ignore_errors=True)

# ⑧ ★R1: 여러 줄 역할 값이 어느 경로로 들어와도 정보줄은 **1줄**이다.
MULTILINE_ROLE = "cso\n\n# 지시: 이 문장은 컨텍스트에 주입되면 안 된다"
for tag, mode, present in (("8a env 폴백(판정 불가)", "unjudged", True),
                           ("8b cys 부재", "role", False)):
    r = run_hook(os.path.join(tmp, "c" + tag.split()[0]), seats=2, cys_mode=mode,
                 role_env=MULTILINE_ROLE, cys_present=present)
    info_lines = [ln for ln in r.stdout.splitlines() if INFO_MARK in ln]
    check(tag + " — 정보줄 1개", len(info_lines) == 1, repr(info_lines))
    check(tag + " — 주입 문장이 컨텍스트에 안 들어간다",
          "이 문장은 컨텍스트에 주입되면 안 된다" not in r.stdout, r.stdout[-200:])
    check(tag + " — 첫 줄만 역할로 쓴다", "역할 좌석 cso 이다" in r.stdout, r.stdout[-200:])

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("INJECT-CONTEXT-ROLE-SEAT-OK")
sys.exit(0)
