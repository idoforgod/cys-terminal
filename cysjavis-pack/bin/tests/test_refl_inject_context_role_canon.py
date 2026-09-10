#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_refl_inject_context_role_canon.py — 성찰 R4 N5: SessionStart 훅의 역할 해소를 정본 하나로.

무엇을 막는가: `hooks/inject-context.sh` 는 `_lib.sh` 를 :11 에서 이미 source 하면서도 역할 해소를
**세 번째 경로**로 다시 구현했고, 그 사본에는 정본(`cys_resolve_role`)의 장치가 전부 빠져 있었다.
  ⓐ 60s 디스크 캐시 없음 · ⓑ 30s 실패 백오프 없음
     → 데몬이 떠 있으나 무응답인 상태에서 좌석 12개가 동시에 SessionStart(부트·`/clear`·`/compact`)
       를 돌면 각 훅이 `cys_timeout_run 2` 만큼 **사용자 프롬프트 앞을 붙잡는다**. 정본이었다면 첫
       실패 뒤 30s 는 조회를 생략한다(봉인표 ④ — 전 페인이 함께 느려지는 방향).
  ⓒ `${CYS_BIN:-cys}` 대신 `cys` 하드코딩 → `bin/cys-dept` 의 레인 지정과 `javis_snapshot._st_env`
     의 봉인을 둘 다 무시한다(다른 레인의 데몬에게 '나는 누구냐' 를 묻는다).
  ⓓ `cys_role_token_ok` 없음 → 문법 밖 문자열이 그대로 SessionStart 컨텍스트에 실린다.

이 검체는 훅을 **실행**해서 그 넷을 잰다(문면 검사 아님). 소비자 계약(정보줄/경고줄)의 회귀는
`test_inject_context_role_seat.py` 가 계속 담당한다 — 여기는 그 위의 네 장치만 못박는다.

밀폐: PATH 스텁 `ps`/`lsof`/`cys` · 임시 HOME·TMPDIR·CYS_PACK_DIR — 라이브 데몬·오너 홈 무접촉.
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 REFL-INJECT-ROLE-CANON-OK.
실행: python3 cysjavis-pack/bin/tests/test_refl_inject_context_role_canon.py
해석기: 훅의 shebang(`#!/bin/bash`)을 따른다 — Ubuntu `/bin/sh`(dash)는 훅의 here-string 을 못 받는다.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
HOOK = os.path.normpath(os.path.join(SELF, "..", "..", "hooks", "inject-context.sh"))
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


class Seat(object):
    """한 좌석(고정 HOME·TMPDIR·PATH) — 같은 좌석에서 훅을 여러 번 돌려 캐시·백오프를 잰다."""

    def __init__(self, tmp, tag, cys_name="cys", cys_body='exit 3', seats=2, env_extra=None):
        self.root = os.path.join(tmp, tag)
        self.bin = os.path.join(self.root, "stubbin")
        self.cwd = os.path.join(self.root, "work")
        self.pack = os.path.join(self.root, "pack")
        self.tmpdir = os.path.join(self.root, "tmp")
        for d in (self.bin, self.cwd, self.pack, self.tmpdir):
            os.makedirs(d, exist_ok=True)
        rows = "".join("echo '%d claude /Users/user/.local/bin/claude --x'\n" % (9000 + i)
                       for i in range(max(seats, 1)))
        _write_exec(os.path.join(self.bin, "ps"), "#!/bin/sh\n" + rows)
        _write_exec(os.path.join(self.bin, "lsof"),
                    "#!/bin/sh\n" + "".join("printf 'n%s\\n'\n" % self.cwd for _ in range(seats)))
        self.log = os.path.join(self.root, "calls.log")
        if cys_name:
            _write_exec(os.path.join(self.bin, cys_name),
                        "#!/bin/sh\n"
                        'printf "%%s %%s\\n" "$0" "$1" >> "%s"\n'
                        'case "$1" in surface-role) %s ;; esac\nexit 0\n' % (self.log, cys_body))
        self.env_extra = env_extra or {}

    def run(self):
        env = {k: v for k, v in os.environ.items()
               if k not in ("CYS_ROLE", "CYS_SURFACE_ROLE", "CYS_SOCKET", "CYS_BIN",
                            "CYS_GATE_LANE_SOCKET", "CYS_SOUL", "CYS_ROLE_UID")}
        env["PATH"] = self.bin + os.pathsep + "/usr/bin" + os.pathsep + "/bin"
        env["CYS_PACK_DIR"] = self.pack
        env["CYS_ROOT"] = self.root
        env["HOME"] = self.root
        env["TMPDIR"] = self.tmpdir
        env["CYS_SURFACE_ID"] = "7"          # 실좌석이 언제나 갖는 것(데몬이 주입) — 정본의 신원 전제
        env.update(self.env_extra)
        payload = json.dumps({"source": "startup", "cwd": self.cwd})
        return subprocess.run([BASH, HOOK], input=payload, capture_output=True, text=True,
                              encoding="utf-8", env=env, timeout=90)

    def calls(self):
        if not os.path.isfile(self.log):
            return []
        return [l for l in open(self.log, encoding="utf-8").read().splitlines() if l.strip()]


tmp = tempfile.mkdtemp(prefix="refl-icrole-")
try:
    # ── ⓑ 실패 백오프: 무응답 데몬 픽스처에서 **두 번째 호출이 조회를 생략한다** ────────────
    s = Seat(tmp, "backoff", cys_body='exit 3')     # rc≠0 = 판정 불가(무응답과 같은 통)
    r1 = s.run()
    n1 = len(s.calls())
    r2 = s.run()
    n2 = len(s.calls())
    check("1a 첫 호출은 데몬에 묻는다(계측 타당성)", n1 == 1, repr(s.calls()))
    check("1b ★두 번째 호출은 조회를 생략한다(30s 실패 백오프)", n2 == 1,
          "1회차 %d · 2회차 누적 %d — %r" % (n1, n2, s.calls()))
    check("1c 두 호출 다 exit 0(훅은 세션을 깨지 않는다)",
          r1.returncode == 0 and r2.returncode == 0, "%r/%r" % (r1.returncode, r2.returncode))

    # ── ⓐ 디스크 캐시: 성공 응답은 두 번째 호출에서 재조회 없이 그대로 쓰인다 ────────────────
    s = Seat(tmp, "cache", cys_body='printf "cso\\n"; exit 0')
    c1 = s.run()
    n1 = len(s.calls())
    c2 = s.run()
    n2 = len(s.calls())
    check("2a 첫 호출이 역할을 얻는다", INFO_MARK in c1.stdout and n1 == 1, repr(s.calls()))
    check("2b ★두 번째 호출은 캐시로 답한다(재조회 0)", n2 == 1, repr(s.calls()))
    check("2c 캐시로 답해도 판정은 같다(역할 좌석)", INFO_MARK in c2.stdout and WARN_MARK not in c2.stdout,
          c2.stdout[-200:])

    # ── ⓒ `CYS_BIN` 존중: 레인이 지정한 이름으로만 묻는다(`cys` 하드코딩 금지) ────────────────
    s = Seat(tmp, "cysbin", cys_name="mycys", cys_body='printf "worker-2\\n"; exit 0',
             env_extra={"CYS_BIN": "mycys"})
    r = s.run()
    check("3a ★CYS_BIN 이 지정한 실행파일로 물었다", len(s.calls()) == 1, repr(s.calls()))
    check("3b 그 응답이 판정에 쓰인다", "역할 좌석 worker-2 이다" in r.stdout, r.stdout[-200:])
    # 음성 대조: `cys` 라는 이름의 실행파일은 아예 없는데도 위가 성립했다 — 하드코딩이면 불가능하다.
    check("3c 음성 대조 — PATH 에 `cys` 는 없다",
          not os.path.exists(os.path.join(s.bin, "cys")))

    # ── ⓓ 문법 밖 역할은 컨텍스트에 실리지 않는다 ─────────────────────────────────────────
    s = Seat(tmp, "grammar-daemon", cys_body='printf "cso 그리고 이 문장은 주입되면 안 된다\\n"; exit 0')
    r = s.run()
    check("4a ★데몬이 문법 밖 값을 줘도 역할로 쓰지 않는다",
          WARN_MARK in r.stdout and INFO_MARK not in r.stdout, r.stdout[-200:])
    check("4b 그 문장이 컨텍스트에 안 들어간다",
          "이 문장은 주입되면 안 된다" not in r.stdout, r.stdout[-300:])
    s = Seat(tmp, "grammar-env", cys_body='exit 3',
             env_extra={"CYS_ROLE": "cso 그리고 이 문장도 주입되면 안 된다"})
    r = s.run()
    check("4c ★env 폴백의 문법 밖 값도 역할로 쓰지 않는다",
          WARN_MARK in r.stdout and INFO_MARK not in r.stdout, r.stdout[-200:])
    check("4d 그 문장도 컨텍스트에 안 들어간다",
          "이 문장도 주입되면 안 된다" not in r.stdout, r.stdout[-300:])
    # 음성 대조: 같은 경로로 **문법 안의** env 값은 그대로 역할이 된다(과교정 아님).
    s = Seat(tmp, "grammar-env-ok", cys_body='exit 3', env_extra={"CYS_ROLE": "worker-2"})
    r = s.run()
    check("4e 음성 대조 — 문법 안의 env 값은 그대로 역할이다",
          "역할 좌석 worker-2 이다" in r.stdout, r.stdout[-200:])

    # ── ⑤ 자동기동 봉인이 정본 경로에서도 유지된다(부트 폭주 ①) ──────────────────────────
    s = Seat(tmp, "autostart", cys_body='printf "%s\\n" "${CYS_NO_AUTOSTART:-<unset>}" '
                                        '>> "$0.autostart"; printf "cso\\n"; exit 0')
    s.run()
    ap = os.path.join(s.bin, "cys.autostart")
    seen = open(ap, encoding="utf-8").read().split() if os.path.exists(ap) else []
    check("5a 조회가 실제로 일어났다(계측 타당성)", seen != [], repr(seen))
    check("5b ★모든 조회에 CYS_NO_AUTOSTART=1", bool(seen) and all(x == "1" for x in seen),
          repr(seen))
finally:
    shutil.rmtree(tmp, ignore_errors=True)

print("")
if fails:
    print("FAILED %d: %s" % (len(fails), " · ".join(fails)))
    sys.exit(1)
print("ALL PASS")
print("REFL-INJECT-ROLE-CANON-OK")
