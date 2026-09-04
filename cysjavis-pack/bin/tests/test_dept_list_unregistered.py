#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_dept_list_unregistered.py — `cys-dept list` 미등재 소켓 디렉터리 고지 핀 (0.14.30 A2 · SURVEY B3).

밀폐 하네스: `HOME` 과 `CYS_DEPTS_JSON`(cys-dept:22 의 `REG` 해소 지점)을 임시 디렉터리로 덮어
레지스트리·소켓 글롭을 통째로 격리한다 — 실 `~/.cys/depts.json`·실 `~/.local/state/cys-dept-*`
무접촉이고 데몬도 부르지 않는다(`list` 는 레지스트리 읽기 전용).

  ① 양성 — 등재 sales + 유령 ghost: stdout 은 `sales` 한 줄, stderr 에 ghost 경고 1건, 등재 부서는 무경고.
  ② 음성 — 등재 부서 디렉터리만 존재: 경고 0건.
  ③ 음성 — 소켓 디렉터리 자체가 0건: 경고 0건.
  ④ 등재 0건 + 유령 2건: stdout 무출력(빈 줄도 없음) + 경고 2건.
  ⑤ 소비자 계약 — `for d in $(cys-dept list)` 가 집는 stdout 토큰 집합에 유령 이름이 없다.
  ⑥ ★검출력 증명(음성 대조): 경고의 `>&2` 를 뗀 **변조 사본**은 같은 조건에서 stdout 을 오염시킨다.
     이 대조가 성립해야 ①⑤ 의 '경고는 stderr 전용' 주장이 무언가를 재는 것이 된다.
  ⑦ 정상 teardown 형상(`~/.local/state/cys-trash/<name>-<ts>/`)은 글롭 무매치라 경고 0건
     (cys-dept:59-60 의 '격리=재발견 절단' 규약이 이 고지에도 그대로 성립한다는 핀).
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 DEPT-LIST-UNREGISTERED-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_dept_list_unregistered.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
PACK_SRC = os.path.abspath(os.path.join(SELF, "..", ".."))
DEPT = os.path.join(PACK_SRC, "bin", "cys-dept")
BASH = shutil.which("bash") or "/bin/bash"
WARN = "미등재 소켓 디렉터리"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def make_home(root, tag, registered=(), sock_dirs=(), trash_dirs=()):
    home = os.path.join(root, "home-" + tag)
    os.makedirs(os.path.join(home, ".cys"), exist_ok=True)
    reg = os.path.join(home, "depts.json")
    with open(reg, "w", encoding="utf-8") as f:
        json.dump({"depts": {n: {} for n in registered}}, f)
    for n in sock_dirs:
        os.makedirs(os.path.join(home, ".local", "state", "cys-dept-" + n), exist_ok=True)
    for n in trash_dirs:
        os.makedirs(os.path.join(home, ".local", "state", "cys-trash", n + "-1788400000"),
                    exist_ok=True)
    return home, reg


def run_list(home, reg, script=DEPT):
    env = dict(os.environ)
    env["HOME"] = home
    env["CYS_DEPTS_JSON"] = reg
    env.pop("CYS_SOCKET", None)
    r = subprocess.run([BASH, script, "list"], capture_output=True, text=True,
                       timeout=60, env=env)
    return r.returncode, r.stdout, r.stderr


if os.name == "nt":
    print("SKIP windows — named pipe 는 파일이 아니라 glob 대상이 아니다(고지 자체가 unix 한정)")
    print("DEPT-LIST-UNREGISTERED-OK")
    sys.exit(0)

root = tempfile.mkdtemp()
try:
    # ① 양성
    home, reg = make_home(root, "1", registered=("sales",), sock_dirs=("sales", "ghost"))
    code, out, err = run_list(home, reg)
    check("1a exit 0", code == 0, repr((code, err[-200:])))
    check("1b stdout 은 등재명만", out == "sales\n", repr(out))
    check("1c stderr 에 유령 경고 1건", err.count(WARN) == 1 and "ghost" in err, repr(err))
    check("1d 등재 부서는 무경고", "sales" not in err.replace("cys-dept", ""), repr(err))

    # ② 음성 — 등재 부서 디렉터리만
    home, reg = make_home(root, "2", registered=("sales",), sock_dirs=("sales",))
    code, out, err = run_list(home, reg)
    check("2 등재만 있으면 경고 0", code == 0 and out == "sales\n" and WARN not in err,
          repr((code, out, err)))

    # ③ 음성 — 소켓 디렉터리 0건(글롭 무매치)
    home, reg = make_home(root, "3", registered=("sales",))
    code, out, err = run_list(home, reg)
    check("3 소켓 디렉터리 0건이면 경고 0", code == 0 and out == "sales\n" and WARN not in err,
          repr((code, out, err)))

    # ④ 등재 0건 + 유령 2건
    home, reg = make_home(root, "4", registered=(), sock_dirs=("g1", "g2"))
    code, out, err = run_list(home, reg)
    check("4 등재 0 → stdout 무출력(빈 줄도 없음) + 경고 2건",
          code == 0 and out == "" and err.count(WARN) == 2, repr((code, out, err)))

    # ⑤ 소비자 계약 — 심박·fan-out 의 `$(cys-dept list)` 토큰 집합
    home, reg = make_home(root, "5", registered=("sales", "ops"), sock_dirs=("sales", "ghost"))
    code, out, err = run_list(home, reg)
    check("5 stdout 토큰 집합에 유령 없음", set(out.split()) == {"sales", "ops"}, repr(out))

    # ⑥ ★검출력 증명 — `>&2` 를 뗀 변조 사본은 stdout 을 오염시킨다
    with open(DEPT, encoding="utf-8") as f:
        src = f.read()
    mut_src = src.replace('자원 게이트 지연의 원인이니 정리하라." >&2',
                          '자원 게이트 지연의 원인이니 정리하라."')
    if mut_src == src:
        check("6 변조 대조 앵커 실재", False, "경고 줄의 `>&2` 앵커를 못 찾았다(핀 무효)")
    else:
        mut = os.path.join(root, "cys-dept-mut")
        with open(mut, "w", encoding="utf-8") as f:
            f.write(mut_src)
        os.chmod(mut, 0o755)
        home, reg = make_home(root, "6", registered=("sales",), sock_dirs=("sales", "ghost"))
        code, out, err = run_list(home, reg, script=mut)
        check("6 변조본은 stdout 오염(계측 타당성)", WARN in out, repr((out, err)))

    # ⑦ 정상 teardown 격리 형상은 무경고
    home, reg = make_home(root, "7", registered=("sales",), sock_dirs=("sales",),
                          trash_dirs=("ghost",))
    code, out, err = run_list(home, reg)
    check("7 cys-trash 격리본은 글롭 무매치 → 경고 0",
          code == 0 and out == "sales\n" and WARN not in err, repr((code, out, err)))
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("DEPT-LIST-UNREGISTERED-OK")
sys.exit(0)
