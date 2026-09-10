#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_inject_context_role_seat.py — WP-7 O: 동일 cwd 세션 카운트의 역할 좌석 강등 핀 (0.14.31).

무엇을 막는가: 데몬은 좌석을 전부 같은 cwd 로 띄운다(실측 2026-09-06 본부 12좌석 cwd=오너 홈).
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
해석기: 훅의 shebang(`#!/bin/bash`)을 따라 **bash** 로 부른다 — Ubuntu 의 `/bin/sh`(dash)는 훅 :34 의
       here-string 을 받지 못한다(실측 rc 2). POSIX 전용 검체이므로 Windows 레그에는 등재하지 않는다.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
HOOK = os.path.normpath(os.path.join(SELF, "..", "..", "hooks", "inject-context.sh"))
# ★R2(수렴 · 리뷰 minor "Ubuntu 레인에서 이 검체가 적색"): 훅은 `#!/bin/bash` 이고 :34 에
#   **bash 전용 here-string**(`<<< "$_PARSED"`)을 쓴다. macOS 의 `sh` 는 posix 모드 bash 라 그것을
#   받지만 Ubuntu 의 `/bin/sh` 는 dash 이고 받지 못한다 — 실측: `dash -n hooks/inject-context.sh`
#   → rc 2 `Syntax error: redirection unexpected`. 이 검체가 등재된 두 레인(release
#   `pack-artifacts` · pack-release `pack-only`)은 **ubuntu-latest** 라, `sh` 로 부르면 훅의 결함이
#   아니라 **호출 규약의 오류**로 레인이 적색이 되고 서명 전에 팩 발행이 멈춘다.
#   저장소 규약과도 어긋났다 — `run_bootstrap_health.py` 는 같은 훅을 `[BASH, hook]` 으로 부른다.
#   해석기는 훅의 shebang 이 정하고, 이 검체는 그 shebang 을 따른다.
BASH = shutil.which("bash") or "/bin/bash"
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
    rows = "".join("echo '%d claude /Users/user/.local/bin/claude --x'\n" % (9000 + i)
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
           if k not in ("CYS_ROLE", "CYS_SURFACE_ROLE", "CYS_SOCKET",
                        "CYS_GATE_LANE_SOCKET", "CYS_SOUL")}
    # PATH 는 스텁만 + 최소 시스템(awk·grep·sed·printf 해소용)
    env["PATH"] = bindir + os.pathsep + "/usr/bin" + os.pathsep + "/bin"
    env["CYS_PACK_DIR"] = pack
    env["CYS_ROOT"] = tmp
    # ★R3(codex · CI 3레인 등재 전제): **홈 무접촉**. 빈 임시 팩에는 soul 이 없어서 훅의 soul 해소가
    #   `$HOME/.claude/soul.md` → `$HOME/.cys/pack/soul.md` 로 폴백한다 — 러너/오너 홈의 내용이
    #   판정에 섞이면 초록이 근거가 되지 못한다. `HOME` 을 임시로 고정하고 `CYS_SOUL` 상속도 끊는다.
    env["HOME"] = tmp
    # ★성찰 R4 N5 이후: 훅은 역할 해소를 정본 `_lib.sh:cys_resolve_role` 에 위임한다. 정본의
    #   **신원 전제** — 숫자 surface id 가 없으면 데몬에게 '나' 를 묻지 않는다(주소가 없다는 사실이
    #   '역할 없음' 판정으로 승격되면 정상 위임 경로가 죽는다) — 때문에 실좌석이 언제나 갖는 이
    #   변수를 픽스처도 갖춰야 한다(데몬이 좌석에 주입한다). 핀의 **단언은 하나도 바뀌지 않았다**.
    env["CYS_SURFACE_ID"] = "7"
    # 정본은 60s 디스크 캐시·30s 실패 백오프를 `$TMPDIR` 아래 uid 전용 디렉터리에 둔다 —
    # 케이스마다 격리하지 않으면 앞 케이스의 캐시가 뒤 케이스의 판정을 정한다.
    env["TMPDIR"] = tmp
    if role_env:
        env["CYS_ROLE"] = role_env
    payload = json.dumps({"source": "startup", "cwd": cwd})
    r = subprocess.run([BASH, HOOK], input=payload, capture_output=True, text=True,
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

    # ⑧ ★R1: 여러 줄 역할 값이 어느 경로로 들어와도 정보줄은 **1줄**이다.
    #    ★R2(리뷰 minor): 이 블록은 종전에 `finally: rmtree(tmp)` **뒤**에 있었고,
    #    `run_hook` 이 `makedirs(exist_ok=True)` 로 지워진 tmp 를 되살려 매 실행 $TMPDIR 에
    #    `ic-roleseat-*/{c8a,c8b}` 가 남았다(실측: 실행 전 9개 → 후 10개). CI 3레인에 등재되면
    #    러너마다 누적된다 — try 안으로 옮겨 같은 finally 가 치우게 한다.
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
finally:
    shutil.rmtree(tmp, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("INJECT-CONTEXT-ROLE-SEAT-OK")
sys.exit(0)
