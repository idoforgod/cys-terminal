#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_todo_stamp — 기존 todo에 선언 블록 v1을 일괄 삽입하는 마이그레이터 (설계 P3-0).

왜 이 도구가 필요한가(SIM-1 실측): M1 시점 선언 보유율이 **0%(63/63 미선언)** 다. 최근 7일 내
수정된 파일이 87%라 "쓸 때 자연히 선언이 붙는다"는 기대가 있었으나, 그 회전만으로는 M2 목표인
미선언 <10%에 도달하지 못한다 — 미선언 파일은 대부분 *더 이상 쓰이지 않는* 파일이라 회전에서
빠지기 때문이다. 스탬프 도구가 없으면 P4-2(미선언 <10% 대기)가 영원히 대기 상태가 되고,
P4-3(휴리스틱 4규칙 삭제)·S2 전환 경로가 구조적으로 열리지 않는다.

설계 원칙 — **이 도구는 판정하지 않는다**:
  · 선언 유무 판정은 전부 `javis_todo_decl`(단일 파서·2언어 파리티 계약)에 위임한다.
    파서를 여기서 재구현하면 "스탬프 도구는 선언이 없다고 보는데 소비자는 있다고 보는" 불일치가
    생기고, 그 불일치는 곧 G7(선언 2개 = 미선언) 자해로 이어진다.
  · 생성한 선언 줄도 **쓰기 전에 파서로 검증**한다(dry-run 포함). 문자 클래스(G4)를 여기서
    다시 적으면 그것이 곧 두 번째 스펙이 된다.
  · 쓴 뒤에도 **파일을 다시 파서로 읽어** `counted` 판정을 확인하고, 아니면 원본으로 되돌린다.
    쓰고 나서 깨진 걸 아는 것은 늦다.

안전 규약(전부 "증거 보존" 원칙에서 나온다):
  · 기본이 dry-run. `--apply` 를 줘야 실제로 쓴다(`scripts/sync-pack.sh` 와 같은 관례).
  · 백업·세대 디렉터리(`state-generations/`·`.pre-`·`backup`·`archive`·`.bak`)는 무조건 제외.
    설계 R9 — `~/.cys/state-generations/` 에 todo 사본이 24개 실재한다. 거기에 선언을 박으면
    사본이 "살아있는 파일"의 자격을 얻어 제2의 유령원이 된다.
  · 유산 파일(`--stale-days` 초과 미수정)은 기본 제외. 종결된 레인을 되살리지 않는다.
  · 원자적 쓰기(임시파일 + os.replace) — 중간에 죽어도 원본이 깨지지 않는다.
  · mtime 보존(기본 켬) — 아래 `PRESERVE_MTIME_RATIONALE` 참조.

★승격 모드(`--promote-retire` · W15 교정 3 동반 · master 심판 2026-07-26):
  소비자가 유령(`stale_ghost`)을 park 차단에서 면제하기로 했는데, 운영자가 그 유령을
  **처분할 경로가 없었다** — 이 도구는 이미 `retired`로 파싱되는 파일(레거시 마커)을 무조건
  skip했고 미선언 유산 파일은 유산이라는 이유로 skip했다. 정책을 지키려면 도구가 그 정책을
  집행할 수 있어야 한다. 승격은 **이미 은퇴로 취급되고 있는 파일**만 대상으로 한다
  (레거시 마커 보유 / 미선언 + 유산). 신선한 미선언 파일은 절대 승격하지 않는다.

★임계 가시화(W18 교정 3 · reviewer1 경미 ㉰): `--include-stale`의 **대상 판정**이 유산 임계를
  쓰므로 임계를 낮추면 승격 대상이 넓어진다(`CYS_TODO_STALE_DAYS=1` → 2일 된 파일도 [promote]).
  `--apply` 없이는 무해하고 조작이 명시적이라 결함은 아니지만, **보이지 않는 상태**(환경변수)가
  파괴적 조작의 범위를 바꾸는 것은 그 자체로 위험하다. 그래서 적용 임계와 **출처**를 실행 헤더에
  항상 찍고, 기본값과 다르면 경고 한 줄을 띄운다. 임계 자체는 고정하지 않는다 — 운영자가
  의도적으로 조정할 여지는 남긴다(`resolve_stale_days`).

사용:
    javis_todo_stamp.py                          # dry-run (팩 round/ 스캔)
    javis_todo_stamp.py --dir /Users/x/proj/_round --scope pack
    javis_todo_stamp.py --apply                  # 실제 삽입 + 사후 파서 재검증
    javis_todo_stamp.py --promote-retire                     # 승격 dry-run
    javis_todo_stamp.py --promote-retire --include-stale --apply   # 유령까지 승격

exit 0=정상 / 1=쓰기·검증 실패(되돌림 포함) / 2=인자·환경 오류.
의존성: 파이썬 표준 라이브러리만(팩 규약 — 외부 의존 절대 금지).
"""

import argparse
import glob
import os
import re
import sys
import tempfile
import time

# 파서는 단일 구현이다(재구현 금지 — ADR-2 2언어 파리티 계약의 Python 측).
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.insert(0, _SELF_DIR)
import javis_todo_decl as todo_decl                                      # noqa: E402

# ── 상수 ────────────────────────────────────────────────────────────────────
TODO_GLOB = "*_TODO.md"
TODO_SUFFIX = "_TODO.md"
# ★유산 판정 임계 — `javis_report.STALE_DAYS_DEFAULT`와 **같은 값이어야 한다**(W14 S20).
# 종전 주석은 "동일 정신"이라고만 적혀 있었다. 정신은 계약이 아니다 — 갈리면 스탬프 대상
# 집합과 소비자의 유산 집합이 어긋나, 소비자는 유산으로 배제하는 파일에 스탬프가 선언을
# 박거나(증거 오염) 그 반대가 된다. `tests/test_todo_shared_constants.py`가 기계 대조한다.
STALE_DAYS_DEFAULT = 7
# 임계를 조정하는 환경변수 키(소비자 `javis_report`와 같은 knob). 이 값이 승격 모드의 **대상
# 집합**을 넓히므로, 적용값과 출처를 실행 헤더에 항상 찍는다(W18 교정 3 · `resolve_stale_days`).
STALE_DAYS_ENV = "CYS_TODO_STALE_DAYS"
MAX_FILE_BYTES = 1 << 20          # 1 MiB 초과는 todo가 아니다 — 통째 메모리 적재를 막는 상한
BOM = "\ufeff"

# 백업·세대·아카이브 경로 표식. **경로 문자열 어디에든** 들어 있으면 대상에서 뺀다(설계 R9).
# 넉넉하게 잡는 쪽이 옳다: 살아있는 파일 하나를 놓치는 비용(사람이 --dir 로 지목하면 됨)보다
# 백업 사본에 선언을 박는 비용(유령원 신설·증거 오염)이 훨씬 크다.
BACKUP_MARKERS = ("state-generations", ".pre-", "backup", "archive", ".bak")

# ── ★W15 교정 3 동반 — 명시 은퇴 **승격** 모드(master 심판 2026-07-26) ─────────────
#
# 왜 필요한가: 소비자가 유령(`orphan`·`stale`)을 park 차단에서 면제하기로 했는데, 운영자가
# 그 유령을 **처분할 경로가 없었다**. 이 도구는 이미 `retired`로 파싱되는 파일(레거시 마커)을
# 무조건 skip했고, 미선언 유산 파일은 유산이라는 이유로 skip했다. 그래서 유령·레거시 마커를
# **명시 은퇴 선언으로 승격**시킬 방법이 아예 없었다.
# 정책을 지키려면 도구가 그 정책을 집행할 수 있어야 한다 — 그 집행기가 `--promote-retire`다.
#
# 승격 대상은 **이미 은퇴로 취급되고 있는 파일**로 한정한다(아래 `_plan_promote`).
# 이 도구가 "아무 파일이나 은퇴시키는 무기"가 되면 그것 자체가 새 유령원이다.
STATUS_ACTIVE = "active"
STATUS_RETIRED = "retired"
# 상태 → 파서가 내야 할 판정. 검증을 문자열 상수 하나로 묶어 두 모드가 갈리지 않게 한다.
WANT_VERDICT = {STATUS_ACTIVE: "counted", STATUS_RETIRED: "retired"}

# 팩 이름 형태(`pack`, `pack-dept-dept-2` …). 팩들의 부모 디렉터리 아래 형제 중에서
# 이 형태를 가진 것만 scope 후보로 인정한다 — `~/.cys/state/`·`~/.cys/viewer/` 같은
# 비-팩 디렉터리가 scope로 승격되면 소비자가 foreign-scope로 조용히 배제해 버린다.
RE_PACK_NAME = re.compile(r"^pack(-[A-Za-z0-9._-]+)?$")

PRESERVE_MTIME_RATIONALE = (
    "스탬프가 mtime을 갱신하면 소비자의 stale 휴리스틱 시계가 리셋된다 — "
    "종결된 레인의 유산 파일이 '방금 수정된 활성 파일'로 되살아난다"
)


# ── 환경 ────────────────────────────────────────────────────────────────────
# ★팩 경로 env 키의 우선순위 목록(W14 S19). Rust 정본 `src/pack.rs::PACK_DIR_ENV_KEYS`와
# **같은 목록·같은 순서**여야 한다 — `tests/test_todo_shared_constants.py`가 기계 대조한다.
# 종전에는 이 목록이 3종으로 갈려 있었고, `cys todo-path`가 `AITERM_JARVIS_DIR`를 인식하지
# 못해 레거시 env 환경에서 **생성 위치와 스캔 위치가 갈려 파일이 보고기에 영영 보이지 않았다**.
PACK_DIR_ENV_KEYS = ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR")

def pack_dir():
    """팩 경로. 키 목록·순서는 `PACK_DIR_ENV_KEYS`(Rust `src/pack.rs`와 기계 대조).

    (javis_report를 import해 재사용하지 않는 이유: 마이그레이터가 소비자 모듈에 의존하면
     소비자가 깨지는 순간 마이그레이터까지 죽는다. 5줄 중복이 그 결합보다 싸다.
     ★그 대가로 "값이 갈릴 여지가 없다"는 **주장**은 계약이 아니므로, W14 S19에서 목록을
     상수로 뽑고 기계 대조를 박았다 — 종전 주석이 정확히 그 자리에서 틀려 있었다.)
    """
    for key in PACK_DIR_ENV_KEYS:
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


def default_stale_days():
    """기본 유산 임계. 소비자와 같은 knob(CYS_TODO_STALE_DAYS)을 존중하되, 실제 적용값은
    실행 헤더에 항상 찍어 '보이지 않는 상태가 판정을 바꾸는' 상황을 만들지 않는다."""
    try:
        v = int(os.environ.get(STALE_DAYS_ENV, "").strip() or STALE_DAYS_DEFAULT)
    except ValueError:
        return STALE_DAYS_DEFAULT
    return v if v >= 0 else STALE_DAYS_DEFAULT


def resolve_stale_days(arg_days):
    """★W18 교정 3 — 적용 임계값과 **그 출처**를 함께 낸다. 반환 = (days, source, note)

    왜 출처까지 내는가(reviewer1 경미 ㉰): `--promote-retire --include-stale`의 **대상 판정**이
    이 임계를 쓰므로, 임계를 낮추면 승격 대상이 넓어진다. 실측 —
        CYS_TODO_STALE_DAYS=1 javis_todo_stamp.py --promote-retire --include-stale
    에서는 기본 임계(7일)에선 보호되던 2일 된 파일이 `[promote]` 대상이 된다.
    `--apply` 없이는 무해하고 조작이 명시적이라 **결함은 아니지만**, 파괴적 조작의 범위를
    **보이지 않는 상태**(환경변수)가 바꾸는 것은 그 자체로 위험하다.

    심판: 임계 자체는 **고정하지 않는다**(운영자가 의도적으로 조정할 여지를 남긴다). 대신
    적용값과 출처를 헤더에 항상 찍고, 기본값과 다르면 경고 한 줄을 띄운다.

      days   실제 적용값(대상 판정에 쓰는 그 값)
      source 사람이 읽는 출처 표기
      note   운영자에게 알려야 할 이례 상태(없으면 None) — 환경변수가 설정됐는데 해석 불가라
             조용히 기본값으로 폴백한 경우가 대표적이다(그 침묵이 곧 오해의 원인이다).
    """
    env_raw = os.environ.get(STALE_DAYS_ENV, "").strip()
    if arg_days is not None:
        note = None
        if env_raw:
            note = ("환경변수 %s=%s 는 --stale-days 인자에 덮여 무시됐다"
                    % (STALE_DAYS_ENV, env_raw))
        return arg_days, "--stale-days 인자", note
    days = default_stale_days()
    if not env_raw:
        return days, "기본값", None
    if days == STALE_DAYS_DEFAULT and env_raw != str(STALE_DAYS_DEFAULT):
        return days, "기본값", ("환경변수 %s=%s 는 해석 불가(정수 아님·음수)라 기본값 %d일로 "
                                "폴백했다" % (STALE_DAYS_ENV, env_raw, STALE_DAYS_DEFAULT))
    return days, "환경변수 %s=%s" % (STALE_DAYS_ENV, env_raw), None


# ── 대상 선정 ───────────────────────────────────────────────────────────────
def is_backup_path(path):
    """백업·세대·아카이브 경로인가(설계 R9). 소문자 전체 경로 부분일치 — 결정론."""
    low = os.path.abspath(path).lower()
    return any(m in low for m in BACKUP_MARKERS)


def discover(dirs):
    """대상 *_TODO.md 절대경로(정렬·중복 제거).

    소비자(javis_report.discover_todo_files)와 동일하게 **비재귀** glob이다. 소비자가 보지
    않는 깊이의 파일에 선언을 박아 봤자 지표는 1도 움직이지 않고, 재귀는 백업 트리를 통째로
    빨아들이는 사고 경로만 넓힌다.
    """
    found = set()
    for d in dirs:
        for p in glob.glob(os.path.join(d, TODO_GLOB)):
            if os.path.isfile(p):
                found.add(os.path.abspath(p))
    return sorted(found)


def owner_from_filename(path):
    """WORKER_2_TODO.md → worker-2 (소문자·언더스코어→하이픈).

    javis_report.node_label과 **같은 정규화**여야 한다. 라벨은 report_gate의 pending·stall·
    idle 조인 키이고, 한 글자만 어긋나도 조인이 조용히 전패한다.
    """
    base = os.path.basename(path)
    if base.endswith(TODO_SUFFIX):
        return base[: -len(TODO_SUFFIX)].lower().replace("_", "-")
    return ""


def _is_within(path, root):
    path, root = os.path.normpath(path), os.path.normpath(root)
    return path == root or path.startswith(root + os.sep)


def scope_for_path(path, pack_root):
    """대상 파일이 속한 팩 basename. 판정 불가면 None(→ `--scope` 요구).

    추측하지 않는 것이 요점이다. 잘못된 scope는 소비자에서 orphan-scope 오배제를 낳고,
    그것은 이 아키텍처가 없애려던 바로 그 사고(살아있는 파일의 조용한 소멸)의 재현이다.
    """
    real = os.path.realpath(path)
    root = os.path.realpath(pack_root)
    if _is_within(real, root):                       # ① 내 팩 안 = my_scope
        return os.path.basename(root)
    packs_parent = os.path.dirname(root)             # ② 형제 팩(부서 팩 등)
    cur = os.path.dirname(real)
    while True:
        if (os.path.dirname(cur) == packs_parent
                and RE_PACK_NAME.match(os.path.basename(cur))
                and os.path.isdir(cur)):
            return os.path.basename(cur)
        nxt = os.path.dirname(cur)
        if nxt == cur:
            return None
        cur = nxt


def build_decl_line(owner, scope, status=STATUS_ACTIVE):
    """선언 줄 생성 + **파서 왕복 검증**. 반환 = (line, None) 또는 (None, 사유).

    문자 클래스(G4)·필수 키(G5)를 여기서 다시 검사하지 않는다 — 검사식이 두 벌이 되는 순간
    그중 하나는 반드시 뒤처지고, 뒤처진 쪽이 소비자와 갈린다. 생성물을 파서에 먹여
    기대 판정이 나오는지 보는 것이 유일한 계약 준수 증명이다.
    """
    want = WANT_VERDICT[status]
    line = "<!-- javis:todo v1 owner=%s scope=%s status=%s -->" % (owner, scope, status)
    decl, diag = todo_decl.parse(line + "\n")
    if decl is None:
        return None, "선언 생성 실패(%s: %s)" % (getattr(diag, "code", "?"), diag)
    if todo_decl.classify(decl, scope, lambda s: True) != want:
        return None, "선언 생성 실패(%s 미달)" % want
    return line, None


# ── 쓰기 ────────────────────────────────────────────────────────────────────
def compose(original_text, decl_line):
    """선언 줄을 **최상단**에 넣고 빈 줄 1개를 뒤에 둔다. 기존 내용은 한 글자도 바꾸지 않는다.

    BOM은 앞에 남긴다(G8): 파서는 선두 BOM만 제거하므로 BOM을 선언 뒤로 밀어도 판정은
    통과하지만, 그러면 파일의 인코딩 표식이 본문 중간으로 이동해 다른 도구가 깨진다.
    """
    if original_text.startswith(BOM):
        return BOM + decl_line + "\n\n" + original_text[len(BOM):]
    return decl_line + "\n\n" + original_text


def atomic_write_bytes(path, data, mode):
    """임시파일 + os.replace. 실패 시 임시파일을 지우고 원본은 손대지 않는다.

    같은 디렉터리에 임시파일을 만드는 이유: /tmp 경유 rename은 파일시스템 경계를 넘으면
    원자성이 깨진다(복사+삭제로 강등).
    """
    d = os.path.dirname(path) or "."
    fd, tmp = tempfile.mkstemp(prefix=".javis_todo_stamp.", suffix=".tmp", dir=d)
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.chmod(tmp, mode)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def verify_written(path, scope, status):
    """쓴 파일을 파서로 다시 읽어 기대 판정이 나오는지 확인한다(사후 게이트).

    scope_exists는 True 고정 — 여기서 묻는 것은 "내가 넣은 선언이 유효하고 내 scope로
    집계되는가"이지 디스크 상태가 아니다. scope 실재 여부는 소비자의 관심사다.

    ★승격 모드에서는 `_legacy` 부재도 함께 요구한다(W15). 레거시 마커가 남아 있는 파일에
    은퇴 선언을 넣으면 판정은 어느 쪽이든 `retired`라 **판정만으로는 승격 성공을 구분할 수
    없다** — 파서가 우리가 넣은 **명시 선언**을 읽었다는 사실까지 확인해야 왕복 검증이다.
    """
    want = WANT_VERDICT[status]
    decl, diag = todo_decl.parse(todo_decl.read_head(path))
    if decl is None:
        return False, "파서 재검증 실패(%s: %s)" % (getattr(diag, "code", "?"), diag)
    verdict = todo_decl.classify(decl, scope, lambda s: True)
    if verdict != want:
        return False, "파서 재검증 실패(판정=%s · 기대=%s)" % (verdict, want)
    if decl.get("_legacy"):
        return False, "파서 재검증 실패(여전히 레거시 마커로 읽힌다 — 승격 미성립)"
    if decl.get("status") != status:
        return False, "파서 재검증 실패(status=%s · 기대=%s)" % (decl.get("status"), status)
    return True, None


# ── 계획 수립 ───────────────────────────────────────────────────────────────
def _owner_scope(path, args, pack_root):
    """owner·scope 산출. 반환 = (owner, scope, None) 또는 (None, None, 사유)."""
    owner = args.owner or owner_from_filename(path)
    if not owner:
        return None, None, ("owner 산출 실패(파일명이 %s 형태가 아님 · --owner 필요)"
                            % TODO_SUFFIX)
    scope = args.scope or scope_for_path(path, pack_root)
    if not scope:
        return None, None, "scope 미확정(팩 밖 경로) — --scope 필수(추측 금지)"
    return owner, scope, None


def _plan_promote(path, args, pack_root, decl, diag, is_stale, age_days):
    """★W15 — 명시 은퇴 **승격** 계획(`--promote-retire`). 반환 = (action, detail).

    승격 대상은 **이미 은퇴로 취급되고 있는 파일** 둘뿐이다. 이 경계가 이 모드의 안전장치
    전부다 — 넓히면 이 도구가 "아무 파일이나 은퇴시키는 무기"가 되고, 그것 자체가 새 유령원이다.
      ① 레거시 은퇴 마커 보유(`_legacy`) — 파서가 이미 `retired`로 확정한 파일.
      ② 미선언(`no-decl`) + **유산**(경과일 초과) — 소비자가 `stale_ghost`로 배제 중인 유령.
         ②는 `--include-stale`을 **명시적으로** 요구한다. "우리 추론으로 배제 중인 파일에
         주인 대신 은퇴 도장을 찍는" 행위라 마찰이 있어야 한다.

    ★신선한 미선언 파일은 절대 승격하지 않는다 — 살아있는 작업일 수 있고, 그것을 은퇴시키는
    것이 이 프로젝트가 고치려던 사고(살아있는 파일의 조용한 소멸) 그 자체다.
    """
    if decl is not None:
        if not decl.get("_legacy"):
            return "skip", "이미 명시 선언 보유(status=%s) — 승격 불필요" % decl.get("status")
        # ① 레거시 마커 — 유산 여부와 무관하게 승격 가능하다(status=retired라 레인 부활 위험이
        #    없다. 유산 게이트는 `active` 스탬프가 종결 레인을 되살리는 것을 막는 장치다).
    else:
        code = getattr(diag, "code", None)
        if code != "no-decl":
            return "skip", "선언 오작성(%s: %s) — 수동 교정 필요" % (code, diag)
        if not is_stale:
            return "skip", "미선언·신선(%d일) — 승격 대상 아님(살아있는 작업 보호)" % int(age_days)
        if not args.include_stale:
            return "skip", ("유령 승격은 --include-stale 필요(%d일 미수정 · 주인 대신 "
                            "은퇴 도장을 찍는 행위다)" % int(age_days))

    owner, scope, err = _owner_scope(path, args, pack_root)
    if err:
        return "skip", err
    line, err = build_decl_line(owner, scope, STATUS_RETIRED)
    if line is None:
        return "skip", err
    return "promote", line


def plan_file(path, args, pack_root, now, stale_days):
    """파일 1건의 처분 결정. 반환 = (action, detail)
       action = "stamp"(detail=선언 줄) | "promote"(detail=은퇴 선언 줄) | "skip"(detail=사유)."""
    if is_backup_path(path):
        return "skip", "백업·세대 경로(증거 보존)"

    try:
        st = os.stat(path)
    except OSError as e:
        return "skip", "stat 실패(%s)" % e.__class__.__name__
    if st.st_size > MAX_FILE_BYTES:
        return "skip", "파일 과대(%d바이트 > %d)" % (st.st_size, MAX_FILE_BYTES)

    age_days = (now - st.st_mtime) / 86400.0
    is_stale = stale_days > 0 and age_days > stale_days

    # ★자해 방어: 선언 후보가 이미 있으면 절대 건드리지 않는다. 하나 더 넣으면 G7(선언 2개 =
    # 미선언)로 **선언 보유율이 오히려 떨어진다**. 판정은 전적으로 파서에 맡긴다.
    decl, diag = todo_decl.parse(todo_decl.read_head(path))

    if args.promote_retire:
        return _plan_promote(path, args, pack_root, decl, diag, is_stale, age_days)

    # 유산 판정 — 종결 레인을 되살리지 않는다. --include-stale 로만 포함.
    if is_stale and not args.include_stale:
        return "skip", "유산 의심(%d일 미수정 · --include-stale 로만 포함)" % int(age_days)

    if decl is not None:
        if decl.get("_legacy"):
            return "skip", ("레거시 은퇴 마커 보유(기계가 이미 retired로 판정 · 명시 은퇴로 "
                            "올리려면 --promote-retire)")
        return "skip", "이미 유효 선언 보유(owner=%s scope=%s status=%s)" % (
            decl.get("owner"), decl.get("scope"), decl.get("status"))
    code = getattr(diag, "code", None)
    if code != "no-decl":
        # 선언 후보는 있는데 깨졌다 — 스탬프하면 후보가 2개가 되어 영구 duplicate가 된다.
        return "skip", "선언 오작성(%s: %s) — 수동 교정 필요" % (code, diag)

    owner, scope, err = _owner_scope(path, args, pack_root)
    if err:
        return "skip", err

    line, err = build_decl_line(owner, scope)
    if line is None:
        return "skip", err
    return "stamp", line


# ── 적용 ────────────────────────────────────────────────────────────────────
def apply_stamp(path, decl_line, preserve_mtime):
    """실제 삽입. 반환 = (True, None) 또는 (False, 사유). 실패 시 원본은 반드시 원상이다."""
    try:
        with open(path, "rb") as f:
            original = f.read()
        st = os.stat(path)
        mode = st.st_mode & 0o7777
        times = (st.st_atime, st.st_mtime)
    except OSError as e:
        return False, "읽기 실패(%s)" % e.__class__.__name__

    text = original.decode("utf-8", "replace")
    if text.encode("utf-8") != original:
        # 손상 인코딩 파일을 lossy 왕복하면 본문이 조용히 바뀐다 — 그것은 "한 글자도 바꾸지
        # 않는다"는 계약 위반이므로 아예 건드리지 않는다.
        return False, "인코딩 손상(무손실 왕복 불가) — 수동 처리 필요"

    new_bytes = compose(text, decl_line).encode("utf-8")
    try:
        atomic_write_bytes(path, new_bytes, mode)
    except OSError as e:
        return False, "쓰기 실패(%s: %s)" % (e.__class__.__name__, e)

    if preserve_mtime:
        # ★ PRESERVE_MTIME_RATIONALE 참조. 소비자 stale 판정의 시계를 스탬프가 흔들면 안 된다.
        try:
            os.utime(path, times)
        except OSError:
            pass                                     # 보존 실패는 치명 아님(파일은 정상)

    scope, status = _scope_status_of_line(decl_line)
    ok, err = verify_written(path, scope, status)
    if not ok:
        try:                                         # 되돌림 — 검증 실패분을 남기지 않는다
            atomic_write_bytes(path, original, mode)
            os.utime(path, times)
            return False, err + " → 원본 복원 완료"
        except OSError as e2:
            return False, err + " → ★원본 복원 실패(%s) 수동 확인 필요" % e2
    return True, None


def _scope_status_of_line(decl_line):
    """자기 생성물에서 scope·status를 되읽는다 — 파서가 유일한 해석기다(문자열 재파싱 금지)."""
    decl, _ = todo_decl.parse(decl_line + "\n")
    d = decl or {}
    return d.get("scope"), d.get("status")


# ── CLI ─────────────────────────────────────────────────────────────────────
def build_parser():
    p = argparse.ArgumentParser(
        description="기존 todo에 선언 블록 v1을 일괄 삽입한다(기본 dry-run).")
    p.add_argument("--dir", action="append", default=None, metavar="D",
                   help="스캔 폴더(반복 지정 가능). 기본=팩 round/")
    p.add_argument("--apply", action="store_true",
                   help="실제로 쓴다(미지정 시 dry-run — 무엇이 바뀔지만 표시)")
    p.add_argument("--owner", default=None, help="owner 명시(기본=파일명 역추론)")
    p.add_argument("--scope", default=None, help="scope 명시(팩 밖 경로에서는 필수)")
    p.add_argument("--stale-days", type=int, default=None, metavar="N",
                   help="N일 초과 미수정 파일은 유산으로 보고 제외(0=비활성)")
    p.add_argument("--include-stale", action="store_true",
                   help="유산 의심 파일도 포함(종결 레인 부활 위험 — 의도적으로만)")
    p.add_argument("--promote-retire", action="store_true",
                   help="★승격 모드 — 레거시 은퇴 마커·유령(미선언 유산) 파일을 "
                        "**명시 은퇴 선언**(status=retired)으로 올린다. 유령 승격은 "
                        "--include-stale 동반 필수. 기본 dry-run.")
    p.add_argument("--no-preserve-mtime", dest="preserve_mtime", action="store_false",
                   help="mtime을 보존하지 않는다(기본은 보존)")
    p.set_defaults(preserve_mtime=True)
    return p


def main(argv=None):
    args = build_parser().parse_args(argv)
    pack_root = pack_dir()
    dirs = args.dir if args.dir else [os.path.join(pack_root, "round")]
    dirs = [os.path.abspath(d) for d in dirs]
    for d in dirs:
        if not os.path.isdir(d):
            print("폴더 없음: %s" % d)
            return 2
    stale, stale_src, stale_note = resolve_stale_days(args.stale_days)
    if stale < 0:
        print("--stale-days 는 0 이상이어야 한다")
        return 2

    now = time.time()
    targets = discover(dirs)
    mode = "PROMOTE-RETIRE" if args.promote_retire else "STAMP"
    print("== javis-todo-stamp: 폴더 %d개 · 대상 %d건  (%s · %s) ==" % (
        len(dirs), len(targets), mode, "APPLY" if args.apply else "DRY-RUN"))
    # ★W18 교정 3 — 적용 임계와 **출처**를 항상 함께 찍는다(승격 모드의 대상 집합을 정하는 값이다).
    print("   유산 임계 %s (출처: %s) · mtime 보존 %s%s" % (
        ("%d일" % stale) if stale else "비활성", stale_src,
        "켬" if args.preserve_mtime else "끔",
        " · 유산 포함" if args.include_stale else ""))
    if stale_note:
        print("   ⚠ %s" % stale_note)
    # 기본값과 다르면 경고 — 파괴적 조작(승격)의 **범위**가 바뀌었다는 사실을 조작 전에 알린다.
    # 임계 자체는 고정하지 않는다(운영자의 의도적 조정 여지를 남긴다) — 보이지 않게만 두지 않는다.
    if args.promote_retire and stale != STALE_DAYS_DEFAULT:
        # 방향: 0 < stale < 기본 = 더 신선한 파일까지 유산으로 본다(대상 **확대** · 위험한 쪽).
        #       stale == 0(비활성)이나 기본 초과 = 유산 판정이 줄어든다(대상 **축소**).
        wider = 0 < stale < STALE_DAYS_DEFAULT
        print("   ⚠ 유산 임계가 기본값(%d일)과 다르다(적용 %s · 출처: %s) — 미선언 파일의 승격 "
              "대상 집합이 %s.%s"
              % (STALE_DAYS_DEFAULT, ("%d일" % stale) if stale else "비활성(0)", stale_src,
                 "넓어졌다" if wider else "좁아졌다",
                 " 기본 임계였다면 보호됐을 파일이 포함될 수 있다." if wider else ""))

    done = skipped = failed = 0
    for path in targets:
        action, detail = plan_file(path, args, pack_root, now, stale)
        if action == "skip":
            skipped += 1
            print("  [skip %s] %s" % (detail, path))
            continue
        if not args.apply:
            done += 1
            print("  [%s] %s" % (action, path))
            print("      %s" % detail)               # 삽입될 선언 줄 전문
            continue
        ok, err = apply_stamp(path, detail, args.preserve_mtime)
        if ok:
            done += 1
            print("  [%s] %s" % (action, path))
            print("      %s" % detail)
        else:
            failed += 1
            print("  [FAIL %s] %s" % (err, path))

    print("== 요약: 대상 %d · %s %d · skip %d%s ==" % (
        len(targets), "승격" if args.promote_retire else "스탬프",
        done, skipped, (" · 실패 %d" % failed) if failed else ""))
    if not args.apply:
        print("(dry-run — 실제 반영: 같은 명령에 --apply 추가)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
