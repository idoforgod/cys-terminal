#!/usr/bin/env python3
"""test_directive_compose.py — W1 invert-merge 계약 핀 (T0 RED-first · 설계 DD-1).

현 코드(`cys-dept` _swap = `cp "$ceo" "$md"` 통째 replace)에서는 **반드시 FAIL** 해야 정상이다
(RED). T1(invert-merge)이 이 파일을 GREEN 으로 만든다.

계약(구현 후 통과 기준):
  1) promote 는 표준 base(MASTER_DIRECTIVE) 전문을 보존한다 — replace 금지, concat(invert-merge).
  2) promote 는 sentinel(CEO-OVERLAY BEGIN/END) 마커로 오버레이 구간을 구획한다.
  3) promote 는 멱등 — 재실행해도 오버레이가 이중 append 되지 않는다(정확히 1개 구간).
  4) C03-on-concat — concat 형태에서도 표준 핀(자율주행·self-clear 금지·복원 등)이 전부 잔존.
  5) recompose 서브커맨드가 존재하고 멱등이다(pack-update 사후 재합성 훅).
  6) demote 는 sentinel 구간만 strip 하고 base 는 무손상.

실행: python3 test_directive_compose.py   (exit 0=전건 PASS / 1=RED 또는 회귀)
CI: release.yml pack 테스트 루프(glob test_parity_*/명시목록) — 실 데몬 무접촉(스텁 cys).
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
DEPT = os.path.join(SELF, "..", "cys-dept")
fails = []

# 표준 base 의 대표 핀(반쪽마스터 canary C03 이 검사하는 조항 계열의 축약 대역).
BASE_PINS = [
    "ABSOLUTE ANCHOR",
    "자율주행",
    "self-clear 금지",
    "복원 체크포인트",
    "워커에게 위임",
]
BASE_TEXT = (
    "# 표준 MASTER_DIRECTIVE (테스트 base)\n\n"
    "## [ABSOLUTE ANCHOR]\n"
    "- 자율주행 3축을 완주한다.\n"
    "- 컨텍스트 60% 초과 전 저장 후 self-clear 금지(마스터 책임).\n"
    "- 복원 체크포인트: SESSION_STATE 갱신.\n"
    "- 구현은 워커에게 위임하라.\n"
)
OVERLAY_MARK = "부서장 거버넌스 오버레이(CEO)"
OVERLAY_TEXT = (
    "# CEO 거버넌스 오버레이\n"
    "이 오버레이는 base 위에 추가되는 거버넌스 계층이다. "
    "충돌 시 우선 조항은 §CEO-1(부서장에게만 지시·타 부서 워커 직접 관할 금지) 뿐.\n"
    "%s\n" % OVERLAY_MARK
)

SENTINEL_BEGIN = re.compile(r"<!--\s*CEO-OVERLAY BEGIN")
SENTINEL_END = re.compile(r"<!--\s*CEO-OVERLAY END")


_total = [0]


def check(name, cond, detail=""):
    _total[0] += 1
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def setup(tmp):
    home = os.path.join(tmp, "home")
    pack = os.path.join(home, ".cys", "pack", "directives")
    bindir = os.path.join(home, ".local", "bin")
    os.makedirs(pack, exist_ok=True)
    os.makedirs(bindir, exist_ok=True)
    with open(os.path.join(pack, "MASTER_DIRECTIVE.md"), "w", encoding="utf-8") as f:
        f.write(BASE_TEXT)
    # 현행은 CEO_TEMPLATE.md, T1 후는 CEO_OVERLAY.md — 리네임 전후 모두 견디게 둘 다 쓴다.
    for fn in ("CEO_TEMPLATE.md", "CEO_OVERLAY.md"):
        with open(os.path.join(pack, fn), "w", encoding="utf-8") as f:
            f.write(OVERLAY_TEXT)
    with open(os.path.join(home, ".cys", "depts.json"), "w", encoding="utf-8") as f:
        json.dump({"depts": {"d0": {}}}, f)
    os.makedirs(os.path.join(home, ".cys", "state"), exist_ok=True)
    # 부트 마커(승격 게이트 통과용).
    with open(os.path.join(home, ".cys", ".master-bootstrapped"), "w", encoding="utf-8") as f:
        f.write("{}")
    stub = os.path.join(bindir, "cys")
    with open(stub, "w", encoding="utf-8", newline="\n") as f:
        f.write('#!/bin/sh\necho "cys $@" >> "%s/calls.log"\n'
                'case "$1" in status) exit 1;; esac\nexit 0\n' % tmp)
    os.chmod(stub, 0o755)
    env = dict(os.environ)
    env.update({"HOME": home, "PATH": bindir + os.pathsep + env.get("PATH", "")})
    for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_ACCOUNT_DIR"):
        env.pop(k, None)
    return env, home


def md_path(home):
    return os.path.join(home, ".cys", "pack", "directives", "MASTER_DIRECTIVE.md")


def run(env, *args):
    r = subprocess.run(["bash", DEPT] + list(args), capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=60)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def read_md(home):
    with open(md_path(home), encoding="utf-8") as f:
        return f.read()


def main():
    # ── promote 계약 ──
    tmp = tempfile.mkdtemp(prefix="compose-")
    env, home = setup(tmp)
    code, out = run(env, "promote-ceo")
    body = read_md(home)

    missing = [p for p in BASE_PINS if p not in body]
    check("1 promote 는 base 전문 보존(replace 금지·invert-merge)",
          not missing, "소실 핀=%r (현행 _swap replace 라 전부 소실=RED)" % missing)
    check("2 오버레이 sentinel BEGIN/END 마커 존재",
          bool(SENTINEL_BEGIN.search(body)) and bool(SENTINEL_END.search(body)),
          "sentinel 부재(현행 replace=RED)")
    check("2b 오버레이 내용도 포함(concat)",
          OVERLAY_MARK in body, "" if OVERLAY_MARK in body else "overlay 마커 부재")

    # ── 멱등: 재-promote → 오버레이 1개 구간 ──
    run(env, "promote-ceo")
    body2 = read_md(home)
    n_begin = len(SENTINEL_BEGIN.findall(body2))
    check("3 promote 멱등 — 오버레이 정확히 1구간(이중 append 금지)",
          n_begin == 1, "BEGIN 마커 %d 개(1 이어야 함)" % n_begin)

    # ── C03-on-concat: 재-promote 후에도 base 핀 전부 잔존 ──
    missing2 = [p for p in BASE_PINS if p not in body2]
    check("4 C03-on-concat — 승격 후에도 표준 핀 전부 잔존",
          not missing2, "소실 핀=%r" % missing2)

    # ── recompose 서브커맨드 존재·멱등 ──
    code_r, out_r = run(env, "recompose")
    check("5 recompose 서브커맨드 존재(pack-update 사후 재합성)",
          code_r == 0, "exit=%d out=%r (현행 미구현=RED)" % (code_r, out_r[-120:]))
    if code_r == 0:
        body_r = read_md(home)
        check("5b recompose 멱등 — 오버레이 1구간·base 핀 잔존",
              len(SENTINEL_BEGIN.findall(body_r)) == 1 and not [p for p in BASE_PINS if p not in body_r])
    else:
        check("5b recompose 멱등 — 오버레이 1구간·base 핀 잔존", False, "recompose 미구현으로 미검(RED)")

    # ── demote: sentinel strip · base 무손상 ──
    # 부서 0 으로 만들어 down 경로의 ceo_demote 를 유발(수동 demote 명령 부재).
    with open(os.path.join(home, ".cys", "depts.json"), "w", encoding="utf-8") as f:
        json.dump({"depts": {}}, f)
    # ceo_demote 는 down/down-sock 내부에서 reg_count=0 시 호출 — 직접 유발이 어려우면 계약만 정적 검사.
    # 여기서는 "demote 후 md 에 sentinel 이 없고 base 핀이 있다"는 최종 상태 계약을 사후 recompose(overlay=none)로 대신 검증.
    code_d, out_d = run(env, "recompose", "--no-overlay")
    if code_d == 0:
        body_d = read_md(home)
        check("6 demote/strip — sentinel 제거·base 무손상",
              not SENTINEL_BEGIN.search(body_d) and not [p for p in BASE_PINS if p not in body_d],
              "sentinel 잔존 또는 base 소실")
    else:
        check("6 demote/strip — sentinel 제거·base 무손상", False,
              "recompose --no-overlay 미구현(RED) exit=%d" % code_d)

    shutil.rmtree(tmp, ignore_errors=True)
    print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
