#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_state_snapshot_root.py — phoenix 세대 스냅샷의 프로젝트 round 해소 회귀 (0.14.30 A4 #3 · PREP #3).

## 무엇을 막는가
`phoenix-snapshot-6h` 잡은 데몬 스케줄러가 **cwd `/` · JAVIS_ROOT 미설정**으로 이 스크립트를 부른다.
종전 산식 `os.environ.get("JAVIS_ROOT") or os.getcwd()` + `_round` 는 그 환경에서 `/_round` 로 해소돼
**복원의 단일 진실인 SESSION_STATE.md 와 노드 TODO 가 매 세대 manifest 에서 빠졌다**(PREP #3 실측:
파일 50건 중 누락 3건 · 연속 5세대+). 재부팅 복원 자료가 백업에서 조용히 사라지는 결함이다.

## 이 파일이 못박는 것
  A) 순수 함수 `project_round_dir(env, cwd)` 3단 폴백 — ①JAVIS_ROOT 우선(프로젝트 레인 거동 불변)
     ②팩 env 키의 `<pack>/round` 가 **디렉터리로 실재**할 때만 채택 ③그 밖은 `<cwd>/_round`(종전).
     키 순서와 **'첫 비어있지 않은 값이 이긴다'(뒤 키 미조회)** 계약까지 고정한다.
  B) 실행 경로 e2e — 데몬 형상(cwd `/` · JAVIS_ROOT 없음 · CYS_PACK_DIR 있음)으로 `snapshot` 을
     서브프로세스 실행하면 세대 manifest 에 SESSION_STATE.md·WORKER_TODO.md 가 **실제로 담긴다**.
  C) ★음성 대조(수정 전 형상 재현) — 같은 실행에서 CYS_PACK_DIR 만 지우면 두 파일이 담기지 않는다.
     이 대조가 없으면 B 는 "원래 담겼던 것"과 구별되지 않는다.

밀폐: HOME·CYS_STATE_DIR·팩 전부 임시 디렉터리(GEN_ROOT 은 모듈 로드 시 HOME 파생 — 서브프로세스로
격리한다). 라이브 `~/.cys/state-generations` 무접촉.

실행(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_state_snapshot_root.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 SNAPSHOT-ROOT-OK.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
MOD = os.path.join(BIN, "javis_state_snapshot.py")
sys.path.insert(0, BIN)

import javis_state_snapshot as ss  # noqa: E402 — 순수 함수 직접 핀(서브프로세스는 B/C 에서만)

PY = sys.executable or "python3"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


# ── A. 순수 함수 3단 폴백 ───────────────────────────────────────────────────────
root = tempfile.mkdtemp(prefix="snap-root-")
try:
    pack = os.path.join(root, "pack")
    os.makedirs(os.path.join(pack, "round"))
    nopack = os.path.join(root, "packless")     # round/ 없는 팩 경로
    os.makedirs(nopack)

    check("A1 JAVIS_ROOT 우선(프로젝트 레인 거동 불변)",
          ss.project_round_dir({"JAVIS_ROOT": "/proj", "CYS_PACK_DIR": pack}, cwd="/") ==
          os.path.join("/proj", "_round"))
    check("A2 JAVIS_ROOT 부재 + 팩 round 실재 → <pack>/round (데몬 잡 형상)",
          ss.project_round_dir({"CYS_PACK_DIR": pack}, cwd="/") == os.path.join(pack, "round"))
    check("A3 팩 env 는 있으나 round/ 부재 → cwd 폴백(존재하지 않는 경로를 채택하지 않는다)",
          ss.project_round_dir({"CYS_PACK_DIR": nopack}, cwd="/tmp") == os.path.join("/tmp", "_round"))
    check("A4 env 전무 → cwd/_round (종전 폴백 보존)",
          ss.project_round_dir({}, cwd="/tmp") == os.path.join("/tmp", "_round"))
    check("A5 키 목록·순서가 정본 미러(CYS_PACK_DIR·JAVIS_PACK_DIR·AITERM_PACK_DIR·AITERM_JARVIS_DIR)",
          ss.PACK_DIR_ENV_KEYS ==
          ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"),
          repr(ss.PACK_DIR_ENV_KEYS))
    # ★'첫 비어있지 않은 값이 이긴다' — 첫 키가 round 없는 경로면 **뒤 키를 보지 않고** cwd 폴백.
    check("A6 첫 키가 이긴다(뒤 키 미조회 — 정본 PACK_DIR_ENV_KEYS 계약)",
          ss.project_round_dir({"CYS_PACK_DIR": nopack, "JAVIS_PACK_DIR": pack}, cwd="/tmp") ==
          os.path.join("/tmp", "_round"))
    check("A7 빈 문자열 키는 '미설정'으로 건너뛴다",
          ss.project_round_dir({"CYS_PACK_DIR": "", "JAVIS_PACK_DIR": pack}, cwd="/") ==
          os.path.join(pack, "round"))

    # ── B/C. 데몬 형상 e2e + 음성 대조 ──────────────────────────────────────────
    def run_snapshot(with_pack):
        """cwd '/' · JAVIS_ROOT 없음 형상으로 snapshot 1회 → (rc, 세대 manifest 파일명 집합)."""
        home = tempfile.mkdtemp(prefix="snap-home-", dir=root)
        p = os.path.join(home, "pack")
        os.makedirs(os.path.join(p, "round"))
        with open(os.path.join(p, "round", "SESSION_STATE.md"), "w", encoding="utf-8") as f:
            f.write("# 복원 정본\n")
        with open(os.path.join(p, "round", "WORKER_TODO.md"), "w", encoding="utf-8") as f:
            f.write("- [ ] 노드 할 일\n")
        # ★음성 대조가 '세대 미생성'으로 접히면 아무것도 증명하지 못한다(누락 vs 미실행 구별 불가).
        #   두 형상 모두에서 세대가 **실제로 만들어지도록** 무관한 소스 1개를 심는다 —
        #   DECLARATIVE_BASENAMES(topology.json 등)는 default_sources 가 항상 수집하는 자리다.
        st = os.path.join(home, ".local", "state", "cys")
        os.makedirs(st)
        with open(os.path.join(st, "topology.json"), "w", encoding="utf-8") as f:
            f.write("{}\n")
        env = {"HOME": home, "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
               "LANG": os.environ.get("LANG", "C.UTF-8")}
        if with_pack:
            env["CYS_PACK_DIR"] = p
        r = subprocess.run([PY, MOD, "snapshot"], capture_output=True, text=True,
                           encoding="utf-8", errors="replace", env=env, cwd="/", timeout=120)
        names = set()
        gen_root = os.path.join(home, ".cys", "state-generations")
        for g in sorted(os.listdir(gen_root)) if os.path.isdir(gen_root) else []:
            mp = os.path.join(gen_root, g, "manifest.json")
            if os.path.isfile(mp):
                with open(mp, encoding="utf-8") as f:
                    for e in json.load(f).get("files", []):
                        names.add(os.path.basename(e.get("source") or e.get("name") or ""))
                        names.add(os.path.basename(e.get("dest") or ""))
        return r.returncode, {n for n in names if n}, r.stdout + r.stderr

    rc_p, names_p, out_p = run_snapshot(True)
    check("B1 데몬 형상 실행 성공(cwd '/' · JAVIS_ROOT 없음 · CYS_PACK_DIR 있음)", rc_p == 0,
          "rc=%s out=%r" % (rc_p, out_p[-200:]))
    check("B2 세대 manifest 에 SESSION_STATE.md 포함(복원 정본 보관)",
          any("SESSION_STATE.md" in n for n in names_p), repr(sorted(names_p)))
    check("B3 세대 manifest 에 WORKER_TODO.md 포함(노드 todo 보관)",
          any("WORKER_TODO.md" in n for n in names_p), repr(sorted(names_p)))

    rc_n, names_n, out_n = run_snapshot(False)
    check("C1 ★음성 대조: CYS_PACK_DIR 없으면 SESSION_STATE.md 미포함(수정 전 형상 재현)",
          not any("SESSION_STATE.md" in n for n in names_n), repr(sorted(names_n)))
    check("C2 ★음성 대조: CYS_PACK_DIR 없으면 WORKER_TODO.md 미포함",
          not any("WORKER_TODO.md" in n for n in names_n), repr(sorted(names_n)))
    check("C3 ★음성 대조에서도 세대는 실제로 생성됐다(누락 vs 미실행 구별 — 무관 소스 topology.json 포함)",
          rc_n == 0 and any("topology.json" in n for n in names_n),
          "rc=%s names=%r out=%r" % (rc_n, sorted(names_n), out_n[-200:]))
    check("C4 양성 형상에도 무관 소스가 함께 담긴다(두 실행의 소스 집합 차이가 팩 round 뿐임을 고정)",
          any("topology.json" in n for n in names_p), repr(sorted(names_p)))
finally:
    shutil.rmtree(root, ignore_errors=True)

print("\n=== %d/%d PASS ===" % (14 - len(fails), 14))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("SNAPSHOT-ROOT-OK")
