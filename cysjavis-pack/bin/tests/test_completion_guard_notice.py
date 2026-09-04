#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_completion_guard_notice.py — completion-guard.sh 이중 휴면 1회 고지 핀 (0.14.30 A5 · SURVEY 2026-09-03 §C1).

밀폐 하네스: 임시 PACK(hooks/_lib.sh + hooks/completion-guard.sh 를 소스 팩에서 복사) + 임시 TMPDIR(마커
격리)로 래퍼를 sh 로 실행한다. 실 팩·실 settings.json·실 TMPDIR·실 CYS_PACK_DIR 무접촉(외부 CYS_PACK_DIR 은
자기 임시 팩으로 덮어쓴다 — CI 규약 `CYS_PACK_DIR="$(mktemp -d)"` 값에 의존하지 않는다).

  ① 미무장(CYS_COMPLETION_GUARD 미설정 · CYS_SURFACE_ID=42) → exit 0 · stderr "env 휴면" 1회 · 마커
     `<TMPDIR>/.cys-guard-env-dormant-42` 실재 · 재실행 시 고지 0(surface 당 1회) · stdout 무출력 ·
     surface 미상이면 접미 nosurface · 다른 surface 는 자기 1회.
  ② 무장 + 본체 부재 → exit 0 · stderr "본체 … 부재" 1회 · env 휴면 고지 0 · 재실행 고지 0.
  ③ 무장 + 본체(스텁 python: GUARD-RAN 출력) → stdout GUARD-RAN · exit 0 · 두 고지 0 · 마커 0 ·
     스텁 rc 전달(exit 3 → 래퍼 3 — 무장 경로 무변경 대조).
  ④ 음성 대조(무스폰 계약 completion-guard.sh:10-13): **`dirname` 하나만 있는 PATH** 로 미무장 실행
     → exit 0 · 고지 여전 · 'not found' 0. (PATH="" 는 프리루드 1단의 외부 `dirname` 이 죽어 `.` 치명
     종료로 훅이 rc=1 이 된다 — 측정 불능이라 쓰지 않는다.) 검출력 증명: 마커에 `$(id -u)` 를 끼운
     변조 사본은 같은 조건에서 'id … not found' 를 낸다(이 대조가 통과해야 ④ 가 의미 있다).
  ⑤ `sh -n` 구문 검사 · dash 가 있으면 `dash -n` + ① 을 dash 로 재실행(POSIX 엄격 · Windows PortableGit
     sh 는 이 하네스 밖 — 내장만 쓰므로 동형이라는 것이 설계 근거).
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 COMPLETION-GUARD-NOTICE-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_completion_guard_notice.py
"""
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
PACK_SRC = os.path.abspath(os.path.join(SELF, "..", ".."))
SH = shutil.which("sh") or "/bin/sh"          # 최소 PATH 케이스에서도 실행되도록 절대경로로 고정
DASH = shutil.which("dash")
ENV_NOTICE = "env 휴면"
BODY_NOTICE = "본체 bin/javis_completion_guard.py 부재"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def make_pack(root):
    pack = os.path.join(root, "pack")
    os.makedirs(os.path.join(pack, "hooks"))
    os.makedirs(os.path.join(pack, "bin"))
    for name in ("_lib.sh", "completion-guard.sh"):
        shutil.copy2(os.path.join(PACK_SRC, "hooks", name), os.path.join(pack, "hooks", name))
    return pack


def fresh_tmpdir(root, tag):
    d = os.path.join(root, "tmp-" + tag)
    os.makedirs(d)
    return d


def base_env(root, pack, tmpdir, surface="42", armed=False, cys_py=None, path=None):
    env = {
        "HOME": os.path.join(root, "home"),
        "TMPDIR": tmpdir,
        "CYS_PACK_DIR": pack,
        "PATH": os.environ.get("PATH", "/usr/bin:/bin") if path is None else path,
    }
    # 로케일은 부모 값을 상속(프리루드 cys_fix_locale 이 UTF-8 이면 `locale` 스폰 없이 즉시 반환).
    for k in ("LANG", "LC_ALL", "LC_CTYPE"):
        if os.environ.get(k):
            env[k] = os.environ[k]
    if not any(k in env for k in ("LANG", "LC_ALL", "LC_CTYPE")):
        env["LANG"] = "C.UTF-8"
    if surface is not None:
        env["CYS_SURFACE_ID"] = surface
    if armed:
        env["CYS_COMPLETION_GUARD"] = "1"
    if cys_py:
        env["CYS_PY"] = cys_py
    return env


def run(hook, env, shell=None):
    r = subprocess.run([shell or SH, hook], capture_output=True, text=True, encoding="utf-8",
                       errors="replace", env=env, stdin=subprocess.DEVNULL, timeout=90,
                       cwd=env["TMPDIR"])
    return r.returncode, r.stdout, r.stderr


def markers(tmpdir):
    return sorted(n for n in os.listdir(tmpdir) if n.startswith(".cys-guard-"))


root = tempfile.mkdtemp(prefix="cg-notice-")
try:
    pack = make_pack(root)
    HOOK = os.path.join(pack, "hooks", "completion-guard.sh")

    # ── ① 미무장: env 휴면 고지 1회/surface ──
    t1 = fresh_tmpdir(root, "1")
    env = base_env(root, pack, t1)
    code, out, err = run(HOOK, env)
    check("1a 미무장 exit 0", code == 0, "rc=%s stderr=%r" % (code, err))
    check("1b 미무장 env 휴면 고지 정확히 1줄", err.count(ENV_NOTICE) == 1, repr(err))
    check("1c 고지 문안 — 무장 env 이름 + README 포인터",
          "CYS_COMPLETION_GUARD=1" in err and "hooks/README.md" in err, repr(err))
    check("1d 고지는 [cys-hook] completion-guard: 접두(기존 stderr 채널 문형)",
          "[cys-hook] completion-guard: env 휴면" in err)
    marker = os.path.join(t1, ".cys-guard-env-dormant-42")
    check("1e 마커 = TMPDIR/.cys-guard-env-dormant-<surface>", os.path.isfile(marker), marker)
    check("1f stdout 무출력", out == "", repr(out))
    check("1g 본체 부재 고지는 미무장 경로에서 0", BODY_NOTICE not in err)
    code2, out2, err2 = run(HOOK, env)
    check("1h 재실행 exit 0", code2 == 0)
    check("1i 재실행 고지 0(surface 당 1회)", ENV_NOTICE not in err2, repr(err2))
    code3, _, err3 = run(HOOK, base_env(root, pack, t1, surface=None))
    check("1j surface 미상 → nosurface 마커 + 고지 1회",
          code3 == 0 and err3.count(ENV_NOTICE) == 1
          and os.path.isfile(os.path.join(t1, ".cys-guard-env-dormant-nosurface")), repr(err3))
    _, _, err4 = run(HOOK, base_env(root, pack, t1, surface="43"))
    check("1k 다른 surface 는 자기 1회", err4.count(ENV_NOTICE) == 1, repr(err4))
    check("1l 마커 집합 = 42·43·nosurface (body-absent 마커 0)",
          markers(t1) == [".cys-guard-env-dormant-42", ".cys-guard-env-dormant-43",
                          ".cys-guard-env-dormant-nosurface"], repr(markers(t1)))

    # ── ② 무장 + 본체 부재 ──
    t2 = fresh_tmpdir(root, "2")
    env = base_env(root, pack, t2, armed=True)
    code, out, err = run(HOOK, env)
    check("2a 무장+본체 부재 exit 0", code == 0, "rc=%s stderr=%r" % (code, err))
    check("2b 본체 부재 고지 정확히 1줄", err.count(BODY_NOTICE) == 1, repr(err))
    check("2c env 휴면 고지 0(무장 상태)", ENV_NOTICE not in err)
    check("2d 마커 .cys-guard-body-absent-42",
          os.path.isfile(os.path.join(t2, ".cys-guard-body-absent-42")), repr(markers(t2)))
    check("2e stdout 무출력", out == "", repr(out))
    code2, _, err2 = run(HOOK, env)
    check("2f 재실행 exit 0 · 고지 0", code2 == 0 and BODY_NOTICE not in err2, repr(err2))

    # ── ③ 무장 + 본체(스텁) — 무장 경로 무변경 대조 ──
    stub = os.path.join(pack, "bin", "javis_completion_guard.py")
    with open(stub, "w", encoding="utf-8") as f:
        f.write('import os, sys\nprint("GUARD-RAN")\n'
                'sys.exit(int(os.environ.get("CG_STUB_RC", "0")))\n')
    t3 = fresh_tmpdir(root, "3")
    env = base_env(root, pack, t3, armed=True, cys_py=sys.executable)
    code, out, err = run(HOOK, env)
    check("3a 무장+본체: 본체 실행(GUARD-RAN)", "GUARD-RAN" in out, repr((out, err)))
    check("3b exit 0", code == 0, "rc=%s stderr=%r" % (code, err))
    check("3c 두 고지 0", ENV_NOTICE not in err and BODY_NOTICE not in err, repr(err))
    # ★휴면 마커의 정확한 두 이름만 본다 — 부분문자열 'absent' 로 걸면 **기존** 코얼레싱 마커
    #   `.cys-guard-timeout-absent-<uid>-<날짜>`(:72 · coreutils timeout 부재 고지 · 무장 경로에서
    #   정상 생성된다)까지 잡아 무장 경로를 결함으로 오판한다(축이 다른 마커다).
    dormancy = [m for m in markers(t3)
                if m.startswith(".cys-guard-env-dormant-") or m.startswith(".cys-guard-body-absent-")]
    check("3d 휴면 마커 0(무장·본체 실행 경로)", not dormancy, repr(markers(t3)))
    env["CG_STUB_RC"] = "3"
    code, out, _ = run(HOOK, env)
    check("3e 본체 rc 전달(스텁 exit 3 → 래퍼 3)", code == 3 and "GUARD-RAN" in out, "rc=%s" % code)

    # ── ④ 음성 대조: 미무장 경로는 셸 내장만(최소 PATH) ──
    # ★PATH="" 가 아니라 **`dirname` 하나만 있는 PATH** 를 쓴다(실측 근거): 프리루드 1단
    #   `. "$(dirname "$0")/_lib.sh"`(_lib.sh:20 규약 문장 · 이 커밋 범위 밖의 기존 행)은 외부
    #   `dirname` 을 쓰므로 PATH="" 에서는 치환이 실패해 `. "/_lib.sh"` 가 되고, POSIX 셸은
    #   비대화형에서 `.` 의 파일 부재를 **치명 종료**로 다룬다 → 훅이 rc=1 로 죽어 2단 폴백에도
    #   닿지 못한다(실측: `/bin/sh: line 5: dirname: No such file or directory` rc=1).
    #   그 상태는 '고지 블록이 외부 명령을 쓰는가' 를 재지 못한다 — 측정 불능이지 통과가 아니다.
    #   그래서 프리루드가 필요한 것 하나만 주고, 그 위에서 고지 블록이 **추가로** 아무것도
    #   스폰하지 않음을 잰다(아래 4f 변조 대조가 이 하네스의 검출력을 실증한다).
    minbin = os.path.join(root, "minbin")
    os.makedirs(minbin, exist_ok=True)
    _dn = shutil.which("dirname") or "/usr/bin/dirname"
    if not os.path.exists(os.path.join(minbin, "dirname")):
        os.symlink(_dn, os.path.join(minbin, "dirname"))
    t4 = fresh_tmpdir(root, "4")
    env = base_env(root, pack, t4, surface="44", path=minbin)
    code, out, err = run(HOOK, env)
    check("4a 최소 PATH 미무장 exit 0", code == 0, "rc=%s stderr=%r" % (code, err))
    check("4b 최소 PATH 고지 여전(내장만으로 출력)", err.count(ENV_NOTICE) == 1, repr(err))
    check("4c 최소 PATH 마커 생성(내장 리다이렉션)",
          os.path.isfile(os.path.join(t4, ".cys-guard-env-dormant-44")), repr(markers(t4)))
    nf = [l for l in err.splitlines() if "not found" in l and "dirname" not in l]
    check("4d 외부 명령 0 — 'not found' 는 프리루드 1단 dirname(기존 행) 외 없음", not nf, repr(nf))
    # 검출력 증명: 마커에 $(id -u) 를 끼운 변조 사본 → PATH='' 에서 'id … not found' 가 나와야 한다.
    with open(HOOK, encoding="utf-8") as f:
        src = f.read()
    anchor = '.cys-guard-env-dormant-${CYS_SURFACE_ID:-nosurface}"'
    check("4e 변조 앵커 실재(마커 행 1개)", src.count(anchor) == 1, str(src.count(anchor)))
    mut = os.path.join(pack, "hooks", "completion-guard-mut.sh")
    with open(mut, "w", encoding="utf-8") as f:
        f.write(src.replace(anchor, '.cys-guard-env-dormant-${CYS_SURFACE_ID:-nosurface}-$(id -u)"'))
    t4m = fresh_tmpdir(root, "4m")
    code, _, err = run(mut, base_env(root, pack, t4m, surface="44", path=minbin))
    nf_m = [l for l in err.splitlines() if "not found" in l and "dirname" not in l]
    check("4f 변조 사본은 'id … not found' 를 낸다(음성 대조 검출력)",
          code == 0 and any("id" in l for l in nf_m), repr(err))

    # ── ⑤ 구문 검사 + dash 재실행 ──
    r = subprocess.run([SH, "-n", HOOK], capture_output=True, text=True)
    check("5a sh -n 구문 OK", r.returncode == 0, r.stderr)
    if DASH:
        r = subprocess.run([DASH, "-n", HOOK], capture_output=True, text=True)
        check("5b dash -n 구문 OK", r.returncode == 0, r.stderr)
        t5 = fresh_tmpdir(root, "5")
        env = base_env(root, pack, t5, surface="45")
        code, out, err = run(HOOK, env, shell=DASH)
        _, _, err2 = run(HOOK, env, shell=DASH)
        check("5c dash: 미무장 고지 1회 + 재실행 0 + exit 0 + stdout 무출력",
              code == 0 and err.count(ENV_NOTICE) == 1 and ENV_NOTICE not in err2 and out == "",
              repr((code, out, err, err2)))
    else:
        print("SKIP 5b/5c dash 부재 — POSIX 엄격 재실행 생략")
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("COMPLETION-GUARD-NOTICE-OK")
sys.exit(0)
