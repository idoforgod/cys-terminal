#!/usr/bin/env python3
# javis_state_snapshot.py — 무손실 복원 Phase 0: 상태 파일 세대(generation) 보관기
#
# 설계 근거: _round/ZERO_LOSS_RESTORE_DESIGN.md §8.1-4
#   "Append-only 세대 보관 + 원본 불파괴 불변식 — 복원 시스템은 상태의 읽기 전용 소비자.
#    상태 파일은 덮어쓰지 않고 세대 보관(atomic rename + GC 정책)."
#
# 이 도구가 하는 일 / 안 하는 일:
#   - 한다: ~/.local/state/cys/topology.json 등 소형 L1 선언 상태 파일을 읽어
#           ~/.cys/state-generations/<timestamp>/ 에 원자적으로 세대 보관한다.
#   - 절대 안 한다: 원본 상태 파일을 수정/삭제하지 않는다(읽기 전용 소비자).
#                   state-generations/ 밖에는 아무것도 쓰지 않는다.
#
# 원자성 보장(성공기준 ①):
#   각 세대는 state-generations/.tmp-<pid>-<ts>/ 에 먼저 완성(파일 복사+fsync+manifest)한 뒤
#   os.rename 으로 최종 <timestamp>/ 로 '디렉터리 원자 승격'한다. 승격 이전에 프로세스가
#   죽으면 최종 세대는 존재하지 않고 .tmp-* 잔재만 남으며(다음 실행이 청소), 소비자는
#   반파 세대를 절대 보지 않는다. --self-test 가 SIGKILL 중단으로 이를 실증한다.
#
# GC 정책: 최근 48세대 + 일별 대표 14일 보관, 그 외 삭제(state-generations/ 내부만).
#   ★B-321: 레인별 풀 — 한 세대는 자기가 담은 모든 레인의 풀에 들고, 정책은 풀마다 적용·합집합 보관한다.
#   depts.json 에서 빠진 부서 레인은 최신 1세대만(최종 상태 보관 · 이력 미보관). compute_gc 참조.
#   ★B317-R1: 부모(d478d97f) 판정을 바닥으로 합집합 — 이 판은 부모가 보관하던 세대를 지우지 않는다(위 예외만).
# B-317: 적재 = 전 레인 round(카드·노드 TODO·법 묶음) + 레인별 정본 카드(capture_lanes) · DB/WAL/SHM/.bak 제외.
# D-07: TODO glob 0건도 필수 누락으로 기록하고, autopilot.json 선택부재는 별도 계수한다.
# D-07: dry-run 은 디렉터리 생성·임시 세대 청소 없이 보관 대상만 표시한다.
# D-07: .tmp-* 는 살아 있는 pid 를 보호하고, 죽은 pid 는 60초·pid 불명은 3600초 후 청소한다.
#
# 사용:
#   javis_state_snapshot.py snapshot [--dry-run]   # 세대 1건 생성 + GC
#   javis_state_snapshot.py list                   # 세대 목록
#   javis_state_snapshot.py verify [--gen <이름>]  # manifest 해시 무결성 검증
#   javis_state_snapshot.py gc [--dry-run]         # 보관정책 적용(삭제 대상 산출/실행)
#   javis_state_snapshot.py card [--gen <이름>] [--lane base|<부서>]  # 레인별 정본 카드 위치(읽기 전용 복원 경로)
#   javis_state_snapshot.py self-test              # 원자성/중단내성 실증(격리 임시폴더)

import argparse
import errno
import fnmatch
import glob as _glob
import hashlib
import json
import os
import re
import shutil
import sys
import tempfile
import time
from datetime import datetime, timedelta, timezone

HOME = os.path.expanduser("~")

# 세대 보관 루트 (이 스크립트가 쓰기를 허용받은 유일한 출력 경로)
GEN_ROOT = os.path.join(HOME, ".cys", "state-generations")

# 세대 보관 대상 = 소형 L1 선언 상태(토폴로지/스케줄/자율주행/부서). 대형 DB(transcripts.db,
# analytics.db)·log 는 세대 보관이 아니라 3-2-1 백업의 몫이라 여기서 제외한다.
#
# ★Phase 7(자동 보호 상속): 소스는 정적 목록이 아니라 default_sources() 로 동적 산출한다 —
#   메인 데몬 선언상태 + 루트 레지스트리(depts.json) + ★발견되는 모든 부서의 선언상태를 자동 포함.
#   부서는 손 배선·부서명 하드코딩 없이 glob(cys-dept-*) ∪ depts.json 레지스트리로 발견된다
#   (모든 사용자 동일 적용 — 개인 경로/계정 무첨가, HOME 파생 상대경로만).
DECLARATIVE_BASENAMES = ["topology.json", "schedule_state.json", "autopilot.json", "event.seq"]
# pause/resume 시 생성되는 선택 상태지만, 존재하면 반드시 보관해야 복원 시 pause 를 유지한다.
OPTIONAL_BASENAMES = ("autopilot.json",)

IS_WINDOWS = os.name == "nt"

# ── Windows named pipe → 상태 디렉터리 매핑 (Rust src/bin/cysd/state.rs::pipe_slug/state_dir 규칙의 단일 소스) ──
# ★javis_phoenix.py 도 이 두 함수를 재사용한다(중복 구현 금지) — 규칙은 여기 한 곳에만 둔다.
#   unix 는 소켓 부모가 곧 상태 dir 이지만 Windows 는 파이프에 파일시스템 부모가 없어 슬러그로 파생해야 한다:
#   기본 데몬(`\\.\pipe\cys`)=%LOCALAPPDATA%\cys · 부서(`\\.\pipe\cys-dept-<n>`)=그 하위 슬러그 디렉터리.

def _win_pipe_slug(socket):
    """named pipe 경로에서 슬러그 추출 — 역/슬래시 마지막 컴포넌트의 안전문자(영숫자·-·_)만(state.rs::pipe_slug)."""
    s = str(socket)
    last = re.split(r"[\\/]", s)[-1] if s else ""
    return "".join(c for c in last if c.isalnum() or c in "-_")


def _win_state_dir_for_socket(socket, localappdata=None, home=HOME):
    """Windows 소켓(named pipe)→상태 디렉터리(state.rs::state_dir). localappdata 주입 가능(테스트·주입용)."""
    base = localappdata or os.environ.get("LOCALAPPDATA") or os.path.join(home, "AppData", "Local")
    root = os.path.join(base, "cys")
    slug = _win_pipe_slug(socket)
    if not slug or slug == "cys":
        return os.path.realpath(root)
    return os.path.realpath(os.path.join(root, slug))


def _dept_state_dirs(state_root, depts_json=None, windows=None, localappdata=None):
    """부서 상태 디렉터리를 동적 발견한다 — 파일시스템 glob(cys-dept-*) ∪ depts.json 소켓 매핑.
    ★부서명 하드코딩 없음. registry(depts.json)가 stale(미등록 부서 존재)여도 파일시스템 truth 로 커버한다
    (Phase7 실측: depts.json=dept-1 만인데 디스크엔 dept-1~5 존재 → glob 이 전부 잡는다).
    ★Windows(권고#1): 부서 디렉터리는 %LOCALAPPDATA%\\cys 하위(cys-dept-*)이고 registry 소켓은 named pipe 라
    부모 dirname 이 아니라 _win_state_dir_for_socket 슬러그 매핑으로 상태 dir 을 파생한다(unix=소켓 부모 그대로)."""
    win = IS_WINDOWS if windows is None else windows
    dirs = set()
    try:
        for name in os.listdir(state_root):
            if name.startswith("cys-dept-"):
                p = os.path.join(state_root, name)
                if os.path.isdir(p):
                    dirs.add(os.path.realpath(p))
    except OSError:
        pass
    dj = depts_json or os.path.join(os.path.dirname(os.path.dirname(state_root)), ".cys", "depts.json")
    if os.path.isfile(dj):
        try:
            reg = json.load(open(dj, encoding="utf-8-sig"))
            for _name, meta in (reg.get("depts") or {}).items():
                # ★B317-R1(R1-b): 항목별 관용 — 깨진 항목 하나(객체 아님·소켓이 문자열 아님)가 뒤 항목의 상태 자리를 버리지
                #   않는다. 같은 레지스트리 문제는 capture_lanes 가 세대 missing 에 올린다(_registry_pack_dirs).
                sock = meta.get("socket") if isinstance(meta, dict) else None
                if isinstance(sock, str) and sock:
                    if win:
                        dirs.add(os.path.realpath(_win_state_dir_for_socket(sock, localappdata=localappdata)))
                    else:
                        dirs.add(os.path.realpath(os.path.dirname(sock)))
        except Exception:
            pass
    return sorted(dirs)


# ★팩 경로 env 키 목록·순서 — 정본은 javis_preflight.PACK_DIR_ENV_KEYS(A11 · src/pack.rs 미러)이고
#   여기는 같은 목록·같은 순서의 **미러**다(먼저 발견되는 비어있지 않은 값이 이긴다). 이 스크립트는
#   형제 모듈을 import 하지 않는다 — 데몬 스케줄 잡이 팩 밖 cwd 에서 단독 실행하는 파일이라 import
#   실패가 곧 세대 누락이다(그래서 사본이 아니라 미러이며, 정본이 바뀌면 이 줄도 함께 고친다).
PACK_DIR_ENV_KEYS = ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR")


def project_round_dir(env=None, cwd=None):
    """복원 SOT(SESSION_STATE.md · *_TODO.md)가 사는 round 디렉터리 해소 — 3단 폴백.

    ★PREP #3(2026-09-03 · phoenix 세대 스냅샷 누락 3건 재발 · 연속 5세대+): phoenix-snapshot-6h 잡은
      데몬 스케줄러가 **cwd `/` · JAVIS_ROOT 미설정**으로 이 스크립트를 부른다. 종전 산식
      `os.environ.get("JAVIS_ROOT") or os.getcwd()` + "_round" 는 그 환경에서 `/_round` 가 되어
      SESSION_STATE.md·노드 TODO 가 manifest 에서 매 세대 빠졌다(실측: 파일 50 · 누락 3건/세대).
      데몬 잡이 반드시 갖는 env 는 팩 경로(CYS_PACK_DIR)이고, 부트 이후 SESSION_STATE/todo 의 실체는
      `<pack>/round/` 에 산다(`cys todo-path` 실측 `~/.cys/pack/round/WORKER_2_TODO.md`).
      ① JAVIS_ROOT 설정 → `<JAVIS_ROOT>/_round`  (종전 1순위 — 프로젝트 레인 거동 불변)
      ② 팩 env 키 중 **첫 비어있지 않은 값**의 `<pack>/round` 가 디렉터리로 실재 → 그것
         (키 순서·'첫 값이 이긴다' 계약은 PACK_DIR_ENV_KEYS 정본과 동일 — 뒤 키를 더 보지 않는다)
      ②' ★[결재 15] 팩 env 키가 **하나도 없으면** `<HOME>/.cys/pack/round` 가 디렉터리로 실재할 때 그것
         (팩 스크립트 기본값 `~/.cys/pack` 과 동일). 종전엔 여기서 곧장 ③으로 떨어져 cwd 가 홈이면
         `~/_round` 를, cwd 가 `/` 면 `/_round` 를 골라 노드 TODO 0건이 됐다(실측 2026-09-19: env 有 round 7건 / 無 0건).
         ★정정(B-317 결정 7 · 2026-10-20): 종전 문면은 `~/_round` 를 「복원 정본이 없는 미끼」라 불렀으나 **틀렸다** —
         그 자리는 save-state.sh 의 .state_log 자리이면서 **본부(base) 레인의 라이브 카드 `~/_round/SESSION_STATE.md`**
         가 사는 곳이다(부트 inject-context.sh 가 cwd 상향 탐색으로 주입 · 실측 220,061 B vs `~/.cys/pack/round` 사본
         716,061 B · 10-16 정지). 이 함수가 그 디렉터리를 고르지 않는 이유는 카드가 없어서가 아니라 **노드 TODO 를 담는
         레인 round 가 아니기 때문**이다(README-NO-TODO-FILES-HERE). 본부 카드는 default_sources() 가 capture_lanes()
         로 따로 선언한다. ⛔범위: 이 정정은 **본부 레인 한정**이다 — 팩 키가 설정된 부서 레인의 라이브 카드는
         `<pack>/round/SESSION_STATE.md` 그 자체이고 정본이다(HEAD 범위 정정 2026-10-20).
         ⛔이 함수의 **단일 레인 해소 계약은 바꾸지 않는다**(호출자·bin/tests/test_state_snapshot_root.py A 계열 의존).
         ★팩 키가 **설정돼 있는데** round/ 가 없을 때는 이 폴백을 타지 않는다 — 명시된 (부서)팩 대신
         본부 팩을 담으면 레인 교차 오염이다(②의 '첫 값이 이긴다' 계약 보존).
      ③ 그 밖 → `<cwd>/_round`  (종전 폴백 그대로 — 사람이 프로젝트 루트에서 손으로 부르는 경우)
    env/cwd 주입은 self-test·bin/tests/test_state_snapshot_root.py 밀폐용(기본=실환경)."""
    env = os.environ if env is None else env
    root = env.get("JAVIS_ROOT")
    if root:
        return os.path.join(root, "_round")
    pack_key_set = False
    for key in PACK_DIR_ENV_KEYS:
        pack = env.get(key)
        if pack:
            pack_key_set = True
            cand = os.path.join(pack, "round")
            if os.path.isdir(cand):
                return cand
            break
    if not pack_key_set and env.get("HOME"):
        cand = os.path.join(env["HOME"], ".cys", "pack", "round")
        if os.path.isdir(cand):
            return cand
    return os.path.join(cwd or os.getcwd(), "_round")


# ★B-317(CEO 결정 1·2 · 2026-10-20): 적재 대상은 **세대를 띄운 데몬의 팩에 의존하지 않는다** — 전 레인 합집합.
#   종전 default_sources() 는 project_round_dir() 를 한 번 불러 **발화 레인 하나**의 round 만 담았다
#   (phoenix-snapshot-6h 는 데몬마다 자기 CYS_PACK_DIR 로 돈다 ⇒ 모든 세대가 단일 레인 · 실측 60세대 중 본부 6).
#   각 세대가 자기 레인에 대해서만 완전했으므로 「누락 0건」이 참이면서 다른 레인은 선언조차 되지 않았다.
ROUND_CARD = "SESSION_STATE.md"
ROUND_TODO_GLOB = "*_TODO.md"
# 법 묶음: 무번호 첫 권(CSO_STANDING_RULES.md)까지 — `CSO_STANDING_RULES_*.md` 는 첫 권을 놓친다(B-317 AMEND-3).
LAW_CORPUS_GLOB = "CSO_STANDING_RULES*.md"
# B-317-AMEND 결박: DB·WAL·SHM 은 B-318(정지 장벽) 전까지 절대 담지 않는다 · 백업 사본(.bak/.bak-*)도 제외.
#   basename 끝 일치(fnmatch)다 — `wall-time_TODO.md` 처럼 이름 **안**의 '-wal' 은 제외하지 않는다(AMEND-4).
CAPTURE_EXCLUDE_GLOBS = ("*.db", "*.sqlite", "*-wal", "*-shm", "*.bak", "*.bak-*")
BASE_LANE = "base"
FIRING_LANE = "firing"


def _capture_excluded(path):
    b = os.path.basename(path)
    return any(fnmatch.fnmatchcase(b, g) for g in CAPTURE_EXCLUDE_GLOBS)


def _registry_pack_dirs(depts_json):
    """depts.json 의 부서별 pack_dir → ([(부서명, pack_dir)] 이름순, [레지스트리 문제 기술]).
    ⛔레지스트리 실패가 본부·발화 레인 적재를 죽이지 않는다(그 레인들은 레지스트리 없이도 선언된다).
    ★B317-R1 M3(codex) + HEAD/CEO fail-closed: 판독 실패를 「부서 0개」로 접지 않는다 — 문제는 do_snapshot 이
      manifest `missing` 에 올려 세대를 INCOMPLETE 로 표시한다(읽지 못한 입력은 출력을 짧게가 아니라 크게 만든다).
      · 파일 부재 = 빈 등재(cys-dept 계약 「depts.json 부재는 정상 빈 등재」) — 문제 아님. 부재 파일 자체는
        default_sources 가 소스로 선언하므로 종전대로 missing 에 뜬다. `depts` 키 부재도 빈 등재(같은 계약의 .get).
      · 판독·디코딩·파싱 실패, 최상위·`depts` 꼴 불일치 = 레지스트리 문제 1건(부서 레인을 하나도 알 수 없다).
      · 항목 꼴 불일치(객체 아님 · pack_dir 이 비어 있거나 문자열 아님) = 그 항목 문제 1건 — ⛔유효한 다른 항목은 버리지 않는다.
    GC 쪽(_lane_caps)은 별도 판정이다 — 같은 문제에서 부재를 판정하지 않고 전 레인 48 을 유지한다."""
    try:
        with open(depts_json, encoding="utf-8-sig") as f:
            reg = json.load(f)
    except FileNotFoundError:
        return [], []
    except Exception as e:
        why = ("%s: %s" % (type(e).__name__, e)).replace("\n", " ")[:160]
        return [], ["%s [registry unreadable: %s]" % (depts_json, why)]
    if not isinstance(reg, dict):
        return [], ["%s [registry shape: top level is not an object]" % depts_json]
    depts = reg.get("depts", {})
    if not isinstance(depts, dict):
        return [], ["%s [registry shape: depts is not an object]" % depts_json]
    out, faults = [], []
    for name, meta in sorted(depts.items()):
        pd = meta.get("pack_dir") if isinstance(meta, dict) else None
        if isinstance(pd, str) and pd:
            out.append((name, pd))
        else:
            faults.append("%s#depts/%s [registry entry: %s]" % (
                depts_json, name, "not an object" if not isinstance(meta, dict) else "no usable pack_dir"))
    return out, faults


def capture_lanes(home=HOME, env=None, depts_json=None, faults=None):
    """[(레인, round 디렉터리, 그 레인의 정본 카드 경로)] — 세대가 담아야 하는 레인 전부.
      · 본부(base): `<home>/.cys/pack` 이 실재하면. round = `<home>/.cys/pack/round`,
        ★정본 카드 = `<home>/_round/SESSION_STATE.md`(결정 7 · 부트가 주입하는 라이브 카드 — 본부 레인 한정).
      · 부서: depts.json 의 pack_dir 마다. round = `<pack>/round`, 정본 카드 = `<pack>/round/SESSION_STATE.md`.
      · 발화 레인: project_round_dir(env) — 위 둘에 이미 있으면 생략(실경로 비교). 레지스트리 밖 팩·JAVIS_ROOT
        프로젝트도 종전처럼 담긴다(종전 단일 레인 계약을 합집합 안에 보존).
    ★fail-closed: 선언된 레인의 round·카드가 없으면 default_sources 가 그 경로를 그대로 남겨 manifest `missing`
      에 뜬다 — 선언하지 않아 「완전해 보이는」 종전 실패 방향을 뒤집는다.
    ★B317-R1 M3: 레지스트리 문제(판독 불가·꼴 불일치·깨진 항목)는 faults(리스트) 에 덧붙는다 — 레인을 알 수 없는 부서도
      조용히 사라지지 않게 do_snapshot 이 그것을 missing 에 올린다. faults=None 이면 종전처럼 버린다(조회 전용 호출)."""
    lanes, seen = [], set()

    def _add(lane, rd, card):
        k = os.path.realpath(rd)
        if k not in seen:
            seen.add(k)
            lanes.append((lane, rd, card))

    base_pack = os.path.join(home, ".cys", "pack")
    if os.path.isdir(base_pack):
        # ★본부 카드 자리 `~/_round` 는 이 배치(본부 master 가 홈에서 돈다 → 부트가 홈의 _round 를 찾는다)의 사실이다.
        #   그 디렉터리가 없는 설치에서는 선언하지 않는다(매 세대 거짓 「누락」 금지 · R2). 디렉터리가 있는데 카드가 없으면
        #   선언된 채 누락으로 뜬다(fail-closed).
        bcard = os.path.join(home, "_round", ROUND_CARD) if os.path.isdir(os.path.join(home, "_round")) else None
        _add(BASE_LANE, os.path.join(base_pack, "round"), bcard)
    entries, errs = _registry_pack_dirs(depts_json or os.path.join(home, ".cys", "depts.json"))
    if faults is not None:
        faults.extend(errs)
    for name, pack in entries:
        rd = os.path.join(pack, "round")
        _add(name, rd, os.path.join(rd, ROUND_CARD))
    rd = project_round_dir(env=env)
    _add(FIRING_LANE, rd, os.path.join(rd, ROUND_CARD))
    return lanes


def canonical_cards(home=HOME, env=None, depts_json=None, lanes=None):
    """{레인: 정본 카드 경로} — 세대 manifest 의 `canonical_cards` 로 기록되어 복원 경로가 이름으로 고른다.
    lanes = 이미 해소한 capture_lanes() 결과(do_snapshot 이 소스와 같은 해소를 넘긴다 · 미주입 = 여기서 해소)."""
    if lanes is None:
        lanes = capture_lanes(home=home, env=env, depts_json=depts_json)
    return {lane: card for lane, _rd, card in lanes if card}


def select_card(manifest, lane=BASE_LANE):
    """세대 manifest 에서 그 레인의 정본 카드 항목을 고른다(없으면 None).
    ★결정 7(CEO 2026-10-20) + HEAD 범위 정정: 본부(base) 레인의 정본 = `~/_round/SESSION_STATE.md` ·
      ★부서 레인의 정본 = `<pack>/round/SESSION_STATE.md` 그 자체 — 부서 카드는 주석·은퇴·강등·건너뛰기 대상이 아니다.
      `~/.cys/pack/round/SESSION_STATE.md`(본부 팩 round 의 사본 · 716,061 B · 10-16 정지)는 세대에 담기지만 **본부
      정본으로 고르지 않는다** — 은퇴 표기는 본부 레인이 자기 파일에 쓴다(이 도구는 라이브 팩에 아무것도 쓰지 않는다).
    ⛔basename(SESSION_STATE.md)으로 고르지 않는다 — 세대 안에 카드가 레인 수만큼 있다(경로 sha 접두로 구분).
    ⛔이름으로 기록된 정본이 없으면(이 필드가 생기기 전 세대 · 카드 누락 세대) 다른 카드로 **대체하지 않는다** —
      종전 세대의 유일한 SESSION_STATE.md 는 다른 계보의 카드였다(복원하면 묵은 카드가 라이브를 덮는다)."""
    want = (manifest.get("canonical_cards") or {}).get(lane)
    if not want:
        return None
    for entry in manifest.get("files", []):
        if entry.get("source") == want:
            return entry
    return None


def default_sources(home=HOME, state_root=None, depts_json=None, windows=None, localappdata=None, env=None,
                    lanes=None):
    """세대 보관 소스를 동적 산출한다(정적 목록 대체). 메인 선언상태 + 루트 레지스트리 +
    ★발견되는 모든 부서의 선언상태를 자동 포함 = '태어날 때부터 보호'(손 배선 0).
    ★Windows(권고#1): 메인 상태=%LOCALAPPDATA%\\cys, 부서=그 하위 cys-dept-*(state.rs 레이아웃). unix 는 종전대로
    <state_root>/cys + <state_root>/cys-dept-*. windows/localappdata 주입으로 mac 에서도 win 분기 단위검증 가능.
    lanes = 이미 해소한 capture_lanes() 결과(B317-R1 · gemini minor: 스냅샷당 레인 해소 1회 — 미주입 = 여기서 해소)."""
    win = IS_WINDOWS if windows is None else windows
    if win:
        base = localappdata or os.environ.get("LOCALAPPDATA") or os.path.join(home, "AppData", "Local")
        main_state = os.path.join(base, "cys")
        dept_scan_root = main_state            # win: 부서는 cys 하위(state.rs 슬러그 레이아웃)
    else:
        state_root = state_root or os.path.join(home, ".local", "state")
        main_state = os.path.join(state_root, "cys")
        dept_scan_root = state_root             # unix: 부서는 state_root 형제(cys-dept-*)
    srcs = [os.path.join(main_state, b) for b in DECLARATIVE_BASENAMES]
    depts_json = depts_json or os.path.join(home, ".cys", "depts.json")
    srcs.append(depts_json)
    for dd in _dept_state_dirs(dept_scan_root, depts_json=depts_json, windows=win, localappdata=localappdata):
        srcs.extend(os.path.join(dd, b) for b in DECLARATIVE_BASENAMES)
    # ★G1(d)(cokacdir 성찰 2026-07-04): 복원 SOT·노드 TODO·장기기억도 세대 보관에 포함 —
    #   SESSION_STATE.md 는 재부팅 복원의 단일 진실인데 세대 보관 대상에서 빠져 있었다.
    #   개인경로 하드코딩 금지(pack scan gate): 프로젝트는 $JAVIS_ROOT(env 또는 CWD),
    #   장기기억은 HOME glob 파생만(javis_wakeup.py 등과 동일 관례).
    # ★PREP #3: 발화 레인 해소는 project_round_dir() 한 곳(사유·순서는 그 docstring).
    # ★B-317(결정 1·2·7): 그 위에 **전 레인** round(카드·노드 TODO·법 묶음) + 레인별 정본 카드를 합집합으로 선언한다
    #   (capture_lanes). 같은 **경로 문자열**은 한 번만 — 같은 basename 은 do_snapshot 이 경로 sha 접두로 구분한다.
    #   ⛔실경로로 접지 않는다: `~/_round` 가 심링크면 정본 카드 경로가 소스에서 사라져 select_card 가 이름으로
    #   찾지 못한다(R1 G9). 같은 바이트가 두 이름으로 담기는 쪽이 정본을 잃는 쪽보다 낫다.
    seen = set()

    def _add_src(p):
        k = os.path.normpath(p)
        if k not in seen:
            seen.add(k)
            srcs.append(p)

    if lanes is None:
        lanes = capture_lanes(home=home, env=env, depts_json=depts_json)
    for _lane, proj_round, card in lanes:
        _add_src(os.path.join(proj_round, ROUND_CARD))
        # D-07: 종전엔 glob 0건이 조용한 0이라 phoenix 세대에서 노드 TODO 누락이 보이지 않았다.
        todo_pattern = os.path.join(proj_round, ROUND_TODO_GLOB)
        todos = [p for p in sorted(_glob.glob(todo_pattern)) if not _capture_excluded(p)]
        for p in todos or [todo_pattern]:
            _add_src(p)
        # 법 묶음은 레인마다 있을 수도 없을 수도 있다(현재 본부만) — 0건은 누락이 아니다(패턴을 남기지 않는다).
        for p in sorted(_glob.glob(os.path.join(proj_round, LAW_CORPUS_GLOB))):
            if not _capture_excluded(p):
                _add_src(p)
        if card:
            _add_src(card)   # 부서 레인은 round 카드와 같은 경로(중복 제거) · 본부는 `~/_round/SESSION_STATE.md`
    srcs.extend(sorted(_glob.glob(os.path.join(
        home, ".claude*", "projects", "*", "memory", "*.md"))))
    return srcs

# GC 파라미터
KEEP_RECENT = 48        # 최근 N세대는 시간 무관 보관
KEEP_DAILY_DAYS = 14    # 최근 D일은 하루당 대표 1세대 보관

GEN_NAME_RE = re.compile(r"^(\d{8}T\d{6}Z)(?:-(\d+))?$")  # 20260703T085230Z 또는 ...-1
TMP_PREFIX = ".tmp-"
TMP_MIN_AGE_SECS = 60
TMP_STALE_AGE_SECS = 3600


def _sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def _fsync_path(path):
    """파일 또는 디렉터리를 fsync 한다(디렉터리는 엔트리 durability 확보).
    ★Windows(S5): 디렉터리 fsync 는 건너뛴다 — Win32 는 디렉터리 핸들 fsync 를 거부(os.open(dir) PermissionError)한다.
    디렉터리 엔트리 durability 대신 os.rename(동일 볼륨 원자적 승격)에 의존한다(파일 fsync 는 그대로 수행)."""
    if os.name == "nt" and os.path.isdir(path):
        return
    flags = os.O_RDONLY
    if hasattr(os, "O_DIRECTORY") and os.path.isdir(path):
        flags |= os.O_DIRECTORY
    fd = os.open(path, flags)
    try:
        os.fsync(fd)
    except OSError:
        # 일부 파일시스템은 디렉터리 fsync를 거부할 수 있으나 rename 자체는 원자적이므로 치명 아님
        pass
    finally:
        os.close(fd)


def _now_utc():
    # 테스트 결정성을 위해 환경변수로 타임스탬프 주입 가능(self-test 용)
    override = os.environ.get("_CYS_SNAPSHOT_NOW")
    if override:
        return datetime.fromtimestamp(float(override), tz=timezone.utc)
    return datetime.now(timezone.utc)


def _gen_stamp(dt):
    return dt.strftime("%Y%m%dT%H%M%SZ")


def list_generations(gen_root=GEN_ROOT):
    """유효 세대 디렉터리만(.tmp-* 제외) 이름 오름차순으로 반환."""
    if not os.path.isdir(gen_root):
        return []
    out = []
    for name in os.listdir(gen_root):
        if name.startswith(TMP_PREFIX):
            continue
        if GEN_NAME_RE.match(name) and os.path.isdir(os.path.join(gen_root, name)):
            out.append(name)
    out.sort()
    return out


def _pid_alive(pid):
    """프로세스 생존 여부(판정 불가면 True). Windows 의 os.kill(pid, 0)은
    TerminateProcess 를 호출하므로 금지하고, OpenProcess 조회 핸들만 사용한다."""
    if os.name == "nt":
        try:
            import ctypes
            from ctypes import wintypes

            kernel32 = ctypes.windll.kernel32
            kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
            kernel32.OpenProcess.restype = wintypes.HANDLE
            kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
            kernel32.CloseHandle.restype = wintypes.BOOL
            kernel32.GetLastError.argtypes = []
            kernel32.GetLastError.restype = wintypes.DWORD
            handle = kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
            if not handle:
                return kernel32.GetLastError() != 87  # ERROR_INVALID_PARAMETER 만 부재 확정
            kernel32.CloseHandle(handle)
            return True
        except Exception:
            return True
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except Exception:
        return True
    return True


def _cleanup_tmp(gen_root):
    """.tmp-* 잔재 중 죽은 pid(60초 이상) 또는 pid 불명(3600초 이상)만 청소한다."""
    removed = []
    if not os.path.isdir(gen_root):
        return removed
    now = time.time()
    for name in os.listdir(gen_root):
        if name.startswith(TMP_PREFIX):
            p = os.path.join(gen_root, name)
            try:
                pid = int(name[len(TMP_PREFIX):].split("-", 1)[0])
                if pid <= 0:
                    raise ValueError("pid must be positive")
            except ValueError:
                min_age = TMP_STALE_AGE_SECS
            else:
                if _pid_alive(pid):
                    continue
                min_age = TMP_MIN_AGE_SECS
            try:
                if not os.path.isdir(p) or now - os.path.getmtime(p) < min_age:
                    continue
                shutil.rmtree(p)
                removed.append(name)
            except OSError:
                pass
    return removed


def do_snapshot(sources=None, gen_root=GEN_ROOT, dry_run=False, crash_hook=None, cards=None):
    """상태 파일 세대 1건을 원자적으로 생성한다. 생성된 세대 이름 반환(dry_run이면 None).
    ★B-317 결정 7: 기본 소스(sources=None)면 레인별 정본 카드 지도(canonical_cards)를 manifest 에 기록한다 —
    복원은 select_card() 가 이 지도의 **경로 이름**으로 고른다. 명시 소스 호출은 cards 를 줄 때만 기록한다.
    ★B317-R1(gemini minor · HEAD 편입): 레인은 스냅샷당 **한 번** 해소해 소스와 카드 지도에 같이 넘긴다 — 두 번 해소하면
      그 사이 레지스트리가 바뀔 때 카드 지도가 이 세대에 없는 경로를 가리킨다.
    ★B317-R1 M3 + HEAD/CEO: 레지스트리 문제(registry_faults)는 manifest `missing` 에 올라 세대가 INCOMPLETE 로 표시되고,
      요약 줄(phoenix-snapshot-6h 가 `| tail -3` 으로 master 에 push 하는 본문)에 INCOMPLETE 가 찍힌다. rc 는 종전대로 0."""
    registry_faults = []
    if sources is None:
        lanes = capture_lanes(faults=registry_faults)
        sources = default_sources(lanes=lanes)
        if cards is None:
            cards = canonical_cards(lanes=lanes)
    present, missing, optional_absent = [], [], []
    for s in sources:
        if os.path.isfile(s):
            present.append(s)
        elif os.path.basename(s) in OPTIONAL_BASENAMES:
            optional_absent.append(s)
        else:
            missing.append(s)
    for f in registry_faults:
        print(f"[snapshot] ⚠ 부서 레지스트리 문제 — 그 부서 round 를 담지 못했을 수 있다: {f}", file=sys.stderr)
    n_missing = len(missing) + len(registry_faults)
    incomplete = ""
    if n_missing:
        incomplete = " — INCOMPLETE: 누락 %d건" % n_missing
        if registry_faults:
            incomplete += " (그중 depts.json registry 문제 %d건)" % len(registry_faults)

    if dry_run:
        print(f"[dry-run] 세대 보관 대상 {len(present)}건:")
        for s in present:
            print(f"  + {s}  ({os.path.getsize(s)}B)")
        for s in missing:
            print(f"  - (없음) {s}")
        for s in registry_faults:
            print(f"  - (registry 문제) {s}")
        for s in optional_absent:
            print(f"  - (선택·없음) {s}")
        if cards:
            print(f"[dry-run] 레인별 정본 카드 {len(cards)}건:")
            for lane in sorted(cards):
                print(f"  * {lane} -> {cards[lane]}  ({'있음' if os.path.isfile(cards[lane]) else '없음'})")
        if incomplete:
            print(f"[dry-run]{incomplete}")
        return None

    os.makedirs(gen_root, exist_ok=True)

    # 승격 전 죽은 잔재 청소(dry-run 은 생성·청소 모두 무기록)
    _cleanup_tmp(gen_root)

    if not present:
        print("[snapshot] 보관할 소스 파일이 하나도 없음 — 세대 미생성", file=sys.stderr)
        return None

    dt = _now_utc()
    stamp = _gen_stamp(dt)

    # 1) 임시 세대 디렉터리에 완성
    tmp_dir = tempfile.mkdtemp(prefix=f"{TMP_PREFIX}{os.getpid()}-", dir=gen_root)
    manifest = {
        "created_at": dt.timestamp(),
        "created_at_iso": dt.isoformat(),
        "generator": "javis_state_snapshot.py",
        "files": [],
        "missing": sorted(missing + registry_faults),
        "optional_absent": sorted(optional_absent),
    }
    if cards:
        manifest["canonical_cards"] = dict(cards)
    try:
        for src in present:
            digest = _sha256(src)
            st = os.stat(src)
            base = os.path.basename(src)
            # 소스명 충돌 방지: 경로 해시 접두
            dest_name = base
            if any(os.path.basename(x) == base for x in present if x != src):
                tag = hashlib.sha256(src.encode()).hexdigest()[:8]
                dest_name = f"{tag}__{base}"
            dest = os.path.join(tmp_dir, dest_name)
            with open(src, "rb") as fsrc, open(dest, "wb") as fdst:
                shutil.copyfileobj(fsrc, fdst)
                fdst.flush()
                os.fsync(fdst.fileno())
            manifest["files"].append({
                "source": src,
                "stored_as": dest_name,
                "sha256": digest,
                "size": st.st_size,
                "src_mtime": st.st_mtime,
            })

        mpath = os.path.join(tmp_dir, "manifest.json")
        with open(mpath, "w", encoding="utf-8") as mf:  # ★(0.14.48 · A) 기본 인코딩에 맡기지 않는다 — 읽는 쪽과 모드가 달라도 같은 바이트
            json.dump(manifest, mf, indent=2, ensure_ascii=False)
            mf.flush()
            os.fsync(mf.fileno())

        _fsync_path(tmp_dir)

        # 중단 시뮬레이션 훅(self-test 전용): 승격 직전 프로세스를 강제 종료
        if crash_hook == "before_rename" or os.environ.get("_CYS_SNAPSHOT_CRASH_BEFORE_RENAME"):
            os._exit(137)  # SIGKILL 유사 — atexit/flush 없이 즉사

        # 2) 최종 이름으로 디렉터리 원자 승격(os.rename). 동일 초 충돌 시 카운터 부여.
        #    ★exists() 선검사는 **이름 선택**에만 쓴다 — 충돌의 **판정은 rename 자신의 오류**다.
        #    gen_root(GEN_ROOT) 는 HOME 한 곳을 전 부서가 공유하고 phoenix-snapshot-6h 는 부서마다
        #    등록돼 있다 — 같은 초에 다른 프로세스가 같은 이름을 승격할 수 있고, 그때 선검사 결과는
        #    rename 시점에 이미 낡았다(검사와 행위 사이의 창). 선검사만 믿으면 그 창에서
        #    ENOTEMPTY 로 **세대가 통째로 유실**된다(2026-10-17 11:52:22Z 실측 · Errno 66).
        #    ★물러난 이름에도 선검사를 적용한다. ⛔초기 이름만 선검사하면, 경합으로 물러난
        #    `<stamp>-k` 가 **비어 있는** 채 이미 존재할 때 POSIX rename 이 **성공하면서 그것을 삼킨다**
        #    (rename 은 빈 디렉터리 대상만 조용히 대체한다) ⇒ 선검사를 거쳤을 때와 결과가 달라지는
        #    **조용한 행동 변경**이다. 독립 검증(reviewer-gemini · INFRA-PH1 F1)이 live 파일에서 실증했다.
        final_dir = os.path.join(gen_root, stamp)
        counter = 1
        while True:
            while os.path.exists(final_dir):
                if counter > 9999:
                    raise OSError(errno.ENOTEMPTY,
                                  "generation name space exhausted after 9999 candidates", final_dir)
                final_dir = os.path.join(gen_root, f"{stamp}-{counter}")
                counter += 1
            try:
                os.rename(tmp_dir, final_dir)  # POSIX: 동일 FS 내 원자적
                break
            except OSError as exc:
                # 경합자가 창 안에서 그 이름을 차지한 경우에만 다음 이름으로 물러난다.
                # ⛔다른 오류는 삼키지 않는다(원인 은폐 금지) · ⛔남의 세대를 덮어쓰지 않는다
                #   (비어 있지 않은 대상은 POSIX rename 이 거부하고, 그 거부가 여기서 신호다).
                if exc.errno not in (errno.ENOTEMPTY, errno.EEXIST) or counter > 9999:
                    raise
                final_dir = os.path.join(gen_root, f"{stamp}-{counter}")
                counter += 1
        tmp_dir = None
        _fsync_path(gen_root)
    finally:
        if tmp_dir is not None and os.path.isdir(tmp_dir):
            shutil.rmtree(tmp_dir, ignore_errors=True)

    gen_name = os.path.basename(final_dir)
    print("[snapshot] 세대 생성: %s  (파일 %d건, 누락 %d건, 선택부재 %d건)%s" %
          (gen_name, len(present), n_missing, len(optional_absent), incomplete))

    # 3) GC
    kept, deleted = do_gc(gen_root=gen_root, dry_run=False)
    if deleted:
        print(f"[gc] {len(deleted)}세대 삭제, {len(kept)}세대 보관")
    return gen_name


def _stamp_second(name):
    """이름의 초 단위 시각(접미 무시) = 부모(d478d97f)의 _parse_stamp. ⛔순서 규칙이 아니다 — compute_gc 의
    부모 바닥(_parent_floor)만 쓴다(B317-R1 M1). 세대의 순서는 언제나 _parse_stamp 하나다."""
    m = GEN_NAME_RE.match(name)
    if not m:
        return None
    return datetime.strptime(m.group(1), "%Y%m%dT%H%M%SZ").replace(tzinfo=timezone.utc)


def _parse_stamp(name):
    """★세대 순서 규칙 하나(B-320 + B317-R1 M2 · HEAD/CEO): 이름 시각, 그다음 **숫자** 충돌 접미.
    GC 정렬(compute_gc·_gen_effective_dt)과 card 의 기본 세대 선택(do_card)이 모두 이 함수로 순서를 정한다 —
    `<s>-10` 은 `<s>-9` 보다 나중이고(문자열 순서와 반대) `<s>-1` 은 `<s>` 보다 나중이다."""
    dt = _stamp_second(name)
    if dt is None:
        return None
    # ★B-320: 충돌 접미(-k)를 버리지 않는다. 종전엔 `<stamp>` 와 `<stamp>-1` 이 같은 시각으로 파싱돼 정렬 동률을
    #   **입력 순서**가 깼다(정책 아님 · 동시 발화한 두 레인 중 하나가 그날 유일 세대를 잃는다). k 마이크로초를 더해
    #   승격 순서(k 증가)대로 구별한다 — k ≤ 9999 < 1초이므로 초·날짜 귀속은 바뀌지 않는다.
    #   ★B317-R1(R1-a): 접미는 999999µs 에서 멈춘다 — do_snapshot 은 9999 를 넘는 접미를 만들지 않지만 손으로 만든 이름의
    #   거대 접미가 다음 초·다음 날로 넘거나(일별 대표 귀속이 바뀐다) timedelta OverflowError 로 GC·card 를 죽이지 않게 한다.
    #   1e6 이상끼리는 같은 시각이 되고, 그 동률은 호출자의 문자열 정렬이 깬다(생산자가 만들지 않는 이름이다).
    sfx = GEN_NAME_RE.match(name).group(2)
    if sfx:
        dt += timedelta(microseconds=min(int(sfx), 999999))
    return dt


def _gen_effective_dt(gen_root, name, nominal_of=_parse_stamp):
    """★E3(P2-5) 세대 실효 시각 — 이름의 명목 타임스탬프와 디렉터리 mtime 중 **더 나중**을 취한다.
    시계 역행 시 실제로 더 최근에 만든 세대가 이름은 lexically 작아지는데, mtime 이 이를 바로잡아 '최근'으로
    인식되게 한다(lexical 오선택·GC 오삭제 방어). 둘 다 없으면 None.
    nominal_of = 명목 시각 함수(기본 = 순서 규칙 _parse_stamp · 부모 바닥은 _stamp_second 로 부모 판정을 재현한다)."""
    nominal = nominal_of(name)
    mt = None
    try:
        mt = datetime.fromtimestamp(os.path.getmtime(os.path.join(gen_root, name)), tz=timezone.utc)
    except OSError:
        pass
    cands = [d for d in (nominal, mt) if d is not None]
    return max(cands) if cands else None


ABSENT_LANE_KEEP = 1     # 레지스트리에서 빠진(삭제된) 부서 레인 풀의 규칙 A 상한


def _parent_floor(generations, now, dt_eff):
    """부모(d478d97f) compute_gc 의 보관 집합 — 단일 풀 · 상한 KEEP_RECENT · 명목 = _stamp_second(접미 무시).
    입력은 list_generations 와 같은 문자열 정렬로 받는다(부모 do_gc 의 입력) — 같은 초의 동률은 부모처럼
    그 정렬 순서가 깬다. dt_eff = 부모의 실효 시각(max(_stamp_second, mtime) · 미주입 시 명목).
    ⛔정책의 순서 규칙이 아니다: 부모가 보관하던 것을 이 판이 지우지 않게 하는 **바닥**이다(B317-R1 M1)."""
    _E0 = datetime.min.replace(tzinfo=timezone.utc)
    keep = set()
    for keyfn in (_stamp_second, dt_eff):
        for name in sorted(generations, key=lambda n: (keyfn(n) or _E0), reverse=True)[:KEEP_RECENT]:
            keep.add(name)
    cutoff = now - timedelta(days=KEEP_DAILY_DAYS)
    days = set()
    for name in sorted(generations, key=lambda n: (_stamp_second(n) or _E0), reverse=True):
        dt = _stamp_second(name)
        if dt is None or dt < cutoff:
            continue
        day = dt.strftime("%Y%m%d")
        if day not in days:
            days.add(day)
            keep.add(name)
    return keep


def compute_gc(generations, now=None, dt_of=None, dt_eff=None, key_of=None, cap_of=None, floor_eff=None):
    """보관/삭제 결정. (keep_set, delete_list) 반환. 순수함수(테스트 가능).
    ★E3(P2-5 + gemini W6): 명목 시각(dt_of=이름)과 실효 시각(dt_eff=mtime 병행)을 **둘 다 보호**한다(union).
    - 명목-recent(이름 최신) KEEP_RECENT 는 항상 보존 → cp/touch 로 과거 세대 mtime 이 오염돼도 **진짜 최근 세대를
      밀어내지 못한다**(gemini 오염 왜곡 차단).
    - 실효-recent(mtime 최신) KEEP_RECENT 도 보존 → 시계 역행으로 이름이 작아진 실 최근 세대가 보존된다(P2-5).
    dt_eff 미주입 시 dt_of 와 동일(하위호환·명목만). 오염 세대가 실효 집합에 추가로 잔존할 수 있으나(무해·상한
    2×KEEP_RECENT), 진짜 최근 세대의 오삭제는 발생하지 않는다.
    ★B-321/결정 3(레인별 풀 · HEAD 형식 결재 2026-10-20): key_of(n) 은 그 세대가 담은 레인 키들의 반복자다 —
      한 세대는 자기가 담은 **모든** 레인의 풀에 든다(분할이 아니다). 규칙 A·B 는 풀 안에서 적용하고 keep 은 풀별
      keep 의 합집합이다. 단일 레인 세대가 섞인 저장소에서 한 레인의 세대가 다른 레인 세대 48개에 밀려 사라지던
      것(실측 8→6)을 막는다. ⛔CEO 문면의 「커버리지 집합 키로 분할」을 그대로 하지 않은 이유: 대체된 집합 키는 새
      구성원을 영영 얻지 못해 최신 48개가 **영구 보관**된다(모형: 30일 뒤 단일 레인 세대 60개 전부 잔존 · 부서 신설마다
      ~48세대 동결). 레인별 풀이면 전 레인 세대가 쌓이는 대로 옛 세대가 모든 풀에서 밀려나 정상 노화한다.
      key_of 미주입 = 단일 풀("") = 종전과 동일한 판정. 키 0개를 돌려준 세대도 단일 풀에 넣는다(⛔어느 풀에도
      없어 삭제되는 길 차단).
    cap_of(key) = 그 풀의 규칙 A 상한(미주입 = 전 풀 KEEP_RECENT). 상한 정책은 do_gc 의 _lane_caps 참조.
    ★B317-R1 M1(codex · 잠근 기준 「상한 변경은 삭제된 부서 레인만」): B-320 순서(접미 µs)는 같은 초 쌍의 승자를 바꾼다 —
      부모는 문자열 정렬상 앞(무접미)을, 이 판은 나중 승격(-1)을 대표로 고른다. 그대로면 부모가 보관하던 세대가 지워진다
      (반례: 어제 같은 초 쌍 + 오늘 48세대 → 부모 보관 무접미가 rmtree). 그래서 keep 에 **부모 판정(_parent_floor)을
      바닥으로 합집합**한다 — 전 레인 현존이면 keep ⊇ 부모 keep 이 구성상 성립한다. 유일한 예외는 위임된 것 하나:
      담은 레인이 **전부** 상한 축소 풀(삭제된 부서)인 세대는 바닥이 되살리지 않는다(최종 상태만 · 이력 미보관).
      floor_eff = 부모의 실효 시각(do_gc 가 max(_stamp_second, mtime) 을 넘긴다 · 미주입 = 명목만 — dt_eff 를 주입하는
      다른 호출자는 바닥의 실효 절반을 갖지 않는다)."""
    now = now or _now_utc()
    _E0 = datetime.min.replace(tzinfo=timezone.utc)
    dt_of = dt_of or _parse_stamp        # 명목(이름)
    dt_eff = dt_eff or dt_of             # 실효(mtime 병행) — 미주입 시 명목과 동일
    key_of = key_of or (lambda n: ("",))
    cap_of = cap_of or (lambda k: KEEP_RECENT)
    keys = {n: (tuple(key_of(n)) or ("",)) for n in generations}
    pools = {}
    for n in generations:
        for k in keys[n]:
            pools.setdefault(k, []).append(n)
    keep = set()
    cutoff = now - timedelta(days=KEEP_DAILY_DAYS)
    for k, pool in pools.items():
        cap = cap_of(k)
        # 규칙 A: 최근 cap — 명목 기준 ∪ 실효 기준(union → 명목-recent 절대 evict 안 됨·실효-recent 도 보존)
        for keyfn in (dt_of, dt_eff):
            ordered = sorted(pool, key=lambda n: (keyfn(n) or _E0), reverse=True)
            for name in ordered[:cap]:
                keep.add(name)
        if cap < KEEP_RECENT:
            continue   # 상한 축소 풀(삭제된 부서)은 일별 대표도 두지 않는다 — 최신 1건이 그 레인의 최종 상태다
        # 규칙 B: 최근 14일, 하루당 대표 1건 — 명목 시각 기준(날짜 귀속은 이름이 정직·오염 mtime 미의존)
        day_rep = {}
        for name in sorted(pool, key=lambda n: (dt_of(n) or _E0), reverse=True):
            dt = dt_of(name)
            if dt is None or dt < cutoff:
                continue
            day = dt.strftime("%Y%m%d")
            if day not in day_rep:
                day_rep[day] = name
                keep.add(name)

    for name in _parent_floor(sorted(generations), now, floor_eff or _stamp_second):
        if any(cap_of(k) >= KEEP_RECENT for k in keys[name]):
            keep.add(name)

    delete = [n for n in generations if n not in keep]
    return keep, sorted(delete)


def _is_lane_file(source):
    """레인 파일 = round/_round 디렉터리 바로 아래의 카드·노드 TODO·법 묶음. ⛔이름 패턴만으로 판정하지 않는다 —
    기억 저장소의 `x_TODO.md` 가 가짜 레인 풀(레지스트리 밖 = 상한 1)을 만들면 그 세대들이 지워진다(R1 H8)."""
    b = os.path.basename(source)
    if os.path.basename(os.path.dirname(source)) not in ("round", "_round"):
        return False
    return (b == ROUND_CARD or fnmatch.fnmatchcase(b, ROUND_TODO_GLOB)
            or fnmatch.fnmatchcase(b, LAW_CORPUS_GLOB))


def _coverage_keys(gen_root, name):
    """그 세대가 담은 레인 키 = 레인 파일(카드·노드 TODO·법 묶음) 소스의 디렉터리 집합(정렬 튜플).
    이 판독은 이 패치 전 세대(단일 레인 · canonical_cards 없음)에도 같은 규칙으로 통한다.
    · 레인 파일이 하나도 없는 세대 → ("",) = 종전 단일 풀(하네스·격리 세대).
    · manifest 판독 실패 → ("?",) 전용 풀 — ⛔측정 불능을 정상 레인 키에 접지 않는다."""
    mp = os.path.join(gen_root, name, "manifest.json")
    manifest = None
    for enc in ("utf-8", None):   # do_verify 와 같은 두 해석(옛 기본 인코딩 manifest)
        try:
            with open(mp, encoding=enc) as f:
                manifest = json.load(f)
            break
        except (OSError, ValueError):
            continue
    try:
        dirs = {os.path.dirname(e["source"]) for e in manifest.get("files", []) if _is_lane_file(e["source"])}
    except (AttributeError, KeyError, TypeError):
        return ("?",)
    return tuple(sorted(dirs)) or ("",)


def _lane_caps(home=HOME, depts_json=None):
    """풀 키 → 규칙 A 상한. ★CEO 비준(2026-10-20) 잔여 상한:
      · 레인 키가 depts.json 의 어느 부서(pack_dir/round)에도 없으면 = 삭제된 부서 → 상한 ABSENT_LANE_KEEP(1),
        그 레인을 담은 나머지 세대는 자기가 속한 다른 풀에서 노화한다.
        ★이유: **삭제된 부서의 최종 상태는 보관하고, 그 이력은 보관하지 않는다.**
      · 본부(base) 레인(`~/.cys/pack/round`·`~/_round`)은 부서 항목이 아니므로 **결코 부재가 아니다**(상한 48).
      · 레지스트리 판독 실패(파일 없음·읽기 불가·파싱 불가·꼴 불일치 — pack_dir 이 문자열이 아닌 항목 포함)는
        「부재」가 아니다: 측정 불능은 판정이 아니므로 **전 레인 48** 을 유지한다(조용히 이력을 지우지 않는다).
      · 레인이 아닌 풀("" 단일 풀 · "?" 판독 불능)은 48.
    ★부재 판정은 HOME 절대경로가 아니라 **구조**로 한다(R2): 부재가 될 수 있는 것은 `<팩>/round` 꼴이면서 본부 팩
      (`.../pack`)이 아닌 키뿐이고, 레지스트리 pack_dir 과 실경로 **또는 팩 이름**이 같으면 현존이다 — HOME 이 옮겨진
      뒤 옛 세대의 키(다른 접두)에서도 본부가 부재로 판정되어 이력이 지워지는 길을 닫는다. `_round` 레인(본부 카드
      자리·JAVIS_ROOT 프로젝트)은 부서 항목이 아니므로 부재가 아니다."""
    dj = depts_json or os.path.join(home, ".cys", "depts.json")
    registered = None
    try:
        with open(dj, encoding="utf-8-sig") as f:
            depts = json.load(f)["depts"]
        if not isinstance(depts, dict):
            raise TypeError("depts is not a mapping")
        rounds, names = set(), set()
        for meta in depts.values():
            pd = meta.get("pack_dir") if isinstance(meta, dict) else None
            if not (isinstance(pd, str) and pd):
                raise TypeError("dept entry without a pack_dir")
            rounds.add(os.path.realpath(os.path.join(pd, "round")))
            names.add(os.path.basename(os.path.normpath(pd)))
        registered = rounds
    except Exception:
        registered = None
    base_pack = os.path.realpath(os.path.join(home, ".cys", "pack"))

    def cap_of(key):
        if registered is None or key in ("", "?"):
            return KEEP_RECENT
        if os.path.basename(key) != "round":
            return KEEP_RECENT          # `_round` 레인 · 그 밖 꼴 = 부서 항목이 아니다
        pack = os.path.dirname(key)
        if os.path.basename(pack) == "pack" or os.path.realpath(pack) == base_pack:
            return KEEP_RECENT          # 본부 팩 — 부서 항목이 아니므로 결코 부재가 아니다
        if os.path.realpath(key) in registered or os.path.basename(pack) in names:
            return KEEP_RECENT
        return ABSENT_LANE_KEEP

    return cap_of, registered is not None


def do_gc(gen_root=GEN_ROOT, dry_run=False, home=HOME, depts_json=None):
    gens = list_generations(gen_root)
    cov = {n: _coverage_keys(gen_root, n) for n in gens}
    cap_of, registry_ok = _lane_caps(home=home, depts_json=depts_json)
    keep, delete = compute_gc(gens, dt_of=_parse_stamp,
                              dt_eff=lambda n: _gen_effective_dt(gen_root, n),
                              key_of=lambda n: cov[n], cap_of=cap_of,
                              floor_eff=lambda n: _gen_effective_dt(gen_root, n, nominal_of=_stamp_second))
    if not registry_ok and any(k not in ("", "?") for ks in cov.values() for k in ks):
        print("[gc] depts.json 판독 불가 — 레인 부재를 판정하지 않음(전 레인 보관 상한 %d)" % KEEP_RECENT)
    if dry_run:
        print(f"[gc dry-run] 총 {len(gens)}세대 → 보관 {len(keep)} / 삭제 {len(delete)}")
        for n in delete:
            print(f"  DELETE {n}")
        return sorted(keep), delete
    for n in delete:
        shutil.rmtree(os.path.join(gen_root, n), ignore_errors=True)
    if delete:
        _fsync_path(gen_root)
    return sorted(keep), delete


def do_verify(gen_root=GEN_ROOT, gen=None):
    """세대의 manifest 해시가 저장된 파일과 일치하는지 검증. 실패 시 exit 1."""
    gens = [gen] if gen else list_generations(gen_root)
    if not gens:
        print("[verify] 검증할 세대 없음", file=sys.stderr)
        return 1
    all_ok = True
    for name in gens:
        gdir = os.path.join(gen_root, name)
        mpath = os.path.join(gdir, "manifest.json")
        if not os.path.isfile(mpath):
            print(f"[verify] {name}: manifest.json 없음 — 반파 의심 FAIL")
            all_ok = False
            continue
        try:
            # ★(0.14.48 · A) 새 manifest 는 UTF-8 이다. 앞선 판은 기본 인코딩으로 썼으므로(cp949 프로세스가 쓴 manifest 가
            #   있을 수 있다) 두 해석을 후보로 두고 **저장 파일이 실제로 다 있는 해석**을 고른다 — 한글 이름의 cp949 바이트는
            #   UTF-8 로도 「읽히는」 경우가 있어(징.md → ¡.md) 디코딩 성공만으로는 가를 수 없다. 둘 다 아니면 먼저 읽힌 것.
            #   (같은 기본 인코딩의 기계에서 쓴 옛 세대까지다 — 다른 인코딩의 기계로 옮긴 옛 세대는 보장하지 않는다.)
            manifest = None
            _first_err = None
            for _enc in ("utf-8", None):
                try:
                    with open(mpath, encoding=_enc) as f:
                        _cand = json.load(f)
                except ValueError as _e:  # UnicodeDecodeError · JSONDecodeError
                    _first_err = _first_err or _e
                    continue
                if manifest is None:
                    manifest = _cand
                try:
                    if all(os.path.isfile(os.path.join(gdir, _x["stored_as"])) for _x in _cand.get("files", [])):
                        manifest = _cand
                        break
                except (AttributeError, KeyError, TypeError):
                    pass  # 꼴이 다른 manifest — 아래 본 검사가 종전대로 다룬다
            if manifest is None:
                raise _first_err
        except (OSError, ValueError) as e:
            print(f"[verify] {name}: manifest 파싱 실패 {e} FAIL")
            all_ok = False
            continue
        ok = True
        for entry in manifest.get("files", []):
            stored = os.path.join(gdir, entry["stored_as"])
            if not os.path.isfile(stored):
                print(f"[verify] {name}: 저장파일 없음 {entry['stored_as']} FAIL")
                ok = False
                continue
            actual = _sha256(stored)
            if actual != entry["sha256"]:
                print(f"[verify] {name}: 해시 불일치 {entry['stored_as']} FAIL")
                ok = False
        if ok:
            print(f"[verify] {name}: OK ({len(manifest.get('files', []))}파일)")
        else:
            all_ok = False
    return 0 if all_ok else 1


def do_card(gen_root=GEN_ROOT, gen=None, lane=BASE_LANE):
    """★결정 7 복원 경로(읽기 전용): 세대에서 그 레인의 정본 카드를 manifest `canonical_cards` 의 **경로 이름**으로
    찾아 `저장파일<TAB>sha256<TAB>원경로` 한 줄을 출력한다(select_card · 범위: 본부=~/_round, 부서=<pack>/round).
    ⛔쓰지 않는다 — 이 도구는 state-generations/ 밖에 쓰지 않는다. 사람이 출력된 저장 파일을 원 경로로 복사한다.
    gen 미지정 = 순서 규칙(_parse_stamp — GC 와 같은 규칙)상 최신 세대. ⛔문자열 최신(gens[-1])이 아니다: 같은 초의
    `-10` 은 문자열로 `-9` 앞에 서므로 그것을 고르면 더 묵은 카드를 복원하게 된다(B317-R1 M2).
    rc 0=찾음·해시 일치 / 1=세대 없음·정본 기록 없음·저장 파일 없음·해시 불일치."""
    gens = list_generations(gen_root)
    name = gen or (max(gens, key=_parse_stamp) if gens else None)
    if not name:
        print("[card] 세대 없음", file=sys.stderr)
        return 1
    gdir = os.path.join(gen_root, name)
    try:
        with open(os.path.join(gdir, "manifest.json"), encoding="utf-8") as f:
            manifest = json.load(f)
    except (OSError, ValueError) as e:
        print(f"[card] {name}: manifest 판독 실패 {e}", file=sys.stderr)
        return 1
    entry = select_card(manifest, lane)
    if entry is None:
        lanes = sorted((manifest.get("canonical_cards") or {}).keys())
        print(f"[card] {name}: 레인 {lane!r} 의 정본 카드가 이 세대에 없다(기록된 레인 {lanes}) — "
              "다른 카드로 대체하지 않는다. --gen 으로 다른 세대를 지정하라", file=sys.stderr)
        return 1
    stored = os.path.join(gdir, entry["stored_as"])
    if not os.path.isfile(stored) or _sha256(stored) != entry["sha256"]:
        print(f"[card] {name}: 저장 카드 {entry['stored_as']} 없음 또는 해시 불일치", file=sys.stderr)
        return 1
    print(f"{stored}\t{entry['sha256']}\t{entry['source']}")
    return 0


def do_self_test():
    """원자성/중단내성 실증. 격리 임시폴더에서만 수행(라이브 상태 불변)."""
    print("=== javis_state_snapshot self-test (격리 임시폴더) ===")
    # ★Windows(S5): 중단내성 T2 만 os.fork(POSIX 전용)에 의존하므로 그 케이스만 [SKIP] 하고 T1·T3~T6 은 실 수행한다
    # (블랜킷 skip 은 false-green 이라 폐기 — 원자성·GC·부서 커버리지는 Windows 에서도 실측 검증). 상세는 T2 블록 참조.
    workdir = tempfile.mkdtemp(prefix="cys-snaptest-")
    try:
        src_dir = os.path.join(workdir, "src")
        gen_root = os.path.join(workdir, "gen")
        os.makedirs(src_dir)
        src_file = os.path.join(src_dir, "topology.json")
        payload = json.dumps({"entries": [{"role": "worker"}], "n": 1}).encode()
        with open(src_file, "wb") as f:
            f.write(payload)
        src_hash = _sha256(src_file)
        sources = [src_file]

        # T1: 정상 세대 생성 + 무결성
        gen = do_snapshot(sources=sources, gen_root=gen_root)
        assert gen is not None, "T1: 세대 생성 실패"
        gens = list_generations(gen_root)
        assert len(gens) == 1, f"T1: 세대 수 {len(gens)} != 1"
        rc = do_verify(gen_root=gen_root)
        assert rc == 0, "T1: verify 실패"
        print("  [PASS] T1 정상 세대 생성 + manifest 해시 검증")

        # T2: 중단 시뮬레이션 — 승격 직전 os._exit(자식 프로세스에서).
        # ★Windows(S5): os.fork 는 POSIX 전용 → 이 케이스만 [SKIP](PASS 아님·나머지는 실행). 원자성 자체는 T1/T3 가
        #   os.rename(동일 볼륨 원자 승격)으로 커버하고, 승격 전 크래시의 반파 회피는 POSIX drill(mac CI)로 확증한다.
        if os.name == "nt":
            print("  [SKIP] T2 승격전-중단 내성은 os.fork(POSIX) 필요 — Windows 미지원(mac CI 가 확증). 나머지 케이스는 실행.")
        else:
            pid = os.fork()
            if pid == 0:
                # 자식: 크래시 훅으로 승격 직전 즉사
                try:
                    do_snapshot(sources=sources, gen_root=gen_root, crash_hook="before_rename")
                except BaseException:
                    pass
                os._exit(0)
            _, status = os.waitpid(pid, 0)
            gens_after = list_generations(gen_root)
            assert len(gens_after) == 1, f"T2: 중단인데 세대 증가 {len(gens_after)} (반파 승격 발생!)"
            tmp_leftover = [n for n in os.listdir(gen_root) if n.startswith(TMP_PREFIX)]
            print(f"  [PASS] T2 승격 직전 중단 → 최종 세대 미증가(반파 없음), .tmp 잔재 {len(tmp_leftover)}건")

        # T3: 죽은 pid + 충분한 나이의 잔재 청소(방금 죽은 pid 는 D-07 최소 보존 시간 적용)
        old_time = time.time() - 2 * 3600
        for name in os.listdir(gen_root):
            if name.startswith(TMP_PREFIX):
                os.utime(os.path.join(gen_root, name), (old_time, old_time))
        do_snapshot(sources=sources, gen_root=gen_root)
        tmp_leftover2 = [n for n in os.listdir(gen_root) if n.startswith(TMP_PREFIX)]
        assert len(tmp_leftover2) == 0, "T3: .tmp 잔재 미청소"
        assert len(list_generations(gen_root)) == 2, "T3: 세대 수 이상"
        print("  [PASS] T3 다음 실행이 반파 .tmp 잔재 청소 + 정상 세대 추가")

        # T3': 살아 있는 pid 는 오래된 잔재도 보호(fork 없이 Windows 에서도 실행)
        live_tmp = tempfile.mkdtemp(prefix=f"{TMP_PREFIX}{os.getpid()}-", dir=gen_root)
        os.utime(live_tmp, (old_time, old_time))
        _cleanup_tmp(gen_root)
        assert os.path.isdir(live_tmp), "T3': 살아 있는 pid 의 .tmp 잔재 삭제됨"
        shutil.rmtree(live_tmp)
        assert len(list_generations(gen_root)) == 2, "T3': 세대 수 이상"
        print("  [PASS] T3' 살아 있는 pid 의 오래된 .tmp 잔재 보호")

        # T4: 원본 불파괴 불변식
        assert _sha256(src_file) == src_hash, "T4: 원본 변조됨(불변식 위반!)"
        print("  [PASS] T4 원본 상태 파일 불파괴(읽기 전용 소비자)")

        # T5: GC 순수함수 — 60세대 합성, 최근48 + 일별14 보관 검증
        base = datetime(2026, 1, 1, tzinfo=timezone.utc)
        synth = []
        for i in range(60):
            synth.append(_gen_stamp(base + timedelta(hours=i)))
        now = base + timedelta(hours=59)
        keep, delete = compute_gc(synth, now=now)
        # 최근 48은 반드시 보관
        recent48 = sorted(synth, reverse=True)[:48]
        for n in recent48:
            assert n in keep, f"T5: 최근48 누락 {n}"
        assert len(keep) + len(delete) == 60, "T5: 보관+삭제 != 총량"
        print(f"  [PASS] T5 GC 정책(최근{KEEP_RECENT}+일별{KEEP_DAILY_DAYS}일): 60세대→보관{len(keep)}/삭제{len(delete)}")

        # T6: ★Phase7 부서 자동 커버리지 — 합성 state_root 에 메인 + 부서 2개(registry엔 1개만) 두고
        #     default_sources()가 손 배선/부서명 하드코딩 없이 전 부서 선언상태를 포함하는지, 그리고
        #     세대 스냅샷이 부서 선언파일을 실제로 담는지 실증(대형 .db/.log 은 제외됨도 확인).
        home6 = os.path.join(workdir, "home6")
        state_root6 = os.path.join(home6, ".local", "state")
        main6 = os.path.join(state_root6, "cys")
        os.makedirs(main6)
        with open(os.path.join(main6, "topology.json"), "w") as f:
            f.write('{"entries":[{"role":"worker"}]}')
        os.makedirs(os.path.join(home6, ".cys"))
        # registry 는 dept-A 만 등록(stale) — 하지만 디스크엔 dept-A·dept-B 둘 다 존재
        with open(os.path.join(home6, ".cys", "depts.json"), "w") as f:
            json.dump({"depts": {"dept-A": {"socket": os.path.join(state_root6, "cys-dept-dept-A", "cys.sock")}}}, f)
        for dep in ("cys-dept-dept-A", "cys-dept-dept-B"):
            dd = os.path.join(state_root6, dep)
            os.makedirs(dd)
            with open(os.path.join(dd, "schedule_state.json"), "w") as f:
                f.write('{"jobs":[]}')
            # 대형 산출물(포함되면 안 됨)
            with open(os.path.join(dd, "analytics.db"), "wb") as f:
                f.write(b"\x00" * 4096)
        srcs6 = default_sources(home=home6, state_root=state_root6)
        # 부서명 하드코딩 없이 stale-registry 밖의 dept-B 까지 발견돼야 함(파일시스템 truth)
        assert any("cys-dept-dept-A" in s and s.endswith("schedule_state.json") for s in srcs6), "T6: dept-A 선언상태 누락"
        assert any("cys-dept-dept-B" in s and s.endswith("schedule_state.json") for s in srcs6), "T6: dept-B(미등록) 누락 — 파일시스템 truth 실패"
        assert not any(s.endswith("analytics.db") for s in srcs6), "T6: 대형 .db 가 소스에 포함됨(제외 위반)"
        gen6_root = os.path.join(workdir, "gen6")
        do_snapshot(sources=srcs6, gen_root=gen6_root)
        gdir6 = os.path.join(gen6_root, list_generations(gen6_root)[0])
        man6 = json.load(open(os.path.join(gdir6, "manifest.json"), encoding="utf-8"))  # ★(0.14.48 · A) 쓰는 쪽이 UTF-8 이다
        stored_srcs = [e["source"] for e in man6["files"]]
        assert any("cys-dept-dept-B" in s for s in stored_srcs), "T6: 스냅샷에 부서 선언상태 미포함"
        assert do_verify(gen_root=gen6_root) == 0, "T6: 부서 포함 세대 무결성 실패"
        print("  [PASS] T6 부서 자동 커버리지: stale-registry 밖 부서까지 glob 발견·스냅샷 포함·대형 .db 제외·무결성 OK")

        # T7: ★경합 승격 — 선검사와 rename 사이의 창에서 다른 프로세스가 같은 이름을 차지한 경우.
        #     GEN_ROOT 는 HOME 한 곳을 전 부서가 공유하고 phoenix-snapshot-6h 는 부서마다 등록돼 있어
        #     같은 초 경합이 상시 가능하다. 2026-10-17 11:52:22Z 에 실제로 ENOTEMPTY(Errno 66)로
        #     세대 1건이 유실됐다. 창의 재현은 선검사를 낡게 만들면 된다 — exists() 가 False 를 답하는
        #     그 순간 그 이름을 **비어 있지 않게** 만든다.
        #     ⛔실프로세스 8개 동시 승격은 이 결함을 놓친다(창이 너무 좁다) — **재현되지 않은 경합은
        #       부재의 증거가 아니다**. 그래서 판정자는 주입이고 실경합은 보강일 뿐이다.
        #     ★주입은 **두 자리**를 뺏고 **끝까지 거짓말한다**(독립 검증 INFRA-PH1 F2-C):
        #       한 번만 거짓말하면 ⒜선검사를 두 번 하는 코드와 ⒝루프 없는 단발 재시도가 통과한다.
        #     ★그리고 승격 결과를 **반환 문자열이 아니라 디스크에서** 확인한다(F2-A·F2-B):
        #       경합자 세대의 **내용**까지 보고, 승격된 세대가 **실재하고 verify 를 통과**하는지 본다.
        #       ⛔반환값만 보는 어서션은, 세대를 통째로 잃고 이름만 돌려주는 구현을 통과시킨다.
        gen7_root = os.path.join(workdir, "gen7")
        os.makedirs(gen7_root)
        _real_exists = os.path.exists
        _racers = {}
        _marker = '{"generator":"the racer that got there first"}'

        def _stale_exists(path):
            p = str(path)
            b = os.path.basename(p)
            if os.path.dirname(p) != gen7_root or b.startswith(TMP_PREFIX) or not b.endswith("Z"):
                # <stamp>-k 는 basename 이 "Z" 로 끝나지 않으므로 여기서 걸러진다 → 아래 사전 조회로 처리
                if p in _racers:
                    return False        # ← 끝까지 낡은 답(한 번만 거짓말하면 재검사가 통과한다)
                return _real_exists(path)
            if not _racers:
                for name in (b, b + "-1"):      # 두 자리를 뺏는다 → 단발 재시도는 두 번째에서 죽는다
                    d = os.path.join(gen7_root, name)
                    os.makedirs(d, exist_ok=True)
                    with open(os.path.join(d, "manifest.json"), "w", encoding="utf-8") as rf:
                        rf.write(_marker)
                    _racers[d] = name
            return False if p in _racers else _real_exists(path)

        promote_err = None
        gen7 = None
        try:
            os.path.exists = _stale_exists
            gen7 = do_snapshot(sources=sources, gen_root=gen7_root)
        except OSError as exc:
            promote_err = exc
        finally:
            os.path.exists = _real_exists
        assert _racers, "T7: 주입 미발화 — 아무것도 검증되지 않았다(검사기 어휘만 증명한 꼴)"
        assert promote_err is None, (
            "T7: 경합 승격이 errno=%s 로 실패 — 세대 유실(%s)" % (getattr(promote_err, "errno", None), promote_err))
        for _vd, _vn in _racers.items():
            _vm = os.path.join(_vd, "manifest.json")
            assert os.path.isfile(_vm), "T7: 경합자 세대 %s 를 덮어썼다" % _vn
            with open(_vm, encoding="utf-8") as rf:
                assert rf.read() == _marker, "T7: 경합자 세대 %s 의 내용이 바뀌었다(덮어쓰기)" % _vn
        assert gen7 and gen7 not in _racers.values(), "T7: 경합자 이름으로 승격 %r" % gen7
        assert os.path.isdir(os.path.join(gen7_root, gen7)), (
            "T7: %r 를 반환했지만 디스크에 없다 — 이름만 돌려주고 세대를 잃었다" % gen7)
        assert do_verify(gen_root=gen7_root, gen=gen7) == 0, "T7: 승격된 세대 %s 가 verify 실패" % gen7
        print("  [PASS] T7 경합 승격: 자리 2개를 뺏겨도 세대 유실 없음(%s · 디스크 실재 · verify OK) · 경합자 %d건 불파괴"
              % (gen7, len(_racers)))

        # T8: ★B-320 — 충돌 접미가 시각에 남는다(같은 스탬프 두 세대가 같은 시각으로 접히지 않는다).
        a8 = _parse_stamp("20261018T235340Z")
        b8 = _parse_stamp("20261018T235340Z-1")
        assert b8 > a8 and b8.strftime("%Y%m%d%H%M%S") == a8.strftime("%Y%m%d%H%M%S"), "T8: 접미 무시 %r %r" % (a8, b8)
        print("  [PASS] T8 B-320 충돌 접미 보존(-1 이 더 나중 · 초·날짜 귀속 불변)")

        # T9: ★B-321 레인별 풀 — 오래된 본부 세대 6개 + 최신 단일 레인 세대 54개(같은 날)에서 본부 6개 전부 보관,
        #     레지스트리에서 빠진 레인은 최신 1건만.
        base9 = datetime(2026, 10, 19, 0, 0, tzinfo=timezone.utc)
        names9 = [_gen_stamp(base9 + timedelta(minutes=10 * i)) for i in range(60)]
        cov9 = {n: (("base",) if i < 6 else ("dept-%d" % (i % 7 + 1),)) for i, n in enumerate(names9)}
        keep9, _ = compute_gc(names9, now=base9 + timedelta(hours=10), key_of=lambda n: cov9[n])
        assert all(n in keep9 for n in names9[:6]), "T9: 레인별 풀인데 본부 세대가 밀려났다"
        keep9b, _ = compute_gc(names9, now=base9 + timedelta(hours=10), key_of=lambda n: cov9[n],
                               cap_of=lambda k: ABSENT_LANE_KEEP if k == "dept-7" else KEEP_RECENT)
        d7 = [n for n in names9 if cov9[n] == ("dept-7",)]
        assert [n for n in d7 if n in keep9b] == [d7[-1]], "T9: 부재 레인 상한 1(최신 보관) 위반"
        print("  [PASS] T9 B-321 레인별 풀: 본부 6/6 보관 · 부재 레인은 최신 1건만")

        # T10: ★결정 7 — 정본 카드는 레인별 경로 이름으로 고른다(본부=~/_round, 부서=<pack>/round · basename 대체 없음).
        m10 = {"canonical_cards": {"base": "/h/_round/SESSION_STATE.md",
                                   "dept-1": "/h/.cys/pack-dept-dept-1/round/SESSION_STATE.md"},
               "files": [{"source": "/h/.cys/pack/round/SESSION_STATE.md", "stored_as": "aa__SESSION_STATE.md"},
                         {"source": "/h/_round/SESSION_STATE.md", "stored_as": "bb__SESSION_STATE.md"},
                         {"source": "/h/.cys/pack-dept-dept-1/round/SESSION_STATE.md", "stored_as": "cc__SESSION_STATE.md"}]}
        assert (select_card(m10, "base") or {}).get("stored_as") == "bb__SESSION_STATE.md", "T10: 본부 정본 오선택"
        assert (select_card(m10, "dept-1") or {}).get("stored_as") == "cc__SESSION_STATE.md", "T10: 부서 정본 오선택"
        assert select_card({"files": m10["files"]}, "base") is None, "T10: 기록 없는 세대에서 basename 대체"
        print("  [PASS] T10 결정 7 레인별 정본 카드 선택(본부 ~/_round · 부서 <pack>/round · 대체 없음)")

        print("=== self-test 전체 PASS ===")
        return 0
    except AssertionError as e:
        print(f"=== self-test FAIL: {e} ===", file=sys.stderr)
        return 1
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


def main():
    # ★Windows 패리티(S5): 과거 os.name=="nt" hard-gate 는 제거됐다 — 디렉터리 fsync 를 Windows 에서 건너뛰고
    # (_fsync_path) os.rename(동일 볼륨 원자 승격)에 의존하므로 snapshot/list/gc/verify 가 Windows 에서 동작한다.
    ap = argparse.ArgumentParser(description="cys 상태 파일 세대 보관기 (무손실 복원 Phase 0)")
    sub = ap.add_subparsers(dest="cmd")

    p_snap = sub.add_parser("snapshot", help="세대 1건 생성 + GC")
    p_snap.add_argument("--dry-run", action="store_true")

    sub.add_parser("list", help="세대 목록")

    p_gc = sub.add_parser("gc", help="보관정책 적용")
    p_gc.add_argument("--dry-run", action="store_true")

    p_ver = sub.add_parser("verify", help="manifest 해시 무결성 검증")
    p_ver.add_argument("--gen", default=None)

    p_card = sub.add_parser("card", help="세대에서 레인별 정본 카드 찾기(읽기 전용 복원 경로)")
    p_card.add_argument("--gen", default=None)
    p_card.add_argument("--lane", default=BASE_LANE)

    sub.add_parser("self-test", help="원자성/중단내성 실증(격리)")

    args = ap.parse_args()
    cmd = args.cmd or "snapshot"

    if cmd == "snapshot":
        do_snapshot(dry_run=getattr(args, "dry_run", False))
        return 0
    if cmd == "list":
        gens = list_generations()
        print(f"세대 {len(gens)}건 @ {GEN_ROOT}")
        for n in gens:
            gdir = os.path.join(GEN_ROOT, n)
            nf = len([x for x in os.listdir(gdir) if x != "manifest.json"]) if os.path.isdir(gdir) else 0
            print(f"  {n}  ({nf}파일)")
        return 0
    if cmd == "gc":
        do_gc(dry_run=getattr(args, "dry_run", False))
        return 0
    if cmd == "verify":
        return do_verify(gen=getattr(args, "gen", None))
    if cmd == "card":
        return do_card(gen=args.gen, lane=args.lane)
    if cmd == "self-test":
        return do_self_test()
    ap.print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
