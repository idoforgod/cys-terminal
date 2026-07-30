#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_report — 5분 주기 진행% 보고의 결정론 산출기 (절대지침: 양방향 소켓 앵커 A6).

master가 "대략 60% 된 것 같다"고 LLM으로 추론하면 환각이다. 진행%는 todo 체크박스의
done/total 산술로만 결정된다 — 이 스크립트 출력이 유일한 사실이다(결정론 환원).

수행:
  ① `cys status --json`으로 노드 현황·feed·idle·데몬 집계 todo를 수집
  ② 전 노드의 `*_TODO.md`(pack/round + 각 surface cwd/_round)를 직접 스캔해
     체크박스(- [x]/- [ ])로 노드별·종합 진행%를 산출
  ③ 주인님께 보고할 텍스트(또는 --json)를 출력

사용: python3 javis_report.py [--json] [--extra-dir <폴더> ...]
의존성: 파이썬 표준 라이브러리 + PATH의 `cys`(없으면 todo 직접 스캔만으로 동작).
"""

import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import time

# ── 선언 기반 판정 (C1 · DESIGN_declared-state.md §4-5 · M1 병행 단계) ─────────────
# 파서는 `javis_todo_decl`이 단일 구현이다(재구현 금지 — 2언어 파리티 계약 ADR-2의 Python 측).
# 임포트 실패는 **치명이 아니다**: 팩이 부분 갱신된 스큐 상태에서도 소비자는 계속 돌아야 한다
# (ADR-2 "상호 전제 금지"). 파서가 없으면 선언 경로만 꺼지고 아래 4규칙 휴리스틱으로 전량 폴백한다.
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)
try:
    import javis_todo_decl as _decl
except ImportError:                       # 구버전 팩 = 선언 미지원. 휴리스틱만으로 정상 동작.
    _decl = None

# governance.rs check_todo와 동일한 체크박스 규칙 (done/total 집계의 단일 진실).
RE_DONE = re.compile(r"- \[[xX]\]")
RE_OPEN = re.compile(r"- \[ \]")
IDLE_ALERT_SECS = 300  # 절대지침 B3: idle 5분+ 즉시 조치 대상

# ── 유령 todo 차단 (2026-07-26 결함 · 공유 폴더 유산 파일이 현재 편대 모수로 유입) ──
# 근거: cwd/_round 스캔은 "cwd의 _round는 현재 편대 소유"를 가정하나, 역사 있는 공유 프로젝트
# 폴더에서는 깨진다(07-11~07-20 종결 레인의 todo 4건이 07-26 편대 집계에 유입·301항목 오염).
# 배제는 삭제가 아니다 — 전건을 excluded[]에 사유와 함께 노출한다(무언의 절삭 금지).
# ── 파싱 예산 — **파서가 단일 진실**이다(★W13 교정 2 · master 심판) ────────────────
# 종전에는 여기 `RETIRE_SCAN_BYTES = 8192`라는 **두 번째 예산**이 있었다. 머리말 *정의*만
# 파서에서 빌리고 *예산*은 빌리지 않았으므로, 1 KiB 밖·8 KiB 안의 은퇴 선언을 휴리스틱만 보고
# 데몬(Rust · 1 KiB)은 못 봤다 — 같은 파일을 두고 팩은 "은퇴", 데몬은 "집계 중"이라 말하는
# 2소비자 불일치다(reviewer1 실측). 규칙을 여기 다시 적는 순간 그것이 곧 다음 drift라고
# 적어 놓고 **예산이라는 두 번째 기준**을 남긴 것이 결함이었다. 하나로 모은다.
#
# ⚠아래 리터럴 1024는 "값의 사본"이 아니라 **파서가 아예 없을 때(ADR-2 팩 부분갱신 스큐)만**
#   쓰는 최후 기본값이다. 파서가 있으면 그 값을 그대로 채택하므로 두 값이 갈릴 수 없다.
HEAD_BYTES = _decl.HEAD_BYTES if _decl is not None else 1024

# ★전방호환(2026-07-26 시뮬레이션 SIM-2 발견): 차기 선언(파서가 모르는 v2+)의 `status=retired`도
# 지금부터 인식한다. 읽는 쪽을 쓰는 쪽보다 **먼저** 배포해야 마이그레이션 중 구버전 소비자가
# 은퇴한 todo를 계속 집계하는 스큐 구멍이 생기지 않는다(실측으로 구멍 확인 후 선반영).
#
# ★W13 교정 1(a) — 이 패턴도 **줄 전체 앵커**다(`^<!--`). 종전에는 줄 어디에나 걸리는 부분일치라
#   `규약: 레인이 끝나면 javis:todo v1 선언의 status=retired 로 바꾼다.` 같은 평범한 머리말
#   산문이 파일을 통째로 은퇴시켰다(reviewer1 실측). 패턴이 아니라 **적용 범위**가 결함이었다.
RE_FUTURE_RETIRED = re.compile(
    r"^<!--[ \t]*javis:todo[ \t]+v\d+[ \t][^>]*\bstatus=retired\b", re.I)

# ADR-2 스큐 전용 — **파서가 없을 때만** 쓰는 레거시 마커 축약 사본(정본은
# `javis_todo_decl.is_retire_marker_line`). 파서가 있으면 이 정규식은 평가되지 않는다.
#
# ★W15 교정 2 — 이 사본도 **주석 안 앵커**로 맞춘다. 종전에는 `<!--.*(토큰)` 이라 주석 안
#   어디든 부분일치했고, 그래서 `<!-- 이 파일은 STALE 무효화 대상이 **아니다** -->` 라는
#   **부정문이 파일을 은퇴시켰다**. `<!--` 와 토큰 사이에는 장식 문자만 허용한다
#   (문자 집합은 `javis_todo_decl.DECOR_CHARS`와 같아야 하며 `tests/test_todo_shared_constants.py`
#   가 **행동으로** 대조한다 — 정규식 문자클래스와 문자열 리터럴은 형태가 달라 문자열
#   대조가 불가능하므로, 같은 입력에 같은 답을 내는지로 묶는다).
RE_LEGACY_RETIRE_SKEW = re.compile(
    r"^<!--[ \t★☆*=#~_+-]*(?:retired|javis:todo-retired|stale[ \t]*무효화)"
    r"|^javis:todo-retired\Z|^stale[ \t]*무효화\Z", re.I)
# 비정본 위치 todo의 유산 판정 임계(0=비활성).
# ★`javis_todo_stamp.STALE_DAYS_DEFAULT`와 **같은 값이어야 한다**(W14 S20 — 기계 대조는
# `tests/test_todo_shared_constants.py`). 갈리면 스탬프 대상 집합 ≠ 소비자 유산 집합이다.
STALE_DAYS_DEFAULT = 7


def stale_days():
    """CYS_TODO_STALE_DAYS 환경변수(정수·0=비활성). 파싱 실패는 기본값(결정론)."""
    try:
        v = int(os.environ.get("CYS_TODO_STALE_DAYS", "").strip() or STALE_DAYS_DEFAULT)
    except ValueError:
        return STALE_DAYS_DEFAULT
    return v if v >= 0 else STALE_DAYS_DEFAULT


# ★팩 경로 env 키의 우선순위 목록(W14 S19). Rust 정본 `src/pack.rs::PACK_DIR_ENV_KEYS`와
# **같은 목록·같은 순서**여야 한다 — `tests/test_todo_shared_constants.py`가 기계 대조한다.
# 종전에는 이 목록이 3종으로 갈려 있었고, `cys todo-path`가 `AITERM_JARVIS_DIR`를 인식하지
# 못해 레거시 env 환경에서 **생성 위치와 스캔 위치가 갈려 파일이 보고기에 영영 보이지 않았다**.
PACK_DIR_ENV_KEYS = ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR")

def pack_dir():
    """팩 경로. 키 목록·순서는 `PACK_DIR_ENV_KEYS`(Rust `src/pack.rs`와 기계 대조)."""
    for key in PACK_DIR_ENV_KEYS:
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


def my_scope():
    """내 팩 식별자 = pack_dir()의 basename. **하드코딩 금지** — 본사는 `pack`, 부서는
    `pack-dept-dept-2` 등으로 팩 이름 자체가 정체성이다(설계 §4-1 `scope` 필드)."""
    return os.path.basename(os.path.normpath(os.path.abspath(pack_dir())))


def scope_exists(scope):
    """선언된 scope가 **디스크에 실재하는 팩**인가 — 팩의 형제 디렉터리 존재로만 판정한다.

    파서(`javis_todo_decl`)에는 파일시스템을 넣지 않는 것이 설계다(픽스처가 계약·ADR-2).
    그래서 디스크 조회는 소비자인 여기서 콜러블로 주입한다. 판정 입력이 "디렉터리 존재"라
    시간 의존이 없고 결정론이다.

    이 판정이 필요한 이유(§4-2 R2 교정): scope가 남의 팩이라고 **무조건 조용히 배제**하면
    부서 teardown·재생성·팩 개명 시 살아있는 파일이 통째로 사라져 07-11 사고를 거울상으로
    재현한다. 실재하면 정상(조용한 배제), 실재하지 않으면 orphan-scope로 시끄럽게 알린다.
    """
    if not scope or scope in (".", "..") or os.sep in scope or (os.altsep and os.altsep in scope):
        return False                      # 경로 탈출 방어(G4 값 문법상 정상 선언엔 나올 수 없다)
    parent = os.path.dirname(os.path.normpath(os.path.abspath(pack_dir())))
    return os.path.isdir(os.path.join(parent, scope))


def cys_status():
    """`cys status --json` 수집. cys 부재·실패 시 None(=todo 직접 스캔만)."""
    cys = shutil.which("cys")
    if not cys:
        return None
    try:
        r = subprocess.run([cys, "status", "--json"], capture_output=True, timeout=10)
        if r.returncode != 0:
            return None
        return json.loads(r.stdout.decode("utf-8", "replace"))
    except Exception:
        return None


def count_checkboxes(path):
    """(done, total) — 64KB 상한(거대 파일 방어, governance.rs와 동일 정신)."""
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            content = f.read(65536)
    except OSError:
        return None
    done = len(RE_DONE.findall(content))
    total = done + len(RE_OPEN.findall(content))
    return done, total


def node_label(filename):
    """MASTER_TODO.md → master, REVIEWER_GEMINI_TODO.md → reviewer-gemini ...

    ★2026-07-26 수정: 언더스코어를 하이픈으로 역정규화해 **role 라벨공간과 일치**시킨다.
    생성자(javis_orchestra 티켓 빌더)는 role을 '대문자화+하이픈→언더스코어'로 파일명화하는데,
    여기서 소문자화만 하면 'reviewer_gemini'가 되어 role 'reviewer-gemini'와 영원히 조인되지
    않는다(javis_report_gate의 pending/todo_labels/node_is_idle 조인이 조용히 전패 → 배정된
    노드를 '무배정'으로 오분류, stall 담당노드 생사를 '미지'로 보수 승격 → 영구 오발화).
    파일명이 이미 하이픈판(REVIEWER-GEMINI_TODO.md)이어도 같은 라벨로 수렴한다(중복행 해소).
    """
    base = os.path.basename(filename)
    if base.endswith("_TODO.md"):
        return base[: -len("_TODO.md")].lower().replace("_", "-")
    return base


def decl_label(decl, roles=None):
    """선언이 유효하면 라벨은 **`owner`** 다 — 파일명→역할 추론을 소거한다(D3 해소 · 교정 3).

    설계는 `owner`가 파일명 추론을 없앤다고 적었는데 소비자 3곳 어디도 이 값을 읽지 않았다.
    그 결과 `owner=master`로 선언된 `WORKER_TODO.md`를 아무도 잡지 못했다 — G5가 owner를
    필수로 강제하면서 정작 값은 검증도 사용도 하지 않는 상태였다(reviewer1 재현).

    ⚠ 라벨은 `javis_report_gate`의 pending·stall·idle **조인 키**다. role 라벨공간
    (`reviewer-gemini`)과 한 글자라도 어긋나면 조인이 조용히 전패하므로 `node_label`과
    **같은 정규화**(소문자·언더스코어→하이픈)로 수렴시킨다. 이 수렴은 회귀 테스트로 핀돼 있다.

    반환 None = 폴백(`node_label(path)`). 선언이 없거나 owner가 ADR-4 C-3 센티널 `"?"`
    (레거시 은퇴 마커 = 주인 미상)면 파일명이 유일한 단서이므로 폴백이 옳다.

    ★W13 교정 3(master 심판 2026-07-26) — **role 실재를 검증한다.** 정규화만 하고 대조를
    하지 않으면 선언 owner가 라벨공간을 벗어나는 순간 `javis_report_gate`의 조인이 다시
    전패한다. reviewer1 실측: `owner=worker-2` 선언 + live role `worker` → 게이트가 그 노드를
    **'무배정'으로 오분류**해 매 주기 WARN(`idle_5min:worker`) 대신 엣지 1회 + 쿨다운
    (`idle_edge:worker`)으로 강등, 결과적으로 **2시간 침묵**했다.
    ★발현원이 우리 문서다 — 설계 §4-1 정본 예시가 `owner=worker-2`였고 데몬 테스트 픽스처도
    같았다. 손기재자가 문서를 그대로 따르면 즉시 발현하는 구조였다(문서도 이번에 교정).

    `roles` = `live_roles(status)`의 반환값. **None이면 검증하지 않는다** — status 미수집은
    "role이 없다"가 아니라 "알 수 없다"이고, 알 수 없을 때 owner를 버리면 선언이 명시한 주인을
    추론(파일명)으로 덮는 셈이라 ADR-1을 거스른다. 보수적으로 owner를 채택한다.
    """
    if not decl:
        return None
    owner = (decl.get("owner") or "").strip()
    if not owner or owner == "?":
        return None
    label = owner.lower().replace("_", "-")
    if roles is not None and label not in roles:
        return None                       # role 라벨공간 밖 = 조인 불능 → 파일명 폴백 + 진단
    return label


def discover_todo_files(status, extra_dirs):
    """스캔 대상 *_TODO.md 절대경로 집합 — 결정론(존재 파일만, 정렬).
    ① pack/round  ② status의 각 surface cwd/_round  ③ --extra-dir/_round ④ CYS_TODO_DIRS."""
    roots = [os.path.join(pack_dir(), "round")]
    if status:
        for s in status.get("surfaces", []):
            # live_cwd(현재 cd 위치) 우선, 없으면 spawn-time cwd — 워커 cd 이동 시 누락 방지.
            for cwd in (s.get("live_cwd"), s.get("cwd")):
                if cwd:
                    roots.append(os.path.join(cwd, "_round"))
    for d in extra_dirs or []:
        roots.append(os.path.join(d, "_round"))
        roots.append(d)
    for d in os.environ.get("CYS_TODO_DIRS", "").split(os.pathsep):
        if d:
            roots.append(d)
    found = set()
    for root in roots:
        for p in glob.glob(os.path.join(root, "*_TODO.md")):
            if os.path.isfile(p):
                found.add(os.path.realpath(p))
    return sorted(found)


def pct(done, total):
    """결정론 진행률(%) — total 0은 0%(미착수)로 정의. 정수 내림."""
    return 0 if total == 0 else (done * 100) // total


def is_retired(path):
    """머리말에 무효화 **선언**이 박힌 todo인가(사람 마커 'STALE 무효화' + 기계 마커 javis:todo-retired).

    07-11 사고(신규 agy가 STALE 사본을 자발 인수)와 07-26 사고(집계기가 STALE 사본을 집계)는
    같은 뿌리다 — 마커가 사람 눈에만 보이고 기계는 읽지 않았다. 여기서 기계도 읽게 한다.

    ★적대검증 교정(2026-07-26): 문구를 8KB 안 아무 데서나 찾으면 **살아있는 작업을 자해**한다.
    실측 — "- [ ] STALE 무효화 마커를 기계가 읽도록 구현"처럼 이 결함을 고치는 워커의 todo 항목이
    자기 자신을 배제시켰다. 그래서 선언의 **위치**를 계약에 넣는다:
      ① 첫 체크박스보다 앞선 머리말 구간에서만 유효(체크박스가 시작되면 그 뒤는 '내용'이다)
      ② 마커 줄 자체가 체크박스 항목이면 무효(할 일로 적은 것은 선언이 아니다)
    무효화는 파일 최상단에 선언하는 것이 규약이므로 이 제약은 정당한 선언을 하나도 잃지 않는다.

    ★W12 교정(2026-07-26 master 실측): ①만으로는 부족하다. 머리말 = "첫 체크박스 이전 **전량**"
    으로 잡으면 코드펜스·인용문·들여쓴 예시 안의 선언까지 진짜 선언으로 읽는다. 실측 재현 —
    머리말에 ```펜스```로 감싼 예시 선언(`status=retired`)을 적고 미완 2건을 가진 살아있는
    파일이 `excluded=[('worker','retired','heuristic')]`로 통째로 소실됐다. 파서 경로는 W9의
    G12 머리말 마스킹으로 이미 `unclaimed`를 냈는데, **휴리스틱 폴백만 옛 정의로 남아** 경계
    사이로 샌 것이다.

    ★특히 위험한 이유: `retired`는 "주인이 끝났다고 명시 선언한 것"이라 아래 `pending_outside`
    (구조적 불변식 · 교정 2-b)의 **면제 대상**이다. 즉 이 오분류만은 마지막 방어선인 QUIET
    불변식조차 잡지 못한다 — 다른 버킷(shadowed·orphan·stale)이었다면 미완 작업이
    `pending_outside_nodes[]`에 남아 park 오발동을 막았다.

    ★두 번째 기준을 만들지 않는다: 머리말 정의는 파서(`javis_todo_decl.header_lines`)가 단일
    구현이고 여기서는 **재사용만** 한다. 마스킹 규칙을 여기 다시 적는 순간 그것이 곧 다음
    drift이고, 이 결함이 정확히 그렇게 태어났다(파서만 고치고 소비자가 옛 정의로 남았다).

    ★W13 교정 2(master 심판 2026-07-26): 정의만이 아니라 **예산도** 파서에서 빌린다. 종전에는
    머리말 정의는 빌리고 예산은 `RETIRE_SCAN_BYTES=8192`로 따로 뒀다 — 그래서 1 KiB 밖·8 KiB
    안의 은퇴 선언을 팩(휴리스틱)만 보고 데몬(Rust · 1 KiB)은 못 보는 2소비자 불일치가 났다
    (reviewer1 실측). "규칙을 여기 다시 적는 순간 그것이 다음 drift"라고 적어 두고 예산이라는
    두 번째 기준을 남긴 것이 결함이었다.

    ★W13 교정 1(a): 마커 판정도 **줄 전체 앵커**다(`javis_todo_decl.is_retire_marker_line` +
    미래 버전용 `RE_FUTURE_RETIRED`). 부분일치이던 종전 판정은 평범한 머리말 산문 한 줄로
    살아있는 파일을 은퇴시켰다.

    ⚠파서 부재(ADR-2 팩 부분갱신 스큐)일 때만 옛 머리말 정의(첫 체크박스 이전 전량)와 축약
      마커 사본(`RE_LEGACY_RETIRE_SKEW`)으로 폴백한다. 그 구간에서는 **펜스 안 예시 선언이
      은퇴로 오분류될 위험이 그대로 남는다**(마커 앵커는 폴백에서도 유지된다). 그럼에도
      fail-open 구조를 유지하는 쪽이 옳다: 여기서 예외를 던지면 팩 스큐 한 번에 진행% 보고
      전체가 죽는다.
    """
    head = _decl.read_head(path) if _decl is not None else _read_head(path)
    for line in _retire_scan_lines(head):
        if _line_declares_retire(line):
            return True
    return False


def _read_head(path, limit=HEAD_BYTES):
    """파서 부재(ADR-2 스큐) 구간의 `javis_todo_decl.read_head` 대역 — 선두 limit **바이트**."""
    try:
        with open(path, "rb") as f:
            raw = f.read(limit)
    except OSError:
        return ""
    return raw.decode("utf-8", "replace")


def _line_declares_retire(line):
    """이 줄이 "주인이 은퇴를 밝힌 줄"인가 — 마커 판정의 단일 진입점(W13 교정 1a).

    ① 파서가 모르는 **미래 버전**(v2+)의 `status=retired` 선언 줄(SIM-2 전방호환).
    ② 레거시 은퇴 마커 줄 — 판정은 파서가 단일 구현(`is_retire_marker_line`)이다.
       파서 부재 시에만 축약 사본으로 폴백한다.

    둘 다 **줄 전체 앵커**다. 부분일치로 두면 이 규약을 설명하는 산문이 자기를 은퇴시킨다.
    """
    s = line.strip(" \t")
    if RE_FUTURE_RETIRED.match(s):
        return True
    if _decl is not None:
        return _decl.is_retire_marker_line(s)
    return RE_LEGACY_RETIRE_SKEW.match(s) is not None


def _retire_scan_lines(head):
    """무효화 마커를 찾아도 되는 줄 목록 — 파서의 머리말 정의를 그대로 쓴다(W12).

    파서가 있으면 `javis_todo_decl.header_lines`(G1'+G12: 첫 체크박스 이전 **중** 코드펜스·
    인용문·들여쓴 코드블록 밖 · 미닫힘 펜스는 회수)가 유일한 기준이다. 없으면 옛 정의(첫
    체크박스 이전 전량)로 폴백하며, 그 경우 자해 위험이 남는다는 것을 위 주석이 명시한다.
    """
    if _decl is not None:
        return _decl.header_lines(head)
    out = []
    for line in head.splitlines():
        if RE_DONE.search(line) or RE_OPEN.search(line):
            break                             # 체크박스 시작 = 머리말 종료(선언 구간 밖)
        out.append(line)
    return out


# ── ★T-0147-2 층4 — 판정 근거 단일화(설계 `_round/T-0147-2-DESIGN-wakeup-demotion.md` §2 층4) ──
#
# 게이트(`javis_report_gate`)는 idle 판정을 `live_nodes[].idle_secs` 와 `idle_nodes[]` **두 갈래**
# 에서 문자열 라벨로 조인해 왔다. 같은 사실을 두 곳에서 읽으면 어느 쪽이 진실인지·언제 잰
# 값인지가 사라지고, 그 공백이 오발화(설계 §0-3 라벨공간 교차조인 실패)의 서식지였다.
# 그래서 **출처와 샘플 시각이 박힌 단일 권위 레코드**를 방출한다: `role_measurements[]`.
#
# ⚠두 번째 판정 기준을 만들지 않는 것이 이 조항의 핵심이다 — 모집단·값은 전부 `live_nodes`
#   항목에서 파생하고(아래 `_measurement_from_node`), 여기서 새로 status 를 훑지 않는다.
MEASURE_SOURCE_STATUS = "daemon.status"
MEASURE_SOURCE_NONE = "unavailable"
MEASURE_SOURCE_IDLE = "daemon.status.idle_secs"


def _measurement_from_node(entry, sampled_at):
    """`live_nodes[]` 항목 1건 → 권위 측정 레코드 1건(설계 층4).

    `idle_secs` 가 없거나 int 가 아니면 값은 `null`, source 는 `unavailable` 이다 —
    **0 으로 접지 않는다**. 0 은 "쟀고 방금 움직였다"는 뜻이라, 미측정을 0 으로 접는 순간
    fail-closed 여야 할 게이트가 fail-open 으로 뒤집힌다.
    """
    idle = entry.get("idle_secs")
    measured = isinstance(idle, int)      # 술어는 `idle_nodes` 선별과 동일(두 기준 금지)
    return {
        "role": entry.get("role"),
        "idle_secs": idle if measured else None,
        "sampled_at": sampled_at,
        "source": MEASURE_SOURCE_IDLE if measured else MEASURE_SOURCE_NONE,
        "agent_alive": entry.get("agent_alive"),
        # ★§2-C fail-closed 확증의 2차 증거 — **idle 판정에는 절대 쓰지 않는다**(아래
        #   live_nodes 산출부의 계약 주석 참조). ②자기보고 증거 / ③usage 토큰 델타.
        "status_age_secs": entry.get("status_age_secs"),
        "usage_ctx_tokens": entry.get("usage_ctx_tokens"),
    }


def live_roles(status):
    """status의 role 집합(소문자). status 미수집이면 None(=귀속 판정 불가 → 배제 안 함)."""
    if not status:
        return None
    roles = {(s.get("role") or "").lower() for s in status.get("surfaces", []) if s.get("role")}
    return roles or None


# ── ★W15 교정 3 — 집계 밖 미완의 **두 갈래**(master 심판 2026-07-26) ────────────────
#
# 종전에는 `pending_outside_nodes[]` 전체가 park를 막았다. 그 결과 07-26 형태의 유령이
# 존재하는 동안 `quiet_branch_holds`가 **영원히** False였다 — **유령을 성공적으로 배제한
# 바로 그 사실이 세션 주차를 영구 차단**한 것이다. 유령은 정의상 오래된 파일이라 저절로
# 사라지지 않으므로 이 차단은 시간이 풀어주지 않는다.
#
# 갈래를 나누는 기준은 **누가 그렇게 말했는가**다.
#   PENDING_STALE_GHOST_BUCKETS  우리 **추론**이고 담당 role도 이미 사라진 것
#   그 외                        주인 **불명**(우리가 마지막 관측자다)
#
# ⚠휴리스틱 `retired`는 여기에 넣지 않는다 — 그 판정은 관측 사실이 아니라 **문자열 매칭**이고,
#   W13 치명 A의 자해(산문 한 줄이 파일을 은퇴시킴)가 정확히 그 경로로 들어와 마지막
#   방어선까지 통과했다. 오탐의 성질이 달라 같은 특권을 줄 수 없다.
#
# ── ★W18 교정 1 — `stale`을 면제에서 **뺀다**(master 심판 2026-07-26 · reviewer1 자기반박 채택) ──
#
# W15는 휴리스틱 `orphan`·`stale`을 한 갈래로 묶었으나 **둘의 차이는 담당 role의 생사**다
# (`classify_files`의 분기: role이 없으면 `orphan`, **살아 있으면** `stale`).
#   · `orphan` = 담당 role이 편대에서 사라진 파일 = **종결 레인**. 07-26 유령 4파일이 전부
#                이 형태였고, W15가 풀려던 livelock은 이 갈래만으로 그대로 해소된다.
#   · `stale`  = 담당 role이 **살아 있는데** 임계일 넘게 손대지 않은 파일. 살아있는 담당자의
#                미완을 **우리 추론(mtime)만으로** 침묵시키는 것은 ADR-3("놓치는 것보다
#                시끄러운 편이 안전")과 A3 교훈에 정면으로 어긋난다.
#
# ★기각한 반론("실측 실패 모드가 과소탐지뿐이므로 과대탐지 위험은 근거가 없다") — 이는
#   **관측되지 않았다는 것을 안전하다는 뜻으로 읽은 것**이고, 이 프로젝트가 반복해서 당한
#   오류의 형태 그 자체다(07-11·07-26 둘 다 "지금까지 문제없었다"가 무너진 사건이다).
#   게다가 M1 구간에는 실측상 todo가 **전부 미선언**이라 현존 파일 전량이 이 휴리스틱의
#   사정권에 있고, 며칠 단위 조사 레인에서 todo를 임계일 동안 안 건드리는 일은 이 조직에서
#   실제로 일어난다. 그때 `stale`이 면제이면 살아있는 담당자의 미완을 들고 주차한다.
#
# ⚠`roles is None`(status 미수집)이면 분기가 전부 `stale`로 떨어진다 — 즉 편대 현황을 모르는
#   상태에서는 어떤 파일도 park 면제를 받지 못한다. 이것도 보수적 방향이라 의도한 결과다.
PENDING_STALE_GHOST_BUCKETS = ("orphan",)
PENDING_KIND_UNRESOLVED = "unresolved"
PENDING_KIND_STALE_GHOST = "stale_ghost"


def pending_kind(bucket, source):
    """집계 밖 미완 1건의 갈래. `stale_ghost`만 park 차단에서 면제된다(보고에는 계속 노출).

    `source == "heuristic"`을 함께 요구하는 것이 계약이다 — 같은 이름의 버킷이 미래에
    선언 경로에서 생기면 그때는 "주인이 말한 것"이므로 판정이 달라야 한다.

    ★W18 — 면제는 휴리스틱 `orphan`(담당 role 부재) **하나뿐**이다. `stale`(담당 role 생존)은
    `unresolved`로 남아 park를 막는다. 위 `PENDING_STALE_GHOST_BUCKETS` 블록 참조.
    """
    if source == "heuristic" and bucket in PENDING_STALE_GHOST_BUCKETS:
        return PENDING_KIND_STALE_GHOST
    return PENDING_KIND_UNRESOLVED


def classify_files(files, status, now):
    """각 *_TODO.md를 active / 배제 / 미선언으로 판정한다 — 결정론.
    반환 (nodes, excluded, unclaimed, pending_outside, decl_stats)

    [판정 우선순위 · C1 M1 병행 단계]
      ① **선언이 1순위**(ADR-1). `javis_todo_decl.parse/classify`가 유효 판정을 내면 파일명·
         위치·mtime·role 생존은 **판정에 쓰지 않는다**. 5분기 = counted / retired /
         foreign-scope / orphan-scope / unclaimed. 라벨도 선언의 `owner`에서 온다(교정 3).
      ② 선언이 없거나 깨졌으면(=unclaimed) **아래 기존 4규칙 휴리스틱으로 폴백**한다.
         이 폴백은 M1의 필수 조건이다 — 실측상 현존 todo 63개가 **전부 미선언**이라 폴백을
         빼면 진행%가 그 즉시 0/0이 된다(SIM-1).
      ③ 폴백에서도 배제되지 않은 미선언 파일은 **집계에 그대로 남고**(온보딩 방어 ②·
         report_gate 무회귀) 동시에 `unclaimed[]` 관측 버킷에 미러링된다. 이 버킷은 M2의
         미선언 비율(`decl_stats`) 측정 근거이지 경고가 아니다(온보딩 방어 ①).
      M3에서 ②가 삭제되면 미선언 파일은 `unclaimed[]`에만 남는다(설계 P4-3).

    [★nodes[] 잔류 규칙 — master 심판 2026-07-26 (교정 1)]
      `nodes[]`에서 **조용히 빼도 되는 것은 두 판정뿐**이다:
        · `retired`       — 주인이 "끝났다"고 명시 은퇴를 선언했다
        · `foreign-scope` — **실재하는** 다른 팩이 주인이라고 스스로 밝혔다(그 팩이 본다)
      `unclaimed`와 `orphan-scope`는 **판정 불능**이지 부재가 아니다. 빼면 게이트의
      `in_progress_tasks`가 `nodes[]`만 읽으므로 "시끄러운 보고"가 사람이 읽는 텍스트에만
      남고 기계 소비자에겐 완전 침묵이 된다 → false QUIET/park(A3 재발). 도달 조건은
      scope 오타 1글자·부서 팩 개명·teardown = **이 조직에서 실제로 일어난 사건**이다.
      Rust 데몬(`governance.rs::todo_is_countable`)이 이미 같은 정책을 쓰고 있었고, 여기서
      정책을 일치시킨다(ADR-3 fail-open · 2언어 동형).

    [배제 규칙 · 2026-07-26 유령데이터 결함 수정]
      retired  : 머리말 무효화 마커(우선순위 최상 — 명시 선언은 mtime·위치보다 강하다)
      shadowed : 같은 라벨의 정본이 따로 있다(정본 우선순위 = 선언 보유 > 정본위치(pack/round)
                 > **미완(done<total)** > 최신 mtime > 경로순).
                 라벨당 1파일만 남겨 소비자의 dict 집계에서 한쪽이 조용히 덮이는 사고를 막는다.
      orphan   : 라벨이 현재 살아있는 role 어디에도 없다(비정본 위치 한정·status 수집 성공 시만).
      stale    : 비정본 위치이고 최종수정이 임계일(기본 7일) 초과 — 종결 레인의 유산.

    정본 위치(pack/round)는 orphan·stale 판정에서 면제한다: 노드 자신의 정규 경로에 있는 미완
    작업을 침묵시켜 시야에서 잃는 쪽이 유령보다 위험하다(보수적 설계).
    """
    pack_round = os.path.realpath(os.path.join(pack_dir(), "round"))
    roles = live_roles(status)
    stale_cut = stale_days()
    scope = my_scope()

    recs = []
    for path in files:
        c = count_checkboxes(path)
        if c is None:
            continue
        try:
            mtime = os.path.getmtime(path)
        except OSError:
            mtime = 0.0
        done, total = c
        # ⓪ 선언 판정 — 1순위(ADR-1). **라벨 산출보다 먼저** 수행한다: 라벨의 진실도 선언의
        #    `owner`이지 파일명이 아니기 때문이다(교정 3). 파서 부재(구버전 팩)면 전량 폴백.
        d, diag, verdict = None, None, "unclaimed"
        if _decl is not None:
            d, diag = _decl.parse(_decl.read_head(path))
            verdict = _decl.classify(d, scope, scope_exists)
        # ★교정 3 — owner는 **role 라벨공간에 실재할 때만** 라벨이 된다. 실재하지 않으면
        #   파일명 폴백 + `owner_unresolved` 진단을 달아 사람·JSON 양쪽에 드러낸다
        #   (조용한 폴백은 "조인이 왜 안 붙는지"를 다시 감춘다 — 그게 D3의 정체였다).
        label = decl_label(d, roles)
        unresolved = None
        if label is None and roles is not None:
            declared = decl_label(d)          # 검증 없이 산출한 라벨(=선언이 주장한 owner)
            if declared is not None:
                unresolved = declared
        recs.append({
            "node": label or node_label(path),
            "path": path,
            "done": done,
            "total": total,
            "pct": pct(done, total),
            "canonical_loc": os.path.dirname(os.path.realpath(path)) == pack_round,
            "mtime": mtime,
            "age_days": int((now - mtime) // 86400) if mtime else None,
            "_diag": diag,
            "_verdict": verdict,
        })
        if unresolved:
            recs[-1]["owner_unresolved"] = unresolved

    # ⓪-b 선언 판정 반영. 유효 선언이 있으면 여기서 판정이 끝나고 휴리스틱은 적용되지 않는다.
    for r in recs:
        verdict = r.pop("_verdict")
        r["_declared"] = verdict != "unclaimed"
        if verdict == "counted":
            continue                      # 선언이 내 팩 소유·활성이라고 명시 → 집계
        if verdict == "orphan-scope":
            # ★교정 1 — 조용히 빼지 않는다. 집계에 남기고 플래그로 시끄럽게 알린다.
            #   (Rust `todo_is_countable`과 동일 정책 · 위 [nodes[] 잔류 규칙] 참조)
            r["flag"] = "orphan-scope"
            r["source"] = "decl"
            continue
        if r["_declared"]:
            r["excluded"] = verdict       # retired | foreign-scope — 이 둘만 조용한 배제다
            r["source"] = "decl"
            # ★주인이 처분을 **명시**한 파일(아래 `pending_outside` 면제 기준 · master 심판
            #   2026-07-26). 이름이 아니라 의미로 판정한다 — 아래 `_OWNER_DECLARED` 주석 참조.
            r["_owner_declared"] = True

    # ── 이하 ①~④는 **미선언 파일에만** 적용되는 폴백 휴리스틱(M3에서 전량 삭제 예정) ──

    # ① retired — 이후 정본 경쟁에서 제외
    for r in recs:
        if r["_declared"] or r.get("excluded"):
            continue
        if is_retired(r["path"]):
            r["excluded"] = "retired"
            r["source"] = "heuristic"
            # ★W13 교정 1(b) — 여기에 `_owner_declared` 면제를 **달지 않는다**(master 심판
            # 2026-07-26). 종전 논거는 "판정 주체는 휴리스틱이어도 판정 **근거**는 주인이 적은
            # 명시 은퇴 선언이다"였다. 그러나 면제는 *"주인이 명시했다"*의 신뢰도에 붙는
            # 특권인데, 이 경로는 **파서가 확정하지 않은 추론**이다 — 실제로 치명 A의 자해가
            # 정확히 이 경로로 들어와 미완 2건을 가진 파일이 `pending_outside_nodes`에조차
            # 뜨지 않았다(마지막 방어선까지 통과). 파서가 확정한 판정(`source=decl`)에만
            # 특권을 준다.
            #
            # 면제를 잃어도 **진짜 은퇴 파일은 영향이 없다**: 은퇴한 파일에는 미완 항목이
            # 남아 있지 않으므로 `pending_outside_nodes`(미완 전건 목록)에 애초에 뜨지 않는다.
            # 미완이 남은 채 은퇴로 읽힌 파일만 이제 시끄러워지는데, 그것이 바로 우리가
            # 보고 싶은 상태다. 이 대칭은 회귀 테스트 2종으로 핀돼 있다
            # (`test_b_heuristic_retired_with_pending_blocks_quiet` / `..._without_pending_...`).

    # ② shadowed — 라벨별 정본 1개 선출
    #
    # 선언 보유 파일도 이 선출에 **참여**시킨다(우선순위 최상). 라벨 중복을 방치하면 소비자
    # (report_gate)의 dict 집계에서 한쪽이 조용히 덮여 정체 감시가 눈이 머는 D3 사고가 그대로
    # 재발하기 때문이다 — 이 방어는 선언 여부와 무관하게 필요하다. 다만 선언 보유 파일이
    # 미선언 파일에게 밀려나는 일은 없도록 정렬 키 선두에 `_declared`를 둔다(ADR-1 존중).
    by_label = {}
    for r in recs:
        if r.get("excluded"):
            continue
        by_label.setdefault(r["node"], []).append(r)
    #
    # ★교정 2(a) — 정렬키에 **미완 우선**을 넣는다. 이전 키는 선언 보유 > 정본위치 > mtime
    # 뿐이라 **미완 여부를 전혀 보지 않았다**: 완료된 파일(1/1)이 미완 파일(0/2)을 밀어내
    # `nodes[]`가 [worker 1/1]만 남고 살아있는 작업이 `shadowed`로 소실됐다(reviewer1 재현).
    # 그 상태는 `in_progress_tasks`를 비워 false QUIET을 성립시킨다. 살아있는 작업이 완료된
    # 파일에 밀리는 일은 없어야 한다 — 다만 이 정렬만으로는 상위 키(선언·정본위치)가 다른
    # 조합을 못 막으므로, 아래 (b) 구조적 불변식이 최종 방어다.
    for label, group in by_label.items():
        if len(group) < 2:
            continue
        group.sort(key=lambda r: (not r["_declared"], not r["canonical_loc"],
                                  not (r["total"] > 0 and r["done"] < r["total"]),
                                  -r["mtime"], r["path"]))
        for r in group[1:]:
            r["excluded"] = "shadowed"
            r["source"] = "heuristic"     # 판정 주체가 휴리스틱이면 선언 보유 파일이어도 heuristic
            r["shadowed_by"] = group[0]["path"]

    # ③ orphan / ④ stale — 비정본 위치 + **경과일 초과** 한정
    #
    # ★적대검증 교정(2026-07-26): orphan에 경과 조건이 없으면 **가장 위험한 상태를 침묵**시킨다.
    # 실측 논증 — 워커가 죽으면 role이 status에서 사라진다. 그 순간 그 워커의 미완 todo가 즉시
    # orphan이 되어 nodes[]에서 빠지고, 게이트의 QUIET/park 판정은 `in_progress_tasks(report)`가
    # 비었는지로 내려지므로 **미완 작업이 남았는데 '할 일 없음'으로 주차**할 수 있다. 유령을 막으려다
    # 진짜 사고를 숨기는 셈이다. 그래서 갓 고아가 된 파일은 계속 집계에 남긴다(담당 role이 없으니
    # stall 승격은 vendor 기본대로 '미지=보수적 발화' — 시끄러운 쪽이 안전한 쪽이다).
    # 유령 4건은 전부 6일 이상 경과라 이 조건으로도 그대로 배제된다(실측).
    for r in recs:
        if r["_declared"] or r.get("excluded") or r["canonical_loc"]:
            continue
        if not (stale_cut and r["age_days"] is not None and r["age_days"] > stale_cut):
            continue                      # 신선 = 무조건 집계 유지(놓치는 것보다 시끄러운 편이 낫다)
        r["excluded"] = "orphan" if (roles is not None and r["node"] not in roles) else "stale"
        r["source"] = "heuristic"

    active = [r for r in recs if not r.get("excluded")]
    excluded = [r for r in recs if r.get("excluded")]

    # 미선언 관측 버킷 — active의 **부분집합 미러**다(집계에서 빼지 않는다).
    # ⚠ 별도 레코드로 nodes[]에 덧붙이면 라벨이 중복돼 report_gate의 stall/idle 판정이
    # 오발화한다. 그래서 새 항목을 만드는 것이 아니라 같은 파일을 다른 시점으로 비출 뿐이다.
    unclaimed = [{"node": r["node"], "path": r["path"], "done": r["done"],
                  "total": r["total"], "pct": r["pct"], "diag": r["_diag"]}
                 for r in active if not r["_declared"]]

    # ★교정 2(b) 구조적 불변식 — **집계 밖에 남은 미완 작업**의 전수 목록.
    #
    #   불변식: 어떤 버킷에 있든 미완 작업이 하나라도 있으면 게이트는 QUIET/park로 가지 않는다.
    #
    # 이것이 핵심이다. 치명 1·2는 둘 다 "버킷 분류 실수 → nodes[] 이탈 → in_progress_tasks
    # 공집합 → false QUIET/park"라는 **같은 사슬**을 탔다. 분류를 고치는 것은 그 사슬의 첫
    # 고리를 하나씩 때우는 일이라 새 분류가 늘 때마다 같은 구멍이 다시 난다. 그래서 마지막
    # 고리를 끊는다 — 분류가 또 틀려도 park 오발동으로는 **번지지 않는다**.
    #
    # 정의를 "excluded 중 …"이 아니라 "**nodes[]에 없는 것 중 …**"으로 잡은 이유: M3에서
    # 휴리스틱 폴백이 삭제되면 미선언 파일이 `unclaimed[]`로만 남는데(설계 P4-3), 그때 이
    # 목록이 **자동으로** 그 파일들을 포함하게 된다. 버킷 이름에 묶으면 M3에서 다시 뚫린다.
    #
    # ── 면제 기준 `_owner_declared` (master 심판 2026-07-26) ────────────────────
    # 면제 = **주인이 이 파일의 처분을 명시한 것**. 버킷 이름이 아니라 의미가 계약이다
    # (이름으로 하드코딩하면 판정이 하나 늘 때마다 여기를 손봐야 하고, 손보는 것을 잊는
    # 순간이 곧 다음 결함이다 — 위 "M3에서 다시 뚫린다"와 같은 이유).
    #
    #   면제  `retired`(source=decl)       파서가 확정한 명시 은퇴 선언이다
    #   면제  `foreign-scope`              **실재하는** 다른 팩이 주인이라고 스스로 밝혔다
    #   비면제 `retired`(source=heuristic) ★W13 교정 1(b) — 파서가 **확정하지 않은** 추론이다.
    #                                      치명 A의 자해가 정확히 이 경로로 들어와 마지막
    #                                      방어선까지 통과했다. 진짜 은퇴 파일은 미완이 없어
    #                                      이 목록에 애초에 뜨지 않으므로 잃는 것이 없다.
    #   비면제 `orphan-scope` 가리키는 팩이 실재하지 않는다 = 주인 **불명**
    #   비면제 `unclaimed`    선언이 없거나 깨졌다 = 주인 **불명**
    #   비면제 `shadowed`·`orphan`·`stale`  우리 **추론**일 뿐 주인은 아무 말도 하지 않았다
    #
    # `foreign-scope` 면제의 근거는 내부 정합성이다. 우리는 이 파일을 이미 "남의 것"이라며
    # `nodes[]` 집계에서 조용히 뺐다. 그런데 같은 파일이 우리 QUIET은 막는다면 **"진행률에는
    # 안 세지만 영구히 주차는 못 하게 한다"**는 모순이 된다. 배제와 면제는 같은 근거 위에 있어야
    # 한다. 그 파일은 실재하는 그 팩의 게이트가 본다.
    #
    # 반대로 `orphan-scope`·`unclaimed`가 면제가 **아닌** 이유: 주인이 불명이라 **우리가
    # 마지막 관측자**다. 우리가 조용해지면 그 미완 작업을 볼 주체가 세상에 없다. 팩 개명·
    # teardown·scope 오타는 이 조직에서 실제로 일어났고, 그때 살아있는 작업이 사라졌다.
    # ── ★W15 교정 3 — `unresolved` / `stale_ghost` **두 갈래로 분리**(master 심판 2026-07-26) ──
    #
    # 결함: 이 목록 전체가 park를 막았으므로, 07-26 형태의 유령이 존재하는 동안
    # `quiet_branch_holds`가 **영원히** False였다. 즉 **유령을 성공적으로 배제한 바로 그 사실이
    # 세션 주차를 영구 차단**했다. 유령은 정의상 오래된 파일이라 저절로 사라지지 않는다.
    #
    #   `unresolved`  주인 **불명** = `unclaimed` · `orphan-scope` · `shadowed`
    #                 → **park 차단 유지.** 우리가 마지막 관측자다. 우리가 조용해지면 그
    #                   미완 작업을 볼 주체가 세상에 없다.
    #   `stale_ghost` 우리 추론이고 **담당 role도 이미 없다** = 휴리스틱 `orphan`
    #                 → **park 차단하지 않는다.** 이미 집계에서 배제할 만큼 확신한 판정을
    #                   park만 영원히 막게 두는 것은 일관성이 없다(배제와 면제는 같은 근거
    #                   위에 있어야 한다 — `foreign-scope` 면제와 같은 논리다).
    #                   대신 **보고에는 계속 노출한다**(숨기는 것이 아니라 park 조건에서만 뺀다).
    #
    # ★W18 교정 1 — 휴리스틱 `stale`은 `stale_ghost`가 **아니다**(`unresolved`로 park를 막는다).
    #   `orphan`과 `stale`을 가르는 것은 **담당 role의 생사**뿐인데, `stale`은 담당 role이
    #   살아 있는 파일이다. 살아있는 담당자의 미완을 우리 추론(mtime)만으로 침묵시키는 것은
    #   ADR-3·A3 교훈에 어긋난다. 07-26 유령 4파일은 전부 종결 레인(role 부재)=`orphan`이라
    #   livelock 해소는 그대로 유지된다. 상세 근거는 `PENDING_STALE_GHOST_BUCKETS` 블록.
    #
    # ★휴리스틱 `retired`는 `stale_ghost`가 아니다 — `unresolved`로 남아 park를 막는다.
    #   W13 교정 1(b)가 그 경로를 비면제로 만든 이유가 그대로 유효하기 때문이다: 치명 A의
    #   자해(산문 한 줄이 파일을 은퇴시킴)가 정확히 이 경로로 들어와 마지막 방어선까지
    #   통과했다. `orphan`은 **경과일 + 비정본 위치 + 담당 role 부재**라는 관측 사실에
    #   근거하지만 휴리스틱 `retired`는 **문자열 매칭**이라 오탐의 성질이 다르다.
    #
    # ★유령을 처분할 **운영 경로**가 동반돼야 이 분리가 정직해진다 —
    #   `javis_todo_stamp.py --promote-retire`가 유령·레거시 마커 파일을 명시 은퇴 선언으로
    #   승격시킨다(그러면 `retired`(source=decl) 면제로 들어가 목록에서 아예 빠진다).
    #   정책을 지키려면 도구가 그 정책을 집행할 수 있어야 한다.
    node_paths = {r["path"] for r in active}
    pending_outside = [
        {"node": r["node"], "path": r["path"], "done": r["done"], "total": r["total"],
         "bucket": r.get("excluded") or "unclaimed", "source": r.get("source"),
         "kind": pending_kind(r.get("excluded") or "unclaimed", r.get("source"))}
        for r in recs
        if r["path"] not in node_paths
        and not r.get("_owner_declared")
        and r["total"] > 0 and r["done"] < r["total"]
    ]

    declared_n = sum(1 for r in recs if r["_declared"])
    stats = {
        "total": len(recs),
        "declared": declared_n,
        # M2 관측 지표(설계 P4-1). 목표 = 0.10 미만이면 M3(휴리스틱 삭제) 전환 가능.
        "unclaimed_ratio": (round((len(recs) - declared_n) / len(recs), 4) if recs else 0.0),
    }
    for r in recs:                        # 내부 판정용 키는 출력 계약에서 제외(nodes[] 형태 불변)
        r.pop("_declared", None)
        r.pop("_diag", None)
        r.pop("_owner_declared", None)
    return active, excluded, unclaimed, pending_outside, stats


def build_report(status, extra_dirs, now=None, sampled_at=None):
    """`sampled_at` = status 를 실제로 수집한 시각(epoch float · 설계 층4).

    호출부가 주지 않으면 `now` 로 폴백한다(테스트 결정론 유지). status 미수집이면 무조건
    `None` 이다 — 수집하지 않은 것에 시각을 붙이면 그 값이 곧 거짓 근거가 된다.
    """
    files = discover_todo_files(status, extra_dirs)
    now = time.time() if now is None else now
    nodes, excluded, unclaimed, pending_outside, decl_stats = classify_files(files, status, now)
    agg_done = sum(n["done"] for n in nodes)
    agg_total = sum(n["total"] for n in nodes)

    # ── ★교정 5(master 심판 2026-07-26) — 종합 진행률이 미완을 숨기지 않는다 ──────────
    #
    # 결함: `shadowed` 선출 승자가 완료 파일이면 `nodes=[('worker','1/1')]`이 되어 종합 100%가
    # 뜨는데 밀려난 파일에는 미완 2건이 남아 있었다. park는 불변식이 막지만, 이 스크립트가
    # 스스로 *"이 출력이 유일한 사실"*(파일 상단)이라 규정한 것과 어긋난다.
    #
    # **택한 계약(코드·출력 양쪽에 명시)**
    #   ① `overall_pct/done/total`의 모수는 **`nodes[]` 그대로 둔다**(불변).
    #   ② 대신 `overall_complete`(bool)와 `hidden_pending`(집계 밖 미완 합계)을 신설하고,
    #      텍스트는 `overall_complete`가 False면 **완료 주장을 렌더하지 않는다** — 백분율에는
    #      항상 "집계 기준"이라는 한정어와 반증(집계 밖 미완 N건)이 함께 나간다.
    #
    # **기각한 대안: 모수에 `pending_outside_nodes` 편입** — 그 목록은 구조상 orphan·stale·
    # shadowed 유령을 포함한다(07-26 사고의 그 4파일이 실제로 여기 있다). 편입하면 8일 지난
    # 유령의 79/117이 진행률로 되돌아와 유령 차단 자체가 무효가 된다(회귀 테스트
    # `test_t9_aggregate_uncontaminated`가 그 방향을 이미 금지하고 있다). 배제와 집계는
    # 같은 근거 위에 있어야 하므로, 숫자를 오염시키는 대신 **완료 주장을 철회**하는 쪽을 택했다.
    #
    # ★W15 교정 3 — `hidden_pending`을 **두 숫자로 쪼갠다**(master 심판 2026-07-26).
    #   합계(files/open)는 종전 계약 그대로 남긴다(구버전 소비자·텍스트 렌더가 읽는다).
    #   그 아래 `unresolved`/`stale_ghost`가 park 판정의 근거다 — 게이트는 `unresolved`만
    #   요구하고, `stale_ghost`는 **보고에 계속 노출하되 park를 막지 않는다**.
    #   `overall_complete`는 종전대로 **엄격**하다: 어디에든 미완이 있으면 완료를 주장하지
    #   않는다. park(주차 가능)와 complete(전부 끝남)는 다른 질문이고, 유령이 남은 상태는
    #   "주차해도 되지만 끝난 것은 아니다"가 정확한 서술이다.
    hidden_open = sum(r["total"] - r["done"] for r in pending_outside)
    unresolved_po = [r for r in pending_outside
                     if r.get("kind") != PENDING_KIND_STALE_GHOST]
    ghost_po = [r for r in pending_outside
                if r.get("kind") == PENDING_KIND_STALE_GHOST]
    hidden_pending = {
        "files": len(pending_outside), "open": hidden_open,
        "unresolved": {"files": len(unresolved_po),
                       "open": sum(r["total"] - r["done"] for r in unresolved_po)},
        "stale_ghost": {"files": len(ghost_po),
                        "open": sum(r["total"] - r["done"] for r in ghost_po)},
    }
    overall_complete = (agg_total > 0 and agg_done == agg_total and not pending_outside)

    # 노드 현황·idle·feed (status 있을 때만)
    live_nodes = []
    idle_nodes = []
    feed_pending = None
    paused = None
    # ★T-0147-2 층4 — 수집 시각·출처는 **레코드에 박아** 내보낸다(소비자가 추정하지 않게).
    sampled_at_val = None if status is None else (now if sampled_at is None else sampled_at)
    measure_source = MEASURE_SOURCE_NONE if status is None else MEASURE_SOURCE_STATUS
    if status:
        feed_pending = status.get("feed", {}).get("pending")
        paused = status.get("paused")
        for s in status.get("surfaces", []):
            role = s.get("role")
            if not role:
                continue
            ag = s.get("status") or {}  # org.status는 자기보고를 "status" 필드로 노출
            us = s.get("usage") or {}   # org.status는 데몬 관측 사용량을 "usage" 필드로 노출
            # idle = PTY 무출력 경과(surface 상위 "idle_secs"). 자기보고 갱신 경과(status.age_secs)는
            # 활동 중에도 갱신되므로 idle 판정에 쓰면 안 된다(절대지침 B3: 출력 멎은 지 5분+).
            idle_secs = s.get("idle_secs")
            # ★T-0147-2 §2-C — stall critical push 의 **fail-closed 확증 2차 증거**(가산).
            #   last_output 유래 idle 은 "픽셀이 안 바뀐 시간"이라 신뢰할 수 없어(설계 §0-4)
            #   critical 승격 입력에서 제외됐다. 그 자리를 아래 두 값이 메운다:
            #     ② status_age_secs — `cys set-status` 자기보고 경과(에이전트가 스스로 말한 것)
            #     ③ usage_ctx_tokens — 데몬 관측 토큰(게이트가 주기 간 **델타**를 낸다)
            #   ⚠idle 판정에는 절대 쓰지 않는다(위 계약 그대로). 용도는 확증 전용이다.
            #   ⚠미측정은 반드시 None 이다 — 0 으로 접으면 "델타 0"(=멎었다)과 구별되지 않아
            #     fail-closed 가 fail-open 으로 뒤집힌다(이 조항의 유일한 치명 함정).
            age_secs = ag.get("age_secs")
            ctx_tokens = us.get("ctx_tokens")
            entry = {
                "role": role,
                "state": ag.get("state"),
                "context_pct": ag.get("context_pct"),
                "idle_secs": idle_secs,
                "agent_alive": s.get("agent_alive"),
                "status_age_secs": age_secs if isinstance(age_secs, int) else None,
                "usage_ctx_tokens": ctx_tokens if isinstance(ctx_tokens, int) else None,
            }
            live_nodes.append(entry)
            if isinstance(idle_secs, int) and idle_secs >= IDLE_ALERT_SECS and s.get("agent_alive"):
                idle_nodes.append(entry)
    # 권위 측정 레코드 — 모집단·값은 `live_nodes` 에서만 파생한다(두 번째 판정 기준 금지).
    # 정렬은 role 오름차순(안정 정렬이라 동명 role 은 status 순서를 그대로 보존한다 — 결정론).
    role_measurements = sorted(
        (_measurement_from_node(n, sampled_at_val) for n in live_nodes),
        key=lambda m: m["role"])

    return {
        # ⚠ 모수는 `nodes[]`다(유령 재유입 금지). "전부 끝났다"는 주장은 아래 두 필드가 한다.
        "overall_pct": pct(agg_done, agg_total),
        "overall_done": agg_done,
        "overall_total": agg_total,
        # ★교정 5 — 미완이 **어디에도** 없을 때만 True. 완료 여부를 묻는 기계 소비자는
        # `overall_pct == 100`이 아니라 이 필드를 봐야 한다(구버전 보고기엔 없으므로 스큐 안전).
        "overall_complete": overall_complete,
        # 집계 밖에 남은 미완의 요약(전건은 `pending_outside_nodes`).
        "hidden_pending": hidden_pending,
        "nodes": nodes,
        "live_nodes": live_nodes,
        "idle_nodes": idle_nodes,
        "feed_pending": feed_pending,
        "paused": paused,
        "status_available": status is not None,
        # ★T-0147-2 층4 — 판정 근거 단일화(가산 · 구버전 게이트는 이 키를 몰라 종전대로 동작).
        #   `sampled_at`/`measure_source` 는 "언제·어디서 잰 값인가"를 레코드에 박는다.
        #   `role_measurements` 는 게이트가 idle 판정에 쓸 **유일한 권위 레코드**다
        #   (live_nodes·idle_nodes 두 갈래를 라벨로 조인하던 종전 방식의 대체재).
        "sampled_at": sampled_at_val,
        "measure_source": measure_source,
        "role_measurements": role_measurements,
        # 배제된 todo 전건(사유 포함) — 소비자는 집계에 쓰지 않되 사람에게는 반드시 보인다.
        # 항목의 "source"는 판정 출처("decl"=선언 / "heuristic"=폴백 4규칙) — 마이그레이션 관측용.
        "excluded": excluded,
        # 선언 미보유이나 배제되지도 않은 파일(=nodes[]에 그대로 집계 중인 것의 미러).
        # 경고가 아니라 **정보**다(온보딩 방어 ①). 게이트는 nodes[]만 소비하므로 무해하다.
        "unclaimed": unclaimed,
        # ★구조적 불변식의 소스 필드(교정 2-b) — `nodes[]` 밖에 남은 **미완** 작업 전건.
        # `javis_report_gate`가 QUIET/park 분기의 **추가 AND 조건**으로 이 목록 중
        # `kind != "stale_ghost"`인 항목의 공집합을 요구한다(W15 교정 3). 구버전 게이트는
        # 이 키를 모르고 무시하고, 구버전 **보고기**가 `kind`를 싣지 않으면 새 게이트는 그
        # 항목을 `unresolved`로 본다 — 양방향 모두 **보수적**으로 스큐 안전하다(ADR-2).
        "pending_outside_nodes": pending_outside,
        # M2 관측 지표 — 미선언 비율이 10% 미만이 되면 휴리스틱 삭제(M3) 전환이 가능해진다.
        "decl_stats": decl_stats,
    }


def render_text(rep):
    lines = []
    lines.append("주인님께 보고 (5분 주기 진행 현황 · 결정론 산출):")
    # ── 전체 진행 (★교정 5 계약: 백분율은 **집계 모수 한정**이고, 집계 밖 미완이 있으면
    #    완료를 주장하지 않는다. 계약 근거는 build_report의 해당 주석에 적혀 있다.) ──
    hp = rep.get("hidden_pending") or {}
    hidden_files, hidden_open = hp.get("files", 0), hp.get("open", 0)
    if rep["overall_total"] == 0 and not hidden_files:
        lines.append("  • 전체 진행: todo 미등록(착수 전) — *_TODO.md에 체크박스가 없습니다")
    else:
        head = ("  • 전체 진행: %d%% (%d/%d 완료, **집계 기준**)"
                % (rep["overall_pct"], rep["overall_done"], rep["overall_total"]))
        if hidden_files:
            # ⚠는 의도적이다 — 집계 밖 미완은 온보딩 정보가 아니라 분류 재확인이 필요한 상태다.
            head += (" · ⚠ 완료 아님: 집계 밖 미완 %d건(파일 %d개) — 아래 목록 참조"
                     % (hidden_open, hidden_files))
        lines.append(head)
    if rep["nodes"]:
        lines.append("  • 노드별 진행:")
        for n in rep["nodes"]:
            lines.append("      - %s: %d%% (%d/%d)"
                         % (n["node"], n["pct"], n["done"], n["total"]))
    else:
        lines.append("  • 노드별 진행: *_TODO.md 미발견")

    # ── 귀속 팩 부재(orphan-scope) — **집계에 남긴 채** 시끄럽게 알린다(교정 1) ──────────
    # 배제 목록이 아니라 여기에 따로 세우는 이유: 이 파일들은 nodes[]에 살아 있어 진행률과
    # 게이트 판정에 그대로 참여한다. "제외했다"고 적으면 사람이 읽는 사실과 기계가 쓰는
    # 사실이 갈린다 — 그 괴리가 정확히 이번 결함의 형태였다.
    orph = [n for n in rep["nodes"] if n.get("flag") == "orphan-scope"]
    if orph:
        lines.append("  • 귀속 팩 부재(선언) todo %d건 — 집계 **유지**(팩 개명·teardown·"
                     "scope 오타 의심 · 확인 필요):" % len(orph))
        for n in orph:
            lines.append("      - %s: %d/%d — %s" % (n["node"], n["done"], n["total"], n["path"]))

    # ── 선언 owner가 살아있는 role에 없다(★교정 3) — 조용한 폴백은 조인 실패를 다시 감춘다 ──
    # 이 상태에서 라벨은 파일명 폴백을 쓰므로 게이트 조인은 살아 있다. 사람에게는 "선언과 편대
    # 현황이 어긋나 있다"는 사실을 보여야 선언을 고칠 수 있다(설계 문서 예시가 발현원이었다).
    unres = [n for n in (rep["nodes"] + (rep.get("excluded") or []))
             if n.get("owner_unresolved")]
    if unres:
        lines.append("  • ⚠ 선언 owner가 현재 role에 없음 %d건 — 라벨은 파일명으로 폴백"
                     "(선언 오기재·역할 종료 의심):" % len(unres))
        for n in unres:
            lines.append("      - 선언 owner=%s → 사용 라벨=%s — %s"
                         % (n["owner_unresolved"], n["node"], n["path"]))

    exc = rep.get("excluded") or []
    if exc:
        why = {"retired": "무효화 선언", "shadowed": "정본 아님(중복 라벨)",
               "orphan": "귀속 노드 없음", "stale": "유산(장기 무수정)",
               "foreign-scope": "남의 팩 소유(선언)", "orphan-scope": "귀속 팩 부재(선언)"}
        lines.append("  • 집계 제외 %d건 (유령 todo 차단 — 삭제 아님·열람 가능):" % len(exc))
        for r in exc:
            age = "" if r.get("age_days") is None else " · %d일 전" % r["age_days"]
            src = "선언" if r.get("source") == "decl" else "휴리스틱"
            lines.append("      - %s [%s%s · 판정=%s] %d/%d — %s"
                         % (r["node"], why.get(r["excluded"], r["excluded"]), age, src,
                            r["done"], r["total"], r["path"]))

    # ── 집계 밖 미완 작업(구조적 불변식의 사람 대면 면) ─────────────────────────
    # 기계는 이 목록으로 QUIET/park를 막고, 사람은 여기서 "왜 조용해지지 않는지"를 읽는다.
    # 두 사실을 같은 소스에서 뽑아야 다시 갈리지 않는다.
    #
    # ★W15 교정 3 — 두 갈래를 **따로** 렌더한다. `unresolved`만 park를 막는다는 사실이
    # 사람 눈에도 보여야 "왜 조용해지지 않는지"와 "왜 조용해져도 되는지"가 함께 읽힌다.
    # `stale_ghost`는 park를 막지 않지만 **숨기지 않는다** — park 조건에서만 뺀 것이지
    # 없는 일로 만든 것이 아니다.
    po = rep.get("pending_outside_nodes") or []
    unres_po = [r for r in po if r.get("kind") != PENDING_KIND_STALE_GHOST]
    ghost_po = [r for r in po if r.get("kind") == PENDING_KIND_STALE_GHOST]
    if unres_po:
        lines.append("  • ⚠ 집계 밖 미완 작업(주인 불명) %d건 — QUIET/세션 주차 금지"
                     "(분류 재확인 필요):" % len(unres_po))
        for r in unres_po:
            lines.append("      - %s [%s] %d/%d — %s"
                         % (r["node"], r.get("bucket"), r["done"], r["total"], r["path"]))
    if ghost_po:
        lines.append("  • ℹ 집계 밖 미완 작업(고아 추론 — 담당 role 부재) %d건 — 주차는 막지 않음 "
                     "(명시 은퇴 승격: javis_todo_stamp.py --promote-retire):" % len(ghost_po))
        for r in ghost_po:
            lines.append("      - %s [%s] %d/%d — %s"
                         % (r["node"], r.get("bucket"), r["done"], r["total"], r["path"]))

    # ── 미선언 todo (온보딩 방어 3규칙) ───────────────────────────────────────
    # ① 경고(⚠)가 아니라 정보(ℹ)다. ⚠는 javis_gate_check의 WARNING_KEYWORDS에 들어 있어
    #    쓰는 순간 게이트 WARN으로 승격된다 — 신규 사용자에게 그건 결함이지 신호가 아니다.
    # ② 진행률(done/total)을 빼앗지 않는다. 이 파일들은 nodes[]에 그대로 집계돼 있고
    #    여기서는 "선언만 없다"는 사실을 알릴 뿐이다.
    # ③ todo가 0개면 이 블록 자체가 출력되지 않는다(unc가 비므로 자연 충족 — 신규 설치 무소음).
    unc = rep.get("unclaimed") or []
    if unc:
        st = rep.get("decl_stats") or {}
        cap = 5
        lines.append("  • ℹ 선언 미보유 todo %d건 (진행률은 위 집계에 그대로 포함 — 경고 아님):"
                     % len(unc))
        for r in unc[:cap]:
            lines.append("      - %s: %d/%d — %s (%s)"
                         % (r["node"], r["done"], r["total"], r.get("diag") or "선언 없음",
                            r["path"]))
        if len(unc) > cap:
            # 무언의 절삭 금지 — 요약했다는 사실을 명시하고 전량 경로를 알려준다.
            lines.append("      … 상위 %d건만 표시했습니다 · 외 %d건 생략 "
                         "(전량은 --json의 unclaimed[])" % (cap, len(unc) - cap))
        if st.get("total"):
            lines.append("      (선언 보유 %d/%d · 미선언 비율 %.0f%%)"
                         % (st.get("declared", 0), st["total"],
                            (st.get("unclaimed_ratio") or 0.0) * 100))

    if not rep["status_available"]:
        lines.append("  • 노드 현황: cys status 수집 실패(데몬 미가동?) — todo 스캔만 반영")
    else:
        if rep["paused"]:
            lines.append("  • ⚠ 큐/스케줄 일시정지(kill-switch) 상태")
        alive = sum(1 for n in rep["live_nodes"] if n.get("agent_alive"))
        lines.append("  • 활성 노드: %d개" % alive)
        if rep["feed_pending"]:
            lines.append("  • ⚠ 미처리 승인(feed): %d건 — 즉결 필요" % rep["feed_pending"])
        if rep["idle_nodes"]:
            roles = ", ".join(n["role"] for n in rep["idle_nodes"])
            lines.append("  • ⚠ idle 5분+ 노드: %s — read-screen 확인·재지시 필요" % roles)
        high_ctx = [n for n in rep["live_nodes"]
                    if isinstance(n.get("context_pct"), int) and n["context_pct"] >= 60]
        if high_ctx:
            roles = ", ".join("%s(%d%%)" % (n["role"], n["context_pct"]) for n in high_ctx)
            lines.append("  • ⚠ 컨텍스트 60%%+ 노드: %s — cycle-agent 집행 검토" % roles)
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description="5분 주기 진행% 결정론 보고 산출기")
    ap.add_argument("--json", action="store_true", help="JSON 출력")
    ap.add_argument("--extra-dir", action="append", default=[], metavar="DIR",
                    help="추가 스캔 폴더(그 안의 _round/*_TODO.md 및 직접 *_TODO.md)")
    args = ap.parse_args()

    # ★T-0147-2 층4 — 수집 **직후**의 시각을 권위 레코드에 싣는다(추정 금지).
    status = cys_status()
    rep = build_report(status, args.extra_dir,
                       sampled_at=(time.time() if status is not None else None))
    if args.json:
        print(json.dumps(rep, ensure_ascii=False, indent=2))
    else:
        print(render_text(rep))
    return 0


if __name__ == "__main__":
    sys.exit(main())
