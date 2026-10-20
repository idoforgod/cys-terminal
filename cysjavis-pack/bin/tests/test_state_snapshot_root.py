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
  F~H) B-317·B-320·B-321 전 레인 적재·정본 카드·레인별 풀 GC.
  R) B317-R1 — 부모(d478d97f) 판정 바닥(무작위 성질 · 부모 원문 exec) · 순서 규칙 하나(GC = card) ·
     레지스트리 문제 fail-closed(적재 = missing·INCOMPLETE / GC = 전 레인 48) · 스냅샷당 레인 해소 1회.

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


# ★B317-R1(codex M1): 부모(d478d97f) GC 판정 원문 — `git show d478d97f:cysjavis-pack/bin/javis_state_snapshot.py` 의
#   _parse_stamp·_gen_effective_dt·compute_gc 소스 구간(ast.get_source_segment)을 빈 줄 둘로 이어 붙인 바이트 그대로다.
#   R 계열은 이것을 exec 해 「수리본은 부모가 보관한 것을 하나도 지우지 않는다」를 부모 코드 자체와 대조한다(재구현 금지 —
#   기준선을 손으로 다시 쓰면 그 사본의 오류가 성질 검사를 통과시킨다). sha 핀(R0)이 원문 변조를 막는다.
PARENT_GC_SHA256 = "800ebd9e27efcab50f8037c48d58ca403ce944c5c6aaeb7cd2f833f6c251fbaf"
PARENT_GC_SRC = r'''def _parse_stamp(name):
    m = GEN_NAME_RE.match(name)
    if not m:
        return None
    return datetime.strptime(m.group(1), "%Y%m%dT%H%M%SZ").replace(tzinfo=timezone.utc)


def _gen_effective_dt(gen_root, name):
    """★E3(P2-5) 세대 실효 시각 — 이름의 명목 타임스탬프와 디렉터리 mtime 중 **더 나중**을 취한다.
    시계 역행 시 실제로 더 최근에 만든 세대가 이름은 lexically 작아지는데, mtime 이 이를 바로잡아 '최근'으로
    인식되게 한다(lexical 오선택·GC 오삭제 방어). 둘 다 없으면 None."""
    nominal = _parse_stamp(name)
    mt = None
    try:
        mt = datetime.fromtimestamp(os.path.getmtime(os.path.join(gen_root, name)), tz=timezone.utc)
    except OSError:
        pass
    cands = [d for d in (nominal, mt) if d is not None]
    return max(cands) if cands else None


def compute_gc(generations, now=None, dt_of=None, dt_eff=None):
    """보관/삭제 결정. (keep_set, delete_list) 반환. 순수함수(테스트 가능).
    ★E3(P2-5 + gemini W6): 명목 시각(dt_of=이름)과 실효 시각(dt_eff=mtime 병행)을 **둘 다 보호**한다(union).
    - 명목-recent(이름 최신) KEEP_RECENT 는 항상 보존 → cp/touch 로 과거 세대 mtime 이 오염돼도 **진짜 최근 세대를
      밀어내지 못한다**(gemini 오염 왜곡 차단).
    - 실효-recent(mtime 최신) KEEP_RECENT 도 보존 → 시계 역행으로 이름이 작아진 실 최근 세대가 보존된다(P2-5).
    dt_eff 미주입 시 dt_of 와 동일(하위호환·명목만). 오염 세대가 실효 집합에 추가로 잔존할 수 있으나(무해·상한
    2×KEEP_RECENT), 진짜 최근 세대의 오삭제는 발생하지 않는다."""
    now = now or _now_utc()
    _E0 = datetime.min.replace(tzinfo=timezone.utc)
    dt_of = dt_of or _parse_stamp        # 명목(이름)
    dt_eff = dt_eff or dt_of             # 실효(mtime 병행) — 미주입 시 명목과 동일
    keep = set()

    # 규칙 A: 최근 KEEP_RECENT — 명목 기준 ∪ 실효 기준(union → 명목-recent 절대 evict 안 됨·실효-recent 도 보존)
    for keyfn in (dt_of, dt_eff):
        ordered = sorted(generations, key=lambda n: (keyfn(n) or _E0), reverse=True)
        for name in ordered[:KEEP_RECENT]:
            keep.add(name)

    # 규칙 B: 최근 14일, 하루당 대표 1건 — 명목 시각 기준(날짜 귀속은 이름이 정직·오염 mtime 미의존)
    cutoff = now - timedelta(days=KEEP_DAILY_DAYS)
    day_rep = {}
    for name in sorted(generations, key=lambda n: (dt_of(n) or _E0), reverse=True):
        dt = dt_of(name)
        if dt is None or dt < cutoff:
            continue
        day = dt.strftime("%Y%m%d")
        if day not in day_rep:
            day_rep[day] = name
            keep.add(name)

    delete = [n for n in generations if n not in keep]
    return keep, sorted(delete)'''


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

    # ── F. ★B-317(CEO 결정 1·2) 전 레인 적재 — 선언 집합 경로 전량 출력 · 법 묶음 · 제외 · 여집합 ─────────
    #   밀폐 함대: 본부 팩 round(은퇴될 사본 카드·노드 TODO·법 묶음 2권·제외 미끼) + `~/_round` 라이브 카드 +
    #   등록 부서 2개(dept-1 실재 · dept-2 는 레지스트리에만 있고 팩 없음 = fail-closed 누락).
    def _w(path, text):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)

    def build_fleet(tag):
        h = tempfile.mkdtemp(prefix="snap-%s-" % tag, dir=root)
        c = os.path.join(h, ".cys")
        brd = os.path.join(c, "pack", "round")
        _w(os.path.join(brd, "SESSION_STATE.md"), "# base pack/round copy (retired lineage)\n")
        _w(os.path.join(brd, "MASTER_TODO.md"), "- [ ] base master\n")
        _w(os.path.join(brd, "wall-time_TODO.md"), "- [ ] '-wal' inside a name is not a WAL file\n")
        _w(os.path.join(brd, "CSO_STANDING_RULES.md"), "law vol 1 (unnumbered first member)\n")
        _w(os.path.join(brd, "CSO_STANDING_RULES_2.md"), "law vol 2\n")
        for bait in ("OLD.bak-1_TODO.md", "state.db", "state.db-wal", "state.db-shm", "x.sqlite"):
            _w(os.path.join(brd, bait), "bait\n")
        _w(os.path.join(h, "_round", "SESSION_STATE.md"), "# LIVE base card (canonical, base lane)\n")
        _w(os.path.join(h, "_round", "README-NO-TODO-FILES-HERE.md"), "no todo here\n")
        d1 = os.path.join(c, "pack-dept-dept-1")
        _w(os.path.join(d1, "round", "SESSION_STATE.md"), "# dept-1 LIVE card (canonical, dept lane)\n")
        _w(os.path.join(d1, "round", "WORKER_TODO.md"), "- [ ] dept-1 worker\n")
        d2 = os.path.join(c, "pack-dept-dept-2")          # registered, never created
        _w(os.path.join(c, "depts.json"), json.dumps({"depts": {
            "dept-1": {"pack_dir": d1, "socket": os.path.join(h, ".local", "state", "cys-dept-dept-1", "cys.sock")},
            "dept-2": {"pack_dir": d2, "socket": os.path.join(h, ".local", "state", "cys-dept-dept-2", "cys.sock")}}}))
        _w(os.path.join(h, ".local", "state", "cys", "topology.json"), "{}\n")
        return h, brd, d1, d2

    def fleet_env(h, pack):
        return {"HOME": h, "USERPROFILE": h, "LOCALAPPDATA": os.path.join(h, "AppData", "Local"),
                "PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": os.environ.get("LANG", "C.UTF-8"),
                "CYS_PACK_DIR": pack}

    def run_tool(h, pack, *args):
        return subprocess.run([PY, MOD] + list(args), capture_output=True, text=True, encoding="utf-8",
                              errors="replace", env=fleet_env(h, pack), cwd="/", timeout=120)

    fh, fbrd, fd1, fd2 = build_fleet("fleet")
    live_card = os.path.join(fh, "_round", "SESSION_STATE.md")
    rdry = run_tool(fh, os.path.join(fh, ".cys", "pack"), "snapshot", "--dry-run")
    plus = [l.strip()[2:].rsplit("  (", 1)[0] for l in rdry.stdout.splitlines() if l.strip().startswith("+ ")]
    absent = [l.split("(없음) ", 1)[1] for l in rdry.stdout.splitlines() if "(없음) " in l]
    print("  F dry-run declared-present paths (printed, not counted):")
    for p in plus:
        print("    + " + p)
    for p in absent:
        print("    - " + p)
    want_present = [os.path.join(fbrd, n) for n in ("SESSION_STATE.md", "MASTER_TODO.md", "wall-time_TODO.md",
                                                   "CSO_STANDING_RULES.md", "CSO_STANDING_RULES_2.md")]
    want_present += [live_card, os.path.join(fd1, "round", "SESSION_STATE.md"),
                     os.path.join(fd1, "round", "WORKER_TODO.md")]
    check("F1 ★dry-run: 전 레인 round(본부·dept-1)·본부 라이브 카드·법 묶음 2권(무번호 첫 권 포함)이 선언·실재로 출력",
          rdry.returncode == 0 and all(p in plus for p in want_present),
          "missing from output: %r" % [p for p in want_present if p not in plus])
    check("F2 ★fail-closed: 등록됐으나 팩이 없는 dept-2 의 카드·TODO 패턴이 (없음) 으로 출력(선언이 적재를 뜻한다)",
          os.path.join(fd2, "round", "SESSION_STATE.md") in absent
          and os.path.join(fd2, "round", "*_TODO.md") in absent, repr(absent))
    check("F3 dry-run 이 레인별 정본 카드 지도를 출력(본부 -> ~/_round · dept-1 -> <pack>/round)",
          ("* base -> %s" % live_card) in rdry.stdout
          and ("* dept-1 -> %s" % os.path.join(fd1, "round", "SESSION_STATE.md")) in rdry.stdout, rdry.stdout[-600:])
    bad = [p for p in plus if ss._capture_excluded(p)]
    check("F4 ★B-317-AMEND 결박: 선언 집합에 .db/.sqlite/-wal/-shm/.bak 0건 · '-wal' 을 이름 안에 가진 TODO 는 적재",
          not bad and os.path.join(fbrd, "OLD.bak-1_TODO.md") not in plus
          and os.path.join(fbrd, "wall-time_TODO.md") in plus, repr(bad))
    check("F5 제외 판정 단위: 끝 일치만 제외(a.db a.sqlite a-wal a-shm a.bak a.md.bak-1 제외 · wall-time_TODO.md 비제외)",
          all(ss._capture_excluded(n) for n in ("a.db", "a.sqlite", "a-wal", "a-shm", "a.bak", "a.md.bak-1"))
          and not ss._capture_excluded("wall-time_TODO.md"))
    comp = sorted(n for n in os.listdir(fbrd) if os.path.join(fbrd, n) not in plus)
    print("  F complement of the base round (present, not declared): %r" % comp)
    check("F6 ★여집합(AMEND-3 방법): 본부 round 에서 선언 밖 파일 = 미끼 5종 정확히(이름으로 대조)",
          comp == sorted(["OLD.bak-1_TODO.md", "state.db", "state.db-wal", "state.db-shm", "x.sqlite"]), repr(comp))
    # F8 (R2): 본부 카드 자리(~/_round)는 이 기계 배치의 사실이다 — 그 디렉터리가 없는 설치(본부 master 가 홈이 아닌
    #     곳에서 도는 사용자)에서 매 세대 거짓 「누락」을 만들지 않는다. 디렉터리가 있는데 카드가 없으면 누락(fail-closed).
    nh, _nb, _n1, _n2 = build_fleet("noround")
    shutil.rmtree(os.path.join(nh, "_round"))
    rno = run_tool(nh, os.path.join(nh, ".cys", "pack"), "snapshot", "--dry-run")
    check("F8a (R2) ~/_round 디렉터리가 없으면 본부 카드를 선언하지 않는다(거짓 누락 0 · 카드 지도에 base 없음)",
          rno.returncode == 0 and os.path.join(nh, "_round", "SESSION_STATE.md") not in rno.stdout
          and "* base -> " not in rno.stdout, rno.stdout[-400:])
    os.makedirs(os.path.join(nh, "_round"))
    rno2 = run_tool(nh, os.path.join(nh, ".cys", "pack"), "snapshot", "--dry-run")
    check("F8b (R2) ~/_round 가 있는데 카드가 없으면 (없음) 으로 선언(fail-closed)",
          ("(없음) %s" % os.path.join(nh, "_round", "SESSION_STATE.md")) in rno2.stdout, rno2.stdout[-400:])
    check("F7 ~/_round 는 레인 round 가 아니다 — 카드만 선언(TODO 패턴 누락을 영구히 만들지 않는다)",
          os.path.join(fh, "_round", "*_TODO.md") not in absent and os.path.join(fh, "_round", "*_TODO.md") not in plus,
          repr(absent))

    # ── G. ★결정 7 복원 시뮬레이션 — 복원된 카드 sha == 원본 정본 카드(본부·부서 레인) ─────────────────
    def _sha(p):
        import hashlib
        with open(p, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()

    def newest_manifest(h):
        gr = os.path.join(h, ".cys", "state-generations")
        gens = sorted(g for g in os.listdir(gr) if not g.startswith(".tmp-")) if os.path.isdir(gr) else []
        if not gens:
            return None, None
        with open(os.path.join(gr, gens[-1], "manifest.json"), encoding="utf-8") as f:
            return gens[-1], json.load(f)

    rb = run_tool(fh, os.path.join(fh, ".cys", "pack"), "snapshot")      # 본부 데몬 잡 형상
    gname, man = newest_manifest(fh)
    srcs_base = sorted(e["source"] for e in (man or {}).get("files", []))
    check("G0 본부 발화 세대 생성(rc 0)", rb.returncode == 0 and gname is not None, (rb.stdout + rb.stderr)[-300:])
    live_entry = [e for e in (man or {}).get("files", []) if e.get("source") == live_card]
    check("G1 ★세대에 본부 라이브 카드(~/_round/SESSION_STATE.md)가 담겼다(종전: 담기지 않음 = 복원 정본 부재)",
          len(live_entry) == 1, "sources=%r" % srcs_base)
    restore_dir = tempfile.mkdtemp(prefix="snap-restore-", dir=root)
    rc_card = run_tool(fh, os.path.join(fh, ".cys", "pack"), "card", "--lane", "base")
    restored_sha = None
    if rc_card.returncode == 0 and rc_card.stdout.strip():
        stored, _dg, src = rc_card.stdout.strip().split("\t")
        dst = os.path.join(restore_dir, "SESSION_STATE.md")
        shutil.copyfile(stored, dst)                      # 복원 = 사람이 출력된 저장 파일을 복사
        restored_sha = _sha(dst)
    check("G2 ★복원 시뮬: card --lane base 로 복원한 카드 sha == 원본 본부 정본 카드 sha(≠ 본부 팩 round 사본)",
          restored_sha == _sha(live_card) and restored_sha != _sha(os.path.join(fbrd, "SESSION_STATE.md")),
          "rc=%s out=%r err=%r" % (rc_card.returncode, rc_card.stdout[-200:], rc_card.stderr[-300:]))
    rc_d1 = run_tool(fh, os.path.join(fh, ".cys", "pack"), "card", "--lane", "dept-1")
    d1_card = os.path.join(fd1, "round", "SESSION_STATE.md")
    d1_ok = False
    if rc_d1.returncode == 0 and rc_d1.stdout.strip():
        stored, _dg, src = rc_d1.stdout.strip().split("\t")
        dst = os.path.join(restore_dir, "dept-1-SESSION_STATE.md")
        shutil.copyfile(stored, dst)
        d1_ok = (src == d1_card and _sha(dst) == _sha(d1_card))
    check("G3 ★HEAD 범위: 부서 레인 복원은 <pack>/round/SESSION_STATE.md 를 고른다(부서 카드는 그대로 정본)",
          d1_ok, "rc=%s out=%r err=%r" % (rc_d1.returncode, rc_d1.stdout[-200:], rc_d1.stderr[-300:]))
    check("G4 fail-closed 기록: 팩 없는 dept-2 의 카드·TODO 패턴이 manifest missing 에(세대는 불완전으로 표시)",
          os.path.join(fd2, "round", "SESSION_STATE.md") in (man or {}).get("missing", [])
          and os.path.join(fd2, "round", "*_TODO.md") in (man or {}).get("missing", []), repr((man or {}).get("missing")))
    rd1 = run_tool(fh, fd1, "snapshot")                                  # 부서 데몬 잡 형상(같은 함대)
    _g2, man2 = newest_manifest(fh)
    srcs_dept = sorted(e["source"] for e in (man2 or {}).get("files", []))
    check("G5 ★적재가 발화 레인에 의존하지 않는다: 본부 발화 세대 소스 집합 == 부서 발화 세대 소스 집합",
          rd1.returncode == 0 and srcs_base == srcs_dept and srcs_base,
          "only-base=%r only-dept=%r" % (sorted(set(srcs_base) - set(srcs_dept)), sorted(set(srcs_dept) - set(srcs_base))))
    # 종전 형상 세대(canonical_cards 없음 · SESSION_STATE.md 1건) — 복원 경로는 basename 으로 대체하지 않는다.
    gr_f = os.path.join(fh, ".cys", "state-generations")
    old = os.path.join(gr_f, "20200101T000000Z")
    os.makedirs(old)
    _w(os.path.join(old, "SESSION_STATE.md"), "# old lineage card\n")
    _w(os.path.join(old, "manifest.json"), json.dumps({"files": [{
        "source": os.path.join(fbrd, "SESSION_STATE.md"), "stored_as": "SESSION_STATE.md",
        "sha256": _sha(os.path.join(old, "SESSION_STATE.md")), "size": 19}]}))
    rc_old = run_tool(fh, os.path.join(fh, ".cys", "pack"), "card", "--gen", "20200101T000000Z")
    check("G6 ★정본 기록 없는 세대에서 card 는 rc 1·출력 0(유일한 SESSION_STATE.md 로 대체하지 않는다)",
          rc_old.returncode == 1 and rc_old.stdout.strip() == "", "rc=%s out=%r" % (rc_old.returncode, rc_old.stdout))
    rc_unk = run_tool(fh, os.path.join(fh, ".cys", "pack"), "card", "--gen", gname, "--lane", "dept-9")
    check("G7 등록되지 않은 레인 이름은 rc 1(추측 대체 없음)", rc_unk.returncode == 1 and not rc_unk.stdout.strip())
    # G9 (R1): `~/_round` 가 본부 팩 round 를 가리키는 심링크여도 본부 정본 카드는 이름으로 고를 수 있어야 한다
    #     (실경로로 중복 제거하면 카드 경로 자체가 소스에서 빠져 card 가 rc 1 이 된다).
    if os.name != "nt":
        sh, sbrd, _sd1, _sd2 = build_fleet("symlink")
        shutil.rmtree(os.path.join(sh, "_round"))
        os.symlink(sbrd, os.path.join(sh, "_round"))
        rs = run_tool(sh, os.path.join(sh, ".cys", "pack"), "snapshot")
        rcs = run_tool(sh, os.path.join(sh, ".cys", "pack"), "card", "--lane", "base")
        check("G9 (R1) ~/_round 심링크 형상에서도 card --lane base 가 rc 0 · 원 경로 = ~/_round/SESSION_STATE.md",
              rs.returncode == 0 and rcs.returncode == 0
              and rcs.stdout.strip().split("\t")[-1] == os.path.join(sh, "_round", "SESSION_STATE.md"),
              "rc=%s/%s out=%r err=%r" % (rs.returncode, rcs.returncode, rcs.stdout[-200:], rcs.stderr[-200:]))
    if live_entry:
        tampered = os.path.join(gr_f, gname, live_entry[0]["stored_as"])
        with open(tampered, "ab") as f:
            f.write(b"x")
        rc_t = run_tool(fh, os.path.join(fh, ".cys", "pack"), "card", "--gen", gname, "--lane", "base")
        check("G8 저장 카드가 변조되면 card 는 rc 1(해시 불일치 · 복원 금지)",
              rc_t.returncode == 1 and not rc_t.stdout.strip(), "rc=%s" % rc_t.returncode)

    # ── H. ★B-321 레인별 풀 GC(HEAD 형식 결재 · CEO 잔여 상한) + B-320 접미 ───────────────────────────
    from datetime import datetime as _dtm, timedelta as _td, timezone as _tz
    LANES8 = ("base",) + tuple("dept-%d" % i for i in range(1, 8))
    t0 = _dtm(2026, 10, 19, 0, 0, tzinfo=_tz.utc)

    def stamp(t):
        return t.strftime("%Y%m%dT%H%M%SZ")

    # H1 전이: 같은 날 10분 간격 60세대, 가장 오래된 6개가 본부(나머지 54개는 부서 단일 레인)
    n60 = [stamp(t0 + _td(minutes=10 * i)) for i in range(60)]
    cov60 = {n: (("base",) if i < 6 else ("dept-%d" % (i % 7 + 1),)) for i, n in enumerate(n60)}
    now60 = t0 + _td(hours=10)
    k_lane, _ = ss.compute_gc(n60, now=now60, key_of=lambda n: cov60[n])
    k_one, _ = ss.compute_gc(n60, now=now60)
    check("H1 ★전이: 단일 레인 54개 사이 본부 6개 → 레인별 풀이면 6/6 보관",
          all(n in k_lane for n in n60[:6]), "base kept %d/6" % sum(n in k_lane for n in n60[:6]))
    check("H1c 대조(구별력): 단일 풀(종전 판정)이면 같은 입력에서 본부 세대가 밀려난다",
          not all(n in k_one for n in n60[:6]), "base kept %d/6" % sum(n in k_one for n in n60[:6]))

    def roll(store, cov, newer, newcov, cap_of=None):
        """newer 를 하나씩 추가하며 매번 GC(스냅샷마다 GC 하는 실제 순서) → 마지막 store."""
        store, cov = list(store), dict(cov)
        for t, n in newer:
            store.append(n)
            cov[n] = newcov
            keep, _ = ss.compute_gc(store, now=t, key_of=lambda x: cov[x], cap_of=cap_of)
            store = [x for x in store if x in keep]
        return store

    old60 = [stamp(t0 + _td(hours=i)) for i in range(60)]
    covold = {n: ((LANES8[(i % 8)]),) for i, n in enumerate(old60)}
    newer120 = [(t0 + _td(days=3, hours=6 * j), stamp(t0 + _td(days=3, hours=6 * j))) for j in range(120)]
    st2 = roll(old60, covold, newer120, LANES8)
    check("H2 ★노화: 단일 레인 60세대 뒤 전 레인 120세대(30일) → 옛 단일 레인 세대 0개 잔존",
          not any(n in st2 for n in old60), "old left %d, store %d" % (sum(n in st2 for n in old60), len(st2)))
    all8 = [stamp(t0 + _td(hours=6 * i)) for i in range(60)]
    cov8 = {n: LANES8 for n in all8}
    newer9 = [(t0 + _td(days=15, hours=6 * j), stamp(t0 + _td(days=15, hours=6 * j))) for j in range(120)]
    st3 = roll(all8, cov8, newer9, LANES8 + ("dept-8",))
    check("H3 ★새 레인 등장: 8레인 세대 뒤 9레인 세대 120개 → 8레인 세대 동결 블록 0개",
          not any(n in st3 for n in all8), "8-lane gens left %d" % sum(n in st3 for n in all8))

    # H4 삭제(부재) 레인: dept-X 를 담은 세대 30개(2시간 간격) → 그 뒤 120세대는 dept-X 미포함 · 레지스트리에 dept-X 없음.
    #     (뒤 세대는 앞 세대가 끝난 뒤에 시작한다 — 이름이 겹치면 검체가 자기 입력을 덮는다)
    gx = [stamp(t0 + _td(hours=2 * i)) for i in range(30)]
    later = [(t0 + _td(days=3, hours=2 * j), stamp(t0 + _td(days=3, hours=2 * j))) for j in range(120)]
    cap_absent = lambda k: ss.ABSENT_LANE_KEEP if k == "dept-X" else ss.KEEP_RECENT
    st4 = roll(gx, {n: ("dept-X",) for n in gx}, later, ("base",), cap_of=cap_absent)
    check("H4 ★부재 레인 잔여 상한 1: dept-X 만 담은 세대는 최신 1건만 남는다(최종 상태 보관 · 이력 미보관)",
          [n for n in st4 if n in gx] == [gx[-1]], "kept %r" % [n for n in st4 if n in gx])
    # (ii) 부재 레인을 담은 나머지 세대는 **자기가 속한 다른 풀**의 정책으로만 남는다:
    #      공유 세대(base+dept-X)에서 보관 = (base 풀만으로 보관될 것) ∪ {dept-X 최신 1건}.
    st4s = roll(gx, {n: ("base", "dept-X") for n in gx}, later, ("base",), cap_of=cap_absent)
    st4b = roll(gx, {n: ("base",) for n in gx}, later, ("base",))
    want4 = sorted(set(n for n in st4b if n in gx) | {gx[-1]})
    check("H4b ★공유 세대: 보관 = base 풀 자체 보관분 ∪ dept-X 최신 1건(나머지는 다른 풀에서 노화)",
          sorted(n for n in st4s if n in gx) == want4,
          "kept %r want %r" % (sorted(n for n in st4s if n in gx), want4))
    st4p = roll(gx, {n: ("dept-X",) for n in gx}, later, ("base",))
    check("H4c 대조: 상한 48(현존 레인)이면 dept-X 세대 30/30 보관",
          sum(n in st4p for n in gx) == 30, "kept %d" % sum(n in st4p for n in gx))

    # H5 레지스트리 판독 → 풀 상한(_lane_caps · 라이브 depts.json 무접촉 — 밀폐 사본만)
    regh = tempfile.mkdtemp(prefix="snap-reg-", dir=root)
    pk_present = os.path.join(regh, ".cys", "pack-dept-dept-1")
    pk_absent = os.path.join(regh, ".cys", "pack-dept-dept-9")
    dj = os.path.join(regh, ".cys", "depts.json")
    base_rd = os.path.join(regh, ".cys", "pack", "round")
    _w(dj, json.dumps({"depts": {"dept-1": {"pack_dir": pk_present}}}))
    cap, ok = ss._lane_caps(home=regh, depts_json=dj)
    check("H5a 현존 레인(레지스트리 pack_dir/round) 상한 48", ok and cap(os.path.join(pk_present, "round")) == 48)
    check("H5b 부재 레인(레지스트리에 없는 팩 round) 상한 1", cap(os.path.join(pk_absent, "round")) == 1)
    check("H5c ★본부는 결코 부재가 아니다(부서 항목이 아님): ~/.cys/pack/round·~/_round 상한 48",
          cap(base_rd) == 48 and cap(os.path.join(regh, "_round")) == 48)
    _w(dj, json.dumps({"depts": {}}))
    cap_e, ok_e = ss._lane_caps(home=regh, depts_json=dj)
    check("H5d 빈 레지스트리(읽힘)에서도 본부는 48 · 부서 팩 round 는 1",
          ok_e and cap_e(base_rd) == 48 and cap_e(os.path.join(pk_present, "round")) == 1)
    bad_regs = {"missing": None, "unparsable": "{not json", "depts-not-a-map": json.dumps({"depts": []}),
                "no-depts-key": json.dumps({"x": 1}), "entry-without-pack_dir": json.dumps({"depts": {"d": {"socket": "s"}}})}
    for label, body in sorted(bad_regs.items()):
        if body is None:
            if os.path.exists(dj):
                os.remove(dj)
        else:
            _w(dj, body)
        cap_b, ok_b = ss._lane_caps(home=regh, depts_json=dj)
        check("H5e ★레지스트리 판독 실패(%s)는 부재가 아니다 — 전 레인 48" % label,
              (not ok_b) and cap_b(os.path.join(pk_absent, "round")) == 48 and cap_b(base_rd) == 48)
    check("H5f 레인이 아닌 풀('' · '?')은 48", cap("") == 48 and cap("?") == 48)
    # H9 (R2): 부재 판정은 HOME 절대경로가 아니라 구조로 한다 — HOME 이 옮겨진 뒤 옛 세대의 키(다른 접두)에서도
    #     본부는 결코 부재가 아니고, 등록된 부서는 팩 이름으로 현존이며, `_round` 레인(프로젝트)은 부서가 아니다.
    _w(dj, json.dumps({"depts": {"dept-1": {"pack_dir": pk_present}}}))
    cap9, _ok9 = ss._lane_caps(home=regh, depts_json=dj)
    oldh = os.path.join(root, "old-home-before-move")
    check("H9a (R2) HOME 이동 뒤 옛 접두의 본부 키(.cys/pack/round)도 48 — 본부는 결코 부재가 아니다",
          cap9(os.path.join(oldh, ".cys", "pack", "round")) == 48)
    check("H9b (R2) HOME 이동 뒤 옛 접두의 등록 부서 키는 팩 이름으로 현존(48) · 미등록 부서 팩은 1",
          cap9(os.path.join(oldh, ".cys", "pack-dept-dept-1", "round")) == 48
          and cap9(os.path.join(oldh, ".cys", "pack-dept-dept-9", "round")) == 1)
    check("H9c (R2) `_round` 레인(본부 카드 자리·JAVIS_ROOT 프로젝트)은 부서가 아니다 — 48",
          cap9(os.path.join(oldh, "_round")) == 48 and cap9(os.path.join(root, "proj", "_round")) == 48)

    # H6 B-320 같은 스탬프 충돌쌍 — 판정이 입력 순서가 아니라 정책(승격 순서)으로 정해진다.
    a = "20261015T120000Z"
    a1 = a + "-1"
    newer48 = [stamp(_dtm(2026, 10, 16, tzinfo=_tz.utc) + _td(minutes=i)) for i in range(48)]
    now6 = _dtm(2026, 10, 18, tzinfo=_tz.utc)
    k_ab, _ = ss.compute_gc([a, a1] + newer48, now=now6)
    k_ba, _ = ss.compute_gc([a1, a] + newer48, now=now6)
    # ★B317-R1 M1(codex): 종전 H6 은 무접미(부모가 보관하던 대표)의 **삭제**를 요구했다 — 잠근 기준 위반을 축복한 검체였다.
    #   순서 규칙의 승자(-1)는 그대로 대표이고, 부모의 승자(무접미)는 부모 바닥이 보관한다 — 둘 다 남는다.
    check("H6 ★B-320 + R1 M1: 같은 스탬프 쌍 — 입력 순서와 무관 · 순서 규칙의 승자(-1)와 부모의 승자(무접미) 둘 다 보관",
          k_ab == k_ba and a1 in k_ab and a in k_ab, "ab=%r/%r ba=%r/%r" % (a1 in k_ab, a in k_ab, a1 in k_ba, a in k_ba))
    check("H6b _parse_stamp(<s>-1) > _parse_stamp(<s>) · 같은 초",
          ss._parse_stamp(a1) > ss._parse_stamp(a)
          and ss._parse_stamp(a1).replace(microsecond=0) == ss._parse_stamp(a))

    # H7 do_gc 배선 — 실제 manifest 판독(_coverage_keys)·레지스트리(_lane_caps) 경유(밀폐 gen_root·사본 레지스트리)
    wh = tempfile.mkdtemp(prefix="snap-wire-", dir=root)
    wgr = os.path.join(wh, "gen")
    wpk = {ln: (os.path.join(wh, ".cys", "pack") if ln == "base" else os.path.join(wh, ".cys", "pack-dept-" + ln))
           for ln in LANES8}
    _w(os.path.join(wh, ".cys", "depts.json"), json.dumps({"depts": {
        ln: {"pack_dir": wpk[ln]} for ln in LANES8 if ln != "base"}}))
    for i, n in enumerate(n60):
        lane = cov60[n][0]
        src = os.path.join(wpk[lane], "round", "SESSION_STATE.md")
        _w(os.path.join(wgr, n, "manifest.json"), json.dumps({"files": [
            {"source": src, "stored_as": "SESSION_STATE.md", "sha256": "0", "size": 0}]}))
    wkeep, wdel = ss.do_gc(gen_root=wgr, dry_run=True, home=wh, depts_json=os.path.join(wh, ".cys", "depts.json"))
    check("H7 ★do_gc 배선: 실제 manifest 에서 레인을 읽어 본부 6/6 보관(삭제 목록에 본부 0)",
          not any(n in wdel for n in n60[:6]), "deleted base %r" % [n for n in n60[:6] if n in wdel])
    # 같은 저장소에 레지스트리 밖 레인(dept-9) 세대 3개 → do_gc 가 최신 1개만 남긴다
    x9 = [stamp(t0 + _td(minutes=1 + 10 * i)) for i in range(3)]
    for n in x9:
        _w(os.path.join(wgr, n, "manifest.json"), json.dumps({"files": [{
            "source": os.path.join(wh, ".cys", "pack-dept-dept-9", "round", "WORKER_TODO.md"),
            "stored_as": "WORKER_TODO.md", "sha256": "0", "size": 0}]}))
    wkeep2, wdel2 = ss.do_gc(gen_root=wgr, dry_run=True, home=wh, depts_json=os.path.join(wh, ".cys", "depts.json"))
    check("H7b ★do_gc 배선: 레지스트리 밖 레인 세대는 최신 1건만 보관",
          [n for n in x9 if n in wkeep2] == [x9[-1]], "kept %r" % [n for n in x9 if n in wkeep2])
    os.remove(os.path.join(wh, ".cys", "depts.json"))
    wkeep3, wdel3 = ss.do_gc(gen_root=wgr, dry_run=True, home=wh, depts_json=os.path.join(wh, ".cys", "depts.json"))
    check("H7c ★do_gc 배선: 레지스트리 판독 불가면 그 세대 3건 모두 보관(부재 판정 안 함)",
          all(n in wkeep3 for n in x9), "kept %r" % [n for n in x9 if n in wkeep3])
    # H8 (R1): 레인 패턴 이름이라도 round/_round 밖(예: 기억 저장소의 x_TODO.md)은 레인이 아니다 —
    #     가짜 레인 풀이 부재 상한 1 을 받아 그 세대들을 지우면 안 된다.
    mh = tempfile.mkdtemp(prefix="snap-mem-", dir=root)
    mgr = os.path.join(mh, "gen")
    _w(os.path.join(mh, ".cys", "depts.json"), json.dumps({"depts": {}}))
    m3 = [stamp(t0 + _td(minutes=i)) for i in range(3)]
    for n in m3:
        _w(os.path.join(mgr, n, "manifest.json"), json.dumps({"files": [{
            "source": os.path.join(mh, ".claude", "projects", "p", "memory", "notes_TODO.md"),
            "stored_as": "notes_TODO.md", "sha256": "0", "size": 0}]}))
    mkeep, _ = ss.do_gc(gen_root=mgr, dry_run=True, home=mh, depts_json=os.path.join(mh, ".cys", "depts.json"))
    check("H8 (R1) round 밖의 레인 패턴 파일은 레인 키가 아니다(기억 x_TODO.md 세대 3건 모두 보관)",
          all(n in mkeep for n in m3), "kept %r" % [n for n in m3 if n in mkeep])

    # ── R. ★B317-R1 수리 — codex R1 M1·M2·M3 + HEAD/CEO 추가(순서 규칙 하나 · 양쪽 fail-closed) + gemini minor ──
    import contextlib
    import hashlib
    import io
    import random as _rnd
    check("R0 부모(d478d97f) GC 판정 원문 sha 핀 — R1~R3 의 기준선은 부모 코드 그 자체다",
          hashlib.sha256(PARENT_GC_SRC.encode("utf-8")).hexdigest() == PARENT_GC_SHA256)
    _pns = {"datetime": _dtm, "timedelta": _td, "timezone": _tz, "os": os, "GEN_NAME_RE": ss.GEN_NAME_RE,
            "KEEP_RECENT": 48, "KEEP_DAILY_DAYS": 14, "_now_utc": ss._now_utc}
    exec(PARENT_GC_SRC, _pns)
    p_parse, p_eff, p_gc = _pns["_parse_stamp"], _pns["_gen_effective_dt"], _pns["compute_gc"]
    nowr = _dtm(2026, 10, 20, 12, 0, tzinfo=_tz.utc)

    # R1 codex 반례(RETENTION-COUNTEREXAMPLE.json): 어제(10-19) 같은 초 쌍 + 오늘 48세대. 부모는 무접미를 일별 대표로
    #    보관하고 -1 을 지웠다. 140e0385 는 접미 순서로 대표를 -1 로 바꿔 무접미를 지웠다(같은 초라도 내용은 다르다).
    cx = ["20261019T120000Z", "20261019T120000Z-1"] + [
        stamp(_dtm(2026, 10, 20, 11, 13, tzinfo=_tz.utc) + _td(minutes=i)) for i in range(48)]
    pk_cx, _ = p_gc(sorted(cx), now=nowr)
    nk_cx, _ = ss.compute_gc(sorted(cx), now=nowr)
    check("R1 ★M1 codex 반례: 부모가 보관한 세대를 하나도 지우지 않는다(10-19 무접미 대표 보관)",
          len(cx) == 50 and pk_cx <= set(nk_cx) and "20261019T120000Z" in nk_cx,
          "parent-kept-now-deleted=%r" % sorted(pk_cx - set(nk_cx)))

    # R2 무작위 성질(시드 고정 400 저장소 · 전 레인 현존 · 명목 시각): 부모 보관 ⊆ 수리본 보관.
    LANES5 = ("base",) + tuple("dept-%d" % i for i in range(1, 5))

    def rand_store(rng, now):
        names = set()
        for _ in range(rng.randint(20, 70)):
            s = stamp(now - _td(seconds=rng.randint(0, 20 * 86400)))
            g = rng.choice((1, 1, 1, 1, 2, 2, 3, 5, 12))
            start = rng.choice((0, 0, 1, 8))      # 0 = 무접미부터 · 그 밖 = 앞 이름이 지워진 뒤의 접미만(-8,-9,-10…)
            for k in range(start, start + g):
                names.add(s if k == 0 else "%s-%d" % (s, k))
        return sorted(names)                       # list_generations 와 같은 문자열 정렬(부모 do_gc 의 입력)

    viol2, sfx2, tot2, odep2 = [], 0, 0, 0
    for seed in range(400):
        rng = _rnd.Random(seed)
        st = rand_store(rng, nowr)
        cov = {n: tuple(sorted(rng.sample(LANES5, rng.randint(1, len(LANES5))))) for n in st}
        pk, _ = p_gc(st, now=nowr)
        nk, _ = ss.compute_gc(st, now=nowr, key_of=lambda n: cov[n])
        shuf = list(st)
        rng.shuffle(shuf)
        nk2, _ = ss.compute_gc(shuf, now=nowr, key_of=lambda n: cov[n])
        tot2 += len(st)
        sfx2 += sum(1 for n in st if "-" in n)
        if not pk <= set(nk):
            viol2.append((seed, sorted(pk - set(nk))[:2]))
        if set(nk) != set(nk2):
            odep2 += 1
    print("  R2 stores 400 · names %d · suffixed %d · stores where the patch deletes a parent-kept gen: %d"
          % (tot2, sfx2, len(viol2)))
    check("R2 ★M1 성질(무작위 400 · 전 레인 현존 · 명목): 부모 보관 ⊆ 수리본 보관", sfx2 > 0 and not viol2,
          repr(viol2[:3]))
    check("R2b B-320: 판정이 입력 순서와 무관(섞은 입력 = 정렬 입력)", odep2 == 0, "%d stores differ" % odep2)

    # R3 디스크 성질(do_gc 실배선 · 실 manifest · 실 mtime): 부모 do_gc 판정 = 부모 compute_gc(list_generations,
    #    부모 _gen_effective_dt) ⊆ 수리본 do_gc 보관. mtime 은 이름 앞·이름 직후(접미 µs 창)·한참 뒤(시계 역행·touch)를 섞는다.
    dh = tempfile.mkdtemp(prefix="snap-r1prop-", dir=root)
    dpk = {ln: (os.path.join(dh, ".cys", "pack") if ln == "base" else os.path.join(dh, ".cys", "pack-dept-" + ln))
           for ln in LANES5}
    ddj = os.path.join(dh, ".cys", "depts.json")
    _w(ddj, json.dumps({"depts": {ln: {"pack_dir": dpk[ln]} for ln in LANES5 if ln != "base"}}))

    def materialize(gr, st, cov, mt):
        for n in st:
            _w(os.path.join(gr, n, "manifest.json"), json.dumps({"files": [
                {"source": os.path.join(dpk[ln], "round", "SESSION_STATE.md"), "stored_as": "x", "sha256": "0",
                 "size": 0} for ln in cov[n]]}))
            os.utime(os.path.join(gr, n), (mt[n], mt[n]))

    def disk_pair(gr):
        gens = ss.list_generations(gr)
        pk, _ = p_gc(gens, now=nowr, dt_of=p_parse, dt_eff=lambda n: p_eff(gr, n))
        with contextlib.redirect_stdout(io.StringIO()):
            nk, _ = ss.do_gc(gen_root=gr, dry_run=True, home=dh, depts_json=ddj)
        return pk, set(nk)

    viol3 = []
    os.environ["_CYS_SNAPSHOT_NOW"] = str(nowr.timestamp())   # do_gc 의 now(_now_utc) = 부모 대조의 now
    try:
        for seed in range(40):
            rng = _rnd.Random(1000 + seed)
            st = rand_store(rng, nowr)
            cov = {n: tuple(sorted(rng.sample(LANES5, rng.randint(1, len(LANES5))))) for n in st}
            mt = {}
            for n in st:
                t, r = p_parse(n).timestamp(), rng.random()
                mt[n] = (t - rng.uniform(0, 5) if r < 0.5 else t + rng.uniform(0, 3e-5) if r < 0.65
                         else min(nowr.timestamp(), t + rng.uniform(1, 3 * 86400)) if r < 0.8 else t)
            gr = os.path.join(dh, "g%02d" % seed)
            materialize(gr, st, cov, mt)
            pk, nk = disk_pair(gr)
            if not pk <= nk:
                viol3.append((seed, sorted(pk - nk)[:2]))
        grx = os.path.join(dh, "codex")
        materialize(grx, sorted(cx), {n: LANES5 for n in cx}, {n: p_parse(n).timestamp() - 1 for n in cx})
        pkx, nkx = disk_pair(grx)
        # R3c 결정론 검체(바닥의 실효 절반): 옛 이름 n 의 mtime 이 Q+2µs(손댄 mtime) · 같은 초 Q 의 x=Q-5 는 이름이 실효.
        #     부모 실효 순서: 최근 47 > n(Q+2µs) > x(Q) → n 보관. 이 판의 순서: 최근 47 > x(Q+5µs) > n → n 은 규칙 A 밖이고
        #     14일 밖이라 일별 대표도 아니다. 부모 바닥이 실효(mtime) 판정까지 재현해야만 n 이 남는다.
        gre = os.path.join(dh, "effhalf")
        q = nowr - _td(days=2)
        recent47 = [stamp(nowr - _td(minutes=i + 1)) for i in range(47)]
        x_e, n_e = stamp(q) + "-5", stamp(nowr - _td(days=30))
        ste = sorted(recent47 + [x_e, n_e])
        materialize(gre, ste, {n: LANES5 for n in ste}, {n: p_parse(n).timestamp() - 1 for n in ste})
        os.utime(os.path.join(gre, n_e), ns=(int(q.timestamp()) * 10 ** 9 + 2000,) * 2)
        pke, nke = disk_pair(gre)
        # R3d 결정론 검체(바닥의 이름 절반 — 변이 MF2a 생존 뒤 추가): 이름 순서 48번째 자리에 같은 초 쌍 S·S-1 이 걸린다.
        #     부모 이름 순서: 최근 46 > T > S > S-1 → S 보관(문자열상 앞). 실효 순서에서는 mtime 이 늦은 옛 이름 Y1·Y2 가
        #     맨 앞이라 S 는 실효 48 밖이고, S 의 날의 대표는 T 다. 그래서 바닥의 **이름 절반이 부모처럼 접미를 무시할 때만** S 가 남는다.
        grd = os.path.join(dh, "namehalf")
        recent46 = [stamp(nowr - _td(minutes=i + 1)) for i in range(46)]
        t_d, s_d = "20261019T120000Z", "20261019T080000Z"
        y_d = [stamp(nowr - _td(days=30, minutes=k)) for k in (1, 2)]
        std = sorted(recent46 + [t_d, s_d, s_d + "-1"] + y_d)
        mtd = {n: p_parse(n).timestamp() - 1 for n in std}
        for y in y_d:
            mtd[y] = nowr.timestamp()
        materialize(grd, std, {n: LANES5 for n in std}, mtd)
        pkd, nkd = disk_pair(grd)
    finally:
        os.environ.pop("_CYS_SNAPSHOT_NOW", None)
    print("  R3 disk stores 40 · stores where do_gc deletes a parent-kept gen: %d" % len(viol3))
    check("R3 ★M1 성질(do_gc 실배선 · mtime 혼합 · 전 레인 현존 40 저장소): 부모 보관 ⊆ 수리본 보관",
          not viol3, repr(viol3[:3]))
    check("R3b ★M1 codex 반례를 디스크(do_gc)로: 10-19 무접미 대표 보관 · 부모 보관 ⊆ 수리본 보관",
          pkx <= nkx and "20261019T120000Z" in nkx, "parent-kept-now-deleted=%r" % sorted(pkx - nkx))
    check("R3d ★M1 바닥의 이름 절반: 48번째 자리의 같은 초 쌍에서 부모가 보관한 무접미(S)를 지우지 않는다",
          s_d in pkd and s_d in nkd and pkd <= nkd, "parent kept S=%r · patch kept S=%r · lost=%r"
          % (s_d in pkd, s_d in nkd, sorted(pkd - nkd)))
    check("R3c ★M1 바닥의 실효(mtime) 절반: 부모가 mtime 으로 보관한 옛 이름 세대를 지우지 않는다",
          n_e in pke and n_e in nke and pke <= nke, "parent kept n=%r · patch kept n=%r · lost=%r"
          % (n_e in pke, n_e in nke, sorted(pke - nke)))

    # R4 ★HEAD/CEO: 순서 규칙은 하나(_parse_stamp = 이름 시각, 그다음 숫자 접미) — GC 와 card 가 같은 규칙을 쓴다.
    s9, s10 = "20261019T080000Z-9", "20261019T080000Z-10"
    u0, u1 = "20261019T090000Z", "20261019T090000Z-1"
    check("R4a 순서 규칙: -10 > -9(숫자 · 문자열 순서와 반대) · -1 > 무접미 · 같은 초",
          ss._parse_stamp(s10) > ss._parse_stamp(s9) and ss._parse_stamp(u1) > ss._parse_stamp(u0)
          and ss._parse_stamp(s10).replace(microsecond=0) == ss._parse_stamp(s9).replace(microsecond=0))

    def card_rig(tag, names):
        h = tempfile.mkdtemp(prefix="snap-card-%s-" % tag, dir=root)
        live = os.path.join(h, "_round", "SESSION_STATE.md")
        gr = os.path.join(h, ".cys", "state-generations")
        for n in names:
            body = "# card of generation %s\n" % n
            _w(os.path.join(gr, n, "cc__SESSION_STATE.md"), body)
            _w(os.path.join(gr, n, "manifest.json"), json.dumps({
                "canonical_cards": {"base": live},
                "files": [{"source": live, "stored_as": "cc__SESSION_STATE.md",
                           "sha256": hashlib.sha256(body.encode("utf-8")).hexdigest(), "size": len(body)}]}))
        r = run_tool(h, os.path.join(h, ".cys", "pack"), "card", "--lane", "base")
        out = r.stdout.strip().split("\t")[0] if r.returncode == 0 and r.stdout.strip() else ""
        return os.path.basename(os.path.dirname(out)), r

    got9, r9 = card_rig("9v10", [s9, s10])
    check("R4b ★M2 card 기본 세대 = 순서 규칙상 최신: -9 와 -10 중 -10", got9 == s10,
          "picked %r rc=%s err=%r" % (got9, r9.returncode, r9.stderr[-200:]))
    gotu, ru = card_rig("0v1", [u0, u1])
    check("R4c card 기본 세대(같은 규칙): 무접미와 -1 중 -1", gotu == u1,
          "picked %r rc=%s err=%r" % (gotu, ru.returncode, ru.stderr[-200:]))
    newer48 = [stamp(_dtm(2026, 10, 20, tzinfo=_tz.utc) + _td(minutes=i)) for i in range(48)]
    kc9, _ = ss.compute_gc(sorted([s9, s10] + newer48), now=nowr)
    kcu, _ = ss.compute_gc(sorted([u0, u1] + newer48), now=nowr)
    check("R4d GC 의 일별 대표도 같은 규칙의 승자(-10 · -1) — card 와 GC 가 같은 세대를 최신으로 본다",
          s10 in kc9 and u1 in kcu, "kc9 has %r kcu has %r" % (sorted(n for n in kc9 if n.startswith("20261019")),
                                                             sorted(n for n in kcu if n.startswith("20261019"))))

    # R5 ★M3 적재 쪽 항목별 관용: 깨진 항목 하나가 유효한 부서 레인을 지우지 않는다.
    eh = tempfile.mkdtemp(prefix="snap-r1reg-", dir=root)
    e1 = os.path.join(eh, ".cys", "pack-dept-dept-1")
    _w(os.path.join(e1, "round", "SESSION_STATE.md"), "# d1\n")
    edj = os.path.join(eh, ".cys", "depts.json")
    _w(edj, json.dumps({"depts": {"dept-1": {"pack_dir": e1}, "dept-2": 42}}))
    lanes5 = ss.capture_lanes(home=eh, env={"HOME": eh}, depts_json=edj)
    check("R5 ★M3 항목별 관용: dept-2=42 가 있어도 dept-1 레인은 적재 대상", any(l[0] == "dept-1" for l in lanes5),
          repr([l[0] for l in lanes5]))

    # R6 ★M3 + HEAD/CEO: 레지스트리 판독 불가·파싱 불가·꼴 불일치·깨진 항목은 세대 manifest `missing` 에 기록되고
    #    (INCOMPLETE), 잡 보고(`snapshot 2>&1 | tail -3` — phoenix-snapshot-6h 의 push 본문)에 드러난다.
    def clean_fleet(tag, reg_bytes):
        h = tempfile.mkdtemp(prefix="snap-r6-%s-" % tag, dir=root)
        c = os.path.join(h, ".cys")
        _w(os.path.join(c, "pack", "round", "SESSION_STATE.md"), "# base pack round\n")
        _w(os.path.join(c, "pack", "round", "MASTER_TODO.md"), "- [ ] m\n")
        _w(os.path.join(h, "_round", "SESSION_STATE.md"), "# live base card\n")
        d1 = os.path.join(c, "pack-dept-dept-1")
        _w(os.path.join(d1, "round", "SESSION_STATE.md"), "# dept-1 card\n")
        _w(os.path.join(d1, "round", "WORKER_TODO.md"), "- [ ] w\n")
        for b in ("topology.json", "schedule_state.json", "event.seq"):
            _w(os.path.join(h, ".local", "state", "cys", b), "{}\n")
        dj = os.path.join(c, "depts.json")
        if reg_bytes is not None:
            with open(dj, "wb") as f:
                f.write(reg_bytes(d1) if callable(reg_bytes) else reg_bytes)
        return h, d1, dj

    def job_run(h):
        env = fleet_env(h, os.path.join(h, ".cys", "pack"))
        env.update({"R6_PY": PY, "R6_MOD": MOD})
        r = subprocess.run(["/bin/sh", "-c", '"$R6_PY" "$R6_MOD" snapshot 2>&1 | tail -3'], capture_output=True,
                           text=True, encoding="utf-8", errors="replace", env=env, cwd="/", timeout=120)
        g, man = newest_manifest(h)
        return r.stdout, (man or {})

    good = lambda d1: json.dumps({"depts": {"dept-1": {"pack_dir": d1}}}).encode()
    cases = [
        ("unparsable", b"{not json", False),
        ("undecodable", "{\"depts\": {\"부서\": {}}}".encode("cp949"), False),
        ("top-not-a-map", b"[]", False),
        ("depts-not-a-map", b"{\"depts\": []}", False),
        ("entry-not-a-map", lambda d1: json.dumps({"depts": {"dept-1": {"pack_dir": d1}, "dept-2": 42}}).encode(),
         True),
        ("entry-without-pack_dir", lambda d1: json.dumps({"depts": {"dept-1": {"pack_dir": d1},
                                                                    "dept-3": {"socket": "s"}}}).encode(), True),
    ]
    for label, body, keeps_d1 in cases:
        h, d1, dj = clean_fleet(label, body)
        tail3, man = job_run(h)
        miss = man.get("missing", [])
        faults = [m for m in miss if m.startswith(dj) and "[registry" in m]
        srcs = [e["source"] for e in man.get("files", [])]
        check("R6 ★M3 적재 fail-closed(%s): 레지스트리 문제가 manifest missing 에 기록 + 잡 보고에 INCOMPLETE" % label,
              bool(faults) and "INCOMPLETE" in tail3 and "registry" in tail3,
              "faults=%r tail3=%r" % (faults, tail3[-300:]))
        if keeps_d1:
            check("R6 (%s) 깨진 항목 밖의 유효 레인(dept-1 카드)은 그대로 적재" % label,
                  os.path.join(d1, "round", "SESSION_STATE.md") in srcs, repr(srcs))
    h, d1, dj = clean_fleet("clean", good)
    tail3, man = job_run(h)
    check("R6c 대조(구별력): 온전한 레지스트리·전 소스 실재면 missing 0 · 잡 보고에 INCOMPLETE 없음",
          man.get("missing") == [] and "INCOMPLETE" not in tail3 and bool(man.get("files")),
          "missing=%r tail3=%r" % (man.get("missing"), tail3[-300:]))
    h, d1, dj = clean_fleet("absent", None)
    tail3, man = job_run(h)
    check("R6d 레지스트리 부재 = 빈 등재(cys-dept 계약) — 레지스트리 문제 0 · 부재 파일 자체는 종전대로 missing",
          [m for m in man.get("missing", []) if "[registry" in m] == [] and dj in man.get("missing", []),
          "missing=%r" % man.get("missing"))

    # R7 ★gemini minor(HEAD 편입): 레인은 스냅샷당 한 번만 해소한다 — 소스와 정본 카드 지도가 같은 해소에서 나온다.
    #    두 번째 레지스트리 판독이 다른 답(부서 추가)을 내도 카드 지도의 모든 경로가 files ∪ missing 안에 있어야 한다.
    h7, _d17, dj7 = clean_fleet("once", good)
    d27 = os.path.join(h7, ".cys", "pack-dept-dept-2")
    _w(os.path.join(d27, "round", "SESSION_STATE.md"), "# dept-2 card (appears only in the 2nd registry read)\n")
    v1 = {"depts": {"dept-1": {"pack_dir": _d17}}}
    v2 = {"depts": {"dept-1": {"pack_dir": _d17}, "dept-2": {"pack_dir": d27}}}
    probe7 = (
        "import json, sys, types\n"
        "sys.path.insert(0, %r)\n"
        "import javis_state_snapshot as m\n"
        "V = [%r, %r]\n"
        "calls = []\n"
        "def load(f, *a, **k):\n"
        "    if sys._getframe(1).f_code.co_name == '_registry_pack_dirs':\n"
        "        calls.append(1)\n"
        "        return json.loads(json.dumps(V[min(len(calls), 2) - 1]))\n"
        "    return json.load(f, *a, **k)\n"
        "m.json = types.SimpleNamespace(load=load, dump=json.dump, loads=json.loads, dumps=json.dumps)\n"
        "g = m.do_snapshot(gen_root=%r)\n"
        "print('R7RESULT', json.dumps({'calls': len(calls), 'gen': g}))\n"
    ) % (BIN, v1, v2, os.path.join(h7, ".cys", "state-generations"))
    r7 = subprocess.run([PY, "-c", probe7], capture_output=True, text=True, encoding="utf-8", errors="replace",
                        env=fleet_env(h7, os.path.join(h7, ".cys", "pack")), cwd="/", timeout=120)
    res7 = {}
    for line in r7.stdout.splitlines():
        if line.startswith("R7RESULT "):
            res7 = json.loads(line[len("R7RESULT "):])
    _g7, man7 = newest_manifest(h7)
    man7 = man7 or {}
    have7 = set(e["source"] for e in man7.get("files", [])) | set(man7.get("missing", []))
    stray7 = sorted(p for p in (man7.get("canonical_cards") or {}).values() if p not in have7)
    check("R7 ★레인 1회 해소: 레지스트리 판독 1회 · 카드 지도의 모든 경로가 이 세대의 files ∪ missing 안",
          r7.returncode == 0 and res7.get("calls") == 1 and not stray7 and man7.get("canonical_cards"),
          "calls=%r stray=%r rc=%s err=%r" % (res7.get("calls"), stray7, r7.returncode, r7.stderr[-300:]))

    # R9 (R1-a) 순서 규칙의 정의역: 접미 µs 는 1초 안에 머문다(다음 초·다음 날로 넘어가지 않는다) · 손으로 만든 거대 접미도
    #    GC·card 를 죽이지 않는다(종전: timedelta OverflowError — do_gc 가 죽으면 그 뒤로 GC 가 영영 돌지 않는다).
    big = "20261019T120000Z-" + "9" * 25
    edge = "20261019T235959Z-1000000"
    try:
        okb = (ss._parse_stamp(big).strftime("%Y%m%d%H%M%S") == "20261019120000"
               and ss._parse_stamp(edge) < ss._parse_stamp("20261020T000000Z")
               and ss._parse_stamp(edge).strftime("%Y%m%d") == "20261019")
        errb = ""
    except Exception as exc:
        okb, errb = False, "%s: %s" % (type(exc).__name__, exc)
    check("R9 (R1-a) 순서 규칙: 접미는 그 초 안에 머문다(1e6 이상도 다음 초·다음 날로 넘지 않음) · 거대 접미에 예외 없음",
          okb, errb)
    bgr = os.path.join(dh, "bigsfx")
    materialize(bgr, sorted([big, "20261019T110000Z"]), {big: LANES5, "20261019T110000Z": LANES5},
                {big: nowr.timestamp() - 7200, "20261019T110000Z": nowr.timestamp() - 7200})
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            kb, _db = ss.do_gc(gen_root=bgr, dry_run=True, home=dh, depts_json=ddj)
        errg = ""
    except Exception as exc:
        kb, errg = [], "%s: %s" % (type(exc).__name__, exc)
    check("R9b (R1-a) 거대 접미 이름의 디렉터리가 있어도 do_gc 가 끝까지 돈다(두 세대 보관)", not errg and len(kb) == 2, errg)
    hb = tempfile.mkdtemp(prefix="snap-bigcard-", dir=root)
    os.makedirs(os.path.join(hb, ".cys", "state-generations", big))
    rbc = run_tool(hb, os.path.join(hb, ".cys", "pack"), "card", "--lane", "base")
    check("R9c (R1-a) 거대 접미 세대에서 card 는 추적(traceback) 없이 rc 1(정본 기록 없음)",
          rbc.returncode == 1 and "Traceback" not in rbc.stderr, "rc=%s err=%r" % (rbc.returncode, rbc.stderr[-300:]))

    # R10 (R1-b) 상태 디렉터리 발견(_dept_state_dirs)도 항목별 관용: 깨진 항목(42) 뒤의 유효 항목 소켓 자리를 잃지 않는다
    #     (glob 이 못 보는 자리 = cys-dept-* 이름이 아닌 소켓 부모). 같은 레지스트리 문제는 적재 쪽(R6)이 missing 에 올린다.
    sh10 = tempfile.mkdtemp(prefix="snap-r10-", dir=root)
    sr10 = os.path.join(sh10, ".local", "state")
    _w(os.path.join(sr10, "cys", "topology.json"), "{}\n")
    dj10 = os.path.join(sh10, ".cys", "depts.json")
    _w(dj10, json.dumps({"depts": {"a": 42, "b": {"socket": os.path.join(sr10, "custom-b", "cys.sock")}}}))
    srcs10 = ss.default_sources(home=sh10, state_root=sr10, depts_json=dj10, env={"HOME": sh10})
    check("R10 (R1-b) _dept_state_dirs 항목별 관용: 깨진 항목 뒤 유효 항목(b)의 상태 자리가 소스에 선언된다",
          os.path.join(os.path.realpath(os.path.join(sr10, "custom-b")), "topology.json") in srcs10,
          repr([x for x in srcs10 if "custom-b" in x]))

    # R8 ★HEAD/CEO: GC 쪽 fail-closed 는 그대로(판독 불가·깨진 항목 → 부재 판정 안 함 · 전 레인 48).
    for label, body in (("unparsable", "{not json"),
                        ("entry-not-a-map", json.dumps({"depts": {"dept-1": {"pack_dir": wpk["dept-1"]}, "x": 42}}))):
        _w(os.path.join(wh, ".cys", "depts.json"), body)
        with contextlib.redirect_stdout(io.StringIO()):
            k8, _d8 = ss.do_gc(gen_root=wgr, dry_run=True, home=wh, depts_json=os.path.join(wh, ".cys", "depts.json"))
        check("R8 ★GC fail-closed(%s): 레지스트리 밖 레인 세대 3건 모두 보관(상한 48 유지)" % label,
              all(n in k8 for n in x9), "kept %r" % [n for n in x9 if n in k8])
finally:
    shutil.rmtree(root, ignore_errors=True)

print("\n=== %d/%d PASS ===" % (checks - len(fails), checks))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("SNAPSHOT-ROOT-OK")
