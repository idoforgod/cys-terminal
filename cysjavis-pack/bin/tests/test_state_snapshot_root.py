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
import re
import shutil
import subprocess
import sys
import tempfile
import time
from unittest import mock

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
MOD = os.path.join(BIN, "javis_state_snapshot.py")
sys.path.insert(0, BIN)

import javis_state_snapshot as ss  # noqa: E402 — 순수 함수 직접 핀(서브프로세스는 B/C 에서만)

PY = sys.executable or "python3"
fails = []
checks = 0


def check(name, cond, detail=""):
    global checks
    checks += 1
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

    # ── A'. ★[결재 15] 팩 env 전무 + 미끼 `_round` 존재 — 기본 팩 폴백 ─────────────────
    #   실기 형상: cwd=홈에 SESSION_STATE.md 없는 `_round`(save-state.sh 의 .state_log 자리)가 있다.
    #   종전 ③폴백은 그 미끼를 골랐다(round 소스 0건). 기본 팩 `<HOME>/.cys/pack/round` 가 먼저다.
    bhome = os.path.join(root, "bhome")
    os.makedirs(os.path.join(bhome, "_round"))                    # 미끼
    os.makedirs(os.path.join(bhome, ".cys", "pack", "round"))     # 기본 팩 정본
    check("A8 ★팩 env 전무 + 미끼 _round → <HOME>/.cys/pack/round (미끼 불채택)",
          ss.project_round_dir({"HOME": bhome}, cwd=bhome) ==
          os.path.join(bhome, ".cys", "pack", "round"))
    check("A9 ★팩 키가 설정돼 있으면(round 부재) 기본 팩으로 새지 않는다 — 레인 교차 오염 차단",
          ss.project_round_dir({"HOME": bhome, "CYS_PACK_DIR": nopack}, cwd="/tmp") ==
          os.path.join("/tmp", "_round"))
    nohome = os.path.join(root, "nohome")
    os.makedirs(nohome)
    check("A10 기본 팩 round/ 도 없으면 cwd 폴백(존재하지 않는 경로 불채택)",
          ss.project_round_dir({"HOME": nohome}, cwd="/tmp") == os.path.join("/tmp", "_round"))

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

    # ── D. ★[결재 15] e2e — 팩 env 전무 · cwd=홈 · 미끼 _round 존재(실기 phoenix 형상) ─────
    home = tempfile.mkdtemp(prefix="snap-bait-", dir=root)
    dp = os.path.join(home, ".cys", "pack", "round")
    os.makedirs(dp)
    with open(os.path.join(dp, "SESSION_STATE.md"), "w", encoding="utf-8") as f:
        f.write("# 복원 정본\n")
    with open(os.path.join(dp, "WORKER_TODO.md"), "w", encoding="utf-8") as f:
        f.write("- [ ] 노드 할 일\n")
    os.makedirs(os.path.join(home, "_round"))                   # 미끼(SESSION_STATE 없음)
    with open(os.path.join(home, "_round", ".state_log"), "w", encoding="utf-8") as f:
        f.write("bait\n")
    st = os.path.join(home, ".local", "state", "cys")
    os.makedirs(st)
    with open(os.path.join(st, "topology.json"), "w", encoding="utf-8") as f:
        f.write("{}\n")
    env = {"HOME": home, "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
           "LANG": os.environ.get("LANG", "C.UTF-8")}
    r = subprocess.run([PY, MOD, "snapshot"], capture_output=True, text=True,
                       encoding="utf-8", errors="replace", env=env, cwd=home, timeout=120)
    dnames = set()
    gen_root = os.path.join(home, ".cys", "state-generations")
    for g in sorted(os.listdir(gen_root)) if os.path.isdir(gen_root) else []:
        mp = os.path.join(gen_root, g, "manifest.json")
        if os.path.isfile(mp):
            with open(mp, encoding="utf-8") as f:
                for e in json.load(f).get("files", []):
                    dnames.add(os.path.basename(e.get("source") or e.get("name") or ""))
    check("D1 ★미끼 형상 실행 성공(팩 env 전무 · cwd=홈 · ~/_round 미끼)", r.returncode == 0,
          "rc=%s out=%r" % (r.returncode, (r.stdout + r.stderr)[-200:]))
    check("D2 ★미끼 형상에서도 SESSION_STATE.md 가 세대에 담긴다", "SESSION_STATE.md" in dnames,
          repr(sorted(dnames)))
    check("D3 ★미끼 형상에서도 WORKER_TODO.md 가 세대에 담긴다", "WORKER_TODO.md" in dnames,
          repr(sorted(dnames)))

    # ── E. D-07(2026-09-21) 잔여 계약 — TODO 누락·선택 상태·무기록·청소 보호 ───────
    def run_d07_snapshot(todo=True, autopilot=False, round_exists=True, dry_run=False):
        """필수 누락은 schedule_state.json 한 건으로 고정한 밀폐 형상이다."""
        h = tempfile.mkdtemp(prefix="snap-d07-", dir=root)
        p = os.path.join(h, "pack")
        rd = os.path.join(p, "round")
        localappdata = os.path.join(h, "AppData", "Local")
        st = (os.path.join(localappdata, "cys") if os.name == "nt"
              else os.path.join(h, ".local", "state", "cys"))
        os.makedirs(st)
        os.makedirs(os.path.join(h, ".cys"))
        for basename in ("topology.json", "event.seq"):
            with open(os.path.join(st, basename), "w", encoding="utf-8") as f:
                f.write("{}\n")
        with open(os.path.join(h, ".cys", "depts.json"), "w", encoding="utf-8") as f:
            json.dump({"depts": {}}, f)
        if round_exists:
            os.makedirs(rd)
            with open(os.path.join(rd, "SESSION_STATE.md"), "w", encoding="utf-8") as f:
                f.write("# D-07 복원 정본\n")
            if todo:
                with open(os.path.join(rd, "WORKER_TODO.md"), "w", encoding="utf-8") as f:
                    f.write("- [ ] D-07 검체\n")
        ap = os.path.join(st, "autopilot.json")
        if autopilot:
            with open(ap, "w", encoding="utf-8") as f:
                f.write("{}\n")
        # Windows 의 expanduser·상태 루트도 같은 임시 HOME 안으로 밀폐한다.
        env = {"HOME": h, "USERPROFILE": h, "LOCALAPPDATA": localappdata,
               "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
               "LANG": os.environ.get("LANG", "C.UTF-8"), "CYS_PACK_DIR": p}
        # round 부재 형상은 폴백 목적지도 밀폐 HOME 안으로 고정한다.
        if not round_exists:
            env["JAVIS_ROOT"] = h
            rd = os.path.join(h, "_round")
        args = [PY, MOD, "snapshot"] + (["--dry-run"] if dry_run else [])
        result = subprocess.run(args, capture_output=True, text=True, encoding="utf-8",
                                errors="replace", env=env, cwd=h, timeout=120)
        manifests = []
        gr = os.path.join(h, ".cys", "state-generations")
        for name in sorted(os.listdir(gr)) if os.path.isdir(gr) else []:
            mp = os.path.join(gr, name, "manifest.json")
            if os.path.isfile(mp):
                with open(mp, encoding="utf-8") as f:
                    manifests.append(json.load(f))
        return result, manifests, rd, st, ap

    r0, _, rd0, _, _ = run_d07_snapshot(todo=False, dry_run=True)
    todo_pattern = os.path.join(rd0, "*_TODO.md")
    check("E1-D07-TODO-0 glob 0건도 dry-run 누락 패턴으로 표시",
          r0.returncode == 0 and any(
              "(없음)" in line and todo_pattern in line for line in r0.stdout.splitlines()),
          "rc=%s out=%r" % (r0.returncode, r0.stdout + r0.stderr))
    r1, _, rd1, _, _ = run_d07_snapshot(todo=True, dry_run=True)
    check("E2-D07-TODO-1 음성 대조: TODO 실재 시 누락 패턴 없음",
          r1.returncode == 0 and os.path.join(rd1, "WORKER_TODO.md") in r1.stdout
          and not any("(없음)" in line and "*_TODO.md" in line
                      for line in r1.stdout.splitlines()),
          "rc=%s out=%r" % (r1.returncode, r1.stdout + r1.stderr))
    rn, _, rdn, _, _ = run_d07_snapshot(round_exists=False, dry_run=True)
    round_missing = [line for line in rn.stdout.splitlines()
                     if "(없음)" in line and rdn + os.sep in line]
    check("E3-D07-ROUND-0 round 자체 부재 시 SESSION_STATE·TODO 패턴 최소 2건 누락",
          rn.returncode == 0 and len(round_missing) >= 2
          and any("SESSION_STATE.md" in line for line in round_missing)
          and any("*_TODO.md" in line for line in round_missing), repr(round_missing))

    check("E4-D07-OPTIONAL-CONST autopilot 은 선택 소스이면서 선언 소스에 잔류",
          getattr(ss, "OPTIONAL_BASENAMES", None) == ("autopilot.json",)
          and "autopilot.json" in ss.DECLARATIVE_BASENAMES,
          repr(getattr(ss, "OPTIONAL_BASENAMES", None)))
    ra, ma, _, sta, apa = run_d07_snapshot(autopilot=False)
    manifest_a = ma[0] if len(ma) == 1 else {}
    check("E5-D07-OPTIONAL-ABSENT 미생성 autopilot 은 manifest 선택부재에 기록",
          ra.returncode == 0 and manifest_a.get("optional_absent") == [apa],
          "rc=%s optional_absent=%r" % (ra.returncode, manifest_a.get("optional_absent")))
    check("E6-D07-OPTIONAL-MISSING manifest 누락은 필수 소스만 계수",
          manifest_a.get("missing") == [os.path.join(sta, "schedule_state.json")],
          "missing=%r autopilot=%r" % (manifest_a.get("missing"), apa))
    missing_count = re.search(r"누락 (\d+)건", ra.stdout)
    check("E7-D07-OPTIONAL-COUNT stdout 누락 1건은 autopilot 을 제외한 수치",
          ra.returncode == 0 and missing_count is not None
          and int(missing_count.group(1)) == 1, repr(ra.stdout + ra.stderr))
    optional_count = re.search(r"선택부재 (\d+)건", ra.stdout)
    check("E8-D07-OPTIONAL-SUMMARY stdout 에 선택부재 1건을 별도 표시",
          ra.returncode == 0 and optional_count is not None
          and int(optional_count.group(1)) == 1, repr(ra.stdout + ra.stderr))
    rp, mp, _, _, app = run_d07_snapshot(autopilot=True)
    manifest_p = mp[0] if len(mp) == 1 else {}
    check("E9a-D07-OPTIONAL-PRESENT 양성 대조: autopilot 실재 시 보관",
          rp.returncode == 0
          and any(entry.get("source") == app for entry in manifest_p.get("files", [])),
          repr(manifest_p))
    check("E9b-D07-OPTIONAL-PRESENT 선택부재 목록도 빈 배열로 명시",
          manifest_p.get("optional_absent") == [], repr(manifest_p.get("optional_absent")))

    # 소스와 gen_root 를 명시해 직접 호출도 라이브 HOME 파생 경로에 닿지 않게 한다.
    dry_new = os.path.join(root, "d07-dry-new")
    ss.do_snapshot(sources=[], gen_root=dry_new, dry_run=True)
    check("E10-D07-DRY-MKDIR dry-run 은 없던 gen_root 를 만들지 않음",
          not os.path.exists(dry_new), dry_new)
    dry_existing = os.path.join(root, "d07-dry-existing")
    dry_tmp = os.path.join(dry_existing, ".tmp-%d-x" % os.getpid())
    os.makedirs(dry_tmp)
    # 살아 있는 pid 보호만 구현해도 초록이 되지 않도록 청소 호출 자체도 감시한다.
    with mock.patch.object(ss, "_cleanup_tmp", wraps=ss._cleanup_tmp) as cleanup:
        ss.do_snapshot(sources=[], gen_root=dry_existing, dry_run=True)
    check("E11-D07-DRY-CLEANUP dry-run 은 자기 pid 임시 세대를 삭제하지 않음",
          os.path.isdir(dry_tmp) and not cleanup.called,
          "%s, 청소 호출=%d" % (dry_tmp, cleanup.call_count))

    check("E12-D07-TMP-AGE-CONST 임시 세대 최소 보존 시간은 60초",
          getattr(ss, "TMP_MIN_AGE_SECS", None) == 60,
          repr(getattr(ss, "TMP_MIN_AGE_SECS", None)))
    cleanup_root = os.path.join(root, "d07-cleanup")
    live_name = ".tmp-%d-a" % os.getpid()
    live_tmp = os.path.join(cleanup_root, live_name)
    os.makedirs(live_tmp)
    old_time = time.time() - 3600
    os.utime(live_tmp, (old_time, old_time))
    dead_pid = None
    if os.name != "nt":
        # 프로세스를 만들거나 신호를 보내지 않고, 실제 부재가 확인된 pid 만 고른다.
        for candidate in range(99990, 0, -1):
            try:
                os.kill(candidate, 0)
            except ProcessLookupError:
                dead_pid = candidate
                break
            except PermissionError:
                continue
        if dead_pid is None:
            raise RuntimeError("D-07 검체용 죽은 pid 를 찾지 못함")
        old_name = ".tmp-%d-b" % dead_pid
        fresh_name = ".tmp-%d-c" % dead_pid
        old_tmp = os.path.join(cleanup_root, old_name)
        fresh_tmp = os.path.join(cleanup_root, fresh_name)
        os.makedirs(old_tmp)
        os.utime(old_tmp, (old_time, old_time))
        os.makedirs(fresh_tmp)
        now = time.time()
        os.utime(fresh_tmp, (now, now))
    removed = ss._cleanup_tmp(cleanup_root)
    check("E13-D07-TMP-LIVE 살아 있는 pid 는 mtime 1시간 전이어도 보호",
          os.path.isdir(live_tmp), "removed=%r" % removed)
    if os.name == "nt":
        print("SKIP E14~E16-D07-TMP-DEAD Windows 는 os.kill(pid, 0) 부재 탐색 제외")
    else:
        check("E14-D07-TMP-DEAD-OLD 죽은 pid 의 1시간 전 임시 세대는 삭제",
              not os.path.exists(old_tmp), "removed=%r" % removed)
        check("E15-D07-TMP-DEAD-FRESH 죽은 pid 라도 60초 미만은 보호",
              os.path.isdir(fresh_tmp), "removed=%r" % removed)
        check("E16-D07-TMP-REMOVED 반환 목록에는 죽은 pid 의 오래된 b 만 포함",
              removed == [old_name], repr(removed))
finally:
    shutil.rmtree(root, ignore_errors=True)

print("\n=== %d/%d PASS ===" % (checks - len(fails), checks))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("SNAPSHOT-ROOT-OK")
